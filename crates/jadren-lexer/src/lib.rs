//! Lossless, UTF-8-aware lexer for Jadren source files.

use jadren_diagnostics::{Diagnostic, Severity};
use jadren_source::{SourceFile, SourceId, Span};

/// A reserved Jadren keyword.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Keyword {
    /// `as`
    As,
    /// `break`
    Break,
    /// `component`
    Component,
    /// `const`
    Const,
    /// `continue`
    Continue,
    /// `else`
    Else,
    /// `enum`
    Enum,
    /// `extern`
    Extern,
    /// `false`
    False,
    /// `fn`
    Fn,
    /// `for`
    For,
    /// `if`
    If,
    /// `import`
    Import,
    /// `in`
    In,
    /// `let`
    Let,
    /// `match`
    Match,
    /// `module`
    Module,
    /// `mut`
    Mut,
    /// `owned`
    Owned,
    /// `panic`
    Panic,
    /// `pub`
    Pub,
    /// `read`
    Read,
    /// `region`
    Region,
    /// `result`
    Result,
    /// `return`
    Return,
    /// `shared`
    Shared,
    /// `struct`
    Struct,
    /// `trait`
    Trait,
    /// `true`
    True,
    /// `type`
    Type,
    /// `unsafe`
    Unsafe,
    /// `var`
    Var,
    /// `where`
    Where,
    /// `while`
    While,
    /// `write`
    Write,
}

impl Keyword {
    /// Every reserved keyword in canonical source order.
    pub const ALL: &'static [Self] = &[
        Self::As,
        Self::Break,
        Self::Component,
        Self::Const,
        Self::Continue,
        Self::Else,
        Self::Enum,
        Self::Extern,
        Self::False,
        Self::Fn,
        Self::For,
        Self::If,
        Self::Import,
        Self::In,
        Self::Let,
        Self::Match,
        Self::Module,
        Self::Mut,
        Self::Owned,
        Self::Panic,
        Self::Pub,
        Self::Read,
        Self::Region,
        Self::Result,
        Self::Return,
        Self::Shared,
        Self::Struct,
        Self::Trait,
        Self::True,
        Self::Type,
        Self::Unsafe,
        Self::Var,
        Self::Where,
        Self::While,
        Self::Write,
    ];

    /// Returns the canonical source spelling of this keyword.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::As => "as",
            Self::Break => "break",
            Self::Component => "component",
            Self::Const => "const",
            Self::Continue => "continue",
            Self::Else => "else",
            Self::Enum => "enum",
            Self::Extern => "extern",
            Self::False => "false",
            Self::Fn => "fn",
            Self::For => "for",
            Self::If => "if",
            Self::Import => "import",
            Self::In => "in",
            Self::Let => "let",
            Self::Match => "match",
            Self::Module => "module",
            Self::Mut => "mut",
            Self::Owned => "owned",
            Self::Panic => "panic",
            Self::Pub => "pub",
            Self::Read => "read",
            Self::Region => "region",
            Self::Result => "result",
            Self::Return => "return",
            Self::Shared => "shared",
            Self::Struct => "struct",
            Self::Trait => "trait",
            Self::True => "true",
            Self::Type => "type",
            Self::Unsafe => "unsafe",
            Self::Var => "var",
            Self::Where => "where",
            Self::While => "while",
            Self::Write => "write",
        }
    }

    fn from_identifier(identifier: &str) -> Option<Self> {
        Some(match identifier {
            "as" => Self::As,
            "break" => Self::Break,
            "component" => Self::Component,
            "const" => Self::Const,
            "continue" => Self::Continue,
            "else" => Self::Else,
            "enum" => Self::Enum,
            "extern" => Self::Extern,
            "false" => Self::False,
            "fn" => Self::Fn,
            "for" => Self::For,
            "if" => Self::If,
            "import" => Self::Import,
            "in" => Self::In,
            "let" => Self::Let,
            "match" => Self::Match,
            "module" => Self::Module,
            "mut" => Self::Mut,
            "owned" => Self::Owned,
            "panic" => Self::Panic,
            "pub" => Self::Pub,
            "read" => Self::Read,
            "region" => Self::Region,
            "result" => Self::Result,
            "return" => Self::Return,
            "shared" => Self::Shared,
            "struct" => Self::Struct,
            "trait" => Self::Trait,
            "true" => Self::True,
            "type" => Self::Type,
            "unsafe" => Self::Unsafe,
            "var" => Self::Var,
            "where" => Self::Where,
            "while" => Self::While,
            "write" => Self::Write,
            _ => return None,
        })
    }
}

