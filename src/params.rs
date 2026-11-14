//! Tool parameter types. Doc comments become the descriptions agents see in the
//! tool schemas, so they are written for agents and kept short.

use crate::store::items::{NewItem, Status};
use crate::store::position::{Edge, Position};
use crate::store::refs::NameOrId;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, de::Error};

/// Item IDs are integers, but agents sometimes send them as text ("12" or "#12").
pub fn lenient_id<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Num(i64),
        Text(String),
    }
    match Raw::deserialize(d)? {
        Raw::Num(n) => Ok(n),
        Raw::Text(s) => s
            .trim()
            .trim_start_matches('#')
            .parse()
            .map_err(|_| D::Error::custom(format!("expected an item ID, got '{s}'"))),
    }
}

/// Like [`lenient_id`], for an optional list of IDs.
pub fn lenient_ids_opt<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<i64>>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Num(i64),
        Text(String),
    }
    let Some(raw) = Option::<Vec<Raw>>::deserialize(d)? else {
        return Ok(None);
    };
    raw.into_iter()
        .map(|r| match r {
            Raw::Num(n) => Ok(n),
            Raw::Text(s) => s
                .trim()
                .trim_start_matches('#')
                .parse()
                .map_err(|_| D::Error::custom(format!("expected an item ID, got '{s}'"))),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddItems {
    /// The list's name or numeric ID.
    pub list: NameOrId,
    /// Items to add (max 200). All-or-nothing: if any item is invalid, none are added.
    pub items: Vec<NewItem>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateItem {
    /// Item ID.
    #[serde(deserialize_with = "lenient_id")]
    pub item: i64,
    /// New title.
    pub title: Option<String>,
    /// New notes (markdown, max 4,000 characters). An empty string clears them.
    pub notes: Option<String>,
    /// New status.
    pub status: Option<Status>,
    /// Replaces the item's whole tag set; an empty list removes all tags. Tags must already exist.
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ItemRef {
    /// Item ID.
    #[serde(deserialize_with = "lenient_id")]
    pub item: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveItem {
    /// Item ID.
    #[serde(deserialize_with = "lenient_id")]
    pub item: i64,
    /// The destination list's name or numeric ID.
    pub to_list: NameOrId,
    /// Where in the destination list: "top", "bottom" (default), or {"after": item_id}.
    pub position: Option<Position>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Dependency {
    /// The item that has to wait.
    #[serde(deserialize_with = "lenient_id")]
    pub item: i64,
    /// The item it waits for. It can be in any list.
    #[serde(deserialize_with = "lenient_id")]
    pub blocked_by: i64,
}

/// A single value or a list of values.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

impl<T> OneOrMany<T> {
    pub fn into_vec(self) -> Vec<T> {
        match self {
            OneOrMany::One(v) => vec![v],
            OneOrMany::Many(v) => v,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetList {
    /// The list's name or numeric ID.
    pub list: NameOrId,
    /// Show only these statuses (one, or a list). Done items are hidden unless you ask for them here.
    pub status: Option<OneOrMany<Status>>,
    /// Show only items with this tag.
    pub tag: Option<String>,
    /// Only items with no open blockers.
    pub unblocked_only: Option<bool>,
    /// Include the list's instructions (default true). Pass false if you already have them in context.
    pub include_instructions: Option<bool>,
    /// Include instructions for the tags on these items (default true). Pass false if you already have them in context.
    pub include_tag_instructions: Option<bool>,
    /// The next_cursor from the previous page. Omit for the first page.
    pub cursor: Option<String>,
    /// Page size (default 50, max 200).
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchItems {
    /// Case-insensitive text to find in titles and notes.
    pub text: Option<String>,
    /// Only items in this list (name or ID).
    pub list: Option<NameOrId>,
    /// Only items with this tag.
    pub tag: Option<String>,
    /// Only these statuses (one, or a list). By default all statuses are searched, including done.
    pub status: Option<OneOrMany<Status>>,
    /// Only items with no open blockers.
    pub unblocked_only: Option<bool>,
    /// Include instructions for the tags on these items (default true). Pass false if you already have them in context.
    pub include_tag_instructions: Option<bool>,
    /// The next_cursor from the previous page. Omit for the first page.
    pub cursor: Option<String>,
    /// Page size (default 50, max 200).
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReorderItem {
    /// Item ID.
    #[serde(deserialize_with = "lenient_id")]
    pub item: i64,
    /// Place the item directly after this item, which must be in the same list.
    #[serde(default, deserialize_with = "lenient_id_opt")]
    pub after_item: Option<i64>,
    /// Or send the item to the "top" or "bottom" of its list. Pass exactly one of after_item or to.
    pub to: Option<Edge>,
}

/// Like [`lenient_id`], for an optional ID.
pub fn lenient_id_opt<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Num(i64),
        Text(String),
    }
    match Option::<Raw>::deserialize(d)? {
        None => Ok(None),
        Some(Raw::Num(n)) => Ok(Some(n)),
        Some(Raw::Text(s)) => s
            .trim()
            .trim_start_matches('#')
            .parse()
            .map(Some)
            .map_err(|_| D::Error::custom(format!("expected an item ID, got '{s}'"))),
    }
}
