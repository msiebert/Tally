//! The MCP server: tool definitions and the handler that serves them.

use crate::error::Result as ToolResult;
use crate::params::*;
use crate::store::reads::{self, ReadOptions};
use crate::store::{dependencies, items, lists, tags};
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router,
};
use rusqlite::Connection;
use serde_json::Value;
use std::sync::{Arc, Mutex, MutexGuard};

const INSTRUCTIONS: &str = "\
Tally keeps lists of items for you. Refer to lists by name (case-insensitive) or ID, and to items and dependencies by item ID.

Instructions: lists and tags can carry instructions from the user. get_list returns the list's instructions and, in tag_instructions, \
the instructions for the tags on that page; search_items returns tag_instructions. Read and follow them. They come only on the first page of a read. \
Pass include_instructions / include_tag_instructions = false when you already have them in context, and read again if you lose them.

Tags: create a tag with create_tag before using it. add_items and update_item reject unknown tags.

Order: list order is priority. get_list shows items in order and hides done items unless you pass status. \
Use reorder_item, or position (top, bottom, {after: id}) on add_items, move_item and reorder_item. move_item without to_list repositions within the current list.

Dependencies: add_dependency makes an item wait for another, in any list. An item is blocked while any blocker is not done or cancelled. \
Reads show blocked and open_blockers, and unblocked_only returns just the workable items. Completing a blocked item is allowed but warns.

Pagination: reads return next_cursor (null on the last page) and total_count. Pass next_cursor back unchanged. limit defaults to 50, max 200.

Deletes that would break links are refused, and the error says what is in the way: an item that blocks others or a list whose items block \
items elsewhere (remove_dependency first), a tag still in use (drop it with update_item). Errors say how to fix the problem; read them and adjust.

add_items is not idempotent and does not dedupe, so don't repeat a call that succeeded.";

#[derive(Clone)]
pub struct TallyServer {
    db: Arc<Mutex<Connection>>,
    tool_router: ToolRouter<Self>,
}

/// Turn a store result into a tool result. Failures are tool errors the agent
/// can read and act on, not protocol errors. JSON is compact to keep tokens low.
fn reply(result: ToolResult<Value>) -> CallToolResult {
    match result {
        Ok(v) => CallToolResult::success(vec![ContentBlock::text(v.to_string())]),
        Err(e) => CallToolResult::error(vec![ContentBlock::text(e.to_string())]),
    }
}

#[tool_router(router = tool_router)]
impl TallyServer {
    pub fn new(conn: Connection) -> Self {
        Self {
            db: Arc::new(Mutex::new(conn)),
            tool_router: Self::tool_router(),
        }
    }

