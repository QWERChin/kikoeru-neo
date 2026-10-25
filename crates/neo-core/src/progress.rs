//! Maps a `play_progress` row to the `Progress` DTO, following the listening
//! status table in book/src/data-model.md.

use crate::dto::{Id, Progress, SafeInteger};

/// The columns of one `play_progress` row joined with its current track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressRow {
    pub track_id: Option<i64>,
    pub position_ms: i64,
    pub finished_at: Option<i64>,
    pub replaying: bool,
    /// Duration of `track_id`, when the track is known and has been probed.
    pub track_duration_ms: Option<i64>,
    /// Whether `track_id` points at a track that is not marked missing.
    pub track_present: bool,
    /// The directory group the user last played, used to rebuild the queue.
    pub group_path: Option<String>,
}

impl ProgressRow {
    pub fn to_dto(self) -> Result<Progress, &'static str> {
        let finished = self.finished_at.is_some();
        let track_id = self.track_id.map(Id::try_from).transpose()?;
        Ok(Progress {
            track_id,
            position_ms: SafeInteger::try_from(self.position_ms)?,
            track_duration_ms: match track_id {
                Some(_) => self
                    .track_duration_ms
                    .map(SafeInteger::try_from)
                    .transpose()?,
                None => None,
            },
            finished,
            // `replaying` only means something after the work was finished.
            replaying: finished && self.replaying,
            // A position can only be restored on a track that still exists.
            resumable: track_id.is_some() && self.track_present,
            group_path: self.group_path,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> ProgressRow {
        ProgressRow {
            track_id: Some(345),
            position_ms: 812_000,
            finished_at: None,
            replaying: false,
            track_duration_ms: Some(1_995_000),
            track_present: true,
            group_path: None,
        }
    }

    fn flags(row: ProgressRow) -> (bool, bool, bool) {
        let p = row.to_dto().unwrap();
        (p.finished, p.replaying, p.resumable)
    }

    #[test]
    fn listening_with_located_track() {
        assert_eq!(flags(row()), (false, false, true));
    }

    #[test]
    fn imported_progress_without_track() {
        let p = ProgressRow {
            track_id: None,
            track_duration_ms: None,
            track_present: false,
            ..row()
        }
        .to_dto()
        .unwrap();
        assert_eq!(
            (p.finished, p.replaying, p.resumable),
            (false, false, false)
        );
        assert_eq!(p.track_id, None);
        assert_eq!(p.track_duration_ms, None);
    }

    #[test]
    fn finished_not_replaying() {
        let r = ProgressRow {
            finished_at: Some(1),
            ..row()
        };
        assert_eq!(flags(r), (true, false, true));
    }

    #[test]
    fn finished_and_replaying() {
        let r = ProgressRow {
            finished_at: Some(1),
            replaying: true,
            ..row()
        };
        assert_eq!(flags(r), (true, true, true));
    }

    #[test]
    fn replaying_without_finish_is_ignored() {
        let r = ProgressRow {
            replaying: true,
            ..row()
        };
        assert_eq!(flags(r), (false, false, true));
    }

    #[test]
    fn missing_track_is_not_resumable() {
        let r = ProgressRow {
            track_present: false,
            ..row()
        };
        assert_eq!(flags(r), (false, false, false));
    }

    #[test]
    fn unknown_duration_stays_null() {
        let p = ProgressRow {
            track_duration_ms: None,
            ..row()
        }
        .to_dto()
        .unwrap();
        assert_eq!(p.track_duration_ms, None);
    }
}
