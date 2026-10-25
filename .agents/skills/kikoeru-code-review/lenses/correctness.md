# Lens: correctness

Always active. Owns runtime behaviour: does the code do what the book says,
including on error, concurrent, cancelled, and restarted paths?

Hunt in this order:

1. Logic and edge cases. Trace each changed or reviewed function with real
   inputs: empty libraries, a work with one track or thousands, non-UTF-8
   and NFD file names, names with spaces or a leading dash, work codes with
   different prefixes and the same number (RJ/BJ/VJ), zero-length files,
   tracks without a duration, the last page of a list. Look for off-by-one,
   wrong predicates, unstable sort orders, and integer overflow or lossy
   casts (`as u32`, `as i32`, f64 to i64).
2. State that must survive a crash, restart, or cancel. Tasks left in
   `running`, half-written cache files served as complete, ffmpeg or other
   child processes that outlive their request or task, scan results
   committed before the step that can still fail (a failed listing page must
   not change any data, see `book/src/scanning.md`).
3. Concurrency. Check-then-act across `.await` points, read-modify-write
   outside a transaction, two scans or two prepares of the same object at
   once, a cancel that arrives between two state changes, lost wake-ups in
   queues and watchers, locks held across `.await`, and network requests or
   ffmpeg runs while holding the single SQLite write transaction, which
   `book/src/data-model.md` forbids.
4. Error paths. An error mapped to the wrong kind (a transient storage
   failure reported as not-found, which the scanner may treat as deletion),
   an error swallowed so the caller reports success, a retry that is not
   safe to repeat (only whole transactions with no side effects may go
   through `neo_core::db::retry_busy`).
5. Frontend state (Svelte 5). A stale async result applied after the user
   moved on (track switch, page change, search typed ahead), `$effect` that
   loops or misses a dependency, `$derived` over a non-reactive value, state
   kept across logout or account switch, timers and listeners not cleaned up.
6. Spec drift. Behaviour that differs from the book chapter for this area in
   a way a user or operator would notice. Quote the book sentence.
7. Tests. A test the change adds that would still pass with the bug it is
   meant to catch. Report it with that bug as the trigger.
