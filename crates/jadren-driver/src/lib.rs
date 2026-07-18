//! Compiler session, normalized configuration, and deterministic frontend orchestration.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::str::FromStr;

use jadren_determinism::{Fingerprint, StableHasher, normalize_path};
use jadren_diagnostics::{Diagnostic, Severity};
use jadren_effects::{
    EffectAnalysis, EffectKind, EffectSet, apply_external_summaries, check_compute_constraints,
    check_effect_constraints, infer_effects_unresolved,
};
use jadren_hir::{HirModule, lower_hir, verify_hir};
use jadren_jir::{
    LowerOptions as JirLowerOptions, Module as JirModule, OptimizationStats,
    canonicalize_loops_and_licm, eliminate_proven_bounds_checks, eliminate_redundant_bounds_checks,
    eliminate_redundant_offsets, fold_constants, inline_tiny_functions, lower_from_mir,
    promote_scalar_stack_slots, simplify_cfg_and_dce, verify as verify_jir,
};
use jadren_lexer::{Token, lex};
use jadren_mir::{
    MirModule, analyze_borrows, analyze_definite_initialization, analyze_lifetimes, analyze_moves,
    analyze_regions, elaborate_drops, elaborate_region_cleanup, infer_lifetimes, lower_mir,
    materialize_returns, verify_mir,
};
use jadren_parser::{AstFile, parse};
use jadren_resolve::{ModuleCatalog, ResolutionOutput, resolve_with_modules};
use jadren_source::{SourceError, SourceFile, SourceId, SourceManager, Span};
use jadren_syntax::SyntaxTree;
use jadren_typeck::{TypeCheckOutput, check_types};

const COMPILER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Language edition selected for one compilation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Edition {
    /// Current Jadren 0.1 draft semantics.
    Draft01,
}

impl Edition {
    /// Returns the canonical configuration spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft01 => "0.1-draft",
        }
    }

    /// Returns the package-manifest spelling for this edition.
    #[must_use]
    pub const fn package_spelling(self) -> &'static str {
        match self {
            Self::Draft01 => "2026",
        }
    }
}

impl FromStr for Edition {
    type Err = EditionError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "2026" | "0.1-draft" => Ok(Self::Draft01),
            _ => Err(EditionError(value.to_owned())),
        }
    }
}

/// Unsupported language edition spelling.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditionError(String);

impl fmt::Display for EditionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unsupported Jadren edition `{}`", self.0)
    }
}

impl std::error::Error for EditionError {}

/// Frontend/build profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuildProfile {
    /// Parse, resolve, and type-check without code generation.
    Check,
    /// Debug-oriented code generation profile.
    Debug,
    /// Optimized release profile.
    Release,
}

impl BuildProfile {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Debug => "debug",
            Self::Release => "release",
        }
    }
}

/// Diagnostic renderer requested by the caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticFormat {
    /// Human-readable terminal text.
    Text,
    /// Machine-readable JSON document.
    Json,
}

/// Canonical target triple.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TargetTriple(String);

impl TargetTriple {
    /// Returns the canonical triple for the compiler host.
    #[must_use]
    pub fn host() -> Self {
        let triple = match (std::env::consts::ARCH, std::env::consts::OS) {
            ("x86_64", "windows") => "x86_64-pc-windows-msvc".to_owned(),
            ("aarch64", "windows") => "aarch64-pc-windows-msvc".to_owned(),
            ("x86_64", "linux") => "x86_64-unknown-linux-gnu".to_owned(),
            ("aarch64", "linux") => "aarch64-unknown-linux-gnu".to_owned(),
            ("x86_64", "macos") => "x86_64-apple-darwin".to_owned(),
            ("aarch64", "macos") => "aarch64-apple-darwin".to_owned(),
            ("x86_64", "android") => "x86_64-linux-android".to_owned(),
            ("aarch64", "android") => "aarch64-linux-android".to_owned(),
            (arch, os) => format!("{arch}-unknown-{os}"),
        };
        Self(triple)
    }

    /// Returns the canonical string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TargetTriple {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn target_pointer_bits(target: &TargetTriple) -> u16 {
    if target
        .as_str()
        .split('-')
        .next()
        .is_some_and(|architecture| architecture.contains("64"))
    {
        64
    } else {
        32
    }
}

fn optimize_release(jir: &mut JirModule) -> OptimizationReport {
    let mut report = OptimizationReport::default();
    report.record("fold-constants.1", fold_constants(jir));
    report.record("simplify-cfg-dce.1", simplify_cfg_and_dce(jir));
    report.record("promote-stack-slots.1", promote_scalar_stack_slots(jir));
    report.record("eliminate-bounds.1", eliminate_proven_bounds_checks(jir));
    report.record("loop-canonicalize-licm.1", canonicalize_loops_and_licm(jir));
    report.record(
        "eliminate-redundant-bounds.1",
        eliminate_redundant_bounds_checks(jir),
    );
    report.record(
        "eliminate-redundant-offsets.1",
        eliminate_redundant_offsets(jir),
    );
    report.record("inline-tiny.1", inline_tiny_functions(jir));
    report.record("fold-constants.2", fold_constants(jir));
    report.record("simplify-cfg-dce.2", simplify_cfg_and_dce(jir));
    report.record("promote-stack-slots.2", promote_scalar_stack_slots(jir));
    report.record("eliminate-bounds.2", eliminate_proven_bounds_checks(jir));
    report.record("loop-canonicalize-licm.2", canonicalize_loops_and_licm(jir));
    report
}

impl FromStr for TargetTriple {
    type Err = TargetTripleError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let canonical = value.trim().to_ascii_lowercase();
        let valid = canonical.split('-').count() >= 2
            && canonical
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
        if canonical.is_empty() || !valid || canonical.contains("--") {
            return Err(TargetTripleError(value.to_owned()));
        }
        Ok(Self(canonical))
    }
}

/// Invalid target triple.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetTripleError(String);

impl fmt::Display for TargetTripleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid target triple `{}`", self.0)
    }
}

impl std::error::Error for TargetTripleError {}

/// Immutable configuration for one compiler session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompilerConfig {
    /// Language edition.
    pub edition: Edition,
    /// Canonical compilation target.
    pub target: TargetTriple,
    /// Build/check profile.
    pub profile: BuildProfile,
    /// Caller-selected diagnostic format.
    pub diagnostic_format: DiagnosticFormat,
    /// Promotes warnings to errors.
    pub warnings_as_errors: bool,
}

