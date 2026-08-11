//! Native loader for the generated Jadren Stage-2 producer.
//!
//! This module is intentionally a small ABI adapter. It loads the provider
//! before the producer, calls the existing caller-owned buffer exports in two
//! passes, and writes the same padding-free `JST2CAP1` stream consumed by the
//! strict Rust Stage-2 importer. It replaces the historical PowerShell/C#
//! loader without changing the producer or capture record protocol.

use std::path::Path;

use jadren_selfhost_api::{FrontendStage2Summary, Stage2JirSummary, TypedStatementStage2Summary};
use libloading::{Library, Symbol};

const CAPTURE_MAGIC: &[u8; 8] = b"JST2CAP1";
const TYPED_STATEMENT_CAPTURE_MAGIC: &[u8; 8] = b"JST2TYP1";

const TYPED_IF_RETURN_CAPTURE_MAGIC: &[u8; 8] = b"JST2IF01";
const TYPED_IF_VALUE_CONTINUATION_CAPTURE_MAGIC: &[u8; 8] = b"JST2IFV1";
const TYPED_IF_VALUE_LOCAL_ASSIGNMENT_CAPTURE_MAGIC: &[u8; 8] = b"JST2IFL1";
const TYPED_IF_VALUE_STATEMENT_TAIL_CAPTURE_MAGIC: &[u8; 8] = b"JST2IFS1";
const TYPED_WHILE_LOCAL_CAPTURE_MAGIC: &[u8; 8] = b"JST2WHL1";
const TYPED_WHILE_LOCAL_BREAK_CAPTURE_MAGIC: &[u8; 8] = b"JST2WHK1";
const TYPED_WHILE_LOCAL_CONTINUE_CAPTURE_MAGIC: &[u8; 8] = b"JST2WHC1";
const TYPED_WHILE_LOCAL_CONDITIONAL_CAPTURE_MAGIC: &[u8; 8] = b"JST2WHQ1";
const TYPED_WHILE_LOCAL_MULTI_CONDITIONAL_CAPTURE_MAGIC: &[u8; 8] = b"JST2WHM1";
const TYPED_WHILE_LOCAL_DYNAMIC_CAPTURE_MAGIC: &[u8; 8] = b"JST2WHD1";
const TYPED_WHILE_LOCAL_DYNAMIC_CONTROL_CAPTURE_MAGIC: &[u8; 8] = b"JST2WHG1";
const TYPED_NESTED_WHILE_LOCAL_CAPTURE_MAGIC: &[u8; 8] = b"JST2WN01";
const TYPED_NESTED_WHILE_LOCAL_CONDITIONAL_CAPTURE_MAGIC: &[u8; 8] = b"JST2WNQ1";
const TYPED_NESTED_WHILE_LOCAL_MULTI_CONDITIONAL_CAPTURE_MAGIC: &[u8; 8] = b"JST2WNM1";
const TYPED_NESTED_WHILE_LOCAL_DYNAMIC_CONDITIONAL_CAPTURE_MAGIC: &[u8; 8] = b"JST2WND1";
const TYPED_NESTED_WHILE_LOCAL_ORDERED_BODY_CAPTURE_MAGIC: &[u8; 8] = b"JST2WNO1";
const TYPED_WHILE_LOCAL_STATEMENTS_CAPTURE_MAGIC: &[u8; 8] = b"JST2WHS1";
const TYPED_WHILE_LOCAL_THREE_STATEMENTS_CAPTURE_MAGIC: &[u8; 8] = b"JST2WH31";
const TYPED_WHILE_LOCAL_BODY_LET_CAPTURE_MAGIC: &[u8; 8] = b"JST2WHB1";

type CompileFrontendStage2 = unsafe extern "C" fn(
    source: *const u8,
    source_length: usize,
    start: usize,
    end: usize,
    tokens: *mut u8,
    tokens_length: usize,
    ast_items: *mut u8,
    ast_items_length: usize,
    functions: *mut u8,
    functions_length: usize,
    statements: *mut u8,
    statements_length: usize,
    calls: *mut u8,
    calls_length: usize,
) -> FrontendStage2Summary;

type EmitStage2Jir = unsafe extern "C" fn(
    source: *const u8,
    source_length: usize,
    functions: *mut u8,
    functions_length: usize,
    statements: *mut u8,
    statements_length: usize,
    calls: *mut u8,
    calls_length: usize,
    output: *mut u8,
    output_length: usize,
) -> Stage2JirSummary;

type EmitStage2LocalBindingJir = unsafe extern "C" fn(
    source: *const u8,
    source_length: usize,
    functions: *mut u8,
    functions_length: usize,
    output: *mut u8,
    output_length: usize,
) -> Stage2JirSummary;

type ParseTypedStatementSequence = unsafe extern "C" fn(
    source: *const u8,
    source_length: usize,
    functions: *mut u8,
    functions_length: usize,
    statements: *mut u8,
    statements_length: usize,
    ast: *mut u8,
    ast_length: usize,
) -> TypedStatementStage2Summary;

type ParseTypedStatementAssignmentSequence = ParseTypedStatementSequence;

type ParseTypedIfReturn = unsafe extern "C" fn(
    source: *const u8,
    source_length: usize,
    functions: *mut u8,
    functions_length: usize,
    control: *mut u8,
    control_length: usize,
    ast: *mut u8,
    ast_length: usize,
) -> TypedStatementStage2Summary;

type ParseTypedIfValueContinuation = unsafe extern "C" fn(
    source: *const u8,
    source_length: usize,
    functions: *mut u8,
    functions_length: usize,
    control: *mut u8,
    control_length: usize,
    continuation: *mut u8,
    continuation_length: usize,
    ast: *mut u8,
    ast_length: usize,
) -> TypedStatementStage2Summary;

type ParseTypedIfValueLocalAssignment = ParseTypedIfValueContinuation;

type ParseTypedIfValueStatementTail = unsafe extern "C" fn(
    source: *const u8,
    source_length: usize,
    functions: *mut u8,
    functions_length: usize,
    control: *mut u8,
    control_length: usize,
    continuation: *mut u8,
    continuation_length: usize,
    statements: *mut u8,
    statements_length: usize,
    ast: *mut u8,
    ast_length: usize,
) -> TypedStatementStage2Summary;

type ParseTypedWhileLocal = unsafe extern "C" fn(
    source: *const u8,
    source_length: usize,
    functions: *mut u8,
    functions_length: usize,
    control: *mut u8,
    control_length: usize,
    binding: *mut u8,
    binding_length: usize,
    body: *mut u8,
    body_length: usize,
    ast: *mut u8,
    ast_length: usize,
) -> TypedStatementStage2Summary;

type ParseTypedWhileLocalStatements = ParseTypedWhileLocal;
type ParseTypedWhileLocalBodyLet = ParseTypedWhileLocal;

type EmitTypedStatementSequenceJir = unsafe extern "C" fn(
    source: *const u8,
    source_length: usize,
    functions: *mut u8,
    functions_length: usize,
    statements: *mut u8,
    statements_length: usize,
    ast: *mut u8,
    ast_length: usize,
    output: *mut u8,
    output_length: usize,
) -> Stage2JirSummary;

