//! Versioned, implementation-neutral contract for the self-hosting frontend.
//!
//! This crate deliberately contains no lexer or parser implementation. A Rust
//! bootstrap, a future Jadren implementation, or another audited host can
//! expose the same `FrontendApiV1` function table.

use std::sync::RwLock;

/// Stable API schema identity.
pub const API_SCHEMA: &str = "jadren-selfhost-api-0.1";
/// Current function-table version.
pub const API_VERSION: u32 = 1;

/// Compact byte class returned by `classify_byte` for an identifier byte.
pub const BYTE_CLASS_IDENTIFIER: u8 = 1;
/// Compact byte class returned by `classify_byte` for an ASCII digit.
pub const BYTE_CLASS_DIGIT: u8 = 2;
/// Compact byte class returned by `classify_byte` for supported ASCII whitespace.
pub const BYTE_CLASS_WHITESPACE: u8 = 3;
/// Compact byte class returned by `classify_byte` for all other bytes.
pub const BYTE_CLASS_OTHER: u8 = 0;

/// C-compatible source span payload.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TokenSpan {
    /// Inclusive source start offset in the frontend's byte coordinate space.
    pub start: u64,
    /// Exclusive source end offset in the frontend's byte coordinate space.
    pub end: u64,
}

/// C-compatible diagnostic payload.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiagnosticValue {
    /// Stable diagnostic code selected by the frontend.
    pub code: u16,
    /// Stable severity value selected by the frontend.
    pub severity: u8,
}

/// C-compatible token payload used by the additive TokenInfo callback.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TokenInfo {
    /// Compact token kind: `0` none, `1` identifier, `2` number, `3` symbol.
    pub kind: u8,
    /// Inclusive source start offset.
    pub start: u64,
    /// Exclusive source end offset.
    pub end: u64,
}

/// C-compatible allocation-free token stream summary.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TokenCounts {
    /// Number of identifier tokens.
    pub identifiers: u64,
    /// Number of decimal-number tokens.
    pub numbers: u64,
    /// Number of one-byte symbol tokens.
    pub symbols: u64,
    /// Total number of tokens represented by the summary.
    pub total: u64,
}

/// Stage-2 record containing one canonical type declaration.
pub const STAGE2_JIR_RECORD_TYPE: u8 = 1;
/// Stage-2 record containing one function declaration.
pub const STAGE2_JIR_RECORD_FUNCTION: u8 = 2;
/// Stage-2 record containing one basic block.
pub const STAGE2_JIR_RECORD_BLOCK: u8 = 3;
/// Stage-2 record containing one non-terminating instruction.
pub const STAGE2_JIR_RECORD_INSTRUCTION: u8 = 4;
/// Stage-2 record containing one block terminator.
pub const STAGE2_JIR_RECORD_TERMINATOR: u8 = 5;
/// Stage-2 metadata record binding one immutable source local to an existing
/// SSA value. The record is additive; legacy streams never contain it.
pub const STAGE2_JIR_RECORD_LOCAL_BINDING_METADATA: u8 = 6;
/// Stage-2 record containing one bounded direct call to a preceding function.
/// The record is additive and is accepted only under the reviewed internal
/// one-literal call contract.
pub const STAGE2_JIR_RECORD_DIRECT_CALL: u8 = 7;
/// Current additive stage-2 record protocol. Legacy records remain accepted
/// because their kind-specific contracts are unchanged.
pub const STAGE2_JIR_PROTOCOL_VERSION: u8 = 2;
/// Opcode used by the immutable local-binding metadata record.
pub const STAGE2_JIR_LOCAL_BINDING_IMMUTABLE: u8 = 1;
/// Opcode used by the bounded type record for a scalar integer.
pub const STAGE2_JIR_TYPE_INTEGER: u8 = 1;
/// Opcode used by the bounded function record for a definition.
pub const STAGE2_JIR_FUNCTION_DEFINITION: u8 = 1;
/// Opcode used by the bounded block record for an entry block.
pub const STAGE2_JIR_BLOCK_ENTRY: u8 = 1;
/// Opcode used by the bounded instruction record for an integer constant.
pub const STAGE2_JIR_INSTRUCTION_CONSTANT: u8 = 1;
/// Opcode used by the bounded instruction record for integer addition.
pub const STAGE2_JIR_INSTRUCTION_ADD: u8 = 2;
/// Opcode used by the bounded instruction record for integer subtraction.
pub const STAGE2_JIR_INSTRUCTION_SUBTRACT: u8 = 3;
/// Opcode used by the bounded instruction record for integer multiplication.
pub const STAGE2_JIR_INSTRUCTION_MULTIPLY: u8 = 4;
/// Maximum number of typed Int32 parameters in the bounded stage-2 preview.
/// Parameter values occupy the first function-local SSA identities; the
/// function record carries this count in `operand_a` without changing the
/// fixed record layout.
pub const STAGE2_JIR_MAX_PARAMETERS: u8 = 2;
/// Opcode used by the bounded terminator record for a return.
pub const STAGE2_JIR_TERMINATOR_RETURN: u8 = 1;
/// Flag carried by the bounded signed-integer type record.
pub const STAGE2_JIR_FLAG_SIGNED: u8 = 1;
/// Flag carried by a bounded exported function definition.
pub const STAGE2_JIR_FLAG_EXPORTED: u8 = 1;
/// Flag carried by a bounded return terminator with an SSA operand.
pub const STAGE2_JIR_FLAG_HAS_VALUE: u8 = 1;
/// Version flag carried by an immutable local-binding metadata record.
pub const STAGE2_JIR_FLAG_METADATA_V2: u8 = 1;
/// Complete stage-2 status: validated input, lowered functions and full output.
pub const STAGE2_JIR_STATUS_COMPLETE: u64 = 7;

