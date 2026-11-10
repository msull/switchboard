//! The MCP stdio transport: one JSON-RPC object per line in, one per
//! line out, nothing but protocol on the output (logs go to stderr),
//! and the loop ends when the input closes. Both handshakes are
//! answered: a legacy client's `initialize` and a current client's
//! `server/discover`. Each `tools/call` runs on its own thread, so a
//! long `wait` never holds up a `ping`, and `notifications/cancelled`
//! stops the call it names.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};

use crate::events::Hub;
use crate::tools;

/// The protocol versions this server speaks, newest first.
pub const PROTOCOL_VERSIONS: &[&str] = &["2026-07-28", "2025-06-18", "2025-03-26", "2024-11-05"];

/// JSON-RPC error codes.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// What every call shares: the config directory, the event feeds, the
/// output, and the cancel flags of calls in flight.
pub struct Server {
    pub dir: PathBuf,
    pub hub: Hub,
    out: Mutex<Box<dyn Write + Send>>,
    cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Each host's `/info`, read once per process.
    pub info: Mutex<HashMap<String, Value>>,
}

impl Server {
    #[must_use]
    pub fn new(dir: PathBuf, out: Box<dyn Write + Send>) -> Arc<Self> {
        Arc::new(Self {
            dir,
            hub: Hub::default(),
            out: Mutex::new(out),
            cancels: Mutex::new(HashMap::new()),
            info: Mutex::new(HashMap::new()),
        })
    }

    /// Write one message as one line.
    pub fn send(&self, message: &Value) {
        let mut out = self.out.lock().unwrap_or_else(PoisonError::into_inner);
        let mut line = message.to_string();
        line.push('\n');
        if out
            .write_all(line.as_bytes())
            .and_then(|()| out.flush())
            .is_err()
        {
            eprintln!("switchboard-mcp: the client stopped reading");
        }
    }

    fn reply(&self, id: &Value, result: &Value) {
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "result": result }));
    }

    fn error(&self, id: &Value, code: i64, message: &str) {
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message },
        }));
    }

    /// Read lines until the input closes, answering each. Calls still
    /// running when it closes are cancelled.
    pub fn run(self: &Arc<Self>, input: impl BufRead) {
        let mut calls = Vec::new();
        for line in input.lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            if let Some(handle) = self.handle(&line) {
                calls.push(handle);
            }
            calls.retain(|h: &std::thread::JoinHandle<()>| !h.is_finished());
        }
        for flag in self
            .cancels
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
        {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        for h in calls {
            let _ = h.join();
        }
    }

    /// One message; the thread of a tool call it started.
    fn handle(self: &Arc<Self>, line: &str) -> Option<std::thread::JoinHandle<()>> {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            self.error(&Value::Null, PARSE_ERROR, "not JSON");
            return None;
        };
        let method = message["method"].as_str().unwrap_or_default();
        let id = message.get("id").cloned();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = id else {
            if method == "notifications/cancelled"
                && let Some(flag) = self
                    .cancels
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(&params["requestId"].to_string())
            {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            return None;
        };
        match method {
            "initialize" => {
                let asked = params["protocolVersion"].as_str().unwrap_or_default();
                let version = PROTOCOL_VERSIONS
                    .iter()
                    .find(|v| **v == asked)
                    .unwrap_or(&PROTOCOL_VERSIONS[1]);
                self.reply(
                    &id,
                    &json!({
                        "protocolVersion": version,
                        "capabilities": { "tools": {} },
                        "serverInfo": server_info(),
                        "instructions": INSTRUCTIONS,
                    }),
                );
            }
            "server/discover" => self.reply(
                &id,
                &json!({
                    "supportedVersions": PROTOCOL_VERSIONS,
                    "protocolVersions": PROTOCOL_VERSIONS,
                    "capabilities": { "tools": {} },
                    "serverInfo": server_info(),
                    "instructions": INSTRUCTIONS,
                }),
            ),
            "ping" => self.reply(&id, &json!({})),
            "tools/list" => self.reply(&id, &json!({ "tools": tools::list() })),
            "tools/call" => return self.call(id, &params),
            "" => self.error(&id, INVALID_REQUEST, "no method"),
            _ => self.error(&id, METHOD_NOT_FOUND, &format!("no method {method}")),
        }
        None
    }

    fn call(self: &Arc<Self>, id: Value, params: &Value) -> Option<std::thread::JoinHandle<()>> {
        let name = params["name"].as_str().unwrap_or_default().to_owned();
        if !tools::known(&name) {
            self.error(&id, INVALID_PARAMS, &format!("Unknown tool: {name}"));
            return None;
        }
        let args = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let progress = params["_meta"]["progressToken"].clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let key = id.to_string();
        self.cancels
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.clone(), Arc::clone(&cancel));
        let server = Arc::clone(self);
        Some(std::thread::spawn(move || {
            let call = tools::Call {
                server: &server,
                cancel: &cancel,
                progress: (!progress.is_null()).then_some(progress),
            };
            let result = tools::run(&call, &name, &args);
            server
                .cancels
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&key);
            // A cancelled call is answered with nothing, as the client
            // has stopped waiting for it.
            if cancel.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            let (text, is_error) = match result {
                Ok(value) => (
                    match value {
                        Value::String(s) => s,
                        v => serde_json::to_string_pretty(&v).unwrap_or_default(),
                    },
                    false,
                ),
                Err(e) => (e, true),
            };
            server.reply(
                &id,
                &json!({
                    "content": [{ "type": "text", "text": text }],
                    "isError": is_error,
                }),
            );
        }))
    }
}

fn server_info() -> Value {
    json!({ "name": "switchboard-mcp", "version": env!("CARGO_PKG_VERSION") })
}

/// What the client is told the server is for.
const INSTRUCTIONS: &str = "Watch and drive paired Switchboard hosts: their projects, sessions, \
    Dispatch tickets and supervisors. Every tool takes `host` (see `hosts`). `ask_supervisor` \
    messages a project's supervisor through the owner channel; it is headed as yours, not the \
    owner's. `resume` spends a paid turn: set `confirm` only when the owner said to resume that \
    session. `run` starts only commands the owner made or approved.";
