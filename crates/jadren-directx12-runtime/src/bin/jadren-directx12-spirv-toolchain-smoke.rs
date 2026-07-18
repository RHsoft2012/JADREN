use jadren_codegen_spirv::{
    F32ArithmeticOp, ResourceAccess, ResourceElementType, SpirvOptions, emit_compute,
    emit_storage_add_artifact_from_jir,
    emit_storage_global_index_binary_dynamic_length_artifact_from_jir,
    emit_storage_global_index_f32_binary_dynamic_length_artifact_from_jir,
    emit_storage_global_index_vector_f32_binary_dynamic_length_artifact_from_jir,
};
use jadren_directx12_runtime::{
    DirectX12ArtifactExecutionReport, DirectX12F32ArtifactExecutionReport,
    DirectX12F32VectorArtifactExecutionReport, SpirvDxilToolchain, UavBindingPayload,
    execute_hlsl_source_artifact, execute_spirv_source_report_with_words,
    run_binary_artifact_smoke, run_f32_binary_artifact_smoke, run_f32_vector_binary_artifact_smoke,
    run_storage_add_artifact_smoke, translate_jir_dynamic_storage_binary_to_dxil,
    translate_jir_dynamic_storage_f32_binary_to_dxil, translate_jir_storage_add_to_dxil,
    translate_spirv_source_to_dxil_report, translate_spirv_to_dxil,
};
use jadren_gpu_runtime::{
    ArtifactDispatchRequest, ArtifactSourceBackend, BackendProbe, FpPolicy, GpuBackend,
    SpirvRawResourceBinding, SpirvSourceExecutionRequest, compare_spirv_source_execution_plans,
    compare_spirv_source_reports, inspect_spirv_source_module, plan_spirv_source_execution,
    select_spirv_raw_output_binding, stable_spirv_word_hash, translate_spirv_source_report,
    validate_spirv_raw_native_adapter,
};
use jadren_jir::{
    AddressSpace, BinaryOp, Block, BlockId, BuiltinOp, Constant, Function, FunctionId, Instruction,
    InstructionKind, Linkage, Module, Parameter, Terminator, Type, TypeId, TypedValue, ValueId,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Serialize)]
struct Report {
    schema: &'static str,
    entry_name: String,
    workgroup_size: [u32; 3],
    word_count: usize,
    resource_binding_count: usize,
    dynamic_entry_name: String,
    dynamic_word_count: usize,
    dynamic_resource_binding_count: usize,
    dynamic_operation: &'static str,
    raw_compute_entry_name: String,
    raw_compute_word_count: usize,
    raw_compute_dxil_translated: bool,
    raw_hlsl_source_translated: bool,
    raw_msl_source_translated: bool,
    raw_source_parity_passed: bool,
    resourceful_raw_compute_entry_name: String,
    resourceful_raw_compute_word_count: usize,
    resourceful_raw_compute_resource_binding_count: usize,
    resourceful_raw_resource_capabilities: Vec<RawResourceCapability>,
    resourceful_raw_native_adapter_contract: &'static str,
    resourceful_raw_backend_view_contract: &'static str,
    resourceful_raw_compute_dxil_translated: bool,
    resourceful_raw_hlsl_source_translated: bool,
    resourceful_raw_msl_source_translated: bool,
    resourceful_raw_source_parity_passed: bool,
    resourceful_raw_source_execution_plan_parity_passed: bool,
    resourceful_raw_source_execution_passed: bool,
    artifact_input_start: u32,
    artifact_input_stride: u32,
    jir_spirv_contract: &'static str,
    dynamic_jir_spirv_contract: &'static str,
    spirv_cross: Option<String>,
    spirv_cross_sha256: Option<String>,
    dxc: Option<String>,
    dxc_sha256: Option<String>,
    artifact_execution: Option<DirectX12ArtifactExecutionReport>,
    artifact_execution_cases: Vec<DirectX12ArtifactExecutionReport>,
    storage_add_artifact_execution: Option<DirectX12ArtifactExecutionReport>,
    f32_artifact_execution: Option<DirectX12F32ArtifactExecutionReport>,
    f32_artifact_execution_cases: Vec<DirectX12F32ArtifactExecutionReport>,
    f32_vector_artifact_execution_cases: Vec<DirectX12F32VectorArtifactExecutionReport>,
    source_execution_plan_cases: Vec<SourceExecutionPlanCase>,
    mixed_srv_uav_execution: Option<MixedSrvUavExecution>,
    mixed_raw_srv_uav_execution: Option<MixedSrvUavExecution>,
    result: &'static str,
}

