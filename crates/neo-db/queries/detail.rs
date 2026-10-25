//! `GET /api/works/{id}`: the summary plus voice actors, tags and the
//! flat track list described in book/src/contract.md.

use neo_core::db::detail::*;

use std::collections::HashMap;

use super::{reviews, works};

use neo_core::dto::{
    Decibels, Id, Loudness, SafeInteger, TagRef, TrackItem, TrackKind, VaRef, VideoInfo,
    WorkDetail, WorkLocation,
};

#[cfg(feature = "testing")]
tokio::task_local! {
    /// Pauses this request after its summary so a test can commit a scan.
    pub static AFTER_SUMMARY: std::sync::Arc<tokio::sync::Barrier>;
}

/// `with_locations` is true for admins only: other users never see which
/// roots there are, so their detail carries no locations.
pub async fn get(
    pool: &crate::Pool,
    user_id: i64,
    work_id: i64,
    with_locations: bool,
) -> Result<Option<WorkDetail>, DetailError> {
    // A scan can hide a copy and move its progress together. Every part of
    // the detail must see the same snapshot so resume still finds its track.
    let mut tx = pool.begin().await?;
    let Some(summary) = works::summary(&mut *tx, user_id, work_id).await? else {
        return Ok(None);
    };
    #[cfg(feature = "testing")]
    if let Ok(barrier) = AFTER_SUMMARY.try_with(std::sync::Arc::clone) {
        barrier.wait().await;
        barrier.wait().await;
    }
    // Copies of a file in another location are not listed.
    let count = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "n!: i64" FROM tracks WHERE work_id = $1 AND duplicate_of IS NULL"#,
        work_id
    )
    .fetch_one(&mut *tx)
    .await?;
    if count > MAX_TRACKS {
        return Err(DetailError::TooManyTracks(count));
    }
    let vas = sqlx::query_as!(
        VaRef,
        r#"SELECT v.id AS "id!", v.name FROM work_vas wv JOIN vas v ON v.id = wv.va_id
           WHERE wv.work_id = $1 ORDER BY v.name, v.id"#,
        work_id
    )
    .fetch_all(&mut *tx)
    .await?;
    let tags = sqlx::query!(
        r#"SELECT t.id AS "id!", t.name FROM work_tags wt JOIN tags t ON t.id = wt.tag_id
           WHERE wt.work_id = $1 ORDER BY t.name, t.id"#,
        work_id
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|r| {
        Ok(TagRef {
            id: Id::try_from(r.id)
                .map_err(|reason| DetailError::InvalidTrack { id: r.id, reason })?,
            name: r.name,
        })
    })
    .collect::<Result<Vec<_>, DetailError>>()?;
    let fresh_since = neo_core::db::now_ms() - neo_core::db::analysis::UNKNOWN_MTIME_TTL_MS;
    let rows = sqlx::query!(
        r#"SELECT t.id AS "id!", t.location_id, t.rel_path, t.display_path, t.kind, t.size,
                  t.duration_ms, t.missing_at IS NOT NULL AS "missing!: bool", t.probe,
                  a.track_id IS NOT NULL AS "analyzed!: bool",
                  a.integrated_lufs AS "integrated_lufs?: f64",
                  a.true_peak_dbtp AS "true_peak_dbtp?: f64"
           FROM tracks t
           LEFT JOIN track_analysis a
             ON a.track_id = t.id AND a.size IS NOT DISTINCT FROM t.size
               AND a.mtime IS NOT DISTINCT FROM t.mtime
               AND (a.mtime IS NOT NULL OR a.analyzed_at >= $1)
           WHERE t.work_id = $2 AND t.duplicate_of IS NULL
           ORDER BY t.sort_key, t.id"#,
        fresh_since,
        work_id
    )
    .fetch_all(&mut *tx)
    .await?;
    // A subtitle belongs to tracks in its own location only: two locations
    // may hold different files under the same name. Hidden copies count,
    // since a shown track can sit next to a subtitle that is a copy.
    let subtitle_rows = sqlx::query!(
        r#"SELECT location_id, rel_path FROM tracks
           WHERE work_id = $1 AND kind = 'subtitle' AND missing_at IS NULL"#,
        work_id
    )
    .fetch_all(&mut *tx)
    .await?;
    // Raw paths, not display text: two directories whose names differ only
    // in Unicode normalization are still different directories.
    let mut subtitles: HashMap<(i64, &[u8]), Vec<&'static str>> = HashMap::new();
    for r in &subtitle_rows {
        if let (Some(stem), Some(format)) =
            (strip_extension(&r.rel_path), subtitle_format(&r.rel_path))
        {
            subtitles
                .entry((r.location_id, stem))
                .or_default()
                .push(format);
        }
    }
    let mut tracks = Vec::with_capacity(rows.len());
    let mut total = Some(0i64);
    let mut audio = 0;
    for r in &rows {
        let kind = track_kind(&r.kind);
        let invalid = |reason| DetailError::InvalidTrack { id: r.id, reason };
        let formats = if matches!(kind, TrackKind::Audio | TrackKind::Video) {
            subtitle_formats(r.location_id, &r.rel_path, &subtitles)
        } else {
            Vec::new()
        };
        if kind == TrackKind::Audio && !r.missing {
            audio += 1;
            total = total.zip(r.duration_ms).map(|(a, b)| a.saturating_add(b));
        }
        tracks.push(TrackItem {
            id: Id::try_from(r.id).map_err(invalid)?,
            path: r.display_path.clone(),
            kind,
            size: r
                .size
                .map(SafeInteger::try_from)
                .transpose()
                .map_err(invalid)?,
            duration_ms: r
                .duration_ms
                .map(SafeInteger::try_from)
                .transpose()
                .map_err(invalid)?,
            has_subtitle: !formats.is_empty(),
            subtitle_formats: formats.into_iter().map(str::to_owned).collect(),
            missing: r.missing,
            video: (kind == TrackKind::Video)
                .then(|| video_info(r.probe.as_deref()))
                .flatten(),
            // Only audio is analysed. A row for anything else is left
            // over from before video was skipped.
            loudness: (r.analyzed && kind == TrackKind::Audio).then_some(Loudness {
                integrated_lufs: r.integrated_lufs.map(Decibels),
                true_peak_dbtp: r.true_peak_dbtp.map(Decibels),
            }),
        });
    }
    let my_review = reviews::get(&mut *tx, user_id, work_id).await?;
    let locations = if with_locations {
        Some(locations(&mut tx, work_id).await?)
    } else {
        None
    };
    tx.commit().await?;
    let total_duration_ms = if audio == 0 { None } else { total }
        .map(SafeInteger::try_from)
        .transpose()
        .map_err(|reason| DetailError::InvalidTrack {
            id: work_id,
            reason,
        })?;
    Ok(Some(WorkDetail {
        summary,
        vas,
        tags,
        total_duration_ms,
        tracks,
        my_review,
        locations,
    }))
}

