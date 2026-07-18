//! Target-neutral typed SSA intermediate representation for Jadren.

use std::fmt::{self, Write};

use jadren_source::Span;

mod alias;
mod lower;
mod optimize;
mod verifier;

pub use alias::{AliasAnalysis, AliasRelation, analyze_aliases};
pub use lower::{LowerError, LowerOptions, lower_from_mir};
pub use optimize::{
    OptimizationStats, canonicalize_loops_and_licm, eliminate_proven_bounds_checks,
    eliminate_redundant_bounds_checks, eliminate_redundant_offsets, fold_constants,
    inline_tiny_functions, promote_scalar_stack_slots, simplify_cfg_and_dce,
};
pub use verifier::{VerificationError, verify, verify_gpu};

/// Current stable text format version.
pub const JIR_TEXT_VERSION: &str = "0.1";

macro_rules! dense_id {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(usize);

        impl $name {
            /// Creates an identity from its dense zero-based index.
            #[must_use]
            pub const fn new(index: usize) -> Self {
                Self(index)
            }

            /// Returns the dense zero-based index.
            #[must_use]
            pub const fn index(self) -> usize {
                self.0
            }
        }
    };
}

dense_id!(TypeId, "Module-local canonical JIR type identity.");
dense_id!(FunctionId, "Module-local function identity.");
dense_id!(BlockId, "Function-local basic-block identity.");
dense_id!(ValueId, "Function-local SSA value identity.");

/// Complete target-neutral JIR compilation unit.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Module {
    /// Canonical types in dense identity order.
    pub types: Vec<Type>,
    /// Functions in deterministic declaration order.
    pub functions: Vec<Function>,
}

impl Module {
    /// Renders the canonical deterministic JIR text format.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut output = String::new();
        writeln!(output, "jir {JIR_TEXT_VERSION}").expect("writing to String cannot fail");
        for (index, ty) in self.types.iter().enumerate() {
            writeln!(output, "type %t{index} = {}", TypeDisplay(ty))
                .expect("writing to String cannot fail");
        }
        for function in &self.functions {
            output.push('\n');
            write_function(&mut output, function);
        }
        output
    }
}

/// One canonical JIR type.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Type {
    /// No runtime value.
    Unit,
    /// Opaque lexical region allocator handle.
    RegionHandle,
    /// One logical bit.
    Bool,
    /// Fixed-width integer scalar.
    Integer { signed: bool, bits: u16 },
    /// IEEE-style floating-point scalar.
    Float { bits: u16 },
    /// Raw address in an explicit address space.
    Pointer {
        pointee: TypeId,
        address_space: AddressSpace,
    },
    /// First-class function pointer with a target-neutral signature.
    ///
    /// The pointed-to code may be a local definition or an imported C ABI
    /// declaration.  Calling convention and aggregate ABI adaptation remain
    /// properties of the referenced module function, not of this value type.
    Function {
        parameters: Vec<TypeId>,
        result: TypeId,
    },
    /// Fixed-size inline array.
    Array { element: TypeId, length: u64 },
    /// Anonymous aggregate with stable field order.
    Struct { fields: Vec<TypeId> },
    /// Identity-preserving nominal record/component layout.
    NominalStruct { identity: u64, fields: Vec<TypeId> },
    /// Tagged alternatives with ordered payload fields per variant.
    Enum { variants: Vec<Vec<TypeId>> },
    /// Identity-preserving nominal enum layout.
    NominalEnum {
        identity: u64,
        variants: Vec<Vec<TypeId>>,
    },
    /// Target-neutral SIMD vector.
    Vector { element: TypeId, lanes: u16 },
}

/// Storage/address-space classification preserved until backend lowering.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AddressSpace {
    /// Backend-neutral ordinary address.
    Generic,
    /// Function stack storage.
    Stack,
    /// Runtime heap allocation.
    Heap,
    /// Lexical region allocation.
    Region,
    /// Device-visible global memory.
    Global,
    /// GPU workgroup-shared memory.
    Workgroup,
    /// Read-only uniform/constant memory.
    Uniform,
    /// Read-write storage buffer memory.
    Storage,
}

impl AddressSpace {
    /// Returns whether this space is part of the portable GPU 0.1 subset.
    #[must_use]
    pub const fn is_gpu(self) -> bool {
        matches!(self, Self::Workgroup | Self::Uniform | Self::Storage)
    }

