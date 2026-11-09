use std::f64::consts::PI;

use crate::Crossfeed;

/// Below this a state is taken as silence, as numbers decaying into the
/// subnormal range are slow on x86.
const SILENT: f64 = 1e-20;

/// What bs2b's level correction leaves can still add up to 2.7 dB over full
/// scale, and loud masters go over often enough to clip on most of them.
const HEADROOM_DB: f64 = -3.0;

/// bs2b's filters for a stereo pair: a lowpass from each channel into the
/// other, and a high shelf on each channel's own path. Its level correction and
/// [`HEADROOM_DB`] are folded into their coefficients.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Filter {
    crossfeed: Crossfeed,
    lowpass: Lowpass,
    shelf: Shelf,
    state: [Channel; 2],
}

#[derive(Clone, Copy, Debug)]
struct Lowpass {
    a0: f64,
    b1: f64,
}

#[derive(Clone, Copy, Debug)]
struct Shelf {
    a0: f64,
    a1: f64,
    b1: f64,
}

#[derive(Clone, Copy, Debug, Default)]
struct Channel {
    lowpassed: f64,
    shelved: f64,
    last: f64,
}

impl Filter {
    pub(crate) fn new(crossfeed: Crossfeed, rate: u32) -> Self {
        let separation = f64::from(crossfeed.separation_db());
        let lowpass_db = separation * -5.0 / 6.0 - 3.0;
        let shelf_db = separation / 6.0 - 3.0;
        let lowpass_gain = 10f64.powf(lowpass_db / 20.0);
        let shelf_gain = 1.0 - 10f64.powf(shelf_db / 20.0);
        let cutoff = f64::from(crossfeed.cutoff_hz());
        let shelf_cutoff = cutoff * 2f64.powf((lowpass_db - 20.0 * shelf_gain.log10()) / 12.0);
        let gain = 10f64.powf(HEADROOM_DB / 20.0) / (1.0 - shelf_gain + lowpass_gain);
        let rate = f64::from(rate.max(1));
        let pole = (-2.0 * PI * cutoff / rate).exp();
        let shelf_pole = (-2.0 * PI * shelf_cutoff / rate).exp();
        Self {
            crossfeed,
            lowpass: Lowpass {
                a0: gain * lowpass_gain * (1.0 - pole),
                b1: pole,
            },
            shelf: Shelf {
                a0: gain * (1.0 - shelf_gain * (1.0 - shelf_pole)),
                a1: -gain * shelf_pole,
                b1: shelf_pole,
            },
            state: Default::default(),
        }
    }

    pub(crate) fn crossfeed(&self) -> Crossfeed {
        self.crossfeed
    }

    /// Goes on from this one's state, for another crossfeed or rate.
    pub(crate) fn succeeded_by(&self, crossfeed: Crossfeed, rate: u32) -> Self {
        Self {
            state: self.state,
            ..Self::new(crossfeed, rate)
        }
    }

    pub(crate) fn run(&mut self, input: [f64; 2]) -> [f64; 2] {
        for (channel, sample) in self.state.iter_mut().zip(input) {
            channel.lowpassed = self.lowpass.a0 * sample + self.lowpass.b1 * channel.lowpassed;
            channel.shelved = self.shelf.a0 * sample
                + self.shelf.a1 * channel.last
                + self.shelf.b1 * channel.shelved;
            channel.last = sample;
        }
        let [left, right] = &self.state;
        [
            left.shelved + right.lowpassed,
            right.shelved + left.lowpassed,
        ]
    }

