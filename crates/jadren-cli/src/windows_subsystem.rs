//! PE subsystem selection is independent of conservative runtime linking.
//! Unused library UI imports must not detach a console program from its terminal.

use std::collections::{BTreeMap, BTreeSet};

use jadren_jir::{InstructionKind, Linkage, Module};

/// Inspect verified JIR after package symbol promotion and entry preparation.
/// Follow calls and address-taken functions, including cross-module definitions.
/// Merely exporting a library function does not make it an executable root.
pub(crate) fn requires_gui(modules: &[&Module], entry_index: usize) -> bool {
    let exports: BTreeMap<_, _> = modules
        .iter()
        .enumerate()
        .flat_map(|(module_index, module)| {
            module.functions.iter().filter_map(move |function| {
                (function.linkage == Linkage::Export)
                    .then_some((function.name.as_str(), (module_index, function.id.index())))
            })
        })
        .collect();
    let Some(entry) = modules[entry_index]
        .functions
        .iter()
        .find(|function| function.name == "jadren_entry" && function.linkage == Linkage::Export)
    else {
        return false;
    };
    let mut pending = vec![(entry_index, entry.id.index())];
    let mut visited = BTreeSet::new();
    while let Some((module_index, function_index)) = pending.pop() {
        if !visited.insert((module_index, function_index)) {
            continue;
        }
        let function = &modules[module_index].functions[function_index];
        if function.linkage == Linkage::Import {
            if let Some(&definition) = exports.get(function.name.as_str()) {
                pending.push(definition);
            } else if matches!(
                function.name.as_str(),
                "jadren_win32_desktop_run"
                    | "jadren_win32_window_begin"
                    | "jadren_win32_window_run"
                    | "ui_window"
                    | "ui_run"
                    | "ui_app_begin"
                    | "ui_app_run"
                    | "ui_file_open_exact"
                    | "ui_directory_open_exact"
                    | "ui_file_open_extension_exact"
                    | "ui_file_save_exact"
                    | "ui_file_save_suggested_exact"
                    | "ui_file_save_extension_exact"
            ) {
                return true;
            }
            continue;
        }
        for instruction in function.blocks.iter().flat_map(|block| &block.instructions) {
            if let InstructionKind::Call { function, .. }
            | InstructionKind::FunctionAddress { function } = instruction.kind
            {
                pending.push((module_index, function.index()));
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use jadren_jir::{
        Block, BlockId, Function, FunctionId, Instruction, Terminator, Type, TypeId, TypedValue,
        ValueId, verify,
    };

    fn module(definitions: &[(&str, Linkage, &[usize])]) -> Module {
        let module = Module {
            types: vec![Type::Unit],
            functions: definitions
                .iter()
                .enumerate()
                .map(|(index, &(name, linkage, calls))| Function {
                    id: FunctionId::new(index),
                    name: name.to_owned(),
                    linkage,
                    parameters: vec![],
                    result: TypeId::new(0),
                    blocks: if linkage == Linkage::Import {
                        vec![]
                    } else {
                        vec![Block {
                            id: BlockId::new(0),
                            parameters: vec![],
                            instructions: calls
                                .iter()
                                .map(|&callee| Instruction {
                                    result: None,
                                    kind: InstructionKind::Call {
                                        function: FunctionId::new(callee),
                                        arguments: vec![],
                                    },
                                    span: None,
                                })
                                .collect(),
                            terminator: Terminator::Return { value: None },
                            span: None,
                        }]
                    },
                    span: None,
                })
                .collect(),
        };
        assert!(verify(&module).is_empty());
        module
    }

    #[test]
    fn unused_ui_imports_and_library_exports_keep_console() {
        let entry = module(&[("jadren_entry", Linkage::Export, &[])]);
        let library = module(&[
            ("unused_window", Linkage::Export, &[1]),
            ("ui_app_begin", Linkage::Import, &[]),
        ]);
        assert!(!requires_gui(&[&entry, &library], 0));
    }

    #[test]
    fn reachable_window_entrypoints_select_gui() {
        for name in [
            "jadren_win32_desktop_run",
            "jadren_win32_window_begin",
            "jadren_win32_window_run",
            "ui_window",
            "ui_run",
            "ui_app_begin",
            "ui_app_run",
        ] {
            let entry = module(&[
                ("jadren_entry", Linkage::Export, &[1]),
                (name, Linkage::Import, &[]),
            ]);
            assert!(requires_gui(&[&entry], 0), "{name}");
        }
    }

    #[test]
    fn follows_promoted_package_calls_from_the_selected_entry() {
        let entry = module(&[
            ("jadren_entry", Linkage::Export, &[1]),
            ("package_window", Linkage::Import, &[]),
        ]);
        let library = module(&[
            ("package_window", Linkage::Export, &[1]),
            ("ui_window", Linkage::Import, &[]),
        ]);
        assert!(requires_gui(&[&library, &entry], 1));
    }

    #[test]
    fn follows_address_taken_functions_conservatively() {
        let mut entry = module(&[
            ("jadren_entry", Linkage::Export, &[]),
            ("window_callback", Linkage::Internal, &[2]),
            ("ui_app_begin", Linkage::Import, &[]),
        ]);
        entry.types.push(Type::Function {
            parameters: vec![],
            result: TypeId::new(0),
        });
        entry.functions[0].blocks[0].instructions.push(Instruction {
            result: Some(TypedValue {
                value: ValueId::new(0),
                ty: TypeId::new(1),
            }),
            kind: InstructionKind::FunctionAddress {
                function: FunctionId::new(1),
            },
            span: None,
        });
        assert!(verify(&entry).is_empty());
        assert!(requires_gui(&[&entry], 0));
    }

    #[test]
    fn reachable_file_dialog_calls_select_gui() {
        for dialog in ["ui_file_open_exact", "ui_directory_open_exact"] {
            let entry = module(&[
                ("jadren_entry", Linkage::Export, &[1]),
                ("recursive_helper", Linkage::Internal, &[1, 2]),
                (dialog, Linkage::Import, &[]),
            ]);
            assert!(requires_gui(&[&entry], 0), "{dialog}");
        }
    }

    #[test]
    fn internal_names_do_not_resolve_external_imports() {
        let entry = module(&[
            ("jadren_entry", Linkage::Export, &[1]),
            ("unresolved_helper", Linkage::Import, &[]),
        ]);
        let library = module(&[
            ("unresolved_helper", Linkage::Internal, &[1]),
            ("ui_window", Linkage::Import, &[]),
        ]);
        assert!(!requires_gui(&[&entry, &library], 0));
    }
}
