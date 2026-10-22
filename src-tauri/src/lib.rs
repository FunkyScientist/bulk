mod cart;
mod commands;
mod dupes;
mod history;
mod live;
mod scan;
mod settings;
mod shell;
mod state;
mod win;

use tauri::Manager;

pub fn run() {
    let opened = shell::open_arg(&std::env::args().collect::<Vec<_>>());
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            shell::on_second_instance(app, args);
        }))
        .plugin(tauri_plugin_dialog::init())
        .manage(state::AppState::default())
        .manage(scan::HelperState::default())
        .manage(cart::CartState::default())
        .manage(dupes::DupesState::default())
        .manage(shell::OpenState(std::sync::Mutex::new(opened)))
        .manage(settings::SettingsState(std::sync::Mutex::new(
            bulk_core::settings::load(),
        )))
        .setup(|app| {
            let window = app.get_webview_window("main").unwrap();
            win::apply_backdrop(&window);
            // Keep the Explorer entry pointing at this copy of the app.
            let _ = shell::set_explorer_menu(settings::get(app.handle()).explorer_menu);
            scan::init_drives(app.handle());
            live::restore(app.handle().clone());
            scan::check_helper(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::drives,
            commands::restored,
            commands::helper_status,
            commands::allow_fast_mode,
            commands::scan,
            commands::cancel_scan,
            commands::rows,
            commands::toggle,
            commands::set_sort,
            commands::reveal,
            commands::node_path,
            commands::rel_path,
            commands::find_path,
            commands::ancestors,
            commands::treemap,
            commands::sunburst,
            commands::node_info,
            commands::search,
            commands::show_in_explorer,
            commands::open_path,
            commands::export,
            commands::backdrop,
            commands::app_version,
            history::history,
            history::diff_open,
            history::diff_scope,
            history::diff_children,
            history::diff_close,
            cart::cart_get,
            cart::cart_add,
            cart::cart_remove,
            cart::cart_clear,
            cart::cart_delete,
            cart::delete_nodes,
            cart::cart_undo,
            dupes::dupes_start,
            dupes::dupes_cancel,
            settings::settings_get,
            settings::settings_set,
            settings::snapshot_storage,
            shell::take_open_path,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Bulk");

    app.run(|app, event| {
        // Save live changes so the next launch opens instantly and up to date.
        if let tauri::RunEvent::ExitRequested { .. } = event {
            live::save_all(app);
        }
    });
}
