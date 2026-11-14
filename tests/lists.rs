//! List tools, end to end through the MCP server.

mod common;
use common::*;
use serde_json::json;

#[test]
fn list_lifecycle() {
    let mut s = Session::start(&temp_db("list-lifecycle"));

    let created = s
        .call(
            "create_list",
            json!({"name": "Groceries", "instructions": "Group by aisle"}),
        )
        .json();
    assert_eq!(created["id"], 1);
    assert_eq!(created["has_instructions"], true);

    let err = s.call("create_list", json!({"name": "groceries"}));
    assert!(err.error().contains("already exists"));

    let renamed = s
        .call(
            "update_list",
            json!({"list": "groceries", "name": "Shopping"}),
        )
        .json();
    assert_eq!(renamed["name"], "Shopping");

    // refer to the list by ID too
    let cleared = s
        .call("update_list", json!({"list": 1, "instructions": ""}))
        .json();
    assert_eq!(cleared["has_instructions"], false);

    let all = s.call("list_lists", json!({})).json();
    assert_eq!(all["total_count"], 1);
    assert_eq!(all["lists"][0]["name"], "Shopping");
    assert!(all["next_cursor"].is_null());

    let deleted = s.call("delete_list", json!({"list": "Shopping"})).json();
    assert_eq!(deleted["deleted"]["name"], "Shopping");
    assert_eq!(s.call("list_lists", json!({})).json()["total_count"], 0);
}

#[test]
fn errors_are_tool_errors_that_say_what_to_do() {
    let mut s = Session::start(&temp_db("list-errors"));
    let err = s.call("delete_list", json!({"list": "ghost"}));
    assert!(err.error().contains("create_list"), "{}", err.text);

    // a malformed call is a readable tool error, and the server keeps working afterwards
    let bad = s.call("create_list", json!({}));
    assert!(bad.error().contains("missing field `name`"), "{}", bad.text);
    assert_eq!(s.call("list_lists", json!({})).json()["total_count"], 0);
}

#[test]
fn list_lists_paginates() {
    let mut s = Session::start(&temp_db("list-pages"));
    for i in 1..=5 {
        s.call("create_list", json!({"name": format!("list{i}")}))
            .json();
    }
    let mut seen = Vec::new();
    let mut cursor = serde_json::Value::Null;
    loop {
        let args = if cursor.is_null() {
            json!({"limit": 2})
        } else {
            json!({"limit": 2, "cursor": cursor})
        };
        let page = s.call("list_lists", args).json();
        for l in page["lists"].as_array().unwrap() {
            seen.push(l["name"].as_str().unwrap().to_string());
        }
        cursor = page["next_cursor"].clone();
        if cursor.is_null() {
            break;
        }
    }
    assert_eq!(seen, ["list1", "list2", "list3", "list4", "list5"]);
}
