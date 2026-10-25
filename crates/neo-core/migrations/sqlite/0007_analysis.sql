-- Loudness measured by analysis tasks (book/src/media.md, "响度和波形").
-- size and mtime are the source's at measurement; a row whose values no
-- longer match the track's is stale and ignored.
CREATE TABLE track_analysis (
  track_id        INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
  size            INTEGER,
  mtime           INTEGER,
  integrated_lufs REAL,
  true_peak_dbtp  REAL,
  analyzed_at     INTEGER NOT NULL
);

-- GET/PUT /api/me/settings. A user without a row has the defaults.
CREATE TABLE user_settings (
  user_id            INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  normalize_loudness INTEGER NOT NULL DEFAULT 0,
  target_lufs        REAL NOT NULL DEFAULT -16,
  updated_at         INTEGER NOT NULL
);
