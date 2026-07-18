#![allow(unsafe_code)]
// JADREN-UNSAFE-AUDIT: the Windows COM/D3D12 ABI boundary is isolated here;
// every pointer is checked before use and released through the matching COM
// vtable operation in this crate.

//! Native DirectX 12 device and compute smoke runtime for JAD-1314.
//!
//! The device probe proves the native queue/list/fence lifecycle. The compute
//! probe additionally compiles a small HLSL fixture with the host's Windows
//! SDK `dxc.exe`, creates an explicit root signature and UAV resources,
//! dispatches two workgroups, and validates a readback against a CPU oracle.
//! General Jadren JIR-to-DXIL lowering remains a separate backend gate.

use std::collections::BTreeMap;
use std::error::Error;
use std::ffi::c_void;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jadren_codegen_spirv::{F32ArithmeticOp, ResourceAccess, ResourceElementType};
use jadren_codegen_spirv::{
    SpirvArtifact, SpirvOptions, emit_storage_add_artifact_from_jir,
    emit_storage_global_2d_strided_write_artifact_from_jir,
    emit_storage_global_2d_write_artifact_from_jir,
    emit_storage_global_3d_strided_write_artifact_from_jir,
    emit_storage_global_3d_write_artifact_from_jir,
    emit_storage_global_index_add_dynamic_length_artifact_from_jir,
    emit_storage_global_index_binary_dynamic_length_artifact_from_jir,
    emit_storage_global_index_f32_binary_dynamic_length_artifact_from_jir,
    emit_storage_global_index_fadd_dynamic_length_artifact_from_jir,
    emit_storage_global_index_strided_write_artifact_from_jir,
    emit_storage_global_index_vector_f32_binary_dynamic_length_artifact_from_jir,
    emit_storage_global_index_vector_f32_binary_dynamic_length_lanes_artifact_from_jir,
    emit_storage_global_index_write_artifact_from_jir, validate_spirv,
};
use jadren_gpu_runtime::{
    ArtifactDispatchRequest, ArtifactResourceRequest, ArtifactSourceBackend,
    ArtifactSourceTranslationError, ArtifactSourceTranslationReport, BackendProbe,
    DispatchGeometry, FpPolicy, GpuBackend, ResourceTable, SpirvRawModuleContract,
    SpirvSourceReportWordsError, SpirvSourceTranslationError, SpirvSourceTranslationReport,
    inspect_spirv_source_module, prepare_artifact_dispatch, select_spirv_raw_output_binding,
    stable_source_hash, stable_spirv_word_hash, translate_spirv_artifact_source,
    translate_spirv_source_report_for_backend, validate_spirv_artifact_contract,
    validate_spirv_raw_native_adapter, validate_spirv_source_report_words,
};
use jadren_jir::{AddressSpace, BinaryOp, FunctionId, Module};
use libloading::{Library, Symbol};
use serde::Serialize;

fn map_raw_source_words_error(error: SpirvSourceReportWordsError) -> DirectX12Error {
    match error {
        SpirvSourceReportWordsError::NativeSpirvTransport { .. }
        | SpirvSourceReportWordsError::SourceBackendMismatch { .. } => {
            DirectX12Error::InvalidSpirv("raw-source-backend")
        }
        SpirvSourceReportWordsError::WordCountMismatch { .. } => {
            DirectX12Error::InvalidSpirv("raw-source-word-count")
        }
        SpirvSourceReportWordsError::WordHashMismatch { .. } => {
            DirectX12Error::InvalidSpirv("raw-source-word-hash")
        }
        SpirvSourceReportWordsError::Source(_) => {
            DirectX12Error::InvalidSpirv("raw-source-word-contract")
        }
        SpirvSourceReportWordsError::IdentityMismatch => {
            DirectX12Error::InvalidSpirv("raw-source-report-identity")
        }
        SpirvSourceReportWordsError::Native(_) => {
            DirectX12Error::InvalidSpirv("raw-source-native-plan")
        }
        SpirvSourceReportWordsError::Specialization(_) => {
            DirectX12Error::InvalidSpirv("raw-source-specialization")
        }
    }
}

fn validate_shared_artifact(artifact: &SpirvArtifact) -> Result<(), DirectX12Error> {
    validate_spirv_artifact_contract(artifact)
        .map(|_| ())
        .map_err(|_| DirectX12Error::InvalidSpirv("shared-artifact-contract"))
}

const D3D_FEATURE_LEVEL_11_0: u32 = 0xB000;
const D3D12_COMMAND_LIST_TYPE_DIRECT: i32 = 0;
const D3D12_COMMAND_QUEUE_FLAG_NONE: u32 = 0;
const D3D12_FENCE_FLAG_NONE: u32 = 0;
const D3D12_HEAP_TYPE_DEFAULT: i32 = 1;
const D3D12_HEAP_TYPE_UPLOAD: i32 = 2;
const D3D12_HEAP_TYPE_READBACK: i32 = 3;
const D3D12_HEAP_FLAG_NONE: u32 = 0;
const D3D12_RESOURCE_DIMENSION_BUFFER: u32 = 1;
const D3D12_TEXTURE_LAYOUT_ROW_MAJOR: u32 = 1;
const D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS: u32 = 0x4;
const D3D12_RESOURCE_STATE_COPY_DEST: u32 = 0x400;
const D3D12_RESOURCE_STATE_COPY_SOURCE: u32 = 0x800;
const D3D12_RESOURCE_STATE_UNORDERED_ACCESS: u32 = 0x8;
const D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE: u32 = 0x40;
const D3D12_RESOURCE_STATE_GENERIC_READ: u32 = 0xAC3;
const D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV: u32 = 0;
const D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE: u32 = 1;
const D3D12_UAV_DIMENSION_BUFFER: u32 = 1;
const D3D12_SRV_DIMENSION_BUFFER: u32 = 1;
const D3D12_BUFFER_UAV_FLAG_RAW: u32 = 0x1;
const D3D12_BUFFER_SRV_FLAG_RAW: u32 = 0x1;
const D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING: u32 = 0x1688;
const D3D12_RESOURCE_BARRIER_TYPE_TRANSITION: u32 = 0;
const D3D12_RESOURCE_BARRIER_FLAG_NONE: u32 = 0;
const D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES: u32 = 0xFFFF_FFFF;
const DXGI_FORMAT_UNKNOWN: u32 = 0;
const DXGI_FORMAT_R32_TYPELESS: u32 = 39;
const D3D12_PIPELINE_STATE_FLAG_NONE: u32 = 0;
const D3D12_ROOT_SIGNATURE_VERSION_1_0: u32 = 1;
const D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE: u32 = 0;
#[allow(dead_code)]
const D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS: u32 = 1;
const D3D12_DESCRIPTOR_RANGE_TYPE_UAV: u32 = 1;
const D3D12_DESCRIPTOR_RANGE_TYPE_SRV: u32 = 0;
const D3D12_ROOT_SIGNATURE_FLAG_NONE: u32 = 0;
const D3D12_SHADER_VISIBILITY_ALL: u32 = 0;
const S_OK: i32 = 0;

// Narrow SPIR-V decoder opcodes used by the internal fallback translator.
// This is intentionally limited to the canonical three-resource dynamic
// BinaryOp artifact emitted by `jadren-codegen-spirv`.
const SPIRV_OP_CONSTANT: u16 = 43;
const SPIRV_OP_STORE: u16 = 62;
const SPIRV_OP_TYPE_FLOAT: u16 = 22;
const SPIRV_OP_TYPE_VECTOR: u16 = 23;
const SPIRV_OP_CONSTANT_COMPOSITE: u16 = 44;
const SPIRV_OP_FADD: u16 = 129;
const SPIRV_OP_FSUB: u16 = 131;
const SPIRV_OP_FMUL: u16 = 133;
const SPIRV_OP_IADD: u16 = 128;
const SPIRV_OP_ISUB: u16 = 130;
const SPIRV_OP_IMUL: u16 = 132;
const SPIRV_OP_UDIV: u16 = 134;
const SPIRV_OP_UMOD: u16 = 137;
const SPIRV_OP_ULT: u16 = 176;
const SPIRV_OP_SHIFT_RIGHT_LOGICAL: u16 = 194;
const SPIRV_OP_SHIFT_LEFT_LOGICAL: u16 = 196;
const SPIRV_OP_BITWISE_OR: u16 = 197;
const SPIRV_OP_BITWISE_XOR: u16 = 198;
const SPIRV_OP_BITWISE_AND: u16 = 199;
const F32_VECTOR_CAPACITY: usize = 128;
const F32_VECTOR_WORKGROUP_SIZE: [u32; 3] = [64, 1, 1];
const F32_VECTOR_OPERAND: f32 = 1.0;
const F32_VECTOR_MULTIPLIER: f32 = 2.0;

#[repr(C)]
#[derive(Clone, Copy)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

const IID_ID3D12_DEVICE: Guid = Guid {
    data1: 0x1898_19f1,
    data2: 0x1db6,
    data3: 0x4b57,
    data4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7],
};

const IID_ID3D12_COMMAND_QUEUE: Guid = Guid {
    data1: 0x0ec8_70a6,
    data2: 0x5d7e,
    data3: 0x4c22,
    data4: [0x8c, 0xfc, 0x5b, 0xaa, 0xe0, 0x76, 0x16, 0xed],
};

const IID_ID3D12_COMMAND_ALLOCATOR: Guid = Guid {
    data1: 0x6102_dee4,
    data2: 0xaf59,
    data3: 0x4b09,
    data4: [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24],
};

const IID_ID3D12_GRAPHICS_COMMAND_LIST: Guid = Guid {
    data1: 0x5b16_0d0f,
    data2: 0xac1b,
    data3: 0x4185,
    data4: [0x8b, 0xa8, 0xb3, 0xae, 0x42, 0xa5, 0xa4, 0x55],
};

const IID_ID3D12_RESOURCE: Guid = Guid {
    data1: 0x6964_42be,
    data2: 0xa72e,
    data3: 0x4059,
    data4: [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad],
};

const IID_ID3D12_DESCRIPTOR_HEAP: Guid = Guid {
    data1: 0x8efb_471d,
    data2: 0x616c,
    data3: 0x4f49,
    data4: [0x90, 0xf7, 0x12, 0x7b, 0xb7, 0x63, 0xfa, 0x51],
};

const IID_ID3D12_PIPELINE_STATE: Guid = Guid {
    data1: 0x765a_30f3,
    data2: 0xf624,
    data3: 0x4c6f,
    data4: [0xa8, 0x28, 0xac, 0xe9, 0x48, 0x62, 0x24, 0x45],
};

const IID_ID3D12_ROOT_SIGNATURE: Guid = Guid {
    data1: 0xc54a_6b66,
    data2: 0x72df,
    data3: 0x4ee8,
    data4: [0x8b, 0xe5, 0xa9, 0x46, 0xa1, 0x42, 0x92, 0x14],
};

const IID_ID3D12_FENCE: Guid = Guid {
    data1: 0x0a75_3dcf,
    data2: 0xc4d8,
    data3: 0x4b91,
    data4: [0xad, 0xf6, 0xbe, 0x5a, 0x60, 0xd9, 0x5a, 0x76],
};

