//! Relative paths inside a root, kept as raw bytes. File names on disk need
//! not be UTF-8 (old works use GBK or Shift_JIS), so a path is opened with
//! exactly the bytes it was listed with, never with guessed text.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("path is absolute")]
    Absolute,
    #[error("path has an empty, '.' or '..' component")]
    BadComponent,
    #[error("path contains a NUL byte")]
    Nul,
}

/// A `/`-separated path relative to a root. The empty path is the root
/// itself. Every component is a real name, so joining it onto the root can
/// never leave the root.
///
/// Backslashes are allowed: byte 0x5C is the second byte of many Shift_JIS
/// characters, such as `ソ`. The OpenList backend refuses them at its own
/// boundary, because OpenList turns them into separators.
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RawPath(Vec<u8>);

impl RawPath {
    pub fn root() -> Self {
        Self(Vec::new())
    }

    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self, PathError> {
        let bytes = bytes.into();
        if bytes.is_empty() {
            return Ok(Self(bytes));
        }
        if bytes[0] == b'/' {
            return Err(PathError::Absolute);
        }
        for part in bytes.split(|b| *b == b'/') {
            check_name(part)?;
        }
        Ok(Self(bytes))
    }

    /// Appends one file name, which must not contain `/`.
    pub fn join(&self, name: &[u8]) -> Result<Self, PathError> {
        if name.contains(&b'/') {
            return Err(PathError::BadComponent);
        }
        check_name(name)?;
        let mut bytes = self.0.clone();
        if !bytes.is_empty() {
            bytes.push(b'/');
        }
        bytes.extend_from_slice(name);
        Ok(Self(bytes))
    }

    /// Joins a relative path, such as a track path onto its work directory.
    pub fn join_path(&self, rest: &RawPath) -> Self {
        match (self.is_root(), rest.is_root()) {
            (_, true) => self.clone(),
            (true, false) => rest.clone(),
            (false, false) => {
                let mut bytes = self.0.clone();
                bytes.push(b'/');
                bytes.extend_from_slice(&rest.0);
                Self(bytes)
            }
        }
    }

    /// The part of `self` below `base`, if `self` is inside it.
    pub fn strip_prefix(&self, base: &RawPath) -> Option<Self> {
        if base.is_root() {
            return Some(self.clone());
        }
        let rest = self.0.strip_prefix(base.0.as_slice())?;
        match rest {
            [] => Some(Self::root()),
            [b'/', tail @ ..] => Some(Self(tail.to_vec())),
            _ => None,
        }
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    pub fn components(&self) -> impl Iterator<Item = &[u8]> {
        self.0.split(|b| *b == b'/').filter(|c| !c.is_empty())
    }

    pub fn file_name(&self) -> Option<&[u8]> {
        self.components().last()
    }

    pub fn parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }
        Some(match self.0.iter().rposition(|b| *b == b'/') {
            Some(i) => Self(self.0[..i].to_vec()),
            None => Self::root(),
        })
    }

    /// Lossy text for logs and error messages. Never use it to open files.
    pub fn to_string_lossy(&self) -> String {
        String::from_utf8_lossy(&self.0).into_owned()
    }
}

fn check_name(name: &[u8]) -> Result<(), PathError> {
    if name.is_empty() || name == b"." || name == b".." {
        return Err(PathError::BadComponent);
    }
    if name.contains(&0) {
        return Err(PathError::Nul);
    }
    Ok(())
}

impl fmt::Debug for RawPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RawPath({:?})", self.to_string_lossy())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> RawPath {
        RawPath::new(s).unwrap()
    }

    #[test]
    fn accepts_relative_paths_only() {
        assert!(p("").is_root());
        assert_eq!(p("a/b c/d.mp3").as_bytes(), b"a/b c/d.mp3");
        assert_eq!(
            RawPath::new(b"\xb2\xe2/x".to_vec()).unwrap().as_bytes(),
            b"\xb2\xe2/x"
        );
        // "ソ.mp3" in Shift_JIS, whose second byte is a backslash.
        assert!(RawPath::new(b"\x83\x5c.mp3".to_vec()).is_ok());
        for (bad, err) in [
            ("/a", PathError::Absolute),
            ("a/../b", PathError::BadComponent),
            ("./a", PathError::BadComponent),
            ("a//b", PathError::BadComponent),
            ("a/", PathError::BadComponent),
            ("a\0b", PathError::Nul),
        ] {
            assert_eq!(RawPath::new(bad), Err(err), "{bad:?}");
        }
    }

    fn bytes() -> impl proptest::strategy::Strategy<Value = Vec<u8>> {
        proptest::collection::vec(
            proptest::sample::select(vec![b'a', b'.', b'/', b'\\', 0, 0xff]),
            0..16,
        )
    }

    fn valid() -> impl proptest::strategy::Strategy<Value = RawPath> {
        use proptest::strategy::Strategy;
        bytes().prop_filter_map("not a valid path", |b| RawPath::new(b).ok())
    }

    proptest::proptest! {
        /// `new` accepts exactly the relative paths of real names, and an
        /// accepted path has no component that could climb out of a root.
        #[test]
        fn accepts_exactly_relative_paths_of_real_names(b in bytes()) {
            let expected = b.is_empty()
                || (b[0] != b'/'
                    && !b.contains(&0)
                    && b.split(|c| *c == b'/').all(|c| !c.is_empty() && c != b"." && c != b".."));
            let path = RawPath::new(b.clone());
            proptest::prop_assert_eq!(path.is_ok(), expected);
            if let Ok(path) = path {
                use std::os::unix::ffi::OsStrExt;
                let joined = std::path::Path::new("/root").join(std::ffi::OsStr::from_bytes(path.as_bytes()));
                proptest::prop_assert!(joined.starts_with("/root"));
                proptest::prop_assert!(joined.components().all(|c| matches!(
                    c,
                    std::path::Component::RootDir | std::path::Component::Normal(_)
                )));
                proptest::prop_assert_eq!(path.components().count(), joined.components().count() - 2);
            }
        }

        #[test]
        fn joining_and_stripping_round_trip(a in valid(), b in valid()) {
            let joined = a.join_path(&b);
            proptest::prop_assert!(RawPath::new(joined.as_bytes().to_vec()).is_ok());
            proptest::prop_assert_eq!(joined.strip_prefix(&a), Some(b));
            if let (Some(parent), Some(name)) = (joined.parent(), joined.file_name()) {
                proptest::prop_assert_eq!(parent.join(name).unwrap(), joined);
            }
        }
    }

    #[test]
    fn joins_and_splits() {
        assert_eq!(RawPath::root().join(b"a").unwrap(), p("a"));
        assert_eq!(p("a").join(b"b").unwrap(), p("a/b"));
        assert!(p("a").join(b"b/c").is_err());
        assert!(p("a").join(b"..").is_err());
        assert_eq!(p("a/b").join_path(&p("c/d")), p("a/b/c/d"));
        assert_eq!(RawPath::root().join_path(&p("c")), p("c"));
        assert_eq!(p("a/b/c").strip_prefix(&p("a")), Some(p("b/c")));
        assert_eq!(p("a/b").strip_prefix(&p("a/b")), Some(RawPath::root()));
        assert_eq!(p("ab/c").strip_prefix(&p("a")), None);
        assert_eq!(p("a/b/c").parent(), Some(p("a/b")));
        assert_eq!(p("a").parent(), Some(RawPath::root()));
        assert_eq!(RawPath::root().parent(), None);
        assert_eq!(p("a/b.mp3").file_name(), Some(&b"b.mp3"[..]));
        assert_eq!(p("a/b/c").components().count(), 3);
    }
}