    /// Returns whether this space belongs to the host/CPU memory model.
    #[must_use]
    pub const fn is_host(self) -> bool {
        !self.is_gpu()
    }
}

/// One function definition or external declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Function {
    /// Dense module identity matching its vector position.
    pub id: FunctionId,
    /// Stable source/link spelling.
    pub name: String,
    /// Linkage and definition ownership.
    pub linkage: Linkage,
    /// SSA parameters defined at function entry.
    pub parameters: Vec<Parameter>,
    /// Return value type.
    pub result: TypeId,
    /// Basic blocks; empty only for external declarations.
    pub blocks: Vec<Block>,
    /// Source declaration range when available.
    pub span: Option<Span>,
}

/// Function linkage classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Linkage {
    /// Visible only inside the compilation unit.
    Internal,
    /// Exported with a stable native symbol.
    Export,
    /// Defined by a linked external object/library.
    Import,
}

/// One function parameter and its pre-defined SSA identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Parameter {
    pub value: ValueId,
    pub ty: TypeId,
    pub name: Option<String>,
}

/// One basic block with SSA block parameters instead of implicit phi nodes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Block {
    /// Dense function-local identity matching its vector position.
    pub id: BlockId,
    /// Values selected by incoming edge arguments.
    pub parameters: Vec<BlockParameter>,
    /// Non-terminating instructions in program order.
    pub instructions: Vec<Instruction>,
    /// Exactly one explicit terminator.
    pub terminator: Terminator,
    /// Source range when available.
    pub span: Option<Span>,
}

/// One SSA block parameter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockParameter {
    pub value: ValueId,
    pub ty: TypeId,
}

/// One typed SSA instruction. Side-effect-only instructions have no result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Instruction {
    pub result: Option<TypedValue>,
    pub kind: InstructionKind,
    pub span: Option<Span>,
}

/// SSA identity paired with its canonical type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TypedValue {
    pub value: ValueId,
    pub ty: TypeId,
}

/// Target-neutral operation set before LLVM lowering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InstructionKind {
    Constant(Constant),
    /// Target-provided scalar value with an explicit GPU subset contract.
    Builtin(BuiltinOp),
    /// UTF-8 bytes placed in immutable module storage and exposed as a String value.
    StringLiteral {
        utf8: Vec<u8>,
    },
    /// Constructs an inline array or struct value in layout order.
    Aggregate {
        elements: Vec<ValueId>,
    },
    /// Extracts a constant-position field from an aggregate value.
    ExtractValue {
        aggregate: ValueId,
        index: u32,
    },
    /// Extracts a dynamically indexed inline-array element after a bounds check.
    ExtractElement {
        aggregate: ValueId,
        index: ValueId,
    },
    /// Constructs one tagged enum/carrier variant.
    EnumConstruct {
        variant: u32,
        fields: Vec<ValueId>,
    },
    /// Reads the stable declaration-order variant tag.
    EnumTag {
        value: ValueId,
    },
    /// Extracts a payload field after the controlling edge proved the variant.
    EnumExtract {
        value: ValueId,
        variant: u32,
        field: u32,
    },
    Unary {
        op: UnaryOp,
        operand: ValueId,
    },
    Binary {
        op: BinaryOp,
        left: ValueId,
        right: ValueId,
    },
    Compare {
        predicate: ComparePredicate,
        left: ValueId,
        right: ValueId,
    },
    Cast {
        op: CastOp,
        value: ValueId,
        target: TypeId,
    },
    Select {
        condition: ValueId,
        when_true: ValueId,
        when_false: ValueId,
    },
    StackAlloc {
        ty: TypeId,
        count: Option<ValueId>,
    },
    RegionAlloc {
        region: ValueId,
        ty: TypeId,
        count: ValueId,
    },
    RegionCreate,
    RegionDestroy {
        region: ValueId,
    },
    Drop {
        value: ValueId,
    },
    Load {
        pointer: ValueId,
        alignment: u32,
        volatile: bool,
    },
    Store {
        pointer: ValueId,
        value: ValueId,
        alignment: u32,
        volatile: bool,
    },
    Offset {
        base: ValueId,
        indices: Vec<ValueId>,
    },
    BoundsCheck {
        index: ValueId,
        length: ValueId,
    },
    /// Overflow-safe contiguous vector slice bounds proof.
    VectorBoundsCheck {
        index: ValueId,
        length: ValueId,
        lanes: u16,
    },
    /// Explicit source-level proof that two borrowed aggregate handles do not
    /// refer to overlapping storage. This is a contract marker, not a runtime
    /// check; only validated `@disjoint` Slice/Buffer parameters may produce it.
    AssumeNoAlias {
        left: ValueId,
        right: ValueId,
    },
    Call {
        function: FunctionId,
        arguments: Vec<ValueId>,
    },
    /// Materializes the address of a local or imported module function.
    FunctionAddress {
        function: FunctionId,
    },
    /// Calls a first-class function pointer value.
    IndirectCall {
        callee: ValueId,
        arguments: Vec<ValueId>,
    },
    VectorSplat {
        value: ValueId,
        lanes: u16,
    },
    /// Lane-wise target-neutral arithmetic/logical vector operation.
    VectorBinary {
        op: BinaryOp,
        left: ValueId,
        right: ValueId,
    },
    VectorExtract {
        vector: ValueId,
        lane: ValueId,
    },
    VectorInsert {
        vector: ValueId,
        lane: ValueId,
        value: ValueId,
    },
}

