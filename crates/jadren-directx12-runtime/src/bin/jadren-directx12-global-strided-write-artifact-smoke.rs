use jadren_codegen_spirv::SpirvOptions;
use jadren_directx12_runtime::{
    GlobalStridedWriteArtifactConfig, run_global_strided_write_artifact_smoke,
};
use jadren_jir::{
    AddressSpace, BinaryOp, Block, BlockId, BuiltinOp, Constant, Function, FunctionId, Instruction,
    InstructionKind, Linkage, Module, Parameter, Terminator, Type, TypeId, TypedValue, ValueId,
};

const LENGTH: u32 = 70;
const STRIDE: u32 = 2;
const CAPACITY: u32 = 160;
const VALUE: u32 = 42;

fn main() {
    let module = global_strided_write_module();
    match run_global_strided_write_artifact_smoke(
        &module,
        FunctionId::new(0),
        SpirvOptions::new([64, 1, 1]).expect("strided smoke workgroup is valid"),
        GlobalStridedWriteArtifactConfig {
            value: VALUE,
            length: LENGTH,
            stride: STRIDE,
            capacity: CAPACITY,
        },
    ) {
        Ok(report) => println!(
            "{}",
            serde_json::to_string(&report).expect("strided DX12 artifact report is serializable")
        ),
        Err(error) => {
            eprintln!("DX12 global-strided artifact smoke failed: {error}");
            std::process::exit(1);
        }
    }
}

fn global_strided_write_module() -> Module {
    let instruction = |result, kind| Instruction {
        result,
        kind,
        span: None,
    };
    Module {
        types: vec![
            Type::Unit,
            Type::Integer {
                signed: false,
                bits: 32,
            },
            Type::Pointer {
                pointee: TypeId::new(1),
                address_space: AddressSpace::Storage,
            },
        ],
        functions: vec![Function {
            id: FunctionId::new(0),
            name: "global_strided_write_u32".to_owned(),
            linkage: Linkage::Export,
            parameters: vec![
                parameter(0, "buffer"),
                parameter(1, "length"),
                parameter(2, "stride"),
                parameter(3, "capacity"),
            ],
            result: TypeId::new(0),
            blocks: vec![Block {
                id: BlockId::new(0),
                parameters: Vec::new(),
                instructions: vec![
                    instruction(
                        result(4, 1),
                        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                    ),
                    instruction(
                        result(5, 1),
                        InstructionKind::Constant(Constant::Integer {
                            value: i128::from(VALUE),
                        }),
                    ),
                    instruction(result(6, 1), load(1)),
                    instruction(result(7, 1), load(2)),
                    instruction(result(8, 1), load(3)),
                    instruction(
                        None,
                        InstructionKind::BoundsCheck {
                            index: ValueId::new(4),
                            length: ValueId::new(6),
                        },
                    ),
                    instruction(
                        result(9, 1),
                        InstructionKind::Binary {
                            op: BinaryOp::Multiply,
                            left: ValueId::new(4),
                            right: ValueId::new(7),
                        },
                    ),
                    instruction(
                        None,
                        InstructionKind::BoundsCheck {
                            index: ValueId::new(9),
                            length: ValueId::new(8),
                        },
                    ),
                    instruction(
                        result(10, 2),
                        InstructionKind::Offset {
                            base: ValueId::new(0),
                            indices: vec![ValueId::new(9)],
                        },
                    ),
                    instruction(
                        None,
                        InstructionKind::Store {
                            pointer: ValueId::new(10),
                            value: ValueId::new(5),
                            alignment: 4,
                            volatile: false,
                        },
                    ),
                ],
                terminator: Terminator::Return { value: None },
                span: None,
            }],
            span: None,
        }],
    }
}

fn parameter(value: usize, name: &str) -> Parameter {
    Parameter {
        value: ValueId::new(value),
        ty: TypeId::new(2),
        name: Some(name.to_owned()),
    }
}

fn result(value: usize, ty: usize) -> Option<TypedValue> {
    Some(TypedValue {
        value: ValueId::new(value),
        ty: TypeId::new(ty),
    })
}

fn load(pointer: usize) -> InstructionKind {
    InstructionKind::Load {
        pointer: ValueId::new(pointer),
        alignment: 4,
        volatile: false,
    }
}