#[derive(Serialize)]
struct SourceExecutionPlanCase {
    schema: &'static str,
    shape: &'static str,
    operation: &'static str,
    entry_name: String,
    word_count: usize,
    word_hash: u64,
    resource_binding_count: usize,
    resource_access: [&'static str; 3],
    output_binding: u32,
    output_selection_validated: bool,
    data_element_kind: &'static str,
    data_lanes: u16,
    data_stride: u32,
    workgroup_size: [u32; 3],
    workgroups: [u32; 3],
    invocation_count: u64,
    hlsl_source_hash: u64,
    msl_source_hash: u64,
    plan_parity_passed: bool,
    dx12_artifact_word_hash_match: bool,
    dx12_execution: &'static str,
    result: &'static str,
}

#[derive(Clone, Copy)]
struct NativeExecutionEvidence {
    word_hash: u64,
    execution_completed: bool,
    translation_path: &'static str,
}

#[derive(Serialize)]
struct MixedSrvUavExecution {
    schema: &'static str,
    entry_name: String,
    view_contract: &'static str,
    logical_length: u32,
    capacity: u32,
    first_output: u32,
    last_output: u32,
    output_checksum: u64,
    expected_checksum: u64,
    untouched_tail_count: u32,
    dxil_translated: bool,
    execution_completed: bool,
    readback_differential: &'static str,
    result: &'static str,
}

#[derive(Serialize)]
struct RawResourceCapability {
    variable_id: u32,
    binding: u32,
    descriptor_set: u32,
    storage_class: Option<u32>,
    element_type: Option<RawResourceElementType>,
    element_stride: Option<u32>,
    access: Option<&'static str>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RawResourceElementType {
    Integer { signed: bool, bits: u16, lanes: u16 },
    Float { bits: u16, lanes: u16 },
}

fn main() {
    let (artifact_input_start, artifact_input_stride) = parse_input_arguments();
    let module = jir_storage_add_module();
    let artifact = emit_storage_add_artifact_from_jir(
        &module,
        FunctionId::new(0),
        SpirvOptions::new([1, 1, 1]).expect("smoke workgroup is valid"),
    )
    .unwrap_or_else(|error| {
        eprintln!("JIR→SPIR-V contract failed: {error}");
        std::process::exit(1);
    });
    let dynamic_module =
        jir_dynamic_binary_module("global_multiply_dynamic_u32", BinaryOp::Multiply, 3);
    let dynamic_artifact = emit_storage_global_index_binary_dynamic_length_artifact_from_jir(
        &dynamic_module,
        FunctionId::new(0),
        SpirvOptions::new([64, 1, 1]).expect("dynamic smoke workgroup is valid"),
        BinaryOp::Multiply,
    )
    .unwrap_or_else(|error| {
        eprintln!("dynamic JIR→SPIR-V contract failed: {error}");
        std::process::exit(1);
    });
    let raw_compute_module = jir_empty_compute_module();
    let raw_compute_words = emit_compute(
        &raw_compute_module,
        FunctionId::new(0),
        SpirvOptions::new([1, 1, 1]).expect("raw compute workgroup is valid"),
    )
    .unwrap_or_else(|error| {
        eprintln!("raw compute SPIR-V contract failed: {error}");
        std::process::exit(1);
    });
    let resourceful_raw_contract =
        inspect_spirv_source_module(&dynamic_artifact.words, &dynamic_artifact.entry_name)
            .unwrap_or_else(|error| {
                eprintln!("resourceful raw SPIR-V contract inspection failed: {error}");
                std::process::exit(1);
            });
    let resourceful_raw_resource_capabilities =
        raw_resource_capabilities(&resourceful_raw_contract.resources);
    let raw_native_plan = validate_spirv_raw_native_adapter(&resourceful_raw_contract)
        .unwrap_or_else(|error| {
            eprintln!("resourceful raw native adapter contract failed: {error}");
            std::process::exit(1);
        });
    for backend in [GpuBackend::Vulkan, GpuBackend::DirectX12, GpuBackend::Metal] {
        if raw_native_plan.project_backend(backend).resources.len()
            != resourceful_raw_contract.resources.len()
        {
            eprintln!("resourceful raw backend view projection lost a resource");
            std::process::exit(1);
        }
    }
    let f32_requests = f32_artifact_requests();
    let artifact_execution_cases = if cfg!(windows) {
        artifact_requests()
            .into_iter()
            .map(|(operation, operand)| {
                let module = jir_dynamic_binary_module(
                    dynamic_entry_name(operation),
                    operation,
                    i128::from(operand),
                );
                run_binary_artifact_smoke(
                    &module,
                    FunctionId::new(0),
                    SpirvOptions::new([64, 1, 1]).expect("dynamic smoke workgroup is valid"),
                    operation,
                    operand,
                    artifact_input_start,
                    artifact_input_stride,
                )
                .unwrap_or_else(|error| {
                    eprintln!("artifact DX12 execution failed: {error}");
                    std::process::exit(1);
                })
            })
            .collect()
    } else {
        Vec::new()
    };
    let artifact_execution = artifact_execution_cases
        .iter()
        .find(|case| case.operation == "multiply")
        .cloned();
    let storage_add_artifact_execution = if cfg!(windows) {
        Some(
            run_storage_add_artifact_smoke(
                &module,
                FunctionId::new(0),
                SpirvOptions::new([1, 1, 1]).expect("storage-add workgroup is valid"),
                1,
                41,
                64,
            )
            .unwrap_or_else(|error| {
                eprintln!("storage-add artifact DX12 execution failed: {error}");
                std::process::exit(1);
            }),
        )
    } else {
        None
    };
    let f32_artifact_execution_cases = if cfg!(windows) {
        f32_requests
            .iter()
            .map(|(operation, entry_name, operand)| {
                let module = jir_dynamic_f32_module(entry_name, operand.to_bits(), *operation);
                run_f32_binary_artifact_smoke(
                    &module,
                    FunctionId::new(0),
                    SpirvOptions::new([64, 1, 1]).expect("f32 smoke workgroup is valid"),
                    *operand,
                    *operation,
                )
                .unwrap_or_else(|error| {
                    eprintln!("f32 binary artifact DX12 execution failed: {error}");
                    std::process::exit(1);
                })
            })
            .collect()
    } else {
        Vec::new()
    };
    let f32_artifact_execution = f32_artifact_execution_cases
        .iter()
        .find(|case| case.operation == "add")
        .cloned();
    let f32_vector_input_values = f32_vector_input_values();
    let f32_vector_artifact_execution_cases = if cfg!(windows) {
        f32_vector_artifact_requests()
            .into_iter()
            .map(|(operation, operand)| {
                run_f32_vector_binary_artifact_smoke(
                    &jir_dynamic_f32x4_module(operation, operand),
                    FunctionId::new(0),
                    SpirvOptions::new([64, 1, 1]).expect("f32x4 smoke workgroup is valid"),
                    &f32_vector_input_values,
                    operation,
                )
                .unwrap_or_else(|error| {
                    eprintln!("f32x4 binary artifact DX12 execution failed: {error}");
                    std::process::exit(1);
                })
            })
            .collect()
    } else {
        Vec::new()
    };
    let toolchain = SpirvDxilToolchain::discover();
    let mixed_srv_uav_execution = if cfg!(windows) && toolchain.is_some() {
        Some(run_mixed_srv_uav_execution(&dynamic_artifact))
    } else {
        None
    };
    let mixed_raw_srv_uav_execution = if cfg!(windows) && toolchain.is_some() {
        Some(run_mixed_raw_srv_uav_execution(&dynamic_artifact))
    } else {
        None
    };
    let (
        spirv_cross,
        spirv_cross_sha256,
        dxc,
        dxc_sha256,
        raw_compute_dxil_translated,
        raw_hlsl_source_translated,
        raw_msl_source_translated,
        raw_source_parity_passed,
        resourceful_raw_compute_dxil_translated,
        resourceful_raw_hlsl_source_translated,
        resourceful_raw_msl_source_translated,
        resourceful_raw_source_parity_passed,
        resourceful_raw_source_execution_plan_parity_passed,
        resourceful_raw_source_execution_passed,
        source_execution_plan_cases,
        result,
    ) = match toolchain {
        Some(toolchain) => {
            let spirv_cross_sha256 = hash_tool(&toolchain.spirv_cross, "SPIRV-Cross");
            let dxc_sha256 = hash_tool(&toolchain.dxc, "DXC");
            let raw_hlsl_source = translate_spirv_source_report(
                &raw_compute_words,
                "raw_compute",
                &toolchain.spirv_cross,
                ArtifactSourceBackend::Hlsl,
            )
            .unwrap_or_else(|error| {
                eprintln!("raw SPIR-V→HLSL source translation failed: {error}");
                std::process::exit(1);
            });
            let raw_msl_source = translate_spirv_source_report(
                &raw_compute_words,
                "raw_compute",
                &toolchain.spirv_cross,
                ArtifactSourceBackend::Msl,
            )
            .unwrap_or_else(|error| {
                eprintln!("raw SPIR-V→MSL source translation failed: {error}");
                std::process::exit(1);
            });
            let raw_hlsl_resource_count = raw_hlsl_source.identity.resources.len();
            let raw_msl_resource_count = raw_msl_source.identity.resources.len();
            compare_spirv_source_reports(&[raw_hlsl_source, raw_msl_source]).unwrap_or_else(
                |error| {
                    eprintln!("raw SPIR-V source parity failed: {error}");
                    std::process::exit(1);
                },
            );
            if raw_hlsl_resource_count != 0 || raw_msl_resource_count != 0 {
                eprintln!("raw no-resource SPIR-V unexpectedly exposed descriptor decorations");
                std::process::exit(1);
            }
            if let Err(error) = translate_spirv_to_dxil(&raw_compute_words, "raw_compute") {
                eprintln!("raw SPIR-V→DXIL translation failed: {error}");
                std::process::exit(1);
            }
            let resourceful_raw_hlsl_source = translate_spirv_source_report(
                &dynamic_artifact.words,
                &dynamic_artifact.entry_name,
                &toolchain.spirv_cross,
                ArtifactSourceBackend::Hlsl,
            )
            .unwrap_or_else(|error| {
                eprintln!("resourceful raw SPIR-V→HLSL source translation failed: {error}");
                std::process::exit(1);
            });
            let resourceful_raw_msl_source = translate_spirv_source_report(
                &dynamic_artifact.words,
                &dynamic_artifact.entry_name,
                &toolchain.spirv_cross,
                ArtifactSourceBackend::Msl,
            )
            .unwrap_or_else(|error| {
                eprintln!("resourceful raw SPIR-V→MSL source translation failed: {error}");
                std::process::exit(1);
            });
            let resourceful_raw_hlsl_resource_count =
                resourceful_raw_hlsl_source.identity.resources.len();
            let resourceful_raw_msl_resource_count =
                resourceful_raw_msl_source.identity.resources.len();
            let expected_resource_type = ResourceElementType::Integer {
                signed: false,
                bits: 32,
                lanes: 1,
            };
            let resourceful_type_policy_passed = resourceful_raw_hlsl_source
                .identity
                .resources
                .iter()
                .enumerate()
                .all(|(index, resource)| {
                    resource.element_type == Some(expected_resource_type)
                        && resource.element_stride == Some(4)
                        && resource.access
                            == Some(match index {
                                0 | 2 => ResourceAccess::ReadOnly,
                                1 => ResourceAccess::WriteOnly,
                                _ => return false,
                            })
                });
            compare_spirv_source_reports(&[
                resourceful_raw_hlsl_source,
                resourceful_raw_msl_source,
            ])
            .unwrap_or_else(|error| {
                eprintln!("resourceful raw SPIR-V source parity failed: {error}");
                std::process::exit(1);
            });
            if resourceful_raw_hlsl_resource_count != dynamic_artifact.resources.len()
                || resourceful_raw_msl_resource_count != dynamic_artifact.resources.len()
            {
                eprintln!(
                    "resourceful raw SPIR-V descriptor count does not match artifact metadata"
                );
                std::process::exit(1);
            }
            if !resourceful_type_policy_passed {
                eprintln!("resourceful raw SPIR-V scalar/access reflection contract failed");
                std::process::exit(1);
            }
            if let Err(error) =
                translate_spirv_to_dxil(&dynamic_artifact.words, &dynamic_artifact.entry_name)
            {
                eprintln!("resourceful raw SPIR-V→DXIL translation failed: {error}");
                std::process::exit(1);
            }
            let resourceful_raw_dxil_report = translate_spirv_source_to_dxil_report(
                &dynamic_artifact.words,
                &dynamic_artifact.entry_name,
            )
            .unwrap_or_else(|error| {
                eprintln!("resourceful raw source report creation failed: {error}");
                std::process::exit(1);
            });
            if !run_resourceful_raw_source_execution(
                &resourceful_raw_dxil_report,
                &dynamic_artifact.words,
            ) {
                eprintln!("resourceful raw source report execution differential failed");
                std::process::exit(1);
            }
            if let Err(error) = translate_jir_storage_add_to_dxil(
                &module,
                FunctionId::new(0),
                SpirvOptions::new([1, 1, 1]).expect("smoke workgroup is valid"),
            ) {
                eprintln!("JIR→SPIR-V→DXIL translation failed: {error}");
                std::process::exit(1);
            }
            if let Err(error) = translate_jir_dynamic_storage_binary_to_dxil(
                &dynamic_module,
                FunctionId::new(0),
                SpirvOptions::new([64, 1, 1]).expect("dynamic smoke workgroup is valid"),
                BinaryOp::Multiply,
            ) {
                eprintln!("dynamic JIR→SPIR-V→DXIL translation failed: {error}");
                std::process::exit(1);
            }
            for (operation, entry_name, operand) in f32_requests {
                let module = jir_dynamic_f32_module(entry_name, operand.to_bits(), operation);
                if let Err(error) = translate_jir_dynamic_storage_f32_binary_to_dxil(
                    &module,
                    FunctionId::new(0),
                    SpirvOptions::new([64, 1, 1]).expect("f32 smoke workgroup is valid"),
                    operation,
                ) {
                    eprintln!(
                        "dynamic f32 {operation:?} JIR→SPIR-V→DXIL translation failed: {error}"
                    );
                    std::process::exit(1);
                }
            }
            (
                Some(toolchain.spirv_cross.display().to_string()),
                Some(spirv_cross_sha256),
                Some(toolchain.dxc.display().to_string()),
                Some(dxc_sha256),
                true,
                true,
                true,
                true,
                true,
                true,
                true,
                true,
                true,
                true,
                build_source_execution_plan_cases(
                    &dynamic_artifact,
                    &f32_artifact_execution_cases,
                    &f32_vector_artifact_execution_cases,
                    &toolchain.spirv_cross,
                ),
                "pass-jir-spirv-dxil",
            )
        }
        None => (
            None,
            None,
            None,
            None,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            Vec::new(),
            "skip-toolchain-unavailable",
        ),
    };
    let report = Report {
        schema: "jadren-directx12-spirv-toolchain-smoke-1.0",
        entry_name: artifact.entry_name,
        workgroup_size: artifact.workgroup_size,
        word_count: artifact.words.len(),
        resource_binding_count: artifact.resources.len(),
        dynamic_entry_name: dynamic_artifact.entry_name.clone(),
        dynamic_word_count: dynamic_artifact.words.len(),
        dynamic_resource_binding_count: dynamic_artifact.resources.len(),
        dynamic_operation: "multiply",
        raw_compute_entry_name: "raw_compute".to_owned(),
        raw_compute_word_count: raw_compute_words.len(),
        raw_compute_dxil_translated,
        raw_hlsl_source_translated,
        raw_msl_source_translated,
        raw_source_parity_passed,
        resourceful_raw_compute_entry_name: dynamic_artifact.entry_name.clone(),
        resourceful_raw_compute_word_count: dynamic_artifact.words.len(),
        resourceful_raw_compute_resource_binding_count: dynamic_artifact.resources.len(),
        resourceful_raw_resource_capabilities,
        resourceful_raw_native_adapter_contract: "passed",
        resourceful_raw_backend_view_contract: "passed",
        resourceful_raw_compute_dxil_translated,
        resourceful_raw_hlsl_source_translated,
        resourceful_raw_msl_source_translated,
        resourceful_raw_source_parity_passed,
        resourceful_raw_source_execution_plan_parity_passed,
        resourceful_raw_source_execution_passed,
        artifact_input_start,
        artifact_input_stride,
        jir_spirv_contract: "passed",
        dynamic_jir_spirv_contract: "passed",
        spirv_cross,
        spirv_cross_sha256,
        dxc,
        dxc_sha256,
        artifact_execution,
        artifact_execution_cases,
        storage_add_artifact_execution,
        f32_artifact_execution,
        f32_artifact_execution_cases,
        f32_vector_artifact_execution_cases,
        source_execution_plan_cases,
        mixed_srv_uav_execution,
        mixed_raw_srv_uav_execution,
        result,
    };
    println!(
        "{}",
        serde_json::to_string(&report).expect("toolchain report is serializable")
    );
}

fn hash_tool(path: &Path, label: &str) -> String {
    let bytes = std::fs::read(path).unwrap_or_else(|error| {
        eprintln!("failed to read {label} executable for provenance: {error}");
        std::process::exit(1);
    });
    format!("{:x}", Sha256::digest(bytes))
}

fn run_mixed_srv_uav_execution(
    source_artifact: &jadren_codegen_spirv::SpirvArtifact,
) -> MixedSrvUavExecution {
    let source = format!(
        r#"StructuredBuffer<uint> input : register(t0);
StructuredBuffer<uint> length : register(t2);
RWStructuredBuffer<uint> output : register(u1);
[numthreads(64, 1, 1)]
void {entry}(uint3 gid : SV_DispatchThreadID) {{
    uint index = gid.x;
    uint logical = length[0];
    if (index < logical) {{
        output[index] = input[index] * 3;
        }}
}}"#,
        entry = source_artifact.entry_name
    );
    run_mixed_srv_uav_case(
        source_artifact,
        source,
        "jadren-directx12-mixed-srv-uav-execution-0.1",
        "mixed SRV/UAV",
    )
}

fn run_mixed_raw_srv_uav_execution(
    source_artifact: &jadren_codegen_spirv::SpirvArtifact,
) -> MixedSrvUavExecution {
    let source = format!(
        r#"ByteAddressBuffer input : register(t0);
StructuredBuffer<uint> length : register(t2);
RWStructuredBuffer<uint> output : register(u1);
[numthreads(64, 1, 1)]
void {entry}(uint3 gid : SV_DispatchThreadID) {{
    uint index = gid.x;
    uint logical = length[0];
    if (index < logical) {{
        output[index] = input.Load(index * 4) * 3;
    }}
}}"#,
        entry = source_artifact.entry_name
    );
    run_mixed_srv_uav_case(
        source_artifact,
        source,
        "jadren-directx12-mixed-raw-srv-uav-execution-0.1",
        "mixed raw SRV/UAV",
    )
}

fn run_mixed_srv_uav_case(
    source_artifact: &jadren_codegen_spirv::SpirvArtifact,
    source: String,
    schema: &'static str,
    label: &str,
) -> MixedSrvUavExecution {
    let mut artifact = source_artifact.clone();
    artifact.resources[0].access = ResourceAccess::ReadOnly;
    artifact.resources[2].access = ResourceAccess::ReadOnly;
    let input_values = (41_u32..111).collect::<Vec<_>>();
    let output_values = vec![0_u32; 128];
    let length_values = vec![input_values.len() as u32];
    let input_bytes = u32_bytes(&input_values);
    let output_bytes = u32_bytes(&output_values);
    let length_bytes = u32_bytes(&length_values);
    let bindings = [
        UavBindingPayload {
            resource_id: 100,
            bytes: &input_bytes,
            element_stride: 4,
        },
        UavBindingPayload {
            resource_id: 101,
            bytes: &output_bytes,
            element_stride: 4,
        },
        UavBindingPayload {
            resource_id: 102,
            bytes: &length_bytes,
            element_stride: 4,
        },
    ];
    let actual_bytes = execute_hlsl_source_artifact(&artifact, &source, [2, 1, 1], &bindings, 1)
        .unwrap_or_else(|error| {
            eprintln!("{label} DX12 execution failed: {error}");
            std::process::exit(1);
        });
    let actual = actual_bytes
        .chunks_exact(4)
        .map(|bytes| u32::from_ne_bytes(bytes.try_into().expect("u32 output chunk")))
        .collect::<Vec<_>>();
    let expected = input_values
        .iter()
        .map(|value| value * 3)
        .chain(std::iter::repeat_n(0, 128 - input_values.len()))
        .collect::<Vec<_>>();
    if actual != expected {
        eprintln!("{label} DX12 readback differential failed");
        std::process::exit(1);
    }
    let output_checksum = actual.iter().map(|value| u64::from(*value)).sum();
    let expected_checksum = expected.iter().map(|value| u64::from(*value)).sum();
    let untouched_tail_count = actual[input_values.len()..]
        .iter()
        .filter(|value| **value == 0)
        .count() as u32;
    MixedSrvUavExecution {
        schema,
        entry_name: artifact.entry_name,
        view_contract: "passed",
        logical_length: input_values.len() as u32,
        capacity: actual.len() as u32,
        first_output: actual[0],
        last_output: actual[input_values.len() - 1],
        output_checksum,
        expected_checksum,
        untouched_tail_count,
        dxil_translated: true,
        execution_completed: true,
        readback_differential: "passed",
        result: "pass-mixed-srv-uav-differential",
    }
}

fn run_resourceful_raw_source_execution(
    report: &jadren_directx12_runtime::DxilSourceTranslationReport,
    words: &[u32],
) -> bool {
    let input_values = (41_u32..111).collect::<Vec<_>>();
    let output_values = vec![0_u32; 128];
    let length_values = vec![input_values.len() as u32];
    let input_bytes = u32_bytes(&input_values);
    let output_bytes = u32_bytes(&output_values);
    let length_bytes = u32_bytes(&length_values);
    let bindings = [
        UavBindingPayload {
            resource_id: 100,
            bytes: &input_bytes,
            element_stride: 4,
        },
        UavBindingPayload {
            resource_id: 101,
            bytes: &output_bytes,
            element_stride: 4,
        },
        UavBindingPayload {
            resource_id: 102,
            bytes: &length_bytes,
            element_stride: 4,
        },
    ];
    let actual_bytes = match execute_spirv_source_report_with_words(
        report,
        words,
        [2, 1, 1],
        &bindings,
        1,
        &std::collections::BTreeMap::new(),
    ) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("resourceful raw source report DX12 execution failed: {error}");
            return false;
        }
    };
    let actual = actual_bytes
        .chunks_exact(4)
        .map(|bytes| u32::from_ne_bytes(bytes.try_into().expect("u32 output chunk")))
        .collect::<Vec<_>>();
    let expected = input_values
        .iter()
        .map(|value| value * 3)
        .chain(std::iter::repeat_n(0, 128 - input_values.len()))
        .collect::<Vec<_>>();
    actual == expected
}

