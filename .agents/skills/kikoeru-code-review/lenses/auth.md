# Lens: auth

Active when the reviewed code touches routing, authentication, access
checks, tokens, passwords, rate limiting, the API client's auth handling, or
the desktop app's IPC permissions. Owns access-control correctness against
`book/src/auth.md`. This is about the spec being enforced, not about
hardening that the spec does not ask for.

The spec's rules to check against:

- API tokens are HS256 JWTs with `aud = api`, valid 30 days. Media tokens
  have `aud = media`, valid 6 hours, and only work with GET and HEAD on the
  media routes listed in `book/src/auth.md`. A media token can
  never call another route or obtain an API token.
- Every verification reads the user row: the user must still exist and
  `ver` must equal the current `token_version`. Password changes, `user
  set-password` and `POST /api/auth/logout-all` bump `token_version`.
  `user delete` invalidates tokens by removing the user row. All these
  operations revoke both API and media tokens.
  Admin rights are read from the user table on every request, never from
  the token.
- The cover routes also read the media token from the `kikoeru-media` or
  `__Secure-kikoeru-media` cookie. No other route reads those cookies.
- Password failures are counted before the password is checked, per client
  address (10 per 1-2 minutes, then 429), per user name (slow down, never
  lock out), and globally. The client address comes from the TCP peer, or
  from the last `X-Forwarded-For` entry only when the peer is in
  `trusted_proxies`. IPv6 counts by /64.
- The last admin cannot be removed.

Hunt for:

1. A route mounted outside the auth or admin layer it needs, or a new route
   a media token can reach.
2. A token path that skips the user-row check, so a revoked token still
   works.
3. A non-admin user reaching admin behaviour, or one user reading or
   changing another user's progress, favourites, reviews, or settings.
4. Counting that happens after the password check, or that a concurrent
   request can slip past.
5. In the desktop app, a Tauri command or capability the web view can reach
   that the book does not intend, given that the web view renders library
   metadata fetched from the internet.