#[repr(C)]
#[derive(Clone, Copy)]
struct CommandQueueDesc {
    queue_type: i32,
    priority: i32,
    flags: u32,
    node_mask: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct HeapProperties {
    heap_type: i32,
    cpu_page_property: i32,
    memory_pool_preference: i32,
    creation_node_mask: u32,
    visible_node_mask: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SampleDesc {
    count: u32,
    quality: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ResourceDesc {
    dimension: u32,
    alignment: u64,
    width: u64,
    height: u32,
    depth_or_array_size: u16,
    mip_levels: u16,
    format: u32,
    sample_desc: SampleDesc,
    layout: u32,
    flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct DescriptorHeapDesc {
    heap_type: u32,
    num_descriptors: u32,
    flags: u32,
    node_mask: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CpuDescriptorHandle {
    ptr: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct GpuDescriptorHandle {
    ptr: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct UnorderedAccessViewBuffer {
    first_element: u64,
    num_elements: u32,
    structure_byte_stride: u32,
    counter_offset_in_bytes: u64,
    flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
union UnorderedAccessViewData {
    buffer: UnorderedAccessViewBuffer,
    _padding: [u64; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct UnorderedAccessViewDesc {
    format: u32,
    view_dimension: u32,
    data: UnorderedAccessViewData,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ShaderResourceViewBuffer {
    first_element: u64,
    num_elements: u32,
    structure_byte_stride: u32,
    flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
union ShaderResourceViewData {
    buffer: ShaderResourceViewBuffer,
    _padding: [u64; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ShaderResourceViewDesc {
    format: u32,
    view_dimension: u32,
    shader4_component_mapping: u32,
    data: ShaderResourceViewData,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ResourceTransition {
    resource: *mut c_void,
    subresource: u32,
    state_before: u32,
    state_after: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ResourceBarrier {
    barrier_type: u32,
    flags: u32,
    transition: ResourceTransition,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ShaderBytecode {
    pointer: *const c_void,
    length: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CachedPipelineState {
    pointer: *const c_void,
    length: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ComputePipelineStateDesc {
    root_signature: *mut c_void,
    compute_shader: ShaderBytecode,
    cached_pso: CachedPipelineState,
    flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Range {
    begin: usize,
    end: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RootDescriptorRange {
    range_type: u32,
    num_descriptors: u32,
    base_shader_register: u32,
    register_space: u32,
    offset_in_descriptors_from_table_start: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RootDescriptorTable {
    num_descriptor_ranges: u32,
    descriptor_ranges: *const RootDescriptorRange,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RootConstants {
    shader_register: u32,
    register_space: u32,
    num_32_bit_values: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RootDescriptor {
    shader_register: u32,
    register_space: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
union RootParameterData {
    descriptor_table: RootDescriptorTable,
    constants: RootConstants,
    descriptor: RootDescriptor,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RootParameter {
    parameter_type: u32,
    data: RootParameterData,
    shader_visibility: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RootSignatureDesc {
    num_parameters: u32,
    parameters: *const RootParameter,
    num_static_samplers: u32,
    static_samplers: *const c_void,
    flags: u32,
}

type D3D12CreateDevice = unsafe extern "system" fn(
    adapter: *mut c_void,
    minimum_feature_level: u32,
    riid: *const Guid,
    device: *mut *mut c_void,
) -> i32;

type Release = unsafe extern "system" fn(object: *mut c_void) -> u32;
type GetNodeCount = unsafe extern "system" fn(device: *mut c_void) -> u32;
type CreateCommandQueue = unsafe extern "system" fn(
    device: *mut c_void,
    desc: *const CommandQueueDesc,
    riid: *const Guid,
    queue: *mut *mut c_void,
) -> i32;
type CreateFence = unsafe extern "system" fn(
    device: *mut c_void,
    initial_value: u64,
    flags: u32,
    riid: *const Guid,
    fence: *mut *mut c_void,
) -> i32;
type SerializeRootSignature = unsafe extern "system" fn(
    desc: *const RootSignatureDesc,
    version: u32,
    blob: *mut *mut c_void,
    error_blob: *mut *mut c_void,
) -> i32;
type BlobGetBufferPointer = unsafe extern "system" fn(blob: *mut c_void) -> *const c_void;
type BlobGetBufferSize = unsafe extern "system" fn(blob: *mut c_void) -> usize;
type QueueSignal =
    unsafe extern "system" fn(queue: *mut c_void, fence: *mut c_void, value: u64) -> i32;
type GetCompletedValue = unsafe extern "system" fn(fence: *mut c_void) -> u64;
type CreateCommandAllocator = unsafe extern "system" fn(
    device: *mut c_void,
    queue_type: i32,
    riid: *const Guid,
    allocator: *mut *mut c_void,
) -> i32;
type CreateCommandList = unsafe extern "system" fn(
    device: *mut c_void,
    node_mask: u32,
    queue_type: i32,
    allocator: *mut c_void,
    initial_pipeline_state: *mut c_void,
    riid: *const Guid,
    command_list: *mut *mut c_void,
) -> i32;
type CloseCommandList = unsafe extern "system" fn(command_list: *mut c_void) -> i32;
type ExecuteCommandLists =
    unsafe extern "system" fn(queue: *mut c_void, count: u32, command_lists: *const *mut c_void);
type CreateCommittedResource = unsafe extern "system" fn(
    device: *mut c_void,
    heap_properties: *const HeapProperties,
    heap_flags: u32,
    resource_desc: *const ResourceDesc,
    initial_state: u32,
    optimized_clear_value: *const c_void,
    riid: *const Guid,
    resource: *mut *mut c_void,
) -> i32;
type MapResource = unsafe extern "system" fn(
    resource: *mut c_void,
    subresource: u32,
    read_range: *const Range,
    data: *mut *mut c_void,
) -> i32;
type UnmapResource =
    unsafe extern "system" fn(resource: *mut c_void, subresource: u32, written_range: *const Range);
type CreateDescriptorHeap = unsafe extern "system" fn(
    device: *mut c_void,
    desc: *const DescriptorHeapDesc,
    riid: *const Guid,
    heap: *mut *mut c_void,
) -> i32;
type GetDescriptorHandleIncrementSize =
    unsafe extern "system" fn(device: *mut c_void, heap_type: u32) -> u32;
type GetCpuDescriptorHandleForHeapStart =
    unsafe extern "system" fn(heap: *mut c_void, result: *mut CpuDescriptorHandle);
type GetGpuDescriptorHandleForHeapStart =
    unsafe extern "system" fn(heap: *mut c_void, result: *mut GpuDescriptorHandle);
type CreateUnorderedAccessView = unsafe extern "system" fn(
    device: *mut c_void,
    resource: *mut c_void,
    counter_resource: *mut c_void,
    desc: *const UnorderedAccessViewDesc,
    destination: CpuDescriptorHandle,
);
type CreateShaderResourceView = unsafe extern "system" fn(
    device: *mut c_void,
    resource: *mut c_void,
    desc: *const ShaderResourceViewDesc,
    destination: CpuDescriptorHandle,
);
type CreateComputePipelineState = unsafe extern "system" fn(
    device: *mut c_void,
    desc: *const ComputePipelineStateDesc,
    riid: *const Guid,
    pipeline_state: *mut *mut c_void,
) -> i32;
type CreateRootSignature = unsafe extern "system" fn(
    device: *mut c_void,
    node_mask: u32,
    blob: *const c_void,
    blob_length: usize,
    riid: *const Guid,
    root_signature: *mut *mut c_void,
) -> i32;
type CopyBufferRegion = unsafe extern "system" fn(
    command_list: *mut c_void,
    destination: *mut c_void,
    destination_offset: u64,
    source: *mut c_void,
    source_offset: u64,
    num_bytes: u64,
);
type ResourceBarrierFn = unsafe extern "system" fn(
    command_list: *mut c_void,
    num_barriers: u32,
    barriers: *const ResourceBarrier,
);
type SetDescriptorHeaps =
    unsafe extern "system" fn(command_list: *mut c_void, num_heaps: u32, heaps: *const *mut c_void);
type SetComputeRootSignature =
    unsafe extern "system" fn(command_list: *mut c_void, root_signature: *mut c_void);
type SetComputeRootDescriptorTable = unsafe extern "system" fn(
    command_list: *mut c_void,
    root_parameter_index: u32,
    base_descriptor: GpuDescriptorHandle,
);
#[allow(dead_code)]
type SetComputeRoot32BitConstant = unsafe extern "system" fn(
    command_list: *mut c_void,
    root_parameter_index: u32,
    shader_input: u32,
    dest_offset: u32,
);
type SetPipelineState =
    unsafe extern "system" fn(command_list: *mut c_void, pipeline_state: *mut c_void);
type Dispatch = unsafe extern "system" fn(
    command_list: *mut c_void,
    thread_group_count_x: u32,
    thread_group_count_y: u32,
    thread_group_count_z: u32,
);

/// Native probe result.
#[derive(Clone, Debug, Serialize)]
pub struct DirectX12DeviceSmokeReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Backend identifier.
    pub backend: &'static str,
    /// Whether d3d12.dll loaded and exported D3D12CreateDevice.
    pub api_loaded: bool,
    /// Whether ID3D12Device creation succeeded.
    pub device_created: bool,
    /// Whether a direct command queue was created.
    pub compute_queue_created: bool,
    /// Whether an empty direct command list was closed successfully.
    pub command_list_recorded: bool,
    /// Whether the queue accepted that command list for execution.
    pub command_list_submitted: bool,
    /// Whether the queue accepted a native fence signal.
    pub fence_signaled: bool,
    /// Whether the signaled fence value was observed as completed.
    pub fence_completed: bool,
    /// Device node count, when device creation succeeded.
    pub node_count: Option<u32>,
    /// Feature level requested by the probe.
    pub minimum_feature_level: &'static str,
    /// DXIL translation status, intentionally not claimed by this probe.
    pub shader_translation: &'static str,
    /// Overall probe result.
    pub result: &'static str,
    /// Completion mechanism used by this probe.
    pub completion_model: &'static str,
    /// Stable diagnostic for a failed host call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Native DirectX12 compute result for the verified u32 add-one fixture.
#[derive(Clone, Debug, Serialize)]
pub struct DirectX12ComputeSmokeReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Number of input/output elements processed by the shader.
    pub length: u32,
    /// First output element observed after readback.
    pub first_output: u32,
    /// Last output element observed after readback.
    pub last_output: u32,
    /// Output checksum.
    pub output_checksum: u64,
    /// Whether the DXIL blob passed container validation.
    pub dxil_validated: bool,
    /// Whether the D3D12 compute PSO was created.
    pub pipeline_created: bool,
    /// Whether command-list execution and fence completion succeeded.
    pub execution_completed: bool,
    /// Shader translation tool used by this smoke.
    pub shader_translation: &'static str,
    /// Overall result.
    pub result: &'static str,
}

/// Native DirectX 12 result for one parametrized runtime-length `u32` binary
/// dispatch. The shader is compiled from the requested operation and the
/// mapped output is compared with the same host oracle used by the report.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DirectX12BinarySmokeReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Operation selected for this dispatch.
    pub operation: &'static str,
    /// Constant `u32` operand embedded in the generated HLSL.
    pub operand: u32,
    /// First value uploaded to the input resource.
    pub input_start: u32,
    /// Difference between adjacent uploaded input values.
    pub input_stride: u32,
    /// Number of logical elements processed by the shader.
    pub length: u32,
    /// Number of UAV resources bound to the dispatch.
    pub resource_binding_count: u32,
    /// First output element observed after readback.
    pub first_output: u32,
    /// Last output element observed after readback.
    pub last_output: u32,
    /// Output checksum.
    pub output_checksum: u64,
    /// Whether the DXIL blob passed container validation.
    pub dxil_validated: bool,
    /// Whether the D3D12 compute PSO was created.
    pub pipeline_created: bool,
    /// Whether command-list execution and fence completion succeeded.
    pub execution_completed: bool,
    /// Shader translation tool used by this smoke.
    pub shader_translation: &'static str,
    /// Overall result.
    pub result: &'static str,
}

/// Native result for a runtime-length BinaryOp dispatched from a validated
/// Jadren SPIR-V artifact. The nested binary report carries the readback
/// values; these fields make the artifact boundary independently auditable.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DirectX12ArtifactExecutionReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Artifact entry point used for translation and dispatch.
    pub entry_name: String,
    /// Number of reflected artifact resources.
    pub artifact_resource_binding_count: usize,
    /// Number of words in the validated artifact.
    pub artifact_word_count: usize,
    /// Stable FNV-1a hash of the validated little-endian SPIR-V word stream.
    pub artifact_word_hash: u64,
    /// Operation requested by the JIR lowering.
    pub operation: &'static str,
    /// Whether the artifact passed structural validation.
    pub artifact_validated: bool,
    /// Whether the selected artifact translation path produced valid DXIL.
    pub dxil_translated: bool,
    /// Whether native D3D12 execution and differential readback completed.
    pub execution_completed: bool,
    /// Native output checksum.
    pub output_checksum: u64,
    /// First mapped output value when the artifact has an array result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_output: Option<u32>,
    /// Last logical mapped output value when the artifact has an array result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_output: Option<u32>,
    /// Logical kernel length when the artifact carries a bounded array shape.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logical_length: Option<u32>,
    /// Backing storage capacity when it exceeds the logical kernel length.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capacity: Option<u32>,
    /// Number of mapped tail elements proven unchanged by differential readback.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub untouched_tail_count: Option<u32>,
    /// Translation path used for the artifact.
    pub translation_path: &'static str,
    /// Overall result.
    pub result: &'static str,
}

/// Native result for the verified runtime-length scalar `f32` binary artifact.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DirectX12F32ArtifactExecutionReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Artifact entry point used for translation and dispatch.
    pub entry_name: String,
    /// Number of reflected artifact resources.
    pub artifact_resource_binding_count: usize,
    /// Number of words in the validated artifact.
    pub artifact_word_count: usize,
    /// Stable FNV-1a hash of the validated little-endian SPIR-V word stream.
    pub artifact_word_hash: u64,
    /// Constant f32 addend used by the kernel.
    pub addend: f32,
    /// Binary operation encoded by the artifact.
    pub operation: &'static str,
    /// Number of logical elements processed by the kernel.
    pub logical_length: u32,
    /// First output value observed after readback.
    pub first_output: f32,
    /// Last output value observed after readback.
    pub last_output: f32,
    /// Exact output checksum in f64 form for JSON stability.
    pub output_checksum: f64,
    /// Whether the artifact passed structural and shape validation.
    pub artifact_validated: bool,
    /// Whether the selected translation path produced valid DXIL.
    pub dxil_translated: bool,
    /// Whether native D3D12 execution and exact bit differential completed.
    pub execution_completed: bool,
    /// Translation path used for the artifact.
    pub translation_path: &'static str,
    /// Overall result.
    pub result: &'static str,
}

/// Native result for the verified runtime-length `f32x4` artifact.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DirectX12F32VectorArtifactExecutionReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Artifact entry point used for translation and dispatch.
    pub entry_name: String,
    /// Number of reflected artifact resources.
    pub artifact_resource_binding_count: usize,
    /// Number of words in the validated artifact.
    pub artifact_word_count: usize,
    /// Stable FNV-1a hash of the validated little-endian SPIR-V word stream.
    pub artifact_word_hash: u64,
    /// Scalar f32 value splatted into each output lane.
    pub operand: f32,
    /// Arithmetic operation applied to every vector lane.
    pub operation: &'static str,
    /// Number of logical vector elements processed by the kernel.
    pub logical_length: u32,
    /// Physical vector capacity allocated for the dispatch.
    pub capacity: u32,
    /// First logical output vector.
    pub first_output: [f32; 4],
    /// Last logical output vector.
    pub last_output: [f32; 4],
    /// Exact input lane checksum.
    pub input_checksum: f64,
    /// Exact output lane checksum.
    pub output_checksum: f64,
    /// Number of capacity vectors left untouched after logical length.
    pub untouched_tail_count: u32,
    /// Whether the artifact passed structural and vector shape validation.
    pub artifact_validated: bool,
    /// Whether the selected translation path produced valid DXIL.
    pub dxil_translated: bool,
    /// Whether native D3D12 execution and exact lane differential completed.
    pub execution_completed: bool,
    /// Translation path used for the artifact.
    pub translation_path: &'static str,
    /// Overall result.
    pub result: &'static str,
}

/// Native result for the verified runtime-length `f32x2`/`f32x3`/`f32x4`
/// artifact path. The existing fixed-width x4 report remains unchanged.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DirectX12F32VectorLanesArtifactExecutionReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Artifact entry point used for translation and dispatch.
    pub entry_name: String,
    /// Vector lane count.
    pub lane_count: u32,
    /// Number of reflected artifact resources.
    pub artifact_resource_binding_count: usize,
    /// Number of words in the validated artifact.
    pub artifact_word_count: usize,
    /// Stable FNV-1a hash of the validated little-endian SPIR-V word stream.
    pub artifact_word_hash: u64,
    /// Scalar f32 value splatted into each output lane.
    pub operand: f32,
    /// Arithmetic operation applied to every vector lane.
    pub operation: &'static str,
    /// Number of logical vector elements processed by the kernel.
    pub logical_length: u32,
    /// Physical vector capacity allocated for the dispatch.
    pub capacity: u32,
    /// First logical output vector.
    pub first_output: Vec<f32>,
    /// Last logical output vector.
    pub last_output: Vec<f32>,
    /// Exact input lane checksum.
    pub input_checksum: f64,
    /// Exact output lane checksum.
    pub output_checksum: f64,
    /// Number of capacity vectors left untouched after logical length.
    pub untouched_tail_count: u32,
    /// Whether the artifact passed structural and vector shape validation.
    pub artifact_validated: bool,
    /// Whether the selected translation path produced valid DXIL.
    pub dxil_translated: bool,
    /// Whether native D3D12 execution and exact lane differential completed.
    pub execution_completed: bool,
    /// Translation path used for the artifact.
    pub translation_path: &'static str,
    /// Overall result.
    pub result: &'static str,
}

/// Runtime length, physical stride and store value for the narrow one-
/// dimensional DX12 artifact smoke path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GlobalStridedWriteArtifactConfig {
    /// Constant written to each reachable physical output index.
    pub value: u32,
    /// Number of logical `GlobalInvocationId.x` invocations.
    pub length: u32,
    /// Physical element distance between adjacent logical writes.
    pub stride: u32,
    /// Number of addressable physical output elements.
    pub capacity: u32,
}

/// Dimensions, affine strides and store value for the narrow 2D DX12 artifact
/// smoke path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Global2dStridedWriteArtifactConfig {
    /// Constant written to each reachable physical output index.
    pub value: u32,
    /// Logical X dimension.
    pub width: u32,
    /// Logical Y dimension.
    pub height: u32,
    /// Physical element stride along X.
    pub stride_x: u32,
    /// Physical element stride along Y.
    pub stride_y: u32,
    /// Number of addressable physical output elements.
    pub capacity: u32,
}

/// Dimensions and store value for the narrow 3D row-major DX12 artifact smoke
/// path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Global3dWriteArtifactConfig {
    /// Constant written to each logical output element.
    pub value: u32,
    /// Logical X dimension.
    pub width: u32,
    /// Logical Y dimension.
    pub height: u32,
    /// Logical Z dimension.
    pub depth: u32,
    /// Number of addressable physical output elements.
    pub capacity: u32,
}

/// Dimensions, affine strides and store value for the narrow 3D DX12 artifact
/// smoke path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Global3dStridedWriteArtifactConfig {
    /// Constant written to each reachable physical output index.
    pub value: u32,
    /// Logical X dimension.
    pub width: u32,
    /// Logical Y dimension.
    pub height: u32,
    /// Logical Z dimension.
    pub depth: u32,
    /// Physical element stride along X.
    pub stride_x: u32,
    /// Physical element stride along Y.
    pub stride_y: u32,
    /// Physical element stride along Z.
    pub stride_z: u32,
    /// Number of addressable physical output elements.
    pub capacity: u32,
}

/// One byte payload and structured element stride for a generic DX12 UAV.
///
/// The bytes must contain a whole number of elements. The stride is encoded in
/// the native structured-buffer view and therefore must match the element type
/// used by the already translated DXIL. This boundary transports the payload;
/// it does not infer or validate a shader's source-level type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UavBindingPayload<'a> {
    /// Host-side identity used to detect accidental resource aliasing.
    ///
    /// Read-only artifact bindings may share an identity when their payload
    /// shape and bytes agree. Any alias group containing a writable artifact
    /// binding is rejected before native resource creation.
    pub resource_id: u64,
    /// Host bytes uploaded to the corresponding artifact resource binding.
    pub bytes: &'a [u8],
    /// Size in bytes of one structured element.
    pub element_stride: u32,
}

/// Executes a validated SPIR-V artifact through a dynamic DX12 UAV table.
///
/// The caller supplies already translated DXIL and one raw structured payload
/// per dense artifact resource binding. The adapter owns native resource
/// creation, descriptor encoding, command submission, fence completion and
/// output readback; the portable prepared scope remains the source of truth
/// for artifact identity, resource capacity and dispatch geometry.
pub fn run_prepared_uav_artifact(
    artifact: &SpirvArtifact,
    dxil: Vec<u8>,
    workgroups: [u32; 3],
    bindings: &[UavBindingPayload<'_>],
    output_binding: usize,
) -> Result<Vec<u8>, DirectX12Error> {
    let views = artifact
        .resources
        .iter()
        .map(|resource| {
            if resource.access.can_write() {
                DxilUavView::Structured
            } else {
                DxilUavView::StructuredSrv
            }
        })
        .collect::<Vec<_>>();
    run_prepared_uav_artifact_with_views(
        artifact,
        dxil,
        workgroups,
        bindings,
        output_binding,
        &views,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DxilUavView {
    /// Structured UAV with an explicit element stride.
    Structured,
    /// Raw UAV addressed in 32-bit words through `ByteAddressBuffer`.
    Raw,
    /// Structured SRV for a read-only resource.
    StructuredSrv,
    /// Raw SRV addressed in 32-bit words through `ByteAddressBuffer`.
    RawSrv,
}

fn dxil_view_is_writable(view: DxilUavView) -> bool {
    matches!(view, DxilUavView::Structured | DxilUavView::Raw)
}

fn structured_views_for_artifact(artifact: &SpirvArtifact) -> Vec<DxilUavView> {
    artifact
        .resources
        .iter()
        .map(|resource| {
            if resource.access.can_write() {
                DxilUavView::Structured
            } else {
                DxilUavView::StructuredSrv
            }
        })
        .collect()
}

fn run_prepared_uav_artifact_with_views(
    artifact: &SpirvArtifact,
    dxil: Vec<u8>,
    workgroups: [u32; 3],
    bindings: &[UavBindingPayload<'_>],
    output_binding: usize,
    views: &[DxilUavView],
) -> Result<Vec<u8>, DirectX12Error> {
    if views.len() != bindings.len() {
        return Err(DirectX12Error::ArtifactContract(
            "generic DX12 UAV view count differs from resource payload count",
        ));
    }
    validate_shared_artifact(artifact)?;
    let layouts = validate_dynamic_uav_request(
        artifact.resources.len(),
        workgroups,
        bindings,
        output_binding,
    )?;
    validate_dxil_native_views(artifact, views, output_binding)?;
    validate_artifact_uav_strides(artifact, bindings)?;
    validate_uav_alias_policy(artifact, bindings)?;
    let mut resource_table = ResourceTable::new();
    let mut resource_ids = Vec::with_capacity(layouts.len());
    for (index, layout) in layouts.iter().copied().enumerate() {
        let resource_id = resource_table
            .create_buffer(layout.byte_size)
            .map_err(|error| {
                DirectX12Error::PreparedDispatch(format!(
                    "create generic UAV resource {index}: {error}"
                ))
            })?;
        resource_table.make_resident(resource_id).map_err(|error| {
            DirectX12Error::PreparedDispatch(format!(
                "make generic UAV resource {index} resident: {error}"
            ))
        })?;
        resource_ids.push(resource_id);
    }
    let mut requests = Vec::with_capacity(resource_ids.len());
    for (binding, (buffer, required_bytes)) in resource_ids
        .iter()
        .copied()
        .zip(layouts.iter().map(|layout| layout.byte_size))
        .enumerate()
    {
        let binding = u32::try_from(binding).map_err(|_| {
            DirectX12Error::ArtifactContract("generic DX12 UAV binding index exceeds u32")
        })?;
        requests.push(ArtifactResourceRequest {
            binding,
            buffer,
            required_bytes,
        });
    }
    let prepared = prepare_artifact_dispatch(
        &mut resource_table,
        GpuBackend::DirectX12,
        BackendProbe {
            device_available: true,
            storage_buffers: true,
            global_invocation_id_x: true,
            structured_bounds: true,
            deterministic_f32: true,
            async_completion: true,
            shader_translation_available: true,
            max_workgroup_size: 1024,
        },
        ArtifactDispatchRequest {
            fp: FpPolicy::Strict,
            require_bounded_global_u32_array: false,
            require_async_completion: true,
        },
        DispatchGeometry::new(workgroups).map_err(|error| {
            DirectX12Error::PreparedDispatch(format!("dispatch geometry: {error}"))
        })?,
        &requests,
        artifact,
    )
    .map_err(|error| DirectX12Error::PreparedDispatch(error.to_string()))?;
    let context = match create_device_and_queue() {
        Ok(context) => context,
        Err(error) => {
            let _ = resource_table.release_prepared_artifact_dispatch(prepared);
            return Err(error);
        }
    };
    let descriptor = prepared.descriptor().clone();
    if let Err(error) = descriptor.validate_source_translation() {
        let _ = resource_table.release_prepared_artifact_dispatch(prepared);
        return Err(DirectX12Error::PreparedDispatch(error.to_string()));
    }
    let execution = run_dynamic_uav_dxil_with_context(
        &context,
        dxil,
        bindings,
        output_binding,
        descriptor.workgroups,
        views,
    );
    let residency_passed = resource_table
        .release_prepared_artifact_dispatch(prepared)
        .is_ok()
        && resource_ids
            .iter()
            .copied()
            .all(|resource_id| resource_table.evict(resource_id).is_ok());
    if !residency_passed {
        return Err(DirectX12Error::PreparedDispatch(
            "generic DX12 UAV prepared lease did not release cleanly".to_owned(),
        ));
    }
    execution
}

/// Translates a validated SPIR-V artifact through the configured
/// SPIRV-Cross→DXC toolchain and executes it through the generic dynamic UAV
/// lifecycle. The portable artifact/resource/alias checks run before any
/// native device or descriptor creation; unsupported toolchains and hosts are
/// reported explicitly rather than falling back to a different shader.
pub fn execute_spirv_artifact(
    artifact: &SpirvArtifact,
    workgroups: [u32; 3],
    bindings: &[UavBindingPayload<'_>],
    output_binding: usize,
) -> Result<Vec<u8>, DirectX12Error> {
    validate_shared_artifact(artifact)?;
    let translation = translate_spirv_artifact_to_dxil_report(artifact)?;
    run_prepared_uav_artifact_with_views(
        artifact,
        translation.dxil,
        workgroups,
        bindings,
        output_binding,
        &translation.resource_views,
    )
}

/// Executes an already audited HLSL source against an artifact resource
/// contract through the same native DX12 queue/fence/readback lifecycle.
///
/// This is intentionally a source-fixture boundary for backend regressions:
/// the caller owns the artifact metadata and source, while this function
/// validates their dense bindings/access classes before compiling DXIL or
/// creating any D3D12 resource. It is not a general SPIR-V translator.
pub fn execute_hlsl_source_artifact(
    artifact: &SpirvArtifact,
    source: &str,
    workgroups: [u32; 3],
    bindings: &[UavBindingPayload<'_>],
    output_binding: usize,
) -> Result<Vec<u8>, DirectX12Error> {
    validate_shared_artifact(artifact)?;
    validate_hlsl_resource_address_spaces(artifact)?;
    validate_external_hlsl_source(source, artifact)?;
    let dxil = compile_hlsl_source_with_entry(source, &artifact.entry_name)?;
    let views = hlsl_source_uav_views(source, artifact)?;
    run_prepared_uav_artifact_with_views(
        artifact,
        dxil,
        workgroups,
        bindings,
        output_binding,
        &views,
    )
}

/// Executes a raw SPIR-V source report after its HLSL/DXIL and native view
/// contracts have been revalidated.
///
/// The report is intentionally the only shader input: source identity,
/// resource capabilities and compiled DXIL must agree before a D3D12 device
/// is touched. This is the first native consumer for the raw source report;
/// it still requires caller-owned payloads and an explicit output binding.
pub fn execute_spirv_source_report(
    report: &DxilSourceTranslationReport,
    workgroups: [u32; 3],
    bindings: &[UavBindingPayload<'_>],
    output_binding: usize,
) -> Result<Vec<u8>, DirectX12Error> {
    execute_spirv_source_report_with_spec_map(
        report,
        workgroups,
        bindings,
        output_binding,
        &BTreeMap::new(),
    )
}

/// Executes a raw SPIR-V source report with caller-supplied specialization
/// values keyed by reflected `SpecId` decorations.
///
/// Literal `LocalSize` reports accept an empty map. `LocalSizeId` reports must
/// have a complete reflected `SpecId` triplet and receive all three values;
/// the map is consumed only for the workgroup geometry gate and does not
/// rewrite SPIR-V or claim specialized shader compilation.
pub fn execute_spirv_source_report_with_spec_map(
    report: &DxilSourceTranslationReport,
    workgroups: [u32; 3],
    bindings: &[UavBindingPayload<'_>],
    output_binding: usize,
    spec_values: &BTreeMap<u32, u32>,
) -> Result<Vec<u8>, DirectX12Error> {
    let source = &report.source;
    if source.identity.backend != ArtifactSourceBackend::Hlsl {
        return Err(DirectX12Error::SpirvTranslation(
            "raw DX12 source report must use the Hlsl backend".to_owned(),
        ));
    }
    if !valid_shader_entry(&source.identity.entry_name)
        || !contains_hlsl_entry(&source.source, &source.identity.entry_name)
    {
        return Err(DirectX12Error::SpirvTranslation(
            "raw DX12 source report has no valid HLSL entry".to_owned(),
        ));
    }
    if source.source_byte_count != source.source.len()
        || source.source_hash != stable_source_hash(&source.source)
    {
        return Err(DirectX12Error::SpirvTranslation(
            "raw DX12 source report source identity is inconsistent".to_owned(),
        ));
    }
    if !is_dxil_container(&report.dxil) {
        return Err(DirectX12Error::InvalidDxil);
    }
    let contract = SpirvRawModuleContract {
        entry_name: source.identity.entry_name.clone(),
        execution_model: source.identity.execution_model,
        workgroup_size: source.identity.workgroup_size,
        workgroup_size_ids: source.identity.workgroup_size_ids,
        workgroup_size_spec_ids: source.identity.workgroup_size_spec_ids,
        resources: source.identity.resources.clone(),
        word_count: source.identity.word_count,
        word_hash: source.identity.word_hash,
    };
    let plan = validate_spirv_raw_native_adapter(&contract)
        .map_err(|_| DirectX12Error::InvalidSpirv("raw-native-adapter-contract"))?;
    let output_binding_u32 = u32::try_from(output_binding)
        .map_err(|_| DirectX12Error::ArtifactContract("raw DX12 output binding exceeds u32"))?;
    let output_selection = select_spirv_raw_output_binding(&plan, output_binding_u32)
        .map_err(|_| DirectX12Error::ArtifactContract("raw DX12 output selection is invalid"))?;
    if plan
        .resolve_workgroup_size_from_spec_map(spec_values)
        .is_err()
    {
        return Err(DirectX12Error::InvalidSpirv(
            "raw source report requires resolved LocalSize metadata",
        ));
    }
    if report.resource_views.len() != plan.resources.len() || bindings.len() != plan.resources.len()
    {
        return Err(DirectX12Error::ArtifactContract(
            "raw DX12 source report resource count differs from payload/view count",
        ));
    }
    if output_selection.resource_index != output_binding {
        return Err(DirectX12Error::ArtifactContract(
            "raw DX12 output binding differs from the dense payload index",
        ));
    }
    for (index, (resource, view)) in plan
        .resources
        .iter()
        .zip(report.resource_views.iter().copied())
        .enumerate()
    {
        let access = resource.access.ok_or(DirectX12Error::InvalidSpirv(
            "raw resource access is unknown",
        ))?;
        let writable_view = dxil_view_is_writable(view);
        if access.can_write() != writable_view {
            return Err(DirectX12Error::ArtifactContract(
                "raw DX12 source report view does not match resource access",
            ));
        }
        let expected_stride = resource.element_stride.ok_or(DirectX12Error::InvalidSpirv(
            "raw resource stride is unknown",
        ))?;
        if bindings[index].element_stride != expected_stride {
            return Err(DirectX12Error::ArtifactContract(
                "raw DX12 source payload stride differs from reflected resource",
            ));
        }
    }
    if !dxil_view_is_writable(report.resource_views[output_selection.resource_index]) {
        return Err(DirectX12Error::ArtifactContract(
            "raw DX12 source report output must use a UAV view",
        ));
    }
    let _ =
        validate_dynamic_uav_request(plan.resources.len(), workgroups, bindings, output_binding)?;
    validate_raw_uav_alias_policy(&plan.resources, bindings)?;
    let context = create_device_and_queue()?;
    run_dynamic_uav_dxil_with_context(
        &context,
        report.dxil.clone(),
        bindings,
        output_binding,
        workgroups,
        &report.resource_views,
    )
}

/// Executes a raw DX12 source report after revalidating it against the exact
/// SPIR-V words that produced the report. This additive entry point closes the
/// word-count/hash/entry integrity bridge before the existing source/native
/// consumer; it still does not rewrite SPIR-V or compile a specialized shader.
pub fn execute_spirv_source_report_with_words(
    report: &DxilSourceTranslationReport,
    words: &[u32],
    workgroups: [u32; 3],
    bindings: &[UavBindingPayload<'_>],
    output_binding: usize,
    spec_values: &BTreeMap<u32, u32>,
) -> Result<Vec<u8>, DirectX12Error> {
    validate_spirv_source_report_words(&report.source, words, GpuBackend::DirectX12, spec_values)
        .map_err(map_raw_source_words_error)?;
    execute_spirv_source_report_with_spec_map(
        report,
        workgroups,
        bindings,
        output_binding,
        spec_values,
    )
}

/// Executes a generic UAV artifact using host `u32` values.
///
/// This compatibility wrapper preserves the original public API while the
/// native adapter itself uses [`UavBindingPayload`] and explicit structured
/// strides.
pub fn run_prepared_u32_uav_artifact(
    artifact: &SpirvArtifact,
    dxil: Vec<u8>,
    workgroups: [u32; 3],
    bindings: &[Vec<u32>],
    output_binding: usize,
) -> Result<Vec<u32>, DirectX12Error> {
    let byte_bindings = bindings
        .iter()
        .map(|values| {
            values
                .iter()
                .flat_map(|value| value.to_ne_bytes())
                .collect::<Vec<u8>>()
        })
        .collect::<Vec<_>>();
    let raw_bindings = byte_bindings
        .iter()
        .enumerate()
        .map(|(index, bytes)| UavBindingPayload {
            resource_id: u64::try_from(index).expect("payload binding count fits u64"),
            bytes,
            element_stride: std::mem::size_of::<u32>() as u32,
        })
        .collect::<Vec<_>>();
    let output =
        run_prepared_uav_artifact(artifact, dxil, workgroups, &raw_bindings, output_binding)?;
    if output.len() % std::mem::size_of::<u32>() != 0 {
        return Err(DirectX12Error::ArtifactContract(
            "generic DX12 u32 output byte length is not element-aligned",
        ));
    }
    Ok(output
        .chunks_exact(std::mem::size_of::<u32>())
        .map(|bytes| u32::from_ne_bytes(bytes.try_into().expect("u32 chunk has fixed width")))
        .collect())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DynamicUavLayout {
    byte_size: u64,
    element_count: u32,
    element_stride: u32,
}

fn validate_artifact_uav_strides(
    artifact: &SpirvArtifact,
    bindings: &[UavBindingPayload<'_>],
) -> Result<(), DirectX12Error> {
    if artifact.resources.len() != bindings.len() {
        return Err(DirectX12Error::ArtifactContract(
            "generic DX12 UAV payload count differs from artifact resources",
        ));
    }
    for (index, binding) in bindings.iter().enumerate() {
        let expected =
            artifact.resources[index]
                .element_stride
                .ok_or(DirectX12Error::ArtifactContract(
                    "generic DX12 artifact resource has no validated element stride",
                ))?;
        if binding.element_stride != expected {
            return Err(DirectX12Error::ArtifactContract(
                "generic DX12 UAV payload stride differs from artifact resource type",
            ));
        }
    }
    Ok(())
}

/// Validates the view/access boundary before any D3D12 resource or descriptor
/// is created. Writable bindings must use UAV views and the selected output
/// must be writable; read-only bindings use SRV views.
fn validate_dxil_native_views(
    artifact: &SpirvArtifact,
    views: &[DxilUavView],
    output_binding: usize,
) -> Result<(), DirectX12Error> {
    if artifact.resources.len() != views.len() {
        return Err(DirectX12Error::ArtifactContract(
            "generic DX12 UAV view count differs from artifact resources",
        ));
    }
    if output_binding >= artifact.resources.len() {
        return Err(DirectX12Error::ArtifactContract(
            "generic DX12 output binding is out of range while validating native views",
        ));
    }
    if !artifact.resources[output_binding].access.can_write() {
        return Err(DirectX12Error::ArtifactContract(
            "generic DX12 output binding must use read-write access",
        ));
    }
    for (index, resource) in artifact.resources.iter().enumerate() {
        let writable_view = matches!(views[index], DxilUavView::Structured | DxilUavView::Raw);
        let read_only_view = matches!(
            views[index],
            DxilUavView::StructuredSrv | DxilUavView::RawSrv
        );
        match resource.access {
            ResourceAccess::WriteOnly | ResourceAccess::ReadWrite if !writable_view => {
                return Err(DirectX12Error::ArtifactContract(
                    "generic DX12 read-write resource requires a UAV view",
                ));
            }
            ResourceAccess::ReadOnly if !read_only_view => {
                return Err(DirectX12Error::ArtifactContract(
                    "generic DX12 read-only resource requires an SRV view",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_uav_alias_policy(
    artifact: &SpirvArtifact,
    bindings: &[UavBindingPayload<'_>],
) -> Result<(), DirectX12Error> {
    if artifact.resources.len() != bindings.len() {
        return Err(DirectX12Error::ArtifactContract(
            "generic DX12 UAV payload count differs from artifact resources",
        ));
    }
    for (first_index, first) in bindings.iter().enumerate() {
        for (second_index, second) in bindings.iter().enumerate().skip(first_index + 1) {
            if first.resource_id != second.resource_id {
                continue;
            }
            let writable = matches!(
                artifact.resources[first_index].access,
                ResourceAccess::WriteOnly | ResourceAccess::ReadWrite
            ) || matches!(
                artifact.resources[second_index].access,
                ResourceAccess::WriteOnly | ResourceAccess::ReadWrite
            );
            if writable {
                return Err(DirectX12Error::ArtifactContract(
                    "generic DX12 writable UAV resource aliases are forbidden",
                ));
            }
            if first.element_stride != second.element_stride || first.bytes != second.bytes {
                return Err(DirectX12Error::ArtifactContract(
                    "generic DX12 read-only alias payloads must match",
                ));
            }
        }
    }
    Ok(())
}

fn validate_raw_uav_alias_policy(
    resources: &[jadren_gpu_runtime::SpirvRawResourceBinding],
    bindings: &[UavBindingPayload<'_>],
) -> Result<(), DirectX12Error> {
    if resources.len() != bindings.len() {
        return Err(DirectX12Error::ArtifactContract(
            "raw DX12 alias policy resource/payload count mismatch",
        ));
    }
    for (first_index, first) in bindings.iter().enumerate() {
        for (second_index, second) in bindings.iter().enumerate().skip(first_index + 1) {
            if first.resource_id != second.resource_id {
                continue;
            }
            let writable = matches!(
                resources[first_index].access,
                Some(ResourceAccess::WriteOnly | ResourceAccess::ReadWrite)
            ) || matches!(
                resources[second_index].access,
                Some(ResourceAccess::WriteOnly | ResourceAccess::ReadWrite)
            );
            if writable {
                return Err(DirectX12Error::ArtifactContract(
                    "raw DX12 writable resource aliases are forbidden",
                ));
            }
            if first.element_stride != second.element_stride || first.bytes != second.bytes {
                return Err(DirectX12Error::ArtifactContract(
                    "raw DX12 read-only aliases must match payload shape",
                ));
            }
        }
    }
    Ok(())
}

fn validate_dynamic_uav_request(
    artifact_binding_count: usize,
    workgroups: [u32; 3],
    bindings: &[UavBindingPayload<'_>],
    output_binding: usize,
) -> Result<Vec<DynamicUavLayout>, DirectX12Error> {
    if artifact_binding_count != bindings.len() || bindings.is_empty() {
        return Err(DirectX12Error::ArtifactContract(
            "generic DX12 UAV dispatch requires one payload per non-empty artifact resource",
        ));
    }
    if output_binding >= bindings.len() {
        return Err(DirectX12Error::ArtifactContract(
            "generic DX12 UAV dispatch output binding is out of range",
        ));
    }
    if workgroups.contains(&0) {
        return Err(DirectX12Error::ArtifactContract(
            "generic DX12 UAV dispatch requires non-zero workgroup dimensions",
        ));
    }
    u32::try_from(bindings.len()).map_err(|_| {
        DirectX12Error::ArtifactContract("generic DX12 UAV binding count exceeds u32")
    })?;
    let mut layouts = Vec::with_capacity(bindings.len());
    for binding in bindings {
        if binding.bytes.is_empty() {
            return Err(DirectX12Error::ArtifactContract(
                "generic DX12 UAV dispatch rejects empty resource payloads",
            ));
        }
        if binding.element_stride == 0 || binding.element_stride > 2048 {
            return Err(DirectX12Error::ArtifactContract(
                "generic DX12 UAV element stride must be between 1 and 2048 bytes",
            ));
        }
        if !binding
            .bytes
            .len()
            .is_multiple_of(binding.element_stride as usize)
        {
            return Err(DirectX12Error::ArtifactContract(
                "generic DX12 UAV payload bytes must align to element stride",
            ));
        }
        let element_count = binding.bytes.len() / binding.element_stride as usize;
        let bytes = u64::try_from(binding.bytes.len()).map_err(|_| {
            DirectX12Error::ArtifactContract("generic DX12 UAV resource byte size overflows")
        })?;
        if element_count > u32::MAX as usize {
            return Err(DirectX12Error::ArtifactContract(
                "generic DX12 UAV resource element count exceeds u32",
            ));
        }
        layouts.push(DynamicUavLayout {
            byte_size: bytes,
            element_count: u32::try_from(element_count).expect("checked element count fits u32"),
            element_stride: binding.element_stride,
        });
    }
    Ok(layouts)
}

/// Host tools required to translate a Jadren SPIR-V artifact to DXIL.
///
/// The paths are explicit so a build can pin a known SPIRV-Cross and DXC
/// pair instead of relying on whichever compiler happens to be first on PATH.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpirvDxilToolchain {
    /// SPIRV-Cross executable used for SPIR-V→HLSL lowering.
    pub spirv_cross: PathBuf,
    /// DXC executable used for HLSL→DXIL compilation.
    pub dxc: PathBuf,
}

/// Source audit and compiled DXIL kept together for one artifact translation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DxilArtifactTranslationReport {
    /// HLSL source audit that produced the DXIL payload.
    pub source: ArtifactSourceTranslationReport,
    /// Validated DXIL container bytes compiled from the audited source.
    pub dxil: Vec<u8>,
    /// Per-binding UAV view policy derived from the audited HLSL declarations.
    pub resource_views: Vec<DxilUavView>,
}

/// Source audit and compiled DXIL kept together for one raw SPIR-V
/// translation.
///
/// This is intentionally independent of [`SpirvArtifact`].  It proves the
/// external SPIRV-Cross/DXC hand-off and the conservative raw storage-buffer
/// view policy, but it does not contain workgroup metadata or claim that the
/// resulting module can be submitted to the native executor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DxilSourceTranslationReport {
    /// Raw SPIR-V source audit used as the compilation input.
    pub source: SpirvSourceTranslationReport,
    /// Validated DXIL container bytes compiled from the audited HLSL.
    pub dxil: Vec<u8>,
    /// Per-binding view policy derived from the audited raw resource contract.
    pub resource_views: Vec<DxilUavView>,
}

impl SpirvDxilToolchain {
    /// Discovers a complete toolchain from explicit environment variables or PATH.
    #[must_use]
    pub fn discover() -> Option<Self> {
        Some(Self {
            spirv_cross: locate_spirv_cross()?,
            dxc: locate_dxc()?,
        })
    }
}

/// Structured native probe error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DirectX12Error {
    /// d3d12.dll could not be loaded or the export was missing.
    Loader(String),
    /// D3D12 returned a failing HRESULT.
    HResult { operation: &'static str, code: i32 },
    /// A native API returned a null COM object despite success.
    NullObject(&'static str),
    /// The queue did not complete its fence signal before the bounded poll.
    CompletionTimeout,
    /// A usable DXC executable was not found.
    ShaderToolchainUnavailable,
    /// A complete SPIR-V→HLSL→DXIL toolchain was not found.
    SpirvToolchainUnavailable,
    /// The SPIRV-Cross source translator was not found.
    SpirvCrossUnavailable,
    /// The input is not a minimally valid SPIR-V binary.
    InvalidSpirv(&'static str),
    /// The requested shader entry name is not a safe identifier.
    InvalidShaderEntry,
    /// SPIRV-Cross or DXC failed during a general translation.
    SpirvTranslation(String),
    /// The JIR module could not be lowered into the portable SPIR-V subset.
    JirSpirvLowering(String),
    /// The internal artifact fallback received a shape outside its narrow ABI.
    ArtifactContract(&'static str),
    /// The shared prepared-dispatch scope could not be created or released.
    PreparedDispatch(String),
    /// DXC failed to compile the verified HLSL fixture.
    ShaderCompilation(String),
    /// The compiler output was not a DXIL container.
    InvalidDxil,
    /// A GPU readback did not match the add-one reference.
    DifferentialMismatch {
        index: u32,
        actual: u32,
        expected: u32,
    },
    /// The requested constant is invalid for the selected unsigned operation.
    InvalidBinaryOperand(&'static str),
}

impl fmt::Display for DirectX12Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Loader(message) => write!(formatter, "DirectX 12 loader unavailable: {message}"),
            Self::HResult { operation, code } => {
                write!(
                    formatter,
                    "DirectX 12 {operation} failed: 0x{:08x}",
                    *code as u32
                )
            }
            Self::NullObject(operation) => {
                write!(formatter, "DirectX 12 {operation} returned a null object")
            }
            Self::CompletionTimeout => formatter
                .write_str("DirectX 12 queue fence did not complete within the smoke timeout"),
            Self::ShaderToolchainUnavailable => {
                formatter.write_str("DirectX 12 DXC executable was not found")
            }
            Self::SpirvToolchainUnavailable => {
                formatter.write_str("SPIR-V to DXIL toolchain was not found")
            }
            Self::SpirvCrossUnavailable => {
                formatter.write_str("SPIRV-Cross source translator was not found")
            }
            Self::InvalidSpirv(reason) => write!(formatter, "invalid SPIR-V binary: {reason}"),
            Self::InvalidShaderEntry => {
                formatter.write_str("shader entry name must be a non-empty ASCII identifier")
            }
            Self::SpirvTranslation(message) => {
                write!(formatter, "SPIR-V to DXIL translation failed: {message}")
            }
            Self::JirSpirvLowering(message) => {
                write!(formatter, "JIR to SPIR-V lowering failed: {message}")
            }
            Self::ArtifactContract(reason) => {
                write!(formatter, "unsupported DX12 artifact contract: {reason}")
            }
            Self::PreparedDispatch(message) => {
                write!(
                    formatter,
                    "DX12 prepared artifact dispatch failed: {message}"
                )
            }
            Self::ShaderCompilation(message) => {
                write!(formatter, "DirectX 12 DXC compilation failed: {message}")
            }
            Self::InvalidDxil => formatter.write_str("DirectX 12 compiler output is not DXIL"),
            Self::DifferentialMismatch {
                index,
                actual,
                expected,
            } => write!(
                formatter,
                "DirectX 12 output mismatch at {index}: actual={actual}, expected={expected}"
            ),
            Self::InvalidBinaryOperand(reason) => {
                write!(formatter, "invalid DirectX 12 u32 binary operand: {reason}")
            }
        }
    }
}

impl Error for DirectX12Error {}

/// Runs a real device and direct command-queue creation probe.
pub fn run_device_smoke() -> Result<DirectX12DeviceSmokeReport, DirectX12Error> {
    let library = unsafe { Library::new("d3d12.dll") }
        .map_err(|error| DirectX12Error::Loader(error.to_string()))?;
    let create_device: Symbol<'_, D3D12CreateDevice> = unsafe {
        library
            .get(b"D3D12CreateDevice\0")
            .map_err(|error| DirectX12Error::Loader(error.to_string()))?
    };

    let mut device = std::ptr::null_mut();
    let code = unsafe {
        create_device(
            std::ptr::null_mut(),
            D3D_FEATURE_LEVEL_11_0,
            &IID_ID3D12_DEVICE,
            &mut device,
        )
    };
    if code < S_OK {
        return Err(DirectX12Error::HResult {
            operation: "D3D12CreateDevice",
            code,
        });
    }
    if device.is_null() {
        return Err(DirectX12Error::NullObject("D3D12CreateDevice"));
    }
    let device = ComObject::new(device, "ID3D12Device");
    let node_count = unsafe { vtable_method::<GetNodeCount>(device.as_ptr(), 7)(device.as_ptr()) };

    let queue_desc = CommandQueueDesc {
        queue_type: D3D12_COMMAND_LIST_TYPE_DIRECT,
        priority: 0,
        flags: D3D12_COMMAND_QUEUE_FLAG_NONE,
        node_mask: 0,
    };
    let mut queue = std::ptr::null_mut();
    let code = unsafe {
        vtable_method::<CreateCommandQueue>(device.as_ptr(), 8)(
            device.as_ptr(),
            &queue_desc,
            &IID_ID3D12_COMMAND_QUEUE,
            &mut queue,
        )
    };
    if code < S_OK {
        return Err(DirectX12Error::HResult {
            operation: "ID3D12Device::CreateCommandQueue",
            code,
        });
    }
    if queue.is_null() {
        return Err(DirectX12Error::NullObject("CreateCommandQueue"));
    }
    let queue = ComObject::new(queue, "ID3D12CommandQueue");

    let mut allocator = std::ptr::null_mut();
    let code = unsafe {
        vtable_method::<CreateCommandAllocator>(device.as_ptr(), 9)(
            device.as_ptr(),
            D3D12_COMMAND_LIST_TYPE_DIRECT,
            &IID_ID3D12_COMMAND_ALLOCATOR,
            &mut allocator,
        )
    };
    if code < S_OK {
        return Err(DirectX12Error::HResult {
            operation: "ID3D12Device::CreateCommandAllocator",
            code,
        });
    }
    if allocator.is_null() {
        return Err(DirectX12Error::NullObject("CreateCommandAllocator"));
    }
    let allocator = ComObject::new(allocator, "ID3D12CommandAllocator");

    let mut command_list = std::ptr::null_mut();
    let code = unsafe {
        vtable_method::<CreateCommandList>(device.as_ptr(), 12)(
            device.as_ptr(),
            0,
            D3D12_COMMAND_LIST_TYPE_DIRECT,
            allocator.as_ptr(),
            std::ptr::null_mut(),
            &IID_ID3D12_GRAPHICS_COMMAND_LIST,
            &mut command_list,
        )
    };
    if code < S_OK {
        return Err(DirectX12Error::HResult {
            operation: "ID3D12Device::CreateCommandList",
            code,
        });
    }
    if command_list.is_null() {
        return Err(DirectX12Error::NullObject("CreateCommandList"));
    }
    let command_list = ComObject::new(command_list, "ID3D12GraphicsCommandList");
    let code = unsafe {
        vtable_method::<CloseCommandList>(command_list.as_ptr(), 9)(command_list.as_ptr())
    };
    if code < S_OK {
        return Err(DirectX12Error::HResult {
            operation: "ID3D12GraphicsCommandList::Close",
            code,
        });
    }
    let command_list_pointer = command_list.as_ptr();
    unsafe {
        vtable_method::<ExecuteCommandLists>(queue.as_ptr(), 10)(
            queue.as_ptr(),
            1,
            &command_list_pointer,
        );
    }

    let mut fence = std::ptr::null_mut();
    let code = unsafe {
        vtable_method::<CreateFence>(device.as_ptr(), 36)(
            device.as_ptr(),
            0,
            D3D12_FENCE_FLAG_NONE,
            &IID_ID3D12_FENCE,
            &mut fence,
        )
    };
    if code < S_OK {
        return Err(DirectX12Error::HResult {
            operation: "ID3D12Device::CreateFence",
            code,
        });
    }
    if fence.is_null() {
        return Err(DirectX12Error::NullObject("CreateFence"));
    }
    let fence = ComObject::new(fence, "ID3D12Fence");
    let target_value = 1_u64;
    let code = unsafe {
        vtable_method::<QueueSignal>(queue.as_ptr(), 14)(
            queue.as_ptr(),
            fence.as_ptr(),
            target_value,
        )
    };
    if code < S_OK {
        return Err(DirectX12Error::HResult {
            operation: "ID3D12CommandQueue::Signal",
            code,
        });
    }
    let get_completed_value = unsafe { vtable_method::<GetCompletedValue>(fence.as_ptr(), 8) };
    let mut fence_completed = false;
    for _ in 0..500 {
        if unsafe { get_completed_value(fence.as_ptr()) } >= target_value {
            fence_completed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    if !fence_completed {
        return Err(DirectX12Error::CompletionTimeout);
    }

    Ok(DirectX12DeviceSmokeReport {
        schema: "jadren-directx12-device-smoke-0.1",
        backend: "directx12",
        api_loaded: true,
        device_created: true,
        compute_queue_created: true,
        command_list_recorded: true,
        command_list_submitted: true,
        fence_signaled: true,
        fence_completed: true,
        node_count: Some(node_count),
        minimum_feature_level: "11_0",
        shader_translation: "not-run-dxil-toolchain-required",
        result: "pass-device-queue-fence",
        completion_model: "native-fence-poll",
        error: None,
    })
}

/// Runs the verified three-UAV `u32 + 1` compute fixture through DXC and D3D12.
pub fn run_compute_smoke() -> Result<DirectX12ComputeSmokeReport, DirectX12Error> {
    let report = run_binary_smoke(BinaryOp::Add, 1)?;
    Ok(DirectX12ComputeSmokeReport {
        schema: "jadren-directx12-compute-smoke-0.1",
        length: report.length,
        first_output: report.first_output,
        last_output: report.last_output,
        output_checksum: report.output_checksum,
        dxil_validated: report.dxil_validated,
        pipeline_created: report.pipeline_created,
        execution_completed: report.execution_completed,
        shader_translation: report.shader_translation,
        result: report.result,
    })
}

/// Compiles and executes one runtime-length `u32` BinaryOp through DXC and
/// the native D3D12 queue. Input, output and runtime length are explicit UAV
/// resources, matching the portable three-resource contract.
pub fn run_binary_smoke(
    operation: BinaryOp,
    operand: u32,
) -> Result<DirectX12BinarySmokeReport, DirectX12Error> {
    run_binary_smoke_with_input(operation, operand, 41, 1)
}

/// Executes one runtime-length BinaryOp with an explicit input sequence start.
/// The parameter is used by cross-backend differential smoke so DX12 and
/// Vulkan consume byte-identical logical inputs.
pub fn run_binary_smoke_with_input(
    operation: BinaryOp,
    operand: u32,
    input_start: u32,
    input_stride: u32,
) -> Result<DirectX12BinarySmokeReport, DirectX12Error> {
    let (dxil, operation_name) = compile_binary_dxil(operation, operand)?;
    run_binary_dxil_with_input(
        dxil,
        operation_name,
        operation,
        operand,
        input_start,
        input_stride,
        "dxc-hlsl-to-dxil",
    )
}

fn run_binary_dxil_with_input(
    dxil: Vec<u8>,
    operation_name: &'static str,
    operation: BinaryOp,
    operand: u32,
    input_start: u32,
    input_stride: u32,
    shader_translation: &'static str,
) -> Result<DirectX12BinarySmokeReport, DirectX12Error> {
    let length = 70_usize;
    let input_values: Vec<u32> = (0..length)
        .map(|index| {
            input_start.wrapping_add(
                u32::try_from(index)
                    .expect("binary fixture index fits u32")
                    .wrapping_mul(input_stride),
            )
        })
        .collect();
    let expected_values: Vec<u32> = input_values
        .iter()
        .map(|value| apply_binary(*value, operation, operand))
        .collect();
    run_binary_dxil_with_values(
        dxil,
        operation_name,
        operation,
        operand,
        input_start,
        input_stride,
        shader_translation,
        &input_values,
        &expected_values,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_binary_dxil_with_values(
    dxil: Vec<u8>,
    operation_name: &'static str,
    _operation: BinaryOp,
    operand: u32,
    input_start: u32,
    input_stride: u32,
    shader_translation: &'static str,
    input_values: &[u32],
    expected_values: &[u32],
) -> Result<DirectX12BinarySmokeReport, DirectX12Error> {
    if input_values.is_empty() || input_values.len() != expected_values.len() {
        return Err(DirectX12Error::ArtifactContract(
            "three-UAV execution requires equally sized non-empty input/oracle arrays",
        ));
    }
    let length = u32::try_from(input_values.len()).map_err(|_| {
        DirectX12Error::ArtifactContract("three-UAV execution input length exceeds u32")
    })?;
    let context = create_device_and_queue()?;
    run_binary_dxil_with_context(
        &context,
        dxil,
        operation_name,
        _operation,
        operand,
        input_start,
        input_stride,
        shader_translation,
        [length.div_ceil(64), 1, 1],
        input_values,
        expected_values,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_binary_dxil_with_context(
    context: &DxContext,
    dxil: Vec<u8>,
    operation_name: &'static str,
    _operation: BinaryOp,
    operand: u32,
    input_start: u32,
    input_stride: u32,
    shader_translation: &'static str,
    dispatch_workgroups: [u32; 3],
    input_values: &[u32],
    expected_values: &[u32],
) -> Result<DirectX12BinarySmokeReport, DirectX12Error> {
    if input_values.is_empty() || input_values.len() != expected_values.len() {
        return Err(DirectX12Error::ArtifactContract(
            "three-UAV execution requires equally sized non-empty input/oracle arrays",
        ));
    }
    if dispatch_workgroups.contains(&0) {
        return Err(DirectX12Error::ArtifactContract(
            "three-UAV execution requires non-zero dispatch dimensions",
        ));
    }
    let length = u32::try_from(input_values.len()).map_err(|_| {
        DirectX12Error::ArtifactContract("three-UAV execution input length exceeds u32")
    })?;
    let bytes = usize::try_from(length)
        .expect("u32 length fits usize")
        .checked_mul(std::mem::size_of::<u32>())
        .expect("fixture byte size does not overflow");

    let default_heap = HeapProperties {
        heap_type: D3D12_HEAP_TYPE_DEFAULT,
        cpu_page_property: 0,
        memory_pool_preference: 0,
        creation_node_mask: 1,
        visible_node_mask: 1,
    };
    let upload_heap = HeapProperties {
        heap_type: D3D12_HEAP_TYPE_UPLOAD,
        ..default_heap
    };
    let readback_heap = HeapProperties {
        heap_type: D3D12_HEAP_TYPE_READBACK,
        ..default_heap
    };
    let default_desc = buffer_desc(bytes, D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS);
    let upload_desc = buffer_desc(bytes, 0);
    let readback_desc = buffer_desc(bytes, 0);
    let length_bytes = std::mem::size_of::<u32>();
    let length_upload_desc = buffer_desc(length_bytes, 0);
    let length_default_desc = buffer_desc(length_bytes, D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS);
    let input_upload = create_resource(
        context.device.as_ptr(),
        &upload_heap,
        &upload_desc,
        D3D12_RESOURCE_STATE_GENERIC_READ,
        "CreateCommittedResource(input_upload)",
    )?;
    write_resource(context.device.as_ptr(), input_upload.as_ptr(), input_values)?;
    let input = create_resource(
        context.device.as_ptr(),
        &default_heap,
        &default_desc,
        D3D12_RESOURCE_STATE_COPY_DEST,
        "CreateCommittedResource(input)",
    )?;
    let output = create_resource(
        context.device.as_ptr(),
        &default_heap,
        &default_desc,
        D3D12_RESOURCE_STATE_COPY_DEST,
        "CreateCommittedResource(output)",
    )?;
    let length_upload = create_resource(
        context.device.as_ptr(),
        &upload_heap,
        &length_upload_desc,
        D3D12_RESOURCE_STATE_GENERIC_READ,
        "CreateCommittedResource(length_upload)",
    )?;
    write_resource(context.device.as_ptr(), length_upload.as_ptr(), &[length])?;
    let length_resource = create_resource(
        context.device.as_ptr(),
        &default_heap,
        &length_default_desc,
        D3D12_RESOURCE_STATE_COPY_DEST,
        "CreateCommittedResource(length)",
    )?;
    let readback = create_resource(
        context.device.as_ptr(),
        &readback_heap,
        &readback_desc,
        D3D12_RESOURCE_STATE_COPY_DEST,
        "CreateCommittedResource(readback)",
    )?;
    let descriptor_heap = create_descriptor_heap(context.device.as_ptr(), 3)?;
    let descriptor_increment = unsafe {
        vtable_method::<GetDescriptorHandleIncrementSize>(context.device.as_ptr(), 15)(
            context.device.as_ptr(),
            D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
        )
    };
    if descriptor_increment == 0 {
        return Err(DirectX12Error::HResult {
            operation: "ID3D12Device::GetDescriptorHandleIncrementSize",
            code: -1,
        });
    }
    let mut cpu_start = CpuDescriptorHandle { ptr: 0 };
    unsafe {
        vtable_method::<GetCpuDescriptorHandleForHeapStart>(descriptor_heap.as_ptr(), 9)(
            descriptor_heap.as_ptr(),
            &mut cpu_start,
        )
    };
    let mut gpu_start = GpuDescriptorHandle { ptr: 0 };
    unsafe {
        vtable_method::<GetGpuDescriptorHandleForHeapStart>(descriptor_heap.as_ptr(), 10)(
            descriptor_heap.as_ptr(),
            &mut gpu_start,
        )
    };
    let uav_desc = UnorderedAccessViewDesc {
        format: DXGI_FORMAT_UNKNOWN,
        view_dimension: D3D12_UAV_DIMENSION_BUFFER,
        data: UnorderedAccessViewData {
            buffer: UnorderedAccessViewBuffer {
                first_element: 0,
                num_elements: length,
                structure_byte_stride: std::mem::size_of::<u32>() as u32,
                counter_offset_in_bytes: 0,
                flags: 0,
            },
        },
    };
    let length_uav_desc = UnorderedAccessViewDesc {
        format: DXGI_FORMAT_UNKNOWN,
        view_dimension: D3D12_UAV_DIMENSION_BUFFER,
        data: UnorderedAccessViewData {
            buffer: UnorderedAccessViewBuffer {
                first_element: 0,
                num_elements: 1,
                structure_byte_stride: std::mem::size_of::<u32>() as u32,
                counter_offset_in_bytes: 0,
                flags: 0,
            },
        },
    };
    unsafe {
        vtable_method::<CreateUnorderedAccessView>(context.device.as_ptr(), 19)(
            context.device.as_ptr(),
            input.as_ptr(),
            std::ptr::null_mut(),
            &uav_desc,
            cpu_start,
        );
        vtable_method::<CreateUnorderedAccessView>(context.device.as_ptr(), 19)(
            context.device.as_ptr(),
            output.as_ptr(),
            std::ptr::null_mut(),
            &uav_desc,
            CpuDescriptorHandle {
                ptr: cpu_start.ptr + descriptor_increment as usize,
            },
        );
        vtable_method::<CreateUnorderedAccessView>(context.device.as_ptr(), 19)(
            context.device.as_ptr(),
            length_resource.as_ptr(),
            std::ptr::null_mut(),
            &length_uav_desc,
            CpuDescriptorHandle {
                ptr: cpu_start.ptr + descriptor_increment as usize * 2,
            },
        );
    }
    let root_signature = create_uav_root_signature(&context._library, context.device.as_ptr(), 3)?;
    let pipeline =
        create_compute_pipeline(context.device.as_ptr(), &dxil, root_signature.as_ptr())?;
    let allocator = create_command_allocator(context.device.as_ptr())?;
    let command_list = create_command_list(context.device.as_ptr(), allocator.as_ptr())?;
    unsafe {
        vtable_method::<CopyBufferRegion>(command_list.as_ptr(), 15)(
            command_list.as_ptr(),
            input.as_ptr(),
            0,
            input_upload.as_ptr(),
            0,
            bytes as u64,
        );
        vtable_method::<CopyBufferRegion>(command_list.as_ptr(), 15)(
            command_list.as_ptr(),
            length_resource.as_ptr(),
            0,
            length_upload.as_ptr(),
            0,
            length_bytes as u64,
        );
    }
    let to_uav = [
        ResourceBarrier {
            barrier_type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            transition: ResourceTransition {
                resource: input.as_ptr(),
                subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                state_before: D3D12_RESOURCE_STATE_COPY_DEST,
                state_after: D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            },
        },
        ResourceBarrier {
            barrier_type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            transition: ResourceTransition {
                resource: output.as_ptr(),
                subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                state_before: D3D12_RESOURCE_STATE_COPY_DEST,
                state_after: D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            },
        },
        ResourceBarrier {
            barrier_type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            transition: ResourceTransition {
                resource: length_resource.as_ptr(),
                subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                state_before: D3D12_RESOURCE_STATE_COPY_DEST,
                state_after: D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            },
        },
    ];
    unsafe {
        vtable_method::<ResourceBarrierFn>(command_list.as_ptr(), 26)(
            command_list.as_ptr(),
            to_uav.len() as u32,
            to_uav.as_ptr(),
        );
        let heaps = [descriptor_heap.as_ptr()];
        vtable_method::<SetDescriptorHeaps>(command_list.as_ptr(), 28)(
            command_list.as_ptr(),
            heaps.len() as u32,
            heaps.as_ptr(),
        );
        vtable_method::<SetPipelineState>(command_list.as_ptr(), 25)(
            command_list.as_ptr(),
            pipeline.as_ptr(),
        );
        vtable_method::<SetComputeRootSignature>(command_list.as_ptr(), 29)(
            command_list.as_ptr(),
            root_signature.as_ptr(),
        );
        vtable_method::<SetComputeRootDescriptorTable>(command_list.as_ptr(), 31)(
            command_list.as_ptr(),
            0,
            gpu_start,
        );
        vtable_method::<Dispatch>(command_list.as_ptr(), 14)(
            command_list.as_ptr(),
            dispatch_workgroups[0],
            dispatch_workgroups[1],
            dispatch_workgroups[2],
        );
        let to_copy = [ResourceBarrier {
            barrier_type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            transition: ResourceTransition {
                resource: output.as_ptr(),
                subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                state_before: D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                state_after: D3D12_RESOURCE_STATE_COPY_SOURCE,
            },
        }];
        vtable_method::<ResourceBarrierFn>(command_list.as_ptr(), 26)(
            command_list.as_ptr(),
            1,
            to_copy.as_ptr(),
        );
        vtable_method::<CopyBufferRegion>(command_list.as_ptr(), 15)(
            command_list.as_ptr(),
            readback.as_ptr(),
            0,
            output.as_ptr(),
            0,
            bytes as u64,
        );
        let code =
            vtable_method::<CloseCommandList>(command_list.as_ptr(), 9)(command_list.as_ptr());
        if code < S_OK {
            return Err(DirectX12Error::HResult {
                operation: "ID3D12GraphicsCommandList::Close(compute)",
                code,
            });
        }
        let command_list_pointer = command_list.as_ptr();
        vtable_method::<ExecuteCommandLists>(context.queue.as_ptr(), 10)(
            context.queue.as_ptr(),
            1,
            &command_list_pointer,
        );
    }
    let fence = create_fence(context.device.as_ptr())?;
    signal_and_wait(context.queue.as_ptr(), fence.as_ptr(), 1)?;
    let actual_values = read_resource(context.device.as_ptr(), readback.as_ptr(), length as usize)?;
    for (index, (actual, expected)) in actual_values.iter().zip(expected_values.iter()).enumerate()
    {
        if actual != expected {
            return Err(DirectX12Error::DifferentialMismatch {
                index: index as u32,
                actual: *actual,
                expected: *expected,
            });
        }
    }
    let output_checksum = actual_values.iter().map(|value| u64::from(*value)).sum();
    Ok(DirectX12BinarySmokeReport {
        schema: "jadren-directx12-binary-smoke-0.1",
        operation: operation_name,
        operand,
        input_start,
        input_stride,
        length,
        resource_binding_count: 3,
        first_output: actual_values[0],
        last_output: actual_values[length as usize - 1],
        output_checksum,
        dxil_validated: true,
        pipeline_created: true,
        execution_completed: true,
        shader_translation,
        result: "pass-compute-differential",
    })
}

/// Executes a runtime-length BinaryOp produced from a verified JIR entry.
/// The preferred path is SPIRV-Cross→DXC. When that external pair is absent,
/// the narrow, structurally decoded three-resource artifact family uses the
/// explicit internal HLSL fallback; arbitrary SPIR-V is never accepted by it.
pub fn run_binary_artifact_smoke(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    operation: BinaryOp,
    operand: u32,
    input_start: u32,
    input_stride: u32,
) -> Result<DirectX12ArtifactExecutionReport, DirectX12Error> {
    let artifact = emit_storage_global_index_binary_dynamic_length_artifact_from_jir(
        module, function, options, operation,
    )
    .map_err(|error| DirectX12Error::JirSpirvLowering(error.to_string()))?;
    validate_shared_artifact(&artifact)?;
    validate_binary_artifact_contract(&artifact, operation, operand)?;
    if artifact.resources.len() != 3 {
        return Err(DirectX12Error::JirSpirvLowering(
            "runtime-length BinaryOp artifact must expose three resources".to_owned(),
        ));
    }
    let (dxil, translation_path) = match translate_spirv_artifact_to_dxil(&artifact) {
        Ok(dxil) => (dxil, "spirv-cross-dxc"),
        Err(
            DirectX12Error::SpirvToolchainUnavailable
            | DirectX12Error::SpirvCrossUnavailable
            | DirectX12Error::ShaderToolchainUnavailable,
        ) => (
            compile_binary_artifact_dxil(&artifact, operation, operand)?,
            "jadren-artifact-narrow-hlsl-to-dxil",
        ),
        Err(error) => return Err(error),
    };
    let input_values: Vec<u32> = (0..70_u32)
        .map(|index| input_start.wrapping_add(index.wrapping_mul(input_stride)))
        .collect();
    let expected_values: Vec<u32> = input_values
        .iter()
        .map(|value| apply_binary(*value, operation, operand))
        .collect();
    let bindings = vec![input_values, vec![0_u32; 70], vec![70]];
    let actual_values = run_prepared_u32_uav_artifact(&artifact, dxil, [2, 1, 1], &bindings, 1)?;
    for (index, (actual, expected)) in actual_values.iter().zip(expected_values.iter()).enumerate()
    {
        if actual != expected {
            return Err(DirectX12Error::DifferentialMismatch {
                index: index as u32,
                actual: *actual,
                expected: *expected,
            });
        }
    }
    let output_checksum = actual_values.iter().map(|value| u64::from(*value)).sum();
    Ok(DirectX12ArtifactExecutionReport {
        schema: "jadren-directx12-artifact-execution-0.1",
        entry_name: artifact.entry_name,
        artifact_resource_binding_count: artifact.resources.len(),
        artifact_word_count: artifact.words.len(),
        artifact_word_hash: stable_spirv_word_hash(&artifact.words),
        operation: binary_name(operation),
        artifact_validated: true,
        dxil_translated: true,
        execution_completed: true,
        output_checksum,
        first_output: actual_values.first().copied(),
        last_output: actual_values.last().copied(),
        logical_length: None,
        capacity: None,
        untouched_tail_count: None,
        translation_path,
        result: "pass-artifact-compute-differential",
    })
}

/// Executes the verified runtime-length `f32` add artifact through DX12.
///
/// The artifact is first validated structurally and against its encoded
/// `OpFAdd`/constant contract. The fallback shader stores IEEE-754 values as
/// `uint` bits so the existing descriptor/readback executor can compare exact
/// results without introducing a second ownership path.
pub fn run_f32_artifact_smoke(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    addend: f32,
) -> Result<DirectX12F32ArtifactExecutionReport, DirectX12Error> {
    run_f32_binary_artifact_smoke(module, function, options, addend, F32ArithmeticOp::Add)
}

/// Executes the verified runtime-length scalar `f32` binary artifact through
/// DX12. The input/output buffers remain bit-preserving `uint` UAVs while the
/// shader performs the selected IEEE-754 operation.
pub fn run_f32_binary_artifact_smoke(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    operand: f32,
    operation: F32ArithmeticOp,
) -> Result<DirectX12F32ArtifactExecutionReport, DirectX12Error> {
    let artifact = emit_storage_global_index_f32_binary_dynamic_length_artifact_from_jir(
        module, function, options, operation,
    )
    .map_err(|error| DirectX12Error::JirSpirvLowering(error.to_string()))?;
    validate_shared_artifact(&artifact)?;
    let operand_bits = operand.to_bits();
    validate_f32_binary_artifact_contract(&artifact, operand_bits, operation)?;
    if artifact.resources.len() != 3 {
        return Err(DirectX12Error::ArtifactContract(
            "runtime-length f32 artifact requires exactly three resources",
        ));
    }
    if artifact.workgroup_size != [64, 1, 1] {
        return Err(DirectX12Error::ArtifactContract(
            "runtime-length f32 DX12 executor currently requires a 64x1x1 workgroup",
        ));
    }
    let (dxil, translation_path) = match translate_spirv_artifact_to_dxil(&artifact) {
        Ok(dxil) => (dxil, "spirv-cross-dxc"),
        Err(
            DirectX12Error::SpirvToolchainUnavailable
            | DirectX12Error::SpirvCrossUnavailable
            | DirectX12Error::ShaderToolchainUnavailable,
        ) => (
            compile_f32_artifact_dxil_for_operation(&artifact, operand_bits, operation)?,
            "jadren-artifact-f32-hlsl-to-dxil",
        ),
        Err(error) => return Err(error),
    };
    let input_values: Vec<f32> = (0..70_u32)
        .map(|index| 7.0_f32 + index as f32 * 3.0_f32)
        .collect();
    let expected_values: Vec<f32> = input_values
        .iter()
        .map(|value| apply_f32(*value, operand, operation))
        .collect();
    let input_bits = input_values
        .iter()
        .map(|value| value.to_bits())
        .collect::<Vec<_>>();
    let expected_bits = expected_values
        .iter()
        .map(|value| value.to_bits())
        .collect::<Vec<_>>();
    let bindings = vec![input_bits, vec![0_u32; 70], vec![70]];
    let actual_bits = run_prepared_u32_uav_artifact(&artifact, dxil, [2, 1, 1], &bindings, 1)?;
    for (index, (actual, expected)) in actual_bits.iter().zip(expected_bits.iter()).enumerate() {
        if actual != expected {
            return Err(DirectX12Error::DifferentialMismatch {
                index: index as u32,
                actual: *actual,
                expected: *expected,
            });
        }
    }
    Ok(DirectX12F32ArtifactExecutionReport {
        schema: "jadren-directx12-f32-artifact-execution-0.1",
        entry_name: artifact.entry_name,
        artifact_resource_binding_count: artifact.resources.len(),
        artifact_word_count: artifact.words.len(),
        artifact_word_hash: stable_spirv_word_hash(&artifact.words),
        addend: operand,
        operation: f32_binary_name(operation),
        logical_length: actual_bits.len() as u32,
        first_output: f32::from_bits(actual_bits[0]),
        last_output: f32::from_bits(actual_bits[actual_bits.len() - 1]),
        output_checksum: expected_values.iter().map(|value| f64::from(*value)).sum(),
        artifact_validated: true,
        dxil_translated: true,
        execution_completed: true,
        translation_path,
        result: match operation {
            F32ArithmeticOp::Add => "pass-f32-artifact-compute-differential",
            F32ArithmeticOp::Subtract | F32ArithmeticOp::Multiply => {
                "pass-f32-binary-artifact-compute-differential"
            }
        },
    })
}

/// Executes the verified runtime-length `f32x4` add artifact through DX12.
///
/// Input/output payloads use 16-byte structured elements and are transported
/// as raw bytes through the generic UAV lifecycle. The internal fallback is a
/// deliberately narrow `float4` HLSL kernel; arbitrary SPIR-V is never
/// accepted when the external SPIRV-Cross toolchain is unavailable.
pub fn run_f32_vector_artifact_smoke(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    input_values: &[[f32; 4]],
) -> Result<DirectX12F32VectorArtifactExecutionReport, DirectX12Error> {
    run_f32_vector_binary_artifact_smoke(
        module,
        function,
        options,
        input_values,
        F32ArithmeticOp::Add,
    )
}

/// Executes a verified runtime-length `f32x4` binary artifact through DX12.
///
/// The operation is encoded in the SPIR-V artifact and is also used by the
/// narrow HLSL fallback. Keeping both paths on the same operation parameter
/// prevents a translated artifact from being compared against an Add-only
/// CPU oracle.
pub fn run_f32_vector_binary_artifact_smoke(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    input_values: &[[f32; 4]],
    operation: F32ArithmeticOp,
) -> Result<DirectX12F32VectorArtifactExecutionReport, DirectX12Error> {
    let operand = f32_vector_operation_operand(operation);
    if input_values.is_empty() || input_values.len() > F32_VECTOR_CAPACITY {
        return Err(DirectX12Error::ArtifactContract(
            "runtime-length f32x4 input count must be in 1..=128",
        ));
    }
    if input_values
        .iter()
        .flatten()
        .any(|value| !value.is_finite())
    {
        return Err(DirectX12Error::ArtifactContract(
            "runtime-length f32x4 input values must be finite",
        ));
    }
    let artifact = emit_storage_global_index_vector_f32_binary_dynamic_length_artifact_from_jir(
        module, function, options, operation,
    )
    .map_err(|error| DirectX12Error::JirSpirvLowering(error.to_string()))?;
    validate_shared_artifact(&artifact)?;
    validate_f32_vector_artifact_contract(&artifact, operand.to_bits(), operation)?;
    if artifact.workgroup_size != F32_VECTOR_WORKGROUP_SIZE {
        return Err(DirectX12Error::ArtifactContract(
            "runtime-length f32x4 DX12 executor requires a 64x1x1 workgroup",
        ));
    }
    let (dxil, translation_path, views) = if external_vector_translation_enabled() {
        match translate_spirv_artifact_to_dxil_report(&artifact) {
            Ok(translation) => (
                translation.dxil,
                "spirv-cross-dxc",
                translation.resource_views,
            ),
            Err(
                DirectX12Error::SpirvToolchainUnavailable
                | DirectX12Error::SpirvCrossUnavailable
                | DirectX12Error::ShaderToolchainUnavailable,
            ) => (
                compile_f32_vector_artifact_dxil_for_operation(
                    &artifact,
                    operand.to_bits(),
                    operation,
                )?,
                "jadren-artifact-f32x4-hlsl-to-dxil",
                structured_views_for_artifact(&artifact),
            ),
            Err(error) => return Err(error),
        }
    } else {
        (
            compile_f32_vector_artifact_dxil_for_operation(
                &artifact,
                operand.to_bits(),
                operation,
            )?,
            "jadren-artifact-f32x4-hlsl-to-dxil",
            structured_views_for_artifact(&artifact),
        )
    };
    let mut input_payload = vec![0_u8; F32_VECTOR_CAPACITY * 16];
    for (index, value) in input_values.iter().enumerate() {
        write_f32x4_bytes(&mut input_payload[index * 16..(index + 1) * 16], value);
    }
    let output_payload = vec![0_u8; F32_VECTOR_CAPACITY * 16];
    let length_payload = (input_values.len() as u32).to_ne_bytes().to_vec();
    let bindings = [
        UavBindingPayload {
            resource_id: 0,
            bytes: &input_payload,
            element_stride: 16,
        },
        UavBindingPayload {
            resource_id: 1,
            bytes: &output_payload,
            element_stride: 16,
        },
        UavBindingPayload {
            resource_id: 2,
            bytes: &length_payload,
            element_stride: 4,
        },
    ];
    let actual_bytes =
        run_prepared_uav_artifact_with_views(&artifact, dxil, [2, 1, 1], &bindings, 1, &views)?;
    let actual_values = parse_f32x4_bytes(&actual_bytes)?;
    let mut expected_values = vec![[0.0_f32; 4]; F32_VECTOR_CAPACITY];
    for (index, value) in input_values.iter().enumerate() {
        expected_values[index] = value.map(|lane| apply_f32(lane, operand, operation));
    }
    if actual_values.len() != expected_values.len()
        || actual_values
            .iter()
            .zip(expected_values.iter())
            .any(|(actual, expected)| {
                actual
                    .iter()
                    .zip(expected.iter())
                    .any(|(actual, expected)| actual.to_bits() != expected.to_bits())
            })
    {
        return Err(DirectX12Error::ArtifactContract(
            "runtime-length f32x4 output failed exact lane differential",
        ));
    }
    let input_checksum = input_values
        .iter()
        .flatten()
        .map(|value| f64::from(*value))
        .sum();
    let output_checksum = actual_values[..input_values.len()]
        .iter()
        .flatten()
        .map(|value| f64::from(*value))
        .sum();
    let untouched_tail_count = actual_values[input_values.len()..]
        .iter()
        .filter(|value| value.iter().all(|lane| lane.to_bits() == 0))
        .count();
    Ok(DirectX12F32VectorArtifactExecutionReport {
        schema: "jadren-directx12-f32x4-artifact-execution-0.1",
        entry_name: artifact.entry_name,
        artifact_resource_binding_count: artifact.resources.len(),
        artifact_word_count: artifact.words.len(),
        artifact_word_hash: stable_spirv_word_hash(&artifact.words),
        operand,
        operation: f32_binary_name(operation),
        logical_length: input_values.len() as u32,
        capacity: F32_VECTOR_CAPACITY as u32,
        first_output: actual_values[0],
        last_output: actual_values[input_values.len() - 1],
        input_checksum,
        output_checksum,
        untouched_tail_count: untouched_tail_count as u32,
        artifact_validated: true,
        dxil_translated: true,
        execution_completed: true,
        translation_path,
        result: match operation {
            F32ArithmeticOp::Add => "pass-f32x4-artifact-compute-differential",
            F32ArithmeticOp::Subtract | F32ArithmeticOp::Multiply => {
                "pass-f32x4-binary-artifact-compute-differential"
            }
        },
    })
}

/// Executes a verified runtime-length vector artifact with two to four f32
/// lanes per structured element through the same generic DX12 UAV lifecycle.
pub fn run_f32_vector_lanes_binary_artifact_smoke(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    input_values: &[Vec<f32>],
    operation: F32ArithmeticOp,
) -> Result<DirectX12F32VectorLanesArtifactExecutionReport, DirectX12Error> {
    let operand = f32_vector_operation_operand(operation);
    if input_values.is_empty() || input_values.len() > F32_VECTOR_CAPACITY {
        return Err(DirectX12Error::ArtifactContract(
            "runtime-length vector input count must be in 1..=128",
        ));
    }
    let lane_count = input_values[0].len();
    if !(2..=4).contains(&lane_count) {
        return Err(DirectX12Error::ArtifactContract(
            "runtime-length vector lane count must be in 2..=4",
        ));
    }
    if input_values
        .iter()
        .any(|value| value.len() != lane_count || value.iter().any(|lane| !lane.is_finite()))
    {
        return Err(DirectX12Error::ArtifactContract(
            "runtime-length vector inputs must have equal finite lane counts",
        ));
    }
    let artifact =
        emit_storage_global_index_vector_f32_binary_dynamic_length_lanes_artifact_from_jir(
            module,
            function,
            options,
            operation,
            lane_count as u32,
        )
        .map_err(|error| DirectX12Error::JirSpirvLowering(error.to_string()))?;
    validate_shared_artifact(&artifact)?;
    validate_f32_vector_lanes_artifact_contract(
        &artifact,
        operand.to_bits(),
        operation,
        lane_count as u32,
    )?;
    if artifact.workgroup_size != F32_VECTOR_WORKGROUP_SIZE {
        return Err(DirectX12Error::ArtifactContract(
            "runtime-length vector DX12 executor requires a 64x1x1 workgroup",
        ));
    }
    let (dxil, translation_path, views) = if external_vector_translation_enabled() {
        match translate_spirv_artifact_to_dxil_report(&artifact) {
            Ok(translation) => (
                translation.dxil,
                "spirv-cross-dxc",
                translation.resource_views,
            ),
            Err(
                DirectX12Error::SpirvToolchainUnavailable
                | DirectX12Error::SpirvCrossUnavailable
                | DirectX12Error::ShaderToolchainUnavailable,
            ) => (
                compile_f32_vector_lanes_artifact_dxil_for_operation(
                    &artifact,
                    operand.to_bits(),
                    operation,
                    lane_count as u32,
                )?,
                "jadren-artifact-f32-vector-lanes-hlsl-to-dxil",
                structured_views_for_artifact(&artifact),
            ),
            Err(error) => return Err(error),
        }
    } else {
        (
            compile_f32_vector_lanes_artifact_dxil_for_operation(
                &artifact,
                operand.to_bits(),
                operation,
                lane_count as u32,
            )?,
            "jadren-artifact-f32-vector-lanes-hlsl-to-dxil",
            structured_views_for_artifact(&artifact),
        )
    };
    let stride = lane_count * std::mem::size_of::<f32>();
    let mut input_payload = vec![0_u8; F32_VECTOR_CAPACITY * stride];
    for (index, value) in input_values.iter().enumerate() {
        for (lane, scalar) in value.iter().enumerate() {
            let start = (index * lane_count + lane) * std::mem::size_of::<f32>();
            input_payload[start..start + 4].copy_from_slice(&scalar.to_ne_bytes());
        }
    }
    let output_payload = vec![0_u8; F32_VECTOR_CAPACITY * stride];
    let length_payload = (input_values.len() as u32).to_ne_bytes().to_vec();
    let bindings = [
        UavBindingPayload {
            resource_id: 0,
            bytes: &input_payload,
            element_stride: stride as u32,
        },
        UavBindingPayload {
            resource_id: 1,
            bytes: &output_payload,
            element_stride: stride as u32,
        },
        UavBindingPayload {
            resource_id: 2,
            bytes: &length_payload,
            element_stride: 4,
        },
    ];
    let actual_bytes =
        run_prepared_uav_artifact_with_views(&artifact, dxil, [2, 1, 1], &bindings, 1, &views)?;
    let actual_values = parse_f32_vector_lanes_bytes(&actual_bytes, lane_count)?;
    let mut expected_values = vec![vec![0.0_f32; lane_count]; F32_VECTOR_CAPACITY];
    for (index, value) in input_values.iter().enumerate() {
        expected_values[index] = value
            .iter()
            .map(|lane| apply_f32(*lane, operand, operation))
            .collect();
    }
    if actual_values.len() != expected_values.len()
        || actual_values
            .iter()
            .zip(expected_values.iter())
            .any(|(actual, expected)| {
                actual
                    .iter()
                    .zip(expected.iter())
                    .any(|(actual, expected)| actual.to_bits() != expected.to_bits())
            })
    {
        return Err(DirectX12Error::ArtifactContract(
            "runtime-length vector output failed exact lane differential",
        ));
    }
    let input_checksum = input_values
        .iter()
        .flatten()
        .map(|value| f64::from(*value))
        .sum();
    let output_checksum = actual_values[..input_values.len()]
        .iter()
        .flatten()
        .map(|value| f64::from(*value))
        .sum();
    let untouched_tail_count = actual_values[input_values.len()..]
        .iter()
        .filter(|value| value.iter().all(|lane| lane.to_bits() == 0))
        .count();
    Ok(DirectX12F32VectorLanesArtifactExecutionReport {
        schema: "jadren-directx12-f32-vector-lanes-artifact-execution-0.1",
        entry_name: artifact.entry_name,
        lane_count: lane_count as u32,
        artifact_resource_binding_count: artifact.resources.len(),
        artifact_word_count: artifact.words.len(),
        artifact_word_hash: stable_spirv_word_hash(&artifact.words),
        operand,
        operation: f32_binary_name(operation),
        logical_length: input_values.len() as u32,
        capacity: F32_VECTOR_CAPACITY as u32,
        first_output: actual_values[0].clone(),
        last_output: actual_values[input_values.len() - 1].clone(),
        input_checksum,
        output_checksum,
        untouched_tail_count: untouched_tail_count as u32,
        artifact_validated: true,
        dxil_translated: true,
        execution_completed: true,
        translation_path,
        result: "pass-f32-vector-lanes-artifact-compute-differential",
    })
}

/// Executes the verified one-resource JIR storage-add artifact. This shape is
/// deliberately separate from the three-resource runtime BinaryOp ABI: the
/// shader mutates element zero of one reflected UAV and leaves the remaining
/// fixture elements untouched.
pub fn run_storage_add_artifact_smoke(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    addend: u32,
    input_start: u32,
    element_count: u32,
) -> Result<DirectX12ArtifactExecutionReport, DirectX12Error> {
    if element_count == 0 {
        return Err(DirectX12Error::ArtifactContract(
            "storage-add fixture requires at least one element",
        ));
    }
    let artifact = emit_storage_add_artifact_from_jir(module, function, options)
        .map_err(|error| DirectX12Error::JirSpirvLowering(error.to_string()))?;
    validate_shared_artifact(&artifact)?;
    validate_storage_add_artifact_contract(&artifact, addend)?;
    if artifact.resources.len() != 1 {
        return Err(DirectX12Error::ArtifactContract(
            "storage-add artifact requires exactly one resource",
        ));
    }
    let (dxil, translation_path) = match translate_spirv_artifact_to_dxil(&artifact) {
        Ok(dxil) => (dxil, "spirv-cross-dxc"),
        Err(
            DirectX12Error::SpirvToolchainUnavailable
            | DirectX12Error::SpirvCrossUnavailable
            | DirectX12Error::ShaderToolchainUnavailable,
        ) => (
            compile_storage_add_artifact_dxil(&artifact, addend)?,
            "jadren-artifact-narrow-hlsl-to-dxil",
        ),
        Err(error) => return Err(error),
    };
    let input_values: Vec<u32> = (0..element_count)
        .map(|index| input_start.wrapping_add(index))
        .collect();
    let mut expected_values = input_values.clone();
    expected_values[0] = expected_values[0].wrapping_add(addend);
    let bindings = vec![input_values];
    let actual_values = run_prepared_u32_uav_artifact(&artifact, dxil, [1, 1, 1], &bindings, 0)?;
    let mismatch = actual_values
        .iter()
        .zip(expected_values.iter())
        .enumerate()
        .find_map(|(index, (actual, expected))| {
            (actual != expected).then_some((index as u32, *actual, *expected))
        });
    if let Some((index, actual, expected)) = mismatch {
        return Err(DirectX12Error::DifferentialMismatch {
            index,
            actual,
            expected,
        });
    }
    Ok(DirectX12ArtifactExecutionReport {
        schema: "jadren-directx12-artifact-execution-0.1",
        entry_name: artifact.entry_name,
        artifact_resource_binding_count: artifact.resources.len(),
        artifact_word_count: artifact.words.len(),
        artifact_word_hash: stable_spirv_word_hash(&artifact.words),
        operation: "storage-add",
        artifact_validated: true,
        dxil_translated: true,
        execution_completed: true,
        output_checksum: actual_values.iter().map(|value| u64::from(*value)).sum(),
        first_output: actual_values.first().copied(),
        last_output: actual_values.last().copied(),
        logical_length: None,
        capacity: None,
        untouched_tail_count: None,
        translation_path,
        result: "pass-artifact-compute-differential",
    })
}

/// Executes the validated one-resource bounds-safe `GlobalInvocationId.x`
/// JIR storage-write artifact through DX12. The artifact carries the value and
/// length constants as well as the structured SPIR-V bounds branch; the
/// narrow fallback decodes those facts before generating HLSL.
pub fn run_global_write_artifact_smoke(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    value: u32,
    length: u32,
    capacity: u32,
) -> Result<DirectX12ArtifactExecutionReport, DirectX12Error> {
    if length == 0 || capacity < length {
        return Err(DirectX12Error::ArtifactContract(
            "global-write fixture requires a non-zero length within capacity",
        ));
    }
    let artifact = emit_storage_global_index_write_artifact_from_jir(module, function, options)
        .map_err(|error| DirectX12Error::JirSpirvLowering(error.to_string()))?;
    validate_shared_artifact(&artifact)?;
    validate_global_write_artifact_contract(&artifact, value, length)?;
    if artifact.resources.len() != 1 {
        return Err(DirectX12Error::ArtifactContract(
            "global-write artifact requires exactly one resource",
        ));
    }
    let [workgroup_x, workgroup_y, workgroup_z] = artifact.workgroup_size;
    if workgroup_x == 0 || workgroup_y != 1 || workgroup_z != 1 {
        return Err(DirectX12Error::ArtifactContract(
            "global-write artifact requires a one-dimensional non-zero workgroup",
        ));
    }
    let (dxil, translation_path) = match translate_spirv_artifact_to_dxil(&artifact) {
        Ok(dxil) => (dxil, "spirv-cross-dxc"),
        Err(
            DirectX12Error::SpirvToolchainUnavailable
            | DirectX12Error::SpirvCrossUnavailable
            | DirectX12Error::ShaderToolchainUnavailable,
        ) => (
            compile_global_write_artifact_dxil(&artifact, value, length)?,
            "jadren-artifact-narrow-hlsl-to-dxil",
        ),
        Err(error) => return Err(error),
    };
    let capacity_usize = usize::try_from(capacity).map_err(|_| {
        DirectX12Error::ArtifactContract("global-write capacity does not fit host usize")
    })?;
    let input_values = vec![0_u32; capacity_usize];
    let dispatch_x =
        capacity
            .checked_add(workgroup_x - 1)
            .ok_or(DirectX12Error::ArtifactContract(
                "global-write dispatch group count overflows",
            ))?
            / workgroup_x;
    let bindings = vec![input_values];
    let actual_values =
        run_prepared_u32_uav_artifact(&artifact, dxil, [dispatch_x, 1, 1], &bindings, 0)?;
    let mut expected_values = vec![0_u32; capacity_usize];
    expected_values[..length as usize].fill(value);
    let mismatch = actual_values
        .iter()
        .zip(expected_values.iter())
        .enumerate()
        .find_map(|(index, (actual, expected))| {
            (actual != expected).then_some((index as u32, *actual, *expected))
        });
    let untouched_tail_count = actual_values[length as usize..]
        .iter()
        .filter(|value| **value == 0)
        .count();
    if let Some((index, actual, expected)) = mismatch {
        return Err(DirectX12Error::DifferentialMismatch {
            index,
            actual,
            expected,
        });
    }
    if untouched_tail_count != (capacity - length) as usize {
        return Err(DirectX12Error::ArtifactContract(
            "global-write artifact modified an out-of-bounds storage tail",
        ));
    }
    Ok(DirectX12ArtifactExecutionReport {
        schema: "jadren-directx12-artifact-execution-0.1",
        entry_name: artifact.entry_name,
        artifact_resource_binding_count: artifact.resources.len(),
        artifact_word_count: artifact.words.len(),
        artifact_word_hash: stable_spirv_word_hash(&artifact.words),
        operation: "global-write",
        artifact_validated: true,
        dxil_translated: true,
        execution_completed: true,
        output_checksum: actual_values.iter().map(|value| u64::from(*value)).sum(),
        first_output: actual_values.first().copied(),
        last_output: actual_values.get(length as usize - 1).copied(),
        logical_length: Some(length),
        capacity: Some(capacity),
        untouched_tail_count: Some(untouched_tail_count as u32),
        translation_path,
        result: "pass-artifact-compute-differential",
    })
}

/// Executes a validated one-dimensional runtime-stride JIR storage-write
/// artifact through native DX12. The narrow fallback accepts only the verified
/// four-resource `GlobalInvocationId.x` shape before emitting HLSL.
pub fn run_global_strided_write_artifact_smoke(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    config: GlobalStridedWriteArtifactConfig,
) -> Result<DirectX12ArtifactExecutionReport, DirectX12Error> {
    let GlobalStridedWriteArtifactConfig {
        value,
        length,
        stride,
        capacity,
    } = config;
    if length == 0 || stride == 0 || capacity == 0 {
        return Err(DirectX12Error::ArtifactContract(
            "global-strided-write fixture requires non-zero length, stride and capacity",
        ));
    }
    let last_physical_index =
        (length - 1)
            .checked_mul(stride)
            .ok_or(DirectX12Error::ArtifactContract(
                "global-strided-write physical index overflows u32",
            ))?;
    if last_physical_index >= capacity {
        return Err(DirectX12Error::ArtifactContract(
            "global-strided-write physical range exceeds capacity",
        ));
    }
    let artifact =
        emit_storage_global_index_strided_write_artifact_from_jir(module, function, options)
            .map_err(|error| DirectX12Error::JirSpirvLowering(error.to_string()))?;
    validate_shared_artifact(&artifact)?;
    validate_global_strided_write_artifact_contract(&artifact, value)?;
    let [workgroup_x, workgroup_y, workgroup_z] = artifact.workgroup_size;
    if workgroup_x == 0 || workgroup_y != 1 || workgroup_z != 1 {
        return Err(DirectX12Error::ArtifactContract(
            "global-strided-write artifact requires a one-dimensional workgroup",
        ));
    }
    let (dxil, translation_path) = match translate_spirv_artifact_to_dxil(&artifact) {
        Ok(dxil) => (dxil, "spirv-cross-dxc"),
        Err(
            DirectX12Error::SpirvToolchainUnavailable
            | DirectX12Error::SpirvCrossUnavailable
            | DirectX12Error::ShaderToolchainUnavailable,
        ) => (
            compile_global_strided_write_artifact_dxil(&artifact, value)?,
            "jadren-artifact-narrow-hlsl-to-dxil",
        ),
        Err(error) => return Err(error),
    };
    let dispatch_x = length.div_ceil(workgroup_x);
    let capacity_usize = usize::try_from(capacity).map_err(|_| {
        DirectX12Error::ArtifactContract("global-strided-write capacity does not fit host usize")
    })?;
    let bindings = vec![
        vec![0_u32; capacity_usize],
        vec![length],
        vec![stride],
        vec![capacity],
    ];
    let actual_values =
        run_prepared_u32_uav_artifact(&artifact, dxil, [dispatch_x, 1, 1], &bindings, 0)?;
    let mut expected_values = vec![0_u32; capacity_usize];
    for index in 0..length {
        expected_values[(index * stride) as usize] = value;
    }
    for (index, (actual, expected)) in actual_values.iter().zip(expected_values.iter()).enumerate()
    {
        if actual != expected {
            return Err(DirectX12Error::DifferentialMismatch {
                index: index as u32,
                actual: *actual,
                expected: *expected,
            });
        }
    }
    let untouched_tail_count = actual_values.iter().filter(|value| **value == 0).count();
    let expected_untouched_count = expected_values.iter().filter(|value| **value == 0).count();
    if untouched_tail_count != expected_untouched_count {
        return Err(DirectX12Error::ArtifactContract(
            "global-strided-write artifact modified an unwritten physical element",
        ));
    }
    Ok(DirectX12ArtifactExecutionReport {
        schema: "jadren-directx12-artifact-execution-0.1",
        entry_name: artifact.entry_name,
        artifact_resource_binding_count: artifact.resources.len(),
        artifact_word_count: artifact.words.len(),
        artifact_word_hash: stable_spirv_word_hash(&artifact.words),
        operation: "global-strided-write",
        artifact_validated: true,
        dxil_translated: true,
        execution_completed: true,
        output_checksum: actual_values.iter().map(|value| u64::from(*value)).sum(),
        first_output: actual_values.first().copied(),
        last_output: actual_values.get(last_physical_index as usize).copied(),
        logical_length: Some(length),
        capacity: Some(capacity),
        untouched_tail_count: Some(untouched_tail_count as u32),
        translation_path,
        result: "pass-artifact-compute-differential",
    })
}

/// Executes a validated two-dimensional row-major JIR storage-write artifact
/// through native DX12. The narrow fallback accepts only the verified
/// four-resource `GlobalInvocationId.x/y` shape before emitting HLSL.
pub fn run_global_2d_write_artifact_smoke(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    value: u32,
    width: u32,
    height: u32,
    capacity: u32,
) -> Result<DirectX12ArtifactExecutionReport, DirectX12Error> {
    let logical_length = width
        .checked_mul(height)
        .ok_or(DirectX12Error::ArtifactContract(
            "global-2d-write logical length overflows u32",
        ))?;
    if width == 0 || height == 0 || capacity < logical_length {
        return Err(DirectX12Error::ArtifactContract(
            "global-2d-write fixture requires non-zero dimensions within capacity",
        ));
    }
    let artifact = emit_storage_global_2d_write_artifact_from_jir(module, function, options)
        .map_err(|error| DirectX12Error::JirSpirvLowering(error.to_string()))?;
    validate_shared_artifact(&artifact)?;
    validate_global_2d_write_artifact_contract(&artifact, value)?;
    let [workgroup_x, workgroup_y, workgroup_z] = artifact.workgroup_size;
    if workgroup_x == 0 || workgroup_y == 0 || workgroup_z != 1 {
        return Err(DirectX12Error::ArtifactContract(
            "global-2d-write artifact requires a two-dimensional workgroup",
        ));
    }
    let (dxil, translation_path) = match translate_spirv_artifact_to_dxil(&artifact) {
        Ok(dxil) => (dxil, "spirv-cross-dxc"),
        Err(
            DirectX12Error::SpirvToolchainUnavailable
            | DirectX12Error::SpirvCrossUnavailable
            | DirectX12Error::ShaderToolchainUnavailable,
        ) => (
            compile_global_2d_write_artifact_dxil(&artifact, value)?,
            "jadren-artifact-narrow-hlsl-to-dxil",
        ),
        Err(error) => return Err(error),
    };
    let dispatch_x = width.div_ceil(workgroup_x);
    let dispatch_y = height.div_ceil(workgroup_y);
    let capacity_usize = usize::try_from(capacity).map_err(|_| {
        DirectX12Error::ArtifactContract("global-2d-write capacity does not fit host usize")
    })?;
    let bindings = vec![
        vec![0_u32; capacity_usize],
        vec![width],
        vec![height],
        vec![capacity],
    ];
    let actual_values =
        run_prepared_u32_uav_artifact(&artifact, dxil, [dispatch_x, dispatch_y, 1], &bindings, 0)?;
    let mut expected_values = vec![0_u32; capacity_usize];
    let width_usize = usize::try_from(width).map_err(|_| {
        DirectX12Error::ArtifactContract("global-2d-write width does not fit host usize")
    })?;
    let height_usize = usize::try_from(height).map_err(|_| {
        DirectX12Error::ArtifactContract("global-2d-write height does not fit host usize")
    })?;
    for y in 0..height_usize {
        for x in 0..width_usize {
            expected_values[y * width_usize + x] = value;
        }
    }
    for (index, (actual, expected)) in actual_values.iter().zip(expected_values.iter()).enumerate()
    {
        if actual != expected {
            return Err(DirectX12Error::DifferentialMismatch {
                index: index as u32,
                actual: *actual,
                expected: *expected,
            });
        }
    }
    let logical_length_usize = usize::try_from(logical_length).map_err(|_| {
        DirectX12Error::ArtifactContract("global-2d-write logical length does not fit host usize")
    })?;
    let untouched_tail_count = actual_values[logical_length_usize..]
        .iter()
        .filter(|value| **value == 0)
        .count();
    if untouched_tail_count != capacity_usize - logical_length_usize {
        return Err(DirectX12Error::ArtifactContract(
            "global-2d-write artifact modified an out-of-bounds storage tail",
        ));
    }
    Ok(DirectX12ArtifactExecutionReport {
        schema: "jadren-directx12-artifact-execution-0.1",
        operation: "global-2d-write",
        entry_name: artifact.entry_name,
        artifact_resource_binding_count: artifact.resources.len(),
        artifact_word_count: artifact.words.len(),
        artifact_word_hash: stable_spirv_word_hash(&artifact.words),
        artifact_validated: true,
        dxil_translated: true,
        execution_completed: true,
        output_checksum: actual_values.iter().map(|value| u64::from(*value)).sum(),
        first_output: actual_values.first().copied(),
        last_output: actual_values.get(logical_length_usize - 1).copied(),
        logical_length: Some(logical_length),
        capacity: Some(capacity),
        untouched_tail_count: Some(untouched_tail_count as u32),
        translation_path,
        result: "pass-artifact-compute-differential",
    })
}

/// Executes a validated two-dimensional affine-stride JIR storage-write
/// artifact through native DX12. The narrow fallback accepts only the
/// verified six-resource `GlobalInvocationId.x/y` shape before emitting HLSL.
pub fn run_global_2d_strided_write_artifact_smoke(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    config: Global2dStridedWriteArtifactConfig,
) -> Result<DirectX12ArtifactExecutionReport, DirectX12Error> {
    let Global2dStridedWriteArtifactConfig {
        value,
        width,
        height,
        stride_x,
        stride_y,
        capacity,
    } = config;
    let logical_length = width
        .checked_mul(height)
        .ok_or(DirectX12Error::ArtifactContract(
            "global-2d-strided-write logical length overflows u32",
        ))?;
    if width == 0 || height == 0 || capacity == 0 {
        return Err(DirectX12Error::ArtifactContract(
            "global-2d-strided-write fixture requires non-zero dimensions and capacity",
        ));
    }
    let last_physical_index = (width - 1)
        .checked_mul(stride_x)
        .and_then(|x_offset| {
            (height - 1)
                .checked_mul(stride_y)
                .and_then(|y_offset| x_offset.checked_add(y_offset))
        })
        .ok_or(DirectX12Error::ArtifactContract(
            "global-2d-strided-write physical index overflows u32",
        ))?;
    if last_physical_index >= capacity {
        return Err(DirectX12Error::ArtifactContract(
            "global-2d-strided-write physical range exceeds capacity",
        ));
    }
    let artifact =
        emit_storage_global_2d_strided_write_artifact_from_jir(module, function, options)
            .map_err(|error| DirectX12Error::JirSpirvLowering(error.to_string()))?;
    validate_shared_artifact(&artifact)?;
    validate_global_2d_strided_write_artifact_contract(&artifact, value)?;
    let [workgroup_x, workgroup_y, workgroup_z] = artifact.workgroup_size;
    if workgroup_x == 0 || workgroup_y == 0 || workgroup_z != 1 {
        return Err(DirectX12Error::ArtifactContract(
            "global-2d-strided-write artifact requires a two-dimensional workgroup",
        ));
    }
    let (dxil, translation_path) = match translate_spirv_artifact_to_dxil(&artifact) {
        Ok(dxil) => (dxil, "spirv-cross-dxc"),
        Err(
            DirectX12Error::SpirvToolchainUnavailable
            | DirectX12Error::SpirvCrossUnavailable
            | DirectX12Error::ShaderToolchainUnavailable,
        ) => (
            compile_global_2d_strided_write_artifact_dxil(&artifact, value)?,
            "jadren-artifact-narrow-hlsl-to-dxil",
        ),
        Err(error) => return Err(error),
    };
    let dispatch_x = width.div_ceil(workgroup_x);
    let dispatch_y = height.div_ceil(workgroup_y);
    let capacity_usize = usize::try_from(capacity).map_err(|_| {
        DirectX12Error::ArtifactContract("global-2d-strided-write capacity does not fit host usize")
    })?;
    let bindings = vec![
        vec![0_u32; capacity_usize],
        vec![width],
        vec![height],
        vec![stride_x],
        vec![stride_y],
        vec![capacity],
    ];
    let actual_values =
        run_prepared_u32_uav_artifact(&artifact, dxil, [dispatch_x, dispatch_y, 1], &bindings, 0)?;
    let mut expected_values = vec![0_u32; capacity_usize];
    for y in 0..height {
        for x in 0..width {
            let physical_index = x * stride_x + y * stride_y;
            expected_values[physical_index as usize] = value;
        }
    }
    for (index, (actual, expected)) in actual_values.iter().zip(expected_values.iter()).enumerate()
    {
        if actual != expected {
            return Err(DirectX12Error::DifferentialMismatch {
                index: index as u32,
                actual: *actual,
                expected: *expected,
            });
        }
    }
    let untouched_tail_count = actual_values.iter().filter(|value| **value == 0).count();
    let expected_untouched_count = expected_values.iter().filter(|value| **value == 0).count();
    if untouched_tail_count != expected_untouched_count {
        return Err(DirectX12Error::ArtifactContract(
            "global-2d-strided-write artifact modified an unwritten physical element",
        ));
    }
    Ok(DirectX12ArtifactExecutionReport {
        schema: "jadren-directx12-artifact-execution-0.1",
        operation: "global-2d-strided-write",
        entry_name: artifact.entry_name,
        artifact_resource_binding_count: artifact.resources.len(),
        artifact_word_count: artifact.words.len(),
        artifact_word_hash: stable_spirv_word_hash(&artifact.words),
        artifact_validated: true,
        dxil_translated: true,
        execution_completed: true,
        output_checksum: actual_values.iter().map(|value| u64::from(*value)).sum(),
        first_output: actual_values.first().copied(),
        last_output: actual_values.get(last_physical_index as usize).copied(),
        logical_length: Some(logical_length),
        capacity: Some(capacity),
        untouched_tail_count: Some(untouched_tail_count as u32),
        translation_path,
        result: "pass-artifact-compute-differential",
    })
}

/// Executes a validated three-dimensional row-major JIR storage-write artifact
/// through native DX12. The narrow fallback accepts only the verified
/// five-resource `GlobalInvocationId.x/y/z` shape before emitting HLSL.
pub fn run_global_3d_write_artifact_smoke(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    config: Global3dWriteArtifactConfig,
) -> Result<DirectX12ArtifactExecutionReport, DirectX12Error> {
    let Global3dWriteArtifactConfig {
        value,
        width,
        height,
        depth,
        capacity,
    } = config;
    let logical_length = width
        .checked_mul(height)
        .and_then(|area| area.checked_mul(depth))
        .ok_or(DirectX12Error::ArtifactContract(
            "global-3d-write logical length overflows u32",
        ))?;
    if width == 0 || height == 0 || depth == 0 || capacity < logical_length {
        return Err(DirectX12Error::ArtifactContract(
            "global-3d-write fixture requires non-zero dimensions within capacity",
        ));
    }
    let artifact = emit_storage_global_3d_write_artifact_from_jir(module, function, options)
        .map_err(|error| DirectX12Error::JirSpirvLowering(error.to_string()))?;
    validate_shared_artifact(&artifact)?;
    validate_global_3d_write_artifact_contract(&artifact, value)?;
    let [workgroup_x, workgroup_y, workgroup_z] = artifact.workgroup_size;
    if workgroup_x == 0 || workgroup_y == 0 || workgroup_z == 0 {
        return Err(DirectX12Error::ArtifactContract(
            "global-3d-write artifact requires a three-dimensional workgroup",
        ));
    }
    let (dxil, translation_path) = match translate_spirv_artifact_to_dxil(&artifact) {
        Ok(dxil) => (dxil, "spirv-cross-dxc"),
        Err(
            DirectX12Error::SpirvToolchainUnavailable
            | DirectX12Error::SpirvCrossUnavailable
            | DirectX12Error::ShaderToolchainUnavailable,
        ) => (
            compile_global_3d_write_artifact_dxil(&artifact, value)?,
            "jadren-artifact-narrow-hlsl-to-dxil",
        ),
        Err(error) => return Err(error),
    };
    let dispatch_x = width.div_ceil(workgroup_x);
    let dispatch_y = height.div_ceil(workgroup_y);
    let dispatch_z = depth.div_ceil(workgroup_z);
    let capacity_usize = usize::try_from(capacity).map_err(|_| {
        DirectX12Error::ArtifactContract("global-3d-write capacity does not fit host usize")
    })?;
    let bindings = vec![
        vec![0_u32; capacity_usize],
        vec![width],
        vec![height],
        vec![depth],
        vec![capacity],
    ];
    let actual_values = run_prepared_u32_uav_artifact(
        &artifact,
        dxil,
        [dispatch_x, dispatch_y, dispatch_z],
        &bindings,
        0,
    )?;
    let mut expected_values = vec![0_u32; capacity_usize];
    expected_values[..logical_length as usize].fill(value);
    for (index, (actual, expected)) in actual_values.iter().zip(expected_values.iter()).enumerate()
    {
        if actual != expected {
            return Err(DirectX12Error::DifferentialMismatch {
                index: index as u32,
                actual: *actual,
                expected: *expected,
            });
        }
    }
    let logical_length_usize = usize::try_from(logical_length).map_err(|_| {
        DirectX12Error::ArtifactContract("global-3d-write logical length does not fit host usize")
    })?;
    let untouched_tail_count = actual_values[logical_length_usize..]
        .iter()
        .filter(|value| **value == 0)
        .count();
    if untouched_tail_count != capacity_usize - logical_length_usize {
        return Err(DirectX12Error::ArtifactContract(
            "global-3d-write artifact modified an out-of-bounds storage tail",
        ));
    }
    Ok(DirectX12ArtifactExecutionReport {
        schema: "jadren-directx12-artifact-execution-0.1",
        entry_name: artifact.entry_name,
        artifact_resource_binding_count: artifact.resources.len(),
        artifact_word_count: artifact.words.len(),
        artifact_word_hash: stable_spirv_word_hash(&artifact.words),
        operation: "global-3d-write",
        artifact_validated: true,
        dxil_translated: true,
        execution_completed: true,
        output_checksum: actual_values.iter().map(|value| u64::from(*value)).sum(),
        first_output: actual_values.first().copied(),
        last_output: actual_values.get(logical_length_usize - 1).copied(),
        logical_length: Some(logical_length),
        capacity: Some(capacity),
        untouched_tail_count: Some(untouched_tail_count as u32),
        translation_path,
        result: "pass-artifact-compute-differential",
    })
}

/// Executes a validated three-dimensional affine-stride JIR storage-write
/// artifact through native DX12. The narrow fallback accepts only the
/// verified eight-resource `GlobalInvocationId.x/y/z` shape before emitting
/// HLSL.
pub fn run_global_3d_strided_write_artifact_smoke(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    config: Global3dStridedWriteArtifactConfig,
) -> Result<DirectX12ArtifactExecutionReport, DirectX12Error> {
    let Global3dStridedWriteArtifactConfig {
        value,
        width,
        height,
        depth,
        stride_x,
        stride_y,
        stride_z,
        capacity,
    } = config;
    let logical_length = width
        .checked_mul(height)
        .and_then(|area| area.checked_mul(depth))
        .ok_or(DirectX12Error::ArtifactContract(
            "global-3d-strided-write logical length overflows u32",
        ))?;
    if width == 0
        || height == 0
        || depth == 0
        || stride_x == 0
        || stride_y == 0
        || stride_z == 0
        || capacity == 0
    {
        return Err(DirectX12Error::ArtifactContract(
            "global-3d-strided-write fixture requires non-zero dimensions, strides and capacity",
        ));
    }
    let last_physical_index = (width - 1)
        .checked_mul(stride_x)
        .and_then(|x_offset| {
            (height - 1)
                .checked_mul(stride_y)
                .and_then(|y_offset| x_offset.checked_add(y_offset))
        })
        .and_then(|xy_offset| {
            (depth - 1)
                .checked_mul(stride_z)
                .and_then(|z_offset| xy_offset.checked_add(z_offset))
        })
        .ok_or(DirectX12Error::ArtifactContract(
            "global-3d-strided-write physical index overflows u32",
        ))?;
    if last_physical_index >= capacity {
        return Err(DirectX12Error::ArtifactContract(
            "global-3d-strided-write physical range exceeds capacity",
        ));
    }
    let artifact =
        emit_storage_global_3d_strided_write_artifact_from_jir(module, function, options)
            .map_err(|error| DirectX12Error::JirSpirvLowering(error.to_string()))?;
    validate_shared_artifact(&artifact)?;
    validate_global_3d_strided_write_artifact_contract(&artifact, value)?;
    let [workgroup_x, workgroup_y, workgroup_z] = artifact.workgroup_size;
    if workgroup_x == 0 || workgroup_y == 0 || workgroup_z == 0 {
        return Err(DirectX12Error::ArtifactContract(
            "global-3d-strided-write artifact requires a three-dimensional workgroup",
        ));
    }
    let (dxil, translation_path) = match translate_spirv_artifact_to_dxil(&artifact) {
        Ok(dxil) => (dxil, "spirv-cross-dxc"),
        Err(
            DirectX12Error::SpirvToolchainUnavailable
            | DirectX12Error::SpirvCrossUnavailable
            | DirectX12Error::ShaderToolchainUnavailable,
        ) => (
            compile_global_3d_strided_write_artifact_dxil(&artifact, value)?,
            "jadren-artifact-narrow-hlsl-to-dxil",
        ),
        Err(error) => return Err(error),
    };
    let dispatch_x = width.div_ceil(workgroup_x);
    let dispatch_y = height.div_ceil(workgroup_y);
    let dispatch_z = depth.div_ceil(workgroup_z);
    let capacity_usize = usize::try_from(capacity).map_err(|_| {
        DirectX12Error::ArtifactContract("global-3d-strided-write capacity does not fit host usize")
    })?;
    let bindings = vec![
        vec![0_u32; capacity_usize],
        vec![width],
        vec![height],
        vec![depth],
        vec![stride_x],
        vec![stride_y],
        vec![stride_z],
        vec![capacity],
    ];
    let actual_values = run_prepared_u32_uav_artifact(
        &artifact,
        dxil,
        [dispatch_x, dispatch_y, dispatch_z],
        &bindings,
        0,
    )?;
    let mut expected_values = vec![0_u32; capacity_usize];
    for z in 0..depth {
        for y in 0..height {
            for x in 0..width {
                let physical_index = x * stride_x + y * stride_y + z * stride_z;
                expected_values[physical_index as usize] = value;
            }
        }
    }
    for (index, (actual, expected)) in actual_values.iter().zip(expected_values.iter()).enumerate()
    {
        if actual != expected {
            return Err(DirectX12Error::DifferentialMismatch {
                index: index as u32,
                actual: *actual,
                expected: *expected,
            });
        }
    }
    let untouched_tail_count = actual_values.iter().filter(|value| **value == 0).count();
    let expected_untouched_count = expected_values.iter().filter(|value| **value == 0).count();
    if untouched_tail_count != expected_untouched_count {
        return Err(DirectX12Error::ArtifactContract(
            "global-3d-strided-write artifact modified an unwritten physical element",
        ));
    }
    Ok(DirectX12ArtifactExecutionReport {
        schema: "jadren-directx12-artifact-execution-0.1",
        operation: "global-3d-strided-write",
        entry_name: artifact.entry_name,
        artifact_resource_binding_count: artifact.resources.len(),
        artifact_word_count: artifact.words.len(),
        artifact_word_hash: stable_spirv_word_hash(&artifact.words),
        artifact_validated: true,
        dxil_translated: true,
        execution_completed: true,
        output_checksum: actual_values.iter().map(|value| u64::from(*value)).sum(),
        first_output: actual_values.first().copied(),
        last_output: actual_values.get(last_physical_index as usize).copied(),
        logical_length: Some(logical_length),
        capacity: Some(capacity),
        untouched_tail_count: Some(untouched_tail_count as u32),
        translation_path,
        result: "pass-artifact-compute-differential",
    })
}

fn compile_storage_add_artifact_dxil(
    artifact: &SpirvArtifact,
    addend: u32,
) -> Result<Vec<u8>, DirectX12Error> {
    validate_storage_add_artifact_contract(artifact, addend)?;
    let [x, y, z] = artifact.workgroup_size;
    if x == 0 || y == 0 || z == 0 {
        return Err(DirectX12Error::ArtifactContract(
            "storage-add artifact has an invalid workgroup",
        ));
    }
    if !valid_shader_entry(&artifact.entry_name) {
        return Err(DirectX12Error::InvalidShaderEntry);
    }
    let source = format!(
        r#"RWStructuredBuffer<uint> Data : register(u0);

[RootSignature("DescriptorTable(UAV(u0, numDescriptors=1))")]
[numthreads({x}, {y}, {z})]
void {entry_name}(uint3 id : SV_DispatchThreadID) {{
    if (id.x == 0 && id.y == 0 && id.z == 0) {{
        Data[0] = Data[0] + {addend}u;
    }}
}}
"#,
        x = x,
        y = y,
        z = z,
        entry_name = artifact.entry_name,
        addend = addend,
    );
    compile_hlsl_source_with_entry(&source, &artifact.entry_name)
}

fn validate_storage_add_artifact_contract(
    artifact: &SpirvArtifact,
    addend: u32,
) -> Result<(), DirectX12Error> {
    if artifact.resources.len() != 1 {
        return Err(DirectX12Error::ArtifactContract(
            "storage-add artifact requires one resource",
        ));
    }
    let mut constants = Vec::new();
    let mut found_add = false;
    let mut cursor = 5_usize;
    while cursor < artifact.words.len() {
        let instruction = artifact.words[cursor];
        let word_count = (instruction >> 16) as usize;
        let opcode = (instruction & 0xFFFF) as u16;
        if word_count == 0 || cursor + word_count > artifact.words.len() {
            return Err(DirectX12Error::ArtifactContract(
                "storage-add SPIR-V instruction stream is truncated",
            ));
        }
        let operands = &artifact.words[cursor + 1..cursor + word_count];
        if opcode == SPIRV_OP_CONSTANT {
            if operands.len() < 3 {
                return Err(DirectX12Error::ArtifactContract(
                    "storage-add OpConstant instruction is incomplete",
                ));
            }
            constants.push((operands[1], operands[2]));
        } else if opcode == SPIRV_OP_IADD {
            if operands.len() < 4 {
                return Err(DirectX12Error::ArtifactContract(
                    "storage-add OpIAdd instruction is incomplete",
                ));
            }
            let constant_id = operands[3];
            let Some((_, encoded_addend)) = constants.iter().find(|(id, _)| *id == constant_id)
            else {
                return Err(DirectX12Error::ArtifactContract(
                    "storage-add addend is not defined before use",
                ));
            };
            if *encoded_addend != addend {
                return Err(DirectX12Error::ArtifactContract(
                    "storage-add artifact addend differs from execution request",
                ));
            }
            if found_add {
                return Err(DirectX12Error::ArtifactContract(
                    "storage-add artifact contains multiple additions",
                ));
            }
            found_add = true;
        }
        cursor += word_count;
    }
    if !found_add {
        return Err(DirectX12Error::ArtifactContract(
            "storage-add artifact does not contain OpIAdd",
        ));
    }
    Ok(())
}

fn compile_global_write_artifact_dxil(
    artifact: &SpirvArtifact,
    value: u32,
    length: u32,
) -> Result<Vec<u8>, DirectX12Error> {
    validate_global_write_artifact_contract(artifact, value, length)?;
    let [x, y, z] = artifact.workgroup_size;
    if x == 0 || y != 1 || z != 1 {
        return Err(DirectX12Error::ArtifactContract(
            "global-write artifact has an invalid one-dimensional workgroup",
        ));
    }
    if !valid_shader_entry(&artifact.entry_name) {
        return Err(DirectX12Error::InvalidShaderEntry);
    }
    let source = format!(
        r#"RWStructuredBuffer<uint> Data : register(u0);

[RootSignature("DescriptorTable(UAV(u0, numDescriptors=1))")]
[numthreads({x}, 1, 1)]
void {entry_name}(uint3 id : SV_DispatchThreadID) {{
    if (id.x < {length}u) {{
        Data[id.x] = {value}u;
    }}
}}
"#,
        x = x,
        entry_name = artifact.entry_name,
        length = length,
        value = value,
    );
    compile_hlsl_source_with_entry(&source, &artifact.entry_name)
}

fn validate_global_write_artifact_contract(
    artifact: &SpirvArtifact,
    value: u32,
    length: u32,
) -> Result<(), DirectX12Error> {
    if artifact.resources.len() != 1 {
        return Err(DirectX12Error::ArtifactContract(
            "global-write artifact requires one resource",
        ));
    }
    let mut constants = Vec::new();
    let mut found_bounds = false;
    let mut found_store = false;
    let mut cursor = 5_usize;
    while cursor < artifact.words.len() {
        let instruction = artifact.words[cursor];
        let word_count = (instruction >> 16) as usize;
        let opcode = (instruction & 0xFFFF) as u16;
        if word_count == 0 || cursor + word_count > artifact.words.len() {
            return Err(DirectX12Error::ArtifactContract(
                "global-write SPIR-V instruction stream is truncated",
            ));
        }
        let operands = &artifact.words[cursor + 1..cursor + word_count];
        match opcode {
            SPIRV_OP_CONSTANT => {
                if operands.len() < 3 {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-write OpConstant instruction is incomplete",
                    ));
                }
                constants.push((operands[1], operands[2]));
            }
            SPIRV_OP_ULT => {
                if operands.len() < 4 || found_bounds {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-write artifact must contain exactly one OpULessThan",
                    ));
                }
                let bound_id = operands[3];
                let Some((_, encoded_length)) = constants.iter().find(|(id, _)| *id == bound_id)
                else {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-write bounds constant is not defined before use",
                    ));
                };
                if *encoded_length != length {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-write artifact length differs from execution request",
                    ));
                }
                found_bounds = true;
            }
            SPIRV_OP_STORE => {
                if operands.len() < 2 || found_store {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-write artifact must contain exactly one OpStore",
                    ));
                }
                let value_id = operands[1];
                let Some((_, encoded_value)) = constants.iter().find(|(id, _)| *id == value_id)
                else {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-write stored constant is not defined before use",
                    ));
                };
                if *encoded_value != value {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-write artifact value differs from execution request",
                    ));
                }
                found_store = true;
            }
            _ => {}
        }
        cursor += word_count;
    }
    if !found_bounds || !found_store {
        return Err(DirectX12Error::ArtifactContract(
            "global-write artifact is missing its bounds branch or store",
        ));
    }
    Ok(())
}

fn compile_global_strided_write_artifact_dxil(
    artifact: &SpirvArtifact,
    value: u32,
) -> Result<Vec<u8>, DirectX12Error> {
    validate_global_strided_write_artifact_contract(artifact, value)?;
    let [x, y, z] = artifact.workgroup_size;
    if x == 0 || y != 1 || z != 1 {
        return Err(DirectX12Error::ArtifactContract(
            "global-strided-write artifact has an invalid one-dimensional workgroup",
        ));
    }
    if !valid_shader_entry(&artifact.entry_name) {
        return Err(DirectX12Error::InvalidShaderEntry);
    }
    let source = format!(
        r#"RWStructuredBuffer<uint> Data : register(u0);
RWStructuredBuffer<uint> Length : register(u1);
RWStructuredBuffer<uint> Stride : register(u2);
RWStructuredBuffer<uint> Capacity : register(u3);

[RootSignature("DescriptorTable(UAV(u0, numDescriptors=4))")]
[numthreads({x}, 1, 1)]
void {entry_name}(uint3 id : SV_DispatchThreadID) {{
    uint length = Length[0];
    uint stride = Stride[0];
    uint capacity = Capacity[0];
    if (id.x < length) {{
        uint index = id.x * stride;
        if (index < capacity) {{
            Data[index] = {value}u;
        }}
    }}
}}
"#,
        x = x,
        entry_name = artifact.entry_name,
        value = value,
    );
    compile_hlsl_source_with_entry(&source, &artifact.entry_name)
}

fn validate_global_strided_write_artifact_contract(
    artifact: &SpirvArtifact,
    value: u32,
) -> Result<(), DirectX12Error> {
    if artifact.resources.len() != 4 {
        return Err(DirectX12Error::ArtifactContract(
            "global-strided-write artifact requires four resources",
        ));
    }
    let mut constants = Vec::new();
    let mut bounds_count = 0_usize;
    let mut multiply_count = 0_usize;
    let mut store_count = 0_usize;
    let mut cursor = 5_usize;
    while cursor < artifact.words.len() {
        let instruction = artifact.words[cursor];
        let word_count = (instruction >> 16) as usize;
        let opcode = (instruction & 0xFFFF) as u16;
        if word_count == 0 || cursor + word_count > artifact.words.len() {
            return Err(DirectX12Error::ArtifactContract(
                "global-strided-write SPIR-V instruction stream is truncated",
            ));
        }
        let operands = &artifact.words[cursor + 1..cursor + word_count];
        match opcode {
            SPIRV_OP_CONSTANT => {
                if operands.len() < 3 {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-strided-write OpConstant instruction is incomplete",
                    ));
                }
                constants.push((operands[1], operands[2]));
            }
            SPIRV_OP_ULT => bounds_count += 1,
            SPIRV_OP_IMUL => multiply_count += 1,
            SPIRV_OP_STORE => {
                if operands.len() < 2 || store_count != 0 {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-strided-write artifact must contain exactly one OpStore",
                    ));
                }
                let value_id = operands[1];
                let Some((_, encoded_value)) = constants.iter().find(|(id, _)| *id == value_id)
                else {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-strided-write stored constant is not defined before use",
                    ));
                };
                if *encoded_value != value {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-strided-write artifact value differs from execution request",
                    ));
                }
                store_count += 1;
            }
            _ => {}
        }
        cursor += word_count;
    }
    if bounds_count != 2 || multiply_count != 1 || store_count != 1 {
        return Err(DirectX12Error::ArtifactContract(
            "global-strided-write artifact is missing its logical/physical bounds or store",
        ));
    }
    Ok(())
}

fn compile_global_2d_write_artifact_dxil(
    artifact: &SpirvArtifact,
    value: u32,
) -> Result<Vec<u8>, DirectX12Error> {
    validate_global_2d_write_artifact_contract(artifact, value)?;
    let [x, y, z] = artifact.workgroup_size;
    if x == 0 || y == 0 || z != 1 {
        return Err(DirectX12Error::ArtifactContract(
            "global-2d-write artifact has an invalid two-dimensional workgroup",
        ));
    }
    if !valid_shader_entry(&artifact.entry_name) {
        return Err(DirectX12Error::InvalidShaderEntry);
    }
    let source = format!(
        r#"RWStructuredBuffer<uint> Data : register(u0);
RWStructuredBuffer<uint> Width : register(u1);
RWStructuredBuffer<uint> Height : register(u2);
RWStructuredBuffer<uint> Capacity : register(u3);

[RootSignature("DescriptorTable(UAV(u0, numDescriptors=4))")]
[numthreads({x}, {y}, 1)]
void {entry_name}(uint3 id : SV_DispatchThreadID) {{
    uint width = Width[0];
    uint height = Height[0];
    uint capacity = Capacity[0];
    if (id.x < width && id.y < height) {{
        uint index = id.y * width + id.x;
        if (index < capacity) {{
            Data[index] = {value}u;
        }}
    }}
}}
"#,
        x = x,
        y = y,
        entry_name = artifact.entry_name,
        value = value,
    );
    compile_hlsl_source_with_entry(&source, &artifact.entry_name)
}

fn validate_global_2d_write_artifact_contract(
    artifact: &SpirvArtifact,
    value: u32,
) -> Result<(), DirectX12Error> {
    if artifact.resources.len() != 4 {
        return Err(DirectX12Error::ArtifactContract(
            "global-2d-write artifact requires four resources",
        ));
    }
    let mut constants = Vec::new();
    let mut bounds_count = 0_usize;
    let mut multiply_count = 0_usize;
    let mut add_count = 0_usize;
    let mut store_count = 0_usize;
    let mut cursor = 5_usize;
    while cursor < artifact.words.len() {
        let instruction = artifact.words[cursor];
        let word_count = (instruction >> 16) as usize;
        let opcode = (instruction & 0xFFFF) as u16;
        if word_count == 0 || cursor + word_count > artifact.words.len() {
            return Err(DirectX12Error::ArtifactContract(
                "global-2d-write SPIR-V instruction stream is truncated",
            ));
        }
        let operands = &artifact.words[cursor + 1..cursor + word_count];
        match opcode {
            SPIRV_OP_CONSTANT => {
                if operands.len() < 3 {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-2d-write OpConstant instruction is incomplete",
                    ));
                }
                constants.push((operands[1], operands[2]));
            }
            SPIRV_OP_ULT => bounds_count += 1,
            SPIRV_OP_IMUL => multiply_count += 1,
            SPIRV_OP_IADD => add_count += 1,
            SPIRV_OP_STORE => {
                if operands.len() < 2 || store_count != 0 {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-2d-write artifact must contain exactly one OpStore",
                    ));
                }
                let value_id = operands[1];
                let Some((_, encoded_value)) = constants.iter().find(|(id, _)| *id == value_id)
                else {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-2d-write stored constant is not defined before use",
                    ));
                };
                if *encoded_value != value {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-2d-write artifact value differs from execution request",
                    ));
                }
                store_count += 1;
            }
            _ => {}
        }
        cursor += word_count;
    }
    if bounds_count != 3 || multiply_count != 1 || add_count != 1 || store_count != 1 {
        return Err(DirectX12Error::ArtifactContract(
            "global-2d-write artifact is missing its coordinate/capacity bounds or row-major store",
        ));
    }
    Ok(())
}

fn compile_global_2d_strided_write_artifact_dxil(
    artifact: &SpirvArtifact,
    value: u32,
) -> Result<Vec<u8>, DirectX12Error> {
    validate_global_2d_strided_write_artifact_contract(artifact, value)?;
    let [x, y, z] = artifact.workgroup_size;
    if x == 0 || y == 0 || z != 1 {
        return Err(DirectX12Error::ArtifactContract(
            "global-2d-strided-write artifact has an invalid two-dimensional workgroup",
        ));
    }
    if !valid_shader_entry(&artifact.entry_name) {
        return Err(DirectX12Error::InvalidShaderEntry);
    }
    let source = format!(
        r#"RWStructuredBuffer<uint> Data : register(u0);
RWStructuredBuffer<uint> Width : register(u1);
RWStructuredBuffer<uint> Height : register(u2);
RWStructuredBuffer<uint> StrideX : register(u3);
RWStructuredBuffer<uint> StrideY : register(u4);
RWStructuredBuffer<uint> Capacity : register(u5);

[RootSignature("DescriptorTable(UAV(u0, numDescriptors=6))")]
[numthreads({x}, {y}, 1)]
void {entry_name}(uint3 id : SV_DispatchThreadID) {{
    uint width = Width[0];
    uint height = Height[0];
    uint strideX = StrideX[0];
    uint strideY = StrideY[0];
    uint capacity = Capacity[0];
    if (id.x < width && id.y < height) {{
        uint index = id.x * strideX + id.y * strideY;
        if (index < capacity) {{
            Data[index] = {value}u;
        }}
    }}
}}
"#,
        x = x,
        y = y,
        entry_name = artifact.entry_name,
        value = value,
    );
    compile_hlsl_source_with_entry(&source, &artifact.entry_name)
}

fn validate_global_2d_strided_write_artifact_contract(
    artifact: &SpirvArtifact,
    value: u32,
) -> Result<(), DirectX12Error> {
    if artifact.resources.len() != 6 {
        return Err(DirectX12Error::ArtifactContract(
            "global-2d-strided-write artifact requires six resources",
        ));
    }
    let mut constants = Vec::new();
    let mut bounds_count = 0_usize;
    let mut multiply_count = 0_usize;
    let mut add_count = 0_usize;
    let mut store_count = 0_usize;
    let mut cursor = 5_usize;
    while cursor < artifact.words.len() {
        let instruction = artifact.words[cursor];
        let word_count = (instruction >> 16) as usize;
        let opcode = (instruction & 0xFFFF) as u16;
        if word_count == 0 || cursor + word_count > artifact.words.len() {
            return Err(DirectX12Error::ArtifactContract(
                "global-2d-strided-write SPIR-V instruction stream is truncated",
            ));
        }
        let operands = &artifact.words[cursor + 1..cursor + word_count];
        match opcode {
            SPIRV_OP_CONSTANT => {
                if operands.len() < 3 {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-2d-strided-write OpConstant instruction is incomplete",
                    ));
                }
                constants.push((operands[1], operands[2]));
            }
            SPIRV_OP_ULT => bounds_count += 1,
            SPIRV_OP_IMUL => multiply_count += 1,
            SPIRV_OP_IADD => add_count += 1,
            SPIRV_OP_STORE => {
                if operands.len() < 2 || store_count != 0 {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-2d-strided-write artifact must contain exactly one OpStore",
                    ));
                }
                let value_id = operands[1];
                let Some((_, encoded_value)) = constants.iter().find(|(id, _)| *id == value_id)
                else {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-2d-strided-write stored constant is not defined before use",
                    ));
                };
                if *encoded_value != value {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-2d-strided-write artifact value differs from execution request",
                    ));
                }
                store_count += 1;
            }
            _ => {}
        }
        cursor += word_count;
    }
    if bounds_count != 3 || multiply_count != 2 || add_count != 1 || store_count != 1 {
        return Err(DirectX12Error::ArtifactContract(
            "global-2d-strided-write artifact is missing its affine bounds or store",
        ));
    }
    Ok(())
}

fn compile_global_3d_write_artifact_dxil(
    artifact: &SpirvArtifact,
    value: u32,
) -> Result<Vec<u8>, DirectX12Error> {
    validate_global_3d_write_artifact_contract(artifact, value)?;
    let [x, y, z] = artifact.workgroup_size;
    if x == 0 || y == 0 || z == 0 {
        return Err(DirectX12Error::ArtifactContract(
            "global-3d-write artifact has an invalid three-dimensional workgroup",
        ));
    }
    if !valid_shader_entry(&artifact.entry_name) {
        return Err(DirectX12Error::InvalidShaderEntry);
    }
    let source = format!(
        r#"RWStructuredBuffer<uint> Data : register(u0);
RWStructuredBuffer<uint> Width : register(u1);
RWStructuredBuffer<uint> Height : register(u2);
RWStructuredBuffer<uint> Depth : register(u3);
RWStructuredBuffer<uint> Capacity : register(u4);

[RootSignature("DescriptorTable(UAV(u0, numDescriptors=5))")]
[numthreads({x}, {y}, {z})]
void {entry_name}(uint3 id : SV_DispatchThreadID) {{
    uint width = Width[0];
    uint height = Height[0];
    uint depth = Depth[0];
    uint capacity = Capacity[0];
    if (id.x < width && id.y < height && id.z < depth) {{
        uint index = (id.z * height + id.y) * width + id.x;
        if (index < capacity) {{
            Data[index] = {value}u;
        }}
    }}
}}
"#,
        x = x,
        y = y,
        z = z,
        entry_name = artifact.entry_name,
        value = value,
    );
    compile_hlsl_source_with_entry(&source, &artifact.entry_name)
}

fn validate_global_3d_write_artifact_contract(
    artifact: &SpirvArtifact,
    value: u32,
) -> Result<(), DirectX12Error> {
    if artifact.resources.len() != 5 {
        return Err(DirectX12Error::ArtifactContract(
            "global-3d-write artifact requires five resources",
        ));
    }
    let mut constants = Vec::new();
    let mut bounds_count = 0_usize;
    let mut multiply_count = 0_usize;
    let mut add_count = 0_usize;
    let mut store_count = 0_usize;
    let mut cursor = 5_usize;
    while cursor < artifact.words.len() {
        let instruction = artifact.words[cursor];
        let word_count = (instruction >> 16) as usize;
        let opcode = (instruction & 0xFFFF) as u16;
        if word_count == 0 || cursor + word_count > artifact.words.len() {
            return Err(DirectX12Error::ArtifactContract(
                "global-3d-write SPIR-V instruction stream is truncated",
            ));
        }
        let operands = &artifact.words[cursor + 1..cursor + word_count];
        match opcode {
            SPIRV_OP_CONSTANT => {
                if operands.len() < 3 {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-3d-write OpConstant instruction is incomplete",
                    ));
                }
                constants.push((operands[1], operands[2]));
            }
            SPIRV_OP_ULT => bounds_count += 1,
            SPIRV_OP_IMUL => multiply_count += 1,
            SPIRV_OP_IADD => add_count += 1,
            SPIRV_OP_STORE => {
                if operands.len() < 2 || store_count != 0 {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-3d-write artifact must contain exactly one OpStore",
                    ));
                }
                let value_id = operands[1];
                let Some((_, encoded_value)) = constants.iter().find(|(id, _)| *id == value_id)
                else {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-3d-write stored constant is not defined before use",
                    ));
                };
                if *encoded_value != value {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-3d-write artifact value differs from execution request",
                    ));
                }
                store_count += 1;
            }
            _ => {}
        }
        cursor += word_count;
    }
    if bounds_count != 4 || multiply_count != 2 || add_count != 2 || store_count != 1 {
        return Err(DirectX12Error::ArtifactContract(
            "global-3d-write artifact is missing its coordinate/capacity bounds or row-major store",
        ));
    }
    Ok(())
}

