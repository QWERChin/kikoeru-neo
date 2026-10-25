//! argon2id password hashing. Hashing is CPU-bound, so async callers run
//! these functions on the blocking thread pool.

use std::sync::LazyLock;

use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};

pub const MIN_LENGTH: usize = 8;

/// Verified against when the user does not exist or has no password, so a
/// failed login takes about as long whatever the reason.
static DUMMY_HASH: LazyLock<String> =
    LazyLock::new(|| hash("kikoeru-neo dummy password").expect("hashing a constant works"));

pub fn hash(password: &str) -> Result<String, argon2::password_hash::Error> {
    Ok(Argon2::default()
        .hash_password(password.as_bytes())?
        .to_string())
}

/// Returns true only if `stored` is a valid hash of `password`. A missing
/// hash, as on accounts imported from the old database, never matches.
pub fn verify(stored: Option<&str>, password: &str) -> bool {
    let (candidate, real) = candidate(stored);
    let ok = PasswordHash::new(candidate)
        .map(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false);
    ok && real
}

/// The hash a check runs argon2 against, and whether a match may succeed.
fn candidate(stored: Option<&str>) -> (&str, bool) {
    match stored {
        Some(h) => (h, true),
        None => (DUMMY_HASH.as_str(), false),
    }
}

pub fn is_long_enough(password: &str) -> bool {
    password.chars().count() >= MIN_LENGTH
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify() {
        let h = hash("correct horse").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify(Some(&h), "correct horse"));
        assert!(!verify(Some(&h), "wrong horse"));
        assert!(!verify(None, "kikoeru-neo dummy password"));
        assert!(!verify(Some("not a hash"), "x"));
    }

    #[test]
    fn missing_hash_still_runs_argon2_with_default_cost() {
        // Unknown users and accounts without a password verify against the
        // dummy, which costs as much as a real hash, so timing does not
        // reveal whether an account exists.
        let real = hash("some password").unwrap();
        let (dummy, can_match) = candidate(None);
        assert!(!can_match);
        let params = |h: &str| {
            let parsed = PasswordHash::new(h).unwrap();
            (parsed.algorithm.to_string(), parsed.params.to_string())
        };
        assert_eq!(params(dummy), params(&real));
        assert_eq!(candidate(Some(&real)), (real.as_str(), true));
    }

    #[test]
    fn length_counts_characters() {
        assert!(!is_long_enough("1234567"));
        assert!(is_long_enough("12345678"));
        assert!(is_long_enough("八个字的中文密码"));
    }
}
