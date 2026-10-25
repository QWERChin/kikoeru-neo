#!/usr/bin/env bats
# Tests for the kikoeru-code-review scripts against a fake Codex and a fake
# /proc, in a throwaway git repository. No real Codex runs.

setup() {
  here=$(cd "$(dirname "$BATS_TEST_FILENAME")" && pwd)
  scripts=$(cd "$here/../scripts" && pwd)
  review=$scripts/codex_review.sh
  export KCR_CODEX=$here/fake-codex
  export KCR_PROC_DIR=$BATS_TEST_TMPDIR/proc
  export FAKE_DIR=$BATS_TEST_TMPDIR/fake
  export KCR_LOCK_DIR=$BATS_TEST_TMPDIR/locks
  export KCR_POLL=0.1
  unset KCR_REVIEW_RUNNING KCR_LOW_PRIORITY KCR_SLOTS KCR_MODEL KCR_EFFORT KCR_TIER
  mkdir -p "$FAKE_DIR" "$KCR_PROC_DIR"

  repo=$BATS_TEST_TMPDIR/repo
  git init -q -b main "$repo"
  cd "$repo"
  git config user.email test@example.com
  git config user.name Test
  mkdir -p src
  printf 'fn main() {}\n' >src/lib.rs
  git add -A
  git commit -qm 'chore: start'
  git checkout -qb feature
}

# Commits a change to PATH with MESSAGE.
change() {
  mkdir -p "$(dirname "$1")"
  printf '%s\n' "${3:-changed}" >>"$1"
  git add -A
  git commit -qm "$2"
}

# Lists a process with the given command line in the fake /proc, working in
# $PROC_CWD (the test repository by default) with the environment in
# $PROC_ENV.
process() {
  local pid=$1
  shift
  mkdir -p "$KCR_PROC_DIR/$pid"
  printf '%s\0' "$@" >"$KCR_PROC_DIR/$pid/cmdline"
  printf '%s\0' HOME=/home/x ${PROC_ENV:+"$PROC_ENV"} >"$KCR_PROC_DIR/$pid/environ"
  ln -s "${PROC_CWD:-$repo}" "$KCR_PROC_DIR/$pid/cwd"
}

# Bats runs each test under `set -e`, which ignores a failing `! cmd`
# unless it is the last command, so negative checks go through refute,
# which fails the test itself.
refute() {
  if "$@"; then
    echo "expected this to fail: $*" >&2
    return 1
  fi
}

prompt_has() {
  prompt | grep -q -- "$1"
}

calls() {
  if [[ -f $FAKE_DIR/calls ]]; then wc -l <"$FAKE_DIR/calls"; else echo 0; fi
}

prompt() {
  cat "$FAKE_DIR"/prompt.*
}

@test "diff review runs Codex read-only at effort high on the standard tier" {
  change src/lib.rs 'feat: add the thing'
  run "$review" diff main "$BATS_TEST_TMPDIR/out/review.md"
  [ "$status" -eq 0 ]
  grep -q -- 'exec ' "$FAKE_DIR/calls"
  grep -q -- '-s read-only' "$FAKE_DIR/calls"
  grep -q -- '-m gpt-6.1-sol' "$FAKE_DIR/calls"
  grep -q -- 'model_reasoning_effort="high"' "$FAKE_DIR/calls"
  grep -q -- 'service_tier="default"' "$FAKE_DIR/calls"
  grep -q -- 'approval_policy="never"' "$FAKE_DIR/calls"
  grep -q -- '--output-schema' "$FAKE_DIR/calls"
  [[ "$output" == *"P1 1"* ]]
}

@test "diff review writes the Markdown, JSON and log next to each other" {
  change src/lib.rs 'feat: add the thing'
  run "$review" diff main "$BATS_TEST_TMPDIR/out/review.md"
  [ "$status" -eq 0 ]
  [ -f "$BATS_TEST_TMPDIR/out/review.json" ]
  [ -f "$BATS_TEST_TMPDIR/out/review.log" ]
  grep -q '示例发现' "$BATS_TEST_TMPDIR/out/review.md"
  grep -q '`src/lib.rs:3`' "$BATS_TEST_TMPDIR/out/review.md"
  grep -q '撤回的示例' "$BATS_TEST_TMPDIR/out/review.md"
}

