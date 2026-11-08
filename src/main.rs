//! Rustcord: a lightweight native (Rust + system WebView2) desktop shell for
//! the official Discord web client. Sign-in happens on discord.com itself;
//! Rustcord never sees or stores credentials.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod badge;
mod config;
mod notify;
mod platform;
mod rpc;
mod webview2;

use config::ConfigState;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

pub const MAIN: &str = "main";
const APP_URL: &str = "https://discord.com/app";
const TRAY_ICON: &[u8] = include_bytes!("../icons/32x32.png");
const BACKGROUND: tauri::window::Color = tauri::window::Color(0x1a, 0x1a, 0x1e, 0xff);

/// WebView2 flags: Tauri's defaults plus features that only cost memory in a
/// single-site chat client.
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,\
msEdgeTranslate,msImplicitSignin,msEdgeShoppingAssistant,msEdgeCollections,AutofillServerCommunication,\
Translate,MediaRouter,OptimizationHints --disable-background-networking --autoplay-policy=no-user-gesture-required";

struct Runtime_ {
    tray: Mutex<Option<TrayIcon>>,
    tray_base: Image<'static>,
    tray_dot: Image<'static>,
    shown_once: AtomicBool,
    start_hidden: bool,
}

struct Strings {
    open: &'static str,
    autostart: &'static str,
    to_tray: &'static str,
    presence: &'static str,
    reload: &'static str,
    quit: &'static str,
}

fn strings() -> Strings {
    if platform::is_turkish() {
        Strings {
            open: "Rustcord'u Aç",
            autostart: "Windows ile başlat",
            to_tray: "Kapatınca tepsiye küçült",
            presence: "Oyun etkinliğini paylaş",
            reload: "Yeniden yükle",
            quit: "Rustcord'dan Çık",
        }
    } else {
        Strings {
            open: "Open Rustcord",
            autostart: "Start with Windows",
            to_tray: "Minimize to tray on close",
            presence: "Share game activity",
            reload: "Reload",
            quit: "Quit Rustcord",
        }
    }
}

pub fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window(MAIN) {
        webview2::set_low_memory(&w, false);
        let _ = w.show();
        if w.is_minimized().unwrap_or(false) {
            let _ = w.unminimize();
        }
        let _ = w.set_focus();
    }
}

fn hide_main<R: Runtime>(w: &WebviewWindow<R>) {
    let _ = w.hide();
    webview2::set_low_memory(w, true);
}

fn close_or_hide<R: Runtime>(w: &WebviewWindow<R>) {
    let to_tray = w.app_handle().state::<ConfigState>().get().minimize_to_tray;
    if to_tray {
        hide_main(w);
    } else {
        w.app_handle().exit(0);
    }
}

// ------------------------------------------------------------------ commands

#[tauri::command]
fn rc_window(window: WebviewWindow, action: String) -> Result<bool, String> {
    match action.as_str() {
        "minimize" => window.minimize().map_err(|e| e.to_string())?,
        "toggle_maximize" => {
            if window.is_maximized().unwrap_or(false) {
                window.unmaximize().map_err(|e| e.to_string())?;
            } else {
                window.maximize().map_err(|e| e.to_string())?;
            }
        }
        "close" => close_or_hide(&window),
        "drag" => window.start_dragging().map_err(|e| e.to_string())?,
        "is_maximized" => {}
        "devtools" => {
            if cfg!(debug_assertions) {
                webview2::open_devtools(&window);
            }
        }
        _ => return Err("unknown action".into()),
    }
    Ok(window.is_maximized().unwrap_or(false))
}

#[tauri::command]
async fn rc_notify(app: AppHandle, id: u32, title: String, body: String, icon: Option<String>) -> Result<(), String> {
    notify::show(&app, id, &title, &body, icon.as_deref())
}

#[tauri::command]
fn rc_set_badge(app: AppHandle, mentions: u32, unread: bool) {
    let _ = unread;
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.set_overlay_icon(if mentions > 0 { Some(badge::overlay(mentions)) } else { None });
    }
    let rt = app.state::<Runtime_>();
    if let Some(tray) = rt.tray.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let icon = if mentions > 0 { rt.tray_dot.clone() } else { rt.tray_base.clone() };
        let _ = tray.set_icon(Some(icon));
        let tip = if mentions > 0 { format!("Rustcord ({mentions})") } else { "Rustcord".to_string() };
        let _ = tray.set_tooltip(Some(tip));
    }
}

#[tauri::command]
fn rc_open_external(url: String) -> bool {
    platform::open_external(&url)
}

#[tauri::command]
fn rc_rpc_ready(app: AppHandle, user: Option<rpc::RpcUser>) {
    rpc::bridge_ready(&app, user);
}

#[tauri::command]
fn rc_rpc_detectables(app: AppHandle, list: Vec<rpc::detect::Detectable>) -> usize {
    app.state::<rpc::detect::Detector>().load(list)
}

// ------------------------------------------------------------------ setup

