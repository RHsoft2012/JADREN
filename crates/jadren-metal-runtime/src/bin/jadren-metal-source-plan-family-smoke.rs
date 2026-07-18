use jadren_codegen_spirv::{F32ArithmeticOp, ResourceAccess, ResourceElementType};
use jadren_gpu_runtime::{
    GpuSupportedSubsetCase, JADREN_GPU_SUPPORTED_SUBSET_SCHEMA_V0_3,
    JADREN_GPU_SUPPORTED_SUBSET_V0_3, admit_gpu_supported_subset_v0_3,
    emit_gpu_supported_subset_case_words, inspect_spirv_source_module,
    select_spirv_raw_output_binding, validate_spirv_raw_native_adapter,
};
use serde::Serialize;

#[cfg(target_os = "macos")]
use jadren_codegen_msl::SpirvMslToolchain;
#[cfg(target_os = "macos")]
use jadren_gpu_runtime::{ArtifactSourceBackend, translate_spirv_source_report};
#[cfg(target_os = "macos")]
use jadren_metal_runtime::{
    MetalBufferInput, MetalDispatchGeometry, execute_msl_raw_source_report_with_words,
};
#[cfg(target_os = "macos")]
use sha2::{Digest, Sha256};
#[cfg(target_os = "macos")]
use std::{collections::BTreeMap, fs, path::Path, process::Command};

