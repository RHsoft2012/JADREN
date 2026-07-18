use std::collections::BTreeMap;
use std::fs;
use std::process::Command;

use inkwell::context::Context;
use jadren_jir::{
    AddressSpace, BinaryOp, Block, BlockId, BlockParameter, ComparePredicate, Constant, Function,
    FunctionId, Instruction, InstructionKind, Linkage, Module, Parameter, Terminator, Type, TypeId,
    TypedValue, UnaryOp, ValueId, verify,
};

use crate::{
    ObjectOptimization, ObjectOptions, TypeLoweringConfig, WindowsLinkOptions,
    link_windows_executable, lower_to_object, write_object,
};

#[derive(Clone, Debug, Eq, PartialEq)]
enum ReferenceValue {
    Bool(bool),
    Integer(u128),
    Pointer(usize),
    Aggregate(Vec<Self>),
}

struct ReferenceExecutor<'module> {
    module: &'module Module,
    remaining_steps: usize,
}

impl<'module> ReferenceExecutor<'module> {
    fn new(module: &'module Module) -> Result<Self, String> {
        let errors = verify(module);
        if !errors.is_empty() {
            return Err(format!(
                "JIR verifier rejected differential input: {errors:?}"
            ));
        }
        Ok(Self {
            module,
            remaining_steps: 10_000,
        })
    }

    fn execute_entry(mut self) -> Result<i32, String> {
        let entry = self
            .module
            .functions
            .iter()
            .find(|function| function.name == "jadren_entry" && function.linkage == Linkage::Export)
            .ok_or_else(|| "differential input has no exported `jadren_entry`".to_owned())?;
        if !entry.parameters.is_empty() {
            return Err("differential entry must not accept parameters".to_owned());
        }
        if self.integer_shape(entry.result) != Some((true, 32)) {
            return Err("differential entry must return Int32".to_owned());
        }
        let value = self
            .execute_function(entry.id, Vec::new())?
            .ok_or_else(|| "differential entry returned Unit".to_owned())?;
        let ReferenceValue::Integer(value) = value else {
            return Err("differential entry returned a non-integer value".to_owned());
        };
        let signed = signed_integer(value, 32);
        i32::try_from(signed).map_err(|_| "Int32 reference result overflowed i32".to_owned())
    }

    fn execute_function(
        &mut self,
        function_id: FunctionId,
        arguments: Vec<ReferenceValue>,
    ) -> Result<Option<ReferenceValue>, String> {
        let function = self
            .module
            .functions
            .get(function_id.index())
            .ok_or_else(|| format!("missing function @f{}", function_id.index()))?;
        if function.linkage == Linkage::Import {
            return Err(format!(
                "reference executor cannot call imported function `{}`",
                function.name
            ));
        }
        if function.parameters.len() != arguments.len() {
            return Err(format!("argument count mismatch for `{}`", function.name));
        }

        let mut values = BTreeMap::new();
        let mut value_types = BTreeMap::new();
        for (parameter, argument) in function.parameters.iter().zip(arguments) {
            values.insert(parameter.value, argument);
            value_types.insert(parameter.value, parameter.ty);
        }
        let mut memory: Vec<Option<ReferenceValue>> = Vec::new();
        let mut current = BlockId::new(0);
        let mut incoming = Vec::new();

        loop {
            self.consume_step()?;
            let block = function
                .blocks
                .get(current.index())
                .ok_or_else(|| format!("missing block ^bb{}", current.index()))?;
            if block.parameters.len() != incoming.len() {
                return Err(format!(
                    "incoming value mismatch for ^bb{}",
                    current.index()
                ));
            }
            for (parameter, value) in block.parameters.iter().zip(incoming.drain(..)) {
                values.insert(parameter.value, value);
                value_types.insert(parameter.value, parameter.ty);
            }

            for instruction in &block.instructions {
                self.consume_step()?;
                let result =
                    self.execute_instruction(instruction, &values, &value_types, &mut memory)?;
                match (instruction.result, result) {
                    (Some(result), Some(value)) => {
                        values.insert(result.value, value);
                        value_types.insert(result.value, result.ty);
                    }
                    (None, None) => {}
                    _ => return Err("reference instruction result contract mismatch".to_owned()),
                }
            }

            match &block.terminator {
                Terminator::Return { value } => {
                    return value
                        .map(|value| required_value(&values, value))
                        .transpose();
                }
                Terminator::Jump { target, arguments } => {
                    incoming = collect_values(&values, arguments)?;
                    current = *target;
                }
                Terminator::Branch {
                    condition,
                    then_target,
                    then_arguments,
                    else_target,
                    else_arguments,
                } => {
                    let ReferenceValue::Bool(condition) = required_value(&values, *condition)?
                    else {
                        return Err("branch condition is not Bool".to_owned());
                    };
                    if condition {
                        incoming = collect_values(&values, then_arguments)?;
                        current = *then_target;
                    } else {
                        incoming = collect_values(&values, else_arguments)?;
                        current = *else_target;
                    }
                }
                Terminator::Switch {
                    discriminant,
                    cases,
                    default,
                    default_arguments,
                } => {
                    let ReferenceValue::Integer(discriminant_value) =
                        required_value(&values, *discriminant)?
                    else {
                        return Err("switch discriminant is not integer".to_owned());
                    };
                    let ty = required_type(&value_types, *discriminant)?;
                    let (_, bits) = self
                        .integer_shape(ty)
                        .ok_or_else(|| "switch discriminant type is not integer".to_owned())?;
                    if let Some(case) = cases.iter().find(|case| {
                        normalize_integer(case.value as u128, bits) == discriminant_value
                    }) {
                        incoming = collect_values(&values, &case.arguments)?;
                        current = case.target;
                    } else {
                        incoming = collect_values(&values, default_arguments)?;
                        current = *default;
                    }
                }
                Terminator::Unreachable => {
                    return Err("reference execution reached unreachable".to_owned());
                }
            }
        }
    }

