-- Intro/credits markers, keyed by what the media *is* (media_key), not by a
-- server's item id, so markers set once apply on every server that has the
-- same episode or movie. Mirrored to JSON files in the markers folder.
--   media_key: "tv/<show>/s01e02" or "movie/<title>/<year>" (normalized)
--   source:    'server' (Plex/Jellyfin), 'manual' or 'detected'
CREATE TABLE markers (
    media_key   TEXT NOT NULL,
    kind        TEXT NOT NULL CHECK (kind IN ('intro', 'credits')),
    source      TEXT NOT NULL CHECK (source IN ('server', 'manual', 'detected')),
    start_ms    INTEGER NOT NULL,
    end_ms      INTEGER NOT NULL,
    -- Length of the file the marker was measured on (credits follow the end).
    duration_ms INTEGER NOT NULL,
    confidence  REAL,
    updated_at  INTEGER NOT NULL,
    PRIMARY KEY (media_key, kind, source)
);

-- Provider ids ("tvdb:81189", "imdb:tt0213338") that identify a media_key,
-- so renamed titles still match.
CREATE TABLE marker_ids (
    provider_id TEXT PRIMARY KEY,
    media_key   TEXT NOT NULL
);

-- Manual season-wide (season NULL = whole show) markers. With from_end,
-- start/end count back from the end of each episode ("credits = last 1:30").
CREATE TABLE marker_rules (
    show_key   TEXT NOT NULL,
    season     INTEGER NOT NULL DEFAULT -1,
    kind       TEXT NOT NULL CHECK (kind IN ('intro', 'credits')),
    start_ms   INTEGER NOT NULL,
    end_ms     INTEGER NOT NULL,
    from_end   INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (show_key, season, kind)
);

-- Cached chromaprint fingerprints for detection. part: 'head' or 'tail'.
CREATE TABLE fingerprints (
    item_id   INTEGER NOT NULL REFERENCES media_items(id) ON DELETE CASCADE,
    part      TEXT NOT NULL,
    offset_ms INTEGER NOT NULL,
    points    BLOB NOT NULL,
    PRIMARY KEY (item_id, part)
);

ALTER TABLE media_items ADD COLUMN media_key TEXT;
-- File path as the media server sees it (for an optional local media mount).
ALTER TABLE media_items ADD COLUMN file_path TEXT;
ALTER TABLE media_items ADD COLUMN provider_ids TEXT;
ALTER TABLE media_items ADD COLUMN source_updated_at INTEGER;
ALTER TABLE media_items ADD COLUMN markers_checked_at INTEGER;
ALTER TABLE media_items ADD COLUMN detect_checked_at INTEGER;
CREATE INDEX media_items_media_key ON media_items(media_key);

ALTER TABLE channels ADD COLUMN skip_intros INTEGER NOT NULL DEFAULT 0;
ALTER TABLE channels ADD COLUMN skip_credits INTEGER NOT NULL DEFAULT 0;

-- JSON [[start_ms, end_ms], ...] of file-time ranges skipped on air.
ALTER TABLE programs ADD COLUMN cuts TEXT;

-- A changed file on the server makes its cached fingerprints useless.
CREATE TRIGGER fingerprints_stale AFTER UPDATE OF source_updated_at ON media_items
WHEN OLD.source_updated_at IS NOT NEW.source_updated_at
BEGIN
    DELETE FROM fingerprints WHERE item_id = NEW.id;
END;
