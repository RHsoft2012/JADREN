//! Minimal, deterministic Language Server Protocol support for Jadren.
//!
//! The server deliberately reuses [`jadren_driver::CompilerSession`] for
//! diagnostics and [`jadren_parser`] artifacts for symbols. It implements a
//! full-document sync transport first; incremental text edits and semantic
//! token full/delta queries share deterministic query state.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use jadren_determinism::StableHasher;
use jadren_diagnostics::{Diagnostic, Severity};
use jadren_driver::{CompilerConfig, CompilerSession};
use jadren_effects::EffectAnalysis;
use jadren_lexer::{Keyword, TokenKind};
use jadren_parser::{Block, Expression, Item, Pattern, Statement};
use jadren_resolve::{
    DeclaredVisibility, ModuleEnumInterface, ModuleFunctionSignature, ModuleRecordInterface,
    ModuleType, Namespace, ResolutionOutput, ScopeId, Symbol, SymbolId, SymbolKind, SymbolOrigin,
};
use jadren_source::{SourceFile, SourceId, Span};
use jadren_types::{
    Capability, FloatWidth, IntegerWidth, NominalTypeId, Signedness, TypeId, TypeKind, TypeStore,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Zero-based LSP UTF-16 position.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Position {
    /// Zero-based line.
    pub line: u32,
    /// Zero-based UTF-16 code-unit column.
    pub character: u32,
}

/// Half-open LSP range.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Range {
    /// Start position.
    pub start: Position,
    /// End position.
    pub end: Position,
}

/// LSP diagnostic payload published by Jadren.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LspDiagnostic {
    /// Highlighted range.
    pub range: Range,
    /// LSP severity: 1 error, 2 warning, 3 information.
    pub severity: u8,
    /// Stable Jadren diagnostic code.
    pub code: String,
    /// Source label shown by editors.
    pub source: &'static str,
    /// Human-readable message.
    pub message: String,
}

/// LSP document symbol payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DocumentSymbol {
    /// Symbol display name.
    pub name: String,
    /// LSP SymbolKind numeric value.
    pub kind: u32,
    /// Full declaration range.
    pub range: Range,
    /// Identifier selection range.
    pub selection_range: Range,
    /// Optional nested symbols.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub children: Vec<DocumentSymbol>,
}

/// URI/range pair returned by definition and references queries.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Location {
    /// Document URI.
    pub uri: String,
    /// Referenced source range.
    pub range: Range,
}

/// One rename edit in a document.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TextEdit {
    /// Replaced range.
    pub range: Range,
    /// Replacement identifier text.
    pub new_text: String,
}

/// Hover markdown payload.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Hover {
    /// Markdown content shown by the editor.
    pub contents: MarkupContent,
    /// Symbol range associated with the hover.
    pub range: Option<Range>,
}

/// LSP markup payload.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MarkupContent {
    /// Markup kind.
    pub kind: &'static str,
    /// Markdown value.
    pub value: String,
}

/// One completion candidate derived from source symbols.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CompletionItem {
    /// Inserted label.
    pub label: String,
    /// LSP SymbolKind-compatible completion kind.
    pub kind: u32,
    /// Short source/type detail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// One deterministic source edit offered by an LSP code action.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CodeAction {
    /// Human-readable quick-fix title.
    pub title: String,
    /// LSP action category.
    pub kind: &'static str,
    /// Diagnostic which caused this action.
    pub diagnostics: Vec<LspDiagnostic>,
    /// Workspace edit applied by the client.
    pub edit: WorkspaceEdit,
    /// Prefer this action when the editor supports ranking.
    pub is_preferred: bool,
}

/// Workspace edit grouped by document URI.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WorkspaceEdit {
    /// Deterministically ordered document edits.
    pub changes: BTreeMap<String, Vec<TextEdit>>,
}

/// Basic inferred-type inlay hint.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InlayHint {
    /// Position immediately after the binding name.
    pub position: Position,
    /// Display label, for example `: Int32`.
    pub label: String,
    /// LSP hint kind: 1 type, 2 parameter.
    pub kind: u8,
    /// Keep the label visually attached to the identifier.
    pub padding_left: bool,
}

/// Query-backed allocation summary for one lexical region.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AllocationAnalytics {
    /// Region handle name.
    pub region: String,
    /// Number of typed allocation sites in the region.
    pub count: u64,
    /// Stable, source-formatted result types used by the sites.
    pub result_types: Vec<String>,
    /// Region declaration/name range.
    pub range: Range,
}

/// Full semantic-token response with a deterministic cache identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SemanticTokens {
    /// Result identity supplied to a subsequent delta request.
    #[serde(rename = "resultId")]
    pub result_id: String,
    /// LSP five-integer delta-encoded token data.
    pub data: Vec<u32>,
}

/// One replacement in a semantic-token delta response.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SemanticTokensEdit {
    /// Zero-based offset into the flattened token data.
    pub start: u32,
    /// Number of old data values to remove.
    #[serde(rename = "deleteCount")]
    pub delete_count: u32,
    /// Replacement data values.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub data: Vec<u32>,
}

/// Incremental semantic-token response.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SemanticTokensDelta {
    /// Result identity for the new token data.
    #[serde(rename = "resultId")]
    pub result_id: String,
    /// Deterministic replacement edits.
    pub edits: Vec<SemanticTokensEdit>,
}

/// One LSP `textDocument/didChange` content change.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TextDocumentContentChange {
    /// Replaced range; `None` means a full-document replacement.
    #[serde(default)]
    pub range: Option<Range>,
    /// Replacement UTF-8 text.
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Document {
    version: i64,
    text: String,
}

#[derive(Clone, Debug)]
struct CheckedContext {
    output: jadren_driver::CheckOutput,
    target_source: SourceFile,
    outputs: BTreeMap<SourceId, jadren_driver::CheckOutput>,
    sources: BTreeMap<SourceId, SourceFile>,
    uris: BTreeMap<SourceId, String>,
}

#[derive(Clone, Debug)]
struct WorkspaceCache {
    documents: BTreeMap<String, Document>,
    outputs: BTreeMap<SourceId, jadren_driver::CheckOutput>,
    sources: BTreeMap<SourceId, SourceFile>,
    uris: BTreeMap<SourceId, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SemanticTokenSnapshot {
    result_id: String,
    data: Vec<u32>,
}

impl WorkspaceCache {
    fn checked_context(&self, uri: &str) -> Option<CheckedContext> {
        let target_id = self
            .uris
            .iter()
            .find_map(|(source_id, document_uri)| (document_uri == uri).then_some(*source_id))?;
        Some(CheckedContext {
            output: self.outputs.get(&target_id)?.clone(),
            target_source: self.sources.get(&target_id)?.clone(),
            outputs: self.outputs.clone(),
            sources: self.sources.clone(),
            uris: self.uris.clone(),
        })
    }
}

/// In-memory document/query state for one LSP connection.
#[derive(Clone, Debug, Default)]
pub struct LanguageServer {
    documents: BTreeMap<String, Document>,
    cache: RefCell<Option<WorkspaceCache>>,
    semantic_token_snapshots: RefCell<BTreeMap<String, SemanticTokenSnapshot>>,
}

impl LanguageServer {
    /// Creates an empty server state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stores a full document and returns current diagnostics.
    pub fn open_document(
        &mut self,
        uri: impl Into<String>,
        version: i64,
        text: impl Into<String>,
    ) -> Vec<LspDiagnostic> {
        let uri = uri.into();
        self.documents.insert(
            uri.clone(),
            Document {
                version,
                text: text.into(),
            },
        );
        self.cache.get_mut().take();
        self.diagnostics(&uri)
    }

    /// Replaces a full document and returns current diagnostics.
    pub fn change_document(
        &mut self,
        uri: impl Into<String>,
        version: i64,
        text: impl Into<String>,
    ) -> Vec<LspDiagnostic> {
        self.open_document(uri, version, text)
    }

    /// Applies ordered LSP content changes to the current document.
    ///
    /// Ranges are decoded as UTF-16 positions and must land on Unicode scalar
    /// boundaries. Invalid ranges leave the document untouched and return
    /// `None`, allowing the transport to ignore malformed notifications.
    pub fn change_document_incremental(
        &mut self,
        uri: impl Into<String>,
        version: i64,
        changes: &[TextDocumentContentChange],
    ) -> Option<Vec<LspDiagnostic>> {
        let uri = uri.into();
        let mut text = self.documents.get(&uri)?.text.clone();
        for change in changes {
            if let Some(range) = change.range {
                let start = position_to_offset(&text, range.start)?;
                let end = position_to_offset(&text, range.end)?;
                if start > end || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
                    return None;
                }
                text.replace_range(start..end, &change.text);
            } else {
                text.clone_from(&change.text);
            }
        }
        Some(self.change_document(uri, version, text))
    }

    /// Returns the last version known for a document.
    #[must_use]
    pub fn version(&self, uri: &str) -> Option<i64> {
        self.documents.get(uri).map(|document| document.version)
    }

    /// Runs the shared compiler frontend and maps diagnostics to LSP ranges.
    #[must_use]
    pub fn diagnostics(&self, uri: &str) -> Vec<LspDiagnostic> {
        let Some(context) = self.checked_output(uri) else {
            return Vec::new();
        };
        context
            .output
            .diagnostics
            .iter()
            .map(|diagnostic| map_diagnostic(diagnostic, &context.target_source))
            .collect()
    }

    /// Returns deterministic top-level document symbols from parsed AST.
    #[must_use]
    pub fn document_symbols(&self, uri: &str) -> Vec<DocumentSymbol> {
        let Some(context) = self.checked_output(uri) else {
            return Vec::new();
        };
        let Some(artifacts) = context.output.artifacts.as_ref() else {
            return Vec::new();
        };
        artifacts
            .ast
            .items
            .iter()
            .filter_map(|item| map_item_symbol(item, &context.target_source))
            .collect()
    }

    /// Resolves the source declaration under an LSP position.
    #[must_use]
    pub fn definition(&self, uri: &str, position: Position) -> Vec<Location> {
        let Some(context) = self.checked_output(uri) else {
            return Vec::new();
        };
        let output = &context.output;
        let source = &context.target_source;
        let Some(symbol_id) = symbol_at_position(
            output
                .artifacts
                .as_ref()
                .map(|artifacts| &artifacts.resolution),
            source,
            position,
        ) else {
            return Vec::new();
        };
        let Some(symbol) = output
            .artifacts
            .as_ref()
            .and_then(|artifacts| artifacts.resolution.symbol(symbol_id))
        else {
            return Vec::new();
        };
        if !matches!(symbol.origin, SymbolOrigin::Source | SymbolOrigin::Imported) {
            return Vec::new();
        }
        location_for_span(&context, symbol.span)
            .into_iter()
            .collect()
    }

    /// Resolves workspace references to the symbol under a position.
    #[must_use]
    pub fn references(
        &self,
        uri: &str,
        position: Position,
        include_declaration: bool,
    ) -> Vec<Location> {
        let Some(context) = self.checked_output(uri) else {
            return Vec::new();
        };
        let output = &context.output;
        let source = &context.target_source;
        let Some(artifacts) = output.artifacts.as_ref() else {
            return Vec::new();
        };
        let Some(symbol_id) = symbol_at_position(Some(&artifacts.resolution), source, position)
        else {
            return Vec::new();
        };
        let Some(target_symbol) = artifacts.resolution.symbol(symbol_id) else {
            return Vec::new();
        };
        let target_key = target_symbol.qualified_id;
        let target_source_id = source.id();
        let mut locations = Vec::new();
        for (source_id, candidate_output) in &context.outputs {
            let Some(candidate_artifacts) = candidate_output.artifacts.as_ref() else {
                continue;
            };
            for reference in &candidate_artifacts.resolution.references {
                let Some(candidate_symbol) =
                    candidate_artifacts.resolution.symbol(reference.symbol)
                else {
                    continue;
                };
                let matches = target_key.map_or(
                    *source_id == target_source_id && reference.symbol == symbol_id,
                    |key| candidate_symbol.qualified_id == Some(key),
                );
                if matches && let Some(location) = location_for_span(&context, reference.span) {
                    locations.push(location);
                }
            }
            if include_declaration {
                for candidate_symbol in &candidate_artifacts.resolution.symbols {
                    let matches = target_key.map_or(
                        *source_id == target_source_id && candidate_symbol.id == symbol_id,
                        |key| {
                            candidate_symbol.origin == SymbolOrigin::Source
                                && candidate_symbol.qualified_id == Some(key)
                        },
                    );
                    if matches
                        && let Some(location) = location_for_span(&context, candidate_symbol.span)
                    {
                        locations.push(location);
                    }
                }
            }
        }
        locations.sort_by_key(|location| {
            (
                location.uri.clone(),
                location.range.start.line,
                location.range.start.character,
                location.range.end.line,
                location.range.end.character,
            )
        });
        locations
    }

    /// Computes a workspace edit for a source symbol rename.
    #[must_use]
    pub fn rename(
        &self,
        uri: &str,
        position: Position,
        new_name: &str,
    ) -> Option<BTreeMap<String, Vec<TextEdit>>> {
        if !valid_identifier(new_name) {
            return None;
        }
        let context = self.checked_output(uri)?;
        let output = &context.output;
        let source = &context.target_source;
        let artifacts = output.artifacts.as_ref()?;
        let symbol_id = symbol_at_position(Some(&artifacts.resolution), source, position)?;
        let symbol = artifacts.resolution.symbol(symbol_id)?;
        if !matches!(symbol.origin, SymbolOrigin::Source | SymbolOrigin::Imported) {
            return None;
        }
        let mut changes = BTreeMap::new();
        let target_key = symbol.qualified_id;
        let target_source_id = source.id();
        for (source_id, candidate_output) in &context.outputs {
            let Some(candidate_artifacts) = candidate_output.artifacts.as_ref() else {
                continue;
            };
            for reference in &candidate_artifacts.resolution.references {
                let Some(candidate_symbol) =
                    candidate_artifacts.resolution.symbol(reference.symbol)
                else {
                    continue;
                };
                let matches = target_key.map_or(
                    *source_id == target_source_id && reference.symbol == symbol_id,
                    |key| candidate_symbol.qualified_id == Some(key),
                );
                if matches && let Some(location) = location_for_span(&context, reference.span) {
                    changes
                        .entry(location.uri)
                        .or_insert_with(Vec::new)
                        .push(TextEdit {
                            range: location.range,
                            new_text: new_name.to_owned(),
                        });
                }
            }
            for candidate_symbol in &candidate_artifacts.resolution.symbols {
                let matches = target_key.map_or(
                    *source_id == target_source_id && candidate_symbol.id == symbol_id,
                    |key| {
                        candidate_symbol.origin == SymbolOrigin::Source
                            && candidate_symbol.qualified_id == Some(key)
                    },
                );
                if matches
                    && let Some(location) = location_for_span(&context, candidate_symbol.span)
                {
                    changes
                        .entry(location.uri)
                        .or_insert_with(Vec::new)
                        .push(TextEdit {
                            range: location.range,
                            new_text: new_name.to_owned(),
                        });
                }
            }
        }
        for edits in changes.values_mut() {
            edits.sort_by_key(|edit| {
                (
                    edit.range.start.line,
                    edit.range.start.character,
                    edit.range.end.line,
                    edit.range.end.character,
                )
            });
        }
        Some(changes)
    }

    /// Returns a symbol hover using resolver identity and stable kind text.
    #[must_use]
    pub fn hover(&self, uri: &str, position: Position) -> Option<Hover> {
        let context = self.checked_output(uri)?;
        let output = &context.output;
        let source = &context.target_source;
        let artifacts = output.artifacts.as_ref()?;
        let Some(symbol_id) = symbol_at_position(Some(&artifacts.resolution), source, position)
        else {
            return enum_variant_hover(&context, position)
                .or_else(|| record_field_hover(&context, position))
                .or_else(|| typed_expression_hover(&context, position));
        };
        let symbol = artifacts.resolution.symbol(symbol_id)?;
        let mut value = format!("`{:?}` **{}**", symbol.kind, symbol.name);
        if let Some(type_id) = artifacts.type_check.symbol_type(symbol_id) {
            value.push_str(&format!(
                "\n\nType: `{}`",
                format_type(&artifacts.type_check.types, type_id)
            ));
        }
        if let Some(signature) = symbol.function_signature.as_ref() {
            value.push_str(&format!(
                "\n\nSignature: `{}`",
                format_module_signature(signature)
            ));
        } else if matches!(symbol.origin, SymbolOrigin::Imported)
            && matches!(
                symbol.kind,
                SymbolKind::Struct | SymbolKind::Component | SymbolKind::Enum
            )
            && let Some(canonical_path) = symbol.canonical_path.as_deref()
        {
            value.push_str(&format!("\n\nType: `{canonical_path}`"));
        }
        let effects = if symbol.kind == SymbolKind::Function {
            artifacts
                .effects
                .as_ref()
                .and_then(|analysis| analysis.function(symbol_id))
                .or_else(|| {
                    symbol.qualified_id.and_then(|qualified_id| {
                        context.outputs.values().find_map(|candidate_output| {
                            let candidate_artifacts = candidate_output.artifacts.as_ref()?;
                            let candidate_symbol =
                                candidate_artifacts.resolution.symbols.iter().find(
                                    |candidate| {
                                        candidate.origin == SymbolOrigin::Source
                                            && candidate.qualified_id == Some(qualified_id)
                                    },
                                )?;
                            candidate_artifacts
                                .effects
                                .as_ref()
                                .and_then(|analysis| analysis.function(candidate_symbol.id))
                        })
                    })
                })
        } else {
            None
        };
        if let Some(effects) = effects {
            let names = effects
                .inferred
                .iter()
                .map(jadren_effects::EffectKind::as_str)
                .collect::<Vec<_>>();
            let display = if names.is_empty() {
                "Pure".to_owned()
            } else {
                names.join(", ")
            };
            value.push_str(&format!("\n\nEffects: `{display}`"));
        }
        Some(Hover {
            contents: MarkupContent {
                kind: "markdown",
                value,
            },
            range: Some(span_range(symbol.span, source)),
        })
    }

