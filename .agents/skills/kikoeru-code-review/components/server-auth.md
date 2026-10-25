Component: server auth and HTTP foundation

Lenses: correctness, auth, contract

Files in scope (crates/neo-server/src/): http/mod.rs (router), http/auth.rs,
http/access.rs, http/extract.rs, http/error.rs, http/limiter.rs,
http/system.rs, token.rs, password.rs, user_cmd.rs, private.rs, lib.rs, and
the tests in crates/neo-server/tests/ that exercise them (api_auth.rs,
process_auth.rs).

Book chapters: auth.md, api.md, testing.md (authentication coverage).

Look especially for: routes mounted without the auth or admin layer the spec
requires; media tokens accepted on non-media routes; tokens that stay valid
after password change, user deletion, or permission change; deleting or
demoting the last admin if a role-changing interface is introduced (there
is currently no public role-change API); timing or comparison bugs in token and password
checks; the cover cookie read anywhere but the cover routes; rate limiting
that departs from auth.md, such as trusting X-Forwarded-For from a peer
outside `trusted_proxies`, using an entry other than the last, counting
after the password check, or windows that never expire; error responses
that expose internal fields or return a status the frontend does not
expect. auth.md accepts that clients behind one proxy without
`trusted_proxies` share a limit, and frontend.md deploys the app at the
site root only, so neither is a defect.
