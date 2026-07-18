use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use inkwell::FloatPredicate;
use inkwell::IntPredicate;
use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::{Linkage as LlvmLinkage, Module as LlvmModule};
use inkwell::targets::TargetTriple;
use inkwell::types::BasicTypeEnum;
use inkwell::values::{
    AggregateValueEnum, BasicMetadataValueEnum, BasicValue, BasicValueEnum, FunctionValue,
    InstructionValue, MetadataValue, PhiValue, UnnamedAddress,
};
use jadren_jir::{
    BinaryOp, Block, CastOp, ComparePredicate, Constant, Function, Instruction, InstructionKind,
    Linkage, Module, Terminator, Type, TypeId, UnaryOp, ValueId,
};

use crate::debug::{DebugInfoConfig, DebugInfoError, DebugState, FunctionDebugInfo};
use crate::{LoweredTypeTable, TypeLowerError, TypeLoweringConfig, lower_types};

const BOUNDS_PANIC_SYMBOL: &str = "jadren_rt_bounds_panic_u64";

/// Failure while lowering verified JIR functions and control flow to LLVM IR.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodegenError {
    Type(TypeLowerError),
    Debug(DebugInfoError),
    InvalidName(String),
    DuplicateSymbol(String),
    MissingFunction(usize),
    MissingBlock(usize),
    MissingValue(ValueId),
    UnitValue(TypeId),
    UnsupportedInstruction(&'static str),
    ValueKind(&'static str),
    Builder(String),
    LlvmVerifier(String),
}

impl fmt::Display for CodegenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Type(error) => error.fmt(formatter),
            Self::Debug(error) => error.fmt(formatter),
            Self::InvalidName(name) => write!(formatter, "LLVM symbol contains NUL: {name:?}"),
            Self::DuplicateSymbol(name) => write!(formatter, "duplicate LLVM symbol: {name}"),
            Self::MissingFunction(id) => write!(formatter, "missing LLVM function @f{id}"),
            Self::MissingBlock(id) => write!(formatter, "missing LLVM block ^bb{id}"),
            Self::MissingValue(value) => {
                write!(formatter, "missing LLVM value %v{}", value.index())
            }
            Self::UnitValue(ty) => {
                write!(formatter, "Unit type %t{} used as LLVM value", ty.index())
            }
            Self::UnsupportedInstruction(kind) => {
                write!(formatter, "{kind} belongs to a later LLVM lowering task")
            }
            Self::ValueKind(message) => formatter.write_str(message),
            Self::Builder(message) => write!(formatter, "LLVM builder failed: {message}"),
            Self::LlvmVerifier(message) => write!(formatter, "LLVM verifier failed: {message}"),
        }
    }
}

impl Error for CodegenError {}

impl From<TypeLowerError> for CodegenError {
    fn from(error: TypeLowerError) -> Self {
        Self::Type(error)
    }
}

impl From<DebugInfoError> for CodegenError {
    fn from(error: DebugInfoError) -> Self {
        Self::Debug(error)
    }
}

/// Lowers function declarations, scalar values and SSA control flow, then runs LLVM's verifier.
pub fn lower_module<'ctx>(
    context: &'ctx Context,
    jir: &Module,
    module_name: &str,
    config: &TypeLoweringConfig,
) -> Result<LlvmModule<'ctx>, CodegenError> {
    lower_module_internal(context, jir, module_name, config, None)
}

/// Lowers verified JIR with source-accurate debug metadata, then runs LLVM's verifier.
pub fn lower_module_with_debug<'ctx>(
    context: &'ctx Context,
    jir: &Module,
    module_name: &str,
    config: &TypeLoweringConfig,
    debug_config: &DebugInfoConfig,
) -> Result<LlvmModule<'ctx>, CodegenError> {
    lower_module_internal(context, jir, module_name, config, Some(debug_config))
}

fn lower_module_internal<'ctx>(
    context: &'ctx Context,
    jir: &Module,
    module_name: &str,
    config: &TypeLoweringConfig,
    debug_config: Option<&DebugInfoConfig>,
) -> Result<LlvmModule<'ctx>, CodegenError> {
    validate_name(module_name)?;
    let types = lower_types(context, jir, config)?;
    let llvm = context.create_module(module_name);
    llvm.set_data_layout(&types.target_data().get_data_layout());
    llvm.set_triple(&TargetTriple::create(&config.target_triple));
    let debug = debug_config
        .map(|debug_config| DebugState::create(context, &llvm, config, debug_config))
        .transpose()?;

    let functions = declare_functions(&llvm, jir, &types)?;
    for function in &jir.functions {
        if function.linkage != Linkage::Import {
            let function_debug = debug
                .as_ref()
                .map(|debug| {
                    debug.create_function(context, function, functions[function.id.index()])
                })
                .transpose()?;
            lower_function(
                context,
                &llvm,
                jir,
                function,
                functions[function.id.index()],
                &functions,
                &types,
                debug.as_ref().zip(function_debug),
            )?;
        }
    }
    if let Some(debug) = &debug {
        debug.finalize();
    }
    llvm.verify()
        .map_err(|error| CodegenError::LlvmVerifier(error.to_string()))?;
    Ok(llvm)
}

fn declare_functions<'ctx>(
    llvm: &LlvmModule<'ctx>,
    jir: &Module,
    types: &LoweredTypeTable<'ctx>,
) -> Result<Vec<FunctionValue<'ctx>>, CodegenError> {
    let mut names = BTreeSet::new();
    let mut functions = Vec::with_capacity(jir.functions.len());
    for function in &jir.functions {
        let name = llvm_name(function);
        validate_name(&name)?;
        if name == BOUNDS_PANIC_SYMBOL {
            return Err(CodegenError::DuplicateSymbol(name));
        }
        if !names.insert(name.clone()) {
            return Err(CodegenError::DuplicateSymbol(name));
        }
        let linkage = match function.linkage {
            Linkage::Internal => LlvmLinkage::Internal,
            Linkage::Export | Linkage::Import => LlvmLinkage::External,
        };
        functions.push(llvm.add_function(&name, types.function_type(function)?, Some(linkage)));
    }
    Ok(functions)
}

fn llvm_name(function: &Function) -> String {
    match function.linkage {
        Linkage::Internal => format!("jadren.f{}.{}", function.id.index(), function.name),
        Linkage::Export | Linkage::Import => function.name.clone(),
    }
}

