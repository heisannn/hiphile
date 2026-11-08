use std::fmt::Debug;
use std::ops::{Add, AddAssign, Mul, Sub};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use realfft::{ComplexToReal, FftNum, RealFftPlanner, RealToComplex};

use crate::simd;

static PLANNER_32: LazyLock<Mutex<RealFftPlanner<f32>>> =
    LazyLock::new(|| Mutex::new(RealFftPlanner::new()));
static PLANNER_64: LazyLock<Mutex<RealFftPlanner<f64>>> =
    LazyLock::new(|| Mutex::new(RealFftPlanner::new()));

/// The floating point that an engine computes in
pub(crate) trait Real:
    FftNum + Default + Debug + Add<Output = Self> + AddAssign + Sub<Output = Self> + Mul<Output = Self>
{
    const ZERO: Self;

    fn from_double(value: f64) -> Self;

    fn to_double(self) -> f64;

    /// From the planner shared by every resampler, which keeps what it has
    /// worked out
    fn plan(len: usize) -> (Arc<dyn RealToComplex<Self>>, Arc<dyn ComplexToReal<Self>>);

    /// What [`Self::dot_padded`] and the others take lengths in multiples of
    const PADDING: usize;

    /// The dot product of `a` and `b` of the same length, in a fixed order
    fn dot_padded(a: &[Self], b: &[Self]) -> Self;

    /// Into each `out[k]`, [`Self::dot_padded`] of `taps` and of the
    /// `taps.len()` samples of `input` from `first + k * stride`
    fn fir(taps: &[Self], input: &[Self], first: usize, stride: usize, out: &mut [Self]);

    /// Into each of `out`, [`Self::dot_padded`] of `span` taps of `taps` from
    /// the matching `tap_starts` and of `input` from the matching
    /// `input_starts`
    fn dots(
        taps: &[Self],
        tap_starts: &[usize],
        input: &[Self],
        input_starts: &[usize],
        span: usize,
        out: &mut [Self],
    );
}

impl Real for f32 {
    const ZERO: Self = 0.0;
    const PADDING: usize = simd::F32_VECTOR;

    fn from_double(value: f64) -> Self {
        value as f32
    }

    fn to_double(self) -> f64 {
        f64::from(self)
    }

    fn plan(len: usize) -> (Arc<dyn RealToComplex<Self>>, Arc<dyn ComplexToReal<Self>>) {
        let mut planner = PLANNER_32.lock().unwrap_or_else(PoisonError::into_inner);
        (planner.plan_fft_forward(len), planner.plan_fft_inverse(len))
    }

    fn dot_padded(a: &[Self], b: &[Self]) -> Self {
        simd::dot_f32(a, b)
    }

    fn dots(
        taps: &[Self],
        tap_starts: &[usize],
        input: &[Self],
        input_starts: &[usize],
        span: usize,
        out: &mut [Self],
    ) {
        simd::dots_f32(taps, tap_starts, input, input_starts, span, out);
    }

    fn fir(taps: &[Self], input: &[Self], first: usize, stride: usize, out: &mut [Self]) {
        simd::fir_f32(taps, input, first, stride, out);
    }
}

impl Real for f64 {
    const ZERO: Self = 0.0;
    const PADDING: usize = simd::F64_VECTOR;

    fn from_double(value: f64) -> Self {
        value
    }

    fn to_double(self) -> f64 {
        self
    }

    fn plan(len: usize) -> (Arc<dyn RealToComplex<Self>>, Arc<dyn ComplexToReal<Self>>) {
        let mut planner = PLANNER_64.lock().unwrap_or_else(PoisonError::into_inner);
        (planner.plan_fft_forward(len), planner.plan_fft_inverse(len))
    }

    fn dot_padded(a: &[Self], b: &[Self]) -> Self {
        simd::dot_f64(a, b)
    }

    fn dots(
        taps: &[Self],
        tap_starts: &[usize],
        input: &[Self],
        input_starts: &[usize],
        span: usize,
        out: &mut [Self],
    ) {
        simd::dots_f64(taps, tap_starts, input, input_starts, span, out);
    }

    fn fir(taps: &[Self], input: &[Self], first: usize, stride: usize, out: &mut [Self]) {
        simd::fir_f64(taps, input, first, stride, out);
    }
}

/// In a fixed order, so the result does not depend on where `a` came from,
/// with four sums that the compiler can keep in one vector register
pub(crate) fn dot<T: Real>(a: &[T], b: &[T]) -> T {
    let (a_chunks, a_rest) = a.as_chunks::<4>();
    let (b_chunks, b_rest) = b[..a.len()].as_chunks::<4>();
    let mut sums = [T::ZERO; 4];
    for (a, b) in a_chunks.iter().zip(b_chunks) {
        for ((sum, a), b) in sums.iter_mut().zip(a).zip(b) {
            *sum += *a * *b;
        }
    }
    for (a, b) in a_rest.iter().zip(b_rest) {
        sums[0] += *a * *b;
    }
    (sums[0] + sums[2]) + (sums[1] + sums[3])
}
