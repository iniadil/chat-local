use crate::state::{self, AppState};
use base64::Engine;
use lanchat_net::{FoundPeer, Message, PeerInfo, Settings};
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
    /// No settings have been saved yet: the app was just installed and
    /// should ask for a name before anything else.
    pub first_run: bool,
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
        first_run: !state.data_dir.join("settings.json").exists(),
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
        // `import_file` checks that it is a regular file under the size cap.
        let msg = node
            .import_file(to.clone(), &path, Some(mime_for(&path)))
            .await
            .map_err(|e| format!("{p}: {e}"))?;
        sent.push(msg);
    }
    Ok(sent)
}

#[derive(Serialize)]
pub struct FileInfo {
    pub path: String,
    pub file_name: String,
    pub size: u64,
    pub mime: String,
    /// `data:` URL of the image, for the composer preview. Only for images
    /// under a few megabytes; larger ones show as a file chip.
    pub preview: Option<String>,
}

/// Preview size cap: a data URL of this many bytes is cheap enough to hold
/// in the webview while the user decides whether to send.
const PREVIEW_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// Describe a file the user dropped or picked, before it is sent. The
/// webview cannot read arbitrary paths (the asset protocol is scoped to the
/// app folder), so image previews travel as a data URL.
#[tauri::command]
pub async fn file_info(path: String) -> Result<FileInfo, String> {
    let p = PathBuf::from(&path);
    let meta = tokio::fs::metadata(&p)
        .await
        .map_err(|e| format!("{path}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{path}: not a file"));
    }
    // Reject oversized files now, so the user hears about it when queueing
    // rather than when pressing Send.
    if meta.len() > lanchat_net::MAX_ATTACHMENT_BYTES {
        let e = lanchat_net::Error::AttachmentTooLarge(meta.len());
        return Err(format!("{path}: {e}"));
    }
    let mime = mime_for(&p);
    let file_name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());
    let preview = if mime.starts_with("image/") && meta.len() <= PREVIEW_MAX_BYTES {
        let bytes = tokio::fs::read(&p)
            .await
            .map_err(|e| format!("{path}: {e}"))?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        Some(format!("data:{mime};base64,{b64}"))
    } else {
        None
    };
    Ok(FileInfo {
        path,
        file_name,
        size: meta.len(),
        mime,
        preview,
    })
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

/// Peers announced over mDNS, for the "Add computer" dialog. Instant.
#[tauri::command]
pub fn discovered_peers(state: State<'_, AppState>) -> Result<Vec<FoundPeer>, String> {
    Ok(node_or_err(&state)?.discovered())
}

/// Probe the local subnets for other ChatLocal installations. Takes a few
/// seconds; the result also includes what mDNS knows.
#[tauri::command]
pub async fn scan_network(state: State<'_, AppState>) -> Result<Vec<FoundPeer>, String> {
    let node = node_or_err(&state)?;
    Ok(node.scan_lan().await)
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
