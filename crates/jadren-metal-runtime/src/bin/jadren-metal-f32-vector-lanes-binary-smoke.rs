use jadren_metal_runtime::{
    MetalError, MetalF32VectorLanesBinaryExecutionCase, run_f32_vector_lanes_binary_artifact_smoke,
};
use serde::Serialize;

const LENGTH: usize = 70;

#[derive(Serialize)]
struct SuiteReport {
    schema: &'static str,
    metal_framework: &'static str,
    case_count: u32,
    lane_counts: Vec<u32>,
    lane_cases: Vec<MetalF32VectorLanesBinaryExecutionCase>,
    result: &'static str,
}

#[derive(Serialize)]
struct SkipReport {
    schema: &'static str,
    metal_framework: &'static str,
    case_count: u32,
    lane_counts: Vec<u32>,
    lane_cases: Vec<MetalF32VectorLanesBinaryExecutionCase>,
    result: &'static str,
    error: String,
}

fn main() {
    let mut lane_cases = Vec::new();
    for lane_count in [2_usize, 3] {
        let input_values = (0..LENGTH)
            .map(|index| {
                let base = 7.0 + index as f32 * 3.0;
                (0..lane_count)
                    .map(|lane| base + lane as f32)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        match run_f32_vector_lanes_binary_artifact_smoke(&input_values) {
            Ok(report) => lane_cases.extend(report.cases),
            Err(MetalError::MacOsRequired) => {
                let report = SkipReport {
                    schema: "jadren-metal-f32-vector-lanes-binary-artifact-suite-0.1",
                    metal_framework: "not-run-macos-required",
                    case_count: 0,
                    lane_counts: Vec::new(),
                    lane_cases: Vec::new(),
                    result: "skip-macos-required",
                    error: MetalError::MacOsRequired.to_string(),
                };
                println!(
                    "{}",
                    serde_json::to_string(&report).expect("Metal vector lane skip is serializable")
                );
                return;
            }
            Err(error) => {
                eprintln!("Metal f32 vector lane binary smoke failed: {error}");
                std::process::exit(1);
            }
        }
    }

    let report = SuiteReport {
        schema: "jadren-metal-f32-vector-lanes-binary-artifact-suite-0.1",
        metal_framework: "loaded",
        case_count: lane_cases.len() as u32,
        lane_counts: vec![2, 3],
        lane_cases,
        result: "pass-metal-f32-vector-lanes-binary-artifact-differential",
    };
    println!(
        "{}",
        serde_json::to_string(&report).expect("Metal vector lane report is serializable")
    );
}
