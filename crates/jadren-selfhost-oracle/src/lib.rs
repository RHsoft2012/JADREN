//! Independent deterministic semantics oracle for the self-hosting lexer slice.

use jadren_determinism::StableHasher;
use jadren_selfhost_api::{
    API_SCHEMA, API_VERSION, DiagnosticValue, ExpressionPrecedenceHeader, FrontendApiLease,
    FrontendApiRegistry, FrontendApiV1, FrontendTokenInfoApiV1, TokenCounts, TokenInfo, TokenSpan,
    TypedCallBindingHeader, TypedCallCandidateHeader, TypedExpressionHeader,
    TypedNameBindingHeader, TypedRegionNameBindingHeader, TypedScopedNameBindingHeader,
};

/// One input row from the versioned self-hosting corpus.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CorpusCase {
    /// Byte value and expected compact lexer class.
    Byte { value: u8, expected_class: u8 },
    /// Token span values, which must be preserved exactly.
    Span {
        start: u64,
        end: u64,
        expected_start: u64,
        expected_end: u64,
    },
    /// Identifier/digit scan over a caller-provided ASCII input range.
    Scan {
        input: String,
        start: u64,
        end: u64,
        expected_start: u64,
        expected_end: u64,
    },
    /// Next token span after ASCII whitespace skipping.
    Token {
        input: String,
        start: u64,
        end: u64,
        expected_start: u64,
        expected_end: u64,
    },
    /// Next token kind and span after ASCII whitespace skipping.
    TokenInfo {
        input: String,
        start: u64,
        end: u64,
        expected_kind: u8,
        expected_start: u64,
        expected_end: u64,
    },
    /// Allocation-free token stream count over a caller-provided ASCII range.
    Count {
        input: String,
        start: u64,
        end: u64,
        expected_identifiers: u64,
        expected_numbers: u64,
        expected_symbols: u64,
        expected_total: u64,
    },
    /// Builtin literal typing over one bounded ASCII source range.
    TypedLiteral {
        input: String,
        start: u64,
        end: u64,
        expected_expression_kind: u8,
        expected_type_kind: u8,
        expected_start: u64,
        expected_end: u64,
        expected_depth: u64,
    },
    /// Typed result for one binary expression whose operands are builtin
    /// literals.
    TypedBinary {
        input: String,
        start: u64,
        end: u64,
        expected_expression_kind: u8,
        expected_type_kind: u8,
        expected_start: u64,
        expected_end: u64,
        expected_depth: u64,
    },
    /// Typed result for one explicit numeric cast over a builtin literal.
    TypedCast {
        input: String,
        start: u64,
        end: u64,
        expected_expression_kind: u8,
        expected_type_kind: u8,
        expected_start: u64,
        expected_end: u64,
        expected_depth: u64,
    },
    /// Typed result for one unary expression whose operand is a builtin
    /// literal.
    TypedUnary {
        input: String,
        start: u64,
        end: u64,
        expected_expression_kind: u8,
        expected_type_kind: u8,
        expected_start: u64,
        expected_end: u64,
        expected_depth: u64,
    },
    /// Syntax-only call header over one explicit expression range.
    Call {
        input: String,
        start: u64,
        end: u64,
        expected_count: usize,
        expected_callee_start: u64,
        expected_callee_end: u64,
        expected_open_start: u64,
        expected_open_end: u64,
        expected_close_start: u64,
        expected_close_end: u64,
        expected_argument_count: u64,
        expected_depth: u64,
    },
    /// Typed result for identifier tokens matched against one caller-owned
    /// symbol-table binding.
    TypedName {
        input: String,
        start: u64,
        end: u64,
        binding_start: u64,
        binding_end: u64,
        binding_type_kind: u8,
        expected_count: usize,
        expected_expression_kind: u8,
        expected_type_kind: u8,
        expected_start: u64,
        expected_end: u64,
        expected_depth: u64,
    },
    /// Typed result for a call matched against a caller-owned function
    /// signature binding.
    TypedCall {
        input: String,
        start: u64,
        end: u64,
        binding_start: u64,
        binding_end: u64,
        binding_parameter_count: u64,
        binding_return_type_kind: u8,
        expected_count: usize,
        expected_expression_kind: u8,
        expected_type_kind: u8,
        expected_start: u64,
        expected_end: u64,
        expected_depth: u64,
    },
    /// Typed result for a call selected from caller-owned concrete and generic
    /// candidates. The bounded resolver prefers the lowest generic count and
    /// rejects ties as ambiguous.
    TypedCallResolve {
        input: String,
        start: u64,
        end: u64,
        candidates: Vec<TypedCallCandidateHeader>,
        expected_count: usize,
        expected_expression_kind: u8,
        expected_type_kind: u8,
        expected_start: u64,
        expected_end: u64,
        expected_depth: u64,
    },
    /// Typed result for an identifier selected by one caller-owned
    /// visibility region and lexical depth.
    TypedRegionName {
        input: String,
        start: u64,
        end: u64,
        binding_start: u64,
        binding_end: u64,
        region_start: u64,
        region_end: u64,
        binding_type_kind: u8,
        binding_scope_depth: u64,
        expected_count: usize,
        expected_expression_kind: u8,
        expected_type_kind: u8,
        expected_start: u64,
        expected_end: u64,
        expected_depth: u64,
    },
    /// Diagnostic payload values, which must be preserved exactly.
    Diagnostic {
        code: u16,
        severity: u8,
        expected_code: u16,
        expected_severity: u8,
    },
}

/// Rust reference for the caller-owned syntax-only call header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExpressionCallHeader {
    pub callee_start: u64,
    pub callee_end: u64,
    pub open_start: u64,
    pub open_end: u64,
    pub close_start: u64,
    pub close_end: u64,
    pub argument_count: u64,
    pub depth: u64,
}

/// Parsed deterministic corpus.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Corpus {
    /// Corpus rows in file order.
    pub cases: Vec<CorpusCase>,
}

/// Summary emitted by the oracle executable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleReport {
    /// Frontend API schema implemented by this oracle.
    pub api_schema: &'static str,
    /// Frontend API version implemented by this oracle.
    pub api_version: u32,
    /// Lifetime policy used while invoking the table.
    pub api_lifetime: &'static str,
    /// Synchronization policy used to obtain the table.
    pub api_registry: &'static str,
    /// Number of byte rows.
    pub byte_cases: usize,
    /// Number of span rows.
    pub span_cases: usize,
    /// Number of span rows executed through the frontend API.
    pub api_span_cases: usize,
    /// Number of identifier scan rows.
    pub scan_cases: usize,
    /// Number of identifier scan rows executed through the frontend API.
    pub api_scan_cases: usize,
    /// Number of next-token scan rows.
    pub token_cases: usize,
    /// Number of token kind and span rows.
    pub token_info_cases: usize,
    /// Number of token kind and span rows executed through the additive API extension.
    pub api_token_info_cases: usize,
    /// Number of token stream count rows.
    pub count_cases: usize,
    /// Number of builtin typed-literal rows.
    pub typed_literal_cases: usize,
    /// Number of builtin typed-binary rows.
    pub typed_binary_cases: usize,
    /// Number of explicit numeric-cast rows.
    pub typed_cast_cases: usize,
    /// Number of builtin typed-unary rows.
    pub typed_unary_cases: usize,
    /// Number of syntax-only call-header rows.
    pub call_cases: usize,
    /// Number of caller-owned typed-name rows.
    pub typed_name_cases: usize,
    /// Number of caller-owned typed-call rows.
    pub typed_call_cases: usize,
    /// Number of bounded overload/generic typed-call resolver rows.
    pub typed_call_resolve_cases: usize,
    /// Number of region-aware caller-owned typed-name rows.
    pub typed_region_name_cases: usize,
    /// Number of diagnostic rows.
    pub diagnostic_cases: usize,
    /// Number of diagnostic rows executed through the frontend API.
    pub api_diagnostic_cases: usize,
    /// Stable fingerprint of the normalized corpus and oracle result.
    pub fingerprint: String,
}

fn split_corpus_fields(line: &str) -> Vec<&str> {
    let mut fields = Vec::new();
    let mut field_start = 0;
    let mut delimiter_depth = 0_u32;
    let mut quoted = false;
    let mut escaped = false;
    for (index, byte) in line.bytes().enumerate() {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
            continue;
        }
        match byte {
            b'"' => quoted = true,
            b'(' | b'[' => delimiter_depth += 1,
            b')' | b']' => delimiter_depth = delimiter_depth.saturating_sub(1),
            b',' if delimiter_depth == 0
                || !line[index + 1..]
                    .bytes()
                    .any(|next| next == b')' || next == b']') =>
            {
                fields.push(line[field_start..index].trim());
                field_start = index + 1;
            }
            _ => {}
        }
    }
    fields.push(line[field_start..].trim());
    fields
}

/// Parses the deliberately small, dependency-free corpus format.
pub fn parse_corpus(input: &str) -> Result<Corpus, String> {
    let mut cases = Vec::new();
    let mut saw_header = false;

    for (line_number, raw_line) in input.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if !saw_header {
            if line != "jadren-selfhost-corpus-0.1" {
                return Err(format!("line {}: expected corpus header", line_number + 1));
            }
            saw_header = true;
            continue;
        }

        let fields = split_corpus_fields(line);
        let case = match fields.first().copied() {
            Some("byte") if fields.len() == 3 => CorpusCase::Byte {
                value: parse_u8(fields[1], line_number)?,
                expected_class: parse_u8(fields[2], line_number)?,
            },
            Some("span") if fields.len() == 5 => CorpusCase::Span {
                start: parse_u64(fields[1], line_number)?,
                end: parse_u64(fields[2], line_number)?,
                expected_start: parse_u64(fields[3], line_number)?,
                expected_end: parse_u64(fields[4], line_number)?,
            },
            Some("scan") if fields.len() == 6 => {
                let input = fields[1].to_owned();
                if !input.is_ascii() {
                    return Err(format!(
                        "line {}: scan input must be ASCII",
                        line_number + 1
                    ));
                }
                let start = parse_u64(fields[2], line_number)?;
                let end = parse_u64(fields[3], line_number)?;
                if start > end || end > input.len() as u64 {
                    return Err(format!(
                        "line {}: scan range [{start}, {end}) exceeds ASCII input length {}",
                        line_number + 1,
                        input.len()
                    ));
                }
                CorpusCase::Scan {
                    input,
                    start,
                    end,
                    expected_start: parse_u64(fields[4], line_number)?,
                    expected_end: parse_u64(fields[5], line_number)?,
                }
            }
            Some("token") if fields.len() == 6 => {
                let input = fields[1].to_owned();
                if !input.is_ascii() {
                    return Err(format!(
                        "line {}: token input must be ASCII",
                        line_number + 1
                    ));
                }
                let start = parse_u64(fields[2], line_number)?;
                let end = parse_u64(fields[3], line_number)?;
                if start > end || end > input.len() as u64 {
                    return Err(format!(
                        "line {}: token range [{start}, {end}) exceeds ASCII input length {}",
                        line_number + 1,
                        input.len()
                    ));
                }
                CorpusCase::Token {
                    input,
                    start,
                    end,
                    expected_start: parse_u64(fields[4], line_number)?,
                    expected_end: parse_u64(fields[5], line_number)?,
                }
            }
            Some("info") if fields.len() == 7 => {
                let input = fields[1].to_owned();
                if !input.is_ascii() {
                    return Err(format!(
                        "line {}: token-info input must be ASCII",
                        line_number + 1
                    ));
                }
                let start = parse_u64(fields[2], line_number)?;
                let end = parse_u64(fields[3], line_number)?;
                if start > end || end > input.len() as u64 {
                    return Err(format!(
                        "line {}: token-info range [{start}, {end}) exceeds ASCII input length {}",
                        line_number + 1,
                        input.len()
                    ));
                }
                CorpusCase::TokenInfo {
                    input,
                    start,
                    end,
                    expected_kind: parse_u8(fields[4], line_number)?,
                    expected_start: parse_u64(fields[5], line_number)?,
                    expected_end: parse_u64(fields[6], line_number)?,
                }
            }
            Some("count") if fields.len() == 8 => {
                let input = fields[1].to_owned();
                if !input.is_ascii() {
                    return Err(format!(
                        "line {}: count input must be ASCII",
                        line_number + 1
                    ));
                }
                let start = parse_u64(fields[2], line_number)?;
                let end = parse_u64(fields[3], line_number)?;
                if start > end || end > input.len() as u64 {
                    return Err(format!(
                        "line {}: count range [{start}, {end}) exceeds ASCII input length {}",
                        line_number + 1,
                        input.len()
                    ));
                }
                CorpusCase::Count {
                    input,
                    start,
                    end,
                    expected_identifiers: parse_u64(fields[4], line_number)?,
                    expected_numbers: parse_u64(fields[5], line_number)?,
                    expected_symbols: parse_u64(fields[6], line_number)?,
                    expected_total: parse_u64(fields[7], line_number)?,
                }
            }
            Some("typed") if fields.len() == 9 => {
                let input = fields[1].to_owned();
                if !input.is_ascii() {
                    return Err(format!(
                        "line {}: typed-literal input must be ASCII",
                        line_number + 1
                    ));
                }
                let start = parse_u64(fields[2], line_number)?;
                let end = parse_u64(fields[3], line_number)?;
                if start > end || end > input.len() as u64 {
                    return Err(format!(
                        "line {}: typed-literal range [{start}, {end}) exceeds ASCII input length {}",
                        line_number + 1,
                        input.len()
                    ));
                }
                CorpusCase::TypedLiteral {
                    input,
                    start,
                    end,
                    expected_expression_kind: parse_u8(fields[4], line_number)?,
                    expected_type_kind: parse_u8(fields[5], line_number)?,
                    expected_start: parse_u64(fields[6], line_number)?,
                    expected_end: parse_u64(fields[7], line_number)?,
                    expected_depth: parse_u64(fields[8], line_number)?,
                }
            }
            Some("typed-binary") if fields.len() == 9 => {
                let input = fields[1].to_owned();
                if !input.is_ascii() {
                    return Err(format!(
                        "line {}: typed-binary input must be ASCII",
                        line_number + 1
                    ));
                }
                let start = parse_u64(fields[2], line_number)?;
                let end = parse_u64(fields[3], line_number)?;
                if start > end || end > input.len() as u64 {
                    return Err(format!(
                        "line {}: typed-binary range [{start}, {end}) exceeds ASCII input length {}",
                        line_number + 1,
                        input.len()
                    ));
                }
                CorpusCase::TypedBinary {
                    input,
                    start,
                    end,
                    expected_expression_kind: parse_u8(fields[4], line_number)?,
                    expected_type_kind: parse_u8(fields[5], line_number)?,
                    expected_start: parse_u64(fields[6], line_number)?,
                    expected_end: parse_u64(fields[7], line_number)?,
                    expected_depth: parse_u64(fields[8], line_number)?,
                }
            }
            Some("typed-cast") if fields.len() == 9 => {
                let input = fields[1].to_owned();
                if !input.is_ascii() {
                    return Err(format!(
                        "line {}: typed-cast input must be ASCII",
                        line_number + 1
                    ));
                }
                let start = parse_u64(fields[2], line_number)?;
                let end = parse_u64(fields[3], line_number)?;
                if start > end || end > input.len() as u64 {
                    return Err(format!(
                        "line {}: typed-cast range [{start}, {end}) exceeds ASCII input length {}",
                        line_number + 1,
                        input.len()
                    ));
                }
                CorpusCase::TypedCast {
                    input,
                    start,
                    end,
                    expected_expression_kind: parse_u8(fields[4], line_number)?,
                    expected_type_kind: parse_u8(fields[5], line_number)?,
                    expected_start: parse_u64(fields[6], line_number)?,
                    expected_end: parse_u64(fields[7], line_number)?,
                    expected_depth: parse_u64(fields[8], line_number)?,
                }
            }
            Some("typed-unary") if fields.len() == 9 => {
                let input = fields[1].to_owned();
                if !input.is_ascii() {
                    return Err(format!(
                        "line {}: typed-unary input must be ASCII",
                        line_number + 1
                    ));
                }
                let start = parse_u64(fields[2], line_number)?;
                let end = parse_u64(fields[3], line_number)?;
                if start > end || end > input.len() as u64 {
                    return Err(format!(
                        "line {}: typed-unary range [{start}, {end}) exceeds ASCII input length {}",
                        line_number + 1,
                        input.len()
                    ));
                }
                CorpusCase::TypedUnary {
                    input,
                    start,
                    end,
                    expected_expression_kind: parse_u8(fields[4], line_number)?,
                    expected_type_kind: parse_u8(fields[5], line_number)?,
                    expected_start: parse_u64(fields[6], line_number)?,
                    expected_end: parse_u64(fields[7], line_number)?,
                    expected_depth: parse_u64(fields[8], line_number)?,
                }
            }
            Some("call") if fields.len() == 13 => {
                let input = fields[1].to_owned();
                if !input.is_ascii() {
                    return Err(format!(
                        "line {}: call input must be ASCII",
                        line_number + 1
                    ));
                }
                let start = parse_u64(fields[2], line_number)?;
                let end = parse_u64(fields[3], line_number)?;
                if start > end || end > input.len() as u64 {
                    return Err(format!(
                        "line {}: call range [{start}, {end}) exceeds ASCII input length {}",
                        line_number + 1,
                        input.len()
                    ));
                }
                CorpusCase::Call {
                    input,
                    start,
                    end,
                    expected_count: parse_usize(fields[4], line_number)?,
                    expected_callee_start: parse_u64(fields[5], line_number)?,
                    expected_callee_end: parse_u64(fields[6], line_number)?,
                    expected_open_start: parse_u64(fields[7], line_number)?,
                    expected_open_end: parse_u64(fields[8], line_number)?,
                    expected_close_start: parse_u64(fields[9], line_number)?,
                    expected_close_end: parse_u64(fields[10], line_number)?,
                    expected_argument_count: parse_u64(fields[11], line_number)?,
                    expected_depth: parse_u64(fields[12], line_number)?,
                }
            }
            Some("typed-name") if fields.len() == 13 => {
                let input = fields[1].to_owned();
                if !input.is_ascii() {
                    return Err(format!(
                        "line {}: typed-name input must be ASCII",
                        line_number + 1
                    ));
                }
                let start = parse_u64(fields[2], line_number)?;
                let end = parse_u64(fields[3], line_number)?;
                let binding_start = parse_u64(fields[4], line_number)?;
                let binding_end = parse_u64(fields[5], line_number)?;
                if start > end || end > input.len() as u64 {
                    return Err(format!(
                        "line {}: typed-name range [{start}, {end}) exceeds ASCII input length {}",
                        line_number + 1,
                        input.len()
                    ));
                }
                if binding_start > binding_end || binding_end > input.len() as u64 {
                    return Err(format!(
                        "line {}: typed-name binding range [{binding_start}, {binding_end}) exceeds ASCII input length {}",
                        line_number + 1,
                        input.len()
                    ));
                }
                CorpusCase::TypedName {
                    input,
                    start,
                    end,
                    binding_start,
                    binding_end,
                    binding_type_kind: parse_u8(fields[6], line_number)?,
                    expected_count: usize::try_from(parse_u64(fields[7], line_number)?).map_err(
                        |_| {
                            format!(
                                "line {}: typed-name expected count overflows usize",
                                line_number + 1
                            )
                        },
                    )?,
                    expected_expression_kind: parse_u8(fields[8], line_number)?,
                    expected_type_kind: parse_u8(fields[9], line_number)?,
                    expected_start: parse_u64(fields[10], line_number)?,
                    expected_end: parse_u64(fields[11], line_number)?,
                    expected_depth: parse_u64(fields[12], line_number)?,
                }
            }
            Some("typed-call") if fields.len() == 14 => {
                let input = fields[1].to_owned();
                if !input.is_ascii() {
                    return Err(format!(
                        "line {}: typed-call input must be ASCII",
                        line_number + 1
                    ));
                }
                let start = parse_u64(fields[2], line_number)?;
                let end = parse_u64(fields[3], line_number)?;
                let binding_start = parse_u64(fields[4], line_number)?;
                let binding_end = parse_u64(fields[5], line_number)?;
                if start > end || end > input.len() as u64 {
                    return Err(format!(
                        "line {}: typed-call range [{start}, {end}) exceeds ASCII input length {}",
                        line_number + 1,
                        input.len()
                    ));
                }
                if binding_start > binding_end || binding_end > input.len() as u64 {
                    return Err(format!(
                        "line {}: typed-call binding range [{binding_start}, {binding_end}) exceeds ASCII input length {}",
                        line_number + 1,
                        input.len()
                    ));
                }
                CorpusCase::TypedCall {
                    input,
                    start,
                    end,
                    binding_start,
                    binding_end,
                    binding_parameter_count: parse_u64(fields[6], line_number)?,
                    binding_return_type_kind: parse_u8(fields[7], line_number)?,
                    expected_count: parse_usize(fields[8], line_number)?,
                    expected_expression_kind: parse_u8(fields[9], line_number)?,
                    expected_type_kind: parse_u8(fields[10], line_number)?,
                    expected_start: parse_u64(fields[11], line_number)?,
                    expected_end: parse_u64(fields[12], line_number)?,
                    expected_depth: parse_u64(fields[13], line_number)?,
                }
            }
            Some("typed-call-resolve") if fields.len() == 11 => {
                let input = fields[1].to_owned();
                if !input.is_ascii() {
                    return Err(format!(
                        "line {}: typed-call-resolve input must be ASCII",
                        line_number + 1
                    ));
                }
                let start = parse_u64(fields[2], line_number)?;
                let end = parse_u64(fields[3], line_number)?;
                if start > end || end > input.len() as u64 {
                    return Err(format!(
                        "line {}: typed-call-resolve range [{start}, {end}) exceeds ASCII input length {}",
                        line_number + 1,
                        input.len()
                    ));
                }
                let candidates =
                    parse_typed_call_candidates(fields[4], input.len() as u64, line_number)?;
                CorpusCase::TypedCallResolve {
                    input,
                    start,
                    end,
                    candidates,
                    expected_count: parse_usize(fields[5], line_number)?,
                    expected_expression_kind: parse_u8(fields[6], line_number)?,
                    expected_type_kind: parse_u8(fields[7], line_number)?,
                    expected_start: parse_u64(fields[8], line_number)?,
                    expected_end: parse_u64(fields[9], line_number)?,
                    expected_depth: parse_u64(fields[10], line_number)?,
                }
            }
            Some("typed-region-name") if fields.len() == 16 => {
                let input = fields[1].to_owned();
                if !input.is_ascii() {
                    return Err(format!(
                        "line {}: typed-region-name input must be ASCII",
                        line_number + 1
                    ));
                }
                let start = parse_u64(fields[2], line_number)?;
                let end = parse_u64(fields[3], line_number)?;
                let binding_start = parse_u64(fields[4], line_number)?;
                let binding_end = parse_u64(fields[5], line_number)?;
                let region_start = parse_u64(fields[6], line_number)?;
                let region_end = parse_u64(fields[7], line_number)?;
                if start > end || end > input.len() as u64 {
                    return Err(format!(
                        "line {}: typed-region-name range [{start}, {end}) exceeds ASCII input length {}",
                        line_number + 1,
                        input.len()
                    ));
                }
                if binding_start > binding_end || binding_end > input.len() as u64 {
                    return Err(format!(
                        "line {}: typed-region-name binding range [{binding_start}, {binding_end}) exceeds ASCII input length {}",
                        line_number + 1,
                        input.len()
                    ));
                }
                if region_start > region_end || region_end > input.len() as u64 {
                    return Err(format!(
                        "line {}: typed-region-name region range [{region_start}, {region_end}) exceeds ASCII input length {}",
                        line_number + 1,
                        input.len()
                    ));
                }
                CorpusCase::TypedRegionName {
                    input,
                    start,
                    end,
                    binding_start,
                    binding_end,
                    region_start,
                    region_end,
                    binding_type_kind: parse_u8(fields[8], line_number)?,
                    binding_scope_depth: parse_u64(fields[9], line_number)?,
                    expected_count: usize::try_from(parse_u64(fields[10], line_number)?).map_err(
                        |_| {
                            format!(
                                "line {}: typed-region-name expected count overflows usize",
                                line_number + 1
                            )
                        },
                    )?,
                    expected_expression_kind: parse_u8(fields[11], line_number)?,
                    expected_type_kind: parse_u8(fields[12], line_number)?,
                    expected_start: parse_u64(fields[13], line_number)?,
                    expected_end: parse_u64(fields[14], line_number)?,
                    expected_depth: parse_u64(fields[15], line_number)?,
                }
            }
            Some("diagnostic") if fields.len() == 5 => CorpusCase::Diagnostic {
                code: parse_u16(fields[1], line_number)?,
                severity: parse_u8(fields[2], line_number)?,
                expected_code: parse_u16(fields[3], line_number)?,
                expected_severity: parse_u8(fields[4], line_number)?,
            },
            Some(kind) => {
                return Err(format!(
                    "line {}: invalid `{kind}` row or field count",
                    line_number + 1
                ));
            }
            None => return Err(format!("line {}: empty row", line_number + 1)),
        };
        cases.push(case);
    }

    if !saw_header {
        return Err("corpus header is missing".to_owned());
    }
    if cases.is_empty() {
        return Err("corpus has no cases".to_owned());
    }
    Ok(Corpus { cases })
}

