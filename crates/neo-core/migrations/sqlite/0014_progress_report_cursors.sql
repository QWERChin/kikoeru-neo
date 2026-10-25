CREATE TABLE progress_report_cursors (
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    last_sequence INTEGER NOT NULL,
    PRIMARY KEY (user_id, work_id, session_id)
);
