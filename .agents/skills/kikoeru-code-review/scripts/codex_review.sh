#!/usr/bin/env bash
# Runs one kikoeru-code-review pass with the local Codex CLI and writes a
# Markdown review, plus the raw JSON, the reviewed commit and Codex's log
# next to it.
#
#   codex_review.sh diff <base> <output.md> [--overwrite]
#   codex_review.sh files <path>... <output.md> [--overwrite]
#   codex_review.sh component <id> <output.md> [--overwrite]
#
# Run it from inside the repository under review. See ../SKILL.md.
set -euo pipefail
# Bash reads a script while it runs it. The braces make it parse the whole
# file first, so editing this file during a long run cannot change the run.
# Inside the braces shellcheck no longer sees that traps call the handlers.
# shellcheck disable=SC2329
{
# The physical path, so the snapshot below copies the skill and not a
# symlink to it, such as .claude/skills/kikoeru-code-review.
skill=$(cd "$(dirname "$0")/.." && pwd -P)

model=${KCR_MODEL:-gpt-6.1-sol}
effort=${KCR_EFFORT:-high}
# Set the tier explicitly so local CLI configuration cannot silently
# override the launcher configuration.
tier=${KCR_TIER:-default}
codex=${KCR_CODEX:-codex}
review_timeout=${KCR_TIMEOUT:-4h}
slots=${KCR_SLOTS:-2}
poll=${KCR_POLL:-20}

usage() {
  echo "usage: codex_review.sh <diff <base> | files <path>... | component <id>> <output.md> [--overwrite]" >&2
  exit 2
}

# Codex runs this script's prompt with the review contract, which forbids
# starting Codex again. Refuse anyway, so a confused review cannot fan out
# into more Codex processes than the slots allow.
if [[ -n ${KCR_REVIEW_RUNNING:-} ]]; then
  echo "codex_review.sh: refusing to start a review from inside a running review" >&2
  exit 3
fi

overwrite=0
args=()
for arg in "$@"; do
  case $arg in
    --overwrite) overwrite=1 ;;
    --*) usage ;;
    *) args+=("$arg") ;;
  esac
