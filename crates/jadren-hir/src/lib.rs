//! Typed high-level IR lowered from resolved and type-checked Jadren AST.

use jadren_diagnostics::Diagnostic;
use jadren_lexer::Operator;
use jadren_parser::{
    AstFile, Block, Expression, Function, Item, LiteralKind, MatchArm, Pattern, Statement,
};
use jadren_resolve::{Namespace, ResolutionOutput, SymbolId};
use jadren_source::{SourceFile, Span};
use jadren_typeck::TypeCheckOutput;
pub use jadren_typeck::{
    ExpressionKind, PropagationKind, PropagationSite, TypedExpression, TypedExpressionId,
    TypedExpressionQuery,
};
use jadren_types::{NominalLayout, TypeId, TypeKind, TypeStore};

/// Fully typed executable high-level representation of one source module.
#[derive(Clone, Debug)]
pub struct HirModule {
    /// Functions in deterministic source order.
    pub functions: Vec<HirFunction>,
    /// Resolver symbol-table size used to validate retained symbol IDs.
    pub symbol_count: usize,
    /// Backend-relevant nominal declaration layouts retained from type checking.
    pub nominal_layouts: Vec<NominalLayout>,
    /// Source-local typed-expression index used to audit HIR hand-off identity.
    pub typed_expressions: Vec<TypedExpression>,
}

impl HirModule {
    /// Starts a deterministic query over the HIR typed-expression index.
    #[must_use]
    pub fn query_typed_expressions(&self) -> TypedExpressionQuery<'_> {
        TypedExpressionQuery::new(&self.typed_expressions)
    }

    /// Walks all HIR expressions in deterministic function/source pre-order.
    ///
    /// The walker yields nested operands exactly once. An expression may carry
    /// `None` as its `typed_id` only for an implicit constructor callee that
    /// was intentionally not emitted as a standalone type-check record.
    #[must_use]
    pub fn walk_typed_expressions(&self) -> HirExpressionWalker<'_> {
        let mut stack = Vec::new();
        for function in self.functions.iter().rev() {
            push_block_expressions(&function.body, &mut stack);
        }
        HirExpressionWalker { stack }
    }

    /// Looks up a retained typed-expression record by its source-local ID.
    #[must_use]
    pub fn typed_expression(&self, id: TypedExpressionId) -> Option<&TypedExpression> {
        self.typed_expressions
            .get(id.index())
            .filter(|expression| expression.id == id)
    }
}

/// One typed function.
#[derive(Clone, Debug)]
pub struct HirFunction {
    /// Resolved declaration identity.
    pub symbol: SymbolId,
    /// Source spelling retained for diagnostics and dumps.
    pub name: String,
    /// Declaration annotations retained for semantic policy passes.
    pub annotations: Vec<HirAnnotation>,
    /// Whether the function explicitly promises pairwise-disjoint borrowed
    /// `Slice`/`Buffer` parameters through `@disjoint`.
    pub disjoint: bool,
    /// Optional stable native export metadata from `@export(...)`.
    pub export: Option<HirExport>,
    /// Canonical function signature type.
    pub signature: TypeId,
    /// Typed parameters in declaration order.
    pub parameters: Vec<HirLocal>,
    /// Canonical result type.
    pub result: TypeId,
    /// Typed body.
    pub body: HirBlock,
    /// Full declaration range.
    pub span: Span,
}

/// Native export metadata retained for MIR/JIR linkage lowering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HirExport {
    /// Exported linker symbol.
    pub name: String,
    /// Calling convention spelling, currently `C`.
    pub abi: String,
    /// Annotation source range.
    pub span: Span,
}

/// One source annotation retained after typed lowering.
#[derive(Clone, Debug)]
pub struct HirAnnotation {
    /// Canonical dot-separated annotation name without `@`.
    pub name: String,
    /// Full annotation range.
    pub span: Span,
}

/// Typed local storage declaration.
#[derive(Clone, Debug)]
pub struct HirLocal {
    /// Resolved symbol identity.
    pub symbol: SymbolId,
    /// Source name.
    pub name: String,
    /// Canonical local type.
    pub ty: TypeId,
    /// Whether source permits reassignment of this binding.
    pub mutable: bool,
    /// Declaration range.
    pub span: Span,
}

/// Typed statement block.
#[derive(Clone, Debug)]
pub struct HirBlock {
    /// Statements in source order.
    pub statements: Vec<HirStatement>,
    /// Full block range.
    pub span: Span,
}

/// Typed statement.
#[derive(Clone, Debug)]
pub enum HirStatement {
    /// Local binding with optional initializer.
    Binding {
        /// Declared local.
        local: HirLocal,
        /// Optional typed initializer.
        value: Option<HirExpression>,
        /// Full statement range.
        span: Span,
    },
    /// Explicit function return.
    Return {
        /// Optional returned value.
        value: Option<HirExpression>,
        /// Full statement range.
        span: Span,
    },
    /// Named lexical allocation region.
    Region {
        /// Region handle local.
        local: HirLocal,
        /// Region body.
        body: HirBlock,
        /// Full statement range.
        span: Span,
    },
    /// Typed `while` loop with a condition and nested body.
    While {
        /// Boolean loop condition.
        condition: HirExpression,
        /// Loop body.
        body: HirBlock,
        /// Full statement range.
        span: Span,
    },
    /// Typed fixed-size array iteration.
    For {
        /// Per-iteration local binding.
        local: HirLocal,
        /// Array expression evaluated once before iteration.
        iterable: HirExpression,
        /// Loop body.
        body: HirBlock,
        /// Full statement range.
        span: Span,
    },
    /// Exits the nearest loop.
    Break {
        /// Full statement range.
        span: Span,
    },
    /// Starts the next iteration of the nearest loop.
    Continue {
        /// Full statement range.
        span: Span,
    },
    /// Expression statement.
    Expression {
        /// Typed expression.
        expression: HirExpression,
        /// Whether source contained an explicit semicolon.
        terminated: bool,
    },
}

