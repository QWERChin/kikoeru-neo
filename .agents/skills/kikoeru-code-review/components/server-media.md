Component: server playback and media endpoints

Lenses: correctness, auth, contract

Files in scope (crates/neo-server/src/): http/playback.rs, http/media.rs,
http/video.rs, http/archive.rs, http/covers.rs, http/progress.rs,
http/analysis.rs, http/web.rs, and their tests in crates/neo-server/tests/
(api_playback.rs, api_media.rs, api_video.rs, api_archive.rs,
api_progress.rs, api_analysis.rs, web.rs).

Book chapters: media.md, api.md, contract.md (playback and progress parts),
frontend.md (static serving and SPA fallback), testing.md.

The current contract shares libraries among authenticated users; it does
not define private works or hidden roots.

Look especially for: unauthenticated access to media, wrong token audiences
or access to another user's progress; HEAD requests
that start work; streaming responses that keep ffmpeg or storage reads alive
after the client disconnects; wrong Content-Length, Content-Range, ETag, or
cache headers that make browsers or iOS Safari replay stale or partial data;
progress writes that go backwards or lose updates under concurrent devices;
archive and cover paths that escape their root; SPA fallback that returns
index.html for missing API routes or assets (the spec forbids it); frontend
assets served from http/web.rs with wrong MIME types or caching.
