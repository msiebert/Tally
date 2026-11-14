//! What agents see: server instructions and tool descriptions. These tests keep
//! the guidance present and the token cost bounded.

mod common;
use common::*;
use serde_json::{Value, json};

fn tools(s: &mut Session) -> Vec<Value> {
    s.request("tools/list", json!({}))["tools"]
        .as_array()
        .unwrap()
        .clone()
}

fn description(tools: &[Value], name: &str) -> String {
    tools
        .iter()
        .find(|t| t["name"] == name)
        .unwrap_or_else(|| panic!("no tool {name}"))["description"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn server_instructions_teach_the_rules_agents_need() {
    let s = Session::start(&temp_db("desc-instructions"));
    let text = s.init["instructions"].as_str().unwrap();
    for needle in [
        "Read and follow",
        "include_instructions",
        "include_tag_instructions",
        "first page",
        "create_tag before using it",
        "reorder_item",
        "add_dependency",
        "unblocked_only",
        "next_cursor",
        "total_count",
        "remove_dependency",
        "not idempotent",
    ] {
        assert!(
            text.contains(needle),
            "instructions are missing '{needle}':\n{text}"
        );
    }
}

#[test]
fn guidance_stays_short_so_it_stays_cheap_in_tokens() {
    let mut s = Session::start(&temp_db("desc-budget"));
    let instructions = s.init["instructions"].as_str().unwrap().len();
    assert!(
        instructions <= 2_200,
        "server instructions are {instructions} bytes"
    );

    let tools = tools(&mut s);
    for t in &tools {
        let name = t["name"].as_str().unwrap();
        let len = t["description"].as_str().unwrap().len();
        assert!(len <= 480, "{name}'s description is {len} bytes");
    }
    // Everything an agent loads up front: all tool names, descriptions and schemas.
    let total = serde_json::to_string(&tools).unwrap().len();
    assert!(total <= 17_000, "the tool list is {total} bytes");
}

#[test]
fn tools_that_take_tags_state_the_create_before_use_rule_first() {
    let mut s = Session::start(&temp_db("desc-tags"));
    let tools = tools(&mut s);
    for name in ["add_items", "update_item"] {
        let d = description(&tools, name);
        assert!(
            d.starts_with("Tags must already exist (create_tag first)"),
            "{name}: {d}"
        );
    }
    assert!(
        description(&tools, "create_tag")
            .starts_with("Create a tag. Tags must be created here before")
    );
}

#[test]
fn delete_tools_describe_the_refusal_and_how_to_resolve_it() {
    let mut s = Session::start(&temp_db("desc-deletes"));
    let tools = tools(&mut s);
    for name in ["delete_item", "delete_list"] {
        let d = description(&tools, name);
        assert!(d.contains("Refused"), "{name}: {d}");
        assert!(d.contains("remove_dependency"), "{name}: {d}");
    }
    let d = description(&tools, "delete_tag");
    assert!(d.contains("Refused") && d.contains("update_item"), "{d}");
}

#[test]
fn every_tool_has_a_description_and_documented_parameters() {
    let mut s = Session::start(&temp_db("desc-params"));
    for t in tools(&mut s) {
        let name = t["name"].as_str().unwrap();
        assert!(
            !t["description"].as_str().unwrap_or_default().is_empty(),
            "{name} has no description"
        );
        // every property in the top-level schema explains itself
        if let Some(props) = t["inputSchema"]["properties"].as_object() {
            for (prop, schema) in props {
                assert!(
                    schema["description"].is_string(),
                    "{name}.{prop} has no description: {schema}"
                );
            }
        }
    }
}
