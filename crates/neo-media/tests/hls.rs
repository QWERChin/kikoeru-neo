//! HLS for video with the real ffmpeg, on generated test pictures.

use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use neo_media::{
    CacheKey, HlsJob, HlsMode, Input, MediaError, PLAYLIST, Tools, TranscodeCache, probe,
    transcode_hls,
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// A test picture with a tone, encoded as `codecs` into `name`.
pub fn video(dir: &Path, name: &str, seconds: u32, codecs: &[&str]) -> PathBuf {
    let path = dir.join(name);
    let status = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg(format!("testsrc=size=320x240:rate=25:duration={seconds}"))
        .args(["-f", "lavfi", "-i"])
        .arg(format!("sine=duration={seconds}"))
        .args(codecs)
        .arg("-y")
        .arg(&path)
        .status()
        .expect("ffmpeg must be on PATH for these tests");
    assert!(status.success());
    path
}

fn key(mode: HlsMode, size: u64) -> CacheKey {
    CacheKey {
        root_id: "local".into(),
        source_version: 0,
        path: b"RJ01/video/movie".to_vec(),
        size,
        mtime: Some(1_700_000_000_000),
        params: mode.params(),
    }
}

async fn run(
    cache: &TranscodeCache,
    source: &Path,
    mode: HlsMode,
) -> Result<neo_media::CachedFile, MediaError> {
    let size = std::fs::metadata(source).unwrap().len();
    let key = key(mode, size);
    let input = Input::File(source.to_owned());
    let (progress, _) = watch::channel(0.0);
    transcode_hls(
        cache,
        HlsJob {
            input: &input,
            key: &key,
            mode,
            duration_ms: Some(13_000),
            timeout: Duration::from_secs(120),
        },
        &progress,
        &CancellationToken::new(),
        async { Ok((key.size, key.mtime)) },
    )
    .await
}

fn segments(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join(PLAYLIST))
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(str::to_owned)
        .collect()
}

