use jadren_codegen_spirv::F32ArithmeticOp;
use jadren_vulkan_runtime::{
    GlobalDynamicF32VectorQueueSmokeReport,
    run_global_dynamic_f32_vector_artifact_queue_with_values_and_operation,
};
use serde::Serialize;

#[derive(Serialize)]
struct Report {
    #[serde(flatten)]
    add: GlobalDynamicF32VectorQueueSmokeReport,
    operation_cases: Vec<GlobalDynamicF32VectorQueueSmokeReport>,
}

fn main() {
    let input_values: Vec<[f32; 4]> = (0..70_u32)
        .map(|index| {
            let base = 7.0_f32 + index as f32 * 3.0;
            [base, base + 1.0, base + 2.0, base + 3.0]
        })
        .collect();
    let operation_cases = [
        F32ArithmeticOp::Add,
        F32ArithmeticOp::Subtract,
        F32ArithmeticOp::Multiply,
    ]
    .into_iter()
    .map(|operation| {
        run_global_dynamic_f32_vector_artifact_queue_with_values_and_operation(
            &input_values,
            operation,
        )
        .map(|(report, _)| report)
    })
    .collect::<Result<Vec<_>, _>>();
    match operation_cases {
        Ok(operation_cases) => {
            let add = operation_cases
                .iter()
                .find(|case| case.operation == "add")
                .cloned()
                .expect("vector operation family contains add");
            println!(
                "{}",
                serde_json::to_string(&Report {
                    add,
                    operation_cases,
                })
                .expect("f32x4 report serializes")
            );
        }
        Err(error) => {
            eprintln!("Vulkan f32x4 artifact smoke failed: {error}");
            std::process::exit(1);
        }
    }
}
