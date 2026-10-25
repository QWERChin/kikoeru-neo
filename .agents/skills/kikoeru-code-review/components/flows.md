Component: cross-component main flows

Lenses: correctness, contract

This review is not limited to one directory. Trace each flow below end to
end through the frontend (web/src), the backend (crates/), and the spec
(book/src), and report places where the layers disagree in a way a user can
observe. Report a finding against the file whose behaviour is wrong
according to the spec; if the spec itself is inconsistent or silent on
something the code must decide, report it as spec-drift against the book
page.

Flows:
1. Login, session restore after reload, logout, and account switch.
2. Browsing and searching the library: filters, sort, pagination, tag and
   circle pages, recent and favorites.
3. Opening a work's detail page (including a cold deep link) and its track
   tree.
4. Playback: direct play, transcode and prepare, HLS, Range seeking, video,
   subtitles, on desktop browsers, iOS Safari, and the Tauri app.
5. Progress and listening state: reporting, resume on another device,
   clearing progress, the listening-state icons.
6. Scanning: adding a root, scan, metadata fetch, missing works, re-scan,
   cancel, and how the frontend shows each task state.
7. Editing metadata and reviews, and what a later scan or metadata refresh
   does to those edits.

Book chapters: all of book/src that these flows touch, especially
contract.md, api.md, frontend.md, scanning.md, media.md, auth.md.

Do not repeat findings that are purely inside one component (other reviews
cover those). Focus on mismatches between layers: a field the frontend reads
that the backend never sets, a status code the frontend does not handle, an
ordering assumption one side makes and the other breaks, a state transition
the spec describes that neither side implements.
