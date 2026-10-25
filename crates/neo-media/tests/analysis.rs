//! Loudness and waveform from the real ffmpeg on generated audio with a
//! known level.

use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use neo_media::{
    AnalysisJob, CacheKey, Input, MediaError, TranscodeCache, WAVEFORM_BUCKETS, WAVEFORM_PARAMS,
    analyze,
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// Writes a mono WAV from an ffmpeg lavfi source graph.
fn generate(dir: &Path, name: &str, graph: &str) -> PathBuf {
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
        .arg(graph)
        .args(["-ac", "1", "-y"])
        .arg(&path)
        .status()
        .expect("ffmpeg must be on PATH for these tests");
    assert!(status.success());
    path
}

fn key(path: &Path) -> CacheKey {
    CacheKey {
        root_id: "local".into(),
        source_version: 0,
        path: path.file_name().unwrap().as_encoded_bytes().to_vec(),
        size: std::fs::metadata(path).unwrap().len(),
        mtime: Some(1_700_000_000_000),
        params: WAVEFORM_PARAMS,
    }
}

async fn run(
    cache: &TranscodeCache,
    path: &Path,
    cancel: &CancellationToken,
) -> Result<(neo_media::Loudness, Vec<u8>), MediaError> {
    let key = key(path);
    let input = Input::File(path.to_owned());
    let (tx, _rx) = watch::channel(0.0);
    let job = AnalysisJob {
        input: &input,
        key: &key,
        duration_ms: Some(4000),
        timeout: Duration::from_secs(60),
    };
    let (loudness, file) =
        analyze(cache, job, &tx, cancel, async { Ok((key.size, key.mtime)) }).await?;
    Ok((loudness, std::fs::read(&file.path).unwrap()))
}

#[tokio::test]
async fn a_tone_of_known_level_measures_within_half_a_unit() {
    let dir = tempfile::tempdir().unwrap();
    let cache = TranscodeCache::open(&dir.path().join("cache"), u64::MAX, 0).unwrap();
    // A 1 kHz sine at peak amplitude A has a true peak of 20·log10(A) dBTP
    // and, mono and K-weighted, an integrated loudness 3 dB below it.
    for (db, expected_lufs) in [(-6.0, -9.0), (-20.0, -23.0), (-36.0, -39.0)] {
        let wav = generate(
            dir.path(),
            &format!("tone{db}.wav"),
            &format!(
                "sine=frequency=1000:sample_rate=48000:duration=4,volume={}dB",
                db + 18.06
            ),
        );
        let (loudness, waveform) = run(&cache, &wav, &CancellationToken::new()).await.unwrap();
        let lufs = loudness.integrated_lufs.unwrap();
        let peak = loudness.true_peak_dbtp.unwrap();
        assert!((lufs - expected_lufs).abs() <= 0.5, "{db} dB: {lufs} LUFS");
        assert!((peak - db).abs() <= 0.5, "{db} dB: {peak} dBTP");
        assert_eq!(waveform.len(), WAVEFORM_BUCKETS);
        // A steady tone is one level throughout. The first and last few
        // buckets are left out: resampling a tone that starts and stops
        // abruptly overshoots there.
        let expected = (10f64.powf(db / 20.0) * 255.0).ceil();
        for (i, b) in waveform.iter().enumerate().take(995).skip(5) {
            assert!(
                (f64::from(*b) - expected).abs() <= 2.0,
                "{db} dB, bucket {i}: {b}"
            );
        }
    }
}

#[tokio::test]
async fn silence_is_unmeasured_and_its_waveform_is_flat() {
    let dir = tempfile::tempdir().unwrap();
    let cache = TranscodeCache::open(&dir.path().join("cache"), u64::MAX, 0).unwrap();
    let wav = generate(
        dir.path(),
        "silence.wav",
        "anullsrc=sample_rate=48000:channel_layout=mono:d=3",
    );
    let (loudness, waveform) = run(&cache, &wav, &CancellationToken::new()).await.unwrap();
    assert_eq!(loudness.integrated_lufs, None);
    assert_eq!(loudness.true_peak_dbtp, None);
    assert!(waveform.iter().all(|b| *b == 0));
}

#[tokio::test]
async fn the_waveform_follows_where_the_sound_is() {
    let dir = tempfile::tempdir().unwrap();
    let cache = TranscodeCache::open(&dir.path().join("cache"), u64::MAX, 0).unwrap();
    // Two seconds of silence, then two seconds of tone.
    let wav = generate(
        dir.path(),
        "half.wav",
        "aevalsrc=exprs='if(lt(t\\,2)\\,0\\,0.5*sin(2*PI*440*t))':s=48000:d=4",
    );
    let (_, waveform) = run(&cache, &wav, &CancellationToken::new()).await.unwrap();
    assert_eq!(waveform.len(), WAVEFORM_BUCKETS);
    assert!(
        waveform[..490].iter().all(|b| *b == 0),
        "{:?}",
        &waveform[..20]
    );
    assert!(
        waveform[510..].iter().all(|b| (126..=130).contains(b)),
        "{:?}",
        &waveform[990..]
    );
}

#[tokio::test]
async fn a_published_waveform_is_in_the_cache_and_nothing_is_left_in_tmp() {
    let dir = tempfile::tempdir().unwrap();
    let cache = TranscodeCache::open(&dir.path().join("cache"), u64::MAX, 0).unwrap();
    let wav = generate(dir.path(), "tone.wav", "sine=frequency=440:duration=2");
    run(&cache, &wav, &CancellationToken::new()).await.unwrap();
    assert!(cache.lookup(&key(&wav)).unwrap().is_some());
    assert_eq!(
        std::fs::read_dir(dir.path().join("cache/tmp"))
            .unwrap()
            .count(),
        0
    );
}

#[tokio::test]
async fn a_cancelled_analysis_publishes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let cache = TranscodeCache::open(&dir.path().join("cache"), u64::MAX, 0).unwrap();
    let wav = generate(dir.path(), "long.wav", "sine=frequency=440:duration=600");
    let cancel = CancellationToken::new();
    cancel.cancel();
    let error = run(&cache, &wav, &cancel).await.unwrap_err();
    assert!(matches!(error, MediaError::Cancelled), "{error:?}");
    assert!(cache.lookup(&key(&wav)).unwrap().is_none());
}