    /// Lock the database connection. One agent at a time is the design, so a
    /// single mutex-guarded connection is enough.
    pub fn db(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[tool(
        description = "Create a list. Names are unique (case-insensitive) and are how you refer to the list afterwards. \
                       Optional instructions are guidance for agents that use this list."
    )]
    fn create_list(&self, Parameters(p): Parameters<CreateList>) -> CallToolResult {
        reply(lists::create(
            &mut self.db(),
            &p.name,
            p.instructions.as_deref(),
        ))
    }

    #[tool(
        description = "List all lists with their open and total item counts. Paginated: pass next_cursor to get the next page."
    )]
    fn list_lists(&self, Parameters(p): Parameters<ListLists>) -> CallToolResult {
        reply(lists::list(&self.db(), p.cursor.as_deref(), p.limit))
    }

    #[tool(
        description = "Rename a list and/or change its instructions. An empty instructions string clears them."
    )]
    fn update_list(&self, Parameters(p): Parameters<UpdateList>) -> CallToolResult {
        reply(lists::update(
            &mut self.db(),
            &p.list,
            p.name.as_deref(),
            p.instructions.as_deref(),
        ))
    }

    #[tool(
        description = "Delete a list and all its items. Refused if any item in it blocks an item in another list; the error names them. \
                       Remove those dependencies first (remove_dependency)."
    )]
    fn delete_list(&self, Parameters(p): Parameters<DeleteList>) -> CallToolResult {
        reply(lists::delete(&mut self.db(), &p.list))
    }

    #[tool(
        description = "Create a tag. Tags must be created here before they can be used: add_items and update_item \
                       reject tags that don't exist. Optional instructions tell agents how to treat items with this tag."
    )]
    fn create_tag(&self, Parameters(p): Parameters<CreateTag>) -> CallToolResult {
        reply(tags::create(
            &mut self.db(),
            &p.name,
            p.instructions.as_deref(),
        ))
    }

    #[tool(
        description = "List all tags with how many items use each. Paginated: pass next_cursor to get the next page."
    )]
    fn list_tags(&self, Parameters(p): Parameters<ListTags>) -> CallToolResult {
        reply(tags::list(&self.db(), p.cursor.as_deref(), p.limit))
    }

    #[tool(
        description = "Rename a tag and/or change its instructions. An empty instructions string clears them."
    )]
    fn update_tag(&self, Parameters(p): Parameters<UpdateTag>) -> CallToolResult {
        reply(tags::update(
            &mut self.db(),
            &p.tag,
            p.name.as_deref(),
            p.instructions.as_deref(),
        ))
    }

    #[tool(
        description = "Delete a tag. Refused while any item still uses it; the error gives the item count. \
                       Remove the tag from those items first (update_item with the tags to keep)."
    )]
    fn delete_tag(&self, Parameters(p): Parameters<DeleteTag>) -> CallToolResult {
        reply(tags::delete(&mut self.db(), &p.tag))
    }

    #[tool(
        description = "Tags must already exist (create_tag first); unknown tags are rejected. Add up to 200 items to a list in one call, \
                       all-or-nothing: if any item is invalid, none are added. Not idempotent: calling twice adds the items twice. \
                       Returns the new item IDs in order."
    )]
    fn add_items(&self, Parameters(p): Parameters<AddItems>) -> CallToolResult {
        reply(items::add(&mut self.db(), &p.list, &p.items))
    }

    #[tool(
        description = "Tags must already exist (create_tag first); unknown tags are rejected. Change an item's title, notes (empty clears), \
                       due_date (YYYY-MM-DD; empty clears), status (todo, doing, done, cancelled) and/or tags. tags replaces the item's whole tag set."
    )]
    fn update_item(&self, Parameters(p): Parameters<UpdateItem>) -> CallToolResult {
        reply(items::update(
            &mut self.db(),
            p.item,
            p.title.as_deref(),
            p.notes.as_deref(),
            p.status,
            p.tags.as_deref(),
            p.due_date.as_deref(),
        ))
    }

    #[tool(
        description = "Mark an item done. Allowed even if it still has open blockers; the response then includes a warning."
    )]
    fn complete_item(&self, Parameters(p): Parameters<ItemRef>) -> CallToolResult {
        reply(items::complete(&mut self.db(), p.item))
    }

    #[tool(
        description = "Delete an item. Refused if other items depend on it; the error lists them by ID and title. \
                       Remove those dependencies first (remove_dependency), or complete or cancel the item instead."
    )]
    fn delete_item(&self, Parameters(p): Parameters<ItemRef>) -> CallToolResult {
        reply(items::delete(&mut self.db(), p.item))
    }

    #[tool(
        description = "Move an item to another list (to_list), or reposition it within its own list (omit to_list). \
                       Goes to the bottom unless position is given. Dependencies are kept."
    )]
    fn move_item(&self, Parameters(p): Parameters<MoveItem>) -> CallToolResult {
        reply(items::move_to(
            &mut self.db(),
            p.item,
            p.to_list.as_ref(),
            p.position,
        ))
    }

    #[tool(
        description = "Make an item wait for another item (blocked_by), which can be in a different list. \
                       Rejected if it would create a cycle; the error names the cycle. \
                       An item counts as blocked while any blocker is neither done nor cancelled."
    )]
    fn add_dependency(&self, Parameters(p): Parameters<Dependency>) -> CallToolResult {
        reply(dependencies::add(&mut self.db(), p.item, p.blocked_by))
    }

    #[tool(
        description = "Remove a dependency so the item no longer waits for blocked_by. \
                       Do this first when delete_item or delete_list is refused because items depend on each other."
    )]
    fn remove_dependency(&self, Parameters(p): Parameters<Dependency>) -> CallToolResult {
        reply(dependencies::remove(&mut self.db(), p.item, p.blocked_by))
    }

    #[tool(
        description = "Show a list's items in order, with tags, notes, timestamps, and a blocked flag listing open blockers. \
                       Done items are hidden unless you pass status. Paginated: pass next_cursor for the next page. \
                       The first page also returns the list's instructions and tag_instructions for tags on the page: read and follow them. \
                       Pass include_instructions / include_tag_instructions = false if you already have them."
    )]
    fn get_list(&self, Parameters(p): Parameters<GetList>) -> CallToolResult {
        let statuses = p.status.map(OneOrMany::into_vec);
        reply(reads::get_list(
            &self.db(),
            &p.list,
            &ReadOptions {
                statuses: statuses.as_deref(),
                tag: p.tag.as_deref(),
                unblocked_only: p.unblocked_only.unwrap_or(false),
                cursor: p.cursor.as_deref(),
                limit: p.limit,
                include_instructions: p.include_instructions.unwrap_or(true),
                include_tag_instructions: p.include_tag_instructions.unwrap_or(true),
            },
        ))
    }

    #[tool(
        description = "Search items across all lists, most recently updated first. Combine text, list, tag, status and \
                       unblocked_only. Searches all statuses unless you pass status. Paginated like get_list. \
                       The first page returns tag_instructions for the tags on the page (read and follow them); \
                       pass include_tag_instructions = false if you already have them."
    )]
    fn search_items(&self, Parameters(p): Parameters<SearchItems>) -> CallToolResult {
        let statuses = p.status.map(OneOrMany::into_vec);
        reply(reads::search_items(
            &self.db(),
            p.text.as_deref(),
            p.list.as_ref(),
            &ReadOptions {
                statuses: statuses.as_deref(),
                tag: p.tag.as_deref(),
                unblocked_only: p.unblocked_only.unwrap_or(false),
                cursor: p.cursor.as_deref(),
                limit: p.limit,
                include_instructions: false,
                include_tag_instructions: p.include_tag_instructions.unwrap_or(true),
            },
        ))
    }

    #[tool(
        description = "Change an item's place within its list: pass after_item (an item in the same list), to = \"top\" / \"bottom\", or position (\"top\", \"bottom\", {after: id}, \
                       as in add_items and move_item); exactly one. \
                       List order is the only way to express priority, so keep the most important items at the top. \
                       To change lists use move_item with to_list."
    )]
    fn reorder_item(&self, Parameters(p): Parameters<ReorderItem>) -> CallToolResult {
        reply(items::reorder(
            &mut self.db(),
            p.item,
            p.after_item,
            p.to,
            p.position,
        ))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for TallyServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("tally", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}
