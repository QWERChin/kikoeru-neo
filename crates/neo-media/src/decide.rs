//! Whether the browser can play a track as it is, or needs the AAC fallback.

use crate::Probe;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Playback {
    /// Serve the original file through `stream`.
    Direct,
    /// Serve an AAC transcode through `playable`.
    Transcode,
}

/// Decides from the probe and the MIME types the client said it can play
/// (from `canPlayType`). `force_fallback` is the client's retry after the
/// original failed to play.
///
/// Only the audio stream counts. A video stream in an audio file, such as
/// Theora in an `.ogg`, does not let it skip the check.
pub fn decide(probe: &Probe, can_play: &[String], force_fallback: bool) -> Playback {
    if force_fallback {
        return Playback::Transcode;
    }
    let candidates = mime_candidates(probe);
    let client: Vec<String> = can_play.iter().map(|m| normalize(m)).collect();
    if candidates
        .iter()
        .any(|c| client.iter().any(|m| m == &normalize(c)))
    {
        Playback::Direct
    } else {
        Playback::Transcode
    }
}

/// How a video track is played.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoPlayback {
    /// Serve the original file through `stream`.
    Direct,
    /// H.264 + AAC in a container the browser does not play: copy the
    /// streams into HLS.
    Remux,
    /// Anything else, or a client retry: transcode to H.264 + AAC HLS.
    Transcode,
}

/// Decides for a video track (book/src/media.md "视频"). A probe without a
/// video stream, such as one ffprobe could not read, is transcoded.
///
/// `file_name` matters because ffprobe reports both matroska and webm as
/// `matroska,webm`, and only a `.webm` is served as `video/webm`.
pub fn decide_video(
    probe: &Probe,
    file_name: &str,
    can_play: &[String],
    force_fallback: bool,
) -> VideoPlayback {
    let Some(video) = &probe.video else {
        return VideoPlayback::Transcode;
    };
    if force_fallback {
        return VideoPlayback::Transcode;
    }
    let audio = probe.audio.as_ref().map(|a| a.codec.as_str());
    let formats: Vec<&str> = probe.format.split(',').collect();
    let has = |name: &str| formats.contains(&name);
    let candidates: &[&str] = match (video.codec.as_str(), audio) {
        ("h264", None | Some("aac" | "mp3")) if has("mp4") || has("mov") => {
            &["video/mp4", "video/mp4; codecs=\"avc1.42E01E, mp4a.40.2\""]
        }
        ("vp8" | "vp9" | "av1", None | Some("opus" | "vorbis"))
            if crate::mime_type(file_name, Some(probe)) == "video/webm" =>
        {
            &["video/webm"]
        }
        _ => &[],
    };
    let client: Vec<String> = can_play.iter().map(|m| normalize(m)).collect();
    if candidates
        .iter()
        .any(|c| client.iter().any(|m| m == &normalize(c)))
    {
        VideoPlayback::Direct
    } else if video.codec == "h264" && matches!(audio, None | Some("aac")) {
        VideoPlayback::Remux
    } else {
        VideoPlayback::Transcode
    }
}

/// MIME types under which a browser would report support for this file.
/// An unknown combination has none, so it is transcoded.
fn mime_candidates(probe: &Probe) -> &'static [&'static str] {
    let Some(audio) = &probe.audio else {
        return &[];
    };
    let formats: Vec<&str> = probe.format.split(',').collect();
    let has = |name: &str| formats.contains(&name);
    match audio.codec.as_str() {
        "mp3" if has("mp3") => &["audio/mpeg", "audio/mp3"],
        "flac" if has("flac") => &["audio/flac", "audio/x-flac"],
        codec if codec.starts_with("pcm_") && has("wav") => {
            &["audio/wav", "audio/wave", "audio/x-wav"]
        }
        "aac" if has("mp4") || has("m4a") || has("mov") => &[
            "audio/mp4",
            "audio/x-m4a",
            "audio/mp4; codecs=\"mp4a.40.2\"",
        ],
        "aac" if has("aac") => &["audio/aac"],
        "alac" if has("m4a") || has("mp4") => &["audio/mp4; codecs=\"alac\""],
        "vorbis" if has("ogg") => &["audio/ogg", "audio/ogg; codecs=\"vorbis\""],
        "opus" if has("ogg") => &["audio/ogg; codecs=\"opus\""],
        "opus" if has("webm") || has("matroska") => &["audio/webm; codecs=\"opus\""],
        _ => &[],
    }
}