fn build_tray(app: &AppHandle) -> tauri::Result<TrayIcon> {
    let s = strings();
    let autostart_on = app.autolaunch().is_enabled().unwrap_or(false);
    let to_tray_on = app.state::<ConfigState>().get().minimize_to_tray;
    let presence_on = app.state::<ConfigState>().get().rich_presence;

    let open = MenuItem::with_id(app, "open", s.open, true, None::<&str>)?;
    let autostart = CheckMenuItem::with_id(app, "autostart", s.autostart, true, autostart_on, None::<&str>)?;
    let to_tray = CheckMenuItem::with_id(app, "to_tray", s.to_tray, true, to_tray_on, None::<&str>)?;
    let presence = CheckMenuItem::with_id(app, "presence", s.presence, true, presence_on, None::<&str>)?;
    let reload = MenuItem::with_id(app, "reload", s.reload, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", s.quit, true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &sep1, &autostart, &to_tray, &presence, &reload, &sep2, &quit])?;

    let base = app.state::<Runtime_>().tray_base.clone();
    TrayIconBuilder::with_id("rustcord")
        .icon(base)
        .tooltip("Rustcord")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "open" => show_main(app),
            "autostart" => {
                let mgr = app.autolaunch();
                let enable = !mgr.is_enabled().unwrap_or(false);
                let _ = if enable { mgr.enable() } else { mgr.disable() };
                let _ = autostart.set_checked(mgr.is_enabled().unwrap_or(false));
            }
            "to_tray" => {
                let state = app.state::<ConfigState>();
                state.update(|c| c.minimize_to_tray = !c.minimize_to_tray);
                let _ = to_tray.set_checked(state.get().minimize_to_tray);
            }
            "presence" => {
                let state = app.state::<ConfigState>();
                state.update(|c| c.rich_presence = !c.rich_presence);
                let on = state.get().rich_presence;
                let _ = presence.set_checked(on);
                rpc::set_enabled(app, on);
            }
            "reload" => {
                if let Some(w) = app.get_webview_window(MAIN) {
                    let _ = w.eval("location.reload()");
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        })
        .build(app)
}

fn build_window(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let script = include_str!("inject/inject.js").replace("__RC_VERSION__", env!("CARGO_PKG_VERSION"));
    let url = APP_URL.parse().expect("valid url");
    WebviewWindowBuilder::new(app, MAIN, WebviewUrl::External(url))
        .title("Rustcord")
        .inner_size(1280.0, 760.0)
        .min_inner_size(940.0, 500.0)
        .center()
        .decorations(false)
        .shadow(true)
        .visible(false)
        .background_color(BACKGROUND)
        .disable_drag_drop_handler()
        .additional_browser_args(BROWSER_ARGS)
        .initialization_script(&script)
        .on_navigation(|url| {
            let internal = match url.scheme() {
                "https" => url.host_str().is_some_and(platform::is_internal_host),
                "http" | "tauri" => url.host_str() == Some("tauri.localhost") || url.scheme() == "tauri",
                "about" | "blob" | "data" => true,
                _ => false,
            };
            if !internal {
                platform::open_external(url.as_str());
            }
            internal
        })
        .on_page_load(|w, payload| {
            if payload.event() == PageLoadEvent::Started {
                rpc::bridge_lost(w.app_handle());
                return;
            }
            let app = w.app_handle();
            let rt = app.state::<Runtime_>();
            if !rt.shown_once.swap(true, Ordering::SeqCst) && !rt.start_hidden {
                show_main(app);
            }
        })
        .build()
}

fn main() {
    let start_hidden_flag = std::env::args().any(|a| a == "--minimized");
    let tray_base = Image::from_bytes(TRAY_ICON).expect("embedded tray icon").to_owned();
    let tray_dot = badge::tray_with_dot(&tray_base);

    tauri::Builder::default()
        // Must be first: a second launch focuses the running instance.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--minimized"])))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::all() - tauri_plugin_window_state::StateFlags::VISIBLE,
                )
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            rc_window,
            rc_notify,
            rc_set_badge,
            rc_open_external,
            rc_rpc_ready,
            rc_rpc_detectables
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let config = ConfigState::load(&handle);
            let start_hidden = start_hidden_flag && config.get().start_minimized;
            app.manage(rpc::Presence::new(config.get().rich_presence));
            app.manage(rpc::detect::Detector::default());
            app.manage(config);
            app.manage(Runtime_ {
                tray: Mutex::new(None),
                tray_base,
                tray_dot,
                shown_once: AtomicBool::new(false),
                start_hidden,
            });

            // Toast identity: icon file next to the cache, AUMID in HKCU.
            if let Ok(dir) = handle.path().app_data_dir() {
                let _ = std::fs::create_dir_all(&dir);
                // Rewrite when it differs so a logo change reaches existing installs.
                const TOAST_ICON: &[u8] = include_bytes!("../icons/128x128.png");
                let icon = dir.join("toast-icon.png");
                if std::fs::read(&icon).ok().as_deref() != Some(TOAST_ICON) {
                    let _ = std::fs::write(&icon, TOAST_ICON);
                }
                platform::register_identity(&icon);
            }
            notify::prune_cache(&handle);

            let window = build_window(&handle)?;
            webview2::configure(&window);
            let tray = build_tray(&handle)?;
            *app.state::<Runtime_>().tray.lock().unwrap_or_else(|e| e.into_inner()) = Some(tray);
            rpc::start(&handle);

            // Safety net: show the window even if the first load never finishes.
            if !start_hidden {
                let h = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(6));
                    let h2 = h.clone();
                    let _ = h.run_on_main_thread(move || {
                        if !h2.state::<Runtime_>().shown_once.swap(true, Ordering::SeqCst) {
                            show_main(&h2);
                        }
                    });
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event
                && window.label() == MAIN
                && window.app_handle().state::<ConfigState>().get().minimize_to_tray
            {
                api.prevent_close();
                if let Some(w) = window.app_handle().get_webview_window(MAIN) {
                    hide_main(&w);
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build Rustcord")
        .run(|_app, _event| {});
}