/// Delimiter or separator token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Punctuation {
    /// `(`
    LeftParen,
    /// `)`
    RightParen,
    /// `{`
    LeftBrace,
    /// `}`
    RightBrace,
    /// `[`
    LeftBracket,
    /// `]`
    RightBracket,
    /// `,`
    Comma,
    /// `.`
    Dot,
    /// `:`
    Colon,
    /// `;`
    Semicolon,
    /// `@`
    At,
}

/// Operator token, matched longest-first.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operator {
    /// `=`
    Assign,
    /// `+`
    Plus,
    /// `-`
    Minus,
    /// `*`
    Star,
    /// `/`
    Slash,
    /// `%`
    Percent,
    /// `!`
    Bang,
    /// `&`
    Ampersand,
    /// `|`
    Pipe,
    /// `^`
    Caret,
    /// `~`
    Tilde,
    /// `?`
    Question,
    /// `<`
    Less,
    /// `>`
    Greater,
    /// `==`
    Equal,
    /// `!=`
    NotEqual,
    /// `<=`
    LessEqual,
    /// `>=`
    GreaterEqual,
    /// `&&`
    And,
    /// `||`
    Or,
    /// `+=`
    PlusAssign,
    /// `-=`
    MinusAssign,
    /// `*=`
    StarAssign,
    /// `/=`
    SlashAssign,
    /// `%=`
    PercentAssign,
    /// `->`
    Arrow,
    /// `=>`
    FatArrow,
    /// `..`
    Range,
    /// `..<`
    RangeExclusive,
    /// `::`
    PathSeparator,
}

/// Lexical token category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenKind {
    /// Spaces, newlines, or an initial UTF-8 BOM.
    Whitespace,
    /// `// ...`
    LineComment,
    /// `/// ...`
    DocLineComment,
    /// `/* ... */`
    BlockComment,
    /// `/** ... */`
    DocBlockComment,
    /// User-defined identifier.
    Identifier,
    /// Reserved keyword.
    Keyword(Keyword),
    /// Integer literal, including an optional type suffix.
    IntegerLiteral,
    /// Floating-point literal, including an optional type suffix.
    FloatLiteral,
    /// Double-quoted string literal.
    StringLiteral,
    /// Single-quoted character literal.
    CharLiteral,
    /// Delimiter or separator.
    Punctuation(Punctuation),
    /// Operator.
    Operator(Operator),
    /// Invalid source fragment retained for lossless processing.
    Invalid,
    /// End of source marker.
    Eof,
}

impl TokenKind {
    /// Returns whether the token carries formatting trivia rather than syntax.
    #[must_use]
    pub const fn is_trivia(self) -> bool {
        matches!(
            self,
            Self::Whitespace
                | Self::LineComment
                | Self::DocLineComment
                | Self::BlockComment
                | Self::DocBlockComment
        )
    }
}

/// One lossless lexical token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Token {
    /// Token category.
    pub kind: TokenKind,
    /// Exact byte range in the input source.
    pub span: Span,
}

/// Complete lexer output.
#[derive(Clone, Debug, Default)]
pub struct LexOutput {
    /// Lossless tokens, including trivia and EOF.
    pub tokens: Vec<Token>,
    /// Lexical diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

impl LexOutput {
    /// Returns whether at least one error was produced.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }
}

/// Lexes one immutable source file.
#[must_use]
pub fn lex(source: &SourceFile) -> LexOutput {
    Lexer::new(source).run()
}

struct Lexer<'source> {
    source_id: SourceId,
    text: &'source str,
    position: usize,
    output: LexOutput,
}

impl<'source> Lexer<'source> {
    fn new(source: &'source SourceFile) -> Self {
        Self {
            source_id: source.id(),
            text: source.text(),
            position: 0,
            output: LexOutput::default(),
        }
    }

