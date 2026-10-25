-- See the SQLite migration of the same number.
ALTER TABLE tasks ADD COLUMN queue_rank BIGINT;
