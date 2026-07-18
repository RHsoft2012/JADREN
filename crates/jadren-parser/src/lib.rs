//! Recovering parser and initial abstract syntax tree for Jadren.

mod lower;

pub use lower::{AstLoweringOutput, lower_syntax_tree};

use jadren_diagnostics::{Diagnostic, Severity};
use jadren_lexer::{Keyword, Operator, Punctuation, Token, TokenKind};
use jadren_source::{SourceFile, Span};
use jadren_syntax::{NodeSpec, SyntaxKind, SyntaxTree};

/// Parsed source file.
#[derive(Clone, Debug, Default)]
pub struct AstFile {
    /// Optional module declaration.
    pub module: Option<Path>,
    /// Imported paths.
    pub imports: Vec<Path>,
    /// Top-level declarations.
    pub items: Vec<Item>,
}

/// Dot-separated source path.
#[derive(Clone, Debug)]
pub struct Path {
    /// Path segments.
    pub segments: Vec<Name>,
    /// Full source range.
    pub span: Span,
}

/// Named source element.
#[derive(Clone, Debug)]
pub struct Name {
    /// Identifier text.
    pub text: String,
    /// Identifier range.
    pub span: Span,
}

/// Top-level item.
#[derive(Clone, Debug)]
pub enum Item {
    /// Function declaration.
    Function(Function),
    /// C or platform ABI extern declaration block.
    ExternBlock(ExternBlock),
    /// Plain value record declaration.
    Struct(RecordDeclaration),
    /// Data-oriented component declaration.
    Component(RecordDeclaration),
    /// Algebraic enum declaration.
    Enum(EnumDeclaration),
}

/// Declaration annotation such as `@noalloc` or `@compute(...)`.
#[derive(Clone, Debug)]
pub struct Annotation {
    /// Annotation path.
    pub name: Path,
    /// Positional or named arguments.
    pub arguments: Vec<AnnotationArgument>,
    /// Full annotation range including `@`.
    pub span: Span,
}

/// One annotation argument.
#[derive(Clone, Debug)]
pub struct AnnotationArgument {
    /// Optional argument name before `:`.
    pub name: Option<Name>,
    /// Argument value.
    pub value: Expression,
    /// Full argument range.
    pub span: Span,
}

/// Generic type parameter with optional trait bounds.
#[derive(Clone, Debug)]
pub struct GenericParameter {
    /// Parameter name.
    pub name: Name,
    /// `+`-separated bounds.
    pub bounds: Vec<TypeRef>,
    /// Full parameter range.
    pub span: Span,
}

/// Function declaration.
#[derive(Clone, Debug)]
pub struct Function {
    /// Declaration annotations.
    pub annotations: Vec<Annotation>,
    /// Whether the declaration is public.
    pub is_public: bool,
    /// Function name.
    pub name: Name,
    /// Generic type parameters.
    pub generic_parameters: Vec<GenericParameter>,
    /// Function parameters.
    pub parameters: Vec<Parameter>,
    /// Optional return type.
    pub return_type: Option<TypeRef>,
    /// Function body.
    pub body: Block,
    /// Full declaration range.
    pub span: Span,
}

/// Foreign-function declaration block such as `extern "C" { ... }`.
#[derive(Clone, Debug)]
pub struct ExternBlock {
    /// Calling-convention spelling, currently `C`.
    pub abi: String,
    /// Functions imported from the foreign ABI.
    pub functions: Vec<ExternFunction>,
    /// Full source range.
    pub span: Span,
}

/// One function imported from an extern block.
#[derive(Clone, Debug)]
pub struct ExternFunction {
    /// Whether the declaration requires an unsafe call boundary.
    pub is_unsafe: bool,
    /// Foreign symbol name.
    pub name: Name,
    /// Typed parameters.
    pub parameters: Vec<Parameter>,
    /// Optional return type.
    pub return_type: Option<TypeRef>,
    /// Full source range.
    pub span: Span,
}

/// `struct` or `component` declaration.
#[derive(Clone, Debug)]
pub struct RecordDeclaration {
    /// Declaration annotations.
    pub annotations: Vec<Annotation>,
    /// Whether the declaration is public.
    pub is_public: bool,
    /// Type name.
    pub name: Name,
    /// Generic type parameters.
    pub generic_parameters: Vec<GenericParameter>,
    /// Declared fields.
    pub fields: Vec<Field>,
    /// Full declaration range.
    pub span: Span,
}

/// Named record field.
#[derive(Clone, Debug)]
pub struct Field {
    /// Whether the field is public.
    pub is_public: bool,
    /// Field name.
    pub name: Name,
    /// Field type.
    pub ty: TypeRef,
    /// Full field range.
    pub span: Span,
}

/// Algebraic enum declaration.
#[derive(Clone, Debug)]
pub struct EnumDeclaration {
    /// Declaration annotations.
    pub annotations: Vec<Annotation>,
    /// Whether the declaration is public.
    pub is_public: bool,
    /// Enum name.
    pub name: Name,
    /// Generic type parameters.
    pub generic_parameters: Vec<GenericParameter>,
    /// Variants.
    pub variants: Vec<EnumVariant>,
    /// Full declaration range.
    pub span: Span,
}

/// Enum variant with optional payload fields.
#[derive(Clone, Debug)]
pub struct EnumVariant {
    /// Variant name.
    pub name: Name,
    /// Tuple-like or named payload fields.
    pub fields: Vec<EnumVariantField>,
    /// Full variant range.
    pub span: Span,
}

/// Enum variant payload field.
#[derive(Clone, Debug)]
pub struct EnumVariantField {
    /// Optional field name for `name: Type` payloads.
    pub name: Option<Name>,
    /// Payload type.
    pub ty: TypeRef,
    /// Full field range.
    pub span: Span,
}

/// Function parameter.
#[derive(Clone, Debug)]
pub struct Parameter {
    /// Parameter name.
    pub name: Name,
    /// Parameter type.
    pub ty: TypeRef,
    /// Full parameter range.
    pub span: Span,
}

/// Type reference.
#[derive(Clone, Debug)]
pub enum TypeRef {
    /// Path type with optional generic arguments.
    Path {
        /// Referenced path.
        path: Path,
        /// Generic arguments.
        arguments: Vec<TypeRef>,
        /// Full type range.
        span: Span,
    },
    /// Fixed-size array type `[T; N]`.
    Array {
        /// Element type.
        element: Box<TypeRef>,
        /// Source range of the constant length expression.
        length: Span,
        /// Full type range.
        span: Span,
    },
    /// Explicit ownership or access capability such as `read Buffer<T>`.
    Capability {
        /// Capability keyword.
        capability: TypeCapability,
        /// Wrapped value type.
        inner: Box<TypeRef>,
        /// Full type range.
        span: Span,
    },
    /// Explicit function pointer type such as `fn(Int32) -> Int32`.
    Function {
        /// Parameter types in declaration order.
        parameters: Vec<TypeRef>,
        /// Optional result type; omitted results are implicit `Unit`.
        return_type: Option<Box<TypeRef>>,
        /// Full type range.
        span: Span,
    },
}

/// Capability keyword used by a source type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TypeCapability {
    /// Unique owning value.
    Owned,
    /// Shared read-only borrow.
    Read,
    /// Exclusive writable borrow.
    Write,
}

impl TypeRef {
    /// Returns the full source range.
    #[must_use]
    pub const fn span(&self) -> Span {
        match self {
            Self::Path { span, .. }
            | Self::Array { span, .. }
            | Self::Capability { span, .. }
            | Self::Function { span, .. } => *span,
        }
    }
}

/// Braced statement block.
#[derive(Clone, Debug)]
pub struct Block {
    /// Statements in source order.
    pub statements: Vec<Statement>,
    /// Full block range.
    pub span: Span,
}

/// Statement.
#[derive(Clone, Debug)]
pub enum Statement {
    /// `let` or `var` binding.
    Binding {
        /// Whether the binding is mutable.
        mutable: bool,
        /// Binding name.
        name: Name,
        /// Optional explicit type.
        ty: Option<TypeRef>,
        /// Optional initializer.
        value: Option<Expression>,
        /// Full statement range.
        span: Span,
    },
    /// `return` statement.
    Return {
        /// Optional return expression.
        value: Option<Expression>,
        /// Full statement range.
        span: Span,
    },
    /// Lexically scoped region allocator.
    Region {
        /// Region handle visible inside the body.
        name: Name,
        /// Region-owned lexical body.
        body: Block,
        /// Full statement range.
        span: Span,
    },
    /// Conditional `while` loop.
    While {
        /// Loop condition evaluated before each iteration.
        condition: Expression,
        /// Loop body.
        body: Block,
        /// Full statement range.
        span: Span,
    },
    /// Iterates a fixed-size array binding in source order.
    For {
        /// Per-iteration binding.
        binding: Name,
        /// Iterable expression.
        iterable: Expression,
        /// Loop body.
        body: Block,
        /// Full statement range.
        span: Span,
    },
    /// Exits the nearest enclosing loop.
    Break {
        /// Full statement range.
        span: Span,
    },
    /// Jumps to the next iteration of the nearest enclosing loop.
    Continue {
        /// Full statement range.
        span: Span,
    },
    /// Expression statement.
    Expression {
        /// Parsed expression.
        expression: Expression,
        /// Whether an explicit semicolon was present.
        terminated: bool,
    },
}