    fn run(mut self) -> LexOutput {
        while self.position < self.text.len() {
            self.lex_one();
        }
        let eof = Span::empty(self.source_id, self.text.len());
        self.output.tokens.push(Token {
            kind: TokenKind::Eof,
            span: eof,
        });
        self.output
    }

    fn lex_one(&mut self) {
        let start = self.position;

        if start == 0 && self.remaining().starts_with('\u{feff}') {
            self.advance_char();
            self.push(TokenKind::Whitespace, start);
            return;
        }

        let Some(character) = self.peek_char() else {
            return;
        };

        if character.is_whitespace() {
            self.advance_char();
            while self.peek_char().is_some_and(char::is_whitespace) {
                self.advance_char();
            }
            self.push(TokenKind::Whitespace, start);
            return;
        }

        if self.remaining().starts_with("//") {
            self.lex_line_comment(start);
            return;
        }
        if self.remaining().starts_with("/*") {
            self.lex_block_comment(start);
            return;
        }
        if character == '"' {
            self.lex_quoted(start, '"', TokenKind::StringLiteral);
            return;
        }
        if character == '\'' {
            self.lex_quoted(start, '\'', TokenKind::CharLiteral);
            return;
        }
        if character.is_ascii_alphabetic() || character == '_' {
            self.lex_identifier(start);
            return;
        }
        if character.is_ascii_digit() {
            self.lex_number(start);
            return;
        }
        if let Some((kind, width)) = match_symbol(self.remaining()) {
            self.position += width;
            self.push(kind, start);
            return;
        }

        self.advance_char();
        let span = self.span(start);
        self.output.tokens.push(Token {
            kind: TokenKind::Invalid,
            span,
        });
        self.output.diagnostics.push(
            Diagnostic::error(
                "J0001",
                "invalid character",
                span,
                format!("`{character}` is not valid Jadren syntax"),
            )
            .with_help("remove the character or replace it with a valid token"),
        );
    }

    fn lex_line_comment(&mut self, start: usize) {
        let doc = self.remaining().starts_with("///");
        self.position += 2;
        while let Some(character) = self.peek_char() {
            if matches!(character, '\r' | '\n') {
                break;
            }
            self.advance_char();
        }
        self.push(
            if doc {
                TokenKind::DocLineComment
            } else {
                TokenKind::LineComment
            },
            start,
        );
    }

    fn lex_block_comment(&mut self, start: usize) {
        let doc = self.remaining().starts_with("/**") && !self.remaining().starts_with("/**/");
        self.position += 2;
        let mut depth = 1_u32;
        while self.position < self.text.len() {
            if self.remaining().starts_with("/*") {
                depth = depth.saturating_add(1);
                self.position += 2;
            } else if self.remaining().starts_with("*/") {
                depth -= 1;
                self.position += 2;
                if depth == 0 {
                    self.push(
                        if doc {
                            TokenKind::DocBlockComment
                        } else {
                            TokenKind::BlockComment
                        },
                        start,
                    );
                    return;
                }
            } else {
                self.advance_char();
            }
        }

        let span = self.span(start);
        self.output.tokens.push(Token {
            kind: if doc {
                TokenKind::DocBlockComment
            } else {
                TokenKind::BlockComment
            },
            span,
        });
        self.output.diagnostics.push(
            Diagnostic::error(
                "J0002",
                "unterminated block comment",
                span,
                "this comment is not closed",
            )
            .with_help("add `*/` before the end of the file"),
        );
    }

