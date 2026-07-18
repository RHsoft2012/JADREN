use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_jadren")
}

#[test]
fn compile_fail_memory_effect_suite() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/invalid/memory-effects");
    for (file, code) in [
        ("uninitialized.jdn", "J0500"),
        ("use-after-move.jdn", "J0501"),
        ("borrow-conflict.jdn", "J0503"),
        ("borrow-escape.jdn", "J0505"),
        ("region-escape.jdn", "J0507"),
        ("noalloc-allocation.jdn", "J0600"),
        ("realtime-blocking.jdn", "J0611"),
        ("compute-string.jdn", "J0625"),
    ] {
        let output = Command::new(binary())
            .arg("check")
            .arg(root.join(file))
            .args(["--format", "json"])
            .output()
            .expect("jadren should start");
        assert_eq!(output.status.code(), Some(1), "{file}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains(&format!("\"code\":\"{code}\"")),
            "{file} did not report {code}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn prints_version() {
    let output = Command::new(binary())
        .arg("version")
        .output()
        .expect("jadren should start");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("jadren 0.1.0"));
}

#[test]
fn checks_hello_world() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/hello.jdn");
    let output = Command::new(binary())
        .arg("check")
        .arg(path)
        .output()
        .expect("jadren should start");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("syntax tokens"));
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 top-level items"));
}

#[test]
fn formats_and_checks_canonical_source() {
    let path = std::env::temp_dir().join(format!(
        "jadren-format-{}-{}.jdn",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::write(&path, "module test;fn main(){let x=1+2;return x;}")
        .expect("temporary source should be writable");

    let output = Command::new(binary())
        .args(["format"])
        .arg(&path)
        .output()
        .expect("jadren should start");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("fn main() {\n    let x = 1 + 2;"));

    let write = Command::new(binary())
        .args(["format"])
        .arg(&path)
        .arg("--write")
        .output()
        .expect("jadren should start");
    assert!(write.status.success());

    let check = Command::new(binary())
        .args(["format"])
        .arg(&path)
        .arg("--check")
        .output()
        .expect("jadren should start");
    let _ = fs::remove_file(path);
    assert!(check.status.success());
}

#[test]
fn emits_hello_world_ast() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/hello.jdn");
    let output = Command::new(binary())
        .args(["emit", "ast"])
        .arg(path)
        .output()
        .expect("jadren should start");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Function("));
    assert!(stdout.contains("text: \"main\""));
}

#[test]
fn emits_deterministic_c_header() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/ffi-export.jdn");
    let output = Command::new(binary())
        .args(["emit", "header"])
        .arg(path)
        .output()
        .expect("jadren should start");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("typedef struct examples_ffi_Vec3"));
    assert!(stdout.contains("int32_t jadren_add(int32_t a, int32_t b);"));
}

#[test]
fn emits_internal_csharp_dllimport() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/ffi-export.jdn");
    let output = Command::new(binary())
        .args(["emit", "csharp"])
        .arg(path)
        .output()
        .expect("jadren should start");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("DllImport(\"jadren_native\""));
    assert!(stdout.contains("internal static extern int jadren_add(int a, int b);"));
}

#[test]
fn emits_safe_csharp_facade() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/ffi-export.jdn");
    let output = Command::new(binary())
        .args(["emit", "facade"])
        .arg(path)
        .output()
        .expect("jadren should start");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("JadrenSliceView"));
    assert!(stdout.contains("public static int jadren_add(int a, int b)"));
    assert!(stdout.contains("IDisposable"));
}

#[test]
fn emits_c_layout_static_asserts() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/ffi-export.jdn");
    let output = Command::new(binary())
        .args(["emit", "abi-tests"])
        .arg(path)
        .output()
        .expect("jadren should start");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("#include \"generated.h\""));
    assert!(stdout.contains("sizeof(examples_ffi_Vec3) == 12u"));
    assert!(stdout.contains("offsetof(examples_ffi_Vec3, z) == 8u"));
}

#[test]
fn emits_verified_hello_world_hir() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/hello.jdn");
    let output = Command::new(binary())
        .args(["emit", "hir"])
        .arg(path)
        .output()
        .expect("jadren should start");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("HirModule"));
    assert!(stdout.contains("name: \"main\""));
    assert!(stdout.contains("Literal("));
}

