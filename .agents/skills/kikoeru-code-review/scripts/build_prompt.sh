#!/usr/bin/env bash
# Prints the review prompt for one target. The stable parts (contract, then
# lenses) come first and the review input last, so repeated reviews share a
# cacheable prefix.
#
#   build_prompt.sh diff <base>
#   build_prompt.sh files <path>...
#   build_prompt.sh component <id>
#   build_prompt.sh --lenses <mode> <args>...   # only the activated lenses
#
# Run it from inside the repository under review.
set -euo pipefail
skill=$(cd "$(dirname "$0")/.." && pwd -P)

usage() {
  echo "usage: build_prompt.sh [--lenses] <diff <base> | files <path>... | component <id>>" >&2
  exit 2
}

# Paths whose change can break the wire format or its consumers.
contract_re='^(crates/neo-core/src/dto/|contract/|book/src/(contract|api)\.md$|crates/neo-typegen/|crates/neo-server/src/http/|web/src/lib/(api|stores|player)/|desktop/src-tauri/src/)'
# Paths that decide who may do what.
auth_re='^(crates/neo-server/src/http/|crates/neo-server/src/(token|password|user_cmd|private|embed|datadir|config)\.rs$|crates/neo-(db/queries|core/src/db)/users\.rs$|web/src/lib/api/(client|transport|cover-cookie)\.ts$|web/src/lib/stores/session\.svelte\.ts$|desktop/src-tauri/(capabilities/|tauri[^/]*\.json$|src/)|book/src/auth\.md$)'
# Paths that store, migrate, copy, or delete user data.
data_re='^(crates/neo-core/migrations/|crates/neo-core/src/db/|crates/neo-db(-sqlite|-postgres)?/|crates/neo-server/src/(import/|(migrate_cmd|backup|scan|embed|serve)\.rs$)|crates/neo-scanner/|book/src/(data-model|migration)\.md$)'

# Reads paths on stdin and prints the lenses they activate, in fixed order.
lenses_for_paths() {
  local paths
  paths=$(cat)
  echo correctness
  grep -qE "$contract_re" <<<"$paths" && echo contract
  grep -qE "$auth_re" <<<"$paths" && echo auth
  grep -qE "$data_re" <<<"$paths" && echo data
  return 0
}

component_file() {
  local file="$skill/components/$1.md"
  if [[ ! "$1" =~ ^[a-z0-9-]+$ || ! -f "$file" ]]; then
    echo "build_prompt.sh: unknown component: $1 (see $skill/components/)" >&2
    exit 2
  fi
  printf '%s\n' "$file"
}

# A component names its lenses on a "Lenses: a, b" line.
lenses_for_component() {
  local line
  line=$(grep -m1 '^Lenses:' "$(component_file "$1")" || true)
  [[ -n "$line" ]] || { echo "build_prompt.sh: $1 has no Lenses: line" >&2; exit 2; }
  tr ',' '\n' <<<"${line#Lenses:}" | tr -d ' ' | grep -v '^$'
}

merge_base() {
  local mb
  if ! mb=$(git merge-base "$1" HEAD 2>/dev/null); then
    echo "build_prompt.sh: cannot find a merge base between $1 and HEAD" >&2
    exit 2
  fi
  if [[ -z $(git rev-list "$mb..HEAD") ]]; then
    echo "build_prompt.sh: no commits in $1..HEAD; commit the change first" >&2
    exit 2
  fi
  printf '%s\n' "$mb"
}

# Lock files and SQLx's offline query cache are generated; their diffs would
# crowd out the code.
diff_excludes=(':(exclude,glob)**/.sqlx/**' ':(exclude,glob)**/Cargo.lock' ':(exclude,glob)**/pnpm-lock.yaml' ':(exclude,glob)**/package-lock.json' ':(exclude)flake.lock')

lenses() {
  local mode=$1 mb path
  shift
  case $mode in
    diff)
      [[ $# -eq 1 ]] || usage
      mb=$(merge_base "$1")
      # Without renames a moved file lists its old path too, so moving code
      # out of a lens's directory still brings that lens.
      git diff --name-only --no-renames "$mb" HEAD | lenses_for_paths
      ;;
    files)
      [[ $# -ge 1 ]] || usage
      # A directory brings the lenses of the files inside it.
      for path in "$@"; do
        if [[ -d $path ]]; then git ls-files -- "$path"; else printf '%s\n' "$path"; fi
      done | lenses_for_paths
      ;;
    component)
      [[ $# -eq 1 ]] || usage
      lenses_for_component "$1"
      ;;
    *) usage ;;
  esac
}


review_input() {
  local mode=$1
  shift
  echo "===== REVIEW INPUT ====="
  echo
  case $mode in
    diff)
      local mb
      mb=$(merge_base "$1")
      echo "Mode: diff. Review the commit series that HEAD $(git rev-parse --short HEAD) adds over"
      echo "${KCR_BASE_LABEL:-$1} (merge base $(git rev-parse --short "$mb")). Each commit message states"
      echo "the intent of its commit: check the code against it. The working tree is"
      echo "checked out at HEAD, so read surrounding code there."
      echo "Read the book chapters that cover the changed areas; book/src/SUMMARY.md"
      echo "lists them. Lock files and .sqlx/ are left out of the diff below; inspect"
      echo "them with git if a finding depends on them."
      echo
      echo "Changed paths:"
      git diff --stat=200 "$mb" HEAD -- "${diff_excludes[@]}" | sed 's/^/  /'
      echo
      # --cc keeps what a merge commit itself changed, such as a conflict
      # resolution, which plain -p leaves out.
      git log -p --cc --reverse --no-color --format='%n===== commit %h =====%n%n[commit message]%n%B%n[changes]%n' \
        "$mb..HEAD" -- . "${diff_excludes[@]}"
      ;;
    files)
      local path
      echo "Mode: files. Review these paths in full, as they are at HEAD"
      echo "$(git rev-parse --short HEAD). Read every file under a directory. Read the"
      echo "book chapters that cover them; book/src/SUMMARY.md lists them."
      echo
      for path in "$@"; do
        [[ -e "$path" ]] || { echo "build_prompt.sh: no such path: $path" >&2; exit 2; }
        echo "- $path"
      done
      ;;
    component)
      echo "Mode: component. Review one component of the repository as it is at"
      echo "HEAD $(git rev-parse --short HEAD). The component description below lists"
      echo "its files, the book chapters to read, and where defects are most likely."
      echo
      cat "$(component_file "$1")"
      ;;
  esac
}

only_lenses=0
if [[ ${1:-} == --lenses ]]; then
  only_lenses=1
  shift
fi
[[ $# -ge 1 ]] || usage
cd "$(git rev-parse --show-toplevel)"

active=$(lenses "$@")
if [[ $only_lenses -eq 1 ]]; then
  printf '%s\n' "$active"
  exit 0
fi

cat "$skill/review_contract.md"
while read -r lens; do
  [[ -f "$skill/lenses/$lens.md" ]] || { echo "build_prompt.sh: unknown lens: $lens" >&2; exit 2; }
  printf '\n===== LENS: %s =====\n\n' "$lens"
  cat "$skill/lenses/$lens.md"
done <<<"$active"
echo
review_input "$@"
