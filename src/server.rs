//! The MCP server: tool definitions and the handler that serves them.

use rmcp::{
    ServerHandler,
    handler::server::router::tool::ToolRouter,
    model::{Implementation, ServerCapabilities, ServerConfig},
    tool_handler, tool_router,
};
use rusqlite::Connection;
use std::sync::{Arc, Mutex, MutexGuard};

const INSTRUCTIONS: &str = "Tally is a list-making app for agents.";

#[derive(Clone)]
pub struct TallyServer {
    db: Arc<Mutex<Connection>>,
    tool_router: ToolRouter<Self>,
}

#[tool_router(router = tool_router, allow_empty)]
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
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for TallyServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("tally", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}
