-- Video playlists from the media servers (a cache, refreshed on every sync).
CREATE TABLE playlists (
    id        INTEGER PRIMARY KEY,
    source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    key       TEXT NOT NULL,
    title     TEXT NOT NULL,
    synced_at INTEGER NOT NULL,
    UNIQUE (source_id, key)
);
-- Entries by the server's item key, so a playlist survives items being
-- re-synced; joined to media_items when the library is loaded.
CREATE TABLE playlist_items (
    playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    item_key    TEXT NOT NULL,
    PRIMARY KEY (playlist_id, position)
);
