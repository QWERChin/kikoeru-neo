-- Roots added through the API (book/src/contract.md, "媒体库管理接口").
-- The row in `roots` stays when a root is removed, because its locations
-- and tracks still refer to it; only its settings go.
CREATE TABLE root_settings (
  root_id     TEXT PRIMARY KEY REFERENCES roots(id),
  kind        TEXT NOT NULL CHECK (kind IN ('local', 'openlist')),
  -- JSON without the OpenList password, which lives in secrets/roots/.
  settings    TEXT NOT NULL,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);
