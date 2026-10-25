//! Runs the real ffmpeg and ffprobe on generated audio.

use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use neo_media::{
    AAC_PARAMS, CacheKey, Input, Job, MediaError, Tools, TranscodeCache, probe, transcode_aac,
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

fn sine(dir: &Path, name: &str, seconds: u32) -> PathBuf {
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
        .arg(format!("sine=frequency=440:duration={seconds}"))
        .arg("-y")
        .arg(&path)
        .status()
        .expect("ffmpeg must be on PATH for these tests");
    assert!(status.success());
    path
}

fn key(size: u64) -> CacheKey {
    CacheKey {
        root_id: "local".into(),
        source_version: 0,
        path: b"RJ01/wav/01.wav".to_vec(),
        size,
        mtime: Some(1_700_000_000_000),
        params: AAC_PARAMS,
    }
}

fn job<'a>(input: &'a Input, key: &'a CacheKey) -> Job<'a> {
    Job {
        input,
        key,
        duration_ms: Some(2000),
        timeout: Duration::from_secs(60),
    }
}

async fn same(key: &CacheKey) -> Result<(u64, Option<i64>), MediaError> {
    Ok((key.size, key.mtime))
}

#[tokio::test]
async fn probe_reads_a_wav() {
    let dir = tempfile::tempdir().unwrap();
    let wav = sine(dir.path(), "a -odd: name.wav", 2);
    let result = probe(
        &Tools::default(),
        &Input::File(wav),
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert_eq!(result.format, "wav");
    let audio = result.audio.unwrap();
    assert!(audio.codec.starts_with("pcm_"), "{audio:?}");
    assert!((1900..=2100).contains(&result.duration_ms.unwrap()));
    assert_eq!(result.video, None);
}

#[tokio::test]
async fn probe_of_a_non_media_file_fails() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, b"not audio").unwrap();
    let error = probe(
        &Tools::default(),
        &Input::File(path),
        Duration::from_secs(10),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, MediaError::Failed { .. }), "{error:?}");
}

#[tokio::test]
async fn transcode_publishes_a_verified_aac_file() {
    let dir = tempfile::tempdir().unwrap();
    let wav = sine(dir.path(), "in.wav", 2);
    let cache = TranscodeCache::open(&dir.path().join("cache"), u64::MAX, 0).unwrap();
    let key = key(std::fs::metadata(&wav).unwrap().len());
    let input = Input::File(wav);
    let (tx, rx) = watch::channel(0.0);

    let file = transcode_aac(
        &cache,
        job(&input, &key),
        &tx,
        &CancellationToken::new(),
        same(&key),
    )
    .await
    .unwrap();
    assert!(file.len > 0);
    assert_eq!(*rx.borrow(), 1.0);
    let out = probe(
        &Tools::default(),
        &Input::File(file.path.clone()),
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert_eq!(out.audio.unwrap().codec, "aac");
    assert_eq!(
        std::fs::read_dir(dir.path().join("cache/tmp"))
            .unwrap()
            .count(),
        0
    );
    assert!(cache.lookup(&key).unwrap().is_some());
}

#[tokio::test]
async fn a_source_that_changed_meanwhile_is_not_published() {
    let dir = tempfile::tempdir().unwrap();
    let wav = sine(dir.path(), "in.wav", 1);
    let cache = TranscodeCache::open(&dir.path().join("cache"), u64::MAX, 0).unwrap();
    let key = key(std::fs::metadata(&wav).unwrap().len());
    let input = Input::File(wav);
    let (tx, _rx) = watch::channel(0.0);
    let changed = async { Ok((key.size + 1, key.mtime)) };

    let error = transcode_aac(
        &cache,
        job(&input, &key),
        &tx,
        &CancellationToken::new(),
        changed,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, MediaError::SourceChanged), "{error:?}");
    assert!(cache.lookup(&key).unwrap().is_none());
    assert_eq!(
        std::fs::read_dir(dir.path().join("cache/tmp"))
            .unwrap()
            .count(),
        0
    );
}

