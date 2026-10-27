-- Marker files are exported per show: look up ids by media_key prefix.
CREATE INDEX IF NOT EXISTS marker_ids_media_key ON marker_ids(media_key);
-- Hourly pruning of old programs.
CREATE INDEX IF NOT EXISTS programs_end ON programs(end_ms);
