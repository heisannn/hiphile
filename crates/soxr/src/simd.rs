//! The inner loops of the filters, in vector instructions where the target has
//! them at compile time: NEON on AArch64 and SSE2 on x86-64. Elsewhere the
//! scalar loops run.
//!
//! The sums of a dot product run in four lanes, lane `i` taking the products
//! of the indices `i` mod 4, and end as `(lane 0 + lane 2) + (lane 1 + lane 3)`.
//! Lengths are whole vectors: multiples of 4 for `f32` and of 2 for `f64`.
//! Every function of a build sums an output that way, so the output has the
//! same bits whichever function computes it and wherever it falls in a batch.
//! NEON rounds each product and its sum once, by fused multiply-add, and SSE2
//! and the scalar loops round both, so NEON's bits differ from theirs.

#![allow(unsafe_code)]

/// The dot product of `a` and `b`, of the same length
pub(crate) fn dot_f32(a: &[f32], b: &[f32]) -> f32 {
    check(a, b, F32_VECTOR);
    imp::dot_f32(a, b)
}

/// As [`dot_f32`]
pub(crate) fn dot_f64(a: &[f64], b: &[f64]) -> f64 {
    check(a, b, F64_VECTOR);
    imp::dot_f64(a, b)
}

/// Into each of `out`, the dot product of `span` taps of `taps` from the
/// matching `tap_starts` and of `input` from the matching `input_starts`, as
/// [`dot_f32`] computes it
pub(crate) fn dots_f32(
    taps: &[f32],
    tap_starts: &[usize],
    input: &[f32],
    input_starts: &[usize],
    span: usize,
    out: &mut [f32],
) {
    check_batch(
        taps,
        tap_starts,
        input,
        input_starts,
        span,
        out.len(),
        F32_VECTOR,
    );
    imp::dots_f32(taps, tap_starts, input, input_starts, span, out);
}

/// As [`dots_f32`]
pub(crate) fn dots_f64(
    taps: &[f64],
    tap_starts: &[usize],
    input: &[f64],
    input_starts: &[usize],
    span: usize,
    out: &mut [f64],
) {
    check_batch(
        taps,
        tap_starts,
        input,
        input_starts,
        span,
        out.len(),
        F64_VECTOR,
    );
    imp::dots_f64(taps, tap_starts, input, input_starts, span, out);
}

/// Into each `out[k]`, the dot product of `taps` and of the `taps.len()`
/// samples of `input` from `first + k * stride`, as [`dot_f32`] computes it
pub(crate) fn fir_f32(taps: &[f32], input: &[f32], first: usize, stride: usize, out: &mut [f32]) {
    check_fir(taps, input, first, stride, out.len(), F32_VECTOR);
    imp::fir_f32(taps, input, first, stride, out);
}

/// As [`fir_f32`]
pub(crate) fn fir_f64(taps: &[f64], input: &[f64], first: usize, stride: usize, out: &mut [f64]) {
    check_fir(taps, input, first, stride, out.len(), F64_VECTOR);
    imp::fir_f64(taps, input, first, stride, out);
}

/// Elements in a vector
pub(crate) const F32_VECTOR: usize = 4;
pub(crate) const F64_VECTOR: usize = 2;

/// The vector loops read every window in whole vectors up to its end
fn check_fir<T>(
    taps: &[T],
    input: &[T],
    first: usize,
    stride: usize,
    outputs: usize,
    vector: usize,
) {
    assert!(taps.len().is_multiple_of(vector));
    if let Some(last) = outputs.checked_sub(1) {
        let end = last
            .checked_mul(stride)
            .and_then(|offset| offset.checked_add(first))
            .and_then(|start| start.checked_add(taps.len()));
        assert!(end.is_some_and(|end| end <= input.len()));
    }
}

/// The vector loops read every window in whole vectors up to its end
fn check_batch<T>(
    taps: &[T],
    tap_starts: &[usize],
    input: &[T],
    input_starts: &[usize],
    span: usize,
    outputs: usize,
    vector: usize,
) {
    assert!(span.is_multiple_of(vector));
    assert!(tap_starts.len() == outputs && input_starts.len() == outputs);
    let within = |starts: &[usize], len: usize| {
        starts
            .iter()
            .copied()
            .max()
            .is_none_or(|last| span <= len && last <= len - span)
    };
    assert!(within(tap_starts, taps.len()) && within(input_starts, input.len()));
}