/// Expression.
#[derive(Clone, Debug)]
pub enum Expression {
    /// Identifier.
    Name(Name),
    /// Literal token retained by range and kind.
    Literal {
        /// Literal kind.
        kind: LiteralKind,
        /// Literal range.
        span: Span,
    },
    /// Unary operator expression.
    Unary {
        /// Operator.
        operator: Operator,
        /// Operand.
        operand: Box<Expression>,
        /// Full expression range.
        span: Span,
    },
    /// Binary or assignment expression.
    Binary {
        /// Left operand.
        left: Box<Expression>,
        /// Operator.
        operator: Operator,
        /// Right operand.
        right: Box<Expression>,
        /// Full expression range.
        span: Span,
    },
    /// Function call.
    Call {
        /// Called expression.
        callee: Box<Expression>,
        /// Arguments.
        arguments: Vec<Expression>,
        /// Full expression range.
        span: Span,
    },
    /// Field access such as `user.name`.
    Field {
        /// Base expression.
        base: Box<Expression>,
        /// Accessed field.
        field: Name,
        /// Full expression range.
        span: Span,
    },
    /// Index expression such as `values[index]`.
    Index {
        /// Indexed base expression.
        base: Box<Expression>,
        /// Index value.
        index: Box<Expression>,
        /// Full expression range.
        span: Span,
    },
    /// Postfix propagation expression such as `load()?`.
    Try {
        /// Propagated operand.
        operand: Box<Expression>,
        /// Full expression range including `?`.
        span: Span,
    },
    /// Explicit scalar cast using `as`.
    Cast {
        /// Source expression.
        expression: Box<Expression>,
        /// Target type.
        target: TypeRef,
        /// Full source range.
        span: Span,
    },
    /// Array literal.
    Array {
        /// Array elements.
        elements: Vec<Expression>,
        /// Full expression range.
        span: Span,
    },
    /// Record construction such as `User { id: 1 }`.
    StructLiteral {
        /// Constructed type expression.
        ty: Box<Expression>,
        /// Field initializers.
        fields: Vec<StructFieldValue>,
        /// Full expression range.
        span: Span,
    },
    /// Parenthesized expression.
    Group {
        /// Inner expression.
        expression: Box<Expression>,
        /// Full expression range.
        span: Span,
    },
    /// Braced block expression.
    Block(Block),
    /// `if` expression.
    If {
        /// Condition.
        condition: Box<Expression>,
        /// Then block.
        then_block: Block,
        /// Optional else branch.
        else_branch: Option<Box<Expression>>,
        /// Full expression range.
        span: Span,
    },
    /// Exhaustive pattern match expression.
    Match {
        /// Matched value.
        value: Box<Expression>,
        /// Match arms.
        arms: Vec<MatchArm>,
        /// Full expression range.
        span: Span,
    },
    /// Error placeholder used for recovery.
    Error(Span),
}

impl Expression {
    /// Returns the full source range.
    #[must_use]
    pub const fn span(&self) -> Span {
        match self {
            Self::Name(name) => name.span,
            Self::Literal { span, .. }
            | Self::Unary { span, .. }
            | Self::Binary { span, .. }
            | Self::Call { span, .. }
            | Self::Field { span, .. }
            | Self::Index { span, .. }
            | Self::Try { span, .. }
            | Self::Cast { span, .. }
            | Self::Array { span, .. }
            | Self::StructLiteral { span, .. }
            | Self::Group { span, .. }
            | Self::If { span, .. }
            | Self::Match { span, .. }
            | Self::Error(span) => *span,
            Self::Block(block) => block.span,
        }
    }
}

/// Field initializer in a struct literal.
#[derive(Clone, Debug)]
pub struct StructFieldValue {
    /// Field name.
    pub name: Name,
    /// Assigned value.
    pub value: Expression,
    /// Full initializer range.
    pub span: Span,
}

/// One `match` arm.
#[derive(Clone, Debug)]
pub struct MatchArm {
    /// Arm pattern.
    pub pattern: Pattern,
    /// Optional guard after `if`.
    pub guard: Option<Expression>,
    /// Arm result expression.
    pub value: Expression,
    /// Full arm range.
    pub span: Span,
}

/// Initial pattern representation.
#[derive(Clone, Debug)]
pub enum Pattern {
    /// `_` wildcard.
    Wildcard(Span),
    /// Name or dot-separated enum path.
    Path(Path),
    /// Constructor pattern such as `Ok(value)`.
    Constructor {
        /// Constructor path.
        path: Path,
        /// Nested argument patterns.
        arguments: Vec<Pattern>,
        /// Full pattern range.
        span: Span,
    },
    /// Literal pattern.
    Literal {
        /// Literal kind.
        kind: LiteralKind,
        /// Literal range.
        span: Span,
    },
    /// Error placeholder.
    Error(Span),
}

impl Pattern {
    /// Returns the full source range.
    #[must_use]
    pub const fn span(&self) -> Span {
        match self {
            Self::Wildcard(span)
            | Self::Constructor { span, .. }
            | Self::Literal { span, .. }
            | Self::Error(span) => *span,
            Self::Path(path) => path.span,
        }
    }
}

/// Literal category preserved by the parser.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiteralKind {
    /// Integer literal.
    Integer,
    /// Floating-point literal.
    Float,
    /// String literal.
    String,
    /// Character literal.
    Char,
    /// Boolean literal.
    Bool,
}

/// Parser result with recovery diagnostics.
#[derive(Clone, Debug)]
pub struct ParseOutput {
    /// Partial or complete AST.
    pub file: AstFile,
    /// Lossless syntax tree retaining every lexer token, trivia, and error token.
    pub syntax: SyntaxTree,
    /// Syntax diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

/// Lossless syntax parser result before AST lowering.
#[derive(Clone, Debug)]
pub struct SyntaxParseOutput {
    /// Lossless concrete syntax tree.
    pub syntax: SyntaxTree,
    /// Lexer-independent syntax diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

impl SyntaxParseOutput {
    /// Returns whether syntax errors occurred.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }
}

impl ParseOutput {
    /// Returns whether syntax errors occurred.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }
}

/// Parses a lossless lexer token stream into the initial AST.
#[must_use]
pub fn parse(source: &SourceFile, tokens: &[Token]) -> ParseOutput {
    let syntax_output = parse_syntax(source, tokens);
    let lowered = lower_syntax_tree(source, &syntax_output.syntax);
    let mut diagnostics = syntax_output.diagnostics;
    diagnostics.extend(lowered.diagnostics);
    ParseOutput {
        file: lowered.file,
        syntax: syntax_output.syntax,
        diagnostics,
    }
}

/// Parses tokens into a lossless syntax tree without performing AST lowering.
#[must_use]
pub fn parse_syntax(source: &SourceFile, tokens: &[Token]) -> SyntaxParseOutput {
    SyntaxParser::new(source, tokens).parse_file()
}

struct SyntaxParser<'source> {
    source: &'source SourceFile,
    all_tokens: Vec<Token>,
    tokens: Vec<Token>,
    position: usize,
    diagnostics: Vec<Diagnostic>,
    syntax_nodes: Vec<NodeSpec>,
    recovery_spans: Vec<Span>,
}

#[derive(Clone, Copy, Debug)]
struct ParsedExpression {
    span: Span,
    can_start_struct_literal: bool,
}

impl ParsedExpression {
    const fn new(span: Span) -> Self {
        Self {
            span,
            can_start_struct_literal: false,
        }
    }

    const fn record_constructor(span: Span) -> Self {
        Self {
            span,
            can_start_struct_literal: true,
        }
    }
}

impl<'source> SyntaxParser<'source> {
    fn new(source: &'source SourceFile, tokens: &[Token]) -> Self {
        Self {
            source,
            all_tokens: tokens.to_vec(),
            tokens: tokens
                .iter()
                .copied()
                .filter(|token| !token.kind.is_trivia())
                .collect(),
            position: 0,
            diagnostics: Vec::new(),
            syntax_nodes: Vec::new(),
            recovery_spans: Vec::new(),
        }
    }