/// Builtin values available to the portable GPU subset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuiltinOp {
    /// The x lane of the compute global invocation id.
    GlobalInvocationIdX,
    /// The y lane of the compute global invocation id.
    GlobalInvocationIdY,
    /// The z lane of the compute global invocation id.
    GlobalInvocationIdZ,
}

/// Literal constant with bit-exact floating representation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Constant {
    Bool(bool),
    Integer { value: i128 },
    FloatBits { bits: u64 },
    Zero,
    Null,
}

/// Unary arithmetic/logical operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnaryOp {
    Negate,
    Not,
    BitNot,
}

/// Binary arithmetic/logical operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinaryOp {
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

/// Scalar `f32` arithmetic operation shared by GPU backend artifact families.
///
/// This is deliberately narrower than [`BinaryOp`]: the backend contract
/// currently supports only operations with well-defined scalar IEEE-754
/// lowering across SPIR-V, DX12 and MSL.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum F32ArithmeticOp {
    Add,
    Subtract,
    Multiply,
}

impl F32ArithmeticOp {
    /// Returns the target-neutral JIR operation represented by this family.
    pub const fn as_binary_op(self) -> BinaryOp {
        match self {
            Self::Add => BinaryOp::Add,
            Self::Subtract => BinaryOp::Subtract,
            Self::Multiply => BinaryOp::Multiply,
        }
    }
}

/// Scalar comparison predicate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComparePredicate {
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

/// Explicit, non-implicit conversion operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CastOp {
    IntegerExtend,
    IntegerTruncate,
    IntegerToFloat,
    FloatToInteger,
    FloatExtend,
    FloatTruncate,
    Bitcast,
    PointerCast,
}

/// Required end of each basic block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Terminator {
    Return {
        value: Option<ValueId>,
    },
    Jump {
        target: BlockId,
        arguments: Vec<ValueId>,
    },
    Branch {
        condition: ValueId,
        then_target: BlockId,
        then_arguments: Vec<ValueId>,
        else_target: BlockId,
        else_arguments: Vec<ValueId>,
    },
    Switch {
        discriminant: ValueId,
        cases: Vec<SwitchCase>,
        default: BlockId,
        default_arguments: Vec<ValueId>,
    },
    Unreachable,
}

/// One constant switch edge with SSA block arguments.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwitchCase {
    pub value: i128,
    pub target: BlockId,
    pub arguments: Vec<ValueId>,
}

struct TypeDisplay<'a>(&'a Type);

