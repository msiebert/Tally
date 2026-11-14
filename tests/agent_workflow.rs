//! A scripted agent session: the same workflow a real agent is asked to do in
//! scripts/e2e-claude-code.sh, checked step by step.

mod common;
use common::*;
use serde_json::json;

#[test]
fn launch_checklist_workflow() {
    let db = temp_db("agent-workflow");
    let mut s = Session::start(&db);

    // An agent sees guidance in the server instructions before anything else.
    assert!(
        s.init["instructions"]
            .as_str()
            .unwrap()
            .contains("Read and follow")
    );

    // 1-2: a list and a tag, each with instructions
    s.call(
        "create_list",
        json!({"name": "launch", "instructions": "Item titles must start with a verb."}),
    )
    .json();
    s.call(
        "create_tag",
        json!({"name": "critical", "instructions": "Do critical items before anything else."}),
    )
    .json();

    // 3: items; the third waits for the second
    let added = s
        .call(
            "add_items",
            json!({"list": "launch", "items": [
                {"title": "Write release notes"},
                {"title": "Cut release branch", "tags": ["critical"]},
                {"title": "Publish release", "blocked_by": [2]},
            ]}),
        )
        .json();
    assert_eq!(added["created_ids"], json!([1, 2, 3]));

    // 4: reading the list delivers both sets of instructions
    let read = s.call("get_list", json!({"list": "launch"})).json();
    assert_eq!(
        read["list"]["instructions"],
        "Item titles must start with a verb."
    );
    assert_eq!(
        read["tag_instructions"]["critical"],
        "Do critical items before anything else."
    );

    // 5: only the first two items are workable
    let free = s
        .call(
            "get_list",
            json!({"list": "launch", "unblocked_only": true}),
        )
        .json();
    let titles: Vec<&str> = free["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Write release notes", "Cut release branch"]);

    // 6: finishing the branch unblocks the publish step
    s.call("complete_item", json!({"item": 2})).json();
    let free = s
        .call(
            "get_list",
            json!({"list": "launch", "unblocked_only": true,
                   "include_instructions": false, "include_tag_instructions": false}),
        )
        .json();
    let titles: Vec<&str> = free["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Write release notes", "Publish release"]);
    assert!(free.get("tag_instructions").is_none());

    // and the whole thing is still there after a restart
    drop(s);
    let mut s = Session::start(&db);
    let after = s
        .call(
            "get_list",
            json!({"list": "launch", "status": ["todo", "done"]}),
        )
        .json();
    assert_eq!(after["total_count"], 3);
    assert_eq!(after["items"][1]["status"], "done");
}