    /// Returns deterministic source-owned completion candidates.
    #[must_use]
    pub fn completion(&self, uri: &str) -> Vec<CompletionItem> {
        let Some(context) = self.checked_output(uri) else {
            return Vec::new();
        };
        let output = &context.output;
        let source = &context.target_source;
        let Some(artifacts) = output.artifacts.as_ref() else {
            return Vec::new();
        };
        let mut candidates = BTreeMap::new();
        for symbol in &artifacts.resolution.symbols {
            if !is_source_bound_completion_symbol(symbol, source) {
                continue;
            }
            candidates
                .entry(symbol.name.clone())
                .or_insert_with(|| CompletionItem {
                    label: symbol.name.clone(),
                    kind: lsp_symbol_kind(symbol.kind),
                    detail: Some(format!("{:?}", symbol.kind)),
                });
        }
        candidates.into_values().collect()
    }

    /// Returns completion candidates visible at an LSP position.
    #[must_use]
    pub fn completion_at(&self, uri: &str, position: Position) -> Vec<CompletionItem> {
        let Some(context) = self.checked_output(uri) else {
            return Vec::new();
        };
        let output = &context.output;
        let source = &context.target_source;
        let Some(artifacts) = output.artifacts.as_ref() else {
            return Vec::new();
        };
        let Some(offset) = position_to_offset(source.text(), position) else {
            return Vec::new();
        };
        let prefix_start = identifier_prefix_start(source.text(), offset);
        let prefix = &source.text()[prefix_start..offset];
        let current_scope = artifacts
            .resolution
            .scopes
            .iter()
            .filter(|scope| {
                scope.span.source == source.id()
                    && offset >= scope.span.start
                    && offset <= scope.span.end
            })
            .min_by_key(|scope| scope.span.len())
            .map(|scope| scope.id)
            .unwrap_or(artifacts.resolution.root_scope);

        let mut candidates = BTreeMap::new();
        let mut scope_id = Some(current_scope);
        while let Some(current_scope_id) = scope_id {
            let Some(scope) = artifacts.resolution.scope(current_scope_id) else {
                break;
            };
            for symbol_id in scope.symbols.values() {
                let Some(symbol) = artifacts.resolution.symbol(*symbol_id) else {
                    continue;
                };
                if !is_source_bound_completion_symbol(symbol, source)
                    || !symbol.name.starts_with(prefix)
                {
                    continue;
                }
                candidates
                    .entry(symbol.name.clone())
                    .or_insert_with(|| CompletionItem {
                        label: symbol.name.clone(),
                        kind: lsp_symbol_kind(symbol.kind),
                        detail: Some(format!("{:?}", symbol.kind)),
                    });
            }
            scope_id = scope.parent;
        }
        let qualified_module_path = qualified_module_completion_path(
            source.text(),
            prefix_start,
            &artifacts.resolution,
            current_scope,
        );
        if let Some(module_path) = qualified_module_path.as_deref() {
            let member_prefix = format!("{module_path}.");
            for candidate_output in context.outputs.values() {
                let Some(candidate_artifacts) = candidate_output.artifacts.as_ref() else {
                    continue;
                };
                for symbol in &candidate_artifacts.resolution.symbols {
                    if symbol.origin != SymbolOrigin::Source
                        || symbol.visibility != DeclaredVisibility::Public
                        || !symbol.name.starts_with(prefix)
                    {
                        continue;
                    }
                    let Some(canonical_path) = symbol.canonical_path.as_deref() else {
                        continue;
                    };
                    let Some(member_path) = canonical_path.strip_prefix(&member_prefix) else {
                        continue;
                    };
                    if member_path != symbol.name {
                        continue;
                    }
                    candidates
                        .entry(symbol.name.clone())
                        .or_insert_with(|| CompletionItem {
                            label: symbol.name.clone(),
                            kind: lsp_symbol_kind(symbol.kind),
                            detail: Some(format!("{:?}", symbol.kind)),
                        });
                }
            }
        }
        let qualified_type_path = qualified_type_completion_path(
            source.text(),
            prefix_start,
            &artifacts.resolution,
            current_scope,
        );
        if let Some(type_path) = qualified_type_path.as_deref() {
            add_enum_variant_completion_candidates(
                &mut candidates,
                prefix,
                type_path,
                &artifacts.resolution,
                &context.outputs,
            );
        }
        let field_completion_constructor =
            nominal_field_completion_constructor(&artifacts.type_check, source, prefix_start);
        if let Some(constructor) = field_completion_constructor {
            add_record_field_completion_candidates(
                &mut candidates,
                prefix,
                constructor,
                &artifacts.resolution,
                &context.outputs,
                artifacts.resolution.module_name.as_deref(),
            );
        }
        if !prefix.is_empty()
            && qualified_module_path.is_none()
            && qualified_type_path.is_none()
            && field_completion_constructor.is_none()
        {
            for keyword in Keyword::ALL {
                let label = keyword.as_str();
                if !label.starts_with(prefix) {
                    continue;
                }
                candidates
                    .entry(label.to_owned())
                    .or_insert_with(|| CompletionItem {
                        label: label.to_owned(),
                        kind: 14,
                        detail: Some("Keyword".to_owned()),
                    });
            }
        }
        candidates.into_values().collect()
    }

    /// Returns LSP delta-encoded semantic tokens for one open document.
    #[must_use]
    pub fn semantic_tokens(&self, uri: &str) -> Vec<u32> {
        self.semantic_token_data(uri)
    }

    /// Returns a full semantic-token response and remembers its result id for
    /// a later `textDocument/semanticTokens/full/delta` request.
    #[must_use]
    pub fn semantic_tokens_full(&self, uri: &str) -> SemanticTokens {
        let data = self.semantic_token_data(uri);
        let result_id = semantic_token_result_id(&data);
        self.semantic_token_snapshots.borrow_mut().insert(
            uri.to_owned(),
            SemanticTokenSnapshot {
                result_id: result_id.clone(),
                data: data.clone(),
            },
        );
        SemanticTokens { result_id, data }
    }

    /// Returns a deterministic one-edit semantic-token delta.
    ///
    /// `None` tells the client that the requested result id is unknown or
    /// stale and that it must request a new full token response.
    #[must_use]
    pub fn semantic_tokens_delta(
        &self,
        uri: &str,
        previous_result_id: &str,
    ) -> Option<SemanticTokensDelta> {
        let data = self.semantic_token_data(uri);
        let result_id = semantic_token_result_id(&data);
        let mut snapshots = self.semantic_token_snapshots.borrow_mut();
        let previous_data = snapshots
            .get(uri)
            .filter(|snapshot| snapshot.result_id == previous_result_id)
            .map(|snapshot| snapshot.data.clone())?;
        let edits = semantic_token_delta_edits(&previous_data, &data);
        snapshots.insert(
            uri.to_owned(),
            SemanticTokenSnapshot {
                result_id: result_id.clone(),
                data,
            },
        );
        Some(SemanticTokensDelta { result_id, edits })
    }

    fn semantic_token_data(&self, uri: &str) -> Vec<u32> {
        let Some(context) = self.checked_output(uri) else {
            return Vec::new();
        };
        let output = &context.output;
        let source = &context.target_source;
        let Some(artifacts) = output.artifacts.as_ref() else {
            return Vec::new();
        };
        let mut entries = Vec::new();
        for token in &output.tokens {
            let token_type = match token.kind {
                TokenKind::Keyword(_) => Some(7),
                TokenKind::IntegerLiteral | TokenKind::FloatLiteral => Some(8),
                TokenKind::StringLiteral | TokenKind::CharLiteral => Some(9),
                TokenKind::LineComment
                | TokenKind::DocLineComment
                | TokenKind::BlockComment
                | TokenKind::DocBlockComment => Some(10),
                _ => None,
            };
            if let Some(token_type) = token_type {
                push_semantic_entry(&mut entries, token.span, token_type, 0, source);
            }
        }
        for symbol in &artifacts.resolution.symbols {
            if symbol.span.source == source.id()
                && let Some(token_type) = semantic_symbol_type(symbol.kind)
            {
                push_semantic_entry(&mut entries, symbol.span, token_type, 1, source);
            }
        }
        for reference in &artifacts.resolution.references {
            if let Some(symbol) = artifacts.resolution.symbol(reference.symbol)
                && let Some(token_type) = semantic_symbol_type(symbol.kind)
            {
                push_semantic_entry(&mut entries, reference.span, token_type, 1, source);
            }
        }
        entries.sort_by_key(|entry| {
            (
                entry.line,
                entry.start,
                entry.priority,
                entry.length,
                entry.token_type,
            )
        });
        entries.dedup_by(|left, right| {
            if left.line == right.line && left.start == right.start {
                if right.priority > left.priority {
                    *left = *right;
                }
                true
            } else {
                false
            }
        });
        let mut data = Vec::with_capacity(entries.len() * 5);
        let mut previous_line = 0;
        let mut previous_start = 0;
        for entry in entries {
            let delta_line = entry.line - previous_line;
            let delta_start = if delta_line == 0 {
                entry.start - previous_start
            } else {
                entry.start
            };
            data.extend([delta_line, delta_start, entry.length, entry.token_type, 0]);
            previous_line = entry.line;
            previous_start = entry.start;
        }
        data
    }

    /// Returns inferred type hints for bindings whose type was omitted.
    #[must_use]
    pub fn inlay_hints(&self, uri: &str) -> Vec<InlayHint> {
        let Some(context) = self.checked_output(uri) else {
            return Vec::new();
        };
        let output = &context.output;
        let source = &context.target_source;
        let Some(artifacts) = output.artifacts.as_ref() else {
            return Vec::new();
        };
        let mut hints = Vec::new();
        for item in &artifacts.ast.items {
            if let Item::Function(function) = item {
                collect_effect_hint(
                    function,
                    artifacts.effects.as_ref(),
                    &artifacts.resolution,
                    source,
                    &mut hints,
                );
                collect_binding_hints(
                    &function.body,
                    &artifacts.type_check,
                    &artifacts.resolution,
                    source,
                    &mut hints,
                );
            }
        }
        hints.sort_by_key(|hint| (hint.position.line, hint.position.character));
        hints
    }

    /// Returns inferred binding hints intersecting an LSP range.
    #[must_use]
    pub fn inlay_hints_in_range(&self, uri: &str, range: Range) -> Vec<InlayHint> {
        self.inlay_hints(uri)
            .into_iter()
            .filter(|hint| position_in_range(hint.position, range))
            .collect()
    }

    /// Returns deterministic allocation counts and result types per region.
    #[must_use]
    pub fn allocation_analytics(&self, uri: &str) -> Vec<AllocationAnalytics> {
        let Some(context) = self.checked_output(uri) else {
            return Vec::new();
        };
        let source = &context.target_source;
        let Some(artifacts) = context.output.artifacts.as_ref() else {
            return Vec::new();
        };
        let mut sites_by_region = BTreeMap::<SymbolId, Vec<TypeId>>::new();
        for site in &artifacts.type_check.region_allocations {
            if site.span.source == source.id() {
                sites_by_region
                    .entry(site.region)
                    .or_default()
                    .push(site.result_type);
            }
        }
        let mut summaries = sites_by_region
            .into_iter()
            .filter_map(|(region_id, types)| {
                let region = artifacts.resolution.symbol(region_id)?;
                if region.kind != SymbolKind::Region || region.span.source != source.id() {
                    return None;
                }
                let mut result_types = types
                    .iter()
                    .map(|type_id| format_type(&artifacts.type_check.types, *type_id))
                    .collect::<Vec<_>>();
                result_types.sort();
                result_types.dedup();
                Some(AllocationAnalytics {
                    region: region.name.clone(),
                    count: types.len() as u64,
                    result_types,
                    range: span_range(region.span, source),
                })
            })
            .collect::<Vec<_>>();
        summaries.sort_by_key(|summary| {
            (
                summary.range.start.line,
                summary.range.start.character,
                summary.region.clone(),
            )
        });
        summaries
    }

    /// Returns safe, query-backed quick-fixes intersecting an LSP range.
    #[must_use]
    pub fn code_actions(&self, uri: &str, range: Range) -> Vec<CodeAction> {
        self.code_actions_filtered(uri, range, None)
    }