impl fmt::Display for TypeDisplay<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Type::Unit => formatter.write_str("unit"),
            Type::RegionHandle => formatter.write_str("region_handle"),
            Type::Bool => formatter.write_str("bool"),
            Type::Integer { signed, bits } => {
                write!(formatter, "{}{bits}", if *signed { 'i' } else { 'u' })
            }
            Type::Float { bits } => write!(formatter, "f{bits}"),
            Type::Pointer {
                pointee,
                address_space,
            } => write!(
                formatter,
                "ptr<{}, %t{}>",
                address_space_name(*address_space),
                pointee.index()
            ),
            Type::Function { parameters, result } => {
                formatter.write_str("fnptr<")?;
                write_type_ids(formatter, parameters)?;
                write!(formatter, " -> %t{}>", result.index())
            }
            Type::Array { element, length } => {
                write!(formatter, "array<{length}, %t{}>", element.index())
            }
            Type::Struct { fields } => {
                formatter.write_str("struct<")?;
                write_type_ids(formatter, fields)?;
                formatter.write_str(">")
            }
            Type::NominalStruct { identity, fields } => {
                write!(formatter, "nominal_struct<0x{identity:016x}; ")?;
                write_type_ids(formatter, fields)?;
                formatter.write_str(">")
            }
            Type::Enum { variants } => {
                formatter.write_str("enum<")?;
                for (index, fields) in variants.iter().enumerate() {
                    if index != 0 {
                        formatter.write_str(", ")?;
                    }
                    formatter.write_str("variant<")?;
                    write_type_ids(formatter, fields)?;
                    formatter.write_str(">")?;
                }
                formatter.write_str(">")
            }
            Type::NominalEnum { identity, variants } => {
                write!(formatter, "nominal_enum<0x{identity:016x}; ")?;
                for (index, fields) in variants.iter().enumerate() {
                    if index != 0 {
                        formatter.write_str(", ")?;
                    }
                    formatter.write_str("variant<")?;
                    write_type_ids(formatter, fields)?;
                    formatter.write_str(">")?;
                }
                formatter.write_str(">")
            }
            Type::Vector { element, lanes } => {
                write!(formatter, "vector<{lanes}, %t{}>", element.index())
            }
        }
    }
}

fn write_type_ids(formatter: &mut fmt::Formatter<'_>, types: &[TypeId]) -> fmt::Result {
    for (index, ty) in types.iter().enumerate() {
        if index != 0 {
            formatter.write_str(", ")?;
        }
        write!(formatter, "%t{}", ty.index())?;
    }
    Ok(())
}

fn write_function(output: &mut String, function: &Function) {
    write!(
        output,
        "fn {} @f{} {:?}(",
        linkage_name(function.linkage),
        function.id.index(),
        function.name
    )
    .expect("writing to String cannot fail");
    for (index, parameter) in function.parameters.iter().enumerate() {
        if index != 0 {
            output.push_str(", ");
        }
        write!(output, "%v{}", parameter.value.index()).expect("writing to String cannot fail");
        if let Some(name) = &parameter.name {
            write!(output, " {name:?}").expect("writing to String cannot fail");
        }
        write!(output, ": %t{}", parameter.ty.index()).expect("writing to String cannot fail");
    }
    write!(output, ") -> %t{}", function.result.index()).expect("writing to String cannot fail");
    if function.linkage == Linkage::Import {
        output.push('\n');
        return;
    }
    output.push_str(" {\n");
    for block in &function.blocks {
        write!(output, "  ^bb{}", block.id.index()).expect("writing to String cannot fail");
        if !block.parameters.is_empty() {
            output.push('(');
            for (index, parameter) in block.parameters.iter().enumerate() {
                if index != 0 {
                    output.push_str(", ");
                }
                write!(
                    output,
                    "%v{}: %t{}",
                    parameter.value.index(),
                    parameter.ty.index()
                )
                .expect("writing to String cannot fail");
            }
            output.push(')');
        }
        output.push_str(":\n");
        for instruction in &block.instructions {
            output.push_str("    ");
            if let Some(result) = instruction.result {
                write!(
                    output,
                    "%v{}: %t{} = ",
                    result.value.index(),
                    result.ty.index()
                )
                .expect("writing to String cannot fail");
            }
            write_instruction(output, &instruction.kind);
            output.push('\n');
        }
        output.push_str("    ");
        write_terminator(output, &block.terminator);
        output.push('\n');
    }
    output.push_str("}\n");
}

