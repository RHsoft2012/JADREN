use jadren_metal_runtime::{
    MetalError, MetalF32VectorArtifactExecutionReport, run_f32_vector_artifact_smoke,
};
use serde::Serialize;

const LENGTH: usize = 70;

#[derive(Serialize)]
struct SkipReport {
    schema: &'static str,
    entry_name: &'static str,
    metal_framework: &'static str,
    resource_binding_count: u32,
    logical_length: u32,
    capacity: u32,
    first_output: [f32; 4],
    last_output: [f32; 4],
    input_checksum: f64,
    output_checksum: f64,
    untouched_tail_count: u32,
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
    error: String,
}

fn main() {
    let input_values: Vec<[f32; 4]> = (0..LENGTH)
        .map(|index| {
            let base = 7.0 + index as f32 * 3.0;
            [base, base + 1.0, base + 2.0, base + 3.0]
        })
        .collect();
    match run_f32_vector_artifact_smoke(&input_values) {
        Ok(report) => print_report(report),
        Err(MetalError::MacOsRequired) => {
            let report = SkipReport {
                schema: "jadren-metal-f32x4-source-execution-0.1",
                entry_name: "global_add_dynamic_f32x4",
                metal_framework: "not-run-macos-required",
                resource_binding_count: 3,
                logical_length: LENGTH as u32,
                capacity: 128,
                first_output: [0.0; 4],
                last_output: [0.0; 4],
                input_checksum: 0.0,
                output_checksum: 0.0,
                untouched_tail_count: 0,
                source_contract_validated: false,
                pipeline_created: false,
                command_queue_created: false,
                command_buffer_created: false,
                command_buffer_committed: false,
                command_buffer_completed: false,
                command_buffer_status: None,
                execution_path: "not-run-macos-required",
                execution_completed: false,
                result: "skip-macos-required",
                error: MetalError::MacOsRequired.to_string(),
            };
            println!(
                "{}",
                serde_json::to_string(&report).expect("Metal vector skip is serializable")
            );
        }
        Err(error) => {
            eprintln!("Metal f32x4 smoke failed: {error}");
            std::process::exit(1);
        }
    }
}

fn print_report(report: MetalF32VectorArtifactExecutionReport) {
    println!(
        "{}",
        serde_json::to_string(&report).expect("Metal vector report is serializable")
    );
}
