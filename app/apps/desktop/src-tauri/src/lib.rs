// SPDX-License-Identifier: GPL-3.0-or-later

mod background;
mod commands;
mod error;
mod guard;
pub mod settings;
pub mod state;
mod webview_recycle;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[allow(clippy::too_many_lines)]
pub fn run() {
    cu_core::harden_process();
    if let Ok(dir) = cu_core::data_dir() {
        cu_core::secure_app_data_dir(&dir);
        cu_core::enable_vault_registry(&dir);
    }
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if argv.iter().any(|arg| arg == guard::PANIC_ARG) {
                guard::panic_and_notify(app);
            } else if !argv.iter().any(|arg| arg == background::BACKGROUND_ARG) {
                background::open_main_window(app);
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![background::BACKGROUND_ARG]),
        ))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(state::AppState::new())
        .on_window_event(background::on_window_event)
        .setup(|app| {
            use tauri::Manager;
            let handle = app.handle();
            let state = handle.state::<state::AppState>();
            if std::env::args().any(|arg| arg == guard::PANIC_ARG) {
                tauri::async_runtime::block_on(guard::panic(&state));
                handle.exit(0);
                return Ok(());
            }
            let security = state.settings.blocking_lock().security.clone();
            guard::apply_capture_protection(handle, security.hide_from_capture);
            let _ = guard::set_panic_shortcut(handle, security.panic_shortcut.as_deref());
            guard::start_session_watcher(handle);
            guard::start_remote_watcher(handle);
            background::startup(
                handle,
                std::env::args().any(|arg| arg == background::BACKGROUND_ARG),
            );
            background::start_scheduler(handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::settings::set_outside_texts,
            commands::vault::vault_create,
            commands::vault::vault_unlock,
            commands::vault::vault_unlock_recovery,
            commands::vault::vault_create_remote,
            commands::vault::vault_unlock_remote,
            commands::vault::vault_generate_recovery_phrase,
            commands::vault::vault_change_password,
            commands::vault::vault_lock,
            commands::vault::vault_peek_unlock_methods,
            commands::vault::vault_unlock_methods,
            commands::vault::unlock_attempts_take,
            commands::vault::vault_unlock_platform,
            commands::vault::vault_unlock_fido2,
            commands::vault::vault_enable_platform_unlock,
            commands::vault::vault_disable_platform_unlock,
            commands::vault::vault_enable_fido2,
            commands::vault::vault_disable_fido2,
            commands::vault::recovery_split,
            commands::vault::vault_enable_two_factor,
            commands::vault::vault_disable_two_factor,
            commands::vault::vault_change_password_two_factor,
            commands::vault::member_identity_exists,
            commands::vault::member_identity_generate,
            commands::vault::member_identity_public_key,
            commands::vault::vault_unlock_member,
            commands::vault::vault_accept_config_rollback,
            commands::vault::webview_recycle,
            commands::vault::lock_notice_take,
            commands::vault::vault_list_members,
            commands::vault::vault_add_member,
            commands::vault::vault_remove_member,
            commands::vault::vault_inbox_status,
            commands::vault::vault_enable_inbox,
            commands::vault::vault_disable_inbox,
            commands::vault::vault_list_inbox,
            commands::vault::vault_collect_inbox,
            commands::inbox::inbox_deposit,
            commands::browse::vault_list,
            commands::browse::vault_find_stream,
            commands::browse::vault_find_cancel,
            commands::browse::vault_list_revealed,
            commands::browse::vault_mkdir,
            commands::browse::local_path_kinds,
            commands::browse::vault_import_many,
            commands::browse::vault_export,
            commands::share::vault_share_file,
            commands::share::open_share_package,
            commands::jobs::job_list,
            commands::jobs::job_add,
            commands::jobs::job_remove,
            commands::jobs::job_set_enabled,
            commands::jobs::job_enable_auto_unlock,
            commands::jobs::job_history,
            commands::jobs::job_run_now,
            commands::overview::job_cancel,
            commands::overview::activity_recent,
            commands::overview::vault_recent,
            commands::overview::vault_sync_states,
            commands::overview::jobs_service_status,
            commands::jobs::job_top_level_folders,
            commands::jobs::job_set_excludes,
            commands::jobs::job_set_schedule_windows,
            commands::jobs::job_set_pre_hook,
            commands::jobs::job_set_post_hook,
            commands::jobs::job_set_anomaly_guard,
            commands::jobs::job_set_timing,
            commands::remotes::remotes_status,
            commands::remotes::remotes_migrate_legacy,
            commands::remotes::remotes_file_delete,
            commands::remotes::remote_list_names,
            commands::remotes::remote_unlock,
            commands::remotes::remote_lock,
            commands::remotes::s3_presets,
            commands::remotes::s3_preset_endpoint,
            commands::remotes::remote_add_s3,
            commands::remotes::remote_probe_sftp_host_key,
            commands::remotes::remote_add_sftp,
            commands::remotes::remote_add_gdrive,
            commands::remotes::remote_add_dropbox,
            commands::remotes::remote_add_onedrive,
            commands::remotes::remote_add_webdav,
            commands::remotes::remote_allow_insecure_http,
            commands::remotes::remote_remove,
            commands::remotes::remote_sweep,
            commands::security::security_policy_get,
            commands::security::app_security_get,
            commands::security::app_security_set,
            commands::security::panic_now,
            commands::security::memory_lock_status,
            commands::security::drive_denied_programs,
            commands::security::drive_access_answer,
            commands::security::security_runtime,
            commands::security::vault_auto_lock,
            commands::security::vault_force_lock,
            commands::security::security_policy_set,
            commands::security::network_proxy_get,
            commands::security::network_proxy_set,
            commands::settings::ui_settings_get,
            commands::settings::ui_settings_set,
            commands::settings::translation_export,
            commands::settings::bug_report_info,
            commands::settings::app_restart,
            commands::trash::vault_trash,
            commands::trash::trash_list,
            commands::trash::trash_restore,
            commands::trash::trash_purge,
            commands::trash::trash_empty,
            commands::mount::mount_start,
            commands::mount::mount_stop,
            commands::mount::mount_status,
            commands::mount::mount_open_folder,
            commands::versions::vault_versions,
            commands::versions::job_verify,
            commands::versions::vault_restore_version,
            commands::usage::storage_usage,
            commands::recovery_kit::recovery_kit_generate,
            commands::recovery_kit::recovery_kit_save,
            commands::registry::vault_registry_verify,
            commands::registry::vault_registry_reset,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| {
        background::on_run_event(app_handle, &event);
        if let tauri::RunEvent::Exit = event {
            use tauri::Manager;
            let state = app_handle.state::<state::AppState>();
            {
                let mut settings = state.settings.blocking_lock();
                if settings.security.clear_recents_on_exit && !settings.recent_vaults.is_empty() {
                    settings.recent_vaults.clear();
                    let _ = settings::save(&state.data_dir, &settings);
                }
            }
            #[cfg(any(target_os = "linux", windows))]
            if let Some(mounted) = state.mount.blocking_lock().take() {
                let _ = mounted.handle.unmount_forced();
            }
        }
    });
}
