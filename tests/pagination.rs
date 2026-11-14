//! Cursor pagination on every paginated tool, including edits between pages.

mod common;
use common::*;
use serde_json::{Value, json};
use std::collections::HashSet;

/// Follow next_cursor until it is null. `between` runs after each page except the last.
fn walk(
    s: &mut Session,
    tool: &str,
    mut args: Value,
    key: &str,
    mut between: impl FnMut(&mut Session, usize, &Value),
) -> Vec<Value> {
    let mut all = Vec::new();
    let mut page_no = 0;
    loop {
        let page = s.call(tool, args.clone()).json();
        all.extend(page[key].as_array().unwrap().iter().cloned());
        let cursor = page["next_cursor"].clone();
        if cursor.is_null() {
            return all;
        }
        between(s, page_no, &page);
        page_no += 1;
        args["cursor"] = cursor;
    }
}

fn names(v: &[Value]) -> Vec<String> {
    v.iter()
        .map(|x| x["name"].as_str().unwrap().to_string())
        .collect()
}

fn titles(v: &[Value]) -> Vec<String> {
    v.iter()
        .map(|x| x["title"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn list_lists_pages_survive_creates_and_deletes() {
    let mut s = Session::start(&temp_db("pages-lists"));
    for i in 1..=6 {
        s.call("create_list", json!({"name": format!("l{i}")}))
            .json();
    }
    let got = walk(
        &mut s,
        "list_lists",
        json!({"limit": 2}),
        "lists",
        |s, page, _| {
            if page == 0 {
                s.call("delete_list", json!({"list": "l1"})).json(); // already seen
                s.call("delete_list", json!({"list": "l4"})).json(); // not yet seen
                s.call("create_list", json!({"name": "l7"})).json();
            }
        },
    );
    assert_eq!(names(&got), ["l1", "l2", "l3", "l5", "l6", "l7"]);
}

#[test]
fn list_tags_pages_survive_creates_and_deletes() {
    let mut s = Session::start(&temp_db("pages-tags"));
    for i in 1..=5 {
        s.call("create_tag", json!({"name": format!("t{i}")}))
            .json();
    }
    let got = walk(
        &mut s,
        "list_tags",
        json!({"limit": 2}),
        "tags",
        |s, page, _| {
            if page == 0 {
                s.call("delete_tag", json!({"tag": "t5"})).json();
                s.call("create_tag", json!({"name": "t6"})).json();
            }
        },
    );
    assert_eq!(names(&got), ["t1", "t2", "t3", "t4", "t6"]);
}

#[test]
fn get_list_pages_survive_edits_deletes_and_adds() {
    let mut s = Session::start(&temp_db("pages-get-list"));
    s.call("create_list", json!({"name": "todo"})).json();
    let items: Vec<Value> = (1..=8).map(|i| json!({"title": format!("i{i}")})).collect();
    s.call("add_items", json!({"list": "todo", "items": items}))
        .json();

    let got = walk(
        &mut s,
        "get_list",
        json!({"list": "todo", "limit": 3}),
        "items",
        |s, page, p| {
            assert_eq!(p["total_count"], if page == 0 { 8 } else { 7 });
            if page == 0 {
                s.call("update_item", json!({"item": 1, "title": "i1 (edited)"}))
                    .json();
                s.call("complete_item", json!({"item": 5})).json(); // done items drop out
                s.call("delete_item", json!({"item": 6})).json();
                s.call(
                    "add_items",
                    json!({"list": "todo", "items": [{"title": "i9"}]}),
                )
                .json();
            }
        },
    );
    // page one showed i1..i3 before the edits; afterwards 5 is done, 6 is gone, 9 is new
    assert_eq!(titles(&got), ["i1", "i2", "i3", "i4", "i7", "i8", "i9"]);
}

#[test]
fn search_items_pages_never_repeat_or_skip_untouched_items() {
    let mut s = Session::start(&temp_db("pages-search"));
    s.call("create_list", json!({"name": "todo"})).json();
    let items: Vec<Value> = (1..=9)
        .map(|i| json!({"title": format!("job {i}")}))
        .collect();
    s.call("add_items", json!({"list": "todo", "items": items}))
        .json();
    // give items distinct update times so the order is well defined
    for id in 1..=9 {
        std::thread::sleep(std::time::Duration::from_millis(3));
        s.call("update_item", json!({"item": id, "notes": "n"}))
            .json();
    }

    let mut seen_first_page: Vec<i64> = Vec::new();
    let got = walk(
        &mut s,
        "search_items",
        json!({"text": "job", "limit": 3}),
        "items",
        |s, page, p| {
            if page == 0 {
                seen_first_page = p["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|i| i["id"].as_i64().unwrap())
                    .collect();
                // touch an already-seen item, delete an unseen one, add a new one
                s.call(
                    "update_item",
                    json!({"item": seen_first_page[0], "title": "job (edited)"}),
                )
                .json();
                s.call("delete_item", json!({"item": 4})).json();
                s.call(
                    "add_items",
                    json!({"list": "todo", "items": [{"title": "job new"}]}),
                )
                .json();
            }
        },
    );
    let ids: Vec<i64> = got.iter().map(|i| i["id"].as_i64().unwrap()).collect();
    let unique: HashSet<i64> = ids.iter().copied().collect();
    assert_eq!(ids.len(), unique.len(), "an item was repeated: {ids:?}");
    // newest-updated first: page one was 9, 8, 7; the rest are 6, 5, (4 deleted), 3, 2, 1
    assert_eq!(seen_first_page, [9, 8, 7]);
    assert_eq!(ids, [9, 8, 7, 6, 5, 3, 2, 1]);
}

#[test]
fn limit_is_capped_at_200_and_total_count_is_the_whole_match() {
    let mut s = Session::start(&temp_db("pages-limit"));
    s.call("create_list", json!({"name": "big"})).json();
    for chunk in 0..2 {
        let items: Vec<Value> = (0..103)
            .map(|i| json!({"title": format!("i{}", chunk * 103 + i)}))
            .collect();
        s.call("add_items", json!({"list": "big", "items": items}))
            .json();
    }
    let page = s
        .call("get_list", json!({"list": "big", "limit": 1000}))
        .json();
    assert_eq!(page["items"].as_array().unwrap().len(), 200);
    assert_eq!(page["total_count"], 206);
    assert!(page["next_cursor"].is_string());

    let default = s.call("get_list", json!({"list": "big"})).json();
    assert_eq!(default["items"].as_array().unwrap().len(), 50);

    // the last page ends with a null cursor
    let rest = s
        .call(
            "get_list",
            json!({"list": "big", "limit": 200, "cursor": page["next_cursor"]}),
        )
        .json();
    assert_eq!(rest["items"].as_array().unwrap().len(), 6);
    assert!(rest["next_cursor"].is_null());
}
