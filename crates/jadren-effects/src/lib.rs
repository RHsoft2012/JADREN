//! Deterministic effect inference over verified Jadren HIR.

use std::collections::{BTreeMap, BTreeSet};

use jadren_hir::{HirBlock, HirExpression, HirExpressionKind, HirModule, HirStatement};
use jadren_lexer::Operator;
use jadren_resolve::{ResolutionOutput, SymbolId, SymbolKind, SymbolOrigin};
use jadren_source::Span;
use jadren_types::{Capability, TypeId, TypeKind, TypeStore};

/// Observable or safety-relevant behavior inferred from a function body.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum EffectKind {
    /// Reads through an explicit capability.
    Read = 0,
    /// Mutates an addressable place.
    Write = 1,
    /// Creates dynamic heap or region storage.
    Allocate = 2,
    /// Interacts with external input or output.
    Io = 3,
    /// May block the current execution context.
    Blocking = 4,
    /// Performs an atomic synchronization operation.
    Atomic = 5,
    /// Crosses an unsafe or currently unanalyzed call boundary.
    Unsafe = 6,
    /// May trigger a language panic.
    Panic = 7,
}

impl EffectKind {
    /// Stable display order for diagnostics and dumps.
    pub const ALL: [Self; 8] = [
        Self::Read,
        Self::Write,
        Self::Allocate,
        Self::Io,
        Self::Blocking,
        Self::Atomic,
        Self::Unsafe,
        Self::Panic,
    ];

    /// Canonical specification spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "Read",
            Self::Write => "Write",
            Self::Allocate => "Allocate",
            Self::Io => "IO",
            Self::Blocking => "Blocking",
            Self::Atomic => "Atomic",
            Self::Unsafe => "Unsafe",
            Self::Panic => "Panic",
        }
    }
}

/// Compact deterministic set of [`EffectKind`] values. An empty set is `Pure`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EffectSet(u16);

impl EffectSet {
    /// Creates an empty, pure effect set.
    #[must_use]
    pub const fn pure() -> Self {
        Self(0)
    }

    /// Creates a set containing one effect.
    #[must_use]
    pub const fn one(effect: EffectKind) -> Self {
        Self(1 << effect as u8)
    }

    /// Returns whether the function has no inferred effects.
    #[must_use]
    pub const fn is_pure(self) -> bool {
        self.0 == 0
    }

    /// Returns whether one effect is present.
    #[must_use]
    pub const fn contains(self, effect: EffectKind) -> bool {
        self.0 & (1 << effect as u8) != 0
    }

    /// Adds one effect and returns whether the set changed.
    pub fn insert(&mut self, effect: EffectKind) -> bool {
        let before = self.0;
        self.0 |= 1 << effect as u8;
        self.0 != before
    }

    /// Adds every effect from another set and returns whether the set changed.
    pub fn union_with(&mut self, other: Self) -> bool {
        let before = self.0;
        self.0 |= other.0;
        self.0 != before
    }

    /// Iterates effects in stable specification order.
    pub fn iter(self) -> impl Iterator<Item = EffectKind> {
        EffectKind::ALL
            .into_iter()
            .filter(move |effect| self.contains(*effect))
    }
}

/// Direct and transitive effect summary for one HIR function.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FunctionEffects {
    /// Function resolver identity.
    pub symbol: SymbolId,
    /// Source name retained for diagnostics and tooling.
    pub name: String,
    /// Effects caused directly by the body, excluding local callees.
    pub direct: EffectSet,
    /// Fixed-point union of direct effects and all reachable local callees.
    pub inferred: EffectSet,
    /// Direct local call edges in deterministic symbol order.
    pub calls: Vec<SymbolId>,
    /// Cross-file call edges represented by canonical symbol path.
    pub external_calls: Vec<String>,
    /// Function declaration range.
    pub span: Span,
}

/// Effect summaries for all functions in one verified HIR module.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EffectAnalysis {
    /// Functions in the same deterministic source order as HIR.
    pub functions: Vec<FunctionEffects>,
}

/// Source-level effect policy violation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectError {
    /// Stable diagnostic code.
    pub code: &'static str,
    /// Annotation or declaration range responsible for the policy.
    pub span: Span,
    /// Human-readable violation description.
    pub message: String,
}