/// Runs the independent Rust semantics oracle over every corpus row.
pub fn run_oracle(corpus: &Corpus) -> Result<OracleReport, String> {
    let mut byte_cases = 0;
    let mut span_cases = 0;
    let mut api_span_cases = 0;
    let mut scan_cases = 0;
    let mut api_scan_cases = 0;
    let mut token_cases = 0;
    let mut token_info_cases = 0;
    let mut api_token_info_cases = 0;
    let mut count_cases = 0;
    let mut typed_literal_cases = 0;
    let mut typed_binary_cases = 0;
    let mut typed_cast_cases = 0;
    let mut typed_unary_cases = 0;
    let mut call_cases = 0;
    let mut typed_name_cases = 0;
    let mut typed_call_cases = 0;
    let mut typed_call_resolve_cases = 0;
    let mut typed_region_name_cases = 0;
    let mut diagnostic_cases = 0;
    let mut api_diagnostic_cases = 0;
    let mut hasher = StableHasher::with_domain("jadren.selfhost.oracle.0.1");
    let registry = FrontendApiRegistry::new();
    registry
        .install(rust_oracle_api())
        .map_err(|error| format!("oracle API install failed: {error:?}"))?;
    let snapshot = registry
        .snapshot()
        .map_err(|error| format!("oracle API borrow failed: {error:?}"))?;
    let api = snapshot
        .borrow()
        .map_err(|error| format!("oracle API lease failed: {error:?}"))?;
    let token_info_snapshot = rust_oracle_token_info_api();
    let token_info_api = token_info_snapshot
        .borrow()
        .map_err(|error| format!("oracle TokenInfo API lease failed: {error:?}"))?;

    for case in &corpus.cases {
        match case {
            CorpusCase::Byte {
                value,
                expected_class,
            } => {
                byte_cases += 1;
                let actual = api.classify_byte(*value);
                if actual != *expected_class {
                    return Err(format!(
                        "byte {value} expected class {expected_class}, got {actual}"
                    ));
                }
                hasher.write_u64(u64::from(*value));
                hasher.write_u64(u64::from(actual));
            }
            CorpusCase::Span {
                start,
                end,
                expected_start,
                expected_end,
            } => {
                span_cases += 1;
                api_span_cases += 1;
                let TokenSpan {
                    start: actual_start,
                    end: actual_end,
                } = api.token_span(*start, *end);
                if (actual_start, actual_end) != (*expected_start, *expected_end) {
                    return Err(format!(
                        "span ({start}, {end}) expected ({expected_start}, {expected_end}), got ({actual_start}, {actual_end})"
                    ));
                }
                hasher.write_u64(*start);
                hasher.write_u64(*end);
                hasher.write_u64(actual_start);
                hasher.write_u64(actual_end);
            }
            CorpusCase::Scan {
                input,
                start,
                end,
                expected_start,
                expected_end,
            } => {
                scan_cases += 1;
                let (actual_start, actual_end) = scan_identifier(input.as_bytes(), *start, *end);
                if (actual_start, actual_end) != (*expected_start, *expected_end) {
                    return Err(format!(
                        "scan ({input:?}, {start}, {end}) expected ({expected_start}, {expected_end}), got ({actual_start}, {actual_end})"
                    ));
                }
                api_scan_cases += 1;
                let (api_start, api_end) =
                    scan_identifier_via_api(&api, input.as_bytes(), *start, *end);
                if (api_start, api_end) != (*expected_start, *expected_end) {
                    return Err(format!(
                        "API scan ({input:?}, {start}, {end}) expected ({expected_start}, {expected_end}), got ({api_start}, {api_end})"
                    ));
                }
                hasher.write_bytes(input.as_bytes());
                hasher.write_u64(*start);
                hasher.write_u64(*end);
                hasher.write_u64(actual_start);
                hasher.write_u64(actual_end);
                hasher.write_u64(api_start);
                hasher.write_u64(api_end);
            }
            CorpusCase::Diagnostic {
                code,
                severity,
                expected_code,
                expected_severity,
            } => {
                diagnostic_cases += 1;
                api_diagnostic_cases += 1;
                let DiagnosticValue {
                    code: actual_code,
                    severity: actual_severity,
                } = api.diagnostic(*code, *severity);
                if (actual_code, actual_severity) != (*expected_code, *expected_severity) {
                    return Err(format!(
                        "diagnostic ({code}, {severity}) expected ({expected_code}, {expected_severity}), got ({actual_code}, {actual_severity})"
                    ));
                }
                hasher.write_u64(u64::from(*code));
                hasher.write_u64(u64::from(*severity));
                hasher.write_u64(u64::from(actual_code));
                hasher.write_u64(u64::from(actual_severity));
            }
            CorpusCase::Token {
                input,
                start,
                end,
                expected_start,
                expected_end,
            } => {
                token_cases += 1;
                let (actual_start, actual_end) = scan_token(input.as_bytes(), *start, *end);
                if (actual_start, actual_end) != (*expected_start, *expected_end) {
                    return Err(format!(
                        "token ({input:?}, {start}, {end}) expected ({expected_start}, {expected_end}), got ({actual_start}, {actual_end})"
                    ));
                }
                hasher.write_bytes(input.as_bytes());
                hasher.write_u64(*start);
                hasher.write_u64(*end);
                hasher.write_u64(actual_start);
                hasher.write_u64(actual_end);
            }
            CorpusCase::TokenInfo {
                input,
                start,
                end,
                expected_kind,
                expected_start,
                expected_end,
            } => {
                token_info_cases += 1;
                let (actual_kind, actual_start, actual_end) =
                    scan_token_info(input.as_bytes(), *start, *end);
                if (actual_kind, actual_start, actual_end)
                    != (*expected_kind, *expected_start, *expected_end)
                {
                    return Err(format!(
                        "token-info ({input:?}, {start}, {end}) expected ({expected_kind}, {expected_start}, {expected_end}), got ({actual_kind}, {actual_start}, {actual_end})"
                    ));
                }
                api_token_info_cases += 1;
                let TokenInfo {
                    kind: api_kind,
                    start: api_start,
                    end: api_end,
                } = token_info_api.token_info(actual_kind, actual_start, actual_end);
                if (api_kind, api_start, api_end)
                    != (*expected_kind, *expected_start, *expected_end)
                {
                    return Err(format!(
                        "API token-info ({input:?}, {start}, {end}) expected ({expected_kind}, {expected_start}, {expected_end}), got ({api_kind}, {api_start}, {api_end})"
                    ));
                }
                hasher.write_bytes(input.as_bytes());
                hasher.write_u64(*start);
                hasher.write_u64(*end);
                hasher.write_u64(u64::from(actual_kind));
                hasher.write_u64(actual_start);
                hasher.write_u64(actual_end);
                hasher.write_u64(u64::from(api_kind));
                hasher.write_u64(api_start);
                hasher.write_u64(api_end);
            }
            CorpusCase::Count {
                input,
                start,
                end,
                expected_identifiers,
                expected_numbers,
                expected_symbols,
                expected_total,
            } => {
                count_cases += 1;
                let actual = count_tokens(input.as_bytes(), *start, *end);
                let expected = TokenCounts {
                    identifiers: *expected_identifiers,
                    numbers: *expected_numbers,
                    symbols: *expected_symbols,
                    total: *expected_total,
                };
                if actual != expected {
                    return Err(format!(
                        "count ({input:?}, {start}, {end}) expected ({expected_identifiers}, {expected_numbers}, {expected_symbols}, {expected_total}), got ({}, {}, {}, {})",
                        actual.identifiers, actual.numbers, actual.symbols, actual.total
                    ));
                }
                hasher.write_bytes(input.as_bytes());
                hasher.write_u64(*start);
                hasher.write_u64(*end);
                hasher.write_u64(actual.identifiers);
                hasher.write_u64(actual.numbers);
                hasher.write_u64(actual.symbols);
                hasher.write_u64(actual.total);
            }
            CorpusCase::TypedLiteral {
                input,
                start,
                end,
                expected_expression_kind,
                expected_type_kind,
                expected_start,
                expected_end,
                expected_depth,
            } => {
                typed_literal_cases += 1;
                let actual = infer_literal_types(input.as_bytes(), *start, *end);
                if actual.len() != 1 {
                    return Err(format!(
                        "typed literal ({input:?}, {start}, {end}) expected one record, got {}",
                        actual.len()
                    ));
                }
                let actual = actual[0];
                if (
                    actual.expression_kind,
                    actual.type_kind,
                    actual.start,
                    actual.end,
                    actual.depth,
                ) != (
                    *expected_expression_kind,
                    *expected_type_kind,
                    *expected_start,
                    *expected_end,
                    *expected_depth,
                ) {
                    return Err(format!(
                        "typed literal ({input:?}, {start}, {end}) expected ({expected_expression_kind}, {expected_type_kind}, {expected_start}, {expected_end}, {expected_depth}), got ({}, {}, {}, {}, {})",
                        actual.expression_kind,
                        actual.type_kind,
                        actual.start,
                        actual.end,
                        actual.depth
                    ));
                }
                hasher.write_bytes(input.as_bytes());
                hasher.write_u64(*start);
                hasher.write_u64(*end);
                hasher.write_u64(u64::from(actual.expression_kind));
                hasher.write_u64(u64::from(actual.type_kind));
                hasher.write_u64(actual.start);
                hasher.write_u64(actual.end);
                hasher.write_u64(actual.depth);
            }
            CorpusCase::TypedBinary {
                input,
                start,
                end,
                expected_expression_kind,
                expected_type_kind,
                expected_start,
                expected_end,
                expected_depth,
            } => {
                typed_binary_cases += 1;
                let actual = infer_binary_literal_types(input.as_bytes(), *start, *end);
                if actual.len() != 1 {
                    return Err(format!(
                        "typed binary ({input:?}, {start}, {end}) expected one record, got {}",
                        actual.len()
                    ));
                }
                let actual = actual[0];
                if (
                    actual.expression_kind,
                    actual.type_kind,
                    actual.start,
                    actual.end,
                    actual.depth,
                ) != (
                    *expected_expression_kind,
                    *expected_type_kind,
                    *expected_start,
                    *expected_end,
                    *expected_depth,
                ) {
                    return Err(format!(
                        "typed binary ({input:?}, {start}, {end}) expected ({expected_expression_kind}, {expected_type_kind}, {expected_start}, {expected_end}, {expected_depth}), got ({}, {}, {}, {}, {})",
                        actual.expression_kind,
                        actual.type_kind,
                        actual.start,
                        actual.end,
                        actual.depth
                    ));
                }
                hasher.write_bytes(input.as_bytes());
                hasher.write_u64(*start);
                hasher.write_u64(*end);
                hasher.write_u64(u64::from(actual.expression_kind));
                hasher.write_u64(u64::from(actual.type_kind));
                hasher.write_u64(actual.start);
                hasher.write_u64(actual.end);
                hasher.write_u64(actual.depth);
            }
            CorpusCase::TypedCast {
                input,
                start,
                end,
                expected_expression_kind,
                expected_type_kind,
                expected_start,
                expected_end,
                expected_depth,
            } => {
                typed_cast_cases += 1;
                let actual = infer_cast_literal_types(input.as_bytes(), *start, *end);
                if actual.len() != 1 {
                    return Err(format!(
                        "typed cast ({input:?}, {start}, {end}) expected one record, got {}",
                        actual.len()
                    ));
                }
                let actual = actual[0];
                if (
                    actual.expression_kind,
                    actual.type_kind,
                    actual.start,
                    actual.end,
                    actual.depth,
                ) != (
                    *expected_expression_kind,
                    *expected_type_kind,
                    *expected_start,
                    *expected_end,
                    *expected_depth,
                ) {
                    return Err(format!(
                        "typed cast ({input:?}, {start}, {end}) expected ({expected_expression_kind}, {expected_type_kind}, {expected_start}, {expected_end}, {expected_depth}), got ({}, {}, {}, {}, {})",
                        actual.expression_kind,
                        actual.type_kind,
                        actual.start,
                        actual.end,
                        actual.depth
                    ));
                }
                hasher.write_bytes(input.as_bytes());
                hasher.write_u64(*start);
                hasher.write_u64(*end);
                hasher.write_u64(u64::from(actual.expression_kind));
                hasher.write_u64(u64::from(actual.type_kind));
                hasher.write_u64(actual.start);
                hasher.write_u64(actual.end);
                hasher.write_u64(actual.depth);
            }
            CorpusCase::TypedUnary {
                input,
                start,
                end,
                expected_expression_kind,
                expected_type_kind,
                expected_start,
                expected_end,
                expected_depth,
            } => {
                typed_unary_cases += 1;
                let actual = infer_unary_literal_types(input.as_bytes(), *start, *end);
                if actual.len() != 1 {
                    return Err(format!(
                        "typed unary ({input:?}, {start}, {end}) expected one record, got {}",
                        actual.len()
                    ));
                }
                let actual = actual[0];
                if (
                    actual.expression_kind,
                    actual.type_kind,
                    actual.start,
                    actual.end,
                    actual.depth,
                ) != (
                    *expected_expression_kind,
                    *expected_type_kind,
                    *expected_start,
                    *expected_end,
                    *expected_depth,
                ) {
                    return Err(format!(
                        "typed unary ({input:?}, {start}, {end}) expected ({expected_expression_kind}, {expected_type_kind}, {expected_start}, {expected_end}, {expected_depth}), got ({}, {}, {}, {}, {})",
                        actual.expression_kind,
                        actual.type_kind,
                        actual.start,
                        actual.end,
                        actual.depth
                    ));
                }
                hasher.write_bytes(input.as_bytes());
                hasher.write_u64(*start);
                hasher.write_u64(*end);
                hasher.write_u64(u64::from(actual.expression_kind));
                hasher.write_u64(u64::from(actual.type_kind));
                hasher.write_u64(actual.start);
                hasher.write_u64(actual.end);
                hasher.write_u64(actual.depth);
            }
            CorpusCase::Call {
                input,
                start,
                end,
                expected_count,
                expected_callee_start,
                expected_callee_end,
                expected_open_start,
                expected_open_end,
                expected_close_start,
                expected_close_end,
                expected_argument_count,
                expected_depth,
            } => {
                call_cases += 1;
                let actual = parse_expression_calls(input.as_bytes(), *start, *end);
                if actual.len() != *expected_count {
                    return Err(format!(
                        "call ({input:?}, {start}, {end}) expected {expected_count} records, got {}",
                        actual.len()
                    ));
                }
                if let Some(first) = actual.first()
                    && (
                        first.callee_start,
                        first.callee_end,
                        first.open_start,
                        first.open_end,
                        first.close_start,
                        first.close_end,
                        first.argument_count,
                        first.depth,
                    ) != (
                        *expected_callee_start,
                        *expected_callee_end,
                        *expected_open_start,
                        *expected_open_end,
                        *expected_close_start,
                        *expected_close_end,
                        *expected_argument_count,
                        *expected_depth,
                    )
                {
                    return Err(format!(
                        "call ({input:?}, {start}, {end}) expected ({expected_callee_start}, {expected_callee_end}, {expected_open_start}, {expected_open_end}, {expected_close_start}, {expected_close_end}, {expected_argument_count}, {expected_depth}), got ({}, {}, {}, {}, {}, {}, {}, {})",
                        first.callee_start,
                        first.callee_end,
                        first.open_start,
                        first.open_end,
                        first.close_start,
                        first.close_end,
                        first.argument_count,
                        first.depth
                    ));
                }
                hasher.write_bytes(input.as_bytes());
                hasher.write_u64(*start);
                hasher.write_u64(*end);
                hasher.write_u64(*expected_count as u64);
                for record in actual {
                    hasher.write_u64(record.callee_start);
                    hasher.write_u64(record.callee_end);
                    hasher.write_u64(record.open_start);
                    hasher.write_u64(record.open_end);
                    hasher.write_u64(record.close_start);
                    hasher.write_u64(record.close_end);
                    hasher.write_u64(record.argument_count);
                    hasher.write_u64(record.depth);
                }
            }
            CorpusCase::TypedName {
                input,
                start,
                end,
                binding_start,
                binding_end,
                binding_type_kind,
                expected_count,
                expected_expression_kind,
                expected_type_kind,
                expected_start,
                expected_end,
                expected_depth,
            } => {
                typed_name_cases += 1;
                let bindings = [TypedNameBindingHeader {
                    name_start: *binding_start,
                    name_end: *binding_end,
                    type_kind: *binding_type_kind,
                }];
                let actual = infer_name_types(input.as_bytes(), *start, *end, &bindings);
                if actual.len() != *expected_count {
                    return Err(format!(
                        "typed name ({input:?}, {start}, {end}) expected {expected_count} records, got {}",
                        actual.len()
                    ));
                }
                if let Some(first) = actual.first()
                    && (
                        first.expression_kind,
                        first.type_kind,
                        first.start,
                        first.end,
                        first.depth,
                    ) != (
                        *expected_expression_kind,
                        *expected_type_kind,
                        *expected_start,
                        *expected_end,
                        *expected_depth,
                    )
                {
                    return Err(format!(
                        "typed name ({input:?}, {start}, {end}) expected ({expected_expression_kind}, {expected_type_kind}, {expected_start}, {expected_end}, {expected_depth}), got ({}, {}, {}, {}, {})",
                        first.expression_kind, first.type_kind, first.start, first.end, first.depth
                    ));
                }
                hasher.write_bytes(input.as_bytes());
                hasher.write_u64(*start);
                hasher.write_u64(*end);
                hasher.write_u64(*binding_start);
                hasher.write_u64(*binding_end);
                hasher.write_u64(u64::from(*binding_type_kind));
                hasher.write_u64(*expected_count as u64);
                for record in actual {
                    hasher.write_u64(u64::from(record.expression_kind));
                    hasher.write_u64(u64::from(record.type_kind));
                    hasher.write_u64(record.start);
                    hasher.write_u64(record.end);
                    hasher.write_u64(record.depth);
                }
            }
            CorpusCase::TypedCall {
                input,
                start,
                end,
                binding_start,
                binding_end,
                binding_parameter_count,
                binding_return_type_kind,
                expected_count,
                expected_expression_kind,
                expected_type_kind,
                expected_start,
                expected_end,
                expected_depth,
            } => {
                typed_call_cases += 1;
                let bindings = [TypedCallBindingHeader {
                    name_start: *binding_start,
                    name_end: *binding_end,
                    parameter_count: *binding_parameter_count,
                    return_type_kind: *binding_return_type_kind,
                }];
                let actual = infer_call_types(input.as_bytes(), *start, *end, &bindings);
                if actual.len() != *expected_count {
                    return Err(format!(
                        "typed call ({input:?}, {start}, {end}) expected {expected_count} records, got {}",
                        actual.len()
                    ));
                }
                if let Some(first) = actual.first()
                    && (
                        first.expression_kind,
                        first.type_kind,
                        first.start,
                        first.end,
                        first.depth,
                    ) != (
                        *expected_expression_kind,
                        *expected_type_kind,
                        *expected_start,
                        *expected_end,
                        *expected_depth,
                    )
                {
                    return Err(format!(
                        "typed call ({input:?}, {start}, {end}) expected ({expected_expression_kind}, {expected_type_kind}, {expected_start}, {expected_end}, {expected_depth}), got ({}, {}, {}, {}, {})",
                        first.expression_kind, first.type_kind, first.start, first.end, first.depth
                    ));
                }
                hasher.write_bytes(input.as_bytes());
                hasher.write_u64(*start);
                hasher.write_u64(*end);
                hasher.write_u64(*binding_start);
                hasher.write_u64(*binding_end);
                hasher.write_u64(*binding_parameter_count);
                hasher.write_u64(u64::from(*binding_return_type_kind));
                hasher.write_u64(*expected_count as u64);
                for record in actual {
                    hasher.write_u64(u64::from(record.expression_kind));
                    hasher.write_u64(u64::from(record.type_kind));
                    hasher.write_u64(record.start);
                    hasher.write_u64(record.end);
                    hasher.write_u64(record.depth);
                }
            }
            CorpusCase::TypedCallResolve {
                input,
                start,
                end,
                candidates,
                expected_count,
                expected_expression_kind,
                expected_type_kind,
                expected_start,
                expected_end,
                expected_depth,
            } => {
                typed_call_resolve_cases += 1;
                let actual = infer_call_types_resolved(input.as_bytes(), *start, *end, candidates);
                if actual.len() != *expected_count {
                    return Err(format!(
                        "typed call resolve ({input:?}, {start}, {end}) expected {expected_count} records, got {}",
                        actual.len()
                    ));
                }
                if let Some(first) = actual.first()
                    && (
                        first.expression_kind,
                        first.type_kind,
                        first.start,
                        first.end,
                        first.depth,
                    ) != (
                        *expected_expression_kind,
                        *expected_type_kind,
                        *expected_start,
                        *expected_end,
                        *expected_depth,
                    )
                {
                    return Err(format!(
                        "typed call resolve ({input:?}, {start}, {end}) expected ({expected_expression_kind}, {expected_type_kind}, {expected_start}, {expected_end}, {expected_depth}), got ({}, {}, {}, {}, {})",
                        first.expression_kind, first.type_kind, first.start, first.end, first.depth
                    ));
                }
                hasher.write_bytes(input.as_bytes());
                hasher.write_u64(*start);
                hasher.write_u64(*end);
                hasher.write_u64(candidates.len() as u64);
                for candidate in candidates {
                    hasher.write_u64(candidate.name_start);
                    hasher.write_u64(candidate.name_end);
                    hasher.write_u64(candidate.parameter_count);
                    hasher.write_u64(candidate.generic_parameter_count);
                    hasher.write_u64(u64::from(candidate.return_type_kind));
                    hasher.write_u64(u64::from(candidate.parameter_type_kind));
                    hasher.write_u64(u64::from(candidate.parameter_type_start));
                    hasher.write_u64(u64::from(candidate.parameter_type_end));
                    hasher.write_u64(u64::from(candidate.generic_bound_kind));
                    hasher.write_u64(u64::from(candidate.generic_substitution_kind));
                }
                hasher.write_u64(*expected_count as u64);
                for record in actual {
                    hasher.write_u64(u64::from(record.expression_kind));
                    hasher.write_u64(u64::from(record.type_kind));
                    hasher.write_u64(record.start);
                    hasher.write_u64(record.end);
                    hasher.write_u64(record.depth);
                }
            }
            CorpusCase::TypedRegionName {
                input,
                start,
                end,
                binding_start,
                binding_end,
                region_start,
                region_end,
                binding_type_kind,
                binding_scope_depth,
                expected_count,
                expected_expression_kind,
                expected_type_kind,
                expected_start,
                expected_end,
                expected_depth,
            } => {
                typed_region_name_cases += 1;
                let bindings = [TypedRegionNameBindingHeader {
                    name_start: *binding_start,
                    name_end: *binding_end,
                    region_start: *region_start,
                    region_end: *region_end,
                    type_kind: *binding_type_kind,
                    scope_depth: *binding_scope_depth,
                }];
                let actual = infer_region_name_types(input.as_bytes(), *start, *end, &bindings);
                if actual.len() != *expected_count {
                    return Err(format!(
                        "typed region name ({input:?}, {start}, {end}) expected {expected_count} records, got {}",
                        actual.len()
                    ));
                }
                if let Some(first) = actual.first()
                    && (
                        first.expression_kind,
                        first.type_kind,
                        first.start,
                        first.end,
                        first.depth,
                    ) != (
                        *expected_expression_kind,
                        *expected_type_kind,
                        *expected_start,
                        *expected_end,
                        *expected_depth,
                    )
                {
                    return Err(format!(
                        "typed region name ({input:?}, {start}, {end}) expected ({expected_expression_kind}, {expected_type_kind}, {expected_start}, {expected_end}, {expected_depth}), got ({}, {}, {}, {}, {})",
                        first.expression_kind, first.type_kind, first.start, first.end, first.depth
                    ));
                }
                hasher.write_bytes(input.as_bytes());
                hasher.write_u64(*start);
                hasher.write_u64(*end);
                hasher.write_u64(*binding_start);
                hasher.write_u64(*binding_end);
                hasher.write_u64(*region_start);
                hasher.write_u64(*region_end);
                hasher.write_u64(u64::from(*binding_type_kind));
                hasher.write_u64(*binding_scope_depth);
                hasher.write_u64(*expected_count as u64);
                for record in actual {
                    hasher.write_u64(u64::from(record.expression_kind));
                    hasher.write_u64(u64::from(record.type_kind));
                    hasher.write_u64(record.start);
                    hasher.write_u64(record.end);
                    hasher.write_u64(record.depth);
                }
            }
        }
    }

    registry
        .clear()
        .map_err(|error| format!("oracle API clear failed: {error:?}"))?;

    Ok(OracleReport {
        api_schema: API_SCHEMA,
        api_version: API_VERSION,
        api_lifetime: "validated-borrowed",
        api_registry: "caller-owned-rwlock-snapshot",
        byte_cases,
        span_cases,
        api_span_cases,
        scan_cases,
        api_scan_cases,
        token_cases,
        token_info_cases,
        api_token_info_cases,
        count_cases,
        typed_literal_cases,
        typed_binary_cases,
        typed_cast_cases,
        typed_unary_cases,
        call_cases,
        typed_name_cases,
        typed_call_cases,
        typed_call_resolve_cases,
        typed_region_name_cases,
        diagnostic_cases,
        api_diagnostic_cases,
        fingerprint: hasher.finish().to_string(),
    })
}

