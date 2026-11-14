#!/usr/bin/env bash
# Register Tally with Claude Code over stdio and run a real agent workflow
# against it. Needs the `claude` CLI, logged in. Uses a scratch database and a
# scratch project directory, and removes its registration when it finishes.
#
#   scripts/e2e-claude-code.sh
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cargo build --release --manifest-path "$root/Cargo.toml"

work="$(mktemp -d)"
db="$work/tally.db"
trap 'cd "$work" && claude mcp remove tally --scope local >/dev/null 2>&1 || true; rm -rf "$work"' EXIT
cd "$work"

claude mcp add tally --scope local -e "TALLY_DB=$db" -- "$root/target/release/tally"
claude mcp list | grep -q "tally.*Connected" || { echo "Tally did not connect"; exit 1; }
echo "Tally is registered and connected."

claude -p --allowedTools "mcp__tally" --max-turns 30 "Use only the tally MCP tools for this.
1) Create a list named 'launch' with the instructions 'Item titles must start with a verb.'
2) Create a tag named 'critical' with the instructions 'Do critical items before anything else.'
3) Add three items to launch: 'Write release notes', 'Cut release branch' (tagged critical), and 'Publish release', which must be blocked by 'Cut release branch'.
4) Read the launch list and follow its instructions.
5) Show which items are currently unblocked.
6) Complete 'Cut release branch', then show the unblocked items again.
7) Finish with a 3-line summary of what you did and anything about the tools that confused you."

echo
echo "Database contents:"
python3 - "$db" <<'PY'
import sqlite3, sys
db = sqlite3.connect(sys.argv[1])
for row in db.execute("select id, title, status from item order by id"):
    print(" ", row)
print("  dependencies:", list(db.execute("select * from dependency")))
PY