@test "diff prompt holds the contract, the lens and every commit's message and diff" {
  change src/lib.rs 'feat: add the first thing' 'first-line'
  change src/lib.rs 'fix: correct the first thing' 'second-line'
  run "$review" diff main "$BATS_TEST_TMPDIR/review.md"
  [ "$status" -eq 0 ]
  prompt | grep -q '^# Review contract'
  prompt | grep -q '^===== LENS: correctness ====='
  prompt | grep -q 'feat: add the first thing'
  prompt | grep -q 'fix: correct the first thing'
  prompt | grep -q '^+first-line'
  prompt | grep -q '^+second-line'
  # Only the change is under review, not the base commit.
  refute prompt_has 'chore: start'
}

@test "diff prompt leaves out lock files and the SQLx cache" {
  change src/lib.rs 'feat: add the thing'
  change Cargo.lock 'chore: bump' 'lock-content'
  change crates/neo-db-sqlite/.sqlx/query-1.json 'chore: prepare' 'sqlx-content'
  run "$review" diff main "$BATS_TEST_TMPDIR/review.md"
  [ "$status" -eq 0 ]
  refute prompt_has 'lock-content'
  refute prompt_has 'sqlx-content'
}

@test "diff prompt includes what a merge commit itself changed" {
  printf 'base\n' >src/conflict.txt
  git add -A
  git commit -qm 'chore: add conflict file'
  git checkout -qb side
  printf 'side\n' >src/conflict.txt
  git commit -qam 'feat: side change'
  git checkout -q feature
  printf 'feature\n' >src/conflict.txt
  git commit -qam 'feat: feature change'
  git merge -q side 2>/dev/null || true
  printf 'resolved-by-merge\n' >src/conflict.txt
  git commit -qam 'chore: merge side'
  run "$review" diff main "$BATS_TEST_TMPDIR/review.md"
  [ "$status" -eq 0 ]
  prompt | grep -q 'resolved-by-merge'
}

@test "diff with no new commits is refused before Codex runs" {
  run "$review" diff main "$BATS_TEST_TMPDIR/review.md"
  [ "$status" -ne 0 ]
  [[ "$output" == *"no commits"* ]]
  [ "$(calls)" -eq 0 ]
}

@test "lenses follow the changed paths" {
  change web/src/routes/page.svelte 'feat: page'
  run "$scripts/build_prompt.sh" --lenses diff main
  [ "$output" = "correctness" ]

  change crates/neo-core/migrations/sqlite/0020_x.sql 'feat: migrate'
  run "$scripts/build_prompt.sh" --lenses diff main
  [ "$output" = "$(printf 'correctness\ndata')" ]

  change crates/neo-server/src/http/auth.rs 'feat: auth'
  run "$scripts/build_prompt.sh" --lenses diff main
  [ "$output" = "$(printf 'correctness\ncontract\nauth\ndata')" ]
}

@test "a directory brings the lenses of the files inside it" {
  change crates/neo-server/src/http/auth.rs 'feat: auth'
  run "$scripts/build_prompt.sh" --lenses files crates/neo-server/src/http
  [ "$output" = "$(printf 'correctness\ncontract\nauth')" ]
}

@test "moving a file out of a lens's directory still brings that lens" {
  mkdir -p crates/neo-server/src/http
  printf 'auth\n' >crates/neo-server/src/http/auth.rs
  git add -A
  git commit -qm 'feat: auth'
  git checkout -q main
  git merge -q --ff-only feature
  git checkout -q feature
  git mv crates/neo-server/src/http/auth.rs crates/neo-server/src/auth.rs
  git commit -qm 'refactor: move auth'
  run "$scripts/build_prompt.sh" --lenses diff main
  [[ "$output" == *auth* ]]
}

@test "files given from a subdirectory are listed relative to the repository" {
  cd src
  run "$review" files lib.rs "$BATS_TEST_TMPDIR/review.md"
  [ "$status" -eq 0 ]
  prompt | grep -q '^- src/lib.rs$'
}

