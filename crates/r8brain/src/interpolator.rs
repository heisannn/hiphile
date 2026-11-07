use std::sync::Arc;

use crate::Options;
use crate::filter_bank::FilterBank;
use crate::processor::Processor;

const BUF_LEN: usize = 256;
const BUF_MASK: usize = BUF_LEN - 1;
/// Past the end of the ring, a copy of its start, so that a filter's window
/// never has to wrap
const LONGEST_OVERRUN: usize = 29;
/// Of the output position, before it restarts from the current fraction. A
/// large multiple of the step would lose the fraction's lower bits.
const POSITION_RESTART: i64 = 1000;
const LARGEST_WHOLE_OUT_STEP: f64 = 1500.0;

/// Reaches the target rate by a bank of short sinc filters. Rates at a ratio of
/// small whole numbers step through a row for each output position, which keeps
/// time exactly. Any others interpolate between rows.
pub(crate) struct Interpolator {
    buf: Vec<f64>,
    bank: Arc<FilterBank>,
    source_rate: f64,
    target_rate: f64,
    /// Samples a window takes from its centre on
    following: isize,
    overrun: usize,
    initial_read: usize,
    latency: usize,
    latency_frac: f64,
    latency_left: usize,
    buf_left: isize,
    write_pos: usize,
    read_pos: usize,
    stepping: Stepping,
}

enum Stepping {
    Whole {
        in_step: usize,
        out_step: usize,
        initial_row: usize,
        row: usize,
    },
    Fractional {
        initial_position: f64,
        step: f64,
        whole: i64,
        fraction: f64,
        /// Output samples since the position last restarted, and what a
        /// restart leaves of a fraction, in those
        output_position: f64,
    },
}

impl Interpolator {
    pub(crate) fn new(
        source_rate: f64,
        target_rate: f64,
        options: &Options,
        is_third: bool,
        prev_latency: f64,
    ) -> Self {
        let initial_position = prev_latency - prev_latency.trunc();
        let attenuation = options.attenuation;
        let steps = whole_stepping(source_rate, target_rate).filter(|_| options.whole_stepping);
        let (bank, stepping, latency_frac) = match steps {
            Some((in_step, out_step)) => {
                let start = initial_position * out_step as f64;
                let initial_row = start as usize;
                (
                    FilterBank::whole_step(out_step, attenuation, is_third),
                    Stepping::Whole {
                        in_step,
                        out_step,
                        initial_row,
                        row: initial_row,
                    },
                    (start - start.trunc()) / in_step as f64,
                )
            }
            None => (
                FilterBank::fractional(
                    attenuation,
                    is_third,
                    options.filter_fractions.map(|fracs| fracs as usize),
                ),
                Stepping::Fractional {
                    initial_position,
                    step: source_rate / target_rate,
                    whole: 0,
                    fraction: initial_position,
                    output_position: 0.0,
                },
                0.0,
            ),
        };

        let following = bank.filter_len() / 2;
        let preceding = following - 1;
        let mut interpolator = Self {
            buf: vec![0.0; BUF_LEN + LONGEST_OVERRUN],
            bank,
            source_rate,
            target_rate,
            following: following as isize,
            overrun: preceding + following,
            initial_read: BUF_LEN - preceding,
            latency: prev_latency as usize,
            latency_frac,
            latency_left: 0,
            buf_left: 0,
            write_pos: 0,
            read_pos: 0,
            stepping,
        };
        interpolator.clear();
        interpolator
    }

    fn store(&mut self, source: &[f64]) {
        self.buf[self.write_pos..self.write_pos + source.len()].copy_from_slice(source);
        if self.write_pos < self.overrun {
            let mirrored = source.len().min(self.overrun - self.write_pos);
            let start = self.write_pos + BUF_LEN;
            self.buf[start..start + mirrored].copy_from_slice(&source[..mirrored]);
        }
        self.write_pos = (self.write_pos + source.len()) & BUF_MASK;
        self.buf_left += source.len() as isize;
    }

    fn convolve(&mut self, output: &mut Vec<f64>) {
        let filter_len = self.bank.filter_len();
        let mut left = self.buf_left - self.following;
        let mut read = self.read_pos;
        match &mut self.stepping {
            Stepping::Whole {
                in_step,
                out_step,
                row,
                ..
            } => {
                while left > 0 {
                    let window = &self.buf[read..read + filter_len];
                    output.push(dot(self.bank.row(*row), window));
                    *row += *in_step;
                    let advance = *row / *out_step;
                    *row -= advance * *out_step;
                    read = (read + advance) & BUF_MASK;
                    left -= advance as isize;
                }
            }
            Stepping::Fractional {
                step,
                whole,
                fraction,
                output_position,
                ..
            } => {
                let rows = self.bank.fracs() as f64;
                while left > 0 {
                    let position = *fraction * rows;
                    let row = position as usize;
                    let between = position - position.trunc();
                    let window = &self.buf[read..read + filter_len];
                    output.push(interpolated_dot(self.bank.row(row), window, between));

                    *output_position += 1.0;
                    let next = *output_position * *step;
                    let next_whole = next as i64;
                    let advance = next_whole - *whole;
                    *fraction = next - next.trunc();
                    *whole = next_whole;
                    left -= advance as isize;
                    read = (read + advance as usize) & BUF_MASK;
                }
            }
        }
        self.buf_left = left + self.following;
        self.read_pos = read;
    }
}

