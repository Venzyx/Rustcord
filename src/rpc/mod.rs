//! Rich Presence: a local Discord RPC server (named pipe) plus process-based
//! game detection. Activities are handed to the web client, which publishes
//! them through its own gateway connection exactly like the desktop app does.

pub mod detect;
pub mod ipc;

use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, Runtime};

/// Identity reported to RPC clients in the READY event.
#[derive(Clone, Default, serde::Deserialize)]
pub struct RpcUser {
    pub id: String,
    pub username: String,
    #[serde(default)]
    pub global_name: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub discriminator: Option<String>,
}

#[derive(Default)]
struct Inner {
    /// socket id -> {socketId, pid, activity} message last sent for it.
    activities: HashMap<String, Value>,
    /// application ids currently set by RPC clients (suppress duplicate game detection).
    rpc_apps: HashMap<String, String>,
    bridge_ready: bool,
    enabled: bool,
    user: Option<RpcUser>,
}

pub struct Presence {
    inner: Mutex<Inner>,
}

impl Presence {
    pub fn new(enabled: bool) -> Self {
        Self { inner: Mutex::new(Inner { enabled, ..Default::default() }) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn user(&self) -> Option<RpcUser> {
        self.lock().user.clone()
    }

    pub fn is_rpc_app_active(&self, application_id: &str) -> bool {
        self.lock().rpc_apps.values().any(|a| a == application_id)
    }
}

fn send_to_page<R: Runtime>(app: &AppHandle<R>, msg: &Value) {
    if let Some(w) = app.get_webview_window(crate::MAIN) {
        // JSON is valid JavaScript; serde_json escapes everything needed.
        let _ = w.eval(format!("window.__rcActivity && window.__rcActivity({msg})"));
    }
}

/// Sets (`Some`) or clears (`None`) the activity for one source.
pub fn publish<R: Runtime>(app: &AppHandle<R>, socket_id: &str, pid: Option<u32>, activity: Option<Value>, from_rpc: bool) {
    let presence = app.state::<Presence>();
    let msg = json!({ "socketId": socket_id, "pid": pid, "activity": activity });
    let forward = {
        let mut inner = presence.lock();
        if from_rpc {
            match activity.as_ref().and_then(|a| a.get("application_id")).and_then(Value::as_str) {
                Some(id) => inner.rpc_apps.insert(socket_id.to_owned(), id.to_owned()),
                None => inner.rpc_apps.remove(socket_id),
            };
        }
        let had = if activity.is_some() {
            inner.activities.insert(socket_id.to_owned(), msg.clone()).is_some()
        } else {
            inner.activities.remove(socket_id).is_some()
        };
        inner.enabled && inner.bridge_ready && (activity.is_some() || had)
    };
    if forward {
        send_to_page(app, &msg);
    }
}

/// The page bridge found Discord's dispatcher (after every page load).
pub fn bridge_ready<R: Runtime>(app: &AppHandle<R>, user: Option<RpcUser>) {
    let presence = app.state::<Presence>();
    let replay: Vec<Value> = {
        let mut inner = presence.lock();
        inner.bridge_ready = true;
        if user.is_some() {
            inner.user = user;
        }
        if inner.enabled { inner.activities.values().cloned().collect() } else { Vec::new() }
    };
    for msg in replay {
        send_to_page(app, &msg);
    }
}

/// Page is reloading/navigating: activities must be re-sent once it is back.
pub fn bridge_lost<R: Runtime>(app: &AppHandle<R>) {
    app.state::<Presence>().lock().bridge_ready = false;
}

pub fn set_enabled<R: Runtime>(app: &AppHandle<R>, enabled: bool) {
    let presence = app.state::<Presence>();
    let msgs: Vec<Value> = {
        let mut inner = presence.lock();
        if inner.enabled == enabled {
            return;
        }
        inner.enabled = enabled;
        if !inner.bridge_ready {
            return;
        }
        inner
            .activities
            .values()
            .map(|m| {
                if enabled {
                    m.clone()
                } else {
                    json!({ "socketId": m["socketId"], "pid": m["pid"], "activity": Value::Null })
                }
            })
            .collect()
    };
    for msg in msgs {
        send_to_page(app, &msg);
    }
}

pub fn start<R: Runtime>(app: &AppHandle<R>) {
    ipc::start(app.clone());
    detect::start(app.clone());
}
