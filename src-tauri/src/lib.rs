mod app_update;
mod assist;
mod commands;
mod config;
mod detection;
mod games;
mod github_release;
#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]
mod linux_embed;
#[cfg(target_os = "linux")]
mod linux_startup;
mod migration;
mod modio_api;
mod mods;
mod nexus;
mod share;
mod thunderstore;
mod tools;

use commands::AppState;
use tauri_plugin_deep_link::DeepLinkExt;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Must run before GTK/Tauri: a full /tmp (quota) makes icon load assert-abort.
    #[cfg(target_os = "linux")]
    {
        let _ = linux_startup::ensure_writable_tmpdir();
    }

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let state = match AppState::new() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to init app state: {e:#}");
            std::process::exit(1);
        }
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .register_uri_scheme_protocol("nxm", |ctx, request| {
            let uri = request.uri().to_string();
            assist::handle_nxm_uri_scheme(ctx.app_handle(), &uri);
            tauri::http::Response::builder()
                .status(204)
                .body(Vec::new())
                .unwrap()
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_info,
            app_update::check_app_update,
            app_update::install_app_update,
            commands::scan_games,
            commands::list_plugins,
            commands::list_managed,
            commands::manage_game,
            commands::update_managed_game,
            commands::detect_ue_layout,
            commands::suggest_catalog_ids,
            commands::suggest_catalog_ids_batch,
            commands::unmanage_game,
            commands::get_game_health,
            commands::relink_managed_game,
            commands::get_settings,
            commands::set_adult_content,
            commands::set_theme,
            commands::set_autoclick_free_download,
            commands::set_install_click_behavior,
            commands::set_active_game,
            commands::set_api_key,
            commands::validate_user,
            commands::clear_api_key,
            commands::set_modio_api_key,
            commands::clear_modio_api_key,
            commands::search_mods,
            commands::search_catalog,
            commands::get_thunderstore_package,
            commands::download_thunderstore_mod,
            commands::get_modio_mod,
            commands::modio_files,
            commands::download_modio_mod,
            commands::get_game,
            commands::get_mod,
            commands::mod_files,
            commands::search_collections,
            commands::search_collection_catalog,
            commands::browse_meta,
            commands::get_collection,
            commands::collection_files,
            commands::download_mod,
            commands::install_collection,
            commands::import_thunderstore_profile,
            commands::export_share_code,
            commands::decode_share_code,
            commands::detect_import_code,
            commands::import_code,
            commands::import_share_code,
            commands::record_emperor_share,
            commands::list_saved_collections,
            commands::save_collection_code,
            commands::get_saved_collection,
            commands::rename_saved_collection,
            commands::delete_saved_collection,
            commands::handle_nxm,
            commands::import_mod_archive,
            commands::import_assist_download,
            commands::scan_mod_orphans,
            commands::recover_legacy_mod_data,
            commands::list_mods,
            commands::check_staged_mod_updates,
            commands::update_staged_mod,
            commands::list_installed_collections,
            commands::uninstall_collection,
            commands::record_nexus_collection,
            commands::set_mod_enabled,
            commands::set_load_order,
            commands::get_mod_options,
            commands::set_mod_options,
            commands::list_mods_with_options,
            commands::remove_mod,
            commands::remove_all_mods,
            commands::deploy_mods,
            commands::purge_mods,
            commands::list_downloads,
            commands::clear_recent_downloads,
            commands::remove_download,
            commands::cancel_download,
            commands::cancel_download_batch,
            commands::pause_download,
            commands::resume_download,
            commands::restart_download,
            commands::force_reset_download,
            assist::open_download_assist,
            assist::close_download_assist,
            assist::clear_assist_session,
            assist::set_assist_bounds,
            assist::set_assist_visible,
            #[cfg(target_os = "linux")]
            tools::get_tools_status,
            #[cfg(target_os = "linux")]
            tools::check_tool_updates,
            #[cfg(target_os = "linux")]
            tools::install_tool,
            tools::get_lsfg_vk_config,
            tools::set_lsfg_vk_config,
            tools::get_autohdr_vk_config,
            tools::set_autohdr_vk_config,
            tools::detect_game_executables,
            tools::get_game_tools,
            tools::set_game_tools,
            tools::sync_game_tools,
        ])
        .setup(|app| {
            // Register nxm:// with the OS when supported (Windows installer / Linux desktop).
            if let Err(e) = app.deep_link().register_all() {
                log::warn!("deep-link register_all failed: {e}");
            }
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    let s = url.to_string();
                    if s.starts_with("nxm://") {
                        assist::emit_nxm_url(&handle, &s);
                    }
                }
            });
            for arg in std::env::args().skip(1) {
                if arg.starts_with("nxm://") {
                    assist::emit_nxm_url(app.handle(), &arg);
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
