//! Storage operations. Each function takes a connection and returns the JSON
//! shown to the agent, or a [`ToolError`] explaining what to fix.

pub mod dependencies;
pub mod export;
pub mod items;
pub mod lists;
pub mod position;
pub mod reads;
pub mod refs;
pub mod tags;

use crate::error::{Result, ToolError};

/// SQL expression for the current UTC time in the format stored in the database.
pub const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";

pub const MAX_NAME_CHARS: usize = 100;
pub const MAX_INSTRUCTIONS_CHARS: usize = 10_000;

/// Trim and validate a list or tag name.
pub fn clean_name(raw: &str, what: &str) -> Result<String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(ToolError::new(format!("A {what} name can't be empty.")));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(ToolError::new(format!(
            "A {what} name can be at most {MAX_NAME_CHARS} characters."
        )));
    }
    if name.contains(['\n', '\r']) {
        return Err(ToolError::new(format!(
            "A {what} name must be a single line."
        )));
    }
    Ok(name.to_string())
}

/// Trim instructions. Empty text means "no instructions".
pub fn clean_instructions(raw: Option<&str>) -> Result<Option<String>> {
    let Some(text) = raw.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    if text.chars().count() > MAX_INSTRUCTIONS_CHARS {
        return Err(ToolError::new(format!(
            "Instructions can be at most {MAX_INSTRUCTIONS_CHARS} characters."
        )));
    }
    Ok(Some(text.to_string()))
}
