//! End-to-end: start the real binary and talk MCP to it over stdio.

mod common;
use common::*;
use serde_json::json;

#[test]
fn server_initializes_and_lists_tools() {
    let db = temp_db("smoke");
    let responses = run_session(
        &db,
        &[json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})],
    );

    let init = response_for(&responses, 0);
    assert_eq!(init["result"]["serverInfo"]["name"], "tally");
    assert!(init["result"]["capabilities"]["tools"].is_object());
    assert!(init["result"]["instructions"].is_string());

    let tools = response_for(&responses, 1);
    assert!(tools["result"]["tools"].is_array(), "got {tools}");

    // The database file is created and migrated at startup.
    assert!(db.exists());
}
