# Tally

A local, UI-less list-making app for agents. Written in Rust, stores data in
SQLite, and exposes an MCP server over stdio. Single user, not multi-tenant.

## Run

```sh
cargo build --release
./target/release/tally   # speaks MCP over stdin/stdout
```

The database lives at `~/.local/share/tally/tally.db`. Set `TALLY_DB` to use a
different file.

Register with Claude Code:

```sh
claude mcp add tally -- /path/to/tally
```

## Develop

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

CI runs the same checks on every commit to `main`.
