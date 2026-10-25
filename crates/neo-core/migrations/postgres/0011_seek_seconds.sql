-- See the SQLite migration of the same number.
ALTER TABLE user_settings ADD COLUMN seek_back_seconds BIGINT NOT NULL DEFAULT 15
  CHECK (seek_back_seconds BETWEEN 1 AND 99);
ALTER TABLE user_settings ADD COLUMN seek_forward_seconds BIGINT NOT NULL DEFAULT 30
  CHECK (seek_forward_seconds BETWEEN 1 AND 99);
