//! Place-based MIR, structural verification, and initial memory dataflow analyses.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use jadren_hir::{
    HirBlock, HirExport, HirExpression, HirExpressionKind, HirFunction, HirLiteral, HirMatchArm,
    HirModule, HirPattern, HirPatternKind, HirStatement, PropagationKind, TypedExpression,
    TypedExpressionQuery,
};
use jadren_lexer::Operator;
use jadren_parser::LiteralKind;
use jadren_resolve::SymbolId;
use jadren_source::Span;
use jadren_types::{Capability, NominalLayout, TypeId, TypeKind, TypeStore};

/// Function-local storage slot.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LocalId(usize);

impl LocalId {
    /// Returns the deterministic function-local index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// Control-flow basic block identity.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BasicBlockId(usize);

impl BasicBlockId {
    /// Returns the deterministic function-local block index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// One MIR local.
#[derive(Clone, Debug)]
pub struct MirLocal {
    /// Stable local identity.
    pub id: LocalId,
    /// Resolver symbol for user-visible storage; absent for future temporaries.
    pub symbol: Option<SymbolId>,
    /// Debug/source name.
    pub name: String,
    /// Canonical value type.
    pub ty: TypeId,
    /// Whether reassignment is allowed.
    pub mutable: bool,
    /// Parameters are initialized on function entry.
    pub is_parameter: bool,
    /// Dedicated temporary carrying a function return value across drop statements.
    pub is_return: bool,
    /// Lexical region containing this declaration, if any.
    pub scope_region: Option<LocalId>,
    /// Region that owns the local value allocation, if any.
    pub owned_region: Option<LocalId>,
    /// Declaration range.
    pub span: Span,
}

/// Addressable local or subplace.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Place {
    /// Root storage slot.
    pub local: LocalId,
    /// Ordered projection from the root.
    pub projection: Vec<Projection>,
}

impl Place {
    /// Creates a whole-local place.
    #[must_use]
    pub const fn local(local: LocalId) -> Self {
        Self {
            local,
            projection: Vec::new(),
        }
    }

    /// Returns whether two places may overlap.
    #[must_use]
    pub fn overlaps(&self, other: &Self) -> bool {
        self.local == other.local
            && (self.projection.starts_with(&other.projection)
                || other.projection.starts_with(&self.projection))
    }
}

/// One place projection.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Projection {
    /// Named record/component field.
    Field(String),
    /// Dynamically indexed element.
    Index,
    /// Pointer/reference dereference.
    Dereference,
}

/// Memory effect of evaluating one operand.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessKind {
    /// Non-consuming read of a copy value.
    Read,
    /// Consuming read of a move-only value.
    Move,
    /// Write through an existing place.
    Write,
    /// Creates a shared read-only borrow for the evaluation duration.
    BorrowRead,
    /// Creates an exclusive writable borrow for the evaluation duration.
    BorrowWrite,
}

/// Persistent borrow stored in a capability local.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum BorrowKind {
    /// Shared read-only borrow.
    Read,
    /// Exclusive writable borrow.
    Write,
}

/// One source-mapped place access.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlaceAccess {
    /// Accessed place.
    pub place: Place,
    /// Read, move, or write behavior.
    pub kind: AccessKind,
    /// Source range that caused the access.
    pub span: Span,
}

/// Typed value computation retained independently from memory accesses.
#[derive(Clone, Debug)]
pub struct MirOperand {
    /// Canonical result type.
    pub ty: TypeId,
    /// Value-producing operation.
    pub kind: MirOperandKind,
    /// Source range.
    pub span: Span,
}

/// Value semantics required by later SSA lowering.
#[derive(Clone, Debug)]
pub enum MirOperandKind {
    /// Unit value with no runtime payload.
    Unit,
    /// Reads or moves an addressable place according to the parallel access list.
    Place(Place),
    /// Source-exact literal.
    Literal(HirLiteral),
    /// Function, builtin, or constructor identity.
    Function {
        name: String,
        symbol: Option<SymbolId>,
    },
    /// Unary value operation.
    Unary {
        operator: Operator,
        operand: Box<MirOperand>,
    },
    /// Explicit numeric cast lowered to a target-typed JIR cast.
    Cast { operand: Box<MirOperand> },
    /// Binary value operation.
    Binary {
        left: Box<MirOperand>,
        operator: Operator,
        right: Box<MirOperand>,
    },
    /// Function or constructor invocation.
    Call {
        callee: Box<MirOperand>,
        arguments: Vec<MirOperand>,
    },
    /// Allocation owned by a lexical region.
    RegionAllocate {
        region: Option<LocalId>,
        arguments: Vec<MirOperand>,
    },
    /// Dynamic index operation retaining the exact index value.
    Index {
        base: Box<MirOperand>,
        index: Box<MirOperand>,
    },
    /// Runtime length of an array, Buffer, or Slice iterable.
    Length { base: Box<MirOperand> },
    /// Non-place field projection, such as a field of a temporary aggregate.
    Field {
        base: Box<MirOperand>,
        field: String,
    },
    /// Inline aggregate array value.
    Array(Vec<MirOperand>),
    /// Named aggregate field values in source order.
    Struct {
        type_name: String,
        fields: Vec<(String, MirOperand)>,
    },
    /// Extracts one matched constructor payload field from a discriminant temporary.
    PatternExtract {
        source: Place,
        /// Dynamic index operands paired with `Projection::Index` segments in `source`.
        source_indices: Vec<MirOperand>,
        path: Vec<String>,
        borrowed: bool,
    },
    /// Extracts the success or residual payload of an Option/Result carrier.
    CarrierExtract {
        source: Place,
        /// Dynamic index operands paired with `Projection::Index` segments in `source`.
        source_indices: Vec<MirOperand>,
        part: CarrierPart,
    },
    /// Constructs the caller's residual carrier on a propagation return edge.
    PropagateResidual {
        source: Place,
        kind: MirPropagationKind,
        residual_type: TypeId,
    },
    /// Transitional high-level value eliminated by JAD-515C CFG lowering.
    HighLevel(Box<HirExpression>),
}

/// MIR statement before borrow/drop elaboration.
#[derive(Clone, Debug)]
pub enum MirStatement {
    /// Begins one local storage lifetime.
    StorageLive { local: LocalId, span: Span },
    /// Ends one local storage lifetime.
    StorageDead { local: LocalId, span: Span },
    /// Creates one lexical region handle.
    RegionEnter { region: LocalId, span: Span },
    /// Bulk-releases allocations owned by a lexical region.
    RegionExit { region: LocalId, span: Span },
    /// Initializes or overwrites a place after evaluating source operands.
    Assign {
        destination: Place,
        /// Dynamic index operands paired with `Projection::Index` segments in order.
        destination_indices: Vec<MirOperand>,
        /// Typed value being assigned. `None` is reserved for synthetic test/transition nodes.
        value: Option<MirOperand>,
        accesses: Vec<PlaceAccess>,
        span: Span,
    },
    /// Stores a borrow in a capability local.
    Borrow {
        /// Local holding the borrow handle.
        destination: LocalId,
        /// Borrowed source place.
        source: Place,
        /// Shared or exclusive capability.
        kind: BorrowKind,
        /// Source range.
        span: Span,
    },
    /// Deterministically destroys one still-owned move-only place.
    Drop {
        /// Value being destroyed.
        place: Place,
        /// Source range of the exiting edge.
        span: Span,
    },
    /// Evaluates side effects without retaining a result.
    Evaluate {
        /// Typed source computation; absent only for return materialization before JAD-515B.
        value: Option<MirOperand>,
        accesses: Vec<PlaceAccess>,
        span: Span,
    },
}

/// End of one basic block.
#[derive(Clone, Debug)]
pub enum Terminator {
    /// Unconditional control-flow edge.
    Goto { target: BasicBlockId, span: Span },
    /// Multi-way conditional edge.
    Switch {
        /// Typed discriminant value.
        value: Option<MirOperand>,
        discriminant: Vec<PlaceAccess>,
        targets: Vec<BasicBlockId>,
        otherwise: BasicBlockId,
        span: Span,
    },
    /// Tests one match pattern and branches to its arm or the next pattern test.
    Match {
        value: MirOperand,
        accesses: Vec<PlaceAccess>,
        pattern: MirPattern,
        matched: BasicBlockId,
        otherwise: BasicBlockId,
        span: Span,
    },
    /// Branches on Option/Result success versus early-return residual.
    Propagate {
        value: MirOperand,
        accesses: Vec<PlaceAccess>,
        kind: MirPropagationKind,
        success: BasicBlockId,
        residual: BasicBlockId,
        span: Span,
    },
    /// Function return after evaluating its optional value.
    Return {
        /// Typed return value, normally a dedicated return local after materialization.
        value: Option<MirOperand>,
        accesses: Vec<PlaceAccess>,
        span: Span,
    },
    /// Block with no legal successor.
    Unreachable { span: Span },
}

/// Payload selected from a carrier temporary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CarrierPart {
    Success,
    Residual,
}

/// Option/Result propagation family retained by MIR.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MirPropagationKind {
    OptionNone,
    ResultError,
}

/// Pattern predicate retained in a match-dispatch terminator.
#[derive(Clone, Debug)]
pub enum MirPattern {
    Wildcard,
    Binding,
    Path {
        path: String,
        constructor: Option<SymbolId>,
    },
    Constructor {
        path: String,
        constructor: Option<SymbolId>,
        arguments: Vec<MirPattern>,
    },
    Literal(HirLiteral),
}

/// One explicit CFG block.
#[derive(Clone, Debug)]
pub struct BasicBlock {
    /// Stable identity matching the vector index.
    pub id: BasicBlockId,
    /// Sequential statements.
    pub statements: Vec<MirStatement>,
    /// Required terminating control-flow operation.
    pub terminator: Terminator,
}

/// MIR for one function.
#[derive(Clone, Debug)]
pub struct MirFunction {
    /// Original HIR/resolver function symbol.
    pub symbol: SymbolId,
    /// Debug name.
    pub name: String,
    /// Optional stable native export metadata.
    pub export: Option<HirExport>,
    /// Explicit pairwise-disjoint borrowed-parameter contract from HIR.
    pub disjoint: bool,
    /// Canonical signature type.
    pub signature: TypeId,
    /// All parameters, user locals, and future temporaries.
    pub locals: Vec<MirLocal>,
    /// Entry block is always block zero.
    pub blocks: Vec<BasicBlock>,
    /// Full source range.
    pub span: Span,
}

/// MIR for one source module.
#[derive(Clone, Debug)]
pub struct MirModule {
    /// Functions in deterministic source order.
    pub functions: Vec<MirFunction>,
    /// Nominal layouts required by target-neutral value lowering.
    pub nominal_layouts: Vec<NominalLayout>,
    /// Source-local typed-expression index retained for MIR/debug queries.
    pub typed_expressions: Vec<TypedExpression>,
}

impl MirModule {
    /// Starts a deterministic query over the typed-expression metadata retained
    /// for MIR/debug consumers.
    #[must_use]
    pub fn query_typed_expressions(&self) -> TypedExpressionQuery<'_> {
        TypedExpressionQuery::new(&self.typed_expressions)
    }
}

/// Lowers verified HIR into initial place-based MIR.
#[must_use]
pub fn lower_mir(hir: &HirModule, types: &TypeStore) -> MirModule {
    MirModule {
        functions: hir
            .functions
            .iter()
            .map(|function| Builder::new(function, types).lower())
            .collect(),
        nominal_layouts: hir.nominal_layouts.clone(),
        typed_expressions: hir.typed_expressions.clone(),
    }
}

struct Builder<'a> {
    function: &'a HirFunction,
    types: &'a TypeStore,
    locals: Vec<MirLocal>,
    symbols: BTreeMap<SymbolId, LocalId>,
    blocks: Vec<PendingBlock>,
    current_block: BasicBlockId,
    current_region: Option<LocalId>,
    pattern_sources: BTreeMap<SymbolId, PatternSource>,
    loop_targets: Vec<LoopTargets>,
}

struct LoopTargets {
    break_block: BasicBlockId,
    continue_block: BasicBlockId,
}

struct PendingBlock {
    id: BasicBlockId,
    statements: Vec<MirStatement>,
    terminator: Option<Terminator>,
}

struct PatternBinding {
    symbol: SymbolId,
    local: LocalId,
    ty: TypeId,
    path: Vec<String>,
    span: Span,
}

#[derive(Clone)]
struct PatternSource {
    source: Place,
    path: Vec<String>,
}

impl<'a> Builder<'a> {
    fn new(function: &'a HirFunction, types: &'a TypeStore) -> Self {
        Self {
            function,
            types,
            locals: Vec::new(),
            symbols: BTreeMap::new(),
            blocks: vec![PendingBlock {
                id: BasicBlockId(0),
                statements: Vec::new(),
                terminator: None,
            }],
            current_block: BasicBlockId(0),
            current_region: None,
            pattern_sources: BTreeMap::new(),
            loop_targets: Vec::new(),
        }
    }

    fn lower(mut self) -> MirFunction {
        for parameter in &self.function.parameters {
            self.add_local(
                Some(parameter.symbol),
                &parameter.name,
                parameter.ty,
                parameter.mutable,
                true,
                parameter.span,
            );
        }
        self.lower_block(&self.function.body);
        if !self.current_terminated() {
            self.terminate(Terminator::Return {
                value: None,
                accesses: Vec::new(),
                span: self.function.body.span,
            });
        }
        let blocks = self
            .blocks
            .into_iter()
            .map(|block| BasicBlock {
                id: block.id,
                statements: block.statements,
                terminator: block.terminator.unwrap_or(Terminator::Unreachable {
                    span: self.function.body.span,
                }),
            })
            .collect();
        MirFunction {
            symbol: self.function.symbol,
            name: self.function.name.clone(),
            export: self.function.export.clone(),
            disjoint: self.function.disjoint,
            signature: self.function.signature,
            locals: self.locals,
            blocks,
            span: self.function.span,
        }
    }

    fn lower_block(&mut self, block: &HirBlock) {
        for statement in &block.statements {
            if self.current_terminated() {
                break;
            }
            self.lower_statement(statement);
        }
    }

    fn push_statement(&mut self, statement: MirStatement) {
        self.blocks[self.current_block.index()]
            .statements
            .push(statement);
    }

    fn terminate(&mut self, terminator: Terminator) {
        self.blocks[self.current_block.index()].terminator = Some(terminator);
    }

    fn current_terminated(&self) -> bool {
        self.blocks[self.current_block.index()].terminator.is_some()
    }

    fn new_block(&mut self) -> BasicBlockId {
        let id = BasicBlockId(self.blocks.len());
        self.blocks.push(PendingBlock {
            id,
            statements: Vec::new(),
            terminator: None,
        });
        id
    }

    fn switch_to(&mut self, block: BasicBlockId) {
        self.current_block = block;
    }