#[test]
fn emits_verified_hello_world_mir() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/hello.jdn");
    let output = Command::new(binary())
        .args(["emit", "mir"])
        .arg(path)
        .output()
        .expect("jadren should start");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("MirModule"));
    assert!(stdout.contains("name: \"main\""));
    assert!(stdout.contains("blocks:"));
}

#[test]
fn emits_verified_native_jir() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/native-add.jdn");
    let output = Command::new(binary())
        .args(["emit", "jir"])
        .arg(path)
        .output()
        .expect("jadren should start");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("jir 0.1\n"), "{stdout}");
    assert!(stdout.contains(" = add "), "{stdout}");
    assert!(stdout.contains("return %v"), "{stdout}");
}

#[test]
fn emits_release_optimization_remarks() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/native-add.jdn");
    let output = Command::new(binary())
        .args(["emit", "remarks"])
        .arg(path)
        .output()
        .expect("jadren should start");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("fold-constants.1 folded="), "{stdout}");
    assert!(
        stdout.contains("loop-canonicalize-licm.2 folded="),
        "{stdout}"
    );
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
#[test]
fn emits_verified_native_llvm_ir() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/native-add.jdn");
    let output = Command::new(binary())
        .args(["emit", "llvm"])
        .arg(path)
        .output()
        .expect("jadren should start");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.starts_with("; ModuleID = 'native-add'\n"),
        "{stdout}"
    );
    let expected_triple = if cfg!(target_os = "windows") {
        "x86_64-pc-windows-msvc"
    } else {
        "x86_64-unknown-linux-gnu"
    };
    assert!(
        stdout.contains(&format!("target triple = \"{expected_triple}\"")),
        "{stdout}"
    );
    assert!(
        stdout.contains("define internal i32 @jadren.f0.add_values"),
        "{stdout}"
    );
}

#[test]
fn emits_reproducible_native_assembly_without_nul() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/native-add.jdn");
    let first = Command::new(binary())
        .args(["emit", "asm"])
        .arg(&path)
        .output()
        .expect("jadren should start");
    let second = Command::new(binary())
        .args(["emit", "asm"])
        .arg(path)
        .output()
        .expect("jadren should start");
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(first.stdout, second.stdout);
    assert!(!first.stdout.contains(&0));
    let stdout = String::from_utf8_lossy(&first.stdout);
    assert!(stdout.contains("jadren.f0.add_values:"), "{stdout}");
    assert!(stdout.contains("addl"), "{stdout}");
}

#[cfg(windows)]
#[test]
fn emits_windows_coff_object_to_explicit_path() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/native-add.jdn");
    let output_path =
        std::env::temp_dir().join(format!("jadren-object-{}.obj", std::process::id()));
    let output = Command::new(binary())
        .args(["emit", "object"])
        .arg(&source)
        .arg(&output_path)
        .output()
        .expect("jadren should start");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    let bytes = fs::read(&output_path).expect("COFF object should be written");
    assert!(bytes.len() > 256);
    fs::remove_file(output_path).expect("test object cleanup");
}

#[cfg(windows)]
#[test]
fn emits_aarch64_android_object_with_explicit_target() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/native-add.jdn");
    let output_path = std::env::temp_dir().join(format!(
        "jadren-aarch64-android-cli-{}.o",
        std::process::id()
    ));
    let output = Command::new(binary())
        .args(["emit", "object"])
        .arg(&source)
        .arg(&output_path)
        .args(["--target", "aarch64-unknown-linux-android24"])
        .output()
        .expect("jadren should start");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    let bytes = fs::read(&output_path).expect("AArch64 ELF object should be written");
    assert_eq!(&bytes[..4], b"\x7fELF");
    let llvm_prefix = std::env::var("JADREN_LLVM_PREFIX")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../Toolchains/LLVM-22.1.8")
        });
    let readobj = Command::new(llvm_prefix.join("bin/llvm-readobj.exe"))
        .args(["--file-headers"])
        .arg(&output_path)
        .output()
        .expect("llvm-readobj should start");
    assert!(
        readobj.status.success(),
        "{}",
        String::from_utf8_lossy(&readobj.stderr)
    );
    let inspection = String::from_utf8_lossy(&readobj.stdout);
    assert!(
        inspection.contains("Machine: EM_AARCH64") || inspection.contains("Machine: AArch64"),
        "{inspection}"
    );
    fs::remove_file(output_path).expect("test object cleanup");
}