    pub(crate) fn settle(&mut self) {
        for channel in &mut self.state {
            for value in [
                &mut channel.lowpassed,
                &mut channel.shelved,
                &mut channel.last,
            ] {
                if value.abs() < SILENT {
                    *value = 0.0;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATES: [u32; 6] = [44_100, 48_000, 88_200, 96_000, 192_000, 384_000];

    struct Preset {
        crossfeed: Crossfeed,
        center_treble_db: f64,
        fed_at_10_khz_db: f64,
    }

    fn presets() -> [Preset; 3] {
        [
            Preset {
                crossfeed: Crossfeed::new(700, 4.5).unwrap(),
                center_treble_db: -4.8,
                fed_at_10_khz_db: -34.0,
            },
            Preset {
                crossfeed: Crossfeed::new(700, 6.0).unwrap(),
                center_treble_db: -4.5,
                fed_at_10_khz_db: -35.0,
            },
            Preset {
                crossfeed: Crossfeed::new(650, 9.5).unwrap(),
                center_treble_db: -4.1,
                fed_at_10_khz_db: -38.0,
            },
        ]
    }

    /// The level of each output channel against a full-scale sine, over one
    /// second after half a second to settle. The left channel gets a
    /// full-scale sine and the right one `right` of it.
    fn levels(crossfeed: Crossfeed, rate: u32, frequency: f64, right: f64) -> [f64; 2] {
        let mut filter = Filter::new(crossfeed, rate);
        let settling = rate as usize / 2;
        let mut sums = [0.0; 2];
        for n in 0..settling + rate as usize {
            let sample = (2.0 * PI * frequency * n as f64 / f64::from(rate)).sin();
            let out = filter.run([sample, right * sample]);
            if n >= settling {
                sums[0] += out[0] * out[0];
                sums[1] += out[1] * out[1];
            }
        }
        sums.map(|sum| 10.0 * (sum / f64::from(rate) / 0.5).log10())
    }

    #[test]
    fn the_bass_fed_across_is_lower_by_the_separation() {
        for preset in presets() {
            for rate in RATES {
                let [direct, fed] = levels(preset.crossfeed, rate, 25.0, 0.0);
                let separation = direct - fed;
                let expected = f64::from(preset.crossfeed.separation_db());
                assert!(
                    (separation - expected).abs() < 0.05,
                    "{preset:?} at {rate}: {separation}",
                    preset = preset.crossfeed
                );
            }
        }
    }

    #[test]
    fn center_bass_drops_by_the_headroom_and_center_treble_further() {
        for preset in presets() {
            for rate in RATES {
                let [bass, _] = levels(preset.crossfeed, rate, 25.0, 1.0);
                assert!((bass - HEADROOM_DB).abs() < 0.05, "{rate}: {bass}");
                let [treble, _] = levels(preset.crossfeed, rate, 10_000.0, 1.0);
                assert!(
                    (treble - preset.center_treble_db).abs() < 0.1,
                    "{rate}: {treble}"
                );
            }
        }
    }

    #[test]
    fn little_treble_is_fed_across() {
        for preset in presets() {
            for rate in RATES {
                let [_, fed] = levels(preset.crossfeed, rate, 10_000.0, 0.0);
                assert!((fed - preset.fed_at_10_khz_db).abs() < 1.0, "{rate}: {fed}");
            }
        }
    }

    /// The sum of the absolute impulse responses into one output channel,
    /// which no input within full scale can take it past.
    #[test]
    fn no_input_within_full_scale_goes_over_it() {
        for preset in presets() {
            for rate in [44_100, 768_000] {
                let mut filter = Filter::new(preset.crossfeed, rate);
                let mut sum = 0.0;
                for n in 0..rate {
                    let out = filter.run([if n == 0 { 1.0 } else { 0.0 }, 0.0]);
                    sum += out[0].abs() + out[1].abs();
                }
                let peak_db = 20.0 * sum.log10();
                assert!(peak_db > -1.5 && peak_db < 0.0, "{rate}: {peak_db}");
            }
        }
    }

    #[test]
    fn a_successor_goes_on_from_the_state() {
        let crossfeed = presets()[0].crossfeed;
        let mut filter = Filter::new(crossfeed, 44_100);
        let mut steady = [0.0; 2];
        for _ in 0..44_100 {
            steady = filter.run([0.5, 0.0]);
        }
        let mut faster = filter.succeeded_by(crossfeed, 96_000);
        let first = faster.run([0.5, 0.0]);
        assert!((first[0] - steady[0]).abs() < 1e-9);
        assert!((first[1] - steady[1]).abs() < 1e-9);
    }
}
