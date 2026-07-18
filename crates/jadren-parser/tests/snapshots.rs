use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use jadren_lexer::lex;
use jadren_parser::{lower_syntax_tree, parse};
use jadren_source::SourceManager;

fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

#[test]
fn parser_tour_syntax_snapshot() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/parser-tour.jdn");
    let text =
        normalize_newlines(&fs::read_to_string(&path).expect("parser tour should be readable"));
    let mut sources = SourceManager::new();
    let id = sources.add(path, text).expect("source ID should fit");
    let source = sources.get(id).expect("source should exist");
    let lexed = lex(source);
    let parsed = parse(source, &lexed.tokens);

    assert!(!lexed.has_errors());
    assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics);
    let lowered = lower_syntax_tree(source, &parsed.syntax);
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    assert_eq!(
        format!("{:#?}", lowered.file),
        format!("{:#?}", parsed.file),
        "node-based lowering must match the current AST builder"
    );
    assert_eq!(
        parsed.syntax.pretty_nodes(),
        normalize_newlines(include_str!("snapshots/parser-tour.syntax.snap"))
    );
}

#[test]
fn recovery_syntax_and_diagnostics_snapshot() {
    let text = concat!(
        "unknown tokens\n",
        "fn broken() { match value { Ok => 1 let recovered = 2; return recovered }\n",
        "fn good() {}\n",
    );
    let mut sources = SourceManager::new();
    let id = sources
        .add("recovery.jdn", text)
        .expect("source ID should fit");
    let source = sources.get(id).expect("source should exist");
    let lexed = lex(source);
    let parsed = parse(source, &lexed.tokens);
    let lowered = lower_syntax_tree(source, &parsed.syntax);

    let mut actual = String::new();
    for diagnostic in &parsed.diagnostics {
        let _ = writeln!(
            actual,
            "diagnostic {} {}..{}",
            diagnostic.code, diagnostic.primary.span.start, diagnostic.primary.span.end
        );
    }
    let _ = writeln!(actual, "items {}", parsed.file.items.len());
    actual.push_str(&parsed.syntax.pretty_nodes());

    assert_eq!(
        actual,
        normalize_newlines(include_str!("snapshots/recovery.syntax.snap"))
    );
    assert_eq!(parsed.syntax.reconstruct(source), text);
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    assert_eq!(
        format!("{:#?}", lowered.file),
        format!("{:#?}", parsed.file),
        "recovery lowering must preserve the partial AST"
    );
}
