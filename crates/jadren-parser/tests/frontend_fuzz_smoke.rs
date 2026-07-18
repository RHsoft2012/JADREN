use jadren_lexer::lex;
use jadren_parser::{lower_syntax_tree, parse_syntax};
use jadren_source::SourceManager;

const HELLO: &[u8] = include_bytes!("../../../fuzz/corpus/lexer_parser/hello.jdn");
const RECOVERY: &[u8] = include_bytes!("../../../fuzz/corpus/lexer_parser/recovery.jdn");
const TRIVIA: &[u8] = include_bytes!("../../../fuzz/corpus/syntax_tree/trivia.jdn");
const UNICODE: &[u8] = include_bytes!("../../../fuzz/corpus/syntax_tree/unicode.jdn");

#[test]
fn deterministic_frontend_fuzz_smoke() {
    let seeds = [HELLO, RECOVERY, TRIVIA, UNICODE];
    for (index, seed) in seeds.iter().enumerate() {
        exercise(seed, &format!("seed-{index}"));
    }

    let mut state = 0x4a41_4452_454e_0101_u64;
    for case in 0..256 {
        let length = (next(&mut state) as usize) % 257;
        let mut bytes = Vec::with_capacity(length);
        for _ in 0..length {
            bytes.push(next(&mut state) as u8);
        }
        exercise(&bytes, &format!("random-{case}"));
    }

    for (seed_index, seed) in seeds.iter().enumerate() {
        for mutation in 0..64 {
            let mut bytes = seed.to_vec();
            if !bytes.is_empty() {
                let offset = (next(&mut state) as usize) % bytes.len();
                bytes[offset] = next(&mut state) as u8;
                if mutation % 4 == 0 {
                    let second = (next(&mut state) as usize) % bytes.len();
                    bytes[second] = next(&mut state) as u8;
                }
            }
            exercise(&bytes, &format!("mutation-{seed_index}-{mutation}"));
        }
    }
}

fn exercise(bytes: &[u8], label: &str) {
    let text = String::from_utf8_lossy(bytes).into_owned();
    let mut sources = SourceManager::new();
    let source_id = sources
        .add(format!("{label}.jdn"), text)
        .expect("one generated source must fit");
    let source = sources.get(source_id).expect("generated source must exist");

    let lexed = lex(source);
    let parsed = parse_syntax(source, &lexed.tokens);
    assert_eq!(
        parsed.syntax.reconstruct(source),
        source.text(),
        "lossless reconstruction failed for {label}"
    );
    assert_eq!(
        parsed.syntax.root().token_count(),
        lexed.tokens.len(),
        "token retention failed for {label}"
    );

    let lowered = lower_syntax_tree(source, &parsed.syntax);
    assert!(
        lowered.diagnostics.is_empty(),
        "lowering invariant failed for {label}: {:?}\nsource: `{}`",
        lowered.diagnostics,
        source.text().escape_debug()
    );
    let _ = parsed.syntax.pretty(source);
    let _ = parsed.syntax.pretty_nodes();
}

fn next(state: &mut u64) -> u64 {
    let mut value = *state;
    value ^= value << 13;
    value ^= value >> 7;
    value ^= value << 17;
    *state = value;
    value
}
