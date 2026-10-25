CREATE TABLE progress_report_cursors (
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id BIGINT NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    session_id TEXT COLLATE "C" NOT NULL,
    last_sequence BIGINT NOT NULL,
    PRIMARY KEY (user_id, work_id, session_id)
);