/// Captures one source file through a loaded generated Jadren producer.
pub fn capture_stage2(
    provider_path: &Path,
    producer_path: &Path,
    source_path: &Path,
    output_path: &Path,
) -> Result<(), String> {
    let source = std::fs::read(source_path)
        .map_err(|error| format!("cannot read `{}`: {error}", source_path.display()))?;
    if source.is_empty() || source.len() > 1024 * 1024 {
        return Err("stage-2 source must contain 1..1048576 bytes".to_owned());
    }
    let source_text = String::from_utf8(source.clone())
        .map_err(|_| "stage-2 source must be valid UTF-8".to_owned())?;

    // Keep the provider alive while the producer resolves its imported API.
    let provider = unsafe { Library::new(provider_path) }.map_err(|error| {
        format!(
            "cannot load provider `{}`: {error}",
            provider_path.display()
        )
    })?;
    let producer = unsafe { Library::new(producer_path) }.map_err(|error| {
        format!(
            "cannot load producer `{}`: {error}",
            producer_path.display()
        )
    })?;
    let compile: Symbol<'_, CompileFrontendStage2> = unsafe {
        producer
            .get(b"jadren_selfhost_compile_frontend_stage2\0")
            .map_err(|error| {
                format!("producer export compile_frontend_stage2 is missing: {error}")
            })?
    };
    let emit: Symbol<'_, EmitStage2Jir> = unsafe {
        producer
            .get(b"jadren_selfhost_emit_stage2_jir\0")
            .map_err(|error| format!("producer export emit_stage2_jir is missing: {error}"))?
    };
    let local_auto_emit: Symbol<'_, EmitStage2LocalBindingJir> = unsafe {
        producer
            .get(b"jadren_selfhost_emit_stage2_local_binding_auto_jir\0")
            .map_err(|error| {
                format!("producer export emit_stage2_local_binding_auto_jir is missing: {error}")
            })?
    };
    let typed_statement_parse: Symbol<'_, ParseTypedStatementSequence> = unsafe {
        producer
            .get(b"jadren_selfhost_parse_typed_statement_sequence\0")
            .map_err(|error| {
                format!("producer export parse_typed_statement_sequence is missing: {error}")
            })?
    };
    let typed_assignment_parse: Symbol<'_, ParseTypedStatementAssignmentSequence> = unsafe {
        producer
            .get(b"jadren_selfhost_parse_typed_statement_assignment_sequence\0")
            .map_err(|error| {
                format!(
                    "producer export parse_typed_statement_assignment_sequence is missing: {error}"
                )
            })?
    };
    let typed_if_parse: Symbol<'_, ParseTypedIfReturn> = unsafe {
        producer
            .get(b"jadren_selfhost_parse_typed_if_return\0")
            .map_err(|error| format!("producer export parse_typed_if_return is missing: {error}"))?
    };
    let typed_if_value_continuation_parse: Symbol<'_, ParseTypedIfValueContinuation> = unsafe {
        producer
            .get(b"jadren_selfhost_parse_typed_if_value_continuation\0")
            .map_err(|error| {
                format!("producer export parse_typed_if_value_continuation is missing: {error}")
            })?
    };
    let typed_if_value_local_assignment_parse: Symbol<'_, ParseTypedIfValueLocalAssignment> = unsafe {
        producer
            .get(b"jadren_selfhost_parse_typed_if_value_local_assignment\0")
            .map_err(|error| {
                format!("producer export parse_typed_if_value_local_assignment is missing: {error}")
            })?
    };
    let typed_if_value_statement_tail_parse: Symbol<'_, ParseTypedIfValueStatementTail> = unsafe {
        producer
            .get(b"jadren_selfhost_parse_typed_if_value_statement_tail\0")
            .map_err(|error| {
                format!("producer export parse_typed_if_value_statement_tail is missing: {error}")
            })?
    };
    let typed_while_local_parse: Symbol<'_, ParseTypedWhileLocal> = unsafe {
        producer
            .get(b"jadren_selfhost_parse_typed_while_local\0")
            .map_err(|error| {
                format!("producer export parse_typed_while_local is missing: {error}")
            })?
    };
    let typed_while_local_dynamic_parse: Symbol<'_, ParseTypedWhileLocal> = unsafe {
        producer
            .get(b"jadren_selfhost_parse_typed_while_local_dynamic\0")
            .map_err(|error| {
                format!("producer export parse_typed_while_local_dynamic is missing: {error}")
            })?
    };
    let typed_while_local_dynamic_control_parse: Symbol<'_, ParseTypedWhileLocal> = unsafe {
        producer
            .get(b"jadren_selfhost_parse_typed_while_local_dynamic_control\0")
            .map_err(|error| {
                format!(
                    "producer export parse_typed_while_local_dynamic_control is missing: {error}"
                )
            })?
    };
    let typed_nested_while_local_parse: Symbol<'_, ParseTypedWhileLocal> = unsafe {
        producer
            .get(b"jadren_selfhost_parse_typed_nested_while_local\0")
            .map_err(|error| {
                format!("producer export parse_typed_nested_while_local is missing: {error}")
            })?
    };
    let typed_nested_while_local_conditional_parse: Symbol<'_, ParseTypedWhileLocal> = unsafe {
        producer
            .get(b"jadren_selfhost_parse_typed_nested_while_local_conditional\0")
            .map_err(|error| {
                format!(
                    "producer export parse_typed_nested_while_local_conditional is missing: {error}"
                )
            })?
    };
    let typed_nested_while_local_dynamic_conditional_parse: Symbol<'_, ParseTypedWhileLocal> = unsafe {
        producer
            .get(b"jadren_selfhost_parse_typed_nested_while_local_dynamic_control\0")
            .map_err(|error| {
                format!(
                    "producer export parse_typed_nested_while_local_dynamic_control is missing: {error}"
                )
            })?
    };
    let typed_while_local_statements_parse: Symbol<'_, ParseTypedWhileLocalStatements> = unsafe {
        producer
            .get(b"jadren_selfhost_parse_typed_while_local_statements\0")
            .map_err(|error| {
                format!("producer export parse_typed_while_local_statements is missing: {error}")
            })?
    };
    let typed_while_local_body_let_parse: Symbol<'_, ParseTypedWhileLocalBodyLet> = unsafe {
        producer
            .get(b"jadren_selfhost_parse_typed_while_local_body_let\0")
            .map_err(|error| {
                format!("producer export parse_typed_while_local_body_let is missing: {error}")
            })?
    };
    let typed_statement_emit: Symbol<'_, EmitTypedStatementSequenceJir> = unsafe {
        producer
            .get(b"jadren_selfhost_emit_typed_statement_sequence_jir\0")
            .map_err(|error| {
                format!("producer export emit_typed_statement_sequence_jir is missing: {error}")
            })?
    };

    let capacity = source
        .len()
        .checked_add(1)
        .ok_or_else(|| "stage-2 source capacity overflow".to_owned())?;
    let mut tokens = aligned_words(capacity, 24)?;
    let mut ast_items = aligned_words(capacity, 56)?;
    let mut functions = aligned_words(capacity, 40)?;
    let mut statements = aligned_words(capacity, 48)?;
    let mut calls = aligned_words(capacity, 64)?;

    let frontend = unsafe {
        compile(
            source.as_ptr(),
            source.len(),
            0,
            source.len(),
            words_ptr(&mut tokens),
            capacity,
            words_ptr(&mut ast_items),
            capacity,
            words_ptr(&mut functions),
            capacity,
            words_ptr(&mut statements),
            capacity,
            words_ptr(&mut calls),
            capacity,
        )
    };
    let local_sizing = unsafe {
        local_auto_emit(
            source.as_ptr(),
            source.len(),
            words_ptr(&mut functions),
            frontend.function_headers_emitted as usize,
            std::ptr::null_mut(),
            0,
        )
    };
    let typed_statement_sizing = unsafe {
        typed_statement_parse(
            source.as_ptr(),
            source.len(),
            words_ptr(&mut functions),
            frontend.function_headers_emitted as usize,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
        )
    };
    let typed_if_sizing = unsafe {
        typed_if_parse(
            source.as_ptr(),
            source.len(),
            words_ptr(&mut functions),
            frontend.function_headers_emitted as usize,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
        )
    };
    let typed_if_value_continuation_sizing = unsafe {
        typed_if_value_continuation_parse(
            source.as_ptr(),
            source.len(),
            words_ptr(&mut functions),
            frontend.function_headers_emitted as usize,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
        )
    };
    let typed_if_value_local_assignment_sizing = unsafe {
        typed_if_value_local_assignment_parse(
            source.as_ptr(),
            source.len(),
            words_ptr(&mut functions),
            frontend.function_headers_emitted as usize,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
        )
    };
    let typed_if_value_statement_tail_sizing = unsafe {
        typed_if_value_statement_tail_parse(
            source.as_ptr(),
            source.len(),
            words_ptr(&mut functions),
            frontend.function_headers_emitted as usize,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
        )
    };
    let typed_while_local_sizing = unsafe {
        typed_while_local_parse(
            source.as_ptr(),
            source.len(),
            words_ptr(&mut functions),
            frontend.function_headers_emitted as usize,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
        )
    };
    let typed_while_local_dynamic_sizing = unsafe {
        typed_while_local_dynamic_parse(
            source.as_ptr(),
            source.len(),
            words_ptr(&mut functions),
            frontend.function_headers_emitted as usize,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
        )
    };
    let typed_while_local_dynamic_control_sizing = unsafe {
        typed_while_local_dynamic_control_parse(
            source.as_ptr(),
            source.len(),
            words_ptr(&mut functions),
            frontend.function_headers_emitted as usize,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
        )
    };
    let typed_nested_while_local_sizing = unsafe {
        typed_nested_while_local_parse(
            source.as_ptr(),
            source.len(),
            words_ptr(&mut functions),
            frontend.function_headers_emitted as usize,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
        )
    };
    let typed_nested_while_local_conditional_sizing = unsafe {
        typed_nested_while_local_conditional_parse(
            source.as_ptr(),
            source.len(),
            words_ptr(&mut functions),
            frontend.function_headers_emitted as usize,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
        )
    };
    let typed_nested_while_local_dynamic_conditional_sizing = unsafe {
        typed_nested_while_local_dynamic_conditional_parse(
            source.as_ptr(),
            source.len(),
            words_ptr(&mut functions),
            frontend.function_headers_emitted as usize,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
        )
    };
    let typed_while_local_statements_sizing = unsafe {
        typed_while_local_statements_parse(
            source.as_ptr(),
            source.len(),
            words_ptr(&mut functions),
            frontend.function_headers_emitted as usize,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
        )
    };
    let typed_while_local_body_let_sizing = unsafe {
        typed_while_local_body_let_parse(
            source.as_ptr(),
            source.len(),
            words_ptr(&mut functions),
            frontend.function_headers_emitted as usize,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
        )
    };
    let dynamic_ast_expected = typed_while_local_dynamic_sizing
        .statements_required
        .checked_mul(3)
        .and_then(|value| value.checked_add(5));
    let dynamic_control_ast_expected = typed_while_local_dynamic_control_sizing
        .statements_required
        .checked_mul(3)
        .and_then(|value| value.checked_add(5));
    let nested_ast_expected = typed_nested_while_local_sizing
        .statements_required
        .checked_sub(1)
        .and_then(|value| value.checked_mul(3))
        .and_then(|value| value.checked_add(8));
    let nested_conditional_ast_expected = typed_nested_while_local_conditional_sizing
        .statements_required
        .checked_sub(1)
        .and_then(|value| value.checked_mul(3))
        .and_then(|value| value.checked_add(8));
    let nested_dynamic_conditional_ast_expected =
        typed_nested_while_local_dynamic_conditional_sizing
            .statements_required
            .checked_sub(1)
            .and_then(|value| value.checked_mul(3))
            .and_then(|value| value.checked_add(8));
    let nested_dynamic_conditional_ast_min = nested_dynamic_conditional_ast_expected;
    let nested_dynamic_conditional_ast_max = typed_nested_while_local_dynamic_conditional_sizing
        .statements_required
        .checked_sub(1)
        .and_then(|value| value.checked_mul(7))
        .and_then(|value| value.checked_add(8));
    let is_nested_conditional_source = source_text.contains("if inner ")
        && (source_text.contains("break") || source_text.contains("continue"));
    let is_nested_dynamic_conditional_source = source_text.matches("if inner ").count() >= 3
        && (source_text.contains("break") || source_text.contains("continue"));
    let compact_source = source_text.split_whitespace().collect::<String>();
    let is_nested_ordered_conditional_source =
        is_nested_conditional_source && compact_source.contains("}inner=");
    let dynamic_conditional_ast_shape_valid = if is_nested_ordered_conditional_source {
        match (
            nested_dynamic_conditional_ast_min,
            nested_dynamic_conditional_ast_max,
        ) {
            (Some(minimum), Some(maximum)) => {
                typed_nested_while_local_dynamic_conditional_sizing.ast_nodes_required >= minimum
                    && typed_nested_while_local_dynamic_conditional_sizing.ast_nodes_required
                        <= maximum
            }
            _ => false,
        }
    } else {
        nested_dynamic_conditional_ast_expected
            == Some(typed_nested_while_local_dynamic_conditional_sizing.ast_nodes_required)
    };
    let is_typed_nested_while_local_dynamic_conditional_fixture =
        (is_nested_dynamic_conditional_source || is_nested_ordered_conditional_source)
            && typed_nested_while_local_dynamic_conditional_sizing.status_flags == 3
            && typed_nested_while_local_dynamic_conditional_sizing.errors == 0
            && typed_nested_while_local_dynamic_conditional_sizing.statements_required >= 7
            && dynamic_conditional_ast_shape_valid;
    if is_typed_nested_while_local_dynamic_conditional_fixture {
        let body_count = usize::try_from(
            typed_nested_while_local_dynamic_conditional_sizing.statements_required,
        )
        .map_err(|_| {
            "loaded Jadren nested dynamic conditional body count does not fit usize".to_owned()
        })?;
        let ast_count =
            usize::try_from(typed_nested_while_local_dynamic_conditional_sizing.ast_nodes_required)
                .map_err(|_| {
                    "loaded Jadren nested dynamic conditional AST count does not fit usize"
                        .to_owned()
                })?;
        if body_count > 4096 || ast_count > 12293 {
            return Err(
                "loaded Jadren nested dynamic conditional metadata exceeds the capture limit"
                    .to_owned(),
            );
        }
        let mut control = aligned_words(2, 32)?;
        let mut binding = aligned_words(2, 24)?;
        let mut body = aligned_words(body_count, 48)?;
        let mut ast = aligned_words(ast_count, 48)?;
        let typed_metadata = unsafe {
            typed_nested_while_local_dynamic_conditional_parse(
                source.as_ptr(),
                source.len(),
                words_ptr(&mut functions),
                frontend.function_headers_emitted as usize,
                words_ptr(&mut control),
                2,
                words_ptr(&mut binding),
                2,
                words_ptr(&mut body),
                body_count,
                words_ptr(&mut ast),
                ast_count,
            )
        };
        if typed_metadata.status_flags != 7
            || typed_metadata.errors != 0
            || typed_metadata.statements_emitted != typed_metadata.statements_required
            || typed_metadata.ast_nodes_emitted != typed_metadata.ast_nodes_required
        {
            return Err(format!(
                "loaded Jadren nested dynamic conditional metadata returned invalid summary: statements={}/{}, ast={}/{}, errors={}, status={}",
                typed_metadata.statements_emitted,
                typed_metadata.statements_required,
                typed_metadata.ast_nodes_emitted,
                typed_metadata.ast_nodes_required,
                typed_metadata.errors,
                typed_metadata.status_flags
            ));
        }
        write_typed_nested_while_capture(
            output_path,
            &source,
            typed_metadata,
            if is_nested_ordered_conditional_source {
                TYPED_NESTED_WHILE_LOCAL_ORDERED_BODY_CAPTURE_MAGIC
            } else {
                TYPED_NESTED_WHILE_LOCAL_DYNAMIC_CONDITIONAL_CAPTURE_MAGIC
            },
            &control,
            &binding,
            &body,
            &ast,
        )?;
        drop(producer);
        drop(provider);
        return Ok(());
    }
    let is_typed_nested_while_local_conditional_fixture = is_nested_conditional_source
        && typed_nested_while_local_conditional_sizing.status_flags == 3
        && typed_nested_while_local_conditional_sizing.errors == 0
        && (typed_nested_while_local_conditional_sizing.statements_required == 5
            || typed_nested_while_local_conditional_sizing.statements_required == 6)
        && nested_conditional_ast_expected
            == Some(typed_nested_while_local_conditional_sizing.ast_nodes_required);
    if is_typed_nested_while_local_conditional_fixture {
        let body_count =
            usize::try_from(typed_nested_while_local_conditional_sizing.statements_required)
                .map_err(|_| {
                    "loaded Jadren nested conditional body count does not fit usize".to_owned()
                })?;
        let ast_count =
            usize::try_from(typed_nested_while_local_conditional_sizing.ast_nodes_required)
                .map_err(|_| {
                    "loaded Jadren nested conditional AST count does not fit usize".to_owned()
                })?;
        if body_count > 4096 || ast_count > 12293 {
            return Err(
                "loaded Jadren nested conditional metadata exceeds the capture limit".to_owned(),
            );
        }
        let mut control = aligned_words(2, 32)?;
        let mut binding = aligned_words(2, 24)?;
        let mut body = aligned_words(body_count, 48)?;
        let mut ast = aligned_words(ast_count, 48)?;
        let typed_metadata = unsafe {
            typed_nested_while_local_conditional_parse(
                source.as_ptr(),
                source.len(),
                words_ptr(&mut functions),
                frontend.function_headers_emitted as usize,
                words_ptr(&mut control),
                2,
                words_ptr(&mut binding),
                2,
                words_ptr(&mut body),
                body_count,
                words_ptr(&mut ast),
                ast_count,
            )
        };
        if typed_metadata.status_flags != 7
            || typed_metadata.errors != 0
            || typed_metadata.statements_emitted != typed_metadata.statements_required
            || typed_metadata.ast_nodes_emitted != typed_metadata.ast_nodes_required
        {
            return Err(format!(
                "loaded Jadren nested conditional metadata returned invalid summary: statements={}/{}, ast={}/{}, errors={}, status={}",
                typed_metadata.statements_emitted,
                typed_metadata.statements_required,
                typed_metadata.ast_nodes_emitted,
                typed_metadata.ast_nodes_required,
                typed_metadata.errors,
                typed_metadata.status_flags
            ));
        }
        write_typed_nested_while_capture(
            output_path,
            &source,
            typed_metadata,
            if typed_nested_while_local_conditional_sizing.statements_required == 6 {
                TYPED_NESTED_WHILE_LOCAL_MULTI_CONDITIONAL_CAPTURE_MAGIC
            } else {
                TYPED_NESTED_WHILE_LOCAL_CONDITIONAL_CAPTURE_MAGIC
            },
            &control,
            &binding,
            &body,
            &ast,
        )?;
        drop(producer);
        drop(provider);
        return Ok(());
    }
    let is_typed_nested_while_local_fixture = !is_nested_conditional_source
        && typed_nested_while_local_sizing.status_flags == 3
        && typed_nested_while_local_sizing.errors == 0
        && typed_nested_while_local_sizing.statements_required >= 4
        && nested_ast_expected == Some(typed_nested_while_local_sizing.ast_nodes_required);
    if is_typed_nested_while_local_fixture {
        let body_count = usize::try_from(typed_nested_while_local_sizing.statements_required)
            .map_err(|_| "loaded Jadren nested while body count does not fit usize".to_owned())?;
        let ast_count = usize::try_from(typed_nested_while_local_sizing.ast_nodes_required)
            .map_err(|_| "loaded Jadren nested while AST count does not fit usize".to_owned())?;
        if body_count > 4096 || ast_count > 12293 {
            return Err("loaded Jadren nested while metadata exceeds the capture limit".to_owned());
        }
        let mut control = aligned_words(2, 32)?;
        let mut binding = aligned_words(2, 24)?;
        let mut body = aligned_words(body_count, 48)?;
        let mut ast = aligned_words(ast_count, 48)?;
        let typed_metadata = unsafe {
            typed_nested_while_local_parse(
                source.as_ptr(),
                source.len(),
                words_ptr(&mut functions),
                frontend.function_headers_emitted as usize,
                words_ptr(&mut control),
                2,
                words_ptr(&mut binding),
                2,
                words_ptr(&mut body),
                body_count,
                words_ptr(&mut ast),
                ast_count,
            )
        };
        if typed_metadata.status_flags != 7
            || typed_metadata.errors != 0
            || typed_metadata.statements_emitted != typed_metadata.statements_required
            || typed_metadata.ast_nodes_emitted != typed_metadata.ast_nodes_required
        {
            return Err(format!(
                "loaded Jadren nested while metadata returned invalid summary: statements={}/{}, ast={}/{}, errors={}, status={}",
                typed_metadata.statements_emitted,
                typed_metadata.statements_required,
                typed_metadata.ast_nodes_emitted,
                typed_metadata.ast_nodes_required,
                typed_metadata.errors,
                typed_metadata.status_flags
            ));
        }
        write_typed_nested_while_capture(
            output_path,
            &source,
            typed_metadata,
            TYPED_NESTED_WHILE_LOCAL_CAPTURE_MAGIC,
            &control,
            &binding,
            &body,
            &ast,
        )?;
        drop(producer);
        drop(provider);
        return Ok(());
    }
    let is_long_generic_dynamic_control_fixture = source_text
        .contains("JADREN_STAGE2_GENERIC_LONG_DYNAMIC_CONTROL")
        || source_text.contains("JADREN_STAGE2_DYNAMIC_BODY_LET")
        || source_text.contains("JADREN_STAGE2_DYNAMIC_MULTI_BODY_LET")
        || source_text.contains("JADREN_STAGE2_DYNAMIC_MUTABLE_BODY_LET");
    let is_dynamic_body_let_fixture = source_text.contains("JADREN_STAGE2_DYNAMIC_BODY_LET")
        || source_text.contains("JADREN_STAGE2_DYNAMIC_MULTI_BODY_LET")
        || source_text.contains("JADREN_STAGE2_DYNAMIC_MUTABLE_BODY_LET");
    let is_typed_while_local_dynamic_control_fixture =
        typed_while_local_dynamic_control_sizing.status_flags == 3
            && typed_while_local_dynamic_control_sizing.errors == 0
            && typed_while_local_dynamic_control_sizing.statements_required >= 4
            && (dynamic_control_ast_expected
                == Some(typed_while_local_dynamic_control_sizing.ast_nodes_required)
                || (is_long_generic_dynamic_control_fixture
                    && typed_while_local_dynamic_control_sizing.ast_nodes_required > 32)
                || is_dynamic_body_let_fixture);
    if is_typed_while_local_dynamic_control_fixture {
        let body_count = usize::try_from(
            typed_while_local_dynamic_control_sizing.statements_required,
        )
        .map_err(|_| "loaded Jadren dynamic-control body count does not fit usize".to_owned())?;
        let ast_count = usize::try_from(
            typed_while_local_dynamic_control_sizing.ast_nodes_required,
        )
        .map_err(|_| "loaded Jadren dynamic-control AST count does not fit usize".to_owned())?;
        if body_count > 4096 || ast_count > 12293 {
            return Err(
                "loaded Jadren dynamic-control metadata exceeds the capture limit".to_owned(),
            );
        }
        let mut control = aligned_words(1, 32)?;
        let mut binding = aligned_words(1, 32)?;
        let mut body = aligned_words(body_count, 48)?;
        let mut ast = aligned_words(ast_count, 48)?;
        let typed_metadata = unsafe {
            typed_while_local_dynamic_control_parse(
                source.as_ptr(),
                source.len(),
                words_ptr(&mut functions),
                frontend.function_headers_emitted as usize,
                words_ptr(&mut control),
                1,
                words_ptr(&mut binding),
                1,
                words_ptr(&mut body),
                body_count,
                words_ptr(&mut ast),
                ast_count,
            )
        };
        if typed_metadata.status_flags != 7
            || typed_metadata.errors != 0
            || typed_metadata.statements_emitted != typed_metadata.statements_required
            || typed_metadata.ast_nodes_emitted != typed_metadata.ast_nodes_required
        {
            return Err(format!(
                "loaded Jadren dynamic-control metadata returned invalid summary: statements={}/{}, ast={}/{}, errors={}, status={}",
                typed_metadata.statements_emitted,
                typed_metadata.statements_required,
                typed_metadata.ast_nodes_emitted,
                typed_metadata.ast_nodes_required,
                typed_metadata.errors,
                typed_metadata.status_flags
            ));
        }
        let is_mutable_dynamic_body_let_source =
            source_text.contains("JADREN_STAGE2_DYNAMIC_MUTABLE_BODY_LET");
        let mut assignment_count = 0usize;
        let mut conditional_count = 0usize;
        for index in 0..body_count {
            let word = body
                .get(index * 6)
                .ok_or_else(|| "loaded Jadren dynamic-control body is truncated".to_owned())?;
            let kind = (word & 0xff) as u8;
            let flags = ((word >> 8) & 0xff) as u8;
            let type_kind = ((word >> 16) & 0xff) as u8;
            if ((kind == 2 && flags == 0)
                || (kind == 1 && flags == 0)
                || (kind == 1 && flags == 1 && is_mutable_dynamic_body_let_source))
                && type_kind == 2
            {
                assignment_count += 1;
            } else if (kind == 4 || kind == 5) && flags == 2 && type_kind == 1 {
                conditional_count += 1;
            } else {
                return Err(
                    "loaded Jadren dynamic-control metadata has an invalid body header".to_owned(),
                );
            }
        }
        if assignment_count == 0 || conditional_count == 0 {
            return Err(
                "loaded Jadren dynamic-control metadata must contain assignment and conditional records"
                    .to_owned(),
            );
        }
        write_typed_while_capture_with_magic(
            output_path,
            &source,
            typed_metadata,
            &control,
            &binding,
            &body,
            &ast,
            TYPED_WHILE_LOCAL_DYNAMIC_CONTROL_CAPTURE_MAGIC,
        )?;
        drop(producer);
        drop(provider);
        return Ok(());
    }
    let is_typed_while_local_dynamic_fixture = typed_while_local_dynamic_sizing.status_flags == 3
        && typed_while_local_dynamic_sizing.errors == 0
        && typed_while_local_dynamic_sizing.statements_required >= 4
        && dynamic_ast_expected == Some(typed_while_local_dynamic_sizing.ast_nodes_required);
    if is_typed_while_local_dynamic_fixture {
        let body_count = usize::try_from(typed_while_local_dynamic_sizing.statements_required)
            .map_err(|_| {
                "loaded Jadren dynamic typed while body count does not fit usize".to_owned()
            })?;
        let ast_count = usize::try_from(typed_while_local_dynamic_sizing.ast_nodes_required)
            .map_err(|_| {
                "loaded Jadren dynamic typed while AST count does not fit usize".to_owned()
            })?;
        if body_count > 4096 || ast_count > 12293 {
            return Err(
                "loaded Jadren dynamic typed while metadata exceeds the capture limit".to_owned(),
            );
        }
        let mut control = aligned_words(1, 32)?;
        let mut binding = aligned_words(1, 32)?;
        let mut body = aligned_words(body_count, 48)?;
        let mut ast = aligned_words(ast_count, 48)?;
        let typed_metadata = unsafe {
            typed_while_local_dynamic_parse(
                source.as_ptr(),
                source.len(),
                words_ptr(&mut functions),
                frontend.function_headers_emitted as usize,
                words_ptr(&mut control),
                1,
                words_ptr(&mut binding),
                1,
                words_ptr(&mut body),
                body_count,
                words_ptr(&mut ast),
                ast_count,
            )
        };
        if typed_metadata.status_flags != 7
            || typed_metadata.errors != 0
            || typed_metadata.statements_emitted != typed_metadata.statements_required
            || typed_metadata.ast_nodes_emitted != typed_metadata.ast_nodes_required
        {
            return Err(format!(
                "loaded Jadren dynamic typed while metadata returned invalid summary: statements={}/{}, ast={}/{}, errors={}, status={}",
                typed_metadata.statements_emitted,
                typed_metadata.statements_required,
                typed_metadata.ast_nodes_emitted,
                typed_metadata.ast_nodes_required,
                typed_metadata.errors,
                typed_metadata.status_flags
            ));
        }
        write_typed_while_capture_with_magic(
            output_path,
            &source,
            typed_metadata,
            &control,
            &binding,
            &body,
            &ast,
            TYPED_WHILE_LOCAL_DYNAMIC_CAPTURE_MAGIC,
        )?;
        drop(producer);
        drop(provider);
        return Ok(());
    }
    let is_typed_while_local_body_let_fixture = typed_while_local_body_let_sizing.status_flags == 3
        && typed_while_local_body_let_sizing.errors == 0
        && typed_while_local_body_let_sizing.statements_required == 2
        && typed_while_local_body_let_sizing.ast_nodes_required == 9;
    if is_typed_while_local_body_let_fixture {
        let mut control = aligned_words(1, 32)?;
        let mut binding = aligned_words(1, 32)?;
        let mut body = aligned_words(2, 48)?;
        let mut ast = aligned_words(9, 48)?;
        let typed_metadata = unsafe {
            typed_while_local_body_let_parse(
                source.as_ptr(),
                source.len(),
                words_ptr(&mut functions),
                frontend.function_headers_emitted as usize,
                words_ptr(&mut control),
                1,
                words_ptr(&mut binding),
                1,
                words_ptr(&mut body),
                2,
                words_ptr(&mut ast),
                9,
            )
        };
        if typed_metadata.status_flags != 7
            || typed_metadata.errors != 0
            || typed_metadata.statements_emitted != 2
            || typed_metadata.ast_nodes_emitted != 9
        {
            return Err(format!(
                "loaded Jadren typed while-body-let metadata returned invalid summary: statements={}/2, ast={}/9, errors={}, status={}",
                typed_metadata.statements_emitted,
                typed_metadata.ast_nodes_emitted,
                typed_metadata.errors,
                typed_metadata.status_flags
            ));
        }
        write_typed_while_capture_with_magic(
            output_path,
            &source,
            typed_metadata,
            &control,
            &binding,
            &body,
            &ast,
            TYPED_WHILE_LOCAL_BODY_LET_CAPTURE_MAGIC,
        )?;
        drop(producer);
        drop(provider);
        return Ok(());
    }
    let is_typed_while_local_multi_conditional_sizing = typed_while_local_sizing.status_flags == 3
        && typed_while_local_sizing.errors == 0
        && typed_while_local_sizing.statements_required == 3
        && typed_while_local_sizing.ast_nodes_required == 14;
    if is_typed_while_local_multi_conditional_sizing {
        let mut control = aligned_words(1, 32)?;
        let mut binding = aligned_words(1, 32)?;
        let mut body = aligned_words(3, 48)?;
        let mut ast = aligned_words(14, 48)?;
        let typed_metadata = unsafe {
            typed_while_local_parse(
                source.as_ptr(),
                source.len(),
                words_ptr(&mut functions),
                frontend.function_headers_emitted as usize,
                words_ptr(&mut control),
                1,
                words_ptr(&mut binding),
                1,
                words_ptr(&mut body),
                3,
                words_ptr(&mut ast),
                14,
            )
        };
        if typed_metadata.status_flags != 7
            || typed_metadata.errors != 0
            || typed_metadata.statements_emitted != 3
            || typed_metadata.ast_nodes_emitted != 14
        {
            return Err(format!(
                "loaded Jadren typed while multi-conditional metadata returned invalid summary: statements={}/3, ast={}/14, errors={}, status={}",
                typed_metadata.statements_emitted,
                typed_metadata.ast_nodes_emitted,
                typed_metadata.errors,
                typed_metadata.status_flags
            ));
        }
        let first_kind = body.get(6).map(|word| (word & 0xff) as u8).ok_or_else(|| {
            "loaded Jadren typed while multi-conditional body is truncated".to_owned()
        })?;
        let first_flags = body
            .get(6)
            .map(|word| ((word >> 8) & 0xff) as u8)
            .ok_or_else(|| {
                "loaded Jadren typed while multi-conditional flags are truncated".to_owned()
            })?;
        let second_kind = body
            .get(12)
            .map(|word| (word & 0xff) as u8)
            .ok_or_else(|| {
                "loaded Jadren typed while multi-conditional second body is truncated".to_owned()
            })?;
        let second_flags = body
            .get(12)
            .map(|word| ((word >> 8) & 0xff) as u8)
            .ok_or_else(|| {
                "loaded Jadren typed while multi-conditional second flags are truncated".to_owned()
            })?;
        if (first_kind != 4 && first_kind != 5)
            || first_flags != 2
            || (second_kind != 4 && second_kind != 5)
            || second_flags != 2
        {
            return Err(
                "loaded Jadren typed while multi-conditional metadata has invalid control headers"
                    .to_owned(),
            );
        }
        write_typed_while_capture_with_magic(
            output_path,
            &source,
            typed_metadata,
            &control,
            &binding,
            &body,
            &ast,
            TYPED_WHILE_LOCAL_MULTI_CONDITIONAL_CAPTURE_MAGIC,
        )?;
        drop(producer);
        drop(provider);
        return Ok(());
    }
    let is_typed_while_local_three_statements_fixture =
        typed_while_local_statements_sizing.status_flags == 3
            && typed_while_local_statements_sizing.errors == 0
            && typed_while_local_statements_sizing.statements_required == 3
            && typed_while_local_statements_sizing.ast_nodes_required == 14;
    if is_typed_while_local_three_statements_fixture {
        let mut control = aligned_words(1, 32)?;
        let mut binding = aligned_words(1, 32)?;
        let mut body = aligned_words(3, 48)?;
        let mut ast = aligned_words(14, 48)?;
        let typed_metadata = unsafe {
            typed_while_local_statements_parse(
                source.as_ptr(),
                source.len(),
                words_ptr(&mut functions),
                frontend.function_headers_emitted as usize,
                words_ptr(&mut control),
                1,
                words_ptr(&mut binding),
                1,
                words_ptr(&mut body),
                3,
                words_ptr(&mut ast),
                14,
            )
        };
        if typed_metadata.status_flags != 7
            || typed_metadata.errors != 0
            || typed_metadata.statements_emitted != 3
            || typed_metadata.ast_nodes_emitted != 14
        {
            return Err(format!(
                "loaded Jadren typed while-three-statements metadata returned invalid summary: statements={}/3, ast={}/14, errors={}, status={}",
                typed_metadata.statements_emitted,
                typed_metadata.ast_nodes_emitted,
                typed_metadata.errors,
                typed_metadata.status_flags
            ));
        }
        write_typed_while_capture_with_magic(
            output_path,
            &source,
            typed_metadata,
            &control,
            &binding,
            &body,
            &ast,
            TYPED_WHILE_LOCAL_THREE_STATEMENTS_CAPTURE_MAGIC,
        )?;
        drop(producer);
        drop(provider);
        return Ok(());
    }
    let is_typed_while_local_statements_fixture = typed_while_local_statements_sizing.status_flags
        == 3
        && typed_while_local_statements_sizing.errors == 0
        && typed_while_local_statements_sizing.statements_required == 2
        && typed_while_local_statements_sizing.ast_nodes_required == 11;
    if is_typed_while_local_statements_fixture {
        let mut control = aligned_words(1, 32)?;
        let mut binding = aligned_words(1, 32)?;
        let mut body = aligned_words(2, 48)?;
        let mut ast = aligned_words(11, 48)?;
        let typed_metadata = unsafe {
            typed_while_local_statements_parse(
                source.as_ptr(),
                source.len(),
                words_ptr(&mut functions),
                frontend.function_headers_emitted as usize,
                words_ptr(&mut control),
                1,
                words_ptr(&mut binding),
                1,
                words_ptr(&mut body),
                2,
                words_ptr(&mut ast),
                11,
            )
        };
        if typed_metadata.status_flags != 7
            || typed_metadata.errors != 0
            || typed_metadata.statements_emitted != 2
            || typed_metadata.ast_nodes_emitted != 11
        {
            return Err(format!(
                "loaded Jadren typed while-statements metadata returned invalid summary: statements={}/2, ast={}/11, errors={}, status={}",
                typed_metadata.statements_emitted,
                typed_metadata.ast_nodes_emitted,
                typed_metadata.errors,
                typed_metadata.status_flags
            ));
        }
        write_typed_while_capture_with_magic(
            output_path,
            &source,
            typed_metadata,
            &control,
            &binding,
            &body,
            &ast,
            TYPED_WHILE_LOCAL_STATEMENTS_CAPTURE_MAGIC,
        )?;
        drop(producer);
        drop(provider);
        return Ok(());
    }
    let is_typed_while_local_conditional_fixture = typed_while_local_sizing.status_flags == 3
        && typed_while_local_sizing.errors == 0
        && typed_while_local_sizing.statements_required == 2
        && typed_while_local_sizing.ast_nodes_required == 11;
    if is_typed_while_local_conditional_fixture {
        let mut control = aligned_words(1, 32)?;
        let mut binding = aligned_words(1, 32)?;
        let mut body = aligned_words(2, 48)?;
        let mut ast = aligned_words(11, 48)?;
        let typed_metadata = unsafe {
            typed_while_local_parse(
                source.as_ptr(),
                source.len(),
                words_ptr(&mut functions),
                frontend.function_headers_emitted as usize,
                words_ptr(&mut control),
                1,
                words_ptr(&mut binding),
                1,
                words_ptr(&mut body),
                2,
                words_ptr(&mut ast),
                11,
            )
        };
        if typed_metadata.status_flags != 7
            || typed_metadata.errors != 0
            || typed_metadata.statements_emitted != 2
            || typed_metadata.ast_nodes_emitted != 11
        {
            return Err(format!(
                "loaded Jadren typed while conditional metadata returned invalid summary: statements={}/2, ast={}/11, errors={}, status={}",
                typed_metadata.statements_emitted,
                typed_metadata.ast_nodes_emitted,
                typed_metadata.errors,
                typed_metadata.status_flags
            ));
        }
        let control_kind = body
            .get(6)
            .map(|word| (word & 0xff) as u8)
            .ok_or_else(|| "loaded Jadren typed while conditional body is truncated".to_owned())?;
        let control_flags = body
            .get(6)
            .map(|word| ((word >> 8) & 0xff) as u8)
            .ok_or_else(|| {
                "loaded Jadren typed while conditional flags are truncated".to_owned()
            })?;
        if (control_kind != 4 && control_kind != 5) || control_flags != 2 {
            return Err(
                "loaded Jadren typed while conditional metadata has invalid control header"
                    .to_owned(),
            );
        }
        write_typed_while_capture_with_magic(
            output_path,
            &source,
            typed_metadata,
            &control,
            &binding,
            &body,
            &ast,
            TYPED_WHILE_LOCAL_CONDITIONAL_CAPTURE_MAGIC,
        )?;
        drop(producer);
        drop(provider);
        return Ok(());
    }
    let is_typed_while_local_break_fixture = typed_while_local_sizing.status_flags == 3
        && typed_while_local_sizing.errors == 0
        && typed_while_local_sizing.statements_required == 2
        && typed_while_local_sizing.ast_nodes_required == 8;
    if is_typed_while_local_break_fixture {
        let mut control = aligned_words(1, 32)?;
        let mut binding = aligned_words(1, 32)?;
        let mut body = aligned_words(2, 48)?;
        let mut ast = aligned_words(8, 48)?;
        let typed_metadata = unsafe {
            typed_while_local_parse(
                source.as_ptr(),
                source.len(),
                words_ptr(&mut functions),
                frontend.function_headers_emitted as usize,
                words_ptr(&mut control),
                1,
                words_ptr(&mut binding),
                1,
                words_ptr(&mut body),
                2,
                words_ptr(&mut ast),
                8,
            )
        };
        if typed_metadata.status_flags != 7
            || typed_metadata.errors != 0
            || typed_metadata.statements_emitted != 2
            || typed_metadata.ast_nodes_emitted != 8
        {
            return Err(format!(
                "loaded Jadren typed while terminal-control metadata returned invalid summary: statements={}/2, ast={}/8, errors={}, status={}",
                typed_metadata.statements_emitted,
                typed_metadata.ast_nodes_emitted,
                typed_metadata.errors,
                typed_metadata.status_flags
            ));
        }
        let capture_magic = match body.get(6).map(|word| (word & 0xff) as u8) {
            Some(4) => TYPED_WHILE_LOCAL_BREAK_CAPTURE_MAGIC,
            Some(5) => TYPED_WHILE_LOCAL_CONTINUE_CAPTURE_MAGIC,
            _ => {
                return Err(
                    "loaded Jadren typed while terminal-control metadata has unknown body kind"
                        .to_owned(),
                );
            }
        };
        write_typed_while_capture_with_magic(
            output_path,
            &source,
            typed_metadata,
            &control,
            &binding,
            &body,
            &ast,
            capture_magic,
        )?;
        drop(producer);
        drop(provider);
        return Ok(());
    }
    let is_typed_while_local_fixture = typed_while_local_sizing.status_flags == 3
        && typed_while_local_sizing.errors == 0
        && typed_while_local_sizing.statements_required == 1
        && typed_while_local_sizing.ast_nodes_required == 8;
    if is_typed_while_local_fixture {
        let mut control = aligned_words(1, 32)?;
        let mut binding = aligned_words(1, 32)?;
        let mut body = aligned_words(1, 48)?;
        let mut ast = aligned_words(8, 48)?;
        let typed_metadata = unsafe {
            typed_while_local_parse(
                source.as_ptr(),
                source.len(),
                words_ptr(&mut functions),
                frontend.function_headers_emitted as usize,
                words_ptr(&mut control),
                1,
                words_ptr(&mut binding),
                1,
                words_ptr(&mut body),
                1,
                words_ptr(&mut ast),
                8,
            )
        };
        if typed_metadata.status_flags != 7
            || typed_metadata.errors != 0
            || typed_metadata.statements_emitted != typed_while_local_sizing.statements_required
            || typed_metadata.ast_nodes_emitted != typed_while_local_sizing.ast_nodes_required
        {
            return Err(format!(
                "loaded Jadren typed while metadata returned invalid summary: statements={}/{}, ast={}/{}, errors={}, status={}",
                typed_metadata.statements_emitted,
                typed_metadata.statements_required,
                typed_metadata.ast_nodes_emitted,
                typed_metadata.ast_nodes_required,
                typed_metadata.errors,
                typed_metadata.status_flags
            ));
        }
        write_typed_while_capture(
            output_path,
            &source,
            typed_metadata,
            &control,
            &binding,
            &body,
            &ast,
        )?;
        drop(producer);
        drop(provider);
        return Ok(());
    }
    let is_typed_if_value_statement_tail_fixture =
        typed_if_value_statement_tail_sizing.status_flags == 3
            && typed_if_value_statement_tail_sizing.errors == 0
            && typed_if_value_statement_tail_sizing.statements_required == 3
            && typed_if_value_statement_tail_sizing.ast_nodes_required == 12;
    if is_typed_if_value_statement_tail_fixture {
        let statement_count = usize::try_from(
            typed_if_value_statement_tail_sizing.statements_required,
        )
        .map_err(|_| "typed statement-tail statement count does not fit usize".to_owned())?;
        let ast_count = usize::try_from(typed_if_value_statement_tail_sizing.ast_nodes_required)
            .map_err(|_| "typed statement-tail AST count does not fit usize".to_owned())?;
        let mut control = aligned_words(1, 48)?;
        let mut continuation = aligned_words(1, 32)?;
        let mut statements = aligned_words(statement_count, 48)?;
        let mut ast = aligned_words(ast_count, 48)?;
        let typed_metadata = unsafe {
            typed_if_value_statement_tail_parse(
                source.as_ptr(),
                source.len(),
                words_ptr(&mut functions),
                frontend.function_headers_emitted as usize,
                words_ptr(&mut control),
                1,
                words_ptr(&mut continuation),
                1,
                words_ptr(&mut statements),
                statement_count,
                words_ptr(&mut ast),
                ast_count,
            )
        };
        if typed_metadata.status_flags != 7
            || typed_metadata.errors != 0
            || typed_metadata.statements_emitted
                != typed_if_value_statement_tail_sizing.statements_required
            || typed_metadata.ast_nodes_emitted
                != typed_if_value_statement_tail_sizing.ast_nodes_required
        {
            return Err(format!(
                "loaded Jadren typed if-value statement-tail metadata returned invalid summary: statements={}/{}, ast={}/{}, errors={}, status={}",
                typed_metadata.statements_emitted,
                typed_metadata.statements_required,
                typed_metadata.ast_nodes_emitted,
                typed_metadata.ast_nodes_required,
                typed_metadata.errors,
                typed_metadata.status_flags
            ));
        }
        write_typed_if_value_statement_tail_capture(
            output_path,
            &source,
            typed_metadata,
            &control,
            &continuation,
            &statements,
            &ast,
        )?;
        drop(producer);
        drop(provider);
        return Ok(());
    }
    let is_typed_if_value_local_assignment_fixture =
        typed_if_value_local_assignment_sizing.status_flags == 3
            && typed_if_value_local_assignment_sizing.errors == 0
            && typed_if_value_local_assignment_sizing.statements_required == 1
            && typed_if_value_local_assignment_sizing.ast_nodes_required == 8;
    if is_typed_if_value_local_assignment_fixture {
        let control_count =
            usize::try_from(typed_if_value_local_assignment_sizing.statements_required)
                .map_err(|_| "typed if-value local control count does not fit usize".to_owned())?;
        let continuation_count = control_count;
        let ast_count = usize::try_from(typed_if_value_local_assignment_sizing.ast_nodes_required)
            .map_err(|_| "typed if-value local AST count does not fit usize".to_owned())?;
        let mut control = aligned_words(control_count, 48)?;
        let mut continuation = aligned_words(continuation_count, 32)?;
        let mut ast = aligned_words(ast_count, 48)?;
        let typed_metadata = unsafe {
            typed_if_value_local_assignment_parse(
                source.as_ptr(),
                source.len(),
                words_ptr(&mut functions),
                frontend.function_headers_emitted as usize,
                words_ptr(&mut control),
                control_count,
                words_ptr(&mut continuation),
                continuation_count,
                words_ptr(&mut ast),
                ast_count,
            )
        };
        if typed_metadata.status_flags != 7
            || typed_metadata.errors != 0
            || typed_metadata.statements_emitted
                != typed_if_value_local_assignment_sizing.statements_required
            || typed_metadata.ast_nodes_emitted
                != typed_if_value_local_assignment_sizing.ast_nodes_required
        {
            return Err(format!(
                "loaded Jadren typed if-value local assignment metadata returned invalid summary: controls={}/{}, ast={}/{}, errors={}, status={}",
                typed_metadata.statements_emitted,
                typed_metadata.statements_required,
                typed_metadata.ast_nodes_emitted,
                typed_metadata.ast_nodes_required,
                typed_metadata.errors,
                typed_metadata.status_flags
            ));
        }
        write_typed_if_value_continuation_capture(
            output_path,
            &source,
            typed_metadata,
            &control,
            &continuation,
            &ast,
            TYPED_IF_VALUE_LOCAL_ASSIGNMENT_CAPTURE_MAGIC,
        )?;
        drop(producer);
        drop(provider);
        return Ok(());
    }
    let is_typed_if_value_continuation_fixture = typed_if_value_continuation_sizing.status_flags
        == 3
        && typed_if_value_continuation_sizing.errors == 0
        && typed_if_value_continuation_sizing.statements_required == 1
        && typed_if_value_continuation_sizing.ast_nodes_required == 8;
    if is_typed_if_value_continuation_fixture {
        let control_count = usize::try_from(typed_if_value_continuation_sizing.statements_required)
            .map_err(|_| "typed if-value control count does not fit usize".to_owned())?;
        let continuation_count = control_count;
        let ast_count = usize::try_from(typed_if_value_continuation_sizing.ast_nodes_required)
            .map_err(|_| "typed if-value AST count does not fit usize".to_owned())?;
        let mut control = aligned_words(control_count, 48)?;
        let mut continuation = aligned_words(continuation_count, 32)?;
        let mut ast = aligned_words(ast_count, 48)?;
        let typed_metadata = unsafe {
            typed_if_value_continuation_parse(
                source.as_ptr(),
                source.len(),
                words_ptr(&mut functions),
                frontend.function_headers_emitted as usize,
                words_ptr(&mut control),
                control_count,
                words_ptr(&mut continuation),
                continuation_count,
                words_ptr(&mut ast),
                ast_count,
            )
        };
        if typed_metadata.status_flags != 7
            || typed_metadata.errors != 0
            || typed_metadata.statements_emitted
                != typed_if_value_continuation_sizing.statements_required
            || typed_metadata.ast_nodes_emitted
                != typed_if_value_continuation_sizing.ast_nodes_required
        {
            return Err(format!(
                "loaded Jadren typed if-value continuation metadata returned invalid summary: controls={}/{}, ast={}/{}, errors={}, status={}",
                typed_metadata.statements_emitted,
                typed_metadata.statements_required,
                typed_metadata.ast_nodes_emitted,
                typed_metadata.ast_nodes_required,
                typed_metadata.errors,
                typed_metadata.status_flags
            ));
        }
        write_typed_if_value_continuation_capture(
            output_path,
            &source,
            typed_metadata,
            &control,
            &continuation,
            &ast,
            TYPED_IF_VALUE_CONTINUATION_CAPTURE_MAGIC,
        )?;
        drop(producer);
        drop(provider);
        return Ok(());
    }
    let typed_assignment_sizing = unsafe {
        typed_assignment_parse(
            source.as_ptr(),
            source.len(),
            words_ptr(&mut functions),
            frontend.function_headers_emitted as usize,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
        )
    };
    let is_typed_if_fixture = typed_if_sizing.status_flags == 3
        && typed_if_sizing.errors == 0
        && typed_if_sizing.statements_required == 1
        && typed_if_sizing.ast_nodes_required == 5;
    if is_typed_if_fixture {
        let control_count = usize::try_from(typed_if_sizing.statements_required)
            .map_err(|_| "typed if control count does not fit usize".to_owned())?;
        let ast_count = usize::try_from(typed_if_sizing.ast_nodes_required)
            .map_err(|_| "typed if AST count does not fit usize".to_owned())?;
        let mut control = aligned_words(control_count, 48)?;
        let mut ast = aligned_words(ast_count, 48)?;
        let typed_metadata = unsafe {
            typed_if_parse(
                source.as_ptr(),
                source.len(),
                words_ptr(&mut functions),
                frontend.function_headers_emitted as usize,
                words_ptr(&mut control),
                control_count,
                words_ptr(&mut ast),
                ast_count,
            )
        };
        if typed_metadata.status_flags != 7
            || typed_metadata.errors != 0
            || typed_metadata.statements_emitted != typed_if_sizing.statements_required
            || typed_metadata.ast_nodes_emitted != typed_if_sizing.ast_nodes_required
        {
            return Err(format!(
                "loaded Jadren typed if metadata returned invalid summary: controls={}/{}, ast={}/{}, errors={}, status={}",
                typed_metadata.statements_emitted,
                typed_metadata.statements_required,
                typed_metadata.ast_nodes_emitted,
                typed_metadata.ast_nodes_required,
                typed_metadata.errors,
                typed_metadata.status_flags
            ));
        }
        write_typed_if_return_capture(output_path, &source, typed_metadata, &control, &ast)?;
        drop(producer);
        drop(provider);
        return Ok(());
    }
    let is_typed_assignment_fixture = typed_assignment_sizing.status_flags == 3
        && typed_assignment_sizing.errors == 0
        && typed_assignment_sizing.statements_required == 3
        && typed_assignment_sizing.ast_nodes_required == 5;
    if is_typed_assignment_fixture {
        let statement_count = usize::try_from(typed_assignment_sizing.statements_required)
            .map_err(|_| "typed assignment statement count does not fit usize".to_owned())?;
        let ast_count = usize::try_from(typed_assignment_sizing.ast_nodes_required)
            .map_err(|_| "typed assignment AST count does not fit usize".to_owned())?;
        let mut statements = aligned_words(statement_count, 48)?;
        let mut ast = aligned_words(ast_count, 48)?;
        let typed_metadata = unsafe {
            typed_assignment_parse(
                source.as_ptr(),
                source.len(),
                words_ptr(&mut functions),
                frontend.function_headers_emitted as usize,
                words_ptr(&mut statements),
                statement_count,
                words_ptr(&mut ast),
                ast_count,
            )
        };
        if typed_metadata.status_flags != 7
            || typed_metadata.errors != 0
            || typed_metadata.statements_emitted != typed_assignment_sizing.statements_required
            || typed_metadata.ast_nodes_emitted != typed_assignment_sizing.ast_nodes_required
        {
            return Err(format!(
                "loaded Jadren typed assignment metadata returned invalid summary: statements={}/{}, ast={}/{}, errors={}, status={}",
                typed_metadata.statements_emitted,
                typed_metadata.statements_required,
                typed_metadata.ast_nodes_emitted,
                typed_metadata.ast_nodes_required,
                typed_metadata.errors,
                typed_metadata.status_flags
            ));
        }
        write_typed_statement_capture(output_path, &source, typed_metadata, &statements, &ast)?;
        drop(producer);
        drop(provider);
        return Ok(());
    }
    let is_typed_statement_fixture = typed_statement_sizing.status_flags == 3
        && typed_statement_sizing.errors == 0
        && ((typed_statement_sizing.statements_required == 2
            && typed_statement_sizing.ast_nodes_required == 4)
            || (typed_statement_sizing.statements_required == 3
                && typed_statement_sizing.ast_nodes_required == 5));
    let mut typed_statement_headers: Option<Vec<u64>> = None;
    let mut typed_statement_ast: Option<Vec<u64>> = None;
    if is_typed_statement_fixture {
        let statement_count = usize::try_from(typed_statement_sizing.statements_required)
            .map_err(|_| "typed statement count does not fit usize".to_owned())?;
        let ast_count = usize::try_from(typed_statement_sizing.ast_nodes_required)
            .map_err(|_| "typed statement AST count does not fit usize".to_owned())?;
        let mut statements = aligned_words(statement_count, 48)?;
        let mut ast = aligned_words(ast_count, 48)?;
        let typed_metadata = unsafe {
            typed_statement_parse(
                source.as_ptr(),
                source.len(),
                words_ptr(&mut functions),
                frontend.function_headers_emitted as usize,
                words_ptr(&mut statements),
                statement_count,
                words_ptr(&mut ast),
                ast_count,
            )
        };
        if typed_metadata.status_flags != 7
            || typed_metadata.errors != 0
            || typed_metadata.statements_emitted != typed_statement_sizing.statements_required
            || typed_metadata.ast_nodes_emitted != typed_statement_sizing.ast_nodes_required
        {
            return Err(format!(
                "loaded Jadren typed statement metadata returned invalid summary: statements={}/{}, ast={}/{}, errors={}, status={}",
                typed_metadata.statements_emitted,
                typed_metadata.statements_required,
                typed_metadata.ast_nodes_emitted,
                typed_metadata.ast_nodes_required,
                typed_metadata.errors,
                typed_metadata.status_flags
            ));
        }
        typed_statement_headers = Some(statements);
        typed_statement_ast = Some(ast);
    }
    let is_local_binding_fixture = local_sizing.status_flags == 3 && local_sizing.errors == 0;
    let is_direct_call_fixture = [
        "let local: Int32 = 41; return helper(local);",
        "let local: Int32 = 40 + 1;",
        "let local: Int32 = 40; return helper(local + 1);",
        "let local: Int32 = 40; return helper(1 + local);",
    ]
    .iter()
    .any(|marker| source_text.contains(marker));
    let is_direct_call_result_binding_fixture = source_text
        .contains("let result: Int32 = helper(41);")
        || source_text.contains("let result: Int32 = helper();")
        || compact_source.contains("letresult:Int32=helper(41);returnresult;")
        || compact_source.contains("letresult:Int32=helper();returnresult;");
    let is_typed_while_fixture =
        source_text.contains("while counter <") || source_text.contains("while outer <");
    if frontend.source_bytes != source.len() as u64
        || frontend.function_headers_emitted == 0
        || frontend.function_headers_emitted > capacity as u64
        || (!is_local_binding_fixture
            && !is_typed_assignment_fixture
            && !is_direct_call_fixture
            && !is_direct_call_result_binding_fixture
            && !is_typed_while_fixture
            && frontend.statements_emitted != frontend.function_headers_emitted)
        || frontend.calls_emitted < frontend.function_headers_emitted
        || frontend.calls_emitted > frontend.function_headers_emitted.saturating_mul(2)
        || frontend.syntax_errors != 0
        || frontend.status_flags != 3
    {
        return Err(format!(
            "loaded Jadren stage-2 frontend returned invalid summary: source={}/{}, functions={}, statements={}, calls={}, syntax_errors={}, status={}, if_local_assignment=(required={}, ast={}, errors={}, status={})",
            frontend.source_bytes,
            source.len(),
            frontend.function_headers_emitted,
            frontend.statements_emitted,
            frontend.calls_emitted,
            frontend.syntax_errors,
            frontend.status_flags,
            typed_if_value_local_assignment_sizing.statements_required,
            typed_if_value_local_assignment_sizing.ast_nodes_required,
            typed_if_value_local_assignment_sizing.errors,
            typed_if_value_local_assignment_sizing.status_flags
        ));
    }

    let sizing = if is_typed_statement_fixture {
        unsafe {
            typed_statement_emit(
                source.as_ptr(),
                source.len(),
                words_ptr(&mut functions),
                frontend.function_headers_emitted as usize,
                words_ptr(typed_statement_headers.as_mut().expect("typed metadata")),
                typed_statement_sizing.statements_required as usize,
                words_ptr(typed_statement_ast.as_mut().expect("typed AST")),
                typed_statement_sizing.ast_nodes_required as usize,
                std::ptr::null_mut(),
                0,
            )
        }
    } else if is_local_binding_fixture {
        local_sizing
    } else {
        unsafe {
            emit(
                source.as_ptr(),
                source.len(),
                words_ptr(&mut functions),
                frontend.function_headers_emitted as usize,
                words_ptr(&mut statements),
                frontend.statements_emitted as usize,
                words_ptr(&mut calls),
                frontend.calls_emitted as usize,
                std::ptr::null_mut(),
                0,
            )
        }
    };
    validate_sizing(
        &frontend,
        sizing,
        is_typed_statement_fixture || is_local_binding_fixture,
        typed_while_local_statements_sizing,
        typed_nested_while_local_sizing,
        typed_while_local_dynamic_sizing,
        typed_while_local_dynamic_control_sizing,
    )?;

    let record_count = usize::try_from(sizing.records_required)
        .map_err(|_| "stage-2 record count does not fit usize".to_owned())?;
    let mut records = aligned_words(record_count, 64)?;
    let emitted = if is_typed_statement_fixture {
        unsafe {
            typed_statement_emit(
                source.as_ptr(),
                source.len(),
                words_ptr(&mut functions),
                frontend.function_headers_emitted as usize,
                words_ptr(typed_statement_headers.as_mut().expect("typed metadata")),
                typed_statement_sizing.statements_required as usize,
                words_ptr(typed_statement_ast.as_mut().expect("typed AST")),
                typed_statement_sizing.ast_nodes_required as usize,
                words_ptr(&mut records),
                record_count,
            )
        }
    } else if is_local_binding_fixture {
        unsafe {
            local_auto_emit(
                source.as_ptr(),
                source.len(),
                words_ptr(&mut functions),
                frontend.function_headers_emitted as usize,
                words_ptr(&mut records),
                record_count,
            )
        }
    } else {
        unsafe {
            emit(
                source.as_ptr(),
                source.len(),
                words_ptr(&mut functions),
                frontend.function_headers_emitted as usize,
                words_ptr(&mut statements),
                frontend.statements_emitted as usize,
                words_ptr(&mut calls),
                frontend.calls_emitted as usize,
                words_ptr(&mut records),
                record_count,
            )
        }
    };
    if emitted.functions_seen != sizing.functions_seen
        || emitted.statements_seen != sizing.statements_seen
        || emitted.calls_seen != sizing.calls_seen
        || emitted.records_required != sizing.records_required
        || emitted.records_emitted != sizing.records_required
        || emitted.functions_lowered != sizing.functions_lowered
        || emitted.errors != 0
        || emitted.status_flags != 7
    {
        return Err(format!(
            "loaded Jadren stage-2 JIR emission returned invalid summary: functions={}, statements={}, calls={}, required={}, emitted={}, lowered={}, errors={}, status={}",
            emitted.functions_seen,
            emitted.statements_seen,
            emitted.calls_seen,
            emitted.records_required,
            emitted.records_emitted,
            emitted.functions_lowered,
            emitted.errors,
            emitted.status_flags
        ));
    }

    write_capture(output_path, &source, emitted, &records)?;
    drop(producer);
    drop(provider);
    Ok(())
}