#[tokio::test]
async fn a_video_without_sound_is_silent_and_flat() {
    let dir = tempfile::tempdir().unwrap();
    let cache = TranscodeCache::open(&dir.path().join("cache"), u64::MAX, 0).unwrap();
    let path = dir.path().join("mute.mkv");
    let status = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=64x64:duration=1",
            "-c:v",
            "mpeg4",
            "-y",
        ])
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success());
    let (loudness, waveform) = run(&cache, &path, &CancellationToken::new()).await.unwrap();
    assert_eq!(loudness.integrated_lufs, None);
    assert_eq!(loudness.true_peak_dbtp, None);
    assert_eq!(waveform.len(), WAVEFORM_BUCKETS);
    assert!(waveform.iter().all(|b| *b == 0));
}

#[cfg(unix)]
#[tokio::test]
async fn an_ffmpeg_that_closes_its_output_but_hangs_still_times_out() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    // Closes stdout, then never exits.
    let fake = dir.path().join("ffmpeg");
    std::fs::write(&fake, "#!/bin/sh\nexec >&-\nsleep 30\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let tools = neo_media::Tools {
        ffmpeg: fake,
        ffprobe: "ffprobe".into(),
    };
    let cache = TranscodeCache::open(&dir.path().join("cache"), u64::MAX, 0)
        .unwrap()
        .with_tools(tools);
    let wav = generate(dir.path(), "tone.wav", "sine=duration=1");
    let key = key(&wav);
    let input = Input::File(wav.clone());
    let (tx, _rx) = watch::channel(0.0);
    let job = AnalysisJob {
        input: &input,
        key: &key,
        duration_ms: Some(1000),
        timeout: Duration::from_millis(300),
    };
    let started = std::time::Instant::now();
    let error = analyze(&cache, job, &tx, &CancellationToken::new(), async {
        Ok((key.size, key.mtime))
    })
    .await
    .unwrap_err();
    assert!(matches!(error, MediaError::Timeout(_)), "{error:?}");
    assert!(started.elapsed() < Duration::from_secs(10));
}