fn validate_name(name: &str) -> Result<(), CodegenError> {
    if name.contains('\0') {
        Err(CodegenError::InvalidName(name.to_owned()))
    } else {
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn lower_function<'ctx>(
    context: &'ctx Context,
    llvm: &LlvmModule<'ctx>,
    jir: &Module,
    function: &Function,
    llvm_function: FunctionValue<'ctx>,
    functions: &[FunctionValue<'ctx>],
    types: &LoweredTypeTable<'ctx>,
    debug: Option<(&DebugState<'ctx, '_>, FunctionDebugInfo<'ctx>)>,
) -> Result<(), CodegenError> {
    let builder = context.create_builder();
    let blocks = function
        .blocks
        .iter()
        .map(|block| context.append_basic_block(llvm_function, &format!("bb{}", block.id.index())))
        .collect::<Vec<_>>();
    let mut values = BTreeMap::new();
    let mut value_types = BTreeMap::new();
    let disjoint_metadata = DisjointMetadata::new(context, function);
    let c_aggregate_return = types.uses_c_aggregate_return(function);
    let c_aggregate_register_return = types.c_aggregate_register_type(function);
    let return_pointer = if c_aggregate_return {
        Some(
            llvm_function
                .get_first_param()
                .ok_or(CodegenError::ValueKind("missing aggregate return pointer"))?
                .into_pointer_value(),
        )
    } else {
        None
    };
    for (index, parameter) in function.parameters.iter().enumerate() {
        let value = llvm_function
            .get_nth_param(
                u32::try_from(index + usize::from(c_aggregate_return))
                    .expect("parameter index fits u32"),
            )
            .ok_or(CodegenError::MissingValue(parameter.value))?;
        if let Some(name) = &parameter.name {
            validate_name(name)?;
            value.set_name(name);
        }
        values.insert(parameter.value, value);
        value_types.insert(parameter.value, parameter.ty);
    }

    let mut phis = Vec::with_capacity(function.blocks.len());
    for block in &function.blocks {
        builder.position_at_end(blocks[block.id.index()]);
        set_debug_location(context, &builder, debug, block.span.or(function.span))?;
        let mut block_phis = Vec::with_capacity(block.parameters.len());
        for parameter in &block.parameters {
            let ty = basic_type(types, parameter.ty)?;
            let phi = builder
                .build_phi(ty, &format!("v{}", parameter.value.index()))
                .map_err(builder_error)?;
            values.insert(parameter.value, phi.as_basic_value());
            value_types.insert(parameter.value, parameter.ty);
            block_phis.push(phi);
        }
        phis.push(block_phis);
    }

    for block in &function.blocks {
        builder.position_at_end(blocks[block.id.index()]);
        for instruction in &block.instructions {
            set_debug_location(
                context,
                &builder,
                debug,
                instruction.span.or(block.span).or(function.span),
            )?;
            if let Some(result) = instruction.result {
                value_types.insert(result.value, result.ty);
            }
            if let Some((id, value)) = lower_instruction(
                context,
                llvm,
                &builder,
                jir,
                function.id.index(),
                instruction,
                &values,
                &value_types,
                functions,
                types,
                disjoint_metadata.as_ref(),
            )? {
                values.insert(id, value);
            }
        }
        let source = builder.get_insert_block().ok_or(CodegenError::ValueKind(
            "LLVM builder lost its insertion block",
        ))?;
        set_debug_location(context, &builder, debug, block.span.or(function.span))?;
        lower_terminator(
            &builder,
            function,
            block,
            source,
            &blocks,
            &phis,
            &values,
            return_pointer,
            c_aggregate_register_return,
        )?;
    }
    if let Some((debug, info)) = debug {
        let first_instruction = blocks
            .first()
            .and_then(|block| block.get_first_instruction())
            .ok_or(CodegenError::ValueKind(
                "debug function has no LLVM instruction",
            ))?;
        debug.insert_parameters(function, llvm_function, info, first_instruction, types)?;
    }
    Ok(())
}

struct DisjointMetadata<'ctx> {
    roots: BTreeMap<ValueId, ValueId>,
    alias_scope: BTreeMap<ValueId, MetadataValue<'ctx>>,
    noalias: BTreeMap<ValueId, MetadataValue<'ctx>>,
    alias_scope_kind: u32,
    noalias_kind: u32,
}

impl<'ctx> DisjointMetadata<'ctx> {
    fn new(context: &'ctx Context, function: &Function) -> Option<Self> {
        let roots = disjoint_scope_roots(function);
        let contract_roots: BTreeSet<_> = roots
            .iter()
            .filter_map(|(value, root)| (value == root).then_some(*root))
            .collect();
        if contract_roots.len() < 2 {
            return None;
        }
        let domain_name = context.metadata_string("jadren.disjoint.domain");
        let domain = context.metadata_node(&[domain_name.into()]);
        let scopes: BTreeMap<_, _> = contract_roots
            .iter()
            .map(|root| {
                let name = context.metadata_string(&format!("jadren.disjoint.{}", root.index()));
                let scope = context.metadata_node(&[name.into(), domain.into()]);
                (*root, scope)
            })
            .collect();
        let alias_scope: BTreeMap<_, _> = scopes
            .iter()
            .map(|(root, scope)| (*root, context.metadata_node(&[(*scope).into()])))
            .collect();
        let noalias: BTreeMap<_, _> = scopes
            .keys()
            .map(|root| {
                let values = scopes
                    .iter()
                    .filter_map(|(other, scope)| (*other != *root).then_some((*scope).into()))
                    .collect::<Vec<BasicMetadataValueEnum<'ctx>>>();
                (*root, context.metadata_node(&values))
            })
            .collect();
        Some(Self {
            roots,
            alias_scope,
            noalias,
            alias_scope_kind: context.get_kind_id("alias.scope"),
            noalias_kind: context.get_kind_id("noalias"),
        })
    }

    fn apply(
        &self,
        instruction: InstructionValue<'ctx>,
        pointer: ValueId,
    ) -> Result<(), CodegenError> {
        let Some(root) = self.roots.get(&pointer).copied() else {
            return Ok(());
        };
        let Some(alias_scope) = self.alias_scope.get(&root).copied() else {
            return Ok(());
        };
        instruction
            .set_metadata(alias_scope, self.alias_scope_kind)
            .map_err(builder_error)?;
        if let Some(noalias) = self.noalias.get(&root).copied() {
            instruction
                .set_metadata(noalias, self.noalias_kind)
                .map_err(builder_error)?;
        }
        Ok(())
    }
}

fn disjoint_scope_roots(function: &Function) -> BTreeMap<ValueId, ValueId> {
    let mut roots = BTreeMap::new();
    for block in &function.blocks {
        for instruction in &block.instructions {
            if let InstructionKind::AssumeNoAlias { left, right } = instruction.kind {
                roots.entry(left).or_insert(left);
                roots.entry(right).or_insert(right);
            }
        }
    }
    let mut storage = BTreeMap::<ValueId, ValueId>::new();
    for block in &function.blocks {
        for instruction in &block.instructions {
            match &instruction.kind {
                InstructionKind::Store { pointer, value, .. } => {
                    if let Some(root) = roots.get(value).copied() {
                        storage.insert(*pointer, root);
                    } else {
                        storage.remove(pointer);
                    }
                }
                InstructionKind::Load { pointer, .. } => {
                    if let Some(result) = instruction.result
                        && let Some(root) = storage.get(pointer).copied()
                    {
                        roots.insert(result.value, root);
                    }
                }
                InstructionKind::ExtractValue { aggregate, .. }
                | InstructionKind::Cast {
                    value: aggregate, ..
                }
                | InstructionKind::Offset {
                    base: aggregate, ..
                } => {
                    if let Some(result) = instruction.result
                        && let Some(root) = roots.get(aggregate).copied()
                    {
                        roots.insert(result.value, root);
                    }
                }
                InstructionKind::Select {
                    when_true,
                    when_false,
                    ..
                } => {
                    if let Some(result) = instruction.result
                        && roots.get(when_true) == roots.get(when_false)
                        && let Some(root) = roots.get(when_true).copied()
                    {
                        roots.insert(result.value, root);
                    }
                }
                _ => {}
            }
        }
    }
    roots
}

fn set_debug_location<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    debug: Option<(&DebugState<'ctx, '_>, FunctionDebugInfo<'ctx>)>,
    span: Option<jadren_source::Span>,
) -> Result<(), CodegenError> {
    match (debug, span) {
        (Some((debug, function)), Some(span)) => {
            builder.set_current_debug_location(debug.location(context, span, function)?);
        }
        _ => builder.unset_current_debug_location(),
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn lower_instruction<'ctx>(
    context: &'ctx Context,
    llvm: &LlvmModule<'ctx>,
    builder: &Builder<'ctx>,
    jir: &Module,
    function_id: usize,
    instruction: &Instruction,
    values: &BTreeMap<ValueId, BasicValueEnum<'ctx>>,
    value_types: &BTreeMap<ValueId, TypeId>,
    functions: &[FunctionValue<'ctx>],
    types: &LoweredTypeTable<'ctx>,
    disjoint_metadata: Option<&DisjointMetadata<'ctx>>,
) -> Result<Option<(ValueId, BasicValueEnum<'ctx>)>, CodegenError> {
    let result = instruction.result;
    let name = result.map_or_else(String::new, |value| format!("v{}", value.value.index()));
    let value = match &instruction.kind {
        InstructionKind::Constant(constant) => Some(lower_constant(
            context,
            builder,
            constant,
            result
                .ok_or(CodegenError::ValueKind("constant has no result"))?
                .ty,
            types,
            &name,
        )?),
        InstructionKind::Unary { op, operand } => Some(lower_unary(
            builder,
            *op,
            required_value(values, *operand)?,
            &name,
        )?),
        InstructionKind::Binary { op, left, right } => Some(lower_binary(
            builder,
            jir,
            result
                .ok_or(CodegenError::ValueKind("binary has no result"))?
                .ty,
            *op,
            required_value(values, *left)?,
            required_value(values, *right)?,
            &name,
        )?),
        InstructionKind::Compare {
            predicate,
            left,
            right,
        } => Some(lower_compare(
            builder,
            jir,
            *predicate,
            required_value(values, *left)?,
            required_value(values, *right)?,
            required_value_type(value_types, *left)?,
            &name,
        )?),
        InstructionKind::Cast { op, value, target } => Some(lower_cast(
            builder,
            jir,
            *op,
            required_value(values, *value)?,
            required_value_type(value_types, *value)?,
            *target,
            types,
            &name,
        )?),
        InstructionKind::Select {
            condition,
            when_true,
            when_false,
        } => {
            let condition = required_value(values, *condition)?;
            let BasicValueEnum::IntValue(condition) = condition else {
                return Err(CodegenError::ValueKind("select condition is not integer"));
            };
            Some(
                builder
                    .build_select(
                        condition,
                        required_value(values, *when_true)?,
                        required_value(values, *when_false)?,
                        &name,
                    )
                    .map_err(builder_error)?,
            )
        }
        InstructionKind::Call {
            function,
            arguments,
        } => {
            let callee = functions
                .get(function.index())
                .copied()
                .ok_or(CodegenError::MissingFunction(function.index()))?;
            let arguments = arguments
                .iter()
                .map(|argument| required_value(values, *argument).map(BasicMetadataValueEnum::from))
                .collect::<Result<Vec<_>, _>>()?;
            if let Some(callee_function) = jir.functions.get(function.index())
                && types.uses_c_aggregate_return(callee_function)
            {
                let aggregate = basic_type(types, callee_function.result)?;
                let result_pointer = builder
                    .build_alloca(aggregate, &format!("{name}.sret"))
                    .map_err(builder_error)?;
                let mut call_arguments = Vec::with_capacity(arguments.len() + 1);
                call_arguments.push(result_pointer.into());
                call_arguments.extend(arguments);
                builder
                    .build_call(callee, &call_arguments, &format!("{name}.call"))
                    .map_err(builder_error)?;
                return if result.is_some() {
                    Ok(Some((
                        result
                            .ok_or(CodegenError::ValueKind("aggregate call has no result"))?
                            .value,
                        builder
                            .build_load(aggregate, result_pointer, &name)
                            .map_err(builder_error)?,
                    )))
                } else {
                    Ok(None)
                };
            }
            if let Some(callee_function) = jir.functions.get(function.index())
                && let Some(register_type) = types.c_aggregate_register_type(callee_function)
            {
                let call = builder
                    .build_call(callee, &arguments, &format!("{name}.call"))
                    .map_err(builder_error)?;
                let packed = call
                    .try_as_basic_value()
                    .basic()
                    .ok_or(CodegenError::ValueKind(
                        "aggregate register call has no result",
                    ))?;
                return if let Some(result) = result {
                    let aggregate = basic_type(types, callee_function.result)?;
                    let result_pointer = builder
                        .build_alloca(register_type, &format!("{name}.register"))
                        .map_err(builder_error)?;
                    builder
                        .build_store(result_pointer, packed)
                        .map_err(builder_error)?;
                    Ok(Some((
                        result.value,
                        builder
                            .build_load(aggregate, result_pointer, &name)
                            .map_err(builder_error)?,
                    )))
                } else {
                    Ok(None)
                };
            }
            let call = builder
                .build_call(callee, &arguments, &name)
                .map_err(builder_error)?;
            call.try_as_basic_value().basic()
        }
        InstructionKind::FunctionAddress { function } => {
            let callee = functions
                .get(function.index())
                .copied()
                .ok_or(CodegenError::MissingFunction(function.index()))?;
            Some(callee.as_global_value().as_pointer_value().into())
        }
        InstructionKind::IndirectCall { callee, arguments } => {
            let callee_type = required_value_type(value_types, *callee)?;
            let function_type = types
                .function_pointer_type(jir, callee_type)
                .map_err(CodegenError::Type)?;
            let BasicValueEnum::PointerValue(callee) = required_value(values, *callee)? else {
                return Err(CodegenError::ValueKind(
                    "indirect call callee is not an LLVM pointer",
                ));
            };
            let arguments = arguments
                .iter()
                .map(|argument| required_value(values, *argument).map(BasicMetadataValueEnum::from))
                .collect::<Result<Vec<_>, _>>()?;
            let call = builder
                .build_indirect_call(function_type, callee, &arguments, &name)
                .map_err(builder_error)?;
            call.try_as_basic_value().basic()
        }
        InstructionKind::StringLiteral { utf8 } => Some(lower_string_literal(
            context,
            llvm,
            builder,
            utf8,
            result
                .ok_or(CodegenError::ValueKind("string literal has no result"))?
                .ty,
            function_id,
            result.expect("checked string result").value,
            types,
            &name,
        )?),
        InstructionKind::Aggregate { elements } => Some(build_aggregate(
            builder,
            basic_type(
                types,
                result
                    .ok_or(CodegenError::ValueKind("aggregate has no result"))?
                    .ty,
            )?,
            &elements
                .iter()
                .map(|element| required_value(values, *element))
                .collect::<Result<Vec<_>, _>>()?,
            &name,
        )?),
        InstructionKind::ExtractValue { aggregate, index } => Some(extract_aggregate_value(
            builder,
            required_value(values, *aggregate)?,
            *index,
            &name,
        )?),
        InstructionKind::EnumConstruct { variant, fields } => Some(lower_enum_construct(
            context,
            builder,
            jir,
            result
                .ok_or(CodegenError::ValueKind("enum construct has no result"))?
                .ty,
            *variant,
            &fields
                .iter()
                .map(|field| required_value(values, *field))
                .collect::<Result<Vec<_>, _>>()?,
            types,
            &name,
        )?),
        InstructionKind::EnumTag { value } => Some(extract_aggregate_value(
            builder,
            required_value(values, *value)?,
            0,
            &name,
        )?),
        InstructionKind::EnumExtract {
            value,
            variant,
            field,
        } => Some(lower_enum_extract(
            context,
            builder,
            jir,
            required_value(values, *value)?,
            required_value_type(value_types, *value)?,
            *variant,
            *field,
            types,
            &name,
        )?),
        InstructionKind::ExtractElement { aggregate, index } => Some(lower_extract_element(
            builder,
            required_value(values, *aggregate)?,
            required_value_type(value_types, *aggregate)?,
            required_value(values, *index)?,
            types,
            &name,
        )?),
        InstructionKind::StackAlloc { ty, count } => {
            let allocated = basic_type(types, *ty)?;
            let pointer = if let Some(count) = count {
                let count = required_value(values, *count)?;
                let BasicValueEnum::IntValue(count) = count else {
                    return Err(CodegenError::ValueKind(
                        "stack allocation count is not integer",
                    ));
                };
                builder
                    .build_array_alloca(allocated, count, &name)
                    .map_err(builder_error)?
            } else {
                builder
                    .build_alloca(allocated, &name)
                    .map_err(builder_error)?
            };
            let allocation = pointer
                .as_instruction_value()
                .ok_or(CodegenError::ValueKind(
                    "alloca did not produce an instruction",
                ))?;
            allocation
                .set_alignment(types.target_data().get_abi_alignment(&allocated))
                .map_err(builder_error)?;
            Some(pointer.into())
        }
        InstructionKind::Load {
            pointer,
            alignment,
            volatile,
        } => {
            let pointer_id = *pointer;
            let pointer = required_value(values, pointer_id)?;
            let BasicValueEnum::PointerValue(pointer) = pointer else {
                return Err(CodegenError::ValueKind("load operand is not a pointer"));
            };
            let loaded = builder
                .build_load(
                    basic_type(
                        types,
                        result
                            .ok_or(CodegenError::ValueKind("load has no result"))?
                            .ty,
                    )?,
                    pointer,
                    &name,
                )
                .map_err(builder_error)?;
            set_memory_properties(loaded, *alignment, *volatile)?;
            if let Some(metadata) = disjoint_metadata
                && let Some(instruction) = loaded.as_instruction_value()
            {
                metadata.apply(instruction, pointer_id)?;
            }
            Some(loaded)
        }
        InstructionKind::Store {
            pointer,
            value,
            alignment,
            volatile,
        } => {
            let pointer_id = *pointer;
            let pointer = required_value(values, pointer_id)?;
            let BasicValueEnum::PointerValue(pointer) = pointer else {
                return Err(CodegenError::ValueKind(
                    "store destination is not a pointer",
                ));
            };
            let store = builder
                .build_store(pointer, required_value(values, *value)?)
                .map_err(builder_error)?;
            store.set_alignment(*alignment).map_err(builder_error)?;
            store.set_volatile(*volatile).map_err(builder_error)?;
            if let Some(metadata) = disjoint_metadata {
                metadata.apply(store, pointer_id)?;
            }
            None
        }
        InstructionKind::BoundsCheck { index, length } => {
            lower_bounds_check(
                context,
                llvm,
                builder,
                required_value(values, *index)?,
                required_value(values, *length)?,
                function_id,
                *index,
                *length,
            )?;
            None
        }
        InstructionKind::VectorBoundsCheck {
            index,
            length,
            lanes,
        } => {
            lower_vector_bounds_check(
                context,
                llvm,
                builder,
                required_value(values, *index)?,
                required_value(values, *length)?,
                *lanes,
                function_id,
                *index,
                *length,
            )?;
            None
        }
        InstructionKind::AssumeNoAlias { .. } => None,
        InstructionKind::Drop { value } => {
            // Drop is ownership bookkeeping in the 0.1 JIR. Values do not
            // carry a destructor callback yet; region lifetime is represented
            // separately by RegionDestroy. Still resolve the SSA operand so a
            // malformed drop cannot silently bypass value validation.
            required_value(values, *value)?;
            None
        }
        InstructionKind::Builtin(_)
        | InstructionKind::RegionAlloc { .. }
        | InstructionKind::RegionCreate
        | InstructionKind::RegionDestroy { .. } => {
            return Err(CodegenError::UnsupportedInstruction(
                "memory/bounds lowering",
            ));
        }
        InstructionKind::Offset { base, indices } => Some(lower_offset(
            context,
            builder,
            jir,
            required_value(values, *base)?,
            required_value_type(value_types, *base)?,
            result
                .ok_or(CodegenError::ValueKind("offset has no result"))?
                .ty,
            &indices
                .iter()
                .map(|index| required_value(values, *index))
                .collect::<Result<Vec<_>, _>>()?,
            types,
            &name,
        )?),
        InstructionKind::VectorSplat { value, lanes } => Some(lower_vector_splat(
            builder,
            basic_type(
                types,
                result
                    .ok_or(CodegenError::ValueKind("vector splat has no result"))?
                    .ty,
            )?,
            required_value(values, *value)?,
            *lanes,
            &name,
        )?),
        InstructionKind::VectorBinary { op, left, right } => Some(lower_vector_binary(
            builder,
            jir,
            result
                .ok_or(CodegenError::ValueKind("vector binary has no result"))?
                .ty,
            *op,
            required_value(values, *left)?,
            required_value(values, *right)?,
            &name,
        )?),
        InstructionKind::VectorExtract { vector, lane } => Some(lower_vector_extract(
            builder,
            required_value(values, *vector)?,
            required_value(values, *lane)?,
            &name,
        )?),
        InstructionKind::VectorInsert {
            vector,
            lane,
            value,
        } => Some(lower_vector_insert(
            builder,
            required_value(values, *vector)?,
            required_value(values, *lane)?,
            required_value(values, *value)?,
            &name,
        )?),
    };
    match (result, value) {
        (Some(result), Some(value)) => Ok(Some((result.value, value))),
        (None, None) => Ok(None),
        (Some(_), None) => Err(CodegenError::ValueKind(
            "value instruction produced no LLVM value",
        )),
        (None, Some(_)) => Err(CodegenError::ValueKind(
            "side-effect instruction produced a value",
        )),
    }
}

fn set_memory_properties(
    value: BasicValueEnum<'_>,
    alignment: u32,
    volatile: bool,
) -> Result<(), CodegenError> {
    let instruction = value.as_instruction_value().ok_or(CodegenError::ValueKind(
        "load did not produce an instruction",
    ))?;
    instruction
        .set_alignment(alignment)
        .map_err(builder_error)?;
    instruction.set_volatile(volatile).map_err(builder_error)
}

fn build_aggregate<'ctx>(
    builder: &Builder<'ctx>,
    ty: BasicTypeEnum<'ctx>,
    elements: &[BasicValueEnum<'ctx>],
    name: &str,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    let mut aggregate = as_aggregate_value(ty.const_zero())?;
    for (index, element) in elements.iter().enumerate() {
        aggregate = builder
            .build_insert_value(
                aggregate,
                *element,
                u32::try_from(index).expect("verified aggregate index fits u32"),
                &format!("{name}.field{index}"),
            )
            .map_err(builder_error)?;
    }
    Ok(aggregate_as_basic(aggregate))
}

fn extract_aggregate_value<'ctx>(
    builder: &Builder<'ctx>,
    aggregate: BasicValueEnum<'ctx>,
    index: u32,
    name: &str,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    builder
        .build_extract_value(as_aggregate_value(aggregate)?, index, name)
        .map_err(builder_error)
}

fn as_aggregate_value(value: BasicValueEnum<'_>) -> Result<AggregateValueEnum<'_>, CodegenError> {
    match value {
        BasicValueEnum::ArrayValue(value) => Ok(AggregateValueEnum::ArrayValue(value)),
        BasicValueEnum::StructValue(value) => Ok(AggregateValueEnum::StructValue(value)),
        _ => Err(CodegenError::ValueKind("LLVM value is not an aggregate")),
    }
}

fn aggregate_as_basic(aggregate: AggregateValueEnum<'_>) -> BasicValueEnum<'_> {
    match aggregate {
        AggregateValueEnum::ArrayValue(value) => value.into(),
        AggregateValueEnum::StructValue(value) => value.into(),
    }
}

#[allow(clippy::too_many_arguments)]
fn lower_string_literal<'ctx>(
    context: &'ctx Context,
    llvm: &LlvmModule<'ctx>,
    builder: &Builder<'ctx>,
    utf8: &[u8],
    result_ty: TypeId,
    function_id: usize,
    value_id: ValueId,
    types: &LoweredTypeTable<'ctx>,
    name: &str,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    let bytes = context.const_string(utf8, false);
    let global = llvm.add_global(
        bytes.get_type(),
        None,
        &format!("jadren.string.f{function_id}.v{}", value_id.index()),
    );
    global.set_initializer(&bytes);
    global.set_constant(true);
    global.set_linkage(LlvmLinkage::Private);
    global.set_unnamed_address(UnnamedAddress::Global);
    global.set_alignment(1);

    let result_type = basic_type(types, result_ty)?;
    let BasicTypeEnum::StructType(string_type) = result_type else {
        return Err(CodegenError::ValueKind("string result is not a struct"));
    };
    let length_type = string_type
        .get_field_types()
        .get(1)
        .copied()
        .ok_or(CodegenError::ValueKind("string layout has no length field"))?;
    let BasicTypeEnum::IntType(length_type) = length_type else {
        return Err(CodegenError::ValueKind(
            "string length field is not integer",
        ));
    };
    build_aggregate(
        builder,
        result_type,
        &[
            global.as_pointer_value().into(),
            length_type.const_int(utf8.len() as u64, false).into(),
        ],
        name,
    )
}

#[allow(clippy::too_many_arguments)]
fn lower_enum_construct<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    jir: &Module,
    enum_ty: TypeId,
    variant: u32,
    fields: &[BasicValueEnum<'ctx>],
    types: &LoweredTypeTable<'ctx>,
    name: &str,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    let enum_type = enum_struct_type(types, enum_ty)?;
    let layout = types
        .enum_layout(enum_ty)
        .ok_or(CodegenError::ValueKind("enum has no target layout"))?;
    let variant_types = enum_variant_types(jir, enum_ty, variant)?;
    let payload_types = variant_types
        .iter()
        .map(|ty| basic_type(types, *ty))
        .collect::<Result<Vec<_>, _>>()?;
    let payload_type = context.struct_type(&payload_types, false);

    let storage = builder
        .build_alloca(enum_type, &format!("{name}.storage"))
        .map_err(builder_error)?;
    let enum_alignment = types.target_data().get_abi_alignment(&enum_type);
    storage
        .as_instruction_value()
        .ok_or(CodegenError::ValueKind("enum alloca is not an instruction"))?
        .set_alignment(enum_alignment)
        .map_err(builder_error)?;
    let initialize = builder
        .build_store(storage, enum_type.const_zero())
        .map_err(builder_error)?;
    initialize
        .set_alignment(enum_alignment)
        .map_err(builder_error)?;

    let tag_pointer = builder
        .build_struct_gep(
            enum_type,
            storage,
            layout.tag_field,
            &format!("{name}.tag.ptr"),
        )
        .map_err(builder_error)?;
    let tag_store = builder
        .build_store(
            tag_pointer,
            context.i32_type().const_int(u64::from(variant), false),
        )
        .map_err(builder_error)?;
    tag_store.set_alignment(4).map_err(builder_error)?;

    if !fields.is_empty() {
        let payload = build_aggregate(
            builder,
            payload_type.into(),
            fields,
            &format!("{name}.payload"),
        )?;
        let payload_pointer = builder
            .build_struct_gep(
                enum_type,
                storage,
                layout.payload_field,
                &format!("{name}.payload.ptr"),
            )
            .map_err(builder_error)?;
        let payload_store = builder
            .build_store(payload_pointer, payload)
            .map_err(builder_error)?;
        payload_store
            .set_alignment(layout.payload_alignment)
            .map_err(builder_error)?;
    }

    let value = builder
        .build_load(enum_type, storage, name)
        .map_err(builder_error)?;
    set_memory_properties(value, enum_alignment, false)?;
    Ok(value)
}

#[allow(clippy::too_many_arguments)]
fn lower_enum_extract<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    jir: &Module,
    value: BasicValueEnum<'ctx>,
    enum_ty: TypeId,
    variant: u32,
    field: u32,
    types: &LoweredTypeTable<'ctx>,
    name: &str,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    let enum_type = enum_struct_type(types, enum_ty)?;
    let layout = types
        .enum_layout(enum_ty)
        .ok_or(CodegenError::ValueKind("enum has no target layout"))?;
    let variant_types = enum_variant_types(jir, enum_ty, variant)?;
    let payload_types = variant_types
        .iter()
        .map(|ty| basic_type(types, *ty))
        .collect::<Result<Vec<_>, _>>()?;
    let payload_type = context.struct_type(&payload_types, false);
    if usize::try_from(field).map_or(true, |index| index >= payload_types.len()) {
        return Err(CodegenError::ValueKind(
            "enum payload field is out of range",
        ));
    }

    let storage = builder
        .build_alloca(enum_type, &format!("{name}.storage"))
        .map_err(builder_error)?;
    let enum_alignment = types.target_data().get_abi_alignment(&enum_type);
    storage
        .as_instruction_value()
        .ok_or(CodegenError::ValueKind("enum alloca is not an instruction"))?
        .set_alignment(enum_alignment)
        .map_err(builder_error)?;
    let store = builder.build_store(storage, value).map_err(builder_error)?;
    store.set_alignment(enum_alignment).map_err(builder_error)?;
    let payload_pointer = builder
        .build_struct_gep(
            enum_type,
            storage,
            layout.payload_field,
            &format!("{name}.payload.ptr"),
        )
        .map_err(builder_error)?;
    let payload = builder
        .build_load(payload_type, payload_pointer, &format!("{name}.payload"))
        .map_err(builder_error)?;
    set_memory_properties(payload, layout.payload_alignment, false)?;
    extract_aggregate_value(builder, payload, field, name)
}

fn enum_struct_type<'ctx>(
    types: &LoweredTypeTable<'ctx>,
    enum_ty: TypeId,
) -> Result<inkwell::types::StructType<'ctx>, CodegenError> {
    let BasicTypeEnum::StructType(enum_type) = basic_type(types, enum_ty)? else {
        return Err(CodegenError::ValueKind("enum LLVM type is not a struct"));
    };
    Ok(enum_type)
}

fn enum_variant_types(
    jir: &Module,
    enum_ty: TypeId,
    variant: u32,
) -> Result<&[TypeId], CodegenError> {
    let variants = match jir.types.get(enum_ty.index()) {
        Some(Type::Enum { variants } | Type::NominalEnum { variants, .. }) => variants,
        _ => return Err(CodegenError::ValueKind("JIR type is not an enum")),
    };
    variants
        .get(variant as usize)
        .map(Vec::as_slice)
        .ok_or(CodegenError::ValueKind("enum variant is out of range"))
}

#[allow(clippy::too_many_arguments)]
fn lower_offset<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    jir: &Module,
    base: BasicValueEnum<'ctx>,
    base_ty: TypeId,
    result_ty: TypeId,
    indices: &[BasicValueEnum<'ctx>],
    types: &LoweredTypeTable<'ctx>,
    name: &str,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    let BasicValueEnum::PointerValue(base) = base else {
        return Err(CodegenError::ValueKind("offset base is not a pointer"));
    };
    let pointee = match jir.types.get(base_ty.index()) {
        Some(Type::Pointer { pointee, .. }) => *pointee,
        _ => {
            return Err(CodegenError::ValueKind(
                "offset base JIR type is not a pointer",
            ));
        }
    };
    let llvm_pointee = basic_type(types, pointee)?;
    let result_pointee = match jir.types.get(result_ty.index()) {
        Some(Type::Pointer { pointee, .. }) => *pointee,
        _ => {
            return Err(CodegenError::ValueKind(
                "offset result JIR type is not a pointer",
            ));
        }
    };
    let indices = indices
        .iter()
        .map(|index| match index {
            BasicValueEnum::IntValue(index) => Ok(*index),
            _ => Err(CodegenError::ValueKind("offset index is not integer")),
        })
        .collect::<Result<Vec<_>, _>>()?;

    let pointer = match jir.types.get(pointee.index()) {
        Some(Type::Struct { .. } | Type::NominalStruct { .. }) => {
            let field = indices
                .as_slice()
                .first()
                .filter(|_| indices.len() == 1)
                .and_then(|field| field.get_zero_extended_constant())
                .and_then(|field| u32::try_from(field).ok());
            let expected = field.and_then(|field| match jir.types.get(pointee.index()) {
                Some(Type::Struct { fields } | Type::NominalStruct { fields, .. }) => {
                    fields.get(field as usize).copied()
                }
                _ => None,
            });
            if let Some(field) = field
                && expected == Some(result_pointee)
            {
                builder
                    .build_struct_gep(llvm_pointee, base, field, name)
                    .map_err(builder_error)?
            } else if pointee == result_pointee {
                build_verified_gep(builder, llvm_pointee, base, &indices, name)?
            } else {
                return Err(CodegenError::ValueKind(
                    "struct offset result pointee differs from field type",
                ));
            }
        }
        Some(Type::Array { .. }) => {
            if !matches!(jir.types.get(pointee.index()), Some(Type::Array { element, .. }) if *element == result_pointee)
            {
                return Err(CodegenError::ValueKind(
                    "array offset result pointee differs from element type",
                ));
            }
            let mut llvm_indices = Vec::with_capacity(indices.len() + 1);
            llvm_indices.push(context.i32_type().const_zero());
            llvm_indices.extend(indices);
            build_verified_gep(builder, llvm_pointee, base, &llvm_indices, name)?
        }
        Some(_) => {
            if pointee != result_pointee {
                return Err(CodegenError::ValueKind(
                    "pointer offset changes scalar pointee type",
                ));
            }
            build_verified_gep(builder, llvm_pointee, base, &indices, name)?
        }
        None => return Err(CodegenError::MissingValue(ValueId::new(pointee.index()))),
    };
    Ok(pointer.into())
}

fn lower_extract_element<'ctx>(
    builder: &Builder<'ctx>,
    aggregate: BasicValueEnum<'ctx>,
    aggregate_ty: TypeId,
    index: BasicValueEnum<'ctx>,
    types: &LoweredTypeTable<'ctx>,
    name: &str,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    let BasicValueEnum::ArrayValue(array) = aggregate else {
        return Err(CodegenError::ValueKind(
            "extract_element source is not an array",
        ));
    };
    let BasicValueEnum::IntValue(index) = index else {
        return Err(CodegenError::ValueKind(
            "extract_element index is not integer",
        ));
    };
    let array_type = array.get_type();
    let element_type = match types.get(aggregate_ty).and_then(|ty| ty.as_basic()) {
        Some(BasicTypeEnum::ArrayType(array)) => array.get_element_type(),
        _ => {
            return Err(CodegenError::ValueKind(
                "extract_element JIR type is not an array",
            ));
        }
    };
    let storage = builder
        .build_alloca(array_type, &format!("{name}.storage"))
        .map_err(builder_error)?;
    let array_alignment = types.target_data().get_abi_alignment(&array_type);
    storage
        .as_instruction_value()
        .ok_or(CodegenError::ValueKind(
            "array alloca is not an instruction",
        ))?
        .set_alignment(array_alignment)
        .map_err(builder_error)?;
    let store = builder.build_store(storage, array).map_err(builder_error)?;
    store
        .set_alignment(array_alignment)
        .map_err(builder_error)?;
    let element_pointer = build_verified_gep(
        builder,
        array_type.into(),
        storage,
        &[context_i32_zero(array_type), index],
        &format!("{name}.ptr"),
    )?;
    let loaded = builder
        .build_load(element_type, element_pointer, name)
        .map_err(builder_error)?;
    set_memory_properties(
        loaded,
        types.target_data().get_abi_alignment(&element_type),
        false,
    )?;
    Ok(loaded)
}

