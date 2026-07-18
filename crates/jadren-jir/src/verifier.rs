use std::collections::{BTreeMap, BTreeSet};

use jadren_source::Span;

use crate::{
    AddressSpace, BinaryOp, BlockId, BuiltinOp, CastOp, Constant, Function, FunctionId,
    Instruction, InstructionKind, Linkage, Module, Terminator, Type, TypeId, UnaryOp, ValueId,
};

/// Structural or typed SSA invariant violated by one JIR module.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationError {
    pub function: Option<FunctionId>,
    pub block: Option<BlockId>,
    pub value: Option<ValueId>,
    pub span: Option<Span>,
    pub message: String,
}

/// Verifies every mandatory JIR 0.1 invariant before backend lowering.
#[must_use]
pub fn verify(module: &Module) -> Vec<VerificationError> {
    let mut errors = Vec::new();
    verify_types(module, &mut errors);
    for (index, function) in module.functions.iter().enumerate() {
        if function.id.index() != index {
            errors.push(module_error(format!(
                "function @f{} is stored at index {index}",
                function.id.index()
            )));
        }
        verify_function(module, function, &mut errors);
    }
    errors
}

/// Verifies the additional address-space invariants required by GPU compute
/// 0.1 after the regular target-neutral JIR checks pass.
#[must_use]
pub fn verify_gpu(module: &Module) -> Vec<VerificationError> {
    let mut errors = verify(module);
    for (index, ty) in module.types.iter().enumerate() {
        if let Type::Pointer { address_space, .. } = ty
            && address_space.is_host()
        {
            errors.push(module_error(format!(
                "GPU pointer %t{index} uses non-GPU address space {address_space:?}"
            )));
        }
    }
    for function in &module.functions {
        let mut value_types = BTreeMap::new();
        for parameter in &function.parameters {
            value_types.insert(parameter.value, parameter.ty);
        }
        for block in &function.blocks {
            for parameter in &block.parameters {
                value_types.insert(parameter.value, parameter.ty);
            }
            for instruction in &block.instructions {
                if let Some(result) = instruction.result {
                    value_types.insert(result.value, result.ty);
                }
                if let InstructionKind::Cast {
                    op: crate::CastOp::PointerCast,
                    value,
                    target,
                } = &instruction.kind
                {
                    let source_space = value_types
                        .get(value)
                        .and_then(|ty| type_kind(module, *ty))
                        .and_then(|ty| match ty {
                            Type::Pointer { address_space, .. } => Some(*address_space),
                            _ => None,
                        });
                    let target_space = match type_kind(module, *target) {
                        Some(Type::Pointer { address_space, .. }) => Some(*address_space),
                        _ => None,
                    };
                    if let (Some(source_space), Some(target_space)) = (source_space, target_space)
                        && source_space != target_space
                        && source_space.is_gpu()
                        && target_space.is_gpu()
                    {
                        errors.push(instruction_error(
                            function,
                            block.id,
                            instruction,
                            "GPU pointer cast changes address space",
                        ));
                    }
                }
            }
        }
    }
    errors
}

#[derive(Clone, Copy)]
enum DefinitionSite {
    FunctionParameter,
    BlockParameter { block: usize },
    Instruction { block: usize, position: usize },
}

#[derive(Clone, Copy)]
struct Definition {
    ty: TypeId,
    site: DefinitionSite,
}

#[derive(Clone, Copy)]
struct UseSite {
    block: usize,
    position: usize,
    span: Option<Span>,
}

fn verify_types(module: &Module, errors: &mut Vec<VerificationError>) {
    for (index, ty) in module.types.iter().enumerate() {
        if let Some(previous) = module.types[..index]
            .iter()
            .position(|candidate| candidate == ty)
        {
            errors.push(module_error(format!(
                "type %t{index} duplicates canonical type %t{previous}"
            )));
        }
        let referenced: Vec<_> = match ty {
            Type::Pointer { pointee, .. } => vec![*pointee],
            Type::Function { parameters, result } => parameters
                .iter()
                .copied()
                .chain(std::iter::once(*result))
                .collect(),
            Type::Array { element, .. } | Type::Vector { element, .. } => vec![*element],
            Type::Struct { fields } | Type::NominalStruct { fields, .. } => fields.clone(),
            Type::Enum { variants } | Type::NominalEnum { variants, .. } => {
                variants.iter().flatten().copied().collect()
            }
            Type::Unit
            | Type::RegionHandle
            | Type::Bool
            | Type::Integer { .. }
            | Type::Float { .. } => Vec::new(),
        };
        for referenced in referenced {
            if referenced.index() >= module.types.len() {
                errors.push(module_error(format!(
                    "type %t{index} references missing type %t{}",
                    referenced.index()
                )));
            }
        }
        match ty {
            Type::Integer { bits, .. } | Type::Float { bits } if *bits == 0 => {
                errors.push(module_error(format!("type %t{index} has zero bit width")));
            }
            Type::Vector { lanes, .. } if *lanes == 0 => {
                errors.push(module_error(format!(
                    "type %t{index} has zero vector lanes"
                )));
            }
            _ => {}
        }
    }
}

