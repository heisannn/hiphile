//! Time in input samples, for the output samples of a stage

/// Rates under this are whole numbers that a double holds exactly
const EXACT_INTEGER: f64 = 9_007_199_254_740_992.0;
/// The largest denominator that a ratio of rates that are not whole numbers
/// is searched for. Near 2^27, a convergent of almost any double divides to
/// it, which would make every ratio look short.
const MOST_SEARCHED_DENOMINATOR: u128 = 1 << 20;
/// 2^64, for 64.64 fixed point
const FIXED_ONE: f64 = 18_446_744_073_709_551_616.0;

/// A ratio of input samples to output samples, with its exact fraction when it
/// has a short one
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Ratio {
    pub(crate) value: f64,
    /// Numerator and denominator, in lowest terms
    pub(crate) fraction: Option<(u64, u64)>,
}

impl Ratio {
    /// `input_rate / output_rate`. Whole rates give their exact fraction, and
    /// others the shortest fraction that divides to the same double, if there
    /// is one with a denominator up to 2^20.
    pub(crate) fn of_rates(input_rate: f64, output_rate: f64) -> Self {
        let value = input_rate / output_rate;
        let is_whole = |rate: f64| rate.fract() == 0.0 && rate < EXACT_INTEGER;
        let fraction = if is_whole(input_rate) && is_whole(output_rate) {
            Some(reduced(input_rate as u128, output_rate as u128))
        } else {
            shortest_fraction(value)
        }
        .and_then(|(numerator, denominator)| {
            Some((
                u64::try_from(numerator).ok()?,
                u64::try_from(denominator).ok()?,
            ))
        });
        Self { value, fraction }
    }

    /// Times `numerator / denominator`, which must be powers of two so that the
    /// double stays exact
    pub(crate) fn scaled(self, numerator: u64, denominator: u64) -> Self {
        let value = self.value * numerator as f64 / denominator as f64;
        let fraction = self.fraction.and_then(|(top, bottom)| {
            let (top, bottom) = reduced(
                u128::from(top) * u128::from(numerator),
                u128::from(bottom) * u128::from(denominator),
            );
            Some((u64::try_from(top).ok()?, u64::try_from(bottom).ok()?))
        });
        Self { value, fraction }
    }

    pub(crate) fn is_one(&self) -> bool {
        self.value == 1.0
    }
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn reduced(numerator: u128, denominator: u128) -> (u128, u128) {
    let divisor = gcd(numerator, denominator).max(1);
    (numerator / divisor, denominator / divisor)
}

/// By continued fractions, the first convergent that divides to `value`
fn shortest_fraction(value: f64) -> Option<(u128, u128)> {
    if !(value.is_finite() && value > 0.0 && value < EXACT_INTEGER) {
        return None;
    }
    let (mut numerator, mut previous_numerator) = (value.floor() as u128, 1u128);
    let (mut denominator, mut previous_denominator) = (1u128, 0u128);
    let mut rest = value - value.floor();
    loop {
        if numerator as f64 / denominator as f64 == value {
            return Some((numerator, denominator));
        }
        if rest == 0.0 {
            return None;
        }
        let inverse = 1.0 / rest;
        let term = inverse.floor();
        rest = inverse - term;
        let term = term as u128;
        (numerator, previous_numerator) = (term * numerator + previous_numerator, numerator);
        (denominator, previous_denominator) =
            (term * denominator + previous_denominator, denominator);
        if denominator > MOST_SEARCHED_DENOMINATOR {
            return None;
        }
    }
}

/// Where an output sample lies in the input: a whole sample and the
/// fraction past it
#[derive(Clone, Copy, Debug)]
pub(crate) enum Fraction {
    /// Of a ratio with a denominator: the numerator of the fraction over it
    Exact(u64),
    /// Of 2^64
    Fixed(u64),
}

/// The time of the next output sample, advancing by a constant step
#[derive(Clone, Copy, Debug)]
pub(crate) enum Clock {
    Exact {
        whole: u64,
        numerator: u64,
        step_whole: u64,
        step_numerator: u64,
        denominator: u64,
    },
    /// 64.64 fixed point
    Fixed { time: u128, step: u128 },
}

impl Clock {
    /// At 0
    pub(crate) fn new(step: Ratio) -> Self {
        match step.fraction {
            Some((numerator, denominator)) => Self::Exact {
                whole: 0,
                numerator: 0,
                step_whole: numerator / denominator,
                step_numerator: numerator % denominator,
                denominator,
            },
            None => Self::Fixed {
                time: 0,
                step: fixed(step.value),
            },
        }
    }

    pub(crate) fn whole(&self) -> u64 {
        match *self {
            Self::Exact { whole, .. } => whole,
            Self::Fixed { time, .. } => (time >> 64) as u64,
        }
    }

    pub(crate) fn fraction(&self) -> Fraction {
        match *self {
            Self::Exact { numerator, .. } => Fraction::Exact(numerator),
            Self::Fixed { time, .. } => Fraction::Fixed(time as u64),
        }
    }

    /// Of a sample, in `[0, 1)`
    pub(crate) fn fraction_f64(&self) -> f64 {
        match *self {
            Self::Exact {
                numerator,
                denominator,
                ..
            } => numerator as f64 / denominator as f64,
            Self::Fixed { time, .. } => (time as u64) as f64 / FIXED_ONE,
        }
    }

    pub(crate) fn advance(&mut self) {
        match self {
            Self::Exact {
                whole,
                numerator,
                step_whole,
                step_numerator,
                denominator,
            } => {
                *whole += *step_whole;
                *numerator += *step_numerator;
                if *numerator >= *denominator {
                    *numerator -= *denominator;
                    *whole += 1;
                }
            }
            Self::Fixed { time, step } => *time += *step,
        }
    }

    pub(crate) fn reset(&mut self) {
        match self {
            Self::Exact {
                whole, numerator, ..
            } => {
                *whole = 0;
                *numerator = 0;
            }
            Self::Fixed { time, .. } => *time = 0,
        }
    }
}

/// In 64.64 fixed point, exact unless `value` has bits under 2^-64
pub(crate) fn fixed(value: f64) -> u128 {
    let whole = value.floor();
    (whole as u128) << 64 | ((value - whole) * FIXED_ONE) as u128
}

#[cfg(test)]
mod tests;
