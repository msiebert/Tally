//! The error behaviors agents rely on, end to end: delete refusals, cycle
//! rejection, unknown tags, and the notes size limit. Each error must say what
//! is in the way, and the suggested fix must actually work.

mod common;
use common::*;
use serde_json::json;

fn session(name: &str) -> Session {
    let mut s = Session::start(&temp_db(name));
    s.call("create_list", json!({"name": "ops"})).json();
    s.call("create_list", json!({"name": "launch"})).json();
    s.call(
        "add_items",
        json!({"list": "ops", "items": [{"title": "Deploy"}]}),
    )
    .json();
    s.call(
        "add_items",
        json!({"list": "launch", "items": [{"title": "Release", "blocked_by": [1]}]}),
    )
    .json();
    s
}

#[test]
fn delete_item_refusal_lists_dependents_and_the_fix_works() {
    let mut s = session("errors-delete-item");
    let err = s.call("delete_item", json!({"item": 1}));
    let text = err.error();
    assert!(text.contains("Can't delete item #1 'Deploy'"), "{text}");
    assert!(text.contains("#2 'Release'"), "{text}");
    assert!(text.contains("remove_dependency"), "{text}");

    s.call("remove_dependency", json!({"item": 2, "blocked_by": 1}))
        .json();
    s.call("delete_item", json!({"item": 1})).json();
}

#[test]
fn delete_list_refusal_names_the_blockers_and_the_fix_works() {
    let mut s = session("errors-delete-list");
    let err = s.call("delete_list", json!({"list": "ops"}));
    let text = err.error();
    assert!(text.contains("Can't delete list 'ops'"), "{text}");
    assert!(
        text.contains("#1 'Deploy' blocks #2 'Release' (list 'launch')"),
        "{text}"
    );

    s.call("remove_dependency", json!({"item": 2, "blocked_by": 1}))
        .json();
    let gone = s.call("delete_list", json!({"list": "ops"})).json();
    assert_eq!(gone["items_deleted"], 1);
}

#[test]
fn delete_tag_refusal_gives_the_count_and_the_fix_works() {
    let mut s = session("errors-delete-tag");
    s.call("create_tag", json!({"name": "hot"})).json();
    s.call("update_item", json!({"item": 1, "tags": ["hot"]}))
        .json();
    s.call("update_item", json!({"item": 2, "tags": ["hot"]}))
        .json();

    let err = s.call("delete_tag", json!({"tag": "hot"}));
    let text = err.error();
    assert!(text.contains("used by 2 items"), "{text}");
    assert!(text.contains("update_item"), "{text}");

    s.call("update_item", json!({"item": 1, "tags": []})).json();
    s.call("update_item", json!({"item": 2, "tags": []})).json();
    s.call("delete_tag", json!({"tag": "hot"})).json();
}

#[test]
fn cycles_are_rejected_and_the_graph_is_unchanged() {
    let mut s = session("errors-cycle");
    let err = s.call("add_dependency", json!({"item": 1, "blocked_by": 2}));
    let text = err.error();
    assert!(text.contains("would create a cycle"), "{text}");
    assert!(
        text.contains("#1 'Deploy' → #2 'Release' → #1 'Deploy'"),
        "{text}"
    );
    // item 1 still has no blockers
    let v = s.call("search_items", json!({"text": "Deploy"})).json();
    assert_eq!(v["items"][0]["blocked"], false);
}

#[test]
fn unknown_tags_are_rejected_everywhere_with_the_existing_tags_listed() {
    let mut s = session("errors-unknown-tag");
    s.call("create_tag", json!({"name": "bug"})).json();

    let checks = [
        s.call(
            "add_items",
            json!({"list": "ops", "items": [{"title": "x", "tags": ["bgu"]}]}),
        ),
        s.call("update_item", json!({"item": 1, "tags": ["bgu"]})),
    ];
    for err in &checks {
        let text = err.error();
        assert!(text.contains("Unknown tag 'bgu'"), "{text}");
        assert!(text.contains("create_tag"), "{text}");
        assert!(text.contains("Existing tags: 'bug'"), "{text}");
    }
    // nothing was added or changed by the failed calls
    let ops = s.call("get_list", json!({"list": "ops"})).json();
    assert_eq!(ops["total_count"], 1);

    // searching by an unknown tag is an error too, not a silent empty result
    let err = s.call("search_items", json!({"tag": "bgu"}));
    assert!(err.error().contains("'bug'"), "{}", err.text);

    // creating the tag fixes it
    s.call("create_tag", json!({"name": "bgu"})).json();
    s.call("update_item", json!({"item": 1, "tags": ["bgu"]}))
        .json();
}

#[test]
fn notes_size_limit_applies_to_add_and_update() {
    let mut s = session("errors-notes");
    let too_long = "n".repeat(4001);
    let just_right = "n".repeat(4000);

    let err = s.call(
        "add_items",
        json!({"list": "ops", "items": [{"title": "x", "notes": too_long}]}),
    );
    assert!(
        err.error()
            .contains("limited to 4000 characters (got 4001)"),
        "{}",
        err.text
    );
    assert!(err.text.contains("Nothing was added"), "{}", err.text);

    let err = s.call("update_item", json!({"item": 1, "notes": too_long}));
    assert!(
        err.error().contains("limited to 4000 characters"),
        "{}",
        err.text
    );

    s.call("update_item", json!({"item": 1, "notes": just_right}))
        .json();
}

#[test]
fn every_error_is_a_tool_error_not_a_protocol_failure() {
    let mut s = session("errors-shape");
    for (tool, args) in [
        ("delete_item", json!({"item": 1})),
        ("delete_list", json!({"list": "ops"})),
        ("add_dependency", json!({"item": 1, "blocked_by": 1})),
        ("get_list", json!({"list": "ghost"})),
        ("update_item", json!({"item": 99, "title": "x"})),
        ("reorder_item", json!({"item": 1})),
    ] {
        let outcome = s.call(tool, args);
        assert!(outcome.is_error, "{tool} should have failed");
        assert!(!outcome.text.is_empty(), "{tool} gave an empty error");
    }
    // the server is still fine
    assert_eq!(s.call("list_lists", json!({})).json()["total_count"], 2);
}
