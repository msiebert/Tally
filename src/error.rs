//! Errors that are reported back to the agent as tool errors.
//!
//! Messages are written for an agent: they say what is in the way and how to fix it.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolError(String);

impl ToolError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ToolError {}

impl From<rusqlite::Error> for ToolError {
    fn from(e: rusqlite::Error) -> Self {
        Self(format!("database error: {e}"))
    }
}

pub type Result<T> = std::result::Result<T, ToolError>;