fn build_source_execution_plan_cases(
    dynamic_artifact: &jadren_codegen_spirv::SpirvArtifact,
    f32_executions: &[DirectX12F32ArtifactExecutionReport],
    f32_vector_executions: &[DirectX12F32VectorArtifactExecutionReport],
    spirv_cross: &std::path::Path,
) -> Vec<SourceExecutionPlanCase> {
    let mut cases = Vec::with_capacity(7);
    cases.push(build_source_execution_plan_case(
        "u32",
        "multiply",
        dynamic_artifact,
        ResourceElementType::Integer {
            signed: false,
            bits: 32,
            lanes: 1,
        },
        4,
        Some(NativeExecutionEvidence {
            word_hash: stable_spirv_word_hash(&dynamic_artifact.words),
            execution_completed: true,
            translation_path: "spirv-cross-dxc",
        }),
        spirv_cross,
    ));
    for (operation, entry_name, operand) in f32_artifact_requests() {
        let module = jir_dynamic_f32_module(entry_name, operand.to_bits(), operation);
        let artifact = emit_storage_global_index_f32_binary_dynamic_length_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([64, 1, 1]).expect("f32 source-plan workgroup is valid"),
            operation,
        )
        .unwrap_or_else(|error| {
            eprintln!("scalar f32 source-plan artifact emission failed: {error}");
            std::process::exit(1);
        });
        let operation_name = f32_operation_name(operation);
        let native = f32_executions
            .iter()
            .find(|report| report.operation == operation_name)
            .map(|report| NativeExecutionEvidence {
                word_hash: report.artifact_word_hash,
                execution_completed: report.execution_completed,
                translation_path: report.translation_path,
            });
        cases.push(build_source_execution_plan_case(
            "f32",
            operation_name,
            &artifact,
            ResourceElementType::Float { bits: 32, lanes: 1 },
            4,
            native,
            spirv_cross,
        ));
    }
    for (operation, operand) in f32_vector_artifact_requests() {
        let module = jir_dynamic_f32x4_module(operation, operand);
        let artifact =
            emit_storage_global_index_vector_f32_binary_dynamic_length_artifact_from_jir(
                &module,
                FunctionId::new(0),
                SpirvOptions::new([64, 1, 1]).expect("f32x4 source-plan workgroup is valid"),
                operation,
            )
            .unwrap_or_else(|error| {
                eprintln!("f32x4 source-plan artifact emission failed: {error}");
                std::process::exit(1);
            });
        let operation_name = f32_operation_name(operation);
        let native = f32_vector_executions
            .iter()
            .find(|report| report.operation == operation_name)
            .map(|report| NativeExecutionEvidence {
                word_hash: report.artifact_word_hash,
                execution_completed: report.execution_completed,
                translation_path: report.translation_path,
            });
        cases.push(build_source_execution_plan_case(
            "f32x4",
            operation_name,
            &artifact,
            ResourceElementType::Float { bits: 32, lanes: 4 },
            16,
            native,
            spirv_cross,
        ));
    }
    if cases.len() != 7 {
        eprintln!("source-execution plan family must contain exactly seven cases");
        std::process::exit(1);
    }
    cases
}

