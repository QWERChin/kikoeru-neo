# Lens: contract

Active when the reviewed code touches the HTTP API, its DTOs, the contract,
or the frontend code that calls the API. Owns the wire format and the
agreement between backend, frontend, desktop app, and the book.

The binding rules are in `book/src/contract.md`: snake_case
names, IDs and times as JSON numbers that stay below 2^53, times in Unix
milliseconds, nullable response fields always present as `null`, booleans not 0/1,
lowercase enum strings, 204 with no body for empty success, paginated list
responses shaped `{items, page, page_size, total}` with `page` starting at
1, every error shaped `{error: {code, message}}`, unknown `/api` paths
returning 404 `not_found` JSON (never the SPA's HTML), wrong methods
returning 405 `invalid_request`, and the gzip rules for JSON and plain
text. The contract makes exceptions for some types and endpoints, for
example string IDs for voice actors and roots, and unpaginated arrays for
the roots, circles, voice actors, tags and fonts lists. The section for the
endpoint wins over the general rules.

Hunt for:

1. A DTO in `crates/neo-core/src/dto/` whose field name, type, or
   nullability differs from `book/src/contract.md` or its sample in
   `contract/`, including serde attributes (`skip_serializing_if`,
   `default`, `flatten`, untagged enums, renames) that change the JSON.
2. ts-rs output that does not match what serde actually emits, so the
   generated TypeScript type lies to the frontend.
3. A handler that returns a status code or error code the contract does not
   list, or a different one from what the frontend checks for.
4. Frontend code (`web/src/lib/api/`, stores, pages) that reads a field the
   backend never sets, ignores `null`, assumes an order or page size the
   backend does not guarantee, or hand-writes a type for an API response
   instead of using the generated one.
5. A contract test that would still pass if a field were renamed, dropped,
   or changed type.
6. A DTO or handler change that alters the wire format with no matching
   change to `book/src/contract.md` and its sample in `contract/`.
