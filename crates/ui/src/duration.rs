use std::time::Duration;

/// `m:ss`, or `h:mm:ss` from one hour.
pub fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_a_music_player() {
        assert_eq!(format_duration(Duration::ZERO), "0:00");
        assert_eq!(format_duration(Duration::from_millis(65_900)), "1:05");
        assert_eq!(format_duration(Duration::from_secs(3_599)), "59:59");
        assert_eq!(format_duration(Duration::from_secs(3_600 + 62)), "1:01:02");
    }
}
