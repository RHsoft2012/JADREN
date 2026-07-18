#![allow(unsafe_code)]
// JADREN-UNSAFE-AUDIT: Objective-C/Metal messaging is platform-gated to macOS
// and wrapped by null-checked, owned guards before command-buffer status is
// read or native objects are released.

//! Minimal native Metal device/command-buffer smoke runtime for Jadren.
//!
//! The macOS path uses the stable C entry point
//! `MTLCreateSystemDefaultDevice` and Objective-C message dispatch for a
//! command queue/buffer lifecycle. Non-macOS hosts return an explicit skip so
//! CI never reports a Metal pass without the Apple framework.

use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt;

use jadren_codegen_msl::{MslOptions, translate_spirv_artifact_to_msl};
use jadren_codegen_spirv::{ResourceAccess, SpirvArtifact};
use jadren_gpu_runtime::{
    ArtifactResourceLayout, ArtifactSourceBackend, ArtifactSourceTranslationReport,
    SpirvRawModuleContract, SpirvSourceReportWordsError, SpirvSourceTranslationReport,
    select_spirv_raw_output_binding, stable_source_hash, validate_spirv_artifact_contract,
    validate_spirv_raw_native_adapter, validate_spirv_source_report_words,
};

fn map_raw_source_words_error(error: SpirvSourceReportWordsError) -> MetalError {
    match error {
        SpirvSourceReportWordsError::NativeSpirvTransport { .. }
        | SpirvSourceReportWordsError::SourceBackendMismatch { .. } => {
            MetalError::SourceReportMismatch("raw source backend")
        }
        SpirvSourceReportWordsError::WordCountMismatch { .. } => {
            MetalError::SourceReportMismatch("raw source word count")
        }
        SpirvSourceReportWordsError::WordHashMismatch { .. } => {
            MetalError::SourceReportMismatch("raw source word hash")
        }
        SpirvSourceReportWordsError::Source(_) => {
            MetalError::SourceReportMismatch("raw source word contract")
        }
        SpirvSourceReportWordsError::IdentityMismatch => {
            MetalError::SourceReportMismatch("raw source report identity")
        }
        SpirvSourceReportWordsError::Native(_) => {
            MetalError::SourceReportMismatch("raw source native plan")
        }
        SpirvSourceReportWordsError::Specialization(_) => {
            MetalError::SourceReportMismatch("raw source specialization")
        }
    }
}

#[cfg(target_os = "macos")]
use jadren_codegen_msl::{
    F32ArithmeticOp, emit_storage_f32_binary, emit_storage_vector_f32_add,
    emit_storage_vector_f32_binary, emit_storage_vector_f32_binary_lanes,
    validate_storage_f32_binary, validate_storage_vector_f32_add,
    validate_storage_vector_f32_binary, validate_storage_vector_f32_binary_lanes,
};

#[cfg(target_os = "macos")]
use std::{
    ffi::{CStr, CString},
    ptr,
};

/// Native Metal smoke report.
#[derive(Clone, Debug, Serialize)]
pub struct MetalDeviceSmokeReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Whether the host can load the Metal framework path.
    pub metal_framework: &'static str,
    /// Whether `MTLCreateSystemDefaultDevice` returned an object.
    pub device_created: bool,
    /// Whether `newCommandQueue` returned an object.
    pub command_queue_created: bool,
    /// Whether `commandBuffer` returned an object.
    pub command_buffer_created: bool,
    /// Whether the command buffer was committed.
    pub command_buffer_committed: bool,
    /// Whether the command buffer reached completed status.
    pub command_buffer_completed: bool,
    /// Numeric `MTLCommandBufferStatus` when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_buffer_status: Option<i64>,
    /// Overall result.
    pub result: &'static str,
    /// Stable host/API diagnostic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Native result for the narrow runtime-length `f32x4` MSL source contract.
///
/// This report deliberately identifies the source-contract path rather than
/// claiming a general SPIR-V-to-MSL translator. The macOS implementation
/// still performs real library compilation, pipeline creation, dispatch and
/// shared-buffer readback.
#[derive(Clone, Debug, Serialize)]
pub struct MetalF32VectorArtifactExecutionReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// MSL entry point compiled by Metal.
    pub entry_name: &'static str,
    /// Whether the Metal framework path was loaded.
    pub metal_framework: &'static str,
    /// Number of source-contract resource bindings.
    pub resource_binding_count: u32,
    /// Number of logical vector elements processed.
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
    /// Capacity vectors that remained untouched after logical length.
    pub untouched_tail_count: u32,
    /// Whether the generated MSL source passed the portable contract.
    pub source_contract_validated: bool,
    /// Whether a compute pipeline was created.
    pub pipeline_created: bool,
    /// Whether the command queue was created.
    pub command_queue_created: bool,
    /// Whether the command buffer was created.
    pub command_buffer_created: bool,
    /// Whether the command buffer was committed.
    pub command_buffer_committed: bool,
    /// Whether the command buffer reached completed status.
    pub command_buffer_completed: bool,
    /// Numeric `MTLCommandBufferStatus` when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_buffer_status: Option<i64>,
    /// Native route used by this narrow executor.
    pub execution_path: &'static str,
    /// Whether exact lane-wise native differential completed.
    pub execution_completed: bool,
    /// Overall result.
    pub result: &'static str,
}

/// One vector `f32x4` operation result from the generic Metal MSL source path.
#[derive(Clone, Debug, Serialize)]
pub struct MetalF32VectorBinaryExecutionCase {
    /// Stable operation name.
    pub operation: &'static str,
    /// Scalar operand splatted into each vector lane.
    pub operand: f32,
    /// Number of logical vector elements processed.
    pub logical_length: u32,
    /// Physical vector capacity allocated for the dispatch.
    pub capacity: u32,
    /// First logical output vector.
    pub first_output: [f32; 4],
    /// Last logical output vector.
    pub last_output: [f32; 4],
    /// Exact output lane checksum.
    pub output_checksum: f64,
    /// Capacity vectors that remained untouched after logical length.
    pub untouched_tail_count: u32,
    /// Whether the source contract was validated before API calls.
    pub source_contract_validated: bool,
    /// Native execution route.
    pub execution_path: &'static str,
    /// Numeric `MTLCommandBufferStatus` when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_buffer_status: Option<i64>,
    /// Whether exact lane-wise differential completed.
    pub execution_completed: bool,
    /// Overall case result.
    pub result: &'static str,
}

/// Aggregate report for the native Metal vector `f32x4` operation family.
#[derive(Clone, Debug, Serialize)]
pub struct MetalF32VectorBinaryExecutionReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Metal framework state.
    pub metal_framework: &'static str,
    /// Number of operation-specific cases.
    pub case_count: u32,
    /// Operation reports in canonical add/subtract/multiply order.
    pub cases: Vec<MetalF32VectorBinaryExecutionCase>,
    /// Aggregate result.
    pub result: &'static str,
}

/// One x2/x3 vector operation result from the generic Metal MSL source path.
#[derive(Clone, Debug, Serialize)]
pub struct MetalF32VectorLanesBinaryExecutionCase {
    /// Number of lanes in each vector element.
    pub lane_count: u32,
    /// Stable operation name.
    pub operation: &'static str,
    /// Scalar operand splatted into each vector lane.
    pub operand: f32,
    /// Number of logical vector elements processed.
    pub logical_length: u32,
    /// Physical vector capacity allocated for the dispatch.
    pub capacity: u32,
    /// First logical output vector.
    pub first_output: Vec<f32>,
    /// Last logical output vector.
    pub last_output: Vec<f32>,
    /// Exact output lane checksum.
    pub output_checksum: f64,
    /// Capacity vectors that remained untouched after logical length.
    pub untouched_tail_count: u32,
    /// Whether the source contract was validated before API calls.
    pub source_contract_validated: bool,
    /// Native execution route.
    pub execution_path: &'static str,
    /// Numeric `MTLCommandBufferStatus` when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_buffer_status: Option<i64>,
    /// Whether exact lane-wise differential completed.
    pub execution_completed: bool,
    /// Overall case result.
    pub result: &'static str,
}

/// Aggregate report for native Metal x2/x3 vector operation cases.
#[derive(Clone, Debug, Serialize)]
pub struct MetalF32VectorLanesBinaryExecutionReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Metal framework state.
    pub metal_framework: &'static str,
    /// Number of operation-specific cases.
    pub case_count: u32,
    /// Operation reports in lane/operation order.
    pub cases: Vec<MetalF32VectorLanesBinaryExecutionCase>,
    /// Aggregate result.
    pub result: &'static str,
}

/// One scalar `f32` operation result from the generic Metal MSL source path.
#[derive(Clone, Debug, Serialize)]
pub struct MetalF32BinaryExecutionCase {
    /// Stable operation name.
    pub operation: &'static str,
    /// Scalar operand embedded in the MSL source.
    pub operand: f32,
    /// Number of logical scalar elements processed.
    pub logical_length: u32,
    /// Physical scalar capacity allocated for the dispatch.
    pub capacity: u32,
    /// First logical output value.
    pub first_output: f32,
    /// Last logical output value.
    pub last_output: f32,
    /// Exact output checksum.
    pub output_checksum: f64,
    /// Whether the source contract was validated before API calls.
    pub source_contract_validated: bool,
    /// Native execution route.
    pub execution_path: &'static str,
    /// Whether exact scalar bit differential completed.
    pub execution_completed: bool,
    /// Overall case result.
    pub result: &'static str,
}

/// Aggregate report for the native Metal scalar `f32` operation family.
#[derive(Clone, Debug, Serialize)]
pub struct MetalF32BinaryExecutionReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// Metal framework state.
    pub metal_framework: &'static str,
    /// Number of operation-specific cases.
    pub case_count: u32,
    /// Operation reports in canonical add/subtract/multiply order.
    pub cases: Vec<MetalF32BinaryExecutionCase>,
    /// Aggregate result.
    pub result: &'static str,
}

/// Maximum number of dense Metal buffer bindings accepted by the generic
/// source executor. This is the portable Metal argument-table limit; a future
/// adapter may expose a device-specific lower limit after capability probing.
pub const METAL_MAX_BUFFER_BINDINGS: usize = 31;

/// Host payload for one dense Metal buffer binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetalBufferInput {
    /// Dense Metal buffer index.
    pub binding: u32,
    /// Host-side identity used to detect accidental resource aliasing.
    ///
    /// Read-only bindings may share an identity. Any alias group containing a
    /// writable binding is rejected because this executor does not yet expose
    /// a native shared-buffer binding path.
    pub resource_id: u64,
    /// Bytes uploaded before dispatch. The explicitly selected output binding
    /// is zero-initialized by the executor after this payload is copied;
    /// additional writable inputs retain their caller-provided bytes.
    pub bytes: Vec<u8>,
    /// Structured element stride reflected by the source contract.
    pub element_stride: usize,
    /// Whether the shader may write this binding.
    pub writable: bool,
}

