<div align="center">

<img src="icons/128x128.png" width="96" alt="Rustcord icon">

# Rustcord

**A lightweight Windows desktop app for Discord, written in Rust.**

Same Discord you already use, without bundling a whole browser.

[English](#english) · [Türkçe](#türkçe)

</div>

> [!CAUTION]
> **Unofficial project. Use at your own risk.** Rustcord is not made, endorsed or
> supported by Discord Inc. Using third-party clients or client modifications is
> against [Discord's Terms of Service](https://discord.com/terms) and **may lead to
> your account being restricted or terminated**. The authors accept **no
> responsibility or liability** of any kind. Read [Risks](#risks) and
> [Disclaimer](#disclaimer) before using it.

---

## English

### What it is

Rustcord is a small native shell around the **official Discord web client**
(`https://discord.com/app`). Instead of shipping its own copy of Chromium and
Node.js like the Electron desktop app, it uses the WebView2 runtime that is
already part of Windows 10/11, and implements the desktop integrations in Rust.

- You sign in on **Discord's own login page** (password, QR code, 2FA, passkeys).
  Rustcord never sees, stores or transmits your credentials or token.
- The interface is Discord's own, so it looks and behaves like Discord.

It was inspired by Discord's announced move away from Electron to a native Rust
client. Rustcord is **not** that client and is not related to it.

### Features

| Area | What Rustcord does |
| --- | --- |
| Window | Frameless window, window controls inside Discord's top bar, size/position remembered |
| Tray | Open, start with Windows, minimize to tray on close, game activity on/off, reload, quit |
| Notifications | Native Windows toasts; clicking one opens the message |
| Badges | Mention count on the taskbar icon, red dot on the tray icon |
| Rich Presence | Local `discord-ipc` server for games and apps that use Rich Presence |
| Game detection | "Playing …" from Discord's public list of detectable games |
| Go Live | Stream Quality menu (resolution + FPS) while streaming, with Discord's own Nitro rules |
| Links | Non-Discord links open in your default browser |
| Privacy | Microphone/camera are granted to discord.com only |
| Offline | Built-in reconnect screen instead of a browser error page |
| Memory | WebView2 memory target is lowered while hidden in the tray |

### Measured memory use

Measured on one Windows 11 machine with Task Manager's "Memory" metric
(private working set), all child processes included. Your numbers will vary.

| State | Memory |
| --- | --- |
| Rustcord, window open (Friends page) | ~460 MB |
| Rustcord, hidden in tray (after ~30 s) | ~180 MB |
| Official app, same account (in a voice call, with a client mod) | ~1,000 MB |

The two apps were not in identical states, so treat this as an indication, not a
benchmark. `scripts/measure-memory.ps1` reproduces the measurement.

### Not supported (yet)

These depend on Discord's closed-source native modules:

- In-game overlay
- Global push-to-talk and keybinds while another app is focused
- Krisp noise suppression (Discord's standard noise suppression works)
- Discord's own screen picker (the Windows picker is used)
- RPC commands that need OAuth (`AUTHORIZE`, `AUTHENTICATE`) and the RPC WebSocket transport

### Risks

Please understand these before using Rustcord:

1. **Terms of Service.** Discord does not allow third-party clients or client
   modifications. Rustcord loads the official web client, but it also changes
   how parts of it behave (see below). Discord could restrict or terminate
   accounts that use it. There is no known safe level of use.
2. **Client modification.** To provide Rich Presence and the Stream Quality menu,
   Rustcord interacts with the web client's internal code at runtime (the same
   technique used by client mods such as Vencord). Discord updates can break
   these features at any time; Rustcord is designed so that Discord itself keeps
   working if a hook stops matching.
3. **Local Rich Presence server.** Like the official app, Rustcord listens on the
   local named pipe `\\.\pipe\discord-ipc-N`. Any program running under your
   Windows user can connect to it and set your activity, and it receives your
   public user info (id, username, avatar). It cannot read messages or act on
   your account. You can turn game activity off from the tray menu.
4. **No warranty.** This is an independent hobby project with no security audit.
   It can contain bugs, including ones that affect your account, data or computer.
5. **Downloads.** Only build from source or download from this repository.
   Never enter your Discord password anywhere other than Discord's own page.

### Build

Requirements: Windows 10/11, Rust (stable), WebView2 runtime (preinstalled on
Windows 11).

```bash
cargo build --release
```

The app is `target/release/rustcord.exe`. To build an installer:

```bash
cargo install tauri-cli --version "^2" --locked
cargo tauri build
```

Tests: `cargo test`

### Project layout

| Path | Purpose |
| --- | --- |
| `src/main.rs` | App setup, window, tray, IPC commands |
| `src/webview2.rs` | WebView2 tuning: permissions, new windows, offline page, memory |
| `src/rpc/` | Rich Presence: `ipc.rs` (named-pipe server), `detect.rs` (game detection) |
| `src/notify.rs` | Native toast notifications |
| `src/badge.rs` | Runtime-rendered taskbar/tray badges |
| `src/platform.rs` | AppUserModelID, locale, safe external links |
| `src/config.rs` | Persistent preferences |
| `src/inject/inject.js` | Page integration: notifications, window controls, badge, presence, stream quality |
| `capabilities/discord.json` | The only commands discord.com is allowed to call |
| `dist/index.html` | Offline/reconnect screen |
| `scripts/` | Memory measurement and an RPC test client |

### Disclaimer

THIS SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND. THE AUTHORS AND
CONTRIBUTORS ACCEPT NO RESPONSIBILITY OR LIABILITY FOR ANY CONSEQUENCES OF USING
IT, INCLUDING BUT NOT LIMITED TO ACCOUNT SUSPENSION OR TERMINATION, DATA LOSS,
OR DAMAGE TO YOUR COMPUTER. YOU ARE SOLELY RESPONSIBLE FOR COMPLYING WITH
DISCORD'S TERMS OF SERVICE AND ANY APPLICABLE LAWS.

Rustcord is not affiliated with, endorsed by or sponsored by Discord Inc.
"Discord" and the Discord logo are trademarks of Discord Inc. Rustcord does not
include or redistribute any of Discord's code or assets; it loads the public web
client from discord.com at runtime.

---

## Türkçe

### Nedir?

Rustcord, **resmi Discord web istemcisini** (`https://discord.com/app`) çalıştıran,
Rust ile yazılmış hafif bir Windows masaüstü uygulamasıdır. Electron sürümü gibi
kendi Chromium ve Node.js kopyasını taşımaz; Windows'ta zaten bulunan WebView2'yi
kullanır ve masaüstü özelliklerini Rust ile sağlar.

- Girişi **Discord'un kendi sayfasında** yaparsın (şifre, QR kod, 2FA, passkey).
  Rustcord şifreni ya da token'ını görmez, saklamaz, hiçbir yere göndermez.
- Arayüz Discord'un kendisidir.

Discord'un Electron'dan Rust'a geçiş duyurusundan ilham alınmıştır, ancak Discord'un
o uygulaması **değildir** ve onunla ilgisi yoktur.

### Özellikler

Çerçevesiz pencere, tepsi menüsü, Windows bildirimleri, görev çubuğu rozeti, oyun
etkinliği (Rich Presence ve oyun algılama), yayın sırasında çözünürlük/FPS seçimi
(Nitro kuralları Discord'unkiyle aynı), harici linklerin tarayıcıda açılması,
çevrimdışı ekranı ve tepsideyken düşük bellek kullanımı.

### Riskler

1. **Kullanım Şartları:** Discord, üçüncü taraf istemcilere ve istemci
   değişikliklerine izin vermez. Rustcord kullanan hesaplar **kısıtlanabilir veya
   kapatılabilir**. Bunun güvenli bir kullanım seviyesi yoktur.
2. **İstemci değişikliği:** Rich Presence ve yayın kalitesi menüsü için Rustcord,
   web istemcisinin iç koduna çalışma anında müdahale eder (Vencord ile aynı
   yöntem). Discord güncellemeleri bu özellikleri her an bozabilir.
3. **Yerel Rich Presence sunucusu:** Resmi uygulama gibi `\\.\pipe\discord-ipc-N`
   üzerinde dinler. Bilgisayarında senin kullanıcınla çalışan her program
   etkinliğini değiştirebilir ve herkese açık kullanıcı bilgilerini (id, kullanıcı
   adı, avatar) görebilir. Mesajlarını okuyamaz. Tepsi menüsünden kapatılabilir.
4. **Garanti yoktur:** Güvenlik denetiminden geçmemiş bağımsız bir hobi projesidir.
5. **İndirme:** Yalnızca kaynaktan derle ya da bu depodan indir. Discord şifreni
   Discord'un kendi sayfası dışında hiçbir yere girme.

### Sorumluluk reddi

Bu yazılım **"OLDUĞU GİBİ"**, hiçbir garanti olmadan sunulur. Yazarlar ve katkıda
bulunanlar; hesap kısıtlanması veya kapatılması, veri kaybı ya da bilgisayarına
gelebilecek zararlar dahil, kullanımından doğabilecek **hiçbir sonuçtan sorumluluk
kabul etmez**. Discord Kullanım Şartları'na ve yasalara uymak tamamen kullanıcının
sorumluluğundadır.

Rustcord, Discord Inc. ile bağlantılı değildir; Discord tarafından onaylanmamış
veya desteklenmemiştir. "Discord" ve Discord logosu Discord Inc.'in ticari
markalarıdır.

## License

[MIT](LICENSE)