done
[[ ${#args[@]} -ge 3 ]] || usage
output=${args[-1]}
target=("${args[@]:0:${#args[@]}-1}")
case ${target[0]} in
  diff | component) [[ ${#target[@]} -eq 2 ]] || usage ;;
  files) ;;
  *) usage ;;
esac
top=$(git rev-parse --show-toplevel)
# The slots belong to this repository, not the machine: the lock files live
# in its git directory, which all of its worktrees share, and only reviews
# of this repository count against them.
common=$(git rev-parse --path-format=absolute --git-common-dir)
lock_dir=${KCR_LOCK_DIR:-$common/kikoeru-code-review}
mkdir -p "$(dirname "$output")" "$lock_dir"
# The physical path, so a symlinked directory cannot give the same output a
# second lock.
output=$(cd "$(dirname "$output")" && pwd -P)/$(basename "$output")
# Two runs writing the same output would mix one review's JSON with the
# other's commit. Codex inherits this lock too, like the slot below.
exec 7>"$lock_dir/out-$(sha1sum <<<"$output" | cut -c1-16).lock"
if ! flock -n 7; then
  echo "codex_review.sh: another review is writing $output" >&2
  exit 2
fi
if [[ -e $output && $overwrite -eq 0 ]]; then
  echo "codex_review.sh: $output exists; pass --overwrite to replace it" >&2
  exit 2
fi
stem=${output%.md}
json=$stem.json
log=$stem.log
# A failed rerun must not leave the previous round's review in place, where
# it would read as this round's result.
rm -f "$output" "$json" "$stem.head"

# KCR_HEAD reviews another commit than HEAD; review_all.sh uses it to keep
# every component of one audit on the same commit.
head=$(git rev-parse --verify --quiet "${KCR_HEAD:-HEAD}^{commit}") || {
  echo "codex_review.sh: unknown commit: $KCR_HEAD" >&2
  exit 2
}
base_sha=""
case ${target[0]} in
  diff)
    # Resolved here, because a name like origin/main~1 can point elsewhere
    # in the clone, whose remote refs are this repository's branches.
    base_sha=$(git rev-parse --verify --quiet "${target[1]}^{commit}") || {
      echo "codex_review.sh: unknown base: ${target[1]}" >&2
      exit 2
    }
    ;;
  files)
    # Paths are resolved against the top of the repository, so ones given
    # relative to a subdirectory have to be rewritten first.
    for ((i = 1; i < ${#target[@]}; i++)); do
      target[i]=$(realpath -m --relative-to="$top" "${target[i]}")
    done
    ;;
esac

marker="" child=""
work=$(mktemp -d "${TMPDIR:-/tmp}/kcr-run.XXXXXX")
cleanup() {
  rm -rf "$work"
  [[ -z $marker ]] || rm -f "$marker"
}
trap cleanup EXIT
# Run the skill as it is now, so switching branches or editing the skill
# while this review waits or runs cannot change or break it.
cp -r "$skill" "$work/skill"
skill=$work/skill
# Codex runs under timeout, which puts it in its own process group, so a
# Ctrl-C or a TERM sent to this script's group would not reach it. Pass the
# signal on, so a stopped review does not keep its slot for hours.
on_signal() {
  if [[ -n $child ]]; then
    kill -TERM "$child" 2>/dev/null || true
    wait "$child" 2>/dev/null || true
  fi
  exit 130
}
trap on_signal INT TERM HUP

# Reviews of this repository that do not go through this script, such as a
# plain `codex review` in one of its worktrees, count against the same
# limit.
running_reviews() {
  local roots
  mapfile -t roots < <(git -C "$top" worktree list --porcelain | sed -n 's/^worktree //p')
  python3 "$skill/scripts/count_reviews.py" "${KCR_PROC_DIR:-/proc}" "$common" "${roots[@]}"
}

# A normal review registers here while it waits. A low-priority one (the
# whole-repository audit) does not take a slot while any is registered, so
# pull-request reviews are not stuck behind a long audit.
waiting_reviews() {
  local entry count=0
  for entry in "$lock_dir"/waiting/*; do
    [[ -e $entry ]] || continue
    if kill -0 "${entry##*/}" 2>/dev/null; then
      count=$((count + 1))
    else
      rm -f "$entry"
    fi
  done
  echo "$count"
}

# Takes one of $slots flock slots on fd 9 and the admission lock on fd 8.
# The slot is held until this script and its Codex child exit; Codex
# inherits fd 9, so the slot stays taken even if this script is killed while
# Codex keeps running. The admission lock makes counting the running reviews
# and starting the new one a single step: without it, two launchers holding
# different slots could both count one outside review and both start.
acquire_slot() {
  local i said=0
  mkdir -p "$lock_dir/waiting"
  if [[ -z ${KCR_LOW_PRIORITY:-} ]]; then
    marker=$lock_dir/waiting/$$
    : >"$marker"
  fi
  while true; do
    if [[ -n $marker || $(waiting_reviews) -eq 0 ]]; then
      for ((i = 1; i <= slots; i++)); do
        exec 9>"$lock_dir/slot$i.lock"
        if flock -n 9; then
          exec 8>"$lock_dir/admit.lock"
          flock 8
          if [[ $(running_reviews) -lt $slots ]]; then
            [[ -z $marker ]] || rm -f "$marker"
            marker=""
            return 0
          fi
          exec 8>&-
          flock -u 9
        fi
        exec 9>&-
      done
    fi
    if [[ $said -eq 0 ]]; then
      echo "codex_review.sh: waiting for one of $slots Codex review slots" >&2
      said=1
    fi
    sleep "$poll"
  done
}

# Fail on a bad target now rather than after waiting for a slot.
if [[ -z ${KCR_HEAD:-} ]]; then
  (cd "$top" && "$skill/scripts/build_prompt.sh" --lenses "${target[@]}") >/dev/null
fi
acquire_slot

# Codex reviews a private clone checked out at the recorded commit, so
# edits and commits made in the worktree while the review waits or runs do
# not change what it reads. Cloning the common git directory works from a
# linked worktree too.
tree=$work/tree
git clone -q --shared --no-checkout "$common" "$tree"
git -C "$tree" checkout -q --detach "$head"
prompt_target=("${target[@]}")
[[ -z $base_sha ]] || prompt_target[1]=$base_sha
(cd "$tree" && KCR_BASE_LABEL=${target[1]} "$skill/scripts/build_prompt.sh" "${prompt_target[@]}") >"$work/prompt"

echo "codex_review.sh: reviewing ${target[*]} at ${head:0:9} with $model, effort $effort, tier $tier" >&2
before=$(running_reviews)
# The user's plugins, memories, hooks and apps would make the review depend
# on unrelated personal state, so they are off. Sub-agents stay on, since
# the contract hands each lens to one. Codex must not inherit the admission
# lock, or it would hold it for the whole review.
# KCR_REVIEW_RUNNING names this repository, so its own reviews count against
# its slots, and it also stops Codex from starting a review inside one.
KCR_REVIEW_RUNNING=$common timeout "$review_timeout" "$codex" exec \
  -C "$tree" \
  -s read-only \
  -m "$model" \
  -c model_reasoning_effort="\"$effort\"" \
  -c service_tier="\"$tier\"" \
  -c approval_policy='"never"' \
  --disable plugins --disable memories --disable hooks --disable apps \
  --output-schema "$skill/schema.json" \
  -o "$json" \
  - <"$work/prompt" >"$log" 2>&1 8>&- &
child=$!
# Keep the admission lock until the new review shows up in ps, so the next
# launcher to count sees it. Give up after KCR_ADMIT_WAIT tenths of a second
# in case the process never matches.
for ((t = 0; t < ${KCR_ADMIT_WAIT:-300}; t++)); do
  [[ $(running_reviews) -gt $before ]] && break
  kill -0 "$child" 2>/dev/null || break
  sleep 0.1
done
exec 8>&-
status=0
wait "$child" || status=$?
child=""
if [[ $status -ne 0 ]]; then
  echo "codex_review.sh: codex exited with status $status; see $log" >&2
  exit 1
fi

meta=(
  "mode=${target[0]}"
  "target=${target[*]:1}"
  "head=${head:0:9}"
  "model=$model"
  "effort=$effort"
  "tier=$tier"
)
# The review appears in one step, and the commit file only after it, so a
# rerun of review_all.sh never takes a half-written review for a finished
# one.
python3 "$skill/scripts/render_review.py" single "$json" "$output" "${meta[@]}"
echo "$head" >"$stem.head"
exit
}