/// One fixed-size record in the bounded stage-2 JIR hand-off.
///
/// The record mirrors a deliberately small canonical JIR subset without
/// claiming to serialize the complete `jadren-jir` model. `kind` is
/// `1=type`, `2=function`, `3=block`, `4=instruction`, `5=terminator`,
/// `6=immutable local-binding metadata`, or `7=bounded direct call`. For kind `6`, `source_start..end`
/// is the declaration identifier span, `operand_a..b` is the use span and
/// `value_index` names the already-emitted SSA value.
/// For kind `7`, `operand_a` is the preceding callee function index,
/// `operand_b` is the already-defined one-literal argument SSA value and
/// `source_start..end` is the complete call expression span.
/// Dense identities and operands are represented as platform-neutral `u64`
/// indices for the currently supported 64-bit hosts. A function may carry at
/// most two explicit Int32 parameters plus a bounded sequence of constant and
/// binary instruction records; the parameter count is stored in the function
/// record's `operand_a`, and parameter values occupy the first function-local
/// SSA identities. The record layout remains fixed while the stream length
/// describes the expression.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Stage2JirRecord {
    pub kind: u8,
    pub opcode: u8,
    pub type_kind: u8,
    pub flags: u8,
    pub function_index: u64,
    pub block_index: u64,
    pub value_index: u64,
    pub operand_a: u64,
    pub operand_b: u64,
    pub source_start: u64,
    pub source_end: u64,
}

/// C-compatible summary returned by the bounded stage-2 JIR emitter.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Stage2JirSummary {
    pub functions_seen: u64,
    pub statements_seen: u64,
    pub calls_seen: u64,
    pub records_required: u64,
    pub records_emitted: u64,
    pub functions_lowered: u64,
    pub errors: u64,
    pub status_flags: u64,
}

/// C-compatible summary returned by the fused bounded Stage-2 frontend-to-JIR
/// hand-off. The 64-byte layout keeps frontend validity/completeness and JIR
/// emission completeness explicit without changing the existing summaries.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Stage2PipelineSummary {
    pub source_bytes: u64,
    pub tokens_emitted: u64,
    pub syntax_errors: u64,
    pub frontend_status_flags: u64,
    pub records_required: u64,
    pub records_emitted: u64,
    pub functions_lowered: u64,
    pub status_flags: u64,
}

