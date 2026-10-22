mod commands;
mod live;
mod scan;
mod settings;
mod state;
mod win;

use tauri::Manager;

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .manage(state::AppState::default())
        .manage(scan::HelperState::default())
        .manage(settings::SettingsState(std::sync::Mutex::new(
            settings::load(),
        )))
        .setup(|app| {
            let window = app.get_webview_window("main").unwrap();
            win::apply_backdrop(&window);
            scan::init_drives(app.handle());
            live::restore(app.handle().clone());
            scan::check_helper(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::drives,
            commands::restored,
            commands::helper_status,
            commands::enable_fast_scan,
            commands::scan,
            commands::cancel_scan,
            commands::rows,
            commands::toggle,
            commands::set_sort,
            commands::reveal,
            commands::node_path,
            commands::ancestors,
            commands::treemap,
            commands::sunburst,
            commands::node_info,
            commands::show_in_explorer,
            commands::backdrop,
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
