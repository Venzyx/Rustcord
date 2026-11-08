//! Game detection: matches running processes against Discord's public list of
//! detectable applications (fetched by the page) and reports "Playing …".

use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager, Runtime};
use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::core::PWSTR;

const SCAN_INTERVAL: Duration = Duration::from_secs(10);

/// One entry of the page-provided list (already filtered to Windows, non-launcher executables).
#[derive(serde::Deserialize)]
pub struct Detectable {
    pub id: String,
    pub name: String,
    pub exes: Vec<String>,
}

struct Game {
    id: String,
    name: String,
}

/// Executable basename -> (game index, full pattern).
#[derive(Default)]
struct Index {
    games: Vec<Game>,
    by_basename: HashMap<String, Vec<(usize, String)>>,
}

#[derive(Default, Clone)]
pub struct Detector {
    index: Arc<Mutex<Option<Index>>>,
}

fn normalize(path: &str) -> String {
    path.to_ascii_lowercase().replace('\\', "/")
}

impl Detector {
    pub fn load(&self, list: Vec<Detectable>) -> usize {
        let mut index = Index::default();
        for d in list.into_iter().take(50_000) {
            if !d.id.bytes().all(|b| b.is_ascii_digit()) || d.id.is_empty() {
                continue;
            }
            let gi = index.games.len();
            let mut any = false;
            for exe in d.exes.iter().take(32) {
                let pattern = normalize(exe.trim());
                let base = pattern.trim_start_matches('>').rsplit('/').next().unwrap_or_default().to_owned();
                if base.len() < 3 {
                    continue;
                }
                index.by_basename.entry(base).or_default().push((gi, pattern));
                any = true;
            }
            if any {
                index.games.push(Game { id: d.id, name: d.name.chars().take(128).collect() });
            }
        }
        let n = index.games.len();
        *self.index.lock().unwrap_or_else(|e| e.into_inner()) = Some(index);
        n
    }
}

/// Match score for a pattern against a process, `None` if it doesn't match.
/// Many games share an executable name (e.g. Source mods all run `hl2.exe`),
/// so a process is attributed to its *most specific* match only: patterns
/// that include directories beat bare names, which beat `>exact` names.
fn match_score(pattern: &str, base: &str, full: &str) -> Option<usize> {
    if let Some(exact) = pattern.strip_prefix('>') {
        return (base == exact).then_some(0);
    }
    if full == pattern || full.ends_with(&format!("/{pattern}")) {
        let dirs = pattern.matches('/').count();
        return Some(1 + dirs * 1000 + pattern.len());
    }
    None
}

/// Picks the best game for one process from its basename candidates.
fn best_match(candidates: &[(usize, String)], base: &str, full: &str) -> Option<usize> {
    let mut best: Option<(usize, usize)> = None; // (score, game index)
    for (gi, pattern) in candidates {
        if let Some(score) = match_score(pattern, base, full) {
            // Strictly greater: on ties the earlier (canonical) entry wins.
            if best.is_none_or(|(s, _)| score > s) {
                best = Some((score, *gi));
            }
        }
    }
    best.map(|(_, gi)| gi)
}

struct Proc {
    pid: u32,
    base: String,
}

fn list_processes() -> Vec<Proc> {
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                let base = String::from_utf16_lossy(&entry.szExeFile[..len]).to_ascii_lowercase();
                out.push(Proc { pid: entry.th32ProcessID, base });
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    out
}

/// Full image path (normalized) and creation time (unix ms) of a process.
fn process_info(pid: u32) -> Option<(String, u64)> {
    unsafe {
        let handle: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let path = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
            .ok()
            .map(|_| normalize(&String::from_utf16_lossy(&buf[..len as usize])));
        let (mut created, mut exited, mut kernel, mut user) =
            (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let start = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user).ok().map(|_| {
            let ticks = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
            // FILETIME is 100ns since 1601-01-01.
            (ticks / 10_000).saturating_sub(11_644_473_600_000)
        });
        let _ = CloseHandle(handle);
        Some((path?, start.unwrap_or(0)))
    }
}

