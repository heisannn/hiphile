use std::time::Duration;

/// Listening that counts as a play however long the song is.
const ENOUGH: Duration = Duration::from_secs(4 * 60);
/// A position that moves further than this between two looks was seeked.
/// Far longer than the player's refresh, so a late one still counts.
const LONGEST_STEP: Duration = Duration::from_secs(2);

/// Decides when one play of a song has been listened to enough to count: half
/// of the song, or [`ENOUGH`] of a long one or one of unknown length.
///
/// Only time actually listened counts, so seeking past half of a song does
/// not count it. Start a new tally for each play.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlayTally {
    listened: Duration,
    position: Option<Duration>,
    counted: bool,
}

impl PlayTally {
    /// Takes in where the song is while it plays. `true` once per tally, when
    /// the play comes to count.
    pub fn observe(&mut self, position: Duration, duration: Option<Duration>) -> bool {
        let last = self.position.replace(position);
        if self.counted {
            return false;
        }
        if let Some(step) = last.and_then(|last| position.checked_sub(last))
            && step <= LONGEST_STEP
        {
            self.listened += step;
        }
        let needed = duration
            .filter(|duration| !duration.is_zero())
            .map_or(ENOUGH, |duration| (duration / 2).min(ENOUGH));
        self.counted = self.listened >= needed;
        self.counted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TICK: Duration = Duration::from_millis(250);

    fn secs(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    fn play(
        tally: &mut PlayTally,
        from: Duration,
        length: Duration,
        duration: Option<Duration>,
    ) -> bool {
        let mut counted = false;
        let mut position = from;
        while position <= from + length {
            counted |= tally.observe(position, duration);
            position += TICK;
        }
        counted
    }

    #[test]
    fn counts_at_half_of_the_song() {
        let duration = Some(secs(100));
        let mut tally = PlayTally::default();
        assert!(!play(&mut tally, Duration::ZERO, secs(49), duration));
        assert!(play(&mut tally, secs(49) + TICK, secs(1), duration));
    }

    #[test]
    fn a_long_song_counts_after_four_minutes() {
        let duration = Some(secs(3600));
        let mut tally = PlayTally::default();
        assert!(!play(&mut tally, Duration::ZERO, secs(239), duration));
        assert!(play(&mut tally, secs(239) + TICK, secs(1), duration));
    }

    #[test]
    fn a_song_of_unknown_length_counts_after_four_minutes() {
        let mut tally = PlayTally::default();
        assert!(!play(&mut tally, Duration::ZERO, secs(239), None));
        assert!(play(&mut tally, secs(239) + TICK, secs(1), None));
    }

    #[test]
    fn counts_once() {
        let duration = Some(secs(10));
        let mut tally = PlayTally::default();
        assert!(play(&mut tally, Duration::ZERO, secs(6), duration));
        assert!(!play(&mut tally, secs(6) + TICK, secs(4), duration));
    }

    #[test]
    fn seeking_forward_is_not_listening() {
        let duration = Some(secs(100));
        let mut tally = PlayTally::default();
        assert!(!play(&mut tally, Duration::ZERO, secs(5), duration));
        assert!(!tally.observe(secs(90), duration));
        assert!(!play(&mut tally, secs(90) + TICK, secs(5), duration));
    }

    #[test]
    fn seeking_back_keeps_what_was_listened() {
        let duration = Some(secs(100));
        let mut tally = PlayTally::default();
        assert!(!play(&mut tally, Duration::ZERO, secs(40), duration));
        assert!(!tally.observe(secs(10), duration));
        assert!(play(&mut tally, secs(10) + TICK, secs(11), duration));
    }

    #[test]
    fn staying_at_one_position_is_not_listening() {
        let duration = Some(secs(10));
        let mut tally = PlayTally::default();
        for _ in 0..100 {
            assert!(!tally.observe(secs(1), duration));
        }
    }

    #[test]
    fn starting_partway_counts_from_there() {
        let duration = Some(secs(100));
        let mut tally = PlayTally::default();
        assert!(!tally.observe(secs(80), duration));
        assert!(!play(&mut tally, secs(80), secs(19), duration));
    }

    #[test]
    fn a_zero_length_counts_like_an_unknown_one() {
        let mut tally = PlayTally::default();
        assert!(!play(
            &mut tally,
            Duration::ZERO,
            secs(10),
            Some(Duration::ZERO)
        ));
    }
}
