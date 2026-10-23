CREATE TABLE users (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL,
    color       TEXT NOT NULL DEFAULT '#f59e0b',
    is_admin    INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL
);

CREATE TABLE sources (
    id          INTEGER PRIMARY KEY,
    kind        TEXT NOT NULL CHECK (kind IN ('plex', 'jellyfin')),
    name        TEXT NOT NULL,
    url         TEXT NOT NULL,
    token       TEXT NOT NULL,
    enabled     INTEGER NOT NULL DEFAULT 1,
    last_sync   INTEGER,
    last_error  TEXT,
    created_at  INTEGER NOT NULL
);

CREATE TABLE libraries (
    id          INTEGER PRIMARY KEY,
    source_id   INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    key         TEXT NOT NULL,
    name        TEXT NOT NULL,
    kind        TEXT NOT NULL,
    enabled     INTEGER NOT NULL DEFAULT 1,
    UNIQUE (source_id, key)
);

CREATE TABLE media_items (
    id             INTEGER PRIMARY KEY,
    source_id      INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    library_id     INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    item_key       TEXT NOT NULL,
    part_key       TEXT,
    kind           TEXT NOT NULL CHECK (kind IN ('movie', 'episode')),
    title          TEXT NOT NULL,
    show_key       TEXT,
    show_title     TEXT,
    season         INTEGER,
    episode        INTEGER,
    year           INTEGER,
    duration_ms    INTEGER NOT NULL,
    summary        TEXT,
    content_rating TEXT,
    studio         TEXT,
    thumb          TEXT,
    art            TEXT,
    missing        INTEGER NOT NULL DEFAULT 0,
    synced_at      INTEGER NOT NULL,
    UNIQUE (source_id, item_key)
);
CREATE INDEX media_items_show ON media_items(show_key);

CREATE TABLE item_genres (
    item_id INTEGER NOT NULL REFERENCES media_items(id) ON DELETE CASCADE,
    genre   TEXT NOT NULL,
    PRIMARY KEY (item_id, genre)
);

CREATE TABLE item_collections (
    item_id    INTEGER NOT NULL REFERENCES media_items(id) ON DELETE CASCADE,
    collection TEXT NOT NULL,
    PRIMARY KEY (item_id, collection)
);

CREATE TABLE channels (
    id                 INTEGER PRIMARY KEY,
    number             INTEGER NOT NULL UNIQUE,
    name               TEXT NOT NULL,
    color              TEXT NOT NULL DEFAULT '#f59e0b',
    logo               TEXT,
    owner_user_id      INTEGER REFERENCES users(id) ON DELETE CASCADE,
    rule               TEXT NOT NULL DEFAULT '{}',
    ordering           TEXT NOT NULL DEFAULT 'shuffle',
    block_size         INTEGER NOT NULL DEFAULT 3,
    avoid_repeat_hours INTEGER NOT NULL DEFAULT 24,
    align_minutes      INTEGER NOT NULL DEFAULT 0,
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL
);

-- Time-of-day programming blocks. days is a bitmask, bit 0 = Monday.
-- end_min <= start_min means the slot runs past midnight.
CREATE TABLE channel_slots (
    id         INTEGER PRIMARY KEY,
    channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    title      TEXT NOT NULL,
    days       INTEGER NOT NULL DEFAULT 127,
    start_min  INTEGER NOT NULL,
    end_min    INTEGER NOT NULL,
    rule       TEXT NOT NULL DEFAULT '{}',
    ordering   TEXT NOT NULL DEFAULT 'shuffle'
);

CREATE TABLE user_channels (
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    hidden     INTEGER NOT NULL DEFAULT 0,
    favorite   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (user_id, channel_id)
);

-- item_id NULL = filler (gap before the next aligned start or an empty pool).
CREATE TABLE programs (
    id         INTEGER PRIMARY KEY,
    channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    start_ms   INTEGER NOT NULL,
    end_ms     INTEGER NOT NULL,
    item_id    INTEGER REFERENCES media_items(id) ON DELETE SET NULL,
    slot_id    INTEGER
);
CREATE INDEX programs_channel_time ON programs(channel_id, start_ms);

-- Ordering cursors (shuffle decks, per-show episode positions) per channel.
CREATE TABLE channel_state (
    channel_id INTEGER PRIMARY KEY REFERENCES channels(id) ON DELETE CASCADE,
    state      TEXT NOT NULL
);

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