pub fn start<R: Runtime>(app: AppHandle<R>) {
    let detector = app.state::<Detector>().inner().clone();
    std::thread::Builder::new()
        .name("rustcord-game-detect".into())
        .spawn(move || {
            let own_pid = std::process::id();
            // application id -> (pid, start ms)
            let mut running: HashMap<String, (u32, u64)> = HashMap::new();
            loop {
                std::thread::sleep(SCAN_INTERVAL);
                let found: HashMap<String, (u32, u64, String)> = {
                    let guard = detector.index.lock().unwrap_or_else(|e| e.into_inner());
                    let Some(index) = guard.as_ref() else { continue };
                    let mut found = HashMap::new();
                    for p in list_processes() {
                        if p.pid == own_pid || p.pid <= 4 {
                            continue;
                        }
                        let Some(candidates) = index.by_basename.get(&p.base) else { continue };
                        // Elevated / protected games may refuse path queries: fall
                        // back to name-only matching instead of missing them.
                        let (full, start) = process_info(p.pid).unwrap_or_else(|| (p.base.clone(), 0));
                        if let Some(gi) = best_match(candidates, &p.base, &full) {
                            let game = &index.games[gi];
                            found.entry(game.id.clone()).or_insert((p.pid, start, game.name.clone()));
                        }
                    }
                    found
                };

                let presence = app.state::<super::Presence>();
                let current: HashSet<String> = found.keys().cloned().collect();
                // Games that stopped (or whose own RPC took over).
                let stale: Vec<String> = running
                    .keys()
                    .filter(|id| !current.contains(*id) || presence.is_rpc_app_active(id))
                    .cloned()
                    .collect();
                for id in stale {
                    running.remove(&id);
                    super::publish(&app, &format!("rustcord-game-{id}"), None, None, false);
                }
                for (id, (pid, start, name)) in found {
                    if running.contains_key(&id) || presence.is_rpc_app_active(&id) {
                        continue;
                    }
                    let start_ms = if start > 0 { start } else { now_ms() };
                    let activity = json!({
                        "application_id": id,
                        "name": name,
                        "type": 0,
                        "timestamps": { "start": start_ms },
                    });
                    super::publish(&app, &format!("rustcord-game-{id}"), Some(pid), Some(activity), false);
                    running.insert(id, (pid, start_ms));
                }
            }
        })
        .expect("spawn game detection thread");
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(entries: &[(&str, &str, &[&str])]) -> Detector {
        let d = Detector::default();
        d.load(
            entries
                .iter()
                .map(|(id, name, exes)| Detectable {
                    id: (*id).into(),
                    name: (*name).into(),
                    exes: exes.iter().map(|s| (*s).into()).collect(),
                })
                .collect(),
        );
        d
    }

    fn detect(d: &Detector, path: &str) -> Option<String> {
        let full = normalize(path);
        let base = full.rsplit('/').next().unwrap().to_owned();
        let guard = d.index.lock().unwrap();
        let idx = guard.as_ref().unwrap();
        let cands = idx.by_basename.get(&base)?;
        best_match(cands, &base, &full).map(|gi| idx.games[gi].name.clone())
    }

    #[test]
    fn shared_executable_resolves_to_most_specific_game() {
        let d = index(&[
            ("111111111111111111", "Garry's Mod", &["garrysmod/hl2.exe", ">hl2.exe", "win64/gmod.exe"]),
            ("222222222222222222", "Team Fortress 2", &[">hl2.exe"]),
            ("333333333333333333", "Portal", &["portal/hl2.exe"]),
        ]);
        assert_eq!(detect(&d, r"C:\Steam\steamapps\common\GarrysMod\hl2.exe").as_deref(), Some("Garry's Mod"));
        assert_eq!(detect(&d, r"C:\Steam\steamapps\common\Portal\hl2.exe").as_deref(), Some("Portal"));
        assert_eq!(detect(&d, r"D:\GarrysMod\bin\win64\gmod.exe").as_deref(), Some("Garry's Mod"));
        // Only exact-name matches: the first (canonical) entry wins, never several games.
        assert_eq!(detect(&d, r"C:\Games\Other\hl2.exe").as_deref(), Some("Garry's Mod"));
    }

    #[test]
    fn path_patterns_require_directory_boundary() {
        let d = index(&[("444444444444444444", "Fortnite", &["fortniteclient-win64-shipping.exe"])]);
        assert_eq!(detect(&d, r"C:\Epic\FortniteGame\Binaries\Win64\FortniteClient-Win64-Shipping.exe").as_deref(), Some("Fortnite"));
        let d = index(&[("555555555555555555", "Portal", &["portal/hl2.exe"])]);
        assert_eq!(detect(&d, r"C:\Games\notportal\hl2.exe"), None);
    }

    #[test]
    fn rejects_invalid_ids_and_short_names() {
        let d = index(&[("abc", "Bad", &["game.exe"]), ("666666666666666666", "Tiny", &["a"])]);
        assert!(d.index.lock().unwrap().as_ref().unwrap().games.is_empty());
    }
}