    /// Returns quick-fixes filtered by the LSP `CodeActionContext.only` kinds.
    ///
    /// The unfiltered [`Self::code_actions`] API remains the compatibility
    /// path for hosts that do not send a context. A requested parent kind
    /// (for example `quickfix`) also matches a future child kind such as
    /// `quickfix.jadren`, while unsupported requested kinds return no edits.
    #[must_use]
    pub fn code_actions_filtered(
        &self,
        uri: &str,
        range: Range,
        only: Option<&[String]>,
    ) -> Vec<CodeAction> {
        let Some(context) = self.checked_output(uri) else {
            return Vec::new();
        };
        let source = &context.target_source;
        let mut actions = Vec::new();
        for diagnostic in &context.output.diagnostics {
            if diagnostic.primary.span.source != source.id()
                || !ranges_overlap(span_range(diagnostic.primary.span, source), range)
            {
                continue;
            }
            if diagnostic.code == "J0001"
                && diagnostic.help.as_deref()
                    == Some("remove the character or replace it with a valid token")
            {
                let mut changes = BTreeMap::new();
                changes.insert(
                    uri.to_owned(),
                    vec![TextEdit {
                        range: span_range(diagnostic.primary.span, source),
                        new_text: String::new(),
                    }],
                );
                actions.push(CodeAction {
                    title: "Odstrániť neplatný znak".to_owned(),
                    kind: "quickfix",
                    diagnostics: vec![map_diagnostic(diagnostic, source)],
                    edit: WorkspaceEdit { changes },
                    is_preferred: true,
                });
                continue;
            }
            if diagnostic.code == "J0002"
                && diagnostic.help.as_deref() == Some("add `*/` before the end of the file")
            {
                let diagnostic_range = span_range(diagnostic.primary.span, source);
                let mut changes = BTreeMap::new();
                changes.insert(
                    uri.to_owned(),
                    vec![TextEdit {
                        range: Range {
                            start: diagnostic_range.end,
                            end: diagnostic_range.end,
                        },
                        new_text: "*/".to_owned(),
                    }],
                );
                actions.push(CodeAction {
                    title: "Uzavrieť blokový komentár".to_owned(),
                    kind: "quickfix",
                    diagnostics: vec![map_diagnostic(diagnostic, source)],
                    edit: WorkspaceEdit { changes },
                    is_preferred: true,
                });
                continue;
            }
            if diagnostic.code == "J0003"
                && diagnostic.help.as_deref().is_some_and(|help| {
                    help.starts_with("add `") && help.ends_with("` before the end of the line")
                })
            {
                let quote = diagnostic
                    .help
                    .as_deref()
                    .and_then(|help| help.strip_prefix("add `"))
                    .and_then(|help| help.strip_suffix("` before the end of the line"));
                let Some(quote) = quote else {
                    continue;
                };
                let diagnostic_range = span_range(diagnostic.primary.span, source);
                let mut changes = BTreeMap::new();
                changes.insert(
                    uri.to_owned(),
                    vec![TextEdit {
                        range: Range {
                            start: diagnostic_range.end,
                            end: diagnostic_range.end,
                        },
                        new_text: quote.to_owned(),
                    }],
                );
                actions.push(CodeAction {
                    title: "Uzavrieť literál".to_owned(),
                    kind: "quickfix",
                    diagnostics: vec![map_diagnostic(diagnostic, source)],
                    edit: WorkspaceEdit { changes },
                    is_preferred: true,
                });
                continue;
            }
            if diagnostic.code == "J0005"
                && diagnostic.help.as_deref() == Some("use double quotes for a string literal")
            {
                let diagnostic_range = span_range(diagnostic.primary.span, source);
                let Some(literal) = source
                    .text()
                    .get(diagnostic.primary.span.start..diagnostic.primary.span.end)
                else {
                    continue;
                };
                let Some(new_text) = character_literal_to_string(literal) else {
                    continue;
                };
                let mut changes = BTreeMap::new();
                changes.insert(
                    uri.to_owned(),
                    vec![TextEdit {
                        range: diagnostic_range,
                        new_text,
                    }],
                );
                actions.push(CodeAction {
                    title: "Previesť znakový literál na reťazec".to_owned(),
                    kind: "quickfix",
                    diagnostics: vec![map_diagnostic(diagnostic, source)],
                    edit: WorkspaceEdit { changes },
                    is_preferred: true,
                });
                continue;
            }
            if diagnostic.code == "J0006"
                && diagnostic.help.as_deref()
                    == Some("remove repeated, leading, or trailing underscores")
            {
                let diagnostic_range = span_range(diagnostic.primary.span, source);
                let Some(literal) = source
                    .text()
                    .get(diagnostic.primary.span.start..diagnostic.primary.span.end)
                else {
                    continue;
                };
                let normalized = literal.replace('_', "");
                if normalized == literal || normalized.is_empty() {
                    continue;
                }
                let mut changes = BTreeMap::new();
                changes.insert(
                    uri.to_owned(),
                    vec![TextEdit {
                        range: diagnostic_range,
                        new_text: normalized,
                    }],
                );
                actions.push(CodeAction {
                    title: "Opraviť oddeľovače číselného literálu".to_owned(),
                    kind: "quickfix",
                    diagnostics: vec![map_diagnostic(diagnostic, source)],
                    edit: WorkspaceEdit { changes },
                    is_preferred: true,
                });
                continue;
            }
            if diagnostic.code == "J0104"
                && diagnostic.primary.message == "required punctuation is missing"
            {
                let Some(punctuation) = missing_punctuation_from_message(&diagnostic.message)
                else {
                    continue;
                };
                let diagnostic_range = span_range(diagnostic.primary.span, source);
                let insertion = Range {
                    start: diagnostic_range.start,
                    end: diagnostic_range.start,
                };
                let mut changes = BTreeMap::new();
                changes.insert(
                    uri.to_owned(),
                    vec![TextEdit {
                        range: insertion,
                        new_text: punctuation.to_owned(),
                    }],
                );
                actions.push(CodeAction {
                    title: format!("Doplniť chýbajúcu interpunkciu `{punctuation}`"),
                    kind: "quickfix",
                    diagnostics: vec![map_diagnostic(diagnostic, source)],
                    edit: WorkspaceEdit { changes },
                    is_preferred: true,
                });
                continue;
            }
            if diagnostic.code == "J0106"
                && diagnostic.primary.message == "required operator is missing"
            {
                let Some(operator) = missing_operator_from_message(&diagnostic.message) else {
                    continue;
                };
                let diagnostic_range = span_range(diagnostic.primary.span, source);
                let insertion = Range {
                    start: diagnostic_range.start,
                    end: diagnostic_range.start,
                };
                let mut changes = BTreeMap::new();
                changes.insert(
                    uri.to_owned(),
                    vec![TextEdit {
                        range: insertion,
                        new_text: operator.to_owned(),
                    }],
                );
                actions.push(CodeAction {
                    title: format!("Doplniť chýbajúci operátor `{operator}`"),
                    kind: "quickfix",
                    diagnostics: vec![map_diagnostic(diagnostic, source)],
                    edit: WorkspaceEdit { changes },
                    is_preferred: true,
                });
                continue;
            }
            if diagnostic.code == "J0103" && diagnostic.primary.message == "keyword is missing" {
                let Some(keyword) = missing_keyword_from_help(diagnostic.help.as_deref()) else {
                    continue;
                };
                let diagnostic_range = span_range(diagnostic.primary.span, source);
                let insertion = Range {
                    start: diagnostic_range.start,
                    end: diagnostic_range.start,
                };
                let mut changes = BTreeMap::new();
                changes.insert(
                    uri.to_owned(),
                    vec![TextEdit {
                        range: insertion,
                        new_text: format!("{keyword} "),
                    }],
                );
                actions.push(CodeAction {
                    title: format!("Doplniť chýbajúce kľúčové slovo `{keyword}`"),
                    kind: "quickfix",
                    diagnostics: vec![map_diagnostic(diagnostic, source)],
                    edit: WorkspaceEdit { changes },
                    is_preferred: true,
                });
                continue;
            }
            if diagnostic.code != "J0205" {
                continue;
            }
            let Some(declaration) = diagnostic
                .secondary
                .iter()
                .find(|label| context.sources.contains_key(&label.span.source))
            else {
                continue;
            };
            let Some(declaration_source) = context.sources.get(&declaration.span.source) else {
                continue;
            };
            let Some(edit_position) =
                visibility_edit_position(declaration_source, declaration.span)
            else {
                continue;
            };
            let Some(declaration_uri) = context.uris.get(&declaration.span.source) else {
                continue;
            };
            let edit = TextEdit {
                range: Range {
                    start: edit_position,
                    end: edit_position,
                },
                new_text: "pub ".to_owned(),
            };
            let mut changes = BTreeMap::new();
            changes.insert(declaration_uri.clone(), vec![edit]);
            actions.push(CodeAction {
                title: "Sprístupniť symbol cez `pub`".to_owned(),
                kind: "quickfix",
                diagnostics: vec![map_diagnostic(diagnostic, source)],
                edit: WorkspaceEdit { changes },
                is_preferred: true,
            });
        }
        let organize_imports_requested = only.is_none_or(|kinds| {
            kinds.is_empty()
                || kinds
                    .iter()
                    .any(|requested| code_action_kind_matches("source.organizeImports", requested))
        });
        if organize_imports_requested
            && let Some(action) = organize_imports_action(&context, uri, source, range)
        {
            actions.push(action);
        }
        let remove_unused_imports_requested = only.is_none_or(|kinds| {
            kinds.is_empty()
                || kinds.iter().any(|requested| {
                    code_action_kind_matches("source.removeUnusedImports", requested)
                })
        });
        if remove_unused_imports_requested
            && let Some(action) = remove_unused_imports_action(&context, uri, source, range)
        {
            actions.push(action);
        }
        let fix_all_requested = only.is_none_or(|kinds| {
            kinds.is_empty()
                || kinds
                    .iter()
                    .any(|requested| code_action_kind_matches("source.fixAll.jadren", requested))
        });
        if fix_all_requested
            && let Some(action) = source_fix_all_imports_action(&context, uri, source, range)
        {
            actions.push(action);
        }
        if let Some(only) = only
            && !only.is_empty()
        {
            actions
                .into_iter()
                .filter(|action| {
                    only.iter()
                        .any(|requested| code_action_kind_matches(action.kind, requested))
                })
                .collect()
        } else {
            actions
        }
    }

    fn checked_output(&self, uri: &str) -> Option<CheckedContext> {
        self.documents.get(uri)?;
        if let Some(cache) = self.cache.borrow().as_ref()
            && cache.documents == self.documents
        {
            return cache.checked_context(uri);
        }
        let mut session = CompilerSession::new(CompilerConfig::default());
        let mut source_ids = BTreeMap::new();
        for (document_uri, document) in &self.documents {
            let source_id = session
                .add_source(uri_to_path(document_uri), document.text.clone())
                .ok()?;
            source_ids.insert(document_uri.clone(), source_id);
        }
        let target_id = *source_ids.get(uri)?;
        let mut outputs = BTreeMap::new();
        for source_id in source_ids.values().copied() {
            if let Ok(output) = session.check(source_id) {
                outputs.insert(source_id, output);
            }
        }
        outputs.get(&target_id)?;
        let sources = session
            .sources()
            .iter()
            .map(|source| (source.id(), source.clone()))
            .collect();
        let uris = source_ids
            .into_iter()
            .map(|(document_uri, source_id)| (source_id, document_uri))
            .collect();
        let cache = WorkspaceCache {
            documents: self.documents.clone(),
            outputs,
            sources,
            uris,
        };
        let context = cache.checked_context(uri);
        *self.cache.borrow_mut() = Some(cache);
        context
    }
}

/// Runs the JSON-RPC/LSP stdio server until `exit` or EOF.
pub fn run_stdio() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    run_transport(stdin.lock(), stdout.lock())
}

/// Runs the framed transport over caller-provided streams (useful for hosts
/// and deterministic tests).
pub fn run_transport<R: BufRead, W: Write>(mut reader: R, mut writer: W) -> io::Result<()> {
    let mut server = LanguageServer::new();
    loop {
        let Some(payload) = read_message(&mut reader)? else {
            return Ok(());
        };
        let request: RpcRequest = match serde_json::from_slice(&payload) {
            Ok(request) => request,
            Err(error) => {
                write_message(
                    &mut writer,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": Value::Null,
                        "error": { "code": -32700, "message": error.to_string() }
                    }),
                )?;
                continue;
            }
        };
        let should_exit = request.method == "exit";
        let response = handle_request(&mut server, &request);
        if let Some(response) = response {
            write_message(&mut writer, &response)?;
        }
        if should_exit {
            return Ok(());
        }
    }
}

#[derive(Debug, Deserialize)]
struct RpcRequest {
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

fn handle_request(server: &mut LanguageServer, request: &RpcRequest) -> Option<Value> {
    match request.method.as_str() {
        "initialize" => response(
            request.id.clone(),
            json!({
                "capabilities": {
                    "textDocumentSync": 2,
                    "documentSymbolProvider": true,
                    "definitionProvider": true,
                    "referencesProvider": true,
                    "renameProvider": true,
                    "hoverProvider": true,
                    "completionProvider": { "triggerCharacters": [".", ":"] },
                    "inlayHintProvider": true,
                    "codeActionProvider": {
                        "codeActionKinds": [
                            "quickfix",
                            "source.organizeImports",
                            "source.removeUnusedImports",
                            "source.fixAll.jadren"
                        ]
                    },
                    "experimental": { "jadrenAllocationAnalytics": true },
                    "semanticTokensProvider": {
                        "legend": {
                            "tokenTypes": [
                                "namespace",
                                "type",
                                "function",
                                "parameter",
                                "variable",
                                "property",
                                "enumMember",
                                "keyword",
                                "number",
                                "string",
                                "comment"
                            ],
                            "tokenModifiers": []
                        },
                        "full": { "delta": true }
                    }
                },
                "serverInfo": { "name": "jadren-lsp", "version": env!("CARGO_PKG_VERSION") }
            }),
        ),
        "initialized" => None,
        "shutdown" => response(request.id.clone(), Value::Null),
        "exit" => None,
        "textDocument/didOpen" => {
            let params = request.params.as_ref()?.as_object()?;
            let text_document = params.get("textDocument")?.as_object()?;
            let uri = text_document.get("uri")?.as_str()?.to_owned();
            let version = text_document.get("version")?.as_i64().unwrap_or(0);
            let text = text_document.get("text")?.as_str()?.to_owned();
            let diagnostics = server.open_document(uri.clone(), version, text);
            Some(notification(
                "textDocument/publishDiagnostics",
                json!({ "uri": uri, "diagnostics": diagnostics }),
            ))
        }
        "textDocument/didChange" => {
            let params = request.params.as_ref()?.as_object()?;
            let text_document = params.get("textDocument")?.as_object()?;
            let uri = text_document.get("uri")?.as_str()?.to_owned();
            let version = text_document.get("version")?.as_i64().unwrap_or(0);
            let changes = serde_json::from_value::<Vec<TextDocumentContentChange>>(
                params.get("contentChanges")?.clone(),
            )
            .ok()?;
            let diagnostics = server.change_document_incremental(uri.clone(), version, &changes)?;
            Some(notification(
                "textDocument/publishDiagnostics",
                json!({ "uri": uri, "diagnostics": diagnostics }),
            ))
        }
        "textDocument/documentSymbol" => {
            let uri = request
                .params
                .as_ref()?
                .get("textDocument")?
                .get("uri")?
                .as_str()?;
            response(request.id.clone(), json!(server.document_symbols(uri)))
        }
        "textDocument/definition" => {
            let params = request.params.as_ref()?;
            let uri = params.get("textDocument")?.get("uri")?.as_str()?;
            let position =
                serde_json::from_value::<Position>(params.get("position")?.clone()).ok()?;
            response(request.id.clone(), json!(server.definition(uri, position)))
        }
        "textDocument/references" => {
            let params = request.params.as_ref()?;
            let uri = params.get("textDocument")?.get("uri")?.as_str()?;
            let position =
                serde_json::from_value::<Position>(params.get("position")?.clone()).ok()?;
            let include_declaration = params
                .get("context")
                .and_then(|context| context.get("includeDeclaration"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            response(
                request.id.clone(),
                json!(server.references(uri, position, include_declaration)),
            )
        }
        "textDocument/rename" => {
            let params = request.params.as_ref()?;
            let uri = params.get("textDocument")?.get("uri")?.as_str()?;
            let position =
                serde_json::from_value::<Position>(params.get("position")?.clone()).ok()?;
            let new_name = params.get("newName")?.as_str()?;
            let changes = server.rename(uri, position, new_name)?;
            response(request.id.clone(), json!({ "changes": changes }))
        }
        "textDocument/hover" => {
            let params = request.params.as_ref()?;
            let uri = params.get("textDocument")?.get("uri")?.as_str()?;
            let position =
                serde_json::from_value::<Position>(params.get("position")?.clone()).ok()?;
            response(request.id.clone(), json!(server.hover(uri, position)))
        }
        "textDocument/completion" => {
            let params = request.params.as_ref()?;
            let uri = params.get("textDocument")?.get("uri")?.as_str()?;
            let completions = params
                .get("position")
                .and_then(|value| serde_json::from_value::<Position>(value.clone()).ok())
                .map_or_else(
                    || server.completion(uri),
                    |position| server.completion_at(uri, position),
                );
            response(request.id.clone(), json!(completions))
        }
        "textDocument/semanticTokens/full" => {
            let uri = request
                .params
                .as_ref()?
                .get("textDocument")?
                .get("uri")?
                .as_str()?;
            response(request.id.clone(), json!(server.semantic_tokens_full(uri)))
        }
        "textDocument/semanticTokens/full/delta" => {
            let params = request.params.as_ref()?;
            let uri = params.get("textDocument")?.get("uri")?.as_str()?;
            let previous_result_id = params.get("previousResultId")?.as_str()?;
            response(
                request.id.clone(),
                json!(server.semantic_tokens_delta(uri, previous_result_id)),
            )
        }
        "textDocument/inlayHint" => {
            let params = request.params.as_ref()?;
            let uri = params.get("textDocument")?.get("uri")?.as_str()?;
            let range = serde_json::from_value::<Range>(params.get("range")?.clone()).ok()?;
            response(
                request.id.clone(),
                json!(server.inlay_hints_in_range(uri, range)),
            )
        }
        "textDocument/codeAction" => {
            let params = request.params.as_ref()?;
            let uri = params.get("textDocument")?.get("uri")?.as_str()?;
            let range = serde_json::from_value::<Range>(params.get("range")?.clone()).ok()?;
            let only = params
                .get("context")
                .and_then(|context| context.get("only"))
                .and_then(Value::as_array)
                .map(|kinds| {
                    kinds
                        .iter()
                        .filter_map(Value::as_str)
                        .map(ToOwned::to_owned)
                        .collect::<Vec<_>>()
                });
            response(
                request.id.clone(),
                json!(server.code_actions_filtered(uri, range, only.as_deref())),
            )
        }
        "jadren/allocationAnalytics" => {
            let uri = request
                .params
                .as_ref()?
                .get("textDocument")
                .and_then(|document| document.get("uri"))
                .or_else(|| request.params.as_ref()?.get("uri"))
                .and_then(Value::as_str)?;
            response(request.id.clone(), json!(server.allocation_analytics(uri)))
        }
        _ => request.id.clone().map(|id| {
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": "method not found" }
            })
        }),
    }
}

fn response(id: Option<Value>, result: Value) -> Option<Value> {
    id.map(|id| json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn notification(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "method": method, "params": params })
}

fn read_message<R: BufRead>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            content_length =
                Some(value.trim().parse::<usize>().map_err(|error| {
                    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
                })?);
        }
    }
    let length = content_length
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length"))?;
    let mut payload = vec![0_u8; length];
    reader.read_exact(&mut payload)?;
    Ok(Some(payload))
}

fn write_message<W: Write>(writer: &mut W, value: &Value) -> io::Result<()> {
    let payload = serde_json::to_vec(value)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    write!(writer, "Content-Length: {}\r\n\r\n", payload.len())?;
    writer.write_all(&payload)?;
    writer.flush()
}

fn map_diagnostic(diagnostic: &Diagnostic, source: &SourceFile) -> LspDiagnostic {
    LspDiagnostic {
        range: span_range(diagnostic.primary.span, source),
        severity: match diagnostic.severity {
            Severity::Error => 1,
            Severity::Warning => 2,
            Severity::Note => 3,
        },
        code: diagnostic.code.to_owned(),
        source: "jadren",
        message: diagnostic.message.clone(),
    }
}

