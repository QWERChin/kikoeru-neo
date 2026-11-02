-- See the SQLite migration of the same number.
ALTER TABLE user_settings ADD COLUMN page_size BIGINT NOT NULL DEFAULT 40
  CHECK (page_size IN (20, 40, 60, 100));
