use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// When something came out (`ItemDate`), as far as the server knows. Orders by time, a date
/// that names only its year before one in that year that names more.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct ReleaseDate {
    year: Option<u32>,
    month: Option<u32>,
    day: Option<u32>,
}

impl ReleaseDate {
    pub fn year(year: u32) -> Self {
        Self {
            year: Some(year),
            month: None,
            day: None,
        }
    }

    /// A date without a year places nothing in time.
    pub(crate) fn has_year(&self) -> bool {
        self.year.is_some()
    }
}

/// `None` for text that is not RFC 3339.
pub(crate) fn parse_time(text: &str) -> Option<SystemTime> {
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(SystemTime::from)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::*;

    #[test]
    fn dates_order_by_time_with_a_bare_year_first_in_it() {
        let date = |value| serde_json::from_value::<ReleaseDate>(value).unwrap();
        let march = date(json!({ "year": 1959, "month": 3 }));
        let august = date(json!({ "year": 1959, "month": 8, "day": 17 }));
        assert!(ReleaseDate::year(1959) < march);
        assert!(march < august);
        assert!(august < ReleaseDate::year(1960));
    }

    #[test]
    fn times_read_whatever_their_offset_and_precision() {
        let utc = parse_time("2024-05-01T12:00:00Z").unwrap();
        let later = parse_time("2024-05-01T12:00:00.5Z").unwrap();
        let tokyo = parse_time("2024-05-01T21:00:00+09:00").unwrap();
        assert_eq!(utc, tokyo);
        assert_eq!(
            later.duration_since(utc).unwrap(),
            Duration::from_millis(500)
        );
        assert_eq!(parse_time("yesterday"), None);
    }
}
