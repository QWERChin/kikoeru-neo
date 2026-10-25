Component: API contract and DTOs

Lenses: correctness, contract

Files in scope:
- crates/neo-core/src/lib.rs, crates/neo-core/src/dto/, crates/neo-core/src/progress.rs, crates/neo-core/src/review.rs
- crates/neo-typegen/
- contract/ (JSON samples) and the contract tests in crates/*/tests that load them
- book/src/contract.md as the binding wire-format spec
- book/src/api.md for API design and call flows

Book chapters: contract.md, api.md, data-model.md (only the parts DTOs map to).

Look especially for: DTO fields whose name, type, or nullability differs from
contract.md or the samples; serde attributes (rename, skip_serializing_if,
default, flatten, untagged enums) that make the wire format differ from the
spec or from the generated TypeScript type; integers that can exceed 2^53
on the wire; ts-rs output that does not match serde's actual JSON; contract
tests that would still pass if a field were wrong; logic in progress.rs and
review.rs (listening-state truth table, merge rules) that disagrees with the
book.
