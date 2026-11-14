//! Global keybinds (work while another app, e.g. a game, has focus).
//!
//! Discord's web client can only see keys while its own window is focused.
//! The page reports the keybinds the user created (their key codes); this
//! module watches the keyboard with a low-level hook **only while at least
//! one keybind exists**, recognises those exact combinations, and tells the
//! page when each one is pressed and released. The page then replays it as a
//! normal key event, so Discord's own keybind system runs the action
//! (toggle deafen, push-to-talk, …).
//!
//! Privacy: the hook never records, logs or forwards keystrokes; it only
//! compares key codes against the registered combinations, never blocks any
//! key, and is removed when the last keybind is gone. Keys sent by gaming
//! keyboard/mouse software (macros) count like physical keys.

use std::collections::HashSet;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Manager, Runtime};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, HHOOK, KBDLLHOOKSTRUCT, MSG, PostThreadMessageW, SetWindowsHookExW,
    TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

const MAX_KEYBINDS: usize = 64;
const MAX_KEYS_PER_COMBO: usize = 5;

/// JS key codes of the modifier keys (Ctrl, Shift, Alt, Meta).
fn is_modifier(code: u32) -> bool {
    matches!(code, 16 | 17 | 18 | 91)
}

/// Maps a Windows virtual-key code to the JS `keyCode` Discord uses
/// (left/right modifiers collapse to the generic one).
pub fn normalize(vk: u32) -> u32 {
    match vk {
        160 | 161 => 16, // shift
        162 | 163 => 17, // ctrl
        164 | 165 => 18, // alt
        91 | 92 => 91,   // win
        other => other,
    }
}

#[derive(serde::Deserialize, Clone, Debug)]
pub struct Keybind {
    pub id: String,
    pub keys: Vec<u32>,
}

struct Combo {
    id: String,
    keys: Vec<u32>,
    active: bool,
}

/// Pure matching logic (unit-tested; the hook only feeds it key events).
#[derive(Default)]
pub struct Matcher {
    combos: Vec<Combo>,
    pressed: HashSet<u32>,
}

impl Matcher {
    pub fn set(&mut self, keybinds: &[Keybind]) -> bool {
        let mut combos: Vec<Combo> = Vec::new();
        for kb in keybinds.iter().take(MAX_KEYBINDS) {
            let mut keys: Vec<u32> = kb.keys.iter().copied().filter(|k| (1..=254).contains(k)).collect();
            keys.sort_unstable();
            keys.dedup();
            if keys.is_empty() || keys.len() > MAX_KEYS_PER_COMBO || kb.id.is_empty() || kb.id.len() > 32 {
                continue;
            }
            let active = self.combos.iter().any(|c| c.id == kb.id && c.keys == keys && c.active);
            combos.push(Combo { id: kb.id.clone(), keys, active });
        }
        self.combos = combos;
        !self.combos.is_empty()
    }

    /// Feeds one key event; returns `(keybind id, pressed)` transitions.
    /// `still_down` reports whether the OS thinks a key is physically down (used
    /// to forget keys whose release was missed, e.g. across a lock screen).
    pub fn handle(&mut self, vk: u32, down: bool, still_down: impl Fn(u32) -> bool) -> Vec<(String, bool)> {
        let code = normalize(vk);
        if down {
            self.pressed.insert(code);
        } else {
            self.pressed.remove(&code);
        }
        // Drop stale keys other than the one in this event.
        let stale: Vec<u32> = self.pressed.iter().copied().filter(|k| *k != code && !still_down(*k)).collect();
        for k in stale {
            self.pressed.remove(&k);
        }

        let mut events = Vec::new();
        for combo in &mut self.combos {
            let all_down = combo.keys.iter().all(|k| self.pressed.contains(k));
            // No extra modifiers: Ctrl+D must not fire on Ctrl+Shift+D.
            let extra_modifier = self.pressed.iter().any(|k| is_modifier(*k) && !combo.keys.contains(k));
            let should_be_active = all_down && !extra_modifier;
            if should_be_active && !combo.active {
                combo.active = true;
                events.push((combo.id.clone(), true));
            } else if !should_be_active && combo.active {
                combo.active = false;
                events.push((combo.id.clone(), false));
            }
        }
        events
    }

    pub fn reset(&mut self) {
        self.pressed.clear();
        for c in &mut self.combos {
            c.active = false;
        }
    }
}

// ------------------------------------------------------------------ OS hook

struct Shared {
    matcher: Mutex<Matcher>,
    events: Mutex<Option<Sender<(String, bool)>>>,
    hook_thread: Mutex<Option<u32>>,
}