    fn parse_file(mut self) -> SyntaxParseOutput {
        if self.at_keyword(Keyword::Module) {
            let start = self.advance().span.start;
            let _ = self.parse_path();
            self.eat_punctuation(Punctuation::Semicolon);
            self.record_syntax(SyntaxKind::ModuleDeclaration, self.span_from(start));
        }
        while self.at_keyword(Keyword::Import) {
            let start = self.advance().span.start;
            let _ = self.parse_path();
            self.eat_punctuation(Punctuation::Semicolon);
            self.record_syntax(SyntaxKind::ImportDeclaration, self.span_from(start));
        }

        while !self.at(TokenKind::Eof) {
            let declaration_start = self.current_span().start;
            self.parse_annotations();
            self.eat_keyword(Keyword::Pub);
            if self.at_keyword(Keyword::Extern) {
                let _ = self.parse_extern_block(declaration_start);
            } else if self.at_keyword(Keyword::Fn) {
                let _ = self.parse_function(declaration_start);
            } else if self.at_keyword(Keyword::Struct) {
                let _ = self.parse_record(declaration_start, Keyword::Struct);
            } else if self.at_keyword(Keyword::Component) {
                let _ = self.parse_record(declaration_start, Keyword::Component);
            } else if self.at_keyword(Keyword::Enum) {
                let _ = self.parse_enum(declaration_start);
            } else {
                self.error_current(
                    "J0100",
                    "expected a top-level declaration",
                    "expected `fn`, `struct`, `component`, or `enum`",
                );
                self.recover_top_level();
            }
        }

        let mut node_specs = self.syntax_nodes.clone();
        node_specs.extend(
            self.recovery_spans
                .iter()
                .copied()
                .map(|span| NodeSpec::new(SyntaxKind::Error, span)),
        );
        let syntax = match SyntaxTree::build(
            self.source.id(),
            self.source.text().len(),
            &self.all_tokens,
            node_specs,
        ) {
            Ok(syntax) => syntax,
            Err(error) => {
                self.diagnostics.push(
                    Diagnostic::error(
                        "J0199",
                        "failed to build the lossless syntax tree",
                        Span::empty(self.source.id(), 0),
                        "internal syntax tree invariant failed",
                    )
                    .with_note(error.to_string()),
                );
                SyntaxTree::flat(self.source.id(), self.source.text().len(), &self.all_tokens)
            }
        };

        SyntaxParseOutput {
            syntax,
            diagnostics: self.diagnostics,
        }
    }

    fn parse_function(&mut self, start: usize) -> Option<()> {
        self.expect_keyword(Keyword::Fn)?;
        self.parse_name("expected a function name")?;
        self.parse_generic_parameters();
        let parameter_list_start = self
            .expect_punctuation(Punctuation::LeftParen, "expected `(` after function name")
            .map(|token| token.span.start);
        while !self.at_punctuation(Punctuation::RightParen) && !self.at(TokenKind::Eof) {
            let parameter_start = self.current_span().start;
            let Some(_) = self.parse_name("expected a parameter name") else {
                self.recover_until(&[
                    TokenKind::Punctuation(Punctuation::Comma),
                    TokenKind::Punctuation(Punctuation::RightParen),
                ]);
                if self.eat_punctuation(Punctuation::Comma) {
                    continue;
                }
                break;
            };
            self.expect_punctuation(Punctuation::Colon, "expected `:` after parameter name");
            if let Some(ty) = self.parse_type() {
                let _ = ty;
                self.record_syntax(SyntaxKind::Parameter, self.span_from(parameter_start));
            }
            if !self.eat_punctuation(Punctuation::Comma) {
                break;
            }
        }
        let parameter_list_end = self
            .expect_punctuation(Punctuation::RightParen, "expected `)` after parameters")
            .map_or(self.previous_end(), |token| token.span.end);
        if let Some(parameter_list_start) = parameter_list_start {
            self.record_syntax(
                SyntaxKind::ParameterList,
                self.join_offsets(parameter_list_start, parameter_list_end),
            );
        }

        if self.eat_operator(Operator::Arrow) {
            let _ = self.parse_type();
        }
        let body = self.parse_block(SyntaxKind::Block)?;
        let span = self.join_offsets(start, body.end);
        self.record_syntax(SyntaxKind::FunctionDeclaration, span);
        Some(())
    }

    fn parse_extern_block(&mut self, start: usize) -> Option<()> {
        self.expect_keyword(Keyword::Extern)?;
        let _abi = if self.current().kind == TokenKind::StringLiteral {
            self.source
                .slice(self.advance().span)
                .unwrap_or_default()
                .trim_matches('"')
                .to_owned()
        } else {
            self.error_current(
                "J0110",
                "expected extern ABI string",
                "write `extern \"C\" { ... }`",
            );
            String::new()
        };
        self.expect_punctuation(Punctuation::LeftBrace, "expected `{` after extern ABI")?;
        while !self.at_punctuation(Punctuation::RightBrace) && !self.at(TokenKind::Eof) {
            let function_start = self.current_span().start;
            let _is_unsafe = self.eat_keyword(Keyword::Unsafe);
            if !self.expect_keyword(Keyword::Fn).is_some() {
                self.recover_until(&[
                    TokenKind::Punctuation(Punctuation::Semicolon),
                    TokenKind::Punctuation(Punctuation::RightBrace),
                ]);
                self.eat_punctuation(Punctuation::Semicolon);
                continue;
            }
            self.parse_name("expected an extern function name")?;
            self.parse_generic_parameters();
            let parameter_list_start = self
                .expect_punctuation(Punctuation::LeftParen, "expected `(` after extern function")
                .map(|token| token.span.start);
            while !self.at_punctuation(Punctuation::RightParen) && !self.at(TokenKind::Eof) {
                let parameter_start = self.current_span().start;
                let Some(_) = self.parse_name("expected an extern parameter name") else {
                    self.recover_until(&[
                        TokenKind::Punctuation(Punctuation::Comma),
                        TokenKind::Punctuation(Punctuation::RightParen),
                    ]);
                    if self.eat_punctuation(Punctuation::Comma) {
                        continue;
                    }
                    break;
                };
                self.expect_punctuation(Punctuation::Colon, "expected `:` after extern parameter");
                let _ = self.parse_type();
                self.record_syntax(SyntaxKind::Parameter, self.span_from(parameter_start));
                if !self.eat_punctuation(Punctuation::Comma) {
                    break;
                }
            }
            let parameter_list_end = self
                .expect_punctuation(
                    Punctuation::RightParen,
                    "expected `)` after extern parameters",
                )
                .map_or(self.previous_end(), |token| token.span.end);
            if let Some(parameter_list_start) = parameter_list_start {
                self.record_syntax(
                    SyntaxKind::ParameterList,
                    self.join_offsets(parameter_list_start, parameter_list_end),
                );
            }
            if self.eat_operator(Operator::Arrow) {
                let _ = self.parse_type();
            }
            self.expect_punctuation(
                Punctuation::Semicolon,
                "expected `;` after extern function declaration",
            );
            self.record_syntax(
                SyntaxKind::ExternFunctionDeclaration,
                self.span_from(function_start),
            );
        }
        let end = self
            .expect_punctuation(Punctuation::RightBrace, "expected `}` after extern block")
            .map_or(self.previous_end(), |token| token.span.end);
        self.record_syntax(SyntaxKind::ExternBlock, self.join_offsets(start, end));
        Some(())
    }

    fn parse_annotations(&mut self) {
        while self.at_punctuation(Punctuation::At) {
            let start = self.advance().span.start;
            let Some(_) = self.parse_path() else {
                self.recover_top_level();
                break;
            };
            if self.eat_punctuation(Punctuation::LeftParen) {
                while !self.at_punctuation(Punctuation::RightParen) && !self.at(TokenKind::Eof) {
                    if self.current().kind == TokenKind::Identifier
                        && self.nth_kind(1) == TokenKind::Punctuation(Punctuation::Colon)
                    {
                        let _ = self.parse_name("expected an annotation argument name");
                        self.advance();
                    }
                    let _ = self.parse_expression(0);
                    if !self.eat_punctuation(Punctuation::Comma) {
                        break;
                    }
                }
                self.expect_punctuation(
                    Punctuation::RightParen,
                    "expected `)` after annotation arguments",
                );
            }
            self.record_syntax(SyntaxKind::Annotation, self.span_from(start));
        }
    }

    fn parse_generic_parameters(&mut self) {
        if !self.at_operator(Operator::Less) {
            return;
        }
        let list_start = self.advance().span.start;
        while !self.at_operator(Operator::Greater) && !self.at(TokenKind::Eof) {
            let start = self.current_span().start;
            let Some(_) = self.parse_name("expected a generic parameter name") else {
                self.recover_until(&[
                    TokenKind::Punctuation(Punctuation::Comma),
                    TokenKind::Operator(Operator::Greater),
                ]);
                if self.eat_punctuation(Punctuation::Comma) {
                    continue;
                }
                break;
            };
            if self.eat_punctuation(Punctuation::Colon) {
                loop {
                    let _ = self.parse_type();
                    if !self.eat_operator(Operator::Plus) {
                        break;
                    }
                }
            }
            self.record_syntax(SyntaxKind::GenericParameter, self.span_from(start));
            if !self.eat_punctuation(Punctuation::Comma) {
                break;
            }
        }
        let list_end = self
            .expect_operator(Operator::Greater, "expected `>` after generic parameters")
            .map_or(self.previous_end(), |token| token.span.end);
        self.record_syntax(
            SyntaxKind::GenericParameterList,
            self.join_offsets(list_start, list_end),
        );
    }

