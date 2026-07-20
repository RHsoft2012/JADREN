//! Strict bridge from the bounded self-host stage-2 ABI to canonical JIR.
//!
//! The current format intentionally accepts only exported functions with zero
//! through two typed `Int32` parameters returning one non-negative `Int32` literal,
//! one supported binary
//! operation, one bounded two-operator literal chain, or one bounded literal
//! expression with three through sixteen operators and at most two disjoint,
//! non-nested binary groups. It is an audited
//! migration boundary, not a complete serialized representation of JIR.

use std::collections::BTreeSet;
use std::fmt;

use jadren_jir::{
    BinaryOp, Block, BlockId, Constant, Function, FunctionId, Instruction, InstructionKind,
    Linkage, Module, Terminator, Type, TypeId, TypedValue, ValueId, VerificationError, verify,
};
use jadren_selfhost_api::{
    STAGE2_JIR_BLOCK_ENTRY, STAGE2_JIR_FLAG_EXPORTED, STAGE2_JIR_FLAG_HAS_VALUE,
    STAGE2_JIR_FLAG_SIGNED, STAGE2_JIR_FUNCTION_DEFINITION, STAGE2_JIR_INSTRUCTION_ADD,
    STAGE2_JIR_INSTRUCTION_CONSTANT, STAGE2_JIR_INSTRUCTION_MULTIPLY,
    STAGE2_JIR_INSTRUCTION_SUBTRACT, STAGE2_JIR_MAX_PARAMETERS, STAGE2_JIR_RECORD_BLOCK,
    STAGE2_JIR_RECORD_FUNCTION, STAGE2_JIR_RECORD_INSTRUCTION, STAGE2_JIR_RECORD_TERMINATOR,
    STAGE2_JIR_RECORD_TYPE, STAGE2_JIR_STATUS_COMPLETE, STAGE2_JIR_TERMINATOR_RETURN,
    STAGE2_JIR_TYPE_INTEGER, Stage2JirRecord, Stage2JirSummary, TYPE_KIND_INTEGER,
};
use jadren_source::{SourceId, Span};

const MIN_RECORDS_PER_FUNCTION: u64 = 4;
const MAX_RECORDS_PER_FUNCTION: u64 = 36;
const TYPE_RECORDS: u64 = 1;
const INT32_BITS: u64 = 32;
const CAPTURE_MAGIC: &[u8; 8] = b"JST2CAP1";
const CAPTURE_RECORD_BYTES: usize = 60;

/// Owned canonical capture emitted after a loaded stage-2 producer call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapturedStage2Jir {
    pub source: String,
    pub summary: Stage2JirSummary,
    pub records: Vec<Stage2JirRecord>,
}

/// Malformed or truncated canonical stage-2 capture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Stage2CaptureError {
    pub message: &'static str,
}

impl fmt::Display for Stage2CaptureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for Stage2CaptureError {}

/// Failure produced before an untrusted bounded stage-2 stream reaches a backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Stage2ImportError {
    /// A summary field is inconsistent with the complete bounded contract.
    InvalidSummary {
        field: &'static str,
        expected: u64,
        actual: u64,
    },
    /// The supplied slice is not exactly the complete emitted stream.
    RecordCount { expected: usize, actual: usize },
    /// One record violates its kind-specific ABI contract.
    InvalidRecord { index: usize, message: &'static str },
    /// One record contains a range outside the source or a non-boundary offset.
    InvalidSourceSpan { index: usize, start: u64, end: u64 },
    /// A function record does not point at a valid unique ASCII identifier.
    InvalidFunctionName { index: usize },
    /// The canonical JIR verifier rejected the constructed module.
    Verification(Vec<VerificationError>),
}

impl fmt::Display for Stage2ImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSummary {
                field,
                expected,
                actual,
            } => write!(
                formatter,
                "invalid stage-2 summary field {field}: expected {expected}, got {actual}"
            ),
            Self::RecordCount { expected, actual } => write!(
                formatter,
                "invalid stage-2 record count: expected {expected}, got {actual}"
            ),
            Self::InvalidRecord { index, message } => {
                write!(formatter, "invalid stage-2 record {index}: {message}")
            }
            Self::InvalidSourceSpan { index, start, end } => write!(
                formatter,
                "invalid source span {start}..{end} in stage-2 record {index}"
            ),
            Self::InvalidFunctionName { index } => {
                write!(formatter, "invalid function name in stage-2 record {index}")
            }
            Self::Verification(errors) => write!(
                formatter,
                "imported stage-2 module failed JIR verification with {} error(s)",
                errors.len()
            ),
        }
    }
}

impl std::error::Error for Stage2ImportError {}

/// Decodes the padding-free little-endian capture written by the loaded-producer bridge.
pub fn decode_stage2_capture(bytes: &[u8]) -> Result<CapturedStage2Jir, Stage2CaptureError> {
    let mut cursor = 0usize;
    if capture_take(bytes, &mut cursor, CAPTURE_MAGIC.len())? != CAPTURE_MAGIC {
        return Err(capture_error("invalid stage-2 capture magic"));
    }
    let source_length = usize::try_from(capture_u64(bytes, &mut cursor)?)
        .map_err(|_| capture_error("stage-2 capture source length does not fit usize"))?;
    let source = String::from_utf8(capture_take(bytes, &mut cursor, source_length)?.to_vec())
        .map_err(|_| capture_error("stage-2 capture source is not UTF-8"))?;
    let summary = Stage2JirSummary {
        functions_seen: capture_u64(bytes, &mut cursor)?,
        statements_seen: capture_u64(bytes, &mut cursor)?,
        calls_seen: capture_u64(bytes, &mut cursor)?,
        records_required: capture_u64(bytes, &mut cursor)?,
        records_emitted: capture_u64(bytes, &mut cursor)?,
        functions_lowered: capture_u64(bytes, &mut cursor)?,
        errors: capture_u64(bytes, &mut cursor)?,
        status_flags: capture_u64(bytes, &mut cursor)?,
    };
    let record_count = usize::try_from(capture_u64(bytes, &mut cursor)?)
        .map_err(|_| capture_error("stage-2 capture record count does not fit usize"))?;
    let expected_record_bytes = record_count
        .checked_mul(CAPTURE_RECORD_BYTES)
        .ok_or_else(|| capture_error("stage-2 capture record byte count overflow"))?;
    if bytes.len().saturating_sub(cursor) != expected_record_bytes {
        return Err(capture_error(
            "stage-2 capture record count does not match its remaining bytes",
        ));
    }
    let mut records = Vec::with_capacity(record_count);
    for _ in 0..record_count {
        let header = capture_take(bytes, &mut cursor, 4)?;
        records.push(Stage2JirRecord {
            kind: header[0],
            opcode: header[1],
            type_kind: header[2],
            flags: header[3],
            function_index: capture_u64(bytes, &mut cursor)?,
            block_index: capture_u64(bytes, &mut cursor)?,
            value_index: capture_u64(bytes, &mut cursor)?,
            operand_a: capture_u64(bytes, &mut cursor)?,
            operand_b: capture_u64(bytes, &mut cursor)?,
            source_start: capture_u64(bytes, &mut cursor)?,
            source_end: capture_u64(bytes, &mut cursor)?,
        });
    }
    Ok(CapturedStage2Jir {
        source,
        summary,
        records,
    })
}

/// Imports one complete bounded stage-2 stream into canonical verified JIR.
///
/// The source must be the same immutable UTF-8 text used by the producer. The
/// function rejects truncated and oversized slices instead of silently using a
/// prefix, so no partially emitted stream can reach LLVM or another backend.
pub fn import_stage2_jir(
    source: &str,
    source_id: SourceId,
    summary: Stage2JirSummary,
    records: &[Stage2JirRecord],
) -> Result<Module, Stage2ImportError> {
    validate_summary(summary, records.len())?;
    validate_type_record(&records[0])?;

    let function_count = usize::try_from(summary.functions_seen).map_err(|_| {
        invalid_summary("functions_seen", usize::MAX as u64, summary.functions_seen)
    })?;
    let mut functions = Vec::with_capacity(function_count);
    let mut names = BTreeSet::new();
    let mut previous_function_end = 0usize;
    let mut record_cursor = 1usize;
    let mut leading_group_count = 0u64;

    for function_index in 0..function_count {
        let record_base = record_cursor;
        let function_record = stage2_record(records, record_base)?;
        let block_record = stage2_record(records, record_base + 1)?;

        validate_function_record(function_record, record_base, function_index)?;
        validate_block_record(block_record, record_base + 1, function_index)?;

        let function_span = source_span(source, source_id, function_record, record_base)?;
        let block_span = source_span(source, source_id, block_record, record_base + 1)?;
        if source.as_bytes()[block_span.start] != b'{'
            || source.as_bytes()[block_span.end - 1] != b'}'
        {
            return Err(invalid_record(
                record_base + 1,
                "entry block span is not delimited by braces",
            ));
        }

        let name = function_name(source, function_span.start, record_base)?;
        if !names.insert(name.clone()) {
            return Err(Stage2ImportError::InvalidFunctionName { index: record_base });
        }

        let parameter_count = usize::try_from(function_record.operand_a).map_err(|_| {
            invalid_record(record_base, "stage-2 parameter count does not fit usize")
        })?;
        if parameter_count > usize::from(STAGE2_JIR_MAX_PARAMETERS) {
            return Err(invalid_record(
                record_base,
                "bounded stage-2 function has too many parameters",
            ));
        }
        let parameters = if parameter_count > 0 {
            function_parameters(
                source,
                source_id,
                function_span,
                block_span,
                parameter_count,
                record_base,
            )?
        } else {
            Vec::new()
        };
        let mut instructions = Vec::with_capacity(33);
        let mut value_spans = Vec::with_capacity(33 + parameter_count);
        for (_, parameter_span) in &parameters {
            value_spans.push(*parameter_span);
        }
        let mut constant_spans = Vec::with_capacity(17);
        let mut binary_shapes = Vec::with_capacity(16);
        let mut saw_binary = false;
        let mut instruction_cursor = record_base + 2;
        loop {
            let instruction_record = stage2_record(records, instruction_cursor)?;
            if instruction_record.kind == STAGE2_JIR_RECORD_TERMINATOR {
                break;
            }
            if instruction_record.kind != STAGE2_JIR_RECORD_INSTRUCTION {
                return Err(invalid_record(
                    instruction_cursor,
                    "expected bounded instruction or return terminator",
                ));
            }
            if instructions.len() >= 33 {
                return Err(invalid_record(
                    instruction_cursor,
                    "bounded stage-2 expression exceeds thirty-three SSA instructions",
                ));
            }

            let value_index = parameter_count + instructions.len();
            let instruction_span =
                source_span(source, source_id, instruction_record, instruction_cursor)?;
            if instruction_record.opcode == STAGE2_JIR_INSTRUCTION_CONSTANT {
                if saw_binary || constant_spans.len() >= 17 {
                    return Err(invalid_record(
                        instruction_cursor,
                        "bounded constants must be dense and precede binary instructions",
                    ));
                }
                validate_constant_record(
                    instruction_record,
                    instruction_cursor,
                    function_index,
                    value_index,
                )?;
                validate_literal(
                    source,
                    instruction_span,
                    instruction_record,
                    instruction_cursor,
                )?;
                instructions.push(constant_instruction(
                    instruction_record,
                    instruction_span,
                    value_index,
                ));
                constant_spans.push(instruction_span);
            } else {
                saw_binary = true;
                if binary_shapes.len() >= 16 {
                    return Err(invalid_record(
                        instruction_cursor,
                        "bounded stage-2 expression exceeds sixteen binary instructions",
                    ));
                }
                let (op, left_index, right_index) = validate_binary_record(
                    instruction_record,
                    instruction_cursor,
                    function_index,
                    value_index,
                )?;
                let left_span = value_spans.get(left_index).copied().ok_or_else(|| {
                    invalid_record(instruction_cursor, "binary left operand is not defined")
                })?;
                let right_span = value_spans.get(right_index).copied().ok_or_else(|| {
                    invalid_record(instruction_cursor, "binary right operand is not defined")
                })?;
                if parameter_count == 1 && left_index == 0 {
                    validate_parameter_binary_source(
                        source,
                        &parameters[0].0,
                        right_span,
                        instruction_span,
                        op,
                        instruction_cursor,
                    )?;
                } else if parameter_count == 2 {
                    validate_parameter_pair_binary_source(
                        source,
                        &parameters[0].0,
                        &parameters[1].0,
                        instruction_span,
                        op,
                        instruction_cursor,
                    )?;
                } else {
                    validate_binary_source(
                        source,
                        left_span,
                        right_span,
                        instruction_span,
                        op,
                        instruction_cursor,
                    )?;
                }
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: ValueId::new(value_index),
                        ty: TypeId::new(0),
                    }),
                    kind: InstructionKind::Binary {
                        op,
                        left: ValueId::new(left_index),
                        right: ValueId::new(right_index),
                    },
                    span: Some(instruction_span),
                });
                binary_shapes.push((
                    op,
                    left_index,
                    right_index,
                    instruction_span,
                    instruction_cursor,
                ));
            }
            value_spans.push(instruction_span);
            instruction_cursor += 1;
        }

        if parameter_count == 0 {
            validate_bounded_expression_shape(
                source,
                &constant_spans,
                &binary_shapes,
                record_base,
            )?;
        } else {
            validate_parameter_expression_shape(
                source,
                &parameters,
                &constant_spans,
                &binary_shapes,
                record_base,
            )?;
        }
        let return_value_index = instructions
            .len()
            .checked_add(parameter_count)
            .and_then(|count| count.checked_sub(1))
            .ok_or_else(|| invalid_record(instruction_cursor, "return has no SSA value"))?;
        let return_record = stage2_record(records, instruction_cursor)?;
        validate_return_record(
            return_record,
            instruction_cursor,
            function_index,
            return_value_index,
        )?;
        let return_span = source_span(source, source_id, return_record, instruction_cursor)?;
        let expression_span = value_spans[return_value_index];
        validate_return_source(
            source,
            return_span,
            expression_span,
            instruction_cursor,
            "return <bounded Int32 expression>;",
        )?;
        record_cursor = instruction_cursor + 1;
        let return_value = ValueId::new(return_value_index);
        let expression_start = expression_span.start;
        let expression_end = expression_span.end;
        if !binary_shapes.is_empty() && source.as_bytes()[expression_start] == b'(' {
            leading_group_count = leading_group_count
                .checked_add(1)
                .ok_or_else(|| invalid_summary("calls_seen", u64::MAX, summary.calls_seen))?;
        }

        if function_span.start < previous_function_end
            || function_span.end != block_span.end
            || block_span.start <= function_span.start
            || expression_start < block_span.start
            || expression_end > block_span.end
            || return_span.start < block_span.start
            || return_span.end > block_span.end
            || return_span.start > expression_start
            || expression_end > return_span.end
        {
            return Err(invalid_record(
                record_base,
                "function, block, expression and return spans are not properly nested",
            ));
        }

        previous_function_end = function_span.end;
        functions.push(Function {
            id: FunctionId::new(function_index),
            name,
            linkage: Linkage::Export,
            parameters: parameters
                .into_iter()
                .enumerate()
                .map(|(index, (name, _))| jadren_jir::Parameter {
                    value: ValueId::new(index),
                    ty: TypeId::new(0),
                    name: Some(name),
                })
                .collect(),
            result: TypeId::new(0),
            blocks: vec![Block {
                id: BlockId::new(0),
                parameters: Vec::new(),
                instructions,
                terminator: Terminator::Return {
                    value: Some(return_value),
                },
                span: Some(block_span),
            }],
            span: Some(function_span),
        });
    }

    if record_cursor != records.len() {
        return Err(Stage2ImportError::RecordCount {
            expected: record_cursor,
            actual: records.len(),
        });
    }

    let expected_calls = summary
        .functions_seen
        .checked_add(leading_group_count)
        .ok_or_else(|| invalid_summary("calls_seen", u64::MAX, summary.calls_seen))?;
    require_summary("calls_seen", expected_calls, summary.calls_seen)?;

    let module = Module {
        types: vec![Type::Integer {
            signed: true,
            bits: 32,
        }],
        functions,
    };
    let errors = verify(&module);
    if errors.is_empty() {
        Ok(module)
    } else {
        Err(Stage2ImportError::Verification(errors))
    }
}

