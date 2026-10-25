#!/usr/bin/env bash
# Fetches the GPL ffmpeg and ffprobe that the desktop bundle ships as Tauri
# sidecars, checks them against the pinned hashes, and writes the license
# files that go into the bundle. Usage: fetch-ffmpeg.sh <rust target triple>
#
# Both builds are pinned by a versioned URL and a sha256. A changed file
# fails the build instead of shipping a binary nobody reviewed.
set -euo pipefail

target=${1:?usage: fetch-ffmpeg.sh <target triple>}
version=9.0.2
root=$(cd "$(dirname "$0")/../.." && pwd)
app=$root/desktop/src-tauri
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

sha256() {
  if command -v sha256sum >/dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1
}

fetch() {
  local url=$1 sum=$2 out=$3
  curl --fail --location --silent --show-error --retry 3 --output "$out" "$url"
  local got
  got=$(sha256 "$out")
  if [ "$got" != "$sum" ]; then
    echo "sha256 mismatch for $url: expected $sum, got $got" >&2
    exit 1
  fi
}

mkdir -p "$app/binaries" "$app/licenses"
case $target in
  aarch64-apple-darwin)
    # Martin Riedl's static macOS builds keep each release under its own
    # timestamped path. The build script is at
    # https://git.martin-riedl.de/ffmpeg/build-script.
    base=https://ffmpeg.martin-riedl.de/download/macos/arm64/1789931890_9.0.2
    fetch "$base/ffmpeg.zip" c8ed4c4e6978a03c485edbfe4e0a5dc2380f8a30bba5150531b31b094492d924 "$work/ffmpeg.zip"
    fetch "$base/ffprobe.zip" fcbe839537485eaee7a7a8bc5cbc0f90d53617e80943e8a5b2e31cb851197ea6 "$work/ffprobe.zip"
    unzip -q "$work/ffmpeg.zip" -d "$work"
    unzip -q "$work/ffprobe.zip" -d "$work"
    exe=
    provider="Martin Riedl's static macOS arm64 build ($base/)"
    build_source=https://git.martin-riedl.de/ffmpeg/build-script
    ;;
  x86_64-pc-windows-msvc)
    # Gyan Doshi's essentials builds, from the GitHub mirror that keeps one
    # release per version.
    url=https://github.com/GyanD/codexffmpeg/releases/download/$version/ffmpeg-$version-essentials_build.zip
    fetch "$url" 60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba "$work/ffmpeg.zip"
    7z x -bd -y -o"$work/zip" "$work/ffmpeg.zip" >/dev/null
    mv "$work/zip/ffmpeg-$version-essentials_build/bin/ffmpeg.exe" "$work/zip/ffmpeg-$version-essentials_build/bin/ffprobe.exe" "$work"
    exe=.exe
    provider="Gyan Doshi's essentials build for Windows ($url)"
    build_source=https://www.gyan.dev/ffmpeg/builds/
    ;;
  *)
    echo "no pinned ffmpeg for $target" >&2
    exit 1
    ;;
esac

for tool in ffmpeg ffprobe; do
  # Tauri looks for each sidecar under its name plus the target triple.
  install -m 0755 "$work/$tool$exe" "$app/binaries/$tool-$target$exe"
done

# Both builds are configured with --enable-gpl --enable-version3, so they
# are under the GPL version 3, whose text is this repository's LICENSE.
{
  cat <<NOTICE
This application includes ffmpeg and ffprobe $version, from
$provider.

FFmpeg is licensed under the GNU General Public License version 3, as built
here with --enable-gpl --enable-version3. Its source code is available at
https://ffmpeg.org/releases/ffmpeg-$version.tar.xz and the build
configuration and the sources of the bundled libraries at
$build_source.

NOTICE
  cat "$root/LICENSE"
} > "$app/licenses/FFMPEG.txt"
cp "$root/LICENSE" "$app/licenses/kikoeru-neo-LICENSE.txt"
echo "ffmpeg $version for $target is in $app/binaries"
