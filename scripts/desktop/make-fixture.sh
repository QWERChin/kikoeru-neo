#!/usr/bin/env bash
# Writes a small synthetic library for the desktop checks: one work with an
# mp3 and an mp4 that a browser plays directly, and a wav and an mkv that go
# through ffmpeg (an audio transcode and the HLS fallback). The mp4 has an
# ass subtitle, which JASSUB draws in a worker with WebAssembly.
# Usage: make-fixture.sh <ffmpeg> <library dir>
set -euo pipefail

ffmpeg=${1:?usage: make-fixture.sh <ffmpeg> <library dir>}
work=${2:?usage: make-fixture.sh <ffmpeg> <library dir>}/RJ01000001
mkdir -p "$work"
run() { "$ffmpeg" -hide_banner -loglevel error -y "$@"; }

tone=(-f lavfi -i "sine=frequency=440:duration=8")
picture=(-f lavfi -i "testsrc2=size=320x240:rate=25:duration=8")
run "${tone[@]}" -c:a libmp3lame -b:a 128k "$work/01 tone.mp3"
run "${tone[@]}" -c:a pcm_s16le "$work/02 tone.wav"
run "${picture[@]}" "${tone[@]}" -c:v libx264 -pix_fmt yuv420p -c:a aac \
  -movflags +faststart -shortest "$work/03 picture.mp4"
run "${picture[@]}" "${tone[@]}" -c:v libx264 -pix_fmt yuv420p -c:a aac \
  -shortest "$work/04 picture.mkv"
cat > "$work/03 picture.ass" <<'ASS'
[Script Info]
ScriptType: v4.00+
PlayResX: 320
PlayResY: 240

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,Arial,28,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,0,2,10,10,10,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:00.00,0:00:08.00,Default,,0,0,0,,fixture subtitle
ASS
echo "fixture library in ${2}"
