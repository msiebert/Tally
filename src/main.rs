use anyhow::Result;
use rmcp::{ServiceExt, transport::stdio};
use tally::{db, server::TallyServer};

#[tokio::main]
async fn main() -> Result<()> {
    // stdout carries the MCP protocol, so anything human-readable goes to stderr.
    let path = db::default_db_path();
    let conn = db::open(&path)?;
    eprintln!("tally: using database {}", path.display());
    let service = TallyServer::new(conn).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
