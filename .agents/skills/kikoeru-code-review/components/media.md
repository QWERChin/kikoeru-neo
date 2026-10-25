Component: media pipeline

Lenses: correctness

Files in scope: crates/neo-media/ (src and tests).

Book chapters: media.md, architecture.md, testing.md (media and cache coverage).

Look especially for: Range and If-Range parsing that returns the wrong bytes
or status (suffix ranges, ranges past EOF, empty files, multi-range); ffmpeg
argument construction that breaks on real file names (leading dash, spaces,
non-UTF-8, special characters in concat or subtitle filters); child processes
that keep running or become zombies after cancel, client disconnect, or
error; cache keys that collide or fail to change when the source changes;
half-written cache files that are later served as complete; concurrent
prepare of the same track creating two jobs or reading a file while it is
written; HLS playlists or segment numbering that skip or repeat audio;
duration and probe parsing errors; zip entries read with wrong offsets or
encodings; subtitle conversion that shifts or drops cues; disk-full handling.
