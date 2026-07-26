//! Small, deterministic JIR optimizations that preserve SSA identities.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    AddressSpace, AliasAnalysis, AliasRelation, BinaryOp, BlockId, CastOp, ComparePredicate,
    Constant, Function, FunctionId, Instruction, InstructionKind, Module, Terminator, Type, TypeId,
    TypedValue, UnaryOp, ValueId, analyze_aliases,
};

const INLINE_MAX_INSTRUCTIONS: usize = 12;

/// Summary of one constant-folding pass.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OptimizationStats {
    /// Number of non-constant instructions replaced by a `Constant`.
    pub folded_instructions: usize,
    /// Number of constant branches or switches rewritten to one edge.
    pub simplified_terminators: usize,
    /// Number of unused or redundant side-effect-free SSA instructions removed.
    pub removed_instructions: usize,
    /// Number of internal tiny calls expanded into the caller.
    pub inlined_calls: usize,
    /// Number of non-escaping stack slots promoted to SSA values.
    pub promoted_stack_slots: usize,
    /// Number of bounds checks proven safe and removed.
    pub eliminated_bounds_checks: usize,
    /// Number of natural loops with a unique preheader recognized.
    pub canonicalized_loops: usize,
    /// Number of pure scalar instructions or proven-invariant stack loads hoisted out of loops.
    pub hoisted_loop_instructions: usize,
}

/// Inlines only small, single-block, side-effect-free internal functions.
///
/// The pass deliberately excludes memory, allocation, runtime, callback and
/// recursive calls. Cloned values receive fresh temporary IDs and the caller
/// is compacted back to dense SSA identities after each changed function.
pub fn inline_tiny_functions(module: &mut Module) -> OptimizationStats {
    let candidates: BTreeMap<_, _> = module
        .functions
        .iter()
        .filter(|function| is_tiny_inline_candidate(function))
        .map(|function| (function.id, function.clone()))
        .collect();
    let mut stats = OptimizationStats::default();
    for caller in &mut module.functions {
        let mut changed = false;
        let caller_id = caller.id;
        let mut next_value = next_value_id(caller);
        for block in &mut caller.blocks {
            let old_instructions = std::mem::take(&mut block.instructions);
            let mut retained = Vec::with_capacity(old_instructions.len());
            for instruction in old_instructions {
                let inline = match &instruction.kind {
                    InstructionKind::Call {
                        function,
                        arguments,
                    } => candidates
                        .get(function)
                        .filter(|callee| callee.id != caller_id)
                        .and_then(|callee| {
                            inline_call(instruction.result, arguments, callee, &mut next_value)
                        }),
                    _ => None,
                };
                if let Some(inlined) = inline {
                    retained.extend(inlined);
                    stats.inlined_calls += 1;
                    changed = true;
                } else {
                    retained.push(instruction);
                }
            }
            block.instructions = retained;
        }
        if changed {
            renumber_values(caller);
        }
    }
    stats
}

/// Promotes non-escaping scalar, vector, and aggregate stack slots to ordinary
/// SSA values.
///
/// The pass remains conservative: a multi-store slot must have every direct
/// load follow a store in the same basic block, while a single immutable store
/// may feed dominated loads across the CFG. Any pointer use other than a
/// non-volatile direct load/store rejects the candidate. This prevents the
/// pass from changing aliasing, volatile, panic or ownership semantics.
pub fn promote_scalar_stack_slots(module: &mut Module) -> OptimizationStats {
    let types = &module.types;
    let mut stats = OptimizationStats::default();
    for function in &mut module.functions {
        promote_function_stack_slots(function, types, &mut stats);
    }
    stats
}

/// Removes bounds checks whose two operands are known non-negative constants
/// and satisfy `index < length`. All uncertain cases retain the runtime panic
/// edge; no signed/unsigned reinterpretation is guessed here.
pub fn eliminate_proven_bounds_checks(module: &mut Module) -> OptimizationStats {
    let mut stats = OptimizationStats::default();
    for function in &mut module.functions {
        let constants: BTreeMap<_, _> = function
            .blocks
            .iter()
            .flat_map(|block| block.instructions.iter())
            .filter_map(
                |instruction| match (&instruction.result, &instruction.kind) {
                    (Some(result), InstructionKind::Constant(Constant::Integer { value })) => {
                        Some((result.value, *value))
                    }
                    _ => None,
                },
            )
            .collect();
        for block in &mut function.blocks {
            block.instructions.retain(|instruction| {
                let (index, length) = match &instruction.kind {
                    InstructionKind::BoundsCheck { index, length } => (*index, *length),
                    _ => return true,
                };
                let proven = constants
                    .get(&index)
                    .zip(constants.get(&length))
                    .is_some_and(|(index, length)| *index >= 0 && *length > 0 && *index < *length);
                if proven {
                    stats.eliminated_bounds_checks += 1;
                    false
                } else {
                    true
                }
            });
        }
    }
    stats
}