/// Typed expression tree.
#[derive(Clone, Debug)]
pub struct HirExpression {
    /// Source-local typed-expression identity, absent only for an implicit
    /// constructor callee that has no standalone type-check record.
    pub typed_id: Option<TypedExpressionId>,
    /// Canonical inferred type.
    pub ty: TypeId,
    /// Source range.
    pub span: Span,
    /// Operation and typed operands.
    pub kind: HirExpressionKind,
}

/// Pre-order iterator over one or more nested HIR expressions.
#[derive(Debug)]
pub struct HirExpressionWalker<'a> {
    stack: Vec<&'a HirExpression>,
}

impl<'a> HirExpressionWalker<'a> {
    fn root(expression: &'a HirExpression) -> Self {
        Self {
            stack: vec![expression],
        }
    }
}

impl<'a> Iterator for HirExpressionWalker<'a> {
    type Item = &'a HirExpression;

    fn next(&mut self) -> Option<Self::Item> {
        let expression = self.stack.pop()?;
        push_expression_children(expression, &mut self.stack);
        Some(expression)
    }
}

impl HirExpression {
    /// Walks this expression and all nested operands in pre-order.
    #[must_use]
    pub fn walk(&self) -> HirExpressionWalker<'_> {
        HirExpressionWalker::root(self)
    }
}

fn push_block_expressions<'a>(block: &'a HirBlock, stack: &mut Vec<&'a HirExpression>) {
    for statement in block.statements.iter().rev() {
        push_statement_expressions(statement, stack);
    }
}

fn push_statement_expressions<'a>(statement: &'a HirStatement, stack: &mut Vec<&'a HirExpression>) {
    match statement {
        HirStatement::Binding { value, .. } | HirStatement::Return { value, .. } => {
            if let Some(value) = value {
                stack.push(value);
            }
        }
        HirStatement::Region { body, .. } => push_block_expressions(body, stack),
        HirStatement::While {
            condition, body, ..
        } => {
            push_block_expressions(body, stack);
            stack.push(condition);
        }
        HirStatement::For { iterable, body, .. } => {
            push_block_expressions(body, stack);
            stack.push(iterable);
        }
        HirStatement::Expression { expression, .. } => stack.push(expression),
        HirStatement::Break { .. } | HirStatement::Continue { .. } => {}
    }
}

fn push_expression_children<'a>(expression: &'a HirExpression, stack: &mut Vec<&'a HirExpression>) {
    match &expression.kind {
        HirExpressionKind::Name { .. }
        | HirExpressionKind::Literal(_)
        | HirExpressionKind::Error => {}
        HirExpressionKind::Unary { operand, .. }
        | HirExpressionKind::Cast {
            expression: operand,
        }
        | HirExpressionKind::Field { base: operand, .. }
        | HirExpressionKind::Try { operand, .. }
        | HirExpressionKind::Group(operand) => stack.push(operand),
        HirExpressionKind::Binary { left, right, .. } => {
            stack.push(right);
            stack.push(left);
        }
        HirExpressionKind::Call { callee, arguments } => {
            stack.extend(arguments.iter().rev());
            stack.push(callee);
        }
        HirExpressionKind::RegionAllocate { arguments, .. } => {
            stack.extend(arguments.iter().rev());
        }
        HirExpressionKind::Index { base, index } => {
            stack.push(index);
            stack.push(base);
        }
        HirExpressionKind::Array(elements) => stack.extend(elements.iter().rev()),
        HirExpressionKind::Struct { fields, .. } => {
            stack.extend(fields.iter().rev().map(|(_, value)| value));
        }
        HirExpressionKind::Block(block) => push_block_expressions(block, stack),
        HirExpressionKind::If {
            condition,
            then_block,
            else_branch,
        } => {
            if let Some(else_branch) = else_branch {
                stack.push(else_branch);
            }
            push_block_expressions(then_block, stack);
            stack.push(condition);
        }
        HirExpressionKind::Match { value, arms } => {
            for arm in arms.iter().rev() {
                stack.push(&arm.value);
                if let Some(guard) = &arm.guard {
                    stack.push(guard);
                }
            }
            stack.push(value);
        }
    }
}

/// Source-exact literal retained for deterministic constant lowering.
#[derive(Clone, Debug)]
pub struct HirLiteral {
    /// Syntactic literal category.
    pub kind: LiteralKind,
    /// Exact UTF-8 token spelling, including suffix and escapes.
    pub text: String,
}

