//! Export everything to JSON or markdown. Backups are manual in v1: this is
//! the way to get your data out.

use crate::error::Result;
use rusqlite::Connection;
use serde_json::{Map, Value, json};

const STATUS_MARKS: [(&str, &str); 4] = [
    ("todo", "[ ]"),
    ("doing", "[~]"),
    ("done", "[x]"),
    ("cancelled", "[-]"),
];

/// (id, title, notes, status, position, created_at, updated_at, due_date)
type RawItem = (
    i64,
    String,
    Option<String>,
    String,
    i64,
    String,
    String,
    Option<String>,
);

struct ItemRow {
    id: i64,
    title: String,
    notes: Option<String>,
    due_date: Option<String>,
    status: String,
    position: i64,
    created_at: String,
    updated_at: String,
    tags: Vec<String>,
    blocked_by: Vec<i64>,
}

struct ListRow {
    id: i64,
    name: String,
    instructions: Option<String>,
    created_at: String,
    updated_at: String,
    items: Vec<ItemRow>,
}

struct TagRow {
    id: i64,
    name: String,
    instructions: Option<String>,
}

struct Snapshot {
    exported_at: String,
    lists: Vec<ListRow>,
    tags: Vec<TagRow>,
    /// (item, blocked_by)
    dependencies: Vec<(i64, i64)>,
}

