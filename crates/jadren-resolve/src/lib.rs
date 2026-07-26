//! Deterministic symbols, lexical scopes, and initial same-file name resolution.

use jadren_determinism::{DeterministicMap, Fingerprint, StableHasher};
use jadren_diagnostics::{Diagnostic, Severity};
use jadren_parser::{
    Annotation, AstFile, Block, EnumDeclaration, Expression, Function, GenericParameter, Item,
    MatchArm, Path, Pattern, RecordDeclaration, Statement, TypeCapability, TypeRef,
};
use jadren_source::{SourceFile, Span};
use jadren_types::{AbiRepr, BuiltinTrait, NominalTypeId};

/// Session-local opaque symbol identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SymbolId(usize);

impl SymbolId {
    /// Returns the deterministic symbol-table index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// Stable module-qualified identity shared by equivalent compiler sessions.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct QualifiedSymbolId(Fingerprint);

impl QualifiedSymbolId {
    /// Creates a versioned identity from a canonical path and namespace.
    #[must_use]
    pub fn from_path(namespace: Namespace, canonical_path: &str) -> Self {
        let mut hasher = StableHasher::with_domain("jadren-qualified-symbol-v1");
        hasher.write_u64(u64::from(namespace.order()));
        hasher.write_str(canonical_path);
        Self(hasher.finish())
    }

    /// Returns the stable fingerprint representation.
    #[must_use]
    pub const fn fingerprint(self) -> Fingerprint {
        self.0
    }
}

/// Session-local lexical scope identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ScopeId(usize);

impl ScopeId {
    /// Returns the deterministic scope-table index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// Independent name lookup namespace.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Namespace {
    /// Modules and module aliases.
    Module,
    /// Types and generic type parameters.
    Type,
    /// Functions, values, parameters, locals, fields, and constructors.
    Value,
}

impl Namespace {
    const fn order(self) -> u8 {
        match self {
            Self::Module => 0,
            Self::Type => 1,
            Self::Value => 2,
        }
    }
}

/// Symbol category retained for later type/HIR phases.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SymbolKind {
    /// Module or imported module alias.
    Module,
    /// Compiler-provided type.
    BuiltinType,
    /// Compiler-provided trait usable as a generic bound.
    BuiltinTrait,
    /// Compiler-provided value or intrinsic.
    BuiltinValue,
    /// Function declaration.
    Function,
    /// Struct declaration.
    Struct,
    /// Data-oriented component declaration.
    Component,
    /// Enum declaration.
    Enum,
    /// Enum variant/constructor.
    EnumVariant,
    /// Generic type parameter.
    GenericParameter,
    /// Function parameter.
    Parameter,
    /// Local immutable or mutable binding.
    Local,
    /// Lexical region allocator handle.
    Region,
    /// Struct, component, or named enum payload field.
    Field,
}

/// Declared visibility retained for JAD-403 enforcement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeclaredVisibility {
    /// Visible only inside the owning module/type.
    Private,
    /// Declared using `pub`.
    Public,
}

/// Whether a symbol came from source or the compiler prelude.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SymbolOrigin {
    /// Built into the current language edition.
    Builtin,
    /// Declared by user source.
    Source,
    /// Imported or reached through a qualified module path.
    Imported,
}

/// Deterministic lookup key inside one scope.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SymbolKey {
    /// Lookup namespace.
    pub namespace: Namespace,
    /// Unqualified source name.
    pub name: String,
}

/// One declared symbol.
#[derive(Clone, Debug)]
pub struct Symbol {
    /// Opaque identity.
    pub id: SymbolId,
    /// Source name.
    pub name: String,
    /// Symbol category.
    pub kind: SymbolKind,
    /// Lookup namespace.
    pub namespace: Namespace,
    /// Scope containing the declaration.
    pub scope: ScopeId,
    /// Optional owning declaration.
    pub owner: Option<SymbolId>,
    /// Declaration name range.
    pub span: Span,
    /// Declared visibility.
    pub visibility: DeclaredVisibility,
    /// Builtin or source origin.
    pub origin: SymbolOrigin,
    /// Stable module-qualified path for top-level and imported declarations.
    pub canonical_path: Option<String>,
    /// Stable identity derived from the canonical path and namespace.
    pub qualified_id: Option<QualifiedSymbolId>,
    /// Portable explicit signature for function symbols from module interfaces.
    pub function_signature: Option<ModuleFunctionSignature>,
    /// Portable field interface for imported record/component symbols.
    pub record_interface: Option<ModuleRecordInterface>,
    /// Portable variant interface for imported enum symbols.
    pub enum_interface: Option<ModuleEnumInterface>,
}

impl Symbol {
    /// Returns a stable nominal type identity for module-qualified type symbols.
    #[must_use]
    pub fn nominal_type_id(&self) -> Option<NominalTypeId> {
        if self.namespace != Namespace::Type {
            return None;
        }
        Some(NominalTypeId::from_symbol_fingerprint(
            self.qualified_id?.fingerprint(),
        ))
    }
}

/// One top-level declaration exposed through a module interface.
#[derive(Clone, Debug)]
pub struct ModuleMember {
    /// Unqualified member name.
    pub name: String,
    /// Independent lookup namespace.
    pub namespace: Namespace,
    /// Original declaration category.
    pub kind: SymbolKind,
    /// Name declaration range.
    pub span: Span,
    /// Declared visibility; enforcement belongs to JAD-403.
    pub visibility: DeclaredVisibility,
    /// Portable explicit function signature for cross-file call checking.
    pub function_signature: Option<ModuleFunctionSignature>,
    /// Portable field interface for record/component declarations.
    pub record_interface: Option<ModuleRecordInterface>,
    /// Portable variant interface for enum declarations.
    pub enum_interface: Option<ModuleEnumInterface>,
}

/// Store-independent explicit type used by module function interfaces.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModuleType {
    /// Language core type or constructor.
    Builtin {
        name: String,
        arguments: Vec<ModuleType>,
    },
    /// Stable module-qualified nominal type application.
    Nominal {
        canonical_path: String,
        arguments: Vec<ModuleType>,
    },
    /// Function generic parameter position.
    GenericParameter(usize),
    /// Fixed-size array.
    Array {
        element: Box<ModuleType>,
        length: u64,
    },
    /// Explicit ownership or borrow capability.
    Capability {
        /// Capability kind.
        capability: jadren_types::Capability,
        /// Wrapped portable type.
        inner: Box<ModuleType>,
    },
    /// Explicit function pointer signature.
    Function {
        /// Parameter types in declaration order.
        parameters: Vec<ModuleType>,
        /// Result type.
        result: Box<ModuleType>,
    },
    /// Invalid interface type retained for a later diagnostic.
    Error,
}

/// Portable non-generic or generic function signature in a module interface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleFunctionSignature {
    /// Foreign calling convention when the declaration came from an extern block.
    pub extern_abi: Option<String>,
    /// Whether calling the function requires an unsafe boundary.
    pub is_unsafe: bool,
    /// Number of declared generic parameters.
    pub generic_count: usize,
    /// Core trait bounds for each generic parameter in declaration order.
    pub generic_bounds: Vec<Vec<BuiltinTrait>>,
    /// Parameter types in declaration order.
    pub parameters: Vec<ModuleType>,
    /// Explicit or implicit `Unit` return type.
    pub result: ModuleType,
}

/// One portable record/component field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleRecordField {
    /// Field source name.
    pub name: String,
    /// Explicit field type.
    pub ty: ModuleType,
    /// Declared field visibility.
    pub visibility: DeclaredVisibility,
    /// Declaration name range.
    pub span: Span,
}

/// Store-independent record/component interface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleRecordInterface {
    /// Fully qualified defining module.
    pub module_name: String,
    /// Number of generic parameters.
    pub generic_count: usize,
    /// Core trait bounds for each generic parameter in declaration order.
    pub generic_bounds: Vec<Vec<BuiltinTrait>>,
    /// ABI representation contract of the record/component.
    pub repr: AbiRepr,
    /// Fields in deterministic declaration order.
    pub fields: Vec<ModuleRecordField>,
}

/// One portable enum variant and its positional/named payload types.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleEnumVariant {
    /// Variant source name.
    pub name: String,
    /// Payload types in declaration order.
    pub fields: Vec<ModuleType>,
    /// Variant declaration range.
    pub span: Span,
}

/// Store-independent enum interface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleEnumInterface {
    /// Fully qualified defining module.
    pub module_name: String,
    /// Number of generic parameters.
    pub generic_count: usize,
    /// Core trait bounds for each generic parameter in declaration order.
    pub generic_bounds: Vec<Vec<BuiltinTrait>>,
    /// ABI representation contract of the enum.
    pub repr: AbiRepr,
    /// Variants in deterministic declaration order.
    pub variants: Vec<ModuleEnumVariant>,
}

/// Deterministic interface of one logical module, possibly assembled from several files.
#[derive(Clone, Debug)]
pub struct ModuleInterface {
    /// Fully qualified module name.
    pub name: String,
    /// First module declaration range.
    pub declaration_span: Span,
    /// Top-level declarations keyed by namespace and name.
    pub members: DeterministicMap<SymbolKey, ModuleMember>,
    /// Raw imports contributed by all files of this module.
    pub imports: Vec<ModuleImport>,
}