fn context_i32_zero(array_type: inkwell::types::ArrayType<'_>) -> inkwell::values::IntValue<'_> {
    array_type.get_context().i32_type().const_zero()
}

// JADREN-UNSAFE-AUDIT: LLVMBuildGEP2 is called only after the verifier-shaped
// aggregate/index contract is established by the lowering path below.
#[allow(unsafe_code)]
fn build_verified_gep<'ctx>(
    builder: &Builder<'ctx>,
    pointee: BasicTypeEnum<'ctx>,
    base: inkwell::values::PointerValue<'ctx>,
    indices: &[inkwell::values::IntValue<'ctx>],
    name: &str,
) -> Result<inkwell::values::PointerValue<'ctx>, CodegenError> {
    // SAFETY: the mandatory JIR verifier proves that `base` is a pointer and every index is
    // integer. The caller derives `pointee` from that pointer's canonical JIR pointee and adds
    // the leading zero required by aggregate arrays. This satisfies LLVMBuildGEP2's type/index
    // shape contract; a runtime bounds proof is emitted separately by JAD-606D.
    unsafe { builder.build_gep(pointee, base, indices, name) }.map_err(builder_error)
}

#[allow(clippy::too_many_arguments)]
fn lower_bounds_check<'ctx>(
    context: &'ctx Context,
    llvm: &LlvmModule<'ctx>,
    builder: &Builder<'ctx>,
    index: BasicValueEnum<'ctx>,
    length: BasicValueEnum<'ctx>,
    function_id: usize,
    index_id: ValueId,
    length_id: ValueId,
) -> Result<(), CodegenError> {
    let BasicValueEnum::IntValue(index) = index else {
        return Err(CodegenError::ValueKind("bounds index is not integer"));
    };
    let BasicValueEnum::IntValue(length) = length else {
        return Err(CodegenError::ValueKind("bounds length is not integer"));
    };
    if index.get_type().get_bit_width() > 64 || length.get_type().get_bit_width() > 64 {
        return Err(CodegenError::ValueKind(
            "bounds operands exceed the runtime u64 panic ABI",
        ));
    }
    let current = builder.get_insert_block().ok_or(CodegenError::ValueKind(
        "bounds check has no insertion block",
    ))?;
    let function = current.get_parent().ok_or(CodegenError::ValueKind(
        "bounds check block has no function",
    ))?;
    let suffix = format!(
        "f{function_id}.v{}.v{}",
        index_id.index(),
        length_id.index()
    );
    let success = context.append_basic_block(function, &format!("bounds.ok.{suffix}"));
    let failure = context.append_basic_block(function, &format!("bounds.fail.{suffix}"));
    let condition = builder
        .build_int_compare(
            IntPredicate::ULT,
            index,
            length,
            &format!("bounds.valid.{suffix}"),
        )
        .map_err(builder_error)?;
    builder
        .build_conditional_branch(condition, success, failure)
        .map_err(builder_error)?;

    builder.position_at_end(failure);
    let u64_type = context.i64_type();
    let index_u64 = builder
        .build_int_z_extend_or_bit_cast(index, u64_type, &format!("bounds.index.{suffix}"))
        .map_err(builder_error)?;
    let length_u64 = builder
        .build_int_z_extend_or_bit_cast(length, u64_type, &format!("bounds.length.{suffix}"))
        .map_err(builder_error)?;
    let panic_type = context
        .void_type()
        .fn_type(&[u64_type.into(), u64_type.into()], false);
    let panic = llvm.get_function(BOUNDS_PANIC_SYMBOL).unwrap_or_else(|| {
        llvm.add_function(BOUNDS_PANIC_SYMBOL, panic_type, Some(LlvmLinkage::External))
    });
    if panic.get_type() != panic_type {
        return Err(CodegenError::ValueKind(
            "bounds panic symbol has an incompatible signature",
        ));
    }
    let noreturn_kind = Attribute::get_named_enum_kind_id("noreturn");
    if noreturn_kind == 0 {
        return Err(CodegenError::ValueKind(
            "LLVM does not provide the noreturn attribute",
        ));
    }
    panic.add_attribute(
        AttributeLoc::Function,
        context.create_enum_attribute(noreturn_kind, 0),
    );
    builder
        .build_call(panic, &[index_u64.into(), length_u64.into()], "")
        .map_err(builder_error)?;
    builder.build_unreachable().map_err(builder_error)?;
    builder.position_at_end(success);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn lower_vector_bounds_check<'ctx>(
    context: &'ctx Context,
    llvm: &LlvmModule<'ctx>,
    builder: &Builder<'ctx>,
    index: BasicValueEnum<'ctx>,
    length: BasicValueEnum<'ctx>,
    lanes: u16,
    function_id: usize,
    index_id: ValueId,
    length_id: ValueId,
) -> Result<(), CodegenError> {
    let BasicValueEnum::IntValue(index) = index else {
        return Err(CodegenError::ValueKind(
            "vector bounds index is not integer",
        ));
    };
    let BasicValueEnum::IntValue(length) = length else {
        return Err(CodegenError::ValueKind(
            "vector bounds length is not integer",
        ));
    };
    let bits = index.get_type().get_bit_width();
    if bits != length.get_type().get_bit_width() || bits > 64 {
        return Err(CodegenError::ValueKind(
            "vector bounds operands exceed the runtime u64 panic ABI",
        ));
    }
    if lanes == 0 {
        return Err(CodegenError::ValueKind(
            "vector bounds check requires at least one lane",
        ));
    }
    let current = builder.get_insert_block().ok_or(CodegenError::ValueKind(
        "vector bounds check has no insertion block",
    ))?;
    let function = current.get_parent().ok_or(CodegenError::ValueKind(
        "vector bounds check block has no function",
    ))?;
    let suffix = format!(
        "f{function_id}.v{}.v{}.l{lanes}",
        index_id.index(),
        length_id.index()
    );
    let success = context.append_basic_block(function, &format!("vector-bounds.ok.{suffix}"));
    let failure = context.append_basic_block(function, &format!("vector-bounds.fail.{suffix}"));
    let lane_count = index.get_type().const_int(u64::from(lanes), false);
    let length_has_lanes = builder
        .build_int_compare(
            IntPredicate::UGE,
            length,
            lane_count,
            &format!("vector-bounds.length-ge.{suffix}"),
        )
        .map_err(builder_error)?;
    let remaining = builder
        .build_int_sub(
            length,
            lane_count,
            &format!("vector-bounds.remaining.{suffix}"),
        )
        .map_err(builder_error)?;
    let index_fits = builder
        .build_int_compare(
            IntPredicate::ULE,
            index,
            remaining,
            &format!("vector-bounds.index-le.{suffix}"),
        )
        .map_err(builder_error)?;
    let condition = builder
        .build_and(
            length_has_lanes,
            index_fits,
            &format!("vector-bounds.valid.{suffix}"),
        )
        .map_err(builder_error)?;
    builder
        .build_conditional_branch(condition, success, failure)
        .map_err(builder_error)?;

    builder.position_at_end(failure);
    let u64_type = context.i64_type();
    let index_u64 = builder
        .build_int_z_extend_or_bit_cast(index, u64_type, &format!("vector-bounds.index.{suffix}"))
        .map_err(builder_error)?;
    let length_u64 = builder
        .build_int_z_extend_or_bit_cast(length, u64_type, &format!("vector-bounds.length.{suffix}"))
        .map_err(builder_error)?;
    let panic_type = context
        .void_type()
        .fn_type(&[u64_type.into(), u64_type.into()], false);
    let panic = llvm.get_function(BOUNDS_PANIC_SYMBOL).unwrap_or_else(|| {
        llvm.add_function(BOUNDS_PANIC_SYMBOL, panic_type, Some(LlvmLinkage::External))
    });
    if panic.get_type() != panic_type {
        return Err(CodegenError::ValueKind(
            "bounds panic symbol has an incompatible signature",
        ));
    }
    let noreturn_kind = Attribute::get_named_enum_kind_id("noreturn");
    if noreturn_kind == 0 {
        return Err(CodegenError::ValueKind(
            "LLVM does not provide the noreturn attribute",
        ));
    }
    panic.add_attribute(
        AttributeLoc::Function,
        context.create_enum_attribute(noreturn_kind, 0),
    );
    builder
        .build_call(panic, &[index_u64.into(), length_u64.into()], "")
        .map_err(builder_error)?;
    builder.build_unreachable().map_err(builder_error)?;
    builder.position_at_end(success);
    Ok(())
}

