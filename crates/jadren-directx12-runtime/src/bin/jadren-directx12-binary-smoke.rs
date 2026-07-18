use jadren_directx12_runtime::{
    DirectX12BinarySmokeReport, run_binary_smoke, run_binary_smoke_with_input,
};
use jadren_jir::BinaryOp;
use serde::Serialize;

#[derive(Serialize)]
struct Report {
    schema: &'static str,
    cases: Vec<DirectX12BinarySmokeReport>,
    passed_cases: usize,
    result: &'static str,
}

fn main() {
    let mut input_start = 41_u32;
    let mut input_stride = 1_u32;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == "--input-start" {
            let Some(value) = arguments.next() else {
                eprintln!("error=missing value for --input-start");
                std::process::exit(2);
            };
            input_start = match value.parse() {
                Ok(value) => value,
                Err(error) => {
                    eprintln!("error=invalid --input-start `{value}`: {error}");
                    std::process::exit(2);
                }
            };
        } else if argument == "--input-stride" {
            let Some(value) = arguments.next() else {
                eprintln!("error=missing value for --input-stride");
                std::process::exit(2);
            };
            input_stride = match value.parse() {
                Ok(value) => value,
                Err(error) => {
                    eprintln!("error=invalid --input-stride `{value}`: {error}");
                    std::process::exit(2);
                }
            };
        } else {
            eprintln!("error=unknown argument `{argument}`");
            std::process::exit(2);
        }
    }
    let requests = [
        (BinaryOp::Add, 1),
        (BinaryOp::Subtract, 1),
        (BinaryOp::Multiply, 2),
        (BinaryOp::Divide, 2),
        (BinaryOp::Remainder, 2),
        (BinaryOp::BitAnd, 1),
        (BinaryOp::BitOr, 1),
        (BinaryOp::BitXor, 1),
        (BinaryOp::ShiftLeft, 1),
        (BinaryOp::ShiftRight, 1),
    ];
    let mut cases = Vec::with_capacity(requests.len());
    for (operation, operand) in requests {
        let result = if input_start == 41 && input_stride == 1 {
            run_binary_smoke(operation, operand)
        } else {
            run_binary_smoke_with_input(operation, operand, input_start, input_stride)
        };
        match result {
            Ok(report) => cases.push(report),
            Err(error) => {
                eprintln!("DX12 binary smoke failed: {error}");
                std::process::exit(1);
            }
        }
    }
    let report = Report {
        schema: "jadren-directx12-binary-smoke-0.1",
        passed_cases: cases.len(),
        cases,
        result: "pass-compute-differential",
    };
    println!(
        "{}",
        serde_json::to_string(&report).expect("binary smoke report is serializable")
    );
}