/// The vector loops read every slice in whole vectors up to the length of
/// `a`, so these must hold for them to stay in bounds
fn check<T>(a: &[T], b: &[T], vector: usize) {
    assert!(a.len() == b.len() && a.len().is_multiple_of(vector));
}

/// The loops that every target can run
#[allow(dead_code)]
mod scalar {
    use crate::real::Real;

    pub(super) fn dot<T: Real>(a: &[T], b: &[T]) -> T {
        let (a_chunks, a_rest) = a.as_chunks::<4>();
        let (b_chunks, b_rest) = b.as_chunks::<4>();
        let mut sums = [T::ZERO; 4];
        for (a, b) in a_chunks.iter().zip(b_chunks) {
            for ((sum, a), b) in sums.iter_mut().zip(a).zip(b) {
                *sum += *a * *b;
            }
        }
        for ((sum, a), b) in sums.iter_mut().zip(a_rest).zip(b_rest) {
            *sum += *a * *b;
        }
        (sums[0] + sums[2]) + (sums[1] + sums[3])
    }

    pub(super) fn fir<T: Real>(
        taps: &[T],
        input: &[T],
        first: usize,
        stride: usize,
        out: &mut [T],
    ) {
        for (index, out) in out.iter_mut().enumerate() {
            *out = dot(taps, &input[first + index * stride..][..taps.len()]);
        }
    }

    pub(super) fn dots<T: Real>(
        taps: &[T],
        tap_starts: &[usize],
        input: &[T],
        input_starts: &[usize],
        span: usize,
        out: &mut [T],
    ) {
        for ((out, tap), at) in out.iter_mut().zip(tap_starts).zip(input_starts) {
            *out = dot(&taps[*tap..][..span], &input[*at..][..span]);
        }
    }
}

#[cfg(not(any(
    all(target_arch = "aarch64", target_feature = "neon"),
    all(target_arch = "x86_64", target_feature = "sse2"),
)))]
mod imp {
    pub(super) fn dot_f32(a: &[f32], b: &[f32]) -> f32 {
        super::scalar::dot(a, b)
    }

    pub(super) fn dot_f64(a: &[f64], b: &[f64]) -> f64 {
        super::scalar::dot(a, b)
    }

    pub(super) fn dots_f32(
        taps: &[f32],
        tap_starts: &[usize],
        input: &[f32],
        input_starts: &[usize],
        span: usize,
        out: &mut [f32],
    ) {
        super::scalar::dots(taps, tap_starts, input, input_starts, span, out);
    }

    pub(super) fn fir_f32(
        taps: &[f32],
        input: &[f32],
        first: usize,
        stride: usize,
        out: &mut [f32],
    ) {
        super::scalar::fir(taps, input, first, stride, out);
    }

    pub(super) fn fir_f64(
        taps: &[f64],
        input: &[f64],
        first: usize,
        stride: usize,
        out: &mut [f64],
    ) {
        super::scalar::fir(taps, input, first, stride, out);
    }

    pub(super) fn dots_f64(
        taps: &[f64],
        tap_starts: &[usize],
        input: &[f64],
        input_starts: &[usize],
        span: usize,
        out: &mut [f64],
    ) {
        super::scalar::dots(taps, tap_starts, input, input_starts, span, out);
    }
}

// In the two modules below, the safe functions call the vector ones, whose
// `#[target_feature]` the `cfg` of the module guarantees at compile time. In
// those, each load reads one vector, 4 elements of `f32` or 2 of `f64`, from
// the start of a slice or window plus an offset that is a multiple of that
// count below the length of `a`, of `taps` or `span`. The public functions
// above check that every slice is as long as `a`, that every window lies
// within its slice, and that those lengths are multiples of that count, so
// each load lies within its slice. The loads take no alignment.
#[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
mod imp {
    use std::arch::aarch64::*;

    pub(super) fn dot_f32(a: &[f32], b: &[f32]) -> f32 {
        // SAFETY: NEON is enabled at compile time
        unsafe { neon_dot_f32(a, b) }
    }

