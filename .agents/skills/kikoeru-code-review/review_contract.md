# Review contract

This prompt was built by the kikoeru-code-review launcher. It already holds
everything the review needs. Do not load the kikoeru-code-review skill, do
not read its SKILL.md, and do not run its scripts. In particular, never start
another `codex` process: the launcher limits how many Codex reviews run on
the machine at once, and a nested one would bypass that limit.

You are reviewing kikoeru-neo, a self-hosted player for audio dramas and
audiobooks whose metadata comes from DLsite.
The backend is Rust (axum, tokio, SQLx on SQLite and PostgreSQL, ffmpeg
subprocesses). The frontend is Svelte 5 + SvelteKit, built as static files
and embedded in the server. The desktop app is Tauri. Packaging is Nix.

## Ground rules

- The review is static. The sandbox is read-only. Do not build, test,
  format, or modify anything, and do not run cargo, pnpm, npm, nix, ffmpeg,
  or network commands. Reading with rg, sed, cat, nl, git log, git show,
  git diff, and git blame is fine.
- Read `AGENTS.md` first, then the book chapters the review input names.
  The book under `book/src/` (Simplified Chinese) is the spec.
  `book/src/contract.md` and the samples in `contract/` are the binding API
  contract. `book/src/open-questions.md` records established product decisions
  and open questions. Distinguish a deliberate product boundary from an
  implementation that violates its contract. An unchecked item in
  `book/src/roadmap.md` is known, unfinished work: report it only when the
  code claims to do it and does it wrong.
- The REVIEW INPUT at the end of this prompt is the unit of review. You may
  read any file in the repository for context, and you should follow callers
  and callees as far as needed to prove a trigger path. Report only defects
  whose root cause is inside the reviewed code (in diff mode, code the change
  adds or modifies, or existing code the change newly makes reachable).

## What counts as a finding

Report correctness problems: the code produces a wrong result, loses or
corrupts data, deadlocks, races, leaks a resource that accumulates in a
running deployment, breaks the API contract, lets a user do something the
spec forbids, or diverges from the spec in a way a user or operator would
notice. A test the change adds that cannot fail counts too, with the bug it
would let through as the trigger. A missing test on its own does not.

Do not report defensive-programming suggestions. That means no "add
validation in case", no "this could panic if the invariant ever breaks" when
every caller upholds the invariant, no "consider handling the error" when
the error cannot occur, and no hardening without a concrete trigger in this
codebase. Do not report style, naming, comments, documentation wording,
refactoring, or commit hygiene.

Every finding needs a concrete trigger: real inputs, state, and ordering of
events that a real deployment can reach, and the wrong outcome the user or
operator observes. Be adversarial while searching. Before you dismiss a
suspected defect, state the input or interleaving that would trigger it, and
drop it only if you can show that case cannot happen. Before you report one,
re-read the code path and its callers to confirm the trigger is reachable
and is not handled somewhere else. If you cannot construct a trigger, do not
report it.

## Severity

- P0: data loss or corruption, a security hole reachable by an
  unauthenticated or non-admin user, or a crash or hang in a common
  production path.
- P1: wrong behaviour in a common flow, a race a normal deployment hits, a
  resource leak that grows in production, or a contract break the frontend
  or desktop app depends on.
- P2: wrong behaviour in an edge case a real user can still reach,
  including a request or task that fails or times out on a real library
  (tens of thousands of tracks, remote storage).
- P3: minor wrong behaviour with a cheap workaround.

## Lenses and delegation

The LENS blocks below list what to hunt for, specific to this repository.
Work every lens. If you can delegate to sub-agents, start one per lens from
this whole prompt, so it keeps this contract and the review input, and tell
it which lens it owns. Then merge their candidate findings yourself. A
sub-agent follows this contract too: it never starts a `codex` process,
never builds or tests, and reports only correctness defects with a trigger.

## Verification

After collecting candidate findings, verify each one on its own. Isolate
the premise it rests on (a caller never passes X, SQLite orders NULLs first,
the frontend reads field Y) and try to refute it by re-reading the code and
the spec. Then:

- confirmed: keep it with `verdict` "confirmed".
- uncertain: keep it with `verdict` "uncertain" and say in `evidence` what
  you could not verify.
- refuted: move it to `retracted` with a one-sentence reason.

Retract only on a confident refutation. Merge findings that share one root
cause into one finding and list the other locations in `related_locations`.

## Output

Return JSON that matches the provided schema. Write every human-readable
field in Simplified Chinese, in complete sentences. Keep code identifiers,
paths, and quoted code as they are, in backticks. `file` is repo-relative
and `line` is the line of the root cause in the current working tree. Order
findings from most to least severe. If there are no defects, return empty
`findings` and say so in `summary`.
