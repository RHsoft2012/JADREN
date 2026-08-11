use std::collections::{BTreeMap, BTreeSet};

use jadren_lexer::Operator;
use jadren_mir::{
    BasicBlockId as MirBlockId, CarrierPart, MirFunction, MirModule, MirOperand, MirOperandKind,
    MirPattern, MirPropagationKind, MirStatement, Place, Projection, Terminator as MirTerminator,
};
use jadren_resolve::SymbolId;
use jadren_source::Span;
use jadren_types::{
    Capability, CarrierTag, FloatWidth, IntegerWidth, NominalLayout, NominalLayoutKind,
    NominalTypeId, Signedness, Substitution, TypeId as SemanticTypeId, TypeKind, TypeStore,
};

use crate::{
    AddressSpace, BinaryOp, Block, BlockId, CarrierDropBranch, CarrierDropField, CastOp,
    ComparePredicate, Constant, Function, FunctionId, Instruction, InstructionKind, Linkage,
    Module, Parameter, RecordDropField, Terminator, Type, TypeId, TypedValue, UnaryOp, ValueId,
};

/// Target choices needed while converting target-dependent semantic types.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LowerOptions {
    /// Width used for `IntSize` and `UIntSize` until target layout is a first-class JIR input.
    pub pointer_bits: u16,
}

impl Default for LowerOptions {
    fn default() -> Self {
        Self { pointer_bits: 64 }
    }
}

/// One explicit reason why valid MIR cannot yet be represented by JIR 0.1.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LowerError {
    pub span: Option<Span>,
    pub message: String,
}

/// Lowers the scalar/core subset of verified place-based MIR into deterministic JIR SSA.
///
/// Mutable MIR locals are represented by entry-block stack slots. This deliberately avoids
/// inventing phi values before the later mem2reg optimization while all instruction results,
/// parameters, and control-flow conditions remain SSA values.
pub fn lower_from_mir(
    mir: &MirModule,
    types: &TypeStore,
    options: LowerOptions,
) -> Result<Module, Vec<LowerError>> {
    if !matches!(options.pointer_bits, 32 | 64) {
        return Err(vec![LowerError {
            span: None,
            message: format!(
                "unsupported target pointer width {}; expected 32 or 64",
                options.pointer_bits
            ),
        }]);
    }

    let local_function_ids: BTreeMap<_, _> = mir
        .functions
        .iter()
        .enumerate()
        .map(|(index, function)| (function.symbol, FunctionId::new(index)))
        .collect();
    // Builtin byte-slice calls may receive arrays of different source lengths.
    // Keep one semantic store for both target collection and lowering so their
    // borrowed `Slice<UInt8>` ABI key is canonical and never duplicates a
    // native symbol such as `file_write`.
    let mut semantic_types = types.clone();
    let call_targets = collect_call_targets(mir, &mut semantic_types, &local_function_ids);
    let mut type_table =
        TypeTable::new(&semantic_types, options.pointer_bits, &mir.nominal_layouts);
    let mut functions = Vec::with_capacity(mir.functions.len() + call_targets.external.len());
    let mut errors = Vec::new();
    for (index, function) in mir.functions.iter().enumerate() {
        match FunctionLowerer::new(
            function,
            FunctionId::new(index),
            &call_targets,
            &mut type_table,
        )
        .lower()
        {
            Ok(function) => functions.push(function),
            Err(mut function_errors) => errors.append(&mut function_errors),
        }
    }
    let mut external_targets: Vec<_> = call_targets.external.iter().collect();
    external_targets.sort_by_key(|(_, external)| external.id.index());
    for (target, external) in external_targets {
        match lower_import(target, external, &mut type_table) {
            Ok(function) => functions.push(function),
            Err(error) => errors.push(error),
        }
    }
    if errors.is_empty() {
        Ok(Module {
            types: type_table.types,
            functions,
        })
    } else {
        Err(errors)
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ExternalTarget {
    symbol: Option<SymbolId>,
    name: String,
    parameters: Vec<SemanticTypeId>,
    result: SemanticTypeId,
}

struct ExternalFunction {
    id: FunctionId,
    span: Span,
}

struct CallTargets {
    local: BTreeMap<SymbolId, FunctionId>,
    external: BTreeMap<ExternalTarget, ExternalFunction>,
}

fn borrowed_runtime_argument_capability(name: &str, index: usize) -> Option<Capability> {
    match name {
        "json_object_field_string"
        | "json_object_field_int"
        | "json_object_field_uint"
        | "json_object_field_float"
        | "json_object_field_bool"
            if index == 2 =>
        {
            Some(Capability::Write)
        }
        "json_array_int" | "json_array_uint" | "json_array_float" | "json_array_bool" => {
            match index {
                0 => Some(Capability::Read),
                1 => Some(Capability::Write),
                _ => None,
            }
        }
        "file_write" | "file_append" if index == 1 => Some(Capability::Read),
        "file_write_at" if index == 2 => Some(Capability::Read),
        "file_read_exact" | "file_read_text_exact" if index == 1 || index == 2 => {
            Some(Capability::Write)
        }
        "app_state_read_text"
        | "app_state_read_text_exact"
        | "app_state_read_int"
        | "app_state_read_uint"
        | "app_state_read_float"
        | "app_state_read_bool"
        | "app_state_read_key"
            if index == 1 =>
        {
            Some(Capability::Write)
        }
        "app_state_read_text_exact" if index == 2 => Some(Capability::Write),
        "app_state_write_json_exact" if index == 0 || index == 1 => Some(Capability::Write),
        "app_state_load_json_exact" if index == 0 => Some(Capability::Read),
        "app_data_write_exact" | "app_data_write_exact_if_revision" if index == 0 || index == 1 => {
            Some(Capability::Write)
        }
        "app_data_load_exact" | "app_data_load_exact_if_revision" if index == 0 => {
            Some(Capability::Read)
        }
        "app_data_journal_read_frame_exact_durable" if index == 3 || index == 4 => {
            Some(Capability::Write)
        }
        "app_data_journal_read_latest_frame_exact_durable" if index == 2 || index == 3 => {
            Some(Capability::Write)
        }
        "app_data_journal_stats_durable" if index == 2 => Some(Capability::Write),
        "app_data_journal_maintenance_plan_durable" if index == 4 => Some(Capability::Write),
        "app_data_journal_frame_span_durable" if index == 3 => Some(Capability::Write),
        "app_data_journal_index_lookup_durable" if index == 4 => Some(Capability::Write),
        "app_data_journal_index_export_csv_durable" if index == 3 => Some(Capability::Write),
        "app_data_journal_index_export_csv_file_durable" if index == 4 => Some(Capability::Write),
        "app_data_journal_index_range_durable" if index == 5 => Some(Capability::Write),
        "app_data_journal_index_read_page_durable" if index == 5 || index == 6 => {
            Some(Capability::Write)
        }
        "app_list_read_text_exact" if index == 2 || index == 3 => Some(Capability::Write),
        "app_table_read_column_name_exact" if index == 2 || index == 3 => Some(Capability::Write),
        "app_table_read_named_cell_exact" if index == 3 || index == 4 => Some(Capability::Write),
        "app_table_read_cell_exact" if index == 3 || index == 4 => Some(Capability::Write),
        "app_table_read_int_exact"
        | "app_table_read_uint_exact"
        | "app_table_read_float_exact"
        | "app_table_read_bool_exact"
            if index == 3 =>
        {
            Some(Capability::Write)
        }
        "app_table_read_named_int_exact"
        | "app_table_read_named_uint_exact"
        | "app_table_read_named_float_exact"
        | "app_table_read_named_bool_exact"
            if index == 3 =>
        {
            Some(Capability::Write)
        }
        "app_state_set_text_bytes" if index == 1 => Some(Capability::Read),
        "app_list_read_text" if index == 2 => Some(Capability::Write),
        "app_list_push_text_bytes" if index == 1 => Some(Capability::Read),
        "app_list_export_csv" if index == 1 => Some(Capability::Write),
        "app_list_set_text_bytes" if index == 2 => Some(Capability::Read),
        "app_list_filter_text_ex_bytes" if index == 2 => Some(Capability::Read),
        "app_table_set_cell_bytes" if index == 3 => Some(Capability::Read),
        "app_table_set_cell_bytes_ex" if index == 3 => Some(Capability::Read),
        "app_table_filter_text_ex_bytes" if index == 3 => Some(Capability::Read),
        "app_table_index_collect_int_range"
        | "app_table_index_collect_uint_range"
        | "app_table_index_collect_float_range"
            if index == 4 =>
        {
            Some(Capability::Write)
        }
        "app_table_export_csv" if index == 1 => Some(Capability::Write),
        "app_table_import_csv" if index == 1 => Some(Capability::Read),
        "app_table_read_cell" if index == 3 => Some(Capability::Write),
        "app_table_read_column_name" if index == 2 => Some(Capability::Write),
        "app_table_read_named_cell" if index == 3 => Some(Capability::Write),
        "net_tcp_send" | "net_tcp_send_prefix" if index == 1 => Some(Capability::Read),
        "net_tcp_receive" if index == 1 => Some(Capability::Write),
        "net_reactor_submit_send_buffer" | "net_reactor_submit_send_buffer_prefix"
            if index == 2 =>
        {
            Some(Capability::Read)
        }
        "net_reactor_submit_receive_buffer" if index == 2 => Some(Capability::Write),
        "net_tls_send" if index == 1 => Some(Capability::Read),
        "net_tls_receive" if index == 1 => Some(Capability::Write),
        "http_response_write" if index == 2 => Some(Capability::Read),
        "http_response_write" if index == 3 => Some(Capability::Write),
        "http_response_write_ex" if index == 2 => Some(Capability::Read),
        "http_response_write_ex" if index == 4 => Some(Capability::Write),
        "http_response_write_header" if index == 4 => Some(Capability::Read),
        "http_response_write_header" if index == 5 => Some(Capability::Write),
        "http_response_write_header_ex" if index == 4 => Some(Capability::Read),
        "http_response_write_header_ex" if index == 6 => Some(Capability::Write),
        "http_response_write_cookie" if index == 5 => Some(Capability::Read),
        "http_response_write_cookie" if index == 6 => Some(Capability::Write),
        "http_response_write_cookie_ex" if index == 5 => Some(Capability::Read),
        "http_response_write_cookie_ex" if index == 7 => Some(Capability::Write),
        "http_response_write_header_block" if index == 3 => Some(Capability::Read),
        "http_response_write_header_block" if index == 4 => Some(Capability::Write),
        "http_response_write_header_block_ex" if index == 3 => Some(Capability::Read),
        "http_response_write_header_block_ex" if index == 5 => Some(Capability::Write),
        "http_response_status" | "http_response_status_prefix" if index == 0 => {
            Some(Capability::Read)
        }
        "http_response_header" if index == 0 => Some(Capability::Read),
        "http_response_header" if index == 2 => Some(Capability::Write),
        "http_response_header_prefix" if index == 0 => Some(Capability::Read),
        "http_response_header_prefix" if index == 3 => Some(Capability::Write),
        "http_response_header_exact" => match index {
            0 => Some(Capability::Read),
            2 | 3 => Some(Capability::Write),
            _ => None,
        },
        "http_response_body" if index == 0 => Some(Capability::Read),
        "http_response_body" if index == 1 => Some(Capability::Write),
        "http_response_body_prefix" if index == 0 => Some(Capability::Read),
        "http_response_body_prefix" if index == 2 => Some(Capability::Write),
        "http_response_body_exact" => match index {
            0 => Some(Capability::Read),
            1 | 2 => Some(Capability::Write),
            _ => None,
        },
        "http_response_body_chunked_exact" => match index {
            0 => Some(Capability::Read),
            1 | 2 => Some(Capability::Write),
            _ => None,
        },
        "http_request_body_chunked_exact" => match index {
            0 => Some(Capability::Read),
            1 | 2 => Some(Capability::Write),
            _ => None,
        },
        "http_request_write" if index == 3 => Some(Capability::Read),
        "http_request_write" if index == 4 => Some(Capability::Write),
        "http_request_write_prefix" if index == 3 => Some(Capability::Read),
        "http_request_write_prefix" if index == 5 => Some(Capability::Write),
        "http_request_write_header" if index == 5 => Some(Capability::Read),
        "http_request_write_header" if index == 6 => Some(Capability::Write),
        "http_request_write_header_block" if index == 4 => Some(Capability::Read),
        "http_request_write_header_block" if index == 5 => Some(Capability::Write),
        "http_request_append" if index == 0 => Some(Capability::Write),
        "http_request_append" if index == 2 => Some(Capability::Read),
        "http_request_is_complete" if index == 0 => Some(Capability::Read),
        "http_request_is_complete_prefix" if index == 0 => Some(Capability::Read),
        "http_request_frame_length_prefix" | "http_request_chunked_frame_length_prefix"
            if index == 0 =>
        {
            Some(Capability::Read)
        }
        "http_request_consume_prefix" if index == 0 => Some(Capability::Write),
        "http_request_keep_alive" if index == 0 => Some(Capability::Read),
        "http_request_method" | "http_request_target" | "http_request_body" => match index {
            0 => Some(Capability::Read),
            1 => Some(Capability::Write),
            _ => None,
        },
        "http_request_header" => match index {
            0 => Some(Capability::Read),
            2 => Some(Capability::Write),
            _ => None,
        },
        "http_query_param" => match index {
            0 => Some(Capability::Read),
            2 => Some(Capability::Write),
            _ => None,
        },
        "http_query_param_exact" => match index {
            0 => Some(Capability::Read),
            2 | 3 => Some(Capability::Write),
            _ => None,
        },
        "http_route_match" if index == 0 => Some(Capability::Read),
        "http_route_match_prefix" if index == 0 => Some(Capability::Read),
        "http_router_add" | "http_router_add_exact" | "http_router_add_prefix" if index == 4 => {
            Some(Capability::Read)
        }
        "http_router_respond" if index == 0 => Some(Capability::Read),
        "http_router_respond" if index == 1 => Some(Capability::Write),
        "http_router_respond_prefix" if index == 0 => Some(Capability::Read),
        "http_router_respond_prefix" if index == 2 => Some(Capability::Write),
        "json_object_read_string" => match index {
            0 => Some(Capability::Read),
            2 => Some(Capability::Write),
            _ => None,
        },
        "json_object_read_string_exact" => match index {
            0 => Some(Capability::Read),
            2 | 3 => Some(Capability::Write),
            _ => None,
        },
        "json_object_read_int"
        | "json_object_read_uint"
        | "json_object_read_float"
        | "json_object_read_bool"
            if index == 0 =>
        {
            Some(Capability::Read)
        }
        "json_object_read_int_exact"
        | "json_object_read_uint_exact"
        | "json_object_read_float_exact"
        | "json_object_read_bool_exact" => match index {
            0 => Some(Capability::Read),
            2 => Some(Capability::Write),
            _ => None,
        },
        "parse_int" | "parse_uint" | "parse_float" | "parse_bool" => match index {
            0 => Some(Capability::Read),
            2 => Some(Capability::Write),
            _ => None,
        },
        "ui_list_read_item" | "ui_app_list_read_item" if index == 2 => Some(Capability::Write),
        "ui_table_read_cell" | "ui_app_table_read_cell" if index == 3 => Some(Capability::Write),
        "time_utc_parts" if index == 1 => Some(Capability::Write),
        "time_utc_offset_parts" if index == 2 => Some(Capability::Write),
        "app_scheduler_poll" if index == 1 => Some(Capability::Write),
        "stdin_read" if index == 0 => Some(Capability::Write),
        "stdout_write" | "stderr_write" if index == 0 => Some(Capability::Read),
        "ui_input_read"
        | "ui_input_read_exact"
        | "file_read"
        | "file_read_text"
        | "process_arg_read"
        | "directory_list"
        | "format_bool"
        | "format_int"
        | "format_uint"
        | "format_float"
        | "csv_escape"
        | "json_escape"
            if index == 1 =>
        {
            Some(Capability::Write)
        }
        "file_read_exact" | "file_read_text_exact" if index == 1 || index == 2 => {
            Some(Capability::Write)
        }
        "ui_input_read_exact" if index == 2 => Some(Capability::Write),
        "file_read_at" if index == 2 => Some(Capability::Write),
        "directory_list_ex" if index == 1 || index == 2 => Some(Capability::Write),
        "string_length" if index == 0 => Some(Capability::Read),
        "string_equals" if index < 2 => Some(Capability::Read),
        "buffer_length" | "buffer_capacity" if index == 0 => Some(Capability::Read),
        "buffer_clear" | "buffer_clear_status" if index == 0 => Some(Capability::Write),
        "buffer_clear_move" | "buffer_clear_move_status" if index == 0 => Some(Capability::Write),
        "buffer_pop" if index == 0 => Some(Capability::Write),
        "buffer_remove_move" if index == 0 => Some(Capability::Write),
        "buffer_remove_move_into" | "buffer_remove_move_into_status"
            if index == 0 || index == 2 =>
        {
            Some(Capability::Write)
        }
        "buffer_pop_move_into" | "buffer_pop_move_into_status" if index == 0 || index == 1 => {
            Some(Capability::Write)
        }
        "buffer_insert_move" | "buffer_insert_move_status" if index == 0 => Some(Capability::Write),
        "buffer_insert_move_from" | "buffer_insert_move_from_status" if index == 0 => {
            Some(Capability::Write)
        }
        "buffer_append_move" | "buffer_append_move_status" if index == 0 => Some(Capability::Write),
        "buffer_resize"
        | "buffer_resize_move"
        | "buffer_append_i32"
        | "buffer_reserve_i32"
        | "buffer_append_i32_grow"
        | "buffer_reserve"
        | "buffer_append"
        | "buffer_insert"
        | "buffer_remove"
        | "buffer_remove_drop"
        | "buffer_resize_status"
        | "buffer_resize_move_status"
        | "buffer_reserve_status"
        | "buffer_append_status"
        | "buffer_insert_status"
        | "buffer_remove_status"
        | "buffer_remove_drop_status"
            if index == 0 =>
        {
            Some(Capability::Write)
        }
        "string_builder_append" if index == 0 => Some(Capability::Read),
        "string_builder_append" if index == 1 => Some(Capability::Write),
        "string_builder_append_bytes" if index == 0 => Some(Capability::Read),
        "string_builder_append_bytes" if index == 1 => Some(Capability::Write),
        "string_owned_append" if index == 0 => Some(Capability::Write),
        "string_owned_append" if index == 1 => Some(Capability::Read),
        "string_owned_length" | "string_owned_copy" if index == 0 => Some(Capability::Read),
        "string_owned_copy" if index == 1 => Some(Capability::Write),
        "string_owned_clear" if index == 0 => Some(Capability::Write),
        _ => None,
    }
}

fn descriptor_runtime_argument(name: &str, index: usize) -> bool {
    matches!(
        (name, index),
        ("string_owned_append", 0)
            | ("string_owned_clear", 0)
            | ("buffer_length", 0)
            | ("buffer_capacity", 0)
            | ("buffer_clear", 0)
            | ("buffer_clear_status", 0)
            | ("buffer_clear_move", 0)
            | ("buffer_clear_move_status", 0)
            | ("buffer_resize", 0)
            | ("buffer_resize_move", 0)
            | ("buffer_append_i32", 0)
            | ("buffer_reserve_i32", 0)
            | ("buffer_append_i32_grow", 0)
            | ("buffer_reserve", 0)
            | ("buffer_append", 0)
            | ("buffer_insert", 0)
            | ("buffer_remove", 0)
            | ("buffer_remove_drop", 0)
            | ("buffer_remove_drop_status", 0)
            | ("buffer_pop", 0)
            | ("buffer_pop_move_into", 0)
            | ("buffer_pop_move_into_status", 0)
            | ("buffer_remove_move", 0)
            | ("buffer_remove_move_into", 0)
            | ("buffer_remove_move_into_status", 0)
            | ("buffer_insert_move", 0)
            | ("buffer_insert_move_status", 0)
            | ("buffer_insert_move_from", 0)
            | ("buffer_insert_move_from_status", 0)
            | ("buffer_append_move", 0)
            | ("buffer_append_move_status", 0)
            | ("buffer_resize_status", 0)
            | ("buffer_resize_move_status", 0)
            | ("buffer_reserve_status", 0)
            | ("buffer_append_status", 0)
            | ("buffer_insert_status", 0)
            | ("buffer_remove_status", 0)
    )
}

/// Returns the concrete element type carried by one of the generic owning
/// Buffer builtins.  The semantic checker has already resolved this before
/// JIR lowering; keeping the lookup here makes the native ABI adaptation
/// explicit and deterministic.
fn generic_buffer_element(
    name: &str,
    result: SemanticTypeId,
    arguments: &[MirOperand],
    types: &TypeStore,
) -> Option<SemanticTypeId> {
    let candidate = if name == "buffer_create" {
        let TypeKind::Result { ok, .. } = types.kind(result)? else {
            return None;
        };
        *ok
    } else if matches!(
        name,
        "buffer_clear_move"
            | "buffer_clear_move_status"
            | "buffer_reserve"
            | "buffer_resize_move"
            | "buffer_append"
            | "buffer_insert"
            | "buffer_remove"
            | "buffer_remove_drop"
            | "buffer_reserve_status"
            | "buffer_resize_move_status"
            | "buffer_append_status"
            | "buffer_insert_status"
            | "buffer_remove_status"
            | "buffer_remove_drop_status"
            | "buffer_pop"
            | "buffer_pop_move_into"
            | "buffer_pop_move_into_status"
            | "buffer_remove_move"
            | "buffer_remove_move_into"
            | "buffer_remove_move_into_status"
            | "buffer_insert_move"
            | "buffer_insert_move_status"
            | "buffer_insert_move_from"
            | "buffer_insert_move_from_status"
            | "buffer_append_move"
            | "buffer_append_move_status"
    ) {
        let argument = arguments.first()?;
        match types.kind(argument.ty) {
            Some(TypeKind::Capability { inner, .. }) => *inner,
            _ => argument.ty,
        }
    } else {
        return None;
    };
    match types.kind(candidate)? {
        TypeKind::Buffer(element) => Some(*element),
        _ => None,
    }
}

fn generic_buffer_runtime_parameters(
    name: &str,
    result: SemanticTypeId,
    arguments: &[MirOperand],
    types: &mut TypeStore,
) -> Vec<SemanticTypeId> {
    if !matches!(
        name,
        "buffer_create"
            | "buffer_clear_move"
            | "buffer_clear_move_status"
            | "buffer_resize_move"
            | "buffer_reserve"
            | "buffer_append"
            | "buffer_insert"
            | "buffer_remove"
            | "buffer_remove_drop"
            | "buffer_reserve_status"
            | "buffer_resize_move_status"
            | "buffer_append_status"
            | "buffer_insert_status"
            | "buffer_remove_status"
            | "buffer_remove_drop_status"
            | "buffer_pop"
            | "buffer_pop_move_into"
            | "buffer_pop_move_into_status"
            | "buffer_remove_move"
            | "buffer_remove_move_into"
            | "buffer_remove_move_into_status"
            | "buffer_insert_move"
            | "buffer_insert_move_status"
            | "buffer_insert_move_from"
            | "buffer_insert_move_from_status"
            | "buffer_append_move"
            | "buffer_append_move_status"
    ) {
        return Vec::new();
    }
    if generic_buffer_element(name, result, arguments, types).is_none() {
        return Vec::new();
    }
    let size = types.core().uint_size;
    if matches!(
        name,
        "buffer_resize_move"
            | "buffer_resize_move_status"
            | "buffer_clear_move"
            | "buffer_clear_move_status"
    ) {
        vec![size, size, size, size, size]
    } else if matches!(
        name,
        "buffer_pop" | "buffer_remove_move" | "buffer_insert_move" | "buffer_insert_move_status"
    ) {
        vec![size, size, size, size]
    } else {
        vec![size, size]
    }
}

fn generic_buffer_nested_element(
    name: &str,
    result: SemanticTypeId,
    arguments: &[MirOperand],
    types: &TypeStore,
) -> Option<SemanticTypeId> {
    let element = generic_buffer_element(name, result, arguments, types)?;
    match types.kind(element)? {
        TypeKind::Buffer(inner) => Some(*inner),
        // `OwnedString` shares the native three-word descriptor ABI with a
        // nested Buffer, so pop/remove/insert_move can reuse the same runtime
        // layout parameters while returning the string descriptor itself.
        TypeKind::OwnedString => Some(element),
        _ => None,
    }
}

fn generic_buffer_nested_depth_and_leaf(
    name: &str,
    result: SemanticTypeId,
    arguments: &[MirOperand],
    types: &TypeStore,
) -> Option<(u64, SemanticTypeId)> {
    let mut cursor = generic_buffer_element(name, result, arguments, types)?;
    let mut depth = 0u64;
    while let Some(TypeKind::Buffer(inner)) = types.kind(cursor) {
        depth = depth.checked_add(1)?;
        cursor = *inner;
    }
    (depth > 0).then_some((depth, cursor))
}

fn generic_buffer_owned_string_element(
    name: &str,
    result: SemanticTypeId,
    arguments: &[MirOperand],
    types: &TypeStore,
) -> Option<SemanticTypeId> {
    let element = generic_buffer_element(name, result, arguments, types)?;
    matches!(types.kind(element), Some(TypeKind::OwnedString)).then_some(element)
}

fn generic_buffer_nested_owned_string(
    name: &str,
    result: SemanticTypeId,
    arguments: &[MirOperand],
    types: &TypeStore,
) -> Option<(u64, SemanticTypeId)> {
    let (depth, leaf) = generic_buffer_nested_depth_and_leaf(name, result, arguments, types)?;
    matches!(types.kind(leaf), Some(TypeKind::OwnedString)).then_some((depth, leaf))
}

fn generic_buffer_value_parameter_index(name: &str) -> Option<usize> {
    match name {
        "buffer_append" | "buffer_append_status" => Some(1),
        "buffer_insert" | "buffer_insert_status" => Some(2),
        "buffer_insert_move"
        | "buffer_insert_move_status"
        | "buffer_insert_move_from"
        | "buffer_insert_move_from_status" => Some(2),
        "buffer_append_move" | "buffer_append_move_status" => Some(1),
        "buffer_pop_move_into" | "buffer_pop_move_into_status" => Some(1),
        _ => None,
    }
}

fn legacy_buffer_move_runtime_name(name: &str) -> Option<&'static str> {
    match name {
        "buffer_append" => Some("buffer_append_move"),
        "buffer_append_status" => Some("buffer_append_move_status"),
        "buffer_insert" => Some("buffer_insert_move_from"),
        "buffer_insert_status" => Some("buffer_insert_move_from_status"),
        _ => None,
    }
}

fn semantic_type_may_require_move(
    types: &TypeStore,
    ty: SemanticTypeId,
    visiting: &mut BTreeSet<SemanticTypeId>,
) -> bool {
    if !visiting.insert(ty) {
        return false;
    }
    match types.kind(ty) {
        Some(TypeKind::Buffer(_))
        | Some(TypeKind::OwnedString)
        | Some(TypeKind::Nominal { .. }) => true,
        Some(TypeKind::Array { element, .. } | TypeKind::Option(element)) => {
            semantic_type_may_require_move(types, *element, visiting)
        }
        Some(TypeKind::Result { ok, error }) => {
            semantic_type_may_require_move(types, *ok, visiting)
                || semantic_type_may_require_move(types, *error, visiting)
        }
        _ => false,
    }
}

/// Canonical pointer type used by the C ABI for a generic Buffer element.
///
/// The runtime receives the value through `const void *`; keeping the JIR
/// import pointer opaque lets one module use multiple `Buffer<T>` instantiations
/// without manufacturing duplicate LLVM declarations for the same symbol.
fn generic_buffer_opaque_pointer(types: &mut TypeStore) -> SemanticTypeId {
    let byte = types.core().uint8;
    types.intern(TypeKind::Pointer(byte))
}

fn is_generic_buffer_builtin(name: &str) -> bool {
    matches!(
        name,
        "buffer_create"
            | "buffer_reserve"
            | "buffer_append"
            | "buffer_insert"
            | "buffer_remove"
            | "buffer_remove_drop"
            | "buffer_length"
            | "buffer_capacity"
            | "buffer_clear"
            | "buffer_clear_status"
            | "buffer_clear_move"
            | "buffer_clear_move_status"
            | "buffer_resize"
            | "buffer_resize_move"
            | "buffer_reserve_status"
            | "buffer_append_status"
            | "buffer_insert_status"
            | "buffer_remove_status"
            | "buffer_remove_drop_status"
            | "buffer_pop"
            | "buffer_pop_move_into"
            | "buffer_pop_move_into_status"
            | "buffer_remove_move"
            | "buffer_remove_move_into"
            | "buffer_remove_move_into_status"
            | "buffer_insert_move"
            | "buffer_insert_move_status"
            | "buffer_insert_move_from"
            | "buffer_insert_move_from_status"
            | "buffer_append_move"
            | "buffer_append_move_status"
            | "buffer_resize_status"
            | "buffer_resize_move_status"
    )
}

fn canonical_external_symbol(name: &str, symbol: Option<SymbolId>) -> Option<SymbolId> {
    if is_generic_buffer_builtin(name) {
        None
    } else {
        symbol
    }
}

/// Returns the declared signature of a source-level function reference.
///
/// Builtin/runtime calls intentionally have no semantic function type (their
/// arity and ABI are described by the intrinsic tables below), while imported
/// Jadren functions do.  Keeping this distinction lets package calls preserve
/// borrow capabilities instead of accidentally canonicalizing them as a
/// runtime call based only on the concrete argument value.
fn source_function_signature(
    symbol: Option<SymbolId>,
    callee_ty: SemanticTypeId,
    types: &TypeStore,
) -> Option<(Vec<SemanticTypeId>, SemanticTypeId)> {
    symbol.and_then(|_| match types.kind(callee_ty).cloned() {
        Some(TypeKind::Function { parameters, result }) => Some((parameters.into_vec(), result)),
        _ => None,
    })
}

fn semantic_type_contains_generic(types: &TypeStore, ty: SemanticTypeId) -> bool {
    let Some(kind) = types.kind(ty) else {
        return false;
    };
    match kind {
        TypeKind::GenericParameter(_) => true,
        TypeKind::Array { element, .. }
        | TypeKind::Vector { element, .. }
        | TypeKind::Buffer(element)
        | TypeKind::Slice(element)
        | TypeKind::Pointer(element)
        | TypeKind::Option(element) => semantic_type_contains_generic(types, *element),
        TypeKind::Result { ok, error } => {
            semantic_type_contains_generic(types, *ok)
                || semantic_type_contains_generic(types, *error)
        }
        TypeKind::Nominal { arguments, .. } => arguments
            .iter()
            .any(|argument| semantic_type_contains_generic(types, *argument)),
        TypeKind::Function { parameters, result } => {
            parameters
                .iter()
                .any(|parameter| semantic_type_contains_generic(types, *parameter))
                || semantic_type_contains_generic(types, *result)
        }
        TypeKind::Capability { inner, .. } => semantic_type_contains_generic(types, *inner),
        _ => false,
    }
}

fn canonical_external_result(
    name: &str,
    result: SemanticTypeId,
    types: &mut TypeStore,
) -> SemanticTypeId {
    if matches!(name, "buffer_create" | "buffer_pop" | "buffer_remove_move")
        && let Some(TypeKind::Result { ok, error }) = types.kind(result).cloned()
        && matches!(types.kind(ok), Some(TypeKind::Buffer(_)))
    {
        let buffer = types.intern(TypeKind::Buffer(types.core().uint8));
        return types.intern(TypeKind::Result { ok: buffer, error });
    }
    result
}

/// Target layout used by the generic native Buffer ABI.  The first generic
/// milestone intentionally accepts concrete scalar, pointer, array and vector
/// elements; nominal record layout will use the exported ABI metadata in the
/// next layout pass instead of guessing here.
fn semantic_type_layout(
    types: &TypeStore,
    ty: SemanticTypeId,
    pointer_bits: u16,
) -> Option<(u64, u64)> {
    match types.kind(ty)? {
        TypeKind::Bool => Some((1, 1)),
        TypeKind::Char => Some((4, 4)),
        TypeKind::Integer { width, .. } => {
            let bytes = match width {
                IntegerWidth::Bits8 => 1,
                IntegerWidth::Bits16 => 2,
                IntegerWidth::Bits32 => 4,
                IntegerWidth::Bits64 => 8,
                IntegerWidth::Pointer => u64::from(pointer_bits / 8),
            };
            Some((bytes, bytes))
        }
        TypeKind::Float(width) => {
            let bytes = match width {
                FloatWidth::Bits16 => 2,
                FloatWidth::Bits32 => 4,
                FloatWidth::Bits64 => 8,
            };
            Some((bytes, bytes))
        }
        TypeKind::Pointer(_) => {
            let bytes = u64::from(pointer_bits / 8);
            Some((bytes, bytes))
        }
        TypeKind::String => {
            let pointer = u64::from(pointer_bits / 8);
            Some((pointer * 2u64, pointer))
        }
        TypeKind::Array { element, length } => {
            let (element_size, element_alignment) =
                semantic_type_layout(types, *element, pointer_bits)?;
            let size = element_size.checked_mul(*length)?;
            Some((size, element_alignment))
        }
        TypeKind::Vector { element, lanes } => {
            let (element_size, element_alignment) =
                semantic_type_layout(types, *element, pointer_bits)?;
            let size = element_size.checked_mul(u64::from(*lanes))?;
            Some((size, element_alignment))
        }
        _ => None,
    }
}

fn align_layout(size: u64, alignment: u64) -> Option<u64> {
    let remainder = size % alignment;
    if remainder == 0 {
        Some(size)
    } else {
        size.checked_add(alignment - remainder)
    }
}

fn generic_buffer_layout(
    name: &str,
    result: SemanticTypeId,
    arguments: &[MirOperand],
    types: &mut TypeTable,
) -> Option<(u64, u64)> {
    let element = generic_buffer_element(name, result, arguments, &types.semantic)?;
    types.semantic_layout(element)
}

fn is_borrowed_runtime_argument(name: &str, index: usize) -> bool {
    borrowed_runtime_argument_capability(name, index).is_some()
}

fn canonical_external_parameter(
    name: &str,
    index: usize,
    ty: SemanticTypeId,
    types: &mut TypeStore,
) -> SemanticTypeId {
    if is_generic_buffer_builtin(name) && descriptor_runtime_argument(name, index) {
        let buffer = types.intern(TypeKind::Buffer(types.core().uint8));
        return types.intern(TypeKind::Pointer(buffer));
    }
    if matches!(
        (name, index),
        ("buffer_remove_move_into", 2)
            | ("buffer_remove_move_into_status", 2)
            | ("buffer_pop_move_into", 1)
            | ("buffer_pop_move_into_status", 1)
    ) {
        // All generic `T` instantiations share one native import symbol. The
        // concrete output layout is carried by the compiler-injected size and
        // alignment constants, so the raw pointer itself stays opaque.
        return generic_buffer_opaque_pointer(types);
    }
    if !is_borrowed_runtime_argument(name, index) {
        return ty;
    }
    if descriptor_runtime_argument(name, index) {
        let inner = match types.kind(ty).cloned() {
            Some(TypeKind::Capability { inner, .. }) => inner,
            _ => ty,
        };
        return types.intern(TypeKind::Pointer(inner));
    }
    let element = match types.kind(ty).cloned() {
        Some(TypeKind::Capability { inner, .. }) => match types.kind(inner).cloned() {
            Some(TypeKind::Array { element, .. })
            | Some(TypeKind::Buffer(element))
            | Some(TypeKind::Slice(element)) => Some(element),
            _ => None,
        },
        Some(TypeKind::Array { element, .. })
        | Some(TypeKind::Buffer(element))
        | Some(TypeKind::Slice(element)) => Some(element),
        _ => None,
    };
    if matches!(types.kind(ty), Some(TypeKind::String)) {
        return types.intern(TypeKind::Capability {
            capability: Capability::Read,
            inner: ty,
        });
    }
    if matches!(types.kind(ty), Some(TypeKind::OwnedString)) {
        let string = types.core().string;
        return types.intern(TypeKind::Capability {
            capability: Capability::Read,
            inner: string,
        });
    }
    element
        .map(|element| types.intern(TypeKind::Slice(element)))
        .unwrap_or(ty)
}

fn collect_call_targets(
    mir: &MirModule,
    types: &mut TypeStore,
    local: &BTreeMap<SymbolId, FunctionId>,
) -> CallTargets {
    let mut targets = CallTargets {
        local: local.clone(),
        external: BTreeMap::new(),
    };
    for function in &mir.functions {
        for block in &function.blocks {
            for statement in &block.statements {
                match statement {
                    MirStatement::Assign {
                        destination_indices,
                        value,
                        ..
                    } => {
                        for index in destination_indices {
                            collect_operand_calls(
                                index,
                                mir,
                                types,
                                mir.functions.len(),
                                &mut targets,
                            );
                        }
                        if let Some(value) = value {
                            collect_operand_calls(
                                value,
                                mir,
                                types,
                                mir.functions.len(),
                                &mut targets,
                            );
                        }
                    }
                    MirStatement::Evaluate { value, .. } => {
                        if let Some(value) = value {
                            collect_operand_calls(
                                value,
                                mir,
                                types,
                                mir.functions.len(),
                                &mut targets,
                            );
                        }
                    }
                    MirStatement::StorageLive { .. }
                    | MirStatement::StorageDead { .. }
                    | MirStatement::RegionEnter { .. }
                    | MirStatement::RegionExit { .. }
                    | MirStatement::Borrow { .. }
                    | MirStatement::Drop { .. } => {}
                }
            }
            match &block.terminator {
                MirTerminator::Switch { value, .. } | MirTerminator::Return { value, .. } => {
                    if let Some(value) = value {
                        collect_operand_calls(value, mir, types, mir.functions.len(), &mut targets);
                    }
                }
                MirTerminator::Match { value, .. } | MirTerminator::Propagate { value, .. } => {
                    collect_operand_calls(value, mir, types, mir.functions.len(), &mut targets);
                }
                MirTerminator::Goto { .. } | MirTerminator::Unreachable { .. } => {}
            }
        }
    }
    targets
}

fn collect_operand_calls(
    operand: &MirOperand,
    mir: &MirModule,
    types: &mut TypeStore,
    local_count: usize,
    targets: &mut CallTargets,
) {
    match &operand.kind {
        MirOperandKind::Call { callee, arguments } => {
            if let MirOperandKind::Function { name, symbol } = &callee.kind
                && symbol.is_none_or(|symbol| !targets.local.contains_key(&symbol))
                && !is_lowered_vector_intrinsic(name)
                && constructor_variant_index(mir, types, operand.ty, name).is_none()
            {
                let source_signature = source_function_signature(*symbol, callee.ty, types).filter(
                    |(parameters, result)| {
                        !parameters
                            .iter()
                            .any(|parameter| semantic_type_contains_generic(types, *parameter))
                            && !semantic_type_contains_generic(types, *result)
                    },
                );
                let (parameters, result) = if let Some((parameters, result)) = source_signature {
                    (parameters, result)
                } else {
                    let mut parameters = arguments
                        .iter()
                        .enumerate()
                        .map(|(index, argument)| {
                            canonical_external_parameter(name, index, argument.ty, types)
                        })
                        .collect::<Vec<_>>();
                    if let Some(index) = generic_buffer_value_parameter_index(name)
                        && parameters.get(index).is_some()
                        && generic_buffer_element(name, operand.ty, arguments, types).is_some()
                    {
                        parameters[index] = generic_buffer_opaque_pointer(types);
                    }
                    parameters.extend(generic_buffer_runtime_parameters(
                        name, operand.ty, arguments, types,
                    ));
                    let result = canonical_external_result(name, operand.ty, types);
                    (parameters, result)
                };
                let target = ExternalTarget {
                    symbol: canonical_external_symbol(name, *symbol),
                    name: name.clone(),
                    parameters,
                    result,
                };
                let next = FunctionId::new(local_count + targets.external.len());
                targets.external.entry(target).or_insert(ExternalFunction {
                    id: next,
                    span: callee.span,
                });
                if let Some(runtime_name) = legacy_buffer_move_runtime_name(name)
                    && generic_buffer_element(name, operand.ty, arguments, types).is_some_and(
                        |element| {
                            semantic_type_may_require_move(types, element, &mut BTreeSet::new())
                        },
                    )
                {
                    let mut move_parameters = arguments
                        .iter()
                        .enumerate()
                        .map(|(index, argument)| {
                            canonical_external_parameter(runtime_name, index, argument.ty, types)
                        })
                        .collect::<Vec<_>>();
                    if let Some(index) = generic_buffer_value_parameter_index(runtime_name)
                        && move_parameters.get(index).is_some()
                        && generic_buffer_element(runtime_name, operand.ty, arguments, types)
                            .is_some()
                    {
                        move_parameters[index] = generic_buffer_opaque_pointer(types);
                    }
                    move_parameters.extend(generic_buffer_runtime_parameters(
                        runtime_name,
                        operand.ty,
                        arguments,
                        types,
                    ));
                    let move_target = ExternalTarget {
                        symbol: canonical_external_symbol(runtime_name, *symbol),
                        name: runtime_name.to_owned(),
                        parameters: move_parameters,
                        result: canonical_external_result(runtime_name, operand.ty, types),
                    };
                    let next = FunctionId::new(local_count + targets.external.len());
                    targets
                        .external
                        .entry(move_target)
                        .or_insert(ExternalFunction {
                            id: next,
                            span: callee.span,
                        });
                }
            }
            collect_operand_calls(callee, mir, types, local_count, targets);
            for argument in arguments {
                collect_operand_calls(argument, mir, types, local_count, targets);
            }
        }
        MirOperandKind::Function { name, symbol }
            if symbol.is_none_or(|symbol| !targets.local.contains_key(&symbol))
                && !is_lowered_vector_intrinsic(name)
                && let Some(TypeKind::Function { parameters, result }) =
                    types.kind(operand.ty).cloned() =>
        {
            let source_signature = source_function_signature(*symbol, operand.ty, types);
            let (parameters, result) = if let Some((parameters, result)) = source_signature {
                (parameters, result)
            } else {
                let parameters = parameters
                    .iter()
                    .enumerate()
                    .map(|(index, parameter)| {
                        canonical_external_parameter(name, index, *parameter, types)
                    })
                    .collect();
                (parameters, canonical_external_result(name, result, types))
            };
            let target = ExternalTarget {
                symbol: canonical_external_symbol(name, *symbol),
                name: name.clone(),
                parameters,
                result,
            };
            let next = FunctionId::new(local_count + targets.external.len());
            targets.external.entry(target).or_insert(ExternalFunction {
                id: next,
                span: operand.span,
            });
        }
        MirOperandKind::Unary { operand, .. } => {
            collect_operand_calls(operand, mir, types, local_count, targets);
        }
        MirOperandKind::Cast { operand } => {
            collect_operand_calls(operand, mir, types, local_count, targets);
        }
        MirOperandKind::Binary { left, right, .. } => {
            collect_operand_calls(left, mir, types, local_count, targets);
            collect_operand_calls(right, mir, types, local_count, targets);
        }
        MirOperandKind::RegionAllocate { arguments, .. } | MirOperandKind::Array(arguments) => {
            for argument in arguments {
                collect_operand_calls(argument, mir, types, local_count, targets);
            }
        }
        MirOperandKind::Index { base, index } => {
            collect_operand_calls(base, mir, types, local_count, targets);
            collect_operand_calls(index, mir, types, local_count, targets);
        }
        MirOperandKind::Length { base } => {
            collect_operand_calls(base, mir, types, local_count, targets);
        }
        MirOperandKind::Field { base, .. } => {
            collect_operand_calls(base, mir, types, local_count, targets);
        }
        MirOperandKind::Struct { fields, .. } => {
            for (_, value) in fields {
                collect_operand_calls(value, mir, types, local_count, targets);
            }
        }
        MirOperandKind::Unit
        | MirOperandKind::Place(_)
        | MirOperandKind::Literal(_)
        | MirOperandKind::Function { .. }
        | MirOperandKind::PatternExtract { .. }
        | MirOperandKind::CarrierExtract { .. }
        | MirOperandKind::PropagateResidual { .. }
        | MirOperandKind::HighLevel(_) => {}
    }
}

fn is_lowered_vector_intrinsic(name: &str) -> bool {
    vector_intrinsic_lanes(name).is_some()
}

fn vector_intrinsic_lanes(name: &str) -> Option<u16> {
    match name {
        "vector_load2" | "vector_splat2" | "vector_store2" => Some(2),
        "vector_load3" | "vector_splat3" | "vector_store3" => Some(3),
        "vector_load4" | "vector_splat4" | "vector_store4" => Some(4),
        "vector_load8" | "vector_splat8" | "vector_store8" => Some(8),
        _ => None,
    }
}

fn constructor_variant_index(
    mir: &MirModule,
    types: &TypeStore,
    ty: SemanticTypeId,
    path: &str,
) -> Option<u32> {
    let name = path.rsplit('.').next().unwrap_or(path);
    let names: Vec<&str> = match types.kind(ty)? {
        TypeKind::Option(_) => vec!["None", "Some"],
        TypeKind::Result { .. } => vec!["Error", "Ok"],
        TypeKind::Nominal { constructor, .. } => {
            let layout = mir
                .nominal_layouts
                .iter()
                .find(|layout| layout.constructor == *constructor)?;
            let NominalLayoutKind::Enum { variants } = &layout.kind else {
                return None;
            };
            variants
                .iter()
                .map(|variant| variant.name.as_str())
                .collect()
        }
        _ => return None,
    };
    names
        .iter()
        .position(|candidate| *candidate == name)
        .map(|index| index as u32)
}

fn lower_import(
    target: &ExternalTarget,
    external: &ExternalFunction,
    types: &mut TypeTable,
) -> Result<Function, LowerError> {
    let mut lowered_parameters = Vec::with_capacity(target.parameters.len());
    for (index, parameter) in target.parameters.iter().enumerate() {
        let lowered = if matches!(
            (target.name.as_str(), index),
            ("buffer_remove_move_into", 2)
                | ("buffer_remove_move_into_status", 2)
                | ("buffer_pop_move_into", 1)
                | ("buffer_pop_move_into_status", 1)
        ) {
            // The output record is a raw caller-owned pointer. It is marked
            // `write` in source for borrow checking, but the native ABI must
            // not reinterpret it as a slice/Buffer view.
            types.lower(*parameter, Some(external.span))?
        } else if descriptor_runtime_argument(&target.name, index) {
            types.lower(*parameter, Some(external.span))?
        } else if is_borrowed_runtime_argument(&target.name, index) {
            // Builtins do not carry a source-level function declaration, so
            // their external target initially records the caller's owning
            // Buffer type.  The native runtime receives the borrowed
            // pointer/length view, matching the source-level
            // `write Slice<UInt8>` contract.
            let inner = match types.semantic.kind(*parameter).cloned() {
                Some(TypeKind::Capability { inner, .. }) => inner,
                Some(TypeKind::Array { element, .. }) => {
                    types.semantic.intern(TypeKind::Slice(element))
                }
                _ => *parameter,
            };
            types.lower_borrow_capability(inner, false, Some(external.span))?
        } else {
            types.lower(*parameter, Some(external.span))?
        };
        lowered_parameters.push(Parameter {
            value: ValueId::new(index),
            ty: lowered,
            name: None,
        });
    }
    Ok(Function {
        id: external.id,
        name: target.name.clone(),
        linkage: Linkage::Import,
        parameters: lowered_parameters,
        result: types.lower(target.result, Some(external.span))?,
        blocks: Vec::new(),
        span: Some(external.span),
    })
}

#[derive(Clone, Copy)]
struct CarrierDropBranchInfo {
    variant: u32,
    depth: u32,
    leaf: SemanticTypeId,
}

#[derive(Clone, Copy)]
struct CarrierDropFieldInfo {
    variant: u32,
    offset: u64,
    depth: u32,
    leaf: SemanticTypeId,
}

#[derive(Clone, Copy)]
struct RecordDropFieldInfo {
    payload_variant: u64,
    offset: u64,
    depth: u32,
    leaf: SemanticTypeId,
}

struct TypeTable {
    semantic: TypeStore,
    pointer_bits: u16,
    types: Vec<Type>,
    lowered: BTreeMap<SemanticTypeId, TypeId>,
    layouts: BTreeMap<NominalTypeId, NominalLayout>,
    lowering: BTreeSet<SemanticTypeId>,
}

impl TypeTable {
    fn new(semantic: &TypeStore, pointer_bits: u16, layouts: &[NominalLayout]) -> Self {
        Self {
            semantic: semantic.clone(),
            pointer_bits,
            types: Vec::new(),
            lowered: BTreeMap::new(),
            layouts: layouts
                .iter()
                .cloned()
                .map(|layout| (layout.constructor, layout))
                .collect(),
            lowering: BTreeSet::new(),
        }
    }

    fn semantic_layout(&mut self, ty: SemanticTypeId) -> Option<(u64, u64)> {
        match self.semantic.kind(ty).cloned()? {
            TypeKind::Nominal {
                constructor,
                arguments,
            } => {
                let layout = self.layouts.get(&constructor)?.clone();
                match layout.kind {
                    NominalLayoutKind::Record { fields } => {
                        let mut substitution = Substitution::new();
                        for (parameter, argument) in layout.generic_parameters.iter().zip(arguments)
                        {
                            substitution.insert(*parameter, argument);
                        }
                        let mut offset = 0u64;
                        let mut alignment = 1u64;
                        for field in fields {
                            let field_ty = substitution.apply(&mut self.semantic, field.ty).ok()?;
                            let (field_size, field_alignment) = self.semantic_layout(field_ty)?;
                            offset = align_layout(offset, field_alignment)?;
                            offset = offset.checked_add(field_size)?;
                            alignment = alignment.max(field_alignment);
                        }
                        Some((align_layout(offset, alignment)?, alignment))
                    }
                    NominalLayoutKind::Enum { variants } => {
                        let mut substitution = Substitution::new();
                        for (parameter, argument) in layout.generic_parameters.iter().zip(arguments)
                        {
                            substitution.insert(*parameter, argument);
                        }
                        let fields = variants
                            .into_iter()
                            .map(|variant| {
                                variant
                                    .fields
                                    .into_iter()
                                    .map(|field| substitution.apply(&mut self.semantic, field))
                                    .collect::<Result<Vec<_>, _>>()
                            })
                            .collect::<Result<Vec<_>, _>>()
                            .ok()?;
                        self.semantic_enum_layout(&fields)
                    }
                }
            }
            // Option and Result are inline tagged carriers. They may be
            // stored in a generic Buffer when every payload is copy-safe;
            // unlike Buffer/String they carry no owning pointer metadata.
            TypeKind::Option(inner) => self.semantic_enum_layout(&[Vec::new(), vec![inner]]),
            TypeKind::Result { ok, error } => self.semantic_enum_layout(&[vec![error], vec![ok]]),
            TypeKind::Array { element, length } => {
                let (element_size, element_alignment) = self.semantic_layout(element)?;
                Some((element_size.checked_mul(length)?, element_alignment))
            }
            TypeKind::Vector { element, lanes } => {
                let (element_size, element_alignment) = self.semantic_layout(element)?;
                Some((
                    element_size.checked_mul(u64::from(lanes))?,
                    element_alignment,
                ))
            }
            // Owning Buffer<T> descriptors are target-native `{pointer,
            // length, capacity}` values regardless of their element type.
            // This layout is required for nested Buffer<Buffer<U>> storage;
            // the inner element stride is carried separately by drop glue.
            TypeKind::Buffer(_) | TypeKind::OwnedString => {
                let pointer = u64::from(self.pointer_bits / 8);
                Some((pointer.checked_mul(3)?, pointer))
            }
            _ => semantic_type_layout(&self.semantic, ty, self.pointer_bits),
        }
    }

    fn semantic_enum_layout(&mut self, variants: &[Vec<SemanticTypeId>]) -> Option<(u64, u64)> {
        // Keep carrier layout identical to the nominal enum path: a 32-bit
        // tag followed by target-aligned payload bytes for the largest
        // variant. This is the stride copied by generic Buffer<T> operations.
        let mut payload_size = 0u64;
        let mut payload_alignment = 1u64;
        for variant in variants {
            let mut offset = 0u64;
            let mut variant_alignment = 1u64;
            for field_ty in variant {
                let (field_size, field_alignment) = self.semantic_layout(*field_ty)?;
                offset = align_layout(offset, field_alignment)?;
                offset = offset.checked_add(field_size)?;
                variant_alignment = variant_alignment.max(field_alignment);
            }
            let variant_size = align_layout(offset, variant_alignment)?;
            payload_size = payload_size.max(variant_size);
            payload_alignment = payload_alignment.max(variant_alignment);
        }
        let tag_size = 4u64;
        let padding = (payload_alignment - tag_size % payload_alignment) % payload_alignment;
        let total = tag_size.checked_add(padding)?.checked_add(payload_size)?;
        Some((align_layout(total, payload_alignment)?, payload_alignment))
    }

    fn buffer_chain_leaf_depth(&self, ty: SemanticTypeId) -> Option<(u32, SemanticTypeId)> {
        let mut cursor = ty;
        let mut depth = 0u32;
        while let Some(TypeKind::Buffer(inner)) = self.semantic.kind(cursor) {
            depth = depth.checked_add(1)?;
            cursor = *inner;
        }
        (depth > 0).then_some((depth, cursor))
    }

    /// Returns the field-table depth/leaf pair for one owning carrier payload.
    /// A direct OwnedString uses the reserved maximum depth marker so runtime
    /// cleanup dispatches to the UTF-8 destructor instead of Buffer recursion.
    fn carrier_owning_leaf_depth(&self, ty: SemanticTypeId) -> Option<(u32, SemanticTypeId)> {
        if matches!(self.semantic.kind(ty), Some(TypeKind::OwnedString)) {
            Some((u32::MAX, ty))
        } else {
            self.buffer_chain_leaf_depth(ty)
        }
    }

    fn carrier_buffer_drop_info(
        &mut self,
        carrier: SemanticTypeId,
    ) -> Option<(CarrierDropBranchInfo, Option<CarrierDropBranchInfo>, u64)> {
        let (primary, alternate) = match self.semantic.kind(carrier).cloned()? {
            TypeKind::Option(inner) => {
                let (depth, leaf) = self.carrier_owning_leaf_depth(inner)?;
                (
                    CarrierDropBranchInfo {
                        variant: 1,
                        depth,
                        leaf,
                    },
                    None,
                )
            }
            TypeKind::Result { ok, error } => {
                let ok_branch =
                    self.carrier_owning_leaf_depth(ok)
                        .map(|(depth, leaf)| CarrierDropBranchInfo {
                            variant: 1,
                            depth,
                            leaf,
                        });
                let error_branch = self.carrier_owning_leaf_depth(error).map(|(depth, leaf)| {
                    CarrierDropBranchInfo {
                        variant: 0,
                        depth,
                        leaf,
                    }
                });
                match (error_branch, ok_branch) {
                    (Some(error), Some(ok)) => (error, Some(ok)),
                    (Some(error), None) => (error, None),
                    (None, Some(ok)) => (ok, None),
                    (None, None) => return None,
                }
            }
            TypeKind::Nominal { .. } => {
                let (branches, _) = self.enum_carrier_drop_info(carrier)?;
                let mut branches = branches.into_iter();
                (branches.next()?, None)
            }
            _ => return None,
        };
        let payload_alignment = self.semantic_layout(carrier)?.1;
        let payload_offset = align_layout(4, payload_alignment)?;
        Some((primary, alternate, payload_offset))
    }

    fn enum_carrier_drop_info(
        &mut self,
        carrier: SemanticTypeId,
    ) -> Option<(Vec<CarrierDropBranchInfo>, u64)> {
        let TypeKind::Nominal {
            constructor,
            arguments,
        } = self.semantic.kind(carrier).cloned()?
        else {
            return None;
        };
        let layout = self.layouts.get(&constructor)?.clone();
        let NominalLayoutKind::Enum { variants } = layout.kind else {
            return None;
        };
        let mut substitution = Substitution::new();
        for (parameter, argument) in layout
            .generic_parameters
            .iter()
            .zip(arguments.iter().copied())
        {
            substitution.insert(*parameter, argument);
        }
        let mut branches = Vec::new();
        for (variant_index, variant) in variants.into_iter().enumerate() {
            let fields = variant
                .fields
                .into_iter()
                .map(|field| substitution.apply(&mut self.semantic, field).ok())
                .collect::<Option<Vec<_>>>()?;
            if fields.len() != 1 {
                continue;
            }
            let Some((depth, leaf)) = self.carrier_owning_leaf_depth(fields[0]) else {
                continue;
            };
            branches.push(CarrierDropBranchInfo {
                variant: u32::try_from(variant_index).ok()?,
                depth,
                leaf,
            });
        }
        if branches.is_empty() {
            return None;
        }
        let payload_alignment = self.semantic_layout(carrier)?.1;
        let payload_offset = align_layout(4, payload_alignment)?;
        Some((branches, payload_offset))
    }

    fn enum_carrier_drop_fields_info(
        &mut self,
        carrier: SemanticTypeId,
    ) -> Option<Vec<CarrierDropFieldInfo>> {
        let TypeKind::Nominal {
            constructor,
            arguments,
        } = self.semantic.kind(carrier).cloned()?
        else {
            return None;
        };
        let layout = self.layouts.get(&constructor)?.clone();
        let NominalLayoutKind::Enum { variants } = layout.kind else {
            return None;
        };
        let mut substitution = Substitution::new();
        for (parameter, argument) in layout
            .generic_parameters
            .iter()
            .zip(arguments.iter().copied())
        {
            substitution.insert(*parameter, argument);
        }
        let payload_alignment = self.semantic_layout(carrier)?.1;
        let payload_offset = align_layout(4, payload_alignment)?;
        let mut fields = Vec::new();
        for (variant_index, variant_layout) in variants.into_iter().enumerate() {
            let variant = u32::try_from(variant_index).ok()?;
            let mut field_offset = 0u64;
            for field in variant_layout.fields {
                let field = substitution.apply(&mut self.semantic, field).ok()?;
                let (field_size, field_alignment) = self.semantic_layout(field)?;
                field_offset = align_layout(field_offset, field_alignment)?;
                if let Some((depth, leaf)) = self.carrier_owning_leaf_depth(field) {
                    fields.push(CarrierDropFieldInfo {
                        variant,
                        offset: payload_offset.checked_add(field_offset)?,
                        depth,
                        leaf,
                    });
                }
                field_offset = field_offset.checked_add(field_size)?;
            }
        }
        (!fields.is_empty()).then_some(fields)
    }

    fn record_drop_fields_info(
        &mut self,
        record_ty: SemanticTypeId,
    ) -> Option<Vec<RecordDropFieldInfo>> {
        let mut fields = Vec::new();
        let mut visiting = BTreeSet::new();
        match self.semantic.kind(record_ty) {
            Some(TypeKind::Array { .. }) => {
                self.collect_record_drop_value_fields(record_ty, 0, &mut fields, &mut visiting)?
            }
            _ => self.collect_record_drop_fields(record_ty, 0, &mut fields, &mut visiting)?,
        }
        (!fields.is_empty()).then_some(fields)
    }

    /// Returns field-table metadata for a record-like Buffer element used by
    /// explicit remove/resize/clear builtins. Inline Option/Result carriers
    /// share the same runtime five-word field ABI, while ordinary drop glue
    /// keeps its dedicated carrier instruction and classification.
    fn buffer_record_drop_fields_info(
        &mut self,
        element_ty: SemanticTypeId,
    ) -> Option<Vec<RecordDropFieldInfo>> {
        if matches!(
            self.semantic.kind(element_ty),
            Some(TypeKind::Option(_)) | Some(TypeKind::Result { .. })
        ) {
            let (primary, alternate, payload_offset) = self.carrier_buffer_drop_info(element_ty)?;
            let mut fields = vec![RecordDropFieldInfo {
                payload_variant: u64::from(primary.variant),
                offset: payload_offset,
                depth: primary.depth,
                leaf: primary.leaf,
            }];
            if let Some(alternate) = alternate {
                fields.push(RecordDropFieldInfo {
                    payload_variant: u64::from(alternate.variant),
                    offset: payload_offset,
                    depth: alternate.depth,
                    leaf: alternate.leaf,
                });
            }
            Some(fields)
        } else if matches!(
            self.semantic.kind(element_ty),
            Some(TypeKind::Nominal { constructor, .. })
                if matches!(
                    self.layouts.get(constructor).map(|layout| &layout.kind),
                    Some(NominalLayoutKind::Enum { .. })
                )
        ) {
            let fields = self.enum_carrier_drop_fields_info(element_ty)?;
            Some(
                fields
                    .into_iter()
                    .map(|field| RecordDropFieldInfo {
                        payload_variant: u64::from(field.variant),
                        offset: field.offset,
                        depth: field.depth,
                        leaf: field.leaf,
                    })
                    .collect(),
            )
        } else {
            self.record_drop_fields_info(element_ty)
        }
    }

    /// Returns whether a generic Buffer element owns a descriptor and must
    /// use the move-aware byte ABI for append/insert. Copy-safe records and
    /// carriers intentionally stay on the ordinary copy path.
    fn buffer_element_requires_move(&mut self, element_ty: SemanticTypeId) -> bool {
        match self.semantic.kind(element_ty).cloned() {
            Some(TypeKind::Buffer(_)) | Some(TypeKind::OwnedString) => true,
            Some(TypeKind::Option(_)) | Some(TypeKind::Result { .. }) => self
                .buffer_record_drop_fields_info(element_ty)
                .is_some_and(|fields| !fields.is_empty()),
            Some(TypeKind::Array { .. }) => self
                .record_drop_fields_info(element_ty)
                .is_some_and(|fields| !fields.is_empty()),
            Some(TypeKind::Nominal { constructor, .. }) => {
                match self.layouts.get(&constructor).map(|layout| &layout.kind) {
                    Some(NominalLayoutKind::Record { .. }) => self
                        .record_drop_fields_info(element_ty)
                        .is_some_and(|fields| !fields.is_empty()),
                    Some(NominalLayoutKind::Enum { .. }) => self
                        .enum_carrier_drop_fields_info(element_ty)
                        .is_some_and(|fields| !fields.is_empty()),
                    None => false,
                }
            }
            _ => false,
        }
    }

    fn nested_buffer_record_drop_fields_info(
        &mut self,
        outer_element: SemanticTypeId,
    ) -> Option<(u32, SemanticTypeId, Vec<RecordDropFieldInfo>)> {
        let (depth, record_ty) = self.buffer_chain_leaf_depth(outer_element)?;
        let fields = self.buffer_record_drop_fields_info(record_ty)?;
        Some((depth, record_ty, fields))
    }

    fn nested_record_drop_fields_info(
        &mut self,
        outer_element: SemanticTypeId,
    ) -> Option<(u32, SemanticTypeId, Vec<RecordDropFieldInfo>)> {
        let (depth, record_ty) = self.buffer_chain_leaf_depth(outer_element)?;
        let fields = self.record_drop_fields_info(record_ty)?;
        Some((depth, record_ty, fields))
    }

    fn collect_record_drop_fields(
        &mut self,
        record_ty: SemanticTypeId,
        base_offset: u64,
        owning_fields: &mut Vec<RecordDropFieldInfo>,
        visiting: &mut BTreeSet<SemanticTypeId>,
    ) -> Option<()> {
        if !visiting.insert(record_ty) {
            return None;
        }
        let TypeKind::Nominal {
            constructor,
            arguments,
        } = self.semantic.kind(record_ty).cloned()?
        else {
            return None;
        };
        let layout = self.layouts.get(&constructor)?.clone();
        let NominalLayoutKind::Record { fields } = layout.kind else {
            return None;
        };
        let mut substitution = Substitution::new();
        for (parameter, argument) in layout
            .generic_parameters
            .iter()
            .zip(arguments.iter().copied())
        {
            substitution.insert(*parameter, argument);
        }
        let mut offset = 0u64;
        for field in fields {
            let field_ty = substitution.apply(&mut self.semantic, field.ty).ok()?;
            let (field_size, field_alignment) = self.semantic_layout(field_ty)?;
            offset = align_layout(offset, field_alignment)?;
            // Records and fixed arrays are inline C-layout values.
            // Flattening every owning Buffer path into the same table keeps
            // the existing five-word runtime ABI and avoids hidden metadata.
            self.collect_record_drop_value_fields(
                field_ty,
                base_offset.checked_add(offset)?,
                owning_fields,
                visiting,
            )?;
            offset = offset.checked_add(field_size)?;
        }
        visiting.remove(&record_ty);
        Some(())
    }

    fn collect_record_drop_value_fields(
        &mut self,
        value_ty: SemanticTypeId,
        base_offset: u64,
        owning_fields: &mut Vec<RecordDropFieldInfo>,
        visiting: &mut BTreeSet<SemanticTypeId>,
    ) -> Option<()> {
        // OwnedString has the same three-word descriptor shape as Buffer, but
        // its payload is a UTF-8 allocation. `u32::MAX` is reserved in the
        // field-table depth slot for this direct string cleanup operation.
        if matches!(self.semantic.kind(value_ty), Some(TypeKind::OwnedString)) {
            owning_fields.push(RecordDropFieldInfo {
                payload_variant: u64::MAX,
                offset: base_offset,
                depth: u32::MAX,
                leaf: value_ty,
            });
            return Some(());
        }
        if let Some((depth, leaf)) = self.buffer_chain_leaf_depth(value_ty) {
            owning_fields.push(RecordDropFieldInfo {
                payload_variant: u64::MAX,
                offset: base_offset,
                depth,
                leaf,
            });
            return Some(());
        }
        // Option/Result keep their tag at the carrier start and their owning
        // Buffer or OwnedString payload at the same target-aligned offset.
        // Reuse the existing five-word field table; `u64::MAX` means
        // unconditional and all other variants are selected from the carrier
        // tag by the runtime.
        if matches!(
            self.semantic.kind(value_ty),
            Some(TypeKind::Option(_)) | Some(TypeKind::Result { .. })
        ) {
            let (primary, alternate, payload_offset) = self.carrier_buffer_drop_info(value_ty)?;
            owning_fields.push(RecordDropFieldInfo {
                payload_variant: u64::from(primary.variant),
                offset: base_offset.checked_add(payload_offset)?,
                depth: primary.depth,
                leaf: primary.leaf,
            });
            if let Some(alternate) = alternate {
                owning_fields.push(RecordDropFieldInfo {
                    payload_variant: u64::from(alternate.variant),
                    offset: base_offset.checked_add(payload_offset)?,
                    depth: alternate.depth,
                    leaf: alternate.leaf,
                });
            }
            return Some(());
        }
        match self.semantic.kind(value_ty).cloned() {
            Some(TypeKind::Array { element, length }) => {
                let (element_size, _) = self.semantic_layout(element)?;
                for index in 0..length {
                    let offset = base_offset.checked_add(index.checked_mul(element_size)?)?;
                    self.collect_record_drop_value_fields(
                        element,
                        offset,
                        owning_fields,
                        visiting,
                    )?;
                }
            }
            Some(TypeKind::Nominal { constructor, .. })
                if matches!(
                    self.layouts.get(&constructor).map(|layout| &layout.kind),
                    Some(NominalLayoutKind::Record { .. })
                ) =>
            {
                // Inline record fields are flattened recursively into the
                // parent table, including records nested in fixed arrays.
                self.collect_record_drop_fields(value_ty, base_offset, owning_fields, visiting)?;
            }
            _ => {}
        }
        Some(())
    }

    fn lower(&mut self, source: SemanticTypeId, span: Option<Span>) -> Result<TypeId, LowerError> {
        if let Some(lowered) = self.lowered.get(&source) {
            return Ok(*lowered);
        }
        let source_kind = self
            .semantic
            .kind(source)
            .cloned()
            .ok_or_else(|| LowerError {
                span,
                message: format!("semantic type #{} does not exist", source.index()),
            })?;
        if let TypeKind::Nominal {
            constructor,
            arguments,
        } = &source_kind
        {
            if *constructor == NominalTypeId::from_path("core.Region") {
                let lowered = self.intern(Type::RegionHandle);
                self.lowered.insert(source, lowered);
                return Ok(lowered);
            }
            return self.lower_nominal_forward(source, *constructor, arguments, span);
        }
        if !self.lowering.insert(source) {
            return Err(LowerError {
                span,
                message: "recursive non-nominal JIR type is invalid".to_owned(),
            });
        }
        let result = (|| {
            let kind = source_kind;
            let lowered = match &kind {
                TypeKind::Bool => self.intern(Type::Bool),
                TypeKind::Char => self.intern(Type::Integer {
                    signed: false,
                    bits: 32,
                }),
                TypeKind::Unit | TypeKind::Never => self.intern(Type::Unit),
                TypeKind::Integer { signedness, width } => self.intern(Type::Integer {
                    signed: *signedness == Signedness::Signed,
                    bits: integer_bits(*width, self.pointer_bits),
                }),
                TypeKind::Float(width) => self.intern(Type::Float {
                    bits: float_bits(*width),
                }),
                TypeKind::Vector { element, lanes } => {
                    let element = self.lower(*element, span)?;
                    self.intern(Type::Vector {
                        element,
                        lanes: *lanes,
                    })
                }
                TypeKind::Array { element, length } => {
                    let element = self.lower(*element, span)?;
                    self.intern(Type::Array {
                        element,
                        length: *length,
                    })
                }
                TypeKind::Pointer(inner) => {
                    let pointee = self.lower(*inner, span)?;
                    self.intern(Type::Pointer {
                        pointee,
                        address_space: AddressSpace::Generic,
                    })
                }
                TypeKind::Capability { capability, inner } => match capability {
                    Capability::Owned => self.lower(*inner, span)?,
                    Capability::Read => self.lower_borrow_capability(*inner, false, span)?,
                    Capability::Write => self.lower_write_capability(*inner, false, span)?,
                },
                TypeKind::Option(payload) => {
                    let payload = self.lower(*payload, span)?;
                    self.intern(Type::Enum {
                        variants: vec![Vec::new(), vec![payload]],
                    })
                }
                TypeKind::Result { ok, error } => {
                    let ok = self.lower(*ok, span)?;
                    let error = self.lower(*error, span)?;
                    self.intern(Type::Enum {
                        variants: vec![vec![error], vec![ok]],
                    })
                }
                TypeKind::Buffer(element) => {
                    self.lower_buffer(*element, AddressSpace::Heap, true, span)?
                }
                TypeKind::Slice(element) => {
                    self.lower_buffer(*element, AddressSpace::Generic, false, span)?
                }
                TypeKind::String => {
                    let byte = self.intern(Type::Integer {
                        signed: false,
                        bits: 8,
                    });
                    let data = self.intern(Type::Pointer {
                        pointee: byte,
                        address_space: AddressSpace::Global,
                    });
                    let size = self.intern(Type::Integer {
                        signed: false,
                        bits: self.pointer_bits,
                    });
                    self.intern(Type::Struct {
                        fields: vec![data, size],
                    })
                }
                TypeKind::OwnedString => {
                    self.lower_buffer(self.semantic.core().uint8, AddressSpace::Heap, true, span)?
                }
                TypeKind::Nominal { .. } => unreachable!("nominal types are handled above"),
                TypeKind::Function { parameters, result } => {
                    let parameters = parameters
                        .iter()
                        .map(|parameter| self.lower(*parameter, span))
                        .collect::<Result<Vec<_>, _>>()?;
                    let result = self.lower(*result, span)?;
                    self.intern(Type::Function { parameters, result })
                }
                TypeKind::Error
                | TypeKind::GenericParameter(_)
                | TypeKind::InferenceVariable(_) => {
                    return Err(LowerError {
                        span,
                        message: format!(
                            "JIR lowering does not yet support semantic type {kind:?}"
                        ),
                    });
                }
            };
            Ok(lowered)
        })();
        self.lowering.remove(&source);
        let lowered = result?;
        self.lowered.insert(source, lowered);
        Ok(lowered)
    }

    fn lower_buffer(
        &mut self,
        _element: SemanticTypeId,
        address_space: AddressSpace,
        owning: bool,
        span: Option<Span>,
    ) -> Result<TypeId, LowerError> {
        // Owning Buffer descriptors cross the generic C ABI as an opaque
        // `void *` pointer. Borrowed views (including Slice) keep their typed
        // pointer so existing slice/borrow contracts remain precise.
        let data_pointee = if owning {
            self.lower(self.semantic.core().uint8, span)?
        } else {
            self.lower(_element, span)?
        };
        let data = self.intern(Type::Pointer {
            pointee: data_pointee,
            address_space,
        });
        let size = self.intern(Type::Integer {
            signed: false,
            bits: self.pointer_bits,
        });
        let fields = if owning {
            vec![data, size, size]
        } else {
            vec![data, size]
        };
        Ok(self.intern(Type::Struct { fields }))
    }

    fn lower_borrow_capability(
        &mut self,
        inner: SemanticTypeId,
        _region_owned: bool,
        span: Option<Span>,
    ) -> Result<TypeId, LowerError> {
        let pointee = match self.semantic.kind(inner).cloned() {
            Some(TypeKind::Buffer(element) | TypeKind::Slice(element)) => {
                return self.lower_buffer(element, AddressSpace::Generic, false, span);
            }
            Some(TypeKind::String) => return self.lower(inner, span),
            Some(TypeKind::OwnedString) => return self.lower(self.semantic.core().string, span),
            _ => self.lower(inner, span)?,
        };
        Ok(self.pointer(pointee, AddressSpace::Generic))
    }

    /// Lowers a mutable Buffer capability as a pointer to the owning
    /// descriptor. A two-word borrowed view is sufficient for Slice and read
    /// access, but a mutable Buffer operation must be able to update the
    /// caller-owned length/capacity words after append/insert/resize.
    fn lower_write_capability(
        &mut self,
        inner: SemanticTypeId,
        region_owned: bool,
        span: Option<Span>,
    ) -> Result<TypeId, LowerError> {
        if let Some(TypeKind::Buffer(element)) = self.semantic.kind(inner).cloned() {
            let address_space = if region_owned {
                AddressSpace::Region
            } else {
                AddressSpace::Heap
            };
            let descriptor = self.lower_buffer(element, address_space, true, span)?;
            return Ok(self.pointer(descriptor, AddressSpace::Generic));
        }
        self.lower_borrow_capability(inner, region_owned, span)
    }

    fn lower_nominal_forward(
        &mut self,
        source: SemanticTypeId,
        constructor: NominalTypeId,
        arguments: &[SemanticTypeId],
        span: Option<Span>,
    ) -> Result<TypeId, LowerError> {
        let layout = self
            .layouts
            .get(&constructor)
            .cloned()
            .ok_or_else(|| LowerError {
                span,
                message: format!("missing nominal layout for {constructor:?}"),
            })?;
        let identity = constructor.fingerprint().as_u64();
        let placeholder = match layout.kind {
            NominalLayoutKind::Record { .. } => Type::NominalStruct {
                identity,
                fields: Vec::new(),
            },
            NominalLayoutKind::Enum { .. } => Type::NominalEnum {
                identity,
                variants: Vec::new(),
            },
        };
        let id = TypeId::new(self.types.len());
        self.types.push(placeholder);
        self.lowered.insert(source, id);
        let result = self.build_nominal(constructor, arguments, span);
        match result {
            Ok(ty) => {
                self.types[id.index()] = ty;
                Ok(id)
            }
            Err(error) => {
                self.lowered.remove(&source);
                Err(error)
            }
        }
    }

    fn build_nominal(
        &mut self,
        constructor: NominalTypeId,
        arguments: &[SemanticTypeId],
        span: Option<Span>,
    ) -> Result<Type, LowerError> {
        let layout = self
            .layouts
            .get(&constructor)
            .cloned()
            .ok_or_else(|| LowerError {
                span,
                message: format!("missing nominal layout for {constructor:?}"),
            })?;
        if layout.generic_parameters.len() != arguments.len() {
            return Err(LowerError {
                span,
                message: "nominal layout generic argument count differs from type".to_owned(),
            });
        }
        let mut substitution = Substitution::new();
        for (parameter, argument) in layout.generic_parameters.iter().zip(arguments) {
            substitution.insert(*parameter, *argument);
        }
        let mut lower_field = |field: SemanticTypeId| -> Result<TypeId, LowerError> {
            let concrete = substitution
                .apply(&mut self.semantic, field)
                .map_err(|error| LowerError {
                    span,
                    message: format!("cannot instantiate nominal field type: {error:?}"),
                })?;
            self.lower(concrete, span)
        };
        match layout.kind {
            NominalLayoutKind::Record { fields } => {
                let fields = fields
                    .into_iter()
                    .map(|field| lower_field(field.ty))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Type::NominalStruct {
                    identity: constructor.fingerprint().as_u64(),
                    fields,
                })
            }
            NominalLayoutKind::Enum { variants } => {
                let variants = variants
                    .into_iter()
                    .map(|variant| {
                        variant
                            .fields
                            .into_iter()
                            .map(&mut lower_field)
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Type::NominalEnum {
                    identity: constructor.fingerprint().as_u64(),
                    variants,
                })
            }
        }
    }

    fn record_fields(
        &mut self,
        ty: SemanticTypeId,
        span: Option<Span>,
    ) -> Result<Vec<(String, SemanticTypeId)>, LowerError> {
        let Some(TypeKind::Nominal {
            constructor,
            arguments,
        }) = self.semantic.kind(ty).cloned()
        else {
            return Err(LowerError {
                span,
                message: "field operation requires a nominal record type".to_owned(),
            });
        };
        let layout = self
            .layouts
            .get(&constructor)
            .cloned()
            .ok_or_else(|| LowerError {
                span,
                message: format!("missing nominal layout for {constructor:?}"),
            })?;
        let NominalLayoutKind::Record { fields } = layout.kind else {
            return Err(LowerError {
                span,
                message: "field operation cannot use an enum layout".to_owned(),
            });
        };
        let mut substitution = Substitution::new();
        for (parameter, argument) in layout.generic_parameters.iter().zip(arguments.iter()) {
            substitution.insert(*parameter, *argument);
        }
        fields
            .into_iter()
            .map(|field| {
                substitution
                    .apply(&mut self.semantic, field.ty)
                    .map(|ty| (field.name, ty))
                    .map_err(|error| LowerError {
                        span,
                        message: format!("cannot instantiate record field type: {error:?}"),
                    })
            })
            .collect()
    }

    fn field_index_and_type(
        &mut self,
        ty: SemanticTypeId,
        field: &str,
        span: Option<Span>,
    ) -> Result<(u32, SemanticTypeId), LowerError> {
        self.record_fields(ty, span)?
            .into_iter()
            .enumerate()
            .find(|(_, (name, _))| name == field)
            .map(|(index, (_, ty))| (index as u32, ty))
            .ok_or_else(|| LowerError {
                span,
                message: format!("record layout has no field {field:?}"),
            })
    }

    fn enum_variants(
        &mut self,
        ty: SemanticTypeId,
        span: Option<Span>,
    ) -> Result<Vec<(String, Vec<SemanticTypeId>)>, LowerError> {
        match self.semantic.kind(ty).cloned() {
            Some(TypeKind::Option(payload)) => Ok(vec![
                ("None".to_owned(), Vec::new()),
                ("Some".to_owned(), vec![payload]),
            ]),
            Some(TypeKind::Result { ok, error }) => Ok(vec![
                ("Error".to_owned(), vec![error]),
                ("Ok".to_owned(), vec![ok]),
            ]),
            Some(TypeKind::Nominal {
                constructor,
                arguments,
            }) => {
                let layout = self
                    .layouts
                    .get(&constructor)
                    .cloned()
                    .ok_or_else(|| LowerError {
                        span,
                        message: format!("missing nominal layout for {constructor:?}"),
                    })?;
                let NominalLayoutKind::Enum { variants } = layout.kind else {
                    return Err(LowerError {
                        span,
                        message: "enum operation requires an enum nominal layout".to_owned(),
                    });
                };
                let mut substitution = Substitution::new();
                for (parameter, argument) in layout.generic_parameters.iter().zip(arguments.iter())
                {
                    substitution.insert(*parameter, *argument);
                }
                variants
                    .into_iter()
                    .map(|variant| {
                        let fields = variant
                            .fields
                            .into_iter()
                            .map(|field| {
                                substitution
                                    .apply(&mut self.semantic, field)
                                    .map_err(|error| LowerError {
                                        span,
                                        message: format!(
                                            "cannot instantiate enum payload type: {error:?}"
                                        ),
                                    })
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        Ok((variant.name, fields))
                    })
                    .collect()
            }
            _ => Err(LowerError {
                span,
                message: "enum operation requires Option, Result, or an enum nominal".to_owned(),
            }),
        }
    }

    fn variant_index_and_fields(
        &mut self,
        ty: SemanticTypeId,
        path: &str,
        span: Option<Span>,
    ) -> Result<(u32, Vec<SemanticTypeId>), LowerError> {
        let name = path.rsplit('.').next().unwrap_or(path);
        self.enum_variants(ty, span)?
            .into_iter()
            .enumerate()
            .find(|(_, (candidate, _))| candidate == name)
            .map(|(index, (_, fields))| (index as u32, fields))
            .ok_or_else(|| LowerError {
                span,
                message: format!("enum layout has no variant {path:?}"),
            })
    }

    fn intern(&mut self, ty: Type) -> TypeId {
        if let Some(index) = self.types.iter().position(|candidate| candidate == &ty) {
            return TypeId::new(index);
        }
        let id = TypeId::new(self.types.len());
        self.types.push(ty);
        id
    }

    fn stack_pointer(&mut self, pointee: TypeId) -> TypeId {
        self.pointer(pointee, AddressSpace::Stack)
    }

    fn pointer(&mut self, pointee: TypeId, address_space: AddressSpace) -> TypeId {
        self.intern(Type::Pointer {
            pointee,
            address_space,
        })
    }

    fn is_unit(&self, ty: TypeId) -> bool {
        matches!(self.types.get(ty.index()), Some(Type::Unit))
    }
}

struct FunctionLowerer<'a, 'types> {
    source: &'a MirFunction,
    id: FunctionId,
    call_targets: &'a CallTargets,
    type_table: &'types mut TypeTable,
    next_value: usize,
    local_addresses: Vec<ValueId>,
    local_types: Vec<TypeId>,
    prelude: Vec<Instruction>,
    synthetic_blocks: Vec<Block>,
    errors: Vec<LowerError>,
}

#[derive(Clone, Copy)]
struct PatternTargets {
    matched: BlockId,
    otherwise: BlockId,
}

impl<'a, 'types> FunctionLowerer<'a, 'types> {
    fn new(
        source: &'a MirFunction,
        id: FunctionId,
        call_targets: &'a CallTargets,
        type_table: &'types mut TypeTable,
    ) -> Self {
        Self {
            source,
            id,
            call_targets,
            type_table,
            next_value: source
                .locals
                .iter()
                .filter(|local| local.is_parameter)
                .count(),
            local_addresses: Vec::new(),
            local_types: Vec::new(),
            prelude: Vec::new(),
            synthetic_blocks: Vec::new(),
            errors: Vec::new(),
        }
    }

    fn lower(mut self) -> Result<Function, Vec<LowerError>> {
        let (parameter_types, result_source) =
            match self.type_table.semantic.kind(self.source.signature) {
                Some(TypeKind::Function { parameters, result }) => (parameters.clone(), *result),
                kind => {
                    return Err(vec![self.error(
                        Some(self.source.span),
                        format!("MIR function signature is not a function type: {kind:?}"),
                    )]);
                }
            };
        let mut parameters = Vec::with_capacity(parameter_types.len());
        for (index, source_ty) in parameter_types.iter().enumerate() {
            match self.type_table.lower(*source_ty, Some(self.source.span)) {
                Ok(ty) => parameters.push(Parameter {
                    value: ValueId::new(index),
                    ty,
                    name: self
                        .source
                        .locals
                        .iter()
                        .filter(|local| local.is_parameter)
                        .nth(index)
                        .map(|local| local.name.clone()),
                }),
                Err(error) => self.errors.push(error),
            }
        }
        let result = match self.type_table.lower(result_source, Some(self.source.span)) {
            Ok(result) => result,
            Err(error) => {
                self.errors.push(error);
                self.type_table.intern(Type::Unit)
            }
        };
        self.prepare_locals(&parameters);
        self.lower_disjoint_contract();

        let mut blocks = Vec::with_capacity(self.source.blocks.len());
        for block in &self.source.blocks {
            let mut instructions = if block.id.index() == 0 {
                std::mem::take(&mut self.prelude)
            } else {
                Vec::new()
            };
            for statement in &block.statements {
                self.lower_statement(statement, &mut instructions);
            }
            let terminator = self.lower_terminator(&block.terminator, &mut instructions);
            blocks.push(Block {
                id: BlockId::new(block.id.index()),
                parameters: Vec::new(),
                instructions,
                terminator,
                span: Some(self.source.span),
            });
        }
        blocks.append(&mut self.synthetic_blocks);
        if self.errors.is_empty() {
            let (name, linkage) = self.source.export.as_ref().map_or_else(
                || (self.source.name.clone(), Linkage::Internal),
                |export| (export.name.clone(), Linkage::Export),
            );
            Ok(Function {
                id: self.id,
                name,
                linkage,
                parameters,
                result,
                blocks,
                span: Some(self.source.span),
            })
        } else {
            Err(self.errors)
        }
    }

    fn prepare_locals(&mut self, parameters: &[Parameter]) {
        let mut parameter_index = 0;
        for local in &self.source.locals {
            let lowered_local = match self.type_table.semantic.kind(local.ty).cloned() {
                Some(TypeKind::Buffer(element)) if local.owned_region.is_some() => self
                    .type_table
                    .lower_buffer(element, AddressSpace::Region, true, Some(local.span)),
                Some(TypeKind::Capability { capability, inner })
                    if local.owned_region.is_some() =>
                {
                    match capability {
                        Capability::Write => {
                            self.type_table
                                .lower_write_capability(inner, true, Some(local.span))
                        }
                        Capability::Read => {
                            self.type_table
                                .lower_borrow_capability(inner, true, Some(local.span))
                        }
                        Capability::Owned => self.type_table.lower(inner, Some(local.span)),
                    }
                }
                _ => self.type_table.lower(local.ty, Some(local.span)),
            };
            let ty = match lowered_local {
                Ok(ty) => ty,
                Err(error) => {
                    self.errors.push(error);
                    self.local_types.push(self.type_table.intern(Type::Unit));
                    self.local_addresses.push(ValueId::new(0));
                    continue;
                }
            };
            let pointer_ty = self.type_table.stack_pointer(ty);
            let address = self.new_value();
            self.prelude.push(Instruction {
                result: Some(TypedValue {
                    value: address,
                    ty: pointer_ty,
                }),
                kind: InstructionKind::StackAlloc { ty, count: None },
                span: Some(local.span),
            });
            self.local_types.push(ty);
            self.local_addresses.push(address);
            if local.is_parameter {
                if let Some(parameter) = parameters.get(parameter_index) {
                    self.prelude.push(Instruction {
                        result: None,
                        kind: InstructionKind::Store {
                            pointer: address,
                            value: parameter.value,
                            alignment: 1,
                            volatile: false,
                        },
                        span: Some(local.span),
                    });
                }
                parameter_index += 1;
            }
        }
    }

    fn lower_disjoint_contract(&mut self) {
        if !self.source.disjoint {
            return;
        }
        let values: Vec<_> = self
            .source
            .locals
            .iter()
            .filter(|local| local.is_parameter)
            .enumerate()
            .filter(|(_, local)| self.is_disjoint_borrow(local.ty))
            .map(|(index, _)| ValueId::new(index))
            .collect();
        for (index, left) in values.iter().enumerate() {
            for right in values.iter().skip(index + 1) {
                self.prelude.push(Instruction {
                    result: None,
                    kind: InstructionKind::AssumeNoAlias {
                        left: *left,
                        right: *right,
                    },
                    span: Some(self.source.span),
                });
            }
        }
    }

    fn is_disjoint_borrow(&self, ty: SemanticTypeId) -> bool {
        let Some(TypeKind::Capability { capability, inner }) = self.type_table.semantic.kind(ty)
        else {
            return false;
        };
        if !matches!(capability, Capability::Read | Capability::Write) {
            return false;
        }
        matches!(
            self.type_table.semantic.kind(*inner),
            Some(TypeKind::Slice(_) | TypeKind::Buffer(_))
        )
    }

    fn lower_statement(&mut self, statement: &MirStatement, instructions: &mut Vec<Instruction>) {
        match statement {
            MirStatement::StorageLive { .. } | MirStatement::StorageDead { .. } => {}
            MirStatement::Assign {
                destination,
                destination_indices,
                value,
                span,
                ..
            } => {
                let Some(value) = value else {
                    self.errors
                        .push(self.error(Some(*span), "MIR assignment has no value"));
                    return;
                };
                let Some(value) = self.lower_operand(value, instructions) else {
                    return;
                };
                let Some(value) = value else {
                    return;
                };
                let Some(pointer) = self.lower_place_address(
                    destination,
                    destination_indices,
                    instructions,
                    Some(*span),
                ) else {
                    return;
                };
                instructions.push(Instruction {
                    result: None,
                    kind: InstructionKind::Store {
                        pointer,
                        value,
                        alignment: 1,
                        volatile: false,
                    },
                    span: Some(*span),
                });
            }
            MirStatement::Evaluate { value, span, .. } => {
                if let Some(value) = value {
                    self.lower_operand(value, instructions);
                } else {
                    self.errors
                        .push(self.error(Some(*span), "MIR evaluation has no value"));
                }
            }
            MirStatement::Borrow {
                destination,
                source,
                span,
                ..
            } => {
                let Some(destination_local) = self.source.locals.get(destination.index()) else {
                    self.errors
                        .push(self.error(Some(*span), "borrow destination local does not exist"));
                    return;
                };
                let Some(TypeKind::Capability { inner, .. }) =
                    self.type_table.semantic.kind(destination_local.ty).cloned()
                else {
                    self.errors.push(
                        self.error(Some(*span), "borrow destination is not a capability type"),
                    );
                    return;
                };
                let Some(borrowed) =
                    self.lower_borrow_from_place(source, &[], inner, instructions, *span)
                else {
                    return;
                };
                let Some(pointer) = self.local_addresses.get(destination.index()).copied() else {
                    self.errors
                        .push(self.error(Some(*span), "borrow destination storage does not exist"));
                    return;
                };
                instructions.push(Instruction {
                    result: None,
                    kind: InstructionKind::Store {
                        pointer,
                        value: borrowed,
                        alignment: 1,
                        volatile: false,
                    },
                    span: Some(*span),
                });
            }
            MirStatement::RegionEnter { region, span } => {
                let region_ty = self.type_table.intern(Type::RegionHandle);
                let handle = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: handle,
                        ty: region_ty,
                    }),
                    kind: InstructionKind::RegionCreate,
                    span: Some(*span),
                });
                let Some(pointer) = self.local_addresses.get(region.index()).copied() else {
                    self.errors
                        .push(self.error(Some(*span), "region storage does not exist"));
                    return;
                };
                instructions.push(Instruction {
                    result: None,
                    kind: InstructionKind::Store {
                        pointer,
                        value: handle,
                        alignment: 1,
                        volatile: false,
                    },
                    span: Some(*span),
                });
            }
            MirStatement::RegionExit { region, span } => {
                let Some((region, _)) =
                    self.load_source_place(&Place::local(*region), &[], instructions, *span)
                else {
                    return;
                };
                instructions.push(Instruction {
                    result: None,
                    kind: InstructionKind::RegionDestroy { region },
                    span: Some(*span),
                });
            }
            MirStatement::Drop { place, span } => {
                let Some((value, _)) = self.load_source_place(place, &[], instructions, *span)
                else {
                    return;
                };
                let owned_string =
                    self.source
                        .locals
                        .get(place.local.index())
                        .is_some_and(|local| {
                            matches!(
                                self.type_table.semantic.kind(local.ty),
                                Some(TypeKind::OwnedString)
                            )
                        });
                let record_buffer_fields = self
                    .source
                    .locals
                    .get(place.local.index())
                    .filter(|local| local.owned_region.is_none())
                    .and_then(|local| {
                        if !place.projection.is_empty() {
                            return None;
                        }
                        let TypeKind::Buffer(element) =
                            self.type_table.semantic.kind(local.ty).cloned()?
                        else {
                            return None;
                        };
                        let fields = self.type_table.record_drop_fields_info(element)?;
                        let element = self.type_table.lower(element, Some(*span)).ok()?;
                        let fields = fields
                            .into_iter()
                            .map(|field| {
                                Some(RecordDropField {
                                    payload_variant: field.payload_variant,
                                    payload_offset: field.offset,
                                    depth: field.depth,
                                    leaf_element: self
                                        .type_table
                                        .lower(field.leaf, Some(*span))
                                        .ok()?,
                                })
                            })
                            .collect::<Option<Vec<_>>>()?;
                        Some((element, fields))
                    });
                // Option/Result carriers with a direct OwnedString payload
                // use the same field-table runtime as owning records. Keep
                // them on that path so the reserved string-depth marker is
                // handled by the string-aware destructor rather than the
                // legacy Buffer-only carrier instruction.
                let string_carrier_buffer_fields = self
                    .source
                    .locals
                    .get(place.local.index())
                    .filter(|local| local.owned_region.is_none())
                    .and_then(|local| {
                        if !place.projection.is_empty() {
                            return None;
                        }
                        let TypeKind::Buffer(carrier) =
                            self.type_table.semantic.kind(local.ty).cloned()?
                        else {
                            return None;
                        };
                        if !matches!(
                            self.type_table.semantic.kind(carrier),
                            Some(TypeKind::Option(_)) | Some(TypeKind::Result { .. })
                        ) {
                            return None;
                        }
                        let fields = self.type_table.buffer_record_drop_fields_info(carrier)?;
                        if !fields.iter().any(|field| field.depth == u32::MAX) {
                            return None;
                        }
                        let element = self.type_table.lower(carrier, Some(*span)).ok()?;
                        let fields = fields
                            .into_iter()
                            .map(|field| {
                                Some(RecordDropField {
                                    payload_variant: field.payload_variant,
                                    payload_offset: field.offset,
                                    depth: field.depth,
                                    leaf_element: self
                                        .type_table
                                        .lower(field.leaf, Some(*span))
                                        .ok()?,
                                })
                            })
                            .collect::<Option<Vec<_>>>()?;
                        Some((element, fields))
                    });
                let recursive_record_buffer_fields = self
                    .source
                    .locals
                    .get(place.local.index())
                    .filter(|local| local.owned_region.is_none())
                    .and_then(|local| {
                        if !place.projection.is_empty() {
                            return None;
                        }
                        let TypeKind::Buffer(outer_element) =
                            self.type_table.semantic.kind(local.ty).cloned()?
                        else {
                            return None;
                        };
                        let (depth, record_ty, fields) = self
                            .type_table
                            .nested_record_drop_fields_info(outer_element)?;
                        let element = self.type_table.lower(outer_element, Some(*span)).ok()?;
                        let record_element = self.type_table.lower(record_ty, Some(*span)).ok()?;
                        let fields = fields
                            .into_iter()
                            .map(|field| {
                                Some(RecordDropField {
                                    payload_variant: field.payload_variant,
                                    payload_offset: field.offset,
                                    depth: field.depth,
                                    leaf_element: self
                                        .type_table
                                        .lower(field.leaf, Some(*span))
                                        .ok()?,
                                })
                            })
                            .collect::<Option<Vec<_>>>()?;
                        Some((element, record_element, fields, depth))
                    });
                let record_owning_fields = self
                    .source
                    .locals
                    .get(place.local.index())
                    .filter(|local| local.owned_region.is_none())
                    .and_then(|local| {
                        if !place.projection.is_empty() {
                            return None;
                        }
                        let fields = self.type_table.record_drop_fields_info(local.ty)?;
                        let element = self.type_table.lower(local.ty, Some(*span)).ok()?;
                        let fields = fields
                            .into_iter()
                            .map(|field| {
                                Some(RecordDropField {
                                    payload_variant: field.payload_variant,
                                    payload_offset: field.offset,
                                    depth: field.depth,
                                    leaf_element: self
                                        .type_table
                                        .lower(field.leaf, Some(*span))
                                        .ok()?,
                                })
                            })
                            .collect::<Option<Vec<_>>>()?;
                        Some((element, fields))
                    });
                let owned_buffer = self
                    .source
                    .locals
                    .get(place.local.index())
                    .filter(|local| local.owned_region.is_none())
                    .and_then(
                        |local| match self.type_table.semantic.kind(local.ty).cloned() {
                            Some(TypeKind::Buffer(element)) if place.projection.is_empty() => {
                                self.type_table.lower(element, Some(*span)).ok()
                            }
                            _ => None,
                        },
                    );
                let owned_string_buffer = self
                    .source
                    .locals
                    .get(place.local.index())
                    .filter(|local| local.owned_region.is_none())
                    .and_then(|local| {
                        if !place.projection.is_empty() {
                            return None;
                        }
                        let TypeKind::Buffer(element) =
                            self.type_table.semantic.kind(local.ty).cloned()?
                        else {
                            return None;
                        };
                        if !matches!(
                            self.type_table.semantic.kind(element),
                            Some(TypeKind::OwnedString)
                        ) {
                            return None;
                        }
                        self.type_table.lower(element, Some(*span)).ok()
                    });
                let nested_owned_buffer = self
                    .source
                    .locals
                    .get(place.local.index())
                    .filter(|local| local.owned_region.is_none())
                    .and_then(|local| {
                        if !place.projection.is_empty() {
                            return None;
                        }
                        let Some(TypeKind::Buffer(element)) =
                            self.type_table.semantic.kind(local.ty).cloned()
                        else {
                            return None;
                        };
                        let TypeKind::Buffer(nested_element) =
                            self.type_table.semantic.kind(element).cloned()?
                        else {
                            return None;
                        };
                        let element = self.type_table.lower(element, Some(*span)).ok()?;
                        let nested_element =
                            self.type_table.lower(nested_element, Some(*span)).ok()?;
                        Some((element, nested_element))
                    });
                let recursive_owned_string_buffer = self
                    .source
                    .locals
                    .get(place.local.index())
                    .filter(|local| local.owned_region.is_none())
                    .and_then(|local| {
                        if !place.projection.is_empty() {
                            return None;
                        }
                        let TypeKind::Buffer(outer_element) =
                            self.type_table.semantic.kind(local.ty).cloned()?
                        else {
                            return None;
                        };
                        let mut cursor = outer_element;
                        let mut depth = 0usize;
                        while let Some(TypeKind::Buffer(inner)) =
                            self.type_table.semantic.kind(cursor)
                        {
                            depth = depth.checked_add(1)?;
                            cursor = *inner;
                        }
                        if depth == 0
                            || !matches!(
                                self.type_table.semantic.kind(cursor),
                                Some(TypeKind::OwnedString)
                            )
                        {
                            return None;
                        }
                        let depth = u32::try_from(depth).ok()?;
                        let element = self.type_table.lower(outer_element, Some(*span)).ok()?;
                        let string_element = self.type_table.lower(cursor, Some(*span)).ok()?;
                        Some((element, string_element, depth))
                    });
                let recursive_owned_buffer = self
                    .source
                    .locals
                    .get(place.local.index())
                    .filter(|local| local.owned_region.is_none())
                    .and_then(|local| {
                        if !place.projection.is_empty() {
                            return None;
                        }
                        let TypeKind::Buffer(outer_element) =
                            self.type_table.semantic.kind(local.ty).cloned()?
                        else {
                            return None;
                        };
                        let mut cursor = outer_element;
                        let mut depth = 0usize;
                        while let Some(TypeKind::Buffer(inner)) =
                            self.type_table.semantic.kind(cursor)
                        {
                            depth = depth.checked_add(1)?;
                            cursor = *inner;
                        }
                        if depth < 2 {
                            return None;
                        }
                        let depth = u32::try_from(depth).ok()?;
                        let element = self.type_table.lower(outer_element, Some(*span)).ok()?;
                        let leaf_element = self.type_table.lower(cursor, Some(*span)).ok()?;
                        Some((element, leaf_element, depth))
                    });
                let enum_carrier_owned_buffer = self
                    .source
                    .locals
                    .get(place.local.index())
                    .filter(|local| local.owned_region.is_none())
                    .and_then(|local| {
                        if !place.projection.is_empty() {
                            return None;
                        }
                        let TypeKind::Buffer(carrier) =
                            self.type_table.semantic.kind(local.ty).cloned()?
                        else {
                            return None;
                        };
                        let (branches, payload_offset) =
                            self.type_table.enum_carrier_drop_info(carrier)?;
                        if branches.len() < 2 {
                            return None;
                        }
                        let fields = self.type_table.enum_carrier_drop_fields_info(carrier)?;
                        if fields.len() != branches.len()
                            || fields.iter().any(|field| {
                                field.offset != payload_offset
                                    || !branches.iter().any(|branch| {
                                        branch.variant == field.variant
                                            && branch.depth == field.depth
                                            && branch.leaf == field.leaf
                                    })
                            })
                        {
                            return None;
                        }
                        let element = self.type_table.lower(carrier, Some(*span)).ok()?;
                        let branches = branches
                            .into_iter()
                            .map(|branch| {
                                Some(CarrierDropBranch {
                                    payload_variant: branch.variant,
                                    depth: branch.depth,
                                    leaf_element: self
                                        .type_table
                                        .lower(branch.leaf, Some(*span))
                                        .ok()?,
                                })
                            })
                            .collect::<Option<Vec<_>>>()?;
                        Some((element, payload_offset, branches))
                    });
                let enum_carrier_fields_buffer = self
                    .source
                    .locals
                    .get(place.local.index())
                    .filter(|local| local.owned_region.is_none())
                    .and_then(|local| {
                        if !place.projection.is_empty() {
                            return None;
                        }
                        let TypeKind::Buffer(carrier) =
                            self.type_table.semantic.kind(local.ty).cloned()?
                        else {
                            return None;
                        };
                        let fields = self.type_table.enum_carrier_drop_fields_info(carrier)?;
                        let use_new_plan = match self.type_table.enum_carrier_drop_info(carrier) {
                            Some((branches, payload_offset)) => {
                                fields.len() != branches.len()
                                    || fields.iter().any(|field| {
                                        field.offset != payload_offset
                                            || !branches.iter().any(|branch| {
                                                branch.variant == field.variant
                                                    && branch.depth == field.depth
                                                    && branch.leaf == field.leaf
                                            })
                                    })
                            }
                            None => true,
                        };
                        if !use_new_plan {
                            return None;
                        }
                        let element = self.type_table.lower(carrier, Some(*span)).ok()?;
                        let fields = fields
                            .into_iter()
                            .map(|field| {
                                Some(CarrierDropField {
                                    payload_variant: field.variant,
                                    payload_offset: field.offset,
                                    depth: field.depth,
                                    leaf_element: self
                                        .type_table
                                        .lower(field.leaf, Some(*span))
                                        .ok()?,
                                })
                            })
                            .collect::<Option<Vec<_>>>()?;
                        Some((element, fields))
                    });
                let carrier_owned_buffer = self
                    .source
                    .locals
                    .get(place.local.index())
                    .filter(|local| local.owned_region.is_none())
                    .and_then(|local| {
                        if !place.projection.is_empty() {
                            return None;
                        }
                        let TypeKind::Buffer(carrier) =
                            self.type_table.semantic.kind(local.ty).cloned()?
                        else {
                            return None;
                        };
                        let (branch, alternate, payload_offset) =
                            self.type_table.carrier_buffer_drop_info(carrier)?;
                        let element = self.type_table.lower(carrier, Some(*span)).ok()?;
                        let leaf_element = self.type_table.lower(branch.leaf, Some(*span)).ok()?;
                        let alternate_leaf_element = alternate.and_then(|branch| {
                            self.type_table.lower(branch.leaf, Some(*span)).ok()
                        });
                        Some((
                            element,
                            leaf_element,
                            branch.variant,
                            payload_offset,
                            branch.depth,
                            alternate_leaf_element,
                            alternate.map(|branch| branch.variant),
                            alternate.map(|branch| branch.depth),
                        ))
                    });
                let owning_carrier = self
                    .source
                    .locals
                    .get(place.local.index())
                    .filter(|local| local.owned_region.is_none())
                    .and_then(|local| {
                        if !place.projection.is_empty() {
                            return None;
                        }
                        let (branch, alternate, payload_offset) =
                            self.type_table.carrier_buffer_drop_info(local.ty)?;
                        let element = self.type_table.lower(local.ty, Some(*span)).ok()?;
                        let leaf_element = self.type_table.lower(branch.leaf, Some(*span)).ok()?;
                        let alternate_leaf_element = alternate.and_then(|branch| {
                            self.type_table.lower(branch.leaf, Some(*span)).ok()
                        });
                        Some((
                            element,
                            leaf_element,
                            branch.variant,
                            payload_offset,
                            branch.depth,
                            alternate_leaf_element,
                            alternate.map(|branch| branch.variant),
                            alternate.map(|branch| branch.depth),
                        ))
                    });
                let enum_owning_carrier = self
                    .source
                    .locals
                    .get(place.local.index())
                    .filter(|local| local.owned_region.is_none())
                    .and_then(|local| {
                        if !place.projection.is_empty() {
                            return None;
                        }
                        let (branches, payload_offset) =
                            self.type_table.enum_carrier_drop_info(local.ty)?;
                        if branches.len() < 2 {
                            return None;
                        }
                        let fields = self.type_table.enum_carrier_drop_fields_info(local.ty)?;
                        if fields.len() != branches.len()
                            || fields.iter().any(|field| {
                                field.offset != payload_offset
                                    || !branches.iter().any(|branch| {
                                        branch.variant == field.variant
                                            && branch.depth == field.depth
                                            && branch.leaf == field.leaf
                                    })
                            })
                        {
                            return None;
                        }
                        let element = self.type_table.lower(local.ty, Some(*span)).ok()?;
                        let branches = branches
                            .into_iter()
                            .map(|branch| {
                                Some(CarrierDropBranch {
                                    payload_variant: branch.variant,
                                    depth: branch.depth,
                                    leaf_element: self
                                        .type_table
                                        .lower(branch.leaf, Some(*span))
                                        .ok()?,
                                })
                            })
                            .collect::<Option<Vec<_>>>()?;
                        Some((element, payload_offset, branches))
                    });
                let enum_owning_fields = self
                    .source
                    .locals
                    .get(place.local.index())
                    .filter(|local| local.owned_region.is_none())
                    .and_then(|local| {
                        if !place.projection.is_empty() {
                            return None;
                        }
                        let fields = self.type_table.enum_carrier_drop_fields_info(local.ty)?;
                        let use_new_plan = match self.type_table.enum_carrier_drop_info(local.ty) {
                            Some((branches, payload_offset)) => {
                                fields.len() != branches.len()
                                    || fields.iter().any(|field| {
                                        field.offset != payload_offset
                                            || !branches.iter().any(|branch| {
                                                branch.variant == field.variant
                                                    && branch.depth == field.depth
                                                    && branch.leaf == field.leaf
                                            })
                                    })
                            }
                            None => true,
                        };
                        if !use_new_plan {
                            return None;
                        }
                        let element = self.type_table.lower(local.ty, Some(*span)).ok()?;
                        let fields = fields
                            .into_iter()
                            .map(|field| {
                                Some(CarrierDropField {
                                    payload_variant: field.variant,
                                    payload_offset: field.offset,
                                    depth: field.depth,
                                    leaf_element: self
                                        .type_table
                                        .lower(field.leaf, Some(*span))
                                        .ok()?,
                                })
                            })
                            .collect::<Option<Vec<_>>>()?;
                        Some((element, fields))
                    });
                instructions.push(Instruction {
                    result: None,
                    kind: if owned_string {
                        InstructionKind::OwnedStringDrop { value }
                    } else if let Some((element, payload_offset, branches)) = enum_owning_carrier {
                        InstructionKind::EnumOwningCarrierDrop {
                            value,
                            element,
                            payload_offset,
                            branches,
                        }
                    } else if let Some((element, fields)) = enum_owning_fields {
                        InstructionKind::EnumOwningFieldsDrop {
                            value,
                            element,
                            fields,
                        }
                    } else if let Some((element, payload_offset, branches)) =
                        enum_carrier_owned_buffer
                    {
                        InstructionKind::EnumCarrierBufferDrop {
                            value,
                            element,
                            payload_offset,
                            branches,
                        }
                    } else if let Some((element, fields)) = enum_carrier_fields_buffer {
                        InstructionKind::EnumCarrierFieldsDrop {
                            value,
                            element,
                            fields,
                        }
                    } else if let Some((element, fields)) = record_owning_fields {
                        InstructionKind::RecordOwningFieldsDrop {
                            value,
                            element,
                            fields,
                        }
                    } else if let Some((element, fields)) = record_buffer_fields {
                        InstructionKind::RecordBufferFieldsDrop {
                            value,
                            element,
                            fields,
                        }
                    } else if let Some((element, fields)) = string_carrier_buffer_fields {
                        InstructionKind::RecordBufferFieldsDrop {
                            value,
                            element,
                            fields,
                        }
                    } else if let Some((
                        element,
                        leaf_element,
                        payload_variant,
                        payload_offset,
                        depth,
                        alternate_leaf_element,
                        alternate_payload_variant,
                        alternate_depth,
                    )) = owning_carrier
                    {
                        InstructionKind::OwningCarrierDrop {
                            value,
                            element,
                            leaf_element,
                            payload_variant,
                            payload_offset,
                            depth,
                            alternate_leaf_element,
                            alternate_payload_variant,
                            alternate_depth,
                        }
                    } else if let Some((
                        element,
                        leaf_element,
                        payload_variant,
                        payload_offset,
                        depth,
                        alternate_leaf_element,
                        alternate_payload_variant,
                        alternate_depth,
                    )) = carrier_owned_buffer
                    {
                        InstructionKind::CarrierBufferDrop {
                            value,
                            element,
                            leaf_element,
                            payload_variant,
                            payload_offset,
                            depth,
                            alternate_leaf_element,
                            alternate_payload_variant,
                            alternate_depth,
                        }
                    } else if let Some((element, record_element, fields, depth)) =
                        recursive_record_buffer_fields
                    {
                        InstructionKind::RecursiveRecordBufferFieldsDrop {
                            value,
                            element,
                            record_element,
                            fields,
                            depth,
                        }
                    } else if let Some((element, string_element, depth)) =
                        recursive_owned_string_buffer
                    {
                        InstructionKind::RecursiveOwnedStringBufferDrop {
                            value,
                            element,
                            string_element,
                            depth,
                        }
                    } else if let Some((element, leaf_element, depth)) = recursive_owned_buffer {
                        InstructionKind::RecursiveBufferDrop {
                            value,
                            element,
                            leaf_element,
                            depth,
                        }
                    } else if let Some((element, nested_element)) = nested_owned_buffer {
                        InstructionKind::NestedBufferDrop {
                            value,
                            element,
                            nested_element,
                        }
                    } else if let Some(element) = owned_buffer {
                        if let Some(element) = owned_string_buffer {
                            InstructionKind::OwnedStringBufferDrop { value, element }
                        } else {
                            InstructionKind::BufferDrop { value, element }
                        }
                    } else {
                        InstructionKind::Drop { value }
                    },
                    span: Some(*span),
                });
            }
        }
    }

    fn lower_terminator(
        &mut self,
        terminator: &MirTerminator,
        instructions: &mut Vec<Instruction>,
    ) -> Terminator {
        match terminator {
            MirTerminator::Goto { target, .. } => Terminator::Jump {
                target: lower_block(*target),
                arguments: Vec::new(),
            },
            MirTerminator::Switch {
                value,
                targets,
                otherwise,
                span,
                ..
            } => {
                let condition = value
                    .as_ref()
                    .and_then(|value| self.lower_operand(value, instructions).flatten());
                match (condition, targets.as_slice()) {
                    (Some(condition), [then_target]) => Terminator::Branch {
                        condition,
                        then_target: lower_block(*then_target),
                        then_arguments: Vec::new(),
                        else_target: lower_block(*otherwise),
                        else_arguments: Vec::new(),
                    },
                    _ => {
                        self.errors.push(self.error(
                            Some(*span),
                            "JIR scalar lowering requires one-target Bool MIR switch",
                        ));
                        Terminator::Unreachable
                    }
                }
            }
            MirTerminator::Return { value, .. } => Terminator::Return {
                value: value
                    .as_ref()
                    .and_then(|value| self.lower_operand(value, instructions).flatten()),
            },
            MirTerminator::Unreachable { .. } => Terminator::Unreachable,
            MirTerminator::Match {
                value,
                pattern,
                matched,
                otherwise,
                span,
                ..
            } => {
                let discriminant = self.lower_required_operand(value, instructions);
                discriminant.map_or(Terminator::Unreachable, |discriminant| {
                    self.lower_pattern_branch(
                        discriminant,
                        value.ty,
                        pattern,
                        PatternTargets {
                            matched: lower_block(*matched),
                            otherwise: lower_block(*otherwise),
                        },
                        instructions,
                        *span,
                    )
                })
            }
            MirTerminator::Propagate {
                value,
                success,
                residual,
                span,
                ..
            } => {
                let carrier = self.lower_required_operand(value, instructions);
                let condition = carrier
                    .map(|carrier| self.emit_variant_condition(carrier, 1, instructions, *span));
                condition.map_or(Terminator::Unreachable, |condition| Terminator::Branch {
                    condition,
                    then_target: lower_block(*success),
                    then_arguments: Vec::new(),
                    else_target: lower_block(*residual),
                    else_arguments: Vec::new(),
                })
            }
        }
    }

    fn lower_pattern_condition(
        &mut self,
        value: ValueId,
        ty: SemanticTypeId,
        pattern: &MirPattern,
        instructions: &mut Vec<Instruction>,
        span: Span,
    ) -> Option<ValueId> {
        match pattern {
            MirPattern::Wildcard | MirPattern::Binding => {
                let bool_ty = self.type_table.intern(Type::Bool);
                let result = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: result,
                        ty: bool_ty,
                    }),
                    kind: InstructionKind::Constant(Constant::Bool(true)),
                    span: Some(span),
                });
                Some(result)
            }
            MirPattern::Literal(literal) => {
                let constant =
                    match lower_constant(&literal.text, self.type_table.semantic.kind(ty)) {
                        Ok(constant) => constant,
                        Err(message) => {
                            self.errors.push(self.error(Some(span), message));
                            return None;
                        }
                    };
                let literal_ty = match self.type_table.lower(ty, Some(span)) {
                    Ok(ty) => ty,
                    Err(error) => {
                        self.errors.push(error);
                        return None;
                    }
                };
                let expected = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: expected,
                        ty: literal_ty,
                    }),
                    kind: InstructionKind::Constant(constant),
                    span: Some(span),
                });
                let bool_ty = self.type_table.intern(Type::Bool);
                let condition = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: condition,
                        ty: bool_ty,
                    }),
                    kind: InstructionKind::Compare {
                        predicate: ComparePredicate::Equal,
                        left: value,
                        right: expected,
                    },
                    span: Some(span),
                });
                Some(condition)
            }
            MirPattern::Path { path, .. } => {
                let (variant, _) =
                    match self
                        .type_table
                        .variant_index_and_fields(ty, path, Some(span))
                    {
                        Ok(variant) => variant,
                        Err(error) => {
                            self.errors.push(error);
                            return None;
                        }
                    };
                Some(self.emit_variant_condition(value, variant, instructions, span))
            }
            MirPattern::Constructor {
                path, arguments, ..
            } => {
                if arguments
                    .iter()
                    .any(|argument| !matches!(argument, MirPattern::Wildcard | MirPattern::Binding))
                {
                    self.errors.push(self.error(
                        Some(span),
                        "nested payload pattern lowering requires synthetic JIR CFG blocks",
                    ));
                    return None;
                }
                let (variant, fields) =
                    match self
                        .type_table
                        .variant_index_and_fields(ty, path, Some(span))
                    {
                        Ok(variant) => variant,
                        Err(error) => {
                            self.errors.push(error);
                            return None;
                        }
                    };
                if arguments.len() != fields.len() {
                    self.errors.push(self.error(
                        Some(span),
                        "MIR constructor pattern payload count differs from enum layout",
                    ));
                    return None;
                }
                Some(self.emit_variant_condition(value, variant, instructions, span))
            }
        }
    }

    fn lower_pattern_branch(
        &mut self,
        value: ValueId,
        ty: SemanticTypeId,
        pattern: &MirPattern,
        targets: PatternTargets,
        instructions: &mut Vec<Instruction>,
        span: Span,
    ) -> Terminator {
        let MirPattern::Constructor {
            path, arguments, ..
        } = pattern
        else {
            return self
                .lower_pattern_condition(value, ty, pattern, instructions, span)
                .map_or(Terminator::Unreachable, |condition| Terminator::Branch {
                    condition,
                    then_target: targets.matched,
                    then_arguments: Vec::new(),
                    else_target: targets.otherwise,
                    else_arguments: Vec::new(),
                });
        };
        let (variant, fields) = match self
            .type_table
            .variant_index_and_fields(ty, path, Some(span))
        {
            Ok(variant) => variant,
            Err(error) => {
                self.errors.push(error);
                return Terminator::Unreachable;
            }
        };
        if arguments.len() != fields.len() {
            self.errors.push(self.error(
                Some(span),
                "MIR constructor pattern payload count differs from enum layout",
            ));
            return Terminator::Unreachable;
        }
        let significant: Vec<_> = arguments
            .iter()
            .enumerate()
            .filter(|(_, pattern)| !matches!(pattern, MirPattern::Wildcard | MirPattern::Binding))
            .collect();
        let condition = self.emit_variant_condition(value, variant, instructions, span);
        if significant.is_empty() {
            return Terminator::Branch {
                condition,
                then_target: targets.matched,
                then_arguments: Vec::new(),
                else_target: targets.otherwise,
                else_arguments: Vec::new(),
            };
        }

        let blocks: Vec<_> = significant
            .iter()
            .map(|_| self.reserve_synthetic_block(span))
            .collect();
        for (position, ((field_index, pattern), block)) in
            significant.into_iter().zip(blocks.iter()).enumerate()
        {
            let field_ty = fields[field_index];
            let jir_ty = match self.type_table.lower(field_ty, Some(span)) {
                Ok(ty) => ty,
                Err(error) => {
                    self.errors.push(error);
                    continue;
                }
            };
            let extracted = self.new_value();
            let mut nested_instructions = vec![Instruction {
                result: Some(TypedValue {
                    value: extracted,
                    ty: jir_ty,
                }),
                kind: InstructionKind::EnumExtract {
                    value,
                    variant,
                    field: field_index as u32,
                },
                span: Some(span),
            }];
            let next = blocks.get(position + 1).copied().unwrap_or(targets.matched);
            let terminator = self.lower_pattern_branch(
                extracted,
                field_ty,
                pattern,
                PatternTargets {
                    matched: next,
                    otherwise: targets.otherwise,
                },
                &mut nested_instructions,
                span,
            );
            self.set_synthetic_block(*block, nested_instructions, terminator, span);
        }
        Terminator::Branch {
            condition,
            then_target: blocks[0],
            then_arguments: Vec::new(),
            else_target: targets.otherwise,
            else_arguments: Vec::new(),
        }
    }

    fn reserve_synthetic_block(&mut self, span: Span) -> BlockId {
        let id = BlockId::new(self.source.blocks.len() + self.synthetic_blocks.len());
        self.synthetic_blocks.push(Block {
            id,
            parameters: Vec::new(),
            instructions: Vec::new(),
            terminator: Terminator::Unreachable,
            span: Some(span),
        });
        id
    }

    fn set_synthetic_block(
        &mut self,
        id: BlockId,
        instructions: Vec<Instruction>,
        terminator: Terminator,
        span: Span,
    ) {
        let index = id.index() - self.source.blocks.len();
        self.synthetic_blocks[index] = Block {
            id,
            parameters: Vec::new(),
            instructions,
            terminator,
            span: Some(span),
        };
    }

    fn emit_variant_condition(
        &mut self,
        value: ValueId,
        variant: u32,
        instructions: &mut Vec<Instruction>,
        span: Span,
    ) -> ValueId {
        let tag_ty = self.type_table.intern(Type::Integer {
            signed: false,
            bits: 32,
        });
        let tag = self.new_value();
        instructions.push(Instruction {
            result: Some(TypedValue {
                value: tag,
                ty: tag_ty,
            }),
            kind: InstructionKind::EnumTag { value },
            span: Some(span),
        });
        let expected = self.new_value();
        instructions.push(Instruction {
            result: Some(TypedValue {
                value: expected,
                ty: tag_ty,
            }),
            kind: InstructionKind::Constant(Constant::Integer {
                value: i128::from(variant),
            }),
            span: Some(span),
        });
        let bool_ty = self.type_table.intern(Type::Bool);
        let condition = self.new_value();
        instructions.push(Instruction {
            result: Some(TypedValue {
                value: condition,
                ty: bool_ty,
            }),
            kind: InstructionKind::Compare {
                predicate: ComparePredicate::Equal,
                left: tag,
                right: expected,
            },
            span: Some(span),
        });
        condition
    }

    fn lower_operand(
        &mut self,
        operand: &MirOperand,
        instructions: &mut Vec<Instruction>,
    ) -> Option<Option<ValueId>> {
        let lowered_ty = match (
            &operand.kind,
            self.type_table.semantic.kind(operand.ty).cloned(),
        ) {
            (MirOperandKind::Place(place), _) if place.projection.is_empty() => self
                .local_types
                .get(place.local.index())
                .copied()
                .ok_or_else(|| LowerError {
                    span: Some(operand.span),
                    message: "MIR place local has no lowered JIR type".to_owned(),
                }),
            (MirOperandKind::RegionAllocate { .. }, Some(TypeKind::Buffer(element))) => self
                .type_table
                .lower_buffer(element, AddressSpace::Region, true, Some(operand.span)),
            _ => self.type_table.lower(operand.ty, Some(operand.span)),
        };
        let ty = match lowered_ty {
            Ok(ty) => ty,
            Err(error) => {
                self.errors.push(error);
                return None;
            }
        };
        if self.type_table.is_unit(ty) && matches!(operand.kind, MirOperandKind::Unit) {
            return Some(None);
        }
        let kind = match &operand.kind {
            MirOperandKind::Unit => return Some(None),
            MirOperandKind::Place(place) => {
                let pointer =
                    self.lower_place_address(place, &[], instructions, Some(operand.span))?;
                InstructionKind::Load {
                    pointer,
                    alignment: 1,
                    volatile: false,
                }
            }
            MirOperandKind::Literal(literal) => {
                if matches!(
                    self.type_table.semantic.kind(operand.ty),
                    Some(TypeKind::String)
                ) {
                    match decode_quoted(&literal.text, '"') {
                        Ok(utf8) => InstructionKind::StringLiteral { utf8 },
                        Err(message) => {
                            self.errors.push(self.error(Some(operand.span), message));
                            return None;
                        }
                    }
                } else {
                    match lower_constant(&literal.text, self.type_table.semantic.kind(operand.ty)) {
                        Ok(constant) => InstructionKind::Constant(constant),
                        Err(message) => {
                            self.errors.push(self.error(Some(operand.span), message));
                            return None;
                        }
                    }
                }
            }
            MirOperandKind::Unary {
                operator,
                operand: inner,
            } => {
                let operand = self.lower_required_operand(inner, instructions)?;
                if *operator == Operator::Plus {
                    return Some(Some(operand));
                }
                let Some(op) = lower_unary(*operator) else {
                    self.errors.push(self.error(
                        Some(inner.span),
                        format!("unsupported MIR unary operator {operator:?}"),
                    ));
                    return None;
                };
                InstructionKind::Unary { op, operand }
            }
            MirOperandKind::Cast { operand: inner } => {
                let source_value = self.lower_required_operand(inner, instructions)?;
                if inner.ty == operand.ty {
                    return Some(Some(source_value));
                }
                let source_kind = self.type_table.semantic.kind(inner.ty);
                let target_kind = self.type_table.semantic.kind(operand.ty);
                let Some(op) =
                    numeric_cast_op(source_kind, target_kind, self.type_table.pointer_bits)
                else {
                    self.errors.push(self.error(
                        Some(operand.span),
                        "MIR cast does not have a supported numeric JIR conversion",
                    ));
                    return None;
                };
                InstructionKind::Cast {
                    op,
                    value: source_value,
                    target: ty,
                }
            }
            MirOperandKind::Binary {
                left,
                operator,
                right,
            } => {
                let left = self.lower_required_operand(left, instructions)?;
                let right = self.lower_required_operand(right, instructions)?;
                if let Some(predicate) = lower_compare(*operator) {
                    if matches!(
                        self.type_table.semantic.kind(operand.ty),
                        Some(TypeKind::Vector { .. })
                    ) {
                        self.errors.push(self.error(
                            Some(operand.span),
                            "vector comparisons are not supported by the source-level SIMD contract",
                        ));
                        return None;
                    }
                    InstructionKind::Compare {
                        predicate,
                        left,
                        right,
                    }
                } else if let Some(op) = lower_binary(*operator) {
                    if matches!(
                        self.type_table.semantic.kind(operand.ty),
                        Some(TypeKind::Vector { .. })
                    ) {
                        InstructionKind::VectorBinary { op, left, right }
                    } else {
                        InstructionKind::Binary { op, left, right }
                    }
                } else {
                    self.errors.push(self.error(
                        Some(operand.span),
                        format!("unsupported MIR binary operator {operator:?}"),
                    ));
                    return None;
                }
            }
            MirOperandKind::Call { callee, arguments } => {
                let Some((name, symbol)) = (match &callee.kind {
                    MirOperandKind::Function { name, symbol } => Some((name, symbol)),
                    _ => None,
                }) else {
                    let callee_value = self.lower_required_operand(callee, instructions)?;
                    let expected_parameters = match self.type_table.semantic.kind(callee.ty) {
                        Some(TypeKind::Function { parameters, .. }) => parameters.to_vec(),
                        _ => {
                            self.errors.push(self.error(
                                Some(callee.span),
                                "indirect call callee is not a function value",
                            ));
                            return None;
                        }
                    };
                    let mut lowered_arguments = Vec::with_capacity(arguments.len());
                    for (index, argument) in arguments.iter().enumerate() {
                        lowered_arguments.push(self.lower_user_call_argument(
                            argument,
                            expected_parameters.get(index).copied(),
                            instructions,
                        )?);
                    }
                    let instruction = InstructionKind::IndirectCall {
                        callee: callee_value,
                        arguments: lowered_arguments,
                    };
                    if self.type_table.is_unit(ty) {
                        instructions.push(Instruction {
                            result: None,
                            kind: instruction,
                            span: Some(operand.span),
                        });
                        return Some(None);
                    }
                    let value = self.new_value();
                    instructions.push(Instruction {
                        result: Some(TypedValue { value, ty }),
                        kind: instruction,
                        span: Some(operand.span),
                    });
                    return Some(Some(value));
                };
                if is_lowered_vector_intrinsic(name) {
                    let instruction =
                        self.lower_vector_intrinsic(name, arguments, operand.span, instructions)?;
                    if self.type_table.is_unit(ty) {
                        instructions.push(Instruction {
                            result: None,
                            kind: instruction,
                            span: Some(operand.span),
                        });
                        return Some(None);
                    }
                    instruction
                } else if let Ok((variant, fields)) =
                    self.type_table
                        .variant_index_and_fields(operand.ty, name, Some(operand.span))
                {
                    if fields.len() != arguments.len() {
                        self.errors.push(self.error(
                            Some(operand.span),
                            "enum constructor argument count differs from layout",
                        ));
                        return None;
                    }
                    let mut lowered = Vec::with_capacity(arguments.len());
                    for argument in arguments {
                        lowered.push(self.lower_required_operand(argument, instructions)?);
                    }
                    InstructionKind::EnumConstruct {
                        variant,
                        fields: lowered,
                    }
                } else {
                    let runtime_name = if let Some(element) = generic_buffer_element(
                        name,
                        operand.ty,
                        arguments,
                        &self.type_table.semantic,
                    ) && self.type_table.buffer_element_requires_move(element)
                        && generic_buffer_value_parameter_index(name)
                            .and_then(|index| {
                                arguments.get(index).filter(|argument| {
                                    self.operand_place_with_indices(argument).is_some()
                                })
                            })
                            .is_some()
                    {
                        match name.as_str() {
                            "buffer_append" => "buffer_append_move",
                            "buffer_append_status" => "buffer_append_move_status",
                            "buffer_insert" => "buffer_insert_move_from",
                            "buffer_insert_status" => "buffer_insert_move_from_status",
                            _ => name.as_str(),
                        }
                    } else {
                        name.as_str()
                    };
                    // Keep the source-level name only for diagnostics; all
                    // ABI adaptation below follows the selected runtime name.
                    let name = runtime_name;
                    let function = symbol
                        .and_then(|symbol| self.call_targets.local.get(&symbol).copied())
                        .or_else(|| {
                            {
                                let source_signature = source_function_signature(
                                    *symbol,
                                    callee.ty,
                                    &self.type_table.semantic,
                                );
                                let (parameters, result) = if let Some((parameters, result)) =
                                    source_signature
                                {
                                    (parameters, result)
                                } else {
                                    let mut parameters = arguments
                                        .iter()
                                        .enumerate()
                                        .map(|(index, argument)| {
                                            canonical_external_parameter(
                                                name,
                                                index,
                                                argument.ty,
                                                &mut self.type_table.semantic,
                                            )
                                        })
                                        .collect::<Vec<_>>();
                                    if let Some(index) = generic_buffer_value_parameter_index(name)
                                        && parameters.get(index).is_some()
                                        && generic_buffer_element(
                                            name,
                                            operand.ty,
                                            arguments,
                                            &self.type_table.semantic,
                                        )
                                        .is_some()
                                    {
                                        parameters[index] = generic_buffer_opaque_pointer(
                                            &mut self.type_table.semantic,
                                        );
                                    }
                                    parameters.extend(generic_buffer_runtime_parameters(
                                        name,
                                        operand.ty,
                                        arguments,
                                        &mut self.type_table.semantic,
                                    ));
                                    let result = canonical_external_result(
                                        name,
                                        operand.ty,
                                        &mut self.type_table.semantic,
                                    );
                                    (parameters, result)
                                };
                                self.call_targets.external.get(&ExternalTarget {
                                    symbol: canonical_external_symbol(name, *symbol),
                                    name: name.to_owned(),
                                    parameters,
                                    result,
                                })
                            }
                            .map(|external| external.id)
                        });
                    let Some(function) = function else {
                        self.errors.push(
                            self.error(Some(callee.span), "JIR call target was not registered"),
                        );
                        return None;
                    };
                    let is_source_function =
                        source_function_signature(*symbol, callee.ty, &self.type_table.semantic)
                            .is_some();
                    let mut lowered_arguments = Vec::with_capacity(arguments.len());
                    let expected_parameters =
                        match self.type_table.semantic.kind(callee.ty).cloned() {
                            Some(TypeKind::Function { parameters, .. }) => parameters.into_vec(),
                            _ => Vec::new(),
                        };
                    for (index, argument) in arguments.iter().enumerate() {
                        if matches!(
                            name,
                            "buffer_remove_move_into"
                                | "buffer_remove_move_into_status"
                                | "buffer_pop_move_into"
                                | "buffer_pop_move_into_status"
                        ) && ((name.starts_with("buffer_pop_move_into") && index == 1)
                            || (!name.starts_with("buffer_pop_move_into") && index == 2))
                        {
                            let expected = expected_parameters.get(index).copied().or_else(|| {
                                generic_buffer_element(
                                    name,
                                    operand.ty,
                                    arguments,
                                    &self.type_table.semantic,
                                )
                                .map(|element| {
                                    self.type_table.semantic.intern(TypeKind::Capability {
                                        capability: Capability::Write,
                                        inner: element,
                                    })
                                })
                            });
                            lowered_arguments.push(self.lower_move_output_argument(
                                argument,
                                expected,
                                instructions,
                            )?);
                            continue;
                        }
                        if descriptor_runtime_argument(name, index) {
                            let inner = match self.type_table.semantic.kind(argument.ty).cloned() {
                                Some(TypeKind::Capability { inner, .. }) => inner,
                                _ => argument.ty,
                            };
                            let expected = self.type_table.semantic.intern(TypeKind::Capability {
                                capability: borrowed_runtime_argument_capability(name, index)
                                    .unwrap_or(Capability::Write),
                                inner,
                            });
                            lowered_arguments.push(self.lower_descriptor_call_argument(
                                argument,
                                Some(expected),
                                instructions,
                            )?);
                            continue;
                        }
                        if is_borrowed_runtime_argument(name, index) {
                            let inner = match self.type_table.semantic.kind(argument.ty).cloned() {
                                Some(TypeKind::Capability { inner, .. }) => inner,
                                Some(TypeKind::Array { element, .. }) => {
                                    self.type_table.semantic.intern(TypeKind::Slice(element))
                                }
                                _ => argument.ty,
                            };
                            let expected = self.type_table.semantic.intern(TypeKind::Capability {
                                capability: borrowed_runtime_argument_capability(name, index)
                                    .unwrap_or(Capability::Write),
                                inner,
                            });
                            lowered_arguments.push(self.lower_call_argument(
                                argument,
                                Some(expected),
                                instructions,
                            )?);
                            continue;
                        }
                        if generic_buffer_value_parameter_index(name) == Some(index) {
                            lowered_arguments.push(
                                self.lower_generic_buffer_value_argument(argument, instructions)?,
                            );
                            continue;
                        }
                        let expected = expected_parameters.get(index).copied();
                        lowered_arguments.push(if is_source_function {
                            self.lower_user_call_argument(argument, expected, instructions)?
                        } else {
                            self.lower_call_argument(argument, expected, instructions)?
                        });
                    }
                    let record_resize_move = if matches!(
                        name,
                        "buffer_clear_move"
                            | "buffer_clear_move_status"
                            | "buffer_resize_move"
                            | "buffer_resize_move_status"
                    ) {
                        generic_buffer_element(
                            name,
                            operand.ty,
                            arguments,
                            &self.type_table.semantic,
                        )
                        .and_then(|outer_element| {
                            let (depth, record_ty, fields) =
                                if let Some((depth, record_ty, fields)) = self
                                    .type_table
                                    .nested_buffer_record_drop_fields_info(outer_element)
                                {
                                    (depth, record_ty, fields)
                                } else {
                                    let fields = self
                                        .type_table
                                        .buffer_record_drop_fields_info(outer_element)?;
                                    (0, outer_element, fields)
                                };
                            let element = self
                                .type_table
                                .lower(outer_element, Some(operand.span))
                                .ok()?;
                            let record_element =
                                self.type_table.lower(record_ty, Some(operand.span)).ok()?;
                            let fields = fields
                                .into_iter()
                                .map(|field| {
                                    Some(RecordDropField {
                                        payload_variant: field.payload_variant,
                                        payload_offset: field.offset,
                                        depth: field.depth,
                                        leaf_element: self
                                            .type_table
                                            .lower(field.leaf, Some(operand.span))
                                            .ok()?,
                                    })
                                })
                                .collect::<Option<Vec<_>>>()?;
                            Some((element, record_element, fields, depth))
                        })
                    } else {
                        None
                    };
                    let owned_string_resize_move = if matches!(
                        name,
                        "buffer_clear_move"
                            | "buffer_clear_move_status"
                            | "buffer_resize_move"
                            | "buffer_resize_move_status"
                    ) {
                        generic_buffer_owned_string_element(
                            name,
                            operand.ty,
                            arguments,
                            &self.type_table.semantic,
                        )
                        .and_then(|element_ty| {
                            self.type_table.lower(element_ty, Some(operand.span)).ok()
                        })
                    } else {
                        None
                    };
                    let nested_owned_string_resize_move = if matches!(
                        name,
                        "buffer_clear_move"
                            | "buffer_clear_move_status"
                            | "buffer_resize_move"
                            | "buffer_resize_move_status"
                    ) {
                        generic_buffer_nested_owned_string(
                            name,
                            operand.ty,
                            arguments,
                            &self.type_table.semantic,
                        )
                        .and_then(|(depth, leaf_ty)| {
                            let element_ty = generic_buffer_element(
                                name,
                                operand.ty,
                                arguments,
                                &self.type_table.semantic,
                            )?;
                            let element =
                                self.type_table.lower(element_ty, Some(operand.span)).ok()?;
                            let string_element =
                                self.type_table.lower(leaf_ty, Some(operand.span)).ok()?;
                            Some((element, string_element, depth))
                        })
                    } else {
                        None
                    };
                    let record_remove_drop =
                        if matches!(name, "buffer_remove_drop" | "buffer_remove_drop_status") {
                            generic_buffer_element(
                                name,
                                operand.ty,
                                arguments,
                                &self.type_table.semantic,
                            )
                            .and_then(|element_ty| {
                                let fields =
                                    self.type_table.buffer_record_drop_fields_info(element_ty)?;
                                let element =
                                    self.type_table.lower(element_ty, Some(operand.span)).ok()?;
                                let fields = fields
                                    .into_iter()
                                    .map(|field| {
                                        Some(RecordDropField {
                                            payload_variant: field.payload_variant,
                                            payload_offset: field.offset,
                                            depth: field.depth,
                                            leaf_element: self
                                                .type_table
                                                .lower(field.leaf, Some(operand.span))
                                                .ok()?,
                                        })
                                    })
                                    .collect::<Option<Vec<_>>>()?;
                                Some((element, fields))
                            })
                        } else {
                            None
                        };
                    let owned_string_remove_drop =
                        if matches!(name, "buffer_remove_drop" | "buffer_remove_drop_status") {
                            generic_buffer_owned_string_element(
                                name,
                                operand.ty,
                                arguments,
                                &self.type_table.semantic,
                            )
                            .and_then(|element_ty| {
                                self.type_table.lower(element_ty, Some(operand.span)).ok()
                            })
                        } else {
                            None
                        };
                    let nested_record_remove_drop =
                        if matches!(name, "buffer_remove_drop" | "buffer_remove_drop_status") {
                            generic_buffer_element(
                                name,
                                operand.ty,
                                arguments,
                                &self.type_table.semantic,
                            )
                            .and_then(|outer_element| {
                                let (depth, record_ty, fields) = self
                                    .type_table
                                    .nested_buffer_record_drop_fields_info(outer_element)?;
                                let element = self
                                    .type_table
                                    .lower(outer_element, Some(operand.span))
                                    .ok()?;
                                let record_element =
                                    self.type_table.lower(record_ty, Some(operand.span)).ok()?;
                                let fields = fields
                                    .into_iter()
                                    .map(|field| {
                                        Some(RecordDropField {
                                            payload_variant: field.payload_variant,
                                            payload_offset: field.offset,
                                            depth: field.depth,
                                            leaf_element: self
                                                .type_table
                                                .lower(field.leaf, Some(operand.span))
                                                .ok()?,
                                        })
                                    })
                                    .collect::<Option<Vec<_>>>()?;
                                Some((element, record_element, fields, depth))
                            })
                        } else {
                            None
                        };
                    if matches!(
                        name,
                        "buffer_create"
                            | "buffer_clear_move"
                            | "buffer_clear_move_status"
                            | "buffer_reserve"
                            | "buffer_reserve_status"
                            | "buffer_resize_move"
                            | "buffer_resize_move_status"
                            | "buffer_append"
                            | "buffer_append_status"
                            | "buffer_insert"
                            | "buffer_insert_status"
                            | "buffer_remove"
                            | "buffer_remove_status"
                            | "buffer_remove_drop"
                            | "buffer_remove_drop_status"
                            | "buffer_pop"
                            | "buffer_pop_move_into"
                            | "buffer_pop_move_into_status"
                            | "buffer_remove_move"
                            | "buffer_remove_move_into"
                            | "buffer_remove_move_into_status"
                            | "buffer_insert_move"
                            | "buffer_insert_move_status"
                            | "buffer_insert_move_from"
                            | "buffer_insert_move_from_status"
                            | "buffer_append_move"
                            | "buffer_append_move_status"
                    ) {
                        let (element_size, element_alignment) =
                            generic_buffer_layout(name, operand.ty, arguments, self.type_table)
                                .ok_or_else(|| {
                                    self.errors.push(self.error(
                                        Some(operand.span),
                                        "generic Buffer element has no supported native layout",
                                    ));
                                })
                                .ok()?;
                        lowered_arguments.push(self.lower_layout_constant(
                            element_size,
                            operand.span,
                            instructions,
                        )?);
                        lowered_arguments.push(self.lower_layout_constant(
                            element_alignment,
                            operand.span,
                            instructions,
                        )?);
                        if matches!(
                            name,
                            "buffer_resize_move"
                                | "buffer_resize_move_status"
                                | "buffer_clear_move"
                                | "buffer_clear_move_status"
                        ) {
                            if owned_string_resize_move.is_some() {
                                // `OwnedString` uses the same 24-byte
                                // descriptor layout as a nested Buffer, but
                                // has a dedicated string-aware runtime path
                                // and therefore carries no nested depth/leaf.
                            } else if let Some((depth, leaf)) = generic_buffer_nested_depth_and_leaf(
                                name,
                                operand.ty,
                                arguments,
                                &self.type_table.semantic,
                            ) {
                                let (leaf_size, leaf_alignment) = self
                                    .type_table
                                    .semantic_layout(leaf)
                                    .ok_or_else(|| {
                                        self.errors.push(self.error(
                                            Some(operand.span),
                                            "nested Buffer leaf has no supported native layout",
                                        ));
                                    })
                                    .ok()?;
                                lowered_arguments.push(self.lower_layout_constant(
                                    depth,
                                    operand.span,
                                    instructions,
                                )?);
                                lowered_arguments.push(self.lower_layout_constant(
                                    leaf_size,
                                    operand.span,
                                    instructions,
                                )?);
                                lowered_arguments.push(self.lower_layout_constant(
                                    leaf_alignment,
                                    operand.span,
                                    instructions,
                                )?);
                            } else if record_resize_move.is_none() {
                                self.errors.push(self.error(
                                    Some(operand.span),
                                    "nested Buffer element has no supported drop layout",
                                ));
                                return None;
                            }
                        } else if matches!(
                            name,
                            "buffer_pop"
                                | "buffer_remove_move"
                                | "buffer_insert_move"
                                | "buffer_insert_move_status"
                        ) {
                            let nested = generic_buffer_nested_element(
                                name,
                                operand.ty,
                                arguments,
                                &self.type_table.semantic,
                            )
                            .ok_or_else(|| {
                                self.errors.push(self.error(
                                    Some(operand.span),
                                    "nested Buffer element has no supported native layout",
                                ));
                            })
                            .ok()?;
                            let (nested_size, nested_alignment) = self
                                .type_table
                                .semantic_layout(nested)
                                .ok_or_else(|| {
                                    self.errors.push(self.error(
                                        Some(operand.span),
                                        "nested Buffer element has no supported native layout",
                                    ));
                                })
                                .ok()?;
                            lowered_arguments.push(self.lower_layout_constant(
                                nested_size,
                                operand.span,
                                instructions,
                            )?);
                            lowered_arguments.push(self.lower_layout_constant(
                                nested_alignment,
                                operand.span,
                                instructions,
                            )?);
                        }
                    }
                    if let Some((element, string_element, depth)) = nested_owned_string_resize_move
                    {
                        let Some(descriptor) = lowered_arguments.first().copied() else {
                            self.errors.push(self.error(
                                Some(operand.span),
                                "nested owned string resize move is missing its descriptor argument",
                            ));
                            return None;
                        };
                        let Some(new_length) = lowered_arguments.get(1).copied() else {
                            self.errors.push(self.error(
                                Some(operand.span),
                                "nested owned string resize move is missing its length argument",
                            ));
                            return None;
                        };
                        let instruction = InstructionKind::BufferResizeMoveNestedOwnedString {
                            descriptor,
                            new_length,
                            element,
                            string_element,
                            depth: u32::try_from(depth).unwrap_or(u32::MAX),
                            status_result: name.ends_with("_status"),
                        };
                        if self.type_table.is_unit(ty) {
                            instructions.push(Instruction {
                                result: None,
                                kind: instruction,
                                span: Some(operand.span),
                            });
                            return Some(None);
                        }
                        let value = self.new_value();
                        instructions.push(Instruction {
                            result: Some(TypedValue { value, ty }),
                            kind: instruction,
                            span: Some(operand.span),
                        });
                        return Some(Some(value));
                    }
                    if let Some(element) = owned_string_resize_move {
                        let Some(descriptor) = lowered_arguments.first().copied() else {
                            self.errors.push(self.error(
                                Some(operand.span),
                                "owned string resize move is missing its descriptor argument",
                            ));
                            return None;
                        };
                        let new_length =
                            if matches!(name, "buffer_clear_move" | "buffer_clear_move_status") {
                                self.lower_layout_constant(0, operand.span, instructions)?
                            } else {
                                let Some(new_length) = lowered_arguments.get(1).copied() else {
                                    self.errors.push(self.error(
                                        Some(operand.span),
                                        "owned string resize move is missing its length argument",
                                    ));
                                    return None;
                                };
                                new_length
                            };
                        let instruction = InstructionKind::BufferResizeMoveOwnedString {
                            descriptor,
                            new_length,
                            element,
                            status_result: name.ends_with("_status"),
                        };
                        if self.type_table.is_unit(ty) {
                            instructions.push(Instruction {
                                result: None,
                                kind: instruction,
                                span: Some(operand.span),
                            });
                            return Some(None);
                        }
                        let value = self.new_value();
                        instructions.push(Instruction {
                            result: Some(TypedValue { value, ty }),
                            kind: instruction,
                            span: Some(operand.span),
                        });
                        return Some(Some(value));
                    }
                    if let Some(element) = owned_string_remove_drop {
                        let Some(descriptor) = lowered_arguments.first().copied() else {
                            self.errors.push(self.error(
                                Some(operand.span),
                                "owned string remove drop is missing its descriptor argument",
                            ));
                            return None;
                        };
                        let Some(index) = lowered_arguments.get(1).copied() else {
                            self.errors.push(self.error(
                                Some(operand.span),
                                "owned string remove drop is missing its index argument",
                            ));
                            return None;
                        };
                        let instruction = InstructionKind::BufferRemoveDropOwnedString {
                            descriptor,
                            index,
                            element,
                            status_result: name.ends_with("_status"),
                        };
                        if self.type_table.is_unit(ty) {
                            instructions.push(Instruction {
                                result: None,
                                kind: instruction,
                                span: Some(operand.span),
                            });
                            return Some(None);
                        }
                        let value = self.new_value();
                        instructions.push(Instruction {
                            result: Some(TypedValue { value, ty }),
                            kind: instruction,
                            span: Some(operand.span),
                        });
                        return Some(Some(value));
                    }
                    if let Some((element, record_element, fields, depth)) = record_resize_move {
                        let Some(descriptor) = lowered_arguments.first().copied() else {
                            self.errors.push(self.error(
                                Some(operand.span),
                                "record clear/resize move is missing its descriptor argument",
                            ));
                            return None;
                        };
                        let new_length =
                            if matches!(name, "buffer_clear_move" | "buffer_clear_move_status") {
                                // Clear has no user-provided length. Reuse the
                                // existing record-aware resize instruction with a
                                // typed zero so the runtime field-table ABI stays
                                // identical for both operations.
                                self.lower_layout_constant(0, operand.span, instructions)?
                            } else {
                                let Some(new_length) = lowered_arguments.get(1).copied() else {
                                    self.errors.push(self.error(
                                        Some(operand.span),
                                        "record resize move is missing its length argument",
                                    ));
                                    return None;
                                };
                                new_length
                            };
                        let instruction = InstructionKind::BufferResizeMoveRecordFields {
                            descriptor,
                            new_length,
                            element,
                            record_element,
                            fields,
                            depth,
                            status_result: name.ends_with("_status"),
                        };
                        if self.type_table.is_unit(ty) {
                            instructions.push(Instruction {
                                result: None,
                                kind: instruction,
                                span: Some(operand.span),
                            });
                            return Some(None);
                        }
                        let value = self.new_value();
                        instructions.push(Instruction {
                            result: Some(TypedValue { value, ty }),
                            kind: instruction,
                            span: Some(operand.span),
                        });
                        return Some(Some(value));
                    }
                    if let Some((element, record_element, fields, depth)) =
                        nested_record_remove_drop
                    {
                        let Some(descriptor) = lowered_arguments.first().copied() else {
                            self.errors.push(self.error(
                                Some(operand.span),
                                "nested record remove drop is missing its descriptor argument",
                            ));
                            return None;
                        };
                        let Some(index) = lowered_arguments.get(1).copied() else {
                            self.errors.push(self.error(
                                Some(operand.span),
                                "nested record remove drop is missing its index argument",
                            ));
                            return None;
                        };
                        let instruction = InstructionKind::BufferRemoveDropNestedRecordFields {
                            descriptor,
                            index,
                            element,
                            record_element,
                            fields,
                            depth,
                            status_result: name.ends_with("_status"),
                        };
                        if self.type_table.is_unit(ty) {
                            instructions.push(Instruction {
                                result: None,
                                kind: instruction,
                                span: Some(operand.span),
                            });
                            return Some(None);
                        }
                        let value = self.new_value();
                        instructions.push(Instruction {
                            result: Some(TypedValue { value, ty }),
                            kind: instruction,
                            span: Some(operand.span),
                        });
                        return Some(Some(value));
                    }
                    if let Some((element, fields)) = record_remove_drop {
                        let Some(descriptor) = lowered_arguments.first().copied() else {
                            self.errors.push(self.error(
                                Some(operand.span),
                                "record remove drop is missing its descriptor argument",
                            ));
                            return None;
                        };
                        let Some(index) = lowered_arguments.get(1).copied() else {
                            self.errors.push(self.error(
                                Some(operand.span),
                                "record remove drop is missing its index argument",
                            ));
                            return None;
                        };
                        let instruction = InstructionKind::BufferRemoveDropRecordFields {
                            descriptor,
                            index,
                            element,
                            fields,
                            status_result: name.ends_with("_status"),
                        };
                        if self.type_table.is_unit(ty) {
                            instructions.push(Instruction {
                                result: None,
                                kind: instruction,
                                span: Some(operand.span),
                            });
                            return Some(None);
                        }
                        let value = self.new_value();
                        instructions.push(Instruction {
                            result: Some(TypedValue { value, ty }),
                            kind: instruction,
                            span: Some(operand.span),
                        });
                        return Some(Some(value));
                    }
                    let instruction = InstructionKind::Call {
                        function,
                        arguments: lowered_arguments,
                    };
                    if self.type_table.is_unit(ty) {
                        instructions.push(Instruction {
                            result: None,
                            kind: instruction,
                            span: Some(operand.span),
                        });
                        return Some(None);
                    }
                    instruction
                }
            }
            MirOperandKind::Array(elements) => {
                let mut lowered = Vec::with_capacity(elements.len());
                for element in elements {
                    lowered.push(self.lower_required_operand(element, instructions)?);
                }
                InstructionKind::Aggregate { elements: lowered }
            }
            MirOperandKind::Struct { fields, .. } => {
                let layout = match self
                    .type_table
                    .record_fields(operand.ty, Some(operand.span))
                {
                    Ok(layout) => layout,
                    Err(error) => {
                        self.errors.push(error);
                        return None;
                    }
                };
                let mut lowered = Vec::with_capacity(layout.len());
                for (name, _) in layout {
                    let Some((_, value)) = fields.iter().find(|(candidate, _)| candidate == &name)
                    else {
                        self.errors.push(self.error(
                            Some(operand.span),
                            format!("record value is missing layout field {name:?}"),
                        ));
                        return None;
                    };
                    lowered.push(self.lower_required_operand(value, instructions)?);
                }
                InstructionKind::Aggregate { elements: lowered }
            }
            MirOperandKind::Field { base, field } => {
                let aggregate = self.lower_required_operand(base, instructions)?;
                let (index, _) =
                    match self
                        .type_table
                        .field_index_and_type(base.ty, field, Some(operand.span))
                    {
                        Ok(field) => field,
                        Err(error) => {
                            self.errors.push(error);
                            return None;
                        }
                    };
                InstructionKind::ExtractValue { aggregate, index }
            }
            MirOperandKind::Index { base, index } => {
                let mut aggregate = self.lower_required_operand(base, instructions)?;
                let index_value = self.lower_required_operand(index, instructions)?;
                let mut base_ty = base.ty;
                let mut capability_view = false;
                if let Some(TypeKind::Capability { inner, .. }) =
                    self.type_table.semantic.kind(base_ty).cloned()
                {
                    base_ty = inner;
                    if let Some(inner_ty) = self.operand_capability_pointee(base) {
                        let loaded = self.new_value();
                        instructions.push(Instruction {
                            result: Some(TypedValue {
                                value: loaded,
                                ty: inner_ty,
                            }),
                            kind: InstructionKind::Load {
                                pointer: aggregate,
                                alignment: 1,
                                volatile: false,
                            },
                            span: Some(base.span),
                        });
                        aggregate = loaded;
                    } else {
                        capability_view = true;
                    }
                }
                match self.type_table.semantic.kind(base_ty).cloned() {
                    Some(TypeKind::Array { length, .. }) => {
                        let index_ty = match self.type_table.lower(index.ty, Some(index.span)) {
                            Ok(index_ty) => index_ty,
                            Err(error) => {
                                self.errors.push(error);
                                return None;
                            }
                        };
                        let length_value = self.new_value();
                        instructions.push(Instruction {
                            result: Some(TypedValue {
                                value: length_value,
                                ty: index_ty,
                            }),
                            kind: InstructionKind::Constant(Constant::Integer {
                                value: i128::from(length),
                            }),
                            span: Some(index.span),
                        });
                        instructions.push(Instruction {
                            result: None,
                            kind: InstructionKind::BoundsCheck {
                                index: index_value,
                                length: length_value,
                            },
                            span: Some(operand.span),
                        });
                        InstructionKind::ExtractElement {
                            aggregate,
                            index: index_value,
                        }
                    }
                    Some(TypeKind::Buffer(element) | TypeKind::Slice(element)) => {
                        let address_space = if capability_view {
                            AddressSpace::Generic
                        } else if self.operand_is_region_owned(base) {
                            AddressSpace::Region
                        } else if matches!(
                            self.type_table.semantic.kind(base_ty),
                            Some(TypeKind::Buffer(_))
                        ) {
                            AddressSpace::Heap
                        } else {
                            AddressSpace::Generic
                        };
                        let element_ty = match self.type_table.lower(element, Some(operand.span)) {
                            Ok(ty) => ty,
                            Err(error) => {
                                self.errors.push(error);
                                return None;
                            }
                        };
                        let data_ty = self.type_table.intern(Type::Pointer {
                            pointee: element_ty,
                            address_space,
                        });
                        let size_ty = self.type_table.intern(Type::Integer {
                            signed: false,
                            bits: self.type_table.pointer_bits,
                        });
                        let aggregate_ty = if capability_view {
                            self.type_table
                                .lower_borrow_capability(base_ty, false, Some(operand.span))
                                .ok()?
                        } else if let Some(pointee) = self.operand_capability_pointee(base) {
                            pointee
                        } else if self.operand_is_region_owned(base) {
                            let MirOperandKind::Place(place) = &base.kind else {
                                return None;
                            };
                            self.local_types.get(place.local.index()).copied()?
                        } else {
                            self.type_table.lower(base_ty, Some(operand.span)).ok()?
                        };
                        let raw_data_ty = match self.type_table.types.get(aggregate_ty.index()) {
                            Some(Type::Struct { fields }) => fields.first().copied()?,
                            _ => {
                                self.errors.push(self.error(
                                    Some(operand.span),
                                    "Buffer/Slice aggregate has no data pointer field",
                                ));
                                return None;
                            }
                        };
                        let raw_data = self.new_value();
                        instructions.push(Instruction {
                            result: Some(TypedValue {
                                value: raw_data,
                                ty: raw_data_ty,
                            }),
                            kind: InstructionKind::ExtractValue {
                                aggregate,
                                index: 0,
                            },
                            span: Some(base.span),
                        });
                        let data = if raw_data_ty == data_ty {
                            raw_data
                        } else {
                            let cast = self.new_value();
                            instructions.push(Instruction {
                                result: Some(TypedValue {
                                    value: cast,
                                    ty: data_ty,
                                }),
                                kind: InstructionKind::Cast {
                                    op: CastOp::PointerCast,
                                    value: raw_data,
                                    target: data_ty,
                                },
                                span: Some(base.span),
                            });
                            cast
                        };
                        let length = self.new_value();
                        instructions.push(Instruction {
                            result: Some(TypedValue {
                                value: length,
                                ty: size_ty,
                            }),
                            kind: InstructionKind::ExtractValue {
                                aggregate,
                                index: 1,
                            },
                            span: Some(base.span),
                        });
                        let index = self.lower_index_to_size(
                            index_value,
                            index.ty,
                            size_ty,
                            instructions,
                            index.span,
                        )?;
                        instructions.push(Instruction {
                            result: None,
                            kind: InstructionKind::BoundsCheck { index, length },
                            span: Some(operand.span),
                        });
                        let pointer = self.new_value();
                        instructions.push(Instruction {
                            result: Some(TypedValue {
                                value: pointer,
                                ty: data_ty,
                            }),
                            kind: InstructionKind::Offset {
                                base: data,
                                indices: vec![index],
                            },
                            span: Some(operand.span),
                        });
                        InstructionKind::Load {
                            pointer,
                            alignment: 1,
                            volatile: false,
                        }
                    }
                    _ => {
                        self.errors.push(self.error(
                            Some(operand.span),
                            "JIR indexing requires an array, Buffer, or Slice",
                        ));
                        return None;
                    }
                }
            }
            MirOperandKind::Length { base } => {
                let mut base_ty = base.ty;
                if let Some(TypeKind::Capability { inner, .. }) =
                    self.type_table.semantic.kind(base_ty).cloned()
                {
                    base_ty = inner;
                }
                match self.type_table.semantic.kind(base_ty).cloned() {
                    Some(TypeKind::Array { length, .. }) => {
                        InstructionKind::Constant(Constant::Integer {
                            value: i128::from(length),
                        })
                    }
                    Some(TypeKind::Buffer(_) | TypeKind::Slice(_)) => {
                        let mut aggregate = self.lower_required_operand(base, instructions)?;
                        if let Some(pointee) = self.operand_capability_pointee(base) {
                            // A user-level `write Buffer<T>` parameter is a
                            // pointer to the owning descriptor.  Length reads
                            // must dereference that descriptor before the
                            // aggregate field extraction; borrowed Buffer and
                            // Slice views remain unchanged.
                            let loaded = self.new_value();
                            instructions.push(Instruction {
                                result: Some(TypedValue {
                                    value: loaded,
                                    ty: pointee,
                                }),
                                kind: InstructionKind::Load {
                                    pointer: aggregate,
                                    alignment: 1,
                                    volatile: false,
                                },
                                span: Some(operand.span),
                            });
                            aggregate = loaded;
                        }
                        InstructionKind::ExtractValue {
                            aggregate,
                            index: 1,
                        }
                    }
                    _ => {
                        self.errors.push(self.error(
                            Some(operand.span),
                            "JIR length requires an array, Buffer, or Slice",
                        ));
                        return None;
                    }
                }
            }
            MirOperandKind::Function { name, .. } => {
                match self
                    .type_table
                    .variant_index_and_fields(operand.ty, name, Some(operand.span))
                {
                    Ok((variant, fields)) if fields.is_empty() => InstructionKind::EnumConstruct {
                        variant,
                        fields: Vec::new(),
                    },
                    Ok(_) => {
                        self.errors.push(self.error(
                            Some(operand.span),
                            "payload enum constructor must be called",
                        ));
                        return None;
                    }
                    Err(_) => {
                        let MirOperandKind::Function { symbol, .. } = &operand.kind else {
                            unreachable!("matched function operand");
                        };
                        let target = symbol
                            .as_ref()
                            .and_then(|symbol| self.call_targets.local.get(symbol).copied())
                            .or_else(|| {
                                let Some(TypeKind::Function { parameters, result }) =
                                    self.type_table.semantic.kind(operand.ty).cloned()
                                else {
                                    return None;
                                };
                                let target = ExternalTarget {
                                    symbol: canonical_external_symbol(name, *symbol),
                                    name: name.clone(),
                                    parameters: parameters
                                        .iter()
                                        .enumerate()
                                        .map(|(index, parameter)| {
                                            canonical_external_parameter(
                                                name,
                                                index,
                                                *parameter,
                                                &mut self.type_table.semantic,
                                            )
                                        })
                                        .collect(),
                                    result: canonical_external_result(
                                        name,
                                        result,
                                        &mut self.type_table.semantic,
                                    ),
                                };
                                self.call_targets
                                    .external
                                    .get(&target)
                                    .map(|external| external.id)
                            });
                        let Some(function) = target else {
                            self.errors.push(self.error(
                                Some(operand.span),
                                "function value target was not registered",
                            ));
                            return None;
                        };
                        InstructionKind::FunctionAddress { function }
                    }
                }
            }
            MirOperandKind::PatternExtract {
                source,
                source_indices,
                path,
                ..
            } => {
                return self
                    .lower_pattern_extract(source, source_indices, path, instructions, operand.span)
                    .map(Some);
            }
            MirOperandKind::CarrierExtract {
                source,
                source_indices,
                part,
            } => {
                let (carrier, carrier_ty) =
                    self.load_source_place(source, source_indices, instructions, operand.span)?;
                let (variant, field) = match (self.type_table.semantic.kind(carrier_ty), part) {
                    (Some(TypeKind::Option(_)), CarrierPart::Success)
                    | (Some(TypeKind::Result { .. }), CarrierPart::Success) => {
                        (CarrierTag::Success.raw(), 0)
                    }
                    (Some(TypeKind::Result { .. }), CarrierPart::Residual) => {
                        (CarrierTag::Residual.raw(), 0)
                    }
                    _ => {
                        self.errors.push(
                            self.error(Some(operand.span), "invalid carrier payload extraction"),
                        );
                        return None;
                    }
                };
                InstructionKind::EnumExtract {
                    value: carrier,
                    variant,
                    field,
                }
            }
            MirOperandKind::PropagateResidual { source, kind, .. } => {
                let (carrier, carrier_ty) =
                    self.load_source_place(source, &[], instructions, operand.span)?;
                match kind {
                    MirPropagationKind::OptionNone => InstructionKind::EnumConstruct {
                        variant: CarrierTag::Residual.raw(),
                        fields: Vec::new(),
                    },
                    MirPropagationKind::ResultError => {
                        let error_ty = match self.type_table.semantic.kind(carrier_ty) {
                            Some(TypeKind::Result { error, .. }) => *error,
                            _ => {
                                self.errors.push(self.error(
                                    Some(operand.span),
                                    "Result residual source has a non-Result type",
                                ));
                                return None;
                            }
                        };
                        let error_jir = match self.type_table.lower(error_ty, Some(operand.span)) {
                            Ok(ty) => ty,
                            Err(error) => {
                                self.errors.push(error);
                                return None;
                            }
                        };
                        let error = self.new_value();
                        instructions.push(Instruction {
                            result: Some(TypedValue {
                                value: error,
                                ty: error_jir,
                            }),
                            kind: InstructionKind::EnumExtract {
                                value: carrier,
                                variant: CarrierTag::Residual.raw(),
                                field: 0,
                            },
                            span: Some(operand.span),
                        });
                        InstructionKind::EnumConstruct {
                            variant: CarrierTag::Residual.raw(),
                            fields: vec![error],
                        }
                    }
                }
            }
            MirOperandKind::RegionAllocate { region, arguments } => {
                let Some(region) = region else {
                    self.errors.push(
                        self.error(Some(operand.span), "region allocation has no region local"),
                    );
                    return None;
                };
                let (region, _) = self.load_source_place(
                    &Place::local(*region),
                    &[],
                    instructions,
                    operand.span,
                )?;
                let Some(count_operand) = arguments.first() else {
                    self.errors.push(
                        self.error(Some(operand.span), "region allocation has no element count"),
                    );
                    return None;
                };
                let count = self.lower_required_operand(count_operand, instructions)?;
                let size_ty = self.type_table.intern(Type::Integer {
                    signed: false,
                    bits: self.type_table.pointer_bits,
                });
                let count = self.lower_index_to_size(
                    count,
                    count_operand.ty,
                    size_ty,
                    instructions,
                    count_operand.span,
                )?;
                let Some(TypeKind::Buffer(element)) =
                    self.type_table.semantic.kind(operand.ty).cloned()
                else {
                    self.errors.push(
                        self.error(Some(operand.span), "region allocation result is not Buffer"),
                    );
                    return None;
                };
                let element_ty = match self.type_table.lower(element, Some(operand.span)) {
                    Ok(ty) => ty,
                    Err(error) => {
                        self.errors.push(error);
                        return None;
                    }
                };
                let pointer_ty = self.type_table.intern(Type::Pointer {
                    pointee: element_ty,
                    address_space: AddressSpace::Region,
                });
                let data = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: data,
                        ty: pointer_ty,
                    }),
                    kind: InstructionKind::RegionAlloc {
                        region,
                        ty: element_ty,
                        count,
                    },
                    span: Some(operand.span),
                });
                let byte_ty = match self
                    .type_table
                    .lower(self.type_table.semantic.core().uint8, Some(operand.span))
                {
                    Ok(ty) => self.type_table.pointer(ty, AddressSpace::Region),
                    Err(error) => {
                        self.errors.push(error);
                        return None;
                    }
                };
                let buffer_data = if pointer_ty == byte_ty {
                    data
                } else {
                    let cast = self.new_value();
                    instructions.push(Instruction {
                        result: Some(TypedValue {
                            value: cast,
                            ty: byte_ty,
                        }),
                        kind: InstructionKind::Cast {
                            op: CastOp::PointerCast,
                            value: data,
                            target: byte_ty,
                        },
                        span: Some(operand.span),
                    });
                    cast
                };
                // Region allocation returns a zero-initialized Buffer whose
                // logical length is the requested element count. This makes
                // the first native Buffer API useful immediately while later
                // append/resize operations can still distinguish capacity.
                InstructionKind::Aggregate {
                    elements: vec![buffer_data, count, count],
                }
            }
            MirOperandKind::HighLevel(_) => {
                self.errors.push(self.error(
                    Some(operand.span),
                    format!(
                        "JIR lowering does not yet support MIR operand {:?}",
                        operand.kind
                    ),
                ));
                return None;
            }
        };
        let value = self.new_value();
        instructions.push(Instruction {
            result: Some(TypedValue { value, ty }),
            kind,
            span: Some(operand.span),
        });
        Some(Some(value))
    }

    fn lower_required_operand(
        &mut self,
        operand: &MirOperand,
        instructions: &mut Vec<Instruction>,
    ) -> Option<ValueId> {
        match self.lower_operand(operand, instructions)? {
            Some(value) => Some(value),
            None => {
                self.errors.push(self.error(
                    Some(operand.span),
                    "Unit operand cannot be used as an instruction value",
                ));
                None
            }
        }
    }

    fn lower_vector_intrinsic(
        &mut self,
        name: &str,
        arguments: &[MirOperand],
        span: Span,
        instructions: &mut Vec<Instruction>,
    ) -> Option<InstructionKind> {
        let core = self.type_table.semantic.core();
        let float32 = core.float32;
        let lanes = vector_intrinsic_lanes(name)?;
        let vector = match lanes {
            2 => core.float2,
            3 => core.float3,
            4 => core.float4,
            8 => core.float8,
            _ => return None,
        };
        let slice = self.type_table.semantic.intern(TypeKind::Slice(float32));
        let capability = match name {
            "vector_load2" | "vector_load3" | "vector_load4" | "vector_load8" => Capability::Read,
            "vector_store2" | "vector_store3" | "vector_store4" | "vector_store8" => {
                Capability::Write
            }
            "vector_splat2" | "vector_splat3" | "vector_splat4" | "vector_splat8" => {
                Capability::Owned
            }
            _ => return None,
        };
        if matches!(
            name,
            "vector_splat2" | "vector_splat3" | "vector_splat4" | "vector_splat8"
        ) {
            let value = arguments
                .first()
                .and_then(|argument| self.lower_required_operand(argument, instructions))?;
            return Some(InstructionKind::VectorSplat { value, lanes });
        }
        let expected_slice = self.type_table.semantic.intern(TypeKind::Capability {
            capability,
            inner: slice,
        });
        let slice_argument = arguments.first()?;
        let slice_value =
            self.lower_call_argument(slice_argument, Some(expected_slice), instructions)?;
        let index_argument = arguments.get(1)?;
        let index = self.lower_required_operand(index_argument, instructions)?;
        let slice_ty = self.type_table.lower(slice, Some(span)).ok()?;
        let fields = match self.type_table.types.get(slice_ty.index()) {
            Some(Type::Struct { fields }) if fields.len() >= 2 => fields.clone(),
            _ => {
                self.errors.push(self.error(
                    Some(span),
                    "vector intrinsic slice does not have a pointer/length layout",
                ));
                return None;
            }
        };
        let data = self.new_value();
        instructions.push(Instruction {
            result: Some(TypedValue {
                value: data,
                ty: fields[0],
            }),
            kind: InstructionKind::ExtractValue {
                aggregate: slice_value,
                index: 0,
            },
            span: Some(span),
        });
        let length = self.new_value();
        instructions.push(Instruction {
            result: Some(TypedValue {
                value: length,
                ty: fields[1],
            }),
            kind: InstructionKind::ExtractValue {
                aggregate: slice_value,
                index: 1,
            },
            span: Some(span),
        });
        instructions.push(Instruction {
            result: None,
            kind: InstructionKind::VectorBoundsCheck {
                index,
                length,
                lanes,
            },
            span: Some(span),
        });
        let offset = self.new_value();
        instructions.push(Instruction {
            result: Some(TypedValue {
                value: offset,
                ty: fields[0],
            }),
            kind: InstructionKind::Offset {
                base: data,
                indices: vec![index],
            },
            span: Some(span),
        });
        let vector_ty = self.type_table.lower(vector, Some(span)).ok()?;
        let vector_pointer_ty = self.type_table.pointer(vector_ty, AddressSpace::Generic);
        let vector_pointer = self.new_value();
        instructions.push(Instruction {
            result: Some(TypedValue {
                value: vector_pointer,
                ty: vector_pointer_ty,
            }),
            kind: InstructionKind::Cast {
                op: CastOp::PointerCast,
                value: offset,
                target: vector_pointer_ty,
            },
            span: Some(span),
        });
        if matches!(
            name,
            "vector_load2" | "vector_load3" | "vector_load4" | "vector_load8"
        ) {
            return Some(InstructionKind::Load {
                pointer: vector_pointer,
                alignment: 4,
                volatile: false,
            });
        }
        let value = arguments
            .get(2)
            .and_then(|argument| self.lower_required_operand(argument, instructions))?;
        Some(InstructionKind::Store {
            pointer: vector_pointer,
            value,
            alignment: 4,
            volatile: false,
        })
    }

    fn lower_call_argument(
        &mut self,
        argument: &MirOperand,
        expected: Option<SemanticTypeId>,
        instructions: &mut Vec<Instruction>,
    ) -> Option<ValueId> {
        let Some(TypeKind::Capability {
            capability: _,
            inner,
        }) = expected.and_then(|ty| self.type_table.semantic.kind(ty).cloned())
        else {
            return self.lower_required_operand(argument, instructions);
        };
        if matches!(
            self.type_table.semantic.kind(argument.ty),
            Some(TypeKind::Capability { .. })
        ) {
            return self.lower_required_operand(argument, instructions);
        }
        if matches!(self.type_table.semantic.kind(inner), Some(TypeKind::String))
            && matches!(argument.kind, MirOperandKind::Literal(_))
        {
            return self.lower_required_operand(argument, instructions);
        }
        let Some((place, dynamic_indices)) = self.operand_place_with_indices(argument) else {
            self.errors.push(self.error(
                Some(argument.span),
                "call-scoped capability coercion requires an addressable MIR place",
            ));
            return None;
        };
        self.lower_borrow_from_place(&place, &dynamic_indices, inner, instructions, argument.span)
    }

    fn lower_user_call_argument(
        &mut self,
        argument: &MirOperand,
        expected: Option<SemanticTypeId>,
        instructions: &mut Vec<Instruction>,
    ) -> Option<ValueId> {
        if matches!(
            expected.and_then(|ty| self.type_table.semantic.kind(ty).cloned()),
            Some(TypeKind::Capability {
                capability: Capability::Write,
                inner,
            }) if matches!(self.type_table.semantic.kind(inner), Some(TypeKind::Buffer(_)))
        ) {
            // Only source-level user functions use the mutable descriptor
            // ABI. Runtime APIs such as app/json output buffers intentionally
            // consume borrowed two-word views and must retain their existing
            // lowering.
            return self.lower_descriptor_call_argument(argument, expected, instructions);
        }
        self.lower_call_argument(argument, expected, instructions)
    }

    fn lower_move_output_argument(
        &mut self,
        argument: &MirOperand,
        expected: Option<SemanticTypeId>,
        instructions: &mut Vec<Instruction>,
    ) -> Option<ValueId> {
        let byte = self
            .type_table
            .lower(self.type_table.semantic.core().uint8, Some(argument.span))
            .ok()?;
        let target = self.type_table.pointer(byte, AddressSpace::Generic);
        if matches!(
            self.type_table.semantic.kind(argument.ty),
            Some(TypeKind::Capability { .. })
        ) {
            let source = self.lower_required_operand(argument, instructions)?;
            let value = self.new_value();
            instructions.push(Instruction {
                result: Some(TypedValue { value, ty: target }),
                kind: InstructionKind::Cast {
                    op: CastOp::PointerCast,
                    value: source,
                    target,
                },
                span: Some(argument.span),
            });
            return Some(value);
        }
        let Some(TypeKind::Capability {
            capability: Capability::Write,
            inner: _,
        }) = expected.and_then(|ty| self.type_table.semantic.kind(ty).cloned())
        else {
            self.errors.push(self.error(
                Some(argument.span),
                "move output argument is missing a write capability",
            ));
            return None;
        };
        let MirOperandKind::Place(place) = &argument.kind else {
            self.errors.push(self.error(
                Some(argument.span),
                "move output argument requires an addressable place",
            ));
            return None;
        };
        let source_pointer =
            self.lower_place_address(place, &[], instructions, Some(argument.span))?;
        let value = self.new_value();
        instructions.push(Instruction {
            result: Some(TypedValue { value, ty: target }),
            kind: InstructionKind::Cast {
                op: CastOp::PointerCast,
                value: source_pointer,
                target,
            },
            span: Some(argument.span),
        });
        Some(value)
    }

    fn lower_layout_constant(
        &mut self,
        value: u64,
        span: Span,
        instructions: &mut Vec<Instruction>,
    ) -> Option<ValueId> {
        let semantic = self.type_table.semantic.core().uint_size;
        let ty = self.type_table.lower(semantic, Some(span)).ok()?;
        let value_id = self.new_value();
        instructions.push(Instruction {
            result: Some(TypedValue {
                value: value_id,
                ty,
            }),
            kind: InstructionKind::Constant(Constant::Integer {
                value: i128::from(value),
            }),
            span: Some(span),
        });
        Some(value_id)
    }

    fn lower_generic_buffer_value_argument(
        &mut self,
        argument: &MirOperand,
        instructions: &mut Vec<Instruction>,
    ) -> Option<ValueId> {
        // Move-aware generic APIs must receive the real caller-owned slot so
        // the native runtime can zero it after success. Keeping the address
        // also lets MIR leave the slot drop-managed when allocation fails.
        if let Some((place, dynamic_indices)) = self.operand_place_with_indices(argument) {
            let source_pointer = self.lower_place_address(
                &place,
                &dynamic_indices,
                instructions,
                Some(argument.span),
            )?;
            let byte = self
                .type_table
                .lower(self.type_table.semantic.core().uint8, Some(argument.span))
                .ok()?;
            let target = self.type_table.pointer(byte, AddressSpace::Generic);
            let value = self.new_value();
            instructions.push(Instruction {
                result: Some(TypedValue { value, ty: target }),
                kind: InstructionKind::Cast {
                    op: CastOp::PointerCast,
                    value: source_pointer,
                    target,
                },
                span: Some(argument.span),
            });
            return Some(value);
        }
        let source = self.lower_required_operand(argument, instructions)?;
        let source_ty = self
            .type_table
            .lower(argument.ty, Some(argument.span))
            .ok()?;
        let stack_pointer_ty = self.type_table.stack_pointer(source_ty);
        let stack_pointer = self.new_value();
        instructions.push(Instruction {
            result: Some(TypedValue {
                value: stack_pointer,
                ty: stack_pointer_ty,
            }),
            kind: InstructionKind::StackAlloc {
                ty: source_ty,
                count: None,
            },
            span: Some(argument.span),
        });
        instructions.push(Instruction {
            result: None,
            kind: InstructionKind::Store {
                pointer: stack_pointer,
                value: source,
                alignment: 1,
                volatile: false,
            },
            span: Some(argument.span),
        });
        let byte = self
            .type_table
            .lower(self.type_table.semantic.core().uint8, Some(argument.span))
            .ok()?;
        let target = self.type_table.pointer(byte, AddressSpace::Generic);
        if stack_pointer_ty == target {
            return Some(stack_pointer);
        }
        let value = self.new_value();
        instructions.push(Instruction {
            result: Some(TypedValue { value, ty: target }),
            kind: InstructionKind::Cast {
                op: CastOp::PointerCast,
                value: stack_pointer,
                target,
            },
            span: Some(argument.span),
        });
        Some(value)
    }

    fn lower_descriptor_call_argument(
        &mut self,
        argument: &MirOperand,
        expected: Option<SemanticTypeId>,
        instructions: &mut Vec<Instruction>,
    ) -> Option<ValueId> {
        let Some(TypeKind::Capability { inner, .. }) =
            expected.and_then(|ty| self.type_table.semantic.kind(ty).cloned())
        else {
            self.errors.push(self.error(
                Some(argument.span),
                "descriptor runtime argument is missing a capability type",
            ));
            return None;
        };
        // Function parameters have one canonical descriptor layout.  A
        // region-owned source descriptor is adapted below with a narrow
        // pointer cast, keeping the function-type ABI independent of the
        // caller's storage address space.
        let pointee = self.type_table.lower(inner, Some(argument.span)).ok()?;
        let target = self.type_table.intern(Type::Pointer {
            pointee,
            address_space: AddressSpace::Generic,
        });
        let source_pointer = if matches!(
            self.type_table.semantic.kind(argument.ty),
            Some(TypeKind::Capability {
                capability: Capability::Write,
                inner: capability_inner,
            }) if matches!(self.type_table.semantic.kind(*capability_inner), Some(TypeKind::Buffer(_)))
        ) {
            // A write Buffer parameter is already a pointer to the owning
            // descriptor. Loading the capability local yields that pointer;
            // taking its address would pass a pointer-to-pointer and silently
            // update only the callee's temporary slot.
            self.lower_required_operand(argument, instructions)?
        } else {
            let Some((place, dynamic_indices)) = self.operand_place_with_indices(argument) else {
                self.errors.push(self.error(
                    Some(argument.span),
                    "descriptor runtime argument requires an addressable owning string",
                ));
                return None;
            };
            self.lower_place_address(&place, &dynamic_indices, instructions, Some(argument.span))?
        };
        // A projected descriptor may live in a heap/region buffer even though
        // the runtime ABI consumes a generic pointer.  The place lowerer has
        // already proved every index and produced the correct pointee type;
        // normalize only the address space at this narrow ABI boundary.
        let value = self.new_value();
        instructions.push(Instruction {
            result: Some(TypedValue { value, ty: target }),
            kind: InstructionKind::Cast {
                op: crate::CastOp::PointerCast,
                value: source_pointer,
                target,
            },
            span: Some(argument.span),
        });
        Some(value)
    }

    /// Recover an addressable MIR place and its dynamic index operands from a
    /// call argument.  MIR keeps index expressions as an operand tree for
    /// calls (unlike assignment destinations, which already carry a separate
    /// index list), so descriptor-consuming builtins must rebuild the exact
    /// projection order before asking JIR to lower the address.
    fn operand_place_with_indices(&self, operand: &MirOperand) -> Option<(Place, Vec<MirOperand>)> {
        match &operand.kind {
            MirOperandKind::Place(place) => Some((place.clone(), Vec::new())),
            MirOperandKind::Index { base, index } => {
                let (mut place, mut indices) = self.operand_place_with_indices(base)?;
                place.projection.push(Projection::Index);
                indices.push((**index).clone());
                Some((place, indices))
            }
            MirOperandKind::Field { base, field } => {
                let (mut place, indices) = self.operand_place_with_indices(base)?;
                place.projection.push(Projection::Field(field.clone()));
                Some((place, indices))
            }
            _ => None,
        }
    }

    fn lower_borrow_from_place(
        &mut self,
        source: &Place,
        dynamic_indices: &[MirOperand],
        inner: SemanticTypeId,
        instructions: &mut Vec<Instruction>,
        span: Span,
    ) -> Option<ValueId> {
        let source_pointer =
            self.lower_place_address(source, dynamic_indices, instructions, Some(span))?;
        if matches!(self.type_table.semantic.kind(inner), Some(TypeKind::String)) {
            let value_ty = self.type_table.lower(inner, Some(span)).ok()?;
            let value = self.new_value();
            instructions.push(Instruction {
                result: Some(TypedValue {
                    value,
                    ty: value_ty,
                }),
                kind: InstructionKind::Load {
                    pointer: source_pointer,
                    alignment: 1,
                    volatile: false,
                },
                span: Some(span),
            });
            return Some(value);
        }
        if matches!(
            self.type_table.semantic.kind(inner),
            Some(TypeKind::OwnedString)
        ) {
            let source_ty = if source.projection.is_empty() {
                self.local_types.get(source.local.index()).copied()
            } else {
                self.type_table.lower(inner, Some(span)).ok()
            }?;
            let fields = match self.type_table.types.get(source_ty.index()) {
                Some(Type::Struct { fields }) if fields.len() >= 3 => fields.clone(),
                _ => {
                    self.errors.push(self.error(
                        Some(span),
                        "OwnedString borrow source has no pointer/length/capacity representation",
                    ));
                    return None;
                }
            };
            let source_value = self.new_value();
            instructions.push(Instruction {
                result: Some(TypedValue {
                    value: source_value,
                    ty: source_ty,
                }),
                kind: InstructionKind::Load {
                    pointer: source_pointer,
                    alignment: 1,
                    volatile: false,
                },
                span: Some(span),
            });
            let data = self.new_value();
            instructions.push(Instruction {
                result: Some(TypedValue {
                    value: data,
                    ty: fields[0],
                }),
                kind: InstructionKind::ExtractValue {
                    aggregate: source_value,
                    index: 0,
                },
                span: Some(span),
            });
            let view_ty = self
                .type_table
                .lower(self.type_table.semantic.core().string, Some(span))
                .ok()?;
            let view_data_ty = match self.type_table.types.get(view_ty.index()) {
                Some(Type::Struct { fields }) if fields.len() >= 2 => fields[0],
                _ => {
                    self.errors.push(self.error(
                        Some(span),
                        "String view has no pointer/length representation",
                    ));
                    return None;
                }
            };
            let data = if fields[0] == view_data_ty {
                data
            } else {
                let cast = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: cast,
                        ty: view_data_ty,
                    }),
                    kind: InstructionKind::Cast {
                        op: crate::CastOp::PointerCast,
                        value: data,
                        target: view_data_ty,
                    },
                    span: Some(span),
                });
                cast
            };
            let length = self.new_value();
            instructions.push(Instruction {
                result: Some(TypedValue {
                    value: length,
                    ty: fields[1],
                }),
                kind: InstructionKind::ExtractValue {
                    aggregate: source_value,
                    index: 1,
                },
                span: Some(span),
            });
            return Some({
                let view = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: view,
                        ty: view_ty,
                    }),
                    kind: InstructionKind::Aggregate {
                        elements: vec![data, length],
                    },
                    span: Some(span),
                });
                view
            });
        }
        if let Some(TypeKind::Slice(element)) = self.type_table.semantic.kind(inner).cloned() {
            let source_ty = if source.projection.is_empty() {
                self.local_types.get(source.local.index()).copied()
            } else {
                None
            }?;
            let Some(Type::Array {
                element: source_element,
                length: source_length,
            }) = self.type_table.types.get(source_ty.index()).cloned()
            else {
                // Buffer/Slice sources are handled by the aggregate path below.
                return self.lower_borrow_from_non_array(
                    source_pointer,
                    source,
                    inner,
                    instructions,
                    span,
                );
            };

            let size_ty = self.type_table.intern(Type::Integer {
                signed: false,
                bits: self.type_table.pointer_bits,
            });
            let zero = self.new_value();
            instructions.push(Instruction {
                result: Some(TypedValue {
                    value: zero,
                    ty: size_ty,
                }),
                kind: InstructionKind::Constant(Constant::Integer { value: 0 }),
                span: Some(span),
            });
            let stack_data_ty = self.type_table.intern(Type::Pointer {
                pointee: source_element,
                address_space: AddressSpace::Stack,
            });
            let stack_data = self.new_value();
            instructions.push(Instruction {
                result: Some(TypedValue {
                    value: stack_data,
                    ty: stack_data_ty,
                }),
                kind: InstructionKind::Offset {
                    base: source_pointer,
                    indices: vec![zero],
                },
                span: Some(span),
            });
            let element_ty = match self.type_table.lower(element, Some(span)) {
                Ok(ty) => ty,
                Err(error) => {
                    self.errors.push(error);
                    return None;
                }
            };
            let generic_data_ty = self.type_table.pointer(element_ty, AddressSpace::Generic);
            let data = if stack_data_ty == generic_data_ty {
                stack_data
            } else {
                let cast = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: cast,
                        ty: generic_data_ty,
                    }),
                    kind: InstructionKind::Cast {
                        op: crate::CastOp::PointerCast,
                        value: stack_data,
                        target: generic_data_ty,
                    },
                    span: Some(span),
                });
                cast
            };
            let length = self.new_value();
            instructions.push(Instruction {
                result: Some(TypedValue {
                    value: length,
                    ty: size_ty,
                }),
                kind: InstructionKind::Constant(Constant::Integer {
                    value: source_length as i128,
                }),
                span: Some(span),
            });
            let view_ty = match self
                .type_table
                .lower_borrow_capability(inner, false, Some(span))
            {
                Ok(ty) => ty,
                Err(error) => {
                    self.errors.push(error);
                    return None;
                }
            };
            let view = self.new_value();
            instructions.push(Instruction {
                result: Some(TypedValue {
                    value: view,
                    ty: view_ty,
                }),
                kind: InstructionKind::Aggregate {
                    elements: vec![data, length],
                },
                span: Some(span),
            });
            return Some(view);
        }
        match self.type_table.semantic.kind(inner).cloned() {
            Some(TypeKind::Buffer(element) | TypeKind::Slice(element)) => {
                let source_ty = if source.projection.is_empty() {
                    self.local_types.get(source.local.index()).copied()
                } else {
                    self.type_table.lower(inner, Some(span)).ok()
                }?;
                let fields = match self.type_table.types.get(source_ty.index()) {
                    Some(Type::Struct { fields }) if fields.len() >= 2 => fields.clone(),
                    _ => {
                        self.errors.push(self.error(
                            Some(span),
                            "Buffer/Slice borrow source has no aggregate representation",
                        ));
                        return None;
                    }
                };
                let source_value = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: source_value,
                        ty: source_ty,
                    }),
                    kind: InstructionKind::Load {
                        pointer: source_pointer,
                        alignment: 1,
                        volatile: false,
                    },
                    span: Some(span),
                });
                let data = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: data,
                        ty: fields[0],
                    }),
                    kind: InstructionKind::ExtractValue {
                        aggregate: source_value,
                        index: 0,
                    },
                    span: Some(span),
                });
                let length = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: length,
                        ty: fields[1],
                    }),
                    kind: InstructionKind::ExtractValue {
                        aggregate: source_value,
                        index: 1,
                    },
                    span: Some(span),
                });
                let element_ty = match self.type_table.lower(element, Some(span)) {
                    Ok(ty) => ty,
                    Err(error) => {
                        self.errors.push(error);
                        return None;
                    }
                };
                let generic_data_ty = self.type_table.pointer(element_ty, AddressSpace::Generic);
                let data = if fields[0] == generic_data_ty {
                    data
                } else {
                    let cast = self.new_value();
                    instructions.push(Instruction {
                        result: Some(TypedValue {
                            value: cast,
                            ty: generic_data_ty,
                        }),
                        kind: InstructionKind::Cast {
                            op: crate::CastOp::PointerCast,
                            value: data,
                            target: generic_data_ty,
                        },
                        span: Some(span),
                    });
                    cast
                };
                let view_ty =
                    match self
                        .type_table
                        .lower_borrow_capability(inner, false, Some(span))
                    {
                        Ok(ty) => ty,
                        Err(error) => {
                            self.errors.push(error);
                            return None;
                        }
                    };
                let view = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: view,
                        ty: view_ty,
                    }),
                    kind: InstructionKind::Aggregate {
                        elements: vec![data, length],
                    },
                    span: Some(span),
                });
                Some(view)
            }
            _ => {
                let target = match self
                    .type_table
                    .lower_borrow_capability(inner, false, Some(span))
                {
                    Ok(ty) => ty,
                    Err(error) => {
                        self.errors.push(error);
                        return None;
                    }
                };
                let borrowed = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: borrowed,
                        ty: target,
                    }),
                    kind: InstructionKind::Cast {
                        op: crate::CastOp::PointerCast,
                        value: source_pointer,
                        target,
                    },
                    span: Some(span),
                });
                Some(borrowed)
            }
        }
    }

    fn lower_borrow_from_non_array(
        &mut self,
        source_pointer: ValueId,
        source: &Place,
        inner: SemanticTypeId,
        instructions: &mut Vec<Instruction>,
        span: Span,
    ) -> Option<ValueId> {
        match self.type_table.semantic.kind(inner).cloned() {
            Some(TypeKind::Buffer(element) | TypeKind::Slice(element)) => {
                let source_ty = if source.projection.is_empty() {
                    self.local_types.get(source.local.index()).copied()
                } else {
                    self.type_table.lower(inner, Some(span)).ok()
                }?;
                let fields = match self.type_table.types.get(source_ty.index()) {
                    Some(Type::Struct { fields }) if fields.len() >= 2 => fields.clone(),
                    _ => {
                        self.errors.push(self.error(
                            Some(span),
                            "Buffer/Slice borrow source has no aggregate representation",
                        ));
                        return None;
                    }
                };
                let source_value = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: source_value,
                        ty: source_ty,
                    }),
                    kind: InstructionKind::Load {
                        pointer: source_pointer,
                        alignment: 1,
                        volatile: false,
                    },
                    span: Some(span),
                });
                let data = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: data,
                        ty: fields[0],
                    }),
                    kind: InstructionKind::ExtractValue {
                        aggregate: source_value,
                        index: 0,
                    },
                    span: Some(span),
                });
                let length = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: length,
                        ty: fields[1],
                    }),
                    kind: InstructionKind::ExtractValue {
                        aggregate: source_value,
                        index: 1,
                    },
                    span: Some(span),
                });
                let element_ty = match self.type_table.lower(element, Some(span)) {
                    Ok(ty) => ty,
                    Err(error) => {
                        self.errors.push(error);
                        return None;
                    }
                };
                let generic_data_ty = self.type_table.pointer(element_ty, AddressSpace::Generic);
                let data = if fields[0] == generic_data_ty {
                    data
                } else {
                    let cast = self.new_value();
                    instructions.push(Instruction {
                        result: Some(TypedValue {
                            value: cast,
                            ty: generic_data_ty,
                        }),
                        kind: InstructionKind::Cast {
                            op: crate::CastOp::PointerCast,
                            value: data,
                            target: generic_data_ty,
                        },
                        span: Some(span),
                    });
                    cast
                };
                let view_ty =
                    match self
                        .type_table
                        .lower_borrow_capability(inner, false, Some(span))
                    {
                        Ok(ty) => ty,
                        Err(error) => {
                            self.errors.push(error);
                            return None;
                        }
                    };
                let view = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: view,
                        ty: view_ty,
                    }),
                    kind: InstructionKind::Aggregate {
                        elements: vec![data, length],
                    },
                    span: Some(span),
                });
                Some(view)
            }
            _ => {
                let target = match self
                    .type_table
                    .lower_borrow_capability(inner, false, Some(span))
                {
                    Ok(ty) => ty,
                    Err(error) => {
                        self.errors.push(error);
                        return None;
                    }
                };
                let borrowed = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: borrowed,
                        ty: target,
                    }),
                    kind: InstructionKind::Cast {
                        op: crate::CastOp::PointerCast,
                        value: source_pointer,
                        target,
                    },
                    span: Some(span),
                });
                Some(borrowed)
            }
        }
    }

    fn operand_is_region_owned(&self, operand: &MirOperand) -> bool {
        match &operand.kind {
            MirOperandKind::Place(place) => self
                .source
                .locals
                .get(place.local.index())
                .is_some_and(|local| local.owned_region.is_some()),
            MirOperandKind::Field { base, .. } | MirOperandKind::Index { base, .. } => {
                self.operand_is_region_owned(base)
            }
            _ => false,
        }
    }

    fn operand_capability_pointee(&self, operand: &MirOperand) -> Option<TypeId> {
        let MirOperandKind::Place(place) = &operand.kind else {
            return None;
        };
        if !place.projection.is_empty() {
            return None;
        }
        let local_ty = *self.local_types.get(place.local.index())?;
        match self.type_table.types.get(local_ty.index()) {
            Some(Type::Pointer { pointee, .. }) => Some(*pointee),
            _ => None,
        }
    }

    fn load_source_place(
        &mut self,
        place: &Place,
        dynamic_indices: &[MirOperand],
        instructions: &mut Vec<Instruction>,
        span: Span,
    ) -> Option<(ValueId, SemanticTypeId)> {
        let mut semantic_ty = self.source.locals.get(place.local.index())?.ty;
        let provided_indices = dynamic_indices;
        let mut dynamic_indices = provided_indices.iter();
        for projection in &place.projection {
            if let Some(TypeKind::Capability { inner, .. }) =
                self.type_table.semantic.kind(semantic_ty).cloned()
            {
                semantic_ty = inner;
            }
            semantic_ty = match projection {
                Projection::Field(field) => {
                    match self
                        .type_table
                        .field_index_and_type(semantic_ty, field, Some(span))
                    {
                        Ok((_, field_ty)) => field_ty,
                        Err(error) => {
                            self.errors.push(error);
                            return None;
                        }
                    }
                }
                Projection::Dereference => {
                    let Some(TypeKind::Pointer(pointee)) =
                        self.type_table.semantic.kind(semantic_ty).cloned()
                    else {
                        self.errors.push(self.error(
                            Some(span),
                            "pattern/carrier source dereference requires a Pointer<T> place",
                        ));
                        return None;
                    };
                    pointee
                }
                Projection::Index => {
                    let Some(index_operand) = dynamic_indices.next() else {
                        self.errors.push(self.error(
                            Some(span),
                            "pattern/carrier source index projection requires an explicit index operand",
                        ));
                        return None;
                    };
                    let element = match self.type_table.semantic.kind(semantic_ty).cloned() {
                        Some(TypeKind::Array { element, .. })
                        | Some(TypeKind::Buffer(element))
                        | Some(TypeKind::Slice(element)) => element,
                        _ => {
                            self.errors.push(self.error(
                                Some(span),
                                "pattern/carrier source indexing requires an array, Buffer, or Slice",
                            ));
                            return None;
                        }
                    };
                    let _ = index_operand;
                    element
                }
            };
        }
        if dynamic_indices.next().is_some() {
            self.errors.push(self.error(
                Some(span),
                "pattern/carrier source has more index operands than index projections",
            ));
            return None;
        }
        let ty = match self.type_table.lower(semantic_ty, Some(span)) {
            Ok(ty) => ty,
            Err(error) => {
                self.errors.push(error);
                return None;
            }
        };
        let pointer =
            self.lower_place_address(place, provided_indices, instructions, Some(span))?;
        let value = self.new_value();
        instructions.push(Instruction {
            result: Some(TypedValue { value, ty }),
            kind: InstructionKind::Load {
                pointer,
                alignment: 1,
                volatile: false,
            },
            span: Some(span),
        });
        Some((value, semantic_ty))
    }

    fn lower_pattern_extract(
        &mut self,
        source: &Place,
        source_indices: &[MirOperand],
        path: &[String],
        instructions: &mut Vec<Instruction>,
        span: Span,
    ) -> Option<ValueId> {
        let (mut value, mut ty) =
            self.load_source_place(source, source_indices, instructions, span)?;
        let mut variant: Option<(u32, Vec<SemanticTypeId>)> = None;
        for segment in path {
            if let Some(path) = segment.strip_prefix("$variant:") {
                variant = match self
                    .type_table
                    .variant_index_and_fields(ty, path, Some(span))
                {
                    Ok(variant) => Some(variant),
                    Err(error) => {
                        self.errors.push(error);
                        return None;
                    }
                };
                continue;
            }
            let Some(index) = segment
                .strip_prefix("$payload")
                .and_then(|index| index.parse::<usize>().ok())
            else {
                self.errors.push(self.error(
                    Some(span),
                    format!("invalid MIR pattern extraction segment {segment:?}"),
                ));
                return None;
            };
            let Some((variant_index, fields)) = variant.take() else {
                self.errors.push(self.error(
                    Some(span),
                    "pattern payload segment has no preceding variant",
                ));
                return None;
            };
            let Some(field_ty) = fields.get(index).copied() else {
                self.errors
                    .push(self.error(Some(span), "pattern payload index exceeds enum layout"));
                return None;
            };
            let jir_ty = match self.type_table.lower(field_ty, Some(span)) {
                Ok(ty) => ty,
                Err(error) => {
                    self.errors.push(error);
                    return None;
                }
            };
            let extracted = self.new_value();
            instructions.push(Instruction {
                result: Some(TypedValue {
                    value: extracted,
                    ty: jir_ty,
                }),
                kind: InstructionKind::EnumExtract {
                    value,
                    variant: variant_index,
                    field: index as u32,
                },
                span: Some(span),
            });
            value = extracted;
            ty = field_ty;
        }
        Some(value)
    }

    fn lower_index_to_size(
        &mut self,
        value: ValueId,
        semantic_ty: SemanticTypeId,
        size_ty: TypeId,
        instructions: &mut Vec<Instruction>,
        span: Span,
    ) -> Option<ValueId> {
        let Some(TypeKind::Integer { signedness, width }) =
            self.type_table.semantic.kind(semantic_ty).cloned()
        else {
            self.errors
                .push(self.error(Some(span), "buffer index is not an integer"));
            return None;
        };
        let bits = integer_bits(width, self.type_table.pointer_bits);
        if bits == self.type_table.pointer_bits && signedness == Signedness::Unsigned {
            return Some(value);
        }
        if bits > self.type_table.pointer_bits {
            self.errors.push(self.error(
                Some(span),
                "index wider than the target pointer size requires a checked narrowing pass",
            ));
            return None;
        }
        let op = if bits < self.type_table.pointer_bits {
            crate::CastOp::IntegerExtend
        } else {
            crate::CastOp::Bitcast
        };
        let converted = self.new_value();
        instructions.push(Instruction {
            result: Some(TypedValue {
                value: converted,
                ty: size_ty,
            }),
            kind: InstructionKind::Cast {
                op,
                value,
                target: size_ty,
            },
            span: Some(span),
        });
        Some(converted)
    }

    fn lower_place_address(
        &mut self,
        place: &Place,
        dynamic_indices: &[MirOperand],
        instructions: &mut Vec<Instruction>,
        span: Option<Span>,
    ) -> Option<ValueId> {
        let mut address = self
            .local_addresses
            .get(place.local.index())
            .copied()
            .or_else(|| {
                self.errors.push(self.error(
                    span,
                    format!("MIR local #{} does not exist", place.local.index()),
                ));
                None
            })?;
        let mut current_ty = self.source.locals.get(place.local.index())?.ty;
        let mut current_address_space = AddressSpace::Stack;
        let mut specialized_current_jir = None;
        let mut capability_view = None;
        if !place.projection.is_empty()
            && let Some(TypeKind::Capability { inner, .. }) =
                self.type_table.semantic.kind(current_ty).cloned()
        {
            let capability_ty = self.local_types.get(place.local.index()).copied()?;
            specialized_current_jir = match self.type_table.types.get(capability_ty.index()) {
                Some(Type::Pointer { pointee, .. }) => Some(*pointee),
                Some(Type::Struct { .. }) => Some(capability_ty),
                _ => None,
            };
            let dereferenced = self.new_value();
            instructions.push(Instruction {
                result: Some(TypedValue {
                    value: dereferenced,
                    ty: capability_ty,
                }),
                kind: InstructionKind::Load {
                    pointer: address,
                    alignment: 1,
                    volatile: false,
                },
                span,
            });
            if matches!(
                self.type_table.types.get(capability_ty.index()),
                Some(Type::Pointer { .. })
            ) {
                address = dereferenced;
            } else {
                capability_view = Some(dereferenced);
            }
            current_ty = inner;
            current_address_space = AddressSpace::Generic;
        }
        let mut dynamic_indices = dynamic_indices.iter();
        for projection in &place.projection {
            match projection {
                Projection::Field(field) => {
                    let (field_index, field_ty) = match self
                        .type_table
                        .field_index_and_type(current_ty, field, span)
                    {
                        Ok(field) => field,
                        Err(error) => {
                            self.errors.push(error);
                            return None;
                        }
                    };
                    let index_ty = self.type_table.intern(Type::Integer {
                        signed: false,
                        bits: self.type_table.pointer_bits,
                    });
                    let index = self.new_value();
                    instructions.push(Instruction {
                        result: Some(TypedValue {
                            value: index,
                            ty: index_ty,
                        }),
                        kind: InstructionKind::Constant(Constant::Integer {
                            value: i128::from(field_index),
                        }),
                        span,
                    });
                    let pointee = match self.type_table.lower(field_ty, span) {
                        Ok(ty) => ty,
                        Err(error) => {
                            self.errors.push(error);
                            return None;
                        }
                    };
                    let pointer_ty = self.type_table.pointer(pointee, current_address_space);
                    let projected = self.new_value();
                    instructions.push(Instruction {
                        result: Some(TypedValue {
                            value: projected,
                            ty: pointer_ty,
                        }),
                        kind: InstructionKind::Offset {
                            base: address,
                            indices: vec![index],
                        },
                        span,
                    });
                    address = projected;
                    current_ty = field_ty;
                    specialized_current_jir = None;
                }
                Projection::Index => {
                    let Some(index_operand) = dynamic_indices.next() else {
                        self.errors.push(self.error(
                            span,
                            "projected MIR place is missing its dynamic index operand",
                        ));
                        return None;
                    };
                    let index = self.lower_required_operand(index_operand, instructions)?;
                    match self.type_table.semantic.kind(current_ty).cloned() {
                        Some(TypeKind::Array { element, length }) => {
                            let index_ty = match self.type_table.lower(index_operand.ty, span) {
                                Ok(ty) => ty,
                                Err(error) => {
                                    self.errors.push(error);
                                    return None;
                                }
                            };
                            let length_value = self.new_value();
                            instructions.push(Instruction {
                                result: Some(TypedValue {
                                    value: length_value,
                                    ty: index_ty,
                                }),
                                kind: InstructionKind::Constant(Constant::Integer {
                                    value: i128::from(length),
                                }),
                                span,
                            });
                            instructions.push(Instruction {
                                result: None,
                                kind: InstructionKind::BoundsCheck {
                                    index,
                                    length: length_value,
                                },
                                span,
                            });
                            let pointee = match self.type_table.lower(element, span) {
                                Ok(ty) => ty,
                                Err(error) => {
                                    self.errors.push(error);
                                    return None;
                                }
                            };
                            let pointer_ty =
                                self.type_table.pointer(pointee, current_address_space);
                            let projected = self.new_value();
                            instructions.push(Instruction {
                                result: Some(TypedValue {
                                    value: projected,
                                    ty: pointer_ty,
                                }),
                                kind: InstructionKind::Offset {
                                    base: address,
                                    indices: vec![index],
                                },
                                span,
                            });
                            address = projected;
                            current_ty = element;
                        }
                        Some(TypeKind::Buffer(element) | TypeKind::Slice(element)) => {
                            let aggregate_ty =
                                if let Some(ty) = specialized_current_jir.take() {
                                    ty
                                } else if place.projection.first().is_some_and(|projection| {
                                    matches!(projection, Projection::Index)
                                }) && self
                                    .source
                                    .locals
                                    .get(place.local.index())
                                    .is_some_and(|local| local.owned_region.is_some())
                                {
                                    // Region-backed Buffer locals use a specialized JIR
                                    // descriptor whose data pointer lives in the region
                                    // address space.  Lowering the source-level Buffer<T>
                                    // here would recreate the default heap descriptor and
                                    // make the subsequent Load/ExtractValue types disagree.
                                    self.local_types.get(place.local.index()).copied()?
                                } else {
                                    match self.type_table.lower(current_ty, span) {
                                        Ok(ty) => ty,
                                        Err(error) => {
                                            self.errors.push(error);
                                            return None;
                                        }
                                    }
                                };
                            let aggregate = if let Some(view) = capability_view.take() {
                                view
                            } else {
                                let aggregate = self.new_value();
                                instructions.push(Instruction {
                                    result: Some(TypedValue {
                                        value: aggregate,
                                        ty: aggregate_ty,
                                    }),
                                    kind: InstructionKind::Load {
                                        pointer: address,
                                        alignment: 1,
                                        volatile: false,
                                    },
                                    span,
                                });
                                aggregate
                            };
                            let address_space = if capability_view.is_some()
                                || matches!(
                                    self.type_table.types.get(aggregate_ty.index()),
                                    Some(Type::Struct { fields })
                                        if fields.len() == 2
                                            && matches!(
                                                self.type_table.types.get(fields[0].index()),
                                                Some(Type::Pointer {
                                                    address_space: AddressSpace::Generic,
                                                    ..
                                                })
                                            )
                                ) {
                                AddressSpace::Generic
                            } else if self
                                .source
                                .locals
                                .get(place.local.index())
                                .is_some_and(|local| local.owned_region.is_some())
                            {
                                AddressSpace::Region
                            } else if matches!(
                                self.type_table.semantic.kind(current_ty),
                                Some(TypeKind::Buffer(_))
                            ) {
                                AddressSpace::Heap
                            } else {
                                AddressSpace::Generic
                            };
                            let pointee = match self.type_table.lower(element, span) {
                                Ok(ty) => ty,
                                Err(error) => {
                                    self.errors.push(error);
                                    return None;
                                }
                            };
                            let data_ty = self.type_table.intern(Type::Pointer {
                                pointee,
                                address_space,
                            });
                            let size_ty = self.type_table.intern(Type::Integer {
                                signed: false,
                                bits: self.type_table.pointer_bits,
                            });
                            let raw_data_ty = match self.type_table.types.get(aggregate_ty.index())
                            {
                                Some(Type::Struct { fields }) => fields.first().copied()?,
                                _ => {
                                    self.errors.push(self.error(
                                        span,
                                        "Buffer/Slice aggregate has no data pointer field",
                                    ));
                                    return None;
                                }
                            };
                            let raw_data = self.new_value();
                            instructions.push(Instruction {
                                result: Some(TypedValue {
                                    value: raw_data,
                                    ty: raw_data_ty,
                                }),
                                kind: InstructionKind::ExtractValue {
                                    aggregate,
                                    index: 0,
                                },
                                span,
                            });
                            let data = if raw_data_ty == data_ty {
                                raw_data
                            } else {
                                let cast = self.new_value();
                                instructions.push(Instruction {
                                    result: Some(TypedValue {
                                        value: cast,
                                        ty: data_ty,
                                    }),
                                    kind: InstructionKind::Cast {
                                        op: CastOp::PointerCast,
                                        value: raw_data,
                                        target: data_ty,
                                    },
                                    span,
                                });
                                cast
                            };
                            let length = self.new_value();
                            instructions.push(Instruction {
                                result: Some(TypedValue {
                                    value: length,
                                    ty: size_ty,
                                }),
                                kind: InstructionKind::ExtractValue {
                                    aggregate,
                                    index: 1,
                                },
                                span,
                            });
                            let index = self.lower_index_to_size(
                                index,
                                index_operand.ty,
                                size_ty,
                                instructions,
                                index_operand.span,
                            )?;
                            instructions.push(Instruction {
                                result: None,
                                kind: InstructionKind::BoundsCheck { index, length },
                                span,
                            });
                            let projected = self.new_value();
                            instructions.push(Instruction {
                                result: Some(TypedValue {
                                    value: projected,
                                    ty: data_ty,
                                }),
                                kind: InstructionKind::Offset {
                                    base: data,
                                    indices: vec![index],
                                },
                                span,
                            });
                            address = projected;
                            current_ty = element;
                            current_address_space = address_space;
                        }
                        _ => {
                            self.errors.push(self.error(
                                span,
                                "projected JIR place indexing requires an array, Buffer, or Slice",
                            ));
                            return None;
                        }
                    }
                }
                Projection::Dereference => {
                    let Some(TypeKind::Pointer(pointee)) =
                        self.type_table.semantic.kind(current_ty).cloned()
                    else {
                        self.errors.push(self.error(
                            span,
                            "pointer dereference projection requires a Pointer<T> place",
                        ));
                        return None;
                    };
                    let pointer_ty = match self.type_table.lower(current_ty, span) {
                        Ok(ty) => ty,
                        Err(error) => {
                            self.errors.push(error);
                            return None;
                        }
                    };
                    let loaded = self.new_value();
                    instructions.push(Instruction {
                        result: Some(TypedValue {
                            value: loaded,
                            ty: pointer_ty,
                        }),
                        kind: InstructionKind::Load {
                            pointer: address,
                            alignment: 1,
                            volatile: false,
                        },
                        span,
                    });
                    address = loaded;
                    current_ty = pointee;
                    current_address_space = AddressSpace::Generic;
                    specialized_current_jir = None;
                    capability_view = None;
                }
            }
        }
        if dynamic_indices.next().is_some() {
            self.errors.push(self.error(
                span,
                "MIR assignment has more dynamic indices than place projections",
            ));
            return None;
        }
        Some(address)
    }

    fn new_value(&mut self) -> ValueId {
        let value = ValueId::new(self.next_value);
        self.next_value += 1;
        value
    }

    fn error(&self, span: Option<Span>, message: impl Into<String>) -> LowerError {
        LowerError {
            span,
            message: message.into(),
        }
    }
}

fn lower_block(block: MirBlockId) -> BlockId {
    BlockId::new(block.index())
}

fn numeric_cast_op(
    source: Option<&TypeKind>,
    target: Option<&TypeKind>,
    pointer_bits: u16,
) -> Option<CastOp> {
    match (source, target) {
        (
            Some(TypeKind::Integer { width: source, .. }),
            Some(TypeKind::Integer { width: target, .. }),
        ) => {
            let source = integer_bits(*source, pointer_bits);
            let target = integer_bits(*target, pointer_bits);
            Some(if source < target {
                CastOp::IntegerExtend
            } else if source > target {
                CastOp::IntegerTruncate
            } else {
                CastOp::Bitcast
            })
        }
        (Some(TypeKind::Integer { .. }), Some(TypeKind::Float { .. })) => {
            Some(CastOp::IntegerToFloat)
        }
        (Some(TypeKind::Float { .. }), Some(TypeKind::Integer { .. })) => {
            Some(CastOp::FloatToInteger)
        }
        (Some(TypeKind::Float(source)), Some(TypeKind::Float(target))) => {
            let source = float_bits(*source);
            let target = float_bits(*target);
            Some(if source < target {
                CastOp::FloatExtend
            } else if source > target {
                CastOp::FloatTruncate
            } else {
                CastOp::Bitcast
            })
        }
        _ => None,
    }
}

const fn integer_bits(width: IntegerWidth, pointer_bits: u16) -> u16 {
    match width {
        IntegerWidth::Bits8 => 8,
        IntegerWidth::Bits16 => 16,
        IntegerWidth::Bits32 => 32,
        IntegerWidth::Bits64 => 64,
        IntegerWidth::Pointer => pointer_bits,
    }
}

const fn float_bits(width: FloatWidth) -> u16 {
    match width {
        FloatWidth::Bits16 => 16,
        FloatWidth::Bits32 => 32,
        FloatWidth::Bits64 => 64,
    }
}

fn lower_constant(text: &str, kind: Option<&TypeKind>) -> Result<Constant, String> {
    match kind {
        Some(TypeKind::Bool) => match text {
            "true" => Ok(Constant::Bool(true)),
            "false" => Ok(Constant::Bool(false)),
            _ => Err(format!("invalid Bool literal {text:?}")),
        },
        Some(TypeKind::Integer { .. }) => {
            parse_integer(text).map(|value| Constant::Integer { value })
        }
        Some(TypeKind::Char) => {
            let utf8 = decode_quoted(text, '\'')?;
            let decoded = std::str::from_utf8(&utf8)
                .map_err(|error| format!("invalid character UTF-8: {error}"))?;
            let mut characters = decoded.chars();
            let value = characters
                .next()
                .ok_or_else(|| "empty character literal".to_owned())?;
            if characters.next().is_some() {
                return Err("character literal contains more than one scalar".to_owned());
            }
            Ok(Constant::Integer {
                value: i128::from(u32::from(value)),
            })
        }
        Some(TypeKind::Float(width)) => parse_float_bits(text, *width),
        _ => Err(format!("unsupported literal type {kind:?}")),
    }
}

fn decode_quoted(text: &str, quote: char) -> Result<Vec<u8>, String> {
    let body = text
        .strip_prefix(quote)
        .and_then(|text| text.strip_suffix(quote))
        .ok_or_else(|| format!("literal is not enclosed by {quote:?}"))?;
    let mut decoded = String::new();
    let mut characters = body.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            decoded.push(character);
            continue;
        }
        let escaped = characters
            .next()
            .ok_or_else(|| "literal ends with an incomplete escape".to_owned())?;
        decoded.push(match escaped {
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            '0' => '\0',
            '\\' => '\\',
            '"' => '"',
            '\'' => '\'',
            _ => return Err(format!("unsupported literal escape \\{escaped}")),
        });
    }
    Ok(decoded.into_bytes())
}

fn parse_integer(text: &str) -> Result<i128, String> {
    let text = strip_suffix(
        text,
        &[
            "isize", "usize", "i64", "u64", "i32", "u32", "i16", "u16", "i8", "u8",
        ],
    );
    let digits = text.replace('_', "");
    let (radix, digits) = if let Some(digits) = digits.strip_prefix("0x") {
        (16, digits)
    } else if let Some(digits) = digits.strip_prefix("0o") {
        (8, digits)
    } else if let Some(digits) = digits.strip_prefix("0b") {
        (2, digits)
    } else {
        (10, digits.as_str())
    };
    i128::from_str_radix(digits, radix).map_err(|error| format!("invalid integer literal: {error}"))
}

fn parse_float_bits(text: &str, width: FloatWidth) -> Result<Constant, String> {
    let text = strip_suffix(text, &["f64", "f32", "f16"]);
    let normalized = text.replace('_', "");
    match width {
        FloatWidth::Bits64 => normalized
            .parse::<f64>()
            .map(|value| Constant::FloatBits {
                bits: value.to_bits(),
            })
            .map_err(|error| format!("invalid Float64 literal: {error}")),
        FloatWidth::Bits32 => normalized
            .parse::<f32>()
            .map(|value| Constant::FloatBits {
                bits: u64::from(value.to_bits()),
            })
            .map_err(|error| format!("invalid Float32 literal: {error}")),
        FloatWidth::Bits16 => normalized
            .parse::<f32>()
            .map(|value| Constant::FloatBits {
                bits: u64::from(f32_to_f16_bits(value)),
            })
            .map_err(|error| format!("invalid Float16 literal: {error}")),
    }
}

fn f32_to_f16_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x7f_ffff;
    if exponent == 0xff {
        let payload = (mantissa >> 13) as u16;
        return sign | 0x7c00 | if payload == 0 { 0 } else { payload | 1 };
    }
    let mut half_exponent = exponent - 127 + 15;
    if half_exponent >= 31 {
        return sign | 0x7c00;
    }
    if half_exponent <= 0 {
        if half_exponent < -10 {
            return sign;
        }
        let mantissa = mantissa | 0x80_0000;
        let shift = (14 - half_exponent) as u32;
        let mut rounded = mantissa >> shift;
        let round_bit = 1_u32 << (shift - 1);
        if mantissa & round_bit != 0 && (mantissa & (round_bit - 1) != 0 || rounded & 1 != 0) {
            rounded += 1;
        }
        return sign | rounded as u16;
    }
    let mut rounded_mantissa = mantissa + 0x1000;
    if rounded_mantissa & 0x80_0000 != 0 {
        rounded_mantissa = 0;
        half_exponent += 1;
        if half_exponent >= 31 {
            return sign | 0x7c00;
        }
    }
    sign | ((half_exponent as u16) << 10) | ((rounded_mantissa >> 13) as u16)
}

fn strip_suffix<'a>(text: &'a str, suffixes: &[&str]) -> &'a str {
    suffixes
        .iter()
        .find_map(|suffix| text.strip_suffix(suffix))
        .unwrap_or(text)
}

const fn lower_unary(operator: Operator) -> Option<UnaryOp> {
    match operator {
        Operator::Minus => Some(UnaryOp::Negate),
        Operator::Bang => Some(UnaryOp::Not),
        Operator::Tilde => Some(UnaryOp::BitNot),
        _ => None,
    }
}

const fn lower_binary(operator: Operator) -> Option<BinaryOp> {
    match operator {
        Operator::Plus => Some(BinaryOp::Add),
        Operator::Minus => Some(BinaryOp::Subtract),
        Operator::Star => Some(BinaryOp::Multiply),
        Operator::Slash => Some(BinaryOp::Divide),
        Operator::Percent => Some(BinaryOp::Remainder),
        Operator::Ampersand | Operator::And => Some(BinaryOp::BitAnd),
        Operator::Pipe | Operator::Or => Some(BinaryOp::BitOr),
        Operator::Caret => Some(BinaryOp::BitXor),
        _ => None,
    }
}

const fn lower_compare(operator: Operator) -> Option<ComparePredicate> {
    match operator {
        Operator::Equal => Some(ComparePredicate::Equal),
        Operator::NotEqual => Some(ComparePredicate::NotEqual),
        Operator::Less => Some(ComparePredicate::Less),
        Operator::LessEqual => Some(ComparePredicate::LessEqual),
        Operator::Greater => Some(ComparePredicate::Greater),
        Operator::GreaterEqual => Some(ComparePredicate::GreaterEqual),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use jadren_hir::lower_hir;
    use jadren_lexer::lex;
    use jadren_mir::{
        CarrierPart, MirOperand, MirOperandKind, Place, Projection, Terminator, elaborate_drops,
        elaborate_region_cleanup, infer_lifetimes, lower_mir, materialize_returns,
    };
    use jadren_parser::parse;
    use jadren_resolve::resolve;
    use jadren_source::SourceManager;
    use jadren_typeck::check_types;
    use jadren_types::NominalLayoutKind;

    use super::{
        Capability, LowerOptions, Type, borrowed_runtime_argument_capability, lower_from_mir,
    };

    #[test]
    fn converts_float16_literals_with_ieee_rounding() {
        assert_eq!(super::f32_to_f16_bits(1.5), 0x3e00);
        assert_eq!(super::f32_to_f16_bits(f32::INFINITY), 0x7c00);
        assert_eq!(super::f32_to_f16_bits(f32::NEG_INFINITY), 0xfc00);
        assert_eq!(super::f32_to_f16_bits(0.0), 0);
    }
    #[test]
    fn lowers_scalar_if_cfg_and_values_to_deterministic_jir() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "test.jdn",
                "module test; fn choose(flag: Bool) -> Int32 { return if flag { 1 + 2 } else { 3 } }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("scalar MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("branch %v"));
        assert!(text.contains(" = add "));
        assert!(text.contains("jump ^bb3()"));
        assert!(text.contains("return %v"));
        assert_eq!(jir.functions[0].blocks.len(), mir.functions[0].blocks.len());
    }

    #[test]
    fn lowers_internal_pointer_dereference_projection_to_load() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "pointer-dereference.jdn",
                "module test; fn deref_value(value: Pointer<Int32>) -> Int32 { return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        let function = mir.functions.first_mut().expect("pointer function");
        let pointer_local = function
            .locals
            .iter()
            .find(|local| local.is_parameter)
            .expect("pointer parameter")
            .id;
        let mut replaced = false;
        for block in &mut function.blocks {
            if let Terminator::Return {
                value: Some(value), ..
            } = &mut block.terminator
            {
                let ty = value.ty;
                let span = value.span;
                *value = MirOperand {
                    ty,
                    kind: MirOperandKind::Place(Place {
                        local: pointer_local,
                        projection: vec![Projection::Dereference],
                    }),
                    span,
                };
                replaced = true;
                break;
            }
        }
        assert!(replaced, "return operand must be replaced");
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("pointer dereference place must lower");
        let errors = crate::verify(&jir);
        assert!(errors.is_empty(), "{errors:?}");
        let text = jir.to_text();
        assert!(text.matches(" = load ").count() >= 2, "{text}");
        assert!(text.contains("ptr<generic,"), "{text}");
    }

    #[test]
    fn rejects_internal_pointer_dereference_projection_on_scalar_place() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "invalid-pointer-dereference.jdn",
                "module test; fn scalar(value: Int32) -> Int32 { return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        let function = mir.functions.first_mut().expect("scalar function");
        let scalar_local = function
            .locals
            .iter()
            .find(|local| local.is_parameter)
            .expect("scalar parameter")
            .id;
        for block in &mut function.blocks {
            if let Terminator::Return {
                value: Some(value), ..
            } = &mut block.terminator
            {
                let ty = value.ty;
                let span = value.span;
                *value = MirOperand {
                    ty,
                    kind: MirOperandKind::Place(Place {
                        local: scalar_local,
                        projection: vec![Projection::Dereference],
                    }),
                    span,
                };
                break;
            }
        }
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let errors = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect_err("scalar dereference must be rejected");
        assert!(errors.iter().any(|error| {
            error
                .message
                .contains("pointer dereference projection requires a Pointer<T> place")
        }));
    }

    #[test]
    fn lowers_pattern_extract_from_projected_record_place() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "pattern-projected-record.jdn",
                "module test; struct Pair { value: Int32 } fn project(pair: Pair) -> Int32 { return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        let function = mir
            .functions
            .iter_mut()
            .find(|function| function.name == "project")
            .expect("record function");
        let pair_local = function
            .locals
            .iter()
            .find(|local| local.is_parameter)
            .expect("record parameter")
            .id;
        let mut replaced = false;
        for block in &mut function.blocks {
            if let Terminator::Return {
                value: Some(value), ..
            } = &mut block.terminator
            {
                let ty = value.ty;
                let span = value.span;
                *value = MirOperand {
                    ty,
                    kind: MirOperandKind::PatternExtract {
                        source: Place {
                            local: pair_local,
                            projection: vec![Projection::Field("value".to_owned())],
                        },
                        source_indices: Vec::new(),
                        path: Vec::new(),
                        borrowed: false,
                    },
                    span,
                };
                replaced = true;
                break;
            }
        }
        assert!(replaced, "return operand must be replaced");
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("projected pattern source must lower");
        let errors = crate::verify(&jir);
        assert!(errors.is_empty(), "{errors:?}");
        let text = jir.to_text();
        assert!(text.contains(" = offset "), "{text}");
        assert!(text.matches(" = load ").count() >= 2, "{text}");
    }

    #[test]
    fn rejects_pattern_extract_from_indexed_source_without_index_operand() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "pattern-indexed-source.jdn",
                "module test; fn project(values: Buffer<Int32>) -> Int32 { return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        let function = mir
            .functions
            .iter_mut()
            .find(|function| function.name == "project")
            .expect("buffer function");
        let values_local = function
            .locals
            .iter()
            .find(|local| local.is_parameter)
            .expect("buffer parameter")
            .id;
        for block in &mut function.blocks {
            if let Terminator::Return {
                value: Some(value), ..
            } = &mut block.terminator
            {
                let ty = value.ty;
                let span = value.span;
                *value = MirOperand {
                    ty,
                    kind: MirOperandKind::PatternExtract {
                        source: Place {
                            local: values_local,
                            projection: vec![Projection::Index],
                        },
                        source_indices: Vec::new(),
                        path: Vec::new(),
                        borrowed: false,
                    },
                    span,
                };
                break;
            }
        }
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let errors = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect_err("indexed pattern source without an operand must be rejected");
        assert!(errors.iter().any(|error| {
            error.message.contains(
                "pattern/carrier source index projection requires an explicit index operand",
            )
        }));
    }

    #[test]
    fn lowers_pattern_extract_from_indexed_source_with_index_operand() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "pattern-indexed-source-valid.jdn",
                "module test; fn project(values: Buffer<Int32>) -> Int32 { return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        let function = mir
            .functions
            .iter_mut()
            .find(|function| function.name == "project")
            .expect("buffer function");
        let values_local = function
            .locals
            .iter()
            .find(|local| local.is_parameter)
            .expect("buffer parameter")
            .id;
        for block in &mut function.blocks {
            if let Terminator::Return {
                value: Some(value), ..
            } = &mut block.terminator
            {
                let ty = value.ty;
                let span = value.span;
                *value = MirOperand {
                    ty,
                    kind: MirOperandKind::PatternExtract {
                        source: Place {
                            local: values_local,
                            projection: vec![Projection::Index],
                        },
                        source_indices: vec![MirOperand {
                            ty,
                            kind: MirOperandKind::Literal(jadren_hir::HirLiteral {
                                kind: jadren_parser::LiteralKind::Integer,
                                text: "0".to_owned(),
                            }),
                            span,
                        }],
                        path: Vec::new(),
                        borrowed: false,
                    },
                    span,
                };
                break;
            }
        }
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("indexed pattern source must lower");
        let errors = crate::verify(&jir);
        assert!(errors.is_empty(), "{errors:?}");
        let text = jir.to_text();
        assert!(text.contains("bounds_check "), "{text}");
        assert!(text.contains(" = offset "), "{text}");
        assert!(text.matches(" = load ").count() >= 2, "{text}");
    }

    #[test]
    fn lowers_carrier_extract_from_indexed_source_with_index_operand() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "carrier-indexed-source-valid.jdn",
                "module test; fn project(values: Buffer<Option<Int32>>) -> Int32 { return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        let function = mir
            .functions
            .iter_mut()
            .find(|function| function.name == "project")
            .expect("carrier function");
        let values_local = function
            .locals
            .iter()
            .find(|local| local.is_parameter)
            .expect("carrier parameter")
            .id;
        for block in &mut function.blocks {
            if let Terminator::Return {
                value: Some(value), ..
            } = &mut block.terminator
            {
                let ty = value.ty;
                let span = value.span;
                *value = MirOperand {
                    ty,
                    kind: MirOperandKind::CarrierExtract {
                        source: Place {
                            local: values_local,
                            projection: vec![Projection::Index],
                        },
                        source_indices: vec![MirOperand {
                            ty,
                            kind: MirOperandKind::Literal(jadren_hir::HirLiteral {
                                kind: jadren_parser::LiteralKind::Integer,
                                text: "0".to_owned(),
                            }),
                            span,
                        }],
                        part: CarrierPart::Success,
                    },
                    span,
                };
                break;
            }
        }
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("indexed carrier source must lower");
        let errors = crate::verify(&jir);
        assert!(errors.is_empty(), "{errors:?}");
        let text = jir.to_text();
        assert!(text.contains("bounds_check "), "{text}");
        assert!(text.contains("enum_extract "), "{text}");
    }

    #[test]
    fn lowers_first_class_function_values_to_address_and_indirect_call() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "function-pointer.jdn",
                "fn increment(value: Int32) -> Int32 { return value + 1; }\
                 pub fn apply_increment(value: Int32) -> Int32 {\
                     let callback = increment; return callback(value);\
                 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("function value MIR must lower");
        let errors = crate::verify(&jir);
        assert!(errors.is_empty(), "{errors:?}");
        let text = jir.to_text();
        assert!(text.contains("function_address @f0"), "{text}");
        assert!(text.contains("indirect_call"), "{text}");
    }

    #[test]
    fn lowers_external_function_values_to_import_address_and_indirect_call() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "external-function-pointer.jdn",
                "extern \"C\" { fn external_adjust(value: Int32) -> Int32; }\
                 pub fn apply_external(value: Int32) -> Int32 {\
                     let callback = external_adjust; return callback(value);\
                 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("external function value MIR must lower");
        let errors = crate::verify(&jir);
        assert!(errors.is_empty(), "{errors:?}");
        let text = jir.to_text();
        assert!(text.contains("function_address @f"), "{text}");
        assert!(text.contains("indirect_call"), "{text}");
        assert!(text.contains("fn import"), "{text}");
    }

    #[test]
    fn lowers_indirect_write_buffer_callbacks_with_descriptor_pointer_abi() {
        let mut sources = SourceManager::new();
        let source_text = include_str!("../../../examples/stdlib-buffer-indirect-callback.jdn");
        let id = sources
            .add("buffer-indirect-callback.jdn", source_text)
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("indirect write Buffer callback must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let invoke = jir
            .functions
            .iter()
            .find(|function| function.name == "invoke")
            .expect("callback function");
        let callback_type = &jir.types[invoke.parameters[0].ty.index()];
        let Type::Function { parameters, .. } = callback_type else {
            panic!("callback parameter must lower to a function type: {callback_type:?}");
        };
        assert!(
            matches!(jir.types[parameters[0].index()], Type::Pointer { .. }),
            "callback write Buffer parameter must use a descriptor pointer: {callback_type:?}"
        );
        assert!(
            matches!(
                jir.types[invoke.parameters[1].ty.index()],
                Type::Pointer { .. }
            ),
            "invoke write Buffer parameter must use a descriptor pointer: {invoke:?}"
        );
        assert!(jir.to_text().contains("indirect_call"));
    }

    #[test]
    fn lowers_windows_ui_control_app_state_bindings() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "ui-control-app-state.jdn",
                "module test; fn main() -> Int32 { ui_checkbox_bind_app_state(10, \"enabled\"); ui_checkbox_refresh_app_state(10); ui_select_bind_app_state(20, \"choice\"); ui_select_refresh_app_state(20); ui_list_bind_app_state(30, \"list_choice\"); ui_list_refresh_app_state(30); ui_table_bind_app_state(40, \"table_choice\"); ui_table_refresh_app_state(40); let list: Bool = ui_app_bind_app_state(30, \"list_choice\"); ui_app_refresh_app_state(30); if list { return 0 } return 1 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("control app state bindings must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("\"ui_checkbox_bind_app_state\""), "{text}");
        assert!(text.contains("\"ui_checkbox_refresh_app_state\""), "{text}");
        assert!(text.contains("\"ui_select_bind_app_state\""), "{text}");
        assert!(text.contains("\"ui_select_refresh_app_state\""), "{text}");
        assert!(text.contains("\"ui_list_bind_app_state\""), "{text}");
        assert!(text.contains("\"ui_list_refresh_app_state\""), "{text}");
        assert!(text.contains("\"ui_table_bind_app_state\""), "{text}");
        assert!(text.contains("\"ui_table_refresh_app_state\""), "{text}");
        assert!(text.contains("\"ui_app_bind_app_state\""), "{text}");
        assert!(text.contains("\"ui_app_refresh_app_state\""), "{text}");
    }

    #[test]
    fn lowers_windows_ui_input_read_array_to_borrowed_slice() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "ui-input-array.jdn",
                "module test; fn process_input() -> Int32 { var output: [UInt8; 4] = [0u8, 0u8, 0u8, 0u8]; var length: [UIntSize; 1] = [0usize]; ui_input_read(10, output); ui_input_read_exact(10, output, length); ui_input_bind_app_state(10, \"note\"); ui_input_refresh_app_state(10); return 0 } fn main() -> Int32 { process_input(); return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("array input buffer must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("stack_alloc"), "{text}");
        assert!(text.contains("offset "), "{text}");
        assert!(text.contains("aggregate "), "{text}");
        assert!(text.contains("fn import @f"), "{text}");
        assert!(text.contains("\"ui_input_read\""), "{text}");
        assert!(text.contains("\"ui_input_read_exact\""), "{text}");
        assert!(text.contains("\"ui_input_bind_app_state\""), "{text}");
        assert!(text.contains("\"ui_input_refresh_app_state\""), "{text}");
        assert_eq!(
            borrowed_runtime_argument_capability("ui_input_read_exact", 1),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("ui_input_read_exact", 2),
            Some(Capability::Write)
        );
    }

    #[test]
    fn lowers_windows_ui_collection_reads_to_borrowed_slices() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "ui-collection-read.jdn",
                r#"module test; fn main() -> Int32 { var output: [UInt8; 8] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; ui_state_bind(20, 0, 1); ui_state_bind_text(20, 1); let text_length: UIntSize = ui_state_text_length(1); let text_read: UIntSize = ui_state_text_read(1, output); let list: UIntSize = ui_list_read_item(20, 0, output); let cell: UIntSize = ui_table_read_cell(30, 0, 1, output); ui_list_bind_app(20, 0); ui_list_refresh_app(20); ui_table_bind_app(30, 0, 2); ui_table_refresh_app(30); ui_refresh_bindings(); return (text_length + text_read + list + cell) as Int32 }"#,
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("collection read buffers must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("\"ui_list_read_item\""), "{text}");
        assert!(text.contains("\"ui_table_read_cell\""), "{text}");
        assert!(text.contains("\"ui_state_bind\""), "{text}");
        assert!(text.contains("\"ui_state_bind_text\""), "{text}");
        assert!(text.contains("\"ui_state_text_read\""), "{text}");
        assert!(text.contains("\"ui_list_bind_app\""), "{text}");
        assert!(text.contains("\"ui_table_refresh_app\""), "{text}");
        assert!(text.contains("\"ui_refresh_bindings\""), "{text}");
    }

    #[test]
    fn lowers_json_object_reads_to_borrowed_slices() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "json-object-read.jdn",
                r#"module test; fn main() -> Int32 { region frame { let input: Buffer<UInt8> = frame.allocate(64); let output: Buffer<UInt8> = frame.allocate(32); var text_length: [UIntSize; 1] = [0usize]; var signed_output: [Int64; 1] = [0i64]; var unsigned_output: [UInt64; 1] = [0u64]; var float_output: [Float64; 1] = [0.0f64]; var bool_output: [Bool; 1] = [false]; let text: UIntSize = json_object_read_string(input, "name", output); let signed: Int64 = json_object_read_int(input, "minutes"); let unsigned: UInt64 = json_object_read_uint(input, "total"); let rate: Float64 = json_object_read_float(input, "rate"); let done: Bool = json_object_read_bool(input, "done"); let text_exact: Bool = json_object_read_string_exact(input, "name", output, text_length); let signed_exact: Bool = json_object_read_int_exact(input, "minutes", signed_output); let unsigned_exact: Bool = json_object_read_uint_exact(input, "total", unsigned_output); let float_exact: Bool = json_object_read_float_exact(input, "rate", float_output); let bool_exact: Bool = json_object_read_bool_exact(input, "done", bool_output); if done { if text_exact { if signed_exact { if unsigned_exact { if float_exact { if bool_exact { return (text as Int32) + (signed as Int32) + (unsigned as Int32) + (rate as Int32) } } } } } } return 0 } }"#,
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("JSON object readers must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        for name in [
            "json_object_read_string",
            "json_object_read_int",
            "json_object_read_uint",
            "json_object_read_float",
            "json_object_read_bool",
            "json_object_read_string_exact",
            "json_object_read_int_exact",
            "json_object_read_uint_exact",
            "json_object_read_float_exact",
            "json_object_read_bool_exact",
        ] {
            assert!(text.contains(&format!("\"{name}\"")), "{text}");
        }
        assert!(text.contains("aggregate "), "{text}");
    }

    #[test]
    fn lowers_app_state_text_output_to_borrowed_slice() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "app-state-read.jdn",
                r#"module test; fn main() -> Int32 { region frame { let output: Buffer<UInt8> = frame.allocate(32); var text_length: [UIntSize; 1] = [0usize]; var data_length: [UIntSize; 1] = [0usize]; var signed_output: [Int64; 1] = [0 as Int64]; var unsigned_output: [UInt64; 1] = [0u64]; var float_output: [Float64; 1] = [0.0f64]; var bool_output: [Bool; 1] = [false]; app_state_clear(); app_state_set_int("minutes", 42 as Int64); app_state_set_uint("total", 7u64); app_state_set_float("rate", 1.25f64); app_state_set_bool("done", true); app_state_set_text("name", "Focus"); let count: Int32 = app_state_count(); let first_kind: Int32 = app_state_type_at(0); let exists: Bool = app_state_exists("name"); let removed: Bool = app_state_remove("missing"); let key: UIntSize = app_state_read_key(0, output); let text: UIntSize = app_state_read_text("name", output); app_state_read_text_exact("name", output, text_length); app_state_read_int("minutes", signed_output); app_state_read_uint("total", unsigned_output); app_state_read_float("rate", float_output); app_state_read_bool("done", bool_output); let signed: Int64 = app_state_get_int("minutes"); let saved: Bool = app_state_save("target/state.json"); let atomic: Bool = app_state_save_atomic("target/state.tmp", "target/state.json"); let loaded: Bool = app_state_load("target/state.json"); let data_saved: Bool = app_data_save("target/data.jdn"); let data_exact: Bool = app_data_write_exact(output, data_length); let data_atomic: Bool = app_data_save_atomic("target/data.tmp", "target/data.jdn"); let data_loaded: Bool = app_data_load("target/data.jdn"); let data_loaded_exact: Bool = app_data_load_exact(output, data_length[0]); let begun: Bool = app_state_tx_begin(); let committed: Bool = app_state_tx_commit(); let rolled_back: Bool = app_state_tx_rollback(); let data_begun: Bool = app_data_tx_begin(); let data_committed: Bool = app_data_tx_commit(); let data_rolled_back: Bool = app_data_tx_rollback(); if !exists { return 1 } return (text + key + (signed as UIntSize) + (count as UIntSize) + (first_kind as UIntSize)) as Int32 } }"#,
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("app state buffers must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        for name in [
            "app_state_clear",
            "app_state_set_int",
            "app_state_set_text",
            "app_state_count",
            "app_state_type_at",
            "app_state_exists",
            "app_state_remove",
            "app_state_read_key",
            "app_state_read_text",
            "app_state_read_text_exact",
            "app_state_read_int",
            "app_state_read_uint",
            "app_state_read_float",
            "app_state_read_bool",
            "app_state_save",
            "app_state_save_atomic",
            "app_state_load",
            "app_data_save",
            "app_data_write_exact",
            "app_data_save_atomic",
            "app_data_load",
            "app_data_load_exact",
            "app_state_tx_begin",
            "app_state_tx_commit",
            "app_state_tx_rollback",
            "app_data_tx_begin",
            "app_data_tx_commit",
            "app_data_tx_rollback",
        ] {
            assert!(text.contains(&format!("\"{name}\"")), "{text}");
        }
        assert_eq!(
            borrowed_runtime_argument_capability("app_state_read_text", 1),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_state_read_text_exact", 1),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_state_read_text_exact", 2),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_state_write_json_exact", 0),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_state_write_json_exact", 1),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_state_load_json_exact", 0),
            Some(Capability::Read)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_data_write_exact", 0),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_data_write_exact", 1),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_data_load_exact", 0),
            Some(Capability::Read)
        );
        assert!(text.contains("aggregate "), "{text}");
    }

    #[test]
    fn lowers_app_state_float_calls() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "app-state-float.jdn",
                "module test; fn main() -> Int32 { var output: [UInt8; 8] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; app_state_clear(); let stored: Bool = app_state_set_float(\"rate\", 1.25f64); let value: Float64 = app_state_get_float(\"rate\"); let length: UIntSize = format_float(value, output); if !stored { return 1 } if length != 8usize { return 2 } return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);
        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("app state float calls must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("\"app_state_set_float\""), "{text}");
        assert!(text.contains("\"app_state_get_float\""), "{text}");
    }

    #[test]
    fn lowers_app_list_text_output_to_borrowed_slice() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "app-list-read.jdn",
                r#"module test; fn main() -> Int32 { region frame { let output: Buffer<UInt8> = frame.allocate(32); var query: [UInt8; 2] = [70u8, 79u8]; app_list_clear(0); let pushed_bytes: Bool = app_list_push_text_bytes(0, query, 2usize); app_list_push_text(0, "Focus"); let count: Int32 = app_list_count(0); let sorted: Bool = app_list_sort_text(0, false); let found: Int32 = app_list_find_text(0, "Focus", 0); let filtered: Bool = app_list_filter_text(0, 1, "Focus"); let filtered_ex: Bool = app_list_filter_text_ex(0, 1, "FO", 5); let filtered_bytes: Bool = app_list_filter_text_ex_bytes(0, 1, query, 2usize, 5); let text: UIntSize = app_list_read_text(0, 0, output); let exported: UIntSize = app_list_export_csv(0, output); let updated: Bool = app_list_set_text(0, 0, "Done"); let updated_bytes: Bool = app_list_set_text_bytes(0, 0, query, 2usize); let removed: Bool = app_list_remove(0, 0); let saved: Bool = app_list_save(0, "target/list.json"); let atomic: Bool = app_list_save_atomic(0, "target/list.tmp", "target/list.json"); let loaded: Bool = app_list_load(0, "target/list.json"); if !pushed_bytes { return 1 } if !sorted { return 2 } if !filtered { return 3 } if !filtered_ex { return 4 } if !filtered_bytes { return 5 } if !updated { return 6 } if !updated_bytes { return 7 } return count + found + (text as Int32) + (exported as Int32) } }"#,
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("app list buffers must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        for name in [
            "app_list_clear",
            "app_list_push_text",
            "app_list_push_text_bytes",
            "app_list_count",
            "app_list_sort_text",
            "app_list_find_text",
            "app_list_filter_text",
            "app_list_filter_text_ex",
            "app_list_filter_text_ex_bytes",
            "app_list_read_text",
            "app_list_export_csv",
            "app_list_set_text",
            "app_list_set_text_bytes",
            "app_list_remove",
            "app_list_save",
            "app_list_save_atomic",
            "app_list_load",
        ] {
            assert!(text.contains(&format!("\"{name}\"")), "{text}");
        }
        assert_eq!(
            borrowed_runtime_argument_capability("app_list_read_text", 2),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_list_filter_text_ex_bytes", 2),
            Some(Capability::Read)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_list_push_text_bytes", 1),
            Some(Capability::Read)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_list_set_text_bytes", 2),
            Some(Capability::Read)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_list_export_csv", 1),
            Some(Capability::Write)
        );
        assert!(text.contains("aggregate "), "{text}");
    }

    #[test]
    fn lowers_app_data_journal_frame_read_outputs_to_borrowed_slices() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "app-data-journal-frame-read.jdn",
                r#"module test; fn main() -> Int32 { var output: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; var length: [UIntSize; 1] = [0usize]; let ok: Bool = app_data_journal_read_frame_exact_durable("target/journal", "target/journal.lock", 0usize, output, length); if ok { return length[0] as Int32 } return 0 }"#,
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);
        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("journal frame read buffers must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(
            text.contains("\"app_data_journal_read_frame_exact_durable\""),
            "{text}"
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_data_journal_read_frame_exact_durable", 3),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_data_journal_read_frame_exact_durable", 4),
            Some(Capability::Write)
        );
    }

    #[test]
    fn lowers_app_data_journal_latest_frame_outputs_to_borrowed_slices() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "app-data-journal-latest-frame-read.jdn",
                r#"module test; fn main() -> Int32 { var output: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; var length: [UIntSize; 1] = [0usize]; let ok: Bool = app_data_journal_read_latest_frame_exact_durable("target/journal", "target/journal.lock", output, length); if ok { return length[0] as Int32 } return 0 }"#,
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);
        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("latest journal frame read buffers must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(
            text.contains("\"app_data_journal_read_latest_frame_exact_durable\""),
            "{text}"
        );
        assert_eq!(
            borrowed_runtime_argument_capability(
                "app_data_journal_read_latest_frame_exact_durable",
                2
            ),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability(
                "app_data_journal_read_latest_frame_exact_durable",
                3
            ),
            Some(Capability::Write)
        );
    }

    #[test]
    fn lowers_app_data_journal_stats_output_to_borrowed_slice() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "app-data-journal-stats.jdn",
                r#"module test; fn main() -> Int32 { var stats: [UIntSize; 2] = [0usize, 0usize]; let ok: Bool = app_data_journal_stats_durable("target/journal", "target/journal.lock", stats); if ok { return stats[0] as Int32 } return 0 }"#,
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);
        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("journal stats output must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(
            text.contains("\"app_data_journal_stats_durable\""),
            "{text}"
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_data_journal_stats_durable", 2),
            Some(Capability::Write)
        );
    }

    #[test]
    fn lowers_app_data_journal_maintenance_plan_output_to_borrowed_slice() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "app-data-journal-maintenance-plan.jdn",
                r#"module test; fn main() -> Int32 { var plan: [UIntSize; 3] = [0usize, 0usize, 0usize]; let ok: Bool = app_data_journal_maintenance_plan_durable("target/journal", "target/journal.lock", 4096usize, 8usize, plan); if ok { return plan[0] as Int32 } return 0 }"#,
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);
        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("journal maintenance plan output must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(
            text.contains("\"app_data_journal_maintenance_plan_durable\""),
            "{text}"
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_data_journal_maintenance_plan_durable", 4),
            Some(Capability::Write)
        );
    }

    #[test]
    fn lowers_app_table_cell_output_to_borrowed_slice() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "app-table-read.jdn",
                r#"module test; fn main() -> Int32 { region frame { let output: Buffer<UInt8> = frame.allocate(32); let lengths: Buffer<UIntSize> = frame.allocate(1); app_table_clear(0); app_table_clear(1); app_table_set_column_type(0, 0, 1); let kind: Int32 = app_table_column_type(0, 0); let valid: Bool = app_table_validate(0); if !valid { return 1 } app_table_append_row(0); app_table_set_cell(0, 0, 0, "42"); let bytes_set: Bool = app_table_set_cell_bytes(0, 0, 0, output); let bytes_set_ex: Bool = app_table_set_cell_bytes_ex(0, 0, 0, output, 2usize); let set_int: Bool = app_table_set_int(0, 0, 0, 42 as Int64); let set_uint: Bool = app_table_set_uint(0, 0, 0, 7u64); let set_bool: Bool = app_table_set_bool(0, 0, 0, true); let rows: Int32 = app_table_row_count(0); let sorted: Bool = app_table_sort_text(0, 0, false); let found: Int32 = app_table_find_text(0, 0, "42", 0); let indexed: Bool = app_table_index_build(0, 0); let index_valid: Bool = app_table_index_is_valid(0, 0); let indexed_found: Int32 = app_table_index_find_text(0, 0, "42"); let index_cleared: Bool = app_table_index_clear(0); let filtered: Bool = app_table_filter_text(0, 1, 0, "42"); let filtered_ex: Bool = app_table_filter_text_ex(0, 1, 0, "2", 1); let filtered_bytes: Bool = app_table_filter_text_ex_bytes(0, 1, 0, output, 2usize, 1); let text: UIntSize = app_table_read_cell(0, 0, 0, output); let exact: Bool = app_table_read_cell_exact(0, 0, 0, output, lengths); let parsed_int: Int64 = app_table_read_int(0, 0, 0); let parsed_uint: UInt64 = app_table_read_uint(0, 0, 0); let parsed_bool: Bool = app_table_read_bool(0, 0, 0); let removed: Bool = app_table_remove_row(0, 0); let schema_saved: Bool = app_table_save_schema(0, "target/schema.json"); let schema_atomic: Bool = app_table_save_schema_atomic(0, "target/schema.tmp", "target/schema.json"); let schema_loaded: Bool = app_table_load_schema(0, "target/schema.json"); let saved: Bool = app_table_save(0, "target/table.json"); let atomic: Bool = app_table_save_atomic(0, "target/table.tmp", "target/table.json"); let loaded: Bool = app_table_load(0, "target/table.json"); if !bytes_set { return 9 } if !bytes_set_ex { return 11 } if !set_int { return 2 } if !set_uint { return 3 } if !set_bool { return 4 } if !indexed { return 12 } if !index_valid { return 12 } if !index_cleared { return 12 } if indexed_found != 0 { return 12 } if !filtered_bytes { return 10 } if !schema_saved { return 5 } if !schema_atomic { return 6 } if !schema_loaded { return 7 } if !exact { return 13 } if parsed_bool { return 8 } return rows + (text as Int32) + found + (parsed_int as Int32) + (parsed_uint as Int32) + kind } }"#,
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);
        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("app table buffers must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        for name in [
            "app_table_clear",
            "app_table_set_column_type",
            "app_table_column_type",
            "app_table_validate",
            "app_table_append_row",
            "app_table_set_cell",
            "app_table_set_cell_bytes",
            "app_table_set_cell_bytes_ex",
            "app_table_set_int",
            "app_table_set_uint",
            "app_table_set_bool",
            "app_table_row_count",
            "app_table_read_cell",
            "app_table_read_cell_exact",
            "app_table_read_int",
            "app_table_read_uint",
            "app_table_read_bool",
            "app_table_remove_row",
            "app_table_sort_text",
            "app_table_find_text",
            "app_table_index_build",
            "app_table_index_clear",
            "app_table_index_find_text",
            "app_table_index_is_valid",
            "app_table_filter_text",
            "app_table_filter_text_ex",
            "app_table_filter_text_ex_bytes",
            "app_table_save",
            "app_table_save_schema",
            "app_table_save_schema_atomic",
            "app_table_save_atomic",
            "app_table_load",
            "app_table_load_schema",
        ] {
            assert!(text.contains(&format!("\"{name}\"")), "{text}");
        }
        assert_eq!(
            borrowed_runtime_argument_capability("app_table_read_cell", 3),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_table_read_cell_exact", 3),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_table_read_cell_exact", 4),
            Some(Capability::Write)
        );
        assert!(text.contains("aggregate "), "{text}");
        assert_eq!(
            borrowed_runtime_argument_capability("app_table_set_cell_bytes", 3),
            Some(Capability::Read)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_table_set_cell_bytes_ex", 3),
            Some(Capability::Read)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_table_filter_text_ex_bytes", 3),
            Some(Capability::Read)
        );
    }

    #[test]
    fn lowers_app_table_csv_export_to_borrowed_slice() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "app-table-export.jdn",
                "module test; fn main() -> Int32 { region frame { let output: Buffer<UInt8> = frame.allocate(64); let length: UIntSize = app_table_export_csv(0, output); return length as Int32 } }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);
        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("app table CSV export must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("\"app_table_export_csv\""), "{text}");
        assert_eq!(
            borrowed_runtime_argument_capability("app_table_export_csv", 1),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_table_index_collect_int_range", 4),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_table_index_collect_uint_range", 4),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_table_index_collect_float_range", 4),
            Some(Capability::Write)
        );
        assert!(text.contains("aggregate "), "{text}");
    }

    #[test]
    fn lowers_app_table_csv_import_to_borrowed_slice() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "app-table-import.jdn",
                "module test; fn main() -> Int32 { region frame { let input: Buffer<UInt8> = frame.allocate(64); let imported: Bool = app_table_import_csv(0, input, 64usize); if imported { return 1 } return 0 } }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);
        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("app table CSV import must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("\"app_table_import_csv\""), "{text}");
        assert_eq!(
            borrowed_runtime_argument_capability("app_table_import_csv", 1),
            Some(Capability::Read)
        );
        assert!(text.contains("aggregate "), "{text}");
    }

    #[test]
    fn lowers_app_table_transaction_calls() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "app-table-transaction.jdn",
                "module test; fn main() -> Int32 { let begun: Bool = app_table_tx_begin(0); let committed: Bool = app_table_tx_commit(); let rolled_back: Bool = app_table_tx_rollback(); let all_begun: Bool = app_table_tx_begin_all(); let all_committed: Bool = app_table_tx_commit_all(); let all_rolled_back: Bool = app_table_tx_rollback_all(); if begun { return 1 } if committed { return 2 } if rolled_back { return 3 } if all_begun { return 4 } if all_committed { return 5 } if all_rolled_back { return 6 } return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);
        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("table transaction calls must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        for name in [
            "app_table_tx_begin",
            "app_table_tx_commit",
            "app_table_tx_rollback",
            "app_table_tx_begin_all",
            "app_table_tx_commit_all",
            "app_table_tx_rollback_all",
        ] {
            assert!(text.contains(&format!("\"{name}\"")), "{text}");
        }
    }

    #[test]
    fn lowers_named_app_table_schema_calls() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "app-table-record.jdn",
                "module test; fn main() -> Int32 { region frame { let output: Buffer<UInt8> = frame.allocate(64); let lengths: Buffer<UIntSize> = frame.allocate(1); let named: Bool = app_table_set_column_name(0, 0, \"id\"); let length: UIntSize = app_table_read_column_name(0, 0, output); let exact: Bool = app_table_read_column_name_exact(0, 0, output, lengths); let found: Int32 = app_table_find_column(0, \"id\"); if !named { return 1 } if !exact { return 2 } return (length as Int32) + found } }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);
        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("named table schema calls must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        for name in [
            "app_table_set_column_name",
            "app_table_read_column_name",
            "app_table_read_column_name_exact",
            "app_table_find_column",
        ] {
            assert!(text.contains(&format!("\"{name}\"")), "{text}");
        }
        assert_eq!(
            borrowed_runtime_argument_capability("app_table_read_column_name", 2),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_table_read_column_name_exact", 2),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_table_read_column_name_exact", 3),
            Some(Capability::Write)
        );
    }

    #[test]
    fn lowers_named_app_table_field_calls() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "app-table-named-fields.jdn",
                "module test; fn main() -> Int32 { region frame { let output: Buffer<UInt8> = frame.allocate(64); let lengths: Buffer<UIntSize> = frame.allocate(1); let text_set: Bool = app_table_set_named_cell(0, 0, \"title\", \"Focus\"); let int_set: Bool = app_table_set_named_int(0, 0, \"id\", 42 as Int64); let _copied: UIntSize = app_table_read_named_cell(0, 0, \"title\", output); let exact: Bool = app_table_read_named_cell_exact(0, 0, \"title\", output, lengths); let _value: Int64 = app_table_read_named_int(0, 0, \"id\"); if !text_set { return 1 } if !int_set { return 1 } if !exact { return 2 } return 0 } }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);
        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("named table field calls must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        for name in [
            "app_table_set_named_cell",
            "app_table_set_named_int",
            "app_table_read_named_cell",
            "app_table_read_named_cell_exact",
            "app_table_read_named_int",
        ] {
            assert!(text.contains(&format!("\"{name}\"")), "{text}");
        }
        assert_eq!(
            borrowed_runtime_argument_capability("app_table_read_named_cell", 3),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_table_read_named_cell_exact", 3),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("app_table_read_named_cell_exact", 4),
            Some(Capability::Write)
        );
    }

    #[test]
    fn lowers_app_table_float_calls() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "app-table-float.jdn",
                r#"module test; fn main() -> Int32 { let set: Bool = app_table_set_float(0, 0, 0, 1.25f64); let _value: Float64 = app_table_read_float(0, 0, 0); let named_set: Bool = app_table_set_named_float(0, 0, "rate", 2.5f64); let _named: Float64 = app_table_read_named_float(0, 0, "rate"); if !set { return 1 } if !named_set { return 1 } return 0 }"#,
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);
        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("float app table calls must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        for name in [
            "app_table_set_float",
            "app_table_read_float",
            "app_table_set_named_float",
            "app_table_read_named_float",
        ] {
            assert!(text.contains(&format!("\"{name}\"")), "{text}");
        }
    }

    #[test]
    fn lowers_file_read_and_write_arrays_to_borrowed_slices() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "file-array.jdn",
                "module test; fn main() -> Int32 { var payload: [UInt8; 4] = [1u8, 2u8, 3u8, 4u8]; file_write(\"target/test.tmp\", payload); file_write_at(\"target/test.tmp\", 1usize, payload); var digits: [UInt8; 20] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let number: UIntSize = format_int(1 as Int64, digits); let unsigned: UIntSize = format_uint(2u64, digits); let float: UIntSize = format_float(1.5f64, digits); file_append(\"target/test.tmp\", digits, number); file_append_text(\"target/test.tmp\", \"row\"); file_replace_atomic(\"target/test.tmp\", \"target/test.bin\"); var output: [UInt8; 4] = [0u8, 0u8, 0u8, 0u8]; var kinds: [UInt8; 4] = [0u8, 0u8, 0u8, 0u8]; file_read(\"target/test.bin\", output); file_read_at(\"target/test.bin\", 0usize, output); file_read_text(\"target/test.bin\", output); directory_list(\"target\", output); directory_list_ex(\"target\", output, kinds); process_arg_count(); process_arg_read(0usize, output); return float as Int32 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("file byte buffers must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("fn import @f"), "{text}");
        assert!(text.contains("\"file_write\""), "{text}");
        assert!(text.contains("\"file_write_at\""), "{text}");
        assert!(text.contains("\"process_arg_count\""), "{text}");
        assert!(text.contains("\"process_arg_read\""), "{text}");
        assert!(text.contains("\"file_read\""), "{text}");
        assert!(text.contains("\"file_read_at\""), "{text}");
        assert!(text.contains("\"file_read_text\""), "{text}");
        assert!(text.contains("\"directory_list\""), "{text}");
        assert!(text.contains("\"directory_list_ex\""), "{text}");
        assert!(text.contains("\"file_replace_atomic\""), "{text}");
        assert!(text.contains("\"file_append_text\""), "{text}");
        assert!(text.contains("\"file_append\""), "{text}");
        assert!(text.contains("\"format_int\""), "{text}");
        assert!(text.contains("\"format_uint\""), "{text}");
        assert!(text.contains("\"format_float\""), "{text}");
        assert!(text.contains("aggregate "), "{text}");
    }

    #[test]
    fn lowers_standard_io_arrays_to_borrowed_slices() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "std-io.jdn",
                "module test; fn main() -> Int32 { var payload: [UInt8; 4] = [0u8, 0u8, 0u8, 0u8]; let received: UIntSize = stdin_read(payload); let sent: UIntSize = stdout_write(payload); let error_sent: UIntSize = stderr_write(payload); return (received + sent + error_sent) as Int32 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);
        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("standard IO byte buffers must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        for name in ["stdin_read", "stdout_write", "stderr_write"] {
            assert!(text.contains(&format!("\"{name}\"")), "{text}");
        }
    }

    #[test]
    fn lowers_csv_and_json_escape_arrays_to_borrowed_slices() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "escaping-array.jdn",
                "module test; fn main() -> Int32 { var output: [UInt8; 32] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let csv: UIntSize = csv_escape(\"a,b\", output); let json: UIntSize = json_escape(\"a\\\"b\", output); return (csv + json) as Int32 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("escaping byte buffers must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("\"csv_escape\""), "{text}");
        assert!(text.contains("\"json_escape\""), "{text}");
        assert!(text.contains("aggregate "), "{text}");
    }

    #[test]
    fn lowers_json_object_field_string_output_to_borrowed_slice() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "json-member-array.jdn",
                "module test; fn main() -> Int32 { var output: [UInt8; 32] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let member: UIntSize = json_object_field_string(\"name\", \"Ada\", output); return member as Int32 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("JSON member output must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("\"json_object_field_string\""), "{text}");
        assert!(text.contains("aggregate "), "{text}");
    }

    #[test]
    fn lowers_json_object_field_numeric_outputs_to_borrowed_slices() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "json-numeric-members.jdn",
                "module test; fn main() -> Int32 { var output: [UInt8; 32] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let signed: UIntSize = json_object_field_int(\"signed\", 42 as Int64, output); let unsigned: UIntSize = json_object_field_uint(\"unsigned\", 7u64, output); let decimal: UIntSize = json_object_field_float(\"decimal\", 1.5f64, output); let flag: UIntSize = json_object_field_bool(\"flag\", true, output); return (signed + unsigned + decimal + flag) as Int32 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("JSON numeric member output must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        for name in [
            "json_object_field_int",
            "json_object_field_uint",
            "json_object_field_float",
            "json_object_field_bool",
        ] {
            assert!(text.contains(&format!("\"{name}\"")), "{text}");
        }
        assert!(text.contains("aggregate "), "{text}");
    }

    #[test]
    fn lowers_json_array_inputs_and_outputs_to_borrowed_slices() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "json-arrays.jdn",
                "module test; fn main() -> Int32 { var signed: [Int64; 2] = [1 as Int64, 2 as Int64]; var unsigned: [UInt64; 2] = [3u64, 4u64]; var decimals: [Float64; 2] = [1.0f64, 2.0f64]; var flags: [Bool; 2] = [true, false]; var output: [UInt8; 32] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let a: UIntSize = json_array_int(signed, output); let b: UIntSize = json_array_uint(unsigned, output); let c: UIntSize = json_array_float(decimals, output); let d: UIntSize = json_array_bool(flags, output); return (a + b + c + d) as Int32 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("JSON array inputs and outputs must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        for name in [
            "json_array_int",
            "json_array_uint",
            "json_array_float",
            "json_array_bool",
        ] {
            assert!(text.contains(&format!("\"{name}\"")), "{text}");
        }
        assert!(text.contains("aggregate "), "{text}");
    }

    #[test]
    fn lowers_tcp_send_and_receive_with_directional_borrows() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "tcp.jdn",
                "module test; fn main() -> Int32 { let token: UIntSize = net_tcp_listen(38123u16); let named: UIntSize = net_tcp_connect_dns(\"localhost\", 38123u16); let timed: Bool = net_socket_set_timeout(token, 1000u32); var input: [UInt8; 4] = [1u8, 2u8, 3u8, 4u8]; var output: [UInt8; 4] = [0u8, 0u8, 0u8, 0u8]; var query_length: [UIntSize; 1] = [0usize]; let sent: UIntSize = net_tcp_send(token, input); let received: UIntSize = net_tcp_receive(token, output); let response: UIntSize = http_response_write(200u16, \"text/plain\", input, output); let request: UIntSize = http_request_write(\"GET\", \"/hello\", \"localhost\", input, output); let prefix_request: UIntSize = http_request_write_prefix(\"POST\", \"/hello\", \"localhost\", input, 2usize, output); let complete: Bool = http_request_is_complete(input); let method: UIntSize = http_request_method(input, output); let target: UIntSize = http_request_target(input, output); let header: UIntSize = http_request_header(input, \"Host\", output); let body: UIntSize = http_request_body(input, output); let query: UIntSize = http_query_param(input, \"id\", output); let query_exact: Bool = http_query_param_exact(input, \"id\", output, query_length); let route: Bool = http_route_match(input, \"GET\", \"/\"); if query_exact { return query as Int32 } return (sent + received + named + response + request + prefix_request + method + target + header + body + query) as Int32 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("TCP send/receive must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("\"net_tcp_send\""), "{text}");
        assert_eq!(
            borrowed_runtime_argument_capability("net_tcp_send_prefix", 1),
            Some(Capability::Read)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("http_query_param", 0),
            Some(Capability::Read)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("http_query_param", 2),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("http_query_param_exact", 0),
            Some(Capability::Read)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("http_query_param_exact", 2),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("http_query_param_exact", 3),
            Some(Capability::Write)
        );
        assert!(text.contains("\"net_tcp_receive\""), "{text}");
        assert!(text.contains("\"net_tcp_connect_dns\""), "{text}");
        assert!(text.contains("\"http_response_write\""), "{text}");
        assert!(text.contains("\"http_request_write\""), "{text}");
        assert!(text.contains("\"http_request_write_prefix\""), "{text}");
        assert!(text.contains("\"http_request_is_complete\""), "{text}");
        assert!(text.contains("\"http_request_method\""), "{text}");
        assert!(text.contains("\"http_request_target\""), "{text}");
        assert!(text.contains("\"http_request_header\""), "{text}");
        assert!(text.contains("\"http_request_body\""), "{text}");
        assert!(text.contains("\"http_query_param\""), "{text}");
        assert!(text.contains("\"http_query_param_exact\""), "{text}");
        assert!(text.contains("\"http_route_match\""), "{text}");
        assert!(text.contains("\"net_socket_set_timeout\""), "{text}");
        assert!(text.contains("aggregate "), "{text}");
    }

    #[test]
    fn lowers_http_response_reader_with_directional_borrows() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "http-response-reader.jdn",
                "module test; fn main() -> Int32 { var input: [UInt8; 4] = [0u8, 0u8, 0u8, 0u8]; var output: [UInt8; 4] = [0u8, 0u8, 0u8, 0u8]; var length: [UIntSize; 1] = [0usize]; let status: UInt16 = http_response_status(input); let bounded_status: UInt16 = http_response_status_prefix(input, 2usize); let header: UIntSize = http_response_header(input, \"Content-Type\", output); let bounded_header: UIntSize = http_response_header_prefix(input, 2usize, \"Content-Type\", output); let header_exact: Bool = http_response_header_exact(input, \"Content-Type\", output, length); let body: UIntSize = http_response_body(input, output); let bounded_body: UIntSize = http_response_body_prefix(input, 2usize, output); let body_exact: Bool = http_response_body_exact(input, output, length); if header_exact { return length[0] as Int32 } if body_exact { return length[0] as Int32 } return (status + bounded_status) as Int32 + (header + bounded_header + body + bounded_body) as Int32 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);
        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("HTTP response reader must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        for name in [
            "http_response_status",
            "http_response_status_prefix",
            "http_response_header",
            "http_response_header_prefix",
            "http_response_header_exact",
            "http_response_body",
            "http_response_body_prefix",
            "http_response_body_exact",
        ] {
            assert!(text.contains(&format!("\"{name}\"")), "{text}");
        }
        assert_eq!(
            borrowed_runtime_argument_capability("http_response_status_prefix", 0),
            Some(Capability::Read)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("http_response_header_prefix", 3),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("http_response_header_exact", 0),
            Some(Capability::Read)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("http_response_header_exact", 2),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("http_response_header_exact", 3),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("http_response_body_prefix", 2),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("http_response_body_exact", 0),
            Some(Capability::Read)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("http_response_body_exact", 1),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("http_response_body_exact", 2),
            Some(Capability::Write)
        );
    }

    #[test]
    fn lowers_http_response_chunked_reader_with_directional_borrows() {
        let source_text = include_str!("../../../examples/http-response-chunked-exact.jdn");
        let mut sources = SourceManager::new();
        let id = sources
            .add("http-response-chunked-exact.jdn", source_text)
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);
        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("chunked HTTP response reader must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        assert!(
            jir.to_text()
                .contains("\"http_response_body_chunked_exact\"")
        );
        assert_eq!(
            borrowed_runtime_argument_capability("http_response_body_chunked_exact", 0),
            Some(Capability::Read)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("http_response_body_chunked_exact", 1),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("http_response_body_chunked_exact", 2),
            Some(Capability::Write)
        );
    }

    #[test]
    fn lowers_http_request_chunked_reader_with_directional_borrows() {
        let source_text = include_str!("../../../examples/http-request-chunked-exact.jdn");
        let mut sources = SourceManager::new();
        let id = sources
            .add("http-request-chunked-exact.jdn", source_text)
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);
        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("chunked HTTP request reader must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        assert!(
            jir.to_text()
                .contains("\"http_request_body_chunked_exact\"")
        );
        assert_eq!(
            borrowed_runtime_argument_capability("http_request_body_chunked_exact", 0),
            Some(Capability::Read)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("http_request_body_chunked_exact", 1),
            Some(Capability::Write)
        );
        assert_eq!(
            borrowed_runtime_argument_capability("http_request_body_chunked_exact", 2),
            Some(Capability::Write)
        );
    }

    #[test]
    fn lowers_http_request_chunked_frame_reader_with_read_borrow() {
        let source_text = include_str!("../../../examples/http-request-chunked-frame-prefix.jdn");
        let mut sources = SourceManager::new();
        let id = sources
            .add("http-request-chunked-frame-prefix.jdn", source_text)
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);
        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("chunked HTTP frame reader must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        assert!(
            jir.to_text()
                .contains("\"http_request_chunked_frame_length_prefix\"")
        );
        assert_eq!(
            borrowed_runtime_argument_capability("http_request_chunked_frame_length_prefix", 0),
            Some(Capability::Read)
        );
    }

    #[test]
    fn lowers_nonterminated_assignment_in_if_block_to_store() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "assignment-in-if.jdn",
                "module test; fn update(flag: Bool) { var count: Int32 = 0; if flag { count = count + 1 } }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("assignment in a unit if block must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("branch %v"));
        assert!(text.contains(" = add "));
        assert!(text.contains("store "));
    }

    #[test]
    fn lowers_compound_assignment_in_loop_to_binary_store() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "compound-assignment.jdn",
                "module test; fn update() { var count: Int32 = 3; while count > 0 { count -= 1 } }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("compound assignment must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains(" = sub "));
        assert!(text.contains("store "));
        assert!(text.contains("jump ^bb"));
    }

    #[test]
    fn lowers_indexed_compound_assignment_with_one_captured_index() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "indexed-compound-assignment.jdn",
                "module test; struct Agent { position: Int32 } fn update(agents: write Slice<Agent>, index: UIntSize) { agents[index].position += 1 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("indexed compound assignment must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("bounds_check "));
        assert!(text.contains(" = add "));
        assert!(text.contains("store "));
    }

    #[test]
    fn lowers_local_and_deduplicated_external_calls() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "calls.jdn",
                "module test; fn twice(value: Int32) -> Int32 { return value + value } fn run() { assert_eq(twice(2), 4); assert_eq(twice(3), 6) }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("direct calls must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert_eq!(jir.functions.len(), 3);
        assert!(text.contains("call @f0("));
        assert_eq!(text.matches("fn import @f2 \"assert_eq\"").count(), 1);
        assert_eq!(text.matches("call @f2(").count(), 2);
    }

    #[test]
    fn lowers_multiple_generic_buffer_types_to_one_native_import_each() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "buffer-multi-type.jdn",
                "module test; fn run() -> Int32 { let ints: Result<Buffer<Int32>, Int32> = buffer_create(1usize); let floats: Result<Buffer<Float32>, Int32> = buffer_create(1usize); match ints { Ok(values) => { if !buffer_append(values, 1) { return 1 } } Error(status) => { return status } } match floats { Ok(values) => { if !buffer_append(values, 1.5f32) { return 2 } } Error(status) => { return status } } return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("multi-type Buffer calls must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert_eq!(text.matches("fn import @f1 \"buffer_create\"").count(), 1);
        assert_eq!(text.matches("fn import @f2 \"buffer_append\"").count(), 1);
    }

    #[test]
    fn lowers_nested_owning_buffer_drop_glue_and_move_calls() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "buffer-nested-owning.jdn",
                "module test; fn run() -> Int32 { let outer: Result<Buffer<Buffer<Int32>>, Int32> = buffer_create(0usize); let inner: Result<Buffer<Int32>, Int32> = buffer_create(0usize); var result_code: Int32 = 0; match outer { Ok(values) => { match inner { Ok(item) => { if !buffer_append(item, 7) { result_code = 1 } if !buffer_append(values, item) { result_code = 2 } } Error(status) => { result_code = status } } } Error(status) => { result_code = status } } return result_code }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("nested owning Buffer must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("nested_buffer_drop "));
        assert_eq!(text.matches("fn import @f1 \"buffer_create\"").count(), 1);
        assert!(text.contains("\"buffer_append_move\""), "{text}");
    }

    #[test]
    fn lowers_inline_owning_terminator_arguments_with_cleanup_edges() {
        let mut sources = SourceManager::new();
        let source_text = include_str!("../../../examples/stdlib-buffer-owning-terminators.jdn");
        let id = sources
            .add("buffer-owning-terminators.jdn", source_text)
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("inline owning terminators must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("fn internal @f0 \"append_return\""), "{text}");
        assert!(text.contains("fn internal @f1 \"append_loop\""), "{text}");
        assert!(text.contains("\"buffer_append_move\""), "{text}");
        assert!(text.matches("owning_carrier_drop ").count() >= 2, "{text}");
        let append_write = jir
            .functions
            .iter()
            .find(|function| function.name == "append_write")
            .expect("write Buffer fixture function");
        let write_parameter = append_write
            .parameters
            .first()
            .expect("write Buffer parameter");
        assert!(
            matches!(jir.types[write_parameter.ty.index()], Type::Pointer { .. }),
            "write Buffer user parameter must use the mutable descriptor pointer ABI: {jir:?}"
        );
    }

    #[test]
    fn lowers_owning_record_buffer_fields_with_direct_and_nested_drop_glue() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "buffer-owning-record.jdn",
                "module test; @repr(C) struct Entry { values: Buffer<Int32>, id: Int32 } fn standalone(inner: Buffer<Int32>) { let spare: Entry = Entry { values: inner, id: 5 }; } fn run() -> Int32 { let outer: Result<Buffer<Entry>, Int32> = buffer_create(0usize); let inner: Result<Buffer<Int32>, Int32> = buffer_create(0usize); let second_inner: Result<Buffer<Int32>, Int32> = buffer_create(0usize); var result_code: Int32 = 0; match outer { Ok(values) => { match inner { Ok(item_values) => { if !buffer_append(item_values, 7) { result_code = 1 } let item: Entry = Entry { values: item_values, id: 42 }; if !buffer_append(values, item) { result_code = 2 } if buffer_length(values) != 1usize { result_code = 3 } match second_inner { Ok(second_values) => { if !buffer_append(second_values, 9) { result_code = 4 } let second: Entry = Entry { values: second_values, id: 7 }; if !buffer_insert(values, 0usize, second) { result_code = 5 } if buffer_length(values) != 2usize { result_code = 6 } } Error(status) => { result_code = status } } } Error(status) => { result_code = status } } } Error(status) => { result_code = status } } return result_code }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("owning record Buffer must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("record_buffer_fields_drop "), "{text}");
        assert!(text.contains("record_owning_fields_drop "), "{text}");
        assert!(
            text.contains("\"buffer_append_move\""),
            "owning append must use the rollback-safe move runtime: {text}"
        );
        assert!(
            text.contains("\"buffer_insert_move_from\""),
            "owning insert must use the rollback-safe move runtime: {text}"
        );
    }

    #[test]
    fn lowers_record_option_result_buffer_fields_with_tagged_drop_glue() {
        let mut sources = SourceManager::new();
        let source_text = include_str!("../../../examples/stdlib-buffer-owning-record-carrier.jdn");
        let id = sources
            .add("buffer-owning-record-carrier.jdn", source_text)
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("record carrier fields must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("record_buffer_fields_drop "), "{text}");
        assert!(text.contains("variant 1, offset 16"), "{text}");
        assert!(text.contains("variant 1, offset 48"), "{text}");
        assert!(text.contains("variant 0, offset 80"), "{text}");
        assert!(text.contains("variant 1, offset 80"), "{text}");
    }

    #[test]
    fn lowers_inline_nested_owning_record_fields_to_flat_drop_offsets() {
        let mut sources = SourceManager::new();
        let source_text = include_str!("../../../examples/stdlib-buffer-owning-nested-record.jdn");
        let id = sources
            .add("buffer-owning-nested-record.jdn", source_text)
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("inline nested owning record must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("record_buffer_fields_drop "), "{text}");
        assert!(text.contains("offset 8"), "{text}");
    }

    #[test]
    fn lowers_nested_buffer_owning_record_chain_with_path_aware_drop_glue() {
        let mut sources = SourceManager::new();
        let source_text =
            include_str!("../../../examples/stdlib-buffer-owning-nested-buffer-record.jdn");
        let id = sources
            .add("buffer-owning-nested-buffer-record.jdn", source_text)
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("nested owning record chain must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(
            text.contains("recursive_record_buffer_fields_drop "),
            "{text}"
        );
        assert!(text.contains("offset 8"), "{text}");
    }

    #[test]
    fn lowers_nested_buffer_resize_move_with_recursive_layout_arguments() {
        let mut sources = SourceManager::new();
        let source_text = include_str!("../../../examples/stdlib-buffer-nested-resize-move.jdn");
        let id = sources
            .add("buffer-nested-resize-move.jdn", source_text)
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("nested resize move must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("buffer_resize_move_status"));
        assert!(text.contains("nested_buffer_drop "));
    }

    #[test]
    fn lowers_nested_buffer_clear_move_with_recursive_layout_arguments() {
        let mut sources = SourceManager::new();
        let source_text = include_str!("../../../examples/stdlib-buffer-nested-clear-move.jdn");
        let id = sources
            .add("buffer-nested-clear-move.jdn", source_text)
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("nested clear move must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("buffer_clear_move_status"), "{text}");
        assert!(text.contains("nested_buffer_drop "), "{text}");
    }

    #[test]
    fn lowers_nested_record_buffer_resize_move_with_field_table() {
        let mut sources = SourceManager::new();
        let source_text =
            include_str!("../../../examples/stdlib-buffer-nested-record-resize-move.jdn");
        let id = sources
            .add("buffer-nested-record-resize-move.jdn", source_text)
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("nested record resize move must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("buffer_resize_move_record_fields "), "{text}");
        assert!(
            text.contains("recursive_record_buffer_fields_drop "),
            "{text}"
        );
        assert!(text.contains("offset 8"), "{text}");
    }

    #[test]
    fn lowers_nested_record_buffer_clear_move_with_field_table() {
        let mut sources = SourceManager::new();
        let source_text =
            include_str!("../../../examples/stdlib-buffer-nested-record-clear-move.jdn");
        let id = sources
            .add("buffer-nested-record-clear-move.jdn", source_text)
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("nested record clear move must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("buffer_resize_move_record_fields "), "{text}");
        assert!(
            text.contains("recursive_record_buffer_fields_drop "),
            "{text}"
        );
        assert!(text.contains("offset 8"), "{text}");
    }

    #[test]
    fn lowers_direct_record_buffer_resize_and_clear_move_with_field_table() {
        let mut sources = SourceManager::new();
        let source_text =
            include_str!("../../../examples/stdlib-buffer-record-resize-clear-move.jdn");
        let id = sources
            .add("buffer-record-resize-clear-move.jdn", source_text)
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("direct record resize/clear move must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(
            text.matches("buffer_resize_move_record_fields ").count() >= 3,
            "{text}"
        );
        assert!(text.contains("offset 8"), "{text}");
    }

    #[test]
    fn lowers_record_fixed_array_owning_fields_to_multiple_offsets() {
        let mut sources = SourceManager::new();
        let source_text = include_str!("../../../examples/stdlib-buffer-owning-record-array.jdn");
        let id = sources
            .add("buffer-owning-record-array.jdn", source_text)
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("record fixed array owning fields must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("record_buffer_fields_drop "), "{text}");
        assert!(text.contains("offset 8"), "{text}");
        assert!(text.contains("offset 32"), "{text}");
        assert!(text.contains("buffer_resize_move_record_fields "), "{text}");
    }

    #[test]
    fn lowers_direct_owning_array_element_to_multiple_offsets() {
        let mut sources = SourceManager::new();
        let source_text = include_str!("../../../examples/stdlib-buffer-owning-array.jdn");
        let id = sources
            .add("buffer-owning-array.jdn", source_text)
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("direct owning array element must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("record_buffer_fields_drop "), "{text}");
        assert!(text.contains("offset 0"), "{text}");
        assert!(text.contains("offset 24"), "{text}");
        assert!(text.contains("buffer_resize_move_record_fields "), "{text}");
        assert!(text.contains("buffer_remove_drop_record_fields "), "{text}");
    }

    #[test]
    fn lowers_owning_record_remove_drop_with_field_table() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "buffer-owning-record-remove-drop.jdn",
                "module test; @repr(C) struct Entry { values: Buffer<Int32>, id: Int32 } fn run(values: write Buffer<Entry>) -> Int32 { let removed: Bool = buffer_remove_drop(values, 0usize); let status: Int32 = buffer_remove_drop_status(values, 0usize); if !removed { return 1 } return status }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("owning record remove drop must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert_eq!(
            text.matches("buffer_remove_drop_record_fields ").count(),
            2,
            "{text}"
        );
        assert!(text.contains("offset 0"), "{text}");
    }

    #[test]
    fn lowers_nested_owning_record_remove_drop_with_field_table() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "buffer-owning-nested-record-remove-drop.jdn",
                "module test; @repr(C) struct Entry { values: Buffer<Int32>, id: Int32 } fn run(values: write Buffer<Buffer<Entry>>) -> Int32 { let removed: Bool = buffer_remove_drop(values, 0usize); let status: Int32 = buffer_remove_drop_status(values, 0usize); if !removed { return 1 } return status }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("nested owning record remove drop must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert_eq!(
            text.matches("buffer_remove_drop_nested_record_fields ")
                .count(),
            2,
            "{text}"
        );
        assert!(text.contains("depth 1"), "{text}");
        assert!(text.contains("offset 0"), "{text}");
    }

    #[test]
    fn lowers_recursive_nested_buffer_drop_glue() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "buffer-recursive-nested.jdn",
                "module test; fn run() -> Int32 { let outer_created: Result<Buffer<Buffer<Buffer<Int32>>>, Int32> = buffer_create(0usize); let middle_created: Result<Buffer<Buffer<Int32>>, Int32> = buffer_create(0usize); let leaf_created: Result<Buffer<Int32>, Int32> = buffer_create(0usize); var result_code: Int32 = 0; match outer_created { Ok(outer) => { match middle_created { Ok(middle) => { match leaf_created { Ok(leaf) => { if !buffer_append(leaf, 42) { result_code = 1 } if !buffer_append(middle, leaf) { result_code = 2 } if !buffer_append(outer, middle) { result_code = 3 } } Error(status) => { result_code = status } } } Error(status) => { result_code = status } } } Error(status) => { result_code = status } } return result_code }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("recursive nested Buffer must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        assert!(jir.to_text().contains("recursive_buffer_drop "));
    }

    #[test]
    fn lowers_buffer_option_carrier_drop_glue() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "buffer-option-carrier.jdn",
                "module test; fn run() -> Int32 { let outer_created: Result<Buffer<Option<Buffer<Int32>>>, Int32> = buffer_create(0usize); let inner_created: Result<Buffer<Int32>, Int32> = buffer_create(0usize); var result_code: Int32 = 0; match outer_created { Ok(outer) => { match inner_created { Ok(inner) => { let state: Option<Buffer<Int32>> = Some(inner); if !buffer_append(outer, state) { result_code = 1 } } Error(status) => { result_code = status } } } Error(status) => { result_code = status } } return result_code }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("Buffer<Option<Buffer<U>>> must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("carrier_buffer_drop "));
        assert_eq!(text.matches("fn import @f2 \"buffer_append\"").count(), 1);
    }

    #[test]
    fn lowers_buffer_owned_string_carrier_drop_glue() {
        let mut sources = SourceManager::new();
        let source_text =
            include_str!("../../../examples/stdlib-buffer-owning-string-carriers.jdn");
        let id = sources
            .add("buffer-owned-string-carrier.jdn", source_text)
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("OwnedString carrier Buffer must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("record_buffer_fields_drop "), "{text}");
        assert!(text.contains("depth 4294967295"), "{text}");
        assert!(!text.contains("carrier_buffer_drop "), "{text}");
    }

    #[test]
    fn lowers_buffer_owned_string_enum_field_table_drop_glue() {
        let mut sources = SourceManager::new();
        let source_text = include_str!("../../../examples/stdlib-buffer-owning-enum-string.jdn");
        let id = sources
            .add("buffer-owned-string-enum.jdn", source_text)
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("OwnedString enum Buffer must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("enum_carrier_fields_drop "), "{text}");
        // Inline owning enum arguments now use the rollback-safe move ABI and
        // therefore contribute their own branch-local drop records. Keep the
        // table-drop assertion structural rather than tying it to the exact
        // number of elaborated cleanup sites.
        assert!(text.matches("depth 4294967295").count() >= 2, "{text}");
        assert!(text.contains("buffer_append_move"), "{text}");
    }

    #[test]
    fn lowers_buffer_owned_string_enum_raw_move_calls() {
        let mut sources = SourceManager::new();
        let source_text =
            include_str!("../../../examples/stdlib-buffer-owning-enum-string-move-into.jdn");
        let id = sources
            .add("buffer-owned-string-enum-move-into.jdn", source_text)
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("OwnedString enum raw moves must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("buffer_remove_move_into"), "{text}");
        assert!(text.contains("buffer_insert_move_from"), "{text}");
        assert!(text.contains("buffer_pop_move_into"), "{text}");
        assert!(text.contains("enum_carrier_fields_drop "), "{text}");
    }

    #[test]
    fn lowers_buffer_owned_string_enum_mutation_field_table_calls() {
        let mut sources = SourceManager::new();
        let source_text =
            include_str!("../../../examples/stdlib-buffer-owning-enum-string-mutations.jdn");
        let id = sources
            .add("buffer-owned-string-enum-mutations.jdn", source_text)
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("OwnedString enum mutations must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("buffer_resize_move_record_fields "), "{text}");
        assert!(text.contains("buffer_remove_drop_record_fields "), "{text}");
        assert!(text.contains("depth 4294967295"), "{text}");
    }

    #[test]
    fn lowers_buffer_carrier_remove_clear_move_through_record_field_table() {
        let mut sources = SourceManager::new();
        let source_text =
            include_str!("../../../examples/stdlib-buffer-carrier-remove-clear-move.jdn");
        let id = sources
            .add("buffer-carrier-remove-clear-move.jdn", source_text)
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("carrier Buffer remove/clear/resize must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("buffer_remove_drop_record_fields "), "{text}");
        assert!(text.contains("buffer_resize_move_record_fields "), "{text}");
        assert!(text.contains("offset 8"), "{text}");
    }

    #[test]
    fn lowers_standalone_owning_carrier_drop_glue() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "standalone-option-buffer.jdn",
                "module test; fn run(value: Option<Buffer<Int32>>) { }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("standalone owning carrier must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        assert!(jir.to_text().contains("owning_carrier_drop "));
    }

    #[test]
    fn lowers_multi_owning_result_carrier_drop_glue() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "multi-result-buffer-carrier.jdn",
                "module test; fn run(value: Result<Buffer<Int32>, Buffer<Buffer<Int32>>>) { }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("multi-owning carrier must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("owning_carrier_drop "));
        assert!(text.contains("alt "));
    }

    #[test]
    fn lowers_nested_buffer_pop_with_outer_and_nested_layouts() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "buffer-nested-pop.jdn",
                "module test; fn run(values: write Buffer<Buffer<Int32>>) { buffer_pop(values) }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("nested buffer pop must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert_eq!(text.matches("buffer_pop").count(), 1);
        assert!(text.contains("call @f1("));
    }

    #[test]
    fn lowers_nested_buffer_remove_move_with_index_and_layouts() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "buffer-nested-remove-move.jdn",
                "module test; fn run(values: write Buffer<Buffer<Int32>>) { buffer_remove_move(values, 0usize) }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("nested buffer remove_move must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert_eq!(text.matches("buffer_remove_move").count(), 1);
        assert!(text.contains("call @f1("));
    }

    #[test]
    fn lowers_owning_record_remove_move_into_with_raw_output_pointer() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "buffer-owning-record-remove-move-into.jdn",
                "module test; @repr(C) struct Entry { values: Buffer<Int32>, id: Int32 } fn run(values: write Buffer<Entry>, output: write Entry) -> Int32 { let moved: Bool = buffer_remove_move_into(values, 0usize, output); let status: Int32 = buffer_remove_move_into_status(values, 99usize, output); if !moved { return 1 } return status }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("owning record remove_move_into must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert_eq!(text.matches("buffer_remove_move_into").count(), 2);
        assert!(text.contains("call @f1("));
    }

    #[test]
    fn lowers_generic_nested_owning_record_remove_move_into_with_field_table() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "buffer-generic-nested-owning-record.jdn",
                "module test; @repr(C) struct Frame<T> { payload: T, marker: Int32 } fn run(values: write Buffer<Frame<Buffer<OwnedString>>>, output: write Frame<Buffer<OwnedString>>) -> Int32 { let moved: Bool = buffer_remove_move_into(values, 0usize, output); if !moved { return 1 } return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("generic nested owning record remove_move_into must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("nominal_struct<"), "{text}");
        assert!(text.contains("buffer_remove_move_into"), "{text}");
    }

    #[test]
    fn lowers_generic_record_carrier_move_into_with_field_table() {
        let mut sources = SourceManager::new();
        let source_text =
            include_str!("../../../examples/stdlib-buffer-generic-record-carrier.jdn");
        let id = sources
            .add("buffer-generic-record-carrier.jdn", source_text)
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("generic carrier record move_into must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("nominal_struct<"), "{text}");
        assert!(text.contains("record_buffer_fields_drop "), "{text}");
        assert!(text.contains("buffer_remove_move_into"), "{text}");
    }

    #[test]
    fn lowers_owning_enum_remove_move_into_with_raw_output_pointer() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "buffer-owning-enum-remove-move-into.jdn",
                "module test; @repr(C) enum Event { Idle, Ready(Buffer<Int32>), Nested(Buffer<Buffer<Int32>>), Done } fn run(values: write Buffer<Event>, output: write Event) -> Int32 { let moved: Bool = buffer_remove_move_into(values, 0usize, output); let status: Int32 = buffer_remove_move_into_status(values, 99usize, output); if !moved { return 1 } return status }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("owning enum remove_move_into must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert_eq!(text.matches("buffer_remove_move_into").count(), 2);
        assert!(text.contains("call @f1("));
    }

    #[test]
    fn lowers_nested_buffer_remove_move_into_with_raw_output_pointer() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "buffer-nested-remove-move-into.jdn",
                "module test; fn run(values: write Buffer<Buffer<Int32>>, output: Buffer<Int32>) -> Int32 { let moved: Bool = buffer_remove_move_into(values, 0usize, output); if !moved { return 1 } return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("nested remove_move_into must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("buffer_remove_move_into"));
        assert!(text.contains("call @f1("));
    }

    #[test]
    fn lowers_nested_buffer_pop_move_into_with_raw_output_pointer() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "buffer-nested-pop-move-into.jdn",
                "module test; fn run(values: write Buffer<Buffer<Int32>>, output: Buffer<Int32>) -> Int32 { let moved: Bool = buffer_pop_move_into(values, output); let status: Int32 = buffer_pop_move_into_status(values, output); if !moved { return 1 } return status }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("nested buffer pop_move_into must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert_eq!(text.matches("buffer_pop_move_into").count(), 2);
        assert!(text.contains("call @f1("));
    }

    #[test]
    fn lowers_nested_buffer_insert_move_with_index_and_layouts() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "buffer-nested-insert-move.jdn",
                "module test; fn run(values: write Buffer<Buffer<Int32>>, inner: Buffer<Int32>) { buffer_insert_move(values, 0usize, inner) }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("nested buffer insert_move must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert_eq!(text.matches("buffer_insert_move").count(), 1);
        assert!(text.contains("call @f1("));
    }

    #[test]
    fn lowers_owning_record_insert_move_from_with_raw_value_pointer() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "buffer-owning-record-insert-move-from.jdn",
                "module test; @repr(C) struct Entry { values: Buffer<Int32>, id: Int32 } fn run(values: write Buffer<Entry>, incoming: Entry) { buffer_insert_move_from(values, 0usize, incoming) }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("owning record insert_move_from must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("buffer_insert_move_from"), "{text}");
        assert!(text.contains("stack_alloc"), "{text}");
        assert!(text.contains("call @f1("), "{text}");
    }

    #[test]
    fn lowers_checked_float4_slice_intrinsics_to_packed_jir() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "vector-slice.jdn",
                "module test; @noalloc fn probe(values: write Slice<Float32>, index: UIntSize, delta: Float32) { let current: Float4 = vector_load4(values, index); let amount: Float4 = vector_splat4(delta); vector_store4(values, index, current + amount) }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("vector slice MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("vector_bounds_check "));
        assert!(text.contains("vector_splat 4"));
        assert!(text.contains("vector.add "));
        assert!(text.contains("load %v"));
        assert!(text.contains("store %v"));
    }

    #[test]
    fn lowers_checked_float8_slice_intrinsics_to_packed_jir() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "vector-slice8.jdn",
                "module test; @noalloc fn probe(values: write Slice<Float32>, index: UIntSize, delta: Float32) { let current: Float8 = vector_load8(values, index); let amount: Float8 = vector_splat8(delta); vector_store8(values, index, current + amount) }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("vector slice MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("vector_bounds_check "));
        assert!(text.contains("vector_splat 8"));
        assert!(text.contains("vector.add "));
        assert!(text.contains("load %v"));
        assert!(text.contains("store %v"));
    }

    #[test]
    fn lowers_checked_float2_and_float3_slice_intrinsics_to_packed_jir() {
        for lanes in [2_u16, 3_u16] {
            let mut sources = SourceManager::new();
            let id = sources
                .add(
                    format!("vector-slice{lanes}.jdn"),
                    format!(
                        "module test; @noalloc fn probe(values: write Slice<Float32>, index: UIntSize, delta: Float32) {{ let current: Float{lanes} = vector_load{lanes}(values, index); let amount: Float{lanes} = vector_splat{lanes}(delta); vector_store{lanes}(values, index, current + amount) }}"
                    ),
                )
                .expect("source");
            let source = sources.get(id).expect("source");
            let lexed = lex(source);
            let parsed = parse(source, &lexed.tokens);
            let resolved = resolve(source, &parsed.file);
            let checked = check_types(source, &parsed.file, &resolved);
            assert!(
                !checked.has_errors(),
                "Float{lanes}: {:?}",
                checked.diagnostics
            );
            let hir = lower_hir(source, &parsed.file, &resolved, &checked);
            let mut mir = lower_mir(&hir.module, &checked.types);
            materialize_returns(&mut mir, &checked.types);
            elaborate_region_cleanup(&mut mir);
            elaborate_drops(&mut mir, &checked.types);

            let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
                .expect("vector slice MIR must lower");
            assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
            let text = jir.to_text();
            assert!(text.contains("vector_bounds_check "));
            assert!(text.contains(&format!("vector_splat {lanes}")));
            assert!(text.contains("vector.add "));
            assert!(text.contains("load %v"));
            assert!(text.contains("store %v"));
        }
    }

    #[test]
    fn lowers_arrays_indexed_assignment_records_and_strings() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "aggregates.jdn",
                "module test; struct Pair { z: Int32, a: Int32 } struct Box<T> { value: T } struct Node { next: Pointer<Node>, value: Int32 } enum Choice { First, Second(Int32) } fn array(index: Int32) -> Int32 { let values: [Int32; 3] = [1, 2, 3]; values[index] = 9; return values[index] } fn record() -> Int32 { let pair = Pair { a: 2, z: 1 }; return pair.a } fn record_value() -> Int32 { return (Pair { a: 2, z: 1 }).a } fn generic_record() -> Int32 { let boxed: Box<Int32> = Box { value: 7 }; return boxed.value } fn buffer(values: Buffer<Int32>, index: Int32) -> Int32 { return values[index] } fn buffer_set(values: Buffer<Int32>, index: Int32) { values[index] = 5 } fn slice(values: Slice<Int32>, index: Int32) -> Int32 { return values[index] } fn inspect(value: Choice) {} fn inspect_node(value: Node) {} fn text() { print(\"A\\n\") }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        assert_eq!(checked.nominal_layouts.len(), 4);
        assert!(checked.nominal_layouts.iter().any(|layout| matches!(
            &layout.kind,
            NominalLayoutKind::Record { fields }
                if fields.iter().map(|field| field.name.as_str()).collect::<Vec<_>>()
                    == ["z", "a"]
        )));
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("aggregate MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("array<3,"));
        assert!(text.contains("aggregate "));
        assert!(text.contains("bounds_check "));
        assert!(text.contains("extract_element "));
        assert!(text.contains("extract_value "));
        assert!(text.contains("string \"A\\n\""));
        assert!(text.contains("nominal_enum<"));
        assert!(text.contains("ptr<heap,"));
        assert!(text.contains("cast.int_extend "));
        assert!(jir.types.iter().enumerate().any(|(index, ty)| {
            let crate::Type::NominalStruct { fields, .. } = ty else {
                return false;
            };
            fields.iter().any(|field| {
                matches!(
                    jir.types.get(field.index()),
                    Some(crate::Type::Pointer { pointee, .. }) if pointee.index() == index
                )
            })
        }));
    }

    #[test]
    fn lowers_dynamic_for_iteration_length_and_indexing() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "dynamic_for.jdn",
                "module test; fn buffer(values: Buffer<Int32>) { for value in values { print(value) } } fn slice(values: Slice<Int32>) { for value in values { print(value) } }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("dynamic for MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("extract_value "));
        assert!(text.contains("bounds_check "));
        assert!(text.contains("extract_element ") || text.contains("load "));
    }

    #[test]
    fn lowers_slice_index_iteration_and_projected_field_updates() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "slice_indices.jdn",
                "module test; struct Agent { position: Int32, velocity: Int32 } fn update(agents: write Slice<Agent>) { for index in agents.indices { agents[index].position = agents[index].position + agents[index].velocity } }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("slice index iteration must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("bounds_check "), "{text}");
        assert!(text.contains("offset "), "{text}");
        assert!(text.contains("store "), "{text}");
    }

    #[test]
    fn lowers_numeric_casts_to_verified_jir_operations() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "casts.jdn",
                "module test; fn widen(value: Int32) -> Int64 { return value as Int64 } fn narrow(value: Int64) -> Int32 { return value as Int32 } fn real(value: Int32) -> Float64 { return value as Float64 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("numeric casts must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("cast.int_extend "), "{text}");
        assert!(text.contains("cast.int_truncate "), "{text}");
        assert!(text.contains("cast.int_to_float "), "{text}");
    }

    #[test]
    fn lowers_enum_match_and_option_result_propagation() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "carriers.jdn",
                "module test; enum Choice { First(Int32), Second(Int32) } enum Inner { Value(Int32) } enum Outer { Wrap(Inner) } fn choose(value: Choice) -> Int32 { return match value { Choice.First(item) if item > 0 => item, Choice.Second(item) => item, _ => 0 } } fn load(flag: Bool) -> Result<Int32, String> { return if flag { Ok(4) } else { Error(\"bad\") } } fn run(flag: Bool) -> Result<Int32, String> { let value = load(flag)?; return Ok(value + 1) } fn optional(value: Option<Int32>) -> Option<Int32> { let item = value?; return Some(item) } fn nested(value: Outer) -> Int32 { return match value { Outer.Wrap(Inner.Value(7)) => 1, _ => 0 } }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("enum and carrier MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert_eq!(jir.functions.len(), 5);
        assert!(text.contains("enum_construct 0("));
        assert!(text.contains("enum_construct 1("));
        assert!(text.contains("enum_tag "));
        assert!(text.contains("enum_extract "));
        assert!(text.contains("branch %v"));
        let nested_mir = mir
            .functions
            .iter()
            .find(|function| function.name == "nested")
            .expect("nested MIR function");
        let nested_jir = jir
            .functions
            .iter()
            .find(|function| function.name == "nested")
            .expect("nested JIR function");
        assert!(nested_jir.blocks.len() > nested_mir.blocks.len());
    }

    #[test]
    fn lowers_capabilities_regions_allocations_and_drops() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "memory.jdn",
                "module test; fn inspect(value: read Buffer<Int32>) {} fn borrow(data: Buffer<Int32>) { let view: read Buffer<Int32> = data; inspect(view) } fn read_at(values: read Buffer<Int32>, index: Int32) -> Int32 { return values[index] } fn release(data: Buffer<Int32>) {} fn region_run() { region frame { let values: Buffer<Int32> = frame.allocate(4); values[0] = 4; let view: read Buffer<Int32> = values; let first = view[0]; assert_eq(first, 0) } }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        infer_lifetimes(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("elaborated memory MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("region_handle"));
        assert!(text.contains("region_create"));
        assert!(text.contains("region_alloc "));
        assert!(text.contains("region_destroy "));
        assert!(text.contains("ptr<region,"));
        assert!(text.contains("cast.pointer "));
        assert!(text.contains("drop %v"));
    }

    #[test]
    fn lowers_time_now_unix_seconds_as_int64_import() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "time-now.jdn",
                "module test; fn main() -> Int32 { let now: Int64 = time_now_unix_seconds(); if now <= (0 as Int64) { return 1 } return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("time builtin MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let time_import = jir
            .functions
            .iter()
            .find(|function| function.name == "time_now_unix_seconds")
            .expect("time builtin import");
        assert_eq!(time_import.parameters.len(), 0);
        assert!(matches!(
            jir.types[time_import.result.index()],
            crate::Type::Integer {
                signed: true,
                bits: 64
            }
        ));
    }

    #[test]
    fn lowers_time_now_monotonic_ms_as_uint64_import() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "time-monotonic.jdn",
                "module test; fn main() -> Int32 { let first: UInt64 = time_now_monotonic_ms(); let second: UInt64 = time_now_monotonic_ms(); if second < first { return 1 } return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("monotonic time builtin MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let time_import = jir
            .functions
            .iter()
            .find(|function| function.name == "time_now_monotonic_ms")
            .expect("monotonic time builtin import");
        assert_eq!(time_import.parameters.len(), 0);
        assert!(matches!(
            jir.types[time_import.result.index()],
            crate::Type::Integer {
                signed: false,
                bits: 64
            }
        ));
    }

    #[test]
    fn lowers_bounded_scalar_parser_imports() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "bounded-parser.jdn",
                "module test; fn main() -> Int32 { var text: [UInt8; 5] = [49u8, 50u8, 46u8, 53u8, 48u8]; var signed: [Int64; 1] = [0 as Int64]; var unsigned: [UInt64; 1] = [0u64]; var float: [Float64; 1] = [0.0f64]; var boolean: [Bool; 1] = [false]; let a: Bool = parse_int(text, 2usize, signed); let b: Bool = parse_uint(text, 2usize, unsigned); let c: Bool = parse_float(text, 5usize, float); let d: Bool = parse_bool(text, 2usize, boolean); if !a { return 1 } if !b { return 2 } if !c { return 3 } if !d { return 4 } return signed[0] as Int32 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("bounded parser MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        for name in ["parse_int", "parse_uint", "parse_float", "parse_bool"] {
            let import = jir
                .functions
                .iter()
                .find(|function| function.name == name)
                .unwrap_or_else(|| panic!("{name} import"));
            assert_eq!(import.parameters.len(), 3, "{name}");
            assert!(matches!(
                jir.types[import.result.index()],
                crate::Type::Bool
            ));
        }
    }

    #[test]
    fn lowers_utc_calendar_parts_as_bounded_write_import() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "time-utc-parts.jdn",
                "module test; fn main() -> Int32 { var parts: [Int32; 6] = [0, 0, 0, 0, 0, 0]; let ok: Bool = time_utc_parts(0 as Int64, parts); if ok { return parts[0] } return 1 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("UTC calendar parts MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let import = jir
            .functions
            .iter()
            .find(|function| function.name == "time_utc_parts")
            .expect("UTC calendar parts import");
        // The slice stays one JIR aggregate parameter.  LLVM lowers its two
        // fields (pointer + length) according to the host C ABI at the native
        // boundary, so the import has two source-level parameters here:
        // timestamp and the borrowed output slice.
        assert_eq!(import.parameters.len(), 2);
        let output_slice = import.parameters[1].ty;
        assert!(matches!(
            &jir.types[output_slice.index()],
            crate::Type::Struct { fields } if fields.len() == 2
        ));
        assert!(matches!(
            jir.types[import.result.index()],
            crate::Type::Bool
        ));
    }

    #[test]
    fn lowers_utc_offset_calendar_parts_with_explicit_offset() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "time-utc-offset-parts.jdn",
                "module test; fn main() -> Int32 { var parts: [Int32; 6] = [0, 0, 0, 0, 0, 0]; let ok: Bool = time_utc_offset_parts(0 as Int64, 60, parts); if ok { return parts[3] } return 1 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("UTC offset calendar parts MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let import = jir
            .functions
            .iter()
            .find(|function| function.name == "time_utc_offset_parts")
            .expect("UTC offset calendar parts import");
        assert_eq!(import.parameters.len(), 3);
        let output_slice = import.parameters[2].ty;
        assert!(matches!(
            &jir.types[output_slice.index()],
            crate::Type::Struct { fields } if fields.len() == 2
        ));
        assert!(matches!(
            jir.types[import.result.index()],
            crate::Type::Bool
        ));
    }

    #[test]
    fn lowers_bounded_scheduler_poll_as_write_slice_import() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "scheduler.jdn",
                "module test; fn main() -> Int32 { app_scheduler_clear(); app_scheduler_set(7, 100 as Int64, 60u64); var due: [Int32; 4] = [0, 0, 0, 0]; let count: UIntSize = app_scheduler_poll(100 as Int64, due); if count == 1usize { return due[0] } return 1 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("scheduler MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let poll_import = jir
            .functions
            .iter()
            .find(|function| function.name == "app_scheduler_poll")
            .expect("scheduler poll import");
        assert_eq!(poll_import.parameters.len(), 2);
        let output_slice = poll_import.parameters[1].ty;
        assert!(matches!(
            &jir.types[output_slice.index()],
            crate::Type::Struct { fields } if fields.len() == 2
        ));
        assert!(matches!(
            jir.types[poll_import.result.index()],
            crate::Type::Integer {
                signed: false,
                bits: _
            }
        ));
    }

    #[test]
    fn lowers_bounded_http_router_with_directional_borrows_active() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "http-router-active.jdn",
                "module test; fn main() -> Int32 { http_router_clear(); var body: [UInt8; 1] = [79u8]; var input: [UInt8; 1] = [0u8]; var output: [UInt8; 8] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let _added: Bool = http_router_add(\"GET\", \"/\", 200u16, \"text/plain\", body); let _prefix_added: Bool = http_router_add_prefix(\"GET\", \"/api/\", 200u16, \"text/plain\", body); let _removed: Bool = http_router_remove(\"GET\", \"/\"); let _prefix_removed: Bool = http_router_remove_prefix(\"GET\", \"/api/\"); let _written: UIntSize = http_router_respond(input, output); let _count: UIntSize = http_router_count(); return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);
        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("router MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        for name in [
            "http_router_clear",
            "http_router_add",
            "http_router_add_prefix",
            "http_router_remove",
            "http_router_remove_prefix",
            "http_router_respond",
            "http_router_count",
        ] {
            assert!(
                jir.functions.iter().any(|function| function.name == name),
                "missing {name}"
            );
        }
    }

    #[test]
    fn lowers_http_keep_alive_with_directional_borrows_active() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "http-keep-alive-active.jdn",
                "module test; fn main() -> Int32 { var body: [UInt8; 1] = [79u8]; var input: [UInt8; 1] = [0u8]; var output: [UInt8; 1] = [0u8]; let keep: Bool = http_request_keep_alive(input); let _written: UIntSize = http_response_write_ex(200u16, \"text/plain\", body, keep, output); return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);
        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("keep-alive MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        for name in ["http_request_keep_alive", "http_response_write_ex"] {
            assert!(
                jir.functions.iter().any(|function| function.name == name),
                "missing {name}"
            );
        }
    }

    #[test]
    fn lowers_http_response_custom_header_with_directional_borrows_active() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "http-response-header.jdn",
                r#"module test; fn main() -> Int32 { var body: [UInt8; 1] = [79u8]; var output: [UInt8; 2] = [0u8, 0u8]; let close: UIntSize = http_response_write_header(429u16, "text/plain", "Retry-After", "1", body, output); let keep: UIntSize = http_response_write_header_ex(200u16, "text/plain", "X-Trace", "ready", body, true, output); return (close + keep) as Int32 }"#,
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics);
        let resolved = resolve(source, &parsed.file);
        assert!(!resolved.has_errors(), "{:?}", resolved.diagnostics);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);
        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("response-header MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        for name in [
            "http_response_write_header",
            "http_response_write_header_ex",
        ] {
            assert!(
                jir.functions.iter().any(|function| function.name == name),
                "missing {name}"
            );
        }
    }

    #[test]
    fn lowers_http_response_custom_header_block_with_directional_borrows_active() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "http-response-header-block.jdn",
                r#"module test; fn main() -> Int32 { var body: [UInt8; 1] = [79u8]; var output: [UInt8; 2] = [0u8, 0u8]; let close: UIntSize = http_response_write_header_block(429u16, "text/plain", "Retry-After: 1\r\nX-Trace: limited", body, output); let keep: UIntSize = http_response_write_header_block_ex(200u16, "text/plain", "X-Trace: ready", body, true, output); return (close + keep) as Int32 }"#,
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics);
        let resolved = resolve(source, &parsed.file);
        assert!(!resolved.has_errors(), "{:?}", resolved.diagnostics);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);
        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("response-header-block MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        for name in [
            "http_response_write_header_block",
            "http_response_write_header_block_ex",
        ] {
            assert!(
                jir.functions.iter().any(|function| function.name == name),
                "missing {name}"
            );
        }
    }

    #[test]
    fn lowers_http_session_builtins_active() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "http-session-active.jdn",
                "module test; fn main() -> Int32 { let listener: UIntSize = net_tcp_listen(38125u16); let session: UIntSize = http_session_open(listener, 2u32, 1024u32, 1024u32); let tls_session: UIntSize = http_session_open_tls(listener, 2u32, 1024u32, 1024u32, \"cert.pem\", \"key.pem\"); let state: UInt32 = http_session_step(session, 1u32); let _closed: Bool = http_session_close(session); let _tls_closed: Bool = http_session_close(tls_session); return state as Int32 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);
        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("HTTP session MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        for name in [
            "http_session_open",
            "http_session_open_tls",
            "http_session_step",
            "http_session_close",
        ] {
            assert!(
                jir.functions.iter().any(|function| function.name == name),
                "missing {name}"
            );
        }
    }

    #[test]
    fn lowers_net_reactor_builtins_active() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "reactor-active.jdn",
                "module test; fn main() -> Int32 { let listener: UIntSize = net_tcp_listen(38129u16); let reactor: UIntSize = net_reactor_open(4u32, 4u32); let watched: Bool = net_reactor_watch(reactor, listener, 1u32, 7usize); let count: UInt32 = net_reactor_poll(reactor, 1u32); let _socket: UIntSize = net_reactor_event_socket(reactor, 0u32); let _flags: UInt32 = net_reactor_event_flags(reactor, 0u32); let _user: UIntSize = net_reactor_event_user(reactor, 0u32); let _error: Int32 = net_reactor_error(reactor); let _unwatch: Bool = net_reactor_unwatch(reactor, listener); let _closed: Bool = net_reactor_close(reactor); let _watch_ok: Bool = watched; let _count: UInt32 = count; return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);
        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("reactor MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        for name in [
            "net_reactor_open",
            "net_reactor_watch",
            "net_reactor_poll",
            "net_reactor_event_socket",
            "net_reactor_event_flags",
            "net_reactor_event_user",
            "net_reactor_error",
            "net_reactor_unwatch",
            "net_reactor_close",
        ] {
            assert!(
                jir.functions.iter().any(|function| function.name == name),
                "missing {name}"
            );
        }
    }

    #[test]
    fn lowers_net_reactor_operations_active() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "reactor-operations-active.jdn",
                "module test; fn main() -> Int32 { let listener: UIntSize = net_tcp_listen(38130u16); let reactor: UIntSize = net_reactor_open(4u32, 8u32); let accept_operation: UIntSize = net_reactor_submit_accept(reactor, listener, 10usize); let connect_operation: UIntSize = net_reactor_submit_connect(reactor, \"127.0.0.1\", 38130u16, 11usize); let receive_operation: UIntSize = net_reactor_submit_receive(reactor, listener, 12usize); let send_operation: UIntSize = net_reactor_submit_send(reactor, listener, 13usize); let cancelled: Bool = net_reactor_cancel(reactor, accept_operation); let event_operation: UIntSize = net_reactor_event_operation(reactor, 0u32); let _closed: Bool = net_reactor_close(reactor); let _connect: UIntSize = connect_operation; let _receive: UIntSize = receive_operation; let _send: UIntSize = send_operation; let _event: UIntSize = event_operation; let _cancelled: Bool = cancelled; return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);
        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("reactor operation MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        for name in [
            "net_reactor_submit_accept",
            "net_reactor_submit_connect",
            "net_reactor_submit_receive",
            "net_reactor_submit_send",
            "net_reactor_cancel",
            "net_reactor_event_operation",
        ] {
            assert!(
                jir.functions.iter().any(|function| function.name == name),
                "missing {name}"
            );
        }
    }

    #[test]
    fn lowers_native_tls_builtins_active() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "tls-active.jdn",
                "module test; fn main() -> Int32 { let socket: UIntSize = net_tcp_connect_dns(\"localhost\", 38127u16); let tls: UIntSize = net_tls_open_client(socket, \"localhost\", false); let server_tls: UIntSize = net_tls_open_server(socket, \"cert.pem\", \"key.pem\"); let step: UInt32 = net_tls_step(tls, 1000u32); let state: UInt32 = net_tls_state(tls); let error: Int32 = net_tls_error(tls); var input: [UInt8; 2] = [1u8, 2u8]; var output: [UInt8; 2] = [0u8, 0u8]; let sent: UIntSize = net_tls_send(tls, input); let received: UIntSize = net_tls_receive(tls, output); let closed: Bool = net_tls_close(tls); let server_closed: Bool = net_tls_close(server_tls); if !server_closed { return 1 } if closed { return (step as Int32) + (state as Int32) + (sent as Int32) + (received as Int32) } return error }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);
        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("TLS MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        for name in [
            "net_tls_open_client",
            "net_tls_open_server",
            "net_tls_step",
            "net_tls_state",
            "net_tls_error",
            "net_tls_send",
            "net_tls_receive",
            "net_tls_close",
        ] {
            assert!(
                jir.functions.iter().any(|function| function.name == name),
                "missing {name}"
            );
        }
    }

    #[test]
    fn lowers_string_helpers_with_string_aggregate_abi() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "string-basics.jdn",
                "module test; fn main() -> Int32 { let value: String = \"Jadren\"; let length: UIntSize = string_length(value); let length32: Int32 = length as Int32; let equal: Bool = string_equals(value, \"Jadren\"); var output: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; var bytes: [UInt8; 3] = [33u8, 33u8, 33u8]; let first: UIntSize = string_builder_append(value, output, 0usize); let second: UIntSize = string_builder_append_bytes(bytes, output, first); if length32 != 6 { return 1 } if !equal { return 1 } if second != 9usize { return 1 } return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("string helper MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        for name in [
            "string_length",
            "string_equals",
            "string_builder_append",
            "string_builder_append_bytes",
        ] {
            assert!(
                jir.functions.iter().any(|function| function.name == name),
                "{name} import missing: {}",
                jir.to_text()
            );
        }
    }

    #[test]
    fn lowers_owned_string_result_and_cleanup_abi() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "string-owned.jdn",
                "module test; fn main() -> Int32 { let created: Result<OwnedString, Int32> = string_owned_from(\"Jadren\"); var result_code: Int32 = 1; match created { Ok(value) => { let appended: Int32 = string_owned_append(value, \"!\"); var output: [UInt8; 8] = [0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]; let copied: UIntSize = string_owned_copy(value, output); let length: UIntSize = string_owned_length(value); let cleared: Int32 = string_owned_clear(value); if appended == 0 { if copied == 6usize { if length == 6usize { result_code = cleared } } } } Error(status) => { result_code = status } } return result_code }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("owned string MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        for name in [
            "string_owned_from",
            "string_owned_append",
            "string_owned_length",
            "string_owned_copy",
            "string_owned_clear",
        ] {
            assert!(
                jir.functions.iter().any(|function| function.name == name),
                "{name} import missing: {}",
                jir.to_text()
            );
        }
    }

    #[test]
    fn lowers_owned_string_buffer_mutations_to_string_aware_runtime() {
        let mut sources = SourceManager::new();
        let source_text = include_str!("../../../examples/stdlib-buffer-owned-string.jdn");
        let id = sources
            .add("stdlib-buffer-owned-string.jdn", source_text)
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("owned string buffer MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("buffer_resize_move_owned_string "), "{text}");
        assert!(text.contains("buffer_remove_drop_owned_string "), "{text}");
        assert!(text.contains("owned_string_buffer_drop "), "{text}");
    }

    #[test]
    fn lowers_nested_owned_string_projection_calls() {
        let mut sources = SourceManager::new();
        let source_text = include_str!("../../../examples/stdlib-buffer-nested-owned-string.jdn");
        let id = sources
            .add("stdlib-buffer-nested-owned-string.jdn", source_text)
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("nested OwnedString projections must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("\"buffer_length\""), "{text}");
        assert!(text.contains("\"string_owned_length\""), "{text}");
        assert!(text.contains("\"string_owned_append\""), "{text}");
        assert!(text.contains("buffer_resize_move_owned_string "), "{text}");
        assert!(
            text.lines()
                .any(|line| line.contains("\"buffer_clear_move\"")),
            "{text}"
        );
        assert!(
            text.contains("recursive_owned_string_buffer_drop "),
            "{text}"
        );
    }

    #[test]
    fn lowers_nested_owned_string_copy_readback() {
        let mut sources = SourceManager::new();
        let source_text =
            include_str!("../../../examples/stdlib-buffer-nested-owned-string-copy.jdn");
        let id = sources
            .add("stdlib-buffer-nested-owned-string-copy.jdn", source_text)
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("nested OwnedString copy readback must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("string_owned_copy"), "{text}");
        assert!(text.contains("string_owned_length"), "{text}");
        assert!(
            text.lines()
                .any(|line| line.contains("\"buffer_clear_move\"")),
            "{text}"
        );
        assert!(
            text.contains("buffer_resize_move_nested_owned_string "),
            "{text}"
        );
        assert!(
            text.contains("recursive_owned_string_buffer_drop "),
            "{text}"
        );
    }

    #[test]
    fn lowers_nested_owned_string_move_into_with_cleanup() {
        let mut sources = SourceManager::new();
        let source_text =
            include_str!("../../../examples/stdlib-buffer-nested-owned-string-move-into.jdn");
        let id = sources
            .add(
                "stdlib-buffer-nested-owned-string-move-into.jdn",
                source_text,
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("nested OwnedString move-into must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("buffer_remove_move_into"), "{text}");
        assert!(text.contains("buffer_clear_move"), "{text}");
        assert!(text.contains("owned_string_buffer_drop "), "{text}");
        assert!(
            text.contains("recursive_owned_string_buffer_drop "),
            "{text}"
        );
    }
}
