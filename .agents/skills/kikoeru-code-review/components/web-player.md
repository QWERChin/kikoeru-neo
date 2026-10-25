Component: frontend player and service worker

Lenses: correctness, contract

Files in scope: web/src/lib/player/ (all files, including tests),
web/src/service-worker/, web/src/lib/sw-update.ts,
web/src/routes/(fullscreen)/player/+page.svelte.

Book chapters: frontend.md, media.md (client side), contract.md (playback and
progress), testing.md (frontend unit tests).

Look especially for: player state machine bugs (track switch while a
prepare or load is pending, stale async results applied to the new track,
play/pause races, ended events firing twice, queue index off by one after
reorder or removal); progress reporting that sends out-of-order or stale
positions, keeps reporting after a clear, or loses the final position on page
hide; resume picking the wrong position or track; Media Session handlers
bound to stale state; HLS and Vidstack source switching that leaks media
elements, listeners, or object URLs; subtitle (ASS/VTT) timing and parsing
errors; loudness normalization and volume curves applied twice; Svelte 5
rune misuse ($effect loops, $derived that captures stale values, state
mutated during render); service worker caching that serves stale app shells
after an update, caches authenticated or partial (206) responses, or never
evicts.
