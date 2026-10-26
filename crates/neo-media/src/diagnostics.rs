//! Advisory checks for distribution-provided media tools, not readiness gates.

use std::{path::Path, process::Stdio, time::Duration};

use tokio::{process::Command, time::timeout};

use crate::Tools;

const TIMEOUT: Duration = Duration::from_secs(5);
const FFMPEG_FEATURES: &str = "audio transcoding, HLS video, loudness/waveform analysis";
const FFPROBE_FEATURES: &str = "duration/stream probing, playback preparation, HLS validation";

// Keep these aligned with the commands in transcode.rs, hls.rs and analysis.rs,
// and the packaged tool's component check in nix/ffmpeg.nix.
const ENCODERS: &[(&str, &str)] = &[
    ("libx264", "HLS video transcoding"),
    ("aac", "audio transcoding, HLS video transcoding"),
    ("pcm_s16le", "loudness/waveform analysis"),
];
const MUXERS: &[(&str, &str)] = &[
    ("hls", "HLS video"),
    ("mpegts", "HLS video"),
    ("mp4", "audio transcoding"),
    ("s16le", "loudness/waveform analysis"),
];
const FILTERS: &[(&str, &str)] = &[
    ("scale", "HLS video transcoding"),
    ("asplit", "loudness/waveform analysis"),
    ("ebur128", "loudness/waveform analysis"),
    ("anullsink", "loudness/waveform analysis"),
    (
        "aresample",
        "audio transcoding, HLS video transcoding, loudness/waveform analysis",
    ),
    (
        "aformat",
        "audio transcoding, HLS video transcoding, loudness/waveform analysis",
    ),
];

impl Tools {
    /// Check tool availability once. Call from a background task: these warnings
    /// do not determine whether the server or browser-native playback can run.
    pub async fn startup_warnings(&self) -> Vec<String> {
        let (mut warnings, probe) = tokio::join!(
            ffmpeg_warnings(&self.ffmpeg, TIMEOUT),
            run(&self.ffprobe, &["-version"], TIMEOUT),
        );
        if let Err(problem) = probe {
            warnings.push(warning(
                "ffprobe",
                &self.ffprobe,
                &problem,
                FFPROBE_FEATURES,
            ));
        }
        warnings
    }
}

fn warning(tool: &str, path: &Path, problem: &str, features: &str) -> String {
    format!(
        "media tool check: {tool} {path:?}: {problem}; affected features: {features}; \
         install a complete ffmpeg build or set media.{tool} in config.toml"
    )
}

async fn ffmpeg_warnings(path: &Path, limit: Duration) -> Vec<String> {
    if let Err(problem) = run(path, &["-version"], limit).await {
        // An unavailable executable would produce the same error for every list.
        return vec![warning("ffmpeg", path, &problem, FFMPEG_FEATURES)];
    }
    let mut warnings = Vec::new();
    for (arg, kind, required) in [
        ("-encoders", "encoder", ENCODERS),
        ("-muxers", "muxer", MUXERS),
        ("-filters", "filter", FILTERS),
    ] {
        match run(path, &["-hide_banner", arg], limit).await {
            Ok(list) => {
                for &(name, features) in required {
                    // Names occupy the second column. Descriptions can mention
                    // an unavailable encoder, and muxers can have comma aliases.
                    let present = list.lines().any(|line| {
                        line.split_whitespace()
                            .nth(1)
                            .is_some_and(|names| names.split(',').any(|entry| entry == name))
                    });
                    if !present {
                        warnings.push(warning(
                            "ffmpeg",
                            path,
                            &format!("missing {kind} {name}"),
                            features,
                        ));
                    }
                }
            }
            Err(problem) => warnings.push(warning("ffmpeg", path, &problem, FFMPEG_FEATURES)),
        }
    }
    warnings
}

async fn run(path: &Path, args: &[&str], limit: Duration) -> Result<String, String> {
    let mut command = Command::new(path);
    command
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let label = args.join(" ");
    let output = timeout(limit, command.output())
        .await
        .map_err(|_| format!("{label} timed out after {}s", limit.as_secs_f64()))?
        .map_err(|error| format!("cannot run {label} ({error})"))?;
    if !output.status.success() {
        return Err(format!("{label} exited with {}", output.status));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{os::unix::fs::PermissionsExt, path::PathBuf};

    // Concurrent fixture writes and process spawns can leave a script open in
    // another child long enough for exec to return ETXTBSY.
    static FIXTURES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    const GOOD: &str = r#"case "$*" in
      -version) echo 'ffmpeg version fake' ;;
      '-hide_banner -encoders') printf 'Encoders:\n V....D libx264 H.264\n A..... aac AAC\n A....D pcm_s16le PCM\n' ;;
      '-hide_banner -muxers') printf 'Muxers:\n E hls HLS\n E mpegts MPEG-TS\n E mp4 MP4\n E s16le PCM\n' ;;
      '-hide_banner -filters') printf 'Filters:\n ... scale V->V\n ... asplit A->N\n ... ebur128 A->N\n ... anullsink A->|\n ... aresample A->A\n ... aformat A->A\n' ;;
      *) exit 7 ;;
    esac
