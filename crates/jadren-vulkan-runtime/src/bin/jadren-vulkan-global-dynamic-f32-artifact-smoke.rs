use jadren_vulkan_runtime::run_global_dynamic_f32_artifact_queue_smoke;

fn main() {
    match run_global_dynamic_f32_artifact_queue_smoke() {
        Ok(report) => {
            println!(
                "{{\"schema\":\"{}\",\"entry\":\"{}\",\"execution_path\":\"{}\",\"words\":{},\"word_hash\":{},\"resources\":{},\"elements\":{},\"capacity\":{},\"dispatch_x\":{},\"first\":{},\"last\":{},\"output_checksum\":{},\"expected_checksum\":{},\"untouched_tail\":{},\"differential\":\"{}\",\"residency\":\"{}\",\"result\":\"pass-vulkan-f32-artifact-differential\"}}",
                report.schema,
                report.entry_name,
                report.execution_path,
                report.spirv_word_count,
                report.spirv_word_hash,
                report.resource_binding_count,
                report.element_count,
                report.capacity,
                report.dispatch_x,
                report.first_output,
                report.last_output,
                report.output_checksum,
                report.expected_checksum,
                report.untouched_tail_elements,
                report.differential_execution,
                report.residency_execution,
            );
        }
        Err(error) => {
            eprintln!("Vulkan f32 artifact smoke failed: {error}");
            std::process::exit(1);
        }
    }
}
