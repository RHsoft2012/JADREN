use std::{env, fs, process};

use jadren_selfhost_oracle::{parse_corpus, run_oracle};

fn main() {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "examples/selfhost/token_counter.corpus".to_owned());
    let input = match fs::read_to_string(&path) {
        Ok(input) => input,
        Err(error) => fail(format!("failed to read corpus `{path}`: {error}")),
    };
    let corpus = match parse_corpus(&input) {
        Ok(corpus) => corpus,
        Err(error) => fail(format!("invalid corpus `{path}`: {error}")),
    };
    let report = match run_oracle(&corpus) {
        Ok(report) => report,
        Err(error) => fail(format!("oracle mismatch for `{path}`: {error}")),
    };
    println!(
        "{{\"schema\":\"jadren-selfhost-oracle-0.1\",\"result\":\"passed\",\"corpus\":\"{}\",\"api_schema\":\"{}\",\"api_version\":{},\"api_lifetime\":\"{}\",\"api_registry\":\"{}\",\"byte_cases\":{},\"span_cases\":{},\"api_span_cases\":{},\"scan_cases\":{},\"api_scan_cases\":{},\"token_cases\":{},\"token_info_cases\":{},\"api_token_info_cases\":{},\"count_cases\":{},\"typed_literal_cases\":{},\"typed_binary_cases\":{},\"typed_cast_cases\":{},\"typed_unary_cases\":{},\"call_cases\":{},\"typed_name_cases\":{},\"typed_call_cases\":{},\"typed_call_resolve_cases\":{},\"typed_region_name_cases\":{},\"diagnostic_cases\":{},\"api_diagnostic_cases\":{},\"fingerprint\":\"{}\"}}",
        escape_json(&path),
        escape_json(report.api_schema),
        report.api_version,
        report.api_lifetime,
        report.api_registry,
        report.byte_cases,
        report.span_cases,
        report.api_span_cases,
        report.scan_cases,
        report.api_scan_cases,
        report.token_cases,
        report.token_info_cases,
        report.api_token_info_cases,
        report.count_cases,
        report.typed_literal_cases,
        report.typed_binary_cases,
        report.typed_cast_cases,
        report.typed_unary_cases,
        report.call_cases,
        report.typed_name_cases,
        report.typed_call_cases,
        report.typed_call_resolve_cases,
        report.typed_region_name_cases,
        report.diagnostic_cases,
        report.api_diagnostic_cases,
        report.fingerprint
    );
}

fn escape_json(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn fail(message: String) -> ! {
    eprintln!("{message}");
    process::exit(1);
}
