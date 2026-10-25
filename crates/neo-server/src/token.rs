//! JWTs as described in book/src/auth.md: HS256 only, `aud` separates API
//! tokens from media tokens, and `ver` ties a token to the user's
//! `token_version` so bumping it revokes every token at once.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

pub const API_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);
pub const MEDIA_TTL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience {
    Api,
    Media,
}

impl Audience {
    fn as_str(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Media => "media",
        }
    }

    fn ttl(self) -> Duration {
        match self {
            Self::Api => API_TTL,
            Self::Media => MEDIA_TTL,
        }
    }
}

/// JWT times are in seconds, unlike the millisecond times in the API.
#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,
    ver: i64,
    aud: String,
    iat: u64,
    exp: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verified {
    pub user_id: i64,
    pub token_version: i64,
}

pub struct Issued {
    pub token: String,
    /// Unix milliseconds, for the `expires_at` response field.
    pub expires_at_ms: i64,
}

pub struct Tokens {
    encoding: EncodingKey,
    decoding: DecodingKey,
}

/// A random starting `token_version` for a new user, so that a token for a
/// deleted account never matches a new account with the same ID.
pub fn initial_version() -> anyhow::Result<i64> {
    getrandom::u32()
        .map(i64::from)
        .map_err(|e| anyhow::anyhow!("no system randomness: {e}"))
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl Tokens {
    pub fn new(secret: &[u8]) -> Self {
        Self {
            encoding: EncodingKey::from_secret(secret),
            decoding: DecodingKey::from_secret(secret),
        }
    }

    pub fn issue(
        &self,
        user_id: i64,
        token_version: i64,
        aud: Audience,
        now: u64,
    ) -> jsonwebtoken::errors::Result<Issued> {
        let exp = now + aud.ttl().as_secs();
        let claims = Claims {
            sub: user_id.to_string(),
            ver: token_version,
            aud: aud.as_str().to_owned(),
            iat: now,
            exp,
        };
        let token = jsonwebtoken::encode(&Header::new(Algorithm::HS256), &claims, &self.encoding)?;
        Ok(Issued {
            token,
            expires_at_ms: i64::try_from(exp).unwrap_or(i64::MAX / 1000) * 1000,
        })
    }

    /// Checks signature, algorithm, audience and expiry. The caller still has
    /// to compare `token_version` with the database.
    pub fn verify(&self, token: &str, aud: Audience) -> Option<Verified> {
        // Only HS256 is accepted, whatever the token header claims.
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_audience(&[aud.as_str()]);
        validation.set_required_spec_claims(&["exp", "aud", "sub"]);
        validation.leeway = 0;
        let data = jsonwebtoken::decode::<Claims>(token, &self.decoding, &validation).ok()?;
        Some(Verified {
            user_id: data.claims.sub.parse().ok()?,
            token_version: data.claims.ver,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &[u8] = b"0123456789abcdef0123456789abcdef";

    #[test]
    fn round_trip_checks_audience() {
        let tokens = Tokens::new(KEY);
        let now = now_secs();
        let api = tokens.issue(7, 3, Audience::Api, now).unwrap();
        assert_eq!(
            tokens.verify(&api.token, Audience::Api),
            Some(Verified {
                user_id: 7,
                token_version: 3
            })
        );
        assert_eq!(tokens.verify(&api.token, Audience::Media), None);
        let media = tokens.issue(7, 3, Audience::Media, now).unwrap();
        assert_eq!(tokens.verify(&media.token, Audience::Api), None);
        assert!(tokens.verify(&media.token, Audience::Media).is_some());
        assert_eq!(
            api.expires_at_ms,
            i64::try_from(now + API_TTL.as_secs()).unwrap() * 1000
        );
    }

    #[test]
    fn expired_and_foreign_tokens_fail() {
        let tokens = Tokens::new(KEY);
        let old = tokens
            .issue(1, 0, Audience::Api, now_secs() - API_TTL.as_secs() - 1)
            .unwrap();
        assert_eq!(tokens.verify(&old.token, Audience::Api), None);
        let other = Tokens::new(b"another key that is 32 bytes lng");
        let foreign = other.issue(1, 0, Audience::Api, now_secs()).unwrap();
        assert_eq!(tokens.verify(&foreign.token, Audience::Api), None);
    }

    #[test]
    fn other_algorithms_are_rejected() {
        let tokens = Tokens::new(KEY);
        let claims = Claims {
            sub: "1".into(),
            ver: 0,
            aud: "api".into(),
            iat: now_secs(),
            exp: now_secs() + 60,
        };
        let hs512 = jsonwebtoken::encode(
            &Header::new(Algorithm::HS512),
            &claims,
            &EncodingKey::from_secret(KEY),
        )
        .unwrap();
        assert_eq!(tokens.verify(&hs512, Audience::Api), None);
    }
}
