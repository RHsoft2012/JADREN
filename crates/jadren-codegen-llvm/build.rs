use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const LLVM_VERSION: &str = "22.1.8";

fn main() {
    println!("cargo:rerun-if-env-changed=JADREN_LLVM_PREFIX");
    println!("cargo:rerun-if-env-changed=LLVM_SYS_221_PREFIX");
    println!("cargo:rerun-if-env-changed=LLVM_SYS_221_STRICT_VERSIONING");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-env=JADREN_LLVM_VERSION={LLVM_VERSION}");

    match env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("windows") => configure_windows(),
        Ok("linux") => configure_unix(),
        _ => {}
    }
}

fn configure_windows() {
    let prefix = env::var_os("JADREN_LLVM_PREFIX")
        .map(PathBuf::from)
        .unwrap_or_else(default_llvm_prefix);
    let import_library = prefix.join("lib").join("LLVM-C.lib");
    let runtime_library = prefix.join("bin").join("LLVM-C.dll");
    if !import_library.is_file() || !runtime_library.is_file() {
        panic!(
            "LLVM {LLVM_VERSION} is missing at `{}`; run `scripts/bootstrap-llvm.ps1` or set JADREN_LLVM_PREFIX",
            prefix.display()
        );
    }

    println!(
        "cargo:rustc-link-search=native={}",
        prefix.join("lib").display()
    );
    println!("cargo:rustc-link-lib=dylib=LLVM-C");
    println!(
        "cargo:rustc-env=JADREN_LLVM_BIN={}",
        prefix.join("bin").display()
    );
    cc::Build::new()
        .file("src/target_wrappers.c")
        .warnings(true)
        .compile("jadren_llvm_target_wrappers");
    copy_runtime_library(&runtime_library);
}

fn configure_unix() {
    let llvm_config = locate_llvm_config().unwrap_or_else(|| {
        panic!(
            "LLVM {LLVM_VERSION} was not found; install the pinned Linux toolchain or set JADREN_LLVM_PREFIX/LLVM_SYS_221_PREFIX"
        )
    });
    let version = run_llvm_config(&llvm_config, &["--version"]);
    let actual_version = version.split_whitespace().next().unwrap_or_default();
    if actual_version != LLVM_VERSION {
        panic!(
            "LLVM version mismatch: expected {LLVM_VERSION}, found `{actual_version}` at `{}`",
            llvm_config.display()
        );
    }

    let bin_dir = llvm_config
        .parent()
        .expect("llvm-config must have a parent directory")
        .to_owned();
    let prefix = bin_dir
        .parent()
        .expect("llvm-config bin directory must have a prefix")
        .to_owned();
    let lib_dir = PathBuf::from(run_llvm_config(&llvm_config, &["--libdir"]));
    if !lib_dir.is_dir() {
        panic!(
            "LLVM {LLVM_VERSION} reported a missing library directory `{}`",
            lib_dir.display()
        );
    }

    let (libraries, link_mode) = try_llvm_config(&llvm_config, &["--libnames", "--link-shared"])
        .ok()
        .and_then(|output| parse_library_names(&output))
        .map(|libraries| (libraries, "dylib"))
        .or_else(|| {
            try_llvm_config(&llvm_config, &["--libs", "--link-shared"])
                .ok()
                .and_then(|output| parse_library_names(&output))
                .map(|libraries| (libraries, "dylib"))
        })
        .or_else(|| {
            try_llvm_config(&llvm_config, &["--libnames", "--link-static"])
                .ok()
                .and_then(|output| parse_library_names(&output))
                .map(|libraries| (libraries, "static"))
        })
        .or_else(|| {
            try_llvm_config(&llvm_config, &["--libs", "--link-static"])
                .ok()
                .and_then(|output| parse_library_names(&output))
                .map(|libraries| (libraries, "static"))
        })
        .unwrap_or_else(|| {
            panic!(
                "llvm-config did not report usable shared or static LLVM libraries for `{}`",
                llvm_config.display()
            )
        });

    println!("cargo:rustc-env=JADREN_LLVM_BIN={}", bin_dir.display());

    // The wrapper intentionally declares the small target-initialization ABI
    // itself, so no LLVM headers are needed here. The C object is linked into
    // the Rust crate and calls the symbols exported by the selected LLVM build.
    cc::Build::new()
        .file("src/target_wrappers.c")
        .warnings(true)
        .compile("jadren_llvm_target_wrappers");

    // Emit the wrapper archive before LLVM libraries so ELF linkers do not
    // discard the dependency under their default --as-needed mode.
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    for library in libraries {
        println!("cargo:rustc-link-lib={link_mode}={library}");
    }
    let system_link_mode = if link_mode == "static" {
        "--link-static"
    } else {
        "--link-shared"
    };
    for token in
        run_llvm_config(&llvm_config, &["--system-libs", system_link_mode]).split_whitespace()
    {
        emit_system_link_token(token);
    }
    if link_mode == "static" {
        // The official Linux archive ships LLVM as static C++ archives and
        // llvm-config does not list the compiler's C++ runtime dependency.
        println!("cargo:rustc-link-lib=dylib=stdc++");
    }
    if link_mode == "dylib" {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
    }

    println!(
        "cargo:warning=Jadren LLVM {LLVM_VERSION} Linux backend configured from `{}` ({link_mode})",
        prefix.display(),
    );
}

