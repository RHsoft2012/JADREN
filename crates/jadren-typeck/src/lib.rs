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
    DeclaredVisibility, ModuleEnumInterface, ModuleFunctionSignature, ModuleRecordInterface,
    ModuleType, Namespace, ResolutionOutput, Symbol, SymbolId, SymbolKind, SymbolOrigin,
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
    Checker::new(source, file, resolution).run()
}

struct Checker<'a> {
    source: &'a SourceFile,
    file: &'a AstFile,
    resolution: &'a ResolutionOutput,
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
    fn new(source: &'a SourceFile, file: &'a AstFile, resolution: &'a ResolutionOutput) -> Self {
        Self {
            source,
            file,
            resolution,
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
        let nominal_layouts = self.export_nominal_layouts();
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
                if !self.is_c_abi_type(field.ty, &mut visiting) {
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
                if !variant.fields.is_empty() {
                    self.diagnostics.push(Diagnostic::error(
                        "J0803",
                        "payload enums are not supported by `repr(C)` in Jadren 0.1",
                        variant.span,
                        "use a `struct` with an explicit tag and payload instead",
                    ));
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
                        && declaration
                            .variants
                            .values()
                            .all(|variant| variant.fields.is_empty())
                } else {
                    false
                };
                visiting.remove(constructor);
                result
            }
            _ => false,
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
                        .map(|value| self.infer_expression(value, return_type));
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
            return self.infer_builtin_call(&name, arguments, &argument_types, span);
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
        let element = self.unification.fresh(&mut self.types);
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
    ) -> TypeId {
        let expected = match name {
            "print" => 1,
            "assert_eq" => 2,
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
        let arg_span = |index: usize| arguments.get(index).map_or(span, Expression::span);
        match name {
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
        if let (
            Some(TypeKind::Capability {
                capability: Capability::Read,
                inner: expected_inner,
            }),
            Some(TypeKind::Capability {
                capability: Capability::Write,
                inner: actual_inner,
            }),
        ) = (
            self.types.kind(expected_resolved).cloned(),
            self.types.kind(actual_resolved).cloned(),
        ) {
            return match self
                .unification
                .unify(&mut self.types, expected_inner, actual_inner)
            {
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
            return match self
                .unification
                .unify(&mut self.types, inner, actual_resolved)
            {
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
        let payload_enum = check("module test; @repr(C) enum Bad { Value(Int32) }");
        assert!(
            payload_enum
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0803")
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
}
