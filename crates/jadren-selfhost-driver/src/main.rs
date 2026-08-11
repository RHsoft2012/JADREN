#![allow(unsafe_code)]
// JADREN-UNSAFE-AUDIT: dynamic loading is isolated to the producer ABI
// adapter in capture.rs; every symbol name and fixed C layout is versioned by
// jadren-selfhost-api, and the loaded libraries remain alive for each call.

use std::path::{Path, PathBuf};

use inkwell::context::Context;
use jadren_codegen_llvm::{
    ObjectOptions, TypeLoweringConfig, lower_to_object_with_summary, write_object,
};
#[cfg(windows)]
use jadren_codegen_llvm::{WindowsLinkOptions, link_windows_executable};
use jadren_selfhost_api::{
    ExpressionAstArgumentHeader, TYPE_KIND_INTEGER, TypedCallBindingHeader,
    TypedExpressionAstNodeHeader, TypedNameBindingHeader,
};
use jadren_selfhost_stage2::{
    TypedLocalFunctionDefinition, decode_stage2_capture, decode_typed_if_return_capture,
    decode_typed_if_value_continuation_capture, decode_typed_if_value_local_assignment_capture,
    decode_typed_if_value_statement_tail_capture, decode_typed_nested_while_local_capture,
    decode_typed_statement_capture, decode_typed_while_local_body_let_capture,
    decode_typed_while_local_capture, decode_typed_while_local_statements_capture,
    import_stage2_jir, lower_typed_if_return, lower_typed_if_value_to_local_with_assignment,
    lower_typed_if_value_to_local_with_statements, lower_typed_if_value_with_continuation,
    lower_typed_nested_while_local, lower_typed_nested_while_local_with_conditional_control,
    lower_typed_nested_while_local_with_conditional_controls,
    lower_typed_nested_while_local_with_ordered_inner_body, lower_typed_statement_sequence,
    lower_typed_while_local, lower_typed_while_local_with_statements,
    materialize_typed_local_function_definitions,
};
use jadren_source::SourceManager;

mod capture;

fn main() {
    if let Err(message) = run() {
        eprintln!("jadren self-host stage-2 driver failed: {message}");
        std::process::exit(1);
    }
}

/// Builds the caller-owned signature/argument side tables for the first
/// dynamic-while call slice. The producer ABI intentionally remains the
/// existing `JST2WHG1` layout; only the host adapter owns these extra records.
fn dynamic_while_call_argument_nodes(
    source: &str,
    ast: &[TypedExpressionAstNodeHeader],
    call_index: usize,
    call: &TypedExpressionAstNodeHeader,
) -> Result<Vec<usize>, String> {
    let callee_index = usize::try_from(call.left)
        .map_err(|_| "dynamic while call callee index is not representable".to_owned())?;
    let callee = ast
        .get(callee_index)
        .ok_or_else(|| "dynamic while call callee index is outside the captured AST".to_owned())?;
    let callee_end = usize::try_from(callee.end)
        .map_err(|_| "dynamic while call callee span is not representable".to_owned())?;
    let call_end = usize::try_from(call.end)
        .map_err(|_| "dynamic while call span is not representable".to_owned())?;
    if callee_end >= call_end || call_end > source.len() {
        return Err("dynamic while call span is outside the source".to_owned());
    }
    let bytes = source.as_bytes();
    let mut cursor = callee_end;
    while cursor < call_end && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    if cursor >= call_end || bytes[cursor] != b'(' {
        return Err("dynamic while call has no opening parenthesis".to_owned());
    }
    cursor += 1;

    let mut parenthesis_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut argument_start = None;
    let mut argument_end = call_end;
    let mut spans = Vec::new();
    let mut closed = false;
    while cursor < call_end {
        let byte = bytes[cursor];
        if byte.is_ascii_whitespace() {
            cursor += 1;
            continue;
        }
        if argument_start.is_none() {
            argument_start = Some(cursor);
        }
        match byte {
            b'(' => {
                parenthesis_depth += 1;
                argument_end = cursor + 1;
            }
            b'[' => {
                bracket_depth += 1;
                argument_end = cursor + 1;
            }
            b')' => {
                if parenthesis_depth == 0 {
                    if bracket_depth != 0 {
                        return Err(
                            "dynamic while call argument has an unclosed bracket".to_owned()
                        );
                    }
                    if let Some(start) = argument_start.take() {
                        spans.push((start, argument_end));
                    }
                    closed = true;
                    cursor += 1;
                    break;
                }
                parenthesis_depth -= 1;
                argument_end = cursor + 1;
            }
            b']' => {
                if bracket_depth == 0 {
                    return Err("dynamic while call argument has an unexpected bracket".to_owned());
                }
                bracket_depth -= 1;
                argument_end = cursor + 1;
            }
            b',' if parenthesis_depth == 0 && bracket_depth == 0 => {
                let start = argument_start
                    .take()
                    .ok_or_else(|| "dynamic while call contains an empty argument".to_owned())?;
                spans.push((start, argument_end));
                argument_end = call_end;
            }
            _ => {
                argument_end = cursor + 1;
            }
        }
        cursor += 1;
    }
    if !closed || parenthesis_depth != 0 || bracket_depth != 0 {
        return Err("dynamic while call has an unterminated argument list".to_owned());
    }
    while cursor < call_end && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    if cursor != call_end {
        return Err("dynamic while call contains trailing tokens".to_owned());
    }
    if spans.len()
        != usize::try_from(call.aux)
            .map_err(|_| "dynamic while call argument count is not representable".to_owned())?
    {
        return Err("dynamic while call argument count does not match its source".to_owned());
    }

    spans
        .into_iter()
        .map(|(start, end)| {
            let node = ast
                .iter()
                .enumerate()
                .take(call_index)
                .rev()
                .find(|(_, candidate)| {
                    candidate.start == start as u64 && candidate.end == end as u64
                })
                .map(|(index, _)| index)
                .ok_or_else(|| "dynamic while call argument span has no AST root".to_owned())?;
            let argument = ast.get(node).ok_or_else(|| {
                "dynamic while call argument root is outside the captured AST".to_owned()
            })?;
            if argument.syntax_kind != 1
                && argument.syntax_kind != 2
                && argument.syntax_kind != 3
                && argument.syntax_kind != 5
                && argument.syntax_kind != 6
                && argument.syntax_kind != 7
            {
                return Err(
                    "dynamic while call arguments must be Int32 names, literals, or arithmetic expressions"
                        .to_owned(),
                );
            }
            Ok(node)
        })
        .collect()
}

