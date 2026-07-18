use jadren_vulkan_runtime::run_global_dynamic_u32_multiply_queue_smoke;

fn main() {
    match run_global_dynamic_u32_multiply_queue_smoke() {
        Ok(report) => {
            println!(
                "schema={} operation={} devices={} device={} queue_family={} queue={} descriptor={} resources={} pipeline={} fence={} data={} differential={} residency={} elements={} capacity={} dispatch_x={} runtime_length={} input_checksum={} output_checksum={} expected_checksum={} first={} last={} untouched_tail={}",
                report.schema,
                report.operation,
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
                report.capacity,
                report.dispatch_x,
                report.runtime_length,
                report.input_checksum,
                report.output_checksum,
                report.expected_checksum,
                report.first_output,
                report.last_output,
                report.untouched_tail_elements,
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
