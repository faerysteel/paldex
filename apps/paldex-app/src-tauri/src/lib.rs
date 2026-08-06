mod commands;

/// Build and run the Paldex desktop application.
///
/// # Panics
///
/// Panics if the webview or window cannot be created, which is unrecoverable.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::list_saves,
            commands::resolve_folder,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Paldex");
}
