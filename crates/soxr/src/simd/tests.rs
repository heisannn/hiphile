use crate::real::Real;

/// In `[-1, 1)`, the same for the same seed
fn noise<T: Real>(len: usize, seed: u64) -> Vec<T> {
    let mut state = seed;
    (0..len)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            T::from_double((state >> 11) as f64 / (1u64 << 52) as f64 - 1.0)
        })
        .collect()
}

fn bits<T: Real>(value: T) -> u64 {
    value.to_double().to_bits()
}

#[cfg(all(target_arch = "x86_64", target_feature = "sse2"))]
fn all_bits<T: Real>(values: &[T]) -> Vec<u64> {
    values.iter().copied().map(bits).collect()
}

fn kernels_agree<T: Real>() {
    for span in (T::PADDING..=44).step_by(T::PADDING) {
        let taps = noise::<T>(3 * span, 1);
        let input = noise::<T>(300, 2);
        let dot = |row: usize, start: usize| {
            T::dot_padded(&taps[row * span..][..span], &input[start..][..span])
        };
        for count in 0..=9 {
            let rows: Vec<_> = (0..count).map(|index| index % 3).collect();
            let tap_starts: Vec<_> = rows.iter().map(|row| row * span).collect();
            let input_starts: Vec<_> = (0..count).map(|index| 5 * index + index % 2).collect();
            let mut out = vec![T::ZERO; count];
            T::dots(&taps, &tap_starts, &input, &input_starts, span, &mut out);
            for ((sum, row), start) in out.iter().zip(&rows).zip(&input_starts) {
                assert_eq!(bits(*sum), bits(dot(*row, *start)), "span {span}");
            }
            T::fir(&taps[span..][..span], &input, 7, 3, &mut out);
            for (index, sum) in out.iter().enumerate() {
                assert_eq!(bits(*sum), bits(dot(1, 7 + 3 * index)), "span {span}");
            }
        }
    }
}

#[test]
fn every_kernel_gives_an_output_the_same_bits() {
    kernels_agree::<f32>();
    kernels_agree::<f64>();
}

fn kernels_sum_products<T: Real>(epsilon: f64) {
    for span in (T::PADDING..=44).step_by(T::PADDING) {
        let (a, b) = (noise::<T>(span, 3), noise::<T>(span, 4));
        let products = a.iter().zip(&b).map(|(a, b)| a.to_double() * b.to_double());
        let (exact, size) = products.fold((0.0, 0.0), |(sum, size), product: f64| {
            (sum + product, size + product.abs())
        });
        let error = (T::dot_padded(&a, &b).to_double() - exact).abs();
        assert!(error <= span as f64 * epsilon * size, "span {span}");
    }
}

#[test]
fn kernels_sum_the_products() {
    kernels_sum_products::<f32>(f64::from(f32::EPSILON));
    kernels_sum_products::<f64>(f64::EPSILON);
}

/// SSE2 rounds as the scalar loops do, so the tests on x86-64 also cover the
/// targets that have neither
#[cfg(all(target_arch = "x86_64", target_feature = "sse2"))]
fn scalar_bits<T: Real>() {
    use super::scalar;
    for span in (T::PADDING..=44).step_by(T::PADDING) {
        let taps = noise::<T>(3 * span, 5);
        let input = noise::<T>(300, 6);
        let tap_starts: Vec<_> = (0..9).map(|index| (index % 3) * span).collect();
        let input_starts: Vec<_> = (0..9).map(|index| 5 * index + index % 2).collect();
        let (mut vector, mut scalar_out) = (vec![T::ZERO; 9], vec![T::ZERO; 9]);
        T::dots(&taps, &tap_starts, &input, &input_starts, span, &mut vector);
        scalar::dots(
            &taps,
            &tap_starts,
            &input,
            &input_starts,
            span,
            &mut scalar_out,
        );
        assert_eq!(all_bits(&vector), all_bits(&scalar_out), "span {span}");
        T::fir(&taps[..span], &input, 7, 3, &mut vector);
        scalar::fir(&taps[..span], &input, 7, 3, &mut scalar_out);
        assert_eq!(all_bits(&vector), all_bits(&scalar_out), "span {span}");
    }
}

#[cfg(all(target_arch = "x86_64", target_feature = "sse2"))]
#[test]
fn sse2_gives_the_scalar_bits() {
    scalar_bits::<f32>();
    scalar_bits::<f64>();
}
