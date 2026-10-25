//! What the scanner derives from a raw path: the work code in a directory
//! name, display text, a natural sort key and the track kind.

use unicode_normalization::UnicodeNormalization;

const CODE_PREFIXES: [&str; 3] = ["RJ", "BJ", "VJ"];

/// The first `RJ`, `BJ` or `VJ` code with 6 to 8 digits in a directory
/// name, normalized the way DLsite writes it: numbers from 1000000 on have
/// 8 digits, older ones 6. So `RJ0123456` and `rj123456` are both
/// `RJ123456`, the code the import derives from the old work ID.
pub fn work_code(name: &str) -> Option<String> {
    work_codes(name).next()
}

/// Every code in `name`, in order and normalized like [`work_code`].
pub fn work_codes(name: &str) -> impl Iterator<Item = String> + '_ {
    let bytes = name.as_bytes();
    (0..bytes.len().saturating_sub(2)).filter_map(move |i| {
        let prefix = name
            .get(i..i + 2)
            .map(str::to_ascii_uppercase)
            .filter(|p| CODE_PREFIXES.contains(&p.as_str()))?;
        // A letter before the prefix makes it part of a longer word.
        if i > 0 && bytes[i - 1].is_ascii_alphabetic() {
            return None;
        }
        let digits = bytes[i + 2..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        if !(6..=8).contains(&digits) {
            return None;
        }
        let n = name[i + 2..i + 2 + digits]
            .parse::<u64>()
            .ok()
            .filter(|n| *n > 0)?;
        let width = if n >= 1_000_000 { 8 } else { 6 };
        Some(format!("{prefix}{n:0width$}"))
    })
}

/// Text for showing and searching a raw path. Valid UTF-8 is kept as is.
/// Anything else is decoded with a guessed legacy encoding (old works use
/// GBK or Shift_JIS), which only affects display: files are always opened
/// by their raw bytes. NFC makes visually equal names search equally.
///
/// Each `/`-separated component is decoded on its own: a GBK file name in
/// a UTF-8 directory must not drag the directory name into GBK too, or the
/// file would show up under a different directory than its siblings.
pub fn display_text(raw: &[u8]) -> String {
    raw.split(|b| *b == b'/')
        .map(decode_component)
        .collect::<Vec<_>>()
        .join("/")
        .nfc()
        .collect()
}

fn decode_component(raw: &[u8]) -> String {
    match std::str::from_utf8(raw) {
        Ok(s) => s.to_owned(),
        Err(_) => {
            let mut detector = chardetng::EncodingDetector::new();
            detector.feed(raw, true);
            let encoding = detector.guess(None, false);
            encoding.decode_without_bom_handling(raw).0.into_owned()
        }
    }
}

