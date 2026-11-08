//! Kaiser-windowed sinc low-pass filters

use std::f64::consts::{PI, TAU};

/// 20·log10(2): decibels per bit of precision
pub(crate) const DB_PER_BIT: f64 = 6.020_599_913_279_624;

/// The modified Bessel function of the first kind and order 0, by its series
pub(crate) fn bessel_i0(x: f64) -> f64 {
    let half = x / 2.0;
    let mut sum = 1.0;
    let mut term = 1.0;
    for k in 1..500 {
        let factor = half / f64::from(k);
        term *= factor * factor;
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

/// Kaiser's fit of the window's shape to a stop-band `attenuation` dB down
pub(crate) fn kaiser_beta(attenuation: f64) -> f64 {
    if attenuation > 50.0 {
        0.1102 * (attenuation - 8.7)
    } else if attenuation >= 21.0 {
        0.5842 * (attenuation - 21.0).powf(0.4) + 0.07886 * (attenuation - 21.0)
    } else {
        0.0
    }
}

/// Kaiser's fit of the length, in samples, of a filter that reaches
/// `attenuation` dB over a transition `width` cycles per sample wide. Past
/// about 150 dB the fit falls short by up to 9 dB, which a few percent more
/// length makes up.
pub(crate) fn kaiser_len(attenuation: f64, width: f64) -> f64 {
    let stretch = 1.0 + ((attenuation - 140.0) / 1000.0).max(0.0);
    (attenuation - 7.95) / (2.285 * TAU * width) * stretch + 1.0
}

/// Exactly 0 at the whole numbers but 0, where `sin` of a multiple of `PI`
/// would leave rounding
fn sinc(x: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else if x.fract() == 0.0 {
        0.0
    } else {
        (PI * x).sin() / (PI * x)
    }
}

/// A low-pass impulse response as a function of time in samples, centred on
/// 0, with a pass-band gain of `gain`
#[derive(Clone, Copy, Debug)]
pub(crate) struct Kernel {
    /// In cycles per sample, where the gain is half
    cutoff: f64,
    /// Past this, in samples either way, the response is 0
    half_len: f64,
    beta: f64,
    window_scale: f64,
    gain: f64,
}

impl Kernel {
    pub(crate) fn new(cutoff: f64, half_len: f64, attenuation: f64, gain: f64) -> Self {
        let beta = kaiser_beta(attenuation);
        Self {
            cutoff,
            half_len,
            beta,
            window_scale: 1.0 / bessel_i0(beta),
            gain,
        }
    }

    /// For a pass-band that ends at `passband_end` and a stop-band that begins
    /// at `stopband_begin`, both in cycles per sample, `attenuation` dB down
    pub(crate) fn low_pass(
        passband_end: f64,
        stopband_begin: f64,
        attenuation: f64,
        gain: f64,
    ) -> Self {
        let width = stopband_begin - passband_end;
        let half_len = (kaiser_len(attenuation, width) - 1.0) / 2.0;
        Self::new(
            (passband_end + stopband_begin) / 2.0,
            half_len,
            attenuation,
            gain,
        )
    }

    pub(crate) fn half_len(&self) -> f64 {
        self.half_len
    }

    pub(crate) fn with_half_len(self, half_len: f64) -> Self {
        Self { half_len, ..self }
    }

    pub(crate) fn with_gain(self, gain: f64) -> Self {
        Self { gain, ..self }
    }

    pub(crate) fn at(&self, time: f64) -> f64 {
        let position = time / self.half_len;
        if position.abs() > 1.0 {
            return 0.0;
        }
        let window = bessel_i0(self.beta * (1.0 - position * position).sqrt()) * self.window_scale;
        self.gain * 2.0 * self.cutoff * sinc(2.0 * self.cutoff * time) * window
    }

    /// The gain at 0 Hz as the response stands, from samples `1 / per_sample`
    /// apart, which is exact for a response with nothing at `per_sample`
    /// cycles per sample or above
    pub(crate) fn dc_gain(&self, per_sample: u32) -> f64 {
        let step = 1.0 / f64::from(per_sample);
        let reach = (self.half_len * f64::from(per_sample)).ceil() as i64;
        (-reach..=reach)
            .map(|index| self.at(index as f64 * step))
            .sum::<f64>()
            * step
    }

    /// Odd-length samples of the response, centred on the middle one, with
    /// their sum made `gain`
    pub(crate) fn taps(&self) -> Vec<f64> {
        let half = self.half_len.ceil() as i64;
        let mut taps: Vec<f64> = (-half..=half).map(|index| self.at(index as f64)).collect();
        let sum: f64 = taps.iter().sum();
        let scale = self.gain / sum;
        for tap in &mut taps {
            *tap *= scale;
        }
        taps
    }
}

/// Of the precision in bits, how far the stop-band lies under the pass-band
pub(crate) fn attenuation(precision: f64) -> f64 {
    (precision + 1.0) * DB_PER_BIT
}
