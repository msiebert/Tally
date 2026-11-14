//! Item order within a list.
//!
//! Positions are integers spaced `GAP` apart. Placing an item between two
//! neighbours takes the midpoint, so a reorder rewrites only one row. When two
//! neighbours have no room between them, the list is renumbered once and the
//! placement retried. Ties are broken by ID, so order is always well defined.

use crate::error::{Result, ToolError};
use rusqlite::{Connection, OptionalExtension};
use schemars::JsonSchema;
use serde::Deserialize;

pub const GAP: i64 = 1000;

/// Where to put an item in a list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Position {
    /// "top" or "bottom".
    Edge(Edge),
    /// Place directly after another item in the same list.
    After(After),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
pub struct After {
    /// ID of the item to place this one after.
    #[serde(deserialize_with = "crate::params::lenient_id")]
    pub after: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Top,
    Bottom,
    After(i64),
}

impl From<Position> for Placement {
    fn from(p: Position) -> Self {
        match p {
            Position::Edge(Edge::Top) => Placement::Top,
            Position::Edge(Edge::Bottom) => Placement::Bottom,
            Position::After(a) => Placement::After(a.after),
        }
    }
}

/// Compute the position value for an item in `list_id`. `moving` is an item
/// already in the table that is about to be (re)placed, so it is ignored when
/// looking at neighbours.
pub fn place(
    conn: &Connection,
    list_id: i64,
    placement: Placement,
    moving: Option<i64>,
) -> Result<i64> {
    let moving = moving.unwrap_or(-1);
    match placement {
        Placement::Bottom => {
            let max: Option<i64> = conn.query_row(
                "SELECT MAX(position) FROM item WHERE list_id = ?1 AND id <> ?2",
                [list_id, moving],
                |r| r.get(0),
            )?;
            Ok(max.map_or(GAP, |m| m + GAP))
        }
        Placement::Top => {
            let min: Option<i64> = conn.query_row(
                "SELECT MIN(position) FROM item WHERE list_id = ?1 AND id <> ?2",
                [list_id, moving],
                |r| r.get(0),
            )?;
            Ok(min.map_or(GAP, |m| m - GAP))
        }
        Placement::After(anchor) => {
            if anchor == moving {
                return Err(ToolError::new("An item can't be placed after itself."));
            }
            place_after(conn, list_id, anchor, moving, true)
        }
    }
}