fn dynamic_while_call_metadata(
    source: &str,
    ast: &[TypedExpressionAstNodeHeader],
) -> Result<
    (
        Vec<TypedCallBindingHeader>,
        Vec<ExpressionAstArgumentHeader>,
    ),
    String,
> {
    let mut bindings = Vec::new();
    let mut arguments = Vec::new();
    for (call_index, call) in ast.iter().enumerate() {
        if call.syntax_kind != 5 {
            continue;
        }
        if call.aux == 0 || call.aux > 2 {
            return Err("dynamic while call currently requires one or two arguments".to_owned());
        }
        let callee_index = usize::try_from(call.left)
            .map_err(|_| "dynamic while call callee index is not representable".to_owned())?;
        let callee = ast.get(callee_index).ok_or_else(|| {
            "dynamic while call callee index is outside the captured AST".to_owned()
        })?;
        if callee.syntax_kind != 1 {
            return Err("dynamic while call callee is not an identifier".to_owned());
        }
        let callee_start = usize::try_from(callee.start)
            .map_err(|_| "dynamic while call callee span is not representable".to_owned())?;
        let callee_end = usize::try_from(callee.end)
            .map_err(|_| "dynamic while call callee span is not representable".to_owned())?;
        if callee_start >= callee_end || callee_end > source.len() {
            return Err("dynamic while call span is outside the source".to_owned());
        }
        let callee_name = &source[callee_start..callee_end];
        if let Some(existing) = bindings.iter().find(|binding: &&TypedCallBindingHeader| {
            let existing_start = usize::try_from(binding.name_start).ok();
            let existing_end = usize::try_from(binding.name_end).ok();
            match (existing_start, existing_end) {
                (Some(start), Some(end)) if start <= end && end <= source.len() => {
                    &source[start..end] == callee_name
                }
                _ => false,
            }
        }) {
            if existing.parameter_count != call.aux
                || existing.return_type_kind != TYPE_KIND_INTEGER
            {
                return Err(
                    "dynamic while call reuses a signature name with incompatible metadata"
                        .to_owned(),
                );
            }
        } else {
            bindings.push(TypedCallBindingHeader {
                name_start: callee.start,
                name_end: callee.end,
                parameter_count: call.aux,
                return_type_kind: TYPE_KIND_INTEGER,
            });
        }
        let argument_nodes = dynamic_while_call_argument_nodes(source, ast, call_index, call)?;
        for ordinal in 0..call.aux {
            let ordinal_index = usize::try_from(ordinal).map_err(|_| {
                "dynamic while call argument ordinal is not representable".to_owned()
            })?;
            let argument_index = *argument_nodes.get(ordinal_index).ok_or_else(|| {
                "dynamic while call argument root is missing from its source stream".to_owned()
            })?;
            let argument = ast.get(argument_index).ok_or_else(|| {
                "dynamic while call argument index is outside the captured AST".to_owned()
            })?;
            if argument.syntax_kind != 1
                && argument.syntax_kind != 2
                && argument.syntax_kind != 3
                && argument.syntax_kind != 5
                && argument.syntax_kind != 6
                && argument.syntax_kind != 7
            {
                return Err(
                    "dynamic while call arguments must be Int32 names, literals, or arithmetic expressions"
                        .to_owned(),
                );
            }
            let argument_start = usize::try_from(argument.start)
                .map_err(|_| "dynamic while call argument span is not representable".to_owned())?;
            let argument_end = usize::try_from(argument.end)
                .map_err(|_| "dynamic while call argument span is not representable".to_owned())?;
            if argument_start >= argument_end || argument_end > source.len() {
                return Err("dynamic while call argument span is outside the source".to_owned());
            }
            arguments.push(ExpressionAstArgumentHeader {
                call_node: call_index as u64,
                ordinal,
                node: argument_index as u64,
                start: argument.start,
                end: argument.end,
            });
        }
    }
    if bindings.is_empty() {
        return Err("dynamic while call fixture contains no call AST node".to_owned());
    }
    Ok((bindings, arguments))
}

