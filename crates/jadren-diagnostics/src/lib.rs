//! Structured diagnostics shared by Jadren compiler phases.

use std::fmt::Write as _;

use jadren_source::{SourceManager, Span};

/// Diagnostic severity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    /// Compilation cannot continue successfully.
    Error,
    /// Compilation can continue, but the source is suspicious.
    Warning,
    /// Additional non-problem information.
    Note,
}

impl Severity {
    /// Returns the stable lowercase representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Note => "note",
        }
    }
}

/// A labelled source range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Label {
    /// Labelled source range.
    pub span: Span,
    /// Explanation attached to the range.
    pub message: String,
}

impl Label {
    /// Creates a source label.
    #[must_use]
    pub fn new(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
        }
    }
}

/// Compiler diagnostic independent of its final renderer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    /// Stable diagnostic code, for example `J0001`.
    pub code: &'static str,
    /// Severity.
    pub severity: Severity,
    /// Short summary.
    pub message: String,
    /// Main source label.
    pub primary: Label,
    /// Related source labels.
    pub secondary: Vec<Label>,
    /// Additional notes.
    pub notes: Vec<String>,
    /// Optional actionable suggestion.
    pub help: Option<String>,
}

impl Diagnostic {
    /// Creates an error diagnostic.
    #[must_use]
    pub fn error(
        code: &'static str,
        message: impl Into<String>,
        span: Span,
        label: impl Into<String>,
    ) -> Self {
        Self {
            code,
            severity: Severity::Error,
            message: message.into(),
            primary: Label::new(span, label),
            secondary: Vec::new(),
            notes: Vec::new(),
            help: None,
        }
    }

    /// Adds a secondary source label.
    #[must_use]
    pub fn with_secondary(mut self, span: Span, message: impl Into<String>) -> Self {
        self.secondary.push(Label::new(span, message));
        self
    }

    /// Adds a note.
    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    /// Adds an actionable suggestion.
    #[must_use]
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

/// Renders a diagnostic for a terminal without ANSI colour escapes.
#[must_use]
pub fn render_text(diagnostic: &Diagnostic, sources: &SourceManager) -> String {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "{}[{}]: {}",
        diagnostic.severity.as_str(),
        diagnostic.code,
        diagnostic.message
    );

    if let Some(file) = sources.get(diagnostic.primary.span.source)
        && let Some(location) = file.location(diagnostic.primary.span.start)
    {
        let _ = writeln!(
            output,
            " --> {}:{}:{}",
            file.path().display(),
            location.line,
            location.column
        );
        let line_number_width = location.line.to_string().len();
        if let Some(line) = file.line_text(location.line) {
            let _ = writeln!(
                output,
                "{space:>width$} |",
                space = "",
                width = line_number_width
            );
            let _ = writeln!(output, "{} | {}", location.line, line);
            let underline_width =
                span_width_on_line(diagnostic.primary.span, file, location.line).max(1);
            let _ = writeln!(
                output,
                "{space:>width$} | {padding}{carets} {}",
                diagnostic.primary.message,
                space = "",
                width = line_number_width,
                padding = " ".repeat(location.column.saturating_sub(1)),
                carets = "^".repeat(underline_width)
            );
        }
    }

    for label in &diagnostic.secondary {
        if let Some(file) = sources.get(label.span.source)
            && let Some(location) = file.location(label.span.start)
        {
            let _ = writeln!(
                output,
                " note: {}:{}:{}: {}",
                file.path().display(),
                location.line,
                location.column,
                label.message
            );
        }
    }
    for note in &diagnostic.notes {
        let _ = writeln!(output, " note: {note}");
    }
    if let Some(help) = &diagnostic.help {
        let _ = writeln!(output, " help: {help}");
    }
    output
}

