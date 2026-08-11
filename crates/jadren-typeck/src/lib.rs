//! Local type lowering, inference, and unification for the Jadren frontend.

use jadren_determinism::{DeterministicMap, DeterministicSet, Fingerprint, StableHasher};
use jadren_diagnostics::{Diagnostic, Severity};
use jadren_lexer::Operator;
use jadren_parser::{
    AstFile, Block, EnumDeclaration, Expression, Function, GenericParameter, Item, LiteralKind,
    MatchArm, Name, Pattern, RecordDeclaration, Statement, StructFieldValue, TypeCapability,
    TypeRef,
};
use jadren_resolve::{
    DeclaredVisibility, ModuleCatalog, ModuleEnumInterface, ModuleFunctionSignature,
    ModuleRecordInterface, ModuleType, Namespace, ResolutionOutput, Symbol, SymbolId, SymbolKind,
    SymbolOrigin,
};
use jadren_source::{SourceFile, SourceId, Span};
use jadren_types::{
    AbiRepr, BuiltinTrait, BuiltinTypeError, Capability, FloatWidth, GenericParameterId,
    MonomorphizationKey, NominalFieldLayout, NominalLayout, NominalLayoutKind, NominalTypeId,
    NominalVariantLayout, Substitution, TypeId, TypeKind, TypeStore, UnificationTable,
};

/// Stable per-file identity of a typed expression.
///
/// The index is assigned after type inference has been finalized and the
/// expression table has been put into source order. It is therefore suitable
/// for consumers that need to retain a typed-expression reference without
/// exposing the type checker's internal traversal order.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TypedExpressionId(usize);

impl TypedExpressionId {
    /// Creates an identity from a zero-based source-order index.
    #[must_use]
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    /// Returns the zero-based index in [`TypeCheckOutput::expressions`].
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// Syntax-level expression category retained by the typed-expression index.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpressionKind {
    /// Identifier expression.
    Name,
    /// Literal expression.
    Literal,
    /// Prefix unary expression.
    Unary,
    /// Binary or assignment expression.
    Binary,
    /// Function or constructor call.
    Call,
    /// Field access expression.
    Field,
    /// Index expression.
    Index,
    /// Postfix propagation expression.
    Try,
    /// Explicit scalar cast.
    Cast,
    /// Array literal.
    Array,
    /// Record construction expression.
    StructLiteral,
    /// Parenthesized expression.
    Group,
    /// Block expression.
    Block,
    /// Conditional expression.
    If,
    /// Pattern match expression.
    Match,
    /// Error recovery placeholder.
    Error,
}

impl ExpressionKind {
    /// Classifies a parser expression without inspecting or allocating its
    /// children. The mapping is intentionally syntax-level; inferred types
    /// remain in [`TypedExpression::ty`].
    #[must_use]
    pub const fn of(expression: &Expression) -> Self {
        match expression {
            Expression::Name(_) => Self::Name,
            Expression::Literal { .. } => Self::Literal,
            Expression::Unary { .. } => Self::Unary,
            Expression::Binary { .. } => Self::Binary,
            Expression::Call { .. } => Self::Call,
            Expression::Field { .. } => Self::Field,
            Expression::Index { .. } => Self::Index,
            Expression::Try { .. } => Self::Try,
            Expression::Cast { .. } => Self::Cast,
            Expression::Array { .. } => Self::Array,
            Expression::StructLiteral { .. } => Self::StructLiteral,
            Expression::Group { .. } => Self::Group,
            Expression::Block(_) => Self::Block,
            Expression::If { .. } => Self::If,
            Expression::Match { .. } => Self::Match,
            Expression::Error(_) => Self::Error,
        }
    }
}

/// Inferred type attached to one source expression.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TypedExpression {
    /// Stable per-file expression identity.
    pub id: TypedExpressionId,
    /// Syntax-level expression category.
    pub kind: ExpressionKind,
    /// Expression source range.
    pub span: Span,
    /// Canonical inferred type.
    pub ty: TypeId,
}

/// Immutable query over the finalized source-order typed-expression index.
///
/// The query deliberately filters the retained index instead of walking the
/// type-checker's inference stack. Consumers therefore observe the same
/// deterministic ordering as HIR, MIR, diagnostics, and editor features.
#[derive(Clone, Copy, Debug)]
pub struct TypedExpressionQuery<'a> {
    expressions: &'a [TypedExpression],
    source: Option<SourceId>,
    kind: Option<ExpressionKind>,
    span: Option<TypedExpressionSpanFilter>,
}

#[derive(Clone, Copy, Debug)]
enum TypedExpressionSpanFilter {
    Exact(Span),
    Within(Span),
    Intersecting(Span),
    At { source: SourceId, offset: usize },
}

impl<'a> TypedExpressionQuery<'a> {
    /// Creates a query over a finalized typed-expression slice.
    #[must_use]
    pub fn new(expressions: &'a [TypedExpression]) -> Self {
        Self {
            expressions,
            source: None,
            kind: None,
            span: None,
        }
    }

    /// Restricts results to one source file.
    #[must_use]
    pub fn source(mut self, source: SourceId) -> Self {
        self.source = Some(source);
        self
    }

    /// Restricts results to one syntax-level expression category.
    #[must_use]
    pub fn kind(mut self, kind: ExpressionKind) -> Self {
        self.kind = Some(kind);
        self
    }

    /// Restricts results to expressions with exactly the supplied span.
    #[must_use]
    pub fn exact_span(mut self, span: Span) -> Self {
        self.span = Some(TypedExpressionSpanFilter::Exact(span));
        self
    }

    /// Restricts results to expressions fully contained in the supplied span.
    #[must_use]
    pub fn within_span(mut self, span: Span) -> Self {
        self.span = Some(TypedExpressionSpanFilter::Within(span));
        self
    }

    /// Restricts results to expressions whose half-open ranges overlap the
    /// supplied span.
    #[must_use]
    pub fn intersecting_span(mut self, span: Span) -> Self {
        self.span = Some(TypedExpressionSpanFilter::Intersecting(span));
        self
    }

    /// Restricts results to expressions containing a source byte offset.
    /// Empty spans match only their exact offset.
    #[must_use]
    pub fn at(mut self, source: SourceId, offset: usize) -> Self {
        self.span = Some(TypedExpressionSpanFilter::At { source, offset });
        self
    }

    fn matches(self, expression: &TypedExpression) -> bool {
        if self
            .source
            .is_some_and(|source| expression.span.source != source)
        {
            return false;
        }
        if self.kind.is_some_and(|kind| expression.kind != kind) {
            return false;
        }
        match self.span {
            None => true,
            Some(TypedExpressionSpanFilter::Exact(span)) => expression.span == span,
            Some(TypedExpressionSpanFilter::Within(span)) => {
                expression.span.source == span.source
                    && expression.span.start >= span.start
                    && expression.span.end <= span.end
            }
            Some(TypedExpressionSpanFilter::Intersecting(span)) => {
                expression.span.source == span.source
                    && expression.span.start < span.end
                    && span.start < expression.span.end
            }
            Some(TypedExpressionSpanFilter::At { source, offset }) => {
                expression.span.source == source
                    && if expression.span.is_empty() {
                        expression.span.start == offset
                    } else {
                        expression.span.start <= offset && offset < expression.span.end
                    }
            }
        }
    }

    /// Iterates matching records in deterministic source-order.
    pub fn iter(self) -> impl DoubleEndedIterator<Item = &'a TypedExpression> + 'a {
        self.expressions
            .iter()
            .filter(move |expression| self.matches(expression))
    }

    /// Returns the first matching record in source-order.
    #[must_use]
    pub fn first(self) -> Option<&'a TypedExpression> {
        self.iter().next()
    }

    /// Returns the smallest matching source range, breaking ties by stable ID.
    /// This is useful for editor caret queries where nested expressions overlap.
    #[must_use]
    pub fn innermost(self) -> Option<&'a TypedExpression> {
        self.iter()
            .min_by_key(|expression| (expression.span.len(), expression.id))
    }
}

/// Backwards-compatible name for a typed expression record.
pub type ExpressionType = TypedExpression;

/// Early-return behavior selected for one postfix `?` expression.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PropagationKind {
    /// Extract `Some` or return `None` from the current function.
    OptionNone,
    /// Extract `Ok` or return `Error` from the current function.
    ResultError,
}

/// Semantic lowering input retained for future HIR/MIR control-flow construction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PropagationSite {
    /// Full postfix expression range.
    pub span: Span,
    /// Selected propagation family.
    pub kind: PropagationKind,
    /// Successful value produced by the expression.
    pub success_type: TypeId,
    /// Residual payload (`Unit` for `None`, error type for `Error`).
    pub residual_type: TypeId,
    /// Current function return carrier.
    pub return_type: TypeId,
}

/// One deduplicated concrete generic function instance requested by this source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MonomorphizationInstance {
    /// Session-local declaration symbol.
    pub declaration: SymbolId,
    /// Stable cross-session instance identity.
    pub key: MonomorphizationKey,
    /// Concrete type arguments in generic parameter order.
    pub arguments: Vec<TypeId>,
    /// Source call range that requested this concrete instance.
    pub span: Span,
}

/// Typed allocation owned by one lexical `region` statement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegionAllocationSite {
    /// Full allocation call range.
    pub span: Span,
    /// Resolver symbol of the region handle.
    pub region: SymbolId,
    /// Contextually inferred allocated value type.
    pub result_type: TypeId,
}

/// Semantic type result for one source file.
#[derive(Clone, Debug)]
pub struct TypeCheckOutput {
    /// Canonical type storage for every returned [`TypeId`].
    pub types: TypeStore,
    /// Type for each resolver symbol-table slot when known.
    pub symbol_types: Vec<Option<TypeId>>,
    /// Expression types in stable source traversal order.
    pub expressions: Vec<ExpressionType>,
    /// Explicit early-return lowering sites for postfix `?`.
    pub propagation_sites: Vec<PropagationSite>,
    /// Concrete generic function instances requested by calls.
    pub monomorphizations: Vec<MonomorphizationInstance>,
    /// Region allocation calls retained for HIR/MIR ownership lowering.
    pub region_allocations: Vec<RegionAllocationSite>,
    /// Record/component/enum layouts required by MIR and JIR lowering.
    pub nominal_layouts: Vec<NominalLayout>,
    /// Local type diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

impl TypeCheckOutput {
    /// Starts a deterministic query over the finalized typed-expression index.
    #[must_use]
    pub fn query_typed_expressions(&self) -> TypedExpressionQuery<'_> {
        TypedExpressionQuery::new(&self.expressions)
    }

    /// Looks up a typed expression only when the dense index and stored ID
    /// agree. This prevents stale IDs from silently reading a reordered entry.
    #[must_use]
    pub fn typed_expression(&self, id: TypedExpressionId) -> Option<&TypedExpression> {
        self.expressions
            .get(id.index())
            .filter(|expression| expression.id == id)
    }

    /// Returns the innermost typed expression containing a source byte offset.
    #[must_use]
    pub fn typed_expression_at(&self, source: SourceId, offset: usize) -> Option<&TypedExpression> {
        self.query_typed_expressions()
            .at(source, offset)
            .innermost()
    }

    /// Returns the first typed expression with an exact source span.
    #[must_use]
    pub fn typed_expression_exact_span(&self, span: Span) -> Option<&TypedExpression> {
        self.query_typed_expressions().exact_span(span).first()
    }

    /// Returns the inferred type of a symbol.
    #[must_use]
    pub fn symbol_type(&self, symbol: SymbolId) -> Option<TypeId> {
        self.symbol_types.get(symbol.index()).copied().flatten()
    }

    /// Returns whether type errors occurred.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }
}

/// Lowers explicit types and infers local expression/binding types.
#[must_use]
pub fn check_types(
    source: &SourceFile,
    file: &AstFile,
    resolution: &ResolutionOutput,
) -> TypeCheckOutput {
    Checker::new(source, file, resolution, None).run()
}

/// Lowers types with the complete compiler-session module catalog available.
///
/// A function imported from another module can carry a nominal type in its
/// signature (for example `Result<Int32, CoreError>`).  The resolver already
/// stores that signature as a portable interface, but a source-local checker
/// also needs the defining record/enum layout for MIR-to-JIR lowering.  The
/// catalog-aware entry point makes that transitive package ABI explicit while
/// preserving [`check_types`] for isolated frontend callers and unit tests.
#[must_use]
pub fn check_types_with_modules(
    source: &SourceFile,
    file: &AstFile,
    resolution: &ResolutionOutput,
    catalog: &ModuleCatalog,
) -> TypeCheckOutput {
    Checker::new(source, file, resolution, Some(catalog)).run()
}

struct Checker<'a> {
    source: &'a SourceFile,
    file: &'a AstFile,
    resolution: &'a ResolutionOutput,
    module_catalog: Option<&'a ModuleCatalog>,
    types: TypeStore,
    unification: UnificationTable,
    symbol_types: Vec<Option<TypeId>>,
    expressions: Vec<ExpressionType>,
    diagnostics: Vec<Diagnostic>,
    records: DeterministicMap<NominalTypeId, RecordInfo>,
    enums: DeterministicMap<NominalTypeId, EnumInfo>,
    propagation_sites: Vec<PropagationSite>,
    monomorphizations: DeterministicMap<MonomorphizationKey, MonomorphizationInstance>,
    generic_bounds: DeterministicMap<GenericParameterId, Vec<BuiltinTrait>>,
    region_allocations: Vec<RegionAllocationSite>,
    loop_depth: usize,
    allow_index_iterable: bool,
}

#[derive(Clone, Debug)]
struct RecordInfo {
    module_name: String,
    generic_parameters: Vec<GenericParameterId>,
    repr: AbiRepr,
    field_order: Vec<String>,
    fields: DeterministicMap<String, RecordFieldInfo>,
}

#[derive(Clone, Debug)]
struct RecordFieldInfo {
    ty: TypeId,
    visibility: DeclaredVisibility,
    span: Span,
}

#[derive(Clone, Debug)]
struct EnumInfo {
    canonical_path: String,
    generic_parameters: Vec<GenericParameterId>,
    repr: AbiRepr,
    variant_order: Vec<String>,
    variants: DeterministicMap<String, EnumVariantInfo>,
}

#[derive(Clone, Debug)]
struct EnumVariantInfo {
    fields: Vec<TypeId>,
    span: Span,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PatternCoverage {
    None,
    All,
    Variant(String),
}

impl<'a> Checker<'a> {
    fn new(
        source: &'a SourceFile,
        file: &'a AstFile,
        resolution: &'a ResolutionOutput,
        module_catalog: Option<&'a ModuleCatalog>,
    ) -> Self {
        Self {
            source,
            file,
            resolution,
            module_catalog,
            types: TypeStore::new(),
            unification: UnificationTable::new(),
            symbol_types: vec![None; resolution.symbols.len()],
            expressions: Vec::new(),
            diagnostics: Vec::new(),
            records: DeterministicMap::new(),
            enums: DeterministicMap::new(),
            propagation_sites: Vec::new(),
            monomorphizations: DeterministicMap::new(),
            generic_bounds: DeterministicMap::new(),
            region_allocations: Vec::new(),
            loop_depth: 0,
            allow_index_iterable: false,
        }
    }

    fn run(mut self) -> TypeCheckOutput {
        self.declare_generic_parameters();
        self.declare_local_generic_bounds();
        self.declare_external_records();
        self.declare_external_enums();
        for item in &self.file.items {
            if let Item::Struct(record) | Item::Component(record) = item {
                self.declare_local_record(record);
            }
            if let Item::Enum(declaration) = item {
                self.declare_local_enum(declaration);
            }
        }
        self.validate_export_annotations();
        self.validate_abi_representations();
        self.declare_external_functions();
        for item in &self.file.items {
            if let Item::Function(function) = item {
                self.declare_function(function);
            }
        }
        for item in &self.file.items {
            match item {
                Item::Function(function) => self.check_function(function),
                Item::Struct(record) | Item::Component(record) => {
                    for field in &record.fields {
                        let ty = self.lower_type(&field.ty);
                        self.assign_declaration(field.name.span, ty);
                    }
                }
                Item::Enum(declaration) => {
                    for variant in &declaration.variants {
                        for field in &variant.fields {
                            let ty = self.lower_type(&field.ty);
                            if let Some(name) = &field.name {
                                self.assign_declaration(name.span, ty);
                            }
                        }
                    }
                }
                Item::ExternBlock(_) => {}
            }
        }
        self.finalize_types();
        let mut nominal_layouts = self.export_nominal_layouts();
        nominal_layouts.extend(self.export_catalog_nominal_layouts());
        nominal_layouts.sort_by_key(|layout| layout.constructor);
        nominal_layouts.dedup_by_key(|layout| layout.constructor);
        TypeCheckOutput {
            types: self.types,
            symbol_types: self.symbol_types,
            expressions: self.expressions,
            propagation_sites: self.propagation_sites,
            monomorphizations: self.monomorphizations.into_values().collect(),
            region_allocations: self.region_allocations,
            nominal_layouts,
            diagnostics: self.diagnostics,
        }
    }

    fn declare_generic_parameters(&mut self) {
        let generic_ids: Vec<_> = self
            .resolution
            .symbols
            .iter()
            .filter(|symbol| symbol.kind == SymbolKind::GenericParameter)
            .map(|symbol| symbol.id)
            .collect();
        for symbol_id in generic_ids {
            let symbol = &self.resolution.symbols[symbol_id.index()];
            let owner_symbol = symbol.owner.and_then(|owner| self.resolution.symbol(owner));
            let owner = owner_symbol
                .and_then(|owner| owner.qualified_id)
                .map_or_else(
                    || {
                        let mut hasher = StableHasher::with_domain("jadren-local-generic-owner-v1");
                        hasher.write_u64(self.source.stable_hash());
                        hasher.write_u64(
                            owner_symbol.map_or(symbol.span.start, |owner| owner.span.start) as u64,
                        );
                        hasher.finish()
                    },
                    jadren_resolve::QualifiedSymbolId::fingerprint,
                );
            let index = self
                .resolution
                .symbols
                .iter()
                .filter(|candidate| {
                    candidate.kind == SymbolKind::GenericParameter
                        && candidate.owner == symbol.owner
                        && candidate.id.index() < symbol.id.index()
                })
                .count();
            let ty = self
                .types
                .intern(TypeKind::GenericParameter(GenericParameterId {
                    owner,
                    index,
                }));
            self.symbol_types[symbol_id.index()] = Some(ty);
        }
    }

    fn declare_local_generic_bounds(&mut self) {
        let parameters: Vec<_> = self
            .file
            .items
            .iter()
            .flat_map(item_generic_parameters)
            .cloned()
            .collect();
        for parameter in parameters {
            let Some(ty) = self
                .declaration_symbol(parameter.name.span)
                .and_then(|symbol| self.symbol_types[symbol.index()])
            else {
                continue;
            };
            let Some(TypeKind::GenericParameter(id)) = self.types.kind(ty) else {
                continue;
            };
            let id = *id;
            let mut bounds = Vec::new();
            for bound in &parameter.bounds {
                if let TypeRef::Path {
                    path,
                    arguments,
                    span,
                } = bound
                    && let Some(bound) = path
                        .segments
                        .last()
                        .and_then(|name| BuiltinTrait::from_name(&name.text))
                {
                    if arguments.is_empty() {
                        bounds.push(bound);
                    } else {
                        self.diagnostics.push(Diagnostic::error(
                            "J0317",
                            format!("trait `{}` does not accept type arguments", bound.name()),
                            *span,
                            "core trait bounds are non-generic",
                        ));
                    }
                }
            }
            self.generic_bounds.insert(id, bounds);
        }
    }

    fn register_external_bounds(&mut self, owner: Fingerprint, bounds: &[Vec<BuiltinTrait>]) {
        for (index, bounds) in bounds.iter().enumerate() {
            self.generic_bounds
                .insert(GenericParameterId { owner, index }, bounds.clone());
        }
    }

    fn declare_function(&mut self, function: &Function) {
        let parameters: Vec<_> = function
            .parameters
            .iter()
            .map(|parameter| {
                let ty = self.lower_type(&parameter.ty);
                self.assign_declaration(parameter.name.span, ty);
                ty
            })
            .collect();
        let result = function
            .return_type
            .as_ref()
            .map_or(self.types.core().unit, |ty| self.lower_type(ty));
        let function_type = self.types.intern(TypeKind::Function {
            parameters: parameters.into_boxed_slice(),
            result,
        });
        self.assign_declaration(function.name.span, function_type);
    }

    fn declare_external_functions(&mut self) {
        let signatures: Vec<_> = self
            .resolution
            .symbols
            .iter()
            .filter_map(|symbol| {
                symbol
                    .function_signature
                    .clone()
                    .map(|signature| (symbol.id, symbol.qualified_id, signature))
            })
            .collect();
        for (symbol, qualified_id, signature) in signatures {
            if self.symbol_types[symbol.index()].is_some() {
                continue;
            }
            let function_type = self.lower_module_signature(qualified_id, &signature);
            self.symbol_types[symbol.index()] = Some(function_type);
        }
    }

    fn declare_external_records(&mut self) {
        let interfaces: Vec<_> = self
            .resolution
            .symbols
            .iter()
            .filter_map(|symbol| {
                Some((
                    symbol.nominal_type_id()?,
                    symbol.qualified_id?,
                    symbol.record_interface.clone()?,
                ))
            })
            .collect();
        for (constructor, owner, interface) in interfaces {
            let info = self.lower_record_interface(owner, &interface);
            self.records.entry(constructor).or_insert(info);
        }
    }

    fn declare_local_record(&mut self, record: &RecordDeclaration) {
        let symbol = self
            .declaration_symbol(record.name.span)
            .and_then(|symbol| self.resolution.symbol(symbol));
        let constructor = symbol.and_then(Symbol::nominal_type_id).unwrap_or_else(|| {
            NominalTypeId::from_path(&format!(
                "{}#{}",
                self.source.path().display(),
                record.name.text
            ))
        });
        let fields = record
            .fields
            .iter()
            .map(|field| {
                let ty = self.lower_type(&field.ty);
                self.assign_declaration(field.name.span, ty);
                (
                    field.name.text.clone(),
                    RecordFieldInfo {
                        ty,
                        visibility: if field.is_public {
                            DeclaredVisibility::Public
                        } else {
                            DeclaredVisibility::Private
                        },
                        span: field.name.span,
                    },
                )
            })
            .collect();
        let repr = self.annotation_repr(&record.annotations);
        self.records.insert(
            constructor,
            RecordInfo {
                module_name: self.resolution.module_name.clone().unwrap_or_default(),
                generic_parameters: generic_parameter_ids(
                    self.generic_owner_fingerprint(symbol, record.name.span.start),
                    record.generic_parameters.len(),
                ),
                repr,
                field_order: record
                    .fields
                    .iter()
                    .map(|field| field.name.text.clone())
                    .collect(),
                fields,
            },
        );
    }

    fn lower_record_interface(
        &mut self,
        owner: jadren_resolve::QualifiedSymbolId,
        interface: &ModuleRecordInterface,
    ) -> RecordInfo {
        self.register_external_bounds(owner.fingerprint(), &interface.generic_bounds);
        let fields = interface
            .fields
            .iter()
            .map(|field| {
                (
                    field.name.clone(),
                    RecordFieldInfo {
                        ty: self.lower_module_type(Some(owner), &field.ty),
                        visibility: field.visibility,
                        span: field.span,
                    },
                )
            })
            .collect();
        RecordInfo {
            module_name: interface.module_name.clone(),
            generic_parameters: generic_parameter_ids(owner.fingerprint(), interface.generic_count),
            repr: interface.repr,
            field_order: interface
                .fields
                .iter()
                .map(|field| field.name.clone())
                .collect(),
            fields,
        }
    }

    fn declare_external_enums(&mut self) {
        let interfaces: Vec<_> = self
            .resolution
            .symbols
            .iter()
            .filter_map(|symbol| {
                Some((
                    symbol.nominal_type_id()?,
                    symbol.qualified_id?,
                    symbol.canonical_path.clone()?,
                    symbol.enum_interface.clone()?,
                ))
            })
            .collect();
        for (constructor, owner, canonical_path, interface) in interfaces {
            let info = self.lower_enum_interface(owner, &canonical_path, &interface);
            self.enums.entry(constructor).or_insert(info);
        }
    }

    fn annotation_repr(&mut self, annotations: &[jadren_parser::Annotation]) -> AbiRepr {
        for annotation in annotations {
            if annotation_path_text(annotation) != "repr" {
                continue;
            }
            if let [argument] = annotation.arguments.as_slice()
                && let Expression::Name(name) = &argument.value
                && name.text == "C"
            {
                return AbiRepr::C;
            }
            self.diagnostics.push(Diagnostic::error(
                "J0800",
                "unsupported `repr` annotation",
                annotation.span,
                "Jadren 0.1 supports only `@repr(C)`",
            ));
        }
        AbiRepr::Jadren
    }

    fn validate_abi_representations(&mut self) {
        for item in &self.file.items {
            if let Item::Function(function) = item
                && function
                    .annotations
                    .iter()
                    .any(|annotation| annotation_path_text(annotation) == "repr")
            {
                self.diagnostics.push(Diagnostic::error(
                    "J0802",
                    "`repr` is only valid on record, component, or enum declarations",
                    function.span,
                    "move `@repr(C)` to an ABI data declaration",
                ));
            }
        }

        let records: Vec<_> = self
            .records
            .iter()
            .map(|(constructor, record)| (*constructor, record.clone()))
            .collect();
        for (constructor, record) in records {
            if record.repr != AbiRepr::C {
                continue;
            }
            for name in &record.field_order {
                let Some(field) = record.fields.get(name) else {
                    continue;
                };
                let mut visiting = DeterministicSet::new();
                if !self.is_c_record_field_type(field.ty, &mut visiting) {
                    self.diagnostics.push(Diagnostic::error(
                        "J0801",
                        format!("field `{name}` is not representable in `repr(C)` type"),
                        field.span,
                        "use fixed-width scalar, array, pointer, or another `@repr(C)` type",
                    ));
                }
            }
            let _ = constructor;
        }

        let enums: Vec<_> = self
            .enums
            .iter()
            .map(|(constructor, declaration)| (*constructor, declaration.clone()))
            .collect();
        for (_, declaration) in enums {
            if declaration.repr != AbiRepr::C {
                continue;
            }
            for variant in declaration.variants.values() {
                for field in &variant.fields {
                    let mut visiting = DeterministicSet::new();
                    if !self.is_c_enum_payload_type(*field, &mut visiting) {
                        self.diagnostics.push(Diagnostic::error(
                            "J0803",
                            "payload field is not representable in `repr(C)` enum",
                            variant.span,
                            "use fixed-width scalar, array, pointer, or another `@repr(C)` type",
                        ));
                    }
                }
            }
        }
    }

    fn validate_export_annotations(&mut self) {
        let mut exported: DeterministicMap<String, Span> = DeterministicMap::new();
        let annotations: Vec<_> = self
            .file
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Function(function) => Some(function.annotations.as_slice()),
                _ => None,
            })
            .flatten()
            .filter(|annotation| {
                annotation_path_text(annotation) == "export"
                    && annotation.arguments.iter().any(|argument| {
                        argument
                            .name
                            .as_ref()
                            .is_some_and(|name| matches!(name.text.as_str(), "name" | "abi"))
                    })
            })
            .cloned()
            .collect();
        for annotation in annotations {
            let mut name = None;
            let mut abi = None;
            for argument in &annotation.arguments {
                let Some(argument_name) = argument.name.as_ref().map(|name| name.text.as_str())
                else {
                    continue;
                };
                match argument_name {
                    "name" => {
                        name = self.export_argument_string(&argument.value, true);
                    }
                    "abi" => {
                        abi = self.export_argument_string(&argument.value, false);
                    }
                    _ => {}
                }
            }
            let Some(name) = name else {
                self.diagnostics.push(Diagnostic::error(
                    "J0804",
                    "`@export` requires a string `name` argument",
                    annotation.span,
                    "write `@export(name: \"symbol\", abi: \"C\")`",
                ));
                continue;
            };
            let Some(abi) = abi else {
                self.diagnostics.push(Diagnostic::error(
                    "J0804",
                    "`@export` requires an `abi` argument",
                    annotation.span,
                    "write `@export(name: \"symbol\", abi: \"C\")`",
                ));
                continue;
            };
            if abi != "C" {
                self.diagnostics.push(Diagnostic::error(
                    "J0805",
                    format!("unsupported export ABI `{abi}`"),
                    annotation.span,
                    "Jadren 0.1 supports only `abi: \"C\"`",
                ));
            }
            if !is_valid_export_symbol(&name) {
                self.diagnostics.push(Diagnostic::error(
                    "J0806",
                    format!("invalid export symbol `{name}`"),
                    annotation.span,
                    "use an ASCII C identifier starting with a letter or `_`",
                ));
            } else if let Some(first) = exported.get(&name) {
                self.diagnostics.push(
                    Diagnostic::error(
                        "J0807",
                        format!("duplicate export symbol `{name}`"),
                        annotation.span,
                        "each exported symbol must be unique in one module",
                    )
                    .with_secondary(*first, "first export is here"),
                );
            } else {
                exported.insert(name, annotation.span);
            }
        }
    }

    fn export_argument_string(
        &self,
        expression: &Expression,
        require_string: bool,
    ) -> Option<String> {
        match expression {
            Expression::Literal {
                kind: LiteralKind::String,
                span,
            } => Some(
                self.source
                    .slice(*span)
                    .unwrap_or_default()
                    .trim_matches('"')
                    .to_owned(),
            ),
            Expression::Name(name) if !require_string => Some(name.text.clone()),
            _ => None,
        }
    }

    fn is_c_abi_type(&self, ty: TypeId, visiting: &mut DeterministicSet<NominalTypeId>) -> bool {
        match self.types.kind(ty) {
            Some(TypeKind::Integer { .. } | TypeKind::Float(_)) => true,
            // Generic C-layout declarations are validated after their
            // nominal arguments are substituted.  The Buffer<T> copy-safety
            // gate performs that concrete validation before byte-copy codegen.
            Some(TypeKind::GenericParameter(_)) => true,
            Some(TypeKind::Vector { element, lanes }) => {
                matches!(
                    self.types.kind(*element),
                    Some(TypeKind::Float(FloatWidth::Bits32))
                ) && matches!(*lanes, 2 | 3 | 4 | 8)
            }
            Some(TypeKind::Array { element, .. }) => self.is_c_abi_type(*element, visiting),
            Some(TypeKind::Pointer(_)) => true,
            Some(TypeKind::Nominal {
                constructor,
                arguments,
            }) => {
                if !arguments
                    .iter()
                    .all(|argument| self.is_c_abi_type(*argument, visiting))
                {
                    return false;
                }
                if !visiting.insert(*constructor) {
                    return false;
                }
                let result = if let Some(record) = self.records.get(constructor) {
                    record.repr == AbiRepr::C
                        && record
                            .fields
                            .values()
                            .all(|field| self.is_c_abi_type(field.ty, visiting))
                } else if let Some(declaration) = self.enums.get(constructor) {
                    declaration.repr == AbiRepr::C
                        && declaration.variants.values().all(|variant| {
                            variant
                                .fields
                                .iter()
                                .all(|field| self.is_c_abi_type(*field, visiting))
                        })
                } else {
                    false
                };
                visiting.remove(constructor);
                result
            }
            _ => false,
        }
    }

    /// Record declarations may contain a fixed-size owning Buffer descriptor.
    /// The generic Buffer element checker validates the concrete leaf later;
    /// this declaration-level pass only needs to reserve the stable descriptor
    /// layout while keeping FFI signatures copy-safe.
    fn is_c_record_field_type(
        &self,
        ty: TypeId,
        visiting: &mut DeterministicSet<NominalTypeId>,
    ) -> bool {
        match self.types.kind(ty) {
            Some(TypeKind::Buffer(_)) => true,
            Some(TypeKind::Option(inner)) => self.is_c_record_field_type(*inner, visiting),
            Some(TypeKind::Result { ok, error }) => {
                self.is_c_record_field_type(*ok, visiting)
                    && self.is_c_record_field_type(*error, visiting)
            }
            Some(TypeKind::Array { element, .. }) => {
                self.is_c_record_field_type(*element, visiting)
            }
            Some(TypeKind::Nominal {
                constructor,
                arguments,
            }) => {
                if !arguments
                    .iter()
                    .all(|argument| self.is_c_record_field_type(*argument, visiting))
                {
                    return false;
                }
                let Some(record) = self.records.get(constructor) else {
                    // Owning enum carriers need tag-aware metadata and are
                    // intentionally not nested inside records in this ABI.
                    return self.is_c_abi_type(ty, visiting);
                };
                if !visiting.insert(*constructor) {
                    return false;
                }
                let result = record.repr == AbiRepr::C
                    && record
                        .fields
                        .values()
                        .all(|field| self.is_c_record_field_type(field.ty, visiting));
                visiting.remove(constructor);
                result
            }
            _ => self.is_c_abi_type(ty, visiting),
        }
    }

    /// Returns whether an enum payload has a stable C layout.  Owning
    /// `Buffer<T>` descriptors are valid inside a tagged carrier even though
    /// they are intentionally rejected from ordinary `repr(C)` records and
    /// FFI signatures.  The owning carrier move/drop checker applies the
    /// stricter one-owning-variant contract below.
    fn is_c_enum_payload_type(
        &self,
        ty: TypeId,
        visiting: &mut DeterministicSet<NominalTypeId>,
    ) -> bool {
        match self.types.kind(ty) {
            Some(TypeKind::Buffer(inner)) => self.is_c_enum_payload_type(*inner, visiting),
            // OwnedString is the same target-native three-word descriptor as
            // Buffer, and the owning enum field table supplies its destructor
            // marker after the declaration-level layout check.
            Some(TypeKind::OwnedString) => true,
            _ => self.is_c_abi_type(ty, visiting),
        }
    }

    fn declare_local_enum(&mut self, declaration: &EnumDeclaration) {
        let symbol = self
            .declaration_symbol(declaration.name.span)
            .and_then(|symbol| self.resolution.symbol(symbol));
        let constructor = symbol.and_then(Symbol::nominal_type_id).unwrap_or_else(|| {
            NominalTypeId::from_path(&format!(
                "{}#{}",
                self.source.path().display(),
                declaration.name.text
            ))
        });
        let variants = declaration
            .variants
            .iter()
            .map(|variant| {
                (
                    variant.name.text.clone(),
                    EnumVariantInfo {
                        fields: variant
                            .fields
                            .iter()
                            .map(|field| self.lower_type(&field.ty))
                            .collect(),
                        span: variant.name.span,
                    },
                )
            })
            .collect();
        let repr = self.annotation_repr(&declaration.annotations);
        self.enums.insert(
            constructor,
            EnumInfo {
                canonical_path: symbol
                    .and_then(|symbol| symbol.canonical_path.clone())
                    .unwrap_or_else(|| declaration.name.text.clone()),
                generic_parameters: generic_parameter_ids(
                    self.generic_owner_fingerprint(symbol, declaration.name.span.start),
                    declaration.generic_parameters.len(),
                ),
                repr,
                variant_order: declaration
                    .variants
                    .iter()
                    .map(|variant| variant.name.text.clone())
                    .collect(),
                variants,
            },
        );
    }

    fn lower_enum_interface(
        &mut self,
        owner: jadren_resolve::QualifiedSymbolId,
        canonical_path: &str,
        interface: &ModuleEnumInterface,
    ) -> EnumInfo {
        self.register_external_bounds(owner.fingerprint(), &interface.generic_bounds);
        let variants = interface
            .variants
            .iter()
            .map(|variant| {
                (
                    variant.name.clone(),
                    EnumVariantInfo {
                        fields: variant
                            .fields
                            .iter()
                            .map(|field| self.lower_module_type(Some(owner), field))
                            .collect(),
                        span: variant.span,
                    },
                )
            })
            .collect();
        EnumInfo {
            canonical_path: canonical_path.to_owned(),
            generic_parameters: generic_parameter_ids(owner.fingerprint(), interface.generic_count),
            repr: interface.repr,
            variant_order: interface
                .variants
                .iter()
                .map(|variant| variant.name.clone())
                .collect(),
            variants,
        }
    }

    fn generic_owner_fingerprint(
        &self,
        symbol: Option<&Symbol>,
        fallback_start: usize,
    ) -> Fingerprint {
        symbol.and_then(|symbol| symbol.qualified_id).map_or_else(
            || {
                let mut hasher = StableHasher::with_domain("jadren-local-generic-owner-v1");
                hasher.write_u64(self.source.stable_hash());
                hasher.write_u64(symbol.map_or(fallback_start, |symbol| symbol.span.start) as u64);
                hasher.finish()
            },
            jadren_resolve::QualifiedSymbolId::fingerprint,
        )
    }

    fn lower_module_signature(
        &mut self,
        qualified_id: Option<jadren_resolve::QualifiedSymbolId>,
        signature: &ModuleFunctionSignature,
    ) -> TypeId {
        if let Some(owner) = qualified_id {
            self.register_external_bounds(owner.fingerprint(), &signature.generic_bounds);
        }
        let parameters = signature
            .parameters
            .iter()
            .map(|ty| self.lower_module_type(qualified_id, ty))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let result = self.lower_module_type(qualified_id, &signature.result);
        self.types.intern(TypeKind::Function { parameters, result })
    }

    fn lower_module_type(
        &mut self,
        owner: Option<jadren_resolve::QualifiedSymbolId>,
        ty: &ModuleType,
    ) -> TypeId {
        match ty {
            ModuleType::Builtin { name, arguments } => {
                let arguments: Vec<_> = arguments
                    .iter()
                    .map(|argument| self.lower_module_type(owner, argument))
                    .collect();
                self.types
                    .apply_builtin(name, &arguments)
                    .unwrap_or(self.types.core().error)
            }
            ModuleType::Nominal {
                canonical_path,
                arguments,
            } => {
                let arguments = arguments
                    .iter()
                    .map(|argument| self.lower_module_type(owner, argument))
                    .collect::<Vec<_>>()
                    .into_boxed_slice();
                self.types.intern(TypeKind::Nominal {
                    constructor: NominalTypeId::from_symbol_fingerprint(
                        jadren_resolve::QualifiedSymbolId::from_path(
                            Namespace::Type,
                            canonical_path,
                        )
                        .fingerprint(),
                    ),
                    arguments,
                })
            }
            ModuleType::GenericParameter(index) => {
                let owner = owner.map_or_else(
                    || NominalTypeId::from_path("<external-generic>").fingerprint(),
                    jadren_resolve::QualifiedSymbolId::fingerprint,
                );
                self.types
                    .intern(TypeKind::GenericParameter(GenericParameterId {
                        owner,
                        index: *index,
                    }))
            }
            ModuleType::Array { element, length } => {
                let element = self.lower_module_type(owner, element);
                self.types.intern(TypeKind::Array {
                    element,
                    length: *length,
                })
            }
            ModuleType::Capability { capability, inner } => {
                let inner = self.lower_module_type(owner, inner);
                self.types.intern(TypeKind::Capability {
                    capability: *capability,
                    inner,
                })
            }
            ModuleType::Function { parameters, result } => {
                let parameters = parameters
                    .iter()
                    .map(|parameter| self.lower_module_type(owner, parameter))
                    .collect::<Vec<_>>()
                    .into_boxed_slice();
                let result = self.lower_module_type(owner, result);
                self.types.intern(TypeKind::Function { parameters, result })
            }
            ModuleType::Error => self.types.core().error,
        }
    }

    fn check_function(&mut self, function: &Function) {
        let result = self
            .declaration_symbol(function.name.span)
            .and_then(|symbol| self.symbol_types[symbol.index()])
            .and_then(|ty| match self.types.kind(ty) {
                Some(TypeKind::Function { result, .. }) => Some(*result),
                _ => None,
            })
            .unwrap_or(self.types.core().error);
        self.validate_disjoint_annotation(function, result);
        self.infer_block(&function.body, result);
    }

    fn validate_disjoint_annotation(&mut self, function: &Function, _result: TypeId) {
        let Some(annotation) = function
            .annotations
            .iter()
            .find(|annotation| annotation_path_text(annotation) == "disjoint")
        else {
            return;
        };
        if !annotation.arguments.is_empty() {
            self.diagnostics.push(Diagnostic::error(
                "J0810",
                "`@disjoint` does not accept arguments",
                annotation.span,
                "write `@disjoint` on a function with borrowed Slice/Buffer parameters",
            ));
        }
        let parameter_types = self
            .declaration_symbol(function.name.span)
            .and_then(|symbol| self.symbol_types.get(symbol.index()).copied())
            .flatten()
            .and_then(|ty| match self.types.kind(ty) {
                Some(TypeKind::Function { parameters, .. }) => Some(parameters.clone()),
                _ => None,
            });
        let eligible = parameter_types
            .as_deref()
            .map(|parameters| {
                parameters
                    .iter()
                    .filter(|parameter| self.is_disjoint_borrow(**parameter))
                    .count()
            })
            .unwrap_or_default();
        if eligible < 2 {
            self.diagnostics.push(Diagnostic::error(
                "J0811",
                "`@disjoint` requires at least two borrowed Slice/Buffer parameters",
                annotation.span,
                "mark only functions whose borrowed data ranges are pairwise disjoint",
            ));
        }
    }

    fn is_disjoint_borrow(&self, ty: TypeId) -> bool {
        let Some(TypeKind::Capability { capability, inner }) = self.types.kind(ty) else {
            return false;
        };
        if !matches!(capability, Capability::Read | Capability::Write) {
            return false;
        }
        matches!(
            self.types.kind(*inner),
            Some(TypeKind::Slice(_) | TypeKind::Buffer(_))
        )
    }

    fn lower_type(&mut self, ty: &TypeRef) -> TypeId {
        match ty {
            TypeRef::Path {
                path,
                arguments,
                span,
            } => {
                let arguments: Vec<_> = arguments.iter().map(|ty| self.lower_type(ty)).collect();
                let symbol = self.reference_symbol(path.span, Namespace::Type).cloned();
                let Some(symbol) = symbol else {
                    return self.type_error("J0300", "unresolved type", *span);
                };
                if symbol.kind == SymbolKind::GenericParameter {
                    return self.symbol_types[symbol.id.index()].unwrap_or(self.types.core().error);
                }
                if symbol.origin == SymbolOrigin::Builtin {
                    return match self.types.apply_builtin(&symbol.name, &arguments) {
                        Ok(ty) => ty,
                        Err(error) => self.builtin_error(error, *span),
                    };
                }
                let constructor = symbol.nominal_type_id().unwrap_or_else(|| {
                    NominalTypeId::from_path(&format!(
                        "{}#{}",
                        self.source.path().display(),
                        symbol.name
                    ))
                });
                let expected_arity = self
                    .records
                    .get(&constructor)
                    .map(|record| record.generic_parameters.len())
                    .or_else(|| {
                        self.enums
                            .get(&constructor)
                            .map(|declaration| declaration.generic_parameters.len())
                    });
                if let Some(expected) = expected_arity
                    && expected != arguments.len()
                {
                    self.diagnostics.push(Diagnostic::error(
                        "J0315",
                        format!(
                            "type `{}` expects {expected} generic arguments but received {}",
                            symbol.name,
                            arguments.len()
                        ),
                        *span,
                        "incorrect generic type argument count",
                    ));
                }
                let generic_parameters = self
                    .records
                    .get(&constructor)
                    .map(|record| record.generic_parameters.clone())
                    .or_else(|| {
                        self.enums
                            .get(&constructor)
                            .map(|declaration| declaration.generic_parameters.clone())
                    })
                    .unwrap_or_default();
                self.validate_generic_bounds(&generic_parameters, &arguments, *span);
                self.types.intern(TypeKind::Nominal {
                    constructor,
                    arguments: arguments.into_boxed_slice(),
                })
            }
            TypeRef::Array {
                element,
                length,
                span,
            } => {
                let element = self.lower_type(element);
                let text = self
                    .source
                    .slice(*length)
                    .unwrap_or_default()
                    .replace('_', "");
                match text.parse::<u64>() {
                    Ok(length) => self.types.intern(TypeKind::Array { element, length }),
                    Err(_) => self.type_error("J0302", "invalid array length", *span),
                }
            }
            TypeRef::Capability {
                capability, inner, ..
            } => {
                let inner = self.lower_type(inner);
                self.types.intern(TypeKind::Capability {
                    capability: match capability {
                        TypeCapability::Owned => jadren_types::Capability::Owned,
                        TypeCapability::Read => jadren_types::Capability::Read,
                        TypeCapability::Write => jadren_types::Capability::Write,
                    },
                    inner,
                })
            }
            TypeRef::Function {
                parameters,
                return_type,
                ..
            } => {
                let parameters = parameters
                    .iter()
                    .map(|parameter| self.lower_type(parameter))
                    .collect::<Vec<_>>()
                    .into_boxed_slice();
                let result = return_type
                    .as_deref()
                    .map_or(self.types.core().unit, |result| self.lower_type(result));
                self.types.intern(TypeKind::Function { parameters, result })
            }
        }
    }

    fn infer_block(&mut self, block: &Block, return_type: TypeId) -> TypeId {
        let mut block_type = self.types.core().unit;
        for statement in &block.statements {
            block_type = match statement {
                Statement::Binding {
                    name,
                    ty,
                    value,
                    span,
                    ..
                } => {
                    let explicit = ty.as_ref().map(|ty| self.lower_type(ty));
                    let inferred = value
                        .as_ref()
                        // Feed an explicit binding type into the initializer
                        // so generic constructors and region allocations can
                        // validate their element type before inference is
                        // finalized.
                        .map(|value| self.infer_expression(value, explicit.unwrap_or(return_type)));
                    let binding_type = match (explicit, inferred) {
                        (Some(expected), Some(actual)) => {
                            self.unify_or_error(expected, actual, *span)
                        }
                        (Some(ty), None) | (None, Some(ty)) => ty,
                        (None, None) => self.type_error(
                            "J0300",
                            "binding without initializer requires an explicit type",
                            *span,
                        ),
                    };
                    // A generic `buffer_create` is commonly inferred before
                    // this binding annotation is unified. Re-check the
                    // finalized `Result<Buffer<T>, E>` carrier here so an
                    // inference variable cannot bypass the element ABI and
                    // ownership contract.
                    if let Some(element) = self.generic_buffer_element_from_expected(binding_type) {
                        self.require_buffer_element_move_abi(element, *span);
                    }
                    self.assign_declaration(name.span, binding_type);
                    self.types.core().unit
                }
                Statement::Return { value, span } => {
                    let actual = value.as_ref().map_or(self.types.core().unit, |value| {
                        self.infer_expression(value, return_type)
                    });
                    self.unify_or_error(return_type, actual, *span);
                    self.types.core().never
                }
                Statement::Region { name, body, .. } => {
                    let region_type = self.types.intern(TypeKind::Nominal {
                        constructor: NominalTypeId::from_path("core.Region"),
                        arguments: Box::new([]),
                    });
                    self.assign_declaration(name.span, region_type);
                    self.infer_block(body, return_type);
                    self.types.core().unit
                }
                Statement::While {
                    condition,
                    body,
                    span,
                } => {
                    let condition_type = self.infer_expression(condition, return_type);
                    self.unify_or_error(self.types.core().bool_, condition_type, *span);
                    self.loop_depth += 1;
                    self.infer_block(body, return_type);
                    self.loop_depth -= 1;
                    self.types.core().unit
                }
                Statement::For {
                    binding,
                    iterable,
                    body,
                    span,
                } => {
                    let previous_allow_index_iterable =
                        std::mem::replace(&mut self.allow_index_iterable, true);
                    let iterable_type = self.infer_expression(iterable, return_type);
                    self.allow_index_iterable = previous_allow_index_iterable;
                    let element_type = if Self::index_iterable_base(iterable).is_some() {
                        self.types.core().uint_size
                    } else {
                        let resolved = self.unification.resolve_shallow(&self.types, iterable_type);
                        match self.iterable_element_type(resolved) {
                            Some(element) => element,
                            None => self.type_error(
                                "J0320",
                                "`for` requires an array, Buffer, Slice, or Buffer/Slice `.indices`",
                                *span,
                            ),
                        }
                    };
                    self.assign_declaration(binding.span, element_type);
                    self.loop_depth += 1;
                    self.infer_block(body, return_type);
                    self.loop_depth -= 1;
                    self.types.core().unit
                }
                Statement::Break { span } => {
                    if self.loop_depth == 0 {
                        self.diagnostics.push(Diagnostic::error(
                            "J0318",
                            "`break` is only valid inside a loop",
                            *span,
                            "move this statement into a `while` loop",
                        ));
                    }
                    self.types.core().unit
                }
                Statement::Continue { span } => {
                    if self.loop_depth == 0 {
                        self.diagnostics.push(Diagnostic::error(
                            "J0319",
                            "`continue` is only valid inside a loop",
                            *span,
                            "move this statement into a `while` loop",
                        ));
                    }
                    self.types.core().unit
                }
                Statement::Expression {
                    expression,
                    terminated,
                } => {
                    let ty = self.infer_expression(expression, return_type);
                    if *terminated {
                        self.types.core().unit
                    } else {
                        ty
                    }
                }
            };
        }
        block_type
    }

    fn infer_expression(&mut self, expression: &Expression, return_type: TypeId) -> TypeId {
        let ty = match expression {
            Expression::Name(name) => self.infer_name(name, return_type),
            Expression::Literal { kind, span } => self.literal_type(*kind, *span),
            Expression::Unary {
                operator,
                operand,
                span,
            } => {
                let operand = self.infer_expression(operand, return_type);
                self.infer_unary(*operator, operand, *span)
            }
            Expression::Binary {
                left,
                operator,
                right,
                span,
            } => {
                let left = self.infer_expression(left, return_type);
                let right = self.infer_expression(right, return_type);
                self.infer_binary(*operator, left, right, *span)
            }
            Expression::Call {
                callee,
                arguments,
                span,
            } => self.infer_call(callee, arguments, *span, return_type),
            Expression::Field {
                base, field, span, ..
            } => self.infer_field(base, field, *span, return_type),
            Expression::Index { base, index, span } => {
                let base = self.infer_expression(base, return_type);
                let index = self.infer_expression(index, return_type);
                if self.unification.resolve_shallow(&self.types, index)
                    != self.types.core().uint_size
                {
                    let int32 = self.types.core().int32;
                    self.unify_or_error(int32, index, *span);
                }
                let mut resolved = self.unification.resolve_shallow(&self.types, base);
                if let Some(TypeKind::Capability { inner, .. }) = self.types.kind(resolved) {
                    resolved = *inner;
                }
                match self.types.kind(resolved) {
                    Some(TypeKind::Array { element, .. })
                    | Some(TypeKind::Buffer(element))
                    | Some(TypeKind::Slice(element)) => *element,
                    _ => self.unification.fresh(&mut self.types),
                }
            }
            Expression::Try { operand, span } => self.infer_try(operand, *span, return_type),
            Expression::Cast {
                expression,
                target,
                span,
            } => {
                let source = self.infer_expression(expression, return_type);
                let target = self.lower_type(target);
                self.validate_numeric_cast(source, target, *span)
            }
            Expression::Array { elements, .. } => {
                let element = if let Some(element) = elements.first() {
                    self.infer_expression(element, return_type)
                } else {
                    self.unification.fresh(&mut self.types)
                };
                for value in elements.iter().skip(1) {
                    let actual = self.infer_expression(value, return_type);
                    self.unify_or_error(element, actual, value.span());
                }
                self.types.intern(TypeKind::Array {
                    element,
                    length: elements.len() as u64,
                })
            }
            Expression::StructLiteral { ty, fields, span } => {
                self.infer_struct_literal(ty, fields, *span, return_type)
            }
            Expression::Group { expression, .. } => self.infer_expression(expression, return_type),
            Expression::Block(block) => self.infer_block(block, return_type),
            Expression::If {
                condition,
                then_block,
                else_branch,
                span,
            } => {
                let condition = self.infer_expression(condition, return_type);
                let bool_ = self.types.core().bool_;
                self.unify_or_error(bool_, condition, *span);
                let then_ty = self.infer_block(then_block, return_type);
                let else_ty = else_branch
                    .as_ref()
                    .map_or(self.types.core().unit, |branch| {
                        self.infer_expression(branch, return_type)
                    });
                self.unify_or_error(then_ty, else_ty, *span)
            }
            Expression::Match { value, arms, .. } => {
                let value_type = self.infer_expression(value, return_type);
                self.infer_match_arms(value_type, arms, return_type)
            }
            Expression::Error(_) => self.types.core().error,
        };
        self.expressions.push(ExpressionType {
            id: TypedExpressionId::default(),
            kind: ExpressionKind::of(expression),
            span: expression.span(),
            ty,
        });
        ty
    }

    fn iterable_element_type(&mut self, ty: TypeId) -> Option<TypeId> {
        let mut resolved = ty;
        if let Some(TypeKind::Capability { inner, .. }) = self.types.kind(resolved) {
            resolved = *inner;
        }
        match self.types.kind(resolved) {
            Some(TypeKind::Array { element, .. })
            | Some(TypeKind::Buffer(element))
            | Some(TypeKind::Slice(element)) => Some(*element),
            _ => None,
        }
    }

    fn index_iterable_base(expression: &Expression) -> Option<&Expression> {
        match expression {
            Expression::Field { base, field, .. } if field.text == "indices" => Some(base),
            _ => None,
        }
    }

    fn validate_numeric_cast(&mut self, source: TypeId, target: TypeId, span: Span) -> TypeId {
        let source = self.unification.resolve_shallow(&self.types, source);
        let target = self.unification.resolve_shallow(&self.types, target);
        let source_numeric = matches!(
            self.types.kind(source),
            Some(TypeKind::Integer { .. } | TypeKind::Float(_))
        );
        let target_numeric = matches!(
            self.types.kind(target),
            Some(TypeKind::Integer { .. } | TypeKind::Float(_))
        );
        if matches!(self.types.kind(source), Some(TypeKind::Error))
            || matches!(self.types.kind(target), Some(TypeKind::Error))
        {
            return self.types.core().error;
        }
        if source_numeric && target_numeric {
            target
        } else {
            self.type_error(
                "J0321",
                "`as` casts require integer or floating-point scalar types",
                span,
            )
        }
    }

    fn infer_match_arms(
        &mut self,
        value_type: TypeId,
        arms: &[MatchArm],
        return_type: TypeId,
    ) -> TypeId {
        let result = self.unification.fresh(&mut self.types);
        let resolved = self.unification.resolve_shallow(&self.types, value_type);
        let enum_info = self.enum_info_for_type(resolved);
        let mut covered = DeterministicSet::new();
        let mut covers_all = false;
        for arm in arms {
            let coverage = self.infer_pattern(value_type, &arm.pattern, enum_info.as_ref());
            if let Some(guard) = &arm.guard {
                let guard_ty = self.infer_expression(guard, return_type);
                self.unify_or_error(self.types.core().bool_, guard_ty, guard.span());
            } else {
                match coverage {
                    PatternCoverage::All => covers_all = true,
                    PatternCoverage::Variant(name) => {
                        covered.insert(name);
                    }
                    PatternCoverage::None => {}
                }
            }
            let arm_ty = self.infer_expression(&arm.value, return_type);
            self.unify_or_error(result, arm_ty, arm.value.span());
        }
        if let Some(info) = enum_info
            && !covers_all
        {
            let missing: Vec<_> = info
                .variants
                .keys()
                .filter(|variant| !covered.contains(*variant))
                .cloned()
                .collect();
            if !missing.is_empty() {
                self.diagnostics.push(Diagnostic::error(
                    "J0311",
                    format!("non-exhaustive match; missing {}", missing.join(", ")),
                    arms.first()
                        .map_or(Span::empty(self.source.id(), 0), |arm| arm.span),
                    "add the missing variants or an unguarded wildcard arm",
                ));
            }
        }
        result
    }

    fn infer_pattern(
        &mut self,
        expected: TypeId,
        pattern: &Pattern,
        enum_info: Option<&EnumInfo>,
    ) -> PatternCoverage {
        match pattern {
            Pattern::Wildcard(_) => PatternCoverage::All,
            Pattern::Error(_) => PatternCoverage::None,
            Pattern::Literal { kind, span } => {
                let actual = self.literal_type(*kind, *span);
                self.unify_or_error(expected, actual, *span);
                PatternCoverage::None
            }
            Pattern::Path(path) if is_binding_pattern(path) => {
                self.assign_declaration(path.segments[0].span, expected);
                PatternCoverage::All
            }
            Pattern::Path(path) => self.infer_variant_pattern(
                path.segments.last().map(|name| name.text.as_str()),
                &[],
                path.span,
                enum_info,
            ),
            Pattern::Constructor {
                path,
                arguments,
                span,
            } => self.infer_variant_pattern(
                path.segments.last().map(|name| name.text.as_str()),
                arguments,
                *span,
                enum_info,
            ),
        }
    }

    fn infer_variant_pattern(
        &mut self,
        name: Option<&str>,
        arguments: &[Pattern],
        span: Span,
        enum_info: Option<&EnumInfo>,
    ) -> PatternCoverage {
        let Some(name) = name else {
            return PatternCoverage::None;
        };
        let Some(info) = enum_info else {
            for argument in arguments {
                let ty = self.unification.fresh(&mut self.types);
                self.infer_pattern(ty, argument, None);
            }
            return PatternCoverage::None;
        };
        let Some(variant) = info.variants.get(name).cloned() else {
            self.diagnostics.push(Diagnostic::error(
                "J0309",
                format!("unknown enum variant `{name}`"),
                span,
                "variant is not declared by the matched enum",
            ));
            return PatternCoverage::None;
        };
        if variant.fields.len() != arguments.len() {
            self.diagnostics.push(
                Diagnostic::error(
                    "J0310",
                    format!(
                        "variant `{name}` expects {} payload patterns but received {}",
                        variant.fields.len(),
                        arguments.len()
                    ),
                    span,
                    "incorrect enum pattern payload count",
                )
                .with_secondary(variant.span, "variant is declared here"),
            );
        }
        for (index, argument) in arguments.iter().enumerate() {
            let expected = variant
                .fields
                .get(index)
                .copied()
                .unwrap_or_else(|| self.unification.fresh(&mut self.types));
            self.infer_pattern(expected, argument, None);
        }
        PatternCoverage::Variant(name.to_owned())
    }

    fn infer_name(&mut self, name: &Name, return_type: TypeId) -> TypeId {
        if let Some(ty) = self
            .reference_symbol(name.span, Namespace::Value)
            .and_then(|symbol| self.symbol_types[symbol.id.index()])
        {
            return ty;
        }
        if let Some(ty) = self.try_infer_enum_constructor(
            &Expression::Name(name.clone()),
            &[],
            name.span,
            return_type,
        ) {
            return ty;
        }
        self.try_infer_core_constructor(
            &Expression::Name(name.clone()),
            &[],
            name.span,
            return_type,
        )
        .unwrap_or_else(|| self.unification.fresh(&mut self.types))
    }

    fn try_infer_enum_constructor(
        &mut self,
        callee: &Expression,
        arguments: &[Expression],
        span: Span,
        return_type: TypeId,
    ) -> Option<TypeId> {
        let (qualifier, variant_name) = enum_constructor_selector(callee)?;
        let candidates: Vec<_> = self
            .enums
            .iter()
            .filter(|(_, info)| {
                qualifier.as_ref().is_none_or(|qualifier| {
                    info.canonical_path == *qualifier
                        || info
                            .canonical_path
                            .strip_suffix(qualifier)
                            .is_some_and(|prefix| prefix.ends_with('.'))
                }) && info.variants.contains_key(&variant_name)
            })
            .map(|(constructor, info)| {
                (
                    *constructor,
                    info.generic_parameters.clone(),
                    info.variants[&variant_name].clone(),
                    info.canonical_path.clone(),
                )
            })
            .collect();
        if candidates.is_empty() {
            return None;
        }
        if candidates.len() > 1 {
            self.diagnostics.push(Diagnostic::error(
                "J0312",
                format!("ambiguous enum constructor `{variant_name}`"),
                span,
                "qualify the constructor with its enum type",
            ));
            return Some(self.types.core().error);
        }
        let (constructor, generic_parameters, variant, _) = &candidates[0];
        let (substitution, type_arguments) = self.fresh_substitution(generic_parameters);
        let variant_fields: Vec<_> = variant
            .fields
            .iter()
            .map(|field| {
                substitution
                    .apply(&mut self.types, *field)
                    .unwrap_or(self.types.core().error)
            })
            .collect();
        if variant.fields.len() != arguments.len() {
            self.diagnostics.push(
                Diagnostic::error(
                    "J0310",
                    format!(
                        "variant `{variant_name}` expects {} arguments but received {}",
                        variant.fields.len(),
                        arguments.len()
                    ),
                    span,
                    "incorrect enum constructor argument count",
                )
                .with_secondary(variant.span, "variant is declared here"),
            );
        }
        let actuals: Vec<_> = arguments
            .iter()
            .map(|argument| self.infer_expression(argument, return_type))
            .collect();
        for ((expected, argument), actual) in variant_fields.iter().zip(arguments).zip(actuals) {
            self.unify_or_error(*expected, actual, argument.span());
        }
        let type_arguments = type_arguments
            .into_iter()
            .map(|argument| self.unification.resolve_shallow(&self.types, argument))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        self.validate_generic_bounds(generic_parameters, &type_arguments, span);
        Some(self.types.intern(TypeKind::Nominal {
            constructor: *constructor,
            arguments: type_arguments,
        }))
    }

    fn try_infer_core_constructor(
        &mut self,
        callee: &Expression,
        arguments: &[Expression],
        span: Span,
        return_type: TypeId,
    ) -> Option<TypeId> {
        let (qualifier, name) = enum_constructor_selector(callee)?;
        let family = match name.as_str() {
            "Some" | "None"
                if qualifier
                    .as_deref()
                    .is_none_or(|path| path == "Option" || path == "core.Option") =>
            {
                "Option"
            }
            "Ok" | "Error"
                if qualifier
                    .as_deref()
                    .is_none_or(|path| path == "Result" || path == "core.Result") =>
            {
                "Result"
            }
            _ => return None,
        };
        let expected = usize::from(name != "None");
        if arguments.len() != expected {
            self.diagnostics.push(Diagnostic::error(
                "J0310",
                format!(
                    "core variant `{name}` expects {expected} arguments but received {}",
                    arguments.len()
                ),
                span,
                "incorrect core constructor argument count",
            ));
        }
        let actuals: Vec<_> = arguments
            .iter()
            .map(|argument| self.infer_expression(argument, return_type))
            .collect();
        if family == "Option" {
            let inner = actuals
                .first()
                .copied()
                .unwrap_or_else(|| self.unification.fresh(&mut self.types));
            Some(self.types.intern(TypeKind::Option(inner)))
        } else {
            let payload = actuals
                .first()
                .copied()
                .unwrap_or_else(|| self.unification.fresh(&mut self.types));
            let other = self.unification.fresh(&mut self.types);
            let (ok, error) = if name == "Ok" {
                (payload, other)
            } else {
                (other, payload)
            };
            Some(self.types.intern(TypeKind::Result { ok, error }))
        }
    }

    fn infer_try(&mut self, operand: &Expression, span: Span, return_type: TypeId) -> TypeId {
        let operand_type = self.infer_expression(operand, return_type);
        let operand_type = self.unification.resolve_shallow(&self.types, operand_type);
        let return_type = self.unification.resolve_shallow(&self.types, return_type);
        match self.types.kind(operand_type).cloned() {
            Some(TypeKind::Option(inner)) => {
                if matches!(self.types.kind(return_type), Some(TypeKind::Option(_))) {
                    self.propagation_sites.push(PropagationSite {
                        span,
                        kind: PropagationKind::OptionNone,
                        success_type: inner,
                        residual_type: self.types.core().unit,
                        return_type,
                    });
                    inner
                } else {
                    self.type_error(
                        "J0313",
                        "`?` on Option requires an Option return type",
                        span,
                    )
                }
            }
            Some(TypeKind::Result { ok, error }) => {
                if let Some(TypeKind::Result {
                    error: return_error,
                    ..
                }) = self.types.kind(return_type).cloned()
                {
                    self.unify_or_error(return_error, error, span);
                    self.propagation_sites.push(PropagationSite {
                        span,
                        kind: PropagationKind::ResultError,
                        success_type: ok,
                        residual_type: error,
                        return_type,
                    });
                    ok
                } else {
                    self.type_error("J0313", "`?` on Result requires a Result return type", span)
                }
            }
            Some(TypeKind::InferenceVariable(_)) => match self.types.kind(return_type).cloned() {
                Some(TypeKind::Option(_)) => {
                    let inner = self.unification.fresh(&mut self.types);
                    let option = self.types.intern(TypeKind::Option(inner));
                    self.unify_or_error(operand_type, option, span);
                    self.propagation_sites.push(PropagationSite {
                        span,
                        kind: PropagationKind::OptionNone,
                        success_type: inner,
                        residual_type: self.types.core().unit,
                        return_type,
                    });
                    inner
                }
                Some(TypeKind::Result { error, .. }) => {
                    let ok = self.unification.fresh(&mut self.types);
                    let result = self.types.intern(TypeKind::Result { ok, error });
                    self.unify_or_error(operand_type, result, span);
                    self.propagation_sites.push(PropagationSite {
                        span,
                        kind: PropagationKind::ResultError,
                        success_type: ok,
                        residual_type: error,
                        return_type,
                    });
                    ok
                }
                _ => self.type_error(
                    "J0313",
                    "`?` requires Option or Result propagation context",
                    span,
                ),
            },
            Some(TypeKind::Error) => self.types.core().error,
            _ => self.type_error("J0313", "`?` operand must be Option or Result", span),
        }
    }

    fn enum_info_for_type(&mut self, ty: TypeId) -> Option<EnumInfo> {
        let span = Span::empty(self.source.id(), 0);
        match self.types.kind(ty).cloned() {
            Some(TypeKind::Nominal {
                constructor,
                arguments,
            }) => {
                let mut info = self.enums.get(&constructor).cloned()?;
                let substitution =
                    substitution_from_arguments(&info.generic_parameters, &arguments);
                for variant in info.variants.values_mut() {
                    for field in &mut variant.fields {
                        *field = substitution
                            .apply(&mut self.types, *field)
                            .unwrap_or(self.types.core().error);
                    }
                }
                info.generic_parameters.clear();
                Some(info)
            }
            Some(TypeKind::Option(inner)) => Some(EnumInfo {
                canonical_path: "core.Option".to_owned(),
                generic_parameters: Vec::new(),
                repr: AbiRepr::Jadren,
                variant_order: vec!["None".to_owned(), "Some".to_owned()],
                variants: [
                    (
                        "None".to_owned(),
                        EnumVariantInfo {
                            fields: Vec::new(),
                            span,
                        },
                    ),
                    (
                        "Some".to_owned(),
                        EnumVariantInfo {
                            fields: vec![inner],
                            span,
                        },
                    ),
                ]
                .into_iter()
                .collect(),
            }),
            Some(TypeKind::Result { ok, error }) => Some(EnumInfo {
                canonical_path: "core.Result".to_owned(),
                generic_parameters: Vec::new(),
                repr: AbiRepr::Jadren,
                variant_order: vec!["Error".to_owned(), "Ok".to_owned()],
                variants: [
                    (
                        "Error".to_owned(),
                        EnumVariantInfo {
                            fields: vec![error],
                            span,
                        },
                    ),
                    (
                        "Ok".to_owned(),
                        EnumVariantInfo {
                            fields: vec![ok],
                            span,
                        },
                    ),
                ]
                .into_iter()
                .collect(),
            }),
            _ => None,
        }
    }

    fn infer_field(
        &mut self,
        base: &Expression,
        field: &Name,
        span: Span,
        return_type: TypeId,
    ) -> TypeId {
        if self.allow_index_iterable && field.text == "indices" {
            let base_type = self.infer_expression(base, return_type);
            let mut resolved = self.unification.resolve_shallow(&self.types, base_type);
            if let Some(TypeKind::Capability { inner, .. }) = self.types.kind(resolved) {
                resolved = *inner;
            }
            if matches!(
                self.types.kind(resolved),
                Some(TypeKind::Buffer(_) | TypeKind::Slice(_))
            ) {
                return self.types.core().uint_size;
            }
            return self.type_error(
                "J0320",
                "`.indices` is available only for Buffer or Slice iteration",
                span,
            );
        }
        if let Some(symbol) = self.reference_symbol(span, Namespace::Value)
            && let Some(ty) = self.symbol_types[symbol.id.index()]
        {
            return ty;
        }
        if let Some(ty) = self.try_infer_enum_constructor(
            &Expression::Field {
                base: Box::new(base.clone()),
                field: field.clone(),
                span,
            },
            &[],
            span,
            return_type,
        ) {
            return ty;
        }
        if let Some(ty) = self.try_infer_core_constructor(
            &Expression::Field {
                base: Box::new(base.clone()),
                field: field.clone(),
                span,
            },
            &[],
            span,
            return_type,
        ) {
            return ty;
        }
        let base_type = self.infer_expression(base, return_type);
        let mut resolved = self.unification.resolve_shallow(&self.types, base_type);
        if let Some(TypeKind::Capability { inner, .. }) = self.types.kind(resolved) {
            resolved = *inner;
        }
        let Some(TypeKind::Nominal {
            constructor,
            arguments,
        }) = self.types.kind(resolved).cloned()
        else {
            if matches!(
                self.types.kind(resolved),
                Some(TypeKind::InferenceVariable(_))
            ) {
                return self.unification.fresh(&mut self.types);
            }
            return self.type_error("J0306", "field access requires a record or component", span);
        };
        let Some(record) = self.records.get(&constructor).cloned() else {
            return self.unification.fresh(&mut self.types);
        };
        let Some(field_info) = record.fields.get(&field.text).cloned() else {
            return self.type_error("J0306", "unknown record field", field.span);
        };
        self.check_field_visibility(&record.module_name, &field.text, &field_info, field.span);
        substitution_from_arguments(&record.generic_parameters, &arguments)
            .apply(&mut self.types, field_info.ty)
            .unwrap_or(self.types.core().error)
    }

    fn infer_struct_literal(
        &mut self,
        ty: &Expression,
        fields: &[StructFieldValue],
        span: Span,
        return_type: TypeId,
    ) -> TypeId {
        let constructor = self
            .reference_symbol(ty.span(), Namespace::Type)
            .and_then(Symbol::nominal_type_id);
        let Some(constructor) = constructor else {
            for field in fields {
                self.infer_expression(&field.value, return_type);
            }
            return self.type_error("J0300", "unresolved record type", span);
        };
        let Some(record) = self.records.get(&constructor).cloned() else {
            for field in fields {
                self.infer_expression(&field.value, return_type);
            }
            return self.types.intern(TypeKind::Nominal {
                constructor,
                arguments: Box::new([]),
            });
        };
        let (substitution, type_arguments) = self.fresh_substitution(&record.generic_parameters);

        let mut seen = DeterministicSet::new();
        for field in fields {
            let actual = self.infer_expression(&field.value, return_type);
            if !seen.insert(field.name.text.clone()) {
                self.diagnostics.push(Diagnostic::error(
                    "J0307",
                    format!("duplicate field initializer `{}`", field.name.text),
                    field.name.span,
                    "field is initialized more than once",
                ));
                continue;
            }
            if let Some(expected) = record.fields.get(&field.name.text) {
                let expected_ty = substitution
                    .apply(&mut self.types, expected.ty)
                    .unwrap_or(self.types.core().error);
                self.unify_or_error(expected_ty, actual, field.value.span());
                self.check_field_visibility(
                    &record.module_name,
                    &field.name.text,
                    expected,
                    field.name.span,
                );
            } else {
                self.diagnostics.push(Diagnostic::error(
                    "J0306",
                    format!("unknown record field `{}`", field.name.text),
                    field.name.span,
                    "this field is not declared by the constructed type",
                ));
            }
        }
        for (name, field) in &record.fields {
            if !seen.contains(name) {
                self.diagnostics.push(
                    Diagnostic::error(
                        "J0308",
                        format!("missing field initializer `{name}`"),
                        span,
                        "record construction must initialize every field",
                    )
                    .with_secondary(field.span, "field is declared here"),
                );
            }
        }
        let arguments = type_arguments
            .into_iter()
            .map(|argument| self.unification.resolve_shallow(&self.types, argument))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        self.validate_generic_bounds(&record.generic_parameters, &arguments, span);
        self.types.intern(TypeKind::Nominal {
            constructor,
            arguments,
        })
    }

    fn check_field_visibility(
        &mut self,
        defining_module: &str,
        name: &str,
        field: &RecordFieldInfo,
        use_span: Span,
    ) {
        if field.visibility == DeclaredVisibility::Private
            && self.resolution.module_name.as_deref() != Some(defining_module)
        {
            self.diagnostics.push(
                Diagnostic::error(
                    "J0205",
                    format!("cannot access private field `{name}`"),
                    use_span,
                    "private fields are visible only inside their defining module",
                )
                .with_secondary(field.span, "private field is declared here"),
            );
        }
    }

    fn infer_call(
        &mut self,
        callee: &Expression,
        arguments: &[Expression],
        span: Span,
        return_type: TypeId,
    ) -> TypeId {
        if let Some(ty) = self.try_infer_region_allocation(callee, arguments, span, return_type) {
            return ty;
        }
        if let Some(ty) = self.try_infer_enum_constructor(callee, arguments, span, return_type) {
            return ty;
        }
        if let Some(ty) = self.try_infer_core_constructor(callee, arguments, span, return_type) {
            return ty;
        }
        let callee_type = self.infer_expression(callee, return_type);
        let argument_types: Vec<_> = arguments
            .iter()
            .map(|argument| self.infer_expression(argument, return_type))
            .collect();

        let builtin_name = if let Expression::Name(name) = callee {
            self.reference_symbol(name.span, Namespace::Value)
                .filter(|symbol| symbol.origin == SymbolOrigin::Builtin)
                .map(|symbol| symbol.name.clone())
        } else {
            None
        };
        if let Some(name) = builtin_name {
            return self.infer_builtin_call(&name, arguments, &argument_types, span, return_type);
        }

        let declaration = self
            .reference_symbol(callee.span(), Namespace::Value)
            .map(|symbol| symbol.id);
        let (instantiated, generic_arguments) = self.instantiate_generic_type(callee_type);
        let resolved = self.unification.resolve_shallow(&self.types, instantiated);
        match self.types.kind(resolved).cloned() {
            Some(TypeKind::Function { parameters, result }) => {
                if parameters.len() != argument_types.len() {
                    self.diagnostics.push(Diagnostic::error(
                        "J0304",
                        format!(
                            "function expects {} arguments but received {}",
                            parameters.len(),
                            argument_types.len()
                        ),
                        span,
                        "incorrect function argument count",
                    ));
                }
                for ((expected, actual), argument) in
                    parameters.iter().zip(&argument_types).zip(arguments)
                {
                    self.unify_or_error(*expected, *actual, argument.span());
                }
                if parameters.len() == argument_types.len()
                    && let Some(declaration) = declaration
                {
                    self.record_monomorphization(declaration, &generic_arguments, span);
                }
                self.unification.resolve_shallow(&self.types, result)
            }
            Some(TypeKind::InferenceVariable(_) | TypeKind::Error) | None => {
                self.unification.fresh(&mut self.types)
            }
            Some(_) => self.type_error("J0305", "expression is not callable", span),
        }
    }

    fn try_infer_region_allocation(
        &mut self,
        callee: &Expression,
        arguments: &[Expression],
        span: Span,
        return_type: TypeId,
    ) -> Option<TypeId> {
        let Expression::Field { base, field, .. } = callee else {
            return None;
        };
        if field.text != "allocate" {
            return None;
        }
        let Expression::Name(region_name) = base.as_ref() else {
            return None;
        };
        let region = self
            .reference_symbol(region_name.span, Namespace::Value)
            .filter(|symbol| symbol.kind == SymbolKind::Region)?
            .id;
        if arguments.len() != 1 {
            self.diagnostics.push(Diagnostic::error(
                "J0508",
                format!(
                    "region allocation expects one element count but received {}",
                    arguments.len()
                ),
                span,
                "use `region.allocate(count)` with an explicit Buffer result type",
            ));
        }
        for argument in arguments {
            let count = self.infer_expression(argument, return_type);
            self.require_integer(count, argument.span());
        }
        let expected = self.unification.resolve_shallow(&self.types, return_type);
        let element = if let Some(TypeKind::Buffer(element)) = self.types.kind(expected) {
            // Region cleanup bulk-frees storage and has no nested destructor
            // walk; only copy-safe elements are valid in this bounded API.
            let element = *element;
            self.require_buffer_element_abi(element, span);
            element
        } else {
            // Without an explicit result annotation the element remains an
            // inference variable and the binding check will validate it once
            // the surrounding type is known.
            self.unification.fresh(&mut self.types)
        };
        let result_type = self.types.intern(TypeKind::Buffer(element));
        self.region_allocations.push(RegionAllocationSite {
            span,
            region,
            result_type,
        });
        Some(result_type)
    }

    fn instantiate_generic_type(
        &mut self,
        ty: TypeId,
    ) -> (TypeId, Vec<(GenericParameterId, TypeId)>) {
        let mut parameters = DeterministicSet::new();
        collect_generic_parameters(&self.types, ty, &mut parameters);
        if parameters.is_empty() {
            return (ty, Vec::new());
        }
        let mut substitution = Substitution::new();
        let arguments: Vec<_> = parameters
            .into_iter()
            .map(|parameter| {
                let argument = self.unification.fresh(&mut self.types);
                substitution.insert(parameter, argument);
                (parameter, argument)
            })
            .collect();
        let instantiated = substitution.apply(&mut self.types, ty).unwrap_or(ty);
        (instantiated, arguments)
    }

    fn fresh_substitution(
        &mut self,
        parameters: &[GenericParameterId],
    ) -> (Substitution, Vec<TypeId>) {
        let arguments: Vec<_> = parameters
            .iter()
            .map(|_| self.unification.fresh(&mut self.types))
            .collect();
        (
            substitution_from_arguments(parameters, &arguments),
            arguments,
        )
    }

    fn record_monomorphization(
        &mut self,
        declaration: SymbolId,
        arguments: &[(GenericParameterId, TypeId)],
        span: Span,
    ) {
        if arguments.is_empty() {
            return;
        }
        let mut concrete = Vec::with_capacity(arguments.len());
        let mut fingerprints = Vec::with_capacity(arguments.len());
        let mut deferred = false;
        for (parameter, argument) in arguments {
            let Ok(argument) = self.unification.resolve_deep(&mut self.types, *argument) else {
                self.type_error("J0314", "cannot infer generic type argument", span);
                return;
            };
            if !self.validate_generic_bound(*parameter, argument, span) {
                return;
            }
            let mut remaining_parameters = DeterministicSet::new();
            collect_generic_parameters(&self.types, argument, &mut remaining_parameters);
            if !remaining_parameters.is_empty() {
                deferred = true;
                continue;
            }
            let Ok(fingerprint) = self.types.stable_fingerprint(argument) else {
                self.type_error("J0314", "cannot infer generic type argument", span);
                return;
            };
            concrete.push(argument);
            fingerprints.push(fingerprint);
        }
        if deferred {
            return;
        }
        let symbol = &self.resolution.symbols[declaration.index()];
        let declaration_fingerprint = symbol.qualified_id.map_or_else(
            || {
                let mut hasher = StableHasher::with_domain("jadren-local-generic-declaration-v1");
                hasher.write_u64(self.source.stable_hash());
                hasher.write_u64(symbol.span.start as u64);
                hasher.finish()
            },
            jadren_resolve::QualifiedSymbolId::fingerprint,
        );
        let key = MonomorphizationKey::new(declaration_fingerprint, &fingerprints);
        self.monomorphizations
            .entry(key)
            .or_insert(MonomorphizationInstance {
                declaration,
                key,
                arguments: concrete,
                span,
            });
    }

    fn validate_generic_bounds(
        &mut self,
        parameters: &[GenericParameterId],
        arguments: &[TypeId],
        span: Span,
    ) -> bool {
        parameters
            .iter()
            .zip(arguments)
            .all(|(parameter, argument)| self.validate_generic_bound(*parameter, *argument, span))
    }

    fn validate_generic_bound(
        &mut self,
        parameter: GenericParameterId,
        argument: TypeId,
        span: Span,
    ) -> bool {
        let bounds = self
            .generic_bounds
            .get(&parameter)
            .cloned()
            .unwrap_or_default();
        let mut valid = true;
        for bound in bounds {
            if !self.type_satisfies_bound(argument, bound) {
                self.diagnostics.push(Diagnostic::error(
                    "J0316",
                    format!("generic type argument does not satisfy `{}`", bound.name()),
                    span,
                    "trait bound is not satisfied by the inferred type",
                ));
                valid = false;
            }
        }
        valid
    }

    fn type_satisfies_bound(&self, ty: TypeId, bound: BuiltinTrait) -> bool {
        let ty = self.unification.resolve_shallow(&self.types, ty);
        match self.types.kind(ty) {
            Some(TypeKind::Error) => true,
            Some(TypeKind::GenericParameter(parameter)) => self
                .generic_bounds
                .get(parameter)
                .is_some_and(|declared| declared.iter().any(|declared| declared.implies(bound))),
            Some(TypeKind::Integer { .. }) => matches!(
                bound,
                BuiltinTrait::Addable
                    | BuiltinTrait::Numeric
                    | BuiltinTrait::Integer
                    | BuiltinTrait::Equatable
                    | BuiltinTrait::Ordered
            ),
            Some(TypeKind::Float(_)) => matches!(
                bound,
                BuiltinTrait::Addable
                    | BuiltinTrait::Numeric
                    | BuiltinTrait::Floating
                    | BuiltinTrait::Equatable
                    | BuiltinTrait::Ordered
            ),
            Some(TypeKind::Vector { element, .. }) => {
                self.type_satisfies_bound(*element, bound)
                    && matches!(
                        bound,
                        BuiltinTrait::Addable | BuiltinTrait::Numeric | BuiltinTrait::Floating
                    )
            }
            Some(TypeKind::String) => matches!(
                bound,
                BuiltinTrait::Addable | BuiltinTrait::Equatable | BuiltinTrait::Ordered
            ),
            Some(TypeKind::Bool | TypeKind::Char | TypeKind::Unit) => {
                bound == BuiltinTrait::Equatable
                    || (matches!(self.types.kind(ty), Some(TypeKind::Char))
                        && bound == BuiltinTrait::Ordered)
            }
            Some(TypeKind::Capability { inner, .. }) => self.type_satisfies_bound(*inner, bound),
            _ => false,
        }
    }

    fn infer_builtin_call(
        &mut self,
        name: &str,
        arguments: &[Expression],
        argument_types: &[TypeId],
        span: Span,
        return_type: TypeId,
    ) -> TypeId {
        let expected = match name {
            "print" | "stdin_read" | "stdout_write" | "stderr_write" => 1,
            "time_now_unix_seconds" => 0,
            "time_now_monotonic_ms" => 0,
            "process_arg_count" => 0,
            "process_arg_read" => 2,
            "time_utc_parts" => 2,
            "time_utc_offset_parts" => 3,
            "app_scheduler_clear" => 0,
            "app_scheduler_set" => 3,
            "app_scheduler_cancel" => 1,
            "app_scheduler_poll" => 2,
            "app_scheduler_count" => 0,
            "buffer_create" | "buffer_create_i32" => 1,
            "buffer_length"
            | "buffer_capacity"
            | "buffer_clear"
            | "buffer_clear_status"
            | "buffer_clear_move"
            | "buffer_clear_move_status" => 1,
            "buffer_pop" => 1,
            "buffer_remove_move" => 2,
            "buffer_pop_move_into" | "buffer_pop_move_into_status" => 2,
            "buffer_remove_move_into" | "buffer_remove_move_into_status" => 3,
            "buffer_insert_move" | "buffer_insert_move_status" => 3,
            "buffer_insert_move_from" | "buffer_insert_move_from_status" => 3,
            "buffer_append_move" | "buffer_append_move_status" => 2,
            "buffer_resize"
            | "buffer_resize_status"
            | "buffer_resize_move"
            | "buffer_resize_move_status"
            | "buffer_append_i32"
            | "buffer_reserve_i32"
            | "buffer_append_i32_grow"
            | "buffer_reserve"
            | "buffer_reserve_status"
            | "buffer_append"
            | "buffer_append_status"
            | "buffer_remove"
            | "buffer_remove_status"
            | "buffer_remove_drop"
            | "buffer_remove_drop_status" => 2,
            "buffer_insert" | "buffer_insert_status" => 3,
            "string_length" => 1,
            "string_equals" => 2,
            "string_builder_append" | "string_builder_append_bytes" => 3,
            "string_owned_create" | "string_owned_from" => 1,
            "string_owned_append" => 2,
            "string_owned_length" => 1,
            "string_owned_copy" => 2,
            "string_owned_clear" => 1,
            "assert_eq" => 2,
            "file_delete" | "file_flush" | "file_exists" | "directory_create"
            | "directory_exists" | "directory_delete" | "file_size" => 1,
            "file_lock" | "file_unlock" => 1,
            "file_replace_atomic" | "file_copy" => 2,
            "directory_list" => 2,
            "directory_list_ex" => 3,
            "file_read_at" => 3,
            "file_write_at" => 3,
            "file_read_exact" | "file_read_text_exact" => 3,
            "file_read" | "file_read_text" | "file_write" | "file_write_text"
            | "file_append_text" | "format_bool" | "format_int" | "format_uint"
            | "format_float" | "csv_escape" | "json_escape" => 2,
            "parse_int" | "parse_uint" | "parse_float" | "parse_bool" => 3,
            "json_object_field_string"
            | "json_object_field_int"
            | "json_object_field_uint"
            | "json_object_field_float"
            | "json_object_field_bool" => 3,
            "json_object_read_string" => 3,
            "json_object_read_string_exact" => 4,
            "json_object_read_int"
            | "json_object_read_uint"
            | "json_object_read_float"
            | "json_object_read_bool" => 2,
            "json_object_read_int_exact"
            | "json_object_read_uint_exact"
            | "json_object_read_float_exact"
            | "json_object_read_bool_exact" => 3,
            "json_array_int" | "json_array_uint" | "json_array_float" | "json_array_bool" => 2,
            "app_state_clear" | "app_state_count" | "app_state_revision" | "app_data_revision"
            | "app_data_validate" => 0,
            "app_state_exists" | "app_state_remove" => 1,
            "app_state_set_int"
            | "app_state_set_uint"
            | "app_state_set_float"
            | "app_state_set_bool"
            | "app_state_set_text" => 2,
            "app_state_set_text_bytes" => 3,
            "app_state_get_int"
            | "app_state_get_uint"
            | "app_state_get_float"
            | "app_state_get_bool"
            | "app_state_save"
            | "app_state_load"
            | "app_data_save"
            | "app_data_load"
            | "app_data_tx_begin_if_revision" => 1,
            "app_data_write_exact" | "app_data_load_exact" => 2,
            "app_data_write_exact_if_revision" => 3,
            "app_data_load_exact_if_revision" => 3,
            "app_state_tx_begin"
            | "app_state_tx_commit"
            | "app_state_tx_rollback"
            | "app_data_tx_begin"
            | "app_data_tx_commit"
            | "app_data_tx_rollback" => 0,
            "app_state_save_atomic_if_revision" | "app_data_save_atomic_if_revision" => 3,
            "app_state_save_atomic"
            | "app_data_save_atomic"
            | "app_data_tx_save_atomic"
            | "app_data_journal_append"
            | "app_data_journal_recover" => 2,
            "app_data_tx_commit_durable" => 3,
            "app_data_journal_append_durable" => 3,
            "app_data_journal_recover_compact" => 3,
            "app_data_journal_recover_compact_durable" => 4,
            "app_data_journal_compact_if_over_durable" => 5,
            "app_data_journal_compact_if_needed_durable" => 6,
            "app_data_journal_compact_if_frames_over_durable" => 5,
            "app_data_journal_retain_last_durable" => 5,
            "app_data_journal_recover_frame_durable" => 4,
            "app_data_journal_count_frames_durable" => 2,
            "app_data_journal_stats_durable" => 3,
            "app_data_journal_maintenance_plan_durable" => 5,
            "app_data_journal_maintenance_retry_durable" => 8,
            "app_data_journal_frame_length_durable" => 3,
            "app_data_journal_frame_span_durable" => 4,
            "app_data_journal_build_index_durable" => 4,
            "app_data_journal_index_lookup_durable" => 5,
            "app_data_journal_index_export_csv_durable" => 4,
            "app_data_journal_index_export_csv_file_durable" => 5,
            "app_data_journal_index_range_durable" => 6,
            "app_data_journal_index_read_page_durable" => 7,
            "app_data_journal_read_frame_exact_durable" => 5,
            "app_data_journal_read_latest_frame_exact_durable" => 4,
            "app_state_read_text_exact" => 3,
            "app_state_write_json_exact" => 2,
            "app_state_load_json_exact" => 2,
            "app_state_read_text"
            | "app_state_read_int"
            | "app_state_read_uint"
            | "app_state_read_float"
            | "app_state_read_bool" => 2,
            "app_state_read_key" => 2,
            "app_state_type_at" => 1,
            "app_list_clear" | "app_list_count" => 1,
            "app_list_push_text" => 2,
            "app_list_push_text_bytes" => 3,
            "app_list_export_csv" => 2,
            "app_list_sort_text" => 2,
            "app_list_sort_callback" => 2,
            "app_list_find_text" => 3,
            "app_list_filter_text" => 3,
            "app_list_filter_text_ex" => 4,
            "app_list_filter_text_ex_bytes" => 5,
            "app_list_filter_callback" => 3,
            "app_list_page" => 4,
            "app_list_read_text" | "app_list_set_text" => 3,
            "app_list_read_text_exact" => 4,
            "app_list_set_text_bytes" => 4,
            "app_list_remove" => 2,
            "app_list_save" | "app_list_load" => 2,
            "app_list_save_atomic" => 3,
            "app_table_clear"
            | "app_table_row_count"
            | "app_table_append_row"
            | "app_table_tx_begin" => 1,
            "app_table_tx_begin_all"
            | "app_table_tx_commit"
            | "app_table_tx_commit_all"
            | "app_table_tx_rollback"
            | "app_table_tx_rollback_all" => 0,
            "app_table_migration_begin" => 3,
            "app_table_migration_rename_column" | "app_table_migration_set_column_type" => 2,
            "app_table_migration_commit" | "app_table_migration_rollback" => 0,
            "app_table_set_column_name" => 3,
            "app_table_read_column_name" => 3,
            "app_table_read_column_name_exact" => 4,
            "app_table_find_column" => 2,
            "app_table_schema_version" => 1,
            "app_table_set_schema_version" => 2,
            "app_table_set_named_cell" => 4,
            "app_table_read_named_cell" => 4,
            "app_table_read_named_cell_exact" => 5,
            "app_table_set_named_int"
            | "app_table_set_named_uint"
            | "app_table_set_named_float"
            | "app_table_set_named_bool" => 4,
            "app_table_read_named_int"
            | "app_table_read_named_uint"
            | "app_table_read_named_float"
            | "app_table_read_named_bool" => 3,
            "app_table_read_named_int_exact"
            | "app_table_read_named_uint_exact"
            | "app_table_read_named_float_exact"
            | "app_table_read_named_bool_exact" => 4,
            "app_table_set_column_type" => 3,
            "app_table_column_type" => 2,
            "app_table_validate" => 1,
            "app_table_remove_row" => 2,
            "app_table_remove_text"
            | "app_table_remove_int"
            | "app_table_remove_uint"
            | "app_table_remove_float"
            | "app_table_remove_bool" => 3,
            "app_table_set_cell" => 4,
            "app_table_set_cell_bytes" => 4,
            "app_table_set_cell_bytes_ex" => 5,
            "app_table_set_int"
            | "app_table_set_uint"
            | "app_table_set_float"
            | "app_table_set_bool" => 4,
            "app_table_read_cell" => 4,
            "app_table_read_cell_exact" => 5,
            "app_table_read_int"
            | "app_table_read_uint"
            | "app_table_read_float"
            | "app_table_read_bool" => 3,
            "app_table_read_int_exact"
            | "app_table_read_uint_exact"
            | "app_table_read_float_exact"
            | "app_table_read_bool_exact" => 4,
            "app_table_sort_text"
            | "app_table_sort_int"
            | "app_table_sort_uint"
            | "app_table_sort_float"
            | "app_table_sort_bool" => 3,
            "app_table_sort_callback" => 2,
            "app_table_page" => 4,
            "app_table_find_text"
            | "app_table_find_int"
            | "app_table_find_uint"
            | "app_table_find_float"
            | "app_table_find_bool" => 4,
            "app_table_upsert_text"
            | "app_table_upsert_int"
            | "app_table_upsert_uint"
            | "app_table_upsert_float"
            | "app_table_upsert_bool" => 3,
            "app_table_index_build"
            | "app_table_index_build_int"
            | "app_table_index_build_uint"
            | "app_table_index_build_float"
            | "app_table_index_build_bool"
            | "app_table_index_is_valid" => 2,
            "app_table_index_build_pair" => 3,
            "app_table_index_clear" => 1,
            "app_table_index_find_text"
            | "app_table_index_find_int"
            | "app_table_index_find_uint"
            | "app_table_index_find_float"
            | "app_table_index_find_bool" => 3,
            "app_table_index_find_pair_text" => 5,
            "app_table_index_collect_int_range"
            | "app_table_index_collect_uint_range"
            | "app_table_index_collect_float_range" => 5,
            "app_table_filter_text" => 4,
            "app_table_filter_text_ex" => 5,
            "app_table_filter_text_ex_bytes" => 6,
            "app_table_filter_int"
            | "app_table_filter_uint"
            | "app_table_filter_float"
            | "app_table_filter_bool" => 4,
            "app_table_filter_callback" => 3,
            "app_table_export_csv" => 2,
            "app_table_import_csv" => 3,
            "app_table_save"
            | "app_table_load"
            | "app_table_save_schema"
            | "app_table_load_schema"
            | "app_table_save_schema_full"
            | "app_table_load_schema_full" => 2,
            "app_table_load_schema_full_if_version" => 3,
            "app_table_save_atomic"
            | "app_table_save_schema_atomic"
            | "app_table_save_schema_full_atomic" => 3,
            "net_tcp_connect" | "net_tcp_connect_dns" => 2,
            "net_tcp_listen" | "net_tcp_accept" | "net_socket_close" => 1,
            "net_reactor_open" => 2,
            "net_reactor_watch" => 4,
            "net_reactor_unwatch" | "net_reactor_poll" => 2,
            "net_reactor_event_socket" | "net_reactor_event_flags" | "net_reactor_event_user" => 2,
            "net_reactor_error" | "net_reactor_close" => 1,
            "net_reactor_submit_accept"
            | "net_reactor_submit_receive"
            | "net_reactor_submit_send" => 3,
            "net_reactor_submit_receive_buffer" | "net_reactor_submit_send_buffer" => 4,
            "net_reactor_submit_send_buffer_prefix" => 5,
            "net_reactor_submit_connect" => 4,
            "net_reactor_cancel" => 2,
            "net_reactor_event_operation" | "net_reactor_event_bytes" => 2,
            "net_tcp_send" | "net_tcp_receive" => 2,
            "net_tcp_send_prefix" => 3,
            "net_socket_set_timeout" => 2,
            "http_response_write" => 4,
            "http_response_write_ex" => 5,
            "http_response_write_header" => 6,
            "http_response_write_header_ex" => 7,
            "http_response_write_cookie" => 7,
            "http_response_write_cookie_ex" => 8,
            "http_response_write_header_block" => 5,
            "http_response_write_header_block_ex" => 6,
            "http_response_status" => 1,
            "http_response_status_prefix" => 2,
            "http_response_header" => 3,
            "http_response_header_prefix" => 4,
            "http_response_body" => 2,
            "http_response_body_prefix" => 3,
            "http_response_body_chunked_exact" | "http_request_body_chunked_exact" => 3,
            "http_request_write" => 5,
            "http_request_write_prefix" => 6,
            "http_request_write_header" => 7,
            "http_request_write_header_block" => 6,
            "http_request_append" => 4,
            "http_request_is_complete" => 1,
            "http_request_is_complete_prefix" => 2,
            "http_request_frame_length_prefix" | "http_request_chunked_frame_length_prefix" => 2,
            "http_request_consume_prefix" => 3,
            "http_request_keep_alive" => 1,
            "http_request_method" | "http_request_target" | "http_request_body" => 2,
            "http_request_header" => 3,
            "http_query_param" => 3,
            "http_query_param_exact" => 4,
            "http_route_match" => 3,
            "http_route_match_prefix" => 4,
            "http_router_clear" => 0,
            "http_router_add" => 5,
            "http_router_add_exact" => 6,
            "http_router_add_prefix" => 5,
            "http_router_remove" => 2,
            "http_router_remove_prefix" => 2,
            "http_router_respond" => 2,
            "http_router_respond_prefix" => 3,
            "http_router_count" => 0,
            "http_session_open" => 4,
            "http_session_open_tls" => 6,
            "http_session_step" => 2,
            "http_session_close" => 1,
            "net_tls_open_client" => 3,
            "net_tls_open_server" => 3,
            "net_tls_step" => 2,
            "net_tls_state" | "net_tls_error" | "net_tls_close" => 1,
            "net_tls_send" | "net_tls_receive" => 2,
            "file_append" => 3,
            "ui_app_begin" => 4,
            "ui_app_on_resize" | "ui_app_on_close" => 1,
            "ui_app_window_width" | "ui_app_window_height" => 0,
            "ui_app_window_set_constraints" => 4,
            "ui_app_window_min_width"
            | "ui_app_window_min_height"
            | "ui_app_window_max_width"
            | "ui_app_window_max_height" => 0,
            "ui_app_panel" => 9,
            "ui_app_row" => 7,
            "ui_app_top_bar" => 9,
            "ui_app_menu" => 8,
            "ui_app_menu_item" => 3,
            "ui_app_tooltip" => 7,
            "ui_app_label" => 8,
            "ui_app_status" => 8,
            "ui_app_button" => 9,
            "ui_app_text_input" => 9,
            "ui_app_checkbox" => 10,
            "ui_app_select" => 8,
            "ui_app_select_option" => 2,
            "ui_app_select_index" => 1,
            "ui_app_select_set_index" => 2,
            "ui_app_list" => 8,
            "ui_app_list_item" | "ui_app_list_bind_app" => 2,
            "ui_app_list_read_item" => 3,
            "ui_app_list_clear"
            | "ui_app_list_count"
            | "ui_app_list_index"
            | "ui_app_list_refresh" => 1,
            "ui_app_list_set_index" => 2,
            "ui_app_bind_app_state" => 2,
            "ui_app_refresh_app_state" => 1,
            "ui_app_table" => 8,
            "ui_app_table_column" | "ui_app_table_cell" => 4,
            "ui_app_table_read_cell" => 4,
            "ui_app_table_bind_app" => 3,
            "ui_app_table_refresh"
            | "ui_app_table_clear"
            | "ui_app_table_row_count"
            | "ui_app_table_selected_row" => 1,
            "ui_app_table_set_selected_row" => 2,
            "ui_app_table_sort_text"
            | "ui_app_table_sort_int"
            | "ui_app_table_sort_uint"
            | "ui_app_table_sort_float"
            | "ui_app_table_sort_bool" => 3,
            "ui_app_table_filter_text" => 4,
            "ui_app_table_filter_text_ex" => 5,
            "ui_app_table_filter_int"
            | "ui_app_table_filter_uint"
            | "ui_app_table_filter_float"
            | "ui_app_table_filter_bool" => 4,
            "ui_app_end" => 1,
            "ui_app_run" => 0,
            "ui_window" => 4,
            "ui_top_bar" => 2,
            "ui_label" | "ui_status" | "ui_text" => 8,
            "ui_scroll_panel" => 8,
            "ui_image" => 5,
            "ui_button" | "ui_toggle_button" | "ui_menu_item" | "ui_icon_button" => 9,
            "ui_menu" => 9,
            "ui_checkbox" | "ui_switch" | "ui_text_input" => 9,
            "ui_checked" | "ui_select_index" => 1,
            "ui_list_clear" | "ui_list_count" | "ui_list_index" => 1,
            "ui_close_button" | "ui_disabled_button" => 8,
            "ui_tooltip" => 7,
            "ui_column" | "ui_panel" => 8,
            "ui_event_button" => 9,
            "ui_row" => 6,
            "ui_layout_label" | "ui_layout_status" => 7,
            "ui_layout_event_button" => 8,
            "ui_layout_end" => 0,
            "ui_run" => 0,
            "ui_set_button_enabled" => 2,
            "ui_set_button_text" => 2,
            "ui_set_checked" | "ui_set_input_enabled" | "ui_set_input_text" => 2,
            "ui_input_length" => 1,
            "ui_input_read_exact" => 3,
            "ui_input_read" => 2,
            "ui_input_bind_app_state" => 2,
            "ui_input_refresh_app_state" => 1,
            "ui_checkbox_bind_app_state"
            | "ui_select_bind_app_state"
            | "ui_list_bind_app_state"
            | "ui_table_bind_app_state" => 2,
            "ui_checkbox_refresh_app_state"
            | "ui_select_refresh_app_state"
            | "ui_list_refresh_app_state"
            | "ui_table_refresh_app_state" => 1,
            "ui_select" => 7,
            "ui_select_option" | "ui_select_set_index" => 2,
            "ui_menu_option" => 3,
            "ui_list" => 7,
            "ui_list_item" | "ui_list_set_index" => 2,
            "ui_list_read_item" => 3,
            "ui_list_set_item" => 3,
            "ui_list_bind_app" => 2,
            "ui_list_refresh_app" => 1,
            "ui_table" => 7,
            "ui_table_column" | "ui_table_cell" => 4,
            "ui_table_read_cell" => 4,
            "ui_table_bind_app" => 3,
            "ui_table_refresh_app" => 1,
            "ui_refresh_bindings" => 0,
            "ui_table_clear" | "ui_table_row_count" | "ui_table_selected_row" => 1,
            "ui_table_set_selected_row" => 2,
            "ui_table_sort_text"
            | "ui_table_sort_int"
            | "ui_table_sort_uint"
            | "ui_table_sort_float"
            | "ui_table_sort_bool" => 3,
            "ui_table_filter_text" => 4,
            "ui_table_filter_text_ex" => 5,
            "ui_table_filter_int"
            | "ui_table_filter_uint"
            | "ui_table_filter_float"
            | "ui_table_filter_bool" => 4,
            "ui_theme" | "ui_theme_color" => 1,
            "ui_set_status" => 1,
            "ui_state_get" => 1,
            "ui_state_bind" => 3,
            "ui_state_bind_text" => 2,
            "ui_state_set" => 2,
            "ui_state_text_length" => 1,
            "ui_state_text_read" => 2,
            "ui_state_text_set" => 2,
            "vector_splat2" | "vector_splat3" | "vector_splat4" | "vector_splat8" => 1,
            "vector_load2" | "vector_load3" | "vector_load4" | "vector_load8" => 2,
            "vector_store2" | "vector_store3" | "vector_store4" | "vector_store8" => 3,
            _ => return self.unification.fresh(&mut self.types),
        };
        if argument_types.len() != expected {
            self.diagnostics.push(Diagnostic::error(
                "J0304",
                format!(
                    "builtin `{name}` expects {expected} arguments but received {}",
                    argument_types.len()
                ),
                span,
                "incorrect builtin argument count",
            ));
        }
        if name == "assert_eq"
            && let (Some(left_expression), Some(right_expression), Some(left), Some(right)) = (
                arguments.first(),
                arguments.get(1),
                argument_types.first(),
                argument_types.get(1),
            )
        {
            let mismatch_span = Span::new(
                right_expression.span().source,
                left_expression.span().start,
                right_expression.span().end,
            )
            .unwrap_or(span);
            self.unify_or_error(*left, *right, mismatch_span);
        }
        let core = self.types.core();
        let float32 = core.float32;
        let vector = match name {
            "vector_splat2" | "vector_load2" | "vector_store2" => Some(core.float2),
            "vector_splat3" | "vector_load3" | "vector_store3" => Some(core.float3),
            "vector_splat4" | "vector_load4" | "vector_store4" => Some(core.float4),
            "vector_splat8" | "vector_load8" | "vector_store8" => Some(core.float8),
            _ => None,
        };
        let uint_size = core.uint_size;
        let slice_float32 = self.types.intern(TypeKind::Slice(float32));
        let read_slice_float32 = self.types.intern(TypeKind::Capability {
            capability: Capability::Read,
            inner: slice_float32,
        });
        let write_slice_float32 = self.types.intern(TypeKind::Capability {
            capability: Capability::Write,
            inner: slice_float32,
        });
        let byte_slice = self.types.intern(TypeKind::Slice(core.uint8));
        let read_byte_slice = self.types.intern(TypeKind::Capability {
            capability: Capability::Read,
            inner: byte_slice,
        });
        let read_string = self.types.intern(TypeKind::Capability {
            capability: Capability::Read,
            inner: core.string,
        });
        let read_owned_string = self.types.intern(TypeKind::Capability {
            capability: Capability::Read,
            inner: core.owned_string,
        });
        let write_owned_string = self.types.intern(TypeKind::Capability {
            capability: Capability::Write,
            inner: core.owned_string,
        });
        let write_byte_slice = self.types.intern(TypeKind::Capability {
            capability: Capability::Write,
            inner: byte_slice,
        });
        let slice_int64 = self.types.intern(TypeKind::Slice(core.int64));
        let slice_int32 = self.types.intern(TypeKind::Slice(core.int32));
        let slice_uint64 = self.types.intern(TypeKind::Slice(core.uint64));
        let slice_uint_size = self.types.intern(TypeKind::Slice(core.uint_size));
        let slice_float64 = self.types.intern(TypeKind::Slice(core.float64));
        let slice_bool = self.types.intern(TypeKind::Slice(core.bool_));
        let read_slice_int64 = self.types.intern(TypeKind::Capability {
            capability: Capability::Read,
            inner: slice_int64,
        });
        let write_slice_int64 = self.types.intern(TypeKind::Capability {
            capability: Capability::Write,
            inner: slice_int64,
        });
        let write_slice_int32 = self.types.intern(TypeKind::Capability {
            capability: Capability::Write,
            inner: slice_int32,
        });
        let read_slice_uint64 = self.types.intern(TypeKind::Capability {
            capability: Capability::Read,
            inner: slice_uint64,
        });
        let write_slice_uint64 = self.types.intern(TypeKind::Capability {
            capability: Capability::Write,
            inner: slice_uint64,
        });
        let write_slice_uint_size = self.types.intern(TypeKind::Capability {
            capability: Capability::Write,
            inner: slice_uint_size,
        });
        let read_slice_float64 = self.types.intern(TypeKind::Capability {
            capability: Capability::Read,
            inner: slice_float64,
        });
        let write_slice_float64 = self.types.intern(TypeKind::Capability {
            capability: Capability::Write,
            inner: slice_float64,
        });
        let read_slice_bool = self.types.intern(TypeKind::Capability {
            capability: Capability::Read,
            inner: slice_bool,
        });
        let write_slice_bool = self.types.intern(TypeKind::Capability {
            capability: Capability::Write,
            inner: slice_bool,
        });
        let arg_span = |index: usize| arguments.get(index).map_or(span, Expression::span);
        match name {
            "time_now_unix_seconds" => core.int64,
            "time_now_monotonic_ms" => core.uint64,
            "process_arg_count" => core.uint_size,
            "process_arg_read" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "stdin_read" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(0));
                }
                core.uint_size
            }
            "stdout_write" | "stderr_write" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                core.uint_size
            }
            "time_utc_parts" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int64, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_slice_int32, *actual, arg_span(1));
                }
                core.bool_
            }
            "time_utc_offset_parts" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int64, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_slice_int32, *actual, arg_span(2));
                }
                core.bool_
            }
            "app_scheduler_clear" => core.unit,
            "app_scheduler_set" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int64, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.uint64, *actual, arg_span(2));
                }
                core.bool_
            }
            "app_scheduler_cancel" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.bool_
            }
            "app_scheduler_poll" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int64, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_slice_int32, *actual, arg_span(1));
                }
                core.uint_size
            }
            "app_scheduler_count" => core.uint_size,
            "buffer_create" | "buffer_create_i32" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                let element = if name == "buffer_create" {
                    self.generic_buffer_element_from_expected(return_type)
                        .unwrap_or_else(|| self.unification.fresh(&mut self.types))
                } else {
                    core.int32
                };
                self.require_buffer_element_move_abi(element, span);
                let buffer = self.types.intern(TypeKind::Buffer(element));
                self.types.intern(TypeKind::Result {
                    ok: buffer,
                    error: core.int32,
                })
            }
            "buffer_length" | "buffer_capacity" => {
                if let Some(actual) = argument_types.first() {
                    self.require_buffer_capability(*actual, Capability::Read, arg_span(0));
                }
                core.uint_size
            }
            "buffer_clear" | "buffer_clear_status" => {
                if let Some(element) = argument_types
                    .first()
                    .and_then(|actual| self.buffer_element_from_type(*actual))
                {
                    // Clear only changes the logical length.  Keep it
                    // copy-safe until a move-aware variant can destroy nested
                    // owning values before publishing length zero.
                    self.require_buffer_element_abi(element, arg_span(0));
                }
                if let Some(actual) = argument_types.first() {
                    self.require_buffer_capability(*actual, Capability::Write, arg_span(0));
                }
                if name.ends_with("_status") {
                    core.int32
                } else {
                    core.bool_
                }
            }
            "buffer_clear_move" | "buffer_clear_move_status" => {
                let Some(outer) = argument_types.first() else {
                    return self.type_error(
                        "J0301",
                        "buffer_clear_move requires Buffer<Buffer<U>>, Buffer<OwnedString>, or an owning @repr(C) record Buffer",
                        span,
                    );
                };
                let Some(element) = self.buffer_element_from_type(*outer) else {
                    return self.type_error(
                        "J0301",
                        "buffer_clear_move requires Buffer<Buffer<U>>, Buffer<OwnedString>, or an owning @repr(C) record Buffer",
                        arg_span(0),
                    );
                };
                let nested = matches!(self.types.kind(element), Some(TypeKind::Buffer(_)));
                let owned_string = matches!(self.types.kind(element), Some(TypeKind::OwnedString));
                let direct_record = if nested || owned_string {
                    false
                } else {
                    let mut record_visiting = DeterministicSet::new();
                    self.resize_record_fields_are_supported(element, &mut record_visiting)
                        && self
                            .buffer_remove_drop_element_is_supported(element, &mut record_visiting)
                };
                if !nested && !owned_string && !direct_record {
                    return self.type_error(
                        "J0301",
                        "buffer_clear_move requires Buffer<Buffer<U>>, Buffer<OwnedString>, or an owning @repr(C) record Buffer",
                        arg_span(0),
                    );
                }
                self.require_buffer_capability(*outer, Capability::Write, arg_span(0));
                let mut visiting = DeterministicSet::new();
                if (nested && !self.nested_buffer_chain_leaf_is_resize_safe(element, &mut visiting))
                    || (direct_record
                        && !self.resize_record_fields_are_supported(element, &mut visiting))
                {
                    return self.type_error(
                        "J0301",
                        "buffer_clear_move requires a copy-safe leaf, OwnedString, or owning @repr(C) record leaf",
                        arg_span(0),
                    );
                }
                self.require_buffer_element_move_abi(element, arg_span(0));
                if name.ends_with("_status") {
                    core.int32
                } else {
                    core.bool_
                }
            }
            "buffer_pop" => {
                let Some(outer) = argument_types.first() else {
                    return self.type_error(
                        "J0301",
                        "buffer_pop requires Buffer<Buffer<U>> or Buffer<OwnedString>",
                        span,
                    );
                };
                let Some(element) = self.buffer_element_from_type(*outer) else {
                    return self.type_error(
                        "J0301",
                        "buffer_pop requires Buffer<Buffer<U>> or Buffer<OwnedString>",
                        arg_span(0),
                    );
                };
                let result_element = match self.types.kind(element).cloned() {
                    Some(TypeKind::Buffer(inner)) => self.types.intern(TypeKind::Buffer(inner)),
                    Some(TypeKind::OwnedString) => element,
                    _ => {
                        return self.type_error(
                            "J0301",
                            "buffer_pop requires Buffer<Buffer<U>> or Buffer<OwnedString>",
                            arg_span(0),
                        );
                    }
                };
                if !self.buffer_element_is_abi_safe(element, &mut DeterministicSet::new(), true) {
                    return self.type_error(
                        "J0301",
                        "buffer_pop requires a move-safe owning element",
                        arg_span(0),
                    );
                }
                self.require_buffer_capability(*outer, Capability::Write, arg_span(0));
                self.require_buffer_element_move_abi(element, arg_span(0));
                self.types.intern(TypeKind::Result {
                    ok: result_element,
                    error: core.int32,
                })
            }
            "buffer_remove_move" => {
                let Some(outer) = argument_types.first() else {
                    return self.type_error(
                        "J0301",
                        "buffer_remove_move requires Buffer<Buffer<U>> or Buffer<OwnedString>",
                        span,
                    );
                };
                let Some(element) = self.buffer_element_from_type(*outer) else {
                    return self.type_error(
                        "J0301",
                        "buffer_remove_move requires Buffer<Buffer<U>> or Buffer<OwnedString>",
                        arg_span(0),
                    );
                };
                let result_element = match self.types.kind(element).cloned() {
                    Some(TypeKind::Buffer(inner)) => self.types.intern(TypeKind::Buffer(inner)),
                    Some(TypeKind::OwnedString) => element,
                    _ => {
                        return self.type_error(
                            "J0301",
                            "buffer_remove_move requires Buffer<Buffer<U>> or Buffer<OwnedString>",
                            arg_span(0),
                        );
                    }
                };
                self.require_buffer_capability(*outer, Capability::Write, arg_span(0));
                self.require_buffer_element_move_abi(element, arg_span(0));
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                self.types.intern(TypeKind::Result {
                    ok: result_element,
                    error: core.int32,
                })
            }
            "buffer_pop_move_into"
            | "buffer_pop_move_into_status"
            | "buffer_remove_move_into"
            | "buffer_remove_move_into_status" => {
                let is_pop_move_into =
                    matches!(name, "buffer_pop_move_into" | "buffer_pop_move_into_status");
                let output_message = if is_pop_move_into {
                    "buffer_pop_move_into requires Buffer<T> and a write output"
                } else {
                    "buffer_remove_move_into requires Buffer<T> and a write output"
                };
                let buffer_message = if is_pop_move_into {
                    "buffer_pop_move_into requires Buffer<T>"
                } else {
                    "buffer_remove_move_into requires Buffer<T>"
                };
                let owning_message = if is_pop_move_into {
                    "buffer_pop_move_into requires a move-safe owning Buffer element"
                } else {
                    "buffer_remove_move_into requires a move-safe owning Buffer element"
                };
                let descriptor_message = if is_pop_move_into {
                    "buffer_pop_move_into requires an owned Buffer output descriptor"
                } else {
                    "buffer_remove_move_into requires an owned Buffer output descriptor"
                };
                let Some(outer) = argument_types.first() else {
                    return self.type_error("J0301", output_message, span);
                };
                let Some(element) = self.buffer_element_from_type(*outer) else {
                    return self.type_error("J0301", buffer_message, arg_span(0));
                };
                let mut visiting = DeterministicSet::new();
                if !self.buffer_move_into_element_is_supported(element, &mut visiting) {
                    return self.type_error("J0301", owning_message, arg_span(0));
                }
                self.require_buffer_capability(*outer, Capability::Write, arg_span(0));
                if !is_pop_move_into && let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                let write_element = self.types.intern(TypeKind::Capability {
                    capability: Capability::Write,
                    inner: element,
                });
                let output_index = if is_pop_move_into { 1 } else { 2 };
                if let Some(actual) = argument_types.get(output_index) {
                    let requires_owned_descriptor =
                        matches!(self.types.kind(element), Some(TypeKind::Buffer(_)))
                            && matches!(
                                self.types.kind(*actual),
                                Some(TypeKind::Capability { .. })
                            );
                    if requires_owned_descriptor {
                        self.type_error("J0301", descriptor_message, arg_span(output_index));
                    } else {
                        self.unify_or_error(write_element, *actual, arg_span(output_index));
                    }
                }
                if name.ends_with("_status") {
                    core.int32
                } else {
                    core.bool_
                }
            }
            "buffer_insert_move" | "buffer_insert_move_status" => {
                let Some(outer) = argument_types.first() else {
                    return self.type_error(
                        "J0301",
                        "buffer_insert_move requires Buffer<Buffer<U>> or Buffer<OwnedString>",
                        span,
                    );
                };
                let Some(element) = self.buffer_element_from_type(*outer) else {
                    return self.type_error(
                        "J0301",
                        "buffer_insert_move requires Buffer<Buffer<U>> or Buffer<OwnedString>",
                        arg_span(0),
                    );
                };
                let expected = match self.types.kind(element).cloned() {
                    Some(TypeKind::Buffer(inner)) => self.types.intern(TypeKind::Buffer(inner)),
                    Some(TypeKind::OwnedString) => element,
                    _ => {
                        return self.type_error(
                            "J0301",
                            "buffer_insert_move requires Buffer<Buffer<U>> or Buffer<OwnedString>",
                            arg_span(0),
                        );
                    }
                };
                self.require_buffer_capability(*outer, Capability::Write, arg_span(0));
                self.require_buffer_element_move_abi(element, arg_span(0));
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(expected, *actual, arg_span(2));
                }
                if name.ends_with("_status") {
                    core.int32
                } else {
                    core.bool_
                }
            }
            "buffer_insert_move_from" | "buffer_insert_move_from_status" => {
                let Some(outer) = argument_types.first() else {
                    return self.type_error(
                        "J0301",
                        "buffer_insert_move_from requires Buffer<T>, an index, and a move-safe T",
                        span,
                    );
                };
                let Some(element) = self.buffer_element_from_type(*outer) else {
                    return self.type_error(
                        "J0301",
                        "buffer_insert_move_from requires Buffer<T>",
                        arg_span(0),
                    );
                };
                let mut visiting = DeterministicSet::new();
                if !self.buffer_move_into_element_is_supported(element, &mut visiting) {
                    return self.type_error(
                        "J0301",
                        "buffer_insert_move_from requires a move-safe owning Buffer element",
                        arg_span(0),
                    );
                }
                self.require_buffer_capability(*outer, Capability::Write, arg_span(0));
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(element, *actual, arg_span(2));
                }
                if name.ends_with("_status") {
                    core.int32
                } else {
                    core.bool_
                }
            }
            "buffer_append_move" | "buffer_append_move_status" => {
                let Some(outer) = argument_types.first() else {
                    return self.type_error(
                        "J0301",
                        "buffer_append_move requires Buffer<T> and a move-safe T",
                        span,
                    );
                };
                let Some(element) = self.buffer_element_from_type(*outer) else {
                    return self.type_error(
                        "J0301",
                        "buffer_append_move requires Buffer<T>",
                        arg_span(0),
                    );
                };
                let mut visiting = DeterministicSet::new();
                if !self.buffer_move_into_element_is_supported(element, &mut visiting) {
                    return self.type_error(
                        "J0301",
                        "buffer_append_move requires a move-safe owning Buffer element",
                        arg_span(0),
                    );
                }
                self.require_buffer_capability(*outer, Capability::Write, arg_span(0));
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(element, *actual, arg_span(1));
                }
                if name.ends_with("_status") {
                    core.int32
                } else {
                    core.bool_
                }
            }
            "buffer_resize_move" | "buffer_resize_move_status" => {
                let Some(outer) = argument_types.first() else {
                    return self.type_error(
                        "J0301",
                        "buffer_resize_move requires Buffer<Buffer<U>>, Buffer<OwnedString>, or an owning @repr(C) record Buffer",
                        span,
                    );
                };
                let Some(element) = self.buffer_element_from_type(*outer) else {
                    return self.type_error(
                        "J0301",
                        "buffer_resize_move requires Buffer<Buffer<U>>, Buffer<OwnedString>, or an owning @repr(C) record Buffer",
                        arg_span(0),
                    );
                };
                let nested = matches!(self.types.kind(element), Some(TypeKind::Buffer(_)));
                let owned_string = matches!(self.types.kind(element), Some(TypeKind::OwnedString));
                let direct_record = if nested || owned_string {
                    false
                } else {
                    let mut record_visiting = DeterministicSet::new();
                    self.resize_record_fields_are_supported(element, &mut record_visiting)
                        && self
                            .buffer_remove_drop_element_is_supported(element, &mut record_visiting)
                };
                if !nested && !owned_string && !direct_record {
                    return self.type_error(
                        "J0301",
                        "buffer_resize_move requires Buffer<Buffer<U>>, Buffer<OwnedString>, or an owning @repr(C) record Buffer",
                        arg_span(0),
                    );
                }
                self.require_buffer_capability(*outer, Capability::Write, arg_span(0));
                let mut visiting = DeterministicSet::new();
                if (nested && !self.nested_buffer_chain_leaf_is_resize_safe(element, &mut visiting))
                    || (direct_record
                        && !self.resize_record_fields_are_supported(element, &mut visiting))
                {
                    return self.type_error(
                        "J0301",
                        "buffer_resize_move requires a copy-safe leaf, OwnedString, or owning @repr(C) record leaf",
                        arg_span(0),
                    );
                }
                self.require_buffer_element_move_abi(element, arg_span(0));
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                if name.ends_with("_status") {
                    core.int32
                } else {
                    core.bool_
                }
            }
            "buffer_resize" | "buffer_resize_status" => {
                if let Some(element) = argument_types
                    .first()
                    .and_then(|actual| self.buffer_element_from_type(*actual))
                {
                    // Resize can expose previously uninitialized slots. The
                    // existing API has no move-only initialization contract,
                    // so nested owning elements stay out until a typed grow
                    // operation is added.
                    self.require_buffer_element_abi(element, arg_span(0));
                }
                if let Some(actual) = argument_types.first() {
                    self.require_buffer_capability(*actual, Capability::Write, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                if name.ends_with("_status") {
                    core.int32
                } else {
                    core.bool_
                }
            }
            "buffer_append_i32" | "buffer_append_i32_grow" => {
                if let Some(actual) = argument_types.first() {
                    self.require_buffer_capability(*actual, Capability::Write, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                core.bool_
            }
            "buffer_reserve_i32" => {
                if let Some(actual) = argument_types.first() {
                    self.require_buffer_capability(*actual, Capability::Write, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                core.bool_
            }
            "buffer_reserve" | "buffer_reserve_status" => {
                if let Some(actual) = argument_types
                    .first()
                    .and_then(|actual| self.buffer_element_from_type(*actual))
                {
                    self.require_buffer_element_move_abi(actual, arg_span(0));
                }
                if let Some(actual) = argument_types.first() {
                    self.require_buffer_capability(*actual, Capability::Write, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                if name.ends_with("_status") {
                    core.int32
                } else {
                    core.bool_
                }
            }
            "buffer_append" | "buffer_append_status" => {
                let element = argument_types
                    .first()
                    .and_then(|actual| self.buffer_element_from_type(*actual));
                if let Some(element) = element {
                    self.require_buffer_element_move_abi(element, arg_span(0));
                }
                if let Some(actual) = argument_types.first() {
                    self.require_buffer_capability(*actual, Capability::Write, arg_span(0));
                }
                if let (Some(element), Some(actual)) = (element, argument_types.get(1)) {
                    self.unify_or_error(element, *actual, arg_span(1));
                }
                if name.ends_with("_status") {
                    core.int32
                } else {
                    core.bool_
                }
            }
            "buffer_insert" | "buffer_insert_status" => {
                let element = argument_types
                    .first()
                    .and_then(|actual| self.buffer_element_from_type(*actual));
                if let Some(element) = element {
                    self.require_buffer_element_move_abi(element, arg_span(0));
                }
                if let Some(actual) = argument_types.first() {
                    self.require_buffer_capability(*actual, Capability::Write, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                if let (Some(element), Some(actual)) = (element, argument_types.get(2)) {
                    self.unify_or_error(element, *actual, arg_span(2));
                }
                if name.ends_with("_status") {
                    core.int32
                } else {
                    core.bool_
                }
            }
            "buffer_remove" | "buffer_remove_status" => {
                if let Some(element) = argument_types
                    .first()
                    .and_then(|actual| self.buffer_element_from_type(*actual))
                {
                    self.require_buffer_element_abi(element, arg_span(0));
                }
                if let Some(actual) = argument_types.first() {
                    self.require_buffer_capability(*actual, Capability::Write, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                if name.ends_with("_status") {
                    core.int32
                } else {
                    core.bool_
                }
            }
            "buffer_remove_drop" | "buffer_remove_drop_status" => {
                if let Some(element) = argument_types
                    .first()
                    .and_then(|actual| self.buffer_element_from_type(*actual))
                {
                    let mut visiting = DeterministicSet::new();
                    if !self.buffer_remove_drop_element_is_supported(element, &mut visiting) {
                        self.type_error(
                            "J0301",
                            "buffer_remove_drop requires a copy-safe element or a direct @repr(C) record with owning Buffer fields",
                            arg_span(0),
                        );
                    }
                }
                if let Some(actual) = argument_types.first() {
                    self.require_buffer_capability(*actual, Capability::Write, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                if name.ends_with("_status") {
                    core.int32
                } else {
                    core.bool_
                }
            }
            "string_length" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_string, *actual, arg_span(0));
                }
                core.uint_size
            }
            "string_equals" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(read_string, *actual, arg_span(index));
                }
                core.bool_
            }
            "string_builder_append" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(2));
                }
                core.uint_size
            }
            "string_builder_append_bytes" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(2));
                }
                core.uint_size
            }
            "string_owned_create" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                self.types.intern(TypeKind::Result {
                    ok: core.owned_string,
                    error: core.int32,
                })
            }
            "string_owned_from" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_string, *actual, arg_span(0));
                }
                self.types.intern(TypeKind::Result {
                    ok: core.owned_string,
                    error: core.int32,
                })
            }
            "string_owned_append" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(write_owned_string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(read_string, *actual, arg_span(1));
                }
                core.int32
            }
            "string_owned_length" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_owned_string, *actual, arg_span(0));
                }
                core.uint_size
            }
            "string_owned_copy" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_owned_string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "string_owned_clear" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(write_owned_string, *actual, arg_span(0));
                }
                core.int32
            }
            "file_delete" | "file_flush" | "file_exists" | "directory_create"
            | "directory_exists" | "directory_delete" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                core.bool_
            }
            "file_lock" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                core.uint_size
            }
            "file_unlock" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                core.bool_
            }
            "directory_list" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "directory_list_ex" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                for (index, actual) in argument_types.iter().skip(1).take(2).enumerate() {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(index + 1));
                }
                core.uint_size
            }
            "file_replace_atomic" | "file_copy" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index));
                }
                core.bool_
            }
            "file_size" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                core.uint_size
            }
            "file_read" | "file_read_text" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "file_read_exact" | "file_read_text_exact" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_slice_uint_size, *actual, arg_span(2));
                }
                core.bool_
            }
            "file_read_at" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(2));
                }
                core.uint_size
            }
            "file_write" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "file_write_at" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(2));
                }
                core.uint_size
            }
            "file_write_text" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index));
                }
                core.uint_size
            }
            "file_append_text" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index));
                }
                core.uint_size
            }
            "file_append" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(2));
                }
                core.uint_size
            }
            "format_bool" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.bool_, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "format_int" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int64, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "format_uint" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint64, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "format_float" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.float64, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "parse_int" | "parse_uint" | "parse_float" | "parse_bool" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    let output = match name {
                        "parse_int" => write_slice_int64,
                        "parse_uint" => write_slice_uint64,
                        "parse_float" => write_slice_float64,
                        "parse_bool" => write_slice_bool,
                        _ => unreachable!("parser builtin was matched above"),
                    };
                    self.unify_or_error(output, *actual, arg_span(2));
                }
                core.bool_
            }
            "csv_escape" | "json_escape" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "json_object_field_string" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(2));
                }
                core.uint_size
            }
            "json_object_read_string" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(2));
                }
                core.uint_size
            }
            "json_object_read_string_exact" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(write_slice_uint_size, *actual, arg_span(3));
                }
                core.bool_
            }
            "json_object_read_int" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                core.int64
            }
            "json_object_read_uint" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                core.uint64
            }
            "json_object_read_float" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                core.float64
            }
            "json_object_read_bool" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                core.bool_
            }
            "json_object_read_int_exact"
            | "json_object_read_uint_exact"
            | "json_object_read_float_exact"
            | "json_object_read_bool_exact" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    let output = match name {
                        "json_object_read_int_exact" => write_slice_int64,
                        "json_object_read_uint_exact" => write_slice_uint64,
                        "json_object_read_float_exact" => write_slice_float64,
                        _ => write_slice_bool,
                    };
                    self.unify_or_error(output, *actual, arg_span(2));
                }
                core.bool_
            }
            "json_array_int" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_slice_int64, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "json_array_uint" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_slice_uint64, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "json_array_float" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_slice_float64, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "json_array_bool" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_slice_bool, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "app_state_clear" => core.unit,
            "app_state_count" => core.int32,
            "app_state_revision" => core.uint64,
            "app_data_revision" => core.uint64,
            "app_data_validate" => core.bool_,
            "app_state_type_at" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.int32
            }
            "app_state_exists" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                core.bool_
            }
            "app_state_tx_begin"
            | "app_state_tx_commit"
            | "app_state_tx_rollback"
            | "app_data_tx_begin"
            | "app_data_tx_begin_if_revision"
            | "app_data_tx_commit"
            | "app_data_tx_rollback" => core.bool_,
            "app_state_remove" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                core.bool_
            }
            "app_state_set_int" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int64, *actual, arg_span(1));
                }
                core.bool_
            }
            "app_state_get_int" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                core.int64
            }
            "app_state_set_uint" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint64, *actual, arg_span(1));
                }
                core.bool_
            }
            "app_state_get_uint" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                core.uint64
            }
            "app_state_set_float" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.float64, *actual, arg_span(1));
                }
                core.bool_
            }
            "app_state_get_float" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                core.float64
            }
            "app_state_set_bool" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.bool_, *actual, arg_span(1));
                }
                core.bool_
            }
            "app_state_get_bool" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                core.bool_
            }
            "app_state_set_text" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index));
                }
                core.bool_
            }
            "app_state_set_text_bytes" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(2));
                }
                core.bool_
            }
            "app_state_read_text" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "app_state_read_text_exact" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_slice_uint_size, *actual, arg_span(2));
                }
                core.bool_
            }
            "app_state_write_json_exact" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_slice_uint_size, *actual, arg_span(1));
                }
                core.bool_
            }
            "app_state_load_json_exact" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                core.bool_
            }
            "app_data_write_exact" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_slice_uint_size, *actual, arg_span(1));
                }
                core.bool_
            }
            "app_data_write_exact_if_revision" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_slice_uint_size, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.uint64, *actual, arg_span(2));
                }
                core.bool_
            }
            "app_data_load_exact" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                core.bool_
            }
            "app_data_load_exact_if_revision" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.uint64, *actual, arg_span(2));
                }
                core.bool_
            }
            "app_state_read_int" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_slice_int64, *actual, arg_span(1));
                }
                core.bool_
            }
            "app_state_read_uint" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_slice_uint64, *actual, arg_span(1));
                }
                core.bool_
            }
            "app_state_read_float" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_slice_float64, *actual, arg_span(1));
                }
                core.bool_
            }
            "app_state_read_bool" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_slice_bool, *actual, arg_span(1));
                }
                core.bool_
            }
            "app_state_read_key" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "app_state_save" | "app_state_load" | "app_data_save" | "app_data_load" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                core.bool_
            }
            "app_state_save_atomic"
            | "app_state_save_atomic_if_revision"
            | "app_data_save_atomic_if_revision"
            | "app_data_save_atomic"
            | "app_data_tx_save_atomic"
            | "app_data_journal_append"
            | "app_data_journal_recover" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index));
                }
                if (name == "app_state_save_atomic_if_revision"
                    || name == "app_data_save_atomic_if_revision")
                    && let Some(actual) = argument_types.get(2)
                {
                    self.unify_or_error(core.uint64, *actual, arg_span(2));
                }
                core.bool_
            }
            "app_data_journal_append_durable" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index));
                }
                core.bool_
            }
            "app_data_tx_commit_durable" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index));
                }
                core.bool_
            }
            "app_data_journal_recover_compact" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index));
                }
                core.bool_
            }
            "app_data_journal_recover_compact_durable" => {
                for (index, actual) in argument_types.iter().take(4).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index));
                }
                core.bool_
            }
            "app_data_journal_compact_if_over_durable" => {
                for (index, actual) in argument_types.iter().take(4).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index));
                }
                self.unify_or_error(core.uint_size, argument_types[4], arg_span(4));
                core.bool_
            }
            "app_data_journal_compact_if_needed_durable" => {
                for (index, actual) in argument_types.iter().take(4).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index));
                }
                self.unify_or_error(core.uint_size, argument_types[4], arg_span(4));
                self.unify_or_error(core.uint_size, argument_types[5], arg_span(5));
                core.bool_
            }
            "app_data_journal_compact_if_frames_over_durable" => {
                for (index, actual) in argument_types.iter().take(4).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index));
                }
                self.unify_or_error(core.uint_size, argument_types[4], arg_span(4));
                core.bool_
            }
            "app_data_journal_retain_last_durable" => {
                for (index, actual) in argument_types.iter().take(4).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index));
                }
                self.unify_or_error(core.uint_size, argument_types[4], arg_span(4));
                core.bool_
            }
            "app_data_journal_recover_frame_durable" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index));
                }
                self.unify_or_error(core.uint_size, argument_types[3], arg_span(3));
                core.bool_
            }
            "app_data_journal_count_frames_durable" => {
                self.unify_or_error(core.string, argument_types[0], arg_span(0));
                self.unify_or_error(core.string, argument_types[1], arg_span(1));
                core.uint_size
            }
            "app_data_journal_stats_durable" => {
                self.unify_or_error(core.string, argument_types[0], arg_span(0));
                self.unify_or_error(core.string, argument_types[1], arg_span(1));
                self.unify_or_error(write_slice_uint_size, argument_types[2], arg_span(2));
                core.bool_
            }
            "app_data_journal_maintenance_plan_durable" => {
                self.unify_or_error(core.string, argument_types[0], arg_span(0));
                self.unify_or_error(core.string, argument_types[1], arg_span(1));
                self.unify_or_error(core.uint_size, argument_types[2], arg_span(2));
                self.unify_or_error(core.uint_size, argument_types[3], arg_span(3));
                self.unify_or_error(write_slice_uint_size, argument_types[4], arg_span(4));
                core.bool_
            }
            "app_data_journal_maintenance_retry_durable" => {
                for index in 0..4 {
                    self.unify_or_error(core.string, argument_types[index], arg_span(index));
                }
                for index in 4..8 {
                    self.unify_or_error(core.uint_size, argument_types[index], arg_span(index));
                }
                core.bool_
            }
            "app_data_journal_read_frame_exact_durable" => {
                self.unify_or_error(core.string, argument_types[0], arg_span(0));
                self.unify_or_error(core.string, argument_types[1], arg_span(1));
                self.unify_or_error(core.uint_size, argument_types[2], arg_span(2));
                self.unify_or_error(write_byte_slice, argument_types[3], arg_span(3));
                self.unify_or_error(write_slice_uint_size, argument_types[4], arg_span(4));
                core.bool_
            }
            "app_data_journal_read_latest_frame_exact_durable" => {
                self.unify_or_error(core.string, argument_types[0], arg_span(0));
                self.unify_or_error(core.string, argument_types[1], arg_span(1));
                self.unify_or_error(write_byte_slice, argument_types[2], arg_span(2));
                self.unify_or_error(write_slice_uint_size, argument_types[3], arg_span(3));
                core.bool_
            }
            "app_data_journal_frame_length_durable" => {
                self.unify_or_error(core.string, argument_types[0], arg_span(0));
                self.unify_or_error(core.string, argument_types[1], arg_span(1));
                self.unify_or_error(core.uint_size, argument_types[2], arg_span(2));
                core.uint_size
            }
            "app_data_journal_frame_span_durable" => {
                self.unify_or_error(core.string, argument_types[0], arg_span(0));
                self.unify_or_error(core.string, argument_types[1], arg_span(1));
                self.unify_or_error(core.uint_size, argument_types[2], arg_span(2));
                self.unify_or_error(write_slice_uint_size, argument_types[3], arg_span(3));
                core.bool_
            }
            "app_data_journal_build_index_durable" => {
                for index in 0..4 {
                    self.unify_or_error(core.string, argument_types[index], arg_span(index));
                }
                core.bool_
            }
            "app_data_journal_index_lookup_durable" => {
                for index in 0..3 {
                    self.unify_or_error(core.string, argument_types[index], arg_span(index));
                }
                self.unify_or_error(core.uint_size, argument_types[3], arg_span(3));
                self.unify_or_error(write_slice_uint_size, argument_types[4], arg_span(4));
                core.bool_
            }
            "app_data_journal_index_export_csv_durable" => {
                self.unify_or_error(core.string, argument_types[0], arg_span(0));
                self.unify_or_error(core.string, argument_types[1], arg_span(1));
                self.unify_or_error(core.string, argument_types[2], arg_span(2));
                self.unify_or_error(write_byte_slice, argument_types[3], arg_span(3));
                core.uint_size
            }
            "app_data_journal_index_export_csv_file_durable" => {
                for index in 0..5 {
                    self.unify_or_error(core.string, argument_types[index], arg_span(index));
                }
                core.bool_
            }
            "app_data_journal_index_range_durable" => {
                for index in 0..3 {
                    self.unify_or_error(core.string, argument_types[index], arg_span(index));
                }
                self.unify_or_error(core.uint_size, argument_types[3], arg_span(3));
                self.unify_or_error(core.uint_size, argument_types[4], arg_span(4));
                self.unify_or_error(write_slice_uint_size, argument_types[5], arg_span(5));
                core.uint_size
            }
            "app_data_journal_index_read_page_durable" => {
                for index in 0..3 {
                    self.unify_or_error(core.string, argument_types[index], arg_span(index));
                }
                self.unify_or_error(core.uint_size, argument_types[3], arg_span(3));
                self.unify_or_error(core.uint_size, argument_types[4], arg_span(4));
                self.unify_or_error(write_byte_slice, argument_types[5], arg_span(5));
                self.unify_or_error(write_slice_uint_size, argument_types[6], arg_span(6));
                core.uint_size
            }
            "app_list_clear" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.unit
            }
            "app_list_count" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.int32
            }
            "app_list_push_text" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                core.bool_
            }
            "app_list_push_text_bytes" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(2));
                }
                core.bool_
            }
            "app_list_export_csv" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "app_list_sort_text" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.bool_, *actual, arg_span(1));
                }
                core.bool_
            }
            "app_list_sort_callback" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    let comparator = self.types.intern(TypeKind::Function {
                        parameters: vec![core.int32, core.int32, core.int32].into_boxed_slice(),
                        result: core.int32,
                    });
                    self.unify_or_error(comparator, *actual, arg_span(1));
                }
                core.bool_
            }
            "app_list_find_text" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.int32, *actual, arg_span(2));
                }
                core.int32
            }
            "app_list_filter_text" | "app_list_filter_text_ex" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.string, *actual, arg_span(2));
                }
                if name == "app_list_filter_text_ex"
                    && let Some(actual) = argument_types.get(3)
                {
                    self.unify_or_error(core.int32, *actual, arg_span(3));
                }
                core.bool_
            }
            "app_list_filter_text_ex_bytes" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(3));
                }
                if let Some(actual) = argument_types.get(4) {
                    self.unify_or_error(core.int32, *actual, arg_span(4));
                }
                core.bool_
            }
            "app_list_filter_callback" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    let predicate = self.types.intern(TypeKind::Function {
                        parameters: vec![core.int32, core.int32].into_boxed_slice(),
                        result: core.bool_,
                    });
                    self.unify_or_error(predicate, *actual, arg_span(2));
                }
                core.bool_
            }
            "app_list_page" => {
                for (index, actual) in argument_types.iter().take(4).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                core.bool_
            }
            "app_list_read_text" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(2));
                }
                core.uint_size
            }
            "app_list_read_text_exact" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(write_slice_uint_size, *actual, arg_span(3));
                }
                core.bool_
            }
            "app_list_set_text" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.string, *actual, arg_span(2));
                }
                core.bool_
            }
            "app_list_set_text_bytes" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(3));
                }
                core.bool_
            }
            "app_list_remove" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                core.bool_
            }
            "app_list_save" | "app_list_load" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                core.bool_
            }
            "app_list_save_atomic" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                for (index, actual) in argument_types.iter().skip(1).take(2).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index + 1));
                }
                core.bool_
            }
            "app_table_clear" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.unit
            }
            "app_table_tx_begin" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.bool_
            }
            "app_table_tx_begin_all"
            | "app_table_tx_commit"
            | "app_table_tx_commit_all"
            | "app_table_tx_rollback"
            | "app_table_tx_rollback_all"
            | "app_table_migration_commit"
            | "app_table_migration_rollback" => core.bool_,
            "app_table_migration_begin" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                core.bool_
            }
            "app_table_migration_rename_column" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index));
                }
                core.bool_
            }
            "app_table_migration_set_column_type" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                core.bool_
            }
            "app_table_set_column_name" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.string, *actual, arg_span(2));
                }
                core.bool_
            }
            "app_table_read_column_name" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(2));
                }
                core.uint_size
            }
            "app_table_read_column_name_exact" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(write_slice_uint_size, *actual, arg_span(3));
                }
                core.bool_
            }
            "app_table_find_column" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                core.int32
            }
            "app_table_schema_version" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.int32
            }
            "app_table_set_schema_version" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                core.bool_
            }
            "app_table_set_named_cell" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                for (index, actual) in argument_types.iter().skip(2).take(2).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index + 2));
                }
                core.bool_
            }
            "app_table_read_named_cell" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.string, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(3));
                }
                core.uint_size
            }
            "app_table_read_named_cell_exact" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.string, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(3));
                }
                if let Some(actual) = argument_types.get(4) {
                    self.unify_or_error(write_slice_uint_size, *actual, arg_span(4));
                }
                core.bool_
            }
            "app_table_set_named_int"
            | "app_table_set_named_uint"
            | "app_table_set_named_float"
            | "app_table_set_named_bool" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.string, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    let expected = match name {
                        "app_table_set_named_int" => core.int64,
                        "app_table_set_named_uint" => core.uint64,
                        "app_table_set_named_float" => core.float64,
                        _ => core.bool_,
                    };
                    self.unify_or_error(expected, *actual, arg_span(3));
                }
                core.bool_
            }
            "app_table_read_named_int"
            | "app_table_read_named_uint"
            | "app_table_read_named_float"
            | "app_table_read_named_bool" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.string, *actual, arg_span(2));
                }
                match name {
                    "app_table_read_named_int" => core.int64,
                    "app_table_read_named_uint" => core.uint64,
                    "app_table_read_named_float" => core.float64,
                    _ => core.bool_,
                }
            }
            "app_table_read_named_int_exact"
            | "app_table_read_named_uint_exact"
            | "app_table_read_named_float_exact"
            | "app_table_read_named_bool_exact" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.string, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    let expected = match name {
                        "app_table_read_named_int_exact" => write_slice_int64,
                        "app_table_read_named_uint_exact" => write_slice_uint64,
                        "app_table_read_named_float_exact" => write_slice_float64,
                        _ => write_slice_bool,
                    };
                    self.unify_or_error(expected, *actual, arg_span(3));
                }
                core.bool_
            }
            "app_table_set_column_type" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.int32, *actual, arg_span(2));
                }
                core.bool_
            }
            "app_table_column_type" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                core.int32
            }
            "app_table_validate" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.bool_
            }
            "app_table_row_count" | "app_table_append_row" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if name == "app_table_row_count" {
                    core.int32
                } else {
                    core.bool_
                }
            }
            "app_table_remove_row" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                core.bool_
            }
            "app_table_set_cell" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.int32, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.string, *actual, arg_span(3));
                }
                core.bool_
            }
            "app_table_set_cell_bytes" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(3));
                }
                core.bool_
            }
            "app_table_set_cell_bytes_ex" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(3));
                }
                if let Some(actual) = argument_types.get(4) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(4));
                }
                core.bool_
            }
            "app_table_read_cell" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.int32, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(3));
                }
                core.uint_size
            }
            "app_table_read_cell_exact" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(3));
                }
                if let Some(actual) = argument_types.get(4) {
                    self.unify_or_error(write_slice_uint_size, *actual, arg_span(4));
                }
                core.bool_
            }
            "app_table_set_int" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.int64, *actual, arg_span(3));
                }
                core.bool_
            }
            "app_table_set_uint" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.uint64, *actual, arg_span(3));
                }
                core.bool_
            }
            "app_table_set_bool" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.bool_, *actual, arg_span(3));
                }
                core.bool_
            }
            "app_table_set_float" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.float64, *actual, arg_span(3));
                }
                core.bool_
            }
            "app_table_read_int" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                core.int64
            }
            "app_table_read_uint" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                core.uint64
            }
            "app_table_read_bool" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                core.bool_
            }
            "app_table_read_float" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                core.float64
            }
            "app_table_read_int_exact"
            | "app_table_read_uint_exact"
            | "app_table_read_float_exact"
            | "app_table_read_bool_exact" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(3) {
                    let expected = match name {
                        "app_table_read_int_exact" => write_slice_int64,
                        "app_table_read_uint_exact" => write_slice_uint64,
                        "app_table_read_float_exact" => write_slice_float64,
                        _ => write_slice_bool,
                    };
                    self.unify_or_error(expected, *actual, arg_span(3));
                }
                core.bool_
            }
            "app_table_sort_text"
            | "app_table_sort_int"
            | "app_table_sort_uint"
            | "app_table_sort_float"
            | "app_table_sort_bool" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.bool_, *actual, arg_span(2));
                }
                core.bool_
            }
            "app_table_sort_callback" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    let comparator = self.types.intern(TypeKind::Function {
                        parameters: vec![core.int32, core.int32, core.int32].into_boxed_slice(),
                        result: core.int32,
                    });
                    self.unify_or_error(comparator, *actual, arg_span(1));
                }
                core.bool_
            }
            "app_table_page" => {
                for (index, actual) in argument_types.iter().take(4).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                core.bool_
            }
            "app_table_find_text" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.string, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.int32, *actual, arg_span(3));
                }
                core.int32
            }
            "app_table_find_int" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.int64, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.int32, *actual, arg_span(3));
                }
                core.int32
            }
            "app_table_find_uint" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.uint64, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.int32, *actual, arg_span(3));
                }
                core.int32
            }
            "app_table_find_float" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.float64, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.int32, *actual, arg_span(3));
                }
                core.int32
            }
            "app_table_find_bool" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.bool_, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.int32, *actual, arg_span(3));
                }
                core.int32
            }
            "app_table_remove_text" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.string, *actual, arg_span(2));
                }
                core.bool_
            }
            "app_table_remove_int" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.int64, *actual, arg_span(2));
                }
                core.bool_
            }
            "app_table_remove_uint" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.uint64, *actual, arg_span(2));
                }
                core.bool_
            }
            "app_table_remove_float" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.float64, *actual, arg_span(2));
                }
                core.bool_
            }
            "app_table_remove_bool" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.bool_, *actual, arg_span(2));
                }
                core.bool_
            }
            "app_table_upsert_text" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.string, *actual, arg_span(2));
                }
                core.int32
            }
            "app_table_upsert_int" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.int64, *actual, arg_span(2));
                }
                core.int32
            }
            "app_table_upsert_uint" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.uint64, *actual, arg_span(2));
                }
                core.int32
            }
            "app_table_upsert_float" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.float64, *actual, arg_span(2));
                }
                core.int32
            }
            "app_table_upsert_bool" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.bool_, *actual, arg_span(2));
                }
                core.int32
            }
            "app_table_index_build"
            | "app_table_index_build_int"
            | "app_table_index_build_uint"
            | "app_table_index_build_float"
            | "app_table_index_build_bool" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                core.bool_
            }
            "app_table_index_build_pair" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                core.bool_
            }
            "app_table_index_clear" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.bool_
            }
            "app_table_index_find_text" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.string, *actual, arg_span(2));
                }
                core.int32
            }
            "app_table_index_find_pair_text" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                for (index, actual) in argument_types.iter().skip(3).take(3).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index + 3));
                }
                core.int32
            }
            "app_table_index_collect_int_range" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                for (index, actual) in argument_types.iter().skip(2).take(2).enumerate() {
                    self.unify_or_error(core.int64, *actual, arg_span(index + 2));
                }
                if let Some(actual) = argument_types.get(4) {
                    self.unify_or_error(write_slice_int32, *actual, arg_span(4));
                }
                core.uint_size
            }
            "app_table_index_collect_uint_range" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                for (index, actual) in argument_types.iter().skip(2).take(2).enumerate() {
                    self.unify_or_error(core.uint64, *actual, arg_span(index + 2));
                }
                if let Some(actual) = argument_types.get(4) {
                    self.unify_or_error(write_slice_int32, *actual, arg_span(4));
                }
                core.uint_size
            }
            "app_table_index_collect_float_range" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                for (index, actual) in argument_types.iter().skip(2).take(2).enumerate() {
                    self.unify_or_error(core.float64, *actual, arg_span(index + 2));
                }
                if let Some(actual) = argument_types.get(4) {
                    self.unify_or_error(write_slice_int32, *actual, arg_span(4));
                }
                core.uint_size
            }
            "app_table_index_find_int" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.int64, *actual, arg_span(2));
                }
                core.int32
            }
            "app_table_index_find_uint" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.uint64, *actual, arg_span(2));
                }
                core.int32
            }
            "app_table_index_find_float" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.float64, *actual, arg_span(2));
                }
                core.int32
            }
            "app_table_index_find_bool" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.bool_, *actual, arg_span(2));
                }
                core.int32
            }
            "app_table_index_is_valid" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                core.bool_
            }
            "app_table_filter_text" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.int32, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.string, *actual, arg_span(3));
                }
                core.bool_
            }
            "app_table_filter_int" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.int64, *actual, arg_span(3));
                }
                core.bool_
            }
            "app_table_filter_uint" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.uint64, *actual, arg_span(3));
                }
                core.bool_
            }
            "app_table_filter_float" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.float64, *actual, arg_span(3));
                }
                core.bool_
            }
            "app_table_filter_bool" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.bool_, *actual, arg_span(3));
                }
                core.bool_
            }
            "app_table_filter_callback" => {
                for (index, actual) in argument_types.iter().take(2).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(2) {
                    let predicate = self.types.intern(TypeKind::Function {
                        parameters: vec![core.int32, core.int32].into_boxed_slice(),
                        result: core.bool_,
                    });
                    self.unify_or_error(predicate, *actual, arg_span(2));
                }
                core.bool_
            }
            "app_table_filter_text_ex" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.string, *actual, arg_span(3));
                }
                if let Some(actual) = argument_types.get(4) {
                    self.unify_or_error(core.int32, *actual, arg_span(4));
                }
                core.bool_
            }
            "app_table_filter_text_ex_bytes" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.int32, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(3));
                }
                if let Some(actual) = argument_types.get(4) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(4));
                }
                if let Some(actual) = argument_types.get(5) {
                    self.unify_or_error(core.int32, *actual, arg_span(5));
                }
                core.bool_
            }
            "app_table_export_csv" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "app_table_import_csv" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(2));
                }
                core.bool_
            }
            "app_table_save"
            | "app_table_load"
            | "app_table_save_schema"
            | "app_table_load_schema"
            | "app_table_save_schema_full"
            | "app_table_load_schema_full" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                core.bool_
            }
            "app_table_load_schema_full_if_version" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.int32, *actual, arg_span(2));
                }
                core.bool_
            }
            "app_table_save_atomic"
            | "app_table_save_schema_atomic"
            | "app_table_save_schema_full_atomic" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                for (index, actual) in argument_types.iter().skip(1).take(2).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index + 1));
                }
                core.bool_
            }
            "http_request_write_prefix" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(3));
                }
                if let Some(actual) = argument_types.get(4) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(4));
                }
                if let Some(actual) = argument_types.get(5) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(5));
                }
                core.uint_size
            }
            "http_request_write" => {
                for (index, actual) in argument_types.iter().take(3).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(3));
                }
                if let Some(actual) = argument_types.get(4) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(4));
                }
                core.uint_size
            }
            "http_request_write_header" => {
                for (index, actual) in argument_types.iter().take(5).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(5) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(5));
                }
                if let Some(actual) = argument_types.get(6) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(6));
                }
                core.uint_size
            }
            "http_request_write_header_block" => {
                for (index, actual) in argument_types.iter().take(4).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index));
                }
                if let Some(actual) = argument_types.get(4) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(4));
                }
                if let Some(actual) = argument_types.get(5) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(5));
                }
                core.uint_size
            }
            "http_response_write" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint16, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(3));
                }
                core.uint_size
            }
            "http_response_write_ex" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint16, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.bool_, *actual, arg_span(3));
                }
                if let Some(actual) = argument_types.get(4) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(4));
                }
                core.uint_size
            }
            "http_response_write_header" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint16, *actual, arg_span(0));
                }
                for index in 1..=3 {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(core.string, *actual, arg_span(index));
                    }
                }
                if let Some(actual) = argument_types.get(4) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(4));
                }
                if let Some(actual) = argument_types.get(5) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(5));
                }
                core.uint_size
            }
            "http_response_write_header_ex" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint16, *actual, arg_span(0));
                }
                for index in 1..=3 {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(core.string, *actual, arg_span(index));
                    }
                }
                if let Some(actual) = argument_types.get(4) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(4));
                }
                if let Some(actual) = argument_types.get(5) {
                    self.unify_or_error(core.bool_, *actual, arg_span(5));
                }
                if let Some(actual) = argument_types.get(6) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(6));
                }
                core.uint_size
            }
            "http_response_write_cookie" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint16, *actual, arg_span(0));
                }
                for index in 1..=4 {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(core.string, *actual, arg_span(index));
                    }
                }
                if let Some(actual) = argument_types.get(5) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(5));
                }
                if let Some(actual) = argument_types.get(6) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(6));
                }
                core.uint_size
            }
            "http_response_write_cookie_ex" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint16, *actual, arg_span(0));
                }
                for index in 1..=4 {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(core.string, *actual, arg_span(index));
                    }
                }
                if let Some(actual) = argument_types.get(5) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(5));
                }
                if let Some(actual) = argument_types.get(6) {
                    self.unify_or_error(core.bool_, *actual, arg_span(6));
                }
                if let Some(actual) = argument_types.get(7) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(7));
                }
                core.uint_size
            }
            "http_response_write_header_block" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint16, *actual, arg_span(0));
                }
                for index in 1..=2 {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(core.string, *actual, arg_span(index));
                    }
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(3));
                }
                if let Some(actual) = argument_types.get(4) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(4));
                }
                core.uint_size
            }
            "http_response_write_header_block_ex" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint16, *actual, arg_span(0));
                }
                for index in 1..=2 {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(core.string, *actual, arg_span(index));
                    }
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(3));
                }
                if let Some(actual) = argument_types.get(4) {
                    self.unify_or_error(core.bool_, *actual, arg_span(4));
                }
                if let Some(actual) = argument_types.get(5) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(5));
                }
                core.uint_size
            }
            "http_response_status" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                core.uint16
            }
            "http_response_status_prefix" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                core.uint16
            }
            "http_response_header" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(2));
                }
                core.uint_size
            }
            "http_response_header_prefix" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.string, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(3));
                }
                core.uint_size
            }
            "http_response_header_exact" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(write_slice_uint_size, *actual, arg_span(3));
                }
                core.bool_
            }
            "http_response_body" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "http_response_body_exact" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_slice_uint_size, *actual, arg_span(2));
                }
                core.bool_
            }
            "http_response_body_chunked_exact" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_slice_uint_size, *actual, arg_span(2));
                }
                core.bool_
            }
            "http_request_body_chunked_exact" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_slice_uint_size, *actual, arg_span(2));
                }
                core.bool_
            }
            "http_response_body_prefix" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(2));
                }
                core.uint_size
            }
            "http_request_is_complete" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                core.bool_
            }
            "http_request_is_complete_prefix" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                core.bool_
            }
            "http_request_frame_length_prefix" | "http_request_chunked_frame_length_prefix" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                core.uint_size
            }
            "http_request_consume_prefix" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(2));
                }
                core.uint_size
            }
            "http_request_append" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(3));
                }
                core.uint_size
            }
            "http_request_keep_alive" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                core.bool_
            }
            "http_request_method" | "http_request_target" | "http_request_body" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "http_request_header" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(2));
                }
                core.uint_size
            }
            "http_query_param" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(2));
                }
                core.uint_size
            }
            "http_query_param_exact" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(write_slice_uint_size, *actual, arg_span(3));
                }
                core.bool_
            }
            "http_route_match" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                for (index, actual) in argument_types.iter().skip(1).take(2).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index + 1));
                }
                core.bool_
            }
            "http_route_match_prefix" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                for (index, actual) in argument_types.iter().skip(2).take(2).enumerate() {
                    self.unify_or_error(core.string, *actual, arg_span(index + 2));
                }
                core.bool_
            }
            "http_router_clear" => core.unit,
            "http_router_add" | "http_router_add_prefix" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.uint16, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.string, *actual, arg_span(3));
                }
                if let Some(actual) = argument_types.get(4) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(4));
                }
                core.bool_
            }
            "http_router_add_exact" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.uint16, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.string, *actual, arg_span(3));
                }
                if let Some(actual) = argument_types.get(4) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(4));
                }
                if let Some(actual) = argument_types.get(5) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(5));
                }
                core.bool_
            }
            "http_router_remove" | "http_router_remove_prefix" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                core.bool_
            }
            "http_router_respond" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "http_router_respond_prefix" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(2));
                }
                core.uint_size
            }
            "http_router_count" => core.uint_size,
            "http_session_open" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                for (index, actual) in argument_types.iter().skip(1).take(3).enumerate() {
                    self.unify_or_error(core.uint32, *actual, arg_span(index + 1));
                }
                core.uint_size
            }
            "http_session_open_tls" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                for (index, actual) in argument_types.iter().skip(1).take(3).enumerate() {
                    self.unify_or_error(core.uint32, *actual, arg_span(index + 1));
                }
                if let Some(actual) = argument_types.get(4) {
                    self.unify_or_error(core.string, *actual, arg_span(4));
                }
                if let Some(actual) = argument_types.get(5) {
                    self.unify_or_error(core.string, *actual, arg_span(5));
                }
                core.uint_size
            }
            "http_session_step" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint32, *actual, arg_span(1));
                }
                core.uint32
            }
            "http_session_close" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                core.bool_
            }
            "net_tls_open_client" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.bool_, *actual, arg_span(2));
                }
                core.uint_size
            }
            "net_tls_open_server" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.string, *actual, arg_span(2));
                }
                core.uint_size
            }
            "net_tls_step" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint32, *actual, arg_span(1));
                }
                core.uint32
            }
            "net_tls_state" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                core.uint32
            }
            "net_tls_error" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                core.int32
            }
            "net_tls_send" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "net_tls_receive" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "net_tls_close" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                core.bool_
            }
            "net_tcp_connect" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint16, *actual, arg_span(1));
                }
                core.uint_size
            }
            "net_tcp_connect_dns" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint16, *actual, arg_span(1));
                }
                core.uint_size
            }
            "net_tcp_listen" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint16, *actual, arg_span(0));
                }
                core.uint_size
            }
            "net_tcp_accept" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                core.uint_size
            }
            "net_tcp_send" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "net_tcp_send_prefix" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(2));
                }
                core.uint_size
            }
            "net_tcp_receive" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "net_reactor_submit_receive_buffer" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(3));
                }
                core.uint_size
            }
            "net_reactor_submit_send_buffer" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(3));
                }
                core.uint_size
            }
            "net_reactor_submit_send_buffer_prefix" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(read_byte_slice, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(3));
                }
                if let Some(actual) = argument_types.get(4) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(4));
                }
                core.uint_size
            }
            "net_socket_set_timeout" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint32, *actual, arg_span(1));
                }
                core.bool_
            }
            "net_socket_close" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                core.bool_
            }
            "net_reactor_open" => {
                for (index, actual) in argument_types.iter().enumerate() {
                    self.unify_or_error(core.uint32, *actual, arg_span(index));
                }
                core.uint_size
            }
            "net_reactor_watch" => {
                for index in [0usize, 1usize, 3usize] {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(core.uint_size, *actual, arg_span(index));
                    }
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.uint32, *actual, arg_span(2));
                }
                core.bool_
            }
            "net_reactor_unwatch" => {
                for (index, actual) in argument_types.iter().enumerate() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(index));
                }
                core.bool_
            }
            "net_reactor_poll" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint32, *actual, arg_span(1));
                }
                core.uint32
            }
            "net_reactor_event_socket" | "net_reactor_event_user" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint32, *actual, arg_span(1));
                }
                core.uint_size
            }
            "net_reactor_event_flags" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint32, *actual, arg_span(1));
                }
                core.uint32
            }
            "net_reactor_event_bytes" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint32, *actual, arg_span(1));
                }
                core.uint_size
            }
            "net_reactor_error" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                core.int32
            }
            "net_reactor_close" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                core.bool_
            }
            "net_reactor_submit_accept"
            | "net_reactor_submit_receive"
            | "net_reactor_submit_send" => {
                for (index, actual) in argument_types.iter().enumerate() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(index));
                }
                core.uint_size
            }
            "net_reactor_submit_connect" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.uint16, *actual, arg_span(2));
                }
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(core.uint_size, *actual, arg_span(3));
                }
                core.uint_size
            }
            "net_reactor_cancel" => {
                for (index, actual) in argument_types.iter().enumerate() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(index));
                }
                core.bool_
            }
            "net_reactor_event_operation" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.uint_size, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint32, *actual, arg_span(1));
                }
                core.uint_size
            }
            "json_object_field_int" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int64, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(2));
                }
                core.uint_size
            }
            "json_object_field_uint" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.uint64, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(2));
                }
                core.uint_size
            }
            "json_object_field_float" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.float64, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(2));
                }
                core.uint_size
            }
            "json_object_field_bool" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.bool_, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(2));
                }
                core.uint_size
            }
            "ui_app_begin" => {
                for (index, expected) in [core.string, core.int32, core.int32, core.uint32]
                    .iter()
                    .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.int32
            }
            "ui_app_on_resize" | "ui_app_on_close" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.unit
            }
            "ui_app_window_width" | "ui_app_window_height" => core.int32,
            "ui_app_window_set_constraints" => {
                for (index, expected) in [core.int32, core.int32, core.int32, core.int32]
                    .iter()
                    .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.bool_
            }
            "ui_app_window_min_width"
            | "ui_app_window_min_height"
            | "ui_app_window_max_width"
            | "ui_app_window_max_height" => core.int32,
            "ui_app_bind_app_state" => {
                for (index, expected) in [core.int32, core.string].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.bool_
            }
            "ui_app_refresh_app_state" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.unit
            }
            "ui_app_panel" => {
                for (index, expected) in [
                    core.int32,
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.int32
            }
            "ui_app_row" => {
                for (index, expected) in [
                    core.int32, core.int32, core.int32, core.int32, core.int32, core.int32,
                    core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.int32
            }
            "ui_app_top_bar" => {
                for (index, expected) in [
                    core.int32,
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.int32
            }
            "ui_app_menu" => {
                for (index, expected) in [
                    core.int32,
                    core.string,
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.uint32,
                    core.int32,
                    core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.int32
            }
            "ui_app_menu_item" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(core.int32, *actual, arg_span(2));
                }
                core.bool_
            }
            "ui_app_tooltip" => {
                for (index, expected) in [
                    core.int32,
                    core.string,
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.uint32,
                    core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.bool_
            }
            "ui_app_status" => {
                for (index, expected) in [
                    core.int32,
                    core.string,
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.uint32,
                    core.int32,
                    core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.int32
            }
            "ui_app_label" => {
                for (index, expected) in [
                    core.int32,
                    core.string,
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.uint32,
                    core.int32,
                    core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.int32
            }
            "ui_app_button" => {
                for (index, expected) in [
                    core.int32,
                    core.string,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.uint32,
                    core.int32,
                    core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.int32
            }
            "ui_app_text_input" => {
                for (index, expected) in [
                    core.int32,
                    core.string,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.uint32,
                    core.int32,
                    core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.int32
            }
            "ui_app_checkbox" => {
                for (index, expected) in [
                    core.int32,
                    core.string,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.uint32,
                    core.int32,
                    core.int32,
                    core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.int32
            }
            "ui_app_select" => {
                for (index, expected) in [
                    core.int32,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.uint32,
                    core.int32,
                    core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.int32
            }
            "ui_app_select_option" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                core.bool_
            }
            "ui_app_select_index" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.int32
            }
            "ui_app_select_set_index" => {
                for (index, expected) in [core.int32, core.int32].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.bool_
            }
            "ui_app_list" => {
                for (index, expected) in [
                    core.int32,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.uint32,
                    core.int32,
                    core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.int32
            }
            "ui_app_list_item" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                core.bool_
            }
            "ui_app_list_bind_app" => {
                for (index, expected) in [core.int32, core.int32].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.bool_
            }
            "ui_app_list_clear" | "ui_app_list_refresh" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.bool_
            }
            "ui_app_list_count" | "ui_app_list_index" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.int32
            }
            "ui_app_list_read_item" => {
                for (index, expected) in [core.int32, core.int32, write_byte_slice]
                    .iter()
                    .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.uint_size
            }
            "ui_app_list_set_index" => {
                for (index, expected) in [core.int32, core.int32].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.bool_
            }
            "ui_app_table" => {
                for (index, expected) in [
                    core.int32,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.uint32,
                    core.int32,
                    core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.int32
            }
            "ui_app_table_column" => {
                for (index, expected) in [core.int32, core.int32, core.string, core.int32]
                    .iter()
                    .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.bool_
            }
            "ui_app_table_cell" => {
                for (index, expected) in [core.int32, core.int32, core.int32, core.string]
                    .iter()
                    .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.bool_
            }
            "ui_app_table_read_cell" => {
                for (index, expected) in [core.int32, core.int32, core.int32, write_byte_slice]
                    .iter()
                    .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.uint_size
            }
            "ui_app_table_bind_app" => {
                for (index, expected) in [core.int32, core.int32, core.int32].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.bool_
            }
            "ui_app_table_refresh" | "ui_app_table_clear" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.bool_
            }
            "ui_app_table_row_count" | "ui_app_table_selected_row" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.int32
            }
            "ui_app_table_set_selected_row" => {
                for (index, expected) in [core.int32, core.int32].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.bool_
            }
            "ui_app_table_sort_text"
            | "ui_app_table_sort_int"
            | "ui_app_table_sort_uint"
            | "ui_app_table_sort_float"
            | "ui_app_table_sort_bool" => {
                for (index, expected) in [core.int32, core.int32, core.bool_].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.bool_
            }
            "ui_app_table_filter_text" => {
                for (index, expected) in [core.int32, core.int32, core.int32, core.string]
                    .iter()
                    .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.bool_
            }
            "ui_app_table_filter_text_ex" => {
                for (index, expected) in
                    [core.int32, core.int32, core.int32, core.string, core.int32]
                        .iter()
                        .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.bool_
            }
            "ui_app_table_filter_int"
            | "ui_app_table_filter_uint"
            | "ui_app_table_filter_float"
            | "ui_app_table_filter_bool" => {
                for (index, expected) in [core.int32, core.int32, core.int32].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                let expected = match name {
                    "ui_app_table_filter_int" => core.int64,
                    "ui_app_table_filter_uint" => core.uint64,
                    "ui_app_table_filter_float" => core.float64,
                    _ => core.bool_,
                };
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(expected, *actual, arg_span(3));
                }
                core.bool_
            }
            "ui_app_end" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.bool_
            }
            "ui_app_run" => core.int32,
            "ui_window" => {
                for (index, expected) in [core.string, core.int32, core.int32, core.uint32]
                    .iter()
                    .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_top_bar" => {
                for (index, expected) in [core.int32, core.uint32].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_label" | "ui_status" | "ui_text" => {
                for (index, expected) in [
                    core.string,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.uint32,
                    core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_scroll_panel" => {
                for (index, expected) in [
                    core.string,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.uint32,
                    core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_image" => {
                for (index, expected) in
                    [core.string, core.int32, core.int32, core.int32, core.int32]
                        .iter()
                        .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_button" | "ui_toggle_button" | "ui_menu_item" | "ui_icon_button" => {
                for (index, expected) in [
                    core.string,
                    core.string,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.uint32,
                    core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_menu" => {
                for (index, expected) in [
                    core.string,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.uint32,
                    core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_event_button" => {
                for (index, expected) in [
                    core.string,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.uint32,
                    core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_checkbox" | "ui_switch" | "ui_text_input" => {
                for (index, expected) in [
                    core.string,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.uint32,
                    core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_column" => {
                for (index, expected) in [
                    core.int32, core.int32, core.int32, core.int32, core.int32, core.int32,
                    core.int32, core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_row" => {
                for (index, expected) in [
                    core.int32, core.int32, core.int32, core.int32, core.int32, core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_panel" => {
                for (index, expected) in [
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_layout_label" | "ui_layout_status" => {
                for (index, expected) in [
                    core.string,
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.uint32,
                    core.int32,
                    core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_layout_event_button" => {
                for (index, expected) in [
                    core.string,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.uint32,
                    core.int32,
                    core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_layout_end" => core.unit,
            "ui_close_button" | "ui_disabled_button" => {
                for (index, expected) in [
                    core.string,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.uint32,
                    core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_set_button_enabled" => {
                for (index, expected) in [core.int32, core.int32].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_set_button_text" => {
                for (index, expected) in [core.int32, core.string].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_checked" | "ui_select_index" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.int32
            }
            "ui_list_clear" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.unit
            }
            "ui_list_count" | "ui_list_index" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.int32
            }
            "ui_set_checked" | "ui_set_input_enabled" | "ui_select_set_index" => {
                for (index, expected) in [core.int32, core.int32].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_list_set_index" => {
                for (index, expected) in [core.int32, core.int32].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_set_input_text" | "ui_select_option" => {
                for (index, expected) in [core.int32, core.string].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_select" => {
                for (index, expected) in [
                    core.int32,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.uint32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_list" => {
                for (index, expected) in [
                    core.int32,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.uint32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_list_item" => {
                for (index, expected) in [core.int32, core.string].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_list_read_item" => {
                for (index, expected) in [core.int32, core.int32, write_byte_slice]
                    .iter()
                    .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.uint_size
            }
            "ui_list_set_item" => {
                for (index, expected) in [core.int32, core.int32, core.string].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_list_bind_app" => {
                for (index, expected) in [core.int32, core.int32].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_list_refresh_app" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.unit
            }
            "ui_table" => {
                for (index, expected) in [
                    core.int32,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.uint32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_table_column" => {
                for (index, expected) in [core.int32, core.int32, core.string, core.int32]
                    .iter()
                    .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_table_cell" => {
                for (index, expected) in [core.int32, core.int32, core.int32, core.string]
                    .iter()
                    .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_table_read_cell" => {
                for (index, expected) in [core.int32, core.int32, core.int32, write_byte_slice]
                    .iter()
                    .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.uint_size
            }
            "ui_table_bind_app" => {
                for (index, expected) in [core.int32, core.int32, core.int32].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_table_refresh_app" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.unit
            }
            "ui_refresh_bindings" => core.unit,
            "ui_table_clear" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.unit
            }
            "ui_table_row_count" | "ui_table_selected_row" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.int32
            }
            "ui_table_set_selected_row" => {
                for (index, expected) in [core.int32, core.int32].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_table_sort_text"
            | "ui_table_sort_int"
            | "ui_table_sort_uint"
            | "ui_table_sort_float"
            | "ui_table_sort_bool" => {
                for (index, expected) in [core.int32, core.int32, core.bool_].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_table_filter_text" => {
                for (index, expected) in [core.int32, core.int32, core.int32, core.string]
                    .iter()
                    .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_table_filter_text_ex" => {
                for (index, expected) in
                    [core.int32, core.int32, core.int32, core.string, core.int32]
                        .iter()
                        .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_table_filter_int"
            | "ui_table_filter_uint"
            | "ui_table_filter_float"
            | "ui_table_filter_bool" => {
                for (index, expected) in [core.int32, core.int32, core.int32].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                let expected = match name {
                    "ui_table_filter_int" => core.int64,
                    "ui_table_filter_uint" => core.uint64,
                    "ui_table_filter_float" => core.float64,
                    _ => core.bool_,
                };
                if let Some(actual) = argument_types.get(3) {
                    self.unify_or_error(expected, *actual, arg_span(3));
                }
                core.unit
            }
            "ui_menu_option" => {
                for (index, expected) in [core.int32, core.string, core.int32].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_tooltip" => {
                for (index, expected) in [
                    core.int32,
                    core.string,
                    core.int32,
                    core.int32,
                    core.uint32,
                    core.uint32,
                    core.int32,
                ]
                .iter()
                .enumerate()
                {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_theme" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.unit
            }
            "ui_theme_color" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.uint32
            }
            "ui_set_status" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.string, *actual, arg_span(0));
                }
                core.unit
            }
            "ui_state_get" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.int32
            }
            "ui_state_bind" => {
                for (index, expected) in [core.int32, core.int32, core.int32].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_state_bind_text" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.int32, *actual, arg_span(1));
                }
                core.unit
            }
            "ui_state_set" => {
                for (index, expected) in [core.int32, core.int32].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_state_text_length" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.uint_size
            }
            "ui_state_text_read" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "ui_state_text_set" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(core.string, *actual, arg_span(1));
                }
                core.bool_
            }
            "ui_input_length" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.uint_size
            }
            "ui_input_read" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                core.uint_size
            }
            "ui_input_read_exact" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(write_byte_slice, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(write_slice_uint_size, *actual, arg_span(2));
                }
                core.bool_
            }
            "ui_input_bind_app_state" => {
                for (index, expected) in [core.int32, core.string].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.unit
            }
            "ui_checkbox_bind_app_state"
            | "ui_select_bind_app_state"
            | "ui_list_bind_app_state"
            | "ui_table_bind_app_state" => {
                for (index, expected) in [core.int32, core.string].iter().enumerate() {
                    if let Some(actual) = argument_types.get(index) {
                        self.unify_or_error(*expected, *actual, arg_span(index));
                    }
                }
                core.bool_
            }
            "ui_input_refresh_app_state" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.unit
            }
            "ui_checkbox_refresh_app_state"
            | "ui_select_refresh_app_state"
            | "ui_list_refresh_app_state"
            | "ui_table_refresh_app_state" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(core.int32, *actual, arg_span(0));
                }
                core.unit
            }
            "ui_run" => core.int32,
            "vector_splat2" | "vector_splat3" | "vector_splat4" | "vector_splat8" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(float32, *actual, arg_span(0));
                }
                vector.expect("known vector intrinsic")
            }
            "vector_load2" | "vector_load3" | "vector_load4" | "vector_load8" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(read_slice_float32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(uint_size, *actual, arg_span(1));
                }
                vector.expect("known vector intrinsic")
            }
            "vector_store2" | "vector_store3" | "vector_store4" | "vector_store8" => {
                if let Some(actual) = argument_types.first() {
                    self.unify_or_error(write_slice_float32, *actual, arg_span(0));
                }
                if let Some(actual) = argument_types.get(1) {
                    self.unify_or_error(uint_size, *actual, arg_span(1));
                }
                if let Some(actual) = argument_types.get(2) {
                    self.unify_or_error(
                        vector.expect("known vector intrinsic"),
                        *actual,
                        arg_span(2),
                    );
                }
                core.unit
            }
            _ => core.unit,
        }
    }

    fn infer_unary(&mut self, operator: Operator, operand: TypeId, span: Span) -> TypeId {
        match operator {
            Operator::Bang => self.unify_or_error(self.types.core().bool_, operand, span),
            Operator::Plus | Operator::Minus => {
                self.require_numeric(operand, span);
                operand
            }
            Operator::Tilde => {
                self.require_integer(operand, span);
                operand
            }
            _ => operand,
        }
    }

    fn infer_binary(
        &mut self,
        operator: Operator,
        left: TypeId,
        right: TypeId,
        span: Span,
    ) -> TypeId {
        match operator {
            Operator::Plus
            | Operator::Minus
            | Operator::Star
            | Operator::Slash
            | Operator::Percent => {
                let ty = self.unify_or_error(left, right, span);
                self.require_numeric(ty, span);
                ty
            }
            Operator::Equal
            | Operator::NotEqual
            | Operator::Less
            | Operator::LessEqual
            | Operator::Greater
            | Operator::GreaterEqual => {
                self.unify_or_error(left, right, span);
                self.types.core().bool_
            }
            Operator::And | Operator::Or => {
                let bool_ = self.types.core().bool_;
                self.unify_or_error(bool_, left, span);
                self.unify_or_error(bool_, right, span);
                bool_
            }
            Operator::Ampersand | Operator::Pipe | Operator::Caret => {
                let ty = self.unify_or_error(left, right, span);
                self.require_integer(ty, span);
                ty
            }
            Operator::Assign
            | Operator::PlusAssign
            | Operator::MinusAssign
            | Operator::StarAssign
            | Operator::SlashAssign
            | Operator::PercentAssign => {
                self.unify_or_error(left, right, span);
                self.types.core().unit
            }
            _ => self.unification.fresh(&mut self.types),
        }
    }

    fn literal_type(&mut self, kind: LiteralKind, span: Span) -> TypeId {
        let text = self
            .source
            .slice(span)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let core = self.types.core();
        match kind {
            LiteralKind::Bool => core.bool_,
            LiteralKind::Char => core.char_,
            LiteralKind::String => core.string,
            LiteralKind::Integer => {
                for (suffix, ty) in [
                    ("usize", core.uint_size),
                    ("isize", core.int_size),
                    ("u64", core.uint64),
                    ("u32", core.uint32),
                    ("u16", core.uint16),
                    ("u8", core.uint8),
                    ("i64", core.int64),
                    ("i32", core.int32),
                    ("i16", core.int16),
                    ("i8", core.int8),
                ] {
                    if text.ends_with(suffix) {
                        return ty;
                    }
                }
                core.int32
            }
            LiteralKind::Float => {
                if text.ends_with("f16") {
                    core.float16
                } else if text.ends_with("f32") {
                    core.float32
                } else {
                    core.float64
                }
            }
        }
    }

    fn require_numeric(&mut self, ty: TypeId, span: Span) {
        let resolved = self.unification.resolve_shallow(&self.types, ty);
        if !self.types.kind(resolved).is_some_and(TypeKind::is_numeric)
            && !matches!(
                self.types.kind(resolved),
                Some(TypeKind::InferenceVariable(_))
            )
        {
            self.type_error("J0303", "expected a numeric type", span);
        }
    }

    fn require_integer(&mut self, ty: TypeId, span: Span) {
        let resolved = self.unification.resolve_shallow(&self.types, ty);
        if !self.types.kind(resolved).is_some_and(TypeKind::is_integer)
            && !matches!(
                self.types.kind(resolved),
                Some(TypeKind::InferenceVariable(_))
            )
        {
            self.type_error("J0303", "expected an integer type", span);
        }
    }

    fn unify_or_error(&mut self, expected: TypeId, actual: TypeId, span: Span) -> TypeId {
        let expected_resolved = self.unification.resolve_shallow(&self.types, expected);
        let actual_resolved = self.unification.resolve_shallow(&self.types, actual);
        let unify_borrowed_inner =
            |checker: &mut Self, expected_inner: TypeId, actual_inner: TypeId| {
                let expected_kind = checker.types.kind(expected_inner).cloned();
                let actual_kind = checker.types.kind(actual_inner).cloned();
                if let Some(TypeKind::Slice(expected_element)) = expected_kind
                    && let Some(actual_element) = match actual_kind {
                        Some(
                            TypeKind::Slice(element)
                            | TypeKind::Buffer(element)
                            | TypeKind::Array { element, .. },
                        ) => Some(element),
                        _ => None,
                    }
                {
                    return checker.unification.unify(
                        &mut checker.types,
                        expected_element,
                        actual_element,
                    );
                }
                checker
                    .unification
                    .unify(&mut checker.types, expected_inner, actual_inner)
            };
        if let (
            Some(TypeKind::Capability {
                capability: expected_capability @ (Capability::Read | Capability::Write),
                inner: expected_inner,
            }),
            Some(TypeKind::Capability {
                capability: actual_capability,
                inner: actual_inner,
            }),
        ) = (
            self.types.kind(expected_resolved).cloned(),
            self.types.kind(actual_resolved).cloned(),
        ) && matches!(
            (expected_capability, actual_capability),
            (Capability::Read, Capability::Read)
                | (Capability::Read, Capability::Write)
                | (Capability::Write, Capability::Write)
        ) {
            return match unify_borrowed_inner(self, expected_inner, actual_inner) {
                Ok(_) => expected_resolved,
                Err(_) => self.type_error("J0301", "incompatible borrowed type", span),
            };
        }
        if let Some(TypeKind::Capability { capability, inner }) =
            self.types.kind(expected_resolved).cloned()
            && !matches!(
                self.types.kind(actual_resolved),
                Some(TypeKind::Capability { .. })
            )
        {
            return match unify_borrowed_inner(self, inner, actual_resolved) {
                Ok(_) => expected_resolved,
                Err(_) => self.type_error(
                    "J0301",
                    match capability {
                        Capability::Owned => "incompatible owned type",
                        Capability::Read | Capability::Write => "incompatible borrowed type",
                    },
                    span,
                ),
            };
        }
        match self.unification.unify(&mut self.types, expected, actual) {
            Ok(ty) => ty,
            Err(_) => self.type_error("J0301", "incompatible types", span),
        }
    }

    fn require_buffer_capability(&mut self, actual: TypeId, capability: Capability, span: Span) {
        let resolved = self.unification.resolve_shallow(&self.types, actual);
        let (actual_capability, inner) = match self.types.kind(resolved).cloned() {
            Some(TypeKind::Capability { capability, inner }) => (Some(capability), inner),
            _ => (None, resolved),
        };
        let is_buffer = matches!(self.types.kind(inner), Some(TypeKind::Buffer(_)));
        let has_access = match capability {
            Capability::Read => matches!(
                actual_capability,
                None | Some(Capability::Owned | Capability::Read | Capability::Write)
            ),
            Capability::Write => matches!(
                actual_capability,
                None | Some(Capability::Owned | Capability::Write)
            ),
            Capability::Owned => actual_capability.is_none(),
        };
        if !is_buffer || !has_access {
            self.type_error(
                "J0321",
                "buffer operation requires a compatible Buffer capability",
                span,
            );
        }
    }

    /// Recovers the element type from an expected generic buffer constructor
    /// result.  A constructor used without an explicit result annotation gets
    /// a fresh inference variable and is resolved by the surrounding binding.
    fn generic_buffer_element_from_expected(&mut self, expected: TypeId) -> Option<TypeId> {
        // Constructor calls are often inferred before the surrounding `let`
        // annotation is unified. Resolve the full expected carrier here so a
        // later annotation cannot silently bypass the element ownership ABI
        // check with a still-unresolved inference variable.
        let resolved = self
            .unification
            .resolve_deep(&mut self.types, expected)
            .unwrap_or_else(|_| self.unification.resolve_shallow(&self.types, expected));
        let TypeKind::Result { ok, .. } = self.types.kind(resolved)? else {
            return None;
        };
        match self.types.kind(*ok)? {
            TypeKind::Buffer(element) => Some(*element),
            _ => None,
        }
    }

    fn buffer_element_from_type(&self, ty: TypeId) -> Option<TypeId> {
        let resolved = self.unification.resolve_shallow(&self.types, ty);
        let inner = match self.types.kind(resolved) {
            Some(TypeKind::Capability { inner, .. }) => *inner,
            _ => resolved,
        };
        match self.types.kind(inner) {
            Some(TypeKind::Buffer(element)) => Some(*element),
            _ => None,
        }
    }

    fn require_buffer_element_abi(&mut self, element: TypeId, span: Span) {
        self.require_buffer_element_abi_with_moves(element, span, false);
    }

    fn require_buffer_element_move_abi(&mut self, element: TypeId, span: Span) {
        self.require_buffer_element_abi_with_moves(element, span, true);
    }

    fn require_buffer_element_abi_with_moves(
        &mut self,
        element: TypeId,
        span: Span,
        allow_move_only: bool,
    ) {
        let mut visiting = DeterministicSet::new();
        if !self.buffer_element_is_abi_safe(element, &mut visiting, allow_move_only) {
            self.type_error(
                "J0301",
                if allow_move_only {
                    "generic Buffer element must be copy-safe, a nested owning Buffer, an inline owning @repr(C) record, or a tag-selected owning carrier"
                } else {
                    "generic Buffer element must be a copy-safe scalar, array, vector, Option/Result carrier, zero-payload @repr(C) enum, or @repr(C) record"
                },
                span,
            );
        }
    }

    fn buffer_element_is_copy_safe(
        &mut self,
        ty: TypeId,
        visiting: &mut DeterministicSet<TypeId>,
    ) -> bool {
        self.buffer_element_is_abi_safe(ty, visiting, false)
    }

    fn buffer_carrier_move_is_abi_safe(
        &mut self,
        ty: TypeId,
        visiting: &mut DeterministicSet<TypeId>,
    ) -> bool {
        match self.types.kind(ty).cloned() {
            Some(TypeKind::Option(inner)) => {
                self.buffer_carrier_payload_move_is_abi_safe(inner, visiting)
            }
            Some(TypeKind::Result { ok, error }) => {
                let ok_is_owning = self.buffer_carrier_payload_move_is_abi_safe(ok, visiting);
                let error_is_owning = self.buffer_carrier_payload_move_is_abi_safe(error, visiting);
                if !ok_is_owning && !error_is_owning {
                    return false;
                }
                if ok_is_owning {
                    (!error_is_owning && self.buffer_element_is_copy_safe(error, visiting))
                        || (error_is_owning
                            && self.buffer_carrier_payload_move_is_abi_safe(error, visiting))
                } else {
                    self.buffer_element_is_copy_safe(ok, visiting) && error_is_owning
                }
            }
            _ => false,
        }
    }

    /// Returns whether a tag-selected payload owns a descriptor that can be
    /// represented by the carrier field-table ABI. Both nested Buffers and
    /// OwnedString use a three-word inline descriptor; the runtime selects the
    /// destructor from the field depth marker.
    fn buffer_carrier_payload_move_is_abi_safe(
        &mut self,
        ty: TypeId,
        visiting: &mut DeterministicSet<TypeId>,
    ) -> bool {
        matches!(
            self.types.kind(ty),
            Some(TypeKind::Buffer(_)) | Some(TypeKind::OwnedString)
        ) && self.buffer_element_is_abi_safe(ty, visiting, true)
    }

    fn buffer_enum_carrier_move_is_abi_safe(
        &mut self,
        constructor: NominalTypeId,
        arguments: &[TypeId],
        visiting: &mut DeterministicSet<TypeId>,
    ) -> bool {
        let Some(declaration) = self.enums.get(&constructor).cloned() else {
            return false;
        };
        if declaration.repr != AbiRepr::C {
            return false;
        }
        let mut substitution = Substitution::new();
        for (parameter, argument) in declaration
            .generic_parameters
            .iter()
            .zip(arguments.iter().copied())
        {
            substitution.insert(*parameter, argument);
        }
        let mut owning_variants = 0usize;
        for variant_name in &declaration.variant_order {
            let Some(variant) = declaration.variants.get(variant_name) else {
                return false;
            };
            let mut owning_fields = 0usize;
            for field in &variant.fields {
                let field_ty = substitution
                    .apply(&mut self.types, *field)
                    .unwrap_or(self.types.core().error);
                if matches!(
                    self.types.kind(field_ty),
                    Some(TypeKind::Buffer(_)) | Some(TypeKind::OwnedString)
                ) {
                    if !self.buffer_element_is_abi_safe(field_ty, visiting, true) {
                        return false;
                    }
                    owning_fields += 1;
                } else if !self.buffer_element_is_copy_safe(field_ty, visiting) {
                    return false;
                }
            }
            if owning_fields > 0 {
                owning_variants += 1;
            }
        }
        owning_variants > 0
    }

    fn owning_buffer_chain_is_abi_safe(
        &mut self,
        ty: TypeId,
        visiting: &mut DeterministicSet<TypeId>,
    ) -> bool {
        let mut cursor = self.unification.resolve_shallow(&self.types, ty);
        let mut depth = 0u32;
        while let Some(TypeKind::Buffer(inner)) = self.types.kind(cursor) {
            depth = depth.saturating_add(1);
            cursor = *inner;
        }
        if depth == 0 {
            return false;
        }
        // A C-layout record leaf may own its own Buffer fields.  Its cleanup
        // metadata is flattened into the path-aware nested-record drop table
        // in JIR; copy-safe leaves continue through the old recursive path.
        if matches!(
            self.types.kind(cursor),
            Some(TypeKind::Nominal { constructor, .. }) if self.records.contains_key(constructor)
        ) {
            self.buffer_element_is_abi_safe(cursor, visiting, true)
        } else if matches!(
            self.types.kind(cursor),
            Some(TypeKind::Option(_)) | Some(TypeKind::Result { .. })
        ) {
            self.buffer_carrier_move_is_abi_safe(cursor, visiting)
        } else if matches!(self.types.kind(cursor), Some(TypeKind::OwnedString)) {
            self.buffer_element_is_abi_safe(cursor, visiting, true)
        } else {
            self.buffer_element_is_copy_safe(cursor, visiting)
        }
    }

    fn nested_buffer_chain_leaf_is_resize_safe(
        &mut self,
        ty: TypeId,
        visiting: &mut DeterministicSet<TypeId>,
    ) -> bool {
        let mut cursor = self.unification.resolve_shallow(&self.types, ty);
        let mut depth = 0u32;
        while let Some(TypeKind::Buffer(inner)) = self.types.kind(cursor) {
            depth = depth.saturating_add(1);
            cursor = self.unification.resolve_shallow(&self.types, *inner);
        }
        if depth == 0 {
            return false;
        }
        if matches!(
            self.types.kind(cursor),
            Some(TypeKind::Nominal { constructor, .. }) if self.records.contains_key(constructor)
        ) {
            self.resize_record_fields_are_supported(cursor, visiting)
        } else if matches!(
            self.types.kind(cursor),
            Some(TypeKind::Option(_)) | Some(TypeKind::Result { .. })
        ) {
            self.buffer_carrier_move_is_abi_safe(cursor, visiting)
        } else if matches!(self.types.kind(cursor), Some(TypeKind::OwnedString)) {
            // Nested owning buffers need a string-aware runtime path when the
            // final leaf is OwnedString; the descriptor layout is shared with
            // Buffer, but byte-only nested cleanup would leak its UTF-8 payload.
            true
        } else {
            self.buffer_element_is_copy_safe(cursor, visiting)
        }
    }

    /// Returns whether a record leaf can use the current path-aware resize
    /// metadata. The runtime field table can recursively destroy inline
    /// records and Buffer chains whose final leaf is copy-safe. Option/Result
    /// carriers are supported when their active branch owns a Buffer or
    /// OwnedString descriptor.
    fn resize_record_fields_are_supported(
        &mut self,
        ty: TypeId,
        visiting: &mut DeterministicSet<TypeId>,
    ) -> bool {
        let resolved = self.unification.resolve_shallow(&self.types, ty);
        if matches!(
            self.types.kind(resolved),
            Some(TypeKind::Option(_)) | Some(TypeKind::Result { .. })
        ) {
            return self.buffer_carrier_move_is_abi_safe(resolved, visiting);
        }
        if !visiting.insert(resolved) {
            return false;
        }
        let result = match self.types.kind(resolved).cloned() {
            Some(TypeKind::Array { .. }) => {
                // Inline arrays use the same path-aware ownership contract as
                // record fields; JIR later flattens every descriptor offset.
                self.resize_record_value_is_supported(resolved, visiting)
            }
            Some(TypeKind::Nominal {
                constructor,
                arguments,
            }) => {
                if let Some(record) = self.records.get(&constructor).cloned() {
                    if record.repr != AbiRepr::C {
                        false
                    } else {
                        let mut substitution = Substitution::new();
                        for (parameter, argument) in record
                            .generic_parameters
                            .iter()
                            .zip(arguments.iter().copied())
                        {
                            substitution.insert(*parameter, argument);
                        }
                        record.fields.values().all(|field| {
                            let field_ty = substitution
                                .apply(&mut self.types, field.ty)
                                .unwrap_or(self.types.core().error);
                            self.resize_record_value_is_supported(field_ty, visiting)
                        })
                    }
                } else if let Some(enum_declaration) = self.enums.get(&constructor) {
                    enum_declaration.repr == AbiRepr::C
                        && self.buffer_enum_carrier_move_is_abi_safe(
                            constructor,
                            &arguments,
                            visiting,
                        )
                } else {
                    false
                }
            }
            _ => false,
        };
        visiting.remove(&resolved);
        result
    }

    fn resize_record_value_is_supported(
        &mut self,
        ty: TypeId,
        visiting: &mut DeterministicSet<TypeId>,
    ) -> bool {
        let resolved = self.unification.resolve_shallow(&self.types, ty);
        // OwnedString is a move-only descriptor with the same inline layout
        // as Buffer. Record field tables carry a dedicated cleanup marker,
        // so it is safe to resize/clear a record containing this field.
        if matches!(self.types.kind(resolved), Some(TypeKind::OwnedString)) {
            return true;
        }
        // The record resize/clear runtime already consumes the same carrier
        // field table as record drop/remove.  A carrier value never escapes
        // this operation, so selecting its active Buffer branch is atomic and
        // does not require a second ownership result ABI.
        if matches!(
            self.types.kind(resolved),
            Some(TypeKind::Option(_)) | Some(TypeKind::Result { .. })
        ) {
            return self.buffer_carrier_move_is_abi_safe(resolved, visiting);
        }
        let mut leaf = resolved;
        let mut depth = 0u32;
        while let Some(TypeKind::Buffer(inner)) = self.types.kind(leaf) {
            depth = depth.saturating_add(1);
            leaf = self.unification.resolve_shallow(&self.types, *inner);
        }
        if depth > 0 {
            // Nested descriptors may terminate in another inline array or an
            // owning record. Continue through that value shape instead of
            // stopping at the first non-Buffer leaf.
            return self.resize_record_value_is_supported(leaf, visiting);
        }
        match self.types.kind(resolved).cloned() {
            Some(TypeKind::Array { element, .. }) => {
                self.resize_record_value_is_supported(element, visiting)
            }
            Some(TypeKind::Nominal { constructor, .. })
                if self.records.contains_key(&constructor) =>
            {
                self.resize_record_fields_are_supported(resolved, visiting)
            }
            _ => self.buffer_element_is_copy_safe(resolved, visiting),
        }
    }

    /// Returns whether a field is a move-safe value that can be represented in
    /// a record stored inside a generic owning buffer. In addition to a direct
    /// Buffer chain, a C-layout record may contain another C-layout record or
    /// a fixed array of such owning values; lowering flattens their static
    /// offsets into one drop table. Option/Result carriers are supported when
    /// their active branch owns a Buffer or OwnedString descriptor.
    fn owning_record_field_is_abi_safe(
        &mut self,
        ty: TypeId,
        visiting: &mut DeterministicSet<TypeId>,
    ) -> bool {
        if matches!(self.types.kind(ty), Some(TypeKind::OwnedString)) {
            return true;
        }
        if self.owning_buffer_chain_is_abi_safe(ty, visiting) {
            return true;
        }
        if let Some(TypeKind::Array { element, .. }) = self.types.kind(ty).cloned() {
            return self.owning_record_field_is_abi_safe(element, visiting);
        }
        // Option/Result are tagged inline carriers. Their owning payload is
        // selected by the tag during record cleanup; the compact field-table
        // ABI supports Buffer and OwnedString branches without allowing
        // arbitrary enum metadata here.
        if matches!(
            self.types.kind(ty),
            Some(TypeKind::Option(_)) | Some(TypeKind::Result { .. })
        ) {
            return self.buffer_carrier_move_is_abi_safe(ty, visiting);
        }
        matches!(self.types.kind(ty), Some(TypeKind::Nominal { constructor, .. }) if self.records.contains_key(constructor))
            && self.buffer_element_is_abi_safe(ty, visiting, true)
    }

    /// Returns whether `buffer_remove_drop` can dispose one element without
    /// returning it to the caller. Copy-safe elements need only byte-wise
    /// compaction. Owning elements are supported for a direct C-layout record
    /// and for a nested Buffer chain whose final leaf is such a record; the
    /// existing path-aware field table carries the cleanup metadata.
    fn buffer_remove_drop_element_is_supported(
        &mut self,
        ty: TypeId,
        visiting: &mut DeterministicSet<TypeId>,
    ) -> bool {
        let resolved = self.unification.resolve_shallow(&self.types, ty);
        if matches!(self.types.kind(resolved), Some(TypeKind::OwnedString)) {
            return true;
        }
        if self.buffer_element_is_copy_safe(resolved, visiting) {
            return true;
        }
        if matches!(
            self.types.kind(resolved),
            Some(TypeKind::Option(_)) | Some(TypeKind::Result { .. })
        ) {
            return self.buffer_carrier_move_is_abi_safe(resolved, visiting);
        }
        if !visiting.insert(resolved) {
            return false;
        }
        let mut nested_cursor = resolved;
        let mut nested_depth = 0u32;
        while let Some(TypeKind::Buffer(inner)) = self.types.kind(nested_cursor) {
            nested_depth = nested_depth.saturating_add(1);
            nested_cursor = self.unification.resolve_shallow(&self.types, *inner);
        }
        if nested_depth > 0
            && self
                .buffer_remove_drop_value_is_supported(nested_cursor, &mut DeterministicSet::new())
        {
            visiting.remove(&resolved);
            return true;
        }
        let result = match self.types.kind(resolved).cloned() {
            Some(TypeKind::Array { .. }) => {
                // Direct arrays are represented by the same offset table as
                // records; owning members are destroyed before compaction.
                self.buffer_remove_drop_value_is_supported(resolved, visiting)
            }
            Some(TypeKind::Nominal {
                constructor,
                arguments,
            }) => {
                if let Some(record) = self.records.get(&constructor).cloned() {
                    if record.repr != AbiRepr::C {
                        false
                    } else {
                        let mut substitution = Substitution::new();
                        for (parameter, argument) in record
                            .generic_parameters
                            .iter()
                            .zip(arguments.iter().copied())
                        {
                            substitution.insert(*parameter, argument);
                        }
                        let mut has_owning_field = false;
                        let fields_supported = record.fields.values().all(|field| {
                            let field_ty = substitution
                                .apply(&mut self.types, field.ty)
                                .unwrap_or(self.types.core().error);
                            if self.buffer_element_is_copy_safe(field_ty, visiting) {
                                true
                            } else if self.buffer_remove_drop_value_is_supported(field_ty, visiting)
                            {
                                has_owning_field = true;
                                true
                            } else {
                                false
                            }
                        });
                        fields_supported && has_owning_field
                    }
                } else if let Some(enum_declaration) = self.enums.get(&constructor) {
                    enum_declaration.repr == AbiRepr::C
                        && self.buffer_enum_carrier_move_is_abi_safe(
                            constructor,
                            &arguments,
                            visiting,
                        )
                } else {
                    false
                }
            }
            _ => false,
        };
        visiting.remove(&resolved);
        result
    }

    /// Returns whether a direct `Buffer<T>` element can be transferred into a
    /// caller-owned output slot.  The runtime operation is a byte-preserving
    /// move: it copies the selected element once, compacts the remaining
    /// bytes, and clears the stale tail.  Therefore every owner embedded in
    /// `T` must already have a complete move/drop ABI, while copy-only values
    /// intentionally stay on the ordinary `buffer_remove` path.
    fn buffer_move_into_element_is_supported(
        &mut self,
        ty: TypeId,
        visiting: &mut DeterministicSet<TypeId>,
    ) -> bool {
        let resolved = self.unification.resolve_shallow(&self.types, ty);
        let mut copy_visiting = DeterministicSet::new();
        if self.buffer_element_is_copy_safe(resolved, &mut copy_visiting) {
            return false;
        }
        self.buffer_element_is_abi_safe(resolved, visiting, true)
    }

    fn buffer_remove_drop_value_is_supported(
        &mut self,
        ty: TypeId,
        visiting: &mut DeterministicSet<TypeId>,
    ) -> bool {
        let resolved = self.unification.resolve_shallow(&self.types, ty);
        if matches!(self.types.kind(resolved), Some(TypeKind::OwnedString)) {
            return true;
        }
        // Drop-only remove never transfers the carrier value to a temporary.
        // The existing field-table ABI can therefore select the active
        // Option/Result Buffer branch before compaction without introducing a
        // second ownership path. Move-return remove remains separate because
        // its result would need to carry the same metadata to its drop glue.
        if matches!(
            self.types.kind(resolved),
            Some(TypeKind::Option(_)) | Some(TypeKind::Result { .. })
        ) {
            return self.buffer_carrier_move_is_abi_safe(resolved, visiting);
        }
        let mut cursor = resolved;
        let mut depth = 0u32;
        while let Some(TypeKind::Buffer(inner)) = self.types.kind(cursor) {
            depth = depth.saturating_add(1);
            cursor = self.unification.resolve_shallow(&self.types, *inner);
        }
        if depth > 0 {
            if self.buffer_element_is_copy_safe(cursor, visiting) {
                return true;
            }
            return self.buffer_remove_drop_value_is_supported(cursor, visiting);
        }
        if let Some(TypeKind::Array { element, .. }) = self.types.kind(resolved).cloned() {
            return self.buffer_remove_drop_value_is_supported(element, visiting);
        }
        if matches!(
            self.types.kind(resolved),
            Some(TypeKind::Nominal { constructor, .. }) if self.records.contains_key(constructor)
        ) {
            return self.buffer_remove_drop_element_is_supported(resolved, visiting);
        }
        false
    }

    fn buffer_element_is_abi_safe(
        &mut self,
        ty: TypeId,
        visiting: &mut DeterministicSet<TypeId>,
        allow_move_only: bool,
    ) -> bool {
        let resolved = self.unification.resolve_shallow(&self.types, ty);
        if !visiting.insert(resolved) {
            return false;
        }
        let result = match self.types.kind(resolved).cloned() {
            Some(
                TypeKind::Error | TypeKind::InferenceVariable(_) | TypeKind::GenericParameter(_),
            ) => true,
            Some(
                TypeKind::Bool | TypeKind::Char | TypeKind::Integer { .. } | TypeKind::Float(_),
            ) => true,
            Some(TypeKind::Array { element, .. } | TypeKind::Vector { element, .. }) => {
                // Fixed arrays/vectors are inline values.  In the move-aware
                // contract they may contain owning Buffer descriptors; the
                // JIR field table flattens every descriptor offset so the
                // runtime can destroy the array element without hidden
                // metadata.  The copy-safe path remains intentionally strict.
                if allow_move_only {
                    self.buffer_element_is_abi_safe(element, visiting, true)
                } else {
                    self.buffer_element_is_copy_safe(element, visiting)
                }
            }
            Some(TypeKind::Option(inner))
                if allow_move_only
                    && self.buffer_carrier_payload_move_is_abi_safe(inner, visiting) =>
            {
                self.buffer_carrier_move_is_abi_safe(resolved, visiting)
            }
            Some(TypeKind::Result { ok, error })
                if allow_move_only
                    && (self.buffer_carrier_payload_move_is_abi_safe(ok, visiting)
                        || self.buffer_carrier_payload_move_is_abi_safe(error, visiting)) =>
            {
                self.buffer_carrier_move_is_abi_safe(resolved, visiting)
            }
            Some(TypeKind::Option(inner)) => {
                // A carrier is copy-safe when every payload is copy-safe;
                // it carries no hidden ownership or destructor metadata.
                self.buffer_element_is_copy_safe(inner, visiting)
            }
            Some(TypeKind::Result { ok, error }) => {
                // Both Result branches must remain plain copy values.
                self.buffer_element_is_copy_safe(ok, visiting)
                    && self.buffer_element_is_copy_safe(error, visiting)
            }
            Some(TypeKind::Nominal {
                constructor,
                arguments,
            }) => {
                if let Some(record) = self.records.get(&constructor).cloned() {
                    if record.repr != AbiRepr::C {
                        false
                    } else {
                        let mut substitution = Substitution::new();
                        for (parameter, argument) in record
                            .generic_parameters
                            .iter()
                            .zip(arguments.iter().copied())
                        {
                            substitution.insert(*parameter, argument);
                        }
                        record.fields.values().all(|field| {
                            let field_ty = substitution
                                .apply(&mut self.types, field.ty)
                                .unwrap_or(self.types.core().error);
                            if allow_move_only
                                && self.owning_record_field_is_abi_safe(field_ty, visiting)
                            {
                                true
                            } else {
                                self.buffer_element_is_copy_safe(field_ty, visiting)
                            }
                        })
                    }
                } else if let Some(declaration) = self.enums.get(&constructor).cloned() {
                    if declaration.repr != AbiRepr::C {
                        false
                    } else if allow_move_only
                        && self.buffer_enum_carrier_move_is_abi_safe(
                            constructor,
                            &arguments,
                            visiting,
                        )
                    {
                        true
                    } else {
                        let mut substitution = Substitution::new();
                        for (parameter, argument) in declaration
                            .generic_parameters
                            .iter()
                            .zip(arguments.iter().copied())
                        {
                            substitution.insert(*parameter, argument);
                        }
                        declaration.variants.values().all(|variant| {
                            variant.fields.iter().all(|field| {
                                let field_ty = substitution
                                    .apply(&mut self.types, *field)
                                    .unwrap_or(self.types.core().error);
                                self.buffer_element_is_copy_safe(field_ty, visiting)
                            })
                        })
                    }
                } else {
                    false
                }
            }
            Some(TypeKind::Buffer(_)) if allow_move_only => {
                self.owning_buffer_chain_is_abi_safe(resolved, visiting)
            }
            // `OwnedString` is a move-only three-word descriptor.  The JIR
            // layout is target-native and its drop glue is emitted by the
            // owning `Buffer<OwnedString>` path.
            Some(TypeKind::OwnedString) if allow_move_only => true,
            Some(
                TypeKind::String
                | TypeKind::OwnedString
                | TypeKind::Unit
                | TypeKind::Never
                | TypeKind::Buffer(_)
                | TypeKind::Slice(_)
                | TypeKind::Pointer(_)
                | TypeKind::Function { .. }
                | TypeKind::Capability { .. },
            )
            | None => false,
        };
        visiting.remove(&resolved);
        result
    }

    fn reference_symbol(&self, span: Span, namespace: Namespace) -> Option<&Symbol> {
        self.resolution
            .references
            .iter()
            .find(|reference| reference.span == span && reference.namespace == namespace)
            .and_then(|reference| self.resolution.symbol(reference.symbol))
    }

    fn declaration_symbol(&self, span: Span) -> Option<SymbolId> {
        self.resolution
            .symbols
            .iter()
            .find(|symbol| symbol.span == span)
            .map(|symbol| symbol.id)
    }

    fn assign_declaration(&mut self, span: Span, ty: TypeId) {
        if let Some(symbol) = self.declaration_symbol(span) {
            self.symbol_types[symbol.index()] = Some(ty);
        }
    }

    fn type_error(&mut self, code: &'static str, message: &'static str, span: Span) -> TypeId {
        self.diagnostics
            .push(Diagnostic::error(code, message, span, message));
        self.types.core().error
    }

    fn builtin_error(&mut self, error: BuiltinTypeError, span: Span) -> TypeId {
        self.diagnostics.push(Diagnostic::error(
            "J0302",
            error.to_string(),
            span,
            "invalid core type application",
        ));
        self.types.core().error
    }

    fn finalize_types(&mut self) {
        for ty in &mut self.symbol_types {
            if let Some(current) = *ty {
                *ty = self.unification.resolve_deep(&mut self.types, current).ok();
            }
        }
        for expression in &mut self.expressions {
            if let Ok(ty) = self
                .unification
                .resolve_deep(&mut self.types, expression.ty)
            {
                expression.ty = ty;
            }
        }
        for record in self.records.values_mut() {
            for field in record.fields.values_mut() {
                if let Ok(ty) = self.unification.resolve_deep(&mut self.types, field.ty) {
                    field.ty = ty;
                }
            }
        }
        for declaration in self.enums.values_mut() {
            for variant in declaration.variants.values_mut() {
                for field in &mut variant.fields {
                    if let Ok(ty) = self.unification.resolve_deep(&mut self.types, *field) {
                        *field = ty;
                    }
                }
            }
        }
        for site in &mut self.propagation_sites {
            if let Ok(ty) = self
                .unification
                .resolve_deep(&mut self.types, site.success_type)
            {
                site.success_type = ty;
            }
            if let Ok(ty) = self
                .unification
                .resolve_deep(&mut self.types, site.residual_type)
            {
                site.residual_type = ty;
            }
            if let Ok(ty) = self
                .unification
                .resolve_deep(&mut self.types, site.return_type)
            {
                site.return_type = ty;
            }
        }
        let mut unresolved_regions = Vec::new();
        for site in &mut self.region_allocations {
            match self
                .unification
                .resolve_deep(&mut self.types, site.result_type)
            {
                Ok(result) if !contains_inference_variable(&self.types, result) => {
                    site.result_type = result;
                }
                Ok(_) | Err(_) => {
                    site.result_type = self.types.core().error;
                    unresolved_regions.push(site.span);
                }
            }
        }
        for span in unresolved_regions {
            self.diagnostics.push(Diagnostic::error(
                "J0508",
                "cannot infer region allocation element type",
                span,
                "add an explicit `Buffer<T>` type annotation to the binding",
            ));
        }
        self.expressions.sort_by_key(|expression| {
            (
                expression.span.start,
                expression.span.end,
                expression.ty.index(),
            )
        });
        for (index, expression) in self.expressions.iter_mut().enumerate() {
            expression.id = TypedExpressionId(index);
        }
    }

    fn export_nominal_layouts(&self) -> Vec<NominalLayout> {
        let records = self
            .records
            .iter()
            .map(|(constructor, record)| NominalLayout {
                constructor: *constructor,
                generic_parameters: record.generic_parameters.clone(),
                repr: record.repr,
                kind: NominalLayoutKind::Record {
                    fields: record
                        .field_order
                        .iter()
                        .filter_map(|name| {
                            record.fields.get(name).map(|field| NominalFieldLayout {
                                name: name.clone(),
                                ty: field.ty,
                            })
                        })
                        .collect(),
                },
            });
        let enums = self
            .enums
            .iter()
            .map(|(constructor, declaration)| NominalLayout {
                constructor: *constructor,
                generic_parameters: declaration.generic_parameters.clone(),
                repr: declaration.repr,
                kind: NominalLayoutKind::Enum {
                    variants: declaration
                        .variant_order
                        .iter()
                        .filter_map(|name| {
                            declaration
                                .variants
                                .get(name)
                                .map(|variant| NominalVariantLayout {
                                    name: name.clone(),
                                    fields: variant.fields.clone(),
                                })
                        })
                        .collect(),
                },
            });
        let mut layouts: Vec<_> = records.chain(enums).collect();
        layouts.sort_by_key(|layout| layout.constructor);
        layouts
    }

    /// Exports layouts for nominal declarations reachable through package
    /// interfaces without adding every dependency enum to the current
    /// constructor-resolution namespace.  Keeping these two concerns separate
    /// prevents unrelated variants such as `BufferStatus.Ok` from making an
    /// unqualified `Ok(...)` constructor ambiguous in another module.
    fn export_catalog_nominal_layouts(&mut self) -> Vec<NominalLayout> {
        let interfaces: Vec<_> = self
            .module_catalog
            .into_iter()
            .flat_map(|catalog| catalog.interfaces())
            .filter(|module| self.resolution.module_name.as_deref() != Some(module.name.as_str()))
            .flat_map(|module| {
                module.members.values().filter_map(|member| {
                    if member.record_interface.is_none() && member.enum_interface.is_none() {
                        return None;
                    }
                    Some((
                        module.name.clone(),
                        member.name.clone(),
                        member.record_interface.clone(),
                        member.enum_interface.clone(),
                    ))
                })
            })
            .collect();

        let mut layouts = Vec::new();
        for (module_name, member_name, record, enumeration) in interfaces {
            let canonical_path = format!("{module_name}.{member_name}");
            let owner =
                jadren_resolve::QualifiedSymbolId::from_path(Namespace::Type, &canonical_path);
            let constructor = NominalTypeId::from_symbol_fingerprint(owner.fingerprint());
            if let Some(record) = record {
                layouts.push(NominalLayout {
                    constructor,
                    generic_parameters: generic_parameter_ids(
                        owner.fingerprint(),
                        record.generic_count,
                    ),
                    repr: record.repr,
                    kind: NominalLayoutKind::Record {
                        fields: record
                            .fields
                            .iter()
                            .map(|field| NominalFieldLayout {
                                name: field.name.clone(),
                                ty: self.lower_module_type(Some(owner), &field.ty),
                            })
                            .collect(),
                    },
                });
            }
            if let Some(enumeration) = enumeration {
                layouts.push(NominalLayout {
                    constructor,
                    generic_parameters: generic_parameter_ids(
                        owner.fingerprint(),
                        enumeration.generic_count,
                    ),
                    repr: enumeration.repr,
                    kind: NominalLayoutKind::Enum {
                        variants: enumeration
                            .variants
                            .iter()
                            .map(|variant| NominalVariantLayout {
                                name: variant.name.clone(),
                                fields: variant
                                    .fields
                                    .iter()
                                    .map(|field| self.lower_module_type(Some(owner), field))
                                    .collect(),
                            })
                            .collect(),
                    },
                });
            }
        }
        layouts
    }
}

fn is_binding_pattern(path: &jadren_parser::Path) -> bool {
    path.segments.len() == 1
        && path.segments[0]
            .text
            .chars()
            .next()
            .is_some_and(char::is_lowercase)
}

fn annotation_path_text(annotation: &jadren_parser::Annotation) -> String {
    annotation
        .name
        .segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect::<Vec<_>>()
        .join(".")
}

fn is_valid_export_symbol(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn collect_generic_parameters(
    store: &TypeStore,
    ty: TypeId,
    output: &mut DeterministicSet<GenericParameterId>,
) {
    match store.kind(ty) {
        Some(TypeKind::GenericParameter(parameter)) => {
            output.insert(*parameter);
        }
        Some(TypeKind::Array { element, .. })
        | Some(TypeKind::Buffer(element))
        | Some(TypeKind::Slice(element))
        | Some(TypeKind::Pointer(element))
        | Some(TypeKind::Option(element)) => {
            collect_generic_parameters(store, *element, output);
        }
        Some(TypeKind::Result { ok, error }) => {
            collect_generic_parameters(store, *ok, output);
            collect_generic_parameters(store, *error, output);
        }
        Some(TypeKind::Nominal { arguments, .. }) => {
            for argument in arguments {
                collect_generic_parameters(store, *argument, output);
            }
        }
        Some(TypeKind::Function { parameters, result }) => {
            for parameter in parameters {
                collect_generic_parameters(store, *parameter, output);
            }
            collect_generic_parameters(store, *result, output);
        }
        Some(TypeKind::Capability { inner, .. }) => {
            collect_generic_parameters(store, *inner, output);
        }
        _ => {}
    }
}

fn contains_inference_variable(store: &TypeStore, ty: TypeId) -> bool {
    match store.kind(ty) {
        Some(TypeKind::InferenceVariable(_)) => true,
        Some(TypeKind::Array { element, .. })
        | Some(TypeKind::Buffer(element))
        | Some(TypeKind::Slice(element))
        | Some(TypeKind::Pointer(element))
        | Some(TypeKind::Option(element))
        | Some(TypeKind::Capability { inner: element, .. }) => {
            contains_inference_variable(store, *element)
        }
        Some(TypeKind::Result { ok, error }) => {
            contains_inference_variable(store, *ok) || contains_inference_variable(store, *error)
        }
        Some(TypeKind::Nominal { arguments, .. }) => arguments
            .iter()
            .any(|argument| contains_inference_variable(store, *argument)),
        Some(TypeKind::Function { parameters, result }) => {
            parameters
                .iter()
                .any(|parameter| contains_inference_variable(store, *parameter))
                || contains_inference_variable(store, *result)
        }
        _ => false,
    }
}

fn generic_parameter_ids(owner: Fingerprint, count: usize) -> Vec<GenericParameterId> {
    (0..count)
        .map(|index| GenericParameterId { owner, index })
        .collect()
}

fn item_generic_parameters(item: &Item) -> &[GenericParameter] {
    match item {
        Item::Function(function) => &function.generic_parameters,
        Item::Struct(record) | Item::Component(record) => &record.generic_parameters,
        Item::Enum(declaration) => &declaration.generic_parameters,
        Item::ExternBlock(_) => &[],
    }
}

fn substitution_from_arguments(
    parameters: &[GenericParameterId],
    arguments: &[TypeId],
) -> Substitution {
    let mut substitution = Substitution::new();
    for (parameter, argument) in parameters.iter().zip(arguments) {
        substitution.insert(*parameter, *argument);
    }
    substitution
}

fn enum_constructor_selector(expression: &Expression) -> Option<(Option<String>, String)> {
    match expression {
        Expression::Name(name) => Some((None, name.text.clone())),
        Expression::Field { base, field, .. } => {
            Some((Some(expression_name_path(base)?), field.text.clone()))
        }
        _ => None,
    }
}

fn expression_name_path(expression: &Expression) -> Option<String> {
    match expression {
        Expression::Name(name) => Some(name.text.clone()),
        Expression::Field { base, field, .. } => {
            let mut path = expression_name_path(base)?;
            path.push('.');
            path.push_str(&field.text);
            Some(path)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use jadren_lexer::lex;
    use jadren_parser::parse;
    use jadren_resolve::resolve;
    use jadren_source::{SourceManager, Span};
    use jadren_types::TypeKind;

    use super::{ExpressionKind, TypedExpression, TypedExpressionId, check_types};

    fn check(text: &str) -> super::TypeCheckOutput {
        let mut sources = SourceManager::new();
        let id = sources.add("test.jdn", text).expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics);
        let resolution = resolve(source, &parsed.file);
        assert!(!resolution.has_errors(), "{:?}", resolution.diagnostics);
        check_types(source, &parsed.file, &resolution)
    }
    #[test]
    fn infers_literals_locals_and_arithmetic() {
        let output = check("module test; fn main() { let x = 1; let y = x + 2; print(y) }");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        assert!(output.symbol_types.iter().flatten().any(|ty| {
            output.types.kind(*ty)
                == Some(&TypeKind::Integer {
                    signedness: jadren_types::Signedness::Signed,
                    width: jadren_types::IntegerWidth::Bits32,
                })
        }));
    }

    #[test]
    fn accepts_numeric_width_aliases_and_rejects_reserved_f128() {
        let output = check(
            "module test; fn main(a: F16, b: Float16, c: F32, d: Float32) -> F64 { let narrow = (a as F32) + (b as F32); return (narrow as F64) + (c as F64) + (d as F64) }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let float16_symbols = output
            .symbol_types
            .iter()
            .flatten()
            .filter(|ty| {
                output.types.kind(**ty) == Some(&TypeKind::Float(jadren_types::FloatWidth::Bits16))
            })
            .count();
        assert_eq!(float16_symbols, 2, "F16 and Float16 must share Float16");

        let reserved = check("module test; fn main(value: F128) { print(value) }");
        assert!(reserved.has_errors());
    }

    #[test]
    fn exposes_stable_typed_expression_index_and_kinds() {
        let output = check(
            "module test; fn main(value: Int32) { let x = value + 2; if x > 0 { print(x) } }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let ids: Vec<_> = output
            .expressions
            .iter()
            .map(|expression| expression.id.index())
            .collect();
        assert_eq!(ids, (0..ids.len()).collect::<Vec<_>>());
        assert!(
            output
                .expressions
                .iter()
                .any(|expression| expression.kind == super::ExpressionKind::Name)
        );
        assert!(
            output
                .expressions
                .iter()
                .any(|expression| expression.kind == super::ExpressionKind::Binary)
        );
        assert!(
            output
                .expressions
                .iter()
                .any(|expression| expression.kind == super::ExpressionKind::If)
        );
        assert!(output.expressions.iter().all(|expression| {
            expression.id.index() < output.expressions.len()
                && output.types.kind(expression.ty).is_some()
                && output.typed_expression(expression.id) == Some(expression)
        }));
        assert!(
            output
                .typed_expression(super::TypedExpressionId::new(output.expressions.len()))
                .is_none()
        );
    }

    #[test]
    fn typed_expression_query_is_deterministic_and_span_aware() {
        let output = check(
            "module test; fn main(value: Int32) { let x = value + 2; if x > 0 { print(x) } }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let source = output
            .expressions
            .first()
            .expect("typed expression")
            .span
            .source;
        let binary = output
            .query_typed_expressions()
            .source(source)
            .kind(super::ExpressionKind::Binary)
            .first()
            .expect("binary expression");
        assert_eq!(
            output
                .typed_expression_exact_span(binary.span)
                .map(|expression| expression.id),
            Some(binary.id)
        );
        assert!(
            output
                .query_typed_expressions()
                .within_span(binary.span)
                .iter()
                .any(|expression| expression.id == binary.id)
        );
        assert!(
            output
                .query_typed_expressions()
                .at(source, binary.span.start)
                .iter()
                .any(|expression| expression.id == binary.id)
        );
        let nested = output
            .query_typed_expressions()
            .intersecting_span(binary.span)
            .iter()
            .map(|expression| expression.id)
            .collect::<Vec<_>>();
        assert!(nested.windows(2).all(|window| window[0] < window[1]));
    }

    #[test]
    fn typed_expression_query_invariants_hold_over_generated_ranges() {
        let mut sources = SourceManager::new();
        let source_a = sources.add("a.jdn", "a").expect("source a");
        let source_b = sources.add("b.jdn", "b").expect("source b");
        let type_check = check("module test; fn main() { let value = 1; }");
        let ty = type_check.expressions.first().expect("typed expression").ty;
        let expressions = (0..256)
            .map(|index| {
                let source = if index % 5 == 0 { source_b } else { source_a };
                let start = index * 3;
                let end = start + index % 7 + 1;
                let kind = match index % 3 {
                    0 => ExpressionKind::Name,
                    1 => ExpressionKind::Literal,
                    _ => ExpressionKind::Binary,
                };
                TypedExpression {
                    id: TypedExpressionId::new(index),
                    kind,
                    span: Span { source, start, end },
                    ty,
                }
            })
            .collect::<Vec<_>>();
        let query = super::TypedExpressionQuery::new(&expressions);
        let ids = query
            .iter()
            .map(|expression| expression.id.index())
            .collect::<Vec<_>>();
        assert_eq!(ids, (0..256).collect::<Vec<_>>());
        for expression in &expressions {
            assert_eq!(
                query
                    .exact_span(expression.span)
                    .first()
                    .map(|candidate| candidate.id),
                Some(expression.id)
            );
            assert!(query.within_span(expression.span).iter().all(
                |candidate| candidate.span.source == expression.span.source
                    && candidate.span.start >= expression.span.start
                    && candidate.span.end <= expression.span.end
            ));
            assert!(
                query
                    .intersecting_span(expression.span)
                    .iter()
                    .all(|candidate| candidate.span.source == expression.span.source
                        && candidate.span.start < expression.span.end
                        && expression.span.start < candidate.span.end)
            );
            let at = query
                .at(expression.span.source, expression.span.start)
                .innermost()
                .expect("caret match");
            assert_eq!(at.span.source, expression.span.source);
            assert!(
                at.span.is_empty() && at.span.start == expression.span.start
                    || at.span.start <= expression.span.start
                        && expression.span.start < at.span.end
            );
        }
        assert!(
            query
                .source(source_a)
                .iter()
                .all(|expression| expression.span.source == source_a)
        );
        assert!(
            query
                .source(source_a)
                .exact_span(expressions[0].span)
                .first()
                .is_none()
        );
    }

    #[test]
    fn validates_numeric_casts_and_rejects_non_numeric_sources() {
        let output = check(
            "module test; fn cast(value: Int32, real: Float32) -> Int64 { let wide = value as Int64; let converted = value as Float64; let narrowed = real as Int32; return wide }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);

        let invalid = check("module test; fn bad() { let value = true as Int32 } ");
        assert!(
            invalid
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0321"),
            "expected J0321, got {:?}",
            invalid.diagnostics
        );
    }

    #[test]
    fn validates_while_condition_and_loop_control() {
        let output = check(
            "module test; fn main() { var count: Int32 = 3; while count > 0 { if count == 1 { break } count -= 1 continue } }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);

        let invalid_condition = check("module test; fn bad() { while 1 { break } }");
        assert!(
            invalid_condition
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0301")
        );

        let invalid_control = check("module test; fn bad() { break continue }");
        assert!(
            invalid_control
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0318")
        );
        assert!(
            invalid_control
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0319")
        );
    }

    #[test]
    fn validates_for_array_binding_and_rejects_non_array_iterables() {
        let output = check(
            "module test; fn main(values: [Int32; 3]) { for value in values { print(value) } }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);

        let buffer = check(
            "module test; fn main(values: Buffer<Int32>) { for value in values { print(value) } }",
        );
        assert!(!buffer.has_errors(), "{:?}", buffer.diagnostics);

        let slice = check(
            "module test; fn main(values: Slice<Int32>) { for value in values { print(value) } }",
        );
        assert!(!slice.has_errors(), "{:?}", slice.diagnostics);

        let invalid =
            check("module test; fn bad(value: Int32) { for item in value { print(item) } }");
        assert!(
            invalid
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0320")
        );
    }

    #[test]
    fn validates_buffer_and_slice_index_iteration() {
        let output = check(
            "module test; fn update(values: write Slice<Int32>) { for index in values.indices { values[index] = values[index] + 1 } }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);

        let invalid = check(
            "module test; fn update(values: [Int32; 2]) { for index in values.indices { values[index] = 1 } }",
        );
        assert!(
            invalid
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0320"),
            "expected J0320, got {:?}",
            invalid.diagnostics
        );
    }

    #[test]
    fn validates_generic_buffer_mutations_and_copy_safe_element_contract() {
        let valid = check(
            "module test; fn mutate(values: write Buffer<UInt8>) { let appended: Bool = buffer_append(values, 7u8); let inserted: Bool = buffer_insert(values, 0usize, 3u8); let removed: Bool = buffer_remove(values, 1usize); }",
        );
        assert!(!valid.has_errors(), "{:?}", valid.diagnostics);

        let record = check(
            "module test; @repr(C) struct Pair { left: Int32, right: Int32 } fn mutate(values: write Buffer<Pair>) { buffer_append(values, Pair { left: 1, right: 2 }); }",
        );
        assert!(!record.has_errors(), "{:?}", record.diagnostics);

        let enum_value = check(
            "module test; @repr(C) enum State { Idle, Running } fn mutate(values: write Buffer<State>) { let state: State = Idle; buffer_append(values, state); }",
        );
        assert!(!enum_value.has_errors(), "{:?}", enum_value.diagnostics);

        let option_value = check(
            "module test; fn mutate(values: write Buffer<Option<Int32>>) { let state: Option<Int32> = Some(7); buffer_append(values, state); }",
        );
        assert!(!option_value.has_errors(), "{:?}", option_value.diagnostics);

        let result_value = check(
            "module test; fn mutate(values: write Buffer<Result<Int32, Int32>>) { let state: Result<Int32, Int32> = Ok(7); buffer_append(values, state); }",
        );
        assert!(!result_value.has_errors(), "{:?}", result_value.diagnostics);

        let owning_option = check(
            "module test; fn mutate(values: write Buffer<Option<Buffer<Int32>>>, inner: Buffer<Int32>) { let state: Option<Buffer<Int32>> = Some(inner); buffer_append(values, state); }",
        );
        assert!(
            !owning_option.has_errors(),
            "{:?}",
            owning_option.diagnostics
        );

        let owning_result = check(
            "module test; fn mutate(values: write Buffer<Result<Buffer<Int32>, Bool>>, inner: Buffer<Int32>) { let state: Result<Buffer<Int32>, Bool> = Ok(inner); buffer_append(values, state); }",
        );
        assert!(
            !owning_result.has_errors(),
            "{:?}",
            owning_result.diagnostics
        );

        let multi_owning_carrier = check(
            "module test; fn mutate(values: write Buffer<Result<Buffer<Int32>, Buffer<Int32>>>, inner: Buffer<Int32>) { let state: Result<Buffer<Int32>, Buffer<Int32>> = Ok(inner); buffer_append(values, state); }",
        );
        assert!(
            !multi_owning_carrier.has_errors(),
            "expected multi-owning carrier to be accepted, got {:?}",
            multi_owning_carrier.diagnostics
        );

        let owning_enum = check(
            "module test; @repr(C) enum Event { Idle, Ready(Buffer<Int32>), Done } fn mutate(values: write Buffer<Event>, inner: Buffer<Int32>) { let state: Event = Event.Ready(inner); buffer_append(values, state); }",
        );
        assert!(
            !owning_enum.has_errors(),
            "expected owning enum carrier to be accepted, got {:?}",
            owning_enum.diagnostics
        );

        let multi_owning_enum = check(
            "module test; @repr(C) enum Bad { First(Buffer<Int32>), Second(Buffer<Int32>), Empty } fn mutate(values: write Buffer<Bad>, inner: Buffer<Int32>) { let state: Bad = Bad.First(inner); buffer_append(values, state); }",
        );
        assert!(
            !multi_owning_enum.has_errors(),
            "expected multiple owning enum variants to be accepted, got {:?}",
            multi_owning_enum.diagnostics
        );

        let multi_field_owning_enum = check(
            "module test; @repr(C) enum Bad { First(Buffer<Int32>, Int32), Empty } fn mutate(values: write Buffer<Bad>, inner: Buffer<Int32>) { let state: Bad = Bad.First(inner, 1); buffer_append(values, state); }",
        );
        assert!(
            !multi_field_owning_enum.has_errors(),
            "expected multiple owning enum fields to be accepted, got {:?}",
            multi_field_owning_enum.diagnostics
        );

        let owning_string_enum = check(
            "module test; @repr(C) enum Event { Idle, Text(OwnedString), Pair(OwnedString, Int32), Done } fn mutate(values: write Buffer<Event>, item: OwnedString, other: OwnedString) { let text: Event = Event.Text(item); let pair: Event = Event.Pair(other, 7); buffer_append(values, text); buffer_append(values, pair); }",
        );
        assert!(
            !owning_string_enum.has_errors(),
            "expected direct OwnedString enum fields to be accepted, got {:?}",
            owning_string_enum.diagnostics
        );

        let owning_string_enum_move = check(
            "module test; @repr(C) enum Event { Idle, Text(OwnedString), Pair(OwnedString, Int32), Done } fn mutate(values: write Buffer<Event>, output: write Event, incoming: Event) { let removed: Bool = buffer_remove_move_into(values, 0usize, output); let removed_status: Int32 = buffer_remove_move_into_status(values, 99usize, output); let inserted: Bool = buffer_insert_move_from(values, 0usize, incoming); if !removed { } if removed_status == 0 { } if !inserted { } }",
        );
        assert!(
            !owning_string_enum_move.has_errors(),
            "expected enum OwnedString raw move operations to be accepted, got {:?}",
            owning_string_enum_move.diagnostics
        );

        let owning_string_enum_mutations = check(
            "module test; @repr(C) enum Event { Idle, Text(OwnedString), Pair(OwnedString, Int32), Done } fn mutate(values: write Buffer<Event>) { let resized: Bool = buffer_resize_move(values, 2usize); let cleared: Bool = buffer_clear_move(values); let removed: Bool = buffer_remove_drop(values, 0usize); if !resized { } if !cleared { } if !removed { } }",
        );
        assert!(
            !owning_string_enum_mutations.has_errors(),
            "expected enum OwnedString resize/clear/remove to be accepted, got {:?}",
            owning_string_enum_mutations.diagnostics
        );

        let owning_array = check(
            "module test; fn mutate(values: write Buffer<[Buffer<Int32>; 2]>, first: Buffer<Int32>, second: Buffer<Int32>) { let item: [Buffer<Int32>; 2] = [first, second]; buffer_append(values, item); let resized: Bool = buffer_resize_move(values, 1usize); let removed: Int32 = buffer_remove_drop_status(values, 0usize); let cleared: Bool = buffer_clear_move(values); }",
        );
        assert!(
            !owning_array.has_errors(),
            "expected owning Buffer array element to be accepted, got {:?}",
            owning_array.diagnostics
        );

        let nested_owning = check(
            "module test; fn mutate(values: write Buffer<Buffer<Int32>>, inner: Buffer<Int32>) { buffer_append(values, inner); }",
        );
        assert!(
            !nested_owning.has_errors(),
            "{:?}",
            nested_owning.diagnostics
        );

        let owned_string_buffer = check(
            "module test; fn mutate(values: write Buffer<OwnedString>, item: OwnedString) { buffer_append(values, item); }",
        );
        assert!(
            !owned_string_buffer.has_errors(),
            "expected Buffer<OwnedString> append to be accepted, got {:?}",
            owned_string_buffer.diagnostics
        );

        let owned_string_pop = check(
            "module test; fn take(values: write Buffer<OwnedString>) -> Result<OwnedString, Int32> { return buffer_pop(values); }",
        );
        assert!(
            !owned_string_pop.has_errors(),
            "expected Buffer<OwnedString> pop to be accepted, got {:?}",
            owned_string_pop.diagnostics
        );

        let owned_string_remove = check(
            "module test; fn take(values: write Buffer<OwnedString>) -> Result<OwnedString, Int32> { return buffer_remove_move(values, 0usize); }",
        );
        assert!(
            !owned_string_remove.has_errors(),
            "expected Buffer<OwnedString> remove_move to be accepted, got {:?}",
            owned_string_remove.diagnostics
        );

        let owned_string_remove_into = check(
            "module test; fn mutate(values: write Buffer<OwnedString>, output: write OwnedString) { let moved: Bool = buffer_remove_move_into(values, 0usize, output); let status: Int32 = buffer_remove_move_into_status(values, 0usize, output); }",
        );
        assert!(
            !owned_string_remove_into.has_errors(),
            "expected Buffer<OwnedString> remove_move_into to be accepted, got {:?}",
            owned_string_remove_into.diagnostics
        );

        let owned_string_pop_into = check(
            "module test; fn mutate(values: write Buffer<OwnedString>, output: write OwnedString) { let moved: Bool = buffer_pop_move_into(values, output); let status: Int32 = buffer_pop_move_into_status(values, output); }",
        );
        assert!(
            !owned_string_pop_into.has_errors(),
            "expected Buffer<OwnedString> pop_move_into to be accepted, got {:?}",
            owned_string_pop_into.diagnostics
        );

        let nested_owned_string_remove_into = check(
            "module test; fn mutate(values: write Buffer<Buffer<OwnedString>>, output: Buffer<OwnedString>) { let moved: Bool = buffer_remove_move_into(values, 0usize, output); let status: Int32 = buffer_remove_move_into_status(values, 0usize, output); }",
        );
        assert!(
            !nested_owned_string_remove_into.has_errors(),
            "expected nested Buffer<OwnedString> remove_move_into to be accepted, got {:?}",
            nested_owned_string_remove_into.diagnostics
        );

        let owned_string_resize = check(
            "module test; fn mutate(values: write Buffer<OwnedString>) { let resized: Bool = buffer_resize_move(values, 2usize); let cleared: Bool = buffer_clear_move(values); }",
        );
        assert!(
            !owned_string_resize.has_errors(),
            "expected Buffer<OwnedString> resize/clear to be accepted, got {:?}",
            owned_string_resize.diagnostics
        );

        let owned_string_remove_drop = check(
            "module test; fn mutate(values: write Buffer<OwnedString>) { let removed: Bool = buffer_remove_drop(values, 0usize); let status: Int32 = buffer_remove_drop_status(values, 0usize); }",
        );
        assert!(
            !owned_string_remove_drop.has_errors(),
            "expected Buffer<OwnedString> remove_drop to be accepted, got {:?}",
            owned_string_remove_drop.diagnostics
        );

        let owned_string_option_carrier = check(
            "module test; fn mutate(values: write Buffer<Option<OwnedString>>, item: OwnedString) { let state: Option<OwnedString> = Some(item); buffer_append(values, state); let resized: Int32 = buffer_resize_move_status(values, 1usize); let cleared: Int32 = buffer_clear_move_status(values); }",
        );
        assert!(
            !owned_string_option_carrier.has_errors(),
            "expected Option<OwnedString> carrier to be accepted, got {:?}",
            owned_string_option_carrier.diagnostics
        );

        let owned_string_result_carrier = check(
            "module test; fn mutate(values: write Buffer<Result<OwnedString, Int32>>, item: OwnedString) { let state: Result<OwnedString, Int32> = Ok(item); buffer_append(values, state); let removed: Bool = buffer_remove_drop(values, 0usize); }",
        );
        assert!(
            !owned_string_result_carrier.has_errors(),
            "expected Result<OwnedString, Int32> carrier to be accepted, got {:?}",
            owned_string_result_carrier.diagnostics
        );

        let nested_pop = check(
            "module test; fn take(values: write Buffer<Buffer<Int32>>) -> Result<Buffer<Int32>, Int32> { return buffer_pop(values); }",
        );
        assert!(!nested_pop.has_errors(), "{:?}", nested_pop.diagnostics);

        let scalar_pop = check(
            "module test; fn take(values: write Buffer<Int32>) { let candidate: Result<Buffer<Int32>, Int32> = buffer_pop(values); }",
        );
        assert!(
            scalar_pop
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0301"),
            "expected nested pop diagnostic, got {:?}",
            scalar_pop.diagnostics
        );

        let nested_remove_move = check(
            "module test; fn take(values: write Buffer<Buffer<Int32>>) -> Result<Buffer<Int32>, Int32> { return buffer_remove_move(values, 0usize); }",
        );
        assert!(
            !nested_remove_move.has_errors(),
            "{:?}",
            nested_remove_move.diagnostics
        );

        let scalar_remove_move = check(
            "module test; fn take(values: write Buffer<Int32>) { let candidate: Result<Buffer<Int32>, Int32> = buffer_remove_move(values, 0usize); }",
        );
        assert!(
            scalar_remove_move
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0301"),
            "expected nested remove_move diagnostic, got {:?}",
            scalar_remove_move.diagnostics
        );

        let owning_record_remove_move_into = check(
            "module test; @repr(C) struct Entry { values: Buffer<Int32>, id: Int32 } fn mutate(values: write Buffer<Entry>, output: write Entry) { let moved: Bool = buffer_remove_move_into(values, 0usize, output); let status: Int32 = buffer_remove_move_into_status(values, 0usize, output); }",
        );
        assert!(
            !owning_record_remove_move_into.has_errors(),
            "expected owning record remove_move_into to be accepted, got {:?}",
            owning_record_remove_move_into.diagnostics
        );

        let owning_enum_remove_move_into = check(
            "module test; @repr(C) enum Event { Idle, Ready(Buffer<Int32>), Nested(Buffer<Buffer<Int32>>), Done } fn mutate(values: write Buffer<Event>, output: write Event) { let moved: Bool = buffer_remove_move_into(values, 0usize, output); let status: Int32 = buffer_remove_move_into_status(values, 0usize, output); }",
        );
        assert!(
            !owning_enum_remove_move_into.has_errors(),
            "expected owning enum remove_move_into to be accepted, got {:?}",
            owning_enum_remove_move_into.diagnostics
        );

        let nested_remove_move_into = check(
            "module test; fn mutate(values: write Buffer<Buffer<Int32>>, output: Buffer<Int32>) { let moved: Bool = buffer_remove_move_into(values, 0usize, output); }",
        );
        assert!(
            !nested_remove_move_into.has_errors(),
            "expected nested buffer remove_move_into to be accepted, got {:?}",
            nested_remove_move_into.diagnostics
        );

        let borrowed_nested_remove_move_into = check(
            "module test; fn mutate(values: write Buffer<Buffer<Int32>>, output: write Buffer<Int32>) { let moved: Bool = buffer_remove_move_into(values, 0usize, output); }",
        );
        assert!(
            borrowed_nested_remove_move_into
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0301"),
            "expected nested move_into to require an owned output descriptor, got {:?}",
            borrowed_nested_remove_move_into.diagnostics
        );

        let copy_safe_record_remove_move_into = check(
            "module test; @repr(C) struct Entry { id: Int32 } fn mutate(values: write Buffer<Entry>, output: write Entry) { let moved: Bool = buffer_remove_move_into(values, 0usize, output); }",
        );
        assert!(
            copy_safe_record_remove_move_into.has_errors(),
            "expected remove_move_into to require an owning record, got {:?}",
            copy_safe_record_remove_move_into.diagnostics
        );

        let nested_insert_move = check(
            "module test; fn mutate(values: write Buffer<Buffer<Int32>>, inner: Buffer<Int32>) { let inserted: Bool = buffer_insert_move(values, 0usize, inner); }",
        );
        assert!(
            !nested_insert_move.has_errors(),
            "expected nested move-only insert to be accepted, got {:?}",
            nested_insert_move.diagnostics
        );

        let scalar_insert_move = check(
            "module test; fn mutate(values: write Buffer<Int32>, inner: Int32) { let inserted: Bool = buffer_insert_move(values, 0usize, inner); }",
        );
        assert!(
            scalar_insert_move
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0301"),
            "expected nested insert_move diagnostic, got {:?}",
            scalar_insert_move.diagnostics
        );

        let owning_record_insert_move_from = check(
            "module test; @repr(C) struct Entry { values: Buffer<Int32>, id: Int32 } fn mutate(values: write Buffer<Entry>, incoming: Entry, incoming_status: Entry) { let moved: Bool = buffer_insert_move_from(values, 0usize, incoming); let status: Int32 = buffer_insert_move_from_status(values, 0usize, incoming_status); }",
        );
        assert!(
            !owning_record_insert_move_from.has_errors(),
            "expected owning record insert_move_from to be accepted, got {:?}",
            owning_record_insert_move_from.diagnostics
        );

        let scalar_insert_move_from = check(
            "module test; fn mutate(values: write Buffer<Int32>, incoming: Int32) { let moved: Bool = buffer_insert_move_from(values, 0usize, incoming); }",
        );
        assert!(
            scalar_insert_move_from
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0301"),
            "expected scalar insert_move_from diagnostic, got {:?}",
            scalar_insert_move_from.diagnostics
        );

        let nested_remove = check(
            "module test; fn mutate(values: write Buffer<Buffer<Int32>>) { buffer_remove(values, 0usize); }",
        );
        assert!(
            nested_remove
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0301"),
            "expected move-only remove diagnostic, got {:?}",
            nested_remove.diagnostics
        );

        let nested_resize_move = check(
            "module test; fn mutate(values: write Buffer<Buffer<Int32>>) { let resized: Bool = buffer_resize_move(values, 2usize); let resized_status: Int32 = buffer_resize_move_status(values, 0usize); }",
        );
        assert!(
            !nested_resize_move.has_errors(),
            "expected move-only resize to be accepted, got {:?}",
            nested_resize_move.diagnostics
        );

        let scalar_resize_move = check(
            "module test; fn mutate(values: write Buffer<Int32>) { buffer_resize_move(values, 2usize); }",
        );
        assert!(
            scalar_resize_move
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0301"),
            "expected scalar move-only resize diagnostic, got {:?}",
            scalar_resize_move.diagnostics
        );

        let nested_composite = check(
            "module test; fn mutate() { let created: Result<Buffer<[Buffer<Int32>; 1]>, Int32> = buffer_create(1usize); }",
        );
        assert!(
            !nested_composite.has_errors(),
            "expected fixed owning array element to be accepted, got {:?}",
            nested_composite.diagnostics
        );

        let statuses = check(
            "module test; fn mutate(values: write Buffer<UInt8>) { let a: Int32 = buffer_reserve_status(values, 2usize); let b: Int32 = buffer_append_status(values, 7u8); let c: Int32 = buffer_insert_status(values, 0usize, 3u8); let d: Int32 = buffer_remove_status(values, 0usize); let e: Int32 = buffer_resize_status(values, 0usize); let f: Int32 = buffer_clear_status(values); let g: Bool = buffer_clear(values); }",
        );
        assert!(!statuses.has_errors(), "{:?}", statuses.diagnostics);

        let nested_clear = check(
            "module test; fn mutate(values: write Buffer<Buffer<Int32>>) { buffer_clear(values); }",
        );
        assert!(
            nested_clear
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0301"),
            "expected nested clear to require a move-aware contract, got {:?}",
            nested_clear.diagnostics
        );

        let nested_record_clear = check(
            "module test; @repr(C) struct Entry { values: Buffer<Int32> } fn mutate(values: write Buffer<Buffer<Entry>>) { buffer_clear_move(values); }",
        );
        assert!(
            !nested_record_clear.has_errors(),
            "expected nested record clear to use the field-table contract, got {:?}",
            nested_record_clear.diagnostics
        );

        let direct_record_resize = check(
            "module test; @repr(C) struct Entry { id: Int32, values: Buffer<Int32> } fn mutate(values: write Buffer<Entry>) { let ok: Bool = buffer_resize_move(values, 2usize); let status: Int32 = buffer_resize_move_status(values, 0usize); }",
        );
        assert!(
            !direct_record_resize.has_errors(),
            "expected direct record resize to use the field-table contract, got {:?}",
            direct_record_resize.diagnostics
        );

        let direct_record_clear = check(
            "module test; @repr(C) struct Entry { id: Int32, values: Buffer<Int32> } fn mutate(values: write Buffer<Entry>) { let ok: Bool = buffer_clear_move(values); let status: Int32 = buffer_clear_move_status(values); }",
        );
        assert!(
            !direct_record_clear.has_errors(),
            "expected direct record clear to use the field-table contract, got {:?}",
            direct_record_clear.diagnostics
        );

        let non_c_record_clear = check(
            "module test; struct Entry { values: Buffer<Int32> } fn mutate(values: write Buffer<Buffer<Entry>>) { buffer_clear_move(values); }",
        );
        assert!(
            non_c_record_clear
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0301"),
            "expected non-C record clear diagnostic, got {:?}",
            non_c_record_clear.diagnostics
        );

        let invalid = check(
            "module test; fn mutate(values: write Buffer<String>) { buffer_append(values, \"text\"); }",
        );
        assert!(
            invalid
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0301"),
            "expected copy-safe element diagnostic, got {:?}",
            invalid.diagnostics
        );
    }

    #[test]
    fn validates_disjoint_borrow_contract_shape() {
        let valid = check(
            "module test; @disjoint fn update(a: write Slice<Int32>, b: read Slice<Int32>) { }",
        );
        assert!(!valid.has_errors(), "{:?}", valid.diagnostics);

        let invalid_count = check("module test; @disjoint fn update(a: write Slice<Int32>) { }");
        assert!(
            invalid_count
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0811"),
            "expected J0811, got {:?}",
            invalid_count.diagnostics
        );

        let invalid_arguments = check(
            "module test; @disjoint(a: true) fn update(a: write Slice<Int32>, b: read Slice<Int32>) { }",
        );
        assert!(
            invalid_arguments
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0810"),
            "expected J0810, got {:?}",
            invalid_arguments.diagnostics
        );
    }

    #[test]
    fn rejects_loop_control_outside_loop() {
        let output = check("module test; fn bad() { break continue }");
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0318")
        );
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0319")
        );
    }

    #[test]
    fn reports_annotation_and_return_mismatches() {
        let output = check("module test; fn wrong() -> Bool { let value: Int32 = true; return 1 }");
        assert!(output.has_errors());
        assert_eq!(
            output
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "J0301")
                .count(),
            2
        );
    }

    #[test]
    fn exports_repr_c_layout_metadata_and_rejects_non_abi_fields() {
        let output =
            check("module test; @repr(C) pub struct Vec3 { x: Float32, y: Float32, z: Float32 }");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        assert!(output.nominal_layouts.iter().any(|layout| {
            layout.repr == jadren_types::AbiRepr::C
                && matches!(layout.kind, jadren_types::NominalLayoutKind::Record { .. })
        }));

        let vector = check("module test; @repr(C) struct Tile { lanes: Float8 }");
        assert!(!vector.has_errors(), "{:?}", vector.diagnostics);

        let invalid = check("module test; @repr(C) struct Bad { text: String }");
        assert!(
            invalid
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0801")
        );
        let payload_enum = check("module test; @repr(C) enum Event { Value(Int32), Empty }");
        assert!(!payload_enum.has_errors(), "{:?}", payload_enum.diagnostics);
        assert!(payload_enum.nominal_layouts.iter().any(|layout| {
            layout.repr == jadren_types::AbiRepr::C
                && matches!(layout.kind, jadren_types::NominalLayoutKind::Enum { .. })
        }));

        let payload_buffer = check(
            "module test; @repr(C) enum Event { Value(Int32), Empty } fn mutate(values: write Buffer<Event>) { let event: Event = Event.Value(1); buffer_append(values, event); }",
        );
        assert!(
            !payload_buffer.has_errors(),
            "{:?}",
            payload_buffer.diagnostics
        );

        let invalid_payload = check("module test; @repr(C) enum Bad { Value(String) }");
        assert!(
            invalid_payload
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0803")
        );

        let generic_record = check(
            "module test; @repr(C) struct Box<T> { value: T } @repr(C) struct Frame<T> { first: Box<T>, samples: [T; 2] } fn mutate(values: write Buffer<Frame<Int64>>) { let frame: Frame<Int64> = Frame { first: Box { value: 42 as Int64 }, samples: [7 as Int64, 9 as Int64] }; buffer_append(values, frame); }",
        );
        assert!(
            !generic_record.has_errors(),
            "{:#?}",
            generic_record.diagnostics
        );

        let generic_nested_owning_record = check(
            "module test; @repr(C) struct Frame<T> { payload: T, marker: Int32 } fn mutate(values: write Buffer<Frame<Buffer<OwnedString>>>, inner: Buffer<OwnedString>) { let item: Frame<Buffer<OwnedString>> = Frame { payload: inner, marker: 42 }; buffer_append(values, item); }",
        );
        assert!(
            !generic_nested_owning_record.has_errors(),
            "expected generic nested owning record to be accepted, got {:#?}",
            generic_nested_owning_record.diagnostics
        );

        let generic_record_carrier = check(
            "module test; @repr(C) struct Frame<T> { payload: T, marker: Int32 } fn mutate(values: write Buffer<Frame<Option<Buffer<OwnedString>>>>, inner: Buffer<OwnedString>) { let item: Frame<Option<Buffer<OwnedString>>> = Frame { payload: Some(inner), marker: 42 }; buffer_append(values, item); }",
        );
        assert!(
            !generic_record_carrier.has_errors(),
            "expected generic carrier record to be accepted, got {:#?}",
            generic_record_carrier.diagnostics
        );

        let generic_owning = check(
            "module test; @repr(C) struct Box<T> { value: T } fn mutate(values: write Buffer<Box<String>>) { let item: Box<String> = Box { value: \"owned\" }; buffer_append(values, item); }",
        );
        assert!(
            generic_owning
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0301"),
            "{:#?}",
            generic_owning.diagnostics
        );

        let owning_record = check(
            "module test; @repr(C) struct Entry { values: Buffer<Int32>, id: Int32 } fn mutate(values: write Buffer<Entry>, inner: Buffer<Int32>) { let item: Entry = Entry { values: inner, id: 7 }; buffer_append(values, item); }",
        );
        assert!(
            !owning_record.has_errors(),
            "{:#?}",
            owning_record.diagnostics
        );

        let owning_record_carrier = check(
            "module test; @repr(C) struct Entry { maybe: Option<Buffer<Int32>>, outcome: Result<Buffer<Int32>, Buffer<Int32>>, id: Int32 } fn mutate(values: write Buffer<Entry>, maybe: Option<Buffer<Int32>>, outcome: Result<Buffer<Int32>, Buffer<Int32>>) { let item: Entry = Entry { maybe: maybe, outcome: outcome, id: 7 }; buffer_append(values, item); }",
        );
        assert!(
            !owning_record_carrier.has_errors(),
            "expected tagged carrier fields to be accepted, got {:#?}",
            owning_record_carrier.diagnostics
        );

        let owning_record_array = check(
            "module test; @repr(C) struct Entry { slots: [Buffer<Int32>; 2], id: Int32 } fn mutate(values: write Buffer<Entry>, first: Buffer<Int32>, second: Buffer<Int32>) { let item: Entry = Entry { slots: [first, second], id: 7 }; buffer_append(values, item); }",
        );
        assert!(
            !owning_record_array.has_errors(),
            "expected fixed array owning record to be accepted, got {:#?}",
            owning_record_array.diagnostics
        );

        let nested_owning_record_array = check(
            "module test; @repr(C) struct Entry { slots: [Buffer<Int32>; 2], id: Int32 } fn mutate(values: write Buffer<Buffer<Entry>>) { buffer_resize_move(values, 1usize); }",
        );
        assert!(
            !nested_owning_record_array.has_errors(),
            "expected nested fixed array owning record resize to be accepted, got {:#?}",
            nested_owning_record_array.diagnostics
        );

        let nested_owning_record = check(
            "module test; @repr(C) struct Inner { values: Buffer<Int32>, id: Int32 } @repr(C) struct Entry { nested: Inner, marker: Int32 } fn mutate(values: write Buffer<Entry>, inner: Buffer<Int32>) { let nested: Inner = Inner { values: inner, id: 7 }; let item: Entry = Entry { nested: nested, marker: 42 }; buffer_append(values, item); }",
        );
        assert!(
            !nested_owning_record.has_errors(),
            "{:#?}",
            nested_owning_record.diagnostics
        );

        let nested_owning_record_chain = check(
            "module test; @repr(C) struct Inner { values: Buffer<Int32>, id: Int32 } @repr(C) struct Entry { nested: Inner, marker: Int32 } fn mutate(values: write Buffer<Buffer<Entry>>, inner: Buffer<Entry>) { buffer_append(values, inner); }",
        );
        assert!(
            !nested_owning_record_chain.has_errors(),
            "expected nested owning record chain to be accepted, got {:#?}",
            nested_owning_record_chain.diagnostics
        );

        let nested_record_chain = check(
            "module test; @repr(C) struct Inner { values: Buffer<Int32>, id: Int32 } @repr(C) struct Entry { nested: Inner, marker: Int32 } fn mutate(values: write Buffer<Buffer<Entry>>) { buffer_resize_move(values, 1usize); }",
        );
        assert!(
            !nested_record_chain.has_errors(),
            "expected path-aware nested record resize to be accepted, got {:#?}",
            nested_record_chain.diagnostics
        );

        let nested_record_chain_status = check(
            "module test; @repr(C) struct Inner { values: Buffer<Int32>, id: Int32 } @repr(C) struct Entry { nested: Inner, marker: Int32 } fn mutate(values: write Buffer<Buffer<Entry>>) -> Int32 { buffer_resize_move_status(values, 1usize) }",
        );
        assert!(
            !nested_record_chain_status.has_errors(),
            "expected path-aware nested record status resize to be accepted, got {:#?}",
            nested_record_chain_status.diagnostics
        );

        let direct_buffer_carrier_remove = check(
            "module test; fn mutate(values: write Buffer<Option<Buffer<Int32>>>) { buffer_remove_drop(values, 0usize); }",
        );
        assert!(
            !direct_buffer_carrier_remove.has_errors(),
            "expected direct Buffer carrier remove to be accepted, got {:#?}",
            direct_buffer_carrier_remove.diagnostics
        );

        let direct_buffer_carrier_clear = check(
            "module test; fn mutate(values: write Buffer<Result<Buffer<Int32>, Buffer<Int32>>>) { buffer_clear_move(values); }",
        );
        assert!(
            !direct_buffer_carrier_clear.has_errors(),
            "expected direct multi-carrier clear to be accepted, got {:#?}",
            direct_buffer_carrier_clear.diagnostics
        );

        let nested_buffer_carrier_resize = check(
            "module test; fn mutate(values: write Buffer<Buffer<Option<Buffer<Int32>>>>) { buffer_resize_move(values, 1usize); }",
        );
        assert!(
            !nested_buffer_carrier_resize.has_errors(),
            "expected nested Buffer carrier resize to be accepted, got {:#?}",
            nested_buffer_carrier_resize.diagnostics
        );

        let nested_record_carrier_resize = check(
            "module test; @repr(C) struct Entry { payload: Option<Buffer<Int32>> } fn mutate(values: write Buffer<Buffer<Entry>>) { buffer_resize_move(values, 1usize); }",
        );
        assert!(
            !nested_record_carrier_resize.has_errors(),
            "expected nested carrier record resize to be accepted, got {:#?}",
            nested_record_carrier_resize.diagnostics
        );

        let direct_record_carrier_clear = check(
            "module test; @repr(C) struct Entry { payload: Option<Buffer<Int32>> } fn mutate(values: write Buffer<Entry>) { buffer_clear_move(values); }",
        );
        assert!(
            !direct_record_carrier_clear.has_errors(),
            "expected direct carrier record clear to be accepted, got {:#?}",
            direct_record_carrier_clear.diagnostics
        );

        let owning_record_copy_remove = check(
            "module test; @repr(C) struct Entry { values: Buffer<Int32>, id: Int32 } fn mutate(values: write Buffer<Entry>) { buffer_remove(values, 0usize); }",
        );
        assert!(
            owning_record_copy_remove
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0301"),
            "expected move-only record remove diagnostic, got {:#?}",
            owning_record_copy_remove.diagnostics
        );

        let owning_record_drop_remove = check(
            "module test; @repr(C) struct Entry { values: Buffer<Int32>, id: Int32 } fn mutate(values: write Buffer<Entry>) { buffer_remove_drop(values, 0usize); let status: Int32 = buffer_remove_drop_status(values, 0usize); }",
        );
        assert!(
            !owning_record_drop_remove.has_errors(),
            "expected explicit owning record remove_drop to be accepted, got {:#?}",
            owning_record_drop_remove.diagnostics
        );

        let nested_owning_record_drop_remove = check(
            "module test; @repr(C) struct Entry { values: Buffer<Int32>, id: Int32 } fn mutate(values: write Buffer<Buffer<Entry>>) { buffer_remove_drop(values, 0usize); let status: Int32 = buffer_remove_drop_status(values, 0usize); }",
        );
        assert!(
            !nested_owning_record_drop_remove.has_errors(),
            "expected nested owning record remove_drop to be accepted, got {:#?}",
            nested_owning_record_drop_remove.diagnostics
        );

        let owning_record_carrier_drop_remove = check(
            "module test; @repr(C) struct Entry { payload: Option<Buffer<Int32>>, id: Int32 } fn mutate(values: write Buffer<Entry>) { buffer_remove_drop(values, 0usize); }",
        );
        assert!(
            !owning_record_carrier_drop_remove.has_errors(),
            "expected tagged owning record remove_drop to be accepted, got {:#?}",
            owning_record_carrier_drop_remove.diagnostics
        );
    }

    #[test]
    fn preserves_record_and_component_field_order_in_layout_metadata() {
        let output = check(
            "module test; @repr(C) pub struct Pair { z: Int32, a: Float32 } @repr(C) component Position { y: Float32, x: Float32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let orders: Vec<Vec<_>> = output
            .nominal_layouts
            .iter()
            .filter_map(|layout| match &layout.kind {
                jadren_types::NominalLayoutKind::Record { fields } => {
                    Some(fields.iter().map(|field| field.name.as_str()).collect())
                }
                jadren_types::NominalLayoutKind::Enum { .. } => None,
            })
            .collect();
        assert!(orders.iter().any(|fields| fields == &["z", "a"]));
        assert!(orders.iter().any(|fields| fields == &["y", "x"]));
    }

    #[test]
    fn validates_c_export_metadata() {
        let output = check(
            "module test; @export(name: \"jadren_add\", abi: \"C\") fn add(a: Int32) -> Int32 { return a }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);

        let invalid = check("module test; @export(name: \"bad-name\", abi: \"Rust\") fn add() {}");
        let codes: Vec<_> = invalid
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect();
        assert!(codes.contains(&"J0805"));
        assert!(codes.contains(&"J0806"));

        let duplicate = check(
            "module test; @export(name: \"same\", abi: \"C\") fn first() {} @export(name: \"same\", abi: \"C\") fn second() {}",
        );
        assert!(
            duplicate
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0807")
        );
    }

    #[test]
    fn infers_arrays_and_conditional_results() {
        let output = check(
            "module test; fn choose(flag: Bool) { let values = [1, 2, 3]; let x = if flag { 1 } else { 2 }; print(x) }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        assert!(output.expressions.iter().any(|expression| matches!(
            output.types.kind(expression.ty),
            Some(TypeKind::Array { length: 3, .. })
        )));
    }

    #[test]
    fn checks_forward_local_function_calls() {
        let output = check(
            "module test; fn main() { let value = add(1, 2); print(value) } fn add(a: Int32, b: Int32) -> Int32 { return a + b }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn reports_call_arity_argument_and_callable_errors() {
        let output = check(
            "module test; fn main() { add(true); let value = 1; value() } fn add(a: Int32, b: Int32) -> Int32 { return a + b }",
        );
        let codes: Vec<_> = output
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect();
        assert!(codes.contains(&"J0301"));
        assert!(codes.contains(&"J0304"));
        assert!(codes.contains(&"J0305"));
    }

    #[test]
    fn checks_builtin_arity_and_assert_eq_types() {
        let output = check("module test; fn main() { print(); assert_eq(1, true) }");
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0304")
        );
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0301")
        );
    }

    #[test]
    fn checks_windows_ui_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { ui_window(\"UI\", 640, 480, 0xF6F8FCu32); ui_top_bar(48, 0x252526u32); ui_text(\"J\", 12, 8, 28, 28, 0xFFFFFFu32, 0x168EF5u32, 6); ui_menu_item(\"File\", \"File menu selected\", 52, 8, 56, 28, 0xFFFFFFu32, 0x252526u32, 6); ui_icon_button(\"?\", \"Search selected\", 12, 8, 28, 28, 0xFFFFFFu32, 0x252526u32, 6); ui_label(\"Text\", 20, 60, 200, 32, 0x111827u32, 0xFFFFFFu32, 12); ui_status(\"Ready\", 20, 110, 200, 32, 0x111827u32, 0xDCFCE7u32, 16); ui_button(\"Run\", \"Done\", 20, 160, 120, 40, 0xFFFFFFu32, 0x168EF5u32, 16); ui_event_button(\"Event\", 1, 150, 160, 120, 40, 0xFFFFFFu32, 0x168EF5u32, 16); ui_set_status(\"Updated\"); ui_set_button_text(1, \"Event\"); ui_set_button_enabled(1, 1); ui_state_bind(1, 0, 0); ui_state_set(0, ui_state_get(0)); ui_toggle_button(\"Pin\", \"Pinned\", 280, 160, 120, 40, 0xFFFFFFu32, 0x18C964u32, 16); ui_disabled_button(\"Unavailable\", 410, 160, 120, 40, 0xFFFFFFu32, 0x64748Bu32, 16); ui_close_button(\"Close\", 540, 160, 80, 40, 0xFFFFFFu32, 0x0B6FC8u32, 16); return ui_run() }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_windows_ui_layout_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { ui_window(\"UI\", 640, 480, 0xF6F8FCu32); ui_column(20, 64, 600, 360, 16, 12, 0, 1); ui_panel(600, 120, 0xFFFFFFu32, 16, 12, 8, 0, 1); ui_layout_label(\"Heading\", 300, 28, 0x111827u32, 0xFFFFFFu32, 0, 1); ui_layout_status(\"Ready\", 300, 32, 0x111827u32, 0xDCFCE7u32, 12, 1); ui_layout_end(); ui_row(600, 48, 0, 12, 1, 1); ui_layout_event_button(\"Run\", 1, 160, 40, 0xFFFFFFu32, 0x168EF5u32, 12, 0); ui_layout_event_button(\"More\", 2, 160, 40, 0xFFFFFFu32, 0x18C964u32, 12, 0); ui_layout_end(); ui_layout_end(); return ui_run() }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_retained_ui_contract_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { let root: Int32 = ui_app_begin(\"UI\", 640, 420, 0xF6F8FCu32); let panel: Int32 = ui_app_panel(root, 560, 260, 0xFFFFFFu32, 16, 12, 8, 0, 1); let label: Int32 = ui_app_label(panel, \"Heading\", 480, 32, 0x111827u32, 0xFFFFFFu32, 8, 1); let input: Int32 = ui_app_text_input(panel, \"Name\", 2, 480, 38, 0x111827u32, 0xFFFFFFu32, 8, 1); let checkbox: Int32 = ui_app_checkbox(panel, \"Enabled\", 3, 220, 38, 0x111827u32, 0x168EF5u32, 8, 0, 1); let select: Int32 = ui_app_select(panel, 4, 240, 38, 0x111827u32, 0xFFFFFFu32, 8, 0); let option_one: Bool = ui_app_select_option(select, \"One\"); let option_two: Bool = ui_app_select_option(select, \"Two\"); let selected: Bool = ui_app_select_set_index(select, 1); let index: Int32 = ui_app_select_index(select); let list: Int32 = ui_app_list(panel, 5, 240, 100, 0x111827u32, 0xFFFFFFu32, 8, 0); let item: Bool = ui_app_list_item(list, \"Item\"); let count: Int32 = ui_app_list_count(list); let list_index: Int32 = ui_app_list_index(list); let table: Int32 = ui_app_table(panel, 6, 480, 120, 0x111827u32, 0xFFFFFFu32, 8, 1); let column: Bool = ui_app_table_column(table, 0, \"Name\", 180); let cell: Bool = ui_app_table_cell(table, 0, 0, \"Item\"); let rows: Int32 = ui_app_table_row_count(table); let table_index: Int32 = ui_app_table_selected_row(table); let table_selected: Bool = ui_app_table_set_selected_row(table, 0); let table_bound: Bool = ui_app_table_bind_app(table, 0, 1); let table_refreshed: Bool = ui_app_table_refresh(table); let button: Int32 = ui_app_button(panel, \"Run\", 1, 160, 40, 0xFFFFFFu32, 0x168EF5u32, 12, 0); let panel_closed: Bool = ui_app_end(panel); let root_closed: Bool = ui_app_end(root); if panel_closed && root_closed && label > 0 && input > 0 && checkbox > 0 && select > 0 && option_one && option_two && selected && index == 1 && list > 0 && item && count == 1 && list_index == -1 && table > 0 && column && cell && rows == 1 && table_index == -1 && table_selected && table_bound && table_refreshed && button > 0 { return ui_app_run() } return 1 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_retained_row_contract_signature() {
        let output = check(
            "module test; fn main() -> Int32 { let root: Int32 = ui_app_begin(\"UI\", 640, 420, 0xF6F8FCu32); let row: Int32 = ui_app_row(root, 600, 80, 8, 12, 0, 1); let button: Int32 = ui_app_button(row, \"Run\", 1, 140, 40, 0xFFFFFFu32, 0x168EF5u32, 8, 0); let row_closed: Bool = ui_app_end(row); let root_closed: Bool = ui_app_end(root); if row > 0 && button > 0 && row_closed && root_closed { return ui_app_run() } return 1 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_retained_top_bar_contract_signature() {
        let output = check(
            "module test; fn main() -> Int32 { let root: Int32 = ui_app_begin(\"UI\", 640, 420, 0xF6F8FCu32); let top_bar: Int32 = ui_app_top_bar(root, 600, 54, 0x1F2937u32, 0, 8, 12, 0, 1); let button: Int32 = ui_app_button(top_bar, \"Menu\", 1, 140, 40, 0xFFFFFFu32, 0x374151u32, 8, 0); let bar_closed: Bool = ui_app_end(top_bar); let root_closed: Bool = ui_app_end(root); if top_bar > 0 && button > 0 && bar_closed && root_closed { return ui_app_run() } return 1 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_retained_popup_menu_contract_signature() {
        let output = check(
            "module test; fn main() -> Int32 { let root: Int32 = ui_app_begin(\"UI\", 640, 420, 0xF6F8FCu32); let top_bar: Int32 = ui_app_top_bar(root, 600, 54, 0x1F2937u32, 0, 8, 12, 0, 1); let menu: Int32 = ui_app_menu(top_bar, \"File\", 96, 38, 0xFFFFFFu32, 0x374151u32, 8, 0); let item: Bool = ui_app_menu_item(menu, \"Open\", 21); let item_two: Bool = ui_app_menu_item(menu, \"Save\", 22); let bar_closed: Bool = ui_app_end(top_bar); let root_closed: Bool = ui_app_end(root); if menu > 0 && item && item_two && bar_closed && root_closed { return ui_app_run() } return 1 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_retained_tooltip_contract_signature() {
        let output = check(
            "module test; fn main() -> Int32 { let root: Int32 = ui_app_begin(\"UI\", 640, 420, 0xF6F8FCu32); let panel: Int32 = ui_app_panel(root, 560, 260, 0xFFFFFFu32, 16, 12, 8, 0, 1); let button: Int32 = ui_app_button(panel, \"Run\", 7, 160, 40, 0xFFFFFFu32, 0x168EF5u32, 12, 0); let tip: Bool = ui_app_tooltip(button, \"Run action\", 240, 42, 0xFFFFFFu32, 0x252526u32, 8); let panel_closed: Bool = ui_app_end(panel); let root_closed: Bool = ui_app_end(root); if button > 0 && tip && panel_closed && root_closed { return ui_app_run() } return 1 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_windows_ui_form_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { ui_window(\"UI\", 640, 480, 0xF6F8FCu32); ui_menu(\"Subor\", 1, 20, 20, 80, 32, 0xFFFFFFu32, 0x252526u32, 6); ui_menu_option(1, \"Otvorit\", 21); ui_checkbox(\"Zapnut\", 10, 20, 70, 180, 36, 0xFFFFFFu32, 0x168EF5u32, 8); ui_tooltip(10, \"Zapne ukazku\", 180, 32, 0xFFFFFFu32, 0x252526u32, 6); ui_switch(\"Tmavy rezim\", 11, 220, 70, 180, 36, 0xFFFFFFu32, 0x18C964u32, 18); ui_text_input(\"Meno\", 12, 20, 130, 220, 36, 0x111827u32, 0xFFFFFFu32, 8); ui_set_input_text(12, \"Nove meno\"); ui_set_input_enabled(12, 1); ui_set_checked(10, ui_checked(10)); ui_select(13, 20, 190, 220, 36, 0x111827u32, 0xFFFFFFu32); ui_select_option(13, \"Prva\"); ui_select_set_index(13, ui_select_index(13)); ui_scroll_panel(\"Text\", 260, 190, 180, 100, 0x111827u32, 0xFFFFFFu32, 8); ui_list(14, 460, 190, 160, 100, 0x111827u32, 0xFFFFFFu32); ui_list_item(14, \"Prva\"); ui_list_set_item(14, 0, \"Prva upravena\"); ui_list_set_index(14, ui_list_index(14)); ui_list_count(14); ui_list_clear(14); ui_table(15, 20, 300, 500, 120, 0x111827u32, 0xFFFFFFu32); ui_table_column(15, 0, \"Nazov\", 180); ui_table_column(15, 1, \"Stav\", 120); ui_table_cell(15, 0, 0, \"Prva\"); ui_table_cell(15, 0, 1, \"Otvorene\"); ui_table_row_count(15); ui_table_selected_row(15); ui_table_set_selected_row(15, 0); ui_table_clear(15); return ui_run() }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_windows_ui_collection_read_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { var output: [UInt8; 32] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let list: UIntSize = ui_list_read_item(20, 0, output); let cell: UIntSize = ui_table_read_cell(30, 0, 1, output); return (list + cell) as Int32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_retained_table_read_signature() {
        let output = check(
            "module test; fn main() -> Int32 { var output: [UInt8; 4] = [0u8, 0u8, 0u8, 0u8]; let copied: UIntSize = ui_app_table_read_cell(31, 0, 0, output); return copied as Int32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_table_sort_filter_ui_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { let root: Int32 = ui_app_begin(\"UI\", 640, 420, 0xF6F8FCu32); let panel: Int32 = ui_app_panel(root, 560, 260, 0xFFFFFFu32, 16, 12, 8, 0, 1); let source: Int32 = ui_app_table(panel, 30, 240, 120, 0x111827u32, 0xFFFFFFu32, 8, 1); let destination: Int32 = ui_app_table(panel, 31, 240, 120, 0x111827u32, 0xFFFFFFu32, 8, 1); let sorted: Bool = ui_app_table_sort_text(source, 0, false); let exact: Bool = ui_app_table_filter_text(source, 1, 1, \"Open\"); let contains: Bool = ui_app_table_filter_text_ex(source, 1, 1, \"pen\", 1); ui_table_sort_text(30, 0, true); ui_table_filter_text(30, 1, 1, \"Open\"); ui_table_filter_text_ex(30, 1, 1, \"pen\", 1); if root > 0 && panel > 0 && source > 0 && destination > 0 && sorted && exact && contains { return 0 } return 1 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_windows_ui_app_collection_binding_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { ui_list_bind_app(20, 0); ui_list_refresh_app(20); ui_table_bind_app(30, 0, 2); ui_table_refresh_app(30); ui_refresh_bindings(); return 0 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_ui_state_binding_signature() {
        let output = check(
            "module test; fn main() -> Int32 { ui_state_bind(10, 0, 0); ui_state_bind(20, 1, 1); ui_state_bind(30, 2, 2); ui_state_bind(40, 3, 3); ui_state_bind_text(50, 4); ui_state_text_set(4, \"Draft\"); var output: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let length: UIntSize = ui_state_text_length(4); let copied: UIntSize = ui_state_text_read(4, output); if length != copied { return 1 } return ui_state_get(0) }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_windows_ui_input_read_signatures() {
        let output = check(
            "module test; fn read_input(output: write Slice<UInt8>) -> UIntSize { return ui_input_read(10, output) } fn main() -> Int32 { ui_window(\"UI\", 640, 360, 0xF6F8FCu32); ui_text_input(\"Text\", 10, 20, 80, 300, 36, 0x111827u32, 0xFFFFFFu32, 8); var output: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let copied: UIntSize = ui_input_read(10, output); let length: UIntSize = ui_input_length(10); return (copied + length) as Int32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_windows_ui_input_exact_read_signature() {
        let output = check(
            "module test; fn main() -> Int32 { ui_window(\"UI\", 640, 360, 0xF6F8FCu32); ui_text_input(\"Text\", 10, 20, 80, 300, 36, 0x111827u32, 0xFFFFFFu32, 8); var output: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; var length: [UIntSize; 1] = [0usize]; let copied: Bool = ui_input_read_exact(10, output, length); if copied { return length[0] as Int32 } return 0 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);

        let invalid = check(
            "module test; fn main() { var output: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; var wrong: [UInt64; 1] = [0u64]; ui_input_read_exact(10, output, wrong) }",
        );
        assert!(
            invalid
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0301"),
            "{:?}",
            invalid.diagnostics
        );
    }

    #[test]
    fn checks_windows_ui_input_app_state_binding_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { ui_text_input(\"Text\", 10, 20, 80, 300, 36, 0x111827u32, 0xFFFFFFu32, 8); ui_input_bind_app_state(10, \"note\"); ui_input_refresh_app_state(10); return 0 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_windows_ui_control_app_state_binding_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { ui_checkbox(\"Enabled\", 10, 0, 0, 220, 36, 0x111827u32, 0x168EF5u32, 8); ui_select(20, 0, 42, 220, 36, 0x111827u32, 0xFFFFFFu32); ui_select_option(20, \"First\"); ui_list(30, 0, 80, 220, 120, 0x111827u32, 0xFFFFFFu32); ui_list_item(30, \"First\"); ui_table(40, 0, 80, 220, 120, 0x111827u32, 0xFFFFFFu32); ui_table_column(40, 0, \"Name\", 160); ui_table_cell(40, 0, 0, \"First\"); let enabled: Bool = ui_checkbox_bind_app_state(10, \"enabled\"); ui_checkbox_refresh_app_state(10); let choice: Bool = ui_select_bind_app_state(20, \"choice\"); ui_select_refresh_app_state(20); let list: Bool = ui_list_bind_app_state(30, \"list_choice\"); ui_list_refresh_app_state(30); let table: Bool = ui_table_bind_app_state(40, \"table_choice\"); ui_table_refresh_app_state(40); let retained: Bool = ui_app_bind_app_state(30, \"list_choice\"); ui_app_refresh_app_state(30); if enabled && choice && list && table && retained { return 0 } return 1 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_file_persistence_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { var payload: [UInt8; 4] = [1u8, 2u8, 3u8, 4u8]; let written: UIntSize = file_write(\"target/test.tmp\", payload); let text: UIntSize = file_append_text(\"target/test.tmp\", \"\\nrow\"); var digits: [UInt8; 20] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let number: UIntSize = format_int(1 as Int64, digits); let unsigned: UIntSize = format_uint(2u64, digits); let appended: UIntSize = file_append(\"target/test.tmp\", digits, number); let flag: UIntSize = format_bool(true, digits); let float: UIntSize = format_float(1.5f64, digits); let replaced: Bool = file_replace_atomic(\"target/test.tmp\", \"target/test.bin\"); var output: [UInt8; 4] = [0u8, 0u8, 0u8, 0u8]; let loaded: UIntSize = file_read(\"target/test.bin\", output); let exists: Bool = file_exists(\"target/test.bin\"); let size: UIntSize = file_size(\"target/test.bin\"); let removed: Bool = file_delete(\"target/test.bin\"); let lock: UIntSize = file_lock(\"target/test.lock\"); let unlocked: Bool = file_unlock(lock); return (written + text + number + unsigned + appended + flag + float + loaded + size) as Int32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_file_flush_builtin_signature() {
        let output = check(
            "module test; fn main() -> Int32 { let flushed: Bool = file_flush(\"target/test.tmp\"); if flushed { return 0 } return 1 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_directory_list_builtin_signature() {
        let output = check(
            "module test; fn main() -> Int32 { var output: [UInt8; 33] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let length: UIntSize = directory_list(\"target\", output); return length as Int32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_file_read_text_builtin_signature() {
        let output = check(
            "module test; fn main() -> Int32 { var output: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let length: UIntSize = file_read_text(\"target/text.txt\", output); return length as Int32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_file_read_exact_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { var bytes: [UInt8; 8] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; var length: [UIntSize; 1] = [0usize]; let binary: Bool = file_read_exact(\"target/data.bin\", bytes, length); let text: Bool = file_read_text_exact(\"target/data.txt\", bytes, length); if binary || text { return length[0] as Int32 } return 0 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);

        let invalid = check(
            "module test; fn main() { var bytes: [UInt8; 8] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; var wrong: [UInt64; 1] = [0u64]; file_read_exact(\"target/data.bin\", bytes, wrong) }",
        );
        assert!(
            invalid
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0301"),
            "{:?}",
            invalid.diagnostics
        );
    }

    #[test]
    fn checks_process_argument_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { let count: UIntSize = process_arg_count(); var output: [UInt8; 1] = [0u8]; let length: UIntSize = process_arg_read(0usize, output); return length as Int32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_standard_io_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { var input: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let received: UIntSize = stdin_read(input); let written: UIntSize = stdout_write(input); let error_written: UIntSize = stderr_write(input); return written as Int32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_file_read_at_builtin_signature() {
        let output = check(
            "module test; fn main() -> Int32 { var output: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let length: UIntSize = file_read_at(\"target/text.txt\", 2usize, output); return length as Int32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_file_write_at_builtin_signature() {
        let output = check(
            "module test; fn main() -> Int32 { var input: [UInt8; 3] = [97u8, 98u8, 99u8]; let length: UIntSize = file_write_at(\"target/text.txt\", 2usize, input); return length as Int32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_directory_list_ex_builtin_signature() {
        let output = check(
            "module test; fn main() -> Int32 { var names: [UInt8; 32] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; var kinds: [UInt8; 4] = [0u8, 0u8, 0u8, 0u8]; let count: UIntSize = directory_list_ex(\"target\", names, kinds); return count as Int32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_string_builder_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { var output: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; var bytes: [UInt8; 3] = [33u8, 33u8, 33u8]; let first: UIntSize = string_builder_append(\"Aho\", output, 0usize); let second: UIntSize = string_builder_append_bytes(bytes, output, first); return second as Int32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_owned_string_builtin_signatures_and_result_flow() {
        let output = check(
            "module test; fn main() -> Int32 { let created: Result<OwnedString, Int32> = string_owned_create(8usize); var result_code: Int32 = 1; match created { Ok(value) => { let appended: Int32 = string_owned_append(value, \"!\"); var output: [UInt8; 8] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let copied: UIntSize = string_owned_copy(value, output); let length: UIntSize = string_owned_length(value); let cleared: Int32 = string_owned_clear(value); if appended == 0 { if copied == 1usize { if length == 1usize { result_code = cleared } } } } Error(status) => { result_code = status } } return result_code }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_app_state_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { app_state_clear(); let a: Bool = app_state_set_int(\"minutes\", 42 as Int64); let b: Bool = app_state_set_uint(\"total\", 7u64); let c: Bool = app_state_set_bool(\"done\", true); let d: Bool = app_state_set_text(\"name\", \"Focus\"); let count: Int32 = app_state_count(); let first_kind: Int32 = app_state_type_at(0); let exists: Bool = app_state_exists(\"name\"); let missing: Bool = app_state_exists(\"missing\"); let removed: Bool = app_state_remove(\"missing\"); var output: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let key: UIntSize = app_state_read_key(0, output); let text: UIntSize = app_state_read_text(\"name\", output); let signed: Int64 = app_state_get_int(\"minutes\"); let unsigned: UInt64 = app_state_get_uint(\"total\"); let done: Bool = app_state_get_bool(\"done\"); let saved: Bool = app_state_save(\"target/state.json\"); let loaded: Bool = app_state_load(\"target/state.json\"); if !exists { return 1 } if missing { return 2 } return (text + key + (signed as UIntSize) + (unsigned as UIntSize) + (count as UIntSize) + (first_kind as UIntSize)) as Int32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_app_state_float_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { var output: [UInt8; 8] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let set: Bool = app_state_set_float(\"rate\", 1.25f64); let value: Float64 = app_state_get_float(\"rate\"); let length: UIntSize = format_float(value, output); if !set { return 1 } if length != 8usize { return 2 } return 0 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_app_state_transaction_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { let begun: Bool = app_state_tx_begin(); let committed: Bool = app_state_tx_commit(); let rolled_back: Bool = app_state_tx_rollback(); let data_begun: Bool = app_data_tx_begin(); let data_committed: Bool = app_data_tx_commit(); let data_rolled_back: Bool = app_data_tx_rollback(); if begun && committed && rolled_back && data_begun && data_committed && data_rolled_back { return 1 } return 0 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_app_data_persistence_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { var output: [UInt8; 1] = [0u8]; var length: [UIntSize; 1] = [0usize]; var journal_stats: [UIntSize; 2] = [0usize, 0usize]; var journal_plan: [UIntSize; 3] = [0usize, 0usize, 0usize]; let saved: Bool = app_data_save(\"target/app-data.jdn\"); let atomic: Bool = app_data_save_atomic(\"target/app-data.tmp\", \"target/app-data.jdn\"); let tx_atomic: Bool = app_data_tx_save_atomic(\"target/app-data.tx.tmp\", \"target/app-data.jdn\"); let durable: Bool = app_data_tx_commit_durable(\"target/app-data.durable.tmp\", \"target/app-data.durable.jdn\", \"target/app-data.durable.lock\"); let journal: Bool = app_data_journal_append(\"target/app-data.journal\", \"target/app-data.scratch\"); let recovered: Bool = app_data_journal_recover(\"target/app-data.journal\", \"target/app-data.scratch\"); let compacted: Bool = app_data_journal_recover_compact(\"target/app-data.journal\", \"target/app-data.scratch\", \"target/app-data.tmp\"); let loaded: Bool = app_data_load(\"target/app-data.jdn\"); let exact_saved: Bool = app_data_write_exact(output, length); let exact_loaded: Bool = app_data_load_exact(output, length[0]); let frame_length: UIntSize = app_data_journal_frame_length_durable(\"target/app-data.journal\", \"target/app-data.lock\", 0usize); let latest: Bool = app_data_journal_read_latest_frame_exact_durable(\"target/app-data.journal\", \"target/app-data.lock\", output, length); let stats: Bool = app_data_journal_stats_durable(\"target/app-data.journal\", \"target/app-data.lock\", journal_stats); let plan: Bool = app_data_journal_maintenance_plan_durable(\"target/app-data.journal\", \"target/app-data.lock\", 4096usize, 8usize, journal_plan); let needed: Bool = app_data_journal_compact_if_needed_durable(\"target/app-data.journal\", \"target/app-data.scratch\", \"target/app-data.tmp\", \"target/app-data.lock\", 4096usize, 8usize); if saved && atomic && tx_atomic && durable && journal && recovered && compacted && loaded && exact_saved && exact_loaded && frame_length >= 0usize && latest && stats && plan && needed { return 0 } return 1 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_app_list_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { app_list_clear(0); let pushed: Bool = app_list_push_text(0, \"Focus\"); var query: [UInt8; 2] = [70u8, 79u8]; let pushed_bytes: Bool = app_list_push_text_bytes(0, query, 2usize); let count: Int32 = app_list_count(0); let sorted: Bool = app_list_sort_text(0, false); let found: Int32 = app_list_find_text(0, \"Focus\", 0); let filtered: Bool = app_list_filter_text(0, 1, \"Focus\"); let filtered_ex: Bool = app_list_filter_text_ex(0, 1, \"FO\", 5); let filtered_bytes: Bool = app_list_filter_text_ex_bytes(0, 1, query, 2usize, 5); var output: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let copied: UIntSize = app_list_read_text(0, 0, output); let exported: UIntSize = app_list_export_csv(0, output); let updated: Bool = app_list_set_text(0, 0, \"Done\"); let updated_bytes: Bool = app_list_set_text_bytes(0, 0, query, 2usize); let removed: Bool = app_list_remove(0, 0); let saved: Bool = app_list_save(0, \"target/list.json\"); let atomic: Bool = app_list_save_atomic(0, \"target/list.tmp\", \"target/list.json\"); let loaded: Bool = app_list_load(0, \"target/list.json\"); if !pushed { return 1 } if !pushed_bytes { return 2 } if !sorted { return 3 } if !filtered { return 4 } if !filtered_ex { return 5 } if !filtered_bytes { return 6 } if !updated || !updated_bytes { return 7 } return count + found + (copied as Int32) + (exported as Int32) }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_app_table_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { app_table_clear(0); app_table_clear(1); let type_set: Bool = app_table_set_column_type(0, 0, 1); let float_type_set: Bool = app_table_set_column_type(0, 1, 4); let kind: Int32 = app_table_column_type(0, 0); let valid: Bool = app_table_validate(0); let appended: Bool = app_table_append_row(0); let set: Bool = app_table_set_cell(0, 0, 0, \"42\"); let set_int: Bool = app_table_set_int(0, 0, 0, 42 as Int64); let set_uint: Bool = app_table_set_uint(0, 0, 0, 7u64); let set_float: Bool = app_table_set_float(0, 0, 1, 1.25f64); let set_bool: Bool = app_table_set_bool(0, 0, 0, true); let rows: Int32 = app_table_row_count(0); let sorted: Bool = app_table_sort_text(0, 0, false); let sorted_int: Bool = app_table_sort_int(0, 0, false); let sorted_uint: Bool = app_table_sort_uint(0, 0, false); let sorted_float: Bool = app_table_sort_float(0, 1, false); let sorted_bool: Bool = app_table_sort_bool(0, 0, false); let found: Int32 = app_table_find_text(0, 0, \"42\", 0); let filtered: Bool = app_table_filter_text(0, 1, 0, \"42\"); let filtered_ex: Bool = app_table_filter_text_ex(0, 1, 0, \"2\", 1); var output: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let copied: UIntSize = app_table_read_cell(0, 0, 0, output); let typed_int: Int64 = app_table_read_int(0, 0, 0); let typed_uint: UInt64 = app_table_read_uint(0, 0, 0); let typed_float: Float64 = app_table_read_float(0, 0, 1); let typed_bool: Bool = app_table_read_bool(0, 0, 0); let removed: Bool = app_table_remove_row(0, 0); let schema_saved: Bool = app_table_save_schema(0, \"target/schema.json\"); let schema_atomic: Bool = app_table_save_schema_atomic(0, \"target/schema.tmp\", \"target/schema.json\"); let schema_loaded: Bool = app_table_load_schema(0, \"target/schema.json\"); let saved: Bool = app_table_save(0, \"target/table.json\"); let atomic: Bool = app_table_save_atomic(0, \"target/table.tmp\", \"target/table.json\"); let loaded: Bool = app_table_load(0, \"target/table.json\"); if !type_set || !float_type_set || !valid || !appended || !set || !set_int || !set_uint || !set_float || !set_bool || !schema_saved || !schema_atomic || !schema_loaded { return 1 } if typed_bool { return 2 } return rows + (copied as Int32) + found + (typed_int as Int32) + (typed_uint as Int32) + (typed_float as Int32) + kind }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_app_table_exact_text_read_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { var output: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; var length: [UIntSize; 1] = [0usize]; let column: Bool = app_table_read_column_name_exact(0, 0, output, length); let cell: Bool = app_table_read_cell_exact(0, 0, 0, output, length); let named: Bool = app_table_read_named_cell_exact(0, 0, \"title\", output, length); if column || cell || named { return 1 } return 0 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_app_table_exact_typed_read_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { var i: [Int64; 1] = [0 as Int64]; var u: [UInt64; 1] = [0u64]; var f: [Float64; 1] = [0.0f64]; var b: [Bool; 1] = [false]; let a: Bool = app_table_read_int_exact(0, 0, 0, i); let c: Bool = app_table_read_uint_exact(0, 0, 1, u); let d: Bool = app_table_read_float_exact(0, 0, 2, f); let e: Bool = app_table_read_bool_exact(0, 0, 3, b); let n: Bool = app_table_read_named_int_exact(0, 0, \"id\", i); if a || c || d || e || n { return 1 } return 0 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_app_table_typed_query_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { let i: Int32 = app_table_find_int(0, 0, 42 as Int64, 0); let u: Int32 = app_table_find_uint(0, 1, 7u64, 0); let f: Int32 = app_table_find_float(0, 2, 1.25f64, 0); let b: Int32 = app_table_find_bool(0, 3, true, 0); return i + u + f + b }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_app_table_upsert_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { let text: Int32 = app_table_upsert_text(0, 0, \"task\"); let i: Int32 = app_table_upsert_int(0, 1, 42 as Int64); let u: Int32 = app_table_upsert_uint(0, 2, 7u64); let f: Int32 = app_table_upsert_float(0, 3, 1.25f64); let b: Int32 = app_table_upsert_bool(0, 4, true); return text + i + u + f + b }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_app_table_remove_key_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { let text: Bool = app_table_remove_text(0, 0, \"task\"); let i: Bool = app_table_remove_int(0, 1, 42 as Int64); let u: Bool = app_table_remove_uint(0, 2, 7u64); let f: Bool = app_table_remove_float(0, 3, 1.25f64); let b: Bool = app_table_remove_bool(0, 4, true); if text || i || u || f || b { return 1 } return 0 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_app_table_index_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { let built: Bool = app_table_index_build(0, 0); let built_i: Bool = app_table_index_build_int(0, 0); let built_u: Bool = app_table_index_build_uint(0, 0); let built_f: Bool = app_table_index_build_float(0, 0); let built_b: Bool = app_table_index_build_bool(0, 0); let valid: Bool = app_table_index_is_valid(0, 0); let found: Int32 = app_table_index_find_text(0, 0, \"Task\"); let found_i: Int32 = app_table_index_find_int(0, 0, 1 as Int64); let found_u: Int32 = app_table_index_find_uint(0, 0, 1u64); let found_f: Int32 = app_table_index_find_float(0, 0, 1.0f64); let found_b: Int32 = app_table_index_find_bool(0, 0, true); let cleared: Bool = app_table_index_clear(0); if built && built_i && built_u && built_f && built_b && valid && cleared { return found + found_i + found_u + found_f + found_b } return -1 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_app_table_byte_setter_signature() {
        let output = check(
            "module test; fn main() -> Int32 { app_table_clear(0); app_table_append_row(0); var value: [UInt8; 4] = [84u8, 97u8, 115u8, 107u8]; let written: Bool = app_table_set_cell_bytes(0, 0, 0, value); if written { return 0 } return 1 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_app_table_byte_setter_length_signature() {
        let output = check(
            "module test; fn main() -> Int32 { app_table_clear(0); app_table_append_row(0); var value: [UInt8; 8] = [84u8, 97u8, 115u8, 107u8, 0u8, 0u8, 0u8, 0u8]; let written: Bool = app_table_set_cell_bytes_ex(0, 0, 0, value, 4usize); if written { return 0 } return 1 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_app_table_byte_filter_signature() {
        let output = check(
            "module test; fn main() -> Int32 { app_table_clear(0); app_table_clear(1); app_table_append_row(0); var query: [UInt8; 4] = [80u8, 108u8, 97u8, 110u8]; let filtered: Bool = app_table_filter_text_ex_bytes(0, 1, 0, query, 4usize, 1); if filtered { return 0 } return 1 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_app_table_transaction_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { let begun: Bool = app_table_tx_begin(0); let committed: Bool = app_table_tx_commit(); let rolled_back: Bool = app_table_tx_rollback(); let all_begun: Bool = app_table_tx_begin_all(); let all_committed: Bool = app_table_tx_commit_all(); let all_rolled_back: Bool = app_table_tx_rollback_all(); if begun && committed && rolled_back && all_begun && all_committed && all_rolled_back { return 1 } return 0 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_app_table_migration_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { let begun: Bool = app_table_migration_begin(0, 1, 2); let renamed: Bool = app_table_migration_rename_column(\"old\", \"new\"); let typed: Bool = app_table_migration_set_column_type(\"new\", 0); let committed: Bool = app_table_migration_commit(); let rolled_back: Bool = app_table_migration_rollback(); if begun && renamed && typed && committed && rolled_back { return 0 } return 1 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_named_app_table_schema_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { let named: Bool = app_table_set_column_name(0, 0, \"id\"); var output: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let length: UIntSize = app_table_read_column_name(0, 0, output); let found: Int32 = app_table_find_column(0, \"id\"); if named { return (length as Int32) + found } return 0 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_named_app_table_field_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { let text_set: Bool = app_table_set_named_cell(0, 0, \"title\", \"Focus\"); let int_set: Bool = app_table_set_named_int(0, 0, \"id\", 42 as Int64); let uint_set: Bool = app_table_set_named_uint(0, 0, \"total\", 7u64); let bool_set: Bool = app_table_set_named_bool(0, 0, \"done\", true); var output: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let _text_read: UIntSize = app_table_read_named_cell(0, 0, \"title\", output); let _signed: Int64 = app_table_read_named_int(0, 0, \"id\"); let _unsigned: UInt64 = app_table_read_named_uint(0, 0, \"total\"); let done: Bool = app_table_read_named_bool(0, 0, \"done\"); let version: Int32 = app_table_schema_version(0); let version_set: Bool = app_table_set_schema_version(0, version); let guarded: Bool = app_table_load_schema_full_if_version(0, \"schema.json\", version); if text_set && int_set && uint_set && bool_set && done && version_set && guarded { return 0 } return 1 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_app_table_float_field_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { let set: Bool = app_table_set_float(0, 0, 0, 1.25f64); let _value: Float64 = app_table_read_float(0, 0, 0); let named_set: Bool = app_table_set_named_float(0, 0, \"rate\", 2.5f64); let _named: Float64 = app_table_read_named_float(0, 0, \"rate\"); if !set { return 1 } if !named_set { return 1 } return 0 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_csv_and_json_escape_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { var output: [UInt8; 32] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let csv: UIntSize = csv_escape(\"a,b\", output); let json: UIntSize = json_escape(\"a\\\"b\", output); return (csv + json) as Int32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_app_table_csv_export_signature() {
        let output = check(
            "module test; fn main() -> Int32 { var output: [UInt8; 4] = [0u8, 0u8, 0u8, 0u8]; let length: UIntSize = app_table_export_csv(0, output); return length as Int32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_app_table_csv_import_signature() {
        let output = check(
            "module test; fn main() -> Int32 { var input: [UInt8; 4] = [0u8, 0u8, 0u8, 0u8]; let imported: Bool = app_table_import_csv(0, input, 4usize); if imported { return 1 } return 0 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_json_object_field_string_builtin_signature() {
        let output = check(
            "module test; fn main() -> Int32 { var output: [UInt8; 32] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let member: UIntSize = json_object_field_string(\"name\", \"Ada\", output); return member as Int32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_json_object_field_numeric_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { var output: [UInt8; 33] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let signed: UIntSize = json_object_field_int(\"signed\", 42 as Int64, output); let unsigned: UIntSize = json_object_field_uint(\"unsigned\", 7u64, output); let decimal: UIntSize = json_object_field_float(\"decimal\", 1.5f64, output); let flag: UIntSize = json_object_field_bool(\"flag\", true, output); return (signed + unsigned + decimal + flag) as Int32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_json_object_read_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { var input: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; var output: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let text: UIntSize = json_object_read_string(input, \"name\", output); let signed: Int64 = json_object_read_int(input, \"minutes\"); let unsigned: UInt64 = json_object_read_uint(input, \"total\"); let rate: Float64 = json_object_read_float(input, \"rate\"); let done: Bool = json_object_read_bool(input, \"done\"); if done { return (text as Int32) + (signed as Int32) + (unsigned as Int32) + (rate as Int32) } return 0 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_utc_calendar_parts_builtin_signature() {
        let output = check(
            "module test; fn main() -> Int32 { var parts: [Int32; 6] = [0, 0, 0, 0, 0, 0]; let ok: Bool = time_utc_parts(0 as Int64, parts); return if ok { parts[0] } else { 1 } }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_utc_offset_calendar_parts_builtin_signature() {
        let output = check(
            "module test; fn main() -> Int32 { var parts: [Int32; 6] = [0, 0, 0, 0, 0, 0]; let ok: Bool = time_utc_offset_parts(0 as Int64, 60, parts); return if ok { parts[3] } else { 1 } }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_native_tls_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { let socket: UIntSize = net_tcp_connect_dns(\"localhost\", 38127u16); let tls: UIntSize = net_tls_open_client(socket, \"localhost\", false); let server_tls: UIntSize = net_tls_open_server(socket, \"cert.pem\", \"key.pem\"); let step: UInt32 = net_tls_step(tls, 1000u32); let state: UInt32 = net_tls_state(tls); let error: Int32 = net_tls_error(tls); var input: [UInt8; 2] = [1u8, 2u8]; var output: [UInt8; 2] = [0u8, 0u8]; let sent: UIntSize = net_tls_send(tls, input); let received: UIntSize = net_tls_receive(tls, output); let closed: Bool = net_tls_close(tls); let server_closed: Bool = net_tls_close(server_tls); if !server_closed { return 1 } if closed { return (step as Int32) + (state as Int32) + (sent as Int32) + (received as Int32) } return error }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_net_reactor_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { let listener: UIntSize = net_tcp_listen(38129u16); let reactor: UIntSize = net_reactor_open(4u32, 4u32); let watched: Bool = net_reactor_watch(reactor, listener, 1u32, 7usize); let count: UInt32 = net_reactor_poll(reactor, 1u32); let socket: UIntSize = net_reactor_event_socket(reactor, 0u32); let flags: UInt32 = net_reactor_event_flags(reactor, 0u32); let user: UIntSize = net_reactor_event_user(reactor, 0u32); let error: Int32 = net_reactor_error(reactor); let unwatched: Bool = net_reactor_unwatch(reactor, listener); let closed: Bool = net_reactor_close(reactor); if watched && unwatched && closed { return (count + flags) as Int32 } return (socket + user) as Int32 + error }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_net_reactor_operation_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { let listener: UIntSize = net_tcp_listen(38130u16); let reactor: UIntSize = net_reactor_open(4u32, 8u32); let accept_operation: UIntSize = net_reactor_submit_accept(reactor, listener, 10usize); let connect_operation: UIntSize = net_reactor_submit_connect(reactor, \"127.0.0.1\", 38130u16, 11usize); let receive_operation: UIntSize = net_reactor_submit_receive(reactor, listener, 12usize); let send_operation: UIntSize = net_reactor_submit_send(reactor, listener, 13usize); let cancelled: Bool = net_reactor_cancel(reactor, accept_operation); let event_operation: UIntSize = net_reactor_event_operation(reactor, 0u32); if cancelled { return (connect_operation + receive_operation + send_operation + event_operation) as Int32 } return 0 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_bounded_scheduler_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { app_scheduler_clear(); let created: Bool = app_scheduler_set(7, 100 as Int64, 60u64); var due: [Int32; 4] = [0, 0, 0, 0]; let count: UIntSize = app_scheduler_poll(100 as Int64, due); let removed: Bool = app_scheduler_cancel(7); let active: UIntSize = app_scheduler_count(); if created && count == 1usize && removed && active == 0usize { return due[0] } return 1 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_json_array_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { var signed: [Int64; 2] = [1 as Int64, 2 as Int64]; var unsigned: [UInt64; 2] = [3u64, 4u64]; var decimals: [Float64; 2] = [1.0f64, 2.0f64]; var flags: [Bool; 2] = [true, false]; var output: [UInt8; 32] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let a: UIntSize = json_array_int(signed, output); let b: UIntSize = json_array_uint(unsigned, output); let c: UIntSize = json_array_float(decimals, output); let d: UIntSize = json_array_bool(flags, output); return (a + b + c + d) as Int32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_http_response_builtin_signature() {
        let output = check(
            "module test; fn main() -> Int32 { var body: [UInt8; 4] = [65u8, 104u8, 111u8, 106u8]; var output: [UInt8; 131] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let response: UIntSize = http_response_write(200u16, \"text/plain\", body, output); return response as Int32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_http_response_reader_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { var input: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; var output: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let status: UInt16 = http_response_status(input); let bounded_status: UInt16 = http_response_status_prefix(input, 8usize); let header: UIntSize = http_response_header(input, \"Content-Type\", output); let bounded_header: UIntSize = http_response_header_prefix(input, 8usize, \"Content-Type\", output); let body: UIntSize = http_response_body(input, output); let bounded_body: UIntSize = http_response_body_prefix(input, 8usize, output); return (status as Int32) + (bounded_status as Int32) + (header + bounded_header + body + bounded_body) as Int32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_http_response_exact_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { var input: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; var output: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; var length: [UIntSize; 1] = [0usize]; let header: Bool = http_response_header_exact(input, \"Content-Type\", output, length); let body: Bool = http_response_body_exact(input, output, length); if header { return length[0] as Int32 } if body { return length[0] as Int32 } return 0 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_http_response_chunked_exact_builtin_signature() {
        let output = check(
            "module test; fn main() -> Int32 { var input: [UInt8; 1] = [0u8]; var output: [UInt8; 1] = [0u8]; var length: [UIntSize; 1] = [0usize]; let decoded: Bool = http_response_body_chunked_exact(input, output, length); if decoded { return length[0] as Int32 } return 0 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_http_request_chunked_exact_builtin_signature() {
        let output = check(
            "module test; fn main() -> Int32 { var input: [UInt8; 1] = [0u8]; var output: [UInt8; 1] = [0u8]; var length: [UIntSize; 1] = [0usize]; let decoded: Bool = http_request_body_chunked_exact(input, output, length); if decoded { return length[0] as Int32 } return 0 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_http_router_builtin_signatures_active() {
        let output = check(
            r#"module test; fn main() -> Int32 { var body: [UInt8; 1] = [79u8]; var input: [UInt8; 1] = [0u8]; var output: [UInt8; 8] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; http_router_clear(); let added: Bool = http_router_add("GET", "/", 200u16, "text/plain", body); let prefix_added: Bool = http_router_add_prefix("GET", "/api/", 200u16, "text/plain", body); let removed: Bool = http_router_remove("GET", "/"); let prefix_removed: Bool = http_router_remove_prefix("GET", "/api/"); let count: UIntSize = http_router_count(); let written: UIntSize = http_router_respond(input, output); if added && prefix_added && removed && prefix_removed && count == 0usize { return written as Int32 } return 0 }"#,
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_http_keep_alive_builtin_signatures_active() {
        let output = check(
            r#"module test; fn main() -> Int32 { var body: [UInt8; 1] = [79u8]; var input: [UInt8; 1] = [0u8]; var output: [UInt8; 140] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let keep: Bool = http_request_keep_alive(input); let response: UIntSize = http_response_write_ex(200u16, "text/plain", body, true, output); if keep { return response as Int32 } return 0 }"#,
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_http_response_custom_header_builtin_signatures_active() {
        let output = check(
            r#"module test; fn main() -> Int32 { var body: [UInt8; 1] = [79u8]; var output: [UInt8; 112] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let close: UIntSize = http_response_write_header(429u16, "text/plain", "Retry-After", "1", body, output); let keep: UIntSize = http_response_write_header_ex(200u16, "text/plain", "X-Trace", "ready", body, true, output); return (close + keep) as Int32 }"#,
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_http_response_custom_header_block_builtin_signatures_active() {
        let output = check(
            r#"module test; fn main() -> Int32 { var body: [UInt8; 1] = [79u8]; var output: [UInt8; 1] = [0u8]; let close: UIntSize = http_response_write_header_block(429u16, "text/plain", "Retry-After: 1\r\nX-Trace: limited", body, output); let keep: UIntSize = http_response_write_header_block_ex(200u16, "text/plain", "X-Trace: ready", body, true, output); return (close + keep) as Int32 }"#,
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_http_session_builtin_signatures_active() {
        let output = check(
            r#"module test; fn main() -> Int32 { let listener: UIntSize = net_tcp_listen(38125u16); let session: UIntSize = http_session_open(listener, 2u32, 1024u32, 1024u32); let tls_session: UIntSize = http_session_open_tls(listener, 2u32, 1024u32, 1024u32, "cert.pem", "key.pem"); let state: UInt32 = http_session_step(session, 1u32); let closed: Bool = http_session_close(session); let tls_closed: Bool = http_session_close(tls_session); if closed && tls_closed { return state as Int32 } return 0 }"#,
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_http_request_writer_builtin_signature() {
        let output = check(
            r#"module test; fn main() -> Int32 { var body: [UInt8; 4] = [65u8, 104u8, 111u8, 106u8]; var output: [UInt8; 4] = [0u8, 0u8, 0u8, 0u8]; let request: UIntSize = http_request_write("GET", "/hello", "localhost", body, output); return request as Int32 }"#,
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_http_request_writer_prefix_builtin_signature() {
        let output = check(
            r#"module test; fn main() -> Int32 { var body: [UInt8; 4] = [65u8, 104u8, 111u8, 106u8]; var output: [UInt8; 4] = [0u8, 0u8, 0u8, 0u8]; let request: UIntSize = http_request_write_prefix("POST", "/hello", "localhost", body, 2usize, output); return request as Int32 }"#,
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    /*
    #[test]
    fn checks_http_request_writer_builtin_signature_broken_previous() {
        let output = check(
            "module test; fn main() -> Int32 { var body: [UInt8; 4] = [65u8, 104u8, 111u8, 106u8]; var output: [UInt8; 128] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8. let response: UIntSize = http_response_write(200u16, "text/plain", body, output); return response as Int32 }",
            "module test; fn main() -> Int32 { var body: [UInt8; 4] = [65u8, 104u8, 111u8, 106u8]; var output: [UInt8; 131] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8. let response: UIntSize = http_response_write(200u16, "text/plain", body, output); return response as Int32 }",

    */

    #[test]
    fn checks_http_request_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { var request: [UInt8; 8] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; var output: [UInt8; 8] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; var query_length: [UIntSize; 1] = [0usize]; let complete: Bool = http_request_is_complete(request); let method: UIntSize = http_request_method(request, output); let target: UIntSize = http_request_target(request, output); let header: UIntSize = http_request_header(request, \"Host\", output); let body: UIntSize = http_request_body(request, output); let query: UIntSize = http_query_param(request, \"id\", output); let query_exact: Bool = http_query_param_exact(request, \"id\", output, query_length); let route: Bool = http_route_match(request, \"GET\", \"/\"); if query_exact { return query as Int32 } return (method + target + header + body + query) as Int32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_http_request_framing_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { var request: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let frame: UIntSize = http_request_frame_length_prefix(request, 8usize); let remaining: UIntSize = http_request_consume_prefix(request, 8usize, frame); return (frame + remaining) as Int32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_http_request_chunked_framing_builtin_signature() {
        let output = check(
            "module test; fn main() -> Int32 { var request: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let frame: UIntSize = http_request_chunked_frame_length_prefix(request, 8usize); return frame as Int32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_tcp_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { let listener: UIntSize = net_tcp_listen(38123u16); let connected: UIntSize = net_tcp_connect(\"127.0.0.1\", 38123u16); let named: UIntSize = net_tcp_connect_dns(\"localhost\", 38123u16); let accepted: UIntSize = net_tcp_accept(listener); let timed: Bool = net_socket_set_timeout(accepted, 1000u32); var request: [UInt8; 8] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; var response: [UInt8; 8] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let sent: UIntSize = net_tcp_send(connected, request); let received: UIntSize = net_tcp_receive(accepted, response); let closed: Bool = net_socket_close(connected); return (listener + connected + named + accepted + sent + received) as Int32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_tcp_send_prefix_builtin_signature() {
        let output = check(
            "module test; fn main() -> Int32 { var request: [UInt8; 8] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let sent: UIntSize = net_tcp_send_prefix(1usize, request, 4usize); return sent as Int32 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn checks_windows_ui_theme_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { ui_theme(1); let background: UInt32 = ui_theme_color(0); ui_window(\"UI\", 640, 480, background); ui_image(\"logo.png\", 20, 20, 64, 64); ui_theme(0); return ui_run() }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn rejects_windows_ui_builtin_type_mismatch() {
        let output = check(
            "module test; fn main() { ui_top_bar(true, 0x252526u32); ui_column(20, 64, 600, true, 16, 12, 0, 1) }",
        );
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0301")
        );
    }

    #[test]
    fn checks_float4_slice_intrinsics_and_write_to_read_coercion() {
        let output = check(
            "module test; @noalloc fn main(values: write Slice<Float32>, index: UIntSize, delta: Float32) { let current: Float4 = vector_load4(values, index); let amount: Float4 = vector_splat4(delta); vector_store4(values, index, current + amount) }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        assert!(
            output
                .expressions
                .iter()
                .any(|expression| expression.ty == output.types.core().float4)
        );
    }

    #[test]
    fn checks_float8_slice_intrinsics() {
        let output = check(
            "module test; @noalloc fn main(values: write Slice<Float32>, index: UIntSize, delta: Float32) { let current: Float8 = vector_load8(values, index); let amount: Float8 = vector_splat8(delta); vector_store8(values, index, current + amount) }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        assert!(
            output
                .expressions
                .iter()
                .any(|expression| expression.ty == output.types.core().float8)
        );
    }

    #[test]
    fn checks_float2_and_float3_slice_intrinsics() {
        for lanes in [2_u16, 3_u16] {
            let source = format!(
                "module test; @noalloc fn main(values: write Slice<Float32>, index: UIntSize, delta: Float32) {{ let current: Float{lanes} = vector_load{lanes}(values, index); let amount: Float{lanes} = vector_splat{lanes}(delta); vector_store{lanes}(values, index, current + amount) }}"
            );
            let output = check(&source);
            assert!(
                !output.has_errors(),
                "Float{lanes}: {:?}",
                output.diagnostics
            );
            assert!(output.expressions.iter().any(|expression| {
                output.types.kind(expression.ty).is_some_and(
                    |kind| matches!(kind, TypeKind::Vector { lanes: actual, .. } if *actual == lanes),
                )
            }));
        }
    }

    #[test]
    fn rejects_mismatched_float2_and_float3_intrinsic_values() {
        let output = check(
            "module test; @noalloc fn main(values: write Slice<Float32>, index: UIntSize, delta: Float32) { let amount: Float3 = vector_splat3(delta); vector_store2(values, index, amount) }",
        );
        assert!(output.has_errors());
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0301")
        );
    }

    #[test]
    fn checks_record_literals_and_field_access() {
        let output = check(
            "module test; struct Point { x: Int32, y: Int32 } fn main() { let point = Point { x: 1, y: 2 }; let value: Int32 = point.x; print(value) }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn reports_record_field_shape_and_type_errors() {
        let output = check(
            "module test; struct Point { x: Int32, y: Int32 } fn main() { let point = Point { x: true, x: 2, z: 3 }; print(point.missing) }",
        );
        let codes: Vec<_> = output
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect();
        assert!(codes.contains(&"J0301"));
        assert!(codes.contains(&"J0306"));
        assert!(codes.contains(&"J0307"));
        assert!(codes.contains(&"J0308"));
    }

    #[test]
    fn types_enum_payload_bindings_and_accepts_exhaustive_match() {
        let output = check(
            "module test; enum Choice { First, Second(Int32) } fn choose(value: Choice) -> Int32 { return match value { First => 0, Second(item) => item } }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn reports_unknown_variant_payload_arity_and_non_exhaustiveness() {
        let output = check(
            "module test; enum Choice { First, Second(Int32) } fn choose(value: Choice) -> Int32 { return match value { Third => 0, Second => 1 } }",
        );
        let codes: Vec<_> = output
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect();
        assert!(codes.contains(&"J0309"));
        assert!(codes.contains(&"J0310"));
        assert!(codes.contains(&"J0311"));
    }

    #[test]
    fn guarded_variant_does_not_make_match_exhaustive() {
        let output = check(
            "module test; enum Choice { First, Second } fn choose(value: Choice) -> Int32 { return match value { First if true => 0, Second => 1 } }",
        );
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0311")
        );
    }

    #[test]
    fn checks_enum_constructor_expressions() {
        let output = check(
            "module test; enum Choice { First, Second(Int32) } fn first() -> Choice { return Choice.First } fn second() -> Choice { return Second(1) }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);

        let invalid = check(
            "module test; enum Choice { First, Second(Int32) } fn second() -> Choice { return Second(true) }",
        );
        assert!(
            invalid
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0301")
        );
    }

    #[test]
    fn requires_qualification_for_ambiguous_enum_constructor() {
        let output = check(
            "module test; enum First { Same } enum Second { Same } fn make() -> First { return Same }",
        );
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0312")
        );
    }

    #[test]
    fn checks_option_result_constructors_and_patterns() {
        let output = check(
            "module test; fn maybe(flag: Bool) -> Option<Int32> { return if flag { Some(1) } else { None } } fn value(input: Option<Int32>) -> Int32 { return match input { Some(item) => item, None => 0 } } fn make_result() -> Result<Int32, String> { return Ok(1) }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn propagates_option_and_result_with_postfix_try() {
        let output = check(
            "module test; fn maybe() -> Option<Int32> { return Some(1) } fn option_outer() -> Option<Int32> { return Some(maybe()?) } fn load() -> Result<Int32, String> { return Ok(1) } fn result_outer() -> Result<Int32, String> { let value = load()?; return Ok(value) }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        assert_eq!(output.propagation_sites.len(), 2);
        assert!(
            output
                .propagation_sites
                .iter()
                .any(|site| site.kind == super::PropagationKind::OptionNone)
        );
        assert!(
            output
                .propagation_sites
                .iter()
                .any(|site| site.kind == super::PropagationKind::ResultError)
        );
    }

    #[test]
    fn rejects_invalid_try_context_and_result_error_conversion() {
        let invalid_context = check("module test; fn bad() -> Int32 { return Some(1)? }");
        assert!(
            invalid_context
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0313")
        );

        let incompatible_error = check(
            "module test; fn load() -> Result<Int32, String> { return Ok(1) } fn bad() -> Result<Int32, Int32> { return Ok(load()?) }",
        );
        assert!(
            incompatible_error
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0301")
        );
    }

    #[test]
    fn requires_exhaustive_option_match() {
        let output = check(
            "module test; fn value(input: Option<Int32>) -> Int32 { return match input { Some(item) => item } }",
        );
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0311")
        );
    }

    #[test]
    fn infers_and_deduplicates_generic_function_instances() {
        let output = check(
            "module test; fn identity<T>(value: T) -> T { return value } fn main() { let first: Int32 = identity(1); let second: Bool = identity(true); let third: Int32 = identity(2) }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        assert_eq!(output.monomorphizations.len(), 2);
    }

    #[test]
    fn reports_underconstrained_generic_call() {
        let output = check("module test; fn make<T>() -> T {} fn main() { make() }");
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0314")
        );
    }

    #[test]
    fn infers_generic_record_arguments_and_substitutes_fields() {
        let output = check(
            "module test; struct Box<T> { value: T } fn main() { let boxed = Box { value: 1 }; let value: Int32 = boxed.value; print(value) }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        assert!(output.expressions.iter().any(|expression| matches!(
            output.types.kind(expression.ty),
            Some(TypeKind::Nominal { arguments, .. }) if arguments.len() == 1
                && output.types.kind(arguments[0]) == Some(&TypeKind::Integer {
                    signedness: jadren_types::Signedness::Signed,
                    width: jadren_types::IntegerWidth::Bits32,
                })
        )));
    }

    #[test]
    fn checks_generic_enum_constructors_patterns_and_exhaustiveness() {
        let output = check(
            "module test; enum Maybe<T> { Missing, Present(T) } fn make() -> Maybe<Int32> { return Present(1) } fn value(input: Maybe<Int32>) -> Int32 { return match input { Missing => 0, Present(item) => item } }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);

        let incomplete = check(
            "module test; enum Maybe<T> { Missing, Present(T) } fn value(input: Maybe<Int32>) -> Int32 { return match input { Present(item) => item } }",
        );
        assert!(
            incomplete
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0311")
        );
    }

    #[test]
    fn reports_generic_nominal_type_arity_mismatch() {
        let output =
            check("module test; struct Box<T> { value: T } fn bad(value: Box<Int32, Bool>) {}");
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0315")
        );
    }

    #[test]
    fn enforces_core_trait_bounds_on_generic_calls() {
        let valid = check(
            "module test; fn identity<T: Numeric>(value: T) -> T { return value } fn main() { let value: Int32 = identity(1) }",
        );
        assert!(!valid.has_errors(), "{:?}", valid.diagnostics);

        let invalid = check(
            "module test; fn identity<T: Numeric>(value: T) -> T { return value } fn main() { identity(true) }",
        );
        assert!(
            invalid
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0316")
        );
        assert!(invalid.monomorphizations.is_empty());
    }

    #[test]
    fn stronger_generic_bound_implies_required_bound() {
        let output = check(
            "module test; fn numeric<T: Numeric>(value: T) -> T { return value } fn integer<T: Integer>(value: T) -> T { return numeric(value) }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
    }

    #[test]
    fn enforces_core_trait_bounds_on_records_and_enums() {
        let output = check(
            "module test; struct NumberBox<T: Numeric> { value: T } enum Number<T: Numeric> { Value(T) } fn main() { let boxed = NumberBox { value: true }; let number = Value(true) }",
        );
        assert_eq!(
            output
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "J0316")
                .count(),
            2
        );
    }

    #[test]
    fn rejects_generic_arguments_on_core_trait_bounds() {
        let output =
            check("module test; fn invalid<T: Numeric<Int32>>(value: T) -> T { return value }");
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0317")
        );
    }

    #[test]
    fn lowers_capability_types_and_allows_explicit_borrow_coercion() {
        let output = check(
            "module test; fn inspect(value: read Buffer<Int32>) {} fn update(value: write Buffer<Int32>) {} fn take(value: owned Buffer<Int32>) {} fn run(data: Buffer<Int32>) { inspect(data); update(data); let view: read Buffer<Int32> = data } fn transfer(data: Buffer<Int32>) { take(data) }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        assert!(output.symbol_types.iter().flatten().any(|ty| matches!(
            output.types.kind(*ty),
            Some(TypeKind::Capability {
                capability: jadren_types::Capability::Read,
                ..
            })
        )));
    }

    #[test]
    fn infers_typed_region_allocation_and_requires_element_context() {
        let output = check(
            "module test; fn main() { region frame { let values: Buffer<Int32> = frame.allocate(4); print(values) } }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        assert_eq!(output.region_allocations.len(), 1);
        assert!(matches!(
            output.types.kind(output.region_allocations[0].result_type),
            Some(TypeKind::Buffer(element))
                if output.types.kind(*element) == Some(&TypeKind::Integer {
                    signedness: jadren_types::Signedness::Signed,
                    width: jadren_types::IntegerWidth::Bits32,
                })
        ));

        let underconstrained =
            check("module test; fn main() { region frame { let values = frame.allocate(4) } }");
        assert!(
            underconstrained
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0508")
        );
    }

    #[test]
    fn checks_bounded_scalar_parser_builtin_signatures() {
        let output = check(
            "module test; fn main() -> Int32 { var text: [UInt8; 5] = [49u8, 50u8, 46u8, 53u8, 48u8]; var signed: [Int64; 1] = [0 as Int64]; var unsigned: [UInt64; 1] = [0u64]; var float: [Float64; 1] = [0.0f64]; var boolean: [Bool; 1] = [false]; let signed_ok: Bool = parse_int(text, 2usize, signed); let unsigned_ok: Bool = parse_uint(text, 2usize, unsigned); let float_ok: Bool = parse_float(text, 5usize, float); let boolean_ok: Bool = parse_bool(text, 2usize, boolean); if signed_ok || unsigned_ok || float_ok || boolean_ok { return 0 } return 1 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);

        let invalid = check(
            "module test; fn main() { var text: [UInt8; 1] = [49u8]; var wrong: [UInt64; 1] = [0u64]; parse_int(text, 1usize, wrong) }",
        );
        assert!(
            invalid
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0301"),
            "{:?}",
            invalid.diagnostics
        );
    }

    #[test]
    fn checks_app_state_typed_read_signatures_and_rejects_mismatches() {
        let output = check(
            "module test; fn main() -> Int32 { var signed: [Int64; 1] = [0 as Int64]; var unsigned: [UInt64; 1] = [0u64]; var decimal: [Float64; 1] = [0.0f64]; var flag: [Bool; 1] = [false]; let a: Bool = app_state_read_int(\"signed\", signed); let b: Bool = app_state_read_uint(\"unsigned\", unsigned); let c: Bool = app_state_read_float(\"decimal\", decimal); let d: Bool = app_state_read_bool(\"flag\", flag); if a || b || c || d { return 1 } return 0 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);

        let invalid = check(
            "module test; fn main() { var wrong: [UInt64; 1] = [0u64]; app_state_read_int(\"signed\", wrong) }",
        );
        assert!(
            invalid
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0301"),
            "{:?}",
            invalid.diagnostics
        );
    }

    #[test]
    fn checks_app_state_exact_text_read_signature() {
        let output = check(
            "module test; fn main() -> Int32 { var bytes: [UInt8; 8] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; var length: [UIntSize; 1] = [0usize]; let ok: Bool = app_state_read_text_exact(\"name\", bytes, length); if ok { return length[0] as Int32 } return 0 }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);

        let invalid = check(
            "module test; fn main() { var bytes: [UInt8; 8] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; var wrong: [UInt64; 1] = [0u64]; app_state_read_text_exact(\"name\", bytes, wrong) }",
        );
        assert!(
            invalid
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0301"),
            "{:?}",
            invalid.diagnostics
        );
    }
}