fn shared() -> &'static Shared {
    static SHARED: OnceLock<Shared> = OnceLock::new();
    SHARED.get_or_init(|| Shared { matcher: Mutex::new(Matcher::default()), events: Mutex::new(None), hook_thread: Mutex::new(None) })
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        // Rustcord never injects keys itself, so there is no feedback loop to guard against.
        let down = match wparam.0 as u32 {
            WM_KEYDOWN | WM_SYSKEYDOWN => Some(true),
            WM_KEYUP | WM_SYSKEYUP => Some(false),
            _ => None,
        };
        if let Some(down) = down {
            let transitions = lock(&shared().matcher)
                .handle(info.vkCode, down, |k| unsafe { (GetAsyncKeyState(k as i32) as u16 & 0x8000) != 0 });
            if !transitions.is_empty()
                && let Some(tx) = lock(&shared().events).as_ref()
            {
                for t in transitions {
                    let _ = tx.send(t);
                }
            }
        }
    }
    // Never swallow a key.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn start_hook_thread() {
    let (ready_tx, ready_rx) = channel::<u32>();
    std::thread::Builder::new()
        .name("rustcord-keyboard-hook".into())
        .spawn(move || unsafe {
            let hook: HHOOK = match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("rustcord: keyboard hook failed: {e}");
                    let _ = ready_tx.send(0);
                    return;
                }
            };
            let _ = ready_tx.send(GetCurrentThreadId());
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let _ = UnhookWindowsHookEx(hook);
        })
        .expect("spawn keyboard hook thread");
    if let Ok(id) = ready_rx.recv_timeout(std::time::Duration::from_secs(3))
        && id != 0
    {
        *lock(&shared().hook_thread) = Some(id);
    }
}

fn stop_hook_thread() {
    if let Some(id) = lock(&shared().hook_thread).take() {
        unsafe {
            let _ = PostThreadMessageW(id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
}

/// Replaces the set of global keybinds. Installs the hook when the first one
/// appears and removes it when the last one is gone.
pub fn set<R: Runtime>(app: &AppHandle<R>, keybinds: Vec<Keybind>) {
    let any = lock(&shared().matcher).set(&keybinds);
    let running = lock(&shared().hook_thread).is_some();
    if any && !running {
        lock(&shared().matcher).reset();
        ensure_dispatcher(app);
        start_hook_thread();
    } else if !any && running {
        stop_hook_thread();
        lock(&shared().matcher).reset();
    }
}

/// Worker that forwards transitions to the page, but only while Rustcord is
/// not the focused window (when it is, the page sees the real key events).
fn ensure_dispatcher<R: Runtime>(app: &AppHandle<R>) {
    let mut slot = lock(&shared().events);
    if slot.is_some() {
        return;
    }
    let (tx, rx) = channel::<(String, bool)>();
    *slot = Some(tx);
    let app = app.clone();
    std::thread::Builder::new()
        .name("rustcord-keybind-dispatch".into())
        .spawn(move || {
            let mut sent_down: HashSet<String> = HashSet::new();
            while let Ok((id, down)) = rx.recv() {
                let Some(window) = app.get_webview_window(crate::MAIN) else { continue };
                if down {
                    if window.is_focused().unwrap_or(false) {
                        continue;
                    }
                    sent_down.insert(id.clone());
                } else if !sent_down.remove(&id) {
                    continue;
                }
                if let Ok(id_js) = serde_json::to_string(&id) {
                    let _ = window.eval(format!("window.__rcKeybind && window.__rcKeybind({id_js}, {down})"));
                }
            }
        })
        .expect("spawn keybind dispatch thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kb(id: &str, keys: &[u32]) -> Keybind {
        Keybind { id: id.into(), keys: keys.to_vec() }
    }

    fn run(m: &mut Matcher, events: &[(u32, bool)]) -> Vec<(String, bool)> {
        // Keys stay "down" for the stale check while they are in the pressed set.
        let mut out = Vec::new();
        for (vk, down) in events {
            out.extend(m.handle(*vk, *down, |_| true));
        }
        out
    }

    #[test]
    fn single_key_press_and_release() {
        let mut m = Matcher::default();
        assert!(m.set(&[kb("1", &[119])]));
        let t = run(&mut m, &[(119, true), (119, true), (119, false)]);
        assert_eq!(t, vec![("1".into(), true), ("1".into(), false)], "auto-repeat must not re-fire");
    }

    #[test]
    fn modifier_combo_is_exact() {
        let mut m = Matcher::default();
        m.set(&[kb("d", &[17, 68])]);
        // Ctrl+Shift+D must not trigger Ctrl+D.
        assert!(run(&mut m, &[(162, true), (160, true), (68, true)]).is_empty());
        run(&mut m, &[(68, false), (160, false)]);
        // Ctrl is still down; D now triggers it.
        let t = run(&mut m, &[(68, true), (68, false)]);
        assert_eq!(t, vec![("d".into(), true), ("d".into(), false)]);
    }

    #[test]
    fn releasing_a_modifier_releases_the_combo() {
        let mut m = Matcher::default();
        m.set(&[kb("p", &[17, 80])]);
        let t = run(&mut m, &[(163, true), (80, true), (163, false)]);
        assert_eq!(t, vec![("p".into(), true), ("p".into(), false)]);
    }

    #[test]
    fn stale_keys_are_forgotten() {
        let mut m = Matcher::default();
        m.set(&[kb("a", &[65])]);
        // Ctrl "stuck" in the set (missed key-up) but the OS says it is up.
        m.handle(162, true, |_| true);
        let t = m.handle(65, true, |k| k != 17);
        assert_eq!(t, vec![("a".into(), true)]);
    }

    #[test]
    fn rejects_invalid_keybinds() {
        let mut m = Matcher::default();
        assert!(!m.set(&[kb("", &[65]), kb("x", &[]), kb("y", &[0]), kb("z", &[1, 2, 3, 4, 5, 6])]));
        assert!(m.set(&[kb("ok", &[65, 65])]), "duplicates collapse");
    }
}
