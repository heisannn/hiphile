use std::sync::Arc;

use realfft::num_complex::Complex;

use crate::fft::RealFft;
use crate::fir_filter::FirFilter;
use crate::math::exact_shift;
use crate::processor::Processor;

/// Overlap-save convolution in one FFT block, with whole-number up- and
/// downsampling built in.
///
/// A power-of-two factor happens in the frequency domain: the input's spectrum
/// is mirrored to double the rate, or the output's spectrum cut short to divide
/// it. Any other factor stuffs zeros before the transforms or drops samples
/// after them.
///
/// Output comes from the block before the one being filled, so that each input
/// sample brings its share of output at once, at the price of one block of
/// latency.
pub(crate) struct BlockConvolver {
    filter: Arc<FirFilter>,
    fft_in: RealFft,
    fft_out: RealFft,
    up_factor: usize,
    down_factor: usize,
    doubled_block_len: usize,
    out_offset: isize,
    input_len: usize,
    latency: usize,
    latency_frac: f64,
    /// `None` if the factor is no power of two
    up_shift: Option<u32>,
    down_shift: Option<u32>,
    /// Zeros ahead of the first block, as a power-of-two down shift can only
    /// take whole steps of its factor, and the latency has to come to some
    input_delay: usize,
    prev_input: Vec<f64>,
    current_input: Vec<f64>,
    current_output: Vec<f64>,
    spectrum: Vec<Complex<f64>>,
    scratch: Vec<Complex<f64>>,
    in_data_left: usize,
    latency_left: usize,
    up_skip: usize,
    down_skip: usize,
}

impl BlockConvolver {
    /// With upsampling, `filter`'s gain has to equal `up_factor`. Only a factor
    /// of up to 2 may be a power of two, and then the other factor may not be
    /// one above 1.
    pub(crate) fn new(
        filter: Arc<FirFilter>,
        up_factor: usize,
        down_factor: usize,
        prev_latency: f64,
    ) -> Self {
        let doubled_block_len = 2usize << filter.block_len_bits;
        let up_shift = exact_shift(up_factor);
        let kernel_len = filter.kernel_len;

        let (mut prev_input_len, mut input_len) = match up_shift {
            Some(_) => {
                let prev = (kernel_len - 1).div_ceil(up_factor);
                (prev, doubled_block_len - prev * up_factor)
            }
            None => (kernel_len - 1, doubled_block_len - (kernel_len - 1)),
        };

        let out_offset = if filter.is_zero_phase() {
            filter.latency
        } else {
            0
        };
        let carried = filter.latency_frac + prev_latency * up_factor as f64;
        let whole_carried = carried as usize;
        let in_latency = whole_carried + filter.latency - out_offset;
        let latency_frac = (carried - carried.trunc()) / down_factor as f64;
        let mut latency = whole_carried + input_len + filter.latency;

        let down_shift_bits = exact_shift(down_factor);
        let mut input_delay = 0;
        if down_factor > 1 && down_shift_bits.is_some() {
            debug_assert!(up_shift.is_none_or(|shift| shift == 0));
            let excess = input_len & (down_factor - 1);
            prev_input_len += excess;
            input_len -= excess;
            latency -= excess;
            let misaligned = in_latency & (down_factor - 1);
            if misaligned > 0 {
                input_delay = down_factor - misaligned;
            }
        }

        let fft_in = RealFft::new(doubled_block_len >> up_shift.unwrap_or(0));
        let fft_out = RealFft::new(doubled_block_len >> down_shift_bits.unwrap_or(0));
        let scratch_len = fft_in.scratch_len().max(fft_out.scratch_len());

        let mut convolver = Self {
            filter,
            fft_in,
            fft_out,
            up_factor,
            down_factor,
            doubled_block_len,
            out_offset: out_offset as isize,
            input_len,
            latency,
            latency_frac,
            up_shift,
            down_shift: down_shift_bits,
            input_delay,
            prev_input: vec![0.0; prev_input_len],
            current_input: vec![0.0; doubled_block_len],
            current_output: vec![0.0; doubled_block_len],
            spectrum: vec![Complex::default(); doubled_block_len / 2 + 1],
            scratch: vec![Complex::default(); scratch_len],
            in_data_left: 0,
            latency_left: 0,
            up_skip: 0,
            down_skip: 0,
        };
        convolver.clear();
        convolver
    }

    /// Takes `len` samples at the upsampled rate's worth of input.
    fn store_input(&mut self, input: &[f64], read: &mut usize, offset: usize, len: usize) {
        match self.up_shift {
            Some(shift) => {
                let count = len >> shift;
                let start = offset >> shift;
                self.current_input[start..start + count]
                    .copy_from_slice(&input[*read..*read + count]);
                *read += count;
            }
            None => self.store_zero_stuffed(input, read, offset, len),
        }
    }

    fn store_zero_stuffed(&mut self, input: &[f64], read: &mut usize, offset: usize, len: usize) {
        let factor = self.up_factor;
        let skipped = self.up_skip.min(len);
        self.up_skip -= skipped;
        let block = &mut self.current_input[offset..offset + len];
        let (zeros, rest) = block.split_at_mut(skipped);
        zeros.fill(0.0);

        let mut chunks = rest.chunks_exact_mut(factor);
        for chunk in &mut chunks {
            chunk[0] = input[*read];
            chunk[1..].fill(0.0);
            *read += 1;
        }
        let partial = chunks.into_remainder();
        if let Some((first, zeros)) = partial.split_first_mut() {
            *first = input[*read];
            *read += 1;
            zeros.fill(0.0);
            self.up_skip = factor - zeros.len() - 1;
        }
    }