fn stage2_record(
    records: &[Stage2JirRecord],
    index: usize,
) -> Result<&Stage2JirRecord, Stage2ImportError> {
    records
        .get(index)
        .ok_or_else(|| invalid_record(index, "stage-2 function record sequence is truncated"))
}

fn constant_instruction(record: &Stage2JirRecord, span: Span, value_index: usize) -> Instruction {
    Instruction {
        result: Some(TypedValue {
            value: ValueId::new(value_index),
            ty: TypeId::new(0),
        }),
        kind: InstructionKind::Constant(Constant::Integer {
            value: i128::from(record.operand_a),
        }),
        span: Some(span),
    }
}

fn validate_summary(
    summary: Stage2JirSummary,
    record_count: usize,
) -> Result<(), Stage2ImportError> {
    if summary.functions_seen == 0 {
        return Err(invalid_summary("functions_seen", 1, 0));
    }
    require_summary(
        "statements_seen",
        summary.functions_seen,
        summary.statements_seen,
    )?;
    let maximum_calls = summary
        .functions_seen
        .checked_mul(2)
        .ok_or_else(|| invalid_summary("calls_seen", u64::MAX, summary.calls_seen))?;
    if summary.calls_seen < summary.functions_seen || summary.calls_seen > maximum_calls {
        return Err(invalid_summary(
            "calls_seen",
            summary.functions_seen,
            summary.calls_seen,
        ));
    }
    require_summary(
        "functions_lowered",
        summary.functions_seen,
        summary.functions_lowered,
    )?;
    require_summary("errors", 0, summary.errors)?;
    require_summary(
        "status_flags",
        STAGE2_JIR_STATUS_COMPLETE,
        summary.status_flags,
    )?;
    let minimum_required = summary
        .functions_seen
        .checked_mul(MIN_RECORDS_PER_FUNCTION)
        .and_then(|count| count.checked_add(TYPE_RECORDS))
        .ok_or_else(|| invalid_summary("records_required", u64::MAX, summary.records_required))?;
    let maximum_required = summary
        .functions_seen
        .checked_mul(MAX_RECORDS_PER_FUNCTION)
        .and_then(|count| count.checked_add(TYPE_RECORDS))
        .ok_or_else(|| invalid_summary("records_required", u64::MAX, summary.records_required))?;
    if summary.records_required < minimum_required || summary.records_required > maximum_required {
        return Err(invalid_summary(
            "records_required",
            minimum_required,
            summary.records_required,
        ));
    }
    require_summary(
        "records_emitted",
        summary.records_required,
        summary.records_emitted,
    )?;
    let expected = usize::try_from(summary.records_required).map_err(|_| {
        invalid_summary(
            "records_required",
            usize::MAX as u64,
            summary.records_required,
        )
    })?;
    if record_count != expected {
        return Err(Stage2ImportError::RecordCount {
            expected,
            actual: record_count,
        });
    }
    Ok(())
}

fn validate_type_record(record: &Stage2JirRecord) -> Result<(), Stage2ImportError> {
    if record.kind != STAGE2_JIR_RECORD_TYPE
        || record.opcode != STAGE2_JIR_TYPE_INTEGER
        || record.type_kind != TYPE_KIND_INTEGER
        || record.flags != STAGE2_JIR_FLAG_SIGNED
        || record.function_index != 0
        || record.block_index != 0
        || record.value_index != 0
        || record.operand_a != INT32_BITS
        || record.operand_b != 0
        || record.source_start != 0
        || record.source_end != 0
    {
        return Err(invalid_record(0, "expected canonical signed Int32 type"));
    }
    Ok(())
}

fn validate_function_record(
    record: &Stage2JirRecord,
    index: usize,
    function_index: usize,
) -> Result<(), Stage2ImportError> {
    if record.kind != STAGE2_JIR_RECORD_FUNCTION
        || record.opcode != STAGE2_JIR_FUNCTION_DEFINITION
        || record.type_kind != TYPE_KIND_INTEGER
        || record.flags != STAGE2_JIR_FLAG_EXPORTED
        || record.function_index != function_index as u64
        || record.block_index != 0
        || record.value_index != 0
        || record.operand_a > u64::from(STAGE2_JIR_MAX_PARAMETERS)
        || record.operand_b != 0
    {
        return Err(invalid_record(
            index,
            "expected dense exported Int32 function with at most two parameters",
        ));
    }
    Ok(())
}

fn validate_block_record(
    record: &Stage2JirRecord,
    index: usize,
    function_index: usize,
) -> Result<(), Stage2ImportError> {
    if record.kind != STAGE2_JIR_RECORD_BLOCK
        || record.opcode != STAGE2_JIR_BLOCK_ENTRY
        || record.type_kind != 0
        || record.flags != 0
        || record.function_index != function_index as u64
        || record.block_index != 0
        || record.value_index != 0
        || record.operand_a != 0
        || record.operand_b != 0
    {
        return Err(invalid_record(index, "expected dense empty entry block"));
    }
    Ok(())
}

fn validate_constant_record(
    record: &Stage2JirRecord,
    index: usize,
    function_index: usize,
    value_index: usize,
) -> Result<(), Stage2ImportError> {
    if record.kind != STAGE2_JIR_RECORD_INSTRUCTION
        || record.opcode != STAGE2_JIR_INSTRUCTION_CONSTANT
        || record.type_kind != TYPE_KIND_INTEGER
        || record.flags != 0
        || record.function_index != function_index as u64
        || record.block_index != 0
        || record.value_index != value_index as u64
        || record.operand_a > i32::MAX as u64
        || record.operand_b != 0
    {
        return Err(invalid_record(
            index,
            "expected dense non-negative Int32 constant",
        ));
    }
    Ok(())
}

fn validate_binary_record(
    record: &Stage2JirRecord,
    index: usize,
    function_index: usize,
    value_index: usize,
) -> Result<(BinaryOp, usize, usize), Stage2ImportError> {
    let op = match record.opcode {
        STAGE2_JIR_INSTRUCTION_ADD => BinaryOp::Add,
        STAGE2_JIR_INSTRUCTION_SUBTRACT => BinaryOp::Subtract,
        STAGE2_JIR_INSTRUCTION_MULTIPLY => BinaryOp::Multiply,
        _ => {
            return Err(invalid_record(
                index,
                "unsupported bounded Int32 binary opcode",
            ));
        }
    };
    if record.kind != STAGE2_JIR_RECORD_INSTRUCTION
        || record.type_kind != TYPE_KIND_INTEGER
        || record.flags != 0
        || record.function_index != function_index as u64
        || record.block_index != 0
        || record.value_index != value_index as u64
    {
        return Err(invalid_record(
            index,
            "expected dense Int32 binary SSA result",
        ));
    }
    let left_index = usize::try_from(record.operand_a).ok();
    let right_index = usize::try_from(record.operand_b).ok();
    let Some((left_index, right_index)) = left_index.zip(right_index) else {
        return Err(invalid_record(
            index,
            "binary operand index does not fit usize",
        ));
    };
    if left_index >= value_index || right_index >= value_index || left_index == right_index {
        return Err(invalid_record(
            index,
            "binary operands must be distinct previously defined SSA values",
        ));
    }
    Ok((op, left_index, right_index))
}

fn validate_return_record(
    record: &Stage2JirRecord,
    index: usize,
    function_index: usize,
    value_index: usize,
) -> Result<(), Stage2ImportError> {
    if record.kind != STAGE2_JIR_RECORD_TERMINATOR
        || record.opcode != STAGE2_JIR_TERMINATOR_RETURN
        || record.type_kind != TYPE_KIND_INTEGER
        || record.flags != STAGE2_JIR_FLAG_HAS_VALUE
        || record.function_index != function_index as u64
        || record.block_index != 0
        || record.value_index != value_index as u64
        || record.operand_a != 0
        || record.operand_b != 0
    {
        return Err(invalid_record(
            index,
            "expected return of a dense SSA value",
        ));
    }
    Ok(())
}

fn source_span(
    source: &str,
    source_id: SourceId,
    record: &Stage2JirRecord,
    index: usize,
) -> Result<Span, Stage2ImportError> {
    let start = usize::try_from(record.source_start).ok();
    let end = usize::try_from(record.source_end).ok();
    let Some((start, end)) = start.zip(end) else {
        return Err(Stage2ImportError::InvalidSourceSpan {
            index,
            start: record.source_start,
            end: record.source_end,
        });
    };
    if start >= end
        || end > source.len()
        || !source.is_char_boundary(start)
        || !source.is_char_boundary(end)
    {
        return Err(Stage2ImportError::InvalidSourceSpan {
            index,
            start: record.source_start,
            end: record.source_end,
        });
    }
    Span::new(source_id, start, end).ok_or(Stage2ImportError::InvalidSourceSpan {
        index,
        start: record.source_start,
        end: record.source_end,
    })
}

fn function_name(
    source: &str,
    start: usize,
    record_index: usize,
) -> Result<String, Stage2ImportError> {
    let bytes = source.as_bytes();
    let Some(first) = bytes.get(start).copied() else {
        return Err(Stage2ImportError::InvalidFunctionName {
            index: record_index,
        });
    };
    if !is_identifier_start(first) {
        return Err(Stage2ImportError::InvalidFunctionName {
            index: record_index,
        });
    }
    let mut end = start + 1;
    while bytes
        .get(end)
        .is_some_and(|byte| is_identifier_continue(*byte))
    {
        end += 1;
    }
    Ok(source[start..end].to_owned())
}

fn function_parameters(
    source: &str,
    source_id: SourceId,
    function_span: Span,
    block_span: Span,
    expected_count: usize,
    record_index: usize,
) -> Result<Vec<(String, Span)>, Stage2ImportError> {
    let function_name = function_name(source, function_span.start, record_index)?;
    let name_end = function_span.start + function_name.len();
    let header = &source[name_end..block_span.start];
    let Some(open) = header.find('(') else {
        return Err(invalid_record(
            record_index,
            "bounded parameter function has no parameter list",
        ));
    };
    let Some(close_offset) = header[open + 1..].find(')') else {
        return Err(invalid_record(
            record_index,
            "bounded parameter function has an unterminated parameter list",
        ));
    };
    let close = open + 1 + close_offset;
    let parameter_text = &header[open + 1..close];
    let mut parameters = Vec::new();
    let mut segment_start = 0usize;
    for segment in parameter_text.split(',') {
        let Some((raw_name, raw_type)) = segment.split_once(':') else {
            return Err(invalid_record(
                record_index,
                "bounded parameter must have a builtin type",
            ));
        };
        let name = raw_name.trim();
        let ty = raw_type.trim();
        if name.is_empty()
            || ty != "Int32"
            || !name.bytes().next().is_some_and(is_identifier_start)
            || !name.bytes().all(is_identifier_continue)
        {
            return Err(invalid_record(
                record_index,
                "bounded parameter must be an identifier of type Int32",
            ));
        }
        let Some(name_offset) = raw_name.find(name) else {
            return Err(invalid_record(
                record_index,
                "bounded parameter name span is not recoverable",
            ));
        };
        let parameter_start = name_end + open + 1 + segment_start + name_offset;
        let parameter_end = parameter_start + name.len();
        let parameter_span =
            Span::new(source_id, parameter_start, parameter_end).ok_or_else(|| {
                invalid_record(record_index, "bounded parameter name span is invalid")
            })?;
        parameters.push((name.to_owned(), parameter_span));
        segment_start += segment.len() + 1;
    }
    if parameters.len() != expected_count
        || parameters.len() > usize::from(STAGE2_JIR_MAX_PARAMETERS)
    {
        return Err(invalid_record(
            record_index,
            "bounded parameter count does not match the function record",
        ));
    }
    Ok(parameters)
}

