//! Release dates (`works.release_date`). The metadata fetch, the legacy
//! import and the metadata edit all check them here, so a date one of them
//! stores is never one another would refuse.

/// Whether `s` is a `YYYY-MM-DD` date that exists in the Gregorian
/// calendar: the month is 1 to 12 and the day fits the month, with
/// February 29 only in leap years.
pub fn is_calendar_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| {
        b[r].iter().try_fold(0u32, |n, c| {
            c.is_ascii_digit().then(|| n * 10 + u32::from(c - b'0'))
        })
    };
    let (Some(year), Some(month), Some(day)) = (digits(0..4), digits(5..7), digits(8..10)) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_dates_that_exist() {
        for date in [
            "2024-05-01",
            "2024-01-31",
            "2024-12-31",
            "2024-02-29",
            "2000-02-29",
        ] {
            assert!(is_calendar_date(date), "{date}");
        }
    }

    #[test]
    fn rejects_days_the_month_does_not_have() {
        for date in [
            "2023-02-29",
            "2024-02-30",
            "1900-02-29",
            "2024-04-31",
            "2024-01-32",
            "2024-01-00",
            "2024-00-10",
            "2024-13-01",
        ] {
            assert!(!is_calendar_date(date), "{date}");
        }
    }

    #[test]
    fn rejects_other_shapes() {
        for text in [
            "",
            "2024-5-1",
            "2024/05/01",
            "2024-05-01 16:00:00",
            "24-05-01",
            "2024-05-0a",
            "２０２４-05-01",
        ] {
            assert!(!is_calendar_date(text), "{text:?}");
        }
    }
}
