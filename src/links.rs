//! `discord://` deep links (e.g. from notification apps): parsing, in-app
//! navigation, and the opt-in registration of Rustcord as their handler.

use crate::config::ConfigState;
use tauri::{AppHandle, Manager, Runtime};

/// Top-level discord.com routes a link may open. Anything else is ignored.
const ALLOWED_ROOTS: &[&str] = &[
    "channels",
    "invite",
    "users",
    "store",
    "shop",
    "discovery",
    "guild-discovery",
    "events",
    "quests",
    "activities",
    "library",
];

/// Converts a `discord://…` link into a discord.com path such as
/// `/channels/123/456`. Returns `None` for anything that is not a safe,
/// known Discord route.
///
/// Accepted forms: `discord://-/channels/…`, `discord:///channels/…`,
/// `discord://discord.com/channels/…`, `discord://channels/…`.
pub fn parse(link: &str) -> Option<String> {
    let url = url::Url::parse(link.trim()).ok()?;
    if !url.scheme().eq_ignore_ascii_case("discord") {
        return None;
    }
    let mut segments: Vec<String> = Vec::new();
    match url.host_str().unwrap_or("") {
        "" | "-" | "app" | "discord.com" | "discordapp.com" | "ptb.discord.com" | "canary.discord.com" => {}
        host => segments.push(host.to_owned()),
    }
    if let Some(path) = url.path_segments() {
        segments.extend(path.filter(|s| !s.is_empty()).map(str::to_owned));
    }
    let root = segments.first()?.to_ascii_lowercase();
    if !ALLOWED_ROOTS.contains(&root.as_str()) || segments.len() > 8 {
        return None;
    }
    let safe = |s: &str| {
        !s.is_empty()
            && s.len() <= 64
            && !s.bytes().all(|b| b == b'.')
            && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"@._-".contains(&b))
    };
    if !segments.iter().all(|s| safe(s)) {
        return None;
    }
    let mut path = format!("/{}", segments.join("/"));
    if let Some(q) = url.query().filter(|q| !q.is_empty() && q.len() <= 512) {
        path.push('?');
        path.push_str(q);
    }
    Some(path)
}

/// The first deep link among command-line arguments.
pub fn find_in_args<S: AsRef<str>>(args: &[S]) -> Option<String> {
    args.iter().find_map(|a| a.as_ref().to_ascii_lowercase().starts_with("discord:").then(|| parse(a.as_ref())).flatten())
}

/// Brings Rustcord forward and opens the path without reloading Discord.
pub fn open<R: Runtime>(app: &AppHandle<R>, path: &str) {
    crate::show_main(app);
    if let Some(w) = app.get_webview_window(crate::MAIN)
        && let Ok(p) = serde_json::to_string(path)
    {
        let _ = w.eval(format!("window.__rcNavigate ? window.__rcNavigate({p}) : location.assign({p})"));
    }
}

// ------------------------------------------------------------------ handler registration

const KEY: &str = r"Software\Classes\discord";
const COMMAND_KEY: &str = r"Software\Classes\discord\shell\open\command";
const ICON_KEY: &str = r"Software\Classes\discord\DefaultIcon";

fn exe() -> Option<String> {
    std::env::current_exe().ok().map(|p| p.display().to_string())
}

fn our_command() -> Option<String> {
    Some(format!("\"{}\" \"%1\"", exe()?))
}

fn current_command() -> Option<String> {
    windows_registry::CURRENT_USER.open(COMMAND_KEY).ok()?.get_string("").ok()
}

fn current_icon() -> Option<String> {
    windows_registry::CURRENT_USER.open(ICON_KEY).ok()?.get_string("").ok()
}

/// True when `discord://` currently opens this Rustcord executable.
pub fn is_registered() -> bool {
    match (current_command(), our_command()) {
        (Some(cur), Some(ours)) => cur.eq_ignore_ascii_case(&ours),
        _ => false,
    }
}