async fn locations(tx: &mut crate::Tx<'_>, work_id: i64) -> sqlx::Result<Vec<WorkLocation>> {
    sqlx::query_as!(
        WorkLocation,
        r#"SELECT l.root_id, r.display_name AS root_name, l.display_path AS path,
                  l.missing_at IS NOT NULL AS "missing!: bool"
           FROM work_locations l JOIN roots r ON r.id = l.root_id
           WHERE l.work_id = $1
           ORDER BY l.missing_at IS NOT NULL, l.root_id, l.display_path, l.id"#,
        work_id
    )
    .fetch_all(&mut **tx)
    .await
}

fn track_kind(kind: &str) -> TrackKind {
    match kind {
        "audio" => TrackKind::Audio,
        "video" => TrackKind::Video,
        "subtitle" => TrackKind::Subtitle,
        "image" => TrackKind::Image,
        "text" => TrackKind::Text,
        _ => TrackKind::Other,
    }
}

/// `mp3/Tr01.lrc` gives `mp3/Tr01`.
pub(crate) fn strip_extension(path: &[u8]) -> Option<&[u8]> {
    let dot = path.iter().rposition(|b| *b == b'.')?;
    let name_start = path.iter().rposition(|b| *b == b'/').map_or(0, |i| i + 1);
    // The dot must be in the file name and must not start it.
    (dot > name_start).then(|| &path[..dot])
}

/// The order `/api/tracks/{id}/subtitle` picks a subtitle in.
const SUBTITLE_ORDER: [&str; 4] = ["vtt", "srt", "lrc", "ass"];