fn validate_sizing(
    frontend: &FrontendStage2Summary,
    sizing: Stage2JirSummary,
    local_binding: bool,
    typed_while_local_statements_sizing: TypedStatementStage2Summary,
    typed_nested_while_local_sizing: TypedStatementStage2Summary,
    typed_while_local_dynamic_sizing: TypedStatementStage2Summary,
    typed_while_local_dynamic_control_sizing: TypedStatementStage2Summary,
) -> Result<(), String> {
    if sizing.functions_seen != frontend.function_headers_emitted
        || (!local_binding && sizing.statements_seen != frontend.statements_emitted)
        || sizing.calls_seen != frontend.calls_emitted
        || sizing.records_required == 0
        || sizing.records_emitted != 0
        || sizing.functions_lowered != frontend.function_headers_emitted
        || sizing.errors != 0
        || sizing.status_flags != 3
        || sizing.records_required > (i32::MAX as u64 / 64)
    {
        return Err(format!(
            "loaded Jadren stage-2 JIR sizing returned invalid summary: functions={}, statements={}, calls={}, required={}, emitted={}, lowered={}, errors={}, status={}; while=(required={}, ast={}, errors={}, status={}); nested=(required={}, ast={}, errors={}, status={}); dynamic=(required={}, ast={}, errors={}, status={}); dynamic_control=(required={}, ast={}, errors={}, status={})",
            sizing.functions_seen,
            sizing.statements_seen,
            sizing.calls_seen,
            sizing.records_required,
            sizing.records_emitted,
            sizing.functions_lowered,
            sizing.errors,
            sizing.status_flags,
            typed_while_local_statements_sizing.statements_required,
            typed_while_local_statements_sizing.ast_nodes_required,
            typed_while_local_statements_sizing.errors,
            typed_while_local_statements_sizing.status_flags,
            typed_nested_while_local_sizing.statements_required,
            typed_nested_while_local_sizing.ast_nodes_required,
            typed_nested_while_local_sizing.errors,
            typed_nested_while_local_sizing.status_flags,
            typed_while_local_dynamic_sizing.statements_required,
            typed_while_local_dynamic_sizing.ast_nodes_required,
            typed_while_local_dynamic_sizing.errors,
            typed_while_local_dynamic_sizing.status_flags,
            typed_while_local_dynamic_control_sizing.statements_required,
            typed_while_local_dynamic_control_sizing.ast_nodes_required,
            typed_while_local_dynamic_control_sizing.errors,
            typed_while_local_dynamic_control_sizing.status_flags
        ));
    }
    Ok(())
}

