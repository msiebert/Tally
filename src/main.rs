use anyhow::{Context, Result, bail};
use rmcp::{ServiceExt, transport::stdio};
use std::io::Write;
use tally::cli::{self, Command, Format, USAGE};
use tally::{db, server::TallyServer, store::export};

#[tokio::main]
async fn main() -> Result<()> {
    let command = match cli::parse(std::env::args().skip(1)) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("tally: {message}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    match command {
        Command::Serve => serve().await,
        Command::Export { format, output } => run_export(format, output),
        Command::Help => {
            println!("{USAGE}");
            Ok(())
        }
        Command::Version => {
            println!("tally {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

async fn serve() -> Result<()> {
    // stdout carries the MCP protocol, so anything human-readable goes to stderr.
    let path = db::default_db_path();
    let conn = db::open(&path)?;
    eprintln!("tally: using database {}", path.display());
    let service = TallyServer::new(conn).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

fn run_export(format: Format, output: Option<std::path::PathBuf>) -> Result<()> {
    let path = db::default_db_path();
    if !path.exists() {
        bail!(
            "no database at {} (set TALLY_DB to export a different file)",
            path.display()
        );
    }
    let conn = db::open(&path)?;
    let text = match format {
        Format::Json => serde_json::to_string_pretty(&export::to_json(&conn)?)?,
        Format::Markdown => export::to_markdown(&conn)?,
    };
    match output {
        None => println!("{text}"),
        Some(target) => {
            // create_new: a backup must never silently replace another file
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .with_context(|| {
                    format!(
                        "can't create {} (it may already exist; choose a new path)",
                        target.display()
                    )
                })?;
            writeln!(file, "{text}")?;
            eprintln!("tally: exported to {}", target.display());
        }
    }
    Ok(())
}
