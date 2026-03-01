use tauri::Manager;

#[tauri::command]
fn get_api_profile() -> String {
    std::env::var("NEBULA_API_PROFILE").unwrap_or_else(|_| "local".to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .invoke_handler(tauri::generate_handler![get_api_profile])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