/// Points `discord://` at Rustcord, remembering the previous handler once so
/// it can be restored when the user turns this off.
pub fn register<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    if is_registered() {
        return Ok(());
    }
    let (Some(command), Some(exe)) = (our_command(), exe()) else { return Err("executable path unknown".into()) };
    let previous = current_command();
    let previous_icon = current_icon();
    app.state::<ConfigState>().update(|c| {
        // Only remember a handler that is not Rustcord itself.
        if previous.as_deref().is_some_and(|p| !p.to_ascii_lowercase().contains("rustcord")) {
            c.previous_discord_handler = previous.clone();
            c.previous_discord_icon = previous_icon.clone();
        }
    });
    let hk = windows_registry::CURRENT_USER;
    let root = hk.create(KEY).map_err(|e| e.to_string())?;
    root.set_string("", "URL:Discord Protocol").map_err(|e| e.to_string())?;
    root.set_string("URL Protocol", "").map_err(|e| e.to_string())?;
    hk.create(ICON_KEY).and_then(|k| k.set_string("", format!("\"{exe}\",0"))).map_err(|e| e.to_string())?;
    hk.create(COMMAND_KEY).and_then(|k| k.set_string("", command)).map_err(|e| e.to_string())?;
    Ok(())
}

/// Gives `discord://` back to the previous handler (normally the Discord app).
pub fn unregister<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    if !is_registered() {
        return Ok(());
    }
    let cfg = app.state::<ConfigState>().get();
    let hk = windows_registry::CURRENT_USER;
    // Restore only if the previous program still exists; otherwise remove our
    // command and let the Discord app register itself again when it starts.
    let restorable = cfg.previous_discord_handler.as_deref().filter(|cmd| {
        let path = cmd.trim().trim_start_matches('"');
        let path = path.split('"').next().unwrap_or(path);
        std::path::Path::new(path).is_file()
    });
    match restorable {
        Some(cmd) => {
            hk.create(COMMAND_KEY).and_then(|k| k.set_string("", cmd)).map_err(|e| e.to_string())?;
            if let Some(icon) = cfg.previous_discord_icon.as_deref() {
                let _ = hk.create(ICON_KEY).and_then(|k| k.set_string("", icon));
            }
        }
        None => {
            hk.remove_tree(r"Software\Classes\discord\shell").map_err(|e| e.to_string())?;
            let _ = hk.remove_tree(ICON_KEY);
        }
    }
    Ok(())
}

/// While enabled, re-claims `discord://` if the Discord app takes it back
/// (it re-registers itself when it starts or updates).
pub fn start_guard<R: Runtime>(app: AppHandle<R>) {
    std::thread::Builder::new()
        .name("rustcord-link-guard".into())
        .spawn(move || loop {
            if app.state::<ConfigState>().get().handle_discord_links && !is_registered() {
                let _ = register(&app);
            }
            std::thread::sleep(std::time::Duration::from_secs(60));
        })
        .expect("spawn link guard thread");
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn parses_common_link_forms() {
        assert_eq!(parse("discord://-/channels/1/2").as_deref(), Some("/channels/1/2"));
        assert_eq!(parse("discord:///channels/@me/9").as_deref(), Some("/channels/@me/9"));
        assert_eq!(parse("discord://discord.com/channels/1/2/3").as_deref(), Some("/channels/1/2/3"));
        assert_eq!(parse("discord://channels/1/2").as_deref(), Some("/channels/1/2"));
        assert_eq!(parse("DISCORD://-/invite/abcDEF").as_deref(), Some("/invite/abcDEF"));
        assert_eq!(parse("discord://-/events/1/2?x=1").as_deref(), Some("/events/1/2?x=1"));
    }

    #[test]
    fn rejects_unknown_or_unsafe_links() {
        for bad in [
            "https://discord.com/channels/1/2",
            "discord://-/",
            "discord://-/login",
            "discord://-/channels/1/a b",
            "discord://-/channels/1/%2Fx",
            "discord://evil.com/x",
            "discord://-/channels/<script>",
            "notaurl",
        ] {
            assert_eq!(parse(bad), None, "{bad} should be rejected");
        }
    }

    #[test]
    fn dot_segments_never_survive() {
        for link in ["discord://-/channels/1/..", "discord://-/channels/1/%2e%2e", "discord://-/channels/./1"] {
            assert!(parse(link).is_none_or(|p| !p.split('/').any(|s| s == "." || s == "..")), "{link}");
        }
    }

    #[test]
    fn finds_link_among_arguments() {
        let args = ["C:\\rustcord.exe", "--minimized", "discord://-/channels/1/2"];
        assert_eq!(super::find_in_args(&args).as_deref(), Some("/channels/1/2"));
        assert_eq!(super::find_in_args(&["C:\\rustcord.exe"]), None);
    }
}
