//! get_list and search_items, end to end through the MCP server.

mod common;
use common::*;
use serde_json::{Value, json};

fn session(name: &str) -> Session {
    let mut s = Session::start(&temp_db(name));
    s.call("create_list", json!({"name": "inbox"})).json();
    s.call("create_list", json!({"name": "work"})).json();
    s.call("create_tag", json!({"name": "bug"})).json();
    s
}

fn titles(v: &Value) -> Vec<String> {
    v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["title"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn get_list_shows_order_status_filters_and_blocked_flags() {
    let mut s = session("reads-get-list");
    s.call(
        "add_items",
        json!({"list": "inbox", "items": [
            {"title": "first", "tags": ["bug"], "notes": "details"},
            {"title": "second"},
            {"title": "third", "position": "top"},
        ]}),
    )
    .json();
    s.call(
        "add_items",
        json!({"list": "work", "items": [{"title": "blocker"}]}),
    )
    .json();
    s.call("add_dependency", json!({"item": 2, "blocked_by": 4}))
        .json();
    s.call("complete_item", json!({"item": 1})).json();

    let v = s.call("get_list", json!({"list": "inbox"})).json();
    assert_eq!(titles(&v), ["third", "second"], "top first, done hidden");
    assert_eq!(v["total_count"], 2);
    assert_eq!(v["list"]["name"], "inbox");
    assert_eq!(v["items"][1]["blocked"], true);
    assert_eq!(v["items"][1]["open_blockers"][0]["title"], "blocker");

    let v = s
        .call("get_list", json!({"list": "inbox", "status": "done"}))
        .json();
    assert_eq!(titles(&v), ["first"]);
    assert_eq!(v["items"][0]["notes"], "details");
    assert_eq!(v["items"][0]["tags"], json!(["bug"]));

    // a single status or a list of statuses both work
    let v = s
        .call(
            "get_list",
            json!({"list": "inbox", "status": ["todo", "done"]}),
        )
        .json();
    assert_eq!(titles(&v), ["third", "first", "second"]);

    let v = s
        .call("get_list", json!({"list": "inbox", "unblocked_only": true}))
        .json();
    assert_eq!(titles(&v), ["third"]);

    let v = s
        .call(
            "get_list",
            json!({"list": "inbox", "tag": "bug", "status": "done"}),
        )
        .json();
    assert_eq!(titles(&v), ["first"]);
}

#[test]
fn get_list_paginates_through_a_whole_list() {
    let mut s = session("reads-pages");
    let items: Vec<Value> = (1..=7).map(|i| json!({"title": format!("i{i}")})).collect();
    s.call("add_items", json!({"list": "inbox", "items": items}))
        .json();

    let mut seen = Vec::new();
    let mut cursor = Value::Null;
    let mut pages = 0;
    loop {
        let mut args = json!({"list": "inbox", "limit": 3});
        if !cursor.is_null() {
            args["cursor"] = cursor.clone();
        }
        let page = s.call("get_list", args).json();
        assert_eq!(page["total_count"], 7);
        seen.extend(titles(&page));
        pages += 1;
        cursor = page["next_cursor"].clone();
        if cursor.is_null() {
            break;
        }
        // edits between pages don't disturb the walk
        s.call("update_item", json!({"item": 1, "title": "renamed"}))
            .json();
    }
    assert_eq!(pages, 3);
    assert_eq!(seen, ["i1", "i2", "i3", "i4", "i5", "i6", "i7"]);
}

#[test]
fn search_items_finds_across_lists() {
    let mut s = session("reads-search");
    s.call(
        "add_items",
        json!({"list": "inbox", "items": [{"title": "Buy milk"}, {"title": "call mum"}]}),
    )
    .json();
    s.call(
        "add_items",
        json!({"list": "work", "items": [
            {"title": "report", "notes": "includes a MILK-run section", "tags": ["bug"]},
        ]}),
    )
    .json();

    let v = s.call("search_items", json!({"text": "milk"})).json();
    assert_eq!(v["total_count"], 2);
    // search results say which list each item is in, newest update first
    assert_eq!(v["items"][0]["list"], "work");
    assert_eq!(v["items"][1]["list"], "inbox");

    let v = s
        .call("search_items", json!({"text": "milk", "list": "inbox"}))
        .json();
    assert_eq!(titles(&v), ["Buy milk"]);

    let v = s.call("search_items", json!({"tag": "bug"})).json();
    assert_eq!(titles(&v), ["report"]);

    s.call("complete_item", json!({"item": 1})).json();
    assert_eq!(
        s.call("search_items", json!({"text": "milk"})).json()["total_count"],
        2,
        "done items are still found"
    );
    let v = s
        .call("search_items", json!({"status": ["todo", "doing"]}))
        .json();
    assert_eq!(v["total_count"], 2);
}

#[test]
fn read_errors_are_readable() {
    let mut s = session("reads-errors");
    let err = s.call("get_list", json!({"list": "nope"}));
    assert!(
        err.error().contains("Existing lists: 'inbox', 'work'"),
        "{}",
        err.text
    );
    let err = s.call("get_list", json!({"list": "inbox", "tag": "bgu"}));
    assert!(err.error().contains("'bug'"), "{}", err.text);
    let err = s.call("get_list", json!({"list": "inbox", "cursor": "junk"}));
    assert!(err.error().contains("Invalid cursor"), "{}", err.text);
    let err = s.call("search_items", json!({"status": "finished"}));
    assert!(
        err.error().contains("failed to deserialize"),
        "{}",
        err.text
    );
}
