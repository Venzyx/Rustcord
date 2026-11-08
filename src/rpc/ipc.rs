//! Discord IPC transport (`\\.\pipe\discord-ipc-N`), the protocol used by the
//! Discord Game SDK, discord-rpc and most Rich Presence libraries.
//!
//! Frame: little-endian `u32 opcode`, `u32 length`, then `length` bytes of JSON.

use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Manager, Runtime};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const OP_CLOSE: u32 = 2;
const OP_PING: u32 = 3;
const OP_PONG: u32 = 4;
const MAX_FRAME: usize = 64 * 1024;

static NEXT_SOCKET: AtomicU64 = AtomicU64::new(1);

fn pipe_name(i: u8) -> String {
    format!(r"\\.\pipe\discord-ipc-{i}")
}

/// Claims the first free `discord-ipc-N`. If the official Discord app is
/// running it already owns `-0`, and clients will keep talking to it.
fn bind_first_free() -> Option<(String, NamedPipeServer)> {
    (0..10u8).find_map(|i| {
        let name = pipe_name(i);
        ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create(&name)
            .ok()
            .map(|server| (name, server))
    })
}

pub fn start<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        let Some((name, mut server)) = bind_first_free() else {
            eprintln!("rustcord: no free discord-ipc pipe");
            return;
        };
        loop {
            if server.connect().await.is_err() {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                continue;
            }
            let connected = server;
            server = match ServerOptions::new().reject_remote_clients(true).create(&name) {
                Ok(s) => s,
                Err(_) => {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    match ServerOptions::new().reject_remote_clients(true).create(&name) {
                        Ok(s) => s,
                        Err(e) => {
                            eprintln!("rustcord: pipe re-create failed: {e}");
                            return;
                        }
                    }
                }
            };
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let socket_id = format!("rustcord-ipc-{}", NEXT_SOCKET.fetch_add(1, Ordering::Relaxed));
                let mut had_activity = false;
                let _ = serve(&app, connected, &socket_id, &mut had_activity).await;
                if had_activity {
                    super::publish(&app, &socket_id, None, None, true);
                }
            });
        }
    });
}

async fn read_frame(pipe: &mut NamedPipeServer) -> std::io::Result<(u32, Value)> {
    let op = pipe.read_u32_le().await?;
    let len = pipe.read_u32_le().await? as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut buf = vec![0u8; len];
    pipe.read_exact(&mut buf).await?;
    let value = serde_json::from_slice(&buf).unwrap_or(Value::Null);
    Ok((op, value))
}

async fn write_frame(pipe: &mut NamedPipeServer, op: u32, value: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(value)?;
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&op.to_le_bytes());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    pipe.write_all(&out).await?;
    pipe.flush().await
}

fn is_snowflake(s: &str) -> bool {
    (15..=25).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())
}

fn ready_payload<R: Runtime>(app: &AppHandle<R>) -> Value {
    let user = app.state::<super::Presence>().user();
    let user = match user {
        Some(u) => json!({
            "id": u.id,
            "username": u.username,
            "global_name": u.global_name,
            "discriminator": u.discriminator.unwrap_or_else(|| "0".into()),
            "avatar": u.avatar,
            "bot": false,
            "flags": 0,
            "premium_type": 0,
        }),
        None => json!({ "id": "0", "username": "rustcord", "discriminator": "0", "avatar": null, "bot": false, "flags": 0, "premium_type": 0 }),
    };
    json!({
        "cmd": "DISPATCH",
        "evt": "READY",
        "nonce": null,
        "data": {
            "v": 1,
            "config": {
                "cdn_host": "cdn.discordapp.com",
                "api_endpoint": "//discord.com/api",
                "environment": "production"
            },
            "user": user
        }
    })
}

fn error(cmd: &Value, nonce: &Value, code: u32, message: &str) -> Value {
    json!({ "cmd": cmd, "evt": "ERROR", "nonce": nonce, "data": { "code": code, "message": message } })
}

async fn serve<R: Runtime>(app: &AppHandle<R>, mut pipe: NamedPipeServer, socket_id: &str, had_activity: &mut bool) -> std::io::Result<()> {
    // Handshake: {"v": 1, "client_id": "<application id>"}
    let (op, hello) = read_frame(&mut pipe).await?;
    let client_id = hello.get("client_id").and_then(Value::as_str).unwrap_or_default().to_owned();
    if op != OP_HANDSHAKE || hello.get("v").and_then(Value::as_u64) != Some(1) || !is_snowflake(&client_id) {
        let _ = write_frame(&mut pipe, OP_CLOSE, &json!({ "code": 4000, "message": "Invalid handshake" })).await;
        return Ok(());
    }
    write_frame(&mut pipe, OP_FRAME, &ready_payload(app)).await?;

    loop {
        let (op, msg) = read_frame(&mut pipe).await?;
        match op {
            OP_PING => write_frame(&mut pipe, OP_PONG, &msg).await?,
            OP_CLOSE => return Ok(()),
            OP_FRAME => {
                let cmd = msg.get("cmd").cloned().unwrap_or(Value::Null);
                let nonce = msg.get("nonce").cloned().unwrap_or(Value::Null);
                let args = msg.get("args").cloned().unwrap_or(Value::Null);
                let reply = match cmd.as_str().unwrap_or_default() {
                    "SET_ACTIVITY" => {
                        let pid = args.get("pid").and_then(Value::as_u64).map(|p| p as u32);
                        match args.get("activity") {
                            Some(Value::Object(obj)) => {
                                let mut activity = obj.clone();
                                // A client can only speak for its own application.
                                activity.insert("application_id".into(), Value::String(client_id.clone()));
                                let activity = Value::Object(activity);
                                *had_activity = true;
                                super::publish(app, socket_id, pid, Some(activity.clone()), true);
                                json!({ "cmd": cmd, "evt": null, "nonce": nonce, "data": activity })
                            }
                            _ => {
                                *had_activity = false;
                                super::publish(app, socket_id, pid, None, true);
                                json!({ "cmd": cmd, "evt": null, "nonce": nonce, "data": null })
                            }
                        }
                    }
                    "SUBSCRIBE" | "UNSUBSCRIBE" => {
                        json!({ "cmd": cmd, "evt": null, "nonce": nonce, "data": { "evt": args.get("evt").cloned().unwrap_or(Value::Null) } })
                    }
                    "AUTHORIZE" | "AUTHENTICATE" => error(&cmd, &nonce, 4006, "OAuth2 is not supported by Rustcord"),
                    _ => error(&cmd, &nonce, 4000, "Unsupported command"),
                };
                write_frame(&mut pipe, OP_FRAME, &reply).await?;
            }
            _ => {
                let _ = write_frame(&mut pipe, OP_CLOSE, &json!({ "code": 4000, "message": "Invalid opcode" })).await;
                return Ok(());
            }
        }
    }
}