fn compile_global_3d_strided_write_artifact_dxil(
    artifact: &SpirvArtifact,
    value: u32,
) -> Result<Vec<u8>, DirectX12Error> {
    validate_global_3d_strided_write_artifact_contract(artifact, value)?;
    let [x, y, z] = artifact.workgroup_size;
    if x == 0 || y == 0 || z == 0 {
        return Err(DirectX12Error::ArtifactContract(
            "global-3d-strided-write artifact has an invalid three-dimensional workgroup",
        ));
    }
    if !valid_shader_entry(&artifact.entry_name) {
        return Err(DirectX12Error::InvalidShaderEntry);
    }
    let source = format!(
        r#"RWStructuredBuffer<uint> Data : register(u0);
RWStructuredBuffer<uint> Width : register(u1);
RWStructuredBuffer<uint> Height : register(u2);
RWStructuredBuffer<uint> Depth : register(u3);
RWStructuredBuffer<uint> StrideX : register(u4);
RWStructuredBuffer<uint> StrideY : register(u5);
RWStructuredBuffer<uint> StrideZ : register(u6);
RWStructuredBuffer<uint> Capacity : register(u7);

[RootSignature("DescriptorTable(UAV(u0, numDescriptors=8))")]
[numthreads({x}, {y}, {z})]
void {entry_name}(uint3 id : SV_DispatchThreadID) {{
    uint width = Width[0];
    uint height = Height[0];
    uint depth = Depth[0];
    uint strideX = StrideX[0];
    uint strideY = StrideY[0];
    uint strideZ = StrideZ[0];
    uint capacity = Capacity[0];
    if (id.x < width && id.y < height && id.z < depth) {{
        uint index = id.x * strideX + id.y * strideY + id.z * strideZ;
        if (index < capacity) {{
            Data[index] = {value}u;
        }}
    }}
}}
"#,
        x = x,
        y = y,
        z = z,
        entry_name = artifact.entry_name,
        value = value,
    );
    compile_hlsl_source_with_entry(&source, &artifact.entry_name)
}