#[tokio::test]
async fn h264_aac_in_matroska_is_copied_into_six_second_segments() {
    let dir = tempfile::tempdir().unwrap();
    let source = video(
        dir.path(),
        "movie.mkv",
        13,
        &["-c:v", "libx264", "-g", "25", "-c:a", "aac"],
    );
    let cache = TranscodeCache::open(&dir.path().join("cache"), u64::MAX, 0).unwrap();
    let published = run(&cache, &source, HlsMode::Copy).await.unwrap();
    assert!(published.path.to_string_lossy().ends_with(".hls"));
    let names = segments(&published.path);
    assert_eq!(names.len(), 3, "13 s in 6 s segments: {names:?}");
    let playlist = std::fs::read_to_string(published.path.join(PLAYLIST)).unwrap();
    assert!(playlist.contains("#EXT-X-PLAYLIST-TYPE:VOD"));
    assert!(playlist.contains("#EXT-X-ENDLIST"));
    let first = probe(
        &Tools::default(),
        &Input::File(published.path.join(&names[0])),
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert_eq!(first.format, "mpegts");
    assert_eq!(first.video.unwrap().codec, "h264");
    assert_eq!(first.audio.unwrap().codec, "aac");
    assert_eq!(
        std::fs::read_dir(dir.path().join("cache/tmp"))
            .unwrap()
            .count(),
        0,
        "the temporary directory was renamed away"
    );
}

#[tokio::test]
async fn other_codecs_are_transcoded_to_h264_and_aac() {
    let dir = tempfile::tempdir().unwrap();
    let source = video(
        dir.path(),
        "old.avi",
        13,
        &["-c:v", "mpeg4", "-c:a", "libmp3lame"],
    );
    let cache = TranscodeCache::open(&dir.path().join("cache"), u64::MAX, 0).unwrap();
    let published = run(&cache, &source, HlsMode::Transcode).await.unwrap();
    let names = segments(&published.path);
    assert!((2..=3).contains(&names.len()), "{names:?}");
    let first = probe(
        &Tools::default(),
        &Input::File(published.path.join(&names[0])),
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert_eq!(first.video.unwrap().codec, "h264");
    assert_eq!(first.audio.unwrap().codec, "aac");

    // A second request is a cache hit, not a second ffmpeg run.
    let again = run(&cache, &source, HlsMode::Transcode).await.unwrap();
    assert_eq!(again.path, published.path);
}

#[tokio::test]
async fn copying_a_codec_hls_cannot_hold_fails_and_leaves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    // VP9 in matroska: copy is the wrong mode, and the check refuses it.
    let source = video(
        dir.path(),
        "vp9.mkv",
        3,
        &["-c:v", "libvpx-vp9", "-c:a", "libopus"],
    );
    let cache = TranscodeCache::open(&dir.path().join("cache"), u64::MAX, 0).unwrap();
    let error = run(&cache, &source, HlsMode::Copy).await.unwrap_err();
    assert!(
        matches!(error, MediaError::BadOutput | MediaError::Failed { .. }),
        "{error:?}"
    );
    assert_eq!(
        std::fs::read_dir(dir.path().join("cache/tmp"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        std::fs::read_dir(dir.path().join("cache/transcode"))
            .unwrap()
            .count(),
        0
    );
}

#[tokio::test]
async fn odd_sized_video_is_scaled_to_even_dimensions() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("odd.mkv");
    let status = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg("testsrc=size=321x241:rate=25:duration=3")
        .args(["-c:v", "libvpx-vp9", "-y"])
        .arg(&source)
        .status()
        .unwrap();
    assert!(status.success());
    let cache = TranscodeCache::open(&dir.path().join("cache"), u64::MAX, 0).unwrap();
    let published = run(&cache, &source, HlsMode::Transcode).await.unwrap();
    let first = &segments(&published.path)[0];
    let video = probe(
        &Tools::default(),
        &Input::File(published.path.join(first)),
        Duration::from_secs(10),
    )
    .await
    .unwrap()
    .video
    .unwrap();
    assert_eq!((video.width, video.height), (Some(320), Some(240)));
}

#[tokio::test]
async fn large_video_is_scaled_down_to_1080p_keeping_its_shape() {
    let dir = tempfile::tempdir().unwrap();
    // Wide, tall, and 4:3 sources; a small one is never enlarged.
    for (size, want) in [
        ("2560x1440", (1920, 1080)),
        ("1440x2560", (1080, 1920)),
        ("2880x2160", (1440, 1080)),
        ("640x360", (640, 360)),
    ] {
        let source = dir.path().join(format!("{size}.avi"));
        let status = Command::new("ffmpeg")
            .args(["-nostdin", "-hide_banner", "-loglevel", "error"])
            .args(["-f", "lavfi", "-i"])
            .arg(format!("testsrc=size={size}:rate=25:duration=1"))
            .args(["-c:v", "mpeg4", "-y"])
            .arg(&source)
            .status()
            .unwrap();
        assert!(status.success());
        let cache =
            TranscodeCache::open(&dir.path().join(format!("cache-{size}")), u64::MAX, 0).unwrap();
        let published = run(&cache, &source, HlsMode::Transcode).await.unwrap();
        let first = &segments(&published.path)[0];
        let video = probe(
            &Tools::default(),
            &Input::File(published.path.join(first)),
            Duration::from_secs(10),
        )
        .await
        .unwrap()
        .video
        .unwrap();
        assert_eq!(
            (video.width, video.height),
            (Some(want.0), Some(want.1)),
            "{size}"
        );
    }
}

#[tokio::test]
async fn a_percent_sign_in_the_data_dir_does_not_reach_ffmpeg_templates() {
    // ffmpeg reads %d and %v in the segment and playlist names as patterns.
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("50%done %v");
    std::fs::create_dir(&data).unwrap();
    let source = video(
        dir.path(),
        "movie.mkv",
        3,
        &["-c:v", "libx264", "-g", "25", "-c:a", "aac"],
    );
    let cache = TranscodeCache::open(&data.join("cache"), u64::MAX, 0).unwrap();
    for mode in [HlsMode::Copy, HlsMode::Transcode] {
        let published = run(&cache, &source, mode).await.unwrap();
        assert!(published.path.starts_with(&data), "{mode:?}");
        let names = segments(&published.path);
        assert!(!names.is_empty(), "{mode:?}");
        for name in names {
            assert!(published.path.join(&name).is_file(), "{mode:?}: {name}");
        }
    }
}

#[tokio::test]
async fn an_expired_set_still_being_sent_is_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let source = video(
        dir.path(),
        "movie.mkv",
        3,
        &["-c:v", "libx264", "-c:a", "aac"],
    );
    let cache = TranscodeCache::open(&dir.path().join("cache"), u64::MAX, 0).unwrap();
    let size = std::fs::metadata(&source).unwrap().len();
    // Without an mtime the entry expires after a week.
    let key = CacheKey {
        mtime: None,
        ..key(HlsMode::Copy, size)
    };
    let input = Input::File(source.clone());
    let make = || async {
        let (progress, _) = watch::channel(0.0);
        transcode_hls(
            &cache,
            HlsJob {
                input: &input,
                key: &key,
                mode: HlsMode::Copy,
                duration_ms: Some(3000),
                timeout: Duration::from_secs(60),
            },
            &progress,
            &CancellationToken::new(),
            async { Ok((key.size, key.mtime)) },
        )
        .await
    };
    let first = make().await.unwrap();
    let week_ago = std::time::SystemTime::now() - Duration::from_secs(8 * 24 * 3600);
    std::fs::File::open(&first.path)
        .unwrap()
        .set_times(
            std::fs::FileTimes::new()
                .set_modified(week_ago)
                .set_accessed(week_ago),
        )
        .unwrap();
    // `first` stays pinned, as a segment response in flight would hold it.
    let second = make().await.unwrap();
    assert_eq!(second.path, first.path);
    assert!(second.path.join(PLAYLIST).exists());
    let published = std::fs::metadata(&second.path).unwrap().modified().unwrap();
    assert!(published > week_ago, "the expired set was replaced");
    assert_eq!(
        std::fs::read_dir(dir.path().join("cache/tmp"))
            .unwrap()
            .count(),
        0
    );
    drop(first);
}

#[tokio::test]
async fn h264_with_b_frames_in_avi_is_copied_despite_missing_timestamps() {
    let dir = tempfile::tempdir().unwrap();
    let source = video(
        dir.path(),
        "movie.avi",
        8,
        &["-c:v", "libx264", "-bf", "2", "-g", "25", "-c:a", "aac"],
    );
    let cache = TranscodeCache::open(&dir.path().join("cache"), u64::MAX, 0).unwrap();
    let published = run(&cache, &source, HlsMode::Copy).await.unwrap();
    assert_eq!(segments(&published.path).len(), 2);
}