/// High-level typed operations retained before control-flow lowering.
#[derive(Clone, Debug)]
pub enum HirExpressionKind {
    /// Resolved or constructor-like name.
    Name {
        /// Source spelling.
        name: String,
        /// Value symbol when name resolution produced one.
        symbol: Option<SymbolId>,
    },
    /// Source literal.
    Literal(HirLiteral),
    /// Unary operation.
    Unary {
        /// Source operator.
        operator: Operator,
        /// Typed operand.
        operand: Box<HirExpression>,
    },
    /// Explicit numeric cast; the expression type is the cast target.
    Cast {
        /// Typed source operand.
        expression: Box<HirExpression>,
    },
    /// Binary or assignment operation.
    Binary {
        /// Typed left operand.
        left: Box<HirExpression>,
        /// Source operator.
        operator: Operator,
        /// Typed right operand.
        right: Box<HirExpression>,
    },
    /// Function or constructor call.
    Call {
        /// Typed called expression.
        callee: Box<HirExpression>,
        /// Typed arguments.
        arguments: Vec<HirExpression>,
    },
    /// Allocation owned and bulk-freed by a lexical region.
    RegionAllocate {
        /// Region handle symbol.
        region: SymbolId,
        /// Allocation arguments, currently the element count.
        arguments: Vec<HirExpression>,
    },
    /// Field access.
    Field {
        /// Typed base value.
        base: Box<HirExpression>,
        /// Field spelling.
        field: String,
    },
    /// Indexed access.
    Index {
        /// Typed container.
        base: Box<HirExpression>,
        /// Typed index.
        index: Box<HirExpression>,
    },
    /// Option/Result propagation with explicit early-return metadata.
    Try {
        /// Carrier operand.
        operand: Box<HirExpression>,
        /// Selected propagation rule.
        propagation: PropagationSite,
    },
    /// Inline array construction.
    Array(Vec<HirExpression>),
    /// Nominal record/component construction.
    Struct {
        /// Constructed type spelling.
        type_name: String,
        /// Field values in source order.
        fields: Vec<(String, HirExpression)>,
    },
    /// Parenthesized expression retained for source-faithful dumps.
    Group(Box<HirExpression>),
    /// Braced block expression.
    Block(HirBlock),
    /// Conditional expression.
    If {
        /// Boolean condition.
        condition: Box<HirExpression>,
        /// Then branch.
        then_block: HirBlock,
        /// Optional else branch.
        else_branch: Option<Box<HirExpression>>,
    },
    /// Pattern match before CFG lowering.
    Match {
        /// Matched value.
        value: Box<HirExpression>,
        /// Typed arms.
        arms: Vec<HirMatchArm>,
    },
    /// Parser recovery node; verifier rejects it in valid HIR.
    Error,
}

/// One typed match arm.
#[derive(Clone, Debug)]
pub struct HirMatchArm {
    /// Lowered pattern.
    pub pattern: HirPattern,
    /// Optional boolean guard.
    pub guard: Option<HirExpression>,
    /// Typed arm result.
    pub value: HirExpression,
    /// Full arm range.
    pub span: Span,
}

/// HIR pattern retaining bindings and constructor shape.
#[derive(Clone, Debug)]
pub struct HirPattern {
    /// Pattern operation.
    pub kind: HirPatternKind,
    /// Full pattern range.
    pub span: Span,
}

/// Pattern operation.
#[derive(Clone, Debug)]
pub enum HirPatternKind {
    /// Matches any value.
    Wildcard,
    /// Name path, optionally bound to a resolver symbol.
    Path {
        /// Dot-separated spelling.
        path: String,
        /// Binding symbol when the path declares a local.
        binding: Option<SymbolId>,
        /// Canonical binding type when this path declares a local.
        binding_type: Option<TypeId>,
        /// Referenced value symbol when this is a constant/variant path.
        constructor: Option<SymbolId>,
    },
    /// Constructor pattern.
    Constructor {
        /// Dot-separated constructor spelling.
        path: String,
        /// Resolved enum variant/constructor identity.
        constructor: Option<SymbolId>,
        /// Nested payload patterns.
        arguments: Vec<HirPattern>,
    },
    /// Literal pattern.
    Literal(HirLiteral),
    /// Parser recovery pattern.
    Error,
}

/// HIR lowering result with invariant diagnostics.
#[derive(Clone, Debug)]
pub struct HirLoweringOutput {
    /// Typed module, including recoverable error nodes when necessary.
    pub module: HirModule,
    /// Internal lowering diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

/// Lowers a resolved, type-checked AST into typed HIR.
#[must_use]
pub fn lower_hir(
    source: &SourceFile,
    file: &AstFile,
    resolution: &ResolutionOutput,
    type_check: &TypeCheckOutput,
) -> HirLoweringOutput {
    Lowerer {
        source,
        resolution,
        type_check,
        diagnostics: Vec::new(),
    }
    .lower_file(file)
}

struct Lowerer<'a> {
    source: &'a SourceFile,
    resolution: &'a ResolutionOutput,
    type_check: &'a TypeCheckOutput,
    diagnostics: Vec<Diagnostic>,
}