fn write_instruction(output: &mut String, instruction: &InstructionKind) {
    match instruction {
        InstructionKind::Constant(constant) => {
            write!(output, "const {constant}").expect("writing to String cannot fail")
        }
        InstructionKind::Builtin(builtin) => write!(output, "builtin {}", builtin_name(*builtin))
            .expect("writing to String cannot fail"),
        InstructionKind::StringLiteral { utf8 } => {
            write!(output, "string {:?}", String::from_utf8_lossy(utf8))
                .expect("writing to String cannot fail")
        }
        InstructionKind::Aggregate { elements } => {
            output.push_str("aggregate ");
            write_values(output, elements);
        }
        InstructionKind::ExtractValue { aggregate, index } => {
            write!(output, "extract_value %v{}, {index}", aggregate.index())
                .expect("writing to String cannot fail")
        }
        InstructionKind::ExtractElement { aggregate, index } => write!(
            output,
            "extract_element %v{}, %v{}",
            aggregate.index(),
            index.index()
        )
        .expect("writing to String cannot fail"),
        InstructionKind::EnumConstruct { variant, fields } => {
            write!(output, "enum_construct {variant}(").expect("writing to String cannot fail");
            write_values(output, fields);
            output.push(')');
        }
        InstructionKind::EnumTag { value } => {
            write!(output, "enum_tag %v{}", value.index()).expect("writing to String cannot fail")
        }
        InstructionKind::EnumExtract {
            value,
            variant,
            field,
        } => write!(
            output,
            "enum_extract %v{}, {variant}, {field}",
            value.index()
        )
        .expect("writing to String cannot fail"),
        InstructionKind::Unary { op, operand } => {
            write!(output, "{} %v{}", unary_name(*op), operand.index())
                .expect("writing to String cannot fail")
        }
        InstructionKind::Binary { op, left, right } => write!(
            output,
            "{} %v{}, %v{}",
            binary_name(*op),
            left.index(),
            right.index()
        )
        .expect("writing to String cannot fail"),
        InstructionKind::Compare {
            predicate,
            left,
            right,
        } => write!(
            output,
            "cmp.{} %v{}, %v{}",
            compare_name(*predicate),
            left.index(),
            right.index()
        )
        .expect("writing to String cannot fail"),
        InstructionKind::Cast { op, value, target } => write!(
            output,
            "cast.{} %v{} to %t{}",
            cast_name(*op),
            value.index(),
            target.index()
        )
        .expect("writing to String cannot fail"),
        InstructionKind::Select {
            condition,
            when_true,
            when_false,
        } => write!(
            output,
            "select %v{}, %v{}, %v{}",
            condition.index(),
            when_true.index(),
            when_false.index()
        )
        .expect("writing to String cannot fail"),
        InstructionKind::StackAlloc { ty, count } => {
            write!(output, "stack_alloc %t{}", ty.index()).expect("writing to String cannot fail");
            if let Some(count) = count {
                write!(output, ", %v{}", count.index()).expect("writing to String cannot fail");
            }
        }
        InstructionKind::RegionAlloc { region, ty, count } => write!(
            output,
            "region_alloc %v{}, %t{}, %v{}",
            region.index(),
            ty.index(),
            count.index()
        )
        .expect("writing to String cannot fail"),
        InstructionKind::RegionCreate => output.push_str("region_create"),
        InstructionKind::RegionDestroy { region } => {
            write!(output, "region_destroy %v{}", region.index())
                .expect("writing to String cannot fail")
        }
        InstructionKind::Drop { value } => {
            write!(output, "drop %v{}", value.index()).expect("writing to String cannot fail")
        }
        InstructionKind::Load {
            pointer,
            alignment,
            volatile,
        } => write!(
            output,
            "load %v{}, align {alignment}{}",
            pointer.index(),
            if *volatile { ", volatile" } else { "" }
        )
        .expect("writing to String cannot fail"),
        InstructionKind::Store {
            pointer,
            value,
            alignment,
            volatile,
        } => write!(
            output,
            "store %v{}, %v{}, align {alignment}{}",
            pointer.index(),
            value.index(),
            if *volatile { ", volatile" } else { "" }
        )
        .expect("writing to String cannot fail"),
        InstructionKind::Offset { base, indices } => {
            write!(output, "offset %v{}", base.index()).expect("writing to String cannot fail");
            for index in indices {
                write!(output, ", %v{}", index.index()).expect("writing to String cannot fail");
            }
        }
        InstructionKind::BoundsCheck { index, length } => write!(
            output,
            "bounds_check %v{}, %v{}",
            index.index(),
            length.index()
        )
        .expect("writing to String cannot fail"),
        InstructionKind::VectorBoundsCheck {
            index,
            length,
            lanes,
        } => write!(
            output,
            "vector_bounds_check %v{}, %v{}, lanes {}",
            index.index(),
            length.index(),
            lanes
        )
        .expect("writing to String cannot fail"),
        InstructionKind::AssumeNoAlias { left, right } => write!(
            output,
            "assume_noalias %v{}, %v{}",
            left.index(),
            right.index()
        )
        .expect("writing to String cannot fail"),
        InstructionKind::Call {
            function,
            arguments,
        } => {
            write!(output, "call @f{}(", function.index()).expect("writing to String cannot fail");
            write_values(output, arguments);
            output.push(')');
        }
        InstructionKind::FunctionAddress { function } => {
            write!(output, "function_address @f{}", function.index())
                .expect("writing to String cannot fail")
        }
        InstructionKind::IndirectCall { callee, arguments } => {
            write!(output, "indirect_call %v{}(", callee.index())
                .expect("writing to String cannot fail");
            write_values(output, arguments);
            output.push(')');
        }
        InstructionKind::VectorSplat { value, lanes } => {
            write!(output, "vector_splat {lanes}, %v{}", value.index())
                .expect("writing to String cannot fail")
        }
        InstructionKind::VectorBinary { op, left, right } => write!(
            output,
            "vector.{} %v{}, %v{}",
            binary_name(*op),
            left.index(),
            right.index()
        )
        .expect("writing to String cannot fail"),
        InstructionKind::VectorExtract { vector, lane } => write!(
            output,
            "vector_extract %v{}, %v{}",
            vector.index(),
            lane.index()
        )
        .expect("writing to String cannot fail"),
        InstructionKind::VectorInsert {
            vector,
            lane,
            value,
        } => write!(
            output,
            "vector_insert %v{}, %v{}, %v{}",
            vector.index(),
            lane.index(),
            value.index()
        )
        .expect("writing to String cannot fail"),
    }
}

