-- Tally schema v1.
-- AUTOINCREMENT gives each entity type its own ID sequence, and IDs are never reused.
-- Item IDs are global (not per list) so dependencies can cross lists.

CREATE TABLE list (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    name         TEXT NOT NULL UNIQUE,
    instructions TEXT,
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE item (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    list_id    INTEGER NOT NULL REFERENCES list(id),
    title      TEXT NOT NULL,
    notes      TEXT,
    status     TEXT NOT NULL DEFAULT 'todo'
               CHECK (status IN ('todo', 'doing', 'done', 'cancelled')),
    position   INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX item_list_position ON item (list_id, position, id);
CREATE INDEX item_updated ON item (updated_at, id);

CREATE TABLE tag (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    name         TEXT NOT NULL UNIQUE,
    instructions TEXT
);

CREATE TABLE item_tag (
    item_id INTEGER NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    tag_id  INTEGER NOT NULL REFERENCES tag(id),
    PRIMARY KEY (item_id, tag_id)
);
CREATE INDEX item_tag_tag ON item_tag (tag_id);

CREATE TABLE dependency (
    item_id            INTEGER NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    blocked_by_item_id INTEGER NOT NULL REFERENCES item(id),
    PRIMARY KEY (item_id, blocked_by_item_id),
    CHECK (item_id <> blocked_by_item_id)
);
CREATE INDEX dependency_blocker ON dependency (blocked_by_item_id);
