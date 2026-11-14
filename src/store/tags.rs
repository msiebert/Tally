//! Tag operations. Tags must be created before use, which stops typos from
//! quietly splitting one tag into several.

use super::refs::{NameOrId, summarize};
use super::{NOW, clean_instructions, clean_name};
use crate::error::{Result, ToolError};
use crate::page;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub struct TagRow {
    pub id: i64,
    pub name: String,
    pub instructions: Option<String>,
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<TagRow> {
    Ok(TagRow {
        id: r.get(0)?,
        name: r.get(1)?,
        instructions: r.get(2)?,
    })
}

fn by_id(conn: &Connection, id: i64) -> Result<Option<TagRow>> {
    Ok(conn
        .query_row(
            "SELECT id, name, instructions FROM tag WHERE id = ?1",
            [id],
            row,
        )
        .optional()?)
}

pub fn by_name(conn: &Connection, name: &str) -> Result<Option<TagRow>> {
    Ok(conn
        .query_row(
            "SELECT id, name, instructions FROM tag WHERE name = ?1 COLLATE NOCASE",
            [name],
            row,
        )
        .optional()?)
}

/// Up to `max + 1` tag names, in creation order, for error messages.
fn existing_names(conn: &Connection, max: usize) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT name FROM tag ORDER BY id LIMIT ?1")?;
    let names = stmt
        .query_map([max as i64 + 1], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(names)
}

fn existing_hint(conn: &Connection) -> Result<String> {
    let names = existing_names(conn, 30)?;
    Ok(if names.is_empty() {
        "There are no tags yet.".to_string()
    } else {
        format!("Existing tags: {}.", summarize(&names, 30))
    })
}

/// Resolve a tag reference by name (case-insensitive) or ID.
pub fn find(conn: &Connection, r: &NameOrId) -> Result<TagRow> {
    let found = match r {
        NameOrId::Id(id) => by_id(conn, *id)?,
        NameOrId::Name(name) => {
            let name = name.trim();
            match by_name(conn, name)? {
                Some(tag) => Some(tag),
                None => match name.parse::<i64>() {
                    Ok(id) => by_id(conn, id)?,
                    Err(_) => None,
                },
            }
        }
    };
    match found {
        Some(tag) => Ok(tag),
        None => {
            let what = match r {
                NameOrId::Id(id) => format!("No tag with id {id}."),
                NameOrId::Name(name) => format!("No tag named '{}'.", name.trim()),
            };
            Err(ToolError::new(format!(
                "{what} {} Create it first with create_tag.",
                existing_hint(conn)?
            )))
        }
    }
}

/// Resolve tag names used on items. Every name must already exist; if any do
/// not, the error lists them all along with the tags that do exist.
/// Duplicates (ignoring case) are collapsed. Returns tag IDs.
pub fn resolve_names(conn: &Connection, names: &[String]) -> Result<Vec<i64>> {
    let mut ids = Vec::new();
    let mut unknown: Vec<String> = Vec::new();
    for name in names {
        let name = name.trim();
        match by_name(conn, name)? {
            Some(tag) => {
                if !ids.contains(&tag.id) {
                    ids.push(tag.id);
                }
            }
            None => {
                if !unknown.iter().any(|u| u.eq_ignore_ascii_case(name)) {
                    unknown.push(name.to_string());
                }
            }
        }
    }
    if unknown.is_empty() {
        return Ok(ids);
    }
    Err(ToolError::new(format!(
        "Unknown tag{} {}. Tags must be created before use: call create_tag first, \
         or fix the spelling. {}",
        if unknown.len() == 1 { "" } else { "s" },
        summarize(&unknown, 10),
        existing_hint(conn)?
    )))
}

fn view(tag: &TagRow) -> Value {
    json!({
        "id": tag.id,
        "name": tag.name,
        "has_instructions": tag.instructions.is_some(),
    })
}

pub fn create(conn: &mut Connection, name: &str, instructions: Option<&str>) -> Result<Value> {
    let name = clean_name(name, "tag")?;
    let instructions = clean_instructions(instructions)?;
    if let Some(existing) = by_name(conn, &name)? {
        return Err(ToolError::new(format!(
            "A tag named '{}' already exists (id {}).",
            existing.name, existing.id
        )));
    }
    conn.execute(
        "INSERT INTO tag (name, instructions) VALUES (?1, ?2)",
        params![name, instructions],
    )?;
    let id = conn.last_insert_rowid();
    Ok(view(&TagRow {
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
        "SELECT t.id, t.name, t.instructions IS NOT NULL,
                (SELECT COUNT(*) FROM item_tag it WHERE it.tag_id = t.id)
           FROM tag t WHERE t.id > ?1 ORDER BY t.id LIMIT ?2",
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
                    "item_count": r.get::<_, i64>(3)?,
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
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM tag", [], |r| r.get(0))?;
    Ok(json!({
        "tags": rows.into_iter().map(|(_, v)| v).collect::<Vec<_>>(),
        "next_cursor": next_cursor,
        "total_count": total,
    }))
}

pub fn update(
    conn: &mut Connection,
    tag: &NameOrId,
    name: Option<&str>,
    instructions: Option<&str>,
) -> Result<Value> {
    if name.is_none() && instructions.is_none() {
        return Err(ToolError::new(
            "Nothing to update: pass name and/or instructions.",
        ));
    }
    let current = find(conn, tag)?;
    let new_name = match name {
        Some(n) => {
            let n = clean_name(n, "tag")?;
            if let Some(other) = by_name(conn, &n)?
                && other.id != current.id
            {
                return Err(ToolError::new(format!(
                    "A tag named '{}' already exists (id {}).",
                    other.name, other.id
                )));
            }
            n
        }
        None => current.name.clone(),
    };
    let new_instructions = match instructions {
        Some(text) => clean_instructions(Some(text))?,
        None => current.instructions.clone(),
    };
    conn.execute(
        "UPDATE tag SET name = ?1, instructions = ?2 WHERE id = ?3",
        params![new_name, new_instructions, current.id],
    )?;
    // Renaming a tag changes how its items read, so refresh their timestamps.
    conn.execute(
        &format!(
            "UPDATE item SET updated_at = {NOW}
              WHERE id IN (SELECT item_id FROM item_tag WHERE tag_id = ?1)"
        ),
        [current.id],
    )?;
    Ok(view(&TagRow {
        id: current.id,
        name: new_name,
        instructions: new_instructions,
    }))
}

pub fn delete(conn: &mut Connection, tag: &NameOrId) -> Result<Value> {
    let current = find(conn, tag)?;
    let in_use: i64 = conn.query_row(
        "SELECT COUNT(*) FROM item_tag WHERE tag_id = ?1",
        [current.id],
        |r| r.get(0),
    )?;
    if in_use > 0 {
        return Err(ToolError::new(format!(
            "Can't delete tag '{}': it is used by {in_use} item{}. Remove it from those items first \
             (update_item with the tags you want to keep; search_items with tag '{}' finds them).",
            current.name,
            if in_use == 1 { "" } else { "s" },
            current.name
        )));
    }
    conn.execute("DELETE FROM tag WHERE id = ?1", [current.id])?;
    Ok(json!({"deleted": {"id": current.id, "name": current.name}}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    fn name(s: &str) -> NameOrId {
        NameOrId::Name(s.into())
    }

    fn use_tag(conn: &Connection, tag_id: i64) {
        conn.execute(
            "INSERT INTO list (name) VALUES ('l') ON CONFLICT DO NOTHING",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO item (list_id, title, position) VALUES (1, 'x', 1000)",
            [],
        )
        .unwrap();
        let item = conn.last_insert_rowid();
        conn.execute("INSERT INTO item_tag VALUES (?1, ?2)", [item, tag_id])
            .unwrap();
    }

    #[test]
    fn create_find_and_reject_duplicates() {
        let mut conn = open_in_memory().unwrap();
        let v = create(&mut conn, " Urgent ", Some("Do these first")).unwrap();
        assert_eq!(v["name"], "Urgent");
        assert_eq!(v["has_instructions"], true);
        assert_eq!(find(&conn, &name("urgent")).unwrap().id, 1);
        assert_eq!(find(&conn, &NameOrId::Id(1)).unwrap().id, 1);
        let err = create(&mut conn, "URGENT", None).unwrap_err().to_string();
        assert!(err.contains("already exists"), "{err}");
        assert!(create(&mut conn, "", None).is_err());
    }

    #[test]
    fn unknown_tag_errors_say_how_to_proceed() {
        let mut conn = open_in_memory().unwrap();
        let err = find(&conn, &name("x")).unwrap_err().to_string();
        assert!(err.contains("There are no tags yet"), "{err}");
        assert!(err.contains("create_tag"), "{err}");
        create(&mut conn, "bug", None).unwrap();
        let err = find(&conn, &name("x")).unwrap_err().to_string();
        assert!(err.contains("'bug'"), "{err}");
    }

    #[test]
    fn resolve_names_requires_existing_tags_and_lists_them() {
        let mut conn = open_in_memory().unwrap();
        create(&mut conn, "bug", None).unwrap();
        create(&mut conn, "idea", None).unwrap();

        let ids = resolve_names(&conn, &["BUG".into(), "bug".into(), "idea".into()]).unwrap();
        assert_eq!(ids, [1, 2], "duplicates ignoring case collapse");

        let err = resolve_names(&conn, &["bug".into(), "bugg".into(), "urgnt".into()])
            .unwrap_err()
            .to_string();
        assert!(err.contains("Unknown tags 'bugg', 'urgnt'"), "{err}");
        assert!(err.contains("create_tag"), "{err}");
        assert!(err.contains("Existing tags: 'bug', 'idea'"), "{err}");

        let err = resolve_names(&conn, &["nope".into()])
            .unwrap_err()
            .to_string();
        assert!(err.contains("Unknown tag 'nope'"), "{err}");

        assert!(resolve_names(&conn, &[]).unwrap().is_empty());
    }

    #[test]
    fn update_renames_and_manages_instructions() {
        let mut conn = open_in_memory().unwrap();
        create(&mut conn, "a", None).unwrap();
        create(&mut conn, "b", Some("keep")).unwrap();
        let v = update(&mut conn, &name("a"), Some("alpha"), Some("first")).unwrap();
        assert_eq!(v["name"], "alpha");
        assert_eq!(v["has_instructions"], true);
        let err = update(&mut conn, &name("alpha"), Some("B"), None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("already exists"), "{err}");
        let v = update(&mut conn, &name("b"), None, Some("")).unwrap();
        assert_eq!(v["has_instructions"], false);
        assert!(update(&mut conn, &name("b"), None, None).is_err());
    }

    #[test]
    fn delete_is_refused_while_the_tag_is_in_use() {
        let mut conn = open_in_memory().unwrap();
        create(&mut conn, "bug", None).unwrap();
        use_tag(&conn, 1);
        use_tag(&conn, 1);
        let err = delete(&mut conn, &name("bug")).unwrap_err().to_string();
        assert!(err.contains("used by 2 items"), "{err}");
        assert!(find(&conn, &name("bug")).is_ok());

        conn.execute("DELETE FROM item_tag", []).unwrap();
        let v = delete(&mut conn, &name("bug")).unwrap();
        assert_eq!(v["deleted"]["name"], "bug");
        assert!(find(&conn, &name("bug")).is_err());
    }

    #[test]
    fn list_tags_counts_uses_and_paginates() {
        let mut conn = open_in_memory().unwrap();
        for n in ["a", "b", "c"] {
            create(&mut conn, n, None).unwrap();
        }
        use_tag(&conn, 2);
        let first = list(&conn, None, Some(2)).unwrap();
        assert_eq!(first["total_count"], 3);
        assert_eq!(first["tags"][1]["item_count"], 1);
        let cursor = first["next_cursor"].as_str().unwrap().to_string();
        let second = list(&conn, Some(&cursor), Some(2)).unwrap();
        assert_eq!(second["tags"].as_array().unwrap().len(), 1);
        assert!(second["next_cursor"].is_null());
    }
}