fn build_source_execution_plan_case(
    shape: &'static str,
    operation: &'static str,
    artifact: &jadren_codegen_spirv::SpirvArtifact,
    expected_data_type: ResourceElementType,
    expected_data_stride: u32,
    native: Option<NativeExecutionEvidence>,
    spirv_cross: &std::path::Path,
) -> SourceExecutionPlanCase {
    let request = ArtifactDispatchRequest {
        fp: FpPolicy::Fast,
        require_bounded_global_u32_array: false,
        require_async_completion: true,
    };
    let spec_values = std::collections::BTreeMap::new();
    let dx12 = match plan_spirv_source_execution(SpirvSourceExecutionRequest {
        backend: GpuBackend::DirectX12,
        probe: BackendProbe {
            device_available: true,
            shader_translation_available: true,
            ..BackendProbe::prototype(GpuBackend::DirectX12)
        },
        request,
        words: &artifact.words,
        entry_name: &artifact.entry_name,
        tool: spirv_cross,
        spec_values: &spec_values,
        workgroups: [2, 1, 1],
    }) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("{shape}/{operation} DX12 source-execution plan failed: {error}");
            std::process::exit(1);
        }
    };
    let metal = match plan_spirv_source_execution(SpirvSourceExecutionRequest {
        backend: GpuBackend::Metal,
        probe: BackendProbe {
            device_available: true,
            shader_translation_available: true,
            ..BackendProbe::prototype(GpuBackend::Metal)
        },
        request,
        words: &artifact.words,
        entry_name: &artifact.entry_name,
        tool: spirv_cross,
        spec_values: &spec_values,
        workgroups: [2, 1, 1],
    }) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("{shape}/{operation} Metal source-execution plan failed: {error}");
            std::process::exit(1);
        }
    };
    let parity = match compare_spirv_source_execution_plans(&[dx12, metal]) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("{shape}/{operation} DX12/Metal source-execution parity failed: {error}");
            std::process::exit(1);
        }
    };
    let u32_scalar = ResourceElementType::Integer {
        signed: false,
        bits: 32,
        lanes: 1,
    };
    let resources_match = parity.resources.len() == 3
        && parity
            .resources
            .iter()
            .enumerate()
            .all(|(index, resource)| {
                let (element_type, stride) = if index < 2 {
                    (expected_data_type, expected_data_stride)
                } else {
                    (u32_scalar, 4)
                };
                resource.binding == index as u32
                    && resource.descriptor_set == 0
                    && resource.element_type == Some(element_type)
                    && resource.element_stride == Some(stride)
                    && resource.access
                        == Some(match index {
                            0 | 2 => ResourceAccess::ReadOnly,
                            1 => ResourceAccess::WriteOnly,
                            _ => return false,
                        })
            });
    let exact_contract = inspect_spirv_source_module(&artifact.words, &artifact.entry_name)
        .unwrap_or_else(|error| {
            eprintln!("{shape}/{operation} exact output contract inspection failed: {error}");
            std::process::exit(1);
        });
    let exact_native = validate_spirv_raw_native_adapter(&exact_contract).unwrap_or_else(|error| {
        eprintln!("{shape}/{operation} exact output native plan failed: {error}");
        std::process::exit(1);
    });
    let output_selection =
        select_spirv_raw_output_binding(&exact_native, 1).unwrap_or_else(|error| {
            eprintln!("{shape}/{operation} explicit output selection failed: {error}");
            std::process::exit(1);
        });
    let passed = parity.entry_name == artifact.entry_name
        && parity.backends == [GpuBackend::DirectX12, GpuBackend::Metal]
        && parity.source_backends == [ArtifactSourceBackend::Hlsl, ArtifactSourceBackend::Msl]
        && parity.workgroup_size == [64, 1, 1]
        && parity.workgroups == [2, 1, 1]
        && parity.invocation_count == 128
        && parity.word_count == artifact.words.len()
        && parity.source_hashes.len() == 2
        && parity.source_hashes[0] != 0
        && parity.source_hashes[1] != 0
        && parity.source_hashes[0] != parity.source_hashes[1]
        && output_selection.resource_index == 1
        && output_selection.access == ResourceAccess::WriteOnly
        && resources_match;
    if !passed {
        eprintln!("{shape}/{operation} DX12/Metal source-execution parity report is incomplete");
        std::process::exit(1);
    }
    let (dx12_artifact_word_hash_match, dx12_execution) = match native {
        Some(native) => {
            let hash_matches = native.word_hash == parity.word_hash;
            if !hash_matches
                || !native.execution_completed
                || native.translation_path != "spirv-cross-dxc"
            {
                eprintln!(
                    "{shape}/{operation} source plan is not tied to a real matching DX12 execution"
                );
                std::process::exit(1);
            }
            (true, "passed")
        }
        None if cfg!(windows) => {
            eprintln!("{shape}/{operation} is missing its Windows DX12 execution evidence");
            std::process::exit(1);
        }
        None => (false, "not-run-platform"),
    };
    let (data_element_kind, data_lanes) = match expected_data_type {
        ResourceElementType::Integer { lanes, .. } => ("integer", lanes),
        ResourceElementType::Float { lanes, .. } => ("float", lanes),
    };
    SourceExecutionPlanCase {
        schema: "jadren-directx12-metal-source-plan-case-0.2",
        shape,
        operation,
        entry_name: parity.entry_name,
        word_count: parity.word_count,
        word_hash: parity.word_hash,
        resource_binding_count: parity.resources.len(),
        resource_access: ["read_only", "write_only", "read_only"],
        output_binding: output_selection.binding,
        output_selection_validated: true,
        data_element_kind,
        data_lanes,
        data_stride: expected_data_stride,
        workgroup_size: parity.workgroup_size,
        workgroups: parity.workgroups,
        invocation_count: parity.invocation_count,
        hlsl_source_hash: parity.source_hashes[0],
        msl_source_hash: parity.source_hashes[1],
        plan_parity_passed: true,
        dx12_artifact_word_hash_match,
        dx12_execution,
        result: if native.is_some() {
            "pass-source-plan-and-dx12-execution"
        } else {
            "pass-source-plan-native-not-run-platform"
        },
    }
}

