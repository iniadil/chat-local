//! Tauri shell around `lanchat-net`: owns the running node, persists identity
//! and settings, forwards network events to the webview and exposes commands.

mod commands;
mod state;

use state::AppState;
use tauri::Manager;

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,mdns_sd=warn".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let state = AppState::load(data_dir)?;
            app.manage(state);
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = state::start_node(&handle).await {
                    tracing::error!("could not start networking: {e}");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_snapshot,
            commands::save_settings,
            commands::send_text,
            commands::send_files,
            commands::send_blob,
            commands::connect_peer,
            commands::open_attachment,
            commands::reveal_attachment,
            commands::clear_history,
            commands::local_addresses,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ChatLocal");
}
