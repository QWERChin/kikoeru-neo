-- Initial schema. book/src/data-model.md is the source of truth for every
-- table and column here. Business timestamps are Unix milliseconds.

CREATE TABLE roots (
  id            TEXT PRIMARY KEY,
  display_name  TEXT NOT NULL,
  last_scan_at  INTEGER,
  status        TEXT NOT NULL DEFAULT 'unknown'
);

CREATE TABLE circles (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
CREATE TABLE vas     (id TEXT PRIMARY KEY,    name TEXT NOT NULL);
CREATE TABLE tags    (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
CREATE TABLE tag_aliases (
  alias   TEXT PRIMARY KEY,
  tag_id  INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE
);

CREATE TABLE works (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  code          TEXT NOT NULL UNIQUE,
  title         TEXT NOT NULL,
  circle_id     INTEGER REFERENCES circles(id),
  release_date  TEXT,
  nsfw          INTEGER NOT NULL DEFAULT 0,
  dl_count      INTEGER,
  price         INTEGER,
  rate_average  REAL,
  rate_count    INTEGER,
  metadata_src  TEXT NOT NULL,
  cover_src     TEXT NOT NULL DEFAULT 'none',
  cover_version INTEGER NOT NULL DEFAULT 0,
  extra         TEXT NOT NULL DEFAULT '{}',
  missing_at    INTEGER,
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL
);

CREATE TABLE work_locations (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  work_id       INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
  root_id       TEXT NOT NULL REFERENCES roots(id),
  rel_path      BLOB NOT NULL,
  display_path  TEXT NOT NULL,
  listing_hash  TEXT,
  listed_at     INTEGER,
  missing_at    INTEGER,
  UNIQUE (root_id, rel_path)
);
CREATE INDEX work_locations_work ON work_locations (work_id);

CREATE TABLE work_vas  (work_id INTEGER REFERENCES works(id) ON DELETE CASCADE,
                        va_id   TEXT    REFERENCES vas(id)   ON DELETE CASCADE,
                        PRIMARY KEY (work_id, va_id));
CREATE TABLE work_tags (work_id INTEGER REFERENCES works(id) ON DELETE CASCADE,
                        tag_id  INTEGER REFERENCES tags(id)  ON DELETE CASCADE,
                        PRIMARY KEY (work_id, tag_id));

CREATE TABLE tracks (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  work_id       INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
  location_id   INTEGER NOT NULL REFERENCES work_locations(id) ON DELETE CASCADE,
  rel_path      BLOB NOT NULL,
  display_path  TEXT NOT NULL,
  sort_key      TEXT NOT NULL,
  kind          TEXT NOT NULL,
  size          INTEGER,
  mtime         INTEGER,
  duration_ms   INTEGER,
  duration_src  TEXT,
  probe         TEXT,
  duplicate_of  INTEGER REFERENCES tracks(id),
  missing_at    INTEGER,
  UNIQUE (location_id, rel_path)
);
CREATE INDEX tracks_work_kind ON tracks (work_id, kind);

CREATE TABLE users (
  id               INTEGER PRIMARY KEY AUTOINCREMENT,
  name             TEXT NOT NULL UNIQUE,
  password_hash    TEXT,
  is_admin         INTEGER NOT NULL DEFAULT 0,
  token_version    INTEGER NOT NULL DEFAULT 0,
  created_at       INTEGER NOT NULL
);

CREATE TABLE favorites (
  user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  work_id     INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
  created_at  INTEGER NOT NULL,
  PRIMARY KEY (user_id, work_id)
);

CREATE TABLE play_progress (
  user_id       INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  work_id       INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
  track_id      INTEGER REFERENCES tracks(id),
  position_ms   INTEGER NOT NULL DEFAULT 0,
  group_path    TEXT,
  finished_at   INTEGER,
  replaying     INTEGER NOT NULL DEFAULT 0,
  updated_at    INTEGER NOT NULL,
  PRIMARY KEY (user_id, work_id)
);

CREATE TABLE legacy_reviews (
  user_id      INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  work_id      INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
  rating       INTEGER,
  review_text  TEXT,
  progress     TEXT,
  PRIMARY KEY (user_id, work_id)
);

CREATE TABLE import_pending (
  user_id      INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  work_id      INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
  source       TEXT NOT NULL,
  durations    TEXT,
  state        TEXT NOT NULL,
  PRIMARY KEY (user_id, work_id)
);

CREATE TABLE tasks (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  kind        TEXT NOT NULL,
  dedupe_key  TEXT,
  state       TEXT NOT NULL,
  payload     TEXT NOT NULL,
  result      TEXT,
  progress    REAL,
  message     TEXT,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);
CREATE UNIQUE INDEX tasks_active_dedupe ON tasks (dedupe_key)
  WHERE state IN ('queued', 'running');
