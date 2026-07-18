use std::time::Instant;

use jadren_vulkan_runtime::run_global_dynamic_u32_queue_smoke;
use serde::Serialize;

#[derive(Serialize)]
struct BenchmarkReport {
    schema: &'static str,
    kind: &'static str,
    warmup_iterations: usize,
    iterations: usize,
    samples_us: Vec<u128>,
    median_us: u128,
    p95_us: u128,
    physical_device_count: usize,
    selected_device: String,
    runtime_length: u32,
    capacity: usize,
    dispatch_x: u32,
    output_checksum: u64,
    expected_checksum: u64,
    differential_execution: &'static str,
}

fn parse_count(args: &[String], flag: &str, default: usize) -> Result<usize, String> {
    let Some(index) = args.iter().position(|arg| arg == flag) else {
        return Ok(default);
    };
    let value = args
        .get(index + 1)
        .ok_or_else(|| format!("missing value for {flag}"))?;
    let count = value
        .parse::<usize>()
        .map_err(|error| format!("invalid {flag} value `{value}`: {error}"))?;
    if count == 0 {
        return Err(format!("{flag} must be greater than zero"));
    }
    Ok(count)
}

fn run() -> Result<BenchmarkReport, String> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let warmup_iterations = parse_count(&args, "--warmup", 2)?;
    let iterations = parse_count(&args, "--iterations", 5)?;
    let mut last_report = None;
    for _ in 0..warmup_iterations {
        let report = run_global_dynamic_u32_queue_smoke().map_err(|error| error.to_string())?;
        validate_report(&report)?;
        last_report = Some(report);
    }
    let mut samples_us = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        let report = run_global_dynamic_u32_queue_smoke().map_err(|error| error.to_string())?;
        let elapsed = start.elapsed().as_micros();
        validate_report(&report)?;
        last_report = Some(report);
        samples_us.push(elapsed);
    }
    samples_us.sort_unstable();
    let report = last_report.ok_or_else(|| "benchmark produced no report".to_owned())?;
    let median_us = samples_us[samples_us.len() / 2];
    let p95_index = ((samples_us.len() * 95).saturating_sub(1) / 100).min(samples_us.len() - 1);
    let p95_us = samples_us[p95_index];
    Ok(BenchmarkReport {
        schema: "jadren-vulkan-benchmark-0.1",
        kind: "native-global-dynamic-u32-lifecycle",
        warmup_iterations,
        iterations,
        samples_us,
        median_us,
        p95_us,
        physical_device_count: report.physical_device_count,
        selected_device: report.selected_device,
        runtime_length: report.runtime_length,
        capacity: report.capacity,
        dispatch_x: report.dispatch_x,
        output_checksum: report.output_checksum,
        expected_checksum: report.expected_checksum,
        differential_execution: "passed",
    })
}

fn validate_report(
    report: &jadren_vulkan_runtime::GlobalDynamicU32QueueSmokeReport,
) -> Result<(), String> {
    if report.differential_execution != "passed"
        || report.output_checksum != report.expected_checksum
        || report.runtime_length != 70
        || report.capacity != 128
        || report.dispatch_x != 2
    {
        return Err("benchmark kernel differential contract failed".to_owned());
    }
    Ok(())
}

fn main() {
    match run() {
        Ok(report) => println!(
            "{}",
            serde_json::to_string(&report).expect("benchmark report serializes")
        ),
        Err(error) => {
            eprintln!("Vulkan benchmark failed: {error}");
            std::process::exit(1);
        }
    }
}
