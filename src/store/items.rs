//! Item operations: add, update, complete, delete, move, and the compact item view
//! shared by every tool that returns items.

use super::position::{Edge, Placement, Position, place};
use super::refs::NameOrId;
use super::{NOW, lists, tags};
use crate::error::{Result, ToolError};
use rusqlite::{Connection, OptionalExtension, params};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

pub const MAX_TITLE_CHARS: usize = 500;
pub const MAX_NOTES_CHARS: usize = 4_000;
pub const MAX_BATCH: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Todo,
    Doing,
    Done,
    Cancelled,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Todo => "todo",
            Status::Doing => "doing",
            Status::Done => "done",
            Status::Cancelled => "cancelled",
        }
    }
}

/// One item to add.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct NewItem {
    /// Short title, one line (max 500 characters).
    pub title: String,
    /// Optional notes, markdown (max 4,000 characters).
    pub notes: Option<String>,
    /// Optional tag names. Tags must already exist (create_tag).
    pub tags: Option<Vec<String>>,
    /// "top", "bottom" (default), or {"after": item_id} for a spot in the list.
    pub position: Option<Position>,
    /// Optional IDs of existing items (in any list) that must be finished before this one.
    #[serde(default, deserialize_with = "crate::params::lenient_ids_opt")]
    pub blocked_by: Option<Vec<i64>>,
}

fn clean_title(raw: &str) -> Result<String> {
    let title = raw.trim();
    if title.is_empty() {
        return Err(ToolError::new("An item title can't be empty."));
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(ToolError::new(format!(
            "An item title can be at most {MAX_TITLE_CHARS} characters. Put the detail in notes."
        )));
    }
    if title.contains(['\n', '\r']) {
        return Err(ToolError::new(
            "An item title must be a single line. Put the detail in notes.",
        ));
    }
    Ok(title.to_string())
}

/// Empty notes mean "no notes".
fn clean_notes(raw: Option<&str>) -> Result<Option<String>> {
    let Some(text) = raw.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    let len = text.chars().count();
    if len > MAX_NOTES_CHARS {
        return Err(ToolError::new(format!(
            "Notes are limited to {MAX_NOTES_CHARS} characters (got {len}). \
             Shorten them, or split the work into several items."
        )));
    }
    Ok(Some(text.to_string()))
}

#[derive(Debug, Clone)]
pub struct ItemRow {
    pub id: i64,
    pub list_id: i64,
    pub title: String,
}

fn item_not_found(id: i64) -> ToolError {
    ToolError::new(format!(
        "No item with id {id}. Use get_list or search_items to find item IDs."
    ))
}