fn write_terminator(output: &mut String, terminator: &Terminator) {
    match terminator {
        Terminator::Return { value: None } => output.push_str("return"),
        Terminator::Return { value: Some(value) } => {
            write!(output, "return %v{}", value.index()).expect("writing to String cannot fail");
        }
        Terminator::Jump { target, arguments } => {
            write!(output, "jump ^bb{}(", target.index()).expect("writing to String cannot fail");
            write_values(output, arguments);
            output.push(')');
        }
        Terminator::Branch {
            condition,
            then_target,
            then_arguments,
            else_target,
            else_arguments,
        } => {
            write!(
                output,
                "branch %v{}, ^bb{}(",
                condition.index(),
                then_target.index()
            )
            .expect("writing to String cannot fail");
            write_values(output, then_arguments);
            write!(output, "), ^bb{}(", else_target.index())
                .expect("writing to String cannot fail");
            write_values(output, else_arguments);
            output.push(')');
        }
        Terminator::Switch {
            discriminant,
            cases,
            default,
            default_arguments,
        } => {
            write!(output, "switch %v{} [", discriminant.index())
                .expect("writing to String cannot fail");
            for (index, case) in cases.iter().enumerate() {
                if index != 0 {
                    output.push_str(", ");
                }
                write!(output, "{}: ^bb{}(", case.value, case.target.index())
                    .expect("writing to String cannot fail");
                write_values(output, &case.arguments);
                output.push(')');
            }
            write!(output, "] default ^bb{}(", default.index())
                .expect("writing to String cannot fail");
            write_values(output, default_arguments);
            output.push(')');
        }
        Terminator::Unreachable => output.push_str("unreachable"),
    }
}

fn write_values(output: &mut String, values: &[ValueId]) {
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            output.push_str(", ");
        }
        write!(output, "%v{}", value.index()).expect("writing to String cannot fail");
    }
}

impl fmt::Display for Constant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool(value) => value.fmt(formatter),
            Self::Integer { value } => value.fmt(formatter),
            Self::FloatBits { bits } => write!(formatter, "bits(0x{bits:016x})"),
            Self::Zero => formatter.write_str("zero"),
            Self::Null => formatter.write_str("null"),
        }
    }
}

const fn address_space_name(space: AddressSpace) -> &'static str {
    match space {
        AddressSpace::Generic => "generic",
        AddressSpace::Stack => "stack",
        AddressSpace::Heap => "heap",
        AddressSpace::Region => "region",
        AddressSpace::Global => "global",
        AddressSpace::Workgroup => "workgroup",
        AddressSpace::Uniform => "uniform",
        AddressSpace::Storage => "storage",
    }
}

