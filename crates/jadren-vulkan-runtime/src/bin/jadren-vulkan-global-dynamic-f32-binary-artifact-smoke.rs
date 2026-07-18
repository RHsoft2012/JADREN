use jadren_codegen_spirv::F32ArithmeticOp;
use jadren_vulkan_runtime::run_global_dynamic_f32_binary_artifact_queue_smoke;

fn main() {
    let operations = [
        F32ArithmeticOp::Add,
        F32ArithmeticOp::Subtract,
        F32ArithmeticOp::Multiply,
    ];
    let mut reports = Vec::with_capacity(operations.len());
    for operation in operations {
        match run_global_dynamic_f32_binary_artifact_queue_smoke(operation) {
            Ok(report) => reports.push(report),
            Err(error) => {
                eprintln!("Vulkan f32 binary artifact smoke failed: {error}");
                std::process::exit(1);
            }
        }
    }
    println!(
        "{}",
        serde_json::to_string(&reports).expect("f32 binary reports serialize")
    );
}
