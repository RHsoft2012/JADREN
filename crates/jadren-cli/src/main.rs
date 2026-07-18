//! Initial command-line driver for the Jadren compiler.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::str::FromStr;

use jadren_diagnostics::{render_json, render_text};
use jadren_driver::{
    BuildProfile, CompilerConfig, CompilerSession, DiagnosticFormat, Edition, TargetTriple,
};
use jadren_format::format_source;
use jadren_lexer::{TokenKind, lex};
use jadren_migration::plan_manifest;
use jadren_package::{
    LOCKFILE_FILE, MANIFEST_FILE, PackageLockfile, PackageManifest, resolve_local,
};
use jadren_parser::{parse, parse_syntax};
use jadren_release::{Provenance, Sbom};
use jadren_source::{SourceId, SourceManager};
use jadren_toolchain::{ArtifactDescriptor, install_file, verify_file};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const LLVM_VERSION: &str = "22.1.8";

fn main() -> ExitCode {
    match run(env::args_os().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(arguments: Vec<OsString>) -> Result<(), String> {
    let Some(command) = arguments.first().and_then(|value| value.to_str()) else {
        print_help();
        return Ok(());
    };

    match command {
        "version" | "--version" | "-V" => {
            println!("jadren {VERSION}");
            Ok(())
        }
        "doctor" => doctor(&arguments[1..]),
        "check" => check(&arguments[1..]),
        "build" => build_executable(&arguments[1..]),
        "run" => run_executable(&arguments[1..]),
        "test" => test_sources(&arguments[1..]),
        "doc" => generate_docs(&arguments[1..]),
        "init" => init_package(&arguments[1..]),
        "lock" => lock_package(&arguments[1..]),
        "resolve" => resolve_package(&arguments[1..]),
        "toolchain" => toolchain(&arguments[1..]),
        "sbom" => generate_sbom(&arguments[1..]),
        "provenance" => generate_provenance(&arguments[1..]),
        "migrate" => migrate_package(&arguments[1..]),
        "format" => format_file(&arguments[1..]),
        "lsp" => lsp(&arguments[1..]),
        "emit" => emit(&arguments[1..]),
        "help" | "--help" | "-h" => {
            print_help();
            Ok(())
        }
        unknown => Err(format!(
            "unknown command `{unknown}`; run `jadren help` for usage"
        )),
    }
}

fn doctor(arguments: &[OsString]) -> Result<(), String> {
    if !arguments.is_empty() {
        return Err("`jadren doctor` does not accept arguments yet".to_owned());
    }
    println!("Jadren compiler: {VERSION}");
    println!("language specification: 0.1-draft");
    let config = CompilerConfig::default();
    println!("host target: {}", config.target);
    println!("config fingerprint: {}", config.semantic_fingerprint());
    println!("source encoding: UTF-8");
    println!(
        "frontend: lexer + CST + AST + resolver + inference + generics + verified HIR/MIR + effect policies + compute eligibility + memory/drop/region analysis available"
    );
    println!("deterministic ordering: enabled");
    let llvm_prefix = verify_llvm_toolchain()?;
    println!(
        "LLVM toolchain: {LLVM_VERSION} verified at {}",
        llvm_prefix.display()
    );
    println!(
        "JIR model: LLVM/COFF/ELF/link/debug/emit/differential pipeline and runtime ABI 0.10 system+region allocators, abort panic boundary, callbacks, Buffer/Slice, UTF-8 String, math scalar, vector value and quaternion Slerp core available"
    );
    println!(
        "native backend: host x86-64 object/assembly emission; Linux hosts emit ELF and Windows hosts emit COFF"
    );
    Ok(())
}

fn verify_llvm_toolchain() -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        let prefix = env::var_os("JADREN_LLVM_PREFIX")
            .map(PathBuf::from)
            .unwrap_or_else(default_llvm_prefix);
        for required in [
            prefix.join("bin").join("clang.exe"),
            prefix.join("bin").join("lld-link.exe"),
            prefix.join("bin").join("llvm-dlltool.exe"),
            prefix.join("bin").join("LLVM-C.dll"),
            prefix.join("lib").join("LLVM-C.lib"),
        ] {
            if !required.is_file() {
                return Err(format!(
                    "LLVM {LLVM_VERSION} toolchain is incomplete at `{}`; run `scripts/bootstrap-llvm.ps1` or set JADREN_LLVM_PREFIX",
                    prefix.display()
                ));
            }
        }
        let output = Command::new(prefix.join("bin").join("clang.exe"))
            .arg("--version")
            .output()
            .map_err(|error| format!("failed to execute pinned clang: {error}"))?;
        let version = String::from_utf8_lossy(&output.stdout);
        if !output.status.success()
            || !version.starts_with(&format!("clang version {LLVM_VERSION}"))
        {
            return Err(format!(
                "expected LLVM {LLVM_VERSION} at `{}`, got `{}`",
                prefix.display(),
                version.lines().next().unwrap_or("no version output")
            ));
        }
        Ok(prefix)
    }

    #[cfg(target_os = "linux")]
    {
        let llvm_config = locate_linux_llvm_tool("llvm-config")
            .or_else(|| locate_linux_llvm_tool("llvm-config-22"))
            .ok_or_else(|| {
                "LLVM 22.1.8 is unavailable; install the pinned Linux toolchain or set JADREN_LLVM_PREFIX/LLVM_SYS_221_PREFIX".to_owned()
            })?;
        let bin = llvm_config
            .parent()
            .ok_or_else(|| "llvm-config has no parent directory".to_owned())?;
        let prefix = bin
            .parent()
            .ok_or_else(|| "llvm-config bin directory has no prefix".to_owned())?
            .to_owned();
        for name in ["clang", "llvm-readobj", "llvm-strings"] {
            let tool = bin.join(name);
            if !tool.is_file() {
                return Err(format!(
                    "LLVM {LLVM_VERSION} toolchain is incomplete at `{}`; missing `{}`",
                    prefix.display(),
                    tool.display()
                ));
            }
        }
        let version = run_tool_output(&llvm_config, &["--version"])?;
        if version.split_whitespace().next() != Some(LLVM_VERSION) {
            return Err(format!(
                "expected LLVM {LLVM_VERSION} at `{}`, got `{}`",
                prefix.display(),
                version.lines().next().unwrap_or("no version output")
            ));
        }
        let libdir = run_tool_output(&llvm_config, &["--libdir"])?;
        if !Path::new(libdir.trim()).is_dir() {
            return Err(format!(
                "LLVM {LLVM_VERSION} library directory is missing: `{}`",
                libdir.trim()
            ));
        }
        Ok(prefix)
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Err("LLVM native emission is not enabled on this host platform".to_owned())
    }
}