@test "a missing file is refused before Codex runs" {
  run "$review" files nope.rs "$BATS_TEST_TMPDIR/review.md"
  [ "$status" -ne 0 ]
  [ "$(calls)" -eq 0 ]
}

@test "component review uses the component's description and lenses" {
  run "$review" component server-auth "$BATS_TEST_TMPDIR/review.md"
  [ "$status" -eq 0 ]
  prompt | grep -q '^Mode: component'
  prompt | grep -q 'http/auth.rs'
  prompt | grep -q '^===== LENS: auth ====='
  prompt | grep -q '^===== LENS: contract ====='
  refute prompt_has '^===== LENS: data ====='
}

@test "every component names lenses that exist" {
  for file in "$here"/../components/*.md; do
    id=$(basename "$file" .md)
    run "$scripts/build_prompt.sh" --lenses component "$id"
    [ "$status" -eq 0 ]
    for lens in $output; do
      [ -f "$here/../lenses/$lens.md" ]
    done
  done
}

@test "an unknown component is refused" {
  run "$review" component nope "$BATS_TEST_TMPDIR/review.md"
  [ "$status" -ne 0 ]
  [[ "$output" == *"unknown component"* ]]
  [ "$(calls)" -eq 0 ]
}

@test "an existing review is kept unless --overwrite is given" {
  change src/lib.rs 'feat: add the thing'
  echo old >"$BATS_TEST_TMPDIR/review.md"
  run "$review" diff main "$BATS_TEST_TMPDIR/review.md"
  [ "$status" -ne 0 ]
  [ "$(cat "$BATS_TEST_TMPDIR/review.md")" = old ]
  run "$review" diff main "$BATS_TEST_TMPDIR/review.md" --overwrite
  [ "$status" -eq 0 ]
  grep -q '示例发现' "$BATS_TEST_TMPDIR/review.md"
}

@test "a review started from inside a review is refused" {
  change src/lib.rs 'feat: add the thing'
  KCR_REVIEW_RUNNING=1 run "$review" diff main "$BATS_TEST_TMPDIR/review.md"
  [ "$status" -eq 3 ]
  [ "$(calls)" -eq 0 ]
}

@test "the review names the commit it was built from even if HEAD moves" {
  change src/lib.rs 'feat: add the thing'
  reviewed=$(git rev-parse HEAD)
  export REPO=$repo
  FAKE_HOOK='git -C "$REPO" commit -q --allow-empty -m moved' run "$review" diff main "$BATS_TEST_TMPDIR/review.md"
  [ "$status" -eq 0 ]
  [ "$(git rev-parse HEAD)" != "$reviewed" ]
  [ "$(cat "$BATS_TEST_TMPDIR/review.head")" = "$reviewed" ]
  grep -q "HEAD \`${reviewed:0:9}\`" "$BATS_TEST_TMPDIR/review.md"
}

@test "Codex reads the committed tree, not uncommitted edits" {
  printf 'uncommitted-edit\n' >>src/lib.rs
  FAKE_HOOK='cat src/lib.rs >"$FAKE_DIR/seen"; pwd >"$FAKE_DIR/tree"' \
    run "$review" files src/lib.rs "$BATS_TEST_TMPDIR/review.md"
  [ "$status" -eq 0 ]
  grep -q 'fn main' "$FAKE_DIR/seen"
  refute grep -q 'uncommitted-edit' "$FAKE_DIR/seen"
  tree=$(cat "$FAKE_DIR/tree")
  [ "$tree" != "$repo" ]
  [ ! -e "$tree" ]
  [ ! -e "$tree.prompt" ]
}

@test "a base given as an expression is resolved in this repository" {
  change src/lib.rs 'feat: first' one
  first=$(git rev-parse HEAD)
  change src/lib.rs 'feat: second' two
  change src/lib.rs 'feat: third' three
  # origin/main and the local main differ, and the clone maps this
  # repository's branches to its origin/ refs.
  git update-ref refs/remotes/origin/main "$first"
  git branch -qf main HEAD
  run "$review" diff origin/main~1 "$BATS_TEST_TMPDIR/review.md"
  [ "$status" -eq 0 ]
  prompt | grep -q 'feat: first'
  prompt | grep -q 'feat: third'
  prompt | grep -q '^origin/main~1 (merge base'
}

@test "two reviews cannot write the same output at once" {
  change src/lib.rs 'feat: add the thing'
  FAKE_SLEEP=2 "$review" diff main "$BATS_TEST_TMPDIR/review.md" 2>/dev/null &
  pid=$!
  for _ in $(seq 50); do
    [ "$(calls)" -eq 1 ] && break
    sleep 0.1
  done
  run "$review" diff main "$BATS_TEST_TMPDIR/review.md" --overwrite
  [ "$status" -eq 2 ]
  [[ "$output" == *"another review is writing"* ]]
  wait "$pid"
  [ "$(calls)" -eq 1 ]
}

@test "a failed rerun does not leave the previous review in place" {
  change src/lib.rs 'feat: add the thing'
  run "$review" diff main "$BATS_TEST_TMPDIR/review.md"
  [ "$status" -eq 0 ]
  FAKE_STATUS=1 run "$review" diff main "$BATS_TEST_TMPDIR/review.md" --overwrite
  [ "$status" -eq 1 ]
  [ ! -e "$BATS_TEST_TMPDIR/review.md" ]
}

@test "stopping the launcher stops Codex and frees the slot" {
  change src/lib.rs 'feat: add the thing'
  export KCR_SLOTS=1
  FAKE_SLEEP=30 "$review" diff main "$BATS_TEST_TMPDIR/a.md" 2>/dev/null &
  pid=$!
  for _ in $(seq 50); do
      [ -n "$(ls "$KCR_PROC_DIR")" ] && break
    sleep 0.1
  done
  [ -n "$(ls "$KCR_PROC_DIR")" ]
  kill -TERM "$pid"
  wait "$pid" || true
  [ -z "$(ls "$KCR_PROC_DIR")" ]
  run timeout 10 "$review" diff main "$BATS_TEST_TMPDIR/b.md"
  [ "$status" -eq 0 ]
}

@test "a failed Codex run fails the review and writes no Markdown" {
  change src/lib.rs 'feat: add the thing'
  FAKE_STATUS=1 run "$review" diff main "$BATS_TEST_TMPDIR/review.md"
  [ "$status" -eq 1 ]
  [ ! -e "$BATS_TEST_TMPDIR/review.md" ]
}

# Prints "start-of-later end-of-earlier" from the fake's timing log for two
# calls, so a test can check that they did not overlap.
later_start_and_earlier_end() {
  local first
  first=$(awk '$1 == "start" {print $2; exit}' "$FAKE_DIR/times")
  awk -v first="$first" '
    $1 == "end" && $2 == first {end = $3}
    $1 == "start" && $2 != first {start = $3}
    END {print start, end}' "$FAKE_DIR/times"
}

@test "reviews beyond the slot count wait for a free slot" {
  change src/lib.rs 'feat: add the thing'
  export KCR_SLOTS=1 FAKE_SLEEP=0.5
  "$review" diff main "$BATS_TEST_TMPDIR/a.md" 2>/dev/null &
  "$review" diff main "$BATS_TEST_TMPDIR/b.md" 2>/dev/null &
  wait
  [ "$(calls)" -eq 2 ]
  read -r start end < <(later_start_and_earlier_end)
  awk -v s="$start" -v e="$end" 'BEGIN {exit !(s >= e)}'
}

@test "two slots let two reviews run at once" {
  change src/lib.rs 'feat: add the thing'
  export FAKE_SLEEP=1
  "$review" diff main "$BATS_TEST_TMPDIR/a.md" 2>/dev/null &
  "$review" diff main "$BATS_TEST_TMPDIR/b.md" 2>/dev/null &
  wait
  [ "$(calls)" -eq 2 ]
  read -r start end < <(later_start_and_earlier_end)
  awk -v s="$start" -v e="$end" 'BEGIN {exit !(s < e)}'
}

@test "a Codex review started outside the launcher takes a slot" {
  change src/lib.rs 'feat: add the thing'
  process 1 /nix/store/x-codex/bin/codex-raw review --base origin/main
  export KCR_SLOTS=1
  "$review" diff main "$BATS_TEST_TMPDIR/review.md" 2>/dev/null &
  pid=$!
  sleep 0.5
  [ "$(calls)" -eq 0 ]
  rm -r "$KCR_PROC_DIR/1"
  wait "$pid"
  [ "$(calls)" -eq 1 ]
}

@test "an outside review whose arguments contain spaces still takes a slot" {
  change src/lib.rs 'feat: add the thing'
  mkdir -p "$repo/dir with space"
  process 1 /nix/store/x-codex/bin/codex -C "$repo/dir with space" review --base main
  export KCR_SLOTS=1
  "$review" diff main "$BATS_TEST_TMPDIR/review.md" 2>/dev/null &
  pid=$!
  sleep 0.5
  [ "$(calls)" -eq 0 ]
  rm -r "$KCR_PROC_DIR/1"
  wait "$pid"
  [ "$(calls)" -eq 1 ]
}

@test "two launchers cannot both take the last slot left by an outside review" {
  change src/lib.rs 'feat: add the thing'
  process 1 /nix/store/x-codex/bin/codex-raw review --base origin/main
  export FAKE_SLEEP=1
  "$review" diff main "$BATS_TEST_TMPDIR/a.md" 2>/dev/null &
  "$review" diff main "$BATS_TEST_TMPDIR/b.md" 2>/dev/null &
  sleep 2.5
  rm -r "$KCR_PROC_DIR/1"
  wait
  [ "$(calls)" -eq 2 ]
  read -r start end < <(later_start_and_earlier_end)
  awk -v s="$start" -v e="$end" 'BEGIN {exit !(s >= e)}'
}

@test "a review started with options before the subcommand takes a slot" {
  change src/lib.rs 'feat: add the thing'
  process 1 /nix/store/x-codex/bin/codex-raw -c model_reasoning_effort=ultra review --base origin/main
  process 2 /nix/store/x-codex/bin/codex-raw -m gpt-6-astra exec -C "$repo" review --base origin/main
  "$review" diff main "$BATS_TEST_TMPDIR/review.md" 2>/dev/null &
  pid=$!
  sleep 0.5
  [ "$(calls)" -eq 0 ]
  rm -r "$KCR_PROC_DIR/1"
  wait "$pid"
  [ "$(calls)" -eq 1 ]
}

@test "an ordinary codex exec, such as an agent waiting for its review, does not take a slot" {
  change src/lib.rs 'feat: add the thing'
  process 1 /nix/store/x-codex/bin/codex-raw exec 'fix the bug and run the review'
  process 2 /nix/store/x-codex/bin/codex-raw -m gpt-6-astra e -C /repo -
  export KCR_SLOTS=1
  run timeout 5 "$review" diff main "$BATS_TEST_TMPDIR/review.md"
  [ "$status" -eq 0 ]
  [ "$(calls)" -eq 1 ]
}

@test "a review of another project does not take a slot" {
  change src/lib.rs 'feat: add the thing'
  mkdir -p "$BATS_TEST_TMPDIR/other"
  PROC_CWD=$BATS_TEST_TMPDIR/other process 1 /nix/store/x-codex/bin/codex-raw review --base main
  PROC_ENV=KCR_REVIEW_RUNNING=/elsewhere/.git process 2 /nix/store/x-codex/bin/codex-raw exec -C /elsewhere -
  export KCR_SLOTS=1
  run timeout 5 "$review" diff main "$BATS_TEST_TMPDIR/review.md"
  [ "$status" -eq 0 ]
  [ "$(calls)" -eq 1 ]
}

@test "a review pointed at this repository with -C takes a slot" {
  change src/lib.rs 'feat: add the thing'
  mkdir -p "$BATS_TEST_TMPDIR/other"
  PROC_CWD=$BATS_TEST_TMPDIR/other process 1 /nix/store/x-codex/bin/codex-raw -C "$repo/src" review --base main
  export KCR_SLOTS=1
  "$review" diff main "$BATS_TEST_TMPDIR/review.md" 2>/dev/null &
  pid=$!
  sleep 0.5
  [ "$(calls)" -eq 0 ]
  rm -r "$KCR_PROC_DIR/1"
  wait "$pid"
  [ "$(calls)" -eq 1 ]
}

@test "worktrees of one repository share its slots, and other repositories have their own" {
  unset KCR_LOCK_DIR
  change src/lib.rs 'feat: add the thing'
  git worktree add -q "$BATS_TEST_TMPDIR/wt" -b second
  (cd "$BATS_TEST_TMPDIR/wt" && change src/lib.rs 'feat: second thing')
  other=$BATS_TEST_TMPDIR/other-repo
  git init -q -b main "$other"
  (cd "$other" && git config user.email t@e && git config user.name T &&
    mkdir src && echo x >src/a.rs && git add -A && git commit -qm start &&
    git checkout -qb f && echo y >>src/a.rs && git commit -qam change)
  export KCR_SLOTS=1 FAKE_SLEEP=1
  "$review" diff main "$BATS_TEST_TMPDIR/a.md" 2>/dev/null &
  (cd "$BATS_TEST_TMPDIR/wt" && "$review" diff main "$BATS_TEST_TMPDIR/b.md" 2>/dev/null) &
  (cd "$other" && "$review" diff main "$BATS_TEST_TMPDIR/c.md" 2>/dev/null) &
  wait
  [ "$(calls)" -eq 3 ]
  [ -d "$repo/.git/kikoeru-code-review" ]
  [ -d "$other/.git/kikoeru-code-review" ]
  # The two reviews of this repository ran one after the other, while the
  # other repository's review overlapped with one of them.
  starts=$(awk '$1 == "start" {print $3}' "$FAKE_DIR/times" | sort -n)
  ends=$(awk '$1 == "end" {print $3}' "$FAKE_DIR/times" | sort -n)
  first_end=$(head -1 <<<"$ends")
  overlapping=$(awk -v e="$first_end" '$1 < e' <<<"$starts" | wc -l)
  [ "$overlapping" -eq 2 ]
}

@test "wrappers, app servers and interactive Codex do not take a slot" {
  change src/lib.rs 'feat: add the thing'
  process 1 timeout 4h /nix/store/x-codex/bin/codex exec -C /repo -
  process 2 /nix/store/x-codex/bin/codex-raw app-server
  process 3 /nix/store/x-codex/bin/codex-raw -c features.code_mode_host=true app-server --listen unix://
  process 4 /nix/store/x-codex/bin/codex-raw
  process 5 bash .agents/skills/kikoeru-code-review/scripts/codex_review.sh diff main out.md
  export KCR_SLOTS=1
  run timeout 5 "$review" diff main "$BATS_TEST_TMPDIR/review.md"
  [ "$status" -eq 0 ]
  [ "$(calls)" -eq 1 ]
}

@test "a low-priority review lets a waiting review go first" {
  change src/lib.rs 'feat: add the thing'
  mkdir -p "$KCR_LOCK_DIR/waiting"
  sleep 30 &
  waiter=$!
  : >"$KCR_LOCK_DIR/waiting/$waiter"
  KCR_LOW_PRIORITY=1 "$review" diff main "$BATS_TEST_TMPDIR/review.md" 2>/dev/null &
  pid=$!
  sleep 0.5
  [ "$(calls)" -eq 0 ]
  kill "$waiter"
  wait "$pid"
  [ "$(calls)" -eq 1 ]
}

@test "review_all skips finished components and merges every review" {
  out=$BATS_TEST_TMPDIR/audit
  mkdir -p "$out"
  run "$scripts/review_all.sh" "$out" web-ui storage
  [ "$status" -eq 0 ]
  [ "$(calls)" -eq 2 ]
  run "$scripts/review_all.sh" "$out" web-ui storage
  [ "$status" -eq 0 ]
  [ "$(calls)" -eq 2 ]
  grep -q '共 2 份审查' "$out/report.md"
  grep -q '\[storage\] 示例发现' "$out/report.md"
  grep -q '\[web-ui\] 示例发现' "$out/report.md"
}

@test "review_all keeps every component on the commit it started at" {
  out=$BATS_TEST_TMPDIR/audit
  started=$(git rev-parse HEAD)
  export REPO=$repo KCR_SLOTS=1
  FAKE_HOOK='git -C "$REPO" commit -q --allow-empty -m moved' \
    run "$scripts/review_all.sh" "$out" web-ui storage
  [ "$status" -eq 0 ]
  [ "$(git rev-parse HEAD)" != "$started" ]
  [ "$(cat "$out/storage.head")" = "$started" ]
  [ "$(cat "$out/web-ui.head")" = "$started" ]
}

@test "an output reached through a symlinked directory shares the output lock" {
  change src/lib.rs 'feat: add the thing'
  mkdir "$BATS_TEST_TMPDIR/real"
  ln -s real "$BATS_TEST_TMPDIR/link"
  FAKE_SLEEP=2 "$review" diff main "$BATS_TEST_TMPDIR/link/review.md" 2>/dev/null &
  pid=$!
  for _ in $(seq 50); do
    [ "$(calls)" -eq 1 ] && break
    sleep 0.1
  done
  run "$review" diff main "$BATS_TEST_TMPDIR/real/review.md" --overwrite
  [ "$status" -eq 2 ]
  wait "$pid"
}

@test "the review runs the skill as it was when it started" {
  change src/lib.rs 'feat: add the thing'
  cp -r "$here/.." "$BATS_TEST_TMPDIR/skill"
  export GONE=$BATS_TEST_TMPDIR/skill
  FAKE_HOOK='rm -rf "$GONE"' run "$BATS_TEST_TMPDIR/skill/scripts/codex_review.sh" diff main "$BATS_TEST_TMPDIR/review.md"
  [ "$status" -eq 0 ]
  [ ! -e "$BATS_TEST_TMPDIR/skill" ]
  grep -q '示例发现' "$BATS_TEST_TMPDIR/review.md"
}

@test "the launcher works when reached through a relative symlink to the skill" {
  change src/lib.rs 'feat: add the thing'
  # The same shape as .claude/skills/kikoeru-code-review, whose target only
  # resolves from where the link lives.
  cp -r "$here/.." "$BATS_TEST_TMPDIR/agents-skill"
  mkdir -p "$BATS_TEST_TMPDIR/claude/skills"
  ln -s ../../agents-skill "$BATS_TEST_TMPDIR/claude/skills/linked"
  run "$BATS_TEST_TMPDIR/claude/skills/linked/scripts/codex_review.sh" diff main "$BATS_TEST_TMPDIR/review.md"
  [ "$status" -eq 0 ]
  grep -q '示例发现' "$BATS_TEST_TMPDIR/review.md"
  [[ "$output" == *"-> $BATS_TEST_TMPDIR/review.md"* ]]
}

@test "an audit runs the skill as it was when it started" {
  cp -r "$here/.." "$BATS_TEST_TMPDIR/skill"
  export GONE=$BATS_TEST_TMPDIR/skill KCR_SLOTS=1
  FAKE_HOOK='rm -rf "$GONE"' run "$BATS_TEST_TMPDIR/skill/scripts/review_all.sh" "$BATS_TEST_TMPDIR/audit" web-ui storage
  [ "$status" -eq 0 ]
  [ ! -e "$BATS_TEST_TMPDIR/skill" ]
  [ "$(calls)" -eq 2 ]
  grep -q '共 2 份审查' "$BATS_TEST_TMPDIR/audit/report.md"
}

@test "editing the launcher in place during a review does not break it" {
  change src/lib.rs 'feat: add the thing'
  cp -r "$here/.." "$BATS_TEST_TMPDIR/skill"
  launcher=$BATS_TEST_TMPDIR/skill/scripts/codex_review.sh
  export LAUNCHER=$launcher
  # Rewrites the file in place with every line shifted, as an editor does.
  FAKE_HOOK='{ printf "# %0200d\n" 0; cat "$LAUNCHER"; } >"$LAUNCHER.new"; cat "$LAUNCHER.new" >"$LAUNCHER"' \
    run "$launcher" diff main "$BATS_TEST_TMPDIR/review.md"
  [ "$status" -eq 0 ]
  [[ "$output" != *"command not found"* ]]
  [ -f "$BATS_TEST_TMPDIR/review.head" ]
  grep -q '示例发现' "$BATS_TEST_TMPDIR/review.md"
}

@test "a review that cannot be rendered leaves no Markdown and no commit file" {
  change src/lib.rs 'feat: add the thing'
  echo '{"summary":"s"}' >"$BATS_TEST_TMPDIR/bad.json"
  FAKE_RESPONSE=$BATS_TEST_TMPDIR/bad.json run "$review" diff main "$BATS_TEST_TMPDIR/review.md"
  [ "$status" -ne 0 ]
  [ ! -e "$BATS_TEST_TMPDIR/review.md" ]
  [ ! -e "$BATS_TEST_TMPDIR/review.head" ]
}

@test "two audits cannot share an output directory" {
  out=$BATS_TEST_TMPDIR/audit
  FAKE_SLEEP=2 "$scripts/review_all.sh" "$out" web-ui 2>/dev/null &
  pid=$!
  for _ in $(seq 50); do
    [ "$(calls)" -eq 1 ] && break
    sleep 0.1
  done
  run "$scripts/review_all.sh" "$out" storage
  [ "$status" -eq 2 ]
  [[ "$output" == *"another audit"* ]]
  wait "$pid"
  [ "$(calls)" -eq 1 ]
}

@test "the merged report leaves out reviews of another commit" {
  out=$BATS_TEST_TMPDIR/audit
  mkdir -p "$out"
  run "$scripts/review_all.sh" "$out" web-ui
  [ "$status" -eq 0 ]
  cp "$out/web-ui.json" "$out/storage.json"
  echo 0000000000000000000000000000000000000000 >"$out/storage.head"
  touch "$out/storage.md"
  git commit -q --allow-empty -m moved
  run "$scripts/review_all.sh" "$out" web-ui
  [ "$status" -eq 0 ]
  grep -q '共 1 份审查' "$out/report.md"
  refute grep -q 'storage' "$out/report.md"
}

@test "stopping an audit stops its reviews and starts no more" {
  out=$BATS_TEST_TMPDIR/audit
  FAKE_SLEEP=30 "$scripts/review_all.sh" "$out" web-ui storage db 2>/dev/null &
  pid=$!
  for _ in $(seq 100); do
    [ "$(calls)" -eq 2 ] && break
    sleep 0.1
  done
  [ "$(calls)" -eq 2 ]
  kill -TERM "$pid"
  wait "$pid" || true
  for _ in $(seq 50); do
    [ -z "$(ls "$KCR_PROC_DIR")" ] && break
    sleep 0.1
  done
  [ -z "$(ls "$KCR_PROC_DIR")" ]
  sleep 1
  [ "$(calls)" -eq 2 ]
}

@test "the merged report ranks findings by severity across reviews" {
  cat >"$BATS_TEST_TMPDIR/low.json" <<'EOF'
{"summary":"s","coverage_notes":"c","retracted":[],"findings":[{"severity":"P3","verdict":"confirmed","lens":"correctness","title":"小问题","file":"a.rs","line":1,"related_locations":[],"description":"d","trigger_scenario":"t","evidence":"e","suggested_fix":"f","suggested_test":"x"}]}
EOF
  cat >"$BATS_TEST_TMPDIR/high.json" <<'EOF'
{"summary":"s","coverage_notes":"c","retracted":[],"findings":[{"severity":"P0","verdict":"uncertain","lens":"data","title":"大问题","file":"b.rs","line":2,"related_locations":["c.rs:3"],"description":"d","trigger_scenario":"t","evidence":"e","suggested_fix":"f","suggested_test":"x"}]}
EOF
  run python3 "$scripts/render_review.py" merge "$BATS_TEST_TMPDIR/report.md" \
    "$BATS_TEST_TMPDIR/low.json" "$BATS_TEST_TMPDIR/high.json"
  [ "$status" -eq 0 ]
  big=$(grep -n '大问题' "$BATS_TEST_TMPDIR/report.md" | head -1 | cut -d: -f1)
  small=$(grep -n '小问题' "$BATS_TEST_TMPDIR/report.md" | head -1 | cut -d: -f1)
  [ "$big" -lt "$small" ]
  grep -q '### 1. \[high\] 大问题' "$BATS_TEST_TMPDIR/report.md"
}
