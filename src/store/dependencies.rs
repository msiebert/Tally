//! Dependencies between items. "Blocked" is never stored: an item is blocked
//! while any of its blockers is neither done nor cancelled (see `items::view`).

use super::NOW;
use super::items::{self, find};
use crate::error::{Result, ToolError};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};

fn label(conn: &Connection, id: i64) -> Result<String> {
    Ok(format!("#{id} '{}'", find(conn, id)?.title))
}

/// A chain of blockers leading from `from` to `to`, if there is one: each item
/// in the chain is blocked by the next. Returns the IDs including both ends.
fn blocker_path(conn: &Connection, from: i64, to: i64) -> Result<Option<Vec<i64>>> {
    let mut stmt =
        conn.prepare("SELECT blocked_by_item_id FROM dependency WHERE item_id = ?1 ORDER BY 1")?;
    let mut parent: HashMap<i64, i64> = HashMap::new();
    let mut queue = VecDeque::from([from]);
    let mut seen = vec![from];
    while let Some(current) = queue.pop_front() {
        if current == to {
            let mut path = vec![to];
            let mut at = to;
            while let Some(&p) = parent.get(&at) {
                path.push(p);
                at = p;
            }
            path.reverse();
            return Ok(Some(path));
        }
        let next: Vec<i64> = stmt
            .query_map([current], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for n in next {
            if !seen.contains(&n) {
                seen.push(n);
                parent.insert(n, current);
                queue.push_back(n);
            }
        }
    }
    Ok(None)
}

fn touch(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        &format!("UPDATE item SET updated_at = {NOW} WHERE id = ?1"),
        [id],
    )?;
    Ok(())
}

/// Make `item` wait for `blocked_by`. Rejected if it would create a cycle.
/// Adding a dependency that already exists is a no-op, not an error.
pub fn add(conn: &mut Connection, item: i64, blocked_by: i64) -> Result<Value> {
    let item_label = label(conn, item)?;
    let blocker_label = label(conn, blocked_by)?;
    if item == blocked_by {
        return Err(ToolError::new(format!(
            "{item_label} can't be blocked by itself."
        )));
    }
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM dependency WHERE item_id = ?1 AND blocked_by_item_id = ?2)",
        [item, blocked_by],
        |r| r.get(0),
    )?;
    if exists {
        return Ok(json!({
            "item": items::view(conn, item, true)?,
            "added": false,
            "note": format!("{item_label} was already blocked by {blocker_label}."),
        }));
    }
    // The new link item -> blocked_by closes a cycle if blocked_by already
    // waits (directly or not) on item.
    if let Some(path) = blocker_path(conn, blocked_by, item)? {
        let mut chain = vec![item];
        chain.extend(path);
        let text = chain
            .iter()
            .map(|id| label(conn, *id))
            .collect::<Result<Vec<_>>>()?
            .join(" → ");
        return Err(ToolError::new(format!(
            "Can't make {item_label} wait for {blocker_label}: that would create a cycle. \
             Cycle (each item is blocked by the next): {text}. \
             Remove one of these dependencies first (remove_dependency)."
        )));
    }
    let tx = conn.transaction()?;
    tx.execute("INSERT INTO dependency VALUES (?1, ?2)", [item, blocked_by])?;
    touch(&tx, item)?;
    tx.commit()?;
    Ok(json!({"item": items::view(conn, item, true)?, "added": true}))
}