    fn execute_instruction(
        &mut self,
        instruction: &Instruction,
        values: &BTreeMap<ValueId, ReferenceValue>,
        value_types: &BTreeMap<ValueId, TypeId>,
        memory: &mut Vec<Option<ReferenceValue>>,
    ) -> Result<Option<ReferenceValue>, String> {
        let result_ty = instruction.result.map(|result| result.ty);
        let value = match &instruction.kind {
            InstructionKind::Constant(constant) => Some(self.constant(
                constant,
                result_ty.ok_or_else(|| "constant has no result type".to_owned())?,
            )?),
            InstructionKind::Unary { op, operand } => Some(self.unary(
                *op,
                required_value(values, *operand)?,
                required_type(value_types, *operand)?,
            )?),
            InstructionKind::Binary { op, left, right } => Some(self.binary(
                *op,
                required_value(values, *left)?,
                required_value(values, *right)?,
                required_type(value_types, *left)?,
            )?),
            InstructionKind::Compare {
                predicate,
                left,
                right,
            } => Some(ReferenceValue::Bool(self.compare(
                *predicate,
                required_value(values, *left)?,
                required_value(values, *right)?,
                required_type(value_types, *left)?,
            )?)),
            InstructionKind::Select {
                condition,
                when_true,
                when_false,
            } => {
                let ReferenceValue::Bool(condition) = required_value(values, *condition)? else {
                    return Err("select condition is not Bool".to_owned());
                };
                Some(required_value(
                    values,
                    if condition { *when_true } else { *when_false },
                )?)
            }
            InstructionKind::StackAlloc { count: None, .. } => {
                let pointer = memory.len();
                memory.push(None);
                Some(ReferenceValue::Pointer(pointer))
            }
            InstructionKind::Store { pointer, value, .. } => {
                let ReferenceValue::Pointer(pointer) = required_value(values, *pointer)? else {
                    return Err("store destination is not a pointer".to_owned());
                };
                let slot = memory
                    .get_mut(pointer)
                    .ok_or_else(|| "store pointer is outside reference memory".to_owned())?;
                *slot = Some(required_value(values, *value)?);
                None
            }
            InstructionKind::Load { pointer, .. } => {
                let ReferenceValue::Pointer(pointer) = required_value(values, *pointer)? else {
                    return Err("load source is not a pointer".to_owned());
                };
                Some(
                    memory
                        .get(pointer)
                        .and_then(Clone::clone)
                        .ok_or_else(|| "reference load reads uninitialized memory".to_owned())?,
                )
            }
            InstructionKind::Aggregate { elements } => {
                Some(ReferenceValue::Aggregate(collect_values(values, elements)?))
            }
            InstructionKind::ExtractValue { aggregate, index } => {
                let ReferenceValue::Aggregate(fields) = required_value(values, *aggregate)? else {
                    return Err("extract source is not an aggregate".to_owned());
                };
                Some(
                    fields
                        .get(*index as usize)
                        .cloned()
                        .ok_or_else(|| "aggregate field is out of range".to_owned())?,
                )
            }
            InstructionKind::Call {
                function,
                arguments,
            } => self.execute_function(*function, collect_values(values, arguments)?)?,
            unsupported => {
                return Err(format!(
                    "reference executor does not support instruction {unsupported:?}"
                ));
            }
        };
        Ok(value)
    }