const fn linkage_name(linkage: Linkage) -> &'static str {
    match linkage {
        Linkage::Internal => "internal",
        Linkage::Export => "export",
        Linkage::Import => "import",
    }
}

const fn unary_name(op: UnaryOp) -> &'static str {
    match op {
        UnaryOp::Negate => "neg",
        UnaryOp::Not => "not",
        UnaryOp::BitNot => "bit_not",
    }
}

const fn binary_name(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "add",
        BinaryOp::Subtract => "sub",
        BinaryOp::Multiply => "mul",
        BinaryOp::Divide => "div",
        BinaryOp::Remainder => "rem",
        BinaryOp::BitAnd => "and",
        BinaryOp::BitOr => "or",
        BinaryOp::BitXor => "xor",
        BinaryOp::ShiftLeft => "shl",
        BinaryOp::ShiftRight => "shr",
    }
}

const fn builtin_name(op: BuiltinOp) -> &'static str {
    match op {
        BuiltinOp::GlobalInvocationIdX => "global_invocation_id.x",
        BuiltinOp::GlobalInvocationIdY => "global_invocation_id.y",
        BuiltinOp::GlobalInvocationIdZ => "global_invocation_id.z",
    }
}

const fn compare_name(predicate: ComparePredicate) -> &'static str {
    match predicate {
        ComparePredicate::Equal => "eq",
        ComparePredicate::NotEqual => "ne",
        ComparePredicate::Less => "lt",
        ComparePredicate::LessEqual => "le",
        ComparePredicate::Greater => "gt",
        ComparePredicate::GreaterEqual => "ge",
    }
}

const fn cast_name(op: CastOp) -> &'static str {
    match op {
        CastOp::IntegerExtend => "int_extend",
        CastOp::IntegerTruncate => "int_truncate",
        CastOp::IntegerToFloat => "int_to_float",
        CastOp::FloatToInteger => "float_to_int",
        CastOp::FloatExtend => "float_extend",
        CastOp::FloatTruncate => "float_truncate",
        CastOp::Bitcast => "bitcast",
        CastOp::PointerCast => "pointer",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BinaryOp, Block, BlockId, Constant, F32ArithmeticOp, Function, FunctionId, Instruction,
        InstructionKind, Linkage, Module, Parameter, Terminator, Type, TypeId, TypedValue, ValueId,
    };

    #[test]
    fn maps_shared_f32_operations_to_jir_binary_operations() {
        assert_eq!(F32ArithmeticOp::Add.as_binary_op(), BinaryOp::Add);
        assert_eq!(F32ArithmeticOp::Subtract.as_binary_op(), BinaryOp::Subtract);
        assert_eq!(F32ArithmeticOp::Multiply.as_binary_op(), BinaryOp::Multiply);
    }

    #[test]
    fn renders_deterministic_typed_ssa_text() {
        let module = Module {
            types: vec![
                Type::Integer {
                    signed: true,
                    bits: 32,
                },
                Type::Unit,
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "add_one".to_owned(),
                linkage: Linkage::Export,
                parameters: vec![Parameter {
                    value: ValueId::new(0),
                    ty: TypeId::new(0),
                    name: Some("value".to_owned()),
                }],
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        Instruction {
                            result: Some(TypedValue {
                                value: ValueId::new(1),
                                ty: TypeId::new(0),
                            }),
                            kind: InstructionKind::Constant(Constant::Integer { value: 1 }),
                            span: None,
                        },
                        Instruction {
                            result: Some(TypedValue {
                                value: ValueId::new(2),
                                ty: TypeId::new(0),
                            }),
                            kind: InstructionKind::Binary {
                                op: BinaryOp::Add,
                                left: ValueId::new(0),
                                right: ValueId::new(1),
                            },
                            span: None,
                        },
                    ],
                    terminator: Terminator::Return {
                        value: Some(ValueId::new(2)),
                    },
                    span: None,
                }],
                span: None,
            }],
        };

        assert_eq!(
            module.to_text(),
            "jir 0.1\ntype %t0 = i32\ntype %t1 = unit\n\nfn export @f0 \"add_one\"(%v0 \"value\": %t0) -> %t0 {\n  ^bb0:\n    %v1: %t0 = const 1\n    %v2: %t0 = add %v0, %v1\n    return %v2\n}\n"
        );
    }
}