impl Lowerer<'_> {
    fn lower_file(mut self, file: &AstFile) -> HirLoweringOutput {
        let functions = file
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Function(function) => Some(self.lower_function(function)),
                Item::Struct(_) | Item::Component(_) | Item::Enum(_) | Item::ExternBlock(_) => None,
            })
            .collect();
        HirLoweringOutput {
            module: HirModule {
                functions,
                symbol_count: self.resolution.symbols.len(),
                nominal_layouts: self.type_check.nominal_layouts.clone(),
                typed_expressions: self.type_check.expressions.clone(),
            },
            diagnostics: self.diagnostics,
        }
    }

    fn lower_function(&mut self, function: &Function) -> HirFunction {
        let symbol = self.declaration(function.name.span);
        let function_ty = self
            .type_check
            .symbol_type(symbol)
            .unwrap_or(self.type_check.types.core().error);
        let result = match self.type_check.types.kind(function_ty) {
            Some(TypeKind::Function { result, .. }) => *result,
            _ => self.type_check.types.core().error,
        };
        let parameters = function
            .parameters
            .iter()
            .map(|parameter| self.lower_local(&parameter.name.text, parameter.name.span, false))
            .collect();
        HirFunction {
            symbol,
            name: function.name.text.clone(),
            disjoint: function
                .annotations
                .iter()
                .any(|annotation| path_text(&annotation.name) == "disjoint"),
            annotations: function
                .annotations
                .iter()
                .map(|annotation| HirAnnotation {
                    name: path_text(&annotation.name),
                    span: annotation.span,
                })
                .collect(),
            export: function
                .annotations
                .iter()
                .filter(|annotation| path_text(&annotation.name) == "export")
                .find_map(|annotation| self.lower_export(annotation)),
            signature: function_ty,
            parameters,
            result,
            body: self.lower_block(&function.body),
            span: function.span,
        }
    }

    fn lower_export(&self, annotation: &jadren_parser::Annotation) -> Option<HirExport> {
        let mut name = None;
        let mut abi = None;
        for argument in &annotation.arguments {
            let Some(argument_name) = argument.name.as_ref().map(|name| name.text.as_str()) else {
                continue;
            };
            let value = match &argument.value {
                Expression::Name(value) => value.text.clone(),
                Expression::Literal {
                    kind: LiteralKind::String,
                    span,
                } => self
                    .source
                    .slice(*span)
                    .unwrap_or_default()
                    .trim_matches('"')
                    .to_owned(),
                _ => continue,
            };
            match argument_name {
                "name" => name = Some(value),
                "abi" => abi = Some(value),
                _ => {}
            }
        }
        Some(HirExport {
            name: name?,
            abi: abi?,
            span: annotation.span,
        })
    }

    fn lower_local(&self, name: &str, span: Span, mutable: bool) -> HirLocal {
        let symbol = self.declaration(span);
        HirLocal {
            symbol,
            name: name.to_owned(),
            ty: self
                .type_check
                .symbol_type(symbol)
                .unwrap_or(self.type_check.types.core().error),
            mutable,
            span,
        }
    }

    fn lower_block(&mut self, block: &Block) -> HirBlock {
        HirBlock {
            statements: block
                .statements
                .iter()
                .map(|statement| self.lower_statement(statement))
                .collect(),
            span: block.span,
        }
    }

    fn lower_statement(&mut self, statement: &Statement) -> HirStatement {
        match statement {
            Statement::Binding {
                mutable,
                name,
                value,
                span,
                ..
            } => HirStatement::Binding {
                local: self.lower_local(&name.text, name.span, *mutable),
                value: value.as_ref().map(|value| self.lower_expression(value)),
                span: *span,
            },
            Statement::Return { value, span } => HirStatement::Return {
                value: value.as_ref().map(|value| self.lower_expression(value)),
                span: *span,
            },
            Statement::Region { name, body, span } => HirStatement::Region {
                local: self.lower_local(&name.text, name.span, false),
                body: self.lower_block(body),
                span: *span,
            },
            Statement::While {
                condition,
                body,
                span,
            } => HirStatement::While {
                condition: self.lower_expression(condition),
                body: self.lower_block(body),
                span: *span,
            },
            Statement::For {
                binding,
                iterable,
                body,
                span,
            } => HirStatement::For {
                local: self.lower_local(&binding.text, binding.span, false),
                iterable: self.lower_expression(iterable),
                body: self.lower_block(body),
                span: *span,
            },
            Statement::Break { span } => HirStatement::Break { span: *span },
            Statement::Continue { span } => HirStatement::Continue { span: *span },
            Statement::Expression {
                expression,
                terminated,
            } => HirStatement::Expression {
                expression: self.lower_expression(expression),
                terminated: *terminated,
            },
        }
    }

    fn lower_expression(&mut self, expression: &Expression) -> HirExpression {
        let span = expression.span();
        let typed = self.typed_expression_optional(expression);
        let (typed_id, ty) = typed.map_or_else(
            || {
                self.diagnostics.push(Diagnostic::error(
                    "J0400",
                    "typed HIR expression is missing an inferred type",
                    span,
                    "frontend invariant failed before HIR lowering",
                ));
                (None, self.type_check.types.core().error)
            },
            |expression| (Some(expression.id), expression.ty),
        );
        let region_allocation = self
            .type_check
            .region_allocations
            .iter()
            .find(|site| site.span == span)
            .copied();
        let kind = if let Some(site) = region_allocation {
            let arguments = match expression {
                Expression::Call { arguments, .. } => arguments
                    .iter()
                    .map(|argument| self.lower_expression(argument))
                    .collect(),
                _ => Vec::new(),
            };
            HirExpressionKind::RegionAllocate {
                region: site.region,
                arguments,
            }
        } else {
            match expression {
                Expression::Name(name) => HirExpressionKind::Name {
                    name: name.text.clone(),
                    symbol: self.reference(name.span, Namespace::Value),
                },
                Expression::Literal { kind, span } => {
                    HirExpressionKind::Literal(self.lower_literal(*kind, *span))
                }
                Expression::Unary {
                    operator, operand, ..
                } => HirExpressionKind::Unary {
                    operator: *operator,
                    operand: Box::new(self.lower_expression(operand)),
                },
                Expression::Cast { expression, .. } => HirExpressionKind::Cast {
                    expression: Box::new(self.lower_expression(expression)),
                },
                Expression::Binary {
                    left,
                    operator,
                    right,
                    ..
                } => HirExpressionKind::Binary {
                    left: Box::new(self.lower_expression(left)),
                    operator: *operator,
                    right: Box::new(self.lower_expression(right)),
                },
                Expression::Call {
                    callee, arguments, ..
                } => HirExpressionKind::Call {
                    callee: Box::new(self.lower_callee(callee)),
                    arguments: arguments
                        .iter()
                        .map(|argument| self.lower_expression(argument))
                        .collect(),
                },
                Expression::Field { base, field, .. } => {
                    if let Some(symbol) = self.reference(span, Namespace::Value) {
                        HirExpressionKind::Name {
                            name: expression_path(expression).unwrap_or_else(|| field.text.clone()),
                            symbol: Some(symbol),
                        }
                    } else {
                        HirExpressionKind::Field {
                            base: Box::new(self.lower_expression(base)),
                            field: field.text.clone(),
                        }
                    }
                }
                Expression::Index { base, index, .. } => HirExpressionKind::Index {
                    base: Box::new(self.lower_expression(base)),
                    index: Box::new(self.lower_expression(index)),
                },
                Expression::Try { operand, .. } => {
                    let propagation = self
                        .type_check
                        .propagation_sites
                        .iter()
                        .find(|site| site.span == span)
                        .copied()
                        .unwrap_or(PropagationSite {
                            span,
                            kind: PropagationKind::OptionNone,
                            success_type: self.type_check.types.core().error,
                            residual_type: self.type_check.types.core().error,
                            return_type: self.type_check.types.core().error,
                        });
                    HirExpressionKind::Try {
                        operand: Box::new(self.lower_expression(operand)),
                        propagation,
                    }
                }
                Expression::Array { elements, .. } => HirExpressionKind::Array(
                    elements
                        .iter()
                        .map(|element| self.lower_expression(element))
                        .collect(),
                ),
                Expression::StructLiteral { ty, fields, .. } => HirExpressionKind::Struct {
                    type_name: expression_path(ty).unwrap_or_else(|| "<error>".to_owned()),
                    fields: fields
                        .iter()
                        .map(|field| (field.name.text.clone(), self.lower_expression(&field.value)))
                        .collect(),
                },
                Expression::Group { expression, .. } => {
                    HirExpressionKind::Group(Box::new(self.lower_expression(expression)))
                }
                Expression::Block(block) => HirExpressionKind::Block(self.lower_block(block)),
                Expression::If {
                    condition,
                    then_block,
                    else_branch,
                    ..
                } => HirExpressionKind::If {
                    condition: Box::new(self.lower_expression(condition)),
                    then_block: self.lower_block(then_block),
                    else_branch: else_branch
                        .as_ref()
                        .map(|branch| Box::new(self.lower_expression(branch))),
                },
                Expression::Match { value, arms, .. } => HirExpressionKind::Match {
                    value: Box::new(self.lower_expression(value)),
                    arms: arms.iter().map(|arm| self.lower_match_arm(arm)).collect(),
                },
                Expression::Error(_) => HirExpressionKind::Error,
            }
        };
        HirExpression {
            typed_id,
            ty,
            span,
            kind,
        }
    }

    fn lower_match_arm(&mut self, arm: &MatchArm) -> HirMatchArm {
        HirMatchArm {
            pattern: self.lower_pattern(&arm.pattern),
            guard: arm.guard.as_ref().map(|guard| self.lower_expression(guard)),
            value: self.lower_expression(&arm.value),
            span: arm.span,
        }
    }

    fn lower_callee(&mut self, expression: &Expression) -> HirExpression {
        if self.typed_expression_optional(expression).is_some() {
            return self.lower_expression(expression);
        }
        HirExpression {
            typed_id: None,
            ty: self.type_check.types.core().error,
            span: expression.span(),
            kind: HirExpressionKind::Name {
                name: expression_path(expression).unwrap_or_else(|| "<constructor>".to_owned()),
                symbol: self.reference(expression.span(), Namespace::Value),
            },
        }
    }

    fn lower_pattern(&self, pattern: &Pattern) -> HirPattern {
        let kind = match pattern {
            Pattern::Wildcard(_) => HirPatternKind::Wildcard,
            Pattern::Path(path) => {
                let binding = path
                    .segments
                    .first()
                    .and_then(|name| self.declaration_optional(name.span));
                HirPatternKind::Path {
                    path: path_text(path),
                    binding,
                    binding_type: binding.and_then(|symbol| self.type_check.symbol_type(symbol)),
                    constructor: self.reference(path.span, Namespace::Value),
                }
            }
            Pattern::Constructor {
                path, arguments, ..
            } => HirPatternKind::Constructor {
                path: path_text(path),
                constructor: self.reference(path.span, Namespace::Value),
                arguments: arguments
                    .iter()
                    .map(|argument| self.lower_pattern(argument))
                    .collect(),
            },
            Pattern::Literal { kind, span } => {
                HirPatternKind::Literal(self.lower_literal(*kind, *span))
            }
            Pattern::Error(_) => HirPatternKind::Error,
        };
        HirPattern {
            kind,
            span: pattern.span(),
        }
    }

    fn lower_literal(&self, kind: LiteralKind, span: Span) -> HirLiteral {
        HirLiteral {
            kind,
            text: self.source.slice(span).unwrap_or_default().to_owned(),
        }
    }

    fn typed_expression_optional(&self, expression: &Expression) -> Option<TypedExpression> {
        let span = expression.span();
        let kind = ExpressionKind::of(expression);
        self.type_check
            .expressions
            .iter()
            .rev()
            .find(|typed| typed.span == span && typed.kind == kind)
            .copied()
    }

    fn declaration(&self, span: Span) -> SymbolId {
        self.declaration_optional(span)
            .unwrap_or_else(|| self.resolution.symbols[0].id)
    }

    fn declaration_optional(&self, span: Span) -> Option<SymbolId> {
        self.resolution
            .symbols
            .iter()
            .find(|symbol| symbol.span == span)
            .map(|symbol| symbol.id)
    }

    fn reference(&self, span: Span, namespace: Namespace) -> Option<SymbolId> {
        self.resolution
            .references
            .iter()
            .find(|reference| reference.span == span && reference.namespace == namespace)
            .map(|reference| reference.symbol)
    }
}

