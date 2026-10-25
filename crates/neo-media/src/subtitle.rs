//! Subtitle files to WebVTT for `/api/tracks/{id}/subtitle`.
//!
//! lrc, srt and vtt become WebVTT. ass also becomes WebVTT with only its
//! text. With `format=ass` the route serves the ass file as it is instead,
//! for JASSUB to render over video.

/// Largest subtitle file read. Real lyric and subtitle files are far below.
pub const MAX_SUBTITLE_BYTES: usize = 4 << 20;
/// Largest WebVTT produced. An lrc line can carry many timestamps and so
/// repeat its text, which must not turn a small file into a huge response.
pub const MAX_WEBVTT_BYTES: usize = 8 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleFormat {
    Lrc,
    Srt,
    Vtt,
    Ass,
}

impl SubtitleFormat {
    /// From a file extension, case-insensitively. `ssa` is read as ass.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "lrc" => Some(Self::Lrc),
            "srt" => Some(Self::Srt),
            "vtt" => Some(Self::Vtt),
            "ass" | "ssa" => Some(Self::Ass),
            _ => None,
        }
    }
}

/// Decodes subtitle bytes. A BOM wins, then valid UTF-8, then the encoding
/// chardetng guesses, which covers the Shift_JIS and GBK files common in
/// this library.
pub fn decode(bytes: &[u8]) -> String {
    if let Some((encoding, bom)) = encoding_rs::Encoding::for_bom(bytes) {
        return encoding
            .decode_without_bom_handling(&bytes[bom..])
            .0
            .into_owned();
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_owned();
    }
    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(bytes, true);
    let encoding = detector.guess(None, false);
    encoding.decode_without_bom_handling(bytes).0.into_owned()
}

/// Converts decoded subtitle text to WebVTT.
pub fn to_webvtt(format: SubtitleFormat, text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let cues = match format {
        SubtitleFormat::Lrc => lrc(&text),
        SubtitleFormat::Srt | SubtitleFormat::Vtt => srt_or_vtt(&text),
        SubtitleFormat::Ass => ass(&text),
    };
    let mut out = String::from("WEBVTT\n");
    for cue in cues {
        let block = format!(
            "\n{} --> {}\n{}\n",
            timestamp(cue.start),
            timestamp(cue.end),
            escape(&cue.text)
        );
        // Measured after escaping, which can make text five times longer.
        if out.len() + block.len() > MAX_WEBVTT_BYTES {
            break;
        }
        out.push_str(&block);
    }
    out
}

#[derive(Debug, PartialEq)]
struct Cue {
    start: u64,
    end: u64,
    /// Shared, because one lrc line with many timestamps yields many cues.
    text: std::rc::Rc<str>,
}

/// How long the last lrc line stays up, since lrc has no end times.
const LAST_LRC_LINE_MS: u64 = 5000;

fn lrc(text: &str) -> Vec<Cue> {
    let mut timed: Vec<(u64, std::rc::Rc<str>)> = Vec::new();
    let mut offset: i64 = 0;
    for line in text.lines() {
        let mut rest = line.trim();
        let mut stamps = Vec::new();
        while let Some(tag) = rest.strip_prefix('[') {
            let Some(close) = tag.find(']') else { break };
            let inner = &tag[..close];
            if let Some(ms) = lrc_time(inner) {
                stamps.push(ms);
            } else if stamps.is_empty() && is_lrc_tag(inner) {
                if let Some(value) = inner.strip_prefix("offset:") {
                    offset = value.trim().parse().unwrap_or(0);
                }
            } else {
                // Brackets after the timestamps are lyric text, such as a
                // sound cue like `[door opens]`.
                break;
            }
            rest = &tag[close + 1..];
        }
        let text: std::rc::Rc<str> = rest.trim().into();
        for stamp in stamps {
            timed.push((stamp, text.clone()));
        }
    }
    // A positive offset shows lyrics earlier.
    let shift = |ms: u64| {
        u64::try_from(i128::from(ms) - i128::from(offset))
            .unwrap_or(0)
            .min(MAX_TIME_MS)
    };
    timed.sort_by_key(|(ms, _)| *ms);
    // The next later time for each line, so an original and its
    // translation sharing a timestamp both stay up until the following line.
    // One backward pass keeps this linear when many lines share a time.
    let mut next_later = vec![None; timed.len()];
    let (mut group, mut after) = (None, None);
    for i in (0..timed.len()).rev() {
        // Times only fall walking backwards through the sorted list.
        if group != Some(timed[i].0) {
            after = group;
            group = Some(timed[i].0);
        }
        next_later[i] = after;
    }
    let mut cues = Vec::new();
    for (i, (start, line)) in timed.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let end = next_later[i].unwrap_or(start + LAST_LRC_LINE_MS);
        cues.push(Cue {
            start: shift(*start),
            end: shift(end.max(*start)),
            text: line.clone(),
        });
    }
    cues
}

