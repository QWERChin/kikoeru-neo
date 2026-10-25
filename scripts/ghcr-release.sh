#!/usr/bin/env bash
# kikoeru-neo-ghcr-release: pushes the Docker image of a release tag to
# GitHub Container Registry. .github/workflows/release.yml runs it, and
# book/src/deployment.md ("发布") describes the tags it publishes.
#
# Only X.Y.Z and the floating X.Y are pushed, never latest: the old
# kikoeru image's latest tag moved to a release that migrates the database
# on start, and a deployment should only change version on purpose.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: kikoeru-neo-ghcr-release --tag vX.Y.Z --image IMAGE.tar [options]

  --tag TAG        the git tag being released; it must be v plus the
                   version in Cargo.toml
  --image FILE     the image as a docker-archive tar
  --repo REPO      where to push (default ghcr.io/qwerchin/kikoeru-neo)
  --cargo FILE     the workspace Cargo.toml (default ./Cargo.toml)
  --policy FILE    the containers policy for skopeo (default
                   $KIKOERU_RELEASE_POLICY)
  --dry-run        check everything, push nothing
  -h, --help       show this help

With GITHUB_TOKEN set, it logs in to the registry as GITHUB_ACTOR first.
EOF
}

die() {
  printf 'kikoeru-neo-ghcr-release: %s\n' "$*" >&2
  exit 1
}

tag=""
image=""
repo=ghcr.io/qwerchin/kikoeru-neo
cargo=Cargo.toml
policy=${KIKOERU_RELEASE_POLICY:-}
dry_run=false
while [ $# -gt 0 ]; do
  case $1 in
    --tag | --image | --repo | --cargo | --policy)
      [ $# -ge 2 ] || die "$1 needs a value"
      case $1 in
        --tag) tag=$2 ;;
        --image) image=$2 ;;
        --repo) repo=$2 ;;
        --cargo) cargo=$2 ;;
        --policy) policy=$2 ;;
      esac
      shift 2
      ;;
    --dry-run)
      dry_run=true
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *) die "unknown argument $1 (see --help)" ;;
  esac
done
[ -n "$tag" ] || die "--tag is required"
[ -n "$image" ] || die "--image is required"
[ -n "$policy" ] || die "--policy is required"
[ -f "$image" ] || die "$image does not exist"
[ -f "$policy" ] || die "$policy does not exist"
skopeo=${SKOPEO:-skopeo}

# The version in the [workspace.package] table of Cargo.toml.
version=""
section=""
while IFS= read -r line || [ -n "$line" ]; do
  case $line in
    '['*) section=$line ;;
    'version = "'*'"')
      if [ "$section" = '[workspace.package]' ]; then
        version=${line#'version = "'}
        version=${version%'"'}
      fi
      ;;
  esac
done <"$cargo"
[ -n "$version" ] || die "no version in the [workspace.package] table of $cargo"

[[ $version =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] ||
  die "version $version in $cargo is not X.Y.Z; pre-releases are not published"
tags=("$version" "${BASH_REMATCH[1]}.${BASH_REMATCH[2]}")
[ "$tag" = "v$version" ] || die "tag $tag does not match version $version in $cargo"

tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT
src_auth=()
dest_auth=()
if [ -n "${GITHUB_TOKEN:-}" ]; then
  [ -n "${GITHUB_ACTOR:-}" ] || die "GITHUB_TOKEN is set but GITHUB_ACTOR is not"
  # The token goes through stdin into a private auth file, so it never
  # shows up in a process listing.
  printf '%s' "$GITHUB_TOKEN" |
    "$skopeo" login --authfile "$tmp/auth.json" -u "$GITHUB_ACTOR" --password-stdin "${repo%%/*}" >/dev/null
  src_auth=(--authfile "$tmp/auth.json")
  dest_auth=(--dest-authfile "$tmp/auth.json")
fi

# The image config of TAG in the registry, into FILE. Returns 1 when the
# registry says the tag or the package does not exist. Any other error,
# such as a denied token, stops the release rather than being taken for a
# free tag.
remote_config() {
  if "$skopeo" --policy "$policy" inspect --config --raw "${src_auth[@]}" \
    "docker://$repo:$1" >"$2" 2>"$tmp/inspect.err"; then
    return 0
  fi
  case $(cat "$tmp/inspect.err") in
    *"manifest unknown"* | *"name unknown"*) return 1 ;;
  esac
  cat "$tmp/inspect.err" >&2
  die "cannot tell whether $repo:$1 exists"
}

# The version an image config is labelled with, as nix/docker.nix sets it.
label_version() {
  local re='"org\.opencontainers\.image\.version"[[:space:]]*:[[:space:]]*"([^"]*)"'
  [[ $(cat "$1") =~ $re ]] && printf '%s' "${BASH_REMATCH[1]}"
}

"$skopeo" --policy "$policy" inspect --config --raw "docker-archive:$image" >"$tmp/local.json"
[ "$(label_version "$tmp/local.json")" = "$version" ] ||
  die "$image is not labelled as version $version"
local_digest=$(sha256sum <"$tmp/local.json")

for t in "${tags[@]}"; do
  [ "$t" != latest ] || die "refusing to push latest"
done

# X.Y.Z is pushed once and never changes under its users. A retry after a
# release that stopped half way finds it already there with this very
# image config, and only finishes the floating tag. The config is compared
# rather than the manifest, whose digest depends on the compression.
push_exact=true
if remote_config "$version" "$tmp/exact.json"; then
  [ "$(sha256sum <"$tmp/exact.json")" = "$local_digest" ] ||
    die "$repo:$version already exists with another image; a published version is never pushed again"
  echo "$repo:$version is already this image"
  push_exact=false
fi

# X.Y only moves forward, so retrying an older release, or one that
# overlaps a newer one, leaves it on the newest patch.
minor=${tags[1]}
push_minor=true
if remote_config "$minor" "$tmp/minor.json"; then
  current=$(label_version "$tmp/minor.json") ||
    die "$repo:$minor has no version label; cannot tell whether $version is newer"
  [[ $current =~ ^[0-9]+\.[0-9]+\.([0-9]+)$ ]] && [ "${current%.*}" = "$minor" ] ||
    die "$repo:$minor is labelled $current, which is not a $minor release"
  if [ "${BASH_REMATCH[1]}" -ge "${version##*.}" ]; then
    echo "$repo:$minor already points at $current; leaving it"
    push_minor=false
  fi
fi

plan=()
[ "$push_exact" = false ] || plan+=("$version")
[ "$push_minor" = false ] || plan+=("$minor")
if [ "$dry_run" = true ]; then
  for t in "${plan[@]}"; do
    echo "would push $image to $repo:$t"
  done
  exit 0
fi
for t in "${plan[@]}"; do
  echo "pushing $repo:$t"
  "$skopeo" --policy "$policy" copy --retry-times 3 "${dest_auth[@]}" \
    "docker-archive:$image" "docker://$repo:$t"
done