fn lower_constant<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    constant: &Constant,
    ty: TypeId,
    types: &LoweredTypeTable<'ctx>,
    name: &str,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    let llvm_type = basic_type(types, ty)?;
    match (constant, llvm_type) {
        (Constant::Bool(value), BasicTypeEnum::IntType(int)) => {
            Ok(int.const_int(u64::from(*value), false).into())
        }
        (Constant::Integer { value }, BasicTypeEnum::IntType(int)) => {
            Ok(integer_constant(int, *value).into())
        }
        (Constant::FloatBits { bits }, BasicTypeEnum::FloatType(float)) => {
            let width = types.target_data().get_bit_size(&float);
            let width = u32::try_from(width)
                .map_err(|_| CodegenError::ValueKind("float width exceeds u32"))?;
            let integer = context
                .custom_width_int_type(
                    std::num::NonZeroU32::new(width)
                        .ok_or(CodegenError::ValueKind("zero-width float"))?,
                )
                .map_err(|message| CodegenError::Builder(message.to_owned()))?
                .const_int(*bits, false);
            builder
                .build_bit_cast(integer, float, name)
                .map_err(builder_error)
        }
        (Constant::Zero, ty) => Ok(ty.const_zero()),
        (Constant::Null, BasicTypeEnum::PointerType(pointer)) => Ok(pointer.const_null().into()),
        _ => Err(CodegenError::ValueKind(
            "constant kind does not match LLVM type",
        )),
    }
}

fn integer_constant<'ctx>(
    ty: inkwell::types::IntType<'ctx>,
    value: i128,
) -> inkwell::values::IntValue<'ctx> {
    let width = ty.get_bit_width();
    let word_count = usize::try_from(width.div_ceil(64)).expect("u32 word count fits usize");
    let mut words = vec![if value < 0 { u64::MAX } else { 0 }; word_count];
    let bits = value as u128;
    if let Some(low) = words.get_mut(0) {
        *low = bits as u64;
    }
    if let Some(high) = words.get_mut(1) {
        *high = (bits >> 64) as u64;
    }
    if let Some(last) = words.last_mut() {
        let remainder = width % 64;
        if remainder != 0 {
            *last &= (1_u64 << remainder) - 1;
        }
    }
    ty.const_int_arbitrary_precision(&words)
}

fn lower_unary<'ctx>(
    builder: &Builder<'ctx>,
    op: UnaryOp,
    operand: BasicValueEnum<'ctx>,
    name: &str,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    match (op, operand) {
        (UnaryOp::Negate, BasicValueEnum::IntValue(value)) => builder
            .build_int_neg(value, name)
            .map(Into::into)
            .map_err(builder_error),
        (UnaryOp::Negate, BasicValueEnum::FloatValue(value)) => builder
            .build_float_neg(value, name)
            .map(Into::into)
            .map_err(builder_error),
        (UnaryOp::Not | UnaryOp::BitNot, BasicValueEnum::IntValue(value)) => builder
            .build_not(value, name)
            .map(Into::into)
            .map_err(builder_error),
        _ => Err(CodegenError::ValueKind("unsupported unary LLVM value kind")),
    }
}

fn lower_binary<'ctx>(
    builder: &Builder<'ctx>,
    jir: &Module,
    ty: TypeId,
    op: BinaryOp,
    left: BasicValueEnum<'ctx>,
    right: BasicValueEnum<'ctx>,
    name: &str,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    match (left, right) {
        (BasicValueEnum::IntValue(left), BasicValueEnum::IntValue(right)) => {
            let signed = is_signed(jir, ty);
            let value = match op {
                BinaryOp::Add => builder.build_int_add(left, right, name),
                BinaryOp::Subtract => builder.build_int_sub(left, right, name),
                BinaryOp::Multiply => builder.build_int_mul(left, right, name),
                BinaryOp::Divide if signed => builder.build_int_signed_div(left, right, name),
                BinaryOp::Divide => builder.build_int_unsigned_div(left, right, name),
                BinaryOp::Remainder if signed => builder.build_int_signed_rem(left, right, name),
                BinaryOp::Remainder => builder.build_int_unsigned_rem(left, right, name),
                BinaryOp::BitAnd => builder.build_and(left, right, name),
                BinaryOp::BitOr => builder.build_or(left, right, name),
                BinaryOp::BitXor => builder.build_xor(left, right, name),
                BinaryOp::ShiftLeft => builder.build_left_shift(left, right, name),
                BinaryOp::ShiftRight => builder.build_right_shift(left, right, signed, name),
            };
            value.map(Into::into).map_err(builder_error)
        }
        (BasicValueEnum::FloatValue(left), BasicValueEnum::FloatValue(right)) => {
            let value = match op {
                BinaryOp::Add => builder.build_float_add(left, right, name),
                BinaryOp::Subtract => builder.build_float_sub(left, right, name),
                BinaryOp::Multiply => builder.build_float_mul(left, right, name),
                BinaryOp::Divide => builder.build_float_div(left, right, name),
                BinaryOp::Remainder => builder.build_float_rem(left, right, name),
                _ => return Err(CodegenError::ValueKind("bit operation requires integers")),
            };
            value.map(Into::into).map_err(builder_error)
        }
        _ => Err(CodegenError::ValueKind(
            "binary operands have different LLVM kinds",
        )),
    }
}

fn lower_vector_splat<'ctx>(
    builder: &Builder<'ctx>,
    vector_type: BasicTypeEnum<'ctx>,
    value: BasicValueEnum<'ctx>,
    lanes: u16,
    name: &str,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    let BasicTypeEnum::VectorType(vector_type) = vector_type else {
        return Err(CodegenError::ValueKind(
            "vector_splat result is not an LLVM vector",
        ));
    };
    if vector_type.get_size() != u32::from(lanes) {
        return Err(CodegenError::ValueKind(
            "vector_splat lane count differs from LLVM result type",
        ));
    }
    let mut vector = vector_type.get_undef();
    let index_type = vector_type.get_context().i32_type();
    for lane in 0..lanes {
        vector = builder
            .build_insert_element(
                vector,
                value,
                index_type.const_int(u64::from(lane), false),
                &format!("{name}.lane{lane}"),
            )
            .map_err(builder_error)?;
    }
    Ok(vector.into())
}