/// Explicit geometry supplied to the generic Metal source executor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MetalDispatchGeometry {
    /// Threads in one Metal threadgroup.
    pub threads_per_threadgroup: [u32; 3],
    /// Number of threadgroups submitted in x/y/z order.
    pub threadgroups: [u32; 3],
}

/// JSON-safe lifecycle report for one generic MSL source dispatch.
#[derive(Clone, Debug, Serialize)]
pub struct MetalSourceExecutionReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// MSL entry point used by the dispatch.
    pub entry_name: String,
    /// Whether the Metal framework path was loaded.
    pub metal_framework: &'static str,
    /// Number of dense buffer bindings.
    pub resource_binding_count: u32,
    /// Threadgroup dimensions supplied to Metal.
    pub threads_per_threadgroup: [u32; 3],
    /// Threadgroup grid supplied to Metal.
    pub threadgroups: [u32; 3],
    /// Binding whose readback is returned to the caller.
    pub output_binding: u32,
    /// Number of bytes returned from the output buffer.
    pub output_byte_length: usize,
    /// Whether Metal created a compute pipeline.
    pub pipeline_created: bool,
    /// Whether Metal created a command queue.
    pub command_queue_created: bool,
    /// Whether Metal created a command buffer.
    pub command_buffer_created: bool,
    /// Whether the command buffer was committed.
    pub command_buffer_committed: bool,
    /// Whether the command buffer reached completed status.
    pub command_buffer_completed: bool,
    /// Numeric `MTLCommandBufferStatus` when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_buffer_status: Option<i64>,
    /// Generic native route used by this executor.
    pub execution_path: &'static str,
    /// Whether native execution and readback completed.
    pub execution_completed: bool,
    /// Overall result.
    pub result: &'static str,
}

/// Native Metal result for the mixed read-only/read-write `uint` fixture.
///
/// The source intentionally mirrors the DX12 `t0`/`t2` SRV plus `u1` UAV
/// case. On macOS this report is backed by real MSL compilation, dispatch and
/// readback; non-macOS targets return `MacOsRequired` without a fake pass.
#[derive(Clone, Debug, Serialize)]
pub struct MetalMixedSrvUavExecutionReport {
    /// Stable report schema.
    pub schema: &'static str,
    /// MSL entry point compiled by Metal.
    pub entry_name: &'static str,
    /// Whether the Metal framework path was loaded.
    pub metal_framework: &'static str,
    /// Explicit source/view contract status.
    pub view_contract: &'static str,
    /// Number of logical input elements processed.
    pub logical_length: u32,
    /// Physical output capacity.
    pub capacity: u32,
    /// First logical output value.
    pub first_output: u32,
    /// Last logical output value.
    pub last_output: u32,
    /// Exact output checksum.
    pub output_checksum: u64,
    /// CPU oracle checksum.
    pub expected_checksum: u64,
    /// Capacity elements left untouched after logical length.
    pub untouched_tail_count: u32,
    /// Whether the portable source contract passed before Metal API calls.
    pub source_contract_validated: bool,
    /// Whether Metal created a compute pipeline.
    pub pipeline_created: bool,
    /// Whether Metal created a command queue.
    pub command_queue_created: bool,
    /// Whether Metal created a command buffer.
    pub command_buffer_created: bool,
    /// Whether the command buffer was committed.
    pub command_buffer_committed: bool,
    /// Whether the command buffer reached completed status.
    pub command_buffer_completed: bool,
    /// Numeric `MTLCommandBufferStatus` when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_buffer_status: Option<i64>,
    /// Native route used by this fixture.
    pub execution_path: &'static str,
    /// Whether exact native readback differential completed.
    pub execution_completed: bool,
    /// Overall result.
    pub result: &'static str,
}

/// Errors raised by the native Metal smoke.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MetalError {
    /// The current target is not macOS.
    MacOsRequired,
    /// A required Objective-C runtime operation failed.
    Native(&'static str),
    /// Caller-provided vector data is outside the bounded smoke contract.
    InvalidInput(&'static str),
    /// Portable MSL source generation or validation failed.
    Source(&'static str),
    /// Portable MSL artifact translation failed with a stable diagnostic.
    SourceOwned(String),
    /// The supplied source report is not an MSL report.
    SourceBackendMismatch(ArtifactSourceBackend),
    /// The source report's audit metadata does not match its payload.
    SourceReportMismatch(&'static str),
}

impl fmt::Display for MetalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MacOsRequired => formatter.write_str("Metal smoke requires macOS"),
            Self::Native(operation) => write!(formatter, "Metal operation failed: {operation}"),
            Self::InvalidInput(reason) => write!(formatter, "invalid Metal smoke input: {reason}"),
            Self::Source(reason) => write!(formatter, "Metal source contract failed: {reason}"),
            Self::SourceOwned(reason) => {
                write!(
                    formatter,
                    "Metal artifact source translation failed: {reason}"
                )
            }
            Self::SourceBackendMismatch(backend) => {
                write!(
                    formatter,
                    "Metal executor received {backend:?} source report"
                )
            }
            Self::SourceReportMismatch(reason) => {
                write!(formatter, "Metal source report contract failed: {reason}")
            }
        }
    }
}

impl std::error::Error for MetalError {}

/// Runs the native device/queue/command-buffer lifecycle when available.
#[cfg(not(target_os = "macos"))]
pub fn run_device_smoke() -> Result<MetalDeviceSmokeReport, MetalError> {
    Err(MetalError::MacOsRequired)
}

/// Runs the native device/queue/command-buffer lifecycle on macOS.
#[cfg(target_os = "macos")]
pub fn run_device_smoke() -> Result<MetalDeviceSmokeReport, MetalError> {
    run_macos_device_smoke()
}

/// Executes the bounded runtime-length `f32x4` source contract through the
/// native Metal API when the current target is macOS.
pub fn run_f32_vector_artifact_smoke(
    input_values: &[[f32; 4]],
) -> Result<MetalF32VectorArtifactExecutionReport, MetalError> {
    if input_values.is_empty() || input_values.len() > 128 {
        return Err(MetalError::InvalidInput(
            "f32x4 input length must be in 1..=128",
        ));
    }
    if input_values
        .iter()
        .flat_map(|lanes| lanes.iter())
        .any(|value| !value.is_finite())
    {
        return Err(MetalError::InvalidInput(
            "f32x4 input values must be finite",
        ));
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = input_values;
        Err(MetalError::MacOsRequired)
    }
    #[cfg(target_os = "macos")]
    {
        run_macos_f32_vector_artifact_smoke(input_values)
    }
}

/// Executes the runtime-length `f32x4` add/subtract/multiply family on macOS
/// through the generic MSL source executor.
pub fn run_f32_vector_binary_artifact_smoke(
    input_values: &[[f32; 4]],
) -> Result<MetalF32VectorBinaryExecutionReport, MetalError> {
    if input_values.is_empty() || input_values.len() > 128 {
        return Err(MetalError::InvalidInput(
            "f32x4 input length must be in 1..=128",
        ));
    }
    if input_values
        .iter()
        .flat_map(|lanes| lanes.iter())
        .any(|value| !value.is_finite())
    {
        return Err(MetalError::InvalidInput(
            "f32x4 input values must be finite",
        ));
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = input_values;
        Err(MetalError::MacOsRequired)
    }
    #[cfg(target_os = "macos")]
    {
        run_macos_f32_vector_binary_artifact_smoke(input_values)
    }
}

/// Executes the runtime-length `f32x2`/`f32x3` operation family on macOS
/// through the generic MSL source executor. Non-macOS hosts return an
/// explicit `MacOsRequired` result after validating the input shape.
pub fn run_f32_vector_lanes_binary_artifact_smoke(
    input_values: &[Vec<f32>],
) -> Result<MetalF32VectorLanesBinaryExecutionReport, MetalError> {
    if input_values.is_empty() || input_values.len() > 128 {
        return Err(MetalError::InvalidInput(
            "vector lane input length must be in 1..=128",
        ));
    }
    let lane_count = input_values[0].len();
    if !(2..=3).contains(&lane_count) {
        return Err(MetalError::InvalidInput(
            "vector lane count must be in 2..=3",
        ));
    }
    if input_values
        .iter()
        .any(|lanes| lanes.len() != lane_count || lanes.iter().any(|value| !value.is_finite()))
    {
        return Err(MetalError::InvalidInput(
            "vector lane inputs must have equal finite lane counts",
        ));
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = input_values;
        Err(MetalError::MacOsRequired)
    }
    #[cfg(target_os = "macos")]
    {
        run_macos_f32_vector_lanes_binary_artifact_smoke(input_values, lane_count as u16)
    }
}

/// Executes the scalar runtime-length `f32` add/subtract/multiply family on
/// macOS through the generic MSL source executor.
pub fn run_f32_binary_artifact_smoke(
    input_values: &[f32],
) -> Result<MetalF32BinaryExecutionReport, MetalError> {
    if input_values.is_empty() || input_values.len() > 128 {
        return Err(MetalError::InvalidInput(
            "f32 input length must be in 1..=128",
        ));
    }
    if input_values.iter().any(|value| !value.is_finite()) {
        return Err(MetalError::InvalidInput("f32 input values must be finite"));
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = input_values;
        Err(MetalError::MacOsRequired)
    }
    #[cfg(target_os = "macos")]
    {
        run_macos_f32_binary_artifact_smoke(input_values)
    }
}

/// Validates a generic MSL source dispatch before any Metal API call.
///
/// The contract is intentionally data-only: source entry identity, explicit
/// geometry, dense bindings, structured strides and writable output are
/// checked without loading Metal or creating a device.
pub fn validate_msl_source_execution(
    source: &str,
    entry_name: &str,
    geometry: MetalDispatchGeometry,
    bindings: &[MetalBufferInput],
    output_binding: u32,
) -> Result<(), MetalError> {
    if !valid_metal_identifier(entry_name) {
        return Err(MetalError::InvalidInput(
            "MSL entry name must be an ASCII identifier",
        ));
    }
    if source.trim().is_empty() || !source.contains(&format!("kernel void {entry_name}(")) {
        return Err(MetalError::Source("MSL entry source is missing"));
    }
    if geometry.threads_per_threadgroup.contains(&0) {
        return Err(MetalError::InvalidInput(
            "Metal threadgroup dimensions must be non-zero",
        ));
    }
    let threadgroup_product = geometry
        .threads_per_threadgroup
        .iter()
        .fold(1_u64, |product, &dimension| {
            product.saturating_mul(u64::from(dimension))
        });
    if threadgroup_product > 1024 {
        return Err(MetalError::InvalidInput(
            "Metal threadgroup product exceeds 1024",
        ));
    }
    if geometry.threadgroups.contains(&0) {
        return Err(MetalError::InvalidInput(
            "Metal threadgroup grid dimensions must be non-zero",
        ));
    }
    if bindings.is_empty() || bindings.len() > METAL_MAX_BUFFER_BINDINGS {
        return Err(MetalError::InvalidInput(
            "Metal bindings must contain 1..=31 dense entries",
        ));
    }
    let output_index = usize::try_from(output_binding)
        .map_err(|_| MetalError::InvalidInput("Metal output binding overflows usize"))?;
    if output_index >= bindings.len() {
        return Err(MetalError::InvalidInput(
            "Metal output binding is outside the dense table",
        ));
    }
    for (index, binding) in bindings.iter().enumerate() {
        if usize::try_from(binding.binding).ok() != Some(index) {
            return Err(MetalError::InvalidInput(
                "Metal bindings must use dense zero-based indices",
            ));
        }
        if binding.bytes.is_empty() {
            return Err(MetalError::InvalidInput(
                "Metal buffer payloads must not be empty",
            ));
        }
        if binding.element_stride == 0 || binding.element_stride > 2048 {
            return Err(MetalError::InvalidInput(
                "Metal element stride must be in 1..=2048",
            ));
        }
        if binding.bytes.len() % binding.element_stride != 0 {
            return Err(MetalError::InvalidInput(
                "Metal payload length must be aligned to element stride",
            ));
        }
    }
    for (first_index, first) in bindings.iter().enumerate() {
        for second in bindings.iter().skip(first_index + 1) {
            if first.resource_id == second.resource_id && (first.writable || second.writable) {
                return Err(MetalError::InvalidInput(
                    "Metal writable resource aliases are forbidden",
                ));
            }
        }
    }
    if !bindings[output_index].writable {
        return Err(MetalError::InvalidInput(
            "Metal output binding must be writable",
        ));
    }
    Ok(())
}