    pub(super) fn dot_f64(a: &[f64], b: &[f64]) -> f64 {
        // SAFETY: NEON is enabled at compile time
        unsafe { neon_dot_f64(a, b) }
    }

    pub(super) fn dots_f32(
        taps: &[f32],
        tap_starts: &[usize],
        input: &[f32],
        input_starts: &[usize],
        span: usize,
        out: &mut [f32],
    ) {
        // SAFETY: NEON is enabled at compile time
        unsafe { neon_dots_f32(taps, tap_starts, input, input_starts, span, out) }
    }

    pub(super) fn dots_f64(
        taps: &[f64],
        tap_starts: &[usize],
        input: &[f64],
        input_starts: &[usize],
        span: usize,
        out: &mut [f64],
    ) {
        // SAFETY: NEON is enabled at compile time
        unsafe { neon_dots_f64(taps, tap_starts, input, input_starts, span, out) }
    }

    pub(super) fn fir_f32(
        taps: &[f32],
        input: &[f32],
        first: usize,
        stride: usize,
        out: &mut [f32],
    ) {
        // SAFETY: NEON is enabled at compile time
        unsafe { neon_fir_f32(taps, input, first, stride, out) }
    }

    pub(super) fn fir_f64(
        taps: &[f64],
        input: &[f64],
        first: usize,
        stride: usize,
        out: &mut [f64],
    ) {
        // SAFETY: NEON is enabled at compile time
        unsafe { neon_fir_f64(taps, input, first, stride, out) }
    }

    #[target_feature(enable = "neon")]
    fn neon_fir_f32(taps: &[f32], input: &[f32], first: usize, stride: usize, out: &mut [f32]) {
        let at = |index: usize| input.as_ptr().wrapping_add(first + index * stride);
        let groups = out.len() / 4;
        for group in 0..groups {
            let start = group * 4;
            let inputs = [at(start), at(start + 1), at(start + 2), at(start + 3)];
            let mut sums = [vdupq_n_f32(0.0); 4];
            for index in (0..taps.len()).step_by(4) {
                // SAFETY: within the slices, as the module says
                let tap = unsafe { vld1q_f32(taps.as_ptr().add(index)) };
                for lane in 0..4 {
                    // SAFETY: within the slices, as the module says
                    let sample = unsafe { vld1q_f32(inputs[lane].add(index)) };
                    sums[lane] = vfmaq_f32(sums[lane], tap, sample);
                }
            }
            for lane in 0..4 {
                out[start + lane] = reduce_f32(sums[lane]);
            }
        }
        for (index, out) in out.iter_mut().enumerate().skip(groups * 4) {
            let mut sum = vdupq_n_f32(0.0);
            for offset in (0..taps.len()).step_by(4) {
                sum = add_products_f32(sum, taps.as_ptr(), at(index), offset);
            }
            *out = reduce_f32(sum);
        }
    }

    #[target_feature(enable = "neon")]
    fn neon_fir_f64(taps: &[f64], input: &[f64], first: usize, stride: usize, out: &mut [f64]) {
        let at = |index: usize| input.as_ptr().wrapping_add(first + index * stride);
        let tap = |offset: usize| {
            // SAFETY: within the slices, as the module says
            unsafe { vld1q_f64(taps.as_ptr().add(offset)) }
        };
        let whole = taps.len() - taps.len() % 4;
        let groups = out.len() / 4;
        for group in 0..groups {
            let start = group * 4;
            let inputs = [at(start), at(start + 1), at(start + 2), at(start + 3)];
            let mut sums = [[vdupq_n_f64(0.0); 2]; 4];
            for index in (0..whole).step_by(4) {
                for (half, offset) in [(0, index), (1, index + 2)] {
                    let tap = tap(offset);
                    for lane in 0..4 {
                        sums[lane][half] =
                            add_pair_f64(sums[lane][half], tap, inputs[lane], offset);
                    }
                }
            }
            if whole < taps.len() {
                let tap = tap(whole);
                for lane in 0..4 {
                    sums[lane][0] = add_pair_f64(sums[lane][0], tap, inputs[lane], whole);
                }
            }
            for lane in 0..4 {
                out[start + lane] = reduce_f64(sums[lane]);
            }
        }
        for (index, out) in out.iter_mut().enumerate().skip(groups * 4) {
            *out = reduce_f64(sum_products_f64(taps.as_ptr(), at(index), taps.len()));
        }
    }