fn validate_global_3d_strided_write_artifact_contract(
    artifact: &SpirvArtifact,
    value: u32,
) -> Result<(), DirectX12Error> {
    if artifact.resources.len() != 8 {
        return Err(DirectX12Error::ArtifactContract(
            "global-3d-strided-write artifact requires eight resources",
        ));
    }
    let mut constants = Vec::new();
    let mut bounds_count = 0_usize;
    let mut multiply_count = 0_usize;
    let mut add_count = 0_usize;
    let mut store_count = 0_usize;
    let mut cursor = 5_usize;
    while cursor < artifact.words.len() {
        let instruction = artifact.words[cursor];
        let word_count = (instruction >> 16) as usize;
        let opcode = (instruction & 0xFFFF) as u16;
        if word_count == 0 || cursor + word_count > artifact.words.len() {
            return Err(DirectX12Error::ArtifactContract(
                "global-3d-strided-write SPIR-V instruction stream is truncated",
            ));
        }
        let operands = &artifact.words[cursor + 1..cursor + word_count];
        match opcode {
            SPIRV_OP_CONSTANT => {
                if operands.len() < 3 {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-3d-strided-write OpConstant instruction is incomplete",
                    ));
                }
                constants.push((operands[1], operands[2]));
            }
            SPIRV_OP_ULT => bounds_count += 1,
            SPIRV_OP_IMUL => multiply_count += 1,
            SPIRV_OP_IADD => add_count += 1,
            SPIRV_OP_STORE => {
                if operands.len() < 2 || store_count != 0 {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-3d-strided-write artifact must contain exactly one OpStore",
                    ));
                }
                let value_id = operands[1];
                let Some((_, encoded_value)) = constants.iter().find(|(id, _)| *id == value_id)
                else {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-3d-strided-write stored constant is not defined before use",
                    ));
                };
                if *encoded_value != value {
                    return Err(DirectX12Error::ArtifactContract(
                        "global-3d-strided-write artifact value differs from execution request",
                    ));
                }
                store_count += 1;
            }
            _ => {}
        }
        cursor += word_count;
    }
    if bounds_count != 4 || multiply_count != 3 || add_count != 2 || store_count != 1 {
        return Err(DirectX12Error::ArtifactContract(
            "global-3d-strided-write artifact is missing its affine bounds or store",
        ));
    }
    Ok(())
}

