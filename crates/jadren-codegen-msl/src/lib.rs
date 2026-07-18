//! Deterministic Metal Shading Language fixture emitter for Jadren.
//!
//! This crate covers the verified bounded runtime-length `u32` storage family
//! (the legacy add-one entrypoint plus the dataflow-tolerant BinaryOp shape)
//! and the matching runtime-length scalar/vector `f32` operation families.
//! It provides a portable source/contract gate for the future Apple adapter;
//! it does not load Metal, invoke `xcrun`, or claim JIR-wide lowering.

use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use jadren_codegen_spirv::{ResourceAccess, ResourceElementType, SpirvArtifact};
use jadren_gpu_runtime::{
    ArtifactSourceBackend, ArtifactSourceTranslationError, ArtifactSourceTranslationReport,
    GpuBackend, SpirvSourceTranslationError, inspect_spirv_source_module,
    translate_spirv_artifact_source, translate_spirv_source_report_for_backend,
    validate_spirv_artifact_contract,
};
pub use jadren_jir::F32ArithmeticOp;
use jadren_jir::{
    AddressSpace, BinaryOp, BuiltinOp, Constant, FunctionId, InstructionKind, Module, Terminator,
    Type, TypeId, verify_gpu,
};

/// MSL workgroup configuration for one compute entrypoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MslOptions {
    /// Threadgroup dimensions in x/y/z order.
    pub workgroup_size: [u32; 3],
}

const fn f32_msl_operator(operation: F32ArithmeticOp) -> &'static str {
    match operation {
        F32ArithmeticOp::Add => "+",
        F32ArithmeticOp::Subtract => "-",
        F32ArithmeticOp::Multiply => "*",
    }
}

impl MslOptions {
    /// Validates the portable baseline workgroup shape.
    pub fn new(workgroup_size: [u32; 3]) -> Result<Self, MslError> {
        if workgroup_size.contains(&0) {
            return Err(MslError::InvalidWorkgroupSize(workgroup_size));
        }
        let product = u64::from(workgroup_size[0])
            .saturating_mul(u64::from(workgroup_size[1]))
            .saturating_mul(u64::from(workgroup_size[2]));
        if product > 1024 {
            return Err(MslError::WorkgroupTooLarge(product));
        }
        Ok(Self { workgroup_size })
    }
}

/// Errors raised before an MSL source artifact is returned.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MslError {
    /// One or more dimensions are zero.
    InvalidWorkgroupSize([u32; 3]),
    /// The product exceeds the portable baseline limit.
    WorkgroupTooLarge(u64),
    /// The entry name is not a safe MSL identifier.
    InvalidEntryName,
    /// The source does not contain the verified bounded kernel contract.
    InvalidContract,
    /// The JIR module failed the GPU verifier.
    JirVerificationFailed(usize),
    /// The JIR body is outside the supported bounded storage shape.
    UnsupportedJirShape(&'static str),
    /// The artifact metadata does not form a safe external-toolchain input.
    InvalidSpirvArtifact(&'static str),
    /// Raw external SPIR-V failed structural validation.
    InvalidSpirv(&'static str),
    /// SPIRV-Cross is not available for an explicitly requested external MSL translation.
    SpirvToolchainUnavailable,
    /// The external SPIRV-Cross process failed or returned an unusable source.
    SpirvTranslation(String),
}

impl fmt::Display for MslError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidWorkgroupSize(size) => {
                write!(formatter, "invalid MSL workgroup size {size:?}")
            }
            Self::WorkgroupTooLarge(product) => {
                write!(formatter, "MSL workgroup product {product} exceeds 1024")
            }
            Self::InvalidEntryName => {
                formatter.write_str("MSL entry name must be an ASCII identifier")
            }
            Self::InvalidContract => {
                formatter.write_str("MSL source does not match the bounded storage contract")
            }
            Self::JirVerificationFailed(count) => {
                write!(
                    formatter,
                    "MSL JIR GPU verification failed with {count} error(s)"
                )
            }
            Self::UnsupportedJirShape(reason) => {
                write!(formatter, "unsupported MSL JIR shape: {reason}")
            }
            Self::InvalidSpirvArtifact(reason) => {
                write!(formatter, "invalid MSL SPIR-V artifact: {reason}")
            }
            Self::InvalidSpirv(reason) => write!(formatter, "invalid MSL SPIR-V: {reason}"),
            Self::SpirvToolchainUnavailable => {
                formatter.write_str("SPIRV-Cross toolchain is unavailable")
            }
            Self::SpirvTranslation(reason) => {
                write!(formatter, "SPIR-V to MSL translation failed: {reason}")
            }
        }
    }
}

impl Error for MslError {}

/// Emits a bounds-safe one-resource `GlobalInvocationId.x` write as
/// deterministic MSL.
///
/// The value and logical length are embedded only after the corresponding JIR
/// shape has been verified. Out-of-range threads leave the storage buffer
/// untouched.
pub fn emit_storage_global_write(
    entry_name: &str,
    options: MslOptions,
    value: u32,
    length: u32,
) -> Result<String, MslError> {
    if !valid_identifier(entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    if length == 0 {
        return Err(MslError::UnsupportedJirShape(
            "global write length must be positive",
        ));
    }
    let [x, y, z] = options.workgroup_size;
    let max_threads = u64::from(x)
        .saturating_mul(u64::from(y))
        .saturating_mul(u64::from(z));
    Ok(format!(
        "#include <metal_stdlib>\n\nusing namespace metal;\n\nkernel void {entry_name}(\n    device uint* data [[buffer(0)]],\n    uint3 gid [[thread_position_in_grid]])\n    [[max_total_threads_per_threadgroup({max_threads})]] {{\n    if (gid.x < {length}u) {{\n        data[gid.x] = {value}u;\n    }}\n}}\n"
    ))
}

/// Lowers the verified one-resource `GlobalInvocationId.x` write shape to a
/// bounded MSL source contract.
pub fn emit_storage_global_write_from_jir(
    module: &Module,
    function: FunctionId,
    options: MslOptions,
) -> Result<String, MslError> {
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(MslError::JirVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(MslError::UnsupportedJirShape("missing function"))?;
    if !valid_identifier(&function_data.name) {
        return Err(MslError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) || function_data.parameters.len() != 1
    {
        return Err(MslError::UnsupportedJirShape(
            "global write requires one resource and Unit result",
        ));
    }
    let pointer_type = module.types.get(function_data.parameters[0].ty.index());
    if !matches!(
        pointer_type,
        Some(Type::Pointer { pointee, address_space: AddressSpace::Storage })
            if matches!(module.types.get(pointee.index()), Some(Type::Integer { signed: false, bits: 32 }))
    ) {
        return Err(MslError::UnsupportedJirShape(
            "global write resource must be a storage u32 pointer",
        ));
    }
    let block = function_data
        .blocks
        .first()
        .ok_or(MslError::UnsupportedJirShape("missing entry block"))?;
    if function_data.blocks.len() != 1
        || !block.parameters.is_empty()
        || block.instructions.len() != 6
    {
        return Err(MslError::UnsupportedJirShape(
            "global write body must contain builtin, value, length, bounds, offset and store",
        ));
    }
    let builtin = &block.instructions[0];
    let Some(builtin_result) = builtin.result else {
        return Err(MslError::UnsupportedJirShape(
            "builtin must produce a value",
        ));
    };
    if !matches!(
        builtin.kind,
        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX)
    ) {
        return Err(MslError::UnsupportedJirShape(
            "first instruction must be GlobalInvocationId.x",
        ));
    }
    let constant = &block.instructions[1];
    let Some(value_result) = constant.result else {
        return Err(MslError::UnsupportedJirShape(
            "write value must produce a value",
        ));
    };
    let value = match (&constant.kind, value_result.ty == builtin_result.ty) {
        (InstructionKind::Constant(Constant::Integer { value }), true) => u32::try_from(*value)
            .map_err(|_| MslError::UnsupportedJirShape("write value is outside u32"))?,
        _ => {
            return Err(MslError::UnsupportedJirShape(
                "second instruction must be a u32 value",
            ));
        }
    };
    let length_instruction = &block.instructions[2];
    let Some(length_result) = length_instruction.result else {
        return Err(MslError::UnsupportedJirShape(
            "write length must produce a value",
        ));
    };
    let length = match (
        &length_instruction.kind,
        length_result.ty == builtin_result.ty,
    ) {
        (InstructionKind::Constant(Constant::Integer { value }), true) => u32::try_from(*value)
            .map_err(|_| MslError::UnsupportedJirShape("write length is outside u32"))?,
        _ => {
            return Err(MslError::UnsupportedJirShape(
                "third instruction must be a u32 length",
            ));
        }
    };
    if length == 0 {
        return Err(MslError::UnsupportedJirShape(
            "write length must be positive",
        ));
    }
    if !matches!(
        &block.instructions[3].kind,
        InstructionKind::BoundsCheck { index, length: bound_length }
            if *index == builtin_result.value && *bound_length == length_result.value
    ) {
        return Err(MslError::UnsupportedJirShape(
            "global write bounds check is invalid",
        ));
    }
    let offset = &block.instructions[4];
    let Some(offset_result) = offset.result else {
        return Err(MslError::UnsupportedJirShape(
            "global write offset must produce a pointer",
        ));
    };
    if !matches!(
        &offset.kind,
        InstructionKind::Offset { base, indices }
            if *base == function_data.parameters[0].value && indices.as_slice() == [builtin_result.value]
    ) {
        return Err(MslError::UnsupportedJirShape(
            "global write offset is invalid",
        ));
    }
    if !matches!(
        &block.instructions[5].kind,
        InstructionKind::Store { pointer, value: stored_value, .. }
            if *pointer == offset_result.value && *stored_value == value_result.value
    ) || !matches!(block.terminator, Terminator::Return { value: None })
    {
        return Err(MslError::UnsupportedJirShape(
            "global write store/return is invalid",
        ));
    }
    emit_storage_global_write(&function_data.name, options, value, length)
}

/// Emits a bounds-safe runtime-stride `GlobalInvocationId.x` write as
/// deterministic MSL.
///
/// Length, stride and capacity remain separate device buffers so both the
/// logical and physical bounds contracts survive the future Metal adapter.
pub fn emit_storage_global_strided_write(
    entry_name: &str,
    options: MslOptions,
    value: u32,
) -> Result<String, MslError> {
    if !valid_identifier(entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    let [x, y, z] = options.workgroup_size;
    let max_threads = u64::from(x)
        .saturating_mul(u64::from(y))
        .saturating_mul(u64::from(z));
    Ok(format!(
        "#include <metal_stdlib>\n\nusing namespace metal;\n\nkernel void {entry_name}(\n    device uint* data [[buffer(0)]],\n    device const uint* length [[buffer(1)]],\n    device const uint* stride [[buffer(2)]],\n    device const uint* capacity [[buffer(3)]],\n    uint3 gid [[thread_position_in_grid]])\n    [[max_total_threads_per_threadgroup({max_threads})]] {{\n    if (gid.x < length[0]) {{\n        uint physical = gid.x * stride[0];\n        if (physical < capacity[0]) {{\n            data[physical] = {value}u;\n        }}\n    }}\n}}\n"
    ))
}

/// Lowers the verified four-resource runtime-stride global-write JIR shape to
/// the corresponding MSL source contract.
pub fn emit_storage_global_strided_write_from_jir(
    module: &Module,
    function: FunctionId,
    options: MslOptions,
) -> Result<String, MslError> {
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(MslError::JirVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(MslError::UnsupportedJirShape("missing function"))?;
    if !valid_identifier(&function_data.name) {
        return Err(MslError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) || function_data.parameters.len() != 4
    {
        return Err(MslError::UnsupportedJirShape(
            "global strided write requires four resources and Unit result",
        ));
    }
    if function_data.parameters.iter().any(|parameter| {
        !matches!(
            module.types.get(parameter.ty.index()),
            Some(Type::Pointer { pointee, address_space: AddressSpace::Storage })
                if matches!(module.types.get(pointee.index()), Some(Type::Integer { signed: false, bits: 32 }))
        )
    }) {
        return Err(MslError::UnsupportedJirShape(
            "global strided write resources must be storage u32 pointers",
        ));
    }
    let block = function_data
        .blocks
        .first()
        .ok_or(MslError::UnsupportedJirShape("missing entry block"))?;
    if function_data.blocks.len() != 1
        || !block.parameters.is_empty()
        || block.instructions.len() != 10
    {
        return Err(MslError::UnsupportedJirShape(
            "global strided write body has an unsupported instruction count",
        ));
    }
    let builtin = &block.instructions[0];
    let Some(builtin_result) = builtin.result else {
        return Err(MslError::UnsupportedJirShape(
            "builtin must produce a value",
        ));
    };
    if !matches!(
        builtin.kind,
        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX)
    ) {
        return Err(MslError::UnsupportedJirShape(
            "expected GlobalInvocationId.x",
        ));
    }
    let constant = &block.instructions[1];
    let Some(value_result) = constant.result else {
        return Err(MslError::UnsupportedJirShape(
            "write value must produce a value",
        ));
    };
    let value = match (&constant.kind, value_result.ty == builtin_result.ty) {
        (InstructionKind::Constant(Constant::Integer { value }), true) => u32::try_from(*value)
            .map_err(|_| MslError::UnsupportedJirShape("write value is outside u32"))?,
        _ => return Err(MslError::UnsupportedJirShape("expected u32 write value")),
    };
    let mut metadata_values = [builtin_result; 3];
    for (index, parameter) in function_data.parameters[1..].iter().enumerate() {
        let instruction = &block.instructions[index + 2];
        let Some(result) = instruction.result else {
            return Err(MslError::UnsupportedJirShape(
                "metadata load must produce a value",
            ));
        };
        if !matches!(
            &instruction.kind,
            InstructionKind::Load { pointer, .. } if *pointer == parameter.value && result.ty == builtin_result.ty
        ) {
            return Err(MslError::UnsupportedJirShape("metadata load is invalid"));
        }
        metadata_values[index] = result;
    }
    if !matches!(
        &block.instructions[5].kind,
        InstructionKind::BoundsCheck { index, length }
            if *index == builtin_result.value && *length == metadata_values[0].value
    ) {
        return Err(MslError::UnsupportedJirShape(
            "logical bounds check is invalid",
        ));
    }
    let multiply = &block.instructions[6];
    let Some(physical_result) = multiply.result else {
        return Err(MslError::UnsupportedJirShape(
            "physical index must produce a value",
        ));
    };
    if !matches!(
        &multiply.kind,
        InstructionKind::Binary { op: BinaryOp::Multiply, left, right }
            if *left == builtin_result.value && *right == metadata_values[1].value && physical_result.ty == builtin_result.ty
    ) {
        return Err(MslError::UnsupportedJirShape(
            "runtime stride multiply is invalid",
        ));
    }
    if !matches!(
        &block.instructions[7].kind,
        InstructionKind::BoundsCheck { index, length }
            if *index == physical_result.value && *length == metadata_values[2].value
    ) {
        return Err(MslError::UnsupportedJirShape(
            "physical bounds check is invalid",
        ));
    }
    let offset = &block.instructions[8];
    let Some(offset_result) = offset.result else {
        return Err(MslError::UnsupportedJirShape(
            "data offset must produce a pointer",
        ));
    };
    if !matches!(
        &offset.kind,
        InstructionKind::Offset { base, indices }
            if *base == function_data.parameters[0].value && indices.as_slice() == [physical_result.value]
    ) {
        return Err(MslError::UnsupportedJirShape("data offset is invalid"));
    }
    if !matches!(
        &block.instructions[9].kind,
        InstructionKind::Store { pointer, value: stored_value, .. }
            if *pointer == offset_result.value && *stored_value == value_result.value
    ) || !matches!(block.terminator, Terminator::Return { value: None })
    {
        return Err(MslError::UnsupportedJirShape(
            "global strided write store/return is invalid",
        ));
    }
    emit_storage_global_strided_write(&function_data.name, options, value)
}

/// Emits a bounds-safe two-dimensional row-major global write as deterministic
/// MSL. Width, height and capacity remain separate device buffers.
pub fn emit_storage_global_2d_write(
    entry_name: &str,
    options: MslOptions,
    value: u32,
) -> Result<String, MslError> {
    if !valid_identifier(entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    let [x, y, z] = options.workgroup_size;
    let max_threads = u64::from(x)
        .saturating_mul(u64::from(y))
        .saturating_mul(u64::from(z));
    Ok(format!(
        "#include <metal_stdlib>\n\nusing namespace metal;\n\nkernel void {entry_name}(\n    device uint* data [[buffer(0)]],\n    device const uint* width [[buffer(1)]],\n    device const uint* height [[buffer(2)]],\n    device const uint* capacity [[buffer(3)]],\n    uint3 gid [[thread_position_in_grid]])\n    [[max_total_threads_per_threadgroup({max_threads})]] {{\n    if (gid.x < width[0] && gid.y < height[0]) {{\n        uint physical = gid.y * width[0] + gid.x;\n        if (physical < capacity[0]) {{\n            data[physical] = {value}u;\n        }}\n    }}\n}}\n"
    ))
}

/// Lowers the verified four-resource two-dimensional row-major global-write
/// JIR shape to the corresponding MSL source contract.
pub fn emit_storage_global_2d_write_from_jir(
    module: &Module,
    function: FunctionId,
    options: MslOptions,
) -> Result<String, MslError> {
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(MslError::JirVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(MslError::UnsupportedJirShape("missing function"))?;
    if !valid_identifier(&function_data.name) {
        return Err(MslError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) || function_data.parameters.len() != 4
    {
        return Err(MslError::UnsupportedJirShape(
            "global 2D write requires four resources and Unit result",
        ));
    }
    if function_data.parameters.iter().any(|parameter| {
        !matches!(
            module.types.get(parameter.ty.index()),
            Some(Type::Pointer { pointee, address_space: AddressSpace::Storage })
                if matches!(module.types.get(pointee.index()), Some(Type::Integer { signed: false, bits: 32 }))
        )
    }) {
        return Err(MslError::UnsupportedJirShape(
            "global 2D write resources must be storage u32 pointers",
        ));
    }
    let block = function_data
        .blocks
        .first()
        .ok_or(MslError::UnsupportedJirShape("missing entry block"))?;
    if function_data.blocks.len() != 1
        || !block.parameters.is_empty()
        || block.instructions.len() != 13
    {
        return Err(MslError::UnsupportedJirShape(
            "global 2D write body has an unsupported instruction count",
        ));
    }
    let x_instruction = &block.instructions[0];
    let y_instruction = &block.instructions[1];
    let Some(x_result) = x_instruction.result else {
        return Err(MslError::UnsupportedJirShape(
            "x builtin must produce a value",
        ));
    };
    let Some(y_result) = y_instruction.result else {
        return Err(MslError::UnsupportedJirShape(
            "y builtin must produce a value",
        ));
    };
    if !matches!(
        x_instruction.kind,
        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX)
    ) || !matches!(
        y_instruction.kind,
        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdY)
    ) || x_result.ty != y_result.ty
    {
        return Err(MslError::UnsupportedJirShape(
            "expected GlobalInvocationId.x/y",
        ));
    }
    let constant = &block.instructions[2];
    let Some(value_result) = constant.result else {
        return Err(MslError::UnsupportedJirShape(
            "write value must produce a value",
        ));
    };
    let value = match (&constant.kind, value_result.ty == x_result.ty) {
        (InstructionKind::Constant(Constant::Integer { value }), true) => u32::try_from(*value)
            .map_err(|_| MslError::UnsupportedJirShape("write value is outside u32"))?,
        _ => return Err(MslError::UnsupportedJirShape("expected u32 write value")),
    };
    let mut metadata = [x_result; 3];
    for (index, parameter) in function_data.parameters[1..].iter().enumerate() {
        let instruction = &block.instructions[index + 3];
        let Some(result) = instruction.result else {
            return Err(MslError::UnsupportedJirShape(
                "metadata load must produce a value",
            ));
        };
        if !matches!(
            &instruction.kind,
            InstructionKind::Load { pointer, .. } if *pointer == parameter.value && result.ty == x_result.ty
        ) {
            return Err(MslError::UnsupportedJirShape("2D metadata load is invalid"));
        }
        metadata[index] = result;
    }
    for (instruction, index, length) in [
        (&block.instructions[6], x_result.value, metadata[0].value),
        (&block.instructions[7], y_result.value, metadata[1].value),
    ] {
        if !matches!(
            &instruction.kind,
            InstructionKind::BoundsCheck { index: actual_index, length: actual_length }
                if *actual_index == index && *actual_length == length
        ) {
            return Err(MslError::UnsupportedJirShape(
                "2D coordinate bounds check is invalid",
            ));
        }
    }
    let multiply = &block.instructions[8];
    let Some(row_result) = multiply.result else {
        return Err(MslError::UnsupportedJirShape(
            "row multiply must produce a value",
        ));
    };
    if !matches!(
        &multiply.kind,
        InstructionKind::Binary { op: BinaryOp::Multiply, left, right }
            if *left == y_result.value && *right == metadata[0].value && row_result.ty == x_result.ty
    ) {
        return Err(MslError::UnsupportedJirShape(
            "row-major multiply is invalid",
        ));
    }
    let add = &block.instructions[9];
    let Some(physical_result) = add.result else {
        return Err(MslError::UnsupportedJirShape(
            "physical index must produce a value",
        ));
    };
    if !matches!(
        &add.kind,
        InstructionKind::Binary { op: BinaryOp::Add, left, right }
            if *left == row_result.value && *right == x_result.value && physical_result.ty == x_result.ty
    ) {
        return Err(MslError::UnsupportedJirShape("row-major add is invalid"));
    }
    if !matches!(
        &block.instructions[10].kind,
        InstructionKind::BoundsCheck { index, length }
            if *index == physical_result.value && *length == metadata[2].value
    ) {
        return Err(MslError::UnsupportedJirShape(
            "physical capacity bounds check is invalid",
        ));
    }
    let offset = &block.instructions[11];
    let Some(offset_result) = offset.result else {
        return Err(MslError::UnsupportedJirShape(
            "data offset must produce a pointer",
        ));
    };
    if !matches!(
        &offset.kind,
        InstructionKind::Offset { base, indices }
            if *base == function_data.parameters[0].value && indices.as_slice() == [physical_result.value]
    ) {
        return Err(MslError::UnsupportedJirShape("data offset is invalid"));
    }
    if !matches!(
        &block.instructions[12].kind,
        InstructionKind::Store { pointer, value: stored_value, .. }
            if *pointer == offset_result.value && *stored_value == value_result.value
    ) || !matches!(block.terminator, Terminator::Return { value: None })
    {
        return Err(MslError::UnsupportedJirShape(
            "global 2D write store/return is invalid",
        ));
    }
    emit_storage_global_2d_write(&function_data.name, options, value)
}

/// Emits a bounds-safe two-dimensional affine-stride write as deterministic
/// MSL. The two physical strides and capacity remain separate device buffers.
pub fn emit_storage_global_2d_strided_write(
    entry_name: &str,
    options: MslOptions,
    value: u32,
) -> Result<String, MslError> {
    if !valid_identifier(entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    let [x, y, z] = options.workgroup_size;
    let max_threads = u64::from(x)
        .saturating_mul(u64::from(y))
        .saturating_mul(u64::from(z));
    Ok(format!(
        "#include <metal_stdlib>\n\nusing namespace metal;\n\nkernel void {entry_name}(\n    device uint* data [[buffer(0)]],\n    device const uint* width [[buffer(1)]],\n    device const uint* height [[buffer(2)]],\n    device const uint* stride_x [[buffer(3)]],\n    device const uint* stride_y [[buffer(4)]],\n    device const uint* capacity [[buffer(5)]],\n    uint3 gid [[thread_position_in_grid]])\n    [[max_total_threads_per_threadgroup({max_threads})]] {{\n    if (gid.x < width[0] && gid.y < height[0]) {{\n        uint physical = gid.x * stride_x[0] + gid.y * stride_y[0];\n        if (physical < capacity[0]) {{\n            data[physical] = {value}u;\n        }}\n    }}\n}}\n"
    ))
}

/// Lowers the verified six-resource 2D affine-stride JIR shape to MSL.
///
/// The accepted one-block shape is intentionally narrow: two coordinate
/// bounds checks must precede `x * stride_x + y * stride_y`, and a distinct
/// physical-capacity guard must precede the storage write.
pub fn emit_storage_global_2d_strided_write_from_jir(
    module: &Module,
    function: FunctionId,
    options: MslOptions,
) -> Result<String, MslError> {
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(MslError::JirVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(MslError::UnsupportedJirShape("missing function"))?;
    if !valid_identifier(&function_data.name) {
        return Err(MslError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) || function_data.parameters.len() != 6
    {
        return Err(MslError::UnsupportedJirShape(
            "global 2D strided write requires six resources and Unit result",
        ));
    }
    if function_data.parameters.iter().any(|parameter| {
        !matches!(
            module.types.get(parameter.ty.index()),
            Some(Type::Pointer { pointee, address_space: AddressSpace::Storage })
                if matches!(module.types.get(pointee.index()), Some(Type::Integer { signed: false, bits: 32 }))
        )
    }) {
        return Err(MslError::UnsupportedJirShape(
            "global 2D strided write resources must be storage u32 pointers",
        ));
    }
    let block = function_data
        .blocks
        .first()
        .ok_or(MslError::UnsupportedJirShape("missing entry block"))?;
    if function_data.blocks.len() != 1
        || !block.parameters.is_empty()
        || block.instructions.len() != 16
    {
        return Err(MslError::UnsupportedJirShape(
            "global 2D strided write body has an unsupported instruction count",
        ));
    }
    let get_value = |instruction: &jadren_jir::Instruction, message| {
        instruction
            .result
            .ok_or(MslError::UnsupportedJirShape(message))
    };
    let x = get_value(&block.instructions[0], "x builtin must produce a value")?;
    let y = get_value(&block.instructions[1], "y builtin must produce a value")?;
    if !matches!(
        block.instructions[0].kind,
        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX)
    ) || !matches!(
        block.instructions[1].kind,
        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdY)
    ) || x.ty != y.ty
    {
        return Err(MslError::UnsupportedJirShape(
            "expected GlobalInvocationId.x/y",
        ));
    }
    let stored = get_value(&block.instructions[2], "write value must produce a value")?;
    let value = match (&block.instructions[2].kind, stored.ty == x.ty) {
        (InstructionKind::Constant(Constant::Integer { value }), true) => u32::try_from(*value)
            .map_err(|_| MslError::UnsupportedJirShape("write value is outside u32"))?,
        _ => return Err(MslError::UnsupportedJirShape("expected u32 write value")),
    };
    let mut metadata = [x; 5];
    for (index, parameter) in function_data.parameters[1..].iter().enumerate() {
        let instruction = &block.instructions[index + 3];
        let result = get_value(instruction, "metadata load must produce a value")?;
        if !matches!(
            &instruction.kind,
            InstructionKind::Load { pointer, .. } if *pointer == parameter.value && result.ty == x.ty
        ) {
            return Err(MslError::UnsupportedJirShape(
                "2D strided metadata load is invalid",
            ));
        }
        metadata[index] = result;
    }
    for (instruction, coordinate, limit) in [
        (&block.instructions[8], x.value, metadata[0].value),
        (&block.instructions[9], y.value, metadata[1].value),
    ] {
        if !matches!(
            &instruction.kind,
            InstructionKind::BoundsCheck { index, length }
                if *index == coordinate && *length == limit
        ) {
            return Err(MslError::UnsupportedJirShape(
                "2D coordinate bounds check is invalid",
            ));
        }
    }
    let x_offset = get_value(
        &block.instructions[10],
        "x stride multiply must produce a value",
    )?;
    let y_offset = get_value(
        &block.instructions[11],
        "y stride multiply must produce a value",
    )?;
    if !matches!(
        &block.instructions[10].kind,
        InstructionKind::Binary { op: BinaryOp::Multiply, left, right }
            if *left == x.value && *right == metadata[2].value && x_offset.ty == x.ty
    ) || !matches!(
        &block.instructions[11].kind,
        InstructionKind::Binary { op: BinaryOp::Multiply, left, right }
            if *left == y.value && *right == metadata[3].value && y_offset.ty == x.ty
    ) {
        return Err(MslError::UnsupportedJirShape(
            "2D affine stride multiply is invalid",
        ));
    }
    let physical = get_value(
        &block.instructions[12],
        "physical index must produce a value",
    )?;
    if !matches!(
        &block.instructions[12].kind,
        InstructionKind::Binary { op: BinaryOp::Add, left, right }
            if *left == x_offset.value && *right == y_offset.value && physical.ty == x.ty
    ) {
        return Err(MslError::UnsupportedJirShape(
            "2D affine stride add is invalid",
        ));
    }
    if !matches!(
        &block.instructions[13].kind,
        InstructionKind::BoundsCheck { index, length }
            if *index == physical.value && *length == metadata[4].value
    ) {
        return Err(MslError::UnsupportedJirShape(
            "physical capacity bounds check is invalid",
        ));
    }
    let pointer = get_value(
        &block.instructions[14],
        "data offset must produce a pointer",
    )?;
    if !matches!(
        &block.instructions[14].kind,
        InstructionKind::Offset { base, indices }
            if *base == function_data.parameters[0].value && indices.as_slice() == [physical.value]
    ) || !matches!(
        &block.instructions[15].kind,
        InstructionKind::Store { pointer: stored_pointer, value: stored_value, .. }
            if *stored_pointer == pointer.value && *stored_value == stored.value
    ) || !matches!(block.terminator, Terminator::Return { value: None })
    {
        return Err(MslError::UnsupportedJirShape(
            "2D affine strided store/return is invalid",
        ));
    }
    emit_storage_global_2d_strided_write(&function_data.name, options, value)
}

/// Emits a bounds-safe three-dimensional row-major write as deterministic MSL.
/// Width, height, depth and capacity remain separate device buffers.
pub fn emit_storage_global_3d_write(
    entry_name: &str,
    options: MslOptions,
    value: u32,
) -> Result<String, MslError> {
    if !valid_identifier(entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    let [x, y, z] = options.workgroup_size;
    let max_threads = u64::from(x)
        .saturating_mul(u64::from(y))
        .saturating_mul(u64::from(z));
    Ok(format!(
        "#include <metal_stdlib>\n\nusing namespace metal;\n\nkernel void {entry_name}(\n    device uint* data [[buffer(0)]],\n    device const uint* width [[buffer(1)]],\n    device const uint* height [[buffer(2)]],\n    device const uint* depth [[buffer(3)]],\n    device const uint* capacity [[buffer(4)]],\n    uint3 gid [[thread_position_in_grid]])\n    [[max_total_threads_per_threadgroup({max_threads})]] {{\n    if (gid.x < width[0] && gid.y < height[0] && gid.z < depth[0]) {{\n        uint physical = (gid.z * height[0] + gid.y) * width[0] + gid.x;\n        if (physical < capacity[0]) {{\n            data[physical] = {value}u;\n        }}\n    }}\n}}\n"
    ))
}

/// Lowers the verified five-resource 3D row-major JIR shape to MSL.
///
/// The accepted one-block shape is intentionally narrow: all coordinate
/// bounds checks must precede the row-major flattening and the distinct
/// physical-capacity guard must precede the storage write.
pub fn emit_storage_global_3d_write_from_jir(
    module: &Module,
    function: FunctionId,
    options: MslOptions,
) -> Result<String, MslError> {
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(MslError::JirVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(MslError::UnsupportedJirShape("missing function"))?;
    if !valid_identifier(&function_data.name) {
        return Err(MslError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) || function_data.parameters.len() != 5
    {
        return Err(MslError::UnsupportedJirShape(
            "global 3D write requires five resources and Unit result",
        ));
    }
    if function_data.parameters.iter().any(|parameter| {
        !matches!(
            module.types.get(parameter.ty.index()),
            Some(Type::Pointer { pointee, address_space: AddressSpace::Storage })
                if matches!(module.types.get(pointee.index()), Some(Type::Integer { signed: false, bits: 32 }))
        )
    }) {
        return Err(MslError::UnsupportedJirShape(
            "global 3D write resources must be storage u32 pointers",
        ));
    }
    let block = function_data
        .blocks
        .first()
        .ok_or(MslError::UnsupportedJirShape("missing entry block"))?;
    if function_data.blocks.len() != 1
        || !block.parameters.is_empty()
        || block.instructions.len() != 18
    {
        return Err(MslError::UnsupportedJirShape(
            "global 3D write body has an unsupported instruction count",
        ));
    }
    let get_value = |instruction: &jadren_jir::Instruction, message| {
        instruction
            .result
            .ok_or(MslError::UnsupportedJirShape(message))
    };
    let x = get_value(&block.instructions[0], "x builtin must produce a value")?;
    let y = get_value(&block.instructions[1], "y builtin must produce a value")?;
    let z = get_value(&block.instructions[2], "z builtin must produce a value")?;
    if !matches!(
        block.instructions[0].kind,
        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX)
    ) || !matches!(
        block.instructions[1].kind,
        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdY)
    ) || !matches!(
        block.instructions[2].kind,
        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdZ)
    ) || x.ty != y.ty
        || x.ty != z.ty
    {
        return Err(MslError::UnsupportedJirShape(
            "expected GlobalInvocationId.x/y/z",
        ));
    }
    let stored = get_value(&block.instructions[3], "write value must produce a value")?;
    let value = match (&block.instructions[3].kind, stored.ty == x.ty) {
        (InstructionKind::Constant(Constant::Integer { value }), true) => u32::try_from(*value)
            .map_err(|_| MslError::UnsupportedJirShape("write value is outside u32"))?,
        _ => return Err(MslError::UnsupportedJirShape("expected u32 write value")),
    };
    let mut metadata = [x; 4];
    for (index, parameter) in function_data.parameters[1..].iter().enumerate() {
        let instruction = &block.instructions[index + 4];
        let result = get_value(instruction, "metadata load must produce a value")?;
        if !matches!(
            &instruction.kind,
            InstructionKind::Load { pointer, .. } if *pointer == parameter.value && result.ty == x.ty
        ) {
            return Err(MslError::UnsupportedJirShape("3D metadata load is invalid"));
        }
        metadata[index] = result;
    }
    for (instruction, coordinate, limit) in [
        (&block.instructions[8], x.value, metadata[0].value),
        (&block.instructions[9], y.value, metadata[1].value),
        (&block.instructions[10], z.value, metadata[2].value),
    ] {
        if !matches!(
            &instruction.kind,
            InstructionKind::BoundsCheck { index, length }
                if *index == coordinate && *length == limit
        ) {
            return Err(MslError::UnsupportedJirShape(
                "3D coordinate bounds check is invalid",
            ));
        }
    }
    let depth_rows = get_value(
        &block.instructions[11],
        "depth row multiply must produce a value",
    )?;
    if !matches!(
        &block.instructions[11].kind,
        InstructionKind::Binary { op: BinaryOp::Multiply, left, right }
            if *left == z.value && *right == metadata[1].value && depth_rows.ty == x.ty
    ) {
        return Err(MslError::UnsupportedJirShape(
            "3D depth row multiply is invalid",
        ));
    }
    let row = get_value(&block.instructions[12], "row offset must produce a value")?;
    if !matches!(
        &block.instructions[12].kind,
        InstructionKind::Binary { op: BinaryOp::Add, left, right }
            if *left == depth_rows.value && *right == y.value && row.ty == x.ty
    ) {
        return Err(MslError::UnsupportedJirShape("3D row offset is invalid"));
    }
    let plane = get_value(
        &block.instructions[13],
        "plane stride multiply must produce a value",
    )?;
    if !matches!(
        &block.instructions[13].kind,
        InstructionKind::Binary { op: BinaryOp::Multiply, left, right }
            if *left == row.value && *right == metadata[0].value && plane.ty == x.ty
    ) {
        return Err(MslError::UnsupportedJirShape(
            "3D plane stride multiply is invalid",
        ));
    }
    let physical = get_value(
        &block.instructions[14],
        "physical index must produce a value",
    )?;
    if !matches!(
        &block.instructions[14].kind,
        InstructionKind::Binary { op: BinaryOp::Add, left, right }
            if *left == plane.value && *right == x.value && physical.ty == x.ty
    ) {
        return Err(MslError::UnsupportedJirShape(
            "3D physical index is invalid",
        ));
    }
    if !matches!(
        &block.instructions[15].kind,
        InstructionKind::BoundsCheck { index, length }
            if *index == physical.value && *length == metadata[3].value
    ) {
        return Err(MslError::UnsupportedJirShape(
            "3D physical capacity bounds check is invalid",
        ));
    }
    let pointer = get_value(
        &block.instructions[16],
        "data offset must produce a pointer",
    )?;
    if !matches!(
        &block.instructions[16].kind,
        InstructionKind::Offset { base, indices }
            if *base == function_data.parameters[0].value && indices.as_slice() == [physical.value]
    ) || !matches!(
        &block.instructions[17].kind,
        InstructionKind::Store { pointer: stored_pointer, value: stored_value, .. }
            if *stored_pointer == pointer.value && *stored_value == stored.value
    ) || !matches!(block.terminator, Terminator::Return { value: None })
    {
        return Err(MslError::UnsupportedJirShape(
            "3D row-major store/return is invalid",
        ));
    }
    emit_storage_global_3d_write(&function_data.name, options, value)
}

/// Emits a bounds-safe three-dimensional affine-stride write as deterministic
/// MSL. Dimensions, physical strides and capacity remain separate device
/// buffers.
pub fn emit_storage_global_3d_strided_write(
    entry_name: &str,
    options: MslOptions,
    value: u32,
) -> Result<String, MslError> {
    if !valid_identifier(entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    let [x, y, z] = options.workgroup_size;
    let max_threads = u64::from(x)
        .saturating_mul(u64::from(y))
        .saturating_mul(u64::from(z));
    Ok(format!(
        "#include <metal_stdlib>\n\nusing namespace metal;\n\nkernel void {entry_name}(\n    device uint* data [[buffer(0)]],\n    device const uint* width [[buffer(1)]],\n    device const uint* height [[buffer(2)]],\n    device const uint* depth [[buffer(3)]],\n    device const uint* stride_x [[buffer(4)]],\n    device const uint* stride_y [[buffer(5)]],\n    device const uint* stride_z [[buffer(6)]],\n    device const uint* capacity [[buffer(7)]],\n    uint3 gid [[thread_position_in_grid]])\n    [[max_total_threads_per_threadgroup({max_threads})]] {{\n    if (gid.x < width[0] && gid.y < height[0] && gid.z < depth[0]) {{\n        uint physical = gid.x * stride_x[0] + gid.y * stride_y[0] + gid.z * stride_z[0];\n        if (physical < capacity[0]) {{\n            data[physical] = {value}u;\n        }}\n    }}\n}}\n"
    ))
}

/// Lowers the verified eight-resource 3D affine-stride JIR shape to MSL.
///
/// The accepted one-block shape is intentionally narrow: three coordinate
/// bounds checks must precede the affine physical calculation and a distinct
/// physical-capacity guard must precede the storage write.
pub fn emit_storage_global_3d_strided_write_from_jir(
    module: &Module,
    function: FunctionId,
    options: MslOptions,
) -> Result<String, MslError> {
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(MslError::JirVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(MslError::UnsupportedJirShape("missing function"))?;
    if !valid_identifier(&function_data.name) {
        return Err(MslError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) || function_data.parameters.len() != 8
    {
        return Err(MslError::UnsupportedJirShape(
            "global 3D strided write requires eight resources and Unit result",
        ));
    }
    if function_data.parameters.iter().any(|parameter| {
        !matches!(
            module.types.get(parameter.ty.index()),
            Some(Type::Pointer { pointee, address_space: AddressSpace::Storage })
                if matches!(module.types.get(pointee.index()), Some(Type::Integer { signed: false, bits: 32 }))
        )
    }) {
        return Err(MslError::UnsupportedJirShape(
            "global 3D strided write resources must be storage u32 pointers",
        ));
    }
    let block = function_data
        .blocks
        .first()
        .ok_or(MslError::UnsupportedJirShape("missing entry block"))?;
    if function_data.blocks.len() != 1
        || !block.parameters.is_empty()
        || block.instructions.len() != 22
    {
        return Err(MslError::UnsupportedJirShape(
            "global 3D strided write body has an unsupported instruction count",
        ));
    }
    let get_value = |instruction: &jadren_jir::Instruction, message| {
        instruction
            .result
            .ok_or(MslError::UnsupportedJirShape(message))
    };
    let x = get_value(&block.instructions[0], "x builtin must produce a value")?;
    let y = get_value(&block.instructions[1], "y builtin must produce a value")?;
    let z = get_value(&block.instructions[2], "z builtin must produce a value")?;
    if !matches!(
        block.instructions[0].kind,
        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX)
    ) || !matches!(
        block.instructions[1].kind,
        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdY)
    ) || !matches!(
        block.instructions[2].kind,
        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdZ)
    ) || x.ty != y.ty
        || x.ty != z.ty
    {
        return Err(MslError::UnsupportedJirShape(
            "expected GlobalInvocationId.x/y/z",
        ));
    }
    let stored = get_value(&block.instructions[3], "write value must produce a value")?;
    let value = match (&block.instructions[3].kind, stored.ty == x.ty) {
        (InstructionKind::Constant(Constant::Integer { value }), true) => u32::try_from(*value)
            .map_err(|_| MslError::UnsupportedJirShape("write value is outside u32"))?,
        _ => return Err(MslError::UnsupportedJirShape("expected u32 write value")),
    };
    let mut metadata = [x; 7];
    for (index, parameter) in function_data.parameters[1..].iter().enumerate() {
        let instruction = &block.instructions[index + 4];
        let result = get_value(instruction, "metadata load must produce a value")?;
        if !matches!(
            &instruction.kind,
            InstructionKind::Load { pointer, .. } if *pointer == parameter.value && result.ty == x.ty
        ) {
            return Err(MslError::UnsupportedJirShape(
                "3D strided metadata load is invalid",
            ));
        }
        metadata[index] = result;
    }
    for (instruction, coordinate, limit) in [
        (&block.instructions[11], x.value, metadata[0].value),
        (&block.instructions[12], y.value, metadata[1].value),
        (&block.instructions[13], z.value, metadata[2].value),
    ] {
        if !matches!(
            &instruction.kind,
            InstructionKind::BoundsCheck { index, length }
                if *index == coordinate && *length == limit
        ) {
            return Err(MslError::UnsupportedJirShape(
                "3D strided coordinate bounds check is invalid",
            ));
        }
    }
    let x_offset = get_value(
        &block.instructions[14],
        "x stride multiply must produce a value",
    )?;
    let y_offset = get_value(
        &block.instructions[15],
        "y stride multiply must produce a value",
    )?;
    if !matches!(
        &block.instructions[14].kind,
        InstructionKind::Binary { op: BinaryOp::Multiply, left, right }
            if *left == x.value && *right == metadata[3].value && x_offset.ty == x.ty
    ) || !matches!(
        &block.instructions[15].kind,
        InstructionKind::Binary { op: BinaryOp::Multiply, left, right }
            if *left == y.value && *right == metadata[4].value && y_offset.ty == x.ty
    ) {
        return Err(MslError::UnsupportedJirShape(
            "3D affine x/y stride multiply is invalid",
        ));
    }
    let xy_offset = get_value(
        &block.instructions[16],
        "x/y affine offset must produce a value",
    )?;
    if !matches!(
        &block.instructions[16].kind,
        InstructionKind::Binary { op: BinaryOp::Add, left, right }
            if *left == x_offset.value && *right == y_offset.value && xy_offset.ty == x.ty
    ) {
        return Err(MslError::UnsupportedJirShape(
            "3D affine x/y offset is invalid",
        ));
    }
    let z_offset = get_value(
        &block.instructions[17],
        "z stride multiply must produce a value",
    )?;
    if !matches!(
        &block.instructions[17].kind,
        InstructionKind::Binary { op: BinaryOp::Multiply, left, right }
            if *left == z.value && *right == metadata[5].value && z_offset.ty == x.ty
    ) {
        return Err(MslError::UnsupportedJirShape(
            "3D affine z stride multiply is invalid",
        ));
    }
    let physical = get_value(
        &block.instructions[18],
        "physical index must produce a value",
    )?;
    if !matches!(
        &block.instructions[18].kind,
        InstructionKind::Binary { op: BinaryOp::Add, left, right }
            if *left == xy_offset.value && *right == z_offset.value && physical.ty == x.ty
    ) {
        return Err(MslError::UnsupportedJirShape(
            "3D affine physical index is invalid",
        ));
    }
    if !matches!(
        &block.instructions[19].kind,
        InstructionKind::BoundsCheck { index, length }
            if *index == physical.value && *length == metadata[6].value
    ) {
        return Err(MslError::UnsupportedJirShape(
            "3D strided physical capacity bounds check is invalid",
        ));
    }
    let pointer = get_value(
        &block.instructions[20],
        "data offset must produce a pointer",
    )?;
    if !matches!(
        &block.instructions[20].kind,
        InstructionKind::Offset { base, indices }
            if *base == function_data.parameters[0].value && indices.as_slice() == [physical.value]
    ) || !matches!(
        &block.instructions[21].kind,
        InstructionKind::Store { pointer: stored_pointer, value: stored_value, .. }
            if *stored_pointer == pointer.value && *stored_value == stored.value
    ) || !matches!(block.terminator, Terminator::Return { value: None })
    {
        return Err(MslError::UnsupportedJirShape(
            "3D affine strided store/return is invalid",
        ));
    }
    emit_storage_global_3d_strided_write(&function_data.name, options, value)
}

/// Emits the verified bounded `u32` add-one kernel as deterministic MSL.
///
/// The contract uses input/output storage buffers at bindings 0/1, a constant
/// length at binding 2, and `thread_position_in_grid` for bounds-safe indexing.
pub fn emit_storage_add(
    entry_name: &str,
    options: MslOptions,
    addend: u32,
) -> Result<String, MslError> {
    if !valid_identifier(entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    let [x, y, z] = options.workgroup_size;
    let max_threads = u64::from(x)
        .saturating_mul(u64::from(y))
        .saturating_mul(u64::from(z));
    Ok(format!(
        "#include <metal_stdlib>\n\nusing namespace metal;\n\nstruct JadrenParams {{\n    uint length;\n}};\n\nkernel void {entry_name}(\n    device const uint* input [[buffer(0)]],\n    device uint* output [[buffer(1)]],\n    constant JadrenParams& params [[buffer(2)]],\n    uint3 gid [[thread_position_in_grid]])\n    [[max_total_threads_per_threadgroup({max_threads})]] {{\n    if (gid.x < params.length) {{\n        output[gid.x] = input[gid.x] + {addend}u;\n    }}\n}}\n"
    ))
}

fn validate_u32_binary_operand(operation: BinaryOp, operand: u32) -> Result<(), MslError> {
    match operation {
        BinaryOp::Divide | BinaryOp::Remainder if operand == 0 => Err(
            MslError::UnsupportedJirShape("unsigned divisor/remainder must be non-zero"),
        ),
        BinaryOp::ShiftLeft | BinaryOp::ShiftRight if operand >= 32 => Err(
            MslError::UnsupportedJirShape("u32 shift operand must be smaller than 32"),
        ),
        _ => Ok(()),
    }
}

const fn u32_msl_operator(operation: BinaryOp) -> &'static str {
    match operation {
        BinaryOp::Add => "+",
        BinaryOp::Subtract => "-",
        BinaryOp::Multiply => "*",
        BinaryOp::Divide => "/",
        BinaryOp::Remainder => "%",
        BinaryOp::BitAnd => "&",
        BinaryOp::BitOr => "|",
        BinaryOp::BitXor => "^",
        BinaryOp::ShiftLeft => "<<",
        BinaryOp::ShiftRight => ">>",
    }
}

/// Emits the bounded runtime-length scalar `u32` binary source contract from
/// an already selected operation and constant operand.
pub fn emit_storage_binary(
    entry_name: &str,
    options: MslOptions,
    operation: BinaryOp,
    operand: u32,
) -> Result<String, MslError> {
    if !valid_identifier(entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    validate_u32_binary_operand(operation, operand)?;
    let [x, y, z] = options.workgroup_size;
    let max_threads = u64::from(x)
        .saturating_mul(u64::from(y))
        .saturating_mul(u64::from(z));
    Ok(format!(
        concat!(
            "#include <metal_stdlib>\n\nusing namespace metal;\n\n",
            "struct JadrenParams {{\n    uint length;\n}};\n\n",
            "kernel void {entry_name}(\n    device const uint* input [[buffer(0)]],\n",
            "    device uint* output [[buffer(1)]],\n",
            "    constant JadrenParams& params [[buffer(2)]],\n",
            "    uint3 gid [[thread_position_in_grid]])\n",
            "    [[max_total_threads_per_threadgroup({max_threads})]] {{\n",
            "    if (gid.x < params.length) {{\n",
            "        output[gid.x] = input[gid.x] {operator} {operand}u;\n",
            "    }}\n}}\n"
        ),
        entry_name = entry_name,
        max_threads = max_threads,
        operator = u32_msl_operator(operation),
        operand = operand,
    ))
}

/// Emits the verified bounded runtime-length `f32` add kernel as deterministic
/// MSL. The addend is supplied as IEEE-754 binary32 bits so the source
/// contract cannot silently round or change the constant during formatting.
pub fn emit_storage_f32_add(
    entry_name: &str,
    options: MslOptions,
    addend_bits: u32,
) -> Result<String, MslError> {
    emit_storage_f32_binary(entry_name, options, addend_bits, F32ArithmeticOp::Add)
}

/// Emits a verified bounded runtime-length scalar `f32` binary kernel as
/// deterministic MSL. The operand is supplied as IEEE-754 binary32 bits so
/// the source contract cannot silently round or change the constant.
pub fn emit_storage_f32_binary(
    entry_name: &str,
    options: MslOptions,
    operand_bits: u32,
    operation: F32ArithmeticOp,
) -> Result<String, MslError> {
    if !valid_identifier(entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    let [x, y, z] = options.workgroup_size;
    let max_threads = u64::from(x)
        .saturating_mul(u64::from(y))
        .saturating_mul(u64::from(z));
    Ok(format!(
        concat!(
            "#include <metal_stdlib>\n\nusing namespace metal;\n\n",
            "struct JadrenParams {{\n    uint length;\n}};\n\n",
            "kernel void {entry_name}(\n",
            "    device const float* input [[buffer(0)]],\n",
            "    device float* output [[buffer(1)]],\n",
            "    constant JadrenParams& params [[buffer(2)]],\n",
            "    uint3 gid [[thread_position_in_grid]])\n",
            "    [[max_total_threads_per_threadgroup({max_threads})]] {{\n",
            "    if (gid.x < params.length) {{\n",
            "        output[gid.x] = input[gid.x] {operator} as_type<float>({operand_bits}u);\n",
            "    }}\n}}\n"
        ),
        entry_name = entry_name,
        max_threads = max_threads,
        operator = f32_msl_operator(operation),
        operand_bits = operand_bits,
    ))
}

/// Emits the bounded runtime-length `f32x4` vector-add source contract.
///
/// The vector element is represented as one 16-byte `float4` storage element.
/// This is source-contract coverage for the future Metal adapter; it does not
/// invoke Metal or claim native dispatch on the current host.
pub fn emit_storage_vector_f32_add(
    entry_name: &str,
    options: MslOptions,
    addend_bits: u32,
) -> Result<String, MslError> {
    emit_storage_vector_f32_binary(entry_name, options, addend_bits, F32ArithmeticOp::Add)
}

/// Validates the narrow scalar runtime-length `f32` artifact subset accepted
/// by the MSL source bridge. Unknown SPIR-V shapes are rejected before source
/// emission rather than being approximated as scalar arithmetic.
pub fn validate_storage_f32_binary_artifact(
    artifact: &SpirvArtifact,
    operand_bits: u32,
    operation: F32ArithmeticOp,
) -> Result<(), MslError> {
    if !valid_identifier(&artifact.entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    artifact
        .validate()
        .map_err(|_| MslError::UnsupportedJirShape("invalid SPIR-V artifact words"))?;
    if artifact.resources.len() != 3
        || artifact
            .resources
            .iter()
            .enumerate()
            .any(|(index, resource)| {
                resource.binding != index as u32 || resource.address_space != AddressSpace::Storage
            })
        || artifact
            .resources
            .iter()
            .any(|resource| resource.element_stride != Some(4))
    {
        return Err(MslError::UnsupportedJirShape(
            "scalar f32 artifact requires three ordered storage stride-4 resources",
        ));
    }

    const OP_TYPE_FLOAT: u16 = 22;
    const OP_CONSTANT: u16 = 43;
    const OP_FADD: u16 = 129;
    const OP_FSUB: u16 = 131;
    const OP_FMUL: u16 = 133;
    const OP_ULT: u16 = 176;
    const OP_STORE: u16 = 62;
    let expected_opcode = match operation {
        F32ArithmeticOp::Add => OP_FADD,
        F32ArithmeticOp::Subtract => OP_FSUB,
        F32ArithmeticOp::Multiply => OP_FMUL,
    };
    let mut float_types = Vec::new();
    let mut scalar_constants = Vec::new();
    let mut binary_count = 0_u32;
    let mut bounds_count = 0_u32;
    let mut store_count = 0_u32;
    let mut cursor = 5_usize;
    while cursor < artifact.words.len() {
        let instruction = artifact.words[cursor];
        let word_count = (instruction >> 16) as usize;
        let opcode = (instruction & 0xFFFF) as u16;
        if word_count == 0 || cursor + word_count > artifact.words.len() {
            return Err(MslError::UnsupportedJirShape(
                "scalar f32 artifact instruction stream is truncated",
            ));
        }
        let operands = &artifact.words[cursor + 1..cursor + word_count];
        match opcode {
            OP_TYPE_FLOAT if operands.len() >= 2 && operands[1] == 32 => {
                float_types.push(operands[0]);
            }
            OP_CONSTANT if operands.len() >= 3 => {
                scalar_constants.push((operands[0], operands[1], operands[2]));
            }
            OP_FADD | OP_FSUB | OP_FMUL if operands.len() >= 4 => {
                if opcode != expected_opcode {
                    return Err(MslError::UnsupportedJirShape(
                        "scalar f32 artifact operation differs from request",
                    ));
                }
                if !float_types.contains(&operands[0])
                    || !scalar_constants.iter().any(|(ty, id, bits)| {
                        *ty == operands[0] && *id == operands[3] && *bits == operand_bits
                    })
                {
                    return Err(MslError::UnsupportedJirShape(
                        "scalar f32 artifact operation operand is not the requested constant",
                    ));
                }
                binary_count += 1;
            }
            OP_ULT => bounds_count += 1,
            OP_STORE => store_count += 1,
            _ => {}
        }
        cursor += word_count;
    }
    if float_types.is_empty() {
        return Err(MslError::UnsupportedJirShape(
            "scalar f32 artifact has no 32-bit float type",
        ));
    }
    if binary_count != 1 || bounds_count != 1 || store_count != 1 {
        return Err(MslError::UnsupportedJirShape(
            "scalar f32 artifact requires one binary operation, bounds predicate and store",
        ));
    }
    Ok(())
}

/// Validates the narrow runtime-length `u32` BinaryOp artifact subset accepted
/// by the MSL source bridge. The operation, constant operand and storage
/// layout must match before any MSL source is generated.
pub fn validate_storage_binary_artifact(
    artifact: &SpirvArtifact,
    operation: BinaryOp,
    operand: u32,
) -> Result<(), MslError> {
    if !valid_identifier(&artifact.entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    validate_u32_binary_operand(operation, operand)?;
    artifact
        .validate()
        .map_err(|_| MslError::UnsupportedJirShape("invalid SPIR-V artifact words"))?;
    if artifact.resources.len() != 3
        || artifact
            .resources
            .iter()
            .enumerate()
            .any(|(index, resource)| {
                resource.binding != index as u32 || resource.address_space != AddressSpace::Storage
            })
        || artifact
            .resources
            .iter()
            .any(|resource| resource.element_stride != Some(4))
    {
        return Err(MslError::UnsupportedJirShape(
            "u32 artifact requires three ordered storage stride-4 resources",
        ));
    }

    const OP_TYPE_INT: u16 = 21;
    const OP_CONSTANT: u16 = 43;
    const OP_ULT: u16 = 176;
    const OP_STORE: u16 = 62;
    const OP_IADD: u16 = 128;
    const OP_ISUB: u16 = 130;
    const OP_IMUL: u16 = 132;
    const OP_UDIV: u16 = 134;
    const OP_UMOD: u16 = 137;
    const OP_SHIFT_RIGHT_LOGICAL: u16 = 194;
    const OP_SHIFT_LEFT_LOGICAL: u16 = 196;
    const OP_BITWISE_OR: u16 = 197;
    const OP_BITWISE_XOR: u16 = 198;
    const OP_BITWISE_AND: u16 = 199;
    let expected_opcode = match operation {
        BinaryOp::Add => OP_IADD,
        BinaryOp::Subtract => OP_ISUB,
        BinaryOp::Multiply => OP_IMUL,
        BinaryOp::Divide => OP_UDIV,
        BinaryOp::Remainder => OP_UMOD,
        BinaryOp::BitAnd => OP_BITWISE_AND,
        BinaryOp::BitOr => OP_BITWISE_OR,
        BinaryOp::BitXor => OP_BITWISE_XOR,
        BinaryOp::ShiftLeft => OP_SHIFT_LEFT_LOGICAL,
        BinaryOp::ShiftRight => OP_SHIFT_RIGHT_LOGICAL,
    };
    let mut integer_types = Vec::new();
    let mut scalar_constants = Vec::new();
    let mut binary_count = 0_u32;
    let mut bounds_count = 0_u32;
    let mut store_count = 0_u32;
    let mut cursor = 5_usize;
    while cursor < artifact.words.len() {
        let instruction = artifact.words[cursor];
        let word_count = (instruction >> 16) as usize;
        let opcode = (instruction & 0xFFFF) as u16;
        if word_count == 0 || cursor + word_count > artifact.words.len() {
            return Err(MslError::UnsupportedJirShape(
                "u32 artifact instruction stream is truncated",
            ));
        }
        let operands = &artifact.words[cursor + 1..cursor + word_count];
        match opcode {
            OP_TYPE_INT if operands.len() >= 3 && operands[1] == 32 && operands[2] == 0 => {
                integer_types.push(operands[0]);
            }
            OP_CONSTANT if operands.len() >= 3 => {
                scalar_constants.push((operands[0], operands[1], operands[2]));
            }
            OP_IADD
            | OP_ISUB
            | OP_IMUL
            | OP_UDIV
            | OP_UMOD
            | OP_SHIFT_RIGHT_LOGICAL
            | OP_SHIFT_LEFT_LOGICAL
            | OP_BITWISE_OR
            | OP_BITWISE_XOR
            | OP_BITWISE_AND
                if operands.len() >= 4 =>
            {
                if opcode != expected_opcode {
                    return Err(MslError::UnsupportedJirShape(
                        "u32 artifact operation differs from request",
                    ));
                }
                if !integer_types.contains(&operands[0])
                    || !operands[2..].iter().any(|value_id| {
                        scalar_constants.iter().any(|(ty, id, bits)| {
                            *ty == operands[0] && *id == *value_id && *bits == operand
                        })
                    })
                {
                    return Err(MslError::UnsupportedJirShape(
                        "u32 artifact operation lacks the requested constant operand",
                    ));
                }
                binary_count += 1;
            }
            OP_ULT => bounds_count += 1,
            OP_STORE => store_count += 1,
            _ => {}
        }
        cursor += word_count;
    }
    if integer_types.is_empty() {
        return Err(MslError::UnsupportedJirShape(
            "u32 artifact has no unsigned 32-bit integer type",
        ));
    }
    if binary_count != 1 || bounds_count != 1 || store_count != 1 {
        return Err(MslError::UnsupportedJirShape(
            "u32 artifact requires one binary operation, bounds predicate and store",
        ));
    }
    Ok(())
}

/// Validates the narrow one-resource `global_write_u32` artifact accepted by
/// the MSL source bridge. The validator checks the complete structured
/// bounds-before-store body rather than merely counting a bounds opcode and a
/// store, so an arbitrary one-UAV SPIR-V module cannot be mistaken for the
/// verified JIR shape.
pub fn validate_storage_global_write_artifact(
    artifact: &SpirvArtifact,
    value: u32,
    length: u32,
) -> Result<(), MslError> {
    if !valid_identifier(&artifact.entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    if length == 0 {
        return Err(MslError::UnsupportedJirShape(
            "global-write artifact length must be positive",
        ));
    }
    artifact
        .validate()
        .map_err(|_| MslError::UnsupportedJirShape("invalid SPIR-V artifact words"))?;
    if artifact.resources.len() != 1
        || artifact.resources[0].binding != 0
        || artifact.resources[0].address_space != AddressSpace::Storage
        || artifact.resources[0].element_stride != Some(4)
    {
        return Err(MslError::UnsupportedJirShape(
            "global-write artifact requires one storage stride-4 resource at binding zero",
        ));
    }

    const OP_CAPABILITY: u16 = 17;
    const OP_MEMORY_MODEL: u16 = 14;
    const OP_ENTRY_POINT: u16 = 15;
    const OP_EXECUTION_MODE: u16 = 16;
    const OP_TYPE_VOID: u16 = 19;
    const OP_TYPE_BOOL: u16 = 20;
    const OP_TYPE_INT: u16 = 21;
    const OP_TYPE_VECTOR: u16 = 23;
    const OP_TYPE_RUNTIME_ARRAY: u16 = 29;
    const OP_TYPE_STRUCT: u16 = 30;
    const OP_TYPE_POINTER: u16 = 32;
    const OP_TYPE_FUNCTION: u16 = 33;
    const OP_CONSTANT: u16 = 43;
    const OP_LOAD: u16 = 61;
    const OP_ACCESS_CHAIN: u16 = 65;
    const OP_COMPOSITE_EXTRACT: u16 = 81;
    const OP_STORE: u16 = 62;
    const OP_ULT: u16 = 176;
    const OP_FUNCTION: u16 = 54;
    const OP_LABEL: u16 = 248;
    const OP_SELECTION_MERGE: u16 = 247;
    const OP_BRANCH: u16 = 249;
    const OP_BRANCH_CONDITIONAL: u16 = 250;
    const OP_RETURN: u16 = 253;
    const OP_FUNCTION_END: u16 = 56;
    const STORAGE_BUFFER: u32 = 12;
    const INPUT: u32 = 1;
    const DECORATION_BINDING: u32 = 33;
    const DECORATION_DESCRIPTOR_SET: u32 = 34;
    const DECORATION_BUILT_IN: u32 = 11;
    const BUILT_IN_GLOBAL_INVOCATION_ID: u32 = 28;
    const EXECUTION_MODEL_GL_COMPUTE: u32 = 5;
    const EXECUTION_MODE_LOCAL_SIZE: u32 = 17;

    let invalid =
        || MslError::UnsupportedJirShape("global-write artifact has an unsupported shape");
    let mut uint_types = Vec::new();
    let mut bool_types = Vec::new();
    let mut vector_types = Vec::new();
    let mut runtime_arrays = Vec::new();
    let mut structs = Vec::new();
    let mut pointer_types = Vec::new();
    let mut constants = Vec::new();
    let mut variables = Vec::new();
    let mut descriptor_sets = Vec::new();
    let mut bindings = Vec::new();
    let mut builtin_variables = Vec::new();
    let mut entry_points = Vec::new();
    let mut execution_modes = Vec::new();
    let mut function_declarations = Vec::new();
    let mut function_type_declarations = Vec::new();
    let mut void_types = Vec::new();
    let mut capability_count = 0_u32;
    let mut memory_model_count = 0_u32;
    let mut function_body = Vec::new();
    let mut in_function = false;

    let mut cursor = 5_usize;
    while cursor < artifact.words.len() {
        let instruction = artifact.words[cursor];
        let word_count = (instruction >> 16) as usize;
        let opcode = (instruction & 0xFFFF) as u16;
        if word_count == 0 || cursor + word_count > artifact.words.len() {
            return Err(invalid());
        }
        let operands = &artifact.words[cursor + 1..cursor + word_count];
        if in_function {
            if opcode == OP_FUNCTION {
                return Err(invalid());
            }
            function_body.push((opcode, operands.to_vec()));
            if opcode == OP_FUNCTION_END {
                in_function = false;
            }
            cursor += word_count;
            continue;
        }
        match opcode {
            OP_CAPABILITY if operands == [1] => capability_count += 1,
            OP_MEMORY_MODEL if operands == [0, 1] => memory_model_count += 1,
            OP_ENTRY_POINT if operands.len() >= 2 => {
                entry_points.push((operands[0], operands[1]));
            }
            OP_EXECUTION_MODE if operands.len() == 5 => execution_modes.push(operands.to_vec()),
            OP_TYPE_VOID if operands.len() == 1 => void_types.push(operands[0]),
            OP_TYPE_BOOL if operands.len() == 1 => bool_types.push(operands[0]),
            OP_TYPE_INT if operands.len() == 3 => {
                if operands[1] != 32 || operands[2] != 0 {
                    return Err(invalid());
                }
                uint_types.push(operands[0]);
            }
            OP_TYPE_VECTOR if operands.len() == 3 => {
                vector_types.push((operands[0], operands[1], operands[2]))
            }
            OP_TYPE_RUNTIME_ARRAY if operands.len() == 2 => {
                runtime_arrays.push((operands[0], operands[1]));
            }
            OP_TYPE_STRUCT if operands.len() == 2 => {
                structs.push((operands[0], operands[1]));
            }
            OP_TYPE_POINTER if operands.len() == 3 => {
                pointer_types.push((operands[0], operands[1], operands[2]));
            }
            OP_TYPE_FUNCTION if operands.len() == 2 => {
                function_type_declarations.push((operands[0], operands[1]));
            }
            OP_CONSTANT if operands.len() == 3 => {
                constants.push((operands[0], operands[1], operands[2]));
            }
            OP_ACCESS_CHAIN
            | OP_LOAD
            | OP_COMPOSITE_EXTRACT
            | OP_STORE
            | OP_ULT
            | OP_LABEL
            | OP_SELECTION_MERGE
            | OP_BRANCH
            | OP_BRANCH_CONDITIONAL
            | OP_RETURN
            | OP_FUNCTION_END => return Err(invalid()),
            OP_FUNCTION if operands.len() == 4 => {
                function_declarations.push(operands.to_vec());
                in_function = true;
            }
            59 if operands.len() == 3 => variables.push(operands.to_vec()),
            71 if operands.len() >= 2 => match operands[1] {
                DECORATION_DESCRIPTOR_SET => {
                    descriptor_sets.push((operands[0], operands[2..].to_vec()))
                }
                DECORATION_BINDING => bindings.push((operands[0], operands[2..].to_vec())),
                DECORATION_BUILT_IN => {
                    builtin_variables.push((operands[0], operands[2..].to_vec()))
                }
                _ => {}
            },
            _ => {}
        }
        cursor += word_count;
    }
    if in_function
        || capability_count != 1
        || memory_model_count != 1
        || entry_points.len() != 1
        || execution_modes.len() != 1
        || void_types.len() != 1
        || uint_types.len() != 1
        || bool_types.len() != 1
        || vector_types.len() != 1
        || runtime_arrays.len() != 1
        || structs.len() != 1
        || pointer_types.len() != 3
        || function_type_declarations.len() != 1
        || function_declarations.len() != 1
        || variables.len() != 2
        || constants.len() != 3
    {
        return Err(invalid());
    }

    let uint_type = uint_types[0];
    let bool_type = bool_types[0];
    let (vector_type, vector_element, vector_lanes) = vector_types[0];
    if vector_element != uint_type
        || vector_lanes != 3
        || runtime_arrays[0].1 != uint_type
        || structs[0].1 != runtime_arrays[0].0
        || function_type_declarations[0].1 != void_types[0]
        || function_declarations[0][0] != void_types[0]
        || function_declarations[0][2] != 0
        || function_declarations[0][3] != function_type_declarations[0].0
        || entry_points[0].0 != EXECUTION_MODEL_GL_COMPUTE
        || entry_points[0].1 != function_declarations[0][1]
        || execution_modes[0][0] != function_declarations[0][1]
        || execution_modes[0][1] != EXECUTION_MODE_LOCAL_SIZE
        || execution_modes[0][2..] != artifact.workgroup_size
    {
        return Err(invalid());
    }

    let storage_struct_pointer = pointer_types
        .iter()
        .find(|(_, storage, pointee)| *storage == STORAGE_BUFFER && *pointee == structs[0].0)
        .map(|(id, _, _)| *id);
    let element_pointer = pointer_types
        .iter()
        .find(|(_, storage, pointee)| *storage == STORAGE_BUFFER && *pointee == uint_type)
        .map(|(id, _, _)| *id);
    let input_pointer = pointer_types
        .iter()
        .find(|(_, storage, pointee)| *storage == INPUT && *pointee == vector_type)
        .map(|(id, _, _)| *id);
    let (Some(storage_struct_pointer), Some(element_pointer), Some(input_pointer)) =
        (storage_struct_pointer, element_pointer, input_pointer)
    else {
        return Err(invalid());
    };
    let Some(resource_variable) = variables.iter().find_map(|operands| {
        (operands.len() == 3
            && operands[0] == storage_struct_pointer
            && operands[2] == STORAGE_BUFFER)
            .then_some(operands[1])
    }) else {
        return Err(invalid());
    };
    let Some(global_variable) = variables.iter().find_map(|operands| {
        (operands.len() == 3 && operands[0] == input_pointer && operands[2] == INPUT)
            .then_some(operands[1])
    }) else {
        return Err(invalid());
    };
    if !descriptor_sets
        .iter()
        .any(|(target, values)| *target == resource_variable && values.as_slice() == [0])
        || !bindings
            .iter()
            .any(|(target, values)| *target == resource_variable && values.as_slice() == [0])
        || !builtin_variables.iter().any(|(target, values)| {
            *target == global_variable && values.as_slice() == [BUILT_IN_GLOBAL_INVOCATION_ID]
        })
    {
        return Err(invalid());
    }

    let find_constant = |id: u32| {
        constants
            .iter()
            .find(|(_, constant_id, _)| *constant_id == id)
    };
    if constants.iter().any(|(ty, _, _)| *ty != uint_type) {
        return Err(invalid());
    }

    let expected_body = [
        OP_LABEL,
        OP_LOAD,
        OP_COMPOSITE_EXTRACT,
        OP_ULT,
        OP_SELECTION_MERGE,
        OP_BRANCH_CONDITIONAL,
        OP_LABEL,
        OP_ACCESS_CHAIN,
        OP_STORE,
        OP_BRANCH,
        OP_LABEL,
        OP_RETURN,
        OP_FUNCTION_END,
    ];
    if function_body.len() != expected_body.len()
        || function_body
            .iter()
            .map(|(opcode, _)| *opcode)
            .collect::<Vec<_>>()
            .as_slice()
            != expected_body
    {
        return Err(invalid());
    }
    let labels = [
        function_body[0].1[0],
        function_body[6].1[0],
        function_body[10].1[0],
    ];
    let load = &function_body[1].1;
    let extract = &function_body[2].1;
    let bounds = &function_body[3].1;
    let selection_merge = &function_body[4].1;
    let branch_conditional = &function_body[5].1;
    let access_chain = &function_body[7].1;
    let store = &function_body[8].1;
    let branch = &function_body[9].1;
    let return_instruction = &function_body[11].1;
    let function_end = &function_body[12].1;
    if load.len() != 3
        || load[0] != vector_type
        || load[2] != global_variable
        || extract.as_slice() != [uint_type, extract[1], load[1], 0]
        || bounds.len() != 4
        || bounds[0] != bool_type
        || bounds[2] != extract[1]
        || selection_merge.as_slice() != [labels[2], 0]
        || branch_conditional.as_slice() != [bounds[1], labels[1], labels[2]]
        || access_chain.len() != 5
        || access_chain[0] != element_pointer
        || access_chain[2] != resource_variable
        || access_chain[3] == 0
        || access_chain[4] != extract[1]
        || store.len() != 2
        || store[0] != access_chain[1]
        || branch.as_slice() != [labels[2]]
        || !return_instruction.is_empty()
        || !function_end.is_empty()
    {
        return Err(invalid());
    }
    let zero_id = access_chain[3];
    let length_id = bounds[3];
    let value_id = store[1];
    let Some((_, _, zero)) = find_constant(zero_id) else {
        return Err(invalid());
    };
    let Some((_, _, encoded_length)) = find_constant(length_id) else {
        return Err(invalid());
    };
    let Some((_, _, encoded_value)) = find_constant(value_id) else {
        return Err(invalid());
    };
    if *zero != 0 || *encoded_length != length || *encoded_value != value {
        return Err(MslError::UnsupportedJirShape(
            "global-write artifact constants differ from execution request",
        ));
    }
    Ok(())
}

/// Lowers a validated one-resource `global_write_u32` artifact to the MSL
/// source contract. The workgroup metadata must remain identical across the
/// SPIR-V hand-off and the MSL options.
pub fn emit_storage_global_write_from_spirv_artifact(
    artifact: &SpirvArtifact,
    options: MslOptions,
    value: u32,
    length: u32,
) -> Result<String, MslError> {
    validate_storage_global_write_artifact(artifact, value, length)?;
    if artifact.workgroup_size != options.workgroup_size {
        return Err(MslError::UnsupportedJirShape(
            "MSL options do not match SPIR-V artifact workgroup",
        ));
    }
    emit_storage_global_write(&artifact.entry_name, options, value, length)
}

/// Validates the four-resource runtime-stride `global_strided_write_u32`
/// artifact accepted by the MSL source bridge. The two bounds predicates and
/// the multiply must remain in the structured order emitted by the SPIR-V
/// exporter; a loose opcode count is not sufficient for this resource shape.
pub fn validate_storage_global_strided_write_artifact(
    artifact: &SpirvArtifact,
    value: u32,
) -> Result<(), MslError> {
    if !valid_identifier(&artifact.entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    artifact
        .validate()
        .map_err(|_| MslError::UnsupportedJirShape("invalid SPIR-V artifact words"))?;
    if artifact.resources.len() != 4
        || artifact
            .resources
            .iter()
            .enumerate()
            .any(|(index, resource)| {
                resource.binding != index as u32
                    || resource.address_space != AddressSpace::Storage
                    || resource.element_stride != Some(4)
            })
    {
        return Err(MslError::UnsupportedJirShape(
            "global-strided-write artifact requires four ordered storage stride-4 resources",
        ));
    }

    const OP_CAPABILITY: u16 = 17;
    const OP_MEMORY_MODEL: u16 = 14;
    const OP_ENTRY_POINT: u16 = 15;
    const OP_EXECUTION_MODE: u16 = 16;
    const OP_TYPE_VOID: u16 = 19;
    const OP_TYPE_BOOL: u16 = 20;
    const OP_TYPE_INT: u16 = 21;
    const OP_TYPE_VECTOR: u16 = 23;
    const OP_TYPE_RUNTIME_ARRAY: u16 = 29;
    const OP_TYPE_STRUCT: u16 = 30;
    const OP_TYPE_POINTER: u16 = 32;
    const OP_TYPE_FUNCTION: u16 = 33;
    const OP_CONSTANT: u16 = 43;
    const OP_LOAD: u16 = 61;
    const OP_ACCESS_CHAIN: u16 = 65;
    const OP_IMUL: u16 = 132;
    const OP_COMPOSITE_EXTRACT: u16 = 81;
    const OP_STORE: u16 = 62;
    const OP_ULT: u16 = 176;
    const OP_FUNCTION: u16 = 54;
    const OP_LABEL: u16 = 248;
    const OP_SELECTION_MERGE: u16 = 247;
    const OP_BRANCH: u16 = 249;
    const OP_BRANCH_CONDITIONAL: u16 = 250;
    const OP_RETURN: u16 = 253;
    const OP_FUNCTION_END: u16 = 56;
    const OP_VARIABLE: u16 = 59;
    const OP_DECORATE: u16 = 71;
    const STORAGE_BUFFER: u32 = 12;
    const INPUT: u32 = 1;
    const DECORATION_BINDING: u32 = 33;
    const DECORATION_DESCRIPTOR_SET: u32 = 34;
    const DECORATION_BUILT_IN: u32 = 11;
    const BUILT_IN_GLOBAL_INVOCATION_ID: u32 = 28;
    const EXECUTION_MODEL_GL_COMPUTE: u32 = 5;
    const EXECUTION_MODE_LOCAL_SIZE: u32 = 17;

    let invalid =
        || MslError::UnsupportedJirShape("global-strided-write artifact has an unsupported shape");
    let mut uint_types = Vec::new();
    let mut bool_types = Vec::new();
    let mut vector_types = Vec::new();
    let mut runtime_arrays = Vec::new();
    let mut structs = Vec::new();
    let mut pointer_types = Vec::new();
    let mut constants = Vec::new();
    let mut variables = Vec::new();
    let mut descriptor_sets = Vec::new();
    let mut bindings = Vec::new();
    let mut builtin_variables = Vec::new();
    let mut entry_points = Vec::new();
    let mut execution_modes = Vec::new();
    let mut function_declarations = Vec::new();
    let mut function_type_declarations = Vec::new();
    let mut void_types = Vec::new();
    let mut capability_count = 0_u32;
    let mut memory_model_count = 0_u32;
    let mut function_body = Vec::new();
    let mut in_function = false;

    let mut cursor = 5_usize;
    while cursor < artifact.words.len() {
        let instruction = artifact.words[cursor];
        let word_count = (instruction >> 16) as usize;
        let opcode = (instruction & 0xFFFF) as u16;
        if word_count == 0 || cursor + word_count > artifact.words.len() {
            return Err(invalid());
        }
        let operands = &artifact.words[cursor + 1..cursor + word_count];
        if in_function {
            if opcode == OP_FUNCTION {
                return Err(invalid());
            }
            function_body.push((opcode, operands.to_vec()));
            if opcode == OP_FUNCTION_END {
                in_function = false;
            }
            cursor += word_count;
            continue;
        }
        match opcode {
            OP_CAPABILITY if operands == [1] => capability_count += 1,
            OP_MEMORY_MODEL if operands == [0, 1] => memory_model_count += 1,
            OP_ENTRY_POINT if operands.len() >= 2 => {
                entry_points.push((operands[0], operands[1]));
            }
            OP_EXECUTION_MODE if operands.len() == 5 => execution_modes.push(operands.to_vec()),
            OP_TYPE_VOID if operands.len() == 1 => void_types.push(operands[0]),
            OP_TYPE_BOOL if operands.len() == 1 => bool_types.push(operands[0]),
            OP_TYPE_INT if operands.len() == 3 => {
                if operands[1] != 32 || operands[2] != 0 {
                    return Err(invalid());
                }
                uint_types.push(operands[0]);
            }
            OP_TYPE_VECTOR if operands.len() == 3 => {
                vector_types.push((operands[0], operands[1], operands[2]))
            }
            OP_TYPE_RUNTIME_ARRAY if operands.len() == 2 => {
                runtime_arrays.push((operands[0], operands[1]));
            }
            OP_TYPE_STRUCT if operands.len() == 2 => {
                structs.push((operands[0], operands[1]));
            }
            OP_TYPE_POINTER if operands.len() == 3 => {
                pointer_types.push((operands[0], operands[1], operands[2]));
            }
            OP_TYPE_FUNCTION if operands.len() == 2 => {
                function_type_declarations.push((operands[0], operands[1]));
            }
            OP_CONSTANT if operands.len() == 3 => {
                constants.push((operands[0], operands[1], operands[2]));
            }
            OP_FUNCTION if operands.len() == 4 => {
                function_declarations.push(operands.to_vec());
                in_function = true;
            }
            OP_VARIABLE if operands.len() == 3 => variables.push(operands.to_vec()),
            OP_DECORATE if operands.len() >= 2 => match operands[1] {
                DECORATION_DESCRIPTOR_SET => {
                    descriptor_sets.push((operands[0], operands[2..].to_vec()))
                }
                DECORATION_BINDING => bindings.push((operands[0], operands[2..].to_vec())),
                DECORATION_BUILT_IN => {
                    builtin_variables.push((operands[0], operands[2..].to_vec()))
                }
                _ => {}
            },
            OP_ACCESS_CHAIN
            | OP_LOAD
            | OP_COMPOSITE_EXTRACT
            | OP_IMUL
            | OP_STORE
            | OP_ULT
            | OP_LABEL
            | OP_SELECTION_MERGE
            | OP_BRANCH
            | OP_BRANCH_CONDITIONAL
            | OP_RETURN
            | OP_FUNCTION_END => return Err(invalid()),
            _ => {}
        }
        cursor += word_count;
    }
    if in_function
        || capability_count != 1
        || memory_model_count != 1
        || entry_points.len() != 1
        || execution_modes.len() != 1
        || void_types.len() != 1
        || uint_types.len() != 1
        || bool_types.len() != 1
        || vector_types.len() != 1
        || runtime_arrays.len() != 1
        || structs.len() != 1
        || pointer_types.len() != 3
        || function_type_declarations.len() != 1
        || function_declarations.len() != 1
        || variables.len() != 5
        || constants.len() != 2
    {
        return Err(invalid());
    }

    let uint_type = uint_types[0];
    let bool_type = bool_types[0];
    let (vector_type, vector_element, vector_lanes) = vector_types[0];
    if vector_element != uint_type
        || vector_lanes != 3
        || runtime_arrays[0].1 != uint_type
        || structs[0].1 != runtime_arrays[0].0
        || function_type_declarations[0].1 != void_types[0]
        || function_declarations[0][0] != void_types[0]
        || function_declarations[0][2] != 0
        || function_declarations[0][3] != function_type_declarations[0].0
        || entry_points[0].0 != EXECUTION_MODEL_GL_COMPUTE
        || entry_points[0].1 != function_declarations[0][1]
        || execution_modes[0][0] != function_declarations[0][1]
        || execution_modes[0][1] != EXECUTION_MODE_LOCAL_SIZE
        || execution_modes[0][2..] != artifact.workgroup_size
    {
        return Err(invalid());
    }

    let storage_struct_pointer = pointer_types
        .iter()
        .find(|(_, storage, pointee)| *storage == STORAGE_BUFFER && *pointee == structs[0].0)
        .map(|(id, _, _)| *id);
    let element_pointer = pointer_types
        .iter()
        .find(|(_, storage, pointee)| *storage == STORAGE_BUFFER && *pointee == uint_type)
        .map(|(id, _, _)| *id);
    let input_pointer = pointer_types
        .iter()
        .find(|(_, storage, pointee)| *storage == INPUT && *pointee == vector_type)
        .map(|(id, _, _)| *id);
    let (Some(storage_struct_pointer), Some(element_pointer), Some(input_pointer)) =
        (storage_struct_pointer, element_pointer, input_pointer)
    else {
        return Err(invalid());
    };
    let resource_variables: Vec<u32> = variables
        .iter()
        .filter(|operands| operands[0] == storage_struct_pointer && operands[2] == STORAGE_BUFFER)
        .map(|operands| operands[1])
        .collect();
    if resource_variables.len() != 4 {
        return Err(invalid());
    }
    let Some(global_variable) = variables.iter().find_map(|operands| {
        (operands[0] == input_pointer && operands[2] == INPUT).then_some(operands[1])
    }) else {
        return Err(invalid());
    };
    let mut bound_resources = [0_u32; 4];
    for (binding, slot) in bound_resources.iter_mut().enumerate() {
        let binding = binding as u32;
        let Some(variable) = bindings
            .iter()
            .find_map(|(target, values)| (values.as_slice() == [binding]).then_some(*target))
        else {
            return Err(invalid());
        };
        if !resource_variables.contains(&variable)
            || !descriptor_sets
                .iter()
                .any(|(target, values)| *target == variable && values.as_slice() == [0])
        {
            return Err(invalid());
        }
        *slot = variable;
    }
    if !builtin_variables.iter().any(|(target, values)| {
        *target == global_variable && values.as_slice() == [BUILT_IN_GLOBAL_INVOCATION_ID]
    }) {
        return Err(invalid());
    }
    let find_constant = |id: u32| {
        constants
            .iter()
            .find(|(_, constant_id, _)| *constant_id == id)
    };
    if constants.iter().any(|(ty, _, _)| *ty != uint_type) {
        return Err(invalid());
    }

    let expected_body = [
        OP_LABEL,
        OP_LOAD,
        OP_COMPOSITE_EXTRACT,
        OP_ACCESS_CHAIN,
        OP_LOAD,
        OP_ACCESS_CHAIN,
        OP_LOAD,
        OP_ACCESS_CHAIN,
        OP_LOAD,
        OP_ULT,
        OP_SELECTION_MERGE,
        OP_BRANCH_CONDITIONAL,
        OP_LABEL,
        OP_IMUL,
        OP_ULT,
        OP_SELECTION_MERGE,
        OP_BRANCH_CONDITIONAL,
        OP_LABEL,
        OP_ACCESS_CHAIN,
        OP_STORE,
        OP_BRANCH,
        OP_LABEL,
        OP_BRANCH,
        OP_LABEL,
        OP_RETURN,
        OP_FUNCTION_END,
    ];
    if function_body.len() != expected_body.len()
        || function_body
            .iter()
            .map(|(opcode, _)| *opcode)
            .collect::<Vec<_>>()
            .as_slice()
            != expected_body
    {
        return Err(invalid());
    }
    let labels = [
        function_body[0].1[0],
        function_body[12].1[0],
        function_body[17].1[0],
        function_body[21].1[0],
        function_body[23].1[0],
    ];
    if labels
        .iter()
        .enumerate()
        .any(|(index, label)| labels[..index].contains(label))
    {
        return Err(invalid());
    }
    let load_global = &function_body[1].1;
    let extract = &function_body[2].1;
    if load_global.len() != 3
        || load_global[0] != vector_type
        || load_global[2] != global_variable
        || extract.len() != 4
        || extract[0] != uint_type
        || extract[2] != load_global[1]
        || extract[3] != 0
    {
        return Err(invalid());
    }
    let index = extract[1];
    let zero_id = function_body[3].1[3];
    let length_address = function_body[3].1[1];
    let length_value = function_body[4].1[1];
    let stride_address = function_body[5].1[1];
    let stride_value = function_body[6].1[1];
    let capacity_address = function_body[7].1[1];
    let capacity_value = function_body[8].1[1];
    if function_body[3].1.as_slice()
        != [
            element_pointer,
            length_address,
            bound_resources[1],
            zero_id,
            zero_id,
        ]
        || function_body[4].1.as_slice() != [uint_type, length_value, length_address]
        || function_body[5].1.as_slice()
            != [
                element_pointer,
                stride_address,
                bound_resources[2],
                zero_id,
                zero_id,
            ]
        || function_body[6].1.as_slice() != [uint_type, stride_value, stride_address]
        || function_body[7].1.as_slice()
            != [
                element_pointer,
                capacity_address,
                bound_resources[3],
                zero_id,
                zero_id,
            ]
        || function_body[8].1.as_slice() != [uint_type, capacity_value, capacity_address]
        || function_body[9].1.len() != 4
        || function_body[9].1[0] != bool_type
        || function_body[9].1[2] != index
        || function_body[10].1.as_slice() != [labels[4], 0]
        || function_body[11].1.as_slice() != [function_body[9].1[1], labels[1], labels[4]]
        || function_body[13].1.len() != 4
        || function_body[13].1[0] != uint_type
        || function_body[13].1[2] != index
        || function_body[13].1[3] != stride_value
        || function_body[14].1.len() != 4
        || function_body[14].1[0] != bool_type
        || function_body[14].1[2] != function_body[13].1[1]
        || function_body[14].1[3] != capacity_value
        || function_body[15].1.as_slice() != [labels[3], 0]
        || function_body[16].1.as_slice() != [function_body[14].1[1], labels[2], labels[3]]
        || function_body[18].1.len() != 5
        || function_body[18].1[0] != element_pointer
        || function_body[18].1[2] != bound_resources[0]
        || function_body[18].1[3] != zero_id
        || function_body[18].1[4] != function_body[13].1[1]
        || function_body[19].1.len() != 2
        || function_body[19].1[0] != function_body[18].1[1]
        || function_body[20].1.as_slice() != [labels[3]]
        || function_body[22].1.as_slice() != [labels[4]]
        || !function_body[24].1.is_empty()
        || !function_body[25].1.is_empty()
    {
        return Err(invalid());
    }
    let value_id = function_body[19].1[1];
    let Some((_, _, zero)) = find_constant(zero_id) else {
        return Err(invalid());
    };
    let Some((_, _, encoded_value)) = find_constant(value_id) else {
        return Err(invalid());
    };
    if value_id == zero_id || *zero != 0 || *encoded_value != value {
        return Err(MslError::UnsupportedJirShape(
            "global-strided-write artifact value differs from execution request",
        ));
    }
    Ok(())
}

/// Lowers a validated runtime-stride `global_strided_write_u32` artifact to
/// MSL while preserving the artifact workgroup metadata.
pub fn emit_storage_global_strided_write_from_spirv_artifact(
    artifact: &SpirvArtifact,
    options: MslOptions,
    value: u32,
) -> Result<String, MslError> {
    validate_storage_global_strided_write_artifact(artifact, value)?;
    if artifact.workgroup_size != options.workgroup_size {
        return Err(MslError::UnsupportedJirShape(
            "MSL options do not match SPIR-V artifact workgroup",
        ));
    }
    emit_storage_global_strided_write(&artifact.entry_name, options, value)
}

/// Validates the four-resource row-major `global_2d_write_u32` artifact
/// accepted by the MSL source bridge. Coordinate guards must dominate the
/// row-major flattening, and the flattened index must pass a capacity guard
/// before the indexed store.
pub fn validate_storage_global_2d_write_artifact(
    artifact: &SpirvArtifact,
    value: u32,
) -> Result<(), MslError> {
    if !valid_identifier(&artifact.entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    artifact
        .validate()
        .map_err(|_| MslError::UnsupportedJirShape("invalid SPIR-V artifact words"))?;
    if artifact.resources.len() != 4
        || artifact
            .resources
            .iter()
            .enumerate()
            .any(|(index, resource)| {
                resource.binding != index as u32
                    || resource.address_space != AddressSpace::Storage
                    || resource.element_stride != Some(4)
            })
    {
        return Err(MslError::UnsupportedJirShape(
            "2D global-write artifact requires four ordered storage stride-4 resources",
        ));
    }

    const OP_CAPABILITY: u16 = 17;
    const OP_MEMORY_MODEL: u16 = 14;
    const OP_ENTRY_POINT: u16 = 15;
    const OP_EXECUTION_MODE: u16 = 16;
    const OP_TYPE_VOID: u16 = 19;
    const OP_TYPE_BOOL: u16 = 20;
    const OP_TYPE_INT: u16 = 21;
    const OP_TYPE_VECTOR: u16 = 23;
    const OP_TYPE_RUNTIME_ARRAY: u16 = 29;
    const OP_TYPE_STRUCT: u16 = 30;
    const OP_TYPE_POINTER: u16 = 32;
    const OP_TYPE_FUNCTION: u16 = 33;
    const OP_CONSTANT: u16 = 43;
    const OP_LOAD: u16 = 61;
    const OP_ACCESS_CHAIN: u16 = 65;
    const OP_IADD: u16 = 128;
    const OP_IMUL: u16 = 132;
    const OP_LOGICAL_AND: u16 = 167;
    const OP_COMPOSITE_EXTRACT: u16 = 81;
    const OP_STORE: u16 = 62;
    const OP_ULT: u16 = 176;
    const OP_FUNCTION: u16 = 54;
    const OP_LABEL: u16 = 248;
    const OP_SELECTION_MERGE: u16 = 247;
    const OP_BRANCH: u16 = 249;
    const OP_BRANCH_CONDITIONAL: u16 = 250;
    const OP_RETURN: u16 = 253;
    const OP_FUNCTION_END: u16 = 56;
    const OP_VARIABLE: u16 = 59;
    const OP_DECORATE: u16 = 71;
    const STORAGE_BUFFER: u32 = 12;
    const INPUT: u32 = 1;
    const DECORATION_BINDING: u32 = 33;
    const DECORATION_DESCRIPTOR_SET: u32 = 34;
    const DECORATION_BUILT_IN: u32 = 11;
    const BUILT_IN_GLOBAL_INVOCATION_ID: u32 = 28;
    const EXECUTION_MODEL_GL_COMPUTE: u32 = 5;
    const EXECUTION_MODE_LOCAL_SIZE: u32 = 17;

    let invalid =
        || MslError::UnsupportedJirShape("2D global-write artifact has an unsupported shape");
    let mut uint_types = Vec::new();
    let mut bool_types = Vec::new();
    let mut vector_types = Vec::new();
    let mut runtime_arrays = Vec::new();
    let mut structs = Vec::new();
    let mut pointer_types = Vec::new();
    let mut constants = Vec::new();
    let mut variables = Vec::new();
    let mut descriptor_sets = Vec::new();
    let mut bindings = Vec::new();
    let mut builtin_variables = Vec::new();
    let mut entry_points = Vec::new();
    let mut execution_modes = Vec::new();
    let mut function_declarations = Vec::new();
    let mut function_type_declarations = Vec::new();
    let mut void_types = Vec::new();
    let mut capability_count = 0_u32;
    let mut memory_model_count = 0_u32;
    let mut function_body = Vec::new();
    let mut in_function = false;

    let mut cursor = 5_usize;
    while cursor < artifact.words.len() {
        let instruction = artifact.words[cursor];
        let word_count = (instruction >> 16) as usize;
        let opcode = (instruction & 0xFFFF) as u16;
        if word_count == 0 || cursor + word_count > artifact.words.len() {
            return Err(invalid());
        }
        let operands = &artifact.words[cursor + 1..cursor + word_count];
        if in_function {
            if opcode == OP_FUNCTION {
                return Err(invalid());
            }
            function_body.push((opcode, operands.to_vec()));
            if opcode == OP_FUNCTION_END {
                in_function = false;
            }
            cursor += word_count;
            continue;
        }
        match opcode {
            OP_CAPABILITY if operands == [1] => capability_count += 1,
            OP_MEMORY_MODEL if operands == [0, 1] => memory_model_count += 1,
            OP_ENTRY_POINT if operands.len() >= 2 => {
                entry_points.push((operands[0], operands[1]));
            }
            OP_EXECUTION_MODE if operands.len() == 5 => execution_modes.push(operands.to_vec()),
            OP_TYPE_VOID if operands.len() == 1 => void_types.push(operands[0]),
            OP_TYPE_BOOL if operands.len() == 1 => bool_types.push(operands[0]),
            OP_TYPE_INT if operands.len() == 3 => {
                if operands[1] != 32 || operands[2] != 0 {
                    return Err(invalid());
                }
                uint_types.push(operands[0]);
            }
            OP_TYPE_VECTOR if operands.len() == 3 => {
                vector_types.push((operands[0], operands[1], operands[2]))
            }
            OP_TYPE_RUNTIME_ARRAY if operands.len() == 2 => {
                runtime_arrays.push((operands[0], operands[1]));
            }
            OP_TYPE_STRUCT if operands.len() == 2 => {
                structs.push((operands[0], operands[1]));
            }
            OP_TYPE_POINTER if operands.len() == 3 => {
                pointer_types.push((operands[0], operands[1], operands[2]));
            }
            OP_TYPE_FUNCTION if operands.len() == 2 => {
                function_type_declarations.push((operands[0], operands[1]));
            }
            OP_CONSTANT if operands.len() == 3 => {
                constants.push((operands[0], operands[1], operands[2]));
            }
            OP_FUNCTION if operands.len() == 4 => {
                function_declarations.push(operands.to_vec());
                in_function = true;
            }
            OP_VARIABLE if operands.len() == 3 => variables.push(operands.to_vec()),
            OP_DECORATE if operands.len() >= 2 => match operands[1] {
                DECORATION_DESCRIPTOR_SET => {
                    descriptor_sets.push((operands[0], operands[2..].to_vec()))
                }
                DECORATION_BINDING => bindings.push((operands[0], operands[2..].to_vec())),
                DECORATION_BUILT_IN => {
                    builtin_variables.push((operands[0], operands[2..].to_vec()))
                }
                _ => {}
            },
            OP_ACCESS_CHAIN
            | OP_LOAD
            | OP_COMPOSITE_EXTRACT
            | OP_IADD
            | OP_IMUL
            | OP_LOGICAL_AND
            | OP_STORE
            | OP_ULT
            | OP_LABEL
            | OP_SELECTION_MERGE
            | OP_BRANCH
            | OP_BRANCH_CONDITIONAL
            | OP_RETURN
            | OP_FUNCTION_END => return Err(invalid()),
            _ => {}
        }
        cursor += word_count;
    }
    if in_function
        || capability_count != 1
        || memory_model_count != 1
        || entry_points.len() != 1
        || execution_modes.len() != 1
        || void_types.len() != 1
        || uint_types.len() != 1
        || bool_types.len() != 1
        || vector_types.len() != 1
        || runtime_arrays.len() != 1
        || structs.len() != 1
        || pointer_types.len() != 3
        || function_type_declarations.len() != 1
        || function_declarations.len() != 1
        || variables.len() != 5
        || constants.len() != 2
    {
        return Err(invalid());
    }
    let uint_type = uint_types[0];
    let bool_type = bool_types[0];
    let (vector_type, vector_element, vector_lanes) = vector_types[0];
    if vector_element != uint_type
        || vector_lanes != 3
        || runtime_arrays[0].1 != uint_type
        || structs[0].1 != runtime_arrays[0].0
        || function_type_declarations[0].1 != void_types[0]
        || function_declarations[0][0] != void_types[0]
        || function_declarations[0][2] != 0
        || function_declarations[0][3] != function_type_declarations[0].0
        || entry_points[0].0 != EXECUTION_MODEL_GL_COMPUTE
        || entry_points[0].1 != function_declarations[0][1]
        || execution_modes[0][0] != function_declarations[0][1]
        || execution_modes[0][1] != EXECUTION_MODE_LOCAL_SIZE
        || execution_modes[0][2..] != artifact.workgroup_size
    {
        return Err(invalid());
    }
    let storage_struct_pointer = pointer_types
        .iter()
        .find(|(_, storage, pointee)| *storage == STORAGE_BUFFER && *pointee == structs[0].0)
        .map(|(id, _, _)| *id);
    let element_pointer = pointer_types
        .iter()
        .find(|(_, storage, pointee)| *storage == STORAGE_BUFFER && *pointee == uint_type)
        .map(|(id, _, _)| *id);
    let input_pointer = pointer_types
        .iter()
        .find(|(_, storage, pointee)| *storage == INPUT && *pointee == vector_type)
        .map(|(id, _, _)| *id);
    let (Some(storage_struct_pointer), Some(element_pointer), Some(input_pointer)) =
        (storage_struct_pointer, element_pointer, input_pointer)
    else {
        return Err(invalid());
    };
    let resource_variables: Vec<u32> = variables
        .iter()
        .filter(|operands| operands[0] == storage_struct_pointer && operands[2] == STORAGE_BUFFER)
        .map(|operands| operands[1])
        .collect();
    if resource_variables.len() != 4 {
        return Err(invalid());
    }
    let Some(global_variable) = variables.iter().find_map(|operands| {
        (operands[0] == input_pointer && operands[2] == INPUT).then_some(operands[1])
    }) else {
        return Err(invalid());
    };
    let mut bound_resources = [0_u32; 4];
    for (binding, slot) in bound_resources.iter_mut().enumerate() {
        let binding = binding as u32;
        let Some(variable) = bindings
            .iter()
            .find_map(|(target, values)| (values.as_slice() == [binding]).then_some(*target))
        else {
            return Err(invalid());
        };
        if !resource_variables.contains(&variable)
            || !descriptor_sets
                .iter()
                .any(|(target, values)| *target == variable && values.as_slice() == [0])
        {
            return Err(invalid());
        }
        *slot = variable;
    }
    if !builtin_variables.iter().any(|(target, values)| {
        *target == global_variable && values.as_slice() == [BUILT_IN_GLOBAL_INVOCATION_ID]
    }) || constants.iter().any(|(ty, _, _)| *ty != uint_type)
    {
        return Err(invalid());
    }
    let expected_body = [
        OP_LABEL,
        OP_LOAD,
        OP_COMPOSITE_EXTRACT,
        OP_COMPOSITE_EXTRACT,
        OP_ACCESS_CHAIN,
        OP_LOAD,
        OP_ACCESS_CHAIN,
        OP_LOAD,
        OP_ACCESS_CHAIN,
        OP_LOAD,
        OP_ULT,
        OP_ULT,
        OP_LOGICAL_AND,
        OP_SELECTION_MERGE,
        OP_BRANCH_CONDITIONAL,
        OP_LABEL,
        OP_IMUL,
        OP_IADD,
        OP_ULT,
        OP_SELECTION_MERGE,
        OP_BRANCH_CONDITIONAL,
        OP_LABEL,
        OP_ACCESS_CHAIN,
        OP_STORE,
        OP_BRANCH,
        OP_LABEL,
        OP_BRANCH,
        OP_LABEL,
        OP_RETURN,
        OP_FUNCTION_END,
    ];
    if function_body.len() != expected_body.len()
        || function_body
            .iter()
            .map(|(opcode, _)| *opcode)
            .collect::<Vec<_>>()
            .as_slice()
            != expected_body
    {
        return Err(invalid());
    }
    let labels = [
        function_body[0].1[0],
        function_body[15].1[0],
        function_body[21].1[0],
        function_body[25].1[0],
        function_body[27].1[0],
    ];
    if labels
        .iter()
        .enumerate()
        .any(|(index, label)| labels[..index].contains(label))
    {
        return Err(invalid());
    }
    let global_load = &function_body[1].1;
    let x_extract = &function_body[2].1;
    let y_extract = &function_body[3].1;
    if global_load.as_slice() != [vector_type, global_load[1], global_variable]
        || x_extract.len() != 4
        || x_extract[0] != uint_type
        || x_extract[2] != global_load[1]
        || x_extract[3] != 0
        || y_extract.len() != 4
        || y_extract[0] != uint_type
        || y_extract[2] != global_load[1]
        || y_extract[3] != 1
    {
        return Err(invalid());
    }
    let x_index = x_extract[1];
    let y_index = y_extract[1];
    let zero_id = function_body[4].1[3];
    let width_address = function_body[4].1[1];
    let width_value = function_body[5].1[1];
    let height_address = function_body[6].1[1];
    let height_value = function_body[7].1[1];
    let capacity_address = function_body[8].1[1];
    let capacity_value = function_body[9].1[1];
    let row_index = function_body[16].1[1];
    let logical_index = function_body[17].1[1];
    if function_body[4].1.as_slice()
        != [
            element_pointer,
            width_address,
            bound_resources[1],
            zero_id,
            zero_id,
        ]
        || function_body[5].1.as_slice() != [uint_type, width_value, width_address]
        || function_body[6].1.as_slice()
            != [
                element_pointer,
                height_address,
                bound_resources[2],
                zero_id,
                zero_id,
            ]
        || function_body[7].1.as_slice() != [uint_type, height_value, height_address]
        || function_body[8].1.as_slice()
            != [
                element_pointer,
                capacity_address,
                bound_resources[3],
                zero_id,
                zero_id,
            ]
        || function_body[9].1.as_slice() != [uint_type, capacity_value, capacity_address]
        || function_body[10].1.len() != 4
        || function_body[10].1[0] != bool_type
        || function_body[10].1[2] != x_index
        || function_body[10].1[3] != width_value
        || function_body[11].1.len() != 4
        || function_body[11].1[0] != bool_type
        || function_body[11].1[2] != y_index
        || function_body[11].1[3] != height_value
        || function_body[12].1.as_slice()
            != [
                bool_type,
                function_body[12].1[1],
                function_body[10].1[1],
                function_body[11].1[1],
            ]
        || function_body[13].1.as_slice() != [labels[4], 0]
        || function_body[14].1.as_slice() != [function_body[12].1[1], labels[1], labels[4]]
        || function_body[16].1.as_slice() != [uint_type, row_index, y_index, width_value]
        || function_body[17].1.as_slice() != [uint_type, logical_index, row_index, x_index]
        || function_body[18].1.len() != 4
        || function_body[18].1[0] != bool_type
        || function_body[18].1[2] != logical_index
        || function_body[18].1[3] != capacity_value
        || function_body[19].1.as_slice() != [labels[3], 0]
        || function_body[20].1.as_slice() != [function_body[18].1[1], labels[2], labels[3]]
        || function_body[22].1.as_slice()
            != [
                element_pointer,
                function_body[22].1[1],
                bound_resources[0],
                zero_id,
                logical_index,
            ]
        || function_body[23].1.len() != 2
        || function_body[23].1[0] != function_body[22].1[1]
        || function_body[24].1.as_slice() != [labels[3]]
        || function_body[26].1.as_slice() != [labels[4]]
        || !function_body[28].1.is_empty()
        || !function_body[29].1.is_empty()
    {
        return Err(invalid());
    }
    let value_id = function_body[23].1[1];
    let find_constant = |id: u32| {
        constants
            .iter()
            .find(|(_, constant_id, _)| *constant_id == id)
    };
    let Some((_, _, zero)) = find_constant(zero_id) else {
        return Err(invalid());
    };
    let Some((_, _, encoded_value)) = find_constant(value_id) else {
        return Err(invalid());
    };
    if value_id == zero_id || *zero != 0 || *encoded_value != value {
        return Err(MslError::UnsupportedJirShape(
            "2D global-write artifact value differs from execution request",
        ));
    }
    Ok(())
}

/// Lowers a validated row-major `global_2d_write_u32` artifact to MSL.
pub fn emit_storage_global_2d_write_from_spirv_artifact(
    artifact: &SpirvArtifact,
    options: MslOptions,
    value: u32,
) -> Result<String, MslError> {
    validate_storage_global_2d_write_artifact(artifact, value)?;
    if artifact.workgroup_size != options.workgroup_size {
        return Err(MslError::UnsupportedJirShape(
            "MSL options do not match SPIR-V artifact workgroup",
        ));
    }
    emit_storage_global_2d_write(&artifact.entry_name, options, value)
}

/// Validates the five-resource row-major `global_3d_write_u32` artifact
/// accepted by the MSL source bridge. Coordinate guards must dominate the
/// row-major flattening, and the flattened index must pass a capacity guard
/// before the indexed store.
pub fn validate_storage_global_3d_write_artifact(
    artifact: &SpirvArtifact,
    value: u32,
) -> Result<(), MslError> {
    if !valid_identifier(&artifact.entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    artifact
        .validate()
        .map_err(|_| MslError::UnsupportedJirShape("invalid SPIR-V artifact words"))?;
    if artifact.resources.len() != 5
        || artifact
            .resources
            .iter()
            .enumerate()
            .any(|(index, resource)| {
                resource.binding != index as u32
                    || resource.address_space != AddressSpace::Storage
                    || resource.element_stride != Some(4)
            })
    {
        return Err(MslError::UnsupportedJirShape(
            "3D global-write artifact requires five ordered storage stride-4 resources",
        ));
    }

    const OP_CAPABILITY: u16 = 17;
    const OP_MEMORY_MODEL: u16 = 14;
    const OP_ENTRY_POINT: u16 = 15;
    const OP_EXECUTION_MODE: u16 = 16;
    const OP_TYPE_VOID: u16 = 19;
    const OP_TYPE_BOOL: u16 = 20;
    const OP_TYPE_INT: u16 = 21;
    const OP_TYPE_VECTOR: u16 = 23;
    const OP_TYPE_RUNTIME_ARRAY: u16 = 29;
    const OP_TYPE_STRUCT: u16 = 30;
    const OP_TYPE_POINTER: u16 = 32;
    const OP_TYPE_FUNCTION: u16 = 33;
    const OP_CONSTANT: u16 = 43;
    const OP_LOAD: u16 = 61;
    const OP_ACCESS_CHAIN: u16 = 65;
    const OP_IADD: u16 = 128;
    const OP_IMUL: u16 = 132;
    const OP_LOGICAL_AND: u16 = 167;
    const OP_COMPOSITE_EXTRACT: u16 = 81;
    const OP_STORE: u16 = 62;
    const OP_ULT: u16 = 176;
    const OP_FUNCTION: u16 = 54;
    const OP_LABEL: u16 = 248;
    const OP_SELECTION_MERGE: u16 = 247;
    const OP_BRANCH: u16 = 249;
    const OP_BRANCH_CONDITIONAL: u16 = 250;
    const OP_RETURN: u16 = 253;
    const OP_FUNCTION_END: u16 = 56;
    const OP_VARIABLE: u16 = 59;
    const OP_DECORATE: u16 = 71;
    const STORAGE_BUFFER: u32 = 12;
    const INPUT: u32 = 1;
    const DECORATION_BINDING: u32 = 33;
    const DECORATION_DESCRIPTOR_SET: u32 = 34;
    const DECORATION_BUILT_IN: u32 = 11;
    const BUILT_IN_GLOBAL_INVOCATION_ID: u32 = 28;
    const EXECUTION_MODEL_GL_COMPUTE: u32 = 5;
    const EXECUTION_MODE_LOCAL_SIZE: u32 = 17;

    let invalid =
        || MslError::UnsupportedJirShape("3D global-write artifact has an unsupported shape");
    let mut uint_types = Vec::new();
    let mut bool_types = Vec::new();
    let mut vector_types = Vec::new();
    let mut runtime_arrays = Vec::new();
    let mut structs = Vec::new();
    let mut pointer_types = Vec::new();
    let mut constants = Vec::new();
    let mut variables = Vec::new();
    let mut descriptor_sets = Vec::new();
    let mut bindings = Vec::new();
    let mut builtin_variables = Vec::new();
    let mut entry_points = Vec::new();
    let mut execution_modes = Vec::new();
    let mut function_declarations = Vec::new();
    let mut function_type_declarations = Vec::new();
    let mut void_types = Vec::new();
    let mut capability_count = 0_u32;
    let mut memory_model_count = 0_u32;
    let mut function_body = Vec::new();
    let mut in_function = false;
    let mut cursor = 5_usize;
    while cursor < artifact.words.len() {
        let instruction = artifact.words[cursor];
        let word_count = (instruction >> 16) as usize;
        let opcode = (instruction & 0xFFFF) as u16;
        if word_count == 0 || cursor + word_count > artifact.words.len() {
            return Err(invalid());
        }
        let operands = &artifact.words[cursor + 1..cursor + word_count];
        if in_function {
            if opcode == OP_FUNCTION {
                return Err(invalid());
            }
            function_body.push((opcode, operands.to_vec()));
            if opcode == OP_FUNCTION_END {
                in_function = false;
            }
            cursor += word_count;
            continue;
        }
        match opcode {
            OP_CAPABILITY if operands == [1] => capability_count += 1,
            OP_MEMORY_MODEL if operands == [0, 1] => memory_model_count += 1,
            OP_ENTRY_POINT if operands.len() >= 2 => {
                entry_points.push((operands[0], operands[1]));
            }
            OP_EXECUTION_MODE if operands.len() == 5 => execution_modes.push(operands.to_vec()),
            OP_TYPE_VOID if operands.len() == 1 => void_types.push(operands[0]),
            OP_TYPE_BOOL if operands.len() == 1 => bool_types.push(operands[0]),
            OP_TYPE_INT if operands.len() == 3 => {
                if operands[1] != 32 || operands[2] != 0 {
                    return Err(invalid());
                }
                uint_types.push(operands[0]);
            }
            OP_TYPE_VECTOR if operands.len() == 3 => {
                vector_types.push((operands[0], operands[1], operands[2]))
            }
            OP_TYPE_RUNTIME_ARRAY if operands.len() == 2 => {
                runtime_arrays.push((operands[0], operands[1]));
            }
            OP_TYPE_STRUCT if operands.len() == 2 => {
                structs.push((operands[0], operands[1]));
            }
            OP_TYPE_POINTER if operands.len() == 3 => {
                pointer_types.push((operands[0], operands[1], operands[2]));
            }
            OP_TYPE_FUNCTION if operands.len() == 2 => {
                function_type_declarations.push((operands[0], operands[1]));
            }
            OP_CONSTANT if operands.len() == 3 => {
                constants.push((operands[0], operands[1], operands[2]));
            }
            OP_FUNCTION if operands.len() == 4 => {
                function_declarations.push(operands.to_vec());
                in_function = true;
            }
            OP_VARIABLE if operands.len() == 3 => variables.push(operands.to_vec()),
            OP_DECORATE if operands.len() >= 2 => match operands[1] {
                DECORATION_DESCRIPTOR_SET => {
                    descriptor_sets.push((operands[0], operands[2..].to_vec()))
                }
                DECORATION_BINDING => bindings.push((operands[0], operands[2..].to_vec())),
                DECORATION_BUILT_IN => {
                    builtin_variables.push((operands[0], operands[2..].to_vec()))
                }
                _ => {}
            },
            OP_ACCESS_CHAIN
            | OP_LOAD
            | OP_COMPOSITE_EXTRACT
            | OP_IADD
            | OP_IMUL
            | OP_LOGICAL_AND
            | OP_STORE
            | OP_ULT
            | OP_LABEL
            | OP_SELECTION_MERGE
            | OP_BRANCH
            | OP_BRANCH_CONDITIONAL
            | OP_RETURN
            | OP_FUNCTION_END => return Err(invalid()),
            _ => {}
        }
        cursor += word_count;
    }
    if in_function
        || capability_count != 1
        || memory_model_count != 1
        || entry_points.len() != 1
        || execution_modes.len() != 1
        || void_types.len() != 1
        || uint_types.len() != 1
        || bool_types.len() != 1
        || vector_types.len() != 1
        || runtime_arrays.len() != 1
        || structs.len() != 1
        || pointer_types.len() != 3
        || function_type_declarations.len() != 1
        || function_declarations.len() != 1
        || variables.len() != 6
        || constants.len() != 2
    {
        return Err(invalid());
    }
    let uint_type = uint_types[0];
    let bool_type = bool_types[0];
    let (vector_type, vector_element, vector_lanes) = vector_types[0];
    if vector_element != uint_type
        || vector_lanes != 3
        || runtime_arrays[0].1 != uint_type
        || structs[0].1 != runtime_arrays[0].0
        || function_type_declarations[0].1 != void_types[0]
        || function_declarations[0][0] != void_types[0]
        || function_declarations[0][2] != 0
        || function_declarations[0][3] != function_type_declarations[0].0
        || entry_points[0].0 != EXECUTION_MODEL_GL_COMPUTE
        || entry_points[0].1 != function_declarations[0][1]
        || execution_modes[0][0] != function_declarations[0][1]
        || execution_modes[0][1] != EXECUTION_MODE_LOCAL_SIZE
        || execution_modes[0][2..] != artifact.workgroup_size
    {
        return Err(invalid());
    }
    let storage_struct_pointer = pointer_types
        .iter()
        .find(|(_, storage, pointee)| *storage == STORAGE_BUFFER && *pointee == structs[0].0)
        .map(|(id, _, _)| *id);
    let element_pointer = pointer_types
        .iter()
        .find(|(_, storage, pointee)| *storage == STORAGE_BUFFER && *pointee == uint_type)
        .map(|(id, _, _)| *id);
    let input_pointer = pointer_types
        .iter()
        .find(|(_, storage, pointee)| *storage == INPUT && *pointee == vector_type)
        .map(|(id, _, _)| *id);
    let (Some(storage_struct_pointer), Some(element_pointer), Some(input_pointer)) =
        (storage_struct_pointer, element_pointer, input_pointer)
    else {
        return Err(invalid());
    };
    let resource_variables: Vec<u32> = variables
        .iter()
        .filter(|operands| operands[0] == storage_struct_pointer && operands[2] == STORAGE_BUFFER)
        .map(|operands| operands[1])
        .collect();
    if resource_variables.len() != 5 {
        return Err(invalid());
    }
    let Some(global_variable) = variables.iter().find_map(|operands| {
        (operands[0] == input_pointer && operands[2] == INPUT).then_some(operands[1])
    }) else {
        return Err(invalid());
    };
    let mut bound_resources = [0_u32; 5];
    for (binding, slot) in bound_resources.iter_mut().enumerate() {
        let binding = binding as u32;
        let Some(variable) = bindings
            .iter()
            .find_map(|(target, values)| (values.as_slice() == [binding]).then_some(*target))
        else {
            return Err(invalid());
        };
        if !resource_variables.contains(&variable)
            || !descriptor_sets
                .iter()
                .any(|(target, values)| *target == variable && values.as_slice() == [0])
        {
            return Err(invalid());
        }
        *slot = variable;
    }
    if !builtin_variables.iter().any(|(target, values)| {
        *target == global_variable && values.as_slice() == [BUILT_IN_GLOBAL_INVOCATION_ID]
    }) || constants.iter().any(|(ty, _, _)| *ty != uint_type)
    {
        return Err(invalid());
    }
    let expected_body = [
        OP_LABEL,
        OP_LOAD,
        OP_COMPOSITE_EXTRACT,
        OP_COMPOSITE_EXTRACT,
        OP_COMPOSITE_EXTRACT,
        OP_ACCESS_CHAIN,
        OP_LOAD,
        OP_ACCESS_CHAIN,
        OP_LOAD,
        OP_ACCESS_CHAIN,
        OP_LOAD,
        OP_ACCESS_CHAIN,
        OP_LOAD,
        OP_ULT,
        OP_ULT,
        OP_ULT,
        OP_LOGICAL_AND,
        OP_LOGICAL_AND,
        OP_SELECTION_MERGE,
        OP_BRANCH_CONDITIONAL,
        OP_LABEL,
        OP_IMUL,
        OP_IADD,
        OP_IMUL,
        OP_IADD,
        OP_ULT,
        OP_SELECTION_MERGE,
        OP_BRANCH_CONDITIONAL,
        OP_LABEL,
        OP_ACCESS_CHAIN,
        OP_STORE,
        OP_BRANCH,
        OP_LABEL,
        OP_BRANCH,
        OP_LABEL,
        OP_RETURN,
        OP_FUNCTION_END,
    ];
    if function_body.len() != expected_body.len()
        || function_body
            .iter()
            .map(|(opcode, _)| *opcode)
            .collect::<Vec<_>>()
            .as_slice()
            != expected_body
    {
        return Err(invalid());
    }
    let labels = [
        function_body[0].1[0],
        function_body[20].1[0],
        function_body[28].1[0],
        function_body[32].1[0],
        function_body[34].1[0],
    ];
    if labels
        .iter()
        .enumerate()
        .any(|(index, label)| labels[..index].contains(label))
    {
        return Err(invalid());
    }
    let global_load = &function_body[1].1;
    let x_extract = &function_body[2].1;
    let y_extract = &function_body[3].1;
    let z_extract = &function_body[4].1;
    if global_load.as_slice() != [vector_type, global_load[1], global_variable]
        || x_extract.len() != 4
        || x_extract[0] != uint_type
        || x_extract[2] != global_load[1]
        || x_extract[3] != 0
        || y_extract.len() != 4
        || y_extract[0] != uint_type
        || y_extract[2] != global_load[1]
        || y_extract[3] != 1
        || z_extract.len() != 4
        || z_extract[0] != uint_type
        || z_extract[2] != global_load[1]
        || z_extract[3] != 2
    {
        return Err(invalid());
    }
    let x_index = x_extract[1];
    let y_index = y_extract[1];
    let z_index = z_extract[1];
    let zero_id = function_body[5].1[3];
    let metadata = [
        (function_body[5].1[1], function_body[6].1[1]),
        (function_body[7].1[1], function_body[8].1[1]),
        (function_body[9].1[1], function_body[10].1[1]),
        (function_body[11].1[1], function_body[12].1[1]),
    ];
    for (index, (address, value_id)) in metadata.iter().enumerate() {
        if function_body[5 + index * 2].1.as_slice()
            != [
                element_pointer,
                *address,
                bound_resources[index + 1],
                zero_id,
                zero_id,
            ]
            || function_body[6 + index * 2].1.as_slice() != [uint_type, *value_id, *address]
        {
            return Err(invalid());
        }
    }
    let width_value = metadata[0].1;
    let height_value = metadata[1].1;
    let depth_value = metadata[2].1;
    let capacity_value = metadata[3].1;
    let x_bound = &function_body[13].1;
    let y_bound = &function_body[14].1;
    let z_bound = &function_body[15].1;
    let xy_bound = &function_body[16].1;
    let logical_bound = &function_body[17].1;
    if x_bound.len() != 4
        || x_bound[0] != bool_type
        || x_bound[2] != x_index
        || x_bound[3] != width_value
        || y_bound.len() != 4
        || y_bound[0] != bool_type
        || y_bound[2] != y_index
        || y_bound[3] != height_value
        || z_bound.len() != 4
        || z_bound[0] != bool_type
        || z_bound[2] != z_index
        || z_bound[3] != depth_value
        || xy_bound.as_slice() != [bool_type, xy_bound[1], x_bound[1], y_bound[1]]
        || logical_bound.as_slice() != [bool_type, logical_bound[1], xy_bound[1], z_bound[1]]
        || function_body[18].1.as_slice() != [labels[4], 0]
        || function_body[19].1.as_slice() != [logical_bound[1], labels[1], labels[4]]
        || function_body[21].1.as_slice()
            != [uint_type, function_body[21].1[1], z_index, height_value]
        || function_body[22].1.as_slice()
            != [
                uint_type,
                function_body[22].1[1],
                function_body[21].1[1],
                y_index,
            ]
        || function_body[23].1.as_slice()
            != [
                uint_type,
                function_body[23].1[1],
                function_body[22].1[1],
                width_value,
            ]
        || function_body[24].1.as_slice()
            != [
                uint_type,
                function_body[24].1[1],
                function_body[23].1[1],
                x_index,
            ]
        || function_body[25].1.len() != 4
        || function_body[25].1[0] != bool_type
        || function_body[25].1[2] != function_body[24].1[1]
        || function_body[25].1[3] != capacity_value
        || function_body[26].1.as_slice() != [labels[3], 0]
        || function_body[27].1.as_slice() != [function_body[25].1[1], labels[2], labels[3]]
        || function_body[29].1.as_slice()
            != [
                element_pointer,
                function_body[29].1[1],
                bound_resources[0],
                zero_id,
                function_body[24].1[1],
            ]
        || function_body[30].1.len() != 2
        || function_body[30].1[0] != function_body[29].1[1]
        || function_body[31].1.as_slice() != [labels[3]]
        || function_body[33].1.as_slice() != [labels[4]]
        || !function_body[35].1.is_empty()
        || !function_body[36].1.is_empty()
    {
        return Err(invalid());
    }
    let zero = constants
        .iter()
        .find(|(_, constant_id, _)| *constant_id == zero_id)
        .map(|(_, _, value)| *value);
    let value_id = function_body[30].1[1];
    let encoded_value = constants
        .iter()
        .find(|(_, constant_id, _)| *constant_id == value_id)
        .map(|(_, _, value)| *value);
    if value_id == zero_id || zero != Some(0) || encoded_value != Some(value) {
        return Err(MslError::UnsupportedJirShape(
            "3D global-write artifact constants differ from execution request",
        ));
    }
    Ok(())
}

/// Lowers a validated row-major `global_3d_write_u32` artifact to MSL.
pub fn emit_storage_global_3d_write_from_spirv_artifact(
    artifact: &SpirvArtifact,
    options: MslOptions,
    value: u32,
) -> Result<String, MslError> {
    validate_storage_global_3d_write_artifact(artifact, value)?;
    if artifact.workgroup_size != options.workgroup_size {
        return Err(MslError::UnsupportedJirShape(
            "MSL options do not match SPIR-V artifact workgroup",
        ));
    }
    emit_storage_global_3d_write(&artifact.entry_name, options, value)
}

/// Validates the eight-resource affine-stride `global_3d_strided_write_u32`
/// artifact accepted by the MSL source bridge. Coordinate guards dominate the
/// three stride multiplies and the final physical-capacity guard dominates the
/// indexed store.
pub fn validate_storage_global_3d_strided_write_artifact(
    artifact: &SpirvArtifact,
    value: u32,
) -> Result<(), MslError> {
    if !valid_identifier(&artifact.entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    artifact
        .validate()
        .map_err(|_| MslError::UnsupportedJirShape("invalid SPIR-V artifact words"))?;
    if artifact.resources.len() != 8
        || artifact
            .resources
            .iter()
            .enumerate()
            .any(|(index, resource)| {
                resource.binding != index as u32
                    || resource.address_space != AddressSpace::Storage
                    || resource.element_stride != Some(4)
            })
    {
        return Err(MslError::UnsupportedJirShape(
            "3D strided artifact requires eight ordered storage stride-4 resources",
        ));
    }

    const OP_CAPABILITY: u16 = 17;
    const OP_MEMORY_MODEL: u16 = 14;
    const OP_ENTRY_POINT: u16 = 15;
    const OP_EXECUTION_MODE: u16 = 16;
    const OP_TYPE_VOID: u16 = 19;
    const OP_TYPE_BOOL: u16 = 20;
    const OP_TYPE_INT: u16 = 21;
    const OP_TYPE_VECTOR: u16 = 23;
    const OP_TYPE_RUNTIME_ARRAY: u16 = 29;
    const OP_TYPE_STRUCT: u16 = 30;
    const OP_TYPE_POINTER: u16 = 32;
    const OP_TYPE_FUNCTION: u16 = 33;
    const OP_CONSTANT: u16 = 43;
    const OP_LOAD: u16 = 61;
    const OP_ACCESS_CHAIN: u16 = 65;
    const OP_IADD: u16 = 128;
    const OP_IMUL: u16 = 132;
    const OP_LOGICAL_AND: u16 = 167;
    const OP_COMPOSITE_EXTRACT: u16 = 81;
    const OP_STORE: u16 = 62;
    const OP_ULT: u16 = 176;
    const OP_FUNCTION: u16 = 54;
    const OP_LABEL: u16 = 248;
    const OP_SELECTION_MERGE: u16 = 247;
    const OP_BRANCH: u16 = 249;
    const OP_BRANCH_CONDITIONAL: u16 = 250;
    const OP_RETURN: u16 = 253;
    const OP_FUNCTION_END: u16 = 56;
    const OP_VARIABLE: u16 = 59;
    const OP_DECORATE: u16 = 71;
    const STORAGE_BUFFER: u32 = 12;
    const INPUT: u32 = 1;
    const DECORATION_BINDING: u32 = 33;
    const DECORATION_DESCRIPTOR_SET: u32 = 34;
    const DECORATION_BUILT_IN: u32 = 11;
    const BUILT_IN_GLOBAL_INVOCATION_ID: u32 = 28;
    const EXECUTION_MODEL_GL_COMPUTE: u32 = 5;
    const EXECUTION_MODE_LOCAL_SIZE: u32 = 17;

    let invalid = || MslError::UnsupportedJirShape("3D strided artifact has an unsupported shape");
    let mut uint_types = Vec::new();
    let mut bool_types = Vec::new();
    let mut vector_types = Vec::new();
    let mut runtime_arrays = Vec::new();
    let mut structs = Vec::new();
    let mut pointer_types = Vec::new();
    let mut constants = Vec::new();
    let mut variables = Vec::new();
    let mut descriptor_sets = Vec::new();
    let mut bindings = Vec::new();
    let mut builtin_variables = Vec::new();
    let mut entry_points = Vec::new();
    let mut execution_modes = Vec::new();
    let mut function_declarations = Vec::new();
    let mut function_type_declarations = Vec::new();
    let mut void_types = Vec::new();
    let mut capability_count = 0_u32;
    let mut memory_model_count = 0_u32;
    let mut function_body = Vec::new();
    let mut in_function = false;
    let mut cursor = 5_usize;
    while cursor < artifact.words.len() {
        let instruction = artifact.words[cursor];
        let word_count = (instruction >> 16) as usize;
        let opcode = (instruction & 0xFFFF) as u16;
        if word_count == 0 || cursor + word_count > artifact.words.len() {
            return Err(invalid());
        }
        let operands = &artifact.words[cursor + 1..cursor + word_count];
        if in_function {
            if opcode == OP_FUNCTION {
                return Err(invalid());
            }
            function_body.push((opcode, operands.to_vec()));
            if opcode == OP_FUNCTION_END {
                in_function = false;
            }
            cursor += word_count;
            continue;
        }
        match opcode {
            OP_CAPABILITY if operands == [1] => capability_count += 1,
            OP_MEMORY_MODEL if operands == [0, 1] => memory_model_count += 1,
            OP_ENTRY_POINT if operands.len() >= 2 => {
                entry_points.push((operands[0], operands[1]));
            }
            OP_EXECUTION_MODE if operands.len() == 5 => execution_modes.push(operands.to_vec()),
            OP_TYPE_VOID if operands.len() == 1 => void_types.push(operands[0]),
            OP_TYPE_BOOL if operands.len() == 1 => bool_types.push(operands[0]),
            OP_TYPE_INT if operands.len() == 3 => {
                if operands[1] != 32 || operands[2] != 0 {
                    return Err(invalid());
                }
                uint_types.push(operands[0]);
            }
            OP_TYPE_VECTOR if operands.len() == 3 => {
                vector_types.push((operands[0], operands[1], operands[2]))
            }
            OP_TYPE_RUNTIME_ARRAY if operands.len() == 2 => {
                runtime_arrays.push((operands[0], operands[1]));
            }
            OP_TYPE_STRUCT if operands.len() == 2 => {
                structs.push((operands[0], operands[1]));
            }
            OP_TYPE_POINTER if operands.len() == 3 => {
                pointer_types.push((operands[0], operands[1], operands[2]));
            }
            OP_TYPE_FUNCTION if operands.len() == 2 => {
                function_type_declarations.push((operands[0], operands[1]));
            }
            OP_CONSTANT if operands.len() == 3 => {
                constants.push((operands[0], operands[1], operands[2]));
            }
            OP_FUNCTION if operands.len() == 4 => {
                function_declarations.push(operands.to_vec());
                in_function = true;
            }
            OP_VARIABLE if operands.len() == 3 => variables.push(operands.to_vec()),
            OP_DECORATE if operands.len() >= 2 => match operands[1] {
                DECORATION_DESCRIPTOR_SET => {
                    descriptor_sets.push((operands[0], operands[2..].to_vec()))
                }
                DECORATION_BINDING => bindings.push((operands[0], operands[2..].to_vec())),
                DECORATION_BUILT_IN => {
                    builtin_variables.push((operands[0], operands[2..].to_vec()))
                }
                _ => {}
            },
            OP_ACCESS_CHAIN
            | OP_LOAD
            | OP_COMPOSITE_EXTRACT
            | OP_IADD
            | OP_IMUL
            | OP_LOGICAL_AND
            | OP_STORE
            | OP_ULT
            | OP_LABEL
            | OP_SELECTION_MERGE
            | OP_BRANCH
            | OP_BRANCH_CONDITIONAL
            | OP_RETURN
            | OP_FUNCTION_END => return Err(invalid()),
            _ => {}
        }
        cursor += word_count;
    }
    if in_function
        || capability_count != 1
        || memory_model_count != 1
        || entry_points.len() != 1
        || execution_modes.len() != 1
        || void_types.len() != 1
        || uint_types.len() != 1
        || bool_types.len() != 1
        || vector_types.len() != 1
        || runtime_arrays.len() != 1
        || structs.len() != 1
        || pointer_types.len() != 3
        || function_type_declarations.len() != 1
        || function_declarations.len() != 1
        || variables.len() != 9
        || constants.len() != 2
    {
        return Err(invalid());
    }
    let uint_type = uint_types[0];
    let bool_type = bool_types[0];
    let (vector_type, vector_element, vector_lanes) = vector_types[0];
    if vector_element != uint_type
        || vector_lanes != 3
        || runtime_arrays[0].1 != uint_type
        || structs[0].1 != runtime_arrays[0].0
        || function_type_declarations[0].1 != void_types[0]
        || function_declarations[0][0] != void_types[0]
        || function_declarations[0][2] != 0
        || function_declarations[0][3] != function_type_declarations[0].0
        || entry_points[0].0 != EXECUTION_MODEL_GL_COMPUTE
        || entry_points[0].1 != function_declarations[0][1]
        || execution_modes[0][0] != function_declarations[0][1]
        || execution_modes[0][1] != EXECUTION_MODE_LOCAL_SIZE
        || execution_modes[0][2..] != artifact.workgroup_size
    {
        return Err(invalid());
    }
    let storage_struct_pointer = pointer_types
        .iter()
        .find(|(_, storage, pointee)| *storage == STORAGE_BUFFER && *pointee == structs[0].0)
        .map(|(id, _, _)| *id);
    let element_pointer = pointer_types
        .iter()
        .find(|(_, storage, pointee)| *storage == STORAGE_BUFFER && *pointee == uint_type)
        .map(|(id, _, _)| *id);
    let input_pointer = pointer_types
        .iter()
        .find(|(_, storage, pointee)| *storage == INPUT && *pointee == vector_type)
        .map(|(id, _, _)| *id);
    let (Some(storage_struct_pointer), Some(element_pointer), Some(input_pointer)) =
        (storage_struct_pointer, element_pointer, input_pointer)
    else {
        return Err(invalid());
    };
    let resource_variables: Vec<u32> = variables
        .iter()
        .filter(|operands| operands[0] == storage_struct_pointer && operands[2] == STORAGE_BUFFER)
        .map(|operands| operands[1])
        .collect();
    if resource_variables.len() != 8 {
        return Err(invalid());
    }
    let Some(global_variable) = variables.iter().find_map(|operands| {
        (operands[0] == input_pointer && operands[2] == INPUT).then_some(operands[1])
    }) else {
        return Err(invalid());
    };
    let mut bound_resources = [0_u32; 8];
    for (binding, slot) in bound_resources.iter_mut().enumerate() {
        let binding = binding as u32;
        let Some(variable) = bindings
            .iter()
            .find_map(|(target, values)| (values.as_slice() == [binding]).then_some(*target))
        else {
            return Err(invalid());
        };
        if !resource_variables.contains(&variable)
            || !descriptor_sets
                .iter()
                .any(|(target, values)| *target == variable && values.as_slice() == [0])
        {
            return Err(invalid());
        }
        *slot = variable;
    }
    if !builtin_variables.iter().any(|(target, values)| {
        *target == global_variable && values.as_slice() == [BUILT_IN_GLOBAL_INVOCATION_ID]
    }) || constants.iter().any(|(ty, _, _)| *ty != uint_type)
    {
        return Err(invalid());
    }
    let expected_body = [
        OP_LABEL,
        OP_LOAD,
        OP_COMPOSITE_EXTRACT,
        OP_COMPOSITE_EXTRACT,
        OP_COMPOSITE_EXTRACT,
        OP_ACCESS_CHAIN,
        OP_LOAD,
        OP_ACCESS_CHAIN,
        OP_LOAD,
        OP_ACCESS_CHAIN,
        OP_LOAD,
        OP_ACCESS_CHAIN,
        OP_LOAD,
        OP_ACCESS_CHAIN,
        OP_LOAD,
        OP_ACCESS_CHAIN,
        OP_LOAD,
        OP_ACCESS_CHAIN,
        OP_LOAD,
        OP_ULT,
        OP_ULT,
        OP_ULT,
        OP_LOGICAL_AND,
        OP_LOGICAL_AND,
        OP_SELECTION_MERGE,
        OP_BRANCH_CONDITIONAL,
        OP_LABEL,
        OP_IMUL,
        OP_IMUL,
        OP_IADD,
        OP_IMUL,
        OP_IADD,
        OP_ULT,
        OP_SELECTION_MERGE,
        OP_BRANCH_CONDITIONAL,
        OP_LABEL,
        OP_ACCESS_CHAIN,
        OP_STORE,
        OP_BRANCH,
        OP_LABEL,
        OP_BRANCH,
        OP_LABEL,
        OP_RETURN,
        OP_FUNCTION_END,
    ];
    if function_body.len() != expected_body.len()
        || function_body
            .iter()
            .map(|(opcode, _)| *opcode)
            .collect::<Vec<_>>()
            .as_slice()
            != expected_body
    {
        return Err(invalid());
    }
    let labels = [
        function_body[0].1[0],
        function_body[26].1[0],
        function_body[35].1[0],
        function_body[39].1[0],
        function_body[41].1[0],
    ];
    if labels
        .iter()
        .enumerate()
        .any(|(index, label)| labels[..index].contains(label))
    {
        return Err(invalid());
    }
    let global_load = &function_body[1].1;
    let x_extract = &function_body[2].1;
    let y_extract = &function_body[3].1;
    let z_extract = &function_body[4].1;
    if global_load.as_slice() != [vector_type, global_load[1], global_variable]
        || x_extract.len() != 4
        || x_extract[0] != uint_type
        || x_extract[2] != global_load[1]
        || x_extract[3] != 0
        || y_extract.len() != 4
        || y_extract[0] != uint_type
        || y_extract[2] != global_load[1]
        || y_extract[3] != 1
        || z_extract.len() != 4
        || z_extract[0] != uint_type
        || z_extract[2] != global_load[1]
        || z_extract[3] != 2
    {
        return Err(invalid());
    }
    let x_index = x_extract[1];
    let y_index = y_extract[1];
    let z_index = z_extract[1];
    let zero_id = function_body[5].1[3];
    let metadata = [
        (function_body[5].1[1], function_body[6].1[1]),
        (function_body[7].1[1], function_body[8].1[1]),
        (function_body[9].1[1], function_body[10].1[1]),
        (function_body[11].1[1], function_body[12].1[1]),
        (function_body[13].1[1], function_body[14].1[1]),
        (function_body[15].1[1], function_body[16].1[1]),
        (function_body[17].1[1], function_body[18].1[1]),
    ];
    for (index, (address, value_id)) in metadata.iter().enumerate() {
        if function_body[5 + index * 2].1.as_slice()
            != [
                element_pointer,
                *address,
                bound_resources[index + 1],
                zero_id,
                zero_id,
            ]
            || function_body[6 + index * 2].1.as_slice() != [uint_type, *value_id, *address]
        {
            return Err(invalid());
        }
    }
    let width_value = metadata[0].1;
    let height_value = metadata[1].1;
    let depth_value = metadata[2].1;
    let stride_x_value = metadata[3].1;
    let stride_y_value = metadata[4].1;
    let stride_z_value = metadata[5].1;
    let capacity_value = metadata[6].1;
    let x_bound = &function_body[19].1;
    let y_bound = &function_body[20].1;
    let z_bound = &function_body[21].1;
    let xy_bound = &function_body[22].1;
    let logical_bound = &function_body[23].1;
    if x_bound.len() != 4
        || x_bound[0] != bool_type
        || x_bound[2] != x_index
        || x_bound[3] != width_value
        || y_bound.len() != 4
        || y_bound[0] != bool_type
        || y_bound[2] != y_index
        || y_bound[3] != height_value
        || z_bound.len() != 4
        || z_bound[0] != bool_type
        || z_bound[2] != z_index
        || z_bound[3] != depth_value
        || xy_bound.as_slice() != [bool_type, xy_bound[1], x_bound[1], y_bound[1]]
        || logical_bound.as_slice() != [bool_type, logical_bound[1], xy_bound[1], z_bound[1]]
        || function_body[24].1.as_slice() != [labels[4], 0]
        || function_body[25].1.as_slice() != [logical_bound[1], labels[1], labels[4]]
        || function_body[27].1.as_slice()
            != [uint_type, function_body[27].1[1], x_index, stride_x_value]
        || function_body[28].1.as_slice()
            != [uint_type, function_body[28].1[1], y_index, stride_y_value]
        || function_body[29].1.as_slice()
            != [
                uint_type,
                function_body[29].1[1],
                function_body[27].1[1],
                function_body[28].1[1],
            ]
        || function_body[30].1.as_slice()
            != [uint_type, function_body[30].1[1], z_index, stride_z_value]
        || function_body[31].1.as_slice()
            != [
                uint_type,
                function_body[31].1[1],
                function_body[29].1[1],
                function_body[30].1[1],
            ]
        || function_body[32].1.len() != 4
        || function_body[32].1[0] != bool_type
        || function_body[32].1[2] != function_body[31].1[1]
        || function_body[32].1[3] != capacity_value
        || function_body[33].1.as_slice() != [labels[3], 0]
        || function_body[34].1.as_slice() != [function_body[32].1[1], labels[2], labels[3]]
        || function_body[36].1.as_slice()
            != [
                element_pointer,
                function_body[36].1[1],
                bound_resources[0],
                zero_id,
                function_body[31].1[1],
            ]
        || function_body[37].1.len() != 2
        || function_body[37].1[0] != function_body[36].1[1]
        || function_body[38].1.as_slice() != [labels[3]]
        || function_body[40].1.as_slice() != [labels[4]]
        || !function_body[42].1.is_empty()
        || !function_body[43].1.is_empty()
    {
        return Err(invalid());
    }
    let zero = constants
        .iter()
        .find(|(_, constant_id, _)| *constant_id == zero_id)
        .map(|(_, _, encoded)| *encoded);
    let value_id = function_body[37].1[1];
    let encoded_value = constants
        .iter()
        .find(|(_, constant_id, _)| *constant_id == value_id)
        .map(|(_, _, encoded)| *encoded);
    if value_id == zero_id || zero != Some(0) || encoded_value != Some(value) {
        return Err(MslError::UnsupportedJirShape(
            "3D strided artifact constants differ from execution request",
        ));
    }
    Ok(())
}

/// Lowers a validated affine-stride `global_3d_strided_write_u32` artifact to
/// MSL while preserving the artifact workgroup metadata.
pub fn emit_storage_global_3d_strided_write_from_spirv_artifact(
    artifact: &SpirvArtifact,
    options: MslOptions,
    value: u32,
) -> Result<String, MslError> {
    validate_storage_global_3d_strided_write_artifact(artifact, value)?;
    if artifact.workgroup_size != options.workgroup_size {
        return Err(MslError::UnsupportedJirShape(
            "MSL options do not match SPIR-V artifact workgroup",
        ));
    }
    emit_storage_global_3d_strided_write(&artifact.entry_name, options, value)
}

/// Validates the six-resource affine-stride `global_2d_strided_write_u32`
/// artifact accepted by the MSL source bridge. The coordinate guards dominate
/// both stride multiplications and the final physical-capacity guard.
pub fn validate_storage_global_2d_strided_write_artifact(
    artifact: &SpirvArtifact,
    value: u32,
) -> Result<(), MslError> {
    if !valid_identifier(&artifact.entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    artifact
        .validate()
        .map_err(|_| MslError::UnsupportedJirShape("invalid SPIR-V artifact words"))?;
    if artifact.resources.len() != 6
        || artifact
            .resources
            .iter()
            .enumerate()
            .any(|(index, resource)| {
                resource.binding != index as u32
                    || resource.address_space != AddressSpace::Storage
                    || resource.element_stride != Some(4)
            })
    {
        return Err(MslError::UnsupportedJirShape(
            "2D strided artifact requires six ordered storage stride-4 resources",
        ));
    }

    const OP_CAPABILITY: u16 = 17;
    const OP_MEMORY_MODEL: u16 = 14;
    const OP_ENTRY_POINT: u16 = 15;
    const OP_EXECUTION_MODE: u16 = 16;
    const OP_TYPE_VOID: u16 = 19;
    const OP_TYPE_BOOL: u16 = 20;
    const OP_TYPE_INT: u16 = 21;
    const OP_TYPE_VECTOR: u16 = 23;
    const OP_TYPE_RUNTIME_ARRAY: u16 = 29;
    const OP_TYPE_STRUCT: u16 = 30;
    const OP_TYPE_POINTER: u16 = 32;
    const OP_TYPE_FUNCTION: u16 = 33;
    const OP_CONSTANT: u16 = 43;
    const OP_LOAD: u16 = 61;
    const OP_ACCESS_CHAIN: u16 = 65;
    const OP_IADD: u16 = 128;
    const OP_IMUL: u16 = 132;
    const OP_LOGICAL_AND: u16 = 167;
    const OP_COMPOSITE_EXTRACT: u16 = 81;
    const OP_STORE: u16 = 62;
    const OP_ULT: u16 = 176;
    const OP_FUNCTION: u16 = 54;
    const OP_LABEL: u16 = 248;
    const OP_SELECTION_MERGE: u16 = 247;
    const OP_BRANCH: u16 = 249;
    const OP_BRANCH_CONDITIONAL: u16 = 250;
    const OP_RETURN: u16 = 253;
    const OP_FUNCTION_END: u16 = 56;
    const OP_VARIABLE: u16 = 59;
    const OP_DECORATE: u16 = 71;
    const STORAGE_BUFFER: u32 = 12;
    const INPUT: u32 = 1;
    const DECORATION_BINDING: u32 = 33;
    const DECORATION_DESCRIPTOR_SET: u32 = 34;
    const DECORATION_BUILT_IN: u32 = 11;
    const BUILT_IN_GLOBAL_INVOCATION_ID: u32 = 28;
    const EXECUTION_MODEL_GL_COMPUTE: u32 = 5;
    const EXECUTION_MODE_LOCAL_SIZE: u32 = 17;

    let invalid = || MslError::UnsupportedJirShape("2D strided artifact has an unsupported shape");
    let mut uint_types = Vec::new();
    let mut bool_types = Vec::new();
    let mut vector_types = Vec::new();
    let mut runtime_arrays = Vec::new();
    let mut structs = Vec::new();
    let mut pointer_types = Vec::new();
    let mut constants = Vec::new();
    let mut variables = Vec::new();
    let mut descriptor_sets = Vec::new();
    let mut bindings = Vec::new();
    let mut builtin_variables = Vec::new();
    let mut entry_points = Vec::new();
    let mut execution_modes = Vec::new();
    let mut function_declarations = Vec::new();
    let mut function_type_declarations = Vec::new();
    let mut void_types = Vec::new();
    let mut capability_count = 0_u32;
    let mut memory_model_count = 0_u32;
    let mut function_body = Vec::new();
    let mut in_function = false;
    let mut cursor = 5_usize;
    while cursor < artifact.words.len() {
        let instruction = artifact.words[cursor];
        let word_count = (instruction >> 16) as usize;
        let opcode = (instruction & 0xFFFF) as u16;
        if word_count == 0 || cursor + word_count > artifact.words.len() {
            return Err(invalid());
        }
        let operands = &artifact.words[cursor + 1..cursor + word_count];
        if in_function {
            if opcode == OP_FUNCTION {
                return Err(invalid());
            }
            function_body.push((opcode, operands.to_vec()));
            if opcode == OP_FUNCTION_END {
                in_function = false;
            }
            cursor += word_count;
            continue;
        }
        match opcode {
            OP_CAPABILITY if operands == [1] => capability_count += 1,
            OP_MEMORY_MODEL if operands == [0, 1] => memory_model_count += 1,
            OP_ENTRY_POINT if operands.len() >= 2 => {
                entry_points.push((operands[0], operands[1]));
            }
            OP_EXECUTION_MODE if operands.len() == 5 => execution_modes.push(operands.to_vec()),
            OP_TYPE_VOID if operands.len() == 1 => void_types.push(operands[0]),
            OP_TYPE_BOOL if operands.len() == 1 => bool_types.push(operands[0]),
            OP_TYPE_INT if operands.len() == 3 => {
                if operands[1] != 32 || operands[2] != 0 {
                    return Err(invalid());
                }
                uint_types.push(operands[0]);
            }
            OP_TYPE_VECTOR if operands.len() == 3 => {
                vector_types.push((operands[0], operands[1], operands[2]))
            }
            OP_TYPE_RUNTIME_ARRAY if operands.len() == 2 => {
                runtime_arrays.push((operands[0], operands[1]));
            }
            OP_TYPE_STRUCT if operands.len() == 2 => {
                structs.push((operands[0], operands[1]));
            }
            OP_TYPE_POINTER if operands.len() == 3 => {
                pointer_types.push((operands[0], operands[1], operands[2]));
            }
            OP_TYPE_FUNCTION if operands.len() == 2 => {
                function_type_declarations.push((operands[0], operands[1]));
            }
            OP_CONSTANT if operands.len() == 3 => {
                constants.push((operands[0], operands[1], operands[2]));
            }
            OP_FUNCTION if operands.len() == 4 => {
                function_declarations.push(operands.to_vec());
                in_function = true;
            }
            OP_VARIABLE if operands.len() == 3 => variables.push(operands.to_vec()),
            OP_DECORATE if operands.len() >= 2 => match operands[1] {
                DECORATION_DESCRIPTOR_SET => {
                    descriptor_sets.push((operands[0], operands[2..].to_vec()))
                }
                DECORATION_BINDING => bindings.push((operands[0], operands[2..].to_vec())),
                DECORATION_BUILT_IN => {
                    builtin_variables.push((operands[0], operands[2..].to_vec()))
                }
                _ => {}
            },
            OP_ACCESS_CHAIN
            | OP_LOAD
            | OP_COMPOSITE_EXTRACT
            | OP_IADD
            | OP_IMUL
            | OP_LOGICAL_AND
            | OP_STORE
            | OP_ULT
            | OP_LABEL
            | OP_SELECTION_MERGE
            | OP_BRANCH
            | OP_BRANCH_CONDITIONAL
            | OP_RETURN
            | OP_FUNCTION_END => return Err(invalid()),
            _ => {}
        }
        cursor += word_count;
    }
    if in_function
        || capability_count != 1
        || memory_model_count != 1
        || entry_points.len() != 1
        || execution_modes.len() != 1
        || void_types.len() != 1
        || uint_types.len() != 1
        || bool_types.len() != 1
        || vector_types.len() != 1
        || runtime_arrays.len() != 1
        || structs.len() != 1
        || pointer_types.len() != 3
        || function_type_declarations.len() != 1
        || function_declarations.len() != 1
        || variables.len() != 7
        || constants.len() != 2
    {
        return Err(invalid());
    }
    let uint_type = uint_types[0];
    let bool_type = bool_types[0];
    let (vector_type, vector_element, vector_lanes) = vector_types[0];
    if vector_element != uint_type
        || vector_lanes != 3
        || runtime_arrays[0].1 != uint_type
        || structs[0].1 != runtime_arrays[0].0
        || function_type_declarations[0].1 != void_types[0]
        || function_declarations[0][0] != void_types[0]
        || function_declarations[0][2] != 0
        || function_declarations[0][3] != function_type_declarations[0].0
        || entry_points[0].0 != EXECUTION_MODEL_GL_COMPUTE
        || entry_points[0].1 != function_declarations[0][1]
        || execution_modes[0][0] != function_declarations[0][1]
        || execution_modes[0][1] != EXECUTION_MODE_LOCAL_SIZE
        || execution_modes[0][2..] != artifact.workgroup_size
    {
        return Err(invalid());
    }
    let storage_struct_pointer = pointer_types
        .iter()
        .find(|(_, storage, pointee)| *storage == STORAGE_BUFFER && *pointee == structs[0].0)
        .map(|(id, _, _)| *id);
    let element_pointer = pointer_types
        .iter()
        .find(|(_, storage, pointee)| *storage == STORAGE_BUFFER && *pointee == uint_type)
        .map(|(id, _, _)| *id);
    let input_pointer = pointer_types
        .iter()
        .find(|(_, storage, pointee)| *storage == INPUT && *pointee == vector_type)
        .map(|(id, _, _)| *id);
    let (Some(storage_struct_pointer), Some(element_pointer), Some(input_pointer)) =
        (storage_struct_pointer, element_pointer, input_pointer)
    else {
        return Err(invalid());
    };
    let resource_variables: Vec<u32> = variables
        .iter()
        .filter(|operands| operands[0] == storage_struct_pointer && operands[2] == STORAGE_BUFFER)
        .map(|operands| operands[1])
        .collect();
    if resource_variables.len() != 6 {
        return Err(invalid());
    }
    let Some(global_variable) = variables.iter().find_map(|operands| {
        (operands[0] == input_pointer && operands[2] == INPUT).then_some(operands[1])
    }) else {
        return Err(invalid());
    };
    let mut bound_resources = [0_u32; 6];
    for (binding, slot) in bound_resources.iter_mut().enumerate() {
        let binding = binding as u32;
        let Some(variable) = bindings
            .iter()
            .find_map(|(target, values)| (values.as_slice() == [binding]).then_some(*target))
        else {
            return Err(invalid());
        };
        if !resource_variables.contains(&variable)
            || !descriptor_sets
                .iter()
                .any(|(target, values)| *target == variable && values.as_slice() == [0])
        {
            return Err(invalid());
        }
        *slot = variable;
    }
    if !builtin_variables.iter().any(|(target, values)| {
        *target == global_variable && values.as_slice() == [BUILT_IN_GLOBAL_INVOCATION_ID]
    }) || constants.iter().any(|(ty, _, _)| *ty != uint_type)
    {
        return Err(invalid());
    }
    let expected_body = [
        OP_LABEL,
        OP_LOAD,
        OP_COMPOSITE_EXTRACT,
        OP_COMPOSITE_EXTRACT,
        OP_ACCESS_CHAIN,
        OP_LOAD,
        OP_ACCESS_CHAIN,
        OP_LOAD,
        OP_ACCESS_CHAIN,
        OP_LOAD,
        OP_ACCESS_CHAIN,
        OP_LOAD,
        OP_ACCESS_CHAIN,
        OP_LOAD,
        OP_ULT,
        OP_ULT,
        OP_LOGICAL_AND,
        OP_SELECTION_MERGE,
        OP_BRANCH_CONDITIONAL,
        OP_LABEL,
        OP_IMUL,
        OP_IMUL,
        OP_IADD,
        OP_ULT,
        OP_SELECTION_MERGE,
        OP_BRANCH_CONDITIONAL,
        OP_LABEL,
        OP_ACCESS_CHAIN,
        OP_STORE,
        OP_BRANCH,
        OP_LABEL,
        OP_BRANCH,
        OP_LABEL,
        OP_RETURN,
        OP_FUNCTION_END,
    ];
    if function_body.len() != expected_body.len()
        || function_body
            .iter()
            .map(|(opcode, _)| *opcode)
            .collect::<Vec<_>>()
            .as_slice()
            != expected_body
    {
        return Err(invalid());
    }
    let labels = [
        function_body[0].1[0],
        function_body[19].1[0],
        function_body[26].1[0],
        function_body[30].1[0],
        function_body[32].1[0],
    ];
    if labels
        .iter()
        .enumerate()
        .any(|(index, label)| labels[..index].contains(label))
    {
        return Err(invalid());
    }
    let global_load = &function_body[1].1;
    let x_extract = &function_body[2].1;
    let y_extract = &function_body[3].1;
    if global_load.as_slice() != [vector_type, global_load[1], global_variable]
        || x_extract.len() != 4
        || x_extract[0] != uint_type
        || x_extract[2] != global_load[1]
        || x_extract[3] != 0
        || y_extract.len() != 4
        || y_extract[0] != uint_type
        || y_extract[2] != global_load[1]
        || y_extract[3] != 1
    {
        return Err(invalid());
    }
    let x_index = x_extract[1];
    let y_index = y_extract[1];
    let zero_id = function_body[4].1[3];
    let width_address = function_body[4].1[1];
    let width_value = function_body[5].1[1];
    let height_address = function_body[6].1[1];
    let height_value = function_body[7].1[1];
    let stride_x_address = function_body[8].1[1];
    let stride_x_value = function_body[9].1[1];
    let stride_y_address = function_body[10].1[1];
    let stride_y_value = function_body[11].1[1];
    let capacity_address = function_body[12].1[1];
    let capacity_value = function_body[13].1[1];
    let x_offset = function_body[20].1[1];
    let y_offset = function_body[21].1[1];
    let physical_index = function_body[22].1[1];
    if function_body[4].1.as_slice()
        != [
            element_pointer,
            width_address,
            bound_resources[1],
            zero_id,
            zero_id,
        ]
        || function_body[5].1.as_slice() != [uint_type, width_value, width_address]
        || function_body[6].1.as_slice()
            != [
                element_pointer,
                height_address,
                bound_resources[2],
                zero_id,
                zero_id,
            ]
        || function_body[7].1.as_slice() != [uint_type, height_value, height_address]
        || function_body[8].1.as_slice()
            != [
                element_pointer,
                stride_x_address,
                bound_resources[3],
                zero_id,
                zero_id,
            ]
        || function_body[9].1.as_slice() != [uint_type, stride_x_value, stride_x_address]
        || function_body[10].1.as_slice()
            != [
                element_pointer,
                stride_y_address,
                bound_resources[4],
                zero_id,
                zero_id,
            ]
        || function_body[11].1.as_slice() != [uint_type, stride_y_value, stride_y_address]
        || function_body[12].1.as_slice()
            != [
                element_pointer,
                capacity_address,
                bound_resources[5],
                zero_id,
                zero_id,
            ]
        || function_body[13].1.as_slice() != [uint_type, capacity_value, capacity_address]
        || function_body[14].1.len() != 4
        || function_body[14].1[0] != bool_type
        || function_body[14].1[2] != x_index
        || function_body[14].1[3] != width_value
        || function_body[15].1.len() != 4
        || function_body[15].1[0] != bool_type
        || function_body[15].1[2] != y_index
        || function_body[15].1[3] != height_value
        || function_body[16].1.as_slice()
            != [
                bool_type,
                function_body[16].1[1],
                function_body[14].1[1],
                function_body[15].1[1],
            ]
        || function_body[17].1.as_slice() != [labels[4], 0]
        || function_body[18].1.as_slice() != [function_body[16].1[1], labels[1], labels[4]]
        || function_body[20].1.as_slice() != [uint_type, x_offset, x_index, stride_x_value]
        || function_body[21].1.as_slice() != [uint_type, y_offset, y_index, stride_y_value]
        || function_body[22].1.as_slice() != [uint_type, physical_index, x_offset, y_offset]
        || function_body[23].1.len() != 4
        || function_body[23].1[0] != bool_type
        || function_body[23].1[2] != physical_index
        || function_body[23].1[3] != capacity_value
        || function_body[24].1.as_slice() != [labels[3], 0]
        || function_body[25].1.as_slice() != [function_body[23].1[1], labels[2], labels[3]]
        || function_body[27].1.as_slice()
            != [
                element_pointer,
                function_body[27].1[1],
                bound_resources[0],
                zero_id,
                physical_index,
            ]
        || function_body[28].1.len() != 2
        || function_body[28].1[0] != function_body[27].1[1]
        || function_body[29].1.as_slice() != [labels[3]]
        || function_body[31].1.as_slice() != [labels[4]]
        || !function_body[33].1.is_empty()
        || !function_body[34].1.is_empty()
    {
        return Err(invalid());
    }
    let value_id = function_body[28].1[1];
    let find_constant = |id: u32| {
        constants
            .iter()
            .find(|(_, constant_id, _)| *constant_id == id)
    };
    let Some((_, _, zero)) = find_constant(zero_id) else {
        return Err(invalid());
    };
    let Some((_, _, encoded_value)) = find_constant(value_id) else {
        return Err(invalid());
    };
    if value_id == zero_id || *zero != 0 || *encoded_value != value {
        return Err(MslError::UnsupportedJirShape(
            "2D strided artifact value differs from execution request",
        ));
    }
    Ok(())
}

/// Lowers a validated affine-stride 2D artifact to MSL.
pub fn emit_storage_global_2d_strided_write_from_spirv_artifact(
    artifact: &SpirvArtifact,
    options: MslOptions,
    value: u32,
) -> Result<String, MslError> {
    validate_storage_global_2d_strided_write_artifact(artifact, value)?;
    if artifact.workgroup_size != options.workgroup_size {
        return Err(MslError::UnsupportedJirShape(
            "MSL options do not match SPIR-V artifact workgroup",
        ));
    }
    emit_storage_global_2d_strided_write(&artifact.entry_name, options, value)
}

/// Lowers a validated runtime-length `u32` BinaryOp artifact to MSL source.
pub fn emit_storage_binary_from_spirv_artifact(
    artifact: &SpirvArtifact,
    options: MslOptions,
    operation: BinaryOp,
    operand: u32,
) -> Result<String, MslError> {
    validate_storage_binary_artifact(artifact, operation, operand)?;
    if artifact.workgroup_size != options.workgroup_size {
        return Err(MslError::UnsupportedJirShape(
            "MSL options do not match SPIR-V artifact workgroup",
        ));
    }
    emit_storage_binary(&artifact.entry_name, options, operation, operand)
}

/// Lowers a validated scalar runtime-length `f32` artifact to MSL source.
pub fn emit_storage_f32_binary_from_spirv_artifact(
    artifact: &SpirvArtifact,
    options: MslOptions,
    operand_bits: u32,
    operation: F32ArithmeticOp,
) -> Result<String, MslError> {
    validate_storage_f32_binary_artifact(artifact, operand_bits, operation)?;
    if artifact.workgroup_size != options.workgroup_size {
        return Err(MslError::UnsupportedJirShape(
            "MSL options do not match SPIR-V artifact workgroup",
        ));
    }
    emit_storage_f32_binary(&artifact.entry_name, options, operand_bits, operation)
}

/// Validates the narrow runtime-length vector artifact subset accepted by the
/// MSL source bridge. This is deliberately not a general SPIR-V translator:
/// it accepts only the verified three-resource f32 vector binary shape with a
/// bounds predicate and one indexed store.
pub fn validate_storage_vector_f32_binary_lanes_artifact(
    artifact: &SpirvArtifact,
    operand_bits: u32,
    operation: F32ArithmeticOp,
    lanes: u16,
) -> Result<(), MslError> {
    if !(2..=4).contains(&lanes) {
        return Err(MslError::UnsupportedJirShape(
            "vector f32 artifact lanes must be in 2..=4",
        ));
    }
    if !valid_identifier(&artifact.entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    artifact
        .validate()
        .map_err(|_| MslError::UnsupportedJirShape("invalid SPIR-V artifact words"))?;
    let stride = u32::from(lanes) * 4;
    if artifact.resources.len() != 3
        || artifact
            .resources
            .iter()
            .enumerate()
            .any(|(index, resource)| {
                resource.binding != index as u32 || resource.address_space != AddressSpace::Storage
            })
        || artifact.resources[0].element_stride != Some(stride)
        || artifact.resources[1].element_stride != Some(stride)
        || artifact.resources[2].element_stride != Some(4)
    {
        return Err(MslError::UnsupportedJirShape(
            "vector f32 artifact requires ordered storage strides lane*4/lane*4/4",
        ));
    }

    const OP_TYPE_FLOAT: u16 = 22;
    const OP_TYPE_VECTOR: u16 = 23;
    const OP_CONSTANT: u16 = 43;
    const OP_CONSTANT_COMPOSITE: u16 = 44;
    const OP_FADD: u16 = 129;
    const OP_FSUB: u16 = 131;
    const OP_FMUL: u16 = 133;
    const OP_ULT: u16 = 176;
    const OP_STORE: u16 = 62;
    let expected_opcode = match operation {
        F32ArithmeticOp::Add => OP_FADD,
        F32ArithmeticOp::Subtract => OP_FSUB,
        F32ArithmeticOp::Multiply => OP_FMUL,
    };
    let mut float_types = Vec::new();
    let mut vector_types = Vec::new();
    let mut scalar_constants = Vec::new();
    let mut composite_count = 0_u32;
    let mut vector_binary_count = 0_u32;
    let mut bounds_count = 0_u32;
    let mut store_count = 0_u32;
    let mut cursor = 5_usize;
    while cursor < artifact.words.len() {
        let instruction = artifact.words[cursor];
        let word_count = (instruction >> 16) as usize;
        let opcode = (instruction & 0xFFFF) as u16;
        if word_count == 0 || cursor + word_count > artifact.words.len() {
            return Err(MslError::UnsupportedJirShape(
                "SPIR-V artifact instruction stream is truncated",
            ));
        }
        let operands = &artifact.words[cursor + 1..cursor + word_count];
        match opcode {
            OP_TYPE_FLOAT if operands.len() >= 2 && operands[1] == 32 => {
                float_types.push(operands[0]);
            }
            OP_TYPE_VECTOR if operands.len() >= 3 && operands[2] == u32::from(lanes) => {
                vector_types.push((operands[0], operands[1]));
            }
            OP_CONSTANT if operands.len() >= 3 => {
                scalar_constants.push((operands[1], operands[2]));
            }
            OP_CONSTANT_COMPOSITE => {
                if operands.len() != 2 + usize::from(lanes)
                    || !vector_types.iter().any(|(id, _)| *id == operands[0])
                    || operands[2..].iter().any(|lane| {
                        !scalar_constants
                            .iter()
                            .any(|(id, bits)| *id == *lane && *bits == operand_bits)
                    })
                {
                    return Err(MslError::UnsupportedJirShape(
                        "vector f32 artifact constant composite is not the requested splat",
                    ));
                }
                composite_count += 1;
            }
            OP_FADD | OP_FSUB | OP_FMUL
                if operands.len() >= 4 && vector_types.iter().any(|(id, _)| *id == operands[0]) =>
            {
                if opcode != expected_opcode {
                    return Err(MslError::UnsupportedJirShape(
                        "vector f32 artifact operation differs from request",
                    ));
                }
                vector_binary_count += 1;
            }
            OP_ULT => bounds_count += 1,
            OP_STORE => store_count += 1,
            _ => {}
        }
        cursor += word_count;
    }
    if !vector_types
        .iter()
        .any(|(_, element)| float_types.contains(element))
    {
        return Err(MslError::UnsupportedJirShape(
            "vector f32 artifact does not use a 32-bit float element",
        ));
    }
    if composite_count != 1 || vector_binary_count != 1 || bounds_count != 1 || store_count != 1 {
        return Err(MslError::UnsupportedJirShape(
            "vector f32 artifact requires one splat, binary operation, bounds predicate and store",
        ));
    }
    Ok(())
}

/// Lowers a validated runtime-length vector artifact to the corresponding MSL
/// source contract. The x4 function is a compatibility wrapper.
pub fn emit_storage_vector_f32_binary_from_spirv_artifact(
    artifact: &SpirvArtifact,
    options: MslOptions,
    operand_bits: u32,
    operation: F32ArithmeticOp,
) -> Result<String, MslError> {
    emit_storage_vector_f32_binary_lanes_from_spirv_artifact(
        artifact,
        options,
        operand_bits,
        operation,
        4,
    )
}

/// Lowers only the validated `f32x2`/`f32x3`/`f32x4` artifact subset to MSL.
pub fn emit_storage_vector_f32_binary_lanes_from_spirv_artifact(
    artifact: &SpirvArtifact,
    options: MslOptions,
    operand_bits: u32,
    operation: F32ArithmeticOp,
    lanes: u16,
) -> Result<String, MslError> {
    validate_storage_vector_f32_binary_lanes_artifact(artifact, operand_bits, operation, lanes)?;
    if artifact.workgroup_size != options.workgroup_size {
        return Err(MslError::UnsupportedJirShape(
            "MSL options do not match SPIR-V artifact workgroup",
        ));
    }
    emit_storage_vector_f32_binary_lanes(
        &artifact.entry_name,
        options,
        operand_bits,
        operation,
        lanes,
    )
}

/// Translates a validated artifact through the currently supported MSL
/// families. This is a shape-dispatch boundary, not a claim that arbitrary
/// SPIR-V is accepted: every candidate still runs its strict family validator
/// before source emission, and unknown opcode/resource/layout combinations are
/// rejected.
type MslGlobalArtifactEmitter = fn(&SpirvArtifact, MslOptions, u32) -> Result<String, MslError>;

pub fn translate_spirv_artifact_to_msl(
    artifact: &SpirvArtifact,
    options: MslOptions,
) -> Result<String, MslError> {
    MslOptions::new(artifact.workgroup_size)?;
    validate_spirv_artifact_contract(artifact)
        .map_err(|_| MslError::InvalidSpirvArtifact("shared artifact contract failed"))?;
    validate_msl_resource_address_spaces(artifact)?;
    validate_msl_descriptor_sets(artifact)?;
    if !valid_identifier(&artifact.entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    let options = MslOptions::new(options.workgroup_size)?;
    if artifact.workgroup_size != options.workgroup_size {
        return Err(MslError::UnsupportedJirShape(
            "MSL options do not match SPIR-V artifact workgroup",
        ));
    }
    let literals = artifact_constant_literals(artifact);
    let global_emitters: [MslGlobalArtifactEmitter; 5] = [
        emit_storage_global_3d_strided_write_from_spirv_artifact,
        emit_storage_global_3d_write_from_spirv_artifact,
        emit_storage_global_2d_strided_write_from_spirv_artifact,
        emit_storage_global_2d_write_from_spirv_artifact,
        emit_storage_global_strided_write_from_spirv_artifact,
    ];
    for literal in literals.iter().copied() {
        for emitter in global_emitters {
            if let Ok(source) = emitter(artifact, options_for(&options), literal) {
                return Ok(source);
            }
        }
    }
    for value in literals.iter().copied() {
        for length in literals.iter().copied() {
            if let Ok(source) = emit_storage_global_write_from_spirv_artifact(
                artifact,
                options_for(&options),
                value,
                length,
            ) {
                return Ok(source);
            }
        }
    }

    let u32_operations = [
        BinaryOp::Add,
        BinaryOp::Subtract,
        BinaryOp::Multiply,
        BinaryOp::Divide,
        BinaryOp::Remainder,
        BinaryOp::BitAnd,
        BinaryOp::BitOr,
        BinaryOp::BitXor,
        BinaryOp::ShiftLeft,
        BinaryOp::ShiftRight,
    ];
    for literal in literals.iter().copied() {
        for operation in u32_operations {
            if let Ok(source) = emit_storage_binary_from_spirv_artifact(
                artifact,
                options_for(&options),
                operation,
                literal,
            ) {
                return Ok(source);
            }
        }
    }

    let f32_operations = [
        F32ArithmeticOp::Add,
        F32ArithmeticOp::Subtract,
        F32ArithmeticOp::Multiply,
    ];
    for operand_bits in literals.iter().copied() {
        for operation in f32_operations {
            if let Ok(source) = emit_storage_f32_binary_from_spirv_artifact(
                artifact,
                options_for(&options),
                operand_bits,
                operation,
            ) {
                return Ok(source);
            }
        }
        for lanes in 2..=4 {
            for operation in f32_operations {
                if let Ok(source) = emit_storage_vector_f32_binary_lanes_from_spirv_artifact(
                    artifact,
                    options_for(&options),
                    operand_bits,
                    operation,
                    lanes,
                ) {
                    return Ok(source);
                }
            }
        }
    }
    Err(MslError::UnsupportedJirShape(
        "no supported MSL artifact family matched",
    ))
}

fn options_for(options: &MslOptions) -> MslOptions {
    MslOptions {
        workgroup_size: options.workgroup_size,
    }
}

fn artifact_constant_literals(artifact: &SpirvArtifact) -> Vec<u32> {
    const OP_CONSTANT: u16 = 43;
    let mut literals = Vec::new();
    let mut cursor = 5_usize;
    while cursor < artifact.words.len() {
        let instruction = artifact.words[cursor];
        let word_count = (instruction >> 16) as usize;
        if word_count == 0 || cursor + word_count > artifact.words.len() {
            break;
        }
        if (instruction & 0xFFFF) as u16 == OP_CONSTANT && word_count == 4 {
            literals.push(artifact.words[cursor + 3]);
        }
        cursor += word_count;
    }
    literals.sort_unstable();
    literals.dedup();
    literals
}

/// Host tool required by an explicitly requested general SPIR-V→MSL
/// translation. The strict family dispatcher does not depend on this tool.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpirvMslToolchain {
    /// SPIRV-Cross executable used for SPIR-V→MSL lowering.
    pub spirv_cross: PathBuf,
}

impl SpirvMslToolchain {
    /// Discovers SPIRV-Cross from `JADREN_SPIRV_CROSS` or the host `PATH`.
    #[must_use]
    pub fn discover() -> Option<Self> {
        let names = if cfg!(windows) {
            ["spirv-cross.exe", "spirv-cross"]
        } else {
            ["spirv-cross", "spirv-cross.exe"]
        };
        if let Some(path) = std::env::var_os("JADREN_SPIRV_CROSS").map(PathBuf::from)
            && path.is_file()
        {
            return Some(Self { spirv_cross: path });
        }
        let path = std::env::var_os("PATH")
            .into_iter()
            .flat_map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
            .flat_map(|directory| names.iter().map(move |name| directory.join(name)))
            .find(|candidate| candidate.is_file())?;
        Some(Self { spirv_cross: path })
    }
}

/// Translates a structurally validated SPIR-V module to MSL through an explicitly
/// discovered SPIRV-Cross process. This is intentionally separate from
/// [`translate_spirv_artifact_to_msl`]: callers opt into the external general
/// route and must perform any family/resource policy checks appropriate for
/// their execution path.
pub fn translate_spirv_to_msl(words: &[u32], entry_name: &str) -> Result<String, MslError> {
    validate_external_spirv(words)?;
    if !valid_identifier(entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    inspect_spirv_source_module(words, entry_name).map_err(|error| match error {
        SpirvSourceTranslationError::InvalidInput(reason)
        | SpirvSourceTranslationError::InvalidSpirv(reason) => MslError::InvalidSpirv(reason),
        SpirvSourceTranslationError::EntryPointNotFound(entry) => {
            MslError::SpirvTranslation(format!("entry point `{entry}` was not found"))
        }
        SpirvSourceTranslationError::Tool(error) => MslError::SpirvTranslation(error.to_string()),
        SpirvSourceTranslationError::EmptySource => {
            MslError::SpirvTranslation("empty source output".to_owned())
        }
    })?;
    let toolchain = SpirvMslToolchain::discover().ok_or(MslError::SpirvToolchainUnavailable)?;
    let source = translate_spirv_source_report_for_backend(
        words,
        entry_name,
        &toolchain.spirv_cross,
        GpuBackend::Metal,
    )
    .map_err(|error| match error {
        SpirvSourceTranslationError::InvalidInput(reason) => {
            MslError::SpirvTranslation(reason.to_owned())
        }
        SpirvSourceTranslationError::InvalidSpirv(reason) => {
            MslError::SpirvTranslation(reason.to_owned())
        }
        SpirvSourceTranslationError::EntryPointNotFound(entry) => {
            MslError::SpirvTranslation(format!("entry point `{entry}` was not found"))
        }
        SpirvSourceTranslationError::Tool(error) => MslError::SpirvTranslation(error.to_string()),
        SpirvSourceTranslationError::EmptySource => {
            MslError::SpirvTranslation("empty source output".to_owned())
        }
    })?
    .source;
    if !source.contains(&format!("kernel void {entry_name}(")) {
        return Err(MslError::SpirvTranslation(
            "SPIRV-Cross output is missing the requested kernel entry".to_owned(),
        ));
    }
    Ok(source)
}

/// Translates a backend-neutral artifact through the explicit SPIRV-Cross
/// route after validating its metadata and structural SPIR-V stream.
///
/// This function is intentionally separate from
/// [`translate_spirv_artifact_to_msl`]. It never falls back to the strict
/// family dispatcher, because arbitrary external shader resources still need
/// a caller-owned binding, alias and native Metal policy.
pub fn translate_spirv_artifact_to_msl_external(
    artifact: &SpirvArtifact,
) -> Result<String, MslError> {
    Ok(translate_spirv_artifact_to_msl_external_report(artifact)?.source)
}

/// Translates an artifact to MSL and returns the shared source audit report.
pub fn translate_spirv_artifact_to_msl_external_report(
    artifact: &SpirvArtifact,
) -> Result<ArtifactSourceTranslationReport, MslError> {
    validate_external_artifact(artifact)?;
    let toolchain = SpirvMslToolchain::discover().ok_or(MslError::SpirvToolchainUnavailable)?;
    let report = translate_spirv_artifact_source(
        artifact,
        &toolchain.spirv_cross,
        ArtifactSourceBackend::Msl,
    )
    .map_err(|error| match error {
        ArtifactSourceTranslationError::Contract(_) => {
            MslError::InvalidSpirvArtifact("source translation contract failed")
        }
        ArtifactSourceTranslationError::Tool(error) => {
            MslError::SpirvTranslation(error.to_string())
        }
    })?;
    validate_external_msl_source(&report.source, artifact)?;
    Ok(report)
}

fn validate_external_artifact(artifact: &SpirvArtifact) -> Result<(), MslError> {
    MslOptions::new(artifact.workgroup_size)?;
    validate_spirv_artifact_contract(artifact)
        .map_err(|_| MslError::InvalidSpirvArtifact("shared artifact contract failed"))?;
    validate_msl_resource_address_spaces(artifact)?;
    validate_msl_descriptor_sets(artifact)?;
    if !valid_identifier(&artifact.entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    Ok(())
}

fn validate_msl_descriptor_sets(artifact: &SpirvArtifact) -> Result<(), MslError> {
    if let Some(resource) = artifact
        .resources
        .iter()
        .find(|resource| resource.descriptor_set != 0)
    {
        return Err(MslError::SpirvTranslation(format!(
            "MSL resource binding {} uses unsupported descriptor set {}",
            resource.binding, resource.descriptor_set
        )));
    }
    Ok(())
}

fn validate_msl_resource_address_spaces(artifact: &SpirvArtifact) -> Result<(), MslError> {
    if let Some(resource) = artifact
        .resources
        .iter()
        .find(|resource| resource.address_space != AddressSpace::Storage)
    {
        return Err(MslError::SpirvTranslation(format!(
            "MSL resource binding {} uses unsupported address space {:?}",
            resource.binding, resource.address_space
        )));
    }
    Ok(())
}

fn validate_external_msl_source(source: &str, artifact: &SpirvArtifact) -> Result<(), MslError> {
    if source.trim().is_empty()
        || !source.contains(&format!("kernel void {}(", artifact.entry_name))
    {
        return Err(MslError::SpirvTranslation(
            "MSL output is missing the requested kernel entry".to_owned(),
        ));
    }
    let mut seen = Vec::new();
    let mut remaining = source;
    while let Some(start) = remaining.find("[[buffer(") {
        let after_marker = &remaining[start + "[[buffer(".len()..];
        let end = after_marker.find(")]]").ok_or_else(|| {
            MslError::SpirvTranslation("MSL buffer attribute is truncated".to_owned())
        })?;
        let binding = after_marker[..end].parse::<u32>().map_err(|_| {
            MslError::SpirvTranslation("MSL buffer attribute is not numeric".to_owned())
        })?;
        let resource = artifact
            .resources
            .iter()
            .find(|resource| resource.binding == binding)
            .ok_or_else(|| {
                MslError::SpirvTranslation(format!(
                    "MSL output contains unknown buffer binding {binding}"
                ))
            })?;
        let declaration = remaining[..start].rsplit(',').next().unwrap_or_default();
        if resource.address_space != AddressSpace::Storage {
            return Err(MslError::SpirvTranslation(format!(
                "MSL resource binding {binding} uses unsupported address space {:?}",
                resource.address_space
            )));
        }
        if resource.descriptor_set != 0 {
            return Err(MslError::SpirvTranslation(format!(
                "MSL resource binding {binding} uses unsupported descriptor set {}",
                resource.descriptor_set
            )));
        }
        let actual_name = msl_device_resource_name(declaration).ok_or_else(|| {
            MslError::SpirvTranslation(format!(
                "MSL resource binding {binding} has no valid resource name"
            ))
        })?;
        if actual_name != resource.name {
            return Err(MslError::SpirvTranslation(format!(
                "MSL resource binding {binding} name differs from artifact metadata"
            )));
        }
        if let Some(expected_type) = resource.element_type_info {
            let actual_type = msl_device_element_type(declaration).ok_or_else(|| {
                MslError::SpirvTranslation(format!(
                    "MSL resource binding {binding} has no supported device element type"
                ))
            })?;
            if actual_type != expected_type {
                return Err(MslError::SpirvTranslation(format!(
                    "MSL resource binding {binding} element type differs from artifact metadata"
                )));
            }
        }
        if let Some(expected_stride) = resource.element_stride {
            let actual_stride = msl_device_element_stride(declaration).ok_or_else(|| {
                MslError::SpirvTranslation(format!(
                    "MSL resource binding {binding} has no supported device element type"
                ))
            })?;
            if actual_stride != expected_stride {
                return Err(MslError::SpirvTranslation(format!(
                    "MSL resource binding {binding} element stride differs from artifact metadata"
                )));
            }
        }
        let read_only = msl_device_resource_is_read_only(declaration).ok_or_else(|| {
            MslError::SpirvTranslation(format!(
                "MSL resource binding {binding} has no device access qualifier"
            ))
        })?;
        let access_is_valid = match resource.access {
            ResourceAccess::ReadOnly => read_only,
            ResourceAccess::WriteOnly | ResourceAccess::ReadWrite => !read_only,
        };
        if !access_is_valid {
            return Err(MslError::SpirvTranslation(format!(
                "MSL resource binding {binding} violates artifact access policy"
            )));
        }
        if seen.contains(&binding) {
            return Err(MslError::SpirvTranslation(format!(
                "MSL output repeats buffer binding {binding}"
            )));
        }
        seen.push(binding);
        remaining = &after_marker[end + ")]]".len()..];
    }
    for resource in &artifact.resources {
        if !seen.contains(&resource.binding) {
            return Err(MslError::SpirvTranslation(format!(
                "MSL output is missing buffer binding {}",
                resource.binding
            )));
        }
    }
    Ok(())
}

fn msl_device_resource_name(declaration: &str) -> Option<&str> {
    let pointer = declaration.rfind('*')?;
    let name = declaration[pointer + 1..].split_whitespace().next()?;
    valid_identifier(name).then_some(name)
}

fn msl_device_element_type(declaration: &str) -> Option<ResourceElementType> {
    if !declaration.contains("device") {
        return None;
    }
    let pointer = declaration.rfind('*')?;
    let element = declaration[..pointer].split_whitespace().last()?;
    ResourceElementType::from_shader_name(element)
}

fn msl_device_element_stride(declaration: &str) -> Option<u32> {
    msl_device_element_type(declaration)?.byte_stride()
}

fn msl_device_resource_is_read_only(declaration: &str) -> Option<bool> {
    let device = declaration.find("device")?;
    let after_device = &declaration[device + "device".len()..];
    Some(after_device.trim_start().starts_with("const"))
}

fn validate_external_spirv(words: &[u32]) -> Result<(), MslError> {
    if words.len() < 5 {
        return Err(MslError::InvalidSpirv("header-too-short"));
    }
    if words[0] != 0x0723_0203 {
        return Err(MslError::InvalidSpirv("bad-magic"));
    }
    if words[1] == 0 || words[3] == 0 {
        return Err(MslError::InvalidSpirv("header-metadata-invalid"));
    }
    Ok(())
}

/// Emits the bounded runtime-length `f32x4` vector binary source contract.
///
/// The vector element is represented as one 16-byte `float4` storage element.
/// Only the shared scalar IEEE operation family is accepted so the generated
/// source stays aligned with the scalar MSL/Vulkan/DX12 contract.
pub fn emit_storage_vector_f32_binary(
    entry_name: &str,
    options: MslOptions,
    operand_bits: u32,
    operation: F32ArithmeticOp,
) -> Result<String, MslError> {
    emit_storage_vector_f32_binary_lanes(entry_name, options, operand_bits, operation, 4)
}

/// Emits the bounded runtime-length `f32` vector binary source contract for
/// two to four lanes. The x4 function remains the compatibility wrapper.
pub fn emit_storage_vector_f32_binary_lanes(
    entry_name: &str,
    options: MslOptions,
    operand_bits: u32,
    operation: F32ArithmeticOp,
    lanes: u16,
) -> Result<String, MslError> {
    if !(2..=4).contains(&lanes) {
        return Err(MslError::UnsupportedJirShape(
            "vector f32 lanes must be in 2..=4",
        ));
    }
    if !valid_identifier(entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    let [x, y, z] = options.workgroup_size;
    let max_threads = u64::from(x)
        .saturating_mul(u64::from(y))
        .saturating_mul(u64::from(z));
    Ok(format!(
        concat!(
            "#include <metal_stdlib>\n\nusing namespace metal;\n\n",
            "struct JadrenParams {{\n    uint length;\n}};\n\n",
            "kernel void {entry_name}(\n",
            "    device const float{lanes}* input [[buffer(0)]],\n",
            "    device float{lanes}* output [[buffer(1)]],\n",
            "    constant JadrenParams& params [[buffer(2)]],\n",
            "    uint3 gid [[thread_position_in_grid]])\n",
            "    [[max_total_threads_per_threadgroup({max_threads})]] {{\n",
            "    if (gid.x < params.length) {{\n",
            "        output[gid.x] = input[gid.x] {operator} float{lanes}(as_type<float>({operand_bits}u));\n",
            "    }}\n}}\n"
        ),
        entry_name = entry_name,
        max_threads = max_threads,
        operator = f32_msl_operator(operation),
        operand_bits = operand_bits,
        lanes = lanes,
    ))
}

/// Lowers the strict JIR `f32x4` runtime-length vector-add shape to MSL.
pub fn emit_storage_vector_f32_add_from_jir(
    module: &Module,
    function: FunctionId,
    options: MslOptions,
) -> Result<String, MslError> {
    emit_storage_vector_f32_binary_from_jir(module, function, options, F32ArithmeticOp::Add)
}

/// Lowers the strict JIR `f32x4` runtime-length vector binary shape to MSL.
///
/// The requested operation must match the JIR vector binary instruction.
pub fn emit_storage_vector_f32_binary_from_jir(
    module: &Module,
    function: FunctionId,
    options: MslOptions,
    operation: F32ArithmeticOp,
) -> Result<String, MslError> {
    emit_storage_vector_f32_binary_lanes_from_jir(module, function, options, operation, 4)
}

/// Lowers the strict runtime-length `f32x2`/`f32x3`/`f32x4` vector binary
/// shape to MSL.
pub fn emit_storage_vector_f32_binary_lanes_from_jir(
    module: &Module,
    function: FunctionId,
    options: MslOptions,
    operation: F32ArithmeticOp,
    lanes: u16,
) -> Result<String, MslError> {
    let addend_bits = vector_f32_binary_operand_from_jir_lanes(module, function, operation, lanes)?;
    let name = module
        .functions
        .get(function.index())
        .ok_or(MslError::UnsupportedJirShape("missing function"))?
        .name
        .clone();
    emit_storage_vector_f32_binary_lanes(&name, options, addend_bits, operation, lanes)
}

fn vector_f32_binary_operand_from_jir_lanes(
    module: &Module,
    function: FunctionId,
    operation: F32ArithmeticOp,
    lanes: u16,
) -> Result<u32, MslError> {
    if !(2..=4).contains(&lanes) {
        return Err(MslError::UnsupportedJirShape(
            "vector f32 lanes must be in 2..=4",
        ));
    }
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(MslError::JirVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(MslError::UnsupportedJirShape("missing function"))?;
    if !valid_identifier(&function_data.name) {
        return Err(MslError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) || function_data.parameters.len() != 3
    {
        return Err(MslError::UnsupportedJirShape(
            "vector f32 binary requires three resources and Unit result",
        ));
    }
    let resource_kind = |parameter: &jadren_jir::Parameter| {
        let Some(Type::Pointer {
            pointee,
            address_space: AddressSpace::Storage,
        }) = module.types.get(parameter.ty.index())
        else {
            return None;
        };
        Some(*pointee)
    };
    let Some(vector_type) = resource_kind(&function_data.parameters[0]) else {
        return Err(MslError::UnsupportedJirShape(
            "vector f32 binary input must be a storage pointer",
        ));
    };
    if resource_kind(&function_data.parameters[1]) != Some(vector_type) {
        return Err(MslError::UnsupportedJirShape(
            "vector f32 binary output must match input vector",
        ));
    }
    let Some(Type::Vector {
        element,
        lanes: vector_lanes,
    }) = module.types.get(vector_type.index())
    else {
        return Err(MslError::UnsupportedJirShape(
            "vector f32 binary input/output must be a supported f32 vector",
        ));
    };
    if *vector_lanes != lanes {
        return Err(MslError::UnsupportedJirShape(
            "vector f32 binary input/output lane count differs from request",
        ));
    }
    let scalar_type = *element;
    if !matches!(
        module.types.get(scalar_type.index()),
        Some(Type::Float { bits: 32 })
    ) {
        return Err(MslError::UnsupportedJirShape(
            "vector f32 binary element must be f32",
        ));
    }
    let Some(length_type) = resource_kind(&function_data.parameters[2]) else {
        return Err(MslError::UnsupportedJirShape(
            "vector f32 binary length must be a storage pointer",
        ));
    };
    if !matches!(
        module.types.get(length_type.index()),
        Some(Type::Integer {
            signed: false,
            bits: 32
        })
    ) {
        return Err(MslError::UnsupportedJirShape(
            "vector f32 binary length must be u32",
        ));
    }
    let block = function_data
        .blocks
        .first()
        .ok_or(MslError::UnsupportedJirShape("missing entry block"))?;
    if function_data.blocks.len() != 1
        || !block.parameters.is_empty()
        || !matches!(block.terminator, Terminator::Return { value: None })
    {
        return Err(MslError::UnsupportedJirShape(
            "vector f32 binary requires one Unit-returning entry block",
        ));
    }
    let input_parameter = &function_data.parameters[0];
    let output_parameter = &function_data.parameters[1];
    let length_parameter = &function_data.parameters[2];
    let mut builtin = None;
    let mut scalar_constant = None;
    let mut splat = None;
    let mut length_load = None;
    let mut input_offset = None;
    let mut output_offset = None;
    let mut input_load = None;
    let mut bounds = None;
    let mut binary = None;
    let mut store = None;

    for (index, instruction) in block.instructions.iter().enumerate() {
        match &instruction.kind {
            InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX) => {
                let result = instruction.result.ok_or(MslError::UnsupportedJirShape(
                    "vector builtin needs a result",
                ))?;
                if result.ty != length_type || builtin.replace((result, index)).is_some() {
                    return Err(MslError::UnsupportedJirShape(
                        "vector body requires one u32 GlobalInvocationId.x",
                    ));
                }
            }
            InstructionKind::Builtin(_) => {
                return Err(MslError::UnsupportedJirShape(
                    "vector source contract only supports GlobalInvocationId.x",
                ));
            }
            InstructionKind::Constant(Constant::FloatBits { bits }) => {
                let result = instruction.result.ok_or(MslError::UnsupportedJirShape(
                    "vector constant needs a result",
                ))?;
                if result.ty != scalar_type
                    || scalar_constant.replace((result, *bits, index)).is_some()
                {
                    return Err(MslError::UnsupportedJirShape(
                        "vector body requires one f32 constant",
                    ));
                }
            }
            InstructionKind::Constant(_) => {
                return Err(MslError::UnsupportedJirShape(
                    "vector source contract only supports FloatBits",
                ));
            }
            InstructionKind::VectorSplat { value, lanes } => {
                let result = instruction
                    .result
                    .ok_or(MslError::UnsupportedJirShape("vector splat needs a result"))?;
                if result.ty != vector_type
                    || *lanes != *vector_lanes
                    || splat.replace((result, *value, index)).is_some()
                {
                    return Err(MslError::UnsupportedJirShape(
                        "vector body requires one matching vector splat",
                    ));
                }
            }
            InstructionKind::Load { pointer, .. } if *pointer == length_parameter.value => {
                let result = instruction.result.ok_or(MslError::UnsupportedJirShape(
                    "vector length load needs a result",
                ))?;
                if result.ty != length_type || length_load.replace((result, index)).is_some() {
                    return Err(MslError::UnsupportedJirShape(
                        "vector body requires one u32 length load",
                    ));
                }
            }
            InstructionKind::Load { pointer, .. } => {
                let result = instruction.result.ok_or(MslError::UnsupportedJirShape(
                    "vector input load needs a result",
                ))?;
                if result.ty != vector_type
                    || input_load.replace((result, *pointer, index)).is_some()
                {
                    return Err(MslError::UnsupportedJirShape(
                        "vector body requires one vector input load",
                    ));
                }
            }
            InstructionKind::Offset { base, indices } => {
                let result = instruction.result.ok_or(MslError::UnsupportedJirShape(
                    "vector offset needs a result",
                ))?;
                let Some((builtin_value, _)) = builtin else {
                    return Err(MslError::UnsupportedJirShape(
                        "vector offset requires GlobalInvocationId.x",
                    ));
                };
                if indices.as_slice() != [builtin_value.value] {
                    return Err(MslError::UnsupportedJirShape(
                        "vector offset must use GlobalInvocationId.x",
                    ));
                }
                if *base == input_parameter.value {
                    if result.ty != input_parameter.ty
                        || input_offset.replace((result, index)).is_some()
                    {
                        return Err(MslError::UnsupportedJirShape(
                            "vector input offset is invalid",
                        ));
                    }
                } else if *base == output_parameter.value {
                    if result.ty != output_parameter.ty
                        || output_offset.replace((result, index)).is_some()
                    {
                        return Err(MslError::UnsupportedJirShape(
                            "vector output offset is invalid",
                        ));
                    }
                } else {
                    return Err(MslError::UnsupportedJirShape(
                        "vector offset base is invalid",
                    ));
                }
            }
            InstructionKind::BoundsCheck {
                index: checked_index,
                length,
            } => {
                if bounds.replace((*checked_index, *length, index)).is_some() {
                    return Err(MslError::UnsupportedJirShape(
                        "vector body requires one bounds check",
                    ));
                }
            }
            InstructionKind::VectorBinary { op, left, right }
                if *op == operation.as_binary_op() =>
            {
                let result = instruction.result.ok_or(MslError::UnsupportedJirShape(
                    "vector binary needs a result",
                ))?;
                if result.ty != vector_type
                    || binary.replace((result, *left, *right, index)).is_some()
                {
                    return Err(MslError::UnsupportedJirShape(
                        "vector binary operation is invalid",
                    ));
                }
            }
            InstructionKind::VectorBinary { .. } => {
                return Err(MslError::UnsupportedJirShape(
                    "vector source contract operation does not match requested f32 family",
                ));
            }
            InstructionKind::Store { pointer, value, .. } => {
                if store.replace((*pointer, *value, index)).is_some() {
                    return Err(MslError::UnsupportedJirShape(
                        "vector body requires one store",
                    ));
                }
            }
            _ => {
                return Err(MslError::UnsupportedJirShape(
                    "vector body contains an unsupported instruction",
                ));
            }
        }
    }
    let (builtin_value, builtin_index) = builtin.ok_or(MslError::UnsupportedJirShape(
        "vector body needs GlobalInvocationId.x",
    ))?;
    let (constant_value, addend_bits, constant_index) = scalar_constant.ok_or(
        MslError::UnsupportedJirShape("vector body needs an f32 constant"),
    )?;
    let (splat_value, splat_operand, splat_index) = splat.ok_or(MslError::UnsupportedJirShape(
        "vector body needs a vector splat",
    ))?;
    if splat_operand != constant_value.value {
        return Err(MslError::UnsupportedJirShape(
            "vector splat must use the f32 constant",
        ));
    }
    let (length_value, length_index) = length_load.ok_or(MslError::UnsupportedJirShape(
        "vector body needs a length load",
    ))?;
    let (input_offset_value, input_offset_index) = input_offset.ok_or(
        MslError::UnsupportedJirShape("vector body needs an input offset"),
    )?;
    let (output_offset_value, output_offset_index) = output_offset.ok_or(
        MslError::UnsupportedJirShape("vector body needs an output offset"),
    )?;
    let (bounds_index_value, bounds_length_value, bounds_index) = bounds.ok_or(
        MslError::UnsupportedJirShape("vector body needs a bounds check"),
    )?;
    if bounds_index_value != builtin_value.value || bounds_length_value != length_value.value {
        return Err(MslError::UnsupportedJirShape(
            "vector bounds check must guard index with length",
        ));
    }
    let (input_result, input_pointer, input_load_index) = input_load.ok_or(
        MslError::UnsupportedJirShape("vector body needs an input load"),
    )?;
    if input_pointer != input_offset_value.value {
        return Err(MslError::UnsupportedJirShape(
            "vector input load must use input offset",
        ));
    }
    let (binary_result, left, right, binary_index) = binary.ok_or(
        MslError::UnsupportedJirShape("vector body needs the requested f32 binary operation"),
    )?;
    if !((left == input_result.value && right == splat_value.value)
        || (right == input_result.value && left == splat_value.value))
    {
        return Err(MslError::UnsupportedJirShape(
            "vector binary operands must be input and splat",
        ));
    }
    let (store_pointer, store_value, store_index) = store.ok_or(MslError::UnsupportedJirShape(
        "vector body needs an output store",
    ))?;
    if store_pointer != output_offset_value.value || store_value != binary_result.value {
        return Err(MslError::UnsupportedJirShape(
            "vector store must write binary result to output offset",
        ));
    }
    if !(builtin_index < bounds_index
        && constant_index < splat_index
        && length_index < bounds_index
        && bounds_index < input_offset_index
        && bounds_index < output_offset_index
        && bounds_index < input_load_index
        && bounds_index < binary_index
        && bounds_index < store_index)
    {
        return Err(MslError::UnsupportedJirShape(
            "vector bounds check must precede all memory effects",
        ));
    }
    u32::try_from(addend_bits)
        .map_err(|_| MslError::UnsupportedJirShape("vector f32 constant exceeds f32 bits"))
}

/// Lowers the verified runtime-length `f32` add JIR shape to the MSL source
/// contract. The shape mirrors the SPIR-V and DX12 artifact path: three
/// storage resources, `GlobalInvocationId.x`, a length bounds check, indexed
/// input/output access and one `OpFAdd`-equivalent JIR binary operation.
pub fn emit_storage_f32_add_from_jir(
    module: &Module,
    function: FunctionId,
    options: MslOptions,
) -> Result<String, MslError> {
    emit_storage_f32_binary_from_jir(module, function, options, F32ArithmeticOp::Add)
}

/// Lowers the verified runtime-length scalar `f32` binary JIR shape to the
/// MSL source contract. The requested operation must match the JIR binary
/// instruction exactly; unsupported or mismatched shapes are rejected.
pub fn emit_storage_f32_binary_from_jir(
    module: &Module,
    function: FunctionId,
    options: MslOptions,
    operation: F32ArithmeticOp,
) -> Result<String, MslError> {
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(MslError::JirVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(MslError::UnsupportedJirShape("missing function"))?;
    if !valid_identifier(&function_data.name) {
        return Err(MslError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) || function_data.parameters.len() != 3
    {
        return Err(MslError::UnsupportedJirShape(
            "dynamic f32 binary requires three resources and Unit result",
        ));
    }
    let data_resources_valid = function_data.parameters[..2].iter().all(|parameter| {
        matches!(
            module.types.get(parameter.ty.index()),
            Some(Type::Pointer {
                pointee,
                address_space: AddressSpace::Storage,
            }) if matches!(module.types.get(pointee.index()), Some(Type::Float { bits: 32 }))
        )
    });
    let length_resource_valid = matches!(
        module.types.get(function_data.parameters[2].ty.index()),
        Some(Type::Pointer {
            pointee,
            address_space: AddressSpace::Storage,
        }) if matches!(module.types.get(pointee.index()), Some(Type::Integer { signed: false, bits: 32 }))
    );
    if !data_resources_valid || !length_resource_valid {
        return Err(MslError::UnsupportedJirShape(
            "input/output must be storage f32 pointers and length a storage u32 pointer",
        ));
    }
    let block = function_data
        .blocks
        .first()
        .ok_or(MslError::UnsupportedJirShape("missing entry block"))?;
    if function_data.blocks.len() != 1
        || !block.parameters.is_empty()
        || block.instructions.len() != 9
    {
        return Err(MslError::UnsupportedJirShape(
            "body must contain builtin, operand, length, bounds, offsets, load, f32 binary and store",
        ));
    }
    let builtin = &block.instructions[0];
    let Some(builtin_result) = builtin.result else {
        return Err(MslError::UnsupportedJirShape(
            "builtin must produce a value",
        ));
    };
    let u32_type = module
        .types
        .iter()
        .position(|ty| {
            matches!(
                ty,
                Type::Integer {
                    signed: false,
                    bits: 32
                }
            )
        })
        .map(TypeId::new)
        .ok_or(MslError::UnsupportedJirShape(
            "module requires a u32 index type",
        ))?;
    let f32_type = module
        .types
        .iter()
        .position(|ty| matches!(ty, Type::Float { bits: 32 }))
        .map(TypeId::new)
        .ok_or(MslError::UnsupportedJirShape(
            "module requires an f32 value type",
        ))?;
    if !matches!(
        &builtin.kind,
        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX)
    ) || builtin_result.ty != u32_type
    {
        return Err(MslError::UnsupportedJirShape(
            "first instruction must be GlobalInvocationId.x with u32 result",
        ));
    }
    let constant = &block.instructions[1];
    let Some(constant_result) = constant.result else {
        return Err(MslError::UnsupportedJirShape(
            "operand must produce a value",
        ));
    };
    let addend_bits = match (&constant.kind, constant_result.ty == f32_type) {
        (InstructionKind::Constant(Constant::FloatBits { bits }), true)
            if *bits <= u64::from(u32::MAX) =>
        {
            *bits as u32
        }
        _ => {
            return Err(MslError::UnsupportedJirShape(
                "second instruction must be an f32 binary32 operand",
            ));
        }
    };
    let length = &block.instructions[2];
    let Some(length_result) = length.result else {
        return Err(MslError::UnsupportedJirShape(
            "length load must produce a value",
        ));
    };
    if !matches!(
        &length.kind,
        InstructionKind::Load { pointer, .. }
            if *pointer == function_data.parameters[2].value
                && length_result.ty == u32_type
    ) {
        return Err(MslError::UnsupportedJirShape(
            "third instruction must load the u32 length resource",
        ));
    }
    let bounds = &block.instructions[3];
    if bounds.result.is_some()
        || !matches!(
            &bounds.kind,
            InstructionKind::BoundsCheck { index, length: bound_length }
                if *index == builtin_result.value && *bound_length == length_result.value
        )
    {
        return Err(MslError::UnsupportedJirShape(
            "fourth instruction must bounds-check the builtin index",
        ));
    }
    let input_offset = &block.instructions[4];
    let Some(input_offset_result) = input_offset.result else {
        return Err(MslError::UnsupportedJirShape(
            "input offset must produce a pointer",
        ));
    };
    if !matches!(
        &input_offset.kind,
        InstructionKind::Offset { base, indices }
            if *base == function_data.parameters[0].value
                && indices.as_slice() == [builtin_result.value]
    ) {
        return Err(MslError::UnsupportedJirShape("input offset is invalid"));
    }
    let input_load = &block.instructions[5];
    let Some(input_result) = input_load.result else {
        return Err(MslError::UnsupportedJirShape(
            "input load must produce a value",
        ));
    };
    if !matches!(
        &input_load.kind,
        InstructionKind::Load { pointer, .. }
            if *pointer == input_offset_result.value && input_result.ty == f32_type
    ) {
        return Err(MslError::UnsupportedJirShape("input load is invalid"));
    }
    let binary = &block.instructions[6];
    let Some(binary_result) = binary.result else {
        return Err(MslError::UnsupportedJirShape(
            "f32 binary must produce a value",
        ));
    };
    if !matches!(
        &binary.kind,
        InstructionKind::Binary { op, left, right }
            if *op == operation.as_binary_op()
                && binary_result.ty == f32_type
                && ((*left == input_result.value && *right == constant_result.value)
                    || (*right == input_result.value && *left == constant_result.value))
    ) {
        return Err(MslError::UnsupportedJirShape(
            "f32 binary operands are invalid",
        ));
    }
    let output_offset = &block.instructions[7];
    let Some(output_offset_result) = output_offset.result else {
        return Err(MslError::UnsupportedJirShape(
            "output offset must produce a pointer",
        ));
    };
    if !matches!(
        &output_offset.kind,
        InstructionKind::Offset { base, indices }
            if *base == function_data.parameters[1].value
                && indices.as_slice() == [builtin_result.value]
    ) {
        return Err(MslError::UnsupportedJirShape("output offset is invalid"));
    }
    let store = &block.instructions[8];
    if store.result.is_some()
        || !matches!(
            &store.kind,
            InstructionKind::Store { pointer, value, .. }
                if *pointer == output_offset_result.value && *value == binary_result.value
        )
        || !matches!(block.terminator, Terminator::Return { value: None })
    {
        return Err(MslError::UnsupportedJirShape(
            "store/return terminator is invalid",
        ));
    }
    emit_storage_f32_binary(&function_data.name, options, addend_bits, operation)
}

/// Lowers the verified dynamic-length JIR storage-add shape to the MSL
/// bounded source contract.
///
/// The accepted body is the same portable kernel used by the SPIR-V backend:
/// `GlobalInvocationId.x`, addend constant, length load, bounds check, input
/// offset/load, add, output offset and store. Unsupported JIR is rejected
/// before source generation.
pub fn emit_storage_add_from_jir(
    module: &Module,
    function: FunctionId,
    options: MslOptions,
) -> Result<String, MslError> {
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(MslError::JirVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(MslError::UnsupportedJirShape("missing function"))?;
    if !valid_identifier(&function_data.name) {
        return Err(MslError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) || function_data.parameters.len() != 3
    {
        return Err(MslError::UnsupportedJirShape(
            "dynamic storage add requires three resources and Unit result",
        ));
    }
    let resources = function_data
        .parameters
        .iter()
        .map(|parameter| {
            module
                .types
                .get(parameter.ty.index())
                .and_then(|ty| match ty {
                    Type::Pointer {
                        pointee,
                        address_space: AddressSpace::Storage,
                    } if matches!(
                        module.types.get(pointee.index()),
                        Some(Type::Integer {
                            signed: false,
                            bits: 32
                        })
                    ) =>
                    {
                        Some(())
                    }
                    _ => None,
                })
        })
        .collect::<Option<Vec<_>>>();
    if resources.is_none() {
        return Err(MslError::UnsupportedJirShape(
            "all resources must be storage u32 pointers",
        ));
    }
    let block = function_data
        .blocks
        .first()
        .ok_or(MslError::UnsupportedJirShape("missing entry block"))?;
    if function_data.blocks.len() != 1
        || !block.parameters.is_empty()
        || block.instructions.len() != 9
    {
        return Err(MslError::UnsupportedJirShape(
            "body must contain builtin, length, bounds, offsets, load, add and store",
        ));
    }
    let builtin = &block.instructions[0];
    let Some(builtin_result) = builtin.result else {
        return Err(MslError::UnsupportedJirShape(
            "builtin must produce a value",
        ));
    };
    if !matches!(
        &builtin.kind,
        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX)
    ) {
        return Err(MslError::UnsupportedJirShape(
            "first instruction must be GlobalInvocationId.x",
        ));
    }
    let constant = &block.instructions[1];
    let Some(constant_result) = constant.result else {
        return Err(MslError::UnsupportedJirShape("addend must produce a value"));
    };
    let addend = match (&constant.kind, constant_result.ty == builtin_result.ty) {
        (InstructionKind::Constant(Constant::Integer { value }), true) => u32::try_from(*value)
            .map_err(|_| MslError::UnsupportedJirShape("addend is outside u32"))?,
        _ => {
            return Err(MslError::UnsupportedJirShape(
                "second instruction must be a u32 constant",
            ));
        }
    };
    let length = &block.instructions[2];
    let Some(length_result) = length.result else {
        return Err(MslError::UnsupportedJirShape(
            "length load must produce a value",
        ));
    };
    if !matches!(
        &length.kind,
        InstructionKind::Load { pointer, .. }
            if *pointer == function_data.parameters[2].value
                && length_result.ty == builtin_result.ty
    ) {
        return Err(MslError::UnsupportedJirShape(
            "third instruction must load the length resource",
        ));
    }
    let bounds = &block.instructions[3];
    if bounds.result.is_some()
        || !matches!(
            &bounds.kind,
            InstructionKind::BoundsCheck { index, length: bound_length }
                if *index == builtin_result.value && *bound_length == length_result.value
        )
    {
        return Err(MslError::UnsupportedJirShape(
            "fourth instruction must bounds-check the builtin index",
        ));
    }
    let input_offset = &block.instructions[4];
    let Some(input_offset_result) = input_offset.result else {
        return Err(MslError::UnsupportedJirShape(
            "input offset must produce a pointer",
        ));
    };
    if !matches!(
        &input_offset.kind,
        InstructionKind::Offset { base, indices }
            if *base == function_data.parameters[0].value
                && indices.as_slice() == [builtin_result.value]
    ) {
        return Err(MslError::UnsupportedJirShape("input offset is invalid"));
    }
    let input_load = &block.instructions[5];
    let Some(input_result) = input_load.result else {
        return Err(MslError::UnsupportedJirShape(
            "input load must produce a value",
        ));
    };
    if !matches!(
        &input_load.kind,
        InstructionKind::Load { pointer, .. }
            if *pointer == input_offset_result.value && input_result.ty == builtin_result.ty
    ) {
        return Err(MslError::UnsupportedJirShape("input load is invalid"));
    }
    let binary = &block.instructions[6];
    let Some(binary_result) = binary.result else {
        return Err(MslError::UnsupportedJirShape("add must produce a value"));
    };
    if !matches!(
        &binary.kind,
        InstructionKind::Binary { op, left, right }
            if *op == BinaryOp::Add
                && binary_result.ty == builtin_result.ty
                && ((*left == input_result.value && *right == constant_result.value)
                    || (*right == input_result.value && *left == constant_result.value))
    ) {
        return Err(MslError::UnsupportedJirShape("add operands are invalid"));
    }
    let output_offset = &block.instructions[7];
    let Some(output_offset_result) = output_offset.result else {
        return Err(MslError::UnsupportedJirShape(
            "output offset must produce a pointer",
        ));
    };
    if !matches!(
        &output_offset.kind,
        InstructionKind::Offset { base, indices }
            if *base == function_data.parameters[1].value
                && indices.as_slice() == [builtin_result.value]
    ) {
        return Err(MslError::UnsupportedJirShape("output offset is invalid"));
    }
    let store = &block.instructions[8];
    if store.result.is_some()
        || !matches!(
            &store.kind,
            InstructionKind::Store { pointer, value, .. }
                if *pointer == output_offset_result.value && *value == binary_result.value
        )
        || !matches!(block.terminator, Terminator::Return { value: None })
    {
        return Err(MslError::UnsupportedJirShape(
            "store/return terminator is invalid",
        ));
    }
    emit_storage_add(&function_data.name, options, addend)
}

/// Lowers a dataflow-tolerant runtime-length `u32` BinaryOp JIR shape to MSL.
///
/// The accepted semantic body contains one `GlobalInvocationIdX`, one length
/// load, one bounds check, indexed input/output offsets, one input load, one
/// requested binary operation and one store. Instructions may be reordered in
/// SSA order, but the bounds check must precede every memory effect. MSL is
/// emitted in canonical order and never substitutes a different operation.
pub fn emit_storage_binary_from_jir(
    module: &Module,
    function: FunctionId,
    options: MslOptions,
    operation: BinaryOp,
) -> Result<String, MslError> {
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(MslError::JirVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(MslError::UnsupportedJirShape("missing function"))?;
    if !valid_identifier(&function_data.name) {
        return Err(MslError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) || function_data.parameters.len() != 3
    {
        return Err(MslError::UnsupportedJirShape(
            "dynamic storage binary requires three resources and Unit result",
        ));
    }
    let resources_valid = function_data.parameters.iter().all(|parameter| {
        matches!(
            module.types.get(parameter.ty.index()),
            Some(Type::Pointer {
                pointee,
                address_space: AddressSpace::Storage,
            }) if matches!(
                module.types.get(pointee.index()),
                Some(Type::Integer {
                    signed: false,
                    bits: 32
                })
            )
        )
    });
    if !resources_valid {
        return Err(MslError::UnsupportedJirShape(
            "all resources must be storage u32 pointers",
        ));
    }
    let block = function_data
        .blocks
        .first()
        .ok_or(MslError::UnsupportedJirShape("missing entry block"))?;
    if function_data.blocks.len() != 1
        || !block.parameters.is_empty()
        || !matches!(block.terminator, Terminator::Return { value: None })
    {
        return Err(MslError::UnsupportedJirShape(
            "body must contain one Unit-returning entry block",
        ));
    }
    let input_parameter = &function_data.parameters[0];
    let output_parameter = &function_data.parameters[1];
    let length_parameter = &function_data.parameters[2];
    let u32_type = match module.types.get(input_parameter.ty.index()) {
        Some(Type::Pointer { pointee, .. }) => *pointee,
        _ => {
            return Err(MslError::UnsupportedJirShape(
                "input resource must be a storage pointer",
            ));
        }
    };
    let mut builtin = None;
    let mut constants = Vec::new();
    let mut length_load = None;
    let mut input_offset = None;
    let mut output_offset = None;
    let mut input_loads = Vec::new();
    let mut bounds = None;
    let mut binary = None;
    let mut store = None;

    for (instruction_index, instruction) in block.instructions.iter().enumerate() {
        match &instruction.kind {
            InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX) => {
                let result = instruction.result.ok_or(MslError::UnsupportedJirShape(
                    "builtin must produce a value",
                ))?;
                if result.ty != u32_type || builtin.replace((result, instruction_index)).is_some() {
                    return Err(MslError::UnsupportedJirShape(
                        "body requires one u32 GlobalInvocationId.x",
                    ));
                }
            }
            InstructionKind::Builtin(_) => {
                return Err(MslError::UnsupportedJirShape(
                    "MSL source contract only supports GlobalInvocationId.x",
                ));
            }
            InstructionKind::Constant(Constant::Integer { value }) => {
                let result = instruction.result.ok_or(MslError::UnsupportedJirShape(
                    "constants must produce values",
                ))?;
                if result.ty != u32_type {
                    return Err(MslError::UnsupportedJirShape("constants must be u32"));
                }
                let value = u32::try_from(*value)
                    .map_err(|_| MslError::UnsupportedJirShape("operand is outside u32"))?;
                constants.push((result, value, instruction_index));
            }
            InstructionKind::Constant(_) => {
                return Err(MslError::UnsupportedJirShape(
                    "MSL source contract only supports integer constants",
                ));
            }
            InstructionKind::Load { pointer, .. } if *pointer == length_parameter.value => {
                let result = instruction.result.ok_or(MslError::UnsupportedJirShape(
                    "length load must produce a value",
                ))?;
                if result.ty != u32_type
                    || length_load.replace((result, instruction_index)).is_some()
                {
                    return Err(MslError::UnsupportedJirShape(
                        "body requires one u32 length load",
                    ));
                }
            }
            InstructionKind::Load { pointer, .. } => {
                let result = instruction.result.ok_or(MslError::UnsupportedJirShape(
                    "input load must produce a value",
                ))?;
                input_loads.push((result, *pointer, instruction_index));
            }
            InstructionKind::Offset { base, indices } => {
                let result = instruction.result.ok_or(MslError::UnsupportedJirShape(
                    "offset must produce a pointer",
                ))?;
                let Some((builtin_value, _)) = builtin else {
                    return Err(MslError::UnsupportedJirShape(
                        "offset requires a previously defined global invocation index",
                    ));
                };
                if indices.as_slice() != [builtin_value.value] {
                    return Err(MslError::UnsupportedJirShape(
                        "offset must use GlobalInvocationId.x",
                    ));
                }
                if *base == input_parameter.value {
                    if result.ty != input_parameter.ty
                        || input_offset.replace((result, instruction_index)).is_some()
                    {
                        return Err(MslError::UnsupportedJirShape("input offset is invalid"));
                    }
                } else if *base == output_parameter.value {
                    if result.ty != output_parameter.ty
                        || output_offset.replace((result, instruction_index)).is_some()
                    {
                        return Err(MslError::UnsupportedJirShape("output offset is invalid"));
                    }
                } else {
                    return Err(MslError::UnsupportedJirShape("offset base is invalid"));
                }
            }
            InstructionKind::BoundsCheck { index, length } => {
                if bounds
                    .replace((*index, *length, instruction_index))
                    .is_some()
                {
                    return Err(MslError::UnsupportedJirShape(
                        "body requires one bounds check",
                    ));
                }
            }
            InstructionKind::Binary { op, left, right } if *op == operation => {
                let result = instruction.result.ok_or(MslError::UnsupportedJirShape(
                    "binary operation must produce a value",
                ))?;
                if result.ty != u32_type
                    || binary
                        .replace((result, *left, *right, instruction_index))
                        .is_some()
                {
                    return Err(MslError::UnsupportedJirShape("binary operation is invalid"));
                }
            }
            InstructionKind::Binary { .. } => {
                return Err(MslError::UnsupportedJirShape(
                    "body contains a different binary operation",
                ));
            }
            InstructionKind::Store { pointer, value, .. } => {
                if store
                    .replace((*pointer, *value, instruction_index))
                    .is_some()
                {
                    return Err(MslError::UnsupportedJirShape("body requires one store"));
                }
            }
            _ => {
                return Err(MslError::UnsupportedJirShape(
                    "body contains an unsupported instruction",
                ));
            }
        }
    }

    let (builtin_value, builtin_index) = builtin.ok_or(MslError::UnsupportedJirShape(
        "body requires GlobalInvocationId.x",
    ))?;
    let (length_value, length_index) =
        length_load.ok_or(MslError::UnsupportedJirShape("body requires a length load"))?;
    let (input_offset_value, input_offset_index) = input_offset.ok_or(
        MslError::UnsupportedJirShape("body requires an input offset"),
    )?;
    let (output_offset_value, output_offset_index) = output_offset.ok_or(
        MslError::UnsupportedJirShape("body requires an output offset"),
    )?;
    let (checked_index, checked_length, bounds_index) = bounds.ok_or(
        MslError::UnsupportedJirShape("body requires a bounds check"),
    )?;
    if checked_index != builtin_value.value || checked_length != length_value.value {
        return Err(MslError::UnsupportedJirShape(
            "bounds check must guard the builtin index with the length",
        ));
    }
    let (input_result, input_load_index) = match input_loads.as_slice() {
        [(result, pointer, index)] if *pointer == input_offset_value.value => {
            if result.ty != u32_type {
                return Err(MslError::UnsupportedJirShape("input load must produce u32"));
            }
            (*result, *index)
        }
        _ => {
            return Err(MslError::UnsupportedJirShape(
                "body requires one input offset load",
            ));
        }
    };
    let (binary_result, binary_left, binary_right, binary_index) = binary.ok_or(
        MslError::UnsupportedJirShape("body requires a binary operation"),
    )?;
    let operand = constants
        .iter()
        .find_map(|(result, value, _)| {
            ((binary_left == result.value && binary_right == input_result.value)
                || (binary_right == result.value && binary_left == input_result.value))
                .then_some(*value)
        })
        .ok_or(MslError::UnsupportedJirShape(
            "binary operation must combine input with a u32 constant",
        ))?;
    match operation {
        BinaryOp::Divide | BinaryOp::Remainder if operand == 0 => {
            return Err(MslError::UnsupportedJirShape(
                "unsigned divisor/remainder must be non-zero",
            ));
        }
        BinaryOp::ShiftLeft | BinaryOp::ShiftRight if operand >= 32 => {
            return Err(MslError::UnsupportedJirShape(
                "u32 shift operand must be smaller than 32",
            ));
        }
        _ => {}
    }
    let (store_pointer, store_value, store_index) = store.ok_or(MslError::UnsupportedJirShape(
        "body requires an output store",
    ))?;
    if store_pointer != output_offset_value.value || store_value != binary_result.value {
        return Err(MslError::UnsupportedJirShape(
            "store must write the binary result to the output offset",
        ));
    }
    if !(builtin_index < bounds_index
        && length_index < bounds_index
        && bounds_index < input_offset_index
        && bounds_index < output_offset_index
        && bounds_index < input_load_index
        && bounds_index < binary_index
        && bounds_index < store_index)
    {
        return Err(MslError::UnsupportedJirShape(
            "bounds check must precede every memory effect",
        ));
    }
    emit_storage_binary(&function_data.name, options, operation, operand)
}

/// Checks that a generated source still carries the bounded fixture contract.
pub fn validate_storage_add(source: &str, entry_name: &str) -> Result<(), MslError> {
    if !valid_identifier(entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    let required = [
        "#include <metal_stdlib>",
        "using namespace metal;",
        "device const uint* input [[buffer(0)]]",
        "device uint* output [[buffer(1)]]",
        "constant JadrenParams& params [[buffer(2)]]",
        "uint3 gid [[thread_position_in_grid]]",
        "if (gid.x < params.length)",
    ];
    if required.iter().all(|fragment| source.contains(fragment))
        && source.contains(&format!("kernel void {entry_name}("))
    {
        Ok(())
    } else {
        Err(MslError::InvalidContract)
    }
}

/// Checks the operation and operand of a generated runtime-length `u32`
/// BinaryOp source contract.
pub fn validate_storage_binary(
    source: &str,
    entry_name: &str,
    operation: BinaryOp,
    operand: u32,
) -> Result<(), MslError> {
    if !valid_identifier(entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    validate_u32_binary_operand(operation, operand)?;
    let required = [
        "#include <metal_stdlib>",
        "using namespace metal;",
        "device const uint* input [[buffer(0)]]",
        "device uint* output [[buffer(1)]]",
        "constant JadrenParams& params [[buffer(2)]]",
        "uint3 gid [[thread_position_in_grid]]",
        "if (gid.x < params.length)",
    ];
    if required.iter().all(|fragment| source.contains(fragment))
        && source.contains(&format!(
            "output[gid.x] = input[gid.x] {} {operand}u;",
            u32_msl_operator(operation)
        ))
        && source.contains(&format!("kernel void {entry_name}("))
    {
        Ok(())
    } else {
        Err(MslError::InvalidContract)
    }
}

/// Checks that a generated source carries the bounded runtime-length `f32`
/// add contract and preserves the IEEE-754 addend bit pattern.
pub fn validate_storage_f32_add(source: &str, entry_name: &str) -> Result<(), MslError> {
    validate_storage_f32_binary(source, entry_name, F32ArithmeticOp::Add)
}

/// Checks that a generated source carries the bounded runtime-length scalar
/// `f32` binary contract for the requested operation.
pub fn validate_storage_f32_binary(
    source: &str,
    entry_name: &str,
    operation: F32ArithmeticOp,
) -> Result<(), MslError> {
    if !valid_identifier(entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    let required = [
        "#include <metal_stdlib>",
        "using namespace metal;",
        "device const float* input [[buffer(0)]]",
        "device float* output [[buffer(1)]]",
        "constant JadrenParams& params [[buffer(2)]]",
        "uint3 gid [[thread_position_in_grid]]",
        "if (gid.x < params.length)",
        "as_type<float>(",
    ];
    if required.iter().all(|fragment| source.contains(fragment))
        && source.contains(&format!(
            "output[gid.x] = input[gid.x] {} as_type<float>(",
            f32_msl_operator(operation)
        ))
        && source.contains(&format!("kernel void {entry_name}("))
    {
        Ok(())
    } else {
        Err(MslError::InvalidContract)
    }
}

/// Checks that a generated source carries the bounded runtime-length `f32x4`
/// vector-add contract and preserves the IEEE-754 addend bit pattern.
pub fn validate_storage_vector_f32_add(source: &str, entry_name: &str) -> Result<(), MslError> {
    validate_storage_vector_f32_binary(source, entry_name, F32ArithmeticOp::Add)
}

/// Checks that a generated source carries the bounded runtime-length `f32x4`
/// vector binary contract and preserves the IEEE-754 operand bit pattern.
pub fn validate_storage_vector_f32_binary(
    source: &str,
    entry_name: &str,
    operation: F32ArithmeticOp,
) -> Result<(), MslError> {
    validate_storage_vector_f32_binary_lanes(source, entry_name, operation, 4)
}

/// Checks the bounded runtime-length vector source contract for two to four
/// f32 lanes and preserves the IEEE-754 operand bit pattern.
pub fn validate_storage_vector_f32_binary_lanes(
    source: &str,
    entry_name: &str,
    operation: F32ArithmeticOp,
    lanes: u16,
) -> Result<(), MslError> {
    if !(2..=4).contains(&lanes) {
        return Err(MslError::UnsupportedJirShape(
            "vector f32 lanes must be in 2..=4",
        ));
    }
    let vector_type = format!("float{lanes}");
    if !valid_identifier(entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    let required = [
        "#include <metal_stdlib>",
        "using namespace metal;",
        &format!("device const {vector_type}* input [[buffer(0)]]"),
        &format!("device {vector_type}* output [[buffer(1)]]"),
        "constant JadrenParams& params [[buffer(2)]]",
        "uint3 gid [[thread_position_in_grid]]",
        "if (gid.x < params.length)",
        &format!("{vector_type}(as_type<float>("),
    ];
    if required.iter().all(|fragment| source.contains(fragment))
        && source.contains(&format!(
            "output[gid.x] = input[gid.x] {} {vector_type}(as_type<float>(",
            f32_msl_operator(operation),
        ))
        && source.contains(&format!("kernel void {entry_name}("))
    {
        Ok(())
    } else {
        Err(MslError::InvalidContract)
    }
}

/// Checks that a generated source carries the one-resource global-write
/// contract.
pub fn validate_storage_global_write(source: &str, entry_name: &str) -> Result<(), MslError> {
    if !valid_identifier(entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    let required = [
        "#include <metal_stdlib>",
        "using namespace metal;",
        "device uint* data [[buffer(0)]],",
        "uint3 gid [[thread_position_in_grid]]",
        "if (gid.x < ",
        "data[gid.x] = ",
    ];
    if required.iter().all(|fragment| source.contains(fragment))
        && source.contains(&format!("kernel void {entry_name}("))
    {
        Ok(())
    } else {
        Err(MslError::InvalidContract)
    }
}

/// Checks that a generated source carries the four-resource runtime-stride
/// global-write contract.
pub fn validate_storage_global_strided_write(
    source: &str,
    entry_name: &str,
) -> Result<(), MslError> {
    if !valid_identifier(entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    let required = [
        "device uint* data [[buffer(0)]],",
        "device const uint* length [[buffer(1)]],",
        "device const uint* stride [[buffer(2)]],",
        "device const uint* capacity [[buffer(3)]],",
        "if (gid.x < length[0])",
        "uint physical = gid.x * stride[0];",
        "if (physical < capacity[0])",
        "data[physical] = ",
    ];
    if required.iter().all(|fragment| source.contains(fragment))
        && source.contains(&format!("kernel void {entry_name}("))
    {
        Ok(())
    } else {
        Err(MslError::InvalidContract)
    }
}

/// Checks that a generated source carries the four-resource 2D row-major
/// global-write contract.
pub fn validate_storage_global_2d_write(source: &str, entry_name: &str) -> Result<(), MslError> {
    if !valid_identifier(entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    let required = [
        "device uint* data [[buffer(0)]],",
        "device const uint* width [[buffer(1)]],",
        "device const uint* height [[buffer(2)]],",
        "device const uint* capacity [[buffer(3)]],",
        "gid.x < width[0] && gid.y < height[0]",
        "uint physical = gid.y * width[0] + gid.x;",
        "if (physical < capacity[0])",
        "data[physical] = ",
    ];
    if required.iter().all(|fragment| source.contains(fragment))
        && source.contains(&format!("kernel void {entry_name}("))
    {
        Ok(())
    } else {
        Err(MslError::InvalidContract)
    }
}

/// Checks that a generated source carries the six-resource 2D affine-stride
/// global-write contract.
pub fn validate_storage_global_2d_strided_write(
    source: &str,
    entry_name: &str,
) -> Result<(), MslError> {
    if !valid_identifier(entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    let required = [
        "device const uint* stride_x [[buffer(3)]],",
        "device const uint* stride_y [[buffer(4)]],",
        "device const uint* capacity [[buffer(5)]],",
        "gid.x < width[0] && gid.y < height[0]",
        "gid.x * stride_x[0] + gid.y * stride_y[0]",
        "if (physical < capacity[0])",
        "data[physical] = ",
    ];
    if required.iter().all(|fragment| source.contains(fragment))
        && source.contains(&format!("kernel void {entry_name}("))
    {
        Ok(())
    } else {
        Err(MslError::InvalidContract)
    }
}

/// Checks that a generated source carries the five-resource 3D row-major
/// global-write contract.
pub fn validate_storage_global_3d_write(source: &str, entry_name: &str) -> Result<(), MslError> {
    if !valid_identifier(entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    let required = [
        "device uint* data [[buffer(0)]],",
        "device const uint* width [[buffer(1)]],",
        "device const uint* height [[buffer(2)]],",
        "device const uint* depth [[buffer(3)]],",
        "device const uint* capacity [[buffer(4)]],",
        "gid.x < width[0] && gid.y < height[0] && gid.z < depth[0]",
        "uint physical = (gid.z * height[0] + gid.y) * width[0] + gid.x;",
        "if (physical < capacity[0])",
        "data[physical] = ",
    ];
    if required.iter().all(|fragment| source.contains(fragment))
        && source.contains(&format!("kernel void {entry_name}("))
    {
        Ok(())
    } else {
        Err(MslError::InvalidContract)
    }
}

/// Checks that a generated source carries the eight-resource 3D affine-stride
/// global-write contract.
pub fn validate_storage_global_3d_strided_write(
    source: &str,
    entry_name: &str,
) -> Result<(), MslError> {
    if !valid_identifier(entry_name) {
        return Err(MslError::InvalidEntryName);
    }
    let required = [
        "device uint* data [[buffer(0)]],",
        "device const uint* width [[buffer(1)]],",
        "device const uint* height [[buffer(2)]],",
        "device const uint* depth [[buffer(3)]],",
        "device const uint* stride_x [[buffer(4)]],",
        "device const uint* stride_y [[buffer(5)]],",
        "device const uint* stride_z [[buffer(6)]],",
        "device const uint* capacity [[buffer(7)]],",
        "gid.x < width[0] && gid.y < height[0] && gid.z < depth[0]",
        "uint physical = gid.x * stride_x[0] + gid.y * stride_y[0] + gid.z * stride_z[0];",
        "if (physical < capacity[0])",
        "data[physical] = ",
    ];
    if required.iter().all(|fragment| source.contains(fragment))
        && source.contains(&format!("kernel void {entry_name}("))
    {
        Ok(())
    } else {
        Err(MslError::InvalidContract)
    }
}

fn valid_identifier(identifier: &str) -> bool {
    let mut bytes = identifier.bytes();
    matches!(bytes.next(), Some(b'A'..=b'Z' | b'a'..=b'z' | b'_'))
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::{
        F32ArithmeticOp, MslError, MslOptions, emit_storage_add, emit_storage_add_from_jir,
        emit_storage_binary_from_jir, emit_storage_binary_from_spirv_artifact,
        emit_storage_f32_add, emit_storage_f32_add_from_jir, emit_storage_f32_binary,
        emit_storage_f32_binary_from_jir, emit_storage_f32_binary_from_spirv_artifact,
        emit_storage_global_2d_strided_write, emit_storage_global_2d_strided_write_from_jir,
        emit_storage_global_2d_strided_write_from_spirv_artifact, emit_storage_global_2d_write,
        emit_storage_global_2d_write_from_jir, emit_storage_global_2d_write_from_spirv_artifact,
        emit_storage_global_3d_strided_write, emit_storage_global_3d_strided_write_from_jir,
        emit_storage_global_3d_strided_write_from_spirv_artifact, emit_storage_global_3d_write,
        emit_storage_global_3d_write_from_jir, emit_storage_global_3d_write_from_spirv_artifact,
        emit_storage_global_strided_write, emit_storage_global_strided_write_from_jir,
        emit_storage_global_strided_write_from_spirv_artifact, emit_storage_global_write,
        emit_storage_global_write_from_jir, emit_storage_global_write_from_spirv_artifact,
        emit_storage_vector_f32_add, emit_storage_vector_f32_add_from_jir,
        emit_storage_vector_f32_binary, emit_storage_vector_f32_binary_from_jir,
        emit_storage_vector_f32_binary_lanes, emit_storage_vector_f32_binary_lanes_from_jir,
        emit_storage_vector_f32_binary_lanes_from_spirv_artifact, msl_device_element_stride,
        msl_device_element_type, msl_device_resource_is_read_only, msl_device_resource_name,
        translate_spirv_artifact_to_msl, translate_spirv_artifact_to_msl_external,
        translate_spirv_artifact_to_msl_external_report, translate_spirv_to_msl,
        validate_external_msl_source, validate_storage_add, validate_storage_binary,
        validate_storage_binary_artifact, validate_storage_f32_add, validate_storage_f32_binary,
        validate_storage_f32_binary_artifact, validate_storage_global_2d_strided_write,
        validate_storage_global_2d_strided_write_artifact, validate_storage_global_2d_write,
        validate_storage_global_2d_write_artifact, validate_storage_global_3d_strided_write,
        validate_storage_global_3d_strided_write_artifact, validate_storage_global_3d_write,
        validate_storage_global_3d_write_artifact, validate_storage_global_strided_write,
        validate_storage_global_strided_write_artifact, validate_storage_global_write,
        validate_storage_global_write_artifact, validate_storage_vector_f32_add,
        validate_storage_vector_f32_binary, validate_storage_vector_f32_binary_lanes,
        validate_storage_vector_f32_binary_lanes_artifact,
    };
    use jadren_codegen_spirv::{
        ResourceAccess, ResourceElementType, SpirvOptions,
        emit_storage_global_2d_strided_write_artifact_from_jir,
        emit_storage_global_2d_write_artifact_from_jir,
        emit_storage_global_3d_strided_write_artifact_from_jir,
        emit_storage_global_3d_write_artifact_from_jir,
        emit_storage_global_index_binary_dynamic_length_artifact_from_jir,
        emit_storage_global_index_f32_binary_dynamic_length_artifact_from_jir,
        emit_storage_global_index_strided_write_artifact_from_jir,
        emit_storage_global_index_vector_f32_binary_dynamic_length_lanes_artifact_from_jir,
        emit_storage_global_index_write_artifact_from_jir,
    };
    use jadren_jir::{
        AddressSpace, BinaryOp, Block, BlockId, BuiltinOp, Constant, Function, FunctionId,
        Instruction, InstructionKind, Linkage, Module, Parameter, Terminator, Type, TypeId,
        TypedValue, ValueId,
    };

    fn jir_dynamic_storage_add_module() -> Module {
        Module {
            types: vec![
                Type::Unit,
                Type::Integer {
                    signed: false,
                    bits: 32,
                },
                Type::Pointer {
                    pointee: TypeId::new(1),
                    address_space: AddressSpace::Storage,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "add_u32".to_owned(),
                linkage: Linkage::Export,
                parameters: vec![
                    Parameter {
                        value: ValueId::new(0),
                        ty: TypeId::new(2),
                        name: Some("input".to_owned()),
                    },
                    Parameter {
                        value: ValueId::new(1),
                        ty: TypeId::new(2),
                        name: Some("output".to_owned()),
                    },
                    Parameter {
                        value: ValueId::new(2),
                        ty: TypeId::new(2),
                        name: Some("length".to_owned()),
                    },
                ],
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        Instruction {
                            result: Some(TypedValue {
                                value: ValueId::new(3),
                                ty: TypeId::new(1),
                            }),
                            kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                            span: None,
                        },
                        Instruction {
                            result: Some(TypedValue {
                                value: ValueId::new(4),
                                ty: TypeId::new(1),
                            }),
                            kind: InstructionKind::Constant(Constant::Integer { value: 1 }),
                            span: None,
                        },
                        Instruction {
                            result: Some(TypedValue {
                                value: ValueId::new(5),
                                ty: TypeId::new(1),
                            }),
                            kind: InstructionKind::Load {
                                pointer: ValueId::new(2),
                                alignment: 4,
                                volatile: false,
                            },
                            span: None,
                        },
                        Instruction {
                            result: None,
                            kind: InstructionKind::BoundsCheck {
                                index: ValueId::new(3),
                                length: ValueId::new(5),
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(TypedValue {
                                value: ValueId::new(6),
                                ty: TypeId::new(2),
                            }),
                            kind: InstructionKind::Offset {
                                base: ValueId::new(0),
                                indices: vec![ValueId::new(3)],
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(TypedValue {
                                value: ValueId::new(7),
                                ty: TypeId::new(1),
                            }),
                            kind: InstructionKind::Load {
                                pointer: ValueId::new(6),
                                alignment: 4,
                                volatile: false,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(TypedValue {
                                value: ValueId::new(8),
                                ty: TypeId::new(1),
                            }),
                            kind: InstructionKind::Binary {
                                op: BinaryOp::Add,
                                left: ValueId::new(7),
                                right: ValueId::new(4),
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(TypedValue {
                                value: ValueId::new(9),
                                ty: TypeId::new(2),
                            }),
                            kind: InstructionKind::Offset {
                                base: ValueId::new(1),
                                indices: vec![ValueId::new(3)],
                            },
                            span: None,
                        },
                        Instruction {
                            result: None,
                            kind: InstructionKind::Store {
                                pointer: ValueId::new(9),
                                value: ValueId::new(8),
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
        }
    }

    fn jir_global_strided_write_module() -> Module {
        let value = |id, ty| {
            Some(TypedValue {
                value: ValueId::new(id),
                ty: TypeId::new(ty),
            })
        };
        let load = |pointer| Instruction {
            result: value(pointer + 5, 1),
            kind: InstructionKind::Load {
                pointer: ValueId::new(pointer),
                alignment: 4,
                volatile: false,
            },
            span: None,
        };
        let mut module = jir_dynamic_storage_add_module();
        let function = &mut module.functions[0];
        function.name = "global_strided_write_u32".to_owned();
        function.parameters = (0..4)
            .map(|id| Parameter {
                value: ValueId::new(id),
                ty: TypeId::new(2),
                name: Some(["data", "length", "stride", "capacity"][id].to_owned()),
            })
            .collect();
        function.blocks[0].instructions = vec![
            Instruction {
                result: value(4, 1),
                kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                span: None,
            },
            Instruction {
                result: value(5, 1),
                kind: InstructionKind::Constant(Constant::Integer { value: 42 }),
                span: None,
            },
            load(1),
            load(2),
            load(3),
            Instruction {
                result: None,
                kind: InstructionKind::BoundsCheck {
                    index: ValueId::new(4),
                    length: ValueId::new(6),
                },
                span: None,
            },
            Instruction {
                result: value(9, 1),
                kind: InstructionKind::Binary {
                    op: BinaryOp::Multiply,
                    left: ValueId::new(4),
                    right: ValueId::new(7),
                },
                span: None,
            },
            Instruction {
                result: None,
                kind: InstructionKind::BoundsCheck {
                    index: ValueId::new(9),
                    length: ValueId::new(8),
                },
                span: None,
            },
            Instruction {
                result: value(10, 2),
                kind: InstructionKind::Offset {
                    base: ValueId::new(0),
                    indices: vec![ValueId::new(9)],
                },
                span: None,
            },
            Instruction {
                result: None,
                kind: InstructionKind::Store {
                    pointer: ValueId::new(10),
                    value: ValueId::new(5),
                    alignment: 4,
                    volatile: false,
                },
                span: None,
            },
        ];
        module
    }

    fn jir_global_2d_write_module() -> Module {
        let value = |id, ty| {
            Some(TypedValue {
                value: ValueId::new(id),
                ty: TypeId::new(ty),
            })
        };
        let load = |pointer, id| Instruction {
            result: value(id, 1),
            kind: InstructionKind::Load {
                pointer: ValueId::new(pointer),
                alignment: 4,
                volatile: false,
            },
            span: None,
        };
        let mut module = jir_dynamic_storage_add_module();
        let function = &mut module.functions[0];
        function.name = "global_2d_write_u32".to_owned();
        function.parameters = (0..4)
            .map(|id| Parameter {
                value: ValueId::new(id),
                ty: TypeId::new(2),
                name: Some(["data", "width", "height", "capacity"][id].to_owned()),
            })
            .collect();
        function.blocks[0].instructions = vec![
            Instruction {
                result: value(4, 1),
                kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                span: None,
            },
            Instruction {
                result: value(5, 1),
                kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdY),
                span: None,
            },
            Instruction {
                result: value(6, 1),
                kind: InstructionKind::Constant(Constant::Integer { value: 42 }),
                span: None,
            },
            load(1, 7),
            load(2, 8),
            load(3, 9),
            Instruction {
                result: None,
                kind: InstructionKind::BoundsCheck {
                    index: ValueId::new(4),
                    length: ValueId::new(7),
                },
                span: None,
            },
            Instruction {
                result: None,
                kind: InstructionKind::BoundsCheck {
                    index: ValueId::new(5),
                    length: ValueId::new(8),
                },
                span: None,
            },
            Instruction {
                result: value(10, 1),
                kind: InstructionKind::Binary {
                    op: BinaryOp::Multiply,
                    left: ValueId::new(5),
                    right: ValueId::new(7),
                },
                span: None,
            },
            Instruction {
                result: value(11, 1),
                kind: InstructionKind::Binary {
                    op: BinaryOp::Add,
                    left: ValueId::new(10),
                    right: ValueId::new(4),
                },
                span: None,
            },
            Instruction {
                result: None,
                kind: InstructionKind::BoundsCheck {
                    index: ValueId::new(11),
                    length: ValueId::new(9),
                },
                span: None,
            },
            Instruction {
                result: value(12, 2),
                kind: InstructionKind::Offset {
                    base: ValueId::new(0),
                    indices: vec![ValueId::new(11)],
                },
                span: None,
            },
            Instruction {
                result: None,
                kind: InstructionKind::Store {
                    pointer: ValueId::new(12),
                    value: ValueId::new(6),
                    alignment: 4,
                    volatile: false,
                },
                span: None,
            },
        ];
        module
    }

    fn jir_global_2d_strided_write_module() -> Module {
        let value = |id, ty| {
            Some(TypedValue {
                value: ValueId::new(id),
                ty: TypeId::new(ty),
            })
        };
        let load = |pointer, id| Instruction {
            result: value(id, 1),
            kind: InstructionKind::Load {
                pointer: ValueId::new(pointer),
                alignment: 4,
                volatile: false,
            },
            span: None,
        };
        let mut module = jir_dynamic_storage_add_module();
        let function = &mut module.functions[0];
        function.name = "global_2d_strided_write_u32".to_owned();
        function.parameters = (0..6)
            .map(|id| Parameter {
                value: ValueId::new(id),
                ty: TypeId::new(2),
                name: Some(
                    [
                        "data", "width", "height", "stride_x", "stride_y", "capacity",
                    ][id]
                        .to_owned(),
                ),
            })
            .collect();
        function.blocks[0].instructions = vec![
            Instruction {
                result: value(6, 1),
                kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                span: None,
            },
            Instruction {
                result: value(7, 1),
                kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdY),
                span: None,
            },
            Instruction {
                result: value(8, 1),
                kind: InstructionKind::Constant(Constant::Integer { value: 42 }),
                span: None,
            },
            load(1, 9),
            load(2, 10),
            load(3, 11),
            load(4, 12),
            load(5, 13),
            Instruction {
                result: None,
                kind: InstructionKind::BoundsCheck {
                    index: ValueId::new(6),
                    length: ValueId::new(9),
                },
                span: None,
            },
            Instruction {
                result: None,
                kind: InstructionKind::BoundsCheck {
                    index: ValueId::new(7),
                    length: ValueId::new(10),
                },
                span: None,
            },
            Instruction {
                result: value(14, 1),
                kind: InstructionKind::Binary {
                    op: BinaryOp::Multiply,
                    left: ValueId::new(6),
                    right: ValueId::new(11),
                },
                span: None,
            },
            Instruction {
                result: value(15, 1),
                kind: InstructionKind::Binary {
                    op: BinaryOp::Multiply,
                    left: ValueId::new(7),
                    right: ValueId::new(12),
                },
                span: None,
            },
            Instruction {
                result: value(16, 1),
                kind: InstructionKind::Binary {
                    op: BinaryOp::Add,
                    left: ValueId::new(14),
                    right: ValueId::new(15),
                },
                span: None,
            },
            Instruction {
                result: None,
                kind: InstructionKind::BoundsCheck {
                    index: ValueId::new(16),
                    length: ValueId::new(13),
                },
                span: None,
            },
            Instruction {
                result: value(17, 2),
                kind: InstructionKind::Offset {
                    base: ValueId::new(0),
                    indices: vec![ValueId::new(16)],
                },
                span: None,
            },
            Instruction {
                result: None,
                kind: InstructionKind::Store {
                    pointer: ValueId::new(17),
                    value: ValueId::new(8),
                    alignment: 4,
                    volatile: false,
                },
                span: None,
            },
        ];
        module
    }

    fn jir_global_3d_write_module() -> Module {
        let value = |id, ty| {
            Some(TypedValue {
                value: ValueId::new(id),
                ty: TypeId::new(ty),
            })
        };
        let load = |pointer, id| Instruction {
            result: value(id, 1),
            kind: InstructionKind::Load {
                pointer: ValueId::new(pointer),
                alignment: 4,
                volatile: false,
            },
            span: None,
        };
        let mut module = jir_dynamic_storage_add_module();
        let function = &mut module.functions[0];
        function.name = "global_3d_write_u32".to_owned();
        function.parameters = (0..5)
            .map(|id| Parameter {
                value: ValueId::new(id),
                ty: TypeId::new(2),
                name: Some(["data", "width", "height", "depth", "capacity"][id].to_owned()),
            })
            .collect();
        function.blocks[0].instructions = vec![
            Instruction {
                result: value(5, 1),
                kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                span: None,
            },
            Instruction {
                result: value(6, 1),
                kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdY),
                span: None,
            },
            Instruction {
                result: value(7, 1),
                kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdZ),
                span: None,
            },
            Instruction {
                result: value(8, 1),
                kind: InstructionKind::Constant(Constant::Integer { value: 42 }),
                span: None,
            },
            load(1, 9),
            load(2, 10),
            load(3, 11),
            load(4, 12),
            Instruction {
                result: None,
                kind: InstructionKind::BoundsCheck {
                    index: ValueId::new(5),
                    length: ValueId::new(9),
                },
                span: None,
            },
            Instruction {
                result: None,
                kind: InstructionKind::BoundsCheck {
                    index: ValueId::new(6),
                    length: ValueId::new(10),
                },
                span: None,
            },
            Instruction {
                result: None,
                kind: InstructionKind::BoundsCheck {
                    index: ValueId::new(7),
                    length: ValueId::new(11),
                },
                span: None,
            },
            Instruction {
                result: value(13, 1),
                kind: InstructionKind::Binary {
                    op: BinaryOp::Multiply,
                    left: ValueId::new(7),
                    right: ValueId::new(10),
                },
                span: None,
            },
            Instruction {
                result: value(14, 1),
                kind: InstructionKind::Binary {
                    op: BinaryOp::Add,
                    left: ValueId::new(13),
                    right: ValueId::new(6),
                },
                span: None,
            },
            Instruction {
                result: value(15, 1),
                kind: InstructionKind::Binary {
                    op: BinaryOp::Multiply,
                    left: ValueId::new(14),
                    right: ValueId::new(9),
                },
                span: None,
            },
            Instruction {
                result: value(16, 1),
                kind: InstructionKind::Binary {
                    op: BinaryOp::Add,
                    left: ValueId::new(15),
                    right: ValueId::new(5),
                },
                span: None,
            },
            Instruction {
                result: None,
                kind: InstructionKind::BoundsCheck {
                    index: ValueId::new(16),
                    length: ValueId::new(12),
                },
                span: None,
            },
            Instruction {
                result: value(17, 2),
                kind: InstructionKind::Offset {
                    base: ValueId::new(0),
                    indices: vec![ValueId::new(16)],
                },
                span: None,
            },
            Instruction {
                result: None,
                kind: InstructionKind::Store {
                    pointer: ValueId::new(17),
                    value: ValueId::new(8),
                    alignment: 4,
                    volatile: false,
                },
                span: None,
            },
        ];
        module
    }

    fn jir_global_3d_strided_write_module() -> Module {
        let value = |id, ty| {
            Some(TypedValue {
                value: ValueId::new(id),
                ty: TypeId::new(ty),
            })
        };
        let load = |pointer, id| Instruction {
            result: value(id, 1),
            kind: InstructionKind::Load {
                pointer: ValueId::new(pointer),
                alignment: 4,
                volatile: false,
            },
            span: None,
        };
        let mut module = jir_dynamic_storage_add_module();
        let function = &mut module.functions[0];
        function.name = "global_3d_strided_write_u32".to_owned();
        function.parameters = (0..8)
            .map(|id| Parameter {
                value: ValueId::new(id),
                ty: TypeId::new(2),
                name: Some(
                    [
                        "data", "width", "height", "depth", "stride_x", "stride_y", "stride_z",
                        "capacity",
                    ][id]
                        .to_owned(),
                ),
            })
            .collect();
        function.blocks[0].instructions = vec![
            Instruction {
                result: value(8, 1),
                kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                span: None,
            },
            Instruction {
                result: value(9, 1),
                kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdY),
                span: None,
            },
            Instruction {
                result: value(10, 1),
                kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdZ),
                span: None,
            },
            Instruction {
                result: value(11, 1),
                kind: InstructionKind::Constant(Constant::Integer { value: 42 }),
                span: None,
            },
            load(1, 12),
            load(2, 13),
            load(3, 14),
            load(4, 15),
            load(5, 16),
            load(6, 17),
            load(7, 18),
            Instruction {
                result: None,
                kind: InstructionKind::BoundsCheck {
                    index: ValueId::new(8),
                    length: ValueId::new(12),
                },
                span: None,
            },
            Instruction {
                result: None,
                kind: InstructionKind::BoundsCheck {
                    index: ValueId::new(9),
                    length: ValueId::new(13),
                },
                span: None,
            },
            Instruction {
                result: None,
                kind: InstructionKind::BoundsCheck {
                    index: ValueId::new(10),
                    length: ValueId::new(14),
                },
                span: None,
            },
            Instruction {
                result: value(19, 1),
                kind: InstructionKind::Binary {
                    op: BinaryOp::Multiply,
                    left: ValueId::new(8),
                    right: ValueId::new(15),
                },
                span: None,
            },
            Instruction {
                result: value(20, 1),
                kind: InstructionKind::Binary {
                    op: BinaryOp::Multiply,
                    left: ValueId::new(9),
                    right: ValueId::new(16),
                },
                span: None,
            },
            Instruction {
                result: value(21, 1),
                kind: InstructionKind::Binary {
                    op: BinaryOp::Add,
                    left: ValueId::new(19),
                    right: ValueId::new(20),
                },
                span: None,
            },
            Instruction {
                result: value(22, 1),
                kind: InstructionKind::Binary {
                    op: BinaryOp::Multiply,
                    left: ValueId::new(10),
                    right: ValueId::new(17),
                },
                span: None,
            },
            Instruction {
                result: value(23, 1),
                kind: InstructionKind::Binary {
                    op: BinaryOp::Add,
                    left: ValueId::new(21),
                    right: ValueId::new(22),
                },
                span: None,
            },
            Instruction {
                result: None,
                kind: InstructionKind::BoundsCheck {
                    index: ValueId::new(23),
                    length: ValueId::new(18),
                },
                span: None,
            },
            Instruction {
                result: value(24, 2),
                kind: InstructionKind::Offset {
                    base: ValueId::new(0),
                    indices: vec![ValueId::new(23)],
                },
                span: None,
            },
            Instruction {
                result: None,
                kind: InstructionKind::Store {
                    pointer: ValueId::new(24),
                    value: ValueId::new(11),
                    alignment: 4,
                    volatile: false,
                },
                span: None,
            },
        ];
        module
    }

    fn jir_dynamic_storage_f32_add_module() -> Module {
        jir_dynamic_storage_f32_binary_module(F32ArithmeticOp::Add, "add_f32", 0x3f80_0000)
    }

    fn jir_dynamic_storage_f32_binary_module(
        operation: F32ArithmeticOp,
        name: &str,
        operand_bits: u32,
    ) -> Module {
        let mut module = jir_dynamic_storage_add_module();
        let f32_type = TypeId::new(module.types.len());
        module.types.push(Type::Float { bits: 32 });
        let f32_pointer_type = TypeId::new(module.types.len());
        module.types.push(Type::Pointer {
            pointee: f32_type,
            address_space: AddressSpace::Storage,
        });
        module.functions[0].parameters[0].ty = f32_pointer_type;
        module.functions[0].parameters[1].ty = f32_pointer_type;
        let instructions = &mut module.functions[0].blocks[0].instructions;
        instructions[1].result.as_mut().expect("constant result").ty = f32_type;
        instructions[1].kind = InstructionKind::Constant(Constant::FloatBits {
            bits: u64::from(operand_bits),
        });
        instructions[4].result.as_mut().expect("input offset").ty = f32_pointer_type;
        instructions[5].result.as_mut().expect("input result").ty = f32_type;
        instructions[6].result.as_mut().expect("binary result").ty = f32_type;
        if let InstructionKind::Binary { op, .. } = &mut instructions[6].kind {
            *op = operation.as_binary_op();
        }
        instructions[7].result.as_mut().expect("output offset").ty = f32_pointer_type;
        module.functions[0].name = name.to_owned();
        module
    }

    fn jir_dynamic_storage_vector_f32_add_module() -> Module {
        let instruction = |result, kind| Instruction {
            result,
            kind,
            span: None,
        };
        Module {
            types: vec![
                Type::Unit,
                Type::Integer {
                    signed: false,
                    bits: 32,
                },
                Type::Float { bits: 32 },
                Type::Vector {
                    element: TypeId::new(2),
                    lanes: 4,
                },
                Type::Pointer {
                    pointee: TypeId::new(3),
                    address_space: AddressSpace::Storage,
                },
                Type::Pointer {
                    pointee: TypeId::new(1),
                    address_space: AddressSpace::Storage,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "add_f32x4".to_owned(),
                linkage: Linkage::Export,
                parameters: vec![
                    Parameter {
                        value: ValueId::new(0),
                        ty: TypeId::new(4),
                        name: Some("input".to_owned()),
                    },
                    Parameter {
                        value: ValueId::new(1),
                        ty: TypeId::new(4),
                        name: Some("output".to_owned()),
                    },
                    Parameter {
                        value: ValueId::new(2),
                        ty: TypeId::new(5),
                        name: Some("length".to_owned()),
                    },
                ],
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(3),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(4),
                                ty: TypeId::new(2),
                            }),
                            InstructionKind::Constant(Constant::FloatBits { bits: 0x3f80_0000 }),
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(5),
                                ty: TypeId::new(3),
                            }),
                            InstructionKind::VectorSplat {
                                value: ValueId::new(4),
                                lanes: 4,
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(6),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Load {
                                pointer: ValueId::new(2),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(3),
                                length: ValueId::new(6),
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(7),
                                ty: TypeId::new(4),
                            }),
                            InstructionKind::Offset {
                                base: ValueId::new(0),
                                indices: vec![ValueId::new(3)],
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(8),
                                ty: TypeId::new(3),
                            }),
                            InstructionKind::Load {
                                pointer: ValueId::new(7),
                                alignment: 16,
                                volatile: false,
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(9),
                                ty: TypeId::new(3),
                            }),
                            InstructionKind::VectorBinary {
                                op: BinaryOp::Add,
                                left: ValueId::new(8),
                                right: ValueId::new(5),
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(10),
                                ty: TypeId::new(4),
                            }),
                            InstructionKind::Offset {
                                base: ValueId::new(1),
                                indices: vec![ValueId::new(3)],
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::Store {
                                pointer: ValueId::new(10),
                                value: ValueId::new(9),
                                alignment: 16,
                                volatile: false,
                            },
                        ),
                    ],
                    terminator: Terminator::Return { value: None },
                    span: None,
                }],
                span: None,
            }],
        }
    }

    #[test]
    fn emits_deterministic_bounded_kernel_contract() {
        let options = MslOptions::new([64, 1, 1]).expect("valid workgroup");
        let first = emit_storage_add("jadren_add_one", options, 1).expect("valid kernel");
        let second = emit_storage_add("jadren_add_one", options, 1).expect("valid kernel");
        assert_eq!(first, second);
        validate_storage_add(&first, "jadren_add_one").expect("contract is valid");
        assert!(first.contains("max_total_threads_per_threadgroup(64)"));
        assert!(first.contains("output[gid.x] = input[gid.x] + 1u;"));
    }

    #[test]
    fn emits_deterministic_bounded_f32_kernel_contract() {
        let options = MslOptions::new([64, 1, 1]).expect("valid workgroup");
        let first =
            emit_storage_f32_add("add_f32", options, 0x3f80_0000).expect("valid f32 kernel");
        let second =
            emit_storage_f32_add("add_f32", options, 0x3f80_0000).expect("valid f32 kernel");
        assert_eq!(first, second);
        validate_storage_f32_add(&first, "add_f32").expect("f32 contract is valid");
        assert!(first.contains("device const float* input [[buffer(0)]]"));
        assert!(first.contains("output[gid.x] = input[gid.x] + as_type<float>(1065353216u);"));
    }

    #[test]
    fn emits_bounded_f32_binary_family_contracts() {
        let options = MslOptions::new([64, 1, 1]).expect("valid workgroup");
        for (operation, entry_name, operand_bits, expression) in [
            (
                F32ArithmeticOp::Add,
                "add_f32",
                0x3f80_0000,
                "output[gid.x] = input[gid.x] + as_type<float>(1065353216u);",
            ),
            (
                F32ArithmeticOp::Subtract,
                "subtract_f32",
                0x3f80_0000,
                "output[gid.x] = input[gid.x] - as_type<float>(1065353216u);",
            ),
            (
                F32ArithmeticOp::Multiply,
                "multiply_f32",
                0x4000_0000,
                "output[gid.x] = input[gid.x] * as_type<float>(1073741824u);",
            ),
        ] {
            let source = emit_storage_f32_binary(entry_name, options, operand_bits, operation)
                .expect("valid f32 binary kernel");
            validate_storage_f32_binary(&source, entry_name, operation)
                .expect("f32 binary contract is valid");
            assert!(source.contains(expression));
        }
    }

    #[test]
    fn emits_bounded_vector_f32_contract() {
        let options = MslOptions::new([64, 1, 1]).expect("valid workgroup");
        let source = emit_storage_vector_f32_add("add_f32x4", options, 0x3f80_0000)
            .expect("valid vector f32 kernel");
        validate_storage_vector_f32_add(&source, "add_f32x4")
            .expect("vector f32 contract is valid");
        assert!(source.contains("device const float4* input [[buffer(0)]]"));
        assert!(
            source.contains("output[gid.x] = input[gid.x] + float4(as_type<float>(1065353216u));")
        );
    }

    #[test]
    fn emits_bounded_vector_f32_binary_family_contracts() {
        let options = MslOptions::new([64, 1, 1]).expect("valid workgroup");
        for (operation, entry_name, operand_bits, expression) in [
            (
                F32ArithmeticOp::Add,
                "add_f32x4",
                0x3f80_0000,
                "output[gid.x] = input[gid.x] + float4(as_type<float>(1065353216u));",
            ),
            (
                F32ArithmeticOp::Subtract,
                "subtract_f32x4",
                0x3f80_0000,
                "output[gid.x] = input[gid.x] - float4(as_type<float>(1065353216u));",
            ),
            (
                F32ArithmeticOp::Multiply,
                "multiply_f32x4",
                0x4000_0000,
                "output[gid.x] = input[gid.x] * float4(as_type<float>(1073741824u));",
            ),
        ] {
            let source =
                emit_storage_vector_f32_binary(entry_name, options, operand_bits, operation)
                    .expect("valid vector f32 binary kernel");
            validate_storage_vector_f32_binary(&source, entry_name, operation)
                .expect("vector f32 binary contract is valid");
            assert!(source.contains(expression));
            if operation == F32ArithmeticOp::Subtract {
                assert!(
                    validate_storage_vector_f32_binary(&source, entry_name, F32ArithmeticOp::Add)
                        .is_err()
                );
            }
        }
    }

    #[test]
    fn emits_bounded_vector_f32_lane_contracts_for_x2_and_x3() {
        let options = MslOptions::new([64, 1, 1]).expect("valid workgroup");
        for lanes in [2_u16, 3_u16] {
            let entry_name = format!("add_f32x{lanes}");
            let source = emit_storage_vector_f32_binary_lanes(
                &entry_name,
                options,
                0x3f80_0000,
                F32ArithmeticOp::Add,
                lanes,
            )
            .expect("valid vector lane source");
            validate_storage_vector_f32_binary_lanes(
                &source,
                &entry_name,
                F32ArithmeticOp::Add,
                lanes,
            )
            .expect("vector lane source contract is valid");
            assert!(source.contains(&format!("device const float{lanes}* input [[buffer(0)]]")));
            assert!(source.contains(&format!(
                "output[gid.x] = input[gid.x] + float{lanes}(as_type<float>(1065353216u));"
            )));

            let mut module = jir_dynamic_storage_vector_f32_add_module();
            module.types[3] = Type::Vector {
                element: TypeId::new(2),
                lanes,
            };
            module.functions[0].name = entry_name.clone();
            if let InstructionKind::VectorSplat {
                lanes: vector_lanes,
                ..
            } = &mut module.functions[0].blocks[0].instructions[2].kind
            {
                *vector_lanes = lanes;
            }
            let source = emit_storage_vector_f32_binary_lanes_from_jir(
                &module,
                FunctionId::new(0),
                options,
                F32ArithmeticOp::Add,
                lanes,
            )
            .expect("vector lane JIR source is supported");
            validate_storage_vector_f32_binary_lanes(
                &source,
                &entry_name,
                F32ArithmeticOp::Add,
                lanes,
            )
            .expect("vector lane JIR source contract is valid");
        }
    }

    #[test]
    fn lowers_validated_scalar_f32_spirv_artifacts_to_msl() {
        for (operation, name, operand_bits) in [
            (F32ArithmeticOp::Add, "add_f32", 0x3f80_0000),
            (F32ArithmeticOp::Subtract, "subtract_f32", 0x3f80_0000),
            (F32ArithmeticOp::Multiply, "multiply_f32", 0x4000_0000),
        ] {
            let module = jir_dynamic_storage_f32_binary_module(operation, name, operand_bits);
            let artifact = emit_storage_global_index_f32_binary_dynamic_length_artifact_from_jir(
                &module,
                FunctionId::new(0),
                SpirvOptions::new([64, 1, 1]).expect("valid SPIR-V workgroup"),
                operation,
            )
            .expect("scalar f32 artifact is supported");
            validate_storage_f32_binary_artifact(&artifact, operand_bits, operation)
                .expect("scalar f32 artifact contract is valid");
            let source = emit_storage_f32_binary_from_spirv_artifact(
                &artifact,
                MslOptions::new([64, 1, 1]).expect("valid MSL workgroup"),
                operand_bits,
                operation,
            )
            .expect("scalar f32 artifact lowers to MSL");
            validate_storage_f32_binary(&source, name, operation)
                .expect("translated scalar MSL contract is valid");
        }
    }

    #[test]
    fn rejects_scalar_f32_spirv_artifact_stride_and_operation_mismatch() {
        let module = jir_dynamic_storage_f32_add_module();
        let mut artifact = emit_storage_global_index_f32_binary_dynamic_length_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([64, 1, 1]).expect("valid SPIR-V workgroup"),
            F32ArithmeticOp::Add,
        )
        .expect("scalar f32 artifact is supported");
        artifact.resources[0].element_stride = Some(16);
        assert!(
            validate_storage_f32_binary_artifact(&artifact, 0x3f80_0000, F32ArithmeticOp::Add,)
                .is_err()
        );
        artifact.resources[0].element_stride = Some(4);
        assert!(validate_storage_f32_binary_artifact(
            &artifact,
            0x3f80_0000,
            F32ArithmeticOp::Subtract,
        )
        .is_err());
    }

    #[test]
    fn lowers_validated_u32_spirv_artifacts_to_msl_operation_family() {
        let options = MslOptions::new([64, 1, 1]).expect("valid MSL workgroup");
        let spirv_options = SpirvOptions::new([64, 1, 1]).expect("valid SPIR-V workgroup");
        for (operation, operand, expression) in [
            (BinaryOp::Add, 1_u32, "+ 1u;"),
            (BinaryOp::Subtract, 1_u32, "- 1u;"),
            (BinaryOp::Multiply, 1_u32, "* 1u;"),
            (BinaryOp::Divide, 1_u32, "/ 1u;"),
            (BinaryOp::Remainder, 1_u32, "% 1u;"),
            (BinaryOp::BitAnd, 1_u32, "& 1u;"),
            (BinaryOp::BitOr, 1_u32, "| 1u;"),
            (BinaryOp::BitXor, 1_u32, "^ 1u;"),
            (BinaryOp::ShiftLeft, 1_u32, "<< 1u;"),
            (BinaryOp::ShiftRight, 1_u32, ">> 1u;"),
        ] {
            let mut module = jir_dynamic_storage_add_module();
            let name = format!("binary_u32_{operation:?}").to_lowercase();
            module.functions[0].name = name.clone();
            module.functions[0].blocks[0].instructions[6].kind = InstructionKind::Binary {
                op: operation,
                left: ValueId::new(7),
                right: ValueId::new(4),
            };
            let artifact = emit_storage_global_index_binary_dynamic_length_artifact_from_jir(
                &module,
                FunctionId::new(0),
                spirv_options,
                operation,
            )
            .expect("u32 artifact is supported");
            validate_storage_binary_artifact(&artifact, operation, operand)
                .expect("u32 artifact contract is valid");
            let source =
                emit_storage_binary_from_spirv_artifact(&artifact, options, operation, operand)
                    .expect("u32 artifact lowers to MSL");
            validate_storage_binary(&source, &name, operation, operand)
                .expect("translated u32 MSL contract is valid");
            assert!(source.contains(expression));
        }
    }

    #[test]
    fn rejects_u32_spirv_artifact_stride_and_operation_mismatch() {
        let module = jir_dynamic_storage_add_module();
        let mut artifact = emit_storage_global_index_binary_dynamic_length_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([64, 1, 1]).expect("valid SPIR-V workgroup"),
            BinaryOp::Add,
        )
        .expect("u32 artifact is supported");
        artifact.resources[1].element_stride = Some(8);
        assert!(validate_storage_binary_artifact(&artifact, BinaryOp::Add, 1).is_err());
        artifact.resources[1].element_stride = Some(4);
        assert!(validate_storage_binary_artifact(&artifact, BinaryOp::Multiply, 1).is_err());
    }

    #[test]
    fn lowers_validated_vector_spirv_artifacts_to_msl_for_x2_x3_and_x4() {
        for lanes in [2_u16, 3, 4] {
            let mut module = jir_dynamic_storage_vector_f32_add_module();
            module.types[3] = Type::Vector {
                element: TypeId::new(2),
                lanes,
            };
            let entry_name = format!("add_f32x{lanes}");
            module.functions[0].name = entry_name.clone();
            if let InstructionKind::VectorSplat {
                lanes: vector_lanes,
                ..
            } = &mut module.functions[0].blocks[0].instructions[2].kind
            {
                *vector_lanes = lanes;
            }
            let artifact =
                emit_storage_global_index_vector_f32_binary_dynamic_length_lanes_artifact_from_jir(
                    &module,
                    FunctionId::new(0),
                    SpirvOptions::new([64, 1, 1]).expect("valid SPIR-V workgroup"),
                    F32ArithmeticOp::Add,
                    u32::from(lanes),
                )
                .expect("vector artifact is supported");
            validate_storage_vector_f32_binary_lanes_artifact(
                &artifact,
                0x3f80_0000,
                F32ArithmeticOp::Add,
                lanes,
            )
            .expect("vector artifact contract is valid");
            let source = emit_storage_vector_f32_binary_lanes_from_spirv_artifact(
                &artifact,
                MslOptions::new([64, 1, 1]).expect("valid MSL workgroup"),
                0x3f80_0000,
                F32ArithmeticOp::Add,
                lanes,
            )
            .expect("vector artifact lowers to MSL");
            validate_storage_vector_f32_binary_lanes(
                &source,
                &entry_name,
                F32ArithmeticOp::Add,
                lanes,
            )
            .expect("translated MSL source contract is valid");
            assert!(source.contains(&format!("device const float{lanes}* input")));
        }
    }

    #[test]
    fn rejects_vector_spirv_artifact_stride_and_operation_mismatch() {
        let mut module = jir_dynamic_storage_vector_f32_add_module();
        module.types[3] = Type::Vector {
            element: TypeId::new(2),
            lanes: 2,
        };
        if let InstructionKind::VectorSplat {
            lanes: vector_lanes,
            ..
        } = &mut module.functions[0].blocks[0].instructions[2].kind
        {
            *vector_lanes = 2;
        }
        let mut artifact =
            emit_storage_global_index_vector_f32_binary_dynamic_length_lanes_artifact_from_jir(
                &module,
                FunctionId::new(0),
                SpirvOptions::new([64, 1, 1]).expect("valid SPIR-V workgroup"),
                F32ArithmeticOp::Add,
                2,
            )
            .expect("vector artifact is supported");
        artifact.resources[0].element_stride = Some(16);
        assert!(
            validate_storage_vector_f32_binary_lanes_artifact(
                &artifact,
                0x3f80_0000,
                F32ArithmeticOp::Add,
                2,
            )
            .is_err()
        );
        artifact.resources[0].element_stride = Some(8);
        assert!(
            validate_storage_vector_f32_binary_lanes_artifact(
                &artifact,
                0x3f80_0000,
                F32ArithmeticOp::Subtract,
                2,
            )
            .is_err()
        );
    }

    #[test]
    fn lowers_verified_jir_vector_f32_contract() {
        let module = jir_dynamic_storage_vector_f32_add_module();
        let source = emit_storage_vector_f32_add_from_jir(
            &module,
            FunctionId::new(0),
            MslOptions::new([64, 1, 1]).unwrap(),
        )
        .expect("vector f32 JIR shape is supported");
        validate_storage_vector_f32_add(&source, "add_f32x4")
            .expect("vector f32 JIR contract is valid");
    }

    #[test]
    fn lowers_verified_jir_vector_f32_operation_family() {
        let module = jir_dynamic_storage_vector_f32_add_module();
        for operation in [
            F32ArithmeticOp::Add,
            F32ArithmeticOp::Subtract,
            F32ArithmeticOp::Multiply,
        ] {
            let mut module = module.clone();
            let entry_name = format!(
                "{}_f32x4",
                match operation {
                    F32ArithmeticOp::Add => "add",
                    F32ArithmeticOp::Subtract => "subtract",
                    F32ArithmeticOp::Multiply => "multiply",
                }
            );
            let function = &mut module.functions[0];
            function.name = entry_name.clone();
            if let InstructionKind::VectorBinary { op, .. } =
                &mut function.blocks[0].instructions[7].kind
            {
                *op = operation.as_binary_op();
            }
            let source = emit_storage_vector_f32_binary_from_jir(
                &module,
                FunctionId::new(0),
                MslOptions::new([64, 1, 1]).unwrap(),
                operation,
            )
            .expect("vector f32 operation JIR shape is supported");
            validate_storage_vector_f32_binary(&source, &entry_name, operation)
                .expect("vector f32 operation JIR contract is valid");
        }
    }

    #[test]
    fn emits_bounds_safe_global_write_contract() {
        let options = MslOptions::new([64, 1, 1]).expect("valid workgroup");
        let source = emit_storage_global_write("global_write_u32", options, 42, 70)
            .expect("valid global write");
        validate_storage_global_write(&source, "global_write_u32")
            .expect("global write contract is valid");
        assert!(source.contains("if (gid.x < 70u)"));
        assert!(source.contains("data[gid.x] = 42u;"));
        assert_eq!(
            emit_storage_global_write("global_write_u32", options, 42, 0),
            Err(MslError::UnsupportedJirShape(
                "global write length must be positive"
            ))
        );
    }

    #[test]
    fn emits_bounds_safe_global_strided_write_contract() {
        let source = emit_storage_global_strided_write(
            "global_strided_write_u32",
            MslOptions::new([64, 1, 1]).unwrap(),
            42,
        )
        .expect("valid global strided write");
        validate_storage_global_strided_write(&source, "global_strided_write_u32")
            .expect("global strided write contract is valid");
        assert!(source.contains("uint physical = gid.x * stride[0];"));
        assert!(source.contains("data[physical] = 42u;"));
    }

    #[test]
    fn emits_bounds_safe_global_2d_write_contract() {
        let source = emit_storage_global_2d_write(
            "global_2d_write_u32",
            MslOptions::new([8, 8, 1]).unwrap(),
            42,
        )
        .expect("valid global 2D write");
        validate_storage_global_2d_write(&source, "global_2d_write_u32")
            .expect("global 2D write contract is valid");
        assert!(source.contains("uint physical = gid.y * width[0] + gid.x;"));
        assert!(source.contains("data[physical] = 42u;"));
    }

    #[test]
    fn emits_bounds_safe_global_2d_strided_write_contract() {
        let source = emit_storage_global_2d_strided_write(
            "global_2d_strided_write_u32",
            MslOptions::new([4, 4, 1]).unwrap(),
            42,
        )
        .expect("valid global 2D strided write");
        validate_storage_global_2d_strided_write(&source, "global_2d_strided_write_u32")
            .expect("global 2D strided write contract is valid");
        assert!(source.contains("gid.x * stride_x[0] + gid.y * stride_y[0]"));
    }

    #[test]
    fn emits_bounds_safe_global_3d_write_contract() {
        let source = emit_storage_global_3d_write(
            "global_3d_write_u32",
            MslOptions::new([4, 4, 2]).unwrap(),
            42,
        )
        .expect("valid global 3D write");
        validate_storage_global_3d_write(&source, "global_3d_write_u32")
            .expect("global 3D write contract is valid");
        assert!(source.contains("(gid.z * height[0] + gid.y) * width[0] + gid.x"));
    }

    #[test]
    fn emits_bounds_safe_global_3d_strided_write_contract() {
        let source = emit_storage_global_3d_strided_write(
            "global_3d_strided_write_u32",
            MslOptions::new([4, 4, 2]).unwrap(),
            42,
        )
        .expect("valid global 3D strided write");
        validate_storage_global_3d_strided_write(&source, "global_3d_strided_write_u32")
            .expect("global 3D strided write contract is valid");
        assert!(source.contains("gid.x * stride_x[0] + gid.y * stride_y[0] + gid.z * stride_z[0]"));
    }

    #[test]
    fn lowers_verified_jir_global_2d_strided_write() {
        let instruction = |result, kind| Instruction {
            result,
            kind,
            span: None,
        };
        let value = |id, ty| {
            Some(TypedValue {
                value: ValueId::new(id),
                ty: TypeId::new(ty),
            })
        };
        let parameter = |id, name: &str| Parameter {
            value: ValueId::new(id),
            ty: TypeId::new(2),
            name: Some(name.to_owned()),
        };
        let load = |pointer| InstructionKind::Load {
            pointer: ValueId::new(pointer),
            alignment: 4,
            volatile: false,
        };
        let module = Module {
            types: vec![
                Type::Unit,
                Type::Integer {
                    signed: false,
                    bits: 32,
                },
                Type::Pointer {
                    pointee: TypeId::new(1),
                    address_space: AddressSpace::Storage,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "global_2d_strided_write_u32".to_owned(),
                linkage: Linkage::Export,
                parameters: vec![
                    parameter(0, "data"),
                    parameter(1, "width"),
                    parameter(2, "height"),
                    parameter(3, "stride_x"),
                    parameter(4, "stride_y"),
                    parameter(5, "capacity"),
                ],
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        instruction(
                            value(6, 1),
                            InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                        ),
                        instruction(
                            value(7, 1),
                            InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdY),
                        ),
                        instruction(
                            value(8, 1),
                            InstructionKind::Constant(Constant::Integer { value: 42 }),
                        ),
                        instruction(value(9, 1), load(1)),
                        instruction(value(10, 1), load(2)),
                        instruction(value(11, 1), load(3)),
                        instruction(value(12, 1), load(4)),
                        instruction(value(13, 1), load(5)),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(6),
                                length: ValueId::new(9),
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(7),
                                length: ValueId::new(10),
                            },
                        ),
                        instruction(
                            value(14, 1),
                            InstructionKind::Binary {
                                op: BinaryOp::Multiply,
                                left: ValueId::new(6),
                                right: ValueId::new(11),
                            },
                        ),
                        instruction(
                            value(15, 1),
                            InstructionKind::Binary {
                                op: BinaryOp::Multiply,
                                left: ValueId::new(7),
                                right: ValueId::new(12),
                            },
                        ),
                        instruction(
                            value(16, 1),
                            InstructionKind::Binary {
                                op: BinaryOp::Add,
                                left: ValueId::new(14),
                                right: ValueId::new(15),
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(16),
                                length: ValueId::new(13),
                            },
                        ),
                        instruction(
                            value(17, 2),
                            InstructionKind::Offset {
                                base: ValueId::new(0),
                                indices: vec![ValueId::new(16)],
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::Store {
                                pointer: ValueId::new(17),
                                value: ValueId::new(8),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                    ],
                    terminator: Terminator::Return { value: None },
                    span: None,
                }],
                span: None,
            }],
        };
        let source = emit_storage_global_2d_strided_write_from_jir(
            &module,
            FunctionId::new(0),
            MslOptions::new([4, 4, 1]).unwrap(),
        )
        .expect("global 2D strided JIR shape is supported");
        validate_storage_global_2d_strided_write(&source, "global_2d_strided_write_u32")
            .expect("global 2D strided MSL contract is valid");
        assert!(source.contains("data[physical] = 42u;"));
    }

    #[test]
    fn lowers_validated_global_2d_strided_write_spirv_artifact_to_msl() {
        let module = jir_global_2d_strided_write_module();
        let artifact = emit_storage_global_2d_strided_write_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([4, 4, 1]).expect("valid SPIR-V workgroup"),
        )
        .expect("2D strided artifact is supported");
        validate_storage_global_2d_strided_write_artifact(&artifact, 42)
            .expect("2D strided artifact contract is valid");
        let source = emit_storage_global_2d_strided_write_from_spirv_artifact(
            &artifact,
            MslOptions::new([4, 4, 1]).expect("valid MSL workgroup"),
            42,
        )
        .expect("2D strided artifact lowers to MSL");
        validate_storage_global_2d_strided_write(&source, "global_2d_strided_write_u32")
            .expect("translated 2D strided MSL contract is valid");
        assert!(source.contains("gid.x * stride_x[0] + gid.y * stride_y[0]"));
        assert!(source.contains("data[physical] = 42u;"));
    }

    #[test]
    fn rejects_global_2d_strided_write_spirv_artifact_contract_mismatch() {
        let module = jir_global_2d_strided_write_module();
        let mut artifact = emit_storage_global_2d_strided_write_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([4, 4, 1]).expect("valid SPIR-V workgroup"),
        )
        .expect("2D strided artifact is supported");
        artifact.resources[3].element_stride = Some(8);
        assert!(validate_storage_global_2d_strided_write_artifact(&artifact, 42).is_err());
        artifact.resources[3].element_stride = Some(4);
        assert!(validate_storage_global_2d_strided_write_artifact(&artifact, 43).is_err());
        assert!(
            emit_storage_global_2d_strided_write_from_spirv_artifact(
                &artifact,
                MslOptions::new([2, 4, 1]).expect("valid MSL workgroup"),
                42,
            )
            .is_err()
        );
    }

    #[test]
    fn lowers_validated_global_3d_write_spirv_artifact_to_msl() {
        let module = jir_global_3d_write_module();
        let artifact = emit_storage_global_3d_write_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([4, 4, 2]).expect("valid SPIR-V workgroup"),
        )
        .expect("3D artifact is supported");
        validate_storage_global_3d_write_artifact(&artifact, 42)
            .expect("3D artifact contract is valid");
        let source = emit_storage_global_3d_write_from_spirv_artifact(
            &artifact,
            MslOptions::new([4, 4, 2]).expect("valid MSL workgroup"),
            42,
        )
        .expect("3D artifact lowers to MSL");
        validate_storage_global_3d_write(&source, "global_3d_write_u32")
            .expect("translated 3D MSL contract is valid");
        assert!(source.contains("gid.z < depth[0]"));
        assert!(source.contains("data[physical] = 42u;"));
    }

    #[test]
    fn rejects_global_3d_write_spirv_artifact_contract_mismatch() {
        let module = jir_global_3d_write_module();
        let mut artifact = emit_storage_global_3d_write_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([4, 4, 2]).expect("valid SPIR-V workgroup"),
        )
        .expect("3D artifact is supported");
        artifact.resources[4].element_stride = Some(8);
        assert!(validate_storage_global_3d_write_artifact(&artifact, 42).is_err());
        artifact.resources[4].element_stride = Some(4);
        assert!(validate_storage_global_3d_write_artifact(&artifact, 43).is_err());
        assert!(
            emit_storage_global_3d_write_from_spirv_artifact(
                &artifact,
                MslOptions::new([2, 4, 2]).expect("valid MSL workgroup"),
                42,
            )
            .is_err()
        );
    }

    #[test]
    fn lowers_validated_global_3d_strided_write_spirv_artifact_to_msl() {
        let module = jir_global_3d_strided_write_module();
        let artifact = emit_storage_global_3d_strided_write_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([4, 4, 2]).expect("valid SPIR-V workgroup"),
        )
        .expect("3D strided artifact is supported");
        validate_storage_global_3d_strided_write_artifact(&artifact, 42)
            .expect("3D strided artifact contract is valid");
        let source = emit_storage_global_3d_strided_write_from_spirv_artifact(
            &artifact,
            MslOptions::new([4, 4, 2]).expect("valid MSL workgroup"),
            42,
        )
        .expect("3D strided artifact lowers to MSL");
        validate_storage_global_3d_strided_write(&source, "global_3d_strided_write_u32")
            .expect("translated 3D strided MSL contract is valid");
        assert!(source.contains("gid.x * stride_x[0]"));
        assert!(source.contains("gid.z * stride_z[0]"));
        assert!(source.contains("data[physical] = 42u;"));
    }

    #[test]
    fn rejects_global_3d_strided_write_spirv_artifact_contract_mismatch() {
        let module = jir_global_3d_strided_write_module();
        let mut artifact = emit_storage_global_3d_strided_write_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([4, 4, 2]).expect("valid SPIR-V workgroup"),
        )
        .expect("3D strided artifact is supported");
        artifact.resources[6].element_stride = Some(8);
        assert!(validate_storage_global_3d_strided_write_artifact(&artifact, 42).is_err());
        artifact.resources[6].element_stride = Some(4);
        assert!(validate_storage_global_3d_strided_write_artifact(&artifact, 43).is_err());
        assert!(
            emit_storage_global_3d_strided_write_from_spirv_artifact(
                &artifact,
                MslOptions::new([2, 4, 2]).expect("valid MSL workgroup"),
                42,
            )
            .is_err()
        );
    }

    #[test]
    fn dispatches_known_spirv_artifact_family_to_msl() {
        let module = jir_global_3d_strided_write_module();
        let artifact = emit_storage_global_3d_strided_write_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([4, 4, 2]).expect("valid SPIR-V workgroup"),
        )
        .expect("3D strided artifact is supported");
        let source = translate_spirv_artifact_to_msl(
            &artifact,
            MslOptions::new([4, 4, 2]).expect("valid MSL workgroup"),
        )
        .expect("known artifact family is dispatched");
        validate_storage_global_3d_strided_write(&source, "global_3d_strided_write_u32")
            .expect("dispatched MSL contract is valid");
    }

    #[test]
    fn rejects_unknown_spirv_artifact_family_before_msl_emission() {
        let module = jir_global_3d_strided_write_module();
        let mut artifact = emit_storage_global_3d_strided_write_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([4, 4, 2]).expect("valid SPIR-V workgroup"),
        )
        .expect("3D strided artifact is supported");
        artifact.resources.truncate(7);
        assert!(matches!(
            translate_spirv_artifact_to_msl(
                &artifact,
                MslOptions::new([4, 4, 2]).expect("valid MSL workgroup"),
            ),
            Err(MslError::UnsupportedJirShape(_))
        ));
    }

    #[test]
    fn rejects_invalid_external_spirv_before_toolchain_lookup() {
        assert!(matches!(
            translate_spirv_to_msl(&[], "main"),
            Err(MslError::InvalidSpirv(_))
        ));
        assert!(matches!(
            translate_spirv_to_msl(&[0x0723_0203, 1, 0, 1, 0], "bad-name!"),
            Err(MslError::InvalidSpirv(_))
                | Err(MslError::InvalidEntryName)
                | Err(MslError::SpirvToolchainUnavailable)
                | Err(MslError::SpirvTranslation(_))
        ));
    }

    #[test]
    fn rejects_invalid_external_artifact_before_toolchain_lookup() {
        let module = jir_global_3d_strided_write_module();
        let options = SpirvOptions::new([4, 4, 2]).expect("valid SPIR-V workgroup");

        let mut truncated = emit_storage_global_3d_strided_write_artifact_from_jir(
            &module,
            FunctionId::new(0),
            options,
        )
        .expect("3D strided artifact is supported");
        truncated.words.truncate(5);
        assert!(matches!(
            translate_spirv_artifact_to_msl_external(&truncated),
            Err(MslError::InvalidSpirvArtifact(_))
        ));
        assert!(matches!(
            translate_spirv_artifact_to_msl_external_report(&truncated),
            Err(MslError::InvalidSpirvArtifact(_))
        ));

        let mut invalid_workgroup = emit_storage_global_3d_strided_write_artifact_from_jir(
            &module,
            FunctionId::new(0),
            options,
        )
        .expect("3D strided artifact is supported");
        invalid_workgroup.workgroup_size = [0, 1, 1];
        assert!(matches!(
            translate_spirv_artifact_to_msl_external(&invalid_workgroup),
            Err(MslError::InvalidWorkgroupSize(_))
        ));

        let mut duplicate_binding = emit_storage_global_3d_strided_write_artifact_from_jir(
            &module,
            FunctionId::new(0),
            options,
        )
        .expect("3D strided artifact is supported");
        duplicate_binding.resources[1].binding = duplicate_binding.resources[0].binding;
        assert!(matches!(
            translate_spirv_artifact_to_msl_external(&duplicate_binding),
            Err(MslError::InvalidSpirvArtifact(_))
        ));
    }

    #[test]
    fn validates_external_msl_resource_bindings() {
        let module = jir_global_3d_strided_write_module();
        let mut artifact = emit_storage_global_3d_strided_write_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([4, 4, 2]).expect("valid SPIR-V workgroup"),
        )
        .expect("3D strided artifact is supported");
        for resource in &mut artifact.resources {
            resource.name = format!("resource_{}", resource.binding);
        }
        let parameters = artifact
            .resources
            .iter()
            .map(|resource| {
                let qualifier = if resource.access == ResourceAccess::ReadOnly {
                    "device const"
                } else {
                    "device"
                };
                let binding = resource.binding;
                format!("{qualifier} uint* resource_{binding} [[buffer({binding})]],")
            })
            .collect::<Vec<_>>()
            .join(" ");
        let source = format!("kernel void {}({}) {{}}", artifact.entry_name, parameters);
        let validation = validate_external_msl_source(&source, &artifact);
        assert!(validation.is_ok(), "{validation:?}");
        let unknown_binding = source.replace(") {}", ", device uint* extra [[buffer(8)]]) {}");
        assert!(matches!(
            validate_external_msl_source(&unknown_binding, &artifact),
            Err(MslError::SpirvTranslation(_))
        ));
        let mut descriptor_set_artifact = artifact.clone();
        descriptor_set_artifact.resources[0].descriptor_set = 1;
        assert!(matches!(
            validate_external_msl_source(&source, &descriptor_set_artifact),
            Err(MslError::SpirvTranslation(_))
        ));
        let mut uniform_artifact = artifact.clone();
        uniform_artifact.resources[0].address_space = AddressSpace::Uniform;
        assert!(matches!(
            validate_external_msl_source(&source, &uniform_artifact),
            Err(MslError::SpirvTranslation(_))
        ));
        let renamed = source.replace("resource_7 [[buffer(7)]]", "renamed_7 [[buffer(7)]]");
        assert!(matches!(
            validate_external_msl_source(&renamed, &artifact),
            Err(MslError::SpirvTranslation(_))
        ));
        let duplicate = source.replace("[[buffer(7)]]", "[[buffer(6)]]");
        assert!(matches!(
            validate_external_msl_source(&duplicate, &artifact),
            Err(MslError::SpirvTranslation(_))
        ));
        let missing = source.replace("resource_7 [[buffer(7)]],", "resource_7,");
        assert!(matches!(
            validate_external_msl_source(&missing, &artifact),
            Err(MslError::SpirvTranslation(_))
        ));
        let non_numeric = source.replace("[[buffer(7)]]", "[[buffer(x)]]");
        assert!(matches!(
            validate_external_msl_source(&non_numeric, &artifact),
            Err(MslError::SpirvTranslation(_))
        ));
        let truncated = source.replace("[[buffer(7)]]", "[[buffer(7)]");
        assert!(matches!(
            validate_external_msl_source(&truncated, &artifact),
            Err(MslError::SpirvTranslation(_))
        ));
        let stride_mismatch = source.replace(
            "device const uint* resource_7",
            "device const uint2* resource_7",
        );
        assert!(matches!(
            validate_external_msl_source(&stride_mismatch, &artifact),
            Err(MslError::SpirvTranslation(_))
        ));
        let signedness_mismatch = source.replace(
            "device const uint* resource_7",
            "device const int* resource_7",
        );
        assert!(matches!(
            validate_external_msl_source(&signedness_mismatch, &artifact),
            Err(MslError::SpirvTranslation(_))
        ));
        let unsupported_type = source.replace(
            "device const uint* resource_7",
            "device const CustomElement* resource_7",
        );
        assert!(matches!(
            validate_external_msl_source(&unsupported_type, &artifact),
            Err(MslError::SpirvTranslation(_))
        ));
        let mut composite_artifact = artifact.clone();
        composite_artifact.resources[7].element_stride = None;
        composite_artifact.resources[7].element_type_info = None;
        assert!(validate_external_msl_source(&unsupported_type, &composite_artifact).is_ok());
        assert_eq!(
            msl_device_element_stride("device const float4* resource"),
            Some(16)
        );
        assert_eq!(
            msl_device_element_stride("device float3* resource"),
            Some(12)
        );
        assert_eq!(
            msl_device_element_type("device float3* resource"),
            Some(ResourceElementType::Float { bits: 32, lanes: 3 })
        );
        let mut read_only_artifact = artifact.clone();
        read_only_artifact.resources[0].access = ResourceAccess::ReadOnly;
        let read_only_source =
            source.replace("device uint* resource_0", "device const uint* resource_0");
        assert!(validate_external_msl_source(&read_only_source, &read_only_artifact).is_ok());
        assert!(matches!(
            validate_external_msl_source(&source, &read_only_artifact),
            Err(MslError::SpirvTranslation(_))
        ));
        assert_eq!(
            msl_device_resource_is_read_only("device const float* resource"),
            Some(true)
        );
        assert_eq!(
            msl_device_resource_is_read_only("device float* resource"),
            Some(false)
        );
        assert_eq!(
            msl_device_resource_name("device const float4* resource_7"),
            Some("resource_7")
        );
    }

    #[test]
    fn validates_external_msl_mixed_resource_access_policy() {
        let module = jir_global_3d_strided_write_module();
        let mut artifact = emit_storage_global_3d_strided_write_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([4, 4, 2]).expect("valid MSL workgroup"),
        )
        .expect("3D strided artifact is supported");
        for resource in &mut artifact.resources {
            resource.name = format!("resource_{}", resource.binding);
            resource.access = if matches!(resource.binding, 0 | 2) {
                ResourceAccess::ReadOnly
            } else {
                ResourceAccess::ReadWrite
            };
        }
        let parameters = (0..8)
            .map(|binding| {
                let qualifier = if matches!(binding, 0 | 2) {
                    "device const uint*"
                } else {
                    "device uint*"
                };
                format!("{qualifier} resource_{binding} [[buffer({binding})]],")
            })
            .collect::<Vec<_>>()
            .join(" ");
        let source = format!("kernel void {}({}) {{}}", artifact.entry_name, parameters);
        assert!(
            validate_external_msl_source(&source, &artifact).is_ok(),
            "mixed read-only/read-write MSL source should match artifact policy"
        );

        let writable_read_only =
            source.replace("device const uint* resource_0", "device uint* resource_0");
        assert!(matches!(
            validate_external_msl_source(&writable_read_only, &artifact),
            Err(MslError::SpirvTranslation(_))
        ));
        let read_only_writable =
            source.replace("device uint* resource_1", "device const uint* resource_1");
        assert!(matches!(
            validate_external_msl_source(&read_only_writable, &artifact),
            Err(MslError::SpirvTranslation(_))
        ));
    }

    #[test]
    fn lowers_verified_jir_global_3d_write() {
        let instruction = |result, kind| Instruction {
            result,
            kind,
            span: None,
        };
        let value = |id, ty| {
            Some(TypedValue {
                value: ValueId::new(id),
                ty: TypeId::new(ty),
            })
        };
        let parameter = |id, name: &str| Parameter {
            value: ValueId::new(id),
            ty: TypeId::new(2),
            name: Some(name.to_owned()),
        };
        let load = |pointer| InstructionKind::Load {
            pointer: ValueId::new(pointer),
            alignment: 4,
            volatile: false,
        };
        let module = Module {
            types: vec![
                Type::Unit,
                Type::Integer {
                    signed: false,
                    bits: 32,
                },
                Type::Pointer {
                    pointee: TypeId::new(1),
                    address_space: AddressSpace::Storage,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "global_3d_write_u32".to_owned(),
                linkage: Linkage::Export,
                parameters: vec![
                    parameter(0, "data"),
                    parameter(1, "width"),
                    parameter(2, "height"),
                    parameter(3, "depth"),
                    parameter(4, "capacity"),
                ],
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        instruction(
                            value(5, 1),
                            InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                        ),
                        instruction(
                            value(6, 1),
                            InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdY),
                        ),
                        instruction(
                            value(7, 1),
                            InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdZ),
                        ),
                        instruction(
                            value(8, 1),
                            InstructionKind::Constant(Constant::Integer { value: 42 }),
                        ),
                        instruction(value(9, 1), load(1)),
                        instruction(value(10, 1), load(2)),
                        instruction(value(11, 1), load(3)),
                        instruction(value(12, 1), load(4)),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(5),
                                length: ValueId::new(9),
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(6),
                                length: ValueId::new(10),
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(7),
                                length: ValueId::new(11),
                            },
                        ),
                        instruction(
                            value(13, 1),
                            InstructionKind::Binary {
                                op: BinaryOp::Multiply,
                                left: ValueId::new(7),
                                right: ValueId::new(10),
                            },
                        ),
                        instruction(
                            value(14, 1),
                            InstructionKind::Binary {
                                op: BinaryOp::Add,
                                left: ValueId::new(13),
                                right: ValueId::new(6),
                            },
                        ),
                        instruction(
                            value(15, 1),
                            InstructionKind::Binary {
                                op: BinaryOp::Multiply,
                                left: ValueId::new(14),
                                right: ValueId::new(9),
                            },
                        ),
                        instruction(
                            value(16, 1),
                            InstructionKind::Binary {
                                op: BinaryOp::Add,
                                left: ValueId::new(15),
                                right: ValueId::new(5),
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(16),
                                length: ValueId::new(12),
                            },
                        ),
                        instruction(
                            value(17, 2),
                            InstructionKind::Offset {
                                base: ValueId::new(0),
                                indices: vec![ValueId::new(16)],
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::Store {
                                pointer: ValueId::new(17),
                                value: ValueId::new(8),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                    ],
                    terminator: Terminator::Return { value: None },
                    span: None,
                }],
                span: None,
            }],
        };
        let source = emit_storage_global_3d_write_from_jir(
            &module,
            FunctionId::new(0),
            MslOptions::new([4, 4, 2]).unwrap(),
        )
        .expect("global 3D JIR shape is supported");
        validate_storage_global_3d_write(&source, "global_3d_write_u32")
            .expect("global 3D MSL contract is valid");
        assert!(source.contains("data[physical] = 42u;"));
    }

    #[test]
    fn lowers_verified_jir_global_3d_strided_write() {
        let value = |id, ty| {
            Some(TypedValue {
                value: ValueId::new(id),
                ty: TypeId::new(ty),
            })
        };
        let parameter = |id, name: &str| Parameter {
            value: ValueId::new(id),
            ty: TypeId::new(2),
            name: Some(name.to_owned()),
        };
        let load = |result, pointer| Instruction {
            result: value(result, 1),
            kind: InstructionKind::Load {
                pointer: ValueId::new(pointer),
                alignment: 4,
                volatile: false,
            },
            span: None,
        };
        let bounds = |index, length| Instruction {
            result: None,
            kind: InstructionKind::BoundsCheck {
                index: ValueId::new(index),
                length: ValueId::new(length),
            },
            span: None,
        };
        let binary = |result, op, left, right| Instruction {
            result: value(result, 1),
            kind: InstructionKind::Binary {
                op,
                left: ValueId::new(left),
                right: ValueId::new(right),
            },
            span: None,
        };
        let module = Module {
            types: vec![
                Type::Unit,
                Type::Integer {
                    signed: false,
                    bits: 32,
                },
                Type::Pointer {
                    pointee: TypeId::new(1),
                    address_space: AddressSpace::Storage,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "global_3d_strided_write_u32".to_owned(),
                linkage: Linkage::Export,
                parameters: vec![
                    parameter(0, "data"),
                    parameter(1, "width"),
                    parameter(2, "height"),
                    parameter(3, "depth"),
                    parameter(4, "stride_x"),
                    parameter(5, "stride_y"),
                    parameter(6, "stride_z"),
                    parameter(7, "capacity"),
                ],
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        Instruction {
                            result: value(8, 1),
                            kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                            span: None,
                        },
                        Instruction {
                            result: value(9, 1),
                            kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdY),
                            span: None,
                        },
                        Instruction {
                            result: value(10, 1),
                            kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdZ),
                            span: None,
                        },
                        Instruction {
                            result: value(11, 1),
                            kind: InstructionKind::Constant(Constant::Integer { value: 42 }),
                            span: None,
                        },
                        load(12, 1),
                        load(13, 2),
                        load(14, 3),
                        load(15, 4),
                        load(16, 5),
                        load(17, 6),
                        load(18, 7),
                        bounds(8, 12),
                        bounds(9, 13),
                        bounds(10, 14),
                        binary(19, BinaryOp::Multiply, 8, 15),
                        binary(20, BinaryOp::Multiply, 9, 16),
                        binary(21, BinaryOp::Add, 19, 20),
                        binary(22, BinaryOp::Multiply, 10, 17),
                        binary(23, BinaryOp::Add, 21, 22),
                        bounds(23, 18),
                        Instruction {
                            result: value(24, 2),
                            kind: InstructionKind::Offset {
                                base: ValueId::new(0),
                                indices: vec![ValueId::new(23)],
                            },
                            span: None,
                        },
                        Instruction {
                            result: None,
                            kind: InstructionKind::Store {
                                pointer: ValueId::new(24),
                                value: ValueId::new(11),
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
        let source = emit_storage_global_3d_strided_write_from_jir(
            &module,
            FunctionId::new(0),
            MslOptions::new([4, 4, 2]).unwrap(),
        )
        .expect("global 3D strided JIR shape is supported");
        validate_storage_global_3d_strided_write(&source, "global_3d_strided_write_u32")
            .expect("global 3D strided MSL contract is valid");
        assert!(source.contains("data[physical] = 42u;"));

        let mut invalid_capacity_guard = module.clone();
        invalid_capacity_guard.functions[0].blocks[0].instructions[19].kind =
            InstructionKind::BoundsCheck {
                index: ValueId::new(23),
                length: ValueId::new(17),
            };
        assert!(matches!(
            emit_storage_global_3d_strided_write_from_jir(
                &invalid_capacity_guard,
                FunctionId::new(0),
                MslOptions::new([4, 4, 2]).unwrap(),
            ),
            Err(MslError::UnsupportedJirShape(_))
        ));
    }

    #[test]
    fn lowers_verified_jir_global_2d_write() {
        let instruction = |result, kind| Instruction {
            result,
            kind,
            span: None,
        };
        let value = |id, ty| {
            Some(TypedValue {
                value: ValueId::new(id),
                ty: TypeId::new(ty),
            })
        };
        let parameter = |id, name: &str| Parameter {
            value: ValueId::new(id),
            ty: TypeId::new(2),
            name: Some(name.to_owned()),
        };
        let load = |pointer| InstructionKind::Load {
            pointer: ValueId::new(pointer),
            alignment: 4,
            volatile: false,
        };
        let module = Module {
            types: vec![
                Type::Unit,
                Type::Integer {
                    signed: false,
                    bits: 32,
                },
                Type::Pointer {
                    pointee: TypeId::new(1),
                    address_space: AddressSpace::Storage,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "global_2d_write_u32".to_owned(),
                linkage: Linkage::Export,
                parameters: vec![
                    parameter(0, "data"),
                    parameter(1, "width"),
                    parameter(2, "height"),
                    parameter(3, "capacity"),
                ],
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        instruction(
                            value(4, 1),
                            InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                        ),
                        instruction(
                            value(5, 1),
                            InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdY),
                        ),
                        instruction(
                            value(6, 1),
                            InstructionKind::Constant(Constant::Integer { value: 42 }),
                        ),
                        instruction(value(7, 1), load(1)),
                        instruction(value(8, 1), load(2)),
                        instruction(value(9, 1), load(3)),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(4),
                                length: ValueId::new(7),
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(5),
                                length: ValueId::new(8),
                            },
                        ),
                        instruction(
                            value(10, 1),
                            InstructionKind::Binary {
                                op: BinaryOp::Multiply,
                                left: ValueId::new(5),
                                right: ValueId::new(7),
                            },
                        ),
                        instruction(
                            value(11, 1),
                            InstructionKind::Binary {
                                op: BinaryOp::Add,
                                left: ValueId::new(10),
                                right: ValueId::new(4),
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(11),
                                length: ValueId::new(9),
                            },
                        ),
                        instruction(
                            value(12, 2),
                            InstructionKind::Offset {
                                base: ValueId::new(0),
                                indices: vec![ValueId::new(11)],
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::Store {
                                pointer: ValueId::new(12),
                                value: ValueId::new(6),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                    ],
                    terminator: Terminator::Return { value: None },
                    span: None,
                }],
                span: None,
            }],
        };
        let source = emit_storage_global_2d_write_from_jir(
            &module,
            FunctionId::new(0),
            MslOptions::new([8, 8, 1]).unwrap(),
        )
        .expect("global 2D JIR shape is supported");
        validate_storage_global_2d_write(&source, "global_2d_write_u32")
            .expect("global 2D MSL contract is valid");
        assert!(source.contains("data[physical] = 42u;"));
    }

    #[test]
    fn lowers_validated_global_2d_write_spirv_artifact_to_msl() {
        let module = jir_global_2d_write_module();
        let artifact = emit_storage_global_2d_write_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([8, 8, 1]).expect("valid SPIR-V workgroup"),
        )
        .expect("2D global write artifact is supported");
        validate_storage_global_2d_write_artifact(&artifact, 42)
            .expect("2D global write artifact contract is valid");
        let source = emit_storage_global_2d_write_from_spirv_artifact(
            &artifact,
            MslOptions::new([8, 8, 1]).expect("valid MSL workgroup"),
            42,
        )
        .expect("2D global write artifact lowers to MSL");
        validate_storage_global_2d_write(&source, "global_2d_write_u32")
            .expect("translated 2D MSL contract is valid");
        assert!(source.contains("gid.x < width[0] && gid.y < height[0]"));
        assert!(source.contains("data[physical] = 42u;"));
    }

    #[test]
    fn rejects_global_2d_write_spirv_artifact_contract_mismatch() {
        let module = jir_global_2d_write_module();
        let mut artifact = emit_storage_global_2d_write_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([8, 8, 1]).expect("valid SPIR-V workgroup"),
        )
        .expect("2D global write artifact is supported");
        artifact.resources[1].element_stride = Some(8);
        assert!(validate_storage_global_2d_write_artifact(&artifact, 42).is_err());
        artifact.resources[1].element_stride = Some(4);
        assert!(validate_storage_global_2d_write_artifact(&artifact, 43).is_err());
        assert!(
            emit_storage_global_2d_write_from_spirv_artifact(
                &artifact,
                MslOptions::new([4, 8, 1]).expect("valid MSL workgroup"),
                42,
            )
            .is_err()
        );
    }

    #[test]
    fn lowers_verified_jir_global_strided_write() {
        let instruction = |result, kind| Instruction {
            result,
            kind,
            span: None,
        };
        let value = |id, ty| {
            Some(TypedValue {
                value: ValueId::new(id),
                ty: TypeId::new(ty),
            })
        };
        let parameter = |id, name: &str| Parameter {
            value: ValueId::new(id),
            ty: TypeId::new(2),
            name: Some(name.to_owned()),
        };
        let module = Module {
            types: vec![
                Type::Unit,
                Type::Integer {
                    signed: false,
                    bits: 32,
                },
                Type::Pointer {
                    pointee: TypeId::new(1),
                    address_space: AddressSpace::Storage,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "global_strided_write_u32".to_owned(),
                linkage: Linkage::Export,
                parameters: vec![
                    parameter(0, "data"),
                    parameter(1, "length"),
                    parameter(2, "stride"),
                    parameter(3, "capacity"),
                ],
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        instruction(
                            value(4, 1),
                            InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                        ),
                        instruction(
                            value(5, 1),
                            InstructionKind::Constant(Constant::Integer { value: 42 }),
                        ),
                        instruction(
                            value(6, 1),
                            InstructionKind::Load {
                                pointer: ValueId::new(1),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        instruction(
                            value(7, 1),
                            InstructionKind::Load {
                                pointer: ValueId::new(2),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        instruction(
                            value(8, 1),
                            InstructionKind::Load {
                                pointer: ValueId::new(3),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(4),
                                length: ValueId::new(6),
                            },
                        ),
                        instruction(
                            value(9, 1),
                            InstructionKind::Binary {
                                op: BinaryOp::Multiply,
                                left: ValueId::new(4),
                                right: ValueId::new(7),
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(9),
                                length: ValueId::new(8),
                            },
                        ),
                        instruction(
                            value(10, 2),
                            InstructionKind::Offset {
                                base: ValueId::new(0),
                                indices: vec![ValueId::new(9)],
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::Store {
                                pointer: ValueId::new(10),
                                value: ValueId::new(5),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                    ],
                    terminator: Terminator::Return { value: None },
                    span: None,
                }],
                span: None,
            }],
        };
        let source = emit_storage_global_strided_write_from_jir(
            &module,
            FunctionId::new(0),
            MslOptions::new([64, 1, 1]).unwrap(),
        )
        .expect("global strided JIR shape is supported");
        validate_storage_global_strided_write(&source, "global_strided_write_u32")
            .expect("global strided MSL contract is valid");
        assert!(source.contains("data[physical] = 42u;"));
    }

    #[test]
    fn lowers_validated_global_strided_write_spirv_artifact_to_msl() {
        let module = jir_global_strided_write_module();
        let artifact = emit_storage_global_index_strided_write_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([64, 1, 1]).expect("valid SPIR-V workgroup"),
        )
        .expect("global strided write artifact is supported");
        validate_storage_global_strided_write_artifact(&artifact, 42)
            .expect("global strided artifact contract is valid");
        let source = emit_storage_global_strided_write_from_spirv_artifact(
            &artifact,
            MslOptions::new([64, 1, 1]).expect("valid MSL workgroup"),
            42,
        )
        .expect("global strided artifact lowers to MSL");
        validate_storage_global_strided_write(&source, "global_strided_write_u32")
            .expect("translated global strided MSL contract is valid");
        assert!(source.contains("uint physical = gid.x * stride[0];"));
        assert!(source.contains("data[physical] = 42u;"));
    }

    #[test]
    fn rejects_global_strided_write_spirv_artifact_contract_mismatch() {
        let module = jir_global_strided_write_module();
        let mut artifact = emit_storage_global_index_strided_write_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([64, 1, 1]).expect("valid SPIR-V workgroup"),
        )
        .expect("global strided write artifact is supported");
        artifact.resources[2].element_stride = Some(8);
        assert!(validate_storage_global_strided_write_artifact(&artifact, 42).is_err());
        artifact.resources[2].element_stride = Some(4);
        assert!(validate_storage_global_strided_write_artifact(&artifact, 43).is_err());
        assert!(
            emit_storage_global_strided_write_from_spirv_artifact(
                &artifact,
                MslOptions::new([32, 1, 1]).expect("valid MSL workgroup"),
                42,
            )
            .is_err()
        );
    }

    #[test]
    fn lowers_verified_jir_global_write() {
        let mut module = jir_dynamic_storage_add_module();
        let function = &mut module.functions[0];
        function.name = "global_write_u32".to_owned();
        function.parameters.truncate(1);
        let instructions = &mut function.blocks[0].instructions;
        instructions.truncate(6);
        instructions[0].result = Some(TypedValue {
            value: ValueId::new(1),
            ty: TypeId::new(1),
        });
        instructions[1].result = Some(TypedValue {
            value: ValueId::new(2),
            ty: TypeId::new(1),
        });
        instructions[2].result = Some(TypedValue {
            value: ValueId::new(3),
            ty: TypeId::new(1),
        });
        instructions[2].kind = InstructionKind::Constant(Constant::Integer { value: 70 });
        instructions[3].kind = InstructionKind::BoundsCheck {
            index: ValueId::new(1),
            length: ValueId::new(3),
        };
        instructions[4].result = Some(TypedValue {
            value: ValueId::new(4),
            ty: TypeId::new(2),
        });
        instructions[4].kind = InstructionKind::Offset {
            base: ValueId::new(0),
            indices: vec![ValueId::new(1)],
        };
        instructions[5].result = None;
        instructions[5].kind = InstructionKind::Store {
            pointer: ValueId::new(4),
            value: ValueId::new(2),
            alignment: 4,
            volatile: false,
        };
        let verification_errors = jadren_jir::verify_gpu(&module);
        assert!(verification_errors.is_empty(), "{verification_errors:?}");
        let source = emit_storage_global_write_from_jir(
            &module,
            FunctionId::new(0),
            MslOptions::new([64, 1, 1]).unwrap(),
        )
        .expect("global write JIR shape is supported");
        validate_storage_global_write(&source, "global_write_u32")
            .expect("global write MSL contract is valid");
        assert!(source.contains("if (gid.x < 70u)"));
        assert!(source.contains("data[gid.x] = 1u;"));
    }

    #[test]
    fn lowers_validated_global_write_spirv_artifact_to_msl() {
        let mut module = jir_dynamic_storage_add_module();
        let function = &mut module.functions[0];
        function.name = "global_write_u32".to_owned();
        function.parameters.truncate(1);
        let instructions = &mut function.blocks[0].instructions;
        instructions.truncate(6);
        instructions[0].result = Some(TypedValue {
            value: ValueId::new(1),
            ty: TypeId::new(1),
        });
        instructions[1].result = Some(TypedValue {
            value: ValueId::new(2),
            ty: TypeId::new(1),
        });
        instructions[2].result = Some(TypedValue {
            value: ValueId::new(3),
            ty: TypeId::new(1),
        });
        instructions[2].kind = InstructionKind::Constant(Constant::Integer { value: 70 });
        instructions[3].kind = InstructionKind::BoundsCheck {
            index: ValueId::new(1),
            length: ValueId::new(3),
        };
        instructions[4].result = Some(TypedValue {
            value: ValueId::new(4),
            ty: TypeId::new(2),
        });
        instructions[4].kind = InstructionKind::Offset {
            base: ValueId::new(0),
            indices: vec![ValueId::new(1)],
        };
        instructions[5].result = None;
        instructions[5].kind = InstructionKind::Store {
            pointer: ValueId::new(4),
            value: ValueId::new(2),
            alignment: 4,
            volatile: false,
        };
        let spirv_options = SpirvOptions::new([64, 1, 1]).expect("valid SPIR-V workgroup");
        let artifact = emit_storage_global_index_write_artifact_from_jir(
            &module,
            FunctionId::new(0),
            spirv_options,
        )
        .expect("global write artifact is supported");
        validate_storage_global_write_artifact(&artifact, 1, 70)
            .expect("global write artifact contract is valid");
        let source = emit_storage_global_write_from_spirv_artifact(
            &artifact,
            MslOptions::new([64, 1, 1]).expect("valid MSL workgroup"),
            1,
            70,
        )
        .expect("global write artifact lowers to MSL");
        validate_storage_global_write(&source, "global_write_u32")
            .expect("translated global write MSL contract is valid");
        assert!(source.contains("if (gid.x < 70u)"));
        assert!(source.contains("data[gid.x] = 1u;"));
    }

    #[test]
    fn rejects_global_write_spirv_artifact_contract_mismatch() {
        let mut module = jir_dynamic_storage_add_module();
        let function = &mut module.functions[0];
        function.name = "global_write_u32".to_owned();
        function.parameters.truncate(1);
        let instructions = &mut function.blocks[0].instructions;
        instructions.truncate(6);
        instructions[0].result = Some(TypedValue {
            value: ValueId::new(1),
            ty: TypeId::new(1),
        });
        instructions[1].result = Some(TypedValue {
            value: ValueId::new(2),
            ty: TypeId::new(1),
        });
        instructions[2].result = Some(TypedValue {
            value: ValueId::new(3),
            ty: TypeId::new(1),
        });
        instructions[2].kind = InstructionKind::Constant(Constant::Integer { value: 70 });
        instructions[3].kind = InstructionKind::BoundsCheck {
            index: ValueId::new(1),
            length: ValueId::new(3),
        };
        instructions[4].result = Some(TypedValue {
            value: ValueId::new(4),
            ty: TypeId::new(2),
        });
        instructions[4].kind = InstructionKind::Offset {
            base: ValueId::new(0),
            indices: vec![ValueId::new(1)],
        };
        instructions[5].result = None;
        instructions[5].kind = InstructionKind::Store {
            pointer: ValueId::new(4),
            value: ValueId::new(2),
            alignment: 4,
            volatile: false,
        };
        let mut artifact = emit_storage_global_index_write_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([64, 1, 1]).expect("valid SPIR-V workgroup"),
        )
        .expect("global write artifact is supported");
        artifact.resources[0].element_stride = Some(8);
        assert!(validate_storage_global_write_artifact(&artifact, 1, 70).is_err());
        artifact.resources[0].element_stride = Some(4);
        assert!(validate_storage_global_write_artifact(&artifact, 2, 70).is_err());
        assert!(validate_storage_global_write_artifact(&artifact, 1, 71).is_err());
    }

    #[test]
    fn rejects_invalid_workgroup_and_entry() {
        assert_eq!(
            MslOptions::new([0, 1, 1]),
            Err(MslError::InvalidWorkgroupSize([0, 1, 1]))
        );
        assert_eq!(
            MslOptions::new([33, 32, 1]),
            Err(MslError::WorkgroupTooLarge(1056))
        );
        let options = MslOptions::new([1, 1, 1]).expect("valid workgroup");
        assert_eq!(
            emit_storage_add("jadren-main", options, 1),
            Err(MslError::InvalidEntryName)
        );
    }

    #[test]
    fn validator_rejects_missing_contract_fragment() {
        assert_eq!(
            validate_storage_add("kernel void main() {}", "main"),
            Err(MslError::InvalidContract)
        );
    }

    #[test]
    fn lowers_verified_jir_dynamic_storage_add() {
        let module = jir_dynamic_storage_add_module();
        let options = MslOptions::new([64, 1, 1]).expect("valid workgroup");
        let source = emit_storage_add_from_jir(&module, FunctionId::new(0), options)
            .expect("JIR shape is supported");
        validate_storage_add(&source, "add_u32").expect("source contract is valid");
        assert!(source.contains("output[gid.x] = input[gid.x] + 1u;"));
    }

    #[test]
    fn lowers_verified_jir_dynamic_storage_f32_add() {
        let module = jir_dynamic_storage_f32_add_module();
        let options = MslOptions::new([64, 1, 1]).expect("valid workgroup");
        let source = emit_storage_f32_add_from_jir(&module, FunctionId::new(0), options)
            .expect("f32 JIR shape is supported");
        validate_storage_f32_add(&source, "add_f32").expect("f32 source contract is valid");
        assert!(source.contains("as_type<float>(1065353216u)"));
    }

    #[test]
    fn lowers_verified_jir_dynamic_storage_f32_binary_family() {
        let options = MslOptions::new([64, 1, 1]).expect("valid workgroup");
        for (operation, name, operand_bits, expression) in [
            (
                F32ArithmeticOp::Subtract,
                "subtract_f32",
                0x3f80_0000,
                "output[gid.x] = input[gid.x] - as_type<float>(1065353216u);",
            ),
            (
                F32ArithmeticOp::Multiply,
                "multiply_f32",
                0x4000_0000,
                "output[gid.x] = input[gid.x] * as_type<float>(1073741824u);",
            ),
        ] {
            let module = jir_dynamic_storage_f32_binary_module(operation, name, operand_bits);
            let source =
                emit_storage_f32_binary_from_jir(&module, FunctionId::new(0), options, operation)
                    .expect("f32 binary JIR shape is supported");
            validate_storage_f32_binary(&source, name, operation)
                .expect("f32 binary source contract is valid");
            assert!(source.contains(expression));
        }
        let add_module = jir_dynamic_storage_f32_add_module();
        assert_eq!(
            emit_storage_f32_binary_from_jir(
                &add_module,
                FunctionId::new(0),
                options,
                F32ArithmeticOp::Multiply,
            ),
            Err(MslError::UnsupportedJirShape(
                "f32 binary operands are invalid"
            ))
        );
    }

    #[test]
    fn lowers_reordered_jir_dynamic_storage_multiply() {
        let mut module = jir_dynamic_storage_add_module();
        module.functions[0].blocks[0].instructions.swap(0, 1);
        module.functions[0].blocks[0].instructions[6].kind = InstructionKind::Binary {
            op: BinaryOp::Multiply,
            left: ValueId::new(7),
            right: ValueId::new(4),
        };
        let options = MslOptions::new([64, 1, 1]).expect("valid workgroup");
        let source =
            emit_storage_binary_from_jir(&module, FunctionId::new(0), options, BinaryOp::Multiply)
                .expect("reordered JIR shape is supported");
        validate_storage_add(&source, "add_u32").expect("source contract is valid");
        assert!(source.contains("output[gid.x] = input[gid.x] * 1u;"));
    }

    #[test]
    fn rejects_unsafe_jir_dynamic_binary_operands() {
        for (operation, operand) in [
            (BinaryOp::Divide, 0_i128),
            (BinaryOp::Remainder, 0_i128),
            (BinaryOp::ShiftLeft, 32_i128),
            (BinaryOp::ShiftRight, 32_i128),
        ] {
            let mut module = jir_dynamic_storage_add_module();
            module.functions[0].blocks[0].instructions[1].kind =
                InstructionKind::Constant(Constant::Integer { value: operand });
            module.functions[0].blocks[0].instructions[6].kind = InstructionKind::Binary {
                op: operation,
                left: ValueId::new(7),
                right: ValueId::new(4),
            };
            assert!(matches!(
                emit_storage_binary_from_jir(
                    &module,
                    FunctionId::new(0),
                    MslOptions::new([64, 1, 1]).unwrap(),
                    operation,
                ),
                Err(MslError::UnsupportedJirShape(_))
            ));
        }
    }
}
