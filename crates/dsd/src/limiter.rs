use std::collections::VecDeque;

const LOOK_AHEAD_SECONDS: f64 = 0.001;
const RELEASE_SECONDS: f64 = 0.05;

/// A linked stereo limiter: both channels share one gain, which is down before
/// the peak that called for it has left the delay line. Delays by
/// [`Self::delay_frames`].
pub(crate) struct Limiter {
    ceiling: f64,
    len: usize,
    delayed: Vec<[f64; 2]>,
    /// Indices into the running count of frames, with the gains they want,
    /// lowest first: the running minimum over the look-ahead.
    wanted: VecDeque<(u64, f64)>,
    held: f64,
    release: f64,
    smoothing: Vec<f64>,
    smoothed_sum: f64,
    frames: u64,
}

impl Limiter {
    pub(crate) fn new(ceiling_db: f64, sample_rate: f64) -> Self {
        let len = ((sample_rate * LOOK_AHEAD_SECONDS) as usize).max(1);
        Self {
            ceiling: 10f64.powf(ceiling_db / 20.0),
            len,
            delayed: vec![[0.0; 2]; len],
            wanted: VecDeque::with_capacity(len + 1),
            held: 1.0,
            release: (-1.0 / (sample_rate * RELEASE_SECONDS)).exp(),
            smoothing: vec![1.0; len],
            smoothed_sum: len as f64,
            frames: 0,
        }
    }

    pub(crate) fn delay_frames(&self) -> usize {
        self.len
    }

    /// Takes one frame in and gives the one from [`Self::delay_frames`] ago,
    /// limited.
    #[inline]
    pub(crate) fn step(&mut self, frame: [f64; 2]) -> [f64; 2] {
        let peak = frame[0].abs().max(frame[1].abs());
        let wanted = if peak > self.ceiling {
            self.ceiling / peak
        } else {
            1.0
        };
        while self.wanted.back().is_some_and(|back| back.1 >= wanted) {
            self.wanted.pop_back();
        }
        self.wanted.push_back((self.frames, wanted));
        while self
            .wanted
            .front()
            .is_some_and(|front| front.0 + self.len as u64 <= self.frames)
        {
            self.wanted.pop_front();
        }
        let lowest = self.wanted.front().map_or(1.0, |front| front.1);
        self.held = if lowest < self.held {
            lowest
        } else {
            lowest + (self.held - lowest) * self.release
        };
        let index = (self.frames % self.len as u64) as usize;
        self.smoothed_sum += self.held - self.smoothing[index];
        self.smoothing[index] = self.held;
        let gain = self.smoothed_sum / self.len as f64;
        let delayed = std::mem::replace(&mut self.delayed[index], frame);
        self.frames += 1;
        [delayed[0] * gain, delayed[1] * gain]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 2_822_400.0;

    fn sine(index: usize, amplitude: f64) -> f64 {
        amplitude * (std::f64::consts::TAU * 1_000.0 * index as f64 / RATE).sin()
    }

    #[test]
    fn what_stays_below_the_ceiling_only_comes_out_later() {
        let mut limiter = Limiter::new(0.0, RATE);
        let delay = limiter.delay_frames();
        for index in 0..100_000 {
            let out = limiter.step([sine(index, 0.9), -sine(index, 0.9)]);
            if index >= delay {
                assert!((out[0] - sine(index - delay, 0.9)).abs() < 1e-9);
                assert!((out[1] + sine(index - delay, 0.9)).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn peaks_do_not_get_past_the_ceiling() {
        let mut limiter = Limiter::new(1.0, RATE);
        let ceiling = 10f64.powf(1.0 / 20.0);
        for index in 0..500_000 {
            let burst = if index % 100_000 < 3_000 { 4.0 } else { 0.5 };
            let out = limiter.step([sine(index, burst), 0.0]);
            assert!(out[0].abs() <= ceiling * (1.0 + 1e-9), "{}", out[0]);
        }
    }

    #[test]
    fn a_peak_on_one_channel_lowers_both() {
        let mut limiter = Limiter::new(0.0, RATE);
        let delay = limiter.delay_frames();
        let mut last = [0.0; 2];
        for _ in 0..=delay {
            last = limiter.step([2.0, 0.5]);
        }
        assert!((last[0] - 1.0).abs() < 1e-9);
        assert!((last[1] - 0.25).abs() < 1e-9);
    }

    #[test]
    fn the_gain_recovers_after_a_peak() {
        let mut limiter = Limiter::new(0.0, RATE);
        for _ in 0..10 {
            limiter.step([4.0, 4.0]);
        }
        let mut last = [0.0; 2];
        for _ in 0..(RATE as usize) {
            last = limiter.step([0.5, 0.5]);
        }
        assert!((last[0] - 0.5).abs() < 1e-6);
    }
}