fn u32_bytes(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect()
}

fn raw_resource_capabilities(resources: &[SpirvRawResourceBinding]) -> Vec<RawResourceCapability> {
    resources
        .iter()
        .map(|resource| RawResourceCapability {
            variable_id: resource.variable_id,
            binding: resource.binding,
            descriptor_set: resource.descriptor_set,
            storage_class: resource.storage_class,
            element_type: resource
                .element_type
                .map(|element_type| match element_type {
                    ResourceElementType::Integer {
                        signed,
                        bits,
                        lanes,
                    } => RawResourceElementType::Integer {
                        signed,
                        bits,
                        lanes,
                    },
                    ResourceElementType::Float { bits, lanes } => {
                        RawResourceElementType::Float { bits, lanes }
                    }
                }),
            element_stride: resource.element_stride,
            access: resource.access.map(|access| match access {
                ResourceAccess::ReadOnly => "read_only",
                ResourceAccess::WriteOnly => "write_only",
                ResourceAccess::ReadWrite => "read_write",
            }),
        })
        .collect()
}

fn jir_empty_compute_module() -> Module {
    Module {
        types: vec![Type::Unit],
        functions: vec![Function {
            id: FunctionId::new(0),
            name: "raw_compute".to_owned(),
            linkage: Linkage::Export,
            parameters: Vec::new(),
            result: TypeId::new(0),
            blocks: vec![Block {
                id: BlockId::new(0),
                parameters: Vec::new(),
                instructions: Vec::new(),
                terminator: Terminator::Return { value: None },
                span: None,
            }],
            span: None,
        }],
    }
}