fn validate_literal(
    source: &str,
    span: Span,
    record: &Stage2JirRecord,
    record_index: usize,
) -> Result<(), Stage2ImportError> {
    let literal = &source[span.start..span.end];
    if literal.is_empty() || !literal.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid_record(
            record_index,
            "constant span is not an unsigned decimal literal",
        ));
    }
    let Ok(parsed) = literal.parse::<u64>() else {
        return Err(invalid_record(
            record_index,
            "constant span exceeds the supported integer range",
        ));
    };
    if parsed != record.operand_a {
        return Err(invalid_record(
            record_index,
            "constant operand does not match the source literal",
        ));
    }
    Ok(())
}

fn validate_return_source(
    source: &str,
    return_span: Span,
    expression_span: Span,
    record_index: usize,
    expected_shape: &'static str,
) -> Result<(), Stage2ImportError> {
    let prefix = source[return_span.start..expression_span.start].trim();
    let suffix = source[expression_span.end..return_span.end].trim();
    if prefix != "return" || suffix != ";" {
        return Err(invalid_record(record_index, expected_shape));
    }
    Ok(())
}

fn validate_parameter_expression_shape(
    source: &str,
    parameters: &[(String, Span)],
    constant_spans: &[Span],
    binary_shapes: &[(BinaryOp, usize, usize, Span, usize)],
    record_base: usize,
) -> Result<(), Stage2ImportError> {
    if binary_shapes.len() != 1 {
        return Err(invalid_record(
            record_base,
            "bounded parameter expression requires one binary node",
        ));
    }
    let shape = binary_shapes[0];
    if shape.1 != 0 || shape.2 != 1 {
        return Err(invalid_record(
            shape.4,
            "bounded parameter expression has an invalid SSA reduction shape",
        ));
    }
    if parameters.len() == 1 {
        if constant_spans.len() != 1 {
            return Err(invalid_record(
                record_base,
                "one bounded parameter requires one literal",
            ));
        }
        validate_parameter_binary_source(
            source,
            &parameters[0].0,
            constant_spans[0],
            shape.3,
            shape.0,
            shape.4,
        )
    } else if parameters.len() == 2 && constant_spans.is_empty() {
        validate_parameter_pair_binary_source(
            source,
            &parameters[0].0,
            &parameters[1].0,
            shape.3,
            shape.0,
            shape.4,
        )
    } else {
        Err(invalid_record(
            record_base,
            "bounded parameters require one parameter-literal or parameter-parameter binary shape",
        ))
    }
}

fn validate_parameter_pair_binary_source(
    source: &str,
    left_name: &str,
    right_name: &str,
    binary_span: Span,
    op: BinaryOp,
    record_index: usize,
) -> Result<(), Stage2ImportError> {
    let expected_operator = match op {
        BinaryOp::Add => "+",
        BinaryOp::Subtract => "-",
        BinaryOp::Multiply => "*",
        _ => {
            return Err(invalid_record(
                record_index,
                "unsupported bounded Int32 binary operation",
            ));
        }
    };
    let Some(left_end) = binary_span.start.checked_add(left_name.len()) else {
        return Err(invalid_record(
            record_index,
            "parameter binary span overflows",
        ));
    };
    if source.get(binary_span.start..left_end) != Some(left_name) {
        return Err(invalid_record(
            record_index,
            "parameter binary left use does not match declaration",
        ));
    }
    let operator_start = source[left_end..binary_span.end]
        .find(expected_operator)
        .map(|offset| left_end + offset)
        .ok_or_else(|| invalid_record(record_index, "parameter binary operator is missing"))?;
    let right_start = operator_start + expected_operator.len();
    if source[left_end..operator_start].trim().is_empty()
        && source[right_start..binary_span.end].trim() == right_name
    {
        Ok(())
    } else {
        Err(invalid_record(
            record_index,
            "parameter binary span does not match the recorded Int32 operator",
        ))
    }
}

fn validate_parameter_binary_source(
    source: &str,
    parameter_name: &str,
    right_span: Span,
    binary_span: Span,
    op: BinaryOp,
    record_index: usize,
) -> Result<(), Stage2ImportError> {
    let expected_operator = match op {
        BinaryOp::Add => "+",
        BinaryOp::Subtract => "-",
        BinaryOp::Multiply => "*",
        _ => {
            return Err(invalid_record(
                record_index,
                "unsupported bounded Int32 binary operation",
            ));
        }
    };
    let Some(parameter_end) = binary_span.start.checked_add(parameter_name.len()) else {
        return Err(invalid_record(
            record_index,
            "bounded parameter binary span overflows",
        ));
    };
    if source.get(binary_span.start..parameter_end) != Some(parameter_name)
        || parameter_end > right_span.start
        || source[parameter_end..right_span.start].trim() != expected_operator
        || binary_span.end != right_span.end
    {
        return Err(invalid_record(
            record_index,
            "parameter binary span does not match the recorded Int32 operator",
        ));
    }
    Ok(())
}

fn validate_binary_source(
    source: &str,
    left_span: Span,
    right_span: Span,
    binary_span: Span,
    op: BinaryOp,
    record_index: usize,
) -> Result<(), Stage2ImportError> {
    let expected_operator = match op {
        BinaryOp::Add => "+",
        BinaryOp::Subtract => "-",
        BinaryOp::Multiply => "*",
        _ => {
            return Err(invalid_record(
                record_index,
                "unsupported bounded Int32 binary operation",
            ));
        }
    };
    let ungrouped = binary_span.start == left_span.start && binary_span.end == right_span.end;
    let grouped = binary_span.start < left_span.start
        && binary_span.end > right_span.end
        && delimiter_run_is(&source[binary_span.start..left_span.start], b'(')
        && delimiter_run_is(&source[right_span.end..binary_span.end], b')');
    if (!ungrouped && !grouped)
        || left_span.end > right_span.start
        || source[left_span.end..right_span.start].trim() != expected_operator
    {
        return Err(invalid_record(
            record_index,
            "binary span does not match the recorded Int32 operator",
        ));
    }
    Ok(())
}

fn validate_bounded_expression_shape(
    source: &str,
    constant_spans: &[Span],
    binary_shapes: &[(BinaryOp, usize, usize, Span, usize)],
    record_base: usize,
) -> Result<(), Stage2ImportError> {
    match (constant_spans.len(), binary_shapes.len()) {
        (1, 0) => Ok(()),
        (2, 1) => {
            let source_op = bounded_source_operator(source, constant_spans[0], constant_spans[1])
                .ok_or_else(|| {
                invalid_record(binary_shapes[0].4, "unsupported bounded source operator")
            })?;
            if binary_shapes[0].0 != source_op || binary_shapes[0].1 != 0 || binary_shapes[0].2 != 1
            {
                return Err(invalid_record(
                    binary_shapes[0].4,
                    "single binary expression has an invalid reduction shape",
                ));
            }
            Ok(())
        }
        (3, 2) => {
            let first_source_op =
                bounded_source_operator(source, constant_spans[0], constant_spans[1]).ok_or_else(
                    || invalid_record(binary_shapes[0].4, "unsupported first source operator"),
                )?;
            let second_source_op =
                bounded_source_operator(source, constant_spans[1], constant_spans[2]).ok_or_else(
                    || invalid_record(binary_shapes[1].4, "unsupported second source operator"),
                )?;
            let first_gap = source[constant_spans[0].end..constant_spans[1].start].trim();
            let second_gap = source[constant_spans[1].end..constant_spans[2].start].trim();
            let left_grouped = second_gap.starts_with(')');
            let right_grouped = first_gap.ends_with('(');
            if left_grouped && right_grouped {
                return Err(invalid_record(
                    binary_shapes[0].4,
                    "bounded chain may contain only one explicit binary group",
                ));
            }
            let reduce_right_first = if left_grouped {
                false
            } else if right_grouped {
                true
            } else {
                second_source_op == BinaryOp::Multiply && first_source_op != BinaryOp::Multiply
            };
            let expected = if reduce_right_first {
                [
                    (second_source_op, 1usize, 2usize),
                    (first_source_op, 0usize, 3usize),
                ]
            } else {
                [
                    (first_source_op, 0usize, 1usize),
                    (second_source_op, 3usize, 2usize),
                ]
            };
            for (index, shape) in binary_shapes.iter().enumerate() {
                if (shape.0, shape.1, shape.2) != expected[index] {
                    return Err(invalid_record(
                        shape.4,
                        "binary chain does not match explicit grouping, multiplication precedence and left associativity",
                    ));
                }
            }
            let first_left_span = constant_spans[binary_shapes[0].1];
            let first_right_span = constant_spans[binary_shapes[0].2];
            let first_is_grouped = binary_span_is_grouped(
                source,
                first_left_span,
                first_right_span,
                binary_shapes[0].3,
            );
            if first_is_grouped != (left_grouped || right_grouped) {
                return Err(invalid_record(
                    binary_shapes[0].4,
                    "first chain reduction does not preserve the explicit source group",
                ));
            }
            let (final_left_span, final_right_span) = if reduce_right_first {
                (constant_spans[0], binary_shapes[0].3)
            } else {
                (binary_shapes[0].3, constant_spans[2])
            };
            if binary_shapes[1].3.start != final_left_span.start
                || binary_shapes[1].3.end != final_right_span.end
            {
                return Err(invalid_record(
                    binary_shapes[1].4,
                    "final chain reduction must span exactly the bounded expression",
                ));
            }
            Ok(())
        }
        (constant_count, binary_count)
            if constant_count == binary_count + 1 && (3..=16).contains(&binary_count) =>
        {
            validate_bounded_expression_plan(
                source,
                constant_spans,
                binary_shapes,
                "three-to-sixteen-operator expression plan does not match explicit grouping, multiplication precedence and left associativity",
            )
        }
        _ => Err(invalid_record(
            record_base,
            "expected one literal or one bounded one-to-sixteen-operator expression",
        )),
    }
}