fn run_dynamic_uav_dxil_with_context(
    context: &DxContext,
    dxil: Vec<u8>,
    bindings: &[UavBindingPayload<'_>],
    output_binding: usize,
    dispatch_workgroups: [u32; 3],
    views: &[DxilUavView],
) -> Result<Vec<u8>, DirectX12Error> {
    if views.len() != bindings.len() {
        return Err(DirectX12Error::ArtifactContract(
            "generic DX12 UAV view count differs from binding count",
        ));
    }
    let byte_sizes = validate_dynamic_uav_request(
        bindings.len(),
        dispatch_workgroups,
        bindings,
        output_binding,
    )?;
    let binding_count = u32::try_from(bindings.len()).map_err(|_| {
        DirectX12Error::ArtifactContract("generic DX12 UAV binding count exceeds u32")
    })?;
    let default_heap = HeapProperties {
        heap_type: D3D12_HEAP_TYPE_DEFAULT,
        cpu_page_property: 0,
        memory_pool_preference: 0,
        creation_node_mask: 1,
        visible_node_mask: 1,
    };
    let upload_heap = HeapProperties {
        heap_type: D3D12_HEAP_TYPE_UPLOAD,
        ..default_heap
    };
    let readback_heap = HeapProperties {
        heap_type: D3D12_HEAP_TYPE_READBACK,
        ..default_heap
    };
    let mut uploads = Vec::with_capacity(bindings.len());
    let mut resources = Vec::with_capacity(bindings.len());
    for ((binding, layout), view) in bindings
        .iter()
        .zip(byte_sizes.iter().copied())
        .zip(views.iter().copied())
    {
        let bytes_usize = usize::try_from(layout.byte_size).map_err(|_| {
            DirectX12Error::ArtifactContract("generic DX12 UAV resource size exceeds host usize")
        })?;
        let upload_desc = buffer_desc(bytes_usize, 0);
        let data_flags = if dxil_view_is_writable(view) {
            D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS
        } else {
            0
        };
        let data_desc = buffer_desc(bytes_usize, data_flags);
        let upload = create_resource(
            context.device.as_ptr(),
            &upload_heap,
            &upload_desc,
            D3D12_RESOURCE_STATE_GENERIC_READ,
            "CreateCommittedResource(generic_uav_upload)",
        )?;
        write_resource_bytes(context.device.as_ptr(), upload.as_ptr(), binding.bytes)?;
        let resource = create_resource(
            context.device.as_ptr(),
            &default_heap,
            &data_desc,
            D3D12_RESOURCE_STATE_COPY_DEST,
            "CreateCommittedResource(generic_uav)",
        )?;
        uploads.push(upload);
        resources.push(resource);
    }
    let output_bytes = byte_sizes[output_binding].byte_size;
    let output_bytes_usize = usize::try_from(output_bytes).map_err(|_| {
        DirectX12Error::ArtifactContract("generic DX12 UAV output size exceeds host usize")
    })?;
    let readback_desc = buffer_desc(output_bytes_usize, 0);
    let readback = create_resource(
        context.device.as_ptr(),
        &readback_heap,
        &readback_desc,
        D3D12_RESOURCE_STATE_COPY_DEST,
        "CreateCommittedResource(generic_uav_readback)",
    )?;
    let descriptor_heap = create_descriptor_heap(context.device.as_ptr(), binding_count)?;
    let descriptor_increment = unsafe {
        vtable_method::<GetDescriptorHandleIncrementSize>(context.device.as_ptr(), 15)(
            context.device.as_ptr(),
            D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
        )
    };
    if descriptor_increment == 0 {
        return Err(DirectX12Error::HResult {
            operation: "ID3D12Device::GetDescriptorHandleIncrementSize(generic_uav)",
            code: -1,
        });
    }
    let mut cpu_start = CpuDescriptorHandle { ptr: 0 };
    let mut gpu_start = GpuDescriptorHandle { ptr: 0 };
    unsafe {
        vtable_method::<GetCpuDescriptorHandleForHeapStart>(descriptor_heap.as_ptr(), 9)(
            descriptor_heap.as_ptr(),
            &mut cpu_start,
        );
        vtable_method::<GetGpuDescriptorHandleForHeapStart>(descriptor_heap.as_ptr(), 10)(
            descriptor_heap.as_ptr(),
            &mut gpu_start,
        );
    }
    for (index, resource) in resources.iter().enumerate() {
        let (format, num_elements, structure_byte_stride, flags) = match views[index] {
            DxilUavView::Structured | DxilUavView::StructuredSrv => (
                DXGI_FORMAT_UNKNOWN,
                byte_sizes[index].element_count,
                byte_sizes[index].element_stride,
                0,
            ),
            DxilUavView::Raw | DxilUavView::RawSrv => {
                let num_elements =
                    u32::try_from(byte_sizes[index].byte_size / 4).map_err(|_| {
                        DirectX12Error::ArtifactContract(
                            "generic DX12 raw UAV byte count exceeds u32",
                        )
                    })?;
                if byte_sizes[index].byte_size % 4 != 0 || num_elements == 0 {
                    return Err(DirectX12Error::ArtifactContract(
                        "generic DX12 raw UAV payload must be a non-empty multiple of four bytes",
                    ));
                }
                let flags = if matches!(views[index], DxilUavView::RawSrv) {
                    D3D12_BUFFER_SRV_FLAG_RAW
                } else {
                    D3D12_BUFFER_UAV_FLAG_RAW
                };
                (DXGI_FORMAT_R32_TYPELESS, num_elements, 0, flags)
            }
        };
        let destination = CpuDescriptorHandle {
            ptr: cpu_start.ptr + descriptor_increment as usize * index,
        };
        if dxil_view_is_writable(views[index]) {
            let descriptor = UnorderedAccessViewDesc {
                format,
                view_dimension: D3D12_UAV_DIMENSION_BUFFER,
                data: UnorderedAccessViewData {
                    buffer: UnorderedAccessViewBuffer {
                        first_element: 0,
                        num_elements,
                        structure_byte_stride,
                        counter_offset_in_bytes: 0,
                        flags,
                    },
                },
            };
            unsafe {
                vtable_method::<CreateUnorderedAccessView>(context.device.as_ptr(), 19)(
                    context.device.as_ptr(),
                    resource.as_ptr(),
                    std::ptr::null_mut(),
                    &descriptor,
                    destination,
                );
            }
        } else {
            let descriptor = ShaderResourceViewDesc {
                format,
                view_dimension: D3D12_SRV_DIMENSION_BUFFER,
                shader4_component_mapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                data: ShaderResourceViewData {
                    buffer: ShaderResourceViewBuffer {
                        first_element: 0,
                        num_elements,
                        structure_byte_stride,
                        flags,
                    },
                },
            };
            unsafe {
                vtable_method::<CreateShaderResourceView>(context.device.as_ptr(), 18)(
                    context.device.as_ptr(),
                    resource.as_ptr(),
                    &descriptor,
                    destination,
                );
            }
        }
    }
    let root_signature =
        create_native_resource_root_signature(&context._library, context.device.as_ptr(), views)?;
    let pipeline =
        create_compute_pipeline(context.device.as_ptr(), &dxil, root_signature.as_ptr())?;
    let allocator = create_command_allocator(context.device.as_ptr())?;
    let command_list = create_command_list(context.device.as_ptr(), allocator.as_ptr())?;
    unsafe {
        for ((resource, upload), layout) in resources
            .iter()
            .zip(uploads.iter())
            .zip(byte_sizes.iter().copied())
        {
            vtable_method::<CopyBufferRegion>(command_list.as_ptr(), 15)(
                command_list.as_ptr(),
                resource.as_ptr(),
                0,
                upload.as_ptr(),
                0,
                layout.byte_size,
            );
        }
    }
    let to_native = resources
        .iter()
        .enumerate()
        .map(|(index, resource)| ResourceBarrier {
            barrier_type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            transition: ResourceTransition {
                resource: resource.as_ptr(),
                subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                state_before: D3D12_RESOURCE_STATE_COPY_DEST,
                state_after: if dxil_view_is_writable(views[index]) {
                    D3D12_RESOURCE_STATE_UNORDERED_ACCESS
                } else {
                    D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE
                },
            },
        })
        .collect::<Vec<_>>();
    unsafe {
        vtable_method::<ResourceBarrierFn>(command_list.as_ptr(), 26)(
            command_list.as_ptr(),
            to_native.len() as u32,
            to_native.as_ptr(),
        );
        let heaps = [descriptor_heap.as_ptr()];
        vtable_method::<SetDescriptorHeaps>(command_list.as_ptr(), 28)(
            command_list.as_ptr(),
            1,
            heaps.as_ptr(),
        );
        vtable_method::<SetPipelineState>(command_list.as_ptr(), 25)(
            command_list.as_ptr(),
            pipeline.as_ptr(),
        );
        vtable_method::<SetComputeRootSignature>(command_list.as_ptr(), 29)(
            command_list.as_ptr(),
            root_signature.as_ptr(),
        );
        vtable_method::<SetComputeRootDescriptorTable>(command_list.as_ptr(), 31)(
            command_list.as_ptr(),
            0,
            gpu_start,
        );
        vtable_method::<Dispatch>(command_list.as_ptr(), 14)(
            command_list.as_ptr(),
            dispatch_workgroups[0],
            dispatch_workgroups[1],
            dispatch_workgroups[2],
        );
        let to_copy = [ResourceBarrier {
            barrier_type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            transition: ResourceTransition {
                resource: resources[output_binding].as_ptr(),
                subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                state_before: D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                state_after: D3D12_RESOURCE_STATE_COPY_SOURCE,
            },
        }];
        vtable_method::<ResourceBarrierFn>(command_list.as_ptr(), 26)(
            command_list.as_ptr(),
            1,
            to_copy.as_ptr(),
        );
        vtable_method::<CopyBufferRegion>(command_list.as_ptr(), 15)(
            command_list.as_ptr(),
            readback.as_ptr(),
            0,
            resources[output_binding].as_ptr(),
            0,
            output_bytes,
        );
        let code =
            vtable_method::<CloseCommandList>(command_list.as_ptr(), 9)(command_list.as_ptr());
        if code < S_OK {
            return Err(DirectX12Error::HResult {
                operation: "ID3D12GraphicsCommandList::Close(generic_uav)",
                code,
            });
        }
        let command_list_pointer = command_list.as_ptr();
        vtable_method::<ExecuteCommandLists>(context.queue.as_ptr(), 10)(
            context.queue.as_ptr(),
            1,
            &command_list_pointer,
        );
    }
    let fence = create_fence(context.device.as_ptr())?;
    signal_and_wait(context.queue.as_ptr(), fence.as_ptr(), 1)?;
    read_resource_bytes(
        context.device.as_ptr(),
        readback.as_ptr(),
        output_bytes_usize,
    )
}

fn compile_binary_dxil(
    operation: BinaryOp,
    operand: u32,
) -> Result<(Vec<u8>, &'static str), DirectX12Error> {
    compile_binary_dxil_for_entry(operation, operand, "main")
}

fn compile_binary_dxil_for_entry(
    operation: BinaryOp,
    operand: u32,
    entry_name: &str,
) -> Result<(Vec<u8>, &'static str), DirectX12Error> {
    validate_binary_operand(operation, operand)?;
    if !valid_shader_entry(entry_name) {
        return Err(DirectX12Error::JirSpirvLowering(
            "artifact entry name is not a valid HLSL entry".to_owned(),
        ));
    }
    let operator = hlsl_operator(operation);
    let source = format!(
        r#"RWStructuredBuffer<uint> Input : register(u0);
RWStructuredBuffer<uint> Output : register(u1);
RWStructuredBuffer<uint> Length : register(u2);

[RootSignature("DescriptorTable(UAV(u0, numDescriptors=3))")]
[numthreads(64, 1, 1)]
void {entry_name}(uint3 id : SV_DispatchThreadID) {{
    if (id.x < Length[0]) {{
        Output[id.x] = Input[id.x] {operator} {operand}u;
    }}
}}
"#,
        operator = operator,
        operand = operand,
        entry_name = entry_name,
    );
    Ok((
        compile_hlsl_source_with_entry(&source, entry_name)?,
        binary_name(operation),
    ))
}

fn compile_binary_artifact_dxil(
    artifact: &SpirvArtifact,
    operation: BinaryOp,
    operand: u32,
) -> Result<Vec<u8>, DirectX12Error> {
    validate_binary_artifact_contract(artifact, operation, operand)?;
    let operator = hlsl_operator(operation);
    let source = format!(
        r#"StructuredBuffer<uint> Input : register(t0);
RWStructuredBuffer<uint> Output : register(u1);
StructuredBuffer<uint> Length : register(t2);

[RootSignature("DescriptorTable(SRV(t0, numDescriptors=1), UAV(u1, numDescriptors=1), SRV(t2, numDescriptors=1))")]
[numthreads(64, 1, 1)]
void {entry_name}(uint3 id : SV_DispatchThreadID) {{
    if (id.x < Length[0]) {{
        Output[id.x] = Input[id.x] {operator} {operand}u;
    }}
}}
"#,
        entry_name = artifact.entry_name,
    );
    compile_hlsl_source_with_entry(&source, &artifact.entry_name)
}

fn compile_f32_artifact_dxil_for_operation(
    artifact: &SpirvArtifact,
    operand_bits: u32,
    operation: F32ArithmeticOp,
) -> Result<Vec<u8>, DirectX12Error> {
    validate_f32_binary_artifact_contract(artifact, operand_bits, operation)?;
    let [workgroup_x, workgroup_y, workgroup_z] = artifact.workgroup_size;
    if !valid_shader_entry(&artifact.entry_name)
        || workgroup_x == 0
        || workgroup_y == 0
        || workgroup_z == 0
    {
        return Err(DirectX12Error::ArtifactContract(
            "f32 artifact has an invalid entry or workgroup",
        ));
    }
    let source = format!(
        r#"StructuredBuffer<uint> Input : register(t0);
RWStructuredBuffer<uint> Output : register(u1);
StructuredBuffer<uint> Length : register(t2);

[RootSignature("DescriptorTable(SRV(t0, numDescriptors=1), UAV(u1, numDescriptors=1), SRV(t2, numDescriptors=1))")]
[numthreads({workgroup_x}, {workgroup_y}, {workgroup_z})]
void {entry_name}(uint3 id : SV_DispatchThreadID) {{
    if (id.x < Length[0]) {{
        Output[id.x] = asuint(asfloat(Input[id.x]) {operator} asfloat({operand_bits}u));
    }}
}}
"#,
        workgroup_x = workgroup_x,
        workgroup_y = workgroup_y,
        workgroup_z = workgroup_z,
        entry_name = artifact.entry_name,
        operator = f32_hlsl_operator(operation),
        operand_bits = operand_bits,
    );
    compile_hlsl_source_with_entry(&source, &artifact.entry_name)
}

fn compile_f32_vector_artifact_dxil_for_operation(
    artifact: &SpirvArtifact,
    operand_bits: u32,
    operation: F32ArithmeticOp,
) -> Result<Vec<u8>, DirectX12Error> {
    validate_f32_vector_artifact_contract(artifact, operand_bits, operation)?;
    let [workgroup_x, workgroup_y, workgroup_z] = artifact.workgroup_size;
    if !valid_shader_entry(&artifact.entry_name)
        || workgroup_x == 0
        || workgroup_y == 0
        || workgroup_z == 0
    {
        return Err(DirectX12Error::ArtifactContract(
            "f32x4 artifact has an invalid entry or workgroup",
        ));
    }
    let source = format!(
        r#"StructuredBuffer<float4> Input : register(t0);
RWStructuredBuffer<float4> Output : register(u1);
StructuredBuffer<uint> Length : register(t2);

[RootSignature("DescriptorTable(SRV(t0, numDescriptors=1), UAV(u1, numDescriptors=1), SRV(t2, numDescriptors=1))")]
[numthreads({workgroup_x}, {workgroup_y}, {workgroup_z})]
void {entry_name}(uint3 id : SV_DispatchThreadID) {{
    if (id.x < Length[0]) {{
        Output[id.x] = Input[id.x] {operator} float4(asfloat({operand_bits}u), asfloat({operand_bits}u), asfloat({operand_bits}u), asfloat({operand_bits}u));
    }}
}}
"#,
        workgroup_x = workgroup_x,
        workgroup_y = workgroup_y,
        workgroup_z = workgroup_z,
        entry_name = artifact.entry_name,
        operator = f32_hlsl_operator(operation),
        operand_bits = operand_bits,
    );
    compile_hlsl_source_with_entry(&source, &artifact.entry_name)
}

fn compile_f32_vector_lanes_artifact_dxil_for_operation(
    artifact: &SpirvArtifact,
    operand_bits: u32,
    operation: F32ArithmeticOp,
    lane_count: u32,
) -> Result<Vec<u8>, DirectX12Error> {
    validate_f32_vector_lanes_artifact_contract(artifact, operand_bits, operation, lane_count)?;
    let [workgroup_x, workgroup_y, workgroup_z] = artifact.workgroup_size;
    if !valid_shader_entry(&artifact.entry_name)
        || workgroup_x == 0
        || workgroup_y == 0
        || workgroup_z == 0
    {
        return Err(DirectX12Error::ArtifactContract(
            "vector lanes artifact has an invalid entry or workgroup",
        ));
    }
    let splat = std::iter::repeat_n(format!("asfloat({operand_bits}u)"), lane_count as usize)
        .collect::<Vec<_>>()
        .join(", ");
    let source = format!(
        r#"StructuredBuffer<float{lane_count}> Input : register(t0);
RWStructuredBuffer<float{lane_count}> Output : register(u1);
StructuredBuffer<uint> Length : register(t2);

[RootSignature("DescriptorTable(SRV(t0, numDescriptors=1), UAV(u1, numDescriptors=1), SRV(t2, numDescriptors=1))")]
[numthreads({workgroup_x}, {workgroup_y}, {workgroup_z})]
void {entry_name}(uint3 id : SV_DispatchThreadID) {{
    if (id.x < Length[0]) {{
        Output[id.x] = Input[id.x] {operator} float{lane_count}({splat});
    }}
}}
"#,
        lane_count = lane_count,
        workgroup_x = workgroup_x,
        workgroup_y = workgroup_y,
        workgroup_z = workgroup_z,
        entry_name = artifact.entry_name,
        operator = f32_hlsl_operator(operation),
        splat = splat,
    );
    compile_hlsl_source_with_entry(&source, &artifact.entry_name)
}

#[allow(dead_code)]
fn validate_f32_artifact_contract(
    artifact: &SpirvArtifact,
    addend_bits: u32,
) -> Result<(), DirectX12Error> {
    match validate_f32_binary_artifact_contract(artifact, addend_bits, F32ArithmeticOp::Add) {
        Err(DirectX12Error::ArtifactContract(
            "f32 artifact operand differs from execution request",
        )) => Err(DirectX12Error::ArtifactContract(
            "f32 artifact addend differs from execution request",
        )),
        result => result,
    }
}

fn validate_f32_binary_artifact_contract(
    artifact: &SpirvArtifact,
    operand_bits: u32,
    operation: F32ArithmeticOp,
) -> Result<(), DirectX12Error> {
    if artifact.resources.len() != 3
        || artifact
            .resources
            .iter()
            .enumerate()
            .any(|(index, resource)| {
                resource.binding != index as u32
                    || resource.address_space != jadren_jir::AddressSpace::Storage
            })
    {
        return Err(DirectX12Error::ArtifactContract(
            "runtime-length f32 artifact requires three ordered storage resources",
        ));
    }
    let mut float_types = Vec::new();
    let mut constants = Vec::new();
    let mut binary_count = 0_u32;
    let mut bounds_count = 0_u32;
    let mut store_count = 0_u32;
    let mut cursor = 5_usize;
    while cursor < artifact.words.len() {
        let instruction = artifact.words[cursor];
        let word_count = (instruction >> 16) as usize;
        let opcode = (instruction & 0xFFFF) as u16;
        if word_count == 0 || cursor + word_count > artifact.words.len() {
            return Err(DirectX12Error::ArtifactContract(
                "f32 SPIR-V instruction stream is truncated",
            ));
        }
        let operands = &artifact.words[cursor + 1..cursor + word_count];
        match opcode {
            SPIRV_OP_TYPE_FLOAT if operands.len() >= 2 && operands[1] == 32 => {
                float_types.push(operands[0]);
            }
            SPIRV_OP_CONSTANT if operands.len() >= 3 => {
                constants.push((operands[0], operands[1], operands[2]));
            }
            SPIRV_OP_FADD | SPIRV_OP_FSUB | SPIRV_OP_FMUL => {
                if opcode != f32_spirv_opcode(operation) {
                    return Err(DirectX12Error::ArtifactContract(
                        "f32 artifact operation differs from execution request",
                    ));
                }
                if operands.len() < 4 || !float_types.contains(&operands[0]) {
                    return Err(DirectX12Error::ArtifactContract(
                        "f32 artifact binary operation has an invalid type",
                    ));
                }
                binary_count += 1;
            }
            SPIRV_OP_ULT => bounds_count += 1,
            SPIRV_OP_STORE => store_count += 1,
            _ => {}
        }
        cursor += word_count;
    }
    if binary_count != 1 || bounds_count != 1 || store_count != 1 {
        return Err(DirectX12Error::ArtifactContract(
            "f32 artifact requires one binary operation, bounds predicate and store",
        ));
    }
    if !constants
        .iter()
        .any(|(type_id, _, bits)| float_types.contains(type_id) && *bits == operand_bits)
    {
        return Err(DirectX12Error::ArtifactContract(
            "f32 artifact operand differs from execution request",
        ));
    }
    Ok(())
}

fn validate_f32_vector_artifact_contract(
    artifact: &SpirvArtifact,
    operand_bits: u32,
    operation: F32ArithmeticOp,
) -> Result<(), DirectX12Error> {
    if artifact.resources.len() != 3
        || artifact
            .resources
            .iter()
            .enumerate()
            .any(|(index, resource)| {
                resource.binding != index as u32
                    || resource.address_space != jadren_jir::AddressSpace::Storage
            })
        || artifact.resources[0].element_stride != Some(16)
        || artifact.resources[1].element_stride != Some(16)
        || artifact.resources[2].element_stride != Some(4)
    {
        return Err(DirectX12Error::ArtifactContract(
            "runtime-length f32x4 artifact requires ordered storage strides 16/16/4",
        ));
    }
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
            return Err(DirectX12Error::ArtifactContract(
                "f32x4 SPIR-V instruction stream is truncated",
            ));
        }
        let operands = &artifact.words[cursor + 1..cursor + word_count];
        match opcode {
            SPIRV_OP_TYPE_FLOAT if operands.len() >= 2 && operands[1] == 32 => {
                float_types.push(operands[0]);
            }
            SPIRV_OP_TYPE_VECTOR if operands.len() >= 3 && operands[2] == 4 => {
                if !float_types.contains(&operands[1]) {
                    return Err(DirectX12Error::ArtifactContract(
                        "f32x4 vector type does not use a 32-bit float element",
                    ));
                }
                vector_types.push(operands[0]);
            }
            SPIRV_OP_CONSTANT if operands.len() >= 3 => {
                scalar_constants.push((operands[1], operands[2]));
            }
            SPIRV_OP_CONSTANT_COMPOSITE if operands.len() == 6 => {
                if !vector_types.contains(&operands[0])
                    || operands[2..].iter().any(|lane| {
                        !scalar_constants
                            .iter()
                            .any(|(id, bits)| *id == *lane && *bits == operand_bits)
                    })
                {
                    return Err(DirectX12Error::ArtifactContract(
                        "f32x4 constant composite does not encode the requested splat",
                    ));
                }
                composite_count += 1;
            }
            SPIRV_OP_FADD | SPIRV_OP_FSUB | SPIRV_OP_FMUL
                if operands.len() >= 4 && vector_types.contains(&operands[0]) =>
            {
                if opcode != f32_spirv_opcode(operation) {
                    return Err(DirectX12Error::ArtifactContract(
                        "f32x4 artifact operation differs from execution request",
                    ));
                }
                vector_binary_count += 1;
            }
            SPIRV_OP_ULT => bounds_count += 1,
            SPIRV_OP_STORE => store_count += 1,
            _ => {}
        }
        cursor += word_count;
    }
    if composite_count != 1 || vector_binary_count != 1 || bounds_count != 1 || store_count != 1 {
        return Err(DirectX12Error::ArtifactContract(
            "f32x4 artifact requires one splat, vector binary operation, bounds predicate and store",
        ));
    }
    Ok(())
}