    #[target_feature(enable = "neon")]
    fn neon_dot_f32(a: &[f32], b: &[f32]) -> f32 {
        let mut sum = vdupq_n_f32(0.0);
        for index in (0..a.len()).step_by(4) {
            sum = add_products_f32(sum, a.as_ptr(), b.as_ptr(), index);
        }
        reduce_f32(sum)
    }

    #[target_feature(enable = "neon")]
    fn neon_dots_f32(
        taps: &[f32],
        tap_starts: &[usize],
        input: &[f32],
        input_starts: &[usize],
        span: usize,
        out: &mut [f32],
    ) {
        let tap = |index: usize| taps.as_ptr().wrapping_add(tap_starts[index]);
        let at = |index: usize| input.as_ptr().wrapping_add(input_starts[index]);
        let groups = out.len() / 4;
        for group in 0..groups {
            let first = group * 4;
            let taps = [tap(first), tap(first + 1), tap(first + 2), tap(first + 3)];
            let inputs = [at(first), at(first + 1), at(first + 2), at(first + 3)];
            let mut sums = [vdupq_n_f32(0.0); 4];
            for index in (0..span).step_by(4) {
                for lane in 0..4 {
                    sums[lane] = add_products_f32(sums[lane], taps[lane], inputs[lane], index);
                }
            }
            for lane in 0..4 {
                out[first + lane] = reduce_f32(sums[lane]);
            }
        }
        for (index, out) in out.iter_mut().enumerate().skip(groups * 4) {
            let mut sum = vdupq_n_f32(0.0);
            for offset in (0..span).step_by(4) {
                sum = add_products_f32(sum, tap(index), at(index), offset);
            }
            *out = reduce_f32(sum);
        }
    }

    #[inline]
    #[target_feature(enable = "neon")]
    fn add_products_f32(
        sum: float32x4_t,
        a: *const f32,
        b: *const f32,
        index: usize,
    ) -> float32x4_t {
        // SAFETY: within the slices, as the module says
        let (x, y) = unsafe { (vld1q_f32(a.add(index)), vld1q_f32(b.add(index))) };
        vfmaq_f32(sum, x, y)
    }

    #[inline]
    #[target_feature(enable = "neon")]
    fn reduce_f32(sum: float32x4_t) -> f32 {
        let pair = vadd_f32(vget_low_f32(sum), vget_high_f32(sum));
        vget_lane_f32::<0>(pair) + vget_lane_f32::<1>(pair)
    }

    #[target_feature(enable = "neon")]
    fn neon_dot_f64(a: &[f64], b: &[f64]) -> f64 {
        reduce_f64(sum_products_f64(a.as_ptr(), b.as_ptr(), a.len()))
    }

    #[target_feature(enable = "neon")]
    fn neon_dots_f64(
        taps: &[f64],
        tap_starts: &[usize],
        input: &[f64],
        input_starts: &[usize],
        span: usize,
        out: &mut [f64],
    ) {
        let tap = |index: usize| taps.as_ptr().wrapping_add(tap_starts[index]);
        let at = |index: usize| input.as_ptr().wrapping_add(input_starts[index]);
        let whole = span - span % 4;
        let groups = out.len() / 4;
        for group in 0..groups {
            let first = group * 4;
            let taps = [tap(first), tap(first + 1), tap(first + 2), tap(first + 3)];
            let inputs = [at(first), at(first + 1), at(first + 2), at(first + 3)];
            let mut sums = [[vdupq_n_f64(0.0); 2]; 4];
            for index in (0..whole).step_by(4) {
                for lane in 0..4 {
                    for (half, offset) in [(0, index), (1, index + 2)] {
                        let tap = load_f64(taps[lane], offset);
                        sums[lane][half] =
                            add_pair_f64(sums[lane][half], tap, inputs[lane], offset);
                    }
                }
            }
            if whole < span {
                for lane in 0..4 {
                    let tap = load_f64(taps[lane], whole);
                    sums[lane][0] = add_pair_f64(sums[lane][0], tap, inputs[lane], whole);
                }
            }
            for lane in 0..4 {
                out[first + lane] = reduce_f64(sums[lane]);
            }
        }
        for (index, out) in out.iter_mut().enumerate().skip(groups * 4) {
            *out = reduce_f64(sum_products_f64(tap(index), at(index), span));
        }
    }