pub fn remove(conn: &mut Connection, item: i64, blocked_by: i64) -> Result<Value> {
    let item_label = label(conn, item)?;
    let blocker_label = label(conn, blocked_by)?;
    let removed = conn.execute(
        "DELETE FROM dependency WHERE item_id = ?1 AND blocked_by_item_id = ?2",
        [item, blocked_by],
    )?;
    if removed == 0 {
        let mut stmt = conn.prepare(
            "SELECT b.id, b.title FROM dependency d JOIN item b ON b.id = d.blocked_by_item_id
              WHERE d.item_id = ?1 ORDER BY b.id",
        )?;
        let current: Vec<String> = stmt
            .query_map([item], |r| {
                Ok(format!(
                    "#{} '{}'",
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let hint = if current.is_empty() {
            format!("{item_label} has no blockers.")
        } else {
            format!("{item_label} is blocked by: {}.", current.join(", "))
        };
        return Err(ToolError::new(format!(
            "{item_label} isn't blocked by {blocker_label}. {hint}"
        )));
    }
    touch(conn, item)?;
    Ok(json!({"item": items::view(conn, item, true)?, "removed": true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::store::items::{NewItem, Status};
    use crate::store::lists;
    use crate::store::refs::NameOrId;

    fn setup(n: usize) -> Connection {
        let mut conn = open_in_memory().unwrap();
        lists::create(&mut conn, "a", None).unwrap();
        lists::create(&mut conn, "b", None).unwrap();
        for i in 1..=n {
            // alternate lists so dependencies cross lists
            let list = if i % 2 == 1 { "a" } else { "b" };
            items::add(
                &mut conn,
                &NameOrId::Name(list.into()),
                &[NewItem {
                    title: format!("item{i}"),
                    notes: None,
                    tags: None,
                    position: None,
                    blocked_by: None,
                }],
            )
            .unwrap();
        }
        conn
    }

    #[test]
    fn add_and_remove_update_the_blocked_state() {
        let mut conn = setup(2);
        let v = add(&mut conn, 1, 2).unwrap();
        assert_eq!(v["added"], true);
        assert_eq!(v["item"]["blocked"], true);
        assert_eq!(v["item"]["open_blockers"][0]["title"], "item2");

        let v = remove(&mut conn, 1, 2).unwrap();
        assert_eq!(v["removed"], true);
        assert_eq!(v["item"]["blocked"], false);
    }

    #[test]
    fn blocked_is_computed_from_blocker_status() {
        let mut conn = setup(2);
        add(&mut conn, 1, 2).unwrap();
        items::update(&mut conn, 2, None, None, Some(Status::Doing), None).unwrap();
        assert_eq!(items::view(&conn, 1, false).unwrap()["blocked"], true);
        items::complete(&mut conn, 2).unwrap();
        assert_eq!(items::view(&conn, 1, false).unwrap()["blocked"], false);
        // reopening the blocker blocks the item again
        items::update(&mut conn, 2, None, None, Some(Status::Todo), None).unwrap();
        assert_eq!(items::view(&conn, 1, false).unwrap()["blocked"], true);
    }

    #[test]
    fn adding_twice_is_a_no_op() {
        let mut conn = setup(2);
        add(&mut conn, 1, 2).unwrap();
        let v = add(&mut conn, 1, 2).unwrap();
        assert_eq!(v["added"], false);
        assert!(v["note"].as_str().unwrap().contains("already blocked"));
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM dependency", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn self_dependency_and_unknown_items_are_rejected() {
        let mut conn = setup(2);
        let err = add(&mut conn, 1, 1).unwrap_err().to_string();
        assert!(err.contains("itself"), "{err}");
        assert!(add(&mut conn, 1, 99).is_err());
        assert!(add(&mut conn, 99, 1).is_err());
        assert!(remove(&mut conn, 99, 1).is_err());
    }

    #[test]
    fn direct_cycle_is_rejected_and_named() {
        let mut conn = setup(2);
        add(&mut conn, 1, 2).unwrap();
        let err = add(&mut conn, 2, 1).unwrap_err().to_string();
        assert!(err.contains("would create a cycle"), "{err}");
        assert!(
            err.contains("#2 'item2' → #1 'item1' → #2 'item2'"),
            "{err}"
        );
    }

    #[test]
    fn long_cycle_across_lists_is_rejected_and_names_every_link() {
        let mut conn = setup(4);
        // 1 waits on 2, 2 on 3, 3 on 4. Making 4 wait on 1 closes the loop.
        add(&mut conn, 1, 2).unwrap();
        add(&mut conn, 2, 3).unwrap();
        add(&mut conn, 3, 4).unwrap();
        let err = add(&mut conn, 4, 1).unwrap_err().to_string();
        assert!(
            err.contains("#4 'item4' → #1 'item1' → #2 'item2' → #3 'item3' → #4 'item4'"),
            "{err}"
        );
        // nothing was written
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM dependency", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 3);
    }

    #[test]
    fn cycles_are_found_even_through_finished_items() {
        let mut conn = setup(3);
        add(&mut conn, 1, 2).unwrap();
        add(&mut conn, 2, 3).unwrap();
        items::complete(&mut conn, 2).unwrap();
        assert!(add(&mut conn, 3, 1).is_err());
    }

    #[test]
    fn diamonds_are_not_cycles() {
        let mut conn = setup(4);
        // 1 waits on 2 and 3; both wait on 4
        add(&mut conn, 1, 2).unwrap();
        add(&mut conn, 1, 3).unwrap();
        add(&mut conn, 2, 4).unwrap();
        add(&mut conn, 3, 4).unwrap();
        assert_eq!(
            items::view(&conn, 1, false).unwrap()["open_blockers"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn removing_a_missing_dependency_lists_the_real_blockers() {
        let mut conn = setup(3);
        add(&mut conn, 1, 2).unwrap();
        let err = remove(&mut conn, 1, 3).unwrap_err().to_string();
        assert!(err.contains("isn't blocked by #3 'item3'"), "{err}");
        assert!(err.contains("is blocked by: #2 'item2'"), "{err}");
        let err = remove(&mut conn, 3, 1).unwrap_err().to_string();
        assert!(err.contains("has no blockers"), "{err}");
    }

    #[test]
    fn the_delete_refusal_can_be_resolved_by_removing_the_dependency() {
        let mut conn = setup(2);
        add(&mut conn, 1, 2).unwrap();
        assert!(items::delete(&mut conn, 2).is_err());
        remove(&mut conn, 1, 2).unwrap();
        items::delete(&mut conn, 2).unwrap();
    }
}