/// C-compatible typed metadata for one builtin literal expression.
///
/// This is deliberately a narrow hand-off: `expression_kind` identifies a
/// literal, a binary expression over builtin literals, a unary expression, a
/// bounded explicit numeric cast over a literal/group and cast chain, a name
/// resolved through a caller-owned binding slice, or a caller-bound call;
/// `type_kind` identifies only builtin result types. It is not a typed AST
/// node or a general name/call resolver.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TypedExpressionHeader {
    /// Stable expression category, currently [`TYPED_EXPRESSION_KIND_LITERAL`].
    pub expression_kind: u8,
    /// Builtin type category selected for the expression.
    pub type_kind: u8,
    /// Inclusive source start offset.
    pub start: u64,
    /// Exclusive source end offset.
    pub end: u64,
    /// Delimiter nesting depth at the expression site.
    pub depth: u64,
}

/// C-compatible caller-owned binding used by the bounded typed-name hand-off.
/// `name_start..name_end` points into the same source byte buffer passed to the
/// hand-off. Bindings are matched by exact ASCII bytes; no scope, shadowing,
/// alias, generic, or user-defined-type resolution is implied.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TypedNameBindingHeader {
    /// Inclusive source start offset of the bound identifier.
    pub name_start: u64,
    /// Exclusive source end offset of the bound identifier.
    pub name_end: u64,
    /// Builtin type category selected by the caller.
    pub type_kind: u8,
}

/// C-compatible caller-owned function signature entry for the bounded typed
/// call hand-off. The name span points into the source buffer supplied to the
/// same call; parameter count and builtin return type are selected by the
/// caller. This is not overload resolution or a complete function type.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TypedCallBindingHeader {
    /// Inclusive source start offset of the bound function identifier.
    pub name_start: u64,
    /// Exclusive source end offset of the bound function identifier.
    pub name_end: u64,
    /// Number of positional arguments accepted by this bounded signature.
    pub parameter_count: u64,
    /// Builtin return type category selected by the caller.
    pub return_type_kind: u8,
}

/// C-compatible caller-owned candidate used by the bounded typed-call
/// resolver. Candidates match the callee bytes and positional arity. A
/// candidate with `generic_parameter_count == 0` is a concrete overload; a
/// positive count is a generic fallback. `parameter_type_kind == 0` keeps the
/// legacy arity-only wildcard; for a one-argument call, `1..=5` requests an
/// exact builtin literal type match. For bounded multi-parameter matching,
/// `parameter_type_start..parameter_type_end` points into the same source
/// buffer and contains a comma-separated list of builtin type names. A zero
/// span keeps the arity-only wildcard. `generic_bound_kind` is zero for an
/// unconstrained generic and `1..=3` for the bounded Numeric, Boolean or Text
/// families. The resolver prefers an exact type match, then a compatible
/// bound/substitution, then the lowest generic count, and rejects ties as
/// ambiguous. `generic_substitution_kind == 1` is a single-argument generic
/// whose return builtin type is copied from that argument; `2` requires two or
/// more literal arguments to share a builtin type and copies that type from the
/// first argument; `3` accepts two or more literal arguments with independently
/// valid builtin types and copies the first argument's type. All use a zero
/// declared `return_type_kind`. This remains a bounded builtin hand-off, not a
/// full trait/substitution resolver.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TypedCallCandidateHeader {
    /// Inclusive source start offset of the candidate function name.
    pub name_start: u64,
    /// Exclusive source end offset of the candidate function name.
    pub name_end: u64,
    /// Number of positional arguments accepted by the candidate.
    pub parameter_count: u64,
    /// Number of generic parameters; zero marks a concrete overload.
    pub generic_parameter_count: u64,
    /// Builtin return type category selected by the caller.
    pub return_type_kind: u8,
    /// Optional exact builtin type for the single positional argument. Zero
    /// means wildcard/legacy arity-only matching.
    pub parameter_type_kind: u8,
    /// Inclusive source start of a bounded comma-separated builtin parameter
    /// type list. Zero with `parameter_type_end == 0` means wildcard.
    pub parameter_type_start: u16,
    /// Exclusive source end of the bounded parameter type list.
    pub parameter_type_end: u16,
    /// Optional builtin family bound for a generic candidate: `0` is
    /// unconstrained, `1` Numeric, `2` Boolean and `3` Text.
    pub generic_bound_kind: u8,
    /// `0` keeps the declared return type; `1` substitutes the first
    /// argument's builtin type for a one-argument generic candidate; `2`
    /// requires all arguments to share a builtin type and substitutes that
    /// type for a multi-argument generic candidate; `3` derives from the first
    /// argument while allowing remaining literal arguments to differ.
    pub generic_substitution_kind: u8,
}

