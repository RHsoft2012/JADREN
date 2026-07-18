use std::env;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// PE subsystem selected for the freestanding Jadren executable.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WindowsSubsystem {
    #[default]
    Console,
    Windows,
}

/// Deterministic lld-link inputs for JAD-609.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowsLinkOptions {
    pub entry_symbol: String,
    pub subsystem: WindowsSubsystem,
}

impl Default for WindowsLinkOptions {
    fn default() -> Self {
        Self {
            entry_symbol: "jadren_entry".to_owned(),
            subsystem: WindowsSubsystem::Console,
        }
    }
}

/// Failure before a valid PE32+ executable exists.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LinkError {
    InvalidEntrySymbol(String),
    MissingObject(String),
    NoObjects,
    NoExports,
    LinkerMissing(String),
    Launch(String),
    Failed { code: Option<i32>, output: String },
    MissingOutput(String),
}

impl fmt::Display for LinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEntrySymbol(symbol) => {
                write!(formatter, "invalid Windows entry symbol `{symbol}`")
            }
            Self::MissingObject(path) => write!(formatter, "object file does not exist: `{path}`"),
            Self::NoObjects => formatter.write_str("executable link requires at least one object"),
            Self::NoExports => {
                formatter.write_str("shared library link requires at least one export")
            }
            Self::LinkerMissing(path) => write!(formatter, "pinned lld-link is missing: `{path}`"),
            Self::Launch(message) => write!(formatter, "failed to launch lld-link: {message}"),
            Self::Failed { code, output } => {
                write!(formatter, "lld-link failed with {code:?}: {output}")
            }
            Self::MissingOutput(path) => {
                write!(formatter, "lld-link succeeded without output `{path}`")
            }
        }
    }
}

/// Archives AMD64 COFF objects into a deterministic MSVC-compatible static library.
pub fn create_windows_static_library(output: &Path, objects: &[PathBuf]) -> Result<(), LinkError> {
    validate_objects(objects)?;
    let librarian = pinned_tool("llvm-lib.exe")?;
    let mut command = Command::new(librarian);
    command.args(["/nologo", "/machine:x64"]);
    command.arg(format!("/out:{}", output.display()));
    command.args(objects);
    run_link_tool(command, output)
}

/// Links AMD64 COFF objects into a deterministic DLL with an explicit export allowlist.
pub fn link_windows_shared_library(
    output: &Path,
    objects: &[PathBuf],
    exports: &[String],
) -> Result<(), LinkError> {
    validate_objects(objects)?;
    if exports.is_empty() {
        return Err(LinkError::NoExports);
    }
    for export in exports {
        if !valid_symbol(export) {
            return Err(LinkError::InvalidEntrySymbol(export.clone()));
        }
    }
    let linker = pinned_tool("lld-link.exe")?;
    let mut command = Command::new(linker);
    command.args([
        "/nologo",
        "/dll",
        "/noentry",
        "/nodefaultlib",
        "/machine:x64",
        "/Brepro",
        "/manifest:no",
        "/noimplib",
        "/opt:ref,noicf",
    ]);
    command.arg(format!("/out:{}", output.display()));
    for export in exports {
        command.arg(format!("/export:{export}"));
    }
    command.args(objects);
    run_link_tool(command, output)
}

impl Error for LinkError {}

/// Links one or more AMD64 COFF objects into a freestanding deterministic PE32+ executable.
pub fn link_windows_executable(
    output: &Path,
    objects: &[PathBuf],
    options: &WindowsLinkOptions,
) -> Result<(), LinkError> {
    validate_objects(objects)?;
    if !valid_symbol(&options.entry_symbol) {
        return Err(LinkError::InvalidEntrySymbol(options.entry_symbol.clone()));
    }
    let linker = pinned_tool("lld-link.exe")?;
    let subsystem = match options.subsystem {
        WindowsSubsystem::Console => "console",
        WindowsSubsystem::Windows => "windows",
    };
    let mut command = Command::new(linker);
    command.args([
        "/nologo",
        "/nodefaultlib",
        "/machine:x64",
        "/Brepro",
        "/manifest:no",
        "/opt:ref,noicf",
    ]);
    command.arg(format!("/entry:{}", options.entry_symbol));
    command.arg(format!("/subsystem:{subsystem}"));
    command.arg(format!("/out:{}", output.display()));
    command.args(objects);
    run_link_tool(command, output)
}

