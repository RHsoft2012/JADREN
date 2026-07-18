use jadren_vulkan_runtime::run_timeline_smoke;

fn main() {
    match run_timeline_smoke() {
        Ok(report) => match serde_json::to_string_pretty(&report) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("failed to serialize Vulkan timeline report: {error}");
                std::process::exit(1);
            }
        },
        Err(error) => {
            eprintln!("Vulkan timeline smoke failed: {error}");
            std::process::exit(1);
        }
    }
}