/// Returns the Rust implementation of the versioned frontend contract.
#[must_use]
pub const fn rust_oracle_api() -> FrontendApiV1 {
    FrontendApiV1::new(oracle_classify_byte, oracle_token_span, oracle_diagnostic)
}

/// Returns the additive TokenInfo callback extension implemented by the oracle.
#[must_use]
pub const fn rust_oracle_token_info_api() -> FrontendTokenInfoApiV1 {
    FrontendTokenInfoApiV1::new(oracle_token_info)
}

extern "C" fn oracle_classify_byte(value: u8) -> u8 {
    classify_byte(value)
}

extern "C" fn oracle_token_span(start: u64, end: u64) -> TokenSpan {
    let (start, end) = token_span(start, end);
    TokenSpan { start, end }
}

extern "C" fn oracle_diagnostic(code: u16, severity: u8) -> DiagnosticValue {
    let (code, severity) = diagnostic(code, severity);
    DiagnosticValue { code, severity }
}

extern "C" fn oracle_token_info(kind: u8, start: u64, end: u64) -> TokenInfo {
    TokenInfo { kind, start, end }
}

/// Rust reference for the Jadren byte-classification contract.
#[must_use]
pub const fn classify_byte(value: u8) -> u8 {
    if value == b'_' || (value >= b'A' && value <= b'Z') || (value >= b'a' && value <= b'z') {
        1
    } else if value >= b'0' && value <= b'9' {
        2
    } else if value == b'\t' || value == b'\n' || value == b' ' {
        3
    } else {
        0
    }
}

/// Rust reference for the Jadren token-span aggregate.
#[must_use]
pub const fn token_span(start: u64, end: u64) -> (u64, u64) {
    (start, end)
}

/// Rust reference for the allocation-free identifier/digit scan primitive.
#[must_use]
pub fn scan_identifier(bytes: &[u8], start: u64, end: u64) -> (u64, u64) {
    let mut index = start;
    while index < end {
        let Some(byte_index) = usize::try_from(index).ok() else {
            break;
        };
        let Some(byte) = bytes.get(byte_index).copied() else {
            break;
        };
        let class = classify_byte(byte);
        if class == 1 || class == 2 {
            index += 1;
        } else {
            break;
        }
    }
    (start, index)
}

/// Rust reference for the token scan through a validated frontend API lease.
#[must_use]
pub fn scan_identifier_via_api(
    api: &FrontendApiLease<'_>,
    bytes: &[u8],
    start: u64,
    end: u64,
) -> (u64, u64) {
    let mut index = start;
    while index < end {
        let Some(byte_index) = usize::try_from(index).ok() else {
            break;
        };
        let Some(byte) = bytes.get(byte_index).copied() else {
            break;
        };
        let class = api.classify_byte(byte);
        if class == 1 || class == 2 {
            index += 1;
        } else {
            break;
        }
    }
    (start, index)
}

/// Rust reference for the next-token scan primitive.
#[must_use]
pub fn scan_token(bytes: &[u8], start: u64, end: u64) -> (u64, u64) {
    let mut index = start;
    while index < end {
        let Some(byte_index) = usize::try_from(index).ok() else {
            break;
        };
        let Some(byte) = bytes.get(byte_index).copied() else {
            break;
        };
        if classify_byte(byte) == 3 {
            index += 1;
        } else {
            break;
        }
    }
    let token_start = index;
    if index >= end {
        return (token_start, index);
    }
    let byte = bytes[usize::try_from(index).expect("validated token index")];
    match classify_byte(byte) {
        1 => {
            while index < end {
                let byte_index = usize::try_from(index).expect("validated token index");
                let class = classify_byte(bytes[byte_index]);
                if class == 1 || class == 2 {
                    index += 1;
                } else {
                    break;
                }
            }
        }
        2 => {
            while index < end {
                let byte_index = usize::try_from(index).expect("validated token index");
                if classify_byte(bytes[byte_index]) == 2 {
                    index += 1;
                } else {
                    break;
                }
            }
        }
        _ => index += 1,
    }
    (token_start, index)
}

/// Rust reference for the token kind and span scan primitive.
#[must_use]
pub fn scan_token_info(bytes: &[u8], start: u64, end: u64) -> (u8, u64, u64) {
    let (token_start, token_end) = scan_token(bytes, start, end);
    if token_start >= end {
        return (0, token_start, token_end);
    }
    let index = usize::try_from(token_start).expect("validated token-info index");
    let kind = match classify_byte(bytes[index]) {
        1 => 1,
        2 => 2,
        _ => 3,
    };
    (kind, token_start, token_end)
}

/// Rust reference for the bounded syntax-only call-header hand-off.
#[must_use]
pub fn parse_expression_calls(bytes: &[u8], start: u64, end: u64) -> Vec<ExpressionCallHeader> {
    let mut calls = Vec::new();
    let mut cursor = start;
    let mut delimiter_depth: u64 = 0;
    let mut callee_start = end;
    let mut callee_end = end;
    let mut can_call = false;

    while cursor < end {
        let (kind, token_start, token_end) = scan_token_info(bytes, cursor, end);
        if token_start >= end || token_end <= token_start {
            break;
        }
        let byte = bytes[usize::try_from(token_start).expect("validated call token")];
        if kind == 1 || kind == 2 {
            if !can_call {
                callee_start = token_start;
            }
            callee_end = token_end;
            can_call = true;
            cursor = token_end;
            continue;
        }
        match byte {
            b')' | b']' => {
                delimiter_depth = delimiter_depth.saturating_sub(1);
                callee_end = token_end;
                can_call = true;
                cursor = token_end;
            }
            b'[' => {
                if !can_call {
                    callee_start = token_start;
                }
                delimiter_depth += 1;
                callee_end = token_end;
                can_call = true;
                cursor = token_end;
            }
            b'(' if !can_call => {
                delimiter_depth += 1;
                cursor = token_end;
            }
            b'(' => {
                let mut nested_depth = 1_u64;
                let mut nested_cursor = token_end;
                let mut close_start = end;
                let mut close_end = end;
                let mut argument_count = 0_u64;
                let mut argument_has_token = false;
                while nested_cursor < end {
                    let (nested_kind, nested_start, nested_end) =
                        scan_token_info(bytes, nested_cursor, end);
                    if nested_start >= end || nested_end <= nested_start {
                        break;
                    }
                    let nested_byte =
                        bytes[usize::try_from(nested_start).expect("validated nested call token")];
                    match nested_byte {
                        b'(' | b'[' => {
                            if nested_depth == 1 {
                                argument_has_token = true;
                            }
                            nested_depth += 1;
                        }
                        b')' => {
                            if nested_depth == 1 {
                                if argument_has_token {
                                    argument_count += 1;
                                }
                                close_start = nested_start;
                                close_end = nested_end;
                                nested_depth = 0;
                                break;
                            }
                            nested_depth = nested_depth.saturating_sub(1);
                        }
                        b']' => {
                            nested_depth = nested_depth.saturating_sub(1);
                        }
                        b',' if nested_depth == 1 => {
                            if argument_has_token {
                                argument_count += 1;
                                argument_has_token = false;
                            }
                        }
                        _ if nested_kind != 0 && nested_depth == 1 => {
                            argument_has_token = true;
                        }
                        _ => {}
                    }
                    nested_cursor = nested_end;
                }
                if nested_depth > 0 && argument_has_token {
                    argument_count += 1;
                }
                calls.push(ExpressionCallHeader {
                    callee_start,
                    callee_end,
                    open_start: token_start,
                    open_end: token_end,
                    close_start,
                    close_end,
                    argument_count,
                    depth: delimiter_depth,
                });
                cursor = close_end.min(end);
                callee_end = close_end;
                can_call = true;
            }
            _ => {
                can_call = false;
                cursor = token_end;
            }
        }
    }
    calls
}

/// Rust reference for the bounded typed-call hand-off. A syntax call is
/// emitted only when its callee span exactly matches a caller-owned function
/// binding and its argument count matches the bound parameter count. The
/// result uses the existing 32-byte typed-expression header; this is not
/// overload resolution, generic inference, or a full function-type check.
#[must_use]
pub fn infer_call_types(
    bytes: &[u8],
    start: u64,
    end: u64,
    bindings: &[TypedCallBindingHeader],
) -> Vec<TypedExpressionHeader> {
    if start > end || end > bytes.len() as u64 {
        return Vec::new();
    }
    let mut output = Vec::new();
    for call in parse_expression_calls(bytes, start, end) {
        if call.close_start >= end
            || call.close_end <= call.open_end
            || call.callee_start >= call.callee_end
        {
            continue;
        }
        if call.callee_start < start || call.close_end > end {
            continue;
        }
        for binding in bindings {
            if binding.return_type_kind == 0
                || binding.return_type_kind > 5
                || binding.name_start >= binding.name_end
                || binding.name_end > bytes.len() as u64
                || binding.parameter_count != call.argument_count
            {
                continue;
            }
            if binding.name_end - binding.name_start != call.callee_end - call.callee_start {
                continue;
            }
            let binding_start = usize::try_from(binding.name_start).expect("valid binding start");
            let binding_end = usize::try_from(binding.name_end).expect("valid binding end");
            let callee_start = usize::try_from(call.callee_start).expect("valid callee start");
            let callee_end = usize::try_from(call.callee_end).expect("valid callee end");
            if bytes[binding_start..binding_end] != bytes[callee_start..callee_end] {
                continue;
            }
            output.push(TypedExpressionHeader {
                expression_kind: 8,
                type_kind: binding.return_type_kind,
                start: call.callee_start,
                end: call.close_end,
                depth: call.depth,
            });
            break;
        }
    }
    output
}

fn builtin_parameter_type_kind(bytes: &[u8], start: u64, end: u64) -> u8 {
    let start = usize::try_from(start).expect("valid builtin type start");
    let end = usize::try_from(end).expect("valid builtin type end");
    match &bytes[start..end] {
        b"Bool" => 1,
        b"Integer" => 2,
        b"Float" => 3,
        b"String" => 4,
        b"Char" => 5,
        _ => 0,
    }
}

fn parse_builtin_parameter_type_list(bytes: &[u8], start: u64, end: u64) -> Option<Vec<u8>> {
    if start >= end || end > bytes.len() as u64 {
        return None;
    }
    let mut cursor = start;
    let mut expecting_type = true;
    let mut types = Vec::new();
    let mut wrapped = false;
    let mut closed = false;
    while cursor < end {
        let (kind, token_start, token_end) = scan_token_info(bytes, cursor, end);
        if token_start >= end || token_end <= token_start {
            break;
        }
        let token_byte = bytes[usize::try_from(token_start).expect("valid type token")];
        if !wrapped && types.is_empty() && kind == 3 && token_byte == b'[' {
            wrapped = true;
        } else if wrapped && kind == 3 && token_byte == b']' {
            if expecting_type {
                return None;
            }
            closed = true;
            cursor = token_end;
            break;
        } else if kind == 1 {
            if !expecting_type {
                return None;
            }
            let type_kind = builtin_parameter_type_kind(bytes, token_start, token_end);
            if type_kind == 0 {
                return None;
            }
            types.push(type_kind);
            expecting_type = false;
        } else if kind == 3 && token_byte == b',' {
            if expecting_type {
                return None;
            }
            expecting_type = true;
        } else {
            return None;
        }
        cursor = token_end;
    }
    if wrapped {
        if !closed {
            return None;
        }
        let (trailing_kind, trailing_start, _) = scan_token_info(bytes, cursor, end);
        if trailing_kind != 0 || trailing_start < end {
            return None;
        }
    }
    if expecting_type || types.is_empty() {
        return None;
    }
    Some(types)
}

fn parse_argument_literal_types(bytes: &[u8], start: u64, end: u64) -> Vec<u8> {
    let mut cursor = start;
    let mut argument_start = start;
    let mut depth = 0_u64;
    let mut has_token = false;
    let mut types = Vec::new();
    while cursor < end {
        let (kind, token_start, token_end) = scan_token_info(bytes, cursor, end);
        if token_start >= end || token_end <= token_start {
            break;
        }
        let token_byte = bytes[usize::try_from(token_start).expect("valid argument token")];
        match token_byte {
            b'(' | b'[' => {
                depth += 1;
                has_token = true;
            }
            b')' | b']' => {
                depth = depth.saturating_sub(1);
                has_token = true;
            }
            b',' if depth == 0 => {
                if has_token {
                    types.push(bounded_literal_type_kind(
                        bytes,
                        argument_start,
                        token_start,
                    ));
                    has_token = false;
                }
                argument_start = token_end;
            }
            _ if kind != 0 => has_token = true,
            _ => {}
        }
        cursor = token_end;
    }
    if has_token {
        types.push(bounded_literal_type_kind(bytes, argument_start, end));
    }
    types
}

fn generic_bound_matches(bound_kind: u8, argument_types: &[u8]) -> bool {
    if bound_kind == 0 {
        return true;
    }
    if bound_kind > 3 || argument_types.is_empty() {
        return false;
    }
    argument_types.iter().all(|type_kind| match bound_kind {
        1 => *type_kind == 2 || *type_kind == 3,
        2 => *type_kind == 1,
        3 => *type_kind == 4 || *type_kind == 5,
        _ => false,
    })
}

/// Rust reference for the bounded overload/generic typed-call resolver. A
/// candidate is eligible when its name bytes and positional arity match the
/// syntax call. Single-argument exact builtin matching uses
/// `parameter_type_kind`; a bounded comma-separated builtin list in
/// `parameter_type_start..parameter_type_end` extends the same rule to all
/// positional arguments. Zero type metadata remains an arity-only wildcard.
/// Exact type matches win over wildcards, then the lowest
/// `generic_parameter_count` wins; zero is a concrete overload and positive
/// values are generic fallbacks. A generic candidate may carry one builtin
/// family bound (`Numeric`, `Boolean`, or `Text`) which must match every
/// argument. Substitution kind `1` derives the return type from one literal
/// argument; kind `2` requires two or more literal arguments to share that
/// type and derives the same return type; kind `3` derives from the first
/// literal argument while accepting independently typed remaining literals.
/// Equal best specificity is ambiguous and emits no record. Full trait bounds
/// and generic substitution remain outside this additive hand-off.
#[must_use]
pub fn infer_call_types_resolved(
    bytes: &[u8],
    start: u64,
    end: u64,
    candidates: &[TypedCallCandidateHeader],
) -> Vec<TypedExpressionHeader> {
    if start > end || end > bytes.len() as u64 {
        return Vec::new();
    }
    let mut output = Vec::new();
    for call in parse_expression_calls(bytes, start, end) {
        if call.close_start >= end
            || call.close_end <= call.open_end
            || call.callee_start >= call.callee_end
            || call.callee_start < start
            || call.close_end > end
        {
            continue;
        }
        let callee_start = usize::try_from(call.callee_start).expect("valid resolved callee start");
        let callee_end = usize::try_from(call.callee_end).expect("valid resolved callee end");
        let callee = &bytes[callee_start..callee_end];
        let argument_types = if call.argument_count == 1 {
            vec![bounded_literal_type_kind(
                bytes,
                call.open_end,
                call.close_start,
            )]
        } else if call.argument_count > 1 {
            parse_argument_literal_types(bytes, call.open_end, call.close_start)
        } else {
            Vec::new()
        };
        let mut best_type_specificity = 0_u8;
        let mut best_generic_count = u64::MAX;
        let mut best_return_type = 0_u8;
        let mut best_matches = 0_u64;
        for candidate in candidates {
            if candidate.return_type_kind > 5
                || candidate.generic_parameter_count > 32
                || candidate.parameter_type_kind > 5
                || candidate.generic_bound_kind > 3
                || candidate.generic_substitution_kind > 3
                || (candidate.return_type_kind == 0 && candidate.generic_substitution_kind == 0)
                || (candidate.generic_substitution_kind == 1
                    && (candidate.generic_parameter_count == 0
                        || candidate.parameter_count != 1
                        || candidate.parameter_type_kind != 0
                        || candidate.parameter_type_start != 0
                        || candidate.parameter_type_end != 0))
                || (candidate.generic_substitution_kind == 2
                    && (candidate.generic_parameter_count == 0
                        || candidate.parameter_count < 2
                        || candidate.parameter_type_kind != 0
                        || candidate.parameter_type_start != 0
                        || candidate.parameter_type_end != 0))
                || (candidate.generic_substitution_kind == 3
                    && (candidate.generic_parameter_count == 0
                        || candidate.parameter_count < 2
                        || candidate.parameter_type_kind != 0
                        || candidate.parameter_type_start != 0
                        || candidate.parameter_type_end != 0))
                || (candidate.generic_parameter_count == 0 && candidate.generic_bound_kind != 0)
                || (candidate.parameter_count != 1 && candidate.parameter_type_kind != 0)
                || (candidate.parameter_type_start == 0 && candidate.parameter_type_end != 0)
                || (candidate.parameter_type_start != 0
                    && (candidate.parameter_type_end <= candidate.parameter_type_start
                        || u64::from(candidate.parameter_type_end) > bytes.len() as u64
                        || candidate.parameter_type_kind != 0))
                || candidate.name_start >= candidate.name_end
                || candidate.name_end > bytes.len() as u64
                || candidate.parameter_count != call.argument_count
            {
                continue;
            }
            if !generic_bound_matches(candidate.generic_bound_kind, &argument_types) {
                continue;
            }
            if candidate.generic_substitution_kind == 1
                && (argument_types.len() != 1 || argument_types[0] == 0)
            {
                continue;
            }
            if candidate.generic_substitution_kind == 2
                && (argument_types.len() < 2
                    || argument_types.contains(&0)
                    || argument_types
                        .iter()
                        .any(|type_kind| *type_kind != argument_types[0]))
            {
                continue;
            }
            if candidate.generic_substitution_kind == 3
                && (argument_types.len() < 2 || argument_types.contains(&0))
            {
                continue;
            }
            let type_specificity = if candidate.parameter_type_start != 0 {
                let Some(parameter_types) = parse_builtin_parameter_type_list(
                    bytes,
                    u64::from(candidate.parameter_type_start),
                    u64::from(candidate.parameter_type_end),
                ) else {
                    continue;
                };
                if parameter_types.len() != argument_types.len()
                    || parameter_types
                        .iter()
                        .zip(argument_types.iter())
                        .any(|(expected, actual)| *actual == 0 || expected != actual)
                {
                    continue;
                }
                2
            } else if candidate.parameter_count == 1 {
                if candidate.parameter_type_kind == 0 {
                    if candidate.generic_bound_kind == 0 && candidate.generic_substitution_kind == 0
                    {
                        0
                    } else {
                        1
                    }
                } else if candidate.parameter_type_kind == argument_types[0] {
                    2
                } else {
                    continue;
                }
            } else {
                if candidate.generic_bound_kind == 0 && candidate.generic_substitution_kind == 0 {
                    0
                } else {
                    1
                }
            };
            let candidate_start =
                usize::try_from(candidate.name_start).expect("valid resolved candidate start");
            let candidate_end =
                usize::try_from(candidate.name_end).expect("valid resolved candidate end");
            if &bytes[candidate_start..candidate_end] != callee {
                continue;
            }
            let candidate_return_type = if candidate.generic_substitution_kind == 1
                || candidate.generic_substitution_kind == 2
                || candidate.generic_substitution_kind == 3
            {
                argument_types[0]
            } else {
                candidate.return_type_kind
            };
            if type_specificity > best_type_specificity {
                best_type_specificity = type_specificity;
                best_generic_count = candidate.generic_parameter_count;
                best_return_type = candidate_return_type;
                best_matches = 1;
            } else if type_specificity == best_type_specificity {
                if candidate.generic_parameter_count < best_generic_count {
                    best_generic_count = candidate.generic_parameter_count;
                    best_return_type = candidate_return_type;
                    best_matches = 1;
                } else if candidate.generic_parameter_count == best_generic_count {
                    best_matches += 1;
                }
            }
        }
        if best_matches == 1 {
            output.push(TypedExpressionHeader {
                expression_kind: 8,
                type_kind: best_return_type,
                start: call.callee_start,
                end: call.close_end,
                depth: call.depth,
            });
        }
    }
    output
}