#[tokio::test]
async fn a_failed_transcode_leaves_nothing_behind() {
    let dir = tempfile::tempdir().unwrap();
    let bogus = dir.path().join("bogus.wav");
    std::fs::write(&bogus, b"RIFF but not really").unwrap();
    let cache = TranscodeCache::open(&dir.path().join("cache"), u64::MAX, 0).unwrap();
    let key = key(19);
    let input = Input::File(bogus);
    let (tx, _rx) = watch::channel(0.0);

    let error = transcode_aac(
        &cache,
        job(&input, &key),
        &tx,
        &CancellationToken::new(),
        same(&key),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, MediaError::Failed { .. }), "{error:?}");
    assert!(cache.lookup(&key).unwrap().is_none());
    assert_eq!(
        std::fs::read_dir(dir.path().join("cache/tmp"))
            .unwrap()
            .count(),
        0
    );
}

/// A FIFO with no writer keeps ffmpeg blocked on its input forever, which
/// makes cancellation and timeout deterministic.
fn stuck_input(dir: &Path) -> Input {
    let fifo = dir.join("stuck.wav");
    rustix::fs::mknodat(
        rustix::fs::CWD,
        &fifo,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o600),
        0,
    )
    .unwrap();
    Input::File(fifo)
}

#[tokio::test]
async fn cancelling_terminates_ffmpeg_and_removes_the_temporary_file() {
    let dir = tempfile::tempdir().unwrap();
    let input = stuck_input(dir.path());
    let cache = TranscodeCache::open(&dir.path().join("cache"), u64::MAX, 0).unwrap();
    let key = key(1);
    let (tx, _rx) = watch::channel(0.0);
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        trigger.cancel();
    });

    let started = std::time::Instant::now();
    let error = transcode_aac(&cache, job(&input, &key), &tx, &cancel, same(&key))
        .await
        .unwrap_err();
    assert!(matches!(error, MediaError::Cancelled), "{error:?}");
    // ffmpeg ignores SIGTERM while blocked opening the FIFO, so this also
    // covers the SIGKILL after the grace period.
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(
        std::fs::read_dir(dir.path().join("cache/tmp"))
            .unwrap()
            .count(),
        0
    );
}

#[tokio::test]
async fn a_timeout_terminates_ffmpeg() {
    let dir = tempfile::tempdir().unwrap();
    let input = stuck_input(dir.path());
    let cache = TranscodeCache::open(&dir.path().join("cache"), u64::MAX, 0).unwrap();
    let key = key(1);
    let (tx, _rx) = watch::channel(0.0);
    let job = Job {
        timeout: Duration::from_millis(300),
        ..job(&input, &key)
    };

    let error = transcode_aac(&cache, job, &tx, &CancellationToken::new(), same(&key))
        .await
        .unwrap_err();
    assert!(matches!(error, MediaError::Timeout("ffmpeg")), "{error:?}");
}

#[tokio::test]
async fn low_disk_space_refuses_before_starting() {
    let dir = tempfile::tempdir().unwrap();
    let wav = sine(dir.path(), "in.wav", 1);
    let cache = TranscodeCache::open(&dir.path().join("cache"), u64::MAX, u64::MAX).unwrap();
    let key = key(std::fs::metadata(&wav).unwrap().len());
    let input = Input::File(wav);
    let (tx, _rx) = watch::channel(0.0);
    let error = transcode_aac(
        &cache,
        job(&input, &key),
        &tx,
        &CancellationToken::new(),
        same(&key),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, MediaError::InsufficientStorage),
        "{error:?}"
    );
}

#[tokio::test]
async fn an_open_handle_is_read_even_after_its_path_is_swapped() {
    let dir = tempfile::tempdir().unwrap();
    let wav = sine(dir.path(), "in.wav", 2);
    let handle = std::sync::Arc::new(std::fs::File::open(&wav).unwrap());
    // Replace the path with a symlink to something else. The handle still
    // names the original file.
    std::fs::remove_file(&wav).unwrap();
    std::os::unix::fs::symlink("/etc/hostname", &wav).unwrap();
    let input = Input::Handle(handle);

    let probed = probe(&Tools::default(), &input, Duration::from_secs(10))
        .await
        .unwrap();
    assert_eq!(probed.format, "wav");

    let cache = TranscodeCache::open(&dir.path().join("cache"), u64::MAX, 0).unwrap();
    let key = key(1);
    let (tx, _rx) = watch::channel(0.0);
    let file = transcode_aac(
        &cache,
        job(&input, &key),
        &tx,
        &CancellationToken::new(),
        same(&key),
    )
    .await
    .unwrap();
    let out = probe(
        &Tools::default(),
        &Input::File(file.path.clone()),
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert!((1900..=2200).contains(&out.duration_ms.unwrap()), "{out:?}");
}

#[tokio::test]
async fn cancelling_during_verification_publishes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let wav = sine(dir.path(), "in.wav", 1);
    let cache = TranscodeCache::open(&dir.path().join("cache"), u64::MAX, 0).unwrap();
    let key = key(std::fs::metadata(&wav).unwrap().len());
    let input = Input::File(wav);
    let (tx, _rx) = watch::channel(0.0);
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let (size, mtime) = (key.size, key.mtime);
    let slow_restat = async move {
        trigger.cancel();
        tokio::time::sleep(Duration::from_secs(60)).await;
        Ok((size, mtime))
    };
    let error = transcode_aac(&cache, job(&input, &key), &tx, &cancel, slow_restat)
        .await
        .unwrap_err();
    assert!(matches!(error, MediaError::Cancelled), "{error:?}");
    assert!(cache.lookup(&key).unwrap().is_none());
}