/// C-compatible caller-owned scoped binding used by the bounded name
/// resolution hand-off. The binding's `scope_depth` is compared with the
/// delimiter depth of each identifier token; the deepest visible matching
/// binding wins, with later entries winning ties. This is deliberately a
/// lexical-depth prototype, not a complete region/span resolver.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TypedScopedNameBindingHeader {
    /// Inclusive source start offset of the bound identifier.
    pub name_start: u64,
    /// Exclusive source end offset of the bound identifier.
    pub name_end: u64,
    /// Builtin type category selected by the caller.
    pub type_kind: u8,
    /// Delimiter/scope depth at which this binding is visible.
    pub scope_depth: u64,
}

/// C-compatible caller-owned region binding used by the bounded
/// region-aware name resolution hand-off. The binding is visible only when
/// the identifier token lies inside `region_start..region_end`; the deepest
/// visible matching binding wins, with later entries winning ties.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TypedRegionNameBindingHeader {
    /// Inclusive source start offset of the bound identifier.
    pub name_start: u64,
    /// Exclusive source end offset of the bound identifier.
    pub name_end: u64,
    /// Inclusive source start offset of the binding's visibility region.
    pub region_start: u64,
    /// Exclusive source end offset of the binding's visibility region.
    pub region_end: u64,
    /// Builtin type category selected by the caller.
    pub type_kind: u8,
    /// Delimiter/scope depth at which this binding is visible.
    pub scope_depth: u64,
}

/// C-compatible bounded binary-expression node carrying parser precedence.
///
/// The self-hosting prototype emits exactly two nodes in child-before-parent
/// order for a three-operand expression. `associativity` is `1` for left and
/// `2` for right associativity; this additive record does not change any
/// existing frontend table layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExpressionPrecedenceHeader {
    /// Compact expression kind matching the source operator family.
    pub kind: u8,
    /// Parser precedence (higher values bind more tightly).
    pub precedence: u8,
    /// `1` for left-associative, `2` for right-associative.
    pub associativity: u8,
    /// Inclusive left operand start.
    pub left_start: u64,
    /// Exclusive left operand end.
    pub left_end: u64,
    /// Inclusive operator start.
    pub operator_start: u64,
    /// Exclusive operator end.
    pub operator_end: u64,
    /// Inclusive right operand start.
    pub right_start: u64,
    /// Exclusive right operand end.
    pub right_end: u64,
    /// Delimiter nesting depth at the operator.
    pub depth: u64,
}

/// Typed-expression category for a builtin literal.
pub const TYPED_EXPRESSION_KIND_LITERAL: u8 = 1;
/// Typed-expression category for a single binary expression over builtin
/// literal operands.
pub const TYPED_EXPRESSION_KIND_BINARY: u8 = 2;
/// Typed-expression category for a single unary expression over one builtin
/// literal operand.
pub const TYPED_EXPRESSION_KIND_UNARY: u8 = 3;
/// Typed-expression category for a caller-bound identifier name.
pub const TYPED_EXPRESSION_KIND_NAME: u8 = 5;
/// Typed-expression category for a name selected by lexical-depth shadowing.
pub const TYPED_EXPRESSION_KIND_SCOPED_NAME: u8 = 6;
/// Typed-expression category for a name selected by region-span visibility.
pub const TYPED_EXPRESSION_KIND_REGION_NAME: u8 = 7;
/// Typed-expression category for a caller-bound function call.
pub const TYPED_EXPRESSION_KIND_CALL: u8 = 8;
/// Builtin type category for `true` and `false`.
pub const TYPE_KIND_BOOL: u8 = 1;
/// Builtin type category for decimal integer literals.
pub const TYPE_KIND_INTEGER: u8 = 2;
/// Builtin type category for decimal floating-point literals.
pub const TYPE_KIND_FLOAT: u8 = 3;
/// Builtin type category for quoted string literals.
pub const TYPE_KIND_STRING: u8 = 4;
/// Builtin type category for quoted character literals.
pub const TYPE_KIND_CHAR: u8 = 5;