impl CompilerConfig {
    /// Returns a fingerprint of fields that can affect semantic/build output.
    #[must_use]
    pub fn semantic_fingerprint(&self) -> Fingerprint {
        let mut hasher = StableHasher::with_domain("jadren-compiler-config-v1");
        hasher.write_str(COMPILER_VERSION);
        hasher.write_str(self.edition.as_str());
        hasher.write_str(self.target.as_str());
        hasher.write_str(self.profile.as_str());
        hasher.write_bool(self.warnings_as_errors);
        hasher.finish()
    }
}

impl Default for CompilerConfig {
    fn default() -> Self {
        Self {
            edition: Edition::Draft01,
            target: TargetTriple::host(),
            profile: BuildProfile::Check,
            diagnostic_format: DiagnosticFormat::Text,
            warnings_as_errors: false,
        }
    }
}

/// Deterministic key for one source/configuration frontend computation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CompilationKey(Fingerprint);

impl CompilationKey {
    /// Returns the underlying stable fingerprint.
    #[must_use]
    pub const fn fingerprint(self) -> Fingerprint {
        self.0
    }
}

impl fmt::Display for CompilationKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// One deterministic record emitted by a Release JIR optimization pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OptimizationRemark {
    /// Stable pass name.
    pub pass: &'static str,
    /// Counters reported by the pass.
    pub stats: OptimizationStats,
}

/// Ordered optimization remarks attached to a successful Release check.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OptimizationReport {
    /// Passes in the exact order in which the pipeline ran.
    pub remarks: Vec<OptimizationRemark>,
}

impl OptimizationReport {
    fn record(&mut self, pass: &'static str, stats: OptimizationStats) {
        self.remarks.push(OptimizationRemark { pass, stats });
    }

    /// Returns aggregate counters across all recorded passes.
    #[must_use]
    pub fn totals(&self) -> OptimizationStats {
        self.remarks
            .iter()
            .fold(OptimizationStats::default(), |mut total, remark| {
                total.folded_instructions += remark.stats.folded_instructions;
                total.simplified_terminators += remark.stats.simplified_terminators;
                total.removed_instructions += remark.stats.removed_instructions;
                total.inlined_calls += remark.stats.inlined_calls;
                total.promoted_stack_slots += remark.stats.promoted_stack_slots;
                total.eliminated_bounds_checks += remark.stats.eliminated_bounds_checks;
                total.canonicalized_loops += remark.stats.canonicalized_loops;
                total.hoisted_loop_instructions += remark.stats.hoisted_loop_instructions;
                total
            })
    }

    /// Renders a stable, line-oriented report for CLI/tooling consumers.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut text = String::new();
        for remark in &self.remarks {
            let stats = remark.stats;
            use std::fmt::Write as _;
            writeln!(
                text,
                "{} folded={} cfg={} removed={} inlined={} promoted={} bounds={} loops={} hoisted={}",
                remark.pass,
                stats.folded_instructions,
                stats.simplified_terminators,
                stats.removed_instructions,
                stats.inlined_calls,
                stats.promoted_stack_slots,
                stats.eliminated_bounds_checks,
                stats.canonicalized_loops,
                stats.hoisted_loop_instructions,
            )
            .expect("writing to String cannot fail");
        }
        text
    }
}

/// Successful parser artifacts, also available for partially valid syntax.
#[derive(Clone, Debug)]
pub struct FrontendArtifacts {
    /// Lossless syntax tree.
    pub syntax: SyntaxTree,
    /// Lowered AST.
    pub ast: AstFile,
    /// Deterministic symbol and lexical scope resolution.
    pub resolution: ResolutionOutput,
    /// Lowered explicit types and local inference results.
    pub type_check: TypeCheckOutput,
    /// Verified typed HIR; absent when an earlier semantic phase has errors.
    pub hir: Option<HirModule>,
    /// Direct and transitive source-level effects; absent without verified HIR.
    pub effects: Option<EffectAnalysis>,
    /// Structurally verified place-based MIR; memory-analysis errors remain in diagnostics.
    pub mir: Option<MirModule>,
    /// Target-neutral JIR; present only after all semantic and memory checks succeed.
    pub jir: Option<JirModule>,
    /// Release optimization remarks; `None` for Check/Debug or failed lowering.
    pub optimization: Option<OptimizationReport>,
}

/// Result of checking one source in a compiler session.
#[derive(Clone, Debug)]
pub struct CheckOutput {
    /// Checked source identifier.
    pub source: SourceId,
    /// Complete lexer token stream.
    pub tokens: Vec<Token>,
    /// Parser artifacts; absent when lexical errors prevent parsing.
    pub artifacts: Option<FrontendArtifacts>,
    /// Deterministically sorted diagnostics.
    pub diagnostics: Vec<Diagnostic>,
    /// Reproducible source/configuration key.
    pub key: CompilationKey,
}

impl CheckOutput {
    /// Returns whether any error diagnostic exists.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }

    /// Returns the parsed top-level item count when parser artifacts exist.
    #[must_use]
    pub fn top_level_item_count(&self) -> Option<usize> {
        self.artifacts
            .as_ref()
            .map(|artifacts| artifacts.ast.items.len())
    }
}

/// Owns source files and immutable configuration for one compiler invocation.
#[derive(Clone, Debug)]
pub struct CompilerSession {
    config: CompilerConfig,
    sources: SourceManager,
}

impl CompilerSession {
    /// Creates an empty compiler session.
    #[must_use]
    pub fn new(config: CompilerConfig) -> Self {
        Self {
            config,
            sources: SourceManager::new(),
        }
    }

    /// Returns the immutable session configuration.
    #[must_use]
    pub const fn config(&self) -> &CompilerConfig {
        &self.config
    }

    /// Returns all registered sources for diagnostic rendering.
    #[must_use]
    pub const fn sources(&self) -> &SourceManager {
        &self.sources
    }

    /// Adds a source using a deterministic lexical display path.
    pub fn add_source(
        &mut self,
        path: impl AsRef<Path>,
        text: impl Into<String>,
    ) -> Result<SourceId, SessionError> {
        self.sources
            .add(normalize_path(path), text)
            .map_err(SessionError::Source)
    }

