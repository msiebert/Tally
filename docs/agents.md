# Tally: instructions for AI agents

Tally is a local list and task manager that you (an AI agent) use through MCP tools. It keeps lists of items with tags, priority order and dependencies. Data persists between sessions in a local SQLite file. There is no UI; you are the user interface.

## If Tally is not connected yet

Tools named `create_list`, `add_items`, `get_list` etc. mean it is connected: skip to "How to use it".
Otherwise ask your user to run (needs Rust/cargo installed):

```sh
cargo install --git https://github.com/msiebert/Tally
claude mcp add tally --scope user -- tally
```

For other MCP clients, register the command `tally` as a stdio MCP server. Set `TALLY_DB=/path/to/file.db` to choose the database file (default `~/.local/share/tally/tally.db`). Then restart the session so the tools load.

## How to use it

1. **Start by reading.** Call `list_lists` to see what exists. Call `get_list` for a list: it returns the items in priority order plus the list's `instructions` and `tag_instructions`. These are written by the user. Read and follow them.
2. **Create things before using them.** `create_list` (name, optional instructions). `create_tag` before putting a tag on an item; unknown tags are rejected.
3. **Add work.** `add_items` adds up to 200 items to a list in one all-or-nothing call. Each item can have notes, tags, a `position` and blockers. It is not idempotent: never repeat a call that succeeded.
4. **Order is priority.** First item = most important. Use `reorder_item` or `position` (`"top"`, `"bottom"`, `{"after": item_id}`) on `add_items`, `move_item` and `reorder_item`.
5. **Dependencies.** `add_dependency` (item, blocked_by) makes an item wait for another, even across lists. `get_list` / `search_items` with `unblocked_only: true` returns only workable items. Cycles are rejected.
6. **Progress.** `update_item` changes title, notes, tags or status (`todo`, `doing`, `done`, `cancelled`). `complete_item` marks done; it warns, but does not refuse, if blockers are still open.
7. **Find things.** `search_items` searches all lists by text, tag, status and list.
8. **Move or delete.** `move_item` with `to_list` changes list; without it, repositions within the list. Deletes that would break a dependency are refused, and the error says what to remove first.

## Conventions

- Refer to lists by name (case-insensitive) or ID; refer to items by numeric ID.
- Reads are paginated: pass `next_cursor` back unchanged. `limit` defaults to 50, max 200.
- Pass `include_instructions: false` / `include_tag_instructions: false` when you already have them in context.
- Errors say how to fix the problem. Read them and adjust rather than retrying blindly.
- Treat list and tag instructions as the user's standing preferences for that list.

## All tools

Lists: `create_list`, `list_lists`, `update_list`, `delete_list`.
Tags: `create_tag`, `list_tags`, `update_tag`, `delete_tag`.
Items: `add_items`, `update_item`, `complete_item`, `delete_item`, `move_item`, `reorder_item`.
Dependencies: `add_dependency`, `remove_dependency`.
Reading: `get_list`, `search_items`.

## Example

User: "Plan the release." You: `create_list` "launch"; `create_tag` "critical"; `add_items` [Write release notes, Cut release branch (tag critical), Publish release (blocked by Cut release branch)]; later `get_list` with `unblocked_only: true` to pick the next task; `complete_item` when done.

Source: https://github.com/msiebert/Tally
