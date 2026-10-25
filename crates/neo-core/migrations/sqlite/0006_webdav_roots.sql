-- WebDAV roots (book/src/storage.md, "webdav") are a third kind of stored
-- root. SQLite cannot change a CHECK constraint in place, so the table is
-- rebuilt with the wider one. Nothing refers to it, so the copy is safe.
CREATE TABLE root_settings_new (
  root_id     TEXT PRIMARY KEY REFERENCES roots(id),
  kind        TEXT NOT NULL CHECK (kind IN ('local', 'openlist', 'webdav')),
  -- JSON without the password, which lives in secrets/roots/.
  settings    TEXT NOT NULL,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);
INSERT INTO root_settings_new (root_id, kind, settings, created_at, updated_at)
  SELECT root_id, kind, settings, created_at, updated_at FROM root_settings;
DROP TABLE root_settings;
ALTER TABLE root_settings_new RENAME TO root_settings;