fn validate_f32_vector_lanes_artifact_contract(
    artifact: &SpirvArtifact,
    operand_bits: u32,
    operation: F32ArithmeticOp,
    lane_count: u32,
) -> Result<(), DirectX12Error> {
    if !(2..=4).contains(&lane_count) {
        return Err(DirectX12Error::ArtifactContract(
            "runtime-length vector lane count must be in 2..=4",
        ));
    }
    let stride = lane_count * 4;
    if artifact.resources.len() != 3
        || artifact
            .resources
            .iter()
            .enumerate()
            .any(|(index, resource)| {
                resource.binding != index as u32
                    || resource.address_space != jadren_jir::AddressSpace::Storage
            })
        || artifact.resources[0].element_stride != Some(stride)
        || artifact.resources[1].element_stride != Some(stride)
        || artifact.resources[2].element_stride != Some(4)
    {
        return Err(DirectX12Error::ArtifactContract(
            "runtime-length vector artifact requires ordered storage strides lane*4/lane*4/4",
        ));
    }
    let mut float_types = Vec::new();
    let mut vector_types = Vec::new();
    let mut vector_elements = Vec::new();
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
            return Err(DirectX12Error::ArtifactContract(
                "vector lanes SPIR-V instruction stream is truncated",
            ));
        }
        let operands = &artifact.words[cursor + 1..cursor + word_count];
        match opcode {
            SPIRV_OP_TYPE_FLOAT if operands.len() >= 2 && operands[1] == 32 => {
                float_types.push(operands[0]);
            }
            SPIRV_OP_TYPE_VECTOR if operands.len() >= 3 && operands[2] == lane_count => {
                vector_types.push(operands[0]);
                vector_elements.push(operands[1]);
            }
            SPIRV_OP_CONSTANT if operands.len() >= 3 => {
                scalar_constants.push((operands[1], operands[2]));
            }
            SPIRV_OP_CONSTANT_COMPOSITE if operands.len() == (2 + lane_count as usize) => {
                if !vector_types.contains(&operands[0])
                    || operands[2..].iter().any(|lane| {
                        !scalar_constants
                            .iter()
                            .any(|(id, bits)| *id == *lane && *bits == operand_bits)
                    })
                {
                    return Err(DirectX12Error::ArtifactContract(
                        "vector lanes constant composite does not encode the requested splat",
                    ));
                }
                composite_count += 1;
            }
            SPIRV_OP_FADD | SPIRV_OP_FSUB | SPIRV_OP_FMUL
                if operands.len() >= 4 && vector_types.contains(&operands[0]) =>
            {
                if opcode != f32_spirv_opcode(operation) {
                    return Err(DirectX12Error::ArtifactContract(
                        "vector lanes artifact operation differs from execution request",
                    ));
                }
                vector_binary_count += 1;
            }
            SPIRV_OP_ULT => bounds_count += 1,
            SPIRV_OP_STORE => store_count += 1,
            _ => {}
        }
        cursor += word_count;
    }
    if !vector_types
        .iter()
        .zip(vector_elements.iter())
        .any(|(_, element)| float_types.contains(element))
    {
        return Err(DirectX12Error::ArtifactContract(
            "vector lanes type does not use a 32-bit float element",
        ));
    }
    if composite_count != 1 || vector_binary_count != 1 || bounds_count != 1 || store_count != 1 {
        return Err(DirectX12Error::ArtifactContract(
            "vector lanes artifact requires one splat, vector binary operation, bounds predicate and store",
        ));
    }
    Ok(())
}

fn validate_binary_artifact_contract(
    artifact: &SpirvArtifact,
    operation: BinaryOp,
    operand: u32,
) -> Result<(), DirectX12Error> {
    if artifact.resources.len() != 3 {
        return Err(DirectX12Error::ArtifactContract(
            "runtime BinaryOp requires exactly three resources",
        ));
    }
    let expected_opcode = spirv_binary_opcode(operation);
    let mut constants = Vec::new();
    let mut found_binary = false;
    let mut cursor = 5_usize;
    while cursor < artifact.words.len() {
        let instruction = artifact.words[cursor];
        let word_count = (instruction >> 16) as usize;
        let opcode = (instruction & 0xFFFF) as u16;
        if word_count == 0 || cursor + word_count > artifact.words.len() {
            return Err(DirectX12Error::ArtifactContract(
                "SPIR-V instruction stream is truncated",
            ));
        }
        let operands = &artifact.words[cursor + 1..cursor + word_count];
        if opcode == SPIRV_OP_CONSTANT {
            if operands.len() < 3 {
                return Err(DirectX12Error::ArtifactContract(
                    "OpConstant instruction is incomplete",
                ));
            }
            constants.push((operands[1], operands[2]));
        } else if opcode == expected_opcode {
            if operands.len() < 4 {
                return Err(DirectX12Error::ArtifactContract(
                    "BinaryOp instruction is incomplete",
                ));
            }
            let constant_id = operands[3];
            let Some((_, encoded_operand)) = constants.iter().find(|(id, _)| *id == constant_id)
            else {
                return Err(DirectX12Error::ArtifactContract(
                    "BinaryOp constant is not defined before use",
                ));
            };
            if *encoded_operand != operand {
                return Err(DirectX12Error::ArtifactContract(
                    "artifact operand differs from execution request",
                ));
            }
            if found_binary {
                return Err(DirectX12Error::ArtifactContract(
                    "artifact contains multiple BinaryOp instructions",
                ));
            }
            found_binary = true;
        }
        cursor += word_count;
    }
    if !found_binary {
        return Err(DirectX12Error::ArtifactContract(
            "artifact does not contain the requested BinaryOp opcode",
        ));
    }
    Ok(())
}

fn validate_binary_operand(operation: BinaryOp, operand: u32) -> Result<(), DirectX12Error> {
    match operation {
        BinaryOp::Divide | BinaryOp::Remainder if operand == 0 => Err(
            DirectX12Error::InvalidBinaryOperand("unsigned divisor/remainder must be non-zero"),
        ),
        BinaryOp::ShiftLeft | BinaryOp::ShiftRight if operand >= 32 => Err(
            DirectX12Error::InvalidBinaryOperand("u32 shift operand must be smaller than 32"),
        ),
        _ => Ok(()),
    }
}

fn apply_binary(value: u32, operation: BinaryOp, operand: u32) -> u32 {
    match operation {
        BinaryOp::Add => value.wrapping_add(operand),
        BinaryOp::Subtract => value.wrapping_sub(operand),
        BinaryOp::Multiply => value.wrapping_mul(operand),
        BinaryOp::Divide => value / operand,
        BinaryOp::Remainder => value % operand,
        BinaryOp::BitAnd => value & operand,
        BinaryOp::BitOr => value | operand,
        BinaryOp::BitXor => value ^ operand,
        BinaryOp::ShiftLeft => value << operand,
        BinaryOp::ShiftRight => value >> operand,
    }
}

fn apply_f32(value: f32, operation_operand: f32, operation: F32ArithmeticOp) -> f32 {
    match operation {
        F32ArithmeticOp::Add => value + operation_operand,
        F32ArithmeticOp::Subtract => value - operation_operand,
        F32ArithmeticOp::Multiply => value * operation_operand,
    }
}

const fn f32_vector_operation_operand(operation: F32ArithmeticOp) -> f32 {
    match operation {
        F32ArithmeticOp::Add | F32ArithmeticOp::Subtract => F32_VECTOR_OPERAND,
        F32ArithmeticOp::Multiply => F32_VECTOR_MULTIPLIER,
    }
}

/// Vector SPIRV-Cross output is enabled by an explicit feature flag or when a
/// complete external toolchain is discoverable through the same environment,
/// PATH or Windows SDK lookup as scalar artifacts. Builds without that
/// toolchain retain the established narrow HLSL fallback path.
fn external_vector_translation_enabled() -> bool {
    match std::env::var("JADREN_ENABLE_EXTERNAL_F32_VECTOR") {
        Ok(value) if matches!(value.as_str(), "0" | "false" | "FALSE") => false,
        Ok(value) if matches!(value.as_str(), "1" | "true" | "TRUE") => true,
        _ => SpirvDxilToolchain::discover().is_some(),
    }
}

fn write_f32x4_bytes(destination: &mut [u8], value: &[f32; 4]) {
    debug_assert_eq!(destination.len(), 16);
    for (lane, bytes) in value.iter().flat_map(|lane| lane.to_ne_bytes()).enumerate() {
        destination[lane] = bytes;
    }
}

fn parse_f32x4_bytes(bytes: &[u8]) -> Result<Vec<[f32; 4]>, DirectX12Error> {
    if !bytes.len().is_multiple_of(16) {
        return Err(DirectX12Error::ArtifactContract(
            "runtime-length f32x4 output bytes are not 16-byte aligned",
        ));
    }
    Ok(bytes
        .chunks_exact(16)
        .map(|chunk| {
            std::array::from_fn(|lane| {
                let start = lane * 4;
                f32::from_ne_bytes(
                    chunk[start..start + 4]
                        .try_into()
                        .expect("f32x4 lane has fixed width"),
                )
            })
        })
        .collect())
}

fn parse_f32_vector_lanes_bytes(
    bytes: &[u8],
    lane_count: usize,
) -> Result<Vec<Vec<f32>>, DirectX12Error> {
    let stride = lane_count
        .checked_mul(4)
        .ok_or(DirectX12Error::ArtifactContract(
            "vector lane byte stride overflow",
        ))?;
    if !(2..=4).contains(&lane_count) || !bytes.len().is_multiple_of(stride) {
        return Err(DirectX12Error::ArtifactContract(
            "runtime-length vector output bytes have an invalid lane stride",
        ));
    }
    Ok(bytes
        .chunks_exact(stride)
        .map(|chunk| {
            (0..lane_count)
                .map(|lane| {
                    let start = lane * 4;
                    f32::from_ne_bytes(
                        chunk[start..start + 4]
                            .try_into()
                            .expect("vector lane has fixed width"),
                    )
                })
                .collect()
        })
        .collect())
}

const fn f32_hlsl_operator(operation: F32ArithmeticOp) -> &'static str {
    match operation {
        F32ArithmeticOp::Add => "+",
        F32ArithmeticOp::Subtract => "-",
        F32ArithmeticOp::Multiply => "*",
    }
}

const fn f32_binary_name(operation: F32ArithmeticOp) -> &'static str {
    match operation {
        F32ArithmeticOp::Add => "add",
        F32ArithmeticOp::Subtract => "subtract",
        F32ArithmeticOp::Multiply => "multiply",
    }
}

const fn f32_spirv_opcode(operation: F32ArithmeticOp) -> u16 {
    match operation {
        F32ArithmeticOp::Add => SPIRV_OP_FADD,
        F32ArithmeticOp::Subtract => SPIRV_OP_FSUB,
        F32ArithmeticOp::Multiply => SPIRV_OP_FMUL,
    }
}

const fn hlsl_operator(operation: BinaryOp) -> &'static str {
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

const fn binary_name(operation: BinaryOp) -> &'static str {
    match operation {
        BinaryOp::Add => "add",
        BinaryOp::Subtract => "subtract",
        BinaryOp::Multiply => "multiply",
        BinaryOp::Divide => "divide",
        BinaryOp::Remainder => "remainder",
        BinaryOp::BitAnd => "bitand",
        BinaryOp::BitOr => "bitor",
        BinaryOp::BitXor => "bitxor",
        BinaryOp::ShiftLeft => "shift-left",
        BinaryOp::ShiftRight => "shift-right",
    }
}

const fn spirv_binary_opcode(operation: BinaryOp) -> u16 {
    match operation {
        BinaryOp::Add => SPIRV_OP_IADD,
        BinaryOp::Subtract => SPIRV_OP_ISUB,
        BinaryOp::Multiply => SPIRV_OP_IMUL,
        BinaryOp::Divide => SPIRV_OP_UDIV,
        BinaryOp::Remainder => SPIRV_OP_UMOD,
        BinaryOp::BitAnd => SPIRV_OP_BITWISE_AND,
        BinaryOp::BitOr => SPIRV_OP_BITWISE_OR,
        BinaryOp::BitXor => SPIRV_OP_BITWISE_XOR,
        BinaryOp::ShiftLeft => SPIRV_OP_SHIFT_LEFT_LOGICAL,
        BinaryOp::ShiftRight => SPIRV_OP_SHIFT_RIGHT_LOGICAL,
    }
}

fn compile_hlsl_source_with_entry(
    source_text: &str,
    entry_name: &str,
) -> Result<Vec<u8>, DirectX12Error> {
    let Some(dxc) = locate_dxc() else {
        return Err(DirectX12Error::ShaderToolchainUnavailable);
    };
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "jadren-directx12-dxil-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir(&directory)
        .map_err(|error| DirectX12Error::ShaderCompilation(error.to_string()))?;
    let source = directory.join("kernel.hlsl");
    let output = directory.join("kernel.dxil");
    let result = (|| {
        fs::write(&source, source_text)
            .map_err(|error| DirectX12Error::ShaderCompilation(error.to_string()))?;
        let command = Command::new(&dxc)
            .args(["-nologo", "-T", "cs_6_0", "-E", entry_name, "-Fo"])
            .arg(&output)
            .arg(&source)
            .output()
            .map_err(|error| DirectX12Error::ShaderCompilation(error.to_string()))?;
        if !command.status.success() {
            let stderr = String::from_utf8_lossy(&command.stderr).trim().to_owned();
            return Err(DirectX12Error::ShaderCompilation(stderr));
        }
        let bytes = fs::read(&output)
            .map_err(|error| DirectX12Error::ShaderCompilation(error.to_string()))?;
        if !is_dxil_container(&bytes) {
            return Err(DirectX12Error::InvalidDxil);
        }
        Ok(bytes)
    })();
    let _ = fs::remove_dir_all(&directory);
    result
}

pub fn validate_spirv_binary(words: &[u32]) -> Result<(), DirectX12Error> {
    if words.len() < 5 {
        return Err(DirectX12Error::InvalidSpirv("header-too-short"));
    }
    if words[0] != 0x0723_0203 {
        return Err(DirectX12Error::InvalidSpirv("bad-magic"));
    }
    if words[1] == 0 {
        return Err(DirectX12Error::InvalidSpirv("missing-version"));
    }
    if words[3] == 0 {
        return Err(DirectX12Error::InvalidSpirv("zero-id-bound"));
    }
    validate_spirv(words)
        .map_err(|_| DirectX12Error::InvalidSpirv("structural-validation-failed"))?;
    Ok(())
}

/// Translates a verified SPIR-V module through SPIRV-Cross and DXC.
///
/// This is deliberately an explicit host-toolchain boundary. It does not
/// claim that arbitrary JIR is supported: the caller must first lower and
/// validate a JIR kernel into the portable SPIR-V subset.
pub fn translate_spirv_to_dxil(words: &[u32], entry_name: &str) -> Result<Vec<u8>, DirectX12Error> {
    Ok(translate_spirv_source_to_dxil_report(words, entry_name)?.dxil)
}

/// Translates a structurally validated raw SPIR-V module through
/// SPIRV-Cross→HLSL→DXC and retains the source/resource audit alongside the
/// compiled DXIL.
///
/// The raw contract is deliberately conservative: only reflected storage
/// buffers with `Uniform`/`StorageBuffer` access are accepted, and every HLSL
/// `tN`/`uN` register must match the selected binding, descriptor space and
/// read/write policy.  This is a translation report, not a native dispatch
/// claim; workgroup geometry and host payload ownership belong to a later
/// adapter boundary.
pub fn translate_spirv_source_to_dxil_report(
    words: &[u32],
    entry_name: &str,
) -> Result<DxilSourceTranslationReport, DirectX12Error> {
    if !valid_shader_entry(entry_name) {
        return Err(DirectX12Error::InvalidShaderEntry);
    }
    inspect_spirv_source_module(words, entry_name).map_err(map_raw_source_contract_error)?;
    let toolchain =
        SpirvDxilToolchain::discover().ok_or(DirectX12Error::SpirvToolchainUnavailable)?;
    let source = translate_spirv_source_report_for_backend(
        words,
        entry_name,
        &toolchain.spirv_cross,
        GpuBackend::DirectX12,
    )
    .map_err(map_raw_source_contract_error)?;
    let resource_views = validate_external_hlsl_raw_source(&source)?;
    let dxil = compile_hlsl_to_dxil(&source.source, entry_name, &toolchain)?;
    Ok(DxilSourceTranslationReport {
        source,
        dxil,
        resource_views,
    })
}

fn map_raw_source_contract_error(error: SpirvSourceTranslationError) -> DirectX12Error {
    match error {
        SpirvSourceTranslationError::InvalidInput(reason)
        | SpirvSourceTranslationError::InvalidSpirv(reason) => DirectX12Error::InvalidSpirv(reason),
        SpirvSourceTranslationError::EntryPointNotFound(entry) => {
            DirectX12Error::SpirvTranslation(format!("entry point `{entry}` was not found"))
        }
        SpirvSourceTranslationError::Tool(error) => {
            DirectX12Error::SpirvTranslation(error.to_string())
        }
        SpirvSourceTranslationError::EmptySource => {
            DirectX12Error::SpirvTranslation("empty source output".to_owned())
        }
    }
}

fn compile_hlsl_to_dxil(
    source: &str,
    entry_name: &str,
    toolchain: &SpirvDxilToolchain,
) -> Result<Vec<u8>, DirectX12Error> {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "jadren-directx12-hlsl-dxil-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir(&directory)
        .map_err(|error| DirectX12Error::SpirvTranslation(error.to_string()))?;
    let hlsl_path = directory.join("kernel.hlsl");
    let dxil_path = directory.join("kernel.dxil");
    let result = (|| {
        fs::write(&hlsl_path, source)
            .map_err(|error| DirectX12Error::SpirvTranslation(error.to_string()))?;
        let compile = Command::new(&toolchain.dxc)
            .args(["-nologo", "-T", "cs_6_0", "-E"])
            .arg(entry_name)
            .args(["-Fo"])
            .arg(&dxil_path)
            .arg(&hlsl_path)
            .output()
            .map_err(|error| DirectX12Error::SpirvTranslation(error.to_string()))?;
        if !compile.status.success() {
            return Err(DirectX12Error::SpirvTranslation(
                String::from_utf8_lossy(&compile.stderr).trim().to_owned(),
            ));
        }
        let bytes = fs::read(&dxil_path)
            .map_err(|error| DirectX12Error::SpirvTranslation(error.to_string()))?;
        if !is_dxil_container(&bytes) {
            return Err(DirectX12Error::InvalidDxil);
        }
        Ok(bytes)
    })();
    let _ = fs::remove_dir_all(&directory);
    result
}

/// Lowers a verified JIR storage-add entry and sends its explicit SPIR-V
/// artifact through the host SPIRV-Cross→DXC toolchain.
///
/// The function deliberately exposes the narrow 0.1 JIR shape supported by
/// `jadren-codegen-spirv`; unsupported bodies return `JirSpirvLowering` rather
/// than being approximated as a different shader.
pub fn translate_jir_storage_add_to_dxil(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<Vec<u8>, DirectX12Error> {
    let artifact = emit_storage_add_artifact_from_jir(module, function, options)
        .map_err(|error| DirectX12Error::JirSpirvLowering(error.to_string()))?;
    translate_spirv_artifact_to_dxil(&artifact)
}

/// Lowers the verified dynamic-length global-index JIR shape and sends its
/// artifact through the same explicit DXIL toolchain boundary.
pub fn translate_jir_dynamic_storage_add_to_dxil(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<Vec<u8>, DirectX12Error> {
    let artifact =
        emit_storage_global_index_add_dynamic_length_artifact_from_jir(module, function, options)
            .map_err(|error| DirectX12Error::JirSpirvLowering(error.to_string()))?;
    translate_spirv_artifact_to_dxil(&artifact)
}

/// Lowers a verified runtime-length global-index `u32` BinaryOp JIR shape and
/// sends the backend-neutral artifact through the explicit SPIRV-Cross→DXC
/// boundary. The operation/operand remain encoded in the artifact; no
/// fallback operation is substituted when a requested shape is unsupported.
pub fn translate_jir_dynamic_storage_binary_to_dxil(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    operation: BinaryOp,
) -> Result<Vec<u8>, DirectX12Error> {
    let artifact = emit_storage_global_index_binary_dynamic_length_artifact_from_jir(
        module, function, options, operation,
    )
    .map_err(|error| DirectX12Error::JirSpirvLowering(error.to_string()))?;
    translate_spirv_artifact_to_dxil(&artifact)
}

/// Lowers a verified runtime-length global-index `f32` add JIR shape and
/// sends the artifact through the explicit SPIRV-Cross→DXC boundary.
pub fn translate_jir_dynamic_storage_fadd_to_dxil(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<Vec<u8>, DirectX12Error> {
    let artifact =
        emit_storage_global_index_fadd_dynamic_length_artifact_from_jir(module, function, options)
            .map_err(|error| DirectX12Error::JirSpirvLowering(error.to_string()))?;
    translate_spirv_artifact_to_dxil(&artifact)
}

/// Lowers a verified runtime-length global-index scalar `f32` binary JIR shape
/// and sends the selected artifact through the explicit SPIRV-Cross→DXC
/// boundary.
pub fn translate_jir_dynamic_storage_f32_binary_to_dxil(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    operation: F32ArithmeticOp,
) -> Result<Vec<u8>, DirectX12Error> {
    let artifact = emit_storage_global_index_f32_binary_dynamic_length_artifact_from_jir(
        module, function, options, operation,
    )
    .map_err(|error| DirectX12Error::JirSpirvLowering(error.to_string()))?;
    translate_spirv_artifact_to_dxil(&artifact)
}

/// Translates a validated backend-neutral SPIR-V artifact to DXIL.
///
/// The source-only hand-off is available through
/// [`translate_spirv_artifact_to_hlsl_source`] when DXC/native DX12 is not
/// part of the current host capability set.
pub fn translate_spirv_artifact_to_dxil(
    artifact: &SpirvArtifact,
) -> Result<Vec<u8>, DirectX12Error> {
    Ok(translate_spirv_artifact_to_dxil_report(artifact)?.dxil)
}

/// Translates a validated artifact to HLSL and DXIL while retaining the
/// source audit report that was used for compilation.
pub fn translate_spirv_artifact_to_dxil_report(
    artifact: &SpirvArtifact,
) -> Result<DxilArtifactTranslationReport, DirectX12Error> {
    validate_shared_artifact(artifact)?;
    validate_hlsl_resource_address_spaces(artifact)?;
    let source = translate_spirv_artifact_to_hlsl_source_report(artifact)?;
    let dxc = locate_dxc().ok_or(DirectX12Error::ShaderToolchainUnavailable)?;
    let toolchain = SpirvDxilToolchain {
        spirv_cross: PathBuf::new(),
        dxc,
    };
    let dxil = compile_hlsl_to_dxil(&source.source, &artifact.entry_name, &toolchain)?;
    let resource_views = hlsl_source_uav_views(&source.source, artifact)?;
    Ok(DxilArtifactTranslationReport {
        source,
        dxil,
        resource_views,
    })
}

/// Translates a validated artifact to HLSL and verifies its portable source
/// contract without requiring DXC or a native D3D12 device.
pub fn translate_spirv_artifact_to_hlsl_source(
    artifact: &SpirvArtifact,
) -> Result<String, DirectX12Error> {
    Ok(translate_spirv_artifact_to_hlsl_source_report(artifact)?.source)
}

/// Translates an artifact to HLSL and returns the shared source audit report.
pub fn translate_spirv_artifact_to_hlsl_source_report(
    artifact: &SpirvArtifact,
) -> Result<ArtifactSourceTranslationReport, DirectX12Error> {
    validate_shared_artifact(artifact)?;
    validate_hlsl_resource_address_spaces(artifact)?;
    let spirv_cross = locate_spirv_cross().ok_or(DirectX12Error::SpirvCrossUnavailable)?;
    let report =
        translate_spirv_artifact_source(artifact, &spirv_cross, ArtifactSourceBackend::Hlsl)
            .map_err(|error| match error {
                ArtifactSourceTranslationError::Contract(_) => {
                    DirectX12Error::InvalidSpirv("source-translation-contract")
                }
                ArtifactSourceTranslationError::Tool(error) => {
                    DirectX12Error::SpirvTranslation(error.to_string())
                }
            })?;
    validate_external_hlsl_source(&report.source, artifact)?;
    Ok(report)
}

fn validate_hlsl_resource_address_spaces(artifact: &SpirvArtifact) -> Result<(), DirectX12Error> {
    if let Some(resource) = artifact
        .resources
        .iter()
        .find(|resource| resource.address_space != AddressSpace::Storage)
    {
        return Err(DirectX12Error::SpirvTranslation(format!(
            "HLSL resource binding {} uses unsupported address space {:?}",
            resource.binding, resource.address_space
        )));
    }
    Ok(())
}

fn validate_external_hlsl_source(
    source: &str,
    artifact: &SpirvArtifact,
) -> Result<(), DirectX12Error> {
    if source.trim().is_empty() || !contains_hlsl_entry(source, &artifact.entry_name) {
        return Err(DirectX12Error::SpirvTranslation(
            "HLSL output is missing the requested compute entry".to_owned(),
        ));
    }
    validate_hlsl_resource_address_spaces(artifact)?;
    let mut seen = Vec::new();
    let mut remaining = source;
    while let Some(start) = remaining.find("register(") {
        let after_marker = &remaining[start + "register(".len()..];
        let end = after_marker.find(')').ok_or_else(|| {
            DirectX12Error::SpirvTranslation("HLSL register attribute is truncated".to_owned())
        })?;
        let token = after_marker[..end].trim();
        let mut token_parts = token.split(',');
        let register_name = token_parts.next().unwrap_or_default().trim();
        if token_parts.clone().count() > 1 {
            return Err(DirectX12Error::SpirvTranslation(
                "HLSL register attribute has too many qualifiers".to_owned(),
            ));
        }
        let descriptor_space = if let Some(space_name) = token_parts.next() {
            let space_name = space_name.trim();
            let Some(space_text) = space_name.strip_prefix("space") else {
                return Err(DirectX12Error::SpirvTranslation(
                    "HLSL register attribute has an invalid descriptor space".to_owned(),
                ));
            };
            let space = space_text.parse::<u32>().map_err(|_| {
                DirectX12Error::SpirvTranslation(
                    "HLSL register descriptor space is not numeric".to_owned(),
                )
            })?;
            Some(space)
        } else {
            None
        };
        let mut characters = register_name.chars();
        let register_class = characters.next().ok_or_else(|| {
            DirectX12Error::SpirvTranslation("HLSL register attribute is empty".to_owned())
        })?;
        if !matches!(register_class, 'b' | 's' | 't' | 'u') {
            return Err(DirectX12Error::SpirvTranslation(
                "HLSL register attribute has an unsupported class".to_owned(),
            ));
        }
        let binding_text = characters.as_str();
        let binding = binding_text.parse::<u32>().map_err(|_| {
            DirectX12Error::SpirvTranslation("HLSL register attribute is not numeric".to_owned())
        })?;
        let resource = artifact
            .resources
            .iter()
            .find(|resource| resource.binding == binding)
            .ok_or_else(|| {
                DirectX12Error::SpirvTranslation(format!(
                    "HLSL output contains unknown register binding {binding}"
                ))
            })?;
        let declaration = remaining[..start].rsplit(';').next().unwrap_or_default();
        let expected_space = resource.descriptor_set;
        if descriptor_space != Some(expected_space)
            && !(descriptor_space.is_none() && expected_space == 0)
        {
            return Err(DirectX12Error::SpirvTranslation(format!(
                "HLSL register descriptor space for binding {binding} differs from artifact metadata"
            )));
        }
        let actual_name = hlsl_resource_name(declaration).ok_or_else(|| {
            DirectX12Error::SpirvTranslation(format!(
                "HLSL resource binding {binding} has no valid resource name"
            ))
        })?;
        if !hlsl_resource_name_matches(actual_name, &resource.name) {
            return Err(DirectX12Error::SpirvTranslation(format!(
                "HLSL resource binding {binding} name `{actual_name}` differs from artifact metadata `{}`; declaration `{}`",
                resource.name,
                declaration.trim()
            )));
        }
        if let Some(expected_type) = resource.element_type_info {
            if let Some(actual_type) = hlsl_structured_element_type(declaration) {
                if actual_type != expected_type {
                    return Err(DirectX12Error::SpirvTranslation(format!(
                        "HLSL resource binding {binding} element type differs from artifact metadata"
                    )));
                }
            } else if !hlsl_byte_address_buffer(declaration) {
                return Err(DirectX12Error::SpirvTranslation(format!(
                    "HLSL resource binding {binding} has no supported structured element type"
                )));
            }
        }
        if let Some(expected_stride) = resource.element_stride {
            if let Some(actual_stride) = hlsl_structured_element_stride(declaration) {
                if actual_stride != expected_stride {
                    return Err(DirectX12Error::SpirvTranslation(format!(
                        "HLSL resource binding {binding} element stride differs from artifact metadata"
                    )));
                }
            } else if !hlsl_byte_address_buffer(declaration) || !expected_stride.is_multiple_of(4) {
                return Err(DirectX12Error::SpirvTranslation(format!(
                    "HLSL resource binding {binding} has no supported structured element type"
                )));
            }
        }
        if seen
            .iter()
            .any(|(seen_binding, _)| *seen_binding == binding)
        {
            return Err(DirectX12Error::SpirvTranslation(format!(
                "HLSL output repeats register binding {binding}"
            )));
        }
        seen.push((binding, register_class));
        remaining = &after_marker[end + 1..];
    }
    for resource in &artifact.resources {
        let Some((_, register_class)) = seen
            .iter()
            .find(|(binding, _)| *binding == resource.binding)
        else {
            return Err(DirectX12Error::SpirvTranslation(format!(
                "HLSL output is missing register binding {}",
                resource.binding
            )));
        };
        let class_is_valid = match resource.access {
            ResourceAccess::ReadOnly => *register_class == 't',
            ResourceAccess::WriteOnly | ResourceAccess::ReadWrite => *register_class == 'u',
        };
        if !class_is_valid {
            return Err(DirectX12Error::SpirvTranslation(format!(
                "HLSL register class for binding {} violates resource access policy",
                resource.binding
            )));
        }
    }
    Ok(())
}

fn validate_external_hlsl_raw_source(
    report: &SpirvSourceTranslationReport,
) -> Result<Vec<DxilUavView>, DirectX12Error> {
    let source = &report.source;
    let entry_name = &report.identity.entry_name;
    let resources = &report.identity.resources;
    if source.trim().is_empty() || !contains_hlsl_entry(source, entry_name) {
        return Err(DirectX12Error::SpirvTranslation(
            "HLSL output is missing the requested compute entry".to_owned(),
        ));
    }
    let mut views = vec![None; resources.len()];
    let mut seen = Vec::new();
    let mut remaining = source.as_str();
    while let Some(start) = remaining.find("register(") {
        let after_marker = &remaining[start + "register(".len()..];
        let end = after_marker.find(')').ok_or_else(|| {
            DirectX12Error::SpirvTranslation("HLSL register attribute is truncated".to_owned())
        })?;
        let mut token_parts = after_marker[..end].trim().split(',');
        let register_name = token_parts.next().unwrap_or_default().trim();
        let descriptor_space = token_parts
            .next()
            .map(|space_name| {
                let space_name = space_name.trim();
                let space_text = space_name.strip_prefix("space").ok_or_else(|| {
                    DirectX12Error::SpirvTranslation(
                        "HLSL register descriptor space is invalid".to_owned(),
                    )
                })?;
                space_text.parse::<u32>().map_err(|_| {
                    DirectX12Error::SpirvTranslation(
                        "HLSL register descriptor space is not numeric".to_owned(),
                    )
                })
            })
            .transpose()?;
        if token_parts.next().is_some() {
            return Err(DirectX12Error::SpirvTranslation(
                "HLSL register attribute has too many qualifiers".to_owned(),
            ));
        }
        let mut register_characters = register_name.chars();
        let register_class = register_characters.next().ok_or_else(|| {
            DirectX12Error::SpirvTranslation("HLSL register attribute is empty".to_owned())
        })?;
        if !matches!(register_class, 't' | 'u') {
            return Err(DirectX12Error::SpirvTranslation(format!(
                "HLSL raw source contains unsupported register class `{register_class}`"
            )));
        }
        let binding = register_characters.as_str().parse::<u32>().map_err(|_| {
            DirectX12Error::SpirvTranslation(
                "HLSL register binding is not a numeric descriptor".to_owned(),
            )
        })?;
        let resource_index = resources
            .iter()
            .position(|resource| resource.binding == binding)
            .ok_or_else(|| {
                DirectX12Error::SpirvTranslation(format!(
                    "HLSL output contains unknown raw register binding {binding}"
                ))
            })?;
        let resource = &resources[resource_index];
        if descriptor_space != Some(resource.descriptor_set)
            && !(descriptor_space.is_none() && resource.descriptor_set == 0)
        {
            return Err(DirectX12Error::SpirvTranslation(format!(
                "HLSL raw register descriptor space for binding {binding} differs from SPIR-V"
            )));
        }
        let expected_class = match (resource.storage_class, resource.access) {
            (Some(2), Some(ResourceAccess::ReadOnly)) => 't',
            (Some(12), Some(ResourceAccess::ReadOnly)) => 't',
            (Some(12), Some(ResourceAccess::WriteOnly | ResourceAccess::ReadWrite)) => 'u',
            (Some(2), Some(_)) => {
                return Err(DirectX12Error::SpirvTranslation(format!(
                    "raw uniform binding {binding} has a writable access policy"
                )));
            }
            (Some(storage_class), Some(_)) => {
                return Err(DirectX12Error::SpirvTranslation(format!(
                    "raw resource binding {binding} has unsupported storage class {storage_class}"
                )));
            }
            (_, None) => {
                return Err(DirectX12Error::SpirvTranslation(format!(
                    "raw resource binding {binding} has no access policy"
                )));
            }
            (None, Some(_)) => {
                return Err(DirectX12Error::SpirvTranslation(format!(
                    "raw resource binding {binding} has no storage class"
                )));
            }
        };
        if register_class != expected_class {
            return Err(DirectX12Error::SpirvTranslation(format!(
                "HLSL register class for raw binding {binding} violates access policy"
            )));
        }
        let declaration = remaining[..start].rsplit(';').next().unwrap_or_default();
        if let Some(expected_type) = resource.element_type {
            if let Some(actual_type) = hlsl_structured_element_type(declaration) {
                if actual_type != expected_type {
                    return Err(DirectX12Error::SpirvTranslation(format!(
                        "HLSL raw binding {binding} element type differs from SPIR-V"
                    )));
                }
            } else if !hlsl_byte_address_buffer(declaration) {
                return Err(DirectX12Error::SpirvTranslation(format!(
                    "HLSL raw binding {binding} has no supported structured element type"
                )));
            }
        }
        if let Some(expected_stride) = resource.element_stride {
            if let Some(actual_stride) = hlsl_structured_element_stride(declaration) {
                if actual_stride != expected_stride {
                    return Err(DirectX12Error::SpirvTranslation(format!(
                        "HLSL raw binding {binding} element stride differs from SPIR-V"
                    )));
                }
            } else if !hlsl_byte_address_buffer(declaration) || !expected_stride.is_multiple_of(4) {
                return Err(DirectX12Error::SpirvTranslation(format!(
                    "HLSL raw binding {binding} has no supported element layout"
                )));
            }
        }
        if seen.contains(&binding) {
            return Err(DirectX12Error::SpirvTranslation(format!(
                "HLSL output repeats raw register binding {binding}"
            )));
        }
        let view = if hlsl_byte_address_buffer(declaration) && register_class == 't' {
            DxilUavView::RawSrv
        } else if hlsl_byte_address_buffer(declaration) {
            DxilUavView::Raw
        } else if declaration.contains("StructuredBuffer") && register_class == 't' {
            DxilUavView::StructuredSrv
        } else if declaration.contains("StructuredBuffer") {
            DxilUavView::Structured
        } else {
            return Err(DirectX12Error::SpirvTranslation(format!(
                "HLSL raw binding {binding} has no supported buffer view"
            )));
        };
        views[resource_index] = Some(view);
        seen.push(binding);
        remaining = &after_marker[end + 1..];
    }
    resources
        .iter()
        .enumerate()
        .map(|(index, resource)| {
            views[index].ok_or_else(|| {
                DirectX12Error::SpirvTranslation(format!(
                    "HLSL output is missing raw register binding {}",
                    resource.binding
                ))
            })
        })
        .collect()
}

fn hlsl_resource_name(declaration: &str) -> Option<&str> {
    let before_register = declaration.split(':').next()?.trim();
    let name = before_register.split_whitespace().last()?;
    valid_shader_entry(name).then_some(name)
}

fn hlsl_resource_name_matches(actual: &str, expected: &str) -> bool {
    // SPIRV-Cross prefixes a source name with one underscore when the target
    // language reserves that spelling (for example `input` -> `_input`).
    // Keep this normalization narrow and deterministic; arbitrary renames
    // remain rejected by the exact-or-single-prefix comparison.
    actual == expected || actual.strip_prefix('_') == Some(expected)
}

fn hlsl_structured_element_type(declaration: &str) -> Option<ResourceElementType> {
    let open = declaration.rfind('<')?;
    let close = declaration[open + 1..].find('>')? + open + 1;
    let container = declaration[..open].split_whitespace().last()?;
    if !matches!(container, "StructuredBuffer" | "RWStructuredBuffer") {
        return None;
    }
    let element = declaration[open + 1..close].trim();
    ResourceElementType::from_shader_name(element)
}

fn hlsl_structured_element_stride(declaration: &str) -> Option<u32> {
    hlsl_structured_element_type(declaration)?.byte_stride()
}

fn hlsl_byte_address_buffer(declaration: &str) -> bool {
    declaration
        .split_whitespace()
        .any(|token| matches!(token, "ByteAddressBuffer" | "RWByteAddressBuffer"))
}

fn hlsl_source_uav_views(
    source: &str,
    artifact: &SpirvArtifact,
) -> Result<Vec<DxilUavView>, DirectX12Error> {
    let mut views = vec![None; artifact.resources.len()];
    let mut remaining = source;
    while let Some(start) = remaining.find("register(") {
        let after_marker = &remaining[start + "register(".len()..];
        let end = after_marker.find(')').ok_or_else(|| {
            DirectX12Error::SpirvTranslation("HLSL register attribute is truncated".to_owned())
        })?;
        let token = after_marker[..end]
            .split(',')
            .next()
            .unwrap_or_default()
            .trim();
        let mut register_characters = token.chars();
        let register_class = register_characters.next().ok_or_else(|| {
            DirectX12Error::SpirvTranslation(
                "HLSL register attribute is empty while classifying native views".to_owned(),
            )
        })?;
        if !matches!(register_class, 't' | 'u') {
            return Err(DirectX12Error::SpirvTranslation(format!(
                "HLSL register class `{register_class}` is unsupported for native resource views"
            )));
        }
        let binding = register_characters.as_str().parse::<u32>().map_err(|_| {
            DirectX12Error::SpirvTranslation(
                "HLSL register attribute is not numeric while classifying UAV views".to_owned(),
            )
        })?;
        let resource_index = artifact
            .resources
            .iter()
            .position(|resource| resource.binding == binding)
            .ok_or_else(|| {
                DirectX12Error::SpirvTranslation(format!(
                    "HLSL view classification contains unknown register binding {binding}"
                ))
            })?;
        let declaration = remaining[..start].rsplit(';').next().unwrap_or_default();
        let resource = &artifact.resources[resource_index];
        let is_srv = register_class == 't';
        if is_srv && resource.access != ResourceAccess::ReadOnly {
            return Err(DirectX12Error::SpirvTranslation(format!(
                "HLSL SRV binding {binding} is not read-only in artifact metadata"
            )));
        }
        if !is_srv && !resource.access.can_write() {
            return Err(DirectX12Error::SpirvTranslation(format!(
                "HLSL UAV binding {binding} is not read-write in artifact metadata"
            )));
        }
        let view = if hlsl_byte_address_buffer(declaration) && is_srv {
            DxilUavView::RawSrv
        } else if hlsl_byte_address_buffer(declaration) {
            DxilUavView::Raw
        } else if declaration.contains("StructuredBuffer") && is_srv {
            DxilUavView::StructuredSrv
        } else if declaration.contains("StructuredBuffer") {
            DxilUavView::Structured
        } else {
            remaining = &after_marker[end + 1..];
            continue;
        };
        if views[resource_index]
            .replace(view)
            .is_some_and(|previous| previous != view)
        {
            return Err(DirectX12Error::SpirvTranslation(format!(
                "HLSL resource binding {binding} changes UAV view kind"
            )));
        }
        remaining = &after_marker[end + 1..];
    }
    views
        .into_iter()
        .enumerate()
        .map(|(index, view)| {
            view.ok_or_else(|| {
                DirectX12Error::SpirvTranslation(format!(
                    "HLSL output is missing a UAV view classification for binding {index}"
                ))
            })
        })
        .collect()
}

fn contains_hlsl_entry(source: &str, entry_name: &str) -> bool {
    source.split("void").skip(1).any(|fragment| {
        let fragment = fragment.trim_start();
        let Some(fragment) = fragment.strip_prefix(entry_name) else {
            return false;
        };
        fragment.trim_start().starts_with('(')
    })
}

fn is_dxil_container(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[0..4] == *b"DXBC" && bytes.windows(4).any(|window| window == b"DXIL")
}

fn valid_shader_entry(entry_name: &str) -> bool {
    let mut bytes = entry_name.bytes();
    matches!(bytes.next(), Some(b'A'..=b'Z' | b'a'..=b'z' | b'_'))
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn locate_dxc() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("JADREN_DXC").map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }
    if let Some(path) = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .map(|directory| directory.join(if cfg!(windows) { "dxc.exe" } else { "dxc" }))
        .find(|path| path.is_file())
    {
        return Some(path);
    }
    let root = Path::new("C:\\Program Files (x86)\\Windows Kits\\10\\bin");
    let mut versions = fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    versions.sort();
    versions.reverse();
    versions
        .into_iter()
        .map(|version| version.join("x64\\dxc.exe"))
        .find(|path| path.is_file())
}

fn locate_spirv_cross() -> Option<PathBuf> {
    locate_executable("JADREN_SPIRV_CROSS", &["spirv-cross.exe", "spirv-cross"])
}

fn locate_executable(environment_key: &str, names: &[&str]) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(environment_key).map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .flat_map(|directory| names.iter().map(move |name| directory.join(name)))
        .find(|path| path.is_file())
}

fn buffer_desc(width: usize, flags: u32) -> ResourceDesc {
    ResourceDesc {
        dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
        alignment: 0,
        width: width as u64,
        height: 1,
        depth_or_array_size: 1,
        mip_levels: 1,
        format: DXGI_FORMAT_UNKNOWN,
        sample_desc: SampleDesc {
            count: 1,
            quality: 0,
        },
        layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
        flags,
    }
}

fn create_device_and_queue() -> Result<DxContext, DirectX12Error> {
    let library = unsafe { libloading::Library::new("d3d12.dll") }
        .map_err(|error| DirectX12Error::Loader(error.to_string()))?;
    let create_device: libloading::Symbol<'_, D3D12CreateDevice> = unsafe {
        library
            .get(b"D3D12CreateDevice\0")
            .map_err(|error| DirectX12Error::Loader(error.to_string()))?
    };
    let mut device = std::ptr::null_mut();
    let code = unsafe {
        create_device(
            std::ptr::null_mut(),
            D3D_FEATURE_LEVEL_11_0,
            &IID_ID3D12_DEVICE,
            &mut device,
        )
    };
    if code < S_OK {
        return Err(DirectX12Error::HResult {
            operation: "D3D12CreateDevice(compute)",
            code,
        });
    }
    if device.is_null() {
        return Err(DirectX12Error::NullObject("D3D12CreateDevice(compute)"));
    }
    let device = ComObject::new(device, "ID3D12Device(compute)");
    let queue_desc = CommandQueueDesc {
        queue_type: D3D12_COMMAND_LIST_TYPE_DIRECT,
        priority: 0,
        flags: D3D12_COMMAND_QUEUE_FLAG_NONE,
        node_mask: 0,
    };
    let mut queue = std::ptr::null_mut();
    let code = unsafe {
        vtable_method::<CreateCommandQueue>(device.as_ptr(), 8)(
            device.as_ptr(),
            &queue_desc,
            &IID_ID3D12_COMMAND_QUEUE,
            &mut queue,
        )
    };
    if code < S_OK {
        return Err(DirectX12Error::HResult {
            operation: "CreateCommandQueue(compute)",
            code,
        });
    }
    if queue.is_null() {
        return Err(DirectX12Error::NullObject("CreateCommandQueue(compute)"));
    }
    Ok(DxContext {
        _library: library,
        device,
        queue: ComObject::new(queue, "ID3D12CommandQueue(compute)"),
    })
}

