Component: frontend API layer, stores, and shared helpers

Lenses: correctness, contract, auth

Files in scope: web/src/lib/api/ (client, transport, query, sse, media,
errors, cover-cookie, and the mock files only where non-test code depends on
them), web/src/lib/stores/, and the top-level helpers in web/src/lib/*.ts
(search-state, search-input, recent-searches, theme, accent, require-session,
navigation, secure-context, storage-error, debounce, format, cover-status,
cover-color, metadata-form, password-form, review-form, track-tree), plus
web/src/app.d.ts and web/src/routes/+layout.ts, web/src/routes/+page.ts,
web/src/routes/(app)/+layout.ts.

Book chapters: frontend.md, contract.md, api.md, auth.md (client side).

Look especially for: out-of-order responses overwriting newer state (search
as you type, pagination, filters); caches not cleared on logout or account
switch, so one user sees another user's data; 401 handling that loops or
drops the user's action; SSE reconnect that duplicates events or never
reconnects; request cancellation that leaves loading state stuck; URL
search-state round-trips that lose or corrupt filters (encoding, arrays,
defaults); localStorage values from older versions that crash parsing;
hand-written types that disagree with the generated ts-rs types; debounce
and throttle helpers with wrong trailing-call semantics.