/// Function type for byte classification.
pub type ClassifyByteFn = extern "C" fn(byte: u8) -> u8;
/// Function type for token-span construction.
pub type TokenSpanFn = extern "C" fn(start: u64, end: u64) -> TokenSpan;
/// Function type for diagnostic payload construction.
pub type DiagnosticFn = extern "C" fn(code: u16, severity: u8) -> DiagnosticValue;
/// Function type for compact token payload construction.
pub type TokenInfoFn = extern "C" fn(kind: u8, start: u64, end: u64) -> TokenInfo;

/// Version for the additive TokenInfo callback extension.
pub const TOKEN_INFO_API_VERSION: u32 = 1;

/// Additive callback contract that extends [`FrontendApiV1`] without changing
/// its stable layout or version.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FrontendTokenInfoApiV1 {
    /// Extension-table version, currently [`TOKEN_INFO_API_VERSION`].
    pub version: u32,
    /// Construct a compact token payload without normalizing its span.
    pub token_info: TokenInfoFn,
}

impl FrontendTokenInfoApiV1 {
    /// Creates a complete TokenInfo extension table.
    #[must_use]
    pub const fn new(token_info: TokenInfoFn) -> Self {
        Self {
            version: TOKEN_INFO_API_VERSION,
            token_info,
        }
    }

    /// Validates the extension version before calling its function pointer.
    pub const fn validate(&self) -> Result<(), ApiError> {
        if self.version == TOKEN_INFO_API_VERSION {
            Ok(())
        } else {
            Err(ApiError::UnsupportedVersion(self.version))
        }
    }

    /// Borrows the extension for a bounded call lifetime.
    pub fn borrow(&self) -> Result<FrontendTokenInfoLease<'_>, ApiError> {
        self.validate()?;
        Ok(FrontendTokenInfoLease { table: self })
    }
}

/// Borrowed view of the additive TokenInfo callback extension.
pub struct FrontendTokenInfoLease<'a> {
    table: &'a FrontendTokenInfoApiV1,
}

impl FrontendTokenInfoLease<'_> {
    /// Returns the validated extension version.
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.table.version
    }

    /// Calls the borrowed TokenInfo constructor.
    #[must_use]
    pub fn token_info(&self, kind: u8, start: u64, end: u64) -> TokenInfo {
        (self.table.token_info)(kind, start, end)
    }
}

/// Versioned frontend function table that a future Jadren implementation can
/// provide without depending on Rust-internal AST/HIR types.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FrontendApiV1 {
    /// Function-table version, currently [`API_VERSION`].
    pub version: u32,
    /// Classify one input byte.
    pub classify_byte: ClassifyByteFn,
    /// Construct a source span without normalizing its offsets.
    pub token_span: TokenSpanFn,
    /// Construct a diagnostic payload without allocating.
    pub diagnostic: DiagnosticFn,
}

impl FrontendApiV1 {
    /// Creates a complete function table with the stable version marker.
    #[must_use]
    pub const fn new(
        classify_byte: ClassifyByteFn,
        token_span: TokenSpanFn,
        diagnostic: DiagnosticFn,
    ) -> Self {
        Self {
            version: API_VERSION,
            classify_byte,
            token_span,
            diagnostic,
        }
    }

    /// Validates the version boundary before a consumer calls any function.
    pub const fn validate(&self) -> Result<(), ApiError> {
        if self.version == API_VERSION {
            Ok(())
        } else {
            Err(ApiError::UnsupportedVersion(self.version))
        }
    }

    /// Borrows the table for a bounded call lifetime after version validation.
    pub fn borrow(&self) -> Result<FrontendApiLease<'_>, ApiError> {
        self.validate()?;
        Ok(FrontendApiLease { table: self })
    }
}

/// Borrowed call view that prevents the producer table from being dropped or
/// replaced while a consumer is using its function pointers.
pub struct FrontendApiLease<'a> {
    table: &'a FrontendApiV1,
}