#[cfg(target_os = "linux")]
#[test]
fn emits_linux_elf_object_with_explicit_target() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/native-add.jdn");
    let output_path =
        std::env::temp_dir().join(format!("jadren-linux-object-{}.o", std::process::id()));
    let output = Command::new(binary())
        .args(["emit", "object"])
        .arg(&source)
        .arg(&output_path)
        .args(["--target", "x86_64-unknown-linux-gnu"])
        .output()
        .expect("jadren should start");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    let bytes = fs::read(&output_path).expect("ELF object should be written");
    assert!(bytes.len() > 256);
    assert_eq!(&bytes[..4], b"\x7fELF");
    fs::remove_file(output_path).expect("test object cleanup");
}

#[test]
fn native_emit_rejects_invalid_source_before_codegen() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/invalid/invalid-character.jdn");
    let output = Command::new(binary())
        .args(["emit", "llvm"])
        .arg(path)
        .output()
        .expect("jadren should start");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("error[J0001]"));
    assert!(output.stdout.is_empty());
}

#[test]
fn emits_inferred_hello_world_effects() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/hello.jdn");
    let output = Command::new(binary())
        .args(["emit", "effects"])
        .arg(path)
        .output()
        .expect("jadren should start");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("main: IO"));
}

#[test]
fn emits_lossless_parser_tour_syntax() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/parser-tour.jdn");
    let output = Command::new(binary())
        .args(["emit", "syntax"])
        .arg(path)
        .output()
        .expect("jadren should start");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("Root "));
    assert!(stdout.contains("StructDeclaration"));
    assert!(stdout.contains("MatchExpression"));
    assert!(stdout.contains("Whitespace"));
}

#[test]
fn checks_parser_tour() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/parser-tour.jdn");
    let output = Command::new(binary())
        .arg("check")
        .arg(path)
        .output()
        .expect("jadren should start");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("4 top-level items"));
}

#[test]
fn checks_with_explicit_canonical_target() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/hello.jdn");
    let output = Command::new(binary())
        .arg("check")
        .arg(path)
        .args(["--target", "X86_64-PC-WINDOWS-MSVC", "--warnings-as-errors"])
        .output()
        .expect("jadren should start");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn doctor_reports_target_and_deterministic_config() {
    let output = Command::new(binary())
        .arg("doctor")
        .output()
        .expect("jadren should start");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("host target:"));
    assert!(stdout.contains("config fingerprint:"));
    assert!(stdout.contains("deterministic ordering: enabled"));
    assert!(stdout.contains("LLVM toolchain: 22.1.8 verified"));
    assert!(stdout.contains(
            "runtime ABI 0.10 system+region allocators, abort panic boundary, callbacks, Buffer/Slice, UTF-8 String, math scalar, vector value and quaternion Slerp core available"
    ));
}

#[test]
fn rejects_invalid_source_with_json_diagnostic() {
    let path = std::env::temp_dir().join(format!(
        "jadren-invalid-{}-{}.jdn",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::write(&path, "fn main() { ľ }").expect("temporary source should be writable");

    let output = Command::new(binary())
        .arg("check")
        .arg(&path)
        .args(["--format", "json"])
        .output()
        .expect("jadren should start");
    let _ = fs::remove_file(path);

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"code\":\"J0001\""));
    assert!(stdout.starts_with("[\n"));
}

#[test]
fn emits_one_json_document_for_parser_errors() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/invalid/missing-closing-brace.jdn");
    let output = Command::new(binary())
        .arg("check")
        .arg(path)
        .args(["--format", "json"])
        .output()
        .expect("jadren should start");

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"code\":\"J0104\""));
    assert_eq!(stdout.matches("[\n").count(), 1);
    assert_eq!(stdout.matches("\n]\n").count(), 1);
}