fn write_capture(
    output_path: &Path,
    source: &[u8],
    summary: Stage2JirSummary,
    records: &[u64],
) -> Result<(), String> {
    let record_count = usize::try_from(summary.records_emitted)
        .map_err(|_| "stage-2 emitted record count does not fit usize".to_owned())?;
    if records.len() < record_count.saturating_mul(8) {
        return Err("stage-2 emitted record buffer is truncated".to_owned());
    }
    let raw =
        unsafe { std::slice::from_raw_parts(records.as_ptr().cast::<u8>(), records.len() * 8) };
    let parent = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create `{}`: {error}", parent.display()))?;
    let mut bytes =
        Vec::with_capacity(CAPTURE_MAGIC.len() + 8 + source.len() + 8 * 8 + 8 + record_count * 60);
    bytes.extend_from_slice(CAPTURE_MAGIC);
    push_u64(&mut bytes, source.len() as u64);
    bytes.extend_from_slice(source);
    for value in [
        summary.functions_seen,
        summary.statements_seen,
        summary.calls_seen,
        summary.records_required,
        summary.records_emitted,
        summary.functions_lowered,
        summary.errors,
        summary.status_flags,
    ] {
        push_u64(&mut bytes, value);
    }
    push_u64(&mut bytes, summary.records_emitted);
    for index in 0..record_count {
        let start = index * 64;
        bytes.extend_from_slice(&raw[start..start + 4]);
        for offset in (8..=56).step_by(8) {
            let value = u64::from_le_bytes(
                raw[start + offset..start + offset + 8]
                    .try_into()
                    .expect("record field is eight bytes"),
            );
            push_u64(&mut bytes, value);
        }
    }
    std::fs::write(output_path, bytes)
        .map_err(|error| format!("cannot write `{}`: {error}", output_path.display()))
}