struct DxContext {
    queue: ComObject,
    device: ComObject,
    _library: libloading::Library,
}

fn create_resource(
    device: *mut c_void,
    heap: &HeapProperties,
    desc: &ResourceDesc,
    initial_state: u32,
    operation: &'static str,
) -> Result<ComObject, DirectX12Error> {
    let mut resource = std::ptr::null_mut();
    let code = unsafe {
        vtable_method::<CreateCommittedResource>(device, 27)(
            device,
            heap,
            D3D12_HEAP_FLAG_NONE,
            desc,
            initial_state,
            std::ptr::null(),
            &IID_ID3D12_RESOURCE,
            &mut resource,
        )
    };
    if code < S_OK {
        return Err(DirectX12Error::HResult { operation, code });
    }
    if resource.is_null() {
        return Err(DirectX12Error::NullObject(operation));
    }
    Ok(ComObject::new(resource, "ID3D12Resource"))
}

fn write_resource(
    device: *mut c_void,
    resource: *mut c_void,
    values: &[u32],
) -> Result<(), DirectX12Error> {
    let mut pointer = std::ptr::null_mut();
    let read_range = Range { begin: 0, end: 0 };
    let code = unsafe {
        vtable_method::<MapResource>(resource, 8)(resource, 0, &read_range, &mut pointer)
    };
    if code < S_OK {
        return Err(DirectX12Error::HResult {
            operation: "ID3D12Resource::Map(upload)",
            code,
        });
    }
    if pointer.is_null() {
        return Err(DirectX12Error::NullObject("Map(upload)"));
    }
    unsafe {
        std::ptr::copy_nonoverlapping(
            values.as_ptr() as *const u8,
            pointer as *mut u8,
            std::mem::size_of_val(values),
        );
        vtable_method::<UnmapResource>(resource, 9)(resource, 0, std::ptr::null());
    }
    let _ = device;
    Ok(())
}

fn write_resource_bytes(
    device: *mut c_void,
    resource: *mut c_void,
    bytes: &[u8],
) -> Result<(), DirectX12Error> {
    let mut pointer = std::ptr::null_mut();
    let read_range = Range { begin: 0, end: 0 };
    let code = unsafe {
        vtable_method::<MapResource>(resource, 8)(resource, 0, &read_range, &mut pointer)
    };
    if code < S_OK {
        return Err(DirectX12Error::HResult {
            operation: "ID3D12Resource::Map(upload)",
            code,
        });
    }
    if pointer.is_null() {
        return Err(DirectX12Error::NullObject("Map(upload)"));
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer as *mut u8, bytes.len());
        vtable_method::<UnmapResource>(resource, 9)(resource, 0, std::ptr::null());
    }
    let _ = device;
    Ok(())
}

fn read_resource(
    _device: *mut c_void,
    resource: *mut c_void,
    length: usize,
) -> Result<Vec<u32>, DirectX12Error> {
    let bytes =
        read_resource_bytes(
            _device,
            resource,
            length.checked_mul(std::mem::size_of::<u32>()).ok_or(
                DirectX12Error::ArtifactContract("readback byte size overflows host usize"),
            )?,
        )?;
    Ok(bytes
        .chunks_exact(std::mem::size_of::<u32>())
        .map(|chunk| u32::from_ne_bytes(chunk.try_into().expect("u32 chunk has fixed width")))
        .collect())
}

fn read_resource_bytes(
    _device: *mut c_void,
    resource: *mut c_void,
    byte_length: usize,
) -> Result<Vec<u8>, DirectX12Error> {
    let mut pointer = std::ptr::null_mut();
    let range = Range {
        begin: 0,
        end: byte_length,
    };
    let code =
        unsafe { vtable_method::<MapResource>(resource, 8)(resource, 0, &range, &mut pointer) };
    if code < S_OK {
        return Err(DirectX12Error::HResult {
            operation: "ID3D12Resource::Map(readback)",
            code,
        });
    }
    if pointer.is_null() {
        return Err(DirectX12Error::NullObject("Map(readback)"));
    }
    let mut values = vec![0_u8; byte_length];
    unsafe {
        std::ptr::copy_nonoverlapping(pointer as *const u8, values.as_mut_ptr(), byte_length);
        vtable_method::<UnmapResource>(resource, 9)(resource, 0, &range);
    }
    Ok(values)
}

fn create_descriptor_heap(
    device: *mut c_void,
    descriptor_count: u32,
) -> Result<ComObject, DirectX12Error> {
    if descriptor_count == 0 {
        return Err(DirectX12Error::HResult {
            operation: "D3D12 descriptor heap count",
            code: -1,
        });
    }
    let desc = DescriptorHeapDesc {
        heap_type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
        num_descriptors: descriptor_count,
        flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
        node_mask: 0,
    };
    let mut heap = std::ptr::null_mut();
    let code = unsafe {
        vtable_method::<CreateDescriptorHeap>(device, 14)(
            device,
            &desc,
            &IID_ID3D12_DESCRIPTOR_HEAP,
            &mut heap,
        )
    };
    if code < S_OK {
        return Err(DirectX12Error::HResult {
            operation: "ID3D12Device::CreateDescriptorHeap",
            code,
        });
    }
    if heap.is_null() {
        return Err(DirectX12Error::NullObject("CreateDescriptorHeap"));
    }
    Ok(ComObject::new(heap, "ID3D12DescriptorHeap"))
}

#[allow(dead_code)]
fn create_root_signature(
    library: &libloading::Library,
    device: *mut c_void,
) -> Result<ComObject, DirectX12Error> {
    let serialize: libloading::Symbol<'_, SerializeRootSignature> = unsafe {
        library
            .get(b"D3D12SerializeRootSignature\0")
            .map_err(|error| DirectX12Error::Loader(error.to_string()))?
    };
    let descriptor_range = RootDescriptorRange {
        range_type: D3D12_DESCRIPTOR_RANGE_TYPE_UAV,
        num_descriptors: 2,
        base_shader_register: 0,
        register_space: 0,
        offset_in_descriptors_from_table_start: 0xFFFF_FFFF,
    };
    let parameters = [
        RootParameter {
            parameter_type: D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS,
            data: RootParameterData {
                constants: RootConstants {
                    num_32_bit_values: 1,
                    shader_register: 0,
                    register_space: 0,
                },
            },
            shader_visibility: D3D12_SHADER_VISIBILITY_ALL,
        },
        RootParameter {
            parameter_type: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
            data: RootParameterData {
                descriptor_table: RootDescriptorTable {
                    num_descriptor_ranges: 1,
                    descriptor_ranges: &descriptor_range,
                },
            },
            shader_visibility: D3D12_SHADER_VISIBILITY_ALL,
        },
    ];
    let descriptor = RootSignatureDesc {
        num_parameters: parameters.len() as u32,
        parameters: parameters.as_ptr(),
        num_static_samplers: 0,
        static_samplers: std::ptr::null(),
        flags: D3D12_ROOT_SIGNATURE_FLAG_NONE,
    };
    let mut blob = std::ptr::null_mut();
    let mut error_blob = std::ptr::null_mut();
    let code = unsafe {
        serialize(
            &descriptor,
            D3D12_ROOT_SIGNATURE_VERSION_1_0,
            &mut blob,
            &mut error_blob,
        )
    };
    if !error_blob.is_null() {
        unsafe {
            vtable_method::<Release>(error_blob, 2)(error_blob);
        }
    }
    if code < S_OK {
        return Err(DirectX12Error::HResult {
            operation: "D3D12SerializeRootSignature",
            code,
        });
    }
    if blob.is_null() {
        return Err(DirectX12Error::NullObject("D3D12SerializeRootSignature"));
    }
    let blob = ComObject::new(blob, "ID3DBlob(root-signature)");
    let get_pointer = unsafe { vtable_method::<BlobGetBufferPointer>(blob.as_ptr(), 3) };
    let get_size = unsafe { vtable_method::<BlobGetBufferSize>(blob.as_ptr(), 4) };
    let blob_pointer = unsafe { get_pointer(blob.as_ptr()) };
    let blob_size = unsafe { get_size(blob.as_ptr()) };
    if blob_pointer.is_null() || blob_size == 0 {
        return Err(DirectX12Error::NullObject(
            "D3D12SerializeRootSignature(blob)",
        ));
    }
    let mut root_signature = std::ptr::null_mut();
    let code = unsafe {
        vtable_method::<CreateRootSignature>(device, 16)(
            device,
            0,
            blob_pointer,
            blob_size,
            &IID_ID3D12_ROOT_SIGNATURE,
            &mut root_signature,
        )
    };
    if code < S_OK {
        return Err(DirectX12Error::HResult {
            operation: "ID3D12Device::CreateRootSignature",
            code,
        });
    }
    if root_signature.is_null() {
        return Err(DirectX12Error::NullObject("CreateRootSignature"));
    }
    Ok(ComObject::new(root_signature, "ID3D12RootSignature"))
}

/// Creates a descriptor-table-only root signature for a reflected UAV count.
/// The artifact execution path keeps all runtime values in explicit resources
/// and therefore does not rely on root constants.
fn create_uav_root_signature(
    library: &libloading::Library,
    device: *mut c_void,
    descriptor_count: u32,
) -> Result<ComObject, DirectX12Error> {
    if descriptor_count == 0 {
        return Err(DirectX12Error::HResult {
            operation: "D3D12 root signature descriptor count",
            code: -1,
        });
    }
    create_native_resource_root_signature(
        library,
        device,
        &vec![DxilUavView::Structured; descriptor_count as usize],
    )
}

/// Creates one descriptor table whose ranges preserve the reflected SRV/UAV
/// class for every dense binding. Each range uses an explicit heap offset so
/// the root table index remains the SPIR-V/HLSL binding ordinal.
fn create_native_resource_root_signature(
    library: &libloading::Library,
    device: *mut c_void,
    views: &[DxilUavView],
) -> Result<ComObject, DirectX12Error> {
    if views.is_empty() {
        return Err(DirectX12Error::HResult {
            operation: "D3D12 native resource root signature view count",
            code: -1,
        });
    }
    let serialize: libloading::Symbol<'_, SerializeRootSignature> = unsafe {
        library
            .get(b"D3D12SerializeRootSignature\0")
            .map_err(|error| DirectX12Error::Loader(error.to_string()))?
    };
    let descriptor_ranges = views
        .iter()
        .enumerate()
        .map(|(binding, view)| RootDescriptorRange {
            range_type: if dxil_view_is_writable(*view) {
                D3D12_DESCRIPTOR_RANGE_TYPE_UAV
            } else {
                D3D12_DESCRIPTOR_RANGE_TYPE_SRV
            },
            num_descriptors: 1,
            base_shader_register: binding as u32,
            register_space: 0,
            offset_in_descriptors_from_table_start: binding as u32,
        })
        .collect::<Vec<_>>();
    let parameter = RootParameter {
        parameter_type: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
        data: RootParameterData {
            descriptor_table: RootDescriptorTable {
                num_descriptor_ranges: descriptor_ranges.len() as u32,
                descriptor_ranges: descriptor_ranges.as_ptr(),
            },
        },
        shader_visibility: D3D12_SHADER_VISIBILITY_ALL,
    };
    let descriptor = RootSignatureDesc {
        num_parameters: 1,
        parameters: &parameter,
        num_static_samplers: 0,
        static_samplers: std::ptr::null(),
        flags: D3D12_ROOT_SIGNATURE_FLAG_NONE,
    };
    let mut blob = std::ptr::null_mut();
    let mut error_blob = std::ptr::null_mut();
    let code = unsafe {
        serialize(
            &descriptor,
            D3D12_ROOT_SIGNATURE_VERSION_1_0,
            &mut blob,
            &mut error_blob,
        )
    };
    if !error_blob.is_null() {
        unsafe {
            vtable_method::<Release>(error_blob, 2)(error_blob);
        }
    }
    if code < S_OK {
        return Err(DirectX12Error::HResult {
            operation: "D3D12SerializeRootSignature(native resource table)",
            code,
        });
    }
    if blob.is_null() {
        return Err(DirectX12Error::NullObject(
            "D3D12SerializeRootSignature(native resource table)",
        ));
    }
    let blob = ComObject::new(blob, "ID3DBlob(uav-root-signature)");
    let get_pointer = unsafe { vtable_method::<BlobGetBufferPointer>(blob.as_ptr(), 3) };
    let get_size = unsafe { vtable_method::<BlobGetBufferSize>(blob.as_ptr(), 4) };
    let blob_pointer = unsafe { get_pointer(blob.as_ptr()) };
    let blob_size = unsafe { get_size(blob.as_ptr()) };
    if blob_pointer.is_null() || blob_size == 0 {
        return Err(DirectX12Error::NullObject(
            "D3D12SerializeRootSignature(native resource table blob)",
        ));
    }
    let mut root_signature = std::ptr::null_mut();
    let code = unsafe {
        vtable_method::<CreateRootSignature>(device, 16)(
            device,
            0,
            blob_pointer,
            blob_size,
            &IID_ID3D12_ROOT_SIGNATURE,
            &mut root_signature,
        )
    };
    if code < S_OK {
        return Err(DirectX12Error::HResult {
            operation: "ID3D12Device::CreateRootSignature(native resource table)",
            code,
        });
    }
    if root_signature.is_null() {
        return Err(DirectX12Error::NullObject(
            "CreateRootSignature(native resource table)",
        ));
    }
    Ok(ComObject::new(
        root_signature,
        "ID3D12RootSignature(native-resource-table)",
    ))
}

fn create_compute_pipeline(
    device: *mut c_void,
    dxil: &[u8],
    root_signature: *mut c_void,
) -> Result<ComObject, DirectX12Error> {
    let desc = ComputePipelineStateDesc {
        root_signature,
        compute_shader: ShaderBytecode {
            pointer: dxil.as_ptr() as *const c_void,
            length: dxil.len(),
        },
        cached_pso: CachedPipelineState {
            pointer: std::ptr::null(),
            length: 0,
        },
        flags: D3D12_PIPELINE_STATE_FLAG_NONE,
    };
    let mut pipeline = std::ptr::null_mut();
    let code = unsafe {
        vtable_method::<CreateComputePipelineState>(device, 11)(
            device,
            &desc,
            &IID_ID3D12_PIPELINE_STATE,
            &mut pipeline,
        )
    };
    if code < S_OK {
        return Err(DirectX12Error::HResult {
            operation: "ID3D12Device::CreateComputePipelineState",
            code,
        });
    }
    if pipeline.is_null() {
        return Err(DirectX12Error::NullObject("CreateComputePipelineState"));
    }
    Ok(ComObject::new(pipeline, "ID3D12PipelineState"))
}

fn create_command_allocator(device: *mut c_void) -> Result<ComObject, DirectX12Error> {
    let mut allocator = std::ptr::null_mut();
    let code = unsafe {
        vtable_method::<CreateCommandAllocator>(device, 9)(
            device,
            D3D12_COMMAND_LIST_TYPE_DIRECT,
            &IID_ID3D12_COMMAND_ALLOCATOR,
            &mut allocator,
        )
    };
    if code < S_OK {
        return Err(DirectX12Error::HResult {
            operation: "ID3D12Device::CreateCommandAllocator(compute)",
            code,
        });
    }
    if allocator.is_null() {
        return Err(DirectX12Error::NullObject(
            "CreateCommandAllocator(compute)",
        ));
    }
    Ok(ComObject::new(allocator, "ID3D12CommandAllocator(compute)"))
}

fn create_command_list(
    device: *mut c_void,
    allocator: *mut c_void,
) -> Result<ComObject, DirectX12Error> {
    let mut command_list = std::ptr::null_mut();
    let code = unsafe {
        vtable_method::<CreateCommandList>(device, 12)(
            device,
            0,
            D3D12_COMMAND_LIST_TYPE_DIRECT,
            allocator,
            std::ptr::null_mut(),
            &IID_ID3D12_GRAPHICS_COMMAND_LIST,
            &mut command_list,
        )
    };
    if code < S_OK {
        return Err(DirectX12Error::HResult {
            operation: "ID3D12Device::CreateCommandList(compute)",
            code,
        });
    }
    if command_list.is_null() {
        return Err(DirectX12Error::NullObject("CreateCommandList(compute)"));
    }
    Ok(ComObject::new(
        command_list,
        "ID3D12GraphicsCommandList(compute)",
    ))
}

fn create_fence(device: *mut c_void) -> Result<ComObject, DirectX12Error> {
    let mut fence = std::ptr::null_mut();
    let code = unsafe {
        vtable_method::<CreateFence>(device, 36)(
            device,
            0,
            D3D12_FENCE_FLAG_NONE,
            &IID_ID3D12_FENCE,
            &mut fence,
        )
    };
    if code < S_OK {
        return Err(DirectX12Error::HResult {
            operation: "ID3D12Device::CreateFence(compute)",
            code,
        });
    }
    if fence.is_null() {
        return Err(DirectX12Error::NullObject("CreateFence(compute)"));
    }
    Ok(ComObject::new(fence, "ID3D12Fence(compute)"))
}