    fn parse_record(&mut self, start: usize, keyword: Keyword) -> Option<()> {
        self.expect_keyword(keyword)?;
        self.parse_name("expected a type name")?;
        self.parse_generic_parameters();
        self.expect_punctuation(Punctuation::LeftBrace, "expected `{` before fields")?;
        while !self.at_punctuation(Punctuation::RightBrace) && !self.at(TokenKind::Eof) {
            if self.at_embedded_top_level_start() {
                break;
            }
            let field_start = self.current_span().start;
            self.eat_keyword(Keyword::Pub);
            let Some(_) = self.parse_name("expected a field name") else {
                self.recover_until(&[
                    TokenKind::Punctuation(Punctuation::Comma),
                    TokenKind::Punctuation(Punctuation::RightBrace),
                ]);
                self.eat_punctuation(Punctuation::Comma);
                continue;
            };
            self.expect_punctuation(Punctuation::Colon, "expected `:` after field name");
            if let Some(ty) = self.parse_type() {
                let _ = ty;
                self.record_syntax(SyntaxKind::FieldDeclaration, self.span_from(field_start));
            }
            self.eat_punctuation(Punctuation::Comma);
        }
        let closing = self.expect_punctuation(Punctuation::RightBrace, "expected `}` after fields");
        let end = closing.map_or(self.previous_end(), |token| token.span.end);
        let kind = if keyword == Keyword::Struct {
            SyntaxKind::StructDeclaration
        } else {
            SyntaxKind::ComponentDeclaration
        };
        self.record_syntax(kind, self.join_offsets(start, end));
        Some(())
    }

    fn parse_enum(&mut self, start: usize) -> Option<()> {
        self.expect_keyword(Keyword::Enum)?;
        self.parse_name("expected an enum name")?;
        self.parse_generic_parameters();
        self.expect_punctuation(Punctuation::LeftBrace, "expected `{` before variants")?;
        while !self.at_punctuation(Punctuation::RightBrace) && !self.at(TokenKind::Eof) {
            if self.at_embedded_top_level_start() {
                break;
            }
            let variant_start = self.current_span().start;
            let Some(_) = self.parse_name("expected a variant name") else {
                self.recover_until(&[
                    TokenKind::Punctuation(Punctuation::Comma),
                    TokenKind::Punctuation(Punctuation::RightBrace),
                ]);
                self.eat_punctuation(Punctuation::Comma);
                continue;
            };
            if self.eat_punctuation(Punctuation::LeftParen) {
                while !self.at_punctuation(Punctuation::RightParen) && !self.at(TokenKind::Eof) {
                    let field_start = self.current_span().start;
                    let is_named = if self.current().kind == TokenKind::Identifier
                        && self.nth_kind(1) == TokenKind::Punctuation(Punctuation::Colon)
                    {
                        let _ = self.parse_name("expected a payload field name");
                        self.advance();
                        true
                    } else {
                        false
                    };
                    if let Some(ty) = self.parse_type() {
                        let _ = ty;
                        if is_named {
                            self.record_syntax(
                                SyntaxKind::EnumVariantField,
                                self.span_from(field_start),
                            );
                        }
                    }
                    if !self.eat_punctuation(Punctuation::Comma) {
                        break;
                    }
                }
                self.expect_punctuation(
                    Punctuation::RightParen,
                    "expected `)` after variant payload",
                );
            }
            self.record_syntax(SyntaxKind::EnumVariant, self.span_from(variant_start));
            self.eat_punctuation(Punctuation::Comma);
        }
        let closing =
            self.expect_punctuation(Punctuation::RightBrace, "expected `}` after variants");
        let end = closing.map_or(self.previous_end(), |token| token.span.end);
        self.record_syntax(SyntaxKind::EnumDeclaration, self.join_offsets(start, end));
        Some(())
    }

    fn parse_type(&mut self) -> Option<Span> {
        if self.at_keyword(Keyword::Fn) {
            let start = self.advance().span.start;
            self.expect_punctuation(Punctuation::LeftParen, "expected `(` after `fn`");
            while !self.at_punctuation(Punctuation::RightParen) && !self.at(TokenKind::Eof) {
                self.parse_type()?;
                if !self.eat_punctuation(Punctuation::Comma) {
                    break;
                }
            }
            self.expect_punctuation(Punctuation::RightParen, "expected `)` after function type");
            if self.eat_operator(Operator::Arrow) {
                self.parse_type()?;
            }
            let span = self.span_from(start);
            self.record_syntax(SyntaxKind::FunctionType, span);
            return Some(span);
        }
        if self.at_keyword(Keyword::Owned)
            || self.at_keyword(Keyword::Read)
            || self.at_keyword(Keyword::Write)
        {
            let start = self.advance().span.start;
            let inner = self.parse_type()?;
            let span = self.join_offsets(start, inner.end);
            self.record_syntax(SyntaxKind::CapabilityType, span);
            return Some(span);
        }
        if self.at_punctuation(Punctuation::LeftBracket) {
            let start = self.advance().span.start;
            self.parse_type()?;
            self.expect_punctuation(Punctuation::Semicolon, "expected `;` in array type");
            while !self.at_punctuation(Punctuation::RightBracket) && !self.at(TokenKind::Eof) {
                self.advance();
            }
            let end = self
                .expect_punctuation(Punctuation::RightBracket, "expected `]` after array type")
                .map_or(self.previous_end(), |token| token.span.end);
            let span = self.join_offsets(start, end);
            self.record_syntax(SyntaxKind::ArrayType, span);
            return Some(span);
        }

        let path = self.parse_path()?;
        let start = path.start;
        if self.eat_operator(Operator::Less) {
            while !self.at_operator(Operator::Greater) && !self.at(TokenKind::Eof) {
                let _ = self.parse_type();
                if !self.eat_punctuation(Punctuation::Comma) {
                    break;
                }
            }
            self.expect_operator(Operator::Greater, "expected `>` after generic arguments");
        }
        let span = self.span_from(start);
        self.record_syntax(SyntaxKind::PathType, span);
        Some(span)
    }

    fn parse_path(&mut self) -> Option<Span> {
        let first = self.parse_name("expected a path segment")?;
        let start = first.start;
        while self.eat_punctuation(Punctuation::Dot) || self.eat_operator(Operator::PathSeparator) {
            let Some(_) = self.parse_name("expected a path segment after separator") else {
                break;
            };
        }
        Some(self.span_from(start))
    }

    fn parse_block(&mut self, syntax_kind: SyntaxKind) -> Option<Span> {
        let start = self
            .expect_punctuation(Punctuation::LeftBrace, "expected a block starting with `{`")?
            .span
            .start;
        while !self.at_punctuation(Punctuation::RightBrace) && !self.at(TokenKind::Eof) {
            if self.at_top_level_start() {
                break;
            }
            let previous = self.position;
            self.parse_statement();
            if self.position == previous {
                self.advance();
            }
        }
        let closing = self.expect_punctuation(
            Punctuation::RightBrace,
            "expected `}` before the end of the file",
        );
        let end = closing.map_or_else(|| self.current_span().start, |token| token.span.end);
        let span = self.join_offsets(start, end);
        self.record_syntax(syntax_kind, span);
        Some(span)
    }

    fn parse_statement(&mut self) {
        if self.at_keyword(Keyword::Let) || self.at_keyword(Keyword::Var) {
            self.parse_binding();
            return;
        }
        if self.at_keyword(Keyword::Return) {
            self.parse_return();
            return;
        }
        if self.at_keyword(Keyword::Region) {
            self.parse_region();
            return;
        }
        if self.at_keyword(Keyword::While) {
            self.parse_while();
            return;
        }
        if self.at_keyword(Keyword::For) {
            self.parse_for();
            return;
        }
        if self.at_keyword(Keyword::Break) {
            self.parse_break();
            return;
        }
        if self.at_keyword(Keyword::Continue) {
            self.parse_continue();
            return;
        }

        let _ = self.parse_expression(0);
        self.eat_punctuation(Punctuation::Semicolon);
    }

    fn parse_binding(&mut self) {
        let keyword = self.advance();
        let start = keyword.span.start;
        let _ = self.parse_name("expected a binding name");
        if self.eat_punctuation(Punctuation::Colon) {
            let _ = self.parse_type();
        }
        if self.eat_operator(Operator::Assign) {
            let _ = self.parse_expression(0);
        }
        self.eat_punctuation(Punctuation::Semicolon);
        self.record_syntax(SyntaxKind::BindingStatement, self.span_from(start));
    }

    fn parse_return(&mut self) {
        let start = self.advance().span.start;
        if !self.at_punctuation(Punctuation::Semicolon)
            && !self.at_punctuation(Punctuation::RightBrace)
        {
            let _ = self.parse_expression(0);
        }
        self.eat_punctuation(Punctuation::Semicolon);
        self.record_syntax(SyntaxKind::ReturnStatement, self.span_from(start));
    }

