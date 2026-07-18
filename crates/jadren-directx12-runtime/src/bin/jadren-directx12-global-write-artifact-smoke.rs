use jadren_codegen_spirv::SpirvOptions;
use jadren_directx12_runtime::run_global_write_artifact_smoke;
use jadren_jir::{
    AddressSpace, Block, BlockId, BuiltinOp, Constant, Function, FunctionId, Instruction,
    InstructionKind, Linkage, Module, Parameter, Terminator, Type, TypeId, TypedValue, ValueId,
};

fn main() {
    let tail = std::env::args()
        .skip(1)
        .any(|argument| argument == "--tail");
    let (length, capacity) = if tail { (70, 128) } else { (64, 64) };
    let module = jir_global_write_module(length);
    match run_global_write_artifact_smoke(
        &module,
        FunctionId::new(0),
        SpirvOptions::new([64, 1, 1]).expect("global-write workgroup is valid"),
        42,
        length,
        capacity,
    ) {
        Ok(report) => println!(
            "{}",
            serde_json::to_string(&report).expect("global-write artifact report is serializable")
        ),
        Err(error) => {
            eprintln!("DX12 global-write artifact smoke failed: {error}");
            std::process::exit(1);
        }
    }
}

fn jir_global_write_module(length: u32) -> Module {
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
            name: "global_write_u32".to_owned(),
            linkage: Linkage::Export,
            parameters: vec![Parameter {
                value: ValueId::new(0),
                ty: TypeId::new(2),
                name: Some("buffer".to_owned()),
            }],
            result: TypeId::new(0),
            blocks: vec![Block {
                id: BlockId::new(0),
                parameters: Vec::new(),
                instructions: vec![
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(1),
                            ty: TypeId::new(1),
                        }),
                        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(2),
                            ty: TypeId::new(1),
                        }),
                        InstructionKind::Constant(Constant::Integer { value: 42 }),
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(3),
                            ty: TypeId::new(1),
                        }),
                        InstructionKind::Constant(Constant::Integer {
                            value: i128::from(length),
                        }),
                    ),
                    instruction(
                        None,
                        InstructionKind::BoundsCheck {
                            index: ValueId::new(1),
                            length: ValueId::new(3),
                        },
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(4),
                            ty: TypeId::new(2),
                        }),
                        InstructionKind::Offset {
                            base: ValueId::new(0),
                            indices: vec![ValueId::new(1)],
                        },
                    ),
                    instruction(
                        None,
                        InstructionKind::Store {
                            pointer: ValueId::new(4),
                            value: ValueId::new(2),
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