/// An ID tag such as `ti:Title` or `offset:500`.
fn is_lrc_tag(inner: &str) -> bool {
    inner
        .split_once(':')
        .is_some_and(|(key, _)| !key.is_empty() && key.chars().all(|c| c.is_ascii_alphabetic()))
}

/// `mm:ss`, `mm:ss.xx` or `mm:ss.xxx`.
fn lrc_time(text: &str) -> Option<u64> {
    let (minutes, seconds) = text.split_once(':')?;
    let minutes: u64 = minutes.trim().parse().ok()?;
    let (whole, fraction) = seconds.split_once(['.', ':']).unwrap_or((seconds, ""));
    let whole: u64 = whole.trim().parse().ok()?;
    if whole >= 60 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let ms = match fraction.len() {
        0 => 0,
        1 => fraction.parse::<u64>().ok()? * 100,
        2 => fraction.parse::<u64>().ok()? * 10,
        _ => fraction[..3].parse::<u64>().ok()?,
    };
    within_limit(
        minutes
            .checked_mul(60)?
            .checked_add(whole)?
            .checked_mul(1000)?
            .checked_add(ms)?,
    )
}

/// Longest time accepted in a subtitle, far beyond any real track. Larger
/// values are rejected so later arithmetic cannot overflow.
const MAX_TIME_MS: u64 = 1000 * 3600 * 1000;

fn within_limit(ms: u64) -> Option<u64> {
    (ms <= MAX_TIME_MS).then_some(ms)
}

/// srt and vtt share the `start --> end` line followed by text lines.
/// WebVTT input escapes `&`, `<` and `>`, and srt files often carry the same
/// HTML entities, so entities are decoded first and escaped once on output.
fn srt_or_vtt(text: &str) -> Vec<Cue> {
    let mut cues = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let Some((start, end)) = line.split_once("-->") else {
            continue;
        };
        let (Some(start), Some(end)) = (
            clock(start),
            clock(end.split_whitespace().next().unwrap_or("")),
        ) else {
            continue;
        };
        let mut body = Vec::new();
        while let Some(next) = lines.peek() {
            if next.trim().is_empty() {
                break;
            }
            let line = strip_tags(lines.next().unwrap_or_default());
            body.push(unescape(&line));
        }
        if !body.is_empty() {
            cues.push(Cue {
                start,
                end: end.max(start),
                text: body.join("\n").into(),
            });
        }
    }
    cues
}

/// `hh:mm:ss,mmm`, `hh:mm:ss.mmm` or `mm:ss.mmm`.
fn clock(text: &str) -> Option<u64> {
    let text = text.trim().replace(',', ".");
    let parts: Vec<&str> = text.split(':').collect();
    let (h, m, s) = match parts.as_slice() {
        [h, m, s] => (h.parse::<u64>().ok()?, m.parse::<u64>().ok()?, *s),
        [m, s] => (0, m.parse::<u64>().ok()?, *s),
        _ => return None,
    };
    let (whole, fraction) = s.split_once('.').unwrap_or((s, "0"));
    let whole: u64 = whole.parse().ok()?;
    let fraction = format!("{fraction:0<3}");
    let ms: u64 = fraction.get(..3)?.parse().ok()?;
    let minutes = h.checked_mul(60)?.checked_add(m)?;
    within_limit(
        minutes
            .checked_mul(60)?
            .checked_add(whole)?
            .checked_mul(1000)?
            .checked_add(ms)?,
    )
}

