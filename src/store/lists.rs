//! List operations: create, list, update, delete.

use super::refs::{NameOrId, summarize};
use super::{NOW, clean_instructions, clean_name};
use crate::error::{Result, ToolError};
use crate::page;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub struct ListRow {
    pub id: i64,
    pub name: String,
    pub instructions: Option<String>,
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ListRow> {
    Ok(ListRow {
        id: r.get(0)?,
        name: r.get(1)?,
        instructions: r.get(2)?,
    })
}

fn by_id(conn: &Connection, id: i64) -> Result<Option<ListRow>> {
    Ok(conn
        .query_row(
            "SELECT id, name, instructions FROM list WHERE id = ?1",
            [id],
            row,
        )
        .optional()?)
}

fn by_name(conn: &Connection, name: &str) -> Result<Option<ListRow>> {
    Ok(conn
        .query_row(
            "SELECT id, name, instructions FROM list WHERE name = ?1 COLLATE NOCASE",
            [name],
            row,
        )
        .optional()?)
}

/// Resolve a list reference. Names match case-insensitively. A numeric string
/// that is not a list name is treated as an ID, since agents often send IDs as text.
pub fn find(conn: &Connection, r: &NameOrId) -> Result<ListRow> {
    let found = match r {
        NameOrId::Id(id) => by_id(conn, *id)?,
        NameOrId::Name(name) => {
            let name = name.trim();
            match by_name(conn, name)? {
                Some(list) => Some(list),
                None => match name.parse::<i64>() {
                    Ok(id) => by_id(conn, id)?,
                    Err(_) => None,
                },
            }
        }
    };
    match found {
        Some(list) => Ok(list),
        None => Err(not_found(conn, r)?),
    }
}

fn not_found(conn: &Connection, r: &NameOrId) -> Result<ToolError> {
    let what = match r {
        NameOrId::Id(id) => format!("No list with id {id}."),
        NameOrId::Name(name) => format!("No list named '{}'.", name.trim()),
    };
    let mut stmt = conn.prepare("SELECT name FROM list ORDER BY id LIMIT 21")?;
    let names: Vec<String> = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(ToolError::new(if names.is_empty() {
        format!("{what} There are no lists yet; create one with create_list.")
    } else {
        format!(
            "{what} Existing lists: {}. Use create_list to make a new one.",
            summarize(&names, 20)
        )
    }))
}

fn view(list: &ListRow) -> Value {
    json!({
        "id": list.id,
        "name": list.name,
        "has_instructions": list.instructions.is_some(),
    })
}

pub fn create(conn: &mut Connection, name: &str, instructions: Option<&str>) -> Result<Value> {
    let name = clean_name(name, "list")?;
    let instructions = clean_instructions(instructions)?;
    if let Some(existing) = by_name(conn, &name)? {
        return Err(ToolError::new(format!(
            "A list named '{}' already exists (id {}).",
            existing.name, existing.id
        )));
    }
    conn.execute(
        "INSERT INTO list (name, instructions) VALUES (?1, ?2)",
        params![name, instructions],
    )?;
    let id = conn.last_insert_rowid();
    Ok(view(&ListRow {
        id,
        name,
        instructions,
    }))
}

pub fn list(conn: &Connection, cursor: Option<&str>, limit: Option<u32>) -> Result<Value> {
    let limit = page::limit(limit);
    let after = match cursor {
        Some(c) => page::parse_id(page::decode(c, 1)?[0])?,
        None => 0,
    };
    let mut stmt = conn.prepare(
        "SELECT l.id, l.name, l.instructions IS NOT NULL,
                (SELECT COUNT(*) FROM item i
                  WHERE i.list_id = l.id AND i.status IN ('todo', 'doing')),
                (SELECT COUNT(*) FROM item i WHERE i.list_id = l.id)
           FROM list l WHERE l.id > ?1 ORDER BY l.id LIMIT ?2",
    )?;
    let mut rows: Vec<(i64, Value)> = stmt
        .query_map(params![after, limit + 1], |r| {
            let id: i64 = r.get(0)?;
            Ok((
                id,
                json!({
                    "id": id,
                    "name": r.get::<_, String>(1)?,
                    "has_instructions": r.get::<_, bool>(2)?,
                    "open_items": r.get::<_, i64>(3)?,
                    "total_items": r.get::<_, i64>(4)?,
                }),
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let has_more = rows.len() as i64 > limit;
    rows.truncate(limit as usize);
    let next_cursor = match (has_more, rows.last()) {
        (true, Some((id, _))) => Some(page::encode(&[id.to_string()])),
        _ => None,
    };
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM list", [], |r| r.get(0))?;
    Ok(json!({
        "lists": rows.into_iter().map(|(_, v)| v).collect::<Vec<_>>(),
        "next_cursor": next_cursor,
        "total_count": total,
    }))
}

pub fn update(
    conn: &mut Connection,
    list: &NameOrId,
    name: Option<&str>,
    instructions: Option<&str>,
) -> Result<Value> {
    if name.is_none() && instructions.is_none() {
        return Err(ToolError::new(
            "Nothing to update: pass name and/or instructions.",
        ));
    }
    let current = find(conn, list)?;
    let new_name = match name {
        Some(n) => {
            let n = clean_name(n, "list")?;
            if let Some(other) = by_name(conn, &n)?
                && other.id != current.id
            {
                return Err(ToolError::new(format!(
                    "A list named '{}' already exists (id {}).",
                    other.name, other.id
                )));
            }
            n
        }
        None => current.name.clone(),
    };
    // Passing empty instructions clears them.
    let new_instructions = match instructions {
        Some(text) => clean_instructions(Some(text))?,
        None => current.instructions.clone(),
    };
    conn.execute(
        &format!("UPDATE list SET name = ?1, instructions = ?2, updated_at = {NOW} WHERE id = ?3"),
        params![new_name, new_instructions, current.id],
    )?;
    Ok(view(&ListRow {
        id: current.id,
        name: new_name,
        instructions: new_instructions,
    }))
}

/// Items in `list_id` that block items in other lists: (blocker, blocked, blocked's list).
fn cross_list_blockers(conn: &Connection, list_id: i64) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT b.id, b.title, d.id, d.title, dl.name
           FROM dependency dep
           JOIN item b ON b.id = dep.blocked_by_item_id
           JOIN item d ON d.id = dep.item_id
           JOIN list dl ON dl.id = d.list_id
          WHERE b.list_id = ?1 AND d.list_id <> ?1
          ORDER BY b.id, d.id",
    )?;
    let lines = stmt
        .query_map([list_id], |r| {
            Ok(format!(
                "#{} '{}' blocks #{} '{}' (list '{}')",
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(lines)
}

pub fn delete(conn: &mut Connection, list: &NameOrId) -> Result<Value> {
    let current = find(conn, list)?;
    let blockers = cross_list_blockers(conn, current.id)?;
    if !blockers.is_empty() {
        let shown: Vec<String> = blockers.iter().take(10).map(|l| format!("- {l}")).collect();
        let more = if blockers.len() > 10 {
            format!("\n- and {} more", blockers.len() - 10)
        } else {
            String::new()
        };
        return Err(ToolError::new(format!(
            "Can't delete list '{}': {} of its items block items in other lists. \
             Remove these dependencies first (remove_dependency), or move or delete the blocked items:\n{}{}",
            current.name,
            blockers.len(),
            shown.join("\n"),
            more
        )));
    }
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM dependency
          WHERE item_id IN (SELECT id FROM item WHERE list_id = ?1)
             OR blocked_by_item_id IN (SELECT id FROM item WHERE list_id = ?1)",
        [current.id],
    )?;
    let items_deleted = tx.execute("DELETE FROM item WHERE list_id = ?1", [current.id])?;
    tx.execute("DELETE FROM list WHERE id = ?1", [current.id])?;
    tx.commit()?;
    Ok(json!({
        "deleted": {"id": current.id, "name": current.name},
        "items_deleted": items_deleted,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    fn name(s: &str) -> NameOrId {
        NameOrId::Name(s.into())
    }

    fn add_item(conn: &Connection, list_id: i64, title: &str) -> i64 {
        conn.execute(
            "INSERT INTO item (list_id, title, position) VALUES (?1, ?2, 1000)",
            params![list_id, title],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn create_and_find_by_name_id_and_numeric_string() {
        let mut conn = open_in_memory().unwrap();
        let v = create(&mut conn, "  Groceries ", Some("Keep it short")).unwrap();
        assert_eq!(v["id"], 1);
        assert_eq!(v["name"], "Groceries");
        assert_eq!(v["has_instructions"], true);
        assert_eq!(find(&conn, &name("groceries")).unwrap().id, 1);
        assert_eq!(find(&conn, &NameOrId::Id(1)).unwrap().id, 1);
        assert_eq!(find(&conn, &name("1")).unwrap().id, 1);
        assert_eq!(
            find(&conn, &name("Groceries"))
                .unwrap()
                .instructions
                .as_deref(),
            Some("Keep it short")
        );
    }

    #[test]
    fn a_list_literally_named_with_digits_wins_over_id() {
        let mut conn = open_in_memory().unwrap();
        create(&mut conn, "first", None).unwrap(); // id 1
        create(&mut conn, "1", None).unwrap(); // id 2, named "1"
        assert_eq!(find(&conn, &name("1")).unwrap().id, 2);
        assert_eq!(find(&conn, &NameOrId::Id(1)).unwrap().id, 1);
    }

    #[test]
    fn duplicate_names_are_rejected_case_insensitively() {
        let mut conn = open_in_memory().unwrap();
        create(&mut conn, "Work", None).unwrap();
        let err = create(&mut conn, "work", None).unwrap_err().to_string();
        assert!(err.contains("already exists"), "{err}");
        assert!(err.contains("id 1"), "{err}");
    }

    #[test]
    fn invalid_names_are_rejected() {
        let mut conn = open_in_memory().unwrap();
        assert!(create(&mut conn, "   ", None).is_err());
        assert!(create(&mut conn, "a\nb", None).is_err());
        assert!(create(&mut conn, &"x".repeat(101), None).is_err());
        assert!(create(&mut conn, "ok", Some(&"x".repeat(10_001))).is_err());
    }

    #[test]
    fn unknown_list_error_names_existing_lists() {
        let mut conn = open_in_memory().unwrap();
        let err = find(&conn, &name("nope")).unwrap_err().to_string();
        assert!(err.contains("create_list"), "{err}");
        create(&mut conn, "Alpha", None).unwrap();
        let err = find(&conn, &name("nope")).unwrap_err().to_string();
        assert!(err.contains("'Alpha'"), "{err}");
        assert!(find(&conn, &NameOrId::Id(99)).is_err());
    }

    #[test]
    fn update_renames_and_sets_and_clears_instructions() {
        let mut conn = open_in_memory().unwrap();
        create(&mut conn, "a", None).unwrap();
        create(&mut conn, "b", None).unwrap();

        let v = update(&mut conn, &name("a"), Some("renamed"), Some("do x")).unwrap();
        assert_eq!(v["name"], "renamed");
        assert_eq!(v["has_instructions"], true);

        // renaming to itself with different case is fine
        update(&mut conn, &name("renamed"), Some("Renamed"), None).unwrap();
        // instructions survive a rename-only update
        assert_eq!(
            find(&conn, &NameOrId::Id(1))
                .unwrap()
                .instructions
                .as_deref(),
            Some("do x")
        );
        // renaming onto another list is refused
        let err = update(&mut conn, &NameOrId::Id(1), Some("B"), None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("already exists"), "{err}");
        // empty instructions clear them
        let v = update(&mut conn, &NameOrId::Id(1), None, Some("")).unwrap();
        assert_eq!(v["has_instructions"], false);
        // no fields is an error
        assert!(update(&mut conn, &NameOrId::Id(1), None, None).is_err());
    }

    #[test]
    fn list_lists_reports_counts_and_paginates() {
        let mut conn = open_in_memory().unwrap();
        for i in 1..=5 {
            create(&mut conn, &format!("list{i}"), None).unwrap();
        }
        let item = add_item(&conn, 1, "open");
        add_item(&conn, 1, "also open");
        conn.execute("UPDATE item SET status = 'done' WHERE id = ?1", [item])
            .unwrap();

        let first = list(&conn, None, Some(2)).unwrap();
        assert_eq!(first["total_count"], 5);
        assert_eq!(first["lists"].as_array().unwrap().len(), 2);
        assert_eq!(first["lists"][0]["open_items"], 1);
        assert_eq!(first["lists"][0]["total_items"], 2);
        let c1 = first["next_cursor"].as_str().unwrap().to_string();

        let second = list(&conn, Some(&c1), Some(2)).unwrap();
        assert_eq!(second["lists"][0]["name"], "list3");
        let c2 = second["next_cursor"].as_str().unwrap().to_string();

        let third = list(&conn, Some(&c2), Some(2)).unwrap();
        assert_eq!(third["lists"].as_array().unwrap().len(), 1);
        assert!(third["next_cursor"].is_null());
    }

    #[test]
    fn pagination_survives_creates_and_deletes_between_pages() {
        let mut conn = open_in_memory().unwrap();
        for i in 1..=4 {
            create(&mut conn, &format!("l{i}"), None).unwrap();
        }
        let first = list(&conn, None, Some(2)).unwrap();
        let cursor = first["next_cursor"].as_str().unwrap().to_string();
        // delete one already-seen list and add a new one before reading page two
        delete(&mut conn, &name("l1")).unwrap();
        create(&mut conn, "l5", None).unwrap();
        let second = list(&conn, Some(&cursor), Some(10)).unwrap();
        let names: Vec<&str> = second["lists"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["l3", "l4", "l5"]);
    }

    #[test]
    fn bad_cursor_is_an_error() {
        let conn = open_in_memory().unwrap();
        assert!(list(&conn, Some("garbage"), None).is_err());
    }

    #[test]
    fn delete_removes_the_list_its_items_and_internal_dependencies() {
        let mut conn = open_in_memory().unwrap();
        create(&mut conn, "a", None).unwrap();
        create(&mut conn, "b", None).unwrap();
        let a1 = add_item(&conn, 1, "a1");
        let a2 = add_item(&conn, 1, "a2");
        let b1 = add_item(&conn, 2, "b1");
        // a2 depends on a1 (same list); a1 depends on b1 (a blocker in another list)
        conn.execute("INSERT INTO dependency VALUES (?1, ?2)", [a2, a1])
            .unwrap();
        conn.execute("INSERT INTO dependency VALUES (?1, ?2)", [a1, b1])
            .unwrap();

        let v = delete(&mut conn, &name("a")).unwrap();
        assert_eq!(v["items_deleted"], 2);
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM item", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 1);
        let deps: i64 = conn
            .query_row("SELECT COUNT(*) FROM dependency", [], |r| r.get(0))
            .unwrap();
        assert_eq!(deps, 0);
        assert!(find(&conn, &name("a")).is_err());
    }

    #[test]
    fn delete_is_refused_when_items_block_items_in_other_lists() {
        let mut conn = open_in_memory().unwrap();
        create(&mut conn, "ops", None).unwrap();
        create(&mut conn, "launch", None).unwrap();
        let deploy = add_item(&conn, 1, "Deploy");
        let release = add_item(&conn, 2, "Release");
        conn.execute("INSERT INTO dependency VALUES (?1, ?2)", [release, deploy])
            .unwrap();

        let err = delete(&mut conn, &name("ops")).unwrap_err().to_string();
        assert!(
            err.contains("#1 'Deploy' blocks #2 'Release' (list 'launch')"),
            "{err}"
        );
        // nothing was deleted
        assert!(find(&conn, &name("ops")).is_ok());

        // the other direction (launch is only blocked, blocks nothing) is allowed
        delete(&mut conn, &name("launch")).unwrap();
    }
}