fn validate_objects(objects: &[PathBuf]) -> Result<(), LinkError> {
    if objects.is_empty() {
        return Err(LinkError::NoObjects);
    }
    for object in objects {
        if !object.is_file() {
            return Err(LinkError::MissingObject(object.display().to_string()));
        }
    }
    Ok(())
}

fn pinned_tool(name: &str) -> Result<PathBuf, LinkError> {
    for variable in ["JADREN_LLVM_PREFIX", "LLVM_SYS_221_PREFIX"] {
        if let Some(prefix) = env::var_os(variable) {
            let tool = PathBuf::from(prefix).join("bin").join(name);
            if tool.is_file() {
                return Ok(tool);
            }
        }
    }
    let tool = Path::new(env!("JADREN_LLVM_BIN")).join(name);
    if tool.is_file() {
        Ok(tool)
    } else {
        Err(LinkError::LinkerMissing(tool.display().to_string()))
    }
}

fn run_link_tool(mut command: Command, output: &Path) -> Result<(), LinkError> {
    let result = command
        .output()
        .map_err(|error| LinkError::Launch(error.to_string()))?;
    if !result.status.success() {
        let mut output = String::from_utf8_lossy(&result.stdout).into_owned();
        output.push_str(&String::from_utf8_lossy(&result.stderr));
        return Err(LinkError::Failed {
            code: result.status.code(),
            output,
        });
    }
    if !output.is_file() {
        return Err(LinkError::MissingOutput(output.display().to_string()));
    }
    Ok(())
}