    fn lex_quoted(&mut self, start: usize, quote: char, kind: TokenKind) {
        self.advance_char();
        let mut logical_characters = 0_usize;
        let mut closed = false;

        while let Some(character) = self.peek_char() {
            if character == quote {
                self.advance_char();
                closed = true;
                break;
            }
            if matches!(character, '\r' | '\n') {
                break;
            }
            if character == '\\' {
                let escape_start = self.position;
                self.advance_char();
                match self.peek_char() {
                    Some('n' | 'r' | 't' | '0' | '\\' | '"' | '\'') => {
                        self.advance_char();
                        logical_characters += 1;
                    }
                    Some(invalid) => {
                        self.advance_char();
                        let span = self.span(escape_start);
                        self.output.diagnostics.push(
                            Diagnostic::error(
                                "J0004",
                                "invalid escape sequence",
                                span,
                                format!("`\\{invalid}` is not a supported escape"),
                            )
                            .with_help("use one of: \\n, \\r, \\t, \\0, \\\\, \\\", or \\'"),
                        );
                    }
                    None => break,
                }
            } else {
                self.advance_char();
                logical_characters += 1;
            }
        }

        let span = self.span(start);
        self.output.tokens.push(Token { kind, span });
        if !closed {
            let description = if quote == '"' { "string" } else { "character" };
            self.output.diagnostics.push(
                Diagnostic::error(
                    "J0003",
                    format!("unterminated {description} literal"),
                    span,
                    format!("this {description} literal is not closed"),
                )
                .with_help(format!("add `{quote}` before the end of the line")),
            );
        } else if quote == '\'' && logical_characters != 1 {
            self.output.diagnostics.push(
                Diagnostic::error(
                    "J0005",
                    "invalid character literal",
                    span,
                    "a character literal must contain exactly one character",
                )
                .with_help("use double quotes for a string literal"),
            );
        }
    }

    fn lex_identifier(&mut self, start: usize) {
        self.advance_char();
        while self
            .peek_char()
            .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            self.advance_char();
        }
        let text = &self.text[start..self.position];
        self.push(
            Keyword::from_identifier(text).map_or(TokenKind::Identifier, TokenKind::Keyword),
            start,
        );
    }

    fn lex_number(&mut self, start: usize) {
        self.consume_digits_and_underscores();
        let mut is_float = false;

        if self.peek_char() == Some('.')
            && !self.remaining().starts_with("..")
            && self
                .peek_second_char()
                .is_some_and(|value| value.is_ascii_digit())
        {
            is_float = true;
            self.advance_char();
            self.consume_digits_and_underscores();
        }

        if matches!(self.peek_char(), Some('e' | 'E')) {
            is_float = true;
            self.advance_char();
            if matches!(self.peek_char(), Some('+' | '-')) {
                self.advance_char();
            }
            self.consume_digits_and_underscores();
        }

        while self
            .peek_char()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        {
            self.advance_char();
        }

        let span = self.span(start);
        let literal = &self.text[start..self.position];
        self.output.tokens.push(Token {
            kind: if is_float {
                TokenKind::FloatLiteral
            } else {
                TokenKind::IntegerLiteral
            },
            span,
        });

        if has_invalid_numeric_separator(literal) {
            self.output.diagnostics.push(
                Diagnostic::error(
                    "J0006",
                    "invalid numeric separator",
                    span,
                    "underscores must separate digits",
                )
                .with_help("remove repeated, leading, or trailing underscores"),
            );
        }
    }

    fn consume_digits_and_underscores(&mut self) {
        while self
            .peek_char()
            .is_some_and(|character| character.is_ascii_digit() || character == '_')
        {
            self.advance_char();
        }
    }

    fn push(&mut self, kind: TokenKind, start: usize) {
        self.output.tokens.push(Token {
            kind,
            span: self.span(start),
        });
    }

    fn span(&self, start: usize) -> Span {
        Span::new(self.source_id, start, self.position)
            .unwrap_or_else(|| Span::empty(self.source_id, start))
    }

    fn remaining(&self) -> &'source str {
        &self.text[self.position..]
    }

    fn peek_char(&self) -> Option<char> {
        self.remaining().chars().next()
    }

    fn peek_second_char(&self) -> Option<char> {
        let mut characters = self.remaining().chars();
        let _ = characters.next();
        characters.next()
    }

    fn advance_char(&mut self) {
        if let Some(character) = self.peek_char() {
            self.position += character.len_utf8();
        }
    }
}

fn has_invalid_numeric_separator(literal: &str) -> bool {
    let bytes = literal.as_bytes();
    if bytes.first() == Some(&b'_') || bytes.last() == Some(&b'_') {
        return true;
    }
    bytes.windows(2).any(|window| {
        window == b"__"
            || matches!(window, [b'_', b'.' | b'e' | b'E'])
            || matches!(window, [b'.' | b'e' | b'E' | b'+' | b'-', b'_'])
    })
}

