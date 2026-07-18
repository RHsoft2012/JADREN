use jadren_codegen_spirv::{
    F32ArithmeticOp, ResourceAccess, SpirvError, SpirvOptions,
    apply_spirv_resource_access_decorations, emit_storage_global_2d_strided_write,
    emit_storage_global_2d_write, emit_storage_global_3d_strided_write,
    emit_storage_global_3d_write, emit_storage_global_index_binary_dynamic_length_from_jir,
    emit_storage_global_index_f32_binary_dynamic_length, emit_storage_global_index_strided_write,
    emit_storage_global_index_vector_f32_binary_dynamic_length_lanes,
    emit_storage_global_index_write,
};
use jadren_jir::{
    AddressSpace, BinaryOp, Block, BlockId, BuiltinOp, Constant, Function, FunctionId, Instruction,
    InstructionKind, Linkage, Module, Parameter, Terminator, Type, TypeId, TypedValue, ValueId,
};

const DYNAMIC_WORKGROUP: [u32; 3] = [64, 1, 1];

/// Ordered candidate inventory for the next exact-word manifest revision.
pub const JADREN_GPU_SUPPORTED_SUBSET_EXPANSION_CASE_IDS: [&str; 28] = [
    "u32.add",
    "u32.subtract",
    "u32.multiply",
    "u32.divide",
    "u32.remainder",
    "u32.bitand",
    "u32.bitor",
    "u32.bitxor",
    "u32.shift-left",
    "u32.shift-right",
    "f32.add",
    "f32.subtract",
    "f32.multiply",
    "f32x2.add",
    "f32x2.subtract",
    "f32x2.multiply",
    "f32x3.add",
    "f32x3.subtract",
    "f32x3.multiply",
    "f32x4.add",
    "f32x4.subtract",
    "f32x4.multiply",
    "u32.write.1d",
    "u32.write.1d-strided",
    "u32.write.2d",
    "u32.write.2d-strided",
    "u32.write.3d",
    "u32.write.3d-strided",
];

/// Returns the canonical compute entry carried by one expansion case.
#[must_use]
pub fn gpu_supported_subset_case_entry_name(case_id: &str) -> Option<&'static str> {
    if let Some((_, _, entry_name)) = u32_case(case_id) {
        return Some(entry_name);
    }
    if let Some((_, _, entry_name)) = f32_case(case_id) {
        return Some(entry_name);
    }
    Some(match case_id {
        "u32.write.1d" => "global_write_u32",
        "u32.write.1d-strided" => "global_strided_write_u32",
        "u32.write.2d" => "global_2d_write_u32",
        "u32.write.2d-strided" => "global_2d_strided_write_u32",
        "u32.write.3d" => "global_3d_write_u32",
        "u32.write.3d-strided" => "global_3d_strided_write_u32",
        _ => return None,
    })
}

/// Emits the canonical SPIR-V words for one named supported-subset case.
///
/// This function is the single source used by DX12 and Metal conformance
/// runners. Unknown IDs are rejected instead of silently selecting a related
/// kernel, and every write-only family receives exact access decorations.
pub fn emit_gpu_supported_subset_case_words(case_id: &str) -> Result<Vec<u32>, SpirvError> {
    if let Some((operation, operand, entry_name)) = u32_case(case_id) {
        let module = dynamic_u32_module(entry_name, operation, operand);
        return emit_storage_global_index_binary_dynamic_length_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new(DYNAMIC_WORKGROUP)?,
            operation,
        );
    }
    if let Some((lanes, operation, entry_name)) = f32_case(case_id) {
        let operand_bits = match operation {
            F32ArithmeticOp::Add | F32ArithmeticOp::Subtract => 1.0_f32.to_bits(),
            F32ArithmeticOp::Multiply => 2.0_f32.to_bits(),
        };
        let options = SpirvOptions::new(DYNAMIC_WORKGROUP)?;
        return if lanes == 1 {
            emit_storage_global_index_f32_binary_dynamic_length(
                entry_name,
                options,
                operand_bits,
                operation,
            )
        } else {
            emit_storage_global_index_vector_f32_binary_dynamic_length_lanes(
                entry_name,
                options,
                operand_bits,
                operation,
                lanes,
            )
        };
    }
    write_case(case_id).ok_or(SpirvError::UnsupportedKernelShape(
        "unknown GPU supported-subset case ID",
    ))?
}

