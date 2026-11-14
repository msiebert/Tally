//! Tool parameter types. Doc comments become the descriptions agents see in the
//! tool schemas, so they are written for agents and kept short.

use crate::store::refs::NameOrId;
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateList {
    /// Unique list name (case-insensitive). You refer to the list by this name afterwards.
    pub name: String,
    /// Optional guidance for agents that use this list.
    pub instructions: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListLists {
    /// The next_cursor from the previous page. Omit for the first page.
    pub cursor: Option<String>,
    /// Page size (default 50, max 200).
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateList {
    /// The list's name or numeric ID.
    pub list: NameOrId,
    /// New name.
    pub name: Option<String>,
    /// New instructions. An empty string clears them.
    pub instructions: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteList {
    /// The list's name or numeric ID.
    pub list: NameOrId,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateTag {
    /// Unique tag name (case-insensitive).
    pub name: String,
    /// Optional guidance for agents on when and how to use items with this tag.
    pub instructions: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListTags {
    /// The next_cursor from the previous page. Omit for the first page.
    pub cursor: Option<String>,
    /// Page size (default 50, max 200).
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateTag {
    /// The tag's name or numeric ID.
    pub tag: NameOrId,
    /// New name.
    pub name: Option<String>,
    /// New instructions. An empty string clears them.
    pub instructions: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteTag {
    /// The tag's name or numeric ID.
    pub tag: NameOrId,
}
