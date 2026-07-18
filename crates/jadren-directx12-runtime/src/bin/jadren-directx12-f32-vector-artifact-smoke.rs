use jadren_codegen_spirv::F32ArithmeticOp;
use jadren_directx12_runtime::{
    DirectX12F32VectorArtifactExecutionReport, run_f32_vector_binary_artifact_smoke,
};
use jadren_jir::{
    AddressSpace, Block, BlockId, BuiltinOp, Constant, Function, FunctionId, Instruction,
    InstructionKind, Linkage, Module, Parameter, Terminator, Type, TypeId, TypedValue, ValueId,
};
use serde::Serialize;

#[derive(Serialize)]
struct Report {
    #[serde(flatten)]
    add: DirectX12F32VectorArtifactExecutionReport,
    operation_cases: Vec<DirectX12F32VectorArtifactExecutionReport>,
}

fn main() {
    let input_values: Vec<[f32; 4]> = (0..70_u32)
        .map(|index| {
            let base = 7.0_f32 + index as f32 * 3.0_f32;
            [base, base + 1.0, base + 2.0, base + 3.0]
        })
        .collect();
    let operation_cases = [
        (F32ArithmeticOp::Add, 1.0_f32),
        (F32ArithmeticOp::Subtract, 1.0_f32),
        (F32ArithmeticOp::Multiply, 2.0_f32),
    ]
    .into_iter()
    .map(|(operation, operand)| {
        run_f32_vector_binary_artifact_smoke(
            &jir_dynamic_f32x4_module(operation, operand),
            FunctionId::new(0),
            jadren_codegen_spirv::SpirvOptions::new([64, 1, 1])
                .expect("f32x4 smoke workgroup is valid"),
            &input_values,
            operation,
        )
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
            eprintln!("DX12 f32x4 artifact smoke failed: {error}");
            std::process::exit(1);
        }
    }
}

fn jir_dynamic_f32x4_module(operation: F32ArithmeticOp, operand: f32) -> Module {
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
                lanes: 4,
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
            name: format!("global_{}_dynamic_f32x4", operation_name(operation)),
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
                            lanes: 4,
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
                            alignment: 16,
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
                            alignment: 16,
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

const fn operation_name(operation: F32ArithmeticOp) -> &'static str {
    match operation {
        F32ArithmeticOp::Add => "add",
        F32ArithmeticOp::Subtract => "subtract",
        F32ArithmeticOp::Multiply => "multiply",
    }
}
