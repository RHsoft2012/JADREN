use jadren_metal_runtime::{MetalError, run_mixed_srv_uav_artifact_smoke};

fn main() {
    match run_mixed_srv_uav_artifact_smoke() {
        Ok(report) => println!(
            "{}",
            serde_json::to_string(&report).expect("Metal mixed report is serializable")
        ),
        Err(MetalError::MacOsRequired) => println!(
            "{}",
            serde_json::json!({
                "schema": "jadren-metal-mixed-srv-uav-source-execution-0.1",
                "metal_framework": "not-run-macos-required",
                "view_contract": "passed",
                "result": "skip-macos-required",
                "error": MetalError::MacOsRequired.to_string(),
            })
        ),
        Err(error) => {
            eprintln!("Metal mixed SRV/UAV smoke failed: {error}");
            std::process::exit(1);
        }
    }
}