/// A key that sorts `Tr2` before `Tr10` under plain byte comparison, so
/// SQLite can order tracks without a custom collation. Each run of digits
/// becomes its length (two digits) followed by the digits without leading
/// zeros, and letters are lowercased. Files in one directory share its
/// prefix, so they stay together.
pub fn sort_key(display: &str) -> String {
    let mut key = String::with_capacity(display.len() + 8);
    let mut chars = display.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            let mut digits = String::from(c);
            while let Some(d) = chars.peek().copied().filter(char::is_ascii_digit) {
                digits.push(d);
                chars.next();
            }
            let trimmed = digits.trim_start_matches('0');
            let trimmed = if trimmed.is_empty() { "0" } else { trimmed };
            // Runs longer than 99 digits are not track numbers; cap the
            // length field so the key stays well formed.
            key.push_str(&format!("{:02}", trimmed.len().min(99)));
            key.push_str(trimmed);
        } else {
            key.extend(c.to_lowercase());
        }
    }
    key
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Audio,
    Video,
    Subtitle,
    Image,
    Text,
    Other,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Video => "video",
            Self::Subtitle => "subtitle",
            Self::Image => "image",
            Self::Text => "text",
            Self::Other => "other",
        }
    }

    /// Decided by extension only. ffprobe confirms the codec later.
    pub fn of(name: &str) -> Self {
        let ext = name
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "mp3" | "wav" | "flac" | "m4a" | "aac" | "ogg" | "oga" | "opus" | "wma" | "ape"
            | "aif" | "aiff" | "alac" | "dsf" | "tta" | "wv" => Self::Audio,
            "mp4" | "m4v" | "mkv" | "webm" | "avi" | "mov" | "wmv" | "flv" | "ts" | "mpg"
            | "mpeg" => Self::Video,
            "lrc" | "srt" | "vtt" | "ass" | "ssa" => Self::Subtitle,
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" => Self::Image,
            "txt" | "md" | "html" | "htm" | "pdf" => Self::Text,
            _ => Self::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_in_directory_names() {
        assert_eq!(work_code("RJ123456").as_deref(), Some("RJ123456"));
        assert_eq!(
            work_code("[circle] rj01234567 title").as_deref(),
            Some("RJ01234567")
        );
        assert_eq!(work_code("RJ0123456-folder").as_deref(), Some("RJ123456"));
        assert_eq!(work_code("VJ01246802").as_deref(), Some("VJ01246802"));
        assert_eq!(work_code("BJ000123").as_deref(), Some("BJ000123"));
        assert_eq!(work_code("音声RJ"), None);
        assert_eq!(work_code("RJ12345"), None, "too few digits");
        assert_eq!(work_code("RJ123456789"), None, "too many digits");
        assert_eq!(work_code("CRJ123456"), None, "part of a longer word");
        let codes: Vec<String> = work_codes("RJ0123456_img_main rj01000001 xVJ123456").collect();
        assert_eq!(codes, ["RJ123456", "RJ01000001"]);
        assert_eq!(work_code("RJ000000"), None);
        assert_eq!(
            work_code("x RJ12 then BJ654321").as_deref(),
            Some("BJ654321")
        );
    }

    #[test]
    fn display_text_decodes_legacy_encodings() {
        assert_eq!(display_text("mp3/01.mp3".as_bytes()), "mp3/01.mp3");
        // "测试" in GBK.
        assert_eq!(display_text(b"\xb2\xe2\xca\xd4.mp3"), "测试.mp3");
        // A GBK name inside a UTF-8 directory keeps the directory intact.
        let mut mixed = "音声/".as_bytes().to_vec();
        mixed.extend_from_slice(b"\xb2\xe2\xca\xd4.mp3");
        assert_eq!(display_text(&mixed), "音声/测试.mp3");
        // NFD "が" becomes NFC.
        assert_eq!(display_text("か\u{3099}".as_bytes()), "が");
    }

    #[test]
    fn natural_order() {
        let mut names = [
            "Tr10.mp3",
            "tr2.mp3",
            "Tr1.mp3",
            "Tr02b.mp3",
            "a/x.mp3",
            "a.mp3",
        ];
        names.sort_by_key(|n| sort_key(n));
        assert_eq!(
            names,
            [
                "a.mp3",
                "a/x.mp3",
                "Tr1.mp3",
                "tr2.mp3",
                "Tr02b.mp3",
                "Tr10.mp3"
            ]
        );
        assert_eq!(sort_key("x007"), sort_key("X7"));
    }

    #[test]
    fn kinds() {
        assert_eq!(Kind::of("a/01.MP3"), Kind::Audio);
        assert_eq!(Kind::of("v.mp4"), Kind::Video);
        assert_eq!(Kind::of("01.lrc"), Kind::Subtitle);
        assert_eq!(Kind::of("cover.JPG"), Kind::Image);
        assert_eq!(Kind::of("readme.txt"), Kind::Text);
        assert_eq!(Kind::of("noext"), Kind::Other);
        assert_eq!(Kind::of("font.ttf"), Kind::Other);
    }
}
