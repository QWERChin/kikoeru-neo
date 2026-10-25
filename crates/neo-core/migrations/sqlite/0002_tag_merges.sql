-- Tags an admin merged away, so the startup dictionary sync does not
-- recreate them. A name collision between two dictionary entries is not a
-- merge, so the sync cannot infer this from the aliases alone.
CREATE TABLE tag_merges (
  from_id  INTEGER PRIMARY KEY,
  into_id  INTEGER NOT NULL,
  merged_at INTEGER NOT NULL
);