#[derive(Clone, Copy)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
enum U32Operation {
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

#[derive(Clone, Copy)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
enum WriteShape {
    OneDimensional,
    OneDimensionalStrided,
    TwoDimensional,
    TwoDimensionalStrided,
    ThreeDimensional,
    ThreeDimensionalStrided,
}

#[derive(Clone, Copy)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
enum CaseKind {
    U32(U32Operation),
    F32 {
        operation: F32ArithmeticOp,
        lanes: usize,
    },
    Write(WriteShape),
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
struct CaseDefinition {
    manifest: GpuSupportedSubsetCase,
    kind: CaseKind,
    words: Vec<u32>,
    workgroups: [u32; 3],
    logical_length: u32,
    capacity: u32,
    metadata: &'static [u32],
}

#[derive(Serialize)]
struct FamilyReport {
    schema: &'static str,
    supported_subset_schema: &'static str,
    supported_subset_manifest_case_count: u32,
    supported_subset_admitted_case_count: u32,
    platform: &'static str,
    metal_framework: &'static str,
    spirv_cross: Option<String>,
    spirv_cross_sha256: Option<String>,
    spirv_cross_version: Option<String>,
    planned_case_count: u32,
    executed_case_count: u32,
    cases: Vec<CaseReport>,
    result: &'static str,
    claim_scope: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
struct CaseReport {
    schema: &'static str,
    subset_case_id: &'static str,
    subset_admitted: bool,
    shape: &'static str,
    operation: &'static str,
    entry_name: &'static str,
    word_count: usize,
    word_hash: u64,
    resource_binding_count: usize,
    resource_access: Vec<&'static str>,
    output_binding: u32,
    output_selection_validated: bool,
    data_element_kind: &'static str,
    data_lanes: u16,
    data_stride: u32,
    workgroup_size: [u32; 3],
    workgroups: [u32; 3],
    invocation_count: u64,
    logical_length: u32,
    capacity: u32,
    msl_source_byte_count: Option<usize>,
    msl_source_hash: Option<u64>,
    first_output: Option<Vec<f64>>,
    last_output: Option<Vec<f64>>,
    output_checksum: Option<f64>,
    untouched_tail_count: Option<u32>,
    source_contract_validated: bool,
    pipeline_created: bool,
    command_queue_created: bool,
    command_buffer_created: bool,
    command_buffer_committed: bool,
    command_buffer_completed: bool,
    command_buffer_status: Option<i64>,
    execution_path: &'static str,
    execution_completed: bool,
    result: &'static str,
}

fn main() {
    let definitions = definitions().unwrap_or_else(|error| fail(error));
    #[cfg(target_os = "macos")]
    print_report(run_macos(definitions).unwrap_or_else(|error| fail(error)));
    #[cfg(not(target_os = "macos"))]
    print_report(skip_non_macos(definitions).unwrap_or_else(|error| fail(error)));
}

fn fail(error: String) -> ! {
    eprintln!("Metal source-plan family smoke failed: {error}");
    std::process::exit(1);
}

fn print_report(report: FamilyReport) {
    println!(
        "{}",
        serde_json::to_string(&report).expect("Metal source-plan family report is serializable")
    );
}

fn definitions() -> Result<Vec<CaseDefinition>, String> {
    JADREN_GPU_SUPPORTED_SUBSET_V0_3
        .cases
        .iter()
        .copied()
        .map(|manifest| {
            let (kind, workgroups, logical_length, capacity, metadata) =
                case_configuration(manifest.id)?;
            let words = emit_gpu_supported_subset_case_words(manifest.id)
                .map_err(|error| error.to_string())?;
            Ok(CaseDefinition {
                manifest,
                kind,
                words,
                workgroups,
                logical_length,
                capacity,
                metadata,
            })
        })
        .collect()
}

type CaseConfiguration = (CaseKind, [u32; 3], u32, u32, &'static [u32]);

fn case_configuration(case_id: &str) -> Result<CaseConfiguration, String> {
    let arithmetic = |kind| (kind, [2, 1, 1], 70, 128, &[][..]);
    Ok(match case_id {
        "u32.add" => arithmetic(CaseKind::U32(U32Operation::Add)),
        "u32.subtract" => arithmetic(CaseKind::U32(U32Operation::Subtract)),
        "u32.multiply" => arithmetic(CaseKind::U32(U32Operation::Multiply)),
        "u32.divide" => arithmetic(CaseKind::U32(U32Operation::Divide)),
        "u32.remainder" => arithmetic(CaseKind::U32(U32Operation::Remainder)),
        "u32.bitand" => arithmetic(CaseKind::U32(U32Operation::BitAnd)),
        "u32.bitor" => arithmetic(CaseKind::U32(U32Operation::BitOr)),
        "u32.bitxor" => arithmetic(CaseKind::U32(U32Operation::BitXor)),
        "u32.shift-left" => arithmetic(CaseKind::U32(U32Operation::ShiftLeft)),
        "u32.shift-right" => arithmetic(CaseKind::U32(U32Operation::ShiftRight)),
        "f32.add" => arithmetic(CaseKind::F32 {
            operation: F32ArithmeticOp::Add,
            lanes: 1,
        }),
        "f32.subtract" => arithmetic(CaseKind::F32 {
            operation: F32ArithmeticOp::Subtract,
            lanes: 1,
        }),
        "f32.multiply" => arithmetic(CaseKind::F32 {
            operation: F32ArithmeticOp::Multiply,
            lanes: 1,
        }),
        "f32x2.add" => arithmetic(CaseKind::F32 {
            operation: F32ArithmeticOp::Add,
            lanes: 2,
        }),
        "f32x2.subtract" => arithmetic(CaseKind::F32 {
            operation: F32ArithmeticOp::Subtract,
            lanes: 2,
        }),
        "f32x2.multiply" => arithmetic(CaseKind::F32 {
            operation: F32ArithmeticOp::Multiply,
            lanes: 2,
        }),
        "f32x3.add" => arithmetic(CaseKind::F32 {
            operation: F32ArithmeticOp::Add,
            lanes: 3,
        }),
        "f32x3.subtract" => arithmetic(CaseKind::F32 {
            operation: F32ArithmeticOp::Subtract,
            lanes: 3,
        }),
        "f32x3.multiply" => arithmetic(CaseKind::F32 {
            operation: F32ArithmeticOp::Multiply,
            lanes: 3,
        }),
        "f32x4.add" => arithmetic(CaseKind::F32 {
            operation: F32ArithmeticOp::Add,
            lanes: 4,
        }),
        "f32x4.subtract" => arithmetic(CaseKind::F32 {
            operation: F32ArithmeticOp::Subtract,
            lanes: 4,
        }),
        "f32x4.multiply" => arithmetic(CaseKind::F32 {
            operation: F32ArithmeticOp::Multiply,
            lanes: 4,
        }),
        "u32.write.1d" => (
            CaseKind::Write(WriteShape::OneDimensional),
            [1, 1, 1],
            64,
            64,
            &[],
        ),
        "u32.write.1d-strided" => (
            CaseKind::Write(WriteShape::OneDimensionalStrided),
            [2, 1, 1],
            70,
            160,
            &[70, 2, 160],
        ),
        "u32.write.2d" => (
            CaseKind::Write(WriteShape::TwoDimensional),
            [2, 1, 1],
            70,
            80,
            &[10, 7, 80],
        ),
        "u32.write.2d-strided" => (
            CaseKind::Write(WriteShape::TwoDimensionalStrided),
            [1, 1, 1],
            12,
            40,
            &[4, 3, 2, 10, 40],
        ),
        "u32.write.3d" => (
            CaseKind::Write(WriteShape::ThreeDimensional),
            [2, 1, 1],
            30,
            40,
            &[5, 3, 2, 40],
        ),
        "u32.write.3d-strided" => (
            CaseKind::Write(WriteShape::ThreeDimensionalStrided),
            [1, 1, 1],
            24,
            72,
            &[4, 3, 2, 2, 11, 37, 72],
        ),
        _ => return Err(format!("unknown manifest case `{case_id}`")),
    })
}

fn planned_case(definition: &CaseDefinition) -> Result<CaseReport, String> {
    let manifest = definition.manifest;
    let admitted =
        admit_gpu_supported_subset_v0_3(manifest.id, &definition.words, manifest.entry_name)
            .map_err(|error| {
                format!("{} supported-subset admission failed: {error}", manifest.id)
            })?;
    if admitted != manifest {
        return Err(format!(
            "{} admission returned a different manifest case",
            manifest.id
        ));
    }
    let contract = inspect_spirv_source_module(&definition.words, manifest.entry_name)
        .map_err(|error| error.to_string())?;
    if contract.workgroup_size != Some(manifest.workgroup_size)
        || contract.resources.len() != manifest.resources.len()
        || contract.word_count != definition.words.len()
    {
        return Err(format!(
            "{} raw SPIR-V contract differs from the manifest",
            manifest.id
        ));
    }
    let native_plan = validate_spirv_raw_native_adapter(&contract)
        .map_err(|error| format!("raw native adapter rejected {}: {error}", manifest.id))?;
    let output = select_spirv_raw_output_binding(&native_plan, manifest.output_binding)
        .map_err(|error| format!("{} output selection failed: {error}", manifest.id))?;
    let expected_output = manifest
        .resources
        .iter()
        .find(|resource| resource.binding == manifest.output_binding)
        .ok_or_else(|| format!("{} has no manifest output resource", manifest.id))?;
    if output.binding != manifest.output_binding || output.access != expected_output.access {
        return Err(format!(
            "{} output selection differs from the manifest",
            manifest.id
        ));
    }
    let resource_access = contract
        .resources
        .iter()
        .map(|resource| {
            resource
                .access
                .map(resource_access_label)
                .ok_or_else(|| "raw resource access is unknown".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (data_element_kind, data_lanes) = element_shape(expected_output.element_type);
    let invocation_count = manifest
        .workgroup_size
        .into_iter()
        .chain(definition.workgroups)
        .try_fold(1_u64, |product, value| {
            product.checked_mul(u64::from(value))
        })
        .ok_or_else(|| format!("{} invocation count overflow", manifest.id))?;
    Ok(CaseReport {
        schema: "jadren-metal-source-plan-family-case-0.3",
        subset_case_id: manifest.id,
        subset_admitted: true,
        shape: manifest.shape,
        operation: manifest.operation,
        entry_name: manifest.entry_name,
        word_count: manifest.word_count,
        word_hash: manifest.word_hash,
        resource_binding_count: contract.resources.len(),
        resource_access,
        output_binding: output.binding,
        output_selection_validated: true,
        data_element_kind,
        data_lanes,
        data_stride: expected_output.element_stride,
        workgroup_size: manifest.workgroup_size,
        workgroups: definition.workgroups,
        invocation_count,
        logical_length: definition.logical_length,
        capacity: definition.capacity,
        msl_source_byte_count: None,
        msl_source_hash: None,
        first_output: None,
        last_output: None,
        output_checksum: None,
        untouched_tail_count: None,
        source_contract_validated: true,
        pipeline_created: false,
        command_queue_created: false,
        command_buffer_created: false,
        command_buffer_committed: false,
        command_buffer_completed: false,
        command_buffer_status: None,
        execution_path: "not-run-macos-required",
        execution_completed: false,
        result: "skip-case-macos-required",
    })
}

const fn element_shape(element: ResourceElementType) -> (&'static str, u16) {
    match element {
        ResourceElementType::Integer { lanes, .. } => ("integer", lanes),
        ResourceElementType::Float { lanes, .. } => ("float", lanes),
    }
}

#[cfg(not(target_os = "macos"))]
fn skip_non_macos(definitions: Vec<CaseDefinition>) -> Result<FamilyReport, String> {
    let cases = definitions
        .iter()
        .map(planned_case)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(FamilyReport {
        schema: "jadren-metal-source-plan-family-execution-0.3",
        supported_subset_schema: JADREN_GPU_SUPPORTED_SUBSET_SCHEMA_V0_3,
        supported_subset_manifest_case_count: JADREN_GPU_SUPPORTED_SUBSET_V0_3.case_count as u32,
        supported_subset_admitted_case_count: cases.len() as u32,
        platform: std::env::consts::OS,
        metal_framework: "not-run-macos-required",
        spirv_cross: None,
        spirv_cross_sha256: None,
        spirv_cross_version: None,
        planned_case_count: cases.len() as u32,
        executed_case_count: 0,
        cases,
        result: "skip-macos-required",
        claim_scope: "canonical 28-case raw SPIR-V identity only; no Metal compilation, dispatch, completion or readback claim on a non-macOS host",
        error: Some("native Metal source-plan execution requires macOS".to_owned()),
    })
}

#[cfg(target_os = "macos")]
fn run_macos(definitions: Vec<CaseDefinition>) -> Result<FamilyReport, String> {
    let toolchain = SpirvMslToolchain::discover()
        .ok_or_else(|| "SPIRV-Cross is required through JADREN_SPIRV_CROSS or PATH".to_owned())?;
    let tool_hash = sha256_file(&toolchain.spirv_cross)?;
    let tool_version = spirv_cross_version(&toolchain.spirv_cross)?;
    let cases = definitions
        .iter()
        .map(|definition| execute_case(definition, &toolchain.spirv_cross))
        .collect::<Result<Vec<_>, _>>()?;
    if cases.len() != JADREN_GPU_SUPPORTED_SUBSET_V0_3.case_count
        || cases.iter().any(|case| !case.execution_completed)
    {
        return Err("Metal source-plan family did not complete all 28 cases".to_owned());
    }
    Ok(FamilyReport {
        schema: "jadren-metal-source-plan-family-execution-0.3",
        supported_subset_schema: JADREN_GPU_SUPPORTED_SUBSET_SCHEMA_V0_3,
        supported_subset_manifest_case_count: JADREN_GPU_SUPPORTED_SUBSET_V0_3.case_count as u32,
        supported_subset_admitted_case_count: cases.len() as u32,
        platform: std::env::consts::OS,
        metal_framework: "loaded",
        spirv_cross: Some(toolchain.spirv_cross.display().to_string()),
        spirv_cross_sha256: Some(tool_hash),
        spirv_cross_version: Some(tool_version),
        planned_case_count: cases.len() as u32,
        executed_case_count: cases.len() as u32,
        cases,
        result: "pass-macos-native-source-plan-family",
        claim_scope: "canonical 28-case raw SPIR-V through SHA-256 identified SPIRV-Cross MSL, native Metal pipeline, dispatch, completion and exact readback differential",
        error: None,
    })
}

#[cfg(target_os = "macos")]
fn execute_case(definition: &CaseDefinition, tool: &Path) -> Result<CaseReport, String> {
    let mut case = planned_case(definition)?;
    let manifest = definition.manifest;
    let source = translate_spirv_source_report(
        &definition.words,
        manifest.entry_name,
        tool,
        ArtifactSourceBackend::Msl,
    )
    .map_err(|error| error.to_string())?;
    if source.identity.word_hash != case.word_hash
        || source.identity.word_count != case.word_count
        || source.identity.workgroup_size != Some(manifest.workgroup_size)
    {
        return Err(format!(
            "{} MSL report lost raw SPIR-V identity",
            manifest.id
        ));
    }
    let bindings = metal_bindings(definition);
    let (execution, output) = execute_msl_raw_source_report_with_words(
        &source,
        &definition.words,
        MetalDispatchGeometry {
            threads_per_threadgroup: manifest.workgroup_size,
            threadgroups: definition.workgroups,
        },
        &bindings,
        manifest.output_binding,
        &BTreeMap::new(),
    )
    .map_err(|error| error.to_string())?;
    let evidence = verify_output(definition, &output)?;
    if execution.metal_framework != "loaded"
        || !execution.pipeline_created
        || !execution.command_queue_created
        || !execution.command_buffer_created
        || !execution.command_buffer_committed
        || !execution.command_buffer_completed
        || execution.command_buffer_status != Some(4)
        || !execution.execution_completed
    {
        return Err(format!("{} Metal lifecycle is incomplete", manifest.id));
    }
    case.msl_source_byte_count = Some(source.source_byte_count);
    case.msl_source_hash = Some(source.source_hash);
    case.first_output = Some(evidence.first);
    case.last_output = Some(evidence.last);
    case.output_checksum = Some(evidence.checksum);
    case.untouched_tail_count = Some(evidence.untouched_tail_count);
    case.pipeline_created = execution.pipeline_created;
    case.command_queue_created = execution.command_queue_created;
    case.command_buffer_created = execution.command_buffer_created;
    case.command_buffer_committed = execution.command_buffer_committed;
    case.command_buffer_completed = execution.command_buffer_completed;
    case.command_buffer_status = execution.command_buffer_status;
    case.execution_path = "spirv-cross-msl-metal";
    case.execution_completed = execution.execution_completed;
    case.result = "pass-source-plan-metal-readback-case";
    Ok(case)
}

#[cfg(target_os = "macos")]
fn metal_bindings(definition: &CaseDefinition) -> Vec<MetalBufferInput> {
    let manifest = definition.manifest;
    match definition.kind {
        CaseKind::U32(_) => {
            let input = (0..definition.capacity)
                .flat_map(|index| (41_u32 + index).to_le_bytes())
                .collect();
            vec![
                metal_binding(manifest, 0, input),
                metal_binding(manifest, 1, vec![0; definition.capacity as usize * 4]),
                metal_binding(
                    manifest,
                    2,
                    definition.logical_length.to_le_bytes().to_vec(),
                ),
            ]
        }
        CaseKind::F32 { lanes, .. } => {
            let input = (0..definition.capacity)
                .flat_map(|index| {
                    let base = 7.0_f32 + index as f32 * 3.0;
                    (0..lanes).flat_map(move |lane| (base + lane as f32).to_le_bytes())
                })
                .collect();
            vec![
                metal_binding(manifest, 0, input),
                metal_binding(
                    manifest,
                    1,
                    vec![0; definition.capacity as usize * lanes * 4],
                ),
                metal_binding(
                    manifest,
                    2,
                    definition.logical_length.to_le_bytes().to_vec(),
                ),
            ]
        }
        CaseKind::Write(_) => manifest
            .resources
            .iter()
            .enumerate()
            .map(|(index, resource)| {
                let bytes = if index == 0 {
                    vec![0; definition.capacity as usize * 4]
                } else {
                    definition.metadata[index - 1].to_le_bytes().to_vec()
                };
                MetalBufferInput {
                    binding: resource.binding,
                    resource_id: u64::from(resource.binding) + 1,
                    bytes,
                    element_stride: resource.element_stride as usize,
                    writable: resource.access.can_write(),
                }
            })
            .collect(),
    }
}

#[cfg(target_os = "macos")]
fn metal_binding(
    manifest: GpuSupportedSubsetCase,
    binding: u32,
    bytes: Vec<u8>,
) -> MetalBufferInput {
    let resource = manifest
        .resources
        .iter()
        .find(|resource| resource.binding == binding)
        .expect("canonical arithmetic binding exists");
    MetalBufferInput {
        binding,
        resource_id: u64::from(binding) + 1,
        bytes,
        element_stride: resource.element_stride as usize,
        writable: resource.access.can_write(),
    }
}

#[cfg(target_os = "macos")]
struct OutputEvidence {
    first: Vec<f64>,
    last: Vec<f64>,
    checksum: f64,
    untouched_tail_count: u32,
}

#[cfg(target_os = "macos")]
fn verify_output(definition: &CaseDefinition, output: &[u8]) -> Result<OutputEvidence, String> {
    let manifest = definition.manifest;
    let output_resource = manifest
        .resources
        .iter()
        .find(|resource| resource.binding == manifest.output_binding)
        .expect("canonical output binding exists");
    let expected_bytes = definition.capacity as usize * output_resource.element_stride as usize;
    if output.len() != expected_bytes {
        return Err(format!(
            "{} returned {} bytes instead of {expected_bytes}",
            manifest.id,
            output.len()
        ));
    }
    match definition.kind {
        CaseKind::U32(operation) => verify_u32_output(definition, output, operation),
        CaseKind::F32 { operation, lanes } => {
            verify_f32_output(definition, output, operation, lanes)
        }
        CaseKind::Write(shape) => verify_write_output(definition, output, shape),
    }
}

#[cfg(target_os = "macos")]
fn verify_u32_output(
    definition: &CaseDefinition,
    output: &[u8],
    operation: U32Operation,
) -> Result<OutputEvidence, String> {
    let actual = output_u32(output);
    let expected = (0..definition.capacity)
        .map(|index| {
            if index < definition.logical_length {
                apply_u32(41 + index, operation)
            } else {
                0
            }
        })
        .collect::<Vec<_>>();
    if actual != expected {
        return Err(format!(
            "{} Metal readback differential failed",
            definition.manifest.id
        ));
    }
    Ok(u32_evidence(&actual, definition.logical_length as usize))
}

#[cfg(target_os = "macos")]
fn verify_f32_output(
    definition: &CaseDefinition,
    output: &[u8],
    operation: F32ArithmeticOp,
    lanes: usize,
) -> Result<OutputEvidence, String> {
    let actual = output_f32(output);
    let expected = (0..definition.capacity)
        .flat_map(|index| {
            (0..lanes).map(move |lane| {
                if index < definition.logical_length {
                    apply_f32(7.0 + index as f32 * 3.0 + lane as f32, operation)
                } else {
                    0.0
                }
            })
        })
        .collect::<Vec<_>>();
    if actual != expected {
        return Err(format!(
            "{} Metal readback differential failed",
            definition.manifest.id
        ));
    }
    Ok(f32_evidence(
        &actual,
        lanes,
        definition.logical_length as usize,
    ))
}

#[cfg(target_os = "macos")]
fn verify_write_output(
    definition: &CaseDefinition,
    output: &[u8],
    shape: WriteShape,
) -> Result<OutputEvidence, String> {
    let actual = output_u32(output);
    let indices = write_indices(shape);
    let mut expected = vec![0_u32; definition.capacity as usize];
    for &index in &indices {
        expected[index] = 42;
    }
    if actual != expected || indices.len() != definition.logical_length as usize {
        return Err(format!(
            "{} Metal write differential failed",
            definition.manifest.id
        ));
    }
    let first_index = *indices.first().expect("write family is non-empty");
    let last_index = *indices.last().expect("write family is non-empty");
    Ok(OutputEvidence {
        first: vec![f64::from(actual[first_index])],
        last: vec![f64::from(actual[last_index])],
        checksum: actual.iter().map(|value| f64::from(*value)).sum(),
        untouched_tail_count: actual.iter().filter(|value| **value == 0).count() as u32,
    })
}

#[cfg(target_os = "macos")]
fn write_indices(shape: WriteShape) -> Vec<usize> {
    match shape {
        WriteShape::OneDimensional => (0..64).collect(),
        WriteShape::OneDimensionalStrided => (0..70).map(|index| index * 2).collect(),
        WriteShape::TwoDimensional => (0..7)
            .flat_map(|y| (0..10).map(move |x| y * 10 + x))
            .collect(),
        WriteShape::TwoDimensionalStrided => (0..3)
            .flat_map(|y| (0..4).map(move |x| x * 2 + y * 10))
            .collect(),
        WriteShape::ThreeDimensional => (0..2)
            .flat_map(|z| (0..3).flat_map(move |y| (0..5).map(move |x| ((z * 3 + y) * 5) + x)))
            .collect(),
        WriteShape::ThreeDimensionalStrided => (0..2)
            .flat_map(|z| (0..3).flat_map(move |y| (0..4).map(move |x| x * 2 + y * 11 + z * 37)))
            .collect(),
    }
}

#[cfg(target_os = "macos")]
fn output_u32(output: &[u8]) -> Vec<u32> {
    output
        .chunks_exact(4)
        .map(|bytes| u32::from_le_bytes(bytes.try_into().expect("u32 output chunk")))
        .collect()
}

#[cfg(target_os = "macos")]
fn output_f32(output: &[u8]) -> Vec<f32> {
    output
        .chunks_exact(4)
        .map(|bytes| f32::from_le_bytes(bytes.try_into().expect("f32 output chunk")))
        .collect()
}

#[cfg(target_os = "macos")]
const fn apply_u32(value: u32, operation: U32Operation) -> u32 {
    match operation {
        U32Operation::Add => value.wrapping_add(1),
        U32Operation::Subtract => value.wrapping_sub(1),
        U32Operation::Multiply => value.wrapping_mul(2),
        U32Operation::Divide => value / 2,
        U32Operation::Remainder => value % 2,
        U32Operation::BitAnd => value & 1,
        U32Operation::BitOr => value | 1,
        U32Operation::BitXor => value ^ 1,
        U32Operation::ShiftLeft => value << 1,
        U32Operation::ShiftRight => value >> 1,
    }
}

#[cfg(target_os = "macos")]
fn apply_f32(value: f32, operation: F32ArithmeticOp) -> f32 {
    match operation {
        F32ArithmeticOp::Add => value + 1.0,
        F32ArithmeticOp::Subtract => value - 1.0,
        F32ArithmeticOp::Multiply => value * 2.0,
    }
}

#[cfg(target_os = "macos")]
fn u32_evidence(actual: &[u32], logical_length: usize) -> OutputEvidence {
    OutputEvidence {
        first: vec![f64::from(actual[0])],
        last: vec![f64::from(actual[logical_length - 1])],
        checksum: actual[..logical_length]
            .iter()
            .map(|value| f64::from(*value))
            .sum(),
        untouched_tail_count: actual[logical_length..]
            .iter()
            .filter(|value| **value == 0)
            .count() as u32,
    }
}

#[cfg(target_os = "macos")]
fn f32_evidence(actual: &[f32], lanes: usize, logical_length: usize) -> OutputEvidence {
    let logical_values = logical_length * lanes;
    OutputEvidence {
        first: actual[..lanes]
            .iter()
            .map(|value| f64::from(*value))
            .collect(),
        last: actual[logical_values - lanes..logical_values]
            .iter()
            .map(|value| f64::from(*value))
            .collect(),
        checksum: actual[..logical_values]
            .iter()
            .map(|value| f64::from(*value))
            .sum(),
        untouched_tail_count: actual[logical_values..]
            .chunks_exact(lanes)
            .filter(|values| values.iter().all(|value| *value == 0.0))
            .count() as u32,
    }
}

const fn resource_access_label(access: ResourceAccess) -> &'static str {
    match access {
        ResourceAccess::ReadOnly => "read_only",
        ResourceAccess::WriteOnly => "write_only",
        ResourceAccess::ReadWrite => "read_write",
    }
}

#[cfg(target_os = "macos")]
fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|error| format!("cannot read SPIRV-Cross: {error}"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[cfg(target_os = "macos")]
fn spirv_cross_version(path: &Path) -> Result<String, String> {
    let output = Command::new(path)
        .arg("--version")
        .output()
        .map_err(|error| format!("cannot query SPIRV-Cross version: {error}"))?;
    let text = if output.stdout.is_empty() {
        String::from_utf8_lossy(&output.stderr)
    } else {
        String::from_utf8_lossy(&output.stdout)
    };
    let version = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .trim()
        .to_owned();
    if version.is_empty() {
        return Err("SPIRV-Cross --version returned empty output".to_owned());
    }
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_cases_match_supported_subset_manifest() {
        let definitions = definitions().unwrap();
        assert_eq!(
            definitions.len(),
            JADREN_GPU_SUPPORTED_SUBSET_V0_3.case_count
        );
        for (definition, expected) in definitions
            .iter()
            .zip(JADREN_GPU_SUPPORTED_SUBSET_V0_3.cases)
        {
            assert_eq!(definition.manifest, *expected);
            assert_eq!(definition.words.len(), expected.word_count);
            assert_eq!(
                jadren_gpu_runtime::stable_spirv_word_hash(&definition.words),
                expected.word_hash
            );
            let report = planned_case(definition).unwrap();
            assert_eq!(report.subset_case_id, expected.id);
            assert_eq!(report.resource_binding_count, expected.resources.len());
            assert_eq!(report.output_binding, expected.output_binding);
        }
    }
}