pub fn find(conn: &Connection, id: i64) -> Result<ItemRow> {
    conn.query_row(
        "SELECT id, list_id, title FROM item WHERE id = ?1",
        [id],
        |r| {
            Ok(ItemRow {
                id: r.get(0)?,
                list_id: r.get(1)?,
                title: r.get(2)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| item_not_found(id))
}

/// Blockers that are neither done nor cancelled: (id, title).
pub fn open_blockers(conn: &Connection, item_id: i64) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare(
        "SELECT b.id, b.title FROM dependency d
           JOIN item b ON b.id = d.blocked_by_item_id
          WHERE d.item_id = ?1 AND b.status NOT IN ('done', 'cancelled')
          ORDER BY b.id",
    )?;
    let rows = stmt
        .query_map([item_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// The compact item shape every tool returns. Empty tags and missing notes are
/// left out, and open blockers are listed only when the item is blocked.
pub fn view(conn: &Connection, id: i64, include_list: bool) -> Result<Value> {
    let (title, notes, status, created_at, updated_at, list_name) = conn
        .query_row(
            "SELECT i.title, i.notes, i.status, i.created_at, i.updated_at, l.name
               FROM item i JOIN list l ON l.id = i.list_id WHERE i.id = ?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| item_not_found(id))?;
    let mut stmt = conn.prepare(
        "SELECT t.name FROM item_tag it JOIN tag t ON t.id = it.tag_id
          WHERE it.item_id = ?1 ORDER BY t.name COLLATE NOCASE",
    )?;
    let tag_names: Vec<String> = stmt
        .query_map([id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let blockers = open_blockers(conn, id)?;

    let mut out = Map::new();
    out.insert("id".into(), json!(id));
    if include_list {
        out.insert("list".into(), json!(list_name));
    }
    out.insert("title".into(), json!(title));
    out.insert("status".into(), json!(status));
    if !tag_names.is_empty() {
        out.insert("tags".into(), json!(tag_names));
    }
    out.insert("blocked".into(), json!(!blockers.is_empty()));
    if !blockers.is_empty() {
        let list: Vec<Value> = blockers
            .iter()
            .map(|(bid, btitle)| json!({"id": bid, "title": btitle}))
            .collect();
        out.insert("open_blockers".into(), Value::Array(list));
    }
    if let Some(notes) = notes {
        out.insert("notes".into(), json!(notes));
    }
    out.insert("created_at".into(), json!(created_at));
    out.insert("updated_at".into(), json!(updated_at));
    Ok(Value::Object(out))
}

fn insert_one(conn: &Connection, list: &lists::ListRow, item: &NewItem) -> Result<i64> {
    let title = clean_title(&item.title)?;
    let notes = clean_notes(item.notes.as_deref())?;
    let tag_ids = tags::resolve_names(conn, item.tags.as_deref().unwrap_or(&[]))?;
    let blockers = item.blocked_by.as_deref().unwrap_or(&[]);
    for blocker in blockers {
        find(conn, *blocker)?;
    }
    let placement = item.position.map_or(Placement::Bottom, Placement::from);
    let position = place(conn, list.id, placement, None)?;
    conn.execute(
        "INSERT INTO item (list_id, title, notes, position) VALUES (?1, ?2, ?3, ?4)",
        params![list.id, title, notes, position],
    )?;
    let id = conn.last_insert_rowid();
    for tag_id in tag_ids {
        conn.execute("INSERT INTO item_tag VALUES (?1, ?2)", [id, tag_id])?;
    }
    for blocker in blockers {
        conn.execute(
            "INSERT OR IGNORE INTO dependency VALUES (?1, ?2)",
            [id, *blocker],
        )?;
    }
    Ok(id)
}

/// Add items to a list in one step: if any item is invalid, none are added.
pub fn add(conn: &mut Connection, list: &NameOrId, items: &[NewItem]) -> Result<Value> {
    if items.is_empty() {
        return Err(ToolError::new("Pass at least one item to add."));
    }
    if items.len() > MAX_BATCH {
        return Err(ToolError::new(format!(
            "Too many items ({}); add at most {MAX_BATCH} per call.",
            items.len()
        )));
    }
    let list = lists::find(conn, list)?;
    let tx = conn.transaction()?;
    let mut ids = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let id = insert_one(&tx, &list, item)
            .map_err(|e| ToolError::new(format!("items[{i}]: {e} Nothing was added.")))?;
        ids.push(id);
    }
    tx.commit()?;
    Ok(json!({"list": list.name, "created_ids": ids}))
}

/// Format open blockers for a warning: `#3 'x', #4 'y'`.
fn describe(blockers: &[(i64, String)]) -> String {
    blockers
        .iter()
        .map(|(id, title)| format!("#{id} '{title}'"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn update(
    conn: &mut Connection,
    id: i64,
    title: Option<&str>,
    notes: Option<&str>,
    status: Option<Status>,
    tag_names: Option<&[String]>,
) -> Result<Value> {
    if title.is_none() && notes.is_none() && status.is_none() && tag_names.is_none() {
        return Err(ToolError::new(
            "Nothing to update: pass title, notes, status and/or tags.",
        ));
    }
    find(conn, id)?;
    let tx = conn.transaction()?;
    if let Some(title) = title {
        tx.execute(
            "UPDATE item SET title = ?1 WHERE id = ?2",
            params![clean_title(title)?, id],
        )?;
    }
    if let Some(notes) = notes {
        tx.execute(
            "UPDATE item SET notes = ?1 WHERE id = ?2",
            params![clean_notes(Some(notes))?, id],
        )?;
    }
    if let Some(status) = status {
        tx.execute(
            "UPDATE item SET status = ?1 WHERE id = ?2",
            params![status.as_str(), id],
        )?;
    }
    if let Some(names) = tag_names {
        let tag_ids = tags::resolve_names(&tx, names)?;
        tx.execute("DELETE FROM item_tag WHERE item_id = ?1", [id])?;
        for tag_id in tag_ids {
            tx.execute("INSERT INTO item_tag VALUES (?1, ?2)", [id, tag_id])?;
        }
    }
    tx.execute(
        &format!("UPDATE item SET updated_at = {NOW} WHERE id = ?1"),
        [id],
    )?;
    tx.commit()?;

    let mut out = json!({"item": view(conn, id, true)?});
    if status == Some(Status::Done) {
        let blockers = open_blockers(conn, id)?;
        if !blockers.is_empty() {
            out["warnings"] = json!([format!(
                "Marked done while blocked by open items: {}.",
                describe(&blockers)
            )]);
        }
    }
    Ok(out)
}

pub fn complete(conn: &mut Connection, id: i64) -> Result<Value> {
    update(conn, id, None, None, Some(Status::Done), None)
}

/// Items that depend on `id`: (id, title).
pub fn dependents(conn: &Connection, id: i64) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare(
        "SELECT d.id, d.title FROM dependency dep
           JOIN item d ON d.id = dep.item_id
          WHERE dep.blocked_by_item_id = ?1 ORDER BY d.id",
    )?;
    let rows = stmt
        .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

pub fn delete(conn: &mut Connection, id: i64) -> Result<Value> {
    let item = find(conn, id)?;
    let blocked = dependents(conn, id)?;
    if !blocked.is_empty() {
        let shown: Vec<(i64, String)> = blocked.iter().take(10).cloned().collect();
        let more = if blocked.len() > 10 {
            format!(", and {} more", blocked.len() - 10)
        } else {
            String::new()
        };
        return Err(ToolError::new(format!(
            "Can't delete item #{id} '{}': it blocks {}{more}. Remove those dependencies first \
             (remove_dependency), or complete or cancel the item instead.",
            item.title,
            describe(&shown)
        )));
    }
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM dependency WHERE item_id = ?1", [id])?;
    tx.execute("DELETE FROM item WHERE id = ?1", [id])?; // item_tag cascades
    tx.commit()?;
    Ok(json!({"deleted": {"id": id, "title": item.title}}))
}

pub fn move_to(
    conn: &mut Connection,
    id: i64,
    to_list: &NameOrId,
    position: Option<Position>,
) -> Result<Value> {
    let item = find(conn, id)?;
    let list = lists::find(conn, to_list)?;
    let placement = position.map_or(Placement::Bottom, Placement::from);
    let tx = conn.transaction()?;
    let pos = place(&tx, list.id, placement, Some(item.id))?;
    tx.execute(
        &format!("UPDATE item SET list_id = ?1, position = ?2, updated_at = {NOW} WHERE id = ?3"),
        params![list.id, pos, id],
    )?;
    tx.commit()?;
    Ok(json!({"item": view(conn, id, true)?}))
}

/// Change an item's place within its own list. Pass exactly one of
/// `after_item` or `to`.
pub fn reorder(
    conn: &mut Connection,
    id: i64,
    after_item: Option<i64>,
    to: Option<Edge>,
) -> Result<Value> {
    let placement = match (after_item, to) {
        (Some(after), None) => Placement::After(after),
        (None, Some(Edge::Top)) => Placement::Top,
        (None, Some(Edge::Bottom)) => Placement::Bottom,
        _ => {
            return Err(ToolError::new(
                "Pass exactly one of after_item (an item ID in the same list) or to (\"top\" or \"bottom\").",
            ));
        }
    };
    let item = find(conn, id)?;
    let tx = conn.transaction()?;
    let pos = place(&tx, item.list_id, placement, Some(id))?;
    tx.execute(
        &format!("UPDATE item SET position = ?1, updated_at = {NOW} WHERE id = ?2"),
        params![pos, id],
    )?;
    tx.commit()?;
    Ok(json!({"item": view(conn, id, true)?}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::store::position::{After, Edge};

    fn name(s: &str) -> NameOrId {
        NameOrId::Name(s.into())
    }

    fn setup() -> Connection {
        let mut conn = open_in_memory().unwrap();
        lists::create(&mut conn, "inbox", None).unwrap();
        lists::create(&mut conn, "work", None).unwrap();
        conn
    }

    fn new(title: &str) -> NewItem {
        NewItem {
            title: title.into(),
            notes: None,
            tags: None,
            position: None,
            blocked_by: None,
        }
    }

    fn add_one(conn: &mut Connection, list: &str, item: NewItem) -> i64 {
        add(conn, &name(list), &[item]).unwrap()["created_ids"][0]
            .as_i64()
            .unwrap()
    }

    fn titles(conn: &Connection, list: &str) -> Vec<String> {
        let id = lists::find(conn, &name(list)).unwrap().id;
        let mut stmt = conn
            .prepare("SELECT title FROM item WHERE list_id = ?1 ORDER BY position, id")
            .unwrap();
        stmt.query_map([id], |r| r.get(0))
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect()
    }

    #[test]
    fn add_returns_ids_in_order_and_appends_to_the_bottom() {
        let mut conn = setup();
        let v = add(&mut conn, &name("inbox"), &[new("a"), new("b"), new("c")]).unwrap();
        assert_eq!(v["created_ids"], json!([1, 2, 3]));
        assert_eq!(v["list"], "inbox");
        assert_eq!(titles(&conn, "inbox"), ["a", "b", "c"]);
        // IDs are global across lists
        assert_eq!(add_one(&mut conn, "work", new("d")), 4);
    }

    #[test]
    fn add_is_not_idempotent_and_does_not_dedupe() {
        let mut conn = setup();
        add_one(&mut conn, "inbox", new("same"));
        add_one(&mut conn, "inbox", new("same"));
        assert_eq!(titles(&conn, "inbox"), ["same", "same"]);
    }

    #[test]
    fn add_respects_positions() {
        let mut conn = setup();
        let a = add_one(&mut conn, "inbox", new("a"));
        add_one(&mut conn, "inbox", new("b"));
        let mut top = new("top");
        top.position = Some(Position::Edge(Edge::Top));
        add_one(&mut conn, "inbox", top);
        let mut after = new("after-a");
        after.position = Some(Position::After(After { after: a }));
        add_one(&mut conn, "inbox", after);
        assert_eq!(titles(&conn, "inbox"), ["top", "a", "after-a", "b"]);
    }

    #[test]
    fn add_stores_notes_tags_and_blockers() {
        let mut conn = setup();
        tags::create(&mut conn, "bug", None).unwrap();
        let blocker = add_one(&mut conn, "work", new("build"));
        let mut item = new("ship");
        item.notes = Some("  see **docs**  ".into());
        item.tags = Some(vec!["BUG".into()]);
        item.blocked_by = Some(vec![blocker]);
        let id = add_one(&mut conn, "inbox", item);

        let v = view(&conn, id, true).unwrap();
        assert_eq!(v["list"], "inbox");
        assert_eq!(v["tags"], json!(["bug"]));
        assert_eq!(v["notes"], "see **docs**");
        assert_eq!(v["blocked"], true);
        assert_eq!(
            v["open_blockers"],
            json!([{"id": blocker, "title": "build"}])
        );
    }

    #[test]
    fn add_is_all_or_nothing_and_names_the_bad_item() {
        let mut conn = setup();
        let mut bad = new("second");
        bad.tags = Some(vec!["nope".into()]);
        let err = add(
            &mut conn,
            &name("inbox"),
            &[new("first"), bad, new("third")],
        )
        .unwrap_err()
        .to_string();
        assert!(err.starts_with("items[1]:"), "{err}");
        assert!(err.contains("Unknown tag 'nope'"), "{err}");
        assert!(err.contains("Nothing was added"), "{err}");
        assert!(titles(&conn, "inbox").is_empty());
    }

    #[test]
    fn add_validates_titles_notes_blockers_and_batch_size() {
        let mut conn = setup();
        assert!(add(&mut conn, &name("inbox"), &[]).is_err());
        assert!(add(&mut conn, &name("inbox"), &[new("  ")]).is_err());
        assert!(add(&mut conn, &name("inbox"), &[new("a\nb")]).is_err());
        assert!(add(&mut conn, &name("inbox"), &[new(&"x".repeat(501))]).is_err());

        let mut long = new("long notes");
        long.notes = Some("x".repeat(MAX_NOTES_CHARS + 1));
        let err = add(&mut conn, &name("inbox"), &[long])
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("limited to 4000 characters (got 4001)"),
            "{err}"
        );

        let mut ok = new("max notes");
        ok.notes = Some("x".repeat(MAX_NOTES_CHARS));
        add(&mut conn, &name("inbox"), &[ok]).unwrap();

        let mut blocked = new("blocked");
        blocked.blocked_by = Some(vec![999]);
        let err = add(&mut conn, &name("inbox"), &[blocked])
            .unwrap_err()
            .to_string();
        assert!(err.contains("No item with id 999"), "{err}");

        let many: Vec<NewItem> = (0..=MAX_BATCH).map(|i| new(&format!("i{i}"))).collect();
        assert!(add(&mut conn, &name("inbox"), &many).is_err());
        assert!(add(&mut conn, &name("nope"), &[new("x")]).is_err());
        assert_eq!(titles(&conn, "inbox"), ["max notes"]);
    }

    #[test]
    fn update_changes_fields_and_replaces_tags() {
        let mut conn = setup();
        tags::create(&mut conn, "a", None).unwrap();
        tags::create(&mut conn, "b", None).unwrap();
        let mut item = new("old");
        item.tags = Some(vec!["a".into()]);
        let id = add_one(&mut conn, "inbox", item);

        let tags_b = ["b".to_string()];
        let v = update(
            &mut conn,
            id,
            Some("new"),
            Some("n"),
            Some(Status::Doing),
            Some(&tags_b),
        )
        .unwrap();
        assert_eq!(v["item"]["title"], "new");
        assert_eq!(v["item"]["notes"], "n");
        assert_eq!(v["item"]["status"], "doing");
        assert_eq!(v["item"]["tags"], json!(["b"]));
        assert!(v.get("warnings").is_none());

        // clearing notes and tags
        let v = update(&mut conn, id, None, Some(""), None, Some(&[])).unwrap();
        assert!(v["item"].get("notes").is_none());
        assert!(v["item"].get("tags").is_none());

        assert!(update(&mut conn, id, None, None, None, None).is_err());
        assert!(update(&mut conn, 99, Some("x"), None, None, None).is_err());
    }

    #[test]
    fn update_with_unknown_tag_changes_nothing() {
        let mut conn = setup();
        let id = add_one(&mut conn, "inbox", new("keep"));
        let bad = ["nope".to_string()];
        let err = update(&mut conn, id, Some("changed"), None, None, Some(&bad))
            .unwrap_err()
            .to_string();
        assert!(err.contains("create_tag"), "{err}");
        assert_eq!(titles(&conn, "inbox"), ["keep"]);
    }

    #[test]
    fn completing_a_blocked_item_succeeds_with_a_warning() {
        let mut conn = setup();
        let blocker = add_one(&mut conn, "work", new("blocker"));
        let mut item = new("blocked");
        item.blocked_by = Some(vec![blocker]);
        let id = add_one(&mut conn, "inbox", item);

        let v = complete(&mut conn, id).unwrap();
        assert_eq!(v["item"]["status"], "done");
        let warning = v["warnings"][0].as_str().unwrap();
        assert!(
            warning.contains(&format!("#{blocker} 'blocker'")),
            "{warning}"
        );

        // once the blocker is done, the item is no longer blocked and nothing warns
        complete(&mut conn, blocker).unwrap();
        let v = complete(&mut conn, id).unwrap();
        assert_eq!(v["item"]["blocked"], false);
        assert!(v.get("warnings").is_none());
    }

    #[test]
    fn cancelled_blockers_do_not_block() {
        let mut conn = setup();
        let blocker = add_one(&mut conn, "work", new("blocker"));
        let mut item = new("waiting");
        item.blocked_by = Some(vec![blocker]);
        let id = add_one(&mut conn, "inbox", item);
        assert_eq!(view(&conn, id, false).unwrap()["blocked"], true);
        update(
            &mut conn,
            blocker,
            None,
            None,
            Some(Status::Cancelled),
            None,
        )
        .unwrap();
        assert_eq!(view(&conn, id, false).unwrap()["blocked"], false);
    }

    #[test]
    fn delete_removes_the_item_its_tags_and_its_own_dependencies() {
        let mut conn = setup();
        tags::create(&mut conn, "t", None).unwrap();
        let blocker = add_one(&mut conn, "work", new("blocker"));
        let mut item = new("doomed");
        item.tags = Some(vec!["t".into()]);
        item.blocked_by = Some(vec![blocker]);
        let id = add_one(&mut conn, "inbox", item);

        let v = delete(&mut conn, id).unwrap();
        assert_eq!(v["deleted"]["title"], "doomed");
        assert!(find(&conn, id).is_err());
        let rows: i64 = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM item_tag) + (SELECT COUNT(*) FROM dependency)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 0);
        // the blocker is untouched
        assert!(find(&conn, blocker).is_ok());
    }

    #[test]
    fn delete_is_refused_when_the_item_blocks_others() {
        let mut conn = setup();
        let blocker = add_one(&mut conn, "work", new("blocker"));
        let mut a = new("needs blocker");
        a.blocked_by = Some(vec![blocker]);
        let a = add_one(&mut conn, "inbox", a);

        let err = delete(&mut conn, blocker).unwrap_err().to_string();
        assert!(err.contains(&format!("#{a} 'needs blocker'")), "{err}");
        assert!(err.contains("remove_dependency"), "{err}");
        assert!(find(&conn, blocker).is_ok());

        // a finished blocker still can't be deleted: the link is structural
        complete(&mut conn, blocker).unwrap();
        assert!(delete(&mut conn, blocker).is_err());
    }

    #[test]
    fn move_changes_list_and_defaults_to_the_bottom() {
        let mut conn = setup();
        let a = add_one(&mut conn, "inbox", new("a"));
        add_one(&mut conn, "work", new("w1"));
        add_one(&mut conn, "work", new("w2"));

        let v = move_to(&mut conn, a, &name("work"), None).unwrap();
        assert_eq!(v["item"]["list"], "work");
        assert_eq!(titles(&conn, "work"), ["w1", "w2", "a"]);
        assert!(titles(&conn, "inbox").is_empty());

        move_to(
            &mut conn,
            a,
            &name("inbox"),
            Some(Position::Edge(Edge::Top)),
        )
        .unwrap();
        assert_eq!(titles(&conn, "inbox"), ["a"]);
    }

    #[test]
    fn move_within_the_same_list_reorders() {
        let mut conn = setup();
        let a = add_one(&mut conn, "inbox", new("a"));
        let b = add_one(&mut conn, "inbox", new("b"));
        add_one(&mut conn, "inbox", new("c"));
        move_to(&mut conn, a, &name("inbox"), None).unwrap();
        assert_eq!(titles(&conn, "inbox"), ["b", "c", "a"]);
        move_to(
            &mut conn,
            a,
            &name("inbox"),
            Some(Position::After(After { after: b })),
        )
        .unwrap();
        assert_eq!(titles(&conn, "inbox"), ["b", "a", "c"]);
    }

    #[test]
    fn moving_keeps_dependencies_across_lists() {
        let mut conn = setup();
        let blocker = add_one(&mut conn, "inbox", new("blocker"));
        let mut item = new("waiting");
        item.blocked_by = Some(vec![blocker]);
        let id = add_one(&mut conn, "inbox", item);
        move_to(&mut conn, blocker, &name("work"), None).unwrap();
        assert_eq!(view(&conn, id, false).unwrap()["blocked"], true);
    }

    #[test]
    fn move_errors() {
        let mut conn = setup();
        let a = add_one(&mut conn, "inbox", new("a"));
        assert!(move_to(&mut conn, a, &name("nope"), None).is_err());
        assert!(move_to(&mut conn, 99, &name("work"), None).is_err());
        // an "after" anchor in another list is refused, and nothing moves
        let other = add_one(&mut conn, "inbox", new("other"));
        let err = move_to(
            &mut conn,
            a,
            &name("work"),
            Some(Position::After(After { after: other })),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("isn't in list 'work'"), "{err}");
        assert_eq!(titles(&conn, "inbox"), ["a", "other"]);
    }

    fn order_ids(conn: &Connection, list: &str) -> Vec<i64> {
        let id = lists::find(conn, &name(list)).unwrap().id;
        let mut stmt = conn
            .prepare("SELECT id FROM item WHERE list_id = ?1 ORDER BY position, id")
            .unwrap();
        stmt.query_map([id], |r| r.get(0))
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect()
    }

    #[test]
    fn reorder_to_top_bottom_and_after() {
        let mut conn = setup();
        let ids: Vec<i64> = ["a", "b", "c", "d"]
            .iter()
            .map(|t| add_one(&mut conn, "inbox", new(t)))
            .collect();
        reorder(&mut conn, ids[3], None, Some(Edge::Top)).unwrap();
        assert_eq!(titles(&conn, "inbox"), ["d", "a", "b", "c"]);
        reorder(&mut conn, ids[0], None, Some(Edge::Bottom)).unwrap();
        assert_eq!(titles(&conn, "inbox"), ["d", "b", "c", "a"]);
        let v = reorder(&mut conn, ids[0], Some(ids[3]), None).unwrap();
        assert_eq!(titles(&conn, "inbox"), ["d", "a", "b", "c"]);
        assert_eq!(v["item"]["title"], "a");
        // moving an item to where it already is changes nothing
        reorder(&mut conn, ids[0], Some(ids[3]), None).unwrap();
        assert_eq!(titles(&conn, "inbox"), ["d", "a", "b", "c"]);
        // sending the item already at the bottom to the bottom changes nothing
        reorder(&mut conn, ids[2], None, Some(Edge::Bottom)).unwrap();
        assert_eq!(titles(&conn, "inbox"), ["d", "a", "b", "c"]);
        reorder(&mut conn, ids[1], None, Some(Edge::Bottom)).unwrap();
        assert_eq!(titles(&conn, "inbox"), ["d", "a", "c", "b"]);
    }

    #[test]
    fn reorder_requires_exactly_one_target() {
        let mut conn = setup();
        let a = add_one(&mut conn, "inbox", new("a"));
        let b = add_one(&mut conn, "inbox", new("b"));
        for (after, to) in [(None, None), (Some(b), Some(Edge::Top))] {
            let err = reorder(&mut conn, a, after, to).unwrap_err().to_string();
            assert!(err.contains("exactly one of after_item"), "{err}");
        }
        assert!(reorder(&mut conn, 99, None, Some(Edge::Top)).is_err());
    }

    #[test]
    fn reorder_rejects_itself_and_items_from_other_lists() {
        let mut conn = setup();
        let a = add_one(&mut conn, "inbox", new("a"));
        let w = add_one(&mut conn, "work", new("w"));
        let err = reorder(&mut conn, a, Some(a), None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("after itself"), "{err}");
        let err = reorder(&mut conn, a, Some(w), None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("isn't in list 'inbox'"), "{err}");
        assert!(err.contains("move_item"), "{err}");
        assert_eq!(titles(&conn, "inbox"), ["a"]);
    }

    /// A tiny deterministic generator, so this test needs no extra crate.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n as u64) as usize
        }
    }

    #[test]
    fn random_adds_and_reorders_match_a_simple_model() {
        // Hundreds of operations, many aimed at the same spot so gaps run out
        // and the list has to renumber. The database order must always equal
        // a plain Vec that applies the same operations.
        let mut conn = setup();
        let mut model: Vec<i64> = Vec::new();
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);

        // Where an operation puts its item, given the other items' order.
        // Every third "after" aims at the first item so its gap keeps shrinking.
        let pick = |rng: &mut Rng, others: &[i64]| -> Placement {
            match rng.next(3) {
                0 => Placement::Top,
                1 => Placement::Bottom,
                _ if others.is_empty() => Placement::Bottom,
                _ if rng.next(3) == 0 => Placement::After(others[0]),
                _ => Placement::After(others[rng.next(others.len())]),
            }
        };
        let apply = |model: &mut Vec<i64>, id: i64, placement: Placement| match placement {
            Placement::Top => model.insert(0, id),
            Placement::Bottom => model.push(id),
            Placement::After(after) => {
                let at = model.iter().position(|m| *m == after).unwrap() + 1;
                model.insert(at, id);
            }
        };

        for step in 0..400 {
            if !model.is_empty() && rng.next(2) == 0 {
                // reorder an existing item
                let id = model[rng.next(model.len())];
                let others: Vec<i64> = model.iter().copied().filter(|m| *m != id).collect();
                let placement = pick(&mut rng, &others);
                match placement {
                    Placement::Top => reorder(&mut conn, id, None, Some(Edge::Top)),
                    Placement::Bottom => reorder(&mut conn, id, None, Some(Edge::Bottom)),
                    Placement::After(a) => reorder(&mut conn, id, Some(a), None),
                }
                .unwrap();
                model.retain(|m| *m != id);
                apply(&mut model, id, placement);
            } else {
                // add a new item
                let placement = pick(&mut rng, &model);
                let mut item = new(&format!("n{step}"));
                item.position = Some(match placement {
                    Placement::Top => Position::Edge(Edge::Top),
                    Placement::Bottom => Position::Edge(Edge::Bottom),
                    Placement::After(after) => Position::After(After { after }),
                });
                let id = add_one(&mut conn, "inbox", item);
                apply(&mut model, id, placement);
            }
            if step % 20 == 0 {
                assert_eq!(order_ids(&conn, "inbox"), model, "diverged at step {step}");
            }
        }
        assert_eq!(order_ids(&conn, "inbox"), model);
        let distinct: i64 = conn
            .query_row("SELECT COUNT(DISTINCT position) FROM item", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(distinct as usize, model.len(), "positions stay unique");
    }
}