#[test]
fn reports_duplicate_local_from_resolver() {
    let path = std::env::temp_dir().join(format!(
        "jadren-duplicate-{}-{}.jdn",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::write(
        &path,
        "fn main() { let value = 1; let value = 2; print(value) }",
    )
    .expect("temporary source should be writable");

    let output = Command::new(binary())
        .arg("check")
        .arg(&path)
        .args(["--format", "json"])
        .output()
        .expect("jadren should start");
    let _ = fs::remove_file(path);

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"code\":\"J0200\""));
    assert!(stdout.contains("duplicate definition of `value`"));
}

#[test]
fn reports_unresolved_import_from_module_resolver() {
    let path = std::env::temp_dir().join(format!(
        "jadren-import-{}-{}.jdn",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::write(&path, "module app; import missing.Value; fn main() {}")
        .expect("temporary source should be writable");

    let output = Command::new(binary())
        .arg("check")
        .arg(&path)
        .args(["--format", "json"])
        .output()
        .expect("jadren should start");
    let _ = fs::remove_file(path);

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"code\":\"J0202\""));
    assert!(stdout.contains("unresolved import `missing.Value`"));
}

#[test]
fn reports_local_type_mismatch() {
    let path = std::env::temp_dir().join(format!(
        "jadren-type-mismatch-{}-{}.jdn",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::write(&path, "module test; fn main() { let value: Int32 = true }")
        .expect("temporary source should be writable");

    let output = Command::new(binary())
        .arg("check")
        .arg(&path)
        .args(["--format", "json"])
        .output()
        .expect("jadren should start");
    let _ = fs::remove_file(path);

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"code\":\"J0301\""));
}

#[test]
fn reports_function_call_arity_mismatch() {
    let path = std::env::temp_dir().join(format!(
        "jadren-call-arity-{}-{}.jdn",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::write(
        &path,
        "module test; fn main() { add(1) } fn add(a: Int32, b: Int32) -> Int32 { return a + b }",
    )
    .expect("temporary source should be writable");

    let output = Command::new(binary())
        .arg("check")
        .arg(&path)
        .args(["--format", "json"])
        .output()
        .expect("jadren should start");
    let _ = fs::remove_file(path);

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"code\":\"J0304\""));
}

#[test]
fn reports_missing_record_field() {
    let path = std::env::temp_dir().join(format!(
        "jadren-record-field-{}-{}.jdn",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::write(
        &path,
        "module test; struct Point { x: Int32 } fn main() { let point = Point {} }",
    )
    .expect("temporary source should be writable");

    let output = Command::new(binary())
        .arg("check")
        .arg(&path)
        .args(["--format", "json"])
        .output()
        .expect("jadren should start");
    let _ = fs::remove_file(path);

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"code\":\"J0308\""));
}

#[test]
fn reports_non_exhaustive_enum_match() {
    let path = std::env::temp_dir().join(format!(
        "jadren-enum-match-{}-{}.jdn",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::write(
        &path,
        "module test; enum Choice { First, Second } fn choose(value: Choice) -> Int32 { return match value { First => 0 } }",
    )
    .expect("temporary source should be writable");

    let output = Command::new(binary())
        .arg("check")
        .arg(&path)
        .args(["--format", "json"])
        .output()
        .expect("jadren should start");
    let _ = fs::remove_file(path);

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"code\":\"J0311\""));
}

#[test]
fn reports_invalid_try_propagation_context() {
    let path = std::env::temp_dir().join(format!(
        "jadren-invalid-try-{}-{}.jdn",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::write(&path, "module test; fn bad() -> Int32 { return Some(1)? }")
        .expect("temporary source should be writable");

    let output = Command::new(binary())
        .arg("check")
        .arg(&path)
        .args(["--format", "json"])
        .output()
        .expect("jadren should start");
    let _ = fs::remove_file(path);

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"code\":\"J0313\""));
}

#[test]
fn reports_unsatisfied_generic_trait_bound() {
    let path = std::env::temp_dir().join(format!(
        "jadren-invalid-bound-{}-{}.jdn",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::write(
        &path,
        "module test; fn numeric<T: Numeric>(value: T) -> T { return value } fn main() { numeric(true) }",
    )
    .expect("temporary source should be writable");

    let output = Command::new(binary())
        .arg("check")
        .arg(&path)
        .args(["--format", "json"])
        .output()
        .expect("jadren should start");
    let _ = fs::remove_file(path);

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"code\":\"J0316\""));
}