/// Copies bounded overload/generic resolver records into caller-owned output.
#[must_use]
pub fn infer_call_types_resolved_into(
    bytes: &[u8],
    start: u64,
    end: u64,
    candidates: &[TypedCallCandidateHeader],
    output: &mut [TypedExpressionHeader],
) -> usize {
    let records = infer_call_types_resolved(bytes, start, end, candidates);
    let count = records.len().min(output.len());
    output[..count].copy_from_slice(&records[..count]);
    count
}

/// Copies typed-call records into a caller-owned output slice and returns the
/// number written, mirroring the bounded JDN/C export contract.
#[must_use]
pub fn infer_call_types_into(
    bytes: &[u8],
    start: u64,
    end: u64,
    bindings: &[TypedCallBindingHeader],
    output: &mut [TypedExpressionHeader],
) -> usize {
    let records = infer_call_types(bytes, start, end, bindings);
    let count = records.len().min(output.len());
    output[..count].copy_from_slice(&records[..count]);
    count
}

/// Rust reference for the bounded two-operator precedence hand-off used by
/// the self-hosting prototype. It deliberately accepts exactly three
/// non-empty operands and emits child-before-parent nodes. Prefix operators
/// are ignored while an operand is expected; recovery remains a later stage.
#[must_use]
pub fn parse_expression_precedence_headers(
    bytes: &[u8],
    start: u64,
    end: u64,
) -> Vec<ExpressionPrecedenceHeader> {
    #[derive(Clone, Copy)]
    struct Operator {
        start: u64,
        end: u64,
        kind: u8,
        precedence: u8,
        associativity: u8,
    }

    let mut operators: Vec<Operator> = Vec::new();
    let mut cursor = start;
    let mut delimiters: Vec<u8> = Vec::new();
    let mut expecting_operand = true;
    while cursor < end {
        let (token_kind, token_start, token_end) = scan_token_info(bytes, cursor, end);
        if token_start >= end || token_end <= token_start {
            break;
        }
        let token_index = usize::try_from(token_start).expect("validated precedence token");
        if token_kind == 3 {
            let byte = bytes[token_index];
            if matches!(byte, b')' | b']') {
                let expected = match delimiters.pop() {
                    Some(b'(') if byte == b')' => true,
                    Some(b'[') if byte == b']' => true,
                    _ => false,
                };
                if !expected {
                    return Vec::new();
                }
                expecting_operand = false;
                cursor = token_end;
                continue;
            }
            if matches!(byte, b'(' | b'[') {
                if delimiters.len() >= 32 {
                    return Vec::new();
                }
                delimiters.push(byte);
                expecting_operand = true;
                cursor = token_end;
                continue;
            }
            if delimiters.is_empty() {
                let operator_end = precedence_operator_end(bytes, token_start, end);
                if let Some((kind, precedence, associativity)) =
                    precedence_operator_info(bytes, token_start, operator_end)
                {
                    if expecting_operand
                        && is_unary_prefix_operator(bytes, token_start, operator_end)
                    {
                        cursor = operator_end;
                        continue;
                    }
                    operators.push(Operator {
                        start: token_start,
                        end: operator_end,
                        kind,
                        precedence,
                        associativity,
                    });
                    if operators.len() > 2 {
                        return Vec::new();
                    }
                    expecting_operand = true;
                    cursor = operator_end;
                    continue;
                }
            }
        }
        expecting_operand = false;
        cursor = token_end;
    }
    if !delimiters.is_empty() || operators.len() != 2 {
        return Vec::new();
    }

    let left = bounded_expression_span(bytes, start, operators[0].start);
    let middle = bounded_expression_span(bytes, operators[0].end, operators[1].start);
    let right = bounded_expression_span(bytes, operators[1].end, end);
    let (left, middle, right) = match (left, middle, right) {
        (Some(left), Some(middle), Some(right)) => (left, middle, right),
        _ => return Vec::new(),
    };

    let first_is_child = operators[0].precedence > operators[1].precedence
        || (operators[0].precedence == operators[1].precedence && operators[0].associativity == 1);
    let child_operator = if first_is_child {
        operators[0]
    } else {
        operators[1]
    };
    let root_operator = if first_is_child {
        operators[1]
    } else {
        operators[0]
    };
    let child_left = if first_is_child { left } else { middle };
    let child_right = if first_is_child { middle } else { right };
    let root_left = (left.0, if first_is_child { middle.1 } else { left.1 });
    let root_right = if first_is_child {
        right
    } else {
        (middle.0, right.1)
    };

    vec![
        ExpressionPrecedenceHeader {
            kind: child_operator.kind,
            precedence: child_operator.precedence,
            associativity: child_operator.associativity,
            left_start: child_left.0,
            left_end: child_left.1,
            operator_start: child_operator.start,
            operator_end: child_operator.end,
            right_start: child_right.0,
            right_end: child_right.1,
            depth: 0,
        },
        ExpressionPrecedenceHeader {
            kind: root_operator.kind,
            precedence: root_operator.precedence,
            associativity: root_operator.associativity,
            left_start: root_left.0,
            left_end: root_left.1,
            operator_start: root_operator.start,
            operator_end: root_operator.end,
            right_start: root_right.0,
            right_end: root_right.1,
            depth: 0,
        },
    ]
}

/// Copies the bounded two-operator precedence hand-off into caller-owned
/// storage. The returned count is the number of records that fit, so a short
/// output buffer cannot cause an out-of-bounds write at the JDN ABI boundary.
#[must_use]
pub fn parse_expression_precedence_headers_into(
    bytes: &[u8],
    start: u64,
    end: u64,
    output: &mut [ExpressionPrecedenceHeader],
) -> usize {
    let records = parse_expression_precedence_headers(bytes, start, end);
    let count = records.len().min(output.len());
    output[..count].copy_from_slice(&records[..count]);
    count
}

/// Rust reference for the additive bounded three-operator precedence hand-off.
/// It emits a post-order binary tree for exactly four non-empty operands and
/// three top-level operators; the existing two-operator contract is unchanged.
#[must_use]
pub fn parse_expression_precedence_chain_headers(
    bytes: &[u8],
    start: u64,
    end: u64,
) -> Vec<ExpressionPrecedenceHeader> {
    #[derive(Clone, Copy)]
    struct Operator {
        start: u64,
        end: u64,
        kind: u8,
        precedence: u8,
        associativity: u8,
    }

    let mut operators: Vec<Operator> = Vec::new();
    let mut cursor = start;
    let mut delimiters: Vec<u8> = Vec::new();
    let mut expecting_operand = true;
    while cursor < end {
        let (token_kind, token_start, token_end) = scan_token_info(bytes, cursor, end);
        if token_start >= end || token_end <= token_start {
            break;
        }
        let token_index = usize::try_from(token_start).expect("validated precedence chain token");
        if token_kind == 3 {
            let byte = bytes[token_index];
            if matches!(byte, b')' | b']') {
                let expected = match delimiters.pop() {
                    Some(b'(') if byte == b')' => true,
                    Some(b'[') if byte == b']' => true,
                    _ => false,
                };
                if !expected {
                    return Vec::new();
                }
                expecting_operand = false;
                cursor = token_end;
                continue;
            }
            if matches!(byte, b'(' | b'[') {
                if delimiters.len() >= 32 {
                    return Vec::new();
                }
                delimiters.push(byte);
                expecting_operand = true;
                cursor = token_end;
                continue;
            }
            if delimiters.is_empty() {
                let operator_end = precedence_operator_end(bytes, token_start, end);
                if let Some((kind, precedence, associativity)) =
                    precedence_operator_info(bytes, token_start, operator_end)
                {
                    if expecting_operand
                        && is_unary_prefix_operator(bytes, token_start, operator_end)
                    {
                        cursor = operator_end;
                        continue;
                    }
                    operators.push(Operator {
                        start: token_start,
                        end: operator_end,
                        kind,
                        precedence,
                        associativity,
                    });
                    if operators.len() > 3 {
                        return Vec::new();
                    }
                    expecting_operand = true;
                    cursor = operator_end;
                    continue;
                }
            }
        }
        expecting_operand = false;
        cursor = token_end;
    }
    if !delimiters.is_empty() || operators.len() != 3 {
        return Vec::new();
    }

    let operands = [
        bounded_expression_span(bytes, start, operators[0].start),
        bounded_expression_span(bytes, operators[0].end, operators[1].start),
        bounded_expression_span(bytes, operators[1].end, operators[2].start),
        bounded_expression_span(bytes, operators[2].end, end),
    ];
    let operands = match operands {
        [Some(first), Some(second), Some(third), Some(fourth)] => [first, second, third, fourth],
        _ => return Vec::new(),
    };

    let mut root_index = 0_usize;
    if operators[1].precedence < operators[0].precedence
        || (operators[1].precedence == operators[0].precedence && operators[1].associativity == 1)
    {
        root_index = 1;
    }
    let root_precedence = operators[root_index].precedence;
    if operators[2].precedence < root_precedence
        || (operators[2].precedence == root_precedence && operators[2].associativity == 1)
    {
        root_index = 2;
    }

    let header =
        |operator: Operator, left: (u64, u64), right: (u64, u64)| ExpressionPrecedenceHeader {
            kind: operator.kind,
            precedence: operator.precedence,
            associativity: operator.associativity,
            left_start: left.0,
            left_end: left.1,
            operator_start: operator.start,
            operator_end: operator.end,
            right_start: right.0,
            right_end: right.1,
            depth: 0,
        };

    let mut nodes = Vec::with_capacity(3);
    match root_index {
        0 => {
            let child_is_first = operators[1].precedence > operators[2].precedence
                || (operators[1].precedence == operators[2].precedence
                    && operators[1].associativity == 1);
            if child_is_first {
                nodes.push(header(operators[1], operands[1], operands[2]));
                nodes.push(header(
                    operators[2],
                    (operands[1].0, operands[2].1),
                    operands[3],
                ));
            } else {
                nodes.push(header(operators[2], operands[2], operands[3]));
                nodes.push(header(
                    operators[1],
                    operands[1],
                    (operands[2].0, operands[3].1),
                ));
            }
            nodes.push(header(
                operators[0],
                operands[0],
                (operands[1].0, operands[3].1),
            ));
        }
        1 => {
            nodes.push(header(operators[0], operands[0], operands[1]));
            nodes.push(header(operators[2], operands[2], operands[3]));
            nodes.push(header(
                operators[1],
                (operands[0].0, operands[1].1),
                (operands[2].0, operands[3].1),
            ));
        }
        _ => {
            let child_is_first = operators[0].precedence > operators[1].precedence
                || (operators[0].precedence == operators[1].precedence
                    && operators[0].associativity == 1);
            if child_is_first {
                nodes.push(header(operators[0], operands[0], operands[1]));
                nodes.push(header(
                    operators[1],
                    (operands[0].0, operands[1].1),
                    operands[2],
                ));
            } else {
                nodes.push(header(operators[1], operands[1], operands[2]));
                nodes.push(header(
                    operators[0],
                    operands[0],
                    (operands[1].0, operands[2].1),
                ));
            }
            nodes.push(header(
                operators[2],
                (operands[0].0, operands[2].1),
                operands[3],
            ));
        }
    }
    nodes
}

/// Copies the bounded three-operator precedence hand-off into caller-owned
/// storage. Truncation preserves child-before-parent order and returns the
/// number of records actually written.
#[must_use]
pub fn parse_expression_precedence_chain_headers_into(
    bytes: &[u8],
    start: u64,
    end: u64,
    output: &mut [ExpressionPrecedenceHeader],
) -> usize {
    let records = parse_expression_precedence_chain_headers(bytes, start, end);
    let count = records.len().min(output.len());
    output[..count].copy_from_slice(&records[..count]);
    count
}

/// Rust reference for the bounded four-operator precedence hand-off.
///
/// The implementation intentionally mirrors the allocation-free Jadran
/// fixture's fixed shunting-yard stacks: exactly five non-empty operands and
/// four top-level operators are accepted, with child-before-parent output.
#[must_use]
pub fn parse_expression_precedence_long_chain_headers(
    bytes: &[u8],
    start: u64,
    end: u64,
) -> Vec<ExpressionPrecedenceHeader> {
    #[derive(Clone, Copy)]
    struct Operator {
        start: u64,
        end: u64,
        kind: u8,
        precedence: u8,
        associativity: u8,
    }

    let mut operators: Vec<Operator> = Vec::new();
    let mut cursor = start;
    let mut delimiters: Vec<u8> = Vec::new();
    let mut expecting_operand = true;
    while cursor < end {
        let (token_kind, token_start, token_end) = scan_token_info(bytes, cursor, end);
        if token_start >= end || token_end <= token_start {
            break;
        }
        let token_index = usize::try_from(token_start).expect("validated long-chain token");
        if token_kind == 3 {
            let byte = bytes[token_index];
            if matches!(byte, b')' | b']') {
                let expected = match delimiters.pop() {
                    Some(b'(') if byte == b')' => true,
                    Some(b'[') if byte == b']' => true,
                    _ => false,
                };
                if !expected {
                    return Vec::new();
                }
                expecting_operand = false;
                cursor = token_end;
                continue;
            }
            if matches!(byte, b'(' | b'[') {
                delimiters.push(byte);
                expecting_operand = true;
                cursor = token_end;
                continue;
            }
            if delimiters.is_empty() {
                let operator_end = precedence_operator_end(bytes, token_start, end);
                if let Some((kind, precedence, associativity)) =
                    precedence_operator_info(bytes, token_start, operator_end)
                {
                    if expecting_operand
                        && is_unary_prefix_operator(bytes, token_start, operator_end)
                    {
                        cursor = operator_end;
                        continue;
                    }
                    operators.push(Operator {
                        start: token_start,
                        end: operator_end,
                        kind,
                        precedence,
                        associativity,
                    });
                    expecting_operand = true;
                    if operators.len() > 4 {
                        return Vec::new();
                    }
                    cursor = operator_end;
                    continue;
                }
            }
        }
        expecting_operand = false;
        cursor = token_end;
    }
    if !delimiters.is_empty() || operators.len() != 4 {
        return Vec::new();
    }

    let operands = [
        bounded_expression_span(bytes, start, operators[0].start),
        bounded_expression_span(bytes, operators[0].end, operators[1].start),
        bounded_expression_span(bytes, operators[1].end, operators[2].start),
        bounded_expression_span(bytes, operators[2].end, operators[3].start),
        bounded_expression_span(bytes, operators[3].end, end),
    ];
    let operands = match operands {
        [
            Some(first),
            Some(second),
            Some(third),
            Some(fourth),
            Some(fifth),
        ] => [first, second, third, fourth, fifth],
        _ => return Vec::new(),
    };

    let header =
        |operator: Operator, left: (u64, u64), right: (u64, u64)| ExpressionPrecedenceHeader {
            kind: operator.kind,
            precedence: operator.precedence,
            associativity: operator.associativity,
            left_start: left.0,
            left_end: left.1,
            operator_start: operator.start,
            operator_end: operator.end,
            right_start: right.0,
            right_end: right.1,
            depth: 0,
        };

    let mut operator_stack: Vec<usize> = Vec::with_capacity(4);
    let mut value_stack: Vec<(u64, u64)> = vec![operands[0]];
    let mut nodes: Vec<ExpressionPrecedenceHeader> = Vec::with_capacity(4);
    for operator_index in 0..4 {
        while let Some(&top_index) = operator_stack.last() {
            let top = operators[top_index];
            let current = operators[operator_index];
            let reduce_top = top.precedence > current.precedence
                || (top.precedence == current.precedence && current.associativity == 1);
            if !reduce_top {
                break;
            }
            let right = match value_stack.pop() {
                Some(value) => value,
                None => return Vec::new(),
            };
            let left = match value_stack.pop() {
                Some(value) => value,
                None => return Vec::new(),
            };
            operator_stack.pop();
            nodes.push(header(top, left, right));
            value_stack.push((left.0, right.1));
        }
        operator_stack.push(operator_index);
        value_stack.push(operands[operator_index + 1]);
    }
    while let Some(top_index) = operator_stack.pop() {
        let right = match value_stack.pop() {
            Some(value) => value,
            None => return Vec::new(),
        };
        let left = match value_stack.pop() {
            Some(value) => value,
            None => return Vec::new(),
        };
        nodes.push(header(operators[top_index], left, right));
        value_stack.push((left.0, right.1));
    }
    if nodes.len() != 4 || value_stack.len() != 1 {
        return Vec::new();
    }
    nodes
}

/// Copies the bounded four-operator precedence hand-off into caller-owned
/// storage. Truncation preserves post-order and never claims unwritten nodes.
#[must_use]
pub fn parse_expression_precedence_long_chain_headers_into(
    bytes: &[u8],
    start: u64,
    end: u64,
    output: &mut [ExpressionPrecedenceHeader],
) -> usize {
    let records = parse_expression_precedence_long_chain_headers(bytes, start, end);
    let count = records.len().min(output.len());
    output[..count].copy_from_slice(&records[..count]);
    count
}

fn bounded_expression_span(bytes: &[u8], start: u64, end: u64) -> Option<(u64, u64)> {
    let start = scan_token_info(bytes, start, end).1;
    if start >= end {
        return None;
    }
    let mut cursor = start;
    let mut last_end = start;
    while cursor < end {
        let (_, token_start, token_end) = scan_token_info(bytes, cursor, end);
        if token_start >= end || token_end <= token_start {
            break;
        }
        last_end = token_end;
        cursor = token_end;
    }
    (start < last_end).then_some((start, last_end))
}

fn precedence_operator_end(bytes: &[u8], start: u64, end: u64) -> u64 {
    let at = |offset: u64| -> Option<u8> {
        if offset < end {
            bytes.get(usize::try_from(offset).ok()?).copied()
        } else {
            None
        }
    };
    let first = at(start);
    let second = at(start + 1);
    let third = at(start + 2);
    if first == Some(b'.') && second == Some(b'.') && third == Some(b'<') {
        return start + 3;
    }
    if matches!(
        (first, second),
        (Some(b'='), Some(b'='))
            | (Some(b'!'), Some(b'='))
            | (Some(b'<'), Some(b'='))
            | (Some(b'>'), Some(b'='))
            | (Some(b'&'), Some(b'&'))
            | (Some(b'|'), Some(b'|'))
            | (Some(b'.'), Some(b'.'))
            | (Some(b'+'), Some(b'='))
            | (Some(b'-'), Some(b'='))
            | (Some(b'*'), Some(b'='))
            | (Some(b'/'), Some(b'='))
            | (Some(b'%'), Some(b'='))
    ) {
        return start + 2;
    }
    start + 1
}

fn precedence_operator_info(bytes: &[u8], start: u64, end: u64) -> Option<(u8, u8, u8)> {
    let width = end.checked_sub(start)?;
    let at = |offset: u64| -> Option<u8> { bytes.get(usize::try_from(offset).ok()?).copied() };
    let first = at(start)?;
    let second = at(start + 1);
    let third = at(start + 2);
    let precedence = match (width, first, second, third) {
        (3, b'.', Some(b'.'), Some(b'<')) => 6,
        (2, b'=', Some(b'='), _) | (2, b'!', Some(b'='), _) => 4,
        (2, b'<', Some(b'='), _) | (2, b'>', Some(b'='), _) => 5,
        (2, b'&', Some(b'&'), _) => 3,
        (2, b'|', Some(b'|'), _) => 2,
        (2, b'.', Some(b'.'), _) => 6,
        (2, b'+', Some(b'='), _)
        | (2, b'-', Some(b'='), _)
        | (2, b'*', Some(b'='), _)
        | (2, b'/', Some(b'='), _)
        | (2, b'%', Some(b'='), _)
        | (1, b'=', _, _) => 1,
        (1, b'<', _, _) | (1, b'>', _, _) => 5,
        (1, b'+', _, _) | (1, b'-', _, _) => 7,
        (1, b'*', _, _) | (1, b'/', _, _) | (1, b'%', _, _) => 8,
        (1, b'&', _, _) => 9,
        (1, b'^', _, _) | (1, b'|', _, _) => 10,
        _ => return None,
    };
    let kind = match precedence {
        1 => 1,
        3 | 4 | 5 | 10 => 4,
        _ => 2,
    };
    let associativity = if precedence == 1 { 2 } else { 1 };
    Some((kind, precedence, associativity))
}

fn is_unary_prefix_operator(bytes: &[u8], start: u64, end: u64) -> bool {
    if end.saturating_sub(start) != 1 {
        return false;
    }
    let Ok(index) = usize::try_from(start) else {
        return false;
    };
    matches!(
        bytes.get(index),
        Some(b'+' | b'-' | b'!' | b'~' | b'&' | b'*')
    )
}