impl EffectAnalysis {
    /// Looks up a function summary by resolver identity.
    #[must_use]
    pub fn function(&self, symbol: SymbolId) -> Option<&FunctionEffects> {
        self.functions
            .iter()
            .find(|function| function.symbol == symbol)
    }
}

/// Infers direct and transitive effects for every verified HIR function.
#[must_use]
pub fn infer_effects(
    module: &HirModule,
    resolution: &ResolutionOutput,
    types: &TypeStore,
) -> EffectAnalysis {
    let mut analysis = infer_effects_unresolved(module, resolution, types);
    apply_external_summaries(&mut analysis, &BTreeMap::new());
    analysis
}

/// Infers direct and local transitive effects while retaining cross-file call edges.
#[must_use]
pub fn infer_effects_unresolved(
    module: &HirModule,
    resolution: &ResolutionOutput,
    types: &TypeStore,
) -> EffectAnalysis {
    let local_symbols: BTreeSet<_> = module
        .functions
        .iter()
        .map(|function| function.symbol)
        .collect();
    let mut functions: Vec<_> = module
        .functions
        .iter()
        .map(|function| {
            let mut collector = Collector {
                resolution,
                types,
                local_symbols: &local_symbols,
                effects: EffectSet::pure(),
                calls: BTreeSet::new(),
                external_calls: BTreeSet::new(),
            };
            collector.block(&function.body);
            FunctionEffects {
                symbol: function.symbol,
                name: function.name.clone(),
                direct: collector.effects,
                inferred: collector.effects,
                calls: collector.calls.into_iter().collect(),
                external_calls: collector.external_calls.into_iter().collect(),
                span: function.span,
            }
        })
        .collect();

    let indexes: BTreeMap<_, _> = functions
        .iter()
        .enumerate()
        .map(|(index, function)| (function.symbol, index))
        .collect();
    loop {
        let previous: Vec<_> = functions.iter().map(|function| function.inferred).collect();
        let mut changed = false;
        for function in &mut functions {
            let mut inferred = function.direct;
            for callee in &function.calls {
                if let Some(index) = indexes.get(callee) {
                    inferred.union_with(previous[*index]);
                }
            }
            if inferred != function.inferred {
                function.inferred = inferred;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    EffectAnalysis { functions }
}

/// Recomputes transitive effects using portable summaries for cross-file calls.
/// Missing summaries are conservatively classified as [`EffectKind::Unsafe`].
pub fn apply_external_summaries(
    analysis: &mut EffectAnalysis,
    summaries: &BTreeMap<String, EffectSet>,
) {
    let indexes: BTreeMap<_, _> = analysis
        .functions
        .iter()
        .enumerate()
        .map(|(index, function)| (function.symbol, index))
        .collect();
    let mut previous: Vec<_> = analysis
        .functions
        .iter()
        .map(|function| function.direct)
        .collect();
    loop {
        let mut next = Vec::with_capacity(analysis.functions.len());
        for function in &analysis.functions {
            let mut inferred = function.direct;
            for callee in &function.calls {
                if let Some(index) = indexes.get(callee) {
                    inferred.union_with(previous[*index]);
                }
            }
            for path in &function.external_calls {
                if let Some(summary) = summaries.get(path) {
                    inferred.union_with(*summary);
                } else {
                    inferred.insert(EffectKind::Unsafe);
                }
            }
            next.push(inferred);
        }
        if next == previous {
            break;
        }
        previous = next;
    }
    for (function, inferred) in analysis.functions.iter_mut().zip(previous) {
        function.inferred = inferred;
    }
}

/// Checks implemented annotation constraints against inferred effects.
#[must_use]
pub fn check_effect_constraints(module: &HirModule, analysis: &EffectAnalysis) -> Vec<EffectError> {
    let mut errors = Vec::new();
    for function in &module.functions {
        let Some(effects) = analysis.function(function.symbol) else {
            continue;
        };
        if let Some(annotation) = function
            .annotations
            .iter()
            .find(|annotation| annotation.name == "noalloc")
        {
            if effects.inferred.contains(EffectKind::Allocate) {
                errors.push(EffectError {
                    code: "J0600",
                    span: annotation.span,
                    message: format!(
                        "@noalloc function `{}` transitively allocates dynamic storage",
                        function.name
                    ),
                });
            }
            if effects.inferred.contains(EffectKind::Unsafe) {
                errors.push(EffectError {
                    code: "J0601",
                    span: annotation.span,
                    message: format!(
                        "@noalloc function `{}` calls code without a proven effect summary",
                        function.name
                    ),
                });
            }
        }
        if let Some(annotation) = function
            .annotations
            .iter()
            .find(|annotation| annotation.name == "realtime")
        {
            for (effect, code, description) in [
                (
                    EffectKind::Allocate,
                    "J0610",
                    "transitively allocates dynamic storage",
                ),
                (EffectKind::Blocking, "J0611", "may block execution"),
                (
                    EffectKind::Unsafe,
                    "J0612",
                    "calls code without a proven real-time-safe effect summary",
                ),
                (EffectKind::Panic, "J0613", "may trigger a panic"),
            ] {
                if effects.inferred.contains(effect) {
                    errors.push(EffectError {
                        code,
                        span: annotation.span,
                        message: format!("@realtime function `{}` {description}", function.name),
                    });
                }
            }
        }
    }
    errors
}

/// Checks whether `@compute` functions belong to the portable compute subset.
#[must_use]
pub fn check_compute_constraints(
    module: &HirModule,
    analysis: &EffectAnalysis,
    types: &TypeStore,
) -> Vec<EffectError> {
    let mut errors = Vec::new();
    for function in &module.functions {
        let Some(annotation) = function
            .annotations
            .iter()
            .find(|annotation| annotation.name == "compute")
        else {
            continue;
        };
        let Some(effects) = analysis.function(function.symbol) else {
            continue;
        };
        for (effect, code, description) in [
            (EffectKind::Allocate, "J0620", "dynamic allocation"),
            (EffectKind::Io, "J0621", "I/O"),
            (EffectKind::Blocking, "J0622", "blocking execution"),
            (EffectKind::Unsafe, "J0623", "an unsafe or unproven call"),
            (EffectKind::Panic, "J0624", "a possible panic"),
        ] {
            if effects.inferred.contains(effect) {
                errors.push(EffectError {
                    code,
                    span: annotation.span,
                    message: format!(
                        "@compute function `{}` transitively contains {description}",
                        function.name
                    ),
                });
            }
        }
        for parameter in &function.parameters {
            if let Some(reason) = compute_type_violation(types, parameter.ty) {
                errors.push(EffectError {
                    code: "J0625",
                    span: parameter.span,
                    message: format!(
                        "@compute parameter `{}` uses unsupported {reason}",
                        parameter.name
                    ),
                });
            }
        }
        if let Some(reason) = compute_type_violation(types, function.result) {
            errors.push(EffectError {
                code: "J0625",
                span: function.span,
                message: format!(
                    "@compute function `{}` returns unsupported {reason}",
                    function.name
                ),
            });
        }
    }
    errors
}

fn compute_type_violation(types: &TypeStore, ty: TypeId) -> Option<&'static str> {
    match types.kind(ty) {
        Some(TypeKind::String) => Some("String data"),
        Some(TypeKind::Pointer(_)) => Some("raw pointer data"),
        Some(TypeKind::Function { .. }) => Some("first-class function data"),
        Some(TypeKind::Capability {
            capability: Capability::Owned,
            ..
        }) => Some("owned capability"),
        Some(TypeKind::Array { element, .. })
        | Some(TypeKind::Buffer(element))
        | Some(TypeKind::Slice(element))
        | Some(TypeKind::Option(element))
        | Some(TypeKind::Capability { inner: element, .. }) => {
            compute_type_violation(types, *element)
        }
        Some(TypeKind::Result { ok, error }) => {
            compute_type_violation(types, *ok).or_else(|| compute_type_violation(types, *error))
        }
        _ => None,
    }
}

struct Collector<'a> {
    resolution: &'a ResolutionOutput,
    types: &'a TypeStore,
    local_symbols: &'a BTreeSet<SymbolId>,
    effects: EffectSet,
    calls: BTreeSet<SymbolId>,
    external_calls: BTreeSet<String>,
}

impl Collector<'_> {
    fn block(&mut self, block: &HirBlock) {
        for statement in &block.statements {
            self.statement(statement);
        }
    }

    fn statement(&mut self, statement: &HirStatement) {
        match statement {
            HirStatement::Binding { value, .. } | HirStatement::Return { value, .. } => {
                if let Some(value) = value {
                    self.expression(value);
                }
            }
            HirStatement::Region { body, .. } => self.block(body),
            HirStatement::While {
                condition, body, ..
            } => {
                self.expression(condition);
                self.block(body);
            }
            HirStatement::For { iterable, body, .. } => {
                self.expression(iterable);
                self.block(body);
            }
            HirStatement::Break { .. } | HirStatement::Continue { .. } => {}
            HirStatement::Expression { expression, .. } => self.expression(expression),
        }
    }

    fn expression(&mut self, expression: &HirExpression) {
        match &expression.kind {
            HirExpressionKind::Name { .. } => self.capability_read(expression),
            HirExpressionKind::Literal(_) => {}
            HirExpressionKind::Unary { operand, .. }
            | HirExpressionKind::Cast {
                expression: operand,
            }
            | HirExpressionKind::Try { operand, .. }
            | HirExpressionKind::Group(operand) => self.expression(operand),
            HirExpressionKind::Binary {
                left,
                operator,
                right,
            } => {
                if is_assignment(*operator) {
                    self.lvalue(left);
                    self.expression(right);
                    self.effects.insert(EffectKind::Write);
                    if *operator != Operator::Assign {
                        self.expression(left);
                    }
                } else {
                    self.expression(left);
                    self.expression(right);
                }
                if matches!(
                    operator,
                    Operator::Slash
                        | Operator::Percent
                        | Operator::SlashAssign
                        | Operator::PercentAssign
                ) {
                    self.effects.insert(EffectKind::Panic);
                }
            }
            HirExpressionKind::Call { callee, arguments } => {
                for argument in arguments {
                    self.expression(argument);
                }
                self.call(callee);
            }
            HirExpressionKind::RegionAllocate { arguments, .. } => {
                self.effects.insert(EffectKind::Allocate);
                for argument in arguments {
                    self.expression(argument);
                }
            }
            HirExpressionKind::Field { base, .. } => self.expression(base),
            HirExpressionKind::Index { base, index } => {
                self.expression(base);
                self.expression(index);
                self.effects.insert(EffectKind::Panic);
            }
            HirExpressionKind::Array(elements) => {
                for element in elements {
                    self.expression(element);
                }
            }
            HirExpressionKind::Struct { fields, .. } => {
                for (_, value) in fields {
                    self.expression(value);
                }
            }
            HirExpressionKind::Block(block) => self.block(block),
            HirExpressionKind::If {
                condition,
                then_block,
                else_branch,
            } => {
                self.expression(condition);
                self.block(then_block);
                if let Some(branch) = else_branch {
                    self.expression(branch);
                }
            }
            HirExpressionKind::Match { value, arms } => {
                self.expression(value);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.expression(guard);
                    }
                    self.expression(&arm.value);
                }
            }
            HirExpressionKind::Error => {
                self.effects.insert(EffectKind::Unsafe);
            }
        }
    }

    fn call(&mut self, callee: &HirExpression) {
        let HirExpressionKind::Name { name, symbol } = &callee.kind else {
            self.expression(callee);
            self.effects.insert(EffectKind::Unsafe);
            return;
        };
        match name.as_str() {
            "print" => {
                self.effects.insert(EffectKind::Io);
                self.effects.insert(EffectKind::Blocking);
                return;
            }
            "assert_eq" => {
                self.effects.insert(EffectKind::Panic);
                return;
            }
            "vector_splat2" | "vector_load2" | "vector_store2" | "vector_splat3"
            | "vector_load3" | "vector_store3" | "vector_splat4" | "vector_load4"
            | "vector_store4" | "vector_splat8" | "vector_load8" | "vector_store8" => {
                // These are lowered to checked, target-neutral JIR vector
                // instructions; they do not allocate or call unknown code.
                return;
            }
            "jadren_rt_math_acos_f32"
            | "jadren_rt_math_sin_f32"
            | "jadren_rt_math_cos_f32"
            | "jadren_rt_math_sqrt_f32" => {
                // The fixed-name runtime math bridge is an allocation-free,
                // non-blocking C ABI contract. Treating it as pure here lets
                // native kernels retain @noalloc without weakening the
                // conservative unknown-FFI rule below.
                return;
            }
            _ => {}
        }
        let Some(symbol) = symbol else {
            self.effects.insert(EffectKind::Unsafe);
            return;
        };
        if self.local_symbols.contains(symbol) {
            self.calls.insert(*symbol);
            return;
        }
        let Some(symbol) = self.resolution.symbol(*symbol) else {
            self.effects.insert(EffectKind::Unsafe);
            return;
        };
        if symbol.kind == SymbolKind::EnumVariant {
            return;
        }
        if symbol.kind == SymbolKind::Function
            && symbol.origin == SymbolOrigin::Imported
            && let Some(path) = &symbol.canonical_path
        {
            self.external_calls.insert(path.clone());
        } else if matches!(symbol.kind, SymbolKind::BuiltinValue | SymbolKind::Function)
            || symbol.origin == SymbolOrigin::Imported
        {
            self.effects.insert(EffectKind::Unsafe);
        }
    }

    fn lvalue(&mut self, expression: &HirExpression) {
        match &expression.kind {
            HirExpressionKind::Field { base, .. } => self.lvalue(base),
            HirExpressionKind::Index { base, index } => {
                self.lvalue(base);
                self.expression(index);
                self.effects.insert(EffectKind::Panic);
            }
            HirExpressionKind::Group(inner) => self.lvalue(inner),
            _ => {}
        }
    }

    fn capability_read(&mut self, expression: &HirExpression) {
        if matches!(
            self.types.kind(expression.ty),
            Some(TypeKind::Capability {
                capability: Capability::Read | Capability::Write,
                ..
            })
        ) {
            self.effects.insert(EffectKind::Read);
        }
    }
}

