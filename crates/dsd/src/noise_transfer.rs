use std::f64::consts::PI;

/// Zero angles as fractions of the band edge, from the roots of Legendre
/// polynomials. An entry of `0` is one zero at 0 Hz; any other is a conjugate
/// pair.
const ZEROS_3: [f64; 2] = [0.0, 0.7746];
const ZEROS_5: [f64; 3] = [0.0, 0.5385, 0.9062];
const ZEROS_7: [f64; 4] = [0.0, 0.4058, 0.7415, 0.9491];
const ZEROS_9: [f64; 5] = [0.0, 0.3243, 0.6134, 0.8360, 0.9682];

const SEARCH_STEPS: usize = 80;
const LOWEST_CORNER: f64 = 1e-4;
const HIGHEST_CORNER: f64 = 0.45;

#[derive(Clone, Copy)]
struct Complex {
    re: f64,
    im: f64,
}

impl Complex {
    const fn real(re: f64) -> Self {
        Self { re, im: 0.0 }
    }

    fn times(self, other: Self) -> Self {
        Self {
            re: self.re * other.re - self.im * other.im,
            im: self.re * other.im + self.im * other.re,
        }
    }

    fn over(self, other: Self) -> Self {
        let scale = other.re * other.re + other.im * other.im;
        Self {
            re: (self.re * other.re + self.im * other.im) / scale,
            im: (self.im * other.re - self.re * other.im) / scale,
        }
    }

    fn distance_to_minus_one(self) -> f64 {
        (1.0 + self.re).hypot(self.im)
    }
}

/// The coefficients of a monic noise transfer function, highest power of `z`
/// first, so that `numerator[0]` and `denominator[0]` are `1`.
pub(crate) struct NoiseTransfer {
    pub(crate) numerator: Vec<f64>,
    pub(crate) denominator: Vec<f64>,
}

impl NoiseTransfer {
    /// Butterworth poles with their corner searched for the gain at half the
    /// sample rate to come to `noise_gain`, over zeros in the noise band.
    pub(crate) fn design(
        order: usize,
        noise_gain: f64,
        sample_rate: f64,
        band: f64,
        optimized_zeros: bool,
    ) -> Self {
        let zeros = zeros(order, sample_rate / (2.0 * band), optimized_zeros);
        let (mut low, mut high) = (LOWEST_CORNER, HIGHEST_CORNER);
        for _ in 0..SEARCH_STEPS {
            let corner = (low + high) / 2.0;
            if gain_at_nyquist(&zeros, &poles(order, corner)) < noise_gain {
                low = corner;
            } else {
                high = corner;
            }
        }
        Self {
            numerator: polynomial(&zeros),
            denominator: polynomial(&poles(order, low)),
        }
    }
}

fn zeros(order: usize, oversampling: f64, optimized: bool) -> Vec<Complex> {
    if !optimized {
        return vec![Complex::real(1.0); order];
    }
    let fractions: &[f64] = match order {
        3 => &ZEROS_3,
        5 => &ZEROS_5,
        7 => &ZEROS_7,
        9 => &ZEROS_9,
        _ => unreachable!("`Order` has no other count"),
    };
    let mut zeros = Vec::with_capacity(order);
    for fraction in fractions {
        if *fraction == 0.0 {
            zeros.push(Complex::real(1.0));
        } else {
            let angle = fraction * PI / oversampling;
            let (im, re) = angle.sin_cos();
            zeros.push(Complex { re, im });
            zeros.push(Complex { re, im: -im });
        }
    }
    zeros
}

/// Through the bilinear transform; `corner` is a fraction of the sample rate.
fn poles(order: usize, corner: f64) -> Vec<Complex> {
    let warped = (PI * corner).tan();
    (0..order)
        .map(|index| {
            let angle = PI * (2 * index + order + 1) as f64 / (2 * order) as f64;
            let (im, re) = angle.sin_cos();
            let (re, im) = (warped * re, warped * im);
            Complex { re: 1.0 + re, im }.over(Complex {
                re: 1.0 - re,
                im: -im,
            })
        })
        .collect()
}

fn gain_at_nyquist(zeros: &[Complex], poles: &[Complex]) -> f64 {
    let product = |roots: &[Complex]| {
        roots
            .iter()
            .map(|root| root.distance_to_minus_one())
            .product::<f64>()
    };
    product(zeros) / product(poles)
}

/// Conjugate roots leave only rounding noise in the imaginary parts.
fn polynomial(roots: &[Complex]) -> Vec<f64> {
    let mut coefficients = vec![Complex::real(1.0)];
    for root in roots {
        let mut grown = vec![Complex::real(0.0); coefficients.len() + 1];
        for (index, coefficient) in coefficients.iter().enumerate() {
            grown[index].re += coefficient.re;
            grown[index].im += coefficient.im;
            let shifted = coefficient.times(Complex {
                re: -root.re,
                im: -root.im,
            });
            grown[index + 1].re += shifted.re;
            grown[index + 1].im += shifted.im;
        }
        coefficients = grown;
    }
    coefficients.iter().map(|value| value.re).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gain_at(transfer: &NoiseTransfer, angle: f64) -> f64 {
        let (im, re) = angle.sin_cos();
        let z = Complex { re, im };
        let value = |coefficients: &[f64]| {
            coefficients.iter().fold(Complex::real(0.0), |sum, c| {
                let shifted = sum.times(z);
                Complex {
                    re: shifted.re + c,
                    im: shifted.im,
                }
            })
        };
        let top = value(&transfer.numerator);
        let bottom = value(&transfer.denominator);
        top.over(bottom).re.hypot(top.over(bottom).im)
    }

    #[test]
    fn the_gain_out_of_band_comes_to_the_one_asked_for() {
        for order in [3, 5, 7, 9] {
            let transfer = NoiseTransfer::design(order, 1.4, 2_822_400.0, 20_000.0, true);
            assert!((gain_at(&transfer, PI) - 1.4).abs() < 1e-3, "{order}");
        }
    }

    #[test]
    fn every_pole_is_inside_the_unit_circle() {
        for order in [3, 5, 7, 9] {
            for pole in poles(order, 0.05) {
                assert!(pole.re.hypot(pole.im) < 1.0, "{order}");
            }
        }
    }

    #[test]
    fn spreading_the_zeros_lowers_the_noise_at_the_band_edge() {
        let edge = 2.0 * PI * 20_000.0 / 2_822_400.0;
        let spread = NoiseTransfer::design(7, 1.4, 2_822_400.0, 20_000.0, true);
        let together = NoiseTransfer::design(7, 1.4, 2_822_400.0, 20_000.0, false);
        assert!(gain_at(&spread, edge) < gain_at(&together, edge) / 10.0);
    }
}
