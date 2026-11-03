-- Optional yearly date range for a time-of-day block ("MM-DD"), e.g.
-- Halloween 10-01..10-31. A range may wrap the new year (12-20..01-05).
ALTER TABLE channel_slots ADD COLUMN date_from TEXT;
ALTER TABLE channel_slots ADD COLUMN date_to TEXT;
