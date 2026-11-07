//! Doubling and halving by sparse symmetric filters: half of their taps are
//! zero, and those left come in pairs around the centre.

mod downsampler;
mod kernels;
mod upsampler;

pub(crate) use downsampler::Downsampler;
pub(crate) use upsampler::Upsampler;

use kernels::{HALF, Kernel, THIRD};

/// The shortest one reaching `attenuation`, or else the longest. `is_third` is
/// for a filter that only has to pass a third of the band.
fn kernel_taps(attenuation: f64, steep_index: usize, is_third: bool) -> &'static [f64] {
    let families = if is_third { &THIRD } else { &HALF };
    let family: &[Kernel] = families[steep_index.min(families.len() - 1)];
    let kernel = family
        .iter()
        .find(|kernel| kernel.attenuation >= attenuation)
        .or(family.last())
        .expect("every family has kernels");
    kernel.taps
}

/// A function per length of kernel, which the compiler can unroll and vectorize
/// as it cannot one that loops over any length
macro_rules! for_each_kernel_len {
    ($function:ident) => {
        [
            $function::<1>,
            $function::<2>,
            $function::<3>,
            $function::<4>,
            $function::<5>,
            $function::<6>,
            $function::<7>,
            $function::<8>,
            $function::<9>,
            $function::<10>,
            $function::<11>,
            $function::<12>,
            $function::<13>,
            $function::<14>,
        ]
    };
}
use for_each_kernel_len;

/// `window` is the samples around the centre, as many on each side as there are
/// taps. In two lanes, as a single sum has to keep the order of its additions,
/// which rules out vectorizing it.
#[inline(always)]
fn symmetric_sum<const N: usize>(taps: &[f64; N], window: &[f64]) -> f64 {
    let (left, right) = window.split_at(N);
    let left: &[f64; N] = left.try_into().expect("the window holds both sides");
    let right: &[f64; N] = right.try_into().expect("the window holds both sides");
    let mut sums = [0.0; 2];
    let mut index = 0;
    while index + 1 < N {
        sums[0] += taps[index] * (right[index] + left[N - 1 - index]);
        sums[1] += taps[index + 1] * (right[index + 1] + left[N - 2 - index]);
        index += 2;
    }
    if N % 2 == 1 {
        sums[0] += taps[N - 1] * (right[N - 1] + left[0]);
    }
    sums[0] + sums[1]
}