fn u32_case(case_id: &str) -> Option<(BinaryOp, i128, &'static str)> {
    Some(match case_id {
        "u32.add" => (BinaryOp::Add, 1, "global_add_dynamic_u32"),
        "u32.subtract" => (BinaryOp::Subtract, 1, "global_subtract_dynamic_u32"),
        "u32.multiply" => (BinaryOp::Multiply, 2, "global_multiply_dynamic_u32"),
        "u32.divide" => (BinaryOp::Divide, 2, "global_divide_dynamic_u32"),
        "u32.remainder" => (BinaryOp::Remainder, 2, "global_remainder_dynamic_u32"),
        "u32.bitand" => (BinaryOp::BitAnd, 1, "global_bitand_dynamic_u32"),
        "u32.bitor" => (BinaryOp::BitOr, 1, "global_bitor_dynamic_u32"),
        "u32.bitxor" => (BinaryOp::BitXor, 1, "global_bitxor_dynamic_u32"),
        "u32.shift-left" => (BinaryOp::ShiftLeft, 1, "global_shift_left_dynamic_u32"),
        "u32.shift-right" => (BinaryOp::ShiftRight, 1, "global_shift_right_dynamic_u32"),
        _ => return None,
    })
}

fn f32_case(case_id: &str) -> Option<(u32, F32ArithmeticOp, &'static str)> {
    let (shape, operation) = case_id.split_once('.')?;
    let lanes = match shape {
        "f32" => 1,
        "f32x2" => 2,
        "f32x3" => 3,
        "f32x4" => 4,
        _ => return None,
    };
    let operation = match operation {
        "add" => F32ArithmeticOp::Add,
        "subtract" => F32ArithmeticOp::Subtract,
        "multiply" => F32ArithmeticOp::Multiply,
        _ => return None,
    };
    let operation_name = match operation {
        F32ArithmeticOp::Add => "add",
        F32ArithmeticOp::Subtract => "subtract",
        F32ArithmeticOp::Multiply => "multiply",
    };
    let entry_name = match (lanes, operation_name) {
        (1, "add") => "global_add_dynamic_f32",
        (1, "subtract") => "global_subtract_dynamic_f32",
        (1, "multiply") => "global_multiply_dynamic_f32",
        (2, "add") => "global_add_dynamic_f32x2",
        (2, "subtract") => "global_subtract_dynamic_f32x2",
        (2, "multiply") => "global_multiply_dynamic_f32x2",
        (3, "add") => "global_add_dynamic_f32x3",
        (3, "subtract") => "global_subtract_dynamic_f32x3",
        (3, "multiply") => "global_multiply_dynamic_f32x3",
        (4, "add") => "global_add_dynamic_f32x4",
        (4, "subtract") => "global_subtract_dynamic_f32x4",
        (4, "multiply") => "global_multiply_dynamic_f32x4",
        _ => return None,
    };
    Some((lanes, operation, entry_name))
}