impl FrontendApiLease<'_> {
    /// Returns the validated function-table version.
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.table.version
    }

    /// Calls the borrowed byte-classification entry.
    #[must_use]
    pub fn classify_byte(&self, byte: u8) -> u8 {
        (self.table.classify_byte)(byte)
    }

    /// Calls the borrowed token-span entry.
    #[must_use]
    pub fn token_span(&self, start: u64, end: u64) -> TokenSpan {
        (self.table.token_span)(start, end)
    }

    /// Calls the borrowed diagnostic entry.
    #[must_use]
    pub fn diagnostic(&self, code: u16, severity: u8) -> DiagnosticValue {
        (self.table.diagnostic)(code, severity)
    }
}

/// Caller-owned optional slot for installing and clearing one frontend table.
/// Synchronization between threads remains the caller's responsibility.
#[derive(Default)]
pub struct FrontendApiSlot {
    table: Option<FrontendApiV1>,
}

impl FrontendApiSlot {
    /// Creates an empty slot with no callable frontend installed.
    #[must_use]
    pub const fn empty() -> Self {
        Self { table: None }
    }

    /// Validates and installs a complete table by value.
    pub fn install(&mut self, table: FrontendApiV1) -> Result<(), ApiError> {
        table.validate()?;
        self.table = Some(table);
        Ok(())
    }

    /// Removes the installed table. Existing leases keep their borrow alive;
    /// the slot itself cannot be mutably accessed until those leases end.
    pub fn clear(&mut self) {
        self.table = None;
    }

    /// Borrows the installed table for a bounded call lifetime.
    pub fn borrow(&self) -> Result<FrontendApiLease<'_>, ApiError> {
        self.table.as_ref().ok_or(ApiError::Unavailable)?.borrow()
    }
}

/// Caller-owned thread-safe registry for one frontend function table.
///
/// Writers replace or clear the table under a lock. Consumers take a copy with
/// [`Self::snapshot`] and then create a borrowed lease from that copy, so calls
/// do not hold the registry lock. The caller must keep the provider code loaded
/// for the lifetime of every snapshot; this type does not manage dynamic
/// library unloading.
pub struct FrontendApiRegistry {
    table: RwLock<Option<FrontendApiV1>>,
}

impl Default for FrontendApiRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl FrontendApiRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            table: RwLock::new(None),
        }
    }

    /// Validates and installs a complete table for future snapshots.
    pub fn install(&self, table: FrontendApiV1) -> Result<(), ApiError> {
        table.validate()?;
        self.table
            .write()
            .map_err(|_| ApiError::RegistryPoisoned)?
            .replace(table);
        Ok(())
    }

    /// Clears the current table. Existing snapshots remain independent copies.
    pub fn clear(&self) -> Result<(), ApiError> {
        self.table
            .write()
            .map_err(|_| ApiError::RegistryPoisoned)?
            .take();
        Ok(())
    }

    /// Copies the current table while holding a read lock.
    ///
    /// The returned table can be borrowed with [`FrontendApiV1::borrow`]. The
    /// copy keeps concurrent install/clear operations from invalidating the
    /// caller's lease, while provider code lifetime remains the caller's duty.
    pub fn snapshot(&self) -> Result<FrontendApiV1, ApiError> {
        self.table
            .read()
            .map_err(|_| ApiError::RegistryPoisoned)?
            .as_ref()
            .copied()
            .ok_or(ApiError::Unavailable)
    }
}

/// Failure while validating or accessing a frontend function table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApiError {
    /// No table has been installed in the caller-owned slot.
    Unavailable,
    /// The producer advertises a function-table version this consumer does not support.
    UnsupportedVersion(u32),
    /// A caller-owned registry lock was poisoned by a panic in another thread.
    RegistryPoisoned,
}

#[cfg(test)]
mod tests {
    use std::mem::{align_of, size_of};