#[cfg(target_os = "linux")]
fn locate_linux_llvm_tool(name: &str) -> Option<PathBuf> {
    for variable in ["JADREN_LLVM_PREFIX", "LLVM_SYS_221_PREFIX"] {
        if let Some(prefix) = env::var_os(variable) {
            let candidate = PathBuf::from(prefix).join("bin").join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    env::var_os("PATH").and_then(|path| {
        env::split_paths(&path)
            .map(|directory| directory.join(name))
            .find(|candidate| candidate.is_file())
    })
}

#[cfg(target_os = "linux")]
fn run_tool_output(path: &Path, arguments: &[&str]) -> Result<String, String> {
    let output = Command::new(path)
        .args(arguments)
        .output()
        .map_err(|error| format!("failed to execute `{}`: {error}", path.display()))?;
    if !output.status.success() {
        return Err(format!(
            "`{}` failed: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[cfg(windows)]
fn default_llvm_prefix() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("CLI crate must be inside workspace/crates")
        .join("Toolchains")
        .join(format!("LLVM-{LLVM_VERSION}"))
}

fn check(arguments: &[OsString]) -> Result<(), String> {
    let (path, config) = parse_check_arguments(arguments)?;
    let format = config.diagnostic_format;
    let text = read_utf8(&path)?;
    let mut session = CompilerSession::new(config);
    let source_id = session
        .add_source(&path, text)
        .map_err(|error| format!("failed to register `{}`: {error}", path.display()))?;
    let output = session
        .check(source_id)
        .map_err(|error| format!("failed to check `{}`: {error}", path.display()))?;

    render_diagnostics(&output.diagnostics, session.sources(), format);
    if output.has_errors() {
        std::process::exit(1);
    }

    let syntax_tokens = output
        .tokens
        .iter()
        .filter(|token| !token.kind.is_trivia() && token.kind != TokenKind::Eof)
        .count();
    if format == DiagnosticFormat::Text {
        println!(
            "checked {}: {syntax_tokens} syntax tokens, {} top-level items",
            path.display(),
            output.top_level_item_count().unwrap_or_default()
        );
    }
    Ok(())
}

fn test_sources(arguments: &[OsString]) -> Result<(), String> {
    let (path, config) = parse_test_arguments(arguments)?;
    let files = collect_jadren_files(&path)?;
    let mut session = CompilerSession::new(config.clone());
    let mut source_ids = Vec::with_capacity(files.len());
    for file in &files {
        let text = read_utf8(file)?;
        let source_id = session
            .add_source(file, text)
            .map_err(|error| format!("failed to register `{}`: {error}", file.display()))?;
        source_ids.push(source_id);
    }

    let mut results = Vec::with_capacity(source_ids.len());
    for (file, source_id) in files.iter().zip(source_ids) {
        let output = session
            .check(source_id)
            .map_err(|error| format!("failed to check `{}`: {error}", file.display()))?;
        results.push((file, output));
    }

    let failed = results
        .iter()
        .filter(|(_, output)| output.has_errors())
        .count();
    match config.diagnostic_format {
        DiagnosticFormat::Text => {
            for (file, output) in &results {
                if output.has_errors() {
                    render_diagnostics(
                        &output.diagnostics,
                        session.sources(),
                        DiagnosticFormat::Text,
                    );
                    println!("FAIL {}", file.display());
                } else {
                    println!("PASS {}", file.display());
                }
            }
            println!(
                "test result: {}. {} passed; {} failed; {} total",
                if failed == 0 { "ok" } else { "FAILED" },
                results.len() - failed,
                failed,
                results.len()
            );
        }
        DiagnosticFormat::Json => {
            print_test_json(&results, session.sources(), failed);
        }
    }
    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn generate_docs(arguments: &[OsString]) -> Result<(), String> {
    let (path, output_path, config) = parse_doc_arguments(arguments)?;
    let files = collect_jadren_files(&path)?;
    let mut session = CompilerSession::new(config);
    let mut source_ids = Vec::with_capacity(files.len());
    for file in &files {
        let text = read_utf8(file)?;
        let source_id = session
            .add_source(file, text)
            .map_err(|error| format!("failed to register `{}`: {error}", file.display()))?;
        source_ids.push(source_id);
    }

    let mut results = Vec::with_capacity(source_ids.len());
    for (file, source_id) in files.iter().zip(source_ids) {
        let output = session
            .check(source_id)
            .map_err(|error| format!("failed to check `{}`: {error}", file.display()))?;
        results.push((file, output));
    }
    let failed = results
        .iter()
        .filter(|(_, output)| output.has_errors())
        .count();
    if failed > 0 {
        for (_, output) in &results {
            render_diagnostics(
                &output.diagnostics,
                session.sources(),
                DiagnosticFormat::Text,
            );
        }
        std::process::exit(1);
    }

    let markdown = render_api_markdown(&results, session.sources());
    fs::write(&output_path, markdown)
        .map_err(|error| format!("failed to write `{}`: {error}", output_path.display()))?;
    println!(
        "generated {} from {} source file(s)",
        output_path.display(),
        results.len()
    );
    Ok(())
}

fn render_api_markdown(
    results: &[(&PathBuf, jadren_driver::CheckOutput)],
    sources: &SourceManager,
) -> String {
    let mut markdown = String::from("# Jadren API\n\nGenerated by `jadren doc` (schema 0.1).\n");
    for (file, output) in results {
        let Some(artifacts) = output.artifacts.as_ref() else {
            continue;
        };
        let Some(source) = sources.get(output.source) else {
            continue;
        };
        let module = artifacts
            .ast
            .module
            .as_ref()
            .and_then(|module| source.slice(module.span))
            .unwrap_or("<root>");
        markdown.push_str(&format!(
            "\n## Module `{}`\n\n<!-- source: {} -->\n",
            markdown_inline(module),
            markdown_inline(&file.display().to_string())
        ));
        let mut declaration_count = 0;
        for item in &artifacts.ast.items {
            match item {
                jadren_parser::Item::Function(function) if function.is_public => {
                    declaration_count += 1;
                    markdown.push_str("\n### Function\n\n```jadren\n");
                    markdown.push_str(&function_signature(function, source));
                    markdown.push_str("\n```\n");
                    append_annotations(&mut markdown, &function.annotations, source);
                }
                jadren_parser::Item::Struct(record) | jadren_parser::Item::Component(record) => {
                    if !record.is_public {
                        continue;
                    }
                    declaration_count += 1;
                    let keyword = if matches!(item, jadren_parser::Item::Component(_)) {
                        "component"
                    } else {
                        "struct"
                    };
                    markdown.push_str(&format!(
                        "\n### {} `{}`\n\n```jadren\n{}\n```\n",
                        keyword,
                        markdown_inline(&record.name.text),
                        record_signature(record, source, keyword)
                    ));
                    append_annotations(&mut markdown, &record.annotations, source);
                }
                jadren_parser::Item::Enum(declaration) if declaration.is_public => {
                    declaration_count += 1;
                    markdown.push_str(&format!(
                        "\n### enum `{}`\n\n```jadren\n{}\n```\n",
                        markdown_inline(&declaration.name.text),
                        enum_signature(declaration, source)
                    ));
                    append_annotations(&mut markdown, &declaration.annotations, source);
                }
                jadren_parser::Item::ExternBlock(extern_block) => {
                    let public_functions = extern_block.functions.len();
                    if public_functions == 0 {
                        continue;
                    }
                    declaration_count += public_functions;
                    markdown.push_str(&format!(
                        "\n### extern `{}`\n\n",
                        markdown_inline(&extern_block.abi)
                    ));
                    for function in &extern_block.functions {
                        markdown.push_str("```jadren\n");
                        markdown.push_str(&extern_function_signature(function, source));
                        markdown.push_str("\n```\n");
                    }
                }
                _ => {}
            }
        }
        if declaration_count == 0 {
            markdown.push_str("\n_No public declarations._\n");
        }
    }
    markdown
}

fn init_package(arguments: &[OsString]) -> Result<(), String> {
    let mut directory = PathBuf::from(".");
    let mut directory_set = false;
    let mut package_name = None;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].to_str() {
            Some("--name") => {
                index += 1;
                package_name = Some(
                    arguments
                        .get(index)
                        .and_then(|value| value.to_str())
                        .ok_or_else(|| "`--name` requires a package identifier".to_owned())?
                        .to_owned(),
                );
            }
            Some(value) if value.starts_with('-') => {
                return Err(format!("unknown option `{value}`"));
            }
            _ if !directory_set => {
                directory = PathBuf::from(&arguments[index]);
                directory_set = true;
            }
            _ => return Err("`jadren init` accepts one directory path".to_owned()),
        }
        index += 1;
    }
    fs::create_dir_all(&directory)
        .map_err(|error| format!("failed to create `{}`: {error}", directory.display()))?;
    let manifest_path = directory.join(MANIFEST_FILE);
    let lock_path = directory.join(LOCKFILE_FILE);
    if manifest_path.exists() || lock_path.exists() {
        return Err(format!(
            "package already initialized at `{}`",
            directory.display()
        ));
    }
    let inferred_name = directory
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("jadren_project");
    let manifest = PackageManifest::starter(package_name.as_deref().unwrap_or(inferred_name))
        .map_err(|error| format!("invalid package name: {error}"))?;
    let lockfile = PackageLockfile::from_manifest(&manifest);
    fs::write(&manifest_path, manifest.to_toml())
        .map_err(|error| format!("failed to write `{}`: {error}", manifest_path.display()))?;
    fs::write(&lock_path, lockfile.to_toml())
        .map_err(|error| format!("failed to write `{}`: {error}", lock_path.display()))?;
    println!(
        "initialized package `{}` at {}",
        manifest.package.name,
        directory.display()
    );
    Ok(())
}

fn lock_package(arguments: &[OsString]) -> Result<(), String> {
    if arguments.len() > 1 {
        return Err("usage: jadren lock [directory|jadren.toml]".to_owned());
    }
    let input = arguments
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let manifest_path = if input.is_dir() {
        input.join(MANIFEST_FILE)
    } else {
        input
    };
    let text = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("failed to read `{}`: {error}", manifest_path.display()))?;
    let manifest = PackageManifest::parse(&text)
        .map_err(|error| format!("invalid `{}`: {error}", manifest_path.display()))?;
    let lock_path = manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(LOCKFILE_FILE);
    let lockfile = PackageLockfile::from_manifest(&manifest);
    fs::write(&lock_path, lockfile.to_toml())
        .map_err(|error| format!("failed to write `{}`: {error}", lock_path.display()))?;
    println!(
        "locked package `{}` -> {}",
        manifest.package.name,
        lock_path.display()
    );
    Ok(())
}

fn resolve_package(arguments: &[OsString]) -> Result<(), String> {
    if arguments.len() > 1 {
        return Err("usage: jadren resolve [directory|jadren.toml]".to_owned());
    }
    let input = arguments
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let resolution = resolve_local(&input).map_err(|error| error.to_string())?;
    println!(
        "resolved `{}` ({})",
        resolution.root.manifest.package.name,
        resolution.root.manifest_path.display()
    );
    for dependency in &resolution.dependencies {
        println!(
            "  {} {} ({})",
            dependency.manifest.package.name,
            dependency.manifest.package.version,
            dependency.manifest_path.display()
        );
    }
    Ok(())
}

fn toolchain(arguments: &[OsString]) -> Result<(), String> {
    let usage = "usage: jadren toolchain verify <manifest> <artifact> | install <manifest> <artifact> <root>";
    let Some(operation) = arguments.first().and_then(|value| value.to_str()) else {
        return Err(usage.to_owned());
    };
    let Some(manifest) = arguments.get(1).map(PathBuf::from) else {
        return Err(usage.to_owned());
    };
    let Some(artifact) = arguments.get(2).map(PathBuf::from) else {
        return Err(usage.to_owned());
    };
    let manifest_text = read_utf8(&manifest)?;
    let descriptor = ArtifactDescriptor::parse_manifest(&manifest_text)
        .map_err(|error| format!("failed to parse toolchain manifest: {error}"))?;
    match operation {
        "verify" if arguments.len() == 3 => {
            verify_file(&descriptor, &artifact)
                .map_err(|error| format!("toolchain verification failed: {error}"))?;
            println!(
                "verified {} {} for {} ({})",
                descriptor.name, descriptor.version, descriptor.target, descriptor.sha256
            );
            Ok(())
        }
        "install" if arguments.len() == 4 => {
            let root = arguments.get(3).expect("length checked");
            let installed = install_file(&descriptor, &artifact, Path::new(root))
                .map_err(|error| format!("toolchain installation failed: {error}"))?;
            println!("installed {}", installed.display());
            Ok(())
        }
        _ => Err(usage.to_owned()),
    }
}

fn generate_sbom(arguments: &[OsString]) -> Result<(), String> {
    let (lock_path, output_path) = parse_release_output_arguments(arguments, "sbom")?;
    let lock_text = read_utf8(&lock_path)?;
    let sbom = Sbom::from_cargo_lock(&lock_text).map_err(|error| error.to_string())?;
    let json = sbom.to_json();
    if let Some(output_path) = output_path {
        fs::write(&output_path, json)
            .map_err(|error| format!("failed to write {}: {error}", output_path.display()))?;
        println!("generated SBOM {}", output_path.display());
    } else {
        print!("{json}");
    }
    Ok(())
}

fn generate_provenance(arguments: &[OsString]) -> Result<(), String> {
    let (lock_path, output_path, source_fingerprint, toolchain) =
        parse_provenance_arguments(arguments)?;
    let lock_text = read_utf8(&lock_path)?;
    let sbom = Sbom::from_cargo_lock(&lock_text).map_err(|error| error.to_string())?;
    let provenance =
        Provenance::new(&sbom, source_fingerprint, toolchain).map_err(|error| error.to_string())?;
    let json = provenance.to_json();
    if let Some(output_path) = output_path {
        fs::write(&output_path, json)
            .map_err(|error| format!("failed to write {}: {error}", output_path.display()))?;
        println!("generated provenance {}", output_path.display());
    } else {
        print!("{json}");
    }
    Ok(())
}

fn migrate_package(arguments: &[OsString]) -> Result<(), String> {
    let (input, from, to, write) = parse_migrate_arguments(arguments)?;
    let manifest_path = if input.is_dir() {
        input.join(MANIFEST_FILE)
    } else {
        input
    };
    let source = read_utf8(&manifest_path)?;
    let plan = plan_manifest(&source, &from, &to).map_err(|error| error.to_string())?;
    if write && plan.changed() {
        fs::write(&manifest_path, &plan.output).map_err(|error| {
            format!(
                "failed to write migrated manifest {}: {error}",
                manifest_path.display()
            )
        })?;
    }
    println!(
        "{{\"schema\":\"jadren-migration-0.1\",\"path\":{},\"from\":{},\"to\":{},\"edits\":{},\"changed\":{},\"mode\":\"{}\",\"result\":\"{}\"}}",
        json_string(&manifest_path.display().to_string()),
        json_string(&plan.from),
        json_string(&plan.to),
        plan.edits.len(),
        plan.changed(),
        if write { "write" } else { "check" },
        if !plan.changed() || write {
            "pass"
        } else {
            "changes-required"
        },
    );
    if !write && plan.changed() {
        std::process::exit(1);
    }
    Ok(())
}

fn parse_migrate_arguments(
    arguments: &[OsString],
) -> Result<(PathBuf, String, String, bool), String> {
    let mut input = None;
    let mut from = None;
    let mut to = None;
    let mut write = false;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].to_str() {
            Some("--from") => {
                index += 1;
                let Some(value) = arguments.get(index).and_then(|value| value.to_str()) else {
                    return Err("--from requires an edition".to_owned());
                };
                from = Some(value.to_owned());
            }
            Some("--to") => {
                index += 1;
                let Some(value) = arguments.get(index).and_then(|value| value.to_str()) else {
                    return Err("--to requires an edition".to_owned());
                };
                to = Some(value.to_owned());
            }
            Some("--check") => write = false,
            Some("--write") => write = true,
            Some(value) if value.starts_with('-') => {
                return Err(format!("unknown option {value}"));
            }
            _ if input.is_none() => input = Some(PathBuf::from(&arguments[index])),
            _ => {
                return Err(
                    "usage: jadren migrate [directory|jadren.toml] --from <edition> --to <edition> [--check|--write]"
                        .to_owned(),
                );
            }
        }
        index += 1;
    }
    let input = input.unwrap_or_else(|| PathBuf::from("."));
    let from = from.ok_or_else(|| "jadren migrate requires --from".to_owned())?;
    let to = to.ok_or_else(|| "jadren migrate requires --to".to_owned())?;
    Ok((input, from, to, write))
}

fn parse_release_output_arguments(
    arguments: &[OsString],
    command: &str,
) -> Result<(PathBuf, Option<PathBuf>), String> {
    let mut lock_path = PathBuf::from("Cargo.lock");
    let mut output_path = None;
    let mut path_seen = false;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].to_str() {
            Some("--output") => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    return Err(format!("--output requires a path for jadren {command}"));
                };
                output_path = Some(PathBuf::from(value));
            }
            Some(value) if value.starts_with('-') => {
                return Err(format!("unknown option {value}"));
            }
            _ if !path_seen => {
                lock_path = PathBuf::from(&arguments[index]);
                path_seen = true;
            }
            _ => {
                return Err(format!(
                    "usage: jadren {command} [Cargo.lock] [--output <file>]"
                ));
            }
        }
        index += 1;
    }
    Ok((lock_path, output_path))
}