fn f32_artifact_requests() -> [(F32ArithmeticOp, &'static str, f32); 3] {
    [
        (F32ArithmeticOp::Add, "global_add_dynamic_f32", 1.0),
        (
            F32ArithmeticOp::Subtract,
            "global_subtract_dynamic_f32",
            1.0,
        ),
        (
            F32ArithmeticOp::Multiply,
            "global_multiply_dynamic_f32",
            2.0,
        ),
    ]
}

fn f32_vector_artifact_requests() -> [(F32ArithmeticOp, f32); 3] {
    [
        (F32ArithmeticOp::Add, 1.0),
        (F32ArithmeticOp::Subtract, 1.0),
        (F32ArithmeticOp::Multiply, 2.0),
    ]
}

fn f32_vector_input_values() -> Vec<[f32; 4]> {
    (0..70_u32)
        .map(|index| {
            let base = 7.0_f32 + index as f32 * 3.0_f32;
            [base, base + 1.0, base + 2.0, base + 3.0]
        })
        .collect()
}

const fn f32_operation_name(operation: F32ArithmeticOp) -> &'static str {
    match operation {
        F32ArithmeticOp::Add => "add",
        F32ArithmeticOp::Subtract => "subtract",
        F32ArithmeticOp::Multiply => "multiply",
    }
}

