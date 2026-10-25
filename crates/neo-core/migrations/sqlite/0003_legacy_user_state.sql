-- User data merged from an old kikoeru library into a library that already
-- has its own works. The old works are not imported, so these rows wait by
-- work code. When a scan creates a work with that code, the rows move into
-- favorites, legacy_reviews, play_progress and import_pending.
CREATE TABLE legacy_user_state (
  user_id            INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  code               TEXT NOT NULL,
  favorite_at        INTEGER,
  has_review         INTEGER NOT NULL DEFAULT 0,
  rating             INTEGER,
  review_text        TEXT,
  review_progress    TEXT,
  progress_at        INTEGER,
  position_ms        INTEGER,
  finished_at        INTEGER,
  pending_source     TEXT,
  pending_durations  TEXT,
  pending_state      TEXT,
  PRIMARY KEY (user_id, code)
);
CREATE INDEX legacy_user_state_code ON legacy_user_state (code);
