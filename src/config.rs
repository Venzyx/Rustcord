//! Persistent user preferences (stored as JSON in the app config directory).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, Runtime};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Closing the window hides it to the tray instead of quitting (Discord's default).
    pub minimize_to_tray: bool,
    /// When launched at login, stay in the tray.
    pub start_minimized: bool,
    /// Share game activity / Rich Presence from local apps.
    pub rich_presence: bool,
    /// Open `discord://` links (e.g. from notification apps) in Rustcord (opt-in).
    pub handle_discord_links: bool,
    /// The `discord://` handler that was registered before Rustcord took over.
    pub previous_discord_handler: Option<String>,
    pub previous_discord_icon: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            minimize_to_tray: true,
            start_minimized: true,
            rich_presence: true,
            handle_discord_links: false,
            previous_discord_handler: None,
            previous_discord_icon: None,
        }
    }
}

pub struct ConfigState {
    path: PathBuf,
    inner: Mutex<Config>,
}

impl ConfigState {
    pub fn load<R: Runtime>(app: &AppHandle<R>) -> Self {
        let path = app
            .path()
            .app_config_dir()
            .unwrap_or_else(|_| std::env::temp_dir().join("rustcord"))
            .join("config.json");
        let inner = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self { path, inner: Mutex::new(inner) }
    }

    pub fn get(&self) -> Config {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn update(&self, f: impl FnOnce(&mut Config)) {
        let snapshot = {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            f(&mut guard);
            guard.clone()
        };
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(&snapshot) {
            // Write-then-rename so a crash never leaves a truncated config.
            let tmp = self.path.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &self.path);
            }
        }
    }
}