    #[inline]
    #[target_feature(enable = "neon")]
    fn load_f64(a: *const f64, offset: usize) -> float64x2_t {
        // SAFETY: within the slices, as the module says
        unsafe { vld1q_f64(a.add(offset)) }
    }

    /// `sum` plus the products of `x` and of the two elements of `b` from
    /// `offset`
    #[inline]
    #[target_feature(enable = "neon")]
    fn add_pair_f64(sum: float64x2_t, x: float64x2_t, b: *const f64, offset: usize) -> float64x2_t {
        vfmaq_f64(sum, x, load_f64(b, offset))
    }

    /// Of the first `len` elements of `a` and `b`, lanes 0 and 1 in the first
    /// sum and 2 and 3 in the second
    #[inline]
    #[target_feature(enable = "neon")]
    fn sum_products_f64(a: *const f64, b: *const f64, len: usize) -> [float64x2_t; 2] {
        let whole = len - len % 4;
        let mut sums = [vdupq_n_f64(0.0); 2];
        for index in (0..whole).step_by(4) {
            for (half, sum) in sums.iter_mut().enumerate() {
                let offset = index + 2 * half;
                *sum = add_pair_f64(*sum, load_f64(a, offset), b, offset);
            }
        }
        if whole < len {
            sums[0] = add_pair_f64(sums[0], load_f64(a, whole), b, whole);
        }
        sums
    }

    #[inline]
    #[target_feature(enable = "neon")]
    fn reduce_f64([low, high]: [float64x2_t; 2]) -> f64 {
        let pair = vaddq_f64(low, high);
        vgetq_lane_f64::<0>(pair) + vgetq_lane_f64::<1>(pair)
    }
}

#[cfg(all(target_arch = "x86_64", target_feature = "sse2"))]
mod imp {
    use std::arch::x86_64::*;

    pub(super) fn dot_f32(a: &[f32], b: &[f32]) -> f32 {
        // SAFETY: SSE2 is enabled at compile time
        unsafe { sse2_dot_f32(a, b) }
    }

    pub(super) fn dot_f64(a: &[f64], b: &[f64]) -> f64 {
        // SAFETY: SSE2 is enabled at compile time
        unsafe { sse2_dot_f64(a, b) }
    }

    pub(super) fn dots_f32(
        taps: &[f32],
        tap_starts: &[usize],
        input: &[f32],
        input_starts: &[usize],
        span: usize,
        out: &mut [f32],
    ) {
        // SAFETY: SSE2 is enabled at compile time
        unsafe { sse2_dots_f32(taps, tap_starts, input, input_starts, span, out) }
    }

    pub(super) fn dots_f64(
        taps: &[f64],
        tap_starts: &[usize],
        input: &[f64],
        input_starts: &[usize],
        span: usize,
        out: &mut [f64],
    ) {
        // SAFETY: SSE2 is enabled at compile time
        unsafe { sse2_dots_f64(taps, tap_starts, input, input_starts, span, out) }
    }

    pub(super) fn fir_f32(
        taps: &[f32],
        input: &[f32],
        first: usize,
        stride: usize,
        out: &mut [f32],
    ) {
        // SAFETY: SSE2 is enabled at compile time
        unsafe { sse2_fir_f32(taps, input, first, stride, out) }
    }

    pub(super) fn fir_f64(
        taps: &[f64],
        input: &[f64],
        first: usize,
        stride: usize,
        out: &mut [f64],
    ) {
        // SAFETY: SSE2 is enabled at compile time
        unsafe { sse2_fir_f64(taps, input, first, stride, out) }
    }

