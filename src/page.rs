//! Cursor pagination helpers.
//!
//! Cursors are opaque to agents. Internally they are the sort key of the last
//! row returned, so edits between pages never skip or repeat rows that were
//! not themselves edited.

use crate::error::{Result, ToolError};

pub const DEFAULT_LIMIT: u32 = 50;
pub const MAX_LIMIT: u32 = 200;

/// The requested page size, defaulting to 50 and capped at 200.
pub fn limit(requested: Option<u32>) -> i64 {
    i64::from(requested.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT))
}

pub fn encode(parts: &[String]) -> String {
    parts.join("|")
}

/// Split a cursor into exactly `n` parts.
pub fn decode(cursor: &str, n: usize) -> Result<Vec<&str>> {
    let parts: Vec<&str> = cursor.splitn(n, '|').collect();
    if parts.len() != n || parts.iter().any(|p| p.is_empty()) {
        return Err(bad_cursor());
    }
    Ok(parts)
}

pub fn parse_id(part: &str) -> Result<i64> {
    part.parse().map_err(|_| bad_cursor())
}

fn bad_cursor() -> ToolError {
    ToolError::new(
        "Invalid cursor. Pass the next_cursor value from the previous response unchanged, \
         or omit it to start from the first page.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_defaults_and_clamps() {
        assert_eq!(limit(None), 50);
        assert_eq!(limit(Some(0)), 1);
        assert_eq!(limit(Some(7)), 7);
        assert_eq!(limit(Some(10_000)), 200);
    }

    #[test]
    fn cursor_round_trip() {
        let c = encode(&["2026-01-01T00:00:00.000Z".to_string(), "7".to_string()]);
        let parts = decode(&c, 2).unwrap();
        assert_eq!(parts, ["2026-01-01T00:00:00.000Z", "7"]);
        assert_eq!(parse_id(parts[1]).unwrap(), 7);
    }

    #[test]
    fn malformed_cursors_are_rejected() {
        assert!(decode("", 1).is_err());
        assert!(decode("a", 2).is_err());
        assert!(decode("a|", 2).is_err());
        assert!(parse_id("x").is_err());
    }
}