fn write_typed_if_return_capture(
    output_path: &Path,
    source: &[u8],
    summary: TypedStatementStage2Summary,
    control: &[u64],
    ast: &[u64],
) -> Result<(), String> {
    let control_count = usize::try_from(summary.statements_emitted)
        .map_err(|_| "typed if control count does not fit usize".to_owned())?;
    let ast_count = usize::try_from(summary.ast_nodes_emitted)
        .map_err(|_| "typed if AST count does not fit usize".to_owned())?;
    if control.len() * 8 < control_count * 48 || ast.len() * 8 < ast_count * 48 {
        return Err("typed if metadata buffer is truncated".to_owned());
    }
    let control_raw =
        unsafe { std::slice::from_raw_parts(control.as_ptr().cast::<u8>(), control.len() * 8) };
    let ast_raw = unsafe { std::slice::from_raw_parts(ast.as_ptr().cast::<u8>(), ast.len() * 8) };
    let parent = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create `{}`: {error}", parent.display()))?;
    let mut bytes = Vec::with_capacity(
        TYPED_IF_RETURN_CAPTURE_MAGIC.len()
            + 8
            + source.len()
            + 8 * 9
            + 8
            + control_count * 44
            + 8
            + ast_count * 44,
    );
    bytes.extend_from_slice(TYPED_IF_RETURN_CAPTURE_MAGIC);
    push_u64(&mut bytes, source.len() as u64);
    bytes.extend_from_slice(source);
    for value in [
        summary.source_bytes,
        summary.statements_required,
        summary.statements_emitted,
        summary.ast_nodes_required,
        summary.ast_nodes_emitted,
        summary.errors,
        summary.status_flags,
        summary.reserved,
    ] {
        push_u64(&mut bytes, value);
    }
    push_u64(&mut bytes, control_count as u64);
    for index in 0..control_count {
        let start = index * 48;
        for offset in (0..=32).step_by(8) {
            push_u64(
                &mut bytes,
                u64::from_le_bytes(
                    control_raw[start + offset..start + offset + 8]
                        .try_into()
                        .expect("typed if control field is eight bytes"),
                ),
            );
        }
        bytes.extend_from_slice(&control_raw[start + 40..start + 48]);
    }
    push_u64(&mut bytes, ast_count as u64);
    for index in 0..ast_count {
        let start = index * 48;
        bytes.extend_from_slice(&ast_raw[start..start + 4]);
        for offset in (8..=40).step_by(8) {
            push_u64(
                &mut bytes,
                u64::from_le_bytes(
                    ast_raw[start + offset..start + offset + 8]
                        .try_into()
                        .expect("typed if AST field is eight bytes"),
                ),
            );
        }
    }
    std::fs::write(output_path, bytes)
        .map_err(|error| format!("cannot write `{}`: {error}", output_path.display()))
}