    fn constant(&self, constant: &Constant, ty: TypeId) -> Result<ReferenceValue, String> {
        match (constant, self.module.types.get(ty.index())) {
            (Constant::Bool(value), Some(Type::Bool)) => Ok(ReferenceValue::Bool(*value)),
            (Constant::Integer { value }, Some(Type::Integer { bits, .. })) => Ok(
                ReferenceValue::Integer(normalize_integer(*value as u128, *bits)),
            ),
            (Constant::Zero, Some(Type::Bool)) => Ok(ReferenceValue::Bool(false)),
            (Constant::Zero, Some(Type::Integer { .. })) => Ok(ReferenceValue::Integer(0)),
            _ => Err("unsupported reference constant/type combination".to_owned()),
        }
    }

    fn unary(
        &self,
        op: UnaryOp,
        value: ReferenceValue,
        ty: TypeId,
    ) -> Result<ReferenceValue, String> {
        match (op, value) {
            (UnaryOp::Not, ReferenceValue::Bool(value)) => Ok(ReferenceValue::Bool(!value)),
            (UnaryOp::Negate, ReferenceValue::Integer(value)) => {
                let (_, bits) = self
                    .integer_shape(ty)
                    .ok_or_else(|| "negate operand type is not integer".to_owned())?;
                Ok(ReferenceValue::Integer(normalize_integer(
                    (!value).wrapping_add(1),
                    bits,
                )))
            }
            (UnaryOp::BitNot, ReferenceValue::Integer(value)) => {
                let (_, bits) = self
                    .integer_shape(ty)
                    .ok_or_else(|| "bit-not operand type is not integer".to_owned())?;
                Ok(ReferenceValue::Integer(normalize_integer(!value, bits)))
            }
            _ => Err("unsupported reference unary operation".to_owned()),
        }
    }

    fn binary(
        &self,
        op: BinaryOp,
        left: ReferenceValue,
        right: ReferenceValue,
        ty: TypeId,
    ) -> Result<ReferenceValue, String> {
        let (signed, bits) = self
            .integer_shape(ty)
            .ok_or_else(|| "reference binary operand is not integer".to_owned())?;
        let (ReferenceValue::Integer(left), ReferenceValue::Integer(right)) = (left, right) else {
            return Err("reference binary values are not integers".to_owned());
        };
        let raw = match op {
            BinaryOp::Add => left.wrapping_add(right),
            BinaryOp::Subtract => left.wrapping_sub(right),
            BinaryOp::Multiply => left.wrapping_mul(right),
            BinaryOp::Divide if right == 0 => return Err("reference division by zero".to_owned()),
            BinaryOp::Divide if signed => {
                (signed_integer(left, bits) / signed_integer(right, bits)) as u128
            }
            BinaryOp::Divide => left / right,
            BinaryOp::Remainder if right == 0 => {
                return Err("reference remainder by zero".to_owned());
            }
            BinaryOp::Remainder if signed => {
                (signed_integer(left, bits) % signed_integer(right, bits)) as u128
            }
            BinaryOp::Remainder => left % right,
            BinaryOp::BitAnd => left & right,
            BinaryOp::BitOr => left | right,
            BinaryOp::BitXor => left ^ right,
            BinaryOp::ShiftLeft | BinaryOp::ShiftRight => {
                return Err("reference shift is outside the JAD-613 matrix".to_owned());
            }
        };
        Ok(ReferenceValue::Integer(normalize_integer(raw, bits)))
    }