/// Executes one validated MSL source request and returns the selected output
/// buffer. Non-macOS targets validate the request but return an explicit
/// `MacOsRequired` result instead of pretending to have Metal execution.
pub fn execute_msl_source(
    source: &str,
    entry_name: &str,
    geometry: MetalDispatchGeometry,
    bindings: &[MetalBufferInput],
    output_binding: u32,
) -> Result<(MetalSourceExecutionReport, Vec<u8>), MetalError> {
    validate_msl_source_execution(source, entry_name, geometry, bindings, output_binding)?;
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (source, entry_name, geometry, bindings, output_binding);
        Err(MetalError::MacOsRequired)
    }
    #[cfg(target_os = "macos")]
    {
        run_macos_msl_source(source, entry_name, geometry, bindings, output_binding)
    }
}

/// Executes the mixed read-only/read-write `uint` source fixture through the
/// generic Metal source executor. Binding `0` and `2` are read-only source
/// inputs (`device const uint*`), while binding `1` is the writable output.
pub fn run_mixed_srv_uav_artifact_smoke() -> Result<MetalMixedSrvUavExecutionReport, MetalError> {
    const ENTRY_NAME: &str = "global_multiply_dynamic_u32";
    const LOGICAL_LENGTH: usize = 70;
    const CAPACITY: usize = 128;
    const WORKGROUP_X: u32 = 64;
    const SOURCE: &str = r#"#include <metal_stdlib>
using namespace metal;

kernel void global_multiply_dynamic_u32(
    device const uint* input [[buffer(0)]],
    device uint* output [[buffer(1)]],
    device const uint* length [[buffer(2)]],
    uint3 gid [[thread_position_in_grid]]) {
    uint index = gid.x;
    if (index < length[0]) {
        output[index] = input[index] * 3u;
    }
}
"#;
    if !SOURCE.contains("device const uint* input [[buffer(0)]]")
        || !SOURCE.contains("device uint* output [[buffer(1)]]")
        || !SOURCE.contains("device const uint* length [[buffer(2)]]")
    {
        return Err(MetalError::Source("mixed MSL view contract"));
    }
    let input_payload = (0..CAPACITY)
        .flat_map(|index| (41_u32 + index as u32).to_le_bytes())
        .collect::<Vec<_>>();
    let bindings = [
        MetalBufferInput {
            binding: 0,
            resource_id: 1,
            bytes: input_payload,
            element_stride: 4,
            writable: false,
        },
        MetalBufferInput {
            binding: 1,
            resource_id: 2,
            bytes: vec![0; CAPACITY * 4],
            element_stride: 4,
            writable: true,
        },
        MetalBufferInput {
            binding: 2,
            resource_id: 3,
            bytes: (LOGICAL_LENGTH as u32).to_le_bytes().to_vec(),
            element_stride: 4,
            writable: false,
        },
    ];
    let geometry = MetalDispatchGeometry {
        threads_per_threadgroup: [WORKGROUP_X, 1, 1],
        threadgroups: [((LOGICAL_LENGTH as u32).div_ceil(WORKGROUP_X)), 1, 1],
    };
    let (execution, output) = execute_msl_source(SOURCE, ENTRY_NAME, geometry, &bindings, 1)?;
    if output.len() != CAPACITY * 4 {
        return Err(MetalError::Native("mixed Metal output byte length"));
    }
    let actual = output
        .chunks_exact(4)
        .map(|bytes| u32::from_le_bytes(bytes.try_into().expect("u32 chunk")))
        .collect::<Vec<_>>();
    let expected = (0..CAPACITY)
        .map(|index| {
            if index < LOGICAL_LENGTH {
                (41_u32 + index as u32) * 3
            } else {
                0
            }
        })
        .collect::<Vec<_>>();
    if actual != expected {
        return Err(MetalError::Native("mixed Metal readback differential"));
    }
    let expected_checksum = expected[..LOGICAL_LENGTH]
        .iter()
        .map(|value| u64::from(*value))
        .sum::<u64>();
    let output_checksum = actual[..LOGICAL_LENGTH]
        .iter()
        .map(|value| u64::from(*value))
        .sum::<u64>();
    Ok(MetalMixedSrvUavExecutionReport {
        schema: "jadren-metal-mixed-srv-uav-source-execution-0.1",
        entry_name: ENTRY_NAME,
        metal_framework: execution.metal_framework,
        view_contract: "passed",
        logical_length: LOGICAL_LENGTH as u32,
        capacity: CAPACITY as u32,
        first_output: actual[0],
        last_output: actual[LOGICAL_LENGTH - 1],
        output_checksum,
        expected_checksum,
        untouched_tail_count: actual[LOGICAL_LENGTH..]
            .iter()
            .filter(|value| **value == 0)
            .count() as u32,
        source_contract_validated: true,
        pipeline_created: execution.pipeline_created,
        command_queue_created: execution.command_queue_created,
        command_buffer_created: execution.command_buffer_created,
        command_buffer_committed: execution.command_buffer_committed,
        command_buffer_completed: execution.command_buffer_completed,
        command_buffer_status: execution.command_buffer_status,
        execution_path: "msl-source-mixed-srv-uav-metal",
        execution_completed: execution.execution_completed && output_checksum == expected_checksum,
        result: "pass-mixed-srv-uav-source-execution-differential",
    })
}

/// Executes an already audited MSL source report. The report backend, source
/// byte count/hash and resource stride/access facts are checked before the
/// generic Metal executor; non-macOS hosts still return `MacOsRequired`.
pub fn execute_msl_source_report(
    report: &ArtifactSourceTranslationReport,
    geometry: MetalDispatchGeometry,
    bindings: &[MetalBufferInput],
    output_binding: u32,
) -> Result<(MetalSourceExecutionReport, Vec<u8>), MetalError> {
    if report.backend != ArtifactSourceBackend::Msl {
        return Err(MetalError::SourceBackendMismatch(report.backend));
    }
    if report.source.trim().is_empty()
        || report.source_byte_count != report.source.len()
        || report.source_hash != stable_source_hash(&report.source)
    {
        return Err(MetalError::SourceReportMismatch(
            "source payload hash or byte count is inconsistent",
        ));
    }
    if report.artifact.resource_binding_count != report.resources.len()
        || report.resources.len() != bindings.len()
    {
        return Err(MetalError::SourceReportMismatch(
            "source artifact/resource count differs from Metal bindings",
        ));
    }
    for (capability, binding) in report.resources.iter().zip(bindings) {
        let expected_stride = match capability.layout {
            ArtifactResourceLayout::ScalarVector { stride, .. }
            | ArtifactResourceLayout::Opaque { stride } => stride,
        };
        let stride_matches = expected_stride
            .map(|stride| usize::try_from(stride).ok() == Some(binding.element_stride))
            .unwrap_or(true);
        let access_matches = match capability.access {
            ResourceAccess::ReadOnly => !binding.writable,
            ResourceAccess::WriteOnly | ResourceAccess::ReadWrite => binding.writable,
        };
        if capability.binding != binding.binding || !stride_matches || !access_matches {
            return Err(MetalError::SourceReportMismatch(
                "source resource capability differs from Metal binding",
            ));
        }
    }
    execute_msl_source(
        &report.source,
        &report.artifact.entry_name,
        geometry,
        bindings,
        output_binding,
    )
}

/// Executes a raw SPIR-V→MSL source report after revalidating the shared raw
/// native-adapter capability boundary. The raw report carries only proven
/// binding/type/stride/access facts; caller-owned Metal buffers still provide
/// resource IDs, payloads and the explicit output binding.
pub fn execute_msl_raw_source_report(
    report: &SpirvSourceTranslationReport,
    geometry: MetalDispatchGeometry,
    bindings: &[MetalBufferInput],
    output_binding: u32,
) -> Result<(MetalSourceExecutionReport, Vec<u8>), MetalError> {
    execute_msl_raw_source_report_with_spec_map(
        report,
        geometry,
        bindings,
        output_binding,
        &BTreeMap::new(),
    )
}