/// Import edge retained for deterministic module graph analysis.
#[derive(Clone, Debug)]
pub struct ModuleImport {
    /// Imported path spelling.
    pub path: String,
    /// Import declaration range.
    pub span: Span,
}

/// Module interfaces visible inside one compiler session.
#[derive(Clone, Debug, Default)]
pub struct ModuleCatalog {
    modules: DeterministicMap<String, ModuleInterface>,
    diagnostics: Vec<Diagnostic>,
    cycle_diagnostic_count: usize,
}

impl ModuleCatalog {
    /// Creates an empty module catalog.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            modules: DeterministicMap::new(),
            diagnostics: Vec::new(),
            cycle_diagnostic_count: 0,
        }
    }

    /// Adds a parsed source file to its declared logical module.
    pub fn add_file(&mut self, source: &SourceFile, file: &AstFile) {
        if self.cycle_diagnostic_count != 0 {
            self.diagnostics
                .truncate(self.diagnostics.len() - self.cycle_diagnostic_count);
            self.cycle_diagnostic_count = 0;
        }
        let Some(module_path) = &file.module else {
            return;
        };
        let module_name = path_text(module_path);
        let interface =
            self.modules
                .entry(module_name.clone())
                .or_insert_with(|| ModuleInterface {
                    name: module_name.clone(),
                    declaration_span: module_path.span,
                    members: DeterministicMap::new(),
                    imports: Vec::new(),
                });

        interface
            .imports
            .extend(file.imports.iter().map(|path| ModuleImport {
                path: path_text(path),
                span: path.span,
            }));

        for item in &file.items {
            if let Item::ExternBlock(block) = item {
                for function in &block.functions {
                    let member =
                        module_extern_member(source, &module_name, &file.imports, block, function);
                    insert_module_member(interface, &mut self.diagnostics, &module_name, member);
                }
                continue;
            }

            let mut member = module_member(item);
            if let Item::Function(function) = item {
                member.function_signature = Some(module_function_signature(
                    source,
                    &module_name,
                    &file.imports,
                    function,
                ));
            } else if let Item::Struct(record) | Item::Component(record) = item {
                member.record_interface = Some(module_record_interface(
                    source,
                    &module_name,
                    &file.imports,
                    record,
                ));
            } else if let Item::Enum(declaration) = item {
                member.enum_interface = Some(module_enum_interface(
                    source,
                    &module_name,
                    &file.imports,
                    declaration,
                ));
            }
            insert_module_member(interface, &mut self.diagnostics, &module_name, member);
        }
    }

    /// Returns one module by its fully qualified name.
    #[must_use]
    pub fn module(&self, name: &str) -> Option<&ModuleInterface> {
        self.modules.get(name)
    }

    /// Returns catalog diagnostics in deterministic insertion order.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Analyzes the completed catalog and reports cycles containing value imports.
    pub fn finalize(&mut self) {
        if self.cycle_diagnostic_count != 0 {
            return;
        }
        let mut adjacency: DeterministicMap<String, Vec<(String, Span)>> = DeterministicMap::new();
        for (module_name, module) in &self.modules {
            for import in &module.imports {
                if let Some(target) = self.value_import_target(&import.path) {
                    adjacency
                        .entry(module_name.clone())
                        .or_default()
                        .push((target, import.span));
                }
            }
        }

        let mut states: DeterministicMap<String, u8> = DeterministicMap::new();
        let mut stack = Vec::new();
        let mut cycle_diagnostics = Vec::new();
        for module_name in self.modules.keys() {
            visit_import_graph(
                module_name,
                &adjacency,
                &mut states,
                &mut stack,
                &mut cycle_diagnostics,
            );
        }
        self.cycle_diagnostic_count = cycle_diagnostics.len();
        self.diagnostics.extend(cycle_diagnostics);
    }

    fn value_import_target(&self, path: &str) -> Option<String> {
        if let Some((module_name, member_name)) = path.rsplit_once('.')
            && let Some(module) = self.module(module_name)
        {
            let value = SymbolKey {
                namespace: Namespace::Value,
                name: member_name.to_owned(),
            };
            let ty = SymbolKey {
                namespace: Namespace::Type,
                name: member_name.to_owned(),
            };
            if module.members.contains_key(&value) {
                return Some(module_name.to_owned());
            }
            if module.members.contains_key(&ty) {
                return None;
            }
        }
        self.module(path).map(|module| module.name.clone())
    }
}

/// One local binding created by an import declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportBinding {
    /// Full imported source path.
    pub path: String,
    /// Local name introduced by the import.
    pub local_name: String,
    /// Namespace of the imported declaration.
    pub namespace: Namespace,
    /// Module-qualified identity of the target.
    pub canonical_path: String,
    /// Stable identity of the target across equivalent sessions.
    pub target: QualifiedSymbolId,
    /// Local session symbol representing the import.
    pub symbol: SymbolId,
}

/// Lexical scope category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScopeKind {
    /// Source module/root scope.
    Module,
    /// Function parameter and top-level body scope.
    Function,
    /// Struct, component, or enum member scope.
    Type,
    /// One enum variant payload scope.
    EnumVariant,
    /// Nested braced block.
    Block,
    /// One match arm and its pattern bindings.
    MatchArm,
    /// Named lexical allocation region.
    Region,
}

/// One deterministic lexical scope.
#[derive(Clone, Debug)]
pub struct Scope {
    /// Opaque scope identity.
    pub id: ScopeId,
    /// Parent lexical scope.
    pub parent: Option<ScopeId>,
    /// Owning symbol when applicable.
    pub owner: Option<SymbolId>,
    /// Scope category.
    pub kind: ScopeKind,
    /// Source range covered by the scope.
    pub span: Span,
    /// Child scopes in deterministic source traversal order.
    pub children: Vec<ScopeId>,
    /// Declarations keyed by independent namespace and source name.
    pub symbols: DeterministicMap<SymbolKey, SymbolId>,
}

/// Successfully bound source reference.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedReference {
    /// Reference source range.
    pub span: Span,
    /// Namespace used for lookup.
    pub namespace: Namespace,
    /// Bound identity.
    pub symbol: SymbolId,
}

/// Reference awaiting module/import or later semantic resolution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnresolvedReference {
    /// Full reference range.
    pub span: Span,
    /// Dot-separated source spelling.
    pub path: String,
    /// Namespace used for lookup.
    pub namespace: Namespace,
}

/// Resolver result for one source module.
#[derive(Clone, Debug)]
pub struct ResolutionOutput {
    /// Declared fully qualified module name, when present.
    pub module_name: Option<String>,
    /// Root module scope.
    pub root_scope: ScopeId,
    /// Scopes indexed by [`ScopeId`].
    pub scopes: Vec<Scope>,
    /// Symbols indexed by [`SymbolId`].
    pub symbols: Vec<Symbol>,
    /// Successfully resolved imports in source order.
    pub imports: Vec<ImportBinding>,
    /// Bound references in stable source order.
    pub references: Vec<ResolvedReference>,
    /// References deferred to JAD-402/JAD-404.
    pub unresolved: Vec<UnresolvedReference>,
    /// Duplicate definition and structural diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

impl ResolutionOutput {
    /// Returns a symbol by opaque identity.
    #[must_use]
    pub fn symbol(&self, id: SymbolId) -> Option<&Symbol> {
        self.symbols.get(id.index())
    }

    /// Returns a lexical scope by identity.
    #[must_use]
    pub fn scope(&self, id: ScopeId) -> Option<&Scope> {
        self.scopes.get(id.index())
    }

