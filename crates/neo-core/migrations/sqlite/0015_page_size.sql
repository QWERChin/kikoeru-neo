-- How many works a page of the works lists shows (book/src/contract.md,
-- "UserSettings"). The values are the DTO's, so a stored value always parses.
ALTER TABLE user_settings ADD COLUMN page_size INTEGER NOT NULL DEFAULT 40
  CHECK (page_size IN (20, 40, 60, 100));
