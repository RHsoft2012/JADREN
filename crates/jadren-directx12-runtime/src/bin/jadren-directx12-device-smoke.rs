use jadren_directx12_runtime::run_device_smoke;

fn main() {
    match run_device_smoke() {
        Ok(report) => {
            println!(
                "{}",
                serde_json::to_string(&report).expect("report is serializable")
            );
        }
        Err(error) => {
            eprintln!("DirectX 12 device smoke failed: {error}");
            std::process::exit(1);
        }
    }
}
