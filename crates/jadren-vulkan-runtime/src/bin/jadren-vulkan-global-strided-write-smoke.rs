use jadren_vulkan_runtime::run_global_strided_write_u32_queue_smoke;

fn main() {
    match run_global_strided_write_u32_queue_smoke() {
        Ok(report) => {
            println!(
                "schema={} devices={} device={} queue_family={} queue={} descriptor={} resources={} pipeline={} fence={} data={} differential={} residency={} logical_length={} capacity={} stride={} dispatch_x={} last_physical_index={} output_checksum={} expected_checksum={} untouched_elements={}",
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
                report.logical_length,
                report.capacity,
                report.stride,
                report.dispatch_x,
                report.last_physical_index,
                report.output_checksum,
                report.expected_checksum,
                report.untouched_elements,
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
