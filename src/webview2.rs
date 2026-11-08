//! Direct WebView2 tuning that Tauri doesn't expose: browser chrome removal,
//! media permissions, new-window routing, offline fallback and memory targets.

use tauri::{Runtime, WebviewWindow};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    NavigationCompletedEventHandler, NewWindowRequestedEventHandler, PermissionRequestedEventHandler,
    take_pwstr,
};
use windows::core::{BOOL, Interface, PWSTR};

fn is_discord_origin(uri: &str) -> bool {
    url::Url::parse(uri)
        .map(|u| u.scheme() == "https" && u.host_str().is_some_and(crate::platform::is_internal_host))
        .unwrap_or(false)
}

pub fn configure<R: Runtime>(window: &WebviewWindow<R>) {
    let _ = window.with_webview(|pw| unsafe {
        let Ok(core) = pw.controller().CoreWebView2() else { return };
        if let Err(e) = apply(&core) {
            eprintln!("rustcord: webview2 configuration failed: {e}");
        }
    });
}

unsafe fn apply(core: &ICoreWebView2) -> windows::core::Result<()> {
    unsafe {
        let settings = core.Settings()?;
        settings.SetIsStatusBarEnabled(false)?;
        settings.SetIsZoomControlEnabled(true)?;
        if let Ok(s3) = settings.cast::<ICoreWebView2Settings3>() {
            // Removes browser-only shortcuts (find bar, print, F5, caret browsing…).
            s3.SetAreBrowserAcceleratorKeysEnabled(false)?;
        }
        if let Ok(s4) = settings.cast::<ICoreWebView2Settings4>() {
            s4.SetIsPasswordAutosaveEnabled(false)?;
            s4.SetIsGeneralAutofillEnabled(false)?;
        }
        if let Ok(s5) = settings.cast::<ICoreWebView2Settings5>() {
            s5.SetIsPinchZoomEnabled(false)?;
        }
        if let Ok(s6) = settings.cast::<ICoreWebView2Settings6>() {
            s6.SetIsSwipeNavigationEnabled(false)?;
        }

        // Grant what a voice/video chat client needs, only to Discord itself;
        // everything else gets the default (prompt/deny) behaviour.
        let mut token = 0i64;
        core.add_PermissionRequested(
            &PermissionRequestedEventHandler::create(Box::new(|_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut uri = PWSTR::null();
                args.Uri(&mut uri)?;
                let uri = take_pwstr(uri);
                let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
                args.PermissionKind(&mut kind)?;
                let allowed = matches!(
                    kind,
                    COREWEBVIEW2_PERMISSION_KIND_MICROPHONE
                        | COREWEBVIEW2_PERMISSION_KIND_CAMERA
                        | COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS
                        | COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ
                        | COREWEBVIEW2_PERMISSION_KIND_AUTOPLAY
                        | COREWEBVIEW2_PERMISSION_KIND_WINDOW_MANAGEMENT
                );
                if allowed && is_discord_origin(&uri) {
                    args.SetState(COREWEBVIEW2_PERMISSION_STATE_ALLOW)?;
                }
                Ok(())
            })),
            &mut token,
        )?;

        // window.open / target=_blank: Discord links stay in the app,
        // everything else opens in the default browser. Never spawn popups.
        let mut token = 0i64;
        core.add_NewWindowRequested(
            &NewWindowRequestedEventHandler::create(Box::new(|sender, args| {
                let Some(args) = args else { return Ok(()) };
                let mut uri = PWSTR::null();
                args.Uri(&mut uri)?;
                let uri = take_pwstr(uri);
                args.SetHandled(true)?;
                if is_discord_origin(&uri) {
                    if let Some(core) = sender {
                        let wide = windows::core::HSTRING::from(uri.as_str());
                        core.Navigate(&wide)?;
                    }
                } else if !uri.is_empty() && uri != "about:blank" {
                    crate::platform::open_external(&uri);
                }
                Ok(())
            })),
            &mut token,
        )?;

        // Replace WebView2's browser error page with our own offline screen.
        let mut token = 0i64;
        core.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(|sender, args| {
                let (Some(core), Some(args)) = (sender, args) else { return Ok(()) };
                let mut ok = BOOL::default();
                args.IsSuccess(&mut ok)?;
                if ok.as_bool() {
                    return Ok(());
                }
                let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
                args.WebErrorStatus(&mut status)?;
                if matches!(
                    status,
                    COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED
                        | COREWEBVIEW2_WEB_ERROR_STATUS_VALID_AUTHENTICATION_CREDENTIALS_REQUIRED
                        | COREWEBVIEW2_WEB_ERROR_STATUS_VALID_PROXY_AUTHENTICATION_REQUIRED
                ) {
                    return Ok(());
                }
                let mut src = PWSTR::null();
                core.Source(&mut src)?;
                let src = take_pwstr(src);
                if !is_discord_origin(&src) {
                    return Ok(());
                }
                let target = url::form_urlencoded::byte_serialize(src.as_bytes()).collect::<String>();
                let offline = format!("http://tauri.localhost/index.html?to={target}");
                core.Navigate(&windows::core::HSTRING::from(offline))?;
                Ok(())
            })),
            &mut token,
        )?;
    }
    Ok(())
}

/// Hint WebView2 to trim memory while hidden in the tray, restore when shown.
pub fn set_low_memory<R: Runtime>(window: &WebviewWindow<R>, low: bool) {
    let _ = window.with_webview(move |pw| unsafe {
        let Ok(core) = pw.controller().CoreWebView2() else { return };
        if let Ok(c19) = core.cast::<ICoreWebView2_19>() {
            let level = if low {
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
            } else {
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
            };
            let _ = c19.SetMemoryUsageTargetLevel(level);
        }
    });
}

pub fn open_devtools<R: Runtime>(window: &WebviewWindow<R>) {
    let _ = window.with_webview(|pw| unsafe {
        if let Ok(core) = pw.controller().CoreWebView2() {
            let _ = core.OpenDevToolsWindow();
        }
    });
}