    fn compare(
        &self,
        predicate: ComparePredicate,
        left: ReferenceValue,
        right: ReferenceValue,
        ty: TypeId,
    ) -> Result<bool, String> {
        match (left, right) {
            (ReferenceValue::Bool(left), ReferenceValue::Bool(right)) => Ok(match predicate {
                ComparePredicate::Equal => left == right,
                ComparePredicate::NotEqual => left != right,
                _ => return Err("ordered Bool comparison is unsupported".to_owned()),
            }),
            (ReferenceValue::Integer(left), ReferenceValue::Integer(right)) => {
                let (signed, bits) = self
                    .integer_shape(ty)
                    .ok_or_else(|| "comparison type is not integer".to_owned())?;
                Ok(match predicate {
                    ComparePredicate::Equal => left == right,
                    ComparePredicate::NotEqual => left != right,
                    ComparePredicate::Less if signed => {
                        signed_integer(left, bits) < signed_integer(right, bits)
                    }
                    ComparePredicate::Less => left < right,
                    ComparePredicate::LessEqual if signed => {
                        signed_integer(left, bits) <= signed_integer(right, bits)
                    }
                    ComparePredicate::LessEqual => left <= right,
                    ComparePredicate::Greater if signed => {
                        signed_integer(left, bits) > signed_integer(right, bits)
                    }
                    ComparePredicate::Greater => left > right,
                    ComparePredicate::GreaterEqual if signed => {
                        signed_integer(left, bits) >= signed_integer(right, bits)
                    }
                    ComparePredicate::GreaterEqual => left >= right,
                })
            }
            _ => Err("reference comparison operands have different kinds".to_owned()),
        }
    }

    fn integer_shape(&self, ty: TypeId) -> Option<(bool, u16)> {
        match self.module.types.get(ty.index()) {
            Some(Type::Integer { signed, bits }) => Some((*signed, *bits)),
            _ => None,
        }
    }

    fn consume_step(&mut self) -> Result<(), String> {
        if self.remaining_steps == 0 {
            Err("reference execution exceeded deterministic step limit".to_owned())
        } else {
            self.remaining_steps -= 1;
            Ok(())
        }
    }
}

fn required_value(
    values: &BTreeMap<ValueId, ReferenceValue>,
    value: ValueId,
) -> Result<ReferenceValue, String> {
    values
        .get(&value)
        .cloned()
        .ok_or_else(|| format!("reference value %v{} is undefined", value.index()))
}

fn required_type(
    value_types: &BTreeMap<ValueId, TypeId>,
    value: ValueId,
) -> Result<TypeId, String> {
    value_types
        .get(&value)
        .copied()
        .ok_or_else(|| format!("reference type for %v{} is undefined", value.index()))
}

fn collect_values(
    values: &BTreeMap<ValueId, ReferenceValue>,
    ids: &[ValueId],
) -> Result<Vec<ReferenceValue>, String> {
    ids.iter().map(|id| required_value(values, *id)).collect()
}

fn normalize_integer(value: u128, bits: u16) -> u128 {
    value & integer_mask(bits)
}

fn integer_mask(bits: u16) -> u128 {
    if bits == 128 {
        u128::MAX
    } else {
        (1_u128 << bits) - 1
    }
}

fn signed_integer(value: u128, bits: u16) -> i128 {
    if bits == 128 {
        value as i128
    } else {
        let mask = integer_mask(bits);
        let sign = 1_u128 << (bits - 1);
        if value & sign == 0 {
            value as i128
        } else {
            (value | !mask) as i128
        }
    }
}

#[test]
fn reference_and_native_debug_release_execution_match() {
    for (name, module) in differential_modules() {
        let reference = ReferenceExecutor::new(&module)
            .and_then(ReferenceExecutor::execute_entry)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert!((0..=255).contains(&reference), "{name}: exit code range");
        for optimization in [ObjectOptimization::Debug, ObjectOptimization::Release] {
            let native = native_exit_code(name, &module, optimization);
            assert_eq!(
                native, reference,
                "{name} differs for {optimization:?} code generation"
            );
        }
    }
}