/// Removes srt's `<i>`-style tags and WebVTT's cue tags, which the output
/// would read differently. A `<` that starts no tag, as in `1 < 2` or `<3`,
/// is text and is escaped on output like any other.
fn strip_tags(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find(['<', '>']) {
            Some(end) if after[end..].starts_with('>') && is_tag(&after[..end]) => {
                rest = &after[end + 1..];
            }
            _ => {
                out.push('<');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Whether what sits between `<` and `>` is a tag: a name, optionally
/// closing, ending the text or followed by whitespace and attributes, a `/`
/// that closes the tag itself, or a WebVTT class (`i`, `/font`,
/// `font color="red"`, `br/`, `c.yellow`, `v Speaker`). A WebVTT timestamp
/// such as `00:01.500` counts too.
fn is_tag(inner: &str) -> bool {
    if inner.starts_with(|c: char| c.is_ascii_digit()) {
        return inner.contains(':')
            && inner
                .chars()
                .all(|c| c.is_ascii_digit() || c == ':' || c == '.');
    }
    let name = inner.strip_prefix('/').unwrap_or(inner);
    let len = name
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(name.len());
    name.starts_with(|c: char| c.is_ascii_alphabetic())
        && name[len..]
            .chars()
            .next()
            .is_none_or(|c| c.is_whitespace() || c == '.' || c == '/')
}

fn ass(text: &str) -> Vec<Cue> {
    let mut fields: Vec<String> = Vec::new();
    let mut in_events = false;
    let mut cues = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_events = line.eq_ignore_ascii_case("[events]");
            continue;
        }
        if !in_events {
            continue;
        }
        if let Some(format) = line.strip_prefix("Format:") {
            fields = format
                .split(',')
                .map(|f| f.trim().to_ascii_lowercase())
                .collect();
            continue;
        }
        let Some(dialogue) = line.strip_prefix("Dialogue:") else {
            continue;
        };
        if fields.is_empty() {
            continue;
        }
        // Text is the last field and may itself contain commas.
        let values: Vec<&str> = dialogue.splitn(fields.len(), ',').map(str::trim).collect();
        let field = |name: &str| {
            fields
                .iter()
                .position(|f| f == name)
                .and_then(|i| values.get(i).copied())
        };
        let (Some(start), Some(end), Some(text)) = (field("start"), field("end"), field("text"))
        else {
            continue;
        };
        let (Some(start), Some(end)) = (clock(start), clock(end)) else {
            continue;
        };
        let text = strip_ass_overrides(text);
        if !text.trim().is_empty() {
            cues.push(Cue {
                start,
                end: end.max(start),
                text: text.into(),
            });
        }
    }
    cues.sort_by_key(|cue| cue.start);
    cues
}

/// Drops `{\...}` override blocks and turns `\N` and `\n` into line breaks.
fn strip_ass_overrides(text: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for ch in text.chars() {
        match ch {
            '{' => depth += 1,
            '}' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out.replace("\\N", "\n")
        .replace("\\n", "\n")
        .replace("\\h", " ")
}

/// Decodes the character references subtitle files use: the common named
/// ones and decimal or hex numeric ones. Anything else is left as text.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let decoded = rest
            // Only the next few bytes can hold a terminator, so a cue full
            // of bare ampersands stays linear.
            .get(
                1..rest.as_bytes()[..rest.len().min(11)]
                    .iter()
                    .position(|&b| b == b';')
                    .unwrap_or(0),
            )
            .and_then(|name| {
                let ch = match name {
                    "amp" => '&',
                    "lt" => '<',
                    "gt" => '>',
                    "quot" => '"',
                    "apos" => '\'',
                    "nbsp" => '\u{a0}',
                    "lrm" => '\u{200e}',
                    "rlm" => '\u{200f}',
                    _ => {
                        let number = name.strip_prefix('#')?;
                        let code = match number.strip_prefix(['x', 'X']) {
                            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                            None => number.parse().ok()?,
                        };
                        char::from_u32(code)?
                    }
                };
                Some((ch, name.len() + 2))
            });
        match decoded {
            Some((ch, len)) => {
                out.push(ch);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn timestamp(ms: u64) -> String {
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

/// Cue text must not contain `-->` or blank lines, and `<` and `&` start
/// markup in WebVTT.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc_lines_become_cues_ending_at_the_next_line() {
        let vtt = to_webvtt(
            SubtitleFormat::Lrc,
            "[ti:title]\n[00:01.50]first\n[00:04.00][01:00.00]second & more\n[00:05.000]\n",
        );
        assert_eq!(
            vtt,
            "WEBVTT\n\n00:00:01.500 --> 00:00:04.000\nfirst\n\n\
             00:00:04.000 --> 00:00:05.000\nsecond &amp; more\n\n\
             00:01:00.000 --> 00:01:05.000\nsecond &amp; more\n"
        );
    }

    #[test]
    fn lrc_offset_shifts_every_line() {
        let vtt = to_webvtt(SubtitleFormat::Lrc, "[offset:500]\n[00:02.00]a\n");
        assert!(vtt.contains("00:00:01.500 --> 00:00:06.500"), "{vtt}");
    }

    #[test]
    fn srt_becomes_vtt_without_tags_or_indices() {
        let vtt = to_webvtt(
            SubtitleFormat::Srt,
            "1\r\n00:00:01,000 --> 00:00:02,500\r\n<i>hello</i>\r\nworld\r\n\r\n2\r\n00:00:03,000 --> 00:00:04,000\r\n-->\r\n",
        );
        assert_eq!(
            vtt,
            "WEBVTT\n\n00:00:01.000 --> 00:00:02.500\nhello\nworld\n\n00:00:03.000 --> 00:00:04.000\n--&gt;\n"
        );
    }

    #[test]
    fn a_less_than_sign_that_starts_no_tag_is_kept() {
        let vtt = to_webvtt(
            SubtitleFormat::Srt,
            "1\n00:00:01,000 --> 00:00:02,000\n1 < 2\n<3 <i>文字</i>\na<b, c > d\n",
        );
        assert!(
            vtt.ends_with("\n1 &lt; 2\n&lt;3 文字\na&lt;b, c &gt; d\n"),
            "{vtt}"
        );
        // Attributes may follow a tab as well as a space, and a tag may
        // close itself.
        let vtt = to_webvtt(
            SubtitleFormat::Srt,
            "1\n00:00:01,000 --> 00:00:02,000\n<font\tcolor=\"red\">文字</font><br/>あ<br />\n",
        );
        assert!(vtt.ends_with("\n文字あ\n"), "{vtt}");
        // A WebVTT cue timestamp is a tag too.
        let vtt = to_webvtt(
            SubtitleFormat::Vtt,
            "WEBVTT\n\n00:01.000 --> 00:02.000\none <00:01.500>two\n",
        );
        assert!(vtt.ends_with("\none two\n"), "{vtt}");
    }

    #[test]
    fn vtt_is_normalized_and_settings_are_dropped() {
        let vtt = to_webvtt(
            SubtitleFormat::Vtt,
            "WEBVTT\n\nNOTE x\n\n00:01.000 --> 00:02.000 align:start\nline\n",
        );
        assert_eq!(vtt, "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nline\n");
    }

    #[test]
    fn ass_dialogue_keeps_only_text() {
        let ass_text = "[Script Info]\nTitle: x\n\n[Events]\n\
            Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
            Dialogue: 0,0:00:02.00,0:00:03.50,Default,,0,0,0,,{\\b1}Bold, with comma\\Nnext\n\
            Comment: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,hidden\n\
            Dialogue: 0,0:00:00.50,0:00:01.00,Default,,0,0,0,,{\\pos(1,2)}first\n";
        assert_eq!(
            to_webvtt(SubtitleFormat::Ass, ass_text),
            "WEBVTT\n\n00:00:00.500 --> 00:00:01.000\nfirst\n\n00:00:02.000 --> 00:00:03.500\nBold, with comma\nnext\n"
        );
    }

    #[test]
    fn decoding_handles_boms_utf8_and_legacy_encodings() {
        assert_eq!(decode(b"\xEF\xBB\xBFabc"), "abc");
        assert_eq!(decode("日本語".as_bytes()), "日本語");
        let (sjis, _, _) = encoding_rs::SHIFT_JIS.encode("こんにちは、世界。これは字幕です。");
        assert_eq!(decode(&sjis), "こんにちは、世界。これは字幕です。");
        assert_eq!(decode(&[0xFF, 0xFE, b'x', 0, b'y', 0]), "xy");
    }

    #[test]
    fn repeated_lrc_timestamps_cannot_blow_up_the_output() {
        let line = format!("{}{}\n", "[00:01.00]".repeat(50_000), "x".repeat(100_000));
        let vtt = to_webvtt(SubtitleFormat::Lrc, &line);
        assert!(vtt.len() <= MAX_WEBVTT_BYTES + 200, "{}", vtt.len());
    }

    #[test]
    fn lrc_lines_sharing_a_time_both_last_until_the_next_line() {
        let vtt = to_webvtt(
            SubtitleFormat::Lrc,
            "[00:01.00]原文\n[00:01.00]translation\n[00:02.00]next\n",
        );
        assert_eq!(
            vtt.matches("00:00:01.000 --> 00:00:02.000").count(),
            2,
            "{vtt}"
        );
    }

    #[test]
    fn many_lines_sharing_a_time_convert_quickly() {
        let line = format!("{}x\n", "[00:01.00]".repeat(200_000));
        let started = std::time::Instant::now();
        to_webvtt(SubtitleFormat::Lrc, &line);
        // A linear pass takes milliseconds, a quadratic one about 2e10
        // steps, which is minutes. The bound sits far from both, so a
        // heavily loaded machine does not fail it.
        assert!(
            started.elapsed() < std::time::Duration::from_secs(30),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn bracketed_lyrics_after_a_timestamp_are_kept() {
        let vtt = to_webvtt(
            SubtitleFormat::Lrc,
            "[ar:someone]\n[00:01.00][door opens]\n[00:03.00]line\n",
        );
        assert!(
            vtt.contains("00:00:01.000 --> 00:00:03.000\n[door opens]"),
            "{vtt}"
        );
    }

    #[test]
    fn named_and_numeric_references_decode() {
        assert_eq!(
            unescape("&quot;a&quot; &#39;b&#39; &#x41; &bogus; & x"),
            "\"a\" 'b' A &bogus; & x"
        );
    }

    #[test]
    fn many_bare_ampersands_decode_quickly() {
        let text = "&".repeat(1_000_000);
        let started = std::time::Instant::now();
        assert_eq!(unescape(&text).len(), 1_000_000);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn absurd_times_are_rejected_instead_of_overflowing() {
        let vtt = to_webvtt(
            SubtitleFormat::Lrc,
            "[307445734561826:00]bad\n[00:01.00]ok\n",
        );
        assert!(vtt.contains("ok") && !vtt.contains("bad"), "{vtt}");
        let vtt = to_webvtt(
            SubtitleFormat::Srt,
            "1\n99999999999999999:00:00,000 --> 1:00:00,000\nbad\n",
        );
        assert!(!vtt.contains("bad"), "{vtt}");
        let vtt = to_webvtt(
            SubtitleFormat::Lrc,
            "[offset:-9223372036854775808]\n[00:01.00]x\n",
        );
        assert!(vtt.contains('x'), "{vtt}");
    }

    #[test]
    fn srt_entities_are_not_escaped_twice() {
        let vtt = to_webvtt(
            SubtitleFormat::Srt,
            "1\n00:00:01,000 --> 00:00:02,000\nTom &amp; Jerry\n",
        );
        assert!(vtt.ends_with("\nTom &amp; Jerry\n"), "{vtt}");
    }

    #[test]
    fn vtt_entities_are_not_escaped_twice() {
        let vtt = to_webvtt(
            SubtitleFormat::Vtt,
            "WEBVTT\n\n00:01.000 --> 00:02.000\nTom &amp; Jerry &lt;3\n",
        );
        assert!(vtt.ends_with("\nTom &amp; Jerry &lt;3\n"), "{vtt}");
    }

    #[test]
    fn the_cap_counts_escaped_bytes() {
        let line = format!("[00:01.00]{}\n", "&".repeat(2_000_000));
        assert!(to_webvtt(SubtitleFormat::Lrc, &line).len() <= MAX_WEBVTT_BYTES);
    }

    #[test]
    fn extensions_map_to_formats() {
        assert_eq!(
            SubtitleFormat::from_extension("LRC"),
            Some(SubtitleFormat::Lrc)
        );
        assert_eq!(
            SubtitleFormat::from_extension("ssa"),
            Some(SubtitleFormat::Ass)
        );
        assert_eq!(SubtitleFormat::from_extension("txt"), None);
    }
}