    #[target_feature(enable = "sse2")]
    fn sse2_fir_f32(taps: &[f32], input: &[f32], first: usize, stride: usize, out: &mut [f32]) {
        let at = |index: usize| input.as_ptr().wrapping_add(first + index * stride);
        let groups = out.len() / 4;
        for group in 0..groups {
            let start = group * 4;
            let inputs = [at(start), at(start + 1), at(start + 2), at(start + 3)];
            let mut sums = [_mm_setzero_ps(); 4];
            for index in (0..taps.len()).step_by(4) {
                // SAFETY: within the slices, as the module says
                let tap = unsafe { _mm_loadu_ps(taps.as_ptr().add(index)) };
                for lane in 0..4 {
                    // SAFETY: within the slices, as the module says
                    let sample = unsafe { _mm_loadu_ps(inputs[lane].add(index)) };
                    sums[lane] = _mm_add_ps(sums[lane], _mm_mul_ps(tap, sample));
                }
            }
            for lane in 0..4 {
                out[start + lane] = reduce_f32(sums[lane]);
            }
        }
        for (index, out) in out.iter_mut().enumerate().skip(groups * 4) {
            let mut sum = _mm_setzero_ps();
            for offset in (0..taps.len()).step_by(4) {
                sum = add_products_f32(sum, taps.as_ptr(), at(index), offset);
            }
            *out = reduce_f32(sum);
        }
    }

    #[target_feature(enable = "sse2")]
    fn sse2_fir_f64(taps: &[f64], input: &[f64], first: usize, stride: usize, out: &mut [f64]) {
        let at = |index: usize| input.as_ptr().wrapping_add(first + index * stride);
        let tap = |offset: usize| {
            // SAFETY: within the slices, as the module says
            unsafe { _mm_loadu_pd(taps.as_ptr().add(offset)) }
        };
        let whole = taps.len() - taps.len() % 4;
        let groups = out.len() / 4;
        for group in 0..groups {
            let start = group * 4;
            let inputs = [at(start), at(start + 1), at(start + 2), at(start + 3)];
            let mut sums = [[_mm_setzero_pd(); 2]; 4];
            for index in (0..whole).step_by(4) {
                for (half, offset) in [(0, index), (1, index + 2)] {
                    let tap = tap(offset);
                    for lane in 0..4 {
                        sums[lane][half] =
                            add_pair_f64(sums[lane][half], tap, inputs[lane], offset);
                    }
                }
            }
            if whole < taps.len() {
                let tap = tap(whole);
                for lane in 0..4 {
                    sums[lane][0] = add_pair_f64(sums[lane][0], tap, inputs[lane], whole);
                }
            }
            for lane in 0..4 {
                out[start + lane] = reduce_f64(sums[lane]);
            }
        }
        for (index, out) in out.iter_mut().enumerate().skip(groups * 4) {
            *out = reduce_f64(sum_products_f64(taps.as_ptr(), at(index), taps.len()));
        }
    }

    #[target_feature(enable = "sse2")]
    fn sse2_dot_f32(a: &[f32], b: &[f32]) -> f32 {
        let mut sum = _mm_setzero_ps();
        for index in (0..a.len()).step_by(4) {
            sum = add_products_f32(sum, a.as_ptr(), b.as_ptr(), index);
        }
        reduce_f32(sum)
    }

    #[target_feature(enable = "sse2")]
    fn sse2_dots_f32(
        taps: &[f32],
        tap_starts: &[usize],
        input: &[f32],
        input_starts: &[usize],
        span: usize,
        out: &mut [f32],
    ) {
        let tap = |index: usize| taps.as_ptr().wrapping_add(tap_starts[index]);
        let at = |index: usize| input.as_ptr().wrapping_add(input_starts[index]);
        let groups = out.len() / 4;
        for group in 0..groups {
            let first = group * 4;
            let taps = [tap(first), tap(first + 1), tap(first + 2), tap(first + 3)];
            let inputs = [at(first), at(first + 1), at(first + 2), at(first + 3)];
            let mut sums = [_mm_setzero_ps(); 4];
            for index in (0..span).step_by(4) {
                for lane in 0..4 {
                    sums[lane] = add_products_f32(sums[lane], taps[lane], inputs[lane], index);
                }
            }
            for lane in 0..4 {
                out[first + lane] = reduce_f32(sums[lane]);
            }
        }
        for (index, out) in out.iter_mut().enumerate().skip(groups * 4) {
            let mut sum = _mm_setzero_ps();
            for offset in (0..span).step_by(4) {
                sum = add_products_f32(sum, tap(index), at(index), offset);
            }
            *out = reduce_f32(sum);
        }
    }