    fn lower_statement(&mut self, statement: &HirStatement) {
        match statement {
            HirStatement::Binding { local, value, span } => {
                let id = self.add_local(
                    Some(local.symbol),
                    &local.name,
                    local.ty,
                    local.mutable,
                    false,
                    local.span,
                );
                if let Some(value) = value {
                    self.locals[id.index()].owned_region = self.expression_region_owner(value);
                }
                self.push_statement(MirStatement::StorageLive {
                    local: id,
                    span: *span,
                });
                if let Some(value) = value {
                    let capability = self.types.kind(local.ty).and_then(|kind| match kind {
                        TypeKind::Capability { capability, .. } => Some(*capability),
                        _ => None,
                    });
                    if let (Some(capability), Some(source)) =
                        (capability, self.expression_place(value))
                        && matches!(capability, Capability::Read | Capability::Write)
                    {
                        self.push_statement(MirStatement::Borrow {
                            destination: id,
                            source,
                            kind: if capability == Capability::Read {
                                BorrowKind::Read
                            } else {
                                BorrowKind::Write
                            },
                            span: *span,
                        });
                    } else {
                        let lowered = self.lower_value(value);
                        let accesses = self.operand_accesses(&lowered);
                        self.push_statement(MirStatement::Assign {
                            destination: Place::local(id),
                            destination_indices: Vec::new(),
                            value: Some(lowered),
                            accesses,
                            span: *span,
                        });
                    }
                }
            }
            HirStatement::Return { value, span } => {
                let value = value.as_ref().map(|value| self.lower_value(value));
                let accesses = value
                    .as_ref()
                    .map_or_else(Vec::new, |value| self.operand_accesses(value));
                self.terminate(Terminator::Return {
                    value,
                    accesses,
                    span: *span,
                });
            }
            HirStatement::Region { local, body, span } => {
                let parent = self.current_region;
                let region = self.add_local(
                    Some(local.symbol),
                    &local.name,
                    local.ty,
                    false,
                    false,
                    local.span,
                );
                self.push_statement(MirStatement::StorageLive {
                    local: region,
                    span: *span,
                });
                self.push_statement(MirStatement::RegionEnter {
                    region,
                    span: *span,
                });
                self.current_region = Some(region);
                self.lower_block(body);
                self.current_region = parent;
                self.push_statement(MirStatement::RegionExit {
                    region,
                    span: *span,
                });
                self.push_statement(MirStatement::StorageDead {
                    local: region,
                    span: *span,
                });
            }
            HirStatement::While {
                condition,
                body,
                span,
            } => {
                let head = self.new_block();
                let body_target = self.new_block();
                let exit = self.new_block();
                self.terminate(Terminator::Goto {
                    target: head,
                    span: *span,
                });

                self.switch_to(head);
                let condition = self.lower_value(condition);
                let discriminant = self.operand_accesses(&condition);
                self.terminate(Terminator::Switch {
                    value: Some(condition),
                    discriminant,
                    targets: vec![body_target],
                    otherwise: exit,
                    span: *span,
                });

                self.switch_to(body_target);
                self.loop_targets.push(LoopTargets {
                    break_block: exit,
                    continue_block: head,
                });
                self.lower_block(body);
                self.loop_targets.pop();
                if !self.current_terminated() {
                    self.terminate(Terminator::Goto {
                        target: head,
                        span: *span,
                    });
                }
                self.switch_to(exit);
            }
            HirStatement::For {
                local,
                iterable,
                body,
                span,
            } => self.lower_for(local, iterable, body, *span),
            HirStatement::Break { span } => {
                if let Some(targets) = self.loop_targets.last() {
                    self.terminate(Terminator::Goto {
                        target: targets.break_block,
                        span: *span,
                    });
                } else {
                    self.terminate(Terminator::Unreachable { span: *span });
                }
            }
            HirStatement::Continue { span } => {
                if let Some(targets) = self.loop_targets.last() {
                    self.terminate(Terminator::Goto {
                        target: targets.continue_block,
                        span: *span,
                    });
                } else {
                    self.terminate(Terminator::Unreachable { span: *span });
                }
            }
            HirStatement::Expression { expression, .. } => {
                if let HirExpressionKind::Binary {
                    left,
                    operator,
                    right,
                } = &expression.kind
                    && let Some(destination) = self.expression_place(left)
                    && self.lower_assignment_expression(
                        left,
                        *operator,
                        right,
                        destination,
                        expression.span,
                    )
                {
                } else {
                    let lowered = self.lower_value(expression);
                    let accesses = self.operand_accesses(&lowered);
                    self.push_statement(MirStatement::Evaluate {
                        value: Some(lowered),
                        accesses,
                        span: expression.span,
                    });
                }
            }
        }
    }

    fn lower_assignment_expression(
        &mut self,
        left: &HirExpression,
        operator: Operator,
        right: &HirExpression,
        destination: Place,
        span: Span,
    ) -> bool {
        match operator {
            Operator::Assign => {
                let destination_indices = self.lower_place_indices(left);
                let lowered = self.lower_value(right);
                let mut accesses = Vec::new();
                for index in &destination_indices {
                    self.collect_operand_accesses(index, &mut accesses);
                }
                self.collect_operand_accesses(&lowered, &mut accesses);
                self.push_statement(MirStatement::Assign {
                    destination,
                    destination_indices,
                    value: Some(lowered),
                    accesses,
                    span,
                });
                true
            }
            operator => {
                let Some(binary_operator) = compound_assignment_operator(operator) else {
                    return false;
                };
                let lowered_left = self.lower_value(left);
                let mut destination_indices = Vec::new();
                let mut temporary_locals = Vec::new();
                let lowered_left = self.capture_compound_indices(
                    lowered_left,
                    &mut destination_indices,
                    &mut temporary_locals,
                );
                let lowered_right = self.lower_value(right);
                let lowered = MirOperand {
                    ty: left.ty,
                    kind: MirOperandKind::Binary {
                        left: Box::new(lowered_left),
                        operator: binary_operator,
                        right: Box::new(lowered_right),
                    },
                    span,
                };
                let accesses = self.operand_accesses(&lowered);
                self.push_statement(MirStatement::Assign {
                    destination,
                    destination_indices,
                    value: Some(lowered),
                    accesses,
                    span,
                });
                for local in temporary_locals.into_iter().rev() {
                    self.push_statement(MirStatement::StorageDead { local, span });
                }
                true
            }
        }
    }

    fn capture_compound_indices(
        &mut self,
        operand: MirOperand,
        destination_indices: &mut Vec<MirOperand>,
        temporary_locals: &mut Vec<LocalId>,
    ) -> MirOperand {
        let MirOperand {
            ty,
            kind,
            span: operand_span,
        } = operand;
        match kind {
            MirOperandKind::Index { base, index } => {
                let base =
                    self.capture_compound_indices(*base, destination_indices, temporary_locals);
                let index_span = index.span;
                let index_ty = index.ty;
                let name = format!("$compound_index{}", self.locals.len());
                let local = self.add_local(None, &name, index_ty, true, false, index_span);
                self.push_statement(MirStatement::StorageLive {
                    local,
                    span: index_span,
                });
                let accesses = self.operand_accesses(&index);
                self.push_statement(MirStatement::Assign {
                    destination: Place::local(local),
                    destination_indices: Vec::new(),
                    value: Some(*index),
                    accesses,
                    span: index_span,
                });
                let index_place = MirOperand {
                    ty: index_ty,
                    kind: MirOperandKind::Place(Place::local(local)),
                    span: index_span,
                };
                destination_indices.push(index_place.clone());
                temporary_locals.push(local);
                MirOperand {
                    ty,
                    kind: MirOperandKind::Index {
                        base: Box::new(base),
                        index: Box::new(index_place),
                    },
                    span: operand_span,
                }
            }
            MirOperandKind::Field { base, field } => MirOperand {
                ty,
                kind: MirOperandKind::Field {
                    base: Box::new(self.capture_compound_indices(
                        *base,
                        destination_indices,
                        temporary_locals,
                    )),
                    field,
                },
                span: operand_span,
            },
            kind => MirOperand {
                ty,
                kind,
                span: operand_span,
            },
        }
    }

    fn lower_for(
        &mut self,
        local: &jadren_hir::HirLocal,
        iterable: &HirExpression,
        body: &HirBlock,
        span: Span,
    ) {
        let (iterable, iterates_indices) = match &iterable.kind {
            HirExpressionKind::Field { base, field } if field == "indices" => (base.as_ref(), true),
            _ => (iterable, false),
        };
        let mut iterable_type = iterable.ty;
        if let Some(TypeKind::Capability { inner, .. }) = self.types.kind(iterable_type) {
            iterable_type = *inner;
        }
        let (fixed_length, dynamic_length) = match self.types.kind(iterable_type) {
            Some(TypeKind::Array { length, .. }) => (Some(*length), false),
            Some(TypeKind::Buffer(_) | TypeKind::Slice(_)) => (None, true),
            _ => {
                self.terminate(Terminator::Unreachable { span });
                return;
            }
        };
        let iterable_local = self.add_local(
            None,
            "$for_iterable",
            iterable.ty,
            false,
            false,
            iterable.span,
        );
        let index_local = self.add_local(
            iterates_indices.then_some(local.symbol),
            "$for_index",
            self.types.core().uint_size,
            true,
            false,
            span,
        );
        let binding_local = (!iterates_indices).then(|| {
            self.add_local(
                Some(local.symbol),
                &local.name,
                local.ty,
                false,
                false,
                local.span,
            )
        });
        self.push_statement(MirStatement::StorageLive {
            local: iterable_local,
            span,
        });
        let iterable_value = self.lower_value(iterable);
        self.push_statement(MirStatement::Assign {
            destination: Place::local(iterable_local),
            destination_indices: Vec::new(),
            accesses: self.operand_accesses(&iterable_value),
            value: Some(iterable_value),
            span,
        });
        self.push_statement(MirStatement::StorageLive {
            local: index_local,
            span,
        });
        let zero = self.synthetic_integer_typed("0", self.types.core().uint_size, span);
        self.push_statement(MirStatement::Assign {
            destination: Place::local(index_local),
            destination_indices: Vec::new(),
            accesses: Vec::new(),
            value: Some(zero),
            span,
        });
        if let Some(binding_local) = binding_local {
            self.push_statement(MirStatement::StorageLive {
                local: binding_local,
                span,
            });
        }

        let head = self.new_block();
        let body_target = self.new_block();
        let increment = self.new_block();
        let exit = self.new_block();
        self.terminate(Terminator::Goto { target: head, span });

        self.switch_to(head);
        let length = if dynamic_length {
            MirOperand {
                ty: self.types.core().uint_size,
                kind: MirOperandKind::Length {
                    base: Box::new(self.place_operand(iterable_local, iterable.ty, span)),
                },
                span,
            }
        } else {
            self.synthetic_integer_typed(
                &fixed_length.expect("fixed array length").to_string(),
                self.types.core().uint_size,
                span,
            )
        };
        let condition = MirOperand {
            ty: self.types.core().bool_,
            kind: MirOperandKind::Binary {
                left: Box::new(self.place_operand(index_local, self.types.core().uint_size, span)),
                operator: Operator::Less,
                right: Box::new(length),
            },
            span,
        };
        let discriminant = self.operand_accesses(&condition);
        self.terminate(Terminator::Switch {
            value: Some(condition),
            discriminant,
            targets: vec![body_target],
            otherwise: exit,
            span,
        });

        self.switch_to(body_target);
        if let Some(binding_local) = binding_local {
            let iterable_place = self.place_operand(iterable_local, iterable.ty, span);
            let index = self.place_operand(index_local, self.types.core().uint_size, span);
            let element = MirOperand {
                ty: local.ty,
                kind: MirOperandKind::Index {
                    base: Box::new(iterable_place),
                    index: Box::new(index),
                },
                span,
            };
            self.push_statement(MirStatement::Assign {
                destination: Place::local(binding_local),
                destination_indices: Vec::new(),
                accesses: self.operand_accesses(&element),
                value: Some(element),
                span,
            });
        }
        self.loop_targets.push(LoopTargets {
            break_block: exit,
            continue_block: increment,
        });
        self.lower_block(body);
        self.loop_targets.pop();
        if !self.current_terminated() {
            self.terminate(Terminator::Goto {
                target: increment,
                span,
            });
        }

        self.switch_to(increment);
        let index_value = self.place_operand(index_local, self.types.core().uint_size, span);
        let next_index = MirOperand {
            ty: self.types.core().uint_size,
            kind: MirOperandKind::Binary {
                left: Box::new(index_value),
                operator: Operator::Plus,
                right: Box::new(self.synthetic_integer_typed(
                    "1",
                    self.types.core().uint_size,
                    span,
                )),
            },
            span,
        };
        self.push_statement(MirStatement::Assign {
            destination: Place::local(index_local),
            destination_indices: Vec::new(),
            accesses: self.operand_accesses(&next_index),
            value: Some(next_index),
            span,
        });
        self.terminate(Terminator::Goto { target: head, span });

        self.switch_to(exit);
        if let Some(binding_local) = binding_local {
            self.push_statement(MirStatement::StorageDead {
                local: binding_local,
                span,
            });
        }
        self.push_statement(MirStatement::StorageDead {
            local: index_local,
            span,
        });
        self.push_statement(MirStatement::StorageDead {
            local: iterable_local,
            span,
        });
    }

    fn synthetic_integer_typed(&self, text: &str, ty: TypeId, span: Span) -> MirOperand {
        MirOperand {
            ty,
            kind: MirOperandKind::Literal(HirLiteral {
                kind: LiteralKind::Integer,
                text: text.to_owned(),
            }),
            span,
        }
    }

    fn place_operand(&self, local: LocalId, ty: TypeId, span: Span) -> MirOperand {
        MirOperand {
            ty,
            kind: MirOperandKind::Place(Place::local(local)),
            span,
        }
    }

    fn lower_value(&mut self, expression: &HirExpression) -> MirOperand {
        match &expression.kind {
            HirExpressionKind::If {
                condition,
                then_block,
                else_branch,
            } => self.lower_if_value(
                expression.ty,
                expression.span,
                condition,
                then_block,
                else_branch.as_deref(),
            ),
            HirExpressionKind::Match { value, arms } => {
                self.lower_match_value(expression.ty, expression.span, value, arms)
            }
            HirExpressionKind::Try {
                operand,
                propagation,
            } => self.lower_try_value(expression.ty, expression.span, operand, *propagation),
            HirExpressionKind::Block(block) => {
                self.lower_block_value(expression.ty, expression.span, block)
            }
            _ => self.lower_operand(expression),
        }
    }

    fn lower_block_value(&mut self, ty: TypeId, span: Span, block: &HirBlock) -> MirOperand {
        let unit = matches!(self.types.kind(ty), Some(TypeKind::Unit));
        let temporary = if unit {
            None
        } else {
            let name = format!("$tmp{}", self.locals.len());
            let local = self.add_local(None, &name, ty, true, false, span);
            self.push_statement(MirStatement::StorageLive { local, span });
            Some(local)
        };
        self.lower_block_result(block, temporary, span);
        temporary.map_or(
            MirOperand {
                ty,
                kind: MirOperandKind::Unit,
                span,
            },
            |local| MirOperand {
                ty,
                kind: MirOperandKind::Place(Place::local(local)),
                span,
            },
        )
    }

    fn lower_if_value(
        &mut self,
        ty: TypeId,
        span: Span,
        condition: &HirExpression,
        then_block: &HirBlock,
        else_branch: Option<&HirExpression>,
    ) -> MirOperand {
        let unit = matches!(self.types.kind(ty), Some(TypeKind::Unit));
        let temporary = if unit {
            None
        } else {
            let name = format!("$tmp{}", self.locals.len());
            let local = self.add_local(None, &name, ty, true, false, span);
            self.push_statement(MirStatement::StorageLive { local, span });
            Some(local)
        };
        let condition = self.lower_value(condition);
        let discriminant = self.operand_accesses(&condition);
        let then_target = self.new_block();
        let else_target = self.new_block();
        let join = self.new_block();
        self.terminate(Terminator::Switch {
            value: Some(condition),
            discriminant,
            targets: vec![then_target],
            otherwise: else_target,
            span,
        });

        self.switch_to(then_target);
        self.lower_block_result(then_block, temporary, span);
        if !self.current_terminated() {
            self.terminate(Terminator::Goto { target: join, span });
        }

        self.switch_to(else_target);
        if let Some(branch) = else_branch {
            self.lower_branch_result(branch, temporary, span);
        }
        if !self.current_terminated() {
            self.terminate(Terminator::Goto { target: join, span });
        }

        self.switch_to(join);
        temporary.map_or(
            MirOperand {
                ty,
                kind: MirOperandKind::Unit,
                span,
            },
            |local| MirOperand {
                ty,
                kind: MirOperandKind::Place(Place::local(local)),
                span,
            },
        )
    }

    fn lower_block_result(&mut self, block: &HirBlock, destination: Option<LocalId>, span: Span) {
        for (index, statement) in block.statements.iter().enumerate() {
            if self.current_terminated() {
                break;
            }
            let is_tail = index + 1 == block.statements.len();
            if is_tail
                && let HirStatement::Expression {
                    expression,
                    terminated: false,
                } = statement
                && !matches!(
                    expression.kind,
                    HirExpressionKind::Binary {
                        operator: Operator::Assign
                            | Operator::PlusAssign
                            | Operator::MinusAssign
                            | Operator::StarAssign
                            | Operator::SlashAssign
                            | Operator::PercentAssign,
                        ..
                    }
                )
            {
                self.lower_result_expression(expression, destination, span);
            } else {
                self.lower_statement(statement);
            }
        }
    }

    fn lower_branch_result(
        &mut self,
        expression: &HirExpression,
        destination: Option<LocalId>,
        span: Span,
    ) {
        if let HirExpressionKind::Block(block) = &expression.kind {
            self.lower_block_result(block, destination, span);
        } else {
            self.lower_result_expression(expression, destination, span);
        }
    }

    fn lower_result_expression(
        &mut self,
        expression: &HirExpression,
        destination: Option<LocalId>,
        span: Span,
    ) {
        let value = self.lower_value(expression);
        let accesses = self.operand_accesses(&value);
        if let Some(destination) = destination {
            self.push_statement(MirStatement::Assign {
                destination: Place::local(destination),
                destination_indices: Vec::new(),
                value: Some(value),
                accesses,
                span,
            });
        } else {
            self.push_statement(MirStatement::Evaluate {
                value: Some(value),
                accesses,
                span,
            });
        }
    }

