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

## Design notes

**Item order.** Order within a list is manual, stored as integer `position`
values spaced 1000 apart. Placing an item takes the midpoint between its new
neighbours, so a reorder rewrites one row. When two neighbours have no room
between them, the list is renumbered once and the placement retried. Ties are
broken by item ID. `reorder_item` (and the `position` option on `add_items` and
`move_item`) are the only way to express priority.

## Use with Claude Code

```sh
claude mcp add tally --scope user -- /path/to/tally
```

To check a real agent against it end to end, run `scripts/e2e-claude-code.sh`
(needs the `claude` CLI, logged in). It registers Tally with a scratch
database, asks Claude to run a small launch-checklist workflow, and prints what
ended up in the database.

## Export

Backups are manual. `tally export` writes every list, item, tag and dependency:

```sh
tally export                                  # JSON to stdout
tally export --format markdown -o backup.md   # readable; refuses to overwrite
```