fn parse_provenance_arguments(
    arguments: &[OsString],
) -> Result<(PathBuf, Option<PathBuf>, String, String), String> {
    let mut lock_path = PathBuf::from("Cargo.lock");
    let mut output_path = None;
    let mut source_fingerprint = None;
    let mut toolchain = None;
    let mut path_seen = false;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].to_str() {
            Some("--output") => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    return Err("--output requires a provenance path".to_owned());
                };
                output_path = Some(PathBuf::from(value));
            }
            Some("--source-fingerprint") => {
                index += 1;
                let Some(value) = arguments.get(index).and_then(|value| value.to_str()) else {
                    return Err("--source-fingerprint requires a value".to_owned());
                };
                source_fingerprint = Some(value.to_owned());
            }
            Some("--toolchain") => {
                index += 1;
                let Some(value) = arguments.get(index).and_then(|value| value.to_str()) else {
                    return Err("--toolchain requires a value".to_owned());
                };
                toolchain = Some(value.to_owned());
            }
            Some(value) if value.starts_with('-') => {
                return Err(format!("unknown option {value}"));
            }
            _ if !path_seen => {
                lock_path = PathBuf::from(&arguments[index]);
                path_seen = true;
            }
            _ => {
                return Err("usage: jadren provenance [Cargo.lock] --source-fingerprint <id> --toolchain <id> [--output <file>]".to_owned());
            }
        }
        index += 1;
    }
    let source_fingerprint = source_fingerprint
        .ok_or_else(|| "jadren provenance requires --source-fingerprint <id>".to_owned())?;
    let toolchain =
        toolchain.ok_or_else(|| "jadren provenance requires --toolchain <id>".to_owned())?;
    Ok((lock_path, output_path, source_fingerprint, toolchain))
}