    use super::{
        API_SCHEMA, API_VERSION, ApiError, DiagnosticValue, ExpressionPrecedenceHeader,
        FrontendApiRegistry, FrontendApiSlot, FrontendApiV1, FrontendTokenInfoApiV1,
        Stage2JirRecord, Stage2JirSummary, Stage2PipelineSummary, TOKEN_INFO_API_VERSION,
        TokenCounts, TokenInfo, TokenSpan, TypedCallBindingHeader, TypedCallCandidateHeader,
        TypedExpressionHeader, TypedNameBindingHeader, TypedRegionNameBindingHeader,
        TypedScopedNameBindingHeader,
    };

    extern "C" fn classify(_: u8) -> u8 {
        0
    }

    extern "C" fn span(start: u64, end: u64) -> TokenSpan {
        TokenSpan { start, end }
    }

    extern "C" fn diagnostic(code: u16, severity: u8) -> DiagnosticValue {
        DiagnosticValue { code, severity }
    }

    extern "C" fn token_info(kind: u8, start: u64, end: u64) -> TokenInfo {
        TokenInfo { kind, start, end }
    }

    #[test]
    fn versioned_table_has_stable_contract_identity() {
        let table = FrontendApiV1::new(classify, span, diagnostic);
        assert_eq!(API_SCHEMA, "jadren-selfhost-api-0.1");
        assert_eq!(table.version, API_VERSION);
        assert_eq!(table.validate(), Ok(()));
        assert_eq!(size_of::<TokenSpan>(), 16);
        assert_eq!(align_of::<TokenSpan>(), 8);
        assert_eq!(size_of::<DiagnosticValue>(), 4);
        assert_eq!(align_of::<DiagnosticValue>(), 2);
        assert_eq!(size_of::<TokenCounts>(), 32);
        assert_eq!(align_of::<TokenCounts>(), 8);
        assert_eq!(size_of::<TypedExpressionHeader>(), 32);
        assert_eq!(align_of::<TypedExpressionHeader>(), 8);
        assert_eq!(size_of::<TypedNameBindingHeader>(), 24);
        assert_eq!(align_of::<TypedNameBindingHeader>(), 8);
        assert_eq!(size_of::<TypedCallBindingHeader>(), 32);
        assert_eq!(align_of::<TypedCallBindingHeader>(), 8);
        assert_eq!(size_of::<TypedCallCandidateHeader>(), 40);
        assert_eq!(align_of::<TypedCallCandidateHeader>(), 8);
        assert_eq!(size_of::<TypedScopedNameBindingHeader>(), 32);
        assert_eq!(align_of::<TypedScopedNameBindingHeader>(), 8);
        assert_eq!(size_of::<TypedRegionNameBindingHeader>(), 48);
        assert_eq!(align_of::<TypedRegionNameBindingHeader>(), 8);
        assert_eq!(size_of::<ExpressionPrecedenceHeader>(), 64);
        assert_eq!(align_of::<ExpressionPrecedenceHeader>(), 8);
        assert_eq!(size_of::<Stage2JirRecord>(), 64);
        assert_eq!(align_of::<Stage2JirRecord>(), 8);
        assert_eq!(size_of::<Stage2JirSummary>(), 64);
        assert_eq!(align_of::<Stage2JirSummary>(), 8);
        assert_eq!(size_of::<Stage2PipelineSummary>(), 64);
        assert_eq!(align_of::<Stage2PipelineSummary>(), 8);
    }

    #[test]
    fn token_info_extension_has_stable_layout_and_borrowed_call() {
        let table = FrontendTokenInfoApiV1::new(token_info);
        assert_eq!(table.version, TOKEN_INFO_API_VERSION);
        assert_eq!(table.validate(), Ok(()));
        assert_eq!(size_of::<TokenInfo>(), 24);
        assert_eq!(align_of::<TokenInfo>(), 8);
        let lease = table.borrow().expect("valid extension table");
        assert_eq!(lease.version(), TOKEN_INFO_API_VERSION);
        assert_eq!(
            lease.token_info(3, 18, 19),
            TokenInfo {
                kind: 3,
                start: 18,
                end: 19
            }
        );
    }

    #[test]
    fn token_info_extension_rejects_bad_version() {
        let mut table = FrontendTokenInfoApiV1::new(token_info);
        table.version = TOKEN_INFO_API_VERSION + 1;
        assert_eq!(
            table.validate(),
            Err(ApiError::UnsupportedVersion(TOKEN_INFO_API_VERSION + 1))
        );
    }