fn subtitle_format(path: &[u8]) -> Option<&'static str> {
    let dot = path.iter().rposition(|b| *b == b'.')?;
    let ext = std::str::from_utf8(&path[dot + 1..])
        .ok()?
        .to_ascii_lowercase();
    // ssa is the older form of ass, and JASSUB and the converter read both.
    let ext = if ext == "ssa" { "ass".to_owned() } else { ext };
    SUBTITLE_ORDER.into_iter().find(|f| *f == ext)
}

/// A subtitle belongs to a track when it sits in the same directory and is
/// named after the track, either replacing its extension (`Tr01.lrc`) or
/// adding one (`Tr01.mp3.vtt`, as some distributions name them). The formats
/// come back in the order the subtitle endpoint prefers them.
fn subtitle_formats(
    location: i64,
    track: &[u8],
    subtitles: &HashMap<(i64, &[u8]), Vec<&'static str>>,
) -> Vec<&'static str> {
    let mut found: Vec<&'static str> = subtitles
        .get(&(location, track))
        .into_iter()
        .chain(strip_extension(track).and_then(|s| subtitles.get(&(location, s))))
        .flatten()
        .copied()
        .collect();
    found.sort_by_key(|f| SUBTITLE_ORDER.iter().position(|o| o == f));
    found.dedup();
    found
}

/// The picture of a probed video, from the `video` object neo-media stores
/// in `tracks.probe`. Anything unreadable counts as not probed.
fn video_info(probe: Option<&str>) -> Option<VideoInfo> {
    let probe: serde_json::Value = serde_json::from_str(probe?).ok()?;
    let video = probe.get("video")?;
    let size = |key| {
        video
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
    };
    Some(VideoInfo {
        codec: video.get("codec")?.as_str()?.to_owned(),
        width: size("width"),
        height: size("height"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtitles_match_by_name_in_the_same_directory() {
        let subs: HashMap<(i64, &[u8]), Vec<&'static str>> = [
            ((1, &b"mp3/Tr01"[..]), vec!["ass", "lrc"]),
            ((1, b"mp3/Tr01.mp3"), vec!["vtt"]),
            ((1, b"mp3/Tr02.mp3"), vec!["srt"]),
            ((1, b"Tr03"), vec!["lrc"]),
            ((1, "e\u{301}/Tr05".as_bytes()), vec!["lrc"]),
        ]
        .into_iter()
        .collect();
        let formats = |loc, path: &[u8]| subtitle_formats(loc, path, &subs);
        assert_eq!(
            formats(1, b"mp3/Tr01.mp3"),
            ["vtt", "lrc", "ass"],
            "in the endpoint's order"
        );
        assert_eq!(formats(1, b"mp3/Tr02.mp3"), ["srt"]);
        assert!(formats(1, b"wav/Tr01.wav").is_empty());
        assert_eq!(formats(1, b"Tr03.wav"), ["lrc"]);
        assert!(formats(1, b"mp3/Tr04.mp3").is_empty());
        assert!(formats(2, b"mp3/Tr01.mp3").is_empty(), "another location");
        assert!(
            formats(1, "\u{e9}/Tr05.mp3".as_bytes()).is_empty(),
            "another directory that only displays the same"
        );
        assert_eq!(subtitle_format(b"a/B.ASS"), Some("ass"));
        assert_eq!(subtitle_format(b"a/b.ssa"), Some("ass"));
        assert_eq!(subtitle_format(b"a/b.txt"), None);
        assert_eq!(strip_extension(b"a/.lrc"), None);
        assert_eq!(strip_extension(b"noext"), None);
        assert_eq!(strip_extension(b"a.b/c"), None);
    }

    #[test]
    fn video_info_comes_from_the_stored_probe() {
        let probe = r#"{"format":"matroska,webm","duration_ms":1000,"audio":null,
                        "video":{"codec":"h264","width":1280,"height":720}}"#;
        assert_eq!(
            video_info(Some(probe)),
            Some(VideoInfo {
                codec: "h264".into(),
                width: Some(1280),
                height: Some(720)
            })
        );
        assert_eq!(video_info(Some(r#"{"format":"mp3","video":null}"#)), None);
        assert_eq!(video_info(Some("not json")), None);
        assert_eq!(video_info(None), None);
    }
}