fn match_symbol(text: &str) -> Option<(TokenKind, usize)> {
    let (kind, width) = if text.starts_with("..<") {
        (TokenKind::Operator(Operator::RangeExclusive), 3)
    } else if text.starts_with("->") {
        (TokenKind::Operator(Operator::Arrow), 2)
    } else if text.starts_with("=>") {
        (TokenKind::Operator(Operator::FatArrow), 2)
    } else if text.starts_with("==") {
        (TokenKind::Operator(Operator::Equal), 2)
    } else if text.starts_with("!=") {
        (TokenKind::Operator(Operator::NotEqual), 2)
    } else if text.starts_with("<=") {
        (TokenKind::Operator(Operator::LessEqual), 2)
    } else if text.starts_with(">=") {
        (TokenKind::Operator(Operator::GreaterEqual), 2)
    } else if text.starts_with("&&") {
        (TokenKind::Operator(Operator::And), 2)
    } else if text.starts_with("||") {
        (TokenKind::Operator(Operator::Or), 2)
    } else if text.starts_with("+=") {
        (TokenKind::Operator(Operator::PlusAssign), 2)
    } else if text.starts_with("-=") {
        (TokenKind::Operator(Operator::MinusAssign), 2)
    } else if text.starts_with("*=") {
        (TokenKind::Operator(Operator::StarAssign), 2)
    } else if text.starts_with("/=") {
        (TokenKind::Operator(Operator::SlashAssign), 2)
    } else if text.starts_with("%=") {
        (TokenKind::Operator(Operator::PercentAssign), 2)
    } else if text.starts_with("..") {
        (TokenKind::Operator(Operator::Range), 2)
    } else if text.starts_with("::") {
        (TokenKind::Operator(Operator::PathSeparator), 2)
    } else {
        let kind = match text.as_bytes().first().copied()? {
            b'(' => TokenKind::Punctuation(Punctuation::LeftParen),
            b')' => TokenKind::Punctuation(Punctuation::RightParen),
            b'{' => TokenKind::Punctuation(Punctuation::LeftBrace),
            b'}' => TokenKind::Punctuation(Punctuation::RightBrace),
            b'[' => TokenKind::Punctuation(Punctuation::LeftBracket),
            b']' => TokenKind::Punctuation(Punctuation::RightBracket),
            b',' => TokenKind::Punctuation(Punctuation::Comma),
            b'.' => TokenKind::Punctuation(Punctuation::Dot),
            b':' => TokenKind::Punctuation(Punctuation::Colon),
            b';' => TokenKind::Punctuation(Punctuation::Semicolon),
            b'@' => TokenKind::Punctuation(Punctuation::At),
            b'=' => TokenKind::Operator(Operator::Assign),
            b'+' => TokenKind::Operator(Operator::Plus),
            b'-' => TokenKind::Operator(Operator::Minus),
            b'*' => TokenKind::Operator(Operator::Star),
            b'/' => TokenKind::Operator(Operator::Slash),
            b'%' => TokenKind::Operator(Operator::Percent),
            b'!' => TokenKind::Operator(Operator::Bang),
            b'&' => TokenKind::Operator(Operator::Ampersand),
            b'|' => TokenKind::Operator(Operator::Pipe),
            b'^' => TokenKind::Operator(Operator::Caret),
            b'~' => TokenKind::Operator(Operator::Tilde),
            b'?' => TokenKind::Operator(Operator::Question),
            b'<' => TokenKind::Operator(Operator::Less),
            b'>' => TokenKind::Operator(Operator::Greater),
            _ => return None,
        };
        (kind, 1)
    };
    Some((kind, width))
}

#[cfg(test)]
mod tests {
    use jadren_source::SourceManager;

    use super::{Keyword, Operator, TokenKind, lex};

    fn lex_text(text: &str) -> super::LexOutput {
        let mut sources = SourceManager::new();
        let id = sources.add("test.jdn", text).expect("source ID should fit");
        lex(sources.get(id).expect("source should exist"))
    }

    fn syntax_kinds(output: &super::LexOutput) -> Vec<TokenKind> {
        output
            .tokens
            .iter()
            .map(|token| token.kind)
            .filter(|kind| !kind.is_trivia())
            .collect()
    }