/// Executes a raw SPIR-V→MSL source report with caller-supplied values keyed
/// by reflected `SpecId` decorations. The map is checked before any Metal API
/// lifecycle starts and does not rewrite SPIR-V or compile a specialized MSL
/// variant.
pub fn execute_msl_raw_source_report_with_spec_map(
    report: &SpirvSourceTranslationReport,
    geometry: MetalDispatchGeometry,
    bindings: &[MetalBufferInput],
    output_binding: u32,
    spec_values: &BTreeMap<u32, u32>,
) -> Result<(MetalSourceExecutionReport, Vec<u8>), MetalError> {
    if report.identity.backend != ArtifactSourceBackend::Msl {
        return Err(MetalError::SourceBackendMismatch(report.identity.backend));
    }
    if report.source.trim().is_empty()
        || report.source_byte_count != report.source.len()
        || report.source_hash != stable_source_hash(&report.source)
        || report.identity.word_count == 0
    {
        return Err(MetalError::SourceReportMismatch(
            "raw source payload or word identity is inconsistent",
        ));
    }
    let contract = SpirvRawModuleContract {
        entry_name: report.identity.entry_name.clone(),
        execution_model: report.identity.execution_model,
        workgroup_size: report.identity.workgroup_size,
        workgroup_size_ids: report.identity.workgroup_size_ids,
        workgroup_size_spec_ids: report.identity.workgroup_size_spec_ids,
        resources: report.identity.resources.clone(),
        word_count: report.identity.word_count,
        word_hash: report.identity.word_hash,
    };
    let native_plan = validate_spirv_raw_native_adapter(&contract)
        .map_err(|_| MetalError::SourceReportMismatch("raw native capability is incomplete"))?;
    let output_selection = select_spirv_raw_output_binding(&native_plan, output_binding)
        .map_err(|_| MetalError::SourceReportMismatch("raw source output selection is invalid"))?;
    if native_plan
        .resolve_workgroup_size_from_spec_map(spec_values)
        .is_err()
    {
        return Err(MetalError::SourceReportMismatch(
            "raw source requires resolved LocalSize metadata",
        ));
    }
    if native_plan.resources.len() != bindings.len() {
        return Err(MetalError::SourceReportMismatch(
            "raw source resource count differs from Metal bindings",
        ));
    }
    for (resource, binding) in native_plan.resources.iter().zip(bindings) {
        let expected_stride = resource
            .element_stride
            .ok_or(MetalError::SourceReportMismatch(
                "raw source stride is unknown",
            ))?;
        let access = resource.access.ok_or(MetalError::SourceReportMismatch(
            "raw source access is unknown",
        ))?;
        if resource.binding != binding.binding
            || usize::try_from(expected_stride).ok() != Some(binding.element_stride)
            || access.can_write() != binding.writable
        {
            return Err(MetalError::SourceReportMismatch(
                "raw source capability differs from Metal binding",
            ));
        }
    }
    if output_selection.resource_index
        != usize::try_from(output_binding).map_err(|_| {
            MetalError::SourceReportMismatch("raw source output binding overflows usize")
        })?
    {
        return Err(MetalError::SourceReportMismatch(
            "raw source output binding differs from the dense payload index",
        ));
    }
    execute_msl_source(
        &report.source,
        &report.identity.entry_name,
        geometry,
        bindings,
        output_binding,
    )
}

/// Executes a raw Metal source report after revalidating it against the exact
/// SPIR-V words that produced the report. The word-count/hash/entry bridge and
/// bounded specialization evaluator run before the existing Metal lifecycle;
/// this does not rewrite SPIR-V or compile a specialized MSL variant.
pub fn execute_msl_raw_source_report_with_words(
    report: &SpirvSourceTranslationReport,
    words: &[u32],
    geometry: MetalDispatchGeometry,
    bindings: &[MetalBufferInput],
    output_binding: u32,
    spec_values: &BTreeMap<u32, u32>,
) -> Result<(MetalSourceExecutionReport, Vec<u8>), MetalError> {
    validate_spirv_source_report_words(
        report,
        words,
        jadren_gpu_runtime::GpuBackend::Metal,
        spec_values,
    )
    .map_err(map_raw_source_words_error)?;
    execute_msl_raw_source_report_with_spec_map(
        report,
        geometry,
        bindings,
        output_binding,
        spec_values,
    )
}

/// Translates a validated Jadren artifact through the MSL family dispatcher
/// and sends the resulting source through the generic Metal executor. The
/// portable path performs artifact/source/resource validation on every host;
/// non-macOS hosts then return `MacOsRequired` instead of claiming execution.
pub fn execute_msl_artifact(
    artifact: &SpirvArtifact,
    options: MslOptions,
    geometry: MetalDispatchGeometry,
    bindings: &[MetalBufferInput],
    output_binding: u32,
) -> Result<(MetalSourceExecutionReport, Vec<u8>), MetalError> {
    validate_shared_artifact(artifact)?;
    if geometry.threads_per_threadgroup != options.workgroup_size {
        return Err(MetalError::InvalidInput(
            "Metal geometry threadgroup must match artifact workgroup",
        ));
    }
    let source = translate_spirv_artifact_to_msl(artifact, options)
        .map_err(|error| MetalError::SourceOwned(error.to_string()))?;
    execute_msl_source(
        &source,
        &artifact.entry_name,
        geometry,
        bindings,
        output_binding,
    )
}

/// Translates a validated artifact through the explicit external SPIRV-Cross
/// route and executes the resulting source through the generic Metal API.
///
/// This route is deliberately separate from [`execute_msl_artifact`]. It
/// requires dense resource metadata and matching known strides, but it does
/// not infer output, alias or writable policy from SPIR-V; those remain
/// caller-owned inputs to the generic executor.
pub fn execute_msl_artifact_external(
    artifact: &SpirvArtifact,
    geometry: MetalDispatchGeometry,
    bindings: &[MetalBufferInput],
    output_binding: u32,
) -> Result<(MetalSourceExecutionReport, Vec<u8>), MetalError> {
    validate_external_artifact_bindings(artifact, bindings)?;
    validate_shared_artifact(artifact)?;
    let report = jadren_codegen_msl::translate_spirv_artifact_to_msl_external_report(artifact)
        .map_err(|error| MetalError::SourceOwned(error.to_string()))?;
    execute_msl_source_report(&report, geometry, bindings, output_binding)
}

fn validate_shared_artifact(artifact: &SpirvArtifact) -> Result<(), MetalError> {
    validate_spirv_artifact_contract(artifact)
        .map(|_| ())
        .map_err(|_| MetalError::SourceOwned("shared SPIR-V artifact contract failed".to_owned()))
}

fn validate_external_artifact_bindings(
    artifact: &SpirvArtifact,
    bindings: &[MetalBufferInput],
) -> Result<(), MetalError> {
    if artifact.resources.len() != bindings.len() {
        return Err(MetalError::InvalidInput(
            "external artifact resource count must match Metal bindings",
        ));
    }
    for (index, (resource, binding)) in artifact.resources.iter().zip(bindings).enumerate() {
        if usize::try_from(resource.binding).ok() != Some(index)
            || usize::try_from(binding.binding).ok() != Some(index)
        {
            return Err(MetalError::InvalidInput(
                "external artifact resources and Metal bindings must be dense",
            ));
        }
        if let Some(stride) = resource.element_stride {
            let stride = usize::try_from(stride).map_err(|_| {
                MetalError::InvalidInput("external artifact stride overflows usize")
            })?;
            if binding.element_stride != stride {
                return Err(MetalError::InvalidInput(
                    "external artifact stride differs from Metal binding",
                ));
            }
        }
    }
    Ok(())
}

fn valid_metal_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

#[cfg(target_os = "macos")]
fn run_macos_device_smoke() -> Result<MetalDeviceSmokeReport, MetalError> {
    let device = unsafe { MTLCreateSystemDefaultDevice() };
    if device.is_null() {
        return Err(MetalError::Native("MTLCreateSystemDefaultDevice"));
    }
    let device_guard = ObjcObject::new(device);

    let queue = unsafe { send_object(device_guard.pointer, selector(c"newCommandQueue")) };
    if queue.is_null() {
        return Err(MetalError::Native("-[MTLDevice newCommandQueue]"));
    }
    let queue_guard = ObjcObject::new(queue);

    let command_buffer = unsafe { send_object(queue_guard.pointer, selector(c"commandBuffer")) };
    if command_buffer.is_null() {
        return Err(MetalError::Native("-[MTLCommandQueue commandBuffer]"));
    }
    let command_buffer_guard = ObjcObject::new(command_buffer);

    unsafe { send_void(command_buffer_guard.pointer, selector(c"commit")) };
    unsafe {
        send_void(
            command_buffer_guard.pointer,
            selector(c"waitUntilCompleted"),
        )
    };
    let status = unsafe { send_i64(command_buffer_guard.pointer, selector(c"status")) };
    let completed = status == 4;
    Ok(MetalDeviceSmokeReport {
        schema: "jadren-metal-device-smoke-0.1",
        metal_framework: "loaded",
        device_created: true,
        command_queue_created: true,
        command_buffer_created: true,
        command_buffer_committed: true,
        command_buffer_completed: completed,
        command_buffer_status: Some(status),
        result: if completed {
            "pass-device-command-buffer"
        } else {
            "fail-command-buffer-status"
        },
        error: None,
    })
}

#[cfg(target_os = "macos")]
fn run_macos_msl_source(
    source: &str,
    entry_name: &str,
    geometry: MetalDispatchGeometry,
    bindings: &[MetalBufferInput],
    output_binding: u32,
) -> Result<(MetalSourceExecutionReport, Vec<u8>), MetalError> {
    let source_c = CString::new(source)
        .map_err(|_| MetalError::Source("MSL source contains an embedded NUL"))?;
    let entry_c = CString::new(entry_name)
        .map_err(|_| MetalError::Source("MSL entry contains an embedded NUL"))?;
    let device = unsafe { MTLCreateSystemDefaultDevice() };
    if device.is_null() {
        return Err(MetalError::Native("MTLCreateSystemDefaultDevice"));
    }
    let device_guard = ObjcObject::new(device);
    let source_string = unsafe { ns_string_from_cstr(&source_c) };
    if source_string.is_null() {
        return Err(MetalError::Native("+[NSString stringWithUTF8String:]"));
    }
    let library = unsafe { send_library_with_source(device_guard.pointer, source_string) };
    if library.is_null() {
        return Err(MetalError::Native(
            "-[MTLDevice newLibraryWithSource:options:error:]",
        ));
    }
    let library_guard = ObjcObject::new(library);
    let entry_string = unsafe { ns_string_from_cstr(&entry_c) };
    if entry_string.is_null() {
        return Err(MetalError::Native("+[NSString stringWithUTF8String:]"));
    }
    let function = unsafe {
        send_object_with_object(
            library_guard.pointer,
            selector(c"newFunctionWithName:"),
            entry_string,
        )
    };
    if function.is_null() {
        return Err(MetalError::Native("-[MTLLibrary newFunctionWithName:]"));
    }
    let function_guard = ObjcObject::new(function);
    let pipeline =
        unsafe { send_pipeline_with_function(device_guard.pointer, function_guard.pointer) };
    if pipeline.is_null() {
        return Err(MetalError::Native(
            "-[MTLDevice newComputePipelineStateWithFunction:error:]",
        ));
    }
    let pipeline_guard = ObjcObject::new(pipeline);

    let queue = unsafe { send_object(device_guard.pointer, selector(c"newCommandQueue")) };
    if queue.is_null() {
        return Err(MetalError::Native("-[MTLDevice newCommandQueue]"));
    }
    let queue_guard = ObjcObject::new(queue);
    let command_buffer = unsafe { send_object(queue_guard.pointer, selector(c"commandBuffer")) };
    if command_buffer.is_null() {
        return Err(MetalError::Native("-[MTLCommandQueue commandBuffer]"));
    }
    let command_buffer_guard = ObjcObject::new(command_buffer);
    let encoder = unsafe {
        send_object(
            command_buffer_guard.pointer,
            selector(c"computeCommandEncoder"),
        )
    };
    if encoder.is_null() {
        return Err(MetalError::Native(
            "-[MTLCommandBuffer computeCommandEncoder]",
        ));
    }
    let encoder_guard = ObjcObject::new(encoder);

    let mut buffer_guards = Vec::with_capacity(bindings.len());
    for binding in bindings {
        let buffer = unsafe { send_new_buffer(device_guard.pointer, binding.bytes.len()) };
        if buffer.is_null() {
            return Err(MetalError::Native(
                "-[MTLDevice newBufferWithLength:options:]",
            ));
        }
        let buffer_guard = ObjcObject::new(buffer);
        unsafe {
            write_buffer_bytes(buffer_guard.pointer, &binding.bytes)?;
            if binding.binding == output_binding {
                clear_buffer(buffer_guard.pointer, binding.bytes.len())?;
            }
            buffer_guards.push(buffer_guard);
        }
    }
    unsafe {
        send_void_with_object(
            encoder_guard.pointer,
            selector(c"setComputePipelineState:"),
            pipeline_guard.pointer,
        );
        for (index, binding) in bindings.iter().enumerate() {
            send_set_buffer(
                encoder_guard.pointer,
                buffer_guards[index].pointer,
                usize::try_from(binding.binding).expect("validated Metal binding fits usize"),
            );
        }
        let groups = MtlSize {
            width: geometry.threadgroups[0] as usize,
            height: geometry.threadgroups[1] as usize,
            depth: geometry.threadgroups[2] as usize,
        };
        let threads = MtlSize {
            width: geometry.threads_per_threadgroup[0] as usize,
            height: geometry.threads_per_threadgroup[1] as usize,
            depth: geometry.threads_per_threadgroup[2] as usize,
        };
        send_dispatch_threadgroups(encoder_guard.pointer, groups, threads);
        send_void(encoder_guard.pointer, selector(c"endEncoding"));
        send_void(command_buffer_guard.pointer, selector(c"commit"));
        send_void(
            command_buffer_guard.pointer,
            selector(c"waitUntilCompleted"),
        );
    }
    let status = unsafe { send_i64(command_buffer_guard.pointer, selector(c"status")) };
    if status != 4 {
        return Err(MetalError::Native(
            "-[MTLCommandBuffer status] != completed",
        ));
    }
    let output_index =
        usize::try_from(output_binding).expect("validated Metal output binding fits usize");
    let output = unsafe {
        read_buffer_bytes(
            buffer_guards[output_index].pointer,
            bindings[output_index].bytes.len(),
        )?
    };
    Ok((
        MetalSourceExecutionReport {
            schema: "jadren-metal-msl-source-execution-0.1",
            entry_name: entry_name.to_owned(),
            metal_framework: "loaded",
            resource_binding_count: bindings.len() as u32,
            threads_per_threadgroup: geometry.threads_per_threadgroup,
            threadgroups: geometry.threadgroups,
            output_binding,
            output_byte_length: output.len(),
            pipeline_created: true,
            command_queue_created: true,
            command_buffer_created: true,
            command_buffer_committed: true,
            command_buffer_completed: true,
            command_buffer_status: Some(status),
            execution_path: "msl-source-metal",
            execution_completed: true,
            result: "pass-msl-source-execution",
        },
        output,
    ))
}

