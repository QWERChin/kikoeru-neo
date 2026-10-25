#!/usr/bin/env bash
# Writes the synthetic library for the iOS Simulator checks: one work whose
# top folder holds mp3s of 90, 50 and 30 seconds and a 20 second wma, and a
# video folder with an mp4 and an mkv. The first track is long enough to
# seek and background, then ends while the phone is locked so the next one
# has to start on its own. iOS cannot play wma or mkv, so those take the
# AAC transcode and the HLS paths. The lengths differ so the test can tell
# the tracks apart by duration alone.
# Usage: make-library.sh <ffmpeg> <library dir>
# macOS runners ship bash 3.2, so this sticks to what it supports.
set -euo pipefail

ffmpeg=${1:?usage: make-library.sh <ffmpeg> <library dir>}
work=${2:?usage: make-library.sh <ffmpeg> <library dir>}/RJ01000001
mkdir -p "$work/video"
run() { "$ffmpeg" -hide_banner -loglevel error -y "$@"; }
tone() { echo "sine=frequency=$1:duration=$2"; }

run -f lavfi -i "$(tone 440 90)" -c:a libmp3lame -b:a 128k "$work/01 long tone.mp3"
run -f lavfi -i "$(tone 550 50)" -c:a libmp3lame -b:a 128k "$work/02 middle tone.mp3"
run -f lavfi -i "$(tone 660 30)" -c:a libmp3lame -b:a 128k "$work/03 short tone.mp3"
run -f lavfi -i "$(tone 770 20)" -c:a wmav2 -b:a 128k "$work/04 transcode.wma"

# The mp4 runs long enough to go fullscreen, rotate and float in picture in
# picture before it ends.
picture() { echo "testsrc2=size=640x360:rate=25:duration=$1"; }
run -f lavfi -i "$(picture 60)" -f lavfi -i "$(tone 880 60)" -c:v libx264 -pix_fmt yuv420p -c:a aac \
  -movflags +faststart -shortest "$work/video/01 picture.mp4"
run -f lavfi -i "$(picture 30)" -f lavfi -i "$(tone 990 30)" -c:v libx264 -pix_fmt yuv420p -c:a aac \
  -shortest "$work/video/02 picture.mkv"
echo "iOS fixture library in ${2}"