/// Rust reference for the narrow builtin literal typing hand-off.
#[must_use]
pub fn infer_literal_types(bytes: &[u8], start: u64, end: u64) -> Vec<TypedExpressionHeader> {
    let mut source_index = start;
    let mut depth: u64 = 0;
    let mut output = Vec::new();
    while source_index < end {
        let (kind, token_start, next_source) = scan_token_info(bytes, source_index, end);
        if token_start >= end {
            break;
        }
        let symbol = if kind == 3 {
            bytes[usize::try_from(token_start).expect("validated literal symbol index")]
        } else {
            0
        };
        if symbol == b')' || symbol == b']' {
            depth = depth.saturating_sub(1);
            source_index = next_source;
            continue;
        }
        if symbol == b'(' || symbol == b'[' {
            depth += 1;
            source_index = next_source;
            continue;
        }

        let token_start_index = usize::try_from(token_start).expect("validated literal start");
        let next_source_index = usize::try_from(next_source).expect("validated literal end");
        if kind == 1 {
            let token = &bytes[token_start_index..next_source_index];
            if token == b"true" || token == b"false" {
                output.push(TypedExpressionHeader {
                    expression_kind: 1,
                    type_kind: 1,
                    start: token_start,
                    end: next_source,
                    depth,
                });
                source_index = next_source;
                continue;
            }
        }
        if kind == 2 {
            let mut type_kind = 2;
            let mut literal_end = next_source;
            if next_source < end
                && bytes[next_source_index] == b'.'
                && next_source + 1 < end
                && bytes[next_source_index + 1] != b'.'
            {
                let (fraction_kind, fraction_start, fraction_end) =
                    scan_token_info(bytes, next_source + 1, end);
                if fraction_kind == 2 && fraction_start == next_source + 1 {
                    type_kind = 3;
                    literal_end = fraction_end;
                }
            }
            output.push(TypedExpressionHeader {
                expression_kind: 1,
                type_kind,
                start: token_start,
                end: literal_end,
                depth,
            });
            source_index = literal_end;
            continue;
        }
        if symbol == b'"' || symbol == b'\'' {
            let mut cursor = next_source;
            let mut escaped = false;
            let mut literal_end = end;
            while cursor < end {
                let (_, quoted_start, quoted_end) = scan_token_info(bytes, cursor, end);
                if quoted_start >= end {
                    break;
                }
                let quoted_index = usize::try_from(quoted_start).expect("validated quote index");
                let quoted_symbol = bytes[quoted_index];
                if escaped {
                    escaped = false;
                } else if quoted_symbol == b'\\' {
                    escaped = true;
                } else if quoted_symbol == symbol {
                    literal_end = quoted_end;
                    break;
                }
                cursor = quoted_end;
            }
            output.push(TypedExpressionHeader {
                expression_kind: 1,
                type_kind: if symbol == b'"' { 4 } else { 5 },
                start: token_start,
                end: literal_end,
                depth,
            });
            source_index = literal_end;
            continue;
        }
        source_index = next_source;
    }
    output
}

fn bounded_literal_type_kind(bytes: &[u8], start: u64, end: u64) -> u8 {
    let (kind, token_start, next_source) = scan_token_info(bytes, start, end);
    if token_start >= end {
        return 0;
    }
    let token_start_index = usize::try_from(token_start).expect("validated binary token start");
    let next_source_index = usize::try_from(next_source).expect("validated binary token end");
    if kind == 1 {
        let token = &bytes[token_start_index..next_source_index];
        if (token == b"true" || token == b"false")
            && scan_token_info(bytes, next_source, end).1 >= end
        {
            return 1;
        }
        return 0;
    }
    if kind == 2 {
        let mut type_kind = 2;
        let mut literal_end = next_source;
        if next_source < end
            && bytes[next_source_index] == b'.'
            && next_source + 1 < end
            && bytes[next_source_index + 1] != b'.'
        {
            let (fraction_kind, fraction_start, fraction_end) =
                scan_token_info(bytes, next_source + 1, end);
            if fraction_kind == 2 && fraction_start == next_source + 1 {
                type_kind = 3;
                literal_end = fraction_end;
            }
        }
        if scan_token_info(bytes, literal_end, end).1 >= end {
            return type_kind;
        }
        return 0;
    }
    if kind != 3 {
        return 0;
    }
    let quote = bytes[token_start_index];
    if quote != b'"' && quote != b'\'' {
        return 0;
    }
    let mut cursor = next_source;
    let mut escaped = false;
    let mut literal_end = end;
    while cursor < end {
        let (_, quoted_start, quoted_end) = scan_token_info(bytes, cursor, end);
        if quoted_start >= end {
            break;
        }
        let quoted_index = usize::try_from(quoted_start).expect("validated binary quote index");
        let quoted_symbol = bytes[quoted_index];
        if escaped {
            escaped = false;
        } else if quoted_symbol == b'\\' {
            escaped = true;
        } else if quoted_symbol == quote {
            literal_end = quoted_end;
            break;
        }
        cursor = quoted_end;
    }
    if scan_token_info(bytes, literal_end, end).1 >= end {
        return if quote == b'"' { 4 } else { 5 };
    }
    0
}

fn bounded_grouped_literal_span(bytes: &[u8], start: u64, end: u64) -> Option<(u8, u64, u64)> {
    let (_, open_start, open_end) = scan_token_info(bytes, start, end);
    if open_start >= end
        || bytes[usize::try_from(open_start).expect("validated group opener")] != b'('
    {
        return None;
    }

    let mut group_depth = 1_u64;
    let mut literal_start = open_end;
    let mut cursor = literal_start;
    loop {
        let (_, nested_start, nested_end) = scan_token_info(bytes, cursor, end);
        if nested_start >= end {
            return None;
        }
        let nested_symbol = bytes[usize::try_from(nested_start).expect("validated nested group")];
        if nested_symbol == b'(' {
            group_depth += 1;
            cursor = nested_end;
        } else {
            literal_start = nested_start;
            cursor = literal_start;
            break;
        }
    }

    let mut literal_end = None;
    let mut literal_type = 0;
    while cursor < end {
        let (_, candidate_start, candidate_end) = scan_token_info(bytes, cursor, end);
        if candidate_start >= end {
            return None;
        }
        if bytes[usize::try_from(candidate_start).expect("validated group candidate")] == b')' {
            return None;
        }
        let candidate_type = bounded_literal_type_kind(bytes, literal_start, candidate_end);
        if candidate_type != 0 {
            literal_end = Some(candidate_end);
            literal_type = candidate_type;
            break;
        }
        cursor = candidate_end;
    }
    let literal_end = literal_end?;
    let mut close_cursor = literal_end;
    for _ in 0..group_depth {
        let (_, close_start, close_end) = scan_token_info(bytes, close_cursor, end);
        if close_start >= end
            || bytes[usize::try_from(close_start).expect("validated group closer")] != b')'
        {
            return None;
        }
        close_cursor = close_end;
    }
    if scan_token_info(bytes, close_cursor, end).1 >= end {
        Some((literal_type, close_cursor, group_depth))
    } else {
        None
    }
}

fn binary_operator_end(bytes: &[u8], start: u64, end: u64) -> u64 {
    if start + 1 < end {
        let index = usize::try_from(start).expect("validated binary operator start");
        let next = bytes[index + 1];
        if matches!(
            (bytes[index], next),
            (b'=', b'=') | (b'!', b'=') | (b'<', b'=') | (b'>', b'=') | (b'&', b'&') | (b'|', b'|')
        ) {
            return start + 2;
        }
    }
    start + 1
}

fn bounded_numeric_literal_end(bytes: &[u8], start: u64, end: u64) -> u64 {
    let (kind, _, token_end) = scan_token_info(bytes, start, end);
    if kind != 2 || token_end >= end {
        return token_end;
    }
    let token_end_index = usize::try_from(token_end).expect("validated numeric literal end");
    if bytes[token_end_index] != b'.' || token_end + 1 >= end || bytes[token_end_index + 1] == b'.'
    {
        return token_end;
    }
    let (fraction_kind, fraction_start, fraction_end) = scan_token_info(bytes, token_end + 1, end);
    if fraction_kind == 2 && fraction_start == token_end + 1 {
        fraction_end
    } else {
        token_end
    }
}

fn binary_operator_kind(bytes: &[u8], start: u64, operator_end: u64) -> u8 {
    let index = usize::try_from(start).expect("validated binary operator kind start");
    let width = operator_end - start;
    if width == 2 {
        return match (bytes[index], bytes[index + 1]) {
            (b'=', b'=') | (b'!', b'=') | (b'<', b'=') | (b'>', b'=') => 3,
            (b'&', b'&') | (b'|', b'|') => 4,
            _ => 0,
        };
    }
    match bytes[index] {
        b'+' | b'-' | b'*' | b'/' | b'%' => 2,
        b'<' | b'>' => 3,
        _ => 0,
    }
}

fn bounded_binary_literal_operand(bytes: &[u8], start: u64, end: u64) -> Option<(u8, u64)> {
    if let Some((type_kind, grouped_end, _)) = bounded_grouped_literal_span(bytes, start, end) {
        return Some((type_kind, grouped_end));
    }
    let type_kind = bounded_literal_type_kind(bytes, start, end);
    if type_kind == 0 {
        return None;
    }
    let operand_end = if type_kind == 3 {
        bounded_numeric_literal_end(bytes, start, end)
    } else {
        scan_token_info(bytes, start, end).2
    };
    Some((type_kind, operand_end))
}

/// Recognizes exactly two top-level binary operators over builtin literal
/// operands. Arithmetic operators may mix with each other, and one arithmetic
/// operator may feed a numeric comparison; the result type follows the
/// bounded precedence shape without introducing a new ABI table.
fn infer_binary_literal_chain(bytes: &[u8], start: u64, end: u64) -> Option<TypedExpressionHeader> {
    let mut source_index = start;
    let mut depth = 0_u64;
    let mut operator_count = 0_u8;
    let mut first_kind = 0_u8;
    let mut second_kind = 0_u8;
    let mut first_operator_start = 0_u64;
    let mut first_operator_end = 0_u64;
    let mut second_operator_start = 0_u64;
    let mut second_operator_end = 0_u64;
    while source_index < end {
        let (kind, token_start, token_end) = scan_token_info(bytes, source_index, end);
        if token_start >= end {
            break;
        }
        let symbol = if kind == 3 {
            bytes[usize::try_from(token_start).expect("validated chain symbol")]
        } else {
            0
        };
        match symbol {
            b'(' | b'[' => {
                depth += 1;
                source_index = token_end;
                continue;
            }
            b')' | b']' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
                source_index = token_end;
                continue;
            }
            _ => {}
        }
        if depth == 0 && kind == 3 {
            let operator_end = binary_operator_end(bytes, token_start, end);
            let candidate_kind = binary_operator_kind(bytes, token_start, operator_end);
            if candidate_kind == 0 {
                return None;
            }
            if operator_count == 0 {
                first_kind = candidate_kind;
                first_operator_start = token_start;
                first_operator_end = operator_end;
            } else if operator_count == 1 {
                second_kind = candidate_kind;
                second_operator_start = token_start;
                second_operator_end = operator_end;
            } else {
                return None;
            }
            operator_count += 1;
            source_index = operator_end;
            continue;
        }
        source_index = token_end;
    }
    if depth != 0 || operator_count != 2 {
        return None;
    }

    let (left_type, left_end) = bounded_binary_literal_operand(bytes, start, first_operator_start)?;
    let (middle_type, middle_end) =
        bounded_binary_literal_operand(bytes, first_operator_end, second_operator_start)?;
    let (right_type, right_end) = bounded_binary_literal_operand(bytes, second_operator_end, end)?;
    if left_end > first_operator_start || middle_end > second_operator_start || right_end > end {
        return None;
    }
    let result_type = if first_kind == 2
        && second_kind == 2
        && matches!(left_type, 2 | 3)
        && matches!(middle_type, 2 | 3)
        && matches!(right_type, 2 | 3)
    {
        if left_type == 3 || middle_type == 3 || right_type == 3 {
            3
        } else {
            2
        }
    } else if (((first_kind == 2 && second_kind == 3) || (first_kind == 3 && second_kind == 2))
        && matches!(left_type, 2 | 3)
        && matches!(middle_type, 2 | 3)
        && matches!(right_type, 2 | 3))
        || (first_kind == 4
            && second_kind == 4
            && left_type == 1
            && middle_type == 1
            && right_type == 1)
    {
        1
    } else {
        return None;
    };
    let (_, expression_start, _) = scan_token_info(bytes, start, first_operator_start);
    Some(TypedExpressionHeader {
        expression_kind: 2,
        type_kind: result_type,
        start: expression_start,
        end: right_end,
        depth: 0,
    })
}

/// Rust reference for one binary typed-semantics hand-off over builtin
/// literal or one-or-more nested parenthesized builtin-literal operands. It
/// accepts one top-level binary expression or a bounded two-operator chain;
/// arithmetic/compare combinations use the builtin precedence categories while
/// names and user-type resolution remain later stages.
#[must_use]
pub fn infer_binary_literal_types(
    bytes: &[u8],
    start: u64,
    end: u64,
) -> Vec<TypedExpressionHeader> {
    let mut source_index = start;
    let mut depth: u64 = 0;
    while source_index < end {
        let (kind, token_start, next_source) = scan_token_info(bytes, source_index, end);
        if token_start >= end {
            break;
        }
        let symbol = if kind == 3 {
            bytes[usize::try_from(token_start).expect("validated binary symbol index")]
        } else {
            0
        };
        match symbol {
            b')' | b']' => {
                depth = depth.saturating_sub(1);
                source_index = next_source;
                continue;
            }
            b'(' | b'[' => {
                depth += 1;
                source_index = next_source;
                continue;
            }
            _ => {}
        }
        if depth == 0 && kind == 3 {
            let operator_end = binary_operator_end(bytes, token_start, end);
            let operator_kind = binary_operator_kind(bytes, token_start, operator_end);
            if operator_kind != 0 {
                if let Some(chained) = infer_binary_literal_chain(bytes, start, end) {
                    return vec![chained];
                }
                let left_grouped = bounded_grouped_literal_span(bytes, start, token_start);
                let right_grouped = bounded_grouped_literal_span(bytes, operator_end, end);
                let left_type = left_grouped
                    .map(|(type_kind, _, _)| type_kind)
                    .unwrap_or_else(|| bounded_literal_type_kind(bytes, start, token_start));
                let right_type = right_grouped
                    .map(|(type_kind, _, _)| type_kind)
                    .unwrap_or_else(|| bounded_literal_type_kind(bytes, operator_end, end));
                let result_type = match operator_kind {
                    2 if matches!(left_type, 2 | 3) && matches!(right_type, 2 | 3) => {
                        if left_type == 3 || right_type == 3 {
                            3
                        } else {
                            2
                        }
                    }
                    3 if matches!(left_type, 2 | 3) && matches!(right_type, 2 | 3) => 1,
                    4 if left_type == 1 && right_type == 1 => 1,
                    _ => 0,
                };
                if result_type != 0 {
                    let (_, left_start, _) = scan_token_info(bytes, start, token_start);
                    let right_end = if let Some((_, grouped_end, _)) = right_grouped {
                        grouped_end
                    } else {
                        let (_, _, right_token_end) = scan_token_info(bytes, operator_end, end);
                        if right_type == 3 {
                            bounded_numeric_literal_end(bytes, operator_end, end)
                        } else {
                            right_token_end
                        }
                    };
                    return vec![TypedExpressionHeader {
                        expression_kind: 2,
                        type_kind: result_type,
                        start: left_start,
                        end: right_end,
                        depth,
                    }];
                }
            }
        }
        source_index = next_source;
    }
    Vec::new()
}

fn bounded_numeric_cast_target_kind(bytes: &[u8], start: u64, end: u64) -> u8 {
    let (kind, token_start, token_end) = scan_token_info(bytes, start, end);
    if kind != 1 {
        return 0;
    }
    let start = usize::try_from(token_start).expect("validated cast target start");
    let end = usize::try_from(token_end).expect("validated cast target end");
    let target_kind = match bytes.get(start..end) {
        Some(
            b"Int8" | b"Int16" | b"Int32" | b"Int64" | b"UInt8" | b"UInt16" | b"UInt32" | b"UInt64",
        ) => 2,
        Some(b"Float32" | b"Float64") => 3,
        _ => 0,
    };
    if target_kind == 0 {
        return 0;
    }
    target_kind
}

fn bounded_numeric_cast_step(bytes: &[u8], start: u64, end: u64) -> Option<(u8, u64)> {
    let (as_kind, as_start, as_end) = scan_token_info(bytes, start, end);
    if as_kind != 1
        || bytes.get(usize::try_from(as_start).ok()?..usize::try_from(as_end).ok()?) != Some(b"as")
    {
        return None;
    }
    let (target_kind, target_start, target_end) = scan_token_info(bytes, as_end, end);
    let type_kind = bounded_numeric_cast_target_kind(bytes, as_end, end);
    (target_kind == 1 && target_start < end && type_kind != 0).then_some((type_kind, target_end))
}

fn is_close_token(bytes: &[u8], start: u64, end: u64) -> bool {
    let (kind, token_start, token_end) = scan_token_info(bytes, start, end);
    let Ok(index) = usize::try_from(token_start) else {
        return false;
    };
    kind == 3 && token_end == token_start + 1 && bytes.get(index) == Some(&b')')
}

fn is_as_token(bytes: &[u8], start: u64, end: u64) -> bool {
    let (kind, token_start, token_end) = scan_token_info(bytes, start, end);
    let (Ok(start), Ok(end)) = (usize::try_from(token_start), usize::try_from(token_end)) else {
        return false;
    };
    kind == 1 && bytes.get(start..end) == Some(b"as")
}

/// Rust reference for one explicit numeric cast over a builtin numeric
/// literal. The expression is intentionally bounded to `literal as Type`;
/// names, grouped operands, chained casts and user-defined types remain later
/// resolver/type-check stages.
#[must_use]
pub fn infer_cast_literal_types(bytes: &[u8], start: u64, end: u64) -> Vec<TypedExpressionHeader> {
    let (kind, expression_start, first_end) = scan_token_info(bytes, start, end);
    if expression_start >= end {
        return Vec::new();
    }
    let mut cursor = first_end;
    let mut depth = 0_u64;
    let literal_start;
    if kind == 2 {
        literal_start = expression_start;
        cursor = bounded_numeric_literal_end(bytes, literal_start, end);
    } else if kind == 3
        && bytes.get(usize::try_from(expression_start).expect("validated cast opener"))
            == Some(&b'(')
    {
        depth = 1;
        loop {
            let (open_kind, open_start, open_end) = scan_token_info(bytes, cursor, end);
            if open_kind != 3
                || open_start >= end
                || bytes.get(usize::try_from(open_start).expect("validated cast opener"))
                    != Some(&b'(')
            {
                break;
            }
            depth += 1;
            if depth > 32 {
                return Vec::new();
            }
            cursor = open_end;
        }
        let (literal_kind, grouped_literal_start, _) = scan_token_info(bytes, cursor, end);
        if literal_kind != 2 || grouped_literal_start >= end {
            return Vec::new();
        }
        literal_start = grouped_literal_start;
        cursor = bounded_numeric_literal_end(bytes, literal_start, end);
    } else {
        return Vec::new();
    }
    if cursor <= literal_start {
        return Vec::new();
    }
    let mut cast_count = 0_u64;
    let mut target_kind = 0_u8;
    let mut target_end = cursor;
    let mut groups_closed = depth == 0;
    loop {
        let (_, token_start, _) = scan_token_info(bytes, cursor, end);
        if token_start >= end {
            if !groups_closed || target_kind == 0 {
                return Vec::new();
            }
            break;
        }
        if !groups_closed && is_close_token(bytes, cursor, end) {
            for _ in 0..depth {
                let (close_kind, close_start, close_end) = scan_token_info(bytes, cursor, end);
                if close_kind != 3
                    || close_start >= end
                    || bytes.get(usize::try_from(close_start).expect("validated cast closer"))
                        != Some(&b')')
                {
                    return Vec::new();
                }
                cursor = close_end;
            }
            groups_closed = true;
            continue;
        }
        let Some((candidate_kind, candidate_token_end)) =
            bounded_numeric_cast_step(bytes, cursor, end)
        else {
            return Vec::new();
        };
        cast_count += 1;
        if cast_count > 4 {
            return Vec::new();
        }
        target_kind = candidate_kind;
        target_end = candidate_token_end;
        cursor = candidate_token_end;
        let (_, trailing_start, _) = scan_token_info(bytes, cursor, end);
        if trailing_start >= end {
            if !groups_closed {
                return Vec::new();
            }
            break;
        }
        if groups_closed && !is_as_token(bytes, cursor, end) {
            return Vec::new();
        }
    }

    vec![TypedExpressionHeader {
        expression_kind: 4,
        type_kind: target_kind,
        start: expression_start,
        end: target_end,
        depth,
    }]
}

/// Rust reference for one unary typed-semantics hand-off over a builtin
/// literal operand or one parenthesized literal group. Contiguous prefix
/// chains are applied right-to-left; prefix plus/minus preserve numeric type,
/// logical not yields bool, and bit-not accepts integers only. ASCII
/// whitespace between prefix operators is accepted, while names and casts
/// remain outside this stage.
#[must_use]
pub fn infer_unary_literal_types(bytes: &[u8], start: u64, end: u64) -> Vec<TypedExpressionHeader> {
    let (kind, operator_start, operator_end) = scan_token_info(bytes, start, end);
    if kind != 3 || operator_start >= end || operator_end != operator_start + 1 {
        return Vec::new();
    }
    let first_operator = bytes[usize::try_from(operator_start).expect("validated unary operator")];
    if !matches!(first_operator, b'+' | b'-' | b'!' | b'~') {
        return Vec::new();
    }
    let mut operand_start = operator_end;
    let mut chain_cursor = operator_end;
    while chain_cursor < end {
        let (_, token_start, token_end) = scan_token_info(bytes, chain_cursor, end);
        if token_start >= end || token_end != token_start + 1 {
            break;
        }
        let token = bytes[usize::try_from(token_start).expect("validated unary prefix")];
        if !matches!(token, b'+' | b'-' | b'!' | b'~') {
            break;
        }
        operand_start = token_end;
        chain_cursor = token_end;
    }
    let grouped = bounded_grouped_literal_span(bytes, operand_start, end);
    let operand_type = grouped
        .map(|(type_kind, _, _)| type_kind)
        .unwrap_or_else(|| bounded_literal_type_kind(bytes, operand_start, end));
    let mut result_type = operand_type;
    let mut operator_cursor = operand_start;
    while operator_cursor > operator_start {
        operator_cursor -= 1;
        if classify_byte(
            bytes[usize::try_from(operator_cursor).expect("validated unary whitespace")],
        ) == 3
        {
            continue;
        }
        let operator = bytes[usize::try_from(operator_cursor).expect("validated unary chain")];
        result_type = match operator {
            b'+' | b'-' if matches!(result_type, 2 | 3) => result_type,
            b'!' if result_type == 1 => 1,
            b'~' if result_type == 2 => 2,
            _ => 0,
        };
        if result_type == 0 {
            break;
        }
    }
    if result_type == 0 {
        return Vec::new();
    }
    let (_, _, operand_token_end) = scan_token_info(bytes, operand_start, end);
    let (operand_end, depth) = if let Some((_, grouped_end, grouped_depth)) = grouped {
        (grouped_end, grouped_depth)
    } else if operand_type == 2 || operand_type == 3 {
        (bounded_numeric_literal_end(bytes, operand_start, end), 0)
    } else {
        (operand_token_end, 0)
    };
    vec![TypedExpressionHeader {
        expression_kind: 3,
        type_kind: result_type,
        start: operator_start,
        end: operand_end,
        depth,
    }]
}