/// Removes repeated bounds checks in one basic block when their operands are
/// the same SSA values or the same read-only stack-load expressions. A store
/// invalidates only checks whose stack-load dependency may alias the stored
/// pointer according to the conservative JIR alias analysis; calls clear the
/// local proof set conservatively.
pub fn eliminate_redundant_bounds_checks(module: &mut Module) -> OptimizationStats {
    let mut stats = OptimizationStats::default();
    let aliases = analyze_aliases(module);
    for function in &mut module.functions {
        let definitions = function
            .blocks
            .iter()
            .flat_map(|block| block.instructions.iter())
            .filter_map(|instruction| {
                instruction
                    .result
                    .map(|result| (result.value, instruction.kind.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        let stack_slots = definitions
            .iter()
            .filter_map(|(value, kind)| match kind {
                InstructionKind::StackAlloc { .. } => Some(*value),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        let mut expressions = BTreeMap::<ValueId, CanonicalValue>::new();
        for block in &mut function.blocks {
            let mut seen = BTreeSet::<(CanonicalValue, CanonicalValue)>::new();
            let mut vector_seen = BTreeSet::<(CanonicalValue, CanonicalValue, u16)>::new();
            let old_instructions = std::mem::take(&mut block.instructions);
            let mut retained = Vec::with_capacity(old_instructions.len());
            for instruction in old_instructions {
                match &instruction.kind {
                    InstructionKind::Store { pointer, .. } => {
                        seen.retain(|(index, length)| {
                            !stack_load_may_alias(
                                index,
                                *pointer,
                                function.id,
                                &aliases,
                                &stack_slots,
                            ) && !stack_load_may_alias(
                                length,
                                *pointer,
                                function.id,
                                &aliases,
                                &stack_slots,
                            )
                        });
                        vector_seen.retain(|(index, length, _)| {
                            !stack_load_may_alias(
                                index,
                                *pointer,
                                function.id,
                                &aliases,
                                &stack_slots,
                            ) && !stack_load_may_alias(
                                length,
                                *pointer,
                                function.id,
                                &aliases,
                                &stack_slots,
                            )
                        });
                    }
                    InstructionKind::Call { .. } => {
                        seen.clear();
                        vector_seen.clear();
                    }
                    InstructionKind::BoundsCheck { index, length } => {
                        let key = (
                            canonical_value(*index, &definitions, &stack_slots, &mut expressions),
                            canonical_value(*length, &definitions, &stack_slots, &mut expressions),
                        );
                        if !seen.insert(key) {
                            stats.eliminated_bounds_checks += 1;
                            continue;
                        }
                    }
                    InstructionKind::VectorBoundsCheck {
                        index,
                        length,
                        lanes,
                    } => {
                        let key = (
                            canonical_value(*index, &definitions, &stack_slots, &mut expressions),
                            canonical_value(*length, &definitions, &stack_slots, &mut expressions),
                            *lanes,
                        );
                        if !vector_seen.insert(key) {
                            stats.eliminated_bounds_checks += 1;
                            continue;
                        }
                    }
                    _ => {}
                }
                retained.push(instruction);
            }
            block.instructions = retained;
        }
    }
    stats
}

/// Reuses identical pointer-offset expressions within each basic block.
/// Offset arithmetic has no memory effect, so stores do not invalidate this
/// local CSE; replacements are applied to later blocks only after dominance
/// has been established by the original defining block.
pub fn eliminate_redundant_offsets(module: &mut Module) -> OptimizationStats {
    let mut stats = OptimizationStats::default();
    for function in &mut module.functions {
        let definitions = function
            .blocks
            .iter()
            .flat_map(|block| block.instructions.iter())
            .filter_map(|instruction| {
                instruction
                    .result
                    .map(|result| (result.value, instruction.kind.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        let stack_slots = definitions
            .iter()
            .filter_map(|(value, kind)| match kind {
                InstructionKind::StackAlloc { .. } => Some(*value),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        let mut expressions = BTreeMap::<ValueId, CanonicalValue>::new();
        let mut pointer_roots = BTreeMap::<ValueId, Option<ValueId>>::new();
        for (value, kind) in &definitions {
            if matches!(kind, InstructionKind::StackAlloc { .. }) {
                pointer_roots.insert(*value, Some(*value));
            }
        }
        let mut replacements = BTreeMap::<ValueId, ValueId>::new();
        for block in &mut function.blocks {
            let mut offsets = BTreeMap::<OffsetKey, ValueId>::new();
            let old_instructions = std::mem::take(&mut block.instructions);
            let mut retained = Vec::with_capacity(old_instructions.len());
            for mut instruction in old_instructions {
                remap_instruction_operands(&mut instruction.kind, &replacements);
                match &instruction.kind {
                    InstructionKind::Store { pointer, .. } => {
                        if pointer_root(*pointer, &definitions, &mut pointer_roots).is_some() {
                            offsets.clear();
                        }
                    }
                    InstructionKind::Call { .. } => offsets.clear(),
                    InstructionKind::Offset { base, indices }
                        if let Some(result) = instruction.result =>
                    {
                        let key = OffsetKey {
                            base: canonical_value(
                                *base,
                                &definitions,
                                &stack_slots,
                                &mut expressions,
                            ),
                            indices: indices
                                .iter()
                                .map(|index| {
                                    canonical_value(
                                        *index,
                                        &definitions,
                                        &stack_slots,
                                        &mut expressions,
                                    )
                                })
                                .collect(),
                        };
                        if let Some(existing) = offsets.get(&key).copied() {
                            replacements.insert(result.value, existing);
                            stats.removed_instructions += 1;
                            continue;
                        }
                        offsets.insert(key, result.value);
                    }
                    _ => {}
                }
                retained.push(instruction);
            }
            block.instructions = retained;
        }
        if !replacements.is_empty() {
            let resolved = replacements
                .keys()
                .copied()
                .map(|value| (value, resolve_replacement(value, &replacements)))
                .collect::<BTreeMap<_, _>>();
            for block in &mut function.blocks {
                for instruction in &mut block.instructions {
                    remap_instruction_operands(&mut instruction.kind, &resolved);
                }
                remap_terminator_operands(&mut block.terminator, &resolved);
            }
            renumber_values(function);
        }
    }
    stats
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct OffsetKey {
    base: CanonicalValue,
    indices: Vec<CanonicalValue>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum CanonicalValue {
    Value(ValueId),
    StackLoad(ValueId),
    Offset { base: Box<Self>, indices: Vec<Self> },
    Extract { aggregate: Box<Self>, index: u32 },
}

impl CanonicalValue {
    fn depends_on_stack_load(&self, pointer: ValueId) -> bool {
        match self {
            Self::StackLoad(value) => *value == pointer,
            Self::Offset { base, indices } => {
                base.depends_on_stack_load(pointer)
                    || indices
                        .iter()
                        .any(|index| index.depends_on_stack_load(pointer))
            }
            Self::Extract { aggregate, .. } => aggregate.depends_on_stack_load(pointer),
            Self::Value(_) => false,
        }
    }
}

fn stack_load_may_alias(
    value: &CanonicalValue,
    pointer: ValueId,
    function: FunctionId,
    aliases: &AliasAnalysis,
    stack_slots: &BTreeSet<ValueId>,
) -> bool {
    stack_slots.iter().any(|root| {
        value.depends_on_stack_load(*root)
            && aliases.relation(function, pointer, *root) != AliasRelation::NoAlias
    })
}

fn canonical_value(
    value: ValueId,
    definitions: &BTreeMap<ValueId, InstructionKind>,
    stack_slots: &BTreeSet<ValueId>,
    expressions: &mut BTreeMap<ValueId, CanonicalValue>,
) -> CanonicalValue {
    if let Some(expression) = expressions.get(&value) {
        return expression.clone();
    }
    let expression = match definitions.get(&value) {
        Some(InstructionKind::Load {
            pointer,
            volatile: false,
            ..
        }) if stack_slots.contains(pointer) => CanonicalValue::StackLoad(*pointer),
        Some(InstructionKind::ExtractValue { aggregate, index }) => {
            let aggregate = canonical_value(*aggregate, definitions, stack_slots, expressions);
            CanonicalValue::Extract {
                aggregate: Box::new(aggregate),
                index: *index,
            }
        }
        Some(InstructionKind::Offset { base, indices }) => CanonicalValue::Offset {
            base: Box::new(canonical_value(
                *base,
                definitions,
                stack_slots,
                expressions,
            )),
            indices: indices
                .iter()
                .map(|index| canonical_value(*index, definitions, stack_slots, expressions))
                .collect(),
        },
        _ => CanonicalValue::Value(value),
    };
    expressions.insert(value, expression.clone());
    expression
}

fn pointer_root(
    value: ValueId,
    definitions: &BTreeMap<ValueId, InstructionKind>,
    roots: &mut BTreeMap<ValueId, Option<ValueId>>,
) -> Option<ValueId> {
    if let Some(root) = roots.get(&value) {
        return *root;
    }
    let root = match definitions.get(&value) {
        Some(InstructionKind::Offset { base, .. })
        | Some(InstructionKind::Cast { value: base, .. }) => {
            pointer_root(*base, definitions, roots)
        }
        _ => None,
    };
    roots.insert(value, root);
    root
}

/// Canonicalizes natural loops with one outside preheader and performs a
/// conservative LICM pass for pure scalar instructions. No CFG edge is
/// rewritten; the pass only moves computations whose operands are defined
/// outside the loop or by an earlier hoisted computation.
pub fn canonicalize_loops_and_licm(module: &mut Module) -> OptimizationStats {
    let mut stats = OptimizationStats::default();
    for function in &mut module.functions {
        let stack_slots = function
            .blocks
            .iter()
            .flat_map(|block| block.instructions.iter())
            .filter_map(
                |instruction| match (&instruction.result, &instruction.kind) {
                    (Some(result), InstructionKind::StackAlloc { .. }) => Some(result.value),
                    _ => None,
                },
            )
            .collect::<BTreeSet<_>>();
        let loops = discover_loops(function);
        for (header, preheader, loop_blocks) in loops {
            let invariant_stack_slots = stack_slots
                .iter()
                .copied()
                .filter(|pointer| !loop_has_memory_write(function, &loop_blocks, *pointer))
                .collect::<BTreeSet<_>>();
            let loop_values = loop_blocks
                .iter()
                .flat_map(|block| {
                    let block = &function.blocks[block.index()];
                    block
                        .parameters
                        .iter()
                        .map(|parameter| parameter.value)
                        .chain(block.instructions.iter().filter_map(|instruction| {
                            instruction.result.map(|result| result.value)
                        }))
                        .collect::<Vec<_>>()
                })
                .collect::<BTreeSet<_>>();
            let mut hoisted_values = BTreeSet::new();
            let mut hoisted = Vec::new();
            for block_id in &loop_blocks {
                let block = &mut function.blocks[block_id.index()];
                let mut retained = Vec::with_capacity(block.instructions.len());
                for instruction in std::mem::take(&mut block.instructions) {
                    let Some(result) = instruction.result else {
                        retained.push(instruction);
                        continue;
                    };
                    let operands = instruction_operands(&instruction.kind);
                    let invariant_load = matches!(
                        &instruction.kind,
                        InstructionKind::Load {
                            pointer,
                            volatile: false,
                            ..
                        } if invariant_stack_slots.contains(pointer)
                    );
                    let invariant = (is_pure_scalar(&instruction) || invariant_load)
                        && operands.iter().all(|operand| {
                            !loop_values.contains(operand) || hoisted_values.contains(operand)
                        });
                    if invariant {
                        hoisted_values.insert(result.value);
                        hoisted.push(instruction);
                        stats.hoisted_loop_instructions += 1;
                    } else {
                        retained.push(instruction);
                    }
                }
                block.instructions = retained;
            }
            if !hoisted.is_empty() {
                function.blocks[preheader.index()]
                    .instructions
                    .extend(hoisted);
            }
            if header != preheader {
                stats.canonicalized_loops += 1;
            }
        }
        if stats.hoisted_loop_instructions != 0 {
            renumber_values(function);
        }
    }
    stats
}

fn loop_has_memory_write(function: &Function, loop_blocks: &[BlockId], pointer: ValueId) -> bool {
    loop_blocks.iter().any(|block_id| {
        function.blocks[block_id.index()]
            .instructions
            .iter()
            .any(|instruction| match &instruction.kind {
                InstructionKind::Store {
                    pointer: store_pointer,
                    ..
                } => *store_pointer == pointer,
                InstructionKind::Call { .. } => true,
                _ => false,
            })
    })
}

fn discover_loops(function: &Function) -> Vec<(BlockId, BlockId, Vec<BlockId>)> {
    if function.blocks.len() < 2 {
        return Vec::new();
    }
    let successors: Vec<Vec<BlockId>> = function
        .blocks
        .iter()
        .map(|block| terminator_targets(&block.terminator))
        .collect();
    let mut predecessors = vec![Vec::<BlockId>::new(); function.blocks.len()];
    for (source, targets) in successors.iter().enumerate() {
        for target in targets {
            if target.index() < function.blocks.len() {
                predecessors[target.index()].push(BlockId::new(source));
            }
        }
    }
    let dominators = compute_dominators(&predecessors);
    let mut loops = Vec::new();
    let mut seen_headers = BTreeSet::new();
    for (source, targets) in successors.iter().enumerate() {
        for header in targets {
            if !dominators[source].contains(&header.index()) || !seen_headers.insert(*header) {
                continue;
            }
            let mut nodes = BTreeSet::from([header.index(), source]);
            let mut pending = vec![source];
            while let Some(node) = pending.pop() {
                for predecessor in &predecessors[node] {
                    if nodes.insert(predecessor.index()) && predecessor.index() != header.index() {
                        pending.push(predecessor.index());
                    }
                }
            }
            let outside: Vec<_> = predecessors[header.index()]
                .iter()
                .copied()
                .filter(|predecessor| !nodes.contains(&predecessor.index()))
                .collect();
            if outside.len() != 1 {
                continue;
            }
            loops.push((
                *header,
                outside[0],
                nodes.into_iter().map(BlockId::new).collect(),
            ));
        }
    }
    loops.sort_by_key(|(header, preheader, _)| (header.index(), preheader.index()));
    loops
}

fn compute_dominators(predecessors: &[Vec<BlockId>]) -> Vec<BTreeSet<usize>> {
    let all: BTreeSet<_> = (0..predecessors.len()).collect();
    let mut dominators = vec![all.clone(); predecessors.len()];
    if let Some(entry) = dominators.first_mut() {
        entry.clear();
        entry.insert(0);
    }
    let mut changed = true;
    while changed {
        changed = false;
        for block in 1..predecessors.len() {
            let mut next = all.clone();
            if predecessors[block].is_empty() {
                next.clear();
            } else {
                for predecessor in &predecessors[block] {
                    next = next
                        .intersection(&dominators[predecessor.index()])
                        .copied()
                        .collect();
                }
            }
            next.insert(block);
            if next != dominators[block] {
                dominators[block] = next;
                changed = true;
            }
        }
    }
    dominators
}

#[derive(Clone, Debug)]
struct StackPromotion {
    removed_indices: BTreeMap<(usize, usize), ()>,
    replacements: BTreeMap<ValueId, ValueId>,
}

fn promote_function_stack_slots(
    function: &mut Function,
    types: &[Type],
    stats: &mut OptimizationStats,
) {
    if function.blocks.is_empty() {
        return;
    }
    let mut candidates = BTreeMap::<ValueId, TypeId>::new();
    for block in &function.blocks {
        for instruction in &block.instructions {
            let InstructionKind::StackAlloc { ty, count: None } = instruction.kind else {
                continue;
            };
            let Some(result) = instruction.result else {
                continue;
            };
            if is_stack_promotable_pointer(types, result.ty, ty) {
                candidates.insert(result.value, ty);
            }
        }
    }
    if candidates.is_empty() {
        return;
    }

    let mut predecessors = vec![Vec::<BlockId>::new(); function.blocks.len()];
    for (source, block) in function.blocks.iter().enumerate() {
        for target in terminator_targets(&block.terminator) {
            if target.index() < predecessors.len() {
                predecessors[target.index()].push(BlockId::new(source));
            }
        }
    }
    let dominators = compute_dominators(&predecessors);

    // First reject pointer escapes. Direct non-volatile loads/stores are the
    // only uses that the local dataflow below is allowed to rewrite.
    let mut valid = candidates
        .keys()
        .copied()
        .map(|pointer| (pointer, true))
        .collect::<BTreeMap<_, _>>();
    for block in &function.blocks {
        for instruction in &block.instructions {
            for operand in instruction_operands(&instruction.kind) {
                let Some(is_valid) = valid.get_mut(&operand) else {
                    continue;
                };
                let direct = match &instruction.kind {
                    InstructionKind::Store {
                        pointer,
                        volatile: false,
                        ..
                    }
                    | InstructionKind::Load {
                        pointer,
                        volatile: false,
                        ..
                    } => *pointer == operand,
                    _ => false,
                };
                if !direct {
                    *is_valid = false;
                }
            }
        }
        for operand in terminator_operands(&block.terminator) {
            if let Some(is_valid) = valid.get_mut(&operand) {
                *is_valid = false;
            }
        }
    }

    let mut promotions = Vec::new();
    for pointer in candidates.keys().copied() {
        if !valid.get(&pointer).copied().unwrap_or(false) {
            continue;
        }

        // A caller-owned aggregate descriptor is commonly staged in the
        // entry block and loaded from a loop block.  When there is exactly one
        // non-volatile store, dominance proves that the stored SSA value is
        // available for every load; promote that immutable staging across the
        // CFG instead of forcing repeated stack traffic. Multiple stores or
        // malformed CFGs stay on the conservative local proof below.
        let mut stores = Vec::<(usize, usize, ValueId)>::new();
        let mut loads = Vec::<(usize, usize, ValueId)>::new();
        let mut stack_alloc = None;
        for (block_index, block) in function.blocks.iter().enumerate() {
            for (instruction_index, instruction) in block.instructions.iter().enumerate() {
                match &instruction.kind {
                    InstructionKind::StackAlloc { .. }
                        if instruction
                            .result
                            .is_some_and(|result| result.value == pointer) =>
                    {
                        stack_alloc = Some((block_index, instruction_index));
                    }
                    InstructionKind::Store {
                        pointer: store_pointer,
                        value,
                        volatile: false,
                        ..
                    } if *store_pointer == pointer => {
                        stores.push((block_index, instruction_index, *value));
                    }
                    InstructionKind::Load {
                        pointer: load_pointer,
                        ..
                    } if *load_pointer == pointer => {
                        if let Some(result) = instruction.result {
                            loads.push((block_index, instruction_index, result.value));
                        }
                    }
                    _ => {}
                }
            }
        }
        let Some(stack_alloc) = stack_alloc else {
            continue;
        };
        if stores.len() == 1
            && loads.iter().all(|(load_block, load_index, _)| {
                let (store_block, store_index, _) = stores[0];
                dominators
                    .get(*load_block)
                    .is_some_and(|set| set.contains(&store_block))
                    && (store_block != *load_block || store_index < *load_index)
            })
            && dominators
                .get(stores[0].0)
                .is_some_and(|set| set.contains(&stack_alloc.0))
        {
            let (store_block, store_index, stored_value) = stores[0];
            let mut replacements = BTreeMap::new();
            let mut removed_indices = BTreeMap::new();
            removed_indices.insert(stack_alloc, ());
            removed_indices.insert((store_block, store_index), ());
            for (block, index, result) in loads {
                replacements.insert(result, stored_value);
                removed_indices.insert((block, index), ());
            }
            promotions.push(StackPromotion {
                removed_indices,
                replacements,
            });
            continue;
        }

        let mut replacements = BTreeMap::new();
        let mut removed_indices = BTreeMap::new();
        let mut has_store = false;
        let mut candidate_valid = true;
        for (block_index, block) in function.blocks.iter().enumerate() {
            // A proof is local to each basic block. A load in a different
            // block must not borrow a store from an unproven predecessor.
            let mut current = None;
            for (instruction_index, instruction) in block.instructions.iter().enumerate() {
                match &instruction.kind {
                    InstructionKind::StackAlloc { .. }
                        if instruction
                            .result
                            .is_some_and(|result| result.value == pointer) =>
                    {
                        removed_indices.insert((block_index, instruction_index), ());
                    }
                    InstructionKind::Store {
                        pointer: store_pointer,
                        value,
                        volatile: false,
                        ..
                    } if *store_pointer == pointer => {
                        current = Some(resolve_replacement(*value, &replacements));
                        has_store = true;
                        removed_indices.insert((block_index, instruction_index), ());
                    }
                    InstructionKind::Load {
                        pointer: load_pointer,
                        ..
                    } if *load_pointer == pointer => {
                        let Some(current) = current else {
                            candidate_valid = false;
                            break;
                        };
                        let Some(result) = instruction.result else {
                            candidate_valid = false;
                            break;
                        };
                        replacements.insert(result.value, current);
                        removed_indices.insert((block_index, instruction_index), ());
                    }
                    _ => {}
                }
            }
            if !candidate_valid {
                break;
            }
        }
        if candidate_valid && has_store {
            promotions.push(StackPromotion {
                removed_indices,
                replacements,
            });
        }
    }
    if promotions.is_empty() {
        return;
    }

    let mut removed_indices = BTreeMap::new();
    let mut replacements = BTreeMap::new();
    for promotion in promotions {
        removed_indices.extend(promotion.removed_indices);
        replacements.extend(promotion.replacements);
        stats.promoted_stack_slots += 1;
    }
    let resolved_replacements = replacements
        .keys()
        .copied()
        .map(|value| (value, resolve_replacement(value, &replacements)))
        .collect::<BTreeMap<_, _>>();
    for (block_index, block) in function.blocks.iter_mut().enumerate() {
        for instruction in &mut block.instructions {
            remap_instruction_operands(&mut instruction.kind, &resolved_replacements);
        }
        remap_terminator_operands(&mut block.terminator, &resolved_replacements);
        let mut instruction_index = 0;
        block.instructions.retain(|_| {
            let keep = !removed_indices.contains_key(&(block_index, instruction_index));
            instruction_index += 1;
            keep
        });
    }
    renumber_values(function);
}

fn is_stack_promotable_pointer(types: &[Type], pointer_ty: TypeId, pointee: TypeId) -> bool {
    let Some(Type::Pointer {
        pointee: actual,
        address_space: AddressSpace::Stack,
    }) = types.get(pointer_ty.index())
    else {
        return false;
    };
    *actual == pointee
        && matches!(
            types.get(pointee.index()),
            Some(
                Type::Bool
                    | Type::Integer { .. }
                    | Type::Float { .. }
                    | Type::Pointer { .. }
                    | Type::Vector { .. }
                    | Type::Struct { .. }
                    | Type::NominalStruct { .. },
            )
        )
}

fn resolve_replacement(mut value: ValueId, replacements: &BTreeMap<ValueId, ValueId>) -> ValueId {
    let mut guard = 0;
    while let Some(next) = replacements.get(&value).copied() {
        value = next;
        guard += 1;
        if guard > replacements.len() {
            break;
        }
    }
    value
}

fn is_tiny_inline_candidate(function: &Function) -> bool {
    if function.linkage != crate::Linkage::Internal
        || function.blocks.len() != 1
        || !function.blocks[0].parameters.is_empty()
    {
        return false;
    }
    if !matches!(function.blocks[0].terminator, Terminator::Return { .. }) {
        return false;
    }
    if function.blocks[0].instructions.len() > INLINE_MAX_INSTRUCTIONS {
        return false;
    }
    function.blocks[0].instructions.iter().all(is_pure_scalar)
}

fn inline_call(
    call_result: Option<TypedValue>,
    arguments: &[ValueId],
    callee: &Function,
    next_value: &mut usize,
) -> Option<Vec<Instruction>> {
    if arguments.len() != callee.parameters.len() {
        return None;
    }
    let return_value = match callee.blocks[0].terminator {
        Terminator::Return { value } => value,
        _ => return None,
    };
    if return_value.is_some_and(|value| {
        callee
            .parameters
            .iter()
            .any(|parameter| parameter.value == value)
    }) {
        return None;
    }
    if return_value.is_some() != call_result.is_some() {
        return None;
    }

    let mut values = BTreeMap::<ValueId, ValueId>::new();
    for (parameter, argument) in callee.parameters.iter().zip(arguments) {
        values.insert(parameter.value, *argument);
    }
    let mut inlined = Vec::with_capacity(callee.blocks[0].instructions.len());
    for instruction in &callee.blocks[0].instructions {
        let mut cloned = instruction.clone();
        remap_instruction_operands(&mut cloned.kind, &values);
        if let Some(result) = &mut cloned.result {
            let mapped = if Some(result.value) == return_value {
                call_result?.value
            } else {
                let fresh = ValueId::new(*next_value);
                *next_value += 1;
                fresh
            };
            values.insert(result.value, mapped);
            result.value = mapped;
        }
        inlined.push(cloned);
    }
    Some(inlined)
}

fn next_value_id(function: &Function) -> usize {
    let mut next = function
        .parameters
        .iter()
        .map(|parameter| parameter.value.index() + 1)
        .chain(function.blocks.iter().flat_map(|block| {
            block
                .parameters
                .iter()
                .map(|parameter| parameter.value.index() + 1)
        }))
        .max()
        .unwrap_or(0);
    for block in &function.blocks {
        for instruction in &block.instructions {
            if let Some(result) = instruction.result {
                next = next.max(result.value.index() + 1);
            }
        }
    }
    next
}

/// Simplifies constant branch/switch terminators and removes dead pure SSA
/// instructions. Unreachable blocks are pruned with a deterministic block-id
/// remap before value IDs are compacted, so the JIR verifier still sees dense
/// identities and valid dominance.
pub fn simplify_cfg_and_dce(module: &mut Module) -> OptimizationStats {
    let mut stats = OptimizationStats::default();
    for function in &mut module.functions {
        simplify_constant_terminators(function, &mut stats);
        prune_unreachable_blocks(function);
        eliminate_dead_instructions(function, &mut stats);
    }
    stats
}

fn prune_unreachable_blocks(function: &mut Function) {
    if function.blocks.is_empty() {
        return;
    }
    let mut reachable = BTreeMap::<usize, bool>::new();
    let mut pending = vec![0usize];
    while let Some(index) = pending.pop() {
        if reachable.insert(index, true).is_some() {
            continue;
        }
        let Some(block) = function.blocks.get(index) else {
            continue;
        };
        for target in terminator_targets(&block.terminator) {
            pending.push(target.index());
        }
    }
    if reachable.len() == function.blocks.len() {
        return;
    }

    let mut block_remap = BTreeMap::<usize, BlockId>::new();
    let mut next = 0;
    for index in 0..function.blocks.len() {
        if reachable.contains_key(&index) {
            block_remap.insert(index, BlockId::new(next));
            next += 1;
        }
    }
    let old_blocks = std::mem::take(&mut function.blocks);
    let mut blocks = Vec::with_capacity(block_remap.len());
    for (index, mut block) in old_blocks.into_iter().enumerate() {
        let Some(new_id) = block_remap.get(&index).copied() else {
            continue;
        };
        block.id = new_id;
        remap_terminator_targets(&mut block.terminator, &block_remap);
        blocks.push(block);
    }
    function.blocks = blocks;
}

fn terminator_targets(terminator: &Terminator) -> Vec<crate::BlockId> {
    match terminator {
        Terminator::Return { .. } | Terminator::Unreachable => Vec::new(),
        Terminator::Jump { target, .. } => vec![*target],
        Terminator::Branch {
            then_target,
            else_target,
            ..
        } => vec![*then_target, *else_target],
        Terminator::Switch { cases, default, .. } => cases
            .iter()
            .map(|case| case.target)
            .chain(std::iter::once(*default))
            .collect(),
    }
}

fn remap_terminator_targets(terminator: &mut Terminator, blocks: &BTreeMap<usize, crate::BlockId>) {
    let remap = |target: &mut crate::BlockId| {
        if let Some(mapped) = blocks.get(&target.index()) {
            *target = *mapped;
        }
    };
    match terminator {
        Terminator::Return { .. } | Terminator::Unreachable => {}
        Terminator::Jump { target, .. } => remap(target),
        Terminator::Branch {
            then_target,
            else_target,
            ..
        } => {
            remap(then_target);
            remap(else_target);
        }
        Terminator::Switch { cases, default, .. } => {
            for case in cases {
                remap(&mut case.target);
            }
            remap(default);
        }
    }
}

fn simplify_constant_terminators(function: &mut Function, stats: &mut OptimizationStats) {
    let constants: BTreeMap<_, _> = function
        .blocks
        .iter()
        .flat_map(|block| block.instructions.iter())
        .filter_map(
            |instruction| match (&instruction.result, &instruction.kind) {
                (Some(result), InstructionKind::Constant(constant)) => {
                    Some((result.value, constant.clone()))
                }
                _ => None,
            },
        )
        .collect();

    for block in &mut function.blocks {
        let replacement = match &block.terminator {
            Terminator::Branch {
                condition,
                then_target,
                then_arguments,
                else_target,
                else_arguments,
            } => match constants.get(condition) {
                Some(Constant::Bool(true)) => Some(Terminator::Jump {
                    target: *then_target,
                    arguments: then_arguments.clone(),
                }),
                Some(Constant::Bool(false)) => Some(Terminator::Jump {
                    target: *else_target,
                    arguments: else_arguments.clone(),
                }),
                _ => None,
            },
            Terminator::Switch {
                discriminant,
                cases,
                default,
                default_arguments,
            } => match constants.get(discriminant) {
                Some(Constant::Integer { value }) => cases
                    .iter()
                    .find(|case| case.value == *value)
                    .map(|case| Terminator::Jump {
                        target: case.target,
                        arguments: case.arguments.clone(),
                    })
                    .or_else(|| {
                        Some(Terminator::Jump {
                            target: *default,
                            arguments: default_arguments.clone(),
                        })
                    }),
                _ => None,
            },
            _ => None,
        };
        if let Some(replacement) = replacement {
            block.terminator = replacement;
            stats.simplified_terminators += 1;
        }
    }
}

fn eliminate_dead_instructions(function: &mut Function, stats: &mut OptimizationStats) {
    loop {
        let mut use_counts = BTreeMap::<ValueId, usize>::new();
        for block in &function.blocks {
            for instruction in &block.instructions {
                for operand in instruction_operands(&instruction.kind) {
                    *use_counts.entry(operand).or_default() += 1;
                }
            }
            for operand in terminator_operands(&block.terminator) {
                *use_counts.entry(operand).or_default() += 1;
            }
        }

        let mut removed = false;
        for block in &mut function.blocks {
            let mut retained = Vec::with_capacity(block.instructions.len());
            for instruction in block.instructions.drain(..) {
                let is_dead = instruction
                    .result
                    .is_some_and(|result| use_counts.get(&result.value).copied().unwrap_or(0) == 0);
                if is_dead && is_pure_scalar(&instruction) {
                    stats.removed_instructions += 1;
                    removed = true;
                } else {
                    retained.push(instruction);
                }
            }
            block.instructions = retained;
        }
        if !removed {
            break;
        }
    }
    if stats.removed_instructions != 0 {
        renumber_values(function);
    }
}

fn renumber_values(function: &mut Function) {
    let mut remap = BTreeMap::<ValueId, ValueId>::new();
    let mut next = 0;
    for parameter in &mut function.parameters {
        let old = parameter.value;
        let new = ValueId::new(next);
        next += 1;
        remap.insert(old, new);
        parameter.value = new;
    }
    for block in &mut function.blocks {
        for parameter in &mut block.parameters {
            let old = parameter.value;
            let new = ValueId::new(next);
            next += 1;
            remap.insert(old, new);
            parameter.value = new;
        }
    }
    for block in &mut function.blocks {
        for instruction in &mut block.instructions {
            if let Some(result) = &mut instruction.result {
                let old = result.value;
                let new = ValueId::new(next);
                next += 1;
                remap.insert(old, new);
                result.value = new;
            }
        }
    }
    for block in &mut function.blocks {
        for instruction in &mut block.instructions {
            remap_instruction_operands(&mut instruction.kind, &remap);
        }
        remap_terminator_operands(&mut block.terminator, &remap);
    }
}

fn remap(value: &mut ValueId, values: &BTreeMap<ValueId, ValueId>) {
    if let Some(mapped) = values.get(value) {
        *value = *mapped;
    }
}

fn remap_instruction_operands(kind: &mut InstructionKind, values: &BTreeMap<ValueId, ValueId>) {
    match kind {
        InstructionKind::Constant(_)
        | InstructionKind::Builtin(_)
        | InstructionKind::StringLiteral { .. } => {}
        InstructionKind::Aggregate { elements } => {
            for value in elements {
                remap(value, values);
            }
        }
        InstructionKind::ExtractValue { aggregate, .. } => remap(aggregate, values),
        InstructionKind::ExtractElement { aggregate, index } => {
            remap(aggregate, values);
            remap(index, values);
        }
        InstructionKind::EnumConstruct { fields, .. } => {
            for value in fields {
                remap(value, values);
            }
        }
        InstructionKind::EnumTag { value }
        | InstructionKind::EnumExtract { value, .. }
        | InstructionKind::Unary { operand: value, .. } => remap(value, values),
        InstructionKind::Binary { left, right, .. }
        | InstructionKind::Compare { left, right, .. } => {
            remap(left, values);
            remap(right, values);
        }
        InstructionKind::Cast { value, .. } => remap(value, values),
        InstructionKind::Select {
            condition,
            when_true,
            when_false,
        } => {
            remap(condition, values);
            remap(when_true, values);
            remap(when_false, values);
        }
        InstructionKind::StackAlloc { count, .. } => {
            if let Some(count) = count {
                remap(count, values);
            }
        }
        InstructionKind::RegionAlloc { region, count, .. } => {
            remap(region, values);
            remap(count, values);
        }
        InstructionKind::RegionCreate => {}
        InstructionKind::RegionDestroy { region } | InstructionKind::Drop { value: region } => {
            remap(region, values)
        }
        InstructionKind::Load { pointer, .. } => remap(pointer, values),
        InstructionKind::Store { pointer, value, .. } => {
            remap(pointer, values);
            remap(value, values);
        }
        InstructionKind::Offset { base, indices } => {
            remap(base, values);
            for index in indices {
                remap(index, values);
            }
        }
        InstructionKind::BoundsCheck { index, length } => {
            remap(index, values);
            remap(length, values);
        }
        InstructionKind::VectorBoundsCheck { index, length, .. } => {
            remap(index, values);
            remap(length, values);
        }
        InstructionKind::AssumeNoAlias { left, right } => {
            remap(left, values);
            remap(right, values);
        }
        InstructionKind::Call { arguments, .. } => {
            for argument in arguments {
                remap(argument, values);
            }
        }
        InstructionKind::FunctionAddress { .. } => {}
        InstructionKind::IndirectCall { callee, arguments } => {
            remap(callee, values);
            for argument in arguments {
                remap(argument, values);
            }
        }
        InstructionKind::VectorSplat { value, .. } => remap(value, values),
        InstructionKind::VectorBinary { left, right, .. } => {
            remap(left, values);
            remap(right, values);
        }
        InstructionKind::VectorExtract { vector, lane } => {
            remap(vector, values);
            remap(lane, values);
        }
        InstructionKind::VectorInsert {
            vector,
            lane,
            value,
        } => {
            remap(vector, values);
            remap(lane, values);
            remap(value, values);
        }
    }
}

fn remap_terminator_operands(terminator: &mut Terminator, values: &BTreeMap<ValueId, ValueId>) {
    match terminator {
        Terminator::Return { value } => {
            if let Some(value) = value {
                remap(value, values);
            }
        }
        Terminator::Jump { arguments, .. } => {
            for argument in arguments {
                remap(argument, values);
            }
        }
        Terminator::Branch {
            condition,
            then_arguments,
            else_arguments,
            ..
        } => {
            remap(condition, values);
            for argument in then_arguments.iter_mut().chain(else_arguments.iter_mut()) {
                remap(argument, values);
            }
        }
        Terminator::Switch {
            discriminant,
            cases,
            default_arguments,
            ..
        } => {
            remap(discriminant, values);
            for argument in cases
                .iter_mut()
                .flat_map(|case| case.arguments.iter_mut())
                .chain(default_arguments.iter_mut())
            {
                remap(argument, values);
            }
        }
        Terminator::Unreachable => {}
    }
}

fn is_pure_scalar(instruction: &Instruction) -> bool {
    matches!(
        &instruction.kind,
        InstructionKind::Constant(_)
            | InstructionKind::FunctionAddress { .. }
            | InstructionKind::Unary { .. }
            | InstructionKind::Binary { .. }
            | InstructionKind::Compare { .. }
            | InstructionKind::Cast { .. }
            | InstructionKind::Select { .. }
            | InstructionKind::VectorSplat { .. }
            | InstructionKind::VectorBinary { .. }
            | InstructionKind::VectorExtract { .. }
            | InstructionKind::VectorInsert { .. }
    )
}

fn instruction_operands(kind: &InstructionKind) -> Vec<ValueId> {
    match kind {
        InstructionKind::Constant(_)
        | InstructionKind::Builtin(_)
        | InstructionKind::StringLiteral { .. } => Vec::new(),
        InstructionKind::Aggregate { elements } => elements.clone(),
        InstructionKind::ExtractValue { aggregate, .. } => vec![*aggregate],
        InstructionKind::ExtractElement { aggregate, index } => vec![*aggregate, *index],
        InstructionKind::EnumConstruct { fields, .. } => fields.clone(),
        InstructionKind::EnumTag { value }
        | InstructionKind::EnumExtract { value, .. }
        | InstructionKind::Unary { operand: value, .. } => vec![*value],
        InstructionKind::Binary { left, right, .. }
        | InstructionKind::Compare { left, right, .. } => vec![*left, *right],
        InstructionKind::Cast { value, .. } => vec![*value],
        InstructionKind::Select {
            condition,
            when_true,
            when_false,
        } => vec![*condition, *when_true, *when_false],
        InstructionKind::StackAlloc { count, .. } => count.iter().copied().collect(),
        InstructionKind::RegionAlloc { region, count, .. } => vec![*region, *count],
        InstructionKind::RegionCreate => Vec::new(),
        InstructionKind::RegionDestroy { region } | InstructionKind::Drop { value: region } => {
            vec![*region]
        }
        InstructionKind::Load { pointer, .. } => vec![*pointer],
        InstructionKind::Store { pointer, value, .. } => vec![*pointer, *value],
        InstructionKind::Offset { base, indices } => std::iter::once(*base)
            .chain(indices.iter().copied())
            .collect(),
        InstructionKind::BoundsCheck { index, length }
        | InstructionKind::VectorBoundsCheck { index, length, .. } => vec![*index, *length],
        InstructionKind::AssumeNoAlias { left, right } => vec![*left, *right],
        InstructionKind::Call { arguments, .. } => arguments.clone(),
        InstructionKind::FunctionAddress { .. } => Vec::new(),
        InstructionKind::IndirectCall { callee, arguments } => std::iter::once(*callee)
            .chain(arguments.iter().copied())
            .collect(),
        InstructionKind::VectorSplat { value, .. } => vec![*value],
        InstructionKind::VectorBinary { left, right, .. } => vec![*left, *right],
        InstructionKind::VectorExtract { vector, lane } => vec![*vector, *lane],
        InstructionKind::VectorInsert {
            vector,
            lane,
            value,
        } => vec![*vector, *lane, *value],
    }
}

fn terminator_operands(terminator: &Terminator) -> Vec<ValueId> {
    match terminator {
        Terminator::Return { value } => value.iter().copied().collect(),
        Terminator::Jump { arguments, .. } => arguments.clone(),
        Terminator::Branch {
            condition,
            then_arguments,
            else_arguments,
            ..
        } => std::iter::once(*condition)
            .chain(then_arguments.iter().copied())
            .chain(else_arguments.iter().copied())
            .collect(),
        Terminator::Switch {
            discriminant,
            cases,
            default_arguments,
            ..
        } => std::iter::once(*discriminant)
            .chain(cases.iter().flat_map(|case| case.arguments.iter().copied()))
            .chain(default_arguments.iter().copied())
            .collect(),
        Terminator::Unreachable => Vec::new(),
    }
}

#[derive(Clone, Debug)]
struct KnownConstant {
    value: Constant,
    ty: TypeId,
}

/// Folds pure scalar instructions whose operands are known constants.
///
/// Value identities and block structure are intentionally retained. This makes
/// the pass safe to run before the verifier and keeps debug/source mappings
/// stable while Release lowering gets smaller scalar expressions.
pub fn fold_constants(module: &mut Module) -> OptimizationStats {
    let types = &module.types;
    let mut stats = OptimizationStats::default();
    for function in &mut module.functions {
        fold_function(function, types, &mut stats);
    }
    stats
}

fn fold_function(function: &mut Function, types: &[Type], stats: &mut OptimizationStats) {
    let mut known = BTreeMap::<ValueId, KnownConstant>::new();
    for block in &mut function.blocks {
        for instruction in &mut block.instructions {
            let Some(result) = instruction.result else {
                continue;
            };
            if let InstructionKind::Constant(constant) = &instruction.kind {
                known.insert(
                    result.value,
                    KnownConstant {
                        value: constant.clone(),
                        ty: result.ty,
                    },
                );
                continue;
            }
            let Some(constant) = fold_instruction(&instruction.kind, result, types, &known) else {
                continue;
            };
            instruction.kind = InstructionKind::Constant(constant.clone());
            known.insert(
                result.value,
                KnownConstant {
                    value: constant,
                    ty: result.ty,
                },
            );
            stats.folded_instructions += 1;
        }
    }
}

fn fold_instruction(
    kind: &InstructionKind,
    result: TypedValue,
    types: &[Type],
    known: &BTreeMap<ValueId, KnownConstant>,
) -> Option<Constant> {
    match kind {
        InstructionKind::Unary { op, operand } => {
            fold_unary(*op, known.get(operand), result.ty, types)
        }
        InstructionKind::Binary { op, left, right } => {
            fold_binary(*op, known.get(left), known.get(right), result.ty, types)
        }
        InstructionKind::Compare {
            predicate,
            left,
            right,
        } => fold_compare(
            *predicate,
            known.get(left),
            known.get(right),
            result.ty,
            types,
        ),
        InstructionKind::Cast { op, value, target } => {
            fold_cast(*op, known.get(value), *target, types)
        }
        InstructionKind::Select {
            condition,
            when_true,
            when_false,
        } => match known.get(condition) {
            Some(KnownConstant {
                value: Constant::Bool(true),
                ..
            }) => known.get(when_true).map(|known| known.value.clone()),
            Some(KnownConstant {
                value: Constant::Bool(false),
                ..
            }) => known.get(when_false).map(|known| known.value.clone()),
            _ => None,
        },
        _ => None,
    }
}

fn fold_unary(
    op: UnaryOp,
    operand: Option<&KnownConstant>,
    result_ty: TypeId,
    types: &[Type],
) -> Option<Constant> {
    let operand = operand?;
    match (op, &operand.value) {
        (UnaryOp::Negate, Constant::Integer { value }) => {
            let (_, bits) = integer_shape(types, result_ty)?;
            let raw = integer_raw(*value, bits);
            Some(Constant::Integer {
                value: normalize_integer(
                    raw.wrapping_neg(),
                    bits,
                    integer_shape(types, result_ty)?.0,
                ),
            })
        }
        (UnaryOp::Negate, Constant::FloatBits { bits }) => {
            fold_float_unary(*bits, result_ty, types, |value| -value)
        }
        (UnaryOp::Not, Constant::Bool(value)) => Some(Constant::Bool(!value)),
        (UnaryOp::Not | UnaryOp::BitNot, Constant::Integer { value }) => {
            let (signed, bits) = integer_shape(types, result_ty)?;
            Some(Constant::Integer {
                value: normalize_integer(!integer_raw(*value, bits), bits, signed),
            })
        }
        _ => None,
    }
}

fn fold_binary(
    op: BinaryOp,
    left: Option<&KnownConstant>,
    right: Option<&KnownConstant>,
    result_ty: TypeId,
    types: &[Type],
) -> Option<Constant> {
    let (left, right) = (left?, right?);
    match (&left.value, &right.value) {
        (Constant::Bool(left), Constant::Bool(right)) => match op {
            BinaryOp::BitAnd => Some(Constant::Bool(*left && *right)),
            BinaryOp::BitOr => Some(Constant::Bool(*left || *right)),
            BinaryOp::BitXor => Some(Constant::Bool(*left ^ *right)),
            _ => None,
        },
        (Constant::Integer { value: left }, Constant::Integer { value: right }) => {
            let (signed, bits) = integer_shape(types, result_ty)?;
            let left_raw = integer_raw(*left, bits);
            let right_raw = integer_raw(*right, bits);
            let raw = match op {
                BinaryOp::Add => left_raw.wrapping_add(right_raw),
                BinaryOp::Subtract => left_raw.wrapping_sub(right_raw),
                BinaryOp::Multiply => left_raw.wrapping_mul(right_raw),
                BinaryOp::Divide => {
                    if right_raw == 0 {
                        return None;
                    }
                    if signed {
                        let left = normalize_integer(left_raw, bits, true);
                        let right = normalize_integer(right_raw, bits, true);
                        if left == i128::MIN && right == -1 {
                            return None;
                        }
                        (left / right) as u128
                    } else {
                        left_raw / right_raw
                    }
                }
                BinaryOp::Remainder => {
                    if right_raw == 0 {
                        return None;
                    }
                    if signed {
                        let left = normalize_integer(left_raw, bits, true);
                        let right = normalize_integer(right_raw, bits, true);
                        if left == i128::MIN && right == -1 {
                            return None;
                        }
                        (left % right) as u128
                    } else {
                        left_raw % right_raw
                    }
                }
                BinaryOp::BitAnd => left_raw & right_raw,
                BinaryOp::BitOr => left_raw | right_raw,
                BinaryOp::BitXor => left_raw ^ right_raw,
                BinaryOp::ShiftLeft => {
                    let shift = u32::try_from(right_raw).ok()?;
                    if shift >= u32::from(bits) {
                        return None;
                    }
                    left_raw << shift
                }
                BinaryOp::ShiftRight => {
                    let shift = u32::try_from(right_raw).ok()?;
                    if shift >= u32::from(bits) {
                        return None;
                    }
                    if signed {
                        (normalize_integer(left_raw, bits, true) >> shift) as u128
                    } else {
                        left_raw >> shift
                    }
                }
            };
            Some(Constant::Integer {
                value: normalize_integer(raw, bits, signed),
            })
        }
        (Constant::FloatBits { bits: left }, Constant::FloatBits { bits: right }) => {
            fold_float_binary(op, *left, *right, result_ty, types)
        }
        _ => None,
    }
}

fn fold_compare(
    predicate: ComparePredicate,
    left: Option<&KnownConstant>,
    right: Option<&KnownConstant>,
    _result_ty: TypeId,
    types: &[Type],
) -> Option<Constant> {
    let (left, right) = (left?, right?);
    if left.ty != right.ty {
        return None;
    }
    match (&left.value, &right.value) {
        (Constant::Bool(left), Constant::Bool(right)) => Some(Constant::Bool(match predicate {
            ComparePredicate::Equal => left == right,
            ComparePredicate::NotEqual => left != right,
            _ => return None,
        })),
        (Constant::Integer { value: left_value }, Constant::Integer { value: right_value }) => {
            let (signed, bits) = integer_shape(types, left.ty)?;
            let left = integer_raw(*left_value, bits);
            let right = integer_raw(*right_value, bits);
            let result = match predicate {
                ComparePredicate::Equal => left == right,
                ComparePredicate::NotEqual => left != right,
                ComparePredicate::Less => {
                    compare_integer(left, right, bits, signed, ComparePredicate::Less)
                }
                ComparePredicate::LessEqual => {
                    compare_integer(left, right, bits, signed, ComparePredicate::LessEqual)
                }
                ComparePredicate::Greater => {
                    compare_integer(left, right, bits, signed, ComparePredicate::Greater)
                }
                ComparePredicate::GreaterEqual => {
                    compare_integer(left, right, bits, signed, ComparePredicate::GreaterEqual)
                }
            };
            Some(Constant::Bool(result))
        }
        (Constant::FloatBits { bits: left_bits }, Constant::FloatBits { bits: right_bits }) => {
            fold_float_compare(
                predicate,
                *left_bits,
                *right_bits,
                float_width(types, left.ty)?,
            )
        }
        _ => None,
    }
}

fn fold_cast(
    op: CastOp,
    value: Option<&KnownConstant>,
    target: TypeId,
    types: &[Type],
) -> Option<Constant> {
    let value = value?;
    match (op, &value.value) {
        (
            CastOp::IntegerExtend | CastOp::IntegerTruncate,
            Constant::Integer { value: value_value },
        ) => {
            let (target_signed, target_bits) = integer_shape(types, target)?;
            let (source_signed, source_bits) = integer_shape(types, value.ty)?;
            let source = integer_raw(*value_value, source_bits);
            let source = if op == CastOp::IntegerExtend && source_signed {
                normalize_integer(source, source_bits, true) as u128
            } else {
                source
            };
            Some(Constant::Integer {
                value: normalize_integer(source, target_bits, target_signed),
            })
        }
        (CastOp::FloatExtend | CastOp::FloatTruncate, Constant::FloatBits { bits }) => {
            let source_bits = float_width(types, value.ty)?;
            let target_bits = float_width(types, target)?;
            match (source_bits, target_bits) {
                (32, 32) => Some(Constant::FloatBits {
                    bits: *bits & u64::from(u32::MAX),
                }),
                (32, 64) => Some(Constant::FloatBits {
                    bits: f64::from(f32::from_bits(*bits as u32)).to_bits(),
                }),
                (64, 32) => Some(Constant::FloatBits {
                    bits: u64::from((f64::from_bits(*bits) as f32).to_bits()),
                }),
                (64, 64) => Some(Constant::FloatBits { bits: *bits }),
                _ => None,
            }
        }
        _ => None,
    }
}

fn fold_float_unary(
    bits: u64,
    result_ty: TypeId,
    types: &[Type],
    operation: impl FnOnce(f64) -> f64,
) -> Option<Constant> {
    match float_width(types, result_ty)? {
        32 => {
            let value = operation(f64::from(f32::from_bits(bits as u32)));
            Some(Constant::FloatBits {
                bits: u64::from((value as f32).to_bits()),
            })
        }
        64 => Some(Constant::FloatBits {
            bits: operation(f64::from_bits(bits)).to_bits(),
        }),
        _ => None,
    }
}

fn fold_float_binary(
    op: BinaryOp,
    left: u64,
    right: u64,
    result_ty: TypeId,
    types: &[Type],
) -> Option<Constant> {
    match float_width(types, result_ty)? {
        32 => {
            let left = f32::from_bits(left as u32);
            let right = f32::from_bits(right as u32);
            let value = match op {
                BinaryOp::Add => left + right,
                BinaryOp::Subtract => left - right,
                BinaryOp::Multiply => left * right,
                BinaryOp::Divide => left / right,
                BinaryOp::Remainder => left % right,
                _ => return None,
            };
            Some(Constant::FloatBits {
                bits: u64::from(value.to_bits()),
            })
        }
        64 => {
            let left = f64::from_bits(left);
            let right = f64::from_bits(right);
            let value = match op {
                BinaryOp::Add => left + right,
                BinaryOp::Subtract => left - right,
                BinaryOp::Multiply => left * right,
                BinaryOp::Divide => left / right,
                BinaryOp::Remainder => left % right,
                _ => return None,
            };
            Some(Constant::FloatBits {
                bits: value.to_bits(),
            })
        }
        _ => None,
    }
}

fn fold_float_compare(
    predicate: ComparePredicate,
    left: u64,
    right: u64,
    width: u16,
) -> Option<Constant> {
    let (left, right) = match width {
        32 => (
            f64::from(f32::from_bits(left as u32)),
            f64::from(f32::from_bits(right as u32)),
        ),
        64 => (f64::from_bits(left), f64::from_bits(right)),
        _ => return None,
    };
    let ordered = !left.is_nan() && !right.is_nan();
    Some(Constant::Bool(match predicate {
        ComparePredicate::Equal => ordered && left == right,
        ComparePredicate::NotEqual => !ordered || left != right,
        ComparePredicate::Less => ordered && left < right,
        ComparePredicate::LessEqual => ordered && left <= right,
        ComparePredicate::Greater => ordered && left > right,
        ComparePredicate::GreaterEqual => ordered && left >= right,
    }))
}

fn compare_integer(
    left: u128,
    right: u128,
    bits: u16,
    signed: bool,
    predicate: ComparePredicate,
) -> bool {
    if signed {
        let left = normalize_integer(left, bits, true);
        let right = normalize_integer(right, bits, true);
        match predicate {
            ComparePredicate::Less => left < right,
            ComparePredicate::LessEqual => left <= right,
            ComparePredicate::Greater => left > right,
            ComparePredicate::GreaterEqual => left >= right,
            ComparePredicate::Equal | ComparePredicate::NotEqual => unreachable!(),
        }
    } else {
        match predicate {
            ComparePredicate::Less => left < right,
            ComparePredicate::LessEqual => left <= right,
            ComparePredicate::Greater => left > right,
            ComparePredicate::GreaterEqual => left >= right,
            ComparePredicate::Equal | ComparePredicate::NotEqual => unreachable!(),
        }
    }
}

fn integer_shape(types: &[Type], ty: TypeId) -> Option<(bool, u16)> {
    match types.get(ty.index())? {
        Type::Integer { signed, bits } if (1..=128).contains(bits) => Some((*signed, *bits)),
        _ => None,
    }
}

fn float_width(types: &[Type], ty: TypeId) -> Option<u16> {
    match types.get(ty.index())? {
        Type::Float { bits } if matches!(*bits, 16 | 32 | 64) => Some(*bits),
        _ => None,
    }
}

fn integer_raw(value: i128, bits: u16) -> u128 {
    (value as u128) & integer_mask(bits)
}

fn integer_mask(bits: u16) -> u128 {
    if bits == 128 {
        u128::MAX
    } else {
        (1_u128 << bits) - 1
    }
}

fn normalize_integer(raw: u128, bits: u16, signed: bool) -> i128 {
    let mask = integer_mask(bits);
    let raw = raw & mask;
    if signed && bits < 128 && (raw & (1_u128 << (bits - 1))) != 0 {
        (raw | !mask) as i128
    } else {
        raw as i128
    }
}

#[cfg(test)]
mod tests {
    use super::fold_constants;
    use crate::{
        AddressSpace, BinaryOp, Block, BlockId, ComparePredicate, Constant, Function, FunctionId,
        Instruction, InstructionKind, Linkage, Module, Terminator, Type, TypeId, TypedValue,
        ValueId,
    };

    fn value(index: usize, ty: usize) -> TypedValue {
        TypedValue {
            value: ValueId::new(index),
            ty: TypeId::new(ty),
        }
    }

    #[test]
    fn folds_integer_boolean_and_select_chain_without_changing_value_ids() {
        let mut module = Module {
            types: vec![
                Type::Integer {
                    signed: true,
                    bits: 32,
                },
                Type::Bool,
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "main".to_owned(),
                linkage: Linkage::Internal,
                parameters: Vec::new(),
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        Instruction {
                            result: Some(value(0, 0)),
                            kind: InstructionKind::Constant(Constant::Integer { value: 40 }),
                            span: None,
                        },
                        Instruction {
                            result: Some(value(1, 0)),
                            kind: InstructionKind::Constant(Constant::Integer { value: 2 }),
                            span: None,
                        },
                        Instruction {
                            result: Some(value(2, 0)),
                            kind: InstructionKind::Binary {
                                op: BinaryOp::Add,
                                left: ValueId::new(0),
                                right: ValueId::new(1),
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(3, 1)),
                            kind: InstructionKind::Compare {
                                predicate: ComparePredicate::Equal,
                                left: ValueId::new(2),
                                right: ValueId::new(2),
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(4, 0)),
                            kind: InstructionKind::Select {
                                condition: ValueId::new(3),
                                when_true: ValueId::new(2),
                                when_false: ValueId::new(1),
                            },
                            span: None,
                        },
                    ],
                    terminator: Terminator::Return {
                        value: Some(ValueId::new(4)),
                    },
                    span: None,
                }],
                span: None,
            }],
        };

        let stats = fold_constants(&mut module);
        assert_eq!(stats.folded_instructions, 3);
        assert!(matches!(
            module.functions[0].blocks[0].instructions[2].kind,
            InstructionKind::Constant(Constant::Integer { value: 42 })
        ));
        assert!(matches!(
            module.functions[0].blocks[0].instructions[3].kind,
            InstructionKind::Constant(Constant::Bool(true))
        ));
        assert!(matches!(
            module.functions[0].blocks[0].instructions[4].kind,
            InstructionKind::Constant(Constant::Integer { value: 42 })
        ));
    }

    #[test]
    fn leaves_divide_by_zero_as_an_operation() {
        let mut module = Module {
            types: vec![Type::Integer {
                signed: true,
                bits: 32,
            }],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "div".to_owned(),
                linkage: Linkage::Internal,
                parameters: Vec::new(),
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        Instruction {
                            result: Some(value(0, 0)),
                            kind: InstructionKind::Constant(Constant::Integer { value: 1 }),
                            span: None,
                        },
                        Instruction {
                            result: Some(value(1, 0)),
                            kind: InstructionKind::Constant(Constant::Integer { value: 0 }),
                            span: None,
                        },
                        Instruction {
                            result: Some(value(2, 0)),
                            kind: InstructionKind::Binary {
                                op: BinaryOp::Divide,
                                left: ValueId::new(0),
                                right: ValueId::new(1),
                            },
                            span: None,
                        },
                    ],
                    terminator: Terminator::Return {
                        value: Some(ValueId::new(2)),
                    },
                    span: None,
                }],
                span: None,
            }],
        };

        assert_eq!(fold_constants(&mut module).folded_instructions, 0);
        assert!(matches!(
            module.functions[0].blocks[0].instructions[2].kind,
            InstructionKind::Binary { .. }
        ));
    }

    #[test]
    fn simplifies_constant_branch_and_removes_dead_scalar_instructions() {
        let mut module = Module {
            types: vec![
                Type::Bool,
                Type::Integer {
                    signed: true,
                    bits: 32,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "branch".to_owned(),
                linkage: Linkage::Internal,
                parameters: Vec::new(),
                result: TypeId::new(1),
                blocks: vec![
                    Block {
                        id: BlockId::new(0),
                        parameters: Vec::new(),
                        instructions: vec![
                            Instruction {
                                result: Some(value(0, 0)),
                                kind: InstructionKind::Constant(Constant::Bool(true)),
                                span: None,
                            },
                            Instruction {
                                result: Some(value(1, 1)),
                                kind: InstructionKind::Constant(Constant::Integer { value: 99 }),
                                span: None,
                            },
                        ],
                        terminator: Terminator::Branch {
                            condition: ValueId::new(0),
                            then_target: BlockId::new(1),
                            then_arguments: Vec::new(),
                            else_target: BlockId::new(2),
                            else_arguments: Vec::new(),
                        },
                        span: None,
                    },
                    Block {
                        id: BlockId::new(1),
                        parameters: Vec::new(),
                        instructions: vec![Instruction {
                            result: Some(value(2, 1)),
                            kind: InstructionKind::Constant(Constant::Integer { value: 1 }),
                            span: None,
                        }],
                        terminator: Terminator::Return {
                            value: Some(ValueId::new(2)),
                        },
                        span: None,
                    },
                    Block {
                        id: BlockId::new(2),
                        parameters: Vec::new(),
                        instructions: vec![Instruction {
                            result: Some(value(3, 1)),
                            kind: InstructionKind::Constant(Constant::Integer { value: 2 }),
                            span: None,
                        }],
                        terminator: Terminator::Return {
                            value: Some(ValueId::new(3)),
                        },
                        span: None,
                    },
                ],
                span: None,
            }],
        };

        let stats = super::simplify_cfg_and_dce(&mut module);
        assert_eq!(stats.simplified_terminators, 1);
        assert_eq!(stats.removed_instructions, 2);
        assert!(module.functions[0].blocks[0].instructions.is_empty());
        assert!(matches!(
            module.functions[0].blocks[0].terminator,
            Terminator::Jump {
                target: BlockId(1),
                ..
            }
        ));
    }

    #[test]
    fn inlines_single_block_scalar_function_and_keeps_dense_values() {
        let int = Type::Integer {
            signed: true,
            bits: 32,
        };
        let mut module = Module {
            types: vec![int],
            functions: vec![
                Function {
                    id: FunctionId::new(0),
                    name: "increment".to_owned(),
                    linkage: Linkage::Internal,
                    parameters: vec![crate::Parameter {
                        value: ValueId::new(0),
                        ty: TypeId::new(0),
                        name: Some("value".to_owned()),
                    }],
                    result: TypeId::new(0),
                    blocks: vec![Block {
                        id: BlockId::new(0),
                        parameters: Vec::new(),
                        instructions: vec![
                            Instruction {
                                result: Some(value(1, 0)),
                                kind: InstructionKind::Constant(Constant::Integer { value: 1 }),
                                span: None,
                            },
                            Instruction {
                                result: Some(value(2, 0)),
                                kind: InstructionKind::Binary {
                                    op: BinaryOp::Add,
                                    left: ValueId::new(0),
                                    right: ValueId::new(1),
                                },
                                span: None,
                            },
                        ],
                        terminator: Terminator::Return {
                            value: Some(ValueId::new(2)),
                        },
                        span: None,
                    }],
                    span: None,
                },
                Function {
                    id: FunctionId::new(1),
                    name: "main".to_owned(),
                    linkage: Linkage::Internal,
                    parameters: Vec::new(),
                    result: TypeId::new(0),
                    blocks: vec![Block {
                        id: BlockId::new(0),
                        parameters: Vec::new(),
                        instructions: vec![
                            Instruction {
                                result: Some(value(0, 0)),
                                kind: InstructionKind::Constant(Constant::Integer { value: 41 }),
                                span: None,
                            },
                            Instruction {
                                result: Some(value(1, 0)),
                                kind: InstructionKind::Call {
                                    function: FunctionId::new(0),
                                    arguments: vec![ValueId::new(0)],
                                },
                                span: None,
                            },
                        ],
                        terminator: Terminator::Return {
                            value: Some(ValueId::new(1)),
                        },
                        span: None,
                    }],
                    span: None,
                },
            ],
        };

        let stats = super::inline_tiny_functions(&mut module);
        assert_eq!(stats.inlined_calls, 1);
        assert!(
            module.functions[1].blocks[0]
                .instructions
                .iter()
                .all(|instruction| !matches!(instruction.kind, InstructionKind::Call { .. }))
        );
        assert!(
            module.functions[1].blocks[0]
                .instructions
                .iter()
                .any(|instruction| matches!(instruction.kind, InstructionKind::Binary { .. }))
        );
        assert!(crate::verify(&module).is_empty());
    }

    #[test]
    fn promotes_non_escaping_scalar_stack_slot_and_preserves_dense_ssa() {
        let mut module = Module {
            types: vec![
                Type::Integer {
                    signed: true,
                    bits: 32,
                },
                Type::Pointer {
                    pointee: TypeId::new(0),
                    address_space: AddressSpace::Stack,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "slot".to_owned(),
                linkage: Linkage::Internal,
                parameters: Vec::new(),
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        Instruction {
                            result: Some(value(0, 1)),
                            kind: InstructionKind::StackAlloc {
                                ty: TypeId::new(0),
                                count: None,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(1, 0)),
                            kind: InstructionKind::Constant(Constant::Integer { value: 7 }),
                            span: None,
                        },
                        Instruction {
                            result: None,
                            kind: InstructionKind::Store {
                                pointer: ValueId::new(0),
                                value: ValueId::new(1),
                                alignment: 4,
                                volatile: false,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(2, 0)),
                            kind: InstructionKind::Load {
                                pointer: ValueId::new(0),
                                alignment: 4,
                                volatile: false,
                            },
                            span: None,
                        },
                    ],
                    terminator: Terminator::Return {
                        value: Some(ValueId::new(2)),
                    },
                    span: None,
                }],
                span: None,
            }],
        };

        let mut volatile_module = module.clone();
        if let InstructionKind::Store { volatile, .. } =
            &mut volatile_module.functions[0].blocks[0].instructions[2].kind
        {
            *volatile = true;
        }
        assert_eq!(
            super::promote_scalar_stack_slots(&mut volatile_module).promoted_stack_slots,
            0
        );

        let stats = super::promote_scalar_stack_slots(&mut module);
        assert_eq!(stats.promoted_stack_slots, 1);
        let block = &module.functions[0].blocks[0];
        assert_eq!(block.instructions.len(), 1);
        assert!(matches!(
            block.instructions[0].kind,
            InstructionKind::Constant(Constant::Integer { value: 7 })
        ));
        assert!(matches!(
            block.terminator,
            Terminator::Return {
                value: Some(ValueId(0))
            }
        ));
        assert!(crate::verify(&module).is_empty());
    }

    #[test]
    fn promotes_non_escaping_vector_stack_slot() {
        let mut module = Module {
            types: vec![
                Type::Float { bits: 32 },
                Type::Vector {
                    element: TypeId::new(0),
                    lanes: 4,
                },
                Type::Pointer {
                    pointee: TypeId::new(1),
                    address_space: AddressSpace::Stack,
                },
                Type::Unit,
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "vector_slot".to_owned(),
                linkage: Linkage::Internal,
                parameters: Vec::new(),
                result: TypeId::new(3),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        Instruction {
                            result: Some(value(0, 2)),
                            kind: InstructionKind::StackAlloc {
                                ty: TypeId::new(1),
                                count: None,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(1, 0)),
                            kind: InstructionKind::Constant(Constant::FloatBits {
                                bits: u64::from(1.0_f32.to_bits()),
                            }),
                            span: None,
                        },
                        Instruction {
                            result: Some(value(2, 1)),
                            kind: InstructionKind::VectorSplat {
                                value: ValueId::new(1),
                                lanes: 4,
                            },
                            span: None,
                        },
                        Instruction {
                            result: None,
                            kind: InstructionKind::Store {
                                pointer: ValueId::new(0),
                                value: ValueId::new(2),
                                alignment: 16,
                                volatile: false,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(3, 1)),
                            kind: InstructionKind::Load {
                                pointer: ValueId::new(0),
                                alignment: 16,
                                volatile: false,
                            },
                            span: None,
                        },
                    ],
                    terminator: Terminator::Return { value: None },
                    span: None,
                }],
                span: None,
            }],
        };

        let stats = super::promote_scalar_stack_slots(&mut module);
        assert_eq!(stats.promoted_stack_slots, 1);
        let block = &module.functions[0].blocks[0];
        assert_eq!(block.instructions.len(), 2);
        assert!(matches!(
            block.instructions[1].kind,
            InstructionKind::VectorSplat { .. }
        ));
        assert!(crate::verify(&module).is_empty());
    }

    #[test]
    fn promotes_non_escaping_struct_stack_slot() {
        let mut module = Module {
            types: vec![
                Type::Float { bits: 32 },
                Type::Integer {
                    signed: false,
                    bits: 32,
                },
                Type::Struct {
                    fields: vec![TypeId::new(0), TypeId::new(1)],
                },
                Type::Pointer {
                    pointee: TypeId::new(2),
                    address_space: AddressSpace::Stack,
                },
                Type::Unit,
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "struct_slot".to_owned(),
                linkage: Linkage::Internal,
                parameters: Vec::new(),
                result: TypeId::new(4),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        Instruction {
                            result: Some(value(0, 3)),
                            kind: InstructionKind::StackAlloc {
                                ty: TypeId::new(2),
                                count: None,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(1, 0)),
                            kind: InstructionKind::Constant(Constant::FloatBits {
                                bits: u64::from(1.0_f32.to_bits()),
                            }),
                            span: None,
                        },
                        Instruction {
                            result: Some(value(2, 1)),
                            kind: InstructionKind::Constant(Constant::Integer { value: 2 }),
                            span: None,
                        },
                        Instruction {
                            result: Some(value(3, 2)),
                            kind: InstructionKind::Aggregate {
                                elements: vec![ValueId::new(1), ValueId::new(2)],
                            },
                            span: None,
                        },
                        Instruction {
                            result: None,
                            kind: InstructionKind::Store {
                                pointer: ValueId::new(0),
                                value: ValueId::new(3),
                                alignment: 4,
                                volatile: false,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(4, 2)),
                            kind: InstructionKind::Load {
                                pointer: ValueId::new(0),
                                alignment: 4,
                                volatile: false,
                            },
                            span: None,
                        },
                    ],
                    terminator: Terminator::Return { value: None },
                    span: None,
                }],
                span: None,
            }],
        };

        let stats = super::promote_scalar_stack_slots(&mut module);
        assert_eq!(stats.promoted_stack_slots, 1);
        let block = &module.functions[0].blocks[0];
        assert_eq!(block.instructions.len(), 3);
        assert!(matches!(
            block.instructions[2].kind,
            InstructionKind::Aggregate { .. }
        ));
        assert!(crate::verify(&module).is_empty());
    }

    #[test]
    fn promotes_local_vector_store_load_across_basic_blocks() {
        let mut module = Module {
            types: vec![
                Type::Float { bits: 32 },
                Type::Vector {
                    element: TypeId::new(0),
                    lanes: 4,
                },
                Type::Pointer {
                    pointee: TypeId::new(1),
                    address_space: AddressSpace::Stack,
                },
                Type::Unit,
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "vector_block_slot".to_owned(),
                linkage: Linkage::Internal,
                parameters: Vec::new(),
                result: TypeId::new(3),
                blocks: vec![
                    Block {
                        id: BlockId::new(0),
                        parameters: Vec::new(),
                        instructions: vec![
                            Instruction {
                                result: Some(value(0, 2)),
                                kind: InstructionKind::StackAlloc {
                                    ty: TypeId::new(1),
                                    count: None,
                                },
                                span: None,
                            },
                            Instruction {
                                result: Some(value(1, 0)),
                                kind: InstructionKind::Constant(Constant::FloatBits {
                                    bits: u64::from(1.0_f32.to_bits()),
                                }),
                                span: None,
                            },
                            Instruction {
                                result: Some(value(2, 1)),
                                kind: InstructionKind::VectorSplat {
                                    value: ValueId::new(1),
                                    lanes: 4,
                                },
                                span: None,
                            },
                        ],
                        terminator: Terminator::Jump {
                            target: BlockId::new(1),
                            arguments: Vec::new(),
                        },
                        span: None,
                    },
                    Block {
                        id: BlockId::new(1),
                        parameters: Vec::new(),
                        instructions: vec![
                            Instruction {
                                result: None,
                                kind: InstructionKind::Store {
                                    pointer: ValueId::new(0),
                                    value: ValueId::new(2),
                                    alignment: 16,
                                    volatile: false,
                                },
                                span: None,
                            },
                            Instruction {
                                result: Some(value(3, 1)),
                                kind: InstructionKind::Load {
                                    pointer: ValueId::new(0),
                                    alignment: 16,
                                    volatile: false,
                                },
                                span: None,
                            },
                        ],
                        terminator: Terminator::Return { value: None },
                        span: None,
                    },
                ],
                span: None,
            }],
        };

        let stats = super::promote_scalar_stack_slots(&mut module);
        assert_eq!(stats.promoted_stack_slots, 1);
        assert_eq!(module.functions[0].blocks[0].instructions.len(), 2);
        assert!(module.functions[0].blocks[1].instructions.is_empty());
        assert!(crate::verify(&module).is_empty());
    }

    #[test]
    fn does_not_promote_cross_cfg_store_that_does_not_dominate_load() {
        let mut module = Module {
            types: vec![
                Type::Integer {
                    signed: true,
                    bits: 32,
                },
                Type::Pointer {
                    pointee: TypeId::new(0),
                    address_space: AddressSpace::Stack,
                },
                Type::Bool,
                Type::Unit,
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "non_dominating_store".to_owned(),
                linkage: Linkage::Internal,
                parameters: vec![crate::Parameter {
                    value: ValueId::new(0),
                    ty: TypeId::new(2),
                    name: Some("condition".to_owned()),
                }],
                result: TypeId::new(3),
                blocks: vec![
                    Block {
                        id: BlockId::new(0),
                        parameters: Vec::new(),
                        instructions: vec![
                            Instruction {
                                result: Some(value(1, 1)),
                                kind: InstructionKind::StackAlloc {
                                    ty: TypeId::new(0),
                                    count: None,
                                },
                                span: None,
                            },
                            Instruction {
                                result: Some(value(2, 0)),
                                kind: InstructionKind::Constant(Constant::Integer { value: 7 }),
                                span: None,
                            },
                        ],
                        terminator: Terminator::Branch {
                            condition: ValueId::new(0),
                            then_target: BlockId::new(1),
                            then_arguments: Vec::new(),
                            else_target: BlockId::new(2),
                            else_arguments: Vec::new(),
                        },
                        span: None,
                    },
                    Block {
                        id: BlockId::new(1),
                        parameters: Vec::new(),
                        instructions: vec![Instruction {
                            result: None,
                            kind: InstructionKind::Store {
                                pointer: ValueId::new(1),
                                value: ValueId::new(2),
                                alignment: 4,
                                volatile: false,
                            },
                            span: None,
                        }],
                        terminator: Terminator::Jump {
                            target: BlockId::new(3),
                            arguments: Vec::new(),
                        },
                        span: None,
                    },
                    Block {
                        id: BlockId::new(2),
                        parameters: Vec::new(),
                        instructions: Vec::new(),
                        terminator: Terminator::Jump {
                            target: BlockId::new(3),
                            arguments: Vec::new(),
                        },
                        span: None,
                    },
                    Block {
                        id: BlockId::new(3),
                        parameters: Vec::new(),
                        instructions: vec![Instruction {
                            result: Some(value(3, 0)),
                            kind: InstructionKind::Load {
                                pointer: ValueId::new(1),
                                alignment: 4,
                                volatile: false,
                            },
                            span: None,
                        }],
                        terminator: Terminator::Return { value: None },
                        span: None,
                    },
                ],
                span: None,
            }],
        };

        let stats = super::promote_scalar_stack_slots(&mut module);
        assert_eq!(stats.promoted_stack_slots, 0);
        assert!(matches!(
            module.functions[0].blocks[0].instructions[0].kind,
            InstructionKind::StackAlloc { .. }
        ));
        assert!(matches!(
            module.functions[0].blocks[1].instructions[0].kind,
            InstructionKind::Store { .. }
        ));
        assert!(matches!(
            module.functions[0].blocks[3].instructions[0].kind,
            InstructionKind::Load { .. }
        ));
        assert!(crate::verify(&module).is_empty());
    }

    #[test]
    fn eliminates_only_proven_constant_bounds_checks() {
        let mut module = Module {
            types: vec![
                Type::Integer {
                    signed: true,
                    bits: 32,
                },
                Type::Unit,
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "bounds".to_owned(),
                linkage: Linkage::Internal,
                parameters: Vec::new(),
                result: TypeId::new(1),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        Instruction {
                            result: Some(value(0, 0)),
                            kind: InstructionKind::Constant(Constant::Integer { value: 1 }),
                            span: None,
                        },
                        Instruction {
                            result: Some(value(1, 0)),
                            kind: InstructionKind::Constant(Constant::Integer { value: 3 }),
                            span: None,
                        },
                        Instruction {
                            result: None,
                            kind: InstructionKind::BoundsCheck {
                                index: ValueId::new(0),
                                length: ValueId::new(1),
                            },
                            span: None,
                        },
                    ],
                    terminator: Terminator::Return { value: None },
                    span: None,
                }],
                span: None,
            }],
        };
        let mut negative = module.clone();
        if let InstructionKind::Constant(Constant::Integer { value }) =
            &mut negative.functions[0].blocks[0].instructions[0].kind
        {
            *value = -1;
        }
        assert_eq!(
            super::eliminate_proven_bounds_checks(&mut negative).eliminated_bounds_checks,
            0
        );
        assert_eq!(
            super::eliminate_proven_bounds_checks(&mut module).eliminated_bounds_checks,
            1
        );
        assert!(
            !module.functions[0].blocks[0]
                .instructions
                .iter()
                .any(|instruction| matches!(instruction.kind, InstructionKind::BoundsCheck { .. }))
        );
        assert!(crate::verify(&module).is_empty());
    }

    #[test]
    fn removes_redundant_bounds_checks_but_resets_after_stack_store() {
        let mut module = Module {
            types: vec![
                Type::Integer {
                    signed: false,
                    bits: 64,
                },
                Type::Unit,
                Type::Pointer {
                    pointee: TypeId::new(0),
                    address_space: AddressSpace::Stack,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "redundant_bounds".to_owned(),
                linkage: Linkage::Internal,
                parameters: vec![
                    crate::Parameter {
                        value: ValueId::new(0),
                        ty: TypeId::new(0),
                        name: Some("index".to_owned()),
                    },
                    crate::Parameter {
                        value: ValueId::new(1),
                        ty: TypeId::new(0),
                        name: Some("length".to_owned()),
                    },
                ],
                result: TypeId::new(1),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        Instruction {
                            result: Some(value(2, 2)),
                            kind: InstructionKind::StackAlloc {
                                ty: TypeId::new(0),
                                count: None,
                            },
                            span: None,
                        },
                        Instruction {
                            result: None,
                            kind: InstructionKind::Store {
                                pointer: ValueId::new(2),
                                value: ValueId::new(0),
                                alignment: 8,
                                volatile: false,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(3, 0)),
                            kind: InstructionKind::Load {
                                pointer: ValueId::new(2),
                                alignment: 8,
                                volatile: false,
                            },
                            span: None,
                        },
                        Instruction {
                            result: None,
                            kind: InstructionKind::BoundsCheck {
                                index: ValueId::new(3),
                                length: ValueId::new(1),
                            },
                            span: None,
                        },
                        Instruction {
                            result: None,
                            kind: InstructionKind::BoundsCheck {
                                index: ValueId::new(3),
                                length: ValueId::new(1),
                            },
                            span: None,
                        },
                        Instruction {
                            result: None,
                            kind: InstructionKind::Store {
                                pointer: ValueId::new(2),
                                value: ValueId::new(0),
                                alignment: 8,
                                volatile: false,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(4, 0)),
                            kind: InstructionKind::Load {
                                pointer: ValueId::new(2),
                                alignment: 8,
                                volatile: false,
                            },
                            span: None,
                        },
                        Instruction {
                            result: None,
                            kind: InstructionKind::BoundsCheck {
                                index: ValueId::new(4),
                                length: ValueId::new(1),
                            },
                            span: None,
                        },
                    ],
                    terminator: Terminator::Return { value: None },
                    span: None,
                }],
                span: None,
            }],
        };

        let stats = super::eliminate_redundant_bounds_checks(&mut module);
        assert_eq!(stats.eliminated_bounds_checks, 1);
        assert_eq!(
            module.functions[0].blocks[0]
                .instructions
                .iter()
                .filter(|instruction| {
                    matches!(instruction.kind, InstructionKind::BoundsCheck { .. })
                })
                .count(),
            2
        );
        assert!(crate::verify(&module).is_empty());
    }

    #[test]
    fn keeps_bounds_proof_across_disjoint_stack_store() {
        let mut module = Module {
            types: vec![
                Type::Integer {
                    signed: false,
                    bits: 64,
                },
                Type::Pointer {
                    pointee: TypeId::new(0),
                    address_space: AddressSpace::Stack,
                },
                Type::Unit,
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "disjoint_bounds".to_owned(),
                linkage: Linkage::Internal,
                parameters: Vec::new(),
                result: TypeId::new(2),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        Instruction {
                            result: Some(value(0, 1)),
                            kind: InstructionKind::StackAlloc {
                                ty: TypeId::new(0),
                                count: None,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(1, 1)),
                            kind: InstructionKind::StackAlloc {
                                ty: TypeId::new(0),
                                count: None,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(2, 0)),
                            kind: InstructionKind::Constant(Constant::Integer { value: 1 }),
                            span: None,
                        },
                        Instruction {
                            result: Some(value(3, 0)),
                            kind: InstructionKind::Constant(Constant::Integer { value: 4 }),
                            span: None,
                        },
                        Instruction {
                            result: None,
                            kind: InstructionKind::Store {
                                pointer: ValueId::new(0),
                                value: ValueId::new(2),
                                alignment: 8,
                                volatile: false,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(4, 0)),
                            kind: InstructionKind::Load {
                                pointer: ValueId::new(0),
                                alignment: 8,
                                volatile: false,
                            },
                            span: None,
                        },
                        Instruction {
                            result: None,
                            kind: InstructionKind::BoundsCheck {
                                index: ValueId::new(4),
                                length: ValueId::new(3),
                            },
                            span: None,
                        },
                        Instruction {
                            result: None,
                            kind: InstructionKind::Store {
                                pointer: ValueId::new(1),
                                value: ValueId::new(2),
                                alignment: 8,
                                volatile: false,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(5, 0)),
                            kind: InstructionKind::Load {
                                pointer: ValueId::new(0),
                                alignment: 8,
                                volatile: false,
                            },
                            span: None,
                        },
                        Instruction {
                            result: None,
                            kind: InstructionKind::BoundsCheck {
                                index: ValueId::new(5),
                                length: ValueId::new(3),
                            },
                            span: None,
                        },
                    ],
                    terminator: Terminator::Return { value: None },
                    span: None,
                }],
                span: None,
            }],
        };

        let stats = super::eliminate_redundant_bounds_checks(&mut module);
        assert_eq!(stats.eliminated_bounds_checks, 1);
        assert_eq!(
            module.functions[0].blocks[0]
                .instructions
                .iter()
                .filter(|instruction| {
                    matches!(instruction.kind, InstructionKind::BoundsCheck { .. })
                })
                .count(),
            1
        );
        assert!(crate::verify(&module).is_empty());
    }

    #[test]
    fn resets_bounds_proof_after_unknown_pointer_store() {
        let mut module = Module {
            types: vec![
                Type::Integer {
                    signed: false,
                    bits: 64,
                },
                Type::Unit,
                Type::Pointer {
                    pointee: TypeId::new(0),
                    address_space: AddressSpace::Stack,
                },
                Type::Bool,
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "unknown_store_bounds".to_owned(),
                linkage: Linkage::Internal,
                parameters: vec![crate::Parameter {
                    value: ValueId::new(0),
                    ty: TypeId::new(2),
                    name: Some("pointer".to_owned()),
                }],
                result: TypeId::new(1),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        Instruction {
                            result: Some(value(1, 2)),
                            kind: InstructionKind::StackAlloc {
                                ty: TypeId::new(0),
                                count: None,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(2, 0)),
                            kind: InstructionKind::Constant(Constant::Integer { value: 1 }),
                            span: None,
                        },
                        Instruction {
                            result: Some(value(3, 0)),
                            kind: InstructionKind::Constant(Constant::Integer { value: 4 }),
                            span: None,
                        },
                        Instruction {
                            result: None,
                            kind: InstructionKind::Store {
                                pointer: ValueId::new(1),
                                value: ValueId::new(2),
                                alignment: 8,
                                volatile: false,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(4, 0)),
                            kind: InstructionKind::Load {
                                pointer: ValueId::new(1),
                                alignment: 8,
                                volatile: false,
                            },
                            span: None,
                        },
                        Instruction {
                            result: None,
                            kind: InstructionKind::BoundsCheck {
                                index: ValueId::new(4),
                                length: ValueId::new(3),
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(6, 3)),
                            kind: InstructionKind::Compare {
                                predicate: ComparePredicate::Equal,
                                left: ValueId::new(2),
                                right: ValueId::new(3),
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(7, 2)),
                            kind: InstructionKind::Select {
                                condition: ValueId::new(6),
                                when_true: ValueId::new(0),
                                when_false: ValueId::new(1),
                            },
                            span: None,
                        },
                        Instruction {
                            result: None,
                            kind: InstructionKind::Store {
                                pointer: ValueId::new(7),
                                value: ValueId::new(2),
                                alignment: 8,
                                volatile: false,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(5, 0)),
                            kind: InstructionKind::Load {
                                pointer: ValueId::new(1),
                                alignment: 8,
                                volatile: false,
                            },
                            span: None,
                        },
                        Instruction {
                            result: None,
                            kind: InstructionKind::BoundsCheck {
                                index: ValueId::new(5),
                                length: ValueId::new(3),
                            },
                            span: None,
                        },
                    ],
                    terminator: Terminator::Return { value: None },
                    span: None,
                }],
                span: None,
            }],
        };

        let stats = super::eliminate_redundant_bounds_checks(&mut module);
        assert_eq!(stats.eliminated_bounds_checks, 0);
        assert_eq!(
            module.functions[0].blocks[0]
                .instructions
                .iter()
                .filter(|instruction| {
                    matches!(instruction.kind, InstructionKind::BoundsCheck { .. })
                })
                .count(),
            2
        );
        let verification_errors = crate::verify(&module);
        assert!(verification_errors.is_empty(), "{verification_errors:?}");
    }

    #[test]
    fn reuses_identical_pointer_offsets_within_a_block() {
        let mut module = Module {
            types: vec![
                Type::Integer {
                    signed: false,
                    bits: 32,
                },
                Type::Pointer {
                    pointee: TypeId::new(0),
                    address_space: AddressSpace::Generic,
                },
                Type::Unit,
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "offset_cse".to_owned(),
                linkage: Linkage::Internal,
                parameters: vec![
                    crate::Parameter {
                        value: ValueId::new(0),
                        ty: TypeId::new(1),
                        name: Some("base".to_owned()),
                    },
                    crate::Parameter {
                        value: ValueId::new(1),
                        ty: TypeId::new(0),
                        name: Some("index".to_owned()),
                    },
                ],
                result: TypeId::new(2),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        Instruction {
                            result: Some(value(2, 1)),
                            kind: InstructionKind::Offset {
                                base: ValueId::new(0),
                                indices: vec![ValueId::new(1)],
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(3, 1)),
                            kind: InstructionKind::Offset {
                                base: ValueId::new(0),
                                indices: vec![ValueId::new(1)],
                            },
                            span: None,
                        },
                    ],
                    terminator: Terminator::Return { value: None },
                    span: None,
                }],
                span: None,
            }],
        };

        let stats = super::eliminate_redundant_offsets(&mut module);
        assert_eq!(stats.removed_instructions, 1);
        assert_eq!(module.functions[0].blocks[0].instructions.len(), 1);
        assert!(crate::verify(&module).is_empty());
    }

    #[test]
    fn hoists_pure_loop_invariant_scalar_with_unique_preheader() {
        let mut module = Module {
            types: vec![
                Type::Bool,
                Type::Integer {
                    signed: true,
                    bits: 32,
                },
                Type::Unit,
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "loop".to_owned(),
                linkage: Linkage::Internal,
                parameters: vec![crate::Parameter {
                    value: ValueId::new(0),
                    ty: TypeId::new(0),
                    name: Some("condition".to_owned()),
                }],
                result: TypeId::new(2),
                blocks: vec![
                    Block {
                        id: BlockId::new(0),
                        parameters: Vec::new(),
                        instructions: vec![
                            Instruction {
                                result: Some(value(1, 1)),
                                kind: InstructionKind::Constant(Constant::Integer { value: 2 }),
                                span: None,
                            },
                            Instruction {
                                result: Some(value(2, 1)),
                                kind: InstructionKind::Constant(Constant::Integer { value: 3 }),
                                span: None,
                            },
                        ],
                        terminator: Terminator::Jump {
                            target: BlockId::new(1),
                            arguments: Vec::new(),
                        },
                        span: None,
                    },
                    Block {
                        id: BlockId::new(1),
                        parameters: Vec::new(),
                        instructions: Vec::new(),
                        terminator: Terminator::Branch {
                            condition: ValueId::new(0),
                            then_target: BlockId::new(2),
                            then_arguments: Vec::new(),
                            else_target: BlockId::new(3),
                            else_arguments: Vec::new(),
                        },
                        span: None,
                    },
                    Block {
                        id: BlockId::new(2),
                        parameters: Vec::new(),
                        instructions: vec![Instruction {
                            result: Some(value(3, 1)),
                            kind: InstructionKind::Binary {
                                op: BinaryOp::Add,
                                left: ValueId::new(1),
                                right: ValueId::new(2),
                            },
                            span: None,
                        }],
                        terminator: Terminator::Jump {
                            target: BlockId::new(1),
                            arguments: Vec::new(),
                        },
                        span: None,
                    },
                    Block {
                        id: BlockId::new(3),
                        parameters: Vec::new(),
                        instructions: Vec::new(),
                        terminator: Terminator::Return { value: None },
                        span: None,
                    },
                ],
                span: None,
            }],
        };

        let stats = super::canonicalize_loops_and_licm(&mut module);
        assert_eq!(stats.canonicalized_loops, 1);
        assert_eq!(stats.hoisted_loop_instructions, 1);
        assert!(
            module.functions[0].blocks[0]
                .instructions
                .iter()
                .any(|instruction| matches!(instruction.kind, InstructionKind::Binary { .. }))
        );
        assert!(module.functions[0].blocks[2].instructions.is_empty());
        assert!(crate::verify(&module).is_empty());
    }

    #[test]
    fn hoists_read_only_stack_loads_but_keeps_mutated_slots_in_loop() {
        let mut module = Module {
            types: vec![
                Type::Integer {
                    signed: true,
                    bits: 32,
                },
                Type::Pointer {
                    pointee: TypeId::new(0),
                    address_space: AddressSpace::Stack,
                },
                Type::Bool,
                Type::Unit,
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "stack_load_loop".to_owned(),
                linkage: Linkage::Internal,
                parameters: vec![crate::Parameter {
                    value: ValueId::new(0),
                    ty: TypeId::new(2),
                    name: Some("condition".to_owned()),
                }],
                result: TypeId::new(3),
                blocks: vec![
                    Block {
                        id: BlockId::new(0),
                        parameters: Vec::new(),
                        instructions: vec![
                            Instruction {
                                result: Some(value(1, 1)),
                                kind: InstructionKind::StackAlloc {
                                    ty: TypeId::new(0),
                                    count: None,
                                },
                                span: None,
                            },
                            Instruction {
                                result: Some(value(2, 1)),
                                kind: InstructionKind::StackAlloc {
                                    ty: TypeId::new(0),
                                    count: None,
                                },
                                span: None,
                            },
                            Instruction {
                                result: Some(value(3, 0)),
                                kind: InstructionKind::Constant(Constant::Integer { value: 7 }),
                                span: None,
                            },
                            Instruction {
                                result: None,
                                kind: InstructionKind::Store {
                                    pointer: ValueId::new(1),
                                    value: ValueId::new(3),
                                    alignment: 4,
                                    volatile: false,
                                },
                                span: None,
                            },
                            Instruction {
                                result: Some(value(4, 0)),
                                kind: InstructionKind::Constant(Constant::Integer { value: 11 }),
                                span: None,
                            },
                            Instruction {
                                result: None,
                                kind: InstructionKind::Store {
                                    pointer: ValueId::new(2),
                                    value: ValueId::new(4),
                                    alignment: 4,
                                    volatile: false,
                                },
                                span: None,
                            },
                        ],
                        terminator: Terminator::Jump {
                            target: BlockId::new(1),
                            arguments: Vec::new(),
                        },
                        span: None,
                    },
                    Block {
                        id: BlockId::new(1),
                        parameters: Vec::new(),
                        instructions: Vec::new(),
                        terminator: Terminator::Branch {
                            condition: ValueId::new(0),
                            then_target: BlockId::new(2),
                            then_arguments: Vec::new(),
                            else_target: BlockId::new(3),
                            else_arguments: Vec::new(),
                        },
                        span: None,
                    },
                    Block {
                        id: BlockId::new(2),
                        parameters: Vec::new(),
                        instructions: vec![
                            Instruction {
                                result: Some(value(5, 0)),
                                kind: InstructionKind::Load {
                                    pointer: ValueId::new(1),
                                    alignment: 4,
                                    volatile: false,
                                },
                                span: None,
                            },
                            Instruction {
                                result: Some(value(6, 0)),
                                kind: InstructionKind::Load {
                                    pointer: ValueId::new(2),
                                    alignment: 4,
                                    volatile: false,
                                },
                                span: None,
                            },
                            Instruction {
                                result: None,
                                kind: InstructionKind::Store {
                                    pointer: ValueId::new(2),
                                    value: ValueId::new(6),
                                    alignment: 4,
                                    volatile: false,
                                },
                                span: None,
                            },
                        ],
                        terminator: Terminator::Jump {
                            target: BlockId::new(1),
                            arguments: Vec::new(),
                        },
                        span: None,
                    },
                    Block {
                        id: BlockId::new(3),
                        parameters: Vec::new(),
                        instructions: Vec::new(),
                        terminator: Terminator::Return { value: None },
                        span: None,
                    },
                ],
                span: None,
            }],
        };

        let stats = super::canonicalize_loops_and_licm(&mut module);
        assert_eq!(stats.canonicalized_loops, 1);
        assert_eq!(stats.hoisted_loop_instructions, 1);
        assert_eq!(
            module.functions[0].blocks[0]
                .instructions
                .iter()
                .filter(|instruction| matches!(instruction.kind, InstructionKind::Load { .. }))
                .count(),
            1
        );
        assert_eq!(
            module.functions[0].blocks[2]
                .instructions
                .iter()
                .filter(|instruction| matches!(instruction.kind, InstructionKind::Load { .. }))
                .count(),
            1
        );
        assert!(crate::verify(&module).is_empty());
    }

    #[test]
    fn verifies_and_renders_target_neutral_vector_binary_operation() {
        let module = Module {
            types: vec![
                Type::Float { bits: 32 },
                Type::Vector {
                    element: TypeId::new(0),
                    lanes: 2,
                },
                Type::Integer {
                    signed: false,
                    bits: 32,
                },
                Type::Unit,
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "vector_add".to_owned(),
                linkage: Linkage::Internal,
                parameters: Vec::new(),
                result: TypeId::new(3),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        Instruction {
                            result: Some(value(0, 0)),
                            kind: InstructionKind::Constant(Constant::FloatBits {
                                bits: u64::from(1.0_f32.to_bits()),
                            }),
                            span: None,
                        },
                        Instruction {
                            result: Some(value(1, 1)),
                            kind: InstructionKind::VectorSplat {
                                value: ValueId::new(0),
                                lanes: 2,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(2, 1)),
                            kind: InstructionKind::VectorBinary {
                                op: BinaryOp::Add,
                                left: ValueId::new(1),
                                right: ValueId::new(1),
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(3, 2)),
                            kind: InstructionKind::Constant(Constant::Integer { value: 0 }),
                            span: None,
                        },
                        Instruction {
                            result: Some(value(4, 0)),
                            kind: InstructionKind::VectorExtract {
                                vector: ValueId::new(2),
                                lane: ValueId::new(3),
                            },
                            span: None,
                        },
                    ],
                    terminator: Terminator::Return { value: None },
                    span: None,
                }],
                span: None,
            }],
        };
        assert!(crate::verify(&module).is_empty());
        assert!(module.to_text().contains("vector.add %v1, %v1"));
    }
}
