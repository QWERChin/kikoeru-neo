//! Byte ranges for `stream` and `playable`: which part of a file to send and
//! with which status, following RFC 9110 section 14.

use std::time::{SystemTime, UNIX_EPOCH};

/// 10000-01-01T00:00:00Z as Unix seconds.
const MAX_HTTP_DATE_SECS: u64 = 253_402_300_800;

/// What to send for one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangePlan {
    /// 200 with the whole file.
    Full,
    /// 206 with bytes `start..=end`.
    Partial { start: u64, end: u64 },
    /// 416 with `Content-Range: bytes */len`.
    Unsatisfiable,
}

impl RangePlan {
    pub fn status(self) -> u16 {
        match self {
            Self::Full => 200,
            Self::Partial { .. } => 206,
            Self::Unsatisfiable => 416,
        }
    }

    /// The `Content-Range` value, absent for a full response.
    pub fn content_range(self, len: u64) -> Option<String> {
        match self {
            Self::Full => None,
            Self::Partial { start, end } => Some(format!("bytes {start}-{end}/{len}")),
            Self::Unsatisfiable => Some(format!("bytes */{len}")),
        }
    }

    /// Bytes in the response body.
    pub fn body_len(self, len: u64) -> u64 {
        match self {
            Self::Full => len,
            Self::Partial { start, end } => end - start + 1,
            Self::Unsatisfiable => 0,
        }
    }
}

/// The validators of one representation, used for `ETag`, `Last-Modified`
/// and `If-Range`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validators {
    /// A strong entity tag including its quotes, or `None` when the file has
    /// no reliable modification time and so nothing to tell versions apart.
    pub etag: Option<String>,
    pub last_modified: Option<SystemTime>,
}

impl Validators {
    pub fn new(len: u64, modified: Option<SystemTime>) -> Self {
        // Times before 1970 keep their own value with a sign, so two old
        // versions of a file never share a tag.
        let etag = modified.map(|time| match time.duration_since(UNIX_EPOCH) {
            Ok(after) => format!("\"{len:x}-{:x}\"", after.as_nanos()),
            Err(before) => format!("\"{len:x}-n{:x}\"", before.duration().as_nanos()),
        });
        Self {
            etag,
            // HTTP dates cover 1970 to 9999, and httpdate panics outside that
            // range, so such files simply get no Last-Modified.
            last_modified: modified.filter(|time| {
                time.duration_since(UNIX_EPOCH)
                    .is_ok_and(|d| d.as_secs() < MAX_HTTP_DATE_SECS)
            }),
        }
    }

    pub fn last_modified_header(&self) -> Option<String> {
        self.last_modified.map(httpdate::fmt_http_date)
    }

    /// Whether an `If-Range` value still names this representation. A weak
    /// tag never matches, and a date matches only the exact `Last-Modified`.
    fn matches(&self, if_range: &str) -> bool {
        let value = if_range.trim();
        if value.starts_with('"') {
            return self.etag.as_deref() == Some(value);
        }
        if value.starts_with("W/") {
            return false;
        }
        let Some(modified) = self.last_modified else {
            return false;
        };
        // Compared at one-second precision, which is all an HTTP date has.
        httpdate::parse_http_date(value)
            .is_ok_and(|date| httpdate::fmt_http_date(date) == httpdate::fmt_http_date(modified))
    }
}

/// Decides what to send for a file of `len` bytes.
///
/// A missing, malformed or multi-range `Range` gets the whole file, as RFC
/// 9110 allows: browsers' media elements only ever send one range, and
/// multipart responses would need a second body format. A stale `If-Range`
/// also gets the whole file.
pub fn plan(
    range: Option<&str>,
    if_range: Option<&str>,
    len: u64,
    validators: &Validators,
) -> RangePlan {
    let Some(range) = range else {
        return RangePlan::Full;
    };
    if if_range.is_some_and(|value| !validators.matches(value)) {
        return RangePlan::Full;
    }
    let Some(spec) = range.trim().strip_prefix("bytes=") else {
        return RangePlan::Full;
    };
    if spec.contains(',') {
        return RangePlan::Full;
    }
    let Some((first, last)) = spec.trim().split_once('-') else {
        return RangePlan::Full;
    };
    let (first, last) = (first.trim(), last.trim());
    if first.is_empty() {
        // Suffix range: the last `n` bytes.
        let Some(n) = parse_number(last) else {
            return RangePlan::Full;
        };
        if n == 0 || len == 0 {
            return RangePlan::Unsatisfiable;
        }
        return RangePlan::Partial {
            start: len.saturating_sub(n),
            end: len - 1,
        };
    }
    let Some(start) = parse_number(first) else {
        return RangePlan::Full;
    };
    let end = if last.is_empty() {
        None
    } else {
        match parse_number(last) {
            Some(end) if end >= start => Some(end),
            _ => return RangePlan::Full,
        }
    };
    if start >= len {
        return RangePlan::Unsatisfiable;
    }
    RangePlan::Partial {
        start,
        end: end.map_or(len - 1, |end| end.min(len - 1)),
    }
}

