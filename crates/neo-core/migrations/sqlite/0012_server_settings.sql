-- Settings for the whole server, changed by an admin (book/src/contract.md,
-- "服务器设置"). There is at most one row. Without it, every setting is its
-- default.
CREATE TABLE server_settings (
  id              INTEGER PRIMARY KEY CHECK (id = 1),
  -- The normalised proxy address. NULL leaves metadata requests to the
  -- proxy environment variables.
  metadata_proxy  TEXT,
  updated_at      INTEGER NOT NULL
);
