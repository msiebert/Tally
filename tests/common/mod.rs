//! Helpers for driving the real `tally` binary over stdio, one request at a time.

#![allow(dead_code)] // each test crate uses a different subset

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

pub struct Session {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    next_id: i64,
    pub init: Value,
}

/// What a tool call returned: the text the agent would see, and whether it was an error.
pub struct Outcome {
    pub is_error: bool,
    pub text: String,
}

impl Outcome {
    /// The text parsed as JSON. Panics if the call failed or the text is not JSON.
    pub fn json(&self) -> Value {
        assert!(!self.is_error, "tool returned an error: {}", self.text);
        serde_json::from_str(&self.text).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.text))
    }

    /// The error text. Panics if the call succeeded.
    pub fn error(&self) -> &str {
        assert!(self.is_error, "expected an error, got: {}", self.text);
        &self.text
    }
}

impl Session {
    pub fn start(db: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_tally"))
            .env("TALLY_DB", db)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn tally");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(l) => {
                        if tx.send(l).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        let mut session = Session {
            child,
            stdin,
            lines,
            next_id: 0,
            init: Value::Null,
        };
        session.init = session.request(
            "initialize",
            json!({
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "0"}
            }),
        );
        session.send(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        session
    }

    fn send(&mut self, message: &Value) {
        writeln!(self.stdin, "{message}").expect("write to tally");
        self.stdin.flush().unwrap();
    }

    /// Send a request and return its `result`, panicking on a protocol error or timeout.
    pub fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
        loop {
            let line = self
                .lines
                .recv_timeout(Duration::from_secs(20))
                .unwrap_or_else(|_| panic!("timed out waiting for response to {method}"));
            if line.trim().is_empty() {
                continue;
            }
            let message: Value = serde_json::from_str(&line).expect("valid JSON-RPC line");
            if message["id"] == id {
                assert!(message.get("error").is_none(), "protocol error: {message}");
                return message["result"].clone();
            }
        }
    }

    pub fn tool_names(&mut self) -> Vec<String> {
        let result = self.request("tools/list", json!({}));
        let mut names: Vec<String> = result["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        names.sort();
        names
    }

    pub fn call(&mut self, tool: &str, arguments: Value) -> Outcome {
        let result = self.request("tools/call", json!({"name": tool, "arguments": arguments}));
        Outcome {
            is_error: result["isError"].as_bool().unwrap_or(false),
            text: result["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn temp_db(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tally-it-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("tally.db")
}