fn place_after(
    conn: &Connection,
    list_id: i64,
    anchor: i64,
    moving: i64,
    may_renumber: bool,
) -> Result<i64> {
    let anchor_pos: Option<i64> = conn
        .query_row(
            "SELECT position FROM item WHERE id = ?1 AND list_id = ?2",
            [anchor, list_id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(anchor_pos) = anchor_pos else {
        let list_name: String =
            conn.query_row("SELECT name FROM list WHERE id = ?1", [list_id], |r| {
                r.get(0)
            })?;
        return Err(ToolError::new(format!(
            "Can't place after item #{anchor}: it isn't in list '{list_name}'. \
             Use move_item to change an item's list first."
        )));
    };
    let next: Option<i64> = conn.query_row(
        "SELECT MIN(position) FROM item
          WHERE list_id = ?1 AND id <> ?2 AND position > ?3",
        [list_id, moving, anchor_pos],
        |r| r.get(0),
    )?;
    match next {
        None => Ok(anchor_pos + GAP),
        Some(next) if next - anchor_pos >= 2 => Ok(anchor_pos + (next - anchor_pos) / 2),
        Some(_) if may_renumber => {
            renumber(conn, list_id)?;
            place_after(conn, list_id, anchor, moving, false)
        }
        Some(_) => Err(ToolError::new(
            "Couldn't make room in the list order; this is a bug in Tally.",
        )),
    }
}

/// Respace every item in the list `GAP` apart, keeping the current order.
pub fn renumber(conn: &Connection, list_id: i64) -> Result<()> {
    let mut stmt = conn.prepare("SELECT id FROM item WHERE list_id = ?1 ORDER BY position, id")?;
    let ids: Vec<i64> = stmt
        .query_map([list_id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut update = conn.prepare("UPDATE item SET position = ?1 WHERE id = ?2")?;
    for (i, id) in ids.iter().enumerate() {
        update.execute([(i as i64 + 1) * GAP, *id])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    fn setup() -> Connection {
        let conn = open_in_memory().unwrap();
        conn.execute("INSERT INTO list (name) VALUES ('l')", [])
            .unwrap();
        conn
    }

    fn add(conn: &Connection, title: &str, placement: Placement) -> i64 {
        let pos = place(conn, 1, placement, None).unwrap();
        conn.execute(
            "INSERT INTO item (list_id, title, position) VALUES (1, ?1, ?2)",
            rusqlite::params![title, pos],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn order(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT title FROM item WHERE list_id = 1 ORDER BY position, id")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect()
    }

    #[test]
    fn bottom_top_and_after() {
        let conn = setup();
        let a = add(&conn, "a", Placement::Bottom);
        add(&conn, "b", Placement::Bottom);
        add(&conn, "top", Placement::Top);
        add(&conn, "after-a", Placement::After(a));
        assert_eq!(order(&conn), ["top", "a", "after-a", "b"]);
    }

    #[test]
    fn after_the_last_item_appends() {
        let conn = setup();
        let a = add(&conn, "a", Placement::Bottom);
        add(&conn, "z", Placement::After(a));
        assert_eq!(order(&conn), ["a", "z"]);
    }

    #[test]
    fn running_out_of_room_renumbers_and_keeps_order() {
        let conn = setup();
        let first = add(&conn, "first", Placement::Bottom);
        add(&conn, "last", Placement::Bottom);
        // Always inserting right after `first` halves the gap each time.
        for i in 0..40 {
            add(&conn, &format!("n{i}"), Placement::After(first));
        }
        let titles = order(&conn);
        assert_eq!(titles.len(), 42);
        assert_eq!(titles[0], "first");
        assert_eq!(titles[41], "last");
        // newest insert lands right after `first`
        assert_eq!(titles[1], "n39");
        assert_eq!(titles[40], "n0");
        let distinct: i64 = conn
            .query_row("SELECT COUNT(DISTINCT position) FROM item", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(distinct, 42, "positions stay unique");
    }

    #[test]
    fn moving_item_is_ignored_when_finding_neighbours() {
        let conn = setup();
        add(&conn, "a", Placement::Bottom);
        let b = add(&conn, "b", Placement::Bottom);
        // moving b to the bottom of its own list keeps it last
        let pos = place(&conn, 1, Placement::Bottom, Some(b)).unwrap();
        let a_pos: i64 = conn
            .query_row("SELECT position FROM item WHERE title = 'a'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(pos, a_pos + GAP);
        assert!(place(&conn, 1, Placement::After(b), Some(b)).is_err());
    }

    #[test]
    fn anchor_must_be_in_the_same_list() {
        let conn = setup();
        conn.execute("INSERT INTO list (name) VALUES ('other')", [])
            .unwrap();
        conn.execute(
            "INSERT INTO item (list_id, title, position) VALUES (2, 'elsewhere', 1000)",
            [],
        )
        .unwrap();
        let err = place(&conn, 1, Placement::After(1), None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("isn't in list 'l'"), "{err}");
        let err = place(&conn, 1, Placement::After(99), None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("isn't in list"), "{err}");
    }

    #[test]
    fn position_parses_from_agent_json() {
        let top: Position = serde_json::from_str("\"top\"").unwrap();
        let bottom: Position = serde_json::from_str("\"bottom\"").unwrap();
        let after: Position = serde_json::from_str("{\"after\": 7}").unwrap();
        let after_text: Position = serde_json::from_str("{\"after\": \"#7\"}").unwrap();
        assert_eq!(Placement::from(top), Placement::Top);
        assert_eq!(Placement::from(bottom), Placement::Bottom);
        assert_eq!(Placement::from(after), Placement::After(7));
        assert_eq!(Placement::from(after_text), Placement::After(7));
        assert!(serde_json::from_str::<Position>("\"middle\"").is_err());
    }
}