    fn lower_match_value(
        &mut self,
        ty: TypeId,
        span: Span,
        matched_value: &HirExpression,
        arms: &[HirMatchArm],
    ) -> MirOperand {
        let discriminant_value = self.lower_value(matched_value);
        let discriminant_accesses = self.operand_accesses(&discriminant_value);
        let discriminant_name = format!("$match{}", self.locals.len());
        let discriminant = self.add_local(
            None,
            &discriminant_name,
            matched_value.ty,
            true,
            false,
            matched_value.span,
        );
        self.push_statement(MirStatement::StorageLive {
            local: discriminant,
            span: matched_value.span,
        });
        self.push_statement(MirStatement::Assign {
            destination: Place::local(discriminant),
            destination_indices: Vec::new(),
            value: Some(discriminant_value),
            accesses: discriminant_accesses,
            span: matched_value.span,
        });

        let unit = matches!(self.types.kind(ty), Some(TypeKind::Unit));
        let result = if unit {
            None
        } else {
            let name = format!("$tmp{}", self.locals.len());
            let local = self.add_local(None, &name, ty, true, false, span);
            self.push_statement(MirStatement::StorageLive { local, span });
            Some(local)
        };
        let join = self.new_block();

        for arm in arms {
            let mut bindings = Vec::new();
            let pattern = self.lower_match_pattern(&arm.pattern, &mut Vec::new(), &mut bindings);
            let matched = self.new_block();
            let otherwise = self.new_block();
            let value = MirOperand {
                ty: matched_value.ty,
                kind: MirOperandKind::Place(Place::local(discriminant)),
                span: arm.pattern.span,
            };
            self.terminate(Terminator::Match {
                value,
                accesses: vec![PlaceAccess {
                    place: Place::local(discriminant),
                    kind: AccessKind::Read,
                    span: arm.pattern.span,
                }],
                pattern,
                matched,
                otherwise,
                span: arm.pattern.span,
            });

            self.switch_to(matched);
            if let Some(guard) = &arm.guard {
                self.begin_pattern_guard(discriminant, &bindings);
                let guard_value = self.lower_value(guard);
                self.end_pattern_guard(&bindings);
                let guard_accesses = self.operand_accesses(&guard_value);
                let body = self.new_block();
                let guard_failed = self.new_block();
                self.terminate(Terminator::Switch {
                    value: Some(guard_value),
                    discriminant: guard_accesses,
                    targets: vec![body],
                    otherwise: guard_failed,
                    span: guard.span,
                });
                self.switch_to(guard_failed);
                self.terminate(Terminator::Goto {
                    target: otherwise,
                    span: guard.span,
                });
                self.switch_to(body);
            }
            self.initialize_pattern_bindings(discriminant, &bindings);
            self.lower_result_expression(&arm.value, result, arm.span);
            self.end_pattern_bindings(&bindings);
            if !self.current_terminated() {
                self.terminate(Terminator::Goto {
                    target: join,
                    span: arm.span,
                });
            }
            self.switch_to(otherwise);
        }
        if !self.current_terminated() {
            self.terminate(Terminator::Unreachable { span });
        }
        self.switch_to(join);
        result.map_or(
            MirOperand {
                ty,
                kind: MirOperandKind::Unit,
                span,
            },
            |local| MirOperand {
                ty,
                kind: MirOperandKind::Place(Place::local(local)),
                span,
            },
        )
    }

    fn lower_match_pattern(
        &mut self,
        pattern: &HirPattern,
        path: &mut Vec<String>,
        bindings: &mut Vec<PatternBinding>,
    ) -> MirPattern {
        match &pattern.kind {
            HirPatternKind::Wildcard => MirPattern::Wildcard,
            HirPatternKind::Literal(literal) => MirPattern::Literal(literal.clone()),
            HirPatternKind::Path {
                path: name,
                binding,
                binding_type,
                constructor,
            } => {
                if let (Some(symbol), Some(ty)) = (binding, binding_type) {
                    let local =
                        self.add_local(Some(*symbol), name, *ty, false, false, pattern.span);
                    bindings.push(PatternBinding {
                        symbol: *symbol,
                        local,
                        ty: *ty,
                        path: path.clone(),
                        span: pattern.span,
                    });
                    MirPattern::Binding
                } else {
                    MirPattern::Path {
                        path: name.clone(),
                        constructor: *constructor,
                    }
                }
            }
            HirPatternKind::Constructor {
                path: name,
                constructor,
                arguments,
            } => {
                path.push(format!("$variant:{name}"));
                let lowered = arguments
                    .iter()
                    .enumerate()
                    .map(|(index, argument)| {
                        path.push(format!("$payload{index}"));
                        let pattern = self.lower_match_pattern(argument, path, bindings);
                        path.pop();
                        pattern
                    })
                    .collect();
                path.pop();
                MirPattern::Constructor {
                    path: name.clone(),
                    constructor: *constructor,
                    arguments: lowered,
                }
            }
            HirPatternKind::Error => MirPattern::Wildcard,
        }
    }

    fn initialize_pattern_bindings(&mut self, source: LocalId, bindings: &[PatternBinding]) {
        for binding in bindings {
            self.push_statement(MirStatement::StorageLive {
                local: binding.local,
                span: binding.span,
            });
            let value = MirOperand {
                ty: binding.ty,
                kind: MirOperandKind::PatternExtract {
                    source: Place::local(source),
                    source_indices: Vec::new(),
                    path: binding.path.clone(),
                    borrowed: false,
                },
                span: binding.span,
            };
            let accesses = self.operand_accesses(&value);
            self.push_statement(MirStatement::Assign {
                destination: Place::local(binding.local),
                destination_indices: Vec::new(),
                value: Some(value),
                accesses,
                span: binding.span,
            });
        }
    }

    fn end_pattern_bindings(&mut self, bindings: &[PatternBinding]) {
        for binding in bindings.iter().rev() {
            self.push_statement(MirStatement::StorageDead {
                local: binding.local,
                span: binding.span,
            });
        }
    }

    fn begin_pattern_guard(&mut self, source: LocalId, bindings: &[PatternBinding]) {
        for binding in bindings {
            self.pattern_sources.insert(
                binding.symbol,
                PatternSource {
                    source: Place::local(source),
                    path: binding.path.clone(),
                },
            );
        }
    }

    fn end_pattern_guard(&mut self, bindings: &[PatternBinding]) {
        for binding in bindings {
            self.pattern_sources.remove(&binding.symbol);
        }
    }

    fn lower_try_value(
        &mut self,
        ty: TypeId,
        span: Span,
        operand: &HirExpression,
        propagation: jadren_hir::PropagationSite,
    ) -> MirOperand {
        let carrier_value = self.lower_value(operand);
        let carrier_accesses = self.operand_accesses(&carrier_value);
        let carrier_name = format!("$carrier{}", self.locals.len());
        let carrier = self.add_local(None, &carrier_name, operand.ty, true, false, operand.span);
        self.push_statement(MirStatement::StorageLive {
            local: carrier,
            span,
        });
        self.push_statement(MirStatement::Assign {
            destination: Place::local(carrier),
            destination_indices: Vec::new(),
            value: Some(carrier_value),
            accesses: carrier_accesses,
            span,
        });
        let result_name = format!("$tmp{}", self.locals.len());
        let result = self.add_local(None, &result_name, ty, true, false, span);
        self.push_statement(MirStatement::StorageLive {
            local: result,
            span,
        });
        let success = self.new_block();
        let residual = self.new_block();
        let join = self.new_block();
        let kind = match propagation.kind {
            PropagationKind::OptionNone => MirPropagationKind::OptionNone,
            PropagationKind::ResultError => MirPropagationKind::ResultError,
        };
        self.terminate(Terminator::Propagate {
            value: MirOperand {
                ty: operand.ty,
                kind: MirOperandKind::Place(Place::local(carrier)),
                span,
            },
            accesses: vec![PlaceAccess {
                place: Place::local(carrier),
                kind: AccessKind::Read,
                span,
            }],
            kind,
            success,
            residual,
            span,
        });

        self.switch_to(success);
        let success_value = MirOperand {
            ty,
            kind: MirOperandKind::CarrierExtract {
                source: Place::local(carrier),
                source_indices: Vec::new(),
                part: CarrierPart::Success,
            },
            span,
        };
        let success_accesses = self.operand_accesses(&success_value);
        self.push_statement(MirStatement::Assign {
            destination: Place::local(result),
            destination_indices: Vec::new(),
            value: Some(success_value),
            accesses: success_accesses,
            span,
        });
        self.terminate(Terminator::Goto { target: join, span });

        self.switch_to(residual);
        let residual_value = MirOperand {
            ty: propagation.return_type,
            kind: MirOperandKind::PropagateResidual {
                source: Place::local(carrier),
                kind,
                residual_type: propagation.residual_type,
            },
            span,
        };
        let residual_accesses = self.operand_accesses(&residual_value);
        self.terminate(Terminator::Return {
            value: Some(residual_value),
            accesses: residual_accesses,
            span,
        });

        self.switch_to(join);
        MirOperand {
            ty,
            kind: MirOperandKind::Place(Place::local(result)),
            span,
        }
    }

    fn add_local(
        &mut self,
        symbol: Option<SymbolId>,
        name: &str,
        ty: TypeId,
        mutable: bool,
        is_parameter: bool,
        span: Span,
    ) -> LocalId {
        let id = LocalId(self.locals.len());
        self.locals.push(MirLocal {
            id,
            symbol,
            name: name.to_owned(),
            ty,
            mutable,
            is_parameter,
            is_return: false,
            scope_region: if is_parameter {
                None
            } else {
                self.current_region
            },
            owned_region: None,
            span,
        });
        if let Some(symbol) = symbol {
            self.symbols.insert(symbol, id);
        }
        id
    }

    fn operand_accesses(&self, operand: &MirOperand) -> Vec<PlaceAccess> {
        let mut accesses = Vec::new();
        self.collect_operand_accesses(operand, &mut accesses);
        accesses
    }

    fn collect_operand_accesses(&self, operand: &MirOperand, output: &mut Vec<PlaceAccess>) {
        match &operand.kind {
            MirOperandKind::Place(place) => output.push(PlaceAccess {
                place: place.clone(),
                kind: if is_move_only(self.types, operand.ty) {
                    AccessKind::Move
                } else {
                    AccessKind::Read
                },
                span: operand.span,
            }),
            MirOperandKind::Unit | MirOperandKind::Literal(_) | MirOperandKind::Function { .. } => {
            }
            MirOperandKind::Unary { operand, .. } => {
                self.collect_operand_accesses(operand, output);
            }
            MirOperandKind::Cast { operand } => self.collect_operand_accesses(operand, output),
            MirOperandKind::Binary { left, right, .. } => {
                self.collect_operand_accesses(left, output);
                self.collect_operand_accesses(right, output);
            }
            MirOperandKind::Call { callee, arguments } => {
                self.collect_operand_accesses(callee, output);
                let parameters = match self.types.kind(callee.ty) {
                    Some(TypeKind::Function { parameters, .. }) => parameters.as_ref(),
                    _ => &[],
                };
                for (index, argument) in arguments.iter().enumerate() {
                    let capability = parameters.get(index).and_then(|parameter| {
                        match self.types.kind(*parameter) {
                            Some(TypeKind::Capability { capability, .. }) => Some(*capability),
                            _ => None,
                        }
                    });
                    if let (Some(capability), Some(place)) =
                        (capability, Self::operand_place(argument))
                        && matches!(capability, Capability::Read | Capability::Write)
                    {
                        output.push(PlaceAccess {
                            place,
                            kind: if capability == Capability::Read {
                                AccessKind::BorrowRead
                            } else {
                                AccessKind::BorrowWrite
                            },
                            span: argument.span,
                        });
                        Self::collect_index_accesses(argument, self, output);
                    } else {
                        self.collect_operand_accesses(argument, output);
                    }
                }
            }
            MirOperandKind::RegionAllocate { region, arguments } => {
                if let Some(region) = region {
                    output.push(PlaceAccess {
                        place: Place::local(*region),
                        kind: AccessKind::Read,
                        span: operand.span,
                    });
                }
                for argument in arguments {
                    self.collect_operand_accesses(argument, output);
                }
            }
            MirOperandKind::Index { base, index } => {
                if let Some(place) = Self::operand_place(operand) {
                    output.push(PlaceAccess {
                        place,
                        kind: if is_move_only(self.types, operand.ty) {
                            AccessKind::Move
                        } else {
                            AccessKind::Read
                        },
                        span: operand.span,
                    });
                    self.collect_operand_accesses(index, output);
                } else {
                    self.collect_operand_accesses(base, output);
                    self.collect_operand_accesses(index, output);
                }
            }
            MirOperandKind::Length { base } => self.collect_operand_accesses(base, output),
            MirOperandKind::Field { base, field } => {
                if let Some(place) = Self::operand_place(operand) {
                    output.push(PlaceAccess {
                        place,
                        kind: if is_move_only(self.types, operand.ty) {
                            AccessKind::Move
                        } else {
                            AccessKind::Read
                        },
                        span: operand.span,
                    });
                } else {
                    let _ = field;
                    self.collect_operand_accesses(base, output);
                }
            }
            MirOperandKind::Array(elements) => {
                for element in elements {
                    self.collect_operand_accesses(element, output);
                }
            }
            MirOperandKind::Struct { fields, .. } => {
                for (_, value) in fields {
                    self.collect_operand_accesses(value, output);
                }
            }
            MirOperandKind::PatternExtract {
                source,
                source_indices,
                path,
                borrowed,
            } => {
                for index in source_indices {
                    self.collect_operand_accesses(index, output);
                }
                let mut place = source.clone();
                place.projection.extend(
                    path.iter()
                        .map(|segment| Projection::Field(segment.clone())),
                );
                output.push(PlaceAccess {
                    place,
                    kind: if *borrowed {
                        AccessKind::Read
                    } else if is_move_only(self.types, operand.ty) {
                        AccessKind::Move
                    } else {
                        AccessKind::Read
                    },
                    span: operand.span,
                });
            }
            MirOperandKind::CarrierExtract {
                source,
                source_indices,
                part,
            } => {
                for index in source_indices {
                    self.collect_operand_accesses(index, output);
                }
                let mut place = source.clone();
                place.projection.push(Projection::Field(match part {
                    CarrierPart::Success => "$success".to_owned(),
                    CarrierPart::Residual => "$residual".to_owned(),
                }));
                output.push(PlaceAccess {
                    place,
                    kind: if is_move_only(self.types, operand.ty) {
                        AccessKind::Move
                    } else {
                        AccessKind::Read
                    },
                    span: operand.span,
                });
            }
            MirOperandKind::PropagateResidual {
                source,
                kind,
                residual_type,
            } => {
                let mut place = source.clone();
                if *kind == MirPropagationKind::ResultError {
                    place
                        .projection
                        .push(Projection::Field("$residual".to_owned()));
                }
                output.push(PlaceAccess {
                    place,
                    kind: if *kind == MirPropagationKind::ResultError
                        && is_move_only(self.types, *residual_type)
                    {
                        AccessKind::Move
                    } else {
                        AccessKind::Read
                    },
                    span: operand.span,
                });
            }
            MirOperandKind::HighLevel(_) => {}
        }
    }

    fn operand_place(operand: &MirOperand) -> Option<Place> {
        match &operand.kind {
            MirOperandKind::Place(place) => Some(place.clone()),
            MirOperandKind::Index { base, .. } => {
                let mut place = Self::operand_place(base)?;
                place.projection.push(Projection::Index);
                Some(place)
            }
            MirOperandKind::Field { base, field } => {
                let mut place = Self::operand_place(base)?;
                place.projection.push(Projection::Field(field.clone()));
                Some(place)
            }
            _ => None,
        }
    }

    fn collect_index_accesses(operand: &MirOperand, builder: &Self, output: &mut Vec<PlaceAccess>) {
        if let MirOperandKind::Index { base, index } = &operand.kind {
            Self::collect_index_accesses(base, builder, output);
            builder.collect_operand_accesses(index, output);
        }
    }

    fn lower_operand(&mut self, expression: &HirExpression) -> MirOperand {
        let kind = match &expression.kind {
            HirExpressionKind::Name { name, symbol } => {
                if let Some(source) = symbol.and_then(|symbol| self.pattern_sources.get(&symbol)) {
                    MirOperandKind::PatternExtract {
                        source: source.source.clone(),
                        source_indices: Vec::new(),
                        path: source.path.clone(),
                        borrowed: true,
                    }
                } else if let Some(place) = self.expression_place(expression) {
                    MirOperandKind::Place(place)
                } else {
                    MirOperandKind::Function {
                        name: name.clone(),
                        symbol: *symbol,
                    }
                }
            }
            HirExpressionKind::Literal(literal) => MirOperandKind::Literal(literal.clone()),
            HirExpressionKind::Unary { operator, operand } => MirOperandKind::Unary {
                operator: *operator,
                operand: Box::new(self.lower_value(operand)),
            },
            HirExpressionKind::Cast { expression } => MirOperandKind::Cast {
                operand: Box::new(self.lower_value(expression)),
            },
            HirExpressionKind::Binary {
                left,
                operator,
                right,
            } => MirOperandKind::Binary {
                left: Box::new(self.lower_value(left)),
                operator: *operator,
                right: Box::new(self.lower_value(right)),
            },
            HirExpressionKind::Call { callee, arguments } => MirOperandKind::Call {
                callee: Box::new(self.lower_value(callee)),
                arguments: arguments
                    .iter()
                    .map(|argument| self.lower_value(argument))
                    .collect(),
            },
            HirExpressionKind::RegionAllocate { region, arguments } => {
                let region = self.symbols.get(region).copied();
                MirOperandKind::RegionAllocate {
                    region,
                    arguments: arguments
                        .iter()
                        .map(|argument| self.lower_value(argument))
                        .collect(),
                }
            }
            HirExpressionKind::Index { base, index } => MirOperandKind::Index {
                base: Box::new(self.lower_value(base)),
                index: Box::new(self.lower_value(index)),
            },
            HirExpressionKind::Field { base, field } => {
                if !Self::expression_contains_index(base)
                    && let Some(place) = self.expression_place(expression)
                {
                    MirOperandKind::Place(place)
                } else {
                    MirOperandKind::Field {
                        base: Box::new(self.lower_value(base)),
                        field: field.clone(),
                    }
                }
            }
            HirExpressionKind::Array(elements) => MirOperandKind::Array(
                elements
                    .iter()
                    .map(|element| self.lower_value(element))
                    .collect(),
            ),
            HirExpressionKind::Struct { type_name, fields } => MirOperandKind::Struct {
                type_name: type_name.clone(),
                fields: fields
                    .iter()
                    .map(|(name, value)| (name.clone(), self.lower_value(value)))
                    .collect(),
            },
            HirExpressionKind::Group(inner) => return self.lower_value(inner),
            HirExpressionKind::Try { .. }
            | HirExpressionKind::Block(_)
            | HirExpressionKind::If { .. }
            | HirExpressionKind::Match { .. } => return self.lower_value(expression),
            HirExpressionKind::Error => MirOperandKind::HighLevel(Box::new(expression.clone())),
        };
        MirOperand {
            ty: expression.ty,
            kind,
            span: expression.span,
        }
    }