"#;

    fn fake(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[tokio::test]
    async fn missing_tools_report_each_path_and_config_key_once() {
        let _guard = FIXTURES.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let tools = Tools {
            ffmpeg: dir.path().join("missing ffmpeg"),
            ffprobe: dir.path().join("missing ffprobe"),
        };
        let warnings = tools.startup_warnings().await;
        assert_eq!(warnings.len(), 2);
        for (i, (tool, path, features)) in [
            ("ffmpeg", &tools.ffmpeg, FFMPEG_FEATURES),
            ("ffprobe", &tools.ffprobe, FFPROBE_FEATURES),
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(
                warnings[i],
                warning(
                    tool,
                    path,
                    &format!(
                        "cannot run -version ({})",
                        std::io::Error::from_raw_os_error(2)
                    ),
                    features
                )
            );
        }
    }

    #[tokio::test]
    async fn complete_tools_are_silent() {
        let _guard = FIXTURES.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let tools = Tools {
            ffmpeg: fake(dir.path(), "ffmpeg with spaces", GOOD),
            ffprobe: fake(dir.path(), "ffprobe", "test \"$*\" = -version"),
        };
        assert_eq!(tools.startup_warnings().await, Vec::<String>::new());
    }

    #[tokio::test]
    async fn incomplete_build_reports_x264_not_its_description() {
        let _guard = FIXTURES.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let tools = Tools {
            ffmpeg: fake(
                dir.path(),
                "ffmpeg-free",
                &GOOD.replace("libx264 H.264", "h264_other libx264 is not installed"),
            ),
            ffprobe: fake(dir.path(), "ffprobe", "exit 0"),
        };
        assert_eq!(
            tools.startup_warnings().await,
            vec![format!(
                "media tool check: ffmpeg {:?}: missing encoder libx264; affected features: HLS video transcoding; install a complete ffmpeg build or set media.ffmpeg in config.toml",
                tools.ffmpeg
            )]
        );
    }

    #[tokio::test]
    async fn missing_pcm_muxer_and_filter_are_individual_problems() {
        let _guard = FIXTURES.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let path = fake(
            dir.path(),
            "ffmpeg",
            &GOOD
                .replace("pcm_s16le PCM", "other PCM")
                .replace("s16le PCM", "other PCM")
                .replace("ebur128 A->N", "other A->N"),
        );
        assert_eq!(
            ffmpeg_warnings(&path, TIMEOUT).await,
            [
                "missing encoder pcm_s16le",
                "missing muxer s16le",
                "missing filter ebur128",
            ]
            .map(|problem| warning(
                "ffmpeg",
                &path,
                problem,
                "loudness/waveform analysis"
            ))
        );
    }

    #[tokio::test]
    async fn failed_query_does_not_claim_every_encoder_is_missing() {
        let _guard = FIXTURES.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let path = fake(
            dir.path(),
            "ffmpeg",
            &GOOD.replace("-encoders'", "-unrecognized'"),
        );
        let warnings = ffmpeg_warnings(&path, TIMEOUT).await;
        assert_eq!(
            warnings,
            vec![warning(
                "ffmpeg",
                &path,
                "-hide_banner -encoders exited with exit status: 7",
                FFMPEG_FEATURES
            )]
        );
    }

    #[tokio::test]
    async fn hung_child_times_out_and_is_killed() {
        let _guard = FIXTURES.lock().await;
        let dir = tempfile::tempdir().unwrap();
        // The PID belongs to this fake only; exec avoids an orphaned sleep child.
        let path = fake(dir.path(), "ffmpeg", "echo $$ > \"$0.pid\"\nexec sleep 30");
        let warnings = ffmpeg_warnings(&path, Duration::from_millis(200)).await;
        assert_eq!(
            warnings,
            vec![warning(
                "ffmpeg",
                &path,
                "-version timed out after 0.2s",
                FFMPEG_FEATURES
            )]
        );
        let pid: i32 = std::fs::read_to_string(path.with_extension("pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let pid = rustix::process::Pid::from_raw(pid).unwrap();
        timeout(Duration::from_secs(5), async {
            while rustix::process::test_kill_process(pid).is_ok() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("timed-out child survived");
    }
}
