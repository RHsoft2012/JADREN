use std::path::PathBuf;

use jadren_gpu_runtime::{
    ArtifactDispatchRequest, ArtifactSourceBackend, BackendProbe, FpPolicy, GpuBackend,
    SpirvSourceExecutionParityError, SpirvSourceExecutionPlanError, SpirvSourceExecutionRequest,
    SpirvSourceReportWordsError, compare_spirv_source_execution_plans,
    compare_spirv_source_reports, plan_spirv_source_execution, stable_source_hash,
    translate_spirv_source_report, translate_spirv_source_report_for_backend,
    validate_spirv_source_report_words,
};
use std::collections::BTreeMap;

fn fixture_tool() -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_jadren_spirv_cross_fixture") {
        return PathBuf::from(path);
    }
    let suffix = std::env::consts::EXE_SUFFIX;
    let test_directory = std::env::current_exe()
        .expect("test executable path should be available")
        .parent()
        .expect("test executable should have a parent directory")
        .to_owned();
    let names = [
        format!("jadren-spirv-cross-fixture{suffix}"),
        format!("jadren_spirv_cross_fixture{suffix}"),
    ];
    let mut candidates = names.iter().flat_map(|name| {
        [
            test_directory.join(name),
            test_directory
                .parent()
                .map(|directory| directory.join(name))
                .unwrap_or_default(),
        ]
    });
    candidates
        .find(|path| path.is_file())
        .expect("Cargo should build the SPIRV-Cross fixture binary")
}

#[test]
fn shared_process_boundary_translates_fixture_for_msl_and_hlsl() {
    let words = [
        0x0723_0203,
        0x0001_0000,
        0,
        8,
        0,
        (7_u32 << 16) | 15,
        5,
        1,
        0x7478_6966,
        0x5f65_7275,
        0x6e69_616d,
        0,
    ];
    let reports = [
        (ArtifactSourceBackend::Msl, "kernel void fixture_main("),
        (ArtifactSourceBackend::Hlsl, "void fixture_main("),
    ]
    .into_iter()
    .map(|(backend, marker)| {
        let report =
            translate_spirv_source_report(&words, "fixture_main", &fixture_tool(), backend)
                .expect("fixture source translation should succeed");
        assert_eq!(report.identity.backend, backend);
        assert_eq!(report.identity.entry_name, "fixture_main");
        assert_eq!(report.identity.execution_model, 5);
        assert!(report.identity.resources.is_empty());
        assert_eq!(report.identity.word_count, words.len());
        assert!(
            report.source.contains(marker),
            "source was: {}",
            report.source
        );
        assert_eq!(report.source_hash, stable_source_hash(&report.source));
        report
    })
    .collect::<Vec<_>>();
    let parity = compare_spirv_source_reports(&reports).expect("fixture source parity should pass");
    assert_eq!(parity.entry_name, "fixture_main");
    assert_eq!(parity.execution_model, 5);
    assert!(parity.resources.is_empty());
    assert_eq!(parity.word_count, words.len());
    assert_eq!(parity.sources.len(), 2);
}

#[test]
fn canonical_backend_route_selects_source_language_and_rejects_native_route() {
    let words = [
        0x0723_0203,
        0x0001_0000,
        0,
        8,
        0,
        (7_u32 << 16) | 15,
        5,
        1,
        0x7478_6966,
        0x5f65_7275,
        0x6e69_616d,
        0,
    ];
    let hlsl = translate_spirv_source_report_for_backend(
        &words,
        "fixture_main",
        &fixture_tool(),
        GpuBackend::DirectX12,
    )
    .expect("DX12 route should select HLSL");
    assert_eq!(hlsl.identity.backend, ArtifactSourceBackend::Hlsl);
    assert!(hlsl.source.contains("void fixture_main("));
    let msl = translate_spirv_source_report_for_backend(
        &words,
        "fixture_main",
        &fixture_tool(),
        GpuBackend::Metal,
    )
    .expect("Metal route should select MSL");
    assert_eq!(msl.identity.backend, ArtifactSourceBackend::Msl);
    assert!(msl.source.contains("kernel void fixture_main("));
    assert_eq!(
        translate_spirv_source_report_for_backend(
            &words,
            "fixture_main",
            &fixture_tool(),
            GpuBackend::Vulkan,
        ),
        Err(
            jadren_gpu_runtime::SpirvSourceTranslationError::InvalidInput(
                "backend uses native SPIR-V transport",
            )
        )
    );
}

#[test]
fn source_execution_plan_preserves_raw_identity_for_dx12_and_metal() {
    let options = jadren_codegen_spirv::SpirvOptions::new([8, 1, 1]).unwrap();
    let words = jadren_codegen_spirv::emit_storage_add("add_u32", options, 1).unwrap();
    let request = ArtifactDispatchRequest {
        fp: FpPolicy::Fast,
        require_bounded_global_u32_array: false,
        require_async_completion: true,
    };
    for backend in [GpuBackend::DirectX12, GpuBackend::Metal] {
        let probe = BackendProbe {
            device_available: true,
            shader_translation_available: true,
            ..BackendProbe::prototype(backend)
        };
        let tool = fixture_tool();
        let specialization = BTreeMap::new();
        let plan = plan_spirv_source_execution(SpirvSourceExecutionRequest {
            backend,
            probe,
            request,
            words: &words,
            entry_name: "add_u32",
            tool: &tool,
            spec_values: &specialization,
            workgroups: [2, 1, 1],
        })
        .expect("source execution plan should preserve raw identity");
        assert_eq!(plan.raw_dispatch.backend.backend, backend);
        assert_eq!(plan.raw_dispatch.workgroup_size, [8, 1, 1]);
        assert_eq!(plan.raw_dispatch.workgroups, [2, 1, 1]);
        assert_eq!(plan.raw_dispatch.invocation_count, 16);
        assert_eq!(plan.source.identity.entry_name, "add_u32");
        assert_eq!(plan.source.identity.word_count, words.len());
        assert_eq!(
            plan.source.identity.word_hash,
            jadren_gpu_runtime::stable_spirv_word_hash(&words)
        );
        assert_eq!(
            plan.source.identity.backend,
            match backend {
                GpuBackend::DirectX12 => ArtifactSourceBackend::Hlsl,
                GpuBackend::Metal => ArtifactSourceBackend::Msl,
                GpuBackend::Vulkan => unreachable!(),
            }
        );
    }
}