fn write_case(case_id: &str) -> Option<Result<Vec<u32>, SpirvError>> {
    let (words, resource_count) = match case_id {
        "u32.write.1d" => (
            emit_storage_global_index_write(
                "global_write_u32",
                SpirvOptions::new([64, 1, 1]).ok()?,
                42,
                64,
            ),
            1,
        ),
        "u32.write.1d-strided" => (
            emit_storage_global_index_strided_write(
                "global_strided_write_u32",
                SpirvOptions::new([64, 1, 1]).ok()?,
                42,
            ),
            4,
        ),
        "u32.write.2d" => (
            emit_storage_global_2d_write(
                "global_2d_write_u32",
                SpirvOptions::new([8, 8, 1]).ok()?,
                42,
            ),
            4,
        ),
        "u32.write.2d-strided" => (
            emit_storage_global_2d_strided_write(
                "global_2d_strided_write_u32",
                SpirvOptions::new([4, 4, 1]).ok()?,
                42,
            ),
            6,
        ),
        "u32.write.3d" => (
            emit_storage_global_3d_write(
                "global_3d_write_u32",
                SpirvOptions::new([4, 4, 2]).ok()?,
                42,
            ),
            5,
        ),
        "u32.write.3d-strided" => (
            emit_storage_global_3d_strided_write(
                "global_3d_strided_write_u32",
                SpirvOptions::new([4, 4, 2]).ok()?,
                42,
            ),
            8,
        ),
        _ => return None,
    };
    Some(words.and_then(|mut words| {
        let access = (0..resource_count)
            .map(|binding| {
                (
                    binding,
                    if binding == 0 {
                        ResourceAccess::WriteOnly
                    } else {
                        ResourceAccess::ReadOnly
                    },
                )
            })
            .collect::<Vec<_>>();
        apply_spirv_resource_access_decorations(&mut words, &access)?;
        Ok(words)
    }))
}