    fn parse_region(&mut self) {
        let start = self.advance().span.start;
        let _ = self.parse_name("expected a region name");
        let body = self.parse_block(SyntaxKind::Block);
        let end = body.map_or(self.previous_end(), |span| span.end);
        self.record_syntax(SyntaxKind::RegionStatement, self.join_offsets(start, end));
    }

    fn parse_while(&mut self) {
        let start = self.advance().span.start;
        let _ = self.parse_expression_with(0, false);
        let body = self.parse_block(SyntaxKind::Block);
        let end = body.map_or(self.previous_end(), |span| span.end);
        self.record_syntax(SyntaxKind::WhileStatement, self.join_offsets(start, end));
    }

    fn parse_for(&mut self) {
        let start = self.advance().span.start;
        let _ = self.parse_name("expected a `for` binding name");
        let _ = self.expect_keyword(Keyword::In);
        let _ = self.parse_expression_with(0, false);
        let body = self.parse_block(SyntaxKind::Block);
        let end = body.map_or(self.previous_end(), |span| span.end);
        self.record_syntax(SyntaxKind::ForStatement, self.join_offsets(start, end));
    }

    fn parse_break(&mut self) {
        let start = self.advance().span.start;
        self.eat_punctuation(Punctuation::Semicolon);
        self.record_syntax(SyntaxKind::BreakStatement, self.span_from(start));
    }

    fn parse_continue(&mut self) {
        let start = self.advance().span.start;
        self.eat_punctuation(Punctuation::Semicolon);
        self.record_syntax(SyntaxKind::ContinueStatement, self.span_from(start));
    }

    fn parse_cast(&mut self, expression: ParsedExpression) -> ParsedExpression {
        let _ = self.expect_keyword(Keyword::As);
        let _ = self.parse_type();
        let span = self.join_offsets(expression.span.start, self.previous_end());
        self.record_syntax(SyntaxKind::CastExpression, span);
        ParsedExpression::new(span)
    }

    fn parse_expression(&mut self, minimum_precedence: u8) -> ParsedExpression {
        self.parse_expression_with(minimum_precedence, true)
    }

    fn parse_expression_with(
        &mut self,
        minimum_precedence: u8,
        allow_struct_literal: bool,
    ) -> ParsedExpression {
        let mut left = self.parse_prefix(allow_struct_literal);
        loop {
            if self.at_punctuation(Punctuation::LeftParen) {
                left = self.parse_call(left);
                continue;
            }
            if self.at_punctuation(Punctuation::Dot) {
                left = self.parse_field_access(left);
                continue;
            }
            if self.at_punctuation(Punctuation::LeftBracket) {
                left = self.parse_index(left);
                continue;
            }
            if self.current_operator() == Some(Operator::Question) {
                left = self.parse_try(left);
                continue;
            }
            if self.at_keyword(Keyword::As) {
                left = self.parse_cast(left);
                continue;
            }
            if allow_struct_literal
                && self.at_punctuation(Punctuation::LeftBrace)
                && left.can_start_struct_literal
            {
                left = self.parse_struct_literal(left);
                continue;
            }
            let Some(operator) = self.current_operator() else {
                break;
            };
            let Some((precedence, right_associative)) = infix_precedence(operator) else {
                break;
            };
            if precedence < minimum_precedence {
                break;
            }
            self.advance();
            let next_minimum = if right_associative {
                precedence
            } else {
                precedence + 1
            };
            let right = self.parse_expression_with(next_minimum, allow_struct_literal);
            let span = self.join_spans(left.span, right.span);
            self.record_syntax(SyntaxKind::BinaryExpression, span);
            left = ParsedExpression::new(span);
        }
        left
    }

    fn parse_prefix(&mut self, allow_struct_literal: bool) -> ParsedExpression {
        let token = self.current();
        match token.kind {
            TokenKind::Identifier => {
                self.advance();
                self.record_syntax(SyntaxKind::NameExpression, token.span);
                ParsedExpression::record_constructor(token.span)
            }
            TokenKind::IntegerLiteral
            | TokenKind::FloatLiteral
            | TokenKind::StringLiteral
            | TokenKind::CharLiteral
            | TokenKind::Keyword(Keyword::True | Keyword::False) => self.parse_literal(),
            TokenKind::Operator(
                operator @ (Operator::Bang | Operator::Minus | Operator::Plus | Operator::Tilde),
            ) => {
                self.advance();
                let operand = self.parse_expression_with(11, allow_struct_literal);
                let _ = operator;
                let span = self.join_spans(token.span, operand.span);
                self.record_syntax(SyntaxKind::UnaryExpression, span);
                ParsedExpression::new(span)
            }
            TokenKind::Punctuation(Punctuation::LeftParen) => self.parse_group(),
            TokenKind::Punctuation(Punctuation::LeftBracket) => self.parse_array(),
            TokenKind::Punctuation(Punctuation::LeftBrace) => self
                .parse_block(SyntaxKind::BlockExpression)
                .map_or_else(|| ParsedExpression::new(token.span), ParsedExpression::new),
            TokenKind::Keyword(Keyword::If) => self.parse_if(),
            TokenKind::Keyword(Keyword::Match) => self.parse_match(),
            _ => {
                self.error_current("J0101", "expected an expression", "not an expression");
                if !self.at(TokenKind::Eof) {
                    self.advance();
                }
                self.record_syntax(SyntaxKind::Error, token.span);
                ParsedExpression::new(token.span)
            }
        }
    }

    fn parse_literal(&mut self) -> ParsedExpression {
        let span = self.advance().span;
        self.record_syntax(SyntaxKind::LiteralExpression, span);
        ParsedExpression::new(span)
    }

    fn parse_group(&mut self) -> ParsedExpression {
        let start = self.advance().span.start;
        let expression = self.parse_expression(0);
        let end = self
            .expect_punctuation(Punctuation::RightParen, "expected `)` after expression")
            .map_or(expression.span.end, |token| token.span.end);
        let span = self.join_offsets(start, end);
        self.record_syntax(SyntaxKind::GroupExpression, span);
        ParsedExpression::new(span)
    }

    fn parse_call(&mut self, callee: ParsedExpression) -> ParsedExpression {
        let start = callee.span.start;
        self.advance();
        let mut arguments = Vec::new();
        while !self.at_punctuation(Punctuation::RightParen) && !self.at(TokenKind::Eof) {
            arguments.push(self.parse_expression(0));
            if !self.eat_punctuation(Punctuation::Comma) {
                break;
            }
        }
        let end = self
            .expect_punctuation(Punctuation::RightParen, "expected `)` after arguments")
            .map_or_else(|| self.current_span().start, |token| token.span.end);
        let _ = arguments;
        let span = self.join_offsets(start, end);
        self.record_syntax(SyntaxKind::CallExpression, span);
        ParsedExpression::new(span)
    }

    fn parse_field_access(&mut self, base: ParsedExpression) -> ParsedExpression {
        let start = base.span.start;
        self.advance();
        let Some(field) = self.parse_name("expected a field name after `.`") else {
            let span = self.span_from(start);
            self.record_syntax(SyntaxKind::Error, span);
            return ParsedExpression::new(span);
        };
        let span = self.join_offsets(start, field.end);
        self.record_syntax(SyntaxKind::FieldExpression, span);
        ParsedExpression::record_constructor(span)
    }

    fn parse_index(&mut self, base: ParsedExpression) -> ParsedExpression {
        let start = base.span.start;
        self.advance();
        let index = self.parse_expression(0);
        let end = self
            .expect_punctuation(Punctuation::RightBracket, "expected `]` after index")
            .map_or(index.span.end, |token| token.span.end);
        let span = self.join_offsets(start, end);
        self.record_syntax(SyntaxKind::IndexExpression, span);
        ParsedExpression::new(span)
    }

    fn parse_try(&mut self, operand: ParsedExpression) -> ParsedExpression {
        let question = self.advance();
        let span = self.join_offsets(operand.span.start, question.span.end);
        self.record_syntax(SyntaxKind::TryExpression, span);
        ParsedExpression::new(span)
    }

    fn parse_array(&mut self) -> ParsedExpression {
        let start = self.advance().span.start;
        while !self.at_punctuation(Punctuation::RightBracket) && !self.at(TokenKind::Eof) {
            let _ = self.parse_expression(0);
            if !self.eat_punctuation(Punctuation::Comma) {
                break;
            }
        }
        let end = self
            .expect_punctuation(
                Punctuation::RightBracket,
                "expected `]` after array elements",
            )
            .map_or(self.previous_end(), |token| token.span.end);
        let span = self.join_offsets(start, end);
        self.record_syntax(SyntaxKind::ArrayExpression, span);
        ParsedExpression::new(span)
    }