fn parse_input_arguments() -> (u32, u32) {
    let mut input_start = 41_u32;
    let mut input_stride = 1_u32;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        let target = match argument.as_str() {
            "--input-start" => &mut input_start,
            "--input-stride" => &mut input_stride,
            _ => {
                eprintln!("error=unknown argument `{argument}`");
                std::process::exit(2);
            }
        };
        let Some(value) = arguments.next() else {
            eprintln!("error=missing value for {argument}");
            std::process::exit(2);
        };
        *target = match value.parse() {
            Ok(value) => value,
            Err(error) => {
                eprintln!("error=invalid {argument} `{value}`: {error}");
                std::process::exit(2);
            }
        };
    }
    (input_start, input_stride)
}

fn artifact_requests() -> [(BinaryOp, u32); 10] {
    [
        (BinaryOp::Add, 1),
        (BinaryOp::Subtract, 1),
        (BinaryOp::Multiply, 2),
        (BinaryOp::Divide, 2),
        (BinaryOp::Remainder, 2),
        (BinaryOp::BitAnd, 1),
        (BinaryOp::BitOr, 1),
        (BinaryOp::BitXor, 1),
        (BinaryOp::ShiftLeft, 1),
        (BinaryOp::ShiftRight, 1),
    ]
}

const fn dynamic_entry_name(operation: BinaryOp) -> &'static str {
    match operation {
        BinaryOp::Add => "global_add_dynamic_u32",
        BinaryOp::Subtract => "global_subtract_dynamic_u32",
        BinaryOp::Multiply => "global_multiply_dynamic_u32",
        BinaryOp::Divide => "global_divide_dynamic_u32",
        BinaryOp::Remainder => "global_remainder_dynamic_u32",
        BinaryOp::BitAnd => "global_bitand_dynamic_u32",
        BinaryOp::BitOr => "global_bitor_dynamic_u32",
        BinaryOp::BitXor => "global_bitxor_dynamic_u32",
        BinaryOp::ShiftLeft => "global_shift_left_dynamic_u32",
        BinaryOp::ShiftRight => "global_shift_right_dynamic_u32",
    }
}