fn function_signature(
    function: &jadren_parser::Function,
    source: &jadren_source::SourceFile,
) -> String {
    let visibility = if function.is_public { "pub " } else { "" };
    let generics = generic_parameters(&function.generic_parameters, source);
    let parameters = function
        .parameters
        .iter()
        .map(|parameter| {
            format!(
                "{}: {}",
                parameter.name.text,
                source.slice(parameter.ty.span()).unwrap_or("_")
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let return_type = function
        .return_type
        .as_ref()
        .map(|ty| format!(" -> {}", source.slice(ty.span()).unwrap_or("_")))
        .unwrap_or_default();
    format!(
        "{visibility}fn {}{generics}({parameters}){return_type} {{ ... }}",
        function.name.text
    )
}

fn extern_function_signature(
    function: &jadren_parser::ExternFunction,
    source: &jadren_source::SourceFile,
) -> String {
    let unsafe_marker = if function.is_unsafe { "unsafe " } else { "" };
    let parameters = function
        .parameters
        .iter()
        .map(|parameter| {
            format!(
                "{}: {}",
                parameter.name.text,
                source.slice(parameter.ty.span()).unwrap_or("_")
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let return_type = function
        .return_type
        .as_ref()
        .map(|ty| format!(" -> {}", source.slice(ty.span()).unwrap_or("_")))
        .unwrap_or_default();
    format!(
        "extern {unsafe_marker}fn {}({parameters}){return_type};",
        function.name.text
    )
}

fn record_signature(
    record: &jadren_parser::RecordDeclaration,
    source: &jadren_source::SourceFile,
    keyword: &str,
) -> String {
    let generics = generic_parameters(&record.generic_parameters, source);
    let fields = record
        .fields
        .iter()
        .filter(|field| field.is_public)
        .map(|field| {
            let visibility = if field.is_public { "pub " } else { "" };
            format!(
                "    {visibility}{}: {},",
                field.name.text,
                source.slice(field.ty.span()).unwrap_or("_")
            )
        })
        .collect::<Vec<_>>();
    format!(
        "{keyword} {}{generics} {{\n{}\n}}",
        record.name.text,
        fields.join("\n")
    )
}

fn enum_signature(
    declaration: &jadren_parser::EnumDeclaration,
    source: &jadren_source::SourceFile,
) -> String {
    let generics = generic_parameters(&declaration.generic_parameters, source);
    let variants = declaration
        .variants
        .iter()
        .map(|variant| {
            let fields = variant
                .fields
                .iter()
                .map(|field| source.slice(field.ty.span()).unwrap_or("_"))
                .collect::<Vec<_>>();
            if fields.is_empty() {
                format!("    {},", variant.name.text)
            } else {
                format!("    {}({}),", variant.name.text, fields.join(", "))
            }
        })
        .collect::<Vec<_>>();
    format!(
        "enum {}{generics} {{\n{}\n}}",
        declaration.name.text,
        variants.join("\n")
    )
}

fn generic_parameters(
    parameters: &[jadren_parser::GenericParameter],
    source: &jadren_source::SourceFile,
) -> String {
    if parameters.is_empty() {
        String::new()
    } else {
        format!(
            "<{}>",
            parameters
                .iter()
                .map(|parameter| source.slice(parameter.span).unwrap_or(&parameter.name.text))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn append_annotations(
    markdown: &mut String,
    annotations: &[jadren_parser::Annotation],
    source: &jadren_source::SourceFile,
) {
    for annotation in annotations {
        if let Some(text) = source.slice(annotation.span) {
            markdown.push_str(&format!("Annotation: `{}`\n\n", markdown_inline(text)));
        }
    }
}

fn markdown_inline(value: &str) -> String {
    value.replace('`', "\\`").replace(['\n', '\r'], " ")
}

fn collect_jadren_files(path: &Path) -> Result<Vec<PathBuf>, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("failed to inspect `{}`: {error}", path.display()))?;
    let mut files = Vec::new();
    if metadata.is_file() {
        if path.extension().and_then(|extension| extension.to_str()) != Some("jdn") {
            return Err(format!(
                "test path must have `.jdn` extension: `{}`",
                path.display()
            ));
        }
        files.push(path.to_path_buf());
    } else if metadata.is_dir() {
        collect_jadren_files_recursive(path, &mut files)?;
    } else {
        return Err(format!(
            "test path is not a file or directory: `{}`",
            path.display()
        ));
    }
    files.sort_by_key(|file| file.display().to_string());
    if files.is_empty() {
        return Err(format!("no `.jdn` files found under `{}`", path.display()));
    }
    Ok(files)
}

fn collect_jadren_files_recursive(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut entries = fs::read_dir(path)
        .map_err(|error| format!("failed to read `{}`: {error}", path.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to enumerate `{}`: {error}", path.display()))?;
    entries.sort_by_key(|entry| entry.path().display().to_string());
    for entry in entries {
        let entry_path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect `{}`: {error}", entry_path.display()))?;
        if file_type.is_dir() {
            collect_jadren_files_recursive(&entry_path, files)?;
        } else if file_type.is_file()
            && entry_path
                .extension()
                .and_then(|extension| extension.to_str())
                == Some("jdn")
        {
            files.push(entry_path);
        }
    }
    Ok(())
}

fn print_test_json(
    results: &[(&PathBuf, jadren_driver::CheckOutput)],
    sources: &SourceManager,
    failed: usize,
) {
    println!("{{\"schema\":\"jadren-test-0.1\",\"files\":[");
    for (index, (file, output)) in results.iter().enumerate() {
        if index > 0 {
            println!(",");
        }
        print!(
            "{{\"path\":{},\"status\":\"{}\",\"diagnostics\":[",
            json_string(&file.display().to_string()),
            if output.has_errors() {
                "failed"
            } else {
                "passed"
            }
        );
        for (diagnostic_index, diagnostic) in output.diagnostics.iter().enumerate() {
            if diagnostic_index > 0 {
                print!(",");
            }
            print!("{}", render_json(diagnostic, sources));
        }
        print!("]}}");
    }
    println!(
        "],\"passed\":{},\"failed\":{},\"total\":{},\"result\":\"{}\"}}",
        results.len() - failed,
        failed,
        results.len(),
        if failed == 0 { "pass" } else { "fail" }
    );
}

fn json_string(value: &str) -> String {
    let mut output = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                output.push_str(&format!("\\u{:04x}", u32::from(character)));
            }
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

fn format_file(arguments: &[OsString]) -> Result<(), String> {
    if arguments.is_empty() || arguments.len() > 2 {
        return Err("usage: jadren format <file.jdn> [--write|--check]".to_owned());
    }
    let path = PathBuf::from(&arguments[0]);
    let mode = arguments.get(1).and_then(|value| value.to_str());
    if let Some(value) = mode
        && value != "--write"
        && value != "--check"
    {
        return Err("usage: jadren format <file.jdn> [--write|--check]".to_owned());
    }
    let text = read_utf8(&path)?;
    let mut sources = SourceManager::new();
    let source_id = sources
        .add(&path, text.clone())
        .map_err(|error| format!("failed to register `{}`: {error}", path.display()))?;
    let source = sources
        .get(source_id)
        .ok_or_else(|| format!("source `{}` disappeared", path.display()))?;
    let lexed = lex(source);
    if !lexed.diagnostics.is_empty() {
        render_diagnostics(&lexed.diagnostics, &sources, DiagnosticFormat::Text);
        return Err(format!(
            "cannot format `{}` because lexing failed",
            path.display()
        ));
    }
    let formatted = format_source(source, &lexed.tokens)
        .map_err(|error| format!("failed to format `{}`: {error}", path.display()))?;

    match mode {
        Some("--write") => {
            fs::write(&path, formatted)
                .map_err(|error| format!("failed to write `{}`: {error}", path.display()))?;
            println!("formatted {}", path.display());
        }
        Some("--check") => {
            if formatted != text {
                println!("would reformat {}", path.display());
                std::process::exit(1);
            }
            println!("already formatted {}", path.display());
        }
        _ => print!("{formatted}"),
    }
    Ok(())
}

fn lsp(arguments: &[OsString]) -> Result<(), String> {
    if !arguments.is_empty() {
        return Err("usage: jadren lsp".to_owned());
    }
    jadren_lsp::run_stdio().map_err(|error| format!("LSP transport failed: {error}"))
}

fn emit(arguments: &[OsString]) -> Result<(), String> {
    let Some(kind) = arguments.first().and_then(|value| value.to_str()) else {
        return Err(emit_usage());
    };
    if kind == "object" {
        let object = parse_object_arguments(arguments)?;
        return emit_object_file(
            &object.source,
            &object.output,
            &object.target,
            object.profile,
            &object.cpu,
        );
    }
    if arguments.len() != 2 {
        return Err(emit_usage());
    }
    let path = PathBuf::from(&arguments[1]);
    if kind == "hir" {
        return emit_hir(&path);
    }
    if kind == "mir" {
        return emit_mir(&path);
    }
    if kind == "effects" {
        return emit_effects(&path);
    }
    if kind == "jir" {
        return emit_jir(&path);
    }
    if kind == "remarks" {
        return emit_remarks(&path);
    }
    if kind == "header" {
        return emit_header(&path);
    }
    if kind == "csharp" {
        return emit_csharp(&path);
    }
    if kind == "facade" {
        return emit_facade(&path);
    }
    if kind == "abi-tests" {
        return emit_abi_tests(&path);
    }
    if matches!(kind, "llvm" | "asm") {
        return emit_native(kind, &path);
    }
    let (sources, source_id) = load_source(&path)?;
    let source = sources
        .get(source_id)
        .ok_or_else(|| "internal source manager error".to_owned())?;
    let output = lex(source);

    match kind {
        "tokens" => {
            for token in &output.tokens {
                let text = source.slice(token.span).unwrap_or_default();
                println!(
                    "{:?} {}..{} `{}`",
                    token.kind,
                    token.span.start,
                    token.span.end,
                    text.escape_debug()
                );
            }
            render_diagnostics(&output.diagnostics, &sources, DiagnosticFormat::Text);
            if output.has_errors() {
                std::process::exit(1);
            }
        }
        "syntax" => {
            render_diagnostics(&output.diagnostics, &sources, DiagnosticFormat::Text);
            if output.has_errors() {
                std::process::exit(1);
            }
            let parsed = parse_syntax(source, &output.tokens);
            render_diagnostics(&parsed.diagnostics, &sources, DiagnosticFormat::Text);
            if parsed.has_errors() {
                std::process::exit(1);
            }
            print!("{}", parsed.syntax.pretty(source));
        }
        "ast" => {
            render_diagnostics(&output.diagnostics, &sources, DiagnosticFormat::Text);
            if output.has_errors() {
                std::process::exit(1);
            }
            let parsed = parse(source, &output.tokens);
            render_diagnostics(&parsed.diagnostics, &sources, DiagnosticFormat::Text);
            if parsed.has_errors() {
                std::process::exit(1);
            }
            println!("{:#?}", parsed.file);
        }
        _ => {
            return Err(format!(
                "unsupported emit kind `{kind}`; use `tokens`, `syntax`, `ast`, `hir`, `mir`, `effects`, `jir`, `remarks`, `header`, `csharp`, `facade`, `abi-tests`, `llvm`, `asm`, or `object <file.jdn> <output.obj> [--target <triple>]`"
            ));
        }
    }
    Ok(())
}

fn emit_usage() -> String {
    "usage: jadren emit tokens|syntax|ast|hir|mir|effects|jir|remarks|header|csharp|facade|abi-tests|llvm|asm <file.jdn> | object <file.jdn> <output.obj> [--target <triple>] [--profile debug|release] [--cpu baseline|avx2|neon]"
        .to_owned()
}

struct ObjectEmitArguments {
    source: PathBuf,
    output: PathBuf,
    target: TargetTriple,
    profile: BuildProfile,
    cpu: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExecutableArguments {
    source: PathBuf,
    output: Option<PathBuf>,
    profile: BuildProfile,
    cpu: String,
}

fn executable_usage(command: &str) -> String {
    format!(
        "usage: jadren {command} <file.jdn> [-o <output.exe>] [--profile debug|release] [--cpu baseline|avx2]"
    )
}

fn parse_executable_arguments(
    command: &str,
    arguments: &[OsString],
) -> Result<ExecutableArguments, String> {
    let Some(source) = arguments.first() else {
        return Err(executable_usage(command));
    };
    if source.to_str().is_some_and(|value| value.starts_with('-')) {
        return Err(executable_usage(command));
    }

    let mut parsed = ExecutableArguments {
        source: PathBuf::from(source),
        output: None,
        profile: BuildProfile::Debug,
        cpu: "baseline".to_owned(),
    };
    let mut index = 1;
    while index < arguments.len() {
        let flag = arguments[index]
            .to_str()
            .ok_or_else(|| "executable option must be UTF-8".to_owned())?;
        let value = arguments.get(index + 1);
        match flag {
            "-o" | "--output" => {
                let value = value.ok_or_else(|| format!("`{flag}` requires a value"))?;
                parsed.output = Some(PathBuf::from(value));
            }
            "--profile" => {
                parsed.profile = match value.and_then(|value| value.to_str()) {
                    Some("debug") => BuildProfile::Debug,
                    Some("release") => BuildProfile::Release,
                    Some(other) => {
                        return Err(format!(
                            "unsupported executable profile `{other}`; use `debug` or `release`"
                        ));
                    }
                    None => return Err("`--profile` requires a value".to_owned()),
                };
            }
            "--cpu" => {
                parsed.cpu = match value.and_then(|value| value.to_str()) {
                    Some("baseline") => "baseline".to_owned(),
                    Some("avx2") => "avx2".to_owned(),
                    Some(other) => {
                        return Err(format!(
                            "unsupported executable CPU `{other}`; use `baseline` or `avx2`"
                        ));
                    }
                    None => return Err("`--cpu` requires a value".to_owned()),
                };
            }
            _ => return Err(format!("unknown executable option `{flag}`")),
        }
        index += 2;
    }
    Ok(parsed)
}

fn build_executable(arguments: &[OsString]) -> Result<(), String> {
    let arguments = parse_executable_arguments("build", arguments)?;
    let output = build_windows_executable(&arguments)?;
    println!("built {}", output.display());
    Ok(())
}

fn run_executable(arguments: &[OsString]) -> Result<(), String> {
    let arguments = parse_executable_arguments("run", arguments)?;
    let output = build_windows_executable(&arguments)?;
    println!("running {}", output.display());
    let status = Command::new(&output)
        .status()
        .map_err(|error| format!("failed to run `{}`: {error}", output.display()))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

#[cfg(windows)]
fn build_windows_executable(arguments: &ExecutableArguments) -> Result<PathBuf, String> {
    use inkwell::context::Context;
    use jadren_codegen_llvm::{
        CpuVariant, ObjectOptimization, ObjectOptions, TypeLoweringConfig, WindowsLinkOptions,
        link_windows_executable, lower_to_object, write_object,
    };

    let target = TargetTriple::from_str("x86_64-pc-windows-msvc")
        .expect("the Windows host target must be valid");
    let mut jir = checked_jir_for_target(&arguments.source, target, arguments.profile)?;
    prepare_executable_entry(&mut jir)?;

    let profile_name = match arguments.profile {
        BuildProfile::Debug => "debug",
        BuildProfile::Release => "release",
        BuildProfile::Check => unreachable!("executable build does not use check profile"),
    };
    let output = arguments.output.clone().unwrap_or_else(|| {
        let name = arguments
            .source
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("jadren-program");
        Path::new("target")
            .join("jadren")
            .join(profile_name)
            .join(format!("{name}.exe"))
    });
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create executable output directory `{}`: {error}",
                parent.display()
            )
        })?;
    }
    let object = output.with_extension("obj");
    let optimization = match arguments.profile {
        BuildProfile::Debug => ObjectOptimization::Debug,
        BuildProfile::Release => ObjectOptimization::Release,
        BuildProfile::Check => unreachable!("executable build does not use check profile"),
    };
    let variant = match arguments.cpu.as_str() {
        "baseline" => CpuVariant::X86_64Baseline,
        "avx2" => CpuVariant::X86_64Avx2,
        other => return Err(format!("unsupported executable CPU `{other}`")),
    };
    let object_options = ObjectOptions::for_variant_with_optimization(variant, optimization);
    let context = Context::create();
    let module_name = arguments
        .source
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("jadren_program");
    let bytes = lower_to_object(
        &context,
        &jir,
        module_name,
        &TypeLoweringConfig::x86_64_windows_msvc(),
        &object_options,
    )
    .map_err(|error| {
        format!(
            "failed to emit executable object for `{}`: {error}",
            arguments.source.display()
        )
    })?;
    write_object(&object, &bytes)
        .map_err(|error| format!("failed to write `{}`: {error}", object.display()))?;
    let mut link_inputs = vec![object];
    if jir
        .functions
        .iter()
        .any(|function| function.name == "print" && function.linkage == jadren_jir::Linkage::Import)
    {
        link_inputs.extend(build_windows_console_runtime(&output)?);
    }
    link_windows_executable(&output, &link_inputs, &WindowsLinkOptions::default())
        .map_err(|error| format!("failed to link `{}`: {error}", output.display()))?;
    Ok(output)
}

#[cfg(windows)]
fn build_windows_console_runtime(output: &Path) -> Result<Vec<PathBuf>, String> {
    const CONSOLE_LLVM: &str = r#"target triple = "x86_64-pc-windows-msvc"

%JadrenString = type { ptr, i64 }

@jadren.newline = private unnamed_addr constant [2 x i8] c"\0D\0A", align 1

declare dllimport ptr @GetStdHandle(i32)
declare dllimport i32 @WriteFile(ptr, ptr, i32, ptr, ptr)

define void @print(%JadrenString %value) {
entry:
  %data = extractvalue %JadrenString %value, 0
  %length64 = extractvalue %JadrenString %value, 1
  %length = trunc i64 %length64 to i32
  %handle = call ptr @GetStdHandle(i32 -11)
  %written = alloca i32, align 4
  %ignored = call i32 @WriteFile(ptr %handle, ptr %data, i32 %length, ptr %written, ptr null)
  %newline = call i32 @WriteFile(ptr %handle, ptr @jadren.newline, i32 2, ptr %written, ptr null)
  ret void
}
"#;
    const KERNEL32_DEF: &str = "LIBRARY kernel32.dll\nEXPORTS\nGetStdHandle\nWriteFile\n";

    let stem = output.with_extension("");
    let llvm_path = stem.with_extension("console.ll");
    let object_path = stem.with_extension("console.obj");
    let definition_path = stem.with_extension("kernel32.def");
    let import_library_path = stem.with_extension("kernel32.lib");
    fs::write(&llvm_path, CONSOLE_LLVM)
        .map_err(|error| format!("failed to write `{}`: {error}", llvm_path.display()))?;
    fs::write(&definition_path, KERNEL32_DEF)
        .map_err(|error| format!("failed to write `{}`: {error}", definition_path.display()))?;

    let llvm_prefix = verify_llvm_toolchain()?;
    run_native_tool(
        &llvm_prefix.join("bin").join("clang.exe"),
        &[
            OsString::from("--target=x86_64-pc-windows-msvc"),
            OsString::from("-O2"),
            OsString::from("-c"),
            llvm_path.as_os_str().to_owned(),
            OsString::from("-o"),
            object_path.as_os_str().to_owned(),
        ],
        "compile the Jadren console runtime",
    )?;
    run_native_tool(
        &llvm_prefix.join("bin").join("llvm-dlltool.exe"),
        &[
            OsString::from("-m"),
            OsString::from("i386:x86-64"),
            OsString::from("-d"),
            definition_path.as_os_str().to_owned(),
            OsString::from("-l"),
            import_library_path.as_os_str().to_owned(),
            OsString::from("-D"),
            OsString::from("kernel32.dll"),
        ],
        "create the kernel32 import library",
    )?;
    for path in [&object_path, &import_library_path] {
        if !path.is_file() {
            return Err(format!(
                "native console runtime tool succeeded without `{}`",
                path.display()
            ));
        }
    }
    Ok(vec![object_path, import_library_path])
}

#[cfg(windows)]
fn run_native_tool(path: &Path, arguments: &[OsString], action: &str) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!(
            "cannot {action}; tool is missing: `{}`",
            path.display()
        ));
    }
    let output = Command::new(path)
        .args(arguments)
        .output()
        .map_err(|error| format!("failed to {action} with `{}`: {error}", path.display()))?;
    if !output.status.success() {
        return Err(format!(
            "failed to {action}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn build_windows_executable(_arguments: &ExecutableArguments) -> Result<PathBuf, String> {
    Err("`jadren build` and `jadren run` currently support Windows x86-64; Linux executable linking is the next platform gate".to_owned())
}

fn prepare_executable_entry(module: &mut jadren_jir::Module) -> Result<(), String> {
    use jadren_jir::{
        Block, BlockId, Constant, Function, FunctionId, Instruction, InstructionKind, Linkage,
        Terminator, Type, TypeId, TypedValue, ValueId, verify,
    };

    let int32 = module
        .types
        .iter()
        .position(|ty| {
            matches!(
                ty,
                Type::Integer {
                    signed: true,
                    bits: 32
                }
            )
        })
        .map(TypeId::new)
        .unwrap_or_else(|| {
            let id = TypeId::new(module.types.len());
            module.types.push(Type::Integer {
                signed: true,
                bits: 32,
            });
            id
        });

    if let Some(entry) = module
        .functions
        .iter()
        .find(|function| function.name == "jadren_entry")
    {
        if entry.linkage != Linkage::Export || !entry.parameters.is_empty() || entry.result != int32
        {
            return Err(
                "explicit `jadren_entry` must be `export fn jadren_entry() -> Int32`".to_owned(),
            );
        }
        return Ok(());
    }

    let main = module
        .functions
        .iter()
        .find(|function| function.name == "main" && function.linkage != Linkage::Import)
        .ok_or_else(|| "executable source must define `fn main()`".to_owned())?;
    if !main.parameters.is_empty() {
        return Err("Jadren 0.1 executable `main` must not accept parameters".to_owned());
    }
    let main_id = main.id;
    let main_result = main.result;
    let main_returns_unit = matches!(module.types.get(main_result.index()), Some(Type::Unit));
    let main_returns_int32 = main_result == int32;
    if !main_returns_unit && !main_returns_int32 {
        return Err("Jadren 0.1 executable `main` must return `Unit` or `Int32`".to_owned());
    }

    let result_value = ValueId::new(0);
    let mut instructions = Vec::with_capacity(if main_returns_unit { 2 } else { 1 });
    instructions.push(Instruction {
        result: main_returns_int32.then_some(TypedValue {
            value: result_value,
            ty: int32,
        }),
        kind: InstructionKind::Call {
            function: main_id,
            arguments: Vec::new(),
        },
        span: None,
    });
    if main_returns_unit {
        instructions.push(Instruction {
            result: Some(TypedValue {
                value: result_value,
                ty: int32,
            }),
            kind: InstructionKind::Constant(Constant::Integer { value: 0 }),
            span: None,
        });
    }
    module.functions.push(Function {
        id: FunctionId::new(module.functions.len()),
        name: "jadren_entry".to_owned(),
        linkage: Linkage::Export,
        parameters: Vec::new(),
        result: int32,
        blocks: vec![Block {
            id: BlockId::new(0),
            parameters: Vec::new(),
            instructions,
            terminator: Terminator::Return {
                value: Some(result_value),
            },
            span: None,
        }],
        span: None,
    });
    let errors = verify(module);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("generated executable entry is invalid: {errors:?}"))
    }
}

fn parse_object_arguments(arguments: &[OsString]) -> Result<ObjectEmitArguments, String> {
    if arguments.len() < 3 {
        return Err(emit_usage());
    }
    let source = PathBuf::from(&arguments[1]);
    let output = PathBuf::from(&arguments[2]);
    let mut target = CompilerConfig::default().target;
    let mut profile = BuildProfile::Debug;
    let mut cpu = "baseline".to_owned();
    let mut index = 3;
    while index < arguments.len() {
        let flag = arguments[index]
            .to_str()
            .ok_or_else(|| "object option must be UTF-8".to_owned())?;
        let value = arguments.get(index + 1).and_then(|value| value.to_str());
        match flag {
            "--target" => {
                let value = value.ok_or_else(|| "`--target` requires a value".to_owned())?;
                target = TargetTriple::from_str(value).map_err(|error| error.to_string())?;
            }
            "--profile" => {
                profile = match value {
                    Some("debug") => BuildProfile::Debug,
                    Some("release") => BuildProfile::Release,
                    Some(other) => {
                        return Err(format!(
                            "unsupported object profile `{other}`; use `debug` or `release`"
                        ));
                    }
                    None => return Err("`--profile` requires a value".to_owned()),
                };
            }
            "--cpu" => {
                cpu = match value {
                    Some("baseline") => "baseline".to_owned(),
                    Some("avx2") => "avx2".to_owned(),
                    Some("neon") => "neon".to_owned(),
                    Some(other) => {
                        return Err(format!(
                            "unsupported object CPU `{other}`; use `baseline`, `avx2`, or `neon`"
                        ));
                    }
                    None => return Err("`--cpu` requires a value".to_owned()),
                };
            }
            _ => return Err(format!("unknown object option `{flag}`")),
        }
        index += 2;
    }
    Ok(ObjectEmitArguments {
        source,
        output,
        target,
        profile,
        cpu,
    })
}

#[cfg(any(windows, target_os = "linux"))]
fn emit_object_file(
    source_path: &Path,
    output_path: &Path,
    target: &TargetTriple,
    profile: BuildProfile,
    cpu: &str,
) -> Result<(), String> {
    use inkwell::context::Context;
    use jadren_codegen_llvm::{
        CpuVariant, ObjectOptimization, ObjectOptions, TypeLoweringConfig, lower_to_object,
        write_object,
    };

    let optimization = match profile {
        BuildProfile::Debug => ObjectOptimization::Debug,
        BuildProfile::Release => ObjectOptimization::Release,
        BuildProfile::Check => unreachable!("object emission does not use check profile"),
    };
    let (type_config, object_options) = match target.as_str() {
        "x86_64-pc-windows-msvc" | "x86_64-unknown-linux-gnu" => {
            let variant = match cpu {
                "baseline" => CpuVariant::X86_64Baseline,
                "avx2" => CpuVariant::X86_64Avx2,
                "neon" => {
                    return Err("NEON is only valid for AArch64 object targets".to_owned());
                }
                other => return Err(format!("unsupported object CPU `{other}`")),
            };
            let type_config = if target.as_str() == "x86_64-pc-windows-msvc" {
                TypeLoweringConfig::x86_64_windows_msvc()
            } else {
                TypeLoweringConfig::x86_64_linux_gnu()
            };
            (
                type_config,
                ObjectOptions::for_variant_with_optimization(variant, optimization),
            )
        }
        "aarch64-unknown-linux-android24" | "aarch64-linux-android24" => {
            let object_options = match cpu {
                "baseline" => ObjectOptions {
                    cpu: "generic".to_owned(),
                    features: String::new(),
                    optimization,
                },
                "neon" => ObjectOptions {
                    cpu: "generic".to_owned(),
                    features: "+neon".to_owned(),
                    optimization,
                },
                "avx2" => return Err("AVX2 is only valid for x86-64 object targets".to_owned()),
                other => return Err(format!("unsupported object CPU `{other}`")),
            };
            (TypeLoweringConfig::aarch64_android(), object_options)
        }
        unsupported => {
            return Err(format!(
                "`jadren emit object` does not support target `{unsupported}` yet"
            ));
        }
    };
    let jir = checked_jir_for_target(source_path, target.clone(), profile)?;
    let context = Context::create();
    let module_name = source_path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("jadren_module");
    let bytes = lower_to_object(&context, &jir, module_name, &type_config, &object_options)
        .map_err(|error| format!("failed to emit `{}` object: {error}", source_path.display()))?;
    write_object(output_path, &bytes)
        .map_err(|error| format!("failed to write `{}`: {error}", output_path.display()))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn emit_object_file(
    _source_path: &Path,
    _output_path: &Path,
    _target: &TargetTriple,
    _profile: BuildProfile,
    _cpu: &str,
) -> Result<(), String> {
    Err("LLVM object emission is not enabled on this host platform".to_owned())
}

fn emit_remarks(path: &Path) -> Result<(), String> {
    let text = read_utf8(path)?;
    let config = CompilerConfig {
        profile: BuildProfile::Release,
        ..CompilerConfig::default()
    };
    let mut session = CompilerSession::new(config);
    let source_id = session
        .add_source(path, text)
        .map_err(|error| format!("failed to register `{}`: {error}", path.display()))?;
    let output = session
        .check(source_id)
        .map_err(|error| format!("failed to check `{}`: {error}", path.display()))?;
    render_diagnostics(
        &output.diagnostics,
        session.sources(),
        DiagnosticFormat::Text,
    );
    if output.has_errors() {
        std::process::exit(1);
    }
    let report = output
        .artifacts
        .and_then(|artifacts| artifacts.optimization)
        .ok_or_else(|| "optimization remarks require a verified Release JIR".to_owned())?;
    print!("{}", report.to_text());
    Ok(())
}

fn emit_header(path: &Path) -> Result<(), String> {
    let text = read_utf8(path)?;
    let mut session = CompilerSession::new(CompilerConfig::default());
    let source_id = session
        .add_source(path, text)
        .map_err(|error| format!("failed to register `{}`: {error}", path.display()))?;
    let output = session
        .check(source_id)
        .map_err(|error| format!("failed to check `{}`: {error}", path.display()))?;
    render_diagnostics(
        &output.diagnostics,
        session.sources(),
        DiagnosticFormat::Text,
    );
    if output.has_errors() {
        std::process::exit(1);
    }
    let source = session
        .sources()
        .get(source_id)
        .ok_or_else(|| "internal source manager error".to_owned())?;
    let artifacts = output
        .artifacts
        .as_ref()
        .ok_or_else(|| "compiler artifacts were not produced".to_owned())?;
    let header = jadren_bindgen::generate_c_header(
        source,
        &artifacts.ast,
        &artifacts.resolution,
        &artifacts.type_check,
    )
    .map_err(|errors| {
        errors
            .into_iter()
            .map(|error| format!("{}: {}", error.code, error.message))
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    print!("{}", header.text);
    Ok(())
}

fn emit_csharp(path: &Path) -> Result<(), String> {
    let text = read_utf8(path)?;
    let mut session = CompilerSession::new(CompilerConfig::default());
    let source_id = session
        .add_source(path, text)
        .map_err(|error| format!("failed to register `{}`: {error}", path.display()))?;
    let output = session
        .check(source_id)
        .map_err(|error| format!("failed to check `{}`: {error}", path.display()))?;
    render_diagnostics(
        &output.diagnostics,
        session.sources(),
        DiagnosticFormat::Text,
    );
    if output.has_errors() {
        std::process::exit(1);
    }
    let source = session
        .sources()
        .get(source_id)
        .ok_or_else(|| "internal source manager error".to_owned())?;
    let artifacts = output
        .artifacts
        .as_ref()
        .ok_or_else(|| "compiler artifacts were not produced".to_owned())?;
    let bindings = jadren_bindgen::generate_csharp_bindings(
        source,
        &artifacts.ast,
        &artifacts.resolution,
        &artifacts.type_check,
        "jadren_native",
    )
    .map_err(|errors| {
        errors
            .into_iter()
            .map(|error| format!("{}: {}", error.code, error.message))
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    print!("{}", bindings.text);
    Ok(())
}

fn emit_facade(path: &Path) -> Result<(), String> {
    let text = read_utf8(path)?;
    let mut session = CompilerSession::new(CompilerConfig::default());
    let source_id = session
        .add_source(path, text)
        .map_err(|error| format!("failed to register `{}`: {error}", path.display()))?;
    let output = session
        .check(source_id)
        .map_err(|error| format!("failed to check `{}`: {error}", path.display()))?;
    render_diagnostics(
        &output.diagnostics,
        session.sources(),
        DiagnosticFormat::Text,
    );
    if output.has_errors() {
        std::process::exit(1);
    }
    let source = session
        .sources()
        .get(source_id)
        .ok_or_else(|| "internal source manager error".to_owned())?;
    let artifacts = output
        .artifacts
        .as_ref()
        .ok_or_else(|| "compiler artifacts were not produced".to_owned())?;
    let facade = jadren_bindgen::generate_csharp_facade(
        source,
        &artifacts.ast,
        &artifacts.resolution,
        &artifacts.type_check,
        "jadren_native",
    )
    .map_err(|errors| {
        errors
            .into_iter()
            .map(|error| format!("{}: {}", error.code, error.message))
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    print!("{}", facade.text);
    Ok(())
}

fn emit_abi_tests(path: &Path) -> Result<(), String> {
    let text = read_utf8(path)?;
    let mut session = CompilerSession::new(CompilerConfig::default());
    let source_id = session
        .add_source(path, text)
        .map_err(|error| format!("failed to register `{}`: {error}", path.display()))?;
    let output = session
        .check(source_id)
        .map_err(|error| format!("failed to check `{}`: {error}", path.display()))?;
    render_diagnostics(
        &output.diagnostics,
        session.sources(),
        DiagnosticFormat::Text,
    );
    if output.has_errors() {
        std::process::exit(1);
    }
    let source = session
        .sources()
        .get(source_id)
        .ok_or_else(|| "internal source manager error".to_owned())?;
    let artifacts = output
        .artifacts
        .as_ref()
        .ok_or_else(|| "compiler artifacts were not produced".to_owned())?;
    let tests = jadren_bindgen::generate_c_layout_tests(
        source,
        &artifacts.ast,
        &artifacts.resolution,
        &artifacts.type_check,
        "generated.h",
        64,
    )
    .map_err(|errors| {
        errors
            .into_iter()
            .map(|error| format!("{}: {}", error.code, error.message))
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    print!("{}", tests.text);
    Ok(())
}

fn emit_jir(path: &Path) -> Result<(), String> {
    let jir = checked_jir(path)?;
    print!("{}", jir.to_text());
    Ok(())
}

#[cfg(any(windows, target_os = "linux"))]
fn emit_native(kind: &str, path: &Path) -> Result<(), String> {
    use inkwell::context::Context;
    use jadren_codegen_llvm::{ObjectOptions, TypeLoweringConfig, emit_assembly, lower_module};

    let jir = checked_jir(path)?;
    let context = Context::create();
    let module_name = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("jadren_module");
    let type_config = if cfg!(target_os = "linux") {
        TypeLoweringConfig::x86_64_linux_gnu()
    } else {
        TypeLoweringConfig::default()
    };
    let llvm = lower_module(&context, &jir, module_name, &type_config)
        .map_err(|error| format!("failed to lower `{}` to LLVM: {error}", path.display()))?;
    match kind {
        "llvm" => print!("{}", llvm.print_to_string().to_string()),
        "asm" => {
            let bytes = emit_assembly(&llvm, &ObjectOptions::default()).map_err(|error| {
                format!("failed to emit `{}` assembly: {error}", path.display())
            })?;
            io::stdout()
                .write_all(&bytes)
                .map_err(|error| format!("failed to write assembly: {error}"))?;
        }
        _ => unreachable!("native emit kind checked by caller"),
    }
    Ok(())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn emit_native(_kind: &str, _path: &Path) -> Result<(), String> {
    Err("LLVM/assembly emission is not enabled on this host platform".to_owned())
}

fn checked_jir(path: &Path) -> Result<jadren_jir::Module, String> {
    checked_jir_for_target(path, CompilerConfig::default().target, BuildProfile::Debug)
}

fn checked_jir_for_target(
    path: &Path,
    target: TargetTriple,
    profile: BuildProfile,
) -> Result<jadren_jir::Module, String> {
    let text = read_utf8(path)?;
    let config = CompilerConfig {
        profile,
        target,
        ..CompilerConfig::default()
    };
    let mut session = CompilerSession::new(config);
    let source_id = session
        .add_source(path, text)
        .map_err(|error| format!("failed to register `{}`: {error}", path.display()))?;
    let output = session
        .check(source_id)
        .map_err(|error| format!("failed to check `{}`: {error}", path.display()))?;
    render_diagnostics(
        &output.diagnostics,
        session.sources(),
        DiagnosticFormat::Text,
    );
    if output.has_errors() {
        std::process::exit(1);
    }
    output
        .artifacts
        .and_then(|artifacts| artifacts.jir)
        .ok_or_else(|| format!("verified JIR was not produced for `{}`", path.display()))
}

fn emit_hir(path: &Path) -> Result<(), String> {
    let text = read_utf8(path)?;
    let mut session = CompilerSession::new(CompilerConfig::default());
    let source_id = session
        .add_source(path, text)
        .map_err(|error| format!("failed to register `{}`: {error}", path.display()))?;
    let output = session
        .check(source_id)
        .map_err(|error| format!("failed to check `{}`: {error}", path.display()))?;
    render_diagnostics(
        &output.diagnostics,
        session.sources(),
        DiagnosticFormat::Text,
    );
    if output.has_errors() {
        std::process::exit(1);
    }
    let hir = output
        .artifacts
        .and_then(|artifacts| artifacts.hir)
        .ok_or_else(|| "typed HIR was not produced for a valid source".to_owned())?;
    println!("{hir:#?}");
    Ok(())
}

fn emit_effects(path: &Path) -> Result<(), String> {
    let text = read_utf8(path)?;
    let mut session = CompilerSession::new(CompilerConfig::default());
    let source_id = session
        .add_source(path, text)
        .map_err(|error| format!("failed to register `{}`: {error}", path.display()))?;
    let output = session
        .check(source_id)
        .map_err(|error| format!("failed to check `{}`: {error}", path.display()))?;
    render_diagnostics(
        &output.diagnostics,
        session.sources(),
        DiagnosticFormat::Text,
    );
    if output.has_errors() {
        std::process::exit(1);
    }
    let effects = output
        .artifacts
        .and_then(|artifacts| artifacts.effects)
        .ok_or_else(|| "effect analysis was not produced for a valid source".to_owned())?;
    for function in effects.functions {
        let names: Vec<_> = function
            .inferred
            .iter()
            .map(|effect| effect.as_str())
            .collect();
        if names.is_empty() {
            println!("{}: Pure", function.name);
        } else {
            println!("{}: {}", function.name, names.join(", "));
        }
    }
    Ok(())
}

fn emit_mir(path: &Path) -> Result<(), String> {
    let text = read_utf8(path)?;
    let mut session = CompilerSession::new(CompilerConfig::default());
    let source_id = session
        .add_source(path, text)
        .map_err(|error| format!("failed to register `{}`: {error}", path.display()))?;
    let output = session
        .check(source_id)
        .map_err(|error| format!("failed to check `{}`: {error}", path.display()))?;
    render_diagnostics(
        &output.diagnostics,
        session.sources(),
        DiagnosticFormat::Text,
    );
    if output.has_errors() {
        std::process::exit(1);
    }
    let mir = output
        .artifacts
        .and_then(|artifacts| artifacts.mir)
        .ok_or_else(|| "verified MIR was not produced for a valid source".to_owned())?;
    println!("{mir:#?}");
    Ok(())
}

fn load_source(path: &Path) -> Result<(SourceManager, SourceId), String> {
    let text = read_utf8(path)?;
    let mut sources = SourceManager::new();
    let source_id = sources
        .add(path, text)
        .map_err(|error| format!("failed to register `{}`: {error}", path.display()))?;
    Ok((sources, source_id))
}

fn read_utf8(path: &Path) -> Result<String, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("failed to read `{}`: {error}", path.display()))?;
    String::from_utf8(bytes)
        .map_err(|error| format!("`{}` is not valid UTF-8: {error}", path.display()))
}

fn parse_check_arguments(arguments: &[OsString]) -> Result<(PathBuf, CompilerConfig), String> {
    let mut path = None;
    let mut config = CompilerConfig::default();
    let mut index = 0;

    while index < arguments.len() {
        match arguments[index].to_str() {
            Some("--format") => {
                index += 1;
                let Some(value) = arguments.get(index).and_then(|value| value.to_str()) else {
                    return Err("`--format` requires `text` or `json`".to_owned());
                };
                config.diagnostic_format = match value {
                    "text" => DiagnosticFormat::Text,
                    "json" => DiagnosticFormat::Json,
                    _ => return Err(format!("unsupported output format `{value}`")),
                };
            }
            Some("--target") => {
                index += 1;
                let Some(value) = arguments.get(index).and_then(|value| value.to_str()) else {
                    return Err("`--target` requires a canonical target triple".to_owned());
                };
                config.target = TargetTriple::from_str(value).map_err(|error| error.to_string())?;
            }
            Some("--edition") => {
                index += 1;
                let Some(value) = arguments.get(index).and_then(|value| value.to_str()) else {
                    return Err("`--edition` requires a supported edition".to_owned());
                };
                config.edition = Edition::from_str(value).map_err(|error| error.to_string())?;
            }
            Some("--warnings-as-errors") => config.warnings_as_errors = true,
            Some(value) if value.starts_with('-') => {
                return Err(format!("unknown option `{value}`"));
            }
            _ if path.is_none() => path = Some(PathBuf::from(&arguments[index])),
            _ => return Err("`jadren check` accepts exactly one source path".to_owned()),
        }
        index += 1;
    }

    path.map(|path| (path, config)).ok_or_else(|| {
        "usage: jadren check <file.jdn> [--format text|json] [--target <triple>] [--edition <edition>] [--warnings-as-errors]"
            .to_owned()
    })
}

fn parse_test_arguments(arguments: &[OsString]) -> Result<(PathBuf, CompilerConfig), String> {
    let mut path = None;
    let mut config = CompilerConfig::default();
    let mut index = 0;

    while index < arguments.len() {
        match arguments[index].to_str() {
            Some("--format") => {
                index += 1;
                let Some(value) = arguments.get(index).and_then(|value| value.to_str()) else {
                    return Err("`--format` requires `text` or `json`".to_owned());
                };
                config.diagnostic_format = match value {
                    "text" => DiagnosticFormat::Text,
                    "json" => DiagnosticFormat::Json,
                    _ => return Err(format!("unsupported output format `{value}`")),
                };
            }
            Some("--target") => {
                index += 1;
                let Some(value) = arguments.get(index).and_then(|value| value.to_str()) else {
                    return Err("`--target` requires a canonical target triple".to_owned());
                };
                config.target = TargetTriple::from_str(value).map_err(|error| error.to_string())?;
            }
            Some("--edition") => {
                index += 1;
                let Some(value) = arguments.get(index).and_then(|value| value.to_str()) else {
                    return Err("`--edition` requires a supported edition".to_owned());
                };
                config.edition = Edition::from_str(value).map_err(|error| error.to_string())?;
            }
            Some("--warnings-as-errors") => config.warnings_as_errors = true,
            Some(value) if value.starts_with('-') => {
                return Err(format!("unknown option `{value}`"));
            }
            _ if path.is_none() => path = Some(PathBuf::from(&arguments[index])),
            _ => return Err("`jadren test` accepts exactly one file or directory path".to_owned()),
        }
        index += 1;
    }

    path.map(|path| (path, config)).ok_or_else(|| {
        "usage: jadren test <file.jdn|directory> [--format text|json] [--target <triple>] [--edition <edition>] [--warnings-as-errors]"
            .to_owned()
    })
}

fn parse_doc_arguments(
    arguments: &[OsString],
) -> Result<(PathBuf, PathBuf, CompilerConfig), String> {
    let mut path = None;
    let mut output = PathBuf::from("JADREN_API.md");
    let mut config = CompilerConfig::default();
    let mut index = 0;

    while index < arguments.len() {
        match arguments[index].to_str() {
            Some("--output") => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    return Err("`--output` requires a Markdown path".to_owned());
                };
                output = PathBuf::from(value);
            }
            Some("--target") => {
                index += 1;
                let Some(value) = arguments.get(index).and_then(|value| value.to_str()) else {
                    return Err("`--target` requires a canonical target triple".to_owned());
                };
                config.target = TargetTriple::from_str(value).map_err(|error| error.to_string())?;
            }
            Some("--edition") => {
                index += 1;
                let Some(value) = arguments.get(index).and_then(|value| value.to_str()) else {
                    return Err("`--edition` requires a supported edition".to_owned());
                };
                config.edition = Edition::from_str(value).map_err(|error| error.to_string())?;
            }
            Some("--warnings-as-errors") => config.warnings_as_errors = true,
            Some(value) if value.starts_with('-') => {
                return Err(format!("unknown option `{value}`"));
            }
            _ if path.is_none() => path = Some(PathBuf::from(&arguments[index])),
            _ => return Err("`jadren doc` accepts exactly one source path".to_owned()),
        }
        index += 1;
    }

    path.map(|path| (path, output, config)).ok_or_else(|| {
        "usage: jadren doc <file.jdn|directory> [--output <file.md>] [--target <triple>] [--edition <edition>] [--warnings-as-errors]"
            .to_owned()
    })
}

fn render_diagnostics(
    diagnostics: &[jadren_diagnostics::Diagnostic],
    sources: &SourceManager,
    format: DiagnosticFormat,
) {
    match format {
        DiagnosticFormat::Text => {
            for diagnostic in diagnostics {
                eprint!("{}", render_text(diagnostic, sources));
            }
        }
        DiagnosticFormat::Json => {
            println!("[");
            for (index, diagnostic) in diagnostics.iter().enumerate() {
                let comma = if index + 1 == diagnostics.len() {
                    ""
                } else {
                    ","
                };
                println!("  {}{comma}", render_json(diagnostic, sources));
            }
            println!("]");
        }
    }
}

fn print_help() {
    println!(
        "Jadren compiler {VERSION}\n\n\
         Usage:\n  \
           jadren version\n  \
           jadren doctor\n  \
           jadren check <file.jdn> [--format text|json] [--target <triple>] [--edition <edition>] [--warnings-as-errors]\n  \
           jadren build <file.jdn> [-o <output.exe>] [--profile debug|release] [--cpu baseline|avx2]\n  \
           jadren run <file.jdn> [-o <output.exe>] [--profile debug|release] [--cpu baseline|avx2]\n  \
           jadren test <file.jdn|directory> [--format text|json] [--target <triple>] [--edition <edition>] [--warnings-as-errors]\n  \
           jadren doc <file.jdn|directory> [--output <file.md>] [--target <triple>] [--edition <edition>] [--warnings-as-errors]\n  \
           jadren init [directory] [--name <package>]\n  \
           jadren lock [directory|jadren.toml]\n  \
           jadren resolve [directory|jadren.toml]\n  \
           jadren toolchain verify <manifest> <artifact>\n  \
           jadren toolchain install <manifest> <artifact> <root>\n  \
           jadren sbom [Cargo.lock] [--output <file>]\n  \
           jadren provenance [Cargo.lock] --source-fingerprint <id> --toolchain <id> [--output <file>]\n  \
           jadren migrate [directory|jadren.toml] --from <edition> --to <edition> [--check|--write]\n  \
           jadren format <file.jdn> [--write|--check]\n  \
           jadren lsp\n  \
           jadren emit tokens|syntax|ast|hir|mir|effects|jir|remarks|header|csharp|facade|abi-tests|llvm|asm <file.jdn>\n  \
           jadren emit object <file.jdn> <output.obj> [--target <triple>] [--profile debug|release] [--cpu baseline|avx2|neon]\n"
    );
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use jadren_driver::{BuildProfile, DiagnosticFormat};

    use super::{
        parse_check_arguments, parse_doc_arguments, parse_object_arguments, parse_test_arguments,
    };

    #[test]
    fn parses_check_path_and_json_format() {
        let arguments = vec![
            OsString::from("sample.jdn"),
            OsString::from("--format"),
            OsString::from("json"),
        ];
        let parsed = parse_check_arguments(&arguments).expect("valid arguments");
        assert_eq!(parsed.0.to_string_lossy(), "sample.jdn");
        assert_eq!(parsed.1.diagnostic_format, DiagnosticFormat::Json);
    }

    #[test]
    fn rejects_multiple_paths() {
        let arguments = vec![OsString::from("a.jdn"), OsString::from("b.jdn")];
        assert!(parse_check_arguments(&arguments).is_err());
    }

    #[test]
    fn parses_target_and_warning_policy() {
        let arguments = vec![
            OsString::from("sample.jdn"),
            OsString::from("--target"),
            OsString::from("X86_64-PC-WINDOWS-MSVC"),
            OsString::from("--warnings-as-errors"),
        ];
        let (_, config) = parse_check_arguments(&arguments).expect("valid arguments");
        assert_eq!(config.target.as_str(), "x86_64-pc-windows-msvc");
        assert!(config.warnings_as_errors);
    }

    #[test]
    fn parses_supported_edition_and_rejects_unknown_edition() {
        let arguments = vec![
            OsString::from("sample.jdn"),
            OsString::from("--edition"),
            OsString::from("2026"),
        ];
        let (_, config) = parse_check_arguments(&arguments).expect("supported edition");
        assert_eq!(config.edition.package_spelling(), "2026");
        let invalid = vec![
            OsString::from("sample.jdn"),
            OsString::from("--edition"),
            OsString::from("2027"),
        ];
        assert!(parse_check_arguments(&invalid).is_err());
    }

    #[test]
    fn parses_test_directory_and_json_format() {
        let arguments = vec![
            OsString::from("tests"),
            OsString::from("--format"),
            OsString::from("json"),
        ];
        let parsed = parse_test_arguments(&arguments).expect("valid test arguments");
        assert_eq!(parsed.0.to_string_lossy(), "tests");
        assert_eq!(parsed.1.diagnostic_format, DiagnosticFormat::Json);
    }

    #[test]
    fn parses_release_avx2_object_profile() {
        let arguments = vec![
            OsString::from("object"),
            OsString::from("kernel.jdn"),
            OsString::from("kernel.obj"),
            OsString::from("--profile"),
            OsString::from("release"),
            OsString::from("--cpu"),
            OsString::from("avx2"),
        ];
        let parsed = parse_object_arguments(&arguments).expect("valid object arguments");
        assert_eq!(parsed.source.to_string_lossy(), "kernel.jdn");
        assert_eq!(parsed.output.to_string_lossy(), "kernel.obj");
        assert_eq!(parsed.profile, BuildProfile::Release);
        assert_eq!(parsed.cpu, "avx2");
    }

    #[test]
    fn rejects_unknown_object_profile_and_accepts_neon_cpu() {
        let profile = vec![
            OsString::from("object"),
            OsString::from("kernel.jdn"),
            OsString::from("kernel.obj"),
            OsString::from("--profile"),
            OsString::from("fast"),
        ];
        assert!(parse_object_arguments(&profile).is_err());
        let neon = vec![
            OsString::from("object"),
            OsString::from("kernel.jdn"),
            OsString::from("kernel.obj"),
            OsString::from("--cpu"),
            OsString::from("neon"),
        ];
        let parsed_neon = parse_object_arguments(&neon).expect("valid neon object arguments");
        assert_eq!(parsed_neon.cpu, "neon");
        let invalid_cpu = vec![
            OsString::from("object"),
            OsString::from("kernel.jdn"),
            OsString::from("kernel.obj"),
            OsString::from("--cpu"),
            OsString::from("sse4"),
        ];
        assert!(parse_object_arguments(&invalid_cpu).is_err());
    }

    #[test]
    fn parses_doc_output_path() {
        let arguments = vec![
            OsString::from("examples"),
            OsString::from("--output"),
            OsString::from("api.md"),
        ];
        let parsed = parse_doc_arguments(&arguments).expect("valid doc arguments");
        assert_eq!(parsed.0.to_string_lossy(), "examples");
        assert_eq!(parsed.1.to_string_lossy(), "api.md");
    }
}
