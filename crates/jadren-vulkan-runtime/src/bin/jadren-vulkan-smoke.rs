use jadren_vulkan_runtime::run_queue_smoke;

fn main() {
    match run_queue_smoke() {
        Ok(report) => match serde_json::to_string_pretty(&report) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("failed to serialize Vulkan smoke report: {error}");
                std::process::exit(1);
            }
        },
        Err(error) => {
            eprintln!("Vulkan queue smoke failed: {error}");
            std::process::exit(1);
        }
    }
}
