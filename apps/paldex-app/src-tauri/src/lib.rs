mod commands;
mod queries;
mod sync;

/// Build and run the Paldex desktop application.
///
/// # Panics
///
/// Panics if the webview or window cannot be created, which is unrecoverable.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::list_saves,
            commands::resolve_folder,
            commands::select_world,
            commands::force_resync,
            commands::pal_roster,
            commands::dex_progress,
            commands::player_progress,
            commands::base_summary,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Paldex");
}
