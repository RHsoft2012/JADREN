use jadren_vulkan_runtime::run_global_dynamic_f32_queue_smoke;

fn main() {
    match run_global_dynamic_f32_queue_smoke() {
        Ok(report) => match serde_json::to_string(&report) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("failed to serialize dynamic f32 Vulkan report: {error}");
                std::process::exit(1);
            }
        },
        Err(error) => {
            eprintln!("dynamic f32 Vulkan smoke failed: {error}");
            std::process::exit(1);
        }
    }
}
