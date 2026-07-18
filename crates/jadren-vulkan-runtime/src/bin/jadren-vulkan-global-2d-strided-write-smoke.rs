fn main() {
    match jadren_vulkan_runtime::run_global_2d_strided_write_u32_queue_smoke() {
        Ok(report) => {
            println!(
                "{}",
                serde_json::to_string(&report).expect("smoke report must serialize")
            );
        }
        Err(error) => {
            eprintln!("jadren Vulkan global 2D strided write smoke failed: {error}");
            std::process::exit(1);
        }
    }
}