    #[test]
    fn version_mismatch_is_rejected_before_calls() {
        let mut table = FrontendApiV1::new(classify, span, diagnostic);
        table.version = API_VERSION + 1;
        assert_eq!(
            table.validate(),
            Err(ApiError::UnsupportedVersion(API_VERSION + 1))
        );
    }

    #[test]
    fn slot_requires_install_and_exposes_borrowed_lease() {
        let mut slot = FrontendApiSlot::empty();
        assert_eq!(
            slot.borrow().map(|lease| lease.version()),
            Err(ApiError::Unavailable)
        );
        slot.install(FrontendApiV1::new(classify, span, diagnostic))
            .expect("valid table");
        {
            let lease = slot.borrow().expect("installed table");
            assert_eq!(lease.version(), API_VERSION);
            assert_eq!(lease.classify_byte(b'A'), 0);
            assert_eq!(lease.token_span(8, 2), TokenSpan { start: 8, end: 2 });
            assert_eq!(
                lease.diagnostic(1001, 2),
                DiagnosticValue {
                    code: 1001,
                    severity: 2
                }
            );
        }
        slot.clear();
        assert_eq!(
            slot.borrow().map(|lease| lease.version()),
            Err(ApiError::Unavailable)
        );
    }

    #[test]
    fn registry_requires_install_and_supports_snapshot() {
        let registry = FrontendApiRegistry::new();
        assert!(matches!(registry.snapshot(), Err(ApiError::Unavailable)));
        registry
            .install(FrontendApiV1::new(classify, span, diagnostic))
            .expect("valid table");

        let snapshot = registry.snapshot().expect("installed table");
        {
            let lease = snapshot.borrow().expect("valid snapshot");
            assert_eq!(lease.version(), API_VERSION);
            assert_eq!(lease.classify_byte(b'B'), 0);
            assert_eq!(lease.token_span(5, 1), TokenSpan { start: 5, end: 1 });
        }

        registry.clear().expect("clear registry");
        assert!(matches!(registry.snapshot(), Err(ApiError::Unavailable)));
    }

    #[test]
    fn registry_rejects_bad_version_without_installing() {
        let registry = FrontendApiRegistry::new();
        let mut table = FrontendApiV1::new(classify, span, diagnostic);
        table.version = API_VERSION + 9;
        assert_eq!(
            registry.install(table),
            Err(ApiError::UnsupportedVersion(API_VERSION + 9))
        );
        assert!(matches!(registry.snapshot(), Err(ApiError::Unavailable)));
    }

    #[test]
    fn registry_snapshot_is_safe_to_use_after_clear() {
        let registry = FrontendApiRegistry::new();
        registry
            .install(FrontendApiV1::new(classify, span, diagnostic))
            .expect("valid table");
        let snapshot = registry.snapshot().expect("installed table");
        registry.clear().expect("clear registry");

        let lease = snapshot.borrow().expect("snapshot remains valid");
        assert_eq!(lease.classify_byte(b'C'), 0);
        assert_eq!(
            lease.diagnostic(7, 1),
            DiagnosticValue {
                code: 7,
                severity: 1
            }
        );
    }

    #[test]
    fn registry_supports_concurrent_snapshots() {
        use std::sync::Arc;

        let registry = Arc::new(FrontendApiRegistry::new());
        registry
            .install(FrontendApiV1::new(classify, span, diagnostic))
            .expect("valid table");

        let workers = (0..4)
            .map(|worker| {
                let registry = Arc::clone(&registry);
                std::thread::spawn(move || {
                    let snapshot = registry.snapshot().expect("installed table");
                    let lease = snapshot.borrow().expect("valid snapshot");
                    assert_eq!(lease.classify_byte(worker), 0);
                    assert_eq!(
                        lease.token_span(worker.into(), 9),
                        TokenSpan {
                            start: worker.into(),
                            end: 9
                        }
                    );
                })
            })
            .collect::<Vec<_>>();

        for worker in workers {
            worker.join().expect("snapshot worker");
        }
    }
}
