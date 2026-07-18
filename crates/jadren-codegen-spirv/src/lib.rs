//! Minimal deterministic SPIR-V compute emitter for the Jadren GPU subset.
//!
//! JAD-1303 provides a validated body-less compute baseline. JAD-1305 adds
//! deliberately narrow `ptr<storage, u32>` storage arithmetic lowering paths;
//! unsupported JIR bodies are rejected rather than silently approximated.

pub use jadren_jir::F32ArithmeticOp;
use jadren_jir::{
    AddressSpace, BinaryOp, BuiltinOp, Constant, Function, FunctionId, Instruction,
    InstructionKind, Module, Terminator, Type, TypeId, ValueId, verify_gpu,
};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

const SPIRV_MAGIC: u32 = 0x0723_0203;
const SPIRV_VERSION_1_3: u32 = 0x0001_0300;
const CAPABILITY_SHADER: u32 = 1;
const ADDRESSING_LOGICAL: u32 = 0;
const MEMORY_MODEL_GLSL450: u32 = 1;
const EXECUTION_MODEL_GL_COMPUTE: u32 = 5;
const EXECUTION_MODE_LOCAL_SIZE: u32 = 17;
const EXECUTION_MODE_LOCAL_SIZE_ID: u32 = 38;
const FUNCTION_CONTROL_NONE: u32 = 0;

const OP_NAME: u16 = 5;
const OP_DECORATE: u16 = 71;
const OP_MEMBER_DECORATE: u16 = 72;
const OP_MEMORY_MODEL: u16 = 14;
const OP_ENTRY_POINT: u16 = 15;
const OP_EXECUTION_MODE: u16 = 16;
const OP_EXECUTION_MODE_ID: u16 = 331;
const OP_CAPABILITY: u16 = 17;
const OP_TYPE_VOID: u16 = 19;
const OP_TYPE_BOOL: u16 = 20;
const OP_TYPE_INT: u16 = 21;
const OP_TYPE_FLOAT: u16 = 22;
const OP_TYPE_VECTOR: u16 = 23;
const OP_TYPE_RUNTIME_ARRAY: u16 = 29;
const OP_TYPE_STRUCT: u16 = 30;
const OP_TYPE_POINTER: u16 = 32;
const OP_TYPE_FUNCTION: u16 = 33;
const OP_CONSTANT: u16 = 43;
const OP_CONSTANT_COMPOSITE: u16 = 44;
const OP_SPEC_CONSTANT: u16 = 50;
const OP_SPEC_CONSTANT_OP: u16 = 52;
const OP_LOAD: u16 = 61;
const OP_ACCESS_CHAIN: u16 = 65;
const OP_COMPOSITE_CONSTRUCT: u16 = 80;
const OP_COMPOSITE_EXTRACT: u16 = 81;
const OP_STORE: u16 = 62;
const OP_IADD: u16 = 128;
const OP_FADD: u16 = 129;
const OP_ISUB: u16 = 130;
const OP_FSUB: u16 = 131;
const OP_IMUL: u16 = 132;
const OP_FMUL: u16 = 133;
const OP_UDIV: u16 = 134;
const OP_UMOD: u16 = 137;
const OP_SHIFT_RIGHT_LOGICAL: u16 = 194;
const OP_SHIFT_LEFT_LOGICAL: u16 = 196;
const OP_BITWISE_OR: u16 = 197;
const OP_BITWISE_XOR: u16 = 198;
const OP_BITWISE_AND: u16 = 199;
const OP_ULT: u16 = 176;
const OP_LOGICAL_AND: u16 = 167;
const OP_FUNCTION: u16 = 54;
const OP_LABEL: u16 = 248;
const OP_SELECTION_MERGE: u16 = 247;
const OP_BRANCH: u16 = 249;
const OP_BRANCH_CONDITIONAL: u16 = 250;
const OP_RETURN: u16 = 253;
const OP_FUNCTION_END: u16 = 56;
const OP_VARIABLE: u16 = 59;

const STORAGE_BUFFER: u32 = 12;
const DECORATION_BLOCK: u32 = 2;
const DECORATION_BINDING: u32 = 33;
const DECORATION_DESCRIPTOR_SET: u32 = 34;
const DECORATION_OFFSET: u32 = 35;
const DECORATION_ARRAY_STRIDE: u32 = 6;
const DECORATION_BUILT_IN: u32 = 11;
const DECORATION_NON_WRITABLE: u32 = 24;
const DECORATION_NON_READABLE: u32 = 25;
const BUILT_IN_GLOBAL_INVOCATION_ID: u32 = 28;
const INPUT: u32 = 1;

/// SPIR-V emission options for one compute entrypoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpirvOptions {
    /// Compile-time local workgroup dimensions.
    pub workgroup_size: [u32; 3],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IntegerArithmeticOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    ShiftRight,
}

impl IntegerArithmeticOp {
    const fn spirv_opcode(self) -> u16 {
        match self {
            Self::Add => OP_IADD,
            Self::Subtract => OP_ISUB,
            Self::Multiply => OP_IMUL,
            Self::Divide => OP_UDIV,
            Self::Remainder => OP_UMOD,
            Self::BitAnd => OP_BITWISE_AND,
            Self::BitOr => OP_BITWISE_OR,
            Self::BitXor => OP_BITWISE_XOR,
            Self::ShiftLeft => OP_SHIFT_LEFT_LOGICAL,
            Self::ShiftRight => OP_SHIFT_RIGHT_LOGICAL,
        }
    }

    const fn jir_op(self) -> BinaryOp {
        match self {
            Self::Add => BinaryOp::Add,
            Self::Subtract => BinaryOp::Subtract,
            Self::Multiply => BinaryOp::Multiply,
            Self::Divide => BinaryOp::Divide,
            Self::Remainder => BinaryOp::Remainder,
            Self::BitAnd => BinaryOp::BitAnd,
            Self::BitOr => BinaryOp::BitOr,
            Self::BitXor => BinaryOp::BitXor,
            Self::ShiftLeft => BinaryOp::ShiftLeft,
            Self::ShiftRight => BinaryOp::ShiftRight,
        }
    }

    const fn from_jir(operation: BinaryOp) -> Option<Self> {
        Some(match operation {
            BinaryOp::Add => Self::Add,
            BinaryOp::Subtract => Self::Subtract,
            BinaryOp::Multiply => Self::Multiply,
            BinaryOp::Divide => Self::Divide,
            BinaryOp::Remainder => Self::Remainder,
            BinaryOp::BitAnd => Self::BitAnd,
            BinaryOp::BitOr => Self::BitOr,
            BinaryOp::BitXor => Self::BitXor,
            BinaryOp::ShiftLeft => Self::ShiftLeft,
            BinaryOp::ShiftRight => Self::ShiftRight,
        })
    }
}

const fn f32_spirv_opcode(operation: F32ArithmeticOp) -> u16 {
    match operation {
        F32ArithmeticOp::Add => OP_FADD,
        F32ArithmeticOp::Subtract => OP_FSUB,
        F32ArithmeticOp::Multiply => OP_FMUL,
    }
}

fn validate_integer_binary_operand(
    operation: IntegerArithmeticOp,
    operand: u32,
) -> Result<(), SpirvError> {
    match operation {
        IntegerArithmeticOp::Divide | IntegerArithmeticOp::Remainder if operand == 0 => {
            Err(SpirvError::UnsupportedKernelShape(
                "dynamic-length unsigned divisor/remainder operand must be non-zero",
            ))
        }
        IntegerArithmeticOp::ShiftLeft | IntegerArithmeticOp::ShiftRight if operand >= 32 => {
            Err(SpirvError::UnsupportedKernelShape(
                "dynamic-length u32 shift operand must be smaller than 32",
            ))
        }
        _ => Ok(()),
    }
}

impl SpirvOptions {
    /// Validates a portable baseline workgroup shape.
    pub fn new(workgroup_size: [u32; 3]) -> Result<Self, SpirvError> {
        if workgroup_size.contains(&0) {
            return Err(SpirvError::InvalidWorkgroupSize(workgroup_size));
        }
        let product = u64::from(workgroup_size[0])
            * u64::from(workgroup_size[1])
            * u64::from(workgroup_size[2]);
        if product > 1024 {
            return Err(SpirvError::WorkgroupTooLarge(product));
        }
        Ok(Self { workgroup_size })
    }
}

/// Errors raised before a SPIR-V module is returned.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SpirvError {
    InvalidWorkgroupSize([u32; 3]),
    WorkgroupTooLarge(u64),
    MissingFunction(FunctionId),
    GpuVerificationFailed(usize),
    UnsupportedKernelShape(&'static str),
    InvalidEntryName,
}

impl fmt::Display for SpirvError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidWorkgroupSize(size) => {
                write!(formatter, "invalid workgroup size {size:?}")
            }
            Self::WorkgroupTooLarge(product) => {
                write!(formatter, "workgroup size product {product} exceeds 1024")
            }
            Self::MissingFunction(function) => {
                write!(
                    formatter,
                    "SPIR-V function {} does not exist",
                    function.index()
                )
            }
            Self::GpuVerificationFailed(count) => {
                write!(
                    formatter,
                    "GPU JIR verification failed with {count} error(s)"
                )
            }
            Self::UnsupportedKernelShape(reason) => {
                write!(
                    formatter,
                    "unsupported minimal SPIR-V kernel shape: {reason}"
                )
            }
            Self::InvalidEntryName => {
                formatter.write_str("SPIR-V entry name must be non-empty and NUL-free")
            }
        }
    }
}

impl Error for SpirvError {}

/// Structural validation failures for the emitted SPIR-V word stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SpirvValidationError {
    HeaderTooShort,
    BadMagic(u32),
    UnsupportedVersion(u32),
    InvalidBound(u32),
    ZeroWordCount(usize),
    TruncatedInstruction { offset: usize, word_count: usize },
    UnknownOpcode { offset: usize, opcode: u16 },
    InvalidInstruction { offset: usize, opcode: u16 },
    MissingInstruction(&'static str),
}

impl fmt::Display for SpirvValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HeaderTooShort => formatter.write_str("SPIR-V header is shorter than five words"),
            Self::BadMagic(magic) => write!(formatter, "invalid SPIR-V magic 0x{magic:08x}"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported SPIR-V version word 0x{version:08x}")
            }
            Self::InvalidBound(bound) => write!(formatter, "invalid SPIR-V ID bound {bound}"),
            Self::ZeroWordCount(offset) => {
                write!(formatter, "zero SPIR-V word count at word {offset}")
            }
            Self::TruncatedInstruction { offset, word_count } => write!(
                formatter,
                "truncated SPIR-V instruction at word {offset} with {word_count} words"
            ),
            Self::UnknownOpcode { offset, opcode } => {
                write!(formatter, "unknown SPIR-V opcode {opcode} at word {offset}")
            }
            Self::InvalidInstruction { offset, opcode } => {
                write!(
                    formatter,
                    "invalid operands for SPIR-V opcode {opcode} at word {offset}"
                )
            }
            Self::MissingInstruction(name) => {
                write!(formatter, "missing required SPIR-V instruction {name}")
            }
        }
    }
}

impl Error for SpirvValidationError {}

/// Scalar/vector element classification carried with reflected resource ABI.
///
/// This is deliberately independent of `TypeId`: backend source translators
/// must not reconstruct JIR type tables from an opaque artifact. `None` on a
/// resource remains an explicit unknown/composite-layout case.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceElementType {
    /// Integer element with signedness, scalar width and vector lane count.
    Integer { signed: bool, bits: u16, lanes: u16 },
    /// Floating-point element with scalar width and vector lane count.
    Float { bits: u16, lanes: u16 },
}

impl ResourceElementType {
    /// Parses the portable shader spelling used by MSL/HLSL source contracts.
    pub fn from_shader_name(name: &str) -> Option<Self> {
        let digit_start = name
            .find(|character: char| character.is_ascii_digit())
            .unwrap_or(name.len());
        let lanes = if digit_start == name.len() {
            1
        } else {
            name[digit_start..].parse::<u16>().ok()?
        };
        if !(1..=4).contains(&lanes) {
            return None;
        }
        match &name[..digit_start] {
            "uint" => Some(Self::Integer {
                signed: false,
                bits: 32,
                lanes,
            }),
            "int" => Some(Self::Integer {
                signed: true,
                bits: 32,
                lanes,
            }),
            "half" => Some(Self::Float { bits: 16, lanes }),
            "float" => Some(Self::Float { bits: 32, lanes }),
            "double" => Some(Self::Float { bits: 64, lanes }),
            _ => None,
        }
    }

    /// Returns the byte stride when the scalar width is byte-addressable.
    #[must_use]
    pub const fn byte_stride(self) -> Option<u32> {
        let (bits, lanes) = match self {
            Self::Integer { bits, lanes, .. } | Self::Float { bits, lanes } => (bits, lanes),
        };
        if bits % 8 != 0 {
            return None;
        }
        Some((bits as u32 / 8) * lanes as u32)
    }

    /// Returns whether the metadata uses a representable scalar/vector shape.
    #[must_use]
    pub const fn is_valid(self) -> bool {
        let (bits, lanes) = match self {
            Self::Integer { bits, lanes, .. } | Self::Float { bits, lanes } => (bits, lanes),
        };
        bits != 0 && lanes >= 1 && lanes <= 4
    }

    fn with_lanes(self, lanes: u16) -> Option<Self> {
        if !(1..=4).contains(&lanes) {
            return None;
        }
        Some(match self {
            Self::Integer { signed, bits, .. } => Self::Integer {
                signed,
                bits,
                lanes,
            },
            Self::Float { bits, .. } => Self::Float { bits, lanes },
        })
    }
}

/// Conservative access mode recorded for one reflected resource.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceAccess {
    /// Uniform/constant data is read-only.
    ReadOnly,
    /// Storage data is written but never read by the shader.
    WriteOnly,
    /// Storage/workgroup data may be read and written.
    ReadWrite,
}

impl ResourceAccess {
    /// Whether shader execution may read this resource.
    #[must_use]
    pub const fn can_read(self) -> bool {
        matches!(self, Self::ReadOnly | Self::ReadWrite)
    }

    /// Whether shader execution may write this resource.
    #[must_use]
    pub const fn can_write(self) -> bool {
        matches!(self, Self::WriteOnly | Self::ReadWrite)
    }
}

/// Stable resource binding metadata consumed by the Vulkan host runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceBinding {
    /// Descriptor binding ordinal in function parameter order.
    pub binding: u32,
    /// Descriptor set/space assigned to this resource by the portable ABI.
    /// The current JIR reflection emits set `0`; later frontends may preserve
    /// explicit descriptor-set metadata without changing the binding ordinal.
    pub descriptor_set: u32,
    /// Source parameter name or generated `resource_N` fallback.
    pub name: String,
    /// Pointee type in the JIR module type table.
    pub element_type: TypeId,
    /// Portable scalar/vector classification when the layout is known.
    pub element_type_info: Option<ResourceElementType>,
    /// Validated byte stride for scalar/vector storage elements when known.
    /// Composite layouts remain `None` until target layout metadata is present.
    pub element_stride: Option<u32>,
    /// GPU address space preserved from JIR.
    pub address_space: AddressSpace,
    /// Conservative access contract.
    pub access: ResourceAccess,
}

/// A validated, portable SPIR-V artifact exported from a JIR compute entry.
///
/// The artifact keeps the entry name, workgroup contract and reflected
/// resources next to the words so native backends do not have to reconstruct
/// ABI metadata from raw SPIR-V. The current 0.1 exporter is intentionally
/// narrow: dedicated entry points cover verified storage `u32` add, the
/// runtime-length global-index `u32` binary family and the runtime-length
/// global-index `f32` add family. The integer family accepts a dataflow-
/// tolerant one-block SSA body while retaining a strict bounds-before-access
/// contract; unsupported JIR is rejected before an artifact exists.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpirvArtifact {
    /// Entry-point spelling used by downstream shader toolchains.
    pub entry_name: String,
    /// Compile-time local workgroup dimensions.
    pub workgroup_size: [u32; 3],
    /// Reflected GPU resource bindings in stable parameter order.
    pub resources: Vec<ResourceBinding>,
    /// Little-endian SPIR-V words, including the five-word module header.
    pub words: Vec<u32>,
}

impl SpirvArtifact {
    /// Returns the module as little-endian bytes for native tool invocations.
    #[must_use]
    pub fn bytes_le(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.words.len() * std::mem::size_of::<u32>());
        for word in &self.words {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes
    }

    /// Re-validates the portable word stream before it crosses a backend ABI.
    pub fn validate(&self) -> Result<(), SpirvValidationError> {
        validate_spirv(&self.words)
    }
}

/// Adds stable `OpName` decorations for descriptor variables before an
/// artifact crosses into source translators such as SPIRV-Cross.
///
/// The compact emitters historically relied on numeric SPIR-V ids, which is
/// valid SPIR-V but makes a source translator invent names such as `_12`.
/// Artifact metadata already carries the authoritative JIR parameter names;
/// preserve those names in the module so HLSL/MSL source audits can verify the
/// same ABI without weakening binding checks.
pub fn annotate_spirv_resource_names(
    mut words: Vec<u32>,
    resources: &[ResourceBinding],
) -> Vec<u32> {
    if resources.is_empty() {
        return words;
    }
    let mut offset = 5_usize;
    let mut binding_ids = BTreeMap::new();
    let mut named_ids = BTreeSet::new();
    let mut insertion = None;
    while offset < words.len() {
        let instruction = words[offset];
        let word_count = (instruction >> 16) as usize;
        let opcode = (instruction & 0xffff) as u16;
        if word_count == 0 || offset + word_count > words.len() {
            return words;
        }
        let operands = &words[offset + 1..offset + word_count];
        if opcode == OP_DECORATE && operands.len() == 3 && operands[1] == DECORATION_BINDING {
            binding_ids.insert(operands[2], operands[0]);
        }
        if opcode == OP_NAME && !operands.is_empty() {
            named_ids.insert(operands[0]);
        }
        if insertion.is_none()
            && matches!(
                opcode,
                OP_TYPE_VOID
                    | OP_TYPE_BOOL
                    | OP_TYPE_INT
                    | OP_TYPE_FLOAT
                    | OP_TYPE_VECTOR
                    | OP_TYPE_RUNTIME_ARRAY
                    | OP_TYPE_STRUCT
                    | OP_TYPE_POINTER
                    | OP_TYPE_FUNCTION
            )
        {
            insertion = Some(offset);
        }
        offset += word_count;
    }
    let mut names = Vec::new();
    for resource in resources {
        let Some(&variable_id) = binding_ids.get(&resource.binding) else {
            continue;
        };
        if named_ids.contains(&variable_id) {
            continue;
        }
        instruction_string(&mut names, OP_NAME, &[variable_id], &resource.name);
    }
    if let Some(insertion) = insertion {
        words.splice(insertion..insertion, names);
    }
    words
}

/// Reflection failures before a Vulkan descriptor layout can be built.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReflectionError {
    MissingFunction(FunctionId),
    HostAddressSpace(AddressSpace),
    DuplicateName(String),
}

impl fmt::Display for ReflectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingFunction(function) => {
                write!(
                    formatter,
                    "reflection function {} does not exist",
                    function.index()
                )
            }
            Self::HostAddressSpace(space) => {
                write!(formatter, "resource uses non-GPU address space {space:?}")
            }
            Self::DuplicateName(name) => write!(formatter, "duplicate resource name `{name}`"),
        }
    }
}

impl Error for ReflectionError {}

/// Reflects stable descriptor candidates from GPU pointer parameters.
///
/// Non-pointer parameters (for example `delta: Float32`) are uniforms owned by
/// the entry wrapper and are not emitted as descriptor resources. Storage and
/// workgroup pointers use exact `Load`/`Store` effects while their parameter
/// roots do not escape. Unsupported pointer flow falls back to conservative
/// read/write access.
pub fn reflect_resources(
    module: &Module,
    function: FunctionId,
) -> Result<Vec<ResourceBinding>, ReflectionError> {
    let function = module
        .functions
        .get(function.index())
        .ok_or(ReflectionError::MissingFunction(function))?;
    let mut names = BTreeSet::new();
    let mut resources = Vec::new();
    let mut resource_parameters = Vec::new();
    for parameter in &function.parameters {
        let Some(Type::Pointer {
            pointee,
            address_space,
        }) = module.types.get(parameter.ty.index())
        else {
            continue;
        };
        if !address_space.is_gpu() {
            return Err(ReflectionError::HostAddressSpace(*address_space));
        }
        let binding = u32::try_from(resources.len()).expect("resource binding count is bounded");
        let name = parameter
            .name
            .clone()
            .unwrap_or_else(|| format!("resource_{binding}"));
        if !names.insert(name.clone()) {
            return Err(ReflectionError::DuplicateName(name));
        }
        resources.push(ResourceBinding {
            binding,
            descriptor_set: 0,
            name,
            element_type: *pointee,
            element_type_info: gpu_element_type(module, *pointee),
            element_stride: gpu_element_stride(module, *pointee),
            address_space: *address_space,
            access: if *address_space == AddressSpace::Uniform {
                ResourceAccess::ReadOnly
            } else {
                ResourceAccess::ReadWrite
            },
        });
        resource_parameters.push(parameter.value);
    }
    if let Some(inferred) = infer_resource_access(function, &resource_parameters, module) {
        for (resource, access) in resources.iter_mut().zip(inferred) {
            if resource.address_space != AddressSpace::Uniform {
                resource.access = access;
            }
        }
    }
    Ok(resources)
}

/// Infers exact read/write effects for non-escaping GPU pointer parameters.
/// Unsupported pointer flow falls back to the conservative reflection above.
fn infer_resource_access(
    function: &Function,
    resource_parameters: &[ValueId],
    module: &Module,
) -> Option<Vec<ResourceAccess>> {
    if function
        .blocks
        .iter()
        .any(|block| !block.parameters.is_empty())
    {
        return None;
    }
    let mut roots = BTreeMap::<ValueId, usize>::new();
    for (index, value) in resource_parameters.iter().copied().enumerate() {
        roots.insert(value, index);
    }
    let mut reads = vec![false; resource_parameters.len()];
    let mut writes = vec![false; resource_parameters.len()];
    for block in &function.blocks {
        for instruction in &block.instructions {
            match &instruction.kind {
                InstructionKind::Offset { base, .. } => {
                    if let Some(root) = roots.get(base).copied() {
                        let result = instruction.result?;
                        if !matches!(
                            module.types.get(result.ty.index()),
                            Some(Type::Pointer { .. })
                        ) {
                            return None;
                        }
                        roots.insert(result.value, root);
                    }
                }
                InstructionKind::Load { pointer, .. } => {
                    if let Some(root) = roots.get(pointer).copied() {
                        reads[root] = true;
                        if instruction.result.is_some_and(|result| {
                            matches!(
                                module.types.get(result.ty.index()),
                                Some(Type::Pointer { .. })
                            )
                        }) {
                            return None;
                        }
                    }
                }
                InstructionKind::Store { pointer, value, .. } => {
                    if roots.contains_key(value) {
                        return None;
                    }
                    if let Some(root) = roots.get(pointer).copied() {
                        writes[root] = true;
                    }
                }
                InstructionKind::AssumeNoAlias { .. } => {}
                InstructionKind::Call { arguments, .. }
                | InstructionKind::IndirectCall { arguments, .. }
                    if arguments.iter().any(|value| roots.contains_key(value)) =>
                {
                    return None;
                }
                InstructionKind::Aggregate { elements }
                | InstructionKind::EnumConstruct {
                    fields: elements, ..
                } if elements.iter().any(|value| roots.contains_key(value)) => {
                    return None;
                }
                InstructionKind::Select {
                    when_true,
                    when_false,
                    ..
                } if roots.contains_key(when_true) || roots.contains_key(when_false) => {
                    return None;
                }
                InstructionKind::Drop { value }
                | InstructionKind::Unary { operand: value, .. }
                | InstructionKind::Cast { value, .. }
                | InstructionKind::VectorSplat { value, .. }
                    if roots.contains_key(value) =>
                {
                    return None;
                }
                _ => {}
            }
        }
        let escapes = match &block.terminator {
            Terminator::Return { value } => value.iter().any(|value| roots.contains_key(value)),
            Terminator::Jump { arguments, .. } => {
                arguments.iter().any(|value| roots.contains_key(value))
            }
            Terminator::Branch {
                then_arguments,
                else_arguments,
                ..
            } => then_arguments
                .iter()
                .chain(else_arguments)
                .any(|value| roots.contains_key(value)),
            Terminator::Switch {
                cases,
                default_arguments,
                ..
            } => cases
                .iter()
                .flat_map(|case| &case.arguments)
                .chain(default_arguments)
                .any(|value| roots.contains_key(value)),
            Terminator::Unreachable => false,
        };
        if escapes {
            return None;
        }
    }
    Some(
        reads
            .into_iter()
            .zip(writes)
            .map(|(read, write)| match (read, write) {
                (true, false) => ResourceAccess::ReadOnly,
                (false, true) => ResourceAccess::WriteOnly,
                (true, true) | (false, false) => ResourceAccess::ReadWrite,
            })
            .collect(),
    )
}

/// Applies an exact binding-to-access contract to the SPIR-V annotation
/// section. Existing compatible decorations are retained; contradictory or
/// missing bindings are rejected before the module is returned.
pub fn apply_spirv_resource_access_decorations(
    words: &mut Vec<u32>,
    resource_access: &[(u32, ResourceAccess)],
) -> Result<(), SpirvError> {
    let mut binding_variables = BTreeMap::new();
    let mut existing_access = BTreeMap::<u32, (bool, bool)>::new();
    let mut type_section_start = None;
    let mut index = 5;
    while index < words.len() {
        let word_count = (words[index] >> 16) as usize;
        let opcode = (words[index] & 0xffff) as u16;
        if word_count == 0 || index + word_count > words.len() {
            return Err(SpirvError::UnsupportedKernelShape(
                "resource access decoration scan found malformed SPIR-V",
            ));
        }
        if opcode == OP_TYPE_VOID && type_section_start.is_none() {
            type_section_start = Some(index);
        }
        if opcode == OP_DECORATE && word_count >= 3 {
            let target = words[index + 1];
            let decoration = words[index + 2];
            match decoration {
                DECORATION_BINDING if word_count == 4 => {
                    if binding_variables.insert(words[index + 3], target).is_some() {
                        return Err(SpirvError::UnsupportedKernelShape(
                            "resource access decoration scan found duplicate binding",
                        ));
                    }
                }
                DECORATION_NON_WRITABLE if word_count == 3 => {
                    existing_access.entry(target).or_default().0 = true;
                }
                DECORATION_NON_READABLE if word_count == 3 => {
                    existing_access.entry(target).or_default().1 = true;
                }
                _ => {}
            }
        }
        index += word_count;
    }
    let insertion = type_section_start.ok_or(SpirvError::UnsupportedKernelShape(
        "resource access decoration scan found no type section",
    ))?;
    let mut decorations = Vec::new();
    for &(binding, access) in resource_access {
        let variable =
            binding_variables
                .get(&binding)
                .copied()
                .ok_or(SpirvError::UnsupportedKernelShape(
                    "reflected resource has no SPIR-V binding variable",
                ))?;
        let (non_writable, non_readable) =
            existing_access.get(&variable).copied().unwrap_or_default();
        let required = match (access, non_writable, non_readable) {
            (ResourceAccess::ReadOnly, false, false) => Some(DECORATION_NON_WRITABLE),
            (ResourceAccess::ReadOnly, true, false) => None,
            (ResourceAccess::WriteOnly, false, false) => Some(DECORATION_NON_READABLE),
            (ResourceAccess::WriteOnly, false, true) => None,
            (ResourceAccess::ReadWrite, false, false) => None,
            _ => {
                return Err(SpirvError::UnsupportedKernelShape(
                    "SPIR-V access decorations contradict reflected resource access",
                ));
            }
        };
        if let Some(decoration) = required {
            instruction(&mut decorations, OP_DECORATE, &[variable, decoration]);
        }
    }
    words.splice(insertion..insertion, decorations);
    validate_spirv(words).map_err(|_| {
        SpirvError::UnsupportedKernelShape("resource access decorated SPIR-V validation failed")
    })
}

/// Mirrors reflected JIR resource access into emitted SPIR-V so artifact
/// metadata and external source translation cannot drift.
fn apply_reflected_resource_access_decorations(
    words: &mut Vec<u32>,
    resources: &[ResourceBinding],
) -> Result<(), SpirvError> {
    let resource_access = resources
        .iter()
        .map(|resource| (resource.binding, resource.access))
        .collect::<Vec<_>>();
    apply_spirv_resource_access_decorations(words, &resource_access)
}

fn gpu_element_type(module: &Module, type_id: TypeId) -> Option<ResourceElementType> {
    match module.types.get(type_id.index())? {
        Type::Integer { signed, bits } => Some(ResourceElementType::Integer {
            signed: *signed,
            bits: *bits,
            lanes: 1,
        }),
        Type::Float { bits } => Some(ResourceElementType::Float {
            bits: *bits,
            lanes: 1,
        }),
        Type::Vector { element, lanes } => gpu_element_type(module, *element)?.with_lanes(*lanes),
        _ => None,
    }
}

fn gpu_element_stride(module: &Module, type_id: TypeId) -> Option<u32> {
    match module.types.get(type_id.index())? {
        Type::Integer { bits, .. } | Type::Float { bits } if bits % 8 == 0 => {
            Some(u32::from(*bits / 8))
        }
        Type::Vector { element, lanes } => gpu_element_stride(module, *element)
            .and_then(|stride| stride.checked_mul(u32::from(*lanes))),
        Type::Array { .. }
        | Type::Struct { .. }
        | Type::NominalStruct { .. }
        | Type::Enum { .. }
        | Type::NominalEnum { .. }
        | Type::Unit
        | Type::RegionHandle
        | Type::Bool
        | Type::Pointer { .. } => None,
        _ => None,
    }
}

/// Validates the structural subset emitted by this crate.
///
/// This is intentionally smaller than Khronos `spirv-val`: it catches corrupt
/// headers, lengths, IDs, required compute declarations and function nesting.
/// A release pipeline should still run the external validator when available.
pub fn validate_spirv(words: &[u32]) -> Result<(), SpirvValidationError> {
    if words.len() < 5 {
        return Err(SpirvValidationError::HeaderTooShort);
    }
    if words[0] != SPIRV_MAGIC {
        return Err(SpirvValidationError::BadMagic(words[0]));
    }
    if words[1] != SPIRV_VERSION_1_3 {
        return Err(SpirvValidationError::UnsupportedVersion(words[1]));
    }
    if words[3] < 2 {
        return Err(SpirvValidationError::InvalidBound(words[3]));
    }

    let bound = words[3];
    let mut offset = 5usize;
    let mut capability = false;
    let mut memory_model = false;
    let mut entry_point = None;
    let mut local_size_entry = None;
    let mut local_size_id_operands = None;
    let mut scalar_integer_types = BTreeSet::new();
    let mut constant_result_types = BTreeMap::new();
    let mut zero_literal_integer_constants = BTreeSet::new();
    let mut type_void = false;
    let mut type_function = false;
    let mut function_id = None;
    let mut label = false;
    let mut returned = false;
    let mut function_end = false;
    let mut in_function = false;

    while offset < words.len() {
        let first = words[offset];
        let word_count = (first >> 16) as usize;
        let opcode = (first & 0xffff) as u16;
        if word_count == 0 {
            return Err(SpirvValidationError::ZeroWordCount(offset));
        }
        let end = offset.saturating_add(word_count);
        if end > words.len() {
            return Err(SpirvValidationError::TruncatedInstruction { offset, word_count });
        }
        let operands = &words[offset + 1..end];
        match opcode {
            OP_CAPABILITY if operands == [CAPABILITY_SHADER] => capability = true,
            OP_MEMORY_MODEL if operands == [ADDRESSING_LOGICAL, MEMORY_MODEL_GLSL450] => {
                memory_model = true;
            }
            OP_ENTRY_POINT if entry_point_operands_valid(operands, bound) => {
                if entry_point.replace(operands[1]).is_some() {
                    return Err(SpirvValidationError::InvalidInstruction { offset, opcode });
                }
            }
            OP_EXECUTION_MODE
                if operands.len() == 5
                    && operands[1] == EXECUTION_MODE_LOCAL_SIZE
                    && operands[0] > 0
                    && operands[0] < bound
                    && operands[2] > 0
                    && operands[3] > 0
                    && operands[4] > 0 =>
            {
                if local_size_entry.replace(operands[0]).is_some() {
                    return Err(SpirvValidationError::InvalidInstruction { offset, opcode });
                }
            }
            OP_EXECUTION_MODE_ID
                if operands.len() == 5
                    && operands[1] == EXECUTION_MODE_LOCAL_SIZE_ID
                    && operands[0] > 0
                    && operands[0] < bound
                    && operands[2] > 0
                    && operands[2] < bound
                    && operands[3] > 0
                    && operands[3] < bound
                    && operands[4] > 0
                    && operands[4] < bound =>
            {
                if local_size_entry.replace(operands[0]).is_some() {
                    return Err(SpirvValidationError::InvalidInstruction { offset, opcode });
                }
                local_size_id_operands = Some([operands[2], operands[3], operands[4]]);
            }
            OP_NAME
                if operands.len() >= 2
                    && operands[0] > 0
                    && operands[0] < bound
                    && string_words_valid(&operands[1..]) => {}
            OP_DECORATE if operands.len() >= 2 && operands[0] > 0 && operands[0] < bound => {}
            OP_MEMBER_DECORATE if operands.len() >= 3 && operands[0] > 0 && operands[0] < bound => {
            }
            OP_TYPE_VOID if operands.len() == 1 && operands[0] > 0 && operands[0] < bound => {
                type_void = true;
            }
            OP_TYPE_BOOL if operands.len() == 1 && operands[0] > 0 && operands[0] < bound => {}
            OP_TYPE_INT
                if operands.len() == 3
                    && operands[0] > 0
                    && operands[0] < bound
                    && operands[1] > 0
                    && operands[2] <= 1 =>
            {
                scalar_integer_types.insert(operands[0]);
            }
            OP_TYPE_FLOAT
                if operands.len() == 2
                    && operands[0] > 0
                    && operands[0] < bound
                    && operands[1] > 0 => {}
            OP_TYPE_VECTOR
                if operands.len() == 3
                    && operands[0] > 0
                    && operands[0] < bound
                    && operands[1] > 0
                    && operands[1] < bound
                    && operands[2] > 0 => {}
            OP_TYPE_RUNTIME_ARRAY
                if operands.len() == 2
                    && operands[0] > 0
                    && operands[0] < bound
                    && operands[1] > 0
                    && operands[1] < bound => {}
            OP_TYPE_STRUCT
                if !operands.is_empty()
                    && operands[0] > 0
                    && operands[0] < bound
                    && operands[1..].iter().all(|id| *id > 0 && *id < bound) => {}
            OP_TYPE_POINTER
                if operands.len() == 3
                    && operands[0] > 0
                    && operands[0] < bound
                    && operands[2] > 0
                    && operands[2] < bound => {}
            OP_TYPE_FUNCTION
                if operands.len() == 2
                    && operands[0] > 0
                    && operands[0] < bound
                    && operands[1] > 0
                    && operands[1] < bound =>
            {
                type_function = true;
            }
            OP_CONSTANT
                if operands.len() >= 3
                    && operands[0] > 0
                    && operands[0] < bound
                    && operands[1] > 0
                    && operands[1] < bound =>
            {
                constant_result_types.insert(operands[1], operands[0]);
                if scalar_integer_types.contains(&operands[0])
                    && operands[2..].iter().all(|word| *word == 0)
                {
                    zero_literal_integer_constants.insert(operands[1]);
                }
            }
            OP_SPEC_CONSTANT
                if operands.len() >= 3
                    && operands[0] > 0
                    && operands[0] < bound
                    && operands[1] > 0
                    && operands[1] < bound =>
            {
                constant_result_types.insert(operands[1], operands[0]);
            }
            OP_SPEC_CONSTANT_OP
                if operands.len() >= 3
                    && operands[0] > 0
                    && operands[0] < bound
                    && operands[1] > 0
                    && operands[1] < bound
                    && operands[2] > 0 =>
            {
                constant_result_types.insert(operands[1], operands[0]);
            }
            OP_CONSTANT_COMPOSITE
                if operands.len() >= 3
                    && operands[0] > 0
                    && operands[0] < bound
                    && operands[1] > 0
                    && operands[1] < bound
                    && operands[2..].iter().all(|id| *id > 0 && *id < bound) => {}
            OP_ACCESS_CHAIN
                if operands.len() >= 4
                    && operands[0] > 0
                    && operands[0] < bound
                    && operands[1] > 0
                    && operands[1] < bound
                    && operands[2] > 0
                    && operands[2] < bound
                    && operands[3..].iter().all(|id| *id > 0 && *id < bound) => {}
            OP_COMPOSITE_EXTRACT
                if operands.len() >= 4
                    && operands[0] > 0
                    && operands[0] < bound
                    && operands[1] > 0
                    && operands[1] < bound
                    && operands[2] > 0
                    && operands[2] < bound => {}
            OP_COMPOSITE_CONSTRUCT
                if operands.len() >= 3
                    && operands[0] > 0
                    && operands[0] < bound
                    && operands[1] > 0
                    && operands[1] < bound
                    && operands[2..].iter().all(|id| *id > 0 && *id < bound) => {}
            OP_STORE
                if operands.len() >= 2
                    && operands[0] > 0
                    && operands[0] < bound
                    && operands[1] > 0
                    && operands[1] < bound => {}
            OP_LOAD
                if operands.len() == 3
                    && operands[0] > 0
                    && operands[0] < bound
                    && operands[1] > 0
                    && operands[1] < bound
                    && operands[2] > 0
                    && operands[2] < bound => {}
            OP_IADD
            | OP_ISUB
            | OP_IMUL
            | OP_UDIV
            | OP_UMOD
            | OP_SHIFT_RIGHT_LOGICAL
            | OP_SHIFT_LEFT_LOGICAL
            | OP_BITWISE_OR
            | OP_BITWISE_XOR
            | OP_BITWISE_AND
            | OP_LOGICAL_AND
                if operands.len() == 4
                    && operands[0] > 0
                    && operands[0] < bound
                    && operands[1] > 0
                    && operands[1] < bound
                    && operands[2] > 0
                    && operands[2] < bound
                    && operands[3] > 0
                    && operands[3] < bound => {}
            OP_FADD | OP_FSUB | OP_FMUL
                if operands.len() == 4
                    && operands[0] > 0
                    && operands[0] < bound
                    && operands[1] > 0
                    && operands[1] < bound
                    && operands[2] > 0
                    && operands[2] < bound
                    && operands[3] > 0
                    && operands[3] < bound => {}
            OP_ULT
                if operands.len() == 4
                    && operands[0] > 0
                    && operands[0] < bound
                    && operands[1] > 0
                    && operands[1] < bound
                    && operands[2] > 0
                    && operands[2] < bound
                    && operands[3] > 0
                    && operands[3] < bound => {}
            OP_FUNCTION
                if operands.len() == 4
                    && operands[0] > 0
                    && operands[0] < bound
                    && operands[1] > 0
                    && operands[1] < bound
                    && !in_function =>
            {
                function_id = Some(operands[1]);
                in_function = true;
            }
            OP_LABEL
                if operands.len() == 1 && operands[0] > 0 && operands[0] < bound && in_function =>
            {
                label = true;
            }
            OP_SELECTION_MERGE
                if operands.len() == 2 && operands[0] > 0 && operands[0] < bound && in_function => {
            }
            OP_BRANCH
                if operands.len() == 1 && operands[0] > 0 && operands[0] < bound && in_function => {
            }
            OP_BRANCH_CONDITIONAL
                if operands.len() == 3
                    && operands[0] > 0
                    && operands[0] < bound
                    && operands[1] > 0
                    && operands[1] < bound
                    && operands[2] > 0
                    && operands[2] < bound
                    && in_function => {}
            OP_RETURN if operands.is_empty() && in_function => returned = true,
            OP_FUNCTION_END if operands.is_empty() && in_function => {
                in_function = false;
                function_end = true;
            }
            OP_VARIABLE
                if operands.len() == 3
                    && operands[0] > 0
                    && operands[0] < bound
                    && operands[1] > 0
                    && operands[1] < bound => {}
            OP_CAPABILITY
            | OP_MEMORY_MODEL
            | OP_ENTRY_POINT
            | OP_EXECUTION_MODE
            | OP_EXECUTION_MODE_ID
            | OP_NAME
            | OP_DECORATE
            | OP_MEMBER_DECORATE
            | OP_TYPE_VOID
            | OP_TYPE_BOOL
            | OP_TYPE_INT
            | OP_TYPE_FLOAT
            | OP_TYPE_VECTOR
            | OP_TYPE_RUNTIME_ARRAY
            | OP_TYPE_STRUCT
            | OP_TYPE_POINTER
            | OP_TYPE_FUNCTION
            | OP_CONSTANT
            | OP_CONSTANT_COMPOSITE
            | OP_SPEC_CONSTANT
            | OP_SPEC_CONSTANT_OP
            | OP_ACCESS_CHAIN
            | OP_COMPOSITE_CONSTRUCT
            | OP_COMPOSITE_EXTRACT
            | OP_STORE
            | OP_LOAD
            | OP_IADD
            | OP_ISUB
            | OP_IMUL
            | OP_UDIV
            | OP_UMOD
            | OP_SHIFT_RIGHT_LOGICAL
            | OP_SHIFT_LEFT_LOGICAL
            | OP_BITWISE_OR
            | OP_BITWISE_XOR
            | OP_BITWISE_AND
            | OP_LOGICAL_AND
            | OP_FADD
            | OP_FSUB
            | OP_FMUL
            | OP_ULT
            | OP_FUNCTION
            | OP_LABEL
            | OP_SELECTION_MERGE
            | OP_BRANCH
            | OP_BRANCH_CONDITIONAL
            | OP_RETURN
            | OP_FUNCTION_END
            | OP_VARIABLE => {
                return Err(SpirvValidationError::InvalidInstruction { offset, opcode });
            }
            _ => return Err(SpirvValidationError::UnknownOpcode { offset, opcode }),
        }
        offset = end;
    }
    if in_function {
        return Err(SpirvValidationError::MissingInstruction("OpFunctionEnd"));
    }
    if !capability {
        return Err(SpirvValidationError::MissingInstruction(
            "OpCapability Shader",
        ));
    }
    if !memory_model {
        return Err(SpirvValidationError::MissingInstruction(
            "OpMemoryModel Logical GLSL450",
        ));
    }
    if entry_point.is_none() {
        return Err(SpirvValidationError::MissingInstruction(
            "OpEntryPoint GLCompute",
        ));
    }
    if local_size_entry != entry_point {
        return Err(SpirvValidationError::MissingInstruction(
            "OpExecutionMode LocalSize for selected entry",
        ));
    }
    if let Some(operands) = local_size_id_operands
        && operands.iter().any(|id| {
            !constant_result_types
                .get(id)
                .is_some_and(|type_id| scalar_integer_types.contains(type_id))
                || zero_literal_integer_constants.contains(id)
        })
    {
        return Err(SpirvValidationError::InvalidInstruction {
            offset: 0,
            opcode: OP_EXECUTION_MODE_ID,
        });
    }
    if !type_void {
        return Err(SpirvValidationError::MissingInstruction("OpTypeVoid"));
    }
    if !type_function {
        return Err(SpirvValidationError::MissingInstruction("OpTypeFunction"));
    }
    if function_id != entry_point {
        return Err(SpirvValidationError::InvalidInstruction {
            offset: 0,
            opcode: OP_ENTRY_POINT,
        });
    }
    if !label {
        return Err(SpirvValidationError::MissingInstruction("OpLabel"));
    }
    if !returned {
        return Err(SpirvValidationError::MissingInstruction("OpReturn"));
    }
    if !function_end {
        return Err(SpirvValidationError::MissingInstruction("OpFunctionEnd"));
    }
    Ok(())
}

fn string_words_valid(words: &[u32]) -> bool {
    let mut terminated = false;
    for word in words {
        for byte in word.to_le_bytes() {
            if terminated && byte != 0 {
                return false;
            }
            if byte == 0 {
                terminated = true;
            }
        }
    }
    terminated
}

fn entry_point_operands_valid(operands: &[u32], bound: u32) -> bool {
    if operands.len() < 3
        || operands[0] != EXECUTION_MODEL_GL_COMPUTE
        || operands[1] == 0
        || operands[1] >= bound
    {
        return false;
    }
    let mut string_end = None;
    'words: for (index, word) in operands[2..].iter().enumerate() {
        for byte in word.to_le_bytes() {
            if byte == 0 {
                string_end = Some(index + 3);
                break 'words;
            }
        }
    }
    let Some(string_end) = string_end else {
        return false;
    };
    operands[string_end..]
        .iter()
        .all(|id| *id > 0 && *id < bound)
}

/// Emits one deterministic SPIR-V 1.3 compute entrypoint.
///
/// The current gate accepts a single body-less `Unit` function. This proves
/// module/header/entrypoint determinism without pretending that arbitrary JIR
/// instructions already have a correct SPIR-V lowering.
pub fn emit_compute(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<Vec<u32>, SpirvError> {
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(SpirvError::GpuVerificationFailed(verification_errors.len()));
    }
    let function = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    if function.name.is_empty() || function.name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    if !function.parameters.is_empty() {
        return Err(SpirvError::UnsupportedKernelShape(
            "parameters are not emitted yet",
        ));
    }
    if !matches!(module.types.get(function.result.index()), Some(Type::Unit)) {
        return Err(SpirvError::UnsupportedKernelShape(
            "entry result must be Unit",
        ));
    }
    if function.blocks.len() != 1 {
        return Err(SpirvError::UnsupportedKernelShape(
            "entry must have one block",
        ));
    }
    let block = &function.blocks[0];
    if !block.instructions.is_empty()
        || !matches!(block.terminator, Terminator::Return { value: None })
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "entry body must be an empty return",
        ));
    }

    let function_id = 1;
    let void_type = 2;
    let function_type = 3;
    let label = 4;
    let mut words = vec![SPIRV_MAGIC, SPIRV_VERSION_1_3, 0, 5, 0];
    instruction(&mut words, OP_CAPABILITY, &[CAPABILITY_SHADER]);
    instruction(
        &mut words,
        OP_MEMORY_MODEL,
        &[ADDRESSING_LOGICAL, MEMORY_MODEL_GLSL450],
    );
    instruction_string(
        &mut words,
        OP_ENTRY_POINT,
        &[EXECUTION_MODEL_GL_COMPUTE, function_id],
        &function.name,
    );
    instruction(
        &mut words,
        OP_EXECUTION_MODE,
        &[
            function_id,
            EXECUTION_MODE_LOCAL_SIZE,
            options.workgroup_size[0],
            options.workgroup_size[1],
            options.workgroup_size[2],
        ],
    );
    instruction_string(&mut words, OP_NAME, &[function_id], &function.name);
    instruction(&mut words, OP_TYPE_VOID, &[void_type]);
    instruction(&mut words, OP_TYPE_FUNCTION, &[function_type, void_type]);
    instruction(
        &mut words,
        OP_FUNCTION,
        &[void_type, function_id, FUNCTION_CONTROL_NONE, function_type],
    );
    instruction(&mut words, OP_LABEL, &[label]);
    instruction(&mut words, OP_RETURN, &[]);
    instruction(&mut words, OP_FUNCTION_END, &[]);
    validate_spirv(&words)
        .map_err(|_| SpirvError::UnsupportedKernelShape("descriptor fixture validation failed"))?;
    Ok(words)
}

/// Emits a deterministic descriptor-bound storage-buffer no-op kernel.
///
/// This is a runtime integration fixture for JAD-1305: the resource is part
/// of the SPIR-V interface and can be bound by Vulkan, but the function body
/// intentionally does not read or write it. Real JIR body/resource lowering
/// remains a separate gate.
pub fn emit_storage_noop(entry_name: &str, options: SpirvOptions) -> Result<Vec<u32>, SpirvError> {
    if entry_name.is_empty() || entry_name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    let function_id = 1;
    let void_type = 2;
    let function_type = 3;
    let label = 4;
    let uint_type = 5;
    let float_type = 6;
    let array_type = 7;
    let struct_type = 8;
    let storage_struct_pointer = 9;
    let resource_variable = 10;
    let mut words = vec![SPIRV_MAGIC, SPIRV_VERSION_1_3, 0, 11, 0];
    instruction(&mut words, OP_CAPABILITY, &[CAPABILITY_SHADER]);
    instruction(
        &mut words,
        OP_MEMORY_MODEL,
        &[ADDRESSING_LOGICAL, MEMORY_MODEL_GLSL450],
    );
    instruction(
        &mut words,
        OP_DECORATE,
        &[array_type, DECORATION_ARRAY_STRIDE, 4],
    );
    instruction(
        &mut words,
        OP_MEMBER_DECORATE,
        &[struct_type, 0, DECORATION_OFFSET, 0],
    );
    instruction(&mut words, OP_DECORATE, &[struct_type, DECORATION_BLOCK]);
    instruction(
        &mut words,
        OP_DECORATE,
        &[resource_variable, DECORATION_DESCRIPTOR_SET, 0],
    );
    instruction(
        &mut words,
        OP_DECORATE,
        &[resource_variable, DECORATION_BINDING, 0],
    );
    instruction_string_with_tail(
        &mut words,
        OP_ENTRY_POINT,
        &[EXECUTION_MODEL_GL_COMPUTE, function_id],
        entry_name,
        &[resource_variable],
    );
    instruction(
        &mut words,
        OP_EXECUTION_MODE,
        &[
            function_id,
            EXECUTION_MODE_LOCAL_SIZE,
            options.workgroup_size[0],
            options.workgroup_size[1],
            options.workgroup_size[2],
        ],
    );
    instruction(&mut words, OP_TYPE_VOID, &[void_type]);
    instruction(&mut words, OP_TYPE_INT, &[uint_type, 32, 0]);
    instruction(&mut words, OP_TYPE_FLOAT, &[float_type, 32]);
    instruction(&mut words, OP_TYPE_RUNTIME_ARRAY, &[array_type, float_type]);
    instruction(&mut words, OP_TYPE_STRUCT, &[struct_type, array_type]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[storage_struct_pointer, STORAGE_BUFFER, struct_type],
    );
    instruction(
        &mut words,
        OP_VARIABLE,
        &[storage_struct_pointer, resource_variable, STORAGE_BUFFER],
    );
    instruction(&mut words, OP_TYPE_FUNCTION, &[function_type, void_type]);
    instruction(
        &mut words,
        OP_FUNCTION,
        &[void_type, function_id, FUNCTION_CONTROL_NONE, function_type],
    );
    instruction(&mut words, OP_LABEL, &[label]);
    instruction(&mut words, OP_RETURN, &[]);
    instruction(&mut words, OP_FUNCTION_END, &[]);
    Ok(words)
}

/// Emits a descriptor-bound storage-buffer kernel that writes one `u32`.
///
/// This is the first executable data path for JAD-1305. It is deliberately a
/// fixed, deterministic fixture (`buffer[0] = value`); general JIR resource
/// and body lowering remains a separate compiler gate.
pub fn emit_storage_write(
    entry_name: &str,
    options: SpirvOptions,
    value: u32,
) -> Result<Vec<u32>, SpirvError> {
    if entry_name.is_empty() || entry_name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    let function_id = 1;
    let void_type = 2;
    let function_type = 3;
    let label = 4;
    let uint_type = 5;
    let array_type = 6;
    let struct_type = 7;
    let storage_struct_pointer = 8;
    let resource_variable = 9;
    let element_pointer = 10;
    let zero = 11;
    let constant_value = 12;
    let mut words = vec![SPIRV_MAGIC, SPIRV_VERSION_1_3, 0, 14, 0];
    instruction(&mut words, OP_CAPABILITY, &[CAPABILITY_SHADER]);
    instruction(
        &mut words,
        OP_MEMORY_MODEL,
        &[ADDRESSING_LOGICAL, MEMORY_MODEL_GLSL450],
    );
    instruction(
        &mut words,
        OP_DECORATE,
        &[array_type, DECORATION_ARRAY_STRIDE, 4],
    );
    instruction(
        &mut words,
        OP_MEMBER_DECORATE,
        &[struct_type, 0, DECORATION_OFFSET, 0],
    );
    instruction(&mut words, OP_DECORATE, &[struct_type, DECORATION_BLOCK]);
    instruction(
        &mut words,
        OP_DECORATE,
        &[resource_variable, DECORATION_DESCRIPTOR_SET, 0],
    );
    instruction(
        &mut words,
        OP_DECORATE,
        &[resource_variable, DECORATION_BINDING, 0],
    );
    instruction_string_with_tail(
        &mut words,
        OP_ENTRY_POINT,
        &[EXECUTION_MODEL_GL_COMPUTE, function_id],
        entry_name,
        &[resource_variable],
    );
    instruction(
        &mut words,
        OP_EXECUTION_MODE,
        &[
            function_id,
            EXECUTION_MODE_LOCAL_SIZE,
            options.workgroup_size[0],
            options.workgroup_size[1],
            options.workgroup_size[2],
        ],
    );
    instruction(&mut words, OP_TYPE_VOID, &[void_type]);
    instruction(&mut words, OP_TYPE_INT, &[uint_type, 32, 0]);
    instruction(&mut words, OP_TYPE_RUNTIME_ARRAY, &[array_type, uint_type]);
    instruction(&mut words, OP_TYPE_STRUCT, &[struct_type, array_type]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[storage_struct_pointer, STORAGE_BUFFER, struct_type],
    );
    instruction(
        &mut words,
        OP_VARIABLE,
        &[storage_struct_pointer, resource_variable, STORAGE_BUFFER],
    );
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[element_pointer, STORAGE_BUFFER, uint_type],
    );
    instruction(&mut words, OP_CONSTANT, &[uint_type, zero, 0]);
    instruction(&mut words, OP_CONSTANT, &[uint_type, constant_value, value]);
    instruction(&mut words, OP_TYPE_FUNCTION, &[function_type, void_type]);
    instruction(
        &mut words,
        OP_FUNCTION,
        &[void_type, function_id, FUNCTION_CONTROL_NONE, function_type],
    );
    instruction(&mut words, OP_LABEL, &[label]);
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[element_pointer, 13, resource_variable, zero, zero],
    );
    instruction(&mut words, OP_STORE, &[13, constant_value]);
    instruction(&mut words, OP_RETURN, &[]);
    instruction(&mut words, OP_FUNCTION_END, &[]);
    validate_spirv(&words)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage write validation failed"))?;
    Ok(words)
}

/// Emits a descriptor-bound storage-buffer kernel that adds one `u32` value at
/// element zero.
pub fn emit_storage_add(
    entry_name: &str,
    options: SpirvOptions,
    addend: u32,
) -> Result<Vec<u32>, SpirvError> {
    emit_storage_add_at(entry_name, options, 0, addend)
}

/// Emits `buffer[index] = buffer[index] + addend` for a constant index.
pub fn emit_storage_add_at(
    entry_name: &str,
    options: SpirvOptions,
    index: u32,
    addend: u32,
) -> Result<Vec<u32>, SpirvError> {
    if entry_name.is_empty() || entry_name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    let function_id = 1;
    let void_type = 2;
    let function_type = 3;
    let label = 4;
    let uint_type = 5;
    let array_type = 6;
    let struct_type = 7;
    let storage_struct_pointer = 8;
    let resource_variable = 9;
    let element_pointer = 10;
    let struct_member_zero = 11;
    let element_index = 12;
    let constant_addend = 13;
    let loaded = 14;
    let element_address = 15;
    let sum = 16;
    let mut words = vec![SPIRV_MAGIC, SPIRV_VERSION_1_3, 0, 17, 0];
    instruction(&mut words, OP_CAPABILITY, &[CAPABILITY_SHADER]);
    instruction(
        &mut words,
        OP_MEMORY_MODEL,
        &[ADDRESSING_LOGICAL, MEMORY_MODEL_GLSL450],
    );
    instruction(
        &mut words,
        OP_DECORATE,
        &[array_type, DECORATION_ARRAY_STRIDE, 4],
    );
    instruction(
        &mut words,
        OP_MEMBER_DECORATE,
        &[struct_type, 0, DECORATION_OFFSET, 0],
    );
    instruction(&mut words, OP_DECORATE, &[struct_type, DECORATION_BLOCK]);
    instruction(
        &mut words,
        OP_DECORATE,
        &[resource_variable, DECORATION_DESCRIPTOR_SET, 0],
    );
    instruction(
        &mut words,
        OP_DECORATE,
        &[resource_variable, DECORATION_BINDING, 0],
    );
    instruction_string_with_tail(
        &mut words,
        OP_ENTRY_POINT,
        &[EXECUTION_MODEL_GL_COMPUTE, function_id],
        entry_name,
        &[resource_variable],
    );
    instruction(
        &mut words,
        OP_EXECUTION_MODE,
        &[
            function_id,
            EXECUTION_MODE_LOCAL_SIZE,
            options.workgroup_size[0],
            options.workgroup_size[1],
            options.workgroup_size[2],
        ],
    );
    instruction(&mut words, OP_TYPE_VOID, &[void_type]);
    instruction(&mut words, OP_TYPE_INT, &[uint_type, 32, 0]);
    instruction(&mut words, OP_TYPE_RUNTIME_ARRAY, &[array_type, uint_type]);
    instruction(&mut words, OP_TYPE_STRUCT, &[struct_type, array_type]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[storage_struct_pointer, STORAGE_BUFFER, struct_type],
    );
    instruction(
        &mut words,
        OP_VARIABLE,
        &[storage_struct_pointer, resource_variable, STORAGE_BUFFER],
    );
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[element_pointer, STORAGE_BUFFER, uint_type],
    );
    instruction(&mut words, OP_CONSTANT, &[uint_type, struct_member_zero, 0]);
    instruction(&mut words, OP_CONSTANT, &[uint_type, element_index, index]);
    instruction(
        &mut words,
        OP_CONSTANT,
        &[uint_type, constant_addend, addend],
    );
    instruction(&mut words, OP_TYPE_FUNCTION, &[function_type, void_type]);
    instruction(
        &mut words,
        OP_FUNCTION,
        &[void_type, function_id, FUNCTION_CONTROL_NONE, function_type],
    );
    instruction(&mut words, OP_LABEL, &[label]);
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            element_pointer,
            element_address,
            resource_variable,
            struct_member_zero,
            element_index,
        ],
    );
    instruction(&mut words, OP_LOAD, &[uint_type, loaded, element_address]);
    instruction(
        &mut words,
        OP_IADD,
        &[uint_type, sum, loaded, constant_addend],
    );
    instruction(&mut words, OP_STORE, &[element_address, sum]);
    instruction(&mut words, OP_RETURN, &[]);
    instruction(&mut words, OP_FUNCTION_END, &[]);
    validate_spirv(&words)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage add validation failed"))?;
    Ok(words)
}

/// Emits a two-resource storage kernel:
/// `output[0] = input[0] + addend`.
pub fn emit_storage_dual_add(
    entry_name: &str,
    options: SpirvOptions,
    addend: u32,
) -> Result<Vec<u32>, SpirvError> {
    if entry_name.is_empty() || entry_name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    let function_id = 1;
    let void_type = 2;
    let function_type = 3;
    let label = 4;
    let uint_type = 5;
    let array_type = 6;
    let struct_type = 7;
    let storage_struct_pointer = 8;
    let element_pointer = 9;
    let input_variable = 10;
    let output_variable = 11;
    let zero = 12;
    let constant_addend = 13;
    let input_address = 14;
    let output_address = 15;
    let loaded = 16;
    let sum = 17;
    let mut words = vec![SPIRV_MAGIC, SPIRV_VERSION_1_3, 0, 18, 0];
    instruction(&mut words, OP_CAPABILITY, &[CAPABILITY_SHADER]);
    instruction(
        &mut words,
        OP_MEMORY_MODEL,
        &[ADDRESSING_LOGICAL, MEMORY_MODEL_GLSL450],
    );
    instruction(
        &mut words,
        OP_DECORATE,
        &[array_type, DECORATION_ARRAY_STRIDE, 4],
    );
    instruction(
        &mut words,
        OP_MEMBER_DECORATE,
        &[struct_type, 0, DECORATION_OFFSET, 0],
    );
    instruction(&mut words, OP_DECORATE, &[struct_type, DECORATION_BLOCK]);
    instruction(
        &mut words,
        OP_DECORATE,
        &[input_variable, DECORATION_DESCRIPTOR_SET, 0],
    );
    instruction(
        &mut words,
        OP_DECORATE,
        &[input_variable, DECORATION_BINDING, 0],
    );
    instruction(
        &mut words,
        OP_DECORATE,
        &[output_variable, DECORATION_DESCRIPTOR_SET, 0],
    );
    instruction(
        &mut words,
        OP_DECORATE,
        &[output_variable, DECORATION_BINDING, 1],
    );
    instruction_string_with_tail(
        &mut words,
        OP_ENTRY_POINT,
        &[EXECUTION_MODEL_GL_COMPUTE, function_id],
        entry_name,
        &[input_variable, output_variable],
    );
    instruction(
        &mut words,
        OP_EXECUTION_MODE,
        &[
            function_id,
            EXECUTION_MODE_LOCAL_SIZE,
            options.workgroup_size[0],
            options.workgroup_size[1],
            options.workgroup_size[2],
        ],
    );
    instruction(&mut words, OP_TYPE_VOID, &[void_type]);
    instruction(&mut words, OP_TYPE_INT, &[uint_type, 32, 0]);
    instruction(&mut words, OP_TYPE_RUNTIME_ARRAY, &[array_type, uint_type]);
    instruction(&mut words, OP_TYPE_STRUCT, &[struct_type, array_type]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[storage_struct_pointer, STORAGE_BUFFER, struct_type],
    );
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[element_pointer, STORAGE_BUFFER, uint_type],
    );
    instruction(
        &mut words,
        OP_VARIABLE,
        &[storage_struct_pointer, input_variable, STORAGE_BUFFER],
    );
    instruction(
        &mut words,
        OP_VARIABLE,
        &[storage_struct_pointer, output_variable, STORAGE_BUFFER],
    );
    instruction(&mut words, OP_CONSTANT, &[uint_type, zero, 0]);
    instruction(
        &mut words,
        OP_CONSTANT,
        &[uint_type, constant_addend, addend],
    );
    instruction(&mut words, OP_TYPE_FUNCTION, &[function_type, void_type]);
    instruction(
        &mut words,
        OP_FUNCTION,
        &[void_type, function_id, FUNCTION_CONTROL_NONE, function_type],
    );
    instruction(&mut words, OP_LABEL, &[label]);
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[element_pointer, input_address, input_variable, zero, zero],
    );
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[element_pointer, output_address, output_variable, zero, zero],
    );
    instruction(&mut words, OP_LOAD, &[uint_type, loaded, input_address]);
    instruction(
        &mut words,
        OP_IADD,
        &[uint_type, sum, loaded, constant_addend],
    );
    instruction(&mut words, OP_STORE, &[output_address, sum]);
    instruction(&mut words, OP_RETURN, &[]);
    instruction(&mut words, OP_FUNCTION_END, &[]);
    validate_spirv(&words)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage dual-add validation failed"))?;
    Ok(words)
}

/// Emits a three-resource dynamic-index kernel:
/// `output[index] = input[index] + addend`, where `index[0]` is loaded on GPU.
pub fn emit_storage_dynamic_index_add(
    entry_name: &str,
    options: SpirvOptions,
    addend: u32,
) -> Result<Vec<u32>, SpirvError> {
    if entry_name.is_empty() || entry_name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    let function_id = 1;
    let void_type = 2;
    let function_type = 3;
    let label = 4;
    let uint_type = 5;
    let array_type = 6;
    let struct_type = 7;
    let storage_struct_pointer = 8;
    let element_pointer = 9;
    let input_variable = 10;
    let output_variable = 11;
    let index_variable = 12;
    let zero = 13;
    let index_address = 14;
    let dynamic_index = 15;
    let constant_addend = 16;
    let input_address = 17;
    let output_address = 18;
    let loaded_input = 19;
    let sum = 20;
    let mut words = vec![SPIRV_MAGIC, SPIRV_VERSION_1_3, 0, 21, 0];
    instruction(&mut words, OP_CAPABILITY, &[CAPABILITY_SHADER]);
    instruction(
        &mut words,
        OP_MEMORY_MODEL,
        &[ADDRESSING_LOGICAL, MEMORY_MODEL_GLSL450],
    );
    instruction(
        &mut words,
        OP_DECORATE,
        &[array_type, DECORATION_ARRAY_STRIDE, 4],
    );
    instruction(
        &mut words,
        OP_MEMBER_DECORATE,
        &[struct_type, 0, DECORATION_OFFSET, 0],
    );
    instruction(&mut words, OP_DECORATE, &[struct_type, DECORATION_BLOCK]);
    for (variable, binding) in [
        (input_variable, 0),
        (output_variable, 1),
        (index_variable, 2),
    ] {
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_DESCRIPTOR_SET, 0],
        );
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_BINDING, binding],
        );
    }
    instruction_string_with_tail(
        &mut words,
        OP_ENTRY_POINT,
        &[EXECUTION_MODEL_GL_COMPUTE, function_id],
        entry_name,
        &[input_variable, output_variable, index_variable],
    );
    instruction(
        &mut words,
        OP_EXECUTION_MODE,
        &[
            function_id,
            EXECUTION_MODE_LOCAL_SIZE,
            options.workgroup_size[0],
            options.workgroup_size[1],
            options.workgroup_size[2],
        ],
    );
    instruction(&mut words, OP_TYPE_VOID, &[void_type]);
    instruction(&mut words, OP_TYPE_INT, &[uint_type, 32, 0]);
    instruction(&mut words, OP_TYPE_RUNTIME_ARRAY, &[array_type, uint_type]);
    instruction(&mut words, OP_TYPE_STRUCT, &[struct_type, array_type]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[storage_struct_pointer, STORAGE_BUFFER, struct_type],
    );
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[element_pointer, STORAGE_BUFFER, uint_type],
    );
    for variable in [input_variable, output_variable, index_variable] {
        instruction(
            &mut words,
            OP_VARIABLE,
            &[storage_struct_pointer, variable, STORAGE_BUFFER],
        );
    }
    instruction(&mut words, OP_CONSTANT, &[uint_type, zero, 0]);
    instruction(
        &mut words,
        OP_CONSTANT,
        &[uint_type, constant_addend, addend],
    );
    instruction(&mut words, OP_TYPE_FUNCTION, &[function_type, void_type]);
    instruction(
        &mut words,
        OP_FUNCTION,
        &[void_type, function_id, FUNCTION_CONTROL_NONE, function_type],
    );
    instruction(&mut words, OP_LABEL, &[label]);
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[element_pointer, index_address, index_variable, zero, zero],
    );
    instruction(
        &mut words,
        OP_LOAD,
        &[uint_type, dynamic_index, index_address],
    );
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            element_pointer,
            input_address,
            input_variable,
            zero,
            dynamic_index,
        ],
    );
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            element_pointer,
            output_address,
            output_variable,
            zero,
            dynamic_index,
        ],
    );
    instruction(
        &mut words,
        OP_LOAD,
        &[uint_type, loaded_input, input_address],
    );
    instruction(
        &mut words,
        OP_IADD,
        &[uint_type, sum, loaded_input, constant_addend],
    );
    instruction(&mut words, OP_STORE, &[output_address, sum]);
    instruction(&mut words, OP_RETURN, &[]);
    instruction(&mut words, OP_FUNCTION_END, &[]);
    validate_spirv(&words).map_err(|_| {
        SpirvError::UnsupportedKernelShape("storage dynamic-index validation failed")
    })?;
    Ok(words)
}

/// Emits a two-resource global-index kernel:
/// `output[GlobalInvocationId.x] = input[GlobalInvocationId.x] + addend`.
///
/// The caller must dispatch an exact multiple of `workgroup_size.x` elements;
/// a length/bounds operand is intentionally deferred to the next kernel shape.
pub fn emit_storage_global_index_add(
    entry_name: &str,
    options: SpirvOptions,
    addend: u32,
) -> Result<Vec<u32>, SpirvError> {
    emit_storage_global_index_add_with_bound(entry_name, options, addend, None)
}

/// Emits a bounds-safe two-resource global-index kernel.
/// `output[GlobalInvocationId.x]` is written only when the invocation index is
/// below `length`; out-of-range invocations leave the output untouched.
pub fn emit_storage_global_index_add_bounded(
    entry_name: &str,
    options: SpirvOptions,
    addend: u32,
    length: u32,
) -> Result<Vec<u32>, SpirvError> {
    if length == 0 {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index bound must be non-zero",
        ));
    }
    emit_storage_global_index_add_with_bound(entry_name, options, addend, Some(length))
}

/// Emits a bounds-safe one-resource global-index kernel:
/// `buffer[GlobalInvocationId.x] = value` when the invocation is below
/// `length`; out-of-range invocations leave the buffer untouched.
///
/// This is the smallest general storage-body shape. It deliberately has one
/// reflected storage resource and no arithmetic dependency on a second
/// buffer, so host runtimes can validate descriptor/resource handling without
/// inheriting the two-buffer add fixture contract.
pub fn emit_storage_global_index_write(
    entry_name: &str,
    options: SpirvOptions,
    value: u32,
    length: u32,
) -> Result<Vec<u32>, SpirvError> {
    if length == 0 {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index write bound must be non-zero",
        ));
    }
    if entry_name.is_empty() || entry_name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    let function_id = 1;
    let void_type = 2;
    let function_type = 3;
    let entry_label = 4;
    let uint_type = 5;
    let uint_array = 6;
    let uint_struct = 7;
    let storage_struct_pointer = 8;
    let uint_element_pointer = 9;
    let resource_variable = 10;
    let vector_type = 11;
    let input_pointer = 12;
    let global_variable = 13;
    let global_value = 14;
    let index = 15;
    let zero = 16;
    let constant_value = 17;
    let bool_type = 18;
    let bound_constant = 19;
    let in_bounds = 20;
    let body_label = 21;
    let merge_label = 22;
    let element_address = 23;
    let mut words = vec![SPIRV_MAGIC, SPIRV_VERSION_1_3, 0, 24, 0];
    instruction(&mut words, OP_CAPABILITY, &[CAPABILITY_SHADER]);
    instruction(
        &mut words,
        OP_MEMORY_MODEL,
        &[ADDRESSING_LOGICAL, MEMORY_MODEL_GLSL450],
    );
    instruction(
        &mut words,
        OP_DECORATE,
        &[uint_array, DECORATION_ARRAY_STRIDE, 4],
    );
    instruction(
        &mut words,
        OP_MEMBER_DECORATE,
        &[uint_struct, 0, DECORATION_OFFSET, 0],
    );
    instruction(&mut words, OP_DECORATE, &[uint_struct, DECORATION_BLOCK]);
    instruction(
        &mut words,
        OP_DECORATE,
        &[resource_variable, DECORATION_DESCRIPTOR_SET, 0],
    );
    instruction(
        &mut words,
        OP_DECORATE,
        &[resource_variable, DECORATION_BINDING, 0],
    );
    instruction(
        &mut words,
        OP_DECORATE,
        &[
            global_variable,
            DECORATION_BUILT_IN,
            BUILT_IN_GLOBAL_INVOCATION_ID,
        ],
    );
    instruction_string_with_tail(
        &mut words,
        OP_ENTRY_POINT,
        &[EXECUTION_MODEL_GL_COMPUTE, function_id],
        entry_name,
        &[resource_variable, global_variable],
    );
    instruction(
        &mut words,
        OP_EXECUTION_MODE,
        &[
            function_id,
            EXECUTION_MODE_LOCAL_SIZE,
            options.workgroup_size[0],
            options.workgroup_size[1],
            options.workgroup_size[2],
        ],
    );
    instruction(&mut words, OP_TYPE_VOID, &[void_type]);
    instruction(&mut words, OP_TYPE_INT, &[uint_type, 32, 0]);
    instruction(&mut words, OP_TYPE_RUNTIME_ARRAY, &[uint_array, uint_type]);
    instruction(&mut words, OP_TYPE_STRUCT, &[uint_struct, uint_array]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[storage_struct_pointer, STORAGE_BUFFER, uint_struct],
    );
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[uint_element_pointer, STORAGE_BUFFER, uint_type],
    );
    instruction(&mut words, OP_TYPE_VECTOR, &[vector_type, uint_type, 3]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[input_pointer, INPUT, vector_type],
    );
    instruction(
        &mut words,
        OP_VARIABLE,
        &[storage_struct_pointer, resource_variable, STORAGE_BUFFER],
    );
    instruction(
        &mut words,
        OP_VARIABLE,
        &[input_pointer, global_variable, INPUT],
    );
    instruction(&mut words, OP_TYPE_BOOL, &[bool_type]);
    instruction(&mut words, OP_CONSTANT, &[uint_type, zero, 0]);
    instruction(&mut words, OP_CONSTANT, &[uint_type, constant_value, value]);
    instruction(
        &mut words,
        OP_CONSTANT,
        &[uint_type, bound_constant, length],
    );
    instruction(&mut words, OP_TYPE_FUNCTION, &[function_type, void_type]);
    instruction(
        &mut words,
        OP_FUNCTION,
        &[void_type, function_id, FUNCTION_CONTROL_NONE, function_type],
    );
    instruction(&mut words, OP_LABEL, &[entry_label]);
    instruction(
        &mut words,
        OP_LOAD,
        &[vector_type, global_value, global_variable],
    );
    instruction(
        &mut words,
        OP_COMPOSITE_EXTRACT,
        &[uint_type, index, global_value, 0],
    );
    instruction(
        &mut words,
        OP_ULT,
        &[bool_type, in_bounds, index, bound_constant],
    );
    instruction(&mut words, OP_SELECTION_MERGE, &[merge_label, 0]);
    instruction(
        &mut words,
        OP_BRANCH_CONDITIONAL,
        &[in_bounds, body_label, merge_label],
    );
    instruction(&mut words, OP_LABEL, &[body_label]);
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            uint_element_pointer,
            element_address,
            resource_variable,
            zero,
            index,
        ],
    );
    instruction(&mut words, OP_STORE, &[element_address, constant_value]);
    instruction(&mut words, OP_BRANCH, &[merge_label]);
    instruction(&mut words, OP_LABEL, &[merge_label]);
    instruction(&mut words, OP_RETURN, &[]);
    instruction(&mut words, OP_FUNCTION_END, &[]);
    validate_spirv(&words)
        .map_err(|_| SpirvError::UnsupportedKernelShape("global-index write validation failed"))?;
    Ok(words)
}

/// Emits a bounds-safe strided global-index write kernel:
/// `buffer[index * stride] = value` when `index < length` and the physical
/// index is below `capacity`. Length, stride and capacity are read from three
/// reflected storage metadata resources at runtime.
pub fn emit_storage_global_index_strided_write(
    entry_name: &str,
    options: SpirvOptions,
    value: u32,
) -> Result<Vec<u32>, SpirvError> {
    if entry_name.is_empty() || entry_name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    let function_id = 1;
    let void_type = 2;
    let function_type = 3;
    let entry_label = 4;
    let uint_type = 5;
    let uint_array = 6;
    let uint_struct = 7;
    let storage_struct_pointer = 8;
    let uint_element_pointer = 9;
    let buffer_variable = 10;
    let length_variable = 11;
    let stride_variable = 12;
    let capacity_variable = 13;
    let vector_type = 14;
    let input_pointer = 15;
    let global_variable = 16;
    let global_value = 17;
    let index = 18;
    let zero = 19;
    let constant_value = 20;
    let length_address = 21;
    let length_value = 22;
    let stride_address = 23;
    let stride_value = 24;
    let capacity_address = 25;
    let capacity_value = 26;
    let bool_type = 27;
    let in_logical_bounds = 28;
    let logical_body_label = 29;
    let logical_merge_label = 30;
    let physical_index = 31;
    let in_capacity_bounds = 32;
    let physical_body_label = 33;
    let physical_merge_label = 34;
    let element_address = 35;
    let mut words = vec![SPIRV_MAGIC, SPIRV_VERSION_1_3, 0, 36, 0];
    instruction(&mut words, OP_CAPABILITY, &[CAPABILITY_SHADER]);
    instruction(
        &mut words,
        OP_MEMORY_MODEL,
        &[ADDRESSING_LOGICAL, MEMORY_MODEL_GLSL450],
    );
    instruction(
        &mut words,
        OP_DECORATE,
        &[uint_array, DECORATION_ARRAY_STRIDE, 4],
    );
    instruction(
        &mut words,
        OP_MEMBER_DECORATE,
        &[uint_struct, 0, DECORATION_OFFSET, 0],
    );
    instruction(&mut words, OP_DECORATE, &[uint_struct, DECORATION_BLOCK]);
    for (variable, binding) in [
        (buffer_variable, 0),
        (length_variable, 1),
        (stride_variable, 2),
        (capacity_variable, 3),
    ] {
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_DESCRIPTOR_SET, 0],
        );
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_BINDING, binding],
        );
    }
    instruction(
        &mut words,
        OP_DECORATE,
        &[
            global_variable,
            DECORATION_BUILT_IN,
            BUILT_IN_GLOBAL_INVOCATION_ID,
        ],
    );
    instruction_string_with_tail(
        &mut words,
        OP_ENTRY_POINT,
        &[EXECUTION_MODEL_GL_COMPUTE, function_id],
        entry_name,
        &[
            buffer_variable,
            length_variable,
            stride_variable,
            capacity_variable,
            global_variable,
        ],
    );
    instruction(
        &mut words,
        OP_EXECUTION_MODE,
        &[
            function_id,
            EXECUTION_MODE_LOCAL_SIZE,
            options.workgroup_size[0],
            options.workgroup_size[1],
            options.workgroup_size[2],
        ],
    );
    instruction(&mut words, OP_TYPE_VOID, &[void_type]);
    instruction(&mut words, OP_TYPE_INT, &[uint_type, 32, 0]);
    instruction(&mut words, OP_TYPE_RUNTIME_ARRAY, &[uint_array, uint_type]);
    instruction(&mut words, OP_TYPE_STRUCT, &[uint_struct, uint_array]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[storage_struct_pointer, STORAGE_BUFFER, uint_struct],
    );
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[uint_element_pointer, STORAGE_BUFFER, uint_type],
    );
    instruction(&mut words, OP_TYPE_VECTOR, &[vector_type, uint_type, 3]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[input_pointer, INPUT, vector_type],
    );
    for variable in [
        buffer_variable,
        length_variable,
        stride_variable,
        capacity_variable,
    ] {
        instruction(
            &mut words,
            OP_VARIABLE,
            &[storage_struct_pointer, variable, STORAGE_BUFFER],
        );
    }
    instruction(
        &mut words,
        OP_VARIABLE,
        &[input_pointer, global_variable, INPUT],
    );
    instruction(&mut words, OP_TYPE_BOOL, &[bool_type]);
    instruction(&mut words, OP_CONSTANT, &[uint_type, zero, 0]);
    instruction(&mut words, OP_CONSTANT, &[uint_type, constant_value, value]);
    instruction(&mut words, OP_TYPE_FUNCTION, &[function_type, void_type]);
    instruction(
        &mut words,
        OP_FUNCTION,
        &[void_type, function_id, FUNCTION_CONTROL_NONE, function_type],
    );
    instruction(&mut words, OP_LABEL, &[entry_label]);
    instruction(
        &mut words,
        OP_LOAD,
        &[vector_type, global_value, global_variable],
    );
    instruction(
        &mut words,
        OP_COMPOSITE_EXTRACT,
        &[uint_type, index, global_value, 0],
    );
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            uint_element_pointer,
            length_address,
            length_variable,
            zero,
            zero,
        ],
    );
    instruction(
        &mut words,
        OP_LOAD,
        &[uint_type, length_value, length_address],
    );
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            uint_element_pointer,
            stride_address,
            stride_variable,
            zero,
            zero,
        ],
    );
    instruction(
        &mut words,
        OP_LOAD,
        &[uint_type, stride_value, stride_address],
    );
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            uint_element_pointer,
            capacity_address,
            capacity_variable,
            zero,
            zero,
        ],
    );
    instruction(
        &mut words,
        OP_LOAD,
        &[uint_type, capacity_value, capacity_address],
    );
    instruction(
        &mut words,
        OP_ULT,
        &[bool_type, in_logical_bounds, index, length_value],
    );
    instruction(&mut words, OP_SELECTION_MERGE, &[logical_merge_label, 0]);
    instruction(
        &mut words,
        OP_BRANCH_CONDITIONAL,
        &[in_logical_bounds, logical_body_label, logical_merge_label],
    );
    instruction(&mut words, OP_LABEL, &[logical_body_label]);
    instruction(
        &mut words,
        OP_IMUL,
        &[uint_type, physical_index, index, stride_value],
    );
    instruction(
        &mut words,
        OP_ULT,
        &[
            bool_type,
            in_capacity_bounds,
            physical_index,
            capacity_value,
        ],
    );
    instruction(&mut words, OP_SELECTION_MERGE, &[physical_merge_label, 0]);
    instruction(
        &mut words,
        OP_BRANCH_CONDITIONAL,
        &[
            in_capacity_bounds,
            physical_body_label,
            physical_merge_label,
        ],
    );
    instruction(&mut words, OP_LABEL, &[physical_body_label]);
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            uint_element_pointer,
            element_address,
            buffer_variable,
            zero,
            physical_index,
        ],
    );
    instruction(&mut words, OP_STORE, &[element_address, constant_value]);
    instruction(&mut words, OP_BRANCH, &[physical_merge_label]);
    instruction(&mut words, OP_LABEL, &[physical_merge_label]);
    instruction(&mut words, OP_BRANCH, &[logical_merge_label]);
    instruction(&mut words, OP_LABEL, &[logical_merge_label]);
    instruction(&mut words, OP_RETURN, &[]);
    instruction(&mut words, OP_FUNCTION_END, &[]);
    validate_spirv(&words).map_err(|_| {
        SpirvError::UnsupportedKernelShape("strided global-index write validation failed")
    })?;
    Ok(words)
}

fn emit_storage_global_index_add_with_bound(
    entry_name: &str,
    options: SpirvOptions,
    addend: u32,
    bound: Option<u32>,
) -> Result<Vec<u32>, SpirvError> {
    if entry_name.is_empty() || entry_name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    let function_id = 1;
    let void_type = 2;
    let function_type = 3;
    let label = 4;
    let uint_type = 5;
    let uint_array = 6;
    let uint_struct = 7;
    let uint_struct_pointer = 8;
    let uint_element_pointer = 9;
    let input_variable = 10;
    let output_variable = 11;
    let vector_type = 12;
    let input_pointer = 13;
    let global_variable = 14;
    let global_value = 15;
    let index = 16;
    let zero = 17;
    let constant_addend = 18;
    let input_address = 19;
    let output_address = 20;
    let loaded_input = 21;
    let sum = 22;
    let bool_type = 23;
    let bound_constant = 24;
    let in_bounds = 25;
    let body_label = 26;
    let merge_label = 27;
    let id_bound = if bound.is_some() { 28 } else { 23 };
    let mut words = vec![SPIRV_MAGIC, SPIRV_VERSION_1_3, 0, id_bound, 0];
    instruction(&mut words, OP_CAPABILITY, &[CAPABILITY_SHADER]);
    instruction(
        &mut words,
        OP_MEMORY_MODEL,
        &[ADDRESSING_LOGICAL, MEMORY_MODEL_GLSL450],
    );
    instruction(
        &mut words,
        OP_DECORATE,
        &[uint_array, DECORATION_ARRAY_STRIDE, 4],
    );
    instruction(
        &mut words,
        OP_MEMBER_DECORATE,
        &[uint_struct, 0, DECORATION_OFFSET, 0],
    );
    instruction(&mut words, OP_DECORATE, &[uint_struct, DECORATION_BLOCK]);
    instruction(
        &mut words,
        OP_DECORATE,
        &[
            global_variable,
            DECORATION_BUILT_IN,
            BUILT_IN_GLOBAL_INVOCATION_ID,
        ],
    );
    for (variable, binding) in [(input_variable, 0), (output_variable, 1)] {
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_DESCRIPTOR_SET, 0],
        );
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_BINDING, binding],
        );
    }
    instruction_string_with_tail(
        &mut words,
        OP_ENTRY_POINT,
        &[EXECUTION_MODEL_GL_COMPUTE, function_id],
        entry_name,
        &[input_variable, output_variable, global_variable],
    );
    instruction(
        &mut words,
        OP_EXECUTION_MODE,
        &[
            function_id,
            EXECUTION_MODE_LOCAL_SIZE,
            options.workgroup_size[0],
            options.workgroup_size[1],
            options.workgroup_size[2],
        ],
    );
    instruction(&mut words, OP_TYPE_VOID, &[void_type]);
    instruction(&mut words, OP_TYPE_INT, &[uint_type, 32, 0]);
    instruction(&mut words, OP_TYPE_RUNTIME_ARRAY, &[uint_array, uint_type]);
    instruction(&mut words, OP_TYPE_STRUCT, &[uint_struct, uint_array]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[uint_struct_pointer, STORAGE_BUFFER, uint_struct],
    );
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[uint_element_pointer, STORAGE_BUFFER, uint_type],
    );
    instruction(&mut words, OP_TYPE_VECTOR, &[vector_type, uint_type, 3]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[input_pointer, INPUT, vector_type],
    );
    instruction(
        &mut words,
        OP_VARIABLE,
        &[uint_struct_pointer, input_variable, STORAGE_BUFFER],
    );
    instruction(
        &mut words,
        OP_VARIABLE,
        &[uint_struct_pointer, output_variable, STORAGE_BUFFER],
    );
    instruction(
        &mut words,
        OP_VARIABLE,
        &[input_pointer, global_variable, INPUT],
    );
    instruction(&mut words, OP_CONSTANT, &[uint_type, zero, 0]);
    instruction(
        &mut words,
        OP_CONSTANT,
        &[uint_type, constant_addend, addend],
    );
    if let Some(length) = bound {
        instruction(&mut words, OP_TYPE_BOOL, &[bool_type]);
        instruction(
            &mut words,
            OP_CONSTANT,
            &[uint_type, bound_constant, length],
        );
    }
    instruction(&mut words, OP_TYPE_FUNCTION, &[function_type, void_type]);
    instruction(
        &mut words,
        OP_FUNCTION,
        &[void_type, function_id, FUNCTION_CONTROL_NONE, function_type],
    );
    instruction(&mut words, OP_LABEL, &[label]);
    instruction(
        &mut words,
        OP_LOAD,
        &[vector_type, global_value, global_variable],
    );
    instruction(
        &mut words,
        OP_COMPOSITE_EXTRACT,
        &[uint_type, index, global_value, 0],
    );
    if bound.is_some() {
        instruction(
            &mut words,
            OP_ULT,
            &[bool_type, in_bounds, index, bound_constant],
        );
        instruction(&mut words, OP_SELECTION_MERGE, &[merge_label, 0]);
        instruction(
            &mut words,
            OP_BRANCH_CONDITIONAL,
            &[in_bounds, body_label, merge_label],
        );
        instruction(&mut words, OP_LABEL, &[body_label]);
    }
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            uint_element_pointer,
            input_address,
            input_variable,
            zero,
            index,
        ],
    );
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            uint_element_pointer,
            output_address,
            output_variable,
            zero,
            index,
        ],
    );
    instruction(
        &mut words,
        OP_LOAD,
        &[uint_type, loaded_input, input_address],
    );
    instruction(
        &mut words,
        OP_IADD,
        &[uint_type, sum, loaded_input, constant_addend],
    );
    instruction(&mut words, OP_STORE, &[output_address, sum]);
    if bound.is_some() {
        instruction(&mut words, OP_BRANCH, &[merge_label]);
        instruction(&mut words, OP_LABEL, &[merge_label]);
    }
    instruction(&mut words, OP_RETURN, &[]);
    instruction(&mut words, OP_FUNCTION_END, &[]);
    validate_spirv(&words)
        .map_err(|_| SpirvError::UnsupportedKernelShape("global-index validation failed"))?;
    Ok(words)
}

/// Lowers the bounded global-index array shape from JIR:
/// `idx = builtin global_invocation_id.x; bounds_check idx, length;
/// output[idx] = input[idx] + addend`.
pub fn emit_storage_global_index_add_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<Vec<u32>, SpirvError> {
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(SpirvError::GpuVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    if function_data.name.is_empty() || function_data.name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) || function_data.parameters.len() != 2
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index add requires two resource parameters and Unit result",
        ));
    }
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    if resources.len() != 2
        || resources
            .iter()
            .any(|resource| resource.address_space != AddressSpace::Storage)
        || resources.iter().any(|resource| {
            !matches!(
                module.types.get(resource.element_type.index()),
                Some(Type::Integer {
                    signed: false,
                    bits: 32
                })
            )
        })
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index add requires two storage u32 resources",
        ));
    }
    let Some(block) = function_data.blocks.first() else {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index add requires one entry block",
        ));
    };
    if function_data.blocks.len() != 1
        || !block.parameters.is_empty()
        || block.instructions.len() != 9
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index add body must contain builtin, constants, bounds check, offsets, load, add and store",
        ));
    }
    let u32_type = resources[0].element_type;
    let builtin = &block.instructions[0];
    let Some(builtin_result) = builtin.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index builtin must produce a value",
        ));
    };
    if !matches!(
        builtin.kind,
        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX)
    ) || builtin_result.ty != u32_type
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index first instruction must produce u32 GlobalInvocationId.x",
        ));
    }
    let constant = &block.instructions[1];
    let Some(constant_result) = constant.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index addend constant must produce a value",
        ));
    };
    let addend = match (&constant.kind, constant_result.ty == u32_type) {
        (InstructionKind::Constant(Constant::Integer { value }), true) => u32::try_from(*value)
            .map_err(|_| {
                SpirvError::UnsupportedKernelShape("global-index addend is outside u32")
            })?,
        _ => {
            return Err(SpirvError::UnsupportedKernelShape(
                "global-index second instruction must be a u32 constant",
            ));
        }
    };
    let length_instruction = &block.instructions[2];
    let Some(length_result) = length_instruction.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index length constant must produce a value",
        ));
    };
    let length = match (&length_instruction.kind, length_result.ty == u32_type) {
        (InstructionKind::Constant(Constant::Integer { value }), true) => u32::try_from(*value)
            .map_err(|_| {
                SpirvError::UnsupportedKernelShape("global-index length is outside u32")
            })?,
        _ => {
            return Err(SpirvError::UnsupportedKernelShape(
                "global-index third instruction must be a u32 length constant",
            ));
        }
    };
    if length == 0 {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index length must be non-zero",
        ));
    }
    let bounds = &block.instructions[3];
    if bounds.result.is_some()
        || !matches!(
            bounds.kind,
            InstructionKind::BoundsCheck { index, length: bound_length }
                if index == builtin_result.value && bound_length == length_result.value
        )
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index fourth instruction must bounds-check the builtin index",
        ));
    }
    let input_offset = &block.instructions[4];
    let Some(input_offset_result) = input_offset.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index input offset must produce a pointer",
        ));
    };
    if !matches!(
        &input_offset.kind,
        InstructionKind::Offset { base, indices }
            if *base == function_data.parameters[0].value
                && indices == &[builtin_result.value]
                && input_offset_result.ty == function_data.parameters[0].ty
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index input offset is invalid",
        ));
    }
    let input_load = &block.instructions[5];
    let Some(input_result) = input_load.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index input load must produce a value",
        ));
    };
    if !matches!(
        &input_load.kind,
        InstructionKind::Load { pointer, .. }
            if *pointer == input_offset_result.value && input_result.ty == u32_type
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index input load is invalid",
        ));
    }
    let binary = &block.instructions[6];
    let Some(binary_result) = binary.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index add must produce a value",
        ));
    };
    if !matches!(
        &binary.kind,
        InstructionKind::Binary { op, left, right }
            if *op == BinaryOp::Add
                && binary_result.ty == u32_type
                && ((*left == input_result.value && *right == constant_result.value)
                    || (*right == input_result.value && *left == constant_result.value))
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index add operands are invalid",
        ));
    }
    let output_offset = &block.instructions[7];
    let Some(output_offset_result) = output_offset.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index output offset must produce a pointer",
        ));
    };
    if !matches!(
        &output_offset.kind,
        InstructionKind::Offset { base, indices }
            if *base == function_data.parameters[1].value
                && indices == &[builtin_result.value]
                && output_offset_result.ty == function_data.parameters[1].ty
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index output offset is invalid",
        ));
    }
    let store = &block.instructions[8];
    if store.result.is_some()
        || !matches!(
            &store.kind,
            InstructionKind::Store { pointer, value, .. }
                if *pointer == output_offset_result.value && *value == binary_result.value
        )
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index store is invalid",
        ));
    }
    if !matches!(block.terminator, Terminator::Return { value: None }) {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index add must return Unit",
        ));
    }
    emit_storage_global_index_add_bounded(&function_data.name, options, addend, length)
}

/// Lowers the bounds-safe one-resource JIR shape:
/// `idx = builtin global_invocation_id.x; bounds_check idx, length;
/// buffer[idx] = value`.
pub fn emit_storage_global_index_write_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<Vec<u32>, SpirvError> {
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(SpirvError::GpuVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    if function_data.name.is_empty() || function_data.name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) || function_data.parameters.len() != 1
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index write requires one resource parameter and Unit result",
        ));
    }
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    let Some(resource) = resources.first() else {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index write requires one reflected resource",
        ));
    };
    if resources.len() != 1
        || resource.address_space != AddressSpace::Storage
        || !matches!(
            module.types.get(resource.element_type.index()),
            Some(Type::Integer {
                signed: false,
                bits: 32
            })
        )
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index write requires one storage u32 resource",
        ));
    }
    let Some(block) = function_data.blocks.first() else {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index write requires one entry block",
        ));
    };
    if function_data.blocks.len() != 1
        || !block.parameters.is_empty()
        || block.instructions.len() != 6
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index write body must contain builtin, constants, bounds, offset and store",
        ));
    }
    let u32_type = resource.element_type;
    let builtin = &block.instructions[0];
    let Some(builtin_result) = builtin.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index write builtin must produce a value",
        ));
    };
    if !matches!(
        &builtin.kind,
        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX)
    ) || builtin_result.ty != u32_type
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index write first instruction must produce u32 GlobalInvocationId.x",
        ));
    }
    let value_instruction = &block.instructions[1];
    let Some(value_result) = value_instruction.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index write value constant must produce a value",
        ));
    };
    let value = match (&value_instruction.kind, value_result.ty == u32_type) {
        (InstructionKind::Constant(Constant::Integer { value }), true) => u32::try_from(*value)
            .map_err(|_| {
                SpirvError::UnsupportedKernelShape("global-index write value is outside u32")
            })?,
        _ => {
            return Err(SpirvError::UnsupportedKernelShape(
                "global-index write second instruction must be a u32 constant",
            ));
        }
    };
    let length_instruction = &block.instructions[2];
    let Some(length_result) = length_instruction.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index write length constant must produce a value",
        ));
    };
    let length = match (&length_instruction.kind, length_result.ty == u32_type) {
        (InstructionKind::Constant(Constant::Integer { value }), true) => u32::try_from(*value)
            .map_err(|_| {
                SpirvError::UnsupportedKernelShape("global-index write length is outside u32")
            })?,
        _ => {
            return Err(SpirvError::UnsupportedKernelShape(
                "global-index write third instruction must be a u32 length constant",
            ));
        }
    };
    if length == 0 {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index write length must be non-zero",
        ));
    }
    let bounds = &block.instructions[3];
    if bounds.result.is_some()
        || !matches!(
            &bounds.kind,
            InstructionKind::BoundsCheck { index, length: bound_length }
                if *index == builtin_result.value && *bound_length == length_result.value
        )
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index write fourth instruction must bounds-check the builtin index",
        ));
    }
    let offset = &block.instructions[4];
    let Some(offset_result) = offset.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index write offset must produce a pointer",
        ));
    };
    if !matches!(
        &offset.kind,
        InstructionKind::Offset { base, indices }
            if *base == function_data.parameters[0].value
                && indices == &[builtin_result.value]
                && offset_result.ty == function_data.parameters[0].ty
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index write offset is invalid",
        ));
    }
    let store = &block.instructions[5];
    if store.result.is_some()
        || !matches!(
            &store.kind,
            InstructionKind::Store { pointer, value: stored_value, .. }
                if *pointer == offset_result.value && *stored_value == value_result.value
        )
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index write store is invalid",
        ));
    }
    if !matches!(block.terminator, Terminator::Return { value: None }) {
        return Err(SpirvError::UnsupportedKernelShape(
            "global-index write must return Unit",
        ));
    }
    let mut words = emit_storage_global_index_write(&function_data.name, options, value, length)?;
    apply_reflected_resource_access_decorations(&mut words, &resources)?;
    Ok(words)
}

/// Lowers the bounds-safe strided JIR shape:
/// `idx = builtin global_invocation_id.x; length = load length[0];
/// stride = load stride[0]; capacity = load capacity[0];
/// bounds_check idx, length; physical = idx * stride;
/// bounds_check physical, capacity; buffer[physical] = value`.
pub fn emit_storage_global_index_strided_write_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<Vec<u32>, SpirvError> {
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(SpirvError::GpuVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    if function_data.name.is_empty() || function_data.name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) || function_data.parameters.len() != 4
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "strided global-index write requires four resource parameters and Unit result",
        ));
    }
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    if resources.len() != 4
        || resources
            .iter()
            .any(|resource| resource.address_space != AddressSpace::Storage)
        || resources.iter().any(|resource| {
            !matches!(
                module.types.get(resource.element_type.index()),
                Some(Type::Integer {
                    signed: false,
                    bits: 32
                })
            )
        })
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "strided global-index write requires four storage u32 resources",
        ));
    }
    let Some(block) = function_data.blocks.first() else {
        return Err(SpirvError::UnsupportedKernelShape(
            "strided global-index write requires one entry block",
        ));
    };
    if function_data.blocks.len() != 1
        || !block.parameters.is_empty()
        || block.instructions.len() != 10
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "strided global-index write body has an unsupported shape",
        ));
    }
    let u32_type = resources[0].element_type;
    let builtin = &block.instructions[0];
    let Some(builtin_result) = builtin.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "strided global-index builtin must produce a value",
        ));
    };
    if !matches!(
        &builtin.kind,
        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX)
    ) || builtin_result.ty != u32_type
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "strided global-index first instruction must produce u32 GlobalInvocationId.x",
        ));
    }
    let value_instruction = &block.instructions[1];
    let Some(value_result) = value_instruction.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "strided global-index value constant must produce a value",
        ));
    };
    let value = match (&value_instruction.kind, value_result.ty == u32_type) {
        (InstructionKind::Constant(Constant::Integer { value }), true) => u32::try_from(*value)
            .map_err(|_| {
                SpirvError::UnsupportedKernelShape("strided global-index value is outside u32")
            })?,
        _ => {
            return Err(SpirvError::UnsupportedKernelShape(
                "strided global-index second instruction must be a u32 constant",
            ));
        }
    };
    let length_load = &block.instructions[2];
    let Some(length_result) = length_load.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "strided global-index length load must produce a value",
        ));
    };
    if !matches!(
        &length_load.kind,
        InstructionKind::Load { pointer, .. }
            if *pointer == function_data.parameters[1].value && length_result.ty == u32_type
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "strided global-index third instruction must load length resource",
        ));
    }
    let stride_load = &block.instructions[3];
    let Some(stride_result) = stride_load.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "strided global-index stride load must produce a value",
        ));
    };
    if !matches!(
        &stride_load.kind,
        InstructionKind::Load { pointer, .. }
            if *pointer == function_data.parameters[2].value && stride_result.ty == u32_type
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "strided global-index fourth instruction must load stride resource",
        ));
    }
    let capacity_load = &block.instructions[4];
    let Some(capacity_result) = capacity_load.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "strided global-index capacity load must produce a value",
        ));
    };
    if !matches!(
        &capacity_load.kind,
        InstructionKind::Load { pointer, .. }
            if *pointer == function_data.parameters[3].value && capacity_result.ty == u32_type
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "strided global-index fifth instruction must load capacity resource",
        ));
    }
    let logical_bounds = &block.instructions[5];
    if logical_bounds.result.is_some()
        || !matches!(
            &logical_bounds.kind,
            InstructionKind::BoundsCheck { index, length }
                if *index == builtin_result.value && *length == length_result.value
        )
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "strided global-index sixth instruction must bounds-check logical index",
        ));
    }
    let physical = &block.instructions[6];
    let Some(physical_result) = physical.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "strided global-index multiply must produce a value",
        ));
    };
    if !matches!(
        &physical.kind,
        InstructionKind::Binary { op, left, right }
            if *op == BinaryOp::Multiply
                && physical_result.ty == u32_type
                && ((*left == builtin_result.value && *right == stride_result.value)
                    || (*right == builtin_result.value && *left == stride_result.value))
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "strided global-index seventh instruction must multiply index by stride",
        ));
    }
    let physical_bounds = &block.instructions[7];
    if physical_bounds.result.is_some()
        || !matches!(
            &physical_bounds.kind,
            InstructionKind::BoundsCheck { index, length }
                if *index == physical_result.value && *length == capacity_result.value
        )
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "strided global-index eighth instruction must bounds-check physical index",
        ));
    }
    let offset = &block.instructions[8];
    let Some(offset_result) = offset.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "strided global-index offset must produce a pointer",
        ));
    };
    if !matches!(
        &offset.kind,
        InstructionKind::Offset { base, indices }
            if *base == function_data.parameters[0].value
                && indices == &[physical_result.value]
                && offset_result.ty == function_data.parameters[0].ty
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "strided global-index offset is invalid",
        ));
    }
    let store = &block.instructions[9];
    if store.result.is_some()
        || !matches!(
            &store.kind,
            InstructionKind::Store { pointer, value: stored_value, .. }
                if *pointer == offset_result.value && *stored_value == value_result.value
        )
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "strided global-index store is invalid",
        ));
    }
    if !matches!(block.terminator, Terminator::Return { value: None }) {
        return Err(SpirvError::UnsupportedKernelShape(
            "strided global-index write must return Unit",
        ));
    }
    let mut words = emit_storage_global_index_strided_write(&function_data.name, options, value)?;
    apply_reflected_resource_access_decorations(&mut words, &resources)?;
    Ok(words)
}

/// Emits a bounds-safe two-dimensional global-index write kernel.
///
/// The logical row-major index is `y * width + x`. Runtime metadata resources
/// provide `width[0]`, `height[0]` and `capacity[0]`; the write is performed
/// only when both coordinates are in bounds and the flattened index is below
/// the physical capacity.
pub fn emit_storage_global_2d_write(
    entry_name: &str,
    options: SpirvOptions,
    value: u32,
) -> Result<Vec<u32>, SpirvError> {
    if entry_name.is_empty() || entry_name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    let function_id = 1;
    let void_type = 2;
    let function_type = 3;
    let entry_label = 4;
    let uint_type = 5;
    let uint_array = 6;
    let uint_struct = 7;
    let storage_struct_pointer = 8;
    let uint_element_pointer = 9;
    let buffer_variable = 10;
    let width_variable = 11;
    let height_variable = 12;
    let capacity_variable = 13;
    let vector_type = 14;
    let input_pointer = 15;
    let global_variable = 16;
    let global_value = 17;
    let x_index = 18;
    let y_index = 19;
    let zero = 20;
    let constant_value = 21;
    let width_address = 22;
    let width_value = 23;
    let height_address = 24;
    let height_value = 25;
    let capacity_address = 26;
    let capacity_value = 27;
    let bool_type = 28;
    let x_in_bounds = 29;
    let y_in_bounds = 30;
    let logical_in_bounds = 31;
    let row_index = 32;
    let logical_index = 33;
    let physical_in_bounds = 34;
    let logical_body_label = 35;
    let logical_merge_label = 36;
    let physical_body_label = 37;
    let physical_merge_label = 38;
    let element_address = 39;
    let mut words = vec![SPIRV_MAGIC, SPIRV_VERSION_1_3, 0, 40, 0];
    instruction(&mut words, OP_CAPABILITY, &[CAPABILITY_SHADER]);
    instruction(
        &mut words,
        OP_MEMORY_MODEL,
        &[ADDRESSING_LOGICAL, MEMORY_MODEL_GLSL450],
    );
    instruction(
        &mut words,
        OP_DECORATE,
        &[uint_array, DECORATION_ARRAY_STRIDE, 4],
    );
    instruction(
        &mut words,
        OP_MEMBER_DECORATE,
        &[uint_struct, 0, DECORATION_OFFSET, 0],
    );
    instruction(&mut words, OP_DECORATE, &[uint_struct, DECORATION_BLOCK]);
    for (variable, binding) in [
        (buffer_variable, 0),
        (width_variable, 1),
        (height_variable, 2),
        (capacity_variable, 3),
    ] {
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_DESCRIPTOR_SET, 0],
        );
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_BINDING, binding],
        );
    }
    instruction(
        &mut words,
        OP_DECORATE,
        &[
            global_variable,
            DECORATION_BUILT_IN,
            BUILT_IN_GLOBAL_INVOCATION_ID,
        ],
    );
    instruction_string_with_tail(
        &mut words,
        OP_ENTRY_POINT,
        &[EXECUTION_MODEL_GL_COMPUTE, function_id],
        entry_name,
        &[
            buffer_variable,
            width_variable,
            height_variable,
            capacity_variable,
            global_variable,
        ],
    );
    instruction(
        &mut words,
        OP_EXECUTION_MODE,
        &[
            function_id,
            EXECUTION_MODE_LOCAL_SIZE,
            options.workgroup_size[0],
            options.workgroup_size[1],
            options.workgroup_size[2],
        ],
    );
    instruction(&mut words, OP_TYPE_VOID, &[void_type]);
    instruction(&mut words, OP_TYPE_INT, &[uint_type, 32, 0]);
    instruction(&mut words, OP_TYPE_RUNTIME_ARRAY, &[uint_array, uint_type]);
    instruction(&mut words, OP_TYPE_STRUCT, &[uint_struct, uint_array]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[storage_struct_pointer, STORAGE_BUFFER, uint_struct],
    );
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[uint_element_pointer, STORAGE_BUFFER, uint_type],
    );
    instruction(&mut words, OP_TYPE_VECTOR, &[vector_type, uint_type, 3]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[input_pointer, INPUT, vector_type],
    );
    for variable in [
        buffer_variable,
        width_variable,
        height_variable,
        capacity_variable,
    ] {
        instruction(
            &mut words,
            OP_VARIABLE,
            &[storage_struct_pointer, variable, STORAGE_BUFFER],
        );
    }
    instruction(
        &mut words,
        OP_VARIABLE,
        &[input_pointer, global_variable, INPUT],
    );
    instruction(&mut words, OP_TYPE_BOOL, &[bool_type]);
    instruction(&mut words, OP_CONSTANT, &[uint_type, zero, 0]);
    instruction(&mut words, OP_CONSTANT, &[uint_type, constant_value, value]);
    instruction(&mut words, OP_TYPE_FUNCTION, &[function_type, void_type]);
    instruction(
        &mut words,
        OP_FUNCTION,
        &[void_type, function_id, FUNCTION_CONTROL_NONE, function_type],
    );
    instruction(&mut words, OP_LABEL, &[entry_label]);
    instruction(
        &mut words,
        OP_LOAD,
        &[vector_type, global_value, global_variable],
    );
    instruction(
        &mut words,
        OP_COMPOSITE_EXTRACT,
        &[uint_type, x_index, global_value, 0],
    );
    instruction(
        &mut words,
        OP_COMPOSITE_EXTRACT,
        &[uint_type, y_index, global_value, 1],
    );
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            uint_element_pointer,
            width_address,
            width_variable,
            zero,
            zero,
        ],
    );
    instruction(
        &mut words,
        OP_LOAD,
        &[uint_type, width_value, width_address],
    );
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            uint_element_pointer,
            height_address,
            height_variable,
            zero,
            zero,
        ],
    );
    instruction(
        &mut words,
        OP_LOAD,
        &[uint_type, height_value, height_address],
    );
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            uint_element_pointer,
            capacity_address,
            capacity_variable,
            zero,
            zero,
        ],
    );
    instruction(
        &mut words,
        OP_LOAD,
        &[uint_type, capacity_value, capacity_address],
    );
    instruction(
        &mut words,
        OP_ULT,
        &[bool_type, x_in_bounds, x_index, width_value],
    );
    instruction(
        &mut words,
        OP_ULT,
        &[bool_type, y_in_bounds, y_index, height_value],
    );
    instruction(
        &mut words,
        OP_LOGICAL_AND,
        &[bool_type, logical_in_bounds, x_in_bounds, y_in_bounds],
    );
    instruction(&mut words, OP_SELECTION_MERGE, &[logical_merge_label, 0]);
    instruction(
        &mut words,
        OP_BRANCH_CONDITIONAL,
        &[logical_in_bounds, logical_body_label, logical_merge_label],
    );
    instruction(&mut words, OP_LABEL, &[logical_body_label]);
    instruction(
        &mut words,
        OP_IMUL,
        &[uint_type, row_index, y_index, width_value],
    );
    instruction(
        &mut words,
        OP_IADD,
        &[uint_type, logical_index, row_index, x_index],
    );
    instruction(
        &mut words,
        OP_ULT,
        &[bool_type, physical_in_bounds, logical_index, capacity_value],
    );
    instruction(&mut words, OP_SELECTION_MERGE, &[physical_merge_label, 0]);
    instruction(
        &mut words,
        OP_BRANCH_CONDITIONAL,
        &[
            physical_in_bounds,
            physical_body_label,
            physical_merge_label,
        ],
    );
    instruction(&mut words, OP_LABEL, &[physical_body_label]);
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            uint_element_pointer,
            element_address,
            buffer_variable,
            zero,
            logical_index,
        ],
    );
    instruction(&mut words, OP_STORE, &[element_address, constant_value]);
    instruction(&mut words, OP_BRANCH, &[physical_merge_label]);
    instruction(&mut words, OP_LABEL, &[physical_merge_label]);
    instruction(&mut words, OP_BRANCH, &[logical_merge_label]);
    instruction(&mut words, OP_LABEL, &[logical_merge_label]);
    instruction(&mut words, OP_RETURN, &[]);
    instruction(&mut words, OP_FUNCTION_END, &[]);
    validate_spirv(&words)
        .map_err(|_| SpirvError::UnsupportedKernelShape("2d global-index validation failed"))?;
    Ok(words)
}

/// Emits a bounds-safe two-dimensional affine-stride global-index write kernel.
///
/// The physical index is `x * stride_x + y * stride_y`. Runtime metadata
/// resources provide the logical width/height, both element strides and the
/// physical capacity. Coordinates are checked before arithmetic and the final
/// physical index is checked before the store.
pub fn emit_storage_global_2d_strided_write(
    entry_name: &str,
    options: SpirvOptions,
    value: u32,
) -> Result<Vec<u32>, SpirvError> {
    if entry_name.is_empty() || entry_name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    let function_id = 1;
    let void_type = 2;
    let function_type = 3;
    let entry_label = 4;
    let uint_type = 5;
    let uint_array = 6;
    let uint_struct = 7;
    let storage_struct_pointer = 8;
    let uint_element_pointer = 9;
    let buffer_variable = 10;
    let width_variable = 11;
    let height_variable = 12;
    let stride_x_variable = 13;
    let stride_y_variable = 14;
    let capacity_variable = 15;
    let vector_type = 16;
    let input_pointer = 17;
    let global_variable = 18;
    let global_value = 19;
    let x_index = 20;
    let y_index = 21;
    let zero = 22;
    let constant_value = 23;
    let width_address = 24;
    let width_value = 25;
    let height_address = 26;
    let height_value = 27;
    let stride_x_address = 28;
    let stride_x_value = 29;
    let stride_y_address = 30;
    let stride_y_value = 31;
    let capacity_address = 32;
    let capacity_value = 33;
    let bool_type = 34;
    let x_in_bounds = 35;
    let y_in_bounds = 36;
    let logical_in_bounds = 37;
    let x_offset = 38;
    let y_offset = 39;
    let physical_index = 40;
    let physical_in_bounds = 41;
    let logical_body_label = 42;
    let logical_merge_label = 43;
    let physical_body_label = 44;
    let physical_merge_label = 45;
    let element_address = 46;
    let mut words = vec![SPIRV_MAGIC, SPIRV_VERSION_1_3, 0, 47, 0];
    instruction(&mut words, OP_CAPABILITY, &[CAPABILITY_SHADER]);
    instruction(
        &mut words,
        OP_MEMORY_MODEL,
        &[ADDRESSING_LOGICAL, MEMORY_MODEL_GLSL450],
    );
    instruction(
        &mut words,
        OP_DECORATE,
        &[uint_array, DECORATION_ARRAY_STRIDE, 4],
    );
    instruction(
        &mut words,
        OP_MEMBER_DECORATE,
        &[uint_struct, 0, DECORATION_OFFSET, 0],
    );
    instruction(&mut words, OP_DECORATE, &[uint_struct, DECORATION_BLOCK]);
    for (variable, binding) in [
        (buffer_variable, 0),
        (width_variable, 1),
        (height_variable, 2),
        (stride_x_variable, 3),
        (stride_y_variable, 4),
        (capacity_variable, 5),
    ] {
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_DESCRIPTOR_SET, 0],
        );
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_BINDING, binding],
        );
    }
    instruction(
        &mut words,
        OP_DECORATE,
        &[
            global_variable,
            DECORATION_BUILT_IN,
            BUILT_IN_GLOBAL_INVOCATION_ID,
        ],
    );
    instruction_string_with_tail(
        &mut words,
        OP_ENTRY_POINT,
        &[EXECUTION_MODEL_GL_COMPUTE, function_id],
        entry_name,
        &[
            buffer_variable,
            width_variable,
            height_variable,
            stride_x_variable,
            stride_y_variable,
            capacity_variable,
            global_variable,
        ],
    );
    instruction(
        &mut words,
        OP_EXECUTION_MODE,
        &[
            function_id,
            EXECUTION_MODE_LOCAL_SIZE,
            options.workgroup_size[0],
            options.workgroup_size[1],
            options.workgroup_size[2],
        ],
    );
    instruction(&mut words, OP_TYPE_VOID, &[void_type]);
    instruction(&mut words, OP_TYPE_INT, &[uint_type, 32, 0]);
    instruction(&mut words, OP_TYPE_RUNTIME_ARRAY, &[uint_array, uint_type]);
    instruction(&mut words, OP_TYPE_STRUCT, &[uint_struct, uint_array]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[storage_struct_pointer, STORAGE_BUFFER, uint_struct],
    );
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[uint_element_pointer, STORAGE_BUFFER, uint_type],
    );
    instruction(&mut words, OP_TYPE_VECTOR, &[vector_type, uint_type, 3]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[input_pointer, INPUT, vector_type],
    );
    for variable in [
        buffer_variable,
        width_variable,
        height_variable,
        stride_x_variable,
        stride_y_variable,
        capacity_variable,
    ] {
        instruction(
            &mut words,
            OP_VARIABLE,
            &[storage_struct_pointer, variable, STORAGE_BUFFER],
        );
    }
    instruction(
        &mut words,
        OP_VARIABLE,
        &[input_pointer, global_variable, INPUT],
    );
    instruction(&mut words, OP_TYPE_BOOL, &[bool_type]);
    instruction(&mut words, OP_CONSTANT, &[uint_type, zero, 0]);
    instruction(&mut words, OP_CONSTANT, &[uint_type, constant_value, value]);
    instruction(&mut words, OP_TYPE_FUNCTION, &[function_type, void_type]);
    instruction(
        &mut words,
        OP_FUNCTION,
        &[void_type, function_id, FUNCTION_CONTROL_NONE, function_type],
    );
    instruction(&mut words, OP_LABEL, &[entry_label]);
    instruction(
        &mut words,
        OP_LOAD,
        &[vector_type, global_value, global_variable],
    );
    instruction(
        &mut words,
        OP_COMPOSITE_EXTRACT,
        &[uint_type, x_index, global_value, 0],
    );
    instruction(
        &mut words,
        OP_COMPOSITE_EXTRACT,
        &[uint_type, y_index, global_value, 1],
    );
    for (variable, address, result) in [
        (width_variable, width_address, width_value),
        (height_variable, height_address, height_value),
        (stride_x_variable, stride_x_address, stride_x_value),
        (stride_y_variable, stride_y_address, stride_y_value),
        (capacity_variable, capacity_address, capacity_value),
    ] {
        instruction(
            &mut words,
            OP_ACCESS_CHAIN,
            &[uint_element_pointer, address, variable, zero, zero],
        );
        instruction(&mut words, OP_LOAD, &[uint_type, result, address]);
    }
    instruction(
        &mut words,
        OP_ULT,
        &[bool_type, x_in_bounds, x_index, width_value],
    );
    instruction(
        &mut words,
        OP_ULT,
        &[bool_type, y_in_bounds, y_index, height_value],
    );
    instruction(
        &mut words,
        OP_LOGICAL_AND,
        &[bool_type, logical_in_bounds, x_in_bounds, y_in_bounds],
    );
    instruction(&mut words, OP_SELECTION_MERGE, &[logical_merge_label, 0]);
    instruction(
        &mut words,
        OP_BRANCH_CONDITIONAL,
        &[logical_in_bounds, logical_body_label, logical_merge_label],
    );
    instruction(&mut words, OP_LABEL, &[logical_body_label]);
    instruction(
        &mut words,
        OP_IMUL,
        &[uint_type, x_offset, x_index, stride_x_value],
    );
    instruction(
        &mut words,
        OP_IMUL,
        &[uint_type, y_offset, y_index, stride_y_value],
    );
    instruction(
        &mut words,
        OP_IADD,
        &[uint_type, physical_index, x_offset, y_offset],
    );
    instruction(
        &mut words,
        OP_ULT,
        &[
            bool_type,
            physical_in_bounds,
            physical_index,
            capacity_value,
        ],
    );
    instruction(&mut words, OP_SELECTION_MERGE, &[physical_merge_label, 0]);
    instruction(
        &mut words,
        OP_BRANCH_CONDITIONAL,
        &[
            physical_in_bounds,
            physical_body_label,
            physical_merge_label,
        ],
    );
    instruction(&mut words, OP_LABEL, &[physical_body_label]);
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            uint_element_pointer,
            element_address,
            buffer_variable,
            zero,
            physical_index,
        ],
    );
    instruction(&mut words, OP_STORE, &[element_address, constant_value]);
    instruction(&mut words, OP_BRANCH, &[physical_merge_label]);
    instruction(&mut words, OP_LABEL, &[physical_merge_label]);
    instruction(&mut words, OP_BRANCH, &[logical_merge_label]);
    instruction(&mut words, OP_LABEL, &[logical_merge_label]);
    instruction(&mut words, OP_RETURN, &[]);
    instruction(&mut words, OP_FUNCTION_END, &[]);
    validate_spirv(&words).map_err(|_| {
        SpirvError::UnsupportedKernelShape("2d strided global-index validation failed")
    })?;
    Ok(words)
}

/// Emits a bounds-safe three-dimensional affine-stride global-index write kernel.
///
/// The physical index is `x * stride_x + y * stride_y + z * stride_z`.
/// Runtime metadata resources provide logical dimensions, element strides and
/// physical capacity. Coordinates are checked before arithmetic and the final
/// physical index is checked before the store.
pub fn emit_storage_global_3d_strided_write(
    entry_name: &str,
    options: SpirvOptions,
    value: u32,
) -> Result<Vec<u32>, SpirvError> {
    if entry_name.is_empty() || entry_name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    let function_id = 1;
    let void_type = 2;
    let function_type = 3;
    let entry_label = 4;
    let uint_type = 5;
    let uint_array = 6;
    let uint_struct = 7;
    let storage_struct_pointer = 8;
    let uint_element_pointer = 9;
    let buffer_variable = 10;
    let width_variable = 11;
    let height_variable = 12;
    let depth_variable = 13;
    let stride_x_variable = 14;
    let stride_y_variable = 15;
    let stride_z_variable = 16;
    let capacity_variable = 17;
    let vector_type = 18;
    let input_pointer = 19;
    let global_variable = 20;
    let global_value = 21;
    let x_index = 22;
    let y_index = 23;
    let z_index = 24;
    let zero = 25;
    let constant_value = 26;
    let width_address = 27;
    let width_value = 28;
    let height_address = 29;
    let height_value = 30;
    let depth_address = 31;
    let depth_value = 32;
    let stride_x_address = 33;
    let stride_x_value = 34;
    let stride_y_address = 35;
    let stride_y_value = 36;
    let stride_z_address = 37;
    let stride_z_value = 38;
    let capacity_address = 39;
    let capacity_value = 40;
    let bool_type = 41;
    let x_in_bounds = 42;
    let y_in_bounds = 43;
    let z_in_bounds = 44;
    let xy_in_bounds = 45;
    let logical_in_bounds = 46;
    let x_offset = 47;
    let y_offset = 48;
    let xy_offset = 49;
    let z_offset = 50;
    let physical_index = 51;
    let physical_in_bounds = 52;
    let logical_body_label = 53;
    let logical_merge_label = 54;
    let physical_body_label = 55;
    let physical_merge_label = 56;
    let element_address = 57;
    let mut words = vec![SPIRV_MAGIC, SPIRV_VERSION_1_3, 0, 58, 0];
    instruction(&mut words, OP_CAPABILITY, &[CAPABILITY_SHADER]);
    instruction(
        &mut words,
        OP_MEMORY_MODEL,
        &[ADDRESSING_LOGICAL, MEMORY_MODEL_GLSL450],
    );
    instruction(
        &mut words,
        OP_DECORATE,
        &[uint_array, DECORATION_ARRAY_STRIDE, 4],
    );
    instruction(
        &mut words,
        OP_MEMBER_DECORATE,
        &[uint_struct, 0, DECORATION_OFFSET, 0],
    );
    instruction(&mut words, OP_DECORATE, &[uint_struct, DECORATION_BLOCK]);
    for (variable, binding) in [
        (buffer_variable, 0),
        (width_variable, 1),
        (height_variable, 2),
        (depth_variable, 3),
        (stride_x_variable, 4),
        (stride_y_variable, 5),
        (stride_z_variable, 6),
        (capacity_variable, 7),
    ] {
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_DESCRIPTOR_SET, 0],
        );
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_BINDING, binding],
        );
    }
    instruction(
        &mut words,
        OP_DECORATE,
        &[
            global_variable,
            DECORATION_BUILT_IN,
            BUILT_IN_GLOBAL_INVOCATION_ID,
        ],
    );
    instruction_string_with_tail(
        &mut words,
        OP_ENTRY_POINT,
        &[EXECUTION_MODEL_GL_COMPUTE, function_id],
        entry_name,
        &[
            buffer_variable,
            width_variable,
            height_variable,
            depth_variable,
            stride_x_variable,
            stride_y_variable,
            stride_z_variable,
            capacity_variable,
            global_variable,
        ],
    );
    instruction(
        &mut words,
        OP_EXECUTION_MODE,
        &[
            function_id,
            EXECUTION_MODE_LOCAL_SIZE,
            options.workgroup_size[0],
            options.workgroup_size[1],
            options.workgroup_size[2],
        ],
    );
    instruction(&mut words, OP_TYPE_VOID, &[void_type]);
    instruction(&mut words, OP_TYPE_INT, &[uint_type, 32, 0]);
    instruction(&mut words, OP_TYPE_RUNTIME_ARRAY, &[uint_array, uint_type]);
    instruction(&mut words, OP_TYPE_STRUCT, &[uint_struct, uint_array]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[storage_struct_pointer, STORAGE_BUFFER, uint_struct],
    );
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[uint_element_pointer, STORAGE_BUFFER, uint_type],
    );
    instruction(&mut words, OP_TYPE_VECTOR, &[vector_type, uint_type, 3]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[input_pointer, INPUT, vector_type],
    );
    for variable in [
        buffer_variable,
        width_variable,
        height_variable,
        depth_variable,
        stride_x_variable,
        stride_y_variable,
        stride_z_variable,
        capacity_variable,
    ] {
        instruction(
            &mut words,
            OP_VARIABLE,
            &[storage_struct_pointer, variable, STORAGE_BUFFER],
        );
    }
    instruction(
        &mut words,
        OP_VARIABLE,
        &[input_pointer, global_variable, INPUT],
    );
    instruction(&mut words, OP_TYPE_BOOL, &[bool_type]);
    instruction(&mut words, OP_CONSTANT, &[uint_type, zero, 0]);
    instruction(&mut words, OP_CONSTANT, &[uint_type, constant_value, value]);
    instruction(&mut words, OP_TYPE_FUNCTION, &[function_type, void_type]);
    instruction(
        &mut words,
        OP_FUNCTION,
        &[void_type, function_id, FUNCTION_CONTROL_NONE, function_type],
    );
    instruction(&mut words, OP_LABEL, &[entry_label]);
    instruction(
        &mut words,
        OP_LOAD,
        &[vector_type, global_value, global_variable],
    );
    for (result, component) in [(x_index, 0), (y_index, 1), (z_index, 2)] {
        instruction(
            &mut words,
            OP_COMPOSITE_EXTRACT,
            &[uint_type, result, global_value, component],
        );
    }
    for (variable, address, result) in [
        (width_variable, width_address, width_value),
        (height_variable, height_address, height_value),
        (depth_variable, depth_address, depth_value),
        (stride_x_variable, stride_x_address, stride_x_value),
        (stride_y_variable, stride_y_address, stride_y_value),
        (stride_z_variable, stride_z_address, stride_z_value),
        (capacity_variable, capacity_address, capacity_value),
    ] {
        instruction(
            &mut words,
            OP_ACCESS_CHAIN,
            &[uint_element_pointer, address, variable, zero, zero],
        );
        instruction(&mut words, OP_LOAD, &[uint_type, result, address]);
    }
    instruction(
        &mut words,
        OP_ULT,
        &[bool_type, x_in_bounds, x_index, width_value],
    );
    instruction(
        &mut words,
        OP_ULT,
        &[bool_type, y_in_bounds, y_index, height_value],
    );
    instruction(
        &mut words,
        OP_ULT,
        &[bool_type, z_in_bounds, z_index, depth_value],
    );
    instruction(
        &mut words,
        OP_LOGICAL_AND,
        &[bool_type, xy_in_bounds, x_in_bounds, y_in_bounds],
    );
    instruction(
        &mut words,
        OP_LOGICAL_AND,
        &[bool_type, logical_in_bounds, xy_in_bounds, z_in_bounds],
    );
    instruction(&mut words, OP_SELECTION_MERGE, &[logical_merge_label, 0]);
    instruction(
        &mut words,
        OP_BRANCH_CONDITIONAL,
        &[logical_in_bounds, logical_body_label, logical_merge_label],
    );
    instruction(&mut words, OP_LABEL, &[logical_body_label]);
    instruction(
        &mut words,
        OP_IMUL,
        &[uint_type, x_offset, x_index, stride_x_value],
    );
    instruction(
        &mut words,
        OP_IMUL,
        &[uint_type, y_offset, y_index, stride_y_value],
    );
    instruction(
        &mut words,
        OP_IADD,
        &[uint_type, xy_offset, x_offset, y_offset],
    );
    instruction(
        &mut words,
        OP_IMUL,
        &[uint_type, z_offset, z_index, stride_z_value],
    );
    instruction(
        &mut words,
        OP_IADD,
        &[uint_type, physical_index, xy_offset, z_offset],
    );
    instruction(
        &mut words,
        OP_ULT,
        &[
            bool_type,
            physical_in_bounds,
            physical_index,
            capacity_value,
        ],
    );
    instruction(&mut words, OP_SELECTION_MERGE, &[physical_merge_label, 0]);
    instruction(
        &mut words,
        OP_BRANCH_CONDITIONAL,
        &[
            physical_in_bounds,
            physical_body_label,
            physical_merge_label,
        ],
    );
    instruction(&mut words, OP_LABEL, &[physical_body_label]);
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            uint_element_pointer,
            element_address,
            buffer_variable,
            zero,
            physical_index,
        ],
    );
    instruction(&mut words, OP_STORE, &[element_address, constant_value]);
    instruction(&mut words, OP_BRANCH, &[physical_merge_label]);
    instruction(&mut words, OP_LABEL, &[physical_merge_label]);
    instruction(&mut words, OP_BRANCH, &[logical_merge_label]);
    instruction(&mut words, OP_LABEL, &[logical_merge_label]);
    instruction(&mut words, OP_RETURN, &[]);
    instruction(&mut words, OP_FUNCTION_END, &[]);
    validate_spirv(&words).map_err(|_| {
        SpirvError::UnsupportedKernelShape("3d strided global-index validation failed")
    })?;
    Ok(words)
}

/// Emits a bounds-safe three-dimensional global-index write kernel.
///
/// The logical row-major index is `((z * height) + y) * width + x`.
/// Runtime metadata resources provide width, height, depth and capacity; all
/// three coordinates and the flattened physical index are checked before the
/// store.
pub fn emit_storage_global_3d_write(
    entry_name: &str,
    options: SpirvOptions,
    value: u32,
) -> Result<Vec<u32>, SpirvError> {
    if entry_name.is_empty() || entry_name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    let function_id = 1;
    let void_type = 2;
    let function_type = 3;
    let entry_label = 4;
    let uint_type = 5;
    let uint_array = 6;
    let uint_struct = 7;
    let storage_struct_pointer = 8;
    let uint_element_pointer = 9;
    let buffer_variable = 10;
    let width_variable = 11;
    let height_variable = 12;
    let depth_variable = 13;
    let capacity_variable = 14;
    let vector_type = 15;
    let input_pointer = 16;
    let global_variable = 17;
    let global_value = 18;
    let x_index = 19;
    let y_index = 20;
    let z_index = 21;
    let zero = 22;
    let constant_value = 23;
    let width_address = 24;
    let width_value = 25;
    let height_address = 26;
    let height_value = 27;
    let depth_address = 28;
    let depth_value = 29;
    let capacity_address = 30;
    let capacity_value = 31;
    let bool_type = 32;
    let x_in_bounds = 33;
    let y_in_bounds = 34;
    let z_in_bounds = 35;
    let xy_in_bounds = 36;
    let logical_in_bounds = 37;
    let z_rows = 38;
    let row_number = 39;
    let row_offset = 40;
    let logical_index = 41;
    let physical_in_bounds = 42;
    let logical_body_label = 43;
    let logical_merge_label = 44;
    let physical_body_label = 45;
    let physical_merge_label = 46;
    let element_address = 47;
    let mut words = vec![SPIRV_MAGIC, SPIRV_VERSION_1_3, 0, 48, 0];
    instruction(&mut words, OP_CAPABILITY, &[CAPABILITY_SHADER]);
    instruction(
        &mut words,
        OP_MEMORY_MODEL,
        &[ADDRESSING_LOGICAL, MEMORY_MODEL_GLSL450],
    );
    instruction(
        &mut words,
        OP_DECORATE,
        &[uint_array, DECORATION_ARRAY_STRIDE, 4],
    );
    instruction(
        &mut words,
        OP_MEMBER_DECORATE,
        &[uint_struct, 0, DECORATION_OFFSET, 0],
    );
    instruction(&mut words, OP_DECORATE, &[uint_struct, DECORATION_BLOCK]);
    for (variable, binding) in [
        (buffer_variable, 0),
        (width_variable, 1),
        (height_variable, 2),
        (depth_variable, 3),
        (capacity_variable, 4),
    ] {
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_DESCRIPTOR_SET, 0],
        );
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_BINDING, binding],
        );
    }
    instruction(
        &mut words,
        OP_DECORATE,
        &[
            global_variable,
            DECORATION_BUILT_IN,
            BUILT_IN_GLOBAL_INVOCATION_ID,
        ],
    );
    instruction_string_with_tail(
        &mut words,
        OP_ENTRY_POINT,
        &[EXECUTION_MODEL_GL_COMPUTE, function_id],
        entry_name,
        &[
            buffer_variable,
            width_variable,
            height_variable,
            depth_variable,
            capacity_variable,
            global_variable,
        ],
    );
    instruction(
        &mut words,
        OP_EXECUTION_MODE,
        &[
            function_id,
            EXECUTION_MODE_LOCAL_SIZE,
            options.workgroup_size[0],
            options.workgroup_size[1],
            options.workgroup_size[2],
        ],
    );
    instruction(&mut words, OP_TYPE_VOID, &[void_type]);
    instruction(&mut words, OP_TYPE_INT, &[uint_type, 32, 0]);
    instruction(&mut words, OP_TYPE_RUNTIME_ARRAY, &[uint_array, uint_type]);
    instruction(&mut words, OP_TYPE_STRUCT, &[uint_struct, uint_array]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[storage_struct_pointer, STORAGE_BUFFER, uint_struct],
    );
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[uint_element_pointer, STORAGE_BUFFER, uint_type],
    );
    instruction(&mut words, OP_TYPE_VECTOR, &[vector_type, uint_type, 3]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[input_pointer, INPUT, vector_type],
    );
    for variable in [
        buffer_variable,
        width_variable,
        height_variable,
        depth_variable,
        capacity_variable,
    ] {
        instruction(
            &mut words,
            OP_VARIABLE,
            &[storage_struct_pointer, variable, STORAGE_BUFFER],
        );
    }
    instruction(
        &mut words,
        OP_VARIABLE,
        &[input_pointer, global_variable, INPUT],
    );
    instruction(&mut words, OP_TYPE_BOOL, &[bool_type]);
    instruction(&mut words, OP_CONSTANT, &[uint_type, zero, 0]);
    instruction(&mut words, OP_CONSTANT, &[uint_type, constant_value, value]);
    instruction(&mut words, OP_TYPE_FUNCTION, &[function_type, void_type]);
    instruction(
        &mut words,
        OP_FUNCTION,
        &[void_type, function_id, FUNCTION_CONTROL_NONE, function_type],
    );
    instruction(&mut words, OP_LABEL, &[entry_label]);
    instruction(
        &mut words,
        OP_LOAD,
        &[vector_type, global_value, global_variable],
    );
    for (result, component) in [(x_index, 0), (y_index, 1), (z_index, 2)] {
        instruction(
            &mut words,
            OP_COMPOSITE_EXTRACT,
            &[uint_type, result, global_value, component],
        );
    }
    for (variable, address, result) in [
        (width_variable, width_address, width_value),
        (height_variable, height_address, height_value),
        (depth_variable, depth_address, depth_value),
        (capacity_variable, capacity_address, capacity_value),
    ] {
        instruction(
            &mut words,
            OP_ACCESS_CHAIN,
            &[uint_element_pointer, address, variable, zero, zero],
        );
        instruction(&mut words, OP_LOAD, &[uint_type, result, address]);
    }
    instruction(
        &mut words,
        OP_ULT,
        &[bool_type, x_in_bounds, x_index, width_value],
    );
    instruction(
        &mut words,
        OP_ULT,
        &[bool_type, y_in_bounds, y_index, height_value],
    );
    instruction(
        &mut words,
        OP_ULT,
        &[bool_type, z_in_bounds, z_index, depth_value],
    );
    instruction(
        &mut words,
        OP_LOGICAL_AND,
        &[bool_type, xy_in_bounds, x_in_bounds, y_in_bounds],
    );
    instruction(
        &mut words,
        OP_LOGICAL_AND,
        &[bool_type, logical_in_bounds, xy_in_bounds, z_in_bounds],
    );
    instruction(&mut words, OP_SELECTION_MERGE, &[logical_merge_label, 0]);
    instruction(
        &mut words,
        OP_BRANCH_CONDITIONAL,
        &[logical_in_bounds, logical_body_label, logical_merge_label],
    );
    instruction(&mut words, OP_LABEL, &[logical_body_label]);
    instruction(
        &mut words,
        OP_IMUL,
        &[uint_type, z_rows, z_index, height_value],
    );
    instruction(
        &mut words,
        OP_IADD,
        &[uint_type, row_number, z_rows, y_index],
    );
    instruction(
        &mut words,
        OP_IMUL,
        &[uint_type, row_offset, row_number, width_value],
    );
    instruction(
        &mut words,
        OP_IADD,
        &[uint_type, logical_index, row_offset, x_index],
    );
    instruction(
        &mut words,
        OP_ULT,
        &[bool_type, physical_in_bounds, logical_index, capacity_value],
    );
    instruction(&mut words, OP_SELECTION_MERGE, &[physical_merge_label, 0]);
    instruction(
        &mut words,
        OP_BRANCH_CONDITIONAL,
        &[
            physical_in_bounds,
            physical_body_label,
            physical_merge_label,
        ],
    );
    instruction(&mut words, OP_LABEL, &[physical_body_label]);
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            uint_element_pointer,
            element_address,
            buffer_variable,
            zero,
            logical_index,
        ],
    );
    instruction(&mut words, OP_STORE, &[element_address, constant_value]);
    instruction(&mut words, OP_BRANCH, &[physical_merge_label]);
    instruction(&mut words, OP_LABEL, &[physical_merge_label]);
    instruction(&mut words, OP_BRANCH, &[logical_merge_label]);
    instruction(&mut words, OP_LABEL, &[logical_merge_label]);
    instruction(&mut words, OP_RETURN, &[]);
    instruction(&mut words, OP_FUNCTION_END, &[]);
    validate_spirv(&words)
        .map_err(|_| SpirvError::UnsupportedKernelShape("3d global-index validation failed"))?;
    Ok(words)
}

/// Lowers the bounds-safe two-dimensional row-major JIR shape:
/// `buffer[y * width + x] = value` with runtime width, height and capacity.
pub fn emit_storage_global_2d_write_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<Vec<u32>, SpirvError> {
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(SpirvError::GpuVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    if function_data.name.is_empty() || function_data.name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) || function_data.parameters.len() != 4
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d global-index write requires four resource parameters and Unit result",
        ));
    }
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    if resources.len() != 4
        || resources
            .iter()
            .any(|resource| resource.address_space != AddressSpace::Storage)
        || resources.iter().any(|resource| {
            !matches!(
                module.types.get(resource.element_type.index()),
                Some(Type::Integer {
                    signed: false,
                    bits: 32
                })
            )
        })
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d global-index write requires four storage u32 resources",
        ));
    }
    let Some(block) = function_data.blocks.first() else {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d global-index write requires one entry block",
        ));
    };
    if function_data.blocks.len() != 1
        || !block.parameters.is_empty()
        || block.instructions.len() != 13
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d global-index write body has an unsupported shape",
        ));
    }
    let u32_type = resources[0].element_type;
    let builtin_x = &block.instructions[0];
    let Some(x_result) = builtin_x.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d global-index x builtin must produce a value",
        ));
    };
    if !matches!(
        &builtin_x.kind,
        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX)
    ) || x_result.ty != u32_type
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d global-index first instruction must produce u32 GlobalInvocationId.x",
        ));
    }
    let builtin_y = &block.instructions[1];
    let Some(y_result) = builtin_y.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d global-index y builtin must produce a value",
        ));
    };
    if !matches!(
        &builtin_y.kind,
        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdY)
    ) || y_result.ty != u32_type
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d global-index second instruction must produce u32 GlobalInvocationId.y",
        ));
    }
    let value_instruction = &block.instructions[2];
    let Some(value_result) = value_instruction.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d global-index value constant must produce a value",
        ));
    };
    let value = match (&value_instruction.kind, value_result.ty == u32_type) {
        (InstructionKind::Constant(Constant::Integer { value }), true) => u32::try_from(*value)
            .map_err(|_| {
                SpirvError::UnsupportedKernelShape("2d global-index value is outside u32")
            })?,
        _ => {
            return Err(SpirvError::UnsupportedKernelShape(
                "2d global-index third instruction must be a u32 constant",
            ));
        }
    };
    let metadata = [
        (
            &block.instructions[3],
            function_data.parameters[1].value,
            "width",
        ),
        (
            &block.instructions[4],
            function_data.parameters[2].value,
            "height",
        ),
        (
            &block.instructions[5],
            function_data.parameters[3].value,
            "capacity",
        ),
    ];
    let mut metadata_values = [ValueId::new(0); 3];
    for (slot, (instruction, pointer, name)) in metadata.into_iter().enumerate() {
        let Some(result) = instruction.result else {
            return Err(SpirvError::UnsupportedKernelShape(
                "2d global-index metadata load must produce a value",
            ));
        };
        if !matches!(
            &instruction.kind,
            InstructionKind::Load { pointer: load_pointer, .. }
                if *load_pointer == pointer && result.ty == u32_type
        ) {
            return Err(SpirvError::UnsupportedKernelShape(match name {
                "width" => "2d global-index fourth instruction must load width resource",
                "height" => "2d global-index fifth instruction must load height resource",
                _ => "2d global-index sixth instruction must load capacity resource",
            }));
        }
        metadata_values[slot] = result.value;
    }
    for (instruction, index, length, message) in [
        (
            &block.instructions[6],
            x_result.value,
            metadata_values[0],
            "2d global-index seventh instruction must bounds-check x against width",
        ),
        (
            &block.instructions[7],
            y_result.value,
            metadata_values[1],
            "2d global-index eighth instruction must bounds-check y against height",
        ),
    ] {
        if instruction.result.is_some()
            || !matches!(
                &instruction.kind,
                InstructionKind::BoundsCheck { index: checked_index, length: checked_length }
                    if *checked_index == index && *checked_length == length
            )
        {
            return Err(SpirvError::UnsupportedKernelShape(message));
        }
    }
    let row = &block.instructions[8];
    let Some(row_result) = row.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d global-index row multiply must produce a value",
        ));
    };
    if !matches!(
        &row.kind,
        InstructionKind::Binary { op, left, right }
            if *op == BinaryOp::Multiply
                && row_result.ty == u32_type
                && ((*left == y_result.value && *right == metadata_values[0])
                    || (*right == y_result.value && *left == metadata_values[0]))
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d global-index ninth instruction must multiply y by width",
        ));
    }
    let flattened = &block.instructions[9];
    let Some(flattened_result) = flattened.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d global-index flatten add must produce a value",
        ));
    };
    if !matches!(
        &flattened.kind,
        InstructionKind::Binary { op, left, right }
            if *op == BinaryOp::Add
                && flattened_result.ty == u32_type
                && ((*left == row_result.value && *right == x_result.value)
                    || (*right == row_result.value && *left == x_result.value))
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d global-index tenth instruction must flatten row-major index",
        ));
    }
    let physical_bounds = &block.instructions[10];
    if physical_bounds.result.is_some()
        || !matches!(
            &physical_bounds.kind,
            InstructionKind::BoundsCheck { index, length }
                if *index == flattened_result.value && *length == metadata_values[2]
        )
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d global-index eleventh instruction must bounds-check capacity",
        ));
    }
    let offset = &block.instructions[11];
    let Some(offset_result) = offset.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d global-index offset must produce a pointer",
        ));
    };
    if !matches!(
        &offset.kind,
        InstructionKind::Offset { base, indices }
            if *base == function_data.parameters[0].value
                && indices == &[flattened_result.value]
                && offset_result.ty == function_data.parameters[0].ty
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d global-index offset is invalid",
        ));
    }
    let store = &block.instructions[12];
    if store.result.is_some()
        || !matches!(
            &store.kind,
            InstructionKind::Store { pointer, value: stored_value, .. }
                if *pointer == offset_result.value && *stored_value == value_result.value
        )
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d global-index store is invalid",
        ));
    }
    if !matches!(block.terminator, Terminator::Return { value: None }) {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d global-index write must return Unit",
        ));
    }
    let mut words = emit_storage_global_2d_write(&function_data.name, options, value)?;
    apply_reflected_resource_access_decorations(&mut words, &resources)?;
    Ok(words)
}

/// Lowers the bounds-safe two-dimensional affine-stride JIR shape:
/// `buffer[x * stride_x + y * stride_y] = value`.
pub fn emit_storage_global_2d_strided_write_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<Vec<u32>, SpirvError> {
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(SpirvError::GpuVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    if function_data.name.is_empty() || function_data.name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) || function_data.parameters.len() != 6
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d strided global-index write requires six resource parameters and Unit result",
        ));
    }
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    if resources.len() != 6
        || resources
            .iter()
            .any(|resource| resource.address_space != AddressSpace::Storage)
        || resources.iter().any(|resource| {
            !matches!(
                module.types.get(resource.element_type.index()),
                Some(Type::Integer {
                    signed: false,
                    bits: 32
                })
            )
        })
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d strided global-index write requires six storage u32 resources",
        ));
    }
    let Some(block) = function_data.blocks.first() else {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d strided global-index write requires one entry block",
        ));
    };
    if function_data.blocks.len() != 1
        || !block.parameters.is_empty()
        || block.instructions.len() != 16
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d strided global-index write body has an unsupported shape",
        ));
    }
    let u32_type = resources[0].element_type;
    let builtins = [
        (&block.instructions[0], BuiltinOp::GlobalInvocationIdX, "x"),
        (&block.instructions[1], BuiltinOp::GlobalInvocationIdY, "y"),
    ];
    let mut coordinate_values = [ValueId::new(0); 2];
    for (slot, (instruction, expected, axis)) in builtins.into_iter().enumerate() {
        let Some(result) = instruction.result else {
            return Err(SpirvError::UnsupportedKernelShape(
                "2d strided global-index builtin must produce a value",
            ));
        };
        if !matches!(&instruction.kind, InstructionKind::Builtin(actual) if *actual == expected)
            || result.ty != u32_type
        {
            return Err(SpirvError::UnsupportedKernelShape(match axis {
                "x" => {
                    "2d strided global-index first instruction must produce u32 GlobalInvocationId.x"
                }
                _ => {
                    "2d strided global-index second instruction must produce u32 GlobalInvocationId.y"
                }
            }));
        }
        coordinate_values[slot] = result.value;
    }
    let value_instruction = &block.instructions[2];
    let Some(value_result) = value_instruction.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d strided global-index value constant must produce a value",
        ));
    };
    let value = match (&value_instruction.kind, value_result.ty == u32_type) {
        (InstructionKind::Constant(Constant::Integer { value }), true) => u32::try_from(*value)
            .map_err(|_| {
                SpirvError::UnsupportedKernelShape("2d strided global-index value is outside u32")
            })?,
        _ => {
            return Err(SpirvError::UnsupportedKernelShape(
                "2d strided global-index third instruction must be a u32 constant",
            ));
        }
    };
    let metadata = [
        (
            &block.instructions[3],
            function_data.parameters[1].value,
            "width",
        ),
        (
            &block.instructions[4],
            function_data.parameters[2].value,
            "height",
        ),
        (
            &block.instructions[5],
            function_data.parameters[3].value,
            "stride_x",
        ),
        (
            &block.instructions[6],
            function_data.parameters[4].value,
            "stride_y",
        ),
        (
            &block.instructions[7],
            function_data.parameters[5].value,
            "capacity",
        ),
    ];
    let mut metadata_values = [ValueId::new(0); 5];
    for (slot, (instruction, pointer, name)) in metadata.into_iter().enumerate() {
        let Some(result) = instruction.result else {
            return Err(SpirvError::UnsupportedKernelShape(
                "2d strided global-index metadata load must produce a value",
            ));
        };
        if !matches!(
            &instruction.kind,
            InstructionKind::Load { pointer: load_pointer, .. }
                if *load_pointer == pointer && result.ty == u32_type
        ) {
            return Err(SpirvError::UnsupportedKernelShape(match name {
                "width" => "2d strided global-index fourth instruction must load width resource",
                "height" => "2d strided global-index fifth instruction must load height resource",
                "stride_x" => {
                    "2d strided global-index sixth instruction must load stride_x resource"
                }
                "stride_y" => {
                    "2d strided global-index seventh instruction must load stride_y resource"
                }
                _ => "2d strided global-index eighth instruction must load capacity resource",
            }));
        }
        metadata_values[slot] = result.value;
    }
    for (instruction, index, length, message) in [
        (
            &block.instructions[8],
            coordinate_values[0],
            metadata_values[0],
            "2d strided global-index ninth instruction must bounds-check x against width",
        ),
        (
            &block.instructions[9],
            coordinate_values[1],
            metadata_values[1],
            "2d strided global-index tenth instruction must bounds-check y against height",
        ),
    ] {
        if instruction.result.is_some()
            || !matches!(
                &instruction.kind,
                InstructionKind::BoundsCheck { index: checked_index, length: checked_length }
                    if *checked_index == index && *checked_length == length
            )
        {
            return Err(SpirvError::UnsupportedKernelShape(message));
        }
    }
    let x_offset = &block.instructions[10];
    let Some(x_offset_result) = x_offset.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d strided global-index x multiply must produce a value",
        ));
    };
    if !matches!(
        &x_offset.kind,
        InstructionKind::Binary { op, left, right }
            if *op == BinaryOp::Multiply
                && x_offset_result.ty == u32_type
                && ((*left == coordinate_values[0] && *right == metadata_values[2])
                    || (*right == coordinate_values[0] && *left == metadata_values[2]))
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d strided global-index eleventh instruction must multiply x by stride_x",
        ));
    }
    let y_offset = &block.instructions[11];
    let Some(y_offset_result) = y_offset.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d strided global-index y multiply must produce a value",
        ));
    };
    if !matches!(
        &y_offset.kind,
        InstructionKind::Binary { op, left, right }
            if *op == BinaryOp::Multiply
                && y_offset_result.ty == u32_type
                && ((*left == coordinate_values[1] && *right == metadata_values[3])
                    || (*right == coordinate_values[1] && *left == metadata_values[3]))
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d strided global-index twelfth instruction must multiply y by stride_y",
        ));
    }
    let physical = &block.instructions[12];
    let Some(physical_result) = physical.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d strided global-index physical add must produce a value",
        ));
    };
    if !matches!(
        &physical.kind,
        InstructionKind::Binary { op, left, right }
            if *op == BinaryOp::Add
                && physical_result.ty == u32_type
                && ((*left == x_offset_result.value && *right == y_offset_result.value)
                    || (*right == x_offset_result.value && *left == y_offset_result.value))
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d strided global-index thirteenth instruction must add affine offsets",
        ));
    }
    let physical_bounds = &block.instructions[13];
    if physical_bounds.result.is_some()
        || !matches!(
            &physical_bounds.kind,
            InstructionKind::BoundsCheck { index, length }
                if *index == physical_result.value && *length == metadata_values[4]
        )
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d strided global-index fourteenth instruction must bounds-check capacity",
        ));
    }
    let offset = &block.instructions[14];
    let Some(offset_result) = offset.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d strided global-index offset must produce a pointer",
        ));
    };
    if !matches!(
        &offset.kind,
        InstructionKind::Offset { base, indices }
            if *base == function_data.parameters[0].value
                && indices == &[physical_result.value]
                && offset_result.ty == function_data.parameters[0].ty
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d strided global-index offset is invalid",
        ));
    }
    let store = &block.instructions[15];
    if store.result.is_some()
        || !matches!(
            &store.kind,
            InstructionKind::Store { pointer, value: stored_value, .. }
                if *pointer == offset_result.value && *stored_value == value_result.value
        )
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d strided global-index store is invalid",
        ));
    }
    if !matches!(block.terminator, Terminator::Return { value: None }) {
        return Err(SpirvError::UnsupportedKernelShape(
            "2d strided global-index write must return Unit",
        ));
    }
    let mut words = emit_storage_global_2d_strided_write(&function_data.name, options, value)?;
    apply_reflected_resource_access_decorations(&mut words, &resources)?;
    Ok(words)
}

/// Lowers the bounds-safe three-dimensional affine-stride JIR shape:
/// `buffer[x * stride_x + y * stride_y + z * stride_z] = value`.
pub fn emit_storage_global_3d_strided_write_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<Vec<u32>, SpirvError> {
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(SpirvError::GpuVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    if function_data.name.is_empty() || function_data.name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) || function_data.parameters.len() != 8
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d strided global-index write requires eight resource parameters and Unit result",
        ));
    }
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    if resources.len() != 8
        || resources
            .iter()
            .any(|resource| resource.address_space != AddressSpace::Storage)
        || resources.iter().any(|resource| {
            !matches!(
                module.types.get(resource.element_type.index()),
                Some(Type::Integer {
                    signed: false,
                    bits: 32
                })
            )
        })
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d strided global-index write requires eight storage u32 resources",
        ));
    }
    let Some(block) = function_data.blocks.first() else {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d strided global-index write requires one entry block",
        ));
    };
    if function_data.blocks.len() != 1
        || !block.parameters.is_empty()
        || block.instructions.len() != 22
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d strided global-index write body has an unsupported shape",
        ));
    }
    let u32_type = resources[0].element_type;
    let builtins = [
        (&block.instructions[0], BuiltinOp::GlobalInvocationIdX, "x"),
        (&block.instructions[1], BuiltinOp::GlobalInvocationIdY, "y"),
        (&block.instructions[2], BuiltinOp::GlobalInvocationIdZ, "z"),
    ];
    let mut coordinate_values = [ValueId::new(0); 3];
    for (slot, (instruction, expected, axis)) in builtins.into_iter().enumerate() {
        let Some(result) = instruction.result else {
            return Err(SpirvError::UnsupportedKernelShape(
                "3d strided global-index builtin must produce a value",
            ));
        };
        if !matches!(&instruction.kind, InstructionKind::Builtin(actual) if *actual == expected)
            || result.ty != u32_type
        {
            return Err(SpirvError::UnsupportedKernelShape(match axis {
                "x" => {
                    "3d strided global-index first instruction must produce u32 GlobalInvocationId.x"
                }
                "y" => {
                    "3d strided global-index second instruction must produce u32 GlobalInvocationId.y"
                }
                _ => {
                    "3d strided global-index third instruction must produce u32 GlobalInvocationId.z"
                }
            }));
        }
        coordinate_values[slot] = result.value;
    }
    let value_instruction = &block.instructions[3];
    let Some(value_result) = value_instruction.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d strided global-index value constant must produce a value",
        ));
    };
    let value = match (&value_instruction.kind, value_result.ty == u32_type) {
        (InstructionKind::Constant(Constant::Integer { value }), true) => u32::try_from(*value)
            .map_err(|_| {
                SpirvError::UnsupportedKernelShape("3d strided global-index value is outside u32")
            })?,
        _ => {
            return Err(SpirvError::UnsupportedKernelShape(
                "3d strided global-index fourth instruction must be a u32 constant",
            ));
        }
    };
    let metadata = [
        (
            &block.instructions[4],
            function_data.parameters[1].value,
            "width",
        ),
        (
            &block.instructions[5],
            function_data.parameters[2].value,
            "height",
        ),
        (
            &block.instructions[6],
            function_data.parameters[3].value,
            "depth",
        ),
        (
            &block.instructions[7],
            function_data.parameters[4].value,
            "stride_x",
        ),
        (
            &block.instructions[8],
            function_data.parameters[5].value,
            "stride_y",
        ),
        (
            &block.instructions[9],
            function_data.parameters[6].value,
            "stride_z",
        ),
        (
            &block.instructions[10],
            function_data.parameters[7].value,
            "capacity",
        ),
    ];
    let mut metadata_values = [ValueId::new(0); 7];
    for (slot, (instruction, pointer, name)) in metadata.into_iter().enumerate() {
        let Some(result) = instruction.result else {
            return Err(SpirvError::UnsupportedKernelShape(
                "3d strided global-index metadata load must produce a value",
            ));
        };
        if !matches!(
            &instruction.kind,
            InstructionKind::Load { pointer: load_pointer, .. }
                if *load_pointer == pointer && result.ty == u32_type
        ) {
            return Err(SpirvError::UnsupportedKernelShape(match name {
                "width" => "3d strided global-index fifth instruction must load width resource",
                "height" => "3d strided global-index sixth instruction must load height resource",
                "depth" => "3d strided global-index seventh instruction must load depth resource",
                "stride_x" => {
                    "3d strided global-index eighth instruction must load stride_x resource"
                }
                "stride_y" => {
                    "3d strided global-index ninth instruction must load stride_y resource"
                }
                "stride_z" => {
                    "3d strided global-index tenth instruction must load stride_z resource"
                }
                _ => "3d strided global-index eleventh instruction must load capacity resource",
            }));
        }
        metadata_values[slot] = result.value;
    }
    for (instruction, index, length, message) in [
        (
            &block.instructions[11],
            coordinate_values[0],
            metadata_values[0],
            "3d strided global-index twelfth instruction must bounds-check x against width",
        ),
        (
            &block.instructions[12],
            coordinate_values[1],
            metadata_values[1],
            "3d strided global-index thirteenth instruction must bounds-check y against height",
        ),
        (
            &block.instructions[13],
            coordinate_values[2],
            metadata_values[2],
            "3d strided global-index fourteenth instruction must bounds-check z against depth",
        ),
    ] {
        if instruction.result.is_some()
            || !matches!(
                &instruction.kind,
                InstructionKind::BoundsCheck { index: checked_index, length: checked_length }
                    if *checked_index == index && *checked_length == length
            )
        {
            return Err(SpirvError::UnsupportedKernelShape(message));
        }
    }
    let binary_matches =
        |instruction: &Instruction, op: BinaryOp, left: ValueId, right: ValueId| {
            let Some(result) = instruction.result else {
                return false;
            };
            matches!(
                &instruction.kind,
                InstructionKind::Binary { op: actual, left: actual_left, right: actual_right }
                    if *actual == op
                        && result.ty == u32_type
                        && ((*actual_left == left && *actual_right == right)
                            || (*actual_left == right && *actual_right == left))
            )
        };
    let Some(x_offset_result) = block.instructions[14].result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d strided global-index x multiply must produce a value",
        ));
    };
    if !binary_matches(
        &block.instructions[14],
        BinaryOp::Multiply,
        coordinate_values[0],
        metadata_values[3],
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d strided global-index fifteenth instruction must multiply x by stride_x",
        ));
    }
    let Some(y_offset_result) = block.instructions[15].result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d strided global-index y multiply must produce a value",
        ));
    };
    if !binary_matches(
        &block.instructions[15],
        BinaryOp::Multiply,
        coordinate_values[1],
        metadata_values[4],
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d strided global-index sixteenth instruction must multiply y by stride_y",
        ));
    }
    let Some(xy_offset_result) = block.instructions[16].result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d strided global-index affine add must produce a value",
        ));
    };
    if !binary_matches(
        &block.instructions[16],
        BinaryOp::Add,
        x_offset_result.value,
        y_offset_result.value,
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d strided global-index seventeenth instruction must add x/y offsets",
        ));
    }
    let Some(z_offset_result) = block.instructions[17].result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d strided global-index z multiply must produce a value",
        ));
    };
    if !binary_matches(
        &block.instructions[17],
        BinaryOp::Multiply,
        coordinate_values[2],
        metadata_values[5],
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d strided global-index eighteenth instruction must multiply z by stride_z",
        ));
    }
    let Some(physical_result) = block.instructions[18].result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d strided global-index physical add must produce a value",
        ));
    };
    if !binary_matches(
        &block.instructions[18],
        BinaryOp::Add,
        xy_offset_result.value,
        z_offset_result.value,
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d strided global-index nineteenth instruction must add affine offsets",
        ));
    }
    let physical_bounds = &block.instructions[19];
    if physical_bounds.result.is_some()
        || !matches!(
            &physical_bounds.kind,
            InstructionKind::BoundsCheck { index, length }
                if *index == physical_result.value && *length == metadata_values[6]
        )
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d strided global-index twentieth instruction must bounds-check capacity",
        ));
    }
    let offset = &block.instructions[20];
    let Some(offset_result) = offset.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d strided global-index offset must produce a pointer",
        ));
    };
    if !matches!(
        &offset.kind,
        InstructionKind::Offset { base, indices }
            if *base == function_data.parameters[0].value
                && indices == &[physical_result.value]
                && offset_result.ty == function_data.parameters[0].ty
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d strided global-index offset is invalid",
        ));
    }
    let store = &block.instructions[21];
    if store.result.is_some()
        || !matches!(
            &store.kind,
            InstructionKind::Store { pointer, value: stored_value, .. }
                if *pointer == offset_result.value && *stored_value == value_result.value
        )
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d strided global-index store is invalid",
        ));
    }
    if !matches!(block.terminator, Terminator::Return { value: None }) {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d strided global-index write must return Unit",
        ));
    }
    let mut words = emit_storage_global_3d_strided_write(&function_data.name, options, value)?;
    apply_reflected_resource_access_decorations(&mut words, &resources)?;
    Ok(words)
}

/// Lowers the bounds-safe three-dimensional row-major JIR shape:
/// `buffer[((z * height) + y) * width + x] = value`.
pub fn emit_storage_global_3d_write_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<Vec<u32>, SpirvError> {
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(SpirvError::GpuVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    if function_data.name.is_empty() || function_data.name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) || function_data.parameters.len() != 5
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d global-index write requires five resource parameters and Unit result",
        ));
    }
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    if resources.len() != 5
        || resources
            .iter()
            .any(|resource| resource.address_space != AddressSpace::Storage)
        || resources.iter().any(|resource| {
            !matches!(
                module.types.get(resource.element_type.index()),
                Some(Type::Integer {
                    signed: false,
                    bits: 32
                })
            )
        })
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d global-index write requires five storage u32 resources",
        ));
    }
    let Some(block) = function_data.blocks.first() else {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d global-index write requires one entry block",
        ));
    };
    if function_data.blocks.len() != 1
        || !block.parameters.is_empty()
        || block.instructions.len() != 18
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d global-index write body has an unsupported shape",
        ));
    }
    let u32_type = resources[0].element_type;
    let builtins = [
        (&block.instructions[0], BuiltinOp::GlobalInvocationIdX, "x"),
        (&block.instructions[1], BuiltinOp::GlobalInvocationIdY, "y"),
        (&block.instructions[2], BuiltinOp::GlobalInvocationIdZ, "z"),
    ];
    let mut coordinate_values = [ValueId::new(0); 3];
    for (slot, (instruction, expected, axis)) in builtins.into_iter().enumerate() {
        let Some(result) = instruction.result else {
            return Err(SpirvError::UnsupportedKernelShape(
                "3d global-index builtin must produce a value",
            ));
        };
        if !matches!(&instruction.kind, InstructionKind::Builtin(actual) if *actual == expected)
            || result.ty != u32_type
        {
            return Err(SpirvError::UnsupportedKernelShape(match axis {
                "x" => "3d global-index first instruction must produce u32 GlobalInvocationId.x",
                "y" => "3d global-index second instruction must produce u32 GlobalInvocationId.y",
                _ => "3d global-index third instruction must produce u32 GlobalInvocationId.z",
            }));
        }
        coordinate_values[slot] = result.value;
    }
    let value_instruction = &block.instructions[3];
    let Some(value_result) = value_instruction.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d global-index value constant must produce a value",
        ));
    };
    let value = match (&value_instruction.kind, value_result.ty == u32_type) {
        (InstructionKind::Constant(Constant::Integer { value }), true) => u32::try_from(*value)
            .map_err(|_| {
                SpirvError::UnsupportedKernelShape("3d global-index value is outside u32")
            })?,
        _ => {
            return Err(SpirvError::UnsupportedKernelShape(
                "3d global-index fourth instruction must be a u32 constant",
            ));
        }
    };
    let metadata = [
        (
            &block.instructions[4],
            function_data.parameters[1].value,
            "width",
        ),
        (
            &block.instructions[5],
            function_data.parameters[2].value,
            "height",
        ),
        (
            &block.instructions[6],
            function_data.parameters[3].value,
            "depth",
        ),
        (
            &block.instructions[7],
            function_data.parameters[4].value,
            "capacity",
        ),
    ];
    let mut metadata_values = [ValueId::new(0); 4];
    for (slot, (instruction, pointer, name)) in metadata.into_iter().enumerate() {
        let Some(result) = instruction.result else {
            return Err(SpirvError::UnsupportedKernelShape(
                "3d global-index metadata load must produce a value",
            ));
        };
        if !matches!(
            &instruction.kind,
            InstructionKind::Load { pointer: load_pointer, .. }
                if *load_pointer == pointer && result.ty == u32_type
        ) {
            return Err(SpirvError::UnsupportedKernelShape(match name {
                "width" => "3d global-index fifth instruction must load width resource",
                "height" => "3d global-index sixth instruction must load height resource",
                "depth" => "3d global-index seventh instruction must load depth resource",
                _ => "3d global-index eighth instruction must load capacity resource",
            }));
        }
        metadata_values[slot] = result.value;
    }
    for (instruction, index, length, message) in [
        (
            &block.instructions[8],
            coordinate_values[0],
            metadata_values[0],
            "3d global-index ninth instruction must bounds-check x against width",
        ),
        (
            &block.instructions[9],
            coordinate_values[1],
            metadata_values[1],
            "3d global-index tenth instruction must bounds-check y against height",
        ),
        (
            &block.instructions[10],
            coordinate_values[2],
            metadata_values[2],
            "3d global-index eleventh instruction must bounds-check z against depth",
        ),
    ] {
        if instruction.result.is_some()
            || !matches!(
                &instruction.kind,
                InstructionKind::BoundsCheck { index: checked_index, length: checked_length }
                    if *checked_index == index && *checked_length == length
            )
        {
            return Err(SpirvError::UnsupportedKernelShape(message));
        }
    }
    let z_rows = &block.instructions[11];
    let Some(z_rows_result) = z_rows.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d global-index depth multiply must produce a value",
        ));
    };
    if !matches!(
        &z_rows.kind,
        InstructionKind::Binary { op, left, right }
            if *op == BinaryOp::Multiply
                && z_rows_result.ty == u32_type
                && ((*left == coordinate_values[2] && *right == metadata_values[1])
                    || (*right == coordinate_values[2] && *left == metadata_values[1]))
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d global-index twelfth instruction must multiply z by height",
        ));
    }
    let row_number = &block.instructions[12];
    let Some(row_number_result) = row_number.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d global-index row add must produce a value",
        ));
    };
    if !matches!(
        &row_number.kind,
        InstructionKind::Binary { op, left, right }
            if *op == BinaryOp::Add
                && row_number_result.ty == u32_type
                && ((*left == z_rows_result.value && *right == coordinate_values[1])
                    || (*right == z_rows_result.value && *left == coordinate_values[1]))
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d global-index thirteenth instruction must add y row",
        ));
    }
    let row_offset = &block.instructions[13];
    let Some(row_offset_result) = row_offset.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d global-index row multiply must produce a value",
        ));
    };
    if !matches!(
        &row_offset.kind,
        InstructionKind::Binary { op, left, right }
            if *op == BinaryOp::Multiply
                && row_offset_result.ty == u32_type
                && ((*left == row_number_result.value && *right == metadata_values[0])
                    || (*right == row_number_result.value && *left == metadata_values[0]))
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d global-index fourteenth instruction must multiply row by width",
        ));
    }
    let flattened = &block.instructions[14];
    let Some(flattened_result) = flattened.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d global-index flatten add must produce a value",
        ));
    };
    if !matches!(
        &flattened.kind,
        InstructionKind::Binary { op, left, right }
            if *op == BinaryOp::Add
                && flattened_result.ty == u32_type
                && ((*left == row_offset_result.value && *right == coordinate_values[0])
                    || (*right == row_offset_result.value && *left == coordinate_values[0]))
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d global-index fifteenth instruction must flatten row-major index",
        ));
    }
    let physical_bounds = &block.instructions[15];
    if physical_bounds.result.is_some()
        || !matches!(
            &physical_bounds.kind,
            InstructionKind::BoundsCheck { index, length }
                if *index == flattened_result.value && *length == metadata_values[3]
        )
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d global-index sixteenth instruction must bounds-check capacity",
        ));
    }
    let offset = &block.instructions[16];
    let Some(offset_result) = offset.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d global-index offset must produce a pointer",
        ));
    };
    if !matches!(
        &offset.kind,
        InstructionKind::Offset { base, indices }
            if *base == function_data.parameters[0].value
                && indices == &[flattened_result.value]
                && offset_result.ty == function_data.parameters[0].ty
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d global-index offset is invalid",
        ));
    }
    let store = &block.instructions[17];
    if store.result.is_some()
        || !matches!(
            &store.kind,
            InstructionKind::Store { pointer, value: stored_value, .. }
                if *pointer == offset_result.value && *stored_value == value_result.value
        )
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d global-index store is invalid",
        ));
    }
    if !matches!(block.terminator, Terminator::Return { value: None }) {
        return Err(SpirvError::UnsupportedKernelShape(
            "3d global-index write must return Unit",
        ));
    }
    let mut words = emit_storage_global_3d_write(&function_data.name, options, value)?;
    apply_reflected_resource_access_decorations(&mut words, &resources)?;
    Ok(words)
}

/// Emits a bounds-safe three-resource global-index integer arithmetic kernel
/// where the third storage resource contains `length[0]` at runtime.
fn emit_storage_global_index_arithmetic_dynamic_length(
    entry_name: &str,
    options: SpirvOptions,
    operand: u32,
    operation: IntegerArithmeticOp,
) -> Result<Vec<u32>, SpirvError> {
    validate_integer_binary_operand(operation, operand)?;
    if entry_name.is_empty() || entry_name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    let function_id = 1;
    let void_type = 2;
    let function_type = 3;
    let entry_label = 4;
    let uint_type = 5;
    let uint_array = 6;
    let uint_struct = 7;
    let storage_struct_pointer = 8;
    let uint_element_pointer = 9;
    let input_variable = 10;
    let output_variable = 11;
    let length_variable = 12;
    let vector_type = 13;
    let input_pointer = 14;
    let global_variable = 15;
    let global_value = 16;
    let index = 17;
    let zero = 18;
    let constant_addend = 19;
    let length_address = 20;
    let length_value = 21;
    let bool_type = 22;
    let in_bounds = 23;
    let body_label = 24;
    let merge_label = 25;
    let input_address = 26;
    let output_address = 27;
    let loaded_input = 28;
    let sum = 29;
    let mut words = vec![SPIRV_MAGIC, SPIRV_VERSION_1_3, 0, 30, 0];
    instruction(&mut words, OP_CAPABILITY, &[CAPABILITY_SHADER]);
    instruction(
        &mut words,
        OP_MEMORY_MODEL,
        &[ADDRESSING_LOGICAL, MEMORY_MODEL_GLSL450],
    );
    instruction(
        &mut words,
        OP_DECORATE,
        &[uint_array, DECORATION_ARRAY_STRIDE, 4],
    );
    instruction(
        &mut words,
        OP_MEMBER_DECORATE,
        &[uint_struct, 0, DECORATION_OFFSET, 0],
    );
    instruction(&mut words, OP_DECORATE, &[uint_struct, DECORATION_BLOCK]);
    instruction(
        &mut words,
        OP_DECORATE,
        &[
            global_variable,
            DECORATION_BUILT_IN,
            BUILT_IN_GLOBAL_INVOCATION_ID,
        ],
    );
    for (variable, binding) in [
        (input_variable, 0),
        (output_variable, 1),
        (length_variable, 2),
    ] {
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_DESCRIPTOR_SET, 0],
        );
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_BINDING, binding],
        );
    }
    for variable in [input_variable, length_variable] {
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_NON_WRITABLE],
        );
    }
    instruction(
        &mut words,
        OP_DECORATE,
        &[output_variable, DECORATION_NON_READABLE],
    );
    instruction_string_with_tail(
        &mut words,
        OP_ENTRY_POINT,
        &[EXECUTION_MODEL_GL_COMPUTE, function_id],
        entry_name,
        &[
            input_variable,
            output_variable,
            length_variable,
            global_variable,
        ],
    );
    instruction(
        &mut words,
        OP_EXECUTION_MODE,
        &[
            function_id,
            EXECUTION_MODE_LOCAL_SIZE,
            options.workgroup_size[0],
            options.workgroup_size[1],
            options.workgroup_size[2],
        ],
    );
    instruction(&mut words, OP_TYPE_VOID, &[void_type]);
    instruction(&mut words, OP_TYPE_INT, &[uint_type, 32, 0]);
    instruction(&mut words, OP_TYPE_RUNTIME_ARRAY, &[uint_array, uint_type]);
    instruction(&mut words, OP_TYPE_STRUCT, &[uint_struct, uint_array]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[storage_struct_pointer, STORAGE_BUFFER, uint_struct],
    );
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[uint_element_pointer, STORAGE_BUFFER, uint_type],
    );
    instruction(&mut words, OP_TYPE_VECTOR, &[vector_type, uint_type, 3]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[input_pointer, INPUT, vector_type],
    );
    for variable in [input_variable, output_variable, length_variable] {
        instruction(
            &mut words,
            OP_VARIABLE,
            &[storage_struct_pointer, variable, STORAGE_BUFFER],
        );
    }
    instruction(
        &mut words,
        OP_VARIABLE,
        &[input_pointer, global_variable, INPUT],
    );
    instruction(&mut words, OP_TYPE_BOOL, &[bool_type]);
    instruction(&mut words, OP_CONSTANT, &[uint_type, zero, 0]);
    instruction(
        &mut words,
        OP_CONSTANT,
        &[uint_type, constant_addend, operand],
    );
    instruction(&mut words, OP_TYPE_FUNCTION, &[function_type, void_type]);
    instruction(
        &mut words,
        OP_FUNCTION,
        &[void_type, function_id, FUNCTION_CONTROL_NONE, function_type],
    );
    instruction(&mut words, OP_LABEL, &[entry_label]);
    instruction(
        &mut words,
        OP_LOAD,
        &[vector_type, global_value, global_variable],
    );
    instruction(
        &mut words,
        OP_COMPOSITE_EXTRACT,
        &[uint_type, index, global_value, 0],
    );
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            uint_element_pointer,
            length_address,
            length_variable,
            zero,
            zero,
        ],
    );
    instruction(
        &mut words,
        OP_LOAD,
        &[uint_type, length_value, length_address],
    );
    instruction(
        &mut words,
        OP_ULT,
        &[bool_type, in_bounds, index, length_value],
    );
    instruction(&mut words, OP_SELECTION_MERGE, &[merge_label, 0]);
    instruction(
        &mut words,
        OP_BRANCH_CONDITIONAL,
        &[in_bounds, body_label, merge_label],
    );
    instruction(&mut words, OP_LABEL, &[body_label]);
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            uint_element_pointer,
            input_address,
            input_variable,
            zero,
            index,
        ],
    );
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            uint_element_pointer,
            output_address,
            output_variable,
            zero,
            index,
        ],
    );
    instruction(
        &mut words,
        OP_LOAD,
        &[uint_type, loaded_input, input_address],
    );
    instruction(
        &mut words,
        operation.spirv_opcode(),
        &[uint_type, sum, loaded_input, constant_addend],
    );
    instruction(&mut words, OP_STORE, &[output_address, sum]);
    instruction(&mut words, OP_BRANCH, &[merge_label]);
    instruction(&mut words, OP_LABEL, &[merge_label]);
    instruction(&mut words, OP_RETURN, &[]);
    instruction(&mut words, OP_FUNCTION_END, &[]);
    validate_spirv(&words).map_err(|_| {
        SpirvError::UnsupportedKernelShape("global-index dynamic-length validation failed")
    })?;
    Ok(words)
}

/// Emits a bounds-safe three-resource global-index `u32` binary kernel where
/// the third storage resource contains `length[0]` at runtime.
pub fn emit_storage_global_index_binary_dynamic_length(
    entry_name: &str,
    options: SpirvOptions,
    operand: u32,
    operation: BinaryOp,
) -> Result<Vec<u32>, SpirvError> {
    let operation = IntegerArithmeticOp::from_jir(operation).ok_or(
        SpirvError::UnsupportedKernelShape("dynamic-length binary operation is unsupported"),
    )?;
    emit_storage_global_index_arithmetic_dynamic_length(entry_name, options, operand, operation)
}

/// Emits a bounds-safe three-resource global-index `u32` add kernel where the
/// third storage resource contains `length[0]` at runtime.
pub fn emit_storage_global_index_add_dynamic_length(
    entry_name: &str,
    options: SpirvOptions,
    addend: u32,
) -> Result<Vec<u32>, SpirvError> {
    emit_storage_global_index_binary_dynamic_length(entry_name, options, addend, BinaryOp::Add)
}

/// Emits a bounds-safe three-resource global-index `u32` multiply kernel where
/// the third storage resource contains `length[0]` at runtime.
pub fn emit_storage_global_index_multiply_dynamic_length(
    entry_name: &str,
    options: SpirvOptions,
    multiplier: u32,
) -> Result<Vec<u32>, SpirvError> {
    emit_storage_global_index_binary_dynamic_length(
        entry_name,
        options,
        multiplier,
        BinaryOp::Multiply,
    )
}

/// Emits a bounds-safe three-resource global-index scalar `f32` binary kernel
/// where the third storage resource contains `length[0]` at runtime.
///
/// The kernel writes `output[GlobalInvocationId.x] = input[...] <op> operand`
/// only when the invocation index is below the caller-provided logical length.
/// Input and output are `f32` storage buffers; the length buffer is `u32`.
pub fn emit_storage_global_index_f32_binary_dynamic_length(
    entry_name: &str,
    options: SpirvOptions,
    operand_bits: u32,
    operation: F32ArithmeticOp,
) -> Result<Vec<u32>, SpirvError> {
    if entry_name.is_empty() || entry_name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    let function_id = 1;
    let void_type = 2;
    let function_type = 3;
    let entry_label = 4;
    let uint_type = 5;
    let uint_array = 6;
    let uint_struct = 7;
    let uint_struct_pointer = 8;
    let uint_element_pointer = 9;
    let float_type = 10;
    let float_array = 11;
    let float_struct = 12;
    let float_struct_pointer = 13;
    let float_element_pointer = 14;
    let input_variable = 15;
    let output_variable = 16;
    let length_variable = 17;
    let vector_type = 18;
    let input_pointer = 19;
    let global_variable = 20;
    let global_value = 21;
    let index = 22;
    let zero = 23;
    let constant_addend = 24;
    let length_address = 25;
    let length_value = 26;
    let bool_type = 27;
    let in_bounds = 28;
    let body_label = 29;
    let merge_label = 30;
    let input_address = 31;
    let output_address = 32;
    let loaded_input = 33;
    let sum = 34;
    let mut words = vec![SPIRV_MAGIC, SPIRV_VERSION_1_3, 0, 35, 0];
    instruction(&mut words, OP_CAPABILITY, &[CAPABILITY_SHADER]);
    instruction(
        &mut words,
        OP_MEMORY_MODEL,
        &[ADDRESSING_LOGICAL, MEMORY_MODEL_GLSL450],
    );
    for (array_type, struct_type) in [(uint_array, uint_struct), (float_array, float_struct)] {
        instruction(
            &mut words,
            OP_DECORATE,
            &[array_type, DECORATION_ARRAY_STRIDE, 4],
        );
        instruction(
            &mut words,
            OP_MEMBER_DECORATE,
            &[struct_type, 0, DECORATION_OFFSET, 0],
        );
        instruction(&mut words, OP_DECORATE, &[struct_type, DECORATION_BLOCK]);
    }
    instruction(
        &mut words,
        OP_DECORATE,
        &[
            global_variable,
            DECORATION_BUILT_IN,
            BUILT_IN_GLOBAL_INVOCATION_ID,
        ],
    );
    for (variable, binding) in [
        (input_variable, 0),
        (output_variable, 1),
        (length_variable, 2),
    ] {
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_DESCRIPTOR_SET, 0],
        );
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_BINDING, binding],
        );
    }
    for variable in [input_variable, length_variable] {
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_NON_WRITABLE],
        );
    }
    instruction(
        &mut words,
        OP_DECORATE,
        &[output_variable, DECORATION_NON_READABLE],
    );
    instruction_string_with_tail(
        &mut words,
        OP_ENTRY_POINT,
        &[EXECUTION_MODEL_GL_COMPUTE, function_id],
        entry_name,
        &[
            input_variable,
            output_variable,
            length_variable,
            global_variable,
        ],
    );
    instruction(
        &mut words,
        OP_EXECUTION_MODE,
        &[
            function_id,
            EXECUTION_MODE_LOCAL_SIZE,
            options.workgroup_size[0],
            options.workgroup_size[1],
            options.workgroup_size[2],
        ],
    );
    instruction(&mut words, OP_TYPE_VOID, &[void_type]);
    instruction(&mut words, OP_TYPE_INT, &[uint_type, 32, 0]);
    instruction(&mut words, OP_TYPE_RUNTIME_ARRAY, &[uint_array, uint_type]);
    instruction(&mut words, OP_TYPE_STRUCT, &[uint_struct, uint_array]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[uint_struct_pointer, STORAGE_BUFFER, uint_struct],
    );
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[uint_element_pointer, STORAGE_BUFFER, uint_type],
    );
    instruction(&mut words, OP_TYPE_FLOAT, &[float_type, 32]);
    instruction(
        &mut words,
        OP_TYPE_RUNTIME_ARRAY,
        &[float_array, float_type],
    );
    instruction(&mut words, OP_TYPE_STRUCT, &[float_struct, float_array]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[float_struct_pointer, STORAGE_BUFFER, float_struct],
    );
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[float_element_pointer, STORAGE_BUFFER, float_type],
    );
    instruction(&mut words, OP_TYPE_VECTOR, &[vector_type, uint_type, 3]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[input_pointer, INPUT, vector_type],
    );
    for variable in [input_variable, output_variable] {
        instruction(
            &mut words,
            OP_VARIABLE,
            &[float_struct_pointer, variable, STORAGE_BUFFER],
        );
    }
    instruction(
        &mut words,
        OP_VARIABLE,
        &[uint_struct_pointer, length_variable, STORAGE_BUFFER],
    );
    instruction(
        &mut words,
        OP_VARIABLE,
        &[input_pointer, global_variable, INPUT],
    );
    instruction(&mut words, OP_TYPE_BOOL, &[bool_type]);
    instruction(&mut words, OP_CONSTANT, &[uint_type, zero, 0]);
    instruction(
        &mut words,
        OP_CONSTANT,
        &[float_type, constant_addend, operand_bits],
    );
    instruction(&mut words, OP_TYPE_FUNCTION, &[function_type, void_type]);
    instruction(
        &mut words,
        OP_FUNCTION,
        &[void_type, function_id, FUNCTION_CONTROL_NONE, function_type],
    );
    instruction(&mut words, OP_LABEL, &[entry_label]);
    instruction(
        &mut words,
        OP_LOAD,
        &[vector_type, global_value, global_variable],
    );
    instruction(
        &mut words,
        OP_COMPOSITE_EXTRACT,
        &[uint_type, index, global_value, 0],
    );
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            uint_element_pointer,
            length_address,
            length_variable,
            zero,
            zero,
        ],
    );
    instruction(
        &mut words,
        OP_LOAD,
        &[uint_type, length_value, length_address],
    );
    instruction(
        &mut words,
        OP_ULT,
        &[bool_type, in_bounds, index, length_value],
    );
    instruction(&mut words, OP_SELECTION_MERGE, &[merge_label, 0]);
    instruction(
        &mut words,
        OP_BRANCH_CONDITIONAL,
        &[in_bounds, body_label, merge_label],
    );
    instruction(&mut words, OP_LABEL, &[body_label]);
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            float_element_pointer,
            input_address,
            input_variable,
            zero,
            index,
        ],
    );
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            float_element_pointer,
            output_address,
            output_variable,
            zero,
            index,
        ],
    );
    instruction(
        &mut words,
        OP_LOAD,
        &[float_type, loaded_input, input_address],
    );
    instruction(
        &mut words,
        f32_spirv_opcode(operation),
        &[float_type, sum, loaded_input, constant_addend],
    );
    instruction(&mut words, OP_STORE, &[output_address, sum]);
    instruction(&mut words, OP_BRANCH, &[merge_label]);
    instruction(&mut words, OP_LABEL, &[merge_label]);
    instruction(&mut words, OP_RETURN, &[]);
    instruction(&mut words, OP_FUNCTION_END, &[]);
    validate_spirv(&words).map_err(|_| {
        SpirvError::UnsupportedKernelShape(
            "global-index f32 binary dynamic-length validation failed",
        )
    })?;
    Ok(words)
}

/// Emits a bounds-safe three-resource global-index `f32` add kernel.
pub fn emit_storage_global_index_fadd_dynamic_length(
    entry_name: &str,
    options: SpirvOptions,
    addend_bits: u32,
) -> Result<Vec<u32>, SpirvError> {
    emit_storage_global_index_f32_binary_dynamic_length(
        entry_name,
        options,
        addend_bits,
        F32ArithmeticOp::Add,
    )
}

/// Emits a bounds-safe three-resource global-index `f32` subtract kernel.
pub fn emit_storage_global_index_fsub_dynamic_length(
    entry_name: &str,
    options: SpirvOptions,
    subtrahend_bits: u32,
) -> Result<Vec<u32>, SpirvError> {
    emit_storage_global_index_f32_binary_dynamic_length(
        entry_name,
        options,
        subtrahend_bits,
        F32ArithmeticOp::Subtract,
    )
}

/// Emits a bounds-safe three-resource global-index `f32` multiply kernel.
pub fn emit_storage_global_index_fmul_dynamic_length(
    entry_name: &str,
    options: SpirvOptions,
    multiplier_bits: u32,
) -> Result<Vec<u32>, SpirvError> {
    emit_storage_global_index_f32_binary_dynamic_length(
        entry_name,
        options,
        multiplier_bits,
        F32ArithmeticOp::Multiply,
    )
}

/// Lowers the dynamic-length JIR shape:
/// `idx = builtin global_invocation_id.x; length = load length[0];
/// bounds_check idx, length; output[idx] = input[idx] <op> operand`.
#[allow(dead_code)]
fn emit_storage_global_index_arithmetic_dynamic_length_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    operation: IntegerArithmeticOp,
) -> Result<Vec<u32>, SpirvError> {
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(SpirvError::GpuVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    if function_data.name.is_empty() || function_data.name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) || function_data.parameters.len() != 3
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length global-index binary requires three resource parameters and Unit result",
        ));
    }
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    if resources.len() != 3
        || resources
            .iter()
            .any(|resource| resource.address_space != AddressSpace::Storage)
        || resources.iter().any(|resource| {
            !matches!(
                module.types.get(resource.element_type.index()),
                Some(Type::Integer {
                    signed: false,
                    bits: 32
                })
            )
        })
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length global-index binary requires three storage u32 resources",
        ));
    }
    let Some(block) = function_data.blocks.first() else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length global-index binary requires one entry block",
        ));
    };
    if function_data.blocks.len() != 1
        || !block.parameters.is_empty()
        || block.instructions.len() != 9
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length global-index binary body has an unsupported shape",
        ));
    }
    let u32_type = resources[0].element_type;
    let builtin = &block.instructions[0];
    let Some(builtin_result) = builtin.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length builtin must produce a value",
        ));
    };
    if !matches!(
        &builtin.kind,
        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX)
    ) || builtin_result.ty != u32_type
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length first instruction must produce u32 GlobalInvocationId.x",
        ));
    }
    let constant = &block.instructions[1];
    let Some(constant_result) = constant.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length binary operand constant must produce a value",
        ));
    };
    let operand = match (&constant.kind, constant_result.ty == u32_type) {
        (InstructionKind::Constant(Constant::Integer { value }), true) => u32::try_from(*value)
            .map_err(|_| {
                SpirvError::UnsupportedKernelShape("dynamic-length binary operand is outside u32")
            })?,
        _ => {
            return Err(SpirvError::UnsupportedKernelShape(
                "dynamic-length second instruction must be a u32 constant",
            ));
        }
    };
    let length_load = &block.instructions[2];
    let Some(length_result) = length_load.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length load must produce a value",
        ));
    };
    if !matches!(
        &length_load.kind,
        InstructionKind::Load { pointer, .. }
            if *pointer == function_data.parameters[2].value && length_result.ty == u32_type
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length third instruction must load length resource",
        ));
    }
    let bounds = &block.instructions[3];
    if bounds.result.is_some()
        || !matches!(
            &bounds.kind,
            InstructionKind::BoundsCheck { index, length: bound_length }
                if *index == builtin_result.value && *bound_length == length_result.value
        )
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length fourth instruction must bounds-check the builtin index",
        ));
    }
    let input_offset = &block.instructions[4];
    let Some(input_offset_result) = input_offset.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length input offset must produce a pointer",
        ));
    };
    if !matches!(
        &input_offset.kind,
        InstructionKind::Offset { base, indices }
            if *base == function_data.parameters[0].value
                && indices == &[builtin_result.value]
                && input_offset_result.ty == function_data.parameters[0].ty
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length input offset is invalid",
        ));
    }
    let input_load = &block.instructions[5];
    let Some(input_result) = input_load.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length input load must produce a value",
        ));
    };
    if !matches!(
        &input_load.kind,
        InstructionKind::Load { pointer, .. }
            if *pointer == input_offset_result.value && input_result.ty == u32_type
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length input load is invalid",
        ));
    }
    let binary = &block.instructions[6];
    let Some(binary_result) = binary.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length binary operation must produce a value",
        ));
    };
    if !matches!(
        &binary.kind,
        InstructionKind::Binary { op, left, right }
            if *op == operation.jir_op()
                && binary_result.ty == u32_type
                && ((*left == input_result.value && *right == constant_result.value)
                    || (*right == input_result.value && *left == constant_result.value))
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length binary operands are invalid",
        ));
    }
    let output_offset = &block.instructions[7];
    let Some(output_offset_result) = output_offset.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length output offset must produce a pointer",
        ));
    };
    if !matches!(
        &output_offset.kind,
        InstructionKind::Offset { base, indices }
            if *base == function_data.parameters[1].value
                && indices == &[builtin_result.value]
                && output_offset_result.ty == function_data.parameters[1].ty
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length output offset is invalid",
        ));
    }
    let store = &block.instructions[8];
    if store.result.is_some()
        || !matches!(
            &store.kind,
            InstructionKind::Store { pointer, value, .. }
                if *pointer == output_offset_result.value && *value == binary_result.value
        )
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length store is invalid",
        ));
    }
    if !matches!(block.terminator, Terminator::Return { value: None }) {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length binary must return Unit",
        ));
    }
    emit_storage_global_index_arithmetic_dynamic_length(
        &function_data.name,
        options,
        operand,
        operation,
    )
}

/// Lowers the strict dynamic-length scalar `f32` JIR shape:
/// `idx = GlobalInvocationId.x; length = load length[0]; bounds_check idx,
/// length; output[idx] = input[idx] <op> operand`.
pub fn emit_storage_global_index_f32_binary_dynamic_length_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    operation: F32ArithmeticOp,
) -> Result<Vec<u32>, SpirvError> {
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(SpirvError::GpuVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    if function_data.name.is_empty() || function_data.name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) || function_data.parameters.len() != 3
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length fadd requires three resource parameters and Unit result",
        ));
    }
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    if resources.len() != 3
        || resources
            .iter()
            .any(|resource| resource.address_space != AddressSpace::Storage)
        || !matches!(
            module.types.get(resources[0].element_type.index()),
            Some(Type::Float { bits: 32 })
        )
        || !matches!(
            module.types.get(resources[1].element_type.index()),
            Some(Type::Float { bits: 32 })
        )
        || !matches!(
            module.types.get(resources[2].element_type.index()),
            Some(Type::Integer {
                signed: false,
                bits: 32
            })
        )
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length fadd requires f32 input/output and u32 length storage resources",
        ));
    }
    let Some(block) = function_data.blocks.first() else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length fadd requires one entry block",
        ));
    };
    if function_data.blocks.len() != 1
        || !block.parameters.is_empty()
        || block.instructions.len() != 9
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length fadd body has an unsupported shape",
        ));
    }
    let value_type = resources[0].element_type;
    let index_type = resources[2].element_type;
    let builtin = &block.instructions[0];
    let Some(builtin_result) = builtin.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length fadd builtin must produce a value",
        ));
    };
    if !matches!(
        builtin.kind,
        InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX)
    ) || builtin_result.ty != index_type
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length fadd first instruction must produce u32 GlobalInvocationId.x",
        ));
    }
    let constant = &block.instructions[1];
    let Some(constant_result) = constant.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length fadd addend must produce a value",
        ));
    };
    let operand_bits = match (&constant.kind, constant_result.ty == value_type) {
        (InstructionKind::Constant(Constant::FloatBits { bits }), true) => u32::try_from(*bits)
            .map_err(|_| {
                SpirvError::UnsupportedKernelShape("dynamic-length fadd bits exceed f32")
            })?,
        _ => {
            return Err(SpirvError::UnsupportedKernelShape(
                "dynamic-length fadd second instruction must be an f32 constant",
            ));
        }
    };
    let length_load = &block.instructions[2];
    let Some(length_result) = length_load.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length fadd length load must produce a value",
        ));
    };
    if !matches!(
        &length_load.kind,
        InstructionKind::Load { pointer, .. }
            if *pointer == function_data.parameters[2].value && length_result.ty == index_type
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length fadd third instruction must load the u32 length resource",
        ));
    }
    let bounds = &block.instructions[3];
    if bounds.result.is_some()
        || !matches!(
            &bounds.kind,
            InstructionKind::BoundsCheck { index, length }
                if *index == builtin_result.value && *length == length_result.value
        )
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length fadd fourth instruction must bounds-check the invocation index",
        ));
    }
    let input_offset = &block.instructions[4];
    let Some(input_offset_result) = input_offset.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length fadd input offset must produce a pointer",
        ));
    };
    if !matches!(
        &input_offset.kind,
        InstructionKind::Offset { base, indices }
            if *base == function_data.parameters[0].value
                && indices == &[builtin_result.value]
                && input_offset_result.ty == function_data.parameters[0].ty
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length fadd input offset is invalid",
        ));
    }
    let input_load = &block.instructions[5];
    let Some(input_result) = input_load.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length fadd input load must produce a value",
        ));
    };
    if !matches!(
        &input_load.kind,
        InstructionKind::Load { pointer, .. }
            if *pointer == input_offset_result.value && input_result.ty == value_type
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length fadd input load is invalid",
        ));
    }
    let binary = &block.instructions[6];
    let Some(binary_result) = binary.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length fadd must produce a value",
        ));
    };
    if !matches!(
        &binary.kind,
        InstructionKind::Binary { op, left, right }
            if binary_result.ty == value_type
                && *op == operation.as_binary_op()
                && ((*left == input_result.value && *right == constant_result.value)
                    || (*right == input_result.value && *left == constant_result.value))
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length fadd operands are invalid",
        ));
    }
    let output_offset = &block.instructions[7];
    let Some(output_offset_result) = output_offset.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length fadd output offset must produce a pointer",
        ));
    };
    if !matches!(
        &output_offset.kind,
        InstructionKind::Offset { base, indices }
            if *base == function_data.parameters[1].value
                && indices == &[builtin_result.value]
                && output_offset_result.ty == function_data.parameters[1].ty
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length fadd output offset is invalid",
        ));
    }
    let store = &block.instructions[8];
    if store.result.is_some()
        || !matches!(
            &store.kind,
            InstructionKind::Store { pointer, value, .. }
                if *pointer == output_offset_result.value && *value == binary_result.value
        )
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length fadd store is invalid",
        ));
    }
    if !matches!(block.terminator, Terminator::Return { value: None }) {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length fadd must return Unit",
        ));
    }
    emit_storage_global_index_f32_binary_dynamic_length(
        &function_data.name,
        options,
        operand_bits,
        operation,
    )
}

/// Lowers the strict dynamic-length `f32` add JIR shape.
pub fn emit_storage_global_index_fadd_dynamic_length_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<Vec<u32>, SpirvError> {
    emit_storage_global_index_f32_binary_dynamic_length_from_jir(
        module,
        function,
        options,
        F32ArithmeticOp::Add,
    )
}

/// Lowers the strict dynamic-length `f32` subtract JIR shape.
pub fn emit_storage_global_index_fsub_dynamic_length_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<Vec<u32>, SpirvError> {
    emit_storage_global_index_f32_binary_dynamic_length_from_jir(
        module,
        function,
        options,
        F32ArithmeticOp::Subtract,
    )
}

/// Lowers the strict dynamic-length `f32` multiply JIR shape.
pub fn emit_storage_global_index_fmul_dynamic_length_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<Vec<u32>, SpirvError> {
    emit_storage_global_index_f32_binary_dynamic_length_from_jir(
        module,
        function,
        options,
        F32ArithmeticOp::Multiply,
    )
}

/// Lowers the generalized one-block dynamic-length `u32` resource shape.
///
/// The semantic operations may be reordered in SSA order (and unused integer
/// constants are tolerated), but the bounds check must dominate every memory
/// access.  SPIR-V is emitted in canonical, safety-preserving order so the
/// source ordering does not leak into the backend contract.
fn emit_storage_global_index_arithmetic_dynamic_length_generic_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    operation: IntegerArithmeticOp,
) -> Result<Vec<u32>, SpirvError> {
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(SpirvError::GpuVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    if function_data.name.is_empty() || function_data.name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) || function_data.parameters.len() != 3
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length global-index binary requires three resource parameters and Unit result",
        ));
    }
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    if resources.len() != 3
        || resources
            .iter()
            .any(|resource| resource.address_space != AddressSpace::Storage)
        || resources.iter().any(|resource| {
            !matches!(
                module.types.get(resource.element_type.index()),
                Some(Type::Integer {
                    signed: false,
                    bits: 32
                })
            )
        })
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length global-index binary requires three storage u32 resources",
        ));
    }
    let Some(block) = function_data.blocks.first() else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length global-index binary requires one entry block",
        ));
    };
    if function_data.blocks.len() != 1
        || !block.parameters.is_empty()
        || !matches!(block.terminator, Terminator::Return { value: None })
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length global-index binary requires one Unit-returning entry block",
        ));
    }
    let u32_type = resources[0].element_type;
    let input_parameter = &function_data.parameters[0];
    let output_parameter = &function_data.parameters[1];
    let length_parameter = &function_data.parameters[2];

    let mut builtin = None;
    let mut constants = Vec::new();
    let mut length_load = None;
    let mut input_offset = None;
    let mut output_offset = None;
    let mut input_loads = Vec::new();
    let mut bounds = None;
    let mut binary = None;
    let mut store = None;

    for (index, instruction) in block.instructions.iter().enumerate() {
        match &instruction.kind {
            InstructionKind::Builtin(op) => {
                let Some(result) = instruction.result else {
                    return Err(SpirvError::UnsupportedKernelShape(
                        "dynamic-length builtin must produce a value",
                    ));
                };
                let lane = match op {
                    BuiltinOp::GlobalInvocationIdX => 0,
                    BuiltinOp::GlobalInvocationIdY => 1,
                    BuiltinOp::GlobalInvocationIdZ => 2,
                };
                if result.ty != u32_type || builtin.replace((result, lane, index)).is_some() {
                    return Err(SpirvError::UnsupportedKernelShape(
                        "dynamic-length binary requires one u32 global invocation builtin",
                    ));
                }
            }
            InstructionKind::Constant(Constant::Integer { value }) => {
                let Some(result) = instruction.result else {
                    return Err(SpirvError::UnsupportedKernelShape(
                        "dynamic-length constants must produce values",
                    ));
                };
                if result.ty != u32_type {
                    return Err(SpirvError::UnsupportedKernelShape(
                        "dynamic-length constants must be u32",
                    ));
                }
                let value = u32::try_from(*value).map_err(|_| {
                    SpirvError::UnsupportedKernelShape(
                        "dynamic-length binary operand is outside u32",
                    )
                })?;
                constants.push((result, value, index));
            }
            InstructionKind::Load { pointer, .. } if *pointer == length_parameter.value => {
                let Some(result) = instruction.result else {
                    return Err(SpirvError::UnsupportedKernelShape(
                        "dynamic-length load must produce a value",
                    ));
                };
                if result.ty != u32_type || length_load.replace((result, index)).is_some() {
                    return Err(SpirvError::UnsupportedKernelShape(
                        "dynamic-length length resource must have one u32 load",
                    ));
                }
            }
            InstructionKind::Load { pointer, .. } => input_loads.push((
                instruction
                    .result
                    .ok_or(SpirvError::UnsupportedKernelShape(
                        "dynamic-length input load must produce a value",
                    ))?,
                *pointer,
                index,
            )),
            InstructionKind::Offset { base, indices } => {
                let Some(result) = instruction.result else {
                    return Err(SpirvError::UnsupportedKernelShape(
                        "dynamic-length offsets must produce pointers",
                    ));
                };
                if result.ty != input_parameter.ty && result.ty != output_parameter.ty {
                    return Err(SpirvError::UnsupportedKernelShape(
                        "dynamic-length offset pointer type is invalid",
                    ));
                }
                let is_valid_index = builtin
                    .as_ref()
                    .is_some_and(|(value, _, _)| indices.as_slice() == [value.value]);
                if !is_valid_index {
                    return Err(SpirvError::UnsupportedKernelShape(
                        "dynamic-length offsets must use the global invocation index",
                    ));
                }
                if *base == input_parameter.value {
                    if result.ty != input_parameter.ty
                        || input_offset.replace((result, index)).is_some()
                    {
                        return Err(SpirvError::UnsupportedKernelShape(
                            "dynamic-length input offset is invalid",
                        ));
                    }
                } else if *base == output_parameter.value {
                    if result.ty != output_parameter.ty
                        || output_offset.replace((result, index)).is_some()
                    {
                        return Err(SpirvError::UnsupportedKernelShape(
                            "dynamic-length output offset is invalid",
                        ));
                    }
                } else {
                    return Err(SpirvError::UnsupportedKernelShape(
                        "dynamic-length offset base is invalid",
                    ));
                }
            }
            InstructionKind::BoundsCheck {
                index: checked_index,
                length,
            } => {
                if bounds.replace((*checked_index, *length, index)).is_some() {
                    return Err(SpirvError::UnsupportedKernelShape(
                        "dynamic-length binary requires one bounds check",
                    ));
                }
            }
            InstructionKind::Binary { op, left, right } if *op == operation.jir_op() => {
                let Some(result) = instruction.result else {
                    return Err(SpirvError::UnsupportedKernelShape(
                        "dynamic-length binary operation must produce a value",
                    ));
                };
                if result.ty != u32_type || binary.replace((result, *left, *right, index)).is_some()
                {
                    return Err(SpirvError::UnsupportedKernelShape(
                        "dynamic-length binary operation is invalid",
                    ));
                }
            }
            InstructionKind::Store { pointer, value, .. } => {
                if store.replace((*pointer, *value, index)).is_some() {
                    return Err(SpirvError::UnsupportedKernelShape(
                        "dynamic-length binary requires one store",
                    ));
                }
            }
            _ => {
                return Err(SpirvError::UnsupportedKernelShape(
                    "dynamic-length binary contains an unsupported instruction",
                ));
            }
        }
    }

    let (builtin_value, builtin_lane, builtin_index) =
        builtin.ok_or(SpirvError::UnsupportedKernelShape(
            "dynamic-length binary requires a global invocation builtin",
        ))?;
    let (length_value, length_index) = length_load.ok_or(SpirvError::UnsupportedKernelShape(
        "dynamic-length binary requires a length load",
    ))?;
    let (input_offset_value, input_offset_index) = input_offset.ok_or(
        SpirvError::UnsupportedKernelShape("dynamic-length binary requires an input offset"),
    )?;
    let (output_offset_value, output_offset_index) = output_offset.ok_or(
        SpirvError::UnsupportedKernelShape("dynamic-length binary requires an output offset"),
    )?;
    let (bounds_index_value, bounds_length_value, bounds_index) = bounds.ok_or(
        SpirvError::UnsupportedKernelShape("dynamic-length binary requires a bounds check"),
    )?;
    if bounds_index_value != builtin_value.value || bounds_length_value != length_value.value {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length bounds check must guard the builtin index with the length",
        ));
    }
    let (input_result, input_load_index) = match input_loads.as_slice() {
        [(result, pointer, index)] if *pointer == input_offset_value.value => {
            if result.ty != u32_type {
                return Err(SpirvError::UnsupportedKernelShape(
                    "dynamic-length input load must produce u32",
                ));
            }
            (*result, *index)
        }
        _ => {
            return Err(SpirvError::UnsupportedKernelShape(
                "dynamic-length binary requires one input offset load",
            ));
        }
    };
    let (binary_result, binary_left, binary_right, binary_index) = binary.ok_or(
        SpirvError::UnsupportedKernelShape("dynamic-length binary requires a binary operation"),
    )?;
    let (_, operand) = constants
        .iter()
        .find_map(|(result, value, _)| {
            if (binary_left == result.value && binary_right == input_result.value)
                || (binary_right == result.value && binary_left == input_result.value)
            {
                Some((*result, *value))
            } else {
                None
            }
        })
        .ok_or(SpirvError::UnsupportedKernelShape(
            "dynamic-length binary must combine the input with a u32 constant",
        ))?;
    validate_integer_binary_operand(operation, operand)?;
    let (store_pointer, store_value, store_index) = store.ok_or(
        SpirvError::UnsupportedKernelShape("dynamic-length binary requires an output store"),
    )?;
    if store_pointer != output_offset_value.value || store_value != binary_result.value {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length store must write the binary result to the output offset",
        ));
    }
    if !(builtin_index < bounds_index
        && length_index < bounds_index
        && bounds_index < input_offset_index
        && bounds_index < output_offset_index
        && bounds_index < input_load_index
        && bounds_index < binary_index
        && bounds_index < store_index)
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-length bounds check must precede all memory effects",
        ));
    }
    let function_id = 1;
    let void_type = 2;
    let uint_type = 3;
    let bool_type = 4;
    let uint_array = 5;
    let uint_struct = 6;
    let storage_struct_pointer = 7;
    let uint_element_pointer = 8;
    let vector_type = 9;
    let input_pointer = 10;
    let function_type = 11;
    let mut next_id = 12;
    let mut fresh = || {
        let id = next_id;
        next_id += 1;
        id
    };
    let input_variable = fresh();
    let output_variable = fresh();
    let length_variable = fresh();
    let global_variable = fresh();
    let zero = fresh();
    let constant_spv = fresh();
    let global_value = fresh();
    let builtin_spv = fresh();
    let length_address = fresh();
    let length_spv = fresh();
    let in_bounds = fresh();
    let entry_label = fresh();
    let body_label = fresh();
    let merge_label = fresh();
    let input_address = fresh();
    let output_address = fresh();
    let input_spv = fresh();
    let binary_spv = fresh();
    let mut words = vec![SPIRV_MAGIC, SPIRV_VERSION_1_3, 0, next_id, 0];
    instruction(&mut words, OP_CAPABILITY, &[CAPABILITY_SHADER]);
    instruction(
        &mut words,
        OP_MEMORY_MODEL,
        &[ADDRESSING_LOGICAL, MEMORY_MODEL_GLSL450],
    );
    instruction(
        &mut words,
        OP_DECORATE,
        &[uint_array, DECORATION_ARRAY_STRIDE, 4],
    );
    instruction(
        &mut words,
        OP_MEMBER_DECORATE,
        &[uint_struct, 0, DECORATION_OFFSET, 0],
    );
    instruction(&mut words, OP_DECORATE, &[uint_struct, DECORATION_BLOCK]);
    instruction(
        &mut words,
        OP_DECORATE,
        &[
            global_variable,
            DECORATION_BUILT_IN,
            BUILT_IN_GLOBAL_INVOCATION_ID,
        ],
    );
    for (variable, binding) in [
        (input_variable, 0),
        (output_variable, 1),
        (length_variable, 2),
    ] {
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_DESCRIPTOR_SET, 0],
        );
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_BINDING, binding],
        );
    }
    for variable in [input_variable, length_variable] {
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_NON_WRITABLE],
        );
    }
    instruction(
        &mut words,
        OP_DECORATE,
        &[output_variable, DECORATION_NON_READABLE],
    );
    instruction_string_with_tail(
        &mut words,
        OP_ENTRY_POINT,
        &[EXECUTION_MODEL_GL_COMPUTE, function_id],
        &function_data.name,
        &[
            input_variable,
            output_variable,
            length_variable,
            global_variable,
        ],
    );
    instruction(
        &mut words,
        OP_EXECUTION_MODE,
        &[
            function_id,
            EXECUTION_MODE_LOCAL_SIZE,
            options.workgroup_size[0],
            options.workgroup_size[1],
            options.workgroup_size[2],
        ],
    );
    instruction(&mut words, OP_TYPE_VOID, &[void_type]);
    instruction(&mut words, OP_TYPE_INT, &[uint_type, 32, 0]);
    instruction(&mut words, OP_TYPE_BOOL, &[bool_type]);
    instruction(&mut words, OP_TYPE_RUNTIME_ARRAY, &[uint_array, uint_type]);
    instruction(&mut words, OP_TYPE_STRUCT, &[uint_struct, uint_array]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[storage_struct_pointer, STORAGE_BUFFER, uint_struct],
    );
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[uint_element_pointer, STORAGE_BUFFER, uint_type],
    );
    instruction(&mut words, OP_TYPE_VECTOR, &[vector_type, uint_type, 3]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[input_pointer, INPUT, vector_type],
    );
    for variable in [input_variable, output_variable, length_variable] {
        instruction(
            &mut words,
            OP_VARIABLE,
            &[storage_struct_pointer, variable, STORAGE_BUFFER],
        );
    }
    instruction(
        &mut words,
        OP_VARIABLE,
        &[input_pointer, global_variable, INPUT],
    );
    instruction(&mut words, OP_CONSTANT, &[uint_type, zero, 0]);
    instruction(&mut words, OP_CONSTANT, &[uint_type, constant_spv, operand]);
    instruction(&mut words, OP_TYPE_FUNCTION, &[function_type, void_type]);
    instruction(
        &mut words,
        OP_FUNCTION,
        &[void_type, function_id, FUNCTION_CONTROL_NONE, function_type],
    );
    instruction(&mut words, OP_LABEL, &[entry_label]);
    instruction(
        &mut words,
        OP_LOAD,
        &[vector_type, global_value, global_variable],
    );
    instruction(
        &mut words,
        OP_COMPOSITE_EXTRACT,
        &[uint_type, builtin_spv, global_value, builtin_lane],
    );
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            uint_element_pointer,
            length_address,
            length_variable,
            zero,
            zero,
        ],
    );
    instruction(
        &mut words,
        OP_LOAD,
        &[uint_type, length_spv, length_address],
    );
    instruction(
        &mut words,
        OP_ULT,
        &[bool_type, in_bounds, builtin_spv, length_spv],
    );
    instruction(&mut words, OP_SELECTION_MERGE, &[merge_label, 0]);
    instruction(
        &mut words,
        OP_BRANCH_CONDITIONAL,
        &[in_bounds, body_label, merge_label],
    );
    instruction(&mut words, OP_LABEL, &[body_label]);
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            uint_element_pointer,
            input_address,
            input_variable,
            zero,
            builtin_spv,
        ],
    );
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            uint_element_pointer,
            output_address,
            output_variable,
            zero,
            builtin_spv,
        ],
    );
    instruction(&mut words, OP_LOAD, &[uint_type, input_spv, input_address]);
    instruction(
        &mut words,
        operation.spirv_opcode(),
        &[uint_type, binary_spv, input_spv, constant_spv],
    );
    instruction(&mut words, OP_STORE, &[output_address, binary_spv]);
    instruction(&mut words, OP_BRANCH, &[merge_label]);
    instruction(&mut words, OP_LABEL, &[merge_label]);
    instruction(&mut words, OP_RETURN, &[]);
    instruction(&mut words, OP_FUNCTION_END, &[]);
    words[3] = next_id;
    validate_spirv(&words).map_err(|_| {
        SpirvError::UnsupportedKernelShape("global-index dynamic-length validation failed")
    })?;
    Ok(words)
}

/// Lowers a verified dynamic-length JIR `u32` binary shape to SPIR-V.
pub fn emit_storage_global_index_binary_dynamic_length_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    operation: BinaryOp,
) -> Result<Vec<u32>, SpirvError> {
    let operation = IntegerArithmeticOp::from_jir(operation).ok_or(
        SpirvError::UnsupportedKernelShape("dynamic-length binary operation is unsupported"),
    )?;
    emit_storage_global_index_arithmetic_dynamic_length_generic_from_jir(
        module, function, options, operation,
    )
}

/// Lowers a verified dynamic-length JIR `u32` add shape to SPIR-V.
pub fn emit_storage_global_index_add_dynamic_length_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<Vec<u32>, SpirvError> {
    emit_storage_global_index_binary_dynamic_length_from_jir(
        module,
        function,
        options,
        BinaryOp::Add,
    )
}

/// Lowers a verified dynamic-length JIR `u32` multiply shape to SPIR-V.
pub fn emit_storage_global_index_multiply_dynamic_length_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<Vec<u32>, SpirvError> {
    emit_storage_global_index_binary_dynamic_length_from_jir(
        module,
        function,
        options,
        BinaryOp::Multiply,
    )
}

/// Emits a bounds-safe runtime-length vector `f32x4` add kernel:
/// `output[index] = input[index] + float4(addend)`.
///
/// This is a deliberately bounded vector family. It proves that the JIR
/// vector operations can cross the SPIR-V artifact boundary with a reflected
/// 16-byte element stride; it is not a general vector lowering path.
pub fn emit_storage_global_index_vector_f32_add_dynamic_length(
    entry_name: &str,
    options: SpirvOptions,
    addend_bits: u32,
) -> Result<Vec<u32>, SpirvError> {
    emit_storage_global_index_vector_f32_binary_dynamic_length(
        entry_name,
        options,
        addend_bits,
        F32ArithmeticOp::Add,
    )
}

/// Emits a bounds-safe runtime-length vector `f32x4` binary kernel:
/// `output[index] = input[index] <op> float4(operand)`.
pub fn emit_storage_global_index_vector_f32_binary_dynamic_length(
    entry_name: &str,
    options: SpirvOptions,
    operand_bits: u32,
    operation: F32ArithmeticOp,
) -> Result<Vec<u32>, SpirvError> {
    emit_storage_global_index_vector_f32_binary_dynamic_length_lanes(
        entry_name,
        options,
        operand_bits,
        operation,
        4,
    )
}

/// Emits a bounds-safe runtime-length `f32` vector binary kernel for two to
/// four lanes. The x4 wrapper above remains the compatibility API; native
/// backend execution for x2/x3 is a separate capability gate.
pub fn emit_storage_global_index_vector_f32_binary_dynamic_length_lanes(
    entry_name: &str,
    options: SpirvOptions,
    operand_bits: u32,
    operation: F32ArithmeticOp,
    lanes: u32,
) -> Result<Vec<u32>, SpirvError> {
    if entry_name.is_empty() || entry_name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    if !(2..=4).contains(&lanes) {
        return Err(SpirvError::UnsupportedKernelShape(
            "vector f32 lanes must be in 2..=4",
        ));
    }
    let vector_stride = lanes
        .checked_mul(4)
        .ok_or(SpirvError::UnsupportedKernelShape(
            "vector f32 lane stride overflows",
        ))?;

    let function_id = 1;
    let void_type = 2;
    let function_type = 3;
    let entry_label = 4;
    let uint_type = 5;
    let bool_type = 6;
    let float_type = 7;
    let vector_type = 8;
    let uint_array = 9;
    let uint_struct = 10;
    let uint_struct_pointer = 11;
    let uint_element_pointer = 12;
    let vector_array = 13;
    let vector_struct = 14;
    let vector_struct_pointer = 15;
    let vector_element_pointer = 16;
    let builtin_vector_type = 17;
    let builtin_pointer = 18;
    let input_variable = 19;
    let output_variable = 20;
    let length_variable = 21;
    let global_variable = 22;
    let zero = 23;
    let scalar_addend = 24;
    let vector_addend = 25;
    let global_value = 26;
    let builtin_spv = 27;
    let length_address = 28;
    let length_spv = 29;
    let in_bounds = 30;
    let body_label = 31;
    let merge_label = 32;
    let input_address = 33;
    let output_address = 34;
    let input_spv = 35;
    let binary_spv = 36;
    let mut words = vec![SPIRV_MAGIC, SPIRV_VERSION_1_3, 0, 37, 0];

    instruction(&mut words, OP_CAPABILITY, &[CAPABILITY_SHADER]);
    instruction(
        &mut words,
        OP_MEMORY_MODEL,
        &[ADDRESSING_LOGICAL, MEMORY_MODEL_GLSL450],
    );
    instruction(
        &mut words,
        OP_DECORATE,
        &[uint_array, DECORATION_ARRAY_STRIDE, 4],
    );
    instruction(
        &mut words,
        OP_MEMBER_DECORATE,
        &[uint_struct, 0, DECORATION_OFFSET, 0],
    );
    instruction(&mut words, OP_DECORATE, &[uint_struct, DECORATION_BLOCK]);
    instruction(
        &mut words,
        OP_DECORATE,
        &[vector_array, DECORATION_ARRAY_STRIDE, vector_stride],
    );
    instruction(
        &mut words,
        OP_MEMBER_DECORATE,
        &[vector_struct, 0, DECORATION_OFFSET, 0],
    );
    instruction(&mut words, OP_DECORATE, &[vector_struct, DECORATION_BLOCK]);
    instruction(
        &mut words,
        OP_DECORATE,
        &[
            global_variable,
            DECORATION_BUILT_IN,
            BUILT_IN_GLOBAL_INVOCATION_ID,
        ],
    );
    for (variable, binding) in [
        (input_variable, 0),
        (output_variable, 1),
        (length_variable, 2),
    ] {
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_DESCRIPTOR_SET, 0],
        );
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_BINDING, binding],
        );
    }
    for variable in [input_variable, length_variable] {
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_NON_WRITABLE],
        );
    }
    instruction(
        &mut words,
        OP_DECORATE,
        &[output_variable, DECORATION_NON_READABLE],
    );
    instruction_string_with_tail(
        &mut words,
        OP_ENTRY_POINT,
        &[EXECUTION_MODEL_GL_COMPUTE, function_id],
        entry_name,
        &[
            input_variable,
            output_variable,
            length_variable,
            global_variable,
        ],
    );
    instruction(
        &mut words,
        OP_EXECUTION_MODE,
        &[
            function_id,
            EXECUTION_MODE_LOCAL_SIZE,
            options.workgroup_size[0],
            options.workgroup_size[1],
            options.workgroup_size[2],
        ],
    );
    instruction(&mut words, OP_TYPE_VOID, &[void_type]);
    instruction(&mut words, OP_TYPE_INT, &[uint_type, 32, 0]);
    instruction(&mut words, OP_TYPE_BOOL, &[bool_type]);
    instruction(&mut words, OP_TYPE_FLOAT, &[float_type, 32]);
    instruction(
        &mut words,
        OP_TYPE_VECTOR,
        &[vector_type, float_type, lanes],
    );
    instruction(&mut words, OP_TYPE_RUNTIME_ARRAY, &[uint_array, uint_type]);
    instruction(&mut words, OP_TYPE_STRUCT, &[uint_struct, uint_array]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[uint_struct_pointer, STORAGE_BUFFER, uint_struct],
    );
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[uint_element_pointer, STORAGE_BUFFER, uint_type],
    );
    instruction(
        &mut words,
        OP_TYPE_RUNTIME_ARRAY,
        &[vector_array, vector_type],
    );
    instruction(&mut words, OP_TYPE_STRUCT, &[vector_struct, vector_array]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[vector_struct_pointer, STORAGE_BUFFER, vector_struct],
    );
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[vector_element_pointer, STORAGE_BUFFER, vector_type],
    );
    instruction(
        &mut words,
        OP_TYPE_VECTOR,
        &[builtin_vector_type, uint_type, 3],
    );
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[builtin_pointer, INPUT, builtin_vector_type],
    );
    for variable in [input_variable, output_variable] {
        instruction(
            &mut words,
            OP_VARIABLE,
            &[vector_struct_pointer, variable, STORAGE_BUFFER],
        );
    }
    instruction(
        &mut words,
        OP_VARIABLE,
        &[uint_struct_pointer, length_variable, STORAGE_BUFFER],
    );
    instruction(
        &mut words,
        OP_VARIABLE,
        &[builtin_pointer, global_variable, INPUT],
    );
    instruction(&mut words, OP_CONSTANT, &[uint_type, zero, 0]);
    instruction(
        &mut words,
        OP_CONSTANT,
        &[float_type, scalar_addend, operand_bits],
    );
    let mut vector_addend_operands = Vec::with_capacity(lanes as usize + 2);
    vector_addend_operands.push(vector_type);
    vector_addend_operands.push(vector_addend);
    vector_addend_operands.extend(std::iter::repeat_n(scalar_addend, lanes as usize));
    instruction(&mut words, OP_CONSTANT_COMPOSITE, &vector_addend_operands);
    instruction(&mut words, OP_TYPE_FUNCTION, &[function_type, void_type]);
    instruction(
        &mut words,
        OP_FUNCTION,
        &[void_type, function_id, FUNCTION_CONTROL_NONE, function_type],
    );
    instruction(&mut words, OP_LABEL, &[entry_label]);
    instruction(
        &mut words,
        OP_LOAD,
        &[builtin_vector_type, global_value, global_variable],
    );
    instruction(
        &mut words,
        OP_COMPOSITE_EXTRACT,
        &[uint_type, builtin_spv, global_value, 0],
    );
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            uint_element_pointer,
            length_address,
            length_variable,
            zero,
            zero,
        ],
    );
    instruction(
        &mut words,
        OP_LOAD,
        &[uint_type, length_spv, length_address],
    );
    instruction(
        &mut words,
        OP_ULT,
        &[bool_type, in_bounds, builtin_spv, length_spv],
    );
    instruction(&mut words, OP_SELECTION_MERGE, &[merge_label, 0]);
    instruction(
        &mut words,
        OP_BRANCH_CONDITIONAL,
        &[in_bounds, body_label, merge_label],
    );
    instruction(&mut words, OP_LABEL, &[body_label]);
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            vector_element_pointer,
            input_address,
            input_variable,
            zero,
            builtin_spv,
        ],
    );
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            vector_element_pointer,
            output_address,
            output_variable,
            zero,
            builtin_spv,
        ],
    );
    instruction(
        &mut words,
        OP_LOAD,
        &[vector_type, input_spv, input_address],
    );
    instruction(
        &mut words,
        f32_spirv_opcode(operation),
        &[vector_type, binary_spv, input_spv, vector_addend],
    );
    instruction(&mut words, OP_STORE, &[output_address, binary_spv]);
    instruction(&mut words, OP_BRANCH, &[merge_label]);
    instruction(&mut words, OP_LABEL, &[merge_label]);
    instruction(&mut words, OP_RETURN, &[]);
    instruction(&mut words, OP_FUNCTION_END, &[]);
    validate_spirv(&words).map_err(|_| {
        SpirvError::UnsupportedKernelShape(
            "global-index dynamic-length vector f32 validation failed",
        )
    })?;
    Ok(words)
}

/// Lowers the strict JIR `f32x4` runtime-length vector-add shape to SPIR-V.
///
/// The body must contain `GlobalInvocationId.x`, a scalar `f32` constant,
/// `VectorSplat(4)`, one length load, bounds check, indexed vector input/output
/// offsets, `VectorBinary(Add)` and a vector store. The bounds check must
/// precede every memory effect.
pub fn emit_storage_global_index_vector_f32_add_dynamic_length_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<Vec<u32>, SpirvError> {
    emit_storage_global_index_vector_f32_binary_dynamic_length_from_jir(
        module,
        function,
        options,
        F32ArithmeticOp::Add,
    )
}

/// Lowers the strict JIR `f32x4` runtime-length vector binary shape to SPIR-V.
///
/// The requested operation must match the JIR `VectorBinary` instruction.
pub fn emit_storage_global_index_vector_f32_binary_dynamic_length_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    operation: F32ArithmeticOp,
) -> Result<Vec<u32>, SpirvError> {
    emit_storage_global_index_vector_f32_binary_dynamic_length_lanes_from_jir(
        module, function, options, operation, 4,
    )
}

/// Lowers the strict runtime-length `f32` vector binary shape for two to four
/// lanes. The instruction order and bounds-before-memory contract are shared
/// with the compatibility x4 wrapper; native execution for x2/x3 remains a
/// separate backend capability gate.
pub fn emit_storage_global_index_vector_f32_binary_dynamic_length_lanes_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    operation: F32ArithmeticOp,
    lanes: u32,
) -> Result<Vec<u32>, SpirvError> {
    if !(2..=4).contains(&lanes) {
        return Err(SpirvError::UnsupportedKernelShape(
            "vector f32 lanes must be in 2..=4",
        ));
    }
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(SpirvError::GpuVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    if function_data.name.is_empty() || function_data.name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) || function_data.parameters.len() != 3
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "vector f32 add requires three resource parameters and Unit result",
        ));
    }
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    if resources.len() != 3
        || resources
            .iter()
            .any(|resource| resource.address_space != AddressSpace::Storage)
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "vector f32 add requires three storage resources",
        ));
    }
    let vector_type = resources[0].element_type;
    let scalar_type = match module.types.get(vector_type.index()) {
        Some(Type::Vector {
            element,
            lanes: vector_lanes,
        }) if u32::from(*vector_lanes) == lanes => *element,
        _ => {
            return Err(SpirvError::UnsupportedKernelShape(
                "vector f32 add input/output element has unsupported lane count",
            ));
        }
    };
    if !matches!(
        module.types.get(scalar_type.index()),
        Some(Type::Float { bits: 32 })
    ) || resources[1].element_type != vector_type
        || !matches!(
            module.types.get(resources[2].element_type.index()),
            Some(Type::Integer {
                signed: false,
                bits: 32
            })
        )
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "vector f32 add resources must be matching f32 vectors and u32",
        ));
    }
    let block = function_data
        .blocks
        .first()
        .ok_or(SpirvError::UnsupportedKernelShape("missing entry block"))?;
    if function_data.blocks.len() != 1
        || !block.parameters.is_empty()
        || !matches!(block.terminator, Terminator::Return { value: None })
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "vector f32 add requires one Unit-returning entry block",
        ));
    }
    let input_parameter = &function_data.parameters[0];
    let output_parameter = &function_data.parameters[1];
    let length_parameter = &function_data.parameters[2];
    let u32_type = resources[2].element_type;
    let mut builtin = None;
    let mut scalar_constant = None;
    let mut splat = None;
    let mut length_load = None;
    let mut input_offset = None;
    let mut output_offset = None;
    let mut input_load = None;
    let mut bounds = None;
    let mut binary = None;
    let mut store = None;

    for (index, instruction) in block.instructions.iter().enumerate() {
        match &instruction.kind {
            InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX) => {
                let result = instruction
                    .result
                    .ok_or(SpirvError::UnsupportedKernelShape(
                        "vector f32 add builtin must produce a value",
                    ))?;
                if result.ty != u32_type || builtin.replace((result, index)).is_some() {
                    return Err(SpirvError::UnsupportedKernelShape(
                        "vector f32 add requires one u32 GlobalInvocationId.x",
                    ));
                }
            }
            InstructionKind::Builtin(_) => {
                return Err(SpirvError::UnsupportedKernelShape(
                    "vector f32 add only supports GlobalInvocationId.x",
                ));
            }
            InstructionKind::Constant(Constant::FloatBits { bits }) => {
                let result = instruction
                    .result
                    .ok_or(SpirvError::UnsupportedKernelShape(
                        "vector f32 add constant must produce a value",
                    ))?;
                if result.ty != scalar_type
                    || scalar_constant.replace((result, *bits, index)).is_some()
                {
                    return Err(SpirvError::UnsupportedKernelShape(
                        "vector f32 add requires one f32 constant",
                    ));
                }
            }
            InstructionKind::Constant(_) => {
                return Err(SpirvError::UnsupportedKernelShape(
                    "vector f32 add only supports a FloatBits constant",
                ));
            }
            InstructionKind::VectorSplat {
                value,
                lanes: vector_lanes,
            } => {
                let result = instruction
                    .result
                    .ok_or(SpirvError::UnsupportedKernelShape(
                        "vector f32 add splat must produce a value",
                    ))?;
                if result.ty != vector_type
                    || u32::from(*vector_lanes) != lanes
                    || splat.replace((result, *value, index)).is_some()
                {
                    return Err(SpirvError::UnsupportedKernelShape(
                        "vector f32 add requires one matching vector splat",
                    ));
                }
            }
            InstructionKind::Load { pointer, .. } if *pointer == length_parameter.value => {
                let result = instruction
                    .result
                    .ok_or(SpirvError::UnsupportedKernelShape(
                        "vector f32 add length load must produce a value",
                    ))?;
                if result.ty != u32_type || length_load.replace((result, index)).is_some() {
                    return Err(SpirvError::UnsupportedKernelShape(
                        "vector f32 add requires one u32 length load",
                    ));
                }
            }
            InstructionKind::Load { pointer, .. } => {
                let result = instruction
                    .result
                    .ok_or(SpirvError::UnsupportedKernelShape(
                        "vector f32 add input load must produce a value",
                    ))?;
                if result.ty != vector_type
                    || input_load.replace((result, *pointer, index)).is_some()
                {
                    return Err(SpirvError::UnsupportedKernelShape(
                        "vector f32 add requires one vector input load",
                    ));
                }
            }
            InstructionKind::Offset { base, indices } => {
                let result = instruction
                    .result
                    .ok_or(SpirvError::UnsupportedKernelShape(
                        "vector f32 add offset must produce a pointer",
                    ))?;
                let Some((builtin_value, _)) = builtin else {
                    return Err(SpirvError::UnsupportedKernelShape(
                        "vector f32 add offset requires GlobalInvocationId.x",
                    ));
                };
                if indices.as_slice() != [builtin_value.value] {
                    return Err(SpirvError::UnsupportedKernelShape(
                        "vector f32 add offset must use GlobalInvocationId.x",
                    ));
                }
                if *base == input_parameter.value {
                    if result.ty != input_parameter.ty
                        || input_offset.replace((result, index)).is_some()
                    {
                        return Err(SpirvError::UnsupportedKernelShape(
                            "vector f32 add input offset is invalid",
                        ));
                    }
                } else if *base == output_parameter.value {
                    if result.ty != output_parameter.ty
                        || output_offset.replace((result, index)).is_some()
                    {
                        return Err(SpirvError::UnsupportedKernelShape(
                            "vector f32 add output offset is invalid",
                        ));
                    }
                } else {
                    return Err(SpirvError::UnsupportedKernelShape(
                        "vector f32 add offset base is invalid",
                    ));
                }
            }
            InstructionKind::BoundsCheck {
                index: checked_index,
                length,
            } => {
                if bounds.replace((*checked_index, *length, index)).is_some() {
                    return Err(SpirvError::UnsupportedKernelShape(
                        "vector f32 add requires one bounds check",
                    ));
                }
            }
            InstructionKind::VectorBinary { op, left, right }
                if *op == operation.as_binary_op() =>
            {
                let result = instruction
                    .result
                    .ok_or(SpirvError::UnsupportedKernelShape(
                        "vector f32 add operation must produce a value",
                    ))?;
                if result.ty != vector_type
                    || binary.replace((result, *left, *right, index)).is_some()
                {
                    return Err(SpirvError::UnsupportedKernelShape(
                        "vector f32 add operation is invalid",
                    ));
                }
            }
            InstructionKind::VectorBinary { .. } => {
                return Err(SpirvError::UnsupportedKernelShape(
                    "vector f32 binary operation does not match requested f32 family",
                ));
            }
            InstructionKind::Store { pointer, value, .. } => {
                if store.replace((*pointer, *value, index)).is_some() {
                    return Err(SpirvError::UnsupportedKernelShape(
                        "vector f32 add requires one store",
                    ));
                }
            }
            _ => {
                return Err(SpirvError::UnsupportedKernelShape(
                    "vector f32 add contains an unsupported instruction",
                ));
            }
        }
    }
    let (builtin_value, builtin_index) = builtin.ok_or(SpirvError::UnsupportedKernelShape(
        "vector f32 add requires GlobalInvocationId.x",
    ))?;
    let (constant_value, addend_bits, constant_index) = scalar_constant.ok_or(
        SpirvError::UnsupportedKernelShape("vector f32 add requires an f32 constant"),
    )?;
    let (splat_value, splat_operand, splat_index) = splat.ok_or(
        SpirvError::UnsupportedKernelShape("vector f32 add requires a vector splat"),
    )?;
    if splat_operand != constant_value.value {
        return Err(SpirvError::UnsupportedKernelShape(
            "vector f32 add splat must use the f32 constant",
        ));
    }
    let (length_value, length_index) = length_load.ok_or(SpirvError::UnsupportedKernelShape(
        "vector f32 add requires a length load",
    ))?;
    let (input_offset_value, input_offset_index) = input_offset.ok_or(
        SpirvError::UnsupportedKernelShape("vector f32 add requires an input offset"),
    )?;
    let (output_offset_value, output_offset_index) = output_offset.ok_or(
        SpirvError::UnsupportedKernelShape("vector f32 add requires an output offset"),
    )?;
    let (bounds_index_value, bounds_length_value, bounds_index) = bounds.ok_or(
        SpirvError::UnsupportedKernelShape("vector f32 add requires a bounds check"),
    )?;
    if bounds_index_value != builtin_value.value || bounds_length_value != length_value.value {
        return Err(SpirvError::UnsupportedKernelShape(
            "vector f32 add bounds check must guard index with length",
        ));
    }
    let (input_result, input_pointer, input_load_index) = input_load.ok_or(
        SpirvError::UnsupportedKernelShape("vector f32 add requires an input load"),
    )?;
    if input_pointer != input_offset_value.value {
        return Err(SpirvError::UnsupportedKernelShape(
            "vector f32 add input load must use input offset",
        ));
    }
    let (binary_result, left, right, binary_index) = binary.ok_or(
        SpirvError::UnsupportedKernelShape("vector f32 add requires VectorBinary(Add)"),
    )?;
    if !((left == input_result.value && right == splat_value.value)
        || (right == input_result.value && left == splat_value.value))
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "vector f32 add operands must be input and splat",
        ));
    }
    let (store_pointer, store_value, store_index) = store.ok_or(
        SpirvError::UnsupportedKernelShape("vector f32 add requires an output store"),
    )?;
    if store_pointer != output_offset_value.value || store_value != binary_result.value {
        return Err(SpirvError::UnsupportedKernelShape(
            "vector f32 add store must write binary result to output offset",
        ));
    }
    if !(builtin_index < bounds_index
        && constant_index < splat_index
        && length_index < bounds_index
        && bounds_index < input_offset_index
        && bounds_index < output_offset_index
        && bounds_index < input_load_index
        && bounds_index < binary_index
        && bounds_index < store_index)
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "vector f32 add bounds check must precede all memory effects",
        ));
    }
    emit_storage_global_index_vector_f32_binary_dynamic_length_lanes(
        &function_data.name,
        options,
        u32::try_from(addend_bits).map_err(|_| {
            SpirvError::UnsupportedKernelShape("vector f32 add constant exceeds f32 bits")
        })?,
        operation,
        lanes,
    )
}

/// Emits a three-resource dynamic-index `f32` kernel:
/// `output[index] = input[index] + addend`.
pub fn emit_storage_dynamic_index_fadd(
    entry_name: &str,
    options: SpirvOptions,
    addend_bits: u32,
) -> Result<Vec<u32>, SpirvError> {
    if entry_name.is_empty() || entry_name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    let function_id = 1;
    let void_type = 2;
    let function_type = 3;
    let label = 4;
    let uint_type = 5;
    let uint_array = 6;
    let uint_struct = 7;
    let uint_struct_pointer = 8;
    let float_type = 9;
    let float_array = 10;
    let float_struct = 11;
    let float_struct_pointer = 12;
    let float_element_pointer = 13;
    let uint_element_pointer = 14;
    let input_variable = 15;
    let output_variable = 16;
    let index_variable = 17;
    let zero = 18;
    let index_address = 19;
    let dynamic_index = 20;
    let constant_addend = 21;
    let input_address = 22;
    let output_address = 23;
    let loaded_input = 24;
    let sum = 25;
    let mut words = vec![SPIRV_MAGIC, SPIRV_VERSION_1_3, 0, 26, 0];
    instruction(&mut words, OP_CAPABILITY, &[CAPABILITY_SHADER]);
    instruction(
        &mut words,
        OP_MEMORY_MODEL,
        &[ADDRESSING_LOGICAL, MEMORY_MODEL_GLSL450],
    );
    for (array_type, struct_type) in [(uint_array, uint_struct), (float_array, float_struct)] {
        instruction(
            &mut words,
            OP_DECORATE,
            &[array_type, DECORATION_ARRAY_STRIDE, 4],
        );
        instruction(
            &mut words,
            OP_MEMBER_DECORATE,
            &[struct_type, 0, DECORATION_OFFSET, 0],
        );
        instruction(&mut words, OP_DECORATE, &[struct_type, DECORATION_BLOCK]);
    }
    for (variable, binding) in [
        (input_variable, 0),
        (output_variable, 1),
        (index_variable, 2),
    ] {
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_DESCRIPTOR_SET, 0],
        );
        instruction(
            &mut words,
            OP_DECORATE,
            &[variable, DECORATION_BINDING, binding],
        );
    }
    instruction_string_with_tail(
        &mut words,
        OP_ENTRY_POINT,
        &[EXECUTION_MODEL_GL_COMPUTE, function_id],
        entry_name,
        &[input_variable, output_variable, index_variable],
    );
    instruction(
        &mut words,
        OP_EXECUTION_MODE,
        &[
            function_id,
            EXECUTION_MODE_LOCAL_SIZE,
            options.workgroup_size[0],
            options.workgroup_size[1],
            options.workgroup_size[2],
        ],
    );
    instruction(&mut words, OP_TYPE_VOID, &[void_type]);
    instruction(&mut words, OP_TYPE_INT, &[uint_type, 32, 0]);
    instruction(&mut words, OP_TYPE_RUNTIME_ARRAY, &[uint_array, uint_type]);
    instruction(&mut words, OP_TYPE_STRUCT, &[uint_struct, uint_array]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[uint_struct_pointer, STORAGE_BUFFER, uint_struct],
    );
    instruction(&mut words, OP_TYPE_FLOAT, &[float_type, 32]);
    instruction(
        &mut words,
        OP_TYPE_RUNTIME_ARRAY,
        &[float_array, float_type],
    );
    instruction(&mut words, OP_TYPE_STRUCT, &[float_struct, float_array]);
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[float_struct_pointer, STORAGE_BUFFER, float_struct],
    );
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[float_element_pointer, STORAGE_BUFFER, float_type],
    );
    instruction(
        &mut words,
        OP_TYPE_POINTER,
        &[uint_element_pointer, STORAGE_BUFFER, uint_type],
    );
    instruction(
        &mut words,
        OP_VARIABLE,
        &[float_struct_pointer, input_variable, STORAGE_BUFFER],
    );
    instruction(
        &mut words,
        OP_VARIABLE,
        &[float_struct_pointer, output_variable, STORAGE_BUFFER],
    );
    instruction(
        &mut words,
        OP_VARIABLE,
        &[uint_struct_pointer, index_variable, STORAGE_BUFFER],
    );
    instruction(&mut words, OP_CONSTANT, &[uint_type, zero, 0]);
    instruction(
        &mut words,
        OP_CONSTANT,
        &[float_type, constant_addend, addend_bits],
    );
    instruction(&mut words, OP_TYPE_FUNCTION, &[function_type, void_type]);
    instruction(
        &mut words,
        OP_FUNCTION,
        &[void_type, function_id, FUNCTION_CONTROL_NONE, function_type],
    );
    instruction(&mut words, OP_LABEL, &[label]);
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            uint_element_pointer,
            index_address,
            index_variable,
            zero,
            zero,
        ],
    );
    instruction(
        &mut words,
        OP_LOAD,
        &[uint_type, dynamic_index, index_address],
    );
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            float_element_pointer,
            input_address,
            input_variable,
            zero,
            dynamic_index,
        ],
    );
    instruction(
        &mut words,
        OP_ACCESS_CHAIN,
        &[
            float_element_pointer,
            output_address,
            output_variable,
            zero,
            dynamic_index,
        ],
    );
    instruction(
        &mut words,
        OP_LOAD,
        &[float_type, loaded_input, input_address],
    );
    instruction(
        &mut words,
        OP_FADD,
        &[float_type, sum, loaded_input, constant_addend],
    );
    instruction(&mut words, OP_STORE, &[output_address, sum]);
    instruction(&mut words, OP_RETURN, &[]);
    instruction(&mut words, OP_FUNCTION_END, &[]);
    validate_spirv(&words).map_err(|_| {
        SpirvError::UnsupportedKernelShape("storage dynamic-index fadd validation failed")
    })?;
    Ok(words)
}

/// Lowers the first supported JIR storage-kernel shape to a real write.
///
/// The supported shape is intentionally narrow but no longer fixture-only:
/// one `ptr<storage, u32>` parameter, one `u32` integer constant, one store to
/// that parameter and `return`. The resource binding is derived through
/// [`reflect_resources`], and malformed/unsupported JIR is rejected before
/// SPIR-V emission.
pub fn emit_storage_write_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<Vec<u32>, SpirvError> {
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(SpirvError::GpuVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    if function_data.name.is_empty() || function_data.name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage write result must be Unit",
        ));
    }
    if function_data.parameters.len() != 1 {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage write requires exactly one resource parameter",
        ));
    }
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    let Some(resource) = resources.first() else {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage write requires one reflected resource",
        ));
    };
    if resources.len() != 1 || resource.address_space != AddressSpace::Storage {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage write requires one storage resource",
        ));
    }
    if !matches!(
        module.types.get(resource.element_type.index()),
        Some(Type::Integer {
            signed: false,
            bits: 32
        })
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage write resource element must be u32",
        ));
    }
    let Some(block) = function_data.blocks.first() else {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage write requires one entry block",
        ));
    };
    if function_data.blocks.len() != 1 || block.instructions.len() != 2 {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage write body must contain const and store",
        ));
    }
    let constant = &block.instructions[0];
    let Some(constant_result) = constant.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage write constant must produce a value",
        ));
    };
    if constant_result.ty != resource.element_type {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage write constant type differs from resource element",
        ));
    }
    let value = match &constant.kind {
        InstructionKind::Constant(Constant::Integer { value }) => {
            u32::try_from(*value).map_err(|_| {
                SpirvError::UnsupportedKernelShape("storage write constant is outside u32")
            })?
        }
        _ => {
            return Err(SpirvError::UnsupportedKernelShape(
                "storage write first instruction must be u32 const",
            ));
        }
    };
    let store = &block.instructions[1];
    if store.result.is_some() {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage write store cannot produce a value",
        ));
    }
    let InstructionKind::Store {
        pointer,
        value: stored_value,
        ..
    } = &store.kind
    else {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage write second instruction must be store",
        ));
    };
    if *pointer != function_data.parameters[0].value || *stored_value != constant_result.value {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage write store must target resource parameter and const",
        ));
    }
    if !matches!(block.terminator, Terminator::Return { value: None }) {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage write must return Unit",
        ));
    }
    emit_storage_write(&function_data.name, options, value)
}

/// Lowers the supported JIR `load -> add -> store` storage-kernel shape.
///
/// The body must contain one `u32` addend constant, a load from the sole
/// storage parameter (or an `Offset` from it with one constant `u32` index),
/// `add(load, constant)` (in either operand order), a store back to that
/// address and a Unit return.
pub fn emit_storage_add_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<Vec<u32>, SpirvError> {
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(SpirvError::GpuVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    if function_data.name.is_empty() || function_data.name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) || function_data.parameters.len() != 1
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage add requires one resource parameter and Unit result",
        ));
    }
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    let Some(resource) = resources.first() else {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage add requires one reflected resource",
        ));
    };
    if resources.len() != 1 || resource.address_space != AddressSpace::Storage {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage add requires one storage resource",
        ));
    }
    if !matches!(
        module.types.get(resource.element_type.index()),
        Some(Type::Integer {
            signed: false,
            bits: 32
        })
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage add resource element must be u32",
        ));
    }
    let Some(block) = function_data.blocks.first() else {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage add requires one entry block",
        ));
    };
    if function_data.blocks.len() != 1 || !matches!(block.instructions.len(), 4 | 6) {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage add body must contain const, optional index/offset, load, add and store",
        ));
    }
    let mut index = 0_u32;
    let mut expected_pointer = function_data.parameters[0].value;
    if block.instructions.len() == 6 {
        let index_instruction = &block.instructions[1];
        let Some(index_result) = index_instruction.result else {
            return Err(SpirvError::UnsupportedKernelShape(
                "storage add offset index must produce a value",
            ));
        };
        if !matches!(
            module.types.get(index_result.ty.index()),
            Some(Type::Integer {
                signed: false,
                bits: 32
            })
        ) {
            return Err(SpirvError::UnsupportedKernelShape(
                "storage add offset index must be u32",
            ));
        }
        if let InstructionKind::Constant(Constant::Integer { value }) = &index_instruction.kind {
            index = u32::try_from(*value).map_err(|_| {
                SpirvError::UnsupportedKernelShape("storage add offset index is outside u32")
            })?;
        } else {
            return Err(SpirvError::UnsupportedKernelShape(
                "storage add offset index must be constant",
            ));
        }
        let offset = &block.instructions[2];
        let Some(offset_result) = offset.result else {
            return Err(SpirvError::UnsupportedKernelShape(
                "storage add offset must produce a pointer",
            ));
        };
        let InstructionKind::Offset { base, indices } = &offset.kind else {
            return Err(SpirvError::UnsupportedKernelShape(
                "storage add third instruction must be offset",
            ));
        };
        if *base != function_data.parameters[0].value
            || indices != &[index_result.value]
            || offset_result.ty != function_data.parameters[0].ty
        {
            return Err(SpirvError::UnsupportedKernelShape(
                "storage add offset must target resource with one index",
            ));
        }
        expected_pointer = offset_result.value;
    }
    let constant = &block.instructions[0];
    let Some(constant_result) = constant.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage add constant must produce a value",
        ));
    };
    if constant_result.ty != resource.element_type {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage add constant type differs from resource element",
        ));
    }
    let addend = match &constant.kind {
        InstructionKind::Constant(Constant::Integer { value }) => {
            u32::try_from(*value).map_err(|_| {
                SpirvError::UnsupportedKernelShape("storage add constant is outside u32")
            })?
        }
        _ => {
            return Err(SpirvError::UnsupportedKernelShape(
                "storage add first instruction must be u32 const",
            ));
        }
    };
    let load = &block.instructions[if block.instructions.len() == 4 { 1 } else { 3 }];
    let Some(load_result) = load.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage add load must produce a value",
        ));
    };
    let InstructionKind::Load { pointer, .. } = &load.kind else {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage add second instruction must be load",
        ));
    };
    if *pointer != expected_pointer || load_result.ty != resource.element_type {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage add load must read the u32 resource parameter",
        ));
    }
    let binary = &block.instructions[if block.instructions.len() == 4 { 2 } else { 4 }];
    let Some(binary_result) = binary.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage add operation must produce a value",
        ));
    };
    let InstructionKind::Binary { op, left, right } = &binary.kind else {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage add third instruction must be add",
        ));
    };
    if *op != BinaryOp::Add
        || binary_result.ty != resource.element_type
        || !((*left == load_result.value && *right == constant_result.value)
            || (*right == load_result.value && *left == constant_result.value))
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage add operands must be load and constant",
        ));
    }
    let store = &block.instructions[if block.instructions.len() == 4 { 3 } else { 5 }];
    if store.result.is_some() {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage add store cannot produce a value",
        ));
    }
    let InstructionKind::Store {
        pointer,
        value: stored_value,
        ..
    } = &store.kind
    else {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage add fourth instruction must be store",
        ));
    };
    if *pointer != expected_pointer || *stored_value != binary_result.value {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage add store must target resource parameter and sum",
        ));
    }
    if !matches!(block.terminator, Terminator::Return { value: None }) {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage add must return Unit",
        ));
    }
    emit_storage_add_at(&function_data.name, options, index, addend)
}

/// Exports the verified JIR storage-add shape as a backend-neutral artifact.
///
/// This is the hand-off used by native backends: JIR verification, GPU shape
/// checks, SPIR-V structural validation and resource reflection all happen
/// before a caller receives words that may be sent to SPIRV-Cross, Vulkan or a
/// different device API.
pub fn emit_storage_add_artifact_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<SpirvArtifact, SpirvError> {
    let words = emit_storage_add_from_jir(module, function, options)?;
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    let artifact = SpirvArtifact {
        entry_name: function_data.name.clone(),
        workgroup_size: options.workgroup_size,
        resources,
        words,
    };
    artifact
        .validate()
        .map_err(|_| SpirvError::UnsupportedKernelShape("exported SPIR-V validation failed"))?;
    Ok(artifact)
}

/// Exports the verified one-resource `GlobalInvocationId.x` storage-write
/// shape as a backend-neutral artifact.
///
/// The artifact preserves the reflected single binding together with the
/// compile-time value/length and structured `OpULessThan` bounds branch, so a
/// native adapter can reject a plain one-UAV shader that lacks the JIR safety
/// contract.
pub fn emit_storage_global_index_write_artifact_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<SpirvArtifact, SpirvError> {
    let words = emit_storage_global_index_write_from_jir(module, function, options)?;
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    let artifact = SpirvArtifact {
        entry_name: function_data.name.clone(),
        workgroup_size: options.workgroup_size,
        resources,
        words,
    };
    artifact
        .validate()
        .map_err(|_| SpirvError::UnsupportedKernelShape("exported SPIR-V validation failed"))?;
    Ok(artifact)
}

/// Exports the verified runtime-stride global-index storage-write shape as a
/// backend-neutral artifact.
///
/// The artifact preserves the four reflected storage bindings, separate
/// logical and physical bounds checks, and the `GlobalInvocationId.x * stride`
/// address calculation. Native backends must validate this contract before
/// translating or dispatching the artifact.
pub fn emit_storage_global_index_strided_write_artifact_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<SpirvArtifact, SpirvError> {
    let words = emit_storage_global_index_strided_write_from_jir(module, function, options)?;
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    let artifact = SpirvArtifact {
        entry_name: function_data.name.clone(),
        workgroup_size: options.workgroup_size,
        resources,
        words,
    };
    artifact
        .validate()
        .map_err(|_| SpirvError::UnsupportedKernelShape("exported SPIR-V validation failed"))?;
    Ok(artifact)
}

/// Exports the verified two-dimensional row-major storage-write shape as a
/// backend-neutral artifact.
///
/// The artifact retains the four reflected storage bindings together with the
/// `GlobalInvocationId.x/y` bounds and capacity checks. Native adapters can
/// therefore distinguish the checked JIR shape from an arbitrary four-UAV
/// compute shader before creating a pipeline.
pub fn emit_storage_global_2d_write_artifact_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<SpirvArtifact, SpirvError> {
    let words = emit_storage_global_2d_write_from_jir(module, function, options)?;
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    let artifact = SpirvArtifact {
        entry_name: function_data.name.clone(),
        workgroup_size: options.workgroup_size,
        resources,
        words,
    };
    artifact
        .validate()
        .map_err(|_| SpirvError::UnsupportedKernelShape("exported SPIR-V validation failed"))?;
    Ok(artifact)
}

/// Exports the verified two-dimensional affine-stride storage-write shape as
/// a backend-neutral artifact.
///
/// The artifact retains six reflected storage bindings and the independent
/// coordinate and physical-capacity bounds checks required before the affine
/// `x * stride_x + y * stride_y` store.
pub fn emit_storage_global_2d_strided_write_artifact_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<SpirvArtifact, SpirvError> {
    let words = emit_storage_global_2d_strided_write_from_jir(module, function, options)?;
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    let artifact = SpirvArtifact {
        entry_name: function_data.name.clone(),
        workgroup_size: options.workgroup_size,
        resources,
        words,
    };
    artifact
        .validate()
        .map_err(|_| SpirvError::UnsupportedKernelShape("exported SPIR-V validation failed"))?;
    Ok(artifact)
}

/// Exports the verified three-dimensional row-major storage-write shape as a
/// backend-neutral artifact.
///
/// The artifact retains five reflected storage bindings and the independent
/// X/Y/Z coordinate and physical-capacity bounds checks required before the
/// `((z * height) + y) * width + x` store.
pub fn emit_storage_global_3d_write_artifact_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<SpirvArtifact, SpirvError> {
    let words = emit_storage_global_3d_write_from_jir(module, function, options)?;
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    let artifact = SpirvArtifact {
        entry_name: function_data.name.clone(),
        workgroup_size: options.workgroup_size,
        resources,
        words,
    };
    artifact
        .validate()
        .map_err(|_| SpirvError::UnsupportedKernelShape("exported SPIR-V validation failed"))?;
    Ok(artifact)
}

/// Exports the verified three-dimensional affine-stride storage-write shape
/// as a backend-neutral artifact.
///
/// The artifact retains eight reflected storage bindings and the independent
/// X/Y/Z coordinate and physical-capacity bounds checks required before the
/// `x * stride_x + y * stride_y + z * stride_z` store.
pub fn emit_storage_global_3d_strided_write_artifact_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<SpirvArtifact, SpirvError> {
    let words = emit_storage_global_3d_strided_write_from_jir(module, function, options)?;
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    let artifact = SpirvArtifact {
        entry_name: function_data.name.clone(),
        workgroup_size: options.workgroup_size,
        resources,
        words,
    };
    artifact
        .validate()
        .map_err(|_| SpirvError::UnsupportedKernelShape("exported SPIR-V validation failed"))?;
    Ok(artifact)
}

/// Exports a verified dynamic-length global-index storage arithmetic shape as
/// the same backend-neutral artifact contract.
fn emit_storage_global_index_arithmetic_dynamic_length_artifact_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    operation: IntegerArithmeticOp,
) -> Result<SpirvArtifact, SpirvError> {
    let words = emit_storage_global_index_arithmetic_dynamic_length_generic_from_jir(
        module, function, options, operation,
    )?;
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    let artifact = SpirvArtifact {
        entry_name: function_data.name.clone(),
        workgroup_size: options.workgroup_size,
        resources,
        words,
    };
    artifact
        .validate()
        .map_err(|_| SpirvError::UnsupportedKernelShape("exported SPIR-V validation failed"))?;
    Ok(artifact)
}

/// Exports a verified dynamic-length global-index storage binary shape as the
/// same backend-neutral artifact contract.
pub fn emit_storage_global_index_binary_dynamic_length_artifact_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    operation: BinaryOp,
) -> Result<SpirvArtifact, SpirvError> {
    let operation = IntegerArithmeticOp::from_jir(operation).ok_or(
        SpirvError::UnsupportedKernelShape("dynamic-length binary operation is unsupported"),
    )?;
    emit_storage_global_index_arithmetic_dynamic_length_artifact_from_jir(
        module, function, options, operation,
    )
}

/// Exports the verified dynamic-length global-index storage-add shape as the
/// same backend-neutral artifact contract.
pub fn emit_storage_global_index_add_dynamic_length_artifact_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<SpirvArtifact, SpirvError> {
    emit_storage_global_index_binary_dynamic_length_artifact_from_jir(
        module,
        function,
        options,
        BinaryOp::Add,
    )
}

/// Exports the verified dynamic-length global-index storage-multiply shape as
/// the same backend-neutral artifact contract.
pub fn emit_storage_global_index_multiply_dynamic_length_artifact_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<SpirvArtifact, SpirvError> {
    emit_storage_global_index_binary_dynamic_length_artifact_from_jir(
        module,
        function,
        options,
        BinaryOp::Multiply,
    )
}

/// Exports a verified runtime-length global-index scalar `f32` binary shape as
/// the backend-neutral artifact contract used by native adapters.
pub fn emit_storage_global_index_f32_binary_dynamic_length_artifact_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    operation: F32ArithmeticOp,
) -> Result<SpirvArtifact, SpirvError> {
    let words = emit_storage_global_index_f32_binary_dynamic_length_from_jir(
        module, function, options, operation,
    )?;
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    let artifact = SpirvArtifact {
        entry_name: function_data.name.clone(),
        workgroup_size: options.workgroup_size,
        resources,
        words,
    };
    artifact
        .validate()
        .map_err(|_| SpirvError::UnsupportedKernelShape("exported SPIR-V validation failed"))?;
    Ok(artifact)
}

/// Exports the verified runtime-length global-index `f32` add shape.
pub fn emit_storage_global_index_fadd_dynamic_length_artifact_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<SpirvArtifact, SpirvError> {
    emit_storage_global_index_f32_binary_dynamic_length_artifact_from_jir(
        module,
        function,
        options,
        F32ArithmeticOp::Add,
    )
}

/// Exports the verified runtime-length global-index `f32` subtract shape.
pub fn emit_storage_global_index_fsub_dynamic_length_artifact_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<SpirvArtifact, SpirvError> {
    emit_storage_global_index_f32_binary_dynamic_length_artifact_from_jir(
        module,
        function,
        options,
        F32ArithmeticOp::Subtract,
    )
}

/// Exports the verified runtime-length global-index `f32` multiply shape.
pub fn emit_storage_global_index_fmul_dynamic_length_artifact_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<SpirvArtifact, SpirvError> {
    emit_storage_global_index_f32_binary_dynamic_length_artifact_from_jir(
        module,
        function,
        options,
        F32ArithmeticOp::Multiply,
    )
}

/// Exports the verified runtime-length `f32x4` vector-add shape as an artifact.
///
/// The reflected input/output bindings carry a 16-byte element stride. Native
/// adapters may consume this metadata, but no DX12/Metal execution claim is
/// made by this source-contract milestone.
pub fn emit_storage_global_index_vector_f32_add_dynamic_length_artifact_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<SpirvArtifact, SpirvError> {
    emit_storage_global_index_vector_f32_binary_dynamic_length_artifact_from_jir(
        module,
        function,
        options,
        F32ArithmeticOp::Add,
    )
}

/// Exports the verified runtime-length `f32x4` vector binary shape as an
/// artifact while preserving the reflected 16-byte vector stride.
pub fn emit_storage_global_index_vector_f32_binary_dynamic_length_artifact_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    operation: F32ArithmeticOp,
) -> Result<SpirvArtifact, SpirvError> {
    emit_storage_global_index_vector_f32_binary_dynamic_length_lanes_artifact_from_jir(
        module, function, options, operation, 4,
    )
}

/// Exports the verified runtime-length vector binary shape for two to four
/// `f32` lanes while preserving the reflected vector stride.
pub fn emit_storage_global_index_vector_f32_binary_dynamic_length_lanes_artifact_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
    operation: F32ArithmeticOp,
    lanes: u32,
) -> Result<SpirvArtifact, SpirvError> {
    let words = emit_storage_global_index_vector_f32_binary_dynamic_length_lanes_from_jir(
        module, function, options, operation, lanes,
    )?;
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    let artifact = SpirvArtifact {
        entry_name: function_data.name.clone(),
        workgroup_size: options.workgroup_size,
        resources,
        words,
    };
    artifact
        .validate()
        .map_err(|_| SpirvError::UnsupportedKernelShape("exported SPIR-V validation failed"))?;
    Ok(artifact)
}

/// Lowers a JIR kernel with two storage resources:
/// `output[0] = input[0] + constant`.
pub fn emit_storage_dual_add_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<Vec<u32>, SpirvError> {
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(SpirvError::GpuVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    if function_data.name.is_empty() || function_data.name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) || function_data.parameters.len() != 2
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage dual-add requires two resource parameters and Unit result",
        ));
    }
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    if resources.len() != 2
        || resources
            .iter()
            .any(|resource| resource.address_space != AddressSpace::Storage)
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage dual-add requires two storage resources",
        ));
    }
    if resources.iter().any(|resource| {
        !matches!(
            module.types.get(resource.element_type.index()),
            Some(Type::Integer {
                signed: false,
                bits: 32
            })
        )
    }) {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage dual-add resource elements must be u32",
        ));
    }
    let Some(block) = function_data.blocks.first() else {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage dual-add requires one entry block",
        ));
    };
    if function_data.blocks.len() != 1 || block.instructions.len() != 4 {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage dual-add body must contain const, load, add and store",
        ));
    }
    let constant = &block.instructions[0];
    let Some(constant_result) = constant.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage dual-add constant must produce a value",
        ));
    };
    if !matches!(
        &constant.kind,
        InstructionKind::Constant(Constant::Integer { .. })
    ) || constant_result.ty != resources[0].element_type
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage dual-add first instruction must be u32 const",
        ));
    }
    let addend = match &constant.kind {
        InstructionKind::Constant(Constant::Integer { value }) => {
            u32::try_from(*value).map_err(|_| {
                SpirvError::UnsupportedKernelShape("storage dual-add const outside u32")
            })?
        }
        _ => unreachable!("constant kind checked above"),
    };
    let load = &block.instructions[1];
    let Some(load_result) = load.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage dual-add load must produce a value",
        ));
    };
    let InstructionKind::Load { pointer, .. } = &load.kind else {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage dual-add second instruction must be load",
        ));
    };
    if *pointer != function_data.parameters[0].value || load_result.ty != resources[0].element_type
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage dual-add load must read input resource",
        ));
    }
    let binary = &block.instructions[2];
    let Some(binary_result) = binary.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage dual-add operation must produce a value",
        ));
    };
    let InstructionKind::Binary { op, left, right } = &binary.kind else {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage dual-add third instruction must be add",
        ));
    };
    if *op != BinaryOp::Add
        || binary_result.ty != resources[1].element_type
        || !((*left == load_result.value && *right == constant_result.value)
            || (*right == load_result.value && *left == constant_result.value))
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage dual-add operands must be input load and constant",
        ));
    }
    let store = &block.instructions[3];
    if store.result.is_some() {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage dual-add store cannot produce a value",
        ));
    }
    let InstructionKind::Store {
        pointer,
        value: stored_value,
        ..
    } = &store.kind
    else {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage dual-add fourth instruction must be store",
        ));
    };
    if *pointer != function_data.parameters[1].value || *stored_value != binary_result.value {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage dual-add store must target output resource and sum",
        ));
    }
    if !matches!(block.terminator, Terminator::Return { value: None }) {
        return Err(SpirvError::UnsupportedKernelShape(
            "storage dual-add must return Unit",
        ));
    }
    emit_storage_dual_add(&function_data.name, options, addend)
}

/// Lowers a JIR kernel whose third storage resource supplies a dynamic index:
/// `output[index_resource[0]] = input[index_resource[0]] + constant`.
pub fn emit_storage_dynamic_index_add_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<Vec<u32>, SpirvError> {
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(SpirvError::GpuVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    if function_data.name.is_empty() || function_data.name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) || function_data.parameters.len() != 3
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index add requires three resource parameters and Unit result",
        ));
    }
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    if resources.len() != 3
        || resources
            .iter()
            .any(|resource| resource.address_space != AddressSpace::Storage)
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index add requires three storage resources",
        ));
    }
    if resources.iter().any(|resource| {
        !matches!(
            module.types.get(resource.element_type.index()),
            Some(Type::Integer {
                signed: false,
                bits: 32
            })
        )
    }) {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index add resource elements must be u32",
        ));
    }
    let Some(block) = function_data.blocks.first() else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index add requires one entry block",
        ));
    };
    if function_data.blocks.len() != 1 || block.instructions.len() != 7 {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index add body must contain const, index load, offsets, load, add and store",
        ));
    }
    let constant = &block.instructions[0];
    let Some(constant_result) = constant.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index add constant must produce a value",
        ));
    };
    let addend = match &constant.kind {
        InstructionKind::Constant(Constant::Integer { value })
            if constant_result.ty == resources[0].element_type =>
        {
            u32::try_from(*value).map_err(|_| {
                SpirvError::UnsupportedKernelShape("dynamic-index add const outside u32")
            })?
        }
        _ => {
            return Err(SpirvError::UnsupportedKernelShape(
                "dynamic-index add first instruction must be u32 const",
            ));
        }
    };
    let index_load = &block.instructions[1];
    let Some(index_result) = index_load.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index load must produce a value",
        ));
    };
    let InstructionKind::Load {
        pointer: index_pointer,
        ..
    } = &index_load.kind
    else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index second instruction must load index resource",
        ));
    };
    if *index_pointer != function_data.parameters[2].value
        || index_result.ty != resources[2].element_type
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index load must read third resource",
        ));
    }
    let input_offset = &block.instructions[2];
    let Some(input_offset_result) = input_offset.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index input offset must produce a pointer",
        ));
    };
    let InstructionKind::Offset {
        base: input_base,
        indices: input_indices,
    } = &input_offset.kind
    else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index third instruction must offset input",
        ));
    };
    if *input_base != function_data.parameters[0].value
        || input_indices != &[index_result.value]
        || input_offset_result.ty != function_data.parameters[0].ty
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index input offset is invalid",
        ));
    }
    let input_load = &block.instructions[3];
    let Some(input_result) = input_load.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index input load must produce a value",
        ));
    };
    let InstructionKind::Load {
        pointer: input_pointer,
        ..
    } = &input_load.kind
    else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fourth instruction must load input",
        ));
    };
    if *input_pointer != input_offset_result.value || input_result.ty != resources[0].element_type {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index input load is invalid",
        ));
    }
    let binary = &block.instructions[4];
    let Some(binary_result) = binary.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index add must produce a value",
        ));
    };
    let InstructionKind::Binary { op, left, right } = &binary.kind else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fifth instruction must be add",
        ));
    };
    if *op != BinaryOp::Add
        || binary_result.ty != resources[1].element_type
        || !((*left == input_result.value && *right == constant_result.value)
            || (*right == input_result.value && *left == constant_result.value))
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index add operands are invalid",
        ));
    }
    let output_offset = &block.instructions[5];
    let Some(output_offset_result) = output_offset.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index output offset must produce a pointer",
        ));
    };
    let InstructionKind::Offset {
        base: output_base,
        indices: output_indices,
    } = &output_offset.kind
    else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index sixth instruction must offset output",
        ));
    };
    if *output_base != function_data.parameters[1].value
        || output_indices != &[index_result.value]
        || output_offset_result.ty != function_data.parameters[1].ty
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index output offset is invalid",
        ));
    }
    let store = &block.instructions[6];
    if store.result.is_some() {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index store cannot produce a value",
        ));
    }
    let InstructionKind::Store {
        pointer,
        value: stored_value,
        ..
    } = &store.kind
    else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index seventh instruction must store output",
        ));
    };
    if *pointer != output_offset_result.value || *stored_value != binary_result.value {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index store is invalid",
        ));
    }
    if !matches!(block.terminator, Terminator::Return { value: None }) {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index add must return Unit",
        ));
    }
    emit_storage_dynamic_index_add(&function_data.name, options, addend)
}

/// Lowers the dynamic-index storage kernel for `f32` input/output resources.
pub fn emit_storage_dynamic_index_fadd_from_jir(
    module: &Module,
    function: FunctionId,
    options: SpirvOptions,
) -> Result<Vec<u32>, SpirvError> {
    let verification_errors = verify_gpu(module);
    if !verification_errors.is_empty() {
        return Err(SpirvError::GpuVerificationFailed(verification_errors.len()));
    }
    let function_data = module
        .functions
        .get(function.index())
        .ok_or(SpirvError::MissingFunction(function))?;
    if function_data.name.is_empty() || function_data.name.as_bytes().contains(&0) {
        return Err(SpirvError::InvalidEntryName);
    }
    if !matches!(
        module.types.get(function_data.result.index()),
        Some(Type::Unit)
    ) || function_data.parameters.len() != 3
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fadd requires three resource parameters and Unit result",
        ));
    }
    let resources = reflect_resources(module, function)
        .map_err(|_| SpirvError::UnsupportedKernelShape("storage resource reflection failed"))?;
    if resources.len() != 3
        || resources
            .iter()
            .any(|resource| resource.address_space != AddressSpace::Storage)
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fadd requires three storage resources",
        ));
    }
    if !matches!(
        module.types.get(resources[0].element_type.index()),
        Some(Type::Float { bits: 32 })
    ) || !matches!(
        module.types.get(resources[1].element_type.index()),
        Some(Type::Float { bits: 32 })
    ) || !matches!(
        module.types.get(resources[2].element_type.index()),
        Some(Type::Integer {
            signed: false,
            bits: 32
        })
    ) {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fadd resource elements must be f32, f32 and u32",
        ));
    }
    let Some(block) = function_data.blocks.first() else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fadd requires one entry block",
        ));
    };
    if function_data.blocks.len() != 1 || block.instructions.len() != 7 {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fadd body must contain const, index load, offsets, load, add and store",
        ));
    }
    let constant = &block.instructions[0];
    let Some(constant_result) = constant.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fadd constant must produce a value",
        ));
    };
    let addend_bits = match &constant.kind {
        InstructionKind::Constant(Constant::FloatBits { bits })
            if constant_result.ty == resources[0].element_type =>
        {
            u32::try_from(*bits).map_err(|_| {
                SpirvError::UnsupportedKernelShape("dynamic-index fadd bits exceed f32")
            })?
        }
        _ => {
            return Err(SpirvError::UnsupportedKernelShape(
                "dynamic-index fadd first instruction must be f32 const",
            ));
        }
    };
    let index_load = &block.instructions[1];
    let Some(index_result) = index_load.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fadd index load must produce a value",
        ));
    };
    let InstructionKind::Load {
        pointer: index_pointer,
        ..
    } = &index_load.kind
    else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fadd second instruction must load index resource",
        ));
    };
    if *index_pointer != function_data.parameters[2].value
        || index_result.ty != resources[2].element_type
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fadd index load must read third resource",
        ));
    }
    let input_offset = &block.instructions[2];
    let Some(input_offset_result) = input_offset.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fadd input offset must produce a pointer",
        ));
    };
    let InstructionKind::Offset {
        base: input_base,
        indices: input_indices,
    } = &input_offset.kind
    else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fadd third instruction must offset input",
        ));
    };
    if *input_base != function_data.parameters[0].value
        || input_indices != &[index_result.value]
        || input_offset_result.ty != function_data.parameters[0].ty
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fadd input offset is invalid",
        ));
    }
    let input_load = &block.instructions[3];
    let Some(input_result) = input_load.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fadd input load must produce a value",
        ));
    };
    let InstructionKind::Load {
        pointer: input_pointer,
        ..
    } = &input_load.kind
    else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fadd fourth instruction must load input",
        ));
    };
    if *input_pointer != input_offset_result.value || input_result.ty != resources[0].element_type {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fadd input load is invalid",
        ));
    }
    let binary = &block.instructions[4];
    let Some(binary_result) = binary.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fadd must produce a value",
        ));
    };
    let InstructionKind::Binary { op, left, right } = &binary.kind else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fadd fifth instruction must be add",
        ));
    };
    if *op != BinaryOp::Add
        || binary_result.ty != resources[1].element_type
        || !((*left == input_result.value && *right == constant_result.value)
            || (*right == input_result.value && *left == constant_result.value))
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fadd operands are invalid",
        ));
    }
    let output_offset = &block.instructions[5];
    let Some(output_offset_result) = output_offset.result else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fadd output offset must produce a pointer",
        ));
    };
    let InstructionKind::Offset {
        base: output_base,
        indices: output_indices,
    } = &output_offset.kind
    else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fadd sixth instruction must offset output",
        ));
    };
    if *output_base != function_data.parameters[1].value
        || output_indices != &[index_result.value]
        || output_offset_result.ty != function_data.parameters[1].ty
    {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fadd output offset is invalid",
        ));
    }
    let store = &block.instructions[6];
    if store.result.is_some() {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fadd store cannot produce a value",
        ));
    }
    let InstructionKind::Store {
        pointer,
        value: stored_value,
        ..
    } = &store.kind
    else {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fadd seventh instruction must store output",
        ));
    };
    if *pointer != output_offset_result.value || *stored_value != binary_result.value {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fadd store is invalid",
        ));
    }
    if !matches!(block.terminator, Terminator::Return { value: None }) {
        return Err(SpirvError::UnsupportedKernelShape(
            "dynamic-index fadd must return Unit",
        ));
    }
    emit_storage_dynamic_index_fadd(&function_data.name, options, addend_bits)
}

fn instruction(words: &mut Vec<u32>, opcode: u16, operands: &[u32]) {
    let word_count = u32::try_from(operands.len() + 1).expect("SPIR-V instruction is bounded");
    words.push((word_count << 16) | u32::from(opcode));
    words.extend_from_slice(operands);
}

fn instruction_string(words: &mut Vec<u32>, opcode: u16, operands: &[u32], value: &str) {
    instruction_string_with_tail(words, opcode, operands, value, &[]);
}

fn instruction_string_with_tail(
    words: &mut Vec<u32>,
    opcode: u16,
    operands: &[u32],
    value: &str,
    tail: &[u32],
) {
    let mut encoded = value.as_bytes().to_vec();
    encoded.push(0);
    while !encoded.len().is_multiple_of(4) {
        encoded.push(0);
    }
    let string_words = encoded
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    let word_count = u32::try_from(operands.len() + 1 + encoded.len() / 4 + tail.len())
        .expect("SPIR-V string instruction is bounded");
    words.push((word_count << 16) | u32::from(opcode));
    words.extend_from_slice(operands);
    words.extend(string_words);
    words.extend_from_slice(tail);
}

#[cfg(test)]
mod tests {
    use super::*;
    use jadren_jir::{
        Block, BlockId, BuiltinOp, Function, Instruction, Linkage, Parameter, TypedValue, ValueId,
    };

    fn empty_kernel(name: &str) -> Module {
        Module {
            types: vec![Type::Unit],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: name.to_owned(),
                linkage: Linkage::Export,
                parameters: Vec::new(),
                result: jadren_jir::TypeId::new(0),
                blocks: vec![Block {
                    id: jadren_jir::BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: Vec::new(),
                    terminator: Terminator::Return { value: None },
                    span: None,
                }],
                span: None,
            }],
        }
    }

    fn jir_storage_add_module() -> Module {
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
                name: "add_u32".to_owned(),
                linkage: Linkage::Export,
                parameters: vec![Parameter {
                    value: ValueId::new(0),
                    ty: TypeId::new(2),
                    name: Some("data".to_owned()),
                }],
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        Instruction {
                            result: Some(TypedValue {
                                value: ValueId::new(1),
                                ty: TypeId::new(1),
                            }),
                            kind: InstructionKind::Constant(Constant::Integer { value: 1 }),
                            span: None,
                        },
                        Instruction {
                            result: Some(TypedValue {
                                value: ValueId::new(2),
                                ty: TypeId::new(1),
                            }),
                            kind: InstructionKind::Load {
                                pointer: ValueId::new(0),
                                alignment: 4,
                                volatile: false,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(TypedValue {
                                value: ValueId::new(3),
                                ty: TypeId::new(1),
                            }),
                            kind: InstructionKind::Binary {
                                op: BinaryOp::Add,
                                left: ValueId::new(2),
                                right: ValueId::new(1),
                            },
                            span: None,
                        },
                        Instruction {
                            result: None,
                            kind: InstructionKind::Store {
                                pointer: ValueId::new(0),
                                value: ValueId::new(3),
                                alignment: 4,
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

    fn jir_storage_dual_add_module() -> Module {
        let mut module = jir_storage_add_module();
        module.functions[0].parameters.push(Parameter {
            value: ValueId::new(1),
            ty: TypeId::new(2),
            name: Some("output".to_owned()),
        });
        module.functions[0].parameters[0].name = Some("input".to_owned());
        let instructions = &mut module.functions[0].blocks[0].instructions;
        instructions[0].result = Some(TypedValue {
            value: ValueId::new(2),
            ty: TypeId::new(1),
        });
        instructions[1].result = Some(TypedValue {
            value: ValueId::new(3),
            ty: TypeId::new(1),
        });
        instructions[1].kind = InstructionKind::Load {
            pointer: ValueId::new(0),
            alignment: 4,
            volatile: false,
        };
        instructions[2].result = Some(TypedValue {
            value: ValueId::new(4),
            ty: TypeId::new(1),
        });
        instructions[2].kind = InstructionKind::Binary {
            op: BinaryOp::Add,
            left: ValueId::new(3),
            right: ValueId::new(2),
        };
        instructions[3].kind = InstructionKind::Store {
            pointer: ValueId::new(1),
            value: ValueId::new(4),
            alignment: 4,
            volatile: false,
        };
        module
    }

    fn jir_storage_dynamic_index_add_module() -> Module {
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
                name: "dynamic_add_u32".to_owned(),
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
                        name: Some("index".to_owned()),
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
                            kind: InstructionKind::Constant(Constant::Integer { value: 1 }),
                            span: None,
                        },
                        Instruction {
                            result: Some(TypedValue {
                                value: ValueId::new(4),
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
                            result: Some(TypedValue {
                                value: ValueId::new(5),
                                ty: TypeId::new(2),
                            }),
                            kind: InstructionKind::Offset {
                                base: ValueId::new(0),
                                indices: vec![ValueId::new(4)],
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(TypedValue {
                                value: ValueId::new(6),
                                ty: TypeId::new(1),
                            }),
                            kind: InstructionKind::Load {
                                pointer: ValueId::new(5),
                                alignment: 4,
                                volatile: false,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(TypedValue {
                                value: ValueId::new(7),
                                ty: TypeId::new(1),
                            }),
                            kind: InstructionKind::Binary {
                                op: BinaryOp::Add,
                                left: ValueId::new(6),
                                right: ValueId::new(3),
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(TypedValue {
                                value: ValueId::new(8),
                                ty: TypeId::new(2),
                            }),
                            kind: InstructionKind::Offset {
                                base: ValueId::new(1),
                                indices: vec![ValueId::new(4)],
                            },
                            span: None,
                        },
                        Instruction {
                            result: None,
                            kind: InstructionKind::Store {
                                pointer: ValueId::new(8),
                                value: ValueId::new(7),
                                alignment: 4,
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

    fn jir_storage_global_index_add_module() -> Module {
        let pointer = Type::Pointer {
            pointee: TypeId::new(1),
            address_space: AddressSpace::Storage,
        };
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
                pointer.clone(),
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "global_add_u32".to_owned(),
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
                ],
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(2),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(3),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Constant(Constant::Integer { value: 1 }),
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(4),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Constant(Constant::Integer { value: 64 }),
                        ),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(2),
                                length: ValueId::new(4),
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(5),
                                ty: TypeId::new(2),
                            }),
                            InstructionKind::Offset {
                                base: ValueId::new(0),
                                indices: vec![ValueId::new(2)],
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(6),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Load {
                                pointer: ValueId::new(5),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(7),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Binary {
                                op: BinaryOp::Add,
                                left: ValueId::new(6),
                                right: ValueId::new(3),
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(8),
                                ty: TypeId::new(2),
                            }),
                            InstructionKind::Offset {
                                base: ValueId::new(1),
                                indices: vec![ValueId::new(2)],
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::Store {
                                pointer: ValueId::new(8),
                                value: ValueId::new(7),
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

    fn jir_storage_global_index_write_module() -> Module {
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
                            InstructionKind::Constant(Constant::Integer { value: 64 }),
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

    fn jir_storage_global_index_strided_write_module() -> Module {
        let instruction = |result, kind| Instruction {
            result,
            kind,
            span: None,
        };
        let pointer = Type::Pointer {
            pointee: TypeId::new(1),
            address_space: AddressSpace::Storage,
        };
        Module {
            types: vec![
                Type::Unit,
                Type::Integer {
                    signed: false,
                    bits: 32,
                },
                pointer,
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "global_strided_write_u32".to_owned(),
                linkage: Linkage::Export,
                parameters: vec![
                    Parameter {
                        value: ValueId::new(0),
                        ty: TypeId::new(2),
                        name: Some("buffer".to_owned()),
                    },
                    Parameter {
                        value: ValueId::new(1),
                        ty: TypeId::new(2),
                        name: Some("length".to_owned()),
                    },
                    Parameter {
                        value: ValueId::new(2),
                        ty: TypeId::new(2),
                        name: Some("stride".to_owned()),
                    },
                    Parameter {
                        value: ValueId::new(3),
                        ty: TypeId::new(2),
                        name: Some("capacity".to_owned()),
                    },
                ],
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(4),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(5),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Constant(Constant::Integer { value: 42 }),
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(6),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Load {
                                pointer: ValueId::new(1),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(7),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Load {
                                pointer: ValueId::new(2),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(8),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Load {
                                pointer: ValueId::new(3),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(4),
                                length: ValueId::new(6),
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(9),
                                ty: TypeId::new(1),
                            }),
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
                            Some(TypedValue {
                                value: ValueId::new(10),
                                ty: TypeId::new(2),
                            }),
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

    fn jir_storage_global_2d_write_module() -> Module {
        let instruction = |result, kind| Instruction {
            result,
            kind,
            span: None,
        };
        let pointer = Type::Pointer {
            pointee: TypeId::new(1),
            address_space: AddressSpace::Storage,
        };
        Module {
            types: vec![
                Type::Unit,
                Type::Integer {
                    signed: false,
                    bits: 32,
                },
                pointer,
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "global_2d_write_u32".to_owned(),
                linkage: Linkage::Export,
                parameters: vec![
                    Parameter {
                        value: ValueId::new(0),
                        ty: TypeId::new(2),
                        name: Some("buffer".to_owned()),
                    },
                    Parameter {
                        value: ValueId::new(1),
                        ty: TypeId::new(2),
                        name: Some("width".to_owned()),
                    },
                    Parameter {
                        value: ValueId::new(2),
                        ty: TypeId::new(2),
                        name: Some("height".to_owned()),
                    },
                    Parameter {
                        value: ValueId::new(3),
                        ty: TypeId::new(2),
                        name: Some("capacity".to_owned()),
                    },
                ],
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(4),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(5),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdY),
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(6),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Constant(Constant::Integer { value: 42 }),
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(7),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Load {
                                pointer: ValueId::new(1),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(8),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Load {
                                pointer: ValueId::new(2),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(9),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Load {
                                pointer: ValueId::new(3),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(4),
                                length: ValueId::new(7),
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(5),
                                length: ValueId::new(8),
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(10),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Binary {
                                op: BinaryOp::Multiply,
                                left: ValueId::new(5),
                                right: ValueId::new(7),
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(11),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Binary {
                                op: BinaryOp::Add,
                                left: ValueId::new(10),
                                right: ValueId::new(4),
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(11),
                                length: ValueId::new(9),
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(12),
                                ty: TypeId::new(2),
                            }),
                            InstructionKind::Offset {
                                base: ValueId::new(0),
                                indices: vec![ValueId::new(11)],
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::Store {
                                pointer: ValueId::new(12),
                                value: ValueId::new(6),
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

    fn jir_storage_global_2d_strided_write_module() -> Module {
        let instruction = |result, kind| Instruction {
            result,
            kind,
            span: None,
        };
        let pointer = Type::Pointer {
            pointee: TypeId::new(1),
            address_space: AddressSpace::Storage,
        };
        Module {
            types: vec![
                Type::Unit,
                Type::Integer {
                    signed: false,
                    bits: 32,
                },
                pointer,
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "global_2d_strided_write_u32".to_owned(),
                linkage: Linkage::Export,
                parameters: (0..6)
                    .map(|index| Parameter {
                        value: ValueId::new(index),
                        ty: TypeId::new(2),
                        name: Some(
                            [
                                "buffer", "width", "height", "stride_x", "stride_y", "capacity",
                            ][index]
                                .to_owned(),
                        ),
                    })
                    .collect(),
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(6),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(7),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdY),
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(8),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Constant(Constant::Integer { value: 42 }),
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(9),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Load {
                                pointer: ValueId::new(1),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(10),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Load {
                                pointer: ValueId::new(2),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(11),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Load {
                                pointer: ValueId::new(3),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(12),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Load {
                                pointer: ValueId::new(4),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(13),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Load {
                                pointer: ValueId::new(5),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
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
                            Some(TypedValue {
                                value: ValueId::new(14),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Binary {
                                op: BinaryOp::Multiply,
                                left: ValueId::new(6),
                                right: ValueId::new(11),
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(15),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Binary {
                                op: BinaryOp::Multiply,
                                left: ValueId::new(7),
                                right: ValueId::new(12),
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(16),
                                ty: TypeId::new(1),
                            }),
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
                            Some(TypedValue {
                                value: ValueId::new(17),
                                ty: TypeId::new(2),
                            }),
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

    fn jir_storage_global_3d_strided_write_module() -> Module {
        let instruction = |result, kind| Instruction {
            result,
            kind,
            span: None,
        };
        let pointer = Type::Pointer {
            pointee: TypeId::new(1),
            address_space: AddressSpace::Storage,
        };
        Module {
            types: vec![
                Type::Unit,
                Type::Integer {
                    signed: false,
                    bits: 32,
                },
                pointer,
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "global_3d_strided_write_u32".to_owned(),
                linkage: Linkage::Export,
                parameters: (0..8)
                    .map(|index| Parameter {
                        value: ValueId::new(index),
                        ty: TypeId::new(2),
                        name: Some(
                            [
                                "buffer", "width", "height", "depth", "stride_x", "stride_y",
                                "stride_z", "capacity",
                            ][index]
                                .to_owned(),
                        ),
                    })
                    .collect(),
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(8),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(9),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdY),
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(10),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdZ),
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(11),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Constant(Constant::Integer { value: 42 }),
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(12),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Load {
                                pointer: ValueId::new(1),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(13),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Load {
                                pointer: ValueId::new(2),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(14),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Load {
                                pointer: ValueId::new(3),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(15),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Load {
                                pointer: ValueId::new(4),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(16),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Load {
                                pointer: ValueId::new(5),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(17),
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
                                value: ValueId::new(18),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Load {
                                pointer: ValueId::new(7),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(8),
                                length: ValueId::new(12),
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(9),
                                length: ValueId::new(13),
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(10),
                                length: ValueId::new(14),
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(19),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Binary {
                                op: BinaryOp::Multiply,
                                left: ValueId::new(8),
                                right: ValueId::new(15),
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(20),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Binary {
                                op: BinaryOp::Multiply,
                                left: ValueId::new(9),
                                right: ValueId::new(16),
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(21),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Binary {
                                op: BinaryOp::Add,
                                left: ValueId::new(19),
                                right: ValueId::new(20),
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(22),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Binary {
                                op: BinaryOp::Multiply,
                                left: ValueId::new(10),
                                right: ValueId::new(17),
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(23),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Binary {
                                op: BinaryOp::Add,
                                left: ValueId::new(21),
                                right: ValueId::new(22),
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(23),
                                length: ValueId::new(18),
                            },
                        ),
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

    fn jir_storage_global_3d_write_module() -> Module {
        let instruction = |result, kind| Instruction {
            result,
            kind,
            span: None,
        };
        let pointer = Type::Pointer {
            pointee: TypeId::new(1),
            address_space: AddressSpace::Storage,
        };
        Module {
            types: vec![
                Type::Unit,
                Type::Integer {
                    signed: false,
                    bits: 32,
                },
                pointer,
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "global_3d_write_u32".to_owned(),
                linkage: Linkage::Export,
                parameters: vec![
                    Parameter {
                        value: ValueId::new(0),
                        ty: TypeId::new(2),
                        name: Some("buffer".to_owned()),
                    },
                    Parameter {
                        value: ValueId::new(1),
                        ty: TypeId::new(2),
                        name: Some("width".to_owned()),
                    },
                    Parameter {
                        value: ValueId::new(2),
                        ty: TypeId::new(2),
                        name: Some("height".to_owned()),
                    },
                    Parameter {
                        value: ValueId::new(3),
                        ty: TypeId::new(2),
                        name: Some("depth".to_owned()),
                    },
                    Parameter {
                        value: ValueId::new(4),
                        ty: TypeId::new(2),
                        name: Some("capacity".to_owned()),
                    },
                ],
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(5),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdX),
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(6),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdY),
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(7),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Builtin(BuiltinOp::GlobalInvocationIdZ),
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(8),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Constant(Constant::Integer { value: 42 }),
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(9),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Load {
                                pointer: ValueId::new(1),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(10),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Load {
                                pointer: ValueId::new(2),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(11),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Load {
                                pointer: ValueId::new(3),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(12),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Load {
                                pointer: ValueId::new(4),
                                alignment: 4,
                                volatile: false,
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(5),
                                length: ValueId::new(9),
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(6),
                                length: ValueId::new(10),
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(7),
                                length: ValueId::new(11),
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(13),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Binary {
                                op: BinaryOp::Multiply,
                                left: ValueId::new(7),
                                right: ValueId::new(10),
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(14),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Binary {
                                op: BinaryOp::Add,
                                left: ValueId::new(13),
                                right: ValueId::new(6),
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(15),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Binary {
                                op: BinaryOp::Multiply,
                                left: ValueId::new(14),
                                right: ValueId::new(9),
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(16),
                                ty: TypeId::new(1),
                            }),
                            InstructionKind::Binary {
                                op: BinaryOp::Add,
                                left: ValueId::new(15),
                                right: ValueId::new(5),
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::BoundsCheck {
                                index: ValueId::new(16),
                                length: ValueId::new(12),
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(17),
                                ty: TypeId::new(2),
                            }),
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

    fn jir_storage_global_index_add_dynamic_length_module() -> Module {
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
                name: "global_add_dynamic_u32".to_owned(),
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
                            InstructionKind::Constant(Constant::Integer { value: 1 }),
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
                                op: BinaryOp::Add,
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

    fn jir_storage_global_index_fadd_dynamic_length_module() -> Module {
        let mut module = jir_storage_global_index_add_dynamic_length_module();
        module.types = vec![
            Type::Unit,
            Type::Integer {
                signed: false,
                bits: 32,
            },
            Type::Float { bits: 32 },
            Type::Pointer {
                pointee: TypeId::new(2),
                address_space: AddressSpace::Storage,
            },
            Type::Pointer {
                pointee: TypeId::new(1),
                address_space: AddressSpace::Storage,
            },
        ];
        let function = &mut module.functions[0];
        function.name = "global_add_dynamic_f32".to_owned();
        function.parameters[0].ty = TypeId::new(3);
        function.parameters[1].ty = TypeId::new(3);
        function.parameters[2].ty = TypeId::new(4);

        let instructions = &mut function.blocks[0].instructions;
        instructions[1].result.as_mut().expect("constant result").ty = TypeId::new(2);
        instructions[1].kind = InstructionKind::Constant(Constant::FloatBits { bits: 0x3f80_0000 });
        instructions[4]
            .result
            .as_mut()
            .expect("input offset result")
            .ty = TypeId::new(3);
        instructions[5]
            .result
            .as_mut()
            .expect("input load result")
            .ty = TypeId::new(2);
        instructions[6].result.as_mut().expect("add result").ty = TypeId::new(2);
        instructions[7]
            .result
            .as_mut()
            .expect("output offset result")
            .ty = TypeId::new(3);
        module
    }

    fn jir_storage_global_index_vector_fadd_dynamic_length_module() -> Module {
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
                name: "global_add_dynamic_f32x4".to_owned(),
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
                                ty: TypeId::new(2),
                            }),
                            InstructionKind::Constant(Constant::FloatBits { bits: 0x3f80_0000 }),
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(5),
                                ty: TypeId::new(3),
                            }),
                            InstructionKind::VectorSplat {
                                value: ValueId::new(4),
                                lanes: 4,
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(6),
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
                                length: ValueId::new(6),
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(7),
                                ty: TypeId::new(4),
                            }),
                            InstructionKind::Offset {
                                base: ValueId::new(0),
                                indices: vec![ValueId::new(3)],
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(8),
                                ty: TypeId::new(3),
                            }),
                            InstructionKind::Load {
                                pointer: ValueId::new(7),
                                alignment: 16,
                                volatile: false,
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(9),
                                ty: TypeId::new(3),
                            }),
                            InstructionKind::VectorBinary {
                                op: BinaryOp::Add,
                                left: ValueId::new(8),
                                right: ValueId::new(5),
                            },
                        ),
                        instruction(
                            Some(TypedValue {
                                value: ValueId::new(10),
                                ty: TypeId::new(4),
                            }),
                            InstructionKind::Offset {
                                base: ValueId::new(1),
                                indices: vec![ValueId::new(3)],
                            },
                        ),
                        instruction(
                            None,
                            InstructionKind::Store {
                                pointer: ValueId::new(10),
                                value: ValueId::new(9),
                                alignment: 16,
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

    #[test]
    fn emits_deterministic_compute_module() {
        let module = empty_kernel("main");
        let options = SpirvOptions::new([64, 1, 1]).unwrap();
        let first = emit_compute(&module, FunctionId::new(0), options).unwrap();
        let second = emit_compute(&module, FunctionId::new(0), options).unwrap();
        assert_eq!(first, second);
        assert_eq!(&first[..5], &[SPIRV_MAGIC, SPIRV_VERSION_1_3, 0, 5, 0]);
        validate_spirv(&first).unwrap();
    }

    #[test]
    fn emits_descriptor_bound_storage_noop() {
        let words = emit_storage_noop("main", SpirvOptions::new([1, 1, 1]).unwrap()).unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.len() > 40);
        assert!(words.contains(&10));
    }

    #[test]
    fn emits_descriptor_bound_storage_write_fixture() {
        let words = emit_storage_write("main", SpirvOptions::new([1, 1, 1]).unwrap(), 42).unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&42));
        assert!(words.contains(&((3_u32 << 16) | u32::from(OP_STORE))));
    }

    #[test]
    fn emits_descriptor_bound_storage_add() {
        let words = emit_storage_add("main", SpirvOptions::new([1, 1, 1]).unwrap(), 1).unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&1));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_IADD))));
    }

    #[test]
    fn emits_descriptor_bound_storage_dual_add() {
        let words =
            emit_storage_dual_add("main", SpirvOptions::new([1, 1, 1]).unwrap(), 1).unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&1));
        assert!(words.contains(&2));
    }

    #[test]
    fn emits_descriptor_bound_storage_dynamic_index_add() {
        let words =
            emit_storage_dynamic_index_add("main", SpirvOptions::new([1, 1, 1]).unwrap(), 1)
                .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&((6_u32 << 16) | u32::from(OP_ACCESS_CHAIN))));
    }

    #[test]
    fn emits_descriptor_bound_storage_global_index_add() {
        let words =
            emit_storage_global_index_add("main", SpirvOptions::new([64, 1, 1]).unwrap(), 1)
                .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&((4_u32 << 16) | u32::from(OP_TYPE_VECTOR))));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_COMPOSITE_EXTRACT))));
        assert!(words.contains(&BUILT_IN_GLOBAL_INVOCATION_ID));
    }

    #[test]
    fn emits_bounds_safe_storage_global_index_add() {
        let words = emit_storage_global_index_add_bounded(
            "main",
            SpirvOptions::new([64, 1, 1]).unwrap(),
            1,
            64,
        )
        .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_ULT))));
        assert!(words.contains(&((4_u32 << 16) | u32::from(OP_BRANCH_CONDITIONAL))));
    }

    #[test]
    fn emits_bounds_safe_storage_global_index_write() {
        let words =
            emit_storage_global_index_write("main", SpirvOptions::new([64, 1, 1]).unwrap(), 42, 64)
                .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&42));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_ULT))));
        assert!(words.contains(&((3_u32 << 16) | u32::from(OP_STORE))));
    }

    #[test]
    fn emits_bounds_safe_storage_global_index_strided_write() {
        let words = emit_storage_global_index_strided_write(
            "main",
            SpirvOptions::new([64, 1, 1]).unwrap(),
            42,
        )
        .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&42));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_IMUL))));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_ULT))));
        assert!(words.contains(&4));
    }

    #[test]
    fn emits_bounds_safe_storage_global_2d_write() {
        let words = emit_storage_global_2d_write("main", SpirvOptions::new([8, 8, 1]).unwrap(), 42)
            .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&42));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_IMUL))));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_IADD))));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_LOGICAL_AND))));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_ULT))));
    }

    #[test]
    fn emits_bounds_safe_storage_global_2d_strided_write() {
        let words =
            emit_storage_global_2d_strided_write("main", SpirvOptions::new([4, 4, 1]).unwrap(), 42)
                .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&42));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_IMUL))));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_IADD))));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_LOGICAL_AND))));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_ULT))));
    }

    #[test]
    fn emits_bounds_safe_storage_global_3d_write() {
        let words = emit_storage_global_3d_write("main", SpirvOptions::new([4, 4, 4]).unwrap(), 42)
            .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&42));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_IMUL))));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_IADD))));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_LOGICAL_AND))));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_ULT))));
    }

    #[test]
    fn emits_bounds_safe_storage_global_3d_strided_write() {
        let words =
            emit_storage_global_3d_strided_write("main", SpirvOptions::new([4, 4, 2]).unwrap(), 42)
                .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&42));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_IMUL))));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_IADD))));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_LOGICAL_AND))));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_ULT))));
    }

    #[test]
    fn emits_dynamic_length_storage_global_index_add() {
        let words = emit_storage_global_index_add_dynamic_length(
            "main",
            SpirvOptions::new([64, 1, 1]).unwrap(),
            1,
        )
        .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_ULT))));
        assert!(words.contains(&3));
    }

    #[test]
    fn emits_dynamic_length_storage_global_index_multiply() {
        let words = emit_storage_global_index_multiply_dynamic_length(
            "main",
            SpirvOptions::new([64, 1, 1]).unwrap(),
            2,
        )
        .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_IMUL))));
        assert!(words.contains(&2));
    }

    #[test]
    fn emits_bounds_safe_dynamic_length_storage_global_index_fadd() {
        let words = emit_storage_global_index_fadd_dynamic_length(
            "main",
            SpirvOptions::new([64, 1, 1]).unwrap(),
            0x3f80_0000,
        )
        .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_FADD))));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_ULT))));
        assert!(words.contains(&0x3f80_0000));
    }

    #[test]
    fn emits_bounds_safe_dynamic_length_storage_global_index_vector_f32_add() {
        let words = emit_storage_global_index_vector_f32_add_dynamic_length(
            "main",
            SpirvOptions::new([64, 1, 1]).unwrap(),
            0x3f80_0000,
        )
        .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&((7_u32 << 16) | u32::from(OP_CONSTANT_COMPOSITE))));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_FADD))));
        assert!(words.contains(&16));
    }

    #[test]
    fn emits_bounds_safe_dynamic_length_storage_global_index_vector_f32_binary_family() {
        let options = SpirvOptions::new([64, 1, 1]).unwrap();
        for (operation, opcode, operand) in [
            (F32ArithmeticOp::Add, OP_FADD, 0x3f80_0000),
            (F32ArithmeticOp::Subtract, OP_FSUB, 0x3f80_0000),
            (F32ArithmeticOp::Multiply, OP_FMUL, 0x4000_0000),
        ] {
            let words = emit_storage_global_index_vector_f32_binary_dynamic_length(
                "main", options, operand, operation,
            )
            .unwrap();
            validate_spirv(&words).unwrap();
            assert!(words.contains(&((5_u32 << 16) | u32::from(opcode))));
            assert!(words.contains(&operand));
            assert!(words.contains(&16));
        }
    }

    #[test]
    fn emits_vector_f32_binary_lanes_two_through_four() {
        let options = SpirvOptions::new([64, 1, 1]).unwrap();
        for lanes in 2..=4 {
            let words = emit_storage_global_index_vector_f32_binary_dynamic_length_lanes(
                "main",
                options,
                0x3f80_0000,
                F32ArithmeticOp::Add,
                lanes,
            )
            .expect("supported vector lane count");
            validate_spirv(&words).unwrap();
            assert!(words.windows(4).any(|window| {
                window[0] == ((4_u32 << 16) | u32::from(OP_TYPE_VECTOR)) && window[3] == lanes
            }));
            assert!(words.windows(4).any(|window| {
                window[0] == ((4_u32 << 16) | u32::from(OP_DECORATE))
                    && window[1] == 13
                    && window[2] == DECORATION_ARRAY_STRIDE
                    && window[3] == lanes * 4
            }));
            assert!(
                words.iter().any(|word| {
                    *word == (((lanes + 3) << 16) | u32::from(OP_CONSTANT_COMPOSITE))
                })
            );
        }
        assert_eq!(
            emit_storage_global_index_vector_f32_binary_dynamic_length_lanes(
                "main",
                options,
                0x3f80_0000,
                F32ArithmeticOp::Add,
                1,
            ),
            Err(SpirvError::UnsupportedKernelShape(
                "vector f32 lanes must be in 2..=4",
            ))
        );
    }

    #[test]
    fn lowers_vector_f32_add_jir_and_reflects_stride() {
        let module = jir_storage_global_index_vector_fadd_dynamic_length_module();
        let options = SpirvOptions::new([64, 1, 1]).unwrap();
        let artifact = emit_storage_global_index_vector_f32_add_dynamic_length_artifact_from_jir(
            &module,
            FunctionId::new(0),
            options,
        )
        .expect("vector f32 JIR shape is supported");
        assert_eq!(artifact.resources.len(), 3);
        assert_eq!(artifact.resources[0].element_stride, Some(16));
        assert_eq!(artifact.resources[1].element_stride, Some(16));
        assert_eq!(artifact.resources[2].element_stride, Some(4));
        artifact
            .validate()
            .expect("vector artifact is valid SPIR-V");
    }

    #[test]
    fn lowers_vector_f32_binary_family_jir_and_reflects_stride() {
        let base = jir_storage_global_index_vector_fadd_dynamic_length_module();
        let options = SpirvOptions::new([64, 1, 1]).unwrap();
        for operation in [
            F32ArithmeticOp::Add,
            F32ArithmeticOp::Subtract,
            F32ArithmeticOp::Multiply,
        ] {
            let mut module = base.clone();
            let entry_name = format!(
                "global_{}_dynamic_f32x4",
                match operation {
                    F32ArithmeticOp::Add => "add",
                    F32ArithmeticOp::Subtract => "subtract",
                    F32ArithmeticOp::Multiply => "multiply",
                }
            );
            let function = &mut module.functions[0];
            function.name = entry_name;
            if let InstructionKind::VectorBinary { op, .. } =
                &mut function.blocks[0].instructions[7].kind
            {
                *op = operation.as_binary_op();
            }
            let artifact =
                emit_storage_global_index_vector_f32_binary_dynamic_length_artifact_from_jir(
                    &module,
                    FunctionId::new(0),
                    options,
                    operation,
                )
                .expect("vector f32 operation JIR shape is supported");
            assert_eq!(artifact.resources[0].element_stride, Some(16));
            assert_eq!(artifact.resources[1].element_stride, Some(16));
            assert_eq!(artifact.resources[2].element_stride, Some(4));
            artifact
                .validate()
                .expect("vector artifact is valid SPIR-V");
        }
    }

    #[test]
    fn lowers_vector_f32_lanes_two_and_three_and_reflects_stride() {
        let base = jir_storage_global_index_vector_fadd_dynamic_length_module();
        let options = SpirvOptions::new([64, 1, 1]).unwrap();
        for lanes in 2..=3 {
            let mut module = base.clone();
            module.types[3] = Type::Vector {
                element: TypeId::new(2),
                lanes: u16::try_from(lanes).unwrap(),
            };
            module.functions[0].name = format!("global_add_dynamic_f32x{lanes}");
            if let InstructionKind::VectorSplat {
                lanes: vector_lanes,
                ..
            } = &mut module.functions[0].blocks[0].instructions[2].kind
            {
                *vector_lanes = u16::try_from(lanes).unwrap();
            }
            let artifact =
                emit_storage_global_index_vector_f32_binary_dynamic_length_lanes_artifact_from_jir(
                    &module,
                    FunctionId::new(0),
                    options,
                    F32ArithmeticOp::Add,
                    lanes,
                )
                .expect("vector f32 lane shape is supported");
            assert_eq!(artifact.resources[0].element_stride, Some(lanes * 4));
            assert_eq!(artifact.resources[1].element_stride, Some(lanes * 4));
            assert_eq!(artifact.resources[2].element_stride, Some(4));
            artifact
                .validate()
                .expect("vector artifact is valid SPIR-V");
        }
    }

    #[test]
    fn emits_dynamic_length_storage_global_index_f32_binary_family() {
        let options = SpirvOptions::new([64, 1, 1]).unwrap();
        for (operation, opcode, operand) in [
            (F32ArithmeticOp::Subtract, OP_FSUB, 0x3f00_0000),
            (F32ArithmeticOp::Multiply, OP_FMUL, 0x4000_0000),
        ] {
            let words = emit_storage_global_index_f32_binary_dynamic_length(
                "main", options, operand, operation,
            )
            .unwrap();
            validate_spirv(&words).unwrap();
            assert!(words.contains(&((5_u32 << 16) | u32::from(opcode))));
            assert!(words.contains(&operand));
        }
    }

    #[test]
    fn lowers_supported_jir_dynamic_length_fadd() {
        let module = jir_storage_global_index_fadd_dynamic_length_module();
        let words = emit_storage_global_index_fadd_dynamic_length_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([64, 1, 1]).unwrap(),
        )
        .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_FADD))));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_ULT))));
        assert!(words.contains(&0x3f80_0000));
    }

    #[test]
    fn lowers_supported_jir_dynamic_length_f32_binary_family() {
        let options = SpirvOptions::new([64, 1, 1]).unwrap();
        for (operation, jir_operation, opcode) in [
            (F32ArithmeticOp::Subtract, BinaryOp::Subtract, OP_FSUB),
            (F32ArithmeticOp::Multiply, BinaryOp::Multiply, OP_FMUL),
        ] {
            let mut module = jir_storage_global_index_fadd_dynamic_length_module();
            module.functions[0].blocks[0].instructions[6].kind = InstructionKind::Binary {
                op: jir_operation,
                left: ValueId::new(7),
                right: ValueId::new(4),
            };
            let words = emit_storage_global_index_f32_binary_dynamic_length_from_jir(
                &module,
                FunctionId::new(0),
                options,
                operation,
            )
            .unwrap();
            validate_spirv(&words).unwrap();
            assert!(words.contains(&((5_u32 << 16) | u32::from(opcode))));
        }
    }

    #[test]
    fn exports_validated_dynamic_length_fadd_artifact() {
        let module = jir_storage_global_index_fadd_dynamic_length_module();
        let artifact = emit_storage_global_index_fadd_dynamic_length_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([64, 1, 1]).unwrap(),
        )
        .unwrap();
        assert_eq!(artifact.entry_name, "global_add_dynamic_f32");
        assert_eq!(artifact.resources.len(), 3);
        assert!(
            artifact
                .words
                .contains(&((5_u32 << 16) | u32::from(OP_FADD)))
        );
        assert!(
            artifact
                .words
                .contains(&((5_u32 << 16) | u32::from(OP_ULT)))
        );
        assert!(artifact.validate().is_ok());
    }

    #[test]
    fn emits_descriptor_bound_storage_dynamic_index_fadd() {
        let words = emit_storage_dynamic_index_fadd(
            "main",
            SpirvOptions::new([1, 1, 1]).unwrap(),
            0x3f80_0000,
        )
        .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_FADD))));
    }

    #[test]
    fn lowers_supported_jir_storage_write() {
        let module = Module {
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
                name: "write_u32".to_owned(),
                linkage: Linkage::Export,
                parameters: vec![Parameter {
                    value: ValueId::new(0),
                    ty: TypeId::new(2),
                    name: Some("data".to_owned()),
                }],
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        Instruction {
                            result: Some(TypedValue {
                                value: ValueId::new(1),
                                ty: TypeId::new(1),
                            }),
                            kind: InstructionKind::Constant(Constant::Integer { value: 42 }),
                            span: None,
                        },
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
                    ],
                    terminator: Terminator::Return { value: None },
                    span: None,
                }],
                span: None,
            }],
        };
        let words = emit_storage_write_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([1, 1, 1]).unwrap(),
        )
        .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&42));
    }

    #[test]
    fn lowers_supported_jir_storage_add() {
        let module = jir_storage_add_module();
        let words = emit_storage_add_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([1, 1, 1]).unwrap(),
        )
        .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&1));
    }

    #[test]
    fn exports_validated_jir_storage_add_artifact() {
        let module = jir_storage_add_module();
        let artifact = emit_storage_add_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([8, 2, 1]).unwrap(),
        )
        .unwrap();
        assert_eq!(artifact.entry_name, "add_u32");
        assert_eq!(artifact.workgroup_size, [8, 2, 1]);
        assert_eq!(artifact.resources.len(), 1);
        assert_eq!(artifact.resources[0].name, "data");
        assert_eq!(artifact.resources[0].element_stride, Some(4));
        artifact.validate().unwrap();
        assert_eq!(artifact.bytes_le().len(), artifact.words.len() * 4);
    }

    #[test]
    fn exports_validated_dynamic_length_jir_artifact() {
        let module = jir_storage_global_index_add_dynamic_length_module();
        let artifact = emit_storage_global_index_add_dynamic_length_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([64, 1, 1]).unwrap(),
        )
        .unwrap();
        assert_eq!(artifact.entry_name, "global_add_dynamic_u32");
        assert_eq!(artifact.resources.len(), 3);
        assert!(
            artifact
                .resources
                .iter()
                .all(|resource| resource.element_stride == Some(4))
        );
        artifact.validate().unwrap();
    }

    #[test]
    fn lowers_supported_jir_dynamic_length_multiply() {
        let mut module = jir_storage_global_index_add_dynamic_length_module();
        module.functions[0].name = "global_multiply_dynamic_u32".to_owned();
        let binary = &mut module.functions[0].blocks[0].instructions[6];
        binary.kind = InstructionKind::Binary {
            op: BinaryOp::Multiply,
            left: ValueId::new(7),
            right: ValueId::new(4),
        };
        let words = emit_storage_global_index_multiply_dynamic_length_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([64, 1, 1]).unwrap(),
        )
        .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_IMUL))));
    }

    #[test]
    fn lowers_all_supported_jir_u32_binary_operations() {
        let cases = [
            (BinaryOp::Subtract, OP_ISUB),
            (BinaryOp::Multiply, OP_IMUL),
            (BinaryOp::Divide, OP_UDIV),
            (BinaryOp::Remainder, OP_UMOD),
            (BinaryOp::BitAnd, OP_BITWISE_AND),
            (BinaryOp::BitOr, OP_BITWISE_OR),
            (BinaryOp::BitXor, OP_BITWISE_XOR),
            (BinaryOp::ShiftLeft, OP_SHIFT_LEFT_LOGICAL),
            (BinaryOp::ShiftRight, OP_SHIFT_RIGHT_LOGICAL),
        ];
        for (operation, opcode) in cases {
            let mut module = jir_storage_global_index_add_dynamic_length_module();
            module.functions[0].name = format!("global_{:?}_dynamic_u32", operation).to_lowercase();
            module.functions[0].blocks[0].instructions[6].kind = InstructionKind::Binary {
                op: operation,
                left: ValueId::new(7),
                right: ValueId::new(4),
            };
            let words = emit_storage_global_index_binary_dynamic_length_from_jir(
                &module,
                FunctionId::new(0),
                SpirvOptions::new([64, 1, 1]).unwrap(),
                operation,
            )
            .unwrap();
            validate_spirv(&words).unwrap();
            assert!(
                words.contains(&((5_u32 << 16) | u32::from(opcode))),
                "missing SPIR-V opcode {opcode} for {operation:?}"
            );
        }
    }

    #[test]
    fn lowers_reordered_dynamic_length_jir_body() {
        let mut module = jir_storage_global_index_add_dynamic_length_module();
        module.functions[0].blocks[0].instructions.swap(0, 1);
        let words = emit_storage_global_index_add_dynamic_length_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([64, 1, 1]).unwrap(),
        )
        .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_IADD))));
    }

    #[test]
    fn exports_reordered_dynamic_length_binary_artifact() {
        let mut module = jir_storage_global_index_add_dynamic_length_module();
        module.functions[0].blocks[0].instructions.swap(0, 1);
        module.functions[0].blocks[0].instructions[6].kind = InstructionKind::Binary {
            op: BinaryOp::Multiply,
            left: ValueId::new(7),
            right: ValueId::new(4),
        };
        let artifact = emit_storage_global_index_binary_dynamic_length_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([64, 1, 1]).unwrap(),
            BinaryOp::Multiply,
        )
        .unwrap();
        artifact.validate().unwrap();
        assert_eq!(artifact.resources.len(), 3);
        assert!(
            artifact
                .words
                .contains(&((5_u32 << 16) | u32::from(OP_IMUL)))
        );
    }

    #[test]
    fn exports_generic_dynamic_length_binary_artifact() {
        let mut module = jir_storage_global_index_add_dynamic_length_module();
        module.functions[0].name = "global_shift_right_dynamic_u32".to_owned();
        module.functions[0].blocks[0].instructions[6].kind = InstructionKind::Binary {
            op: BinaryOp::ShiftRight,
            left: ValueId::new(7),
            right: ValueId::new(4),
        };
        let artifact = emit_storage_global_index_binary_dynamic_length_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([64, 1, 1]).unwrap(),
            BinaryOp::ShiftRight,
        )
        .unwrap();
        assert_eq!(artifact.entry_name, "global_shift_right_dynamic_u32");
        assert_eq!(artifact.resources.len(), 3);
        artifact.validate().unwrap();
        assert!(
            artifact
                .words
                .contains(&((5_u32 << 16) | u32::from(OP_SHIFT_RIGHT_LOGICAL)))
        );
    }

    #[test]
    fn rejects_undefined_u32_divide_and_shift_operands() {
        for (operation, operand) in [
            (BinaryOp::Divide, 0_i128),
            (BinaryOp::Remainder, 0_i128),
            (BinaryOp::ShiftLeft, 32_i128),
            (BinaryOp::ShiftRight, 32_i128),
        ] {
            let mut module = jir_storage_global_index_add_dynamic_length_module();
            module.functions[0].blocks[0].instructions[1].kind =
                InstructionKind::Constant(Constant::Integer { value: operand });
            module.functions[0].blocks[0].instructions[6].kind = InstructionKind::Binary {
                op: operation,
                left: ValueId::new(7),
                right: ValueId::new(4),
            };
            assert!(matches!(
                emit_storage_global_index_binary_dynamic_length_from_jir(
                    &module,
                    FunctionId::new(0),
                    SpirvOptions::new([64, 1, 1]).unwrap(),
                    operation,
                ),
                Err(SpirvError::UnsupportedKernelShape(_))
            ));
        }
    }

    #[test]
    fn exports_validated_dynamic_length_multiply_artifact() {
        let mut module = jir_storage_global_index_add_dynamic_length_module();
        module.functions[0].name = "global_multiply_dynamic_u32".to_owned();
        module.functions[0].blocks[0].instructions[6].kind = InstructionKind::Binary {
            op: BinaryOp::Multiply,
            left: ValueId::new(7),
            right: ValueId::new(4),
        };
        let artifact = emit_storage_global_index_multiply_dynamic_length_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([64, 1, 1]).unwrap(),
        )
        .unwrap();
        assert_eq!(artifact.entry_name, "global_multiply_dynamic_u32");
        assert_eq!(artifact.resources.len(), 3);
        artifact.validate().unwrap();
        assert!(
            artifact
                .words
                .contains(&((5_u32 << 16) | u32::from(OP_IMUL)))
        );
    }

    #[test]
    fn rejects_dynamic_length_add_when_multiply_is_requested() {
        let module = jir_storage_global_index_add_dynamic_length_module();
        assert!(matches!(
            emit_storage_global_index_multiply_dynamic_length_from_jir(
                &module,
                FunctionId::new(0),
                SpirvOptions::new([64, 1, 1]).unwrap(),
            ),
            Err(SpirvError::UnsupportedKernelShape(_))
        ));
    }

    #[test]
    fn lowers_supported_jir_storage_dual_add() {
        let module = jir_storage_dual_add_module();
        let words = emit_storage_dual_add_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([1, 1, 1]).unwrap(),
        )
        .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&1));
    }

    #[test]
    fn lowers_supported_jir_storage_dynamic_index_add() {
        let module = jir_storage_dynamic_index_add_module();
        let words = emit_storage_dynamic_index_add_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([1, 1, 1]).unwrap(),
        )
        .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&1));
    }

    #[test]
    fn lowers_supported_jir_storage_global_index_add() {
        let module = jir_storage_global_index_add_module();
        let words = emit_storage_global_index_add_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([64, 1, 1]).unwrap(),
        )
        .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&64));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_ULT))));
    }

    #[test]
    fn lowers_supported_jir_storage_global_index_write() {
        let module = jir_storage_global_index_write_module();
        let words = emit_storage_global_index_write_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([64, 1, 1]).unwrap(),
        )
        .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&42));
        assert!(words.contains(&64));
    }

    #[test]
    fn exports_validated_global_index_write_artifact() {
        let module = jir_storage_global_index_write_module();
        let artifact = emit_storage_global_index_write_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([64, 1, 1]).unwrap(),
        )
        .unwrap();
        assert_eq!(artifact.entry_name, "global_write_u32");
        assert_eq!(artifact.resources.len(), 1);
        assert!(artifact.words.contains(&42));
        assert!(
            artifact
                .words
                .contains(&((5_u32 << 16) | u32::from(OP_ULT)))
        );
        assert!(artifact.validate().is_ok());
    }

    #[test]
    fn lowers_supported_jir_storage_global_index_strided_write() {
        let module = jir_storage_global_index_strided_write_module();
        let words = emit_storage_global_index_strided_write_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([64, 1, 1]).unwrap(),
        )
        .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&42));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_IMUL))));
    }

    #[test]
    fn exports_validated_global_index_strided_write_artifact() {
        let module = jir_storage_global_index_strided_write_module();
        let artifact = emit_storage_global_index_strided_write_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([64, 1, 1]).unwrap(),
        )
        .unwrap();
        assert_eq!(artifact.entry_name, "global_strided_write_u32");
        assert_eq!(artifact.resources.len(), 4);
        assert!(
            artifact
                .words
                .contains(&((5_u32 << 16) | u32::from(OP_ULT)))
        );
        assert!(
            artifact
                .words
                .contains(&((5_u32 << 16) | u32::from(OP_IMUL)))
        );
        assert!(artifact.validate().is_ok());
    }

    #[test]
    fn lowers_supported_jir_storage_global_2d_write() {
        let module = jir_storage_global_2d_write_module();
        let words = emit_storage_global_2d_write_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([8, 8, 1]).unwrap(),
        )
        .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_LOGICAL_AND))));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_IMUL))));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_IADD))));
    }

    #[test]
    fn exports_validated_global_2d_write_artifact() {
        let module = jir_storage_global_2d_write_module();
        let artifact = emit_storage_global_2d_write_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([8, 8, 1]).unwrap(),
        )
        .unwrap();
        assert_eq!(artifact.entry_name, "global_2d_write_u32");
        assert_eq!(artifact.resources.len(), 4);
        assert!(
            artifact
                .words
                .contains(&((5_u32 << 16) | u32::from(OP_LOGICAL_AND)))
        );
        assert!(artifact.validate().is_ok());
    }

    #[test]
    fn lowers_supported_jir_storage_global_2d_strided_write() {
        let module = jir_storage_global_2d_strided_write_module();
        let words = emit_storage_global_2d_strided_write_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([4, 4, 1]).unwrap(),
        )
        .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&42));
        assert!(words.contains(&BUILT_IN_GLOBAL_INVOCATION_ID));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_IMUL))));
    }

    #[test]
    fn lowers_supported_jir_storage_global_3d_write() {
        let module = jir_storage_global_3d_write_module();
        let words = emit_storage_global_3d_write_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([4, 4, 4]).unwrap(),
        )
        .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_LOGICAL_AND))));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_IMUL))));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_IADD))));
    }

    #[test]
    fn exports_validated_global_2d_strided_write_artifact() {
        let module = jir_storage_global_2d_strided_write_module();
        let artifact = emit_storage_global_2d_strided_write_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([4, 4, 1]).unwrap(),
        )
        .unwrap();
        assert_eq!(artifact.entry_name, "global_2d_strided_write_u32");
        assert_eq!(artifact.resources.len(), 6);
        assert!(
            artifact
                .words
                .contains(&((5_u32 << 16) | u32::from(OP_IMUL)))
        );
        assert!(artifact.validate().is_ok());
    }

    #[test]
    fn exports_validated_global_3d_write_artifact() {
        let module = jir_storage_global_3d_write_module();
        let artifact = emit_storage_global_3d_write_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([4, 4, 2]).unwrap(),
        )
        .unwrap();
        assert_eq!(artifact.entry_name, "global_3d_write_u32");
        assert_eq!(artifact.resources.len(), 5);
        assert!(
            artifact
                .words
                .contains(&((5_u32 << 16) | u32::from(OP_IMUL)))
        );
        assert!(artifact.validate().is_ok());
    }

    #[test]
    fn exports_validated_global_3d_strided_write_artifact() {
        let module = jir_storage_global_3d_strided_write_module();
        let artifact = emit_storage_global_3d_strided_write_artifact_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([4, 4, 2]).unwrap(),
        )
        .unwrap();
        assert_eq!(artifact.entry_name, "global_3d_strided_write_u32");
        assert_eq!(artifact.resources.len(), 8);
        assert!(
            artifact
                .words
                .contains(&((5_u32 << 16) | u32::from(OP_IMUL)))
        );
        assert!(artifact.validate().is_ok());
    }

    #[test]
    fn lowers_supported_jir_storage_global_3d_strided_write() {
        let module = jir_storage_global_3d_strided_write_module();
        let words = emit_storage_global_3d_strided_write_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([4, 4, 2]).unwrap(),
        )
        .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&42));
        assert!(words.contains(&BUILT_IN_GLOBAL_INVOCATION_ID));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_IMUL))));
        assert!(words.contains(&((5_u32 << 16) | u32::from(OP_IADD))));
    }

    #[test]
    fn lowers_supported_jir_storage_dynamic_index_fadd() {
        let mut module = jir_storage_dynamic_index_add_module();
        module.types = vec![
            Type::Unit,
            Type::Integer {
                signed: false,
                bits: 32,
            },
            Type::Float { bits: 32 },
            Type::Pointer {
                pointee: TypeId::new(2),
                address_space: AddressSpace::Storage,
            },
            Type::Pointer {
                pointee: TypeId::new(1),
                address_space: AddressSpace::Storage,
            },
        ];
        module.functions[0].parameters[0].ty = TypeId::new(3);
        module.functions[0].parameters[1].ty = TypeId::new(3);
        module.functions[0].parameters[2].ty = TypeId::new(4);
        let instructions = &mut module.functions[0].blocks[0].instructions;
        instructions[0].result.as_mut().unwrap().ty = TypeId::new(2);
        instructions[0].kind = InstructionKind::Constant(Constant::FloatBits { bits: 0x3f80_0000 });
        instructions[1].result.as_mut().unwrap().ty = TypeId::new(1);
        instructions[2].result.as_mut().unwrap().ty = TypeId::new(3);
        instructions[3].result.as_mut().unwrap().ty = TypeId::new(2);
        instructions[4].result.as_mut().unwrap().ty = TypeId::new(2);
        instructions[5].result.as_mut().unwrap().ty = TypeId::new(3);
        let words = emit_storage_dynamic_index_fadd_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([1, 1, 1]).unwrap(),
        )
        .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&0x3f80_0000));
    }

    #[test]
    fn lowers_supported_jir_storage_add_offset() {
        let mut module = jir_storage_add_module();
        let instructions = &mut module.functions[0].blocks[0].instructions;
        instructions.insert(
            1,
            Instruction {
                result: Some(TypedValue {
                    value: ValueId::new(2),
                    ty: TypeId::new(1),
                }),
                kind: InstructionKind::Constant(Constant::Integer { value: 1 }),
                span: None,
            },
        );
        instructions.insert(
            2,
            Instruction {
                result: Some(TypedValue {
                    value: ValueId::new(3),
                    ty: TypeId::new(2),
                }),
                kind: InstructionKind::Offset {
                    base: ValueId::new(0),
                    indices: vec![ValueId::new(2)],
                },
                span: None,
            },
        );
        instructions[3].result = Some(TypedValue {
            value: ValueId::new(4),
            ty: TypeId::new(1),
        });
        instructions[3].kind = InstructionKind::Load {
            pointer: ValueId::new(3),
            alignment: 4,
            volatile: false,
        };
        instructions[4].result = Some(TypedValue {
            value: ValueId::new(5),
            ty: TypeId::new(1),
        });
        instructions[4].kind = InstructionKind::Binary {
            op: BinaryOp::Add,
            left: ValueId::new(4),
            right: ValueId::new(1),
        };
        instructions[5].kind = InstructionKind::Store {
            pointer: ValueId::new(3),
            value: ValueId::new(5),
            alignment: 4,
            volatile: false,
        };
        let words = emit_storage_add_from_jir(
            &module,
            FunctionId::new(0),
            SpirvOptions::new([1, 1, 1]).unwrap(),
        )
        .unwrap();
        validate_spirv(&words).unwrap();
        assert!(words.contains(&1));
    }

    #[test]
    fn rejects_unsupported_jir_storage_write_body() {
        let module = Module {
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
                name: "write_u32".to_owned(),
                linkage: Linkage::Export,
                parameters: vec![Parameter {
                    value: ValueId::new(0),
                    ty: TypeId::new(2),
                    name: Some("data".to_owned()),
                }],
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: Vec::new(),
                    terminator: Terminator::Return { value: None },
                    span: None,
                }],
                span: None,
            }],
        };
        assert!(matches!(
            emit_storage_write_from_jir(
                &module,
                FunctionId::new(0),
                SpirvOptions::new([1, 1, 1]).unwrap()
            ),
            Err(SpirvError::UnsupportedKernelShape(_))
        ));
    }

    #[test]
    fn rejects_non_empty_kernel_and_invalid_workgroup() {
        assert_eq!(
            SpirvOptions::new([0, 1, 1]),
            Err(SpirvError::InvalidWorkgroupSize([0, 1, 1]))
        );
        let mut module = empty_kernel("main");
        module.functions[0].blocks[0].terminator = Terminator::Unreachable;
        assert!(matches!(
            emit_compute(
                &module,
                FunctionId::new(0),
                SpirvOptions::new([1, 1, 1]).unwrap()
            ),
            Err(SpirvError::UnsupportedKernelShape(_))
        ));
    }

    #[test]
    fn validator_rejects_corrupt_header_and_word_count() {
        let module = empty_kernel("main");
        let options = SpirvOptions::new([1, 1, 1]).unwrap();
        let words = emit_compute(&module, FunctionId::new(0), options).unwrap();
        let mut bad_magic = words.clone();
        bad_magic[0] = 0;
        assert_eq!(
            validate_spirv(&bad_magic),
            Err(SpirvValidationError::BadMagic(0))
        );
        let mut bad_length = words;
        bad_length[5] = 0;
        assert_eq!(
            validate_spirv(&bad_length),
            Err(SpirvValidationError::ZeroWordCount(5))
        );
    }

    #[test]
    fn validator_requires_local_size_for_selected_entry() {
        let module = empty_kernel("main");
        let options = SpirvOptions::new([1, 1, 1]).unwrap();
        let mut words = emit_compute(&module, FunctionId::new(0), options).unwrap();
        let mut offset = 5;
        while offset < words.len() {
            let word_count = (words[offset] >> 16) as usize;
            let opcode = (words[offset] & 0xffff) as u16;
            if opcode == OP_EXECUTION_MODE {
                words[offset + 1] = 4;
                break;
            }
            offset += word_count;
        }
        assert!(matches!(
            validate_spirv(&words),
            Err(SpirvValidationError::MissingInstruction(
                "OpExecutionMode LocalSize for selected entry"
            ))
        ));
    }

    #[test]
    fn validator_accepts_structural_local_size_id_mode() {
        let module = empty_kernel("main");
        let options = SpirvOptions::new([1, 1, 1]).unwrap();
        let mut words = emit_compute(&module, FunctionId::new(0), options).unwrap();
        words[3] = 9;
        let mut offset = 5;
        while offset < words.len() {
            let word_count = (words[offset] >> 16) as usize;
            let opcode = (words[offset] & 0xffff) as u16;
            if opcode == OP_EXECUTION_MODE {
                words[offset] = (6_u32 << 16) | u32::from(OP_EXECUTION_MODE_ID);
                words[offset + 2] = EXECUTION_MODE_LOCAL_SIZE_ID;
                words[offset + 3] = 6;
                words[offset + 4] = 7;
                words[offset + 5] = 8;
                break;
            }
            offset += word_count;
        }
        let mut constants = Vec::new();
        instruction(&mut constants, OP_TYPE_INT, &[5, 32, 0]);
        instruction(&mut constants, OP_SPEC_CONSTANT, &[5, 6, 1]);
        instruction(&mut constants, OP_SPEC_CONSTANT, &[5, 7, 1]);
        instruction(&mut constants, OP_SPEC_CONSTANT, &[5, 8, 1]);
        let mut insert_at = 5;
        while insert_at < words.len() {
            let word_count = (words[insert_at] >> 16) as usize;
            let opcode = (words[insert_at] & 0xffff) as u16;
            if opcode == OP_FUNCTION {
                words.splice(insert_at..insert_at, constants);
                break;
            }
            insert_at += word_count;
        }
        assert_eq!(validate_spirv(&words), Ok(()));
    }

    #[test]
    fn validator_accepts_spec_constant_op_local_size_id_mode() {
        let module = empty_kernel("main");
        let options = SpirvOptions::new([1, 1, 1]).unwrap();
        let mut words = emit_compute(&module, FunctionId::new(0), options).unwrap();
        words[3] = 10;
        let mut offset = 5;
        while offset < words.len() {
            let word_count = (words[offset] >> 16) as usize;
            let opcode = (words[offset] & 0xffff) as u16;
            if opcode == OP_EXECUTION_MODE {
                words[offset] = (6_u32 << 16) | u32::from(OP_EXECUTION_MODE_ID);
                words[offset + 2] = EXECUTION_MODE_LOCAL_SIZE_ID;
                words[offset + 3] = 6;
                words[offset + 4] = 7;
                words[offset + 5] = 8;
                break;
            }
            offset += word_count;
        }
        let mut constants = Vec::new();
        instruction(&mut constants, OP_TYPE_INT, &[5, 32, 0]);
        instruction(&mut constants, OP_SPEC_CONSTANT, &[5, 6, 1]);
        instruction(&mut constants, OP_SPEC_CONSTANT, &[5, 7, 1]);
        instruction(
            &mut constants,
            OP_SPEC_CONSTANT_OP,
            &[5, 8, u32::from(OP_IADD), 6, 7],
        );
        let mut insert_at = 5;
        while insert_at < words.len() {
            let word_count = (words[insert_at] >> 16) as usize;
            let opcode = (words[insert_at] & 0xffff) as u16;
            if opcode == OP_FUNCTION {
                words.splice(insert_at..insert_at, constants);
                break;
            }
            insert_at += word_count;
        }
        assert_eq!(validate_spirv(&words), Ok(()));
    }

    #[test]
    fn validator_rejects_zero_literal_local_size_id() {
        let module = empty_kernel("main");
        let options = SpirvOptions::new([1, 1, 1]).unwrap();
        let mut words = emit_compute(&module, FunctionId::new(0), options).unwrap();
        words[3] = 9;
        let mut offset = 5;
        while offset < words.len() {
            let word_count = (words[offset] >> 16) as usize;
            let opcode = (words[offset] & 0xffff) as u16;
            if opcode == OP_EXECUTION_MODE {
                words[offset] = (6_u32 << 16) | u32::from(OP_EXECUTION_MODE_ID);
                words[offset + 2] = EXECUTION_MODE_LOCAL_SIZE_ID;
                words[offset + 3] = 6;
                words[offset + 4] = 7;
                words[offset + 5] = 8;
                break;
            }
            offset += word_count;
        }
        let mut constants = Vec::new();
        instruction(&mut constants, OP_TYPE_INT, &[5, 32, 0]);
        instruction(&mut constants, OP_CONSTANT, &[5, 6, 1]);
        instruction(&mut constants, OP_CONSTANT, &[5, 7, 0]);
        instruction(&mut constants, OP_CONSTANT, &[5, 8, 1]);
        let mut insert_at = 5;
        while insert_at < words.len() {
            let word_count = (words[insert_at] >> 16) as usize;
            let opcode = (words[insert_at] & 0xffff) as u16;
            if opcode == OP_FUNCTION {
                words.splice(insert_at..insert_at, constants);
                break;
            }
            insert_at += word_count;
        }
        let validation = validate_spirv(&words);
        println!("invalid signedness validation: {validation:?}");
        assert!(matches!(
            validation,
            Err(SpirvValidationError::InvalidInstruction {
                opcode: OP_EXECUTION_MODE_ID,
                ..
            })
        ));
    }

    #[test]
    fn validator_rejects_invalid_integer_signedness() {
        let module = empty_kernel("main");
        let options = SpirvOptions::new([1, 1, 1]).unwrap();
        let mut words = emit_compute(&module, FunctionId::new(0), options).unwrap();
        words[3] = 9;
        let mut constants = Vec::new();
        instruction(&mut constants, OP_TYPE_INT, &[5, 32, 2]);
        instruction(&mut constants, OP_CONSTANT, &[5, 6, 1]);
        instruction(&mut constants, OP_CONSTANT, &[5, 7, 1]);
        instruction(&mut constants, OP_CONSTANT, &[5, 8, 1]);
        let mut offset = 5;
        while offset < words.len() {
            let word_count = (words[offset] >> 16) as usize;
            let opcode = (words[offset] & 0xffff) as u16;
            if opcode == OP_EXECUTION_MODE {
                words[offset] = (6_u32 << 16) | u32::from(OP_EXECUTION_MODE_ID);
                words[offset + 2] = EXECUTION_MODE_LOCAL_SIZE_ID;
                words[offset + 3] = 6;
                words[offset + 4] = 7;
                words[offset + 5] = 8;
                break;
            }
            offset += word_count;
        }
        let mut insert_at = 5;
        while insert_at < words.len() {
            let word_count = (words[insert_at] >> 16) as usize;
            let opcode = (words[insert_at] & 0xffff) as u16;
            if opcode == OP_FUNCTION {
                words.splice(insert_at..insert_at, constants);
                break;
            }
            insert_at += word_count;
        }
        assert!(matches!(
            validate_spirv(&words),
            Err(SpirvValidationError::InvalidInstruction {
                opcode: OP_TYPE_INT,
                ..
            })
        ));
    }

    #[test]
    fn validator_rejects_non_constant_local_size_id() {
        let module = empty_kernel("main");
        let options = SpirvOptions::new([1, 1, 1]).unwrap();
        let mut words = emit_compute(&module, FunctionId::new(0), options).unwrap();
        let mut offset = 5;
        while offset < words.len() {
            let word_count = (words[offset] >> 16) as usize;
            let opcode = (words[offset] & 0xffff) as u16;
            if opcode == OP_EXECUTION_MODE {
                words[offset] = (6_u32 << 16) | u32::from(OP_EXECUTION_MODE_ID);
                words[offset + 2] = EXECUTION_MODE_LOCAL_SIZE_ID;
                words[offset + 3] = 2;
                words[offset + 4] = 2;
                words[offset + 5] = 2;
                break;
            }
            offset += word_count;
        }
        assert!(matches!(
            validate_spirv(&words),
            Err(SpirvValidationError::InvalidInstruction {
                opcode: OP_EXECUTION_MODE_ID,
                ..
            })
        ));
    }

    #[test]
    fn validator_rejects_mixed_local_size_modes() {
        let module = empty_kernel("main");
        let options = SpirvOptions::new([1, 1, 1]).unwrap();
        let mut words = emit_compute(&module, FunctionId::new(0), options).unwrap();
        let mut offset = 5;
        while offset < words.len() {
            let word_count = (words[offset] >> 16) as usize;
            let opcode = (words[offset] & 0xffff) as u16;
            if opcode == OP_EXECUTION_MODE {
                let mut local_size_id = Vec::new();
                instruction(
                    &mut local_size_id,
                    OP_EXECUTION_MODE_ID,
                    &[1, EXECUTION_MODE_LOCAL_SIZE_ID, 2, 2, 2],
                );
                words.splice(offset..offset + word_count, local_size_id);
                break;
            }
            offset += word_count;
        }
        let mut duplicate = words.clone();
        let mut insert_at = 5;
        while insert_at < duplicate.len() {
            let word_count = (duplicate[insert_at] >> 16) as usize;
            let opcode = (duplicate[insert_at] & 0xffff) as u16;
            if opcode == OP_NAME {
                let mut literal = Vec::new();
                instruction(
                    &mut literal,
                    OP_EXECUTION_MODE,
                    &[1, EXECUTION_MODE_LOCAL_SIZE, 1, 1, 1],
                );
                duplicate.splice(insert_at..insert_at, literal);
                break;
            }
            insert_at += word_count;
        }
        assert!(matches!(
            validate_spirv(&duplicate),
            Err(SpirvValidationError::InvalidInstruction { opcode, .. })
                if opcode == OP_EXECUTION_MODE
        ));
    }

    #[test]
    fn reflection_is_stable_and_conservative() {
        let module = Module {
            types: vec![
                Type::Unit,
                Type::Pointer {
                    pointee: TypeId::new(0),
                    address_space: AddressSpace::Storage,
                },
                Type::Pointer {
                    pointee: TypeId::new(0),
                    address_space: AddressSpace::Uniform,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "resources".to_owned(),
                linkage: Linkage::Export,
                parameters: vec![
                    Parameter {
                        value: jadren_jir::ValueId::new(0),
                        ty: TypeId::new(1),
                        name: Some("positions".to_owned()),
                    },
                    Parameter {
                        value: jadren_jir::ValueId::new(1),
                        ty: TypeId::new(2),
                        name: None,
                    },
                ],
                result: TypeId::new(0),
                blocks: Vec::new(),
                span: None,
            }],
        };
        let resources = reflect_resources(&module, FunctionId::new(0)).unwrap();
        assert_eq!(resources.len(), 2);
        assert_eq!(resources[0].binding, 0);
        assert_eq!(resources[0].name, "positions");
        assert_eq!(resources[0].access, ResourceAccess::ReadWrite);
        assert_eq!(resources[1].name, "resource_1");
        assert_eq!(resources[1].access, ResourceAccess::ReadOnly);
    }

    #[test]
    fn resource_element_type_metadata_parses_shader_names() {
        assert_eq!(
            ResourceElementType::from_shader_name("uint"),
            Some(ResourceElementType::Integer {
                signed: false,
                bits: 32,
                lanes: 1,
            })
        );
        assert_eq!(
            ResourceElementType::from_shader_name("int4"),
            Some(ResourceElementType::Integer {
                signed: true,
                bits: 32,
                lanes: 4,
            })
        );
        assert_eq!(
            ResourceElementType::from_shader_name("float3")
                .and_then(ResourceElementType::byte_stride),
            Some(12)
        );
        assert_eq!(
            ResourceElementType::from_shader_name("half2")
                .and_then(ResourceElementType::byte_stride),
            Some(4)
        );
        assert_eq!(ResourceElementType::from_shader_name("uint5"), None);
        assert_eq!(ResourceElementType::from_shader_name("CustomElement"), None);
    }

    #[test]
    fn reflection_rejects_host_resource_and_duplicate_name() {
        let host_module = Module {
            types: vec![
                Type::Unit,
                Type::Pointer {
                    pointee: TypeId::new(0),
                    address_space: AddressSpace::Heap,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "host".to_owned(),
                linkage: Linkage::Internal,
                parameters: vec![Parameter {
                    value: jadren_jir::ValueId::new(0),
                    ty: TypeId::new(1),
                    name: None,
                }],
                result: TypeId::new(0),
                blocks: Vec::new(),
                span: None,
            }],
        };
        assert_eq!(
            reflect_resources(&host_module, FunctionId::new(0)),
            Err(ReflectionError::HostAddressSpace(AddressSpace::Heap))
        );
        let mut duplicate = host_module.clone();
        duplicate.types[1] = Type::Pointer {
            pointee: TypeId::new(0),
            address_space: AddressSpace::Storage,
        };
        duplicate.functions[0].parameters.push(Parameter {
            value: jadren_jir::ValueId::new(1),
            ty: TypeId::new(1),
            name: Some("resource_0".to_owned()),
        });
        assert_eq!(
            reflect_resources(&duplicate, FunctionId::new(0)),
            Err(ReflectionError::DuplicateName("resource_0".to_owned()))
        );
    }
}
