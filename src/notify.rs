//! Native Windows toasts for the web client's Notification API.

use base64::Engine;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use tauri::{AppHandle, Manager, Runtime};
use tauri_winrt_notification::{IconCrop, Sound, Toast};

const MAX_ICON_BYTES: usize = 512 * 1024;

fn cache_icon<R: Runtime>(app: &AppHandle<R>, b64: &str) -> Option<PathBuf> {
    if b64.len() > MAX_ICON_BYTES * 4 / 3 + 4 {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    // Only accept PNG (the page script re-encodes everything to PNG).
    if !bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return None;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    let dir = app.path().app_cache_dir().ok()?.join("notification-icons");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{:016x}.png", hasher.finish()));
    if !path.exists() {
        std::fs::write(&path, bytes).ok()?;
    }
    Some(path)
}

/// Trims the icon cache so it can't grow without bound.
pub fn prune_cache<R: Runtime>(app: &AppHandle<R>) {
    let Ok(dir) = app.path().app_cache_dir().map(|d| d.join("notification-icons")) else { return };
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    let mut files: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    if files.len() <= 300 {
        return;
    }
    files.sort();
    for (_, path) in files.iter().take(files.len() - 200) {
        let _ = std::fs::remove_file(path);
    }
}

pub fn show<R: Runtime>(app: &AppHandle<R>, id: u32, title: &str, body: &str, icon_b64: Option<&str>) -> Result<(), String> {
    let title: String = title.chars().take(256).collect();
    let body: String = body.chars().take(1024).collect();
    let mut toast = Toast::new(crate::platform::AUMID).title(&title).text1(&body).sound(None::<Sound>);
    if let Some(path) = icon_b64.and_then(|b| cache_icon(app, b)) {
        toast = toast.icon(&path, IconCrop::Circular, "");
    }
    let handle = app.clone();
    toast
        .on_activated(move |_| {
            let h = handle.clone();
            let _ = handle.run_on_main_thread(move || {
                crate::show_main(&h);
                if let Some(w) = h.get_webview_window(crate::MAIN) {
                    let _ = w.eval(format!("window.__rcNotificationActivated && window.__rcNotificationActivated({id})"));
                }
            });
            Ok(())
        })
        .show()
        .map_err(|e| e.to_string())
}
