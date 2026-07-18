#![allow(unsafe_code)]
// JADREN-UNSAFE-AUDIT: ash calls are confined to Vulkan lifecycle wrappers;
// instance/device/queue/fence ownership is explicit and public C ABI pointer
// contracts are documented with `# Safety`.

//! Minimal explicit Vulkan host runtime for JAD-1305.
//!
//! This crate owns the loader/instance/device/queue/descriptor/command-pool/
//! fence lifecycle and submits descriptor-bound JIR storage `u32`/`f32`
//! arithmetic commands, then verifies mapped writeback and host residency
//! scope.

use std::ffi::{CStr, CString, c_void};
use std::thread::{self, JoinHandle};

use ash::{Entry, Instance, vk};
use jadren_codegen_spirv::{
    F32ArithmeticOp, SpirvArtifact, SpirvOptions, emit_storage_add_artifact_from_jir,
    emit_storage_dynamic_index_add_from_jir, emit_storage_dynamic_index_fadd_from_jir,
    emit_storage_global_2d_strided_write_artifact_from_jir,
    emit_storage_global_2d_write_artifact_from_jir,
    emit_storage_global_3d_strided_write_artifact_from_jir,
    emit_storage_global_3d_write_artifact_from_jir, emit_storage_global_index_add_from_jir,
    emit_storage_global_index_binary_dynamic_length_artifact_from_jir,
    emit_storage_global_index_f32_binary_dynamic_length_artifact_from_jir,
    emit_storage_global_index_strided_write_artifact_from_jir,
    emit_storage_global_index_vector_f32_binary_dynamic_length_artifact_from_jir,
    emit_storage_global_index_vector_f32_binary_dynamic_length_lanes_artifact_from_jir,
    emit_storage_global_index_write_artifact_from_jir, reflect_resources,
};
use jadren_gpu_runtime::{
    AccessKind, AccessToken, ArtifactDispatchRequest, ArtifactResourceRequest, BackendProbe,
    BufferId, DifferentialPolicy, DispatchGeometry, FpPolicy, GpuBackend, ResourceTable,
    TensorLayout2D, TensorLayout3D, compare_f32, compare_u32, prepare_artifact_dispatch,
    stable_spirv_word_hash, validate_spirv_artifact_contract,
};
use jadren_jir::{
    AddressSpace, BinaryOp, Block, BlockId, BuiltinOp, Constant, Function, FunctionId, Instruction,
    InstructionKind, Linkage, Module, Parameter, Terminator, Type, TypeId, TypedValue, ValueId,
};
use serde::Serialize;

const STORAGE_KERNEL_INPUT: u32 = 41;
const STORAGE_KERNEL_ADDEND: u32 = 1;
const STORAGE_KERNEL_INDEX_VALUE: u32 = 1;
const STORAGE_KERNEL_INPUT_INDEX: u32 = STORAGE_KERNEL_INDEX_VALUE;
const STORAGE_KERNEL_OUTPUT_INDEX: u32 = STORAGE_KERNEL_INDEX_VALUE;
const STORAGE_KERNEL_INPUT_BASE_OFFSET: u64 = 0;
const STORAGE_KERNEL_OUTPUT_BASE_OFFSET: u64 = 8;
const STORAGE_KERNEL_INDEX_BASE_OFFSET: u64 = 16;
const STORAGE_KERNEL_BUFFER_SIZE: u64 = 20;
const GLOBAL_KERNEL_ELEMENT_COUNT: usize = 64;
const GLOBAL_KERNEL_WORKGROUP_SIZE: u32 = 64;
const GLOBAL_KERNEL_INPUT_BASE_OFFSET: u64 = 0;
const GLOBAL_KERNEL_OUTPUT_BASE_OFFSET: u64 = 256;
const GLOBAL_KERNEL_BUFFER_SIZE: u64 = 512;
const GLOBAL_KERNEL_ADDEND: u32 = 1;
const GLOBAL_WRITE_ELEMENT_COUNT: usize = 64;
const GLOBAL_WRITE_WORKGROUP_SIZE: u32 = 64;
const GLOBAL_WRITE_VALUE: u32 = 42;
const GLOBAL_WRITE_TAIL_ELEMENT_COUNT: usize = 70;
const GLOBAL_WRITE_TAIL_CAPACITY: usize = 128;
const STORAGE_ADD_ARTIFACT_ELEMENT_COUNT: usize = 64;
const STORAGE_ADD_ARTIFACT_WORKGROUP_SIZE: [u32; 3] = [1, 1, 1];
const STORAGE_ADD_ARTIFACT_BUFFER_SIZE: u64 =
    (STORAGE_ADD_ARTIFACT_ELEMENT_COUNT * std::mem::size_of::<u32>()) as u64;
const STORAGE_ADD_ARTIFACT_INPUT_START: u32 = 41;
const STORAGE_ADD_ARTIFACT_ADDEND: u32 = 1;
const STRIDED_WRITE_ELEMENT_COUNT: usize = 70;
const STRIDED_WRITE_CAPACITY: usize = 160;
const STRIDED_WRITE_WORKGROUP_SIZE: u32 = 64;
const STRIDED_WRITE_STRIDE: u32 = 2;
const STRIDED_WRITE_VALUE: u32 = 42;
const STRIDED_WRITE_BUFFER_BASE_OFFSET: u64 = 0;
const STRIDED_WRITE_LENGTH_BASE_OFFSET: u64 =
    (STRIDED_WRITE_CAPACITY * std::mem::size_of::<u32>()) as u64;
const STRIDED_WRITE_STRIDE_BASE_OFFSET: u64 = STRIDED_WRITE_LENGTH_BASE_OFFSET + 4;
const STRIDED_WRITE_CAPACITY_BASE_OFFSET: u64 = STRIDED_WRITE_STRIDE_BASE_OFFSET + 4;
const STRIDED_WRITE_BUFFER_SIZE: u64 = STRIDED_WRITE_CAPACITY_BASE_OFFSET + 4;
const TWO_D_WRITE_WIDTH: usize = 10;
const TWO_D_WRITE_HEIGHT: usize = 7;
const TWO_D_WRITE_CAPACITY: usize = 80;
const TWO_D_WRITE_WORKGROUP_SIZE: [u32; 3] = [8, 8, 1];
const TWO_D_WRITE_VALUE: u32 = 42;
const TWO_D_WRITE_BUFFER_BASE_OFFSET: u64 = 0;
const TWO_D_WRITE_WIDTH_BASE_OFFSET: u64 =
    (TWO_D_WRITE_CAPACITY * std::mem::size_of::<u32>()) as u64;
const TWO_D_WRITE_HEIGHT_BASE_OFFSET: u64 = TWO_D_WRITE_WIDTH_BASE_OFFSET + 4;
const TWO_D_WRITE_CAPACITY_BASE_OFFSET: u64 = TWO_D_WRITE_HEIGHT_BASE_OFFSET + 4;
const TWO_D_WRITE_BUFFER_SIZE: u64 = TWO_D_WRITE_CAPACITY_BASE_OFFSET + 4;
const TWO_D_STRIDED_WRITE_WIDTH: usize = 4;
const TWO_D_STRIDED_WRITE_HEIGHT: usize = 3;
const TWO_D_STRIDED_WRITE_STRIDE_X: u32 = 2;
const TWO_D_STRIDED_WRITE_STRIDE_Y: u32 = 10;
const TWO_D_STRIDED_WRITE_CAPACITY: usize = 40;
const TWO_D_STRIDED_WRITE_WORKGROUP_SIZE: [u32; 3] = [4, 4, 1];
const TWO_D_STRIDED_WRITE_VALUE: u32 = 42;
const TWO_D_STRIDED_WRITE_BUFFER_BASE_OFFSET: u64 = 0;
const TWO_D_STRIDED_WRITE_WIDTH_BASE_OFFSET: u64 =
    (TWO_D_STRIDED_WRITE_CAPACITY * std::mem::size_of::<u32>()) as u64;
const TWO_D_STRIDED_WRITE_HEIGHT_BASE_OFFSET: u64 = TWO_D_STRIDED_WRITE_WIDTH_BASE_OFFSET + 4;
const TWO_D_STRIDED_WRITE_STRIDE_X_BASE_OFFSET: u64 = TWO_D_STRIDED_WRITE_HEIGHT_BASE_OFFSET + 4;
const TWO_D_STRIDED_WRITE_STRIDE_Y_BASE_OFFSET: u64 = TWO_D_STRIDED_WRITE_STRIDE_X_BASE_OFFSET + 4;
const TWO_D_STRIDED_WRITE_CAPACITY_BASE_OFFSET: u64 = TWO_D_STRIDED_WRITE_STRIDE_Y_BASE_OFFSET + 4;
const TWO_D_STRIDED_WRITE_BUFFER_SIZE: u64 = TWO_D_STRIDED_WRITE_CAPACITY_BASE_OFFSET + 4;
const THREE_D_WRITE_WIDTH: usize = 5;
const THREE_D_WRITE_HEIGHT: usize = 3;
const THREE_D_WRITE_DEPTH: usize = 2;
const THREE_D_WRITE_CAPACITY: usize = 40;
const THREE_D_WRITE_WORKGROUP_SIZE: [u32; 3] = [4, 4, 2];
const THREE_D_WRITE_VALUE: u32 = 42;
const THREE_D_WRITE_BUFFER_BASE_OFFSET: u64 = 0;
const THREE_D_WRITE_WIDTH_BASE_OFFSET: u64 =
    (THREE_D_WRITE_CAPACITY * std::mem::size_of::<u32>()) as u64;
const THREE_D_WRITE_HEIGHT_BASE_OFFSET: u64 = THREE_D_WRITE_WIDTH_BASE_OFFSET + 4;
const THREE_D_WRITE_DEPTH_BASE_OFFSET: u64 = THREE_D_WRITE_HEIGHT_BASE_OFFSET + 4;
const THREE_D_WRITE_CAPACITY_BASE_OFFSET: u64 = THREE_D_WRITE_DEPTH_BASE_OFFSET + 4;
const THREE_D_WRITE_BUFFER_SIZE: u64 = THREE_D_WRITE_CAPACITY_BASE_OFFSET + 4;
const THREE_D_STRIDED_WRITE_WIDTH: usize = 4;
const THREE_D_STRIDED_WRITE_HEIGHT: usize = 3;
const THREE_D_STRIDED_WRITE_DEPTH: usize = 2;
const THREE_D_STRIDED_WRITE_STRIDE_X: u32 = 2;
const THREE_D_STRIDED_WRITE_STRIDE_Y: u32 = 11;
const THREE_D_STRIDED_WRITE_STRIDE_Z: u32 = 37;
const THREE_D_STRIDED_WRITE_CAPACITY: usize = 72;
const THREE_D_STRIDED_WRITE_WORKGROUP_SIZE: [u32; 3] = [4, 4, 2];
const THREE_D_STRIDED_WRITE_VALUE: u32 = 42;

#[derive(Clone, Copy, Debug)]
struct Global3dStridedWriteConfig {
    width: usize,
    height: usize,
    depth: usize,
    stride_x: u32,
    stride_y: u32,
    stride_z: u32,
    capacity: usize,
    value: u32,
    workgroup_size: [u32; 3],
}

#[derive(Clone, Copy, Debug)]
struct GlobalWriteArtifactConfig {
    value: u32,
    length: usize,
    capacity: usize,
    workgroup_size: u32,
}

impl GlobalWriteArtifactConfig {
    const fn baseline() -> Self {
        Self {
            value: GLOBAL_WRITE_VALUE,
            length: GLOBAL_WRITE_ELEMENT_COUNT,
            capacity: GLOBAL_WRITE_ELEMENT_COUNT,
            workgroup_size: GLOBAL_WRITE_WORKGROUP_SIZE,
        }
    }

    const fn tail() -> Self {
        Self {
            value: GLOBAL_WRITE_VALUE,
            length: GLOBAL_WRITE_TAIL_ELEMENT_COUNT,
            capacity: GLOBAL_WRITE_TAIL_CAPACITY,
            workgroup_size: GLOBAL_WRITE_WORKGROUP_SIZE,
        }
    }

    fn validate(self) -> Result<Self, RuntimeError> {
        if self.length == 0 || self.capacity < self.length || self.workgroup_size == 0 {
            return Err(RuntimeError::DescriptorContract(
                "global-write length/workgroup must be positive and capacity must cover length"
                    .to_owned(),
            ));
        }
        let _ = u32::try_from(self.length).map_err(|_| {
            RuntimeError::DescriptorContract("global-write length exceeds u32".to_owned())
        })?;
        let _ = u32::try_from(self.capacity).map_err(|_| {
            RuntimeError::DescriptorContract("global-write capacity exceeds u32".to_owned())
        })?;
        let _ = self.buffer_size()?;
        Ok(self)
    }

    fn buffer_size(self) -> Result<u64, RuntimeError> {
        u64::try_from(self.capacity)
            .ok()
            .and_then(|capacity| capacity.checked_mul(std::mem::size_of::<u32>() as u64))
            .ok_or_else(|| {
                RuntimeError::DescriptorContract("global-write buffer size overflows".to_owned())
            })
    }

    fn dispatch_x(self) -> Result<u32, RuntimeError> {
        let capacity = u32::try_from(self.capacity).map_err(|_| {
            RuntimeError::DescriptorContract("global-write capacity exceeds u32".to_owned())
        })?;
        capacity
            .checked_add(self.workgroup_size - 1)
            .map(|value| value / self.workgroup_size)
            .ok_or_else(|| {
                RuntimeError::DescriptorContract("global-write dispatch overflows".to_owned())
            })
    }
}

impl Global3dStridedWriteConfig {
    const fn fixture() -> Self {
        Self {
            width: THREE_D_STRIDED_WRITE_WIDTH,
            height: THREE_D_STRIDED_WRITE_HEIGHT,
            depth: THREE_D_STRIDED_WRITE_DEPTH,
            stride_x: THREE_D_STRIDED_WRITE_STRIDE_X,
            stride_y: THREE_D_STRIDED_WRITE_STRIDE_Y,
            stride_z: THREE_D_STRIDED_WRITE_STRIDE_Z,
            capacity: THREE_D_STRIDED_WRITE_CAPACITY,
            value: THREE_D_STRIDED_WRITE_VALUE,
            workgroup_size: THREE_D_STRIDED_WRITE_WORKGROUP_SIZE,
        }
    }

    fn validate(self) -> Result<Self, RuntimeError> {
        if self.width == 0 || self.height == 0 || self.depth == 0 || self.capacity == 0 {
            return Err(RuntimeError::DescriptorContract(
                "3D affine-stride dimensions and capacity must be positive".to_owned(),
            ));
        }
        if self.stride_x == 0 || self.stride_y == 0 || self.stride_z == 0 {
            return Err(RuntimeError::DescriptorContract(
                "3D affine-stride values must be positive".to_owned(),
            ));
        }
        if self.workgroup_size.contains(&0) {
            return Err(RuntimeError::DescriptorContract(
                "3D affine-stride workgroup dimensions must be positive".to_owned(),
            ));
        }
        let last_index = (self.width - 1)
            .checked_mul(self.stride_x as usize)
            .and_then(|index| {
                index.checked_add(
                    (self.height - 1)
                        .checked_mul(self.stride_y as usize)?
                        .checked_add((self.depth - 1).checked_mul(self.stride_z as usize)?)?,
                )
            })
            .ok_or_else(|| {
                RuntimeError::DescriptorContract(
                    "3D affine-stride physical index overflow".to_owned(),
                )
            })?;
        if last_index >= self.capacity {
            return Err(RuntimeError::DescriptorContract(
                "3D affine-stride physical index exceeds capacity".to_owned(),
            ));
        }
        let logical_element_count = self
            .width
            .checked_mul(self.height)
            .and_then(|count| count.checked_mul(self.depth))
            .ok_or_else(|| {
                RuntimeError::DescriptorContract(
                    "3D affine-stride logical element count overflow".to_owned(),
                )
            })?;
        if self.width > u32::MAX as usize
            || self.height > u32::MAX as usize
            || self.depth > u32::MAX as usize
            || self.capacity > u32::MAX as usize
            || logical_element_count > u32::MAX as usize
        {
            return Err(RuntimeError::DescriptorContract(
                "3D affine-stride metadata exceeds u32 ABI range".to_owned(),
            ));
        }
        Ok(self)
    }

    fn buffer_size(self) -> Result<u64, RuntimeError> {
        let elements_bytes = u64::try_from(self.capacity)
            .map_err(|_| {
                RuntimeError::DescriptorContract(
                    "3D affine-stride capacity conversion failed".to_owned(),
                )
            })?
            .checked_mul(std::mem::size_of::<u32>() as u64)
            .ok_or_else(|| {
                RuntimeError::DescriptorContract(
                    "3D affine-stride buffer byte size overflow".to_owned(),
                )
            })?;
        let metadata_bytes = 7_u64
            .checked_mul(std::mem::size_of::<u32>() as u64)
            .ok_or_else(|| {
                RuntimeError::DescriptorContract(
                    "3D affine-stride metadata size overflow".to_owned(),
                )
            })?;
        elements_bytes.checked_add(metadata_bytes).ok_or_else(|| {
            RuntimeError::DescriptorContract("3D affine-stride buffer size overflow".to_owned())
        })
    }

    fn metadata_offsets(self) -> [u64; 7] {
        let base = (self.capacity as u64) * std::mem::size_of::<u32>() as u64;
        [
            base,
            base + 4,
            base + 8,
            base + 12,
            base + 16,
            base + 20,
            base + 24,
        ]
    }
}

struct Global3dStridedWriteExecution {
    report: Global3dStridedWriteU32QueueSmokeReport,
    output_values: Vec<u32>,
}

const GLOBAL_DYNAMIC_ELEMENT_COUNT: usize = 70;
const GLOBAL_DYNAMIC_CAPACITY: usize = 128;
const GLOBAL_DYNAMIC_WORKGROUP_SIZE: u32 = 64;
const GLOBAL_DYNAMIC_INPUT_BASE_OFFSET: u64 = 0;
const GLOBAL_DYNAMIC_OUTPUT_BASE_OFFSET: u64 = 512;
const GLOBAL_DYNAMIC_LENGTH_BASE_OFFSET: u64 = 1024;
const GLOBAL_DYNAMIC_BUFFER_SIZE: u64 = 1028;
const GLOBAL_DYNAMIC_VECTOR_ELEMENT_COUNT: usize = 70;
const GLOBAL_DYNAMIC_VECTOR_CAPACITY: usize = 128;
const GLOBAL_DYNAMIC_VECTOR_WORKGROUP_SIZE: u32 = 64;
const GLOBAL_DYNAMIC_VECTOR_INPUT_BASE_OFFSET: u64 = 0;
const GLOBAL_DYNAMIC_VECTOR_OUTPUT_BASE_OFFSET: u64 = 2048;
const GLOBAL_DYNAMIC_VECTOR_LENGTH_BASE_OFFSET: u64 = 4096;
const GLOBAL_DYNAMIC_VECTOR_BUFFER_SIZE: u64 = 4100;
const GLOBAL_DYNAMIC_F32_ADDEND: f32 = 1.0;
const GLOBAL_DYNAMIC_F32_MULTIPLIER: f32 = 2.0;
const GLOBAL_DYNAMIC_ADDEND: u32 = 1;
const GLOBAL_DYNAMIC_MULTIPLIER: u32 = 2;
const GLOBAL_DYNAMIC_SUBTRACTOR: u32 = 1;
const GLOBAL_DYNAMIC_DIVISOR: u32 = 2;
const GLOBAL_DYNAMIC_REMAINDER_DIVISOR: u32 = 2;
const GLOBAL_DYNAMIC_BIT_MASK: u32 = 1;
const GLOBAL_DYNAMIC_SHIFT_COUNT: u32 = 1;

const fn f32_operation_name(operation: F32ArithmeticOp) -> &'static str {
    match operation {
        F32ArithmeticOp::Add => "add",
        F32ArithmeticOp::Subtract => "subtract",
        F32ArithmeticOp::Multiply => "multiply",
    }
}

const fn f32_operation_operand(operation: F32ArithmeticOp) -> f32 {
    match operation {
        F32ArithmeticOp::Add | F32ArithmeticOp::Subtract => GLOBAL_DYNAMIC_F32_ADDEND,
        F32ArithmeticOp::Multiply => GLOBAL_DYNAMIC_F32_MULTIPLIER,
    }
}

fn apply_f32_operation(value: f32, operand: f32, operation: F32ArithmeticOp) -> f32 {
    match operation {
        F32ArithmeticOp::Add => value + operand,
        F32ArithmeticOp::Subtract => value - operand,
        F32ArithmeticOp::Multiply => value * operand,
    }
}

const fn f32_binary_op(operation: F32ArithmeticOp) -> BinaryOp {
    match operation {
        F32ArithmeticOp::Add => BinaryOp::Add,
        F32ArithmeticOp::Subtract => BinaryOp::Subtract,
        F32ArithmeticOp::Multiply => BinaryOp::Multiply,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GlobalDynamicOperation {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    ShiftRight,
}

impl GlobalDynamicOperation {
    const fn from_abi(operation: u32) -> Option<Self> {
        Some(match operation {
            0 => Self::Add,
            1 => Self::Subtract,
            2 => Self::Multiply,
            3 => Self::Divide,
            4 => Self::Remainder,
            5 => Self::BitAnd,
            6 => Self::BitOr,
            7 => Self::BitXor,
            8 => Self::ShiftLeft,
            9 => Self::ShiftRight,
            _ => return None,
        })
    }

    const fn abi_code(self) -> u32 {
        match self {
            Self::Add => 0,
            Self::Subtract => 1,
            Self::Multiply => 2,
            Self::Divide => 3,
            Self::Remainder => 4,
            Self::BitAnd => 5,
            Self::BitOr => 6,
            Self::BitXor => 7,
            Self::ShiftLeft => 8,
            Self::ShiftRight => 9,
        }
    }

    const fn operand(self) -> u32 {
        match self {
            Self::Add => GLOBAL_DYNAMIC_ADDEND,
            Self::Subtract => GLOBAL_DYNAMIC_SUBTRACTOR,
            Self::Multiply => GLOBAL_DYNAMIC_MULTIPLIER,
            Self::Divide => GLOBAL_DYNAMIC_DIVISOR,
            Self::Remainder => GLOBAL_DYNAMIC_REMAINDER_DIVISOR,
            Self::BitAnd | Self::BitOr | Self::BitXor => GLOBAL_DYNAMIC_BIT_MASK,
            Self::ShiftLeft | Self::ShiftRight => GLOBAL_DYNAMIC_SHIFT_COUNT,
        }
    }

    const fn jir_op(self) -> BinaryOp {
        match self {
            Self::Add => BinaryOp::Add,
            Self::Subtract => BinaryOp::Subtract,
            Self::Multiply => BinaryOp::Multiply,
            Self::Divide => BinaryOp::Divide,
            Self::Remainder => BinaryOp::Remainder,
            Self::BitAnd => BinaryOp::BitAnd,
            Self::BitOr => BinaryOp::BitOr,
            Self::BitXor => BinaryOp::BitXor,
            Self::ShiftLeft => BinaryOp::ShiftLeft,
            Self::ShiftRight => BinaryOp::ShiftRight,
        }
    }

    const fn from_jir(operation: BinaryOp) -> Option<Self> {
        Some(match operation {
            BinaryOp::Add => Self::Add,
            BinaryOp::Subtract => Self::Subtract,
            BinaryOp::Multiply => Self::Multiply,
            BinaryOp::Divide => Self::Divide,
            BinaryOp::Remainder => Self::Remainder,
            BinaryOp::BitAnd => Self::BitAnd,
            BinaryOp::BitOr => Self::BitOr,
            BinaryOp::BitXor => Self::BitXor,
            BinaryOp::ShiftLeft => Self::ShiftLeft,
            BinaryOp::ShiftRight => Self::ShiftRight,
        })
    }

    const fn entry_name(self) -> &'static str {
        match self {
            Self::Add => "global_add_dynamic_u32",
            Self::Subtract => "global_subtract_dynamic_u32",
            Self::Multiply => "global_multiply_dynamic_u32",
            Self::Divide => "global_divide_dynamic_u32",
            Self::Remainder => "global_remainder_dynamic_u32",
            Self::BitAnd => "global_bitand_dynamic_u32",
            Self::BitOr => "global_bitor_dynamic_u32",
            Self::BitXor => "global_bitxor_dynamic_u32",
            Self::ShiftLeft => "global_shift_left_dynamic_u32",
            Self::ShiftRight => "global_shift_right_dynamic_u32",
        }
    }

    const fn schema(self) -> &'static str {
        match self {
            Self::Add => "jadren-vulkan-global-dynamic-u32-queue-smoke-0.1",
            Self::Subtract => "jadren-vulkan-global-dynamic-subtract-u32-queue-smoke-0.1",
            Self::Multiply => "jadren-vulkan-global-dynamic-multiply-u32-queue-smoke-0.1",
            Self::Divide => "jadren-vulkan-global-dynamic-divide-u32-queue-smoke-0.1",
            Self::Remainder => "jadren-vulkan-global-dynamic-remainder-u32-queue-smoke-0.1",
            Self::BitAnd => "jadren-vulkan-global-dynamic-bitand-u32-queue-smoke-0.1",
            Self::BitOr => "jadren-vulkan-global-dynamic-bitor-u32-queue-smoke-0.1",
            Self::BitXor => "jadren-vulkan-global-dynamic-bitxor-u32-queue-smoke-0.1",
            Self::ShiftLeft => "jadren-vulkan-global-dynamic-shift-left-u32-queue-smoke-0.1",
            Self::ShiftRight => "jadren-vulkan-global-dynamic-shift-right-u32-queue-smoke-0.1",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Subtract => "subtract",
            Self::Multiply => "multiply",
            Self::Divide => "divide",
            Self::Remainder => "remainder",
            Self::BitAnd => "bitand",
            Self::BitOr => "bitor",
            Self::BitXor => "bitxor",
            Self::ShiftLeft => "shift-left",
            Self::ShiftRight => "shift-right",
        }
    }

    fn validate_operand(self, operand: u32) -> Result<(), &'static str> {
        match self {
            Self::Divide | Self::Remainder if operand == 0 => {
                Err("division and remainder operands must be non-zero")
            }
            Self::ShiftLeft | Self::ShiftRight if operand >= u32::BITS => {
                Err("shift operands must be smaller than 32")
            }
            _ => Ok(()),
        }
    }

    fn apply_with_operand(self, value: u32, operand: u32) -> u32 {
        match self {
            Self::Add => value.wrapping_add(operand),
            Self::Subtract => value.wrapping_sub(operand),
            Self::Multiply => value.wrapping_mul(operand),
            Self::Divide => value / operand,
            Self::Remainder => value % operand,
            Self::BitAnd => value & operand,
            Self::BitOr => value | operand,
            Self::BitXor => value ^ operand,
            Self::ShiftLeft => value.wrapping_shl(operand),
            Self::ShiftRight => value >> operand,
        }
    }
}

#[derive(Clone, Copy)]
struct F32KernelValues {
    input: f32,
    addend: f32,
}

fn begin_storage_scope(size: u64) -> Result<(ResourceTable, BufferId, AccessToken), RuntimeError> {
    let mut table = ResourceTable::new();
    let buffer = table
        .create_buffer(size)
        .map_err(|error| RuntimeError::DescriptorContract(error.to_string()))?;
    table
        .make_resident(buffer)
        .map_err(|error| RuntimeError::DescriptorContract(error.to_string()))?;
    let token = table
        .acquire(buffer, AccessKind::ReadWrite)
        .map_err(|error| RuntimeError::DescriptorContract(error.to_string()))?;
    Ok((table, buffer, token))
}

/// Structured failure from the Vulkan host smoke.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeError {
    /// Vulkan loader could not be loaded.
    Loader(String),
    /// Jadren SPIR-V lowering rejected the requested kernel.
    Codegen(String),
    /// Vulkan operation failed.
    Vulkan {
        operation: &'static str,
        code: vk::Result,
    },
    /// No physical device exposed a compute queue.
    NoComputeQueue,
    /// JIR resource metadata cannot be represented by this smoke contract.
    DescriptorContract(String),
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Loader(message) => write!(formatter, "Vulkan loader unavailable: {message}"),
            Self::Codegen(message) => write!(formatter, "Jadren SPIR-V codegen failed: {message}"),
            Self::Vulkan { operation, code } => {
                write!(formatter, "Vulkan {operation} failed: {code:?}")
            }
            Self::NoComputeQueue => {
                formatter.write_str("no Vulkan physical device exposes a compute queue")
            }
            Self::DescriptorContract(message) => {
                write!(formatter, "invalid Vulkan descriptor contract: {message}")
            }
        }
    }
}

impl std::error::Error for RuntimeError {}

/// JSON-safe result of the explicit queue/fence smoke.
#[derive(Clone, Debug, Serialize)]
pub struct QueueSmokeReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Loader-reported API version requested by the instance.
    pub requested_api_version: String,
    /// Number of enumerated physical devices.
    pub physical_device_count: usize,
    /// Selected physical device name.
    pub selected_device: String,
    /// Selected compute-capable queue family.
    pub queue_family_index: u32,
    /// Whether queue creation and command submission completed.
    pub queue_execution: &'static str,
    /// Whether the submitted fence completed.
    pub fence_execution: &'static str,
    /// Descriptor setup status.
    pub descriptor_setup: &'static str,
    /// Number of descriptor bindings derived from JIR reflection.
    pub resource_binding_count: usize,
    /// SPIR-V compute dispatch status.
    pub compute_execution: &'static str,
    /// Descriptor-bound compute pipeline execution status.
    pub pipeline_execution: &'static str,
    /// Data writeback status for the JIR storage add kernel.
    pub data_kernel_execution: &'static str,
    /// Exact CPU/GPU differential status.
    pub differential_execution: &'static str,
    /// Host-side residency/access scope status.
    pub residency_execution: &'static str,
    /// Value observed in the mapped storage buffer after the GPU fence.
    pub data_kernel_value: u32,
    /// Value produced by the CPU reference calculation.
    pub cpu_reference_value: u32,
    /// Host value uploaded before dispatch.
    pub data_kernel_input: u32,
    /// JIR addend lowered into the SPIR-V kernel.
    pub data_kernel_addend: u32,
    /// Constant storage-array index lowered into the SPIR-V access chain.
    pub data_kernel_index: u32,
}

/// JSON-safe result of the native timeline-semaphore completion smoke.
#[derive(Clone, Debug, Serialize)]
pub struct TimelineSmokeReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Number of enumerated physical devices.
    pub physical_device_count: usize,
    /// Selected physical device name.
    pub selected_device: String,
    /// Selected compute-capable queue family.
    pub queue_family_index: u32,
    /// Timeline semaphore creation status.
    pub semaphore_execution: &'static str,
    /// Queue submission status.
    pub submit_execution: &'static str,
    /// Host wait status.
    pub wait_execution: &'static str,
    /// Counter value observed after the wait.
    pub observed_value: u64,
    /// Value signalled by the submission.
    pub expected_value: u64,
    /// Overall timeline completion status.
    pub timeline_execution: &'static str,
}

/// JSON-safe result of the native f32 queue smoke.
#[derive(Clone, Debug, Serialize)]
pub struct F32QueueSmokeReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Number of enumerated physical devices.
    pub physical_device_count: usize,
    /// Selected physical device name.
    pub selected_device: String,
    /// Selected compute-capable queue family.
    pub queue_family_index: u32,
    /// Queue submission status.
    pub queue_execution: &'static str,
    /// Descriptor setup status.
    pub descriptor_setup: &'static str,
    /// Number of reflection-derived bindings.
    pub resource_binding_count: usize,
    /// Pipeline and dispatch status.
    pub pipeline_execution: &'static str,
    /// Fence completion status.
    pub fence_execution: &'static str,
    /// f32 readback status.
    pub data_kernel_execution: &'static str,
    /// Exact CPU/GPU f32 differential status.
    pub differential_execution: &'static str,
    /// Host-side residency/access scope status.
    pub residency_execution: &'static str,
    /// GPU output value.
    pub data_kernel_value: f32,
    /// CPU reference value.
    pub cpu_reference_value: f32,
    /// Input value.
    pub data_kernel_input: f32,
    /// f32 addend.
    pub data_kernel_addend: f32,
    /// Dynamic index loaded from the index resource.
    pub data_kernel_index: u32,
}

/// JSON-safe result of the global-invocation-id array smoke.
#[derive(Clone, Debug, Serialize)]
pub struct GlobalU32QueueSmokeReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Number of enumerated physical devices.
    pub physical_device_count: usize,
    /// Selected physical device name.
    pub selected_device: String,
    /// Selected compute-capable queue family.
    pub queue_family_index: u32,
    /// Queue submission status.
    pub queue_execution: &'static str,
    /// Descriptor setup status.
    pub descriptor_setup: &'static str,
    /// Number of reflection-derived bindings.
    pub resource_binding_count: usize,
    /// Pipeline and dispatch status.
    pub pipeline_execution: &'static str,
    /// Fence completion status.
    pub fence_execution: &'static str,
    /// Array readback status.
    pub data_kernel_execution: &'static str,
    /// Exact CPU/GPU array differential status.
    pub differential_execution: &'static str,
    /// Host-side residency/access scope status.
    pub residency_execution: &'static str,
    /// Number of elements processed by GlobalInvocationId.x.
    pub element_count: usize,
    /// Input checksum uploaded before dispatch.
    pub input_checksum: u64,
    /// GPU output checksum after readback.
    pub output_checksum: u64,
    /// CPU reference checksum.
    pub expected_checksum: u64,
    /// First output element.
    pub first_output: u32,
    /// Last output element.
    pub last_output: u32,
}

/// JSON-safe result of the one-resource global-index write smoke.
#[derive(Clone, Debug, Serialize)]
pub struct GlobalWriteU32QueueSmokeReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Number of enumerated physical devices.
    pub physical_device_count: usize,
    /// Selected physical device name.
    pub selected_device: String,
    /// Selected compute-capable queue family.
    pub queue_family_index: u32,
    /// Verified artifact entrypoint sent directly to `vkCreateShaderModule`.
    pub artifact_entry_name: String,
    /// Number of structurally validated SPIR-V words.
    pub artifact_word_count: usize,
    /// Stable FNV-1a hash of the validated little-endian SPIR-V word stream.
    pub artifact_word_hash: u64,
    /// Whether the artifact passed its portable validation contract.
    pub artifact_validated: bool,
    /// Queue submission status.
    pub queue_execution: &'static str,
    /// Descriptor setup status.
    pub descriptor_setup: &'static str,
    /// Number of reflection-derived bindings.
    pub resource_binding_count: usize,
    /// Pipeline and dispatch status.
    pub pipeline_execution: &'static str,
    /// Fence completion status.
    pub fence_execution: &'static str,
    /// Array readback status.
    pub data_kernel_execution: &'static str,
    /// Exact CPU/GPU array differential status.
    pub differential_execution: &'static str,
    /// Host-side residency/access scope status.
    pub residency_execution: &'static str,
    /// Number of elements written by GlobalInvocationId.x.
    pub element_count: usize,
    /// Backing storage capacity; excess elements must remain unchanged.
    pub capacity: usize,
    /// Number of X dispatch groups submitted to the queue.
    pub dispatch_x: u32,
    /// Number of capacity tail elements proven unchanged by readback.
    pub untouched_elements: usize,
    /// GPU output checksum after readback.
    pub output_checksum: u64,
    /// CPU reference checksum.
    pub expected_checksum: u64,
    /// First output element.
    pub first_output: u32,
    /// Last output element.
    pub last_output: u32,
}

/// JSON-safe result of the verified one-resource JIR storage-add artifact.
///
/// The kernel only mutates element zero. The remaining backing buffer is kept
/// in the readback report so a cross-backend gate can prove the narrow shape
/// did not accidentally widen into an array operation.
#[derive(Clone, Debug, Serialize)]
pub struct StorageAddArtifactQueueSmokeReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Number of enumerated physical devices.
    pub physical_device_count: usize,
    /// Selected physical device name.
    pub selected_device: String,
    /// Selected compute-capable queue family.
    pub queue_family_index: u32,
    /// Verified artifact entrypoint sent directly to `vkCreateShaderModule`.
    pub artifact_entry_name: String,
    /// Number of structurally validated SPIR-V words.
    pub artifact_word_count: usize,
    /// Stable FNV-1a hash of the validated little-endian SPIR-V word stream.
    pub artifact_word_hash: u64,
    /// Whether the artifact passed its portable validation contract.
    pub artifact_validated: bool,
    /// Queue submission status.
    pub queue_execution: &'static str,
    /// Descriptor setup status.
    pub descriptor_setup: &'static str,
    /// Number of reflection-derived bindings.
    pub resource_binding_count: usize,
    /// Pipeline and dispatch status.
    pub pipeline_execution: &'static str,
    /// Fence completion status.
    pub fence_execution: &'static str,
    /// Array readback status.
    pub data_kernel_execution: &'static str,
    /// Exact CPU/GPU array differential status.
    pub differential_execution: &'static str,
    /// Host-side residency/access scope status.
    pub residency_execution: &'static str,
    /// Number of input/output elements in the backing storage buffer.
    pub element_count: usize,
    /// Constant encoded by the validated JIR storage-add artifact.
    pub addend: u32,
    /// GPU output checksum after readback.
    pub output_checksum: u64,
    /// CPU reference checksum.
    pub expected_checksum: u64,
    /// Changed output at storage element zero.
    pub first_output: u32,
    /// Last output, which must remain untouched by this kernel shape.
    pub last_output: u32,
    /// Number of elements unchanged after element zero.
    pub untouched_tail_count: usize,
}

/// JSON-safe result of the runtime-stride global-index write smoke.
#[derive(Clone, Debug, Serialize)]
pub struct GlobalStridedWriteU32QueueSmokeReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Artifact entrypoint executed by the native Vulkan shader module.
    pub artifact_entry_name: String,
    /// Number of validated SPIR-V words in the shared artifact.
    pub artifact_word_count: usize,
    /// Stable FNV-1a hash of the shared SPIR-V artifact words.
    pub artifact_word_hash: u64,
    /// Whether artifact structural validation completed before dispatch.
    pub artifact_validated: bool,
    /// Number of enumerated physical devices.
    pub physical_device_count: usize,
    /// Selected physical device name.
    pub selected_device: String,
    /// Selected compute-capable queue family.
    pub queue_family_index: u32,
    /// Queue submission status.
    pub queue_execution: &'static str,
    /// Descriptor setup status.
    pub descriptor_setup: &'static str,
    /// Number of reflection-derived bindings.
    pub resource_binding_count: usize,
    /// Pipeline and dispatch status.
    pub pipeline_execution: &'static str,
    /// Fence completion status.
    pub fence_execution: &'static str,
    /// Array readback status.
    pub data_kernel_execution: &'static str,
    /// Exact CPU/GPU array differential status.
    pub differential_execution: &'static str,
    /// Host-side residency/access scope status.
    pub residency_execution: &'static str,
    /// Logical invocation length.
    pub logical_length: usize,
    /// Physical buffer capacity in elements.
    pub capacity: usize,
    /// Runtime stride in elements.
    pub stride: u32,
    /// Number of dispatched x workgroups.
    pub dispatch_x: u32,
    /// Last physical index written by the logical range.
    pub last_physical_index: usize,
    /// GPU output checksum after readback.
    pub output_checksum: u64,
    /// CPU reference checksum.
    pub expected_checksum: u64,
    /// Number of physical elements left zero by the strided writes.
    pub untouched_elements: usize,
}

/// JSON-safe result of the runtime 2D row-major global-index write smoke.
#[derive(Clone, Debug, Serialize)]
pub struct Global2dWriteU32QueueSmokeReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Artifact entrypoint executed by the native Vulkan shader module.
    pub artifact_entry_name: String,
    /// Number of validated SPIR-V words in the shared artifact.
    pub artifact_word_count: usize,
    /// Stable FNV-1a hash of the shared SPIR-V artifact words.
    pub artifact_word_hash: u64,
    /// Whether artifact structural validation completed before dispatch.
    pub artifact_validated: bool,
    /// Number of enumerated physical devices.
    pub physical_device_count: usize,
    /// Selected physical device name.
    pub selected_device: String,
    /// Selected compute-capable queue family.
    pub queue_family_index: u32,
    /// Queue submission status.
    pub queue_execution: &'static str,
    /// Descriptor setup status.
    pub descriptor_setup: &'static str,
    /// Number of reflection-derived bindings.
    pub resource_binding_count: usize,
    /// Pipeline and dispatch status.
    pub pipeline_execution: &'static str,
    /// Fence completion status.
    pub fence_execution: &'static str,
    /// Array readback status.
    pub data_kernel_execution: &'static str,
    /// Exact CPU/GPU array differential status.
    pub differential_execution: &'static str,
    /// Host-side residency/access scope status.
    pub residency_execution: &'static str,
    /// Logical row width.
    pub width: usize,
    /// Logical row count.
    pub height: usize,
    /// Physical buffer capacity in elements.
    pub capacity: usize,
    /// Number of dispatched x workgroups.
    pub dispatch_x: u32,
    /// Number of dispatched y workgroups.
    pub dispatch_y: u32,
    /// Last row-major physical index written.
    pub last_physical_index: usize,
    /// GPU output checksum after readback.
    pub output_checksum: u64,
    /// CPU reference checksum.
    pub expected_checksum: u64,
    /// Number of physical elements left zero by the 2D writes.
    pub untouched_elements: usize,
}

/// JSON-safe result of the runtime 2D affine-stride global-index write smoke.
#[derive(Clone, Debug, Serialize)]
pub struct Global2dStridedWriteU32QueueSmokeReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Artifact entrypoint executed by the native Vulkan shader module.
    pub artifact_entry_name: String,
    /// Number of validated SPIR-V words in the shared artifact.
    pub artifact_word_count: usize,
    /// Stable FNV-1a hash of the shared SPIR-V artifact words.
    pub artifact_word_hash: u64,
    /// Whether artifact structural validation completed before dispatch.
    pub artifact_validated: bool,
    /// Number of enumerated physical devices.
    pub physical_device_count: usize,
    /// Selected physical device name.
    pub selected_device: String,
    /// Selected compute-capable queue family.
    pub queue_family_index: u32,
    /// Queue submission status.
    pub queue_execution: &'static str,
    /// Descriptor setup status.
    pub descriptor_setup: &'static str,
    /// Number of reflection-derived bindings.
    pub resource_binding_count: usize,
    /// Pipeline and dispatch status.
    pub pipeline_execution: &'static str,
    /// Fence completion status.
    pub fence_execution: &'static str,
    /// Array readback status.
    pub data_kernel_execution: &'static str,
    /// Exact CPU/GPU array differential status.
    pub differential_execution: &'static str,
    /// Host-side residency/access scope status.
    pub residency_execution: &'static str,
    /// Logical row width.
    pub width: usize,
    /// Logical row count.
    pub height: usize,
    /// Runtime element stride for the x coordinate.
    pub stride_x: u32,
    /// Runtime element stride for the y coordinate.
    pub stride_y: u32,
    /// Physical buffer capacity in elements.
    pub capacity: usize,
    /// Number of dispatched x workgroups.
    pub dispatch_x: u32,
    /// Number of dispatched y workgroups.
    pub dispatch_y: u32,
    /// Last affine physical index written.
    pub last_physical_index: usize,
    /// GPU output checksum after readback.
    pub output_checksum: u64,
    /// CPU reference checksum.
    pub expected_checksum: u64,
    /// Number of physical elements left zero by the affine writes.
    pub untouched_elements: usize,
}

/// JSON-safe result of the runtime 3D row-major global-index write smoke.
#[derive(Clone, Debug, Serialize)]
pub struct Global3dWriteU32QueueSmokeReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Artifact entrypoint executed by the native Vulkan shader module.
    pub artifact_entry_name: String,
    /// Number of validated SPIR-V words in the shared artifact.
    pub artifact_word_count: usize,
    /// Stable FNV-1a hash of the shared SPIR-V artifact words.
    pub artifact_word_hash: u64,
    /// Whether artifact structural validation completed before dispatch.
    pub artifact_validated: bool,
    /// Number of enumerated physical devices.
    pub physical_device_count: usize,
    /// Selected physical device name.
    pub selected_device: String,
    /// Selected compute-capable queue family.
    pub queue_family_index: u32,
    /// Queue submission status.
    pub queue_execution: &'static str,
    /// Descriptor setup status.
    pub descriptor_setup: &'static str,
    /// Number of reflection-derived bindings.
    pub resource_binding_count: usize,
    /// Pipeline and dispatch status.
    pub pipeline_execution: &'static str,
    /// Fence completion status.
    pub fence_execution: &'static str,
    /// Array readback status.
    pub data_kernel_execution: &'static str,
    /// Exact CPU/GPU array differential status.
    pub differential_execution: &'static str,
    /// Host-side residency/access scope status.
    pub residency_execution: &'static str,
    /// Logical row width.
    pub width: usize,
    /// Logical row height.
    pub height: usize,
    /// Logical depth.
    pub depth: usize,
    /// Physical buffer capacity in elements.
    pub capacity: usize,
    /// Number of dispatched x workgroups.
    pub dispatch_x: u32,
    /// Number of dispatched y workgroups.
    pub dispatch_y: u32,
    /// Number of dispatched z workgroups.
    pub dispatch_z: u32,
    /// Last row-major physical index written.
    pub last_physical_index: usize,
    /// GPU output checksum after readback.
    pub output_checksum: u64,
    /// CPU reference checksum.
    pub expected_checksum: u64,
    /// Number of physical elements left zero by the 3D writes.
    pub untouched_elements: usize,
}

/// JSON-safe result of the runtime 3D affine-stride global-index write smoke.
#[derive(Clone, Debug, Serialize)]
pub struct Global3dStridedWriteU32QueueSmokeReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Artifact entrypoint executed by the native Vulkan shader module.
    pub artifact_entry_name: String,
    /// Number of validated SPIR-V words in the shared artifact.
    pub artifact_word_count: usize,
    /// Stable FNV-1a hash of the shared SPIR-V artifact words.
    pub artifact_word_hash: u64,
    /// Whether artifact structural validation completed before dispatch.
    pub artifact_validated: bool,
    /// Number of enumerated physical devices.
    pub physical_device_count: usize,
    /// Selected physical device name.
    pub selected_device: String,
    /// Selected compute-capable queue family.
    pub queue_family_index: u32,
    /// Queue submission status.
    pub queue_execution: &'static str,
    /// Descriptor setup status.
    pub descriptor_setup: &'static str,
    /// Number of reflection-derived bindings.
    pub resource_binding_count: usize,
    /// Pipeline and dispatch status.
    pub pipeline_execution: &'static str,
    /// Fence completion status.
    pub fence_execution: &'static str,
    /// Timeline semaphore completion status for the same kernel submission.
    pub timeline_execution: &'static str,
    /// Timeline counter observed after the GPU submission.
    pub timeline_value: u64,
    /// Array readback status.
    pub data_kernel_execution: &'static str,
    /// Exact CPU/GPU array differential status.
    pub differential_execution: &'static str,
    /// Host-side residency/access scope status.
    pub residency_execution: &'static str,
    /// Logical x dimension.
    pub width: usize,
    /// Logical y dimension.
    pub height: usize,
    /// Logical z dimension.
    pub depth: usize,
    /// Runtime x stride in elements.
    pub stride_x: u32,
    /// Runtime y stride in elements.
    pub stride_y: u32,
    /// Runtime z stride in elements.
    pub stride_z: u32,
    /// Physical buffer capacity in elements.
    pub capacity: usize,
    /// Number of dispatched x workgroups.
    pub dispatch_x: u32,
    /// Number of dispatched y workgroups.
    pub dispatch_y: u32,
    /// Number of dispatched z workgroups.
    pub dispatch_z: u32,
    /// Last affine physical index written.
    pub last_physical_index: usize,
    /// GPU output checksum after readback.
    pub output_checksum: u64,
    /// CPU reference checksum.
    pub expected_checksum: u64,
    /// Number of physical elements left zero by the affine writes.
    pub untouched_elements: usize,
}

/// JSON-safe result of the dynamic-length, multi-workgroup array smoke.
#[derive(Clone, Debug, Serialize)]
pub struct GlobalDynamicU32QueueSmokeReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Arithmetic operation lowered into the JIR/SPIR-V fixture.
    pub operation: &'static str,
    /// Artifact entrypoint executed by the native Vulkan shader module.
    pub artifact_entry_name: String,
    /// Number of validated SPIR-V words in the shared artifact.
    pub artifact_word_count: usize,
    /// Stable FNV-1a hash of the shared SPIR-V artifact words.
    pub artifact_word_hash: u64,
    /// Whether artifact structural validation completed before dispatch.
    pub artifact_validated: bool,
    /// Number of enumerated physical devices.
    pub physical_device_count: usize,
    /// Selected physical device name.
    pub selected_device: String,
    /// Selected compute-capable queue family.
    pub queue_family_index: u32,
    /// Queue submission status.
    pub queue_execution: &'static str,
    /// Descriptor setup status.
    pub descriptor_setup: &'static str,
    /// Number of reflection-derived bindings.
    pub resource_binding_count: usize,
    /// Pipeline and dispatch status.
    pub pipeline_execution: &'static str,
    /// Fence completion status.
    pub fence_execution: &'static str,
    /// Array readback status.
    pub data_kernel_execution: &'static str,
    /// Exact CPU/GPU array differential status.
    pub differential_execution: &'static str,
    /// Host-side residency/access scope status.
    pub residency_execution: &'static str,
    /// Logical number of elements from the runtime length buffer.
    pub element_count: usize,
    /// Capacity allocated for two workgroups.
    pub capacity: usize,
    /// Number of dispatched workgroups.
    pub dispatch_x: u32,
    /// Runtime length value uploaded to the third resource.
    pub runtime_length: u32,
    /// Input checksum for the logical range.
    pub input_checksum: u64,
    /// GPU output checksum for the logical range.
    pub output_checksum: u64,
    /// CPU reference checksum for the logical range.
    pub expected_checksum: u64,
    /// First logical output element.
    pub first_output: u32,
    /// Last logical output element.
    pub last_output: u32,
    /// Number of capacity elements beyond the runtime length left untouched.
    pub untouched_tail_elements: usize,
}

struct GlobalDynamicU32Execution {
    report: GlobalDynamicU32QueueSmokeReport,
    output_values: Vec<u32>,
}

/// JSON-safe result of the runtime-length, multi-workgroup `f32` smoke.
#[derive(Clone, Debug, Serialize)]
pub struct GlobalDynamicF32QueueSmokeReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Entrypoint executed by the native Vulkan shader module.
    pub entry_name: String,
    /// Scalar operation encoded by the artifact.
    pub operation: &'static str,
    /// Constant f32 operand encoded by the artifact.
    pub operand: f32,
    /// Whether the shader module came from the validated artifact boundary.
    pub execution_path: &'static str,
    /// Number of validated SPIR-V words.
    pub spirv_word_count: usize,
    /// Stable FNV-1a hash of the generated SPIR-V words.
    pub spirv_word_hash: u64,
    /// Number of enumerated physical devices.
    pub physical_device_count: usize,
    /// Selected physical device name.
    pub selected_device: String,
    /// Selected compute-capable queue family.
    pub queue_family_index: u32,
    /// Number of reflection-derived bindings.
    pub resource_binding_count: usize,
    /// Logical number of elements from the runtime length buffer.
    pub element_count: usize,
    /// Physical capacity allocated for the shader dispatch.
    pub capacity: usize,
    /// Number of dispatched workgroups.
    pub dispatch_x: u32,
    /// Runtime length uploaded to the third storage resource.
    pub runtime_length: u32,
    /// Input checksum for the logical range.
    pub input_checksum: f64,
    /// GPU output checksum for the logical range.
    pub output_checksum: f64,
    /// CPU reference checksum for the logical range.
    pub expected_checksum: f64,
    /// First logical output element.
    pub first_output: f32,
    /// Last logical output element.
    pub last_output: f32,
    /// Number of capacity elements beyond the runtime length left untouched.
    pub untouched_tail_elements: usize,
    /// Exact CPU/GPU differential status.
    pub differential_execution: &'static str,
    /// Host-side residency/access scope status.
    pub residency_execution: &'static str,
}

struct GlobalDynamicF32Execution {
    report: GlobalDynamicF32QueueSmokeReport,
    output_values: Vec<f32>,
}

/// JSON-safe result of the native runtime-length `f32x4` artifact smoke.
#[derive(Clone, Debug, Serialize)]
pub struct GlobalDynamicF32VectorQueueSmokeReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Entrypoint executed by the native Vulkan shader module.
    pub entry_name: String,
    /// Vector operation encoded by the artifact.
    pub operation: &'static str,
    /// Scalar f32 operand splatted into each vector lane.
    pub operand: f32,
    /// Whether the shader module came from the validated artifact boundary.
    pub execution_path: &'static str,
    /// Number of validated SPIR-V words.
    pub spirv_word_count: usize,
    /// Stable FNV-1a hash of the SPIR-V words.
    pub spirv_word_hash: u64,
    /// Number of enumerated physical devices.
    pub physical_device_count: usize,
    /// Selected physical device name.
    pub selected_device: String,
    /// Selected compute-capable queue family.
    pub queue_family_index: u32,
    /// Number of reflection-derived bindings.
    pub resource_binding_count: usize,
    /// Logical number of vector elements.
    pub element_count: usize,
    /// Physical vector capacity.
    pub capacity: usize,
    /// Number of dispatched workgroups.
    pub dispatch_x: u32,
    /// Runtime length uploaded to the third storage resource.
    pub runtime_length: u32,
    /// Sum of all input lanes in the logical range.
    pub input_checksum: f64,
    /// Sum of all GPU output lanes in the logical range.
    pub output_checksum: f64,
    /// CPU reference checksum.
    pub expected_checksum: f64,
    /// First logical output vector.
    pub first_output: [f32; 4],
    /// Last logical output vector.
    pub last_output: [f32; 4],
    /// Number of capacity vectors beyond runtime length left untouched.
    pub untouched_tail_elements: usize,
    /// Exact CPU/GPU differential status.
    pub differential_execution: &'static str,
    /// Host-side residency/access scope status.
    pub residency_execution: &'static str,
}

struct GlobalDynamicF32VectorExecution {
    report: GlobalDynamicF32VectorQueueSmokeReport,
    output_values: Vec<[f32; 4]>,
}

/// JSON-safe result of a native runtime-length vector artifact for 2..=4 lanes.
#[derive(Clone, Debug, Serialize)]
pub struct GlobalDynamicF32VectorLanesQueueSmokeReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Entrypoint executed by the native Vulkan shader module.
    pub entry_name: String,
    /// Number of lanes in each logical vector.
    pub lane_count: usize,
    /// Vector operation encoded by the artifact.
    pub operation: &'static str,
    /// Scalar f32 operand splatted into each vector lane.
    pub operand: f32,
    /// Whether the shader module came from the validated artifact boundary.
    pub execution_path: &'static str,
    /// Number of validated SPIR-V words.
    pub spirv_word_count: usize,
    /// Stable FNV-1a hash of the SPIR-V words.
    pub spirv_word_hash: u64,
    /// Number of enumerated physical devices.
    pub physical_device_count: usize,
    /// Selected physical device name.
    pub selected_device: String,
    /// Selected compute-capable queue family.
    pub queue_family_index: u32,
    /// Number of reflection-derived bindings.
    pub resource_binding_count: usize,
    /// Logical number of vector elements.
    pub element_count: usize,
    /// Physical vector capacity.
    pub capacity: usize,
    /// Number of dispatched workgroups.
    pub dispatch_x: u32,
    /// Runtime length uploaded to the third storage resource.
    pub runtime_length: u32,
    /// Sum of all input lanes in the logical range.
    pub input_checksum: f64,
    /// Sum of all GPU output lanes in the logical range.
    pub output_checksum: f64,
    /// CPU reference checksum.
    pub expected_checksum: f64,
    /// First logical output vector.
    pub first_output: Vec<f32>,
    /// Last logical output vector.
    pub last_output: Vec<f32>,
    /// Number of capacity vectors beyond runtime length left untouched.
    pub untouched_tail_elements: usize,
    /// Exact CPU/GPU differential status.
    pub differential_execution: &'static str,
    /// Host-side residency/access scope status.
    pub residency_execution: &'static str,
}

struct GlobalDynamicF32VectorLanesExecution {
    report: GlobalDynamicF32VectorLanesQueueSmokeReport,
    output_values: Vec<Vec<f32>>,
}

struct GlobalDynamicDeviceContext<'a> {
    instance: &'a Instance,
    device: &'a ash::Device,
    physical_device: vk::PhysicalDevice,
    queue_family_index: u32,
    physical_device_count: usize,
    selected_device: String,
    input_values: &'a [u32],
    operation: GlobalDynamicOperation,
    operand: u32,
}

/// Creates a Vulkan instance/device, binds a storage-buffer descriptor, submits
/// a descriptor-bound compute shader and waits for its fence. The JIR fixture
/// adds one to an initialized `41` at buffer element zero and reads `42` back.
pub fn run_queue_smoke() -> Result<QueueSmokeReport, RuntimeError> {
    let entry =
        unsafe { Entry::load() }.map_err(|error| RuntimeError::Loader(error.to_string()))?;
    let app_name = CString::new("Jadren Vulkan Runtime").expect("static app name has no NUL");
    let application_info = vk::ApplicationInfo::default()
        .application_name(&app_name)
        .application_version(1)
        .engine_name(&app_name)
        .engine_version(1)
        .api_version(vk::API_VERSION_1_2);
    let instance_info = vk::InstanceCreateInfo::default().application_info(&application_info);
    let instance = unsafe { entry.create_instance(&instance_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_instance",
            code,
        }
    })?;

    let result = run_on_instance(&instance);
    unsafe { instance.destroy_instance(None) };
    result
}

/// Creates a Vulkan timeline semaphore, signals value `1` from a queue submit,
/// waits for that value and reads the completed counter. This is a lifecycle
/// gate for asynchronous completion; it intentionally submits no shader work.
pub fn run_timeline_smoke() -> Result<TimelineSmokeReport, RuntimeError> {
    let entry =
        unsafe { Entry::load() }.map_err(|error| RuntimeError::Loader(error.to_string()))?;
    let app_name =
        CString::new("Jadren Vulkan Timeline Runtime").expect("static app name has no NUL");
    let application_info = vk::ApplicationInfo::default()
        .application_name(&app_name)
        .application_version(1)
        .engine_name(&app_name)
        .engine_version(1)
        .api_version(vk::API_VERSION_1_2);
    let instance_info = vk::InstanceCreateInfo::default().application_info(&application_info);
    let instance = unsafe { entry.create_instance(&instance_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_timeline_instance",
            code,
        }
    })?;
    let result = run_timeline_on_instance(&instance);
    unsafe { instance.destroy_instance(None) };
    result
}

fn run_timeline_on_instance(instance: &ash::Instance) -> Result<TimelineSmokeReport, RuntimeError> {
    let physical_devices =
        unsafe { instance.enumerate_physical_devices() }.map_err(|code| RuntimeError::Vulkan {
            operation: "enumerate_timeline_devices",
            code,
        })?;
    let physical_device_count = physical_devices.len();
    let (physical_device, queue_family_index) = physical_devices
        .iter()
        .find_map(|physical_device| {
            let families =
                unsafe { instance.get_physical_device_queue_family_properties(*physical_device) };
            families
                .iter()
                .enumerate()
                .find(|(_, family)| family.queue_flags.contains(vk::QueueFlags::COMPUTE))
                .map(|(index, _)| (*physical_device, index as u32))
        })
        .ok_or(RuntimeError::NoComputeQueue)?;
    let properties = unsafe { instance.get_physical_device_properties(physical_device) };
    let selected_device = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    let mut available_vulkan12 = vk::PhysicalDeviceVulkan12Features::default();
    let mut available_features =
        vk::PhysicalDeviceFeatures2::default().push_next(&mut available_vulkan12);
    unsafe { instance.get_physical_device_features2(physical_device, &mut available_features) };
    if available_vulkan12.timeline_semaphore == vk::FALSE {
        return Err(RuntimeError::DescriptorContract(
            "selected Vulkan device does not expose timelineSemaphore".to_owned(),
        ));
    }
    let queue_priority = [1.0_f32];
    let queue_info = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family_index)
        .queue_priorities(&queue_priority);
    let mut enabled_vulkan12 =
        vk::PhysicalDeviceVulkan12Features::default().timeline_semaphore(true);
    let device_info = vk::DeviceCreateInfo::default()
        .queue_create_infos(std::slice::from_ref(&queue_info))
        .push_next(&mut enabled_vulkan12);
    let device =
        unsafe { instance.create_device(physical_device, &device_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_timeline_device",
                code,
            }
        })?;
    let queue = unsafe { device.get_device_queue(queue_family_index, 0) };
    let result = (|| {
        let mut timeline_type = vk::SemaphoreTypeCreateInfo::default()
            .semaphore_type(vk::SemaphoreType::TIMELINE)
            .initial_value(0);
        let semaphore_info = vk::SemaphoreCreateInfo::default().push_next(&mut timeline_type);
        let semaphore =
            unsafe { device.create_semaphore(&semaphore_info, None) }.map_err(|code| {
                RuntimeError::Vulkan {
                    operation: "create_timeline_semaphore",
                    code,
                }
            })?;
        let expected_value = 1_u64;
        let signal_values = [expected_value];
        let mut timeline_submit =
            vk::TimelineSemaphoreSubmitInfo::default().signal_semaphore_values(&signal_values);
        let submit_info = vk::SubmitInfo::default()
            .push_next(&mut timeline_submit)
            .signal_semaphores(std::slice::from_ref(&semaphore));
        let submit_result = unsafe {
            device.queue_submit(queue, std::slice::from_ref(&submit_info), vk::Fence::null())
        };
        if let Err(code) = submit_result {
            unsafe { device.destroy_semaphore(semaphore, None) };
            return Err(RuntimeError::Vulkan {
                operation: "submit_timeline_signal",
                code,
            });
        }
        let wait_values = [expected_value];
        let wait_info = vk::SemaphoreWaitInfo::default()
            .semaphores(std::slice::from_ref(&semaphore))
            .values(&wait_values);
        unsafe { device.wait_semaphores(&wait_info, u64::MAX) }.map_err(|code| {
            unsafe { device.destroy_semaphore(semaphore, None) };
            RuntimeError::Vulkan {
                operation: "wait_timeline_semaphore",
                code,
            }
        })?;
        let observed_value = match unsafe { device.get_semaphore_counter_value(semaphore) } {
            Ok(value) => value,
            Err(code) => {
                unsafe { device.destroy_semaphore(semaphore, None) };
                return Err(RuntimeError::Vulkan {
                    operation: "read_timeline_counter",
                    code,
                });
            }
        };
        unsafe { device.destroy_semaphore(semaphore, None) };
        Ok(TimelineSmokeReport {
            schema: "jadren-vulkan-timeline-smoke-0.1",
            physical_device_count,
            selected_device: selected_device.clone(),
            queue_family_index,
            semaphore_execution: "passed",
            submit_execution: "passed",
            wait_execution: "passed",
            observed_value,
            expected_value,
            timeline_execution: if observed_value == expected_value {
                "passed"
            } else {
                "failed"
            },
        })
    })();
    unsafe {
        let _ = device.device_wait_idle();
        device.destroy_device(None);
    }
    result
}

/// Runs a 64-element storage-array kernel indexed by
/// `GlobalInvocationId.x`. Each element is independently compared with the
/// CPU reference after the queue fence completes.
pub fn run_global_u32_queue_smoke() -> Result<GlobalU32QueueSmokeReport, RuntimeError> {
    let entry =
        unsafe { Entry::load() }.map_err(|error| RuntimeError::Loader(error.to_string()))?;
    let app_name =
        CString::new("Jadren Vulkan global-index Runtime").expect("static app name has no NUL");
    let application_info = vk::ApplicationInfo::default()
        .application_name(&app_name)
        .application_version(1)
        .engine_name(&app_name)
        .engine_version(1)
        .api_version(vk::API_VERSION_1_2);
    let instance_info = vk::InstanceCreateInfo::default().application_info(&application_info);
    let instance = unsafe { entry.create_instance(&instance_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_global_index_instance",
            code,
        }
    })?;

    let result = run_global_on_instance(&instance);
    unsafe { instance.destroy_instance(None) };
    result
}

/// Runs the one-resource bounds-safe global-index write kernel across one
/// workgroup. The fixture proves a reflected single storage binding and
/// writes a constant value to every in-range element.
pub fn run_global_write_u32_queue_smoke() -> Result<GlobalWriteU32QueueSmokeReport, RuntimeError> {
    run_global_write_artifact_with_config(GlobalWriteArtifactConfig::baseline())
}

/// Runs the same one-resource artifact with a non-divisible logical length
/// across two workgroups. The mapped capacity tail proves the `BoundsCheck`
/// branch prevents writes from the final 58 invocations.
pub fn run_global_write_u32_tail_queue_smoke()
-> Result<GlobalWriteU32QueueSmokeReport, RuntimeError> {
    run_global_write_artifact_with_config(GlobalWriteArtifactConfig::tail())
}

fn run_global_write_artifact_with_config(
    config: GlobalWriteArtifactConfig,
) -> Result<GlobalWriteU32QueueSmokeReport, RuntimeError> {
    let config = config.validate()?;
    let entry =
        unsafe { Entry::load() }.map_err(|error| RuntimeError::Loader(error.to_string()))?;
    let app_name =
        CString::new("Jadren Vulkan global-write Runtime").expect("static app name has no NUL");
    let application_info = vk::ApplicationInfo::default()
        .application_name(&app_name)
        .application_version(1)
        .engine_name(&app_name)
        .engine_version(1)
        .api_version(vk::API_VERSION_1_2);
    let instance_info = vk::InstanceCreateInfo::default().application_info(&application_info);
    let instance = unsafe { entry.create_instance(&instance_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_global_write_instance",
            code,
        }
    })?;
    let result = run_global_write_on_instance(&instance, config);
    unsafe { instance.destroy_instance(None) };
    result
}

/// Executes the validated one-resource JIR `storage-add` artifact directly
/// through Vulkan, then compares the full mapped buffer with the CPU oracle.
///
/// This is intentionally distinct from the dynamic three-resource array
/// family: the emitted JIR shape loads, adds and stores only storage element
/// zero, so the untouched tail is part of the execution contract.
pub fn run_storage_add_artifact_queue_smoke()
-> Result<StorageAddArtifactQueueSmokeReport, RuntimeError> {
    let entry =
        unsafe { Entry::load() }.map_err(|error| RuntimeError::Loader(error.to_string()))?;
    let app_name = CString::new("Jadren Vulkan storage-add artifact Runtime")
        .expect("static app name has no NUL");
    let application_info = vk::ApplicationInfo::default()
        .application_name(&app_name)
        .application_version(1)
        .engine_name(&app_name)
        .engine_version(1)
        .api_version(vk::API_VERSION_1_2);
    let instance_info = vk::InstanceCreateInfo::default().application_info(&application_info);
    let instance = unsafe { entry.create_instance(&instance_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_storage_add_artifact_instance",
            code,
        }
    })?;
    let result = run_storage_add_artifact_on_instance(&instance);
    unsafe { instance.destroy_instance(None) };
    result
}

/// Runs the runtime-stride global-index write kernel across two workgroups.
/// Logical indices are multiplied by a reflected runtime stride and both the
/// logical and physical bounds are checked on the device.
pub fn run_global_strided_write_u32_queue_smoke()
-> Result<GlobalStridedWriteU32QueueSmokeReport, RuntimeError> {
    let entry =
        unsafe { Entry::load() }.map_err(|error| RuntimeError::Loader(error.to_string()))?;
    let app_name = CString::new("Jadren Vulkan global-strided-write Runtime")
        .expect("static app name has no NUL");
    let application_info = vk::ApplicationInfo::default()
        .application_name(&app_name)
        .application_version(1)
        .engine_name(&app_name)
        .engine_version(1)
        .api_version(vk::API_VERSION_1_2);
    let instance_info = vk::InstanceCreateInfo::default().application_info(&application_info);
    let instance = unsafe { entry.create_instance(&instance_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_global_strided_write_instance",
            code,
        }
    })?;
    let result = run_global_strided_write_on_instance(&instance);
    unsafe { instance.destroy_instance(None) };
    result
}

/// Runs the runtime 2D row-major global-index write kernel.
/// Width/height/capacity are reflected metadata resources and the device
/// validates both coordinates before flattening `y * width + x`.
pub fn run_global_2d_write_u32_queue_smoke()
-> Result<Global2dWriteU32QueueSmokeReport, RuntimeError> {
    let entry =
        unsafe { Entry::load() }.map_err(|error| RuntimeError::Loader(error.to_string()))?;
    let app_name =
        CString::new("Jadren Vulkan global-2d-write Runtime").expect("static app name has no NUL");
    let application_info = vk::ApplicationInfo::default()
        .application_name(&app_name)
        .application_version(1)
        .engine_name(&app_name)
        .engine_version(1)
        .api_version(vk::API_VERSION_1_2);
    let instance_info = vk::InstanceCreateInfo::default().application_info(&application_info);
    let instance = unsafe { entry.create_instance(&instance_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_global_2d_write_instance",
            code,
        }
    })?;
    let result = run_global_2d_write_on_instance(&instance);
    unsafe { instance.destroy_instance(None) };
    result
}

/// Runs the runtime 2D affine-stride global-index write kernel.
/// Width/height, both element strides and capacity are reflected metadata
/// resources; coordinates are checked before affine physical addressing.
pub fn run_global_2d_strided_write_u32_queue_smoke()
-> Result<Global2dStridedWriteU32QueueSmokeReport, RuntimeError> {
    let entry =
        unsafe { Entry::load() }.map_err(|error| RuntimeError::Loader(error.to_string()))?;
    let app_name = CString::new("Jadren Vulkan global-2d-strided-write Runtime")
        .expect("static app name has no NUL");
    let application_info = vk::ApplicationInfo::default()
        .application_name(&app_name)
        .application_version(1)
        .engine_name(&app_name)
        .engine_version(1)
        .api_version(vk::API_VERSION_1_2);
    let instance_info = vk::InstanceCreateInfo::default().application_info(&application_info);
    let instance = unsafe { entry.create_instance(&instance_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_global_2d_strided_write_instance",
            code,
        }
    })?;
    let result = run_global_2d_strided_write_on_instance(&instance);
    unsafe { instance.destroy_instance(None) };
    result
}

/// Runs the runtime 3D row-major global-index write kernel.
/// Width/height/depth/capacity are reflected metadata resources and all three
/// coordinates are checked before flattening the physical buffer index.
pub fn run_global_3d_write_u32_queue_smoke()
-> Result<Global3dWriteU32QueueSmokeReport, RuntimeError> {
    let entry =
        unsafe { Entry::load() }.map_err(|error| RuntimeError::Loader(error.to_string()))?;
    let app_name =
        CString::new("Jadren Vulkan global-3d-write Runtime").expect("static app name has no NUL");
    let application_info = vk::ApplicationInfo::default()
        .application_name(&app_name)
        .application_version(1)
        .engine_name(&app_name)
        .engine_version(1)
        .api_version(vk::API_VERSION_1_2);
    let instance_info = vk::InstanceCreateInfo::default().application_info(&application_info);
    let instance = unsafe { entry.create_instance(&instance_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_global_3d_write_instance",
            code,
        }
    })?;
    let result = run_global_3d_write_on_instance(&instance);
    unsafe { instance.destroy_instance(None) };
    result
}

/// Runs the runtime 3D affine-stride global-index write kernel.
/// Width/height/depth, all three element strides and capacity are reflected
/// metadata resources; coordinates are checked before affine physical access.
pub fn run_global_3d_strided_write_u32_queue_smoke()
-> Result<Global3dStridedWriteU32QueueSmokeReport, RuntimeError> {
    let entry =
        unsafe { Entry::load() }.map_err(|error| RuntimeError::Loader(error.to_string()))?;
    let app_name = CString::new("Jadren Vulkan global-3d-strided-write Runtime")
        .expect("static app name has no NUL");
    let application_info = vk::ApplicationInfo::default()
        .application_name(&app_name)
        .application_version(1)
        .engine_name(&app_name)
        .engine_version(1)
        .api_version(vk::API_VERSION_1_2);
    let instance_info = vk::InstanceCreateInfo::default().application_info(&application_info);
    let instance = unsafe { entry.create_instance(&instance_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_global_3d_strided_write_instance",
            code,
        }
    })?;
    let result = run_global_3d_strided_write_on_instance(&instance);
    unsafe { instance.destroy_instance(None) };
    result
}

fn run_global_3d_strided_write_with_config(
    config: Global3dStridedWriteConfig,
) -> Result<Global3dStridedWriteExecution, RuntimeError> {
    let config = config.validate()?;
    let entry =
        unsafe { Entry::load() }.map_err(|error| RuntimeError::Loader(error.to_string()))?;
    let app_name = CString::new("Jadren Vulkan global-3d-strided-write ABI Runtime")
        .expect("static app name has no NUL");
    let application_info = vk::ApplicationInfo::default()
        .application_name(&app_name)
        .application_version(1)
        .engine_name(&app_name)
        .engine_version(1)
        .api_version(vk::API_VERSION_1_2);
    let instance_info = vk::InstanceCreateInfo::default().application_info(&application_info);
    let instance = unsafe { entry.create_instance(&instance_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_global_3d_strided_write_abi_instance",
            code,
        }
    })?;
    let result = run_global_3d_strided_write_with_config_on_instance(&instance, config);
    unsafe { instance.destroy_instance(None) };
    result
}

/// Runs the dynamic-length global-index kernel across two workgroups. The
/// length resource contains 70 while 128 invocations are dispatched; the
/// bounds branch must protect the final 58 output elements.
pub fn run_global_dynamic_u32_queue_smoke() -> Result<GlobalDynamicU32QueueSmokeReport, RuntimeError>
{
    run_global_dynamic_operation_smoke(GlobalDynamicOperation::Add)
}

/// Runs the dynamic-length global-index subtract kernel across two workgroups.
pub fn run_global_dynamic_u32_subtract_queue_smoke()
-> Result<GlobalDynamicU32QueueSmokeReport, RuntimeError> {
    run_global_dynamic_operation_smoke(GlobalDynamicOperation::Subtract)
}

/// Runs the dynamic-length global-index multiply kernel across two workgroups.
pub fn run_global_dynamic_u32_multiply_queue_smoke()
-> Result<GlobalDynamicU32QueueSmokeReport, RuntimeError> {
    run_global_dynamic_operation_smoke(GlobalDynamicOperation::Multiply)
}

/// Runs the dynamic-length global-index divide kernel across two workgroups.
pub fn run_global_dynamic_u32_divide_queue_smoke()
-> Result<GlobalDynamicU32QueueSmokeReport, RuntimeError> {
    run_global_dynamic_operation_smoke(GlobalDynamicOperation::Divide)
}

/// Runs one of the native dynamic-length `u32` arithmetic operations supported
/// by this Vulkan smoke contract.
pub fn run_global_dynamic_u32_binary_queue_smoke(
    operation: BinaryOp,
) -> Result<GlobalDynamicU32QueueSmokeReport, RuntimeError> {
    let operation = GlobalDynamicOperation::from_jir(operation).ok_or_else(|| {
        RuntimeError::DescriptorContract(
            "native dynamic-length smoke supports all u32 BinaryOp variants".to_owned(),
        )
    })?;
    run_global_dynamic_operation_smoke(operation)
}

fn run_global_dynamic_operation_smoke(
    operation: GlobalDynamicOperation,
) -> Result<GlobalDynamicU32QueueSmokeReport, RuntimeError> {
    let entry =
        unsafe { Entry::load() }.map_err(|error| RuntimeError::Loader(error.to_string()))?;
    let app_name = CString::new(format!(
        "Jadren Vulkan dynamic-length {} Runtime",
        operation.label()
    ))
    .expect("operation labels have no NUL");
    let application_info = vk::ApplicationInfo::default()
        .application_name(&app_name)
        .application_version(1)
        .engine_name(&app_name)
        .engine_version(1)
        .api_version(vk::API_VERSION_1_2);
    let instance_info = vk::InstanceCreateInfo::default().application_info(&application_info);
    let instance = unsafe { entry.create_instance(&instance_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_dynamic_length_instance",
            code,
        }
    })?;

    let input_values: Vec<u32> = (0..GLOBAL_DYNAMIC_ELEMENT_COUNT)
        .map(|index| 7_u32.saturating_add(index as u32 * 3))
        .collect();
    let result =
        run_global_dynamic_on_instance(&instance, &input_values, operation, operation.operand())
            .map(|execution| execution.report);
    unsafe { instance.destroy_instance(None) };
    result
}

/// Executes the dynamic-length kernel for caller-provided u32 values and
/// returns the GPU report together with the full capacity-sized readback.
pub fn run_global_dynamic_u32_queue_with_values(
    input_values: &[u32],
) -> Result<(GlobalDynamicU32QueueSmokeReport, Vec<u32>), RuntimeError> {
    run_global_dynamic_u32_binary_queue_with_values(input_values, BinaryOp::Add, 1)
}

/// Executes a caller-sized runtime-length `u32` binary kernel with an explicit
/// operation and operand. The operation is lowered through the same verified
/// JIR/SPIR-V shape used by the ten differential smoke variants.
pub fn run_global_dynamic_u32_binary_queue_with_values(
    input_values: &[u32],
    operation: BinaryOp,
    operand: u32,
) -> Result<(GlobalDynamicU32QueueSmokeReport, Vec<u32>), RuntimeError> {
    if input_values.is_empty() || input_values.len() > GLOBAL_DYNAMIC_CAPACITY {
        return Err(RuntimeError::DescriptorContract(format!(
            "dynamic-length input count must be in 1..={GLOBAL_DYNAMIC_CAPACITY}"
        )));
    }
    let operation = GlobalDynamicOperation::from_jir(operation).ok_or_else(|| {
        RuntimeError::DescriptorContract(
            "native dynamic-length smoke supports all u32 BinaryOp variants".to_owned(),
        )
    })?;
    operation
        .validate_operand(operand)
        .map_err(|message| RuntimeError::DescriptorContract(message.to_owned()))?;
    let entry =
        unsafe { Entry::load() }.map_err(|error| RuntimeError::Loader(error.to_string()))?;
    let app_name = CString::new("Jadren Vulkan dynamic-length ABI Runtime")
        .expect("static app name has no NUL");
    let application_info = vk::ApplicationInfo::default()
        .application_name(&app_name)
        .application_version(1)
        .engine_name(&app_name)
        .engine_version(1)
        .api_version(vk::API_VERSION_1_2);
    let instance_info = vk::InstanceCreateInfo::default().application_info(&application_info);
    let instance = unsafe { entry.create_instance(&instance_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_dynamic_length_abi_instance",
            code,
        }
    })?;
    let result = run_global_dynamic_on_instance(&instance, input_values, operation, operand)
        .map(|execution| (execution.report, execution.output_values));
    unsafe { instance.destroy_instance(None) };
    result
}

/// Runs the bounds-safe runtime-length `f32` add kernel across two workgroups.
/// The 70 logical elements exercise a partial second 64-lane workgroup; the
/// remaining capacity must remain untouched by the GPU bounds branch.
pub fn run_global_dynamic_f32_queue_smoke() -> Result<GlobalDynamicF32QueueSmokeReport, RuntimeError>
{
    let input_values: Vec<f32> = (0..GLOBAL_DYNAMIC_ELEMENT_COUNT)
        .map(|index| 7.0_f32 + index as f32 * 3.0)
        .collect();
    run_global_dynamic_f32_queue_with_values(&input_values).map(|(report, _)| report)
}

/// Executes the runtime-length `f32` kernel for caller-provided values and
/// returns the full capacity-sized GPU readback. Every logical element is
/// incremented by one; elements above `input_values.len()` stay zero.
pub fn run_global_dynamic_f32_queue_with_values(
    input_values: &[f32],
) -> Result<(GlobalDynamicF32QueueSmokeReport, Vec<f32>), RuntimeError> {
    run_global_dynamic_f32_queue_with_artifact(input_values, None, F32ArithmeticOp::Add)
}

/// Runs the same runtime-length `f32` kernel, but submits the validated
/// backend-neutral `SpirvArtifact` rather than regenerating raw words inside
/// the Vulkan host path. This is the artifact-side counterpart of the DX12
/// f32 execution smoke and is still subject to the Vulkan device gate.
pub fn run_global_dynamic_f32_artifact_queue_smoke()
-> Result<GlobalDynamicF32QueueSmokeReport, RuntimeError> {
    run_global_dynamic_f32_binary_artifact_queue_smoke(F32ArithmeticOp::Add)
}

/// Runs the runtime-length scalar `f32` artifact family for the selected
/// operation. The same three-resource SPIR-V artifact is dispatched natively
/// and checked against the operation-specific CPU oracle.
pub fn run_global_dynamic_f32_binary_artifact_queue_smoke(
    operation: F32ArithmeticOp,
) -> Result<GlobalDynamicF32QueueSmokeReport, RuntimeError> {
    let input_values: Vec<f32> = (0..GLOBAL_DYNAMIC_ELEMENT_COUNT)
        .map(|index| 7.0_f32 + index as f32 * 3.0)
        .collect();
    run_global_dynamic_f32_artifact_queue_with_operation(&input_values, operation)
        .map(|(report, _)| report)
}

/// Executes the runtime-length `f32` artifact for caller-provided values and
/// returns the full capacity-sized Vulkan readback.
pub fn run_global_dynamic_f32_artifact_queue_with_values(
    input_values: &[f32],
) -> Result<(GlobalDynamicF32QueueSmokeReport, Vec<f32>), RuntimeError> {
    run_global_dynamic_f32_artifact_queue_with_operation(input_values, F32ArithmeticOp::Add)
}

/// Executes the selected runtime-length scalar `f32` artifact for caller-
/// provided values and returns the full capacity-sized Vulkan readback.
pub fn run_global_dynamic_f32_artifact_queue_with_operation(
    input_values: &[f32],
    operation: F32ArithmeticOp,
) -> Result<(GlobalDynamicF32QueueSmokeReport, Vec<f32>), RuntimeError> {
    let operand = f32_operation_operand(operation);
    let kernel =
        storage_global_dynamic_index_f32_binary_fixture_module(operand.to_bits(), operation);
    let artifact = emit_storage_global_index_f32_binary_dynamic_length_artifact_from_jir(
        &kernel,
        FunctionId::new(0),
        SpirvOptions::new([GLOBAL_DYNAMIC_WORKGROUP_SIZE, 1, 1])
            .map_err(|error| RuntimeError::Codegen(error.to_string()))?,
        operation,
    )
    .map_err(|error| RuntimeError::Codegen(error.to_string()))?;
    run_global_dynamic_f32_queue_with_artifact(input_values, Some(artifact), operation)
}

/// Runs the runtime-length `f32x4` artifact over two workgroups and checks the
/// complete vector readback against an exact CPU oracle.
pub fn run_global_dynamic_f32_vector_artifact_queue_smoke()
-> Result<GlobalDynamicF32VectorQueueSmokeReport, RuntimeError> {
    let input_values: Vec<[f32; 4]> = (0..GLOBAL_DYNAMIC_VECTOR_ELEMENT_COUNT)
        .map(|index| {
            let base = 7.0_f32 + index as f32 * 3.0;
            [base, base + 1.0, base + 2.0, base + 3.0]
        })
        .collect();
    run_global_dynamic_f32_vector_artifact_queue_with_values(&input_values)
        .map(|(report, _)| report)
}

/// Executes the native runtime-length `f32x4` artifact for caller-provided
/// vectors and returns the full capacity-sized readback.
pub fn run_global_dynamic_f32_vector_artifact_queue_with_values(
    input_values: &[[f32; 4]],
) -> Result<(GlobalDynamicF32VectorQueueSmokeReport, Vec<[f32; 4]>), RuntimeError> {
    run_global_dynamic_f32_vector_artifact_queue_with_values_and_operation(
        input_values,
        F32ArithmeticOp::Add,
    )
}

/// Executes the runtime-length `f32x4` artifact with an explicit arithmetic
/// operation and returns the full capacity-sized readback.
pub fn run_global_dynamic_f32_vector_artifact_queue_with_values_and_operation(
    input_values: &[[f32; 4]],
    operation: F32ArithmeticOp,
) -> Result<(GlobalDynamicF32VectorQueueSmokeReport, Vec<[f32; 4]>), RuntimeError> {
    if input_values.is_empty() || input_values.len() > GLOBAL_DYNAMIC_VECTOR_CAPACITY {
        return Err(RuntimeError::DescriptorContract(format!(
            "dynamic f32x4 input count must be in 1..={GLOBAL_DYNAMIC_VECTOR_CAPACITY}"
        )));
    }
    if input_values
        .iter()
        .flatten()
        .any(|value| !value.is_finite())
    {
        return Err(RuntimeError::DescriptorContract(
            "dynamic f32x4 input values must be finite".to_owned(),
        ));
    }
    let operand = f32_operation_operand(operation);
    let kernel = storage_global_dynamic_index_vector_f32_binary_fixture_module(operand, operation);
    let artifact = emit_storage_global_index_vector_f32_binary_dynamic_length_artifact_from_jir(
        &kernel,
        FunctionId::new(0),
        SpirvOptions::new([GLOBAL_DYNAMIC_VECTOR_WORKGROUP_SIZE, 1, 1])
            .map_err(|error| RuntimeError::Codegen(error.to_string()))?,
        operation,
    )
    .map_err(|error| RuntimeError::Codegen(error.to_string()))?;
    let entry =
        unsafe { Entry::load() }.map_err(|error| RuntimeError::Loader(error.to_string()))?;
    let app_name =
        CString::new("Jadren Vulkan dynamic f32x4 Runtime").expect("static app name has no NUL");
    let application_info = vk::ApplicationInfo::default()
        .application_name(&app_name)
        .application_version(1)
        .engine_name(&app_name)
        .engine_version(1)
        .api_version(vk::API_VERSION_1_2);
    let instance_info = vk::InstanceCreateInfo::default().application_info(&application_info);
    let instance = unsafe { entry.create_instance(&instance_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_dynamic_f32x4_instance",
            code,
        }
    })?;
    let result =
        run_global_dynamic_f32_vector_on_instance(&instance, input_values, &artifact, operation);
    unsafe { instance.destroy_instance(None) };
    result.map(|execution| (execution.report, execution.output_values))
}

/// Executes a native runtime-length vector artifact with a caller-selected
/// lane width in the supported 2..=4 range. All input vectors must have the
/// same lane width and contain only finite values.
pub fn run_global_dynamic_f32_vector_lanes_artifact_queue_with_values(
    input_values: &[Vec<f32>],
) -> Result<(GlobalDynamicF32VectorLanesQueueSmokeReport, Vec<Vec<f32>>), RuntimeError> {
    run_global_dynamic_f32_vector_lanes_artifact_queue_with_values_and_operation(
        input_values,
        F32ArithmeticOp::Add,
    )
}

/// Executes a native runtime-length vector artifact with a caller-selected
/// lane width in the supported 2..=4 range and an explicit arithmetic op.
pub fn run_global_dynamic_f32_vector_lanes_artifact_queue_with_values_and_operation(
    input_values: &[Vec<f32>],
    operation: F32ArithmeticOp,
) -> Result<(GlobalDynamicF32VectorLanesQueueSmokeReport, Vec<Vec<f32>>), RuntimeError> {
    if input_values.is_empty() || input_values.len() > GLOBAL_DYNAMIC_VECTOR_CAPACITY {
        return Err(RuntimeError::DescriptorContract(format!(
            "dynamic f32 vector input count must be in 1..={GLOBAL_DYNAMIC_VECTOR_CAPACITY}"
        )));
    }
    let lane_count = input_values[0].len();
    if !(2..=4).contains(&lane_count) {
        return Err(RuntimeError::DescriptorContract(
            "dynamic f32 vector lane count must be in 2..=4".to_owned(),
        ));
    }
    if input_values
        .iter()
        .any(|value| value.len() != lane_count || value.iter().any(|lane| !lane.is_finite()))
    {
        return Err(RuntimeError::DescriptorContract(
            "dynamic f32 vector inputs must have equal finite lane counts".to_owned(),
        ));
    }
    let operand = f32_operation_operand(operation);
    let kernel = storage_global_dynamic_index_vector_f32_binary_fixture_module_lanes(
        operand,
        operation,
        lane_count as u16,
    );
    let artifact =
        emit_storage_global_index_vector_f32_binary_dynamic_length_lanes_artifact_from_jir(
            &kernel,
            FunctionId::new(0),
            SpirvOptions::new([GLOBAL_DYNAMIC_VECTOR_WORKGROUP_SIZE, 1, 1])
                .map_err(|error| RuntimeError::Codegen(error.to_string()))?,
            operation,
            lane_count as u32,
        )
        .map_err(|error| RuntimeError::Codegen(error.to_string()))?;
    let entry =
        unsafe { Entry::load() }.map_err(|error| RuntimeError::Loader(error.to_string()))?;
    let app_name = CString::new(format!("Jadren Vulkan dynamic f32x{lane_count} Runtime"))
        .expect("lane count and static text have no NUL");
    let application_info = vk::ApplicationInfo::default()
        .application_name(&app_name)
        .application_version(1)
        .engine_name(&app_name)
        .engine_version(1)
        .api_version(vk::API_VERSION_1_2);
    let instance_info = vk::InstanceCreateInfo::default().application_info(&application_info);
    let instance = unsafe { entry.create_instance(&instance_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_dynamic_f32_vector_lanes_instance",
            code,
        }
    })?;
    let result = run_global_dynamic_f32_vector_lanes_on_instance(
        &instance,
        input_values,
        lane_count,
        &artifact,
        operation,
    );
    unsafe { instance.destroy_instance(None) };
    result.map(|execution| (execution.report, execution.output_values))
}

fn run_global_dynamic_f32_queue_with_artifact(
    input_values: &[f32],
    artifact: Option<SpirvArtifact>,
    operation: F32ArithmeticOp,
) -> Result<(GlobalDynamicF32QueueSmokeReport, Vec<f32>), RuntimeError> {
    if input_values.is_empty() || input_values.len() > GLOBAL_DYNAMIC_CAPACITY {
        return Err(RuntimeError::DescriptorContract(format!(
            "dynamic f32 input count must be in 1..={GLOBAL_DYNAMIC_CAPACITY}"
        )));
    }
    if input_values.iter().any(|value| !value.is_finite()) {
        return Err(RuntimeError::DescriptorContract(
            "dynamic f32 input values must be finite".to_owned(),
        ));
    }
    let entry =
        unsafe { Entry::load() }.map_err(|error| RuntimeError::Loader(error.to_string()))?;
    let app_name =
        CString::new("Jadren Vulkan dynamic f32 Runtime").expect("static app name has no NUL");
    let application_info = vk::ApplicationInfo::default()
        .application_name(&app_name)
        .application_version(1)
        .engine_name(&app_name)
        .engine_version(1)
        .api_version(vk::API_VERSION_1_2);
    let instance_info = vk::InstanceCreateInfo::default().application_info(&application_info);
    let instance = unsafe { entry.create_instance(&instance_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_dynamic_f32_instance",
            code,
        }
    })?;
    let result =
        run_global_dynamic_f32_on_instance(&instance, input_values, artifact.as_ref(), operation)
            .map(|execution| (execution.report, execution.output_values));
    unsafe { instance.destroy_instance(None) };
    result
}

/// Runs the native f32 dynamic-index Vulkan smoke.
pub fn run_f32_queue_smoke() -> Result<F32QueueSmokeReport, RuntimeError> {
    run_f32_queue_smoke_with_values(41.0, 1.0)
}

/// Runs the native f32 kernel with an explicit input/addend pair. The first
/// native ABI adapter uses the same reflected index-1 fixture contract.
pub fn run_f32_queue_smoke_with_values(
    input_value: f32,
    addend_value: f32,
) -> Result<F32QueueSmokeReport, RuntimeError> {
    let entry =
        unsafe { Entry::load() }.map_err(|error| RuntimeError::Loader(error.to_string()))?;
    let app_name = CString::new("Jadren Vulkan f32 Runtime").expect("static app name has no NUL");
    let application_info = vk::ApplicationInfo::default()
        .application_name(&app_name)
        .application_version(1)
        .engine_name(&app_name)
        .engine_version(1)
        .api_version(vk::API_VERSION_1_2);
    let instance_info = vk::InstanceCreateInfo::default().application_info(&application_info);
    let instance = unsafe { entry.create_instance(&instance_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_f32_instance",
            code,
        }
    })?;
    let result = run_f32_on_instance(&instance, input_value, addend_value);
    unsafe { instance.destroy_instance(None) };
    result
}

/// Fixed-layout result returned by the first Unity/native Vulkan bridge.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct VulkanF32DispatchResult {
    /// Zero on successful GPU execution; negative values are stable rejects.
    pub status: i32,
    /// Value written at `output[1]` by the GPU kernel.
    pub output_value: f32,
    /// Number of physical devices observed by the runtime.
    pub physical_device_count: u32,
}

/// Variable-length u32 result returned by the Unity/native Vulkan bridge.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct VulkanU32ArrayDispatchResult {
    /// Zero on successful GPU execution; negative values are stable rejects.
    pub status: i32,
    /// Sum of the logical output range after the GPU fence.
    pub output_checksum: u64,
    /// Number of physical devices observed by the runtime.
    pub physical_device_count: u32,
    /// Number of caller elements processed by the bounds-safe kernel.
    pub processed_length: u32,
}

/// Variable-length f32 result returned by the bounds-safe Unity/native Vulkan
/// bridge. The checksum is accumulated as `f64` to avoid a lossy ABI summary.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct VulkanF32ArrayDispatchResult {
    /// Zero on successful GPU execution; negative values are stable rejects.
    pub status: i32,
    /// Sum of the logical output range after the GPU fence.
    pub output_checksum: f64,
    /// Number of physical devices observed by the runtime.
    pub physical_device_count: u32,
    /// Number of caller elements processed by the bounds-safe kernel.
    pub processed_length: u32,
}

/// Parametrized runtime-length `u32` binary result returned by the native
/// Vulkan bridge. Operation codes are stable ABI values 0..=9 in the order
/// documented by `jadren_vk_u32_binary_array`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct VulkanU32BinaryArrayDispatchResult {
    /// Zero on successful GPU execution; negative values are stable rejects.
    pub status: i32,
    /// Sum of the logical output range after the GPU fence.
    pub output_checksum: u64,
    /// Number of physical devices observed by the runtime.
    pub physical_device_count: u32,
    /// Number of caller elements processed by the bounds-safe kernel.
    pub processed_length: u32,
    /// Stable operation code supplied by the caller.
    pub operation: u32,
    /// Operand supplied to the binary operation.
    pub operand: u32,
}

/// Parametrized 3D affine-stride result returned by the Unity/native Vulkan
/// bridge. The output buffer itself is borrowed from the caller and is filled
/// only after the GPU fence and exact CPU differential complete.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct VulkanU32Tensor3DDispatchResult {
    /// Zero on successful GPU execution; negative values are stable rejects.
    pub status: i32,
    /// Sum of all physical output elements after GPU readback.
    pub output_checksum: u64,
    /// Timeline semaphore counter observed after the kernel submission.
    pub timeline_value: u64,
    /// One when the timeline completion contract passed.
    pub timeline_completed: u32,
    /// Number of physical devices observed by the runtime.
    pub physical_device_count: u32,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub stride_x: u32,
    pub stride_y: u32,
    pub stride_z: u32,
    pub capacity: u32,
    pub last_physical_index: u32,
    pub written_elements: u32,
    pub untouched_elements: u32,
}

/// Opaque handle returned by the non-blocking 3D native dispatch entrypoint.
/// The handle owns the worker thread and is consumed by `complete` or
/// `release`; callers must not use it after either operation returns.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VulkanU32Tensor3DAsyncBeginResult {
    /// Zero when a worker was created and the handle is valid.
    pub status: i32,
    /// Opaque handle consumed by poll/complete/release.
    pub handle: *mut c_void,
}

struct VulkanU32Tensor3DAsyncHandle {
    worker: Option<JoinHandle<VulkanU32Tensor3DDispatchResult>>,
    template: VulkanU32Tensor3DDispatchResult,
}

#[allow(clippy::too_many_arguments)]
fn tensor3d_failure_result(
    status: i32,
    width: u32,
    height: u32,
    depth: u32,
    stride_x: u32,
    stride_y: u32,
    stride_z: u32,
    capacity: u32,
) -> VulkanU32Tensor3DDispatchResult {
    VulkanU32Tensor3DDispatchResult {
        status,
        output_checksum: 0,
        timeline_value: 0,
        timeline_completed: 0,
        physical_device_count: 0,
        width,
        height,
        depth,
        stride_x,
        stride_y,
        stride_z,
        capacity,
        last_physical_index: 0,
        written_elements: 0,
        untouched_elements: capacity,
    }
}

fn tensor3d_result_from_report(
    report: &Global3dStridedWriteU32QueueSmokeReport,
) -> VulkanU32Tensor3DDispatchResult {
    VulkanU32Tensor3DDispatchResult {
        status: 0,
        output_checksum: report.output_checksum,
        timeline_value: report.timeline_value,
        timeline_completed: u32::from(report.timeline_execution == "passed"),
        physical_device_count: u32::try_from(report.physical_device_count).unwrap_or(u32::MAX),
        width: u32::try_from(report.width).unwrap_or(u32::MAX),
        height: u32::try_from(report.height).unwrap_or(u32::MAX),
        depth: u32::try_from(report.depth).unwrap_or(u32::MAX),
        stride_x: report.stride_x,
        stride_y: report.stride_y,
        stride_z: report.stride_z,
        capacity: u32::try_from(report.capacity).unwrap_or(u32::MAX),
        last_physical_index: u32::try_from(report.last_physical_index).unwrap_or(u32::MAX),
        written_elements: u32::try_from(report.width * report.height * report.depth)
            .unwrap_or(u32::MAX),
        untouched_elements: u32::try_from(report.untouched_elements).unwrap_or(u32::MAX),
    }
}

fn vulkan_error_status(error: &RuntimeError) -> i32 {
    match error {
        RuntimeError::Loader(_) => -20,
        RuntimeError::Codegen(_) => -21,
        RuntimeError::Vulkan { .. } => -22,
        RuntimeError::NoComputeQueue => -23,
        RuntimeError::DescriptorContract(_) => -24,
    }
}

/// Executes the reflected f32 index-1 kernel through the native C ABI.
///
/// This first ABI slice deliberately requires two elements and writes only
/// `output[1]`; it is a contract probe, not yet a general vector kernel.
///
/// # Safety
///
/// The caller must provide non-null, writable/readable `f32` buffers with at
/// least `length` elements. The buffers must remain valid for the duration of
/// the call and may not alias memory that is concurrently accessed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_vk_f32_add_one(
    input: *const f32,
    output: *mut f32,
    length: u32,
) -> VulkanF32DispatchResult {
    if input.is_null() || output.is_null() {
        return VulkanF32DispatchResult {
            status: -10,
            output_value: 0.0,
            physical_device_count: 0,
        };
    }
    if length < 2 {
        return VulkanF32DispatchResult {
            status: -11,
            output_value: 0.0,
            physical_device_count: 0,
        };
    }
    let input_value = unsafe { *input.add(1) };
    match run_f32_queue_smoke_with_values(input_value, 1.0) {
        Ok(report) => {
            unsafe { *output.add(1) = report.data_kernel_value };
            VulkanF32DispatchResult {
                status: 0,
                output_value: report.data_kernel_value,
                physical_device_count: u32::try_from(report.physical_device_count)
                    .unwrap_or(u32::MAX),
            }
        }
        Err(error) => VulkanF32DispatchResult {
            status: vulkan_error_status(&error),
            output_value: 0.0,
            physical_device_count: 0,
        },
    }
}

/// Executes the bounds-safe runtime-length `f32` add kernel through the
/// native C ABI. It writes `output[i] = input[i] + 1.0` for every caller
/// element and never accesses capacity beyond the uploaded `length` value.
///
/// # Safety
///
/// The caller must provide non-null, readable `input` and writable `output`
/// buffers with at least `length` f32 elements. The buffers must remain valid
/// and must not be concurrently accessed for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_vk_f32_add_one_array(
    input: *const f32,
    output: *mut f32,
    length: u32,
) -> VulkanF32ArrayDispatchResult {
    let make_result = |status| VulkanF32ArrayDispatchResult {
        status,
        output_checksum: 0.0,
        physical_device_count: 0,
        processed_length: 0,
    };
    if input.is_null() || output.is_null() {
        return make_result(-32);
    }
    let length_usize = length as usize;
    if length_usize == 0 || length_usize > GLOBAL_DYNAMIC_CAPACITY {
        return make_result(-33);
    }
    let input_values = unsafe { std::slice::from_raw_parts(input, length_usize) };
    if input_values.iter().any(|value| !value.is_finite()) {
        return make_result(-34);
    }
    match run_global_dynamic_f32_queue_with_values(input_values) {
        Ok((report, output_values))
            if report.differential_execution == "passed"
                && report.residency_execution == "passed" =>
        {
            unsafe {
                std::ptr::copy_nonoverlapping(output_values.as_ptr(), output, length_usize);
            }
            VulkanF32ArrayDispatchResult {
                status: 0,
                output_checksum: report.output_checksum,
                physical_device_count: u32::try_from(report.physical_device_count)
                    .unwrap_or(u32::MAX),
                processed_length: report.runtime_length,
            }
        }
        Ok(_) => make_result(-35),
        Err(error) => make_result(vulkan_error_status(&error)),
    }
}

/// Executes the dynamic-length u32 array kernel through the native C ABI.
///
/// The runtime accepts `1..=128` elements, dispatches the required number of
/// 64-lane workgroups and writes `output[i] = input[i] + 1` for every caller
/// element. The third GPU resource carries the runtime length.
///
/// # Safety
///
/// The caller must provide non-null, readable `input` and writable `output`
/// buffers with at least `length` u32 elements. The buffers must remain valid
/// and must not be concurrently accessed for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_vk_u32_add_one_array(
    input: *const u32,
    output: *mut u32,
    length: u32,
) -> VulkanU32ArrayDispatchResult {
    if input.is_null() || output.is_null() {
        return VulkanU32ArrayDispatchResult {
            status: -30,
            output_checksum: 0,
            physical_device_count: 0,
            processed_length: 0,
        };
    }
    let length_usize = length as usize;
    if length_usize == 0 || length_usize > GLOBAL_DYNAMIC_CAPACITY {
        return VulkanU32ArrayDispatchResult {
            status: -31,
            output_checksum: 0,
            physical_device_count: 0,
            processed_length: 0,
        };
    }
    let input_values = unsafe { std::slice::from_raw_parts(input, length_usize) };
    match run_global_dynamic_u32_queue_with_values(input_values) {
        Ok((report, output_values)) => {
            unsafe {
                std::ptr::copy_nonoverlapping(output_values.as_ptr(), output, length_usize);
            }
            VulkanU32ArrayDispatchResult {
                status: 0,
                output_checksum: report.output_checksum,
                physical_device_count: u32::try_from(report.physical_device_count)
                    .unwrap_or(u32::MAX),
                processed_length: report.runtime_length,
            }
        }
        Err(error) => VulkanU32ArrayDispatchResult {
            status: vulkan_error_status(&error),
            output_checksum: 0,
            physical_device_count: 0,
            processed_length: 0,
        },
    }
}

/// Executes a parametrized runtime-length `u32` binary kernel through the
/// native Vulkan C ABI. Operation codes are: `0 Add`, `1 Subtract`, `2
/// Multiply`, `3 Divide`, `4 Remainder`, `5 BitAnd`, `6 BitOr`, `7 BitXor`,
/// `8 ShiftLeft`, `9 ShiftRight`. The output pointer is borrowed only for the
/// duration of this call.
///
/// # Safety
///
/// The caller must provide non-null, readable `input` and writable `output`
/// buffers with at least `length` u32 elements. The buffers must remain valid
/// and must not be concurrently accessed for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_vk_u32_binary_array(
    input: *const u32,
    output: *mut u32,
    length: u32,
    operation: u32,
    operand: u32,
) -> VulkanU32BinaryArrayDispatchResult {
    let make_result = |status: i32| VulkanU32BinaryArrayDispatchResult {
        status,
        output_checksum: 0,
        physical_device_count: 0,
        processed_length: 0,
        operation,
        operand,
    };
    if input.is_null() || output.is_null() {
        return make_result(-50);
    }
    let operation_kind = match GlobalDynamicOperation::from_abi(operation) {
        Some(operation_kind) => operation_kind,
        None => return make_result(-51),
    };
    if operation_kind.validate_operand(operand).is_err() {
        return make_result(-52);
    }
    let length_usize = length as usize;
    if length_usize == 0 || length_usize > GLOBAL_DYNAMIC_CAPACITY {
        return make_result(-53);
    }
    let input_values = unsafe { std::slice::from_raw_parts(input, length_usize) };
    match run_global_dynamic_u32_binary_queue_with_values(
        input_values,
        operation_kind.jir_op(),
        operand,
    ) {
        Ok((report, output_values)) => {
            unsafe {
                std::ptr::copy_nonoverlapping(output_values.as_ptr(), output, length_usize);
            }
            VulkanU32BinaryArrayDispatchResult {
                status: 0,
                output_checksum: report.output_checksum,
                physical_device_count: u32::try_from(report.physical_device_count)
                    .unwrap_or(u32::MAX),
                processed_length: report.runtime_length,
                operation: operation_kind.abi_code(),
                operand,
            }
        }
        Err(error) => VulkanU32BinaryArrayDispatchResult {
            status: vulkan_error_status(&error),
            output_checksum: 0,
            physical_device_count: 0,
            processed_length: 0,
            operation: operation_kind.abi_code(),
            operand,
        },
    }
}

/// Executes a parametrized 3D affine-stride u32 write through the native
/// Vulkan C ABI. The kernel writes `value` at each valid physical index
/// `x*stride_x + y*stride_y + z*stride_z` and leaves other capacity elements
/// zero. The caller's output pointer is borrowed until the function returns.
///
/// # Safety
///
/// The caller must provide a non-null writable buffer with at least `capacity`
/// u32 elements. The buffer must remain valid and must not be concurrently
/// accessed for the duration of the call. Metadata must describe a layout
/// whose final affine index is within `capacity`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_vk_u32_3d_strided_write(
    output: *mut u32,
    width: u32,
    height: u32,
    depth: u32,
    stride_x: u32,
    stride_y: u32,
    stride_z: u32,
    capacity: u32,
    value: u32,
) -> VulkanU32Tensor3DDispatchResult {
    let make_result = |status: i32| {
        tensor3d_failure_result(
            status, width, height, depth, stride_x, stride_y, stride_z, capacity,
        )
    };
    if output.is_null() {
        return make_result(-40);
    }
    let config = Global3dStridedWriteConfig {
        width: width as usize,
        height: height as usize,
        depth: depth as usize,
        stride_x,
        stride_y,
        stride_z,
        capacity: capacity as usize,
        value,
        workgroup_size: THREE_D_STRIDED_WRITE_WORKGROUP_SIZE,
    };
    let config = match config.validate() {
        Ok(config) => config,
        Err(_) => return make_result(-41),
    };
    match run_global_3d_strided_write_with_config(config) {
        Ok(execution) => {
            let report = execution.report;
            if report.data_kernel_execution != "passed"
                || report.differential_execution != "passed"
                || report.residency_execution != "passed"
                || report.timeline_execution != "passed"
            {
                return make_result(-43);
            }
            unsafe {
                std::ptr::copy_nonoverlapping(
                    execution.output_values.as_ptr(),
                    output,
                    config.capacity,
                );
            }
            tensor3d_result_from_report(&report)
        }
        Err(error) => make_result(vulkan_error_status(&error)),
    }
}

/// Starts a worker-backed 3D affine-stride dispatch without blocking the
/// calling thread. The output pointer is borrowed until complete/release.
///
/// # Safety
///
/// `output` must point to at least `capacity` writable `u32` elements and must
/// remain valid and exclusively owned by the caller until the handle is
/// completed or released. The caller must consume the returned handle exactly
/// once with complete or release.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_vk_u32_3d_strided_write_async(
    output: *mut u32,
    width: u32,
    height: u32,
    depth: u32,
    stride_x: u32,
    stride_y: u32,
    stride_z: u32,
    capacity: u32,
    value: u32,
) -> VulkanU32Tensor3DAsyncBeginResult {
    if output.is_null() {
        return VulkanU32Tensor3DAsyncBeginResult {
            status: -40,
            handle: std::ptr::null_mut(),
        };
    }
    let config = Global3dStridedWriteConfig {
        width: width as usize,
        height: height as usize,
        depth: depth as usize,
        stride_x,
        stride_y,
        stride_z,
        capacity: capacity as usize,
        value,
        workgroup_size: THREE_D_STRIDED_WRITE_WORKGROUP_SIZE,
    };
    let config = match config.validate() {
        Ok(config) => config,
        Err(_) => {
            return VulkanU32Tensor3DAsyncBeginResult {
                status: -41,
                handle: std::ptr::null_mut(),
            };
        }
    };
    let template = tensor3d_failure_result(
        -62, width, height, depth, stride_x, stride_y, stride_z, capacity,
    );
    // Raw pointers are intentionally represented as an integer while crossing
    // the worker boundary; the caller's safety contract keeps the allocation
    // alive and exclusive until the handle is consumed.
    let output_address = output as usize;
    let worker = thread::Builder::new()
        .name("jadren-vulkan-3d-async".to_owned())
        .spawn(move || {
            let execution = match run_global_3d_strided_write_with_config(config) {
                Ok(execution) => execution,
                Err(error) => {
                    return tensor3d_failure_result(
                        vulkan_error_status(&error),
                        width,
                        height,
                        depth,
                        stride_x,
                        stride_y,
                        stride_z,
                        capacity,
                    );
                }
            };
            let report = execution.report;
            if report.data_kernel_execution != "passed"
                || report.differential_execution != "passed"
                || report.residency_execution != "passed"
                || report.timeline_execution != "passed"
            {
                return tensor3d_failure_result(
                    -43, width, height, depth, stride_x, stride_y, stride_z, capacity,
                );
            }
            unsafe {
                std::ptr::copy_nonoverlapping(
                    execution.output_values.as_ptr(),
                    output_address as *mut u32,
                    config.capacity,
                );
            }
            tensor3d_result_from_report(&report)
        });
    let worker = match worker {
        Ok(worker) => worker,
        Err(_) => {
            return VulkanU32Tensor3DAsyncBeginResult {
                status: -62,
                handle: std::ptr::null_mut(),
            };
        }
    };
    let handle = Box::new(VulkanU32Tensor3DAsyncHandle {
        worker: Some(worker),
        template,
    });
    VulkanU32Tensor3DAsyncBeginResult {
        status: 0,
        handle: Box::into_raw(handle).cast(),
    }
}

/// Returns one when the worker has finished, zero while it is still running,
/// or a negative stable status for a null handle.
///
/// # Safety
///
/// `handle` must be a live handle returned by the async begin function, or
/// null. Polling does not consume the handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_vk_u32_3d_strided_write_async_poll(handle: *mut c_void) -> i32 {
    if handle.is_null() {
        return -60;
    }
    let handle = unsafe { &*(handle.cast::<VulkanU32Tensor3DAsyncHandle>()) };
    match handle.worker.as_ref() {
        Some(worker) => i32::from(worker.is_finished()),
        None => 1,
    }
}

/// Consumes the handle, joins the worker and returns the completed dispatch
/// metadata. A worker panic is converted to stable status `-61`.
///
/// # Safety
///
/// `handle` must be a live handle returned by the async begin function, or
/// null. A non-null handle is consumed exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_vk_u32_3d_strided_write_async_complete(
    handle: *mut c_void,
) -> VulkanU32Tensor3DDispatchResult {
    if handle.is_null() {
        return tensor3d_failure_result(-60, 0, 0, 0, 0, 0, 0, 0);
    }
    let mut handle = unsafe { Box::from_raw(handle.cast::<VulkanU32Tensor3DAsyncHandle>()) };
    let worker = handle.worker.take().expect("async handle worker present");
    match worker.join() {
        Ok(result) => result,
        Err(_) => {
            let mut result = handle.template;
            result.status = -61;
            result
        }
    }
}

/// Consumes the handle and joins the worker. Release is safe to call when the
/// caller no longer needs the result, but it may block until the GPU work is
/// complete so the borrowed output lifetime remains sound.
///
/// # Safety
///
/// `handle` must be a live handle returned by the async begin function, or
/// null. A non-null handle is consumed exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_vk_u32_3d_strided_write_async_release(handle: *mut c_void) -> i32 {
    if handle.is_null() {
        return -60;
    }
    let mut handle = unsafe { Box::from_raw(handle.cast::<VulkanU32Tensor3DAsyncHandle>()) };
    let worker = handle.worker.take().expect("async handle worker present");
    if worker.join().is_ok() { 0 } else { -61 }
}

fn run_f32_on_instance(
    instance: &Instance,
    input_value: f32,
    addend_value: f32,
) -> Result<F32QueueSmokeReport, RuntimeError> {
    let physical_devices =
        unsafe { instance.enumerate_physical_devices() }.map_err(|code| RuntimeError::Vulkan {
            operation: "enumerate_f32_physical_devices",
            code,
        })?;
    let (physical_device, queue_family_index) = physical_devices
        .iter()
        .find_map(|physical_device| {
            let families =
                unsafe { instance.get_physical_device_queue_family_properties(*physical_device) };
            families
                .iter()
                .enumerate()
                .find(|(_, properties)| {
                    properties.queue_count > 0
                        && properties.queue_flags.contains(vk::QueueFlags::COMPUTE)
                })
                .map(|(index, _)| (*physical_device, index as u32))
        })
        .ok_or(RuntimeError::NoComputeQueue)?;
    let properties = unsafe { instance.get_physical_device_properties(physical_device) };
    let selected_device = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    let priorities = [1.0_f32];
    let queue_info = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family_index)
        .queue_priorities(&priorities);
    let mut available_vulkan12 = vk::PhysicalDeviceVulkan12Features::default();
    let mut available_features =
        vk::PhysicalDeviceFeatures2::default().push_next(&mut available_vulkan12);
    unsafe { instance.get_physical_device_features2(physical_device, &mut available_features) };
    if available_vulkan12.timeline_semaphore == vk::FALSE {
        return Err(RuntimeError::DescriptorContract(
            "selected Vulkan device does not expose timelineSemaphore for 3D affine-stride dispatch"
                .to_owned(),
        ));
    }
    let mut enabled_vulkan12 =
        vk::PhysicalDeviceVulkan12Features::default().timeline_semaphore(true);
    let device_info = vk::DeviceCreateInfo::default()
        .queue_create_infos(std::slice::from_ref(&queue_info))
        .push_next(&mut enabled_vulkan12);
    let device =
        unsafe { instance.create_device(physical_device, &device_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_f32_device",
                code,
            }
        })?;
    let result = run_f32_on_device(
        instance,
        &device,
        physical_device,
        queue_family_index,
        physical_devices.len(),
        selected_device,
        F32KernelValues {
            input: input_value,
            addend: addend_value,
        },
    );
    unsafe {
        let _ = device.device_wait_idle();
        device.destroy_device(None);
    }
    result
}

fn run_f32_on_device(
    instance: &Instance,
    device: &ash::Device,
    physical_device: vk::PhysicalDevice,
    queue_family_index: u32,
    physical_device_count: usize,
    selected_device: String,
    values: F32KernelValues,
) -> Result<F32QueueSmokeReport, RuntimeError> {
    let queue = unsafe { device.get_device_queue(queue_family_index, 0) };
    let kernel = storage_dynamic_index_fadd_fixture_module(values.addend.to_bits());
    let resources = reflect_resources(&kernel, FunctionId::new(0)).map_err(|error| {
        RuntimeError::DescriptorContract(format!("f32 JIR reflection failed: {error}"))
    })?;
    if resources.len() != 3 {
        return Err(RuntimeError::DescriptorContract(
            "f32 kernel must expose input/output/index resources".to_owned(),
        ));
    }
    let descriptor_bindings = resources
        .iter()
        .map(|resource| {
            if resource.address_space != AddressSpace::Storage {
                return Err(RuntimeError::DescriptorContract(format!(
                    "f32 resource `{}` is not storage address space",
                    resource.name
                )));
            }
            Ok(vk::DescriptorSetLayoutBinding::default()
                .binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let spirv = emit_storage_dynamic_index_fadd_from_jir(
        &kernel,
        FunctionId::new(0),
        SpirvOptions::new([1, 1, 1]).map_err(|error| RuntimeError::Codegen(error.to_string()))?,
    )
    .map_err(|error| RuntimeError::Codegen(error.to_string()))?;
    let shader_info = vk::ShaderModuleCreateInfo::default().code(&spirv);
    let shader_module =
        unsafe { device.create_shader_module(&shader_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_f32_shader_module",
                code,
            }
        })?;
    let entry_name = CString::new(kernel.functions[0].name.as_str())
        .expect("validated f32 JIR entry name has no NUL");
    let stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(shader_module)
        .name(&entry_name);
    let storage_buffer_info = vk::BufferCreateInfo::default()
        .size(STORAGE_KERNEL_BUFFER_SIZE)
        .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let storage_buffer =
        unsafe { device.create_buffer(&storage_buffer_info, None) }.map_err(|code| {
            unsafe { device.destroy_shader_module(shader_module, None) };
            RuntimeError::Vulkan {
                operation: "create_f32_storage_buffer",
                code,
            }
        })?;
    let requirements = unsafe { device.get_buffer_memory_requirements(storage_buffer) };
    let memory_properties =
        unsafe { instance.get_physical_device_memory_properties(physical_device) };
    let memory_type_index = (0..memory_properties.memory_type_count)
        .find(|index| {
            let compatible = requirements.memory_type_bits & (1_u32 << index) != 0;
            compatible
                && memory_properties.memory_types[*index as usize]
                    .property_flags
                    .contains(
                        vk::MemoryPropertyFlags::HOST_VISIBLE
                            | vk::MemoryPropertyFlags::HOST_COHERENT,
                    )
        })
        .ok_or_else(|| {
            RuntimeError::Loader(
                "no host-visible coherent Vulkan memory type for f32 smoke".to_owned(),
            )
        })?;
    let allocation_info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_type_index);
    let storage_memory =
        unsafe { device.allocate_memory(&allocation_info, None) }.map_err(|code| {
            unsafe { device.destroy_buffer(storage_buffer, None) };
            RuntimeError::Vulkan {
                operation: "allocate_f32_storage_memory",
                code,
            }
        })?;
    unsafe { device.bind_buffer_memory(storage_buffer, storage_memory, 0) }.map_err(|code| {
        unsafe {
            device.free_memory(storage_memory, None);
            device.destroy_buffer(storage_buffer, None);
        }
        RuntimeError::Vulkan {
            operation: "bind_f32_storage_memory",
            code,
        }
    })?;
    let (mut resource_table, resource_id, access_token) =
        begin_storage_scope(STORAGE_KERNEL_BUFFER_SIZE)?;
    let input_byte_offset =
        STORAGE_KERNEL_INPUT_BASE_OFFSET + u64::from(STORAGE_KERNEL_INPUT_INDEX) * 4;
    let output_byte_offset =
        STORAGE_KERNEL_OUTPUT_BASE_OFFSET + u64::from(STORAGE_KERNEL_OUTPUT_INDEX) * 4;
    unsafe {
        for (offset, value, operation) in [
            (
                input_byte_offset,
                values.input,
                "map_f32_storage_memory_for_upload",
            ),
            (
                output_byte_offset,
                0.0_f32,
                "map_f32_storage_memory_for_output_clear",
            ),
        ] {
            let mapped = device
                .map_memory(storage_memory, offset, 4, vk::MemoryMapFlags::empty())
                .map_err(|code| RuntimeError::Vulkan { operation, code })?;
            std::ptr::write_unaligned(mapped.cast::<f32>(), value);
            device.unmap_memory(storage_memory);
        }
        let mapped = device
            .map_memory(
                storage_memory,
                STORAGE_KERNEL_INDEX_BASE_OFFSET,
                4,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_f32_storage_memory_for_index_upload",
                code,
            })?;
        std::ptr::write_unaligned(mapped.cast::<u32>(), STORAGE_KERNEL_INDEX_VALUE);
        device.unmap_memory(storage_memory);
    }
    let descriptor_layout_info =
        vk::DescriptorSetLayoutCreateInfo::default().bindings(&descriptor_bindings);
    let descriptor_set_layout =
        unsafe { device.create_descriptor_set_layout(&descriptor_layout_info, None) }.map_err(
            |code| RuntimeError::Vulkan {
                operation: "create_f32_descriptor_set_layout",
                code,
            },
        )?;
    let descriptor_pool_size = vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(u32::try_from(resources.len()).expect("resource count is bounded"));
    let descriptor_pool_info = vk::DescriptorPoolCreateInfo::default()
        .max_sets(1)
        .pool_sizes(std::slice::from_ref(&descriptor_pool_size));
    let descriptor_pool = unsafe { device.create_descriptor_pool(&descriptor_pool_info, None) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "create_f32_descriptor_pool",
            code,
        })?;
    let descriptor_set_allocate_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(descriptor_pool)
        .set_layouts(std::slice::from_ref(&descriptor_set_layout));
    let descriptor_set = unsafe { device.allocate_descriptor_sets(&descriptor_set_allocate_info) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "allocate_f32_descriptor_set",
            code,
        })?[0];
    let descriptor_buffer_infos = [
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(STORAGE_KERNEL_INPUT_BASE_OFFSET)
            .range(8),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(STORAGE_KERNEL_OUTPUT_BASE_OFFSET)
            .range(8),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(STORAGE_KERNEL_INDEX_BASE_OFFSET)
            .range(4),
    ];
    let descriptor_writes = resources
        .iter()
        .enumerate()
        .map(|(index, resource)| {
            vk::WriteDescriptorSet::default()
                .dst_set(descriptor_set)
                .dst_binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(std::slice::from_ref(&descriptor_buffer_infos[index]))
        })
        .collect::<Vec<_>>();
    unsafe { device.update_descriptor_sets(&descriptor_writes, &[]) };
    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(std::slice::from_ref(&descriptor_set_layout));
    let pipeline_layout =
        unsafe { device.create_pipeline_layout(&layout_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_f32_pipeline_layout",
                code,
            }
        })?;
    let pipeline_info = vk::ComputePipelineCreateInfo::default()
        .stage(stage)
        .layout(pipeline_layout);
    let pipeline = match unsafe {
        device.create_compute_pipelines(
            vk::PipelineCache::null(),
            std::slice::from_ref(&pipeline_info),
            None,
        )
    } {
        Ok(pipelines) => pipelines[0],
        Err((_, code)) => {
            unsafe {
                device.destroy_pipeline_layout(pipeline_layout, None);
                device.destroy_shader_module(shader_module, None);
            }
            return Err(RuntimeError::Vulkan {
                operation: "create_f32_compute_pipeline",
                code,
            });
        }
    };
    let pool_info = vk::CommandPoolCreateInfo::default()
        .queue_family_index(queue_family_index)
        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
    let command_pool = unsafe { device.create_command_pool(&pool_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_f32_command_pool",
            code,
        }
    })?;
    let allocation_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let command_buffer =
        unsafe { device.allocate_command_buffers(&allocation_info) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "allocate_f32_command_buffer",
                code,
            }
        })?[0];
    unsafe { device.begin_command_buffer(command_buffer, &vk::CommandBufferBeginInfo::default()) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "begin_f32_command_buffer",
            code,
        })?;
    unsafe {
        device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::COMPUTE, pipeline);
        device.cmd_bind_descriptor_sets(
            command_buffer,
            vk::PipelineBindPoint::COMPUTE,
            pipeline_layout,
            0,
            std::slice::from_ref(&descriptor_set),
            &[],
        );
        device.cmd_dispatch(command_buffer, 1, 1, 1);
    }
    unsafe { device.end_command_buffer(command_buffer) }.map_err(|code| RuntimeError::Vulkan {
        operation: "end_f32_command_buffer",
        code,
    })?;
    let fence =
        unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_f32_fence",
                code,
            }
        })?;
    let submit_info =
        vk::SubmitInfo::default().command_buffers(std::slice::from_ref(&command_buffer));
    unsafe { device.queue_submit(queue, std::slice::from_ref(&submit_info), fence) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "submit_f32_queue",
            code,
        },
    )?;
    unsafe { device.wait_for_fences(std::slice::from_ref(&fence), true, u64::MAX) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "wait_f32_fence",
            code,
        },
    )?;
    let data_kernel_value = unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                output_byte_offset,
                4,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_f32_storage_memory_for_readback",
                code,
            })?;
        let value = std::ptr::read_unaligned(mapped.cast::<f32>());
        device.unmap_memory(storage_memory);
        value
    };
    let cpu_reference_value = values.input + values.addend;
    let differential_passed = compare_f32(
        &[cpu_reference_value],
        &[data_kernel_value],
        DifferentialPolicy::Exact,
    )
    .is_ok();
    let residency_passed =
        resource_table.release(access_token).is_ok() && resource_table.evict(resource_id).is_ok();
    unsafe {
        device.destroy_fence(fence, None);
        device.destroy_command_pool(command_pool, None);
        device.destroy_pipeline(pipeline, None);
        device.destroy_pipeline_layout(pipeline_layout, None);
        device.destroy_shader_module(shader_module, None);
        device.destroy_descriptor_pool(descriptor_pool, None);
        device.destroy_descriptor_set_layout(descriptor_set_layout, None);
        device.free_memory(storage_memory, None);
        device.destroy_buffer(storage_buffer, None);
    }
    Ok(F32QueueSmokeReport {
        schema: "jadren-vulkan-f32-queue-smoke-0.1",
        physical_device_count,
        selected_device,
        queue_family_index,
        queue_execution: "passed",
        descriptor_setup: "passed",
        resource_binding_count: resources.len(),
        pipeline_execution: "passed",
        fence_execution: "passed",
        data_kernel_execution: if differential_passed {
            "passed"
        } else {
            "failed"
        },
        differential_execution: if differential_passed {
            "passed"
        } else {
            "failed"
        },
        residency_execution: if residency_passed { "passed" } else { "failed" },
        data_kernel_value,
        cpu_reference_value,
        data_kernel_input: values.input,
        data_kernel_addend: values.addend,
        data_kernel_index: STORAGE_KERNEL_INDEX_VALUE,
    })
}

fn run_global_dynamic_on_instance(
    instance: &Instance,
    input_values: &[u32],
    operation: GlobalDynamicOperation,
    operand: u32,
) -> Result<GlobalDynamicU32Execution, RuntimeError> {
    operation
        .validate_operand(operand)
        .map_err(|message| RuntimeError::DescriptorContract(message.to_owned()))?;
    let physical_devices =
        unsafe { instance.enumerate_physical_devices() }.map_err(|code| RuntimeError::Vulkan {
            operation: "enumerate_dynamic_length_physical_devices",
            code,
        })?;
    let (physical_device, queue_family_index) = physical_devices
        .iter()
        .find_map(|physical_device| {
            let families =
                unsafe { instance.get_physical_device_queue_family_properties(*physical_device) };
            families
                .iter()
                .enumerate()
                .find(|(_, properties)| {
                    properties.queue_count > 0
                        && properties.queue_flags.contains(vk::QueueFlags::COMPUTE)
                })
                .map(|(index, _)| (*physical_device, index as u32))
        })
        .ok_or(RuntimeError::NoComputeQueue)?;
    let properties = unsafe { instance.get_physical_device_properties(physical_device) };
    let selected_device = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    let priorities = [1.0_f32];
    let queue_info = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family_index)
        .queue_priorities(&priorities);
    let device_info =
        vk::DeviceCreateInfo::default().queue_create_infos(std::slice::from_ref(&queue_info));
    let device =
        unsafe { instance.create_device(physical_device, &device_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_dynamic_length_device",
                code,
            }
        })?;
    let result = run_global_dynamic_on_device(GlobalDynamicDeviceContext {
        instance,
        device: &device,
        physical_device,
        queue_family_index,
        physical_device_count: physical_devices.len(),
        selected_device,
        input_values,
        operation,
        operand,
    });
    unsafe {
        let _ = device.device_wait_idle();
        device.destroy_device(None);
    }
    result
}

fn run_global_dynamic_f32_on_instance(
    instance: &Instance,
    input_values: &[f32],
    artifact: Option<&SpirvArtifact>,
    operation: F32ArithmeticOp,
) -> Result<GlobalDynamicF32Execution, RuntimeError> {
    let physical_devices =
        unsafe { instance.enumerate_physical_devices() }.map_err(|code| RuntimeError::Vulkan {
            operation: "enumerate_dynamic_f32_physical_devices",
            code,
        })?;
    let (physical_device, queue_family_index) = physical_devices
        .iter()
        .find_map(|physical_device| {
            let families =
                unsafe { instance.get_physical_device_queue_family_properties(*physical_device) };
            families
                .iter()
                .enumerate()
                .find(|(_, family)| {
                    family.queue_count > 0 && family.queue_flags.contains(vk::QueueFlags::COMPUTE)
                })
                .map(|(index, _)| (*physical_device, index as u32))
        })
        .ok_or(RuntimeError::NoComputeQueue)?;
    let properties = unsafe { instance.get_physical_device_properties(physical_device) };
    let selected_device = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    let priorities = [1.0_f32];
    let queue_info = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family_index)
        .queue_priorities(&priorities);
    let device_info =
        vk::DeviceCreateInfo::default().queue_create_infos(std::slice::from_ref(&queue_info));
    let device =
        unsafe { instance.create_device(physical_device, &device_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_dynamic_f32_device",
                code,
            }
        })?;
    let result = run_global_dynamic_f32_on_device(
        instance,
        &device,
        physical_device,
        queue_family_index,
        physical_devices.len(),
        selected_device,
        input_values,
        artifact,
        operation,
    );
    unsafe {
        let _ = device.device_wait_idle();
        device.destroy_device(None);
    }
    result
}

fn run_global_dynamic_f32_vector_on_instance(
    instance: &Instance,
    input_values: &[[f32; 4]],
    artifact: &SpirvArtifact,
    operation: F32ArithmeticOp,
) -> Result<GlobalDynamicF32VectorExecution, RuntimeError> {
    let physical_devices =
        unsafe { instance.enumerate_physical_devices() }.map_err(|code| RuntimeError::Vulkan {
            operation: "enumerate_dynamic_f32x4_physical_devices",
            code,
        })?;
    let (physical_device, queue_family_index) = physical_devices
        .iter()
        .find_map(|physical_device| {
            let families =
                unsafe { instance.get_physical_device_queue_family_properties(*physical_device) };
            families
                .iter()
                .enumerate()
                .find(|(_, family)| {
                    family.queue_count > 0 && family.queue_flags.contains(vk::QueueFlags::COMPUTE)
                })
                .map(|(index, _)| (*physical_device, index as u32))
        })
        .ok_or(RuntimeError::NoComputeQueue)?;
    let properties = unsafe { instance.get_physical_device_properties(physical_device) };
    let selected_device = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    let priorities = [1.0_f32];
    let queue_info = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family_index)
        .queue_priorities(&priorities);
    let device_info =
        vk::DeviceCreateInfo::default().queue_create_infos(std::slice::from_ref(&queue_info));
    let device =
        unsafe { instance.create_device(physical_device, &device_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_dynamic_f32x4_device",
                code,
            }
        })?;
    let result = run_global_dynamic_f32_vector_on_device(
        instance,
        &device,
        physical_device,
        queue_family_index,
        physical_devices.len(),
        selected_device,
        input_values,
        artifact,
        operation,
    );
    unsafe {
        let _ = device.device_wait_idle();
        device.destroy_device(None);
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn run_global_dynamic_f32_vector_on_device(
    instance: &Instance,
    device: &ash::Device,
    physical_device: vk::PhysicalDevice,
    queue_family_index: u32,
    physical_device_count: usize,
    selected_device: String,
    input_values: &[[f32; 4]],
    artifact: &SpirvArtifact,
    operation: F32ArithmeticOp,
) -> Result<GlobalDynamicF32VectorExecution, RuntimeError> {
    let queue = unsafe { device.get_device_queue(queue_family_index, 0) };
    validate_spirv_artifact_contract(artifact)
        .map_err(|error| RuntimeError::Codegen(error.to_string()))?;
    let operand = f32_operation_operand(operation);
    let kernel = storage_global_dynamic_index_vector_f32_binary_fixture_module(operand, operation);
    let resources = reflect_resources(&kernel, FunctionId::new(0)).map_err(|error| {
        RuntimeError::DescriptorContract(format!("dynamic f32x4 JIR reflection failed: {error}"))
    })?;
    if resources.len() != 3
        || resources
            .iter()
            .any(|resource| resource.address_space != AddressSpace::Storage)
        || resources[0].element_stride != Some(16)
        || resources[1].element_stride != Some(16)
        || resources[2].element_stride != Some(4)
    {
        return Err(RuntimeError::DescriptorContract(
            "dynamic f32x4 kernel must expose two 16-byte vectors and one u32 length".to_owned(),
        ));
    }
    if artifact.entry_name != kernel.functions[0].name
        || artifact.workgroup_size != [GLOBAL_DYNAMIC_VECTOR_WORKGROUP_SIZE, 1, 1]
        || artifact.resources != resources
    {
        return Err(RuntimeError::DescriptorContract(
            "dynamic f32x4 artifact metadata does not match the JIR fixture".to_owned(),
        ));
    }
    let spirv = &artifact.words;
    let descriptor_bindings = resources
        .iter()
        .map(|resource| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE)
        })
        .collect::<Vec<_>>();
    let shader_info = vk::ShaderModuleCreateInfo::default().code(spirv);
    let shader_module =
        unsafe { device.create_shader_module(&shader_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_dynamic_f32x4_shader_module",
                code,
            }
        })?;
    let storage_buffer_info = vk::BufferCreateInfo::default()
        .size(GLOBAL_DYNAMIC_VECTOR_BUFFER_SIZE)
        .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let storage_buffer =
        unsafe { device.create_buffer(&storage_buffer_info, None) }.map_err(|code| {
            unsafe { device.destroy_shader_module(shader_module, None) };
            RuntimeError::Vulkan {
                operation: "create_dynamic_f32x4_storage_buffer",
                code,
            }
        })?;
    let requirements = unsafe { device.get_buffer_memory_requirements(storage_buffer) };
    let memory_properties =
        unsafe { instance.get_physical_device_memory_properties(physical_device) };
    let memory_type_index = (0..memory_properties.memory_type_count)
        .find(|index| {
            requirements.memory_type_bits & (1_u32 << index) != 0
                && memory_properties.memory_types[*index as usize]
                    .property_flags
                    .contains(
                        vk::MemoryPropertyFlags::HOST_VISIBLE
                            | vk::MemoryPropertyFlags::HOST_COHERENT,
                    )
        })
        .ok_or_else(|| RuntimeError::Loader("no coherent memory for dynamic f32x4".to_owned()))?;
    let allocation_info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_type_index);
    let storage_memory =
        unsafe { device.allocate_memory(&allocation_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "allocate_dynamic_f32x4_storage_memory",
                code,
            }
        })?;
    unsafe { device.bind_buffer_memory(storage_buffer, storage_memory, 0) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "bind_dynamic_f32x4_storage_memory",
            code,
        }
    })?;
    let (mut resource_table, resource_id, access_token) =
        begin_storage_scope(GLOBAL_DYNAMIC_VECTOR_BUFFER_SIZE)?;
    let capacity_bytes = (GLOBAL_DYNAMIC_VECTOR_CAPACITY * std::mem::size_of::<[f32; 4]>()) as u64;
    let element_count = input_values.len();
    unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                GLOBAL_DYNAMIC_VECTOR_INPUT_BASE_OFFSET,
                capacity_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_dynamic_f32x4_input",
                code,
            })?;
        for (index, value) in input_values.iter().enumerate() {
            std::ptr::write_unaligned(mapped.cast::<[f32; 4]>().add(index), *value);
        }
        device.unmap_memory(storage_memory);
        let mapped = device
            .map_memory(
                storage_memory,
                GLOBAL_DYNAMIC_VECTOR_OUTPUT_BASE_OFFSET,
                capacity_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_dynamic_f32x4_output_clear",
                code,
            })?;
        for index in 0..GLOBAL_DYNAMIC_VECTOR_CAPACITY {
            std::ptr::write_unaligned(mapped.cast::<[f32; 4]>().add(index), [0.0; 4]);
        }
        device.unmap_memory(storage_memory);
        let mapped = device
            .map_memory(
                storage_memory,
                GLOBAL_DYNAMIC_VECTOR_LENGTH_BASE_OFFSET,
                4,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_dynamic_f32x4_length",
                code,
            })?;
        std::ptr::write_unaligned(mapped.cast::<u32>(), element_count as u32);
        device.unmap_memory(storage_memory);
    }
    let descriptor_set_layout = unsafe {
        device.create_descriptor_set_layout(
            &vk::DescriptorSetLayoutCreateInfo::default().bindings(&descriptor_bindings),
            None,
        )
    }
    .map_err(|code| RuntimeError::Vulkan {
        operation: "create_dynamic_f32x4_descriptor_set_layout",
        code,
    })?;
    let pool_size = vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(3);
    let descriptor_pool = unsafe {
        device.create_descriptor_pool(
            &vk::DescriptorPoolCreateInfo::default()
                .max_sets(1)
                .pool_sizes(std::slice::from_ref(&pool_size)),
            None,
        )
    }
    .map_err(|code| RuntimeError::Vulkan {
        operation: "create_dynamic_f32x4_descriptor_pool",
        code,
    })?;
    let descriptor_set = unsafe {
        device.allocate_descriptor_sets(
            &vk::DescriptorSetAllocateInfo::default()
                .descriptor_pool(descriptor_pool)
                .set_layouts(std::slice::from_ref(&descriptor_set_layout)),
        )
    }
    .map_err(|code| RuntimeError::Vulkan {
        operation: "allocate_dynamic_f32x4_descriptor_set",
        code,
    })?[0];
    let descriptor_buffer_infos = [
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(GLOBAL_DYNAMIC_VECTOR_INPUT_BASE_OFFSET)
            .range(capacity_bytes),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(GLOBAL_DYNAMIC_VECTOR_OUTPUT_BASE_OFFSET)
            .range(capacity_bytes),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(GLOBAL_DYNAMIC_VECTOR_LENGTH_BASE_OFFSET)
            .range(4),
    ];
    let descriptor_writes = resources
        .iter()
        .enumerate()
        .map(|(index, resource)| {
            vk::WriteDescriptorSet::default()
                .dst_set(descriptor_set)
                .dst_binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(std::slice::from_ref(&descriptor_buffer_infos[index]))
        })
        .collect::<Vec<_>>();
    unsafe { device.update_descriptor_sets(&descriptor_writes, &[]) };
    let pipeline_layout = unsafe {
        device.create_pipeline_layout(
            &vk::PipelineLayoutCreateInfo::default()
                .set_layouts(std::slice::from_ref(&descriptor_set_layout)),
            None,
        )
    }
    .map_err(|code| RuntimeError::Vulkan {
        operation: "create_dynamic_f32x4_pipeline_layout",
        code,
    })?;
    let entry_name =
        CString::new(kernel.functions[0].name.as_str()).expect("validated f32x4 entry name");
    let stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(shader_module)
        .name(&entry_name);
    let pipeline_info = vk::ComputePipelineCreateInfo::default()
        .stage(stage)
        .layout(pipeline_layout);
    let pipeline = unsafe {
        device.create_compute_pipelines(
            vk::PipelineCache::null(),
            std::slice::from_ref(&pipeline_info),
            None,
        )
    }
    .map_err(|(_, code)| RuntimeError::Vulkan {
        operation: "create_dynamic_f32x4_compute_pipeline",
        code,
    })?[0];
    let command_pool = unsafe {
        device.create_command_pool(
            &vk::CommandPoolCreateInfo::default()
                .queue_family_index(queue_family_index)
                .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
            None,
        )
    }
    .map_err(|code| RuntimeError::Vulkan {
        operation: "create_dynamic_f32x4_command_pool",
        code,
    })?;
    let command_buffer = unsafe {
        device.allocate_command_buffers(
            &vk::CommandBufferAllocateInfo::default()
                .command_pool(command_pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1),
        )
    }
    .map_err(|code| RuntimeError::Vulkan {
        operation: "allocate_dynamic_f32x4_command_buffer",
        code,
    })?[0];
    unsafe { device.begin_command_buffer(command_buffer, &vk::CommandBufferBeginInfo::default()) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "begin_dynamic_f32x4_command_buffer",
            code,
        })?;
    let dispatch_x = (element_count as u32).div_ceil(GLOBAL_DYNAMIC_VECTOR_WORKGROUP_SIZE);
    unsafe {
        device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::COMPUTE, pipeline);
        device.cmd_bind_descriptor_sets(
            command_buffer,
            vk::PipelineBindPoint::COMPUTE,
            pipeline_layout,
            0,
            std::slice::from_ref(&descriptor_set),
            &[],
        );
        device.cmd_dispatch(command_buffer, dispatch_x, 1, 1);
    }
    unsafe { device.end_command_buffer(command_buffer) }.map_err(|code| RuntimeError::Vulkan {
        operation: "end_dynamic_f32x4_command_buffer",
        code,
    })?;
    let fence =
        unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_dynamic_f32x4_fence",
                code,
            }
        })?;
    let submit_info =
        vk::SubmitInfo::default().command_buffers(std::slice::from_ref(&command_buffer));
    unsafe { device.queue_submit(queue, std::slice::from_ref(&submit_info), fence) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "submit_dynamic_f32x4_queue",
            code,
        },
    )?;
    unsafe { device.wait_for_fences(std::slice::from_ref(&fence), true, u64::MAX) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "wait_dynamic_f32x4_fence",
            code,
        },
    )?;
    let mut output_values = vec![[0.0_f32; 4]; GLOBAL_DYNAMIC_VECTOR_CAPACITY];
    unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                GLOBAL_DYNAMIC_VECTOR_OUTPUT_BASE_OFFSET,
                capacity_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_dynamic_f32x4_readback",
                code,
            })?;
        for (index, value) in output_values.iter_mut().enumerate() {
            *value = std::ptr::read_unaligned(mapped.cast::<[f32; 4]>().add(index));
        }
        device.unmap_memory(storage_memory);
    }
    let expected_values = input_values
        .iter()
        .map(|value| value.map(|lane| apply_f32_operation(lane, operand, operation)))
        .collect::<Vec<_>>();
    let mut expected_capacity = expected_values.clone();
    expected_capacity.resize(GLOBAL_DYNAMIC_VECTOR_CAPACITY, [0.0; 4]);
    let expected_flat = expected_capacity
        .iter()
        .flatten()
        .copied()
        .collect::<Vec<_>>();
    let output_flat = output_values.iter().flatten().copied().collect::<Vec<_>>();
    let differential_passed =
        compare_f32(&expected_flat, &output_flat, DifferentialPolicy::Exact).is_ok();
    let residency_passed =
        resource_table.release(access_token).is_ok() && resource_table.evict(resource_id).is_ok();
    let input_checksum = input_values
        .iter()
        .flatten()
        .map(|value| f64::from(*value))
        .sum();
    let output_checksum = output_values[..element_count]
        .iter()
        .flatten()
        .map(|value| f64::from(*value))
        .sum();
    let expected_checksum = expected_values
        .iter()
        .flatten()
        .map(|value| f64::from(*value))
        .sum();
    let untouched_tail_elements = output_values[element_count..]
        .iter()
        .filter(|value| value.iter().all(|lane| *lane == 0.0))
        .count();
    let first_output = output_values[0];
    let last_output = output_values[element_count - 1];
    unsafe {
        device.destroy_fence(fence, None);
        device.destroy_command_pool(command_pool, None);
        device.destroy_pipeline(pipeline, None);
        device.destroy_pipeline_layout(pipeline_layout, None);
        device.destroy_shader_module(shader_module, None);
        device.destroy_descriptor_pool(descriptor_pool, None);
        device.destroy_descriptor_set_layout(descriptor_set_layout, None);
        device.free_memory(storage_memory, None);
        device.destroy_buffer(storage_buffer, None);
    }
    Ok(GlobalDynamicF32VectorExecution {
        report: GlobalDynamicF32VectorQueueSmokeReport {
            schema: "jadren-vulkan-global-dynamic-f32x4-artifact-queue-smoke-0.1",
            entry_name: kernel.functions[0].name.clone(),
            operation: f32_operation_name(operation),
            operand,
            execution_path: "spirv-artifact",
            spirv_word_count: spirv.len(),
            spirv_word_hash: stable_spirv_word_hash(spirv),
            physical_device_count,
            selected_device,
            queue_family_index,
            resource_binding_count: resources.len(),
            element_count,
            capacity: GLOBAL_DYNAMIC_VECTOR_CAPACITY,
            dispatch_x,
            runtime_length: element_count as u32,
            input_checksum,
            output_checksum,
            expected_checksum,
            first_output,
            last_output,
            untouched_tail_elements,
            differential_execution: if differential_passed {
                "passed"
            } else {
                "failed"
            },
            residency_execution: if residency_passed { "passed" } else { "failed" },
        },
        output_values,
    })
}

fn run_global_dynamic_f32_vector_lanes_on_instance(
    instance: &Instance,
    input_values: &[Vec<f32>],
    lane_count: usize,
    artifact: &SpirvArtifact,
    operation: F32ArithmeticOp,
) -> Result<GlobalDynamicF32VectorLanesExecution, RuntimeError> {
    let physical_devices =
        unsafe { instance.enumerate_physical_devices() }.map_err(|code| RuntimeError::Vulkan {
            operation: "enumerate_dynamic_f32_vector_lanes_physical_devices",
            code,
        })?;
    let (physical_device, queue_family_index) = physical_devices
        .iter()
        .find_map(|physical_device| {
            let families =
                unsafe { instance.get_physical_device_queue_family_properties(*physical_device) };
            families
                .iter()
                .enumerate()
                .find(|(_, family)| {
                    family.queue_count > 0 && family.queue_flags.contains(vk::QueueFlags::COMPUTE)
                })
                .map(|(index, _)| (*physical_device, index as u32))
        })
        .ok_or(RuntimeError::NoComputeQueue)?;
    let properties = unsafe { instance.get_physical_device_properties(physical_device) };
    let selected_device = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    let priorities = [1.0_f32];
    let queue_info = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family_index)
        .queue_priorities(&priorities);
    let device_info =
        vk::DeviceCreateInfo::default().queue_create_infos(std::slice::from_ref(&queue_info));
    let device =
        unsafe { instance.create_device(physical_device, &device_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_dynamic_f32_vector_lanes_device",
                code,
            }
        })?;
    let result = run_global_dynamic_f32_vector_lanes_on_device(
        instance,
        &device,
        physical_device,
        queue_family_index,
        physical_devices.len(),
        selected_device,
        input_values,
        lane_count,
        artifact,
        operation,
    );
    unsafe {
        let _ = device.device_wait_idle();
        device.destroy_device(None);
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn run_global_dynamic_f32_vector_lanes_on_device(
    instance: &Instance,
    device: &ash::Device,
    physical_device: vk::PhysicalDevice,
    queue_family_index: u32,
    physical_device_count: usize,
    selected_device: String,
    input_values: &[Vec<f32>],
    lane_count: usize,
    artifact: &SpirvArtifact,
    operation: F32ArithmeticOp,
) -> Result<GlobalDynamicF32VectorLanesExecution, RuntimeError> {
    let queue = unsafe { device.get_device_queue(queue_family_index, 0) };
    validate_spirv_artifact_contract(artifact)
        .map_err(|error| RuntimeError::Codegen(error.to_string()))?;
    let operand = f32_operation_operand(operation);
    let kernel = storage_global_dynamic_index_vector_f32_binary_fixture_module_lanes(
        operand,
        operation,
        lane_count as u16,
    );
    let resources = reflect_resources(&kernel, FunctionId::new(0)).map_err(|error| {
        RuntimeError::DescriptorContract(format!(
            "dynamic f32x{lane_count} JIR reflection failed: {error}"
        ))
    })?;
    let lane_stride = (lane_count * std::mem::size_of::<f32>()) as u32;
    if resources.len() != 3
        || resources
            .iter()
            .any(|resource| resource.address_space != AddressSpace::Storage)
        || resources[0].element_stride != Some(lane_stride)
        || resources[1].element_stride != Some(lane_stride)
        || resources[2].element_stride != Some(4)
    {
        return Err(RuntimeError::DescriptorContract(format!(
            "dynamic f32x{lane_count} kernel must expose two {lane_stride}-byte vectors and one u32 length"
        )));
    }
    if artifact.entry_name != kernel.functions[0].name
        || artifact.workgroup_size != [GLOBAL_DYNAMIC_VECTOR_WORKGROUP_SIZE, 1, 1]
        || artifact.resources != resources
    {
        return Err(RuntimeError::DescriptorContract(format!(
            "dynamic f32x{lane_count} artifact metadata does not match the JIR fixture"
        )));
    }
    let spirv = &artifact.words;
    let descriptor_bindings = resources
        .iter()
        .map(|resource| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE)
        })
        .collect::<Vec<_>>();
    let shader_info = vk::ShaderModuleCreateInfo::default().code(spirv);
    let shader_module =
        unsafe { device.create_shader_module(&shader_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_dynamic_f32_vector_lanes_shader_module",
                code,
            }
        })?;
    let storage_buffer_info = vk::BufferCreateInfo::default()
        .size(GLOBAL_DYNAMIC_VECTOR_BUFFER_SIZE)
        .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let storage_buffer =
        unsafe { device.create_buffer(&storage_buffer_info, None) }.map_err(|code| {
            unsafe { device.destroy_shader_module(shader_module, None) };
            RuntimeError::Vulkan {
                operation: "create_dynamic_f32_vector_lanes_storage_buffer",
                code,
            }
        })?;
    let requirements = unsafe { device.get_buffer_memory_requirements(storage_buffer) };
    let memory_properties =
        unsafe { instance.get_physical_device_memory_properties(physical_device) };
    let memory_type_index = (0..memory_properties.memory_type_count)
        .find(|index| {
            requirements.memory_type_bits & (1_u32 << index) != 0
                && memory_properties.memory_types[*index as usize]
                    .property_flags
                    .contains(
                        vk::MemoryPropertyFlags::HOST_VISIBLE
                            | vk::MemoryPropertyFlags::HOST_COHERENT,
                    )
        })
        .ok_or_else(|| {
            RuntimeError::Loader("no coherent memory for dynamic f32 vector".to_owned())
        })?;
    let allocation_info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_type_index);
    let storage_memory =
        unsafe { device.allocate_memory(&allocation_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "allocate_dynamic_f32_vector_lanes_storage_memory",
                code,
            }
        })?;
    unsafe { device.bind_buffer_memory(storage_buffer, storage_memory, 0) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "bind_dynamic_f32_vector_lanes_storage_memory",
            code,
        }
    })?;
    let (mut resource_table, resource_id, access_token) =
        begin_storage_scope(GLOBAL_DYNAMIC_VECTOR_BUFFER_SIZE)?;
    let capacity_bytes =
        (GLOBAL_DYNAMIC_VECTOR_CAPACITY * lane_count * std::mem::size_of::<f32>()) as u64;
    let element_count = input_values.len();
    unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                GLOBAL_DYNAMIC_VECTOR_INPUT_BASE_OFFSET,
                capacity_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_dynamic_f32_vector_lanes_input",
                code,
            })?;
        let mapped = mapped.cast::<f32>();
        for (index, value) in input_values.iter().enumerate() {
            for (lane, scalar) in value.iter().enumerate() {
                std::ptr::write_unaligned(mapped.add(index * lane_count + lane), *scalar);
            }
        }
        device.unmap_memory(storage_memory);
        let mapped = device
            .map_memory(
                storage_memory,
                GLOBAL_DYNAMIC_VECTOR_OUTPUT_BASE_OFFSET,
                capacity_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_dynamic_f32_vector_lanes_output_clear",
                code,
            })?;
        let mapped = mapped.cast::<f32>();
        for index in 0..(GLOBAL_DYNAMIC_VECTOR_CAPACITY * lane_count) {
            std::ptr::write_unaligned(mapped.add(index), 0.0);
        }
        device.unmap_memory(storage_memory);
        let mapped = device
            .map_memory(
                storage_memory,
                GLOBAL_DYNAMIC_VECTOR_LENGTH_BASE_OFFSET,
                4,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_dynamic_f32_vector_lanes_length",
                code,
            })?;
        std::ptr::write_unaligned(mapped.cast::<u32>(), element_count as u32);
        device.unmap_memory(storage_memory);
    }

    let descriptor_set_layout = unsafe {
        device.create_descriptor_set_layout(
            &vk::DescriptorSetLayoutCreateInfo::default().bindings(&descriptor_bindings),
            None,
        )
    }
    .map_err(|code| RuntimeError::Vulkan {
        operation: "create_dynamic_f32_vector_lanes_descriptor_set_layout",
        code,
    })?;
    let pool_size = vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(3);
    let descriptor_pool = unsafe {
        device.create_descriptor_pool(
            &vk::DescriptorPoolCreateInfo::default()
                .max_sets(1)
                .pool_sizes(std::slice::from_ref(&pool_size)),
            None,
        )
    }
    .map_err(|code| RuntimeError::Vulkan {
        operation: "create_dynamic_f32_vector_lanes_descriptor_pool",
        code,
    })?;
    let descriptor_set = unsafe {
        device.allocate_descriptor_sets(
            &vk::DescriptorSetAllocateInfo::default()
                .descriptor_pool(descriptor_pool)
                .set_layouts(std::slice::from_ref(&descriptor_set_layout)),
        )
    }
    .map_err(|code| RuntimeError::Vulkan {
        operation: "allocate_dynamic_f32_vector_lanes_descriptor_set",
        code,
    })?[0];
    let descriptor_buffer_infos = [
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(GLOBAL_DYNAMIC_VECTOR_INPUT_BASE_OFFSET)
            .range(capacity_bytes),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(GLOBAL_DYNAMIC_VECTOR_OUTPUT_BASE_OFFSET)
            .range(capacity_bytes),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(GLOBAL_DYNAMIC_VECTOR_LENGTH_BASE_OFFSET)
            .range(4),
    ];
    let descriptor_writes = resources
        .iter()
        .enumerate()
        .map(|(index, resource)| {
            vk::WriteDescriptorSet::default()
                .dst_set(descriptor_set)
                .dst_binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(std::slice::from_ref(&descriptor_buffer_infos[index]))
        })
        .collect::<Vec<_>>();
    unsafe { device.update_descriptor_sets(&descriptor_writes, &[]) };
    let pipeline_layout = unsafe {
        device.create_pipeline_layout(
            &vk::PipelineLayoutCreateInfo::default()
                .set_layouts(std::slice::from_ref(&descriptor_set_layout)),
            None,
        )
    }
    .map_err(|code| RuntimeError::Vulkan {
        operation: "create_dynamic_f32_vector_lanes_pipeline_layout",
        code,
    })?;
    let entry_name =
        CString::new(kernel.functions[0].name.as_str()).expect("validated vector lane entry name");
    let stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(shader_module)
        .name(&entry_name);
    let pipeline_info = vk::ComputePipelineCreateInfo::default()
        .stage(stage)
        .layout(pipeline_layout);
    let pipeline = unsafe {
        device.create_compute_pipelines(
            vk::PipelineCache::null(),
            std::slice::from_ref(&pipeline_info),
            None,
        )
    }
    .map_err(|(_, code)| RuntimeError::Vulkan {
        operation: "create_dynamic_f32_vector_lanes_compute_pipeline",
        code,
    })?[0];
    let command_pool = unsafe {
        device.create_command_pool(
            &vk::CommandPoolCreateInfo::default()
                .queue_family_index(queue_family_index)
                .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
            None,
        )
    }
    .map_err(|code| RuntimeError::Vulkan {
        operation: "create_dynamic_f32_vector_lanes_command_pool",
        code,
    })?;
    let command_buffer = unsafe {
        device.allocate_command_buffers(
            &vk::CommandBufferAllocateInfo::default()
                .command_pool(command_pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1),
        )
    }
    .map_err(|code| RuntimeError::Vulkan {
        operation: "allocate_dynamic_f32_vector_lanes_command_buffer",
        code,
    })?[0];
    unsafe { device.begin_command_buffer(command_buffer, &vk::CommandBufferBeginInfo::default()) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "begin_dynamic_f32_vector_lanes_command_buffer",
            code,
        })?;
    let dispatch_x = (element_count as u32).div_ceil(GLOBAL_DYNAMIC_VECTOR_WORKGROUP_SIZE);
    unsafe {
        device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::COMPUTE, pipeline);
        device.cmd_bind_descriptor_sets(
            command_buffer,
            vk::PipelineBindPoint::COMPUTE,
            pipeline_layout,
            0,
            std::slice::from_ref(&descriptor_set),
            &[],
        );
        device.cmd_dispatch(command_buffer, dispatch_x, 1, 1);
    }
    unsafe { device.end_command_buffer(command_buffer) }.map_err(|code| RuntimeError::Vulkan {
        operation: "end_dynamic_f32_vector_lanes_command_buffer",
        code,
    })?;
    let fence =
        unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_dynamic_f32_vector_lanes_fence",
                code,
            }
        })?;
    let submit_info =
        vk::SubmitInfo::default().command_buffers(std::slice::from_ref(&command_buffer));
    unsafe { device.queue_submit(queue, std::slice::from_ref(&submit_info), fence) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "submit_dynamic_f32_vector_lanes_queue",
            code,
        },
    )?;
    unsafe { device.wait_for_fences(std::slice::from_ref(&fence), true, u64::MAX) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "wait_dynamic_f32_vector_lanes_fence",
            code,
        },
    )?;
    let mut output_values = vec![vec![0.0_f32; lane_count]; GLOBAL_DYNAMIC_VECTOR_CAPACITY];
    unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                GLOBAL_DYNAMIC_VECTOR_OUTPUT_BASE_OFFSET,
                capacity_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_dynamic_f32_vector_lanes_readback",
                code,
            })?;
        let mapped = mapped.cast::<f32>();
        for (index, value) in output_values.iter_mut().enumerate() {
            for (lane, scalar) in value.iter_mut().enumerate() {
                *scalar = std::ptr::read_unaligned(mapped.add(index * lane_count + lane));
            }
        }
        device.unmap_memory(storage_memory);
    }
    let expected_values = input_values
        .iter()
        .map(|value| {
            value
                .iter()
                .map(|lane| apply_f32_operation(*lane, operand, operation))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut expected_capacity = expected_values.clone();
    expected_capacity.resize(GLOBAL_DYNAMIC_VECTOR_CAPACITY, vec![0.0; lane_count]);
    let expected_flat = expected_capacity
        .iter()
        .flatten()
        .copied()
        .collect::<Vec<_>>();
    let output_flat = output_values.iter().flatten().copied().collect::<Vec<_>>();
    let differential_passed =
        compare_f32(&expected_flat, &output_flat, DifferentialPolicy::Exact).is_ok();
    let residency_passed =
        resource_table.release(access_token).is_ok() && resource_table.evict(resource_id).is_ok();
    let input_checksum = input_values
        .iter()
        .flatten()
        .map(|value| f64::from(*value))
        .sum();
    let output_checksum = output_values[..element_count]
        .iter()
        .flatten()
        .map(|value| f64::from(*value))
        .sum();
    let expected_checksum = expected_values
        .iter()
        .flatten()
        .map(|value| f64::from(*value))
        .sum();
    let untouched_tail_elements = output_values[element_count..]
        .iter()
        .filter(|value| value.iter().all(|lane| *lane == 0.0))
        .count();
    let first_output = output_values[0].clone();
    let last_output = output_values[element_count - 1].clone();
    unsafe {
        device.destroy_fence(fence, None);
        device.destroy_command_pool(command_pool, None);
        device.destroy_pipeline(pipeline, None);
        device.destroy_pipeline_layout(pipeline_layout, None);
        device.destroy_shader_module(shader_module, None);
        device.destroy_descriptor_pool(descriptor_pool, None);
        device.destroy_descriptor_set_layout(descriptor_set_layout, None);
        device.free_memory(storage_memory, None);
        device.destroy_buffer(storage_buffer, None);
    }
    Ok(GlobalDynamicF32VectorLanesExecution {
        report: GlobalDynamicF32VectorLanesQueueSmokeReport {
            schema: "jadren-vulkan-global-dynamic-f32-vector-lanes-artifact-queue-smoke-0.1",
            entry_name: kernel.functions[0].name.clone(),
            lane_count,
            operation: f32_operation_name(operation),
            operand,
            execution_path: "spirv-artifact",
            spirv_word_count: spirv.len(),
            spirv_word_hash: stable_spirv_word_hash(spirv),
            physical_device_count,
            selected_device,
            queue_family_index,
            resource_binding_count: resources.len(),
            element_count,
            capacity: GLOBAL_DYNAMIC_VECTOR_CAPACITY,
            dispatch_x,
            runtime_length: element_count as u32,
            input_checksum,
            output_checksum,
            expected_checksum,
            first_output,
            last_output,
            untouched_tail_elements,
            differential_execution: if differential_passed {
                "passed"
            } else {
                "failed"
            },
            residency_execution: if residency_passed { "passed" } else { "failed" },
        },
        output_values,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_global_dynamic_f32_on_device(
    instance: &Instance,
    device: &ash::Device,
    physical_device: vk::PhysicalDevice,
    queue_family_index: u32,
    physical_device_count: usize,
    selected_device: String,
    input_values: &[f32],
    artifact: Option<&SpirvArtifact>,
    operation: F32ArithmeticOp,
) -> Result<GlobalDynamicF32Execution, RuntimeError> {
    let queue = unsafe { device.get_device_queue(queue_family_index, 0) };
    let operand = f32_operation_operand(operation);
    let kernel =
        storage_global_dynamic_index_f32_binary_fixture_module(operand.to_bits(), operation);
    let artifact_execution = artifact.is_some();
    let generated_artifact = if artifact.is_none() {
        Some(
            emit_storage_global_index_f32_binary_dynamic_length_artifact_from_jir(
                &kernel,
                FunctionId::new(0),
                SpirvOptions::new([GLOBAL_DYNAMIC_WORKGROUP_SIZE, 1, 1])
                    .map_err(|error| RuntimeError::Codegen(error.to_string()))?,
                operation,
            )
            .map_err(|error| RuntimeError::Codegen(error.to_string()))?,
        )
    } else {
        None
    };
    let artifact = artifact.or(generated_artifact.as_ref()).ok_or_else(|| {
        RuntimeError::Codegen("f32 artifact was not provided or generated".to_owned())
    })?;
    validate_spirv_artifact_contract(artifact)
        .map_err(|error| RuntimeError::Codegen(error.to_string()))?;
    let resources = reflect_resources(&kernel, FunctionId::new(0)).map_err(|error| {
        RuntimeError::DescriptorContract(format!("dynamic f32 JIR reflection failed: {error}"))
    })?;
    if resources.len() != 3
        || resources
            .iter()
            .any(|resource| resource.address_space != AddressSpace::Storage)
    {
        return Err(RuntimeError::DescriptorContract(
            "dynamic f32 kernel must expose three storage resources".to_owned(),
        ));
    }
    if artifact.entry_name != kernel.functions[0].name
        || artifact.workgroup_size != [GLOBAL_DYNAMIC_WORKGROUP_SIZE, 1, 1]
        || artifact.resources != resources
    {
        return Err(RuntimeError::DescriptorContract(
            "dynamic f32 artifact metadata does not match the JIR fixture".to_owned(),
        ));
    }
    let spirv = &artifact.words;
    let descriptor_bindings = resources
        .iter()
        .map(|resource| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE)
        })
        .collect::<Vec<_>>();
    let shader_info = vk::ShaderModuleCreateInfo::default().code(spirv);
    let shader_module =
        unsafe { device.create_shader_module(&shader_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_dynamic_f32_shader_module",
                code,
            }
        })?;
    let storage_buffer_info = vk::BufferCreateInfo::default()
        .size(GLOBAL_DYNAMIC_BUFFER_SIZE)
        .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let storage_buffer =
        unsafe { device.create_buffer(&storage_buffer_info, None) }.map_err(|code| {
            unsafe { device.destroy_shader_module(shader_module, None) };
            RuntimeError::Vulkan {
                operation: "create_dynamic_f32_storage_buffer",
                code,
            }
        })?;
    let requirements = unsafe { device.get_buffer_memory_requirements(storage_buffer) };
    let memory_properties =
        unsafe { instance.get_physical_device_memory_properties(physical_device) };
    let memory_type_index = (0..memory_properties.memory_type_count)
        .find(|index| {
            requirements.memory_type_bits & (1_u32 << index) != 0
                && memory_properties.memory_types[*index as usize]
                    .property_flags
                    .contains(
                        vk::MemoryPropertyFlags::HOST_VISIBLE
                            | vk::MemoryPropertyFlags::HOST_COHERENT,
                    )
        })
        .ok_or_else(|| RuntimeError::Loader("no coherent memory for dynamic f32".to_owned()))?;
    let allocation_info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_type_index);
    let storage_memory =
        unsafe { device.allocate_memory(&allocation_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "allocate_dynamic_f32_storage_memory",
                code,
            }
        })?;
    unsafe { device.bind_buffer_memory(storage_buffer, storage_memory, 0) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "bind_dynamic_f32_storage_memory",
            code,
        }
    })?;
    let (mut resource_table, resource_id, access_token) =
        begin_storage_scope(GLOBAL_DYNAMIC_BUFFER_SIZE)?;
    let capacity_bytes = (GLOBAL_DYNAMIC_CAPACITY * std::mem::size_of::<f32>()) as u64;
    let element_count = input_values.len();
    unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                GLOBAL_DYNAMIC_INPUT_BASE_OFFSET,
                capacity_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_dynamic_f32_input",
                code,
            })?;
        for (index, value) in input_values.iter().enumerate() {
            std::ptr::write_unaligned(mapped.cast::<f32>().add(index), *value);
        }
        device.unmap_memory(storage_memory);
        let mapped = device
            .map_memory(
                storage_memory,
                GLOBAL_DYNAMIC_OUTPUT_BASE_OFFSET,
                capacity_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_dynamic_f32_output_clear",
                code,
            })?;
        for index in 0..GLOBAL_DYNAMIC_CAPACITY {
            std::ptr::write_unaligned(mapped.cast::<f32>().add(index), 0.0);
        }
        device.unmap_memory(storage_memory);
        let mapped = device
            .map_memory(
                storage_memory,
                GLOBAL_DYNAMIC_LENGTH_BASE_OFFSET,
                4,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_dynamic_f32_length",
                code,
            })?;
        std::ptr::write_unaligned(mapped.cast::<u32>(), element_count as u32);
        device.unmap_memory(storage_memory);
    }
    let descriptor_set_layout = unsafe {
        device.create_descriptor_set_layout(
            &vk::DescriptorSetLayoutCreateInfo::default().bindings(&descriptor_bindings),
            None,
        )
    }
    .map_err(|code| RuntimeError::Vulkan {
        operation: "create_dynamic_f32_descriptor_set_layout",
        code,
    })?;
    let pool_size = vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(3);
    let descriptor_pool = unsafe {
        device.create_descriptor_pool(
            &vk::DescriptorPoolCreateInfo::default()
                .max_sets(1)
                .pool_sizes(std::slice::from_ref(&pool_size)),
            None,
        )
    }
    .map_err(|code| RuntimeError::Vulkan {
        operation: "create_dynamic_f32_descriptor_pool",
        code,
    })?;
    let descriptor_set = unsafe {
        device.allocate_descriptor_sets(
            &vk::DescriptorSetAllocateInfo::default()
                .descriptor_pool(descriptor_pool)
                .set_layouts(std::slice::from_ref(&descriptor_set_layout)),
        )
    }
    .map_err(|code| RuntimeError::Vulkan {
        operation: "allocate_dynamic_f32_descriptor_set",
        code,
    })?[0];
    let descriptor_buffer_infos = [
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(GLOBAL_DYNAMIC_INPUT_BASE_OFFSET)
            .range(capacity_bytes),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(GLOBAL_DYNAMIC_OUTPUT_BASE_OFFSET)
            .range(capacity_bytes),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(GLOBAL_DYNAMIC_LENGTH_BASE_OFFSET)
            .range(4),
    ];
    let descriptor_writes = resources
        .iter()
        .enumerate()
        .map(|(index, resource)| {
            vk::WriteDescriptorSet::default()
                .dst_set(descriptor_set)
                .dst_binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(std::slice::from_ref(&descriptor_buffer_infos[index]))
        })
        .collect::<Vec<_>>();
    unsafe { device.update_descriptor_sets(&descriptor_writes, &[]) };
    let pipeline_layout = unsafe {
        device.create_pipeline_layout(
            &vk::PipelineLayoutCreateInfo::default()
                .set_layouts(std::slice::from_ref(&descriptor_set_layout)),
            None,
        )
    }
    .map_err(|code| RuntimeError::Vulkan {
        operation: "create_dynamic_f32_pipeline_layout",
        code,
    })?;
    let entry_name =
        CString::new(kernel.functions[0].name.as_str()).expect("validated f32 entry name");
    let stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(shader_module)
        .name(&entry_name);
    let pipeline_info = vk::ComputePipelineCreateInfo::default()
        .stage(stage)
        .layout(pipeline_layout);
    let pipeline = unsafe {
        device.create_compute_pipelines(
            vk::PipelineCache::null(),
            std::slice::from_ref(&pipeline_info),
            None,
        )
    }
    .map_err(|(_, code)| RuntimeError::Vulkan {
        operation: "create_dynamic_f32_compute_pipeline",
        code,
    })?[0];
    let command_pool = unsafe {
        device.create_command_pool(
            &vk::CommandPoolCreateInfo::default()
                .queue_family_index(queue_family_index)
                .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
            None,
        )
    }
    .map_err(|code| RuntimeError::Vulkan {
        operation: "create_dynamic_f32_command_pool",
        code,
    })?;
    let command_buffer = unsafe {
        device.allocate_command_buffers(
            &vk::CommandBufferAllocateInfo::default()
                .command_pool(command_pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1),
        )
    }
    .map_err(|code| RuntimeError::Vulkan {
        operation: "allocate_dynamic_f32_command_buffer",
        code,
    })?[0];
    unsafe { device.begin_command_buffer(command_buffer, &vk::CommandBufferBeginInfo::default()) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "begin_dynamic_f32_command_buffer",
            code,
        })?;
    let dispatch_x = (element_count as u32).div_ceil(GLOBAL_DYNAMIC_WORKGROUP_SIZE);
    unsafe {
        device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::COMPUTE, pipeline);
        device.cmd_bind_descriptor_sets(
            command_buffer,
            vk::PipelineBindPoint::COMPUTE,
            pipeline_layout,
            0,
            std::slice::from_ref(&descriptor_set),
            &[],
        );
        device.cmd_dispatch(command_buffer, dispatch_x, 1, 1);
    }
    unsafe { device.end_command_buffer(command_buffer) }.map_err(|code| RuntimeError::Vulkan {
        operation: "end_dynamic_f32_command_buffer",
        code,
    })?;
    let fence =
        unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_dynamic_f32_fence",
                code,
            }
        })?;
    let submit_info =
        vk::SubmitInfo::default().command_buffers(std::slice::from_ref(&command_buffer));
    unsafe { device.queue_submit(queue, std::slice::from_ref(&submit_info), fence) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "submit_dynamic_f32_queue",
            code,
        },
    )?;
    unsafe { device.wait_for_fences(std::slice::from_ref(&fence), true, u64::MAX) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "wait_dynamic_f32_fence",
            code,
        },
    )?;
    let mut output_values = vec![0.0_f32; GLOBAL_DYNAMIC_CAPACITY];
    unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                GLOBAL_DYNAMIC_OUTPUT_BASE_OFFSET,
                capacity_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_dynamic_f32_readback",
                code,
            })?;
        for (index, value) in output_values.iter_mut().enumerate() {
            *value = std::ptr::read_unaligned(mapped.cast::<f32>().add(index));
        }
        device.unmap_memory(storage_memory);
    }
    let expected_values = input_values
        .iter()
        .map(|value| apply_f32_operation(*value, operand, operation))
        .collect::<Vec<_>>();
    let mut expected_capacity = expected_values.clone();
    expected_capacity.resize(GLOBAL_DYNAMIC_CAPACITY, 0.0);
    let differential_passed = compare_f32(
        &expected_capacity,
        &output_values,
        DifferentialPolicy::Exact,
    )
    .is_ok();
    let residency_passed =
        resource_table.release(access_token).is_ok() && resource_table.evict(resource_id).is_ok();
    let input_checksum = input_values.iter().map(|value| f64::from(*value)).sum();
    let output_checksum = output_values[..element_count]
        .iter()
        .map(|value| f64::from(*value))
        .sum();
    let expected_checksum = expected_values.iter().map(|value| f64::from(*value)).sum();
    let untouched_tail_elements = output_values[element_count..]
        .iter()
        .filter(|value| **value == 0.0)
        .count();
    unsafe {
        device.destroy_fence(fence, None);
        device.destroy_command_pool(command_pool, None);
        device.destroy_pipeline(pipeline, None);
        device.destroy_pipeline_layout(pipeline_layout, None);
        device.destroy_shader_module(shader_module, None);
        device.destroy_descriptor_pool(descriptor_pool, None);
        device.destroy_descriptor_set_layout(descriptor_set_layout, None);
        device.free_memory(storage_memory, None);
        device.destroy_buffer(storage_buffer, None);
    }
    Ok(GlobalDynamicF32Execution {
        report: GlobalDynamicF32QueueSmokeReport {
            schema: if artifact_execution {
                "jadren-vulkan-global-dynamic-f32-artifact-queue-smoke-0.1"
            } else {
                "jadren-vulkan-global-dynamic-f32-queue-smoke-0.1"
            },
            entry_name: kernel.functions[0].name.clone(),
            operation: f32_operation_name(operation),
            operand,
            execution_path: if artifact_execution {
                "spirv-artifact"
            } else {
                "generated-artifact"
            },
            spirv_word_count: spirv.len(),
            spirv_word_hash: stable_spirv_word_hash(spirv),
            physical_device_count,
            selected_device,
            queue_family_index,
            resource_binding_count: resources.len(),
            element_count,
            capacity: GLOBAL_DYNAMIC_CAPACITY,
            dispatch_x,
            runtime_length: element_count as u32,
            input_checksum,
            output_checksum,
            expected_checksum,
            first_output: output_values[0],
            last_output: output_values[element_count - 1],
            untouched_tail_elements,
            differential_execution: if differential_passed {
                "passed"
            } else {
                "failed"
            },
            residency_execution: if residency_passed { "passed" } else { "failed" },
        },
        output_values,
    })
}

fn run_global_write_on_instance(
    instance: &Instance,
    config: GlobalWriteArtifactConfig,
) -> Result<GlobalWriteU32QueueSmokeReport, RuntimeError> {
    let physical_devices =
        unsafe { instance.enumerate_physical_devices() }.map_err(|code| RuntimeError::Vulkan {
            operation: "enumerate_global_write_physical_devices",
            code,
        })?;
    let (physical_device, queue_family_index) = physical_devices
        .iter()
        .find_map(|physical_device| {
            let families =
                unsafe { instance.get_physical_device_queue_family_properties(*physical_device) };
            families
                .iter()
                .enumerate()
                .find(|(_, properties)| {
                    properties.queue_count > 0
                        && properties.queue_flags.contains(vk::QueueFlags::COMPUTE)
                })
                .map(|(index, _)| (*physical_device, index as u32))
        })
        .ok_or(RuntimeError::NoComputeQueue)?;
    let properties = unsafe { instance.get_physical_device_properties(physical_device) };
    let selected_device = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    let priorities = [1.0_f32];
    let queue_info = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family_index)
        .queue_priorities(&priorities);
    let device_info =
        vk::DeviceCreateInfo::default().queue_create_infos(std::slice::from_ref(&queue_info));
    let device =
        unsafe { instance.create_device(physical_device, &device_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_write_device",
                code,
            }
        })?;
    let result = run_global_write_on_device(
        instance,
        &device,
        physical_device,
        queue_family_index,
        physical_devices.len(),
        selected_device,
        config,
    );
    unsafe {
        let _ = device.device_wait_idle();
        device.destroy_device(None);
    }
    result
}

fn run_global_write_on_device(
    instance: &Instance,
    device: &ash::Device,
    physical_device: vk::PhysicalDevice,
    queue_family_index: u32,
    physical_device_count: usize,
    selected_device: String,
    config: GlobalWriteArtifactConfig,
) -> Result<GlobalWriteU32QueueSmokeReport, RuntimeError> {
    let queue = unsafe { device.get_device_queue(queue_family_index, 0) };
    let kernel =
        storage_global_index_write_fixture_module_with_config(config.value, config.length)?;
    let resources = reflect_resources(&kernel, FunctionId::new(0)).map_err(|error| {
        RuntimeError::DescriptorContract(format!("global-write JIR reflection failed: {error}"))
    })?;
    if resources.len() != 1
        || resources[0].address_space != AddressSpace::Storage
        || !matches!(
            kernel.types.get(resources[0].element_type.index()),
            Some(Type::Integer {
                signed: false,
                bits: 32
            })
        )
    {
        return Err(RuntimeError::DescriptorContract(
            "global-write kernel must expose one storage u32 resource".to_owned(),
        ));
    }
    let descriptor_bindings = resources
        .iter()
        .map(|resource| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE)
        })
        .collect::<Vec<_>>();
    let artifact = emit_storage_global_index_write_artifact_from_jir(
        &kernel,
        FunctionId::new(0),
        SpirvOptions::new([config.workgroup_size, 1, 1])
            .map_err(|error| RuntimeError::Codegen(error.to_string()))?,
    )
    .map_err(|error| RuntimeError::Codegen(error.to_string()))?;
    validate_spirv_artifact_contract(&artifact)
        .map_err(|error| RuntimeError::Codegen(error.to_string()))?;
    let shader_info = vk::ShaderModuleCreateInfo::default().code(&artifact.words);
    let shader_module =
        unsafe { device.create_shader_module(&shader_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_write_shader_module",
                code,
            }
        })?;
    let entry_name =
        CString::new(artifact.entry_name.as_str()).expect("validated JIR entry name has no NUL");
    let stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(shader_module)
        .name(&entry_name);
    let element_bytes = config.buffer_size()?;
    let storage_buffer_info = vk::BufferCreateInfo::default()
        .size(element_bytes)
        .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let storage_buffer =
        unsafe { device.create_buffer(&storage_buffer_info, None) }.map_err(|code| {
            unsafe { device.destroy_shader_module(shader_module, None) };
            RuntimeError::Vulkan {
                operation: "create_global_write_storage_buffer",
                code,
            }
        })?;
    let requirements = unsafe { device.get_buffer_memory_requirements(storage_buffer) };
    let memory_properties =
        unsafe { instance.get_physical_device_memory_properties(physical_device) };
    let memory_type_index = (0..memory_properties.memory_type_count)
        .find(|index| {
            let compatible = requirements.memory_type_bits & (1_u32 << index) != 0;
            compatible
                && memory_properties.memory_types[*index as usize]
                    .property_flags
                    .contains(
                        vk::MemoryPropertyFlags::HOST_VISIBLE
                            | vk::MemoryPropertyFlags::HOST_COHERENT,
                    )
        })
        .ok_or_else(|| {
            RuntimeError::Loader(
                "no host-visible coherent Vulkan memory type for global-write smoke".to_owned(),
            )
        })?;
    let allocation_info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_type_index);
    let storage_memory =
        unsafe { device.allocate_memory(&allocation_info, None) }.map_err(|code| {
            unsafe { device.destroy_buffer(storage_buffer, None) };
            RuntimeError::Vulkan {
                operation: "allocate_global_write_storage_memory",
                code,
            }
        })?;
    unsafe { device.bind_buffer_memory(storage_buffer, storage_memory, 0) }.map_err(|code| {
        unsafe {
            device.free_memory(storage_memory, None);
            device.destroy_buffer(storage_buffer, None);
        }
        RuntimeError::Vulkan {
            operation: "bind_global_write_storage_memory",
            code,
        }
    })?;
    let (mut resource_table, resource_id, access_token) = begin_storage_scope(element_bytes)?;
    unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                0,
                element_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_global_write_output_clear",
                code,
            })?;
        for index in 0..config.capacity {
            std::ptr::write_unaligned(mapped.cast::<u32>().add(index), 0);
        }
        device.unmap_memory(storage_memory);
    }
    let descriptor_layout_info =
        vk::DescriptorSetLayoutCreateInfo::default().bindings(&descriptor_bindings);
    let descriptor_set_layout =
        unsafe { device.create_descriptor_set_layout(&descriptor_layout_info, None) }.map_err(
            |code| RuntimeError::Vulkan {
                operation: "create_global_write_descriptor_set_layout",
                code,
            },
        )?;
    let descriptor_pool_size = vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(1);
    let descriptor_pool_info = vk::DescriptorPoolCreateInfo::default()
        .max_sets(1)
        .pool_sizes(std::slice::from_ref(&descriptor_pool_size));
    let descriptor_pool = unsafe { device.create_descriptor_pool(&descriptor_pool_info, None) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "create_global_write_descriptor_pool",
            code,
        })?;
    let descriptor_set_allocate_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(descriptor_pool)
        .set_layouts(std::slice::from_ref(&descriptor_set_layout));
    let descriptor_set = unsafe { device.allocate_descriptor_sets(&descriptor_set_allocate_info) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "allocate_global_write_descriptor_set",
            code,
        })?[0];
    let descriptor_buffer_info = vk::DescriptorBufferInfo::default()
        .buffer(storage_buffer)
        .offset(0)
        .range(element_bytes);
    let descriptor_writes = resources
        .iter()
        .map(|resource| {
            vk::WriteDescriptorSet::default()
                .dst_set(descriptor_set)
                .dst_binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(std::slice::from_ref(&descriptor_buffer_info))
        })
        .collect::<Vec<_>>();
    unsafe { device.update_descriptor_sets(&descriptor_writes, &[]) };
    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(std::slice::from_ref(&descriptor_set_layout));
    let pipeline_layout =
        unsafe { device.create_pipeline_layout(&layout_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_write_pipeline_layout",
                code,
            }
        })?;
    let pipeline_info = vk::ComputePipelineCreateInfo::default()
        .stage(stage)
        .layout(pipeline_layout);
    let pipeline = match unsafe {
        device.create_compute_pipelines(
            vk::PipelineCache::null(),
            std::slice::from_ref(&pipeline_info),
            None,
        )
    } {
        Ok(pipelines) => pipelines[0],
        Err((_, code)) => {
            return Err(RuntimeError::Vulkan {
                operation: "create_global_write_compute_pipeline",
                code,
            });
        }
    };
    let pool_info = vk::CommandPoolCreateInfo::default()
        .queue_family_index(queue_family_index)
        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
    let command_pool = unsafe { device.create_command_pool(&pool_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_global_write_command_pool",
            code,
        }
    })?;
    let allocation_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let command_buffer =
        unsafe { device.allocate_command_buffers(&allocation_info) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "allocate_global_write_command_buffer",
                code,
            }
        })?[0];
    unsafe { device.begin_command_buffer(command_buffer, &vk::CommandBufferBeginInfo::default()) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "begin_global_write_command_buffer",
            code,
        })?;
    unsafe {
        device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::COMPUTE, pipeline);
        device.cmd_bind_descriptor_sets(
            command_buffer,
            vk::PipelineBindPoint::COMPUTE,
            pipeline_layout,
            0,
            std::slice::from_ref(&descriptor_set),
            &[],
        );
        device.cmd_dispatch(command_buffer, config.dispatch_x()?, 1, 1);
    }
    unsafe { device.end_command_buffer(command_buffer) }.map_err(|code| RuntimeError::Vulkan {
        operation: "end_global_write_command_buffer",
        code,
    })?;
    let fence =
        unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_write_fence",
                code,
            }
        })?;
    let submit_info =
        vk::SubmitInfo::default().command_buffers(std::slice::from_ref(&command_buffer));
    unsafe { device.queue_submit(queue, std::slice::from_ref(&submit_info), fence) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "submit_global_write_queue",
            code,
        },
    )?;
    unsafe { device.wait_for_fences(std::slice::from_ref(&fence), true, u64::MAX) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "wait_global_write_fence",
            code,
        },
    )?;
    let mut output_values = vec![0_u32; config.capacity];
    unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                0,
                element_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_global_write_readback",
                code,
            })?;
        for (index, value) in output_values.iter_mut().enumerate() {
            *value = std::ptr::read_unaligned(mapped.cast::<u32>().add(index));
        }
        device.unmap_memory(storage_memory);
    }
    let mut expected_values = vec![0_u32; config.capacity];
    expected_values[..config.length].fill(config.value);
    let untouched_elements = output_values[config.length..]
        .iter()
        .filter(|value| **value == 0)
        .count();
    let differential_passed = compare_u32(&expected_values, &output_values).is_ok()
        && untouched_elements == config.capacity - config.length;
    let residency_passed =
        resource_table.release(access_token).is_ok() && resource_table.evict(resource_id).is_ok();
    let output_checksum = output_values.iter().map(|value| u64::from(*value)).sum();
    let expected_checksum = expected_values.iter().map(|value| u64::from(*value)).sum();
    unsafe {
        device.destroy_fence(fence, None);
        device.destroy_command_pool(command_pool, None);
        device.destroy_pipeline(pipeline, None);
        device.destroy_pipeline_layout(pipeline_layout, None);
        device.destroy_shader_module(shader_module, None);
        device.destroy_descriptor_pool(descriptor_pool, None);
        device.destroy_descriptor_set_layout(descriptor_set_layout, None);
        device.free_memory(storage_memory, None);
        device.destroy_buffer(storage_buffer, None);
    }
    Ok(GlobalWriteU32QueueSmokeReport {
        schema: "jadren-vulkan-global-write-u32-queue-smoke-0.1",
        physical_device_count,
        selected_device,
        queue_family_index,
        artifact_entry_name: artifact.entry_name,
        artifact_word_count: artifact.words.len(),
        artifact_word_hash: stable_spirv_word_hash(&artifact.words),
        artifact_validated: true,
        queue_execution: "passed",
        descriptor_setup: "passed",
        resource_binding_count: resources.len(),
        pipeline_execution: "passed",
        fence_execution: "passed",
        data_kernel_execution: if differential_passed {
            "passed"
        } else {
            "failed"
        },
        differential_execution: if differential_passed {
            "passed"
        } else {
            "failed"
        },
        residency_execution: if residency_passed { "passed" } else { "failed" },
        element_count: config.length,
        capacity: config.capacity,
        dispatch_x: config.dispatch_x()?,
        untouched_elements,
        output_checksum,
        expected_checksum,
        first_output: output_values[0],
        last_output: output_values[config.length - 1],
    })
}

fn run_storage_add_artifact_on_instance(
    instance: &Instance,
) -> Result<StorageAddArtifactQueueSmokeReport, RuntimeError> {
    let physical_devices =
        unsafe { instance.enumerate_physical_devices() }.map_err(|code| RuntimeError::Vulkan {
            operation: "enumerate_storage_add_artifact_physical_devices",
            code,
        })?;
    let (physical_device, queue_family_index) = physical_devices
        .iter()
        .find_map(|physical_device| {
            let families =
                unsafe { instance.get_physical_device_queue_family_properties(*physical_device) };
            families
                .iter()
                .enumerate()
                .find(|(_, properties)| {
                    properties.queue_count > 0
                        && properties.queue_flags.contains(vk::QueueFlags::COMPUTE)
                })
                .map(|(index, _)| (*physical_device, index as u32))
        })
        .ok_or(RuntimeError::NoComputeQueue)?;
    let properties = unsafe { instance.get_physical_device_properties(physical_device) };
    let selected_device = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    let priorities = [1.0_f32];
    let queue_info = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family_index)
        .queue_priorities(&priorities);
    let device_info =
        vk::DeviceCreateInfo::default().queue_create_infos(std::slice::from_ref(&queue_info));
    let device =
        unsafe { instance.create_device(physical_device, &device_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_storage_add_artifact_device",
                code,
            }
        })?;
    let result = run_storage_add_artifact_on_device(
        instance,
        &device,
        physical_device,
        queue_family_index,
        physical_devices.len(),
        selected_device,
    );
    unsafe {
        let _ = device.device_wait_idle();
        device.destroy_device(None);
    }
    result
}

fn run_storage_add_artifact_on_device(
    instance: &Instance,
    device: &ash::Device,
    physical_device: vk::PhysicalDevice,
    queue_family_index: u32,
    physical_device_count: usize,
    selected_device: String,
) -> Result<StorageAddArtifactQueueSmokeReport, RuntimeError> {
    let queue = unsafe { device.get_device_queue(queue_family_index, 0) };
    let kernel = storage_add_artifact_fixture_module();
    let artifact = emit_storage_add_artifact_from_jir(
        &kernel,
        FunctionId::new(0),
        SpirvOptions::new(STORAGE_ADD_ARTIFACT_WORKGROUP_SIZE)
            .map_err(|error| RuntimeError::Codegen(error.to_string()))?,
    )
    .map_err(|error| RuntimeError::Codegen(error.to_string()))?;
    let max_workgroup_size = unsafe { instance.get_physical_device_properties(physical_device) }
        .limits
        .max_compute_work_group_invocations;
    let mut resource_table = ResourceTable::new();
    let resource_id = resource_table
        .create_buffer(STORAGE_ADD_ARTIFACT_BUFFER_SIZE)
        .map_err(|error| RuntimeError::DescriptorContract(error.to_string()))?;
    resource_table
        .make_resident(resource_id)
        .map_err(|error| RuntimeError::DescriptorContract(error.to_string()))?;
    let prepared = prepare_artifact_dispatch(
        &mut resource_table,
        GpuBackend::Vulkan,
        BackendProbe {
            device_available: true,
            storage_buffers: true,
            global_invocation_id_x: true,
            structured_bounds: true,
            deterministic_f32: true,
            async_completion: true,
            shader_translation_available: false,
            max_workgroup_size,
        },
        ArtifactDispatchRequest {
            fp: FpPolicy::Strict,
            require_bounded_global_u32_array: false,
            require_async_completion: true,
        },
        DispatchGeometry::new([1, 1, 1])
            .map_err(|error| RuntimeError::DescriptorContract(error.to_string()))?,
        &[ArtifactResourceRequest {
            binding: 0,
            buffer: resource_id,
            required_bytes: STORAGE_ADD_ARTIFACT_BUFFER_SIZE,
        }],
        &artifact,
    )
    .map_err(|error| RuntimeError::DescriptorContract(error.to_string()))?;
    let descriptor = prepared.descriptor().clone();
    if let Err(error) = descriptor.validate_source_translation() {
        let _ = resource_table.release_prepared_artifact_dispatch(prepared);
        return Err(RuntimeError::DescriptorContract(error.to_string()));
    }
    let resources = &artifact.resources;
    if resources.len() != 1
        || resources[0].binding != 0
        || resources[0].address_space != AddressSpace::Storage
        || !matches!(
            kernel.types.get(resources[0].element_type.index()),
            Some(Type::Integer {
                signed: false,
                bits: 32
            })
        )
    {
        return Err(RuntimeError::DescriptorContract(
            "storage-add artifact must expose one storage u32 resource at binding zero".to_owned(),
        ));
    }
    let descriptor_bindings = descriptor
        .resources
        .iter()
        .map(|resource| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE)
        })
        .collect::<Vec<_>>();
    let shader_info = vk::ShaderModuleCreateInfo::default().code(&artifact.words);
    let shader_module =
        unsafe { device.create_shader_module(&shader_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_storage_add_artifact_shader_module",
                code,
            }
        })?;
    let entry_name =
        CString::new(artifact.entry_name.as_str()).expect("validated JIR entry name has no NUL");
    let stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(shader_module)
        .name(&entry_name);
    let storage_buffer_info = vk::BufferCreateInfo::default()
        .size(STORAGE_ADD_ARTIFACT_BUFFER_SIZE)
        .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let storage_buffer =
        unsafe { device.create_buffer(&storage_buffer_info, None) }.map_err(|code| {
            unsafe { device.destroy_shader_module(shader_module, None) };
            RuntimeError::Vulkan {
                operation: "create_storage_add_artifact_storage_buffer",
                code,
            }
        })?;
    let requirements = unsafe { device.get_buffer_memory_requirements(storage_buffer) };
    let memory_properties =
        unsafe { instance.get_physical_device_memory_properties(physical_device) };
    let memory_type_index = (0..memory_properties.memory_type_count)
        .find(|index| {
            let compatible = requirements.memory_type_bits & (1_u32 << index) != 0;
            compatible
                && memory_properties.memory_types[*index as usize]
                    .property_flags
                    .contains(
                        vk::MemoryPropertyFlags::HOST_VISIBLE
                            | vk::MemoryPropertyFlags::HOST_COHERENT,
                    )
        })
        .ok_or_else(|| {
            RuntimeError::Loader(
                "no host-visible coherent Vulkan memory type for storage-add artifact smoke"
                    .to_owned(),
            )
        })?;
    let allocation_info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_type_index);
    let storage_memory =
        unsafe { device.allocate_memory(&allocation_info, None) }.map_err(|code| {
            unsafe { device.destroy_buffer(storage_buffer, None) };
            RuntimeError::Vulkan {
                operation: "allocate_storage_add_artifact_storage_memory",
                code,
            }
        })?;
    unsafe { device.bind_buffer_memory(storage_buffer, storage_memory, 0) }.map_err(|code| {
        unsafe {
            device.free_memory(storage_memory, None);
            device.destroy_buffer(storage_buffer, None);
        }
        RuntimeError::Vulkan {
            operation: "bind_storage_add_artifact_storage_memory",
            code,
        }
    })?;
    let element_bytes = STORAGE_ADD_ARTIFACT_BUFFER_SIZE;
    let input_values: Vec<u32> = (0..STORAGE_ADD_ARTIFACT_ELEMENT_COUNT)
        .map(|index| STORAGE_ADD_ARTIFACT_INPUT_START + index as u32)
        .collect();
    unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                0,
                element_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_storage_add_artifact_input",
                code,
            })?;
        for (index, value) in input_values.iter().enumerate() {
            std::ptr::write_unaligned(mapped.cast::<u32>().add(index), *value);
        }
        device.unmap_memory(storage_memory);
    }
    let descriptor_layout_info =
        vk::DescriptorSetLayoutCreateInfo::default().bindings(&descriptor_bindings);
    let descriptor_set_layout =
        unsafe { device.create_descriptor_set_layout(&descriptor_layout_info, None) }.map_err(
            |code| RuntimeError::Vulkan {
                operation: "create_storage_add_artifact_descriptor_set_layout",
                code,
            },
        )?;
    let descriptor_pool_size = vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(1);
    let descriptor_pool_info = vk::DescriptorPoolCreateInfo::default()
        .max_sets(1)
        .pool_sizes(std::slice::from_ref(&descriptor_pool_size));
    let descriptor_pool = unsafe { device.create_descriptor_pool(&descriptor_pool_info, None) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "create_storage_add_artifact_descriptor_pool",
            code,
        })?;
    let descriptor_set_allocate_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(descriptor_pool)
        .set_layouts(std::slice::from_ref(&descriptor_set_layout));
    let descriptor_set = unsafe { device.allocate_descriptor_sets(&descriptor_set_allocate_info) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "allocate_storage_add_artifact_descriptor_set",
            code,
        })?[0];
    let descriptor_buffer_info = vk::DescriptorBufferInfo::default()
        .buffer(storage_buffer)
        .offset(0)
        .range(element_bytes);
    let descriptor_writes = descriptor
        .resources
        .iter()
        .map(|resource| {
            vk::WriteDescriptorSet::default()
                .dst_set(descriptor_set)
                .dst_binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(std::slice::from_ref(&descriptor_buffer_info))
        })
        .collect::<Vec<_>>();
    unsafe { device.update_descriptor_sets(&descriptor_writes, &[]) };
    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(std::slice::from_ref(&descriptor_set_layout));
    let pipeline_layout =
        unsafe { device.create_pipeline_layout(&layout_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_storage_add_artifact_pipeline_layout",
                code,
            }
        })?;
    let pipeline_info = vk::ComputePipelineCreateInfo::default()
        .stage(stage)
        .layout(pipeline_layout);
    let pipeline = match unsafe {
        device.create_compute_pipelines(
            vk::PipelineCache::null(),
            std::slice::from_ref(&pipeline_info),
            None,
        )
    } {
        Ok(pipelines) => pipelines[0],
        Err((_, code)) => {
            return Err(RuntimeError::Vulkan {
                operation: "create_storage_add_artifact_compute_pipeline",
                code,
            });
        }
    };
    let pool_info = vk::CommandPoolCreateInfo::default()
        .queue_family_index(queue_family_index)
        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
    let command_pool = unsafe { device.create_command_pool(&pool_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_storage_add_artifact_command_pool",
            code,
        }
    })?;
    let command_buffer_allocate_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let command_buffer = unsafe { device.allocate_command_buffers(&command_buffer_allocate_info) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "allocate_storage_add_artifact_command_buffer",
            code,
        })?[0];
    unsafe { device.begin_command_buffer(command_buffer, &vk::CommandBufferBeginInfo::default()) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "begin_storage_add_artifact_command_buffer",
            code,
        })?;
    unsafe {
        device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::COMPUTE, pipeline);
        device.cmd_bind_descriptor_sets(
            command_buffer,
            vk::PipelineBindPoint::COMPUTE,
            pipeline_layout,
            0,
            std::slice::from_ref(&descriptor_set),
            &[],
        );
        let [dispatch_x, dispatch_y, dispatch_z] = descriptor.workgroups;
        device.cmd_dispatch(command_buffer, dispatch_x, dispatch_y, dispatch_z);
    }
    unsafe { device.end_command_buffer(command_buffer) }.map_err(|code| RuntimeError::Vulkan {
        operation: "end_storage_add_artifact_command_buffer",
        code,
    })?;
    let fence =
        unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_storage_add_artifact_fence",
                code,
            }
        })?;
    let submit_info =
        vk::SubmitInfo::default().command_buffers(std::slice::from_ref(&command_buffer));
    unsafe { device.queue_submit(queue, std::slice::from_ref(&submit_info), fence) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "submit_storage_add_artifact_queue",
            code,
        },
    )?;
    unsafe { device.wait_for_fences(std::slice::from_ref(&fence), true, u64::MAX) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "wait_storage_add_artifact_fence",
            code,
        },
    )?;
    let mut output_values = vec![0_u32; STORAGE_ADD_ARTIFACT_ELEMENT_COUNT];
    unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                0,
                element_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_storage_add_artifact_readback",
                code,
            })?;
        for (index, value) in output_values.iter_mut().enumerate() {
            *value = std::ptr::read_unaligned(mapped.cast::<u32>().add(index));
        }
        device.unmap_memory(storage_memory);
    }
    let mut expected_values = input_values.clone();
    expected_values[0] = expected_values[0]
        .checked_add(STORAGE_ADD_ARTIFACT_ADDEND)
        .expect("storage-add fixture cannot overflow");
    let untouched_tail_count = output_values
        .iter()
        .skip(1)
        .zip(input_values.iter().skip(1))
        .filter(|(actual, input)| actual == input)
        .count();
    let differential_passed = compare_u32(&expected_values, &output_values).is_ok()
        && untouched_tail_count == STORAGE_ADD_ARTIFACT_ELEMENT_COUNT - 1;
    let residency_passed = resource_table
        .release_prepared_artifact_dispatch(prepared)
        .is_ok()
        && resource_table.evict(resource_id).is_ok();
    let output_checksum = output_values.iter().map(|value| u64::from(*value)).sum();
    let expected_checksum = expected_values.iter().map(|value| u64::from(*value)).sum();
    unsafe {
        device.destroy_fence(fence, None);
        device.destroy_command_pool(command_pool, None);
        device.destroy_pipeline(pipeline, None);
        device.destroy_pipeline_layout(pipeline_layout, None);
        device.destroy_shader_module(shader_module, None);
        device.destroy_descriptor_pool(descriptor_pool, None);
        device.destroy_descriptor_set_layout(descriptor_set_layout, None);
        device.free_memory(storage_memory, None);
        device.destroy_buffer(storage_buffer, None);
    }
    Ok(StorageAddArtifactQueueSmokeReport {
        schema: "jadren-vulkan-storage-add-artifact-smoke-0.1",
        physical_device_count,
        selected_device,
        queue_family_index,
        artifact_entry_name: descriptor.artifact.entry_name,
        artifact_word_count: descriptor.artifact.word_count,
        artifact_word_hash: descriptor.artifact.word_hash,
        artifact_validated: true,
        queue_execution: "passed",
        descriptor_setup: "passed",
        resource_binding_count: descriptor.resources.len(),
        pipeline_execution: "passed",
        fence_execution: "passed",
        data_kernel_execution: if differential_passed {
            "passed"
        } else {
            "failed"
        },
        differential_execution: if differential_passed {
            "passed"
        } else {
            "failed"
        },
        residency_execution: if residency_passed { "passed" } else { "failed" },
        element_count: STORAGE_ADD_ARTIFACT_ELEMENT_COUNT,
        addend: STORAGE_ADD_ARTIFACT_ADDEND,
        output_checksum,
        expected_checksum,
        first_output: output_values[0],
        last_output: output_values[STORAGE_ADD_ARTIFACT_ELEMENT_COUNT - 1],
        untouched_tail_count,
    })
}

fn run_global_strided_write_on_instance(
    instance: &Instance,
) -> Result<GlobalStridedWriteU32QueueSmokeReport, RuntimeError> {
    let physical_devices =
        unsafe { instance.enumerate_physical_devices() }.map_err(|code| RuntimeError::Vulkan {
            operation: "enumerate_global_strided_write_physical_devices",
            code,
        })?;
    let (physical_device, queue_family_index) = physical_devices
        .iter()
        .find_map(|physical_device| {
            let families =
                unsafe { instance.get_physical_device_queue_family_properties(*physical_device) };
            families
                .iter()
                .enumerate()
                .find(|(_, properties)| {
                    properties.queue_count > 0
                        && properties.queue_flags.contains(vk::QueueFlags::COMPUTE)
                })
                .map(|(index, _)| (*physical_device, index as u32))
        })
        .ok_or(RuntimeError::NoComputeQueue)?;
    let properties = unsafe { instance.get_physical_device_properties(physical_device) };
    let selected_device = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    let priorities = [1.0_f32];
    let queue_info = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family_index)
        .queue_priorities(&priorities);
    let device_info =
        vk::DeviceCreateInfo::default().queue_create_infos(std::slice::from_ref(&queue_info));
    let device =
        unsafe { instance.create_device(physical_device, &device_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_strided_write_device",
                code,
            }
        })?;
    let result = run_global_strided_write_on_device(
        instance,
        &device,
        physical_device,
        queue_family_index,
        physical_devices.len(),
        selected_device,
    );
    unsafe {
        let _ = device.device_wait_idle();
        device.destroy_device(None);
    }
    result
}

fn run_global_2d_write_on_instance(
    instance: &Instance,
) -> Result<Global2dWriteU32QueueSmokeReport, RuntimeError> {
    let physical_devices =
        unsafe { instance.enumerate_physical_devices() }.map_err(|code| RuntimeError::Vulkan {
            operation: "enumerate_global_2d_write_physical_devices",
            code,
        })?;
    let (physical_device, queue_family_index) = physical_devices
        .iter()
        .find_map(|physical_device| {
            let families =
                unsafe { instance.get_physical_device_queue_family_properties(*physical_device) };
            families
                .iter()
                .enumerate()
                .find(|(_, properties)| {
                    properties.queue_count > 0
                        && properties.queue_flags.contains(vk::QueueFlags::COMPUTE)
                })
                .map(|(index, _)| (*physical_device, index as u32))
        })
        .ok_or(RuntimeError::NoComputeQueue)?;
    let properties = unsafe { instance.get_physical_device_properties(physical_device) };
    let selected_device = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    let priorities = [1.0_f32];
    let queue_info = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family_index)
        .queue_priorities(&priorities);
    let device_info =
        vk::DeviceCreateInfo::default().queue_create_infos(std::slice::from_ref(&queue_info));
    let device =
        unsafe { instance.create_device(physical_device, &device_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_2d_write_device",
                code,
            }
        })?;
    let result = run_global_2d_write_on_device(
        instance,
        &device,
        physical_device,
        queue_family_index,
        physical_devices.len(),
        selected_device,
    );
    unsafe {
        let _ = device.device_wait_idle();
        device.destroy_device(None);
    }
    result
}

fn run_global_2d_strided_write_on_instance(
    instance: &Instance,
) -> Result<Global2dStridedWriteU32QueueSmokeReport, RuntimeError> {
    let physical_devices =
        unsafe { instance.enumerate_physical_devices() }.map_err(|code| RuntimeError::Vulkan {
            operation: "enumerate_global_2d_write_physical_devices",
            code,
        })?;
    let (physical_device, queue_family_index) = physical_devices
        .iter()
        .find_map(|physical_device| {
            let families =
                unsafe { instance.get_physical_device_queue_family_properties(*physical_device) };
            families
                .iter()
                .enumerate()
                .find(|(_, properties)| {
                    properties.queue_count > 0
                        && properties.queue_flags.contains(vk::QueueFlags::COMPUTE)
                })
                .map(|(index, _)| (*physical_device, index as u32))
        })
        .ok_or(RuntimeError::NoComputeQueue)?;
    let properties = unsafe { instance.get_physical_device_properties(physical_device) };
    let selected_device = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    let priorities = [1.0_f32];
    let queue_info = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family_index)
        .queue_priorities(&priorities);
    let device_info =
        vk::DeviceCreateInfo::default().queue_create_infos(std::slice::from_ref(&queue_info));
    let device =
        unsafe { instance.create_device(physical_device, &device_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_2d_strided_write_device",
                code,
            }
        })?;
    let result = run_global_2d_strided_write_on_device(
        instance,
        &device,
        physical_device,
        queue_family_index,
        physical_devices.len(),
        selected_device,
    );
    unsafe {
        let _ = device.device_wait_idle();
        device.destroy_device(None);
    }
    result
}

fn run_global_strided_write_on_device(
    instance: &Instance,
    device: &ash::Device,
    physical_device: vk::PhysicalDevice,
    queue_family_index: u32,
    physical_device_count: usize,
    selected_device: String,
) -> Result<GlobalStridedWriteU32QueueSmokeReport, RuntimeError> {
    let queue = unsafe { device.get_device_queue(queue_family_index, 0) };
    let kernel = storage_global_index_strided_write_fixture_module();
    let artifact = emit_storage_global_index_strided_write_artifact_from_jir(
        &kernel,
        FunctionId::new(0),
        SpirvOptions::new([STRIDED_WRITE_WORKGROUP_SIZE, 1, 1])
            .map_err(|error| RuntimeError::Codegen(error.to_string()))?,
    )
    .map_err(|error| RuntimeError::Codegen(error.to_string()))?;
    let resources = &artifact.resources;
    if resources.len() != 4
        || resources
            .iter()
            .any(|resource| resource.address_space != AddressSpace::Storage)
        || resources.iter().any(|resource| {
            !matches!(
                kernel.types.get(resource.element_type.index()),
                Some(Type::Integer {
                    signed: false,
                    bits: 32
                })
            )
        })
    {
        return Err(RuntimeError::DescriptorContract(
            "global-strided-write kernel must expose four storage u32 resources".to_owned(),
        ));
    }
    let descriptor_bindings = resources
        .iter()
        .map(|resource| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE)
        })
        .collect::<Vec<_>>();
    let shader_info = vk::ShaderModuleCreateInfo::default().code(&artifact.words);
    let shader_module =
        unsafe { device.create_shader_module(&shader_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_strided_write_shader_module",
                code,
            }
        })?;
    let entry_name =
        CString::new(artifact.entry_name.as_str()).expect("validated JIR entry name has no NUL");
    let stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(shader_module)
        .name(&entry_name);
    let buffer_bytes = (STRIDED_WRITE_CAPACITY * std::mem::size_of::<u32>()) as u64;
    let storage_buffer_info = vk::BufferCreateInfo::default()
        .size(STRIDED_WRITE_BUFFER_SIZE)
        .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let storage_buffer =
        unsafe { device.create_buffer(&storage_buffer_info, None) }.map_err(|code| {
            unsafe { device.destroy_shader_module(shader_module, None) };
            RuntimeError::Vulkan {
                operation: "create_global_strided_write_storage_buffer",
                code,
            }
        })?;
    let requirements = unsafe { device.get_buffer_memory_requirements(storage_buffer) };
    let memory_properties =
        unsafe { instance.get_physical_device_memory_properties(physical_device) };
    let memory_type_index = (0..memory_properties.memory_type_count)
        .find(|index| {
            let compatible = requirements.memory_type_bits & (1_u32 << index) != 0;
            compatible
                && memory_properties.memory_types[*index as usize]
                    .property_flags
                    .contains(
                        vk::MemoryPropertyFlags::HOST_VISIBLE
                            | vk::MemoryPropertyFlags::HOST_COHERENT,
                    )
        })
        .ok_or_else(|| {
            RuntimeError::Loader(
                "no host-visible coherent Vulkan memory type for global-strided-write smoke"
                    .to_owned(),
            )
        })?;
    let allocation_info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_type_index);
    let storage_memory =
        unsafe { device.allocate_memory(&allocation_info, None) }.map_err(|code| {
            unsafe { device.destroy_buffer(storage_buffer, None) };
            RuntimeError::Vulkan {
                operation: "allocate_global_strided_write_storage_memory",
                code,
            }
        })?;
    unsafe { device.bind_buffer_memory(storage_buffer, storage_memory, 0) }.map_err(|code| {
        unsafe {
            device.free_memory(storage_memory, None);
            device.destroy_buffer(storage_buffer, None);
        }
        RuntimeError::Vulkan {
            operation: "bind_global_strided_write_storage_memory",
            code,
        }
    })?;
    let (mut resource_table, resource_id, access_token) =
        begin_storage_scope(STRIDED_WRITE_BUFFER_SIZE)?;
    unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                STRIDED_WRITE_BUFFER_BASE_OFFSET,
                buffer_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_global_strided_write_output_clear",
                code,
            })?;
        for index in 0..STRIDED_WRITE_CAPACITY {
            std::ptr::write_unaligned(mapped.cast::<u32>().add(index), 0);
        }
        device.unmap_memory(storage_memory);
        for (offset, value, operation) in [
            (
                STRIDED_WRITE_LENGTH_BASE_OFFSET,
                STRIDED_WRITE_ELEMENT_COUNT as u32,
                "map_global_strided_write_length",
            ),
            (
                STRIDED_WRITE_STRIDE_BASE_OFFSET,
                STRIDED_WRITE_STRIDE,
                "map_global_strided_write_stride",
            ),
            (
                STRIDED_WRITE_CAPACITY_BASE_OFFSET,
                STRIDED_WRITE_CAPACITY as u32,
                "map_global_strided_write_capacity",
            ),
        ] {
            let mapped = device
                .map_memory(storage_memory, offset, 4, vk::MemoryMapFlags::empty())
                .map_err(|code| RuntimeError::Vulkan { operation, code })?;
            std::ptr::write_unaligned(mapped.cast::<u32>(), value);
            device.unmap_memory(storage_memory);
        }
    }
    let descriptor_layout_info =
        vk::DescriptorSetLayoutCreateInfo::default().bindings(&descriptor_bindings);
    let descriptor_set_layout =
        unsafe { device.create_descriptor_set_layout(&descriptor_layout_info, None) }.map_err(
            |code| RuntimeError::Vulkan {
                operation: "create_global_strided_write_descriptor_set_layout",
                code,
            },
        )?;
    let descriptor_pool_size = vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(4);
    let descriptor_pool_info = vk::DescriptorPoolCreateInfo::default()
        .max_sets(1)
        .pool_sizes(std::slice::from_ref(&descriptor_pool_size));
    let descriptor_pool = unsafe { device.create_descriptor_pool(&descriptor_pool_info, None) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "create_global_strided_write_descriptor_pool",
            code,
        })?;
    let descriptor_set_allocate_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(descriptor_pool)
        .set_layouts(std::slice::from_ref(&descriptor_set_layout));
    let descriptor_set = unsafe { device.allocate_descriptor_sets(&descriptor_set_allocate_info) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "allocate_global_strided_write_descriptor_set",
            code,
        })?[0];
    let descriptor_buffer_infos = [
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(STRIDED_WRITE_BUFFER_BASE_OFFSET)
            .range(buffer_bytes),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(STRIDED_WRITE_LENGTH_BASE_OFFSET)
            .range(4),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(STRIDED_WRITE_STRIDE_BASE_OFFSET)
            .range(4),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(STRIDED_WRITE_CAPACITY_BASE_OFFSET)
            .range(4),
    ];
    let descriptor_writes = resources
        .iter()
        .enumerate()
        .map(|(index, resource)| {
            vk::WriteDescriptorSet::default()
                .dst_set(descriptor_set)
                .dst_binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(std::slice::from_ref(&descriptor_buffer_infos[index]))
        })
        .collect::<Vec<_>>();
    unsafe { device.update_descriptor_sets(&descriptor_writes, &[]) };
    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(std::slice::from_ref(&descriptor_set_layout));
    let pipeline_layout =
        unsafe { device.create_pipeline_layout(&layout_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_strided_write_pipeline_layout",
                code,
            }
        })?;
    let pipeline_info = vk::ComputePipelineCreateInfo::default()
        .stage(stage)
        .layout(pipeline_layout);
    let pipeline = match unsafe {
        device.create_compute_pipelines(
            vk::PipelineCache::null(),
            std::slice::from_ref(&pipeline_info),
            None,
        )
    } {
        Ok(pipelines) => pipelines[0],
        Err((_, code)) => {
            return Err(RuntimeError::Vulkan {
                operation: "create_global_strided_write_compute_pipeline",
                code,
            });
        }
    };
    let pool_info = vk::CommandPoolCreateInfo::default()
        .queue_family_index(queue_family_index)
        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
    let command_pool = unsafe { device.create_command_pool(&pool_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_global_strided_write_command_pool",
            code,
        }
    })?;
    let allocation_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let command_buffer =
        unsafe { device.allocate_command_buffers(&allocation_info) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "allocate_global_strided_write_command_buffer",
                code,
            }
        })?[0];
    unsafe { device.begin_command_buffer(command_buffer, &vk::CommandBufferBeginInfo::default()) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "begin_global_strided_write_command_buffer",
            code,
        })?;
    unsafe {
        device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::COMPUTE, pipeline);
        device.cmd_bind_descriptor_sets(
            command_buffer,
            vk::PipelineBindPoint::COMPUTE,
            pipeline_layout,
            0,
            std::slice::from_ref(&descriptor_set),
            &[],
        );
        let dispatch_x =
            (STRIDED_WRITE_ELEMENT_COUNT as u32).div_ceil(STRIDED_WRITE_WORKGROUP_SIZE);
        device.cmd_dispatch(command_buffer, dispatch_x, 1, 1);
    }
    unsafe { device.end_command_buffer(command_buffer) }.map_err(|code| RuntimeError::Vulkan {
        operation: "end_global_strided_write_command_buffer",
        code,
    })?;
    let fence =
        unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_strided_write_fence",
                code,
            }
        })?;
    let submit_info =
        vk::SubmitInfo::default().command_buffers(std::slice::from_ref(&command_buffer));
    unsafe { device.queue_submit(queue, std::slice::from_ref(&submit_info), fence) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "submit_global_strided_write_queue",
            code,
        },
    )?;
    unsafe { device.wait_for_fences(std::slice::from_ref(&fence), true, u64::MAX) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "wait_global_strided_write_fence",
            code,
        },
    )?;
    let mut output_values = vec![0_u32; STRIDED_WRITE_CAPACITY];
    unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                STRIDED_WRITE_BUFFER_BASE_OFFSET,
                buffer_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_global_strided_write_readback",
                code,
            })?;
        for (index, value) in output_values.iter_mut().enumerate() {
            *value = std::ptr::read_unaligned(mapped.cast::<u32>().add(index));
        }
        device.unmap_memory(storage_memory);
    }
    let mut expected_values = vec![0_u32; STRIDED_WRITE_CAPACITY];
    for index in 0..STRIDED_WRITE_ELEMENT_COUNT {
        expected_values[index * STRIDED_WRITE_STRIDE as usize] = STRIDED_WRITE_VALUE;
    }
    let differential_passed = compare_u32(&expected_values, &output_values).is_ok();
    let residency_passed =
        resource_table.release(access_token).is_ok() && resource_table.evict(resource_id).is_ok();
    let output_checksum = output_values.iter().map(|value| u64::from(*value)).sum();
    let expected_checksum = expected_values.iter().map(|value| u64::from(*value)).sum();
    let untouched_elements = output_values.iter().filter(|value| **value == 0).count();
    unsafe {
        device.destroy_fence(fence, None);
        device.destroy_command_pool(command_pool, None);
        device.destroy_pipeline(pipeline, None);
        device.destroy_pipeline_layout(pipeline_layout, None);
        device.destroy_shader_module(shader_module, None);
        device.destroy_descriptor_pool(descriptor_pool, None);
        device.destroy_descriptor_set_layout(descriptor_set_layout, None);
        device.free_memory(storage_memory, None);
        device.destroy_buffer(storage_buffer, None);
    }
    Ok(GlobalStridedWriteU32QueueSmokeReport {
        schema: "jadren-vulkan-global-strided-write-u32-queue-smoke-0.1",
        artifact_entry_name: artifact.entry_name,
        artifact_word_count: artifact.words.len(),
        artifact_word_hash: stable_spirv_word_hash(&artifact.words),
        artifact_validated: true,
        physical_device_count,
        selected_device,
        queue_family_index,
        queue_execution: "passed",
        descriptor_setup: "passed",
        resource_binding_count: resources.len(),
        pipeline_execution: "passed",
        fence_execution: "passed",
        data_kernel_execution: if differential_passed {
            "passed"
        } else {
            "failed"
        },
        differential_execution: if differential_passed {
            "passed"
        } else {
            "failed"
        },
        residency_execution: if residency_passed { "passed" } else { "failed" },
        logical_length: STRIDED_WRITE_ELEMENT_COUNT,
        capacity: STRIDED_WRITE_CAPACITY,
        stride: STRIDED_WRITE_STRIDE,
        dispatch_x: (STRIDED_WRITE_ELEMENT_COUNT as u32).div_ceil(STRIDED_WRITE_WORKGROUP_SIZE),
        last_physical_index: (STRIDED_WRITE_ELEMENT_COUNT - 1) * STRIDED_WRITE_STRIDE as usize,
        output_checksum,
        expected_checksum,
        untouched_elements,
    })
}

fn run_global_3d_write_on_instance(
    instance: &Instance,
) -> Result<Global3dWriteU32QueueSmokeReport, RuntimeError> {
    let physical_devices =
        unsafe { instance.enumerate_physical_devices() }.map_err(|code| RuntimeError::Vulkan {
            operation: "enumerate_global_3d_write_physical_devices",
            code,
        })?;
    let (physical_device, queue_family_index) = physical_devices
        .iter()
        .find_map(|physical_device| {
            let families =
                unsafe { instance.get_physical_device_queue_family_properties(*physical_device) };
            families
                .iter()
                .enumerate()
                .find(|(_, properties)| {
                    properties.queue_count > 0
                        && properties.queue_flags.contains(vk::QueueFlags::COMPUTE)
                })
                .map(|(index, _)| (*physical_device, index as u32))
        })
        .ok_or(RuntimeError::NoComputeQueue)?;
    let properties = unsafe { instance.get_physical_device_properties(physical_device) };
    let selected_device = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    let priorities = [1.0_f32];
    let queue_info = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family_index)
        .queue_priorities(&priorities);
    let device_info =
        vk::DeviceCreateInfo::default().queue_create_infos(std::slice::from_ref(&queue_info));
    let device =
        unsafe { instance.create_device(physical_device, &device_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_3d_write_device",
                code,
            }
        })?;
    let result = run_global_3d_write_on_device(
        instance,
        &device,
        physical_device,
        queue_family_index,
        physical_devices.len(),
        selected_device,
    );
    unsafe {
        let _ = device.device_wait_idle();
        device.destroy_device(None);
    }
    result
}

fn run_global_3d_write_on_device(
    instance: &Instance,
    device: &ash::Device,
    physical_device: vk::PhysicalDevice,
    queue_family_index: u32,
    physical_device_count: usize,
    selected_device: String,
) -> Result<Global3dWriteU32QueueSmokeReport, RuntimeError> {
    let queue = unsafe { device.get_device_queue(queue_family_index, 0) };
    let kernel = storage_global_3d_write_fixture_module();
    let artifact = emit_storage_global_3d_write_artifact_from_jir(
        &kernel,
        FunctionId::new(0),
        SpirvOptions::new(THREE_D_WRITE_WORKGROUP_SIZE)
            .map_err(|error| RuntimeError::Codegen(error.to_string()))?,
    )
    .map_err(|error| RuntimeError::Codegen(error.to_string()))?;
    let resources = &artifact.resources;
    if resources.len() != 5
        || resources
            .iter()
            .any(|resource| resource.address_space != AddressSpace::Storage)
        || resources.iter().any(|resource| {
            !matches!(
                kernel.types.get(resource.element_type.index()),
                Some(Type::Integer {
                    signed: false,
                    bits: 32
                })
            )
        })
    {
        return Err(RuntimeError::DescriptorContract(
            "global-3d-write kernel must expose five storage u32 resources".to_owned(),
        ));
    }
    let descriptor_bindings = resources
        .iter()
        .map(|resource| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE)
        })
        .collect::<Vec<_>>();
    let shader_info = vk::ShaderModuleCreateInfo::default().code(&artifact.words);
    let shader_module =
        unsafe { device.create_shader_module(&shader_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_3d_write_shader_module",
                code,
            }
        })?;
    let entry_name =
        CString::new(artifact.entry_name.as_str()).expect("validated JIR entry name has no NUL");
    let stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(shader_module)
        .name(&entry_name);
    let buffer_bytes = (THREE_D_WRITE_CAPACITY * std::mem::size_of::<u32>()) as u64;
    let storage_buffer_info = vk::BufferCreateInfo::default()
        .size(THREE_D_WRITE_BUFFER_SIZE)
        .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let storage_buffer =
        unsafe { device.create_buffer(&storage_buffer_info, None) }.map_err(|code| {
            unsafe { device.destroy_shader_module(shader_module, None) };
            RuntimeError::Vulkan {
                operation: "create_global_3d_write_storage_buffer",
                code,
            }
        })?;
    let requirements = unsafe { device.get_buffer_memory_requirements(storage_buffer) };
    let memory_properties =
        unsafe { instance.get_physical_device_memory_properties(physical_device) };
    let memory_type_index = (0..memory_properties.memory_type_count)
        .find(|index| {
            let compatible = requirements.memory_type_bits & (1_u32 << index) != 0;
            compatible
                && memory_properties.memory_types[*index as usize]
                    .property_flags
                    .contains(
                        vk::MemoryPropertyFlags::HOST_VISIBLE
                            | vk::MemoryPropertyFlags::HOST_COHERENT,
                    )
        })
        .ok_or_else(|| {
            RuntimeError::Loader(
                "no host-visible coherent Vulkan memory type for global-3d-write smoke".to_owned(),
            )
        })?;
    let allocation_info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_type_index);
    let storage_memory =
        unsafe { device.allocate_memory(&allocation_info, None) }.map_err(|code| {
            unsafe { device.destroy_buffer(storage_buffer, None) };
            RuntimeError::Vulkan {
                operation: "allocate_global_3d_write_storage_memory",
                code,
            }
        })?;
    unsafe { device.bind_buffer_memory(storage_buffer, storage_memory, 0) }.map_err(|code| {
        unsafe {
            device.free_memory(storage_memory, None);
            device.destroy_buffer(storage_buffer, None);
        }
        RuntimeError::Vulkan {
            operation: "bind_global_3d_write_storage_memory",
            code,
        }
    })?;
    let (mut resource_table, resource_id, access_token) =
        begin_storage_scope(THREE_D_WRITE_BUFFER_SIZE)?;
    unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                THREE_D_WRITE_BUFFER_BASE_OFFSET,
                buffer_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_global_3d_write_output_clear",
                code,
            })?;
        for index in 0..THREE_D_WRITE_CAPACITY {
            std::ptr::write_unaligned(mapped.cast::<u32>().add(index), 0);
        }
        device.unmap_memory(storage_memory);
        for (offset, value, operation) in [
            (
                THREE_D_WRITE_WIDTH_BASE_OFFSET,
                THREE_D_WRITE_WIDTH as u32,
                "map_global_3d_write_width",
            ),
            (
                THREE_D_WRITE_HEIGHT_BASE_OFFSET,
                THREE_D_WRITE_HEIGHT as u32,
                "map_global_3d_write_height",
            ),
            (
                THREE_D_WRITE_DEPTH_BASE_OFFSET,
                THREE_D_WRITE_DEPTH as u32,
                "map_global_3d_write_depth",
            ),
            (
                THREE_D_WRITE_CAPACITY_BASE_OFFSET,
                THREE_D_WRITE_CAPACITY as u32,
                "map_global_3d_write_capacity",
            ),
        ] {
            let mapped = device
                .map_memory(storage_memory, offset, 4, vk::MemoryMapFlags::empty())
                .map_err(|code| RuntimeError::Vulkan { operation, code })?;
            std::ptr::write_unaligned(mapped.cast::<u32>(), value);
            device.unmap_memory(storage_memory);
        }
    }
    let descriptor_layout_info =
        vk::DescriptorSetLayoutCreateInfo::default().bindings(&descriptor_bindings);
    let descriptor_set_layout =
        unsafe { device.create_descriptor_set_layout(&descriptor_layout_info, None) }.map_err(
            |code| RuntimeError::Vulkan {
                operation: "create_global_3d_write_descriptor_set_layout",
                code,
            },
        )?;
    let descriptor_pool_size = vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(5);
    let descriptor_pool_info = vk::DescriptorPoolCreateInfo::default()
        .max_sets(1)
        .pool_sizes(std::slice::from_ref(&descriptor_pool_size));
    let descriptor_pool = unsafe { device.create_descriptor_pool(&descriptor_pool_info, None) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "create_global_3d_write_descriptor_pool",
            code,
        })?;
    let descriptor_set_allocate_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(descriptor_pool)
        .set_layouts(std::slice::from_ref(&descriptor_set_layout));
    let descriptor_set = unsafe { device.allocate_descriptor_sets(&descriptor_set_allocate_info) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "allocate_global_3d_write_descriptor_set",
            code,
        })?[0];
    let descriptor_buffer_infos = [
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(THREE_D_WRITE_BUFFER_BASE_OFFSET)
            .range(buffer_bytes),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(THREE_D_WRITE_WIDTH_BASE_OFFSET)
            .range(4),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(THREE_D_WRITE_HEIGHT_BASE_OFFSET)
            .range(4),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(THREE_D_WRITE_DEPTH_BASE_OFFSET)
            .range(4),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(THREE_D_WRITE_CAPACITY_BASE_OFFSET)
            .range(4),
    ];
    let descriptor_writes = resources
        .iter()
        .enumerate()
        .map(|(index, resource)| {
            vk::WriteDescriptorSet::default()
                .dst_set(descriptor_set)
                .dst_binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(std::slice::from_ref(&descriptor_buffer_infos[index]))
        })
        .collect::<Vec<_>>();
    unsafe { device.update_descriptor_sets(&descriptor_writes, &[]) };
    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(std::slice::from_ref(&descriptor_set_layout));
    let pipeline_layout =
        unsafe { device.create_pipeline_layout(&layout_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_3d_write_pipeline_layout",
                code,
            }
        })?;
    let pipeline_info = vk::ComputePipelineCreateInfo::default()
        .stage(stage)
        .layout(pipeline_layout);
    let pipeline = match unsafe {
        device.create_compute_pipelines(
            vk::PipelineCache::null(),
            std::slice::from_ref(&pipeline_info),
            None,
        )
    } {
        Ok(pipelines) => pipelines[0],
        Err((_, code)) => {
            return Err(RuntimeError::Vulkan {
                operation: "create_global_3d_write_compute_pipeline",
                code,
            });
        }
    };
    let pool_info = vk::CommandPoolCreateInfo::default()
        .queue_family_index(queue_family_index)
        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
    let command_pool = unsafe { device.create_command_pool(&pool_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_global_3d_write_command_pool",
            code,
        }
    })?;
    let allocation_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let command_buffer =
        unsafe { device.allocate_command_buffers(&allocation_info) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "allocate_global_3d_write_command_buffer",
                code,
            }
        })?[0];
    unsafe { device.begin_command_buffer(command_buffer, &vk::CommandBufferBeginInfo::default()) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "begin_global_3d_write_command_buffer",
            code,
        })?;
    unsafe {
        device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::COMPUTE, pipeline);
        device.cmd_bind_descriptor_sets(
            command_buffer,
            vk::PipelineBindPoint::COMPUTE,
            pipeline_layout,
            0,
            std::slice::from_ref(&descriptor_set),
            &[],
        );
        let dispatch_x = (THREE_D_WRITE_WIDTH as u32).div_ceil(THREE_D_WRITE_WORKGROUP_SIZE[0]);
        let dispatch_y = (THREE_D_WRITE_HEIGHT as u32).div_ceil(THREE_D_WRITE_WORKGROUP_SIZE[1]);
        let dispatch_z = (THREE_D_WRITE_DEPTH as u32).div_ceil(THREE_D_WRITE_WORKGROUP_SIZE[2]);
        device.cmd_dispatch(command_buffer, dispatch_x, dispatch_y, dispatch_z);
    }
    unsafe { device.end_command_buffer(command_buffer) }.map_err(|code| RuntimeError::Vulkan {
        operation: "end_global_3d_write_command_buffer",
        code,
    })?;
    let fence =
        unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_3d_write_fence",
                code,
            }
        })?;
    let submit_info =
        vk::SubmitInfo::default().command_buffers(std::slice::from_ref(&command_buffer));
    unsafe { device.queue_submit(queue, std::slice::from_ref(&submit_info), fence) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "submit_global_3d_write_queue",
            code,
        },
    )?;
    unsafe { device.wait_for_fences(std::slice::from_ref(&fence), true, u64::MAX) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "wait_global_3d_write_fence",
            code,
        },
    )?;
    let mut output_values = vec![0_u32; THREE_D_WRITE_CAPACITY];
    unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                THREE_D_WRITE_BUFFER_BASE_OFFSET,
                buffer_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_global_3d_write_readback",
                code,
            })?;
        for (index, value) in output_values.iter_mut().enumerate() {
            *value = std::ptr::read_unaligned(mapped.cast::<u32>().add(index));
        }
        device.unmap_memory(storage_memory);
    }
    let mut expected_values = vec![0_u32; THREE_D_WRITE_CAPACITY];
    for z in 0..THREE_D_WRITE_DEPTH {
        for y in 0..THREE_D_WRITE_HEIGHT {
            for x in 0..THREE_D_WRITE_WIDTH {
                expected_values[(z * THREE_D_WRITE_HEIGHT + y) * THREE_D_WRITE_WIDTH + x] =
                    THREE_D_WRITE_VALUE;
            }
        }
    }
    let differential_passed = compare_u32(&expected_values, &output_values).is_ok();
    let residency_passed =
        resource_table.release(access_token).is_ok() && resource_table.evict(resource_id).is_ok();
    let output_checksum = output_values.iter().map(|value| u64::from(*value)).sum();
    let expected_checksum = expected_values.iter().map(|value| u64::from(*value)).sum();
    let untouched_elements = output_values.iter().filter(|value| **value == 0).count();
    unsafe {
        device.destroy_fence(fence, None);
        device.destroy_command_pool(command_pool, None);
        device.destroy_pipeline(pipeline, None);
        device.destroy_pipeline_layout(pipeline_layout, None);
        device.destroy_shader_module(shader_module, None);
        device.destroy_descriptor_pool(descriptor_pool, None);
        device.destroy_descriptor_set_layout(descriptor_set_layout, None);
        device.free_memory(storage_memory, None);
        device.destroy_buffer(storage_buffer, None);
    }
    Ok(Global3dWriteU32QueueSmokeReport {
        schema: "jadren-vulkan-global-3d-write-u32-queue-smoke-0.1",
        artifact_entry_name: artifact.entry_name,
        artifact_word_count: artifact.words.len(),
        artifact_word_hash: stable_spirv_word_hash(&artifact.words),
        artifact_validated: true,
        physical_device_count,
        selected_device,
        queue_family_index,
        queue_execution: "passed",
        descriptor_setup: "passed",
        resource_binding_count: resources.len(),
        pipeline_execution: "passed",
        fence_execution: "passed",
        data_kernel_execution: if differential_passed {
            "passed"
        } else {
            "failed"
        },
        differential_execution: if differential_passed {
            "passed"
        } else {
            "failed"
        },
        residency_execution: if residency_passed { "passed" } else { "failed" },
        width: THREE_D_WRITE_WIDTH,
        height: THREE_D_WRITE_HEIGHT,
        depth: THREE_D_WRITE_DEPTH,
        capacity: THREE_D_WRITE_CAPACITY,
        dispatch_x: (THREE_D_WRITE_WIDTH as u32).div_ceil(THREE_D_WRITE_WORKGROUP_SIZE[0]),
        dispatch_y: (THREE_D_WRITE_HEIGHT as u32).div_ceil(THREE_D_WRITE_WORKGROUP_SIZE[1]),
        dispatch_z: (THREE_D_WRITE_DEPTH as u32).div_ceil(THREE_D_WRITE_WORKGROUP_SIZE[2]),
        last_physical_index: (THREE_D_WRITE_DEPTH * THREE_D_WRITE_HEIGHT * THREE_D_WRITE_WIDTH) - 1,
        output_checksum,
        expected_checksum,
        untouched_elements,
    })
}

fn run_global_2d_write_on_device(
    instance: &Instance,
    device: &ash::Device,
    physical_device: vk::PhysicalDevice,
    queue_family_index: u32,
    physical_device_count: usize,
    selected_device: String,
) -> Result<Global2dWriteU32QueueSmokeReport, RuntimeError> {
    let queue = unsafe { device.get_device_queue(queue_family_index, 0) };
    let kernel = storage_global_2d_write_fixture_module();
    let artifact = emit_storage_global_2d_write_artifact_from_jir(
        &kernel,
        FunctionId::new(0),
        SpirvOptions::new(TWO_D_WRITE_WORKGROUP_SIZE)
            .map_err(|error| RuntimeError::Codegen(error.to_string()))?,
    )
    .map_err(|error| RuntimeError::Codegen(error.to_string()))?;
    let resources = &artifact.resources;
    if resources.len() != 4
        || resources
            .iter()
            .any(|resource| resource.address_space != AddressSpace::Storage)
        || resources.iter().any(|resource| {
            !matches!(
                kernel.types.get(resource.element_type.index()),
                Some(Type::Integer {
                    signed: false,
                    bits: 32
                })
            )
        })
    {
        return Err(RuntimeError::DescriptorContract(
            "global-2d-write kernel must expose four storage u32 resources".to_owned(),
        ));
    }
    let descriptor_bindings = resources
        .iter()
        .map(|resource| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE)
        })
        .collect::<Vec<_>>();
    let shader_info = vk::ShaderModuleCreateInfo::default().code(&artifact.words);
    let shader_module =
        unsafe { device.create_shader_module(&shader_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_2d_write_shader_module",
                code,
            }
        })?;
    let entry_name =
        CString::new(artifact.entry_name.as_str()).expect("validated JIR entry name has no NUL");
    let stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(shader_module)
        .name(&entry_name);
    let buffer_bytes = (TWO_D_WRITE_CAPACITY * std::mem::size_of::<u32>()) as u64;
    let storage_buffer_info = vk::BufferCreateInfo::default()
        .size(TWO_D_WRITE_BUFFER_SIZE)
        .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let storage_buffer =
        unsafe { device.create_buffer(&storage_buffer_info, None) }.map_err(|code| {
            unsafe { device.destroy_shader_module(shader_module, None) };
            RuntimeError::Vulkan {
                operation: "create_global_2d_write_storage_buffer",
                code,
            }
        })?;
    let requirements = unsafe { device.get_buffer_memory_requirements(storage_buffer) };
    let memory_properties =
        unsafe { instance.get_physical_device_memory_properties(physical_device) };
    let memory_type_index = (0..memory_properties.memory_type_count)
        .find(|index| {
            let compatible = requirements.memory_type_bits & (1_u32 << index) != 0;
            compatible
                && memory_properties.memory_types[*index as usize]
                    .property_flags
                    .contains(
                        vk::MemoryPropertyFlags::HOST_VISIBLE
                            | vk::MemoryPropertyFlags::HOST_COHERENT,
                    )
        })
        .ok_or_else(|| {
            RuntimeError::Loader(
                "no host-visible coherent Vulkan memory type for global-2d-write smoke".to_owned(),
            )
        })?;
    let allocation_info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_type_index);
    let storage_memory =
        unsafe { device.allocate_memory(&allocation_info, None) }.map_err(|code| {
            unsafe { device.destroy_buffer(storage_buffer, None) };
            RuntimeError::Vulkan {
                operation: "allocate_global_2d_write_storage_memory",
                code,
            }
        })?;
    unsafe { device.bind_buffer_memory(storage_buffer, storage_memory, 0) }.map_err(|code| {
        unsafe {
            device.free_memory(storage_memory, None);
            device.destroy_buffer(storage_buffer, None);
        }
        RuntimeError::Vulkan {
            operation: "bind_global_2d_write_storage_memory",
            code,
        }
    })?;
    let (mut resource_table, resource_id, access_token) =
        begin_storage_scope(TWO_D_WRITE_BUFFER_SIZE)?;
    unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                TWO_D_WRITE_BUFFER_BASE_OFFSET,
                buffer_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_global_2d_write_output_clear",
                code,
            })?;
        for index in 0..TWO_D_WRITE_CAPACITY {
            std::ptr::write_unaligned(mapped.cast::<u32>().add(index), 0);
        }
        device.unmap_memory(storage_memory);
        for (offset, value, operation) in [
            (
                TWO_D_WRITE_WIDTH_BASE_OFFSET,
                TWO_D_WRITE_WIDTH as u32,
                "map_global_2d_write_width",
            ),
            (
                TWO_D_WRITE_HEIGHT_BASE_OFFSET,
                TWO_D_WRITE_HEIGHT as u32,
                "map_global_2d_write_height",
            ),
            (
                TWO_D_WRITE_CAPACITY_BASE_OFFSET,
                TWO_D_WRITE_CAPACITY as u32,
                "map_global_2d_write_capacity",
            ),
        ] {
            let mapped = device
                .map_memory(storage_memory, offset, 4, vk::MemoryMapFlags::empty())
                .map_err(|code| RuntimeError::Vulkan { operation, code })?;
            std::ptr::write_unaligned(mapped.cast::<u32>(), value);
            device.unmap_memory(storage_memory);
        }
    }
    let descriptor_layout_info =
        vk::DescriptorSetLayoutCreateInfo::default().bindings(&descriptor_bindings);
    let descriptor_set_layout =
        unsafe { device.create_descriptor_set_layout(&descriptor_layout_info, None) }.map_err(
            |code| RuntimeError::Vulkan {
                operation: "create_global_2d_write_descriptor_set_layout",
                code,
            },
        )?;
    let descriptor_pool_size = vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(4);
    let descriptor_pool_info = vk::DescriptorPoolCreateInfo::default()
        .max_sets(1)
        .pool_sizes(std::slice::from_ref(&descriptor_pool_size));
    let descriptor_pool = unsafe { device.create_descriptor_pool(&descriptor_pool_info, None) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "create_global_2d_write_descriptor_pool",
            code,
        })?;
    let descriptor_set_allocate_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(descriptor_pool)
        .set_layouts(std::slice::from_ref(&descriptor_set_layout));
    let descriptor_set = unsafe { device.allocate_descriptor_sets(&descriptor_set_allocate_info) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "allocate_global_2d_write_descriptor_set",
            code,
        })?[0];
    let descriptor_buffer_infos = [
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(TWO_D_WRITE_BUFFER_BASE_OFFSET)
            .range(buffer_bytes),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(TWO_D_WRITE_WIDTH_BASE_OFFSET)
            .range(4),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(TWO_D_WRITE_HEIGHT_BASE_OFFSET)
            .range(4),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(TWO_D_WRITE_CAPACITY_BASE_OFFSET)
            .range(4),
    ];
    let descriptor_writes = resources
        .iter()
        .enumerate()
        .map(|(index, resource)| {
            vk::WriteDescriptorSet::default()
                .dst_set(descriptor_set)
                .dst_binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(std::slice::from_ref(&descriptor_buffer_infos[index]))
        })
        .collect::<Vec<_>>();
    unsafe { device.update_descriptor_sets(&descriptor_writes, &[]) };
    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(std::slice::from_ref(&descriptor_set_layout));
    let pipeline_layout =
        unsafe { device.create_pipeline_layout(&layout_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_2d_write_pipeline_layout",
                code,
            }
        })?;
    let pipeline_info = vk::ComputePipelineCreateInfo::default()
        .stage(stage)
        .layout(pipeline_layout);
    let pipeline = match unsafe {
        device.create_compute_pipelines(
            vk::PipelineCache::null(),
            std::slice::from_ref(&pipeline_info),
            None,
        )
    } {
        Ok(pipelines) => pipelines[0],
        Err((_, code)) => {
            return Err(RuntimeError::Vulkan {
                operation: "create_global_2d_write_compute_pipeline",
                code,
            });
        }
    };
    let pool_info = vk::CommandPoolCreateInfo::default()
        .queue_family_index(queue_family_index)
        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
    let command_pool = unsafe { device.create_command_pool(&pool_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_global_2d_write_command_pool",
            code,
        }
    })?;
    let allocation_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let command_buffer =
        unsafe { device.allocate_command_buffers(&allocation_info) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "allocate_global_2d_write_command_buffer",
                code,
            }
        })?[0];
    unsafe { device.begin_command_buffer(command_buffer, &vk::CommandBufferBeginInfo::default()) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "begin_global_2d_write_command_buffer",
            code,
        })?;
    unsafe {
        device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::COMPUTE, pipeline);
        device.cmd_bind_descriptor_sets(
            command_buffer,
            vk::PipelineBindPoint::COMPUTE,
            pipeline_layout,
            0,
            std::slice::from_ref(&descriptor_set),
            &[],
        );
        let dispatch_x = (TWO_D_WRITE_WIDTH as u32).div_ceil(TWO_D_WRITE_WORKGROUP_SIZE[0]);
        let dispatch_y = (TWO_D_WRITE_HEIGHT as u32).div_ceil(TWO_D_WRITE_WORKGROUP_SIZE[1]);
        device.cmd_dispatch(command_buffer, dispatch_x, dispatch_y, 1);
    }
    unsafe { device.end_command_buffer(command_buffer) }.map_err(|code| RuntimeError::Vulkan {
        operation: "end_global_2d_write_command_buffer",
        code,
    })?;
    let fence =
        unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_2d_write_fence",
                code,
            }
        })?;
    let submit_info =
        vk::SubmitInfo::default().command_buffers(std::slice::from_ref(&command_buffer));
    unsafe { device.queue_submit(queue, std::slice::from_ref(&submit_info), fence) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "submit_global_2d_write_queue",
            code,
        },
    )?;
    unsafe { device.wait_for_fences(std::slice::from_ref(&fence), true, u64::MAX) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "wait_global_2d_write_fence",
            code,
        },
    )?;
    let mut output_values = vec![0_u32; TWO_D_WRITE_CAPACITY];
    unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                TWO_D_WRITE_BUFFER_BASE_OFFSET,
                buffer_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_global_2d_write_readback",
                code,
            })?;
        for (index, value) in output_values.iter_mut().enumerate() {
            *value = std::ptr::read_unaligned(mapped.cast::<u32>().add(index));
        }
        device.unmap_memory(storage_memory);
    }
    let mut expected_values = vec![0_u32; TWO_D_WRITE_CAPACITY];
    for y in 0..TWO_D_WRITE_HEIGHT {
        for x in 0..TWO_D_WRITE_WIDTH {
            expected_values[y * TWO_D_WRITE_WIDTH + x] = TWO_D_WRITE_VALUE;
        }
    }
    let differential_passed = compare_u32(&expected_values, &output_values).is_ok();
    let residency_passed =
        resource_table.release(access_token).is_ok() && resource_table.evict(resource_id).is_ok();
    let output_checksum = output_values.iter().map(|value| u64::from(*value)).sum();
    let expected_checksum = expected_values.iter().map(|value| u64::from(*value)).sum();
    let untouched_elements = output_values.iter().filter(|value| **value == 0).count();
    unsafe {
        device.destroy_fence(fence, None);
        device.destroy_command_pool(command_pool, None);
        device.destroy_pipeline(pipeline, None);
        device.destroy_pipeline_layout(pipeline_layout, None);
        device.destroy_shader_module(shader_module, None);
        device.destroy_descriptor_pool(descriptor_pool, None);
        device.destroy_descriptor_set_layout(descriptor_set_layout, None);
        device.free_memory(storage_memory, None);
        device.destroy_buffer(storage_buffer, None);
    }
    Ok(Global2dWriteU32QueueSmokeReport {
        schema: "jadren-vulkan-global-2d-write-u32-queue-smoke-0.1",
        artifact_entry_name: artifact.entry_name.clone(),
        artifact_word_count: artifact.words.len(),
        artifact_word_hash: stable_spirv_word_hash(&artifact.words),
        artifact_validated: true,
        physical_device_count,
        selected_device,
        queue_family_index,
        queue_execution: "passed",
        descriptor_setup: "passed",
        resource_binding_count: resources.len(),
        pipeline_execution: "passed",
        fence_execution: "passed",
        data_kernel_execution: if differential_passed {
            "passed"
        } else {
            "failed"
        },
        differential_execution: if differential_passed {
            "passed"
        } else {
            "failed"
        },
        residency_execution: if residency_passed { "passed" } else { "failed" },
        width: TWO_D_WRITE_WIDTH,
        height: TWO_D_WRITE_HEIGHT,
        capacity: TWO_D_WRITE_CAPACITY,
        dispatch_x: (TWO_D_WRITE_WIDTH as u32).div_ceil(TWO_D_WRITE_WORKGROUP_SIZE[0]),
        dispatch_y: (TWO_D_WRITE_HEIGHT as u32).div_ceil(TWO_D_WRITE_WORKGROUP_SIZE[1]),
        last_physical_index: TWO_D_WRITE_WIDTH * TWO_D_WRITE_HEIGHT - 1,
        output_checksum,
        expected_checksum,
        untouched_elements,
    })
}

fn run_global_2d_strided_write_on_device(
    instance: &Instance,
    device: &ash::Device,
    physical_device: vk::PhysicalDevice,
    queue_family_index: u32,
    physical_device_count: usize,
    selected_device: String,
) -> Result<Global2dStridedWriteU32QueueSmokeReport, RuntimeError> {
    let queue = unsafe { device.get_device_queue(queue_family_index, 0) };
    let kernel = storage_global_2d_strided_write_fixture_module();
    let artifact = emit_storage_global_2d_strided_write_artifact_from_jir(
        &kernel,
        FunctionId::new(0),
        SpirvOptions::new(TWO_D_STRIDED_WRITE_WORKGROUP_SIZE)
            .map_err(|error| RuntimeError::Codegen(error.to_string()))?,
    )
    .map_err(|error| RuntimeError::Codegen(error.to_string()))?;
    let resources = &artifact.resources;
    if resources.len() != 6
        || resources
            .iter()
            .any(|resource| resource.address_space != AddressSpace::Storage)
        || resources.iter().any(|resource| {
            !matches!(
                kernel.types.get(resource.element_type.index()),
                Some(Type::Integer {
                    signed: false,
                    bits: 32
                })
            )
        })
    {
        return Err(RuntimeError::DescriptorContract(
            "global-2d-strided-write kernel must expose six storage u32 resources".to_owned(),
        ));
    }
    let descriptor_bindings = resources
        .iter()
        .map(|resource| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE)
        })
        .collect::<Vec<_>>();
    let shader_info = vk::ShaderModuleCreateInfo::default().code(&artifact.words);
    let shader_module =
        unsafe { device.create_shader_module(&shader_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_2d_write_shader_module",
                code,
            }
        })?;
    let entry_name =
        CString::new(artifact.entry_name.as_str()).expect("validated JIR entry name has no NUL");
    let stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(shader_module)
        .name(&entry_name);
    let buffer_bytes = (TWO_D_STRIDED_WRITE_CAPACITY * std::mem::size_of::<u32>()) as u64;
    let storage_buffer_info = vk::BufferCreateInfo::default()
        .size(TWO_D_STRIDED_WRITE_BUFFER_SIZE)
        .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let storage_buffer =
        unsafe { device.create_buffer(&storage_buffer_info, None) }.map_err(|code| {
            unsafe { device.destroy_shader_module(shader_module, None) };
            RuntimeError::Vulkan {
                operation: "create_global_2d_write_storage_buffer",
                code,
            }
        })?;
    let requirements = unsafe { device.get_buffer_memory_requirements(storage_buffer) };
    let memory_properties =
        unsafe { instance.get_physical_device_memory_properties(physical_device) };
    let memory_type_index = (0..memory_properties.memory_type_count)
        .find(|index| {
            let compatible = requirements.memory_type_bits & (1_u32 << index) != 0;
            compatible
                && memory_properties.memory_types[*index as usize]
                    .property_flags
                    .contains(
                        vk::MemoryPropertyFlags::HOST_VISIBLE
                            | vk::MemoryPropertyFlags::HOST_COHERENT,
                    )
        })
        .ok_or_else(|| {
            RuntimeError::Loader(
                "no host-visible coherent Vulkan memory type for global-2d-strided-write smoke"
                    .to_owned(),
            )
        })?;
    let allocation_info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_type_index);
    let storage_memory =
        unsafe { device.allocate_memory(&allocation_info, None) }.map_err(|code| {
            unsafe { device.destroy_buffer(storage_buffer, None) };
            RuntimeError::Vulkan {
                operation: "allocate_global_2d_write_storage_memory",
                code,
            }
        })?;
    unsafe { device.bind_buffer_memory(storage_buffer, storage_memory, 0) }.map_err(|code| {
        unsafe {
            device.free_memory(storage_memory, None);
            device.destroy_buffer(storage_buffer, None);
        }
        RuntimeError::Vulkan {
            operation: "bind_global_2d_write_storage_memory",
            code,
        }
    })?;
    let (mut resource_table, resource_id, access_token) =
        begin_storage_scope(TWO_D_STRIDED_WRITE_BUFFER_SIZE)?;
    unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                TWO_D_STRIDED_WRITE_BUFFER_BASE_OFFSET,
                buffer_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_global_2d_write_output_clear",
                code,
            })?;
        for index in 0..TWO_D_STRIDED_WRITE_CAPACITY {
            std::ptr::write_unaligned(mapped.cast::<u32>().add(index), 0);
        }
        device.unmap_memory(storage_memory);
        for (offset, value, operation) in [
            (
                TWO_D_STRIDED_WRITE_WIDTH_BASE_OFFSET,
                TWO_D_STRIDED_WRITE_WIDTH as u32,
                "map_global_2d_strided_write_width",
            ),
            (
                TWO_D_STRIDED_WRITE_HEIGHT_BASE_OFFSET,
                TWO_D_STRIDED_WRITE_HEIGHT as u32,
                "map_global_2d_strided_write_height",
            ),
            (
                TWO_D_STRIDED_WRITE_STRIDE_X_BASE_OFFSET,
                TWO_D_STRIDED_WRITE_STRIDE_X,
                "map_global_2d_strided_write_stride_x",
            ),
            (
                TWO_D_STRIDED_WRITE_STRIDE_Y_BASE_OFFSET,
                TWO_D_STRIDED_WRITE_STRIDE_Y,
                "map_global_2d_strided_write_stride_y",
            ),
            (
                TWO_D_STRIDED_WRITE_CAPACITY_BASE_OFFSET,
                TWO_D_STRIDED_WRITE_CAPACITY as u32,
                "map_global_2d_strided_write_capacity",
            ),
        ] {
            let mapped = device
                .map_memory(storage_memory, offset, 4, vk::MemoryMapFlags::empty())
                .map_err(|code| RuntimeError::Vulkan { operation, code })?;
            std::ptr::write_unaligned(mapped.cast::<u32>(), value);
            device.unmap_memory(storage_memory);
        }
    }
    let descriptor_layout_info =
        vk::DescriptorSetLayoutCreateInfo::default().bindings(&descriptor_bindings);
    let descriptor_set_layout =
        unsafe { device.create_descriptor_set_layout(&descriptor_layout_info, None) }.map_err(
            |code| RuntimeError::Vulkan {
                operation: "create_global_2d_write_descriptor_set_layout",
                code,
            },
        )?;
    let descriptor_pool_size = vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(6);
    let descriptor_pool_info = vk::DescriptorPoolCreateInfo::default()
        .max_sets(1)
        .pool_sizes(std::slice::from_ref(&descriptor_pool_size));
    let descriptor_pool = unsafe { device.create_descriptor_pool(&descriptor_pool_info, None) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "create_global_2d_write_descriptor_pool",
            code,
        })?;
    let descriptor_set_allocate_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(descriptor_pool)
        .set_layouts(std::slice::from_ref(&descriptor_set_layout));
    let descriptor_set = unsafe { device.allocate_descriptor_sets(&descriptor_set_allocate_info) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "allocate_global_2d_write_descriptor_set",
            code,
        })?[0];
    let descriptor_buffer_infos = [
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(TWO_D_STRIDED_WRITE_BUFFER_BASE_OFFSET)
            .range(buffer_bytes),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(TWO_D_STRIDED_WRITE_WIDTH_BASE_OFFSET)
            .range(4),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(TWO_D_STRIDED_WRITE_HEIGHT_BASE_OFFSET)
            .range(4),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(TWO_D_STRIDED_WRITE_STRIDE_X_BASE_OFFSET)
            .range(4),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(TWO_D_STRIDED_WRITE_STRIDE_Y_BASE_OFFSET)
            .range(4),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(TWO_D_STRIDED_WRITE_CAPACITY_BASE_OFFSET)
            .range(4),
    ];
    let descriptor_writes = resources
        .iter()
        .enumerate()
        .map(|(index, resource)| {
            vk::WriteDescriptorSet::default()
                .dst_set(descriptor_set)
                .dst_binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(std::slice::from_ref(&descriptor_buffer_infos[index]))
        })
        .collect::<Vec<_>>();
    unsafe { device.update_descriptor_sets(&descriptor_writes, &[]) };
    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(std::slice::from_ref(&descriptor_set_layout));
    let pipeline_layout =
        unsafe { device.create_pipeline_layout(&layout_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_2d_write_pipeline_layout",
                code,
            }
        })?;
    let pipeline_info = vk::ComputePipelineCreateInfo::default()
        .stage(stage)
        .layout(pipeline_layout);
    let pipeline = match unsafe {
        device.create_compute_pipelines(
            vk::PipelineCache::null(),
            std::slice::from_ref(&pipeline_info),
            None,
        )
    } {
        Ok(pipelines) => pipelines[0],
        Err((_, code)) => {
            return Err(RuntimeError::Vulkan {
                operation: "create_global_2d_write_compute_pipeline",
                code,
            });
        }
    };
    let pool_info = vk::CommandPoolCreateInfo::default()
        .queue_family_index(queue_family_index)
        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
    let command_pool = unsafe { device.create_command_pool(&pool_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_global_2d_write_command_pool",
            code,
        }
    })?;
    let allocation_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let command_buffer =
        unsafe { device.allocate_command_buffers(&allocation_info) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "allocate_global_2d_write_command_buffer",
                code,
            }
        })?[0];
    unsafe { device.begin_command_buffer(command_buffer, &vk::CommandBufferBeginInfo::default()) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "begin_global_2d_write_command_buffer",
            code,
        })?;
    unsafe {
        device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::COMPUTE, pipeline);
        device.cmd_bind_descriptor_sets(
            command_buffer,
            vk::PipelineBindPoint::COMPUTE,
            pipeline_layout,
            0,
            std::slice::from_ref(&descriptor_set),
            &[],
        );
        let dispatch_x =
            (TWO_D_STRIDED_WRITE_WIDTH as u32).div_ceil(TWO_D_STRIDED_WRITE_WORKGROUP_SIZE[0]);
        let dispatch_y =
            (TWO_D_STRIDED_WRITE_HEIGHT as u32).div_ceil(TWO_D_STRIDED_WRITE_WORKGROUP_SIZE[1]);
        device.cmd_dispatch(command_buffer, dispatch_x, dispatch_y, 1);
    }
    unsafe { device.end_command_buffer(command_buffer) }.map_err(|code| RuntimeError::Vulkan {
        operation: "end_global_2d_write_command_buffer",
        code,
    })?;
    let fence =
        unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_2d_write_fence",
                code,
            }
        })?;
    let submit_info =
        vk::SubmitInfo::default().command_buffers(std::slice::from_ref(&command_buffer));
    unsafe { device.queue_submit(queue, std::slice::from_ref(&submit_info), fence) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "submit_global_2d_write_queue",
            code,
        },
    )?;
    unsafe { device.wait_for_fences(std::slice::from_ref(&fence), true, u64::MAX) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "wait_global_2d_write_fence",
            code,
        },
    )?;
    let mut output_values = vec![0_u32; TWO_D_STRIDED_WRITE_CAPACITY];
    unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                TWO_D_STRIDED_WRITE_BUFFER_BASE_OFFSET,
                buffer_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_global_2d_write_readback",
                code,
            })?;
        for (index, value) in output_values.iter_mut().enumerate() {
            *value = std::ptr::read_unaligned(mapped.cast::<u32>().add(index));
        }
        device.unmap_memory(storage_memory);
    }
    let layout = TensorLayout2D::new(
        TWO_D_STRIDED_WRITE_WIDTH,
        TWO_D_STRIDED_WRITE_HEIGHT,
        TWO_D_STRIDED_WRITE_STRIDE_X as usize,
        TWO_D_STRIDED_WRITE_STRIDE_Y as usize,
        TWO_D_STRIDED_WRITE_CAPACITY,
    )
    .map_err(|error| RuntimeError::DescriptorContract(error.to_string()))?;
    let mut expected_values = vec![0_u32; layout.capacity()];
    for y in 0..TWO_D_STRIDED_WRITE_HEIGHT {
        for x in 0..TWO_D_STRIDED_WRITE_WIDTH {
            let physical_index = layout.physical_index(x, y).ok_or_else(|| {
                RuntimeError::DescriptorContract(
                    "2D affine-stride CPU oracle produced an out-of-capacity index".to_owned(),
                )
            })?;
            expected_values[physical_index] = TWO_D_STRIDED_WRITE_VALUE;
        }
    }
    let differential_passed = compare_u32(&expected_values, &output_values).is_ok();
    let residency_passed =
        resource_table.release(access_token).is_ok() && resource_table.evict(resource_id).is_ok();
    let output_checksum = output_values.iter().map(|value| u64::from(*value)).sum();
    let expected_checksum = expected_values.iter().map(|value| u64::from(*value)).sum();
    let untouched_elements = output_values.iter().filter(|value| **value == 0).count();
    unsafe {
        device.destroy_fence(fence, None);
        device.destroy_command_pool(command_pool, None);
        device.destroy_pipeline(pipeline, None);
        device.destroy_pipeline_layout(pipeline_layout, None);
        device.destroy_shader_module(shader_module, None);
        device.destroy_descriptor_pool(descriptor_pool, None);
        device.destroy_descriptor_set_layout(descriptor_set_layout, None);
        device.free_memory(storage_memory, None);
        device.destroy_buffer(storage_buffer, None);
    }
    Ok(Global2dStridedWriteU32QueueSmokeReport {
        schema: "jadren-vulkan-global-2d-strided-write-u32-queue-smoke-0.1",
        artifact_entry_name: artifact.entry_name.clone(),
        artifact_word_count: artifact.words.len(),
        artifact_word_hash: stable_spirv_word_hash(&artifact.words),
        artifact_validated: true,
        physical_device_count,
        selected_device,
        queue_family_index,
        queue_execution: "passed",
        descriptor_setup: "passed",
        resource_binding_count: resources.len(),
        pipeline_execution: "passed",
        fence_execution: "passed",
        data_kernel_execution: if differential_passed {
            "passed"
        } else {
            "failed"
        },
        differential_execution: if differential_passed {
            "passed"
        } else {
            "failed"
        },
        residency_execution: if residency_passed { "passed" } else { "failed" },
        width: TWO_D_STRIDED_WRITE_WIDTH,
        height: TWO_D_STRIDED_WRITE_HEIGHT,
        stride_x: TWO_D_STRIDED_WRITE_STRIDE_X,
        stride_y: TWO_D_STRIDED_WRITE_STRIDE_Y,
        capacity: TWO_D_STRIDED_WRITE_CAPACITY,
        dispatch_x: (TWO_D_STRIDED_WRITE_WIDTH as u32)
            .div_ceil(TWO_D_STRIDED_WRITE_WORKGROUP_SIZE[0]),
        dispatch_y: (TWO_D_STRIDED_WRITE_HEIGHT as u32)
            .div_ceil(TWO_D_STRIDED_WRITE_WORKGROUP_SIZE[1]),
        last_physical_index: (TWO_D_STRIDED_WRITE_WIDTH - 1)
            * TWO_D_STRIDED_WRITE_STRIDE_X as usize
            + (TWO_D_STRIDED_WRITE_HEIGHT - 1) * TWO_D_STRIDED_WRITE_STRIDE_Y as usize,
        output_checksum,
        expected_checksum,
        untouched_elements,
    })
}

fn run_global_on_instance(instance: &Instance) -> Result<GlobalU32QueueSmokeReport, RuntimeError> {
    let physical_devices =
        unsafe { instance.enumerate_physical_devices() }.map_err(|code| RuntimeError::Vulkan {
            operation: "enumerate_global_index_physical_devices",
            code,
        })?;
    let (physical_device, queue_family_index) = physical_devices
        .iter()
        .find_map(|physical_device| {
            let families =
                unsafe { instance.get_physical_device_queue_family_properties(*physical_device) };
            families
                .iter()
                .enumerate()
                .find(|(_, properties)| {
                    properties.queue_count > 0
                        && properties.queue_flags.contains(vk::QueueFlags::COMPUTE)
                })
                .map(|(index, _)| (*physical_device, index as u32))
        })
        .ok_or(RuntimeError::NoComputeQueue)?;
    let properties = unsafe { instance.get_physical_device_properties(physical_device) };
    let selected_device = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
        .to_string_lossy()
        .into_owned();

    let priorities = [1.0_f32];
    let queue_info = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family_index)
        .queue_priorities(&priorities);
    let device_info =
        vk::DeviceCreateInfo::default().queue_create_infos(std::slice::from_ref(&queue_info));
    let device =
        unsafe { instance.create_device(physical_device, &device_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_index_device",
                code,
            }
        })?;
    let result = run_global_on_device(
        instance,
        &device,
        physical_device,
        queue_family_index,
        physical_devices.len(),
        selected_device,
    );
    unsafe {
        let _ = device.device_wait_idle();
        device.destroy_device(None);
    }
    result
}

fn run_global_on_device(
    instance: &Instance,
    device: &ash::Device,
    physical_device: vk::PhysicalDevice,
    queue_family_index: u32,
    physical_device_count: usize,
    selected_device: String,
) -> Result<GlobalU32QueueSmokeReport, RuntimeError> {
    let queue = unsafe { device.get_device_queue(queue_family_index, 0) };
    let kernel = storage_global_index_add_fixture_module();
    let resources = reflect_resources(&kernel, FunctionId::new(0)).map_err(|error| {
        RuntimeError::DescriptorContract(format!("global-index JIR reflection failed: {error}"))
    })?;
    let descriptor_bindings = resources
        .iter()
        .map(|resource| {
            if resource.address_space != AddressSpace::Storage {
                return Err(RuntimeError::DescriptorContract(format!(
                    "resource `{}` is not storage address space",
                    resource.name
                )));
            }
            Ok(vk::DescriptorSetLayoutBinding::default()
                .binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if descriptor_bindings.len() != 2 {
        return Err(RuntimeError::DescriptorContract(
            "global-index kernel must expose exactly input/output storage resources".to_owned(),
        ));
    }
    let spirv = emit_storage_global_index_add_from_jir(
        &kernel,
        FunctionId::new(0),
        SpirvOptions::new([GLOBAL_KERNEL_WORKGROUP_SIZE, 1, 1])
            .map_err(|error| RuntimeError::Codegen(error.to_string()))?,
    )
    .map_err(|error| RuntimeError::Codegen(error.to_string()))?;
    let shader_info = vk::ShaderModuleCreateInfo::default().code(&spirv);
    let shader_module =
        unsafe { device.create_shader_module(&shader_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_index_shader_module",
                code,
            }
        })?;
    let entry_name = CString::new(kernel.functions[0].name.as_str())
        .expect("validated JIR entry name has no NUL");
    let stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(shader_module)
        .name(&entry_name);
    let storage_buffer_info = vk::BufferCreateInfo::default()
        .size(GLOBAL_KERNEL_BUFFER_SIZE)
        .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let storage_buffer =
        unsafe { device.create_buffer(&storage_buffer_info, None) }.map_err(|code| {
            unsafe { device.destroy_shader_module(shader_module, None) };
            RuntimeError::Vulkan {
                operation: "create_global_index_storage_buffer",
                code,
            }
        })?;
    let requirements = unsafe { device.get_buffer_memory_requirements(storage_buffer) };
    let memory_properties =
        unsafe { instance.get_physical_device_memory_properties(physical_device) };
    let memory_type_index = (0..memory_properties.memory_type_count)
        .find(|index| {
            let compatible = requirements.memory_type_bits & (1_u32 << index) != 0;
            compatible
                && memory_properties.memory_types[*index as usize]
                    .property_flags
                    .contains(
                        vk::MemoryPropertyFlags::HOST_VISIBLE
                            | vk::MemoryPropertyFlags::HOST_COHERENT,
                    )
        })
        .ok_or_else(|| {
            RuntimeError::Loader(
                "no host-visible coherent Vulkan memory type for global-index smoke".to_owned(),
            )
        })?;
    let allocation_info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_type_index);
    let storage_memory =
        unsafe { device.allocate_memory(&allocation_info, None) }.map_err(|code| {
            unsafe { device.destroy_buffer(storage_buffer, None) };
            RuntimeError::Vulkan {
                operation: "allocate_global_index_storage_memory",
                code,
            }
        })?;
    unsafe { device.bind_buffer_memory(storage_buffer, storage_memory, 0) }.map_err(|code| {
        unsafe {
            device.free_memory(storage_memory, None);
            device.destroy_buffer(storage_buffer, None);
        }
        RuntimeError::Vulkan {
            operation: "bind_global_index_storage_memory",
            code,
        }
    })?;
    let (mut resource_table, resource_id, access_token) =
        begin_storage_scope(GLOBAL_KERNEL_BUFFER_SIZE)?;
    let input_values: Vec<u32> = (0..GLOBAL_KERNEL_ELEMENT_COUNT)
        .map(|index| 7_u32.saturating_add(index as u32 * 3))
        .collect();
    let expected_values: Vec<u32> = input_values
        .iter()
        .map(|value| value.saturating_add(GLOBAL_KERNEL_ADDEND))
        .collect();
    let element_bytes = (GLOBAL_KERNEL_ELEMENT_COUNT * std::mem::size_of::<u32>()) as u64;
    unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                GLOBAL_KERNEL_INPUT_BASE_OFFSET,
                element_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_global_index_input",
                code,
            })?;
        for (index, value) in input_values.iter().enumerate() {
            std::ptr::write_unaligned(mapped.cast::<u32>().add(index), *value);
        }
        device.unmap_memory(storage_memory);
        let mapped = device
            .map_memory(
                storage_memory,
                GLOBAL_KERNEL_OUTPUT_BASE_OFFSET,
                element_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_global_index_output_clear",
                code,
            })?;
        for index in 0..GLOBAL_KERNEL_ELEMENT_COUNT {
            std::ptr::write_unaligned(mapped.cast::<u32>().add(index), 0);
        }
        device.unmap_memory(storage_memory);
    }
    let descriptor_layout_info =
        vk::DescriptorSetLayoutCreateInfo::default().bindings(&descriptor_bindings);
    let descriptor_set_layout =
        unsafe { device.create_descriptor_set_layout(&descriptor_layout_info, None) }.map_err(
            |code| RuntimeError::Vulkan {
                operation: "create_global_index_descriptor_set_layout",
                code,
            },
        )?;
    let descriptor_pool_size = vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(2);
    let descriptor_pool_info = vk::DescriptorPoolCreateInfo::default()
        .max_sets(1)
        .pool_sizes(std::slice::from_ref(&descriptor_pool_size));
    let descriptor_pool = unsafe { device.create_descriptor_pool(&descriptor_pool_info, None) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "create_global_index_descriptor_pool",
            code,
        })?;
    let descriptor_set_allocate_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(descriptor_pool)
        .set_layouts(std::slice::from_ref(&descriptor_set_layout));
    let descriptor_set = unsafe { device.allocate_descriptor_sets(&descriptor_set_allocate_info) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "allocate_global_index_descriptor_set",
            code,
        })?[0];
    let descriptor_buffer_infos = [
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(GLOBAL_KERNEL_INPUT_BASE_OFFSET)
            .range(element_bytes),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(GLOBAL_KERNEL_OUTPUT_BASE_OFFSET)
            .range(element_bytes),
    ];
    let descriptor_writes = resources
        .iter()
        .enumerate()
        .map(|(index, resource)| {
            vk::WriteDescriptorSet::default()
                .dst_set(descriptor_set)
                .dst_binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(std::slice::from_ref(&descriptor_buffer_infos[index]))
        })
        .collect::<Vec<_>>();
    unsafe { device.update_descriptor_sets(&descriptor_writes, &[]) };
    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(std::slice::from_ref(&descriptor_set_layout));
    let pipeline_layout =
        unsafe { device.create_pipeline_layout(&layout_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_index_pipeline_layout",
                code,
            }
        })?;
    let pipeline_info = vk::ComputePipelineCreateInfo::default()
        .stage(stage)
        .layout(pipeline_layout);
    let pipeline = match unsafe {
        device.create_compute_pipelines(
            vk::PipelineCache::null(),
            std::slice::from_ref(&pipeline_info),
            None,
        )
    } {
        Ok(pipelines) => pipelines[0],
        Err((_, code)) => {
            return Err(RuntimeError::Vulkan {
                operation: "create_global_index_compute_pipeline",
                code,
            });
        }
    };
    let pool_info = vk::CommandPoolCreateInfo::default()
        .queue_family_index(queue_family_index)
        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
    let command_pool = unsafe { device.create_command_pool(&pool_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_global_index_command_pool",
            code,
        }
    })?;
    let allocation_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let command_buffer =
        unsafe { device.allocate_command_buffers(&allocation_info) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "allocate_global_index_command_buffer",
                code,
            }
        })?[0];
    unsafe { device.begin_command_buffer(command_buffer, &vk::CommandBufferBeginInfo::default()) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "begin_global_index_command_buffer",
            code,
        })?;
    unsafe {
        device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::COMPUTE, pipeline);
        device.cmd_bind_descriptor_sets(
            command_buffer,
            vk::PipelineBindPoint::COMPUTE,
            pipeline_layout,
            0,
            std::slice::from_ref(&descriptor_set),
            &[],
        );
        device.cmd_dispatch(command_buffer, 1, 1, 1);
    }
    unsafe { device.end_command_buffer(command_buffer) }.map_err(|code| RuntimeError::Vulkan {
        operation: "end_global_index_command_buffer",
        code,
    })?;
    let fence =
        unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_index_fence",
                code,
            }
        })?;
    let submit_info =
        vk::SubmitInfo::default().command_buffers(std::slice::from_ref(&command_buffer));
    unsafe { device.queue_submit(queue, std::slice::from_ref(&submit_info), fence) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "submit_global_index_queue",
            code,
        },
    )?;
    unsafe { device.wait_for_fences(std::slice::from_ref(&fence), true, u64::MAX) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "wait_global_index_fence",
            code,
        },
    )?;
    let mut output_values = vec![0_u32; GLOBAL_KERNEL_ELEMENT_COUNT];
    unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                GLOBAL_KERNEL_OUTPUT_BASE_OFFSET,
                element_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_global_index_readback",
                code,
            })?;
        for (index, value) in output_values.iter_mut().enumerate() {
            *value = std::ptr::read_unaligned(mapped.cast::<u32>().add(index));
        }
        device.unmap_memory(storage_memory);
    }
    let differential_passed = compare_u32(&expected_values, &output_values).is_ok();
    let residency_passed =
        resource_table.release(access_token).is_ok() && resource_table.evict(resource_id).is_ok();
    let input_checksum = input_values.iter().map(|value| u64::from(*value)).sum();
    let output_checksum = output_values.iter().map(|value| u64::from(*value)).sum();
    let expected_checksum = expected_values.iter().map(|value| u64::from(*value)).sum();
    unsafe {
        device.destroy_fence(fence, None);
        device.destroy_command_pool(command_pool, None);
        device.destroy_pipeline(pipeline, None);
        device.destroy_pipeline_layout(pipeline_layout, None);
        device.destroy_shader_module(shader_module, None);
        device.destroy_descriptor_pool(descriptor_pool, None);
        device.destroy_descriptor_set_layout(descriptor_set_layout, None);
        device.free_memory(storage_memory, None);
        device.destroy_buffer(storage_buffer, None);
    }
    Ok(GlobalU32QueueSmokeReport {
        schema: "jadren-vulkan-global-u32-queue-smoke-0.1",
        physical_device_count,
        selected_device,
        queue_family_index,
        queue_execution: "passed",
        descriptor_setup: "passed",
        resource_binding_count: resources.len(),
        pipeline_execution: "passed",
        fence_execution: "passed",
        data_kernel_execution: if differential_passed {
            "passed"
        } else {
            "failed"
        },
        differential_execution: if differential_passed {
            "passed"
        } else {
            "failed"
        },
        residency_execution: if residency_passed { "passed" } else { "failed" },
        element_count: GLOBAL_KERNEL_ELEMENT_COUNT,
        input_checksum,
        output_checksum,
        expected_checksum,
        first_output: output_values[0],
        last_output: output_values[GLOBAL_KERNEL_ELEMENT_COUNT - 1],
    })
}

fn run_global_dynamic_on_device(
    context: GlobalDynamicDeviceContext<'_>,
) -> Result<GlobalDynamicU32Execution, RuntimeError> {
    let GlobalDynamicDeviceContext {
        instance,
        device,
        physical_device,
        queue_family_index,
        physical_device_count,
        selected_device,
        input_values,
        operation,
        operand,
    } = context;
    let queue = unsafe { device.get_device_queue(queue_family_index, 0) };
    let kernel =
        storage_global_dynamic_index_arithmetic_fixture_module_with_operand(operation, operand);
    let artifact = emit_storage_global_index_binary_dynamic_length_artifact_from_jir(
        &kernel,
        FunctionId::new(0),
        SpirvOptions::new([GLOBAL_DYNAMIC_WORKGROUP_SIZE, 1, 1])
            .map_err(|error| RuntimeError::Codegen(error.to_string()))?,
        operation.jir_op(),
    )
    .map_err(|error| RuntimeError::Codegen(error.to_string()))?;
    validate_spirv_artifact_contract(&artifact).map_err(|error| {
        RuntimeError::Codegen(format!(
            "dynamic-length artifact validation failed: {error}"
        ))
    })?;
    let resources = &artifact.resources;
    if resources.len() != 3
        || resources
            .iter()
            .any(|resource| resource.address_space != AddressSpace::Storage)
    {
        return Err(RuntimeError::DescriptorContract(
            "dynamic-length kernel must expose three storage resources".to_owned(),
        ));
    }
    let descriptor_bindings = resources
        .iter()
        .map(|resource| {
            Ok(vk::DescriptorSetLayoutBinding::default()
                .binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE))
        })
        .collect::<Result<Vec<_>, RuntimeError>>()?;
    let shader_info = vk::ShaderModuleCreateInfo::default().code(&artifact.words);
    let shader_module =
        unsafe { device.create_shader_module(&shader_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_dynamic_length_shader_module",
                code,
            }
        })?;
    let entry_name =
        CString::new(artifact.entry_name.as_str()).expect("validated JIR entry name has no NUL");
    let stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(shader_module)
        .name(&entry_name);
    let storage_buffer_info = vk::BufferCreateInfo::default()
        .size(GLOBAL_DYNAMIC_BUFFER_SIZE)
        .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let storage_buffer =
        unsafe { device.create_buffer(&storage_buffer_info, None) }.map_err(|code| {
            unsafe { device.destroy_shader_module(shader_module, None) };
            RuntimeError::Vulkan {
                operation: "create_dynamic_length_storage_buffer",
                code,
            }
        })?;
    let requirements = unsafe { device.get_buffer_memory_requirements(storage_buffer) };
    let memory_properties =
        unsafe { instance.get_physical_device_memory_properties(physical_device) };
    let memory_type_index = (0..memory_properties.memory_type_count)
        .find(|index| {
            let compatible = requirements.memory_type_bits & (1_u32 << index) != 0;
            compatible
                && memory_properties.memory_types[*index as usize]
                    .property_flags
                    .contains(
                        vk::MemoryPropertyFlags::HOST_VISIBLE
                            | vk::MemoryPropertyFlags::HOST_COHERENT,
                    )
        })
        .ok_or_else(|| {
            RuntimeError::Loader(format!(
                "no host-visible coherent Vulkan memory type for dynamic-length {} smoke",
                operation.label()
            ))
        })?;
    let allocation_info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_type_index);
    let storage_memory =
        unsafe { device.allocate_memory(&allocation_info, None) }.map_err(|code| {
            unsafe { device.destroy_buffer(storage_buffer, None) };
            RuntimeError::Vulkan {
                operation: "allocate_dynamic_length_storage_memory",
                code,
            }
        })?;
    unsafe { device.bind_buffer_memory(storage_buffer, storage_memory, 0) }.map_err(|code| {
        unsafe {
            device.free_memory(storage_memory, None);
            device.destroy_buffer(storage_buffer, None);
        }
        RuntimeError::Vulkan {
            operation: "bind_dynamic_length_storage_memory",
            code,
        }
    })?;
    let (mut resource_table, resource_id, access_token) =
        begin_storage_scope(GLOBAL_DYNAMIC_BUFFER_SIZE)?;
    let capacity_bytes = (GLOBAL_DYNAMIC_CAPACITY * std::mem::size_of::<u32>()) as u64;
    let element_count = input_values.len();
    let expected_values: Vec<u32> = input_values
        .iter()
        .map(|value| operation.apply_with_operand(*value, operand))
        .collect();
    unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                GLOBAL_DYNAMIC_INPUT_BASE_OFFSET,
                capacity_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_dynamic_length_input",
                code,
            })?;
        for (index, value) in input_values.iter().enumerate() {
            std::ptr::write_unaligned(mapped.cast::<u32>().add(index), *value);
        }
        device.unmap_memory(storage_memory);
        let mapped = device
            .map_memory(
                storage_memory,
                GLOBAL_DYNAMIC_OUTPUT_BASE_OFFSET,
                capacity_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_dynamic_length_output_clear",
                code,
            })?;
        for index in 0..GLOBAL_DYNAMIC_CAPACITY {
            std::ptr::write_unaligned(mapped.cast::<u32>().add(index), 0);
        }
        device.unmap_memory(storage_memory);
        let mapped = device
            .map_memory(
                storage_memory,
                GLOBAL_DYNAMIC_LENGTH_BASE_OFFSET,
                4,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_dynamic_length_upload",
                code,
            })?;
        std::ptr::write_unaligned(mapped.cast::<u32>(), element_count as u32);
        device.unmap_memory(storage_memory);
    }
    let descriptor_layout_info =
        vk::DescriptorSetLayoutCreateInfo::default().bindings(&descriptor_bindings);
    let descriptor_set_layout =
        unsafe { device.create_descriptor_set_layout(&descriptor_layout_info, None) }.map_err(
            |code| RuntimeError::Vulkan {
                operation: "create_dynamic_length_descriptor_set_layout",
                code,
            },
        )?;
    let descriptor_pool_size = vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(3);
    let descriptor_pool_info = vk::DescriptorPoolCreateInfo::default()
        .max_sets(1)
        .pool_sizes(std::slice::from_ref(&descriptor_pool_size));
    let descriptor_pool = unsafe { device.create_descriptor_pool(&descriptor_pool_info, None) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "create_dynamic_length_descriptor_pool",
            code,
        })?;
    let descriptor_set_allocate_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(descriptor_pool)
        .set_layouts(std::slice::from_ref(&descriptor_set_layout));
    let descriptor_set = unsafe { device.allocate_descriptor_sets(&descriptor_set_allocate_info) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "allocate_dynamic_length_descriptor_set",
            code,
        })?[0];
    let descriptor_buffer_infos = [
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(GLOBAL_DYNAMIC_INPUT_BASE_OFFSET)
            .range(capacity_bytes),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(GLOBAL_DYNAMIC_OUTPUT_BASE_OFFSET)
            .range(capacity_bytes),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(GLOBAL_DYNAMIC_LENGTH_BASE_OFFSET)
            .range(4),
    ];
    let descriptor_writes = resources
        .iter()
        .enumerate()
        .map(|(index, resource)| {
            vk::WriteDescriptorSet::default()
                .dst_set(descriptor_set)
                .dst_binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(std::slice::from_ref(&descriptor_buffer_infos[index]))
        })
        .collect::<Vec<_>>();
    unsafe { device.update_descriptor_sets(&descriptor_writes, &[]) };
    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(std::slice::from_ref(&descriptor_set_layout));
    let pipeline_layout =
        unsafe { device.create_pipeline_layout(&layout_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_dynamic_length_pipeline_layout",
                code,
            }
        })?;
    let pipeline_info = vk::ComputePipelineCreateInfo::default()
        .stage(stage)
        .layout(pipeline_layout);
    let pipeline = match unsafe {
        device.create_compute_pipelines(
            vk::PipelineCache::null(),
            std::slice::from_ref(&pipeline_info),
            None,
        )
    } {
        Ok(pipelines) => pipelines[0],
        Err((_, code)) => {
            return Err(RuntimeError::Vulkan {
                operation: "create_dynamic_length_compute_pipeline",
                code,
            });
        }
    };
    let pool_info = vk::CommandPoolCreateInfo::default()
        .queue_family_index(queue_family_index)
        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
    let command_pool = unsafe { device.create_command_pool(&pool_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_dynamic_length_command_pool",
            code,
        }
    })?;
    let allocation_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let command_buffer =
        unsafe { device.allocate_command_buffers(&allocation_info) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "allocate_dynamic_length_command_buffer",
                code,
            }
        })?[0];
    unsafe { device.begin_command_buffer(command_buffer, &vk::CommandBufferBeginInfo::default()) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "begin_dynamic_length_command_buffer",
            code,
        })?;
    unsafe {
        device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::COMPUTE, pipeline);
        device.cmd_bind_descriptor_sets(
            command_buffer,
            vk::PipelineBindPoint::COMPUTE,
            pipeline_layout,
            0,
            std::slice::from_ref(&descriptor_set),
            &[],
        );
        let dispatch_x = (element_count as u32).div_ceil(GLOBAL_DYNAMIC_WORKGROUP_SIZE);
        device.cmd_dispatch(command_buffer, dispatch_x, 1, 1);
    }
    unsafe { device.end_command_buffer(command_buffer) }.map_err(|code| RuntimeError::Vulkan {
        operation: "end_dynamic_length_command_buffer",
        code,
    })?;
    let fence =
        unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_dynamic_length_fence",
                code,
            }
        })?;
    let submit_info =
        vk::SubmitInfo::default().command_buffers(std::slice::from_ref(&command_buffer));
    unsafe { device.queue_submit(queue, std::slice::from_ref(&submit_info), fence) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "submit_dynamic_length_queue",
            code,
        },
    )?;
    unsafe { device.wait_for_fences(std::slice::from_ref(&fence), true, u64::MAX) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "wait_dynamic_length_fence",
            code,
        },
    )?;
    let mut output_values = vec![0_u32; GLOBAL_DYNAMIC_CAPACITY];
    unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                GLOBAL_DYNAMIC_OUTPUT_BASE_OFFSET,
                capacity_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_dynamic_length_readback",
                code,
            })?;
        for (index, value) in output_values.iter_mut().enumerate() {
            *value = std::ptr::read_unaligned(mapped.cast::<u32>().add(index));
        }
        device.unmap_memory(storage_memory);
    }
    let mut expected_capacity = expected_values.clone();
    expected_capacity.resize(GLOBAL_DYNAMIC_CAPACITY, 0);
    let differential_passed = compare_u32(&expected_capacity, &output_values).is_ok();
    let residency_passed =
        resource_table.release(access_token).is_ok() && resource_table.evict(resource_id).is_ok();
    let input_checksum = input_values.iter().map(|value| u64::from(*value)).sum();
    let output_checksum = output_values[..element_count]
        .iter()
        .map(|value| u64::from(*value))
        .sum();
    let expected_checksum = expected_values.iter().map(|value| u64::from(*value)).sum();
    let untouched_tail_elements = output_values[element_count..]
        .iter()
        .filter(|value| **value == 0)
        .count();
    unsafe {
        device.destroy_fence(fence, None);
        device.destroy_command_pool(command_pool, None);
        device.destroy_pipeline(pipeline, None);
        device.destroy_pipeline_layout(pipeline_layout, None);
        device.destroy_shader_module(shader_module, None);
        device.destroy_descriptor_pool(descriptor_pool, None);
        device.destroy_descriptor_set_layout(descriptor_set_layout, None);
        device.free_memory(storage_memory, None);
        device.destroy_buffer(storage_buffer, None);
    }
    let dispatch_x = (element_count as u32).div_ceil(GLOBAL_DYNAMIC_WORKGROUP_SIZE);
    let report = GlobalDynamicU32QueueSmokeReport {
        schema: operation.schema(),
        operation: operation.label(),
        artifact_entry_name: artifact.entry_name.clone(),
        artifact_word_count: artifact.words.len(),
        artifact_word_hash: stable_spirv_word_hash(&artifact.words),
        artifact_validated: true,
        physical_device_count,
        selected_device,
        queue_family_index,
        queue_execution: "passed",
        descriptor_setup: "passed",
        resource_binding_count: resources.len(),
        pipeline_execution: "passed",
        fence_execution: "passed",
        data_kernel_execution: if differential_passed {
            "passed"
        } else {
            "failed"
        },
        differential_execution: if differential_passed {
            "passed"
        } else {
            "failed"
        },
        residency_execution: if residency_passed { "passed" } else { "failed" },
        element_count,
        capacity: GLOBAL_DYNAMIC_CAPACITY,
        dispatch_x,
        runtime_length: element_count as u32,
        input_checksum,
        output_checksum,
        expected_checksum,
        first_output: output_values[0],
        last_output: output_values[element_count - 1],
        untouched_tail_elements,
    };
    Ok(GlobalDynamicU32Execution {
        report,
        output_values,
    })
}

fn run_on_instance(instance: &Instance) -> Result<QueueSmokeReport, RuntimeError> {
    let physical_devices =
        unsafe { instance.enumerate_physical_devices() }.map_err(|code| RuntimeError::Vulkan {
            operation: "enumerate_physical_devices",
            code,
        })?;
    let (physical_device, queue_family_index) = physical_devices
        .iter()
        .find_map(|physical_device| {
            let families =
                unsafe { instance.get_physical_device_queue_family_properties(*physical_device) };
            families
                .iter()
                .enumerate()
                .find(|(_, properties)| {
                    properties.queue_count > 0
                        && properties.queue_flags.contains(vk::QueueFlags::COMPUTE)
                })
                .map(|(index, _)| (*physical_device, index as u32))
        })
        .ok_or(RuntimeError::NoComputeQueue)?;
    let properties = unsafe { instance.get_physical_device_properties(physical_device) };
    let selected_device = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
        .to_string_lossy()
        .into_owned();

    let priorities = [1.0_f32];
    let queue_info = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family_index)
        .queue_priorities(&priorities);
    let device_info =
        vk::DeviceCreateInfo::default().queue_create_infos(std::slice::from_ref(&queue_info));
    let device =
        unsafe { instance.create_device(physical_device, &device_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_device",
                code,
            }
        })?;
    let result = run_on_device(
        instance,
        &device,
        physical_device,
        queue_family_index,
        physical_devices.len(),
        selected_device,
    );
    unsafe {
        let _ = device.device_wait_idle();
        device.destroy_device(None);
    }
    result
}

fn run_on_device(
    instance: &Instance,
    device: &ash::Device,
    physical_device: vk::PhysicalDevice,
    queue_family_index: u32,
    physical_device_count: usize,
    selected_device: String,
) -> Result<QueueSmokeReport, RuntimeError> {
    let queue = unsafe { device.get_device_queue(queue_family_index, 0) };
    let kernel = storage_dynamic_index_add_fixture_module();
    let resources = reflect_resources(&kernel, FunctionId::new(0)).map_err(|error| {
        RuntimeError::DescriptorContract(format!("JIR reflection failed: {error}"))
    })?;
    let descriptor_bindings = resources
        .iter()
        .map(|resource| {
            if resource.address_space != AddressSpace::Storage {
                return Err(RuntimeError::DescriptorContract(format!(
                    "resource `{}` is not storage address space",
                    resource.name
                )));
            }
            Ok(vk::DescriptorSetLayoutBinding::default()
                .binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if descriptor_bindings.is_empty() {
        return Err(RuntimeError::DescriptorContract(
            "kernel has no reflected storage resources".to_owned(),
        ));
    }
    let spirv = emit_storage_dynamic_index_add_from_jir(
        &kernel,
        FunctionId::new(0),
        SpirvOptions::new([1, 1, 1]).map_err(|error| RuntimeError::Codegen(error.to_string()))?,
    )
    .map_err(|error| RuntimeError::Codegen(error.to_string()))?;
    let shader_info = vk::ShaderModuleCreateInfo::default().code(&spirv);
    let shader_module =
        unsafe { device.create_shader_module(&shader_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_shader_module",
                code,
            }
        })?;
    let entry_name = CString::new(kernel.functions[0].name.as_str())
        .expect("validated JIR entry name has no NUL");
    let stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(shader_module)
        .name(&entry_name);
    let storage_buffer_info = vk::BufferCreateInfo::default()
        .size(STORAGE_KERNEL_BUFFER_SIZE)
        .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let storage_buffer =
        unsafe { device.create_buffer(&storage_buffer_info, None) }.map_err(|code| {
            unsafe { device.destroy_shader_module(shader_module, None) };
            RuntimeError::Vulkan {
                operation: "create_storage_buffer",
                code,
            }
        })?;
    let requirements = unsafe { device.get_buffer_memory_requirements(storage_buffer) };
    let memory_properties =
        unsafe { instance.get_physical_device_memory_properties(physical_device) };
    let memory_type_index = (0..memory_properties.memory_type_count)
        .find(|index| {
            let compatible = requirements.memory_type_bits & (1_u32 << index) != 0;
            compatible
                && memory_properties.memory_types[*index as usize]
                    .property_flags
                    .contains(
                        vk::MemoryPropertyFlags::HOST_VISIBLE
                            | vk::MemoryPropertyFlags::HOST_COHERENT,
                    )
        })
        .ok_or_else(|| {
            RuntimeError::Loader(
                "no host-visible coherent Vulkan memory type for storage smoke".to_owned(),
            )
        })?;
    let allocation_info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_type_index);
    let storage_memory =
        unsafe { device.allocate_memory(&allocation_info, None) }.map_err(|code| {
            unsafe { device.destroy_buffer(storage_buffer, None) };
            RuntimeError::Vulkan {
                operation: "allocate_storage_memory",
                code,
            }
        })?;
    unsafe { device.bind_buffer_memory(storage_buffer, storage_memory, 0) }.map_err(|code| {
        unsafe {
            device.free_memory(storage_memory, None);
            device.destroy_buffer(storage_buffer, None);
        }
        RuntimeError::Vulkan {
            operation: "bind_storage_memory",
            code,
        }
    })?;
    let (mut resource_table, resource_id, access_token) =
        begin_storage_scope(STORAGE_KERNEL_BUFFER_SIZE)?;
    let input_byte_offset =
        STORAGE_KERNEL_INPUT_BASE_OFFSET + u64::from(STORAGE_KERNEL_INPUT_INDEX) * 4;
    let output_byte_offset =
        STORAGE_KERNEL_OUTPUT_BASE_OFFSET + u64::from(STORAGE_KERNEL_OUTPUT_INDEX) * 4;
    unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                input_byte_offset,
                4,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_storage_memory_for_upload",
                code,
            })?;
        std::ptr::write_unaligned(mapped.cast::<u32>(), STORAGE_KERNEL_INPUT);
        device.unmap_memory(storage_memory);
        let mapped = device
            .map_memory(
                storage_memory,
                output_byte_offset,
                4,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_storage_memory_for_output_clear",
                code,
            })?;
        std::ptr::write_unaligned(mapped.cast::<u32>(), 0);
        device.unmap_memory(storage_memory);
        let mapped = device
            .map_memory(
                storage_memory,
                STORAGE_KERNEL_INDEX_BASE_OFFSET,
                4,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_storage_memory_for_index_upload",
                code,
            })?;
        std::ptr::write_unaligned(mapped.cast::<u32>(), STORAGE_KERNEL_INDEX_VALUE);
        device.unmap_memory(storage_memory);
    }
    let descriptor_layout_info =
        vk::DescriptorSetLayoutCreateInfo::default().bindings(&descriptor_bindings);
    let descriptor_set_layout =
        unsafe { device.create_descriptor_set_layout(&descriptor_layout_info, None) }.map_err(
            |code| {
                unsafe {
                    device.free_memory(storage_memory, None);
                    device.destroy_buffer(storage_buffer, None);
                }
                RuntimeError::Vulkan {
                    operation: "create_descriptor_set_layout",
                    code,
                }
            },
        )?;
    let descriptor_pool_size = vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(u32::try_from(resources.len()).expect("resource count is bounded"));
    let descriptor_pool_info = vk::DescriptorPoolCreateInfo::default()
        .max_sets(1)
        .pool_sizes(std::slice::from_ref(&descriptor_pool_size));
    let descriptor_pool = unsafe { device.create_descriptor_pool(&descriptor_pool_info, None) }
        .map_err(|code| {
            unsafe {
                device.destroy_descriptor_set_layout(descriptor_set_layout, None);
                device.free_memory(storage_memory, None);
                device.destroy_buffer(storage_buffer, None);
            }
            RuntimeError::Vulkan {
                operation: "create_descriptor_pool",
                code,
            }
        })?;
    let descriptor_set_allocate_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(descriptor_pool)
        .set_layouts(std::slice::from_ref(&descriptor_set_layout));
    let descriptor_set = unsafe { device.allocate_descriptor_sets(&descriptor_set_allocate_info) }
        .map_err(|code| {
            unsafe {
                device.destroy_descriptor_pool(descriptor_pool, None);
                device.destroy_descriptor_set_layout(descriptor_set_layout, None);
                device.free_memory(storage_memory, None);
                device.destroy_buffer(storage_buffer, None);
            }
            RuntimeError::Vulkan {
                operation: "allocate_descriptor_set",
                code,
            }
        })?[0];
    let descriptor_buffer_infos = [
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(STORAGE_KERNEL_INPUT_BASE_OFFSET)
            .range(8),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(STORAGE_KERNEL_OUTPUT_BASE_OFFSET)
            .range(8),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(STORAGE_KERNEL_INDEX_BASE_OFFSET)
            .range(4),
    ];
    let descriptor_writes = resources
        .iter()
        .enumerate()
        .map(|(index, resource)| {
            vk::WriteDescriptorSet::default()
                .dst_set(descriptor_set)
                .dst_binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(std::slice::from_ref(&descriptor_buffer_infos[index]))
        })
        .collect::<Vec<_>>();
    unsafe { device.update_descriptor_sets(&descriptor_writes, &[]) };
    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(std::slice::from_ref(&descriptor_set_layout));
    let pipeline_layout =
        unsafe { device.create_pipeline_layout(&layout_info, None) }.map_err(|code| {
            unsafe { device.destroy_shader_module(shader_module, None) };
            RuntimeError::Vulkan {
                operation: "create_pipeline_layout",
                code,
            }
        })?;
    let pipeline_info = vk::ComputePipelineCreateInfo::default()
        .stage(stage)
        .layout(pipeline_layout);
    let pipeline = match unsafe {
        device.create_compute_pipelines(
            vk::PipelineCache::null(),
            std::slice::from_ref(&pipeline_info),
            None,
        )
    } {
        Ok(pipelines) => pipelines[0],
        Err((_, code)) => {
            unsafe {
                device.destroy_pipeline_layout(pipeline_layout, None);
                device.destroy_shader_module(shader_module, None);
            }
            return Err(RuntimeError::Vulkan {
                operation: "create_compute_pipeline",
                code,
            });
        }
    };
    let pool_info = vk::CommandPoolCreateInfo::default()
        .queue_family_index(queue_family_index)
        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
    let command_pool = unsafe { device.create_command_pool(&pool_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_command_pool",
            code,
        }
    })?;

    let allocation_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let command_buffer =
        unsafe { device.allocate_command_buffers(&allocation_info) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "allocate_command_buffers",
                code,
            }
        })?[0];
    let begin_info = vk::CommandBufferBeginInfo::default();
    unsafe { device.begin_command_buffer(command_buffer, &begin_info) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "begin_command_buffer",
            code,
        }
    })?;
    unsafe {
        device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::COMPUTE, pipeline);
        device.cmd_bind_descriptor_sets(
            command_buffer,
            vk::PipelineBindPoint::COMPUTE,
            pipeline_layout,
            0,
            std::slice::from_ref(&descriptor_set),
            &[],
        );
        device.cmd_dispatch(command_buffer, 1, 1, 1);
    }
    unsafe { device.end_command_buffer(command_buffer) }.map_err(|code| RuntimeError::Vulkan {
        operation: "end_command_buffer",
        code,
    })?;

    let fence =
        unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_fence",
                code,
            }
        })?;
    let submit_info =
        vk::SubmitInfo::default().command_buffers(std::slice::from_ref(&command_buffer));
    unsafe { device.queue_submit(queue, std::slice::from_ref(&submit_info), fence) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "queue_submit",
            code,
        },
    )?;
    unsafe { device.wait_for_fences(std::slice::from_ref(&fence), true, u64::MAX) }.map_err(
        |code| RuntimeError::Vulkan {
            operation: "wait_for_fences",
            code,
        },
    )?;

    let data_kernel_value = unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                output_byte_offset,
                4,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_storage_memory_for_readback",
                code,
            })?;
        let value = std::ptr::read_unaligned(mapped.cast::<u32>());
        device.unmap_memory(storage_memory);
        value
    };
    let cpu_reference_value = STORAGE_KERNEL_INPUT + STORAGE_KERNEL_ADDEND;
    let differential_passed = compare_u32(&[cpu_reference_value], &[data_kernel_value]).is_ok();
    let residency_passed =
        resource_table.release(access_token).is_ok() && resource_table.evict(resource_id).is_ok();

    unsafe {
        device.destroy_fence(fence, None);
        device.destroy_command_pool(command_pool, None);
        device.destroy_pipeline(pipeline, None);
        device.destroy_pipeline_layout(pipeline_layout, None);
        device.destroy_shader_module(shader_module, None);
        device.destroy_descriptor_pool(descriptor_pool, None);
        device.destroy_descriptor_set_layout(descriptor_set_layout, None);
        device.free_memory(storage_memory, None);
        device.destroy_buffer(storage_buffer, None);
    }
    Ok(QueueSmokeReport {
        schema: "jadren-vulkan-queue-smoke-0.1",
        requested_api_version: format!(
            "{}.{}.{}",
            vk::api_version_major(vk::API_VERSION_1_2),
            vk::api_version_minor(vk::API_VERSION_1_2),
            vk::api_version_patch(vk::API_VERSION_1_2)
        ),
        physical_device_count,
        selected_device,
        queue_family_index,
        queue_execution: "passed",
        fence_execution: "passed",
        descriptor_setup: "passed",
        resource_binding_count: resources.len(),
        compute_execution: "passed",
        pipeline_execution: "passed",
        data_kernel_execution: if differential_passed {
            "passed"
        } else {
            "failed"
        },
        differential_execution: if differential_passed {
            "passed"
        } else {
            "failed"
        },
        residency_execution: if residency_passed { "passed" } else { "failed" },
        data_kernel_value,
        cpu_reference_value,
        data_kernel_input: STORAGE_KERNEL_INPUT,
        data_kernel_addend: STORAGE_KERNEL_ADDEND,
        data_kernel_index: STORAGE_KERNEL_INDEX_VALUE,
    })
}

fn storage_global_index_strided_write_fixture_module() -> Module {
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
            name: "global_strided_write_u32".to_owned(),
            linkage: Linkage::Export,
            parameters: vec![
                Parameter {
                    value: ValueId::new(0),
                    ty: TypeId::new(2),
                    name: Some("buffer".to_owned()),
                },
                Parameter {
                    value: ValueId::new(1),
                    ty: TypeId::new(2),
                    name: Some("length".to_owned()),
                },
                Parameter {
                    value: ValueId::new(2),
                    ty: TypeId::new(2),
                    name: Some("stride".to_owned()),
                },
                Parameter {
                    value: ValueId::new(3),
                    ty: TypeId::new(2),
                    name: Some("capacity".to_owned()),
                },
            ],
            result: TypeId::new(0),
            blocks: vec![Block {
                id: BlockId::new(0),
                parameters: Vec::new(),
                instructions: vec![
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(4),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(5),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Constant(Constant::Integer {
                            value: i128::from(STRIDED_WRITE_VALUE),
                        }),
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(6),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Load {
                            pointer: ValueId::new(1),
                            alignment: 4,
                            volatile: false,
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(7),
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
                        result: Some(TypedValue {
                            value: ValueId::new(8),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Load {
                            pointer: ValueId::new(3),
                            alignment: 4,
                            volatile: false,
                        },
                        span: None,
                    },
                    Instruction {
                        result: None,
                        kind: InstructionKind::BoundsCheck {
                            index: ValueId::new(4),
                            length: ValueId::new(6),
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(9),
                            ty: TypeId::new(1),
                        }),
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
                        result: Some(TypedValue {
                            value: ValueId::new(10),
                            ty: TypeId::new(2),
                        }),
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
                ],
                terminator: Terminator::Return { value: None },
                span: None,
            }],
            span: None,
        }],
    }
}

#[cfg(test)]
fn storage_global_index_write_fixture_module() -> Module {
    storage_global_index_write_fixture_module_with_config(
        GLOBAL_WRITE_VALUE,
        GLOBAL_WRITE_ELEMENT_COUNT,
    )
    .expect("baseline global-write fixture is valid")
}

fn storage_global_index_write_fixture_module_with_config(
    value: u32,
    length: usize,
) -> Result<Module, RuntimeError> {
    let length = u32::try_from(length).map_err(|_| {
        RuntimeError::DescriptorContract("global-write length exceeds u32".to_owned())
    })?;
    if length == 0 {
        return Err(RuntimeError::DescriptorContract(
            "global-write length must be positive".to_owned(),
        ));
    }
    Ok(Module {
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
            name: "global_write_u32".to_owned(),
            linkage: Linkage::Export,
            parameters: vec![Parameter {
                value: ValueId::new(0),
                ty: TypeId::new(2),
                name: Some("buffer".to_owned()),
            }],
            result: TypeId::new(0),
            blocks: vec![Block {
                id: BlockId::new(0),
                parameters: Vec::new(),
                instructions: vec![
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(1),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(2),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Constant(Constant::Integer {
                            value: i128::from(value),
                        }),
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(3),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Constant(Constant::Integer {
                            value: i128::from(length),
                        }),
                        span: None,
                    },
                    Instruction {
                        result: None,
                        kind: InstructionKind::BoundsCheck {
                            index: ValueId::new(1),
                            length: ValueId::new(3),
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(4),
                            ty: TypeId::new(2),
                        }),
                        kind: InstructionKind::Offset {
                            base: ValueId::new(0),
                            indices: vec![ValueId::new(1)],
                        },
                        span: None,
                    },
                    Instruction {
                        result: None,
                        kind: InstructionKind::Store {
                            pointer: ValueId::new(4),
                            value: ValueId::new(2),
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
    })
}

fn storage_global_2d_write_fixture_module() -> Module {
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
            name: "global_2d_write_u32".to_owned(),
            linkage: Linkage::Export,
            parameters: vec![
                Parameter {
                    value: ValueId::new(0),
                    ty: TypeId::new(2),
                    name: Some("buffer".to_owned()),
                },
                Parameter {
                    value: ValueId::new(1),
                    ty: TypeId::new(2),
                    name: Some("width".to_owned()),
                },
                Parameter {
                    value: ValueId::new(2),
                    ty: TypeId::new(2),
                    name: Some("height".to_owned()),
                },
                Parameter {
                    value: ValueId::new(3),
                    ty: TypeId::new(2),
                    name: Some("capacity".to_owned()),
                },
            ],
            result: TypeId::new(0),
            blocks: vec![Block {
                id: BlockId::new(0),
                parameters: Vec::new(),
                instructions: vec![
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(4),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(5),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdY),
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(6),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Constant(Constant::Integer {
                            value: i128::from(TWO_D_WRITE_VALUE),
                        }),
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(7),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Load {
                            pointer: ValueId::new(1),
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
                        kind: InstructionKind::Load {
                            pointer: ValueId::new(2),
                            alignment: 4,
                            volatile: false,
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(9),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Load {
                            pointer: ValueId::new(3),
                            alignment: 4,
                            volatile: false,
                        },
                        span: None,
                    },
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
                        result: Some(TypedValue {
                            value: ValueId::new(10),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Binary {
                            op: BinaryOp::Multiply,
                            left: ValueId::new(5),
                            right: ValueId::new(7),
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(11),
                            ty: TypeId::new(1),
                        }),
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
                        result: Some(TypedValue {
                            value: ValueId::new(12),
                            ty: TypeId::new(2),
                        }),
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
                ],
                terminator: Terminator::Return { value: None },
                span: None,
            }],
            span: None,
        }],
    }
}

fn storage_global_2d_strided_write_fixture_module() -> Module {
    let instruction = |result, kind| Instruction {
        result,
        kind,
        span: None,
    };
    let pointer = Type::Pointer {
        pointee: TypeId::new(1),
        address_space: AddressSpace::Storage,
    };
    Module {
        types: vec![
            Type::Unit,
            Type::Integer {
                signed: false,
                bits: 32,
            },
            pointer,
        ],
        functions: vec![Function {
            id: FunctionId::new(0),
            name: "global_2d_strided_write_u32".to_owned(),
            linkage: Linkage::Export,
            parameters: (0..6)
                .map(|index| Parameter {
                    value: ValueId::new(index),
                    ty: TypeId::new(2),
                    name: Some(
                        [
                            "buffer", "width", "height", "stride_x", "stride_y", "capacity",
                        ][index]
                            .to_owned(),
                    ),
                })
                .collect(),
            result: TypeId::new(0),
            blocks: vec![Block {
                id: BlockId::new(0),
                parameters: Vec::new(),
                instructions: vec![
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(6),
                            ty: TypeId::new(1),
                        }),
                        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(7),
                            ty: TypeId::new(1),
                        }),
                        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdY),
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(8),
                            ty: TypeId::new(1),
                        }),
                        InstructionKind::Constant(Constant::Integer {
                            value: i128::from(TWO_D_STRIDED_WRITE_VALUE),
                        }),
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(9),
                            ty: TypeId::new(1),
                        }),
                        InstructionKind::Load {
                            pointer: ValueId::new(1),
                            alignment: 4,
                            volatile: false,
                        },
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(10),
                            ty: TypeId::new(1),
                        }),
                        InstructionKind::Load {
                            pointer: ValueId::new(2),
                            alignment: 4,
                            volatile: false,
                        },
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(11),
                            ty: TypeId::new(1),
                        }),
                        InstructionKind::Load {
                            pointer: ValueId::new(3),
                            alignment: 4,
                            volatile: false,
                        },
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(12),
                            ty: TypeId::new(1),
                        }),
                        InstructionKind::Load {
                            pointer: ValueId::new(4),
                            alignment: 4,
                            volatile: false,
                        },
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(13),
                            ty: TypeId::new(1),
                        }),
                        InstructionKind::Load {
                            pointer: ValueId::new(5),
                            alignment: 4,
                            volatile: false,
                        },
                    ),
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
                        Some(TypedValue {
                            value: ValueId::new(14),
                            ty: TypeId::new(1),
                        }),
                        InstructionKind::Binary {
                            op: BinaryOp::Multiply,
                            left: ValueId::new(6),
                            right: ValueId::new(11),
                        },
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(15),
                            ty: TypeId::new(1),
                        }),
                        InstructionKind::Binary {
                            op: BinaryOp::Multiply,
                            left: ValueId::new(7),
                            right: ValueId::new(12),
                        },
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(16),
                            ty: TypeId::new(1),
                        }),
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
                        Some(TypedValue {
                            value: ValueId::new(17),
                            ty: TypeId::new(2),
                        }),
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
    }
}

fn storage_global_3d_write_fixture_module() -> Module {
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
            name: "global_3d_write_u32".to_owned(),
            linkage: Linkage::Export,
            parameters: vec![
                Parameter {
                    value: ValueId::new(0),
                    ty: TypeId::new(2),
                    name: Some("buffer".to_owned()),
                },
                Parameter {
                    value: ValueId::new(1),
                    ty: TypeId::new(2),
                    name: Some("width".to_owned()),
                },
                Parameter {
                    value: ValueId::new(2),
                    ty: TypeId::new(2),
                    name: Some("height".to_owned()),
                },
                Parameter {
                    value: ValueId::new(3),
                    ty: TypeId::new(2),
                    name: Some("depth".to_owned()),
                },
                Parameter {
                    value: ValueId::new(4),
                    ty: TypeId::new(2),
                    name: Some("capacity".to_owned()),
                },
            ],
            result: TypeId::new(0),
            blocks: vec![Block {
                id: BlockId::new(0),
                parameters: Vec::new(),
                instructions: vec![
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(5),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(6),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdY),
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(7),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdZ),
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(8),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Constant(Constant::Integer {
                            value: i128::from(THREE_D_WRITE_VALUE),
                        }),
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(9),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Load {
                            pointer: ValueId::new(1),
                            alignment: 4,
                            volatile: false,
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(10),
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
                        result: Some(TypedValue {
                            value: ValueId::new(11),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Load {
                            pointer: ValueId::new(3),
                            alignment: 4,
                            volatile: false,
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(12),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Load {
                            pointer: ValueId::new(4),
                            alignment: 4,
                            volatile: false,
                        },
                        span: None,
                    },
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
                        result: Some(TypedValue {
                            value: ValueId::new(13),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Binary {
                            op: BinaryOp::Multiply,
                            left: ValueId::new(7),
                            right: ValueId::new(10),
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(14),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Binary {
                            op: BinaryOp::Add,
                            left: ValueId::new(13),
                            right: ValueId::new(6),
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(15),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Binary {
                            op: BinaryOp::Multiply,
                            left: ValueId::new(14),
                            right: ValueId::new(9),
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(16),
                            ty: TypeId::new(1),
                        }),
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
                        result: Some(TypedValue {
                            value: ValueId::new(17),
                            ty: TypeId::new(2),
                        }),
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
                ],
                terminator: Terminator::Return { value: None },
                span: None,
            }],
            span: None,
        }],
    }
}

#[cfg(test)]
fn storage_global_3d_strided_write_fixture_module() -> Module {
    storage_global_3d_strided_write_fixture_module_with_value(THREE_D_STRIDED_WRITE_VALUE)
}

fn storage_global_3d_strided_write_fixture_module_with_value(value: u32) -> Module {
    let pointer = Type::Pointer {
        pointee: TypeId::new(1),
        address_space: AddressSpace::Storage,
    };
    let typed = |value| {
        Some(TypedValue {
            value: ValueId::new(value),
            ty: TypeId::new(1),
        })
    };
    let load = |value, pointer| Instruction {
        result: typed(value),
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
    let binary = |value, op, left, right| Instruction {
        result: typed(value),
        kind: InstructionKind::Binary {
            op,
            left: ValueId::new(left),
            right: ValueId::new(right),
        },
        span: None,
    };
    Module {
        types: vec![
            Type::Unit,
            Type::Integer {
                signed: false,
                bits: 32,
            },
            pointer,
        ],
        functions: vec![Function {
            id: FunctionId::new(0),
            name: "global_3d_strided_write_u32".to_owned(),
            linkage: Linkage::Export,
            parameters: [
                "buffer", "width", "height", "depth", "stride_x", "stride_y", "stride_z",
                "capacity",
            ]
            .into_iter()
            .enumerate()
            .map(|(index, name)| Parameter {
                value: ValueId::new(index),
                ty: TypeId::new(2),
                name: Some(name.to_owned()),
            })
            .collect(),
            result: TypeId::new(0),
            blocks: vec![Block {
                id: BlockId::new(0),
                parameters: Vec::new(),
                instructions: vec![
                    Instruction {
                        result: typed(8),
                        kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                        span: None,
                    },
                    Instruction {
                        result: typed(9),
                        kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdY),
                        span: None,
                    },
                    Instruction {
                        result: typed(10),
                        kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdZ),
                        span: None,
                    },
                    Instruction {
                        result: typed(11),
                        kind: InstructionKind::Constant(Constant::Integer {
                            value: i128::from(value),
                        }),
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
                        result: Some(TypedValue {
                            value: ValueId::new(24),
                            ty: TypeId::new(2),
                        }),
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
    }
}

#[cfg(test)]
fn storage_global_dynamic_index_arithmetic_fixture_module(
    operation: GlobalDynamicOperation,
) -> Module {
    storage_global_dynamic_index_arithmetic_fixture_module_with_operand(
        operation,
        operation.operand(),
    )
}

fn storage_global_dynamic_index_arithmetic_fixture_module_with_operand(
    operation: GlobalDynamicOperation,
    operand: u32,
) -> Module {
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
            name: operation.entry_name().to_owned(),
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
                        kind: InstructionKind::Constant(Constant::Integer {
                            value: i128::from(operand),
                        }),
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
                            op: operation.jir_op(),
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

fn storage_global_dynamic_index_f32_binary_fixture_module(
    operand_bits: u32,
    operation: F32ArithmeticOp,
) -> Module {
    Module {
        types: vec![
            Type::Unit,
            Type::Integer {
                signed: false,
                bits: 32,
            },
            Type::Float { bits: 32 },
            Type::Pointer {
                pointee: TypeId::new(2),
                address_space: AddressSpace::Storage,
            },
            Type::Pointer {
                pointee: TypeId::new(1),
                address_space: AddressSpace::Storage,
            },
        ],
        functions: vec![Function {
            id: FunctionId::new(0),
            name: format!("global_{}_dynamic_f32", f32_operation_name(operation)),
            linkage: Linkage::Export,
            parameters: vec![
                Parameter {
                    value: ValueId::new(0),
                    ty: TypeId::new(3),
                    name: Some("input".to_owned()),
                },
                Parameter {
                    value: ValueId::new(1),
                    ty: TypeId::new(3),
                    name: Some("output".to_owned()),
                },
                Parameter {
                    value: ValueId::new(2),
                    ty: TypeId::new(4),
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
                            ty: TypeId::new(2),
                        }),
                        kind: InstructionKind::Constant(Constant::FloatBits {
                            bits: u64::from(operand_bits),
                        }),
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
                            ty: TypeId::new(3),
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
                            ty: TypeId::new(2),
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
                            ty: TypeId::new(2),
                        }),
                        kind: InstructionKind::Binary {
                            op: operation.as_binary_op(),
                            left: ValueId::new(7),
                            right: ValueId::new(4),
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(9),
                            ty: TypeId::new(3),
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

fn storage_global_dynamic_index_vector_f32_binary_fixture_module(
    operand: f32,
    operation: F32ArithmeticOp,
) -> Module {
    storage_global_dynamic_index_vector_f32_binary_fixture_module_lanes(operand, operation, 4)
}

fn storage_global_dynamic_index_vector_f32_binary_fixture_module_lanes(
    operand: f32,
    operation: F32ArithmeticOp,
    lanes: u16,
) -> Module {
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
                lanes,
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
            name: format!(
                "global_{}_dynamic_f32x{lanes}",
                f32_operation_name(operation)
            ),
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
                            ty: TypeId::new(2),
                        }),
                        kind: InstructionKind::Constant(Constant::FloatBits {
                            bits: u64::from(operand.to_bits()),
                        }),
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(5),
                            ty: TypeId::new(3),
                        }),
                        kind: InstructionKind::VectorSplat {
                            value: ValueId::new(4),
                            lanes,
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(6),
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
                            length: ValueId::new(6),
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(7),
                            ty: TypeId::new(4),
                        }),
                        kind: InstructionKind::Offset {
                            base: ValueId::new(0),
                            indices: vec![ValueId::new(3)],
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(8),
                            ty: TypeId::new(3),
                        }),
                        kind: InstructionKind::Load {
                            pointer: ValueId::new(7),
                            alignment: if lanes == 4 { 16 } else { 4 },
                            volatile: false,
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(9),
                            ty: TypeId::new(3),
                        }),
                        kind: InstructionKind::VectorBinary {
                            op: f32_binary_op(operation),
                            left: ValueId::new(8),
                            right: ValueId::new(5),
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(10),
                            ty: TypeId::new(4),
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
                            pointer: ValueId::new(10),
                            value: ValueId::new(9),
                            alignment: if lanes == 4 { 16 } else { 4 },
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

#[cfg(test)]
fn storage_global_dynamic_index_fadd_fixture_module(operand_bits: u32) -> Module {
    storage_global_dynamic_index_f32_binary_fixture_module(operand_bits, F32ArithmeticOp::Add)
}

fn storage_global_index_add_fixture_module() -> Module {
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
            name: "global_add_u32".to_owned(),
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
            ],
            result: TypeId::new(0),
            blocks: vec![Block {
                id: BlockId::new(0),
                parameters: Vec::new(),
                instructions: vec![
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(2),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(3),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Constant(Constant::Integer {
                            value: i128::from(GLOBAL_KERNEL_ADDEND),
                        }),
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(4),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Constant(Constant::Integer {
                            value: GLOBAL_KERNEL_ELEMENT_COUNT as i128,
                        }),
                        span: None,
                    },
                    Instruction {
                        result: None,
                        kind: InstructionKind::BoundsCheck {
                            index: ValueId::new(2),
                            length: ValueId::new(4),
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(5),
                            ty: TypeId::new(2),
                        }),
                        kind: InstructionKind::Offset {
                            base: ValueId::new(0),
                            indices: vec![ValueId::new(2)],
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(6),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Load {
                            pointer: ValueId::new(5),
                            alignment: 4,
                            volatile: false,
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(7),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Binary {
                            op: jadren_jir::BinaryOp::Add,
                            left: ValueId::new(6),
                            right: ValueId::new(3),
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(8),
                            ty: TypeId::new(2),
                        }),
                        kind: InstructionKind::Offset {
                            base: ValueId::new(1),
                            indices: vec![ValueId::new(2)],
                        },
                        span: None,
                    },
                    Instruction {
                        result: None,
                        kind: InstructionKind::Store {
                            pointer: ValueId::new(8),
                            value: ValueId::new(7),
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

fn storage_add_artifact_fixture_module() -> Module {
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
            parameters: vec![Parameter {
                value: ValueId::new(0),
                ty: TypeId::new(2),
                name: Some("data".to_owned()),
            }],
            result: TypeId::new(0),
            blocks: vec![Block {
                id: BlockId::new(0),
                parameters: Vec::new(),
                instructions: vec![
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(1),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Constant(Constant::Integer {
                            value: i128::from(STORAGE_ADD_ARTIFACT_ADDEND),
                        }),
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(2),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Load {
                            pointer: ValueId::new(0),
                            alignment: 4,
                            volatile: false,
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(3),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Binary {
                            op: BinaryOp::Add,
                            left: ValueId::new(2),
                            right: ValueId::new(1),
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
                ],
                terminator: Terminator::Return { value: None },
                span: None,
            }],
            span: None,
        }],
    }
}

fn storage_dynamic_index_fadd_fixture_module(addend_bits: u32) -> Module {
    Module {
        types: vec![
            Type::Unit,
            Type::Integer {
                signed: false,
                bits: 32,
            },
            Type::Float { bits: 32 },
            Type::Pointer {
                pointee: TypeId::new(2),
                address_space: AddressSpace::Storage,
            },
            Type::Pointer {
                pointee: TypeId::new(1),
                address_space: AddressSpace::Storage,
            },
        ],
        functions: vec![Function {
            id: FunctionId::new(0),
            name: "dynamic_fadd_f32".to_owned(),
            linkage: Linkage::Export,
            parameters: vec![
                Parameter {
                    value: ValueId::new(0),
                    ty: TypeId::new(3),
                    name: Some("input".to_owned()),
                },
                Parameter {
                    value: ValueId::new(1),
                    ty: TypeId::new(3),
                    name: Some("output".to_owned()),
                },
                Parameter {
                    value: ValueId::new(2),
                    ty: TypeId::new(4),
                    name: Some("index".to_owned()),
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
                            ty: TypeId::new(2),
                        }),
                        kind: InstructionKind::Constant(Constant::FloatBits {
                            bits: addend_bits as u64,
                        }),
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(4),
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
                        result: Some(TypedValue {
                            value: ValueId::new(5),
                            ty: TypeId::new(3),
                        }),
                        kind: InstructionKind::Offset {
                            base: ValueId::new(0),
                            indices: vec![ValueId::new(4)],
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(6),
                            ty: TypeId::new(2),
                        }),
                        kind: InstructionKind::Load {
                            pointer: ValueId::new(5),
                            alignment: 4,
                            volatile: false,
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(7),
                            ty: TypeId::new(2),
                        }),
                        kind: InstructionKind::Binary {
                            op: jadren_jir::BinaryOp::Add,
                            left: ValueId::new(6),
                            right: ValueId::new(3),
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(8),
                            ty: TypeId::new(3),
                        }),
                        kind: InstructionKind::Offset {
                            base: ValueId::new(1),
                            indices: vec![ValueId::new(4)],
                        },
                        span: None,
                    },
                    Instruction {
                        result: None,
                        kind: InstructionKind::Store {
                            pointer: ValueId::new(8),
                            value: ValueId::new(7),
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

fn storage_dynamic_index_add_fixture_module() -> Module {
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
            name: "dynamic_add_u32".to_owned(),
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
                    name: Some("index".to_owned()),
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
                        kind: InstructionKind::Constant(Constant::Integer {
                            value: i128::from(STORAGE_KERNEL_ADDEND),
                        }),
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(4),
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
                        result: Some(TypedValue {
                            value: ValueId::new(5),
                            ty: TypeId::new(2),
                        }),
                        kind: InstructionKind::Offset {
                            base: ValueId::new(0),
                            indices: vec![ValueId::new(4)],
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(6),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Load {
                            pointer: ValueId::new(5),
                            alignment: 4,
                            volatile: false,
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(7),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Binary {
                            op: jadren_jir::BinaryOp::Add,
                            left: ValueId::new(6),
                            right: ValueId::new(3),
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(8),
                            ty: TypeId::new(2),
                        }),
                        kind: InstructionKind::Offset {
                            base: ValueId::new(1),
                            indices: vec![ValueId::new(4)],
                        },
                        span: None,
                    },
                    Instruction {
                        result: None,
                        kind: InstructionKind::Store {
                            pointer: ValueId::new(8),
                            value: ValueId::new(7),
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

fn run_global_3d_strided_write_on_device(
    instance: &Instance,
    device: &ash::Device,
    physical_device: vk::PhysicalDevice,
    queue_family_index: u32,
    physical_device_count: usize,
    selected_device: String,
    config: Global3dStridedWriteConfig,
) -> Result<Global3dStridedWriteExecution, RuntimeError> {
    let config = config.validate()?;
    let width = config.width;
    let height = config.height;
    let depth = config.depth;
    let stride_x = config.stride_x;
    let stride_y = config.stride_y;
    let stride_z = config.stride_z;
    let capacity = config.capacity;
    let workgroup_size = config.workgroup_size;
    let value = config.value;
    let buffer_base_offset = 0_u64;
    let [
        width_offset,
        height_offset,
        depth_offset,
        stride_x_offset,
        stride_y_offset,
        stride_z_offset,
        capacity_offset,
    ] = config.metadata_offsets();
    let buffer_size = config.buffer_size()?;
    let buffer_bytes = (capacity as u64)
        .checked_mul(std::mem::size_of::<u32>() as u64)
        .ok_or_else(|| {
            RuntimeError::DescriptorContract(
                "3D affine-stride output byte size overflow".to_owned(),
            )
        })?;
    let queue = unsafe { device.get_device_queue(queue_family_index, 0) };
    let kernel = storage_global_3d_strided_write_fixture_module_with_value(value);
    let artifact = emit_storage_global_3d_strided_write_artifact_from_jir(
        &kernel,
        FunctionId::new(0),
        SpirvOptions::new(workgroup_size)
            .map_err(|error| RuntimeError::Codegen(error.to_string()))?,
    )
    .map_err(|error| RuntimeError::Codegen(error.to_string()))?;
    let resources = &artifact.resources;
    if resources.len() != 8
        || resources
            .iter()
            .any(|resource| resource.address_space != AddressSpace::Storage)
        || resources.iter().any(|resource| {
            !matches!(
                kernel.types.get(resource.element_type.index()),
                Some(Type::Integer {
                    signed: false,
                    bits: 32
                })
            )
        })
    {
        return Err(RuntimeError::DescriptorContract(
            "global-3d-strided-write kernel must expose eight storage u32 resources".to_owned(),
        ));
    }
    let descriptor_bindings = resources
        .iter()
        .map(|resource| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE)
        })
        .collect::<Vec<_>>();
    let shader_info = vk::ShaderModuleCreateInfo::default().code(&artifact.words);
    let shader_module =
        unsafe { device.create_shader_module(&shader_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_3d_strided_write_shader_module",
                code,
            }
        })?;
    let entry_name =
        CString::new(artifact.entry_name.as_str()).expect("validated JIR entry name has no NUL");
    let stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(shader_module)
        .name(&entry_name);
    let storage_buffer_info = vk::BufferCreateInfo::default()
        .size(buffer_size)
        .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let storage_buffer =
        unsafe { device.create_buffer(&storage_buffer_info, None) }.map_err(|code| {
            unsafe { device.destroy_shader_module(shader_module, None) };
            RuntimeError::Vulkan {
                operation: "create_global_3d_strided_write_storage_buffer",
                code,
            }
        })?;
    let requirements = unsafe { device.get_buffer_memory_requirements(storage_buffer) };
    let memory_properties =
        unsafe { instance.get_physical_device_memory_properties(physical_device) };
    let memory_type_index = (0..memory_properties.memory_type_count)
        .find(|index| {
            let compatible = requirements.memory_type_bits & (1_u32 << index) != 0;
            compatible
                && memory_properties.memory_types[*index as usize]
                    .property_flags
                    .contains(
                        vk::MemoryPropertyFlags::HOST_VISIBLE
                            | vk::MemoryPropertyFlags::HOST_COHERENT,
                    )
        })
        .ok_or_else(|| {
            RuntimeError::Loader(
                "no host-visible coherent Vulkan memory type for global-3d-strided-write smoke"
                    .to_owned(),
            )
        })?;
    let allocation_info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_type_index);
    let storage_memory =
        unsafe { device.allocate_memory(&allocation_info, None) }.map_err(|code| {
            unsafe { device.destroy_buffer(storage_buffer, None) };
            RuntimeError::Vulkan {
                operation: "allocate_global_3d_strided_write_storage_memory",
                code,
            }
        })?;
    unsafe { device.bind_buffer_memory(storage_buffer, storage_memory, 0) }.map_err(|code| {
        unsafe {
            device.free_memory(storage_memory, None);
            device.destroy_buffer(storage_buffer, None);
        }
        RuntimeError::Vulkan {
            operation: "bind_global_3d_strided_write_storage_memory",
            code,
        }
    })?;
    let (mut resource_table, resource_id, access_token) = begin_storage_scope(buffer_size)?;
    unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                buffer_base_offset,
                buffer_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_global_3d_strided_write_output_clear",
                code,
            })?;
        for index in 0..capacity {
            std::ptr::write_unaligned(mapped.cast::<u32>().add(index), 0);
        }
        device.unmap_memory(storage_memory);
        for (offset, value, operation) in [
            (
                width_offset,
                width as u32,
                "map_global_3d_strided_write_width",
            ),
            (
                height_offset,
                height as u32,
                "map_global_3d_strided_write_height",
            ),
            (
                depth_offset,
                depth as u32,
                "map_global_3d_strided_write_depth",
            ),
            (
                stride_x_offset,
                stride_x,
                "map_global_3d_strided_write_stride_x",
            ),
            (
                stride_y_offset,
                stride_y,
                "map_global_3d_strided_write_stride_y",
            ),
            (
                stride_z_offset,
                stride_z,
                "map_global_3d_strided_write_stride_z",
            ),
            (
                capacity_offset,
                capacity as u32,
                "map_global_3d_strided_write_capacity",
            ),
        ] {
            let mapped = device
                .map_memory(storage_memory, offset, 4, vk::MemoryMapFlags::empty())
                .map_err(|code| RuntimeError::Vulkan { operation, code })?;
            std::ptr::write_unaligned(mapped.cast::<u32>(), value);
            device.unmap_memory(storage_memory);
        }
    }
    let descriptor_layout_info =
        vk::DescriptorSetLayoutCreateInfo::default().bindings(&descriptor_bindings);
    let descriptor_set_layout =
        unsafe { device.create_descriptor_set_layout(&descriptor_layout_info, None) }.map_err(
            |code| RuntimeError::Vulkan {
                operation: "create_global_3d_strided_write_descriptor_set_layout",
                code,
            },
        )?;
    let descriptor_pool_size = vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(8);
    let descriptor_pool_info = vk::DescriptorPoolCreateInfo::default()
        .max_sets(1)
        .pool_sizes(std::slice::from_ref(&descriptor_pool_size));
    let descriptor_pool = unsafe { device.create_descriptor_pool(&descriptor_pool_info, None) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "create_global_3d_strided_write_descriptor_pool",
            code,
        })?;
    let descriptor_set_allocate_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(descriptor_pool)
        .set_layouts(std::slice::from_ref(&descriptor_set_layout));
    let descriptor_set = unsafe { device.allocate_descriptor_sets(&descriptor_set_allocate_info) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "allocate_global_3d_strided_write_descriptor_set",
            code,
        })?[0];
    let descriptor_buffer_infos = [
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(buffer_base_offset)
            .range(buffer_bytes),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(width_offset)
            .range(4),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(height_offset)
            .range(4),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(depth_offset)
            .range(4),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(stride_x_offset)
            .range(4),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(stride_y_offset)
            .range(4),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(stride_z_offset)
            .range(4),
        vk::DescriptorBufferInfo::default()
            .buffer(storage_buffer)
            .offset(capacity_offset)
            .range(4),
    ];
    let descriptor_writes = resources
        .iter()
        .enumerate()
        .map(|(index, resource)| {
            vk::WriteDescriptorSet::default()
                .dst_set(descriptor_set)
                .dst_binding(resource.binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(std::slice::from_ref(&descriptor_buffer_infos[index]))
        })
        .collect::<Vec<_>>();
    unsafe { device.update_descriptor_sets(&descriptor_writes, &[]) };
    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(std::slice::from_ref(&descriptor_set_layout));
    let pipeline_layout =
        unsafe { device.create_pipeline_layout(&layout_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_3d_strided_write_pipeline_layout",
                code,
            }
        })?;
    let pipeline_info = vk::ComputePipelineCreateInfo::default()
        .stage(stage)
        .layout(pipeline_layout);
    let pipeline = match unsafe {
        device.create_compute_pipelines(
            vk::PipelineCache::null(),
            std::slice::from_ref(&pipeline_info),
            None,
        )
    } {
        Ok(pipelines) => pipelines[0],
        Err((_, code)) => {
            return Err(RuntimeError::Vulkan {
                operation: "create_global_3d_strided_write_compute_pipeline",
                code,
            });
        }
    };
    let pool_info = vk::CommandPoolCreateInfo::default()
        .queue_family_index(queue_family_index)
        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
    let command_pool = unsafe { device.create_command_pool(&pool_info, None) }.map_err(|code| {
        RuntimeError::Vulkan {
            operation: "create_global_3d_strided_write_command_pool",
            code,
        }
    })?;
    let allocation_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let command_buffer =
        unsafe { device.allocate_command_buffers(&allocation_info) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "allocate_global_3d_strided_write_command_buffer",
                code,
            }
        })?[0];
    unsafe { device.begin_command_buffer(command_buffer, &vk::CommandBufferBeginInfo::default()) }
        .map_err(|code| RuntimeError::Vulkan {
            operation: "begin_global_3d_strided_write_command_buffer",
            code,
        })?;
    unsafe {
        device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::COMPUTE, pipeline);
        device.cmd_bind_descriptor_sets(
            command_buffer,
            vk::PipelineBindPoint::COMPUTE,
            pipeline_layout,
            0,
            std::slice::from_ref(&descriptor_set),
            &[],
        );
        let dispatch_x = (width as u32).div_ceil(workgroup_size[0]);
        let dispatch_y = (height as u32).div_ceil(workgroup_size[1]);
        let dispatch_z = (depth as u32).div_ceil(workgroup_size[2]);
        device.cmd_dispatch(command_buffer, dispatch_x, dispatch_y, dispatch_z);
    }
    unsafe { device.end_command_buffer(command_buffer) }.map_err(|code| RuntimeError::Vulkan {
        operation: "end_global_3d_strided_write_command_buffer",
        code,
    })?;
    let fence =
        unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_3d_strided_write_fence",
                code,
            }
        })?;
    let mut timeline_type = vk::SemaphoreTypeCreateInfo::default()
        .semaphore_type(vk::SemaphoreType::TIMELINE)
        .initial_value(0);
    let timeline_info = vk::SemaphoreCreateInfo::default().push_next(&mut timeline_type);
    let timeline_semaphore =
        unsafe { device.create_semaphore(&timeline_info, None) }.map_err(|code| {
            unsafe { device.destroy_fence(fence, None) };
            RuntimeError::Vulkan {
                operation: "create_global_3d_strided_write_timeline_semaphore",
                code,
            }
        })?;
    let expected_timeline_value = 1_u64;
    let signal_values = [expected_timeline_value];
    let mut timeline_submit =
        vk::TimelineSemaphoreSubmitInfo::default().signal_semaphore_values(&signal_values);
    let submit_info = vk::SubmitInfo::default()
        .push_next(&mut timeline_submit)
        .command_buffers(std::slice::from_ref(&command_buffer))
        .signal_semaphores(std::slice::from_ref(&timeline_semaphore));
    if let Err(code) =
        unsafe { device.queue_submit(queue, std::slice::from_ref(&submit_info), fence) }
    {
        unsafe {
            device.destroy_semaphore(timeline_semaphore, None);
            device.destroy_fence(fence, None);
        }
        return Err(RuntimeError::Vulkan {
            operation: "submit_global_3d_strided_write_queue",
            code,
        });
    }
    let wait_values = [expected_timeline_value];
    let wait_info = vk::SemaphoreWaitInfo::default()
        .semaphores(std::slice::from_ref(&timeline_semaphore))
        .values(&wait_values);
    if let Err(code) = unsafe { device.wait_semaphores(&wait_info, u64::MAX) } {
        unsafe {
            let _ = device.device_wait_idle();
            device.destroy_semaphore(timeline_semaphore, None);
            device.destroy_fence(fence, None);
        }
        return Err(RuntimeError::Vulkan {
            operation: "wait_global_3d_strided_write_timeline",
            code,
        });
    }
    let timeline_value = match unsafe { device.get_semaphore_counter_value(timeline_semaphore) } {
        Ok(value) => value,
        Err(code) => {
            unsafe {
                let _ = device.device_wait_idle();
                device.destroy_semaphore(timeline_semaphore, None);
                device.destroy_fence(fence, None);
            }
            return Err(RuntimeError::Vulkan {
                operation: "read_global_3d_strided_write_timeline",
                code,
            });
        }
    };
    let timeline_passed = timeline_value == expected_timeline_value;
    if let Err(code) =
        unsafe { device.wait_for_fences(std::slice::from_ref(&fence), true, u64::MAX) }
    {
        unsafe {
            let _ = device.device_wait_idle();
            device.destroy_semaphore(timeline_semaphore, None);
            device.destroy_fence(fence, None);
        }
        return Err(RuntimeError::Vulkan {
            operation: "wait_global_3d_strided_write_fence",
            code,
        });
    }
    unsafe {
        device.destroy_semaphore(timeline_semaphore, None);
    }
    let mut output_values = vec![0_u32; capacity];
    unsafe {
        let mapped = device
            .map_memory(
                storage_memory,
                buffer_base_offset,
                buffer_bytes,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|code| RuntimeError::Vulkan {
                operation: "map_global_3d_strided_write_readback",
                code,
            })?;
        for (index, value) in output_values.iter_mut().enumerate() {
            *value = std::ptr::read_unaligned(mapped.cast::<u32>().add(index));
        }
        device.unmap_memory(storage_memory);
    }
    let layout = TensorLayout3D::new(
        width,
        height,
        depth,
        stride_x as usize,
        stride_y as usize,
        stride_z as usize,
        capacity,
    )
    .map_err(|error| RuntimeError::DescriptorContract(error.to_string()))?;
    let mut expected_values = vec![0_u32; layout.capacity()];
    let mut written_positions = vec![false; layout.capacity()];
    for z in 0..depth {
        for y in 0..height {
            for x in 0..width {
                let physical_index = layout.physical_index(x, y, z).ok_or_else(|| {
                    RuntimeError::DescriptorContract(
                        "3D affine-stride CPU oracle produced an out-of-capacity index".to_owned(),
                    )
                })?;
                expected_values[physical_index] = value;
                written_positions[physical_index] = true;
            }
        }
    }
    let differential_passed = compare_u32(&expected_values, &output_values).is_ok();
    let residency_passed =
        resource_table.release(access_token).is_ok() && resource_table.evict(resource_id).is_ok();
    let output_checksum = output_values.iter().map(|value| u64::from(*value)).sum();
    let expected_checksum = expected_values.iter().map(|value| u64::from(*value)).sum();
    let untouched_elements = written_positions
        .iter()
        .filter(|written| !**written)
        .count();
    unsafe {
        device.destroy_fence(fence, None);
        device.destroy_command_pool(command_pool, None);
        device.destroy_pipeline(pipeline, None);
        device.destroy_pipeline_layout(pipeline_layout, None);
        device.destroy_shader_module(shader_module, None);
        device.destroy_descriptor_pool(descriptor_pool, None);
        device.destroy_descriptor_set_layout(descriptor_set_layout, None);
        device.free_memory(storage_memory, None);
        device.destroy_buffer(storage_buffer, None);
    }
    Ok(Global3dStridedWriteExecution {
        report: Global3dStridedWriteU32QueueSmokeReport {
            schema: "jadren-vulkan-global-3d-strided-write-u32-queue-smoke-0.1",
            artifact_entry_name: artifact.entry_name,
            artifact_word_count: artifact.words.len(),
            artifact_word_hash: stable_spirv_word_hash(&artifact.words),
            artifact_validated: true,
            physical_device_count,
            selected_device,
            queue_family_index,
            queue_execution: "passed",
            descriptor_setup: "passed",
            resource_binding_count: resources.len(),
            pipeline_execution: "passed",
            fence_execution: "passed",
            timeline_execution: if timeline_passed { "passed" } else { "failed" },
            timeline_value,
            data_kernel_execution: if differential_passed {
                "passed"
            } else {
                "failed"
            },
            differential_execution: if differential_passed {
                "passed"
            } else {
                "failed"
            },
            residency_execution: if residency_passed { "passed" } else { "failed" },
            width,
            height,
            depth,
            stride_x,
            stride_y,
            stride_z,
            capacity,
            dispatch_x: (width as u32).div_ceil(workgroup_size[0]),
            dispatch_y: (height as u32).div_ceil(workgroup_size[1]),
            dispatch_z: (depth as u32).div_ceil(workgroup_size[2]),
            last_physical_index: (width - 1) * stride_x as usize
                + (height - 1) * stride_y as usize
                + (depth - 1) * stride_z as usize,
            output_checksum,
            expected_checksum,
            untouched_elements,
        },
        output_values,
    })
}
fn run_global_3d_strided_write_on_instance(
    instance: &Instance,
) -> Result<Global3dStridedWriteU32QueueSmokeReport, RuntimeError> {
    run_global_3d_strided_write_with_config_on_instance(
        instance,
        Global3dStridedWriteConfig::fixture(),
    )
    .map(|execution| execution.report)
}

fn run_global_3d_strided_write_with_config_on_instance(
    instance: &Instance,
    config: Global3dStridedWriteConfig,
) -> Result<Global3dStridedWriteExecution, RuntimeError> {
    let config = config.validate()?;
    let physical_devices =
        unsafe { instance.enumerate_physical_devices() }.map_err(|code| RuntimeError::Vulkan {
            operation: "enumerate_global_3d_strided_write_physical_devices",
            code,
        })?;
    let (physical_device, queue_family_index) = physical_devices
        .iter()
        .find_map(|physical_device| {
            let families =
                unsafe { instance.get_physical_device_queue_family_properties(*physical_device) };
            families
                .iter()
                .enumerate()
                .find(|(_, properties)| {
                    properties.queue_count > 0
                        && properties.queue_flags.contains(vk::QueueFlags::COMPUTE)
                })
                .map(|(index, _)| (*physical_device, index as u32))
        })
        .ok_or(RuntimeError::NoComputeQueue)?;
    let properties = unsafe { instance.get_physical_device_properties(physical_device) };
    let selected_device = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    let priorities = [1.0_f32];
    let queue_info = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family_index)
        .queue_priorities(&priorities);
    let device_info =
        vk::DeviceCreateInfo::default().queue_create_infos(std::slice::from_ref(&queue_info));
    let device =
        unsafe { instance.create_device(physical_device, &device_info, None) }.map_err(|code| {
            RuntimeError::Vulkan {
                operation: "create_global_3d_strided_write_device",
                code,
            }
        })?;
    let result = run_global_3d_strided_write_on_device(
        instance,
        &device,
        physical_device,
        queue_family_index,
        physical_devices.len(),
        selected_device,
        config,
    );
    unsafe {
        let _ = device.device_wait_idle();
        device.destroy_device(None);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::{
        Global3dStridedWriteConfig, GlobalDynamicOperation, GlobalWriteArtifactConfig,
        jadren_vk_f32_add_one_array, jadren_vk_u32_3d_strided_write,
        jadren_vk_u32_3d_strided_write_async, jadren_vk_u32_3d_strided_write_async_complete,
        jadren_vk_u32_3d_strided_write_async_poll, jadren_vk_u32_3d_strided_write_async_release,
        jadren_vk_u32_binary_array, storage_add_artifact_fixture_module,
        storage_dynamic_index_add_fixture_module, storage_global_2d_strided_write_fixture_module,
        storage_global_2d_write_fixture_module, storage_global_3d_strided_write_fixture_module,
        storage_global_3d_write_fixture_module,
        storage_global_dynamic_index_arithmetic_fixture_module,
        storage_global_dynamic_index_f32_binary_fixture_module,
        storage_global_dynamic_index_fadd_fixture_module,
        storage_global_dynamic_index_vector_f32_binary_fixture_module,
        storage_global_dynamic_index_vector_f32_binary_fixture_module_lanes,
        storage_global_index_strided_write_fixture_module,
        storage_global_index_write_fixture_module,
    };
    use ash::vk;
    use jadren_codegen_spirv::{
        F32ArithmeticOp, SpirvOptions, emit_storage_add_artifact_from_jir,
        emit_storage_global_index_f32_binary_dynamic_length_artifact_from_jir,
        emit_storage_global_index_vector_f32_binary_dynamic_length_artifact_from_jir,
        emit_storage_global_index_vector_f32_binary_dynamic_length_lanes_artifact_from_jir,
        reflect_resources,
    };

    #[test]
    fn pins_the_portable_runtime_floor() {
        assert_eq!(vk::api_version_major(vk::API_VERSION_1_2), 1);
        assert_eq!(vk::api_version_minor(vk::API_VERSION_1_2), 2);
    }

    #[test]
    fn tensor3d_abi_config_rejects_zero_and_out_of_capacity_layouts() {
        let zero = Global3dStridedWriteConfig {
            width: 0,
            height: 1,
            depth: 1,
            stride_x: 1,
            stride_y: 1,
            stride_z: 1,
            capacity: 1,
            value: 42,
            workgroup_size: [4, 4, 2],
        };
        assert!(zero.validate().is_err());
        let out_of_capacity = Global3dStridedWriteConfig {
            width: 2,
            height: 2,
            depth: 2,
            stride_x: 1,
            stride_y: 5,
            stride_z: 13,
            capacity: 19,
            value: 7,
            workgroup_size: [4, 4, 2],
        };
        assert!(out_of_capacity.validate().is_err());
    }

    #[test]
    fn tensor3d_c_abi_rejects_null_output_before_vulkan_load() {
        let result = unsafe {
            jadren_vk_u32_3d_strided_write(std::ptr::null_mut(), 2, 2, 2, 1, 5, 13, 32, 7)
        };
        assert_eq!(result.status, -40);
        assert_eq!(result.capacity, 32);
    }

    #[test]
    fn tensor3d_async_c_abi_rejects_null_handles_and_output() {
        let begin = unsafe {
            jadren_vk_u32_3d_strided_write_async(std::ptr::null_mut(), 2, 2, 2, 1, 5, 13, 32, 7)
        };
        assert_eq!(begin.status, -40);
        assert!(begin.handle.is_null());
        assert_eq!(
            unsafe { jadren_vk_u32_3d_strided_write_async_poll(std::ptr::null_mut()) },
            -60
        );
        assert_eq!(
            unsafe { jadren_vk_u32_3d_strided_write_async_complete(std::ptr::null_mut()) }.status,
            -60
        );
        assert_eq!(
            unsafe { jadren_vk_u32_3d_strided_write_async_release(std::ptr::null_mut()) },
            -60
        );
    }

    #[test]
    fn binary_c_abi_rejects_invalid_operation_and_operand_before_vulkan_load() {
        let input = [1_u32];
        let mut output = [0_u32; 1];
        let invalid_operation =
            unsafe { jadren_vk_u32_binary_array(input.as_ptr(), output.as_mut_ptr(), 1, 10, 1) };
        assert_eq!(invalid_operation.status, -51);
        let zero_divisor =
            unsafe { jadren_vk_u32_binary_array(input.as_ptr(), output.as_mut_ptr(), 1, 3, 0) };
        assert_eq!(zero_divisor.status, -52);
        let invalid_shift =
            unsafe { jadren_vk_u32_binary_array(input.as_ptr(), output.as_mut_ptr(), 1, 8, 32) };
        assert_eq!(invalid_shift.status, -52);
        let null_input =
            unsafe { jadren_vk_u32_binary_array(std::ptr::null(), output.as_mut_ptr(), 1, 0, 1) };
        assert_eq!(null_input.status, -50);
    }

    #[test]
    fn dynamic_f32_c_abi_rejects_invalid_arguments_before_vulkan_load() {
        let input = [1.0_f32];
        let mut output = [0.0_f32; 1];
        assert_eq!(
            unsafe { jadren_vk_f32_add_one_array(std::ptr::null(), output.as_mut_ptr(), 1) }.status,
            -32
        );
        assert_eq!(
            unsafe { jadren_vk_f32_add_one_array(input.as_ptr(), output.as_mut_ptr(), 0) }.status,
            -33
        );
        let nan = [f32::NAN];
        assert_eq!(
            unsafe { jadren_vk_f32_add_one_array(nan.as_ptr(), output.as_mut_ptr(), 1) }.status,
            -34
        );
    }

    #[test]
    fn dynamic_f32_fixture_exposes_runtime_length_and_fadd_contract() {
        let module = storage_global_dynamic_index_fadd_fixture_module(1.0_f32.to_bits());
        let resources = reflect_resources(&module, jadren_jir::FunctionId::new(0)).unwrap();
        assert_eq!(module.functions[0].name, "global_add_dynamic_f32");
        assert_eq!(resources.len(), 3);
        assert_eq!(resources[0].name, "input");
        assert_eq!(resources[1].name, "output");
        assert_eq!(resources[2].name, "length");
        let instructions = &module.functions[0].blocks[0].instructions;
        assert_eq!(instructions.len(), 9);
        assert!(matches!(
            instructions[0].kind,
            jadren_jir::InstructionKind::Builtin(jadren_jir::BuiltinOp::GlobalInvocationIdX)
        ));
        assert!(matches!(
            instructions[6].kind,
            jadren_jir::InstructionKind::Binary {
                op: jadren_jir::BinaryOp::Add,
                ..
            }
        ));
    }

    #[test]
    fn dynamic_f32_artifact_fixture_exposes_subtract_and_multiply_contracts() {
        for (operation, expected_name, expected_jir) in [
            (
                F32ArithmeticOp::Subtract,
                "global_subtract_dynamic_f32",
                jadren_jir::BinaryOp::Subtract,
            ),
            (
                F32ArithmeticOp::Multiply,
                "global_multiply_dynamic_f32",
                jadren_jir::BinaryOp::Multiply,
            ),
        ] {
            let operand = super::f32_operation_operand(operation);
            let module = storage_global_dynamic_index_f32_binary_fixture_module(
                operand.to_bits(),
                operation,
            );
            let artifact = emit_storage_global_index_f32_binary_dynamic_length_artifact_from_jir(
                &module,
                jadren_jir::FunctionId::new(0),
                SpirvOptions::new([64, 1, 1]).unwrap(),
                operation,
            )
            .unwrap();
            assert_eq!(module.functions[0].name, expected_name);
            assert!(artifact.validate().is_ok());
            assert!(matches!(
                module.functions[0].blocks[0].instructions[6].kind,
                jadren_jir::InstructionKind::Binary { op, .. } if op == expected_jir
            ));
        }
    }

    #[test]
    fn dynamic_f32x4_artifact_fixture_exposes_subtract_and_multiply_contracts() {
        for (operation, expected_name, expected_jir) in [
            (
                F32ArithmeticOp::Subtract,
                "global_subtract_dynamic_f32x4",
                jadren_jir::BinaryOp::Subtract,
            ),
            (
                F32ArithmeticOp::Multiply,
                "global_multiply_dynamic_f32x4",
                jadren_jir::BinaryOp::Multiply,
            ),
        ] {
            let operand = super::f32_operation_operand(operation);
            let module =
                storage_global_dynamic_index_vector_f32_binary_fixture_module(operand, operation);
            let artifact =
                emit_storage_global_index_vector_f32_binary_dynamic_length_artifact_from_jir(
                    &module,
                    jadren_jir::FunctionId::new(0),
                    SpirvOptions::new([64, 1, 1]).unwrap(),
                    operation,
                )
                .unwrap();
            assert_eq!(module.functions[0].name, expected_name);
            assert!(artifact.validate().is_ok());
            assert!(matches!(
                module.functions[0].blocks[0].instructions[7].kind,
                jadren_jir::InstructionKind::VectorBinary { op, .. } if op == expected_jir
            ));
        }
    }

    #[test]
    fn dynamic_f32_vector_lane_artifacts_expose_native_x2_and_x3_contracts() {
        for lanes in [2_u16, 3_u16] {
            let operand = super::f32_operation_operand(F32ArithmeticOp::Add);
            let module = storage_global_dynamic_index_vector_f32_binary_fixture_module_lanes(
                operand,
                F32ArithmeticOp::Add,
                lanes,
            );
            let resources = reflect_resources(&module, jadren_jir::FunctionId::new(0)).unwrap();
            assert_eq!(
                module.functions[0].name,
                format!("global_add_dynamic_f32x{lanes}")
            );
            assert_eq!(resources[0].element_stride, Some(u32::from(lanes) * 4));
            assert_eq!(resources[1].element_stride, Some(u32::from(lanes) * 4));
            assert_eq!(resources[2].element_stride, Some(4));
            let artifact =
                emit_storage_global_index_vector_f32_binary_dynamic_length_lanes_artifact_from_jir(
                    &module,
                    jadren_jir::FunctionId::new(0),
                    SpirvOptions::new([64, 1, 1]).unwrap(),
                    F32ArithmeticOp::Add,
                    u32::from(lanes),
                )
                .unwrap();
            assert!(artifact.validate().is_ok());
            assert_eq!(artifact.entry_name, module.functions[0].name);
            assert_eq!(artifact.resources, resources);
        }
    }

    #[test]
    fn dynamic_f32_vector_lane_api_rejects_mismatched_or_unsupported_shapes() {
        assert!(
            super::run_global_dynamic_f32_vector_lanes_artifact_queue_with_values(&[
                vec![1.0, 2.0],
                vec![3.0],
            ])
            .is_err()
        );
        assert!(
            super::run_global_dynamic_f32_vector_lanes_artifact_queue_with_values(&[vec![1.0],])
                .is_err()
        );
        assert!(
            super::run_global_dynamic_f32_vector_lanes_artifact_queue_with_values(&[vec![
                f32::NAN,
                2.0
            ],])
            .is_err()
        );
    }

    #[test]
    fn descriptor_contract_comes_from_jir_reflection() {
        let module = storage_dynamic_index_add_fixture_module();
        let resources = reflect_resources(&module, jadren_jir::FunctionId::new(0)).unwrap();
        assert_eq!(resources.len(), 3);
        assert_eq!(resources[0].binding, 0);
        assert_eq!(resources[0].name, "input");
        assert_eq!(resources[1].binding, 1);
        assert_eq!(resources[1].name, "output");
        assert_eq!(resources[2].binding, 2);
        assert_eq!(resources[2].name, "index");
        assert_eq!(
            resources[0].address_space,
            jadren_jir::AddressSpace::Storage
        );
    }

    #[test]
    fn global_write_fixture_exposes_one_storage_resource() {
        let module = storage_global_index_write_fixture_module();
        let resources = reflect_resources(&module, jadren_jir::FunctionId::new(0)).unwrap();
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].binding, 0);
        assert_eq!(resources[0].name, "buffer");
        assert_eq!(module.functions[0].blocks[0].instructions.len(), 6);
        assert!(matches!(
            module.functions[0].blocks[0].instructions[0].kind,
            jadren_jir::InstructionKind::Builtin(jadren_jir::BuiltinOp::GlobalInvocationIdX)
        ));
    }

    #[test]
    fn global_write_tail_config_dispatches_two_groups_and_preserves_capacity_contract() {
        let config = GlobalWriteArtifactConfig::tail().validate().unwrap();
        assert_eq!(config.length, 70);
        assert_eq!(config.capacity, 128);
        assert_eq!(config.dispatch_x().unwrap(), 2);
        assert_eq!(config.buffer_size().unwrap(), 512);
    }

    #[test]
    fn storage_add_artifact_fixture_pins_one_resource_and_encoded_addend() {
        let module = storage_add_artifact_fixture_module();
        let resources = reflect_resources(&module, jadren_jir::FunctionId::new(0)).unwrap();
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].binding, 0);
        assert_eq!(resources[0].name, "data");
        let artifact = emit_storage_add_artifact_from_jir(
            &module,
            jadren_jir::FunctionId::new(0),
            SpirvOptions::new([1, 1, 1]).unwrap(),
        )
        .unwrap();
        assert_eq!(artifact.entry_name, "add_u32");
        assert_eq!(artifact.resources.len(), 1);
        assert_eq!(artifact.words.len(), 108);
        assert!(artifact.validate().is_ok());
    }

    #[test]
    fn global_strided_write_fixture_exposes_four_metadata_resources() {
        let module = storage_global_index_strided_write_fixture_module();
        let resources = reflect_resources(&module, jadren_jir::FunctionId::new(0)).unwrap();
        assert_eq!(resources.len(), 4);
        assert_eq!(resources[0].name, "buffer");
        assert_eq!(resources[1].name, "length");
        assert_eq!(resources[2].name, "stride");
        assert_eq!(resources[3].name, "capacity");
        assert_eq!(module.functions[0].blocks[0].instructions.len(), 10);
        assert!(matches!(
            module.functions[0].blocks[0].instructions[6].kind,
            jadren_jir::InstructionKind::Binary {
                op: jadren_jir::BinaryOp::Multiply,
                ..
            }
        ));
    }

    #[test]
    fn global_2d_write_fixture_exposes_xy_and_capacity_contract() {
        let module = storage_global_2d_write_fixture_module();
        let resources = reflect_resources(&module, jadren_jir::FunctionId::new(0)).unwrap();
        assert_eq!(resources.len(), 4);
        assert_eq!(resources[0].name, "buffer");
        assert_eq!(resources[1].name, "width");
        assert_eq!(resources[2].name, "height");
        assert_eq!(resources[3].name, "capacity");
        let instructions = &module.functions[0].blocks[0].instructions;
        assert_eq!(instructions.len(), 13);
        assert!(matches!(
            instructions[1].kind,
            jadren_jir::InstructionKind::Builtin(jadren_jir::BuiltinOp::GlobalInvocationIdY)
        ));
        assert!(matches!(
            instructions[9].kind,
            jadren_jir::InstructionKind::Binary {
                op: jadren_jir::BinaryOp::Add,
                ..
            }
        ));
    }

    #[test]
    fn global_2d_strided_write_fixture_exposes_affine_stride_contract() {
        let module = storage_global_2d_strided_write_fixture_module();
        let resources = reflect_resources(&module, jadren_jir::FunctionId::new(0)).unwrap();
        assert_eq!(resources.len(), 6);
        assert_eq!(resources[0].name, "buffer");
        assert_eq!(resources[1].name, "width");
        assert_eq!(resources[2].name, "height");
        assert_eq!(resources[3].name, "stride_x");
        assert_eq!(resources[4].name, "stride_y");
        assert_eq!(resources[5].name, "capacity");
        let instructions = &module.functions[0].blocks[0].instructions;
        assert_eq!(instructions.len(), 16);
        assert!(matches!(
            instructions[10].kind,
            jadren_jir::InstructionKind::Binary {
                op: jadren_jir::BinaryOp::Multiply,
                ..
            }
        ));
        assert!(matches!(
            instructions[12].kind,
            jadren_jir::InstructionKind::Binary {
                op: jadren_jir::BinaryOp::Add,
                ..
            }
        ));
    }

    #[test]
    fn global_3d_write_fixture_exposes_xyz_and_capacity_contract() {
        let module = storage_global_3d_write_fixture_module();
        let resources = reflect_resources(&module, jadren_jir::FunctionId::new(0)).unwrap();
        assert_eq!(resources.len(), 5);
        assert_eq!(resources[0].name, "buffer");
        assert_eq!(resources[1].name, "width");
        assert_eq!(resources[2].name, "height");
        assert_eq!(resources[3].name, "depth");
        assert_eq!(resources[4].name, "capacity");
        let instructions = &module.functions[0].blocks[0].instructions;
        assert_eq!(instructions.len(), 18);
        assert!(matches!(
            instructions[2].kind,
            jadren_jir::InstructionKind::Builtin(jadren_jir::BuiltinOp::GlobalInvocationIdZ)
        ));
        assert!(matches!(
            instructions[14].kind,
            jadren_jir::InstructionKind::Binary {
                op: jadren_jir::BinaryOp::Add,
                ..
            }
        ));
    }

    #[test]
    fn global_3d_strided_write_fixture_exposes_affine_stride_contract() {
        let module = storage_global_3d_strided_write_fixture_module();
        let resources = reflect_resources(&module, jadren_jir::FunctionId::new(0)).unwrap();
        assert_eq!(resources.len(), 8);
        assert_eq!(resources[0].name, "buffer");
        assert_eq!(resources[1].name, "width");
        assert_eq!(resources[2].name, "height");
        assert_eq!(resources[3].name, "depth");
        assert_eq!(resources[4].name, "stride_x");
        assert_eq!(resources[5].name, "stride_y");
        assert_eq!(resources[6].name, "stride_z");
        assert_eq!(resources[7].name, "capacity");
        let instructions = &module.functions[0].blocks[0].instructions;
        assert_eq!(instructions.len(), 22);
        assert!(matches!(
            instructions[2].kind,
            jadren_jir::InstructionKind::Builtin(jadren_jir::BuiltinOp::GlobalInvocationIdZ)
        ));
        assert!(matches!(
            instructions[17].kind,
            jadren_jir::InstructionKind::Binary {
                op: jadren_jir::BinaryOp::Multiply,
                ..
            }
        ));
        assert!(matches!(
            instructions[18].kind,
            jadren_jir::InstructionKind::Binary {
                op: jadren_jir::BinaryOp::Add,
                ..
            }
        ));
    }

    #[test]
    fn dynamic_multiply_fixture_preserves_jir_operation_contract() {
        let module = storage_global_dynamic_index_arithmetic_fixture_module(
            GlobalDynamicOperation::Multiply,
        );
        assert_eq!(module.functions[0].name, "global_multiply_dynamic_u32");
        let instructions = &module.functions[0].blocks[0].instructions;
        assert!(matches!(
            instructions[1].kind,
            jadren_jir::InstructionKind::Constant(jadren_jir::Constant::Integer { value: 2 })
        ));
        assert!(matches!(
            instructions[6].kind,
            jadren_jir::InstructionKind::Binary {
                op: jadren_jir::BinaryOp::Multiply,
                ..
            }
        ));
    }

    #[test]
    fn dynamic_native_arithmetic_fixtures_preserve_operation_contracts() {
        let cases = [
            (
                GlobalDynamicOperation::Subtract,
                "global_subtract_dynamic_u32",
                1_i128,
                jadren_jir::BinaryOp::Subtract,
            ),
            (
                GlobalDynamicOperation::Divide,
                "global_divide_dynamic_u32",
                2_i128,
                jadren_jir::BinaryOp::Divide,
            ),
            (
                GlobalDynamicOperation::Remainder,
                "global_remainder_dynamic_u32",
                2_i128,
                jadren_jir::BinaryOp::Remainder,
            ),
            (
                GlobalDynamicOperation::BitAnd,
                "global_bitand_dynamic_u32",
                1_i128,
                jadren_jir::BinaryOp::BitAnd,
            ),
            (
                GlobalDynamicOperation::BitOr,
                "global_bitor_dynamic_u32",
                1_i128,
                jadren_jir::BinaryOp::BitOr,
            ),
            (
                GlobalDynamicOperation::BitXor,
                "global_bitxor_dynamic_u32",
                1_i128,
                jadren_jir::BinaryOp::BitXor,
            ),
            (
                GlobalDynamicOperation::ShiftLeft,
                "global_shift_left_dynamic_u32",
                1_i128,
                jadren_jir::BinaryOp::ShiftLeft,
            ),
            (
                GlobalDynamicOperation::ShiftRight,
                "global_shift_right_dynamic_u32",
                1_i128,
                jadren_jir::BinaryOp::ShiftRight,
            ),
        ];
        for (operation, name, operand, expected_binary) in cases {
            let module = storage_global_dynamic_index_arithmetic_fixture_module(operation);
            assert_eq!(module.functions[0].name, name);
            let instructions = &module.functions[0].blocks[0].instructions;
            assert!(matches!(
                instructions[1].kind,
                jadren_jir::InstructionKind::Constant(jadren_jir::Constant::Integer { value })
                    if value == operand
            ));
            assert!(matches!(
                instructions[6].kind,
                jadren_jir::InstructionKind::Binary { op, .. }
                    if op == expected_binary
            ));
        }
    }
}
