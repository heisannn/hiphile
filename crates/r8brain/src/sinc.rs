use std::f64::consts::PI;

use crate::math::{SineGenerator, bessel_i0, pow_abs};

/// Under this, a fractional delay counts as landing on a sample
const ZERO_CROSSING_MARGIN: f64 = 2.3e-13;

/// Of a Kaiser window raised to a power
#[derive(Clone, Copy)]
pub(crate) struct KaiserParams {
    pub(crate) beta: f64,
    pub(crate) power: f64,
}

struct KaiserWindow {
    beta: f64,
    power: f64,
    inverse_i0_of_beta: f64,
    inverse_half_len: f64,
    offset: f64,
    position: i32,
}

impl KaiserWindow {
    fn new(params: KaiserParams, half_len: f64, frac_delay: f64, position: i32) -> Self {
        let beta = params.beta.clamp(1.0, 350.0);
        let inverse_half_len = 1.0 / half_len;
        Self {
            beta,
            power: params.power.abs(),
            inverse_i0_of_beta: 1.0 / bessel_i0(beta),
            inverse_half_len,
            offset: frac_delay * inverse_half_len,
            position,
        }
    }

    fn next_value(&mut self) -> f64 {
        let x = f64::from(self.position) * self.inverse_half_len + self.offset;
        self.position += 1;
        let inside = 1.0 - x * x;
        let window = if inside <= 0.0 {
            0.0
        } else {
            bessel_i0(self.beta * inside.sqrt()) * self.inverse_i0_of_beta
        };
        pow_abs(window, self.power)
    }
}

/// Fills `taps`, of an odd length, with a low-pass windowed sinc. `half_len` is
/// the window's half length, of which `taps` holds the whole samples, and
/// `cutoff` a circular frequency.
pub(crate) fn low_pass(taps: &mut [f64], half_len: f64, cutoff: f64, params: KaiserParams) {
    let center = taps.len() / 2;
    let mut sine = SineGenerator::new(cutoff, 0.0, 1.0 / PI);
    sine.next_value();
    let mut window = KaiserWindow::new(params, half_len, 0.0, 0);

    taps[center] = cutoff * window.next_value() / PI;
    for distance in 1..=center {
        let value = sine.next_value() * window.next_value() / distance as f64;
        taps[center + distance] = value;
        taps[center - distance] = value;
    }
}

/// Fills every `stride`th element of `out` with the `half_len * 2` taps of a
/// sinc filter of full bandwidth. A `frac_delay` of 1 delays by nothing and one
/// of 0 by a whole sample, and values beyond those serve to interpolate
/// between such filters.
pub(crate) fn fractional_delay(
    out: &mut [f64],
    stride: usize,
    half_len: usize,
    frac_delay: f64,
    params: KaiserParams,
) {
    let half = half_len as f64;
    let last = half_len as i32;
    let mut window = KaiserWindow::new(params, half, frac_delay, -last);
    let mut index = 0;
    let mut put = |value: f64| {
        out[index] = value;
        index += stride;
    };

    let mut t = -last;
    if f64::from(t) + frac_delay < -half {
        window.next_value();
        put(0.0);
        t += 1;
    }

    let mut sign_and_sine = (frac_delay * PI).sin() / PI;
    if t & 1 != 0 {
        sign_and_sine = -sign_and_sine;
    }

    let lands_on_previous = (frac_delay - 1.0).abs() < ZERO_CROSSING_MARGIN;
    let lands_on_sample = lands_on_previous || frac_delay.abs() < ZERO_CROSSING_MARGIN;
    let crossing = if lands_on_previous { -1 } else { 0 };

    while t < crossing {
        put(sign_and_sine * window.next_value() / (f64::from(t) + frac_delay));
        t += 1;
        sign_and_sine = -sign_and_sine;
    }

    if lands_on_sample {
        put(window.next_value());
    } else {
        put(sign_and_sine * window.next_value() / frac_delay);
    }

    while t < last - 2 {
        t += 1;
        sign_and_sine = -sign_and_sine;
        put(sign_and_sine * window.next_value() / (f64::from(t) + frac_delay));
    }

    t += 1;
    sign_and_sine = -sign_and_sine;
    let distance = f64::from(t) + frac_delay;
    put(if distance > half {
        0.0
    } else {
        sign_and_sine * window.next_value() / distance
    });
}
