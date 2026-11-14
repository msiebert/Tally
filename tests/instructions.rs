//! Instruction delivery through the include flags, end to end.

mod common;
use common::*;
use serde_json::{Value, json};

fn session(name: &str) -> Session {
    let mut s = Session::start(&temp_db(name));
    s.call(
        "create_list",
        json!({"name": "inbox", "instructions": "Keep titles short"}),
    )
    .json();
    s.call(
        "create_tag",
        json!({"name": "bug", "instructions": "Reproduce before fixing"}),
    )
    .json();
    s.call("create_tag", json!({"name": "plain"})).json();
    s
}

#[test]
fn instructions_arrive_once_and_can_be_switched_off() {
    let mut s = session("instr-flags");
    s.call(
        "add_items",
        json!({"list": "inbox", "items": [
            {"title": "a", "tags": ["bug"]},
            {"title": "b", "tags": ["bug", "plain"]},
        ]}),
    )
    .json();

    let v = s.call("get_list", json!({"list": "inbox"})).json();
    assert_eq!(v["list"]["instructions"], "Keep titles short");
    assert_eq!(
        v["tag_instructions"],
        json!({"bug": "Reproduce before fixing"})
    );

    // an agent that already has them in context says so
    let v = s
        .call(
            "get_list",
            json!({"list": "inbox", "include_instructions": false, "include_tag_instructions": false}),
        )
        .json();
    assert!(v["list"].get("instructions").is_none());
    assert!(v.get("tag_instructions").is_none());

    // after losing context, asking again simply returns them again
    let again = s.call("get_list", json!({"list": "inbox"})).json();
    assert_eq!(again["list"]["instructions"], "Keep titles short");
}

#[test]
fn only_the_first_page_carries_instructions() {
    let mut s = session("instr-pages");
    let items: Vec<Value> = (1..=3)
        .map(|i| json!({"title": format!("i{i}"), "tags": ["bug"]}))
        .collect();
    s.call("add_items", json!({"list": "inbox", "items": items}))
        .json();

    let first = s
        .call("get_list", json!({"list": "inbox", "limit": 2}))
        .json();
    assert!(first["list"]["instructions"].is_string());
    assert!(first["tag_instructions"].is_object());
    let cursor = first["next_cursor"].as_str().unwrap();

    // even with the flags explicitly on, page two stays quiet
    let second = s
        .call(
            "get_list",
            json!({"list": "inbox", "limit": 2, "cursor": cursor,
                   "include_instructions": true, "include_tag_instructions": true}),
        )
        .json();
    assert!(second["list"].get("instructions").is_none());
    assert!(second.get("tag_instructions").is_none());
    assert_eq!(second["items"].as_array().unwrap().len(), 1);
}

#[test]
fn search_items_returns_tag_instructions_too() {
    let mut s = session("instr-search");
    s.call(
        "add_items",
        json!({"list": "inbox", "items": [{"title": "crash on save", "tags": ["bug"]}]}),
    )
    .json();
    let v = s.call("search_items", json!({"text": "crash"})).json();
    assert_eq!(
        v["tag_instructions"],
        json!({"bug": "Reproduce before fixing"})
    );
    let v = s
        .call(
            "search_items",
            json!({"text": "crash", "include_tag_instructions": false}),
        )
        .json();
    assert!(v.get("tag_instructions").is_none());
}

#[test]
fn there_is_no_separate_tag_instructions_tool_and_descriptions_explain_the_flags() {
    let mut s = session("instr-tools");
    let names = s.tool_names();
    assert!(
        !names.iter().any(|n| n.contains("instructions")),
        "{names:?}"
    );

    let tools = s.request("tools/list", json!({}));
    let describe = |name: &str| -> String {
        tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap()["description"]
            .as_str()
            .unwrap()
            .to_string()
    };
    for tool in ["get_list", "search_items"] {
        let d = describe(tool);
        assert!(d.contains("include_tag_instructions"), "{tool}: {d}");
        assert!(
            d.contains("Read and follow") || d.contains("read and follow"),
            "{tool}: {d}"
        );
    }
    assert!(describe("get_list").contains("include_instructions"));
}