fn lower_vector_binary<'ctx>(
    builder: &Builder<'ctx>,
    jir: &Module,
    vector_type: TypeId,
    op: BinaryOp,
    left: BasicValueEnum<'ctx>,
    right: BasicValueEnum<'ctx>,
    name: &str,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    let (BasicValueEnum::VectorValue(left), BasicValueEnum::VectorValue(right)) = (left, right)
    else {
        return Err(CodegenError::ValueKind(
            "vector_binary operands are not LLVM vectors",
        ));
    };
    let signed =
        vector_element_type(jir, vector_type).is_some_and(|element| is_signed(jir, element));
    let value = match vector_element_type(jir, vector_type)
        .and_then(|element| jir.types.get(element.index()))
    {
        Some(Type::Float { .. }) => match op {
            BinaryOp::Add => builder.build_float_add(left, right, name),
            BinaryOp::Subtract => builder.build_float_sub(left, right, name),
            BinaryOp::Multiply => builder.build_float_mul(left, right, name),
            BinaryOp::Divide => builder.build_float_div(left, right, name),
            BinaryOp::Remainder => builder.build_float_rem(left, right, name),
            _ => {
                return Err(CodegenError::ValueKind(
                    "vector bit operation requires integers",
                ));
            }
        },
        Some(Type::Integer { .. } | Type::Bool) => match op {
            BinaryOp::Add => builder.build_int_add(left, right, name),
            BinaryOp::Subtract => builder.build_int_sub(left, right, name),
            BinaryOp::Multiply => builder.build_int_mul(left, right, name),
            BinaryOp::Divide if signed => builder.build_int_signed_div(left, right, name),
            BinaryOp::Divide => builder.build_int_unsigned_div(left, right, name),
            BinaryOp::Remainder if signed => builder.build_int_signed_rem(left, right, name),
            BinaryOp::Remainder => builder.build_int_unsigned_rem(left, right, name),
            BinaryOp::BitAnd => builder.build_and(left, right, name),
            BinaryOp::BitOr => builder.build_or(left, right, name),
            BinaryOp::BitXor => builder.build_xor(left, right, name),
            BinaryOp::ShiftLeft => builder.build_left_shift(left, right, name),
            BinaryOp::ShiftRight => builder.build_right_shift(left, right, signed, name),
        },
        _ => return Err(CodegenError::ValueKind("unsupported vector element type")),
    };
    value.map(Into::into).map_err(builder_error)
}

fn lower_vector_extract<'ctx>(
    builder: &Builder<'ctx>,
    vector: BasicValueEnum<'ctx>,
    lane: BasicValueEnum<'ctx>,
    name: &str,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    let BasicValueEnum::VectorValue(vector) = vector else {
        return Err(CodegenError::ValueKind(
            "vector_extract source is not a vector",
        ));
    };
    let BasicValueEnum::IntValue(lane) = lane else {
        return Err(CodegenError::ValueKind(
            "vector_extract lane is not an integer",
        ));
    };
    builder
        .build_extract_element(vector, lane, name)
        .map_err(builder_error)
}

fn lower_vector_insert<'ctx>(
    builder: &Builder<'ctx>,
    vector: BasicValueEnum<'ctx>,
    lane: BasicValueEnum<'ctx>,
    value: BasicValueEnum<'ctx>,
    name: &str,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    let BasicValueEnum::VectorValue(vector) = vector else {
        return Err(CodegenError::ValueKind(
            "vector_insert source is not a vector",
        ));
    };
    let BasicValueEnum::IntValue(lane) = lane else {
        return Err(CodegenError::ValueKind(
            "vector_insert lane is not an integer",
        ));
    };
    builder
        .build_insert_element(vector, value, lane, name)
        .map(Into::into)
        .map_err(builder_error)
}

fn vector_element_type(jir: &Module, vector_type: TypeId) -> Option<TypeId> {
    match jir.types.get(vector_type.index()) {
        Some(Type::Vector { element, .. }) => Some(*element),
        _ => None,
    }
}

