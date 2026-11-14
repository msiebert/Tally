//! Manual ordering, end to end through the MCP server.

mod common;
use common::*;
use serde_json::{Value, json};

fn order(s: &mut Session, list: &str) -> Vec<String> {
    let v = s
        .call("get_list", json!({"list": list, "limit": 200}))
        .json();
    v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["title"].as_str().unwrap().to_string())
        .collect()
}

fn session(name: &str) -> Session {
    let mut s = Session::start(&temp_db(name));
    s.call("create_list", json!({"name": "todo"})).json();
    s.call(
        "add_items",
        json!({"list": "todo", "items": [
            {"title": "a"}, {"title": "b"}, {"title": "c"}, {"title": "d"},
        ]}),
    )
    .json();
    s
}

#[test]
fn reorder_item_changes_what_get_list_shows() {
    let mut s = session("order-basic");
    s.call("reorder_item", json!({"item": 4, "to": "top"}))
        .json();
    assert_eq!(order(&mut s, "todo"), ["d", "a", "b", "c"]);
    s.call("reorder_item", json!({"item": 4, "after_item": 2}))
        .json();
    assert_eq!(order(&mut s, "todo"), ["a", "b", "d", "c"]);
    s.call("reorder_item", json!({"item": "#1", "to": "bottom"}))
        .json();
    assert_eq!(order(&mut s, "todo"), ["b", "d", "c", "a"]);
}

#[test]
fn reorder_errors_say_what_to_pass() {
    let mut s = session("order-errors");
    let err = s.call("reorder_item", json!({"item": 1}));
    assert!(
        err.error().contains("exactly one of after_item"),
        "{}",
        err.text
    );
    let err = s.call(
        "reorder_item",
        json!({"item": 1, "after_item": 2, "to": "top"}),
    );
    assert!(
        err.error().contains("exactly one of after_item"),
        "{}",
        err.text
    );
    let err = s.call("reorder_item", json!({"item": 1, "to": "middle"}));
    assert!(
        err.error().contains("failed to deserialize"),
        "{}",
        err.text
    );
    assert_eq!(order(&mut s, "todo"), ["a", "b", "c", "d"]);
}

#[test]
fn order_survives_many_inserts_at_the_same_spot() {
    // Repeatedly inserting right after item 1 uses up the space between
    // neighbours, which forces the list to be renumbered. Order must hold.
    let mut s = session("order-crowded");
    let mut expected: Vec<String> = Vec::new();
    for i in 0..30 {
        let title = format!("x{i}");
        s.call(
            "add_items",
            json!({"list": "todo", "items": [
                {"title": title, "position": {"after": 1}},
            ]}),
        )
        .json();
        expected.insert(0, title);
    }
    let mut want: Vec<String> = vec!["a".into()];
    want.extend(expected);
    want.extend(["b", "c", "d"].map(String::from));
    assert_eq!(order(&mut s, "todo"), want);
}

#[test]
fn order_is_kept_across_a_restart() {
    let db = temp_db("order-restart");
    {
        let mut s = Session::start(&db);
        s.call("create_list", json!({"name": "todo"})).json();
        let items: Vec<Value> = ["a", "b", "c"]
            .iter()
            .map(|t| json!({"title": t}))
            .collect();
        s.call("add_items", json!({"list": "todo", "items": items}))
            .json();
        s.call("reorder_item", json!({"item": 3, "to": "top"}))
            .json();
    }
    let mut s = Session::start(&db);
    assert_eq!(order(&mut s, "todo"), ["c", "a", "b"]);
}