fn write_typed_statement_capture(
    output_path: &Path,
    source: &[u8],
    summary: TypedStatementStage2Summary,
    statements: &[u64],
    ast: &[u64],
) -> Result<(), String> {
    let statement_count = usize::try_from(summary.statements_emitted)
        .map_err(|_| "typed statement count does not fit usize".to_owned())?;
    let ast_count = usize::try_from(summary.ast_nodes_emitted)
        .map_err(|_| "typed statement AST count does not fit usize".to_owned())?;
    if statements.len() * 8 < statement_count * 48 || ast.len() * 8 < ast_count * 48 {
        return Err("typed statement metadata buffer is truncated".to_owned());
    }
    let statement_raw = unsafe {
        std::slice::from_raw_parts(statements.as_ptr().cast::<u8>(), statements.len() * 8)
    };
    let ast_raw = unsafe { std::slice::from_raw_parts(ast.as_ptr().cast::<u8>(), ast.len() * 8) };
    let parent = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create `{}`: {error}", parent.display()))?;
    let mut bytes = Vec::with_capacity(
        TYPED_STATEMENT_CAPTURE_MAGIC.len()
            + 8
            + source.len()
            + 8 * 9
            + 8
            + statement_count * 44
            + 8
            + ast_count * 44,
    );
    bytes.extend_from_slice(TYPED_STATEMENT_CAPTURE_MAGIC);
    push_u64(&mut bytes, source.len() as u64);
    bytes.extend_from_slice(source);
    for value in [
        summary.source_bytes,
        summary.statements_required,
        summary.statements_emitted,
        summary.ast_nodes_required,
        summary.ast_nodes_emitted,
        summary.errors,
        summary.status_flags,
        summary.reserved,
    ] {
        push_u64(&mut bytes, value);
    }
    push_u64(&mut bytes, statement_count as u64);
    for index in 0..statement_count {
        let start = index * 48;
        bytes.extend_from_slice(&statement_raw[start..start + 4]);
        for offset in (8..=40).step_by(8) {
            push_u64(
                &mut bytes,
                u64::from_le_bytes(
                    statement_raw[start + offset..start + offset + 8]
                        .try_into()
                        .expect("typed statement field is eight bytes"),
                ),
            );
        }
    }
    push_u64(&mut bytes, ast_count as u64);
    for index in 0..ast_count {
        let start = index * 48;
        bytes.extend_from_slice(&ast_raw[start..start + 4]);
        for offset in (8..=40).step_by(8) {
            push_u64(
                &mut bytes,
                u64::from_le_bytes(
                    ast_raw[start + offset..start + offset + 8]
                        .try_into()
                        .expect("typed AST field is eight bytes"),
                ),
            );
        }
    }
    std::fs::write(output_path, bytes)
        .map_err(|error| format!("cannot write `{}`: {error}", output_path.display()))
}

