#![no_main]

use jadren_lexer::lex;
use jadren_parser::{lower_syntax_tree, parse_syntax};
use jadren_source::SourceManager;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let mut sources = SourceManager::new();
    let source_id = sources
        .add("fuzz.jdn", text.as_ref())
        .expect("one fuzz source must fit in the source manager");
    let source = sources
        .get(source_id)
        .expect("registered fuzz source must exist");

    let lexed = lex(source);
    let parsed = parse_syntax(source, &lexed.tokens);
    let lowered = lower_syntax_tree(source, &parsed.syntax);

    assert_eq!(parsed.syntax.reconstruct(source), text);
    assert_eq!(parsed.syntax.root().token_count(), lexed.tokens.len());
    assert!(
        lowered.diagnostics.is_empty(),
        "lowering invariant failed: {:?}",
        lowered.diagnostics
    );
});
