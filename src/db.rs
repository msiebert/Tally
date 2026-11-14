//! SQLite access: opening the database and running migrations.

use anyhow::{Context, Result, bail};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// Migrations are append-only. `PRAGMA user_version` records how many have run.
const MIGRATIONS: &[&str] = &[include_str!("../migrations/001_init.sql")];

/// `TALLY_DB` if set and non-empty, otherwise `~/.local/share/tally/tally.db`.
pub fn default_db_path() -> PathBuf {
    match std::env::var_os("TALLY_DB") {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".local/share/tally/tally.db"),
    }
}

/// Open (creating if needed) the database at `path` and bring it up to date.
pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating database directory {}", parent.display()))?;
    }
    let mut conn =
        Connection::open(path).with_context(|| format!("opening database {}", path.display()))?;
    conn.execute_batch("PRAGMA journal_mode = WAL;")?;
    prepare(&mut conn)?;
    Ok(conn)
}

/// A fresh in-memory database, for tests.
pub fn open_in_memory() -> Result<Connection> {
    let mut conn = Connection::open_in_memory()?;
    prepare(&mut conn)?;
    Ok(conn)
}

fn prepare(conn: &mut Connection) -> Result<()> {
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    migrate(conn)
}

/// Run any migrations that have not been applied yet, each in its own transaction.
pub fn migrate(conn: &mut Connection) -> Result<()> {
    let applied: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let applied = usize::try_from(applied).unwrap_or(0);
    if applied > MIGRATIONS.len() {
        bail!(
            "database schema version {applied} is newer than this build supports ({})",
            MIGRATIONS.len()
        );
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(applied) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)
            .with_context(|| format!("running migration {}", i + 1))?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_names(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' \
                 AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn migrations_create_all_tables() {
        let conn = open_in_memory().unwrap();
        assert_eq!(
            table_names(&conn),
            ["dependency", "item", "item_tag", "list", "tag"]
        );
    }

    #[test]
    fn migrations_are_idempotent() {
        let mut conn = open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let v: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v as usize, MIGRATIONS.len());
    }

    #[test]
    fn newer_database_is_rejected() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 999).unwrap();
        assert!(migrate(&mut conn).is_err());
    }

    #[test]
    fn list_names_are_unique() {
        let conn = open_in_memory().unwrap();
        conn.execute("INSERT INTO list (name) VALUES ('a')", [])
            .unwrap();
        assert!(
            conn.execute("INSERT INTO list (name) VALUES ('a')", [])
                .is_err()
        );
    }

    #[test]
    fn ids_are_separate_sequences_and_not_reused() {
        let conn = open_in_memory().unwrap();
        conn.execute("INSERT INTO list (name) VALUES ('a')", [])
            .unwrap();
        conn.execute("INSERT INTO tag (name) VALUES ('t')", [])
            .unwrap();
        conn.execute(
            "INSERT INTO item (list_id, title, position) VALUES (1, 'x', 1000)",
            [],
        )
        .unwrap();
        conn.execute("DELETE FROM item WHERE id = 1", []).unwrap();
        conn.execute(
            "INSERT INTO item (list_id, title, position) VALUES (1, 'y', 1000)",
            [],
        )
        .unwrap();
        let item_id: i64 = conn
            .query_row("SELECT id FROM item", [], |r| r.get(0))
            .unwrap();
        let tag_id: i64 = conn
            .query_row("SELECT id FROM tag", [], |r| r.get(0))
            .unwrap();
        assert_eq!(item_id, 2);
        assert_eq!(tag_id, 1);
    }

    #[test]
    fn foreign_keys_and_checks_are_enforced() {
        let conn = open_in_memory().unwrap();
        // item needs an existing list
        assert!(
            conn.execute(
                "INSERT INTO item (list_id, title, position) VALUES (9, 'x', 1)",
                []
            )
            .is_err()
        );
        conn.execute("INSERT INTO list (name) VALUES ('a')", [])
            .unwrap();
        // status must be one of the four values
        assert!(
            conn.execute(
                "INSERT INTO item (list_id, title, status, position) VALUES (1, 'x', 'bogus', 1)",
                []
            )
            .is_err()
        );
        // an item cannot block itself
        conn.execute(
            "INSERT INTO item (list_id, title, position) VALUES (1, 'x', 1)",
            [],
        )
        .unwrap();
        assert!(
            conn.execute("INSERT INTO dependency VALUES (1, 1)", [])
                .is_err()
        );
    }

    #[test]
    fn open_creates_parent_directories() {
        let dir = std::env::temp_dir().join(format!("tally-test-{}", std::process::id()));
        let path = dir.join("nested/tally.db");
        let conn = open(&path).unwrap();
        drop(conn);
        assert!(path.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