    /// Filters the input block that is now full, into `current_input`. The
    /// previous input follows the new one, where it wraps around to come
    /// before it, as the kernel's tail needs.
    fn convolve_block(&mut self) {
        let new_len = self.input_len >> self.up_shift.unwrap_or(0);
        let prev_len = self.prev_input.len();
        self.current_input[new_len..new_len + prev_len].copy_from_slice(&self.prev_input);
        self.prev_input
            .copy_from_slice(&self.current_input[new_len - prev_len..new_len]);

        self.fft_in.forward(
            &mut self.current_input,
            &mut self.spectrum,
            &mut self.scratch,
        );
        if self.up_shift.is_some_and(|shift| shift > 0) {
            self.mirror_input_spectrum();
        }

        let nyquist_bin = self.fft_out.len() / 2;
        let nyquist_after_shift = self.down_shift.filter(|shift| *shift > 0).map(|_| {
            self.filter
                .nyquist_after_down_shift(self.spectrum[nyquist_bin])
        });
        self.filter.apply(&mut self.spectrum[..=nyquist_bin]);
        if let Some(nyquist) = nyquist_after_shift {
            self.spectrum[nyquist_bin] = Complex::new(nyquist, 0.0);
        }

        self.fft_out.inverse(
            &mut self.spectrum,
            &mut self.current_input,
            &mut self.scratch,
        );
    }

    /// The spectrum of the input with a zero after every sample
    fn mirror_input_spectrum(&mut self) {
        let input_bins = self.fft_in.len();
        for bin in 1..input_bins / 2 {
            self.spectrum[input_bins - bin] = self.spectrum[bin].conj();
        }
        self.spectrum[input_bins] = self.spectrum[0];
    }

    /// `offset` below zero wraps around to the end of the block.
    fn copy_to_output(&mut self, offset: isize, len: usize, output: &mut Vec<f64>) {
        let (mut offset, mut len) = (offset, len);
        if offset < 0 {
            let wrapped = offset.unsigned_abs();
            if len <= wrapped {
                offset += self.doubled_block_len as isize;
            } else {
                self.copy_to_output(offset + self.doubled_block_len as isize, wrapped, output);
                len -= wrapped;
                offset = 0;
            }
        }
        let mut offset = offset as usize;

        if self.latency_left != 0 {
            if self.latency_left >= len {
                self.latency_left -= len;
                return;
            }
            offset += self.latency_left;
            len -= self.latency_left;
            self.latency_left = 0;
        }

        let factor = self.down_factor;
        match self.down_shift {
            Some(shift) if shift > 0 => {
                let misaligned = offset & (factor - 1);
                if misaligned > 0 {
                    let skip = factor - misaligned;
                    if len <= skip {
                        return;
                    }
                    len -= skip;
                    offset += skip;
                }
                let count = (len + factor - 1) >> shift;
                let start = offset >> shift;
                output.extend_from_slice(&self.current_output[start..start + count]);
            }
            _ if factor > 1 => {
                let count = (len + factor - 1 - self.down_skip) / factor;
                let start = offset + self.down_skip;
                output.extend((0..count).map(|index| self.current_output[start + index * factor]));
                self.down_skip = self.down_skip + count * factor - len;
            }
            _ => output.extend_from_slice(&self.current_output[offset..offset + len]),
        }
    }
}

impl Processor for BlockConvolver {
    fn input_before_output(&self, position: i64) -> i64 {
        let up = self.up_factor as f64;
        let down = self.down_factor as f64;
        ((self.latency as f64 + position as f64 * down) / up + self.latency_frac * down / up) as i64
    }

    fn latency_frac(&self) -> f64 {
        self.latency_frac
    }

    fn max_output_len(&self, input_len: usize) -> usize {
        (input_len * self.up_factor).div_ceil(self.down_factor)
    }

    fn clear(&mut self) {
        self.prev_input.fill(0.0);
        self.latency_left = self.latency;
        self.current_input[..self.input_delay].fill(0.0);
        self.in_data_left = self.input_len - self.input_delay;
        self.up_skip = 0;
        self.down_skip = 0;
    }

    fn process(&mut self, input: &[f64], output: &mut Vec<f64>) -> usize {
        let mut read = 0;
        let mut left = input.len() * self.up_factor;
        while left > 0 {
            let offset = self.input_len - self.in_data_left;
            let output_offset = offset as isize - self.out_offset;
            if left < self.in_data_left {
                self.in_data_left -= left;
                self.store_input(input, &mut read, offset, left);
                self.copy_to_output(output_offset, left, output);
                break;
            }
            let taken = self.in_data_left;
            left -= taken;
            self.in_data_left = self.input_len;
            self.store_input(input, &mut read, offset, taken);
            self.convolve_block();
            self.copy_to_output(output_offset, taken, output);
            std::mem::swap(&mut self.current_input, &mut self.current_output);
        }
        0
    }
}