#[cfg(target_os = "macos")]
fn run_macos_f32_vector_artifact_smoke(
    input_values: &[[f32; 4]],
) -> Result<MetalF32VectorArtifactExecutionReport, MetalError> {
    const ENTRY_NAME: &str = "global_add_dynamic_f32x4";
    const CAPACITY: usize = 128;
    const WORKGROUP_X: usize = 64;
    const OPERAND: f32 = 1.0;

    let options = MslOptions::new([WORKGROUP_X as u32, 1, 1])
        .map_err(|_| MetalError::Source("invalid vector workgroup"))?;
    let source = emit_storage_vector_f32_add(ENTRY_NAME, options, OPERAND.to_bits())
        .map_err(|_| MetalError::Source("vector MSL emission"))?;
    validate_storage_vector_f32_add(&source, ENTRY_NAME)
        .map_err(|_| MetalError::Source("vector MSL validation"))?;

    let input_bytes = input_values.len() * 16;
    let output_bytes = CAPACITY * 16;
    let mut input_payload = Vec::with_capacity(input_bytes);
    for lanes in input_values {
        for lane in lanes {
            input_payload.extend_from_slice(&lane.to_le_bytes());
        }
    }
    let bindings = [
        MetalBufferInput {
            binding: 0,
            resource_id: 1,
            bytes: input_payload,
            element_stride: 16,
            writable: false,
        },
        MetalBufferInput {
            binding: 1,
            resource_id: 2,
            bytes: vec![0; output_bytes],
            element_stride: 16,
            writable: true,
        },
        MetalBufferInput {
            binding: 2,
            resource_id: 3,
            bytes: (input_values.len() as u32).to_le_bytes().to_vec(),
            element_stride: 4,
            writable: false,
        },
    ];
    let geometry = MetalDispatchGeometry {
        threads_per_threadgroup: [WORKGROUP_X as u32, 1, 1],
        threadgroups: [input_values.len().div_ceil(WORKGROUP_X) as u32, 1, 1],
    };
    let (execution, output) = execute_msl_source(&source, ENTRY_NAME, geometry, &bindings, 1)?;
    let actual_values = bytes_to_f32x4(&output, CAPACITY)?;
    let mut expected_values = vec![[0.0; 4]; CAPACITY];
    for (index, value) in input_values.iter().enumerate() {
        expected_values[index] = value.map(|lane| lane + OPERAND);
    }
    if actual_values
        .iter()
        .zip(&expected_values)
        .any(|(actual, expected)| {
            actual
                .iter()
                .zip(expected)
                .any(|(actual, expected)| actual.to_bits() != expected.to_bits())
        })
    {
        return Err(MetalError::Native("Metal f32x4 readback differential"));
    }
    let input_checksum = input_values
        .iter()
        .flat_map(|lanes| lanes.iter())
        .map(|value| f64::from(*value))
        .sum();
    let output_checksum = actual_values
        .iter()
        .flat_map(|lanes| lanes.iter())
        .map(|value| f64::from(*value))
        .sum();
    let untouched_tail_count = actual_values[input_values.len()..]
        .iter()
        .filter(|lanes| lanes.iter().all(|value| value.to_bits() == 0))
        .count();
    Ok(MetalF32VectorArtifactExecutionReport {
        schema: "jadren-metal-f32x4-source-execution-0.1",
        entry_name: ENTRY_NAME,
        metal_framework: "loaded",
        resource_binding_count: 3,
        logical_length: input_values.len() as u32,
        capacity: CAPACITY as u32,
        first_output: actual_values[0],
        last_output: actual_values[input_values.len() - 1],
        input_checksum,
        output_checksum,
        untouched_tail_count: untouched_tail_count as u32,
        source_contract_validated: true,
        pipeline_created: execution.pipeline_created,
        command_queue_created: execution.command_queue_created,
        command_buffer_created: execution.command_buffer_created,
        command_buffer_committed: execution.command_buffer_committed,
        command_buffer_completed: execution.command_buffer_completed,
        command_buffer_status: execution.command_buffer_status,
        execution_path: "msl-source-contract-metal",
        execution_completed: execution.execution_completed,
        result: if execution.execution_completed {
            "pass-f32x4-source-execution-differential"
        } else {
            "fail-f32x4-source-execution"
        },
    })
}

#[cfg(target_os = "macos")]
fn run_macos_f32_vector_binary_artifact_smoke(
    input_values: &[[f32; 4]],
) -> Result<MetalF32VectorBinaryExecutionReport, MetalError> {
    let operations = [
        F32ArithmeticOp::Add,
        F32ArithmeticOp::Subtract,
        F32ArithmeticOp::Multiply,
    ];
    let mut cases = Vec::with_capacity(operations.len());
    for operation in operations {
        cases.push(run_macos_f32_vector_binary_case(input_values, operation)?);
    }
    Ok(MetalF32VectorBinaryExecutionReport {
        schema: "jadren-metal-f32x4-binary-source-execution-0.1",
        metal_framework: "loaded",
        case_count: cases.len() as u32,
        cases,
        result: "pass-f32x4-binary-source-execution-differential",
    })
}

#[cfg(target_os = "macos")]
fn run_macos_f32_vector_binary_case(
    input_values: &[[f32; 4]],
    operation: F32ArithmeticOp,
) -> Result<MetalF32VectorBinaryExecutionCase, MetalError> {
    const CAPACITY: usize = 128;
    const WORKGROUP_X: usize = 64;
    let operation_name = f32_operation_name(operation);
    let entry_name = format!("global_{operation_name}_dynamic_f32x4");
    let operand = f32_operation_operand(operation);
    let options = MslOptions::new([WORKGROUP_X as u32, 1, 1])
        .map_err(|_| MetalError::Source("invalid vector f32 workgroup"))?;
    let source = emit_storage_vector_f32_binary(&entry_name, options, operand.to_bits(), operation)
        .map_err(|_| MetalError::Source("vector f32 MSL emission"))?;
    validate_storage_vector_f32_binary(&source, &entry_name, operation)
        .map_err(|_| MetalError::Source("vector f32 MSL validation"))?;

    let input_payload = input_values
        .iter()
        .flat_map(|lanes| lanes.iter().flat_map(|value| value.to_le_bytes()))
        .collect::<Vec<_>>();
    let bindings = [
        MetalBufferInput {
            binding: 0,
            resource_id: 1,
            bytes: input_payload,
            element_stride: 16,
            writable: false,
        },
        MetalBufferInput {
            binding: 1,
            resource_id: 2,
            bytes: vec![0; CAPACITY * 16],
            element_stride: 16,
            writable: true,
        },
        MetalBufferInput {
            binding: 2,
            resource_id: 3,
            bytes: (input_values.len() as u32).to_le_bytes().to_vec(),
            element_stride: 4,
            writable: false,
        },
    ];
    let geometry = MetalDispatchGeometry {
        threads_per_threadgroup: [WORKGROUP_X as u32, 1, 1],
        threadgroups: [input_values.len().div_ceil(WORKGROUP_X) as u32, 1, 1],
    };
    let (execution, output) = execute_msl_source(&source, &entry_name, geometry, &bindings, 1)?;
    let actual_values = bytes_to_f32x4(&output, CAPACITY)?;
    let mut expected_values = vec![[0.0; 4]; CAPACITY];
    for (index, value) in input_values.iter().enumerate() {
        expected_values[index] = value.map(|lane| apply_f32_operation(lane, operand, operation));
    }
    if actual_values
        .iter()
        .zip(&expected_values)
        .any(|(actual, expected)| {
            actual
                .iter()
                .zip(expected)
                .any(|(actual, expected)| actual.to_bits() != expected.to_bits())
        })
    {
        return Err(MetalError::Native(
            "Metal f32x4 binary readback differential",
        ));
    }
    let output_checksum = actual_values
        .iter()
        .flat_map(|lanes| lanes.iter())
        .map(|value| f64::from(*value))
        .sum();
    let untouched_tail_count = actual_values[input_values.len()..]
        .iter()
        .filter(|lanes| lanes.iter().all(|value| value.to_bits() == 0))
        .count();
    Ok(MetalF32VectorBinaryExecutionCase {
        operation: operation_name,
        operand,
        logical_length: input_values.len() as u32,
        capacity: CAPACITY as u32,
        first_output: actual_values[0],
        last_output: actual_values[input_values.len() - 1],
        output_checksum,
        untouched_tail_count: untouched_tail_count as u32,
        source_contract_validated: true,
        execution_path: "msl-source-metal",
        command_buffer_status: execution.command_buffer_status,
        execution_completed: execution.execution_completed,
        result: if execution.execution_completed {
            "pass-f32x4-binary-case"
        } else {
            "fail-f32x4-binary-case"
        },
    })
}