fn snapshot(conn: &Connection) -> Result<Snapshot> {
    let exported_at: String =
        conn.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |r| {
            r.get(0)
        })?;

    let mut stmt = conn.prepare("SELECT id, name, instructions FROM tag ORDER BY id")?;
    let tags: Vec<TagRow> = stmt
        .query_map([], |r| {
            Ok(TagRow {
                id: r.get(0)?,
                name: r.get(1)?,
                instructions: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;

    let mut stmt =
        conn.prepare("SELECT item_id, blocked_by_item_id FROM dependency ORDER BY 1, 2")?;
    let dependencies: Vec<(i64, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;

    let mut tag_stmt = conn.prepare(
        "SELECT t.name FROM item_tag it JOIN tag t ON t.id = it.tag_id
          WHERE it.item_id = ?1 ORDER BY t.name COLLATE NOCASE",
    )?;
    let mut item_stmt = conn.prepare(
        "SELECT id, title, notes, status, position, created_at, updated_at, due_date
           FROM item WHERE list_id = ?1 ORDER BY position, id",
    )?;
    let mut list_stmt = conn
        .prepare("SELECT id, name, instructions, created_at, updated_at FROM list ORDER BY id")?;
    let list_heads: Vec<(i64, String, Option<String>, String, String)> = list_stmt
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<rusqlite::Result<_>>()?;

    let mut lists = Vec::new();
    for (id, name, instructions, created_at, updated_at) in list_heads {
        let rows: Vec<RawItem> = item_stmt
            .query_map([id], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut items = Vec::new();
        for (item_id, title, notes, status, position, created, updated, due_date) in rows {
            let tags: Vec<String> = tag_stmt
                .query_map([item_id], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            let blocked_by = dependencies
                .iter()
                .filter(|(i, _)| *i == item_id)
                .map(|(_, b)| *b)
                .collect();
            items.push(ItemRow {
                id: item_id,
                title,
                notes,
                due_date,
                status,
                position,
                created_at: created,
                updated_at: updated,
                tags,
                blocked_by,
            });
        }
        lists.push(ListRow {
            id,
            name,
            instructions,
            created_at,
            updated_at,
            items,
        });
    }
    Ok(Snapshot {
        exported_at,
        lists,
        tags,
        dependencies,
    })
}

fn opt(map: &mut Map<String, Value>, key: &str, value: &Option<String>) {
    if let Some(v) = value {
        map.insert(key.into(), json!(v));
    }
}

/// Everything in the database as one JSON document. Items are in list order.
pub fn to_json(conn: &Connection) -> Result<Value> {
    let snap = snapshot(conn)?;
    let lists: Vec<Value> = snap
        .lists
        .iter()
        .map(|l| {
            let items: Vec<Value> = l
                .items
                .iter()
                .map(|i| {
                    let mut m = Map::new();
                    m.insert("id".into(), json!(i.id));
                    m.insert("title".into(), json!(i.title));
                    m.insert("status".into(), json!(i.status));
                    m.insert("position".into(), json!(i.position));
                    m.insert("tags".into(), json!(i.tags));
                    m.insert("blocked_by".into(), json!(i.blocked_by));
                    opt(&mut m, "due_date", &i.due_date);
                    opt(&mut m, "notes", &i.notes);
                    m.insert("created_at".into(), json!(i.created_at));
                    m.insert("updated_at".into(), json!(i.updated_at));
                    Value::Object(m)
                })
                .collect();
            let mut m = Map::new();
            m.insert("id".into(), json!(l.id));
            m.insert("name".into(), json!(l.name));
            opt(&mut m, "instructions", &l.instructions);
            m.insert("created_at".into(), json!(l.created_at));
            m.insert("updated_at".into(), json!(l.updated_at));
            m.insert("items".into(), Value::Array(items));
            Value::Object(m)
        })
        .collect();
    let tags: Vec<Value> = snap
        .tags
        .iter()
        .map(|t| {
            let mut m = Map::new();
            m.insert("id".into(), json!(t.id));
            m.insert("name".into(), json!(t.name));
            opt(&mut m, "instructions", &t.instructions);
            Value::Object(m)
        })
        .collect();
    let dependencies: Vec<Value> = snap
        .dependencies
        .iter()
        .map(|(item, blocker)| json!({"item": item, "blocked_by": blocker}))
        .collect();
    Ok(json!({
        "format": "tally-export",
        "version": 1,
        "exported_at": snap.exported_at,
        "lists": lists,
        "tags": tags,
        "dependencies": dependencies,
    }))
}

fn quote(text: &str, indent: &str) -> String {
    text.lines()
        .map(|l| {
            if l.is_empty() {
                format!("{indent}>")
            } else {
                format!("{indent}> {l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Everything in the database as readable markdown. Items are in list order.
pub fn to_markdown(conn: &Connection) -> Result<String> {
    let snap = snapshot(conn)?;
    let status_of: std::collections::HashMap<i64, (&str, &str)> = snap
        .lists
        .iter()
        .flat_map(|l| {
            l.items
                .iter()
                .map(|i| (i.id, (i.title.as_str(), i.status.as_str())))
        })
        .collect();

    let mut out = String::new();
    out.push_str("# Tally export\n\n");
    out.push_str(&format!("Exported {}\n", snap.exported_at));

    out.push_str("\n## Tags\n\n");
    if snap.tags.is_empty() {
        out.push_str("_No tags._\n");
    }
    for t in &snap.tags {
        out.push_str(&format!("- **{}**\n", t.name));
        if let Some(text) = &t.instructions {
            out.push_str(&quote(text, "  "));
            out.push('\n');
        }
    }

    if snap.lists.is_empty() {
        out.push_str("\n## Lists\n\n_No lists._\n");
    }
    for l in &snap.lists {
        out.push_str(&format!("\n## {}\n\n", l.name));
        if let Some(text) = &l.instructions {
            out.push_str("Instructions:\n\n");
            out.push_str(&quote(text, ""));
            out.push_str("\n\n");
        }
        if l.items.is_empty() {
            out.push_str("_No items._\n");
        }
        for i in &l.items {
            let mark = STATUS_MARKS
                .iter()
                .find(|(s, _)| *s == i.status)
                .map_or("[ ]", |(_, m)| m);
            out.push_str(&format!("- {mark} **#{}** {}", i.id, i.title));
            if i.status == "doing" || i.status == "cancelled" {
                out.push_str(&format!(" ({})", i.status));
            }
            if let Some(due) = &i.due_date {
                out.push_str(&format!(" — due {due}"));
            }
            if !i.tags.is_empty() {
                out.push_str(&format!(" — tags: {}", i.tags.join(", ")));
            }
            out.push('\n');
            if !i.blocked_by.is_empty() {
                let blockers: Vec<String> = i
                    .blocked_by
                    .iter()
                    .map(|b| match status_of.get(b) {
                        Some((title, status)) if *status == "done" || *status == "cancelled" => {
                            format!("#{b} {title} ({status})")
                        }
                        Some((title, _)) => format!("#{b} {title}"),
                        None => format!("#{b}"),
                    })
                    .collect();
                out.push_str(&format!("  - blocked by: {}\n", blockers.join("; ")));
            }
            if let Some(notes) = &i.notes {
                out.push_str(&quote(notes, "  "));
                out.push('\n');
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::store::items::{NewItem, Status};
    use crate::store::refs::NameOrId;
    use crate::store::{dependencies, items, lists, tags};

    fn name(s: &str) -> NameOrId {
        NameOrId::Name(s.into())
    }

    fn sample() -> Connection {
        let mut conn = open_in_memory().unwrap();
        lists::create(
            &mut conn,
            "launch",
            Some("Titles start with a verb.\nKeep it short."),
        )
        .unwrap();
        lists::create(&mut conn, "empty", None).unwrap();
        tags::create(&mut conn, "critical", Some("Do these first")).unwrap();
        tags::create(&mut conn, "plain", None).unwrap();
        let item = |title: &str, notes: Option<&str>, tags: &[&str]| NewItem {
            title: title.into(),
            notes: notes.map(Into::into),
            due_date: None,
            tags: Some(tags.iter().map(|t| t.to_string()).collect()),
            position: None,
            blocked_by: None,
        };
        items::add(
            &mut conn,
            &name("launch"),
            &[
                item("Write notes", Some("line one\n\nline three"), &[]),
                item("Cut branch", None, &["critical", "plain"]),
                item("Publish", None, &[]),
            ],
        )
        .unwrap();
        dependencies::add(&mut conn, 3, 2).unwrap();
        items::update(&mut conn, 1, None, None, Some(Status::Doing), None, None).unwrap();
        conn
    }

    #[test]
    fn json_has_everything_in_list_order() {
        let mut conn = sample();
        items::move_to(
            &mut conn,
            3,
            Some(&name("launch")),
            Some(crate::store::position::Position::Edge(
                crate::store::position::Edge::Top,
            )),
        )
        .unwrap();
        let v = to_json(&conn).unwrap();
        assert_eq!(v["format"], "tally-export");
        assert_eq!(v["version"], 1);
        assert!(v["exported_at"].as_str().unwrap().ends_with('Z'));

        assert_eq!(v["lists"].as_array().unwrap().len(), 2);
        let launch = &v["lists"][0];
        assert_eq!(launch["name"], "launch");
        assert_eq!(
            launch["instructions"],
            "Titles start with a verb.\nKeep it short."
        );
        let order: Vec<&str> = launch["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["title"].as_str().unwrap())
            .collect();
        assert_eq!(order, ["Publish", "Write notes", "Cut branch"]);

        let publish = &launch["items"][0];
        assert_eq!(publish["blocked_by"], json!([2]));
        let branch = &launch["items"][2];
        assert_eq!(branch["tags"], json!(["critical", "plain"]));
        assert_eq!(launch["items"][1]["status"], "doing");
        assert_eq!(launch["items"][1]["notes"], "line one\n\nline three");
        assert!(launch["items"][1]["created_at"].is_string());

        assert_eq!(v["tags"][0]["instructions"], "Do these first");
        assert!(v["tags"][1].get("instructions").is_none());
        assert_eq!(v["dependencies"], json!([{"item": 3, "blocked_by": 2}]));
        assert_eq!(v["lists"][1]["items"], json!([]));
    }

    #[test]
    fn json_of_an_empty_database_is_valid() {
        let conn = open_in_memory().unwrap();
        let v = to_json(&conn).unwrap();
        assert_eq!(v["lists"], json!([]));
        assert_eq!(v["tags"], json!([]));
        assert_eq!(v["dependencies"], json!([]));
    }

    #[test]
    fn markdown_is_readable_and_complete() {
        let mut conn = sample();
        items::complete(&mut conn, 2).unwrap();
        let md = to_markdown(&conn).unwrap();

        assert!(md.starts_with("# Tally export\n"), "{md}");
        assert!(md.contains("## Tags"), "{md}");
        assert!(md.contains("- **critical**\n  > Do these first"), "{md}");
        assert!(md.contains("## launch"), "{md}");
        assert!(
            md.contains("> Titles start with a verb.\n> Keep it short."),
            "{md}"
        );
        assert!(md.contains("- [~] **#1** Write notes (doing)"), "{md}");
        assert!(
            md.contains("- [x] **#2** Cut branch — tags: critical, plain"),
            "{md}"
        );
        assert!(
            md.contains("- [ ] **#3** Publish\n  - blocked by: #2 Cut branch (done)"),
            "{md}"
        );
        // multi-line notes stay inside their item, with blank lines preserved
        assert!(md.contains("  > line one\n  >\n  > line three"), "{md}");
        assert!(md.contains("## empty\n\n_No items._"), "{md}");
    }

    #[test]
    fn markdown_of_an_empty_database_says_so() {
        let conn = open_in_memory().unwrap();
        let md = to_markdown(&conn).unwrap();
        assert!(md.contains("_No tags._"), "{md}");
        assert!(md.contains("_No lists._"), "{md}");
    }
}
