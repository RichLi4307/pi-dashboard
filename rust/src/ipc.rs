//! Unix socket IPC server.
//!
//! Mirrors `pi_dashboard/ipc_server.py`. Supports screenshots, status,
//! switch_mode and scroll_containers. Stateful commands are forwarded to the
//! main loop via a control channel and answered with a oneshot.

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufStream};
use tokio::net::UnixListener;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, warn};

use crate::config::SOCKET_PATH;
use crate::fb::Framebuffer;
use crate::metrics::Metrics;
use crate::screenshot::screenshot_base64;

#[derive(Debug)]
pub enum IpcCommand {
    SwitchMode(&'static str),
    ScrollContainers(oneshot::Sender<Result<(usize, usize), String>>),
}

#[derive(Debug, Deserialize)]
struct Request {
    action: String,
    #[serde(flatten)]
    extra: HashMap<String, Value>,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    status: &'static str,
    message: String,
}

fn error(message: String) -> Value {
    json!({"status": "error", "message": message})
}

fn ok(data: Value) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("status".to_string(), json!("ok"));
    if let Value::Object(obj) = data {
        for (k, v) in obj {
            map.insert(k, v);
        }
    }
    Value::Object(map)
}

pub struct IpcServer {
    fb: Arc<Mutex<Framebuffer>>,
    metrics: Arc<Metrics>,
    control_tx: mpsc::Sender<IpcCommand>,
}

impl IpcServer {
    pub fn new(
        fb: Arc<Mutex<Framebuffer>>,
        metrics: Arc<Metrics>,
        control_tx: mpsc::Sender<IpcCommand>,
    ) -> Self {
        Self {
            fb,
            metrics,
            control_tx,
        }
    }

    pub fn start(self) {
        tokio::spawn(self.serve());
    }

    async fn serve(self) {
        if let Err(e) = ensure_socket_dir() {
            warn!("IPC socket directory unavailable: {}", e);
            return;
        }
        let _ = fs::remove_file(SOCKET_PATH);

        let listener = match UnixListener::bind(SOCKET_PATH) {
            Ok(l) => l,
            Err(e) => {
                warn!("Failed to bind IPC socket: {}", e);
                return;
            }
        };
        if let Err(e) = fs::set_permissions(SOCKET_PATH, std::fs::Permissions::from_mode(0o666)) {
            warn!("Failed to chmod IPC socket: {}", e);
        }
        debug!("IPC server listening on {}", SOCKET_PATH);

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let mut stream = BufStream::new(stream);
                    let mut line = String::new();
                    match stream.read_line(&mut line).await {
                        Ok(0) => continue,
                        Ok(_) => {
                            let response = self.handle_request(&line).await;
                            let _ = stream.write_all(response.to_string().as_bytes()).await;
                            let _ = stream.write_all(b"\n").await;
                            let _ = stream.flush().await;
                        }
                        Err(e) => {
                            warn!("IPC read failed: {}", e);
                        }
                    }
                }
                Err(e) => {
                    warn!("IPC accept failed: {}", e);
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
        }
    }

    async fn handle_request(&self, line: &str) -> Value {
        let req: Request = match serde_json::from_str(line.trim()) {
            Ok(r) => r,
            Err(e) => return error(format!("invalid json: {e}")),
        };

        match req.action.as_str() {
            "screenshot" => self.handle_screenshot().await,
            "status" => self.handle_status(),
            "switch_mode" => self.handle_switch_mode(&req.extra).await,
            "scroll_containers" => self.handle_scroll_containers().await,
            _ => error(format!("unknown action: {}", req.action)),
        }
    }

    async fn handle_screenshot(&self) -> Value {
        let encoded = {
            let fb = match self.fb.lock() {
                Ok(g) => g,
                Err(_) => return error("framebuffer lock poisoned".to_string()),
            };
            screenshot_base64(&*fb)
        };
        match encoded {
            Some(data) => ok(json!({"data": data})),
            None => error("screenshot encoding failed".to_string()),
        }
    }

    fn handle_status(&self) -> Value {
        let snap = self.metrics.snapshot();
        ok(json!({
            "ips": snap.ips,
            "tailscale": snap.tailscale,
        }))
    }

    async fn handle_switch_mode(&self, extra: &HashMap<String, Value>) -> Value {
        let mode = extra
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("monitor");
        if mode != "monitor" {
            return error(format!("unknown mode: {mode}"));
        }
        if let Err(e) = self.control_tx.send(IpcCommand::SwitchMode("monitor")).await {
            return error(format!("control channel closed: {e}"));
        }
        ok(json!({"mode": mode}))
    }

    async fn handle_scroll_containers(&self) -> Value {
        let (tx, rx) = oneshot::channel();
        if let Err(e) = self.control_tx.send(IpcCommand::ScrollContainers(tx)).await {
            return error(format!("control channel closed: {e}"));
        }
        match rx.await {
            Ok(Ok((offset, total))) => ok(json!({"offset": offset, "total": total})),
            Ok(Err(msg)) => error(msg),
            Err(_) => error("scroll request dropped".to_string()),
        }
    }
}

fn ensure_socket_dir() -> std::io::Result<()> {
    let dir = Path::new(SOCKET_PATH).parent().unwrap_or(Path::new("/"));
    if !dir.exists() {
        fs::create_dir_all(dir)?;
    }
    Ok(())
}
