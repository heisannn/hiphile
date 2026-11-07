use super::{for_each_kernel_len, kernel_taps, symmetric_sum};
use crate::processor::Processor;

const BUF_LEN: usize = 1024;
const BUF_MASK: usize = BUF_LEN - 1;
/// Past the end of each ring, a copy of its start, so that a filter's window
/// never has to wrap
const LONGEST_OVERRUN: usize = 27;

/// By length of kernel, from 1. Fills `output` with a sample for each of
/// `centers` and of the windows in `odd`, from its start.
type Convolve = fn(taps: &[f64], centers: &[f64], odd: &[f64], output: &mut [f64]);
const CONVOLVE: [Convolve; 14] = for_each_kernel_len!(convolve);

fn convolve<const N: usize>(taps: &[f64], centers: &[f64], odd: &[f64], output: &mut [f64]) {
    let taps: &[f64; N] = taps
        .try_into()
        .expect("this length was chosen by the kernel");
    let windows = odd.windows(2 * N);
    for ((sample, center), window) in output.iter_mut().zip(centers).zip(windows) {
        *sample = center + symmetric_sum(taps, window);
    }
}

/// Halves the rate at a gain of 2. Even input samples go to one ring and odd
/// ones to another, as only one of them holds taps other than the centre's.
pub(crate) struct Downsampler {
    even: Vec<f64>,
    odd: Vec<f64>,
    taps: &'static [f64],
    convolve: Convolve,
    /// Odd samples a window takes after its centre, less one
    following: usize,
    overrun: usize,
    initial_read: usize,
    latency: usize,
    latency_frac: f64,
    latency_left: usize,
    buf_left: usize,
    write_even: usize,
    write_odd: usize,
    read_pos: usize,
}

impl Downsampler {
    pub(crate) fn new(
        attenuation: f64,
        steep_index: usize,
        is_third: bool,
        prev_latency: f64,
    ) -> Self {
        let taps = kernel_taps(attenuation, steep_index, is_third);
        let following = taps.len() - 1;
        let carried = prev_latency * 0.5;
        let mut downsampler = Self {
            even: vec![0.0; BUF_LEN + LONGEST_OVERRUN],
            odd: vec![0.0; BUF_LEN + LONGEST_OVERRUN],
            taps,
            convolve: CONVOLVE[taps.len() - 1],
            following,
            overrun: taps.len() + following,
            initial_read: BUF_LEN - taps.len(),
            latency: carried as usize,
            latency_frac: carried - carried.trunc(),
            latency_left: 0,
            buf_left: 0,
            write_even: 0,
            write_odd: 0,
            read_pos: 0,
        };
        downsampler.clear();
        downsampler
    }

    fn store(&mut self, input: &mut &[f64]) {
        if self.write_even != self.write_odd {
            // A previous input ended between an even sample and its odd one
            let sample = input[0];
            self.odd[self.write_odd] = sample;
            if self.write_odd < self.overrun {
                self.odd[self.write_odd + BUF_LEN] = sample;
            }
            *input = &input[1..];
            self.write_odd = self.write_even;
            self.buf_left += 1;
        }

        let len = input.len();
        let start = self.write_even;
        let evens = len
            .div_ceil(2)
            .min(BUF_LEN - start)
            .min(self.initial_read - self.buf_left);
        let odds = evens - usize::from(evens * 2 > len);
        for (index, pair) in input[..odds * 2].as_chunks::<2>().0.iter().enumerate() {
            self.even[start + index] = pair[0];
            self.odd[start + index] = pair[1];
        }
        if evens != odds {
            self.even[start + odds] = input[odds * 2];
        }
        *input = &input[evens + odds..];

        if start < self.overrun {
            let room = self.overrun - start;
            self.even
                .copy_within(start..start + evens.min(room), start + BUF_LEN);
            self.odd
                .copy_within(start..start + odds.min(room), start + BUF_LEN);
        }

        self.write_even = (start + evens) & BUF_MASK;
        self.write_odd = (start + odds) & BUF_MASK;
        self.buf_left += odds;
    }
}

impl Processor for Downsampler {
    fn input_before_output(&self, position: i64) -> i64 {
        self.overrun as i64
            + ((self.latency as f64 + self.latency_frac + position as f64) * 2.0) as i64
    }

    fn latency_frac(&self) -> f64 {
        self.latency_frac
    }

    fn max_output_len(&self, input_len: usize) -> usize {
        input_len.div_ceil(2)
    }

    fn clear(&mut self) {
        self.latency_left = self.latency;
        self.buf_left = 0;
        self.write_even = 0;
        self.write_odd = 0;
        self.read_pos = self.initial_read;
        self.even[self.initial_read..BUF_LEN].fill(0.0);
        self.odd[self.initial_read..BUF_LEN].fill(0.0);
    }

    fn process(&mut self, input: &[f64], output: &mut Vec<f64>) -> usize {
        let mut rest = input;
        while !rest.is_empty() {
            self.store(&mut rest);
            while self.buf_left > self.following {
                let count = (self.buf_left - self.following).min(BUF_LEN - self.read_pos);
                let start = output.len();
                output.resize(start + count, 0.0);
                (self.convolve)(
                    self.taps,
                    &self.even[self.read_pos + self.taps.len()..],
                    &self.odd[self.read_pos..],
                    &mut output[start..],
                );
                self.read_pos = (self.read_pos + count) & BUF_MASK;
                self.buf_left -= count;
            }
        }
        let start = self.latency_left.min(output.len());
        self.latency_left -= start;
        start
    }
}