#[test]
fn reports_uninitialized_and_moved_place_uses() {
    for (name, source, code) in [
        (
            "uninitialized",
            "module test; fn main() { let value: Int32; print(value) }",
            "J0500",
        ),
        (
            "moved",
            "module test; fn consume(data: Buffer<Int32>) { let first = data; print(data) }",
            "J0501",
        ),
    ] {
        let path = std::env::temp_dir().join(format!(
            "jadren-{name}-{}-{}.jdn",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        fs::write(&path, source).expect("temporary source should be writable");
        let output = Command::new(binary())
            .arg("check")
            .arg(&path)
            .args(["--format", "json"])
            .output()
            .expect("jadren should start");
        let _ = fs::remove_file(path);

        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stdout).contains(&format!("\"code\":\"{code}\"")));
    }
}

#[test]
fn reports_overlapping_read_write_borrow() {
    let path = std::env::temp_dir().join(format!(
        "jadren-borrow-conflict-{}-{}.jdn",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::write(
        &path,
        "module test; fn update(first: read Buffer<Int32>, second: write Buffer<Int32>) {} fn run(data: Buffer<Int32>) { update(data, data) }",
    )
    .expect("temporary source should be writable");
    let output = Command::new(binary())
        .arg("check")
        .arg(&path)
        .args(["--format", "json"])
        .output()
        .expect("jadren should start");
    let _ = fs::remove_file(path);

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"code\":\"J0503\""));
}

#[test]
fn reports_borrow_escaping_its_owner() {
    let path = std::env::temp_dir().join(format!(
        "jadren-borrow-escape-{}-{}.jdn",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::write(
        &path,
        "module test; fn borrow(data: Buffer<Int32>) -> read Buffer<Int32> { let view: read Buffer<Int32> = data; return view }",
    )
    .expect("temporary source should be writable");
    let output = Command::new(binary())
        .arg("check")
        .arg(&path)
        .args(["--format", "json"])
        .output()
        .expect("jadren should start");
    let _ = fs::remove_file(path);

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"code\":\"J0505\""));
}

#[test]
fn reports_region_owned_value_escaping_its_region() {
    let path = std::env::temp_dir().join(format!(
        "jadren-region-escape-{}-{}.jdn",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::write(
        &path,
        "module test; fn leak() -> Buffer<Int32> { region frame { let values: Buffer<Int32> = frame.allocate(4); return values } }",
    )
    .expect("temporary source should be writable");
    let output = Command::new(binary())
        .arg("check")
        .arg(&path)
        .args(["--format", "json"])
        .output()
        .expect("jadren should start");
    let _ = fs::remove_file(path);

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"code\":\"J0507\""));
}

#[test]
fn reports_transitive_allocation_from_noalloc_function() {
    let path = std::env::temp_dir().join(format!(
        "jadren-noalloc-{}-{}.jdn",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::write(
        &path,
        "module test; fn allocate() { region frame { let values: Buffer<Int32> = frame.allocate(4) } } @noalloc fn update() { allocate() }",
    )
    .expect("temporary source should be writable");
    let output = Command::new(binary())
        .arg("check")
        .arg(&path)
        .args(["--format", "json"])
        .output()
        .expect("jadren should start");
    let _ = fs::remove_file(path);

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"code\":\"J0600\""));
}

#[test]
fn reports_blocking_effect_in_realtime_function() {
    let path = std::env::temp_dir().join(format!(
        "jadren-realtime-{}-{}.jdn",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::write(
        &path,
        "module test; @realtime fn update(value: Int32) { print(value) }",
    )
    .expect("temporary source should be writable");
    let output = Command::new(binary())
        .arg("check")
        .arg(&path)
        .args(["--format", "json"])
        .output()
        .expect("jadren should start");
    let _ = fs::remove_file(path);

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"code\":\"J0611\""));
}

#[test]
fn reports_unsupported_compute_signature() {
    let path = std::env::temp_dir().join(format!(
        "jadren-compute-{}-{}.jdn",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::write(&path, "module test; @compute fn kernel(value: String) {}")
        .expect("temporary source should be writable");
    let output = Command::new(binary())
        .arg("check")
        .arg(&path)
        .args(["--format", "json"])
        .output()
        .expect("jadren should start");
    let _ = fs::remove_file(path);

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"code\":\"J0625\""));
}