impl Processor for Interpolator {
    fn input_before_output(&self, position: i64) -> i64 {
        let prior = self.following as i64 + self.latency as i64;
        let position = position as f64;
        prior
            + match self.stepping {
                Stepping::Whole {
                    in_step,
                    out_step,
                    initial_row,
                    ..
                } => {
                    let (in_step, out_step) = (in_step as f64, out_step as f64);
                    ((initial_row as f64 + position * in_step) / out_step
                        + self.latency_frac * in_step / out_step) as i64
                }
                Stepping::Fractional {
                    initial_position, ..
                } => (initial_position + position * self.source_rate / self.target_rate) as i64,
            }
    }

    fn latency_frac(&self) -> f64 {
        self.latency_frac
    }

    fn max_output_len(&self, input_len: usize) -> usize {
        (input_len as f64 * self.target_rate / self.source_rate).ceil() as usize + 1
    }

    fn clear(&mut self) {
        self.latency_left = self.latency;
        self.buf_left = 0;
        self.write_pos = 0;
        self.read_pos = self.initial_read;
        self.buf[self.initial_read..BUF_LEN].fill(0.0);
        match &mut self.stepping {
            Stepping::Whole {
                initial_row, row, ..
            } => *row = *initial_row,
            Stepping::Fractional {
                initial_position,
                whole,
                fraction,
                output_position,
                ..
            } => {
                *fraction = *initial_position;
                *whole = 0;
                *output_position = *initial_position / self.source_rate * self.target_rate;
            }
        }
    }

    fn process(&mut self, input: &[f64], output: &mut Vec<f64>) -> usize {
        let skipped = self.latency_left.min(input.len());
        self.latency_left -= skipped;
        let mut rest = &input[skipped..];

        let ratio = self.target_rate / self.source_rate;
        output.reserve((rest.len() as f64 * ratio).ceil() as usize + 1);

        while !rest.is_empty() {
            let room = (BUF_LEN - self.write_pos)
                .min((self.initial_read as isize - self.buf_left) as usize);
            let (source, others) = rest.split_at(rest.len().min(room));
            self.store(source);
            rest = others;
            self.convolve(output);
        }

        if let Stepping::Fractional {
            whole,
            fraction,
            output_position,
            ..
        } = &mut self.stepping
            && *output_position as i64 > POSITION_RESTART
        {
            *whole = 0;
            *output_position = *fraction / self.source_rate * self.target_rate;
        }
        0
    }
}

/// In separate lanes, as a single sum has to keep the order of its additions,
/// which rules out vectorizing it. Rows have an even length.
#[inline]
fn dot(row: &[f64], window: &[f64]) -> f64 {
    let mut sums = [0.0; 4];
    let (rows, row_rest) = row.as_chunks::<4>();
    let (windows, window_rest) = window.as_chunks::<4>();
    for (taps, samples) in rows.iter().zip(windows) {
        sums[0] += taps[0] * samples[0];
        sums[1] += taps[1] * samples[1];
        sums[2] += taps[2] * samples[2];
        sums[3] += taps[3] * samples[3];
    }
    if let ([tap_0, tap_1], [sample_0, sample_1]) = (row_rest, window_rest) {
        sums[0] += tap_0 * sample_0;
        sums[1] += tap_1 * sample_1;
    }
    (sums[0] + sums[1]) + (sums[2] + sums[3])
}

/// `row` holds each tap as a quadratic of `x`.
#[inline]
fn interpolated_dot(row: &[f64], window: &[f64], x: f64) -> f64 {
    let x2 = x * x;
    let mut sums = [0.0; 2];
    for (c, samples) in row.as_chunks::<6>().0.iter().zip(window.as_chunks::<2>().0) {
        sums[0] += (c[0] + c[1] * x + c[2] * x2) * samples[0];
        sums[1] += (c[3] + c[4] * x + c[5] * x2) * samples[1];
    }
    sums[0] + sums[1]
}

/// The input and output steps of two rates at a ratio of whole numbers, unless
/// the output step would make for a large and thus cache-unfriendly bank.
pub(crate) fn whole_stepping(source_rate: f64, target_rate: f64) -> Option<(usize, usize)> {
    let divisor = greatest_common_divisor(source_rate, target_rate)?;
    let in_step = source_rate / divisor;
    let out_step = target_rate / divisor;
    let is_whole = |step: f64| step == f64::from(step as i32);
    (out_step <= LARGEST_WHOLE_OUT_STEP && is_whole(in_step) && is_whole(out_step))
        .then_some((in_step as usize, out_step as usize))
}

fn greatest_common_divisor(a: f64, b: f64) -> Option<f64> {
    const MOST_STEPS: usize = 149;
    let (mut large, mut small) = (a, b);
    for _ in 0..MOST_STEPS {
        let difference = large - small;
        if difference == 0.0 {
            return (small > 0.0).then_some(small);
        }
        large = small;
        small = difference.abs();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_at_a_ratio_of_small_whole_numbers_step_wholly() {
        assert_eq!(whole_stepping(88_200.0, 48_000.0), Some((147, 80)));
        assert_eq!(whole_stepping(96_000.0, 44_100.0), Some((320, 147)));
    }

    #[test]
    fn other_rates_do_not_step_wholly() {
        assert_eq!(whole_stepping(44_100.0, 47_999.5), None);
        assert_eq!(whole_stepping(1.0, 3000.0), None);
    }
}
