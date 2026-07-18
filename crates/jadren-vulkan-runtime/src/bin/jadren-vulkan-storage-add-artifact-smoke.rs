use jadren_vulkan_runtime::run_storage_add_artifact_queue_smoke;

fn main() {
    match run_storage_add_artifact_queue_smoke() {
        Ok(report) => {
            println!(
                "schema={} entry={} words={} word_hash={} resources={} elements={} addend={} output_checksum={} expected_checksum={} first={} last={} untouched_tail={}",
                report.schema,
                report.artifact_entry_name,
                report.artifact_word_count,
                report.artifact_word_hash,
                report.resource_binding_count,
                report.element_count,
                report.addend,
                report.output_checksum,
                report.expected_checksum,
                report.first_output,
                report.last_output,
                report.untouched_tail_count,
            );
            println!(
                "json={}",
                serde_json::to_string(&report).expect("storage-add report is serializable")
            );
        }
        Err(error) => {
            eprintln!("Vulkan storage-add artifact smoke failed: {error}");
            std::process::exit(1);
        }
    }
}
