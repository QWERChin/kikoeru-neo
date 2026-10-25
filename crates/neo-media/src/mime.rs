//! `Content-Type` for original files.

use crate::Probe;

/// The MIME type for a file, by its probe when it has one and otherwise by
/// extension. Unknown files are `application/octet-stream`, so a browser
/// never guesses and renders a library file as HTML.
pub fn mime_type(file_name: &str, probe: Option<&Probe>) -> &'static str {
    let ext = file_name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    // ffprobe names one demuxer `matroska,webm` for both containers, so the
    // extension tells them apart.
    let webm = ext == "webm";
    if let Some(probe) = probe {
        let formats: Vec<&str> = probe.format.split(',').collect();
        let has = |name: &str| formats.contains(&name);
        if probe.video.is_some() {
            if has("mp4") || has("mov") {
                return "video/mp4";
            }
            if has("matroska") || has("webm") {
                return if webm {
                    "video/webm"
                } else {
                    "video/x-matroska"
                };
            }
        } else if probe.audio.is_some() {
            match () {
                () if has("mp3") => return "audio/mpeg",
                () if has("flac") => return "audio/flac",
                () if has("wav") => return "audio/wav",
                () if has("mp4") || has("m4a") || has("mov") => return "audio/mp4",
                () if has("ogg") => return "audio/ogg",
                () if has("aac") => return "audio/aac",
                () if (has("matroska") || has("webm")) && webm => return "audio/webm",
                () if has("matroska") => return "audio/x-matroska",
                _ => {}
            }
        }
    }
    match ext.as_str() {
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "mka" => "audio/x-matroska",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AudioStream, VideoStream};

    fn probe(format: &str, video: bool) -> Probe {
        Probe {
            format: format.into(),
            duration_ms: None,
            audio: Some(AudioStream {
                codec: "aac".into(),
                profile: None,
                sample_rate: None,
                channels: None,
            }),
            video: video.then(|| VideoStream {
                codec: "h264".into(),
                width: None,
                height: None,
            }),
        }
    }

    #[test]
    fn probe_wins_over_a_misleading_extension() {
        assert_eq!(
            mime_type("song.mp3", Some(&probe("mov,mp4,m4a,3gp,3g2,mj2", false))),
            "audio/mp4"
        );
        assert_eq!(
            mime_type("clip.m4a", Some(&probe("mov,mp4,m4a,3gp,3g2,mj2", true))),
            "video/mp4"
        );
    }

    #[test]
    fn matroska_and_webm_share_a_demuxer_and_the_extension_decides() {
        assert_eq!(
            mime_type("movie.mkv", Some(&probe("matroska,webm", true))),
            "video/x-matroska"
        );
        assert_eq!(
            mime_type("clip.webm", Some(&probe("matroska,webm", true))),
            "video/webm"
        );
        assert_eq!(
            mime_type("song.webm", Some(&probe("matroska,webm", false))),
            "audio/webm"
        );
        assert_eq!(
            mime_type("song.mka", Some(&probe("matroska,webm", false))),
            "audio/x-matroska"
        );
    }

    #[test]
    fn extension_is_the_fallback_and_unknown_is_opaque() {
        assert_eq!(mime_type("a.MP3", None), "audio/mpeg");
        assert_eq!(mime_type("cover.jpg", None), "image/jpeg");
        assert_eq!(mime_type("readme.html", None), "application/octet-stream");
        assert_eq!(mime_type("noext", None), "application/octet-stream");
    }
}