fn jir_dynamic_binary_module(name: &str, operation: BinaryOp, operand: i128) -> Module {
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
            Type::Pointer {
                pointee: TypeId::new(1),
                address_space: AddressSpace::Storage,
            },
        ],
        functions: vec![Function {
            id: FunctionId::new(0),
            name: name.to_owned(),
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
                            ty: TypeId::new(1),
                        }),
                        InstructionKind::Constant(Constant::Integer { value: operand }),
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(5),
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
                            length: ValueId::new(5),
                        },
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(6),
                            ty: TypeId::new(2),
                        }),
                        InstructionKind::Offset {
                            base: ValueId::new(0),
                            indices: vec![ValueId::new(3)],
                        },
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(7),
                            ty: TypeId::new(1),
                        }),
                        InstructionKind::Load {
                            pointer: ValueId::new(6),
                            alignment: 4,
                            volatile: false,
                        },
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(8),
                            ty: TypeId::new(1),
                        }),
                        InstructionKind::Binary {
                            op: operation,
                            left: ValueId::new(7),
                            right: ValueId::new(4),
                        },
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(9),
                            ty: TypeId::new(2),
                        }),
                        InstructionKind::Offset {
                            base: ValueId::new(1),
                            indices: vec![ValueId::new(3)],
                        },
                    ),
                    instruction(
                        None,
                        InstructionKind::Store {
                            pointer: ValueId::new(9),
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

fn jir_dynamic_f32_module(name: &str, operand_bits: u32, operation: F32ArithmeticOp) -> Module {
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
            name: name.to_owned(),
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
                        InstructionKind::Constant(Constant::FloatBits {
                            bits: u64::from(operand_bits),
                        }),
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(5),
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
                            length: ValueId::new(5),
                        },
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(6),
                            ty: TypeId::new(3),
                        }),
                        InstructionKind::Offset {
                            base: ValueId::new(0),
                            indices: vec![ValueId::new(3)],
                        },
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(7),
                            ty: TypeId::new(2),
                        }),
                        InstructionKind::Load {
                            pointer: ValueId::new(6),
                            alignment: 4,
                            volatile: false,
                        },
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(8),
                            ty: TypeId::new(2),
                        }),
                        InstructionKind::Binary {
                            op: operation.as_binary_op(),
                            left: ValueId::new(7),
                            right: ValueId::new(4),
                        },
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(9),
                            ty: TypeId::new(3),
                        }),
                        InstructionKind::Offset {
                            base: ValueId::new(1),
                            indices: vec![ValueId::new(3)],
                        },
                    ),
                    instruction(
                        None,
                        InstructionKind::Store {
                            pointer: ValueId::new(9),
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

fn jir_dynamic_f32x4_module(operation: F32ArithmeticOp, operand: f32) -> Module {
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
            name: format!("global_{}_dynamic_f32x4", f32_operation_name(operation)),
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
                            lanes: 4,
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
                            alignment: 16,
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
                            op: operation.as_binary_op(),
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
    }
}

fn jir_storage_add_module() -> Module {
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
                        kind: InstructionKind::Constant(Constant::Integer { value: 1 }),
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