/// Copies the unary typed-semantics hand-off into caller-owned storage. An
/// empty output slice returns zero even when the expression is valid.
#[must_use]
pub fn infer_unary_literal_types_into(
    bytes: &[u8],
    start: u64,
    end: u64,
    output: &mut [TypedExpressionHeader],
) -> usize {
    let records = infer_unary_literal_types(bytes, start, end);
    let count = records.len().min(output.len());
    output[..count].copy_from_slice(&records[..count]);
    count
}

/// Rust reference for the bounded caller-owned name binding hand-off. Each
/// identifier token is matched by exact bytes against the supplied binding
/// spans; results preserve source order and report delimiter depth.
#[must_use]
pub fn infer_name_types(
    bytes: &[u8],
    start: u64,
    end: u64,
    bindings: &[TypedNameBindingHeader],
) -> Vec<TypedExpressionHeader> {
    if start > end || end > bytes.len() as u64 {
        return Vec::new();
    }
    let mut source_index = start;
    let mut depth = 0_u64;
    let mut output = Vec::new();
    while source_index < end {
        let (kind, token_start, token_end) = scan_token_info(bytes, source_index, end);
        if token_start >= end {
            break;
        }
        if kind == 3 {
            match bytes[usize::try_from(token_start).expect("validated symbol start")] {
                b')' | b']' => depth = depth.saturating_sub(1),
                b'(' | b'[' => {
                    depth = depth.saturating_add(1);
                    source_index = token_end;
                    continue;
                }
                _ => {}
            }
            if matches!(
                bytes[usize::try_from(token_start).expect("validated symbol start")],
                b')' | b']'
            ) {
                source_index = token_end;
                continue;
            }
        }
        if kind == 1 {
            for binding in bindings {
                if binding.type_kind == 0
                    || binding.type_kind > 5
                    || binding.name_start >= binding.name_end
                    || binding.name_end > bytes.len() as u64
                {
                    continue;
                }
                if binding.name_end - binding.name_start == token_end - token_start {
                    let binding_start =
                        usize::try_from(binding.name_start).expect("validated binding start");
                    let binding_end =
                        usize::try_from(binding.name_end).expect("validated binding end");
                    let token_start = usize::try_from(token_start).expect("validated token start");
                    let token_end = usize::try_from(token_end).expect("validated token end");
                    if bytes[binding_start..binding_end] == bytes[token_start..token_end] {
                        output.push(TypedExpressionHeader {
                            expression_kind: 5,
                            type_kind: binding.type_kind,
                            start: token_start as u64,
                            end: token_end as u64,
                            depth,
                        });
                        break;
                    }
                }
            }
        }
        source_index = token_end;
    }
    output
}

/// Copies typed-name records into a caller-owned output slice and returns the
/// number written. This mirrors the bounded JDN/C export while keeping the
/// allocating `Vec` reference useful for the oracle corpus.
#[must_use]
pub fn infer_name_types_into(
    bytes: &[u8],
    start: u64,
    end: u64,
    bindings: &[TypedNameBindingHeader],
    output: &mut [TypedExpressionHeader],
) -> usize {
    let records = infer_name_types(bytes, start, end, bindings);
    let count = records.len().min(output.len());
    output[..count].copy_from_slice(&records[..count]);
    count
}

/// Rust reference for the bounded lexical-depth name binding hand-off. The
/// deepest visible matching binding wins and later entries win equal-depth
/// ties. This intentionally stops short of region-aware scope resolution.
#[must_use]
pub fn infer_scoped_name_types(
    bytes: &[u8],
    start: u64,
    end: u64,
    bindings: &[TypedScopedNameBindingHeader],
) -> Vec<TypedExpressionHeader> {
    if start > end || end > bytes.len() as u64 {
        return Vec::new();
    }
    let mut source_index = start;
    let mut depth = 0_u64;
    let mut output = Vec::new();
    while source_index < end {
        let (kind, token_start, token_end) = scan_token_info(bytes, source_index, end);
        if token_start >= end {
            break;
        }
        if kind == 3 {
            match bytes[usize::try_from(token_start).expect("validated symbol start")] {
                b')' | b']' => depth = depth.saturating_sub(1),
                b'(' | b'[' => {
                    depth = depth.saturating_add(1);
                    source_index = token_end;
                    continue;
                }
                _ => {}
            }
            if matches!(
                bytes[usize::try_from(token_start).expect("validated symbol start")],
                b')' | b']'
            ) {
                source_index = token_end;
                continue;
            }
        }
        if kind == 1 {
            let mut selected: Option<(u64, u8)> = None;
            for binding in bindings {
                if binding.type_kind == 0
                    || binding.type_kind > 5
                    || binding.scope_depth > depth
                    || binding.name_start >= binding.name_end
                    || binding.name_end > bytes.len() as u64
                {
                    continue;
                }
                if binding.name_end - binding.name_start != token_end - token_start {
                    continue;
                }
                let binding_start =
                    usize::try_from(binding.name_start).expect("validated binding start");
                let binding_end = usize::try_from(binding.name_end).expect("validated binding end");
                let token_start_usize =
                    usize::try_from(token_start).expect("validated token start");
                let token_end_usize = usize::try_from(token_end).expect("validated token end");
                if bytes[binding_start..binding_end] != bytes[token_start_usize..token_end_usize] {
                    continue;
                }
                if selected
                    .map(|(selected_scope, _)| binding.scope_depth >= selected_scope)
                    .unwrap_or(true)
                {
                    selected = Some((binding.scope_depth, binding.type_kind));
                }
            }
            if let Some((_, type_kind)) = selected {
                output.push(TypedExpressionHeader {
                    expression_kind: 6,
                    type_kind,
                    start: token_start,
                    end: token_end,
                    depth,
                });
            }
        }
        source_index = token_end;
    }
    output
}

/// Copies scoped-name records into a caller-owned output slice and returns the
/// number written. The truncation rule is intentionally identical to the
/// region-name helper and the bounded Jadren export.
#[must_use]
pub fn infer_scoped_name_types_into(
    bytes: &[u8],
    start: u64,
    end: u64,
    bindings: &[TypedScopedNameBindingHeader],
    output: &mut [TypedExpressionHeader],
) -> usize {
    let records = infer_scoped_name_types(bytes, start, end, bindings);
    let count = records.len().min(output.len());
    output[..count].copy_from_slice(&records[..count]);
    count
}

/// Rust reference for bounded region-aware name resolution. A binding is
/// eligible only when its name and visibility region are valid, the token is
/// inside that region, and its lexical depth is visible at the token. The
/// deepest eligible binding wins and later entries win equal-depth ties.
#[must_use]
pub fn infer_region_name_types(
    bytes: &[u8],
    start: u64,
    end: u64,
    bindings: &[TypedRegionNameBindingHeader],
) -> Vec<TypedExpressionHeader> {
    if start > end || end > bytes.len() as u64 {
        return Vec::new();
    }
    if !delimiters_well_formed_through(bytes, end) {
        return Vec::new();
    }
    let mut source_index = start;
    let mut depth = 0_u64;
    let mut output = Vec::new();
    while source_index < end {
        let (kind, token_start, token_end) = scan_token_info(bytes, source_index, end);
        if token_start >= end {
            break;
        }
        if kind == 3 {
            let symbol = bytes[usize::try_from(token_start).expect("validated symbol start")];
            match symbol {
                b')' | b']' => depth = depth.saturating_sub(1),
                b'(' | b'[' => {
                    depth = depth.saturating_add(1);
                    source_index = token_end;
                    continue;
                }
                _ => {}
            }
            if matches!(symbol, b')' | b']') {
                source_index = token_end;
                continue;
            }
        }
        if kind == 1 {
            let mut selected: Option<(u64, u64, u8)> = None;
            for binding in bindings {
                if binding.type_kind == 0
                    || binding.type_kind > 5
                    || binding.name_start >= binding.name_end
                    || binding.name_end > bytes.len() as u64
                    || binding.region_start >= binding.region_end
                    || binding.region_end > bytes.len() as u64
                    || binding.name_start < binding.region_start
                    || binding.name_end > binding.region_end
                    || binding.region_start > token_start
                    || token_end > binding.region_end
                    || binding.scope_depth > depth
                    || delimiter_depth_at(bytes, binding.region_start) != Some(binding.scope_depth)
                    || delimiter_depth_at(bytes, binding.region_end) != Some(binding.scope_depth)
                    || delimiter_top_at(bytes, binding.region_start)
                        != delimiter_top_at(bytes, binding.region_end)
                {
                    continue;
                }
                if binding.name_end - binding.name_start != token_end - token_start {
                    continue;
                }
                let binding_start =
                    usize::try_from(binding.name_start).expect("validated binding start");
                let binding_end = usize::try_from(binding.name_end).expect("validated binding end");
                let token_start_usize =
                    usize::try_from(token_start).expect("validated token start");
                let token_end_usize = usize::try_from(token_end).expect("validated token end");
                if bytes[binding_start..binding_end] != bytes[token_start_usize..token_end_usize] {
                    continue;
                }
                let region_width = binding.region_end - binding.region_start;
                if selected
                    .map(|(selected_scope, selected_width, _)| {
                        binding.scope_depth > selected_scope
                            || (binding.scope_depth == selected_scope
                                && region_width <= selected_width)
                    })
                    .unwrap_or(true)
                {
                    selected = Some((binding.scope_depth, region_width, binding.type_kind));
                }
            }
            if let Some((_, _, type_kind)) = selected {
                output.push(TypedExpressionHeader {
                    expression_kind: 7,
                    type_kind,
                    start: token_start,
                    end: token_end,
                    depth,
                });
            }
        }
        source_index = token_end;
    }
    output
}

/// Copies region-name records into a caller-owned output slice and returns the
/// number written. The oracle keeps its allocating `Vec` reference above, while
/// this helper mirrors the bounded C/JDN export's truncation rule.
#[must_use]
pub fn infer_region_name_types_into(
    bytes: &[u8],
    start: u64,
    end: u64,
    bindings: &[TypedRegionNameBindingHeader],
    output: &mut [TypedExpressionHeader],
) -> usize {
    let records = infer_region_name_types(bytes, start, end, bindings);
    let count = records.len().min(output.len());
    output[..count].copy_from_slice(&records[..count]);
    count
}

/// Returns whether every delimiter in the source prefix is properly nested.
///
/// Region resolution is a metadata hand-off, not a recovery parser.  Rejecting
/// a malformed prefix before emitting names keeps the Rust oracle and the
/// bounded Jadren implementation from resolving identifiers after an unmatched
/// or mismatched delimiter.
fn delimiters_well_formed_through(bytes: &[u8], position: u64) -> bool {
    if position > bytes.len() as u64 {
        return false;
    }
    let mut source_index = 0_u64;
    let mut stack = [0_u8; 32];
    let mut stack_len = 0_usize;
    while source_index < position {
        let (kind, token_start, token_end) = scan_token_info(bytes, source_index, position);
        if token_start >= position {
            break;
        }
        if kind == 3 {
            let symbol = bytes[usize::try_from(token_start).expect("validated symbol start")];
            match symbol {
                b'(' | b'[' => {
                    if stack_len >= stack.len() {
                        return false;
                    }
                    stack[stack_len] = symbol;
                    stack_len += 1;
                }
                b')' => {
                    if stack_len == 0 || stack[stack_len - 1] != b'(' {
                        return false;
                    }
                    stack_len -= 1;
                }
                b']' => {
                    if stack_len == 0 || stack[stack_len - 1] != b'[' {
                        return false;
                    }
                    stack_len -= 1;
                }
                _ => {}
            }
        }
        source_index = token_end;
    }
    true
}

/// Returns the delimiter depth immediately before `position`, rejecting a
/// position that lies after an unmatched closing delimiter. Region bindings
/// must start and end at the same lexical depth as their declared scope so a
/// visibility span cannot cut through a nested block.
fn delimiter_depth_at(bytes: &[u8], position: u64) -> Option<u64> {
    if position > bytes.len() as u64 {
        return None;
    }
    let mut source_index = 0_u64;
    let mut depth = 0_u64;
    while source_index < position {
        let (kind, token_start, token_end) = scan_token_info(bytes, source_index, position);
        if token_start >= position {
            break;
        }
        if kind == 3 {
            let symbol = bytes[usize::try_from(token_start).expect("validated symbol start")];
            match symbol {
                b')' | b']' => {
                    if depth == 0 {
                        return None;
                    }
                    depth -= 1;
                }
                b'(' | b'[' => depth += 1,
                _ => {}
            }
        }
        source_index = token_end;
    }
    Some(depth)
}

/// Returns the top delimiter immediately before `position`. The exact top
/// kind is checked in addition to numeric depth so a malformed `(`/`[` pair
/// cannot be accepted merely because it leaves the same depth value.
fn delimiter_top_at(bytes: &[u8], position: u64) -> Option<u8> {
    if position > bytes.len() as u64 {
        return None;
    }
    let mut source_index = 0_u64;
    let mut stack = Vec::new();
    while source_index < position {
        let (kind, token_start, token_end) = scan_token_info(bytes, source_index, position);
        if token_start >= position {
            break;
        }
        if kind == 3 {
            let symbol = bytes[usize::try_from(token_start).expect("validated symbol start")];
            match symbol {
                b'(' | b'[' => {
                    if stack.len() >= 32 {
                        return None;
                    }
                    stack.push(symbol);
                }
                b')' => match stack.pop() {
                    Some(b'(') => {}
                    _ => return None,
                },
                b']' => match stack.pop() {
                    Some(b'[') => {}
                    _ => return None,
                },
                _ => {}
            }
        }
        source_index = token_end;
    }
    Some(stack.last().copied().unwrap_or(0))
}

/// Rust reference for the allocation-free token stream reducer.
#[must_use]
pub fn count_tokens(bytes: &[u8], start: u64, end: u64) -> TokenCounts {
    let mut index = start;
    let mut counts = TokenCounts {
        identifiers: 0,
        numbers: 0,
        symbols: 0,
        total: 0,
    };
    while index < end {
        let (kind, token_start, token_end) = scan_token_info(bytes, index, end);
        if token_start >= end {
            break;
        }
        match kind {
            1 => counts.identifiers += 1,
            2 => counts.numbers += 1,
            _ => counts.symbols += 1,
        }
        counts.total += 1;
        index = token_end;
    }
    counts
}

/// Rust reference for the Jadren diagnostic aggregate.
#[must_use]
pub const fn diagnostic(code: u16, severity: u8) -> (u16, u8) {
    (code, severity)
}

fn parse_typed_call_candidates(
    value: &str,
    source_length: u64,
    line_number: usize,
) -> Result<Vec<TypedCallCandidateHeader>, String> {
    let mut candidates = Vec::new();
    for raw_candidate in value.split('|') {
        let fields: Vec<&str> = raw_candidate.split(':').map(str::trim).collect();
        if fields.len() != 5
            && fields.len() != 6
            && fields.len() != 8
            && fields.len() != 9
            && fields.len() != 10
        {
            return Err(format!(
                "line {}: typed-call-resolve candidate must have five, six, eight, nine, or ten fields",
                line_number + 1
            ));
        }
        let name_start = parse_u64(fields[0], line_number)?;
        let name_end = parse_u64(fields[1], line_number)?;
        let parameter_count = parse_u64(fields[2], line_number)?;
        let generic_parameter_count = parse_u64(fields[3], line_number)?;
        let return_type_kind = parse_u8(fields[4], line_number)?;
        let parameter_type_kind = if fields.len() == 6 {
            parse_u8(fields[5], line_number)?
        } else {
            0
        };
        let parameter_type_start = if fields.len() >= 8 {
            parse_u16(fields[6], line_number)?
        } else {
            0
        };
        let parameter_type_end = if fields.len() >= 8 {
            parse_u16(fields[7], line_number)?
        } else {
            0
        };
        let generic_bound_kind = if fields.len() >= 9 {
            parse_u8(fields[8], line_number)?
        } else {
            0
        };
        let generic_substitution_kind = if fields.len() == 10 {
            parse_u8(fields[9], line_number)?
        } else {
            0
        };
        if name_start >= name_end
            || name_end > source_length
            || return_type_kind > 5
            || parameter_type_kind > 5
            || generic_bound_kind > 3
            || generic_substitution_kind > 3
            || (return_type_kind == 0 && generic_substitution_kind == 0)
            || (generic_substitution_kind == 1
                && (generic_parameter_count == 0
                    || parameter_count != 1
                    || parameter_type_kind != 0
                    || parameter_type_start != 0
                    || parameter_type_end != 0))
            || (generic_substitution_kind == 2
                && (generic_parameter_count == 0
                    || parameter_count < 2
                    || parameter_type_kind != 0
                    || parameter_type_start != 0
                    || parameter_type_end != 0))
            || (generic_substitution_kind == 3
                && (generic_parameter_count == 0
                    || parameter_count < 2
                    || parameter_type_kind != 0
                    || parameter_type_start != 0
                    || parameter_type_end != 0))
            || (generic_parameter_count == 0 && generic_bound_kind != 0)
            || (parameter_count != 1 && parameter_type_kind != 0)
            || (parameter_type_start == 0 && parameter_type_end != 0)
            || (parameter_type_start != 0
                && (parameter_type_end <= parameter_type_start
                    || u64::from(parameter_type_end) > source_length))
            || (parameter_type_start != 0 && parameter_type_kind != 0)
        {
            return Err(format!(
                "line {}: typed-call-resolve candidate has invalid span, generic count, or return type",
                line_number + 1
            ));
        }
        candidates.push(TypedCallCandidateHeader {
            name_start,
            name_end,
            parameter_count,
            generic_parameter_count,
            return_type_kind,
            parameter_type_kind,
            parameter_type_start,
            parameter_type_end,
            generic_bound_kind,
            generic_substitution_kind,
        });
    }
    if candidates.is_empty() || candidates.len() > 32 {
        return Err(format!(
            "line {}: typed-call-resolve requires one to 32 candidates",
            line_number + 1
        ));
    }
    Ok(candidates)
}

fn parse_u8(value: &str, line_number: usize) -> Result<u8, String> {
    parse_integer(value, line_number)?
        .try_into()
        .map_err(|_| format!("line {}: `{value}` does not fit into u8", line_number + 1))
}

fn parse_u16(value: &str, line_number: usize) -> Result<u16, String> {
    parse_integer(value, line_number)?
        .try_into()
        .map_err(|_| format!("line {}: `{value}` does not fit into u16", line_number + 1))
}

fn parse_u64(value: &str, line_number: usize) -> Result<u64, String> {
    parse_integer(value, line_number)
}

fn parse_usize(value: &str, line_number: usize) -> Result<usize, String> {
    parse_integer(value, line_number)?.try_into().map_err(|_| {
        format!(
            "line {}: `{value}` does not fit into usize",
            line_number + 1
        )
    })
}

