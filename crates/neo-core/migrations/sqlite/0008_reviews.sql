-- The user's own rating and short review per work (book/src/contract.md,
-- "评分和短评"). legacy_reviews stays an untouched archive of t_review.
CREATE TABLE reviews (
  user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  work_id     INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
  rating      INTEGER CHECK (rating BETWEEN 1 AND 5),
  text        TEXT,
  updated_at  INTEGER,
  PRIMARY KEY (user_id, work_id),
  CHECK (rating IS NOT NULL OR text IS NOT NULL)
);

-- A merged review's time, so it reaches reviews.updated_at when its work is
-- scanned.
ALTER TABLE legacy_user_state ADD COLUMN review_at INTEGER;

-- Libraries imported before this table existed get their old ratings and
-- reviews by the rules of neo_core::review::from_legacy. SQLite's trim takes
-- an explicit set, so it lists every Unicode White_Space character, which is
-- what Rust's str::trim strips there. legacy_reviews has no time, so
-- updated_at stays NULL.
INSERT INTO reviews (user_id, work_id, rating, text, updated_at)
SELECT user_id, work_id, rating, text, NULL
FROM (
  SELECT user_id, work_id,
         CASE WHEN typeof(rating) = 'integer' AND rating BETWEEN 1 AND 5
              THEN rating END AS rating,
         substr(NULLIF(trim(review_text, char(9, 10, 11, 12, 13, 32, 133, 160, 5760, 8192, 8193,
                                       8194, 8195, 8196, 8197, 8198, 8199, 8200, 8201,
                                       8202, 8232, 8233, 8239, 8287, 12288)), ''),
                1, 2000) AS text
  FROM legacy_reviews
)
WHERE rating IS NOT NULL OR text IS NOT NULL;