fn signal_and_wait(
    queue: *mut c_void,
    fence: *mut c_void,
    target_value: u64,
) -> Result<(), DirectX12Error> {
    let code = unsafe { vtable_method::<QueueSignal>(queue, 14)(queue, fence, target_value) };
    if code < S_OK {
        return Err(DirectX12Error::HResult {
            operation: "ID3D12CommandQueue::Signal(compute)",
            code,
        });
    }
    let get_completed_value = unsafe { vtable_method::<GetCompletedValue>(fence, 8) };
    for _ in 0..500 {
        if unsafe { get_completed_value(fence) } >= target_value {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    Err(DirectX12Error::CompletionTimeout)
}

struct ComObject {
    pointer: *mut c_void,
    name: &'static str,
}

impl ComObject {
    fn new(pointer: *mut c_void, name: &'static str) -> Self {
        Self { pointer, name }
    }

    const fn as_ptr(&self) -> *mut c_void {
        self.pointer
    }
}

impl Drop for ComObject {
    fn drop(&mut self) {
        if !self.pointer.is_null() {
            let release = unsafe { vtable_method::<Release>(self.pointer, 2) };
            let _ = unsafe { release(self.pointer) };
        }
        let _ = self.name;
    }
}

unsafe fn vtable_method<T: Copy>(object: *mut c_void, index: usize) -> T {
    let vtable = unsafe { *(object as *mut *mut *mut c_void) };
    let method = unsafe { *vtable.add(index) };
    unsafe { std::mem::transmute_copy(&method) }
}

#[cfg(test)]
mod tests {
    use super::{
        CommandQueueDesc, DirectX12Error, DxilSourceTranslationReport, DxilUavView,
        DynamicUavLayout, Guid, IID_ID3D12_COMMAND_ALLOCATOR, IID_ID3D12_COMMAND_QUEUE,
        IID_ID3D12_DEVICE, IID_ID3D12_FENCE, IID_ID3D12_GRAPHICS_COMMAND_LIST, SPIRV_OP_CONSTANT,
        SPIRV_OP_CONSTANT_COMPOSITE, SPIRV_OP_FADD, SPIRV_OP_FMUL, SPIRV_OP_FSUB, SPIRV_OP_STORE,
        SPIRV_OP_TYPE_FLOAT, SPIRV_OP_TYPE_VECTOR, SPIRV_OP_ULT, UavBindingPayload,
        execute_spirv_artifact, execute_spirv_source_report,
        execute_spirv_source_report_with_words, hlsl_source_uav_views, run_binary_smoke,
        translate_spirv_artifact_to_dxil_report, translate_spirv_artifact_to_hlsl_source,
        translate_spirv_artifact_to_hlsl_source_report, translate_spirv_source_to_dxil_report,
        validate_artifact_uav_strides, validate_binary_artifact_contract,
        validate_dxil_native_views, validate_external_hlsl_raw_source,
        validate_external_hlsl_source, validate_f32_artifact_contract,
        validate_f32_binary_artifact_contract, validate_f32_vector_artifact_contract,
        validate_f32_vector_lanes_artifact_contract,
        validate_global_2d_strided_write_artifact_contract,
        validate_global_3d_strided_write_artifact_contract,
        validate_global_3d_write_artifact_contract,
        validate_global_strided_write_artifact_contract, validate_global_write_artifact_contract,
        validate_spirv_binary, validate_storage_add_artifact_contract, validate_uav_alias_policy,
    };
    use jadren_codegen_spirv::{
        F32ArithmeticOp, ResourceAccess, ResourceBinding, ResourceElementType, SpirvArtifact,
    };
    use jadren_gpu_runtime::{
        ArtifactSourceBackend, SpirvRawResourceBinding, SpirvSourceTranslationIdentity,
        SpirvSourceTranslationReport, stable_source_hash,
    };
    use jadren_jir::{AddressSpace, BinaryOp, TypeId};

    #[test]
    fn native_abi_layout_and_iids_are_stable() {
        assert_eq!(std::mem::size_of::<Guid>(), 16);
        assert_eq!(std::mem::size_of::<CommandQueueDesc>(), 16);
        assert_eq!(IID_ID3D12_DEVICE.data1, 0x1898_19f1);
        assert_eq!(IID_ID3D12_COMMAND_QUEUE.data1, 0x0ec8_70a6);
        assert_eq!(IID_ID3D12_COMMAND_ALLOCATOR.data1, 0x6102_dee4);
        assert_eq!(IID_ID3D12_GRAPHICS_COMMAND_LIST.data1, 0x5b16_0d0f);
        assert_eq!(IID_ID3D12_FENCE.data1, 0x0a75_3dcf);
    }

    #[test]
    fn validates_spirv_container_structure() {
        assert_eq!(
            validate_spirv_binary(&[0; 5]),
            Err(DirectX12Error::InvalidSpirv("bad-magic"))
        );
        assert_eq!(
            validate_spirv_binary(&[0x0723_0203, 1, 0, 1]),
            Err(DirectX12Error::InvalidSpirv("header-too-short"))
        );
        assert_eq!(
            validate_spirv_binary(&[0x0723_0203, 1, 0, 1, 0]),
            Err(DirectX12Error::InvalidSpirv("structural-validation-failed"))
        );
    }

    #[test]
    fn validates_external_hlsl_resource_bindings() {
        let artifact = SpirvArtifact {
            entry_name: "global_kernel".to_owned(),
            workgroup_size: [1, 1, 1],
            resources: (0..3)
                .map(|binding| ResourceBinding {
                    binding,
                    descriptor_set: 0,
                    name: format!("resource_{binding}"),
                    element_type: TypeId::new(0),
                    element_type_info: None,
                    element_stride: Some(4),
                    address_space: AddressSpace::Storage,
                    access: ResourceAccess::ReadWrite,
                })
                .collect(),
            words: Vec::new(),
        };
        let source = concat!(
            "RWStructuredBuffer<uint> resource_0 : register(u0);\n",
            "RWStructuredBuffer<uint> resource_1 : register(u1, space0);\n",
            "RWStructuredBuffer<uint> resource_2 : register(u2);\n",
            "[numthreads(1, 1, 1)] void global_kernel(uint3 gid : SV_DispatchThreadID) {}"
        );
        let mut typed_artifact = artifact.clone();
        typed_artifact.resources[2].element_type_info = Some(ResourceElementType::Integer {
            signed: false,
            bits: 32,
            lanes: 1,
        });
        assert!(validate_external_hlsl_source(source, &artifact).is_ok());
        assert!(validate_external_hlsl_source(source, &typed_artifact).is_ok());
        let unknown_binding = source.replace(
            "[numthreads",
            "RWStructuredBuffer<uint> extra : register(u3);\n[numthreads",
        );
        assert!(matches!(
            validate_external_hlsl_source(&unknown_binding, &artifact),
            Err(DirectX12Error::SpirvTranslation(_))
        ));
        let mut descriptor_space_artifact = artifact.clone();
        descriptor_space_artifact.resources[2].descriptor_set = 1;
        let descriptor_space_source = source.replace("register(u2)", "register(u2, space1)");
        assert!(
            validate_external_hlsl_source(&descriptor_space_source, &descriptor_space_artifact)
                .is_ok()
        );
        assert!(matches!(
            validate_external_hlsl_source(source, &descriptor_space_artifact),
            Err(DirectX12Error::SpirvTranslation(_))
        ));
        let mut uniform_artifact = artifact.clone();
        uniform_artifact.resources[0].address_space = AddressSpace::Uniform;
        assert!(matches!(
            validate_external_hlsl_source(source, &uniform_artifact),
            Err(DirectX12Error::SpirvTranslation(_))
        ));
        let renamed = source.replace("resource_2 : register(u2)", "renamed_2 : register(u2)");
        assert!(matches!(
            validate_external_hlsl_source(&renamed, &artifact),
            Err(DirectX12Error::SpirvTranslation(_))
        ));
        let signedness_mismatch = source.replace(
            "RWStructuredBuffer<uint> resource_2",
            "RWStructuredBuffer<int> resource_2",
        );
        assert!(matches!(
            validate_external_hlsl_source(&signedness_mismatch, &typed_artifact),
            Err(DirectX12Error::SpirvTranslation(_))
        ));
        for malformed in [
            source.replace("register(u2)", "register(u1)"),
            source.replace("resource_2 : register(u2)", "resource_2 :"),
            source.replace("register(u2)", "register(ux)"),
            source.replace("register(u2)", "register(u2"),
            source.replace("register(u2)", "register(q2)"),
            source.replace("void global_kernel", "void other_kernel"),
            source.replace("register(u2)", "register(u2, space1)"),
            source.replace("register(u2)", "register(u2, spacex)"),
            source.replace("register(u2)", "register(u2, space0, space0)"),
        ] {
            assert!(matches!(
                validate_external_hlsl_source(&malformed, &artifact),
                Err(DirectX12Error::SpirvTranslation(_))
            ));
        }
        let mut read_only_artifact = artifact.clone();
        read_only_artifact.resources[0].access = ResourceAccess::ReadOnly;
        let srv_source = source.replace("register(u0)", "register(t0)");
        assert!(validate_external_hlsl_source(&srv_source, &read_only_artifact).is_ok());
        let constant_source = source.replace("register(u0)", "register(b0)");
        assert!(matches!(
            validate_external_hlsl_source(&constant_source, &read_only_artifact),
            Err(DirectX12Error::SpirvTranslation(_))
        ));
        assert!(matches!(
            validate_external_hlsl_source(source, &read_only_artifact),
            Err(DirectX12Error::SpirvTranslation(_))
        ));
        let stride_mismatch = source.replace(
            "RWStructuredBuffer<uint> resource_2",
            "RWStructuredBuffer<uint2> resource_2",
        );
        assert!(matches!(
            validate_external_hlsl_source(&stride_mismatch, &artifact),
            Err(DirectX12Error::SpirvTranslation(_))
        ));
        let unsupported_type = source.replace(
            "RWStructuredBuffer<uint> resource_2",
            "RWStructuredBuffer<CustomElement> resource_2",
        );
        assert!(matches!(
            validate_external_hlsl_source(&unsupported_type, &artifact),
            Err(DirectX12Error::SpirvTranslation(_))
        ));
        let mut composite_artifact = artifact.clone();
        composite_artifact.resources[2].element_stride = None;
        assert!(validate_external_hlsl_source(&unsupported_type, &composite_artifact).is_ok());
    }

    #[test]
    fn hlsl_source_maps_mixed_views_by_dense_binding() {
        let artifact = SpirvArtifact {
            entry_name: "main".to_owned(),
            workgroup_size: [1, 1, 1],
            resources: (0..2)
                .map(|binding| ResourceBinding {
                    binding,
                    descriptor_set: 0,
                    name: format!("resource_{binding}"),
                    element_type: TypeId::new(0),
                    element_type_info: None,
                    element_stride: Some(4),
                    address_space: AddressSpace::Storage,
                    access: ResourceAccess::ReadWrite,
                })
                .collect(),
            words: Vec::new(),
        };
        let source = concat!(
            "RWByteAddressBuffer resource_0 : register(u0);\n",
            "RWStructuredBuffer<uint> resource_1 : register(u1);\n",
            "[numthreads(1, 1, 1)] void main() {}"
        );
        assert_eq!(
            hlsl_source_uav_views(source, &artifact),
            Ok(vec![DxilUavView::Raw, DxilUavView::Structured])
        );
    }

    #[test]
    fn hlsl_source_maps_read_only_srv_and_writable_uav_views() {
        let artifact = SpirvArtifact {
            entry_name: "mixed_views".to_owned(),
            workgroup_size: [1, 1, 1],
            resources: vec![
                ResourceBinding {
                    binding: 0,
                    descriptor_set: 0,
                    name: "input".to_owned(),
                    element_type: TypeId::new(0),
                    element_type_info: Some(ResourceElementType::Integer {
                        signed: false,
                        bits: 32,
                        lanes: 1,
                    }),
                    element_stride: Some(4),
                    address_space: AddressSpace::Storage,
                    access: ResourceAccess::ReadOnly,
                },
                ResourceBinding {
                    binding: 1,
                    descriptor_set: 0,
                    name: "output".to_owned(),
                    element_type: TypeId::new(0),
                    element_type_info: Some(ResourceElementType::Integer {
                        signed: false,
                        bits: 32,
                        lanes: 1,
                    }),
                    element_stride: Some(4),
                    address_space: AddressSpace::Storage,
                    access: ResourceAccess::ReadWrite,
                },
            ],
            words: Vec::new(),
        };
        let source = concat!(
            "StructuredBuffer<uint> input : register(t0);\n",
            "RWStructuredBuffer<uint> output : register(u1);\n",
            "[numthreads(1, 1, 1)] void mixed_views() {}"
        );
        assert_eq!(
            hlsl_source_uav_views(source, &artifact),
            Ok(vec![DxilUavView::StructuredSrv, DxilUavView::Structured])
        );
        assert!(
            validate_dxil_native_views(
                &artifact,
                &[DxilUavView::StructuredSrv, DxilUavView::Structured],
                1
            )
            .is_ok()
        );
    }

    #[test]
    fn validates_raw_hlsl_source_report_mixed_access_and_descriptor_space() {
        let element = ResourceElementType::Integer {
            signed: false,
            bits: 32,
            lanes: 1,
        };
        let source = concat!(
            "StructuredBuffer<uint> input : register(t0);\n",
            "RWByteAddressBuffer output : register(u1, space2);\n",
            "[numthreads(1, 1, 1)] void raw_entry(uint3 gid : SV_DispatchThreadID) {}"
        )
        .to_owned();
        let report = SpirvSourceTranslationReport {
            identity: SpirvSourceTranslationIdentity {
                backend: ArtifactSourceBackend::Hlsl,
                entry_name: "raw_entry".to_owned(),
                execution_model: 5,
                workgroup_size: Some([1, 1, 1]),
                workgroup_size_ids: None,
                workgroup_size_spec_ids: None,
                resources: vec![
                    SpirvRawResourceBinding {
                        variable_id: 10,
                        binding: 0,
                        descriptor_set: 0,
                        storage_class: Some(2),
                        element_type: Some(element),
                        element_stride: Some(4),
                        access: Some(ResourceAccess::ReadOnly),
                    },
                    SpirvRawResourceBinding {
                        variable_id: 11,
                        binding: 1,
                        descriptor_set: 2,
                        storage_class: Some(12),
                        element_type: Some(element),
                        element_stride: Some(4),
                        access: Some(ResourceAccess::ReadWrite),
                    },
                ],
                word_count: 41,
                word_hash: 7,
            },
            source_byte_count: source.len(),
            source_hash: 11,
            source,
        };
        assert_eq!(
            validate_external_hlsl_raw_source(&report),
            Ok(vec![DxilUavView::StructuredSrv, DxilUavView::Raw])
        );
        let wrong_space = report
            .source
            .replace("register(u1, space2)", "register(u1, space1)");
        let wrong_space_report = SpirvSourceTranslationReport {
            source: wrong_space,
            ..report.clone()
        };
        assert!(matches!(
            validate_external_hlsl_raw_source(&wrong_space_report),
            Err(DirectX12Error::SpirvTranslation(_))
        ));
        let wrong_access = report.source.replace("register(t0)", "register(u0)");
        let wrong_access_report = SpirvSourceTranslationReport {
            source: wrong_access,
            ..report
        };
        assert!(matches!(
            validate_external_hlsl_raw_source(&wrong_access_report),
            Err(DirectX12Error::SpirvTranslation(_))
        ));
    }

    #[test]
    fn raw_source_report_executor_rejects_tampered_contract_before_native_load() {
        let source = "[numthreads(1, 1, 1)] void raw_entry(uint3 gid : SV_DispatchThreadID) {}";
        let raw_source = SpirvSourceTranslationReport {
            identity: SpirvSourceTranslationIdentity {
                backend: ArtifactSourceBackend::Hlsl,
                entry_name: "raw_entry".to_owned(),
                execution_model: 5,
                workgroup_size: Some([1, 1, 1]),
                workgroup_size_ids: None,
                workgroup_size_spec_ids: None,
                resources: vec![SpirvRawResourceBinding {
                    variable_id: 10,
                    binding: 0,
                    descriptor_set: 0,
                    storage_class: Some(12),
                    element_type: Some(ResourceElementType::Integer {
                        signed: false,
                        bits: 32,
                        lanes: 1,
                    }),
                    element_stride: Some(4),
                    access: Some(ResourceAccess::ReadWrite),
                }],
                word_count: 41,
                word_hash: 7,
            },
            source_byte_count: source.len(),
            source_hash: stable_source_hash(source),
            source: source.to_owned(),
        };
        let report = DxilSourceTranslationReport {
            source: raw_source,
            dxil: Vec::new(),
            resource_views: vec![DxilUavView::Structured],
        };
        let bytes = [0_u8; 4];
        let bindings = [UavBindingPayload {
            resource_id: 1,
            bytes: &bytes,
            element_stride: 4,
        }];
        assert_eq!(
            execute_spirv_source_report(&report, [1, 1, 1], &bindings, 0),
            Err(DirectX12Error::InvalidDxil)
        );
        assert_eq!(
            execute_spirv_source_report_with_words(
                &report,
                &[0; 5],
                [1, 1, 1],
                &bindings,
                0,
                &std::collections::BTreeMap::new(),
            ),
            Err(DirectX12Error::InvalidSpirv("raw-source-word-count"))
        );
    }

    #[test]
    fn raw_source_report_executor_rejects_source_hash_mismatch_before_native_load() {
        let source = "[numthreads(1, 1, 1)] void raw_entry(uint3 gid : SV_DispatchThreadID) {}";
        let raw_source = SpirvSourceTranslationReport {
            identity: SpirvSourceTranslationIdentity {
                backend: ArtifactSourceBackend::Hlsl,
                entry_name: "raw_entry".to_owned(),
                execution_model: 5,
                workgroup_size: Some([1, 1, 1]),
                workgroup_size_ids: None,
                workgroup_size_spec_ids: None,
                resources: Vec::new(),
                word_count: 41,
                word_hash: 7,
            },
            source_byte_count: source.len(),
            source_hash: 0,
            source: source.to_owned(),
        };
        let report = DxilSourceTranslationReport {
            source: raw_source,
            dxil: Vec::new(),
            resource_views: Vec::new(),
        };
        assert!(matches!(
            execute_spirv_source_report(&report, [1, 1, 1], &[], 0),
            Err(DirectX12Error::SpirvTranslation(_))
        ));
    }

    #[test]
    fn hlsl_source_boundary_rejects_invalid_artifact_before_toolchain_lookup() {
        let artifact = SpirvArtifact {
            entry_name: "global_kernel".to_owned(),
            workgroup_size: [1, 1, 1],
            resources: Vec::new(),
            words: vec![0],
        };
        assert_eq!(
            translate_spirv_artifact_to_hlsl_source(&artifact),
            Err(DirectX12Error::InvalidSpirv("shared-artifact-contract"))
        );
        assert_eq!(
            translate_spirv_artifact_to_hlsl_source_report(&artifact),
            Err(DirectX12Error::InvalidSpirv("shared-artifact-contract"))
        );
        assert_eq!(
            translate_spirv_artifact_to_dxil_report(&artifact),
            Err(DirectX12Error::InvalidSpirv("shared-artifact-contract"))
        );
        assert_eq!(
            translate_spirv_source_to_dxil_report(&[0; 5], "invalid"),
            Err(DirectX12Error::InvalidSpirv("invalid SPIR-V magic"))
        );
    }

    #[test]
    fn generic_spirv_executor_rejects_invalid_artifact_before_native_load() {
        let artifact = SpirvArtifact {
            entry_name: "invalid".to_owned(),
            workgroup_size: [1, 1, 1],
            resources: Vec::new(),
            words: Vec::new(),
        };
        assert_eq!(
            execute_spirv_artifact(&artifact, [1, 1, 1], &[], 0),
            Err(DirectX12Error::InvalidSpirv("shared-artifact-contract"))
        );
    }

    #[test]
    fn rejects_unsafe_binary_operands_before_native_load() {
        assert_eq!(
            run_binary_smoke(BinaryOp::Divide, 0),
            Err(DirectX12Error::InvalidBinaryOperand(
                "unsigned divisor/remainder must be non-zero",
            ))
        );
        assert_eq!(
            run_binary_smoke(BinaryOp::ShiftRight, 32),
            Err(DirectX12Error::InvalidBinaryOperand(
                "u32 shift operand must be smaller than 32",
            ))
        );
    }

    #[test]
    fn artifact_decoder_pins_binary_opcode_and_operand() {
        let resources = (0..3)
            .map(|binding| ResourceBinding {
                binding,
                descriptor_set: 0,
                name: format!("resource_{binding}"),
                element_type: TypeId::new(0),
                element_type_info: None,
                element_stride: Some(4),
                address_space: AddressSpace::Storage,
                access: ResourceAccess::ReadWrite,
            })
            .collect();
        let artifact = SpirvArtifact {
            entry_name: "global_multiply_dynamic_u32".to_owned(),
            workgroup_size: [64, 1, 1],
            resources,
            words: vec![
                0x0723_0203,
                0x0001_0300,
                0,
                32,
                0,
                (4_u32 << 16) | 43,
                1,
                9,
                3,
                (5_u32 << 16) | 132,
                1,
                10,
                11,
                9,
            ],
        };
        assert!(validate_binary_artifact_contract(&artifact, BinaryOp::Multiply, 3).is_ok());
        assert_eq!(
            validate_binary_artifact_contract(&artifact, BinaryOp::Multiply, 2),
            Err(DirectX12Error::ArtifactContract(
                "artifact operand differs from execution request",
            ))
        );
    }

    #[test]
    fn artifact_decoder_pins_f32_addend_and_shape() {
        let artifact = SpirvArtifact {
            entry_name: "global_add_dynamic_f32".to_owned(),
            workgroup_size: [64, 1, 1],
            resources: (0..3)
                .map(|binding| ResourceBinding {
                    binding,
                    descriptor_set: 0,
                    name: format!("resource_{binding}"),
                    element_type: TypeId::new(0),
                    element_type_info: None,
                    element_stride: Some(4),
                    address_space: AddressSpace::Storage,
                    access: ResourceAccess::ReadWrite,
                })
                .collect(),
            words: vec![
                0x0723_0203,
                0x0001_0300,
                0,
                32,
                0,
                (3_u32 << 16) | u32::from(SPIRV_OP_TYPE_FLOAT),
                9,
                32,
                (4_u32 << 16) | u32::from(SPIRV_OP_CONSTANT),
                9,
                10,
                0x3f80_0000,
                (5_u32 << 16) | u32::from(SPIRV_OP_FADD),
                9,
                11,
                12,
                10,
                (5_u32 << 16) | u32::from(SPIRV_OP_ULT),
                1,
                13,
                14,
                15,
                (3_u32 << 16) | u32::from(SPIRV_OP_STORE),
                16,
                11,
            ],
        };
        assert!(validate_f32_artifact_contract(&artifact, 0x3f80_0000).is_ok());
        assert_eq!(
            validate_f32_artifact_contract(&artifact, 0x4000_0000),
            Err(DirectX12Error::ArtifactContract(
                "f32 artifact addend differs from execution request",
            ))
        );
    }

    #[test]
    fn artifact_decoder_accepts_f32_subtract_and_multiply_shapes() {
        for (operation, opcode) in [
            (F32ArithmeticOp::Subtract, SPIRV_OP_FSUB),
            (F32ArithmeticOp::Multiply, SPIRV_OP_FMUL),
        ] {
            let artifact = SpirvArtifact {
                entry_name: "global_binary_dynamic_f32".to_owned(),
                workgroup_size: [64, 1, 1],
                resources: (0..3)
                    .map(|binding| ResourceBinding {
                        binding,
                        descriptor_set: 0,
                        name: format!("resource_{binding}"),
                        element_type: TypeId::new(0),
                        element_type_info: None,
                        element_stride: Some(4),
                        address_space: AddressSpace::Storage,
                        access: ResourceAccess::ReadWrite,
                    })
                    .collect(),
                words: vec![
                    0x0723_0203,
                    0x0001_0300,
                    0,
                    32,
                    0,
                    (3_u32 << 16) | u32::from(SPIRV_OP_TYPE_FLOAT),
                    9,
                    32,
                    (4_u32 << 16) | u32::from(SPIRV_OP_CONSTANT),
                    9,
                    10,
                    0x3f80_0000,
                    (5_u32 << 16) | u32::from(opcode),
                    9,
                    11,
                    12,
                    10,
                    (5_u32 << 16) | u32::from(SPIRV_OP_ULT),
                    1,
                    13,
                    14,
                    15,
                    (3_u32 << 16) | u32::from(SPIRV_OP_STORE),
                    16,
                    11,
                ],
            };
            assert!(
                validate_f32_binary_artifact_contract(&artifact, 0x3f80_0000, operation,).is_ok()
            );
        }
    }

    #[test]
    fn artifact_decoder_pins_f32x4_stride_and_splat_shape() {
        let artifact = SpirvArtifact {
            entry_name: "global_add_dynamic_f32x4".to_owned(),
            workgroup_size: [64, 1, 1],
            resources: [16_u32, 16, 4]
                .into_iter()
                .enumerate()
                .map(|(binding, stride)| ResourceBinding {
                    binding: binding as u32,
                    descriptor_set: 0,
                    name: format!("resource_{binding}"),
                    element_type: TypeId::new(0),
                    element_type_info: None,
                    element_stride: Some(stride),
                    address_space: AddressSpace::Storage,
                    access: ResourceAccess::ReadWrite,
                })
                .collect(),
            words: vec![
                0x0723_0203,
                0x0001_0300,
                0,
                32,
                0,
                (3_u32 << 16) | u32::from(SPIRV_OP_TYPE_FLOAT),
                9,
                32,
                (4_u32 << 16) | u32::from(SPIRV_OP_TYPE_VECTOR),
                10,
                9,
                4,
                (4_u32 << 16) | u32::from(SPIRV_OP_CONSTANT),
                9,
                11,
                0x3f80_0000,
                (4_u32 << 16) | u32::from(SPIRV_OP_CONSTANT),
                9,
                12,
                0x3f80_0000,
                (4_u32 << 16) | u32::from(SPIRV_OP_CONSTANT),
                9,
                13,
                0x3f80_0000,
                (4_u32 << 16) | u32::from(SPIRV_OP_CONSTANT),
                9,
                14,
                0x3f80_0000,
                (7_u32 << 16) | u32::from(SPIRV_OP_CONSTANT_COMPOSITE),
                10,
                15,
                11,
                12,
                13,
                14,
                (5_u32 << 16) | u32::from(SPIRV_OP_FADD),
                10,
                16,
                17,
                15,
                (5_u32 << 16) | u32::from(SPIRV_OP_ULT),
                1,
                18,
                19,
                20,
                (3_u32 << 16) | u32::from(SPIRV_OP_STORE),
                21,
                16,
            ],
        };
        assert!(
            validate_f32_vector_artifact_contract(&artifact, 0x3f80_0000, F32ArithmeticOp::Add)
                .is_ok()
        );
        let mut wrong_stride = artifact.clone();
        wrong_stride.resources[0].element_stride = Some(4);
        assert_eq!(
            validate_f32_vector_artifact_contract(&wrong_stride, 0x3f80_0000, F32ArithmeticOp::Add,),
            Err(DirectX12Error::ArtifactContract(
                "runtime-length f32x4 artifact requires ordered storage strides 16/16/4",
            ))
        );
    }

    #[test]
    fn artifact_decoder_accepts_native_f32x2_and_f32x3_lane_shapes() {
        for lanes in [2_u32, 3_u32] {
            let resources = [lanes * 4, lanes * 4, 4]
                .into_iter()
                .enumerate()
                .map(|(binding, stride)| ResourceBinding {
                    binding: binding as u32,
                    descriptor_set: 0,
                    name: format!("resource_{binding}"),
                    element_type: TypeId::new(0),
                    element_type_info: None,
                    element_stride: Some(stride),
                    address_space: AddressSpace::Storage,
                    access: ResourceAccess::ReadWrite,
                })
                .collect();
            let mut words = vec![
                0x0723_0203,
                0x0001_0300,
                0,
                32,
                0,
                (3_u32 << 16) | u32::from(SPIRV_OP_TYPE_FLOAT),
                9,
                32,
                (4_u32 << 16) | u32::from(SPIRV_OP_TYPE_VECTOR),
                10,
                9,
                lanes,
            ];
            let mut constant_ids = Vec::new();
            for index in 0..lanes {
                let id = 11 + index;
                constant_ids.push(id);
                words.extend([
                    (4_u32 << 16) | u32::from(SPIRV_OP_CONSTANT),
                    9,
                    id,
                    0x3f80_0000,
                ]);
            }
            words.push((3 + lanes) << 16 | u32::from(SPIRV_OP_CONSTANT_COMPOSITE));
            words.extend([10, 20]);
            words.extend(constant_ids);
            words.extend([
                (5_u32 << 16) | u32::from(SPIRV_OP_FADD),
                10,
                21,
                22,
                20,
                (5_u32 << 16) | u32::from(SPIRV_OP_ULT),
                1,
                23,
                24,
                25,
                (3_u32 << 16) | u32::from(SPIRV_OP_STORE),
                26,
                21,
            ]);
            let artifact = SpirvArtifact {
                entry_name: format!("global_add_dynamic_f32x{lanes}"),
                workgroup_size: [64, 1, 1],
                resources,
                words,
            };
            assert!(
                validate_f32_vector_lanes_artifact_contract(
                    &artifact,
                    0x3f80_0000,
                    F32ArithmeticOp::Add,
                    lanes,
                )
                .is_ok()
            );
        }
    }

    #[test]
    fn storage_add_artifact_decoder_pins_single_resource_and_addend() {
        let artifact = SpirvArtifact {
            entry_name: "add_u32".to_owned(),
            workgroup_size: [1, 1, 1],
            resources: vec![ResourceBinding {
                binding: 0,
                descriptor_set: 0,
                name: "data".to_owned(),
                element_type: TypeId::new(0),
                element_type_info: None,
                element_stride: Some(4),
                address_space: AddressSpace::Storage,
                access: ResourceAccess::ReadWrite,
            }],
            words: vec![
                0x0723_0203,
                0x0001_0300,
                0,
                32,
                0,
                (4_u32 << 16) | 43,
                1,
                9,
                1,
                (5_u32 << 16) | 128,
                1,
                10,
                11,
                9,
            ],
        };
        assert!(validate_storage_add_artifact_contract(&artifact, 1).is_ok());
        assert_eq!(
            validate_storage_add_artifact_contract(&artifact, 2),
            Err(DirectX12Error::ArtifactContract(
                "storage-add artifact addend differs from execution request",
            ))
        );
    }

    #[test]
    fn global_write_artifact_decoder_pins_bounds_and_stored_value() {
        let artifact = SpirvArtifact {
            entry_name: "global_write_u32".to_owned(),
            workgroup_size: [64, 1, 1],
            resources: vec![ResourceBinding {
                binding: 0,
                descriptor_set: 0,
                name: "buffer".to_owned(),
                element_type: TypeId::new(0),
                element_type_info: None,
                element_stride: Some(4),
                address_space: AddressSpace::Storage,
                access: ResourceAccess::ReadWrite,
            }],
            words: vec![
                0x0723_0203,
                0x0001_0300,
                0,
                32,
                0,
                (4_u32 << 16) | 43,
                1,
                9,
                64,
                (4_u32 << 16) | 43,
                1,
                10,
                42,
                (5_u32 << 16) | 176,
                2,
                20,
                15,
                9,
                (3_u32 << 16) | 62,
                23,
                10,
            ],
        };
        assert!(validate_global_write_artifact_contract(&artifact, 42, 64).is_ok());
        assert_eq!(
            validate_global_write_artifact_contract(&artifact, 43, 64),
            Err(DirectX12Error::ArtifactContract(
                "global-write artifact value differs from execution request",
            ))
        );
    }

    #[test]
    fn global_strided_artifact_decoder_pins_bounds_stride_and_stored_value() {
        let resources = (0..4)
            .map(|binding| ResourceBinding {
                binding,
                descriptor_set: 0,
                name: format!("resource_{binding}"),
                element_type: TypeId::new(0),
                element_type_info: None,
                element_stride: Some(4),
                address_space: AddressSpace::Storage,
                access: ResourceAccess::ReadWrite,
            })
            .collect();
        let artifact = SpirvArtifact {
            entry_name: "global_strided_write_u32".to_owned(),
            workgroup_size: [64, 1, 1],
            resources,
            words: vec![
                0x0723_0203,
                0x0001_0300,
                0,
                64,
                0,
                (4_u32 << 16) | 43,
                1,
                9,
                42,
                (5_u32 << 16) | 176,
                2,
                20,
                11,
                12,
                (5_u32 << 16) | 132,
                1,
                21,
                11,
                13,
                (5_u32 << 16) | 176,
                2,
                22,
                21,
                14,
                (3_u32 << 16) | 62,
                23,
                9,
            ],
        };
        assert!(validate_global_strided_write_artifact_contract(&artifact, 42).is_ok());
        assert_eq!(
            validate_global_strided_write_artifact_contract(&artifact, 43),
            Err(DirectX12Error::ArtifactContract(
                "global-strided-write artifact value differs from execution request",
            ))
        );
    }

    #[test]
    fn global_2d_strided_artifact_decoder_pins_affine_shape_and_value() {
        let resources = (0..6)
            .map(|binding| ResourceBinding {
                binding,
                descriptor_set: 0,
                name: format!("resource_{binding}"),
                element_type: TypeId::new(0),
                element_type_info: None,
                element_stride: Some(4),
                address_space: AddressSpace::Storage,
                access: ResourceAccess::ReadWrite,
            })
            .collect();
        let artifact = SpirvArtifact {
            entry_name: "global_2d_strided_write_u32".to_owned(),
            workgroup_size: [4, 4, 1],
            resources,
            words: vec![
                0x0723_0203,
                0x0001_0300,
                0,
                64,
                0,
                (4_u32 << 16) | 43,
                1,
                9,
                42,
                (5_u32 << 16) | 176,
                2,
                20,
                11,
                12,
                (5_u32 << 16) | 176,
                2,
                21,
                13,
                14,
                (5_u32 << 16) | 176,
                2,
                22,
                15,
                16,
                (5_u32 << 16) | 132,
                1,
                23,
                11,
                17,
                (5_u32 << 16) | 132,
                1,
                24,
                13,
                18,
                (5_u32 << 16) | 128,
                1,
                25,
                23,
                24,
                (3_u32 << 16) | 62,
                26,
                9,
            ],
        };
        assert!(validate_global_2d_strided_write_artifact_contract(&artifact, 42).is_ok());
        assert_eq!(
            validate_global_2d_strided_write_artifact_contract(&artifact, 43),
            Err(DirectX12Error::ArtifactContract(
                "global-2d-strided-write artifact value differs from execution request",
            ))
        );
    }

    #[test]
    fn global_3d_artifact_decoder_pins_row_major_shape_and_value() {
        let resources = (0..5)
            .map(|binding| ResourceBinding {
                binding,
                descriptor_set: 0,
                name: format!("resource_{binding}"),
                element_type: TypeId::new(0),
                element_type_info: None,
                element_stride: Some(4),
                address_space: AddressSpace::Storage,
                access: ResourceAccess::ReadWrite,
            })
            .collect();
        let artifact = SpirvArtifact {
            entry_name: "global_3d_write_u32".to_owned(),
            workgroup_size: [4, 4, 2],
            resources,
            words: vec![
                0x0723_0203,
                0x0001_0300,
                0,
                64,
                0,
                (4_u32 << 16) | 43,
                1,
                9,
                42,
                (5_u32 << 16) | 176,
                2,
                20,
                11,
                12,
                (5_u32 << 16) | 176,
                2,
                21,
                13,
                14,
                (5_u32 << 16) | 176,
                2,
                22,
                15,
                16,
                (5_u32 << 16) | 176,
                2,
                23,
                17,
                18,
                (5_u32 << 16) | 132,
                1,
                24,
                11,
                13,
                (5_u32 << 16) | 132,
                1,
                25,
                24,
                12,
                (5_u32 << 16) | 128,
                1,
                26,
                25,
                14,
                (5_u32 << 16) | 128,
                1,
                27,
                26,
                15,
                (3_u32 << 16) | 62,
                28,
                9,
            ],
        };
        assert!(validate_global_3d_write_artifact_contract(&artifact, 42).is_ok());
        assert_eq!(
            validate_global_3d_write_artifact_contract(&artifact, 43),
            Err(DirectX12Error::ArtifactContract(
                "global-3d-write artifact value differs from execution request",
            ))
        );
    }

    #[test]
    fn global_3d_strided_artifact_decoder_pins_affine_shape_and_value() {
        let resources = (0..8)
            .map(|binding| ResourceBinding {
                binding,
                descriptor_set: 0,
                name: format!("resource_{binding}"),
                element_type: TypeId::new(0),
                element_type_info: None,
                element_stride: Some(4),
                address_space: AddressSpace::Storage,
                access: ResourceAccess::ReadWrite,
            })
            .collect();
        let artifact = SpirvArtifact {
            entry_name: "global_3d_strided_write_u32".to_owned(),
            workgroup_size: [4, 4, 2],
            resources,
            words: vec![
                0x0723_0203,
                0x0001_0300,
                0,
                64,
                0,
                (4_u32 << 16) | 43,
                1,
                9,
                42,
                (5_u32 << 16) | 176,
                2,
                20,
                11,
                12,
                (5_u32 << 16) | 176,
                2,
                21,
                13,
                14,
                (5_u32 << 16) | 176,
                2,
                22,
                15,
                16,
                (5_u32 << 16) | 176,
                2,
                23,
                17,
                18,
                (5_u32 << 16) | 132,
                1,
                24,
                11,
                13,
                (5_u32 << 16) | 132,
                1,
                25,
                12,
                14,
                (5_u32 << 16) | 132,
                1,
                26,
                15,
                17,
                (5_u32 << 16) | 128,
                1,
                27,
                24,
                25,
                (5_u32 << 16) | 128,
                1,
                28,
                27,
                26,
                (3_u32 << 16) | 62,
                29,
                9,
            ],
        };
        assert!(validate_global_3d_strided_write_artifact_contract(&artifact, 42).is_ok());
        assert_eq!(
            validate_global_3d_strided_write_artifact_contract(&artifact, 43),
            Err(DirectX12Error::ArtifactContract(
                "global-3d-strided-write artifact value differs from execution request",
            ))
        );
    }

    #[test]
    fn generic_uav_request_pins_dense_payloads_and_geometry() {
        let first = vec![0_u8; 16];
        let second = vec![7_u8; 4];
        let bindings = vec![
            UavBindingPayload {
                resource_id: 0,
                bytes: &first,
                element_stride: 4,
            },
            UavBindingPayload {
                resource_id: 1,
                bytes: &second,
                element_stride: 4,
            },
        ];
        assert_eq!(
            super::validate_dynamic_uav_request(2, [1, 2, 1], &bindings, 1).unwrap(),
            vec![
                DynamicUavLayout {
                    byte_size: 16,
                    element_count: 4,
                    element_stride: 4,
                },
                DynamicUavLayout {
                    byte_size: 4,
                    element_count: 1,
                    element_stride: 4,
                },
            ]
        );
        assert_eq!(
            super::validate_dynamic_uav_request(1, [1, 1, 1], &bindings, 0),
            Err(DirectX12Error::ArtifactContract(
                "generic DX12 UAV dispatch requires one payload per non-empty artifact resource",
            ))
        );
        assert_eq!(
            super::validate_dynamic_uav_request(2, [1, 0, 1], &bindings, 0),
            Err(DirectX12Error::ArtifactContract(
                "generic DX12 UAV dispatch requires non-zero workgroup dimensions",
            ))
        );
        assert_eq!(
            super::validate_dynamic_uav_request(2, [1, 1, 1], &bindings, 2),
            Err(DirectX12Error::ArtifactContract(
                "generic DX12 UAV dispatch output binding is out of range",
            ))
        );
    }

    #[test]
    fn generic_uav_request_rejects_misaligned_and_oversized_stride() {
        let bytes = [0_u8; 10];
        let misaligned = [UavBindingPayload {
            resource_id: 0,
            bytes: &bytes,
            element_stride: 4,
        }];
        assert_eq!(
            super::validate_dynamic_uav_request(1, [1, 1, 1], &misaligned, 0),
            Err(DirectX12Error::ArtifactContract(
                "generic DX12 UAV payload bytes must align to element stride",
            ))
        );

        let oversized = [UavBindingPayload {
            resource_id: 0,
            bytes: &[0_u8; 1],
            element_stride: 2049,
        }];
        assert_eq!(
            super::validate_dynamic_uav_request(1, [1, 1, 1], &oversized, 0),
            Err(DirectX12Error::ArtifactContract(
                "generic DX12 UAV element stride must be between 1 and 2048 bytes",
            ))
        );
    }

    #[test]
    fn generic_uav_payload_stride_matches_reflected_resource_type() {
        let artifact = SpirvArtifact {
            entry_name: "typed_uav".to_owned(),
            workgroup_size: [1, 1, 1],
            resources: vec![ResourceBinding {
                binding: 0,
                descriptor_set: 0,
                name: "data".to_owned(),
                element_type: TypeId::new(0),
                element_type_info: None,
                element_stride: Some(8),
                address_space: AddressSpace::Storage,
                access: ResourceAccess::ReadWrite,
            }],
            words: Vec::new(),
        };
        let bytes = [0_u8; 16];
        let payload = [UavBindingPayload {
            resource_id: 0,
            bytes: &bytes,
            element_stride: 8,
        }];
        assert!(validate_artifact_uav_strides(&artifact, &payload).is_ok());

        let wrong_payload = [UavBindingPayload {
            resource_id: 0,
            bytes: &bytes,
            element_stride: 4,
        }];
        assert_eq!(
            validate_artifact_uav_strides(&artifact, &wrong_payload),
            Err(DirectX12Error::ArtifactContract(
                "generic DX12 UAV payload stride differs from artifact resource type",
            ))
        );
    }

    #[test]
    fn generic_native_view_gate_matches_access_before_api_creation() {
        let artifact = SpirvArtifact {
            entry_name: "readonly_uav_gate".to_owned(),
            workgroup_size: [1, 1, 1],
            resources: vec![ResourceBinding {
                binding: 0,
                descriptor_set: 0,
                name: "data".to_owned(),
                element_type: TypeId::new(0),
                element_type_info: Some(ResourceElementType::Integer {
                    signed: false,
                    bits: 32,
                    lanes: 1,
                }),
                element_stride: Some(4),
                address_space: AddressSpace::Uniform,
                access: ResourceAccess::ReadOnly,
            }],
            words: Vec::new(),
        };
        assert!(validate_dxil_native_views(&artifact, &[DxilUavView::StructuredSrv], 0).is_err());
        let writable_output = SpirvArtifact {
            resources: vec![ResourceBinding {
                access: ResourceAccess::ReadWrite,
                address_space: AddressSpace::Storage,
                ..artifact.resources[0].clone()
            }],
            ..artifact
        };
        assert!(
            validate_dxil_native_views(&writable_output, &[DxilUavView::Structured], 0).is_ok()
        );
    }

    #[test]
    fn generic_uav_alias_policy_allows_matching_read_only_aliases() {
        let artifact = SpirvArtifact {
            entry_name: "readonly_alias".to_owned(),
            workgroup_size: [1, 1, 1],
            resources: (0..2)
                .map(|binding| ResourceBinding {
                    binding,
                    descriptor_set: 0,
                    name: format!("resource_{binding}"),
                    element_type: TypeId::new(0),
                    element_type_info: None,
                    element_stride: Some(4),
                    address_space: AddressSpace::Uniform,
                    access: ResourceAccess::ReadOnly,
                })
                .collect(),
            words: Vec::new(),
        };
        let bytes = [1_u8; 8];
        let bindings = [
            UavBindingPayload {
                resource_id: 7,
                bytes: &bytes,
                element_stride: 4,
            },
            UavBindingPayload {
                resource_id: 7,
                bytes: &bytes,
                element_stride: 4,
            },
        ];
        assert!(validate_uav_alias_policy(&artifact, &bindings).is_ok());
    }

    #[test]
    fn generic_uav_alias_policy_rejects_writable_or_inconsistent_aliases() {
        let artifact = SpirvArtifact {
            entry_name: "write_alias".to_owned(),
            workgroup_size: [1, 1, 1],
            resources: (0..2)
                .map(|binding| ResourceBinding {
                    binding,
                    descriptor_set: 0,
                    name: format!("resource_{binding}"),
                    element_type: TypeId::new(0),
                    element_type_info: None,
                    element_stride: Some(4),
                    address_space: if binding == 0 {
                        AddressSpace::Storage
                    } else {
                        AddressSpace::Uniform
                    },
                    access: if binding == 0 {
                        ResourceAccess::ReadWrite
                    } else {
                        ResourceAccess::ReadOnly
                    },
                })
                .collect(),
            words: Vec::new(),
        };
        let first = [1_u8; 8];
        let second = [2_u8; 8];
        let bindings = [
            UavBindingPayload {
                resource_id: 9,
                bytes: &first,
                element_stride: 4,
            },
            UavBindingPayload {
                resource_id: 9,
                bytes: &first,
                element_stride: 4,
            },
        ];
        assert_eq!(
            validate_uav_alias_policy(&artifact, &bindings),
            Err(DirectX12Error::ArtifactContract(
                "generic DX12 writable UAV resource aliases are forbidden",
            ))
        );

        let write_only_second = SpirvArtifact {
            resources: artifact
                .resources
                .iter()
                .enumerate()
                .map(|(index, resource)| ResourceBinding {
                    address_space: if index == 1 {
                        AddressSpace::Storage
                    } else {
                        AddressSpace::Uniform
                    },
                    access: if index == 1 {
                        ResourceAccess::WriteOnly
                    } else {
                        ResourceAccess::ReadOnly
                    },
                    ..resource.clone()
                })
                .collect(),
            ..artifact.clone()
        };
        assert_eq!(
            validate_uav_alias_policy(&write_only_second, &bindings),
            Err(DirectX12Error::ArtifactContract(
                "generic DX12 writable UAV resource aliases are forbidden",
            ))
        );

        let readonly_artifact = SpirvArtifact {
            resources: artifact
                .resources
                .iter()
                .map(|resource| ResourceBinding {
                    access: ResourceAccess::ReadOnly,
                    ..resource.clone()
                })
                .collect(),
            ..artifact
        };
        let inconsistent = [
            UavBindingPayload {
                resource_id: 10,
                bytes: &first,
                element_stride: 4,
            },
            UavBindingPayload {
                resource_id: 10,
                bytes: &second,
                element_stride: 4,
            },
        ];
        assert_eq!(
            validate_uav_alias_policy(&readonly_artifact, &inconsistent),
            Err(DirectX12Error::ArtifactContract(
                "generic DX12 read-only alias payloads must match",
            ))
        );
    }
}
