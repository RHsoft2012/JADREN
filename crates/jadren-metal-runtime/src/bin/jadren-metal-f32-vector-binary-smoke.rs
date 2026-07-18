use jadren_metal_runtime::{
    MetalError, MetalF32VectorBinaryExecutionReport, run_f32_vector_binary_artifact_smoke,
};
use serde::Serialize;

const LENGTH: usize = 70;

#[derive(Serialize)]
struct SkipReport {
    schema: &'static str,
    metal_framework: &'static str,
    case_count: u32,
    cases: Vec<serde_json::Value>,
    result: &'static str,
    error: String,
}

fn main() {
    let input_values: Vec<[f32; 4]> = (0..LENGTH)
        .map(|index| {
            let base = 7.0 + index as f32 * 3.0;
            [base, base + 1.0, base + 2.0, base + 3.0]
        })
        .collect();
    match run_f32_vector_binary_artifact_smoke(&input_values) {
        Ok(report) => print_report(report),
        Err(MetalError::MacOsRequired) => {
            let report = SkipReport {
                schema: "jadren-metal-f32x4-binary-source-execution-0.1",
                metal_framework: "not-run-macos-required",
                case_count: 0,
                cases: Vec::new(),
                result: "skip-macos-required",
                error: MetalError::MacOsRequired.to_string(),
            };
            println!(
                "{}",
                serde_json::to_string(&report).expect("Metal vector binary skip is serializable")
            );
        }
        Err(error) => {
            eprintln!("Metal f32x4 binary smoke failed: {error}");
            std::process::exit(1);
        }
    }
}

fn print_report(report: MetalF32VectorBinaryExecutionReport) {
    println!(
        "{}",
        serde_json::to_string(&report).expect("Metal vector binary report is serializable")
    );
}
