use jadren_metal_runtime::{MetalDeviceSmokeReport, MetalError, run_device_smoke};
use serde::Serialize;

#[derive(Serialize)]
struct SkipReport {
    schema: &'static str,
    metal_framework: &'static str,
    device_created: bool,
    command_queue_created: bool,
    command_buffer_created: bool,
    command_buffer_committed: bool,
    command_buffer_completed: bool,
    command_buffer_status: Option<i64>,
    result: &'static str,
    error: String,
}

fn main() {
    match run_device_smoke() {
        Ok(report) => print_report(report),
        Err(MetalError::MacOsRequired) => {
            let report = SkipReport {
                schema: "jadren-metal-device-smoke-0.1",
                metal_framework: "not-run-macos-required",
                device_created: false,
                command_queue_created: false,
                command_buffer_created: false,
                command_buffer_committed: false,
                command_buffer_completed: false,
                command_buffer_status: None,
                result: "skip-macos-required",
                error: MetalError::MacOsRequired.to_string(),
            };
            println!(
                "{}",
                serde_json::to_string(&report).expect("skip report is serializable")
            );
        }
        Err(error) => {
            eprintln!("Metal device smoke failed: {error}");
            std::process::exit(1);
        }
    }
}

fn print_report(report: MetalDeviceSmokeReport) {
    println!(
        "{}",
        serde_json::to_string(&report).expect("Metal report is serializable")
    );
}
