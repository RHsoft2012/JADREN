//! Canonical, token-preserving formatting for Jadren source.
//!
//! The formatter intentionally works below the semantic layers: comments and
//! literal spelling are retained from the original source while whitespace is
//! regenerated from lexical structure. This makes formatting safe for code
//! that has not type-checked yet and keeps diagnostics/source spans meaningful.

use std::fmt;

use jadren_lexer::{Keyword, Operator, Punctuation, Token, TokenKind};
use jadren_source::SourceFile;

/// Formatting failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormatError {
    /// The lexer retained an invalid token, so formatting would hide a source error.
    InvalidToken,
    /// A token span did not belong to the supplied source.
    InvalidSpan,
}

impl fmt::Display for FormatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidToken => {
                formatter.write_str("cannot format source containing invalid tokens")
            }
            Self::InvalidSpan => formatter.write_str("token span does not belong to source"),
        }
    }
}

impl std::error::Error for FormatError {}

/// Formats one lexed source into the canonical Jadren layout.
pub fn format_source(source: &SourceFile, tokens: &[Token]) -> Result<String, FormatError> {
    let significant: Vec<Token> = tokens
        .iter()
        .copied()
        .filter(|token| token.kind != TokenKind::Eof)
        .collect();
    if significant
        .iter()
        .any(|token| token.kind == TokenKind::Invalid)
    {
        return Err(FormatError::InvalidToken);
    }

    let mut formatter = Formatter::new(source, &significant);
    formatter.run()?;
    Ok(formatter.finish())
}

struct Formatter<'source> {
    source: &'source SourceFile,
    tokens: &'source [Token],
    output: String,
    indent: usize,
    line_start: bool,
    previous: Option<Token>,
    angle_depth: usize,
}

impl<'source> Formatter<'source> {
    const INDENT: &'static str = "    ";

    fn new(source: &'source SourceFile, tokens: &'source [Token]) -> Self {
        Self {
            source,
            tokens,
            output: String::new(),
            indent: 0,
            line_start: true,
            previous: None,
            angle_depth: 0,
        }
    }

    fn run(&mut self) -> Result<(), FormatError> {
        for index in 0..self.tokens.len() {
            let token = self.tokens[index];
            let text = self
                .source
                .slice(token.span)
                .ok_or(FormatError::InvalidSpan)?;
            if token.kind.is_trivia() {
                self.trivia(token.kind, text);
                continue;
            }
            let next = self.next_significant(index + 1)?;
            self.syntax(token, text, next);
            self.previous = Some(token);
        }
        Ok(())
    }

    fn next_significant(&self, mut index: usize) -> Result<Option<(Token, String)>, FormatError> {
        while let Some(token) = self.tokens.get(index).copied() {
            index += 1;
            if token.kind.is_trivia() {
                continue;
            }
            let text = self
                .source
                .slice(token.span)
                .ok_or(FormatError::InvalidSpan)?
                .to_owned();
            return Ok(Some((token, text)));
        }
        Ok(None)
    }

    fn trivia(&mut self, kind: TokenKind, text: &str) {
        match kind {
            TokenKind::Whitespace => {
                if text.contains(['\n', '\r']) && self.should_break_at_newline() {
                    self.newline();
                }
            }
            TokenKind::LineComment | TokenKind::DocLineComment => {
                self.space_if_needed();
                self.write(text.trim_end_matches(['\r', '\n']));
                self.newline();
            }
            TokenKind::BlockComment | TokenKind::DocBlockComment => {
                self.space_if_needed();
                self.write(text.trim());
                if text.contains(['\n', '\r']) {
                    self.newline();
                } else {
                    self.space_if_needed();
                }
            }
            _ => {}
        }
    }

