use jadren_vulkan_runtime::run_f32_queue_smoke;

fn main() {
    match run_f32_queue_smoke() {
        Ok(report) => match serde_json::to_string_pretty(&report) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("failed to serialize Vulkan f32 smoke report: {error}");
                std::process::exit(1);
            }
        },
        Err(error) => {
            eprintln!("Vulkan f32 queue smoke failed: {error}");
            std::process::exit(1);
        }
    }
}