fn dynamic_u32_module(name: &str, operation: BinaryOp, operand: i128) -> Module {
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
            name: name.to_owned(),
            linkage: Linkage::Export,
            parameters: vec![
                Parameter {
                    value: ValueId::new(0),
                    ty: TypeId::new(2),
                    name: Some("input".to_owned()),
                },
                Parameter {
                    value: ValueId::new(1),
                    ty: TypeId::new(2),
                    name: Some("output".to_owned()),
                },
                Parameter {
                    value: ValueId::new(2),
                    ty: TypeId::new(2),
                    name: Some("length".to_owned()),
                },
            ],
            result: TypeId::new(0),
            blocks: vec![Block {
                id: BlockId::new(0),
                parameters: Vec::new(),
                instructions: vec![
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(3),
                            ty: TypeId::new(1),
                        }),
                        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(4),
                            ty: TypeId::new(1),
                        }),
                        InstructionKind::Constant(Constant::Integer { value: operand }),
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(5),
                            ty: TypeId::new(1),
                        }),
                        InstructionKind::Load {
                            pointer: ValueId::new(2),
                            alignment: 4,
                            volatile: false,
                        },
                    ),
                    instruction(
                        None,
                        InstructionKind::BoundsCheck {
                            index: ValueId::new(3),
                            length: ValueId::new(5),
                        },
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(6),
                            ty: TypeId::new(2),
                        }),
                        InstructionKind::Offset {
                            base: ValueId::new(0),
                            indices: vec![ValueId::new(3)],
                        },
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(7),
                            ty: TypeId::new(1),
                        }),
                        InstructionKind::Load {
                            pointer: ValueId::new(6),
                            alignment: 4,
                            volatile: false,
                        },
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(8),
                            ty: TypeId::new(1),
                        }),
                        InstructionKind::Binary {
                            op: operation,
                            left: ValueId::new(7),
                            right: ValueId::new(4),
                        },
                    ),
                    instruction(
                        Some(TypedValue {
                            value: ValueId::new(9),
                            ty: TypeId::new(2),
                        }),
                        InstructionKind::Offset {
                            base: ValueId::new(1),
                            indices: vec![ValueId::new(3)],
                        },
                    ),
                    instruction(
                        None,
                        InstructionKind::Store {
                            pointer: ValueId::new(9),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        GpuSupportedSubsetAdmissionError, JADREN_GPU_SUPPORTED_SUBSET_V0_3,
        admit_gpu_supported_subset_v0_3, inspect_spirv_source_module, stable_spirv_word_hash,
    };

    #[test]
    fn expansion_inventory_emits_valid_exact_resource_contracts() {
        assert_eq!(JADREN_GPU_SUPPORTED_SUBSET_V0_3.case_count, 28);
        for (case_id, expected) in JADREN_GPU_SUPPORTED_SUBSET_EXPANSION_CASE_IDS
            .into_iter()
            .zip(JADREN_GPU_SUPPORTED_SUBSET_V0_3.cases)
        {
            assert_eq!(case_id, expected.id);
            let entry_name = gpu_supported_subset_case_entry_name(case_id).unwrap();
            let words = emit_gpu_supported_subset_case_words(case_id).unwrap();
            let contract = inspect_spirv_source_module(&words, entry_name).unwrap();
            assert_eq!(contract.word_count, words.len());
            assert_eq!(contract.word_hash, stable_spirv_word_hash(&words));
            assert_eq!(contract.word_count, expected.word_count);
            assert_eq!(contract.word_hash, expected.word_hash);
            assert_eq!(contract.workgroup_size, Some(expected.workgroup_size));
            assert_eq!(contract.resources.len(), expected.resources.len());
            assert!(
                contract
                    .resources
                    .iter()
                    .any(|resource| { resource.access.is_some_and(ResourceAccess::can_write) })
            );
            assert_eq!(
                admit_gpu_supported_subset_v0_3(case_id, &words, entry_name).unwrap(),
                *expected
            );
        }
    }

    #[test]
    fn supported_subset_v0_3_rejects_variable_resource_and_identity_drift() {
        let case_id = "u32.write.3d-strided";
        let entry_name = gpu_supported_subset_case_entry_name(case_id).unwrap();
        let words = emit_gpu_supported_subset_case_words(case_id).unwrap();
        assert!(matches!(
            admit_gpu_supported_subset_v0_3("unknown", &words, entry_name),
            Err(GpuSupportedSubsetAdmissionError::UnknownCase(_))
        ));
        assert!(matches!(
            admit_gpu_supported_subset_v0_3(case_id, &words, "other_entry"),
            Err(GpuSupportedSubsetAdmissionError::EntryMismatch { .. })
        ));

        let mut wrong_workgroup = words.clone();
        let execution_mode = find_instruction(&wrong_workgroup, 16, |operands| {
            operands.len() == 5 && operands[1] == 17
        });
        wrong_workgroup[execution_mode + 3] = 8;
        assert!(matches!(
            admit_gpu_supported_subset_v0_3(case_id, &wrong_workgroup, entry_name),
            Err(GpuSupportedSubsetAdmissionError::WorkgroupMismatch { .. })
        ));

        let mut wrong_access = words.clone();
        let non_writable = find_instruction(&wrong_access, 71, |operands| {
            operands.len() == 2 && operands[1] == 24
        });
        wrong_access[non_writable + 2] = 25;
        assert!(matches!(
            admit_gpu_supported_subset_v0_3(case_id, &wrong_access, entry_name),
            Err(GpuSupportedSubsetAdmissionError::ResourceContractMismatch)
        ));

        let write_case_id = "u32.write.1d";
        let write_entry = gpu_supported_subset_case_entry_name(write_case_id).unwrap();
        let mut wrong_hash = emit_gpu_supported_subset_case_words(write_case_id).unwrap();
        let value_constant = find_instruction(&wrong_hash, 43, |operands| {
            operands.len() == 3 && operands[2] == 42
        });
        wrong_hash[value_constant + 3] = 43;
        assert!(matches!(
            admit_gpu_supported_subset_v0_3(write_case_id, &wrong_hash, write_entry),
            Err(GpuSupportedSubsetAdmissionError::WordHashMismatch { .. })
        ));

        let mut truncated = words;
        truncated.pop();
        assert!(matches!(
            admit_gpu_supported_subset_v0_3(case_id, &truncated, entry_name),
            Err(GpuSupportedSubsetAdmissionError::Source(_))
                | Err(GpuSupportedSubsetAdmissionError::WordCountMismatch { .. })
        ));
    }

    fn find_instruction(
        words: &[u32],
        expected_opcode: u16,
        predicate: impl Fn(&[u32]) -> bool,
    ) -> usize {
        let mut index = 5;
        while index < words.len() {
            let word_count = (words[index] >> 16) as usize;
            let opcode = (words[index] & 0xffff) as u16;
            assert!(word_count > 0 && index + word_count <= words.len());
            if opcode == expected_opcode && predicate(&words[index + 1..index + word_count]) {
                return index;
            }
            index += word_count;
        }
        panic!("SPIR-V instruction {expected_opcode} was not found")
    }
}