/// Parses the deliberately small same-module definition slice used by the
/// dynamic typed-call proof.  The producer already owns the full source; this
/// host-side parser only admits `fn name(Int32 params...) -> Int32 { return
/// <Int32 expression>; }` bodies and emits the same caller-owned post-order
/// AST shape consumed by Stage-2.  It is intentionally not a replacement for
/// the Jadren parser and rejects unsupported function bodies loudly.
fn dynamic_while_local_function_definitions(
    source: &str,
    call_bindings: &[TypedCallBindingHeader],
) -> Result<Vec<TypedLocalFunctionDefinition>, String> {
    call_bindings
        .iter()
        .map(|binding| {
            let name_start = usize::try_from(binding.name_start)
                .map_err(|_| "same-module function name start is not representable".to_owned())?;
            let name_end = usize::try_from(binding.name_end)
                .map_err(|_| "same-module function name end is not representable".to_owned())?;
            let name = source
                .get(name_start..name_end)
                .ok_or_else(|| "same-module function name span is outside source".to_owned())?;
            let mut found = None;
            for (offset, _) in source.match_indices("fn") {
                let mut cursor = offset + 2;
                if cursor >= source.len()
                    || !source.as_bytes()[offset..cursor]
                        .iter()
                        .all(|byte| byte.is_ascii_alphabetic())
                {
                    continue;
                }
                while cursor < source.len() && source.as_bytes()[cursor].is_ascii_whitespace() {
                    cursor += 1;
                }
                let candidate_start = cursor;
                while cursor < source.len()
                    && (source.as_bytes()[cursor].is_ascii_alphanumeric()
                        || source.as_bytes()[cursor] == b'_')
                {
                    cursor += 1;
                }
                if source.get(candidate_start..cursor) != Some(name) {
                    continue;
                }
                found = Some((offset, candidate_start, cursor));
                break;
            }
            let (_, candidate_start, candidate_end) = found
                .ok_or_else(|| format!("same-module function definition `{name}` was not found"))?;
            let mut cursor = candidate_end;
            while cursor < source.len() && source.as_bytes()[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if source.as_bytes().get(cursor) != Some(&b'(') {
                return Err("same-module function definition has no parameter list".to_owned());
            }
            let parameter_open = cursor;
            let parameter_close = matching_delimiter(source, parameter_open, b'(', b')')?;
            let parameters =
                parse_same_module_parameters(source, parameter_open + 1, parameter_close)?;
            if parameters.len()
                != usize::try_from(binding.parameter_count).map_err(|_| {
                    "same-module function parameter count is not representable".to_owned()
                })?
            {
                return Err("same-module function parameter count does not match call".to_owned());
            }
            cursor = parameter_close + 1;
            while cursor < source.len() && source.as_bytes()[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if !source[cursor..].starts_with("->") {
                return Err("same-module function definition has no return arrow".to_owned());
            }
            cursor += 2;
            while cursor < source.len() && source.as_bytes()[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            let return_start = cursor;
            while cursor < source.len() && source.as_bytes()[cursor].is_ascii_alphanumeric() {
                cursor += 1;
            }
            if source.get(return_start..cursor) != Some("Int32") {
                return Err("same-module function definition must return Int32".to_owned());
            }
            while cursor < source.len() && source.as_bytes()[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if source.as_bytes().get(cursor) != Some(&b'{') {
                return Err("same-module function definition has no body".to_owned());
            }
            let body_open = cursor;
            let body_close = matching_delimiter(source, body_open, b'{', b'}')?;
            let body_inner_start = body_open + 1;
            let body_inner_end = body_close;
            let body = source
                .get(body_inner_start..body_inner_end)
                .ok_or_else(|| "same-module function body span is outside source".to_owned())?;
            let body_trim_start = body_inner_start + body.len() - body.trim_start().len();
            let body_trim = &source[body_trim_start..body_inner_end];
            if !body_trim.starts_with("return") {
                return Err("same-module function body must contain one return".to_owned());
            }
            let mut expression_start = body_trim_start + "return".len();
            while expression_start < body_inner_end
                && source.as_bytes()[expression_start].is_ascii_whitespace()
            {
                expression_start += 1;
            }
            let semicolon = source[expression_start..body_inner_end]
                .rfind(';')
                .map(|offset| expression_start + offset)
                .ok_or_else(|| "same-module function return has no semicolon".to_owned())?;
            if !source[semicolon + 1..body_inner_end].trim().is_empty() {
                return Err("same-module function body has statements after return".to_owned());
            }
            let expression_end = semicolon;
            if expression_start >= expression_end {
                return Err("same-module function return expression is empty".to_owned());
            }
            let mut parser =
                SameModuleExpressionParser::new(source, expression_start, expression_end);
            let parsed = parser.parse()?;
            Ok(TypedLocalFunctionDefinition {
                name_start: candidate_start as u64,
                name_end: candidate_end as u64,
                body_start: body_open as u64,
                body_end: (body_close + 1) as u64,
                return_type_kind: TYPE_KIND_INTEGER,
                parameters,
                ast: parsed.ast,
                call_bindings: parsed.call_bindings,
                arguments: parsed.arguments,
            })
        })
        .collect()
}

fn matching_delimiter(
    source: &str,
    open: usize,
    opening: u8,
    closing: u8,
) -> Result<usize, String> {
    let mut depth = 0usize;
    for (offset, byte) in source.as_bytes().iter().enumerate().skip(open) {
        if *byte == opening {
            depth = depth.saturating_add(1);
        } else if *byte == closing {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Ok(offset);
            }
        }
    }
    Err("same-module definition has an unterminated delimiter".to_owned())
}

fn parse_same_module_parameters(
    source: &str,
    start: usize,
    end: usize,
) -> Result<Vec<TypedNameBindingHeader>, String> {
    let content = source
        .get(start..end)
        .ok_or_else(|| "same-module parameter span is outside source".to_owned())?;
    if content.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut parameters = Vec::new();
    let mut segment_start = start;
    for cursor in start..=end {
        if cursor != end && source.as_bytes()[cursor] != b',' {
            continue;
        }
        let segment = source[segment_start..cursor].trim();
        let leading =
            source[segment_start..cursor].len() - source[segment_start..cursor].trim_start().len();
        let absolute_start = segment_start + leading;
        let colon = segment
            .find(':')
            .ok_or_else(|| "same-module parameter has no type".to_owned())?;
        let name = segment[..colon].trim();
        let ty = segment[colon + 1..].trim();
        if ty != "Int32" || name.is_empty() {
            return Err("same-module parameters currently require named Int32 values".to_owned());
        }
        let name_start =
            absolute_start + segment[..colon].len() - segment[..colon].trim_start().len();
        parameters.push(TypedNameBindingHeader {
            name_start: name_start as u64,
            name_end: (name_start + name.len()) as u64,
            type_kind: TYPE_KIND_INTEGER,
        });
        segment_start = cursor + 1;
    }
    Ok(parameters)
}

struct SameModuleExpressionParser<'a> {
    source: &'a str,
    end: usize,
    cursor: usize,
    ast: Vec<TypedExpressionAstNodeHeader>,
    call_bindings: Vec<TypedCallBindingHeader>,
    arguments: Vec<ExpressionAstArgumentHeader>,
}

struct SameModuleExpressionParse {
    ast: Vec<TypedExpressionAstNodeHeader>,
    call_bindings: Vec<TypedCallBindingHeader>,
    arguments: Vec<ExpressionAstArgumentHeader>,
}

impl<'a> SameModuleExpressionParser<'a> {
    fn new(source: &'a str, start: usize, end: usize) -> Self {
        Self {
            source,
            end,
            cursor: start,
            ast: Vec::new(),
            call_bindings: Vec::new(),
            arguments: Vec::new(),
        }
    }

    fn parse(&mut self) -> Result<SameModuleExpressionParse, String> {
        let root = self.parse_additive()?;
        self.skip_whitespace();
        if self.cursor != self.end || root + 1 != self.ast.len() {
            return Err("same-module return expression contains unsupported tokens".to_owned());
        }
        Ok(SameModuleExpressionParse {
            ast: std::mem::take(&mut self.ast),
            call_bindings: std::mem::take(&mut self.call_bindings),
            arguments: std::mem::take(&mut self.arguments),
        })
    }

    fn parse_additive(&mut self) -> Result<usize, String> {
        let mut left = self.parse_multiplicative()?;
        loop {
            self.skip_whitespace();
            let operator = match self.source.as_bytes().get(self.cursor) {
                Some(b'+') => 7,
                Some(b'-') => 8,
                _ => break,
            };
            self.cursor += 1;
            let right = self.parse_multiplicative()?;
            left = self.push_node(7, left, right, operator);
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<usize, String> {
        let mut left = self.parse_unary()?;
        loop {
            self.skip_whitespace();
            if self.source.as_bytes().get(self.cursor) != Some(&b'*') {
                break;
            }
            self.cursor += 1;
            let right = self.parse_unary()?;
            left = self.push_node(7, left, right, 9);
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<usize, String> {
        self.skip_whitespace();
        let start = self.cursor;
        let operator = match self.source.as_bytes().get(self.cursor) {
            Some(b'+') => Some(10),
            Some(b'-') => Some(11),
            _ => None,
        };
        if let Some(operator) = operator {
            self.cursor += 1;
            let operand = self.parse_unary()?;
            let end = self.ast[operand].end as usize;
            let index = self.ast.len();
            self.ast.push(TypedExpressionAstNodeHeader {
                syntax_kind: 6,
                type_kind: TYPE_KIND_INTEGER,
                flags: 1,
                reserved: 0,
                left: operand as u64,
                right: 0,
                aux: operator as u64,
                start: start as u64,
                end: end as u64,
            });
            return Ok(index);
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<usize, String> {
        self.skip_whitespace();
        let start = self.cursor;
        match self.source.as_bytes().get(self.cursor).copied() {
            Some(b'(') => {
                self.cursor += 1;
                let child = self.parse_additive()?;
                self.skip_whitespace();
                if self.source.as_bytes().get(self.cursor) != Some(&b')') {
                    return Err("same-module expression group is unterminated".to_owned());
                }
                self.cursor += 1;
                let index = self.ast.len();
                self.ast.push(TypedExpressionAstNodeHeader {
                    syntax_kind: 3,
                    type_kind: TYPE_KIND_INTEGER,
                    flags: 1,
                    reserved: 0,
                    left: child as u64,
                    right: 0,
                    aux: 0,
                    start: start as u64,
                    end: self.cursor as u64,
                });
                Ok(index)
            }
            Some(byte) if byte.is_ascii_digit() => {
                while self
                    .source
                    .as_bytes()
                    .get(self.cursor)
                    .is_some_and(|byte| byte.is_ascii_digit())
                {
                    self.cursor += 1;
                }
                let index = self.ast.len();
                self.ast.push(TypedExpressionAstNodeHeader {
                    syntax_kind: 2,
                    type_kind: TYPE_KIND_INTEGER,
                    flags: 1,
                    reserved: 0,
                    left: 0,
                    right: 0,
                    aux: 0,
                    start: start as u64,
                    end: self.cursor as u64,
                });
                Ok(index)
            }
            Some(byte) if byte.is_ascii_alphabetic() || byte == b'_' => {
                self.cursor += 1;
                while self
                    .source
                    .as_bytes()
                    .get(self.cursor)
                    .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                {
                    self.cursor += 1;
                }
                let identifier_end = self.cursor;
                let identifier_index = self.ast.len();
                self.ast.push(TypedExpressionAstNodeHeader {
                    syntax_kind: 1,
                    type_kind: TYPE_KIND_INTEGER,
                    flags: 1,
                    reserved: 0,
                    left: 0,
                    right: 0,
                    aux: 0,
                    start: start as u64,
                    end: identifier_end as u64,
                });
                self.skip_whitespace();
                if self.source.as_bytes().get(self.cursor) != Some(&b'(') {
                    return Ok(identifier_index);
                }

                self.cursor += 1;
                self.skip_whitespace();
                if self.source.as_bytes().get(self.cursor) == Some(&b')') {
                    return Err("same-module calls require at least one Int32 argument".to_owned());
                }
                let mut argument_nodes = Vec::new();
                loop {
                    let argument = self.parse_additive()?;
                    argument_nodes.push(argument);
                    self.skip_whitespace();
                    match self.source.as_bytes().get(self.cursor) {
                        Some(b',') => {
                            self.cursor += 1;
                            self.skip_whitespace();
                            if self.source.as_bytes().get(self.cursor) == Some(&b')') {
                                return Err(
                                    "same-module call has a trailing argument separator".to_owned()
                                );
                            }
                        }
                        Some(b')') => break,
                        _ => {
                            return Err(
                                "same-module call arguments must be comma-separated".to_owned()
                            );
                        }
                    }
                }
                self.cursor += 1;
                let call_end = self.cursor;
                let call_index = self.ast.len();
                let first_argument = argument_nodes[0];
                self.ast.push(TypedExpressionAstNodeHeader {
                    syntax_kind: 5,
                    type_kind: TYPE_KIND_INTEGER,
                    flags: 1,
                    reserved: 0,
                    left: identifier_index as u64,
                    right: first_argument as u64,
                    aux: argument_nodes.len() as u64,
                    start: start as u64,
                    end: call_end as u64,
                });
                let binding = TypedCallBindingHeader {
                    name_start: start as u64,
                    name_end: identifier_end as u64,
                    parameter_count: argument_nodes.len() as u64,
                    return_type_kind: TYPE_KIND_INTEGER,
                };
                let duplicate = self.call_bindings.iter().any(|existing| {
                    existing.parameter_count == binding.parameter_count
                        && existing.name_start == binding.name_start
                        && existing.name_end == binding.name_end
                });
                if !duplicate {
                    self.call_bindings.push(binding);
                }
                for (ordinal, argument_index) in argument_nodes.iter().copied().enumerate() {
                    let argument = self.ast[argument_index];
                    self.arguments.push(ExpressionAstArgumentHeader {
                        call_node: call_index as u64,
                        ordinal: ordinal as u64,
                        node: argument_index as u64,
                        start: argument.start,
                        end: argument.end,
                    });
                }
                Ok(call_index)
            }
            _ => Err("same-module expression requires an Int32 literal or identifier".to_owned()),
        }
    }

    fn skip_whitespace(&mut self) {
        while self.cursor < self.end && self.source.as_bytes()[self.cursor].is_ascii_whitespace() {
            self.cursor += 1;
        }
    }

    fn push_node(&mut self, syntax_kind: u8, left: usize, right: usize, operator: u8) -> usize {
        let index = self.ast.len();
        self.ast.push(TypedExpressionAstNodeHeader {
            syntax_kind,
            type_kind: TYPE_KIND_INTEGER,
            flags: 1,
            reserved: 0,
            left: left as u64,
            right: right as u64,
            aux: operator as u64,
            start: self.ast[left].start,
            end: self.ast[right].end,
        });
        index
    }
}

fn run() -> Result<(), String> {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("capture")) {
        return run_capture_command();
    }
    let arguments = parse_arguments()?;
    let capture_bytes = std::fs::read(&arguments.capture)
        .map_err(|error| format!("cannot read `{}`: {error}", arguments.capture.display()))?;
    let typed_while_local_body_let_capture = capture_bytes.starts_with(b"JST2WHB1");
    let typed_nested_while_local_capture = capture_bytes.starts_with(b"JST2WN01")
        || capture_bytes.starts_with(b"JST2WNQ1")
        || capture_bytes.starts_with(b"JST2WNM1")
        || capture_bytes.starts_with(b"JST2WND1")
        || capture_bytes.starts_with(b"JST2WNO1");
    let typed_nested_while_local_conditional_capture = capture_bytes.starts_with(b"JST2WNQ1")
        || capture_bytes.starts_with(b"JST2WNM1")
        || capture_bytes.starts_with(b"JST2WND1")
        || capture_bytes.starts_with(b"JST2WNO1");
    let typed_nested_while_local_multi_conditional_capture = capture_bytes.starts_with(b"JST2WNM1");
    let typed_nested_while_local_dynamic_conditional_capture =
        capture_bytes.starts_with(b"JST2WND1");
    let typed_nested_while_local_ordered_body_capture = capture_bytes.starts_with(b"JST2WNO1");
    let typed_while_local_capture = capture_bytes.starts_with(b"JST2WHL1");
    let typed_while_local_statements_capture = capture_bytes.starts_with(b"JST2WHS1")
        || capture_bytes.starts_with(b"JST2WHK1")
        || capture_bytes.starts_with(b"JST2WHC1")
        || capture_bytes.starts_with(b"JST2WHQ1")
        || capture_bytes.starts_with(b"JST2WHM1")
        || capture_bytes.starts_with(b"JST2WHD1")
        || capture_bytes.starts_with(b"JST2WHG1")
        || capture_bytes.starts_with(b"JST2WH31");
    let typed_if_value_statement_tail_capture = capture_bytes.starts_with(b"JST2IFS1");
    let typed_if_value_local_assignment_capture = capture_bytes.starts_with(b"JST2IFL1");
    let typed_if_value_continuation_capture = capture_bytes.starts_with(b"JST2IFV1");
    let typed_if_capture = capture_bytes.starts_with(b"JST2IF01");
    let typed_capture = capture_bytes.starts_with(b"JST2TYP1");
    let (source_text, module, capture_record_count) = if typed_nested_while_local_capture {
        let capture = decode_typed_nested_while_local_capture(&capture_bytes)
            .map_err(|error| error.to_string())?;
        let source_text = capture.source;
        let mut sources = SourceManager::new();
        let source_id = sources
            .add(arguments.capture.clone(), source_text.clone())
            .map_err(|error| error.to_string())?;
        let inner_index = capture
            .body
            .iter()
            .position(|statement| statement.kind == 1)
            .ok_or_else(|| {
                "nested typed while capture is missing its inner initializer".to_owned()
            })?;
        let inner_initializer = capture.body[inner_index];
        let outer_prefix = capture.body[..inner_index].to_vec();
        let after_initializer = &capture.body[inner_index + 1..];
        let inner_body_count = after_initializer
            .iter()
            .take_while(|statement| {
                statement.kind == 2
                    && statement.start >= capture.controls[1].start
                    && statement.end <= capture.controls[1].end
            })
            .count();
        let inner_body = after_initializer[..inner_body_count].to_vec();
        let module = if typed_nested_while_local_conditional_capture {
            let conditional_index = after_initializer
                .iter()
                .position(|statement| {
                    statement.flags == 2 && (statement.kind == 4 || statement.kind == 5)
                })
                .ok_or_else(|| {
                    "nested conditional capture is missing its inner control".to_owned()
                })?;
            if !typed_nested_while_local_ordered_body_capture
                && conditional_index != inner_body_count
            {
                return Err("nested conditional capture has an unordered inner control".to_owned());
            }
            let conditional_controls = after_initializer[conditional_index..]
                .iter()
                .take_while(|statement| {
                    statement.flags == 2 && (statement.kind == 4 || statement.kind == 5)
                })
                .copied()
                .collect::<Vec<_>>();
            if conditional_controls.is_empty()
                || (typed_nested_while_local_multi_conditional_capture
                    && conditional_controls.len() != 2)
                || (!typed_nested_while_local_multi_conditional_capture
                    && !typed_nested_while_local_dynamic_conditional_capture
                    && conditional_controls.len() != 1)
            {
                return Err("nested conditional capture has an invalid control list".to_owned());
            }
            if typed_nested_while_local_ordered_body_capture {
                let ordered_inner_body = after_initializer
                    .iter()
                    .take_while(|statement| {
                        statement.start >= capture.controls[1].start
                            && statement.end <= capture.controls[1].end
                    })
                    .copied()
                    .collect::<Vec<_>>();
                if ordered_inner_body.is_empty() {
                    return Err("ordered nested capture has no inner body records".to_owned());
                }
                let ordered_tail = after_initializer[ordered_inner_body.len()..].to_vec();
                lower_typed_nested_while_local_with_ordered_inner_body(
                    &source_text,
                    source_id,
                    &capture.ast,
                    &[],
                    capture.initializer_node,
                    &capture.bindings[0],
                    &capture.controls[0],
                    &outer_prefix,
                    &inner_initializer,
                    &capture.bindings[1],
                    &capture.controls[1],
                    &ordered_inner_body,
                    &ordered_tail,
                    &[],
                    &[],
                    &[],
                )
            } else {
                let outer_tail =
                    after_initializer[conditional_index + conditional_controls.len()..].to_vec();
                if typed_nested_while_local_multi_conditional_capture
                    || typed_nested_while_local_dynamic_conditional_capture
                {
                    lower_typed_nested_while_local_with_conditional_controls(
                        &source_text,
                        source_id,
                        &capture.ast,
                        &[],
                        capture.initializer_node,
                        &capture.bindings[0],
                        &capture.controls[0],
                        &outer_prefix,
                        &inner_initializer,
                        &capture.bindings[1],
                        &capture.controls[1],
                        &inner_body,
                        &conditional_controls,
                        &outer_tail,
                        &[],
                        &[],
                        &[],
                    )
                } else {
                    lower_typed_nested_while_local_with_conditional_control(
                        &source_text,
                        source_id,
                        &capture.ast,
                        &[],
                        capture.initializer_node,
                        &capture.bindings[0],
                        &capture.controls[0],
                        &outer_prefix,
                        &inner_initializer,
                        &capture.bindings[1],
                        &capture.controls[1],
                        &inner_body,
                        &conditional_controls[0],
                        &outer_tail,
                        &[],
                        &[],
                        &[],
                    )
                }
            }
        } else {
            let outer_tail = after_initializer[inner_body_count..].to_vec();
            lower_typed_nested_while_local(
                &source_text,
                source_id,
                &capture.ast,
                &[],
                capture.initializer_node,
                &capture.bindings[0],
                &capture.controls[0],
                &outer_prefix,
                &inner_initializer,
                &capture.bindings[1],
                &capture.controls[1],
                &inner_body,
                &outer_tail,
                &[],
                &[],
                &[],
            )
        }
        .map_err(|error| error.to_string())?;
        (source_text, module, 0usize)
    } else if typed_while_local_body_let_capture {
        let capture = decode_typed_while_local_body_let_capture(&capture_bytes)
            .map_err(|error| error.to_string())?;
        let source_text = capture.source;
        let mut sources = SourceManager::new();
        let source_id = sources
            .add(arguments.capture.clone(), source_text.clone())
            .map_err(|error| error.to_string())?;
        let module = lower_typed_while_local_with_statements(
            &source_text,
            source_id,
            &capture.ast,
            &[],
            capture.initializer_node,
            &capture.binding,
            &capture.control,
            &capture.body,
            &[],
            &[],
            &[],
        )
        .map_err(|error| error.to_string())?;
        (source_text, module, 0usize)
    } else if typed_while_local_statements_capture {
        let capture = decode_typed_while_local_statements_capture(&capture_bytes)
            .map_err(|error| error.to_string())?;
        let source_text = capture.source;
        let mut sources = SourceManager::new();
        let source_id = sources
            .add(arguments.capture.clone(), source_text.clone())
            .map_err(|error| error.to_string())?;
        let (call_bindings, call_arguments) =
            if source_text.contains("JADREN_STAGE2_DYNAMIC_TYPED_CALL") {
                dynamic_while_call_metadata(&source_text, &capture.ast)?
            } else {
                (Vec::new(), Vec::new())
            };
        let same_module_definitions =
            if source_text.contains("JADREN_STAGE2_DYNAMIC_SAME_MODULE_DEFINITIONS") {
                dynamic_while_local_function_definitions(&source_text, &call_bindings)?
            } else {
                Vec::new()
            };
        let module = lower_typed_while_local_with_statements(
            &source_text,
            source_id,
            &capture.ast,
            &[],
            capture.initializer_node,
            &capture.binding,
            &capture.control,
            &capture.body,
            &call_bindings,
            &[],
            &call_arguments,
        )
        .map_err(|error| error.to_string())?;
        let module = materialize_typed_local_function_definitions(
            &source_text,
            source_id,
            module,
            &same_module_definitions,
        )
        .map_err(|error| error.to_string())?;
        (source_text, module, 0usize)
    } else if typed_while_local_capture {
        let capture =
            decode_typed_while_local_capture(&capture_bytes).map_err(|error| error.to_string())?;
        let source_text = capture.source;
        let mut sources = SourceManager::new();
        let source_id = sources
            .add(arguments.capture.clone(), source_text.clone())
            .map_err(|error| error.to_string())?;
        let module = lower_typed_while_local(
            &source_text,
            source_id,
            &capture.ast,
            &[],
            capture.initializer_node,
            &capture.binding,
            &capture.control,
            &capture.body,
            &[],
            &[],
            &[],
        )
        .map_err(|error| error.to_string())?;
        (source_text, module, 0usize)
    } else if typed_if_value_statement_tail_capture {
        let capture = decode_typed_if_value_statement_tail_capture(&capture_bytes)
            .map_err(|error| error.to_string())?;
        let source_text = capture.source;
        let mut sources = SourceManager::new();
        let source_id = sources
            .add(arguments.capture.clone(), source_text.clone())
            .map_err(|error| error.to_string())?;
        let module = lower_typed_if_value_to_local_with_statements(
            &source_text,
            source_id,
            &capture.ast,
            &[],
            &capture.control,
            Some(&capture.continuation_binding),
            &capture.statements,
            &[],
            &[],
            &[],
        )
        .map_err(|error| error.to_string())?;
        (source_text, module, 0usize)
    } else if typed_if_value_local_assignment_capture {
        let capture = decode_typed_if_value_local_assignment_capture(&capture_bytes)
            .map_err(|error| error.to_string())?;
        let source_text = capture.source;
        let mut sources = SourceManager::new();
        let source_id = sources
            .add(arguments.capture.clone(), source_text.clone())
            .map_err(|error| error.to_string())?;
        let module = lower_typed_if_value_to_local_with_assignment(
            &source_text,
            source_id,
            &capture.ast,
            &[],
            &capture.control,
            Some(&capture.continuation_binding),
            capture.continuation_node,
            &[],
            &[],
            &[],
        )
        .map_err(|error| error.to_string())?;
        (source_text, module, 0usize)
    } else if typed_if_value_continuation_capture {
        let capture = decode_typed_if_value_continuation_capture(&capture_bytes)
            .map_err(|error| error.to_string())?;
        let source_text = capture.source;
        let mut sources = SourceManager::new();
        let source_id = sources
            .add(arguments.capture.clone(), source_text.clone())
            .map_err(|error| error.to_string())?;
        let module = lower_typed_if_value_with_continuation(
            &source_text,
            source_id,
            &capture.ast,
            &[],
            &capture.control,
            Some(&capture.continuation_binding),
            capture.continuation_node,
            &[],
            &[],
            &[],
        )
        .map_err(|error| error.to_string())?;
        (source_text, module, 0usize)
    } else if typed_if_capture {
        let capture =
            decode_typed_if_return_capture(&capture_bytes).map_err(|error| error.to_string())?;
        let source_text = capture.source;
        let mut sources = SourceManager::new();
        let source_id = sources
            .add(arguments.capture.clone(), source_text.clone())
            .map_err(|error| error.to_string())?;
        let module = lower_typed_if_return(
            &source_text,
            source_id,
            &capture.ast,
            &[],
            &capture.control,
            &[],
            &[],
            &[],
        )
        .map_err(|error| error.to_string())?;
        (source_text, module, 0usize)
    } else if typed_capture {
        let capture =
            decode_typed_statement_capture(&capture_bytes).map_err(|error| error.to_string())?;
        let source_text = capture.source;
        let mut sources = SourceManager::new();
        let source_id = sources
            .add(arguments.capture.clone(), source_text.clone())
            .map_err(|error| error.to_string())?;
        let module = lower_typed_statement_sequence(
            &source_text,
            source_id,
            &capture.ast,
            &[],
            &capture.statements,
        )
        .map_err(|error| error.to_string())?;
        (source_text, module, 0usize)
    } else {
        let capture = decode_stage2_capture(&capture_bytes).map_err(|error| error.to_string())?;
        let source_text = capture.source.clone();
        let mut sources = SourceManager::new();
        let source_id = sources
            .add(arguments.capture.clone(), source_text.clone())
            .map_err(|error| error.to_string())?;
        let module = import_stage2_jir(&source_text, source_id, capture.summary, &capture.records)
            .map_err(|error| error.to_string())?;
        (source_text, module, capture.records.len())
    };
    let constant_instructions = module
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .filter(|instruction| {
            matches!(
                instruction.kind,
                jadren_jir::InstructionKind::Constant(jadren_jir::Constant::Integer { .. })
            )
        })
        .count();
    let parameter_functions = module
        .functions
        .iter()
        .filter(|function| function.parameters.len() == 1)
        .count();
    let binary_adds = module
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .filter(|instruction| {
            matches!(
                instruction.kind,
                jadren_jir::InstructionKind::Binary {
                    op: jadren_jir::BinaryOp::Add,
                    ..
                }
            )
        })
        .count();
    let binary_subtracts = module
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .filter(|instruction| {
            matches!(
                instruction.kind,
                jadren_jir::InstructionKind::Binary {
                    op: jadren_jir::BinaryOp::Subtract,
                    ..
                }
            )
        })
        .count();
    let binary_multiplies = module
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .filter(|instruction| {
            matches!(
                instruction.kind,
                jadren_jir::InstructionKind::Binary {
                    op: jadren_jir::BinaryOp::Multiply,
                    ..
                }
            )
        })
        .count();
    let binary_chain_functions = module
        .functions
        .iter()
        .filter(|function| {
            function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .filter(|instruction| {
                    matches!(instruction.kind, jadren_jir::InstructionKind::Binary { .. })
                })
                .count()
                == 2
        })
        .count();
    let long_binary_chain_functions = module
        .functions
        .iter()
        .filter(|function| {
            function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .filter(|instruction| {
                    matches!(instruction.kind, jadren_jir::InstructionKind::Binary { .. })
                })
                .count()
                == 3
        })
        .count();
    let expression_plan_functions = module
        .functions
        .iter()
        .filter(|function| {
            function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .filter(|instruction| {
                    matches!(instruction.kind, jadren_jir::InstructionKind::Binary { .. })
                })
                .count()
                == 4
        })
        .count();
    let grouped_binary_chain_functions = module
        .functions
        .iter()
        .filter(|function| {
            let binary_instructions = function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .filter(|instruction| {
                    matches!(instruction.kind, jadren_jir::InstructionKind::Binary { .. })
                })
                .collect::<Vec<_>>();
            binary_instructions.len() == 2
                && binary_instructions.iter().any(|instruction| {
                    instruction.span.is_some_and(|span| {
                        source_text[span.start..span.end].trim().starts_with('(')
                            && source_text[span.start..span.end].trim().ends_with(')')
                    })
                })
        })
        .count();
    let grouped_expression_plan_functions = module
        .functions
        .iter()
        .filter(|function| {
            let binary_instructions = function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .filter(|instruction| {
                    matches!(instruction.kind, jadren_jir::InstructionKind::Binary { .. })
                })
                .collect::<Vec<_>>();
            (binary_instructions.len() == 3 || binary_instructions.len() == 4)
                && binary_instructions.iter().any(|instruction| {
                    instruction.span.is_some_and(|span| {
                        source_text[span.start..span.end].trim().starts_with('(')
                            && source_text[span.start..span.end].trim().ends_with(')')
                    })
                })
        })
        .count();
    let streaming_expression_plan_functions = module
        .functions
        .iter()
        .filter(|function| {
            let binary_count = function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .filter(|instruction| {
                    matches!(instruction.kind, jadren_jir::InstructionKind::Binary { .. })
                })
                .count();
            (5..=16).contains(&binary_count)
        })
        .count();
    let grouped_streaming_expression_plan_functions = module
        .functions
        .iter()
        .filter(|function| {
            let binary_instructions = function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .filter(|instruction| {
                    matches!(instruction.kind, jadren_jir::InstructionKind::Binary { .. })
                })
                .collect::<Vec<_>>();
            (5..=16).contains(&binary_instructions.len())
                && binary_instructions.iter().any(|instruction| {
                    instruction.span.is_some_and(|span| {
                        source_text[span.start..span.end].trim().starts_with('(')
                            && source_text[span.start..span.end].trim().ends_with(')')
                    })
                })
        })
        .count();
    let multi_group_streaming_expression_plan_functions = module
        .functions
        .iter()
        .filter(|function| {
            let binary_instructions = function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .filter(|instruction| {
                    matches!(instruction.kind, jadren_jir::InstructionKind::Binary { .. })
                })
                .collect::<Vec<_>>();
            (5..=16).contains(&binary_instructions.len())
                && binary_instructions
                    .iter()
                    .filter(|instruction| {
                        instruction.span.is_some_and(|span| {
                            source_text[span.start..span.end].trim().starts_with('(')
                                && source_text[span.start..span.end].trim().ends_with(')')
                        })
                    })
                    .count()
                    >= 2
        })
        .count();
    let nested_streaming_expression_plan_functions = module
        .functions
        .iter()
        .filter(|function| {
            let binary_instructions = function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .filter(|instruction| {
                    matches!(instruction.kind, jadren_jir::InstructionKind::Binary { .. })
                })
                .collect::<Vec<_>>();
            (5..=16).contains(&binary_instructions.len())
                && binary_instructions.iter().any(|instruction| {
                    instruction.span.is_some_and(|span| {
                        source_text[span.start..span.end].trim().starts_with("((")
                            && source_text[span.start..span.end].trim().ends_with("))")
                    })
                })
        })
        .count();

    create_parent(&arguments.object)?;
    let context = Context::create();
    let lowering_config = if cfg!(windows) {
        TypeLoweringConfig::x86_64_windows_msvc()
    } else {
        TypeLoweringConfig::x86_64_linux_gnu()
    };
    let (backend_summary, object) = lower_to_object_with_summary(
        &context,
        &module,
        "jadren_selfhost_stage2_driver",
        &lowering_config,
        &ObjectOptions::x86_64_baseline_release(),
    )
    .map_err(|error| error.to_string())?;
    write_object(&arguments.object, &object).map_err(|error| error.to_string())?;

    #[cfg(windows)]
    let mut link_summary: Option<(u64, u64)> = None;
    #[cfg(not(windows))]
    let link_summary: Option<(u64, u64)> = None;
    if let Some(executable) = &arguments.executable {
        #[cfg(not(windows))]
        {
            let _ = executable;
            return Err("--executable is currently supported only on Windows".to_owned());
        }
        #[cfg(windows)]
        {
            create_parent(executable)?;
            link_windows_executable(
                executable,
                std::slice::from_ref(&arguments.object),
                &WindowsLinkOptions {
                    entry_symbol: arguments.entry.clone(),
                    ..WindowsLinkOptions::default()
                },
            )
            .map_err(|error| error.to_string())?;
            let executable_bytes = std::fs::metadata(executable)
                .map_err(|error| format!("cannot stat linked executable: {error}"))?
                .len();
            link_summary = Some((executable_bytes, 1_u64));
        }
    }

    println!(
        "stage2-backend pass functions={} blocks={} instructions={} status_flags={} object_bytes={}",
        backend_summary.module_functions,
        backend_summary.module_blocks,
        backend_summary.module_instructions,
        backend_summary.backend_status_flags,
        backend_summary.object_bytes,
    );
    if let Some((executable_bytes, status_flags)) = link_summary {
        println!(
            "stage2-link pass executable_bytes={} status_flags={}",
            executable_bytes, status_flags
        );
    }
    println!(
        "stage2-driver pass functions={} records={} constants={} parameter_functions={} binary_adds={} binary_subtracts={} binary_multiplies={} binary_chain_functions={} long_binary_chain_functions={} expression_plan_functions={} streaming_expression_plan_functions={} grouped_binary_chain_functions={} grouped_expression_plan_functions={} grouped_streaming_expression_plan_functions={} multi_group_streaming_expression_plan_functions={} nested_streaming_expression_plan_functions={} object_bytes={} entry={}",
        module.functions.len(),
        capture_record_count,
        constant_instructions,
        parameter_functions,
        binary_adds,
        binary_subtracts,
        binary_multiplies,
        binary_chain_functions,
        long_binary_chain_functions,
        expression_plan_functions,
        streaming_expression_plan_functions,
        grouped_binary_chain_functions,
        grouped_expression_plan_functions,
        grouped_streaming_expression_plan_functions,
        multi_group_streaming_expression_plan_functions,
        nested_streaming_expression_plan_functions,
        object.len(),
        arguments.entry
    );
    let internal_definitions = module
        .functions
        .iter()
        .filter(|function| function.linkage == jadren_jir::Linkage::Internal)
        .count();
    let imported_functions = module
        .functions
        .iter()
        .filter(|function| function.linkage == jadren_jir::Linkage::Import)
        .count();
    println!(
        "stage2-definitions pass internal={} imports={}",
        internal_definitions, imported_functions
    );
    Ok(())
}

fn run_capture_command() -> Result<(), String> {
    let mut values = std::env::args_os().skip(2);
    let provider = values.next().map(PathBuf::from).ok_or_else(capture_usage)?;
    let producer = values.next().map(PathBuf::from).ok_or_else(capture_usage)?;
    let source = values.next().map(PathBuf::from).ok_or_else(capture_usage)?;
    let output = values.next().map(PathBuf::from).ok_or_else(capture_usage)?;
    if values.next().is_some() {
        return Err(capture_usage());
    }
    capture::capture_stage2(&provider, &producer, &source, &output)
}

struct Arguments {
    capture: PathBuf,
    object: PathBuf,
    executable: Option<PathBuf>,
    entry: String,
}

fn parse_arguments() -> Result<Arguments, String> {
    let mut values = std::env::args_os().skip(1);
    let capture = values.next().map(PathBuf::from).ok_or_else(usage)?;
    let object = values.next().map(PathBuf::from).ok_or_else(usage)?;
    let mut executable = None;
    let mut entry = "main".to_owned();
    while let Some(argument) = values.next() {
        let argument = argument
            .into_string()
            .map_err(|_| "driver option is not valid UTF-8".to_owned())?;
        match argument.as_str() {
            "--executable" => {
                if executable.is_some() {
                    return Err("--executable may be supplied only once".to_owned());
                }
                executable = Some(values.next().map(PathBuf::from).ok_or_else(usage)?);
            }
            "--entry" => {
                entry = values
                    .next()
                    .ok_or_else(usage)?
                    .into_string()
                    .map_err(|_| "entry symbol is not valid UTF-8".to_owned())?;
            }
            _ => return Err(format!("unknown driver option `{argument}`\n{}", usage())),
        }
    }
    Ok(Arguments {
        capture,
        object,
        executable,
        entry,
    })
}

fn create_parent(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create `{}`: {error}", parent.display()))
}

fn usage() -> String {
    "usage: jadren-selfhost-driver <capture.bin> <output.obj> [--executable <output.exe>] [--entry <symbol>]".to_owned()
}

fn capture_usage() -> String {
    "usage: jadren-selfhost-driver capture <provider.dll> <producer.dll> <source.jdn> <capture.bin>"
        .to_owned()
}