    fn expression_place(&self, expression: &HirExpression) -> Option<Place> {
        match &expression.kind {
            HirExpressionKind::Name {
                symbol: Some(symbol),
                ..
            } => self.symbols.get(symbol).copied().map(Place::local),
            HirExpressionKind::Field { base, field } => {
                let mut place = self.expression_place(base)?;
                place.projection.push(Projection::Field(field.clone()));
                Some(place)
            }
            HirExpressionKind::Index { base, .. } => {
                let mut place = self.expression_place(base)?;
                place.projection.push(Projection::Index);
                Some(place)
            }
            HirExpressionKind::Group(inner) => self.expression_place(inner),
            _ => None,
        }
    }

    fn expression_contains_index(expression: &HirExpression) -> bool {
        match &expression.kind {
            HirExpressionKind::Index { .. } => true,
            HirExpressionKind::Field { base, .. } | HirExpressionKind::Group(base) => {
                Self::expression_contains_index(base)
            }
            _ => false,
        }
    }

    fn lower_place_indices(&mut self, expression: &HirExpression) -> Vec<MirOperand> {
        let mut indices = Vec::new();
        self.collect_place_indices(expression, &mut indices);
        indices
    }

    fn collect_place_indices(&mut self, expression: &HirExpression, indices: &mut Vec<MirOperand>) {
        match &expression.kind {
            HirExpressionKind::Index { base, index } => {
                self.collect_place_indices(base, indices);
                indices.push(self.lower_value(index));
            }
            HirExpressionKind::Field { base, .. } | HirExpressionKind::Group(base) => {
                self.collect_place_indices(base, indices);
            }
            _ => {}
        }
    }

    fn expression_region_owner(&self, expression: &HirExpression) -> Option<LocalId> {
        match &expression.kind {
            HirExpressionKind::RegionAllocate { region, .. } => self.symbols.get(region).copied(),
            HirExpressionKind::Name {
                symbol: Some(symbol),
                ..
            } => self
                .symbols
                .get(symbol)
                .and_then(|local| self.locals[local.index()].owned_region),
            HirExpressionKind::Group(inner) => self.expression_region_owner(inner),
            _ => None,
        }
    }
}

/// One stable MIR verifier or dataflow diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MirError {
    /// Stable diagnostic code.
    pub code: &'static str,
    /// Source range.
    pub span: Span,
    /// Explanation.
    pub message: String,
}

/// Verifies local/block identities and CFG targets.
#[must_use]
pub fn verify_mir(module: &MirModule, types: &TypeStore) -> Vec<MirError> {
    let mut errors = Vec::new();
    for function in &module.functions {
        if types.kind(function.signature).is_none() {
            errors.push(error("J0502", function.span, "invalid MIR function TypeId"));
        }
        for (index, local) in function.locals.iter().enumerate() {
            if local.id.index() != index || types.kind(local.ty).is_none() {
                errors.push(error(
                    "J0502",
                    local.span,
                    "invalid MIR local identity or type",
                ));
            }
        }
        for (index, block) in function.blocks.iter().enumerate() {
            if block.id.index() != index {
                errors.push(error(
                    "J0502",
                    function.span,
                    "invalid MIR basic block identity",
                ));
            }
            for statement in &block.statements {
                verify_statement(statement, function, types, &mut errors);
            }
            verify_terminator(&block.terminator, function, types, &mut errors);
        }
    }
    errors
}

fn verify_statement(
    statement: &MirStatement,
    function: &MirFunction,
    types: &TypeStore,
    errors: &mut Vec<MirError>,
) {
    match statement {
        MirStatement::StorageLive { local, span } | MirStatement::StorageDead { local, span } => {
            verify_local(*local, *span, function, errors);
        }
        MirStatement::RegionEnter { region, span } | MirStatement::RegionExit { region, span } => {
            verify_local(*region, *span, function, errors);
        }
        MirStatement::Assign {
            destination,
            destination_indices,
            value,
            accesses,
            span,
            ..
        } => {
            verify_place(destination, *span, function, errors);
            let expected_indices = destination
                .projection
                .iter()
                .filter(|projection| matches!(projection, Projection::Index))
                .count();
            if destination_indices.len() != expected_indices {
                errors.push(error(
                    "J0502",
                    *span,
                    "MIR assignment index operand count differs from destination place",
                ));
            }
            for index in destination_indices {
                verify_operand(index, function, types, errors);
                if !types.kind(index.ty).is_some_and(TypeKind::is_integer) {
                    errors.push(error(
                        "J0502",
                        index.span,
                        "MIR assignment index operand is not an integer",
                    ));
                }
            }
            if let Some(value) = value {
                verify_operand(value, function, types, errors);
                if destination.projection.is_empty()
                    && let Some(local) = function.locals.get(destination.local.index())
                    && !mir_value_compatible(types, local.ty, value.ty)
                {
                    errors.push(error(
                        "J0502",
                        *span,
                        "MIR assignment operand type differs from destination type",
                    ));
                }
            } else {
                errors.push(error("J0502", *span, "MIR assignment has no rvalue"));
            }
            verify_accesses(accesses, function, errors);
        }
        MirStatement::Borrow {
            destination,
            source,
            span,
            ..
        } => {
            verify_local(*destination, *span, function, errors);
            verify_place(source, *span, function, errors);
        }
        MirStatement::Drop { place, span } => verify_place(place, *span, function, errors),
        MirStatement::Evaluate {
            value, accesses, ..
        } => {
            if let Some(value) = value {
                verify_operand(value, function, types, errors);
            } else if !accesses.is_empty() {
                errors.push(error(
                    "J0502",
                    statement_span(statement),
                    "MIR evaluation has accesses but no value",
                ));
            }
            verify_accesses(accesses, function, errors);
        }
    }
}

fn verify_operand(
    operand: &MirOperand,
    function: &MirFunction,
    types: &TypeStore,
    errors: &mut Vec<MirError>,
) {
    if types.kind(operand.ty).is_none() {
        errors.push(error("J0502", operand.span, "invalid MIR operand TypeId"));
    }
    match &operand.kind {
        MirOperandKind::Place(place) => verify_place(place, operand.span, function, errors),
        MirOperandKind::Unit | MirOperandKind::Literal(_) | MirOperandKind::Function { .. } => {}
        MirOperandKind::Unary { operand, .. } => {
            verify_operand(operand, function, types, errors);
        }
        MirOperandKind::Cast { operand } => {
            verify_operand(operand, function, types, errors);
        }
        MirOperandKind::Binary { left, right, .. } => {
            verify_operand(left, function, types, errors);
            verify_operand(right, function, types, errors);
        }
        MirOperandKind::Call { callee, arguments } => {
            verify_operand(callee, function, types, errors);
            for argument in arguments {
                verify_operand(argument, function, types, errors);
            }
        }
        MirOperandKind::RegionAllocate { region, arguments } => {
            if let Some(region) = region {
                verify_local(*region, operand.span, function, errors);
            } else {
                errors.push(error(
                    "J0502",
                    operand.span,
                    "region allocation has no MIR region local",
                ));
            }
            for argument in arguments {
                verify_operand(argument, function, types, errors);
            }
        }
        MirOperandKind::Index { base, index } => {
            verify_operand(base, function, types, errors);
            verify_operand(index, function, types, errors);
        }
        MirOperandKind::Length { base } => {
            verify_operand(base, function, types, errors);
            let mut base_ty = base.ty;
            if let Some(TypeKind::Capability { inner, .. }) = types.kind(base_ty) {
                base_ty = *inner;
            }
            if !matches!(
                types.kind(base_ty),
                Some(TypeKind::Array { .. } | TypeKind::Buffer(_) | TypeKind::Slice(_))
            ) {
                errors.push(error(
                    "J0502",
                    operand.span,
                    "MIR length operand requires an array, Buffer, or Slice",
                ));
            }
            if !types.kind(operand.ty).is_some_and(TypeKind::is_integer) {
                errors.push(error(
                    "J0502",
                    operand.span,
                    "MIR length operand result is not an integer",
                ));
            }
        }
        MirOperandKind::Field { base, .. } => {
            verify_operand(base, function, types, errors);
        }
        MirOperandKind::Array(elements) => {
            for element in elements {
                verify_operand(element, function, types, errors);
            }
        }
        MirOperandKind::Struct { fields, .. } => {
            for (_, value) in fields {
                verify_operand(value, function, types, errors);
            }
        }
        MirOperandKind::PatternExtract {
            source,
            source_indices,
            ..
        } => {
            verify_place(source, operand.span, function, errors);
            verify_extract_source_indices(source, source_indices, operand.span, types, errors);
            for index in source_indices {
                verify_operand(index, function, types, errors);
            }
        }
        MirOperandKind::CarrierExtract {
            source,
            source_indices,
            ..
        } => {
            verify_place(source, operand.span, function, errors);
            verify_extract_source_indices(source, source_indices, operand.span, types, errors);
            for index in source_indices {
                verify_operand(index, function, types, errors);
            }
        }
        MirOperandKind::PropagateResidual {
            source,
            residual_type,
            ..
        } => {
            verify_place(source, operand.span, function, errors);
            if types.kind(*residual_type).is_none() {
                errors.push(error(
                    "J0502",
                    operand.span,
                    "invalid propagation residual TypeId",
                ));
            }
        }
        MirOperandKind::HighLevel(_) => errors.push(error(
            "J0502",
            operand.span,
            "high-level expression reached verified MIR",
        )),
    }
}

fn verify_extract_source_indices(
    source: &Place,
    source_indices: &[MirOperand],
    span: Span,
    types: &TypeStore,
    errors: &mut Vec<MirError>,
) {
    let projection_count = source
        .projection
        .iter()
        .filter(|projection| matches!(projection, Projection::Index))
        .count();
    if projection_count != source_indices.len() {
        errors.push(error(
            "J0502",
            span,
            "MIR extract source index operand count differs from source projections",
        ));
    }
    for index in source_indices {
        if !types.kind(index.ty).is_some_and(TypeKind::is_integer) {
            errors.push(error(
                "J0502",
                index.span,
                "MIR extract source index operand is not an integer",
            ));
        }
    }
}

fn verify_terminator(
    terminator: &Terminator,
    function: &MirFunction,
    types: &TypeStore,
    errors: &mut Vec<MirError>,
) {
    match terminator {
        Terminator::Goto { target, span } => verify_block(*target, *span, function, errors),
        Terminator::Switch {
            value,
            discriminant,
            targets,
            otherwise,
            span,
        } => {
            if let Some(value) = value {
                verify_operand(value, function, types, errors);
                if types.kind(value.ty) != Some(&TypeKind::Bool) {
                    errors.push(error("J0502", *span, "MIR switch condition is not Bool"));
                }
            } else {
                errors.push(error("J0502", *span, "MIR switch has no typed condition"));
            }
            verify_accesses(discriminant, function, errors);
            for target in targets {
                verify_block(*target, *span, function, errors);
            }
            verify_block(*otherwise, *span, function, errors);
        }
        Terminator::Match {
            value,
            accesses,
            pattern,
            matched,
            otherwise,
            span,
            ..
        } => {
            verify_operand(value, function, types, errors);
            verify_mir_pattern(pattern, *span, errors);
            verify_accesses(accesses, function, errors);
            verify_block(*matched, *span, function, errors);
            verify_block(*otherwise, *span, function, errors);
        }
        Terminator::Propagate {
            value,
            accesses,
            kind,
            success,
            residual,
            span,
            ..
        } => {
            verify_operand(value, function, types, errors);
            let valid_carrier = matches!(
                (kind, types.kind(value.ty)),
                (MirPropagationKind::OptionNone, Some(TypeKind::Option(_)))
                    | (
                        MirPropagationKind::ResultError,
                        Some(TypeKind::Result { .. })
                    )
            );
            if !valid_carrier {
                errors.push(error(
                    "J0502",
                    *span,
                    "MIR propagation kind does not match its carrier type",
                ));
            }
            verify_accesses(accesses, function, errors);
            verify_block(*success, *span, function, errors);
            verify_block(*residual, *span, function, errors);
        }
        Terminator::Return {
            value,
            accesses,
            span,
        } => {
            if let Some(value) = value {
                verify_operand(value, function, types, errors);
            }
            if let Some(TypeKind::Function { result, .. }) = types.kind(function.signature) {
                match value {
                    Some(value) if !mir_value_compatible(types, *result, value.ty) => {
                        errors.push(error(
                            "J0502",
                            *span,
                            "MIR return value type differs from function result",
                        ))
                    }
                    None if types.kind(*result) != Some(&TypeKind::Unit) => errors.push(error(
                        "J0502",
                        *span,
                        "non-Unit MIR function returns without a value",
                    )),
                    _ => {}
                }
            }
            verify_accesses(accesses, function, errors);
        }
        Terminator::Unreachable { .. } => {}
    }
}

fn verify_accesses(accesses: &[PlaceAccess], function: &MirFunction, errors: &mut Vec<MirError>) {
    for access in accesses {
        verify_place(&access.place, access.span, function, errors);
    }
}

fn verify_place(place: &Place, span: Span, function: &MirFunction, errors: &mut Vec<MirError>) {
    verify_local(place.local, span, function, errors);
}

fn verify_mir_pattern(pattern: &MirPattern, span: Span, errors: &mut Vec<MirError>) {
    match pattern {
        MirPattern::Wildcard | MirPattern::Binding | MirPattern::Literal(_) => {}
        MirPattern::Path {
            path, constructor, ..
        } => {
            if constructor.is_none() && path.is_empty() {
                errors.push(error("J0502", span, "MIR pattern path has no identity"));
            }
        }
        MirPattern::Constructor {
            path,
            constructor,
            arguments,
            ..
        } => {
            if constructor.is_none() && path.is_empty() {
                errors.push(error(
                    "J0502",
                    span,
                    "MIR pattern constructor has no identity",
                ));
            }
            for argument in arguments {
                verify_mir_pattern(argument, span, errors);
            }
        }
    }
}

fn mir_value_compatible(types: &TypeStore, expected: TypeId, actual: TypeId) -> bool {
    if expected == actual {
        return true;
    }
    matches!(
        types.kind(expected),
        Some(TypeKind::Capability { inner, .. }) if *inner == actual
    )
}

fn verify_local(local: LocalId, span: Span, function: &MirFunction, errors: &mut Vec<MirError>) {
    if local.index() >= function.locals.len() {
        errors.push(error(
            "J0502",
            span,
            "MIR place references an unknown local",
        ));
    }
}

fn verify_block(
    block: BasicBlockId,
    span: Span,
    function: &MirFunction,
    errors: &mut Vec<MirError>,
) {
    if block.index() >= function.blocks.len() {
        errors.push(error(
            "J0502",
            span,
            "MIR terminator references an unknown block",
        ));
    }
}

/// Finds reads that are not initialized on every incoming CFG edge.
#[must_use]
pub fn analyze_definite_initialization(module: &MirModule) -> Vec<MirError> {
    analyze(module, AnalysisKind::Initialization)
}

/// Finds reads or moves that overlap a place consumed on any incoming CFG edge.
#[must_use]
pub fn analyze_moves(module: &MirModule) -> Vec<MirError> {
    analyze(module, AnalysisKind::Moves)
}

/// Finds overlapping read/write loans and writes through read-only capabilities.
#[must_use]
pub fn analyze_borrows(module: &MirModule, types: &TypeStore) -> Vec<MirError> {
    let mut errors = Vec::new();
    for function in &module.functions {
        errors.extend(analyze_function_borrows(function, types));
    }
    errors
}