    fn syntax(&mut self, token: Token, text: &str, next: Option<(Token, String)>) {
        match token.kind {
            TokenKind::Punctuation(Punctuation::LeftBrace) => {
                if !self.line_start
                    && !matches!(
                        self.previous.map(|p| p.kind),
                        Some(TokenKind::Punctuation(
                            Punctuation::LeftParen | Punctuation::LeftBracket
                        ))
                    )
                {
                    self.space_if_needed();
                }
                self.write(text);
                self.indent += 1;
                self.newline();
            }
            TokenKind::Punctuation(Punctuation::RightBrace) => {
                if !self.line_start {
                    self.newline();
                }
                self.indent = self.indent.saturating_sub(1);
                self.write(text);
            }
            TokenKind::Punctuation(Punctuation::Semicolon) => {
                self.trim_space();
                self.write(text);
                self.newline();
            }
            TokenKind::Punctuation(Punctuation::Comma) => {
                self.trim_space();
                self.write(text);
                if self.indent > 0 {
                    self.newline();
                } else {
                    self.space_if_needed();
                }
            }
            TokenKind::Punctuation(Punctuation::Colon) => {
                self.trim_space();
                self.write(text);
                self.space_if_needed();
            }
            TokenKind::Punctuation(Punctuation::Dot) => {
                self.trim_space();
                self.write(text);
            }
            TokenKind::Punctuation(Punctuation::At) => {
                self.write_indentation();
                self.write(text);
            }
            TokenKind::Punctuation(Punctuation::LeftParen) => {
                if self.previous.is_some_and(|previous| {
                    matches!(
                        previous.kind,
                        TokenKind::Keyword(
                            Keyword::If | Keyword::For | Keyword::While | Keyword::Match
                        )
                    )
                }) {
                    self.space_if_needed();
                }
                self.trim_space();
                self.write(text);
            }
            TokenKind::Punctuation(Punctuation::RightParen | Punctuation::RightBracket) => {
                self.trim_space();
                self.write(text);
            }
            TokenKind::Punctuation(Punctuation::LeftBracket) => {
                self.trim_space();
                self.write(text);
            }
            TokenKind::Operator(Operator::PathSeparator) => {
                self.trim_space();
                self.write(text);
            }
            TokenKind::Operator(Operator::Question) => {
                self.trim_space();
                self.write(text);
            }
            TokenKind::Operator(Operator::Less) if self.is_generic_open(next.as_ref()) => {
                self.trim_space();
                self.write(text);
                self.angle_depth += 1;
            }
            TokenKind::Operator(Operator::Greater) if self.angle_depth > 0 => {
                self.trim_space();
                self.write(text);
                self.angle_depth -= 1;
            }
            TokenKind::Operator(_) => {
                self.space_if_needed();
                self.write(text);
                self.space_if_needed();
            }
            _ if is_word_like(token.kind) => {
                if self.previous.is_some_and(|previous| {
                    is_word_like(previous.kind)
                        || matches!(
                            previous.kind,
                            TokenKind::Punctuation(
                                Punctuation::RightParen | Punctuation::RightBracket
                            )
                        )
                }) {
                    self.space_if_needed();
                } else if self.previous.is_some_and(|previous| {
                    previous.kind == TokenKind::Punctuation(Punctuation::RightBrace)
                }) && text != "else"
                {
                    self.newline();
                } else if self.previous.is_some_and(|previous| {
                    previous.kind == TokenKind::Punctuation(Punctuation::RightBrace)
                }) {
                    self.space_if_needed();
                }
                self.write_indentation();
                self.write(text);
            }
            _ => {
                self.write_indentation();
                self.write(text);
            }
        }
    }

    fn is_generic_open(&self, next: Option<&(Token, String)>) -> bool {
        self.previous
            .is_some_and(|previous| is_word_like(previous.kind))
            && next.is_some_and(|(token, _)| is_word_like(token.kind))
    }

    fn should_break_at_newline(&self) -> bool {
        if self.line_start {
            return false;
        }
        !self.previous.is_some_and(|previous| {
            matches!(
                previous.kind,
                TokenKind::Operator(_)
                    | TokenKind::Punctuation(
                        Punctuation::LeftParen
                            | Punctuation::LeftBracket
                            | Punctuation::Comma
                            | Punctuation::Colon
                            | Punctuation::Dot
                            | Punctuation::At
                    )
            )
        })
    }

    fn write_indentation(&mut self) {
        if self.line_start {
            for _ in 0..self.indent {
                self.output.push_str(Self::INDENT);
            }
            self.line_start = false;
        }
    }

    fn write(&mut self, text: &str) {
        self.write_indentation();
        self.output.push_str(text);
    }

    fn space_if_needed(&mut self) {
        if !self.line_start && !self.output.ends_with([' ', '\n']) {
            self.output.push(' ');
        }
    }

    fn trim_space(&mut self) {
        while self.output.ends_with(' ') {
            self.output.pop();
        }
    }

    fn newline(&mut self) {
        self.trim_space();
        if !self.output.is_empty() && !self.output.ends_with('\n') {
            self.output.push('\n');
        }
        self.line_start = true;
    }

    fn finish(mut self) -> String {
        self.trim_space();
        while self.output.ends_with('\n') {
            self.output.pop();
        }
        if !self.output.is_empty() {
            self.output.push('\n');
        }
        self.output
    }
}

fn is_word_like(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Identifier
            | TokenKind::Keyword(_)
            | TokenKind::IntegerLiteral
            | TokenKind::FloatLiteral
            | TokenKind::StringLiteral
            | TokenKind::CharLiteral
    )
}

#[cfg(test)]
mod tests {
    use jadren_lexer::lex;
    use jadren_source::SourceManager;

    use super::format_source;

    fn format(text: &str) -> String {
        let mut sources = SourceManager::new();
        let id = sources.add("memory.jdn", text).expect("source id");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        assert!(lexed.diagnostics.is_empty(), "test input must lex cleanly");
        format_source(source, &lexed.tokens).expect("format succeeds")
    }

    #[test]
    fn formats_spacing_and_indentation() {
        assert_eq!(
            format("module test;fn main(){let x=1+2;return x;}"),
            "module test;\nfn main() {\n    let x = 1 + 2;\n    return x;\n}\n"
        );
    }

    #[test]
    fn preserves_literals_and_comments() {
        let formatted = format("module test; // keep\nfn main(){let text=\"a b\";}");
        assert!(formatted.contains("// keep"));
        assert!(formatted.contains("\"a b\""));
    }

    #[test]
    fn formatting_is_idempotent() {
        let once = format("module test;fn main(){let x=1+2;return x;}");
        assert_eq!(format(&once), once);
    }
}
