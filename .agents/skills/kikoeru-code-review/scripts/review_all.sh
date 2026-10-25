#!/usr/bin/env bash
# Reviews every component (or the named ones) at the current HEAD and merges
# the results into <outdir>/report.md.
#
#   review_all.sh <outdir> [component...]
#
# A component that already has a review of the current HEAD in <outdir> is
# skipped, so rerunning the same command resumes an interrupted audit, and a
# review of an older commit is redone. The audit runs at low priority: a
# pull-request review waiting for a slot goes first.
set -euo pipefail
# Bash reads a script while it runs it. The braces make it parse the whole
# file first, so editing this file during a long run cannot change the run.
# Inside the braces shellcheck no longer sees that traps call the handlers.
# shellcheck disable=SC2329
{
skill=$(cd "$(dirname "$0")/.." && pwd -P)

[[ $# -ge 1 ]] || { echo "usage: review_all.sh <outdir> [component...]" >&2; exit 2; }
outdir=$(mkdir -p "$1" && cd "$1" && pwd -P)
shift
if [[ $# -gt 0 ]]; then
  components=("$@")
else
  mapfile -t components < <(find "$skill/components" -name '*.md' -printf '%f\n' | sed 's/\.md$//' | sort)
fi

# Run the skill as it is now, so switching branches during a long audit
# cannot break the reviews still queued or the final merge.
work=$(mktemp -d "${TMPDIR:-/tmp}/kcr-audit.XXXXXX")
trap 'rm -rf "$work"' EXIT
cp -r "$skill" "$work/skill"
here=$work/skill/scripts

# Two audits sharing one directory would overwrite each other's reviews and
# merge reviews of different commits into one report.
exec 6>"$outdir/.audit.lock"
if ! flock -n 6; then
  echo "review_all.sh: another audit is writing to $outdir" >&2
  exit 2
fi

# Every component reviews the commit HEAD is at now, even if HEAD moves
# while the audit runs.
head=$(git rev-parse HEAD)
export KCR_HEAD=$head
pending=()
for id in "${components[@]}"; do
  if [[ ! -f $outdir/$id.md || $(cat "$outdir/$id.head" 2>/dev/null) != "$head" ]]; then
    pending+=("$id")
  fi
done

export KCR_LOW_PRIORITY=1
# Each review takes a slot inside codex_review.sh, so starting as many
# reviews as there are slots keeps every slot busy without overrunning it.
printf '%s\n' "${pending[@]}" | xargs -r -P "${KCR_SLOTS:-2}" -I{} \
  "$here/codex_review.sh" component {} "$outdir/{}.md" --overwrite 6>&- &
scheduler=$!
# Stopping the audit stops the reviews in progress (each launcher stops its
# Codex) and starts no new ones. xargs is paused first so it cannot start
# another review between listing its children and being stopped.
on_signal() {
  kill -STOP "$scheduler" 2>/dev/null || true
  pkill -TERM -P "$scheduler" 2>/dev/null || true
  kill -TERM "$scheduler" 2>/dev/null || true
  kill -CONT "$scheduler" 2>/dev/null || true
  wait 2>/dev/null || true
  exit 130
}
trap on_signal INT TERM HUP
failed=0
wait "$scheduler" || failed=1

# Merge only reviews of this commit; an older review left in the directory
# is not part of this audit.
done_json=()
for id in "${components[@]}"; do
  if [[ -f $outdir/$id.md && $(cat "$outdir/$id.head" 2>/dev/null) == "$head" ]]; then
    done_json+=("$outdir/$id.json")
  fi
done
if [[ ${#done_json[@]} -gt 0 ]]; then
  python3 "$here/render_review.py" merge "$outdir/report.md" "${done_json[@]}"
fi
if [[ $failed -ne 0 ]]; then
  echo "review_all.sh: some reviews failed; rerun the same command to retry them" >&2
  exit 1
fi
exit
}
