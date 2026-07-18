use jadren_vulkan_runtime::run_global_3d_write_u32_queue_smoke;

fn main() {
    match run_global_3d_write_u32_queue_smoke() {
        Ok(report) => {
            println!(
                "{}",
                serde_json::to_string(&report).expect("report is serializable")
            );
        }
        Err(error) => {
            eprintln!("jadren Vulkan global 3D write smoke failed: {error}");
            std::process::exit(1);
        }
    }
}
