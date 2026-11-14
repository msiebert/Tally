//! End-to-end: start the real binary and talk MCP to it over stdio.

mod common;
use common::*;

#[test]
fn server_initializes_and_lists_tools() {
    let db = temp_db("smoke");
    let mut s = Session::start(&db);

    assert_eq!(s.init["serverInfo"]["name"], "tally");
    assert!(s.init["capabilities"]["tools"].is_object());
    assert!(s.init["instructions"].is_string());
    assert_eq!(
        s.tool_names(),
        [
            "create_list",
            "create_tag",
            "delete_list",
            "delete_tag",
            "list_lists",
            "list_tags",
            "update_list",
            "update_tag"
        ]
    );

    // The database file is created and migrated at startup.
    assert!(db.exists());
}

#[test]
fn data_survives_a_restart() {
    let db = temp_db("restart");
    {
        let mut s = Session::start(&db);
        s.call("create_list", serde_json::json!({"name": "Persistent"}))
            .json();
    }
    let mut s = Session::start(&db);
    let lists = s.call("list_lists", serde_json::json!({})).json();
    assert_eq!(lists["lists"][0]["name"], "Persistent");
}