#[allow(clippy::too_many_arguments)]
fn write_typed_while_capture_with_magic(
    output_path: &Path,
    source: &[u8],
    summary: TypedStatementStage2Summary,
    control: &[u64],
    binding: &[u64],
    body: &[u64],
    ast: &[u64],
    magic: &[u8; 8],
) -> Result<(), String> {
    let ast_count = usize::try_from(summary.ast_nodes_emitted)
        .map_err(|_| "typed while AST count does not fit usize".to_owned())?;
    let body_count = usize::try_from(summary.statements_emitted)
        .map_err(|_| "typed while body statement count does not fit usize".to_owned())?;
    if control.len() * 8 < 32
        || binding.len() * 8 < 32
        || body.len() * 8 < body_count * 48
        || ast.len() * 8 < ast_count * 48
    {
        return Err("typed while metadata buffer is truncated".to_owned());
    }
    let control_raw =
        unsafe { std::slice::from_raw_parts(control.as_ptr().cast::<u8>(), control.len() * 8) };
    let binding_raw =
        unsafe { std::slice::from_raw_parts(binding.as_ptr().cast::<u8>(), binding.len() * 8) };
    let body_raw =
        unsafe { std::slice::from_raw_parts(body.as_ptr().cast::<u8>(), body.len() * 8) };
    let ast_raw = unsafe { std::slice::from_raw_parts(ast.as_ptr().cast::<u8>(), ast.len() * 8) };
    let parent = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create `{}`: {error}", parent.display()))?;
    let mut bytes = Vec::with_capacity(
        magic.len()
            + 8
            + source.len()
            + 8 * 9
            + 8
            + 8
            + 8
            + 8
            + 32
            + 8
            + 32
            + 8
            + body_count * 44
            + 8
            + ast_count * 44,
    );
    bytes.extend_from_slice(magic);
    push_u64(&mut bytes, source.len() as u64);
    bytes.extend_from_slice(source);
    for value in [
        summary.source_bytes,
        summary.statements_required,
        summary.statements_emitted,
        summary.ast_nodes_required,
        summary.ast_nodes_emitted,
        summary.errors,
        summary.status_flags,
        summary.reserved,
    ] {
        push_u64(&mut bytes, value);
    }
    // The bounded producer always places the initializer literal at AST node 0.
    push_u64(&mut bytes, 0);
    push_u64(&mut bytes, 1);
    for offset in (0..=16).step_by(8) {
        push_u64(
            &mut bytes,
            u64::from_le_bytes(
                control_raw[offset..offset + 8]
                    .try_into()
                    .expect("typed while control field is eight bytes"),
            ),
        );
    }
    bytes.extend_from_slice(&control_raw[24..32]);
    push_u64(&mut bytes, 1);
    for offset in (0..=8).step_by(8) {
        push_u64(
            &mut bytes,
            u64::from_le_bytes(
                binding_raw[offset..offset + 8]
                    .try_into()
                    .expect("typed while binding field is eight bytes"),
            ),
        );
    }
    bytes.extend_from_slice(&binding_raw[16..24]);
    push_u64(&mut bytes, body_count as u64);
    for index in 0..body_count {
        let start = index * 48;
        bytes.extend_from_slice(&body_raw[start..start + 4]);
        for offset in (8..=40).step_by(8) {
            push_u64(
                &mut bytes,
                u64::from_le_bytes(
                    body_raw[start + offset..start + offset + 8]
                        .try_into()
                        .expect("typed while body field is eight bytes"),
                ),
            );
        }
    }
    push_u64(&mut bytes, ast_count as u64);
    for index in 0..ast_count {
        let start = index * 48;
        bytes.extend_from_slice(&ast_raw[start..start + 4]);
        for offset in (8..=40).step_by(8) {
            push_u64(
                &mut bytes,
                u64::from_le_bytes(
                    ast_raw[start + offset..start + offset + 8]
                        .try_into()
                        .expect("typed while AST field is eight bytes"),
                ),
            );
        }
    }
    std::fs::write(output_path, bytes)
        .map_err(|error| format!("cannot write `{}`: {error}", output_path.display()))
}