    /// Runs the implemented frontend phases for one registered source.
    pub fn check(&self, source_id: SourceId) -> Result<CheckOutput, SessionError> {
        let source = self
            .sources
            .get(source_id)
            .ok_or(SessionError::UnknownSource(source_id))?;
        let key = compilation_key(&self.config, source, &self.sources);
        let lexed = lex(source);
        let lexical_errors = lexed.has_errors();
        let tokens = lexed.tokens;
        let mut diagnostics = lexed.diagnostics;
        let artifacts = if lexical_errors {
            None
        } else {
            let parsed = parse(source, &tokens);
            let syntax_errors = parsed.has_errors();
            diagnostics.extend(parsed.diagnostics);
            let catalog = self.build_module_catalog();
            diagnostics.extend(catalog.diagnostics().iter().cloned());
            let effect_summaries = self.build_effect_summaries(&catalog);
            let resolution = resolve_with_modules(source, &parsed.file, &catalog);
            let resolution_errors = resolution.has_errors();
            diagnostics.extend(resolution.diagnostics.iter().cloned());
            let type_check = check_types(source, &parsed.file, &resolution);
            let type_errors = type_check.has_errors();
            diagnostics.extend(type_check.diagnostics.iter().cloned());
            let (hir, effects, mir, jir, optimization) = if syntax_errors
                || resolution_errors
                || type_errors
            {
                (None, None, None, None, None)
            } else {
                let lowered = lower_hir(source, &parsed.file, &resolution, &type_check);
                let lowering_errors = !lowered.diagnostics.is_empty();
                diagnostics.extend(lowered.diagnostics);
                let verification = verify_hir(&lowered.module, &type_check.types);
                for error in &verification {
                    diagnostics.push(Diagnostic::error(
                        "J0401",
                        "typed HIR verification failed",
                        error.span,
                        error.message.clone(),
                    ));
                }
                if lowering_errors || !verification.is_empty() {
                    (None, None, None, None, None)
                } else {
                    let mut effects =
                        infer_effects_unresolved(&lowered.module, &resolution, &type_check.types);
                    apply_external_summaries(&mut effects, &effect_summaries);
                    for error in check_effect_constraints(&lowered.module, &effects)
                        .into_iter()
                        .chain(check_compute_constraints(
                            &lowered.module,
                            &effects,
                            &type_check.types,
                        ))
                    {
                        diagnostics.push(Diagnostic::error(
                            error.code,
                            "effect constraint is violated",
                            error.span,
                            error.message,
                        ));
                    }
                    let mut mir = lower_mir(&lowered.module, &type_check.types);
                    materialize_returns(&mut mir, &type_check.types);
                    elaborate_region_cleanup(&mut mir);
                    infer_lifetimes(&mut mir);
                    let lifetime_errors = analyze_lifetimes(&mir, &type_check.types);
                    elaborate_drops(&mut mir, &type_check.types);
                    let mir_verification = verify_mir(&mir, &type_check.types);
                    for error in &mir_verification {
                        diagnostics.push(Diagnostic::error(
                            error.code,
                            "MIR verification failed",
                            error.span,
                            error.message.clone(),
                        ));
                    }
                    if mir_verification.is_empty() {
                        let memory_errors: Vec<_> = analyze_definite_initialization(&mir)
                            .into_iter()
                            .chain(analyze_moves(&mir))
                            .chain(analyze_borrows(&mir, &type_check.types))
                            .chain(lifetime_errors)
                            .chain(analyze_regions(&mir))
                            .collect();
                        for error in &memory_errors {
                            diagnostics.push(Diagnostic::error(
                                error.code,
                                error.message.clone(),
                                error.span,
                                "place-based memory rule is violated",
                            ));
                        }
                        let (jir, optimization) = if memory_errors.is_empty()
                            && !diagnostics
                                .iter()
                                .any(|diagnostic| diagnostic.severity == Severity::Error)
                        {
                            match lower_from_mir(
                                &mir,
                                &type_check.types,
                                JirLowerOptions {
                                    pointer_bits: target_pointer_bits(&self.config.target),
                                },
                            ) {
                                Ok(mut jir) => {
                                    let optimization = (self.config.profile
                                        == BuildProfile::Release)
                                        .then(|| optimize_release(&mut jir));
                                    let verification = verify_jir(&jir);
                                    if verification.is_empty() {
                                        (Some(jir), optimization)
                                    } else {
                                        for error in verification {
                                            diagnostics.push(Diagnostic::error(
                                                "J0701",
                                                "JIR verification failed",
                                                error
                                                    .span
                                                    .unwrap_or_else(|| Span::empty(source.id(), 0)),
                                                error.message,
                                            ));
                                        }
                                        (None, None)
                                    }
                                }
                                Err(errors) => {
                                    if self.config.profile != BuildProfile::Check {
                                        for error in errors {
                                            diagnostics.push(Diagnostic::error(
                                                "J0700",
                                                "MIR-to-JIR lowering failed",
                                                error
                                                    .span
                                                    .unwrap_or_else(|| Span::empty(source.id(), 0)),
                                                error.message,
                                            ));
                                        }
                                    }
                                    (None, None)
                                }
                            }
                        } else {
                            (None, None)
                        };
                        (
                            Some(lowered.module),
                            Some(effects),
                            Some(mir),
                            jir,
                            optimization,
                        )
                    } else {
                        (Some(lowered.module), Some(effects), None, None, None)
                    }
                }
            };
            Some(FrontendArtifacts {
                syntax: parsed.syntax,
                ast: parsed.file,
                resolution,
                type_check,
                hir,
                effects,
                mir,
                jir,
                optimization,
            })
        };
        if self.config.warnings_as_errors {
            for diagnostic in &mut diagnostics {
                if diagnostic.severity == Severity::Warning {
                    diagnostic.severity = Severity::Error;
                }
            }
        }
        sort_diagnostics(&self.sources, &mut diagnostics);
        Ok(CheckOutput {
            source: source_id,
            tokens,
            artifacts,
            diagnostics,
            key,
        })
    }

    fn build_module_catalog(&self) -> ModuleCatalog {
        let mut catalog = ModuleCatalog::new();
        for source in self.sources.iter() {
            let lexed = lex(source);
            if lexed.has_errors() {
                continue;
            }
            let parsed = parse(source, &lexed.tokens);
            if parsed.has_errors() {
                continue;
            }
            catalog.add_file(source, &parsed.file);
        }
        catalog.finalize();
        catalog
    }

