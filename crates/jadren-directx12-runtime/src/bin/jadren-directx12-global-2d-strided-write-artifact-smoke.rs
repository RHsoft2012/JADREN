use jadren_codegen_spirv::SpirvOptions;
use jadren_directx12_runtime::{
    Global2dStridedWriteArtifactConfig, run_global_2d_strided_write_artifact_smoke,
};
use jadren_jir::{
    AddressSpace, BinaryOp, Block, BlockId, BuiltinOp, Constant, Function, FunctionId, Instruction,
    InstructionKind, Linkage, Module, Parameter, Terminator, Type, TypeId, TypedValue, ValueId,
};

const WIDTH: u32 = 4;
const HEIGHT: u32 = 3;
const STRIDE_X: u32 = 2;
const STRIDE_Y: u32 = 10;
const CAPACITY: u32 = 40;
const VALUE: u32 = 42;

fn main() {
    let module = global_2d_strided_write_module();
    match run_global_2d_strided_write_artifact_smoke(
        &module,
        FunctionId::new(0),
        SpirvOptions::new([4, 4, 1]).expect("2D strided smoke workgroup is valid"),
        Global2dStridedWriteArtifactConfig {
            value: VALUE,
            width: WIDTH,
            height: HEIGHT,
            stride_x: STRIDE_X,
            stride_y: STRIDE_Y,
            capacity: CAPACITY,
        },
    ) {
        Ok(report) => println!(
            "{}",
            serde_json::to_string(&report)
                .expect("2D strided DX12 artifact report is serializable")
        ),
        Err(error) => {
            eprintln!("DX12 global-2d-strided artifact smoke failed: {error}");
            std::process::exit(1);
        }
    }
}

fn global_2d_strided_write_module() -> Module {
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
            name: "global_2d_strided_write_u32".to_owned(),
            linkage: Linkage::Export,
            parameters: vec![
                parameter(0, "buffer"),
                parameter(1, "width"),
                parameter(2, "height"),
                parameter(3, "stride_x"),
                parameter(4, "stride_y"),
                parameter(5, "capacity"),
            ],
            result: TypeId::new(0),
            blocks: vec![Block {
                id: BlockId::new(0),
                parameters: Vec::new(),
                instructions: vec![
                    instruction(
                        result(6, 1),
                        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                    ),
                    instruction(
                        result(7, 1),
                        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdY),
                    ),
                    instruction(
                        result(8, 1),
                        InstructionKind::Constant(Constant::Integer {
                            value: i128::from(VALUE),
                        }),
                    ),
                    instruction(result(9, 1), load(1)),
                    instruction(result(10, 1), load(2)),
                    instruction(result(11, 1), load(3)),
                    instruction(result(12, 1), load(4)),
                    instruction(result(13, 1), load(5)),
                    instruction(
                        None,
                        InstructionKind::BoundsCheck {
                            index: ValueId::new(6),
                            length: ValueId::new(9),
                        },
                    ),
                    instruction(
                        None,
                        InstructionKind::BoundsCheck {
                            index: ValueId::new(7),
                            length: ValueId::new(10),
                        },
                    ),
                    instruction(
                        result(14, 1),
                        InstructionKind::Binary {
                            op: BinaryOp::Multiply,
                            left: ValueId::new(6),
                            right: ValueId::new(11),
                        },
                    ),
                    instruction(
                        result(15, 1),
                        InstructionKind::Binary {
                            op: BinaryOp::Multiply,
                            left: ValueId::new(7),
                            right: ValueId::new(12),
                        },
                    ),
                    instruction(
                        result(16, 1),
                        InstructionKind::Binary {
                            op: BinaryOp::Add,
                            left: ValueId::new(14),
                            right: ValueId::new(15),
                        },
                    ),
                    instruction(
                        None,
                        InstructionKind::BoundsCheck {
                            index: ValueId::new(16),
                            length: ValueId::new(13),
                        },
                    ),
                    instruction(
                        result(17, 2),
                        InstructionKind::Offset {
                            base: ValueId::new(0),
                            indices: vec![ValueId::new(16)],
                        },
                    ),
                    instruction(
                        None,
                        InstructionKind::Store {
                            pointer: ValueId::new(17),
                            value: ValueId::new(8),
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