    /// Returns whether resolver errors occurred.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }
}

/// Builds deterministic symbols/scopes and resolves same-file lexical references.
#[must_use]
pub fn resolve(source: &SourceFile, file: &AstFile) -> ResolutionOutput {
    let catalog = ModuleCatalog::new();
    Resolver::new(source, file, &catalog).run()
}

/// Resolves one source using module interfaces collected for the compiler session.
#[must_use]
pub fn resolve_with_modules(
    source: &SourceFile,
    file: &AstFile,
    catalog: &ModuleCatalog,
) -> ResolutionOutput {
    Resolver::new(source, file, catalog).run()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QualifiedLookup {
    Resolved,
    KnownModuleMissing,
    NotModule,
}

struct Resolver<'source> {
    source: &'source SourceFile,
    file: &'source AstFile,
    catalog: &'source ModuleCatalog,
    module_name: Option<String>,
    scopes: Vec<Scope>,
    symbols: Vec<Symbol>,
    imports: Vec<ImportBinding>,
    external_symbols: DeterministicMap<SymbolKey, SymbolId>,
    references: Vec<ResolvedReference>,
    unresolved: Vec<UnresolvedReference>,
    diagnostics: Vec<Diagnostic>,
}

impl<'source> Resolver<'source> {
    fn new(
        source: &'source SourceFile,
        file: &'source AstFile,
        catalog: &'source ModuleCatalog,
    ) -> Self {
        Self {
            source,
            file,
            catalog,
            module_name: file.module.as_ref().map(path_text),
            scopes: Vec::new(),
            symbols: Vec::new(),
            imports: Vec::new(),
            external_symbols: DeterministicMap::new(),
            references: Vec::new(),
            unresolved: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn run(mut self) -> ResolutionOutput {
        let root_span = Span::new(self.source.id(), 0, self.source.text().len())
            .unwrap_or_else(|| Span::empty(self.source.id(), 0));
        let root = self.new_scope(None, None, ScopeKind::Module, root_span);
        self.install_builtins(root);
        self.install_same_module_members(root);
        self.resolve_imports(root);

        let item_symbols: Vec<_> = self
            .file
            .items
            .iter()
            .map(|item| match item {
                Item::ExternBlock(block) => self.declare_extern_block(root, block),
                _ => vec![self.declare_item(root, item)],
            })
            .collect();
        for (item, symbols) in self.file.items.iter().zip(item_symbols) {
            match item {
                Item::ExternBlock(block) => {
                    for (function, symbol) in block.functions.iter().zip(symbols) {
                        self.resolve_extern_function(root, function, symbol);
                    }
                }
                _ => self.resolve_item(
                    root,
                    item,
                    symbols.into_iter().next().expect("normal item declaration"),
                ),
            }
        }

        self.references.sort_by_key(|reference| {
            (
                reference.span.start,
                reference.span.end,
                reference.namespace.order(),
                reference.symbol.index(),
            )
        });
        self.unresolved.sort_by(|left, right| {
            (
                left.span.start,
                left.span.end,
                left.namespace.order(),
                &left.path,
            )
                .cmp(&(
                    right.span.start,
                    right.span.end,
                    right.namespace.order(),
                    &right.path,
                ))
        });

        ResolutionOutput {
            module_name: self.module_name,
            root_scope: root,
            scopes: self.scopes,
            symbols: self.symbols,
            imports: self.imports,
            references: self.references,
            unresolved: self.unresolved,
            diagnostics: self.diagnostics,
        }
    }

    fn install_builtins(&mut self, root: ScopeId) {
        const TYPES: &[&str] = &[
            "Bool", "Buffer", "Char", "Float16", "F16", "Float2", "Float32", "F32", "Float3",
            "Float4", "Float8", "Float64", "F64", "Int8", "Int16", "Int32", "Int64", "IntSize",
            "Never", "Option", "Pointer", "Result", "Slice", "Status", "String", "UInt8", "UInt16",
            "UInt32", "UInt64", "UIntSize", "Unit",
        ];
        const VALUES: &[&str] = &[
            "assert_eq",
            "print",
            "vector_load2",
            "vector_splat2",
            "vector_store2",
            "vector_load3",
            "vector_splat3",
            "vector_store3",
            "vector_load4",
            "vector_splat4",
            "vector_store4",
            "vector_load8",
            "vector_splat8",
            "vector_store8",
        ];
        let span = Span::empty(self.source.id(), 0);
        for name in TYPES {
            let symbol = self.define(
                root,
                name,
                Namespace::Type,
                SymbolKind::BuiltinType,
                span,
                None,
                DeclaredVisibility::Public,
                SymbolOrigin::Builtin,
            );
            self.set_canonical_path(symbol, &format!("core.{name}"));
        }
        for bound in BuiltinTrait::ALL {
            let name = bound.name();
            let symbol = self.define(
                root,
                name,
                Namespace::Type,
                SymbolKind::BuiltinTrait,
                span,
                None,
                DeclaredVisibility::Public,
                SymbolOrigin::Builtin,
            );
            self.set_canonical_path(symbol, &format!("core.{name}"));
        }
        for name in VALUES {
            let symbol = self.define(
                root,
                name,
                Namespace::Value,
                SymbolKind::BuiltinValue,
                span,
                None,
                DeclaredVisibility::Public,
                SymbolOrigin::Builtin,
            );
            self.set_canonical_path(symbol, &format!("core.{name}"));
        }
    }

    fn resolve_imports(&mut self, root: ScopeId) {
        for import in &self.file.imports {
            let imported_path = path_text(import);
            let Some(local_name) = import.segments.last().map(|segment| segment.text.clone())
            else {
                continue;
            };
            let mut members = Vec::new();
            if let Some((module_name, member_name)) = imported_path.rsplit_once('.')
                && let Some(module) = self.catalog.module(module_name)
            {
                for namespace in [Namespace::Type, Namespace::Value] {
                    let key = SymbolKey {
                        namespace,
                        name: member_name.to_owned(),
                    };
                    if let Some(member) = module.members.get(&key) {
                        members.push((module_name.to_owned(), member.clone()));
                    }
                }
            }

            if !members.is_empty() {
                for (module_name, member) in members {
                    let canonical_path = format!("{module_name}.{}", member.name);
                    self.check_module_visibility(&module_name, &member, import.span, "import");
                    let symbol = self.define(
                        root,
                        &local_name,
                        member.namespace,
                        member.kind,
                        import.span,
                        None,
                        member.visibility,
                        SymbolOrigin::Imported,
                    );
                    self.set_canonical_path(symbol, &canonical_path);
                    self.symbols[symbol.index()].function_signature =
                        member.function_signature.clone();
                    self.symbols[symbol.index()].record_interface = member.record_interface.clone();
                    self.symbols[symbol.index()].enum_interface = member.enum_interface.clone();
                    self.imports.push(ImportBinding {
                        path: imported_path.clone(),
                        local_name: local_name.clone(),
                        namespace: member.namespace,
                        target: QualifiedSymbolId::from_path(member.namespace, &canonical_path),
                        canonical_path,
                        symbol,
                    });
                }
            } else if self.catalog.module(&imported_path).is_some() {
                let symbol = self.define(
                    root,
                    &local_name,
                    Namespace::Module,
                    SymbolKind::Module,
                    import.span,
                    None,
                    DeclaredVisibility::Public,
                    SymbolOrigin::Imported,
                );
                self.set_canonical_path(symbol, &imported_path);
                self.imports.push(ImportBinding {
                    path: imported_path.clone(),
                    local_name,
                    namespace: Namespace::Module,
                    target: QualifiedSymbolId::from_path(Namespace::Module, &imported_path),
                    canonical_path: imported_path,
                    symbol,
                });
            } else {
                self.diagnostics.push(Diagnostic::error(
                    "J0202",
                    format!("unresolved import `{imported_path}`"),
                    import.span,
                    "no matching module or module member exists in this compiler session",
                ));
            }
        }
    }

    fn install_same_module_members(&mut self, root: ScopeId) {
        let Some(module_name) = self.module_name.clone() else {
            return;
        };
        let Some(module) = self.catalog.module(&module_name) else {
            return;
        };
        let local_keys: Vec<_> = self
            .file
            .items
            .iter()
            .flat_map(|item| match item {
                Item::ExternBlock(block) => block
                    .functions
                    .iter()
                    .map(|function| SymbolKey {
                        namespace: Namespace::Value,
                        name: function.name.text.clone(),
                    })
                    .collect::<Vec<_>>(),
                _ => {
                    let member = module_member(item);
                    vec![SymbolKey {
                        namespace: member.namespace,
                        name: member.name,
                    }]
                }
            })
            .collect();
        let members: Vec<_> = module
            .members
            .iter()
            .filter(|(key, member)| {
                member.span.source != self.source.id() && !local_keys.contains(key)
            })
            .map(|(_, member)| member.clone())
            .collect();
        for member in members {
            let canonical_path = format!("{module_name}.{}", member.name);
            let symbol = self.define(
                root,
                &member.name,
                member.namespace,
                member.kind,
                member.span,
                None,
                member.visibility,
                SymbolOrigin::Imported,
            );
            self.set_canonical_path(symbol, &canonical_path);
            self.symbols[symbol.index()].function_signature = member.function_signature;
            self.symbols[symbol.index()].record_interface = member.record_interface;
            self.symbols[symbol.index()].enum_interface = member.enum_interface;
        }
    }

    fn declare_item(&mut self, root: ScopeId, item: &Item) -> SymbolId {
        let (name, namespace, kind, span, visibility) = match item {
            Item::Function(function) => (
                &function.name,
                Namespace::Value,
                SymbolKind::Function,
                function.name.span,
                visibility(function.is_public),
            ),
            Item::Struct(record) => (
                &record.name,
                Namespace::Type,
                SymbolKind::Struct,
                record.name.span,
                visibility(record.is_public),
            ),
            Item::Component(record) => (
                &record.name,
                Namespace::Type,
                SymbolKind::Component,
                record.name.span,
                visibility(record.is_public),
            ),
            Item::Enum(declaration) => (
                &declaration.name,
                Namespace::Type,
                SymbolKind::Enum,
                declaration.name.span,
                visibility(declaration.is_public),
            ),
            Item::ExternBlock(_) => unreachable!("extern blocks are declared separately"),
        };
        let symbol = self.define(
            root,
            &name.text,
            namespace,
            kind,
            span,
            None,
            visibility,
            SymbolOrigin::Source,
        );
        if let Some(module_name) = &self.module_name {
            self.set_canonical_path(symbol, &format!("{module_name}.{}", name.text));
        }
        symbol
    }

    fn declare_extern_block(
        &mut self,
        root: ScopeId,
        block: &jadren_parser::ExternBlock,
    ) -> Vec<SymbolId> {
        if block.abi != "C" {
            self.diagnostics.push(Diagnostic::error(
                "J0205",
                format!("unsupported extern ABI `{}`", block.abi),
                block.span,
                "Jadren 0.1 supports only `extern \"C\"` declarations",
            ));
        }
        block
            .functions
            .iter()
            .map(|function| {
                let symbol = self.define(
                    root,
                    &function.name.text,
                    Namespace::Value,
                    SymbolKind::Function,
                    function.name.span,
                    None,
                    DeclaredVisibility::Public,
                    SymbolOrigin::Source,
                );
                if let Some(module_name) = &self.module_name {
                    self.set_canonical_path(
                        symbol,
                        &format!("{module_name}.{}", function.name.text),
                    );
                }
                self.symbols[symbol.index()].function_signature =
                    Some(module_extern_function_signature(
                        self.source,
                        self.module_name.as_deref().unwrap_or("<anonymous>"),
                        &self.file.imports,
                        block,
                        function,
                    ));
                symbol
            })
            .collect()
    }

    fn resolve_item(&mut self, root: ScopeId, item: &Item, owner: SymbolId) {
        match item {
            Item::Function(function) => self.resolve_function(root, function, owner),
            Item::Struct(record) | Item::Component(record) => {
                self.resolve_record(root, record, owner)
            }
            Item::Enum(declaration) => self.resolve_enum(root, declaration, owner),
            Item::ExternBlock(_) => unreachable!("extern blocks are resolved separately"),
        }
    }

    fn resolve_extern_function(
        &mut self,
        root: ScopeId,
        function: &jadren_parser::ExternFunction,
        owner: SymbolId,
    ) {
        let scope = self.new_scope(Some(root), Some(owner), ScopeKind::Function, function.span);
        for parameter in &function.parameters {
            self.resolve_type(scope, &parameter.ty);
        }
        if let Some(return_type) = &function.return_type {
            self.resolve_type(scope, return_type);
        }
    }

    fn resolve_function(&mut self, root: ScopeId, function: &Function, owner: SymbolId) {
        self.resolve_annotations(root, &function.annotations);
        let scope = self.new_scope(Some(root), Some(owner), ScopeKind::Function, function.span);
        self.declare_generics(scope, &function.generic_parameters, owner);
        for generic in &function.generic_parameters {
            for bound in &generic.bounds {
                self.resolve_type(scope, bound);
            }
        }
        for parameter in &function.parameters {
            let _ = self.define(
                scope,
                &parameter.name.text,
                Namespace::Value,
                SymbolKind::Parameter,
                parameter.name.span,
                Some(owner),
                DeclaredVisibility::Private,
                SymbolOrigin::Source,
            );
        }
        for parameter in &function.parameters {
            self.resolve_type(scope, &parameter.ty);
        }
        if let Some(return_type) = &function.return_type {
            self.resolve_type(scope, return_type);
        }
        self.resolve_block_in_scope(&function.body, scope);
    }

    fn resolve_record(&mut self, root: ScopeId, record: &RecordDeclaration, owner: SymbolId) {
        self.resolve_annotations(root, &record.annotations);
        let scope = self.new_scope(Some(root), Some(owner), ScopeKind::Type, record.span);
        self.declare_generics(scope, &record.generic_parameters, owner);
        for generic in &record.generic_parameters {
            for bound in &generic.bounds {
                self.resolve_type(scope, bound);
            }
        }
        for field in &record.fields {
            let _ = self.define(
                scope,
                &field.name.text,
                Namespace::Value,
                SymbolKind::Field,
                field.name.span,
                Some(owner),
                visibility(field.is_public),
                SymbolOrigin::Source,
            );
            self.resolve_type(scope, &field.ty);
        }
    }

    fn resolve_enum(&mut self, root: ScopeId, declaration: &EnumDeclaration, owner: SymbolId) {
        self.resolve_annotations(root, &declaration.annotations);
        let scope = self.new_scope(Some(root), Some(owner), ScopeKind::Type, declaration.span);
        self.declare_generics(scope, &declaration.generic_parameters, owner);
        for generic in &declaration.generic_parameters {
            for bound in &generic.bounds {
                self.resolve_type(scope, bound);
            }
        }
        for variant in &declaration.variants {
            let variant_symbol = self.define(
                scope,
                &variant.name.text,
                Namespace::Value,
                SymbolKind::EnumVariant,
                variant.name.span,
                Some(owner),
                DeclaredVisibility::Public,
                SymbolOrigin::Source,
            );
            let variant_scope = self.new_scope(
                Some(scope),
                Some(variant_symbol),
                ScopeKind::EnumVariant,
                variant.span,
            );
            for field in &variant.fields {
                if let Some(name) = &field.name {
                    let _ = self.define(
                        variant_scope,
                        &name.text,
                        Namespace::Value,
                        SymbolKind::Field,
                        name.span,
                        Some(variant_symbol),
                        DeclaredVisibility::Private,
                        SymbolOrigin::Source,
                    );
                }
                self.resolve_type(variant_scope, &field.ty);
            }
        }
    }

    fn declare_generics(&mut self, scope: ScopeId, generics: &[GenericParameter], owner: SymbolId) {
        for generic in generics {
            let _ = self.define(
                scope,
                &generic.name.text,
                Namespace::Type,
                SymbolKind::GenericParameter,
                generic.name.span,
                Some(owner),
                DeclaredVisibility::Private,
                SymbolOrigin::Source,
            );
        }
    }

    fn resolve_annotations(&mut self, scope: ScopeId, annotations: &[Annotation]) {
        for annotation in annotations {
            for argument in &annotation.arguments {
                self.resolve_expression(scope, &argument.value);
            }
        }
    }

    fn resolve_type(&mut self, scope: ScopeId, ty: &TypeRef) {
        match ty {
            TypeRef::Path {
                path, arguments, ..
            } => {
                self.resolve_path(scope, path, Namespace::Type);
                for argument in arguments {
                    self.resolve_type(scope, argument);
                }
            }
            TypeRef::Array { element, .. } => self.resolve_type(scope, element),
            TypeRef::Capability { inner, .. } => self.resolve_type(scope, inner),
            TypeRef::Function {
                parameters,
                return_type,
                ..
            } => {
                for parameter in parameters {
                    self.resolve_type(scope, parameter);
                }
                if let Some(return_type) = return_type {
                    self.resolve_type(scope, return_type);
                }
            }
        }
    }

    fn resolve_block_in_scope(&mut self, block: &Block, scope: ScopeId) {
        for statement in &block.statements {
            match statement {
                Statement::Binding {
                    name, ty, value, ..
                } => {
                    if let Some(ty) = ty {
                        self.resolve_type(scope, ty);
                    }
                    if let Some(value) = value {
                        self.resolve_expression(scope, value);
                    }
                    let _ = self.define(
                        scope,
                        &name.text,
                        Namespace::Value,
                        SymbolKind::Local,
                        name.span,
                        self.scopes[scope.index()].owner,
                        DeclaredVisibility::Private,
                        SymbolOrigin::Source,
                    );
                }
                Statement::Return { value, .. } => {
                    if let Some(value) = value {
                        self.resolve_expression(scope, value);
                    }
                }
                Statement::Region { name, body, span } => {
                    let region_scope = self.new_scope(
                        Some(scope),
                        self.scopes[scope.index()].owner,
                        ScopeKind::Region,
                        *span,
                    );
                    let _ = self.define(
                        region_scope,
                        &name.text,
                        Namespace::Value,
                        SymbolKind::Region,
                        name.span,
                        self.scopes[scope.index()].owner,
                        DeclaredVisibility::Private,
                        SymbolOrigin::Source,
                    );
                    self.resolve_block_in_scope(body, region_scope);
                }
                Statement::While {
                    condition, body, ..
                } => {
                    self.resolve_expression(scope, condition);
                    self.resolve_nested_block(body, scope);
                }
                Statement::For {
                    binding,
                    iterable,
                    body,
                    span,
                } => {
                    self.resolve_expression(scope, iterable);
                    let loop_scope = self.new_scope(
                        Some(scope),
                        self.scopes[scope.index()].owner,
                        ScopeKind::Block,
                        *span,
                    );
                    let _ = self.define(
                        loop_scope,
                        &binding.text,
                        Namespace::Value,
                        SymbolKind::Local,
                        binding.span,
                        self.scopes[scope.index()].owner,
                        DeclaredVisibility::Private,
                        SymbolOrigin::Source,
                    );
                    self.resolve_block_in_scope(body, loop_scope);
                }
                Statement::Break { .. } | Statement::Continue { .. } => {}
                Statement::Expression { expression, .. } => {
                    self.resolve_expression(scope, expression);
                }
            }
        }
    }

    fn resolve_nested_block(&mut self, block: &Block, parent: ScopeId) {
        let scope = self.new_scope(
            Some(parent),
            self.scopes[parent.index()].owner,
            ScopeKind::Block,
            block.span,
        );
        self.resolve_block_in_scope(block, scope);
    }

    fn resolve_expression(&mut self, scope: ScopeId, expression: &Expression) {
        match expression {
            Expression::Name(name) => {
                self.resolve_name(scope, &name.text, name.span, Namespace::Value);
            }
            Expression::Literal { .. } | Expression::Error(_) => {}
            Expression::Unary { operand, .. } => self.resolve_expression(scope, operand),
            Expression::Binary { left, right, .. } => {
                self.resolve_expression(scope, left);
                self.resolve_expression(scope, right);
            }
            Expression::Call {
                callee, arguments, ..
            } => {
                self.resolve_expression(scope, callee);
                for argument in arguments {
                    self.resolve_expression(scope, argument);
                }
            }
            Expression::Field { base, .. } => {
                let lookup = expression_path(expression)
                    .map_or(QualifiedLookup::NotModule, |path| {
                        self.try_resolve_qualified(scope, &path.0, path.1, Namespace::Value)
                    });
                if lookup == QualifiedLookup::NotModule {
                    self.resolve_expression(scope, base);
                }
            }
            Expression::Index { base, index, .. } => {
                self.resolve_expression(scope, base);
                self.resolve_expression(scope, index);
            }
            Expression::Try { operand, .. } => self.resolve_expression(scope, operand),
            Expression::Cast {
                expression, target, ..
            } => {
                self.resolve_expression(scope, expression);
                self.resolve_type(scope, target);
            }
            Expression::Array { elements, .. } => {
                for element in elements {
                    self.resolve_expression(scope, element);
                }
            }
            Expression::StructLiteral { ty, fields, .. } => {
                if let Some((path, span)) = expression_path(ty) {
                    self.resolve_path_text(scope, path, span, Namespace::Type);
                } else {
                    self.resolve_expression(scope, ty);
                }
                for field in fields {
                    self.resolve_expression(scope, &field.value);
                }
            }
            Expression::Group { expression, .. } => self.resolve_expression(scope, expression),
            Expression::Block(block) => self.resolve_nested_block(block, scope),
            Expression::If {
                condition,
                then_block,
                else_branch,
                ..
            } => {
                self.resolve_expression(scope, condition);
                self.resolve_nested_block(then_block, scope);
                if let Some(else_branch) = else_branch {
                    self.resolve_expression(scope, else_branch);
                }
            }
            Expression::Match { value, arms, .. } => {
                self.resolve_expression(scope, value);
                for arm in arms {
                    self.resolve_match_arm(scope, arm);
                }
            }
        }
    }

    fn resolve_match_arm(&mut self, parent: ScopeId, arm: &MatchArm) {
        let scope = self.new_scope(
            Some(parent),
            self.scopes[parent.index()].owner,
            ScopeKind::MatchArm,
            arm.span,
        );
        self.resolve_pattern(scope, &arm.pattern);
        if let Some(guard) = &arm.guard {
            self.resolve_expression(scope, guard);
        }
        self.resolve_expression(scope, &arm.value);
    }

    fn resolve_pattern(&mut self, scope: ScopeId, pattern: &Pattern) {
        match pattern {
            Pattern::Wildcard(_) | Pattern::Literal { .. } | Pattern::Error(_) => {}
            Pattern::Path(path) if is_binding_path(path) => {
                let name = &path.segments[0];
                let _ = self.define(
                    scope,
                    &name.text,
                    Namespace::Value,
                    SymbolKind::Local,
                    name.span,
                    self.scopes[scope.index()].owner,
                    DeclaredVisibility::Private,
                    SymbolOrigin::Source,
                );
            }
            Pattern::Path(path) => self.resolve_path(scope, path, Namespace::Value),
            Pattern::Constructor {
                path, arguments, ..
            } => {
                self.resolve_path(scope, path, Namespace::Value);
                for argument in arguments {
                    self.resolve_pattern(scope, argument);
                }
            }
        }
    }

    fn resolve_path(&mut self, scope: ScopeId, path: &Path, namespace: Namespace) {
        self.resolve_path_text(scope, path_text(path), path.span, namespace);
    }

    fn resolve_path_text(
        &mut self,
        scope: ScopeId,
        path: String,
        span: Span,
        namespace: Namespace,
    ) {
        if !path.contains('.') {
            self.resolve_name(scope, &path, span, namespace);
        } else if self.try_resolve_qualified(scope, &path, span, namespace)
            == QualifiedLookup::NotModule
        {
            self.unresolved.push(UnresolvedReference {
                span,
                path,
                namespace,
            });
        }
    }

    fn try_resolve_qualified(
        &mut self,
        scope: ScopeId,
        path: &str,
        span: Span,
        namespace: Namespace,
    ) -> QualifiedLookup {
        let segments: Vec<_> = path.split('.').collect();
        if segments.len() < 2 {
            return QualifiedLookup::NotModule;
        }

        let alias = self
            .lookup(scope, Namespace::Module, segments[0])
            .and_then(|symbol| self.symbols[symbol.index()].canonical_path.clone());
        let member_name = segments.last().copied().unwrap_or_default();
        let module_name = if let Some(alias) = &alias {
            if segments.len() == 2 {
                alias.clone()
            } else {
                format!("{alias}.{}", segments[1..segments.len() - 1].join("."))
            }
        } else {
            segments[..segments.len() - 1].join(".")
        };

        let Some(module) = self.catalog.module(&module_name) else {
            if alias.is_some() {
                self.diagnostics.push(Diagnostic::error(
                    "J0203",
                    format!("unknown member path `{path}`"),
                    span,
                    "the imported module alias does not contain this nested module",
                ));
                return QualifiedLookup::KnownModuleMissing;
            }
            return QualifiedLookup::NotModule;
        };
        let key = SymbolKey {
            namespace,
            name: member_name.to_owned(),
        };
        let Some(member) = module.members.get(&key).cloned() else {
            self.diagnostics.push(Diagnostic::error(
                "J0203",
                format!("module `{module_name}` has no {namespace:?} member `{member_name}`"),
                span,
                "unknown qualified module member",
            ));
            return QualifiedLookup::KnownModuleMissing;
        };
        self.check_module_visibility(&module_name, &member, span, "access");
        let canonical_path = format!("{module_name}.{member_name}");
        let symbol = self.external_symbol(scope, &canonical_path, &member);
        self.references.push(ResolvedReference {
            span,
            namespace,
            symbol,
        });
        QualifiedLookup::Resolved
    }

    fn check_module_visibility(
        &mut self,
        defining_module: &str,
        member: &ModuleMember,
        use_span: Span,
        action: &str,
    ) {
        if member.visibility == DeclaredVisibility::Private
            && self.module_name.as_deref() != Some(defining_module)
        {
            self.diagnostics.push(
                Diagnostic::error(
                    "J0205",
                    format!(
                        "cannot {action} private symbol `{defining_module}.{}`",
                        member.name
                    ),
                    use_span,
                    "private symbols are visible only inside their defining module",
                )
                .with_secondary(member.span, "private declaration is here"),
            );
        }
    }

    fn external_symbol(
        &mut self,
        scope: ScopeId,
        canonical_path: &str,
        member: &ModuleMember,
    ) -> SymbolId {
        let key = SymbolKey {
            namespace: member.namespace,
            name: canonical_path.to_owned(),
        };
        if let Some(symbol) = self.external_symbols.get(&key) {
            return *symbol;
        }
        let id = SymbolId(self.symbols.len());
        self.symbols.push(Symbol {
            id,
            name: member.name.clone(),
            kind: member.kind,
            namespace: member.namespace,
            scope,
            owner: None,
            span: member.span,
            visibility: member.visibility,
            origin: SymbolOrigin::Imported,
            canonical_path: Some(canonical_path.to_owned()),
            qualified_id: Some(QualifiedSymbolId::from_path(
                member.namespace,
                canonical_path,
            )),
            function_signature: member.function_signature.clone(),
            record_interface: member.record_interface.clone(),
            enum_interface: member.enum_interface.clone(),
        });
        self.external_symbols.insert(key, id);
        id
    }

    fn resolve_name(&mut self, scope: ScopeId, name: &str, span: Span, namespace: Namespace) {
        if let Some(symbol) = self.lookup(scope, namespace, name) {
            self.references.push(ResolvedReference {
                span,
                namespace,
                symbol,
            });
        } else {
            self.unresolved.push(UnresolvedReference {
                span,
                path: name.to_owned(),
                namespace,
            });
        }
    }

    fn lookup(&self, mut scope: ScopeId, namespace: Namespace, name: &str) -> Option<SymbolId> {
        let key = SymbolKey {
            namespace,
            name: name.to_owned(),
        };
        loop {
            let current = &self.scopes[scope.index()];
            if let Some(symbol) = current.symbols.get(&key) {
                return Some(*symbol);
            }
            scope = current.parent?;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn define(
        &mut self,
        scope: ScopeId,
        name: &str,
        namespace: Namespace,
        kind: SymbolKind,
        span: Span,
        owner: Option<SymbolId>,
        visibility: DeclaredVisibility,
        origin: SymbolOrigin,
    ) -> SymbolId {
        let id = SymbolId(self.symbols.len());
        let key = SymbolKey {
            namespace,
            name: name.to_owned(),
        };
        let existing = self.scopes[scope.index()].symbols.get(&key).copied();
        self.symbols.push(Symbol {
            id,
            name: name.to_owned(),
            kind,
            namespace,
            scope,
            owner,
            span,
            visibility,
            origin,
            canonical_path: None,
            qualified_id: None,
            function_signature: None,
            record_interface: None,
            enum_interface: None,
        });
        if let Some(existing) = existing {
            let first = self.symbols[existing.index()].span;
            self.diagnostics.push(
                Diagnostic::error(
                    "J0200",
                    format!("duplicate definition of `{name}`"),
                    span,
                    "duplicate declaration in the same namespace and scope",
                )
                .with_secondary(first, "first declaration is here"),
            );
        } else {
            self.scopes[scope.index()].symbols.insert(key, id);
        }
        id
    }

    fn set_canonical_path(&mut self, symbol: SymbolId, canonical_path: &str) {
        let entry = &mut self.symbols[symbol.index()];
        entry.canonical_path = Some(canonical_path.to_owned());
        entry.qualified_id = Some(QualifiedSymbolId::from_path(
            entry.namespace,
            canonical_path,
        ));
    }

    fn new_scope(
        &mut self,
        parent: Option<ScopeId>,
        owner: Option<SymbolId>,
        kind: ScopeKind,
        span: Span,
    ) -> ScopeId {
        let id = ScopeId(self.scopes.len());
        self.scopes.push(Scope {
            id,
            parent,
            owner,
            kind,
            span,
            children: Vec::new(),
            symbols: DeterministicMap::new(),
        });
        if let Some(parent) = parent {
            self.scopes[parent.index()].children.push(id);
        }
        id
    }
}

fn visibility(is_public: bool) -> DeclaredVisibility {
    if is_public {
        DeclaredVisibility::Public
    } else {
        DeclaredVisibility::Private
    }
}

fn visit_import_graph(
    module: &str,
    adjacency: &DeterministicMap<String, Vec<(String, Span)>>,
    states: &mut DeterministicMap<String, u8>,
    stack: &mut Vec<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some(1 | 2) = states.get(module).copied() {
        return;
    }
    states.insert(module.to_owned(), 1);
    stack.push(module.to_owned());
    if let Some(edges) = adjacency.get(module) {
        for (target, span) in edges {
            match states.get(target).copied() {
                Some(1) => {
                    let start = stack
                        .iter()
                        .position(|entry| entry == target)
                        .unwrap_or_default();
                    let mut cycle = stack[start..].to_vec();
                    cycle.push(target.clone());
                    diagnostics.push(Diagnostic::error(
                        "J0204",
                        format!("cyclic value imports: {}", cycle.join(" -> ")),
                        *span,
                        "this value import closes a module cycle",
                    ));
                }
                Some(2) => {}
                _ => visit_import_graph(target, adjacency, states, stack, diagnostics),
            }
        }
    }
    let popped = stack.pop();
    debug_assert_eq!(popped.as_deref(), Some(module));
    states.insert(module.to_owned(), 2);
}

fn module_member(item: &Item) -> ModuleMember {
    let (name, namespace, kind, span, visibility) = match item {
        Item::Function(function) => (
            &function.name.text,
            Namespace::Value,
            SymbolKind::Function,
            function.name.span,
            visibility(function.is_public),
        ),
        Item::Struct(record) => (
            &record.name.text,
            Namespace::Type,
            SymbolKind::Struct,
            record.name.span,
            visibility(record.is_public),
        ),
        Item::Component(record) => (
            &record.name.text,
            Namespace::Type,
            SymbolKind::Component,
            record.name.span,
            visibility(record.is_public),
        ),
        Item::Enum(declaration) => (
            &declaration.name.text,
            Namespace::Type,
            SymbolKind::Enum,
            declaration.name.span,
            visibility(declaration.is_public),
        ),
        Item::ExternBlock(_) => unreachable!("extern blocks have one member per function"),
    };
    ModuleMember {
        name: name.clone(),
        namespace,
        kind,
        span,
        visibility,
        function_signature: None,
        record_interface: None,
        enum_interface: None,
    }
}

fn module_function_signature(
    source: &SourceFile,
    module_name: &str,
    imports: &[Path],
    function: &Function,
) -> ModuleFunctionSignature {
    let generics: Vec<_> = function
        .generic_parameters
        .iter()
        .map(|generic| generic.name.text.as_str())
        .collect();
    ModuleFunctionSignature {
        extern_abi: None,
        is_unsafe: false,
        generic_count: generics.len(),
        generic_bounds: module_generic_bounds(&function.generic_parameters),
        parameters: function
            .parameters
            .iter()
            .map(|parameter| module_type(source, module_name, imports, &generics, &parameter.ty))
            .collect(),
        result: function.return_type.as_ref().map_or_else(
            || ModuleType::Builtin {
                name: "Unit".to_owned(),
                arguments: Vec::new(),
            },
            |ty| module_type(source, module_name, imports, &generics, ty),
        ),
    }
}

fn module_extern_member(
    source: &SourceFile,
    module_name: &str,
    imports: &[Path],
    block: &jadren_parser::ExternBlock,
    function: &jadren_parser::ExternFunction,
) -> ModuleMember {
    ModuleMember {
        name: function.name.text.clone(),
        namespace: Namespace::Value,
        kind: SymbolKind::Function,
        span: function.name.span,
        visibility: DeclaredVisibility::Public,
        function_signature: Some(module_extern_function_signature(
            source,
            module_name,
            imports,
            block,
            function,
        )),
        record_interface: None,
        enum_interface: None,
    }
}

fn module_extern_function_signature(
    source: &SourceFile,
    module_name: &str,
    imports: &[Path],
    block: &jadren_parser::ExternBlock,
    function: &jadren_parser::ExternFunction,
) -> ModuleFunctionSignature {
    let parameters = function
        .parameters
        .iter()
        .map(|parameter| module_type(source, module_name, imports, &[], &parameter.ty))
        .collect();
    ModuleFunctionSignature {
        extern_abi: Some(block.abi.clone()),
        is_unsafe: function.is_unsafe,
        generic_count: 0,
        generic_bounds: Vec::new(),
        parameters,
        result: function.return_type.as_ref().map_or_else(
            || ModuleType::Builtin {
                name: "Unit".to_owned(),
                arguments: Vec::new(),
            },
            |ty| module_type(source, module_name, imports, &[], ty),
        ),
    }
}

fn insert_module_member(
    interface: &mut ModuleInterface,
    diagnostics: &mut Vec<Diagnostic>,
    module_name: &str,
    member: ModuleMember,
) {
    let key = SymbolKey {
        namespace: member.namespace,
        name: member.name.clone(),
    };
    if let Some(existing) = interface.members.get(&key) {
        if existing.span.source != member.span.source {
            diagnostics.push(
                Diagnostic::error(
                    "J0201",
                    format!(
                        "duplicate module member `{}` in `{module_name}`",
                        member.name
                    ),
                    member.span,
                    "duplicate declaration across module files",
                )
                .with_secondary(existing.span, "first declaration is here"),
            );
        }
    } else {
        interface.members.insert(key, member);
    }
}

fn module_record_interface(
    source: &SourceFile,
    module_name: &str,
    imports: &[Path],
    record: &RecordDeclaration,
) -> ModuleRecordInterface {
    let generics: Vec<_> = record
        .generic_parameters
        .iter()
        .map(|generic| generic.name.text.as_str())
        .collect();
    ModuleRecordInterface {
        module_name: module_name.to_owned(),
        generic_count: generics.len(),
        generic_bounds: module_generic_bounds(&record.generic_parameters),
        repr: annotation_repr(&record.annotations),
        fields: record
            .fields
            .iter()
            .map(|field| ModuleRecordField {
                name: field.name.text.clone(),
                ty: module_type(source, module_name, imports, &generics, &field.ty),
                visibility: visibility(field.is_public),
                span: field.name.span,
            })
            .collect(),
    }
}

fn module_enum_interface(
    source: &SourceFile,
    module_name: &str,
    imports: &[Path],
    declaration: &EnumDeclaration,
) -> ModuleEnumInterface {
    let generics: Vec<_> = declaration
        .generic_parameters
        .iter()
        .map(|generic| generic.name.text.as_str())
        .collect();
    ModuleEnumInterface {
        module_name: module_name.to_owned(),
        generic_count: generics.len(),
        generic_bounds: module_generic_bounds(&declaration.generic_parameters),
        repr: annotation_repr(&declaration.annotations),
        variants: declaration
            .variants
            .iter()
            .map(|variant| ModuleEnumVariant {
                name: variant.name.text.clone(),
                fields: variant
                    .fields
                    .iter()
                    .map(|field| module_type(source, module_name, imports, &generics, &field.ty))
                    .collect(),
                span: variant.name.span,
            })
            .collect(),
    }
}

fn module_generic_bounds(generics: &[GenericParameter]) -> Vec<Vec<BuiltinTrait>> {
    generics
        .iter()
        .map(|generic| {
            generic
                .bounds
                .iter()
                .filter_map(|bound| match bound {
                    TypeRef::Path {
                        path, arguments, ..
                    } if arguments.is_empty() => path
                        .segments
                        .last()
                        .and_then(|name| BuiltinTrait::from_name(&name.text)),
                    _ => None,
                })
                .collect()
        })
        .collect()
}

fn module_type(
    source: &SourceFile,
    module_name: &str,
    imports: &[Path],
    generics: &[&str],
    ty: &TypeRef,
) -> ModuleType {
    match ty {
        TypeRef::Path {
            path, arguments, ..
        } => {
            let spelling = path_text(path);
            if path.segments.len() == 1
                && let Some(index) = generics.iter().position(|generic| *generic == spelling)
            {
                return ModuleType::GenericParameter(index);
            }
            let arguments: Vec<_> = arguments
                .iter()
                .map(|argument| module_type(source, module_name, imports, generics, argument))
                .collect();
            if is_builtin_type_name(&spelling) {
                ModuleType::Builtin {
                    name: spelling,
                    arguments,
                }
            } else {
                ModuleType::Nominal {
                    canonical_path: canonical_interface_path(module_name, imports, path),
                    arguments,
                }
            }
        }
        TypeRef::Array {
            element, length, ..
        } => {
            let text = source.slice(*length).unwrap_or_default().replace('_', "");
            let Ok(length) = text.parse::<u64>() else {
                return ModuleType::Error;
            };
            ModuleType::Array {
                element: Box::new(module_type(source, module_name, imports, generics, element)),
                length,
            }
        }
        TypeRef::Capability {
            capability, inner, ..
        } => ModuleType::Capability {
            capability: match capability {
                TypeCapability::Owned => jadren_types::Capability::Owned,
                TypeCapability::Read => jadren_types::Capability::Read,
                TypeCapability::Write => jadren_types::Capability::Write,
            },
            inner: Box::new(module_type(source, module_name, imports, generics, inner)),
        },
        TypeRef::Function {
            parameters,
            return_type,
            ..
        } => ModuleType::Function {
            parameters: parameters
                .iter()
                .map(|parameter| module_type(source, module_name, imports, generics, parameter))
                .collect(),
            result: Box::new(return_type.as_deref().map_or_else(
                || ModuleType::Builtin {
                    name: "Unit".to_owned(),
                    arguments: Vec::new(),
                },
                |result| module_type(source, module_name, imports, generics, result),
            )),
        },
    }
}

fn canonical_interface_path(module_name: &str, imports: &[Path], path: &Path) -> String {
    let first = &path.segments[0].text;
    if let Some(import) = imports.iter().find(|import| {
        import
            .segments
            .last()
            .is_some_and(|name| name.text == *first)
    }) {
        let mut canonical = path_text(import);
        for segment in path.segments.iter().skip(1) {
            canonical.push('.');
            canonical.push_str(&segment.text);
        }
        canonical
    } else if path.segments.len() == 1 {
        format!("{module_name}.{first}")
    } else {
        path_text(path)
    }
}

fn is_builtin_type_name(name: &str) -> bool {
    matches!(
        name,
        "Bool"
            | "Buffer"
            | "Char"
            | "Float16"
            | "F16"
            | "Float2"
            | "Float32"
            | "F32"
            | "Float3"
            | "Float64"
            | "F64"
            | "Int8"
            | "Int16"
            | "Int32"
            | "Int64"
            | "IntSize"
            | "Never"
            | "Option"
            | "Pointer"
            | "Result"
            | "Slice"
            | "Status"
            | "String"
            | "UInt8"
            | "UInt16"
            | "UInt32"
            | "UInt64"
            | "UIntSize"
            | "Unit"
    )
}

fn path_text(path: &Path) -> String {
    path.segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect::<Vec<_>>()
        .join(".")
}

fn annotation_repr(annotations: &[Annotation]) -> AbiRepr {
    annotations
        .iter()
        .find(|annotation| path_text(&annotation.name) == "repr")
        .and_then(|annotation| {
            (annotation.arguments.len() == 1).then(|| match &annotation.arguments[0].value {
                Expression::Name(name) if name.text == "C" => AbiRepr::C,
                _ => AbiRepr::Jadren,
            })
        })
        .unwrap_or(AbiRepr::Jadren)
}

fn expression_path(expression: &Expression) -> Option<(String, Span)> {
    match expression {
        Expression::Name(name) => Some((name.text.clone(), name.span)),
        Expression::Field { base, field, span } => {
            let (mut path, _) = expression_path(base)?;
            path.push('.');
            path.push_str(&field.text);
            Some((path, *span))
        }
        _ => None,
    }
}

fn is_binding_path(path: &Path) -> bool {
    path.segments.len() == 1
        && path.segments[0]
            .text
            .chars()
            .next()
            .is_some_and(char::is_lowercase)
}

#[cfg(test)]
mod tests {
    use jadren_lexer::lex;
    use jadren_parser::{AstFile, parse};
    use jadren_source::{SourceFile, SourceManager};

    use super::{
        ModuleCatalog, Namespace, ScopeKind, SymbolKind, SymbolOrigin, resolve,
        resolve_with_modules,
    };

    fn parse_file(source: &SourceFile) -> AstFile {
        let lexed = lex(source);
        assert!(!lexed.has_errors(), "test source must lex");
        let parsed = parse(source, &lexed.tokens);
        assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics);
        parsed.file
    }

    fn resolve_text(text: &str) -> (SourceManager, super::ResolutionOutput) {
        let mut sources = SourceManager::new();
        let id = sources.add("test.jdn", text).expect("source should fit");
        let source = sources.get(id).expect("source exists");
        let file = parse_file(source);
        let output = resolve(source, &file);
        (sources, output)
    }

    #[test]
    fn creates_scopes_and_resolves_forward_and_shadowed_values() {
        let text = r#"
fn first(a: Int32) {
    let x = a;
    { let x = 2; print(x) }
    second(x)
}
fn second(value: Int32) { print(value) }
"#;
        let (sources, output) = resolve_text(text);
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        assert!(
            output
                .scopes
                .iter()
                .any(|scope| scope.kind == ScopeKind::Block)
        );
        assert!(
            output
                .references
                .iter()
                .filter_map(|reference| output.symbol(reference.symbol))
                .any(|symbol| symbol.kind == SymbolKind::Function && symbol.name == "second")
        );

        let source = sources
            .get(output.scopes[0].span.source)
            .expect("source exists");
        let x_symbols: Vec<_> = output
            .references
            .iter()
            .filter(|reference| source.slice(reference.span) == Some("x"))
            .map(|reference| reference.symbol)
            .collect();
        assert_eq!(x_symbols.len(), 2);
        assert_ne!(
            x_symbols[0], x_symbols[1],
            "nested shadow must bind separately"
        );
    }

    #[test]
    fn reports_duplicates_in_one_scope_but_keeps_namespaces_separate() {
        let (_, output) = resolve_text(
            r#"
struct Item {}
fn Item(a: Int32, a: Int32) {
    let x = 1
    let x = 2
    { let x = 3 }
}
"#,
        );
        let duplicate_codes: Vec<_> = output
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect();
        assert_eq!(duplicate_codes, ["J0200", "J0200"]);
        let root = output.scope(output.root_scope).expect("root scope");
        assert!(
            root.symbols
                .keys()
                .any(|key| { key.name == "Item" && key.namespace == Namespace::Type })
        );
        assert!(
            root.symbols
                .keys()
                .any(|key| { key.name == "Item" && key.namespace == Namespace::Value })
        );
    }

    #[test]
    fn reports_duplicate_type_members_in_their_owner_scopes() {
        let (_, output) = resolve_text(
            r#"
struct Pair { value: Int32, value: Int32 }
enum Choice { First, First }
"#,
        );
        assert_eq!(
            output
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code)
                .collect::<Vec<_>>(),
            ["J0200", "J0200"]
        );
    }

    #[test]
    fn match_pattern_bindings_live_in_the_match_arm_scope() {
        let text = r#"
enum Outcome<T> { Ok(T) }
fn use(value: Outcome<Int32>) {
    match value { Ok(item) => print(item) }
}
"#;
        let (sources, output) = resolve_text(text);
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let source = sources
            .get(output.scopes[0].span.source)
            .expect("source exists");
        let item_reference = output
            .references
            .iter()
            .find(|reference| source.slice(reference.span) == Some("item"))
            .expect("pattern binding must resolve in arm body");
        let item = output
            .symbol(item_reference.symbol)
            .expect("resolved symbol exists");
        assert_eq!(item.kind, SymbolKind::Local);
        assert_eq!(
            output.scope(item.scope).map(|scope| scope.kind),
            Some(ScopeKind::MatchArm)
        );
        assert!(
            output
                .unresolved
                .iter()
                .any(|reference| reference.path == "Ok"),
            "unqualified enum constructors remain for module resolution"
        );
    }

    #[test]
    fn retains_unresolved_names_for_module_resolution_without_cascade() {
        let (_, output) = resolve_text("fn main() { load(missing) }");
        assert!(!output.has_errors());
        assert_eq!(
            output
                .unresolved
                .iter()
                .map(|reference| reference.path.as_str())
                .collect::<Vec<_>>(),
            ["load", "missing"]
        );
    }

    #[test]
    fn resolves_every_specified_core_type_name() {
        let (_, output) = resolve_text(
            "fn scalars(a: Char, b: IntSize, c: UIntSize, d: Float16, e: Float2, f: Float3, g: Float8) -> Unit {}",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        assert!(output.unresolved.is_empty(), "{:?}", output.unresolved);
    }

    #[test]
    fn resolves_and_catalogs_extern_c_functions() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "ffi.jdn",
                r#"
module ffi.hash;
extern "C" {
    unsafe fn external_hash(data: Pointer<UInt8>, count: UIntSize) -> UInt64;
}
fn use_hash(data: Pointer<UInt8>, count: UIntSize) -> UInt64 {
    return external_hash(data, count)
}
"#,
            )
            .expect("source should fit");
        let source = sources.get(id).expect("source");
        let file = parse_file(source);
        let mut catalog = ModuleCatalog::new();
        catalog.add_file(source, &file);
        let output = resolve_with_modules(source, &file, &catalog);
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        assert!(output.unresolved.is_empty(), "{:?}", output.unresolved);

        let member = catalog
            .module("ffi.hash")
            .and_then(|module| {
                module.members.get(&super::SymbolKey {
                    namespace: Namespace::Value,
                    name: "external_hash".to_owned(),
                })
            })
            .expect("extern function module member");
        let signature = member
            .function_signature
            .as_ref()
            .expect("extern function signature");
        assert_eq!(signature.extern_abi.as_deref(), Some("C"));
        assert!(signature.is_unsafe);
        assert!(output.references.iter().any(|reference| {
            output.symbol(reference.symbol).is_some_and(|symbol| {
                symbol.name == "external_hash"
                    && symbol
                        .function_signature
                        .as_ref()
                        .is_some_and(|signature| signature.extern_abi.as_deref() == Some("C"))
            })
        }));
    }

    #[test]
    fn resolves_core_traits_and_exports_generic_bounds() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "test.jdn",
                "module test; pub fn number<T: Numeric + Equatable>(value: T) -> T { return value }",
            )
            .expect("source should fit");
        let source = sources.get(id).expect("source");
        let file = parse_file(source);
        let output = resolve(source, &file);
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        assert!(output.unresolved.is_empty(), "{:?}", output.unresolved);

