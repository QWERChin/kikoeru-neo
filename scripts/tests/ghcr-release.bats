#!/usr/bin/env bats
# Tests for scripts/ghcr-release.sh against a fake skopeo. RELEASE points at
# the script under test, the packaged one in the flake check.

setup() {
  here=$(cd "$(dirname "$BATS_TEST_FILENAME")" && pwd)
  : "${RELEASE:=$here/../ghcr-release.sh}"
  work=$BATS_TEST_TMPDIR
  export FAKE_LOG=$work/skopeo.log
  export SKOPEO=$here/fake-skopeo
  export FAKE_REGISTRY=$BATS_TEST_TMPDIR/registry
  mkdir -p "$FAKE_REGISTRY"
  unset GITHUB_TOKEN GITHUB_ACTOR
  : >"$FAKE_LOG"
  cd "$work"
  image 0.4.2 >image.tar
  echo '{}' >policy.json
  cargo 0.4.2
}

# An image config labelled VERSION, with EXTRA to tell builds apart.
image() {
  printf '{"config":{"Labels":{"org.opencontainers.image.version":"%s"}},"build":"%s"}\n' "$1" "${2:-}"
}

# Puts an image config under TAG in the fake registry.
published() {
  image "$2" "${3:-}" >"$FAKE_REGISTRY/$1"
}

# A workspace Cargo.toml with VERSION, and a member-like table before it
# whose version must be ignored.
cargo() {
  cat >Cargo.toml <<TOML
[package]
version = "9.9.9"

[workspace.package]
version = "$1"
edition = "2024"
TOML
}

release() {
  run "$RELEASE" --image image.tar --policy policy.json "$@"
}

pushed() {
  while read -r line; do
    case " $line " in
      *" copy "*) echo "${line##* docker://}" ;;
    esac
  done <"$FAKE_LOG"
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

# Whatever a test does, latest is never pushed.
teardown() {
  refute grep -q ':latest' "$FAKE_LOG"
}

@test "pushes X.Y.Z, then X.Y, from the archive with the given policy" {
  release --tag v0.4.2
  [ "$status" -eq 0 ]
  [ "$(pushed)" = "$(printf 'ghcr.io/qwerchin/kikoeru-neo:0.4.2\nghcr.io/qwerchin/kikoeru-neo:0.4')" ]
  grep -q -- '--policy policy.json inspect --config --raw docker://ghcr.io/qwerchin/kikoeru-neo:0.4.2$' "$FAKE_LOG"
  grep -q -- '--policy policy.json copy --retry-times 3 docker-archive:image.tar docker://ghcr.io/qwerchin/kikoeru-neo:0.4.2$' "$FAKE_LOG"
}

@test "refuses a tag that does not match Cargo.toml" {
  release --tag v0.4.3
  [ "$status" -ne 0 ]
  [[ $output == *"does not match version 0.4.2"* ]]
  [ ! -s "$FAKE_LOG" ]
}

@test "refuses a tag without the v" {
  release --tag 0.4.2
  [ "$status" -ne 0 ]
  [ -z "$(pushed)" ]
}

@test "refuses a pre-release version" {
  cargo 0.5.0-rc.1
  release --tag v0.5.0-rc.1
  [ "$status" -ne 0 ]
  [[ $output == *"not X.Y.Z"* ]]
  [ ! -s "$FAKE_LOG" ]
}

@test "never pushes a version that already exists with another image" {
  published 0.4.2 0.4.2 other-build
  release --tag v0.4.2
  [ "$status" -ne 0 ]
  [[ $output == *"already exists with another image"* ]]
  [ -z "$(pushed)" ]
}

@test "a retry finishes a release whose floating tag failed" {
  published 0.4.2 0.4.2
  published 0.4 0.4.1
  release --tag v0.4.2
  [ "$status" -eq 0 ]
  [ "$(pushed)" = ghcr.io/qwerchin/kikoeru-neo:0.4 ]
}