#[cfg(target_os = "macos")]
fn run_macos_f32_vector_lanes_binary_artifact_smoke(
    input_values: &[Vec<f32>],
    lane_count: u16,
) -> Result<MetalF32VectorLanesBinaryExecutionReport, MetalError> {
    let operations = [
        F32ArithmeticOp::Add,
        F32ArithmeticOp::Subtract,
        F32ArithmeticOp::Multiply,
    ];
    let mut cases = Vec::with_capacity(operations.len());
    for operation in operations {
        cases.push(run_macos_f32_vector_lanes_binary_case(
            input_values,
            lane_count,
            operation,
        )?);
    }
    Ok(MetalF32VectorLanesBinaryExecutionReport {
        schema: "jadren-metal-f32-vector-lanes-binary-source-execution-0.1",
        metal_framework: "loaded",
        case_count: cases.len() as u32,
        cases,
        result: "pass-f32-vector-lanes-binary-source-execution-differential",
    })
}

#[cfg(target_os = "macos")]
fn run_macos_f32_vector_lanes_binary_case(
    input_values: &[Vec<f32>],
    lane_count: u16,
    operation: F32ArithmeticOp,
) -> Result<MetalF32VectorLanesBinaryExecutionCase, MetalError> {
    const CAPACITY: usize = 128;
    const WORKGROUP_X: usize = 64;
    let operation_name = f32_operation_name(operation);
    let entry_name = format!("global_{operation_name}_dynamic_f32x{lane_count}");
    let operand = f32_operation_operand(operation);
    let options = MslOptions::new([WORKGROUP_X as u32, 1, 1])
        .map_err(|_| MetalError::Source("invalid vector lane workgroup"))?;
    let source = emit_storage_vector_f32_binary_lanes(
        &entry_name,
        options,
        operand.to_bits(),
        operation,
        lane_count,
    )
    .map_err(|_| MetalError::Source("vector lane MSL emission"))?;
    validate_storage_vector_f32_binary_lanes(&source, &entry_name, operation, lane_count)
        .map_err(|_| MetalError::Source("vector lane MSL validation"))?;
    let stride = usize::from(lane_count) * 4;
    let input_payload = input_values
        .iter()
        .flatten()
        .flat_map(|value| value.to_le_bytes())
        .collect::<Vec<_>>();
    let bindings = [
        MetalBufferInput {
            binding: 0,
            resource_id: 1,
            bytes: input_payload,
            element_stride: stride,
            writable: false,
        },
        MetalBufferInput {
            binding: 1,
            resource_id: 2,
            bytes: vec![0; CAPACITY * stride],
            element_stride: stride,
            writable: true,
        },
        MetalBufferInput {
            binding: 2,
            resource_id: 3,
            bytes: (input_values.len() as u32).to_le_bytes().to_vec(),
            element_stride: 4,
            writable: false,
        },
    ];
    let geometry = MetalDispatchGeometry {
        threads_per_threadgroup: [WORKGROUP_X as u32, 1, 1],
        threadgroups: [input_values.len().div_ceil(WORKGROUP_X) as u32, 1, 1],
    };
    let (execution, output) = execute_msl_source(&source, &entry_name, geometry, &bindings, 1)?;
    let actual_values = bytes_to_f32_lanes(&output, CAPACITY, lane_count as usize)?;
    let mut expected_values = vec![vec![0.0; usize::from(lane_count)]; CAPACITY];
    for (index, value) in input_values.iter().enumerate() {
        expected_values[index] = value
            .iter()
            .map(|lane| apply_f32_operation(*lane, operand, operation))
            .collect();
    }
    if actual_values
        .iter()
        .zip(&expected_values)
        .any(|(actual, expected)| {
            actual
                .iter()
                .zip(expected)
                .any(|(actual, expected)| actual.to_bits() != expected.to_bits())
        })
    {
        return Err(MetalError::Native(
            "Metal vector lane binary readback differential",
        ));
    }
    let output_checksum = actual_values[..input_values.len()]
        .iter()
        .flatten()
        .map(|value| f64::from(*value))
        .sum();
    let untouched_tail_count = actual_values[input_values.len()..]
        .iter()
        .filter(|lanes| lanes.iter().all(|value| value.to_bits() == 0))
        .count();
    Ok(MetalF32VectorLanesBinaryExecutionCase {
        lane_count: u32::from(lane_count),
        operation: operation_name,
        operand,
        logical_length: input_values.len() as u32,
        capacity: CAPACITY as u32,
        first_output: actual_values[0].clone(),
        last_output: actual_values[input_values.len() - 1].clone(),
        output_checksum,
        untouched_tail_count: untouched_tail_count as u32,
        source_contract_validated: true,
        execution_path: "msl-source-metal",
        command_buffer_status: execution.command_buffer_status,
        execution_completed: execution.execution_completed,
        result: if execution.execution_completed {
            "pass-f32-vector-lanes-binary-case"
        } else {
            "fail-f32-vector-lanes-binary-case"
        },
    })
}

#[cfg(target_os = "macos")]
fn run_macos_f32_binary_artifact_smoke(
    input_values: &[f32],
) -> Result<MetalF32BinaryExecutionReport, MetalError> {
    let operations = [
        F32ArithmeticOp::Add,
        F32ArithmeticOp::Subtract,
        F32ArithmeticOp::Multiply,
    ];
    let mut cases = Vec::with_capacity(operations.len());
    for operation in operations {
        cases.push(run_macos_f32_binary_case(input_values, operation)?);
    }
    Ok(MetalF32BinaryExecutionReport {
        schema: "jadren-metal-f32-binary-source-execution-0.1",
        metal_framework: "loaded",
        case_count: cases.len() as u32,
        cases,
        result: "pass-f32-binary-source-execution-differential",
    })
}

#[cfg(target_os = "macos")]
fn run_macos_f32_binary_case(
    input_values: &[f32],
    operation: F32ArithmeticOp,
) -> Result<MetalF32BinaryExecutionCase, MetalError> {
    const CAPACITY: usize = 128;
    const WORKGROUP_X: usize = 64;
    let operation_name = f32_operation_name(operation);
    let entry_name = format!("global_{operation_name}_dynamic_f32");
    let operand = f32_operation_operand(operation);
    let options = MslOptions::new([WORKGROUP_X as u32, 1, 1])
        .map_err(|_| MetalError::Source("invalid scalar f32 workgroup"))?;
    let source = emit_storage_f32_binary(&entry_name, options, operand.to_bits(), operation)
        .map_err(|_| MetalError::Source("scalar f32 MSL emission"))?;
    validate_storage_f32_binary(&source, &entry_name, operation)
        .map_err(|_| MetalError::Source("scalar f32 MSL validation"))?;

    let input_payload = input_values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect::<Vec<_>>();
    let bindings = [
        MetalBufferInput {
            binding: 0,
            resource_id: 1,
            bytes: input_payload,
            element_stride: 4,
            writable: false,
        },
        MetalBufferInput {
            binding: 1,
            resource_id: 2,
            bytes: vec![0; CAPACITY * 4],
            element_stride: 4,
            writable: true,
        },
        MetalBufferInput {
            binding: 2,
            resource_id: 3,
            bytes: (input_values.len() as u32).to_le_bytes().to_vec(),
            element_stride: 4,
            writable: false,
        },
    ];
    let geometry = MetalDispatchGeometry {
        threads_per_threadgroup: [WORKGROUP_X as u32, 1, 1],
        threadgroups: [input_values.len().div_ceil(WORKGROUP_X) as u32, 1, 1],
    };
    let (execution, output) = execute_msl_source(&source, &entry_name, geometry, &bindings, 1)?;
    let actual_values = bytes_to_f32(&output, CAPACITY)?;
    let mut expected_values = vec![0.0; CAPACITY];
    for (index, value) in input_values.iter().enumerate() {
        expected_values[index] = apply_f32_operation(*value, operand, operation);
    }
    if actual_values
        .iter()
        .zip(&expected_values)
        .any(|(actual, expected)| actual.to_bits() != expected.to_bits())
    {
        return Err(MetalError::Native("Metal scalar f32 readback differential"));
    }
    let output_checksum = actual_values.iter().map(|value| f64::from(*value)).sum();
    Ok(MetalF32BinaryExecutionCase {
        operation: operation_name,
        operand,
        logical_length: input_values.len() as u32,
        capacity: CAPACITY as u32,
        first_output: actual_values[0],
        last_output: actual_values[input_values.len() - 1],
        output_checksum,
        source_contract_validated: true,
        execution_path: "msl-source-metal",
        execution_completed: execution.execution_completed,
        result: if execution.execution_completed {
            "pass-f32-binary-case"
        } else {
            "fail-f32-binary-case"
        },
    })
}

#[cfg(target_os = "macos")]
const fn f32_operation_name(operation: F32ArithmeticOp) -> &'static str {
    match operation {
        F32ArithmeticOp::Add => "add",
        F32ArithmeticOp::Subtract => "subtract",
        F32ArithmeticOp::Multiply => "multiply",
    }
}

#[cfg(target_os = "macos")]
const fn f32_operation_operand(operation: F32ArithmeticOp) -> f32 {
    match operation {
        F32ArithmeticOp::Add | F32ArithmeticOp::Subtract => 1.0,
        F32ArithmeticOp::Multiply => 2.0,
    }
}