fn valid_symbol(symbol: &str) -> bool {
    let mut characters = symbol.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && characters.all(|character| {
            character == '_'
                || character == '.'
                || character == '$'
                || character == '@'
                || character == '?'
                || character.is_ascii_alphanumeric()
        })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use inkwell::context::Context;
    use jadren_jir::{
        Block, BlockId, Constant, Function, FunctionId, Instruction, InstructionKind, Linkage,
        Module, Terminator, Type, TypeId, TypedValue, ValueId,
    };

    use super::{
        WindowsLinkOptions, create_windows_static_library, link_windows_executable,
        link_windows_shared_library,
    };
    use crate::{ObjectOptions, TypeLoweringConfig, lower_to_object, write_object};

    #[test]
    fn links_reproducible_pe_and_runs_freestanding_entry() {
        let context = Context::create();
        let object = lower_to_object(
            &context,
            &entry_module(),
            "entry_object",
            &TypeLoweringConfig::default(),
            &ObjectOptions::default(),
        )
        .expect("entry COFF object");
        let directory = std::env::temp_dir().join(format!("jadren-link-{}", std::process::id()));
        fs::create_dir_all(&directory).expect("create link test directory");
        let object_path = directory.join("entry.obj");
        let first_path = directory.join("first.exe");
        let second_path = directory.join("second.exe");
        write_object(&object_path, &object).expect("write entry object");
        let objects = [object_path];
        link_windows_executable(&first_path, &objects, &WindowsLinkOptions::default())
            .expect("link first executable");
        link_windows_executable(&second_path, &objects, &WindowsLinkOptions::default())
            .expect("link second executable");

        let first = fs::read(&first_path).expect("read first PE");
        let second = fs::read(&second_path).expect("read second PE");
        assert_eq!(first, second, "PE output must be reproducible");
        assert_eq!(&first[..2], b"MZ");
        let status = Command::new(&first_path)
            .status()
            .expect("run freestanding PE");
        assert_eq!(status.code(), Some(42));
        verify_pe(&first_path);

        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn links_reproducible_static_and_shared_libraries_with_explicit_export() {
        let context = Context::create();
        let object = lower_to_object(
            &context,
            &entry_module(),
            "library_object",
            &TypeLoweringConfig::default(),
            &ObjectOptions::default(),
        )
        .expect("library COFF object");
        let directory = std::env::temp_dir().join(format!("jadren-library-{}", std::process::id()));
        fs::create_dir_all(&directory).expect("create library test directory");
        let object_path = directory.join("library.obj");
        let static_first = directory.join("first.lib");
        let static_second = directory.join("second.lib");
        let shared = directory.join("library.dll");
        write_object(&object_path, &object).expect("write library object");
        let objects = [object_path];

        create_windows_static_library(&static_first, &objects).expect("first static library");
        create_windows_static_library(&static_second, &objects).expect("second static library");
        assert_eq!(
            fs::read(&static_first).expect("read first static library"),
            fs::read(&static_second).expect("read second static library")
        );
        assert_eq!(
            &fs::read(&static_first).expect("read static archive")[..8],
            b"!<arch>\n"
        );

        let exports = ["jadren_entry".to_owned()];
        link_windows_shared_library(&shared, &objects, &exports).expect("first DLL");
        let first_dll = fs::read(&shared).expect("read first DLL");
        link_windows_shared_library(&shared, &objects, &exports).expect("second DLL");
        assert_eq!(first_dll, fs::read(&shared).expect("read second DLL"));
        verify_static_symbol(&static_first);
        verify_dll_export(&shared);

        let _ = fs::remove_dir_all(directory);
    }

    fn verify_pe(path: &Path) {
        let output = Command::new(Path::new(env!("JADREN_LLVM_BIN")).join("llvm-readobj.exe"))
            .args(["--file-headers", "--symbols"])
            .arg(path)
            .output()
            .expect("llvm-readobj should inspect PE");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let inspection = String::from_utf8_lossy(&output.stdout);
        assert!(inspection.contains("Format: COFF-x86-64"), "{inspection}");
        assert!(
            inspection.contains("Subsystem: IMAGE_SUBSYSTEM_WINDOWS_CUI"),
            "{inspection}"
        );
        assert!(
            inspection.contains("AddressOfEntryPoint: 0x"),
            "{inspection}"
        );
    }

    fn verify_static_symbol(path: &Path) {
        let output = Command::new(Path::new(env!("JADREN_LLVM_BIN")).join("llvm-nm.exe"))
            .arg(path)
            .output()
            .expect("llvm-nm should inspect static library");
        assert!(output.status.success());
        let inspection = String::from_utf8_lossy(&output.stdout);
        assert!(inspection.contains("jadren_entry"), "{inspection}");
    }

    fn verify_dll_export(path: &Path) {
        let output = Command::new(Path::new(env!("JADREN_LLVM_BIN")).join("llvm-readobj.exe"))
            .arg("--coff-exports")
            .arg(path)
            .output()
            .expect("llvm-readobj should inspect DLL exports");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let inspection = String::from_utf8_lossy(&output.stdout);
        assert!(inspection.contains("Name: jadren_entry"), "{inspection}");
    }

    fn entry_module() -> Module {
        Module {
            types: vec![Type::Integer {
                signed: true,
                bits: 32,
            }],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "jadren_entry".to_owned(),
                linkage: Linkage::Export,
                parameters: Vec::new(),
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(0),
                            ty: TypeId::new(0),
                        }),
                        kind: InstructionKind::Constant(Constant::Integer { value: 42 }),
                        span: None,
                    }],
                    terminator: Terminator::Return {
                        value: Some(ValueId::new(0)),
                    },
                    span: None,
                }],
                span: None,
            }],
        }
    }
}