        let mut catalog = ModuleCatalog::new();
        catalog.add_file(source, &file);
        let signature = catalog
            .module("test")
            .and_then(|module| {
                module.members.get(&super::SymbolKey {
                    namespace: Namespace::Value,
                    name: "number".to_owned(),
                })
            })
            .and_then(|member| member.function_signature.as_ref())
            .expect("function signature");
        assert_eq!(
            signature.generic_bounds,
            vec![vec![
                jadren_types::BuiltinTrait::Numeric,
                jadren_types::BuiltinTrait::Equatable,
            ]]
        );
    }

    #[test]
    fn resolves_imported_members_module_aliases_and_qualified_calls() {
        let mut sources = SourceManager::new();
        let math_id = sources
            .add(
                "math.jdn",
                r#"
module math
pub struct Vec3 {}
pub fn length() -> Float32 { return 0.0f32 }
"#,
            )
            .expect("source should fit");
        let app_id = sources
            .add(
                "app.jdn",
                r#"
module game.app
import math.Vec3
import math
fn measure(value: Vec3) -> Float32 { return math.length() }
"#,
            )
            .expect("source should fit");
        let math = parse_file(sources.get(math_id).expect("math source"));
        let app_source = sources.get(app_id).expect("app source");
        let app = parse_file(app_source);
        let mut catalog = ModuleCatalog::new();
        catalog.add_file(sources.get(math_id).expect("math source"), &math);
        catalog.add_file(app_source, &app);

        let output = resolve_with_modules(app_source, &app, &catalog);
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        assert_eq!(output.module_name.as_deref(), Some("game.app"));
        assert_eq!(output.imports.len(), 2);
        assert!(output.imports.iter().any(|binding| {
            binding.local_name == "Vec3"
                && binding.namespace == Namespace::Type
                && binding.canonical_path == "math.Vec3"
                && output
                    .symbol(binding.symbol)
                    .and_then(super::Symbol::nominal_type_id)
                    .is_some()
        }));
        assert!(output.imports.iter().any(|binding| {
            binding.local_name == "math" && binding.namespace == Namespace::Module
        }));
        assert!(output.references.iter().any(|reference| {
            output.symbol(reference.symbol).is_some_and(|symbol| {
                symbol.origin == SymbolOrigin::Imported
                    && symbol.canonical_path.as_deref() == Some("math.length")
                    && symbol.function_signature.as_ref().is_some_and(|signature| {
                        signature.parameters.is_empty()
                            && signature.result
                                == super::ModuleType::Builtin {
                                    name: "Float32".to_owned(),
                                    arguments: Vec::new(),
                                }
                    })
            })
        }));
    }

