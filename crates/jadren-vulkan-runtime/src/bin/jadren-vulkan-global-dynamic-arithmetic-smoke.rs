use jadren_jir::BinaryOp;
use jadren_vulkan_runtime::run_global_dynamic_u32_binary_queue_smoke;

fn parse_operation(value: &str) -> Option<BinaryOp> {
    match value {
        "add" => Some(BinaryOp::Add),
        "subtract" => Some(BinaryOp::Subtract),
        "multiply" => Some(BinaryOp::Multiply),
        "divide" => Some(BinaryOp::Divide),
        "remainder" => Some(BinaryOp::Remainder),
        "bitand" => Some(BinaryOp::BitAnd),
        "bitor" => Some(BinaryOp::BitOr),
        "bitxor" => Some(BinaryOp::BitXor),
        "shift-left" => Some(BinaryOp::ShiftLeft),
        "shift-right" => Some(BinaryOp::ShiftRight),
        _ => None,
    }
}

fn main() {
    let mut operation_name = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == "--operation" {
            operation_name = arguments.next();
        } else {
            eprintln!("error=unknown argument `{argument}`");
            std::process::exit(2);
        }
    }
    let Some(operation_name) = operation_name else {
        eprintln!(
            "error=missing --operation (add|subtract|multiply|divide|remainder|bitand|bitor|bitxor|shift-left|shift-right)"
        );
        std::process::exit(2);
    };
    let Some(operation) = parse_operation(&operation_name) else {
        eprintln!("error=unsupported operation `{operation_name}`");
        std::process::exit(2);
    };
    match run_global_dynamic_u32_binary_queue_smoke(operation) {
        Ok(report) => {
            println!(
                "schema={} operation={} devices={} device={} queue_family={} queue={} descriptor={} resources={} pipeline={} fence={} data={} differential={} residency={} elements={} capacity={} dispatch_x={} runtime_length={} input_checksum={} output_checksum={} expected_checksum={} first={} last={} untouched_tail={}",
                report.schema,
                report.operation,
                report.physical_device_count,
                report.selected_device,
                report.queue_family_index,
                report.queue_execution,
                report.descriptor_setup,
                report.resource_binding_count,
                report.pipeline_execution,
                report.fence_execution,
                report.data_kernel_execution,
                report.differential_execution,
                report.residency_execution,
                report.element_count,
                report.capacity,
                report.dispatch_x,
                report.runtime_length,
                report.input_checksum,
                report.output_checksum,
                report.expected_checksum,
                report.first_output,
                report.last_output,
                report.untouched_tail_elements,
            );
            println!(
                "json={}",
                serde_json::to_string(&report).expect("report serializes")
            );
        }
        Err(error) => {
            eprintln!("error={error}");
            std::process::exit(1);
        }
    }
}