    fn parse_struct_literal(&mut self, ty: ParsedExpression) -> ParsedExpression {
        let start = ty.span.start;
        self.advance();
        while !self.at_punctuation(Punctuation::RightBrace) && !self.at(TokenKind::Eof) {
            let field_start = self.current_span().start;
            let Some(_) = self.parse_name("expected a struct literal field name") else {
                self.recover_until(&[
                    TokenKind::Punctuation(Punctuation::Comma),
                    TokenKind::Punctuation(Punctuation::RightBrace),
                ]);
                self.eat_punctuation(Punctuation::Comma);
                continue;
            };
            self.expect_punctuation(Punctuation::Colon, "expected `:` after field name");
            let _ = self.parse_expression(0);
            self.record_syntax(
                SyntaxKind::StructFieldInitializer,
                self.span_from(field_start),
            );
            if !self.eat_punctuation(Punctuation::Comma) {
                break;
            }
        }
        let end = self
            .expect_punctuation(Punctuation::RightBrace, "expected `}` after struct literal")
            .map_or(self.previous_end(), |token| token.span.end);
        let span = self.join_offsets(start, end);
        self.record_syntax(SyntaxKind::StructExpression, span);
        ParsedExpression::new(span)
    }

    fn parse_if(&mut self) -> ParsedExpression {
        let start = self.advance().span.start;
        let _ = self.parse_expression_with(0, false);
        let then_block = self
            .parse_block(SyntaxKind::Block)
            .unwrap_or(self.current_span());
        let else_branch = if self.eat_keyword(Keyword::Else) {
            if self.at_keyword(Keyword::If) {
                Some(self.parse_if())
            } else {
                self.parse_block(SyntaxKind::BlockExpression)
                    .map(ParsedExpression::new)
            }
        } else {
            None
        };
        let end = else_branch
            .as_ref()
            .map_or(then_block.end, |branch| branch.span.end);
        let span = self.join_offsets(start, end);
        self.record_syntax(SyntaxKind::IfExpression, span);
        ParsedExpression::new(span)
    }

    fn parse_match(&mut self) -> ParsedExpression {
        let start = self.advance().span.start;
        let _ = self.parse_expression_with(0, false);
        self.expect_punctuation(Punctuation::LeftBrace, "expected `{` after match value");
        while !self.at_punctuation(Punctuation::RightBrace) && !self.at(TokenKind::Eof) {
            if self.at_match_recovery_boundary() {
                break;
            }
            let arm_start = self.current_span().start;
            let _ = self.parse_pattern();
            if self.eat_keyword(Keyword::If) {
                let _ = self.parse_expression_with(0, false);
            }
            self.expect_operator(Operator::FatArrow, "expected `=>` after match pattern");
            let _ = self.parse_expression(0);
            self.record_syntax(SyntaxKind::MatchArm, self.span_from(arm_start));
            self.eat_punctuation(Punctuation::Comma);
        }
        let end = self
            .expect_punctuation(Punctuation::RightBrace, "expected `}` after match arms")
            .map_or(self.previous_end(), |token| token.span.end);
        let span = self.join_offsets(start, end);
        self.record_syntax(SyntaxKind::MatchExpression, span);
        ParsedExpression::new(span)
    }

    fn parse_pattern(&mut self) -> Span {
        let token = self.current();
        match token.kind {
            TokenKind::Identifier => {
                let Some(path) = self.parse_path() else {
                    self.record_syntax(SyntaxKind::Error, token.span);
                    return token.span;
                };
                if self.text(path) == "_" {
                    self.record_syntax(SyntaxKind::WildcardPattern, path);
                    return path;
                }
                if self.eat_punctuation(Punctuation::LeftParen) {
                    let start = path.start;
                    while !self.at_punctuation(Punctuation::RightParen) && !self.at(TokenKind::Eof)
                    {
                        let _ = self.parse_pattern();
                        if !self.eat_punctuation(Punctuation::Comma) {
                            break;
                        }
                    }
                    let end = self
                        .expect_punctuation(
                            Punctuation::RightParen,
                            "expected `)` after pattern arguments",
                        )
                        .map_or(self.previous_end(), |token| token.span.end);
                    let span = self.join_offsets(start, end);
                    self.record_syntax(SyntaxKind::ConstructorPattern, span);
                    span
                } else {
                    self.record_syntax(SyntaxKind::PathPattern, path);
                    path
                }
            }
            TokenKind::IntegerLiteral => {
                self.advance();
                self.record_syntax(SyntaxKind::LiteralPattern, token.span);
                token.span
            }
            TokenKind::StringLiteral => {
                self.advance();
                self.record_syntax(SyntaxKind::LiteralPattern, token.span);
                token.span
            }
            TokenKind::Keyword(Keyword::True | Keyword::False) => {
                self.advance();
                self.record_syntax(SyntaxKind::LiteralPattern, token.span);
                token.span
            }
            _ => {
                self.error_current("J0105", "expected a match pattern", "not a pattern");
                if !self.at(TokenKind::Eof) {
                    self.advance();
                }
                self.record_syntax(SyntaxKind::Error, token.span);
                token.span
            }
        }
    }

    fn parse_name(&mut self, message: &str) -> Option<Span> {
        let token = self.current();
        if token.kind == TokenKind::Identifier {
            self.advance();
            Some(token.span)
        } else {
            self.error_current("J0102", message, "expected an identifier");
            None
        }
    }

    fn recover_top_level(&mut self) {
        let start = self.position;
        while !self.at(TokenKind::Eof)
            && !self.at_punctuation(Punctuation::At)
            && !self.at_keyword(Keyword::Fn)
            && !self.at_keyword(Keyword::Struct)
            && !self.at_keyword(Keyword::Component)
            && !self.at_keyword(Keyword::Enum)
            && !self.at_keyword(Keyword::Pub)
        {
            self.advance();
        }
        self.record_recovery(start);
    }

    fn record_syntax(&mut self, kind: SyntaxKind, span: Span) {
        if !span.is_empty() {
            self.syntax_nodes.push(NodeSpec::new(kind, span));
        }
    }

    fn at_top_level_start(&self) -> bool {
        self.at_punctuation(Punctuation::At)
            || self.at_keyword(Keyword::Pub)
            || self.at_declaration_keyword()
    }

    fn at_embedded_top_level_start(&self) -> bool {
        self.at_punctuation(Punctuation::At)
            || self.at_declaration_keyword()
            || (self.at_keyword(Keyword::Pub) && Self::is_declaration_kind(self.nth_kind(1)))
    }

    fn at_match_recovery_boundary(&self) -> bool {
        self.at_top_level_start()
            || self.at_keyword(Keyword::Let)
            || self.at_keyword(Keyword::Var)
            || self.at_keyword(Keyword::Return)
            || self.at_keyword(Keyword::If)
            || self.at_keyword(Keyword::Match)
    }

    fn at_declaration_keyword(&self) -> bool {
        Self::is_declaration_kind(self.current().kind)
    }

    const fn is_declaration_kind(kind: TokenKind) -> bool {
        matches!(
            kind,
            TokenKind::Keyword(Keyword::Fn | Keyword::Struct | Keyword::Component | Keyword::Enum)
        )
    }

    fn recover_until(&mut self, kinds: &[TokenKind]) {
        let start = self.position;
        while !self.at(TokenKind::Eof) && !kinds.contains(&self.current().kind) {
            self.advance();
        }
        self.record_recovery(start);
    }

    fn record_recovery(&mut self, start: usize) {
        if self.position <= start {
            return;
        }
        let Some(first) = self.tokens.get(start) else {
            return;
        };
        let end = self
            .tokens
            .get(self.position - 1)
            .map_or(first.span.end, |token| token.span.end);
        if let Some(span) = Span::new(self.source.id(), first.span.start, end)
            && !span.is_empty()
        {
            self.recovery_spans.push(span);
        }
    }

    fn error_current(&mut self, code: &'static str, message: &str, label: &str) {
        self.diagnostics
            .push(Diagnostic::error(code, message, self.current_span(), label));
    }

    fn expect_keyword(&mut self, keyword: Keyword) -> Option<Token> {
        if self.at_keyword(keyword) {
            Some(self.advance())
        } else {
            let spelling = keyword.as_str();
            self.diagnostics.push(
                Diagnostic::error(
                    "J0103",
                    format!("expected `{spelling}`"),
                    self.current_span(),
                    "keyword is missing",
                )
                .with_help(format!("insert `{spelling}`")),
            );
            None
        }
    }

    fn expect_punctuation(&mut self, punctuation: Punctuation, message: &str) -> Option<Token> {
        if self.at_punctuation(punctuation) {
            Some(self.advance())
        } else {
            self.error_current("J0104", message, "required punctuation is missing");
            None
        }
    }

    fn expect_operator(&mut self, operator: Operator, message: &str) -> Option<Token> {
        if self.at_operator(operator) {
            Some(self.advance())
        } else {
            self.error_current("J0106", message, "required operator is missing");
            None
        }
    }

