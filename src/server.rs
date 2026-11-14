//! The MCP server: tool definitions and the handler that serves them.

use crate::error::Result as ToolResult;
use crate::params::*;
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

const INSTRUCTIONS: &str = "Tally is a list-making app for agents.";

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
        description = "Delete a list and all of its items. Refused if any item in it blocks an item in another list; \
                       the error names those items so you can remove the dependencies first."
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
        description = "Delete a tag. Refused while any item still uses it; the error gives the item count."
    )]
    fn delete_tag(&self, Parameters(p): Parameters<DeleteTag>) -> CallToolResult {
        reply(tags::delete(&mut self.db(), &p.tag))
    }

    #[tool(
        description = "Add items to a list in one call (max 200). All-or-nothing: if any item is invalid, none are added. \
                       Tags must already exist (create_tag first); unknown tags are rejected. \
                       Not idempotent: calling twice adds the items twice. Returns the new item IDs in order."
    )]
    fn add_items(&self, Parameters(p): Parameters<AddItems>) -> CallToolResult {
        reply(items::add(&mut self.db(), &p.list, &p.items))
    }

    #[tool(
        description = "Change an item's title, notes, status (todo, doing, done, cancelled) and/or tags. \
                       tags replaces the whole tag set, and every tag must already exist (create_tag first). \
                       Empty notes clear them."
    )]
    fn update_item(&self, Parameters(p): Parameters<UpdateItem>) -> CallToolResult {
        reply(items::update(
            &mut self.db(),
            p.item,
            p.title.as_deref(),
            p.notes.as_deref(),
            p.status,
            p.tags.as_deref(),
        ))
    }

    #[tool(
        description = "Mark an item done. Allowed even if it still has open blockers; the response then includes a warning."
    )]
    fn complete_item(&self, Parameters(p): Parameters<ItemRef>) -> CallToolResult {
        reply(items::complete(&mut self.db(), p.item))
    }

    #[tool(
        description = "Delete an item. Refused if other items depend on it; the error lists them by ID and title \
                       so you can remove those dependencies first."
    )]
    fn delete_item(&self, Parameters(p): Parameters<ItemRef>) -> CallToolResult {
        reply(items::delete(&mut self.db(), p.item))
    }

    #[tool(
        description = "Move an item to another list (or reposition it within its own list). Goes to the bottom unless position is given. \
                       Dependencies are kept."
    )]
    fn move_item(&self, Parameters(p): Parameters<MoveItem>) -> CallToolResult {
        reply(items::move_to(
            &mut self.db(),
            p.item,
            &p.to_list,
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
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for TallyServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("tally", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}