fn validate_bounded_expression_plan(
    source: &str,
    constant_spans: &[Span],
    binary_shapes: &[(BinaryOp, usize, usize, Span, usize)],
    mismatch_message: &'static str,
) -> Result<(), Stage2ImportError> {
    let mut source_ops = Vec::with_capacity(binary_shapes.len());
    for (pair_index, pair) in constant_spans.windows(2).enumerate() {
        source_ops.push(
            bounded_source_operator(source, pair[0], pair[1]).ok_or_else(|| {
                invalid_record(
                    binary_shapes[pair_index].4,
                    "unsupported source operator in bounded expression plan",
                )
            })?,
        );
    }
    if source_ops.len() != binary_shapes.len() {
        return Err(invalid_record(binary_shapes[0].4, mismatch_message));
    }

    let expression_span = binary_shapes
        .last()
        .map(|shape| shape.3)
        .ok_or_else(|| invalid_record(binary_shapes[0].4, mismatch_message))?;
    let expression_text = &source[expression_span.start..expression_span.end];
    let opening_count = expression_text.bytes().filter(|byte| *byte == b'(').count();
    let closing_count = expression_text.bytes().filter(|byte| *byte == b')').count();
    if opening_count != closing_count || opening_count > 2 {
        return Err(invalid_record(
            binary_shapes[0].4,
            "bounded expression plan permits at most two disjoint non-nested binary groups",
        ));
    }

    let mut groups = Vec::<(usize, usize, usize)>::with_capacity(opening_count);
    if opening_count > 0 {
        for pair_index in 0..source_ops.len() {
            let left = constant_spans[pair_index];
            let right = constant_spans[pair_index + 1];
            let opening_range = if pair_index == 0 {
                expression_span.start..left.start
            } else {
                constant_spans[pair_index - 1].end..left.start
            };
            let closing_range = if pair_index + 1 == constant_spans.len() - 1 {
                right.end..expression_span.end
            } else {
                right.end..constant_spans[pair_index + 2].start
            };
            let opening_text = &source[opening_range.clone()];
            let closing_text = &source[closing_range.clone()];
            let Some(opening_offset) = opening_text.rfind('(') else {
                continue;
            };
            let Some(closing_offset) = closing_text.find(')') else {
                continue;
            };
            if !opening_text[opening_offset..].trim().eq("(")
                || !closing_text[..=closing_offset].trim().eq(")")
            {
                continue;
            }
            groups.push((
                pair_index,
                opening_range.start + opening_offset,
                closing_range.start + closing_offset + 1,
            ));
        }
        if groups.len() == 1 && opening_count == 2 {
            let (group_pair, inner_start, inner_end) = groups[0];
            let source_bytes = source.as_bytes();
            let mut outer_start = inner_start;
            while outer_start > expression_span.start
                && source_bytes[outer_start - 1].is_ascii_whitespace()
            {
                outer_start -= 1;
            }
            let mut outer_end = inner_end;
            while outer_end < expression_span.end && source_bytes[outer_end].is_ascii_whitespace() {
                outer_end += 1;
            }
            if outer_start == expression_span.start
                || outer_end >= expression_span.end
                || source_bytes[outer_start - 1] != b'('
                || source_bytes[outer_end] != b')'
            {
                return Err(invalid_record(
                    binary_shapes[0].4,
                    "bounded expression plan nested group delimiters are not adjacent to one binary pair",
                ));
            }
            groups[0] = (group_pair, outer_start - 1, outer_end + 1);
        } else if groups.len() != opening_count {
            return Err(invalid_record(
                binary_shapes[0].4,
                "bounded expression plan groups must each wrap one adjacent binary pair",
            ));
        }
        if groups.windows(2).any(|pair| pair[1].0 <= pair[0].0 + 1) {
            return Err(invalid_record(
                binary_shapes[0].4,
                "bounded expression plan groups must be disjoint and non-nested",
            ));
        }
    }

    let mut expected = Vec::<(BinaryOp, usize, usize, usize, usize)>::new();
    let mut effective_values =
        Vec::<(usize, usize, usize)>::with_capacity(constant_spans.len() - groups.len());
    let mut effective_ops = Vec::<BinaryOp>::with_capacity(source_ops.len() - groups.len());
    for (group_index, (group_pair, group_start, group_end)) in groups.iter().copied().enumerate() {
        expected.push((
            source_ops[group_pair],
            group_pair,
            group_pair + 1,
            group_start,
            group_end,
        ));
        if group_index > 0 && group_pair <= groups[group_index - 1].0 + 1 {
            return Err(invalid_record(
                binary_shapes[0].4,
                "bounded expression plan groups must be disjoint and source ordered",
            ));
        }
    }
    let mut group_index = 0usize;
    for (literal_index, span) in constant_spans.iter().copied().enumerate() {
        if group_index < groups.len() && literal_index == groups[group_index].0 {
            let (_, group_start, group_end) = groups[group_index];
            effective_values.push((constant_spans.len() + group_index, group_start, group_end));
            group_index += 1;
        } else if groups
            .iter()
            .any(|(group_pair, _, _)| literal_index == group_pair + 1)
        {
            continue;
        } else {
            effective_values.push((literal_index, span.start, span.end));
        }
    }
    effective_ops.extend(source_ops.iter().enumerate().filter_map(|(index, op)| {
        groups
            .iter()
            .all(|(group_pair, _, _)| index != *group_pair)
            .then_some(*op)
    }));

    let mut operator_stack = Vec::<usize>::with_capacity(effective_ops.len());
    let mut value_stack = Vec::<(usize, usize, usize)>::with_capacity(effective_values.len());
    value_stack.push(effective_values[0]);

    for (operator_index, current_op) in effective_ops.iter().copied().enumerate() {
        while operator_stack.last().is_some_and(|top_index| {
            bounded_binary_precedence(effective_ops[*top_index])
                >= bounded_binary_precedence(current_op)
        }) {
            let record_index = binary_shapes[expected.len()].4;
            reduce_expected_expression_node(
                &effective_ops,
                &mut operator_stack,
                &mut value_stack,
                constant_spans.len(),
                &mut expected,
                record_index,
            )?;
        }
        operator_stack.push(operator_index);
        value_stack.push(effective_values[operator_index + 1]);
    }
    while !operator_stack.is_empty() {
        let record_index = binary_shapes[expected.len()].4;
        reduce_expected_expression_node(
            &effective_ops,
            &mut operator_stack,
            &mut value_stack,
            constant_spans.len(),
            &mut expected,
            record_index,
        )?;
    }
    if value_stack.len() != 1 || expected.len() != binary_shapes.len() {
        return Err(invalid_record(binary_shapes[0].4, mismatch_message));
    }
    for (shape, expected_shape) in binary_shapes.iter().zip(&expected) {
        if (shape.0, shape.1, shape.2, shape.3.start, shape.3.end) != *expected_shape {
            return Err(invalid_record(shape.4, mismatch_message));
        }
    }
    Ok(())
}

fn reduce_expected_expression_node(
    source_ops: &[BinaryOp],
    operator_stack: &mut Vec<usize>,
    value_stack: &mut Vec<(usize, usize, usize)>,
    constant_count: usize,
    expected: &mut Vec<(BinaryOp, usize, usize, usize, usize)>,
    record_index: usize,
) -> Result<(), Stage2ImportError> {
    let operator_index = operator_stack.pop().ok_or_else(|| {
        invalid_record(record_index, "bounded expression operator stack is empty")
    })?;
    let right = value_stack.pop().ok_or_else(|| {
        invalid_record(record_index, "bounded expression right operand is missing")
    })?;
    let left = value_stack.pop().ok_or_else(|| {
        invalid_record(record_index, "bounded expression left operand is missing")
    })?;
    let result_index = constant_count + expected.len();
    expected.push((source_ops[operator_index], left.0, right.0, left.1, right.2));
    value_stack.push((result_index, left.1, right.2));
    Ok(())
}

fn bounded_binary_precedence(op: BinaryOp) -> u8 {
    if op == BinaryOp::Multiply { 2 } else { 1 }
}

fn delimiter_run_is(source: &str, delimiter: u8) -> bool {
    let mut count = 0usize;
    for byte in source.bytes() {
        if byte.is_ascii_whitespace() {
            continue;
        }
        if byte != delimiter {
            return false;
        }
        count += 1;
    }
    (1..=2).contains(&count)
}

fn bounded_source_operator(source: &str, left: Span, right: Span) -> Option<BinaryOp> {
    if left.end > right.start {
        return None;
    }
    let mut between = source[left.end..right.start].trim();
    while let Some(rest) = between.strip_prefix(')') {
        between = rest.trim_start();
    }
    while let Some(rest) = between.strip_suffix('(') {
        between = rest.trim_end();
    }
    match between {
        "+" => Some(BinaryOp::Add),
        "-" => Some(BinaryOp::Subtract),
        "*" => Some(BinaryOp::Multiply),
        _ => None,
    }
}

fn binary_span_is_grouped(source: &str, left: Span, right: Span, binary: Span) -> bool {
    binary.start < left.start
        && binary.end > right.end
        && delimiter_run_is(&source[binary.start..left.start], b'(')
        && delimiter_run_is(&source[right.end..binary.end], b')')
}

const fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

const fn is_identifier_continue(byte: u8) -> bool {
    is_identifier_start(byte) || byte.is_ascii_digit()
}

fn require_summary(
    field: &'static str,
    expected: u64,
    actual: u64,
) -> Result<(), Stage2ImportError> {
    if actual == expected {
        Ok(())
    } else {
        Err(invalid_summary(field, expected, actual))
    }
}

const fn invalid_summary(field: &'static str, expected: u64, actual: u64) -> Stage2ImportError {
    Stage2ImportError::InvalidSummary {
        field,
        expected,
        actual,
    }
}

const fn invalid_record(index: usize, message: &'static str) -> Stage2ImportError {
    Stage2ImportError::InvalidRecord { index, message }
}

fn capture_take<'a>(
    bytes: &'a [u8],
    cursor: &mut usize,
    count: usize,
) -> Result<&'a [u8], Stage2CaptureError> {
    let end = cursor
        .checked_add(count)
        .ok_or_else(|| capture_error("stage-2 capture offset overflow"))?;
    let value = bytes
        .get(*cursor..end)
        .ok_or_else(|| capture_error("truncated stage-2 capture"))?;
    *cursor = end;
    Ok(value)
}

fn capture_u64(bytes: &[u8], cursor: &mut usize) -> Result<u64, Stage2CaptureError> {
    let raw: [u8; 8] = capture_take(bytes, cursor, 8)?
        .try_into()
        .map_err(|_| capture_error("invalid stage-2 capture u64 field"))?;
    Ok(u64::from_le_bytes(raw))
}

