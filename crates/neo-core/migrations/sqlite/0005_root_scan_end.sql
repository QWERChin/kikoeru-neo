-- When the last scan of a root ended, whatever its outcome. Automatic scans
-- count their interval from it, and finished tasks are pruned, so it cannot
-- come from the tasks table.
ALTER TABLE roots ADD COLUMN last_scan_end_at INTEGER;