    #[test]
    fn lexes_hello_world() {
        let output = lex_text("fn main() { print(\"Hello, Jadren\") }\n");
        assert!(!output.has_errors());
        let kinds = syntax_kinds(&output);
        assert_eq!(kinds.first(), Some(&TokenKind::Keyword(Keyword::Fn)));
        assert_eq!(
            kinds
                .iter()
                .filter(|kind| **kind == TokenKind::Identifier)
                .count(),
            2
        );
        assert!(kinds.contains(&TokenKind::StringLiteral));
        assert_eq!(kinds.last(), Some(&TokenKind::Eof));
    }

    #[test]
    fn keyword_spelling_is_canonical() {
        assert_eq!(Keyword::Fn.as_str(), "fn");
        assert_eq!(Keyword::In.as_str(), "in");
        assert_eq!(Keyword::Unsafe.as_str(), "unsafe");
    }

    #[test]
    fn keyword_catalog_is_unique_and_complete() {
        let labels = Keyword::ALL
            .iter()
            .map(|keyword| keyword.as_str())
            .collect::<Vec<_>>();
        assert_eq!(labels.len(), 35);
        let mut unique = labels.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), labels.len());
    }

    #[test]
    fn matches_long_operators_before_short_operators() {
        let output = lex_text("a..<b a..b a->b a=>b a+=b a==b");
        let kinds = syntax_kinds(&output);
        assert!(kinds.contains(&TokenKind::Operator(Operator::RangeExclusive)));
        assert!(kinds.contains(&TokenKind::Operator(Operator::Range)));
        assert!(kinds.contains(&TokenKind::Operator(Operator::Arrow)));
        assert!(kinds.contains(&TokenKind::Operator(Operator::FatArrow)));
        assert!(kinds.contains(&TokenKind::Operator(Operator::PlusAssign)));
        assert!(kinds.contains(&TokenKind::Operator(Operator::Equal)));
    }

    #[test]
    fn preserves_nested_block_comments() {
        let output = lex_text("/* outer /* nested */ done */fn");
        assert!(!output.has_errors());
        assert_eq!(output.tokens[0].kind, TokenKind::BlockComment);
        assert_eq!(
            syntax_kinds(&output),
            vec![TokenKind::Keyword(Keyword::Fn), TokenKind::Eof]
        );
    }

    #[test]
    fn reports_unterminated_literals_and_comments() {
        let string = lex_text("\"open");
        let comment = lex_text("/* open");
        assert_eq!(string.diagnostics[0].code, "J0003");
        assert_eq!(comment.diagnostics[0].code, "J0002");
    }

    #[test]
    fn reports_invalid_character_without_losing_utf8_boundaries() {
        let output = lex_text("fn ľ");
        assert!(output.has_errors());
        assert_eq!(output.diagnostics[0].code, "J0001");
        assert_eq!(output.diagnostics[0].primary.span.len(), 'ľ'.len_utf8());
    }

    #[test]
    fn rejects_explicit_lifetime_annotation_spelling_in_0_1() {
        let output = lex_text(
            "fn borrow<'a>(data: read Buffer<Int32>) -> read Buffer<Int32> { return data }",
        );
        assert!(output.has_errors());
        assert!(!output.diagnostics.is_empty());
    }

    #[test]
    fn rejects_user_layout_attribute_spelling_in_0_1() {
        let output = lex_text("#[align(16)] component Position { value: Float32 }");
        assert!(output.has_errors());
        assert!(!output.diagnostics.is_empty());
    }

    #[test]
    fn separates_ranges_from_float_literals() {
        let output = lex_text("0..10 0.5f32 1e-3");
        let kinds = syntax_kinds(&output);
        assert_eq!(
            kinds,
            vec![
                TokenKind::IntegerLiteral,
                TokenKind::Operator(Operator::Range),
                TokenKind::IntegerLiteral,
                TokenKind::FloatLiteral,
                TokenKind::FloatLiteral,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn validates_character_width_and_numeric_separators() {
        let characters = lex_text("'' 'ab' 'x'");
        let numbers = lex_text("1__000 2_ 3_000");
        assert_eq!(
            characters
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "J0005")
                .count(),
            2
        );
        assert_eq!(
            numbers
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "J0006")
                .count(),
            2
        );
    }
}