#[test]
fn source_execution_plan_rejects_vulkan_native_transport_before_tool_lookup() {
    let specialization = BTreeMap::new();
    let error = plan_spirv_source_execution(SpirvSourceExecutionRequest {
        backend: GpuBackend::Vulkan,
        probe: BackendProbe::prototype(GpuBackend::Vulkan),
        request: ArtifactDispatchRequest {
            fp: FpPolicy::Fast,
            require_bounded_global_u32_array: false,
            require_async_completion: true,
        },
        words: &[],
        entry_name: "unused",
        tool: std::path::Path::new("__jadren_missing_tool__"),
        spec_values: &specialization,
        workgroups: [1, 1, 1],
    })
    .unwrap_err();
    assert_eq!(
        error,
        SpirvSourceExecutionPlanError::NativeSpirvTransport {
            backend: GpuBackend::Vulkan
        }
    );
}

#[test]
fn words_validator_rejects_tampering_and_native_transport() {
    let options = jadren_codegen_spirv::SpirvOptions::new([8, 1, 1]).unwrap();
    let words = jadren_codegen_spirv::emit_storage_add("add_u32", options, 1).unwrap();
    let report = translate_spirv_source_report(
        &words,
        "add_u32",
        &fixture_tool(),
        ArtifactSourceBackend::Hlsl,
    )
    .expect("fixture HLSL report should be valid");
    let specialization = BTreeMap::new();
    let plan =
        validate_spirv_source_report_words(&report, &words, GpuBackend::DirectX12, &specialization)
            .expect("exact words should validate");
    assert_eq!(plan.entry_name, "add_u32");
    let mut tampered = words.clone();
    tampered[5] ^= 1;
    assert!(matches!(
        validate_spirv_source_report_words(
            &report,
            &tampered,
            GpuBackend::DirectX12,
            &specialization,
        ),
        Err(SpirvSourceReportWordsError::WordHashMismatch { .. })
    ));
    assert_eq!(
        validate_spirv_source_report_words(&report, &words, GpuBackend::Vulkan, &specialization,),
        Err(SpirvSourceReportWordsError::NativeSpirvTransport {
            backend: GpuBackend::Vulkan
        })
    );
}

#[test]
fn source_execution_parity_requires_shared_geometry_and_distinct_routes() {
    let options = jadren_codegen_spirv::SpirvOptions::new([8, 1, 1]).unwrap();
    let words = jadren_codegen_spirv::emit_storage_add("add_u32", options, 1).unwrap();
    let request = ArtifactDispatchRequest {
        fp: FpPolicy::Fast,
        require_bounded_global_u32_array: false,
        require_async_completion: true,
    };
    let tool = fixture_tool();
    let specialization = BTreeMap::new();
    let dx12 = plan_spirv_source_execution(SpirvSourceExecutionRequest {
        backend: GpuBackend::DirectX12,
        probe: BackendProbe {
            device_available: true,
            shader_translation_available: true,
            ..BackendProbe::prototype(GpuBackend::DirectX12)
        },
        request,
        words: &words,
        entry_name: "add_u32",
        tool: &tool,
        spec_values: &specialization,
        workgroups: [2, 1, 1],
    })
    .expect("DX12 plan should be valid");
    let metal = plan_spirv_source_execution(SpirvSourceExecutionRequest {
        backend: GpuBackend::Metal,
        probe: BackendProbe {
            device_available: true,
            shader_translation_available: true,
            ..BackendProbe::prototype(GpuBackend::Metal)
        },
        request,
        words: &words,
        entry_name: "add_u32",
        tool: &tool,
        spec_values: &specialization,
        workgroups: [2, 1, 1],
    })
    .expect("Metal plan should be valid");
    let parity = compare_spirv_source_execution_plans(&[dx12.clone(), metal.clone()])
        .expect("DX12/Metal metadata parity should pass");
    assert_eq!(
        parity.backends,
        vec![GpuBackend::DirectX12, GpuBackend::Metal]
    );
    assert_eq!(parity.workgroup_size, [8, 1, 1]);
    assert_eq!(parity.workgroups, [2, 1, 1]);
    assert_eq!(parity.invocation_count, 16);
    assert_eq!(parity.source_hashes.len(), 2);
    let mut different_geometry = metal;
    different_geometry.raw_dispatch.workgroups = [3, 1, 1];
    different_geometry.raw_dispatch.invocation_count = 24;
    assert_eq!(
        compare_spirv_source_execution_plans(&[dx12.clone(), different_geometry]),
        Err(SpirvSourceExecutionParityError::DispatchGeometryMismatch)
    );
    assert_eq!(
        compare_spirv_source_execution_plans(&[dx12.clone(), dx12]),
        Err(SpirvSourceExecutionParityError::DuplicateBackend(
            GpuBackend::DirectX12
        ))
    );
}
