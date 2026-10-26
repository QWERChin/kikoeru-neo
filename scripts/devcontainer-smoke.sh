#!/usr/bin/env bash
set -euo pipefail
image=${1:?usage: devcontainer-smoke.sh IMAGE}
name=""
cleanup() { if [ -n "$name" ]; then docker rm -fv "$name" >/dev/null 2>&1 || true; fi; }
trap cleanup EXIT
# Entry and exec must both work offline, without invoking Nix.
name=$(docker run -d --network none "$image" /bin/bash -c 'exec sleep infinity')
for shell in '/bin/bash -c' '/bin/bash -lc' '/bin/bash -ic' '/bin/sh -c'; do
  # The shell values above are deliberate executable/argument pairs.
  # shellcheck disable=SC2086
  docker exec "$name" $shell \
    'test "$(id -u):$(id -g)" = 1000:1000 && cargo --version && pnpm --version && ffmpeg -version && sqlx --version && mold --version && test -r "$KIKOERU_FALLBACK_FONTS_DIR/fallback-cjk.woff2" && test -d "$PLAYWRIGHT_BROWSERS_PATH"'
done
docker exec "$name" cargo --version
# A tiny native build exercises the same mold flag as .cargo/config.toml.
docker exec "$name" /bin/bash -c \
  'tmp=$(mktemp -d); trap '\''rm -rf "$tmp"'\'' EXIT; printf '\''fn main() {}\n'\'' > "$tmp/main.rs"; rustc -C link-arg=-fuse-ld=mold "$tmp/main.rs" -o "$tmp/test"; "$tmp/test"'