    fn build_effect_summaries(&self, catalog: &ModuleCatalog) -> BTreeMap<String, EffectSet> {
        #[derive(Clone)]
        struct Node {
            direct: EffectSet,
            calls: Vec<String>,
        }

        let mut nodes = BTreeMap::new();
        for source in self.sources.iter() {
            let lexed = lex(source);
            if lexed.has_errors() {
                continue;
            }
            let parsed = parse(source, &lexed.tokens);
            if parsed.has_errors() {
                continue;
            }
            let resolution = resolve_with_modules(source, &parsed.file, catalog);
            if resolution.has_errors() {
                continue;
            }
            let checked = check_types(source, &parsed.file, &resolution);
            if checked.has_errors() {
                continue;
            }
            let lowered = lower_hir(source, &parsed.file, &resolution, &checked);
            if !lowered.diagnostics.is_empty()
                || !verify_hir(&lowered.module, &checked.types).is_empty()
            {
                continue;
            }
            let analysis = infer_effects_unresolved(&lowered.module, &resolution, &checked.types);
            for function in analysis.functions {
                let Some(path) = resolution
                    .symbol(function.symbol)
                    .and_then(|symbol| symbol.canonical_path.clone())
                else {
                    continue;
                };
                let mut calls = function.external_calls;
                calls.extend(function.calls.into_iter().filter_map(|callee| {
                    resolution
                        .symbol(callee)
                        .and_then(|symbol| symbol.canonical_path.clone())
                }));
                calls.sort();
                calls.dedup();
                nodes.insert(
                    path,
                    Node {
                        direct: function.direct,
                        calls,
                    },
                );
            }
        }

        let mut summaries: BTreeMap<_, _> = nodes
            .iter()
            .map(|(path, node)| (path.clone(), node.direct))
            .collect();
        loop {
            let mut next = BTreeMap::new();
            for (path, node) in &nodes {
                let mut effects = node.direct;
                for callee in &node.calls {
                    if let Some(summary) = summaries.get(callee) {
                        effects.union_with(*summary);
                    } else {
                        effects.insert(EffectKind::Unsafe);
                    }
                }
                next.insert(path.clone(), effects);
            }
            if next == summaries {
                return summaries;
            }
            summaries = next;
        }
    }
}

/// Compiler session failure unrelated to user syntax.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionError {
    /// Source storage failed.
    Source(SourceError),
    /// Source ID belongs to another session or does not exist.
    UnknownSource(SourceId),
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(error) => error.fmt(formatter),
            Self::UnknownSource(id) => write!(formatter, "unknown source ID {}", id.index()),
        }
    }
}

impl std::error::Error for SessionError {}

fn compilation_key(
    config: &CompilerConfig,
    source: &SourceFile,
    sources: &SourceManager,
) -> CompilationKey {
    let mut hasher = StableHasher::with_domain("jadren-frontend-key-v2");
    hasher.write_u64(config.semantic_fingerprint().as_u64());
    hasher.write_str(&normalize_path(source.path()));
    let mut inputs: Vec<_> = sources
        .iter()
        .map(|input| (normalize_path(input.path()), input.stable_hash()))
        .collect();
    inputs.sort_unstable();
    hasher.write_u64(inputs.len() as u64);
    for (path, hash) in inputs {
        hasher.write_str(&path);
        hasher.write_u64(hash);
    }
    CompilationKey(hasher.finish())
}

fn sort_diagnostics(sources: &SourceManager, diagnostics: &mut [Diagnostic]) {
    diagnostics.sort_by_cached_key(|diagnostic| {
        let path = sources
            .get(diagnostic.primary.span.source)
            .map_or_else(String::new, |source| normalize_path(source.path()));
        (
            path,
            diagnostic.primary.span.start,
            diagnostic.primary.span.end,
            severity_rank(diagnostic.severity),
            diagnostic.code,
            diagnostic.message.clone(),
        )
    });
}