@test "the floating tag never moves back to an older patch" {
  published 0.4 0.4.3
  release --tag v0.4.2
  [ "$status" -eq 0 ]
  [ "$(pushed)" = ghcr.io/qwerchin/kikoeru-neo:0.4.2 ]
  [[ $output == *"already points at 0.4.3"* ]]
}

@test "the floating tag moves forward" {
  published 0.4 0.4.1
  release --tag v0.4.2
  [ "$status" -eq 0 ]
  [ "$(pushed | wc -l)" -eq 2 ]
  grep -q 0.4.2 "$FAKE_REGISTRY/0.4"
}

@test "a release that is already complete pushes nothing" {
  published 0.4.2 0.4.2
  published 0.4 0.4.2
  release --tag v0.4.2
  [ "$status" -eq 0 ]
  [ -z "$(pushed)" ]
}

@test "refuses a floating tag without a version label" {
  echo '{}' >"$FAKE_REGISTRY/0.4"
  release --tag v0.4.2
  [ "$status" -ne 0 ]
  [ -z "$(pushed)" ]
}

@test "refuses an archive labelled with another version" {
  image 0.4.1 >image.tar
  release --tag v0.4.2
  [ "$status" -ne 0 ]
  [[ $output == *"not labelled as version 0.4.2"* ]]
  [ -z "$(pushed)" ]
}

@test "a registry error is not taken as a free tag" {
  FAKE_INSPECT=denied release --tag v0.4.2
  [ "$status" -ne 0 ]
  [[ $output == *"403 Forbidden"* ]]
  [ -z "$(pushed)" ]
}

@test "a package that does not exist yet can be published" {
  FAKE_INSPECT=name-unknown release --tag v0.4.2
  [ "$status" -eq 0 ]
  [ "$(pushed | wc -l)" -eq 2 ]
}

@test "a dry run checks everything and pushes nothing" {
  release --tag v0.4.2 --dry-run
  [ "$status" -eq 0 ]
  [[ $output == *"would push image.tar to ghcr.io/qwerchin/kikoeru-neo:0.4"* ]]
  grep -q ' inspect --config --raw docker://' "$FAKE_LOG"
  [ -z "$(pushed)" ]
}

@test "the token reaches skopeo only through stdin" {
  export GITHUB_TOKEN=secret-token GITHUB_ACTOR=someone
  release --tag v0.4.2
  [ "$status" -eq 0 ]
  refute grep -q secret-token "$FAKE_LOG"
  [ "$(cat "$FAKE_LOG.password")" = secret-token ]
  grep -q -- 'login --authfile .*/auth.json -u someone --password-stdin ghcr.io$' "$FAKE_LOG"
  grep -q -- 'inspect --config --raw --authfile .*/auth.json docker://' "$FAKE_LOG"
  grep -q -- 'copy --retry-times 3 --dest-authfile .*/auth.json docker-archive:' "$FAKE_LOG"
}

@test "a failed push stops before the floating tag" {
  FAKE_FAIL_TAG=0.4.2 release --tag v0.4.2
  [ "$status" -ne 0 ]
  [ "$(pushed)" = ghcr.io/qwerchin/kikoeru-neo:0.4.2 ]
}

# The flake check sets RELEASE_PACKAGED for the packaged script, which
# brings its own tools and the pinned skopeo's policy.
@test "the packaged script needs nothing on PATH and has a default policy" {
  [ -n "${RELEASE_PACKAGED:-}" ] || skip "only for the packaged script"
  export FAKE_PATH=$PATH
  run env PATH=/var/empty "$RELEASE" --image image.tar --tag v0.4.2
  [ "$status" -eq 0 ]
  grep -q -- '--policy /nix/store/[^ ]*-skopeo-[^ ]*/etc/containers/default-policy.json inspect' "$FAKE_LOG"
}
