//! Helpers for driving the real `tally` binary over stdio.

use serde_json::{Value, json};
use std::io::Write;
use std::process::{Command, Stdio};

/// Run the server with the given JSON-RPC requests, close stdin, and return
/// every response (one per request that has an `id`), keyed by that id.
pub fn run_session(db_path: &std::path::Path, requests: &[Value]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_tally"))
        .env("TALLY_DB", db_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn tally");
    {
        let mut stdin = child.stdin.take().unwrap();
        let init = json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{
            "protocolVersion":"2025-03-26","capabilities":{},
            "clientInfo":{"name":"test","version":"0"}}});
        writeln!(stdin, "{init}").unwrap();
        let initialized = json!({"jsonrpc":"2.0","method":"notifications/initialized"});
        writeln!(stdin, "{initialized}").unwrap();
        for r in requests {
            writeln!(stdin, "{r}").unwrap();
        }
        // dropping stdin closes it, which ends the server
    }
    let out = child.wait_with_output().expect("wait for tally");
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("valid JSON-RPC line"))
        .collect()
}

pub fn response_for(responses: &[Value], id: i64) -> &Value {
    responses
        .iter()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("no response for id {id}: {responses:?}"))
}

pub fn temp_db(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("tally-it-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("tally.db")
}