    #[inline]
    #[target_feature(enable = "sse2")]
    fn add_products_f32(sum: __m128, a: *const f32, b: *const f32, index: usize) -> __m128 {
        // SAFETY: within the slices, as the module says
        let (x, y) = unsafe { (_mm_loadu_ps(a.add(index)), _mm_loadu_ps(b.add(index))) };
        _mm_add_ps(sum, _mm_mul_ps(x, y))
    }

    #[inline]
    #[target_feature(enable = "sse2")]
    fn reduce_f32(sum: __m128) -> f32 {
        let pair = _mm_add_ps(sum, _mm_movehl_ps(sum, sum));
        _mm_cvtss_f32(pair) + _mm_cvtss_f32(_mm_shuffle_ps::<1>(pair, pair))
    }

    #[target_feature(enable = "sse2")]
    fn sse2_dot_f64(a: &[f64], b: &[f64]) -> f64 {
        reduce_f64(sum_products_f64(a.as_ptr(), b.as_ptr(), a.len()))
    }

    #[target_feature(enable = "sse2")]
    fn sse2_dots_f64(
        taps: &[f64],
        tap_starts: &[usize],
        input: &[f64],
        input_starts: &[usize],
        span: usize,
        out: &mut [f64],
    ) {
        let tap = |index: usize| taps.as_ptr().wrapping_add(tap_starts[index]);
        let at = |index: usize| input.as_ptr().wrapping_add(input_starts[index]);
        let whole = span - span % 4;
        let groups = out.len() / 4;
        for group in 0..groups {
            let first = group * 4;
            let taps = [tap(first), tap(first + 1), tap(first + 2), tap(first + 3)];
            let inputs = [at(first), at(first + 1), at(first + 2), at(first + 3)];
            let mut sums = [[_mm_setzero_pd(); 2]; 4];
            for index in (0..whole).step_by(4) {
                for lane in 0..4 {
                    for (half, offset) in [(0, index), (1, index + 2)] {
                        let tap = load_f64(taps[lane], offset);
                        sums[lane][half] =
                            add_pair_f64(sums[lane][half], tap, inputs[lane], offset);
                    }
                }
            }
            if whole < span {
                for lane in 0..4 {
                    let tap = load_f64(taps[lane], whole);
                    sums[lane][0] = add_pair_f64(sums[lane][0], tap, inputs[lane], whole);
                }
            }
            for lane in 0..4 {
                out[first + lane] = reduce_f64(sums[lane]);
            }
        }
        for (index, out) in out.iter_mut().enumerate().skip(groups * 4) {
            *out = reduce_f64(sum_products_f64(tap(index), at(index), span));
        }
    }

    #[inline]
    #[target_feature(enable = "sse2")]
    fn load_f64(a: *const f64, offset: usize) -> __m128d {
        // SAFETY: within the slices, as the module says
        unsafe { _mm_loadu_pd(a.add(offset)) }
    }

    /// `sum` plus the products of `x` and of the two elements of `b` from
    /// `offset`
    #[inline]
    #[target_feature(enable = "sse2")]
    fn add_pair_f64(sum: __m128d, x: __m128d, b: *const f64, offset: usize) -> __m128d {
        _mm_add_pd(sum, _mm_mul_pd(x, load_f64(b, offset)))
    }

    /// Of the first `len` elements of `a` and `b`, lanes 0 and 1 in the first
    /// sum and 2 and 3 in the second
    #[inline]
    #[target_feature(enable = "sse2")]
    fn sum_products_f64(a: *const f64, b: *const f64, len: usize) -> [__m128d; 2] {
        let whole = len - len % 4;
        let mut sums = [_mm_setzero_pd(); 2];
        for index in (0..whole).step_by(4) {
            for (half, sum) in sums.iter_mut().enumerate() {
                let offset = index + 2 * half;
                *sum = add_pair_f64(*sum, load_f64(a, offset), b, offset);
            }
        }
        if whole < len {
            sums[0] = add_pair_f64(sums[0], load_f64(a, whole), b, whole);
        }
        sums
    }

    #[inline]
    #[target_feature(enable = "sse2")]
    fn reduce_f64([low, high]: [__m128d; 2]) -> f64 {
        let pair = _mm_add_pd(low, high);
        _mm_cvtsd_f64(pair) + _mm_cvtsd_f64(_mm_unpackhi_pd(pair, pair))
    }
}

#[cfg(test)]
mod tests;
