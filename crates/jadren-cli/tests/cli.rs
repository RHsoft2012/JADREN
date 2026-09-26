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
        ("callback-region-escape.jdn", "J0507"),
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
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .starts_with(concat!("jadren ", env!("CARGO_PKG_VERSION")))
    );
}

#[test]
fn init_desktop_template_creates_checkable_native_app() {
    let directory = std::env::temp_dir().join(format!(
        "jadren-cli-init-desktop-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    if directory.exists() {
        fs::remove_dir_all(&directory).expect("old template directory should be removable");
    }
    fs::create_dir_all(&directory).expect("template working directory should be creatable");

    let init = Command::new(binary())
        .args(["init", "--template", "desktop", "--name", "native_app"])
        .current_dir(&directory)
        .output()
        .expect("jadren init should start");
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );
    assert!(directory.join("jadren.toml").is_file());
    assert!(directory.join("jadren.lock").is_file());
    assert!(directory.join("README.md").is_file());
    let source = directory.join("src/main.jdn");
    assert!(source.is_file());
    let source_text = fs::read_to_string(&source).expect("desktop source should be readable");
    assert!(source_text.contains("module native_app.main"));
    assert!(source_text.contains("@export(name: \"jadren_ui_on_click\", abi: \"C\")"));
    assert!(source_text.contains("ui_app_text_input"));
    assert!(source_text.contains("ui_app_bind_app_state_exact"));
    assert!(source_text.contains("ui_set_input_text"));
    assert!(source_text.contains("ui_app_input_read_exact"));
    assert!(source_text.contains("ui_app_commit_app_state_if_revision"));
    assert!(source_text.contains("verify_smoke_input_binding"));
    assert!(source_text.contains("ui_app_list_set_index"));
    assert!(source_text.contains("ui_app_table_set_selected_row"));
    assert!(source_text.contains("verify_smoke_collection_bindings"));
    assert!(source_text.contains("ui_app_list_sort_text"));
    assert!(source_text.contains("ui_app_list_sort_text_if_revision"));
    assert!(source_text.contains("ui_app_table_sort_text"));
    assert!(source_text.contains("ui_app_table_sort_text_if_revision"));
    assert!(source_text.contains("verify_smoke_collection_sorting"));
    assert!(source_text.contains("ui_app_list_filter_text_ex_if_revision"));
    assert!(source_text.contains("ui_app_table_filter_text_ex_if_revision"));
    assert!(source_text.contains("verify_smoke_collection_filters"));
    assert!(source_text.contains("ui_app_list_page_if_revision"));
    assert!(source_text.contains("ui_app_table_page_if_revision"));
    assert!(source_text.contains("verify_smoke_collection_paging"));
    assert!(source_text.contains("app_list_insert_text_if_revision"));
    assert!(source_text.contains("app_list_remove_if_revision"));
    assert!(source_text.contains("app_table_insert_row_if_revision"));
    assert!(source_text.contains("app_table_set_cell_if_revision"));
    assert!(source_text.contains("app_table_remove_row_if_revision"));
    assert!(source_text.contains("ui_app_list_refresh"));
    assert!(source_text.contains("ui_app_table_refresh"));
    assert!(source_text.contains("verify_smoke_collection_mutations"));
    assert!(source_text.contains("ui_app_list_filter_callback_if_revision"));
    assert!(source_text.contains("ui_app_table_filter_callback_if_revision"));
    assert!(source_text.contains("desktop_keep_first_collection_item"));
    assert!(source_text.contains("verify_smoke_collection_callback_filters"));
    assert!(source_text.contains("ui_dispatch_event(1)"));
    assert!(source_text.contains("app_state_read_text_exact"));
    assert!(source_text.contains("desktop_save_atomic_durable_buffer"));
    assert!(source_text.contains("desktop_load_file_buffer"));
    assert!(source_text.contains("desktop_export_table_csv_durable"));
    assert!(source_text.contains("desktop_parse_serve_api_port"));
    assert!(source_text.contains("desktop_add_api_snapshot_routes"));
    assert!(source_text.contains("desktop_serve_api"));
    assert!(source_text.contains("/api/status"));
    assert!(source_text.contains("/api/revision"));
    assert!(source_text.contains("/api/model"));
    assert!(source_text.contains("buffer_resize"));
    assert!(source_text.contains("verify_smoke_persistence"));
    assert!(source_text.contains("app_data_validate"));
    assert!(source_text.contains("--smoke"));
    assert!(source_text.contains("ui_app_run()"));

    let check = Command::new(binary())
        .args(["check", "."])
        .current_dir(&directory)
        .output()
        .expect("generated desktop package should check");
    assert!(
        check.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    fs::remove_dir_all(&directory).expect("template directory should be removable");
}

#[test]
fn init_full_app_template_creates_checkable_native_application_foundation() {
    let directory = std::env::temp_dir().join(format!(
        "jadren-cli-init-full-app-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    if directory.exists() {
        fs::remove_dir_all(&directory).expect("old template directory should be removable");
    }
    fs::create_dir_all(&directory).expect("template working directory should be creatable");

    let init = Command::new(binary())
        .args([
            "init",
            "--template",
            "full-app",
            "--name",
            "native_application",
        ])
        .current_dir(&directory)
        .output()
        .expect("jadren init should start");
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );
    assert!(directory.join("jadren.toml").is_file());
    assert!(directory.join("jadren.lock").is_file());
    assert!(directory.join("README.md").is_file());
    let main_source = directory.join("src/main.jdn");
    let model_source = directory.join("src/model.jdn");
    let data_source = directory.join("src/data.jdn");
    let controller_source = directory.join("src/controller.jdn");
    let view_source = directory.join("src/view.jdn");
    let storage_source = directory.join("src/storage.jdn");
    let exports_source = directory.join("src/exports.jdn");
    let api_source = directory.join("src/api.jdn");
    assert!(main_source.is_file());
    assert!(model_source.is_file());
    assert!(data_source.is_file());
    assert!(controller_source.is_file());
    assert!(view_source.is_file());
    assert!(storage_source.is_file());
    assert!(exports_source.is_file());
    assert!(api_source.is_file());
    let main_text =
        fs::read_to_string(&main_source).expect("application main source should be readable");
    let model_text =
        fs::read_to_string(&model_source).expect("application model source should be readable");
    let data_text =
        fs::read_to_string(&data_source).expect("application data source should be readable");
    let controller_text = fs::read_to_string(&controller_source)
        .expect("application controller source should be readable");
    let view_text =
        fs::read_to_string(&view_source).expect("application view source should be readable");
    let storage_text =
        fs::read_to_string(&storage_source).expect("application storage source should be readable");
    let exports_text =
        fs::read_to_string(&exports_source).expect("application exports source should be readable");
    let api_text =
        fs::read_to_string(&api_source).expect("application API source should be readable");
    let readme_text =
        fs::read_to_string(directory.join("README.md")).expect("template README should read");
    assert!(readme_text.contains("_if_model_revision"));
    for marker in [
        "module native_application.main",
        "native_application.model.bootstrap",
        "native_application.model.verify_interaction_smoke_model",
        "native_application.model.verify_due_reminder_smoke_model",
        "native_application.model.verify_scheduled_reminder_smoke_model",
        "native_application.model.verify_filtered_projects_smoke_model",
        "native_application.model.verify_filtered_tasks_smoke_model",
        "native_application.model.verify_tasks_ascending_smoke_model",
        "native_application.model.verify_tasks_descending_smoke_model",
        "native_application.model.verify_removal_smoke_model",
        "native_application.controller.cleanup_interactive_checkpoint",
        "native_application.controller.verify_interactive_checkpoint_slot",
        "native_application.controller.cleanup_interactive_exports",
        "native_application.controller.verify_interactive_exports",
        "native_application.controller.cleanup_interactive_json_backup",
        "native_application.controller.verify_interactive_json_backup",
        "native_application.storage.smoke_checkpoint_roundtrip",
        "native_application.storage.smoke_file_backed_model_roundtrip",
        "native_application.exports.smoke_durable_exports",
        "native_application.api.serve_health_once",
        "native_application.api.serve_snapshot",
        "native_application.api.serve_draft_update",
        "native_application.api.serve_draft_commit",
        "native_application.api.serve_model_sync",
        "native_application.view.build",
        "native_application.view.dispatch_smoke_actions",
        "native_application.view.dispatch_smoke_edit",
        "native_application.view.dispatch_smoke_due_reminders",
        "native_application.view.dispatch_smoke_future_reminder",
        "native_application.view.dispatch_smoke_filter",
        "native_application.view.dispatch_smoke_json_backup",
        "native_application.view.dispatch_smoke_sort_ascending",
        "native_application.view.dispatch_smoke_sort_descending",
        "native_application.view.dispatch_smoke_persistence",
        "native_application.view.dispatch_smoke_exports",
        "native_application.view.dispatch_smoke_removal",
        "native_application.view.reset_smoke_event_queue",
        "native_application.view.verify_interaction_smoke_view",
        "native_application.view.verify_due_reminder_smoke_view",
        "native_application.view.verify_scheduled_reminder_smoke_view",
        "native_application.view.verify_filtered_projects_smoke_view",
        "native_application.view.verify_filtered_tasks_smoke_view",
        "native_application.view.verify_tasks_ascending_smoke_view",
        "native_application.view.verify_tasks_descending_smoke_view",
        "native_application.view.verify_interactive_checkpoint_slot_view",
        "native_application.view.verify_removal_smoke_view",
        "native_application.view.verify_smoke_event_queue",
        "full_app_has_smoke_flag",
        "full_app_parse_serve_once_port",
        "full_app_parse_serve_api_port",
        "full_app_parse_serve_draft_port",
        "full_app_parse_serve_draft_commit_port",
        "full_app_parse_serve_model_sync_port",
        "ui_app_run()",
    ] {
        assert!(
            main_text.contains(marker),
            "missing full-app main marker: {marker}"
        );
    }
    for marker in [
        "module native_application.data",
        "app_data_snapshot_length",
        "pub fn revision",
        "snapshot_length_if_model_revision",
        "app_data_write_exact",
        "pub fn write_exact",
        "pub fn write_exact_if_model_revision",
        "file_write_atomic_durable",
        "file_read_exact",
        "app_data_load_exact",
        "pub fn load_exact_if_model_revision",
        "pub fn save_atomic_durable",
        "pub fn save_atomic_durable_if_model_revision",
        "pub fn load_file",
        "pub fn load_file_if_model_revision",
        "pub fn load_exact",
        "pub fn tx_begin_if_model_revision",
        "pub fn tx_commit_durable_directory_if_model_revision",
        "pub fn tx_rollback",
    ] {
        assert!(
            data_text.contains(marker),
            "missing full-app data marker: {marker}"
        );
    }
    for marker in [
        "module native_application.model",
        "app_list_push_text",
        "checkpoint_slot",
        "project_draft",
        "project_editor",
        "task_editor",
        "task_status_editor",
        "task_notes_editor",
        "task_estimate_editor",
        "task_reminder_due_editor",
        "selected_task_id",
        "next_task_id",
        "project_filter",
        "filtered_project_choice",
        "task_filter",
        "filtered_task_choice",
        "app_list_filter_text_ex_bytes",
        "app_table_filter_text_ex_bytes",
        "app_table_set_column_type",
        "app_table_append_row",
        "app_table_set_named_cell",
        "app_table_upsert_int",
        "app_table_find_int",
        "app_table_set_column_name(0, 2, \"Project\")",
        "app_table_set_column_name(0, 3, \"Notes\")",
        "app_table_set_column_name(0, 4, \"EstimateMinutes\")",
        "app_table_set_column_name(0, 5, \"ReminderDueUnixSeconds\")",
        "add_draft_task",
        "add_project",
        "complete_selected_task",
        "load_selected_project_editor",
        "sync_selected_project_from_list",
        "rename_selected_project",
        "remove_selected_project",
        "load_selected_task_editor",
        "load_selected_task_status_editor",
        "load_selected_task_notes_editor",
        "load_selected_task_estimate_editor",
        "load_selected_task_reminder_due_editor",
        "load_selected_task_fields",
        "sync_selected_task_from_row",
        "task_ids_are_unique",
        "task_projects_are_valid",
        "task_reminders_are_valid",
        "rebuild_task_reminders",
        "normalize_next_task_id",
        "rename_selected_task",
        "update_selected_task_status",
        "update_selected_task_notes",
        "update_selected_task_estimate",
        "update_selected_task_reminder",
        "poll_due_task_reminders",
        "parse_int",
        "move_selected_task_to_selected_project",
        "remove_selected_task",
        "refresh_project_filter",
        "refresh_task_filter",
        "sort_tasks_by_title",
        "verify_selected_project_editor",
        "verify_interaction_smoke_model",
        "verify_due_reminder_smoke_model",
        "verify_scheduled_reminder_smoke_model",
        "verify_filtered_projects_smoke_model",
        "verify_filtered_tasks_smoke_model",
        "verify_tasks_ascending_smoke_model",
        "verify_tasks_descending_smoke_model",
        "verify_removal_smoke_model",
        "app_data_tx_begin_if_revision",
        "app_data_tx_commit",
        "app_data_tx_rollback",
        "verify_smoke_model",
    ] {
        assert!(
            model_text.contains(marker),
            "missing full-app model marker: {marker}"
        );
    }
    for marker in [
        "module native_application.api",
        "serve_health_once",
        "serve_snapshot",
        "serve_draft_update",
        "serve_draft_commit",
        "serve_model_sync",
        "full_app_load_model_if_revision",
        "full_app_commit_draft_if_revision",
        "net_tcp_listen",
        "net_tcp_accept",
        "net_tcp_receive",
        "net_tcp_send_all_prefix",
        "http_request_body_exact_prefix",
        "http_router_add",
        "http_router_add_exact",
        "app_state_write_json_exact",
        "app_list_export_json_exact",
        "app_table_export_json_exact",
        "app_state_set_text_bytes",
        "state_body: [UInt8; 512]",
        "app_data_tx_begin_if_revision",
        "app_data_tx_commit_durable_if_revision",
        "app_data_tx_rollback",
        "import native_application.data.revision",
        "import native_application.data.snapshot_length_if_model_revision",
        "import native_application.data.write_exact_if_model_revision",
        "import native_application.data.load_exact_if_model_revision",
        "revision()",
        "snapshot_length_if_model_revision(expected_revision)",
        "write_exact_if_model_revision(output, output_length, expected_revision)",
        "write_exact(rollback, rollback_length)",
        "load_exact_if_model_revision(input, input_length, expected_revision)",
        "load_exact(rollback, rollback_length[0])",
        "task_reminders_are_valid",
        "rebuild_task_reminders",
        "http_request_header_exact",
        "http_session_open",
        "http_session_step",
    ] {
        assert!(
            api_text.contains(marker),
            "missing full-app API marker: {marker}"
        );
    }
    for marker in [
        "module native_application.view",
        "ui_app_text_input",
        "ui_app_list_bind_app",
        "ui_app_table_bind_app",
        "ui_app_bind_app_state_exact",
        "project_draft",
        "project_editor",
        "task_editor",
        "task_status_editor",
        "task_notes_editor",
        "task_estimate_editor",
        "task_reminder_due_editor",
        "ui_app_table_column(tasks, 3, \"Notes\", 185)",
        "ui_app_table_column(tasks, 4, \"Estimate minutes\", 105)",
        "ui_app_table_column(tasks, 5, \"Reminder due (Unix)\", 210)",
        "ui_app_table_bind_app(tasks, 0, 6)",
        "ui_table_read_cell(30, 2, 3, notes)",
        "ui_table_read_cell(30, 2, 4, estimate)",
        "checkpoint_slots",
        "filtered_projects",
        "filtered_tasks",
        "ui_app_button",
        "Remove unused project",
        "Assign task to selected project",
        "Update notes",
        "Update estimate",
        "Schedule reminder",
        "Check due reminders",
        "controller.handle",
        "dispatch_smoke_actions",
        "dispatch_smoke_edit",
        "dispatch_smoke_due_reminders",
        "dispatch_smoke_future_reminder",
        "dispatch_smoke_filter",
        "dispatch_smoke_json_backup",
        "dispatch_smoke_sort_ascending",
        "dispatch_smoke_sort_descending",
        "dispatch_smoke_persistence",
        "dispatch_smoke_exports",
        "dispatch_smoke_removal",
        "reset_smoke_event_queue",
        "verify_interaction_smoke_view",
        "verify_due_reminder_smoke_view",
        "verify_scheduled_reminder_smoke_view",
        "verify_filtered_projects_smoke_view",
        "verify_filtered_tasks_smoke_view",
        "verify_tasks_ascending_smoke_view",
        "verify_tasks_descending_smoke_view",
        "verify_interactive_checkpoint_slot_view",
        "verify_removal_smoke_view",
        "verify_smoke_event_queue",
        "ui_event_queue_clear",
        "ui_event_queue_poll_exact",
        "ui_list_count",
        "ui_table_selected_row",
        "verify_smoke_view",
    ] {
        assert!(
            view_text.contains(marker),
            "missing full-app view marker: {marker}"
        );
    }
    for marker in [
        "module native_application.controller",
        "native_application.model.add_draft_task",
        "native_application.model.add_project",
        "native_application.model.complete_selected_task",
        "native_application.model.rename_selected_project",
        "native_application.model.remove_selected_project",
        "native_application.model.load_selected_task_fields",
        "native_application.model.sync_selected_project_from_list",
        "native_application.model.sync_selected_task_from_row",
        "native_application.model.task_ids_are_unique",
        "native_application.model.task_projects_are_valid",
        "native_application.model.task_reminders_are_valid",
        "native_application.model.rebuild_task_reminders",
        "native_application.model.normalize_next_task_id",
        "native_application.model.rename_selected_task",
        "native_application.model.update_selected_task_status",
        "native_application.model.update_selected_task_notes",
        "native_application.model.update_selected_task_reminder",
        "native_application.model.poll_due_task_reminders",
        "native_application.model.move_selected_task_to_selected_project",
        "native_application.model.remove_selected_task",
        "native_application.model.refresh_project_filter",
        "native_application.model.refresh_task_filter",
        "native_application.model.sort_tasks_by_title",
        "native_application.storage.save_checkpoint_durable",
        "native_application.storage.load_checkpoint",
        "native_application.exports.export_list_csv_durable",
        "native_application.exports.export_table_csv_durable",
        "pub fn handle",
        "save_interactive_checkpoint",
        "restore_interactive_checkpoint",
        "interactive_checkpoint_slot",
        "verify_interactive_checkpoint_slot",
        "cleanup_interactive_checkpoint",
        "export_interactive_list",
        "export_interactive_table",
        "verify_interactive_exports",
        "cleanup_interactive_exports",
        "export_interactive_table_json",
        "import_interactive_table_json",
        "verify_interactive_json_backup",
        "cleanup_interactive_json_backup",
        "app_table_export_json_file_durable",
        "app_table_import_json_file",
    ] {
        assert!(
            controller_text.contains(marker),
            "missing full-app controller marker: {marker}"
        );
    }
    for marker in [
        "module native_application.storage",
        "save_checkpoint_durable",
        "restore_project_choice",
        "restore_task_choice",
        "restore_selected_task_id",
        "restore_next_task_id",
        "restore_filtered_project_choice",
        "restore_filtered_task_choice",
        "restore_checkpoint_slot",
        "normalize_selection_keys",
        "normalize_next_task_id",
        "task_projects_are_valid",
        "task_reminders_are_valid",
        "rebuild_task_reminders",
        "app_state_read_uint",
        "tx_commit_durable_directory_if_model_revision",
        "save_model_file_if_model_revision",
        "load_model_file_if_model_revision",
        "snapshot_length_if_model_revision",
        "smoke_checkpoint_roundtrip",
        "save_model_file",
        "load_model_file",
        "smoke_file_backed_model_roundtrip",
    ] {
        assert!(
            storage_text.contains(marker),
            "missing full-app storage marker: {marker}"
        );
    }
    for marker in [
        "module native_application.exports",
        "export_list_csv_durable",
        "app_list_export_csv_file_durable",
        "app_table_export_csv_file_durable",
        "smoke_durable_exports",
    ] {
        assert!(
            exports_text.contains(marker),
            "missing full-app exports marker: {marker}"
        );
    }

    let check = Command::new(binary())
        .args(["check", "."])
        .current_dir(&directory)
        .output()
        .expect("generated full-app package should check");
    assert!(
        check.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    fs::remove_dir_all(&directory).expect("template directory should be removable");
}

#[test]
fn init_server_template_creates_checkable_native_service() {
    let directory = std::env::temp_dir().join(format!(
        "jadren-cli-init-server-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    if directory.exists() {
        fs::remove_dir_all(&directory).expect("old template directory should be removable");
    }
    fs::create_dir_all(&directory).expect("template working directory should be creatable");

    let init = Command::new(binary())
        .args(["init", "--template", "server", "--name", "native_server"])
        .current_dir(&directory)
        .output()
        .expect("jadren init should start");
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );
    assert!(directory.join("jadren.toml").is_file());
    assert!(directory.join("jadren.lock").is_file());
    assert!(directory.join("README.md").is_file());
    let source = directory.join("src/main.jdn");
    assert!(source.is_file());
    let source_text = fs::read_to_string(&source).expect("server source should be readable");
    assert!(source_text.contains("module native_server.main"));
    assert!(source_text.contains("http_session_open"));
    assert!(source_text.contains("http_router_add"));
    assert!(source_text.contains("process_arg_count"));
    assert!(source_text.contains("server_parse_port"));
    assert!(source_text.contains("server_parse_max_requests"));
    assert!(source_text.contains("while running"));

    let check = Command::new(binary())
        .args(["check", "."])
        .current_dir(&directory)
        .output()
        .expect("generated server package should check");
    assert!(
        check.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    fs::remove_dir_all(&directory).expect("template directory should be removable");
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn builds_and_runs_host_executables_from_main() {
    let directory = std::env::temp_dir().join(format!(
        "jadren-cli-executable-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::create_dir_all(&directory).expect("temporary directory should be writable");

    let exit_source = directory.join("exit_code.jdn");
    let exit_executable = directory.join(if cfg!(windows) {
        "exit_code.exe"
    } else {
        "exit_code"
    });
    fs::write(&exit_source, "fn main() -> Int32 { return 42 }")
        .expect("temporary source should be writable");
    let build = Command::new(binary())
        .arg("build")
        .arg(&exit_source)
        .args(["--profile", "release", "--cpu", "baseline", "-o"])
        .arg(&exit_executable)
        .output()
        .expect("jadren build should start");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    assert!(String::from_utf8_lossy(&build.stdout).contains("built"));
    let executable_bytes = fs::read(&exit_executable).expect("built executable should be readable");
    if cfg!(windows) {
        assert_eq!(executable_bytes.get(..2), Some(b"MZ".as_slice()));
    } else {
        assert_eq!(executable_bytes.get(..4), Some(b"\x7fELF".as_slice()));
    }
    assert_eq!(
        Command::new(&exit_executable)
            .status()
            .expect("built executable should run")
            .code(),
        Some(42)
    );

    let float_source = directory.join("float_exit_code.jdn");
    let float_executable = directory.join(if cfg!(windows) {
        "float_exit_code.exe"
    } else {
        "float_exit_code"
    });
    fs::write(&float_source, "fn main() -> Float64 { return 42.5f64 }")
        .expect("Float64 source should be writable");
    let float_build = Command::new(binary())
        .arg("build")
        .arg(&float_source)
        .args(["--profile", "release", "--cpu", "baseline", "-o"])
        .arg(&float_executable)
        .output()
        .expect("Jadren Float64 build should start");
    assert!(
        float_build.status.success(),
        "{}",
        String::from_utf8_lossy(&float_build.stderr)
    );
    assert_eq!(
        Command::new(&float_executable)
            .status()
            .expect("built Float64 executable should run")
            .code(),
        Some(42)
    );

    let unit_source = directory.join("unit_main.jdn");
    let unit_executable = directory.join(if cfg!(windows) {
        "unit_main.exe"
    } else {
        "unit_main"
    });
    fs::write(&unit_source, "fn main() {}").expect("temporary source should be writable");
    let run = Command::new(binary())
        .arg("run")
        .arg(&unit_source)
        .args(["-o"])
        .arg(&unit_executable)
        .output()
        .expect("jadren run should start");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(String::from_utf8_lossy(&run.stdout).contains("running"));

    let hello_source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/hello.jdn");
    let hello_executable = directory.join(if cfg!(windows) { "hello.exe" } else { "hello" });
    let hello = Command::new(binary())
        .arg("run")
        .arg(hello_source)
        .args(["-o"])
        .arg(&hello_executable)
        .output()
        .expect("jadren run should execute hello.jdn");
    assert!(
        hello.status.success(),
        "{}",
        String::from_utf8_lossy(&hello.stderr)
    );
    assert!(
        String::from_utf8_lossy(&hello.stdout).contains("Hello, Jadren"),
        "{}",
        String::from_utf8_lossy(&hello.stdout)
    );

    let _ = fs::remove_dir_all(directory);
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
fn checks_package_and_resolves_cross_file_imports() {
    let directory = std::env::temp_dir().join(format!(
        "jadren-cli-package-check-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let source = directory.join("src");
    fs::create_dir_all(&source).expect("package source directory should be writable");
    fs::write(
        directory.join("jadren.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2026\"\n\n[dependencies]\n\n[targets]\nlibrary = true\n",
    )
    .expect("manifest should be writable");
    let lock = Command::new(binary())
        .args(["lock"])
        .arg(&directory)
        .output()
        .expect("jadren lock should start");
    assert!(
        lock.status.success(),
        "{}",
        String::from_utf8_lossy(&lock.stderr)
    );
    fs::write(
        source.join("math.jdn"),
        "module demo.math\npub fn answer() -> Int32 { return 42 }\n",
    )
    .expect("module source should be writable");
    fs::write(
        source.join("main.jdn"),
        "module demo.main\nimport demo.math.answer\nfn main() { let value: Int32 = answer() if value == 42 { print(\"package ready\") } }\n",
    )
    .expect("application source should be writable");

    let check = Command::new(binary())
        .args(["check"])
        .arg(&directory)
        .output()
        .expect("package-aware check should start");
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(String::from_utf8_lossy(&check.stdout).contains("module imports resolved"));
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn builds_and_runs_cross_file_package_imports() {
    let directory = std::env::temp_dir().join(format!(
        "jadren-cli-package-build-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("package")
    ));
    let source = directory.join("src");
    fs::create_dir_all(&source).expect("package source directory should be writable");
    fs::write(
        directory.join("jadren.toml"),
        "[package]\nname = \"demo-build\"\nversion = \"0.1.0\"\nedition = \"2026\"\n\n[dependencies]\n\n[targets]\nlibrary = true\n",
    )
    .expect("manifest should be writable");
    let lock = Command::new(binary())
        .args(["lock"])
        .arg(&directory)
        .output()
        .expect("jadren lock should start");
    assert!(
        lock.status.success(),
        "{}",
        String::from_utf8_lossy(&lock.stderr)
    );
    fs::write(
        source.join("math.jdn"),
        "module demo_build.math\npub fn answer() -> Int32 { return 42 }\n",
    )
    .expect("module source should be writable");
    fs::write(
        source.join("main.jdn"),
        "module demo_build.main\nimport demo_build.math.answer\nfn main() -> Int32 { return answer() }\n",
    )
    .expect("entry source should be writable");

    let output = directory.join(if cfg!(windows) {
        "demo-build.exe"
    } else {
        "demo-build"
    });
    let build = Command::new(binary())
        .args(["build"])
        .arg(&directory)
        .args(["-o"])
        .arg(&output)
        .args(["--profile", "release"])
        .output()
        .expect("package build should start");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&output)
        .status()
        .expect("package executable should run");
    assert_eq!(run.code(), Some(42));
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn builds_and_runs_package_modules_with_colliding_function_names() {
    let directory = std::env::temp_dir().join(format!(
        "jadren-cli-qualified-symbols-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("package")
    ));
    let source = directory.join("src");
    fs::create_dir_all(&source).expect("package source directory should be writable");
    fs::write(
        directory.join("jadren.toml"),
        "[package]\nname = \"demo-qualified-symbols\"\nversion = \"0.1.0\"\nedition = \"2026\"\n\n[dependencies]\n\n[targets]\nlibrary = true\n",
    )
    .expect("manifest should be writable");
    let lock = Command::new(binary())
        .args(["lock"])
        .arg(&directory)
        .output()
        .expect("jadren lock should start");
    assert!(
        lock.status.success(),
        "{}",
        String::from_utf8_lossy(&lock.stderr)
    );
    fs::write(
        source.join("alpha.jdn"),
        "module demo_collision.alpha\npub fn compute() -> Int32 { return 10 }\n",
    )
    .expect("alpha source should be writable");
    fs::write(
        source.join("beta.jdn"),
        "module demo_collision.beta\npub fn compute() -> Int32 { return 32 }\n",
    )
    .expect("beta source should be writable");
    fs::write(
        source.join("main.jdn"),
        "module demo_collision.main\nimport demo_collision.alpha\nimport demo_collision.beta\nfn main() -> Int32 { return alpha.compute() + beta.compute() }\n",
    )
    .expect("entry source should be writable");

    let output = directory.join(if cfg!(windows) {
        "demo-qualified-symbols.exe"
    } else {
        "demo-qualified-symbols"
    });
    let build = Command::new(binary())
        .args(["build"])
        .arg(&directory)
        .args(["-o"])
        .arg(&output)
        .args(["--profile", "release"])
        .output()
        .expect("qualified-symbol package build should start");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&output)
        .status()
        .expect("qualified-symbol package executable should run");
    assert_eq!(run.code(), Some(42));
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn builds_cross_file_nominal_signature_without_type_import() {
    let directory = std::env::temp_dir().join(format!(
        "jadren-cli-nominal-build-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("nominal")
    ));
    let source = directory.join("src");
    fs::create_dir_all(&source).expect("package source directory should be writable");
    fs::write(
        directory.join("jadren.toml"),
        "[package]\nname = \"demo-nominal\"\nversion = \"0.1.0\"\nedition = \"2026\"\n\n[dependencies]\n\n[targets]\nlibrary = true\n",
    )
    .expect("manifest should be writable");
    let lock = Command::new(binary())
        .args(["lock"])
        .arg(&directory)
        .output()
        .expect("jadren lock should start");
    assert!(
        lock.status.success(),
        "{}",
        String::from_utf8_lossy(&lock.stderr)
    );
    fs::write(
        source.join("status.jdn"),
        "module demo_nominal.status\npub enum DemoState { Ready, Failed(Int32) }\npub fn make_status(value: Int32) -> DemoState { if value == 42 { return Ready } return Failed(value) }\npub fn status_code(value: DemoState) -> Int32 { return match value { Ready => 42, Failed(_) => 1 } }\n",
    )
    .expect("nominal source should be writable");
    fs::write(
        source.join("main.jdn"),
        "module demo_nominal.main\nimport demo_nominal.status.make_status\nimport demo_nominal.status.status_code\nfn main() -> Int32 { return status_code(make_status(42)) }\n",
    )
    .expect("entry source should be writable");

    let output = directory.join(if cfg!(windows) {
        "demo-nominal.exe"
    } else {
        "demo-nominal"
    });
    let build = Command::new(binary())
        .args(["build"])
        .arg(&directory)
        .args(["-o"])
        .arg(&output)
        .args(["--profile", "release"])
        .output()
        .expect("nominal package build should start");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&output)
        .status()
        .expect("nominal package executable should run");
    assert_eq!(run.code(), Some(42));
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn builds_cross_file_nested_generic_nominal_buffer() {
    let directory = std::env::temp_dir().join(format!(
        "jadren-cli-nested-generic-nominal-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("package")
    ));
    let source = directory.join("src");
    fs::create_dir_all(&source).expect("package source directory should be writable");
    fs::write(
        directory.join("jadren.toml"),
        "[package]\nname = \"demo-nested-generic-nominal\"\nversion = \"0.1.0\"\nedition = \"2026\"\n\n[dependencies]\n\n[targets]\nlibrary = true\n",
    )
    .expect("manifest should be writable");
    let lock = Command::new(binary())
        .args(["lock"])
        .arg(&directory)
        .output()
        .expect("jadren lock should start");
    assert!(
        lock.status.success(),
        "{}",
        String::from_utf8_lossy(&lock.stderr)
    );
    fs::write(
        source.join("model.jdn"),
        "module demo_nested.model\n@repr(C)\npub struct Box<T> { pub value: T }\n@repr(C)\npub struct Frame<T> { pub first: Box<T>, pub samples: [T; 2] }\n",
    )
    .expect("model source should be writable");
    fs::write(
        source.join("main.jdn"),
        "module demo_nested.main\nimport demo_nested.model.Box\nimport demo_nested.model.Frame\nfn main() -> Int32 { let created: Result<Buffer<Frame<Int64>>, Int32> = buffer_create(0usize) var result_code: Int32 = 0 match created { Ok(values) => { let item: Frame<Int64> = Frame { first: Box { value: 42 as Int64 }, samples: [7 as Int64, 9 as Int64] } if !buffer_append(values, item) { result_code = 1 } if values[0].first.value != 42 as Int64 { result_code = 2 } if values[0].samples[1] != 9 as Int64 { result_code = 3 } } Error(status) => { result_code = status } } return result_code }\n",
    )
    .expect("entry source should be writable");

    let output = directory.join(if cfg!(windows) {
        "demo-nested-generic-nominal.exe"
    } else {
        "demo-nested-generic-nominal"
    });
    let build = Command::new(binary())
        .args(["build"])
        .arg(&directory)
        .args(["-o"])
        .arg(&output)
        .args(["--profile", "release"])
        .output()
        .expect("nested generic nominal package build should start");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&output)
        .status()
        .expect("nested generic nominal package executable should run");
    assert_eq!(run.code(), Some(0));
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn builds_cross_file_generic_buffer_query_repeated_calls() {
    let directory = std::env::temp_dir().join(format!(
        "jadren-cli-generic-buffer-query-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("package")
    ));
    let source = directory.join("src");
    fs::create_dir_all(&source).expect("package source directory should be writable");
    fs::write(
        directory.join("jadren.toml"),
        "[package]\nname = \"demo-generic-buffer-query\"\nversion = \"0.1.0\"\nedition = \"2026\"\n\n[dependencies]\n\n[targets]\nlibrary = true\n",
    )
    .expect("manifest should be writable");
    let lock = Command::new(binary())
        .args(["lock"])
        .arg(&directory)
        .output()
        .expect("jadren lock should start");
    assert!(
        lock.status.success(),
        "{}",
        String::from_utf8_lossy(&lock.stderr)
    );
    fs::write(
        source.join("library.jdn"),
        "module demo_generic_buffer_query.library\npub fn has_index<T>(values: read Buffer<T>, index: UIntSize) -> Bool { for candidate in values.indices { if candidate == index { return true } } return false }\n",
    )
    .expect("library source should be writable");
    fs::write(
        source.join("main.jdn"),
        "module demo_generic_buffer_query.main\nimport demo_generic_buffer_query.library.has_index\nfn main() -> Int32 { let created: Result<Buffer<Int32>, Int32> = buffer_create(0usize) var result_code: Int32 = 0 match created { Ok(values) => { if !buffer_append(values, 7) { result_code = 1 } if !has_index(values, 0usize) { result_code = 2 } if has_index(values, 1usize) { result_code = 3 } let clear_status: Int32 = buffer_clear_status(values) if clear_status != 0 { result_code = clear_status } } Error(status) => { result_code = status } } return result_code }\n",
    )
    .expect("entry source should be writable");

    let output = directory.join(if cfg!(windows) {
        "demo-generic-buffer-query.exe"
    } else {
        "demo-generic-buffer-query"
    });
    let build = Command::new(binary())
        .args(["build"])
        .arg(&directory)
        .args(["-o"])
        .arg(&output)
        .args(["--profile", "release"])
        .output()
        .expect("generic buffer query package build should start");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&output)
        .status()
        .expect("generic buffer query package executable should run");
    assert_eq!(run.code(), Some(0));
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn selects_cross_file_generic_return_specialization() {
    let directory = std::env::temp_dir().join(format!(
        "jadren-cli-generic-return-specialization-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("package")
    ));
    let source = directory.join("src");
    fs::create_dir_all(&source).expect("package source directory should be writable");
    fs::write(
        directory.join("jadren.toml"),
        "[package]\nname = \"demo-generic-return-specialization\"\nversion = \"0.1.0\"\nedition = \"2026\"\n\n[dependencies]\n\n[targets]\nlibrary = true\n",
    )
    .expect("manifest should be writable");
    let lock = Command::new(binary())
        .args(["lock"])
        .arg(&directory)
        .output()
        .expect("jadren lock should start");
    assert!(
        lock.status.success(),
        "{}",
        String::from_utf8_lossy(&lock.stderr)
    );
    fs::write(
        source.join("library.jdn"),
        "module demo_generic_return.library\npub fn make<T>(size: UIntSize) -> Result<Buffer<T>, Int32> { return buffer_create(size) }\n",
    )
    .expect("library source should be writable");
    fs::write(
        source.join("main.jdn"),
        "module demo_generic_return.main\nimport demo_generic_return.library.make\nfn main() -> Int32 { let outer_result: Result<Buffer<Buffer<Int32>>, Int32> = make(1usize) let inner_result: Result<Buffer<Int32>, Int32> = make(0usize) var result_code: Int32 = 0 match outer_result { Ok(outer) => { match inner_result { Ok(inner) => { if !buffer_append_move(outer, inner) { result_code = 1 } if buffer_length(outer) != 1usize { result_code = 2 } if !buffer_clear_move(outer) { result_code = 3 } } Error(status) => { result_code = status } } } Error(status) => { result_code = status } } return result_code }\n",
    )
    .expect("entry source should be writable");

    let output = directory.join(if cfg!(windows) {
        "demo-generic-return-specialization.exe"
    } else {
        "demo-generic-return-specialization"
    });
    let build = Command::new(binary())
        .args(["build"])
        .arg(&directory)
        .args(["-o"])
        .arg(&output)
        .args(["--profile", "release"])
        .output()
        .expect("generic return specialization build should start");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&output)
        .status()
        .expect("generic return specialization executable should run");
    assert_eq!(run.code(), Some(0));
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn builds_cross_file_generic_nominal_with_owning_buffer_field() {
    let directory = std::env::temp_dir().join(format!(
        "jadren-cli-generic-owning-nominal-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("package")
    ));
    let source = directory.join("src");
    fs::create_dir_all(&source).expect("package source directory should be writable");
    fs::write(
        directory.join("jadren.toml"),
        "[package]\nname = \"demo-generic-owning-nominal\"\nversion = \"0.1.0\"\nedition = \"2026\"\n\n[dependencies]\n\n[targets]\nlibrary = true\n",
    )
    .expect("manifest should be writable");
    let lock = Command::new(binary())
        .args(["lock"])
        .arg(&directory)
        .output()
        .expect("jadren lock should start");
    assert!(
        lock.status.success(),
        "{}",
        String::from_utf8_lossy(&lock.stderr)
    );
    fs::write(
        source.join("model.jdn"),
        "module demo_owning.model\n@repr(C)\npub struct Holder<T> { pub values: Buffer<T>, pub id: Int32 }\n",
    )
    .expect("model source should be writable");
    fs::write(
        source.join("main.jdn"),
        "module demo_owning.main\nimport demo_owning.model.Holder\nfn main() -> Int32 { let inner: Result<Buffer<Int32>, Int32> = buffer_create(0usize) let outer: Result<Buffer<Holder<Int32>>, Int32> = buffer_create(0usize) var result_code: Int32 = 0 match inner { Ok(values) => { if !buffer_append(values, 7) { result_code = 1 } match outer { Ok(items) => { let item: Holder<Int32> = Holder { values: values, id: 42 } if !buffer_append(items, item) { result_code = 2 } if items[0].values[0] != 7 { result_code = 3 } if items[0].id != 42 { result_code = 4 } } Error(status) => { result_code = status } } } Error(status) => { result_code = status } } return result_code }\n",
    )
    .expect("entry source should be writable");

    let output = directory.join(if cfg!(windows) {
        "demo-generic-owning-nominal.exe"
    } else {
        "demo-generic-owning-nominal"
    });
    let build = Command::new(binary())
        .args(["build"])
        .arg(&directory)
        .args(["-o"])
        .arg(&output)
        .args(["--profile", "release"])
        .output()
        .expect("generic owning nominal package build should start");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&output)
        .status()
        .expect("generic owning nominal package executable should run");
    assert_eq!(run.code(), Some(0));
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn builds_cross_file_nested_generic_nominal_with_owning_leaf() {
    let directory = std::env::temp_dir().join(format!(
        "jadren-cli-nested-generic-owning-leaf-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("package")
    ));
    let source = directory.join("src");
    fs::create_dir_all(&source).expect("package source directory should be writable");
    fs::write(
        directory.join("jadren.toml"),
        "[package]\nname = \"demo-nested-generic-owning-leaf\"\nversion = \"0.1.0\"\nedition = \"2026\"\n\n[dependencies]\n\n[targets]\nlibrary = true\n",
    )
    .expect("manifest should be writable");
    let lock = Command::new(binary())
        .args(["lock"])
        .arg(&directory)
        .output()
        .expect("jadren lock should start");
    assert!(
        lock.status.success(),
        "{}",
        String::from_utf8_lossy(&lock.stderr)
    );
    fs::write(
        source.join("model.jdn"),
        "module demo_nested_owning.model\n@repr(C)\npub struct Box<T> { pub value: T }\n@repr(C)\npub struct Frame<T> { pub first: Box<T> }\n",
    )
    .expect("model source should be writable");
    fs::write(
        source.join("main.jdn"),
        "module demo_nested_owning.main\nimport demo_nested_owning.model.Box\nimport demo_nested_owning.model.Frame\nfn main() -> Int32 { let inner: Result<Buffer<Int32>, Int32> = buffer_create(0usize) let outer: Result<Buffer<Frame<Buffer<Int32>>>, Int32> = buffer_create(0usize) var result_code: Int32 = 0 match inner { Ok(values) => { if !buffer_append(values, 7) { result_code = 1 } let item: Frame<Buffer<Int32>> = Frame { first: Box { value: values } } match outer { Ok(items) => { if !buffer_append(items, item) { result_code = 2 } if items[0].first.value[0] != 7 { result_code = 3 } } Error(status) => { result_code = status } } } Error(status) => { result_code = status } } return result_code }\n",
    )
    .expect("entry source should be writable");

    let output = directory.join(if cfg!(windows) {
        "demo-nested-generic-owning-leaf.exe"
    } else {
        "demo-nested-generic-owning-leaf"
    });
    let build = Command::new(binary())
        .args(["build"])
        .arg(&directory)
        .args(["-o"])
        .arg(&output)
        .args(["--profile", "release"])
        .output()
        .expect("nested generic owning leaf package build should start");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&output)
        .status()
        .expect("nested generic owning leaf package executable should run");
    assert_eq!(run.code(), Some(0));
    let _ = fs::remove_dir_all(directory);
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
            "runtime ABI 0.22 system+region allocators, abort panic boundary, callbacks, Buffer/Slice, UTF-8 String, math scalar, vector value, quaternion Slerp, enum carrier branch tables, field tables, direct/nested drop-only record remove and caller-owned insert/remove/pop move available"
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
fn reports_app_state_float_delta_type_mismatch() {
    let path = std::env::temp_dir().join(format!(
        "jadren-app-state-float-type-{}-{}.jdn",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::write(
        &path,
        "module test; fn main() { let value: Bool = app_state_add_float(\"rate\", 1i64) }",
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
    assert!(stdout.contains("\"code\":\"J0301\""), "{stdout}");
}

#[test]
fn reports_app_state_revision_set_type_mismatch() {
    let path = std::env::temp_dir().join(format!(
        "jadren-app-state-set-int-revision-type-{}-{}.jdn",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::write(
        &path,
        "module test; fn main() { let value: Bool = app_state_set_int_if_revision(\"minutes\", true, 0u64) }",
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
    assert!(stdout.contains("\"code\":\"J0301\""), "{stdout}");
}

#[test]
fn reports_app_state_typed_revision_set_mismatch() {
    let path = std::env::temp_dir().join(format!(
        "jadren-app-state-typed-revision-set-type-{}-{}.jdn",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::write(
        &path,
        "module test; fn main() { let value: Bool = app_state_set_text_if_revision(\"name\", 1i64, 0u64) }",
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
    assert!(stdout.contains("\"code\":\"J0301\""), "{stdout}");
}

#[test]
fn reports_app_state_text_bytes_revision_set_mismatch() {
    let path = std::env::temp_dir().join(format!(
        "jadren-app-state-set-text-bytes-revision-type-{}-{}.jdn",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::write(
        &path,
        "module test; fn main() { let value: Bool = app_state_set_text_bytes_if_revision(\"name\", 1i64, 1usize, 0u64) }",
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
    assert!(stdout.contains("\"code\":\"J0301\""), "{stdout}");
}

#[test]
fn reports_app_state_text_bytes_model_revision_set_mismatch() {
    let path = std::env::temp_dir().join(format!(
        "jadren-app-state-set-text-bytes-model-revision-type-{}-{}.jdn",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::write(
        &path,
        "module test; fn main() { let value: Bool = app_state_set_text_bytes_if_model_revision(\"name\", 1i64, 1usize, 0u64) }",
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
    assert!(stdout.contains("\"code\":\"J0301\""), "{stdout}");
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