/// Renders a diagnostic as one self-contained JSON object.
#[must_use]
pub fn render_json(diagnostic: &Diagnostic, sources: &SourceManager) -> String {
    let mut output = String::from("{");
    push_json_field(&mut output, "code", diagnostic.code, false);
    push_json_field(&mut output, "severity", diagnostic.severity.as_str(), true);
    push_json_field(&mut output, "message", &diagnostic.message, true);

    output.push_str(",\"primary\":");
    push_label_json(&mut output, &diagnostic.primary, sources);

    output.push_str(",\"secondary\":[");
    for (index, label) in diagnostic.secondary.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        push_label_json(&mut output, label, sources);
    }
    output.push(']');

    output.push_str(",\"notes\":[");
    for (index, note) in diagnostic.notes.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        push_json_string(&mut output, note);
    }
    output.push(']');

    output.push_str(",\"help\":");
    if let Some(help) = &diagnostic.help {
        push_json_string(&mut output, help);
    } else {
        output.push_str("null");
    }
    output.push('}');
    output
}

fn span_width_on_line(
    span: Span,
    file: &jadren_source::SourceFile,
    one_based_line: usize,
) -> usize {
    let Some(start) = file.location(span.start) else {
        return 1;
    };
    if start.line != one_based_line {
        return 1;
    }
    let end_offset = span.end.min(file.text().len());
    let Some(end) = file.location(end_offset) else {
        return 1;
    };
    if end.line == start.line {
        end.column.saturating_sub(start.column)
    } else {
        file.line_text(start.line)
            .map_or(1, |line| line.chars().count() + 1 - start.column)
    }
}

fn push_label_json(output: &mut String, label: &Label, sources: &SourceManager) {
    output.push('{');
    let path = sources
        .get(label.span.source)
        .map(|file| file.path().display().to_string())
        .unwrap_or_default();
    let location = sources
        .get(label.span.source)
        .and_then(|file| file.location(label.span.start));
    push_json_field(output, "file", &path, false);
    let _ = write!(
        output,
        ",\"start\":{},\"end\":{}",
        label.span.start, label.span.end
    );
    if let Some(location) = location {
        let _ = write!(
            output,
            ",\"line\":{},\"column\":{}",
            location.line, location.column
        );
    } else {
        output.push_str(",\"line\":null,\"column\":null");
    }
    push_json_field(output, "label", &label.message, true);
    output.push('}');
}

fn push_json_field(output: &mut String, key: &str, value: &str, comma: bool) {
    if comma {
        output.push(',');
    }
    push_json_string(output, key);
    output.push(':');
    push_json_string(output, value);
}

fn push_json_string(output: &mut String, value: &str) {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                let _ = write!(output, "\\u{:04x}", u32::from(character));
            }
            character => output.push(character),
        }
    }
    output.push('"');
}

#[cfg(test)]
mod tests {
    use jadren_source::{SourceManager, Span};

    use super::{Diagnostic, render_json, render_text};

    #[test]
    fn renders_text_with_source_location() {
        let mut sources = SourceManager::new();
        let id = sources
            .add("sample.jdn", "fn main() { @ }\n")
            .expect("source ID should fit");
        let offset = sources
            .get(id)
            .and_then(|file| file.text().find('@'))
            .expect("test source contains @");
        let diagnostic = Diagnostic::error(
            "J0001",
            "invalid character",
            Span::new(id, offset, offset + 1).expect("ordered span"),
            "not valid here",
        )
        .with_help("remove the character");

        let rendered = render_text(&diagnostic, &sources);
        assert!(rendered.contains("error[J0001]: invalid character"));
        assert!(rendered.contains("sample.jdn:1:13"));
        assert!(rendered.contains("^ not valid here"));
        assert!(rendered.contains("help: remove the character"));
    }

    #[test]
    fn renders_valid_json_escaping() {
        let mut sources = SourceManager::new();
        let id = sources
            .add("quote\".jdn", "x")
            .expect("source ID should fit");
        let diagnostic = Diagnostic::error(
            "J0001",
            "bad \"token\"",
            Span::new(id, 0, 1).expect("ordered span"),
            "bad",
        );

        let rendered = render_json(&diagnostic, &sources);
        assert!(rendered.starts_with('{'));
        assert!(rendered.ends_with('}'));
        assert!(rendered.contains("bad \\\"token\\\""));
        assert!(rendered.contains("quote\\\".jdn"));
    }
}