/// Inserts `StorageDead` after the last non-terminator use of persistent borrow locals.
pub fn infer_lifetimes(module: &mut MirModule) {
    for function in &mut module.functions {
        infer_function_lifetimes(function);
    }
}

/// Inserts bulk region cleanup on every return edge, including propagation residuals.
pub fn elaborate_region_cleanup(module: &mut MirModule) {
    for function in &mut module.functions {
        elaborate_function_region_cleanup(function);
    }
}

fn elaborate_function_region_cleanup(function: &mut MirFunction) {
    if function.blocks.is_empty() {
        return;
    }
    let mut inputs: Vec<Option<BTreeSet<LocalId>>> = vec![None; function.blocks.len()];
    inputs[0] = Some(BTreeSet::new());
    let mut queue = VecDeque::from([BasicBlockId(0)]);
    while let Some(block_id) = queue.pop_front() {
        let Some(mut active) = inputs[block_id.index()].clone() else {
            continue;
        };
        let block = &function.blocks[block_id.index()];
        transfer_region_activity(&block.statements, &mut active);
        for successor in successors(&block.terminator) {
            let changed = match &mut inputs[successor.index()] {
                Some(existing) => {
                    let before = existing.len();
                    existing.extend(active.iter().copied());
                    existing.len() != before
                }
                slot @ None => {
                    *slot = Some(active.clone());
                    true
                }
            };
            if changed {
                queue.push_back(successor);
            }
        }
    }

    for block in &mut function.blocks {
        if !matches!(block.terminator, Terminator::Return { .. }) {
            continue;
        }
        let Some(mut active) = inputs[block.id.index()].clone() else {
            continue;
        };
        transfer_region_activity(&block.statements, &mut active);
        let span = terminator_span(&block.terminator);
        for region in active.iter().rev() {
            block.statements.push(MirStatement::RegionExit {
                region: *region,
                span,
            });
            block.statements.push(MirStatement::StorageDead {
                local: *region,
                span,
            });
        }
    }
}

fn transfer_region_activity(statements: &[MirStatement], active: &mut BTreeSet<LocalId>) {
    for statement in statements {
        match statement {
            MirStatement::RegionEnter { region, .. } => {
                active.insert(*region);
            }
            MirStatement::RegionExit { region, .. } => {
                active.remove(region);
            }
            _ => {}
        }
    }
}

/// Inserts explicit reverse-order drops for available move-only locals on return edges.
pub fn elaborate_drops(module: &mut MirModule, types: &TypeStore) {
    materialize_returns(module, types);
    for function in &mut module.functions {
        elaborate_function_drops(function, types);
    }
}

/// Evaluates every return expression into a dedicated local before cleanup/drop statements.
pub fn materialize_returns(module: &mut MirModule, types: &TypeStore) {
    for function in &mut module.functions {
        materialize_function_returns(function, types);
    }
}

fn materialize_function_returns(function: &mut MirFunction, types: &TypeStore) {
    for block_index in 0..function.blocks.len() {
        let (return_value, return_accesses, span) = {
            let Terminator::Return {
                value,
                accesses,
                span,
            } = &mut function.blocks[block_index].terminator
            else {
                continue;
            };
            if value.as_ref().is_none_or(|value| {
                matches!(
                    &value.kind,
                    MirOperandKind::Place(place)
                        if function.locals[place.local.index()].is_return
                )
            }) {
                continue;
            }
            (
                value.take().expect("return value checked above"),
                std::mem::take(accesses),
                *span,
            )
        };
        let owned_region = operand_owned_region(&return_value, function);
        let local = LocalId(function.locals.len());
        function.locals.push(MirLocal {
            id: local,
            symbol: None,
            name: format!("$return{}", local.index()),
            ty: return_value.ty,
            mutable: false,
            is_parameter: false,
            is_return: true,
            scope_region: None,
            owned_region,
            span,
        });
        let insertion = trailing_region_cleanup_start(&function.blocks[block_index].statements);
        let materialization = match (types.kind(return_value.ty), &return_value.kind) {
            (
                Some(TypeKind::Capability {
                    capability: Capability::Read | Capability::Write,
                    ..
                }),
                MirOperandKind::Place(source),
            ) => vec![
                MirStatement::StorageLive { local, span },
                MirStatement::Borrow {
                    destination: local,
                    source: source.clone(),
                    kind: match types.kind(return_value.ty) {
                        Some(TypeKind::Capability {
                            capability: Capability::Write,
                            ..
                        }) => BorrowKind::Write,
                        _ => BorrowKind::Read,
                    },
                    span,
                },
            ],
            _ => vec![
                MirStatement::StorageLive { local, span },
                MirStatement::Assign {
                    destination: Place::local(local),
                    destination_indices: Vec::new(),
                    value: Some(return_value),
                    accesses: return_accesses,
                    span,
                },
            ],
        };
        function.blocks[block_index]
            .statements
            .splice(insertion..insertion, materialization);
        let access_kind = if is_move_only(types, function.locals[local.index()].ty) {
            AccessKind::Move
        } else {
            AccessKind::Read
        };
        let Terminator::Return {
            value, accesses, ..
        } = &mut function.blocks[block_index].terminator
        else {
            unreachable!();
        };
        *value = Some(MirOperand {
            ty: function.locals[local.index()].ty,
            kind: MirOperandKind::Place(Place::local(local)),
            span,
        });
        *accesses = vec![PlaceAccess {
            place: Place::local(local),
            kind: access_kind,
            span,
        }];
    }
}

fn trailing_region_cleanup_start(statements: &[MirStatement]) -> usize {
    statements
        .iter()
        .rposition(|statement| {
            !matches!(
                statement,
                MirStatement::RegionExit { .. } | MirStatement::StorageDead { .. }
            )
        })
        .map_or(0, |index| index + 1)
}

fn operand_owned_region(operand: &MirOperand, function: &MirFunction) -> Option<LocalId> {
    match &operand.kind {
        MirOperandKind::Place(place) => function.locals[place.local.index()].owned_region,
        _ => None,
    }
}

/// Checks that returned or still-live loans cannot outlive their source owner.
#[must_use]
pub fn analyze_lifetimes(module: &MirModule, types: &TypeStore) -> Vec<MirError> {
    let mut errors = Vec::new();
    for function in &module.functions {
        errors.extend(analyze_function_lifetimes(function, types));
    }
    errors
}

/// Checks lexical region ownership, cleanup, and escape rules.
#[must_use]
pub fn analyze_regions(module: &MirModule) -> Vec<MirError> {
    let mut errors = Vec::new();
    for function in &module.functions {
        errors.extend(analyze_function_regions(function));
    }
    errors
}

#[derive(Clone, Copy)]
enum AnalysisKind {
    Initialization,
    Moves,
}

fn analyze(module: &MirModule, kind: AnalysisKind) -> Vec<MirError> {
    let mut errors = Vec::new();
    for function in &module.functions {
        errors.extend(match kind {
            AnalysisKind::Initialization => analyze_initialization(function),
            AnalysisKind::Moves => analyze_function_moves(function),
        });
    }
    errors
}

fn analyze_initialization(function: &MirFunction) -> Vec<MirError> {
    let entry: BTreeSet<_> = function
        .locals
        .iter()
        .filter(|local| local.is_parameter)
        .map(|local| local.id)
        .collect();
    let mut inputs = vec![None; function.blocks.len()];
    if inputs.is_empty() {
        return vec![error(
            "J0502",
            function.span,
            "MIR function has no entry block",
        )];
    }
    inputs[0] = Some(entry);
    let mut queue = VecDeque::from([BasicBlockId(0)]);
    let mut reported = BTreeSet::new();
    let mut errors = Vec::new();
    while let Some(block_id) = queue.pop_front() {
        let Some(mut state) = inputs[block_id.index()].clone() else {
            continue;
        };
        let block = &function.blocks[block_id.index()];
        for statement in &block.statements {
            match statement {
                MirStatement::StorageLive { local, .. }
                | MirStatement::StorageDead { local, .. } => {
                    state.remove(local);
                }
                MirStatement::RegionEnter { region, .. } => {
                    state.insert(*region);
                }
                MirStatement::RegionExit { region, .. } => {
                    state.remove(region);
                }
                MirStatement::Assign {
                    destination,
                    accesses,
                    ..
                } => {
                    check_initialized(accesses, &state, &mut reported, &mut errors);
                    if destination.projection.is_empty() {
                        state.insert(destination.local);
                    }
                }
                MirStatement::Borrow {
                    destination,
                    source,
                    span,
                    ..
                } => {
                    check_initialized(
                        &[PlaceAccess {
                            place: source.clone(),
                            kind: AccessKind::BorrowRead,
                            span: *span,
                        }],
                        &state,
                        &mut reported,
                        &mut errors,
                    );
                    state.insert(*destination);
                }
                MirStatement::Drop { place, span } => {
                    check_initialized(
                        &[PlaceAccess {
                            place: place.clone(),
                            kind: AccessKind::Move,
                            span: *span,
                        }],
                        &state,
                        &mut reported,
                        &mut errors,
                    );
                    if place.projection.is_empty() {
                        state.remove(&place.local);
                    }
                }
                MirStatement::Evaluate { accesses, .. } => {
                    check_initialized(accesses, &state, &mut reported, &mut errors);
                }
            }
        }
        check_initialized(
            terminator_accesses(&block.terminator),
            &state,
            &mut reported,
            &mut errors,
        );
        for successor in successors(&block.terminator) {
            let changed = match &mut inputs[successor.index()] {
                Some(existing) => {
                    let joined: BTreeSet<_> = existing.intersection(&state).copied().collect();
                    if *existing == joined {
                        false
                    } else {
                        *existing = joined;
                        true
                    }
                }
                slot @ None => {
                    *slot = Some(state.clone());
                    true
                }
            };
            if changed {
                queue.push_back(successor);
            }
        }
    }
    errors
}

fn check_initialized(
    accesses: &[PlaceAccess],
    initialized: &BTreeSet<LocalId>,
    reported: &mut BTreeSet<(u32, usize, usize)>,
    errors: &mut Vec<MirError>,
) {
    for access in accesses {
        if access.kind != AccessKind::Write && !initialized.contains(&access.place.local) {
            let key = (
                access.span.source.index(),
                access.span.start,
                access.place.local.index(),
            );
            if reported.insert(key) {
                errors.push(error(
                    "J0500",
                    access.span,
                    "use of a place that is not definitely initialized",
                ));
            }
        }
    }
}

