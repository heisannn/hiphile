//! Overlap-save convolution with a long filter, which also doubles or halves
//! the rate

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealToComplex};

use crate::real::Real;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Factor {
    /// Doubles the rate. The transform of the input block is repeated to make
    /// that of the input with a zero after each sample.
    Up2,
    /// Halves the rate. The two halves of the filtered spectrum are added,
    /// which makes the spectrum of every other sample.
    Down2,
}

/// What every channel shares
pub(crate) struct FftFilter<T> {
    factor: Factor,
    forward: Arc<dyn RealToComplex<T>>,
    inverse: Arc<dyn ComplexToReal<T>>,
    /// Over the transform at the filter's rate, scaled for both transforms
    spectrum: Vec<Complex<T>>,
    /// Input samples that each block keeps from the one before
    history: usize,
    /// New input samples in each block
    block: usize,
    /// Output samples that come before the first one aligned with the input
    delay: usize,
}

impl<T: Real> FftFilter<T> {
    /// `taps` at the filter's rate, which is the output's when doubling and
    /// the input's when halving, with a gain of 1 and the sample at `delay`
    /// aligned with the input. `len` is the length of the transform at the
    /// filter's rate.
    pub(crate) fn new(factor: Factor, taps: &[f64], delay: usize, len: usize) -> Self {
        let mut taps = taps.to_vec();
        let mut delay = delay;
        if factor == Factor::Down2 && delay % 2 == 1 {
            // Keeps the kept samples on even positions of each block
            taps.insert(0, 0.0);
            delay += 1;
        }
        let (history, block, gain) = match factor {
            Factor::Up2 => {
                let history = taps.len() / 2;
                (history, len / 2 - history, 2.0)
            }
            Factor::Down2 => {
                let history = (taps.len() - 1).next_multiple_of(2);
                (history, len - history, 1.0)
            }
        };
        assert!(block > 0, "the transform is longer than the filter");
        let spectrum = filter_spectrum(&taps, len, gain / len as f64);
        let (forward, inverse) = match factor {
            Factor::Up2 => (T::plan(len / 2).0, T::plan(len).1),
            Factor::Down2 => (T::plan(len).0, T::plan(len / 2).1),
        };
        let delay = match factor {
            Factor::Up2 => delay,
            Factor::Down2 => delay / 2,
        };
        Self {
            factor,
            forward,
            inverse,
            spectrum,
            history,
            block,
            delay,
        }
    }
}

/// `taps` padded to `len`, transformed in 64-bit floating point and scaled
fn filter_spectrum<T: Real>(taps: &[f64], len: usize, scale: f64) -> Vec<Complex<T>> {
    let (forward, _) = f64::plan(len);
    let mut time = vec![0.0; len];
    time[..taps.len()].copy_from_slice(taps);
    let mut spectrum = forward.make_output_vec();
    forward
        .process(&mut time, &mut spectrum)
        .expect("the buffers are the transform's own");
    spectrum
        .iter()
        .map(|bin| {
            Complex::new(
                T::from_double(bin.re * scale),
                T::from_double(bin.im * scale),
            )
        })
        .collect()
}

/// One channel's state
pub(crate) struct FftStage<T> {
    filter: Arc<FftFilter<T>>,
    /// The block being filled, after the history from the block before. The
    /// transform overwrites it.
    time: Vec<T>,
    filled: usize,
    /// The next block, which takes its history before the transform runs
    next: Vec<T>,
    spectrum: Vec<Complex<T>>,
    product: Vec<Complex<T>>,
    output: Vec<T>,
    scratch: Vec<Complex<T>>,
    skip: usize,
}

impl<T: Real> FftStage<T> {
    pub(crate) fn new(filter: Arc<FftFilter<T>>) -> Self {
        let scratch_len = filter
            .forward
            .get_scratch_len()
            .max(filter.inverse.get_scratch_len());
        let time = filter.forward.make_input_vec();
        debug_assert_eq!(time.len(), filter.history + filter.block);
        Self {
            next: time.clone(),
            time,
            filled: filter.history,
            spectrum: filter.forward.make_output_vec(),
            product: filter.inverse.make_input_vec(),
            output: filter.inverse.make_output_vec(),
            scratch: vec![Complex::new(T::ZERO, T::ZERO); scratch_len],
            skip: filter.delay,
            filter,
        }
    }

    pub(crate) fn push(&mut self, mut input: &[T], out: &mut Vec<T>) {
        let (history, block) = (self.filter.history, self.filter.block);
        while !input.is_empty() {
            let take = (self.time.len() - self.filled).min(input.len());
            self.time[self.filled..][..take].copy_from_slice(&input[..take]);
            self.filled += take;
            input = &input[take..];
            if self.filled == self.time.len() {
                self.next[..history].copy_from_slice(&self.time[block..]);
                self.run_block(out);
                std::mem::swap(&mut self.time, &mut self.next);
                self.filled = history;
            }
        }
    }

    /// Once this many more samples have come, every block with the earlier
    /// ones in it has been processed
    pub(crate) fn memory(&self) -> usize {
        self.filter.history + self.filter.block
    }

    pub(crate) fn clear(&mut self) {
        self.time[..self.filter.history].fill(T::ZERO);
        self.filled = self.filter.history;
        self.skip = self.filter.delay;
    }

    fn run_block(&mut self, out: &mut Vec<T>) {
        let filter = &*self.filter;
        let result = filter.forward.process_with_scratch(
            &mut self.time,
            &mut self.spectrum,
            &mut self.scratch,
        );
        debug_assert!(result.is_ok(), "{result:?}");
        let spectrum = &mut self.spectrum[..];
        match filter.factor {
            Factor::Up2 => {
                // The bins up to the input's Nyquist frequency, then the same
                // bins again backwards and conjugated, ending at 0 Hz
                let half = spectrum.len() - 1;
                let (low, high) = self.product.split_at_mut(half + 1);
                let (gain_low, gain_high) = filter.spectrum.split_at(half + 1);
                for ((product, bin), gain) in low.iter_mut().zip(&*spectrum).zip(gain_low) {
                    *product = *bin * *gain;
                }
                let mirrored = spectrum[..half].iter().rev().map(Complex::conj);
                for ((product, bin), gain) in high.iter_mut().zip(mirrored).zip(gain_high) {
                    *product = bin * *gain;
                }
            }
            Factor::Down2 => {
                for (bin, gain) in spectrum.iter_mut().zip(&filter.spectrum) {
                    *bin = *bin * *gain;
                }
                let quarter = self.product.len() - 1;
                let (low, high) = (&spectrum[..=quarter], &spectrum[quarter..]);
                for ((product, low), high) in
                    self.product.iter_mut().zip(low).zip(high.iter().rev())
                {
                    *product = *low + high.conj();
                }
            }
        }
        let last = self.product.len() - 1;
        self.product[0].im = T::ZERO;
        self.product[last].im = T::ZERO;
        let result = filter.inverse.process_with_scratch(
            &mut self.product,
            &mut self.output,
            &mut self.scratch,
        );
        debug_assert!(result.is_ok(), "{result:?}");
        let first = match filter.factor {
            Factor::Up2 => 2 * filter.history,
            Factor::Down2 => filter.history / 2,
        };
        let valid = &self.output[first..];
        let skipped = self.skip.min(valid.len());
        self.skip -= skipped;
        out.extend_from_slice(&valid[skipped..]);
    }
}