    #[test]
    fn reports_unresolved_imports_and_known_module_members() {
        let mut sources = SourceManager::new();
        let math_id = sources
            .add("math.jdn", "module math; pub struct Vec3 {}")
            .expect("source should fit");
        let app_id = sources
            .add(
                "app.jdn",
                "module app; import missing.Value; fn use(value: math.Unknown) {}",
            )
            .expect("source should fit");
        let math = parse_file(sources.get(math_id).expect("math source"));
        let app_source = sources.get(app_id).expect("app source");
        let app = parse_file(app_source);
        let mut catalog = ModuleCatalog::new();
        catalog.add_file(sources.get(math_id).expect("math source"), &math);
        catalog.add_file(app_source, &app);

        let output = resolve_with_modules(app_source, &app, &catalog);
        assert_eq!(
            output
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code)
                .collect::<Vec<_>>(),
            ["J0202", "J0203"]
        );
    }

    #[test]
    fn catalog_reports_duplicate_members_across_files_of_one_module() {
        let mut sources = SourceManager::new();
        let first_id = sources
            .add("first.jdn", "module common; fn update() {}")
            .expect("source should fit");
        let second_id = sources
            .add("second.jdn", "module common; fn update() {}")
            .expect("source should fit");
        let first = parse_file(sources.get(first_id).expect("first source"));
        let second = parse_file(sources.get(second_id).expect("second source"));
        let mut catalog = ModuleCatalog::new();
        catalog.add_file(sources.get(first_id).expect("first source"), &first);
        catalog.add_file(sources.get(second_id).expect("second source"), &second);

        assert_eq!(catalog.diagnostics().len(), 1);
        assert_eq!(catalog.diagnostics()[0].code, "J0201");
    }

    #[test]
    fn catalog_rejects_value_import_cycles_but_allows_type_only_cycles() {
        let mut sources = SourceManager::new();
        let a_id = sources
            .add(
                "a.jdn",
                "module a; import b.run_b; pub struct A {} pub fn run_a() {}",
            )
            .expect("source should fit");
        let b_id = sources
            .add(
                "b.jdn",
                "module b; import a.run_a; pub struct B {} pub fn run_b() {}",
            )
            .expect("source should fit");
        let a = parse_file(sources.get(a_id).expect("a source"));
        let b = parse_file(sources.get(b_id).expect("b source"));
        let mut values = ModuleCatalog::new();
        values.add_file(sources.get(a_id).expect("a source"), &a);
        values.add_file(sources.get(b_id).expect("b source"), &b);
        values.finalize();
        assert!(
            values
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == "J0204")
        );

        let mut type_sources = SourceManager::new();
        let a_id = type_sources
            .add("type-a.jdn", "module a; import b.B; pub struct A {}")
            .expect("source should fit");
        let b_id = type_sources
            .add("type-b.jdn", "module b; import a.A; pub struct B {}")
            .expect("source should fit");
        let a = parse_file(type_sources.get(a_id).expect("a source"));
        let b = parse_file(type_sources.get(b_id).expect("b source"));
        let mut types = ModuleCatalog::new();
        types.add_file(type_sources.get(a_id).expect("a source"), &a);
        types.add_file(type_sources.get(b_id).expect("b source"), &b);
        types.finalize();
        assert!(types.diagnostics().is_empty());
    }

    #[test]
    fn rejects_private_symbols_across_modules_but_still_binds_them() {
        let mut sources = SourceManager::new();
        let library_id = sources
            .add(
                "library.jdn",
                "module library; struct Secret {} fn hidden() {}",
            )
            .expect("source should fit");
        let app_id = sources
            .add(
                "app.jdn",
                r#"
module app
import library.Secret
import library
fn use(value: library.Secret) { library.hidden() }
"#,
            )
            .expect("source should fit");
        let library = parse_file(sources.get(library_id).expect("library source"));
        let app_source = sources.get(app_id).expect("app source");
        let app = parse_file(app_source);
        let mut catalog = ModuleCatalog::new();
        catalog.add_file(sources.get(library_id).expect("library source"), &library);
        catalog.add_file(app_source, &app);
        catalog.finalize();

        let output = resolve_with_modules(app_source, &app, &catalog);
        assert_eq!(
            output
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "J0205")
                .count(),
            3
        );
        assert!(
            output
                .references
                .iter()
                .filter_map(|reference| output.symbol(reference.symbol))
                .any(|symbol| symbol.canonical_path.as_deref() == Some("library.hidden")),
            "private targets stay bound to avoid cascading unknown-name errors"
        );
    }
}