fn analyze_function_moves(function: &MirFunction) -> Vec<MirError> {
    let mut inputs: Vec<Option<BTreeSet<Place>>> = vec![None; function.blocks.len()];
    if inputs.is_empty() {
        return Vec::new();
    }
    inputs[0] = Some(BTreeSet::new());
    let mut queue = VecDeque::from([BasicBlockId(0)]);
    let mut reported = BTreeSet::new();
    let mut errors = Vec::new();
    while let Some(block_id) = queue.pop_front() {
        let Some(mut moved) = inputs[block_id.index()].clone() else {
            continue;
        };
        let block = &function.blocks[block_id.index()];
        for statement in &block.statements {
            match statement {
                MirStatement::StorageLive { local, .. }
                | MirStatement::StorageDead { local, .. } => {
                    moved.retain(|place| place.local != *local);
                }
                MirStatement::RegionEnter { region, .. }
                | MirStatement::RegionExit { region, .. } => {
                    moved.retain(|place| place.local != *region);
                }
                MirStatement::Assign {
                    destination,
                    accesses,
                    ..
                } => {
                    apply_move_accesses(accesses, &mut moved, &mut reported, &mut errors);
                    moved.retain(|place| !place.overlaps(destination));
                }
                MirStatement::Borrow {
                    destination,
                    source,
                    span,
                    ..
                } => {
                    apply_move_accesses(
                        &[PlaceAccess {
                            place: source.clone(),
                            kind: AccessKind::BorrowRead,
                            span: *span,
                        }],
                        &mut moved,
                        &mut reported,
                        &mut errors,
                    );
                    moved.retain(|place| place.local != *destination);
                }
                MirStatement::Drop { place, span } => {
                    apply_move_accesses(
                        &[PlaceAccess {
                            place: place.clone(),
                            kind: AccessKind::Move,
                            span: *span,
                        }],
                        &mut moved,
                        &mut reported,
                        &mut errors,
                    );
                }
                MirStatement::Evaluate { accesses, .. } => {
                    apply_move_accesses(accesses, &mut moved, &mut reported, &mut errors);
                }
            }
        }
        apply_move_accesses(
            terminator_accesses(&block.terminator),
            &mut moved,
            &mut reported,
            &mut errors,
        );
        for successor in successors(&block.terminator) {
            let changed = match &mut inputs[successor.index()] {
                Some(existing) => {
                    let before = existing.len();
                    existing.extend(moved.iter().cloned());
                    existing.len() != before
                }
                slot @ None => {
                    *slot = Some(moved.clone());
                    true
                }
            };
            if changed {
                queue.push_back(successor);
            }
        }
    }
    errors
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Loan {
    holder: LocalId,
    source: Place,
    kind: BorrowKind,
}

fn infer_function_lifetimes(function: &mut MirFunction) {
    let borrowed: BTreeSet<_> = function
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match statement {
            MirStatement::Borrow { destination, .. } => Some(*destination),
            _ => None,
        })
        .collect();
    if borrowed.is_empty() || function.blocks.is_empty() {
        return;
    }

    let (live_in, live_out) = local_liveness(function);
    let _ = live_in;
    for block in &mut function.blocks {
        let mut live = live_out[block.id.index()].clone();
        live.extend(
            terminator_accesses(&block.terminator)
                .iter()
                .map(|access| access.place.local),
        );
        let mut after = vec![BTreeSet::new(); block.statements.len()];
        for (index, statement) in block.statements.iter().enumerate().rev() {
            after[index] = live.clone();
            transfer_liveness_backward(statement, &mut live);
        }

        let original = std::mem::take(&mut block.statements);
        for (index, statement) in original.into_iter().enumerate() {
            let uses = statement_uses(&statement);
            let borrow_destination = match &statement {
                MirStatement::Borrow { destination, .. } => Some(*destination),
                _ => None,
            };
            let span = statement_span(&statement);
            block.statements.push(statement);
            let mut dying: BTreeSet<_> = uses
                .intersection(&borrowed)
                .filter(|local| !after[index].contains(local))
                .copied()
                .collect();
            if let Some(destination) = borrow_destination
                && !after[index].contains(&destination)
            {
                dying.insert(destination);
            }
            for local in dying {
                block
                    .statements
                    .push(MirStatement::StorageDead { local, span });
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DropState {
    initialized: BTreeSet<LocalId>,
    moved: BTreeSet<Place>,
}

fn elaborate_function_drops(function: &mut MirFunction, types: &TypeStore) {
    if function.blocks.iter().any(|block| {
        block
            .statements
            .iter()
            .any(|statement| matches!(statement, MirStatement::Drop { .. }))
    }) || function.blocks.is_empty()
    {
        return;
    }
    let mut inputs: Vec<Option<DropState>> = vec![None; function.blocks.len()];
    inputs[0] = Some(DropState {
        initialized: function
            .locals
            .iter()
            .filter(|local| local.is_parameter)
            .map(|local| local.id)
            .collect(),
        moved: BTreeSet::new(),
    });
    let mut queue = VecDeque::from([BasicBlockId(0)]);
    while let Some(block_id) = queue.pop_front() {
        let Some(mut state) = inputs[block_id.index()].clone() else {
            continue;
        };
        let block = &function.blocks[block_id.index()];
        for statement in &block.statements {
            transfer_drop_statement(statement, &mut state);
        }
        apply_drop_accesses(terminator_accesses(&block.terminator), &mut state);
        for successor in successors(&block.terminator) {
            let changed = match &mut inputs[successor.index()] {
                Some(existing) => {
                    let joined = DropState {
                        initialized: existing
                            .initialized
                            .intersection(&state.initialized)
                            .copied()
                            .collect(),
                        moved: existing.moved.union(&state.moved).cloned().collect(),
                    };
                    if *existing == joined {
                        false
                    } else {
                        *existing = joined;
                        true
                    }
                }
                slot @ None => {
                    *slot = Some(state.clone());
                    true
                }
            };
            if changed {
                queue.push_back(successor);
            }
        }
    }

    for block in &mut function.blocks {
        let Some(mut state) = inputs[block.id.index()].clone() else {
            continue;
        };
        for statement in &block.statements {
            transfer_drop_statement(statement, &mut state);
        }
        let Terminator::Return { span, .. } = &mut block.terminator else {
            continue;
        };
        for local in function.locals.iter().rev() {
            let place = Place::local(local.id);
            if state.initialized.contains(&local.id)
                && is_move_only(types, local.ty)
                && !local.is_return
                && local.owned_region.is_none()
                && !state.moved.iter().any(|moved| moved.overlaps(&place))
            {
                block.statements.push(MirStatement::Drop {
                    place: place.clone(),
                    span: *span,
                });
                state.initialized.remove(&local.id);
                state.moved.insert(place);
            }
        }
    }
}

fn transfer_drop_statement(statement: &MirStatement, state: &mut DropState) {
    match statement {
        MirStatement::StorageLive { local, .. } | MirStatement::StorageDead { local, .. } => {
            state.initialized.remove(local);
            state.moved.retain(|place| place.local != *local);
        }
        MirStatement::RegionEnter { region, .. } => {
            state.initialized.insert(*region);
            state.moved.retain(|place| place.local != *region);
        }
        MirStatement::RegionExit { region, .. } => {
            state.initialized.remove(region);
            state.moved.retain(|place| place.local != *region);
        }
        MirStatement::Assign {
            destination,
            accesses,
            ..
        } => {
            apply_drop_accesses(accesses, state);
            if destination.projection.is_empty() {
                state.initialized.insert(destination.local);
            }
            state.moved.retain(|place| !place.overlaps(destination));
        }
        MirStatement::Borrow {
            destination,
            source,
            ..
        } => {
            apply_drop_accesses(
                &[PlaceAccess {
                    place: source.clone(),
                    kind: AccessKind::BorrowRead,
                    span: statement_span(statement),
                }],
                state,
            );
            state.initialized.insert(*destination);
            state.moved.retain(|place| place.local != *destination);
        }
        MirStatement::Drop { place, .. } => {
            state.initialized.remove(&place.local);
            state.moved.insert(place.clone());
        }
        MirStatement::Evaluate { accesses, .. } => apply_drop_accesses(accesses, state),
    }
}

fn apply_drop_accesses(accesses: &[PlaceAccess], state: &mut DropState) {
    for access in accesses {
        if access.kind == AccessKind::Move {
            state.moved.insert(access.place.clone());
        }
    }
}

fn local_liveness(function: &MirFunction) -> (Vec<BTreeSet<LocalId>>, Vec<BTreeSet<LocalId>>) {
    let mut uses = Vec::with_capacity(function.blocks.len());
    let mut definitions = Vec::with_capacity(function.blocks.len());
    for block in &function.blocks {
        let mut block_uses = BTreeSet::new();
        let mut block_definitions = BTreeSet::new();
        for statement in &block.statements {
            for local in statement_uses(statement) {
                if !block_definitions.contains(&local) {
                    block_uses.insert(local);
                }
            }
            block_definitions.extend(statement_definitions(statement));
        }
        for access in terminator_accesses(&block.terminator) {
            if !block_definitions.contains(&access.place.local) {
                block_uses.insert(access.place.local);
            }
        }
        uses.push(block_uses);
        definitions.push(block_definitions);
    }

    let mut live_in = vec![BTreeSet::new(); function.blocks.len()];
    let mut live_out = vec![BTreeSet::new(); function.blocks.len()];
    loop {
        let mut changed = false;
        for block in function.blocks.iter().rev() {
            let index = block.id.index();
            let output: BTreeSet<_> = successors(&block.terminator)
                .into_iter()
                .flat_map(|successor| live_in[successor.index()].iter().copied())
                .collect();
            let mut input = uses[index].clone();
            input.extend(output.difference(&definitions[index]).copied());
            if live_out[index] != output || live_in[index] != input {
                live_out[index] = output;
                live_in[index] = input;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    (live_in, live_out)
}

fn transfer_liveness_backward(statement: &MirStatement, live: &mut BTreeSet<LocalId>) {
    for definition in statement_definitions(statement) {
        live.remove(&definition);
    }
    live.extend(statement_uses(statement));
}

fn statement_uses(statement: &MirStatement) -> BTreeSet<LocalId> {
    match statement {
        MirStatement::Assign {
            destination,
            accesses,
            ..
        } => {
            let mut uses: BTreeSet<_> = accesses.iter().map(|access| access.place.local).collect();
            if !destination.projection.is_empty() {
                uses.insert(destination.local);
            }
            uses
        }
        MirStatement::Borrow { source, .. } => BTreeSet::from([source.local]),
        MirStatement::Drop { place, .. } => BTreeSet::from([place.local]),
        MirStatement::Evaluate { accesses, .. } => {
            accesses.iter().map(|access| access.place.local).collect()
        }
        MirStatement::RegionExit { region, .. } => BTreeSet::from([*region]),
        MirStatement::StorageLive { .. }
        | MirStatement::StorageDead { .. }
        | MirStatement::RegionEnter { .. } => BTreeSet::new(),
    }
}

fn statement_definitions(statement: &MirStatement) -> BTreeSet<LocalId> {
    match statement {
        MirStatement::StorageLive { local, .. }
        | MirStatement::StorageDead { local, .. }
        | MirStatement::RegionEnter { region: local, .. }
        | MirStatement::RegionExit { region: local, .. }
        | MirStatement::Borrow {
            destination: local, ..
        } => BTreeSet::from([*local]),
        MirStatement::Drop { place, .. } if place.projection.is_empty() => {
            BTreeSet::from([place.local])
        }
        MirStatement::Assign { destination, .. } if destination.projection.is_empty() => {
            BTreeSet::from([destination.local])
        }
        MirStatement::Assign { .. } | MirStatement::Drop { .. } | MirStatement::Evaluate { .. } => {
            BTreeSet::new()
        }
    }
}

fn statement_span(statement: &MirStatement) -> Span {
    match statement {
        MirStatement::StorageLive { span, .. }
        | MirStatement::StorageDead { span, .. }
        | MirStatement::RegionEnter { span, .. }
        | MirStatement::RegionExit { span, .. }
        | MirStatement::Assign { span, .. }
        | MirStatement::Borrow { span, .. }
        | MirStatement::Drop { span, .. }
        | MirStatement::Evaluate { span, .. } => *span,
    }
}

fn terminator_span(terminator: &Terminator) -> Span {
    match terminator {
        Terminator::Goto { span, .. }
        | Terminator::Switch { span, .. }
        | Terminator::Match { span, .. }
        | Terminator::Propagate { span, .. }
        | Terminator::Return { span, .. }
        | Terminator::Unreachable { span } => *span,
    }
}

fn analyze_function_lifetimes(function: &MirFunction, types: &TypeStore) -> Vec<MirError> {
    let mut inputs: Vec<Option<BTreeSet<Loan>>> = vec![None; function.blocks.len()];
    if inputs.is_empty() {
        return Vec::new();
    }
    inputs[0] = Some(BTreeSet::new());
    let mut queue = VecDeque::from([BasicBlockId(0)]);
    let mut errors = Vec::new();
    let mut reported = BTreeSet::new();
    while let Some(block_id) = queue.pop_front() {
        let Some(mut loans) = inputs[block_id.index()].clone() else {
            continue;
        };
        let block = &function.blocks[block_id.index()];
        for statement in &block.statements {
            match statement {
                MirStatement::Borrow {
                    destination,
                    source,
                    kind,
                    ..
                } => {
                    loans.retain(|loan| loan.holder != *destination);
                    loans.insert(Loan {
                        holder: *destination,
                        source: source.clone(),
                        kind: *kind,
                    });
                }
                MirStatement::StorageDead { local, span } => {
                    if loans.iter().any(|loan| loan.source.local == *local) {
                        report_borrow_error(
                            "J0506",
                            *span,
                            *local,
                            "borrowed value does not live long enough",
                            &mut reported,
                            &mut errors,
                        );
                    }
                    loans.retain(|loan| loan.holder != *local);
                }
                MirStatement::Assign { destination, .. } if destination.projection.is_empty() => {
                    loans.retain(|loan| loan.holder != destination.local);
                }
                MirStatement::StorageLive { local, .. } => {
                    loans.retain(|loan| loan.holder != *local);
                }
                MirStatement::RegionEnter { .. } | MirStatement::RegionExit { .. } => {}
                MirStatement::Assign { .. } | MirStatement::Evaluate { .. } => {}
                MirStatement::Drop { .. } => {}
            }
        }
        if let Terminator::Return { accesses, span, .. } = &block.terminator {
            for access in accesses {
                let Some(loan) = loans.iter().find(|loan| loan.holder == access.place.local) else {
                    continue;
                };
                let source = &function.locals[loan.source.local.index()];
                let externally_borrowed_parameter = source.is_parameter
                    && matches!(
                        types.kind(source.ty),
                        Some(TypeKind::Capability {
                            capability: Capability::Read | Capability::Write,
                            ..
                        })
                    );
                if !externally_borrowed_parameter {
                    report_borrow_error(
                        "J0505",
                        *span,
                        access.place.local,
                        "returned borrow would outlive its source owner",
                        &mut reported,
                        &mut errors,
                    );
                }
            }
        }
        for successor in successors(&block.terminator) {
            let changed = match &mut inputs[successor.index()] {
                Some(existing) => {
                    let before = existing.len();
                    existing.extend(loans.iter().cloned());
                    existing.len() != before
                }
                slot @ None => {
                    *slot = Some(loans.clone());
                    true
                }
            };
            if changed {
                queue.push_back(successor);
            }
        }
    }
    errors
}

fn analyze_function_borrows(function: &MirFunction, types: &TypeStore) -> Vec<MirError> {
    let mut inputs: Vec<Option<BTreeSet<Loan>>> = vec![None; function.blocks.len()];
    if inputs.is_empty() {
        return Vec::new();
    }
    inputs[0] = Some(BTreeSet::new());
    let mut queue = VecDeque::from([BasicBlockId(0)]);
    let mut reported = BTreeSet::new();
    let mut errors = Vec::new();
    while let Some(block_id) = queue.pop_front() {
        let Some(mut loans) = inputs[block_id.index()].clone() else {
            continue;
        };
        let block = &function.blocks[block_id.index()];
        for statement in &block.statements {
            match statement {
                MirStatement::StorageLive { local, .. }
                | MirStatement::StorageDead { local, .. } => {
                    loans.retain(|loan| loan.holder != *local);
                }
                MirStatement::RegionEnter { .. } | MirStatement::RegionExit { .. } => {}
                MirStatement::Assign {
                    destination,
                    accesses,
                    span,
                    ..
                } => {
                    check_read_only_write(
                        destination,
                        *span,
                        function,
                        types,
                        &mut reported,
                        &mut errors,
                    );
                    check_borrow_accesses(accesses, &loans, &mut reported, &mut errors);
                    if destination.projection.is_empty() {
                        loans.retain(|loan| loan.holder != destination.local);
                    }
                }
                MirStatement::Borrow {
                    destination,
                    source,
                    kind,
                    span,
                } => {
                    loans.retain(|loan| loan.holder != *destination);
                    let access = PlaceAccess {
                        place: source.clone(),
                        kind: match kind {
                            BorrowKind::Read => AccessKind::BorrowRead,
                            BorrowKind::Write => AccessKind::BorrowWrite,
                        },
                        span: *span,
                    };
                    check_borrow_accesses(&[access], &loans, &mut reported, &mut errors);
                    loans.insert(Loan {
                        holder: *destination,
                        source: source.clone(),
                        kind: *kind,
                    });
                }
                MirStatement::Drop { place, span } => check_borrow_accesses(
                    &[PlaceAccess {
                        place: place.clone(),
                        kind: AccessKind::Move,
                        span: *span,
                    }],
                    &loans,
                    &mut reported,
                    &mut errors,
                ),
                MirStatement::Evaluate { accesses, .. } => {
                    check_borrow_accesses(accesses, &loans, &mut reported, &mut errors)
                }
            }
        }
        check_borrow_accesses(
            terminator_accesses(&block.terminator),
            &loans,
            &mut reported,
            &mut errors,
        );
        for successor in successors(&block.terminator) {
            let changed = match &mut inputs[successor.index()] {
                Some(existing) => {
                    let before = existing.len();
                    existing.extend(loans.iter().cloned());
                    existing.len() != before
                }
                slot @ None => {
                    *slot = Some(loans.clone());
                    true
                }
            };
            if changed {
                queue.push_back(successor);
            }
        }
    }
    errors
}

fn analyze_function_regions(function: &MirFunction) -> Vec<MirError> {
    let mut inputs: Vec<Option<BTreeSet<LocalId>>> = vec![None; function.blocks.len()];
    if inputs.is_empty() {
        return Vec::new();
    }
    inputs[0] = Some(BTreeSet::new());
    let mut queue = VecDeque::from([BasicBlockId(0)]);
    let mut reported = BTreeSet::new();
    let mut errors = Vec::new();
    while let Some(block_id) = queue.pop_front() {
        let Some(mut active) = inputs[block_id.index()].clone() else {
            continue;
        };
        let block = &function.blocks[block_id.index()];
        for statement in &block.statements {
            match statement {
                MirStatement::RegionEnter { region, .. } => {
                    active.insert(*region);
                }
                MirStatement::RegionExit { region, .. } => {
                    active.remove(region);
                }
                MirStatement::Assign {
                    destination,
                    accesses,
                    ..
                } => {
                    check_region_accesses(
                        accesses,
                        Some(destination.local),
                        &active,
                        function,
                        &mut reported,
                        &mut errors,
                    );
                }
                MirStatement::Borrow {
                    destination,
                    source,
                    span,
                    ..
                } => check_region_accesses(
                    &[PlaceAccess {
                        place: source.clone(),
                        kind: AccessKind::BorrowRead,
                        span: *span,
                    }],
                    Some(*destination),
                    &active,
                    function,
                    &mut reported,
                    &mut errors,
                ),
                MirStatement::Evaluate { accesses, .. } => check_region_accesses(
                    accesses,
                    None,
                    &active,
                    function,
                    &mut reported,
                    &mut errors,
                ),
                MirStatement::Drop { place, span } => {
                    if function.locals[place.local.index()].owned_region.is_some() {
                        report_region_error(
                            "J0509",
                            *span,
                            place.local,
                            "region-owned value must be released by RegionExit, not Drop",
                            &mut reported,
                            &mut errors,
                        );
                    }
                }
                MirStatement::StorageLive { .. } | MirStatement::StorageDead { .. } => {}
            }
        }
        check_region_accesses(
            terminator_accesses(&block.terminator),
            None,
            &active,
            function,
            &mut reported,
            &mut errors,
        );
        for successor in successors(&block.terminator) {
            let changed = match &mut inputs[successor.index()] {
                Some(existing) => {
                    let joined: BTreeSet<_> = existing.intersection(&active).copied().collect();
                    if *existing == joined {
                        false
                    } else {
                        *existing = joined;
                        true
                    }
                }
                slot @ None => {
                    *slot = Some(active.clone());
                    true
                }
            };
            if changed {
                queue.push_back(successor);
            }
        }
    }
    errors
}

fn check_region_accesses(
    accesses: &[PlaceAccess],
    destination: Option<LocalId>,
    active: &BTreeSet<LocalId>,
    function: &MirFunction,
    reported: &mut BTreeSet<(u32, usize, usize)>,
    errors: &mut Vec<MirError>,
) {
    for access in accesses {
        let local = &function.locals[access.place.local.index()];
        let Some(owner) = local.owned_region else {
            continue;
        };
        if !active.contains(&owner) {
            report_region_error(
                if access.kind == AccessKind::Move {
                    "J0507"
                } else {
                    "J0509"
                },
                access.span,
                access.place.local,
                if access.kind == AccessKind::Move {
                    "region-owned value cannot escape its owning region"
                } else {
                    "region-owned value used after its region exited"
                },
                reported,
                errors,
            );
            continue;
        }
        if access.kind == AccessKind::Move {
            let remains_in_region = destination.is_some_and(|destination| {
                function.locals[destination.index()].scope_region == Some(owner)
            });
            if !remains_in_region {
                report_region_error(
                    "J0507",
                    access.span,
                    access.place.local,
                    "region-owned value cannot escape its owning region",
                    reported,
                    errors,
                );
            }
        }
    }
}

fn report_region_error(
    code: &'static str,
    span: Span,
    local: LocalId,
    message: &str,
    reported: &mut BTreeSet<(u32, usize, usize)>,
    errors: &mut Vec<MirError>,
) {
    let key = (span.source.index(), span.start, local.index());
    if reported.insert(key) {
        errors.push(error(code, span, message));
    }
}

fn check_read_only_write(
    destination: &Place,
    span: Span,
    function: &MirFunction,
    types: &TypeStore,
    reported: &mut BTreeSet<(u32, usize, usize)>,
    errors: &mut Vec<MirError>,
) {
    if destination.projection.is_empty() {
        return;
    }
    let Some(local) = function.locals.get(destination.local.index()) else {
        return;
    };
    if matches!(
        types.kind(local.ty),
        Some(TypeKind::Capability {
            capability: Capability::Read,
            ..
        })
    ) {
        report_borrow_error(
            "J0504",
            span,
            destination.local,
            "cannot write through a read-only capability",
            reported,
            errors,
        );
    }
}

fn check_borrow_accesses(
    accesses: &[PlaceAccess],
    active: &BTreeSet<Loan>,
    reported: &mut BTreeSet<(u32, usize, usize)>,
    errors: &mut Vec<MirError>,
) {
    let mut ephemeral: Vec<(Place, BorrowKind)> = Vec::new();
    for access in accesses {
        let place = resolve_loan_place(&access.place, active);
        for loan in active {
            if place.overlaps(&loan.source) && access_conflicts_with_loan(access.kind, loan.kind) {
                report_borrow_error(
                    "J0503",
                    access.span,
                    access.place.local,
                    "overlapping access conflicts with an active borrow",
                    reported,
                    errors,
                );
            }
        }
        for (borrowed, kind) in &ephemeral {
            if place.overlaps(borrowed) && access_conflicts_with_loan(access.kind, *kind) {
                report_borrow_error(
                    "J0503",
                    access.span,
                    access.place.local,
                    "overlapping call arguments require incompatible borrows",
                    reported,
                    errors,
                );
            }
        }
        match access.kind {
            AccessKind::BorrowRead => ephemeral.push((place, BorrowKind::Read)),
            AccessKind::BorrowWrite => ephemeral.push((place, BorrowKind::Write)),
            AccessKind::Read | AccessKind::Move | AccessKind::Write => {}
        }
    }
}

fn resolve_loan_place(place: &Place, active: &BTreeSet<Loan>) -> Place {
    let Some(loan) = active.iter().find(|loan| loan.holder == place.local) else {
        return place.clone();
    };
    let mut resolved = loan.source.clone();
    resolved.projection.extend(place.projection.iter().cloned());
    resolved
}

const fn access_conflicts_with_loan(access: AccessKind, loan: BorrowKind) -> bool {
    match (access, loan) {
        (AccessKind::Read | AccessKind::BorrowRead, BorrowKind::Read) => false,
        (
            AccessKind::Read
            | AccessKind::Move
            | AccessKind::Write
            | AccessKind::BorrowRead
            | AccessKind::BorrowWrite,
            BorrowKind::Read | BorrowKind::Write,
        ) => true,
    }
}

fn report_borrow_error(
    code: &'static str,
    span: Span,
    local: LocalId,
    message: &str,
    reported: &mut BTreeSet<(u32, usize, usize)>,
    errors: &mut Vec<MirError>,
) {
    let key = (span.source.index(), span.start, local.index());
    if reported.insert(key) {
        errors.push(error(code, span, message));
    }
}

fn apply_move_accesses(
    accesses: &[PlaceAccess],
    moved: &mut BTreeSet<Place>,
    reported: &mut BTreeSet<(u32, usize, usize)>,
    errors: &mut Vec<MirError>,
) {
    for access in accesses {
        if moved.iter().any(|place| place.overlaps(&access.place)) {
            let key = (
                access.span.source.index(),
                access.span.start,
                access.place.local.index(),
            );
            if reported.insert(key) {
                errors.push(error(
                    "J0501",
                    access.span,
                    "use of a place after its value was moved",
                ));
            }
            continue;
        }
        if access.kind == AccessKind::Move {
            moved.insert(access.place.clone());
        }
    }
}

fn terminator_accesses(terminator: &Terminator) -> &[PlaceAccess] {
    match terminator {
        Terminator::Switch { discriminant, .. } => discriminant,
        Terminator::Match { accesses, .. } => accesses,
        Terminator::Propagate { accesses, .. } => accesses,
        Terminator::Return { accesses, .. } => accesses,
        Terminator::Goto { .. } | Terminator::Unreachable { .. } => &[],
    }
}

fn successors(terminator: &Terminator) -> Vec<BasicBlockId> {
    match terminator {
        Terminator::Goto { target, .. } => vec![*target],
        Terminator::Switch {
            targets, otherwise, ..
        } => {
            let mut output = targets.clone();
            output.push(*otherwise);
            output.sort_unstable();
            output.dedup();
            output
        }
        Terminator::Match {
            matched, otherwise, ..
        } => vec![*matched, *otherwise],
        Terminator::Propagate {
            success, residual, ..
        } => vec![*success, *residual],
        Terminator::Return { .. } | Terminator::Unreachable { .. } => Vec::new(),
    }
}

/// Returns whether plain value use consumes this type in the initial 0.1 model.
#[must_use]
pub fn is_move_only(types: &TypeStore, ty: TypeId) -> bool {
    match types.kind(ty) {
        Some(TypeKind::String | TypeKind::Buffer(_) | TypeKind::Nominal { .. }) => true,
        Some(TypeKind::Array { element, .. } | TypeKind::Option(element)) => {
            is_move_only(types, *element)
        }
        Some(TypeKind::Result { ok, error }) => {
            is_move_only(types, *ok) || is_move_only(types, *error)
        }
        Some(TypeKind::Capability {
            capability: Capability::Owned,
            ..
        }) => true,
        Some(TypeKind::Capability {
            capability: Capability::Read | Capability::Write,
            ..
        }) => false,
        _ => false,
    }
}

const fn compound_assignment_operator(operator: Operator) -> Option<Operator> {
    match operator {
        Operator::PlusAssign => Some(Operator::Plus),
        Operator::MinusAssign => Some(Operator::Minus),
        Operator::StarAssign => Some(Operator::Star),
        Operator::SlashAssign => Some(Operator::Slash),
        Operator::PercentAssign => Some(Operator::Percent),
        _ => None,
    }
}

fn error(code: &'static str, span: Span, message: &str) -> MirError {
    MirError {
        code,
        span,
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use jadren_hir::lower_hir;
    use jadren_lexer::lex;
    use jadren_parser::parse;
    use jadren_resolve::resolve;
    use jadren_source::SourceManager;
    use jadren_typeck::check_types;
    use jadren_types::TypeStore;

    use super::{
        AccessKind, BasicBlockId, MirModule, Place, Terminator, analyze_borrows,
        analyze_definite_initialization, analyze_lifetimes, analyze_moves, analyze_regions,
        elaborate_drops, elaborate_region_cleanup, infer_lifetimes, lower_mir, verify_mir,
    };

    #[test]
    fn lowers_places_and_verifies_initial_mir() {
        let (module, types) = lower(
            "module test; fn consume(data: Buffer<Int32>) { let moved = data; print(moved) }",
        );
        let verification = verify_mir(&module, &types);
        assert!(verification.is_empty(), "{verification:?}");
        assert!(!module.typed_expressions.is_empty());
        assert!(
            module
                .typed_expressions
                .iter()
                .enumerate()
                .all(|(index, expression)| expression.id.index() == index)
        );
        let source = module.typed_expressions[0].span.source;
        assert!(
            module
                .query_typed_expressions()
                .source(source)
                .first()
                .is_some()
        );
        let function = &module.functions[0];
        assert_eq!(function.blocks[0].id, BasicBlockId(0));
        assert!(function.blocks[0].statements.iter().any(|statement| {
            matches!(
                statement,
                super::MirStatement::Assign { accesses, .. }
                    if accesses.iter().any(|access| access.kind == AccessKind::Move)
            )
        }));
    }

    #[test]
    fn lowers_while_break_and_continue_to_verified_cfg() {
        let (module, types) = lower(
            "module test; fn main() { var count: Int32 = 3; while count > 0 { if count == 1 { break } count -= 1 continue } }",
        );
        assert!(verify_mir(&module, &types).is_empty());
        let function = &module.functions[0];
        assert!(
            function
                .blocks
                .iter()
                .any(|block| { matches!(block.terminator, Terminator::Switch { .. }) })
        );
        assert!(function.blocks.iter().any(|block| {
            matches!(block.terminator, Terminator::Goto { target, .. } if target.index() <= block.id.index())
        }));
    }

    #[test]
    fn lowers_for_array_iteration_to_verified_indexed_cfg() {
        let (module, types) = lower(
            "module test; fn main(values: [Int32; 3]) { for value in values { if value == 2 { continue } print(value) } }",
        );
        assert!(verify_mir(&module, &types).is_empty());
        let function = &module.functions[0];
        assert!(
            function
                .blocks
                .iter()
                .any(|block| { matches!(block.terminator, Terminator::Switch { .. }) })
        );
        assert!(function.blocks.iter().any(|block| {
            block.statements.iter().any(|statement| {
                matches!(statement, super::MirStatement::Assign { value: Some(value), .. }
                    if matches!(value.kind, super::MirOperandKind::Index { .. }))
            })
        }));
    }

    #[test]
    fn lowers_buffer_and_slice_iteration_with_runtime_length() {
        let (module, types) = lower(
            "module test; fn buffer(values: Buffer<Int32>) { for value in values { print(value) } } fn slice(values: Slice<Int32>) { for value in values { print(value) } }",
        );
        assert!(verify_mir(&module, &types).is_empty());
        for function in &module.functions {
            assert!(function.blocks.iter().any(|block| {
                block.statements.iter().any(|statement| {
                    matches!(statement, super::MirStatement::Assign { value: Some(value), .. }
                        if matches!(value.kind, super::MirOperandKind::Index { .. }))
                })
            }));
            assert!(function.blocks.iter().any(|block| {
                matches!(block.terminator, Terminator::Switch { ref value, .. }
                    if value.as_ref().is_some_and(|value| matches!(value.kind, super::MirOperandKind::Binary { ref right, .. }
                        if matches!(right.kind, super::MirOperandKind::Length { .. }))))
            }));
        }
    }

    #[test]
    fn lowers_slice_index_iteration_to_writable_projected_places() {
        let (module, types) = lower(
            "module test; struct Agent { position: Int32, velocity: Int32 } fn update(agents: write Slice<Agent>) { for index in agents.indices { agents[index].position = agents[index].position + agents[index].velocity } }",
        );
        assert!(verify_mir(&module, &types).is_empty());
        let function = &module.functions[0];
        assert!(function.blocks.iter().any(|block| {
            block.statements.iter().any(|statement| {
                matches!(statement, super::MirStatement::Assign { destination, .. }
                    if destination.projection.iter().any(|projection| matches!(projection, super::Projection::Index))
                        && destination.projection.iter().any(|projection| matches!(projection, super::Projection::Field(field) if field == "position")))
            })
        }));
    }

    #[test]
    fn reports_use_of_uninitialized_local() {
        let (module, _) = lower("module test; fn main() { let value: Int32; print(value) }");
        let errors = analyze_definite_initialization(&module);
        assert!(errors.iter().any(|error| error.code == "J0500"));

        let (initialized, _) =
            lower("module test; fn main() { var value: Int32; value = 1; print(value) }");
        assert!(analyze_definite_initialization(&initialized).is_empty());
    }

    #[test]
    fn reports_use_after_move_and_accepts_copy_scalars() {
        let (moved, _) =
            lower("module test; fn consume(data: Buffer<Int32>) { let first = data; print(data) }");
        assert!(
            analyze_moves(&moved)
                .iter()
                .any(|error| error.code == "J0501")
        );

        let (copied, _) =
            lower("module test; fn copy(value: Int32) { let first = value; print(value) }");
        assert!(analyze_moves(&copied).is_empty());

        let (reinitialized, _) = lower(
            "module test; fn replace(first: Buffer<Int32>, second: Buffer<Int32>) { var data = first; let old = data; data = second; print(data) }",
        );
        assert!(analyze_moves(&reinitialized).is_empty());
    }

    #[test]
    fn definite_initialization_joins_cfg_with_intersection() {
        let (mut module, _) = lower("module test; fn main() { let value: Int32 }");
        let function = &mut module.functions[0];
        let span = function.span;
        let value = Place::local(function.locals[0].id);
        function.blocks = vec![
            super::BasicBlock {
                id: BasicBlockId(0),
                statements: Vec::new(),
                terminator: Terminator::Switch {
                    value: None,
                    discriminant: Vec::new(),
                    targets: vec![BasicBlockId(1)],
                    otherwise: BasicBlockId(2),
                    span,
                },
            },
            super::BasicBlock {
                id: BasicBlockId(1),
                statements: vec![super::MirStatement::Assign {
                    destination: value.clone(),
                    destination_indices: Vec::new(),
                    value: None,
                    accesses: Vec::new(),
                    span,
                }],
                terminator: Terminator::Goto {
                    target: BasicBlockId(3),
                    span,
                },
            },
            super::BasicBlock {
                id: BasicBlockId(2),
                statements: Vec::new(),
                terminator: Terminator::Goto {
                    target: BasicBlockId(3),
                    span,
                },
            },
            super::BasicBlock {
                id: BasicBlockId(3),
                statements: vec![super::MirStatement::Evaluate {
                    value: None,
                    accesses: vec![super::PlaceAccess {
                        place: value,
                        kind: AccessKind::Read,
                        span,
                    }],
                    span,
                }],
                terminator: Terminator::Return {
                    value: None,
                    accesses: Vec::new(),
                    span,
                },
            },
        ];
        assert!(
            analyze_definite_initialization(&module)
                .iter()
                .any(|error| error.code == "J0500")
        );
    }

    #[test]
    fn permits_shared_borrows_and_rejects_overlapping_write_borrow() {
        let (shared, types) = lower(
            "module test; fn inspect(first: read Buffer<Int32>, second: read Buffer<Int32>) {} fn run(data: Buffer<Int32>) { inspect(data, data) }",
        );
        assert!(analyze_borrows(&shared, &types).is_empty());
        assert!(analyze_moves(&shared).is_empty());

        let (conflict, types) = lower(
            "module test; fn update(first: read Buffer<Int32>, second: write Buffer<Int32>) {} fn run(data: Buffer<Int32>) { update(data, data) }",
        );
        assert!(
            analyze_borrows(&conflict, &types)
                .iter()
                .any(|error| error.code == "J0503")
        );
    }

    #[test]
    fn persistent_borrow_blocks_move_and_read_capability_blocks_write() {
        let (borrowed, types) = lower(
            "module test; fn consume(value: Buffer<Int32>) {} fn run(data: Buffer<Int32>) { let view: read Buffer<Int32> = data; consume(data); print(view) }",
        );
        assert!(
            analyze_borrows(&borrowed, &types)
                .iter()
                .any(|error| error.code == "J0503")
        );

        let (read_only, types) =
            lower("module test; fn mutate(data: read Buffer<Int32>) { data[0] = 1 }");
        assert!(
            analyze_borrows(&read_only, &types)
                .iter()
                .any(|error| error.code == "J0504")
        );
    }

    #[test]
    fn lifetime_inference_ends_borrow_after_last_use() {
        let (mut module, types) = lower(
            "module test; fn consume(value: Buffer<Int32>) {} fn run(data: Buffer<Int32>) { let view: read Buffer<Int32> = data; print(view); consume(data) }",
        );
        assert!(
            analyze_borrows(&module, &types)
                .iter()
                .any(|error| error.code == "J0503")
        );
        infer_lifetimes(&mut module);
        assert!(analyze_borrows(&module, &types).is_empty());
        assert!(module.functions[1].blocks[0]
            .statements
            .iter()
            .any(|statement| matches!(statement, super::MirStatement::StorageDead { local, .. } if local.index() == 1)));
    }

    #[test]
    fn rejects_escaping_borrow_and_owner_ending_before_borrow() {
        let (escaping, types) = lower(
            "module test; fn borrow(data: Buffer<Int32>) -> read Buffer<Int32> { let view: read Buffer<Int32> = data; return view }",
        );
        assert!(
            analyze_lifetimes(&escaping, &types)
                .iter()
                .any(|error| error.code == "J0505")
        );

        let (mut short, types) = lower(
            "module test; fn run(data: Buffer<Int32>) { let view: read Buffer<Int32> = data; print(view) }",
        );
        let function = &mut short.functions[0];
        let span = function.span;
        function.blocks[0].statements.insert(
            2,
            super::MirStatement::StorageDead {
                local: function.locals[0].id,
                span,
            },
        );
        assert!(
            analyze_lifetimes(&short, &types)
                .iter()
                .any(|error| error.code == "J0506")
        );
    }

    #[test]
    fn elaborates_available_move_only_drops_in_reverse_order() {
        let (mut module, types) =
            lower("module test; fn release(first: Buffer<Int32>, second: Buffer<Int32>) {}");
        elaborate_drops(&mut module, &types);
        let dropped: Vec<_> = module.functions[0].blocks[0]
            .statements
            .iter()
            .filter_map(|statement| match statement {
                super::MirStatement::Drop { place, .. } => Some(place.local.index()),
                _ => None,
            })
            .collect();
        assert_eq!(dropped, [1, 0]);
        assert!(verify_mir(&module, &types).is_empty());
        assert!(analyze_definite_initialization(&module).is_empty());
        assert!(analyze_moves(&module).is_empty());

        let (mut moved, types) =
            lower("module test; fn release(data: Buffer<Int32>) { let replacement = data }");
        elaborate_drops(&mut moved, &types);
        let dropped: Vec<_> = moved.functions[0].blocks[0]
            .statements
            .iter()
            .filter_map(|statement| match statement {
                super::MirStatement::Drop { place, .. } => Some(place.local.index()),
                _ => None,
            })
            .collect();
        assert_eq!(dropped, [1]);
    }

    #[test]
    fn evaluates_return_move_before_drop_elaboration() {
        let (mut module, types) =
            lower("module test; fn pass(data: Buffer<Int32>) -> Buffer<Int32> { return data }");
        elaborate_drops(&mut module, &types);
        let function = &module.functions[0];
        let block = &function.blocks[0];
        let return_local = function
            .locals
            .iter()
            .find(|local| local.is_return)
            .expect("return value must be materialized")
            .id;
        assert!(matches!(
            block.statements.iter().find(|statement| matches!(
                statement,
                super::MirStatement::Assign { destination, .. }
                    if destination.local == return_local
            )),
            Some(super::MirStatement::Assign {
                destination,
                value: Some(super::MirOperand {
                    kind: super::MirOperandKind::Place(_),
                    ..
                }),
                accesses,
                ..
            }) if destination.local == return_local
                && accesses.iter().any(|access| access.kind == AccessKind::Move)
        ));
        assert!(matches!(
            block.terminator,
            Terminator::Return {
                value: Some(super::MirOperand {
                    kind: super::MirOperandKind::Place(ref place),
                    ..
                }),
                ref accesses,
                ..
            } if place.local == return_local
                && accesses.iter().any(|access| {
                    access.place.local == return_local && access.kind == AccessKind::Move
                })
        ));
        assert!(!block.statements.iter().any(|statement| matches!(
            statement,
            super::MirStatement::Drop { place, .. }
                if place.local == return_local || place.local.index() == 0
        )));
    }

    #[test]
    fn owned_parameter_transfer_moves_and_drops_exactly_once() {
        let (mut module, types) = lower(
            "module test; fn take(value: owned Buffer<Int32>) {} fn run(data: Buffer<Int32>) { take(data); print(data) }",
        );
        assert!(
            analyze_moves(&module)
                .iter()
                .any(|error| error.code == "J0501")
        );
        elaborate_drops(&mut module, &types);
        let take_drops = module.functions[0].blocks[0]
            .statements
            .iter()
            .filter(|statement| matches!(statement, super::MirStatement::Drop { .. }))
            .count();
        assert_eq!(take_drops, 1);
        assert!(!module.functions[1].blocks[0]
            .statements
            .iter()
            .any(|statement| matches!(statement, super::MirStatement::Drop { place, .. } if place.local.index() == 0)));
    }

    #[test]
    fn lowers_region_cleanup_and_rejects_region_owned_escape() {
        let (mut valid, types) = lower(
            "module test; fn inspect(value: read Buffer<Int32>) {} fn run() { region frame { let values: Buffer<Int32> = frame.allocate(4); let replacement = values; inspect(replacement) } }",
        );
        infer_lifetimes(&mut valid);
        elaborate_drops(&mut valid, &types);
        assert!(analyze_regions(&valid).is_empty());
        let function = &valid.functions[1];
        assert!(
            function.blocks[0]
                .statements
                .iter()
                .any(|statement| matches!(statement, super::MirStatement::RegionEnter { .. }))
        );
        assert!(
            function.blocks[0]
                .statements
                .iter()
                .any(|statement| matches!(statement, super::MirStatement::RegionExit { .. }))
        );
        assert!(!function.blocks[0].statements.iter().any(|statement| {
            matches!(statement, super::MirStatement::Drop { place, .. }
                if function.locals[place.local.index()].owned_region.is_some())
        }));

        let (escaping, _) = lower(
            "module test; fn leak() -> Buffer<Int32> { region frame { let values: Buffer<Int32> = frame.allocate(4); return values } }",
        );
        assert!(
            analyze_regions(&escaping)
                .iter()
                .any(|error| error.code == "J0507")
        );

        let (assignment, _) = lower(
            "module test; fn leak() { var outside: Buffer<Int32>; region frame { let values: Buffer<Int32> = frame.allocate(4); outside = values } }",
        );
        assert!(
            analyze_regions(&assignment)
                .iter()
                .any(|error| error.code == "J0507")
        );
    }

    #[test]
    fn preserves_typed_literal_operator_and_call_operands() {
        let (module, _) = lower(
            "module test; fn add(value: Int32) -> Int32 { let sum = value + 1; print(sum); return sum }",
        );
        let statements = &module.functions[0].blocks[0].statements;
        let Some(super::MirStatement::Assign {
            value: Some(value), ..
        }) = statements
            .iter()
            .find(|statement| matches!(statement, super::MirStatement::Assign { .. }))
        else {
            panic!("expected value assignment");
        };
        let super::MirOperandKind::Binary {
            operator,
            left,
            right,
        } = &value.kind
        else {
            panic!("expected binary rvalue");
        };
        assert_eq!(*operator, jadren_lexer::Operator::Plus);
        assert!(matches!(left.kind, super::MirOperandKind::Place(_)));
        let super::MirOperandKind::Literal(literal) = &right.kind else {
            panic!("expected literal operand");
        };
        assert_eq!(literal.text, "1");

        let Some(super::MirStatement::Evaluate {
            value: Some(value), ..
        }) = statements
            .iter()
            .find(|statement| matches!(statement, super::MirStatement::Evaluate { .. }))
        else {
            panic!("expected call evaluation");
        };
        let super::MirOperandKind::Call { callee, arguments } = &value.kind else {
            panic!("expected call operand");
        };
        assert_eq!(arguments.len(), 1);
        assert!(matches!(
            &callee.kind,
            super::MirOperandKind::Function { name, symbol: Some(_) } if name == "print"
        ));
    }

    #[test]
    fn lowers_if_value_to_temporary_and_cfg_join() {
        let (module, types) = lower(
            "module test; fn choose(flag: Bool) -> Int32 { let selected = if flag { 1 } else { 2 }; return selected }",
        );
        let function = &module.functions[0];
        assert_eq!(function.blocks.len(), 4);
        assert!(matches!(
            function.blocks[0].terminator,
            Terminator::Switch {
                value: Some(_),
                ref targets,
                otherwise: BasicBlockId(2),
                ..
            } if targets == &[BasicBlockId(1)]
        ));
        for block in &function.blocks[1..3] {
            assert!(block.statements.iter().any(|statement| matches!(
                statement,
                super::MirStatement::Assign {
                    destination,
                    value: Some(super::MirOperand {
                        kind: super::MirOperandKind::Literal(_),
                        ..
                    }),
                    ..
                } if destination.local.index() == 2
            )));
            assert!(matches!(
                block.terminator,
                Terminator::Goto {
                    target: BasicBlockId(3),
                    ..
                }
            ));
        }
        assert!(verify_mir(&module, &types).is_empty());
        assert!(analyze_definite_initialization(&module).is_empty());
        assert!(analyze_moves(&module).is_empty());
        assert!(!function.blocks.iter().any(|block| {
            block.statements.iter().any(|statement| match statement {
                super::MirStatement::Assign {
                    value: Some(value), ..
                }
                | super::MirStatement::Evaluate {
                    value: Some(value), ..
                } => matches!(value.kind, super::MirOperandKind::HighLevel(_)),
                _ => false,
            })
        }));
    }

    #[test]
    fn lowers_guarded_match_with_typed_payload_bindings() {
        let (module, types) = lower(
            "module test; enum Choice { First(Int32), Second(Int32) } fn choose(value: Choice) -> Int32 { return match value { Choice.First(item) if item > 0 => item, Choice.Second(item) => item, _ => 0 } }",
        );
        let function = &module.functions[0];
        assert_eq!(
            function
                .blocks
                .iter()
                .filter(|block| matches!(block.terminator, Terminator::Match { .. }))
                .count(),
            3
        );
        assert!(function.blocks.iter().any(|block| matches!(
            &block.terminator,
            Terminator::Switch {
                value: Some(super::MirOperand {
                    kind: super::MirOperandKind::Binary { left, .. },
                    ..
                }),
                ..
            } if matches!(
                left.kind,
                super::MirOperandKind::PatternExtract { borrowed: true, .. }
            )
        )));
        let extracted: Vec<_> = function
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter_map(|statement| match statement {
                super::MirStatement::Assign {
                    value:
                        Some(super::MirOperand {
                            kind:
                                super::MirOperandKind::PatternExtract {
                                    path,
                                    borrowed: false,
                                    ..
                                },
                            ..
                        }),
                    ..
                } => Some(path),
                _ => None,
            })
            .collect();
        assert_eq!(extracted.len(), 2);
        assert_ne!(extracted[0][0], extracted[1][0]);
        let verification = verify_mir(&module, &types);
        assert!(verification.is_empty(), "{verification:?}");
        assert!(analyze_definite_initialization(&module).is_empty());
        assert!(analyze_moves(&module).is_empty());
    }

    #[test]
    fn verifier_rejects_extract_index_count_mismatch_before_jir_lowering() {
        let (mut module, types) = lower(
            "module test; enum Choice { First(Int32), Second(Int32) } fn choose(value: Choice) -> Int32 { return match value { Choice.First(item) => item, Choice.Second(item) => item, _ => 0 } }",
        );
        let extraction = module.functions[0]
            .blocks
            .iter_mut()
            .flat_map(|block| block.statements.iter_mut())
            .find_map(|statement| match statement {
                super::MirStatement::Assign {
                    value:
                        Some(super::MirOperand {
                            kind:
                                super::MirOperandKind::PatternExtract {
                                    source,
                                    source_indices,
                                    ..
                                },
                            ..
                        }),
                    ..
                } => Some((source, source_indices)),
                _ => None,
            })
            .expect("pattern extraction assignment");
        extraction.0.projection.push(super::Projection::Index);
        assert!(extraction.1.is_empty());
        assert!(verify_mir(&module, &types).iter().any(|error| {
            error
                .message
                .contains("extract source index operand count differs")
        }));
    }

    #[test]
    fn verifier_rejects_non_integer_extract_index_before_jir_lowering() {
        let (mut module, types) = lower(
            "module test; enum Choice { First(Int32), Second(Int32) } fn choose(value: Choice) -> Int32 { return match value { Choice.First(item) => item, Choice.Second(item) => item, _ => 0 } }",
        );
        let function_span = module.functions[0].span;
        let extraction = module.functions[0]
            .blocks
            .iter_mut()
            .flat_map(|block| block.statements.iter_mut())
            .find_map(|statement| match statement {
                super::MirStatement::Assign {
                    value:
                        Some(super::MirOperand {
                            kind:
                                super::MirOperandKind::PatternExtract {
                                    source,
                                    source_indices,
                                    ..
                                },
                            ..
                        }),
                    ..
                } => Some((source, source_indices)),
                _ => None,
            })
            .expect("pattern extraction assignment");
        extraction.0.projection.push(super::Projection::Index);
        extraction.1.push(super::MirOperand {
            kind: super::MirOperandKind::Literal(jadren_hir::HirLiteral {
                kind: jadren_parser::LiteralKind::Bool,
                text: "true".to_owned(),
            }),
            ty: types.core().bool_,
            span: function_span,
        });
        assert!(verify_mir(&module, &types).iter().any(|error| {
            error
                .message
                .contains("extract source index operand is not an integer")
        }));
    }

    #[test]
    fn lowers_result_propagation_to_success_and_residual_cfg() {
        let (module, types) = lower(
            "module test; fn load() -> Result<Int32, String> { return Ok(1) } fn run() -> Result<Int32, String> { let value = load()?; return Ok(value) }",
        );
        let function = &module.functions[1];
        assert_eq!(function.blocks.len(), 4);
        assert!(matches!(
            function.blocks[0].terminator,
            Terminator::Propagate {
                kind: super::MirPropagationKind::ResultError,
                success: BasicBlockId(1),
                residual: BasicBlockId(2),
                ..
            }
        ));
        assert!(
            function.blocks[1]
                .statements
                .iter()
                .any(|statement| matches!(
                    statement,
                    super::MirStatement::Assign {
                        value: Some(super::MirOperand {
                            kind: super::MirOperandKind::CarrierExtract {
                                part: super::CarrierPart::Success,
                                ..
                            },
                            ..
                        }),
                        ..
                    }
                ))
        );
        assert!(matches!(
            &function.blocks[2].terminator,
            Terminator::Return {
                value: Some(super::MirOperand {
                    kind: super::MirOperandKind::PropagateResidual {
                        kind: super::MirPropagationKind::ResultError,
                        ..
                    },
                    ..
                }),
                ..
            }
        ));
        assert!(verify_mir(&module, &types).is_empty());
        assert!(analyze_definite_initialization(&module).is_empty());
        assert!(analyze_moves(&module).is_empty());

        let (option, option_types) = lower(
            "module test; fn load() -> Option<Int32> { return Some(1) } fn run() -> Option<Int32> { let value = load()?; return Some(value) }",
        );
        assert!(matches!(
            option.functions[1].blocks[0].terminator,
            Terminator::Propagate {
                kind: super::MirPropagationKind::OptionNone,
                ..
            }
        ));
        assert!(verify_mir(&option, &option_types).is_empty());
        assert!(analyze_definite_initialization(&option).is_empty());
        assert!(analyze_moves(&option).is_empty());
    }

    #[test]
    fn elaborates_region_cleanup_on_propagation_return_edges() {
        let (mut module, types) = lower(
            "module test; fn load() -> Result<Int32, String> { return Ok(1) } fn run() -> Result<Int32, String> { region frame { let values: Buffer<Int32> = frame.allocate(4); let value = load()?; return Ok(value) } }",
        );
        elaborate_region_cleanup(&mut module);
        let function = &module.functions[1];
        let return_blocks: Vec<_> = function
            .blocks
            .iter()
            .filter(|block| matches!(block.terminator, Terminator::Return { .. }))
            .collect();
        assert_eq!(return_blocks.len(), 2);
        assert!(return_blocks.iter().all(|block| {
            block
                .statements
                .iter()
                .any(|statement| matches!(statement, super::MirStatement::RegionExit { .. }))
        }));
        assert!(verify_mir(&module, &types).is_empty());
        assert!(analyze_regions(&module).is_empty());
    }

    #[test]
    fn recursively_lowers_nested_control_without_high_level_operands() {
        let (module, types) = lower(
            "module test; fn load() -> Result<Int32, String> { return Ok(1) } fn run(flag: Bool) -> Result<Int32, String> { let value = if flag { load()? } else { 0 }; return Ok(value) }",
        );
        assert!(module.functions[1].blocks.len() > 4);
        assert!(verify_mir(&module, &types).is_empty());
        assert!(analyze_definite_initialization(&module).is_empty());
        assert!(analyze_moves(&module).is_empty());
    }

    #[test]
    fn verifier_rejects_missing_rvalue_and_non_bool_switch() {
        let (mut missing, types) = lower("module test; fn run() { let value = 1 }");
        let statement = missing.functions[0].blocks[0]
            .statements
            .iter_mut()
            .find(|statement| matches!(statement, super::MirStatement::Assign { .. }))
            .expect("assignment");
        let super::MirStatement::Assign { value, .. } = statement else {
            unreachable!();
        };
        *value = None;
        assert!(
            verify_mir(&missing, &types)
                .iter()
                .any(|error| error.message.contains("no rvalue"))
        );

        let (mut switched, types) = lower(
            "module test; fn choose(flag: Bool) -> Int32 { return if flag { 1 } else { 2 } }",
        );
        let Terminator::Switch {
            value: Some(condition),
            ..
        } = &mut switched.functions[0].blocks[0].terminator
        else {
            panic!("switch");
        };
        condition.ty = types.core().int32;
        assert!(
            verify_mir(&switched, &types)
                .iter()
                .any(|error| error.message.contains("not Bool"))
        );
    }

    #[test]
    fn preserves_dynamic_indices_for_projected_assignment() {
        let (mut module, types) = lower(
            "module test; fn update(index: Int32) { let values: [Int32; 2] = [1, 2]; values[index] = 9 }",
        );
        let assignment = module.functions[0].blocks[0]
            .statements
            .iter_mut()
            .find(|statement| {
                matches!(
                    statement,
                    super::MirStatement::Assign { destination, .. }
                        if destination.projection.iter().any(|projection| {
                            matches!(projection, super::Projection::Index)
                        })
                )
            })
            .expect("indexed assignment");
        let super::MirStatement::Assign {
            destination_indices,
            accesses,
            ..
        } = assignment
        else {
            unreachable!();
        };
        assert!(matches!(
            destination_indices.as_slice(),
            [super::MirOperand {
                kind: super::MirOperandKind::Place(place),
                ..
            }] if place.local.index() == 0
        ));
        assert!(
            accesses.iter().any(|access| {
                access.place.local.index() == 0 && access.kind == AccessKind::Read
            })
        );
        destination_indices.clear();
        assert!(verify_mir(&module, &types).iter().any(|error| {
            error
                .message
                .contains("index operand count differs from destination")
        }));
    }

    fn lower(text: &str) -> (MirModule, TypeStore) {
        let mut sources = SourceManager::new();
        let id = sources.add("test.jdn", text).expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics);
        let resolution = resolve(source, &parsed.file);
        assert!(!resolution.has_errors(), "{:?}", resolution.diagnostics);
        let checked = check_types(source, &parsed.file, &resolution);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolution, &checked);
        assert!(hir.diagnostics.is_empty(), "{:?}", hir.diagnostics);
        (lower_mir(&hir.module, &checked.types), checked.types)
    }
}
