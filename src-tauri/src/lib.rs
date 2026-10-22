mod commands;
mod scan;
mod state;
mod win;

use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .manage(state::AppState::default())
        .manage(scan::HelperState::default())
        .setup(|app| {
            let window = app.get_webview_window("main").unwrap();
            win::apply_backdrop(&window);
            scan::init_drives(app.handle());
            scan::check_helper(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::drives,
            commands::helper_status,
            commands::enable_fast_scan,
            commands::scan,
            commands::cancel_scan,
            commands::rows,
            commands::toggle,
            commands::set_sort,
            commands::reveal,
            commands::node_path,
            commands::show_in_explorer,
            commands::backdrop,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Bulk");
}