fn map_item_symbol(item: &Item, source: &SourceFile) -> Option<DocumentSymbol> {
    let (name, name_span, item_span, kind) = match item {
        Item::Function(function) => (&function.name.text, function.name.span, function.span, 12),
        Item::ExternBlock(block) => {
            return Some(DocumentSymbol {
                name: "extern".to_owned(),
                kind: 2,
                range: span_range(block.span, source),
                selection_range: span_range(block.span, source),
                children: block
                    .functions
                    .iter()
                    .map(|function| DocumentSymbol {
                        name: function.name.text.clone(),
                        kind: 12,
                        range: span_range(function.span, source),
                        selection_range: span_range(function.name.span, source),
                        children: Vec::new(),
                    })
                    .collect(),
            });
        }
        Item::Struct(record) => (&record.name.text, record.name.span, record.span, 23),
        Item::Component(record) => (&record.name.text, record.name.span, record.span, 23),
        Item::Enum(enumeration) => (
            &enumeration.name.text,
            enumeration.name.span,
            enumeration.span,
            10,
        ),
    };
    Some(DocumentSymbol {
        name: name.clone(),
        kind,
        range: span_range(item_span, source),
        selection_range: span_range(name_span, source),
        children: Vec::new(),
    })
}

fn character_literal_to_string(literal: &str) -> Option<String> {
    let inner = literal.strip_prefix('\'')?.strip_suffix('\'')?;
    let mut converted = String::with_capacity(inner.len());
    let mut characters = inner.chars();
    while let Some(character) = characters.next() {
        if character == '\\' {
            let escaped = characters.next()?;
            if !matches!(escaped, 'n' | 'r' | 't' | '0' | '\\' | '"' | '\'') {
                return None;
            }
            converted.push('\\');
            converted.push(escaped);
        } else if character == '"' {
            return None;
        } else {
            converted.push(character);
        }
    }
    Some(format!("\"{converted}\""))
}

fn missing_punctuation_from_message(message: &str) -> Option<&'static str> {
    let expected = message.strip_prefix("expected `")?.split_once('`')?.0;
    match expected {
        "(" => Some("("),
        ")" => Some(")"),
        "{" => Some("{"),
        "}" => Some("}"),
        "[" => Some("["),
        "]" => Some("]"),
        "," => Some(","),
        "." => Some("."),
        ":" => Some(":"),
        ";" => Some(";"),
        "@" => Some("@"),
        _ => None,
    }
}

fn missing_operator_from_message(message: &str) -> Option<&'static str> {
    let expected = message.strip_prefix("expected `")?.split_once('`')?.0;
    match expected {
        ">" => Some(">"),
        "=>" => Some("=>"),
        _ => None,
    }
}

fn missing_keyword_from_help(help: Option<&str>) -> Option<&'static str> {
    let spelling = help?.strip_prefix("insert `")?.strip_suffix('`')?;
    match spelling {
        "as" => Some("as"),
        "break" => Some("break"),
        "component" => Some("component"),
        "const" => Some("const"),
        "continue" => Some("continue"),
        "else" => Some("else"),
        "enum" => Some("enum"),
        "extern" => Some("extern"),
        "false" => Some("false"),
        "fn" => Some("fn"),
        "for" => Some("for"),
        "if" => Some("if"),
        "import" => Some("import"),
        "in" => Some("in"),
        "let" => Some("let"),
        "match" => Some("match"),
        "module" => Some("module"),
        "mut" => Some("mut"),
        "owned" => Some("owned"),
        "panic" => Some("panic"),
        "pub" => Some("pub"),
        "read" => Some("read"),
        "region" => Some("region"),
        "result" => Some("result"),
        "return" => Some("return"),
        "shared" => Some("shared"),
        "struct" => Some("struct"),
        "trait" => Some("trait"),
        "true" => Some("true"),
        "type" => Some("type"),
        "unsafe" => Some("unsafe"),
        "var" => Some("var"),
        "where" => Some("where"),
        "while" => Some("while"),
        "write" => Some("write"),
        _ => None,
    }
}

fn span_range(span: Span, source: &SourceFile) -> Range {
    Range {
        start: position_at(source.text(), span.start),
        end: position_at(source.text(), span.end),
    }
}

fn typed_expression_hover(context: &CheckedContext, position: Position) -> Option<Hover> {
    let artifacts = context.output.artifacts.as_ref()?;
    let source = &context.target_source;
    let offset = position_to_offset(source.text(), position)?;
    let expression = artifacts
        .type_check
        .typed_expression_at(source.id(), offset)?;
    let type_name = format_type(&artifacts.type_check.types, expression.ty);
    Some(Hover {
        contents: MarkupContent {
            kind: "markdown",
            value: format!("Expression: `{:?}`\n\nType: `{type_name}`", expression.kind),
        },
        range: Some(span_range(expression.span, source)),
    })
}

fn code_action_kind_matches(action_kind: &str, requested_kind: &str) -> bool {
    action_kind == requested_kind
        || action_kind
            .strip_prefix(requested_kind)
            .is_some_and(|suffix| suffix.starts_with('.'))
}

fn organize_imports_action(
    context: &CheckedContext,
    uri: &str,
    source: &SourceFile,
    requested_range: Range,
) -> Option<CodeAction> {
    let lines = simple_import_lines(context, source)?;
    if lines.len() < 2 {
        return None;
    }
    let newline = import_block_newline(source.text(), &lines)?;
    let block_range = Range {
        start: position_at(source.text(), lines[0].start),
        end: position_at(source.text(), lines[lines.len() - 1].end),
    };
    if !ranges_overlap(block_range, requested_range) {
        return None;
    }
    let current_lines = lines
        .iter()
        .map(|line| line.text.clone())
        .collect::<Vec<_>>();
    let mut sorted_lines = current_lines.clone();
    sorted_lines.sort();
    if current_lines == sorted_lines {
        return None;
    }
    let mut changes = BTreeMap::new();
    changes.insert(
        uri.to_owned(),
        vec![TextEdit {
            range: block_range,
            new_text: sorted_lines.join(newline),
        }],
    );
    Some(CodeAction {
        title: "Usporiadať importy".to_owned(),
        kind: "source.organizeImports",
        diagnostics: Vec::new(),
        edit: WorkspaceEdit { changes },
        is_preferred: true,
    })
}

fn remove_unused_imports_action(
    context: &CheckedContext,
    uri: &str,
    source: &SourceFile,
    requested_range: Range,
) -> Option<CodeAction> {
    let lines = simple_import_lines(context, source)?;
    let artifacts = context.output.artifacts.as_ref()?;
    let resolution = &artifacts.resolution;
    let unused_paths = confirmed_unused_import_paths(&lines, resolution);
    if unused_paths.is_empty() {
        return None;
    }
    let block_range = Range {
        start: position_at(source.text(), lines[0].start),
        end: position_at(source.text(), lines[lines.len() - 1].end),
    };
    if !ranges_overlap(block_range, requested_range) {
        return None;
    }
    let newline = import_block_newline(source.text(), &lines).unwrap_or("\n");
    let remaining = lines
        .iter()
        .filter(|line| !unused_paths.contains(&line.path))
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>();
    let mut changes = BTreeMap::new();
    changes.insert(
        uri.to_owned(),
        vec![TextEdit {
            range: block_range,
            new_text: remaining.join(newline),
        }],
    );
    Some(CodeAction {
        title: "Odstrániť nepoužité importy".to_owned(),
        kind: "source.removeUnusedImports",
        diagnostics: Vec::new(),
        edit: WorkspaceEdit { changes },
        is_preferred: true,
    })
}

fn source_fix_all_imports_action(
    context: &CheckedContext,
    uri: &str,
    source: &SourceFile,
    requested_range: Range,
) -> Option<CodeAction> {
    let lines = simple_import_lines(context, source)?;
    let artifacts = context.output.artifacts.as_ref()?;
    let unused_paths = confirmed_unused_import_paths(&lines, &artifacts.resolution);
    let mut fixed_lines = lines
        .iter()
        .filter(|line| !unused_paths.contains(&line.path))
        .map(|line| line.text.clone())
        .collect::<Vec<_>>();
    fixed_lines.sort();
    let current_lines = lines
        .iter()
        .map(|line| line.text.clone())
        .collect::<Vec<_>>();
    if fixed_lines == current_lines {
        return None;
    }
    let block_range = Range {
        start: position_at(source.text(), lines[0].start),
        end: position_at(source.text(), lines[lines.len() - 1].end),
    };
    if !ranges_overlap(block_range, requested_range) {
        return None;
    }
    let newline = import_block_newline(source.text(), &lines).unwrap_or("\n");
    let mut changes = BTreeMap::new();
    changes.insert(
        uri.to_owned(),
        vec![TextEdit {
            range: block_range,
            new_text: fixed_lines.join(newline),
        }],
    );
    Some(CodeAction {
        title: "Opraviť bezpečné importy".to_owned(),
        kind: "source.fixAll.jadren",
        diagnostics: Vec::new(),
        edit: WorkspaceEdit { changes },
        is_preferred: true,
    })
}

fn confirmed_unused_import_paths(
    lines: &[SimpleImportLine],
    resolution: &ResolutionOutput,
) -> Vec<String> {
    lines
        .iter()
        .filter(|line| {
            let bindings = resolution
                .imports
                .iter()
                .filter(|binding| binding.path == line.path)
                .collect::<Vec<_>>();
            !bindings.is_empty()
                && !bindings
                    .iter()
                    .any(|binding| import_binding_used(binding, resolution))
        })
        .map(|line| line.path.clone())
        .collect()
}

#[derive(Clone, Debug)]
struct SimpleImportLine {
    start: usize,
    end: usize,
    path: String,
    text: String,
}

fn simple_import_lines(
    context: &CheckedContext,
    source: &SourceFile,
) -> Option<Vec<SimpleImportLine>> {
    let artifacts = context.output.artifacts.as_ref()?;
    if artifacts.ast.imports.is_empty() {
        return None;
    }
    let text = source.text();
    let mut lines = Vec::with_capacity(artifacts.ast.imports.len());
    for path in &artifacts.ast.imports {
        if path.span.source != source.id() {
            return None;
        }
        let path_text = path
            .segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<Vec<_>>()
            .join(".");
        if path_text.is_empty() {
            return None;
        }
        let line_start = text[..path.span.start]
            .rfind('\n')
            .map_or(0, |offset| offset + 1);
        let line_limit = text[path.span.end..]
            .find('\n')
            .map_or(text.len(), |offset| path.span.end + offset);
        let line_end = if line_limit > line_start && text.as_bytes()[line_limit - 1] == b'\r' {
            line_limit - 1
        } else {
            line_limit
        };
        let line = text.get(line_start..line_end)?;
        let expected = format!("import {path_text};");
        if line != expected {
            return None;
        }
        lines.push(SimpleImportLine {
            start: line_start,
            end: line_end,
            path: path_text,
            text: line.to_owned(),
        });
    }
    lines.sort_by_key(|line| line.start);
    if lines.windows(2).any(|pair| {
        text.get(pair[0].end..pair[1].start)
            .is_none_or(|separator| separator != "\n" && separator != "\r\n")
    }) {
        return None;
    }
    Some(lines)
}

fn import_block_newline<'a>(text: &'a str, lines: &[SimpleImportLine]) -> Option<&'a str> {
    let mut newline = None;
    for pair in lines.windows(2) {
        let separator = text.get(pair[0].end..pair[1].start)?;
        if separator != "\n" && separator != "\r\n" {
            return None;
        }
        if let Some(previous) = newline {
            if previous != separator {
                return None;
            }
        } else {
            newline = Some(separator);
        }
    }
    newline.or_else(|| {
        if text.contains("\r\n") {
            Some("\r\n")
        } else {
            Some("\n")
        }
    })
}

fn import_binding_used(
    binding: &jadren_resolve::ImportBinding,
    resolution: &ResolutionOutput,
) -> bool {
    let module_prefix = format!("{}.", binding.canonical_path);
    resolution.references.iter().any(|reference| {
        if reference.symbol == binding.symbol {
            return true;
        }
        binding.namespace == Namespace::Module
            && resolution
                .symbol(reference.symbol)
                .and_then(|symbol| symbol.canonical_path.as_deref())
                .is_some_and(|path| path.starts_with(&module_prefix))
    })
}

fn ranges_overlap(left: Range, right: Range) -> bool {
    if left.start == left.end {
        return !position_before(left.start, right.start)
            && !position_before(right.end, left.start);
    }
    if right.start == right.end {
        return !position_before(right.start, left.start)
            && !position_before(left.end, right.start);
    }
    position_before(left.start, right.end) && position_before(right.start, left.end)
}

fn position_before(left: Position, right: Position) -> bool {
    (left.line, left.character) < (right.line, right.character)
}

fn visibility_edit_position(source: &SourceFile, declaration: Span) -> Option<Position> {
    if declaration.source != source.id() || declaration.start > source.text().len() {
        return None;
    }
    let line_start = source.text()[..declaration.start]
        .rfind('\n')
        .map_or(0, |offset| offset + 1);
    let prefix = &source.text()[line_start..declaration.start];
    let trimmed = prefix.trim_start();
    let declaration_keywords = ["fn ", "struct ", "component ", "enum "];
    let keyword_offset = declaration_keywords
        .iter()
        .filter_map(|keyword| {
            trimmed
                .find(keyword)
                .map(|offset| offset + prefix.len() - trimmed.len())
        })
        .min();
    let offset = keyword_offset.map_or(declaration.start, |offset| line_start + offset);
    Some(position_at(source.text(), offset))
}

#[derive(Clone, Copy, Debug)]
struct SemanticTokenEntry {
    line: u32,
    start: u32,
    length: u32,
    token_type: u32,
    priority: u8,
}

fn semantic_symbol_type(kind: SymbolKind) -> Option<u32> {
    match kind {
        SymbolKind::Module => Some(0),
        SymbolKind::BuiltinType
        | SymbolKind::Struct
        | SymbolKind::Component
        | SymbolKind::Enum
        | SymbolKind::GenericParameter => Some(1),
        SymbolKind::Function | SymbolKind::BuiltinValue => Some(2),
        SymbolKind::Parameter => Some(3),
        SymbolKind::Local | SymbolKind::Region => Some(4),
        SymbolKind::Field => Some(5),
        SymbolKind::EnumVariant => Some(6),
        SymbolKind::BuiltinTrait => Some(1),
    }
}

fn push_semantic_entry(
    entries: &mut Vec<SemanticTokenEntry>,
    span: Span,
    token_type: u32,
    priority: u8,
    source: &SourceFile,
) {
    if span.source != source.id() {
        return;
    }
    let range = span_range(span, source);
    if range.start.line != range.end.line || range.end.character <= range.start.character {
        return;
    }
    entries.push(SemanticTokenEntry {
        line: range.start.line,
        start: range.start.character,
        length: range.end.character - range.start.character,
        token_type,
        priority,
    });
}

fn semantic_token_result_id(data: &[u32]) -> String {
    let mut hasher = StableHasher::with_domain("jadren-lsp-semantic-tokens-v1");
    hasher.write_u64(data.len() as u64);
    for value in data {
        hasher.write_u64(u64::from(*value));
    }
    format!("v1-{}", hasher.finish())
}

fn semantic_token_delta_edits(old: &[u32], new: &[u32]) -> Vec<SemanticTokensEdit> {
    let mut prefix = 0;
    while prefix < old.len() && prefix < new.len() && old[prefix] == new[prefix] {
        prefix += 1;
    }
    if prefix == old.len() && prefix == new.len() {
        return Vec::new();
    }
    let mut suffix = 0;
    while old.len() > prefix + suffix
        && new.len() > prefix + suffix
        && old[old.len() - suffix - 1] == new[new.len() - suffix - 1]
    {
        suffix += 1;
    }
    vec![SemanticTokensEdit {
        start: prefix as u32,
        delete_count: (old.len() - prefix - suffix) as u32,
        data: new[prefix..new.len() - suffix].to_vec(),
    }]
}

fn location_for_span(context: &CheckedContext, span: Span) -> Option<Location> {
    Some(Location {
        uri: context.uris.get(&span.source)?.clone(),
        range: span_range(span, context.sources.get(&span.source)?),
    })
}

fn position_at(text: &str, offset: usize) -> Position {
    let offset = offset.min(text.len());
    let mut line = 0_u32;
    let mut character = 0_u32;
    let mut index = 0;
    for value in text[..offset].chars() {
        if value == '\n' {
            line = line.saturating_add(1);
            character = 0;
        } else {
            character = character.saturating_add(value.len_utf16() as u32);
        }
        index += value.len_utf8();
        if index >= offset {
            break;
        }
    }
    Position { line, character }
}

fn position_in_range(position: Position, range: Range) -> bool {
    let key = |value: Position| (value.line, value.character);
    key(range.start) <= key(position) && key(position) <= key(range.end)
}

