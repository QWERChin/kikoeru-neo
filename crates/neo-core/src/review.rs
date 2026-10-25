//! Rules for a user's rating and short review (book/src/contract.md,
//! "评分和短评"), shared by the API and the legacy import.

use crate::dto::ReviewInput;

pub const MAX_TEXT_CHARS: usize = 2000;
const RATINGS: std::ops::RangeInclusive<u8> = 1..=5;

/// A rating and text that may be stored: the rating is 1 to 5, the text is
/// trimmed and not empty, and at least one of them is present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Valid {
    pub rating: Option<u8>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputError {
    Rating,
    TooLong,
    Empty,
}

/// Checks a `PUT /api/me/reviews/{work_id}` body.
///
/// # Errors
/// The first rule the input breaks.
pub fn validate(input: &ReviewInput) -> Result<Valid, InputError> {
    if input.rating.is_some_and(|r| !RATINGS.contains(&r)) {
        return Err(InputError::Rating);
    }
    let text = clean(input.text.as_deref());
    if text
        .as_ref()
        .is_some_and(|t| t.chars().count() > MAX_TEXT_CHARS)
    {
        return Err(InputError::TooLong);
    }
    if input.rating.is_none() && text.is_none() {
        return Err(InputError::Empty);
    }
    Ok(Valid {
        rating: input.rating,
        text,
    })
}

/// What an old `t_review` row contributes to `reviews`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FromLegacy {
    /// `None` when the row has neither a usable rating nor text.
    pub review: Option<Valid>,
    /// The row had a rating outside 1 to 5, which was dropped.
    pub dropped_rating: bool,
    /// The text was cut to [`MAX_TEXT_CHARS`].
    pub truncated: bool,
}

/// Old kikoeru only checked that a rating was an integer, so out-of-range
/// values are dropped and long texts cut rather than refusing the import.
/// The untouched row stays in `legacy_reviews`.
pub fn from_legacy(rating: Option<i64>, text: Option<&str>) -> FromLegacy {
    let kept_rating = rating
        .and_then(|r| u8::try_from(r).ok())
        .filter(|r| RATINGS.contains(r));
    let mut text = clean(text);
    let truncated = text
        .as_ref()
        .is_some_and(|t| t.chars().count() > MAX_TEXT_CHARS);
    if truncated {
        text = text.map(|t| t.chars().take(MAX_TEXT_CHARS).collect());
    }
    let review = (kept_rating.is_some() || text.is_some()).then_some(Valid {
        rating: kept_rating,
        text,
    });
    FromLegacy {
        review,
        dropped_rating: rating.is_some() && kept_rating.is_none(),
        truncated,
    }
}

fn clean(text: Option<&str>) -> Option<String> {
    text.map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(rating: Option<u8>, text: Option<&str>) -> ReviewInput {
        ReviewInput {
            rating,
            text: text.map(str::to_owned),
        }
    }

    #[test]
    fn accepts_a_rating_a_text_or_both() {
        assert_eq!(
            validate(&input(Some(5), None)),
            Ok(Valid {
                rating: Some(5),
                text: None
            })
        );
        assert_eq!(
            validate(&input(None, Some("  好听\n"))),
            Ok(Valid {
                rating: None,
                text: Some("好听".to_owned())
            })
        );
        assert!(validate(&input(Some(1), Some("ok"))).is_ok());
    }

    #[test]
    fn rejects_out_of_range_ratings() {
        assert_eq!(validate(&input(Some(0), None)), Err(InputError::Rating));
        assert_eq!(validate(&input(Some(6), None)), Err(InputError::Rating));
    }

    #[test]
    fn blank_text_counts_as_no_text() {
        assert_eq!(
            validate(&input(None, Some(" \u{3000} "))),
            Err(InputError::Empty)
        );
        assert_eq!(validate(&input(None, None)), Err(InputError::Empty));
    }

    #[test]
    fn the_limit_counts_characters_after_trimming() {
        let at_limit = format!(" {} ", "音".repeat(MAX_TEXT_CHARS));
        assert!(validate(&input(None, Some(&at_limit))).is_ok());
        let over = "音".repeat(MAX_TEXT_CHARS + 1);
        assert_eq!(
            validate(&input(None, Some(&over))),
            Err(InputError::TooLong)
        );
    }

    #[test]
    fn legacy_rows_keep_what_is_valid() {
        let both = from_legacy(Some(4), Some(" 好 "));
        assert_eq!(
            both.review,
            Some(Valid {
                rating: Some(4),
                text: Some("好".to_owned())
            })
        );
        assert!(!both.dropped_rating && !both.truncated);

        let bad_rating = from_legacy(Some(10), Some("text"));
        assert_eq!(bad_rating.review.unwrap().rating, None);
        assert!(bad_rating.dropped_rating);

        let nothing = from_legacy(Some(-1), Some("   "));
        assert_eq!(nothing.review, None);
        assert!(nothing.dropped_rating);

        assert_eq!(from_legacy(None, None).review, None);
    }

    #[test]
    fn legacy_text_is_cut_to_the_limit() {
        let long = "a".repeat(MAX_TEXT_CHARS + 5);
        let cut = from_legacy(None, Some(&long));
        assert!(cut.truncated);
        assert_eq!(
            cut.review.unwrap().text.unwrap().chars().count(),
            MAX_TEXT_CHARS
        );
    }
}
