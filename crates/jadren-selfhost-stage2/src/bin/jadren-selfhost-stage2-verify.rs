use std::{env, fs, process};

use jadren_selfhost_stage2::{decode_stage2_capture, import_stage2_jir};
use jadren_source::SourceManager;

fn main() {
    let capture_path = match env::args().nth(1) {
        Some(path) => path,
        None => fail("usage: jadren-selfhost-stage2-verify <capture>"),
    };
    let capture_bytes = match fs::read(&capture_path) {
        Ok(bytes) => bytes,
        Err(error) => fail(format!("cannot read capture `{capture_path}`: {error}")),
    };
    let capture = match decode_stage2_capture(&capture_bytes) {
        Ok(capture) => capture,
        Err(error) => fail(format!("invalid stage-2 capture: {error}")),
    };
    let mut sources = SourceManager::new();
    let source_id = match sources.add(&capture_path, capture.source.clone()) {
        Ok(source_id) => source_id,
        Err(error) => fail(format!("cannot register capture source: {error}")),
    };
    let module = match import_stage2_jir(
        &capture.source,
        source_id,
        capture.summary,
        &capture.records,
    ) {
        Ok(module) => module,
        Err(error) => fail(format!("strict stage-2 import failed: {error}")),
    };
    let metadata_records = capture
        .records
        .iter()
        .filter(|record| record.kind == 6)
        .count();
    if !(1..=3).contains(&metadata_records) || module.functions.len() != 1 {
        fail(format!(
            "unexpected imported local module: functions={}, metadata_records={metadata_records}",
            module.functions.len()
        ));
    }
    println!(
        "JADREN_LOCAL_CAPTURE_VERIFY pass functions={} records={} metadata_records={} status={}",
        module.functions.len(),
        capture.records.len(),
        metadata_records,
        capture.summary.status_flags
    );
}

fn fail(message: impl AsRef<str>) -> ! {
    eprintln!("{}", message.as_ref());
    process::exit(1);
}
