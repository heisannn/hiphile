use std::num::NonZeroUsize;
use std::ops::RangeInclusive;
use std::thread;

use crate::bits;

const MIN_DFT_SIZES: RangeInclusive<u32> = 8..=15;
const LARGE_DFT_SIZES: RangeInclusive<u32> = 8..=20;

/// How a filter interpolates between the phases it stores
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CoefInterpolation {
    /// For a ratio with a short fraction, every phase that it takes, with no
    /// interpolation, if they fit in [`RuntimeSpec::coef_size_kbytes`].
    /// Otherwise [`Self::Low`] if its table fits there, or else
    /// [`Self::High`].
    #[default]
    Auto,
    /// Linear: less computation and more memory. Past 8 MiB of memory it is
    /// [`Self::High`].
    Low,
    /// Cubic: more computation and less memory
    High,
}

/// Choices of speed and memory, which leave the length of the output alone
/// and change its values by no more than rounding
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct RuntimeSpec {
    /// From 8 to 15: the base-2 logarithm of the least length of a DFT. A
    /// value outside is taken as the default, 10.
    pub log2_min_dft_size: u32,
    /// From 8 to 20: the base-2 logarithm of the length over which a DFT is
    /// made no longer for speed, only as long as the filter needs. A value
    /// outside is taken as the default, 17.
    pub log2_large_dft_size: u32,
    /// The most memory, in kB, for the table of a filter's phases before it
    /// keeps fewer and interpolates between them
    pub coef_size_kbytes: usize,
    /// Threads that share out the work of a call that has enough of it. 0
    /// is as many as there are cores. The output is the same with any number.
    pub num_threads: usize,
    pub coef_interpolation: CoefInterpolation,
}

impl RuntimeSpec {
    pub const fn new(num_threads: usize) -> Self {
        Self {
            log2_min_dft_size: 10,
            log2_large_dft_size: 17,
            coef_size_kbytes: 400,
            num_threads,
            coef_interpolation: CoefInterpolation::Auto,
        }
    }

    /// With the values of libsoxr's environment variables that are set and
    /// can be read: `SOXR_MIN_DFT_SIZE`, `SOXR_LARGE_DFT_SIZE`,
    /// `SOXR_COEFS_SIZE`, `SOXR_NUM_THREADS` and `SOXR_COEF_INTERP` (0 to 3,
    /// where 1 is [`CoefInterpolation::Auto`] as 0 is). A library that reads
    /// the environment can surprise the program it is in, so this happens
    /// only when asked for.
    pub fn with_env(self) -> Self {
        self.with_vars(|name| std::env::var(name).ok())
    }

    pub(crate) fn with_vars(mut self, var: impl Fn(&str) -> Option<String>) -> Self {
        let number = |name: &str| var(name).and_then(|value| value.trim().parse::<u64>().ok());
        if let Some(size) = number("SOXR_MIN_DFT_SIZE").and_then(|n| u32::try_from(n).ok()) {
            self.log2_min_dft_size = size;
        }
        if let Some(size) = number("SOXR_LARGE_DFT_SIZE").and_then(|n| u32::try_from(n).ok()) {
            self.log2_large_dft_size = size;
        }
        if let Some(size) = number("SOXR_COEFS_SIZE").and_then(|n| usize::try_from(n).ok()) {
            self.coef_size_kbytes = size;
        }
        if let Some(threads) = number("SOXR_NUM_THREADS").and_then(|n| usize::try_from(n).ok()) {
            self.num_threads = threads;
        }
        match number("SOXR_COEF_INTERP").and_then(|n| u32::try_from(n).ok()) {
            Some(bits::COEF_INTERP_AUTO | 1) => self.coef_interpolation = CoefInterpolation::Auto,
            Some(bits::COEF_INTERP_LOW) => self.coef_interpolation = CoefInterpolation::Low,
            Some(bits::COEF_INTERP_HIGH) => self.coef_interpolation = CoefInterpolation::High,
            _ => {}
        }
        self
    }

    /// The base-2 logarithms of the least and the large length of a DFT, in
    /// their ranges
    pub(crate) fn dft_sizes(&self) -> (u32, u32) {
        let default = Self::new(1);
        let within = |size: u32, sizes: RangeInclusive<u32>, default: u32| {
            Some(size)
                .filter(|size| sizes.contains(size))
                .unwrap_or(default)
        };
        (
            within(
                self.log2_min_dft_size,
                MIN_DFT_SIZES,
                default.log2_min_dft_size,
            ),
            within(
                self.log2_large_dft_size,
                LARGE_DFT_SIZES,
                default.log2_large_dft_size,
            ),
        )
    }

    /// Of [`Self::num_threads`], with 0 resolved
    pub(crate) fn threads(&self) -> usize {
        match self.num_threads {
            0 => thread::available_parallelism().map_or(1, NonZeroUsize::get),
            threads => threads,
        }
    }
}

impl Default for RuntimeSpec {
    fn default() -> Self {
        Self::new(1)
    }
}

#[cfg(test)]
mod tests;
