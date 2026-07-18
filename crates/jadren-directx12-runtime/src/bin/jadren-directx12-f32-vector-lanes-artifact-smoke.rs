use jadren_codegen_spirv::F32ArithmeticOp;
use jadren_directx12_runtime::{
    DirectX12F32VectorLanesArtifactExecutionReport, run_f32_vector_lanes_binary_artifact_smoke,
};
use jadren_jir::{
    AddressSpace, Block, BlockId, BuiltinOp, Constant, Function, FunctionId, Instruction,
    InstructionKind, Linkage, Module, Parameter, Terminator, Type, TypeId, TypedValue, ValueId,
};
use serde::Serialize;

#[derive(Serialize)]
struct Report {
    lane_cases: Vec<DirectX12F32VectorLanesArtifactExecutionReport>,
}

fn main() {
    let cases = [2_u16, 3]
        .into_iter()
        .flat_map(|lanes| {
            [
                F32ArithmeticOp::Add,
                F32ArithmeticOp::Subtract,
                F32ArithmeticOp::Multiply,
            ]
            .into_iter()
            .map(move |operation| (lanes, operation))
        })
        .map(|(lanes, operation)| {
            let input_values: Vec<Vec<f32>> = (0..70_u32)
                .map(|index| {
                    let base = 7.0_f32 + index as f32 * 3.0;
                    (0..usize::from(lanes))
                        .map(|lane| base + lane as f32)
                        .collect()
                })
                .collect();
            run_f32_vector_lanes_binary_artifact_smoke(
                &jir_dynamic_f32_vector_module(operation, f32_operand(operation), lanes),
                FunctionId::new(0),
                jadren_codegen_spirv::SpirvOptions::new([64, 1, 1])
                    .expect("vector lane workgroup is valid"),
                &input_values,
                operation,
            )
        })
        .collect::<Result<Vec<_>, _>>();
    match cases {
        Ok(lane_cases) => println!(
            "{}",
            serde_json::to_string(&Report { lane_cases }).expect("vector lane report serializes")
        ),
        Err(error) => {
            eprintln!("DX12 vector lane artifact smoke failed: {error}");
            std::process::exit(1);
        }
    }
}

fn jir_dynamic_f32_vector_module(operation: F32ArithmeticOp, operand: f32, lanes: u16) -> Module {
    Module {
        types: vec![
            Type::Unit,
            Type::Integer {
                signed: false,
                bits: 32,
            },
            Type::Float { bits: 32 },
            Type::Vector {
                element: TypeId::new(2),
                lanes,
            },
            Type::Pointer {
                pointee: TypeId::new(3),
                address_space: AddressSpace::Storage,
            },
            Type::Pointer {
                pointee: TypeId::new(1),
                address_space: AddressSpace::Storage,
            },
        ],
        functions: vec![Function {
            id: FunctionId::new(0),
            name: format!("global_{}_dynamic_f32x{lanes}", operation_name(operation)),
            linkage: Linkage::Export,
            parameters: vec![
                Parameter {
                    value: ValueId::new(0),
                    ty: TypeId::new(4),
                    name: Some("input".to_owned()),
                },
                Parameter {
                    value: ValueId::new(1),
                    ty: TypeId::new(4),
                    name: Some("output".to_owned()),
                },
                Parameter {
                    value: ValueId::new(2),
                    ty: TypeId::new(5),
                    name: Some("length".to_owned()),
                },
            ],
            result: TypeId::new(0),
            blocks: vec![Block {
                id: BlockId::new(0),
                parameters: Vec::new(),
                instructions: vec![
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(3),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(4),
                            ty: TypeId::new(2),
                        }),
                        kind: InstructionKind::Constant(Constant::FloatBits {
                            bits: u64::from(operand.to_bits()),
                        }),
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(5),
                            ty: TypeId::new(3),
                        }),
                        kind: InstructionKind::VectorSplat {
                            value: ValueId::new(4),
                            lanes,
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(6),
                            ty: TypeId::new(1),
                        }),
                        kind: InstructionKind::Load {
                            pointer: ValueId::new(2),
                            alignment: 4,
                            volatile: false,
                        },
                        span: None,
                    },
                    Instruction {
                        result: None,
                        kind: InstructionKind::BoundsCheck {
                            index: ValueId::new(3),
                            length: ValueId::new(6),
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(7),
                            ty: TypeId::new(4),
                        }),
                        kind: InstructionKind::Offset {
                            base: ValueId::new(0),
                            indices: vec![ValueId::new(3)],
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(8),
                            ty: TypeId::new(3),
                        }),
                        kind: InstructionKind::Load {
                            pointer: ValueId::new(7),
                            alignment: if lanes == 4 { 16 } else { 4 },
                            volatile: false,
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(9),
                            ty: TypeId::new(3),
                        }),
                        kind: InstructionKind::VectorBinary {
                            op: operation.as_binary_op(),
                            left: ValueId::new(8),
                            right: ValueId::new(5),
                        },
                        span: None,
                    },
                    Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(10),
                            ty: TypeId::new(4),
                        }),
                        kind: InstructionKind::Offset {
                            base: ValueId::new(1),
                            indices: vec![ValueId::new(3)],
                        },
                        span: None,
                    },
                    Instruction {
                        result: None,
                        kind: InstructionKind::Store {
                            pointer: ValueId::new(10),
                            value: ValueId::new(9),
                            alignment: if lanes == 4 { 16 } else { 4 },
                            volatile: false,
                        },
                        span: None,
                    },
                ],
                terminator: Terminator::Return { value: None },
                span: None,
            }],
            span: None,
        }],
    }
}

const fn f32_operand(operation: F32ArithmeticOp) -> f32 {
    match operation {
        F32ArithmeticOp::Add | F32ArithmeticOp::Subtract => 1.0,
        F32ArithmeticOp::Multiply => 2.0,
    }
}

const fn operation_name(operation: F32ArithmeticOp) -> &'static str {
    match operation {
        F32ArithmeticOp::Add => "add",
        F32ArithmeticOp::Subtract => "subtract",
        F32ArithmeticOp::Multiply => "multiply",
    }
}