#[cfg(target_os = "macos")]
const fn apply_f32_operation(value: f32, operand: f32, operation: F32ArithmeticOp) -> f32 {
    match operation {
        F32ArithmeticOp::Add => value + operand,
        F32ArithmeticOp::Subtract => value - operand,
        F32ArithmeticOp::Multiply => value * operand,
    }
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct MtlSize {
    width: usize,
    height: usize,
    depth: usize,
}

#[cfg(target_os = "macos")]
unsafe fn ns_string_from_cstr(value: &CStr) -> *mut std::ffi::c_void {
    let class = unsafe { objc_get_class(c"NSString".as_ptr()) };
    let message: unsafe extern "C" fn(
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
        *const std::ffi::c_char,
    ) -> *mut std::ffi::c_void = unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { message(class, selector(c"stringWithUTF8String:"), value.as_ptr()) }
}

#[cfg(target_os = "macos")]
unsafe fn send_library_with_source(
    receiver: *mut std::ffi::c_void,
    source: *mut std::ffi::c_void,
) -> *mut std::ffi::c_void {
    let message: unsafe extern "C" fn(
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
        *mut *mut std::ffi::c_void,
    ) -> *mut std::ffi::c_void = unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    let mut error = ptr::null_mut();
    unsafe {
        message(
            receiver,
            selector(c"newLibraryWithSource:options:error:"),
            source,
            ptr::null_mut(),
            &mut error,
        )
    }
}

#[cfg(target_os = "macos")]
unsafe fn send_object_with_object(
    receiver: *mut std::ffi::c_void,
    selector_value: *mut std::ffi::c_void,
    object: *mut std::ffi::c_void,
) -> *mut std::ffi::c_void {
    let message: unsafe extern "C" fn(
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
    ) -> *mut std::ffi::c_void = unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { message(receiver, selector_value, object) }
}

#[cfg(target_os = "macos")]
unsafe fn send_pipeline_with_function(
    receiver: *mut std::ffi::c_void,
    function: *mut std::ffi::c_void,
) -> *mut std::ffi::c_void {
    let message: unsafe extern "C" fn(
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
        *mut *mut std::ffi::c_void,
    ) -> *mut std::ffi::c_void = unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    let mut error = ptr::null_mut();
    unsafe {
        message(
            receiver,
            selector(c"newComputePipelineStateWithFunction:error:"),
            function,
            &mut error,
        )
    }
}

#[cfg(target_os = "macos")]
unsafe fn send_new_buffer(receiver: *mut std::ffi::c_void, length: usize) -> *mut std::ffi::c_void {
    let message: unsafe extern "C" fn(
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
        usize,
        usize,
    ) -> *mut std::ffi::c_void = unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe {
        message(
            receiver,
            selector(c"newBufferWithLength:options:"),
            length,
            0,
        )
    }
}

#[cfg(target_os = "macos")]
unsafe fn send_void_with_object(
    receiver: *mut std::ffi::c_void,
    selector_value: *mut std::ffi::c_void,
    object: *mut std::ffi::c_void,
) {
    let message: unsafe extern "C" fn(
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
    ) = unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { message(receiver, selector_value, object) };
}

#[cfg(target_os = "macos")]
unsafe fn send_set_buffer(
    receiver: *mut std::ffi::c_void,
    buffer: *mut std::ffi::c_void,
    index: usize,
) {
    let message: unsafe extern "C" fn(
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
        usize,
        usize,
    ) = unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe {
        message(
            receiver,
            selector(c"setBuffer:offset:atIndex:"),
            buffer,
            0,
            index,
        )
    };
}

#[cfg(target_os = "macos")]
unsafe fn send_dispatch_threadgroups(
    receiver: *mut std::ffi::c_void,
    groups: MtlSize,
    threads: MtlSize,
) {
    let message: unsafe extern "C" fn(
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
        MtlSize,
        MtlSize,
    ) = unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe {
        message(
            receiver,
            selector(c"dispatchThreadgroups:threadsPerThreadgroup:"),
            groups,
            threads,
        )
    };
}

#[cfg(target_os = "macos")]
unsafe fn buffer_contents(receiver: *mut std::ffi::c_void) -> *mut std::ffi::c_void {
    let message: unsafe extern "C" fn(
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
    ) -> *mut std::ffi::c_void = unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { message(receiver, selector(c"contents")) }
}

#[cfg(target_os = "macos")]
unsafe fn write_buffer_bytes(
    buffer: *mut std::ffi::c_void,
    bytes: &[u8],
) -> Result<(), MetalError> {
    let destination = unsafe { buffer_contents(buffer) } as *mut u8;
    if destination.is_null() {
        return Err(MetalError::Native("-[MTLBuffer contents]"));
    }
    unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), destination, bytes.len()) };
    Ok(())
}

#[cfg(target_os = "macos")]
unsafe fn read_buffer_bytes(
    buffer: *mut std::ffi::c_void,
    bytes: usize,
) -> Result<Vec<u8>, MetalError> {
    let source = unsafe { buffer_contents(buffer) } as *const u8;
    if source.is_null() {
        return Err(MetalError::Native("-[MTLBuffer contents]"));
    }
    Ok(unsafe { std::slice::from_raw_parts(source, bytes).to_vec() })
}

#[cfg(target_os = "macos")]
unsafe fn clear_buffer(buffer: *mut std::ffi::c_void, bytes: usize) -> Result<(), MetalError> {
    let destination = unsafe { buffer_contents(buffer) } as *mut u8;
    if destination.is_null() {
        return Err(MetalError::Native("-[MTLBuffer contents]"));
    }
    unsafe { ptr::write_bytes(destination, 0, bytes) };
    Ok(())
}

#[cfg(target_os = "macos")]
fn bytes_to_f32x4(bytes: &[u8], count: usize) -> Result<Vec<[f32; 4]>, MetalError> {
    let expected_bytes = count.checked_mul(16).ok_or(MetalError::InvalidInput(
        "f32x4 byte length overflows usize",
    ))?;
    if bytes.len() != expected_bytes {
        return Err(MetalError::Native("Metal f32x4 output byte length"));
    }
    let mut values = Vec::with_capacity(count);
    for chunk in bytes.chunks_exact(16) {
        values.push([
            f32::from_le_bytes(chunk[0..4].try_into().expect("lane width is fixed")),
            f32::from_le_bytes(chunk[4..8].try_into().expect("lane width is fixed")),
            f32::from_le_bytes(chunk[8..12].try_into().expect("lane width is fixed")),
            f32::from_le_bytes(chunk[12..16].try_into().expect("lane width is fixed")),
        ]);
    }
    Ok(values)
}

#[cfg(target_os = "macos")]
fn bytes_to_f32_lanes(
    bytes: &[u8],
    count: usize,
    lane_count: usize,
) -> Result<Vec<Vec<f32>>, MetalError> {
    let stride = lane_count
        .checked_mul(4)
        .ok_or(MetalError::InvalidInput("vector lane byte stride overflow"))?;
    let expected_bytes = count.checked_mul(stride).ok_or(MetalError::InvalidInput(
        "vector lane byte length overflows usize",
    ))?;
    if !(2..=3).contains(&lane_count) || bytes.len() != expected_bytes {
        return Err(MetalError::Native("Metal vector lane output byte length"));
    }
    Ok(bytes
        .chunks_exact(stride)
        .map(|chunk| {
            (0..lane_count)
                .map(|lane| {
                    let start = lane * 4;
                    f32::from_le_bytes(chunk[start..start + 4].try_into().expect("lane is fixed"))
                })
                .collect()
        })
        .collect())
}

#[cfg(target_os = "macos")]
fn bytes_to_f32(bytes: &[u8], count: usize) -> Result<Vec<f32>, MetalError> {
    let expected_bytes = count
        .checked_mul(4)
        .ok_or(MetalError::InvalidInput("f32 byte length overflows usize"))?;
    if bytes.len() != expected_bytes {
        return Err(MetalError::Native("Metal scalar f32 output byte length"));
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("scalar lane width is fixed")))
        .collect())
}

#[cfg(target_os = "macos")]
struct ObjcObject {
    pointer: *mut std::ffi::c_void,
}

#[cfg(target_os = "macos")]
impl ObjcObject {
    const fn new(pointer: *mut std::ffi::c_void) -> Self {
        Self { pointer }
    }
}

#[cfg(target_os = "macos")]
impl Drop for ObjcObject {
    fn drop(&mut self) {
        if !self.pointer.is_null() {
            unsafe { send_void(self.pointer, selector(c"release")) };
        }
    }
}

#[cfg(target_os = "macos")]
unsafe fn selector(name: &std::ffi::CStr) -> *mut std::ffi::c_void {
    unsafe { sel_registerName(name.as_ptr()) }
}

#[cfg(target_os = "macos")]
unsafe fn send_object(
    receiver: *mut std::ffi::c_void,
    selector: *mut std::ffi::c_void,
) -> *mut std::ffi::c_void {
    let message: unsafe extern "C" fn(
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
    ) -> *mut std::ffi::c_void = unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { message(receiver, selector) }
}

#[cfg(target_os = "macos")]
unsafe fn send_void(receiver: *mut std::ffi::c_void, selector: *mut std::ffi::c_void) {
    let message: unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void) =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { message(receiver, selector) };
}

#[cfg(target_os = "macos")]
unsafe fn send_i64(receiver: *mut std::ffi::c_void, selector: *mut std::ffi::c_void) -> i64 {
    let message: unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void) -> i64 =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { message(receiver, selector) }
}

#[cfg(target_os = "macos")]
#[link(name = "Metal", kind = "framework")]
unsafe extern "C" {
    fn MTLCreateSystemDefaultDevice() -> *mut std::ffi::c_void;
}