const fn is_assignment(operator: Operator) -> bool {
    matches!(
        operator,
        Operator::Assign
            | Operator::PlusAssign
            | Operator::MinusAssign
            | Operator::StarAssign
            | Operator::SlashAssign
            | Operator::PercentAssign
    )
}

#[cfg(test)]
mod tests {
    use jadren_hir::lower_hir;
    use jadren_lexer::lex;
    use jadren_parser::parse;
    use jadren_resolve::resolve;
    use jadren_source::SourceManager;
    use jadren_typeck::check_types;

    use super::{EffectKind, check_compute_constraints, check_effect_constraints, infer_effects};

    #[test]
    fn infers_direct_and_transitive_effects_to_fixed_point() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "effects.jdn",
                "module test; fn top() { wrapper() } fn pure(value: Int32) -> Int32 { return value + value } fn allocate() { region frame { let values: Buffer<Int32> = frame.allocate(4) } } fn wrapper() { allocate() } fn inspect(data: read Buffer<Int32>) -> Int32 { return data[0] } fn update(data: write Buffer<Int32>) { data[0] = 1 } fn display(value: Int32) { print(value) } fn verify(value: Int32) { assert_eq(value, value) }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics);
        let resolution = resolve(source, &parsed.file);
        assert!(!resolution.has_errors(), "{:?}", resolution.diagnostics);
        let checked = check_types(source, &parsed.file, &resolution);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let lowered = lower_hir(source, &parsed.file, &resolution, &checked);
        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);

        let effects = infer_effects(&lowered.module, &resolution, &checked.types);
        let by_name = |name: &str| {
            effects
                .functions
                .iter()
                .find(|function| function.name == name)
                .expect("function")
        };
        assert!(by_name("pure").inferred.is_pure());
        assert!(by_name("allocate").direct.contains(EffectKind::Allocate));
        assert!(by_name("wrapper").direct.is_pure());
        assert!(by_name("wrapper").inferred.contains(EffectKind::Allocate));
        assert!(by_name("top").inferred.contains(EffectKind::Allocate));
        assert!(by_name("inspect").inferred.contains(EffectKind::Read));
        assert!(by_name("inspect").inferred.contains(EffectKind::Panic));
        assert!(by_name("update").inferred.contains(EffectKind::Write));
        assert!(by_name("display").inferred.contains(EffectKind::Io));
        assert!(by_name("verify").inferred.contains(EffectKind::Panic));
    }

    #[test]
    fn vector2_and_vector3_intrinsics_are_pure_for_noalloc() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "vector-effects.jdn",
                "module test; @noalloc fn probe(values: write Slice<Float32>, index: UIntSize, delta: Float32) { let pair: Float2 = vector_load2(values, index); let triple: Float3 = vector_load3(values, index); vector_store2(values, index, pair + vector_splat2(delta)); vector_store3(values, index, triple + vector_splat3(delta)) }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics);
        let resolution = resolve(source, &parsed.file);
        assert!(!resolution.has_errors(), "{:?}", resolution.diagnostics);
        let checked = check_types(source, &parsed.file, &resolution);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let lowered = lower_hir(source, &parsed.file, &resolution, &checked);
        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        let effects = infer_effects(&lowered.module, &resolution, &checked.types);
        let errors = check_effect_constraints(&lowered.module, &effects);
        assert!(errors.is_empty(), "{:?}", errors);
        let probe = effects
            .functions
            .iter()
            .find(|function| function.name == "probe")
            .expect("probe effect summary");
        assert!(!probe.inferred.contains(EffectKind::Allocate));
        assert!(!probe.inferred.contains(EffectKind::Unsafe));
    }

    #[test]
    fn rejects_direct_and_transitive_allocation_in_noalloc_functions() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "noalloc.jdn",
                "module test; fn allocate() { region frame { let values: Buffer<Int32> = frame.allocate(4) } } @noalloc fn direct() { region frame { let values: Buffer<Int32> = frame.allocate(4) } } @noalloc fn transitive() { allocate() } @noalloc fn pure(value: Int32) { print(value) }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics);
        let resolution = resolve(source, &parsed.file);
        assert!(!resolution.has_errors(), "{:?}", resolution.diagnostics);
        let checked = check_types(source, &parsed.file, &resolution);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let lowered = lower_hir(source, &parsed.file, &resolution, &checked);
        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        let effects = infer_effects(&lowered.module, &resolution, &checked.types);
        let errors = check_effect_constraints(&lowered.module, &effects);
        assert_eq!(
            errors.iter().filter(|error| error.code == "J0600").count(),
            2
        );
        assert!(!errors.iter().any(|error| error.message.contains("pure")));
    }

    #[test]
    fn enforces_realtime_allocation_blocking_and_panic_policy() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "realtime.jdn",
                "module test; @realtime fn pure(value: Int32) -> Int32 { return value + value } @realtime fn allocation() { region frame { let values: Buffer<Int32> = frame.allocate(4) } } @realtime fn output(value: Int32) { print(value) } @realtime fn risky(value: Int32) { assert_eq(value, value) }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics);
        let resolution = resolve(source, &parsed.file);
        assert!(!resolution.has_errors(), "{:?}", resolution.diagnostics);
        let checked = check_types(source, &parsed.file, &resolution);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let lowered = lower_hir(source, &parsed.file, &resolution, &checked);
        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        let effects = infer_effects(&lowered.module, &resolution, &checked.types);
        let errors = check_effect_constraints(&lowered.module, &effects);
        for code in ["J0610", "J0611", "J0613"] {
            assert!(errors.iter().any(|error| error.code == code), "{code}");
        }
        assert!(!errors.iter().any(|error| error.message.contains("pure")));
    }

    #[test]
    fn checks_compute_effects_and_signature_types() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "compute.jdn",
                "module test; @compute fn valid(value: Int32) -> Int32 { return value + value } @compute fn allocation() { region frame { let values: Buffer<Int32> = frame.allocate(4) } } @compute fn output(value: Int32) { print(value) } @compute fn text(value: String) {} @compute fn risky(data: read Buffer<Int32>) -> Int32 { return data[0] }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics);
        let resolution = resolve(source, &parsed.file);
        assert!(!resolution.has_errors(), "{:?}", resolution.diagnostics);
        let checked = check_types(source, &parsed.file, &resolution);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let lowered = lower_hir(source, &parsed.file, &resolution, &checked);
        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        let effects = infer_effects(&lowered.module, &resolution, &checked.types);
        let errors = check_compute_constraints(&lowered.module, &effects, &checked.types);
        for code in ["J0620", "J0621", "J0622", "J0624", "J0625"] {
            assert!(errors.iter().any(|error| error.code == code), "{code}");
        }
        assert!(!errors.iter().any(|error| error.message.contains("valid")));
    }
}
