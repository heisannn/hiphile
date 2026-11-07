use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use crate::mix::rising_gain;

/// How long the sound falls when paused or stopped, and rises when played
const LENGTH: Duration = Duration::from_millis(250);

/// The level of the one stream that may be sounding on PCM. The audio callback
/// moves it, a frame at a time, towards where the engine thread points it.
///
/// A stream must not be sounding while [`Self::start`] or [`Self::silence`]
/// run, as the callback would write over what they set.
pub(crate) struct PlayFade {
    /// Frames from silence to full level, zero until a stream starts
    frames: AtomicU64,
    level: AtomicU64,
    falling: AtomicBool,
}

impl PlayFade {
    pub(crate) fn new() -> Self {
        Self {
            frames: AtomicU64::new(0),
            level: AtomicU64::new(0),
            falling: AtomicBool::new(false),
        }
    }

    /// At full level, for a new stream at `rate`
    pub(crate) fn start(&self, rate: u32) {
        let frames = ((LENGTH.as_secs_f64() * f64::from(rate)) as u64).max(1);
        self.frames.store(frames, Ordering::Relaxed);
        self.level.store(frames, Ordering::Relaxed);
        self.falling.store(false, Ordering::Relaxed);
    }

    pub(crate) fn fall(&self) {
        self.falling.store(true, Ordering::Relaxed);
    }

    pub(crate) fn rise(&self) {
        self.falling.store(false, Ordering::Relaxed);
    }

    pub(crate) fn silence(&self) {
        self.level.store(0, Ordering::Relaxed);
        self.falling.store(false, Ordering::Relaxed);
    }

    /// Goes to full level without a fade. Safe on a stream that is sounding: it
    /// can only be rising from then on.
    pub(crate) fn skip_to_full(&self) {
        self.falling.store(false, Ordering::Relaxed);
        self.level
            .store(self.frames.load(Ordering::Relaxed), Ordering::Relaxed);
    }

    /// Whether it has fallen all the way, so that what is left of the track
    /// should stay in the ring buffer to be heard later.
    pub(crate) fn has_fallen(&self) -> bool {
        self.falling.load(Ordering::Relaxed) && self.level.load(Ordering::Relaxed) == 0
    }

    /// Whether there is sound left to fall from
    pub(crate) fn is_falling(&self) -> bool {
        self.falling.load(Ordering::Relaxed) && self.level.load(Ordering::Relaxed) > 0
    }

    /// Applies the level to whole frames, moving it along. Called by the audio
    /// callback, so it does not allocate or lock.
    pub(crate) fn apply(&self, samples: &mut [f32], channels: usize) {
        let frames = self.frames.load(Ordering::Relaxed);
        let falling = self.falling.load(Ordering::Relaxed);
        let mut level = self.level.load(Ordering::Relaxed);
        if !falling && level >= frames {
            return;
        }
        for frame in samples.chunks_exact_mut(channels) {
            level = if falling {
                level.saturating_sub(1)
            } else {
                (level + 1).min(frames)
            };
            let gain = rising_gain(level, frames);
            for sample in frame {
                *sample *= gain;
            }
        }
        self.level.store(level, Ordering::Relaxed);
    }

    /// The device has run out of samples, so a fall has nothing left to fall
    /// from, while a rise waits for the sound to come back.
    pub(crate) fn ran_out(&self) {
        if self.falling.load(Ordering::Relaxed) {
            self.level.store(0, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame per millisecond, so 250 frames long
    fn fade() -> PlayFade {
        let fade = PlayFade::new();
        fade.start(1_000);
        fade
    }

    fn applied(fade: &PlayFade, frames: usize) -> Vec<f32> {
        let mut samples = vec![1.0; frames * 2];
        fade.apply(&mut samples, 2);
        samples
    }

    #[test]
    fn full_level_leaves_samples_as_they_are() {
        assert_eq!(applied(&fade(), 300), vec![1.0; 600]);
        assert_eq!(applied(&PlayFade::new(), 4), vec![1.0; 8]);
    }

    #[test]
    fn a_fall_lowers_each_frame_as_a_whole_down_to_nothing() {
        let fade = fade();
        fade.fall();
        let samples = applied(&fade, 250);
        for (frame, next) in samples.chunks(2).zip(samples.chunks(2).skip(1)) {
            assert_eq!(frame[0], frame[1]);
            assert!(next[0] <= frame[0]);
        }
        assert_eq!(samples[498..], [0.0, 0.0]);
        assert!(fade.has_fallen());
    }

    #[test]
    fn a_rise_comes_up_from_silence_to_the_samples_as_they_are() {
        let fade = fade();
        fade.silence();
        assert!(!fade.has_fallen());
        let samples = applied(&fade, 250);
        assert!(samples[0] < 0.01);
        assert!(samples.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(samples[498..], [1.0, 1.0]);
        assert_eq!(applied(&fade, 1), [1.0, 1.0]);
    }

    #[test]
    fn turning_back_goes_on_from_the_level_it_is_at() {
        let fade = fade();
        fade.fall();
        let last_of_the_fall = *applied(&fade, 100).last().unwrap();
        fade.rise();
        let first_of_the_rise = applied(&fade, 1)[0];
        assert!(first_of_the_rise > last_of_the_fall);
        assert!(first_of_the_rise - last_of_the_fall < 0.02);
    }

    #[test]
    fn running_out_ends_a_fall_but_not_a_rise() {
        let falling = fade();
        falling.fall();
        falling.ran_out();
        assert!(falling.has_fallen());

        let rising = fade();
        rising.silence();
        rising.ran_out();
        assert!(applied(&rising, 1)[0] < 0.01);
    }

    #[test]
    fn it_can_skip_to_full_level() {
        let fade = fade();
        fade.silence();
        fade.skip_to_full();
        assert_eq!(applied(&fade, 1), [1.0, 1.0]);
    }
}