fn parse_integer(value: &str, line_number: usize) -> Result<u64, String> {
    let (radix, digits) = value
        .strip_prefix("0x")
        .map_or((10, value), |digits| (16, digits));
    u64::from_str_radix(digits, radix).map_err(|error| {
        format!(
            "line {}: invalid integer `{value}`: {error}",
            line_number + 1
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{
        CorpusCase, classify_byte, count_tokens, diagnostic, infer_binary_literal_types,
        infer_call_types, infer_call_types_into, infer_call_types_resolved,
        infer_call_types_resolved_into, infer_cast_literal_types, infer_name_types,
        infer_name_types_into, infer_region_name_types, infer_region_name_types_into,
        infer_scoped_name_types, infer_scoped_name_types_into, infer_unary_literal_types,
        infer_unary_literal_types_into, parse_corpus, parse_expression_precedence_chain_headers,
        parse_expression_precedence_chain_headers_into, parse_expression_precedence_headers,
        parse_expression_precedence_headers_into, parse_expression_precedence_long_chain_headers,
        parse_expression_precedence_long_chain_headers_into, run_oracle, scan_identifier,
        scan_identifier_via_api, scan_token, scan_token_info, token_span,
    };

    #[test]
    fn parses_and_runs_versioned_corpus() {
        let corpus = parse_corpus(
            "jadren-selfhost-corpus-0.1\nbyte,0x41,1\nspan,3,9,3,9\nscan,play1!,0,6,0,5\ntoken,x  foo,1,6,3,6\ninfo,123 +,0,5,2,0,3\ncount,foo+12,0,6,1,1,1,3\ndiagnostic,1001,2,1001,2\n",
        )
        .expect("corpus");
        assert_eq!(corpus.cases.len(), 7);
        let report = run_oracle(&corpus).expect("oracle");
        assert_eq!(report.byte_cases, 1);
        assert_eq!(report.scan_cases, 1);
        assert_eq!(report.api_span_cases, 1);
        assert_eq!(report.api_scan_cases, 1);
        assert_eq!(report.api_diagnostic_cases, 1);
        assert_eq!(report.token_cases, 1);
        assert_eq!(report.token_info_cases, 1);
        assert_eq!(report.api_token_info_cases, 1);
        assert_eq!(report.count_cases, 1);
    }

    #[test]
    fn rejects_missing_header_and_bad_rows() {
        assert!(parse_corpus("byte,1,1\n").is_err());
        assert!(parse_corpus("jadren-selfhost-corpus-0.1\nspan,1,2\n").is_err());
        assert!(parse_corpus("jadren-selfhost-corpus-0.1\nscan,ž,0,1,0,0\n").is_err());
    }

    #[test]
    fn reference_functions_are_deterministic() {
        assert_eq!(classify_byte(b'_'), 1);
        assert_eq!(classify_byte(b'7'), 2);
        assert_eq!(classify_byte(b' '), 3);
        assert_eq!(classify_byte(0xff), 0);
        assert_eq!(token_span(8, 2), (8, 2));
        assert_eq!(scan_identifier(b"play1!", 0, 6), (0, 5));
        assert_eq!(scan_token(b"x  foo", 1, 6), (3, 6));
        assert_eq!(scan_token_info(b"123 +", 0, 5), (2, 0, 3));
        let binary = infer_binary_literal_types(b"1 + 2.0", 0, 7);
        assert_eq!(binary.len(), 1);
        assert_eq!(binary[0].expression_kind, 2);
        assert_eq!(binary[0].type_kind, 3);
        let calls = infer_call_types(
            b"foo(1) + foo()",
            0,
            14,
            &[
                super::TypedCallBindingHeader {
                    name_start: 0,
                    name_end: 3,
                    parameter_count: 1,
                    return_type_kind: 2,
                },
                super::TypedCallBindingHeader {
                    name_start: 9,
                    name_end: 12,
                    parameter_count: 0,
                    return_type_kind: 3,
                },
            ],
        );
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].expression_kind, 8);
        assert_eq!(calls[0].type_kind, 2);
        assert_eq!((calls[0].start, calls[0].end, calls[0].depth), (0, 6, 0));
        assert_eq!(calls[1].type_kind, 3);
        assert_eq!((calls[1].start, calls[1].end, calls[1].depth), (9, 14, 0));
        assert!(
            infer_call_types(
                b"foo(1)",
                0,
                6,
                &[super::TypedCallBindingHeader {
                    name_start: 0,
                    name_end: 3,
                    parameter_count: 0,
                    return_type_kind: 2,
                }]
            )
            .is_empty()
        );
        let mut one_call_output = [super::TypedExpressionHeader {
            expression_kind: 0,
            type_kind: 0,
            start: 0,
            end: 0,
            depth: 0,
        }];
        assert_eq!(
            infer_call_types_into(
                b"foo(1) + foo()",
                0,
                14,
                &[super::TypedCallBindingHeader {
                    name_start: 0,
                    name_end: 3,
                    parameter_count: 1,
                    return_type_kind: 2,
                }],
                &mut one_call_output,
            ),
            1
        );
        assert_eq!(one_call_output[0], calls[0]);
        let resolved = infer_call_types_resolved(
            b"foo(1) + foo()",
            0,
            14,
            &[
                super::TypedCallCandidateHeader {
                    name_start: 0,
                    name_end: 3,
                    parameter_count: 1,
                    generic_parameter_count: 1,
                    return_type_kind: 3,
                    parameter_type_kind: 0,
                    parameter_type_start: 0,
                    parameter_type_end: 0,
                    generic_bound_kind: 0,
                    generic_substitution_kind: 0,
                },
                super::TypedCallCandidateHeader {
                    name_start: 0,
                    name_end: 3,
                    parameter_count: 1,
                    generic_parameter_count: 0,
                    return_type_kind: 2,
                    parameter_type_kind: 0,
                    parameter_type_start: 0,
                    parameter_type_end: 0,
                    generic_bound_kind: 0,
                    generic_substitution_kind: 0,
                },
                super::TypedCallCandidateHeader {
                    name_start: 9,
                    name_end: 12,
                    parameter_count: 0,
                    generic_parameter_count: 2,
                    return_type_kind: 4,
                    parameter_type_kind: 0,
                    parameter_type_start: 0,
                    parameter_type_end: 0,
                    generic_bound_kind: 0,
                    generic_substitution_kind: 0,
                },
            ],
        );
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].type_kind, 2);
        assert_eq!(resolved[1].type_kind, 4);
        let typed_resolved = infer_call_types_resolved(
            b"foo(1)",
            0,
            6,
            &[
                super::TypedCallCandidateHeader {
                    name_start: 0,
                    name_end: 3,
                    parameter_count: 1,
                    generic_parameter_count: 0,
                    return_type_kind: 3,
                    parameter_type_kind: 3,
                    parameter_type_start: 0,
                    parameter_type_end: 0,
                    generic_bound_kind: 0,
                    generic_substitution_kind: 0,
                },
                super::TypedCallCandidateHeader {
                    name_start: 0,
                    name_end: 3,
                    parameter_count: 1,
                    generic_parameter_count: 0,
                    return_type_kind: 4,
                    parameter_type_kind: 0,
                    parameter_type_start: 0,
                    parameter_type_end: 0,
                    generic_bound_kind: 0,
                    generic_substitution_kind: 0,
                },
                super::TypedCallCandidateHeader {
                    name_start: 0,
                    name_end: 3,
                    parameter_count: 1,
                    generic_parameter_count: 1,
                    return_type_kind: 2,
                    parameter_type_kind: 2,
                    parameter_type_start: 0,
                    parameter_type_end: 0,
                    generic_bound_kind: 0,
                    generic_substitution_kind: 0,
                },
            ],
        );
        assert_eq!(typed_resolved.len(), 1);
        assert_eq!(typed_resolved[0].type_kind, 2);
        let typed_mismatch = infer_call_types_resolved(
            b"foo(true)",
            0,
            9,
            &[super::TypedCallCandidateHeader {
                name_start: 0,
                name_end: 3,
                parameter_count: 1,
                generic_parameter_count: 0,
                return_type_kind: 3,
                parameter_type_kind: 2,
                parameter_type_start: 0,
                parameter_type_end: 0,
                generic_bound_kind: 0,
                generic_substitution_kind: 0,
            }],
        );
        assert!(typed_mismatch.is_empty());
        let parameter_list = infer_call_types_resolved(
            b"foo(1, true) [Integer, Bool]",
            0,
            12,
            &[
                super::TypedCallCandidateHeader {
                    name_start: 0,
                    name_end: 3,
                    parameter_count: 2,
                    generic_parameter_count: 0,
                    return_type_kind: 4,
                    parameter_type_kind: 0,
                    parameter_type_start: 13,
                    parameter_type_end: 28,
                    generic_bound_kind: 0,
                    generic_substitution_kind: 0,
                },
                super::TypedCallCandidateHeader {
                    name_start: 0,
                    name_end: 3,
                    parameter_count: 2,
                    generic_parameter_count: 1,
                    return_type_kind: 5,
                    parameter_type_kind: 0,
                    parameter_type_start: 0,
                    parameter_type_end: 0,
                    generic_bound_kind: 0,
                    generic_substitution_kind: 0,
                },
            ],
        );
        assert_eq!(parameter_list.len(), 1);
        assert_eq!(parameter_list[0].type_kind, 4);
        let parameter_mismatch = infer_call_types_resolved(
            b"foo(1, true) [Integer, Integer]",
            0,
            12,
            &[super::TypedCallCandidateHeader {
                name_start: 0,
                name_end: 3,
                parameter_count: 2,
                generic_parameter_count: 0,
                return_type_kind: 4,
                parameter_type_kind: 0,
                parameter_type_start: 13,
                parameter_type_end: 31,
                generic_bound_kind: 0,
                generic_substitution_kind: 0,
            }],
        );
        assert!(parameter_mismatch.is_empty());
        let numeric_bound = infer_call_types_resolved(
            b"foo(1, 2)",
            0,
            9,
            &[
                super::TypedCallCandidateHeader {
                    name_start: 0,
                    name_end: 3,
                    parameter_count: 2,
                    generic_parameter_count: 1,
                    return_type_kind: 4,
                    parameter_type_kind: 0,
                    parameter_type_start: 0,
                    parameter_type_end: 0,
                    generic_bound_kind: 1,
                    generic_substitution_kind: 0,
                },
                super::TypedCallCandidateHeader {
                    name_start: 0,
                    name_end: 3,
                    parameter_count: 2,
                    generic_parameter_count: 1,
                    return_type_kind: 5,
                    parameter_type_kind: 0,
                    parameter_type_start: 0,
                    parameter_type_end: 0,
                    generic_bound_kind: 0,
                    generic_substitution_kind: 0,
                },
            ],
        );
        assert_eq!(numeric_bound.len(), 1);
        assert_eq!(numeric_bound[0].type_kind, 4);
        let bound_fallback = infer_call_types_resolved(
            b"foo(true)",
            0,
            9,
            &[
                super::TypedCallCandidateHeader {
                    name_start: 0,
                    name_end: 3,
                    parameter_count: 1,
                    generic_parameter_count: 1,
                    return_type_kind: 4,
                    parameter_type_kind: 0,
                    parameter_type_start: 0,
                    parameter_type_end: 0,
                    generic_bound_kind: 1,
                    generic_substitution_kind: 0,
                },
                super::TypedCallCandidateHeader {
                    name_start: 0,
                    name_end: 3,
                    parameter_count: 1,
                    generic_parameter_count: 1,
                    return_type_kind: 5,
                    parameter_type_kind: 0,
                    parameter_type_start: 0,
                    parameter_type_end: 0,
                    generic_bound_kind: 0,
                    generic_substitution_kind: 0,
                },
            ],
        );
        assert_eq!(bound_fallback.len(), 1);
        assert_eq!(bound_fallback[0].type_kind, 5);
        let substituted_integer = infer_call_types_resolved(
            b"foo(42)",
            0,
            7,
            &[
                super::TypedCallCandidateHeader {
                    name_start: 0,
                    name_end: 3,
                    parameter_count: 1,
                    generic_parameter_count: 1,
                    return_type_kind: 0,
                    parameter_type_kind: 0,
                    parameter_type_start: 0,
                    parameter_type_end: 0,
                    generic_bound_kind: 0,
                    generic_substitution_kind: 1,
                },
                super::TypedCallCandidateHeader {
                    name_start: 0,
                    name_end: 3,
                    parameter_count: 1,
                    generic_parameter_count: 1,
                    return_type_kind: 5,
                    parameter_type_kind: 0,
                    parameter_type_start: 0,
                    parameter_type_end: 0,
                    generic_bound_kind: 0,
                    generic_substitution_kind: 0,
                },
            ],
        );
        assert_eq!(substituted_integer.len(), 1);
        assert_eq!(substituted_integer[0].type_kind, 2);
        let substituted_boolean = infer_call_types_resolved(
            b"foo(true)",
            0,
            9,
            &[super::TypedCallCandidateHeader {
                name_start: 0,
                name_end: 3,
                parameter_count: 1,
                generic_parameter_count: 1,
                return_type_kind: 0,
                parameter_type_kind: 0,
                parameter_type_start: 0,
                parameter_type_end: 0,
                generic_bound_kind: 2,
                generic_substitution_kind: 1,
            }],
        );
        assert_eq!(substituted_boolean.len(), 1);
        assert_eq!(substituted_boolean[0].type_kind, 1);
        let substituted_same_integer = infer_call_types_resolved(
            b"same(1, 2)",
            0,
            10,
            &[
                super::TypedCallCandidateHeader {
                    name_start: 0,
                    name_end: 4,
                    parameter_count: 2,
                    generic_parameter_count: 1,
                    return_type_kind: 0,
                    parameter_type_kind: 0,
                    parameter_type_start: 0,
                    parameter_type_end: 0,
                    generic_bound_kind: 0,
                    generic_substitution_kind: 2,
                },
                super::TypedCallCandidateHeader {
                    name_start: 0,
                    name_end: 4,
                    parameter_count: 2,
                    generic_parameter_count: 1,
                    return_type_kind: 5,
                    parameter_type_kind: 0,
                    parameter_type_start: 0,
                    parameter_type_end: 0,
                    generic_bound_kind: 0,
                    generic_substitution_kind: 0,
                },
            ],
        );
        assert_eq!(substituted_same_integer.len(), 1);
        assert_eq!(substituted_same_integer[0].type_kind, 2);
        let substituted_same_boolean = infer_call_types_resolved(
            b"same(true, false)",
            0,
            17,
            &[super::TypedCallCandidateHeader {
                name_start: 0,
                name_end: 4,
                parameter_count: 2,
                generic_parameter_count: 1,
                return_type_kind: 0,
                parameter_type_kind: 0,
                parameter_type_start: 0,
                parameter_type_end: 0,
                generic_bound_kind: 2,
                generic_substitution_kind: 2,
            }],
        );
        assert_eq!(substituted_same_boolean.len(), 1);
        assert_eq!(substituted_same_boolean[0].type_kind, 1);
        let substituted_same_mismatch = infer_call_types_resolved(
            b"same(1, true)",
            0,
            13,
            &[
                super::TypedCallCandidateHeader {
                    name_start: 0,
                    name_end: 4,
                    parameter_count: 2,
                    generic_parameter_count: 1,
                    return_type_kind: 0,
                    parameter_type_kind: 0,
                    parameter_type_start: 0,
                    parameter_type_end: 0,
                    generic_bound_kind: 1,
                    generic_substitution_kind: 2,
                },
                super::TypedCallCandidateHeader {
                    name_start: 0,
                    name_end: 4,
                    parameter_count: 2,
                    generic_parameter_count: 1,
                    return_type_kind: 5,
                    parameter_type_kind: 0,
                    parameter_type_start: 0,
                    parameter_type_end: 0,
                    generic_bound_kind: 0,
                    generic_substitution_kind: 0,
                },
            ],
        );
        assert_eq!(substituted_same_mismatch.len(), 1);
        assert_eq!(substituted_same_mismatch[0].type_kind, 5);
        let substituted_first_integer = infer_call_types_resolved(
            b"first(1, true)",
            0,
            14,
            &[
                super::TypedCallCandidateHeader {
                    name_start: 0,
                    name_end: 5,
                    parameter_count: 2,
                    generic_parameter_count: 2,
                    return_type_kind: 0,
                    parameter_type_kind: 0,
                    parameter_type_start: 0,
                    parameter_type_end: 0,
                    generic_bound_kind: 0,
                    generic_substitution_kind: 3,
                },
                super::TypedCallCandidateHeader {
                    name_start: 0,
                    name_end: 5,
                    parameter_count: 2,
                    generic_parameter_count: 0,
                    return_type_kind: 5,
                    parameter_type_kind: 0,
                    parameter_type_start: 0,
                    parameter_type_end: 0,
                    generic_bound_kind: 0,
                    generic_substitution_kind: 0,
                },
            ],
        );
        assert_eq!(substituted_first_integer.len(), 1);
        assert_eq!(substituted_first_integer[0].type_kind, 2);
        let substituted_first_boolean = infer_call_types_resolved(
            b"first(true, 42)",
            0,
            15,
            &[super::TypedCallCandidateHeader {
                name_start: 0,
                name_end: 5,
                parameter_count: 2,
                generic_parameter_count: 2,
                return_type_kind: 0,
                parameter_type_kind: 0,
                parameter_type_start: 0,
                parameter_type_end: 0,
                generic_bound_kind: 0,
                generic_substitution_kind: 3,
            }],
        );
        assert_eq!(substituted_first_boolean.len(), 1);
        assert_eq!(substituted_first_boolean[0].type_kind, 1);
        let ambiguous = infer_call_types_resolved(
            b"foo()",
            0,
            5,
            &[
                super::TypedCallCandidateHeader {
                    name_start: 0,
                    name_end: 3,
                    parameter_count: 0,
                    generic_parameter_count: 0,
                    return_type_kind: 2,
                    parameter_type_kind: 0,
                    parameter_type_start: 0,
                    parameter_type_end: 0,
                    generic_bound_kind: 0,
                    generic_substitution_kind: 0,
                },
                super::TypedCallCandidateHeader {
                    name_start: 0,
                    name_end: 3,
                    parameter_count: 0,
                    generic_parameter_count: 0,
                    return_type_kind: 3,
                    parameter_type_kind: 0,
                    parameter_type_start: 0,
                    parameter_type_end: 0,
                    generic_bound_kind: 0,
                    generic_substitution_kind: 0,
                },
            ],
        );
        assert!(ambiguous.is_empty());
        let mut resolved_output = [super::TypedExpressionHeader {
            expression_kind: 0,
            type_kind: 0,
            start: 0,
            end: 0,
            depth: 0,
        }];
        assert_eq!(
            infer_call_types_resolved_into(
                b"foo(1)",
                0,
                6,
                &[super::TypedCallCandidateHeader {
                    name_start: 0,
                    name_end: 3,
                    parameter_count: 1,
                    generic_parameter_count: 1,
                    return_type_kind: 5,
                    parameter_type_kind: 0,
                    parameter_type_start: 0,
                    parameter_type_end: 0,
                    generic_bound_kind: 0,
                    generic_substitution_kind: 0,
                }],
                &mut resolved_output,
            ),
            1
        );
        assert_eq!(resolved_output[0].type_kind, 5);
        let names = infer_name_types(
            b"(foo) + bar",
            0,
            11,
            &[
                super::TypedNameBindingHeader {
                    name_start: 1,
                    name_end: 4,
                    type_kind: 2,
                },
                super::TypedNameBindingHeader {
                    name_start: 8,
                    name_end: 11,
                    type_kind: 3,
                },
            ],
        );
        assert_eq!(names.len(), 2);
        assert_eq!(
            (names[0].expression_kind, names[0].type_kind, names[0].depth),
            (5, 2, 1)
        );
        assert_eq!(
            (names[1].expression_kind, names[1].type_kind, names[1].depth),
            (5, 3, 0)
        );
        let mut bounded_name_output = [super::TypedExpressionHeader {
            expression_kind: 0,
            type_kind: 0,
            start: 0,
            end: 0,
            depth: 0,
        }];
        let bounded_name_count = infer_name_types_into(
            b"foo bar foo",
            0,
            11,
            &[
                super::TypedNameBindingHeader {
                    name_start: 0,
                    name_end: 3,
                    type_kind: 2,
                },
                super::TypedNameBindingHeader {
                    name_start: 4,
                    name_end: 7,
                    type_kind: 3,
                },
            ],
            &mut bounded_name_output,
        );
        assert_eq!(bounded_name_count, 1);
        assert_eq!(
            (
                bounded_name_output[0].expression_kind,
                bounded_name_output[0].type_kind,
                bounded_name_output[0].start,
                bounded_name_output[0].end,
            ),
            (5, 2, 0, 3)
        );
        assert!(
            infer_name_types(
                b"foo",
                4,
                4,
                &[super::TypedNameBindingHeader {
                    name_start: 0,
                    name_end: 3,
                    type_kind: 2,
                }],
            )
            .is_empty()
        );
        assert!(
            infer_name_types(
                b"foo",
                2,
                1,
                &[super::TypedNameBindingHeader {
                    name_start: 0,
                    name_end: 3,
                    type_kind: 2,
                }],
            )
            .is_empty()
        );
        let scoped_names = infer_scoped_name_types(
            b"foo (foo)",
            0,
            9,
            &[
                super::TypedScopedNameBindingHeader {
                    name_start: 0,
                    name_end: 3,
                    type_kind: 2,
                    scope_depth: 0,
                },
                super::TypedScopedNameBindingHeader {
                    name_start: 0,
                    name_end: 3,
                    type_kind: 3,
                    scope_depth: 1,
                },
                super::TypedScopedNameBindingHeader {
                    name_start: 0,
                    name_end: 3,
                    type_kind: 4,
                    scope_depth: 2,
                },
                super::TypedScopedNameBindingHeader {
                    name_start: 0,
                    name_end: 3,
                    type_kind: 5,
                    scope_depth: 1,
                },
            ],
        );
        assert_eq!(scoped_names.len(), 2);
        assert_eq!(
            (
                scoped_names[0].expression_kind,
                scoped_names[0].type_kind,
                scoped_names[0].depth
            ),
            (6, 2, 0)
        );
        assert_eq!(
            (
                scoped_names[1].expression_kind,
                scoped_names[1].type_kind,
                scoped_names[1].depth
            ),
            (6, 5, 1)
        );
        let mut bounded_scoped_output = [super::TypedExpressionHeader {
            expression_kind: 0,
            type_kind: 0,
            start: 0,
            end: 0,
            depth: 0,
        }];
        let bounded_scoped_count = infer_scoped_name_types_into(
            b"foo (foo)",
            0,
            9,
            &[
                super::TypedScopedNameBindingHeader {
                    name_start: 0,
                    name_end: 3,
                    type_kind: 2,
                    scope_depth: 0,
                },
                super::TypedScopedNameBindingHeader {
                    name_start: 0,
                    name_end: 3,
                    type_kind: 5,
                    scope_depth: 1,
                },
            ],
            &mut bounded_scoped_output,
        );
        assert_eq!(bounded_scoped_count, 1);
        assert_eq!(
            (
                bounded_scoped_output[0].expression_kind,
                bounded_scoped_output[0].type_kind,
                bounded_scoped_output[0].start,
                bounded_scoped_output[0].end,
                bounded_scoped_output[0].depth,
            ),
            (6, 2, 0, 3, 0)
        );
        assert!(
            infer_scoped_name_types(
                b"foo",
                0,
                3,
                &[super::TypedScopedNameBindingHeader {
                    name_start: 0,
                    name_end: 4,
                    type_kind: 2,
                    scope_depth: 0,
                }],
            )
            .is_empty()
        );
        let region_names = infer_region_name_types(
            b"foo (foo)",
            0,
            9,
            &[
                super::TypedRegionNameBindingHeader {
                    name_start: 0,
                    name_end: 3,
                    region_start: 0,
                    region_end: 9,
                    type_kind: 2,
                    scope_depth: 0,
                },
                super::TypedRegionNameBindingHeader {
                    name_start: 5,
                    name_end: 8,
                    region_start: 5,
                    region_end: 8,
                    type_kind: 3,
                    scope_depth: 1,
                },
                super::TypedRegionNameBindingHeader {
                    name_start: 5,
                    name_end: 8,
                    region_start: 5,
                    region_end: 8,
                    type_kind: 4,
                    scope_depth: 2,
                },
                super::TypedRegionNameBindingHeader {
                    name_start: 5,
                    name_end: 8,
                    region_start: 5,
                    region_end: 8,
                    type_kind: 5,
                    scope_depth: 1,
                },
            ],
        );
        assert_eq!(region_names.len(), 2);
        assert_eq!(
            (
                region_names[0].expression_kind,
                region_names[0].type_kind,
                region_names[0].depth
            ),
            (7, 2, 0)
        );
        assert_eq!(
            (
                region_names[1].expression_kind,
                region_names[1].type_kind,
                region_names[1].depth
            ),
            (7, 5, 1)
        );
        let overlapping_regions = infer_region_name_types(
            b"foo + foo",
            0,
            9,
            &[
                super::TypedRegionNameBindingHeader {
                    name_start: 0,
                    name_end: 3,
                    region_start: 0,
                    region_end: 9,
                    type_kind: 2,
                    scope_depth: 0,
                },
                super::TypedRegionNameBindingHeader {
                    name_start: 0,
                    name_end: 3,
                    region_start: 0,
                    region_end: 3,
                    type_kind: 3,
                    scope_depth: 0,
                },
            ],
        );
        assert_eq!(overlapping_regions.len(), 2);
        assert_eq!(overlapping_regions[0].type_kind, 3);
        assert_eq!(overlapping_regions[1].type_kind, 2);
        assert!(
            infer_region_name_types(
                b"foo",
                0,
                3,
                &[super::TypedRegionNameBindingHeader {
                    name_start: 0,
                    name_end: 3,
                    region_start: 0,
                    region_end: 4,
                    type_kind: 2,
                    scope_depth: 0,
                }],
            )
            .is_empty()
        );
        assert!(
            infer_region_name_types(
                b"(foo)",
                0,
                5,
                &[super::TypedRegionNameBindingHeader {
                    name_start: 1,
                    name_end: 4,
                    region_start: 0,
                    region_end: 4,
                    type_kind: 2,
                    scope_depth: 0,
                }],
            )
            .is_empty()
        );
        assert!(
            infer_region_name_types(
                b"([foo)bar]",
                0,
                10,
                &[super::TypedRegionNameBindingHeader {
                    name_start: 2,
                    name_end: 5,
                    region_start: 1,
                    region_end: 6,
                    type_kind: 2,
                    scope_depth: 1,
                }],
            )
            .is_empty()
        );
        assert!(
            infer_region_name_types(
                b"foo)",
                0,
                4,
                &[super::TypedRegionNameBindingHeader {
                    name_start: 0,
                    name_end: 3,
                    region_start: 0,
                    region_end: 4,
                    type_kind: 2,
                    scope_depth: 0,
                }],
            )
            .is_empty()
        );
        let mut deep_source = "(".repeat(33);
        deep_source.push_str("foo");
        deep_source.push_str(&")".repeat(33));
        assert!(
            infer_region_name_types(
                deep_source.as_bytes(),
                0,
                deep_source.len() as u64,
                &[super::TypedRegionNameBindingHeader {
                    name_start: 33,
                    name_end: 36,
                    region_start: 0,
                    region_end: deep_source.len() as u64,
                    type_kind: 2,
                    scope_depth: 0,
                }],
            )
            .is_empty()
        );
        let mut bounded_region_output = [super::TypedExpressionHeader {
            expression_kind: 0,
            type_kind: 0,
            start: 0,
            end: 0,
            depth: 0,
        }];
        let bounded_region_count = infer_region_name_types_into(
            b"foo foo foo",
            0,
            11,
            &[super::TypedRegionNameBindingHeader {
                name_start: 0,
                name_end: 3,
                region_start: 0,
                region_end: 11,
                type_kind: 2,
                scope_depth: 0,
            }],
            &mut bounded_region_output,
        );
        assert_eq!(bounded_region_count, 1);
        assert_eq!(
            (
                bounded_region_output[0].expression_kind,
                bounded_region_output[0].type_kind,
                bounded_region_output[0].start,
                bounded_region_output[0].end,
            ),
            (7, 2, 0, 3)
        );
        assert_eq!((binary[0].start, binary[0].end), (0, 7));
        let chain = infer_binary_literal_types(b"1 + 2 + 3", 0, 9);
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].type_kind, 2);
        let mixed_arithmetic = infer_binary_literal_types(b"1 + 2 * 3", 0, 9);
        assert_eq!(mixed_arithmetic.len(), 1);
        assert_eq!(mixed_arithmetic[0].type_kind, 2);
        assert_eq!((mixed_arithmetic[0].start, mixed_arithmetic[0].end), (0, 9));
        let mixed_compare = infer_binary_literal_types(b"1 + 2 == 3", 0, 10);
        assert_eq!(mixed_compare.len(), 1);
        assert_eq!(mixed_compare[0].type_kind, 1);
        assert_eq!((mixed_compare[0].start, mixed_compare[0].end), (0, 10));
        assert!(infer_binary_literal_types(b"true & false", 0, 12).is_empty());
        let precedence = parse_expression_precedence_headers(b"a + b * c", 0, 9);
        assert_eq!(precedence.len(), 2);
        assert_eq!(
            precedence[0],
            jadren_selfhost_api::ExpressionPrecedenceHeader {
                kind: 2,
                precedence: 8,
                associativity: 1,
                left_start: 4,
                left_end: 5,
                operator_start: 6,
                operator_end: 7,
                right_start: 8,
                right_end: 9,
                depth: 0,
            }
        );
        let assignment = parse_expression_precedence_headers(b"a = b = c", 0, 9);
        assert_eq!(assignment.len(), 2);
        assert_eq!(assignment[0].precedence, 1);
        assert_eq!(assignment[0].associativity, 2);
        assert_eq!((assignment[0].left_start, assignment[0].right_end), (4, 9));
        assert_eq!((assignment[1].left_start, assignment[1].right_end), (0, 9));
        assert!(parse_expression_precedence_headers(b"a + b", 0, 5).is_empty());
        assert!(parse_expression_precedence_headers(b"a + b + c + d", 0, 13).is_empty());
        assert!(parse_expression_precedence_headers(b"a + (b * c)", 0, 11).is_empty());
        assert!(parse_expression_precedence_headers(b"a + (b * c]", 0, 11).is_empty());
        assert!(parse_expression_precedence_headers(b"a + b * c)", 0, 10).is_empty());
        let unary_precedence = parse_expression_precedence_headers(b"a + -b * c", 0, 10);
        assert_eq!(unary_precedence.len(), 2);
        assert_eq!(
            (
                unary_precedence[0].operator_start,
                unary_precedence[0].left_start,
                unary_precedence[0].right_end
            ),
            (7, 4, 10)
        );
        assert_eq!(
            (
                unary_precedence[1].operator_start,
                unary_precedence[1].left_start,
                unary_precedence[1].right_end
            ),
            (2, 0, 10)
        );
        let unary_assignment = parse_expression_precedence_headers(b"a = -b = c", 0, 10);
        assert_eq!(unary_assignment.len(), 2);
        assert_eq!(
            (
                unary_assignment[0].operator_start,
                unary_assignment[0].left_start,
                unary_assignment[0].right_end
            ),
            (7, 4, 10)
        );
        assert_eq!(
            (
                unary_assignment[1].operator_start,
                unary_assignment[1].left_start,
                unary_assignment[1].right_end
            ),
            (2, 0, 10)
        );
        let precedence_chain = parse_expression_precedence_chain_headers(b"a + b * c + d", 0, 13);
        assert_eq!(precedence_chain.len(), 3);
        assert_eq!(
            (
                precedence_chain[0].operator_start,
                precedence_chain[0].operator_end
            ),
            (6, 7)
        );
        assert_eq!(
            (
                precedence_chain[0].left_start,
                precedence_chain[0].right_end
            ),
            (4, 9)
        );
        assert_eq!(
            (
                precedence_chain[1].operator_start,
                precedence_chain[1].operator_end
            ),
            (2, 3)
        );
        assert_eq!(
            (
                precedence_chain[1].left_start,
                precedence_chain[1].right_end
            ),
            (0, 9)
        );
        assert_eq!(
            (
                precedence_chain[2].operator_start,
                precedence_chain[2].operator_end
            ),
            (10, 11)
        );
        assert_eq!(
            (
                precedence_chain[2].left_start,
                precedence_chain[2].right_end
            ),
            (0, 13)
        );
        let assignment_chain = parse_expression_precedence_chain_headers(b"a = b = c = d", 0, 13);
        assert_eq!(assignment_chain.len(), 3);
        assert_eq!(
            (
                assignment_chain[0].operator_start,
                assignment_chain[0].operator_end
            ),
            (10, 11)
        );
        assert_eq!(
            (
                assignment_chain[0].left_start,
                assignment_chain[0].right_end
            ),
            (8, 13)
        );
        assert_eq!(
            (
                assignment_chain[1].operator_start,
                assignment_chain[1].operator_end
            ),
            (6, 7)
        );
        assert_eq!(
            (
                assignment_chain[1].left_start,
                assignment_chain[1].right_end
            ),
            (4, 13)
        );
        assert_eq!(
            (
                assignment_chain[2].operator_start,
                assignment_chain[2].operator_end
            ),
            (2, 3)
        );
        assert_eq!(
            (
                assignment_chain[2].left_start,
                assignment_chain[2].right_end
            ),
            (0, 13)
        );
        assert!(parse_expression_precedence_chain_headers(b"a + b + c", 0, 9).is_empty());
        assert!(parse_expression_precedence_chain_headers(b"a + (b * c) + d", 0, 15).is_empty());
        assert!(parse_expression_precedence_chain_headers(b"a + (b * c] + d", 0, 15).is_empty());
        assert!(parse_expression_precedence_chain_headers(b"a + b * c) + d", 0, 14).is_empty());
        let unary_precedence_chain =
            parse_expression_precedence_chain_headers(b"a + -b * c + d", 0, 14);
        assert_eq!(unary_precedence_chain.len(), 3);
        assert_eq!(
            (
                unary_precedence_chain[0].operator_start,
                unary_precedence_chain[0].left_start,
                unary_precedence_chain[0].right_end
            ),
            (7, 4, 10)
        );
        assert_eq!(
            (
                unary_precedence_chain[1].operator_start,
                unary_precedence_chain[1].left_start,
                unary_precedence_chain[1].right_end
            ),
            (2, 0, 10)
        );
        assert_eq!(
            (
                unary_precedence_chain[2].operator_start,
                unary_precedence_chain[2].left_start,
                unary_precedence_chain[2].right_end
            ),
            (11, 0, 14)
        );
        let unary_assignment_chain =
            parse_expression_precedence_chain_headers(b"a = -b = c = d", 0, 14);
        assert_eq!(unary_assignment_chain.len(), 3);
        assert_eq!(
            (
                unary_assignment_chain[0].operator_start,
                unary_assignment_chain[0].left_start,
                unary_assignment_chain[0].right_end
            ),
            (11, 9, 14)
        );
        assert_eq!(
            (
                unary_assignment_chain[1].operator_start,
                unary_assignment_chain[1].left_start,
                unary_assignment_chain[1].right_end
            ),
            (7, 4, 14)
        );
        assert_eq!(
            (
                unary_assignment_chain[2].operator_start,
                unary_assignment_chain[2].left_start,
                unary_assignment_chain[2].right_end
            ),
            (2, 0, 14)
        );
        let long_chain =
            parse_expression_precedence_long_chain_headers(b"a + b * c + d - e", 0, 17);
        assert_eq!(long_chain.len(), 4);
        assert_eq!(
            (long_chain[0].operator_start, long_chain[0].right_end),
            (6, 9)
        );
        assert_eq!(
            (long_chain[1].operator_start, long_chain[1].right_end),
            (2, 9)
        );
        assert_eq!(
            (long_chain[2].operator_start, long_chain[2].right_end),
            (10, 13)
        );
        assert_eq!(
            (long_chain[3].operator_start, long_chain[3].right_end),
            (14, 17)
        );
        let long_assignment =
            parse_expression_precedence_long_chain_headers(b"a = b = c = d = e", 0, 17);
        assert_eq!(long_assignment.len(), 4);
        assert_eq!(
            (
                long_assignment[0].operator_start,
                long_assignment[0].right_end
            ),
            (14, 17)
        );
        assert_eq!(
            (
                long_assignment[1].operator_start,
                long_assignment[1].right_end
            ),
            (10, 17)
        );
        assert_eq!(
            (
                long_assignment[2].operator_start,
                long_assignment[2].right_end
            ),
            (6, 17)
        );
        assert_eq!(
            (
                long_assignment[3].operator_start,
                long_assignment[3].right_end
            ),
            (2, 17)
        );
        let prefix_chain =
            parse_expression_precedence_long_chain_headers(b"-a + b * c + d - e", 0, 18);
        assert_eq!(prefix_chain.len(), 4);
        assert_eq!(
            (prefix_chain[0].operator_start, prefix_chain[0].right_end),
            (7, 10)
        );
        assert_eq!(
            (
                prefix_chain[1].left_start,
                prefix_chain[1].operator_start,
                prefix_chain[1].right_end
            ),
            (0, 3, 10)
        );
        assert_eq!(
            (prefix_chain[2].operator_start, prefix_chain[2].right_end),
            (11, 14)
        );
        assert_eq!(
            (prefix_chain[3].operator_start, prefix_chain[3].right_end),
            (15, 18)
        );
        let assignment_prefix_chain =
            parse_expression_precedence_long_chain_headers(b"a = -b = c = d = e", 0, 18);
        assert_eq!(assignment_prefix_chain.len(), 4);
        assert_eq!(
            (
                assignment_prefix_chain[0].operator_start,
                assignment_prefix_chain[0].left_start
            ),
            (15, 13)
        );
        assert_eq!(
            (
                assignment_prefix_chain[1].operator_start,
                assignment_prefix_chain[1].left_start
            ),
            (11, 9)
        );
        assert_eq!(
            (
                assignment_prefix_chain[2].operator_start,
                assignment_prefix_chain[2].left_start
            ),
            (7, 4)
        );
        assert_eq!(
            (
                assignment_prefix_chain[3].operator_start,
                assignment_prefix_chain[3].left_start
            ),
            (2, 0)
        );
        assert!(
            assignment_prefix_chain
                .iter()
                .all(|node| node.right_end == 18)
        );
        assert!(parse_expression_precedence_long_chain_headers(b"a + b + c + d", 0, 13).is_empty());
        assert!(
            parse_expression_precedence_long_chain_headers(b"a + (b * c] + d - e", 0, 19)
                .is_empty()
        );
        assert!(
            parse_expression_precedence_long_chain_headers(b"a + b * c) + d - e", 0, 18).is_empty()
        );
        assert!(
            parse_expression_precedence_long_chain_headers(b"a + b + c + d + e + f", 0, 21)
                .is_empty()
        );
        let empty_precedence_output: &mut [super::ExpressionPrecedenceHeader] = &mut [];
        assert_eq!(
            parse_expression_precedence_headers_into(b"a + b * c", 0, 9, empty_precedence_output,),
            0
        );
        let mut one_precedence_output = [super::ExpressionPrecedenceHeader {
            kind: 0,
            precedence: 0,
            associativity: 0,
            left_start: 0,
            left_end: 0,
            operator_start: 0,
            operator_end: 0,
            right_start: 0,
            right_end: 0,
            depth: 0,
        }];
        let two_operator_records = parse_expression_precedence_headers(b"a + b * c", 0, 9);
        assert_eq!(
            parse_expression_precedence_headers_into(
                b"a + b * c",
                0,
                9,
                &mut one_precedence_output,
            ),
            1
        );
        assert_eq!(one_precedence_output[0], two_operator_records[0]);
        let mut one_chain_output = one_precedence_output;
        let three_operator_records =
            parse_expression_precedence_chain_headers(b"a + b * c + d", 0, 13);
        assert_eq!(
            parse_expression_precedence_chain_headers_into(
                b"a + b * c + d",
                0,
                13,
                &mut one_chain_output,
            ),
            1
        );
        assert_eq!(one_chain_output[0], three_operator_records[0]);
        let mut one_long_chain_output = one_precedence_output;
        let four_operator_records =
            parse_expression_precedence_long_chain_headers(b"a + b * c + d - e", 0, 17);
        assert_eq!(
            parse_expression_precedence_long_chain_headers_into(
                b"a + b * c + d - e",
                0,
                17,
                &mut one_long_chain_output,
            ),
            1
        );
        assert_eq!(one_long_chain_output[0], four_operator_records[0]);
        let mut one_unary_output = [super::TypedExpressionHeader {
            expression_kind: 0,
            type_kind: 0,
            start: 0,
            end: 0,
            depth: 0,
        }];
        let unary_records = infer_unary_literal_types(b"--42", 0, 4);
        assert_eq!(
            infer_unary_literal_types_into(b"--42", 0, 4, &mut one_unary_output),
            1
        );
        assert_eq!(one_unary_output[0], unary_records[0]);
        let empty_unary_output: &mut [super::TypedExpressionHeader] = &mut [];
        assert_eq!(
            infer_unary_literal_types_into(b"--42", 0, 4, empty_unary_output),
            0
        );
        assert_eq!(
            infer_binary_literal_types(b"((1)) + ((2))", 0, 13)[0].end,
            13
        );
        let integer_cast = infer_cast_literal_types(b"42 as Int32", 0, 11);
        assert_eq!(integer_cast.len(), 1);
        assert_eq!(integer_cast[0].expression_kind, 4);
        assert_eq!(integer_cast[0].type_kind, 2);
        assert_eq!((integer_cast[0].start, integer_cast[0].end), (0, 11));
        let float_cast = infer_cast_literal_types(b"3.5 as Float32", 0, 14);
        assert_eq!(float_cast.len(), 1);
        assert_eq!(float_cast[0].type_kind, 3);
        assert_eq!((float_cast[0].start, float_cast[0].end), (0, 14));
        let grouped_cast = infer_cast_literal_types(b"((3.5)) as Float64", 0, 18);
        assert_eq!(grouped_cast.len(), 1);
        assert_eq!(grouped_cast[0].type_kind, 3);
        assert_eq!((grouped_cast[0].start, grouped_cast[0].end), (0, 18));
        assert_eq!(grouped_cast[0].depth, 2);
        let chained_cast = infer_cast_literal_types(b"42 as Int32 as Float64", 0, 22);
        assert_eq!(chained_cast.len(), 1);
        assert_eq!(chained_cast[0].type_kind, 3);
        assert_eq!((chained_cast[0].start, chained_cast[0].end), (0, 22));
        let nested_cast = infer_cast_literal_types(b"(42 as Int32) as Float64", 0, 24);
        assert_eq!(nested_cast.len(), 1);
        assert_eq!(nested_cast[0].type_kind, 3);
        assert_eq!((nested_cast[0].start, nested_cast[0].end), (0, 24));
        assert_eq!(nested_cast[0].depth, 1);
        let nested_group_cast = infer_cast_literal_types(b"((42 as Int32)) as Float64", 0, 26);
        assert_eq!(nested_group_cast.len(), 1);
        assert_eq!(nested_group_cast[0].type_kind, 3);
        assert_eq!(
            (nested_group_cast[0].start, nested_group_cast[0].end),
            (0, 26)
        );
        assert_eq!(nested_group_cast[0].depth, 2);
        assert!(infer_cast_literal_types(b"true as Int32", 0, 13).is_empty());
        assert!(infer_cast_literal_types(b"42 as Bool", 0, 10).is_empty());
        assert!(infer_cast_literal_types(b"(42 as Bool)", 0, 12).is_empty());
        assert!(infer_cast_literal_types(b"42 as Int32 + 1", 0, 15).is_empty());
        assert!(
            infer_cast_literal_types(
                b"42 as Int32 as Float64 as Int32 as Float64 as Int32",
                0,
                51
            )
            .is_empty()
        );
        assert!(infer_cast_literal_types(b"(42 as Int32) + 1", 0, 17).is_empty());
        let unary = infer_unary_literal_types(b"+3.5", 0, 4);
        assert_eq!(unary.len(), 1);
        assert_eq!(unary[0].expression_kind, 3);
        assert_eq!(unary[0].type_kind, 3);
        assert_eq!((unary[0].start, unary[0].end), (0, 4));
        let grouped_integer = infer_unary_literal_types(b"-(42)", 0, 5);
        assert_eq!(grouped_integer.len(), 1);
        assert_eq!(grouped_integer[0].type_kind, 2);
        assert_eq!((grouped_integer[0].start, grouped_integer[0].end), (0, 5));
        assert_eq!(grouped_integer[0].depth, 1);
        let grouped_bool = infer_unary_literal_types(b"!(true)", 0, 7);
        assert_eq!(grouped_bool.len(), 1);
        assert_eq!(grouped_bool[0].type_kind, 1);
        assert_eq!((grouped_bool[0].start, grouped_bool[0].end), (0, 7));
        assert_eq!(grouped_bool[0].depth, 1);
        let nested_integer = infer_unary_literal_types(b"-((42))", 0, 7);
        assert_eq!(nested_integer.len(), 1);
        assert_eq!(nested_integer[0].type_kind, 2);
        assert_eq!((nested_integer[0].start, nested_integer[0].end), (0, 7));
        assert_eq!(nested_integer[0].depth, 2);
        let nested_bool = infer_unary_literal_types(b"!((true))", 0, 9);
        assert_eq!(nested_bool.len(), 1);
        assert_eq!(nested_bool[0].type_kind, 1);
        assert_eq!((nested_bool[0].start, nested_bool[0].end), (0, 9));
        assert_eq!(nested_bool[0].depth, 2);
        let chained_integer = infer_unary_literal_types(b"--42", 0, 4);
        assert_eq!(chained_integer.len(), 1);
        assert_eq!(chained_integer[0].type_kind, 2);
        assert_eq!((chained_integer[0].start, chained_integer[0].end), (0, 4));
        assert_eq!(chained_integer[0].depth, 0);
        let chained_bool = infer_unary_literal_types(b"!!true", 0, 6);
        assert_eq!(chained_bool.len(), 1);
        assert_eq!(chained_bool[0].type_kind, 1);
        assert_eq!((chained_bool[0].start, chained_bool[0].end), (0, 6));
        assert_eq!(chained_bool[0].depth, 0);
        assert!(infer_unary_literal_types(b"!-1", 0, 3).is_empty());
        assert!(infer_unary_literal_types(b"-foo", 0, 4).is_empty());
        assert_eq!(
            count_tokens(b"foo+12", 0, 6),
            super::TokenCounts {
                identifiers: 1,
                numbers: 1,
                symbols: 1,
                total: 3
            }
        );
        let table = super::rust_oracle_api();
        let api = table.borrow().expect("oracle API");
        assert_eq!(scan_identifier_via_api(&api, b"play1!", 0, 6), (0, 5));
        let token_info_table = super::rust_oracle_token_info_api();
        let token_info_api = token_info_table.borrow().expect("oracle TokenInfo API");
        assert_eq!(
            token_info_api.token_info(2, 0, 3),
            super::TokenInfo {
                kind: 2,
                start: 0,
                end: 3
            }
        );
        assert_eq!(diagnostic(1001, 2), (1001, 2));
        assert_eq!(
            CorpusCase::Byte {
                value: b'A',
                expected_class: 1,
            },
            CorpusCase::Byte {
                value: b'A',
                expected_class: 1,
            }
        );
    }

    #[test]
    fn checked_in_corpus_has_stable_contract() {
        let corpus = parse_corpus(include_str!(
            "../../../examples/selfhost/token_counter.corpus"
        ))
        .expect("checked-in corpus");
        let report = run_oracle(&corpus).expect("checked-in oracle");
        assert_eq!(report.byte_cases, 18);
        assert_eq!(report.span_cases, 4);
        assert_eq!(report.api_span_cases, 4);
        assert_eq!(report.scan_cases, 4);
        assert_eq!(report.api_scan_cases, 4);
        assert_eq!(report.token_cases, 4);
        assert_eq!(report.token_info_cases, 4);
        assert_eq!(report.api_token_info_cases, 4);
        assert_eq!(report.count_cases, 4);
        assert_eq!(report.typed_literal_cases, 6);
        assert_eq!(report.typed_binary_cases, 12);
        assert_eq!(report.typed_cast_cases, 10);
        assert_eq!(report.typed_unary_cases, 12);
        assert_eq!(report.call_cases, 5);
        assert_eq!(report.typed_name_cases, 5);
        assert_eq!(report.typed_call_cases, 5);
        assert_eq!(report.typed_call_resolve_cases, 22);
        assert_eq!(report.typed_region_name_cases, 6);
        assert_eq!(report.diagnostic_cases, 4);
        assert_eq!(report.api_diagnostic_cases, 4);
        assert_eq!(report.fingerprint, "ef229dc1977d511f");
    }
}
