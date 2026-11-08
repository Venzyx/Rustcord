//! Small Windows integrations: AppUserModelID, toast registration, locale,
//! and opening links in the user's default browser.

use windows::Win32::UI::Shell::{SetCurrentProcessExplicitAppUserModelID, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, w};

pub const AUMID: &str = "dev.rustcord.app";

/// Gives the process a stable identity (taskbar grouping, toast sender) and
/// registers the unpackaged app as a toast notification source.
pub fn register_identity(icon_png: &std::path::Path) {
    unsafe {
        let _ = SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(AUMID));
    }
    if let Ok(key) = windows_registry::CURRENT_USER
        .create(format!(r"Software\Classes\AppUserModelId\{AUMID}"))
    {
        let _ = key.set_string("DisplayName", "Rustcord");
        let _ = key.set_string("IconUri", icon_png.to_string_lossy().as_ref());
        let _ = key.set_string("IconBackgroundColor", "FFDE5A24");
    }
}

/// True when the Windows UI language is Turkish.
pub fn is_turkish() -> bool {
    let lang = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    (lang & 0x3ff) == 0x1f
}

/// Opens a URL in the default handler. Only web and mail links are allowed so
/// page content can never launch arbitrary protocols or local files.
pub fn open_external(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else { return false };
    if !matches!(parsed.scheme(), "http" | "https" | "mailto") {
        return false;
    }
    let target = HSTRING::from(parsed.as_str());
    let result = unsafe { ShellExecuteW(None, w!("open"), &target, None, None, SW_SHOWNORMAL) };
    result.0 as usize > 32
}

/// Hosts that stay inside the app window. Everything else opens in the browser.
pub fn is_internal_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == "discord.com"
        || host.ends_with(".discord.com")
        || host == "discord.gg"
        || host == "discordapp.com"
        || host.ends_with(".discordapp.com")
}
