use crate::noise_transfer::NoiseTransfer;
use crate::settings::Modulation;

const MAX_ORDER: usize = 9;

/// How far the quantizer's input may go before the loop counts as lost.
/// Music reaches about 5 on a stable design; one that has lost control grows
/// without bound.
const WATCHDOG_LIMIT: f64 = 30.0;

/// Maps full scale PCM to 50 % modulation, the level DSD players expect.
const FULL_SCALE: f64 = 0.5;

/// A 1-bit error-feedback modulator for one channel. Its noise transfer
/// function is `N(z)/D(z)`, and its signal transfer function is 1.
#[derive(Clone)]
pub(crate) struct Modulator {
    order: usize,
    from_error: [f64; MAX_ORDER],
    from_feedback: [f64; MAX_ORDER],
    errors: [f64; MAX_ORDER],
    feedbacks: [f64; MAX_ORDER],
    dither_scale: f64,
    random: u64,
    is_lost: bool,
}

impl Modulator {
    pub(crate) fn new(modulation: &Modulation, bit_rate: f64, seed: u64) -> Self {
        let order = modulation.order.count();
        let transfer = NoiseTransfer::design(
            order,
            modulation.noise_gain,
            bit_rate,
            modulation.noise_band.hertz(),
            modulation.optimized_zeros,
        );
        let mut from_error = [0.0; MAX_ORDER];
        let mut from_feedback = [0.0; MAX_ORDER];
        for index in 0..order {
            let numerator = transfer.numerator[index + 1];
            let denominator = transfer.denominator[index + 1];
            from_error[index] = numerator - denominator;
            from_feedback[index] = -denominator;
        }
        Self {
            order,
            from_error,
            from_feedback,
            errors: [0.0; MAX_ORDER],
            feedbacks: [0.0; MAX_ORDER],
            dither_scale: modulation
                .dither_db
                .map_or(0.0, |dither_db| 10f64.powf(dither_db / 20.0)),
            // Never zero, where the generator would stay
            random: seed | 1,
            is_lost: false,
        }
    }

    /// One bit for `sample`, where `1.0` is full scale PCM.
    #[inline]
    pub(crate) fn step(&mut self, sample: f64) -> bool {
        let order = self.order;
        let mut feedback = 0.0;
        for index in 0..order {
            feedback += self.from_error[index] * self.errors[index]
                + self.from_feedback[index] * self.feedbacks[index];
        }
        let input = sample * FULL_SCALE + feedback;
        if input.abs() > WATCHDOG_LIMIT {
            self.is_lost = true;
            self.errors = [0.0; MAX_ORDER];
            self.feedbacks = [0.0; MAX_ORDER];
            return sample >= 0.0;
        }
        let bit = input + self.dither() >= 0.0;
        let output = if bit { 1.0 } else { -1.0 };
        self.errors.copy_within(0..order - 1, 1);
        self.feedbacks.copy_within(0..order - 1, 1);
        self.errors[0] = output - input;
        self.feedbacks[0] = feedback;
        bit
    }

    /// Whether the loop has ever had to be reset. It rarely sounds right again
    /// after that, as what drove it there tends to go on.
    pub(crate) fn is_lost(&self) -> bool {
        self.is_lost
    }

    #[inline]
    fn dither(&mut self) -> f64 {
        if self.dither_scale == 0.0 {
            return 0.0;
        }
        let first = self.next_random();
        let second = self.next_random();
        (first - second) * self.dither_scale
    }

    /// Xorshift, from 0 to 1.
    #[inline]
    fn next_random(&mut self) -> f64 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;
        (self.random >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Order;
    use std::f64::consts::PI;

    const RATE: f64 = 2_822_400.0;

    /// The level of a sine in the output, and of all else below `band` hertz,
    /// from a Hann-windowed DFT over the band only.
    fn levels(bits: &[bool], frequency: f64, band: f64) -> (f64, f64) {
        let len = bits.len();
        let bin_width = RATE / len as f64;
        let window = |index: usize| 0.5 - 0.5 * (2.0 * PI * index as f64 / len as f64).cos();
        let bin_power = |bin: usize| {
            let step = 2.0 * PI * bin as f64 / len as f64;
            let (mut re, mut im) = (0.0, 0.0);
            for (index, bit) in bits.iter().enumerate() {
                let value = if *bit { 1.0 } else { -1.0 } * window(index);
                let (sin, cos) = (step * index as f64).sin_cos();
                re += value * cos;
                im += value * sin;
            }
            re * re + im * im
        };
        let tone = (frequency / bin_width).round() as usize;
        // A sparse sampling of the band, which is enough to bound the floor
        let mut noise = 0.0;
        let last = (band / bin_width) as usize;
        let spacing = (last / 64).max(1);
        let mut counted = 0;
        for bin in (4..last).step_by(spacing) {
            if bin.abs_diff(tone) > 4 {
                noise += bin_power(bin);
                counted += 1;
            }
        }
        noise *= (last - 4) as f64 / f64::from(counted);
        let signal: f64 = (tone - 2..=tone + 2).map(bin_power).sum();
        (signal, noise)
    }

    #[test]
    fn a_tone_stands_far_above_the_noise_in_band_at_every_order() {
        for (order, least_db) in [
            (Order::Three, 55.0),
            (Order::Five, 80.0),
            (Order::Seven, 100.0),
            (Order::Nine, 100.0),
        ] {
            let mut modulator = Modulator::new(&Modulation::default().with_order(order), RATE, 1);
            let frequency = RATE * 64.0 / (1 << 16) as f64;
            let bits: Vec<bool> = (0..1 << 16)
                .map(|index| {
                    let phase = 2.0 * PI * frequency * f64::from(index) / RATE;
                    modulator.step(0.5 * phase.sin())
                })
                .collect();
            let (signal, noise) = levels(&bits, frequency, 20_000.0);
            let db = 10.0 * (signal / noise).log10();
            assert!(db > least_db, "{order:?}: {db:.1} dB");
            assert!(!modulator.is_lost());
        }
    }

    #[test]
    fn full_scale_stays_stable() {
        let mut modulator = Modulator::new(&Modulation::default(), RATE, 1);
        for index in 0..2_000_000 {
            let phase = 2.0 * PI * 1_000.0 * f64::from(index) / RATE;
            modulator.step(phase.sin());
        }
        assert!(!modulator.is_lost());
    }

    #[test]
    fn a_design_that_cannot_hold_is_noticed_and_goes_on_from_zero() {
        let modulation = Modulation::default().with_noise_gain(3.0);
        let mut modulator = Modulator::new(&modulation, RATE, 1);
        for index in 0..2_000_000 {
            let phase = 2.0 * PI * 1_000.0 * f64::from(index) / RATE;
            modulator.step(1.1 * phase.sin());
        }
        assert!(modulator.is_lost());
    }

    #[test]
    fn dither_keeps_the_loop_stable_and_the_noise_shaped() {
        let modulation = Modulation::default().with_dither_db(Some(-40.0));
        let mut modulator = Modulator::new(&modulation, RATE, 1);
        let frequency = RATE * 64.0 / (1 << 16) as f64;
        let bits: Vec<bool> = (0..1 << 16)
            .map(|index| {
                let phase = 2.0 * PI * frequency * f64::from(index) / RATE;
                modulator.step(0.5 * phase.sin())
            })
            .collect();
        let (signal, noise) = levels(&bits, frequency, 20_000.0);
        assert!(10.0 * (signal / noise).log10() > 100.0);
    }
}