    fn eat_keyword(&mut self, keyword: Keyword) -> bool {
        if self.at_keyword(keyword) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn eat_punctuation(&mut self, punctuation: Punctuation) -> bool {
        if self.at_punctuation(punctuation) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn eat_operator(&mut self, operator: Operator) -> bool {
        if self.current_operator() == Some(operator) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn at(&self, kind: TokenKind) -> bool {
        self.current().kind == kind
    }

    fn at_keyword(&self, keyword: Keyword) -> bool {
        self.at(TokenKind::Keyword(keyword))
    }

    fn at_punctuation(&self, punctuation: Punctuation) -> bool {
        self.at(TokenKind::Punctuation(punctuation))
    }

    fn at_operator(&self, operator: Operator) -> bool {
        self.at(TokenKind::Operator(operator))
    }

    fn current_operator(&self) -> Option<Operator> {
        match self.current().kind {
            TokenKind::Operator(operator) => Some(operator),
            _ => None,
        }
    }

    fn current(&self) -> Token {
        self.tokens.get(self.position).copied().unwrap_or(Token {
            kind: TokenKind::Eof,
            span: Span::empty(self.source.id(), self.source.text().len()),
        })
    }

    fn nth_kind(&self, offset: usize) -> TokenKind {
        self.tokens
            .get(self.position.saturating_add(offset))
            .map_or(TokenKind::Eof, |token| token.kind)
    }

    fn current_span(&self) -> Span {
        self.current().span
    }

    fn advance(&mut self) -> Token {
        let token = self.current();
        if token.kind != TokenKind::Eof {
            self.position += 1;
        }
        token
    }

    fn text(&self, span: Span) -> &str {
        self.source.slice(span).unwrap_or_default()
    }

    fn span_from(&self, start: usize) -> Span {
        self.join_offsets(start, self.previous_end())
    }

    fn previous_end(&self) -> usize {
        self.position
            .checked_sub(1)
            .and_then(|index| self.tokens.get(index))
            .map_or(self.current_span().start, |token| token.span.end)
    }

    fn join_spans(&self, left: Span, right: Span) -> Span {
        self.join_offsets(left.start, right.end)
    }

    fn join_offsets(&self, start: usize, end: usize) -> Span {
        Span::new(self.source.id(), start.min(end), end.max(start))
            .unwrap_or_else(|| Span::empty(self.source.id(), start))
    }
}

fn infix_precedence(operator: Operator) -> Option<(u8, bool)> {
    Some(match operator {
        Operator::Assign
        | Operator::PlusAssign
        | Operator::MinusAssign
        | Operator::StarAssign
        | Operator::SlashAssign
        | Operator::PercentAssign => (1, true),
        Operator::Or => (2, false),
        Operator::And => (3, false),
        Operator::Equal | Operator::NotEqual => (4, false),
        Operator::Less | Operator::LessEqual | Operator::Greater | Operator::GreaterEqual => {
            (5, false)
        }
        Operator::Range | Operator::RangeExclusive => (6, false),
        Operator::Plus | Operator::Minus => (7, false),
        Operator::Star | Operator::Slash | Operator::Percent => (8, false),
        Operator::Ampersand => (9, false),
        Operator::Caret | Operator::Pipe => (10, false),
        Operator::Bang
        | Operator::Tilde
        | Operator::Question
        | Operator::Arrow
        | Operator::FatArrow
        | Operator::PathSeparator => return None,
    })
}

#[cfg(test)]
mod tests {
    use jadren_lexer::lex;
    use jadren_source::SourceManager;

    use super::{
        Expression, Function, Item, Statement, TypeCapability, TypeRef, lower_syntax_tree, parse,
        parse_syntax,
    };

    fn function(item: &Item) -> &Function {
        match item {
            Item::Function(function) => function,
            Item::Struct(_) | Item::Component(_) | Item::Enum(_) | Item::ExternBlock(_) => {
                panic!("expected a function item")
            }
        }
    }

    fn parse_text(text: &str) -> super::ParseOutput {
        let mut sources = SourceManager::new();
        let id = sources.add("test.jdn", text).expect("source ID should fit");
        let source = sources.get(id).expect("source should exist");
        let tokens = lex(source);
        assert!(!tokens.has_errors(), "test input must lex cleanly");
        parse(source, &tokens.tokens)
    }

    #[test]
    fn syntax_parse_output_lowers_in_a_separate_step() {
        let text = "module demo; fn main() { return 42 }";
        let mut sources = SourceManager::new();
        let id = sources.add("test.jdn", text).expect("source ID should fit");
        let source = sources.get(id).expect("source should exist");
        let lexed = lex(source);

        let syntax = parse_syntax(source, &lexed.tokens);
        assert!(!syntax.has_errors(), "{:?}", syntax.diagnostics);
        assert!(syntax.syntax.pretty_nodes().contains("FunctionDeclaration"));

        let lowered = lower_syntax_tree(source, &syntax.syntax);
        assert!(lowered.diagnostics.is_empty());
        assert_eq!(lowered.file.items.len(), 1);
    }

    #[test]
    fn parses_module_and_hello_function() {
        let output =
            parse_text("module examples.hello\n\nfn main() {\n  print(\"Hello, Jadren\")\n}\n");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        assert_eq!(output.file.module.expect("module").segments.len(), 2);
        assert_eq!(output.file.items.len(), 1);
        let function = function(&output.file.items[0]);
        assert_eq!(function.name.text, "main");
        assert_eq!(function.body.statements.len(), 1);
        assert!(matches!(
            function.body.statements[0],
            Statement::Expression {
                expression: Expression::Call { .. },
                ..
            }
        ));
    }

    #[test]
    fn records_lowering_ready_declaration_and_type_nodes() {
        let output = parse_text(
            "module demo.types; import core.math; fn map<T: Addable>(values: [T; 4]) -> Result<T> {}",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        assert_eq!(output.file.imports.len(), 1);

        let nodes = output.syntax.pretty_nodes();
        for expected in [
            "ModuleDeclaration",
            "ImportDeclaration",
            "FunctionDeclaration",
            "GenericParameterList",
            "GenericParameter",
            "ParameterList",
            "Parameter",
            "ArrayType",
            "PathType",
            "Block",
        ] {
            assert!(nodes.contains(expected), "missing {expected} in:\n{nodes}");
        }
    }

    #[test]
    fn lowers_if_groups_unary_expressions_and_match_guards() {
        let output = parse_text(
            r#"
fn choose(flag: Bool, value: Int32) -> Int32 {
    let selected = if !flag { (value + 1) } else { -value }
    return match selected {
        item if flag => item,
        _ => -1,
    }
}
"#,
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let function = function(&output.file.items[0]);
        assert_eq!(function.body.statements.len(), 2);
    }

    #[test]
    fn parses_parameters_bindings_return_and_precedence() {
        let output = parse_text(
            "pub fn add(a: Int32, b: Int32) -> Int32 { let scale = 2; return a + b * scale; }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let function = function(&output.file.items[0]);
        assert!(function.is_public);
        assert_eq!(function.parameters.len(), 2);
        assert!(function.return_type.is_some());
        assert_eq!(function.body.statements.len(), 2);
    }

    #[test]
    fn recovers_to_next_top_level_function() {
        let output = parse_text("unknown tokens fn valid() {}");
        assert!(output.has_errors());
        assert_eq!(output.file.items.len(), 1);
        let function = function(&output.file.items[0]);
        assert_eq!(function.name.text, "valid");
    }

    #[test]
    fn parses_annotations_records_enums_and_generic_types() {
        let output = parse_text(
            r#"
@repr(C)
pub struct Pair<T: Addable> {
    pub left: T,
    right: Option<T>,
}

component Position { value: Float3 }

enum Result<T, E> {
    Ok(T),
    Error(message: E),
}

@noalloc
fn first(values: Slice<Pair<Int32>>) -> Result<Int32, LoadError> {
    return values[0].left
}
"#,
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        assert_eq!(output.file.items.len(), 4);

        let Item::Struct(record) = &output.file.items[0] else {
            panic!("expected struct")
        };
        assert_eq!(record.annotations.len(), 1);
        assert_eq!(record.generic_parameters.len(), 1);
        assert_eq!(record.fields.len(), 2);
        assert!(matches!(record.fields[1].ty, TypeRef::Path { .. }));

        let Item::Enum(declaration) = &output.file.items[2] else {
            panic!("expected enum")
        };
        assert_eq!(declaration.variants.len(), 2);
        assert_eq!(declaration.variants[1].fields.len(), 1);

        assert_eq!(function(&output.file.items[3]).annotations.len(), 1);
    }

    #[test]
    fn parses_extern_c_block_and_unsafe_function_signatures() {
        let output = parse_text(
            r#"
module ffi.hash;
extern "C" {
    unsafe fn external_hash(data: Pointer<UInt8>, count: UIntSize) -> UInt64;
}
"#,
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let Item::ExternBlock(block) = &output.file.items[0] else {
            panic!("expected extern block")
        };
        assert_eq!(block.abi, "C");
        assert_eq!(block.functions.len(), 1);
        assert!(block.functions[0].is_unsafe);
        assert_eq!(block.functions[0].name.text, "external_hash");
        assert_eq!(block.functions[0].parameters.len(), 2);
        assert!(block.functions[0].return_type.is_some());
        assert!(
            output
                .syntax
                .pretty_nodes()
                .contains("ExternFunctionDeclaration")
        );
    }

    #[test]
    fn parses_owned_read_and_write_capability_types() {
        let output = parse_text(
            "fn access(owner: owned Buffer<Int32>, reader: read Buffer<Int32>, writer: write Buffer<Int32>) {}",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let function = function(&output.file.items[0]);
        assert!(matches!(
            function.parameters[0].ty,
            TypeRef::Capability {
                capability: TypeCapability::Owned,
                ..
            }
        ));
        assert!(matches!(
            function.parameters[1].ty,
            TypeRef::Capability {
                capability: TypeCapability::Read,
                ..
            }
        ));
        assert!(matches!(
            function.parameters[2].ty,
            TypeRef::Capability {
                capability: TypeCapability::Write,
                ..
            }
        ));
    }

    #[test]
    fn parses_explicit_function_type() {
        let output = parse_text(
            "fn apply(callback: fn(Int32, UInt8) -> UInt64, value: Int32) -> UInt64 { return callback(value, 1) }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let function = function(&output.file.items[0]);
        let TypeRef::Function {
            parameters,
            return_type,
            ..
        } = &function.parameters[0].ty
        else {
            panic!("expected explicit function type")
        };
        assert_eq!(parameters.len(), 2);
        assert!(return_type.is_some());
        assert!(output.syntax.pretty_nodes().contains("FunctionType"));
    }

    #[test]
    fn parses_named_region_statement_and_body() {
        let output = parse_text(
            "fn main() { region frame { let values: Buffer<Int32> = frame.allocate(4) } }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let function = function(&output.file.items[0]);
        assert!(matches!(
            &function.body.statements[0],
            Statement::Region { name, body, .. }
                if name.text == "frame" && body.statements.len() == 1
        ));
        assert!(output.syntax.pretty_nodes().contains("RegionStatement"));
    }

    #[test]
    fn parses_while_break_and_continue_statements() {
        let output = parse_text(
            "fn tick(count: Int32) { while count > 0 { if count == 2 { continue } count -= 1 if count == 1 { break } } }",
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let function = function(&output.file.items[0]);
        let Statement::While { body, .. } = &function.body.statements[0] else {
            panic!("expected while statement")
        };
        assert_eq!(body.statements.len(), 3);
        assert!(output.syntax.pretty_nodes().contains("WhileStatement"));
        assert!(output.syntax.pretty_nodes().contains("BreakStatement"));
        assert!(output.syntax.pretty_nodes().contains("ContinueStatement"));
    }

    #[test]
    fn parses_for_array_iteration() {
        let output =
            parse_text("fn sum(values: [Int32; 3]) { for value in values { print(value) } }");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let function = function(&output.file.items[0]);
        let Statement::For {
            binding,
            iterable: Expression::Name(iterable),
            body,
            ..
        } = &function.body.statements[0]
        else {
            panic!("expected for statement")
        };
        assert_eq!(binding.text, "value");
        assert_eq!(iterable.text, "values");
        assert_eq!(body.statements.len(), 1);
        assert!(output.syntax.pretty_nodes().contains("ForStatement"));
    }

    #[test]
    fn rejects_user_iterator_trait_declaration_in_0_1() {
        let output = parse_text("trait Iterator<T> { fn next(value: T) -> T }");
        assert!(output.has_errors());
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0100")
        );
    }

    #[test]
    fn parses_numeric_cast_expression() {
        let output = parse_text("fn widen(value: Int32) -> Int64 { return value as Int64 }");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let function = function(&output.file.items[0]);
        assert!(matches!(
            &function.body.statements[0],
            Statement::Return {
                value: Some(Expression::Cast { .. }),
                ..
            }
        ));
        assert!(output.syntax.pretty_nodes().contains("CastExpression"));
    }

    #[test]
    fn parses_struct_literals_arrays_fields_and_match() {
        let output = parse_text(
            r#"
fn create() {
    let user = User { name: "Ada", scores: [1, 2, 3] }
    match load() {
        Ok(value) => print(value.name),
        Error(_) => print("error"),
    }
}
"#,
        );
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let function = function(&output.file.items[0]);
        assert_eq!(function.body.statements.len(), 2);
        assert!(matches!(
            function.body.statements[1],
            Statement::Expression {
                expression: Expression::Match { .. },
                ..
            }
        ));
    }

    #[test]
    fn reports_missing_closing_brace() {
        let output = parse_text("fn main() { print(1)");
        assert!(output.has_errors());
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "J0104")
        );
    }

    #[test]
    fn reports_expected_keyword_spelling_for_recovery() {
        let output = parse_text("fn main() { for value values { } }");
        let diagnostic = output
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "J0103")
            .expect("missing for keyword diagnostic");
        assert_eq!(diagnostic.message, "expected `in`");
        assert_eq!(diagnostic.primary.message, "keyword is missing");
        assert_eq!(diagnostic.help.as_deref(), Some("insert `in`"));
    }

    #[test]
    fn syntax_tree_reconstructs_valid_and_recovered_source() {
        for text in [
            "// trivia\nfn main() { let values = [1, 2]; return values[0] }\n",
            "unknown tokens /* kept */ fn valid() {}\n",
        ] {
            let mut sources = SourceManager::new();
            let id = sources.add("test.jdn", text).expect("source ID should fit");
            let source = sources.get(id).expect("source should exist");
            let lexed = lex(source);
            assert!(!lexed.has_errors(), "test input must lex cleanly");

            let parsed = parse(source, &lexed.tokens);
            assert_eq!(parsed.syntax.reconstruct(source), text);
            assert_eq!(parsed.syntax.root().token_count(), lexed.tokens.len());
            assert!(
                parsed
                    .diagnostics
                    .iter()
                    .all(|diagnostic| diagnostic.code != "J0199"),
                "syntax tree projection failed: {:?}",
                parsed.diagnostics
            );
        }
    }

    #[test]
    fn recovers_missing_block_brace_before_next_function() {
        let text = "fn broken() { let value = ; fn good() { return 1 }";
        let mut sources = SourceManager::new();
        let id = sources.add("test.jdn", text).expect("source ID should fit");
        let source = sources.get(id).expect("source should exist");
        let lexed = lex(source);
        let output = parse(source, &lexed.tokens);

        assert!(output.has_errors());
        assert_eq!(output.file.items.len(), 2, "{:?}", output.diagnostics);
        assert_eq!(function(&output.file.items[0]).name.text, "broken");
        assert_eq!(function(&output.file.items[1]).name.text, "good");
        assert_eq!(output.syntax.reconstruct(source), text);
        assert!(
            output
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "J0199"),
            "syntax projection must remain valid: {:?}",
            output.diagnostics
        );
    }

    #[test]
    fn recovers_missing_record_brace_before_function() {
        let output = parse_text("struct Broken { value: Int32 fn good() { return 1 }");
        assert!(output.has_errors());
        assert_eq!(output.file.items.len(), 2, "{:?}", output.diagnostics);
        assert!(matches!(output.file.items[0], Item::Struct(_)));
        assert_eq!(function(&output.file.items[1]).name.text, "good");
        assert!(
            output
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "J0199")
        );
    }

    #[test]
    fn recovers_match_before_following_statement() {
        let output =
            parse_text("fn main() { match value { Ok => 1 let recovered = 2; return recovered }");
        assert!(output.has_errors());
        let body = &function(&output.file.items[0]).body;
        assert_eq!(body.statements.len(), 3, "{:?}", output.diagnostics);
        assert!(matches!(body.statements[1], Statement::Binding { .. }));
        assert!(matches!(body.statements[2], Statement::Return { .. }));
    }

    #[test]
    fn skipped_tokens_are_marked_as_error_syntax() {
        let text = "unknown tokens fn valid() {}";
        let mut sources = SourceManager::new();
        let id = sources.add("test.jdn", text).expect("source ID should fit");
        let source = sources.get(id).expect("source should exist");
        let lexed = lex(source);
        let output = parse(source, &lexed.tokens);

        assert!(output.syntax.pretty(source).contains("Error 0..14"));
        assert_eq!(output.syntax.reconstruct(source), text);
    }

    #[test]
    fn parses_postfix_try_before_following_field_access() {
        let output = parse_text("fn main() { let value = load()?.field }");
        assert!(!output.has_errors(), "{:?}", output.diagnostics);
        let body = &function(&output.file.items[0]).body;
        let Statement::Binding {
            value: Some(Expression::Field { base, .. }),
            ..
        } = &body.statements[0]
        else {
            panic!("expected field initializer");
        };
        assert!(matches!(
            base.as_ref(),
            Expression::Try { operand, .. }
                if matches!(operand.as_ref(), Expression::Call { .. })
        ));
    }
}
