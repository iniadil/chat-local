use crate::state::{self, AppState};
use base64::Engine;
use lanchat_net::{Message, PeerInfo, Settings};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, State};

#[derive(Serialize)]
pub struct Snapshot {
    pub me: lanchat_net::Identity,
    pub settings: Settings,
    pub port: Option<u16>,
    pub addresses: Vec<String>,
    pub peers: Vec<PeerInfo>,
    pub messages: Vec<Message>,
    pub data_dir: String,
    pub start_error: Option<String>,
}

fn snapshot(state: &AppState) -> Snapshot {
    let node = state.node();
    Snapshot {
        me: lanchat_net::Identity {
            id: state.identity.id.clone(),
            name: node
                .as_ref()
                .map(|n| n.identity.name.clone())
                .unwrap_or_else(|| state.identity.name.clone()),
        },
        settings: state.settings(),
        port: node.as_ref().map(|n| n.port()),
        addresses: lanchat_net::local_ipv4_addresses(),
        peers: node.as_ref().map(|n| n.peers()).unwrap_or_default(),
        messages: node.as_ref().map(|n| n.messages()).unwrap_or_default(),
        data_dir: state.data_dir.to_string_lossy().to_string(),
        start_error: state.start_error.lock().unwrap().clone(),
    }
}

fn node_or_err(state: &AppState) -> Result<std::sync::Arc<lanchat_net::Node>, String> {
    state
        .node()
        .ok_or_else(|| "networking is not running; check Settings".to_string())
}

#[tauri::command]
pub fn app_snapshot(state: State<'_, AppState>) -> Snapshot {
    snapshot(&state)
}

#[tauri::command]
pub async fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<Snapshot, String> {
    let mut settings = settings;
    settings.display_name = settings.display_name.trim().chars().take(48).collect();
    settings.manual_peers = settings
        .manual_peers
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if settings.port == 0 {
        return Err("port must be between 1 and 65535".into());
    }
    state.save_settings(settings)?;
    state::start_node(&app).await?;
    Ok(snapshot(&state))
}

#[tauri::command]
pub async fn send_text(
    state: State<'_, AppState>,
    to: Option<String>,
    text: String,
) -> Result<Message, String> {
    node_or_err(&state)?
        .send_text(to, text)
        .await
        .map_err(|e| e.to_string())
}

/// Send files chosen in a dialog or dropped on the window. A copy of each file
/// is kept in the app's files folder so the preview keeps working even if the
/// original is moved.
#[tauri::command]
pub async fn send_files(
    state: State<'_, AppState>,
    to: Option<String>,
    paths: Vec<String>,
) -> Result<Vec<Message>, String> {
    let node = node_or_err(&state)?;
    let mut sent = Vec::new();
    for p in paths {
        let path = PathBuf::from(&p);
        let meta = tokio::fs::metadata(&path)
            .await
            .map_err(|e| format!("{p}: {e}"))?;
        if !meta.is_file() {
            return Err(format!("{p}: not a file"));
        }
        if meta.len() > lanchat_net::MAX_ATTACHMENT_BYTES {
            return Err(format!(
                "{p}: larger than {} MB",
                lanchat_net::MAX_ATTACHMENT_BYTES / (1024 * 1024)
            ));
        }
        let msg = node
            .import_file(to.clone(), &path, Some(mime_for(&path)))
            .await
            .map_err(|e| e.to_string())?;
        sent.push(msg);
    }
    Ok(sent)
}

#[derive(Deserialize)]
pub struct Blob {
    pub file_name: String,
    pub mime: String,
    /// Base64 (standard alphabet) of the bytes.
    pub data: String,
    pub duration_ms: Option<u32>,
}

/// Send bytes produced in the webview: a voice note or a pasted image.
#[tauri::command]
pub async fn send_blob(
    state: State<'_, AppState>,
    to: Option<String>,
    blob: Blob,
) -> Result<Message, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(blob.data.as_bytes())
        .map_err(|e| format!("bad base64: {e}"))?;
    node_or_err(&state)?
        .send_bytes(to, blob.file_name, blob.mime, bytes, blob.duration_ms)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn connect_peer(state: State<'_, AppState>, target: String) -> Result<(), String> {
    let node = node_or_err(&state)?;
    node.connect(&target).await.map_err(|e| e.to_string())
}

fn attachment_path(state: &AppState, message_id: &str) -> Result<PathBuf, String> {
    let node = node_or_err(state)?;
    let msg = node
        .find_message(message_id)
        .ok_or_else(|| "message not found".to_string())?;
    let path = msg
        .attachment
        .and_then(|a| a.path)
        .ok_or_else(|| "message has no file".to_string())?;
    // Only ever open files we wrote ourselves.
    let files_dir = node.files_dir();
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    let root = files_dir.canonicalize().map_err(|e| e.to_string())?;
    if !canonical.starts_with(&root) {
        return Err("file is outside the app folder".into());
    }
    Ok(canonical)
}

#[tauri::command]
pub fn open_attachment(state: State<'_, AppState>, message_id: String) -> Result<(), String> {
    let path = attachment_path(&state, &message_id)?;
    tauri_plugin_opener::open_path(path, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn reveal_attachment(state: State<'_, AppState>, message_id: String) -> Result<(), String> {
    let path = attachment_path(&state, &message_id)?;
    tauri_plugin_opener::reveal_item_in_dir(path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn clear_history(state: State<'_, AppState>) -> Result<(), String> {
    node_or_err(&state)?
        .clear_history()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn local_addresses() -> Vec<String> {
    lanchat_net::local_ipv4_addresses()
}

fn mime_for(path: &Path) -> String {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "heic" => "image/heic",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "ogg" | "oga" => "audio/ogg",
        "pdf" => "application/pdf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "txt" | "md" | "csv" => "text/plain",
        "zip" => "application/zip",
        "mp4" => "video/mp4",
        _ => "application/octet-stream",
    }
    .to_string()
}
