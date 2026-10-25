-- See the SQLite migration of the same number.
CREATE TABLE server_settings (
  id              BIGINT PRIMARY KEY CHECK (id = 1),
  metadata_proxy  TEXT COLLATE "C",
  updated_at      BIGINT NOT NULL
);
