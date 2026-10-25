---
name: kikoeru-code-review
description: Review kikoeru-neo code for concrete correctness defects with the local Codex CLI and write a Markdown report. Use when an AI-assisted review of a branch, selected files, or a component is requested.
---

# kikoeru-code-review

Review a committed revision for correctness defects with concrete triggers.
The launcher produces a Markdown report and machine-readable findings. This
is an optional contributor tool, not a requirement to obtain an AI account.
The specification in `book/src/` is maintained in Simplified Chinese only.
Book chapter names in `components/` are relative to that directory.

## Run a review

Run the launcher from the repository root. It requires the Codex CLI with
working authentication, Git, Bash, Python 3, `flock` and `timeout`.

```sh
.agents/skills/kikoeru-code-review/scripts/codex_review.sh diff origin/main /tmp/review-branch.md
.agents/skills/kikoeru-code-review/scripts/codex_review.sh files crates/neo-media/src/range.rs /tmp/review.md
.agents/skills/kikoeru-code-review/scripts/codex_review.sh component server-auth /tmp/review.md
```

- `diff <base>` reviews the commits HEAD adds over its merge base with the
  base, including commit messages and diffs.
- `files <path>...` reviews the named files or directories in full.
- `component <id>` uses `components/<id>.md` to define scope.

For a requested whole-repository review, run:

```sh
.agents/skills/kikoeru-code-review/scripts/review_all.sh /tmp/kikoeru-review
```

It combines component reports into `report.md` and reuses component reports
that already cover the selected revision.

Every mode reviews committed files in an isolated clone, not working-tree
edits. Confirm that the selected revision includes the intended changes.
The tool does not commit changes on the caller's behalf.

The output consists of the requested `.md` file, raw `.json`, reviewed
commit in `.head`, and process output in `.log`. Existing outputs are
refused unless `--overwrite` is supplied. An overwrite removes the old
report before starting, so a failed rerun cannot look like a fresh success.

Use the launcher rather than starting a nested review process. It manages
repository-wide concurrency across worktrees and stops its child process
on cancellation. A waiting whole-repository review gives priority to a
branch review. Wait for completion before treating an output as final.

## Configuration and review input

`KCR_MODEL`, `KCR_EFFORT`, `KCR_TIER`, `KCR_TIMEOUT` and `KCR_SLOTS` configure
the launcher. Current executable defaults are in `scripts/codex_review.sh`.
They are tool defaults, not contributor requirements. `KCR_HEAD` selects a
commit other than HEAD; the whole-repository launcher uses it to keep all
component reviews on one revision.

The launcher constructs a prompt from `review_contract.md`, the active
files in `lenses/`, and the selected review input. Path-based rules in
`scripts/build_prompt.sh` choose lenses. `correctness` always applies;
contract, authentication and data changes add their corresponding lenses.
Moved paths are considered at both their old and new locations.

The review runs read-only with a JSON output schema and without personal
plugins, memories, hooks or apps. `scripts/render_review.py` converts the
result into Markdown. If running inside that review process, follow the
provided review contract and do not invoke this skill again.

## Act on findings

Verify each reported trigger against the code and specification before
changing anything. Fix confirmed defects with regression coverage, and
explain rejected or deferred findings. Use the repository's review and
merge rules in `AGENTS.md`; this tool grants no permission to push, merge
or publish. Do not keep rerunning reviews without an agreed scope or stop
condition. A clean report does not replace tests or prove correctness.

## Maintain the tool

- Keep shared review rules in `review_contract.md` and area-specific
  knowledge in the relevant lens or component.
- A new lens needs its file, activation rules in `scripts/build_prompt.sh`
  and an entry in the `lens` enum of `schema.json`.
- Each component needs a `Lenses:` line for deterministic prompt selection.
- `tests/codex-review.bats` tests the launcher with a fake CLI. Run the
  flake's `codex-review` check after changing the tool.
