-- How far the back and forward buttons skip (book/src/contract.md,
-- "UserSettings"). The button icon draws the number, so it has two digits.
ALTER TABLE user_settings ADD COLUMN seek_back_seconds INTEGER NOT NULL DEFAULT 15
  CHECK (seek_back_seconds BETWEEN 1 AND 99);
ALTER TABLE user_settings ADD COLUMN seek_forward_seconds INTEGER NOT NULL DEFAULT 30
  CHECK (seek_forward_seconds BETWEEN 1 AND 99);
