//! Dependency tools, end to end through the MCP server.

mod common;
use common::*;
use serde_json::json;

fn session(name: &str) -> Session {
    let mut s = Session::start(&temp_db(name));
    s.call("create_list", json!({"name": "a"})).json();
    s.call("create_list", json!({"name": "b"})).json();
    s.call(
        "add_items",
        json!({"list": "a", "items": [{"title": "deploy"}, {"title": "announce"}]}),
    )
    .json();
    s.call(
        "add_items",
        json!({"list": "b", "items": [{"title": "release"}]}),
    )
    .json();
    s
}

#[test]
fn blocking_across_lists_and_unblocking_by_completing() {
    let mut s = session("deps-flow");
    // release (3, list b) waits for deploy (1, list a)
    let v = s
        .call("add_dependency", json!({"item": 3, "blocked_by": 1}))
        .json();
    assert_eq!(v["added"], true);
    assert_eq!(v["item"]["blocked"], true);
    assert_eq!(v["item"]["open_blockers"][0]["title"], "deploy");

    let done = s.call("complete_item", json!({"item": 1})).json();
    assert!(done.get("warnings").is_none());
    let v = s
        .call("update_item", json!({"item": 3, "status": "doing"}))
        .json();
    assert_eq!(v["item"]["blocked"], false);
    assert!(v["item"].get("open_blockers").is_none());
}

#[test]
fn cycles_are_rejected_with_the_cycle_named() {
    let mut s = session("deps-cycle");
    s.call("add_dependency", json!({"item": 2, "blocked_by": 1}))
        .json();
    s.call("add_dependency", json!({"item": 3, "blocked_by": 2}))
        .json();
    let err = s.call("add_dependency", json!({"item": 1, "blocked_by": 3}));
    let text = err.error();
    assert!(text.contains("would create a cycle"), "{text}");
    assert!(
        text.contains("#1 'deploy' → #3 'release' → #2 'announce' → #1 'deploy'"),
        "{text}"
    );
}

#[test]
fn delete_refusals_are_resolved_by_removing_the_dependency() {
    let mut s = session("deps-delete");
    s.call("add_dependency", json!({"item": 3, "blocked_by": 1}))
        .json();

    let refused = s.call("delete_item", json!({"item": 1}));
    assert!(refused.error().contains("#3 'release'"), "{}", refused.text);
    let refused = s.call("delete_list", json!({"list": "a"}));
    assert!(
        refused
            .error()
            .contains("#1 'deploy' blocks #3 'release' (list 'b')"),
        "{}",
        refused.text
    );

    s.call("remove_dependency", json!({"item": 3, "blocked_by": 1}))
        .json();
    s.call("delete_item", json!({"item": 1})).json();
    s.call("delete_list", json!({"list": "a"})).json();
}

#[test]
fn bad_dependency_requests_are_readable_errors() {
    let mut s = session("deps-errors");
    let err = s.call("add_dependency", json!({"item": 1, "blocked_by": 1}));
    assert!(err.error().contains("itself"), "{}", err.text);
    let err = s.call("add_dependency", json!({"item": 1, "blocked_by": 99}));
    assert!(err.error().contains("No item with id 99"), "{}", err.text);
    let err = s.call("remove_dependency", json!({"item": 1, "blocked_by": 2}));
    assert!(err.error().contains("isn't blocked by"), "{}", err.text);
    // ids may be sent as text
    s.call("add_dependency", json!({"item": "#2", "blocked_by": "1"}))
        .json();
}