fn lower_compare<'ctx>(
    builder: &Builder<'ctx>,
    jir: &Module,
    predicate: ComparePredicate,
    left: BasicValueEnum<'ctx>,
    right: BasicValueEnum<'ctx>,
    operand_type: TypeId,
    name: &str,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    match (left, right) {
        (BasicValueEnum::IntValue(left), BasicValueEnum::IntValue(right)) => {
            let signed = is_signed(jir, operand_type);
            let predicate = match (predicate, signed) {
                (ComparePredicate::Equal, _) => IntPredicate::EQ,
                (ComparePredicate::NotEqual, _) => IntPredicate::NE,
                (ComparePredicate::Less, true) => IntPredicate::SLT,
                (ComparePredicate::LessEqual, true) => IntPredicate::SLE,
                (ComparePredicate::Greater, true) => IntPredicate::SGT,
                (ComparePredicate::GreaterEqual, true) => IntPredicate::SGE,
                (ComparePredicate::Less, false) => IntPredicate::ULT,
                (ComparePredicate::LessEqual, false) => IntPredicate::ULE,
                (ComparePredicate::Greater, false) => IntPredicate::UGT,
                (ComparePredicate::GreaterEqual, false) => IntPredicate::UGE,
            };
            builder
                .build_int_compare(predicate, left, right, name)
                .map(Into::into)
                .map_err(builder_error)
        }
        (BasicValueEnum::FloatValue(left), BasicValueEnum::FloatValue(right)) => {
            let predicate = match predicate {
                ComparePredicate::Equal => FloatPredicate::OEQ,
                ComparePredicate::NotEqual => FloatPredicate::UNE,
                ComparePredicate::Less => FloatPredicate::OLT,
                ComparePredicate::LessEqual => FloatPredicate::OLE,
                ComparePredicate::Greater => FloatPredicate::OGT,
                ComparePredicate::GreaterEqual => FloatPredicate::OGE,
            };
            builder
                .build_float_compare(predicate, left, right, name)
                .map(Into::into)
                .map_err(builder_error)
        }
        _ => Err(CodegenError::ValueKind(
            "compare operands have different LLVM kinds",
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn lower_cast<'ctx>(
    builder: &Builder<'ctx>,
    jir: &Module,
    op: CastOp,
    value: BasicValueEnum<'ctx>,
    source: TypeId,
    target: TypeId,
    types: &LoweredTypeTable<'ctx>,
    name: &str,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    let target_type = basic_type(types, target)?;
    match (op, value, target_type) {
        (
            CastOp::IntegerExtend,
            BasicValueEnum::IntValue(value),
            BasicTypeEnum::IntType(target),
        ) => {
            let result = if is_signed(jir, source) {
                builder.build_int_s_extend(value, target, name)
            } else {
                builder.build_int_z_extend(value, target, name)
            };
            result.map(Into::into).map_err(builder_error)
        }
        (
            CastOp::IntegerTruncate,
            BasicValueEnum::IntValue(value),
            BasicTypeEnum::IntType(target),
        ) => builder
            .build_int_truncate(value, target, name)
            .map(Into::into)
            .map_err(builder_error),
        (
            CastOp::IntegerToFloat,
            BasicValueEnum::IntValue(value),
            BasicTypeEnum::FloatType(target),
        ) => {
            let result = if is_signed(jir, source) {
                builder.build_signed_int_to_float(value, target, name)
            } else {
                builder.build_unsigned_int_to_float(value, target, name)
            };
            result.map(Into::into).map_err(builder_error)
        }
        (
            CastOp::FloatToInteger,
            BasicValueEnum::FloatValue(value),
            BasicTypeEnum::IntType(target_int),
        ) => {
            let result = if is_signed(jir, target) {
                builder.build_float_to_signed_int(value, target_int, name)
            } else {
                builder.build_float_to_unsigned_int(value, target_int, name)
            };
            result.map(Into::into).map_err(builder_error)
        }
        (
            CastOp::FloatExtend,
            BasicValueEnum::FloatValue(value),
            BasicTypeEnum::FloatType(target),
        ) => builder
            .build_float_ext(value, target, name)
            .map(Into::into)
            .map_err(builder_error),
        (
            CastOp::FloatTruncate,
            BasicValueEnum::FloatValue(value),
            BasicTypeEnum::FloatType(target),
        ) => builder
            .build_float_trunc(value, target, name)
            .map(Into::into)
            .map_err(builder_error),
        (CastOp::Bitcast, value, target) => builder
            .build_bit_cast(value, target, name)
            .map_err(builder_error),
        (
            CastOp::PointerCast,
            BasicValueEnum::PointerValue(value),
            BasicTypeEnum::PointerType(target),
        ) => builder
            .build_pointer_cast(value, target, name)
            .map(Into::into)
            .map_err(builder_error),
        _ => Err(CodegenError::ValueKind(
            "cast kind does not match LLVM types",
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn lower_terminator<'ctx>(
    builder: &Builder<'ctx>,
    function: &Function,
    block: &Block,
    source: BasicBlock<'ctx>,
    blocks: &[BasicBlock<'ctx>],
    phis: &[Vec<PhiValue<'ctx>>],
    values: &BTreeMap<ValueId, BasicValueEnum<'ctx>>,
    return_pointer: Option<inkwell::values::PointerValue<'ctx>>,
    return_register: Option<inkwell::types::IntType<'ctx>>,
) -> Result<(), CodegenError> {
    match &block.terminator {
        Terminator::Return { value } => {
            let value = value
                .map(|value| required_value(values, value))
                .transpose()?;
            if let Some(return_pointer) = return_pointer {
                let value = value.ok_or(CodegenError::ValueKind(
                    "aggregate return is missing its value",
                ))?;
                builder
                    .build_store(return_pointer, value)
                    .map_err(builder_error)?;
                builder.build_return(None).map_err(builder_error)?;
            } else if let Some(return_type) = return_register {
                let value = value.ok_or(CodegenError::ValueKind(
                    "aggregate register return is missing its value",
                ))?;
                let storage = builder
                    .build_alloca(return_type, "cabi.return")
                    .map_err(builder_error)?;
                builder
                    .build_store(storage, return_type.const_zero())
                    .map_err(builder_error)?;
                builder.build_store(storage, value).map_err(builder_error)?;
                let packed = builder
                    .build_load(return_type, storage, "cabi.return.packed")
                    .map_err(builder_error)?;
                builder.build_return(Some(&packed)).map_err(builder_error)?;
            } else {
                builder
                    .build_return(value.as_ref().map(|value| value as &dyn BasicValue<'ctx>))
                    .map_err(builder_error)?;
            }
        }
        Terminator::Jump { target, arguments } => {
            add_edge(function, source, *target, arguments, phis, values)?;
            builder
                .build_unconditional_branch(required_block(blocks, target.index())?)
                .map_err(builder_error)?;
        }
        Terminator::Branch {
            condition,
            then_target,
            then_arguments,
            else_target,
            else_arguments,
        } => {
            add_edge(function, source, *then_target, then_arguments, phis, values)?;
            add_edge(function, source, *else_target, else_arguments, phis, values)?;
            let condition = required_value(values, *condition)?;
            let BasicValueEnum::IntValue(condition) = condition else {
                return Err(CodegenError::ValueKind("branch condition is not integer"));
            };
            builder
                .build_conditional_branch(
                    condition,
                    required_block(blocks, then_target.index())?,
                    required_block(blocks, else_target.index())?,
                )
                .map_err(builder_error)?;
        }
        Terminator::Switch {
            discriminant,
            cases,
            default,
            default_arguments,
        } => {
            add_edge(function, source, *default, default_arguments, phis, values)?;
            let discriminant = required_value(values, *discriminant)?;
            let BasicValueEnum::IntValue(discriminant) = discriminant else {
                return Err(CodegenError::ValueKind(
                    "switch discriminant is not integer",
                ));
            };
            let mut llvm_cases = Vec::with_capacity(cases.len());
            for case in cases {
                add_edge(function, source, case.target, &case.arguments, phis, values)?;
                llvm_cases.push((
                    integer_constant(discriminant.get_type(), case.value),
                    required_block(blocks, case.target.index())?,
                ));
            }
            builder
                .build_switch(
                    discriminant,
                    required_block(blocks, default.index())?,
                    &llvm_cases,
                )
                .map_err(builder_error)?;
        }
        Terminator::Unreachable => {
            builder.build_unreachable().map_err(builder_error)?;
        }
    }
    Ok(())
}

fn add_edge<'ctx>(
    function: &Function,
    source: BasicBlock<'ctx>,
    target: jadren_jir::BlockId,
    arguments: &[ValueId],
    phis: &[Vec<PhiValue<'ctx>>],
    values: &BTreeMap<ValueId, BasicValueEnum<'ctx>>,
) -> Result<(), CodegenError> {
    let parameters = function
        .blocks
        .get(target.index())
        .ok_or(CodegenError::MissingBlock(target.index()))?;
    let target_phis = phis
        .get(target.index())
        .ok_or(CodegenError::MissingBlock(target.index()))?;
    for ((argument, _parameter), phi) in arguments
        .iter()
        .zip(&parameters.parameters)
        .zip(target_phis)
    {
        let value = required_value(values, *argument)?;
        phi.add_incoming(&[(&value, source)]);
    }
    Ok(())
}

fn required_value<'ctx>(
    values: &BTreeMap<ValueId, BasicValueEnum<'ctx>>,
    value: ValueId,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    values
        .get(&value)
        .copied()
        .ok_or(CodegenError::MissingValue(value))
}

fn required_block<'ctx>(
    blocks: &[BasicBlock<'ctx>],
    index: usize,
) -> Result<BasicBlock<'ctx>, CodegenError> {
    blocks
        .get(index)
        .copied()
        .ok_or(CodegenError::MissingBlock(index))
}

fn basic_type<'ctx>(
    types: &LoweredTypeTable<'ctx>,
    ty: TypeId,
) -> Result<BasicTypeEnum<'ctx>, CodegenError> {
    types
        .get(ty)
        .ok_or(CodegenError::UnitValue(ty))?
        .as_basic()
        .ok_or(CodegenError::UnitValue(ty))
}

fn is_signed(jir: &Module, ty: TypeId) -> bool {
    matches!(
        jir.types.get(ty.index()),
        Some(Type::Integer { signed: true, .. })
    )
}

fn required_value_type(
    value_types: &BTreeMap<ValueId, TypeId>,
    value: ValueId,
) -> Result<TypeId, CodegenError> {
    value_types
        .get(&value)
        .copied()
        .ok_or(CodegenError::MissingValue(value))
}

fn builder_error(error: impl fmt::Display) -> CodegenError {
    CodegenError::Builder(error.to_string())
}

#[cfg(test)]
mod tests {
    use inkwell::context::Context;
    use jadren_jir::{
        AddressSpace, BinaryOp, Block, BlockId, BlockParameter, CastOp, ComparePredicate, Constant,
        Function, FunctionId, Instruction, InstructionKind, Linkage, Module, Parameter, SwitchCase,
        Terminator, Type, TypeId, TypedValue, ValueId,
    };

    use crate::{ObjectOptions, TypeLoweringConfig, emit_assembly, lower_module};

    #[test]
    fn lowers_ownership_only_drop_without_emitting_destructor() {
        let jir = Module {
            types: vec![
                Type::Unit,
                Type::Integer {
                    signed: false,
                    bits: 64,
                },
                Type::Struct {
                    fields: vec![TypeId::new(1)],
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "drop_aggregate".to_owned(),
                linkage: Linkage::Export,
                parameters: vec![Parameter {
                    value: ValueId::new(0),
                    ty: TypeId::new(2),
                    name: Some("value".to_owned()),
                }],
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![Instruction {
                        result: None,
                        kind: InstructionKind::Drop {
                            value: ValueId::new(0),
                        },
                        span: None,
                    }],
                    terminator: Terminator::Return { value: None },
                    span: None,
                }],
                span: None,
            }],
        };
        let context = Context::create();
        let llvm = lower_module(
            &context,
            &jir,
            "drop_aggregate",
            &TypeLoweringConfig::default(),
        )
        .expect("ownership-only aggregate drop lowers");
        let text = llvm.print_to_string().to_string();
        assert!(text.contains("define void @drop_aggregate"), "{text}");
        assert!(llvm.verify().is_ok());
    }

    #[test]
    fn lowers_pointer_dereference_load_chain_to_verified_llvm() {
        let jir = Module {
            types: vec![
                Type::Unit,
                Type::Integer {
                    signed: true,
                    bits: 32,
                },
                Type::Pointer {
                    pointee: TypeId::new(1),
                    address_space: AddressSpace::Generic,
                },
                Type::Pointer {
                    pointee: TypeId::new(2),
                    address_space: AddressSpace::Stack,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "deref_pointer".to_owned(),
                linkage: Linkage::Export,
                parameters: vec![Parameter {
                    value: ValueId::new(0),
                    ty: TypeId::new(3),
                    name: Some("pointer_slot".to_owned()),
                }],
                result: TypeId::new(1),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        value_instruction(
                            1,
                            2,
                            InstructionKind::Load {
                                pointer: ValueId::new(0),
                                alignment: 8,
                                volatile: false,
                            },
                        ),
                        value_instruction(
                            2,
                            1,
                            InstructionKind::Load {
                                pointer: ValueId::new(1),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                    ],
                    terminator: Terminator::Return {
                        value: Some(ValueId::new(2)),
                    },
                    span: None,
                }],
                span: None,
            }],
        };
        let context = Context::create();
        let llvm = lower_module(
            &context,
            &jir,
            "deref_pointer",
            &TypeLoweringConfig::default(),
        )
        .expect("pointer dereference load chain must lower");
        let text = llvm.print_to_string().to_string();
        assert!(text.contains("define i32 @deref_pointer"), "{text}");
        assert!(text.contains("load ptr"), "{text}");
        assert!(text.contains("load i32"), "{text}");
        assert!(llvm.verify().is_ok());
    }

    #[test]
    fn lowers_branches_block_arguments_calls_and_returns_to_verified_llvm() {
        let types = vec![
            Type::Unit,
            Type::Bool,
            Type::Integer {
                signed: true,
                bits: 32,
            },
        ];
        let imported = Function {
            id: FunctionId::new(0),
            name: "external_adjust".to_owned(),
            linkage: Linkage::Import,
            parameters: vec![Parameter {
                value: ValueId::new(0),
                ty: TypeId::new(2),
                name: None,
            }],
            result: TypeId::new(2),
            blocks: Vec::new(),
            span: None,
        };
        let choose = Function {
            id: FunctionId::new(1),
            name: "choose".to_owned(),
            linkage: Linkage::Export,
            parameters: vec![
                Parameter {
                    value: ValueId::new(0),
                    ty: TypeId::new(2),
                    name: Some("value".to_owned()),
                },
                Parameter {
                    value: ValueId::new(1),
                    ty: TypeId::new(1),
                    name: Some("condition".to_owned()),
                },
            ],
            result: TypeId::new(2),
            blocks: vec![
                Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: Vec::new(),
                    terminator: Terminator::Branch {
                        condition: ValueId::new(1),
                        then_target: BlockId::new(1),
                        then_arguments: vec![ValueId::new(0)],
                        else_target: BlockId::new(2),
                        else_arguments: vec![ValueId::new(0)],
                    },
                    span: None,
                },
                arithmetic_block(1, 2, 3, 4, BinaryOp::Add, 1),
                arithmetic_block(2, 5, 6, 7, BinaryOp::Subtract, 2),
                Block {
                    id: BlockId::new(3),
                    parameters: vec![BlockParameter {
                        value: ValueId::new(8),
                        ty: TypeId::new(2),
                    }],
                    instructions: vec![Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(9),
                            ty: TypeId::new(2),
                        }),
                        kind: InstructionKind::Call {
                            function: FunctionId::new(0),
                            arguments: vec![ValueId::new(8)],
                        },
                        span: None,
                    }],
                    terminator: Terminator::Return {
                        value: Some(ValueId::new(9)),
                    },
                    span: None,
                },
            ],
            span: None,
        };
        let jir = Module {
            types,
            functions: vec![imported, choose],
        };
        let context = Context::create();
        let llvm = lower_module(&context, &jir, "cfg_test", &TypeLoweringConfig::default())
            .expect("valid LLVM module");
        let text = llvm.print_to_string().to_string();

        assert!(text.contains("define i32 @choose(i32 %value, i1 %condition)"));
        assert!(text.contains("phi i32"));
        assert!(text.contains("call i32 @external_adjust"));
        assert!(text.contains("br i1 %condition"));
        assert!(llvm.verify().is_ok());
    }

    #[test]
    fn lowers_x86_64_baseline_vector_operations_without_target_specific_intrinsics() {
        let jir = Module {
            types: vec![
                Type::Unit,
                Type::Float { bits: 32 },
                Type::Integer {
                    signed: false,
                    bits: 32,
                },
                Type::Vector {
                    element: TypeId::new(1),
                    lanes: 4,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "baseline_vector".to_owned(),
                linkage: Linkage::Export,
                parameters: vec![
                    Parameter {
                        value: ValueId::new(0),
                        ty: TypeId::new(3),
                        name: Some("input".to_owned()),
                    },
                    Parameter {
                        value: ValueId::new(1),
                        ty: TypeId::new(2),
                        name: Some("lane".to_owned()),
                    },
                    Parameter {
                        value: ValueId::new(2),
                        ty: TypeId::new(1),
                        name: Some("scalar".to_owned()),
                    },
                ],
                result: TypeId::new(3),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        value_instruction(
                            3,
                            3,
                            InstructionKind::VectorSplat {
                                value: ValueId::new(2),
                                lanes: 4,
                            },
                        ),
                        value_instruction(
                            4,
                            3,
                            InstructionKind::VectorBinary {
                                op: BinaryOp::Add,
                                left: ValueId::new(0),
                                right: ValueId::new(3),
                            },
                        ),
                        value_instruction(
                            5,
                            1,
                            InstructionKind::VectorExtract {
                                vector: ValueId::new(4),
                                lane: ValueId::new(1),
                            },
                        ),
                        value_instruction(
                            6,
                            3,
                            InstructionKind::VectorInsert {
                                vector: ValueId::new(4),
                                lane: ValueId::new(1),
                                value: ValueId::new(5),
                            },
                        ),
                    ],
                    terminator: Terminator::Return {
                        value: Some(ValueId::new(6)),
                    },
                    span: None,
                }],
                span: None,
            }],
        };
        let context = Context::create();
        let llvm = lower_module(
            &context,
            &jir,
            "baseline_vector",
            &TypeLoweringConfig::default(),
        )
        .expect("baseline vector LLVM module");
        let text = llvm.print_to_string().to_string();
        assert!(text.contains("insertelement <4 x float>"), "{text}");
        assert!(text.contains("fadd <4 x float>"), "{text}");
        assert!(text.contains("extractelement <4 x float>"), "{text}");
        assert!(llvm.verify().is_ok());

        let baseline =
            emit_assembly(&llvm, &ObjectOptions::x86_64_baseline()).expect("baseline assembly");
        let avx2 = emit_assembly(&llvm, &ObjectOptions::x86_64_avx2()).expect("AVX2 assembly");
        assert_ne!(baseline, avx2, "AVX2 must be a distinct target policy");
        assert!(
            String::from_utf8(avx2)
                .expect("AVX2 assembly is UTF-8")
                .contains("vaddps")
        );

        let arm_llvm = lower_module(
            &context,
            &jir,
            "baseline_vector_arm",
            &TypeLoweringConfig::aarch64_android(),
        )
        .expect("AArch64 vector LLVM module");
        let arm_scalar = emit_assembly(&arm_llvm, &ObjectOptions::aarch64_android())
            .expect("AArch64 scalar assembly");
        let arm_neon = emit_assembly(&arm_llvm, &ObjectOptions::aarch64_neon())
            .expect("AArch64 NEON assembly");
        let arm_neon_release = emit_assembly(&arm_llvm, &ObjectOptions::aarch64_neon_release())
            .expect("AArch64 NEON release assembly");
        let arm_scalar_text = String::from_utf8(arm_scalar).expect("AArch64 assembly is UTF-8");
        let arm_neon_text = String::from_utf8(arm_neon).expect("NEON assembly is UTF-8");
        let arm_neon_release_text =
            String::from_utf8(arm_neon_release).expect("NEON release assembly is UTF-8");
        assert!(arm_scalar_text.contains("fadd\tv"), "{arm_scalar_text}");
        assert!(arm_neon_text.contains("fadd\tv"), "{arm_neon_text}");
        assert!(
            arm_neon_release_text.contains("fadd\tv"),
            "{arm_neon_release_text}"
        );
        assert_ne!(
            arm_neon_text, arm_neon_release_text,
            "NEON release must apply a distinct LLVM cost/scheduling policy"
        );
    }

    #[test]
    fn lowers_vector_memory_load_store_to_packed_llvm_values() {
        let jir = Module {
            types: vec![
                Type::Unit,
                Type::Float { bits: 32 },
                Type::Integer {
                    signed: false,
                    bits: 32,
                },
                Type::Vector {
                    element: TypeId::new(1),
                    lanes: 4,
                },
                Type::Pointer {
                    pointee: TypeId::new(3),
                    address_space: AddressSpace::Stack,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "vector_memory".to_owned(),
                linkage: Linkage::Export,
                parameters: Vec::new(),
                result: TypeId::new(3),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        value_instruction(
                            0,
                            4,
                            InstructionKind::StackAlloc {
                                ty: TypeId::new(3),
                                count: None,
                            },
                        ),
                        value_instruction(
                            1,
                            1,
                            InstructionKind::Constant(Constant::FloatBits {
                                bits: u64::from(1.0f32.to_bits()),
                            }),
                        ),
                        value_instruction(
                            2,
                            3,
                            InstructionKind::VectorSplat {
                                value: ValueId::new(1),
                                lanes: 4,
                            },
                        ),
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
                        value_instruction(
                            3,
                            3,
                            InstructionKind::Load {
                                pointer: ValueId::new(0),
                                alignment: 16,
                                volatile: false,
                            },
                        ),
                        value_instruction(
                            4,
                            3,
                            InstructionKind::VectorBinary {
                                op: BinaryOp::Add,
                                left: ValueId::new(3),
                                right: ValueId::new(2),
                            },
                        ),
                        Instruction {
                            result: None,
                            kind: InstructionKind::Store {
                                pointer: ValueId::new(0),
                                value: ValueId::new(4),
                                alignment: 16,
                                volatile: false,
                            },
                            span: None,
                        },
                        value_instruction(
                            5,
                            3,
                            InstructionKind::Load {
                                pointer: ValueId::new(0),
                                alignment: 16,
                                volatile: false,
                            },
                        ),
                    ],
                    terminator: Terminator::Return {
                        value: Some(ValueId::new(5)),
                    },
                    span: None,
                }],
                span: None,
            }],
        };
        let context = Context::create();
        let llvm = lower_module(
            &context,
            &jir,
            "vector_memory",
            &TypeLoweringConfig::default(),
        )
        .expect("vector memory LLVM module");
        let text = llvm.print_to_string().to_string();
        assert!(text.contains("store <4 x float>"), "{text}");
        assert!(text.contains("load <4 x float>"), "{text}");
        assert!(text.contains("fadd <4 x float>"), "{text}");
        assert!(llvm.verify().is_ok());

        let avx2 = emit_assembly(&llvm, &ObjectOptions::x86_64_avx2())
            .expect("AVX2 vector memory assembly");
        let avx2 = String::from_utf8(avx2).expect("AVX2 assembly is UTF-8");
        assert!(avx2.contains("vaddps"), "{avx2}");
    }

    #[test]
    fn lowers_float8_memory_to_avx2_packed_llvm_values() {
        let jir = Module {
            types: vec![
                Type::Unit,
                Type::Float { bits: 32 },
                Type::Vector {
                    element: TypeId::new(1),
                    lanes: 8,
                },
                Type::Pointer {
                    pointee: TypeId::new(2),
                    address_space: AddressSpace::Stack,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "float8_memory".to_owned(),
                linkage: Linkage::Export,
                parameters: vec![Parameter {
                    value: ValueId::new(1),
                    ty: TypeId::new(2),
                    name: Some("input".to_owned()),
                }],
                result: TypeId::new(2),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        value_instruction(
                            0,
                            3,
                            InstructionKind::StackAlloc {
                                ty: TypeId::new(2),
                                count: None,
                            },
                        ),
                        Instruction {
                            result: None,
                            kind: InstructionKind::Store {
                                pointer: ValueId::new(0),
                                value: ValueId::new(1),
                                alignment: 16,
                                volatile: false,
                            },
                            span: None,
                        },
                        value_instruction(
                            2,
                            2,
                            InstructionKind::Load {
                                pointer: ValueId::new(0),
                                alignment: 16,
                                volatile: false,
                            },
                        ),
                        value_instruction(
                            3,
                            2,
                            InstructionKind::VectorBinary {
                                op: BinaryOp::Add,
                                left: ValueId::new(2),
                                right: ValueId::new(1),
                            },
                        ),
                    ],
                    terminator: Terminator::Return {
                        value: Some(ValueId::new(3)),
                    },
                    span: None,
                }],
                span: None,
            }],
        };
        let context = Context::create();
        let llvm = lower_module(
            &context,
            &jir,
            "float8_memory",
            &TypeLoweringConfig::default(),
        )
        .expect("Float8 memory LLVM module");
        let text = llvm.print_to_string().to_string();
        assert!(text.contains("store <8 x float>"), "{text}");
        assert!(text.contains("load <8 x float>"), "{text}");
        assert!(text.contains("fadd <8 x float>"), "{text}");
        assert!(llvm.verify().is_ok());

        let avx2 = emit_assembly(&llvm, &ObjectOptions::x86_64_avx2())
            .expect("AVX2 Float8 memory assembly");
        let avx2 = String::from_utf8(avx2).expect("AVX2 assembly is UTF-8");
        assert!(avx2.contains("vaddps"), "{avx2}");

        let arm_llvm = lower_module(
            &context,
            &jir,
            "float8_memory_arm",
            &TypeLoweringConfig::aarch64_android(),
        )
        .expect("AArch64 Float8 memory LLVM module");
        let neon = emit_assembly(&arm_llvm, &ObjectOptions::aarch64_neon_release())
            .expect("AArch64 NEON Float8 memory assembly");
        let neon = String::from_utf8(neon).expect("AArch64 NEON assembly is UTF-8");
        assert!(neon.contains("fadd\tv"), "{neon}");
    }

    #[test]
    fn lowers_signed_cast_compare_select_and_switch_with_target_identity() {
        let jir = Module {
            types: vec![
                Type::Unit,
                Type::Bool,
                Type::Integer {
                    signed: true,
                    bits: 32,
                },
                Type::Integer {
                    signed: true,
                    bits: 64,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "select_wide".to_owned(),
                linkage: Linkage::Export,
                parameters: vec![
                    Parameter {
                        value: ValueId::new(0),
                        ty: TypeId::new(2),
                        name: Some("left".to_owned()),
                    },
                    Parameter {
                        value: ValueId::new(1),
                        ty: TypeId::new(2),
                        name: Some("right".to_owned()),
                    },
                ],
                result: TypeId::new(3),
                blocks: vec![
                    Block {
                        id: BlockId::new(0),
                        parameters: Vec::new(),
                        instructions: vec![
                            value_instruction(
                                2,
                                1,
                                InstructionKind::Compare {
                                    predicate: ComparePredicate::Less,
                                    left: ValueId::new(0),
                                    right: ValueId::new(1),
                                },
                            ),
                            value_instruction(
                                3,
                                3,
                                InstructionKind::Cast {
                                    op: CastOp::IntegerExtend,
                                    value: ValueId::new(0),
                                    target: TypeId::new(3),
                                },
                            ),
                            value_instruction(
                                4,
                                3,
                                InstructionKind::Cast {
                                    op: CastOp::IntegerExtend,
                                    value: ValueId::new(1),
                                    target: TypeId::new(3),
                                },
                            ),
                            value_instruction(
                                5,
                                3,
                                InstructionKind::Select {
                                    condition: ValueId::new(2),
                                    when_true: ValueId::new(3),
                                    when_false: ValueId::new(4),
                                },
                            ),
                        ],
                        terminator: Terminator::Switch {
                            discriminant: ValueId::new(0),
                            cases: vec![SwitchCase {
                                value: 0,
                                target: BlockId::new(1),
                                arguments: vec![ValueId::new(5)],
                            }],
                            default: BlockId::new(2),
                            default_arguments: vec![ValueId::new(5)],
                        },
                        span: None,
                    },
                    return_parameter_block(1, 6),
                    return_parameter_block(2, 7),
                ],
                span: None,
            }],
        };
        let context = Context::create();
        let llvm = lower_module(&context, &jir, "scalar_cfg", &TypeLoweringConfig::default())
            .expect("valid scalar LLVM module");
        let text = llvm.print_to_string().to_string();

        assert!(text.contains("target triple = \"x86_64-pc-windows-msvc\""));
        assert!(text.contains("icmp slt i32"));
        assert!(text.contains("sext i32"));
        assert!(text.contains("select i1"));
        assert!(text.contains("switch i32"));
        assert!(llvm.verify().is_ok());
    }

    #[test]
    fn lowers_stack_alloc_load_store_alignment_and_volatile() {
        let jir = Module {
            types: vec![
                Type::Unit,
                Type::Integer {
                    signed: true,
                    bits: 32,
                },
                Type::Pointer {
                    pointee: TypeId::new(1),
                    address_space: AddressSpace::Stack,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "allocate".to_owned(),
                linkage: Linkage::Internal,
                parameters: Vec::new(),
                result: TypeId::new(1),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        value_instruction(
                            0,
                            2,
                            InstructionKind::StackAlloc {
                                ty: TypeId::new(1),
                                count: None,
                            },
                        ),
                        value_instruction(
                            1,
                            1,
                            InstructionKind::Constant(Constant::Integer { value: 42 }),
                        ),
                        Instruction {
                            result: None,
                            kind: InstructionKind::Store {
                                pointer: ValueId::new(0),
                                value: ValueId::new(1),
                                alignment: 4,
                                volatile: true,
                            },
                            span: None,
                        },
                        value_instruction(
                            2,
                            1,
                            InstructionKind::Load {
                                pointer: ValueId::new(0),
                                alignment: 4,
                                volatile: true,
                            },
                        ),
                    ],
                    terminator: Terminator::Return {
                        value: Some(ValueId::new(2)),
                    },
                    span: None,
                }],
                span: None,
            }],
        };
        let context = Context::create();
        let llvm = lower_module(
            &context,
            &jir,
            "stack_memory",
            &TypeLoweringConfig::default(),
        )
        .expect("stack memory LLVM module");
        let text = llvm.print_to_string().to_string();
        assert!(text.contains("alloca i32, align 4"));
        assert!(text.contains("store volatile i32 42"));
        assert!(text.contains("load volatile i32"));
        assert!(llvm.verify().is_ok());
    }

    #[test]
    fn lowers_inline_aggregate_extract_and_utf8_string_storage() {
        let jir = Module {
            types: vec![
                Type::Unit,
                Type::Integer {
                    signed: false,
                    bits: 8,
                },
                Type::Pointer {
                    pointee: TypeId::new(1),
                    address_space: AddressSpace::Global,
                },
                Type::Integer {
                    signed: false,
                    bits: 64,
                },
                Type::Struct {
                    fields: vec![TypeId::new(2), TypeId::new(3)],
                },
                Type::Integer {
                    signed: true,
                    bits: 32,
                },
                Type::Array {
                    element: TypeId::new(5),
                    length: 2,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "aggregate_string".to_owned(),
                linkage: Linkage::Internal,
                parameters: vec![
                    Parameter {
                        value: ValueId::new(0),
                        ty: TypeId::new(5),
                        name: Some("left".to_owned()),
                    },
                    Parameter {
                        value: ValueId::new(1),
                        ty: TypeId::new(5),
                        name: Some("right".to_owned()),
                    },
                ],
                result: TypeId::new(5),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        value_instruction(
                            2,
                            6,
                            InstructionKind::Aggregate {
                                elements: vec![ValueId::new(0), ValueId::new(1)],
                            },
                        ),
                        value_instruction(
                            3,
                            5,
                            InstructionKind::ExtractValue {
                                aggregate: ValueId::new(2),
                                index: 1,
                            },
                        ),
                        value_instruction(
                            4,
                            4,
                            InstructionKind::StringLiteral {
                                utf8: b"A\n".to_vec(),
                            },
                        ),
                    ],
                    terminator: Terminator::Return {
                        value: Some(ValueId::new(3)),
                    },
                    span: None,
                }],
                span: None,
            }],
        };
        let context = Context::create();
        let llvm = lower_module(
            &context,
            &jir,
            "aggregate_string",
            &TypeLoweringConfig::default(),
        )
        .expect("aggregate/string LLVM module");
        let text = llvm.print_to_string().to_string();
        assert!(text.contains("private unnamed_addr constant [2 x i8] c\"A\\0A\""));
        assert!(text.contains("insertvalue [2 x i32]"), "{text}");
        assert!(text.contains("extractvalue [2 x i32]"), "{text}");
        assert!(llvm.verify().is_ok());
    }

    #[test]
    fn lowers_small_c_aggregate_returns_for_all_supported_packed_targets() {
        let jir = Module {
            types: vec![
                Type::Integer {
                    signed: false,
                    bits: 16,
                },
                Type::Integer {
                    signed: false,
                    bits: 8,
                },
                Type::Struct {
                    fields: vec![TypeId::new(0), TypeId::new(1)],
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "small_aggregate".to_owned(),
                linkage: Linkage::Export,
                parameters: vec![
                    Parameter {
                        value: ValueId::new(0),
                        ty: TypeId::new(0),
                        name: Some("code".to_owned()),
                    },
                    Parameter {
                        value: ValueId::new(1),
                        ty: TypeId::new(1),
                        name: Some("severity".to_owned()),
                    },
                ],
                result: TypeId::new(2),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![value_instruction(
                        2,
                        2,
                        InstructionKind::Aggregate {
                            elements: vec![ValueId::new(0), ValueId::new(1)],
                        },
                    )],
                    terminator: Terminator::Return {
                        value: Some(ValueId::new(2)),
                    },
                    span: None,
                }],
                span: None,
            }],
        };
        for (module_name, config) in [
            (
                "small_aggregate_windows",
                TypeLoweringConfig::x86_64_windows_msvc(),
            ),
            (
                "small_aggregate_linux",
                TypeLoweringConfig::x86_64_linux_gnu(),
            ),
            (
                "small_aggregate_android",
                TypeLoweringConfig::aarch64_android(),
            ),
        ] {
            let context = Context::create();
            let llvm = lower_module(&context, &jir, module_name, &config)
                .expect("small aggregate C ABI module");
            let text = llvm.print_to_string().to_string();
            assert!(text.contains("define i32 @small_aggregate"), "{text}");
            assert!(text.contains("load i32"), "{text}");
            assert!(llvm.verify().is_ok());
        }
    }

    #[test]
    fn lowers_enum_construct_tag_and_proven_payload_extract() {
        let jir = Module {
            types: vec![
                Type::Unit,
                Type::Integer {
                    signed: true,
                    bits: 32,
                },
                Type::Integer {
                    signed: false,
                    bits: 32,
                },
                Type::Enum {
                    variants: vec![Vec::new(), vec![TypeId::new(1)]],
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "enum_roundtrip".to_owned(),
                linkage: Linkage::Internal,
                parameters: vec![Parameter {
                    value: ValueId::new(0),
                    ty: TypeId::new(1),
                    name: Some("payload".to_owned()),
                }],
                result: TypeId::new(1),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        value_instruction(
                            1,
                            3,
                            InstructionKind::EnumConstruct {
                                variant: 1,
                                fields: vec![ValueId::new(0)],
                            },
                        ),
                        value_instruction(
                            2,
                            2,
                            InstructionKind::EnumTag {
                                value: ValueId::new(1),
                            },
                        ),
                        value_instruction(
                            3,
                            1,
                            InstructionKind::EnumExtract {
                                value: ValueId::new(1),
                                variant: 1,
                                field: 0,
                            },
                        ),
                    ],
                    terminator: Terminator::Return {
                        value: Some(ValueId::new(3)),
                    },
                    span: None,
                }],
                span: None,
            }],
        };
        let context = Context::create();
        let llvm = lower_module(
            &context,
            &jir,
            "enum_roundtrip",
            &TypeLoweringConfig::default(),
        )
        .expect("enum LLVM module");
        let text = llvm.print_to_string().to_string();
        assert!(text.contains("store i32 1"), "{text}");
        assert!(text.contains("store { i32 }"), "{text}");
        assert!(text.contains("extractvalue { i32 }"), "{text}");
        assert!(llvm.verify().is_ok());
    }

    #[test]
    fn lowers_array_and_record_projected_offsets() {
        let jir = Module {
            types: vec![
                Type::Unit,
                Type::Integer {
                    signed: true,
                    bits: 32,
                },
                Type::Integer {
                    signed: false,
                    bits: 64,
                },
                Type::Array {
                    element: TypeId::new(1),
                    length: 4,
                },
                Type::Pointer {
                    pointee: TypeId::new(3),
                    address_space: AddressSpace::Stack,
                },
                Type::Pointer {
                    pointee: TypeId::new(1),
                    address_space: AddressSpace::Stack,
                },
                Type::Struct {
                    fields: vec![TypeId::new(1), TypeId::new(1)],
                },
                Type::Pointer {
                    pointee: TypeId::new(6),
                    address_space: AddressSpace::Stack,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "project".to_owned(),
                linkage: Linkage::Internal,
                parameters: vec![
                    Parameter {
                        value: ValueId::new(0),
                        ty: TypeId::new(2),
                        name: Some("index".to_owned()),
                    },
                    Parameter {
                        value: ValueId::new(1),
                        ty: TypeId::new(1),
                        name: Some("value".to_owned()),
                    },
                ],
                result: TypeId::new(1),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        value_instruction(
                            2,
                            4,
                            InstructionKind::StackAlloc {
                                ty: TypeId::new(3),
                                count: None,
                            },
                        ),
                        value_instruction(
                            3,
                            5,
                            InstructionKind::Offset {
                                base: ValueId::new(2),
                                indices: vec![ValueId::new(0)],
                            },
                        ),
                        store_instruction(3, 1),
                        value_instruction(
                            4,
                            1,
                            InstructionKind::Load {
                                pointer: ValueId::new(3),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        value_instruction(
                            5,
                            7,
                            InstructionKind::StackAlloc {
                                ty: TypeId::new(6),
                                count: None,
                            },
                        ),
                        value_instruction(
                            6,
                            2,
                            InstructionKind::Constant(Constant::Integer { value: 1 }),
                        ),
                        value_instruction(
                            7,
                            5,
                            InstructionKind::Offset {
                                base: ValueId::new(5),
                                indices: vec![ValueId::new(6)],
                            },
                        ),
                        store_instruction(7, 1),
                        value_instruction(
                            8,
                            1,
                            InstructionKind::Load {
                                pointer: ValueId::new(7),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        value_instruction(
                            9,
                            1,
                            InstructionKind::Binary {
                                op: BinaryOp::Add,
                                left: ValueId::new(4),
                                right: ValueId::new(8),
                            },
                        ),
                    ],
                    terminator: Terminator::Return {
                        value: Some(ValueId::new(9)),
                    },
                    span: None,
                }],
                span: None,
            }],
        };
        let context = Context::create();
        let llvm = lower_module(
            &context,
            &jir,
            "projected_offsets",
            &TypeLoweringConfig::default(),
        )
        .expect("projected offset LLVM module");
        let text = llvm.print_to_string().to_string();
        assert!(text.contains("getelementptr [4 x i32]"), "{text}");
        assert!(
            text.contains("getelementptr inbounds nuw { i32, i32 }"),
            "{text}"
        );
        assert!(llvm.verify().is_ok());
    }

    #[test]
    fn lowers_bounds_check_to_panic_edge_and_preserves_phi_predecessor() {
        let jir = Module {
            types: vec![
                Type::Unit,
                Type::Integer {
                    signed: false,
                    bits: 64,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "checked_index".to_owned(),
                linkage: Linkage::Internal,
                parameters: vec![
                    Parameter {
                        value: ValueId::new(0),
                        ty: TypeId::new(1),
                        name: Some("index".to_owned()),
                    },
                    Parameter {
                        value: ValueId::new(1),
                        ty: TypeId::new(1),
                        name: Some("length".to_owned()),
                    },
                ],
                result: TypeId::new(1),
                blocks: vec![
                    Block {
                        id: BlockId::new(0),
                        parameters: Vec::new(),
                        instructions: vec![Instruction {
                            result: None,
                            kind: InstructionKind::BoundsCheck {
                                index: ValueId::new(0),
                                length: ValueId::new(1),
                            },
                            span: None,
                        }],
                        terminator: Terminator::Jump {
                            target: BlockId::new(1),
                            arguments: vec![ValueId::new(0)],
                        },
                        span: None,
                    },
                    Block {
                        id: BlockId::new(1),
                        parameters: vec![BlockParameter {
                            value: ValueId::new(2),
                            ty: TypeId::new(1),
                        }],
                        instructions: Vec::new(),
                        terminator: Terminator::Return {
                            value: Some(ValueId::new(2)),
                        },
                        span: None,
                    },
                ],
                span: None,
            }],
        };
        let context = Context::create();
        let llvm = lower_module(
            &context,
            &jir,
            "bounds_check",
            &TypeLoweringConfig::default(),
        )
        .expect("bounds-check LLVM module");
        let text = llvm.print_to_string().to_string();
        assert!(text.contains("icmp ult i64 %index, %length"), "{text}");
        assert!(
            text.contains("call void @jadren_rt_bounds_panic_u64"),
            "{text}"
        );
        assert!(text.contains("noreturn"), "{text}");
        assert!(text.contains("unreachable"), "{text}");
        assert!(text.contains("phi i64 [ %index, %bounds.ok"), "{text}");
        assert!(llvm.verify().is_ok());
    }

    #[test]
    fn lowers_buffer_and_slice_data_pointer_offsets() {
        let jir = Module {
            types: vec![
                Type::Integer {
                    signed: true,
                    bits: 32,
                },
                Type::Integer {
                    signed: false,
                    bits: 64,
                },
                Type::Pointer {
                    pointee: TypeId::new(0),
                    address_space: AddressSpace::Heap,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "buffer_offset".to_owned(),
                linkage: Linkage::Internal,
                parameters: vec![
                    Parameter {
                        value: ValueId::new(0),
                        ty: TypeId::new(2),
                        name: Some("data".to_owned()),
                    },
                    Parameter {
                        value: ValueId::new(1),
                        ty: TypeId::new(1),
                        name: Some("index".to_owned()),
                    },
                ],
                result: TypeId::new(2),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![value_instruction(
                        2,
                        2,
                        InstructionKind::Offset {
                            base: ValueId::new(0),
                            indices: vec![ValueId::new(1)],
                        },
                    )],
                    terminator: Terminator::Return {
                        value: Some(ValueId::new(2)),
                    },
                    span: None,
                }],
                span: None,
            }],
        };
        let context = Context::create();
        let llvm = lower_module(
            &context,
            &jir,
            "buffer_offset",
            &TypeLoweringConfig::default(),
        )
        .expect("buffer offset LLVM module");
        let text = llvm.print_to_string().to_string();
        assert!(
            text.contains("getelementptr i32, ptr %data, i64 %index"),
            "{text}"
        );
        assert!(llvm.verify().is_ok());
    }

    #[test]
    fn lowers_dynamic_offsets_over_nominal_struct_elements() {
        let jir = Module {
            types: vec![
                Type::Float { bits: 32 },
                Type::Integer {
                    signed: false,
                    bits: 64,
                },
                Type::NominalStruct {
                    identity: 0x0123_4567_89ab_cdef,
                    fields: vec![TypeId::new(0), TypeId::new(0)],
                },
                Type::Pointer {
                    pointee: TypeId::new(2),
                    address_space: AddressSpace::Generic,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "nominal_element_offset".to_owned(),
                linkage: Linkage::Internal,
                parameters: vec![
                    Parameter {
                        value: ValueId::new(0),
                        ty: TypeId::new(3),
                        name: Some("data".to_owned()),
                    },
                    Parameter {
                        value: ValueId::new(1),
                        ty: TypeId::new(1),
                        name: Some("index".to_owned()),
                    },
                ],
                result: TypeId::new(3),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![value_instruction(
                        2,
                        3,
                        InstructionKind::Offset {
                            base: ValueId::new(0),
                            indices: vec![ValueId::new(1)],
                        },
                    )],
                    terminator: Terminator::Return {
                        value: Some(ValueId::new(2)),
                    },
                    span: None,
                }],
                span: None,
            }],
        };
        let context = Context::create();
        let llvm = lower_module(
            &context,
            &jir,
            "nominal_element_offset",
            &TypeLoweringConfig::default(),
        )
        .expect("nominal element offset LLVM module");
        let text = llvm.print_to_string().to_string();
        assert!(
            text.contains("getelementptr %jadren.record.0123456789abcdef.t2"),
            "{text}"
        );
        assert!(text.contains(", i64 %index"), "{text}");
        assert!(llvm.verify().is_ok());
    }

    #[test]
    fn lowers_dynamic_inline_array_extraction() {
        let jir = Module {
            types: vec![
                Type::Integer {
                    signed: true,
                    bits: 32,
                },
                Type::Integer {
                    signed: false,
                    bits: 64,
                },
                Type::Array {
                    element: TypeId::new(0),
                    length: 4,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "extract_dynamic".to_owned(),
                linkage: Linkage::Internal,
                parameters: vec![
                    Parameter {
                        value: ValueId::new(0),
                        ty: TypeId::new(2),
                        name: Some("values".to_owned()),
                    },
                    Parameter {
                        value: ValueId::new(1),
                        ty: TypeId::new(1),
                        name: Some("index".to_owned()),
                    },
                ],
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![value_instruction(
                        2,
                        0,
                        InstructionKind::ExtractElement {
                            aggregate: ValueId::new(0),
                            index: ValueId::new(1),
                        },
                    )],
                    terminator: Terminator::Return {
                        value: Some(ValueId::new(2)),
                    },
                    span: None,
                }],
                span: None,
            }],
        };
        let context = Context::create();
        let llvm = lower_module(
            &context,
            &jir,
            "extract_dynamic",
            &TypeLoweringConfig::default(),
        )
        .expect("dynamic extraction LLVM module");
        let text = llvm.print_to_string().to_string();
        assert!(text.contains("alloca [4 x i32]"), "{text}");
        assert!(
            text.contains("getelementptr [4 x i32], ptr %v2.storage, i32 0, i64 %index"),
            "{text}"
        );
        assert!(llvm.verify().is_ok());
    }

    #[test]
    fn lowers_function_address_and_indirect_call_to_verified_llvm() {
        let i32_ty = TypeId::new(1);
        let function_pointer_ty = TypeId::new(2);
        let module = Module {
            types: vec![
                Type::Unit,
                Type::Integer {
                    signed: true,
                    bits: 32,
                },
                Type::Function {
                    parameters: vec![i32_ty],
                    result: i32_ty,
                },
            ],
            functions: vec![
                Function {
                    id: FunctionId::new(0),
                    name: "increment".to_owned(),
                    linkage: Linkage::Internal,
                    parameters: vec![Parameter {
                        value: ValueId::new(0),
                        ty: i32_ty,
                        name: Some("value".to_owned()),
                    }],
                    result: i32_ty,
                    blocks: vec![Block {
                        id: BlockId::new(0),
                        parameters: Vec::new(),
                        instructions: vec![
                            value_instruction(
                                1,
                                i32_ty.index(),
                                InstructionKind::Constant(Constant::Integer { value: 1 }),
                            ),
                            value_instruction(
                                2,
                                i32_ty.index(),
                                InstructionKind::Binary {
                                    op: BinaryOp::Add,
                                    left: ValueId::new(0),
                                    right: ValueId::new(1),
                                },
                            ),
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
                    name: "apply_increment".to_owned(),
                    linkage: Linkage::Export,
                    parameters: vec![Parameter {
                        value: ValueId::new(0),
                        ty: i32_ty,
                        name: Some("value".to_owned()),
                    }],
                    result: i32_ty,
                    blocks: vec![Block {
                        id: BlockId::new(0),
                        parameters: Vec::new(),
                        instructions: vec![
                            value_instruction(
                                1,
                                function_pointer_ty.index(),
                                InstructionKind::FunctionAddress {
                                    function: FunctionId::new(0),
                                },
                            ),
                            value_instruction(
                                2,
                                i32_ty.index(),
                                InstructionKind::IndirectCall {
                                    callee: ValueId::new(1),
                                    arguments: vec![ValueId::new(0)],
                                },
                            ),
                        ],
                        terminator: Terminator::Return {
                            value: Some(ValueId::new(2)),
                        },
                        span: None,
                    }],
                    span: None,
                },
            ],
        };
        let context = Context::create();
        let llvm = lower_module(
            &context,
            &module,
            "function_pointer",
            &TypeLoweringConfig::x86_64_windows_msvc(),
        )
        .expect("function pointer module must verify in LLVM");
        let text = llvm.print_to_string().to_string();
        assert!(
            text.contains("call i32 %") || text.contains("call i32 @jadren.f0.increment"),
            "{text}"
        );
    }

    fn arithmetic_block(
        block: usize,
        parameter: usize,
        constant: usize,
        result: usize,
        op: BinaryOp,
        literal: i128,
    ) -> Block {
        Block {
            id: BlockId::new(block),
            parameters: vec![BlockParameter {
                value: ValueId::new(parameter),
                ty: TypeId::new(2),
            }],
            instructions: vec![
                Instruction {
                    result: Some(TypedValue {
                        value: ValueId::new(constant),
                        ty: TypeId::new(2),
                    }),
                    kind: InstructionKind::Constant(Constant::Integer { value: literal }),
                    span: None,
                },
                Instruction {
                    result: Some(TypedValue {
                        value: ValueId::new(result),
                        ty: TypeId::new(2),
                    }),
                    kind: InstructionKind::Binary {
                        op,
                        left: ValueId::new(parameter),
                        right: ValueId::new(constant),
                    },
                    span: None,
                },
            ],
            terminator: Terminator::Jump {
                target: BlockId::new(3),
                arguments: vec![ValueId::new(result)],
            },
            span: None,
        }
    }

    fn value_instruction(value: usize, ty: usize, kind: InstructionKind) -> Instruction {
        Instruction {
            result: Some(TypedValue {
                value: ValueId::new(value),
                ty: TypeId::new(ty),
            }),
            kind,
            span: None,
        }
    }

    fn store_instruction(pointer: usize, value: usize) -> Instruction {
        Instruction {
            result: None,
            kind: InstructionKind::Store {
                pointer: ValueId::new(pointer),
                value: ValueId::new(value),
                alignment: 4,
                volatile: false,
            },
            span: None,
        }
    }

    fn return_parameter_block(block: usize, value: usize) -> Block {
        Block {
            id: BlockId::new(block),
            parameters: vec![BlockParameter {
                value: ValueId::new(value),
                ty: TypeId::new(3),
            }],
            instructions: Vec::new(),
            terminator: Terminator::Return {
                value: Some(ValueId::new(value)),
            },
            span: None,
        }
    }
}