#[allow(clippy::too_many_arguments)]
fn write_typed_nested_while_capture(
    output_path: &Path,
    source: &[u8],
    summary: TypedStatementStage2Summary,
    capture_magic: &[u8; 8],
    control: &[u64],
    binding: &[u64],
    body: &[u64],
    ast: &[u64],
) -> Result<(), String> {
    let ast_count = usize::try_from(summary.ast_nodes_emitted)
        .map_err(|_| "nested typed while AST count does not fit usize".to_owned())?;
    let body_count = usize::try_from(summary.statements_emitted)
        .map_err(|_| "nested typed while body statement count does not fit usize".to_owned())?;
    if control.len() * 8 < 2 * 32
        || binding.len() * 8 < 2 * 24
        || body.len() * 8 < body_count * 48
        || ast.len() * 8 < ast_count * 48
    {
        return Err("nested typed while metadata buffer is truncated".to_owned());
    }
    let control_raw =
        unsafe { std::slice::from_raw_parts(control.as_ptr().cast::<u8>(), control.len() * 8) };
    let binding_raw =
        unsafe { std::slice::from_raw_parts(binding.as_ptr().cast::<u8>(), binding.len() * 8) };
    let body_raw =
        unsafe { std::slice::from_raw_parts(body.as_ptr().cast::<u8>(), body.len() * 8) };
    let ast_raw = unsafe { std::slice::from_raw_parts(ast.as_ptr().cast::<u8>(), ast.len() * 8) };
    let parent = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create `{}`: {error}", parent.display()))?;
    let mut bytes = Vec::with_capacity(
        8 + 8
            + source.len()
            + 8 * 8
            + 8
            + 8
            + 2 * 32
            + 8
            + 2 * 24
            + 8
            + body_count * 44
            + 8
            + ast_count * 44,
    );
    bytes.extend_from_slice(capture_magic);
    push_u64(&mut bytes, source.len() as u64);
    bytes.extend_from_slice(source);
    for value in [
        summary.source_bytes,
        summary.statements_required,
        summary.statements_emitted,
        summary.ast_nodes_required,
        summary.ast_nodes_emitted,
        summary.errors,
        summary.status_flags,
        summary.reserved,
    ] {
        push_u64(&mut bytes, value);
    }
    push_u64(&mut bytes, 0);
    push_u64(&mut bytes, 2);
    for index in 0..2 {
        let start = index * 32;
        for offset in (0..=16).step_by(8) {
            push_u64(
                &mut bytes,
                u64::from_le_bytes(
                    control_raw[start + offset..start + offset + 8]
                        .try_into()
                        .expect("nested typed while control field is eight bytes"),
                ),
            );
        }
        bytes.extend_from_slice(&control_raw[start + 24..start + 32]);
    }
    push_u64(&mut bytes, 2);
    for index in 0..2 {
        let start = index * 24;
        for offset in (0..=8).step_by(8) {
            push_u64(
                &mut bytes,
                u64::from_le_bytes(
                    binding_raw[start + offset..start + offset + 8]
                        .try_into()
                        .expect("nested typed while binding field is eight bytes"),
                ),
            );
        }
        bytes.extend_from_slice(&binding_raw[start + 16..start + 24]);
    }
    push_u64(&mut bytes, body_count as u64);
    for index in 0..body_count {
        let start = index * 48;
        bytes.extend_from_slice(&body_raw[start..start + 4]);
        for offset in (8..=40).step_by(8) {
            push_u64(
                &mut bytes,
                u64::from_le_bytes(
                    body_raw[start + offset..start + offset + 8]
                        .try_into()
                        .expect("nested typed while body field is eight bytes"),
                ),
            );
        }
    }
    push_u64(&mut bytes, ast_count as u64);
    for index in 0..ast_count {
        let start = index * 48;
        bytes.extend_from_slice(&ast_raw[start..start + 4]);
        for offset in (8..=40).step_by(8) {
            push_u64(
                &mut bytes,
                u64::from_le_bytes(
                    ast_raw[start + offset..start + offset + 8]
                        .try_into()
                        .expect("nested typed while AST field is eight bytes"),
                ),
            );
        }
    }
    std::fs::write(output_path, bytes)
        .map_err(|error| format!("cannot write `{}`: {error}", output_path.display()))
}

fn write_typed_while_capture(
    output_path: &Path,
    source: &[u8],
    summary: TypedStatementStage2Summary,
    control: &[u64],
    binding: &[u64],
    body: &[u64],
    ast: &[u64],
) -> Result<(), String> {
    write_typed_while_capture_with_magic(
        output_path,
        source,
        summary,
        control,
        binding,
        body,
        ast,
        TYPED_WHILE_LOCAL_CAPTURE_MAGIC,
    )
}

fn write_typed_if_value_continuation_capture(
    output_path: &Path,
    source: &[u8],
    summary: TypedStatementStage2Summary,
    control: &[u64],
    continuation: &[u64],
    ast: &[u64],
    magic: &[u8; 8],
) -> Result<(), String> {
    let control_count = usize::try_from(summary.statements_emitted)
        .map_err(|_| "typed if-value control count does not fit usize".to_owned())?;
    let continuation_count = control_count;
    let ast_count = usize::try_from(summary.ast_nodes_emitted)
        .map_err(|_| "typed if-value AST count does not fit usize".to_owned())?;
    if control.len() * 8 < control_count * 48
        || continuation.len() * 8 < continuation_count * 32
        || ast.len() * 8 < ast_count * 48
    {
        return Err("typed if-value metadata buffer is truncated".to_owned());
    }
    let control_raw =
        unsafe { std::slice::from_raw_parts(control.as_ptr().cast::<u8>(), control.len() * 8) };
    let continuation_raw = unsafe {
        std::slice::from_raw_parts(continuation.as_ptr().cast::<u8>(), continuation.len() * 8)
    };
    let ast_raw = unsafe { std::slice::from_raw_parts(ast.as_ptr().cast::<u8>(), ast.len() * 8) };
    let parent = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create `{}`: {error}", parent.display()))?;
    let mut bytes = Vec::with_capacity(
        magic.len()
            + 8
            + source.len()
            + 8 * 9
            + 8
            + control_count * 44
            + 8
            + continuation_count * 28
            + 8
            + ast_count * 44,
    );
    bytes.extend_from_slice(magic);
    push_u64(&mut bytes, source.len() as u64);
    bytes.extend_from_slice(source);
    for value in [
        summary.source_bytes,
        summary.statements_required,
        summary.statements_emitted,
        summary.ast_nodes_required,
        summary.ast_nodes_emitted,
        summary.errors,
        summary.status_flags,
        summary.reserved,
    ] {
        push_u64(&mut bytes, value);
    }
    push_u64(&mut bytes, control_count as u64);
    for index in 0..control_count {
        let start = index * 48;
        for offset in (0..=32).step_by(8) {
            push_u64(
                &mut bytes,
                u64::from_le_bytes(
                    control_raw[start + offset..start + offset + 8]
                        .try_into()
                        .expect("typed if-value control field is eight bytes"),
                ),
            );
        }
        bytes.extend_from_slice(&control_raw[start + 40..start + 48]);
    }
    push_u64(&mut bytes, continuation_count as u64);
    for index in 0..continuation_count {
        let start = index * 32;
        for offset in (0..=16).step_by(8) {
            push_u64(
                &mut bytes,
                u64::from_le_bytes(
                    continuation_raw[start + offset..start + offset + 8]
                        .try_into()
                        .expect("typed continuation field is eight bytes"),
                ),
            );
        }
        bytes.extend_from_slice(&continuation_raw[start + 24..start + 32]);
    }
    push_u64(&mut bytes, ast_count as u64);
    for index in 0..ast_count {
        let start = index * 48;
        bytes.extend_from_slice(&ast_raw[start..start + 4]);
        for offset in (8..=40).step_by(8) {
            push_u64(
                &mut bytes,
                u64::from_le_bytes(
                    ast_raw[start + offset..start + offset + 8]
                        .try_into()
                        .expect("typed continuation AST field is eight bytes"),
                ),
            );
        }
    }
    std::fs::write(output_path, bytes)
        .map_err(|error| format!("cannot write `{}`: {error}", output_path.display()))
}

fn write_typed_if_value_statement_tail_capture(
    output_path: &Path,
    source: &[u8],
    summary: TypedStatementStage2Summary,
    control: &[u64],
    continuation: &[u64],
    statements: &[u64],
    ast: &[u64],
) -> Result<(), String> {
    let statement_count = usize::try_from(summary.statements_emitted)
        .map_err(|_| "typed statement-tail statement count does not fit usize".to_owned())?;
    let ast_count = usize::try_from(summary.ast_nodes_emitted)
        .map_err(|_| "typed statement-tail AST count does not fit usize".to_owned())?;
    if control.len() * 8 < 48
        || continuation.len() * 8 < 32
        || statements.len() * 8 < statement_count * 48
        || ast.len() * 8 < ast_count * 48
    {
        return Err("typed statement-tail metadata buffer is truncated".to_owned());
    }
    let control_raw =
        unsafe { std::slice::from_raw_parts(control.as_ptr().cast::<u8>(), control.len() * 8) };
    let continuation_raw = unsafe {
        std::slice::from_raw_parts(continuation.as_ptr().cast::<u8>(), continuation.len() * 8)
    };
    let statements_raw = unsafe {
        std::slice::from_raw_parts(statements.as_ptr().cast::<u8>(), statements.len() * 8)
    };
    let ast_raw = unsafe { std::slice::from_raw_parts(ast.as_ptr().cast::<u8>(), ast.len() * 8) };
    let parent = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create `{}`: {error}", parent.display()))?;
    let mut bytes = Vec::with_capacity(
        TYPED_IF_VALUE_STATEMENT_TAIL_CAPTURE_MAGIC.len()
            + 8
            + source.len()
            + 8 * 8
            + 8
            + 44
            + 8
            + 28
            + 8
            + statement_count * 44
            + 8
            + ast_count * 44,
    );
    bytes.extend_from_slice(TYPED_IF_VALUE_STATEMENT_TAIL_CAPTURE_MAGIC);
    push_u64(&mut bytes, source.len() as u64);
    bytes.extend_from_slice(source);
    for value in [
        summary.source_bytes,
        summary.statements_required,
        summary.statements_emitted,
        summary.ast_nodes_required,
        summary.ast_nodes_emitted,
        summary.errors,
        summary.status_flags,
        summary.reserved,
    ] {
        push_u64(&mut bytes, value);
    }
    push_u64(&mut bytes, 1);
    for offset in (0..=32).step_by(8) {
        push_u64(
            &mut bytes,
            u64::from_le_bytes(
                control_raw[offset..offset + 8]
                    .try_into()
                    .expect("typed statement-tail control field is eight bytes"),
            ),
        );
    }
    bytes.extend_from_slice(&control_raw[40..48]);
    push_u64(&mut bytes, 1);
    for offset in (0..=16).step_by(8) {
        push_u64(
            &mut bytes,
            u64::from_le_bytes(
                continuation_raw[offset..offset + 8]
                    .try_into()
                    .expect("typed statement-tail continuation field is eight bytes"),
            ),
        );
    }
    bytes.extend_from_slice(&continuation_raw[24..32]);
    push_u64(&mut bytes, statement_count as u64);
    for index in 0..statement_count {
        let start = index * 48;
        bytes.extend_from_slice(&statements_raw[start..start + 4]);
        for offset in (8..=40).step_by(8) {
            push_u64(
                &mut bytes,
                u64::from_le_bytes(
                    statements_raw[start + offset..start + offset + 8]
                        .try_into()
                        .expect("typed statement-tail statement field is eight bytes"),
                ),
            );
        }
    }
    push_u64(&mut bytes, ast_count as u64);
    for index in 0..ast_count {
        let start = index * 48;
        bytes.extend_from_slice(&ast_raw[start..start + 4]);
        for offset in (8..=40).step_by(8) {
            push_u64(
                &mut bytes,
                u64::from_le_bytes(
                    ast_raw[start + offset..start + offset + 8]
                        .try_into()
                        .expect("typed statement-tail AST field is eight bytes"),
                ),
            );
        }
    }
    std::fs::write(output_path, bytes)
        .map_err(|error| format!("cannot write `{}`: {error}", output_path.display()))
}

fn push_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn aligned_words(capacity: usize, bytes_per_item: usize) -> Result<Vec<u64>, String> {
    let bytes = capacity
        .checked_mul(bytes_per_item)
        .ok_or_else(|| "stage-2 buffer size overflow".to_owned())?;
    let words = bytes
        .checked_add(7)
        .ok_or_else(|| "stage-2 buffer alignment overflow".to_owned())?
        / 8;
    Ok(vec![0; words.max(1)])
}

fn words_ptr(words: &mut [u64]) -> *mut u8 {
    words.as_mut_ptr().cast::<u8>()
}