#[test]
fn reference_executor_rejects_invalid_entry_contract() {
    let mut module = arithmetic_module();
    module.functions[0].parameters.push(Parameter {
        value: ValueId::new(7),
        ty: TypeId::new(0),
        name: Some("unexpected".to_owned()),
    });
    assert!(
        ReferenceExecutor::new(&module)
            .expect("module remains structurally valid")
            .execute_entry()
            .expect_err("entry parameters must be rejected")
            .contains("must not accept parameters")
    );

    let mut module = arithmetic_module();
    module.functions[0].name = "other".to_owned();
    assert!(
        ReferenceExecutor::new(&module)
            .expect("renamed module remains valid")
            .execute_entry()
            .expect_err("missing entry must be rejected")
            .contains("no exported `jadren_entry`")
    );
}

fn native_exit_code(name: &str, module: &Module, optimization: ObjectOptimization) -> i32 {
    let context = Context::create();
    let options = ObjectOptions {
        optimization,
        ..ObjectOptions::default()
    };
    let object = lower_to_object(
        &context,
        module,
        &format!("differential_{name}"),
        &TypeLoweringConfig::default(),
        &options,
    )
    .unwrap_or_else(|error| panic!("{name}: native object lowering failed: {error}"));
    let suffix = match optimization {
        ObjectOptimization::Debug => "debug",
        ObjectOptimization::Release => "release",
    };
    let directory = std::env::temp_dir().join(format!(
        "jadren-differential-{}-{name}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).expect("create differential directory");
    let object_path = directory.join("program.obj");
    let executable_path = directory.join("program.exe");
    write_object(&object_path, &object).expect("write differential object");
    link_windows_executable(
        &executable_path,
        &[object_path],
        &WindowsLinkOptions::default(),
    )
    .unwrap_or_else(|error| panic!("{name}: differential link failed: {error}"));
    let code = Command::new(&executable_path)
        .status()
        .unwrap_or_else(|error| panic!("{name}: differential executable failed: {error}"))
        .code()
        .unwrap_or_else(|| panic!("{name}: differential executable was terminated"));
    let _ = fs::remove_dir_all(directory);
    code
}

fn differential_modules() -> Vec<(&'static str, Module)> {
    vec![
        ("arithmetic", arithmetic_module()),
        ("branch_phi", branch_phi_module()),
        ("call_switch", call_switch_module()),
        ("memory_aggregate", memory_aggregate_module()),
        ("pointer_dereference", pointer_dereference_module()),
        ("signed_division", signed_division_module()),
        ("select_unary", select_unary_module()),
    ]
}

fn scalar_types() -> Vec<Type> {
    vec![
        Type::Integer {
            signed: true,
            bits: 32,
        },
        Type::Bool,
        Type::Pointer {
            pointee: TypeId::new(0),
            address_space: AddressSpace::Stack,
        },
        Type::Struct {
            fields: vec![TypeId::new(0), TypeId::new(0)],
        },
        Type::Pointer {
            pointee: TypeId::new(2),
            address_space: AddressSpace::Stack,
        },
    ]
}

fn entry_function(blocks: Vec<Block>) -> Function {
    Function {
        id: FunctionId::new(0),
        name: "jadren_entry".to_owned(),
        linkage: Linkage::Export,
        parameters: Vec::new(),
        result: TypeId::new(0),
        blocks,
        span: None,
    }
}

fn value_instruction(value: usize, ty: usize, kind: InstructionKind) -> Instruction {
    Instruction {
        result: Some(TypedValue {
            value: ValueId::new(value),
            ty: TypeId::new(ty),
        }),
        kind,
        span: None,
    }
}

fn integer(value: usize, literal: i128) -> Instruction {
    value_instruction(
        value,
        0,
        InstructionKind::Constant(Constant::Integer { value: literal }),
    )
}

fn arithmetic_module() -> Module {
    Module {
        types: scalar_types(),
        functions: vec![entry_function(vec![Block {
            id: BlockId::new(0),
            parameters: Vec::new(),
            instructions: vec![
                integer(0, 20),
                integer(1, 4),
                value_instruction(
                    2,
                    0,
                    InstructionKind::Binary {
                        op: BinaryOp::Add,
                        left: ValueId::new(0),
                        right: ValueId::new(1),
                    },
                ),
                integer(3, 2),
                value_instruction(
                    4,
                    0,
                    InstructionKind::Binary {
                        op: BinaryOp::Multiply,
                        left: ValueId::new(2),
                        right: ValueId::new(3),
                    },
                ),
                integer(5, 6),
                value_instruction(
                    6,
                    0,
                    InstructionKind::Binary {
                        op: BinaryOp::Subtract,
                        left: ValueId::new(4),
                        right: ValueId::new(5),
                    },
                ),
            ],
            terminator: Terminator::Return {
                value: Some(ValueId::new(6)),
            },
            span: None,
        }])],
    }
}

fn branch_phi_module() -> Module {
    Module {
        types: scalar_types(),
        functions: vec![entry_function(vec![
            Block {
                id: BlockId::new(0),
                parameters: Vec::new(),
                instructions: vec![
                    integer(0, -5),
                    integer(1, 2),
                    value_instruction(
                        2,
                        1,
                        InstructionKind::Compare {
                            predicate: ComparePredicate::Less,
                            left: ValueId::new(0),
                            right: ValueId::new(1),
                        },
                    ),
                ],
                terminator: Terminator::Branch {
                    condition: ValueId::new(2),
                    then_target: BlockId::new(1),
                    then_arguments: Vec::new(),
                    else_target: BlockId::new(2),
                    else_arguments: Vec::new(),
                },
                span: None,
            },
            Block {
                id: BlockId::new(1),
                parameters: Vec::new(),
                instructions: vec![integer(3, 42)],
                terminator: Terminator::Jump {
                    target: BlockId::new(3),
                    arguments: vec![ValueId::new(3)],
                },
                span: None,
            },
            Block {
                id: BlockId::new(2),
                parameters: Vec::new(),
                instructions: vec![integer(4, 1)],
                terminator: Terminator::Jump {
                    target: BlockId::new(3),
                    arguments: vec![ValueId::new(4)],
                },
                span: None,
            },
            Block {
                id: BlockId::new(3),
                parameters: vec![BlockParameter {
                    value: ValueId::new(5),
                    ty: TypeId::new(0),
                }],
                instructions: Vec::new(),
                terminator: Terminator::Return {
                    value: Some(ValueId::new(5)),
                },
                span: None,
            },
        ])],
    }
}

fn call_switch_module() -> Module {
    let choose = Function {
        id: FunctionId::new(0),
        name: "choose".to_owned(),
        linkage: Linkage::Internal,
        parameters: vec![Parameter {
            value: ValueId::new(0),
            ty: TypeId::new(0),
            name: Some("selector".to_owned()),
        }],
        result: TypeId::new(0),
        blocks: vec![
            Block {
                id: BlockId::new(0),
                parameters: Vec::new(),
                instructions: Vec::new(),
                terminator: Terminator::Switch {
                    discriminant: ValueId::new(0),
                    cases: vec![jadren_jir::SwitchCase {
                        value: 7,
                        target: BlockId::new(1),
                        arguments: Vec::new(),
                    }],
                    default: BlockId::new(2),
                    default_arguments: Vec::new(),
                },
                span: None,
            },
            Block {
                id: BlockId::new(1),
                parameters: Vec::new(),
                instructions: vec![integer(1, 42)],
                terminator: Terminator::Return {
                    value: Some(ValueId::new(1)),
                },
                span: None,
            },
            Block {
                id: BlockId::new(2),
                parameters: Vec::new(),
                instructions: vec![integer(2, 1)],
                terminator: Terminator::Return {
                    value: Some(ValueId::new(2)),
                },
                span: None,
            },
        ],
        span: None,
    };
    let mut entry = entry_function(vec![Block {
        id: BlockId::new(0),
        parameters: Vec::new(),
        instructions: vec![
            integer(0, 7),
            value_instruction(
                1,
                0,
                InstructionKind::Call {
                    function: FunctionId::new(0),
                    arguments: vec![ValueId::new(0)],
                },
            ),
        ],
        terminator: Terminator::Return {
            value: Some(ValueId::new(1)),
        },
        span: None,
    }]);
    entry.id = FunctionId::new(1);
    Module {
        types: scalar_types(),
        functions: vec![choose, entry],
    }
}

fn memory_aggregate_module() -> Module {
    Module {
        types: scalar_types(),
        functions: vec![entry_function(vec![Block {
            id: BlockId::new(0),
            parameters: Vec::new(),
            instructions: vec![
                value_instruction(
                    0,
                    2,
                    InstructionKind::StackAlloc {
                        ty: TypeId::new(0),
                        count: None,
                    },
                ),
                integer(1, 40),
                Instruction {
                    result: None,
                    kind: InstructionKind::Store {
                        pointer: ValueId::new(0),
                        value: ValueId::new(1),
                        alignment: 1,
                        volatile: false,
                    },
                    span: None,
                },
                value_instruction(
                    2,
                    0,
                    InstructionKind::Load {
                        pointer: ValueId::new(0),
                        alignment: 1,
                        volatile: false,
                    },
                ),
                integer(3, 2),
                value_instruction(
                    4,
                    3,
                    InstructionKind::Aggregate {
                        elements: vec![ValueId::new(2), ValueId::new(3)],
                    },
                ),
                value_instruction(
                    5,
                    0,
                    InstructionKind::ExtractValue {
                        aggregate: ValueId::new(4),
                        index: 0,
                    },
                ),
                value_instruction(
                    6,
                    0,
                    InstructionKind::ExtractValue {
                        aggregate: ValueId::new(4),
                        index: 1,
                    },
                ),
                value_instruction(
                    7,
                    0,
                    InstructionKind::Binary {
                        op: BinaryOp::Add,
                        left: ValueId::new(5),
                        right: ValueId::new(6),
                    },
                ),
            ],
            terminator: Terminator::Return {
                value: Some(ValueId::new(7)),
            },
            span: None,
        }])],
    }
}

fn pointer_dereference_module() -> Module {
    Module {
        types: scalar_types(),
        functions: vec![entry_function(vec![Block {
            id: BlockId::new(0),
            parameters: Vec::new(),
            instructions: vec![
                value_instruction(
                    0,
                    2,
                    InstructionKind::StackAlloc {
                        ty: TypeId::new(0),
                        count: None,
                    },
                ),
                integer(1, 42),
                Instruction {
                    result: None,
                    kind: InstructionKind::Store {
                        pointer: ValueId::new(0),
                        value: ValueId::new(1),
                        alignment: 4,
                        volatile: false,
                    },
                    span: None,
                },
                value_instruction(
                    2,
                    4,
                    InstructionKind::StackAlloc {
                        ty: TypeId::new(2),
                        count: None,
                    },
                ),
                Instruction {
                    result: None,
                    kind: InstructionKind::Store {
                        pointer: ValueId::new(2),
                        value: ValueId::new(0),
                        alignment: 8,
                        volatile: false,
                    },
                    span: None,
                },
                value_instruction(
                    3,
                    2,
                    InstructionKind::Load {
                        pointer: ValueId::new(2),
                        alignment: 8,
                        volatile: false,
                    },
                ),
                value_instruction(
                    4,
                    0,
                    InstructionKind::Load {
                        pointer: ValueId::new(3),
                        alignment: 4,
                        volatile: false,
                    },
                ),
            ],
            terminator: Terminator::Return {
                value: Some(ValueId::new(4)),
            },
            span: None,
        }])],
    }
}

fn signed_division_module() -> Module {
    Module {
        types: scalar_types(),
        functions: vec![entry_function(vec![Block {
            id: BlockId::new(0),
            parameters: Vec::new(),
            instructions: vec![
                integer(0, -84),
                integer(1, -2),
                value_instruction(
                    2,
                    0,
                    InstructionKind::Binary {
                        op: BinaryOp::Divide,
                        left: ValueId::new(0),
                        right: ValueId::new(1),
                    },
                ),
            ],
            terminator: Terminator::Return {
                value: Some(ValueId::new(2)),
            },
            span: None,
        }])],
    }
}

fn select_unary_module() -> Module {
    Module {
        types: scalar_types(),
        functions: vec![entry_function(vec![Block {
            id: BlockId::new(0),
            parameters: Vec::new(),
            instructions: vec![
                value_instruction(0, 1, InstructionKind::Constant(Constant::Bool(false))),
                value_instruction(
                    1,
                    1,
                    InstructionKind::Unary {
                        op: UnaryOp::Not,
                        operand: ValueId::new(0),
                    },
                ),
                integer(2, 42),
                integer(3, 1),
                value_instruction(
                    4,
                    0,
                    InstructionKind::Select {
                        condition: ValueId::new(1),
                        when_true: ValueId::new(2),
                        when_false: ValueId::new(3),
                    },
                ),
            ],
            terminator: Terminator::Return {
                value: Some(ValueId::new(4)),
            },
            span: None,
        }])],
    }
}
