-- The order of queued scans after an admin moved one to the front
-- (book/src/data-model.md, "后台任务"). NULL counts as 0, and a move gives
-- the task one less than the smallest rank among queued scans, so startup
-- recovery queues by (rank, id) in the order the scheduler last held.
ALTER TABLE tasks ADD COLUMN queue_rank INTEGER;
