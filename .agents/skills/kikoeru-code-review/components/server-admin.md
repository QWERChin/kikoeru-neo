Component: server admin endpoints and background tasks

Lenses: correctness, auth, data

Files in scope (crates/neo-server/src/): http/roots.rs, http/tasks.rs,
http/edit.rs, http/metadata.rs, http/works.rs, http/library.rs,
http/admin.rs, http/server_settings.rs, tasks.rs, tasks/scan_queue.rs, scan.rs, autoscan.rs,
watch.rs, roots.rs, and their tests in crates/neo-server/tests/
(tasks.rs, task_log.rs, api_task_admin.rs, api_roots.rs, api_scan.rs,
api_edit.rs, api_metadata.rs, api_works.rs, api_library.rs, api_detail.rs,
api_tags.rs, api_remote.rs, api_webdav.rs, api_settings.rs,
process_m4.rs, process_m6.rs).

Book chapters: scanning.md, contract.md (admin and task endpoints),
api.md, storage.md, data-model.md and testing.md.

Look especially for: task queue races (the same root scanned twice at once,
a cancel that arrives between states, tasks stuck in running after a crash
or restart, lost wake-ups); autoscan and file watcher scheduling that misses
changes, loops, or fires during a running scan; removing or editing a root
while a scan is running; library listing, search, sort, and filter endpoints
returning wrong pages or counts; metadata edits that a later scan or refresh
silently overwrites; admin endpoints that accept input producing an
inconsistent database state.