fn verify_function(module: &Module, function: &Function, errors: &mut Vec<VerificationError>) {
    check_type(module, function.result, function, None, None, errors);
    if function.linkage == Linkage::Import {
        if !function.blocks.is_empty() {
            errors.push(function_error(function, "import function has a body"));
        }
    } else if function.blocks.is_empty() {
        errors.push(function_error(
            function,
            "defined function has no entry block",
        ));
    }
    let mut definitions = BTreeMap::new();
    for parameter in &function.parameters {
        check_type(
            module,
            parameter.ty,
            function,
            None,
            Some(parameter.value),
            errors,
        );
        define(
            parameter.value,
            parameter.ty,
            DefinitionSite::FunctionParameter,
            function,
            None,
            None,
            &mut definitions,
            errors,
        );
    }
    for (block_index, block) in function.blocks.iter().enumerate() {
        if block.id.index() != block_index {
            errors.push(block_error(
                function,
                block.id,
                block.span,
                format!(
                    "block ^bb{} is stored at index {block_index}",
                    block.id.index()
                ),
            ));
        }
        for parameter in &block.parameters {
            check_type(
                module,
                parameter.ty,
                function,
                Some(block.id),
                Some(parameter.value),
                errors,
            );
            define(
                parameter.value,
                parameter.ty,
                DefinitionSite::BlockParameter { block: block_index },
                function,
                Some(block.id),
                block.span,
                &mut definitions,
                errors,
            );
        }
        for (position, instruction) in block.instructions.iter().enumerate() {
            if let Some(result) = instruction.result {
                check_type(
                    module,
                    result.ty,
                    function,
                    Some(block.id),
                    Some(result.value),
                    errors,
                );
                define(
                    result.value,
                    result.ty,
                    DefinitionSite::Instruction {
                        block: block_index,
                        position,
                    },
                    function,
                    Some(block.id),
                    instruction.span,
                    &mut definitions,
                    errors,
                );
            }
        }
    }
    for (expected, value) in definitions.keys().enumerate() {
        if value.index() != expected {
            errors.push(function_error(
                function,
                format!(
                    "SSA values are not dense: expected %v{expected}, found %v{}",
                    value.index()
                ),
            ));
            break;
        }
    }
    if function.linkage == Linkage::Import {
        return;
    }

    let predecessors = predecessors(function);
    let dominators = dominators(function.blocks.len(), &predecessors);
    for (block_index, block) in function.blocks.iter().enumerate() {
        for (position, instruction) in block.instructions.iter().enumerate() {
            verify_instruction(
                module,
                function,
                block.id,
                instruction,
                UseSite {
                    block: block_index,
                    position,
                    span: instruction.span,
                },
                &definitions,
                &dominators,
                errors,
            );
        }
        verify_terminator(
            module,
            function,
            block.id,
            &block.terminator,
            UseSite {
                block: block_index,
                position: usize::MAX,
                span: block.span,
            },
            &definitions,
            &dominators,
            errors,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn define(
    value: ValueId,
    ty: TypeId,
    site: DefinitionSite,
    function: &Function,
    block: Option<BlockId>,
    span: Option<Span>,
    definitions: &mut BTreeMap<ValueId, Definition>,
    errors: &mut Vec<VerificationError>,
) {
    if definitions.insert(value, Definition { ty, site }).is_some() {
        errors.push(VerificationError {
            function: Some(function.id),
            block,
            value: Some(value),
            span,
            message: format!("SSA value %v{} has multiple definitions", value.index()),
        });
    }
}

fn predecessors(function: &Function) -> Vec<Vec<usize>> {
    let mut predecessors = vec![Vec::new(); function.blocks.len()];
    for (source, block) in function.blocks.iter().enumerate() {
        for target in successor_ids(&block.terminator) {
            if let Some(target_predecessors) = predecessors.get_mut(target.index()) {
                target_predecessors.push(source);
            }
        }
    }
    predecessors
}

fn dominators(block_count: usize, predecessors: &[Vec<usize>]) -> Vec<BTreeSet<usize>> {
    if block_count == 0 {
        return Vec::new();
    }
    let all: BTreeSet<_> = (0..block_count).collect();
    let mut dominators = vec![all; block_count];
    dominators[0] = BTreeSet::from([0]);
    let mut changed = true;
    while changed {
        changed = false;
        for block in 1..block_count {
            let mut next = if predecessors[block].is_empty() {
                BTreeSet::new()
            } else {
                let mut incoming = predecessors[block].iter();
                let first = *incoming.next().expect("nonempty predecessor list");
                incoming.fold(dominators[first].clone(), |current, predecessor| {
                    current
                        .intersection(&dominators[*predecessor])
                        .copied()
                        .collect()
                })
            };
            next.insert(block);
            if next != dominators[block] {
                dominators[block] = next;
                changed = true;
            }
        }
    }
    dominators
}

fn successor_ids(terminator: &Terminator) -> Vec<BlockId> {
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
            .chain([*default])
            .collect(),
    }
}

fn check_type(
    module: &Module,
    ty: TypeId,
    function: &Function,
    block: Option<BlockId>,
    value: Option<ValueId>,
    errors: &mut Vec<VerificationError>,
) {
    if ty.index() >= module.types.len() {
        errors.push(VerificationError {
            function: Some(function.id),
            block,
            value,
            span: function.span,
            message: format!("reference to missing type %t{}", ty.index()),
        });
    }
}

fn module_error(message: impl Into<String>) -> VerificationError {
    VerificationError {
        function: None,
        block: None,
        value: None,
        span: None,
        message: message.into(),
    }
}

fn function_error(function: &Function, message: impl Into<String>) -> VerificationError {
    VerificationError {
        function: Some(function.id),
        block: None,
        value: None,
        span: function.span,
        message: message.into(),
    }
}

fn block_error(
    function: &Function,
    block: BlockId,
    span: Option<Span>,
    message: impl Into<String>,
) -> VerificationError {
    VerificationError {
        function: Some(function.id),
        block: Some(block),
        value: None,
        span,
        message: message.into(),
    }
}

#[allow(clippy::too_many_arguments)]
fn verify_instruction(
    module: &Module,
    function: &Function,
    block: BlockId,
    instruction: &Instruction,
    site: UseSite,
    definitions: &BTreeMap<ValueId, Definition>,
    dominators: &[BTreeSet<usize>],
    errors: &mut Vec<VerificationError>,
) {
    let result = instruction.result.map(|result| result.ty);
    match &instruction.kind {
        InstructionKind::Constant(constant) => {
            let Some(result) = require_result(instruction, function, block, errors) else {
                return;
            };
            let compatible = match constant {
                Constant::Bool(_) => matches!(type_kind(module, result.ty), Some(Type::Bool)),
                Constant::Integer { .. } => {
                    matches!(type_kind(module, result.ty), Some(Type::Integer { .. }))
                }
                Constant::FloatBits { .. } => {
                    matches!(type_kind(module, result.ty), Some(Type::Float { .. }))
                }
                Constant::Null => {
                    matches!(type_kind(module, result.ty), Some(Type::Pointer { .. }))
                }
                Constant::Zero => !matches!(
                    type_kind(module, result.ty),
                    None | Some(Type::Unit | Type::RegionHandle)
                ),
            };
            if !compatible {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "constant kind differs from its result type",
                ));
            }
        }
        InstructionKind::Builtin(builtin) => {
            let Some(result) = require_result(instruction, function, block, errors) else {
                return;
            };
            let valid = match builtin {
                BuiltinOp::GlobalInvocationIdX
                | BuiltinOp::GlobalInvocationIdY
                | BuiltinOp::GlobalInvocationIdZ => matches!(
                    type_kind(module, result.ty),
                    Some(Type::Integer {
                        signed: false,
                        bits: 32
                    })
                ),
            };
            if !valid {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "GPU builtin result must be u32",
                ));
            }
        }
        InstructionKind::StringLiteral { .. } => {
            let Some(result) = require_result(instruction, function, block, errors) else {
                return;
            };
            if !matches!(type_kind(module, result.ty), Some(Type::Struct { .. })) {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "string literal result is not the String aggregate representation",
                ));
            }
        }
        InstructionKind::Aggregate { elements } => {
            let Some(result) = require_result(instruction, function, block, errors) else {
                return;
            };
            let expected: Option<Vec<_>> = match type_kind(module, result.ty) {
                Some(Type::Array { element, length }) => usize::try_from(*length)
                    .ok()
                    .map(|length| vec![*element; length]),
                Some(Type::Struct { fields } | Type::NominalStruct { fields, .. }) => {
                    Some(fields.clone())
                }
                _ => None,
            };
            let Some(expected) = expected else {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "aggregate result is not an array or struct",
                ));
                return;
            };
            check_typed_values(
                elements,
                &expected,
                module,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
        }
        InstructionKind::ExtractValue { aggregate, index } => {
            let Some(result) = require_result(instruction, function, block, errors) else {
                return;
            };
            let aggregate_ty = use_value(
                *aggregate,
                None,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            let expected = aggregate_ty.and_then(|ty| aggregate_element(module, ty, *index));
            if expected != Some(result.ty) {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "extract_value index or result type differs from aggregate layout",
                ));
            }
        }
        InstructionKind::ExtractElement { aggregate, index } => {
            let Some(result) = require_result(instruction, function, block, errors) else {
                return;
            };
            let aggregate_ty = use_value(
                *aggregate,
                None,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            use_integer(
                module,
                *index,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            let expected = aggregate_ty.and_then(|ty| match type_kind(module, ty) {
                Some(Type::Array { element, .. }) => Some(*element),
                _ => None,
            });
            if expected != Some(result.ty) {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "extract_element source or result type differs from array layout",
                ));
            }
        }
        InstructionKind::EnumConstruct { variant, fields } => {
            let Some(result) = require_result(instruction, function, block, errors) else {
                return;
            };
            let expected = enum_variant(module, result.ty, *variant);
            let Some(expected) = expected else {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "enum_construct variant does not exist in result type",
                ));
                return;
            };
            check_typed_values(
                fields,
                expected,
                module,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
        }
        InstructionKind::EnumTag { value } => {
            let Some(result) = require_result(instruction, function, block, errors) else {
                return;
            };
            let value_ty = use_value(
                *value,
                None,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            if !value_ty.is_some_and(|ty| is_enum(module, ty))
                || !matches!(
                    type_kind(module, result.ty),
                    Some(Type::Integer {
                        signed: false,
                        bits: 32
                    })
                )
            {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "enum_tag requires an enum and returns u32",
                ));
            }
        }
        InstructionKind::EnumExtract {
            value,
            variant,
            field,
        } => {
            let Some(result) = require_result(instruction, function, block, errors) else {
                return;
            };
            let value_ty = use_value(
                *value,
                None,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            let expected = value_ty
                .and_then(|ty| enum_variant(module, ty, *variant))
                .and_then(|fields| fields.get(*field as usize).copied());
            if expected != Some(result.ty) {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "enum_extract variant, field, or result type is invalid",
                ));
            }
        }
        InstructionKind::Unary { op, operand } => {
            let Some(result) = require_result(instruction, function, block, errors) else {
                return;
            };
            use_value(
                *operand,
                Some(result.ty),
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            let valid = match op {
                UnaryOp::Negate => is_numeric(module, result.ty),
                UnaryOp::Not => matches!(type_kind(module, result.ty), Some(Type::Bool)),
                UnaryOp::BitNot => is_integer_type(module, result.ty),
            };
            if !valid {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "unary operator is incompatible with its result type",
                ));
            }
        }
        InstructionKind::Binary { op, left, right } => {
            let Some(result) = require_result(instruction, function, block, errors) else {
                return;
            };
            for value in [*left, *right] {
                use_value(
                    value,
                    Some(result.ty),
                    function,
                    block,
                    site,
                    definitions,
                    dominators,
                    errors,
                );
            }
            let valid = match op {
                BinaryOp::Add
                | BinaryOp::Subtract
                | BinaryOp::Multiply
                | BinaryOp::Divide
                | BinaryOp::Remainder => is_numeric(module, result.ty),
                BinaryOp::BitAnd
                | BinaryOp::BitOr
                | BinaryOp::BitXor
                | BinaryOp::ShiftLeft
                | BinaryOp::ShiftRight => is_integer_type(module, result.ty),
            };
            if !valid {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "binary operator is incompatible with its result type",
                ));
            }
        }
        InstructionKind::Compare { left, right, .. } => {
            let Some(result) = require_result(instruction, function, block, errors) else {
                return;
            };
            let left_ty = use_value(
                *left,
                None,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            use_value(
                *right,
                left_ty,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            if !matches!(type_kind(module, result.ty), Some(Type::Bool)) {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "comparison result is not Bool",
                ));
            }
        }
        InstructionKind::Cast { op, value, target } => {
            let Some(result) = require_result(instruction, function, block, errors) else {
                return;
            };
            let source = use_value(
                *value,
                None,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            let compatible =
                source.is_some_and(|source| cast_compatible(module, *op, source, *target));
            if result.ty != *target || type_kind(module, *target).is_none() || !compatible {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "cast target differs from result type",
                ));
            }
        }
        InstructionKind::Select {
            condition,
            when_true,
            when_false,
        } => {
            let Some(result) = require_result(instruction, function, block, errors) else {
                return;
            };
            use_bool(
                module,
                *condition,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            for value in [*when_true, *when_false] {
                use_value(
                    value,
                    Some(result.ty),
                    function,
                    block,
                    site,
                    definitions,
                    dominators,
                    errors,
                );
            }
        }
        InstructionKind::StackAlloc { ty, count } => {
            let Some(result) = require_result(instruction, function, block, errors) else {
                return;
            };
            if !matches!(
                type_kind(module, result.ty),
                Some(Type::Pointer {
                    pointee,
                    address_space: AddressSpace::Stack
                }) if pointee == ty
            ) {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "stack_alloc result is not a stack pointer to the allocated type",
                ));
            }
            if let Some(count) = count {
                use_integer(
                    module,
                    *count,
                    function,
                    block,
                    site,
                    definitions,
                    dominators,
                    errors,
                );
            }
        }
        InstructionKind::RegionCreate => {
            let Some(result) = require_result(instruction, function, block, errors) else {
                return;
            };
            if !matches!(type_kind(module, result.ty), Some(Type::RegionHandle)) {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "region_create result is not region_handle",
                ));
            }
        }
        InstructionKind::RegionDestroy { region } => {
            forbid_result(instruction, function, block, errors);
            use_typed_kind(
                module,
                *region,
                |ty| matches!(ty, Type::RegionHandle),
                "region_destroy operand is not region_handle",
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
        }
        InstructionKind::Drop { value } => {
            forbid_result(instruction, function, block, errors);
            use_value(
                *value,
                None,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
        }
        InstructionKind::RegionAlloc { region, ty, count } => {
            let Some(result) = require_result(instruction, function, block, errors) else {
                return;
            };
            use_typed_kind(
                module,
                *region,
                |ty| matches!(ty, Type::RegionHandle),
                "region_alloc region is not region_handle",
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            use_integer(
                module,
                *count,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            if !matches!(
                type_kind(module, result.ty),
                Some(Type::Pointer {
                    pointee,
                    address_space: AddressSpace::Region
                }) if pointee == ty
            ) {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "region_alloc result is not a region pointer to the element type",
                ));
            }
        }
        InstructionKind::Load {
            pointer, alignment, ..
        } => {
            let Some(result) = require_result(instruction, function, block, errors) else {
                return;
            };
            let pointer_ty = use_value(
                *pointer,
                None,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            if !matches!(
                pointer_ty.and_then(|ty| type_kind(module, ty)),
                Some(Type::Pointer { pointee, .. }) if *pointee == result.ty
            ) {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "load pointer pointee differs from result type",
                ));
            }
            check_alignment(*alignment, function, block, instruction, errors);
        }
        InstructionKind::Store {
            pointer,
            value,
            alignment,
            ..
        } => {
            forbid_result(instruction, function, block, errors);
            let pointer_ty = use_value(
                *pointer,
                None,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            let value_ty = use_value(
                *value,
                None,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            if !matches!(
                pointer_ty.and_then(|ty| type_kind(module, ty)),
                Some(Type::Pointer { pointee, .. }) if Some(*pointee) == value_ty
            ) {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "store value differs from pointer pointee type",
                ));
            }
            check_alignment(*alignment, function, block, instruction, errors);
        }
        InstructionKind::Offset { base, indices } => {
            let Some(result) = require_result(instruction, function, block, errors) else {
                return;
            };
            let base_ty = use_value(
                *base,
                None,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            for index in indices {
                use_integer(
                    module,
                    *index,
                    function,
                    block,
                    site,
                    definitions,
                    dominators,
                    errors,
                );
            }
            if !matches!(type_kind(module, result.ty), Some(Type::Pointer { .. })) {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "offset result is not a pointer",
                ));
            } else if let (
                Some(Type::Pointer {
                    address_space: base_space,
                    ..
                }),
                Some(Type::Pointer {
                    address_space: result_space,
                    ..
                }),
            ) = (
                base_ty.and_then(|ty| type_kind(module, ty)),
                type_kind(module, result.ty),
            ) && base_space != result_space
            {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "offset changes pointer address space",
                ));
            }
        }
        InstructionKind::BoundsCheck { index, length } => {
            forbid_result(instruction, function, block, errors);
            let index_ty = use_value(
                *index,
                None,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            let length_ty = use_value(
                *length,
                index_ty,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            if !index_ty.is_some_and(|ty| is_integer_type(module, ty))
                || !length_ty.is_some_and(|ty| is_integer_type(module, ty))
            {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "bounds_check operands are not equal integer types",
                ));
            }
        }
        InstructionKind::VectorBoundsCheck {
            index,
            length,
            lanes,
        } => {
            forbid_result(instruction, function, block, errors);
            let index_ty = use_value(
                *index,
                None,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            let length_ty = use_value(
                *length,
                index_ty,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            if !index_ty.is_some_and(|ty| is_integer_type(module, ty))
                || !length_ty.is_some_and(|ty| is_integer_type(module, ty))
            {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "vector_bounds_check operands are not equal integer types",
                ));
            }
            if *lanes == 0 {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "vector_bounds_check requires at least one lane",
                ));
            }
        }
        InstructionKind::AssumeNoAlias { left, right } => {
            forbid_result(instruction, function, block, errors);
            let left_ty = use_value(
                *left,
                None,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            let right_ty = use_value(
                *right,
                None,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            if left == right {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "assume_noalias operands must be distinct",
                ));
            }
            if !left_ty.is_some_and(|ty| is_disjoint_handle_type(module, ty))
                || !right_ty.is_some_and(|ty| is_disjoint_handle_type(module, ty))
            {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "assume_noalias operands must be Slice/Buffer handles",
                ));
            }
        }
        InstructionKind::Call {
            function: callee,
            arguments,
        } => verify_call(
            module,
            function,
            block,
            instruction,
            *callee,
            arguments,
            site,
            definitions,
            dominators,
            errors,
        ),
        InstructionKind::FunctionAddress { function: target } => {
            let Some(result) = require_result(instruction, function, block, errors) else {
                return;
            };
            let Some(target) = module.functions.get(target.index()) else {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "function_address references a missing function",
                ));
                return;
            };
            let valid = match type_kind(module, result.ty) {
                Some(Type::Function {
                    parameters,
                    result: result_type,
                }) => {
                    parameters.len() == target.parameters.len()
                        && parameters
                            .iter()
                            .zip(&target.parameters)
                            .all(|(expected, actual)| *expected == actual.ty)
                        && *result_type == target.result
                }
                _ => false,
            };
            if !valid {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "function_address result does not match target signature",
                ));
            }
        }
        InstructionKind::IndirectCall { callee, arguments } => verify_indirect_call(
            module,
            function,
            block,
            instruction,
            *callee,
            arguments,
            site,
            definitions,
            dominators,
            errors,
        ),
        InstructionKind::VectorSplat { value, lanes } => {
            let Some(result) = require_result(instruction, function, block, errors) else {
                return;
            };
            let expected = match type_kind(module, result.ty) {
                Some(Type::Vector {
                    element,
                    lanes: result_lanes,
                }) if result_lanes == lanes => Some(*element),
                _ => None,
            };
            use_value(
                *value,
                expected,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            if expected.is_none() {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "vector_splat result lanes are inconsistent",
                ));
            }
        }
        InstructionKind::VectorBinary { op, left, right } => {
            let Some(result) = require_result(instruction, function, block, errors) else {
                return;
            };
            let element = match type_kind(module, result.ty) {
                Some(Type::Vector { element, .. }) => Some(*element),
                _ => None,
            };
            use_value(
                *left,
                Some(result.ty),
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            use_value(
                *right,
                Some(result.ty),
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            let valid = element.is_some_and(|element| match op {
                BinaryOp::Add
                | BinaryOp::Subtract
                | BinaryOp::Multiply
                | BinaryOp::Divide
                | BinaryOp::Remainder => is_numeric(module, element),
                BinaryOp::BitAnd
                | BinaryOp::BitOr
                | BinaryOp::BitXor
                | BinaryOp::ShiftLeft
                | BinaryOp::ShiftRight => is_integer_type(module, element),
            });
            if !valid {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "vector_binary operation is incompatible with its element type",
                ));
            }
        }
        InstructionKind::VectorExtract { vector, lane } => {
            let Some(result) = require_result(instruction, function, block, errors) else {
                return;
            };
            let vector_ty = use_value(
                *vector,
                None,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            use_integer(
                module,
                *lane,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            if !matches!(
                vector_ty.and_then(|ty| type_kind(module, ty)),
                Some(Type::Vector { element, .. }) if *element == result.ty
            ) {
                errors.push(instruction_error(
                    function,
                    block,
                    instruction,
                    "vector_extract result differs from lane type",
                ));
            }
        }
        InstructionKind::VectorInsert {
            vector,
            lane,
            value,
        } => {
            let Some(result) = require_result(instruction, function, block, errors) else {
                return;
            };
            let vector_ty = use_value(
                *vector,
                Some(result.ty),
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            use_integer(
                module,
                *lane,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            let element = vector_ty.and_then(|ty| match type_kind(module, ty) {
                Some(Type::Vector { element, .. }) => Some(*element),
                _ => None,
            });
            use_value(
                *value,
                element,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
        }
    }
    let _ = result;
}

#[allow(clippy::too_many_arguments)]
fn verify_terminator(
    module: &Module,
    function: &Function,
    block: BlockId,
    terminator: &Terminator,
    site: UseSite,
    definitions: &BTreeMap<ValueId, Definition>,
    dominators: &[BTreeSet<usize>],
    errors: &mut Vec<VerificationError>,
) {
    match terminator {
        Terminator::Return { value } => {
            if matches!(type_kind(module, function.result), Some(Type::Unit)) {
                if value.is_some() {
                    errors.push(block_error(
                        function,
                        block,
                        site.span,
                        "Unit function returns a value",
                    ));
                }
            } else if let Some(value) = value {
                use_value(
                    *value,
                    Some(function.result),
                    function,
                    block,
                    site,
                    definitions,
                    dominators,
                    errors,
                );
            } else {
                errors.push(block_error(
                    function,
                    block,
                    site.span,
                    "non-Unit function returns no value",
                ));
            }
        }
        Terminator::Jump { target, arguments } => check_edge(
            module,
            function,
            block,
            *target,
            arguments,
            site,
            definitions,
            dominators,
            errors,
        ),
        Terminator::Branch {
            condition,
            then_target,
            then_arguments,
            else_target,
            else_arguments,
        } => {
            use_bool(
                module,
                *condition,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            check_edge(
                module,
                function,
                block,
                *then_target,
                then_arguments,
                site,
                definitions,
                dominators,
                errors,
            );
            check_edge(
                module,
                function,
                block,
                *else_target,
                else_arguments,
                site,
                definitions,
                dominators,
                errors,
            );
        }
        Terminator::Switch {
            discriminant,
            cases,
            default,
            default_arguments,
        } => {
            use_integer(
                module,
                *discriminant,
                function,
                block,
                site,
                definitions,
                dominators,
                errors,
            );
            let mut values = BTreeSet::new();
            for case in cases {
                if !values.insert(case.value) {
                    errors.push(block_error(
                        function,
                        block,
                        site.span,
                        "switch contains duplicate case values",
                    ));
                }
                check_edge(
                    module,
                    function,
                    block,
                    case.target,
                    &case.arguments,
                    site,
                    definitions,
                    dominators,
                    errors,
                );
            }
            check_edge(
                module,
                function,
                block,
                *default,
                default_arguments,
                site,
                definitions,
                dominators,
                errors,
            );
        }
        Terminator::Unreachable => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn verify_call(
    module: &Module,
    function: &Function,
    block: BlockId,
    instruction: &Instruction,
    callee: FunctionId,
    arguments: &[ValueId],
    site: UseSite,
    definitions: &BTreeMap<ValueId, Definition>,
    dominators: &[BTreeSet<usize>],
    errors: &mut Vec<VerificationError>,
) {
    let Some(callee) = module.functions.get(callee.index()) else {
        errors.push(instruction_error(
            function,
            block,
            instruction,
            "call references a missing function",
        ));
        return;
    };
    let expected: Vec<_> = callee
        .parameters
        .iter()
        .map(|parameter| parameter.ty)
        .collect();
    check_typed_values(
        arguments,
        &expected,
        module,
        function,
        block,
        site,
        definitions,
        dominators,
        errors,
    );
    if matches!(type_kind(module, callee.result), Some(Type::Unit)) {
        forbid_result(instruction, function, block, errors);
    } else if instruction.result.map(|result| result.ty) != Some(callee.result) {
        errors.push(instruction_error(
            function,
            block,
            instruction,
            "call result differs from callee result type",
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn verify_indirect_call(
    module: &Module,
    function: &Function,
    block: BlockId,
    instruction: &Instruction,
    callee: ValueId,
    arguments: &[ValueId],
    site: UseSite,
    definitions: &BTreeMap<ValueId, Definition>,
    dominators: &[BTreeSet<usize>],
    errors: &mut Vec<VerificationError>,
) {
    let Some(callee_type) = use_value(
        callee,
        None,
        function,
        block,
        site,
        definitions,
        dominators,
        errors,
    ) else {
        return;
    };
    let Some(Type::Function { parameters, result }) = type_kind(module, callee_type) else {
        errors.push(instruction_error(
            function,
            block,
            instruction,
            "indirect_call callee is not a function pointer",
        ));
        return;
    };
    check_typed_values(
        arguments,
        parameters,
        module,
        function,
        block,
        site,
        definitions,
        dominators,
        errors,
    );
    if matches!(type_kind(module, *result), Some(Type::Unit)) {
        forbid_result(instruction, function, block, errors);
    } else if instruction.result.map(|value| value.ty) != Some(*result) {
        errors.push(instruction_error(
            function,
            block,
            instruction,
            "indirect_call result differs from function pointer result type",
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn check_edge(
    module: &Module,
    function: &Function,
    source: BlockId,
    target: BlockId,
    arguments: &[ValueId],
    site: UseSite,
    definitions: &BTreeMap<ValueId, Definition>,
    dominators: &[BTreeSet<usize>],
    errors: &mut Vec<VerificationError>,
) {
    let Some(target_block) = function.blocks.get(target.index()) else {
        errors.push(block_error(
            function,
            source,
            site.span,
            format!("edge references missing block ^bb{}", target.index()),
        ));
        return;
    };
    let expected: Vec<_> = target_block
        .parameters
        .iter()
        .map(|parameter| parameter.ty)
        .collect();
    check_typed_values(
        arguments,
        &expected,
        module,
        function,
        source,
        site,
        definitions,
        dominators,
        errors,
    );
}

#[allow(clippy::too_many_arguments)]
fn check_typed_values(
    values: &[ValueId],
    expected: &[TypeId],
    _module: &Module,
    function: &Function,
    block: BlockId,
    site: UseSite,
    definitions: &BTreeMap<ValueId, Definition>,
    dominators: &[BTreeSet<usize>],
    errors: &mut Vec<VerificationError>,
) {
    if values.len() != expected.len() {
        errors.push(block_error(
            function,
            block,
            site.span,
            format!(
                "value argument count {} differs from expected {}",
                values.len(),
                expected.len()
            ),
        ));
    }
    for (value, expected) in values.iter().zip(expected) {
        use_value(
            *value,
            Some(*expected),
            function,
            block,
            site,
            definitions,
            dominators,
            errors,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn use_value(
    value: ValueId,
    expected: Option<TypeId>,
    function: &Function,
    block: BlockId,
    site: UseSite,
    definitions: &BTreeMap<ValueId, Definition>,
    dominators: &[BTreeSet<usize>],
    errors: &mut Vec<VerificationError>,
) -> Option<TypeId> {
    let Some(definition) = definitions.get(&value).copied() else {
        errors.push(VerificationError {
            function: Some(function.id),
            block: Some(block),
            value: Some(value),
            span: site.span,
            message: format!("use of undefined SSA value %v{}", value.index()),
        });
        return None;
    };
    if let Some(expected) = expected
        && definition.ty != expected
    {
        errors.push(VerificationError {
            function: Some(function.id),
            block: Some(block),
            value: Some(value),
            span: site.span,
            message: format!(
                "SSA value %v{} has type %t{} but %t{} is required",
                value.index(),
                definition.ty.index(),
                expected.index()
            ),
        });
    }
    let dominates = match definition.site {
        DefinitionSite::FunctionParameter => true,
        DefinitionSite::BlockParameter {
            block: definition_block,
        } => {
            definition_block == site.block
                || dominators
                    .get(site.block)
                    .is_some_and(|set| set.contains(&definition_block))
        }
        DefinitionSite::Instruction {
            block: definition_block,
            position,
        } => {
            if definition_block == site.block {
                position < site.position
            } else {
                dominators
                    .get(site.block)
                    .is_some_and(|set| set.contains(&definition_block))
            }
        }
    };
    if !dominates {
        errors.push(VerificationError {
            function: Some(function.id),
            block: Some(block),
            value: Some(value),
            span: site.span,
            message: format!("SSA value %v{} does not dominate this use", value.index()),
        });
    }
    Some(definition.ty)
}

#[allow(clippy::too_many_arguments)]
fn use_integer(
    module: &Module,
    value: ValueId,
    function: &Function,
    block: BlockId,
    site: UseSite,
    definitions: &BTreeMap<ValueId, Definition>,
    dominators: &[BTreeSet<usize>],
    errors: &mut Vec<VerificationError>,
) {
    use_typed_kind(
        module,
        value,
        |ty| matches!(ty, Type::Integer { .. }),
        "SSA value is not an integer",
        function,
        block,
        site,
        definitions,
        dominators,
        errors,
    );
}

#[allow(clippy::too_many_arguments)]
fn use_bool(
    module: &Module,
    value: ValueId,
    function: &Function,
    block: BlockId,
    site: UseSite,
    definitions: &BTreeMap<ValueId, Definition>,
    dominators: &[BTreeSet<usize>],
    errors: &mut Vec<VerificationError>,
) {
    use_typed_kind(
        module,
        value,
        |ty| matches!(ty, Type::Bool),
        "SSA value is not Bool",
        function,
        block,
        site,
        definitions,
        dominators,
        errors,
    );
}

#[allow(clippy::too_many_arguments)]
fn use_typed_kind(
    module: &Module,
    value: ValueId,
    predicate: impl FnOnce(&Type) -> bool,
    message: &str,
    function: &Function,
    block: BlockId,
    site: UseSite,
    definitions: &BTreeMap<ValueId, Definition>,
    dominators: &[BTreeSet<usize>],
    errors: &mut Vec<VerificationError>,
) {
    let ty = use_value(
        value,
        None,
        function,
        block,
        site,
        definitions,
        dominators,
        errors,
    );
    if !ty
        .and_then(|ty| type_kind(module, ty))
        .is_some_and(predicate)
    {
        errors.push(VerificationError {
            function: Some(function.id),
            block: Some(block),
            value: Some(value),
            span: site.span,
            message: message.to_owned(),
        });
    }
}

fn require_result<'a>(
    instruction: &'a Instruction,
    function: &Function,
    block: BlockId,
    errors: &mut Vec<VerificationError>,
) -> Option<&'a crate::TypedValue> {
    instruction.result.as_ref().or_else(|| {
        errors.push(instruction_error(
            function,
            block,
            instruction,
            "value-producing instruction has no result",
        ));
        None
    })
}

fn forbid_result(
    instruction: &Instruction,
    function: &Function,
    block: BlockId,
    errors: &mut Vec<VerificationError>,
) {
    if instruction.result.is_some() {
        errors.push(instruction_error(
            function,
            block,
            instruction,
            "side-effect-only instruction has a result",
        ));
    }
}

fn instruction_error(
    function: &Function,
    block: BlockId,
    instruction: &Instruction,
    message: impl Into<String>,
) -> VerificationError {
    VerificationError {
        function: Some(function.id),
        block: Some(block),
        value: instruction.result.map(|result| result.value),
        span: instruction.span,
        message: message.into(),
    }
}

fn type_kind(module: &Module, ty: TypeId) -> Option<&Type> {
    module.types.get(ty.index())
}

fn is_integer_type(module: &Module, ty: TypeId) -> bool {
    matches!(type_kind(module, ty), Some(Type::Integer { .. }))
}

fn is_numeric(module: &Module, ty: TypeId) -> bool {
    matches!(
        type_kind(module, ty),
        Some(Type::Integer { .. } | Type::Float { .. })
    )
}

fn cast_compatible(module: &Module, op: CastOp, source: TypeId, target: TypeId) -> bool {
    match (op, type_kind(module, source), type_kind(module, target)) {
        (
            CastOp::IntegerExtend,
            Some(Type::Integer {
                bits: source_bits, ..
            }),
            Some(Type::Integer {
                bits: target_bits, ..
            }),
        ) => source_bits < target_bits,
        (
            CastOp::IntegerTruncate,
            Some(Type::Integer {
                bits: source_bits, ..
            }),
            Some(Type::Integer {
                bits: target_bits, ..
            }),
        ) => source_bits > target_bits,
        (CastOp::IntegerToFloat, Some(Type::Integer { .. }), Some(Type::Float { .. }))
        | (CastOp::FloatToInteger, Some(Type::Float { .. }), Some(Type::Integer { .. })) => true,
        (
            CastOp::FloatExtend,
            Some(Type::Float { bits: source_bits }),
            Some(Type::Float { bits: target_bits }),
        ) => source_bits < target_bits,
        (
            CastOp::FloatTruncate,
            Some(Type::Float { bits: source_bits }),
            Some(Type::Float { bits: target_bits }),
        ) => source_bits > target_bits,
        (CastOp::Bitcast, Some(source), Some(target)) => {
            scalar_bits(source).is_some_and(|bits| scalar_bits(target) == Some(bits))
        }
        (CastOp::PointerCast, Some(Type::Pointer { .. }), Some(Type::Pointer { .. })) => true,
        _ => false,
    }
}

const fn scalar_bits(ty: &Type) -> Option<u16> {
    match ty {
        Type::Bool => Some(1),
        Type::Integer { bits, .. } | Type::Float { bits } => Some(*bits),
        _ => None,
    }
}

fn check_alignment(
    alignment: u32,
    function: &Function,
    block: BlockId,
    instruction: &Instruction,
    errors: &mut Vec<VerificationError>,
) {
    if !alignment.is_power_of_two() {
        errors.push(instruction_error(
            function,
            block,
            instruction,
            "memory alignment is not a nonzero power of two",
        ));
    }
}

fn is_disjoint_handle_type(module: &Module, ty: TypeId) -> bool {
    let fields = match type_kind(module, ty) {
        Some(Type::Struct { fields } | Type::NominalStruct { fields, .. }) => fields,
        _ => return false,
    };
    (fields.len() == 2 || fields.len() == 3)
        && matches!(type_kind(module, fields[0]), Some(Type::Pointer { .. }))
        && fields[1..]
            .iter()
            .all(|field| is_integer_type(module, *field))
}

fn aggregate_element(module: &Module, ty: TypeId, index: u32) -> Option<TypeId> {
    match type_kind(module, ty)? {
        Type::Array { element, length } if u64::from(index) < *length => Some(*element),
        Type::Struct { fields } | Type::NominalStruct { fields, .. } => {
            fields.get(index as usize).copied()
        }
        _ => None,
    }
}

fn enum_variant(module: &Module, ty: TypeId, variant: u32) -> Option<&[TypeId]> {
    match type_kind(module, ty)? {
        Type::Enum { variants } | Type::NominalEnum { variants, .. } => {
            variants.get(variant as usize).map(Vec::as_slice)
        }
        _ => None,
    }
}

fn is_enum(module: &Module, ty: TypeId) -> bool {
    matches!(
        type_kind(module, ty),
        Some(Type::Enum { .. } | Type::NominalEnum { .. })
    )
}

#[cfg(test)]
mod tests {
    use crate::{
        AddressSpace, BinaryOp, Block, BlockId, CastOp, Constant, Function, FunctionId,
        Instruction, InstructionKind, Linkage, Module, Parameter, Terminator, Type, TypeId,
        TypedValue, ValueId,
    };

    use super::{verify, verify_gpu};

    #[test]
    fn accepts_well_typed_dense_ssa() {
        let module = scalar_module();
        assert!(verify(&module).is_empty(), "{:?}", verify(&module));
    }

    #[test]
    fn gpu_verifier_rejects_host_pointer_types() {
        let module = Module {
            types: vec![
                Type::Unit,
                Type::Pointer {
                    pointee: TypeId::new(0),
                    address_space: AddressSpace::Stack,
                },
            ],
            functions: Vec::new(),
        };
        assert!(contains(&verify_gpu(&module), "uses non-GPU address space"));
    }

    #[test]
    fn gpu_verifier_rejects_cross_space_pointer_cast() {
        let storage = TypeId::new(1);
        let uniform = TypeId::new(2);
        let module = Module {
            types: vec![
                Type::Unit,
                Type::Pointer {
                    pointee: TypeId::new(0),
                    address_space: AddressSpace::Storage,
                },
                Type::Pointer {
                    pointee: TypeId::new(0),
                    address_space: AddressSpace::Uniform,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "cast_space".to_owned(),
                linkage: Linkage::Internal,
                parameters: vec![Parameter {
                    value: ValueId::new(0),
                    ty: storage,
                    name: None,
                }],
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(1),
                            ty: uniform,
                        }),
                        kind: InstructionKind::Cast {
                            op: CastOp::PointerCast,
                            value: ValueId::new(0),
                            target: uniform,
                        },
                        span: None,
                    }],
                    terminator: Terminator::Return { value: None },
                    span: None,
                }],
                span: None,
            }],
        };
        assert!(contains(
            &verify_gpu(&module),
            "GPU pointer cast changes address space"
        ));
    }

    #[test]
    fn rejects_identity_duplicate_undefined_and_operation_corruption() {
        let mut module = scalar_module();
        module.functions[0].id = FunctionId::new(1);
        module.functions[0].blocks[0].instructions[0].result = Some(TypedValue {
            value: ValueId::new(0),
            ty: TypeId::new(0),
        });
        module.functions[0].blocks[0].instructions[0].kind =
            InstructionKind::Constant(Constant::Bool(true));
        module.functions[0].blocks[0].terminator = Terminator::Return {
            value: Some(ValueId::new(99)),
        };

        let errors = verify(&module);
        assert!(contains(&errors, "stored at index"));
        assert!(contains(&errors, "multiple definitions"));
        assert!(contains(&errors, "constant kind differs"));
        assert!(contains(&errors, "undefined SSA value"));
    }

    #[test]
    fn rejects_value_that_does_not_dominate_join_use() {
        let i32_ty = TypeId::new(0);
        let bool_ty = TypeId::new(1);
        let module = Module {
            types: vec![
                Type::Integer {
                    signed: true,
                    bits: 32,
                },
                Type::Bool,
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "bad_join".to_owned(),
                linkage: Linkage::Internal,
                parameters: vec![Parameter {
                    value: ValueId::new(0),
                    ty: bool_ty,
                    name: None,
                }],
                result: i32_ty,
                blocks: vec![
                    Block {
                        id: BlockId::new(0),
                        parameters: Vec::new(),
                        instructions: Vec::new(),
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
                            result: Some(TypedValue {
                                value: ValueId::new(1),
                                ty: i32_ty,
                            }),
                            kind: InstructionKind::Constant(Constant::Integer { value: 1 }),
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
                            result: Some(TypedValue {
                                value: ValueId::new(2),
                                ty: i32_ty,
                            }),
                            kind: InstructionKind::Binary {
                                op: BinaryOp::Add,
                                left: ValueId::new(1),
                                right: ValueId::new(1),
                            },
                            span: None,
                        }],
                        terminator: Terminator::Return {
                            value: Some(ValueId::new(2)),
                        },
                        span: None,
                    },
                ],
                span: None,
            }],
        };
        assert!(contains(&verify(&module), "does not dominate"));
    }

    #[test]
    fn rejects_edge_arity_and_return_type_mismatch() {
        let i32_ty = TypeId::new(0);
        let bool_ty = TypeId::new(1);
        let module = Module {
            types: vec![
                Type::Integer {
                    signed: true,
                    bits: 32,
                },
                Type::Bool,
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "bad_edge".to_owned(),
                linkage: Linkage::Internal,
                parameters: Vec::new(),
                result: bool_ty,
                blocks: vec![
                    Block {
                        id: BlockId::new(0),
                        parameters: Vec::new(),
                        instructions: Vec::new(),
                        terminator: Terminator::Jump {
                            target: BlockId::new(1),
                            arguments: Vec::new(),
                        },
                        span: None,
                    },
                    Block {
                        id: BlockId::new(1),
                        parameters: vec![crate::BlockParameter {
                            value: ValueId::new(0),
                            ty: i32_ty,
                        }],
                        instructions: Vec::new(),
                        terminator: Terminator::Return {
                            value: Some(ValueId::new(0)),
                        },
                        span: None,
                    },
                ],
                span: None,
            }],
        };
        let errors = verify(&module);
        assert!(contains(&errors, "argument count"));
        assert!(contains(&errors, "is required"));
    }

    fn scalar_module() -> Module {
        let i32_ty = TypeId::new(0);
        Module {
            types: vec![Type::Integer {
                signed: true,
                bits: 32,
            }],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "add_one".to_owned(),
                linkage: Linkage::Internal,
                parameters: vec![Parameter {
                    value: ValueId::new(0),
                    ty: i32_ty,
                    name: None,
                }],
                result: i32_ty,
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        Instruction {
                            result: Some(TypedValue {
                                value: ValueId::new(1),
                                ty: i32_ty,
                            }),
                            kind: InstructionKind::Constant(Constant::Integer { value: 1 }),
                            span: None,
                        },
                        Instruction {
                            result: Some(TypedValue {
                                value: ValueId::new(2),
                                ty: i32_ty,
                            }),
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
            }],
        }
    }

    fn contains(errors: &[super::VerificationError], text: &str) -> bool {
        errors.iter().any(|error| error.message.contains(text))
    }
}
