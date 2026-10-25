-- The user's accent colour (book/src/contract.md, "Accent"). NULL is the
-- default blue. The names are the DTO's, so a stored value always parses.
ALTER TABLE user_settings ADD COLUMN accent TEXT
  CHECK (accent IN ('blue', 'purple', 'pink', 'red', 'orange', 'green', 'teal', 'indigo'));
