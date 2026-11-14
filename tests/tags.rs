//! Tag tools, end to end through the MCP server.

mod common;
use common::*;
use serde_json::json;

#[test]
fn tag_lifecycle() {
    let mut s = Session::start(&temp_db("tag-lifecycle"));

    let created = s
        .call(
            "create_tag",
            json!({"name": "urgent", "instructions": "Handle before anything else"}),
        )
        .json();
    assert_eq!(created["id"], 1);
    assert_eq!(created["has_instructions"], true);

    let dup = s.call("create_tag", json!({"name": "URGENT"}));
    assert!(dup.error().contains("already exists"));

    let renamed = s
        .call("update_tag", json!({"tag": "urgent", "name": "p0"}))
        .json();
    assert_eq!(renamed["name"], "p0");

    let all = s.call("list_tags", json!({})).json();
    assert_eq!(all["total_count"], 1);
    assert_eq!(all["tags"][0]["name"], "p0");
    assert_eq!(all["tags"][0]["item_count"], 0);

    let gone = s.call("delete_tag", json!({"tag": "p0"})).json();
    assert_eq!(gone["deleted"]["name"], "p0");
    assert_eq!(s.call("list_tags", json!({})).json()["total_count"], 0);
}

#[test]
fn unknown_tag_errors_point_at_create_tag() {
    let mut s = Session::start(&temp_db("tag-errors"));
    s.call("create_tag", json!({"name": "bug"})).json();
    let err = s.call("delete_tag", json!({"tag": "bgu"}));
    assert!(err.error().contains("create_tag"), "{}", err.text);
    assert!(err.text.contains("'bug'"), "{}", err.text);
}

#[test]
fn tool_description_states_create_before_use_up_front() {
    let mut s = Session::start(&temp_db("tag-desc"));
    let tools = s.request("tools/list", json!({}));
    let create_tag = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "create_tag")
        .unwrap();
    let description = create_tag["description"].as_str().unwrap();
    assert!(
        description.starts_with("Create a tag. Tags must be created here before"),
        "{description}"
    );
}