/// One verifier failure. HIR must never reach later compiler phases with these errors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HirVerificationError {
    /// Source range associated with the invalid node.
    pub span: Span,
    /// Stable explanatory text for tests and internal diagnostics.
    pub message: String,
}

/// Verifies type and symbol invariants required by later control-flow lowering.
#[must_use]
pub fn verify_hir(module: &HirModule, types: &TypeStore) -> Vec<HirVerificationError> {
    let mut verifier = Verifier {
        module,
        types,
        errors: Vec::new(),
    };
    for function in &module.functions {
        verifier.verify_function(function);
    }
    verifier.errors
}

struct Verifier<'a> {
    module: &'a HirModule,
    types: &'a TypeStore,
    errors: Vec<HirVerificationError>,
}

impl Verifier<'_> {
    fn verify_function(&mut self, function: &HirFunction) {
        self.verify_symbol(function.symbol, function.span);
        self.verify_type(function.signature, function.span);
        self.verify_type(function.result, function.span);
        for parameter in &function.parameters {
            self.verify_local(parameter);
        }
        match self.types.kind(function.signature).cloned() {
            Some(TypeKind::Function { parameters, result }) => {
                if result != function.result {
                    self.error(
                        function.span,
                        "HIR function result differs from its signature",
                    );
                }
                if parameters.len() != function.parameters.len()
                    || parameters
                        .iter()
                        .zip(&function.parameters)
                        .any(|(expected, actual)| *expected != actual.ty)
                {
                    self.error(
                        function.span,
                        "HIR parameters differ from the function signature",
                    );
                }
            }
            Some(_) => self.error(
                function.span,
                "HIR function signature is not a function type",
            ),
            None => {}
        }
        self.verify_block(&function.body, function.result);
    }

    fn verify_local(&mut self, local: &HirLocal) {
        self.verify_symbol(local.symbol, local.span);
        self.verify_type(local.ty, local.span);
    }

    fn verify_block(&mut self, block: &HirBlock, return_type: TypeId) {
        for statement in &block.statements {
            match statement {
                HirStatement::Binding { local, value, .. } => {
                    self.verify_local(local);
                    if let Some(value) = value {
                        self.verify_expression(value, return_type);
                        if !self.types_compatible(local.ty, value.ty)
                            && !self.is_error(value.ty)
                            && !self.is_error(local.ty)
                        {
                            self.error(
                                value.span,
                                "binding initializer type differs from local type",
                            );
                        }
                    }
                }
                HirStatement::Return { value, span } => {
                    if let Some(value) = value {
                        self.verify_expression(value, return_type);
                        if !self.types_compatible(return_type, value.ty)
                            && !self.is_never(value.ty)
                            && !self.is_error(value.ty)
                            && !self.is_error(return_type)
                        {
                            self.error(*span, "return value type differs from function result");
                        }
                    } else if return_type != self.types.core().unit && !self.is_error(return_type) {
                        self.error(*span, "empty return requires Unit function result");
                    }
                }
                HirStatement::Region { local, body, .. } => {
                    self.verify_local(local);
                    self.verify_block(body, return_type);
                }
                HirStatement::While {
                    condition,
                    body,
                    span,
                } => {
                    self.verify_expression(condition, return_type);
                    if !self.types_compatible(condition.ty, self.types.core().bool_)
                        && !self.is_error(condition.ty)
                    {
                        self.error(*span, "while condition must be Bool");
                    }
                    self.verify_block(body, return_type);
                }
                HirStatement::For {
                    local,
                    iterable,
                    body,
                    ..
                } => {
                    self.verify_expression(iterable, return_type);
                    self.verify_local(local);
                    self.verify_block(body, return_type);
                }
                HirStatement::Break { .. } | HirStatement::Continue { .. } => {}
                HirStatement::Expression { expression, .. } => {
                    self.verify_expression(expression, return_type);
                }
            }
        }
    }

    fn verify_expression(&mut self, expression: &HirExpression, return_type: TypeId) {
        self.verify_type(expression.ty, expression.span);
        if let Some(typed_id) = expression.typed_id {
            match self.module.typed_expression(typed_id) {
                Some(typed) if typed.span == expression.span && typed.ty == expression.ty => {}
                Some(_) => self.error(
                    expression.span,
                    "HIR typed-expression identity disagrees with its source span or type",
                ),
                None => self.error(
                    expression.span,
                    "HIR typed-expression identity is outside the source index",
                ),
            }
        }
        match &expression.kind {
            HirExpressionKind::Name { symbol, .. } => {
                if let Some(symbol) = symbol {
                    self.verify_symbol(*symbol, expression.span);
                }
            }
            HirExpressionKind::Literal(_) => {}
            HirExpressionKind::Unary { operand, .. } => {
                self.verify_expression(operand, return_type);
            }
            HirExpressionKind::Cast { expression } => {
                self.verify_expression(expression, return_type);
            }
            HirExpressionKind::Binary { left, right, .. } => {
                self.verify_expression(left, return_type);
                self.verify_expression(right, return_type);
            }
            HirExpressionKind::Call { callee, arguments } => {
                self.verify_expression(callee, return_type);
                for argument in arguments {
                    self.verify_expression(argument, return_type);
                }
            }
            HirExpressionKind::RegionAllocate { region, arguments } => {
                self.verify_symbol(*region, expression.span);
                for argument in arguments {
                    self.verify_expression(argument, return_type);
                }
            }
            HirExpressionKind::Field { base, .. } => self.verify_expression(base, return_type),
            HirExpressionKind::Index { base, index } => {
                self.verify_expression(base, return_type);
                self.verify_expression(index, return_type);
            }
            HirExpressionKind::Try {
                operand,
                propagation,
            } => {
                self.verify_expression(operand, return_type);
                self.verify_type(propagation.success_type, expression.span);
                self.verify_type(propagation.residual_type, expression.span);
                self.verify_type(propagation.return_type, expression.span);
                if propagation.span != expression.span || propagation.success_type != expression.ty
                {
                    self.error(
                        expression.span,
                        "try expression propagation metadata is inconsistent",
                    );
                }
            }
            HirExpressionKind::Array(elements) => {
                for element in elements {
                    self.verify_expression(element, return_type);
                }
            }
            HirExpressionKind::Struct { fields, .. } => {
                for (_, value) in fields {
                    self.verify_expression(value, return_type);
                }
            }
            HirExpressionKind::Group(inner) => {
                self.verify_expression(inner, return_type);
                if inner.ty != expression.ty {
                    self.error(expression.span, "group expression changed its inner type");
                }
            }
            HirExpressionKind::Block(block) => self.verify_block(block, return_type),
            HirExpressionKind::If {
                condition,
                then_block,
                else_branch,
            } => {
                self.verify_expression(condition, return_type);
                if self.types.kind(condition.ty) != Some(&TypeKind::Bool) {
                    self.error(condition.span, "if condition is not Bool");
                }
                self.verify_block(then_block, return_type);
                if let Some(branch) = else_branch {
                    self.verify_expression(branch, return_type);
                }
            }
            HirExpressionKind::Match { value, arms } => {
                self.verify_expression(value, return_type);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.verify_expression(guard, return_type);
                        if self.types.kind(guard.ty) != Some(&TypeKind::Bool) {
                            self.error(guard.span, "match guard is not Bool");
                        }
                    }
                    self.verify_expression(&arm.value, return_type);
                    if arm.value.ty != expression.ty && !self.is_error(arm.value.ty) {
                        self.error(arm.span, "match arm type differs from match result");
                    }
                    self.verify_pattern(&arm.pattern);
                }
            }
            HirExpressionKind::Error => {
                self.error(expression.span, "error expression reached verified HIR");
            }
        }
    }

    fn verify_pattern(&mut self, pattern: &HirPattern) {
        match &pattern.kind {
            HirPatternKind::Path {
                binding,
                binding_type,
                constructor,
                ..
            } => {
                if let Some(binding) = binding {
                    self.verify_symbol(*binding, pattern.span);
                    if let Some(binding_type) = binding_type {
                        self.verify_type(*binding_type, pattern.span);
                    } else {
                        self.error(pattern.span, "pattern binding is missing its type");
                    }
                } else if let Some(constructor) = constructor {
                    self.verify_symbol(*constructor, pattern.span);
                }
            }
            HirPatternKind::Constructor {
                path,
                constructor,
                arguments,
                ..
            } => {
                if let Some(constructor) = constructor {
                    self.verify_symbol(*constructor, pattern.span);
                } else if path.is_empty() {
                    self.error(pattern.span, "pattern constructor has no identity");
                }
                for argument in arguments {
                    self.verify_pattern(argument);
                }
            }
            HirPatternKind::Error => self.error(pattern.span, "error pattern reached verified HIR"),
            HirPatternKind::Wildcard | HirPatternKind::Literal(_) => {}
        }
    }

    fn verify_type(&mut self, ty: TypeId, span: Span) {
        if self.types.kind(ty).is_none() {
            self.error(span, "HIR references a TypeId outside its TypeStore");
        }
    }

    fn verify_symbol(&mut self, symbol: SymbolId, span: Span) {
        if symbol.index() >= self.module.symbol_count {
            self.error(
                span,
                "HIR references a SymbolId outside its resolution table",
            );
        }
    }

    fn is_error(&self, ty: TypeId) -> bool {
        self.types.kind(ty) == Some(&TypeKind::Error)
    }

    fn is_never(&self, ty: TypeId) -> bool {
        self.types.kind(ty) == Some(&TypeKind::Never)
    }

    fn types_compatible(&self, expected: TypeId, actual: TypeId) -> bool {
        if expected == actual {
            return true;
        }
        matches!(
            self.types.kind(expected),
            Some(TypeKind::Capability {
                capability: jadren_types::Capability::Owned
                    | jadren_types::Capability::Read
                    | jadren_types::Capability::Write,
                inner,
            }) if *inner == actual
        )
    }

    fn error(&mut self, span: Span, message: &str) {
        self.errors.push(HirVerificationError {
            span,
            message: message.to_owned(),
        });
    }
}