fn parse_number(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn validators() -> Validators {
        Validators::new(1000, Some(UNIX_EPOCH + Duration::from_secs(1_700_000_000)))
    }

    fn p(range: &str) -> RangePlan {
        plan(Some(range), None, 1000, &validators())
    }

    #[test]
    fn ordinary_ranges() {
        assert_eq!(p("bytes=0-99"), RangePlan::Partial { start: 0, end: 99 });
        assert_eq!(
            p("bytes=500-"),
            RangePlan::Partial {
                start: 500,
                end: 999
            }
        );
        assert_eq!(
            p("bytes=900-5000"),
            RangePlan::Partial {
                start: 900,
                end: 999
            }
        );
        assert_eq!(
            p(" bytes= 10 - 20 "),
            RangePlan::Partial { start: 10, end: 20 }
        );
    }

    #[test]
    fn suffix_ranges() {
        assert_eq!(
            p("bytes=-500"),
            RangePlan::Partial {
                start: 500,
                end: 999
            }
        );
        assert_eq!(p("bytes=-5000"), RangePlan::Partial { start: 0, end: 999 });
        assert_eq!(p("bytes=-0"), RangePlan::Unsatisfiable);
    }

    #[test]
    fn out_of_range_is_416() {
        assert_eq!(p("bytes=1000-"), RangePlan::Unsatisfiable);
        assert_eq!(p("bytes=1000-1001"), RangePlan::Unsatisfiable);
        assert_eq!(
            RangePlan::Unsatisfiable.content_range(1000).as_deref(),
            Some("bytes */1000")
        );
    }

    #[test]
    fn malformed_and_multi_ranges_get_the_whole_file() {
        for range in [
            "bytes=0-1,5-6",
            "items=0-1",
            "bytes=5-1",
            "bytes=a-",
            "bytes=",
            "bytes=-",
            "bytes=+1-2",
        ] {
            assert_eq!(p(range), RangePlan::Full, "{range}");
        }
        assert_eq!(plan(None, None, 1000, &validators()), RangePlan::Full);
    }

    #[test]
    fn empty_files_cannot_satisfy_a_range() {
        let v = Validators::new(0, None);
        assert_eq!(
            plan(Some("bytes=0-"), None, 0, &v),
            RangePlan::Unsatisfiable
        );
        assert_eq!(
            plan(Some("bytes=-1"), None, 0, &v),
            RangePlan::Unsatisfiable
        );
        assert_eq!(plan(None, None, 0, &v), RangePlan::Full);
    }

    #[test]
    fn if_range_with_the_current_tag_or_date_keeps_the_range() {
        let v = validators();
        let tag = v.etag.clone().unwrap();
        let date = v.last_modified_header().unwrap();
        let partial = RangePlan::Partial { start: 0, end: 9 };
        assert_eq!(plan(Some("bytes=0-9"), Some(&tag), 1000, &v), partial);
        assert_eq!(plan(Some("bytes=0-9"), Some(&date), 1000, &v), partial);
    }

    #[test]
    fn a_stale_or_weak_if_range_gets_the_whole_file() {
        let v = validators();
        for stale in [
            "\"other\"",
            "W/\"3e8-0\"",
            "Mon, 01 Jan 2001 00:00:00 GMT",
            "garbage",
        ] {
            assert_eq!(
                plan(Some("bytes=0-9"), Some(stale), 1000, &v),
                RangePlan::Full,
                "{stale}"
            );
        }
        let weak = format!("W/{}", v.etag.clone().unwrap());
        assert_eq!(
            plan(Some("bytes=0-9"), Some(&weak), 1000, &v),
            RangePlan::Full
        );
    }

    #[test]
    fn without_a_modification_time_there_is_no_tag_and_if_range_never_matches() {
        let v = Validators::new(1000, None);
        assert_eq!(v.etag, None);
        assert_eq!(
            plan(Some("bytes=0-9"), Some("\"x\""), 1000, &v),
            RangePlan::Full
        );
    }

    #[test]
    fn times_outside_http_dates_get_no_last_modified() {
        for time in [
            UNIX_EPOCH - Duration::from_secs(1),
            UNIX_EPOCH + Duration::from_secs(MAX_HTTP_DATE_SECS),
        ] {
            let v = Validators::new(10, Some(time));
            assert_eq!(v.last_modified_header(), None);
            assert!(v.etag.is_some());
            assert_eq!(
                plan(
                    Some("bytes=0-1"),
                    Some("Thu, 01 Jan 1970 00:00:00 GMT"),
                    10,
                    &v
                ),
                RangePlan::Full
            );
        }
    }

    #[test]
    fn pre_epoch_times_get_distinct_tags() {
        let a = Validators::new(10, Some(UNIX_EPOCH - Duration::from_secs(1)));
        let b = Validators::new(10, Some(UNIX_EPOCH - Duration::from_secs(2)));
        let epoch = Validators::new(10, Some(UNIX_EPOCH));
        assert_ne!(a.etag, b.etag);
        assert_ne!(a.etag, epoch.etag);
    }

    #[test]
    fn tags_change_with_size_or_time() {
        let a = Validators::new(1000, Some(UNIX_EPOCH + Duration::from_secs(1)));
        let b = Validators::new(1001, Some(UNIX_EPOCH + Duration::from_secs(1)));
        let c = Validators::new(1000, Some(UNIX_EPOCH + Duration::from_secs(2)));
        assert_ne!(a.etag, b.etag);
        assert_ne!(a.etag, c.etag);
    }

    #[test]
    fn body_lengths() {
        assert_eq!(RangePlan::Full.body_len(1000), 1000);
        assert_eq!(RangePlan::Partial { start: 10, end: 19 }.body_len(1000), 10);
        assert_eq!(RangePlan::Unsatisfiable.body_len(1000), 0);
    }
}
