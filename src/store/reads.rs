//! Reading items: `get_list` (one list, in order) and `search_items` (across
//! lists, most recently updated first). Both are cursor-paginated.
//!
//! Cursors hold the sort key of the last row returned. Items added, deleted or
//! edited between pages therefore never cause a repeat, and never hide an item
//! that wasn't touched. (In `search_items` an item edited mid-read moves to the
//! front of the order, so it can be missed by a read already in progress.)

use super::items::{self, Status};
use super::refs::NameOrId;
use super::{lists, tags};
use crate::error::{Result, ToolError};
use crate::page;
use rusqlite::{Connection, params_from_iter, types::Value as Sql};
use serde_json::{Map, Value, json};

/// Filters and paging shared by both reads, plus the instruction flags.
#[derive(Debug)]
pub struct ReadOptions<'a> {
    pub statuses: Option<&'a [Status]>,
    pub tag: Option<&'a str>,
    pub unblocked_only: bool,
    pub cursor: Option<&'a str>,
    pub limit: Option<u32>,
    /// `get_list` only: return the list's own instructions.
    pub include_instructions: bool,
    /// Return instructions for the tags on this page's items.
    pub include_tag_instructions: bool,
}

impl Default for ReadOptions<'_> {
    fn default() -> Self {
        ReadOptions {
            statuses: None,
            tag: None,
            unblocked_only: false,
            cursor: None,
            limit: None,
            include_instructions: true,
            include_tag_instructions: true,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Order {
    /// List order: position, then ID.
    Position,
    /// Most recently updated first, then ID.
    Recent,
}

struct Query {
    clauses: Vec<String>,
    args: Vec<Sql>,
}

impl Query {
    fn new() -> Self {
        Query {
            clauses: Vec::new(),
            args: Vec::new(),
        }
    }

    fn push(&mut self, clause: &str, args: impl IntoIterator<Item = Sql>) {
        self.clauses.push(clause.to_string());
        self.args.extend(args);
    }

    fn where_sql(&self) -> String {
        if self.clauses.is_empty() {
            "1 = 1".to_string()
        } else {
            self.clauses.join(" AND ")
        }
    }
}

fn filters(
    conn: &Connection,
    list_id: Option<i64>,
    text: Option<&str>,
    opts: &ReadOptions<'_>,
    hide_done_by_default: bool,
) -> Result<Query> {
    let mut q = Query::new();
    if let Some(id) = list_id {
        q.push("i.list_id = ?", [Sql::Integer(id)]);
    }
    match opts.statuses {
        Some(statuses) if !statuses.is_empty() => {
            let names: Vec<String> = statuses
                .iter()
                .map(|s| format!("'{}'", s.as_str()))
                .collect();
            q.push(&format!("i.status IN ({})", names.join(", ")), []);
        }
        _ if hide_done_by_default => q.push("i.status <> 'done'", []),
        _ => {}
    }
    if let Some(tag) = opts.tag {
        let tag = tag.trim();
        let Some(row) = tags::by_name(conn, tag)? else {
            return Err(ToolError::new(format!(
                "No tag named '{tag}'. {}",
                tags::existing_hint(conn)?
            )));
        };
        q.push(
            "EXISTS (SELECT 1 FROM item_tag it WHERE it.item_id = i.id AND it.tag_id = ?)",
            [Sql::Integer(row.id)],
        );
    }
    if let Some(text) = text.map(str::trim).filter(|t| !t.is_empty()) {
        let needle = text.to_lowercase();
        q.push(
            "(instr(lower(i.title), ?) > 0 OR instr(lower(coalesce(i.notes, '')), ?) > 0)",
            [Sql::Text(needle.clone()), Sql::Text(needle)],
        );
    }
    if opts.unblocked_only {
        q.push(
            "NOT EXISTS (SELECT 1 FROM dependency d JOIN item b ON b.id = d.blocked_by_item_id
                          WHERE d.item_id = i.id AND b.status NOT IN ('done', 'cancelled'))",
            [],
        );
    }
    Ok(q)
}

struct Page {
    items: Vec<Value>,
    next_cursor: Option<String>,
    total_count: i64,
}

fn run(
    conn: &Connection,
    mut q: Query,
    order: Order,
    opts: &ReadOptions<'_>,
    include_list: bool,
) -> Result<Page> {
    let limit = page::limit(opts.limit);
    let total_count: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM item i WHERE {}", q.where_sql()),
        params_from_iter(q.args.iter()),
        |r| r.get(0),
    )?;

    if let Some(cursor) = opts.cursor {
        let parts = page::decode(cursor, 2)?;
        let id = page::parse_id(parts[1])?;
        match order {
            Order::Position => {
                let position = page::parse_id(parts[0])?;
                q.push(
                    "(i.position > ? OR (i.position = ? AND i.id > ?))",
                    [
                        Sql::Integer(position),
                        Sql::Integer(position),
                        Sql::Integer(id),
                    ],
                );
            }
            Order::Recent => {
                let updated = parts[0].to_string();
                q.push(
                    "(i.updated_at < ? OR (i.updated_at = ? AND i.id > ?))",
                    [
                        Sql::Text(updated.clone()),
                        Sql::Text(updated),
                        Sql::Integer(id),
                    ],
                );
            }
        }
    }
    let order_sql = match order {
        Order::Position => "i.position, i.id",
        Order::Recent => "i.updated_at DESC, i.id",
    };
    let sql = format!(
        "SELECT i.id, i.position, i.updated_at FROM item i WHERE {} ORDER BY {order_sql} LIMIT {}",
        q.where_sql(),
        limit + 1
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut rows: Vec<(i64, i64, String)> = stmt
        .query_map(params_from_iter(q.args.iter()), |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let has_more = rows.len() as i64 > limit;
    rows.truncate(limit as usize);
    let next_cursor = match (has_more, rows.last()) {
        (true, Some((id, position, updated))) => Some(page::encode(&[
            match order {
                Order::Position => position.to_string(),
                Order::Recent => updated.clone(),
            },
            id.to_string(),
        ])),
        _ => None,
    };
    let items = rows
        .iter()
        .map(|(id, _, _)| items::view(conn, *id, include_list))
        .collect::<Result<Vec<_>>>()?;
    Ok(Page {
        items,
        next_cursor,
        total_count,
    })
}

/// Instructions for the tags that appear on `items`, keyed by tag name. Tags
/// without instructions are left out. The server remembers nothing about what
/// it has sent, so an agent that lost context just asks again.
fn tag_instructions(conn: &Connection, items: &[Value]) -> Result<Map<String, Value>> {
    let mut map = Map::new();
    for item in items {
        let Some(names) = item["tags"].as_array() else {
            continue;
        };
        for name in names.iter().filter_map(Value::as_str) {
            if map.contains_key(name) {
                continue;
            }
            if let Some(text) = tags::by_name(conn, name)?.and_then(|t| t.instructions) {
                map.insert(name.to_string(), Value::String(text));
            }
        }
    }
    Ok(map)
}

/// One list's items in list order. Done items are hidden unless `statuses` is given.
pub fn get_list(conn: &Connection, list: &NameOrId, opts: &ReadOptions<'_>) -> Result<Value> {
    let list = lists::find(conn, list)?;
    let q = filters(conn, Some(list.id), None, opts, true)?;
    let page = run(conn, q, Order::Position, opts, false)?;
    // Instructions ride along on the first page only, so paging through a long
    // list doesn't repeat them.
    let first_page = opts.cursor.is_none();
    let mut list_json = json!({"id": list.id, "name": list.name});
    if first_page
        && opts.include_instructions
        && let Some(text) = &list.instructions
    {
        list_json["instructions"] = json!(text);
    }
    let mut out = json!({
        "list": list_json,
        "items": page.items,
        "next_cursor": page.next_cursor,
        "total_count": page.total_count,
    });
    if first_page && opts.include_tag_instructions {
        let map = tag_instructions(conn, &page.items)?;
        if !map.is_empty() {
            out["tag_instructions"] = Value::Object(map);
        }
    }
    Ok(out)
}

/// Items across lists, most recently updated first. All statuses are searched
/// unless `statuses` is given.
pub fn search_items(
    conn: &Connection,
    text: Option<&str>,
    list: Option<&NameOrId>,
    opts: &ReadOptions<'_>,
) -> Result<Value> {
    let list_id = match list {
        Some(l) => Some(lists::find(conn, l)?.id),
        None => None,
    };
    let q = filters(conn, list_id, text, opts, false)?;
    let page = run(conn, q, Order::Recent, opts, true)?;
    let mut out = json!({
        "items": page.items,
        "next_cursor": page.next_cursor,
        "total_count": page.total_count,
    });
    if opts.cursor.is_none() && opts.include_tag_instructions {
        let map = tag_instructions(conn, &page.items)?;
        if !map.is_empty() {
            out["tag_instructions"] = Value::Object(map);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::store::items::NewItem;
    use crate::store::{dependencies, items};

    fn name(s: &str) -> NameOrId {
        NameOrId::Name(s.into())
    }

    fn setup() -> Connection {
        let mut conn = open_in_memory().unwrap();
        lists::create(&mut conn, "inbox", None).unwrap();
        lists::create(&mut conn, "work", None).unwrap();
        conn
    }

    fn add(conn: &mut Connection, list: &str, title: &str) -> i64 {
        add_with(conn, list, title, None, None)
    }

    fn add_with(
        conn: &mut Connection,
        list: &str,
        title: &str,
        notes: Option<&str>,
        tag_names: Option<&[&str]>,
    ) -> i64 {
        let item = NewItem {
            title: title.into(),
            notes: notes.map(Into::into),
            tags: tag_names.map(|t| t.iter().map(|s| s.to_string()).collect()),
            position: None,
            blocked_by: None,
        };
        items::add(conn, &name(list), &[item]).unwrap()["created_ids"][0]
            .as_i64()
            .unwrap()
    }

    fn titles(v: &Value) -> Vec<String> {
        v["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["title"].as_str().unwrap().to_string())
            .collect()
    }

    fn set_status(conn: &mut Connection, id: i64, status: Status) {
        items::update(conn, id, None, None, Some(status), None).unwrap();
    }

    #[test]
    fn get_list_is_in_position_order_and_hides_done_but_shows_cancelled() {
        let mut conn = setup();
        add(&mut conn, "inbox", "a");
        let b = add(&mut conn, "inbox", "b");
        let c = add(&mut conn, "inbox", "c");
        add(&mut conn, "inbox", "d");
        add(&mut conn, "work", "elsewhere");
        set_status(&mut conn, b, Status::Done);
        set_status(&mut conn, c, Status::Cancelled);

        let v = get_list(&conn, &name("inbox"), &ReadOptions::default()).unwrap();
        assert_eq!(titles(&v), ["a", "c", "d"]);
        assert_eq!(v["total_count"], 3);
        assert_eq!(v["list"]["name"], "inbox");
        assert!(v["next_cursor"].is_null());
        assert!(
            v["items"][0].get("list").is_none(),
            "list name is not repeated per item"
        );

        let done = [Status::Done];
        let v = get_list(
            &conn,
            &name("inbox"),
            &ReadOptions {
                statuses: Some(&done),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(titles(&v), ["b"]);

        let both = [Status::Todo, Status::Done];
        let v = get_list(
            &conn,
            &name("inbox"),
            &ReadOptions {
                statuses: Some(&both),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(titles(&v), ["a", "b", "d"]);
    }

    #[test]
    fn get_list_filters_by_tag_and_unblocked() {
        let mut conn = setup();
        tags::create(&mut conn, "bug", None).unwrap();
        add_with(&mut conn, "inbox", "tagged", None, Some(&["bug"]));
        let blocker = add(&mut conn, "work", "blocker");
        let waiting = add(&mut conn, "inbox", "waiting");
        add(&mut conn, "inbox", "free");
        dependencies::add(&mut conn, waiting, blocker).unwrap();

        let by_tag = get_list(
            &conn,
            &name("inbox"),
            &ReadOptions {
                tag: Some("BUG"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(titles(&by_tag), ["tagged"]);
        assert_eq!(by_tag["items"][0]["tags"], json!(["bug"]));

        let free = get_list(
            &conn,
            &name("inbox"),
            &ReadOptions {
                unblocked_only: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(titles(&free), ["tagged", "free"]);

        let all = get_list(&conn, &name("inbox"), &ReadOptions::default()).unwrap();
        assert_eq!(all["items"][1]["blocked"], true);
        assert_eq!(all["items"][1]["open_blockers"][0]["title"], "blocker");

        // finishing the blocker makes the item show up as unblocked
        set_status(&mut conn, blocker, Status::Done);
        let free = get_list(
            &conn,
            &name("inbox"),
            &ReadOptions {
                unblocked_only: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(titles(&free), ["tagged", "waiting", "free"]);
    }

    #[test]
    fn unknown_tag_filter_is_an_error_not_an_empty_result() {
        let mut conn = setup();
        tags::create(&mut conn, "bug", None).unwrap();
        let err = get_list(
            &conn,
            &name("inbox"),
            &ReadOptions {
                tag: Some("bgu"),
                ..Default::default()
            },
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("No tag named 'bgu'"), "{err}");
        assert!(err.contains("'bug'"), "{err}");
    }

    #[test]
    fn get_list_pages_are_stable_across_edits_adds_and_deletes() {
        let mut conn = setup();
        let ids: Vec<i64> = (1..=7)
            .map(|i| add(&mut conn, "inbox", &format!("i{i}")))
            .collect();
        let opts = ReadOptions {
            limit: Some(3),
            ..Default::default()
        };
        let first = get_list(&conn, &name("inbox"), &opts).unwrap();
        assert_eq!(titles(&first), ["i1", "i2", "i3"]);
        assert_eq!(first["total_count"], 7);
        let cursor = first["next_cursor"].as_str().unwrap().to_string();

        // between pages: edit a seen item, delete an unseen one, add a new one at the bottom
        items::update(&mut conn, ids[0], Some("i1 edited"), None, None, None).unwrap();
        items::delete(&mut conn, ids[4]).unwrap();
        add(&mut conn, "inbox", "i8");

        let opts = ReadOptions {
            limit: Some(3),
            cursor: Some(&cursor),
            ..Default::default()
        };
        let second = get_list(&conn, &name("inbox"), &opts).unwrap();
        assert_eq!(titles(&second), ["i4", "i6", "i7"]);
        let cursor = second["next_cursor"].as_str().unwrap().to_string();
        let opts = ReadOptions {
            limit: Some(3),
            cursor: Some(&cursor),
            ..Default::default()
        };
        let third = get_list(&conn, &name("inbox"), &opts).unwrap();
        assert_eq!(titles(&third), ["i8"]);
        assert!(third["next_cursor"].is_null());
    }

    #[test]
    fn moving_an_item_to_the_top_between_pages_does_not_repeat_it() {
        let mut conn = setup();
        for i in 1..=4 {
            add(&mut conn, "inbox", &format!("i{i}"));
        }
        let opts = ReadOptions {
            limit: Some(2),
            ..Default::default()
        };
        let first = get_list(&conn, &name("inbox"), &opts).unwrap();
        let cursor = first["next_cursor"].as_str().unwrap().to_string();
        // i2 was already shown; sending it to the bottom must not show it again before i3/i4
        items::move_to(&mut conn, 2, &name("inbox"), None).unwrap();
        let opts = ReadOptions {
            limit: Some(10),
            cursor: Some(&cursor),
            ..Default::default()
        };
        let second = get_list(&conn, &name("inbox"), &opts).unwrap();
        assert_eq!(titles(&second), ["i3", "i4", "i2"]);
    }

    #[test]
    fn limit_defaults_to_50_and_caps_at_200() {
        let mut conn = setup();
        let many: Vec<NewItem> = (0..210)
            .map(|i| NewItem {
                title: format!("i{i}"),
                notes: None,
                tags: None,
                position: None,
                blocked_by: None,
            })
            .collect();
        items::add(&mut conn, &name("inbox"), &many[..200]).unwrap();
        items::add(&mut conn, &name("inbox"), &many[200..]).unwrap();
        let v = get_list(&conn, &name("inbox"), &ReadOptions::default()).unwrap();
        assert_eq!(v["items"].as_array().unwrap().len(), 50);
        assert_eq!(v["total_count"], 210);
        let v = get_list(
            &conn,
            &name("inbox"),
            &ReadOptions {
                limit: Some(5000),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(v["items"].as_array().unwrap().len(), 200);
        assert!(v["next_cursor"].is_string());
    }

    #[test]
    fn search_matches_title_and_notes_case_insensitively_and_literally() {
        let mut conn = setup();
        add_with(&mut conn, "inbox", "Buy MILK", None, None);
        add_with(&mut conn, "work", "report", Some("mentions milk run"), None);
        add_with(&mut conn, "work", "100% done_ish", None, None);
        add(&mut conn, "work", "unrelated");

        let v = search_items(&conn, Some("milk"), None, &ReadOptions::default()).unwrap();
        assert_eq!(v["total_count"], 2);
        // search results name the list each item is in
        let lists: Vec<&str> = v["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["list"].as_str().unwrap())
            .collect();
        assert!(lists.contains(&"inbox") && lists.contains(&"work"));

        // % and _ are plain characters, not wildcards
        let v = search_items(&conn, Some("100%"), None, &ReadOptions::default()).unwrap();
        assert_eq!(titles(&v), ["100% done_ish"]);
        let v = search_items(&conn, Some("done_ish"), None, &ReadOptions::default()).unwrap();
        assert_eq!(v["total_count"], 1);
        let v = search_items(&conn, Some("%"), None, &ReadOptions::default()).unwrap();
        assert_eq!(v["total_count"], 1);
    }

    #[test]
    fn search_combines_filters_and_includes_done_items_by_default() {
        let mut conn = setup();
        tags::create(&mut conn, "bug", None).unwrap();
        let a = add_with(&mut conn, "inbox", "fix login", None, Some(&["bug"]));
        add_with(&mut conn, "work", "fix logout", None, Some(&["bug"]));
        add(&mut conn, "work", "fix typo");
        set_status(&mut conn, a, Status::Done);

        let v = search_items(&conn, Some("fix"), None, &ReadOptions::default()).unwrap();
        assert_eq!(v["total_count"], 3, "done items are searched by default");

        let opts = ReadOptions {
            tag: Some("bug"),
            ..Default::default()
        };
        let v = search_items(&conn, Some("fix"), Some(&name("work")), &opts).unwrap();
        assert_eq!(titles(&v), ["fix logout"]);

        let done = [Status::Done];
        let opts = ReadOptions {
            statuses: Some(&done),
            ..Default::default()
        };
        let v = search_items(&conn, None, None, &opts).unwrap();
        assert_eq!(titles(&v), ["fix login"]);
        assert!(search_items(&conn, None, Some(&name("nope")), &ReadOptions::default()).is_err());
    }

    #[test]
    fn search_orders_by_most_recently_updated_then_id() {
        let mut conn = setup();
        let a = add(&mut conn, "inbox", "a");
        add(&mut conn, "inbox", "b");
        add(&mut conn, "inbox", "c");
        // make updates land in distinct milliseconds
        conn.execute(
            "UPDATE item SET updated_at = '2026-01-01T00:00:00.000Z'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE item SET updated_at = '2026-01-02T00:00:00.000Z' WHERE id = ?1",
            [a],
        )
        .unwrap();
        let v = search_items(&conn, None, None, &ReadOptions::default()).unwrap();
        assert_eq!(titles(&v), ["a", "b", "c"]);
        conn.execute(
            "UPDATE item SET updated_at = '2026-01-03T00:00:00.000Z' WHERE id = 3",
            [],
        )
        .unwrap();
        let v = search_items(&conn, None, None, &ReadOptions::default()).unwrap();
        assert_eq!(titles(&v), ["c", "a", "b"]);
    }

    #[test]
    fn search_pages_never_repeat_or_skip_untouched_items() {
        let mut conn = setup();
        for i in 1..=8 {
            add(&mut conn, "inbox", &format!("i{i}"));
        }
        // give every item a distinct, ordered timestamp: i8 is newest
        for i in 1..=8 {
            conn.execute(
                "UPDATE item SET updated_at = ?1 WHERE id = ?2",
                rusqlite::params![format!("2026-01-{i:02}T00:00:00.000Z"), i],
            )
            .unwrap();
        }
        let opts = ReadOptions {
            limit: Some(3),
            ..Default::default()
        };
        let first = search_items(&conn, None, None, &opts).unwrap();
        assert_eq!(titles(&first), ["i8", "i7", "i6"]);
        let cursor = first["next_cursor"].as_str().unwrap().to_string();

        // between pages: touch a seen item (it jumps to the front), delete an unseen one,
        // and add a brand new item (also newest)
        items::update(&mut conn, 7, Some("i7 edited"), None, None, None).unwrap();
        items::delete(&mut conn, 4).unwrap();
        add(&mut conn, "inbox", "new");

        let opts = ReadOptions {
            limit: Some(10),
            cursor: Some(&cursor),
            ..Default::default()
        };
        let rest = search_items(&conn, None, None, &opts).unwrap();
        assert_eq!(titles(&rest), ["i5", "i3", "i2", "i1"]);
    }

    #[test]
    fn bad_cursor_is_rejected() {
        let conn = setup();
        let opts = ReadOptions {
            cursor: Some("nonsense"),
            ..Default::default()
        };
        assert!(get_list(&conn, &name("inbox"), &opts).is_err());
        assert!(search_items(&conn, None, None, &opts).is_err());
    }

    fn instructions_setup() -> Connection {
        let mut conn = setup();
        lists::update(&mut conn, &name("inbox"), None, Some("Keep titles short")).unwrap();
        tags::create(&mut conn, "bug", Some("Reproduce before fixing")).unwrap();
        tags::create(&mut conn, "idea", Some("Park, don't start")).unwrap();
        tags::create(&mut conn, "plain", None).unwrap();
        conn
    }

    #[test]
    fn get_list_returns_list_and_tag_instructions_once_on_the_first_page() {
        let mut conn = instructions_setup();
        add_with(&mut conn, "inbox", "a", None, Some(&["bug", "plain"]));
        add_with(&mut conn, "inbox", "b", None, Some(&["bug"]));
        add_with(&mut conn, "inbox", "c", None, Some(&["idea"]));

        let v = get_list(&conn, &name("inbox"), &ReadOptions::default()).unwrap();
        assert_eq!(v["list"]["instructions"], "Keep titles short");
        // one entry per tag, not per item; tags without instructions are omitted
        assert_eq!(
            v["tag_instructions"],
            json!({"bug": "Reproduce before fixing", "idea": "Park, don't start"})
        );
    }

    #[test]
    fn tag_instructions_cover_only_tags_on_the_returned_page() {
        let mut conn = instructions_setup();
        add_with(&mut conn, "inbox", "a", None, Some(&["bug"]));
        add_with(&mut conn, "inbox", "b", None, Some(&["idea"]));
        let opts = ReadOptions {
            limit: Some(1),
            ..Default::default()
        };
        let v = get_list(&conn, &name("inbox"), &opts).unwrap();
        assert_eq!(
            v["tag_instructions"],
            json!({"bug": "Reproduce before fixing"})
        );
        assert!(v["tag_instructions"].get("idea").is_none());
    }

    #[test]
    fn later_pages_never_repeat_instructions_whatever_the_flags_say() {
        let mut conn = instructions_setup();
        add_with(&mut conn, "inbox", "a", None, Some(&["bug"]));
        add_with(&mut conn, "inbox", "b", None, Some(&["idea"]));
        let first = get_list(
            &conn,
            &name("inbox"),
            &ReadOptions {
                limit: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        let cursor = first["next_cursor"].as_str().unwrap().to_string();
        let opts = ReadOptions {
            limit: Some(1),
            cursor: Some(&cursor),
            include_instructions: true,
            include_tag_instructions: true,
            ..Default::default()
        };
        let second = get_list(&conn, &name("inbox"), &opts).unwrap();
        assert_eq!(titles(&second), ["b"]);
        assert!(second["list"].get("instructions").is_none());
        assert!(second.get("tag_instructions").is_none());
    }

    #[test]
    fn flags_turn_instructions_off() {
        let mut conn = instructions_setup();
        add_with(&mut conn, "inbox", "a", None, Some(&["bug"]));
        let opts = ReadOptions {
            include_instructions: false,
            include_tag_instructions: false,
            ..Default::default()
        };
        let v = get_list(&conn, &name("inbox"), &opts).unwrap();
        assert!(v["list"].get("instructions").is_none());
        assert!(v.get("tag_instructions").is_none());
        // items still show their tag names
        assert_eq!(v["items"][0]["tags"], json!(["bug"]));

        let only_list = ReadOptions {
            include_tag_instructions: false,
            ..Default::default()
        };
        let v = get_list(&conn, &name("inbox"), &only_list).unwrap();
        assert_eq!(v["list"]["instructions"], "Keep titles short");
        assert!(v.get("tag_instructions").is_none());
    }

    #[test]
    fn asking_again_returns_the_same_instructions() {
        // the server is stateless: a second identical read repeats them in full
        let mut conn = instructions_setup();
        add_with(&mut conn, "inbox", "a", None, Some(&["bug"]));
        let first = get_list(&conn, &name("inbox"), &ReadOptions::default()).unwrap();
        let again = get_list(&conn, &name("inbox"), &ReadOptions::default()).unwrap();
        assert_eq!(first, again);
    }

    #[test]
    fn search_returns_tag_instructions_on_the_first_page_and_honors_the_flag() {
        let mut conn = instructions_setup();
        add_with(&mut conn, "inbox", "a", None, Some(&["bug"]));
        add_with(&mut conn, "work", "b", None, Some(&["idea"]));

        let v = search_items(&conn, None, None, &ReadOptions::default()).unwrap();
        assert_eq!(
            v["tag_instructions"],
            json!({"bug": "Reproduce before fixing", "idea": "Park, don't start"})
        );
        assert!(
            v.get("list").is_none(),
            "search has no single list to describe"
        );

        let off = ReadOptions {
            include_tag_instructions: false,
            ..Default::default()
        };
        let v = search_items(&conn, None, None, &off).unwrap();
        assert!(v.get("tag_instructions").is_none());

        let first = search_items(
            &conn,
            None,
            None,
            &ReadOptions {
                limit: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        let cursor = first["next_cursor"].as_str().unwrap().to_string();
        let second = search_items(
            &conn,
            None,
            None,
            &ReadOptions {
                limit: Some(1),
                cursor: Some(&cursor),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(second.get("tag_instructions").is_none());
    }
}
