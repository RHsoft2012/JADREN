use jadren_vulkan_runtime::{
    run_global_write_u32_queue_smoke, run_global_write_u32_tail_queue_smoke,
};

fn main() {
    let tail = std::env::args()
        .skip(1)
        .any(|argument| argument == "--tail");
    let result = if tail {
        run_global_write_u32_tail_queue_smoke()
    } else {
        run_global_write_u32_queue_smoke()
    };
    match result {
        Ok(report) => {
            println!(
                "schema={} devices={} device={} queue_family={} queue={} descriptor={} resources={} pipeline={} fence={} data={} differential={} residency={} elements={} output_checksum={} expected_checksum={} first={} last={}",
                report.schema,
                report.physical_device_count,
                report.selected_device,
                report.queue_family_index,
                report.queue_execution,
                report.descriptor_setup,
                report.resource_binding_count,
                report.pipeline_execution,
                report.fence_execution,
                report.data_kernel_execution,
                report.differential_execution,
                report.residency_execution,
                report.element_count,
                report.output_checksum,
                report.expected_checksum,
                report.first_output,
                report.last_output,
            );
            println!(
                "json={}",
                serde_json::to_string(&report).expect("report serializes")
            );
        }
        Err(error) => {
            eprintln!("error={error}");
            std::process::exit(1);
        }
    }
}
