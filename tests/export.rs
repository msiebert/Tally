//! `tally export` against a database built through the real MCP server.

mod common;
use common::*;
use serde_json::{Value, json};
use std::process::Command;

fn build(name: &str) -> std::path::PathBuf {
    let db = temp_db(name);
    let mut s = Session::start(&db);
    s.call(
        "create_list",
        json!({"name": "launch", "instructions": "Verbs first."}),
    )
    .json();
    s.call("create_tag", json!({"name": "critical"})).json();
    s.call(
        "add_items",
        json!({"list": "launch", "items": [
            {"title": "Cut branch", "tags": ["critical"], "notes": "a\nb"},
            {"title": "Publish", "blocked_by": [1]},
        ]}),
    )
    .json();
    drop(s);
    db
}

fn tally(db: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_tally"))
        .args(args)
        .env("TALLY_DB", db)
        .output()
        .unwrap()
}

#[test]
fn json_export_to_stdout() {
    let db = build("export-json");
    let out = tally(&db, &["export"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["format"], "tally-export");
    assert_eq!(v["lists"][0]["items"][1]["blocked_by"], json!([1]));
    assert_eq!(v["lists"][0]["items"][0]["tags"], json!(["critical"]));
    assert_eq!(v["dependencies"], json!([{"item": 2, "blocked_by": 1}]));
}

#[test]
fn markdown_export_to_a_new_file_and_refuses_to_overwrite() {
    let db = build("export-md");
    let target = db.with_file_name("backup.md");
    let _ = std::fs::remove_file(&target);
    let out = tally(
        &db,
        &[
            "export",
            "--format",
            "markdown",
            "--output",
            target.to_str().unwrap(),
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let md = std::fs::read_to_string(&target).unwrap();
    assert!(md.contains("## launch"), "{md}");
    assert!(md.contains("blocked by: #1 Cut branch"), "{md}");

    let again = tally(
        &db,
        &["export", "-f", "markdown", "-o", target.to_str().unwrap()],
    );
    assert!(!again.status.success());
    assert!(String::from_utf8_lossy(&again.stderr).contains("already exist"));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), md);
}

#[test]
fn missing_database_and_bad_arguments_fail_clearly() {
    let db = temp_db("export-missing");
    let _ = std::fs::remove_file(&db);
    let out = tally(&db, &["export"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no database"));

    let out = tally(&db, &["export", "--format", "xml"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("USAGE"));
}

#[test]
fn help_prints_usage() {
    let out = tally(&temp_db("export-help"), &["--help"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("tally export"));
}
