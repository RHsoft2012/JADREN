use jadren_metal_runtime::{MetalError, run_f32_binary_artifact_smoke};

const LENGTH: usize = 70;

fn main() {
    let input_values: Vec<f32> = (0..LENGTH).map(|index| 7.0 + index as f32 * 3.0).collect();
    match run_f32_binary_artifact_smoke(&input_values) {
        Ok(report) => println!(
            "{}",
            serde_json::to_string(&report).expect("Metal scalar report is serializable")
        ),
        Err(MetalError::MacOsRequired) => println!(
            "{}",
            serde_json::json!({
                "schema": "jadren-metal-f32-binary-source-execution-0.1",
                "metal_framework": "not-run-macos-required",
                "case_count": 0,
                "cases": [],
                "result": "skip-macos-required",
                "error": MetalError::MacOsRequired.to_string(),
            })
        ),
        Err(error) => {
            eprintln!("Metal scalar f32 smoke failed: {error}");
            std::process::exit(1);
        }
    }
}