const fn severity_rank(severity: Severity) -> u8 {
    match severity {
        Severity::Error => 0,
        Severity::Warning => 1,
        Severity::Note => 2,
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use jadren_diagnostics::Diagnostic;
    use jadren_effects::EffectKind;
    use jadren_jir::{Linkage, OptimizationStats};
    use jadren_source::{SourceManager, Span};

    use super::{
        BuildProfile, CompilerConfig, CompilerSession, DiagnosticFormat, Edition,
        OptimizationReport, TargetTriple, sort_diagnostics,
    };

    #[test]
    fn optimization_report_is_ordered_and_aggregates_counters() {
        let mut report = OptimizationReport::default();
        report.record(
            "first",
            OptimizationStats {
                folded_instructions: 2,
                ..OptimizationStats::default()
            },
        );
        report.record(
            "second",
            OptimizationStats {
                removed_instructions: 3,
                ..OptimizationStats::default()
            },
        );
        assert_eq!(report.totals().folded_instructions, 2);
        assert_eq!(report.totals().removed_instructions, 3);
        assert_eq!(
            report.to_text(),
            "first folded=2 cfg=0 removed=0 inlined=0 promoted=0 bounds=0 loops=0 hoisted=0\nsecond folded=0 cfg=0 removed=3 inlined=0 promoted=0 bounds=0 loops=0 hoisted=0\n"
        );
    }

    #[test]
    fn target_and_config_fingerprints_are_canonical() {
        let target = TargetTriple::from_str(" X86_64-PC-WINDOWS-MSVC ").expect("valid triple");
        assert_eq!(target.as_str(), "x86_64-pc-windows-msvc");
        assert!(TargetTriple::from_str("not a target").is_err());

        let first = CompilerConfig {
            target: target.clone(),
            ..CompilerConfig::default()
        };
        assert_eq!(first.semantic_fingerprint().to_string(), "d55b3a1088fb6c67");
        let mut second = first.clone();
        second.diagnostic_format = DiagnosticFormat::Json;
        assert_eq!(
            first.semantic_fingerprint(),
            second.semantic_fingerprint(),
            "rendering must not invalidate semantic cache keys"
        );
        second.profile = BuildProfile::Release;
        assert_ne!(first.semantic_fingerprint(), second.semantic_fingerprint());
    }

    #[test]
    fn edition_spelling_and_package_mapping_are_stable() {
        assert_eq!(Edition::from_str("2026").unwrap(), Edition::Draft01);
        assert_eq!(Edition::from_str("0.1-draft").unwrap(), Edition::Draft01);
        assert_eq!(Edition::Draft01.package_spelling(), "2026");
        assert!(Edition::from_str("2027").is_err());
    }

    #[test]
    fn equivalent_paths_and_sources_have_equal_compilation_keys() {
        let config = CompilerConfig::default();
        let mut first = CompilerSession::new(config.clone());
        let first_id = first
            .add_source(r"C:\work\src\..\main.jdn", "fn main() {}")
            .expect("source should fit");
        let first_output = first.check(first_id).expect("source should exist");

        let mut second = CompilerSession::new(config);
        let second_id = second
            .add_source("c:/work/main.jdn", "fn main() {}")
            .expect("source should fit");
        let second_output = second.check(second_id).expect("source should exist");

        assert_eq!(first_output.key, second_output.key);
        assert_eq!(first_output.top_level_item_count(), Some(1));
        assert!(!first_output.has_errors());
        assert_eq!(
            first
                .sources()
                .get(first_id)
                .expect("source exists")
                .path()
                .to_string_lossy(),
            "c:/work/main.jdn"
        );
    }

    #[test]
    fn diagnostics_sort_by_normalized_path_then_span() {
        let mut sources = SourceManager::new();
        let b = sources.add("b.jdn", "x").expect("source should fit");
        let a = sources.add("a.jdn", "y").expect("source should fit");
        let mut diagnostics = vec![
            Diagnostic::error("JTEST", "b", Span::new(b, 0, 1).expect("span"), "b"),
            Diagnostic::error("JTEST", "a", Span::new(a, 0, 1).expect("span"), "a"),
        ];
        sort_diagnostics(&sources, &mut diagnostics);
        assert_eq!(diagnostics[0].message, "a");
        assert_eq!(diagnostics[1].message, "b");
    }

    #[test]
    fn session_runs_resolver_and_propagates_duplicate_diagnostics() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        let id = session
            .add_source(
                "duplicate.jdn",
                "fn main() { let value = 1; let value = 2 }",
            )
            .expect("source should fit");
        let output = session.check(id).expect("source should exist");

        assert!(output.has_errors());
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0200")
        );
        let artifacts = output.artifacts.expect("lexing should produce artifacts");
        assert!(artifacts.resolution.has_errors());
        assert!(!artifacts.resolution.scopes.is_empty());
    }

    #[test]
    fn session_resolves_imports_across_registered_sources() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        session
            .add_source(
                "math.jdn",
                "module math; pub struct Vec3 {} pub fn length() -> Float32 { return 0.0f32 }",
            )
            .expect("source should fit");
        let app = session
            .add_source(
                "app.jdn",
                "module app; import math.Vec3; import math; fn measure(value: Vec3) -> Float32 { return math.length() }",
            )
            .expect("source should fit");

        let output = session.check(app).expect("source should exist");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let resolution = &output.artifacts.expect("artifacts").resolution;
        assert_eq!(resolution.imports.len(), 2);
        assert!(
            resolution
                .symbols
                .iter()
                .any(|symbol| { symbol.canonical_path.as_deref() == Some("math.length") })
        );
    }

    #[test]
    fn compilation_key_changes_when_a_registered_dependency_changes() {
        fn key_with_dependency(body: &str) -> super::CompilationKey {
            let mut session = CompilerSession::new(CompilerConfig::default());
            session
                .add_source("math.jdn", format!("module math; {body}"))
                .expect("source should fit");
            let app = session
                .add_source("app.jdn", "module app; import math")
                .expect("source should fit");
            session.check(app).expect("source should exist").key
        }

        assert_ne!(
            key_with_dependency("fn first() {}"),
            key_with_dependency("fn second() {}")
        );
    }

    #[test]
    fn files_in_the_same_module_share_top_level_declarations() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        session
            .add_source("common-helper.jdn", "module common; fn helper() {}")
            .expect("source should fit");
        let user = session
            .add_source(
                "common-user.jdn",
                "module common; fn use_helper() { helper() }",
            )
            .expect("source should fit");

        let output = session.check(user).expect("source should exist");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let resolution = &output.artifacts.expect("artifacts").resolution;
        assert!(resolution.references.iter().any(|reference| {
            resolution
                .symbol(reference.symbol)
                .is_some_and(|symbol| symbol.canonical_path.as_deref() == Some("common.helper"))
        }));
    }

    #[test]
    fn session_propagates_cross_module_visibility_errors() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        session
            .add_source("library.jdn", "module library; fn hidden() {}")
            .expect("source should fit");
        let app = session
            .add_source(
                "app.jdn",
                "module app; import library.hidden; fn main() { hidden() }",
            )
            .expect("source should fit");

        let output = session.check(app).expect("source should exist");
        assert!(output.has_errors());
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0205")
        );
    }

    #[test]
    fn session_runs_local_type_inference_and_propagates_mismatches() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        let source = session
            .add_source(
                "types.jdn",
                "module types; fn wrong() -> Bool { let value: Int32 = true; return value }",
            )
            .expect("source should fit");

        let output = session.check(source).expect("source should exist");
        assert!(output.has_errors());
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0301")
        );
        assert!(output.artifacts.expect("artifacts").type_check.has_errors());
    }

    #[test]
    fn session_checks_function_calls_across_module_boundaries() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        session
            .add_source(
                "math.jdn",
                "module math; pub fn add(a: Int32, b: Int32) -> Int32 { return a + b }",
            )
            .expect("source should fit");
        let app = session
            .add_source(
                "app.jdn",
                "module app; import math.add; fn main() { add(true) }",
            )
            .expect("source should fit");

        let output = session.check(app).expect("source should exist");
        let codes: Vec<_> = output
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect();
        assert!(codes.contains(&"J0301"));
        assert!(codes.contains(&"J0304"));
    }

    #[test]
    fn session_checks_extern_c_import_and_call() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        session
            .add_source(
                "ffi.jdn",
                r#"
module ffi;
extern "C" {
    unsafe fn external_hash(data: Pointer<UInt8>, count: UIntSize) -> UInt64;
}
"#,
            )
            .expect("source should fit");
        let app = session
            .add_source(
                "app.jdn",
                "module app; import ffi.external_hash; fn main(data: Pointer<UInt8>, count: UIntSize) -> UInt64 { return external_hash(data, count) }",
            )
            .expect("source should fit");

        let output = session.check(app).expect("source should exist");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        assert!(output.artifacts.expect("artifacts").jir.is_some());
    }

    #[test]
    fn session_lowers_c_export_annotation_to_jir_export_linkage() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        let source = session
            .add_source(
                "exports.jdn",
                "module exports; @export(name: \"jadren_add\", abi: \"C\") fn add(a: Int32, b: Int32) -> Int32 { return a + b }",
            )
            .expect("source should fit");

        let output = session.check(source).expect("source should exist");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let function = output
            .artifacts
            .expect("artifacts")
            .jir
            .expect("JIR")
            .functions
            .into_iter()
            .find(|function| function.name == "jadren_add")
            .expect("exported JIR function");
        assert_eq!(function.linkage, Linkage::Export);
    }

    #[test]
    fn release_profile_folds_scalar_constants_before_jir_verification() {
        let mut session = CompilerSession::new(CompilerConfig {
            profile: BuildProfile::Release,
            ..CompilerConfig::default()
        });
        let source = session
            .add_source(
                "constant-folding.jdn",
                "module test; fn main() -> Int32 { return (40 + 2) * 1 }",
            )
            .expect("source should fit");

        let output = session.check(source).expect("source should exist");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let jir = output
            .artifacts
            .expect("artifacts")
            .jir
            .expect("lowered JIR");
        let text = jir.to_text();
        assert!(text.contains("const 42"), "{text}");
        assert!(!text.contains(" = add "), "{text}");
        assert!(!text.contains(" = mul "), "{text}");
    }

    #[test]
    fn release_profile_simplifies_constant_branch_before_jir_verification() {
        let mut session = CompilerSession::new(CompilerConfig {
            profile: BuildProfile::Release,
            ..CompilerConfig::default()
        });
        let source = session
            .add_source(
                "constant-branch.jdn",
                "module test; fn main() -> Int32 { return if true { 1 } else { 2 } }",
            )
            .expect("source should fit");

        let output = session.check(source).expect("source should exist");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let jir = output
            .artifacts
            .expect("artifacts")
            .jir
            .expect("lowered JIR");
        let text = jir.to_text();
        assert!(!text.contains("branch "), "{text}");
        assert!(text.contains("const 1"), "{text}");
    }

    #[test]
    fn release_profile_inlines_tiny_scalar_function_before_jir_verification() {
        let mut session = CompilerSession::new(CompilerConfig {
            profile: BuildProfile::Release,
            ..CompilerConfig::default()
        });
        let source = session
            .add_source(
                "tiny-inline.jdn",
                "module test; fn add_one(value: Int32) -> Int32 { return value + 1 } fn main() -> Int32 { return add_one(41) }",
            )
            .expect("source should fit");

        let output = session.check(source).expect("source should exist");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let jir = output
            .artifacts
            .expect("artifacts")
            .jir
            .expect("lowered JIR");
        let text = jir.to_text();
        assert!(!text.contains("call @f0"), "{text}");
        assert!(!text.contains("stack_alloc"), "{text}");
        assert!(text.contains("const 42"), "{text}");
    }

    #[test]
    fn release_profile_eliminates_constant_array_bounds_check() {
        let mut session = CompilerSession::new(CompilerConfig {
            profile: BuildProfile::Release,
            ..CompilerConfig::default()
        });
        let source = session
            .add_source(
                "constant-bounds.jdn",
                "module test; fn main() -> Int32 { let values: [Int32; 3] = [1, 2, 3]; return values[1] }",
            )
            .expect("source should fit");

        let output = session.check(source).expect("source should exist");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let jir = output
            .artifacts
            .expect("artifacts")
            .jir
            .expect("lowered JIR");
        assert!(!jir.to_text().contains("bounds_check"), "{}", jir.to_text());
    }

    #[test]
    fn cross_module_signatures_preserve_nominal_type_identity() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        session
            .add_source(
                "math.jdn",
                "module math; pub struct Vector {} pub fn consume(value: Vector) {}",
            )
            .expect("source should fit");
        let app = session
            .add_source(
                "app.jdn",
                "module app; import math.Vector; import math.consume; fn main() { let value = Vector {}; consume(value) }",
            )
            .expect("source should fit");

        let output = session.check(app).expect("source should exist");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn session_enforces_record_field_visibility_across_modules() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        session
            .add_source(
                "library.jdn",
                "module library; pub struct Data { pub visible: Int32, secret: Int32 }",
            )
            .expect("source should fit");
        let app = session
            .add_source(
                "app.jdn",
                "module app; import library.Data; fn main() { let data = Data { visible: 1, secret: 2 }; print(data.visible); print(data.secret) }",
            )
            .expect("source should fit");

        let output = session.check(app).expect("source should exist");
        assert_eq!(
            output
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "J0205")
                .count(),
            2
        );
    }

    #[test]
    fn files_in_one_module_share_private_record_fields() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        session
            .add_source("data.jdn", "module common; struct Data { value: Int32 }")
            .expect("source should fit");
        let user = session
            .add_source(
                "user.jdn",
                "module common; fn main() { let data = Data { value: 1 }; print(data.value) }",
            )
            .expect("source should fit");

        let output = session.check(user).expect("source should exist");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn session_checks_enum_matches_across_modules() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        session
            .add_source(
                "choice.jdn",
                "module choice; pub enum Choice { First, Second(Int32) }",
            )
            .expect("source should fit");
        let exhaustive = session
            .add_source(
                "exhaustive.jdn",
                "module app; import choice.Choice; fn choose(value: Choice) -> Int32 { return match value { First => 0, Second(item) => item } }",
            )
            .expect("source should fit");

        let output = session.check(exhaustive).expect("source should exist");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);

        let mut incomplete_session = CompilerSession::new(CompilerConfig::default());
        incomplete_session
            .add_source(
                "choice.jdn",
                "module choice; pub enum Choice { First, Second(Int32) }",
            )
            .expect("source should fit");
        let incomplete = incomplete_session
            .add_source(
                "incomplete.jdn",
                "module app; import choice.Choice; fn choose(value: Choice) -> Int32 { return match value { First => 0 } }",
            )
            .expect("source should fit");
        let output = incomplete_session
            .check(incomplete)
            .expect("source should exist");
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0311")
        );
    }

    #[test]
    fn session_propagates_result_across_module_call() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        session
            .add_source(
                "library.jdn",
                "module library; pub fn load() -> Result<Int32, String> { return Ok(1) }",
            )
            .expect("source should fit");
        let app = session
            .add_source(
                "app.jdn",
                "module app; import library.load; fn run() -> Result<Int32, String> { let value = load()?; return Ok(value) }",
            )
            .expect("source should fit");

        let output = session.check(app).expect("source should exist");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        assert_eq!(
            output
                .artifacts
                .expect("artifacts")
                .type_check
                .propagation_sites
                .len(),
            1
        );
    }

    #[test]
    fn session_instantiates_generic_functions_across_modules() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        session
            .add_source(
                "library.jdn",
                "module library; pub fn identity<T>(value: T) -> T { return value }",
            )
            .expect("source should fit");
        let app = session
            .add_source(
                "app.jdn",
                "module app; import library.identity; fn main() { let number: Int32 = identity(1); let flag: Bool = identity(true) }",
            )
            .expect("source should fit");

        let output = session.check(app).expect("source should exist");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        assert_eq!(
            output
                .artifacts
                .expect("artifacts")
                .type_check
                .monomorphizations
                .len(),
            2
        );
    }

    #[test]
    fn session_rejects_underconstrained_generic_call_across_modules() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        session
            .add_source("library.jdn", "module library; pub fn make<T>() -> T {}")
            .expect("source should fit");
        let app = session
            .add_source(
                "app.jdn",
                "module app; import library.make; fn main() { make() }",
            )
            .expect("source should fit");

        let output = session.check(app).expect("source should exist");
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0314")
        );
    }

    #[test]
    fn session_substitutes_generic_records_across_modules() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        session
            .add_source(
                "library.jdn",
                "module library; pub struct Holder<T> { pub value: T }",
            )
            .expect("source should fit");
        let app = session
            .add_source(
                "app.jdn",
                "module app; import library.Holder; fn main() { let holder = Holder { value: true }; let flag: Bool = holder.value }",
            )
            .expect("source should fit");

        let output = session.check(app).expect("source should exist");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn session_substitutes_generic_enums_across_modules() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        session
            .add_source(
                "library.jdn",
                "module library; pub enum Maybe<T> { Missing, Present(T) }",
            )
            .expect("source should fit");
        let app = session
            .add_source(
                "app.jdn",
                "module app; import library.Maybe; fn value(input: Maybe<Int32>) -> Int32 { return match input { Missing => 0, Present(item) => item } }",
            )
            .expect("source should fit");

        let output = session.check(app).expect("source should exist");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn session_enforces_generic_bounds_across_modules() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        session
            .add_source(
                "library.jdn",
                "module library; pub fn numeric<T: Numeric>(value: T) -> T { return value } pub struct NumberBox<T: Numeric> { pub value: T } pub enum Number<T: Numeric> { Value(T) }",
            )
            .expect("source should fit");
        let app = session
            .add_source(
                "app.jdn",
                "module app; import library.numeric; import library.NumberBox; import library.Number; fn main() { numeric(true); let boxed = NumberBox { value: true }; let number: Number<Bool> = Number.Value(true) }",
            )
            .expect("source should fit");

        let output = session.check(app).expect("source should exist");
        assert!(
            output
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "J0316")
                .count()
                >= 3
        );
    }

    #[test]
    fn session_produces_verified_hir_only_for_valid_sources() {
        let mut valid_session = CompilerSession::new(CompilerConfig::default());
        let valid = valid_session
            .add_source(
                "valid.jdn",
                "module valid; fn value(flag: Bool) -> Int32 { return if flag { 1 } else { 2 } }",
            )
            .expect("source should fit");
        let valid_output = valid_session.check(valid).expect("source should exist");
        assert!(!valid_output.has_errors(), "{:?}", valid_output.diagnostics);
        let valid_artifacts = valid_output.artifacts.expect("artifacts");
        assert!(valid_artifacts.hir.is_some());
        assert!(valid_artifacts.mir.is_some());
        assert!(valid_artifacts.jir.is_some());

        let mut invalid_session = CompilerSession::new(CompilerConfig::default());
        let invalid = invalid_session
            .add_source(
                "invalid.jdn",
                "module invalid; fn value() -> Int32 { return true }",
            )
            .expect("source should fit");
        let invalid_output = invalid_session.check(invalid).expect("source should exist");
        assert!(invalid_output.has_errors());
        let invalid_artifacts = invalid_output.artifacts.expect("artifacts");
        assert!(invalid_artifacts.hir.is_none());
        assert!(invalid_artifacts.jir.is_none());
    }

    #[test]
    fn check_profile_allows_non_codegen_external_while_debug_requires_jir() {
        let source = "module test; fn run() { unknown(1) }";
        let mut checking = CompilerSession::new(CompilerConfig::default());
        let id = checking
            .add_source("check.jdn", source)
            .expect("source should fit");
        let output = checking.check(id).expect("source should exist");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        assert!(output.artifacts.expect("artifacts").jir.is_none());

        let mut building = CompilerSession::new(CompilerConfig {
            profile: BuildProfile::Debug,
            ..CompilerConfig::default()
        });
        let id = building
            .add_source("build.jdn", source)
            .expect("source should fit");
        let output = building.check(id).expect("source should exist");
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0700")
        );
        assert!(output.artifacts.expect("artifacts").jir.is_none());
    }

    #[test]
    fn session_reports_uninitialized_and_moved_place_uses() {
        let mut uninitialized_session = CompilerSession::new(CompilerConfig::default());
        let uninitialized = uninitialized_session
            .add_source(
                "uninitialized.jdn",
                "module test; fn main() { let value: Int32; print(value) }",
            )
            .expect("source should fit");
        let output = uninitialized_session
            .check(uninitialized)
            .expect("source should exist");
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0500")
        );

        let mut moved_session = CompilerSession::new(CompilerConfig::default());
        let moved = moved_session
            .add_source(
                "moved.jdn",
                "module test; fn consume(data: Buffer<Int32>) { let first = data; print(data) }",
            )
            .expect("source should fit");
        let output = moved_session.check(moved).expect("source should exist");
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0501")
        );
    }

    #[test]
    fn session_checks_borrow_conflicts_and_cross_module_capabilities() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        session
            .add_source(
                "library.jdn",
                "module library; pub fn inspect(first: read Buffer<Int32>, second: read Buffer<Int32>) {} pub fn update(first: read Buffer<Int32>, second: write Buffer<Int32>) {} pub fn take(value: owned Buffer<Int32>) {}",
            )
            .expect("source should fit");
        let app = session
            .add_source(
                "app.jdn",
                "module app; import library.inspect; import library.update; import library.take; fn run(data: Buffer<Int32>) { inspect(data, data); update(data, data); take(data) }",
            )
            .expect("source should fit");
        let output = session.check(app).expect("source should exist");
        assert_eq!(
            output
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "J0503")
                .count(),
            1
        );

        let mut read_only = CompilerSession::new(CompilerConfig::default());
        let source = read_only
            .add_source(
                "readonly.jdn",
                "module test; fn mutate(data: read Buffer<Int32>) { data[0] = 1 }",
            )
            .expect("source should fit");
        let output = read_only.check(source).expect("source should exist");
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0504")
        );
    }

    #[test]
    fn session_infers_borrow_end_and_rejects_escape() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        let valid = session
            .add_source(
                "valid_lifetime.jdn",
                "module test; fn consume(value: Buffer<Int32>) {} fn run(data: Buffer<Int32>) { let view: read Buffer<Int32> = data; print(view); consume(data) }",
            )
            .expect("source should fit");
        let output = session.check(valid).expect("source should exist");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let artifacts = output.artifacts.expect("artifacts");
        let jir = artifacts.jir.expect("lowered JIR");
        assert!(jir.to_text().contains("cast.pointer "));
        let mir = artifacts.mir.expect("verified MIR");
        assert!(
            mir.functions[1].blocks[0]
                .statements
                .iter()
                .any(|statement| matches!(statement, jadren_mir::MirStatement::StorageDead { .. }))
        );

        let mut forwarded = CompilerSession::new(CompilerConfig::default());
        let source = forwarded
            .add_source(
                "forwarded_capability.jdn",
                "module test; fn forward(data: read Buffer<Int32>) -> read Buffer<Int32> { return data }",
            )
            .expect("source should fit");
        let output = forwarded.check(source).expect("source should exist");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);

        let mut escaping = CompilerSession::new(CompilerConfig::default());
        let invalid = escaping
            .add_source(
                "escaping.jdn",
                "module test; fn borrow(data: Buffer<Int32>) -> read Buffer<Int32> { let view: read Buffer<Int32> = data; return view }",
            )
            .expect("source should fit");
        let output = escaping.check(invalid).expect("source should exist");
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0505")
        );
    }

    #[test]
    fn session_elaborates_move_only_drops() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        let source = session
            .add_source(
                "drops.jdn",
                "module test; fn release(first: Buffer<Int32>, second: Buffer<Int32>) {}",
            )
            .expect("source should fit");
        let output = session.check(source).expect("source should exist");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let artifacts = output.artifacts.expect("artifacts");
        assert!(
            artifacts
                .jir
                .as_ref()
                .expect("lowered JIR")
                .to_text()
                .contains("drop %v")
        );
        let mir = artifacts.mir.expect("verified MIR");
        let dropped: Vec<_> = mir.functions[0].blocks[0]
            .statements
            .iter()
            .filter_map(|statement| match statement {
                jadren_mir::MirStatement::Drop { place, .. } => Some(place.local.index()),
                _ => None,
            })
            .collect();
        assert_eq!(dropped, [1, 0]);
    }

    #[test]
    fn session_tracks_region_cleanup_and_rejects_escape() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        let source = session
            .add_source(
                "regions.jdn",
                "module test; fn inspect(value: read Buffer<Int32>) {} fn run() { region frame { let values: Buffer<Int32> = frame.allocate(4); inspect(values) } }",
            )
            .expect("source should fit");
        let output = session.check(source).expect("source should exist");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let artifacts = output.artifacts.expect("artifacts");
        assert!(
            artifacts
                .jir
                .as_ref()
                .expect("lowered JIR")
                .to_text()
                .contains("region_alloc ")
        );
        let mir = artifacts.mir.expect("verified MIR");
        let run = &mir.functions[1];
        assert!(
            run.blocks[0]
                .statements
                .iter()
                .any(|statement| matches!(statement, jadren_mir::MirStatement::RegionEnter { .. }))
        );
        assert!(
            run.blocks[0]
                .statements
                .iter()
                .any(|statement| matches!(statement, jadren_mir::MirStatement::RegionExit { .. }))
        );
        assert!(!run.blocks[0].statements.iter().any(|statement| {
            matches!(statement, jadren_mir::MirStatement::Drop { place, .. }
                if run.locals[place.local.index()].owned_region.is_some())
        }));

        let mut escaping = CompilerSession::new(CompilerConfig::default());
        let source = escaping
            .add_source(
                "region_escape.jdn",
                "module test; fn leak() -> Buffer<Int32> { region frame { let values: Buffer<Int32> = frame.allocate(4); return values } }",
            )
            .expect("source should fit");
        let output = escaping.check(source).expect("source should exist");
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0507")
        );
        assert!(output.artifacts.expect("artifacts").jir.is_none());
    }

    #[test]
    fn session_exposes_transitive_effect_inference() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        let source = session
            .add_source(
                "effects.jdn",
                "module test; fn allocate() { region frame { let values: Buffer<Int32> = frame.allocate(4) } } fn wrapper() { allocate() } fn output(value: Int32) { print(value) }",
            )
            .expect("source should fit");
        let output = session.check(source).expect("source should exist");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let effects = output
            .artifacts
            .expect("artifacts")
            .effects
            .expect("effect analysis");
        let wrapper = effects
            .functions
            .iter()
            .find(|function| function.name == "wrapper")
            .expect("wrapper");
        assert!(wrapper.direct.is_pure());
        assert!(wrapper.inferred.contains(EffectKind::Allocate));
        let output = effects
            .functions
            .iter()
            .find(|function| function.name == "output")
            .expect("output");
        assert!(output.inferred.contains(EffectKind::Io));
    }

    #[test]
    fn session_rejects_transitive_allocation_from_noalloc() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        let source = session
            .add_source(
                "noalloc.jdn",
                "module test; fn allocate() { region frame { let values: Buffer<Int32> = frame.allocate(4) } } @noalloc fn update() { allocate() }",
            )
            .expect("source should fit");
        let output = session.check(source).expect("source should exist");
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0600")
        );
    }

    #[test]
    fn session_propagates_noalloc_effects_across_modules() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        session
            .add_source(
                "library.jdn",
                "module library; pub fn allocate() { region frame { let values: Buffer<Int32> = frame.allocate(4) } } pub fn output(value: Int32) { print(value) }",
            )
            .expect("source should fit");
        let allocating = session
            .add_source(
                "app.jdn",
                "module app; import library.allocate; @noalloc fn update() { allocate() }",
            )
            .expect("source should fit");
        let safe = session
            .add_source(
                "safe.jdn",
                "module safe; import library.output; @noalloc fn update() { output(1) }",
            )
            .expect("source should fit");

        let output = session.check(allocating).expect("source should exist");
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0600")
        );
        assert!(
            !output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0601")
        );

        let output = session.check(safe).expect("source should exist");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn session_enforces_realtime_policy_transitively_across_modules() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        session
            .add_source(
                "library.jdn",
                "module library; pub fn output(value: Int32) { print(value) }",
            )
            .expect("source should fit");
        let source = session
            .add_source(
                "app.jdn",
                "module app; import library.output; @realtime fn update(value: Int32) { output(value) }",
            )
            .expect("source should fit");
        let output = session.check(source).expect("source should exist");
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0611")
        );
        assert!(
            !output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0612")
        );
    }

    #[test]
    fn session_checks_compute_eligibility_across_modules() {
        let mut session = CompilerSession::new(CompilerConfig::default());
        session
            .add_source(
                "library.jdn",
                "module library; pub fn output(value: Int32) { print(value) }",
            )
            .expect("source should fit");
        let invalid = session
            .add_source(
                "app.jdn",
                "module app; import library.output; @compute fn kernel(value: Int32) { output(value) }",
            )
            .expect("source should fit");
        let valid = session
            .add_source(
                "valid.jdn",
                "module valid; @compute fn kernel(value: Int32) -> Int32 { return value + value }",
            )
            .expect("source should fit");

        let output = session.check(invalid).expect("source should exist");
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0621")
        );
        assert!(
            !output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0623")
        );

        let output = session.check(valid).expect("source should exist");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }
}
