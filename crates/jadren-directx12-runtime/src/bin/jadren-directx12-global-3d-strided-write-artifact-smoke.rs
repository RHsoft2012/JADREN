use jadren_codegen_spirv::SpirvOptions;
use jadren_directx12_runtime::{
    Global3dStridedWriteArtifactConfig, run_global_3d_strided_write_artifact_smoke,
};
use jadren_jir::{
    AddressSpace, BinaryOp, Block, BlockId, BuiltinOp, Constant, Function, FunctionId, Instruction,
    InstructionKind, Linkage, Module, Parameter, Terminator, Type, TypeId, TypedValue, ValueId,
};

const WIDTH: u32 = 4;
const HEIGHT: u32 = 3;
const DEPTH: u32 = 2;
const STRIDE_X: u32 = 2;
const STRIDE_Y: u32 = 11;
const STRIDE_Z: u32 = 37;
const CAPACITY: u32 = 72;
const VALUE: u32 = 42;

fn main() {
    let module = global_3d_strided_write_module();
    match run_global_3d_strided_write_artifact_smoke(
        &module,
        FunctionId::new(0),
        SpirvOptions::new([4, 4, 2]).expect("3D strided smoke workgroup is valid"),
        Global3dStridedWriteArtifactConfig {
            value: VALUE,
            width: WIDTH,
            height: HEIGHT,
            depth: DEPTH,
            stride_x: STRIDE_X,
            stride_y: STRIDE_Y,
            stride_z: STRIDE_Z,
            capacity: CAPACITY,
        },
    ) {
        Ok(report) => println!(
            "{}",
            serde_json::to_string(&report)
                .expect("3D strided DX12 artifact report is serializable")
        ),
        Err(error) => {
            eprintln!("DX12 global-3d-strided artifact smoke failed: {error}");
            std::process::exit(1);
        }
    }
}

fn global_3d_strided_write_module() -> Module {
    let instruction = |result, kind| Instruction {
        result,
        kind,
        span: None,
    };
    let result = |value| {
        Some(TypedValue {
            value: ValueId::new(value),
            ty: TypeId::new(1),
        })
    };
    let load = |value, pointer| {
        instruction(
            result(value),
            InstructionKind::Load {
                pointer: ValueId::new(pointer),
                alignment: 4,
                volatile: false,
            },
        )
    };
    let bounds = |index, length| {
        instruction(
            None,
            InstructionKind::BoundsCheck {
                index: ValueId::new(index),
                length: ValueId::new(length),
            },
        )
    };
    let binary = |value, op, left, right| {
        instruction(
            result(value),
            InstructionKind::Binary {
                op,
                left: ValueId::new(left),
                right: ValueId::new(right),
            },
        )
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
            name: "global_3d_strided_write_u32".to_owned(),
            linkage: Linkage::Export,
            parameters: [
                "buffer", "width", "height", "depth", "stride_x", "stride_y", "stride_z",
                "capacity",
            ]
            .into_iter()
            .enumerate()
            .map(|(index, name)| Parameter {
                value: ValueId::new(index),
                ty: TypeId::new(2),
                name: Some(name.to_owned()),
            })
            .collect(),
            result: TypeId::new(0),
            blocks: vec![Block {
                id: BlockId::new(0),
                parameters: Vec::new(),
                instructions: vec![
                    instruction(
                        result(8),
                        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                    ),
                    instruction(
                        result(9),
                        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdY),
                    ),
                    instruction(
                        result(10),
                        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdZ),
                    ),
                    instruction(
                        result(11),
                        InstructionKind::Constant(Constant::Integer {
                            value: i128::from(VALUE),
                        }),
                    ),
                    load(12, 1),
                    load(13, 2),
                    load(14, 3),
                    load(15, 4),
                    load(16, 5),
                    load(17, 6),
                    load(18, 7),
                    bounds(8, 12),
                    bounds(9, 13),
                    bounds(10, 14),
                    binary(19, BinaryOp::Multiply, 8, 15),
                    binary(20, BinaryOp::Multiply, 9, 16),
                    binary(21, BinaryOp::Add, 19, 20),
                    binary(22, BinaryOp::Multiply, 10, 17),
                    binary(23, BinaryOp::Add, 21, 22),
                    bounds(23, 18),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(24),
                            ty: TypeId::new(2),
                        }),
                        InstructionKind::Offset {
                            base: ValueId::new(0),
                            indices: vec![ValueId::new(23)],
                        },
                    ),
                    instruction(
                        None,
                        InstructionKind::Store {
                            pointer: ValueId::new(24),
                            value: ValueId::new(11),
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
