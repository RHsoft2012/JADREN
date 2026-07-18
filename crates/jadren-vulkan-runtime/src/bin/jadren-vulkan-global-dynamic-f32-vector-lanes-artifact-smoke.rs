use jadren_codegen_spirv::F32ArithmeticOp;
use jadren_vulkan_runtime::{
    GlobalDynamicF32VectorLanesQueueSmokeReport,
    run_global_dynamic_f32_vector_lanes_artifact_queue_with_values_and_operation,
};
use serde::Serialize;

#[derive(Serialize)]
struct Report {
    lane_cases: Vec<GlobalDynamicF32VectorLanesQueueSmokeReport>,
}

fn main() {
    let operation_cases = [2_usize, 3]
        .into_iter()
        .flat_map(|lane_count| {
            [
                F32ArithmeticOp::Add,
                F32ArithmeticOp::Subtract,
                F32ArithmeticOp::Multiply,
            ]
            .into_iter()
            .map(move |operation| (lane_count, operation))
        })
        .map(|(lane_count, operation)| {
            let input_values: Vec<Vec<f32>> = (0..70_u32)
                .map(|index| {
                    let base = 7.0_f32 + index as f32 * 3.0;
                    (0..lane_count).map(|lane| base + lane as f32).collect()
                })
                .collect();
            run_global_dynamic_f32_vector_lanes_artifact_queue_with_values_and_operation(
                &input_values,
                operation,
            )
            .map(|(report, _)| report)
        })
        .collect::<Result<Vec<_>, _>>();
    match operation_cases {
        Ok(lane_cases) => println!(
            "{}",
            serde_json::to_string(&Report { lane_cases }).expect("vector lane report serializes")
        ),
        Err(error) => {
            eprintln!("Vulkan vector lane artifact smoke failed: {error}");
            std::process::exit(1);
        }
    }
}
