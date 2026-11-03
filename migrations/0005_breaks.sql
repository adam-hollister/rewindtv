-- What a library is for: 'content' (shows/movies), 'commercials' (fill
-- breaks) or 'bumpers' (station IDs before shows).
ALTER TABLE libraries ADD COLUMN role TEXT NOT NULL DEFAULT 'content';

ALTER TABLE channels ADD COLUMN fill_commercials INTEGER NOT NULL DEFAULT 0;
ALTER TABLE channels ADD COLUMN station_ids INTEGER NOT NULL DEFAULT 0;

-- 'show', 'break' (a commercial) or 'bumper' (a station ID).
ALTER TABLE programs ADD COLUMN kind TEXT NOT NULL DEFAULT 'show';
