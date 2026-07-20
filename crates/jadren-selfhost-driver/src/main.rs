#[cfg(windows)]
use std::path::{Path, PathBuf};

#[cfg(windows)]
use inkwell::context::Context;
#[cfg(windows)]
use jadren_codegen_llvm::{
    ObjectOptions, TypeLoweringConfig, WindowsLinkOptions, link_windows_executable,
    lower_to_object, write_object,
};
#[cfg(windows)]
use jadren_selfhost_stage2::{decode_stage2_capture, import_stage2_jir};
#[cfg(windows)]
use jadren_source::SourceManager;

#[cfg(windows)]
fn main() {
    if let Err(message) = run() {
        eprintln!("jadren self-host stage-2 driver failed: {message}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("jadren self-host stage-2 driver currently supports Windows x86-64 only");
    std::process::exit(1);
}

#[cfg(windows)]
fn run() -> Result<(), String> {
    let arguments = parse_arguments()?;
    let capture_bytes = std::fs::read(&arguments.capture)
        .map_err(|error| format!("cannot read `{}`: {error}", arguments.capture.display()))?;
    let capture = decode_stage2_capture(&capture_bytes).map_err(|error| error.to_string())?;
    let mut sources = SourceManager::new();
    let source_id = sources
        .add(arguments.capture.clone(), capture.source.clone())
        .map_err(|error| error.to_string())?;
    let module = import_stage2_jir(
        &capture.source,
        source_id,
        capture.summary,
        &capture.records,
    )
    .map_err(|error| error.to_string())?;
    let constant_instructions = module
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .filter(|instruction| {
            matches!(
                instruction.kind,
                jadren_jir::InstructionKind::Constant(jadren_jir::Constant::Integer { .. })
            )
        })
        .count();
    let parameter_functions = module
        .functions
        .iter()
        .filter(|function| function.parameters.len() == 1)
        .count();
    let binary_adds = module
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .filter(|instruction| {
            matches!(
                instruction.kind,
                jadren_jir::InstructionKind::Binary {
                    op: jadren_jir::BinaryOp::Add,
                    ..
                }
            )
        })
        .count();
    let binary_subtracts = module
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .filter(|instruction| {
            matches!(
                instruction.kind,
                jadren_jir::InstructionKind::Binary {
                    op: jadren_jir::BinaryOp::Subtract,
                    ..
                }
            )
        })
        .count();
    let binary_multiplies = module
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .filter(|instruction| {
            matches!(
                instruction.kind,
                jadren_jir::InstructionKind::Binary {
                    op: jadren_jir::BinaryOp::Multiply,
                    ..
                }
            )
        })
        .count();
    let binary_chain_functions = module
        .functions
        .iter()
        .filter(|function| {
            function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .filter(|instruction| {
                    matches!(instruction.kind, jadren_jir::InstructionKind::Binary { .. })
                })
                .count()
                == 2
        })
        .count();
    let long_binary_chain_functions = module
        .functions
        .iter()
        .filter(|function| {
            function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .filter(|instruction| {
                    matches!(instruction.kind, jadren_jir::InstructionKind::Binary { .. })
                })
                .count()
                == 3
        })
        .count();
    let expression_plan_functions = module
        .functions
        .iter()
        .filter(|function| {
            function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .filter(|instruction| {
                    matches!(instruction.kind, jadren_jir::InstructionKind::Binary { .. })
                })
                .count()
                == 4
        })
        .count();
    let grouped_binary_chain_functions = module
        .functions
        .iter()
        .filter(|function| {
            let binary_instructions = function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .filter(|instruction| {
                    matches!(instruction.kind, jadren_jir::InstructionKind::Binary { .. })
                })
                .collect::<Vec<_>>();
            binary_instructions.len() == 2
                && binary_instructions.iter().any(|instruction| {
                    instruction.span.is_some_and(|span| {
                        capture.source[span.start..span.end].trim().starts_with('(')
                            && capture.source[span.start..span.end].trim().ends_with(')')
                    })
                })
        })
        .count();
    let grouped_expression_plan_functions = module
        .functions
        .iter()
        .filter(|function| {
            let binary_instructions = function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .filter(|instruction| {
                    matches!(instruction.kind, jadren_jir::InstructionKind::Binary { .. })
                })
                .collect::<Vec<_>>();
            (binary_instructions.len() == 3 || binary_instructions.len() == 4)
                && binary_instructions.iter().any(|instruction| {
                    instruction.span.is_some_and(|span| {
                        capture.source[span.start..span.end].trim().starts_with('(')
                            && capture.source[span.start..span.end].trim().ends_with(')')
                    })
                })
        })
        .count();
    let streaming_expression_plan_functions = module
        .functions
        .iter()
        .filter(|function| {
            let binary_count = function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .filter(|instruction| {
                    matches!(instruction.kind, jadren_jir::InstructionKind::Binary { .. })
                })
                .count();
            (5..=16).contains(&binary_count)
        })
        .count();
    let grouped_streaming_expression_plan_functions = module
        .functions
        .iter()
        .filter(|function| {
            let binary_instructions = function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .filter(|instruction| {
                    matches!(instruction.kind, jadren_jir::InstructionKind::Binary { .. })
                })
                .collect::<Vec<_>>();
            (5..=16).contains(&binary_instructions.len())
                && binary_instructions.iter().any(|instruction| {
                    instruction.span.is_some_and(|span| {
                        capture.source[span.start..span.end].trim().starts_with('(')
                            && capture.source[span.start..span.end].trim().ends_with(')')
                    })
                })
        })
        .count();
    let multi_group_streaming_expression_plan_functions = module
        .functions
        .iter()
        .filter(|function| {
            let binary_instructions = function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .filter(|instruction| {
                    matches!(instruction.kind, jadren_jir::InstructionKind::Binary { .. })
                })
                .collect::<Vec<_>>();
            (5..=16).contains(&binary_instructions.len())
                && binary_instructions
                    .iter()
                    .filter(|instruction| {
                        instruction.span.is_some_and(|span| {
                            capture.source[span.start..span.end].trim().starts_with('(')
                                && capture.source[span.start..span.end].trim().ends_with(')')
                        })
                    })
                    .count()
                    >= 2
        })
        .count();
    let nested_streaming_expression_plan_functions = module
        .functions
        .iter()
        .filter(|function| {
            let binary_instructions = function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .filter(|instruction| {
                    matches!(instruction.kind, jadren_jir::InstructionKind::Binary { .. })
                })
                .collect::<Vec<_>>();
            (5..=16).contains(&binary_instructions.len())
                && binary_instructions.iter().any(|instruction| {
                    instruction.span.is_some_and(|span| {
                        capture.source[span.start..span.end]
                            .trim()
                            .starts_with("((")
                            && capture.source[span.start..span.end].trim().ends_with("))")
                    })
                })
        })
        .count();

    create_parent(&arguments.object)?;
    let context = Context::create();
    let object = lower_to_object(
        &context,
        &module,
        "jadren_selfhost_stage2_driver",
        &TypeLoweringConfig::default(),
        &ObjectOptions::x86_64_baseline_release(),
    )
    .map_err(|error| error.to_string())?;
    write_object(&arguments.object, &object).map_err(|error| error.to_string())?;

    if let Some(executable) = &arguments.executable {
        create_parent(executable)?;
        link_windows_executable(
            executable,
            std::slice::from_ref(&arguments.object),
            &WindowsLinkOptions {
                entry_symbol: arguments.entry.clone(),
                ..WindowsLinkOptions::default()
            },
        )
        .map_err(|error| error.to_string())?;
    }

    println!(
        "stage2-driver pass functions={} records={} constants={} parameter_functions={} binary_adds={} binary_subtracts={} binary_multiplies={} binary_chain_functions={} long_binary_chain_functions={} expression_plan_functions={} streaming_expression_plan_functions={} grouped_binary_chain_functions={} grouped_expression_plan_functions={} grouped_streaming_expression_plan_functions={} multi_group_streaming_expression_plan_functions={} nested_streaming_expression_plan_functions={} object_bytes={} entry={}",
        module.functions.len(),
        capture.records.len(),
        constant_instructions,
        parameter_functions,
        binary_adds,
        binary_subtracts,
        binary_multiplies,
        binary_chain_functions,
        long_binary_chain_functions,
        expression_plan_functions,
        streaming_expression_plan_functions,
        grouped_binary_chain_functions,
        grouped_expression_plan_functions,
        grouped_streaming_expression_plan_functions,
        multi_group_streaming_expression_plan_functions,
        nested_streaming_expression_plan_functions,
        object.len(),
        arguments.entry
    );
    Ok(())
}

#[cfg(windows)]
struct Arguments {
    capture: PathBuf,
    object: PathBuf,
    executable: Option<PathBuf>,
    entry: String,
}

#[cfg(windows)]
fn parse_arguments() -> Result<Arguments, String> {
    let mut values = std::env::args_os().skip(1);
    let capture = values.next().map(PathBuf::from).ok_or_else(usage)?;
    let object = values.next().map(PathBuf::from).ok_or_else(usage)?;
    let mut executable = None;
    let mut entry = "main".to_owned();
    while let Some(argument) = values.next() {
        let argument = argument
            .into_string()
            .map_err(|_| "driver option is not valid UTF-8".to_owned())?;
        match argument.as_str() {
            "--executable" => {
                if executable.is_some() {
                    return Err("--executable may be supplied only once".to_owned());
                }
                executable = Some(values.next().map(PathBuf::from).ok_or_else(usage)?);
            }
            "--entry" => {
                entry = values
                    .next()
                    .ok_or_else(usage)?
                    .into_string()
                    .map_err(|_| "entry symbol is not valid UTF-8".to_owned())?;
            }
            _ => return Err(format!("unknown driver option `{argument}`\n{}", usage())),
        }
    }
    Ok(Arguments {
        capture,
        object,
        executable,
        entry,
    })
}

#[cfg(windows)]
fn create_parent(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create `{}`: {error}", parent.display()))
}

#[cfg(windows)]
fn usage() -> String {
    "usage: jadren-selfhost-driver <capture.bin> <output.obj> [--executable <output.exe>] [--entry <symbol>]".to_owned()
}