fn position_to_offset(text: &str, target: Position) -> Option<usize> {
    let mut line = 0_u32;
    let mut character = 0_u32;
    for (offset, value) in text.char_indices() {
        if line == target.line {
            if character == target.character {
                return Some(offset);
            }
            if value == '\n' {
                return None;
            }
            let next = character.saturating_add(value.len_utf16() as u32);
            if target.character < next {
                return None;
            }
            character = next;
        } else if value == '\n' {
            line = line.saturating_add(1);
            character = 0;
        }
    }
    (line == target.line && character == target.character).then_some(text.len())
}

fn identifier_prefix_start(text: &str, offset: usize) -> usize {
    let mut start = offset;
    while start > 0 {
        let Some((index, character)) = text[..start].char_indices().next_back() else {
            break;
        };
        if character != '_' && !character.is_alphanumeric() {
            break;
        }
        start = index;
    }
    start
}

fn identifier_span_at(text: &str, offset: usize) -> Option<(usize, usize)> {
    if offset > text.len() {
        return None;
    }
    let mut start = offset;
    if start == text.len()
        || text[start..]
            .chars()
            .next()
            .is_none_or(|character| !is_identifier_character(character))
    {
        if start == 0 {
            return None;
        }
        let (_, previous) = text[..start].char_indices().next_back()?;
        if !is_identifier_character(previous) {
            return None;
        }
    }
    while start > 0 {
        let Some((index, character)) = text[..start].char_indices().next_back() else {
            break;
        };
        if !is_identifier_character(character) {
            break;
        }
        start = index;
    }
    let mut end = offset;
    while end < text.len() {
        let Some(character) = text[end..].chars().next() else {
            break;
        };
        if !is_identifier_character(character) {
            break;
        }
        end += character.len_utf8();
    }
    (start < end).then_some((start, end))
}

fn is_identifier_character(character: char) -> bool {
    character == '_' || character.is_alphanumeric()
}

fn lexical_scope_at_offset(
    resolution: &ResolutionOutput,
    source: &SourceFile,
    offset: usize,
) -> ScopeId {
    resolution
        .scopes
        .iter()
        .filter(|scope| {
            scope.span.source == source.id()
                && offset >= scope.span.start
                && offset <= scope.span.end
        })
        .min_by_key(|scope| scope.span.len())
        .map(|scope| scope.id)
        .unwrap_or(resolution.root_scope)
}

fn symbol_at_position(
    resolution: Option<&ResolutionOutput>,
    source: &SourceFile,
    position: Position,
) -> Option<SymbolId> {
    let offset = position_to_offset(source.text(), position)?;
    let resolution = resolution?;
    if let Some(reference) = resolution
        .references
        .iter()
        .find(|reference| span_contains(reference.span, offset))
    {
        return Some(reference.symbol);
    }
    resolution
        .symbols
        .iter()
        .filter(|symbol| {
            symbol.origin == SymbolOrigin::Source
                && symbol.span.source == source.id()
                && span_contains(symbol.span, offset)
        })
        .min_by_key(|symbol| symbol.span.len())
        .map(|symbol| symbol.id)
}

fn span_contains(span: Span, offset: usize) -> bool {
    if span.is_empty() {
        offset == span.start
    } else {
        offset >= span.start && offset <= span.end
    }
}

fn valid_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_alphabetic())
        && chars.all(|character| character == '_' || character.is_alphanumeric())
}

fn is_source_bound_completion_symbol(symbol: &Symbol, source: &SourceFile) -> bool {
    matches!(symbol.origin, SymbolOrigin::Source | SymbolOrigin::Imported)
        && symbol.span.source == source.id()
}

fn qualified_module_completion_path(
    text: &str,
    prefix_start: usize,
    resolution: &ResolutionOutput,
    current_scope: ScopeId,
) -> Option<String> {
    qualified_alias_completion_path(
        text,
        prefix_start,
        resolution,
        current_scope,
        Namespace::Module,
    )
}

fn qualified_type_completion_path(
    text: &str,
    prefix_start: usize,
    resolution: &ResolutionOutput,
    current_scope: ScopeId,
) -> Option<String> {
    qualified_alias_completion_path(
        text,
        prefix_start,
        resolution,
        current_scope,
        Namespace::Type,
    )
}

fn qualified_alias_completion_path(
    text: &str,
    prefix_start: usize,
    resolution: &ResolutionOutput,
    current_scope: ScopeId,
    namespace: Namespace,
) -> Option<String> {
    if prefix_start == 0 || text.as_bytes().get(prefix_start - 1) != Some(&b'.') {
        return None;
    }
    let path_end = prefix_start - 1;
    let mut path_start = path_end;
    while path_start > 0 {
        let Some((index, character)) = text[..path_start].char_indices().next_back() else {
            break;
        };
        if character == '.' || character == '_' || character.is_alphanumeric() {
            path_start = index;
        } else {
            break;
        }
    }
    let path = &text[path_start..path_end];
    let segments = path.split('.').collect::<Vec<_>>();
    if segments.is_empty() || segments.iter().any(|segment| !valid_identifier(segment)) {
        return None;
    }
    let alias = segments[0];
    let mut scope_id = Some(current_scope);
    let alias_symbol = loop {
        let scope = resolution.scope(scope_id?)?;
        let symbol = scope.symbols.values().find_map(|symbol_id| {
            let symbol = resolution.symbol(*symbol_id)?;
            (symbol.namespace == namespace && symbol.name == alias).then_some(symbol)
        });
        if symbol.is_some() {
            break symbol;
        }
        scope_id = scope.parent;
    }?;
    let mut module_path = alias_symbol.canonical_path.clone()?;
    if segments.len() > 1 {
        module_path.push('.');
        module_path.push_str(&segments[1..].join("."));
    }
    Some(module_path)
}

fn add_enum_variant_completion_candidates(
    candidates: &mut BTreeMap<String, CompletionItem>,
    prefix: &str,
    type_path: &str,
    resolution: &ResolutionOutput,
    outputs: &BTreeMap<SourceId, jadren_driver::CheckOutput>,
) {
    for symbol in &resolution.symbols {
        if symbol.canonical_path.as_deref() != Some(type_path) {
            continue;
        }
        if let Some(interface) = symbol.enum_interface.as_ref() {
            add_enum_interface_variants(candidates, prefix, interface);
        }
    }
    for output in outputs.values() {
        let Some(artifacts) = output.artifacts.as_ref() else {
            continue;
        };
        for symbol in &artifacts.resolution.symbols {
            if symbol.canonical_path.as_deref() != Some(type_path) {
                continue;
            }
            if let Some(interface) = symbol.enum_interface.as_ref() {
                add_enum_interface_variants(candidates, prefix, interface);
            }
        }
    }
    for symbol in &resolution.symbols {
        if symbol.kind != SymbolKind::EnumVariant || symbol.origin != SymbolOrigin::Source {
            continue;
        }
        let Some(owner) = symbol.owner.and_then(|owner| resolution.symbol(owner)) else {
            continue;
        };
        if owner.canonical_path.as_deref() != Some(type_path) || !symbol.name.starts_with(prefix) {
            continue;
        }
        candidates
            .entry(symbol.name.clone())
            .or_insert_with(|| CompletionItem {
                label: symbol.name.clone(),
                kind: lsp_symbol_kind(SymbolKind::EnumVariant),
                detail: Some("EnumVariant".to_owned()),
            });
    }
}

fn add_enum_interface_variants(
    candidates: &mut BTreeMap<String, CompletionItem>,
    prefix: &str,
    interface: &ModuleEnumInterface,
) {
    for variant in &interface.variants {
        if !variant.name.starts_with(prefix) {
            continue;
        }
        candidates
            .entry(variant.name.clone())
            .or_insert_with(|| CompletionItem {
                label: variant.name.clone(),
                kind: lsp_symbol_kind(SymbolKind::EnumVariant),
                detail: Some("EnumVariant".to_owned()),
            });
    }
}

fn enum_variant_hover(context: &CheckedContext, position: Position) -> Option<Hover> {
    let source = &context.target_source;
    let offset = position_to_offset(source.text(), position)?;
    let (variant_start, variant_end) = identifier_span_at(source.text(), offset)?;
    let artifacts = context.output.artifacts.as_ref()?;
    let scope = lexical_scope_at_offset(&artifacts.resolution, source, offset);
    let type_path =
        qualified_type_completion_path(source.text(), variant_start, &artifacts.resolution, scope)?;
    let variant_name = &source.text()[variant_start..variant_end];
    let payload_count = enum_variant_payload_count(
        &artifacts.resolution,
        &context.outputs,
        &type_path,
        variant_name,
    )?;
    let mut value = format!("`EnumVariant` **{variant_name}**\n\nType: `{type_path}`");
    if payload_count != 0 {
        value.push_str(&format!("\n\nPayloads: `{payload_count}`"));
    }
    Some(Hover {
        contents: MarkupContent {
            kind: "markdown",
            value,
        },
        range: Some(Range {
            start: position_at(source.text(), variant_start),
            end: position_at(source.text(), variant_end),
        }),
    })
}

fn enum_variant_payload_count(
    resolution: &ResolutionOutput,
    outputs: &BTreeMap<SourceId, jadren_driver::CheckOutput>,
    type_path: &str,
    variant_name: &str,
) -> Option<usize> {
    let interface_count = |resolution: &ResolutionOutput| {
        resolution.symbols.iter().find_map(|symbol| {
            (symbol.canonical_path.as_deref() == Some(type_path))
                .then_some(symbol.enum_interface.as_ref())
                .flatten()
                .and_then(|interface| {
                    interface
                        .variants
                        .iter()
                        .find(|variant| variant.name == variant_name)
                        .map(|variant| variant.fields.len())
                })
        })
    };
    if let Some(count) = interface_count(resolution) {
        return Some(count);
    }
    for output in outputs.values() {
        let Some(artifacts) = output.artifacts.as_ref() else {
            continue;
        };
        if let Some(count) = interface_count(&artifacts.resolution) {
            return Some(count);
        }
        if artifacts.resolution.symbols.iter().any(|symbol| {
            symbol.kind == SymbolKind::EnumVariant
                && symbol.origin == SymbolOrigin::Source
                && symbol.name == variant_name
                && symbol
                    .owner
                    .and_then(|owner| artifacts.resolution.symbol(owner))
                    .and_then(|owner| owner.canonical_path.as_deref())
                    == Some(type_path)
        }) {
            return Some(0);
        }
    }
    if resolution.symbols.iter().any(|symbol| {
        symbol.kind == SymbolKind::EnumVariant
            && symbol.origin == SymbolOrigin::Source
            && symbol.name == variant_name
            && symbol
                .owner
                .and_then(|owner| resolution.symbol(owner))
                .and_then(|owner| owner.canonical_path.as_deref())
                == Some(type_path)
    }) {
        Some(0)
    } else {
        None
    }
}

fn record_field_hover(context: &CheckedContext, position: Position) -> Option<Hover> {
    let source = &context.target_source;
    let offset = position_to_offset(source.text(), position)?;
    let (field_start, field_end) = identifier_span_at(source.text(), offset)?;
    let artifacts = context.output.artifacts.as_ref()?;
    let constructor =
        nominal_field_completion_constructor(&artifacts.type_check, source, field_start)?;
    let field_name = &source.text()[field_start..field_end];
    if !record_field_is_visible(
        constructor,
        field_name,
        &artifacts.resolution,
        &context.outputs,
        artifacts.resolution.module_name.as_deref(),
    ) {
        return None;
    }
    let field_type = artifacts
        .type_check
        .expressions
        .iter()
        .filter(|expression| {
            expression.span.source == source.id()
                && expression.span.start < field_start
                && expression.span.end >= field_end
        })
        .min_by_key(|expression| expression.span.len())
        .map(|expression| expression.ty)?;
    Some(Hover {
        contents: MarkupContent {
            kind: "markdown",
            value: format!(
                "`Field` **{field_name}**\n\nType: `{}`",
                format_type(&artifacts.type_check.types, field_type)
            ),
        },
        range: Some(Range {
            start: position_at(source.text(), field_start),
            end: position_at(source.text(), field_end),
        }),
    })
}

fn record_field_is_visible(
    constructor: NominalTypeId,
    field_name: &str,
    resolution: &ResolutionOutput,
    outputs: &BTreeMap<SourceId, jadren_driver::CheckOutput>,
    current_module: Option<&str>,
) -> bool {
    for symbol in &resolution.symbols {
        if symbol.kind != SymbolKind::Field
            || symbol.name != field_name
            || symbol
                .owner
                .and_then(|owner| resolution.symbol(owner))
                .and_then(Symbol::nominal_type_id)
                != Some(constructor)
        {
            continue;
        }
        return symbol.visibility == DeclaredVisibility::Public
            || same_module_as_owner(resolution, symbol, current_module);
    }
    for symbol in &resolution.symbols {
        if symbol.nominal_type_id() != Some(constructor) {
            continue;
        }
        if let Some(interface) = symbol.record_interface.as_ref()
            && let Some(field) = interface
                .fields
                .iter()
                .find(|field| field.name == field_name)
        {
            return field.visibility == DeclaredVisibility::Public
                || current_module == Some(interface.module_name.as_str());
        }
    }
    for output in outputs.values() {
        let Some(artifacts) = output.artifacts.as_ref() else {
            continue;
        };
        for symbol in &artifacts.resolution.symbols {
            if symbol.nominal_type_id() != Some(constructor) {
                continue;
            }
            if let Some(interface) = symbol.record_interface.as_ref()
                && let Some(field) = interface
                    .fields
                    .iter()
                    .find(|field| field.name == field_name)
            {
                return field.visibility == DeclaredVisibility::Public
                    || current_module == Some(interface.module_name.as_str());
            }
        }
    }
    false
}

fn nominal_field_completion_constructor(
    type_check: &jadren_typeck::TypeCheckOutput,
    source: &SourceFile,
    prefix_start: usize,
) -> Option<NominalTypeId> {
    let dot_offset = prefix_start.checked_sub(1)?;
    if source.text().as_bytes().get(dot_offset) != Some(&b'.') {
        return None;
    }
    let base_type = type_check
        .expressions
        .iter()
        .filter(|expression| {
            expression.span.source == source.id() && expression.span.end == dot_offset
        })
        .max_by_key(|expression| expression.span.start)
        .map(|expression| expression.ty)?;
    let mut resolved = base_type;
    while let Some(TypeKind::Capability { inner, .. }) = type_check.types.kind(resolved) {
        resolved = *inner;
    }
    match type_check.types.kind(resolved) {
        Some(TypeKind::Nominal { constructor, .. }) => Some(*constructor),
        _ => None,
    }
}

fn add_record_field_completion_candidates(
    candidates: &mut BTreeMap<String, CompletionItem>,
    prefix: &str,
    constructor: NominalTypeId,
    resolution: &ResolutionOutput,
    outputs: &BTreeMap<SourceId, jadren_driver::CheckOutput>,
    current_module: Option<&str>,
) {
    for symbol in &resolution.symbols {
        if symbol.nominal_type_id() != Some(constructor) {
            continue;
        }
        if let Some(interface) = symbol.record_interface.as_ref() {
            add_record_interface_fields(candidates, prefix, interface, current_module);
        }
    }
    for output in outputs.values() {
        let Some(artifacts) = output.artifacts.as_ref() else {
            continue;
        };
        for symbol in &artifacts.resolution.symbols {
            if symbol.nominal_type_id() != Some(constructor) {
                continue;
            }
            if let Some(interface) = symbol.record_interface.as_ref() {
                add_record_interface_fields(candidates, prefix, interface, current_module);
            }
        }
    }
    for symbol in &resolution.symbols {
        if symbol.kind != SymbolKind::Field
            || symbol.origin != SymbolOrigin::Source
            || symbol
                .owner
                .and_then(|owner| resolution.symbol(owner))
                .and_then(Symbol::nominal_type_id)
                != Some(constructor)
        {
            continue;
        }
        if symbol.visibility == DeclaredVisibility::Private
            && !same_module_as_owner(resolution, symbol, current_module)
        {
            continue;
        }
        if !symbol.name.starts_with(prefix) {
            continue;
        }
        candidates
            .entry(symbol.name.clone())
            .or_insert_with(|| CompletionItem {
                label: symbol.name.clone(),
                kind: lsp_symbol_kind(SymbolKind::Field),
                detail: Some("Field".to_owned()),
            });
    }
}

fn add_record_interface_fields(
    candidates: &mut BTreeMap<String, CompletionItem>,
    prefix: &str,
    interface: &ModuleRecordInterface,
    current_module: Option<&str>,
) {
    let same_module = current_module == Some(interface.module_name.as_str());
    for field in &interface.fields {
        if field.visibility == DeclaredVisibility::Private && !same_module {
            continue;
        }
        if !field.name.starts_with(prefix) {
            continue;
        }
        candidates
            .entry(field.name.clone())
            .or_insert_with(|| CompletionItem {
                label: field.name.clone(),
                kind: lsp_symbol_kind(SymbolKind::Field),
                detail: Some("Field".to_owned()),
            });
    }
}

