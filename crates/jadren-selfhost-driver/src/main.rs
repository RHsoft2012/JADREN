use std::path::{Path, PathBuf};

use inkwell::context::Context;
use jadren_codegen_llvm::{
    ObjectOptions, TypeLoweringConfig, lower_to_object_with_summary, write_object,
};
#[cfg(windows)]
use jadren_codegen_llvm::{WindowsLinkOptions, link_windows_executable};
use jadren_selfhost_stage2::{decode_stage2_capture, import_stage2_jir};
use jadren_source::SourceManager;

fn main() {
    if let Err(message) = run() {
        eprintln!("jadren self-host stage-2 driver failed: {message}");
        std::process::exit(1);
    }
}

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
    let lowering_config = if cfg!(windows) {
        TypeLoweringConfig::x86_64_windows_msvc()
    } else {
        TypeLoweringConfig::x86_64_linux_gnu()
    };
    let (backend_summary, object) = lower_to_object_with_summary(
        &context,
        &module,
        "jadren_selfhost_stage2_driver",
        &lowering_config,
        &ObjectOptions::x86_64_baseline_release(),
    )
    .map_err(|error| error.to_string())?;
    write_object(&arguments.object, &object).map_err(|error| error.to_string())?;

    let mut link_summary: Option<(u64, u64)> = None;
    if let Some(executable) = &arguments.executable {
        #[cfg(not(windows))]
        {
            let _ = executable;
            return Err("--executable is currently supported only on Windows".to_owned());
        }
        #[cfg(windows)]
        {
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
            let executable_bytes = std::fs::metadata(executable)
                .map_err(|error| format!("cannot stat linked executable: {error}"))?
                .len();
            link_summary = Some((executable_bytes, 1_u64));
        }
    }

    println!(
        "stage2-backend pass functions={} blocks={} instructions={} status_flags={} object_bytes={}",
        backend_summary.module_functions,
        backend_summary.module_blocks,
        backend_summary.module_instructions,
        backend_summary.backend_status_flags,
        backend_summary.object_bytes,
    );
    if let Some((executable_bytes, status_flags)) = link_summary {
        println!(
            "stage2-link pass executable_bytes={} status_flags={}",
            executable_bytes, status_flags
        );
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

struct Arguments {
    capture: PathBuf,
    object: PathBuf,
    executable: Option<PathBuf>,
    entry: String,
}

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

fn create_parent(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create `{}`: {error}", parent.display()))
}

fn usage() -> String {
    "usage: jadren-selfhost-driver <capture.bin> <output.obj> [--executable <output.exe>] [--entry <symbol>]".to_owned()
}
