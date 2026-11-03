-- "Remind me" per profile. Channel/start/title are copied so a reminder
-- survives the guide being rebuilt.
CREATE TABLE reminders (
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    program_id INTEGER NOT NULL,
    channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    start_ms   INTEGER NOT NULL,
    title      TEXT NOT NULL,
    PRIMARY KEY (user_id, program_id)
);
CREATE INDEX reminders_start ON reminders(start_ms);
