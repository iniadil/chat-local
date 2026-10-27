use lanchat_net::{Event, Identity, Node, NodeConfig, Settings};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

/// Name of the webview event carrying a [`lanchat_net::Event`].
pub const NET_EVENT: &str = "net";

pub struct AppState {
    pub data_dir: PathBuf,
    pub identity: Identity,
    pub settings: Mutex<Settings>,
    pub node: Mutex<Option<Arc<Node>>>,
    /// Last error from starting the node, shown in the UI.
    pub start_error: Mutex<Option<String>>,
}

#[derive(Serialize, Deserialize)]
struct StoredIdentity {
    id: String,
    name: String,
}

impl AppState {
    pub fn load(data_dir: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        std::fs::create_dir_all(&data_dir)?;
        let identity = load_or_create_identity(&data_dir)?;
        let settings = load_settings(&data_dir);
        Ok(Self {
            data_dir,
            identity,
            settings: Mutex::new(settings),
            node: Mutex::new(None),
            start_error: Mutex::new(None),
        })
    }

    pub fn node(&self) -> Option<Arc<Node>> {
        self.node.lock().unwrap().clone()
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    pub fn save_settings(&self, settings: Settings) -> Result<(), String> {
        let path = self.data_dir.join("settings.json");
        let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| e.to_string())?;
        *self.settings.lock().unwrap() = settings;
        Ok(())
    }
}

fn load_or_create_identity(dir: &Path) -> Result<Identity, Box<dyn std::error::Error>> {
    let path = dir.join("identity.json");
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(s) = serde_json::from_str::<StoredIdentity>(&text) {
            return Ok(Identity {
                id: s.id,
                name: s.name,
            });
        }
    }
    let name = hostname::get()
        .ok()
        .map(|h| h.to_string_lossy().to_string())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "Me".to_string());
    let stored = StoredIdentity {
        id: uuid::Uuid::new_v4().to_string(),
        name,
    };
    std::fs::write(&path, serde_json::to_string_pretty(&stored)?)?;
    Ok(Identity {
        id: stored.id,
        name: stored.name,
    })
}

fn load_settings(dir: &Path) -> Settings {
    std::fs::read_to_string(dir.join("settings.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// (Re)start the node with the current settings and forward its events to
/// the webview. Any previously running node is shut down first.
pub async fn start_node(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let old = state.node.lock().unwrap().take();
    if let Some(old) = old {
        old.shutdown();
        drop(old);
        // Give the listener a moment to release the port before rebinding.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    *state.start_error.lock().unwrap() = None;

    let config = NodeConfig {
        identity: state.identity.clone(),
        settings: state.settings(),
        data_dir: state.data_dir.clone(),
    };
    let (node, mut events) = match Node::start(config).await {
        Ok(v) => v,
        Err(e) => {
            let msg = format!("{e}");
            *state.start_error.lock().unwrap() = Some(msg.clone());
            let _ = app.emit(
                NET_EVENT,
                Event::Error {
                    message: msg.clone(),
                },
            );
            return Err(msg);
        }
    };
    *state.node.lock().unwrap() = Some(node);

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(ev) = events.recv().await {
            if let Err(e) = app.emit(NET_EVENT, &ev) {
                tracing::warn!("could not emit event: {e}");
            }
        }
    });
    Ok(())
}
