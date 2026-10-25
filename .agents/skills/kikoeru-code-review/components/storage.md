Component: storage backends

Lenses: correctness

Files in scope: crates/neo-storage/ (src and tests), crates/webdav-client/
(src and tests). The OpenList client itself is pinned from
https://github.com/chinrw/openlist-client by rev in the root Cargo.toml; read
its pinned source from an available Cargo cache only for context. If it is
not available, report that limitation rather than assuming a local checkout.

Book chapters: storage.md, scanning.md (storage parts), testing.md
(storage coverage).

Look especially for: path joining and normalization that escapes the root,
double-encodes, or mismatches between backends (percent-encoding, trailing
slashes, Unicode NFC/NFD, Windows separators and drive letters in
local_windows.rs); listing pagination that skips or repeats entries;
re-login and expired direct-link handling that loops or returns stale links;
upstream servers that ignore Range; error mapping that turns a transient
failure into "not found" (which the scanner may treat as deletion); WebDAV
PROPFIND XML parsing of real servers (namespaces, hrefs with host, encoded
names); symlink handling in local storage.