const fn capture_error(message: &'static str) -> Stage2CaptureError {
    Stage2CaptureError { message }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use std::path::PathBuf;
    #[cfg(windows)]
    use std::process::Command;

    #[cfg(windows)]
    use inkwell::context::Context;
    #[cfg(windows)]
    use jadren_codegen_llvm::{
        ObjectOptions, TypeLoweringConfig, WindowsLinkOptions, link_windows_executable,
        lower_to_object, write_object,
    };
    use jadren_jir::verify;
    use jadren_selfhost_api::{
        STAGE2_JIR_INSTRUCTION_ADD, STAGE2_JIR_INSTRUCTION_MULTIPLY,
        STAGE2_JIR_INSTRUCTION_SUBTRACT, Stage2JirRecord, Stage2JirSummary,
    };
    use jadren_source::SourceManager;

    use super::{Stage2ImportError, decode_stage2_capture, import_stage2_jir};

    const SOURCE: &str = "fn first() -> Int32 { return 7; } fn main() -> Int32 { return 42; }";
    const ADD_SOURCE: &str = "fn main() -> Int32 { return 40 + 2; }";
    const SUBTRACT_SOURCE: &str = "fn main() -> Int32 { return 40 - 2; }";
    const MULTIPLY_SOURCE: &str = "fn main() -> Int32 { return 40 * 2; }";
    const PRECEDENCE_CHAIN_SOURCE: &str = "fn main() -> Int32 { return 2 + 4 * 10; }";
    const LEFT_ASSOCIATIVE_CHAIN_SOURCE: &str = "fn main() -> Int32 { return 50 - 6 - 2; }";
    const GROUPED_RIGHT_CHAIN_SOURCE: &str = "fn main() -> Int32 { return 6 * (3 + 4); }";
    const GROUPED_LEFT_CHAIN_SOURCE: &str = "fn main() -> Int32 { return (50 - 6) - 2; }";
    const GROUPED_SINGLE_BINARY_SOURCE: &str = "fn main() -> Int32 { return (40 + 2); }";
    const LONG_PRECEDENCE_CHAIN_SOURCE: &str = "fn main() -> Int32 { return 2 + 4 * 10 + 0; }";
    const LONG_MULTIPLY_CHAIN_SOURCE: &str = "fn main() -> Int32 { return 2 + 3 * 4 * 5; }";
    const LONG_LEFT_ASSOCIATIVE_CHAIN_SOURCE: &str = "fn main() -> Int32 { return 6 * 7 + 2 - 2; }";
    const EXPRESSION_PLAN_SOURCE: &str = "fn main() -> Int32 { return 2 + 4 * 10 + 5 - 5; }";
    const GROUPED_LONG_CHAIN_SOURCE: &str = "fn main() -> Int32 { return 50 - (2 + 2) * 2; }";
    const GROUPED_EXPRESSION_PLAN_SOURCE: &str =
        "fn main() -> Int32 { return 50 - (2 + 2) * 2 + 0; }";
    const STREAMING_EXPRESSION_PLAN_SOURCE: &str =
        "fn main() -> Int32 { return 2 + 4 * 10 + 5 - 5 + 0 + 0 + 0; }";
    const GROUPED_STREAMING_EXPRESSION_PLAN_SOURCE: &str =
        "fn main() -> Int32 { return 50 - (2 + 2) * 2 + 0 + 0 + 0; }";
    const MULTI_GROUP_STREAMING_EXPRESSION_PLAN_SOURCE: &str =
        "fn main() -> Int32 { return (20 + 1) * 2 + (3 - 3) + 0 + 0; }";
    const NESTED_STREAMING_EXPRESSION_PLAN_SOURCE: &str =
        "fn main() -> Int32 { return ((20 + 1)) * 2 + 0 + 0 + 0; }";
    const PARAMETER_BINARY_SOURCE: &str = "fn main(x: Int32) -> Int32 { return x + 1; }";
    const PARAMETER_PAIR_SOURCE: &str = "fn main(x: Int32, y: Int32) -> Int32 { return x + y; }";

    fn summary() -> Stage2JirSummary {
        Stage2JirSummary {
            functions_seen: 2,
            statements_seen: 2,
            calls_seen: 2,
            records_required: 9,
            records_emitted: 9,
            functions_lowered: 2,
            errors: 0,
            status_flags: 7,
        }
    }

    fn record(
        header: [u8; 4],
        coordinates: [u64; 3],
        operand_a: u64,
        span: [u64; 2],
    ) -> Stage2JirRecord {
        record_with_operands(header, coordinates, [operand_a, 0], span)
    }

    fn record_with_operands(
        header: [u8; 4],
        coordinates: [u64; 3],
        operands: [u64; 2],
        span: [u64; 2],
    ) -> Stage2JirRecord {
        Stage2JirRecord {
            kind: header[0],
            opcode: header[1],
            type_kind: header[2],
            flags: header[3],
            function_index: coordinates[0],
            block_index: coordinates[1],
            value_index: coordinates[2],
            operand_a: operands[0],
            operand_b: operands[1],
            source_start: span[0],
            source_end: span[1],
        }
    }

    fn records() -> Vec<Stage2JirRecord> {
        vec![
            record([1, 1, 2, 1], [0, 0, 0], 32, [0, 0]),
            record([2, 1, 2, 1], [0, 0, 0], 0, [3, 33]),
            record([3, 1, 0, 0], [0, 0, 0], 0, [20, 33]),
            record([4, 1, 2, 0], [0, 0, 0], 7, [29, 30]),
            record([5, 1, 2, 1], [0, 0, 0], 0, [22, 31]),
            record([2, 1, 2, 1], [1, 0, 0], 0, [37, 67]),
            record([3, 1, 0, 0], [1, 0, 0], 0, [53, 67]),
            record([4, 1, 2, 0], [1, 0, 0], 42, [62, 64]),
            record([5, 1, 2, 1], [1, 0, 0], 0, [55, 65]),
        ]
    }

    fn binary_summary() -> Stage2JirSummary {
        Stage2JirSummary {
            functions_seen: 1,
            statements_seen: 1,
            calls_seen: 1,
            records_required: 7,
            records_emitted: 7,
            functions_lowered: 1,
            errors: 0,
            status_flags: 7,
        }
    }

    fn binary_records(opcode: u8) -> Vec<Stage2JirRecord> {
        vec![
            record([1, 1, 2, 1], [0, 0, 0], 32, [0, 0]),
            record([2, 1, 2, 1], [0, 0, 0], 0, [3, 37]),
            record([3, 1, 0, 0], [0, 0, 0], 0, [19, 37]),
            record([4, 1, 2, 0], [0, 0, 0], 40, [28, 30]),
            record([4, 1, 2, 0], [0, 0, 1], 2, [33, 34]),
            record_with_operands([4, opcode, 2, 0], [0, 0, 2], [0, 1], [28, 34]),
            record([5, 1, 2, 1], [0, 0, 2], 0, [21, 35]),
        ]
    }

    fn chain_summary() -> Stage2JirSummary {
        Stage2JirSummary {
            functions_seen: 1,
            statements_seen: 1,
            calls_seen: 1,
            records_required: 9,
            records_emitted: 9,
            functions_lowered: 1,
            errors: 0,
            status_flags: 7,
        }
    }

    fn with_leading_group(mut summary: Stage2JirSummary) -> Stage2JirSummary {
        summary.calls_seen += 1;
        summary
    }

    fn long_chain_summary() -> Stage2JirSummary {
        Stage2JirSummary {
            functions_seen: 1,
            statements_seen: 1,
            calls_seen: 1,
            records_required: 11,
            records_emitted: 11,
            functions_lowered: 1,
            errors: 0,
            status_flags: 7,
        }
    }

    fn expression_plan_summary() -> Stage2JirSummary {
        Stage2JirSummary {
            functions_seen: 1,
            statements_seen: 1,
            calls_seen: 1,
            records_required: 13,
            records_emitted: 13,
            functions_lowered: 1,
            errors: 0,
            status_flags: 7,
        }
    }

    fn streaming_expression_plan_summary(operator_count: u64) -> Stage2JirSummary {
        let record_count = 5 + operator_count * 2;
        Stage2JirSummary {
            functions_seen: 1,
            statements_seen: 1,
            calls_seen: 1,
            records_required: record_count,
            records_emitted: record_count,
            functions_lowered: 1,
            errors: 0,
            status_flags: 7,
        }
    }

    fn precedence_chain_records() -> Vec<Stage2JirRecord> {
        vec![
            record([1, 1, 2, 1], [0, 0, 0], 32, [0, 0]),
            record([2, 1, 2, 1], [0, 0, 0], 0, [3, 41]),
            record([3, 1, 0, 0], [0, 0, 0], 0, [19, 41]),
            record([4, 1, 2, 0], [0, 0, 0], 2, [28, 29]),
            record([4, 1, 2, 0], [0, 0, 1], 4, [32, 33]),
            record([4, 1, 2, 0], [0, 0, 2], 10, [36, 38]),
            record_with_operands([4, 4, 2, 0], [0, 0, 3], [1, 2], [32, 38]),
            record_with_operands([4, 2, 2, 0], [0, 0, 4], [0, 3], [28, 38]),
            record([5, 1, 2, 1], [0, 0, 4], 0, [21, 39]),
        ]
    }

    fn left_associative_chain_records() -> Vec<Stage2JirRecord> {
        vec![
            record([1, 1, 2, 1], [0, 0, 0], 32, [0, 0]),
            record([2, 1, 2, 1], [0, 0, 0], 0, [3, 41]),
            record([3, 1, 0, 0], [0, 0, 0], 0, [19, 41]),
            record([4, 1, 2, 0], [0, 0, 0], 50, [28, 30]),
            record([4, 1, 2, 0], [0, 0, 1], 6, [33, 34]),
            record([4, 1, 2, 0], [0, 0, 2], 2, [37, 38]),
            record_with_operands([4, 3, 2, 0], [0, 0, 3], [0, 1], [28, 34]),
            record_with_operands([4, 3, 2, 0], [0, 0, 4], [3, 2], [28, 38]),
            record([5, 1, 2, 1], [0, 0, 4], 0, [21, 39]),
        ]
    }

    fn grouped_right_chain_records() -> Vec<Stage2JirRecord> {
        vec![
            record([1, 1, 2, 1], [0, 0, 0], 32, [0, 0]),
            record([2, 1, 2, 1], [0, 0, 0], 0, [3, 42]),
            record([3, 1, 0, 0], [0, 0, 0], 0, [19, 42]),
            record([4, 1, 2, 0], [0, 0, 0], 6, [28, 29]),
            record([4, 1, 2, 0], [0, 0, 1], 3, [33, 34]),
            record([4, 1, 2, 0], [0, 0, 2], 4, [37, 38]),
            record_with_operands([4, 2, 2, 0], [0, 0, 3], [1, 2], [32, 39]),
            record_with_operands([4, 4, 2, 0], [0, 0, 4], [0, 3], [28, 39]),
            record([5, 1, 2, 1], [0, 0, 4], 0, [21, 40]),
        ]
    }

    fn grouped_left_chain_records() -> Vec<Stage2JirRecord> {
        vec![
            record([1, 1, 2, 1], [0, 0, 0], 32, [0, 0]),
            record([2, 1, 2, 1], [0, 0, 0], 0, [3, 43]),
            record([3, 1, 0, 0], [0, 0, 0], 0, [19, 43]),
            record([4, 1, 2, 0], [0, 0, 0], 50, [29, 31]),
            record([4, 1, 2, 0], [0, 0, 1], 6, [34, 35]),
            record([4, 1, 2, 0], [0, 0, 2], 2, [39, 40]),
            record_with_operands([4, 3, 2, 0], [0, 0, 3], [0, 1], [28, 36]),
            record_with_operands([4, 3, 2, 0], [0, 0, 4], [3, 2], [28, 40]),
            record([5, 1, 2, 1], [0, 0, 4], 0, [21, 41]),
        ]
    }

    fn grouped_single_binary_records() -> Vec<Stage2JirRecord> {
        vec![
            record([1, 1, 2, 1], [0, 0, 0], 32, [0, 0]),
            record([2, 1, 2, 1], [0, 0, 0], 0, [3, 39]),
            record([3, 1, 0, 0], [0, 0, 0], 0, [19, 39]),
            record([4, 1, 2, 0], [0, 0, 0], 40, [29, 31]),
            record([4, 1, 2, 0], [0, 0, 1], 2, [34, 35]),
            record_with_operands([4, 2, 2, 0], [0, 0, 2], [0, 1], [28, 36]),
            record([5, 1, 2, 1], [0, 0, 2], 0, [21, 37]),
        ]
    }

    fn long_precedence_chain_records() -> Vec<Stage2JirRecord> {
        vec![
            record([1, 1, 2, 1], [0, 0, 0], 32, [0, 0]),
            record([2, 1, 2, 1], [0, 0, 0], 0, [3, 45]),
            record([3, 1, 0, 0], [0, 0, 0], 0, [19, 45]),
            record([4, 1, 2, 0], [0, 0, 0], 2, [28, 29]),
            record([4, 1, 2, 0], [0, 0, 1], 4, [32, 33]),
            record([4, 1, 2, 0], [0, 0, 2], 10, [36, 38]),
            record([4, 1, 2, 0], [0, 0, 3], 0, [41, 42]),
            record_with_operands([4, 4, 2, 0], [0, 0, 4], [1, 2], [32, 38]),
            record_with_operands([4, 2, 2, 0], [0, 0, 5], [0, 4], [28, 38]),
            record_with_operands([4, 2, 2, 0], [0, 0, 6], [5, 3], [28, 42]),
            record([5, 1, 2, 1], [0, 0, 6], 0, [21, 43]),
        ]
    }

    fn long_multiply_chain_records() -> Vec<Stage2JirRecord> {
        vec![
            record([1, 1, 2, 1], [0, 0, 0], 32, [0, 0]),
            record([2, 1, 2, 1], [0, 0, 0], 0, [3, 44]),
            record([3, 1, 0, 0], [0, 0, 0], 0, [19, 44]),
            record([4, 1, 2, 0], [0, 0, 0], 2, [28, 29]),
            record([4, 1, 2, 0], [0, 0, 1], 3, [32, 33]),
            record([4, 1, 2, 0], [0, 0, 2], 4, [36, 37]),
            record([4, 1, 2, 0], [0, 0, 3], 5, [40, 41]),
            record_with_operands([4, 4, 2, 0], [0, 0, 4], [1, 2], [32, 37]),
            record_with_operands([4, 4, 2, 0], [0, 0, 5], [4, 3], [32, 41]),
            record_with_operands([4, 2, 2, 0], [0, 0, 6], [0, 5], [28, 41]),
            record([5, 1, 2, 1], [0, 0, 6], 0, [21, 42]),
        ]
    }

    fn long_left_associative_chain_records() -> Vec<Stage2JirRecord> {
        vec![
            record([1, 1, 2, 1], [0, 0, 0], 32, [0, 0]),
            record([2, 1, 2, 1], [0, 0, 0], 0, [3, 44]),
            record([3, 1, 0, 0], [0, 0, 0], 0, [19, 44]),
            record([4, 1, 2, 0], [0, 0, 0], 6, [28, 29]),
            record([4, 1, 2, 0], [0, 0, 1], 7, [32, 33]),
            record([4, 1, 2, 0], [0, 0, 2], 2, [36, 37]),
            record([4, 1, 2, 0], [0, 0, 3], 2, [40, 41]),
            record_with_operands([4, 4, 2, 0], [0, 0, 4], [0, 1], [28, 33]),
            record_with_operands([4, 2, 2, 0], [0, 0, 5], [4, 2], [28, 37]),
            record_with_operands([4, 3, 2, 0], [0, 0, 6], [5, 3], [28, 41]),
            record([5, 1, 2, 1], [0, 0, 6], 0, [21, 42]),
        ]
    }

    fn expression_plan_records() -> Vec<Stage2JirRecord> {
        vec![
            record([1, 1, 2, 1], [0, 0, 0], 32, [0, 0]),
            record([2, 1, 2, 1], [0, 0, 0], 0, [3, 49]),
            record([3, 1, 0, 0], [0, 0, 0], 0, [19, 49]),
            record([4, 1, 2, 0], [0, 0, 0], 2, [28, 29]),
            record([4, 1, 2, 0], [0, 0, 1], 4, [32, 33]),
            record([4, 1, 2, 0], [0, 0, 2], 10, [36, 38]),
            record([4, 1, 2, 0], [0, 0, 3], 5, [41, 42]),
            record([4, 1, 2, 0], [0, 0, 4], 5, [45, 46]),
            record_with_operands([4, 4, 2, 0], [0, 0, 5], [1, 2], [32, 38]),
            record_with_operands([4, 2, 2, 0], [0, 0, 6], [0, 5], [28, 38]),
            record_with_operands([4, 2, 2, 0], [0, 0, 7], [6, 3], [28, 42]),
            record_with_operands([4, 3, 2, 0], [0, 0, 8], [7, 4], [28, 46]),
            record([5, 1, 2, 1], [0, 0, 8], 0, [21, 47]),
        ]
    }

    fn grouped_long_chain_records() -> Vec<Stage2JirRecord> {
        vec![
            record([1, 1, 2, 1], [0, 0, 0], 32, [0, 0]),
            record([2, 1, 2, 1], [0, 0, 0], 0, [3, 47]),
            record([3, 1, 0, 0], [0, 0, 0], 0, [19, 47]),
            record([4, 1, 2, 0], [0, 0, 0], 50, [28, 30]),
            record([4, 1, 2, 0], [0, 0, 1], 2, [34, 35]),
            record([4, 1, 2, 0], [0, 0, 2], 2, [38, 39]),
            record([4, 1, 2, 0], [0, 0, 3], 2, [43, 44]),
            record_with_operands([4, 2, 2, 0], [0, 0, 4], [1, 2], [33, 40]),
            record_with_operands([4, 4, 2, 0], [0, 0, 5], [4, 3], [33, 44]),
            record_with_operands([4, 3, 2, 0], [0, 0, 6], [0, 5], [28, 44]),
            record([5, 1, 2, 1], [0, 0, 6], 0, [21, 45]),
        ]
    }

    fn grouped_expression_plan_records() -> Vec<Stage2JirRecord> {
        vec![
            record([1, 1, 2, 1], [0, 0, 0], 32, [0, 0]),
            record([2, 1, 2, 1], [0, 0, 0], 0, [3, 51]),
            record([3, 1, 0, 0], [0, 0, 0], 0, [19, 51]),
            record([4, 1, 2, 0], [0, 0, 0], 50, [28, 30]),
            record([4, 1, 2, 0], [0, 0, 1], 2, [34, 35]),
            record([4, 1, 2, 0], [0, 0, 2], 2, [38, 39]),
            record([4, 1, 2, 0], [0, 0, 3], 2, [43, 44]),
            record([4, 1, 2, 0], [0, 0, 4], 0, [47, 48]),
            record_with_operands([4, 2, 2, 0], [0, 0, 5], [1, 2], [33, 40]),
            record_with_operands([4, 4, 2, 0], [0, 0, 6], [5, 3], [33, 44]),
            record_with_operands([4, 3, 2, 0], [0, 0, 7], [0, 6], [28, 44]),
            record_with_operands([4, 2, 2, 0], [0, 0, 8], [7, 4], [28, 48]),
            record([5, 1, 2, 1], [0, 0, 8], 0, [21, 49]),
        ]
    }

    fn streaming_expression_plan_records() -> Vec<Stage2JirRecord> {
        vec![
            record([1, 1, 2, 1], [0, 0, 0], 32, [0, 0]),
            record([2, 1, 2, 1], [0, 0, 0], 0, [3, 61]),
            record([3, 1, 0, 0], [0, 0, 0], 0, [19, 61]),
            record([4, 1, 2, 0], [0, 0, 0], 2, [28, 29]),
            record([4, 1, 2, 0], [0, 0, 1], 4, [32, 33]),
            record([4, 1, 2, 0], [0, 0, 2], 10, [36, 38]),
            record([4, 1, 2, 0], [0, 0, 3], 5, [41, 42]),
            record([4, 1, 2, 0], [0, 0, 4], 5, [45, 46]),
            record([4, 1, 2, 0], [0, 0, 5], 0, [49, 50]),
            record([4, 1, 2, 0], [0, 0, 6], 0, [53, 54]),
            record([4, 1, 2, 0], [0, 0, 7], 0, [57, 58]),
            record_with_operands([4, 4, 2, 0], [0, 0, 8], [1, 2], [32, 38]),
            record_with_operands([4, 2, 2, 0], [0, 0, 9], [0, 8], [28, 38]),
            record_with_operands([4, 2, 2, 0], [0, 0, 10], [9, 3], [28, 42]),
            record_with_operands([4, 3, 2, 0], [0, 0, 11], [10, 4], [28, 46]),
            record_with_operands([4, 2, 2, 0], [0, 0, 12], [11, 5], [28, 50]),
            record_with_operands([4, 2, 2, 0], [0, 0, 13], [12, 6], [28, 54]),
            record_with_operands([4, 2, 2, 0], [0, 0, 14], [13, 7], [28, 58]),
            record([5, 1, 2, 1], [0, 0, 14], 0, [21, 59]),
        ]
    }

    fn grouped_streaming_expression_plan_records() -> Vec<Stage2JirRecord> {
        vec![
            record([1, 1, 2, 1], [0, 0, 0], 32, [0, 0]),
            record([2, 1, 2, 1], [0, 0, 0], 0, [3, 59]),
            record([3, 1, 0, 0], [0, 0, 0], 0, [19, 59]),
            record([4, 1, 2, 0], [0, 0, 0], 50, [28, 30]),
            record([4, 1, 2, 0], [0, 0, 1], 2, [34, 35]),
            record([4, 1, 2, 0], [0, 0, 2], 2, [38, 39]),
            record([4, 1, 2, 0], [0, 0, 3], 2, [43, 44]),
            record([4, 1, 2, 0], [0, 0, 4], 0, [47, 48]),
            record([4, 1, 2, 0], [0, 0, 5], 0, [51, 52]),
            record([4, 1, 2, 0], [0, 0, 6], 0, [55, 56]),
            record_with_operands([4, 2, 2, 0], [0, 0, 7], [1, 2], [33, 40]),
            record_with_operands([4, 4, 2, 0], [0, 0, 8], [7, 3], [33, 44]),
            record_with_operands([4, 3, 2, 0], [0, 0, 9], [0, 8], [28, 44]),
            record_with_operands([4, 2, 2, 0], [0, 0, 10], [9, 4], [28, 48]),
            record_with_operands([4, 2, 2, 0], [0, 0, 11], [10, 5], [28, 52]),
            record_with_operands([4, 2, 2, 0], [0, 0, 12], [11, 6], [28, 56]),
            record([5, 1, 2, 1], [0, 0, 12], 0, [21, 57]),
        ]
    }

    fn multi_group_streaming_expression_plan_records() -> Vec<Stage2JirRecord> {
        vec![
            record([1, 1, 2, 1], [0, 0, 0], 32, [0, 0]),
            record([2, 1, 2, 1], [0, 0, 0], 0, [3, 61]),
            record([3, 1, 0, 0], [0, 0, 0], 0, [19, 61]),
            record([4, 1, 2, 0], [0, 0, 0], 20, [29, 31]),
            record([4, 1, 2, 0], [0, 0, 1], 1, [34, 35]),
            record([4, 1, 2, 0], [0, 0, 2], 2, [39, 40]),
            record([4, 1, 2, 0], [0, 0, 3], 3, [44, 45]),
            record([4, 1, 2, 0], [0, 0, 4], 3, [48, 49]),
            record([4, 1, 2, 0], [0, 0, 5], 0, [53, 54]),
            record([4, 1, 2, 0], [0, 0, 6], 0, [57, 58]),
            record_with_operands([4, 2, 2, 0], [0, 0, 7], [0, 1], [28, 36]),
            record_with_operands([4, 3, 2, 0], [0, 0, 8], [3, 4], [43, 50]),
            record_with_operands([4, 4, 2, 0], [0, 0, 9], [7, 2], [28, 40]),
            record_with_operands([4, 2, 2, 0], [0, 0, 10], [9, 8], [28, 50]),
            record_with_operands([4, 2, 2, 0], [0, 0, 11], [10, 5], [28, 54]),
            record_with_operands([4, 2, 2, 0], [0, 0, 12], [11, 6], [28, 58]),
            record([5, 1, 2, 1], [0, 0, 12], 0, [21, 59]),
        ]
    }

    fn nested_streaming_expression_plan_records() -> Vec<Stage2JirRecord> {
        vec![
            record([1, 1, 2, 1], [0, 0, 0], 32, [0, 0]),
            record([2, 1, 2, 1], [0, 0, 0], 0, [3, 57]),
            record([3, 1, 0, 0], [0, 0, 0], 0, [19, 57]),
            record([4, 1, 2, 0], [0, 0, 0], 20, [30, 32]),
            record([4, 1, 2, 0], [0, 0, 1], 1, [35, 36]),
            record([4, 1, 2, 0], [0, 0, 2], 2, [41, 42]),
            record([4, 1, 2, 0], [0, 0, 3], 0, [45, 46]),
            record([4, 1, 2, 0], [0, 0, 4], 0, [49, 50]),
            record([4, 1, 2, 0], [0, 0, 5], 0, [53, 54]),
            record_with_operands([4, 2, 2, 0], [0, 0, 6], [0, 1], [28, 38]),
            record_with_operands([4, 4, 2, 0], [0, 0, 7], [6, 2], [28, 42]),
            record_with_operands([4, 2, 2, 0], [0, 0, 8], [7, 3], [28, 46]),
            record_with_operands([4, 2, 2, 0], [0, 0, 9], [8, 4], [28, 50]),
            record_with_operands([4, 2, 2, 0], [0, 0, 10], [9, 5], [28, 54]),
            record([5, 1, 2, 1], [0, 0, 10], 0, [21, 55]),
        ]
    }

    fn parameter_binary_summary() -> Stage2JirSummary {
        Stage2JirSummary {
            functions_seen: 1,
            statements_seen: 1,
            calls_seen: 1,
            records_required: 6,
            records_emitted: 6,
            functions_lowered: 1,
            errors: 0,
            status_flags: 7,
        }
    }

    fn parameter_binary_records() -> Vec<Stage2JirRecord> {
        vec![
            record([1, 1, 2, 1], [0, 0, 0], 32, [0, 0]),
            record([2, 1, 2, 1], [0, 0, 0], 1, [3, 44]),
            record([3, 1, 0, 0], [0, 0, 0], 0, [27, 44]),
            record([4, 1, 2, 0], [0, 0, 1], 1, [40, 41]),
            record_with_operands([4, 2, 2, 0], [0, 0, 2], [0, 1], [36, 41]),
            record([5, 1, 2, 1], [0, 0, 2], 0, [29, 42]),
        ]
    }

    fn parameter_pair_summary() -> Stage2JirSummary {
        Stage2JirSummary {
            functions_seen: 1,
            statements_seen: 1,
            calls_seen: 1,
            records_required: 5,
            records_emitted: 5,
            functions_lowered: 1,
            errors: 0,
            status_flags: 7,
        }
    }

    fn parameter_pair_records() -> Vec<Stage2JirRecord> {
        vec![
            record([1, 1, 2, 1], [0, 0, 0], 32, [0, 0]),
            record([2, 1, 2, 1], [0, 0, 0], 2, [3, 54]),
            record([3, 1, 0, 0], [0, 0, 0], 0, [37, 54]),
            record_with_operands([4, 2, 2, 0], [0, 0, 2], [0, 1], [46, 51]),
            record([5, 1, 2, 1], [0, 0, 2], 0, [39, 52]),
        ]
    }

    fn capture_bytes() -> Vec<u8> {
        let summary = summary();
        let records = records();
        let mut bytes = b"JST2CAP1".to_vec();
        bytes.extend_from_slice(&(SOURCE.len() as u64).to_le_bytes());
        bytes.extend_from_slice(SOURCE.as_bytes());
        for value in [
            summary.functions_seen,
            summary.statements_seen,
            summary.calls_seen,
            summary.records_required,
            summary.records_emitted,
            summary.functions_lowered,
            summary.errors,
            summary.status_flags,
            records.len() as u64,
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for record in records {
            bytes.extend_from_slice(&[record.kind, record.opcode, record.type_kind, record.flags]);
            for value in [
                record.function_index,
                record.block_index,
                record.value_index,
                record.operand_a,
                record.operand_b,
                record.source_start,
                record.source_end,
            ] {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        bytes
    }

    fn source_id_for(source: &str) -> jadren_source::SourceId {
        SourceManager::new()
            .add("stage2.jdn", source)
            .expect("source ID should fit")
    }

    fn source_id() -> jadren_source::SourceId {
        source_id_for(SOURCE)
    }

    #[test]
    fn imports_complete_stream_into_verified_canonical_jir() {
        let module = import_stage2_jir(SOURCE, source_id(), summary(), &records())
            .expect("complete stage-2 stream should import");

        assert!(verify(&module).is_empty());
        assert_eq!(module.functions[0].name, "first");
        assert_eq!(module.functions[1].name, "main");
        assert_eq!(
            module.to_text(),
            "jir 0.1\ntype %t0 = i32\n\nfn export @f0 \"first\"() -> %t0 {\n  ^bb0:\n    %v0: %t0 = const 7\n    return %v0\n}\n\nfn export @f1 \"main\"() -> %t0 {\n  ^bb0:\n    %v0: %t0 = const 42\n    return %v0\n}\n"
        );
    }

    #[test]
    fn imports_literal_binary_family_as_three_dense_jir_values() {
        for (source, opcode, expected_op) in [
            (
                ADD_SOURCE,
                STAGE2_JIR_INSTRUCTION_ADD,
                jadren_jir::BinaryOp::Add,
            ),
            (
                SUBTRACT_SOURCE,
                STAGE2_JIR_INSTRUCTION_SUBTRACT,
                jadren_jir::BinaryOp::Subtract,
            ),
            (
                MULTIPLY_SOURCE,
                STAGE2_JIR_INSTRUCTION_MULTIPLY,
                jadren_jir::BinaryOp::Multiply,
            ),
        ] {
            let module = import_stage2_jir(
                source,
                source_id_for(source),
                binary_summary(),
                &binary_records(opcode),
            )
            .expect("bounded binary operation should import");

            assert!(verify(&module).is_empty());
            let instructions = &module.functions[0].blocks[0].instructions;
            assert_eq!(instructions.len(), 3);
            assert!(matches!(
                instructions[2].kind,
                jadren_jir::InstructionKind::Binary { op, left, right }
                    if op == expected_op
                        && left == jadren_jir::ValueId::new(0)
                        && right == jadren_jir::ValueId::new(1)
            ));
            assert!(matches!(
                module.functions[0].blocks[0].terminator,
                jadren_jir::Terminator::Return { value: Some(value) }
                    if value == jadren_jir::ValueId::new(2)
            ));
        }
    }

    #[test]
    fn rejects_corrupted_binary_opcode_operands_and_operator_source() {
        let mut bad_opcode = binary_records(STAGE2_JIR_INSTRUCTION_ADD);
        bad_opcode[5].opcode = 99;
        assert!(matches!(
            import_stage2_jir(
                ADD_SOURCE,
                source_id_for(ADD_SOURCE),
                binary_summary(),
                &bad_opcode,
            ),
            Err(Stage2ImportError::InvalidRecord { index: 5, .. })
        ));

        let mut bad_operand = binary_records(STAGE2_JIR_INSTRUCTION_ADD);
        bad_operand[5].operand_b = 0;
        assert!(matches!(
            import_stage2_jir(
                ADD_SOURCE,
                source_id_for(ADD_SOURCE),
                binary_summary(),
                &bad_operand,
            ),
            Err(Stage2ImportError::InvalidRecord { index: 5, .. })
        ));

        assert!(matches!(
            import_stage2_jir(
                SUBTRACT_SOURCE,
                source_id_for(SUBTRACT_SOURCE),
                binary_summary(),
                &binary_records(STAGE2_JIR_INSTRUCTION_ADD),
            ),
            Err(Stage2ImportError::InvalidRecord { index: 5, .. })
        ));
    }

    #[test]
    fn imports_two_operator_chains_with_precedence_and_left_associativity() {
        let precedence_module = import_stage2_jir(
            PRECEDENCE_CHAIN_SOURCE,
            source_id_for(PRECEDENCE_CHAIN_SOURCE),
            chain_summary(),
            &precedence_chain_records(),
        )
        .expect("multiplication-precedence chain should import");
        let precedence_instructions = &precedence_module.functions[0].blocks[0].instructions;
        assert!(verify(&precedence_module).is_empty());
        assert_eq!(precedence_instructions.len(), 5);
        assert!(matches!(
            precedence_instructions[3].kind,
            jadren_jir::InstructionKind::Binary {
                op: jadren_jir::BinaryOp::Multiply,
                left,
                right
            } if left == jadren_jir::ValueId::new(1) && right == jadren_jir::ValueId::new(2)
        ));
        assert!(matches!(
            precedence_instructions[4].kind,
            jadren_jir::InstructionKind::Binary {
                op: jadren_jir::BinaryOp::Add,
                left,
                right
            } if left == jadren_jir::ValueId::new(0) && right == jadren_jir::ValueId::new(3)
        ));

        let left_associative_module = import_stage2_jir(
            LEFT_ASSOCIATIVE_CHAIN_SOURCE,
            source_id_for(LEFT_ASSOCIATIVE_CHAIN_SOURCE),
            chain_summary(),
            &left_associative_chain_records(),
        )
        .expect("left-associative subtraction chain should import");
        let left_associative_instructions =
            &left_associative_module.functions[0].blocks[0].instructions;
        assert!(verify(&left_associative_module).is_empty());
        assert_eq!(left_associative_instructions.len(), 5);
        assert!(matches!(
            left_associative_instructions[3].kind,
            jadren_jir::InstructionKind::Binary {
                op: jadren_jir::BinaryOp::Subtract,
                left,
                right
            } if left == jadren_jir::ValueId::new(0) && right == jadren_jir::ValueId::new(1)
        ));
        assert!(matches!(
            left_associative_instructions[4].kind,
            jadren_jir::InstructionKind::Binary {
                op: jadren_jir::BinaryOp::Subtract,
                left,
                right
            } if left == jadren_jir::ValueId::new(3) && right == jadren_jir::ValueId::new(2)
        ));
    }

    #[test]
    fn rejects_binary_chains_that_violate_precedence_or_associativity() {
        let mut wrong_precedence = precedence_chain_records();
        wrong_precedence[6] = record_with_operands([4, 2, 2, 0], [0, 0, 3], [0, 1], [28, 33]);
        wrong_precedence[7] = record_with_operands([4, 4, 2, 0], [0, 0, 4], [3, 2], [28, 38]);
        assert!(matches!(
            import_stage2_jir(
                PRECEDENCE_CHAIN_SOURCE,
                source_id_for(PRECEDENCE_CHAIN_SOURCE),
                chain_summary(),
                &wrong_precedence,
            ),
            Err(Stage2ImportError::InvalidRecord { index: 6, .. })
        ));

        let mut wrong_associativity = left_associative_chain_records();
        wrong_associativity[6] = record_with_operands([4, 3, 2, 0], [0, 0, 3], [1, 2], [33, 38]);
        wrong_associativity[7] = record_with_operands([4, 3, 2, 0], [0, 0, 4], [0, 3], [28, 38]);
        assert!(matches!(
            import_stage2_jir(
                LEFT_ASSOCIATIVE_CHAIN_SOURCE,
                source_id_for(LEFT_ASSOCIATIVE_CHAIN_SOURCE),
                chain_summary(),
                &wrong_associativity,
            ),
            Err(Stage2ImportError::InvalidRecord { index: 6, .. })
        ));
    }

    #[test]
    fn imports_three_operator_chains_with_precedence_and_left_associativity() {
        for (source, records) in [
            (
                LONG_PRECEDENCE_CHAIN_SOURCE,
                long_precedence_chain_records(),
            ),
            (LONG_MULTIPLY_CHAIN_SOURCE, long_multiply_chain_records()),
            (
                LONG_LEFT_ASSOCIATIVE_CHAIN_SOURCE,
                long_left_associative_chain_records(),
            ),
        ] {
            let module = import_stage2_jir(
                source,
                source_id_for(source),
                long_chain_summary(),
                &records,
            )
            .expect("three-operator chain should import");
            assert!(verify(&module).is_empty());
            assert_eq!(module.functions[0].blocks[0].instructions.len(), 7);
        }
    }

    #[test]
    fn rejects_three_operator_chain_with_a_valid_but_wrong_reduction_tree() {
        let mut wrong_reduction = long_precedence_chain_records();
        wrong_reduction[7] = record_with_operands([4, 2, 2, 0], [0, 0, 4], [2, 3], [36, 42]);
        wrong_reduction[8] = record_with_operands([4, 4, 2, 0], [0, 0, 5], [1, 4], [32, 42]);
        wrong_reduction[9] = record_with_operands([4, 2, 2, 0], [0, 0, 6], [0, 5], [28, 42]);
        assert!(matches!(
            import_stage2_jir(
                LONG_PRECEDENCE_CHAIN_SOURCE,
                source_id_for(LONG_PRECEDENCE_CHAIN_SOURCE),
                long_chain_summary(),
                &wrong_reduction,
            ),
            Err(Stage2ImportError::InvalidRecord { index: 7, .. })
        ));
    }

    #[test]
    fn imports_four_operator_expression_plan_in_postorder() {
        let module = import_stage2_jir(
            EXPRESSION_PLAN_SOURCE,
            source_id_for(EXPRESSION_PLAN_SOURCE),
            expression_plan_summary(),
            &expression_plan_records(),
        )
        .expect("four-operator expression plan should import");
        assert!(verify(&module).is_empty());
        let instructions = &module.functions[0].blocks[0].instructions;
        assert_eq!(instructions.len(), 9);
        assert!(matches!(
            instructions[5].kind,
            jadren_jir::InstructionKind::Binary {
                op: jadren_jir::BinaryOp::Multiply,
                left,
                right
            } if left == jadren_jir::ValueId::new(1) && right == jadren_jir::ValueId::new(2)
        ));
        assert!(matches!(
            module.functions[0].blocks[0].terminator,
            jadren_jir::Terminator::Return { value: Some(value) }
                if value == jadren_jir::ValueId::new(8)
        ));
    }

    #[test]
    fn rejects_four_operator_plan_with_individually_valid_wrong_tree() {
        let mut wrong = expression_plan_records();
        wrong[8] = record_with_operands([4, 2, 2, 0], [0, 0, 5], [0, 1], [28, 33]);
        wrong[9] = record_with_operands([4, 2, 2, 0], [0, 0, 6], [2, 3], [36, 42]);
        wrong[10] = record_with_operands([4, 4, 2, 0], [0, 0, 7], [5, 6], [28, 42]);
        wrong[11] = record_with_operands([4, 3, 2, 0], [0, 0, 8], [7, 4], [28, 46]);
        assert!(matches!(
            import_stage2_jir(
                EXPRESSION_PLAN_SOURCE,
                source_id_for(EXPRESSION_PLAN_SOURCE),
                expression_plan_summary(),
                &wrong,
            ),
            Err(Stage2ImportError::InvalidRecord { index: 8, .. })
        ));
    }

    #[test]
    fn imports_grouped_three_and_four_operator_expression_plans() {
        for (source, summary, records, expected_instruction_count, grouped_index) in [
            (
                GROUPED_LONG_CHAIN_SOURCE,
                long_chain_summary(),
                grouped_long_chain_records(),
                7usize,
                4usize,
            ),
            (
                GROUPED_EXPRESSION_PLAN_SOURCE,
                expression_plan_summary(),
                grouped_expression_plan_records(),
                9usize,
                5usize,
            ),
        ] {
            let module = import_stage2_jir(source, source_id_for(source), summary, &records)
                .expect("grouped long expression plan should import");
            assert!(verify(&module).is_empty());
            let instructions = &module.functions[0].blocks[0].instructions;
            assert_eq!(instructions.len(), expected_instruction_count);
            let grouped = &instructions[grouped_index];
            assert!(matches!(
                grouped.kind,
                jadren_jir::InstructionKind::Binary {
                    op: jadren_jir::BinaryOp::Add,
                    left,
                    right,
                } if left == jadren_jir::ValueId::new(1)
                    && right == jadren_jir::ValueId::new(2)
            ));
            assert!(
                grouped
                    .span
                    .is_some_and(|span| { source[span.start..span.end].trim() == "(2 + 2)" })
            );
        }
    }

    #[test]
    fn imports_ungrouped_and_grouped_streaming_expression_plans() {
        for (source, operator_count, records, expected_instructions) in [
            (
                STREAMING_EXPRESSION_PLAN_SOURCE,
                7u64,
                streaming_expression_plan_records(),
                15usize,
            ),
            (
                GROUPED_STREAMING_EXPRESSION_PLAN_SOURCE,
                6u64,
                grouped_streaming_expression_plan_records(),
                13usize,
            ),
        ] {
            let module = import_stage2_jir(
                source,
                source_id_for(source),
                streaming_expression_plan_summary(operator_count),
                &records,
            )
            .expect("streaming expression plan should import");
            assert!(verify(&module).is_empty());
            assert_eq!(
                module.functions[0].blocks[0].instructions.len(),
                expected_instructions
            );
        }
    }

    #[test]
    fn rejects_streaming_plan_with_locally_valid_wrong_global_tree() {
        let mut wrong = streaming_expression_plan_records();
        wrong[11] = record_with_operands([4, 2, 2, 0], [0, 0, 8], [0, 1], [28, 33]);
        wrong[12] = record_with_operands([4, 4, 2, 0], [0, 0, 9], [8, 2], [28, 38]);
        assert!(matches!(
            import_stage2_jir(
                STREAMING_EXPRESSION_PLAN_SOURCE,
                source_id_for(STREAMING_EXPRESSION_PLAN_SOURCE),
                streaming_expression_plan_summary(7),
                &wrong,
            ),
            Err(Stage2ImportError::InvalidRecord { index: 11, .. })
        ));
    }

    #[test]
    fn imports_two_disjoint_streaming_expression_groups() {
        let module = import_stage2_jir(
            MULTI_GROUP_STREAMING_EXPRESSION_PLAN_SOURCE,
            source_id_for(MULTI_GROUP_STREAMING_EXPRESSION_PLAN_SOURCE),
            with_leading_group(streaming_expression_plan_summary(6)),
            &multi_group_streaming_expression_plan_records(),
        )
        .expect("two disjoint streaming expression groups should import");
        assert!(verify(&module).is_empty());
        assert_eq!(module.functions[0].blocks[0].instructions.len(), 13);
    }

    #[test]
    fn imports_one_nested_streaming_expression_group() {
        let module = import_stage2_jir(
            NESTED_STREAMING_EXPRESSION_PLAN_SOURCE,
            source_id_for(NESTED_STREAMING_EXPRESSION_PLAN_SOURCE),
            with_leading_group(streaming_expression_plan_summary(5)),
            &nested_streaming_expression_plan_records(),
        )
        .expect("one nested streaming expression group should import");
        assert!(verify(&module).is_empty());
        assert_eq!(module.functions[0].blocks[0].instructions.len(), 11);
    }

    #[test]
    fn imports_one_typed_parameter_binary_expression() {
        let module = import_stage2_jir(
            PARAMETER_BINARY_SOURCE,
            source_id_for(PARAMETER_BINARY_SOURCE),
            parameter_binary_summary(),
            &parameter_binary_records(),
        )
        .expect("one typed parameter binary expression should import");
        assert!(verify(&module).is_empty());
        assert_eq!(module.functions[0].parameters.len(), 1);
        assert_eq!(module.functions[0].parameters[0].value.index(), 0);
        assert_eq!(module.functions[0].parameters[0].name.as_deref(), Some("x"));
        assert_eq!(module.functions[0].blocks[0].instructions.len(), 2);
        assert!(matches!(
            module.functions[0].blocks[0].instructions[1].kind,
            jadren_jir::InstructionKind::Binary {
                op: jadren_jir::BinaryOp::Add,
                left,
                right,
            } if left == jadren_jir::ValueId::new(0)
                && right == jadren_jir::ValueId::new(1)
        ));
    }

    #[test]
    fn imports_two_typed_parameter_binary_expression() {
        let module = import_stage2_jir(
            PARAMETER_PAIR_SOURCE,
            source_id_for(PARAMETER_PAIR_SOURCE),
            parameter_pair_summary(),
            &parameter_pair_records(),
        )
        .expect("two typed parameter binary expression should import");
        assert!(verify(&module).is_empty());
        assert_eq!(module.functions[0].parameters.len(), 2);
        assert_eq!(module.functions[0].parameters[0].value.index(), 0);
        assert_eq!(module.functions[0].parameters[0].name.as_deref(), Some("x"));
        assert_eq!(module.functions[0].parameters[1].value.index(), 1);
        assert_eq!(module.functions[0].parameters[1].name.as_deref(), Some("y"));
        assert!(matches!(
            module.functions[0].blocks[0].instructions[0].kind,
            jadren_jir::InstructionKind::Binary {
                op: jadren_jir::BinaryOp::Add,
                left,
                right,
            } if left == jadren_jir::ValueId::new(0)
                && right == jadren_jir::ValueId::new(1)
        ));
    }

    #[test]
    fn rejects_parameter_function_with_two_parameters() {
        let mut records = parameter_binary_records();
        records[1].operand_a = 2;
        assert!(matches!(
            import_stage2_jir(
                PARAMETER_BINARY_SOURCE,
                source_id_for(PARAMETER_BINARY_SOURCE),
                parameter_binary_summary(),
                &records,
            ),
            Err(Stage2ImportError::InvalidRecord { index: 1, .. })
        ));
    }

    #[test]
    fn rejects_nested_streaming_expression_with_inner_only_span() {
        let mut wrong = nested_streaming_expression_plan_records();
        wrong[9].source_start = 29;
        wrong[9].source_end = 37;
        assert!(matches!(
            import_stage2_jir(
                NESTED_STREAMING_EXPRESSION_PLAN_SOURCE,
                source_id_for(NESTED_STREAMING_EXPRESSION_PLAN_SOURCE),
                with_leading_group(streaming_expression_plan_summary(5)),
                &wrong,
            ),
            Err(Stage2ImportError::InvalidRecord { .. })
        ));
    }

    #[test]
    fn rejects_two_streaming_groups_emitted_out_of_source_order() {
        let mut wrong = multi_group_streaming_expression_plan_records();
        wrong[10] = record_with_operands([4, 3, 2, 0], [0, 0, 7], [3, 4], [43, 50]);
        wrong[11] = record_with_operands([4, 2, 2, 0], [0, 0, 8], [0, 1], [28, 36]);
        assert!(matches!(
            import_stage2_jir(
                MULTI_GROUP_STREAMING_EXPRESSION_PLAN_SOURCE,
                source_id_for(MULTI_GROUP_STREAMING_EXPRESSION_PLAN_SOURCE),
                with_leading_group(streaming_expression_plan_summary(6)),
                &wrong,
            ),
            Err(Stage2ImportError::InvalidRecord { .. })
        ));
    }

    #[test]
    fn rejects_grouped_long_plan_when_delimiter_span_is_lost() {
        let mut wrong = grouped_expression_plan_records();
        wrong[8].source_start = 34;
        wrong[8].source_end = 39;
        assert!(matches!(
            import_stage2_jir(
                GROUPED_EXPRESSION_PLAN_SOURCE,
                source_id_for(GROUPED_EXPRESSION_PLAN_SOURCE),
                expression_plan_summary(),
                &wrong,
            ),
            Err(Stage2ImportError::InvalidRecord { index: 9, .. })
        ));
    }

    #[test]
    fn imports_explicit_left_right_and_single_binary_groups() {
        let grouped_right = import_stage2_jir(
            GROUPED_RIGHT_CHAIN_SOURCE,
            source_id_for(GROUPED_RIGHT_CHAIN_SOURCE),
            chain_summary(),
            &grouped_right_chain_records(),
        )
        .expect("right-grouped chain should import");
        let right_instructions = &grouped_right.functions[0].blocks[0].instructions;
        assert!(verify(&grouped_right).is_empty());
        assert!(matches!(
            right_instructions[3].kind,
            jadren_jir::InstructionKind::Binary {
                op: jadren_jir::BinaryOp::Add,
                left,
                right
            } if left == jadren_jir::ValueId::new(1) && right == jadren_jir::ValueId::new(2)
        ));
        assert!(matches!(
            right_instructions[4].kind,
            jadren_jir::InstructionKind::Binary {
                op: jadren_jir::BinaryOp::Multiply,
                left,
                right
            } if left == jadren_jir::ValueId::new(0) && right == jadren_jir::ValueId::new(3)
        ));

        let grouped_left = import_stage2_jir(
            GROUPED_LEFT_CHAIN_SOURCE,
            source_id_for(GROUPED_LEFT_CHAIN_SOURCE),
            with_leading_group(chain_summary()),
            &grouped_left_chain_records(),
        )
        .expect("left-grouped chain should import");
        let left_instructions = &grouped_left.functions[0].blocks[0].instructions;
        assert!(verify(&grouped_left).is_empty());
        assert!(matches!(
            left_instructions[3].kind,
            jadren_jir::InstructionKind::Binary {
                op: jadren_jir::BinaryOp::Subtract,
                left,
                right
            } if left == jadren_jir::ValueId::new(0) && right == jadren_jir::ValueId::new(1)
        ));

        let grouped_single = import_stage2_jir(
            GROUPED_SINGLE_BINARY_SOURCE,
            source_id_for(GROUPED_SINGLE_BINARY_SOURCE),
            with_leading_group(binary_summary()),
            &grouped_single_binary_records(),
        )
        .expect("whole grouped single binary should import");
        assert!(verify(&grouped_single).is_empty());
        assert_eq!(grouped_single.functions[0].blocks[0].instructions.len(), 3);
    }

    #[test]
    fn rejects_grouped_streams_with_lost_delimiters_or_wrong_reduction() {
        let mut lost_open = grouped_right_chain_records();
        lost_open[6].source_start = 33;
        assert!(matches!(
            import_stage2_jir(
                GROUPED_RIGHT_CHAIN_SOURCE,
                source_id_for(GROUPED_RIGHT_CHAIN_SOURCE),
                chain_summary(),
                &lost_open,
            ),
            Err(Stage2ImportError::InvalidRecord { index: 6, .. })
        ));

        let mut wrong_reduction = grouped_right_chain_records();
        wrong_reduction[6] = record_with_operands([4, 4, 2, 0], [0, 0, 3], [0, 1], [28, 34]);
        wrong_reduction[7] = record_with_operands([4, 2, 2, 0], [0, 0, 4], [3, 2], [28, 38]);
        assert!(matches!(
            import_stage2_jir(
                GROUPED_RIGHT_CHAIN_SOURCE,
                source_id_for(GROUPED_RIGHT_CHAIN_SOURCE),
                chain_summary(),
                &wrong_reduction,
            ),
            Err(Stage2ImportError::InvalidRecord { .. })
        ));

        let mut lost_close = grouped_single_binary_records();
        lost_close[5].source_end = 35;
        assert!(matches!(
            import_stage2_jir(
                GROUPED_SINGLE_BINARY_SOURCE,
                source_id_for(GROUPED_SINGLE_BINARY_SOURCE),
                with_leading_group(binary_summary()),
                &lost_close,
            ),
            Err(Stage2ImportError::InvalidRecord { index: 5, .. })
        ));

        assert!(matches!(
            import_stage2_jir(
                GROUPED_LEFT_CHAIN_SOURCE,
                source_id_for(GROUPED_LEFT_CHAIN_SOURCE),
                chain_summary(),
                &grouped_left_chain_records(),
            ),
            Err(Stage2ImportError::InvalidSummary {
                field: "calls_seen",
                ..
            })
        ));
    }

    #[test]
    fn decodes_padding_free_capture_without_trusting_abi_padding() {
        let capture = decode_stage2_capture(&capture_bytes()).expect("canonical capture");
        assert_eq!(capture.source, SOURCE);
        assert_eq!(capture.summary, summary());
        assert_eq!(capture.records, records());
    }

    #[test]
    fn rejects_invalid_magic_and_impossible_capture_record_count() {
        let mut invalid_magic = capture_bytes();
        invalid_magic[0] = b'X';
        assert!(decode_stage2_capture(&invalid_magic).is_err());

        let mut impossible_count = capture_bytes();
        let count_offset = 8 + 8 + SOURCE.len() + 8 * 8;
        impossible_count[count_offset..count_offset + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(decode_stage2_capture(&impossible_count).is_err());
    }

    #[test]
    fn rejects_incomplete_summary_and_truncated_records() {
        let mut incomplete = summary();
        incomplete.status_flags = 3;
        assert!(matches!(
            import_stage2_jir(SOURCE, source_id(), incomplete, &records()),
            Err(Stage2ImportError::InvalidSummary {
                field: "status_flags",
                ..
            })
        ));

        let records = records();
        assert_eq!(
            import_stage2_jir(SOURCE, source_id(), summary(), &records[..8]),
            Err(Stage2ImportError::RecordCount {
                expected: 9,
                actual: 8
            })
        );
    }

    #[test]
    fn rejects_corrupted_type_and_gapped_function_identity() {
        let mut bad_type = records();
        bad_type[0].operand_a = 64;
        assert!(matches!(
            import_stage2_jir(SOURCE, source_id(), summary(), &bad_type),
            Err(Stage2ImportError::InvalidRecord { index: 0, .. })
        ));

        let mut gapped = records();
        gapped[5].function_index = 2;
        assert!(matches!(
            import_stage2_jir(SOURCE, source_id(), summary(), &gapped),
            Err(Stage2ImportError::InvalidRecord { index: 5, .. })
        ));
    }

    #[test]
    fn rejects_wrong_opcode_reserved_operand_and_source_literal() {
        let mut wrong_opcode = records();
        wrong_opcode[3].opcode = 2;
        assert!(matches!(
            import_stage2_jir(SOURCE, source_id(), summary(), &wrong_opcode),
            Err(Stage2ImportError::InvalidRecord { index: 3, .. })
        ));

        let mut reserved = records();
        reserved[4].operand_b = 1;
        assert!(matches!(
            import_stage2_jir(SOURCE, source_id(), summary(), &reserved),
            Err(Stage2ImportError::InvalidRecord { index: 4, .. })
        ));

        let mut wrong_value = records();
        wrong_value[7].operand_a = 41;
        assert!(matches!(
            import_stage2_jir(SOURCE, source_id(), summary(), &wrong_value),
            Err(Stage2ImportError::InvalidRecord { index: 7, .. })
        ));
    }

    #[test]
    fn rejects_out_of_bounds_and_duplicate_function_names() {
        let mut out_of_bounds = records();
        out_of_bounds[8].source_end = SOURCE.len() as u64 + 1;
        assert!(matches!(
            import_stage2_jir(SOURCE, source_id(), summary(), &out_of_bounds),
            Err(Stage2ImportError::InvalidSourceSpan { index: 8, .. })
        ));

        let duplicate_source =
            "fn first() -> Int32 { return 7; } fn first() -> Int32 { return 42; }";
        let mut duplicate = records();
        duplicate[5].source_end = 68;
        duplicate[6].source_start = 54;
        duplicate[6].source_end = 68;
        duplicate[7].source_start = 63;
        duplicate[7].source_end = 65;
        duplicate[8].source_start = 56;
        duplicate[8].source_end = 66;
        assert!(matches!(
            import_stage2_jir(
                duplicate_source,
                source_id_for(duplicate_source),
                summary(),
                &duplicate
            ),
            Err(Stage2ImportError::InvalidFunctionName { index: 5 })
        ));
    }

    #[cfg(windows)]
    #[test]
    fn emits_and_runs_windows_object_from_imported_stage2_jir() {
        let module = import_stage2_jir(SOURCE, source_id(), summary(), &records())
            .expect("complete stage-2 stream should import");
        let context = Context::create();
        let first_object = lower_to_object(
            &context,
            &module,
            "selfhost_stage2",
            &TypeLoweringConfig::default(),
            &ObjectOptions::x86_64_baseline_release(),
        )
        .expect("first imported stage-2 COFF object");
        let second_object = lower_to_object(
            &context,
            &module,
            "selfhost_stage2",
            &TypeLoweringConfig::default(),
            &ObjectOptions::x86_64_baseline_release(),
        )
        .expect("second imported stage-2 COFF object");
        assert_eq!(
            first_object, second_object,
            "COFF output must be reproducible"
        );

        let output = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("selfhost-stage2-object");
        std::fs::create_dir_all(&output).expect("create retained stage-2 artifact directory");
        let first_object_path = output.join("stage2-first.obj");
        let second_object_path = output.join("stage2-second.obj");
        let first_executable = output.join("stage2-first.exe");
        let second_executable = output.join("stage2-second.exe");
        write_object(&first_object_path, &first_object).expect("write first COFF object");
        write_object(&second_object_path, &second_object).expect("write second COFF object");

        let link_options = WindowsLinkOptions {
            entry_symbol: "main".to_owned(),
            ..WindowsLinkOptions::default()
        };
        link_windows_executable(
            &first_executable,
            std::slice::from_ref(&first_object_path),
            &link_options,
        )
        .expect("link first imported stage-2 executable");
        link_windows_executable(
            &second_executable,
            std::slice::from_ref(&second_object_path),
            &link_options,
        )
        .expect("link second imported stage-2 executable");
        assert_eq!(
            std::fs::read(&first_executable).expect("read first executable"),
            std::fs::read(&second_executable).expect("read second executable"),
            "PE output must be reproducible"
        );

        let status = Command::new(&first_executable)
            .status()
            .expect("run imported stage-2 executable");
        assert_eq!(status.code(), Some(42));
    }

    #[cfg(windows)]
    #[test]
    fn imports_loaded_producer_capture_and_runs_windows_object() {
        let Some(capture_path) = std::env::var_os("JADREN_STAGE2_CAPTURE_PATH").map(PathBuf::from)
        else {
            return;
        };
        let capture = std::fs::read(capture_path).expect("read loaded-producer capture");
        let captured = decode_stage2_capture(&capture).expect("decode loaded-producer capture");
        let mut sources = SourceManager::new();
        let source_id = sources
            .add("loaded-producer-stage2.jdn", captured.source.clone())
            .expect("source ID should fit");
        let module = import_stage2_jir(
            &captured.source,
            source_id,
            captured.summary,
            &captured.records,
        )
        .expect("loaded producer stage-2 stream should import");
        assert_eq!(module.functions[0].name, "first");
        assert_eq!(module.functions[1].name, "main");

        let context = Context::create();
        let first_object = lower_to_object(
            &context,
            &module,
            "selfhost_stage2_loaded_producer",
            &TypeLoweringConfig::default(),
            &ObjectOptions::x86_64_baseline_release(),
        )
        .expect("first loaded-producer COFF object");
        let second_object = lower_to_object(
            &context,
            &module,
            "selfhost_stage2_loaded_producer",
            &TypeLoweringConfig::default(),
            &ObjectOptions::x86_64_baseline_release(),
        )
        .expect("second loaded-producer COFF object");
        assert_eq!(
            first_object, second_object,
            "COFF output must be reproducible"
        );

        let output = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("selfhost-stage2-loaded-producer");
        std::fs::create_dir_all(&output).expect("create loaded-producer artifact directory");
        let first_object_path = output.join("stage2-loaded-first.obj");
        let second_object_path = output.join("stage2-loaded-second.obj");
        let first_executable = output.join("stage2-loaded-first.exe");
        let second_executable = output.join("stage2-loaded-second.exe");
        write_object(&first_object_path, &first_object).expect("write first loaded COFF object");
        write_object(&second_object_path, &second_object).expect("write second loaded COFF object");
        let link_options = WindowsLinkOptions {
            entry_symbol: "main".to_owned(),
            ..WindowsLinkOptions::default()
        };
        link_windows_executable(
            &first_executable,
            std::slice::from_ref(&first_object_path),
            &link_options,
        )
        .expect("link first loaded-producer executable");
        link_windows_executable(
            &second_executable,
            std::slice::from_ref(&second_object_path),
            &link_options,
        )
        .expect("link second loaded-producer executable");
        assert_eq!(
            std::fs::read(&first_executable).expect("read first loaded executable"),
            std::fs::read(&second_executable).expect("read second loaded executable"),
            "PE output must be reproducible"
        );
        let status = Command::new(&first_executable)
            .status()
            .expect("run loaded-producer stage-2 executable");
        assert_eq!(status.code(), Some(42));
    }
}