#[tokio::test]
async fn an_expired_cache_entry_is_transcoded_again() {
    let dir = tempfile::tempdir().unwrap();
    let wav = sine(dir.path(), "in.wav", 1);
    let cache = TranscodeCache::open(&dir.path().join("cache"), u64::MAX, 0).unwrap();
    let key = CacheKey {
        mtime: None,
        ..key(std::fs::metadata(&wav).unwrap().len())
    };
    let input = Input::File(wav);
    let (tx, _rx) = watch::channel(0.0);
    let first = transcode_aac(
        &cache,
        job(&input, &key),
        &tx,
        &CancellationToken::new(),
        same(&key),
    )
    .await
    .unwrap();
    let path = first.path.clone();
    drop(first);
    std::fs::write(&path, b"stale").unwrap();
    let old = std::time::SystemTime::now() - Duration::from_secs(8 * 24 * 3600);
    std::fs::File::open(&path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(old))
        .unwrap();
    let again = transcode_aac(
        &cache,
        job(&input, &key),
        &tx,
        &CancellationToken::new(),
        same(&key),
    )
    .await
    .unwrap();
    assert!(again.len > 5, "the stale entry was replaced");
}

#[tokio::test]
async fn a_manifest_disguised_as_audio_cannot_open_other_files() {
    let dir = tempfile::tempdir().unwrap();
    let outside = sine(dir.path(), "outside.wav", 1);
    let manifest = dir.path().join("disguised.mp3");
    std::fs::write(
        &manifest,
        format!(
            r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT1S"
     minBufferTime="PT1S" profiles="urn:mpeg:dash:profile:isoff-on-demand:2011">
 <Period><AdaptationSet mimeType="audio/wav"><Representation id="a" bandwidth="1000">
  <BaseURL>file:{}</BaseURL>
 </Representation></AdaptationSet></Period></MPD>"#,
            outside.display()
        ),
    )
    .unwrap();
    let error = probe(
        &Tools::default(),
        &Input::File(manifest),
        Duration::from_secs(10),
    )
    .await
    .unwrap_err();
    let MediaError::Failed { stderr, .. } = error else {
        panic!("{error:?}");
    };
    assert!(stderr.contains("not on whitelist"), "{stderr}");
}

/// The configured binaries are the ones run: pointing them at a path that
/// does not exist makes both fail to start, instead of finding the PATH
/// copies.
#[tokio::test]
async fn configured_tool_paths_are_the_ones_run() {
    let dir = tempfile::tempdir().unwrap();
    let wav = sine(dir.path(), "a.wav", 1);
    let missing = Tools {
        ffmpeg: dir.path().join("no-ffmpeg"),
        ffprobe: dir.path().join("no-ffprobe"),
    };
    let error = probe(&missing, &Input::File(wav.clone()), Duration::from_secs(10))
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            MediaError::Spawn {
                program: "ffprobe",
                ..
            }
        ),
        "{error:?}"
    );

    let cache = TranscodeCache::open(&dir.path().join("cache"), u64::MAX, 0)
        .unwrap()
        .with_tools(missing);
    let input = Input::File(wav);
    let size = std::fs::metadata(dir.path().join("a.wav")).unwrap().len();
    let key = key(size);
    let (progress, _) = watch::channel(0.0);
    let error = transcode_aac(
        &cache,
        job(&input, &key),
        &progress,
        &CancellationToken::new(),
        same(&key),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            error,
            MediaError::Spawn {
                program: "ffmpeg",
                ..
            }
        ),
        "{error:?}"
    );
}