fn locate_llvm_config() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    for variable in ["JADREN_LLVM_PREFIX", "LLVM_SYS_221_PREFIX"] {
        if let Some(prefix) = env::var_os(variable) {
            let prefix = PathBuf::from(prefix);
            candidates.push(prefix.join("bin").join("llvm-config"));
            candidates.push(prefix.join("bin").join("llvm-config-22"));
        }
    }
    if let Some(path) = find_on_path(&["llvm-config", "llvm-config-22"]) {
        candidates.push(path);
    }
    candidates.into_iter().find(|path| path.is_file())
}

fn find_on_path(names: &[&str]) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    for directory in env::split_paths(&path) {
        for name in names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn run_llvm_config(path: &Path, arguments: &[&str]) -> String {
    try_llvm_config(path, arguments).unwrap_or_else(|error| panic!("{error}"))
}

fn try_llvm_config(path: &Path, arguments: &[&str]) -> Result<String, String> {
    let output = Command::new(path)
        .args(arguments)
        .output()
        .map_err(|error| format!("failed to execute `{}`: {error}", path.display()))?;
    if !output.status.success() {
        return Err(format!(
            "`{}` {:?} failed: {}",
            path.display(),
            arguments,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map(|output| output.trim().to_owned())
        .map_err(|error| format!("llvm-config returned non-UTF-8 output: {error}"))
}

fn parse_library_names(output: &str) -> Option<Vec<String>> {
    let mut libraries = BTreeSet::new();
    for token in output.split_whitespace() {
        if let Some(name) = normalize_library_name(token) {
            libraries.insert(name);
        }
    }
    if libraries.is_empty() {
        None
    } else {
        Some(libraries.into_iter().collect())
    }
}

fn normalize_library_name(token: &str) -> Option<String> {
    if let Some(name) = token.strip_prefix("-l") {
        return (!name.is_empty()).then(|| name.to_owned());
    }
    let filename = Path::new(token).file_name()?.to_str()?;
    let filename = filename.strip_prefix("lib").unwrap_or(filename);
    let stem = filename
        .split_once(".so")
        .map(|(stem, _)| stem)
        .or_else(|| filename.strip_suffix(".dylib"))
        .or_else(|| filename.strip_suffix(".a"))
        .or_else(|| filename.strip_suffix(".lib"))?;
    (!stem.is_empty()).then(|| stem.to_owned())
}

fn emit_system_link_token(token: &str) {
    if let Some(name) = token.strip_prefix("-l") {
        if !name.is_empty() {
            println!("cargo:rustc-link-lib=dylib={name}");
        }
    } else if let Some(path) = token.strip_suffix(".a") {
        let path = Path::new(path);
        if let (Some(parent), Some(file)) = (
            path.parent(),
            path.file_stem().and_then(|name| name.to_str()),
        ) {
            let name = file.strip_prefix("lib").unwrap_or(file);
            println!("cargo:rustc-link-search=native={}", parent.display());
            println!("cargo:rustc-link-lib=static={name}");
        }
    } else if token.starts_with("-Wl,") || token == "-pthread" {
        println!("cargo:rustc-link-arg={token}");
    }
}

fn default_llvm_prefix() -> PathBuf {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let workspace = manifest
        .parent()
        .and_then(Path::parent)
        .expect("backend crate must be inside workspace/crates");
    workspace
        .parent()
        .expect("workspace must have a parent directory")
        .join("Toolchains")
        .join(format!("LLVM-{LLVM_VERSION}"))
}

fn copy_runtime_library(source: &Path) {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    let profile_dir = out_dir
        .ancestors()
        .nth(3)
        .expect("Cargo OUT_DIR must be under target/<profile>/build/<package>/out");
    for destination_dir in [profile_dir.to_owned(), profile_dir.join("deps")] {
        fs::create_dir_all(&destination_dir).expect("create Cargo runtime directory");
        fs::copy(source, destination_dir.join("LLVM-C.dll"))
            .expect("copy LLVM-C.dll beside Cargo binaries");
    }
}