#[cfg(target_os = "macos")]
#[link(name = "objc")]
unsafe extern "C" {
    fn objc_msgSend();
    #[link_name = "objc_getClass"]
    fn objc_get_class(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
    fn sel_registerName(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
}

#[cfg(test)]
mod tests {
    use super::{
        MetalBufferInput, MetalDispatchGeometry, MetalError, execute_msl_artifact,
        execute_msl_artifact_external, execute_msl_source_report, run_f32_binary_artifact_smoke,
        run_f32_vector_artifact_smoke, run_f32_vector_binary_artifact_smoke,
        run_f32_vector_lanes_binary_artifact_smoke, validate_msl_source_execution,
    };
    #[cfg(not(target_os = "macos"))]
    use super::{
        execute_msl_raw_source_report, execute_msl_raw_source_report_with_words, run_device_smoke,
        run_mixed_srv_uav_artifact_smoke,
    };

    #[test]
    fn artifact_executor_rejects_unknown_shape_before_metal_api() {
        let artifact = super::SpirvArtifact {
            entry_name: "unknown".to_owned(),
            workgroup_size: [1, 1, 1],
            resources: Vec::new(),
            words: Vec::new(),
        };
        assert!(matches!(
            execute_msl_artifact(
                &artifact,
                super::MslOptions::new([1, 1, 1]).expect("valid workgroup"),
                MetalDispatchGeometry {
                    threads_per_threadgroup: [1, 1, 1],
                    threadgroups: [1, 1, 1],
                },
                &[],
                0,
            ),
            Err(MetalError::SourceOwned(_))
        ));
    }

    #[test]
    fn external_artifact_executor_rejects_binding_mismatch_before_toolchain() {
        let artifact = super::SpirvArtifact {
            entry_name: "unknown".to_owned(),
            workgroup_size: [1, 1, 1],
            resources: Vec::new(),
            words: Vec::new(),
        };
        let binding = MetalBufferInput {
            binding: 0,
            resource_id: 1,
            bytes: vec![0; 16],
            element_stride: 4,
            writable: true,
        };
        assert!(matches!(
            execute_msl_artifact_external(
                &artifact,
                MetalDispatchGeometry {
                    threads_per_threadgroup: [1, 1, 1],
                    threadgroups: [1, 1, 1],
                },
                &[binding],
                0,
            ),
            Err(MetalError::InvalidInput(_))
        ));
    }

    #[test]
    fn source_report_executor_rejects_wrong_backend_before_metal_api() {
        let artifact = super::SpirvArtifact {
            entry_name: "main".to_owned(),
            workgroup_size: [1, 1, 1],
            resources: Vec::new(),
            words: jadren_codegen_spirv::emit_storage_add(
                "main",
                jadren_codegen_spirv::SpirvOptions::new([1, 1, 1]).unwrap(),
                1,
            )
            .unwrap(),
        };
        let report = jadren_gpu_runtime::ArtifactSourceTranslationReport::from_artifact(
            &artifact,
            jadren_gpu_runtime::ArtifactSourceBackend::Hlsl,
            "kernel void main() {}".to_owned(),
        )
        .unwrap();
        assert!(matches!(
            execute_msl_source_report(
                &report,
                MetalDispatchGeometry {
                    threads_per_threadgroup: [1, 1, 1],
                    threadgroups: [1, 1, 1],
                },
                &[],
                0,
            ),
            Err(MetalError::SourceBackendMismatch(
                jadren_gpu_runtime::ArtifactSourceBackend::Hlsl
            ))
        ));
    }

    #[test]
    fn source_report_executor_rejects_tampered_audit_before_metal_api() {
        let artifact = super::SpirvArtifact {
            entry_name: "main".to_owned(),
            workgroup_size: [1, 1, 1],
            resources: Vec::new(),
            words: jadren_codegen_spirv::emit_storage_add(
                "main",
                jadren_codegen_spirv::SpirvOptions::new([1, 1, 1]).unwrap(),
                1,
            )
            .unwrap(),
        };
        let mut report = jadren_gpu_runtime::ArtifactSourceTranslationReport::from_artifact(
            &artifact,
            jadren_gpu_runtime::ArtifactSourceBackend::Msl,
            "kernel void main() {}".to_owned(),
        )
        .unwrap();
        report.source_hash ^= 1;
        assert!(matches!(
            execute_msl_source_report(
                &report,
                MetalDispatchGeometry {
                    threads_per_threadgroup: [1, 1, 1],
                    threadgroups: [1, 1, 1],
                },
                &[],
                0,
            ),
            Err(MetalError::SourceReportMismatch(
                "source payload hash or byte count is inconsistent"
            ))
        ));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_host_is_explicitly_skipped() {
        assert!(matches!(run_device_smoke(), Err(MetalError::MacOsRequired)));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_vector_host_is_explicitly_skipped() {
        let input = [[7.0, 8.0, 9.0, 10.0]];
        assert!(matches!(
            run_f32_vector_artifact_smoke(&input),
            Err(MetalError::MacOsRequired)
        ));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_vector_binary_host_is_explicitly_skipped() {
        let input = [[7.0, 8.0, 9.0, 10.0]];
        assert!(matches!(
            run_f32_vector_binary_artifact_smoke(&input),
            Err(MetalError::MacOsRequired)
        ));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_vector_lanes_host_is_explicitly_skipped() {
        let input = vec![vec![7.0, 8.0], vec![10.0, 11.0]];
        assert!(matches!(
            run_f32_vector_lanes_binary_artifact_smoke(&input),
            Err(MetalError::MacOsRequired)
        ));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_scalar_host_is_explicitly_skipped() {
        assert!(matches!(
            run_f32_binary_artifact_smoke(&[7.0, 10.0]),
            Err(MetalError::MacOsRequired)
        ));
    }

    #[test]
    fn vector_input_contract_rejects_empty_and_non_finite_values() {
        assert!(matches!(
            run_f32_vector_artifact_smoke(&[]),
            Err(MetalError::InvalidInput(_))
        ));
        assert!(matches!(
            run_f32_binary_artifact_smoke(&[]),
            Err(MetalError::InvalidInput(_))
        ));
        assert!(matches!(
            run_f32_binary_artifact_smoke(&[f32::INFINITY]),
            Err(MetalError::InvalidInput(_))
        ));
        assert!(matches!(
            run_f32_vector_artifact_smoke(&[[f32::NAN, 0.0, 0.0, 0.0]]),
            Err(MetalError::InvalidInput(_))
        ));
        assert!(matches!(
            run_f32_vector_binary_artifact_smoke(&[[f32::NAN, 0.0, 0.0, 0.0]]),
            Err(MetalError::InvalidInput(_))
        ));
        assert!(matches!(
            run_f32_vector_lanes_binary_artifact_smoke(&[vec![1.0],]),
            Err(MetalError::InvalidInput(_))
        ));
        assert!(matches!(
            run_f32_vector_lanes_binary_artifact_smoke(&[vec![1.0, 2.0], vec![3.0]]),
            Err(MetalError::InvalidInput(_))
        ));
    }

    #[test]
    fn generic_msl_request_accepts_dense_structured_bindings() {
        let source = "kernel void main(device uint* data [[buffer(0)]]) {}";
        let bindings = [MetalBufferInput {
            binding: 0,
            resource_id: 1,
            bytes: vec![0; 16],
            element_stride: 4,
            writable: true,
        }];
        let geometry = MetalDispatchGeometry {
            threads_per_threadgroup: [64, 1, 1],
            threadgroups: [2, 1, 1],
        };
        assert!(validate_msl_source_execution(source, "main", geometry, &bindings, 0).is_ok());
    }

    #[test]
    fn generic_msl_request_rejects_sparse_stride_and_read_only_output() {
        let source = "kernel void main(device uint* data [[buffer(0)]]) {}";
        let geometry = MetalDispatchGeometry {
            threads_per_threadgroup: [64, 1, 1],
            threadgroups: [1, 1, 1],
        };
        let sparse = [MetalBufferInput {
            binding: 1,
            resource_id: 1,
            bytes: vec![0; 16],
            element_stride: 4,
            writable: true,
        }];
        assert!(matches!(
            validate_msl_source_execution(source, "main", geometry, &sparse, 0),
            Err(MetalError::InvalidInput(_))
        ));
        let misaligned = [MetalBufferInput {
            binding: 0,
            resource_id: 1,
            bytes: vec![0; 15],
            element_stride: 4,
            writable: true,
        }];
        assert!(matches!(
            validate_msl_source_execution(source, "main", geometry, &misaligned, 0),
            Err(MetalError::InvalidInput(_))
        ));
        let read_only = [MetalBufferInput {
            binding: 0,
            resource_id: 1,
            bytes: vec![0; 16],
            element_stride: 4,
            writable: false,
        }];
        assert!(matches!(
            validate_msl_source_execution(source, "main", geometry, &read_only, 0),
            Err(MetalError::InvalidInput(_))
        ));
    }

    #[test]
    fn generic_msl_request_allows_read_only_aliases() {
        let source = "kernel void main(device uint* data [[buffer(0)]]) {}";
        let geometry = MetalDispatchGeometry {
            threads_per_threadgroup: [64, 1, 1],
            threadgroups: [1, 1, 1],
        };
        let bindings = [
            MetalBufferInput {
                binding: 0,
                resource_id: 7,
                bytes: vec![1; 16],
                element_stride: 4,
                writable: false,
            },
            MetalBufferInput {
                binding: 1,
                resource_id: 7,
                bytes: vec![1; 16],
                element_stride: 4,
                writable: false,
            },
            MetalBufferInput {
                binding: 2,
                resource_id: 8,
                bytes: vec![0; 16],
                element_stride: 4,
                writable: true,
            },
        ];
        assert!(validate_msl_source_execution(source, "main", geometry, &bindings, 2).is_ok());
    }

    #[test]
    fn generic_msl_request_rejects_writable_aliases() {
        let source = "kernel void main(device uint* data [[buffer(0)]]) {}";
        let geometry = MetalDispatchGeometry {
            threads_per_threadgroup: [64, 1, 1],
            threadgroups: [1, 1, 1],
        };
        let bindings = [
            MetalBufferInput {
                binding: 0,
                resource_id: 9,
                bytes: vec![1; 16],
                element_stride: 4,
                writable: false,
            },
            MetalBufferInput {
                binding: 1,
                resource_id: 9,
                bytes: vec![0; 16],
                element_stride: 4,
                writable: true,
            },
        ];
        assert!(matches!(
            validate_msl_source_execution(source, "main", geometry, &bindings, 1),
            Err(MetalError::InvalidInput(
                "Metal writable resource aliases are forbidden"
            ))
        ));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn raw_msl_source_report_validates_mixed_capability_before_skip() {
        let source = "kernel void main(device const uint* input [[buffer(0)]], device uint* output [[buffer(1)]]) {}";
        let report = jadren_gpu_runtime::SpirvSourceTranslationReport {
            identity: jadren_gpu_runtime::SpirvSourceTranslationIdentity {
                backend: jadren_gpu_runtime::ArtifactSourceBackend::Msl,
                entry_name: "main".to_owned(),
                execution_model: 5,
                workgroup_size: Some([1, 1, 1]),
                workgroup_size_ids: None,
                workgroup_size_spec_ids: None,
                resources: vec![
                    jadren_gpu_runtime::SpirvRawResourceBinding {
                        variable_id: 1,
                        binding: 0,
                        descriptor_set: 0,
                        storage_class: Some(2),
                        element_type: Some(jadren_codegen_spirv::ResourceElementType::Integer {
                            signed: false,
                            bits: 32,
                            lanes: 1,
                        }),
                        element_stride: Some(4),
                        access: Some(jadren_codegen_spirv::ResourceAccess::ReadOnly),
                    },
                    jadren_gpu_runtime::SpirvRawResourceBinding {
                        variable_id: 2,
                        binding: 1,
                        descriptor_set: 0,
                        storage_class: Some(12),
                        element_type: Some(jadren_codegen_spirv::ResourceElementType::Integer {
                            signed: false,
                            bits: 32,
                            lanes: 1,
                        }),
                        element_stride: Some(4),
                        access: Some(jadren_codegen_spirv::ResourceAccess::ReadWrite),
                    },
                ],
                word_count: 5,
                word_hash: 1,
            },
            source: source.to_owned(),
            source_byte_count: source.len(),
            source_hash: jadren_gpu_runtime::stable_source_hash(source),
        };
        let bindings = [
            MetalBufferInput {
                binding: 0,
                resource_id: 1,
                bytes: vec![0; 16],
                element_stride: 4,
                writable: false,
            },
            MetalBufferInput {
                binding: 1,
                resource_id: 2,
                bytes: vec![0; 16],
                element_stride: 4,
                writable: true,
            },
        ];
        assert!(matches!(
            execute_msl_raw_source_report(
                &report,
                MetalDispatchGeometry {
                    threads_per_threadgroup: [1, 1, 1],
                    threadgroups: [1, 1, 1],
                },
                &bindings,
                1,
            ),
            Err(MetalError::MacOsRequired)
        ));
        assert!(matches!(
            execute_msl_raw_source_report_with_words(
                &report,
                &[0; 4],
                MetalDispatchGeometry {
                    threads_per_threadgroup: [1, 1, 1],
                    threadgroups: [1, 1, 1],
                },
                &bindings,
                1,
                &std::collections::BTreeMap::new(),
            ),
            Err(MetalError::SourceReportMismatch("raw source word count"))
        ));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_mixed_srv_uav_host_is_explicitly_skipped() {
        assert!(matches!(
            run_mixed_srv_uav_artifact_smoke(),
            Err(MetalError::MacOsRequired)
        ));
    }
}
