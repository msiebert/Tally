//! Item tools, end to end through the MCP server.

mod common;
use common::*;
use serde_json::json;

fn session(name: &str) -> Session {
    let mut s = Session::start(&temp_db(name));
    s.call("create_list", json!({"name": "inbox"})).json();
    s.call("create_list", json!({"name": "work"})).json();
    s
}

#[test]
fn add_update_complete_move_delete_workflow() {
    let mut s = session("items-workflow");
    s.call("create_tag", json!({"name": "bug"})).json();

    let added = s
        .call(
            "add_items",
            json!({"list": "inbox", "items": [
                {"title": "write tests", "notes": "cover **errors**", "tags": ["bug"]},
                {"title": "ship it", "blocked_by": [1]},
                {"title": "celebrate", "position": "top"},
            ]}),
        )
        .json();
    assert_eq!(added["created_ids"], json!([1, 2, 3]));
    assert_eq!(added["list"], "inbox");

    // an item blocked by an open item says so, and completing it warns
    let done = s.call("complete_item", json!({"item": 2})).json();
    assert_eq!(done["item"]["status"], "done");
    assert!(
        done["warnings"][0]
            .as_str()
            .unwrap()
            .contains("#1 'write tests'")
    );

    let updated = s
        .call(
            "update_item",
            json!({"item": 1, "status": "doing", "title": "write more tests", "tags": []}),
        )
        .json();
    assert_eq!(updated["item"]["status"], "doing");
    assert_eq!(updated["item"]["title"], "write more tests");
    assert!(updated["item"].get("tags").is_none());

    let moved = s
        .call("move_item", json!({"item": 1, "to_list": "work"}))
        .json();
    assert_eq!(moved["item"]["list"], "work");

    // item 1 still blocks item 2, so it can't be deleted
    let refused = s.call("delete_item", json!({"item": 1}));
    assert!(refused.error().contains("#2 'ship it'"), "{}", refused.text);
    assert!(refused.text.contains("remove_dependency"));

    // an item that blocks nothing can be deleted
    let deleted = s.call("delete_item", json!({"item": 3})).json();
    assert_eq!(deleted["deleted"]["title"], "celebrate");

    let lists = s.call("list_lists", json!({})).json();
    assert_eq!(lists["lists"][0]["total_items"], 1); // inbox: just "ship it"
    assert_eq!(lists["lists"][1]["total_items"], 1); // work: "write more tests"
}

#[test]
fn unknown_tags_are_rejected_and_nothing_is_added() {
    let mut s = session("items-unknown-tag");
    s.call("create_tag", json!({"name": "bug"})).json();
    let err = s.call(
        "add_items",
        json!({"list": "inbox", "items": [
            {"title": "fine", "tags": ["bug"]},
            {"title": "typo", "tags": ["bgu"]},
        ]}),
    );
    let text = err.error();
    assert!(text.starts_with("items[1]:"), "{text}");
    assert!(text.contains("Unknown tag 'bgu'"), "{text}");
    assert!(text.contains("create_tag"), "{text}");
    assert!(text.contains("'bug'"), "{text}");
    let lists = s.call("list_lists", json!({})).json();
    assert_eq!(lists["lists"][0]["total_items"], 0);

    s.call(
        "add_items",
        json!({"list": "inbox", "items": [{"title": "x"}]}),
    )
    .json();
    let err = s.call("update_item", json!({"item": 1, "tags": ["nope"]}));
    assert!(err.error().contains("create_tag"));
}

#[test]
fn notes_over_the_limit_are_rejected_with_a_clear_error() {
    let mut s = session("items-notes");
    let long = "x".repeat(4001);
    let err = s.call(
        "add_items",
        json!({"list": "inbox", "items": [{"title": "t", "notes": long}]}),
    );
    assert!(
        err.error().contains("limited to 4000 characters"),
        "{}",
        err.text
    );
    let exact = "x".repeat(4000);
    s.call(
        "add_items",
        json!({"list": "inbox", "items": [{"title": "t", "notes": exact}]}),
    )
    .json();
}

#[test]
fn agents_can_send_ids_as_text_and_positions_as_objects() {
    let mut s = session("items-lenient");
    s.call(
        "add_items",
        json!({"list": "inbox", "items": [{"title": "a"}, {"title": "b"}]}),
    )
    .json();
    // "#2" and "2" both mean item 2
    s.call("update_item", json!({"item": "#2", "status": "doing"}))
        .json();
    s.call("update_item", json!({"item": "2", "status": "todo"}))
        .json();
    let added = s
        .call(
            "add_items",
            json!({"list": "inbox", "items": [
                {"title": "after a", "position": {"after": "1"}, "blocked_by": ["#2"]},
            ]}),
        )
        .json();
    assert_eq!(added["created_ids"], json!([3]));
    // numeric list IDs work, and "after" must name an item in the destination list
    let moved = s
        .call(
            "move_item",
            json!({"item": 2, "to_list": 2, "position": "top"}),
        )
        .json();
    assert_eq!(moved["item"]["list"], "work");
    let err = s.call(
        "move_item",
        json!({"item": 3, "to_list": "work", "position": {"after": 1}}),
    );
    assert!(err.error().contains("isn't in list 'work'"), "{}", err.text);
    let reordered = s
        .call(
            "move_item",
            json!({"item": 3, "to_list": "inbox", "position": {"after": "#1"}}),
        )
        .json();
    assert_eq!(reordered["item"]["list"], "inbox");
}

#[test]
fn bad_input_is_a_readable_error() {
    let mut s = session("items-bad-input");
    let err = s.call("update_item", json!({"item": 1, "title": "x"}));
    assert!(err.error().contains("No item with id 1"), "{}", err.text);
    let err = s.call("update_item", json!({"item": 1}));
    assert!(err.error().contains("Nothing to update"), "{}", err.text);
    let err = s.call("update_item", json!({"item": "abc", "title": "x"}));
    assert!(err.error().contains("expected an item ID"), "{}", err.text);
    let err = s.call("update_item", json!({"item": 1, "status": "finished"}));
    assert!(
        err.error().contains("failed to deserialize"),
        "{}",
        err.text
    );
    let err = s.call("add_items", json!({"list": "inbox", "items": []}));
    assert!(err.error().contains("at least one item"), "{}", err.text);
    // the server is still healthy
    assert_eq!(s.call("list_lists", json!({})).json()["total_count"], 2);
}
