use jadren_directx12_runtime::run_compute_smoke;

fn main() {
    match run_compute_smoke() {
        Ok(report) => {
            println!(
                "{}",
                serde_json::to_string(&report).expect("report is serializable")
            );
        }
        Err(error) => {
            eprintln!("DirectX 12 compute smoke failed: {error}");
            std::process::exit(1);
        }
    }
}