/// Lowercase without spaces, so `audio/ogg; codecs="opus"` and
/// `audio/ogg;codecs=opus` compare equal.
fn normalize(mime: &str) -> String {
    mime.chars()
        .filter(|c| !c.is_whitespace() && *c != '"' && *c != '\'')
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AudioStream, VideoStream};

    fn audio(format: &str, codec: &str) -> Probe {
        Probe {
            format: format.into(),
            duration_ms: Some(1000),
            audio: Some(AudioStream {
                codec: codec.into(),
                profile: None,
                sample_rate: Some(44100),
                channels: Some(2),
            }),
            video: None,
        }
    }

    fn chrome() -> Vec<String> {
        [
            "audio/mpeg",
            "audio/flac",
            "audio/wav",
            "audio/mp4",
            "audio/ogg; codecs=\"opus\"",
        ]
        .map(String::from)
        .to_vec()
    }

    fn video(format: &str, video: &str, audio_codec: Option<&str>) -> Probe {
        Probe {
            format: format.into(),
            duration_ms: Some(1000),
            audio: audio_codec.map(|codec| AudioStream {
                codec: codec.into(),
                profile: None,
                sample_rate: Some(48000),
                channels: Some(2),
            }),
            video: Some(VideoStream {
                codec: video.into(),
                width: Some(1280),
                height: Some(720),
            }),
        }
    }

    #[test]
    fn videos_play_directly_remux_or_transcode() {
        let mp4 = "mov,mp4,m4a,3gp,3g2,mj2";
        let client = ["video/mp4", "video/webm"].map(String::from).to_vec();
        let cases = [
            (
                "a.mp4",
                video(mp4, "h264", Some("aac")),
                VideoPlayback::Direct,
            ),
            ("a.mp4", video(mp4, "h264", None), VideoPlayback::Direct),
            (
                "a.webm",
                video("matroska,webm", "vp9", Some("opus")),
                VideoPlayback::Direct,
            ),
            // The same streams in .mkv are served as video/x-matroska.
            (
                "a.mkv",
                video("matroska,webm", "vp9", Some("opus")),
                VideoPlayback::Transcode,
            ),
            (
                "a.mkv",
                video("matroska,webm", "h264", Some("aac")),
                VideoPlayback::Remux,
            ),
            (
                "a.avi",
                video("avi", "h264", Some("aac")),
                VideoPlayback::Remux,
            ),
            (
                "a.mp4",
                video(mp4, "hevc", Some("aac")),
                VideoPlayback::Transcode,
            ),
            (
                "a.mkv",
                video("matroska,webm", "h264", Some("flac")),
                VideoPlayback::Transcode,
            ),
            (
                "a.avi",
                video("avi", "mpeg4", Some("mp3")),
                VideoPlayback::Transcode,
            ),
        ];
        for (name, probe, want) in cases {
            assert_eq!(
                decide_video(&probe, name, &client, false),
                want,
                "{name} {probe:?}"
            );
        }
        // Without client support an mp4 is still H.264 + AAC, so it remuxes.
        assert_eq!(
            decide_video(&video(mp4, "h264", Some("aac")), "a.mp4", &[], false),
            VideoPlayback::Remux
        );
        assert_eq!(
            decide_video(&video(mp4, "h264", Some("aac")), "a.mp4", &client, true),
            VideoPlayback::Transcode
        );
        let unreadable = Probe {
            video: None,
            ..video(mp4, "h264", None)
        };
        assert_eq!(
            decide_video(&unreadable, "a.mp4", &client, false),
            VideoPlayback::Transcode
        );
    }

    #[test]
    fn common_formats_play_directly_when_the_client_supports_them() {
        for (format, codec) in [
            ("mp3", "mp3"),
            ("flac", "flac"),
            ("wav", "pcm_s16le"),
            ("mov,mp4,m4a,3gp,3g2,mj2", "aac"),
            ("ogg", "opus"),
        ] {
            assert_eq!(
                decide(&audio(format, codec), &chrome(), false),
                Playback::Direct,
                "{format} {codec}"
            );
        }
    }

    #[test]
    fn unsupported_or_unknown_audio_is_transcoded() {
        assert_eq!(
            decide(&audio("ogg", "vorbis"), &chrome(), false),
            Playback::Transcode
        );
        assert_eq!(
            decide(&audio("ape", "ape"), &chrome(), false),
            Playback::Transcode
        );
        assert_eq!(
            decide(&audio("wav", "pcm_s16le"), &[], false),
            Playback::Transcode
        );
        let no_audio = Probe {
            audio: None,
            ..audio("mp3", "mp3")
        };
        assert_eq!(decide(&no_audio, &chrome(), false), Playback::Transcode);
    }

    #[test]
    fn force_fallback_always_transcodes() {
        assert_eq!(
            decide(&audio("mp3", "mp3"), &chrome(), true),
            Playback::Transcode
        );
    }

    #[test]
    fn mime_comparison_ignores_case_quotes_and_spaces() {
        let client = vec!["AUDIO/OGG;codecs=opus".to_owned()];
        assert_eq!(
            decide(&audio("ogg", "opus"), &client, false),
            Playback::Direct
        );
    }

    #[test]
    fn audio_tracks_with_a_video_stream_follow_the_mime_check() {
        let with_video = |probe: Probe, codec: &str| Probe {
            video: Some(VideoStream {
                codec: codec.into(),
                width: Some(640),
                height: Some(360),
            }),
            ..probe
        };
        let ogg = with_video(audio("ogg", "vorbis"), "theora");
        assert_eq!(decide(&ogg, &chrome(), false), Playback::Transcode);
        let m4a = with_video(audio("mov,mp4,m4a,3gp,3g2,mj2", "aac"), "h264");
        assert_eq!(decide(&m4a, &chrome(), false), Playback::Direct);
        assert_eq!(decide(&m4a, &[], false), Playback::Transcode);
    }
}
