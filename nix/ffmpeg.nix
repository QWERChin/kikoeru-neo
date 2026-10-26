# The server uses ffmpeg-headless without unused external libraries.
# Required components are checked below against the media input whitelist.
# Cache availability depends on the pinned inputs and substituters.
# See book/src/deployment.md (Chinese) for the packaging rationale.
{ pkgs, inputRs }:

let
  ffmpeg = pkgs.ffmpeg-headless.override {
    # Encoders: H.264 (libx264), native AAC and PCM for waveform analysis.
    withAom = false;
    withMp3lame = false;
    withOpenapv = false;
    withSvtav1 = false;
    withTheora = false;
    withVpx = false;
    withWebp = false;
    withX265 = false;
    withXvid = false;
    # Decoders ffmpeg also has natively: Opus, Vorbis, Speex, JPEG 2000.
    # libopenmpt is for tracker music, which the format whitelist excludes,
    # and pulled in JACK, D-Bus and systemd.
    withOpenjpeg = false;
    withOpenmpt = false;
    withOpus = false;
    withSpeex = false;
    withVorbis = false;
    # Inputs are local files and pipes only (crates/neo-media/src/input.rs).
    withNetwork = false;
    withGmp = false;
    withGnutls = false;
    withRist = false;
    withSrt = false;
    withSsh = false;
    withXml2 = false;
    # Devices, discs and hardware acceleration.
    withAlsa = false;
    withAmf = false;
    withBluray = false;
    withCudaLLVM = false;
    withCuvid = false;
    withDrm = false;
    withNvcodec = false;
    withNvdec = false;
    withNvenc = false;
    withOpencl = false;
    withV4l2 = false;
    withVaapi = false;
    withVulkan = false;
    # Text drawing and subtitle burn-in: JASSUB draws ass in the browser.
    withAss = false;
    withFontconfig = false;
    withFreetype = false;
    withFribidi = false;
    withHarfbuzz = false;
    withZvbi = false;
    # Filters the server does not run.
    withSoxr = false;
    withVidStab = false;
    withZimg = false;
    withHtmlDoc = false;
    withManPages = false;
    withPodDoc = false;
    withTxtDoc = false;
  };
in
{
  package = ffmpeg;

  # A nixpkgs update could drop a component the server relies on without
  # anything else noticing until a track fails to play. The demuxers are
  # read from the whitelist in input.rs, so the two cannot drift apart.
  check = pkgs.runCommand "kikoeru-neo-ffmpeg-check"
    {
      nativeBuildInputs = [ ffmpeg ];
    }
    ''
      set -euo pipefail
      names() {
        # Combined demuxer names such as "mov,mp4,m4a" count for each part.
        ffmpeg -hide_banner -"$1" 2>/dev/null | awk 'NR > 1 { print $2 }' | tr , '\n'
      }
      need() {
        local kind=$1 list
        shift
        list=$(names "$kind")
        for n in "$@"; do
          if ! grep -qx -- "$n" <<<"$list"; then
            echo "ffmpeg has no $kind entry $n" >&2
            exit 1
          fi
        done
      }
      formats=$(sed -n 's/^const FORMATS: &str = "\(.*\)";$/\1/p' ${inputRs} | tr , ' ')
      test -n "$formats"
      need demuxers $formats
      need decoders h264 hevc mpeg4 mpeg2video vp8 vp9 libdav1d wmv2 vc1 \
        aac mp3float flac alac opus vorbis ape wavpack tta dsd_lsbf ac3 eac3 dca \
        wmav2 amrnb amrwb speex pcm_s16le pcm_s24le pcm_f32le
      need encoders libx264 aac pcm_s16le
      need muxers hls mpegts mp4 s16le
      need filters scale asplit ebur128 anullsink aresample aformat
      protocols=$(ffmpeg -hide_banner -protocols 2>/dev/null | awk '{ print $1 }')
      for p in file pipe; do
        grep -qx "$p" <<<"$protocols" || { echo "ffmpeg has no protocol $p" >&2; exit 1; }
      done

      # And one real run of each encoder the server uses.
      ffmpeg -nostdin -hide_banner -loglevel error \
        -f lavfi -i testsrc=size=64x48:rate=10:duration=1 -f lavfi -i sine=duration=1 \
        -c:v libx264 -c:a aac -f mp4 "$TMPDIR/out.mp4"
      ffprobe -v error -show_entries stream=codec_name -of csv=p=0 "$TMPDIR/out.mp4" \
        | sort | tr '\n' ' ' | grep -qx 'aac h264 '
      touch $out
    '';
}