fn same_module_as_owner(
    resolution: &ResolutionOutput,
    field: &Symbol,
    current_module: Option<&str>,
) -> bool {
    let Some(owner) = field.owner.and_then(|owner| resolution.symbol(owner)) else {
        return current_module.is_none();
    };
    let Some(path) = owner.canonical_path.as_deref() else {
        return current_module.is_none();
    };
    let Some((module, _)) = path.rsplit_once('.') else {
        return current_module.is_none();
    };
    current_module == Some(module)
}

fn lsp_symbol_kind(kind: SymbolKind) -> u32 {
    match kind {
        SymbolKind::Module => 2,
        SymbolKind::Function => 12,
        SymbolKind::Struct | SymbolKind::Component => 23,
        SymbolKind::Enum => 10,
        SymbolKind::EnumVariant => 22,
        SymbolKind::Parameter | SymbolKind::Local | SymbolKind::Region => 13,
        SymbolKind::Field => 8,
        SymbolKind::GenericParameter => 26,
        SymbolKind::BuiltinType | SymbolKind::BuiltinTrait => 26,
        SymbolKind::BuiltinValue => 14,
    }
}

fn format_type(store: &TypeStore, id: TypeId) -> String {
    format_type_at(store, id, 0)
}

fn format_module_signature(signature: &ModuleFunctionSignature) -> String {
    let generic_parameters = if signature.generic_count == 0 {
        String::new()
    } else {
        let parameters = (0..signature.generic_count)
            .map(|index| {
                let bounds = signature
                    .generic_bounds
                    .get(index)
                    .into_iter()
                    .flatten()
                    .map(|bound| bound.name())
                    .collect::<Vec<_>>()
                    .join(" + ");
                if bounds.is_empty() {
                    format!("T{index}")
                } else {
                    format!("T{index}: {bounds}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("<{parameters}>")
    };
    let parameters = signature
        .parameters
        .iter()
        .map(format_module_type)
        .collect::<Vec<_>>()
        .join(", ");
    let prefix = if signature.is_unsafe { "unsafe " } else { "" };
    let abi = signature
        .extern_abi
        .as_deref()
        .map_or(String::new(), |abi| format!("extern \"{abi}\" "));
    format!(
        "{abi}{prefix}fn{generic_parameters}({parameters}) -> {}",
        format_module_type(&signature.result)
    )
}

fn format_module_type(ty: &ModuleType) -> String {
    match ty {
        ModuleType::Builtin { name, arguments } => {
            if arguments.is_empty() {
                name.clone()
            } else {
                format!(
                    "{}<{}>",
                    name,
                    arguments
                        .iter()
                        .map(format_module_type)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        }
        ModuleType::Nominal {
            canonical_path,
            arguments,
        } => {
            if arguments.is_empty() {
                canonical_path.clone()
            } else {
                format!(
                    "{}<{}>",
                    canonical_path,
                    arguments
                        .iter()
                        .map(format_module_type)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        }
        ModuleType::GenericParameter(index) => format!("T{index}"),
        ModuleType::Array { element, length } => {
            format!("[{}; {length}]", format_module_type(element))
        }
        ModuleType::Capability { capability, inner } => {
            let capability = match capability {
                Capability::Owned => "owned",
                Capability::Read => "read",
                Capability::Write => "write",
            };
            format!("{capability} {}", format_module_type(inner))
        }
        ModuleType::Function { parameters, result } => format!(
            "fn({}) -> {}",
            parameters
                .iter()
                .map(format_module_type)
                .collect::<Vec<_>>()
                .join(", "),
            format_module_type(result)
        ),
        ModuleType::Error => "Error".to_owned(),
    }
}

fn format_type_at(store: &TypeStore, id: TypeId, depth: usize) -> String {
    if depth >= 8 {
        return "…".to_owned();
    }
    let Some(kind) = store.kind(id) else {
        return "unknown".to_owned();
    };
    match kind {
        TypeKind::Error => "Error".to_owned(),
        TypeKind::Bool => "Bool".to_owned(),
        TypeKind::Char => "Char".to_owned(),
        TypeKind::String => "String".to_owned(),
        TypeKind::Unit => "Unit".to_owned(),
        TypeKind::Never => "Never".to_owned(),
        TypeKind::Integer { signedness, width } => {
            let prefix = match signedness {
                Signedness::Signed => "Int",
                Signedness::Unsigned => "UInt",
            };
            let width = match width {
                IntegerWidth::Bits8 => "8",
                IntegerWidth::Bits16 => "16",
                IntegerWidth::Bits32 => "32",
                IntegerWidth::Bits64 => "64",
                IntegerWidth::Pointer => "Ptr",
            };
            format!("{prefix}{width}")
        }
        TypeKind::Float(width) => match width {
            FloatWidth::Bits16 => "Float16".to_owned(),
            FloatWidth::Bits32 => "Float32".to_owned(),
            FloatWidth::Bits64 => "Float64".to_owned(),
        },
        TypeKind::Vector { element, lanes } => {
            if *element == store.core().float32 {
                match lanes {
                    2 => "Float2".to_owned(),
                    3 => "Float3".to_owned(),
                    4 => "Float4".to_owned(),
                    8 => "Float8".to_owned(),
                    _ => format!(
                        "Vector<{}, {lanes}>",
                        format_type_at(store, *element, depth + 1)
                    ),
                }
            } else {
                format!(
                    "Vector<{}, {lanes}>",
                    format_type_at(store, *element, depth + 1)
                )
            }
        }
        TypeKind::Array { element, length } => {
            format!("[{}; {length}]", format_type_at(store, *element, depth + 1))
        }
        TypeKind::Buffer(element) => {
            format!("Buffer<{}>", format_type_at(store, *element, depth + 1))
        }
        TypeKind::Slice(element) => {
            format!("Slice<{}>", format_type_at(store, *element, depth + 1))
        }
        TypeKind::Pointer(element) => {
            format!("*{}", format_type_at(store, *element, depth + 1))
        }
        TypeKind::Option(inner) => {
            format!("Option<{}>", format_type_at(store, *inner, depth + 1))
        }
        TypeKind::Result { ok, error } => format!(
            "Result<{}, {}>",
            format_type_at(store, *ok, depth + 1),
            format_type_at(store, *error, depth + 1)
        ),
        TypeKind::Nominal { arguments, .. } => {
            if arguments.is_empty() {
                "Nominal".to_owned()
            } else {
                let arguments = arguments
                    .iter()
                    .map(|argument| format_type_at(store, *argument, depth + 1))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("Nominal<{arguments}>")
            }
        }
        TypeKind::GenericParameter(_) => "T".to_owned(),
        TypeKind::InferenceVariable(_) => "_".to_owned(),
        TypeKind::Function { parameters, result } => {
            let parameters = parameters
                .iter()
                .map(|parameter| format_type_at(store, *parameter, depth + 1))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "fn({parameters}) -> {}",
                format_type_at(store, *result, depth + 1)
            )
        }
        TypeKind::Capability { capability, inner } => {
            let capability = match capability {
                Capability::Owned => "owned",
                Capability::Read => "read",
                Capability::Write => "write",
            };
            format!("{capability} {}", format_type_at(store, *inner, depth + 1))
        }
    }
}

fn collect_binding_hints(
    block: &Block,
    type_check: &jadren_typeck::TypeCheckOutput,
    resolution: &ResolutionOutput,
    source: &SourceFile,
    hints: &mut Vec<InlayHint>,
) {
    for statement in &block.statements {
        match statement {
            Statement::Binding {
                ty: None,
                name,
                value: Some(value),
                ..
            } => {
                if let Some(expression) = type_check.typed_expression_exact_span(value.span()) {
                    let type_name = type_check.types.kind(expression.ty).map_or_else(
                        || "unknown".to_owned(),
                        |_| format_type(&type_check.types, expression.ty),
                    );
                    hints.push(InlayHint {
                        position: position_at(source.text(), name.span.end),
                        label: format!(": {type_name}"),
                        kind: 1,
                        padding_left: true,
                    });
                }
                collect_expression_hints(value, type_check, resolution, source, hints);
            }
            Statement::Binding {
                value: Some(value), ..
            }
            | Statement::Expression {
                expression: value, ..
            } => collect_expression_hints(value, type_check, resolution, source, hints),
            Statement::Region { body, .. } => {
                collect_binding_hints(body, type_check, resolution, source, hints);
            }
            Statement::While {
                condition, body, ..
            } => {
                collect_expression_hints(condition, type_check, resolution, source, hints);
                collect_binding_hints(body, type_check, resolution, source, hints);
            }
            Statement::For {
                binding,
                iterable,
                body,
                ..
            } => {
                if let Some(symbol) = resolution
                    .symbols
                    .iter()
                    .find(|symbol| symbol.span == binding.span && symbol.kind == SymbolKind::Local)
                    && let Some(ty) = type_check.symbol_type(symbol.id)
                {
                    hints.push(InlayHint {
                        position: position_at(source.text(), binding.span.end),
                        label: format!(": {}", format_type(&type_check.types, ty)),
                        kind: 1,
                        padding_left: true,
                    });
                }
                collect_expression_hints(iterable, type_check, resolution, source, hints);
                collect_binding_hints(body, type_check, resolution, source, hints);
            }
            Statement::Break { .. } | Statement::Continue { .. } => {}
            Statement::Return {
                value: Some(value), ..
            } => collect_expression_hints(value, type_check, resolution, source, hints),
            Statement::Return { value: None, .. } | Statement::Binding { .. } => {}
        }
    }
}

fn collect_effect_hint(
    function: &jadren_parser::Function,
    effects: Option<&EffectAnalysis>,
    resolution: &ResolutionOutput,
    source: &SourceFile,
    hints: &mut Vec<InlayHint>,
) {
    let Some(effects) = effects else {
        return;
    };
    let Some(symbol) = resolution.symbols.iter().find(|symbol| {
        symbol.kind == SymbolKind::Function
            && symbol.origin == SymbolOrigin::Source
            && symbol.span == function.name.span
    }) else {
        return;
    };
    let Some(summary) = effects.function(symbol.id) else {
        return;
    };
    if summary.inferred.is_pure() {
        return;
    }
    let display = summary
        .inferred
        .iter()
        .map(jadren_effects::EffectKind::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    hints.push(InlayHint {
        position: position_at(source.text(), function.name.span.end),
        label: format!(" effects: {display}"),
        kind: 1,
        padding_left: true,
    });
}

fn collect_expression_hints(
    expression: &Expression,
    type_check: &jadren_typeck::TypeCheckOutput,
    resolution: &ResolutionOutput,
    source: &SourceFile,
    hints: &mut Vec<InlayHint>,
) {
    if let Some(site) = type_check
        .region_allocations
        .iter()
        .find(|site| site.span == expression.span())
    {
        let region_name = resolution
            .symbol(site.region)
            .map_or("?", |symbol| symbol.name.as_str());
        hints.push(InlayHint {
            position: position_at(source.text(), site.span.end),
            label: format!(
                " alloc: region {region_name} -> {}",
                format_type(&type_check.types, site.result_type)
            ),
            kind: 1,
            padding_left: true,
        });
    }
    match expression {
        Expression::Unary { operand, .. }
        | Expression::Try { operand, .. }
        | Expression::Cast {
            expression: operand,
            ..
        } => {
            collect_expression_hints(operand, type_check, resolution, source, hints);
        }
        Expression::Binary { left, right, .. } => {
            collect_expression_hints(left, type_check, resolution, source, hints);
            collect_expression_hints(right, type_check, resolution, source, hints);
        }
        Expression::Call {
            callee, arguments, ..
        } => {
            collect_expression_hints(callee, type_check, resolution, source, hints);
            for argument in arguments {
                collect_expression_hints(argument, type_check, resolution, source, hints);
            }
        }
        Expression::Field { base, .. } => {
            collect_expression_hints(base, type_check, resolution, source, hints);
        }
        Expression::Index { base, index, .. } => {
            collect_expression_hints(base, type_check, resolution, source, hints);
            collect_expression_hints(index, type_check, resolution, source, hints);
        }
        Expression::Array { elements, .. } => {
            for element in elements {
                collect_expression_hints(element, type_check, resolution, source, hints);
            }
        }
        Expression::StructLiteral { ty, fields, .. } => {
            collect_expression_hints(ty, type_check, resolution, source, hints);
            for field in fields {
                collect_expression_hints(&field.value, type_check, resolution, source, hints);
            }
        }
        Expression::Group { expression, .. } => {
            collect_expression_hints(expression, type_check, resolution, source, hints);
        }
        Expression::Block(block) => {
            collect_binding_hints(block, type_check, resolution, source, hints);
        }
        Expression::If {
            condition,
            then_block,
            else_branch,
            ..
        } => {
            collect_expression_hints(condition, type_check, resolution, source, hints);
            collect_binding_hints(then_block, type_check, resolution, source, hints);
            if let Some(else_branch) = else_branch {
                collect_expression_hints(else_branch, type_check, resolution, source, hints);
            }
        }
        Expression::Match { value, arms, .. } => {
            collect_expression_hints(value, type_check, resolution, source, hints);
            for arm in arms {
                collect_pattern_hints(&arm.pattern, type_check, resolution, source, hints);
                if let Some(guard) = &arm.guard {
                    collect_expression_hints(guard, type_check, resolution, source, hints);
                }
                collect_expression_hints(&arm.value, type_check, resolution, source, hints);
            }
        }
        Expression::Name(_) | Expression::Literal { .. } | Expression::Error(_) => {}
    }
}

fn collect_pattern_hints(
    pattern: &Pattern,
    type_check: &jadren_typeck::TypeCheckOutput,
    resolution: &ResolutionOutput,
    source: &SourceFile,
    hints: &mut Vec<InlayHint>,
) {
    match pattern {
        Pattern::Path(path) => {
            let Some(name) = path.segments.first() else {
                return;
            };
            let Some(symbol) = resolution
                .symbols
                .iter()
                .find(|symbol| symbol.span == name.span && symbol.kind == SymbolKind::Local)
            else {
                return;
            };
            let Some(ty) = type_check.symbol_type(symbol.id) else {
                return;
            };
            hints.push(InlayHint {
                position: position_at(source.text(), name.span.end),
                label: format!(": {}", format_type(&type_check.types, ty)),
                kind: 1,
                padding_left: true,
            });
        }
        Pattern::Constructor { arguments, .. } => {
            for argument in arguments {
                collect_pattern_hints(argument, type_check, resolution, source, hints);
            }
        }
        Pattern::Wildcard(_) | Pattern::Literal { .. } | Pattern::Error(_) => {}
    }
}

fn uri_to_path(uri: &str) -> PathBuf {
    let path = uri.strip_prefix("file://").unwrap_or(uri);
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn frame(value: Value) -> Vec<u8> {
        let payload = serde_json::to_vec(&value).unwrap();
        format!("Content-Length: {}\r\n\r\n", payload.len())
            .into_bytes()
            .into_iter()
            .chain(payload)
            .collect()
    }

    #[test]
    fn diagnostics_use_shared_driver_and_utf16_positions() {
        let mut server = LanguageServer::new();
        let diagnostics = server.open_document(
            "file:///tmp/č.jdn",
            1,
            "module m;\nfn wrong() -> Bool { let value: Int32 = true; return value }",
        );
        assert!(!diagnostics.is_empty());
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0301")
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.source == "jadren")
        );
        assert_eq!(
            position_at("😀x", "😀".len()),
            Position {
                line: 0,
                character: 2
            }
        );
    }

    #[test]
    fn symbols_are_deterministic_and_include_extern_children() {
        let mut server = LanguageServer::new();
        server.open_document(
            "file:///tmp/symbols.jdn",
            1,
            "module symbols;\nstruct Vec3 { x: Float32 }\nfn main() {}\n",
        );
        let symbols = server.document_symbols("file:///tmp/symbols.jdn");
        assert_eq!(
            symbols
                .iter()
                .map(|symbol| symbol.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Vec3", "main"]
        );
        assert_eq!(symbols[0].kind, 23);
    }

    #[test]
    fn definition_references_and_rename_use_resolver_identity() {
        let text = "fn add(value: Int32) -> Int32 { return value }\nfn main() { add(1) }";
        let mut server = LanguageServer::new();
        server.open_document("file:///tmp/query.jdn", 1, text);
        let call_offset = text.rfind("add(1)").unwrap();
        let position = position_at(text, call_offset);
        let definition = server.definition("file:///tmp/query.jdn", position);
        assert_eq!(definition.len(), 1);
        assert_eq!(definition[0].range.start.character, 3);
        let references = server.references("file:///tmp/query.jdn", position, true);
        assert_eq!(references.len(), 2);
        let changes = server
            .rename("file:///tmp/query.jdn", position, "sum")
            .unwrap();
        assert_eq!(changes["file:///tmp/query.jdn"].len(), 2);
        assert!(
            server
                .rename("file:///tmp/query.jdn", position, "1invalid")
                .is_none()
        );
    }

    #[test]
    fn cross_file_navigation_uses_qualified_symbol_identity() {
        let math_uri = "file:///math.jdn";
        let app_uri = "file:///app.jdn";
        let app_text = "module app; import math; fn main() { math.length(1) }";
        let mut server = LanguageServer::new();
        server.open_document(
            math_uri,
            1,
            "module math; pub fn length(value: Int32) -> Int32 { return value }",
        );
        server.open_document(app_uri, 1, app_text);
        let position = position_at(app_text, app_text.rfind("length").unwrap());

        let definition = server.definition(app_uri, position);
        assert_eq!(definition.len(), 1);
        assert_eq!(definition[0].uri, math_uri);
        let hover = server.hover(app_uri, position).unwrap();
        assert!(hover.contents.value.contains("Type:"));
        assert!(hover.contents.value.contains("Effects: `Pure`"));
        assert!(
            hover
                .contents
                .value
                .contains("Signature: `fn(Int32) -> Int32`")
        );

        let references = server.references(app_uri, position, true);
        assert_eq!(references.len(), 2);
        assert!(references.iter().any(|location| location.uri == math_uri));
        assert!(references.iter().any(|location| location.uri == app_uri));

        let changes = server.rename(app_uri, position, "size").unwrap();
        assert_eq!(changes.len(), 2);
        assert!(changes.contains_key(math_uri));
        assert!(changes.contains_key(app_uri));
    }

    #[test]
    fn code_actions_offer_public_visibility_edit_for_private_import() {
        let library_uri = "file:///tmp/library.jdn";
        let app_uri = "file:///tmp/app-code-action.jdn";
        let library_text = "module library; fn hidden() {}";
        let app_text = "module app; import library; fn main() { library.hidden() }";
        let mut server = LanguageServer::new();
        server.open_document(library_uri, 1, library_text);
        server.open_document(app_uri, 1, app_text);

        let actions = server.code_actions(
            app_uri,
            Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 0,
                    character: app_text.encode_utf16().count() as u32,
                },
            },
        );
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, "quickfix");
        assert!(actions[0].title.contains("pub"));
        let edits = &actions[0].edit.changes[library_uri];
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "pub ");
        assert_eq!(
            edits[0].range.start,
            position_at(library_text, library_text.find("fn hidden").unwrap())
        );
    }

    #[test]
    fn code_actions_offer_safe_import_organization_for_simple_block() {
        let alpha_uri = "file:///tmp/alpha.jdn";
        let zeta_uri = "file:///tmp/zeta.jdn";
        let app_uri = "file:///tmp/imports.jdn";
        let app_text = "module app;\nimport zeta;\nimport alpha;\nfn main() {}";
        let mut server = LanguageServer::new();
        server.open_document(alpha_uri, 1, "module alpha;");
        server.open_document(zeta_uri, 1, "module zeta;");
        server.open_document(app_uri, 1, app_text);
        let only = ["source.organizeImports".to_owned()];
        let actions = server.code_actions_filtered(
            app_uri,
            Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: position_at(app_text, app_text.len()),
            },
            Some(&only),
        );
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, "source.organizeImports");
        assert!(actions[0].diagnostics.is_empty());
        assert_eq!(
            actions[0].edit.changes[app_uri][0].new_text,
            "import alpha;\nimport zeta;"
        );

        let crlf_uri = "file:///tmp/imports-crlf.jdn";
        let crlf_text = "module app_crlf;\r\nimport zeta;\r\nimport alpha;\r\nfn main() {}";
        server.open_document(crlf_uri, 1, crlf_text);
        let crlf_actions = server.code_actions_filtered(
            crlf_uri,
            Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: position_at(crlf_text, crlf_text.len()),
            },
            Some(&only),
        );
        assert_eq!(
            crlf_actions[0].edit.changes[crlf_uri][0].new_text,
            "import alpha;\r\nimport zeta;"
        );
    }

    #[test]
    fn code_actions_reject_import_organization_with_intervening_comment() {
        let alpha_uri = "file:///tmp/alpha-comment.jdn";
        let zeta_uri = "file:///tmp/zeta-comment.jdn";
        let app_uri = "file:///tmp/imports-comment.jdn";
        let app_text =
            "module app;\nimport zeta;\n// keep this comment\nimport alpha;\nfn main() {}";
        let mut server = LanguageServer::new();
        server.open_document(alpha_uri, 1, "module alpha;");
        server.open_document(zeta_uri, 1, "module zeta;");
        server.open_document(app_uri, 1, app_text);
        let only = ["source.organizeImports".to_owned()];
        assert!(
            server
                .code_actions_filtered(
                    app_uri,
                    Range {
                        start: Position {
                            line: 0,
                            character: 0,
                        },
                        end: position_at(app_text, app_text.len()),
                    },
                    Some(&only),
                )
                .is_empty()
        );
        let remove_only = ["source.removeUnusedImports".to_owned()];
        assert!(
            server
                .code_actions_filtered(
                    app_uri,
                    Range {
                        start: Position {
                            line: 0,
                            character: 0,
                        },
                        end: position_at(app_text, app_text.len()),
                    },
                    Some(&remove_only),
                )
                .is_empty()
        );
        let fix_all_only = ["source.fixAll".to_owned()];
        assert!(
            server
                .code_actions_filtered(
                    app_uri,
                    Range {
                        start: Position {
                            line: 0,
                            character: 0,
                        },
                        end: position_at(app_text, app_text.len()),
                    },
                    Some(&fix_all_only),
                )
                .is_empty()
        );
    }

    #[test]
    fn code_actions_remove_only_resolver_confirmed_unused_imports() {
        let alpha_uri = "file:///tmp/alpha-unused.jdn";
        let zeta_uri = "file:///tmp/zeta-unused.jdn";
        let app_uri = "file:///tmp/imports-unused.jdn";
        let app_text = "module app;\nimport zeta;\nimport alpha;\nfn main() { alpha.ping(1) }";
        let mut server = LanguageServer::new();
        server.open_document(
            alpha_uri,
            1,
            "module alpha; pub fn ping(value: Int32) -> Int32 { return value }",
        );
        server.open_document(zeta_uri, 1, "module zeta; pub fn idle() {}");
        server.open_document(app_uri, 1, app_text);
        let only = ["source.removeUnusedImports".to_owned()];
        let actions = server.code_actions_filtered(
            app_uri,
            Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: position_at(app_text, app_text.len()),
            },
            Some(&only),
        );
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, "source.removeUnusedImports");
        assert_eq!(
            actions[0].edit.changes[app_uri][0].new_text,
            "import alpha;"
        );

        let fix_all = ["source.fixAll".to_owned()];
        let fix_all_actions = server.code_actions_filtered(
            app_uri,
            Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: position_at(app_text, app_text.len()),
            },
            Some(&fix_all),
        );
        assert_eq!(fix_all_actions.len(), 1);
        assert_eq!(fix_all_actions[0].kind, "source.fixAll.jadren");
        assert_eq!(
            fix_all_actions[0].edit.changes[app_uri][0].new_text,
            "import alpha;"
        );
    }

    #[test]
    fn code_actions_offer_removing_an_invalid_character() {
        let uri = "file:///tmp/invalid-character.jdn";
        let text = "fn main() {\n  ľ\n}";
        let mut server = LanguageServer::new();
        server.open_document(uri, 1, text);
        let actions = server.code_actions(
            uri,
            Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 2,
                    character: 1,
                },
            },
        );
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].title, "Odstrániť neplatný znak");
        assert_eq!(actions[0].edit.changes[uri][0].new_text, "");
        assert_eq!(
            actions[0].edit.changes[uri][0].range.start,
            position_at(text, text.find('ľ').unwrap())
        );
    }

    #[test]
    fn code_actions_respect_requested_kind_filter() {
        let uri = "file:///tmp/code-action-kind-filter.jdn";
        let text = "fn main() {\n  ľ\n}";
        let mut server = LanguageServer::new();
        server.open_document(uri, 1, text);
        let range = Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 2,
                character: 1,
            },
        };
        let refactor_only = ["refactor".to_owned()];
        assert!(
            server
                .code_actions_filtered(uri, range, Some(&refactor_only))
                .is_empty()
        );
        let quickfix_only = ["quickfix".to_owned()];
        let actions = server.code_actions_filtered(uri, range, Some(&quickfix_only));
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, "quickfix");
    }

    #[test]
    fn code_actions_offer_inserting_missing_punctuation_at_parser_cursor() {
        let uri = "file:///tmp/missing-punctuation.jdn";
        let text = "fn main(value Int32) {}";
        let mut server = LanguageServer::new();
        server.open_document(uri, 1, text);
        let actions = server.code_actions(
            uri,
            Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 0,
                    character: text.encode_utf16().count() as u32,
                },
            },
        );
        assert_eq!(actions.len(), 1);
        let action = actions
            .iter()
            .find(|action| action.title == "Doplniť chýbajúcu interpunkciu `:`")
            .expect("missing punctuation quick-fix");
        let edit = &action.edit.changes[uri][0];
        assert_eq!(edit.new_text, ":");
        let insertion = position_at(text, text.find("Int32").unwrap());
        assert_eq!(edit.range.start, insertion);
        assert_eq!(edit.range.end, insertion);
    }

    #[test]
    fn code_actions_offer_inserting_missing_match_operator_at_parser_cursor() {
        let uri = "file:///tmp/missing-match-operator.jdn";
        let text = "fn main() { match value { Ok 1 } }";
        let mut server = LanguageServer::new();
        server.open_document(uri, 1, text);
        let actions = server.code_actions(
            uri,
            Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 0,
                    character: text.encode_utf16().count() as u32,
                },
            },
        );
        assert_eq!(actions.len(), 1);
        let action = actions
            .iter()
            .find(|action| action.title == "Doplniť chýbajúci operátor `=>`")
            .expect("missing match operator quick-fix");
        let edit = &action.edit.changes[uri][0];
        assert_eq!(edit.new_text, "=>");
        let insertion = position_at(text, text.find('1').unwrap());
        assert_eq!(edit.range.start, insertion);
        assert_eq!(edit.range.end, insertion);
    }

    #[test]
    fn code_actions_offer_inserting_missing_keyword_at_parser_cursor() {
        let uri = "file:///tmp/missing-keyword.jdn";
        let text = "fn main() { for value values { } }";
        let mut server = LanguageServer::new();
        server.open_document(uri, 1, text);
        let actions = server.code_actions(
            uri,
            Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 0,
                    character: text.encode_utf16().count() as u32,
                },
            },
        );
        let action = actions
            .iter()
            .find(|action| action.title == "Doplniť chýbajúce kľúčové slovo `in`")
            .expect("missing keyword quick-fix");
        let edit = &action.edit.changes[uri][0];
        assert_eq!(edit.new_text, "in ");
        let insertion = position_at(text, text.find("values").unwrap());
        assert_eq!(edit.range.start, insertion);
        assert_eq!(edit.range.end, insertion);
    }

    #[test]
    fn code_actions_offer_closing_an_unterminated_block_comment() {
        let uri = "file:///tmp/unterminated-comment.jdn";
        let text = "fn main() { /* comment\n}";
        let mut server = LanguageServer::new();
        server.open_document(uri, 1, text);
        let actions = server.code_actions(
            uri,
            Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 1,
                    character: 1,
                },
            },
        );
        let action = actions
            .iter()
            .find(|action| action.title == "Uzavrieť blokový komentár")
            .expect("unterminated comment quick-fix");
        let edit = &action.edit.changes[uri][0];
        assert_eq!(edit.new_text, "*/");
        assert_eq!(edit.range.start, position_at(text, text.len()));
        assert_eq!(edit.range.end, edit.range.start);
    }

    #[test]
    fn code_actions_offer_closing_an_unterminated_string_literal() {
        let uri = "file:///tmp/unterminated-string.jdn";
        let text = "fn main() { \"hello\n}";
        let mut server = LanguageServer::new();
        server.open_document(uri, 1, text);
        let actions = server.code_actions(
            uri,
            Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 1,
                    character: 1,
                },
            },
        );
        let action = actions
            .iter()
            .find(|action| action.title == "Uzavrieť literál")
            .expect("unterminated string quick-fix");
        let edit = &action.edit.changes[uri][0];
        assert_eq!(edit.new_text, "\"");
        assert_eq!(
            edit.range.start,
            position_at(text, text.find('\n').unwrap())
        );
        assert_eq!(edit.range.end, edit.range.start);
    }

    #[test]
    fn code_actions_offer_converting_an_invalid_character_literal() {
        let uri = "file:///tmp/invalid-character-literal.jdn";
        let text = "fn main() { let value = 'ab'; }";
        let mut server = LanguageServer::new();
        server.open_document(uri, 1, text);
        let actions = server.code_actions(
            uri,
            Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 0,
                    character: text.encode_utf16().count() as u32,
                },
            },
        );
        let action = actions
            .iter()
            .find(|action| action.title == "Previesť znakový literál na reťazec")
            .expect("invalid character literal quick-fix");
        assert_eq!(action.edit.changes[uri][0].new_text, "\"ab\"");
    }

    #[test]
    fn code_actions_offer_normalizing_invalid_numeric_separators() {
        let uri = "file:///tmp/invalid-numeric-separator.jdn";
        let text = "fn main() { let value = 1__2; }";
        let mut server = LanguageServer::new();
        server.open_document(uri, 1, text);
        let actions = server.code_actions(
            uri,
            Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 0,
                    character: text.encode_utf16().count() as u32,
                },
            },
        );
        let action = actions
            .iter()
            .find(|action| action.title == "Opraviť oddeľovače číselného literálu")
            .expect("numeric separator quick-fix");
        assert_eq!(action.edit.changes[uri][0].new_text, "12");
    }

    #[test]
    fn code_actions_reject_character_literals_with_invalid_escapes() {
        let uri = "file:///invalid-character-escape.jdn";
        let text = r#"fn main() { let value = 'ab\q'; }"#;
        let mut server = LanguageServer::new();
        server.open_document(uri, 1, text);
        let actions = server.code_actions(
            uri,
            Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 0,
                    character: text.encode_utf16().count() as u32,
                },
            },
        );
        assert!(
            !actions
                .iter()
                .any(|action| action.title == "Previesť znakový literál na reťazec")
        );
    }

    #[test]
    fn allocation_analytics_aggregate_typed_sites_by_region() {
        let uri = "file:///tmp/allocation-analytics.jdn";
        let text = "fn main() { region frame { let a: Buffer<Int32> = frame.allocate(4); let b: Buffer<Int32> = frame.allocate(8); } }";
        let mut server = LanguageServer::new();
        server.open_document(uri, 1, text);
        let summaries = server.allocation_analytics(uri);
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].region, "frame");
        assert_eq!(summaries[0].count, 2);
        assert_eq!(summaries[0].result_types, vec!["Buffer<Int32>"]);
        assert_eq!(
            summaries[0].range.start,
            position_at(text, text.find("frame").unwrap())
        );
    }

    #[test]
    fn hover_and_completion_reuse_source_symbols() {
        let text = "struct Vec3 { x: Float32 }\nfn main() { }";
        let mut server = LanguageServer::new();
        server.open_document("file:///tmp/hover.jdn", 1, text);
        let position = position_at(text, 7);
        let hover = server.hover("file:///tmp/hover.jdn", position).unwrap();
        assert!(hover.contents.value.contains("Vec3"));
        let completion = server.completion("file:///tmp/hover.jdn");
        assert_eq!(
            completion
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Vec3", "main", "x"]
        );
    }

    #[test]
    fn completion_at_filters_prefix_and_lexical_scope() {
        let text = "fn main() { let outer = 1; if true { let inner = 2; inn } }";
        let mut server = LanguageServer::new();
        server.open_document("file:///tmp/completion-scope.jdn", 1, text);
        let inner_end = text.find("inn }").unwrap() + "inn".len();
        let labels = server
            .completion_at(
                "file:///tmp/completion-scope.jdn",
                position_at(text, inner_end),
            )
            .into_iter()
            .map(|item| item.label)
            .collect::<Vec<_>>();
        assert_eq!(labels, vec!["inner"]);

        let text = "fn outer() { let local = 1; } fn main() { local }";
        let mut server = LanguageServer::new();
        server.open_document("file:///tmp/completion-hidden.jdn", 1, text);
        let local_end = text.rfind("local").unwrap() + "local".len();
        assert!(
            server
                .completion_at(
                    "file:///tmp/completion-hidden.jdn",
                    position_at(text, local_end),
                )
                .is_empty(),
            "a local from a sibling function must not leak into completion"
        );
    }

    #[test]
    fn completion_at_offers_prefix_matched_keywords() {
        let uri = "file:///tmp/completion-keyword.jdn";
        let text = "fn main() { ret }";
        let mut server = LanguageServer::new();
        server.open_document(uri, 1, text);
        let items = server.completion_at(uri, position_at(text, text.find("ret").unwrap() + 3));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "return");
        assert_eq!(items[0].kind, 14);
        assert_eq!(items[0].detail.as_deref(), Some("Keyword"));
    }

    #[test]
    fn completion_includes_explicit_import_bindings_without_sibling_leaks() {
        let library_uri = "file:///tmp/completion-library.jdn";
        let app_uri = "file:///tmp/completion-app.jdn";
        let library_text = "module library; pub fn length(value: Int32) -> Int32 { return value }";
        let app_text = "module app; import library.length; fn main() { leng }";
        let mut server = LanguageServer::new();
        server.open_document(library_uri, 1, library_text);
        server.open_document(app_uri, 1, app_text);
        let position = position_at(app_text, app_text.rfind("leng").unwrap() + 4);
        let items = server.completion_at(app_uri, position);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "length");
        assert_eq!(items[0].kind, 12);
        assert_eq!(items[0].detail.as_deref(), Some("Function"));
    }

    #[test]
    fn completion_includes_public_members_after_module_alias() {
        let library_uri = "file:///tmp/qualified-completion-library.jdn";
        let app_uri = "file:///tmp/qualified-completion-app.jdn";
        let library_text =
            "module math; pub fn length(value: Int32) -> Int32 { return value } fn hidden() {}";
        let app_text = "module app; import math; fn main() { math.le }";
        let mut server = LanguageServer::new();
        server.open_document(library_uri, 1, library_text);
        server.open_document(app_uri, 1, app_text);
        let position = position_at(app_text, app_text.rfind("le").unwrap() + 2);
        let items = server.completion_at(app_uri, position);
        assert_eq!(
            items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            vec!["length"]
        );
        assert_eq!(items[0].kind, 12);
        assert_eq!(items[0].detail.as_deref(), Some("Function"));
    }

    #[test]
    fn completion_supports_nested_module_alias_paths() {
        let namespace_uri = "file:///tmp/nested-completion-namespace.jdn";
        let library_uri = "file:///tmp/nested-completion-library.jdn";
        let app_uri = "file:///tmp/nested-completion-app.jdn";
        let namespace_text = "module math;";
        let library_text =
            "module math.linear; pub fn length(value: Int32) -> Int32 { return value }";
        let app_text = "module app; import math; fn main() { math.linear.le }";
        let mut server = LanguageServer::new();
        server.open_document(namespace_uri, 1, namespace_text);
        server.open_document(library_uri, 1, library_text);
        server.open_document(app_uri, 1, app_text);
        let position = position_at(app_text, app_text.rfind("le").unwrap() + 2);
        let items = server.completion_at(app_uri, position);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "length");
    }

    #[test]
    fn completion_includes_public_record_fields_without_private_leaks() {
        let library_uri = "file:///tmp/field-completion-library.jdn";
        let app_uri = "file:///tmp/field-completion-app.jdn";
        let library_text = "module math; pub struct Point { pub length: Int32, hidden: Int32 }";
        let app_text = "module app; import math.Point; fn main(point: Point) { point.le }";
        let mut server = LanguageServer::new();
        server.open_document(library_uri, 1, library_text);
        server.open_document(app_uri, 1, app_text);
        let position = position_at(app_text, app_text.rfind("le").unwrap() + 2);
        let items = server.completion_at(app_uri, position);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "length");
        assert_eq!(items[0].kind, 8);
        assert_eq!(items[0].detail.as_deref(), Some("Field"));
    }

    #[test]
    fn hover_resolves_imported_record_field_type_without_resolver_reference() {
        let library_uri = "file:///tmp/field-hover-library.jdn";
        let app_uri = "file:///tmp/field-hover-app.jdn";
        let library_text = "module math; pub struct Point { pub length: Int32 }";
        let app_text = "module app; import math.Point; fn main(point: Point) { point.length }";
        let mut server = LanguageServer::new();
        server.open_document(library_uri, 1, library_text);
        server.open_document(app_uri, 1, app_text);
        let field_start = app_text.rfind("length").unwrap();
        let hover = server
            .hover(app_uri, position_at(app_text, field_start + 1))
            .expect("record field hover");
        assert!(hover.contents.value.contains("`Field` **length**"));
        assert!(hover.contents.value.contains("Type: `Int32`"));
        assert_eq!(
            hover.range.unwrap().start,
            position_at(app_text, field_start)
        );
    }

    #[test]
    fn completion_includes_imported_enum_variants_after_type_alias() {
        let library_uri = "file:///tmp/enum-completion-library.jdn";
        let app_uri = "file:///tmp/enum-completion-app.jdn";
        let library_text = "module colors; pub enum Color { Ready, Retry, Failed }";
        let app_text = "module app; import colors.Color; fn main() { Color.Re }";
        let mut server = LanguageServer::new();
        server.open_document(library_uri, 1, library_text);
        server.open_document(app_uri, 1, app_text);
        let position = position_at(app_text, app_text.rfind("Re").unwrap() + 2);
        let items = server.completion_at(app_uri, position);
        assert_eq!(
            items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Ready", "Retry"]
        );
        assert_eq!(items[0].label, "Ready");
        assert_eq!(items[0].kind, 22);
        assert_eq!(items[0].detail.as_deref(), Some("EnumVariant"));
    }

    #[test]
    fn hover_resolves_imported_enum_variant_without_resolver_reference() {
        let library_uri = "file:///tmp/enum-hover-library.jdn";
        let app_uri = "file:///tmp/enum-hover-app.jdn";
        let library_text = "module colors; pub enum Color { Ready, Retry }";
        let app_text = "module app; import colors.Color; fn main() { Color.Ready }";
        let mut server = LanguageServer::new();
        server.open_document(library_uri, 1, library_text);
        server.open_document(app_uri, 1, app_text);
        let position = position_at(app_text, app_text.find("Ready").unwrap() + 1);
        let hover = server.hover(app_uri, position).expect("enum variant hover");
        assert!(hover.contents.value.contains("EnumVariant"));
        assert!(hover.contents.value.contains("colors.Color"));
        assert_eq!(
            hover.range.unwrap().start,
            position_at(app_text, app_text.find("Ready").unwrap())
        );
    }

    #[test]
    fn semantic_tokens_are_delta_encoded_from_lexer_and_resolver() {
        let text = "fn main(parameter: Int32) { let value = 42; return value }";
        let mut server = LanguageServer::new();
        server.open_document("file:///tmp/tokens.jdn", 1, text);
        let data = server.semantic_tokens("file:///tmp/tokens.jdn");
        assert!(!data.is_empty());
        assert_eq!(data.len() % 5, 0);
        assert!(data.chunks(5).any(|token| token[3] == 2));
        assert!(data.chunks(5).any(|token| token[3] == 3));
        assert!(data.chunks(5).any(|token| token[3] == 4));
        assert!(data.chunks(5).any(|token| token[3] == 7));
        assert!(data.chunks(5).any(|token| token[3] == 8));
    }

    #[test]
    fn semantic_tokens_full_and_delta_use_deterministic_result_ids() {
        let uri = "file:///tmp/tokens-delta.jdn";
        let first_text = "fn main() { let value = 42; return value }";
        let second_text = "fn main() { let value = 420; return value }";
        let mut server = LanguageServer::new();
        server.open_document(uri, 1, first_text);
        let first = server.semantic_tokens_full(uri);
        assert!(first.result_id.starts_with("v1-"));
        let repeated = server.semantic_tokens_full(uri);
        assert_eq!(first, repeated);

        server.change_document(uri, 2, second_text);
        let delta = server
            .semantic_tokens_delta(uri, &first.result_id)
            .expect("matching semantic token result id");
        assert_ne!(delta.result_id, first.result_id);
        assert_eq!(delta.edits.len(), 1);
        let edit = &delta.edits[0];
        let start = edit.start as usize;
        let end = start + edit.delete_count as usize;
        let mut reconstructed = first.data;
        reconstructed.splice(start..end, edit.data.iter().copied());
        assert_eq!(reconstructed, server.semantic_tokens(uri));
        assert!(
            server
                .semantic_tokens_delta(uri, &first.result_id)
                .is_none()
        );
    }

    #[test]
    fn hover_includes_type_and_effects_from_shared_analysis() {
        let text = "fn main() { }";
        let mut server = LanguageServer::new();
        server.open_document("file:///tmp/typed-hover.jdn", 1, text);
        let hover = server
            .hover("file:///tmp/typed-hover.jdn", position_at(text, 3))
            .unwrap();
        assert!(hover.contents.value.contains("Type:"));
        assert!(hover.contents.value.contains("Effects: `Pure`"));
    }

    #[test]
    fn hover_uses_typed_expression_query_for_literal() {
        let text = "fn main() { let value = 1 + 2; }";
        let mut server = LanguageServer::new();
        server.open_document("file:///tmp/expression-hover.jdn", 1, text);
        let literal_start = text.find('1').expect("literal");
        let hover = server
            .hover(
                "file:///tmp/expression-hover.jdn",
                position_at(text, literal_start),
            )
            .expect("literal hover");
        assert!(hover.contents.value.contains("Expression: `Literal`"));
        assert!(hover.contents.value.contains("Type: `Int32`"));
        assert_eq!(
            hover.range.expect("literal range").start,
            position_at(text, literal_start)
        );
    }

    #[test]
    fn inlay_hints_use_shared_type_check_output() {
        let text = "fn main() { let value = 1; }";
        let mut server = LanguageServer::new();
        server.open_document("file:///tmp/inlay.jdn", 1, text);
        let hints = server.inlay_hints("file:///tmp/inlay.jdn");
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].label, ": Int32");
        assert_eq!(
            hints[0].position,
            position_at(text, text.find("value").unwrap() + 5)
        );
        assert_eq!(hints[0].kind, 1);
        assert!(hints[0].padding_left);
        assert!(
            server
                .inlay_hints_in_range(
                    "file:///tmp/inlay.jdn",
                    Range {
                        start: Position {
                            line: 0,
                            character: 0,
                        },
                        end: Position {
                            line: 0,
                            character: 10,
                        },
                    },
                )
                .is_empty()
        );
        assert_eq!(
            server
                .inlay_hints_in_range(
                    "file:///tmp/inlay.jdn",
                    Range {
                        start: Position {
                            line: 0,
                            character: 0,
                        },
                        end: Position {
                            line: 0,
                            character: 30,
                        },
                    },
                )
                .len(),
            1
        );
    }

    #[test]
    fn inlay_hints_include_query_backed_effects() {
        let text = "fn main() { print(1) }";
        let mut server = LanguageServer::new();
        server.open_document("file:///tmp/effect-inlay.jdn", 1, text);
        let hints = server.inlay_hints("file:///tmp/effect-inlay.jdn");
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].label, " effects: IO, Blocking");
        assert_eq!(
            hints[0].position,
            position_at(text, text.find("main").unwrap() + "main".len())
        );
    }

    #[test]
    fn inlay_hints_include_region_allocation_details() {
        let text = "fn main() { region frame { let values: Buffer<Int32> = frame.allocate(4); } }";
        let mut server = LanguageServer::new();
        server.open_document("file:///tmp/allocation-inlay.jdn", 1, text);
        let hints = server.inlay_hints("file:///tmp/allocation-inlay.jdn");
        let allocation = hints
            .iter()
            .find(|hint| hint.label.starts_with(" alloc: region"))
            .expect("allocation detail hint");
        assert_eq!(allocation.label, " alloc: region frame -> Buffer<Int32>");
        let call_end = text.find("frame.allocate(4)").unwrap() + "frame.allocate(4)".len();
        assert_eq!(allocation.position, position_at(text, call_end));
    }

    #[test]
    fn incremental_changes_apply_utf16_ranges() {
        let original = "fn main() { let value = 1; }";
        let mut server = LanguageServer::new();
        server.open_document("file:///tmp/incremental.jdn", 1, original);
        let start_offset = original.find('1').unwrap();
        let end_offset = start_offset + 1;
        let diagnostics = server
            .change_document_incremental(
                "file:///tmp/incremental.jdn",
                2,
                &[TextDocumentContentChange {
                    range: Some(Range {
                        start: position_at(original, start_offset),
                        end: position_at(original, end_offset),
                    }),
                    text: "42".to_owned(),
                }],
            )
            .expect("valid incremental range");
        assert!(diagnostics.is_empty());
        assert_eq!(server.version("file:///tmp/incremental.jdn"), Some(2));
        assert_eq!(server.inlay_hints("file:///tmp/incremental.jdn").len(), 1);
    }

    #[test]
    fn incremental_changes_reject_mid_unicode_scalar() {
        let text = "fn main() { let value = \"😀\"; }";
        let mut server = LanguageServer::new();
        server.open_document("file:///tmp/incremental-unicode.jdn", 1, text);
        let start = text.find('😀').unwrap();
        let invalid = Position {
            line: 0,
            character: position_at(text, start).character + 1,
        };
        assert!(
            server
                .change_document_incremental(
                    "file:///tmp/incremental-unicode.jdn",
                    2,
                    &[TextDocumentContentChange {
                        range: Some(Range {
                            start: invalid,
                            end: invalid,
                        }),
                        text: "x".to_owned(),
                    }],
                )
                .is_none()
        );
        assert_eq!(
            server.version("file:///tmp/incremental-unicode.jdn"),
            Some(1)
        );
    }

    #[test]
    fn workspace_query_cache_reuses_and_invalidates_deterministically() {
        let uri = "file:///tmp/cache.jdn";
        let text = "fn main() { let value = 1; }";
        let mut server = LanguageServer::new();
        server.open_document(uri, 1, text);
        // `open_document` returns diagnostics, so the first checked query
        // materializes the workspace cache immediately.
        assert!(server.cache.borrow().is_some());
        let _ = server.inlay_hints(uri);
        assert!(server.cache.borrow().is_some());
        let _ = server.semantic_tokens(uri);
        assert!(server.cache.borrow().is_some());
        server.change_document(uri, 2, "fn main() { let value = 2; }");
        assert_eq!(
            server
                .cache
                .borrow()
                .as_ref()
                .and_then(|cache| cache.documents.get(uri))
                .map(|document| document.version),
            Some(2)
        );
        let _ = server.semantic_tokens(uri);
        assert!(server.cache.borrow().is_some());
    }

    #[test]
    fn inlay_hints_traverse_nested_control_flow_blocks() {
        let text = "fn main() { if true { let nested = 2; } }";
        let mut server = LanguageServer::new();
        server.open_document("file:///tmp/inlay-control-flow.jdn", 1, text);
        let hints = server.inlay_hints("file:///tmp/inlay-control-flow.jdn");
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].label, ": Int32");
        assert_eq!(
            hints[0].position,
            position_at(text, text.find("nested").unwrap() + "nested".len())
        );
    }

    #[test]
    fn inlay_hints_include_for_binding_element_type() {
        let text = "fn sum(values: [Int32; 3]) { for value in values { print(value) } }";
        let mut server = LanguageServer::new();
        server.open_document("file:///tmp/inlay-for.jdn", 1, text);
        let binding_end = text.find("for value").unwrap() + "for value".len();
        let hints = server.inlay_hints("file:///tmp/inlay-for.jdn");
        assert!(
            hints
                .iter()
                .any(|hint| hint.label == ": Int32"
                    && hint.position == position_at(text, binding_end)),
            "missing for-binding type hint: {hints:?}"
        );
    }

    #[test]
    fn inlay_hints_cover_else_match_arm_and_region_blocks() {
        let text = "module test; enum Choice { First, Second(Int32) } fn main() { let choice = Choice.First; if true { let then_value = 1; } else { let else_value = 2; } region frame { let region_value = 3; } match choice { First => { let first_value = 4; 0 }, Second(payload) => { let second_value = 5; 0 } } }";
        let mut server = LanguageServer::new();
        server.open_document("file:///tmp/inlay-all-control-flow.jdn", 1, text);
        let hints = server.inlay_hints("file:///tmp/inlay-all-control-flow.jdn");
        let labels = hints
            .iter()
            .map(|hint| hint.label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(hints.len(), 7);
        assert_eq!(
            labels,
            vec![
                ": Nominal",
                ": Int32",
                ": Int32",
                ": Int32",
                ": Int32",
                ": Int32",
                ": Int32"
            ]
        );
        for name in [
            "choice",
            "then_value",
            "else_value",
            "region_value",
            "first_value",
            "payload",
            "second_value",
        ] {
            let offset = text.find(name).unwrap() + name.len();
            assert!(
                hints
                    .iter()
                    .any(|hint| hint.position == position_at(text, offset)),
                "missing hint for {name}"
            );
        }
    }

    #[test]
    fn transport_handles_initialize_open_symbols_and_shutdown() {
        let input = [
            frame(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}})),
            frame(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///tmp/a.jdn","version":1,"text":"fn main() {}"}}})),
            frame(json!({"jsonrpc":"2.0","id":2,"method":"textDocument/documentSymbol","params":{"textDocument":{"uri":"file:///tmp/a.jdn"}}})),
            frame(json!({"jsonrpc":"2.0","id":3,"method":"shutdown"})),
            frame(json!({"jsonrpc":"2.0","method":"exit"})),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        let mut output = Vec::new();
        run_transport(Cursor::new(input), &mut output).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("documentSymbolProvider"));
        assert!(text.contains("codeActionKinds"));
        assert!(text.contains("source.fixAll.jadren"));
        assert!(text.contains("semanticTokensProvider"));
        assert!(text.contains("publishDiagnostics"));
        assert!(text.contains("\"id\":2"));
        assert!(text.contains("\"main\""));
    }
}
