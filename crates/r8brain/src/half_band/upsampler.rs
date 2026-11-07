use super::{for_each_kernel_len, kernel_taps, symmetric_sum};
use crate::processor::Processor;

const BUF_LEN: usize = 128;
const BUF_MASK: usize = BUF_LEN - 1;
/// Past the end of the ring, a copy of its start, so that a filter's window
/// never has to wrap
const LONGEST_OVERRUN: usize = 27;

/// By length of kernel, from 1. Fills `output` with a sample and the one after
/// it for each window in `source`, from its start.
type Convolve = fn(taps: &[f64], source: &[f64], output: &mut [f64]);
const CONVOLVE: [Convolve; 14] = for_each_kernel_len!(convolve);

fn convolve<const N: usize>(taps: &[f64], source: &[f64], output: &mut [f64]) {
    let taps: &[f64; N] = taps
        .try_into()
        .expect("this length was chosen by the kernel");
    let windows = source.windows(2 * N);
    for (pair, window) in output.as_chunks_mut::<2>().0.iter_mut().zip(windows) {
        pair[0] = window[N - 1];
        pair[1] = symmetric_sum(taps, window);
    }
}

/// Doubles the rate, keeping every input sample as it is.
pub(crate) struct Upsampler {
    buf: Vec<f64>,
    taps: &'static [f64],
    convolve: Convolve,
    /// Samples a window takes after its centre
    following: usize,
    overrun: usize,
    initial_read: usize,
    latency: usize,
    latency_frac: f64,
    latency_left: usize,
    buf_left: usize,
    write_pos: usize,
    read_pos: usize,
}

impl Upsampler {
    pub(crate) fn new(
        attenuation: f64,
        steep_index: usize,
        is_third: bool,
        prev_latency: f64,
    ) -> Self {
        let taps = kernel_taps(attenuation, steep_index, is_third);
        let preceding = taps.len() - 1;
        let carried = prev_latency * 2.0;
        let mut upsampler = Self {
            buf: vec![0.0; BUF_LEN + LONGEST_OVERRUN],
            taps,
            convolve: CONVOLVE[taps.len() - 1],
            following: taps.len(),
            overrun: preceding + taps.len(),
            initial_read: BUF_LEN - preceding,
            latency: carried as usize,
            latency_frac: carried - carried.trunc(),
            latency_left: 0,
            buf_left: 0,
            write_pos: 0,
            read_pos: 0,
        };
        upsampler.clear();
        upsampler
    }
}

impl Processor for Upsampler {
    fn input_before_output(&self, position: i64) -> i64 {
        self.following as i64
            + ((self.latency as f64 + self.latency_frac + position as f64) * 0.5) as i64
    }

    fn latency_frac(&self) -> f64 {
        self.latency_frac
    }

    fn max_output_len(&self, input_len: usize) -> usize {
        input_len * 2
    }

    fn clear(&mut self) {
        self.latency_left = self.latency;
        self.buf_left = 0;
        self.write_pos = 0;
        self.read_pos = self.initial_read & BUF_MASK;
        self.buf[self.read_pos..self.read_pos + BUF_LEN - self.initial_read].fill(0.0);
    }

    fn process(&mut self, input: &[f64], output: &mut Vec<f64>) -> usize {
        // Through the ring until the windows lie wholly within the input
        let mut copied = 0;
        while copied < input.len() && copied < self.overrun {
            let count = (input.len() - copied)
                .min(self.overrun - copied)
                .min(BUF_LEN - self.write_pos)
                .min(self.initial_read - self.buf_left);
            let source = &input[copied..copied + count];
            self.buf[self.write_pos..self.write_pos + count].copy_from_slice(source);
            if self.write_pos < self.overrun {
                let mirrored = count.min(self.overrun - self.write_pos);
                let start = self.write_pos + BUF_LEN;
                self.buf[start..start + mirrored].copy_from_slice(&source[..mirrored]);
            }
            copied += count;
            self.write_pos = (self.write_pos + count) & BUF_MASK;
            self.buf_left += count;

            while self.buf_left > self.following {
                let windows = (self.buf_left - self.following).min(BUF_LEN - self.read_pos);
                let start = output.len();
                output.resize(start + windows * 2, 0.0);
                (self.convolve)(self.taps, &self.buf[self.read_pos..], &mut output[start..]);
                self.read_pos = (self.read_pos + windows) & BUF_MASK;
                self.buf_left -= windows;
            }
        }

        let rest = input.len() - copied;
        if rest > 0 {
            let start = output.len();
            output.resize(start + rest * 2, 0.0);
            (self.convolve)(self.taps, input, &mut output[start..]);
            // The ring takes up where the input ends, with the samples still
            // owed a window
            self.buf[..self.overrun].copy_from_slice(&input[input.len() - self.overrun..]);
            self.read_pos = 0;
            self.write_pos = self.overrun;
            self.buf_left = self.following;
        }

        let start = self.latency_left.min(output.len());
        self.latency_left -= start;
        start
    }
}