fn path_text(path: &jadren_parser::Path) -> String {
    path.segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect::<Vec<_>>()
        .join(".")
}

fn expression_path(expression: &Expression) -> Option<String> {
    match expression {
        Expression::Name(name) => Some(name.text.clone()),
        Expression::Field { base, field, .. } => {
            let mut path = expression_path(base)?;
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
    use jadren_source::SourceManager;
    use jadren_typeck::check_types;

    use super::{HirExpressionKind, HirStatement, TypedExpressionId, lower_hir, verify_hir};

    #[test]
    fn lowers_and_verifies_typed_control_flow_and_propagation() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "test.jdn",
                "module test; fn load() -> Result<Int32, String> { return Ok(1) } fn run(flag: Bool) -> Result<Int32, String> { let value = if flag { load()? } else { 0 }; return Ok(value) }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolution = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolution);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);

        let lowered = lower_hir(source, &parsed.file, &resolution, &checked);
        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        assert!(verify_hir(&lowered.module, &checked.types).is_empty());
        assert_eq!(lowered.module.functions.len(), 2);
        assert_eq!(
            lowered.module.typed_expressions.len(),
            checked.expressions.len()
        );
        assert!(
            lowered
                .module
                .typed_expressions
                .iter()
                .enumerate()
                .all(|(index, expression)| expression.id.index() == index)
        );
        assert!(
            lowered
                .module
                .typed_expressions
                .iter()
                .all(|expression| lowered.module.typed_expression(expression.id).is_some())
        );
        let source = lowered.module.typed_expressions[0].span.source;
        assert!(
            lowered
                .module
                .query_typed_expressions()
                .source(source)
                .kind(super::ExpressionKind::If)
                .first()
                .is_some()
        );
        let walked = lowered.module.walk_typed_expressions().collect::<Vec<_>>();
        assert!(!walked.is_empty());
        assert!(
            walked
                .windows(2)
                .all(|window| window[0].span.start <= window[1].span.start)
        );
        let mut walked_ids = walked
            .iter()
            .filter_map(|expression| expression.typed_id)
            .collect::<Vec<_>>();
        let mut indexed_ids = lowered
            .module
            .typed_expressions
            .iter()
            .map(|expression| expression.id)
            .collect::<Vec<_>>();
        walked_ids.sort_unstable();
        indexed_ids.sort_unstable();
        assert_eq!(walked_ids, indexed_ids);
        assert!(module_contains_try(&lowered.module));
    }

    #[test]
    fn verifier_rejects_return_type_corruption() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "test.jdn",
                "module test; fn value() -> Bool { return true }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolution = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolution);
        let mut module = lower_hir(source, &parsed.file, &resolution, &checked).module;
        module.functions[0].result = checked.types.core().int32;

        let errors = verify_hir(&module, &checked.types);
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("return value"))
        );
    }

    #[test]
    fn verifier_rejects_typed_expression_identity_corruption() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "test.jdn",
                "module test; fn value() -> Bool { return true }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolution = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolution);
        let mut module = lower_hir(source, &parsed.file, &resolution, &checked).module;
        let HirStatement::Return {
            value: Some(value), ..
        } = &mut module.functions[0].body.statements[0]
        else {
            panic!("expected return value");
        };
        value.typed_id = Some(TypedExpressionId::default());
        module.typed_expressions.clear();

        let errors = verify_hir(&module, &checked.types);
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("typed-expression identity"))
        );
    }

    #[test]
    fn preserves_exact_expression_and_pattern_literal_text() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "literals.jdn",
                "module test; fn value() -> UInt64 { return 42u64 } fn classify(value: Int32) -> Bool { return match value { 7 => true, _ => false } }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolution = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolution);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let module = lower_hir(source, &parsed.file, &resolution, &checked).module;

        let HirStatement::Return {
            value: Some(value), ..
        } = &module.functions[0].body.statements[0]
        else {
            panic!("expected return literal");
        };
        let HirExpressionKind::Literal(literal) = &value.kind else {
            panic!("expected literal");
        };
        assert_eq!(literal.text, "42u64");

        let HirStatement::Return {
            value: Some(value), ..
        } = &module.functions[1].body.statements[0]
        else {
            panic!("expected match return");
        };
        let HirExpressionKind::Match { arms, .. } = &value.kind else {
            panic!("expected match");
        };
        let super::HirPatternKind::Literal(literal) = &arms[0].pattern.kind else {
            panic!("expected literal pattern");
        };
        assert_eq!(literal.text, "7");
    }

    fn module_contains_try(module: &super::HirModule) -> bool {
        module
            .functions
            .iter()
            .any(|function| function.body.statements.iter().any(statement_contains_try))
    }

    fn statement_contains_try(statement: &HirStatement) -> bool {
        match statement {
            HirStatement::Binding {
                value: Some(value), ..
            }
            | HirStatement::Return {
                value: Some(value), ..
            }
            | HirStatement::Expression {
                expression: value, ..
            } => expression_contains_try(value),
            HirStatement::Region { body, .. } => body.statements.iter().any(statement_contains_try),
            HirStatement::While {
                condition, body, ..
            } => {
                expression_contains_try(condition)
                    || body.statements.iter().any(statement_contains_try)
            }
            HirStatement::For { iterable, body, .. } => {
                expression_contains_try(iterable)
                    || body.statements.iter().any(statement_contains_try)
            }
            HirStatement::Break { .. } | HirStatement::Continue { .. } => false,
            HirStatement::Binding { value: None, .. }
            | HirStatement::Return { value: None, .. } => false,
        }
    }

    fn expression_contains_try(expression: &super::HirExpression) -> bool {
        match &expression.kind {
            HirExpressionKind::Try { .. } => true,
            HirExpressionKind::If {
                condition,
                then_block,
                else_branch,
            } => {
                expression_contains_try(condition)
                    || then_block
                        .statements
                        .iter()
                        .any(|statement| match statement {
                            HirStatement::Expression { expression, .. } => {
                                expression_contains_try(expression)
                            }
                            _ => false,
                        })
                    || else_branch.as_deref().is_some_and(expression_contains_try)
            }
            HirExpressionKind::Group(inner) => expression_contains_try(inner),
            HirExpressionKind::Cast { expression } => expression_contains_try(expression),
            _ => false,
        }
    }
}
