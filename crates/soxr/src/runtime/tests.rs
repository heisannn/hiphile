use super::*;

fn with(vars: &[(&str, &str)]) -> RuntimeSpec {
    RuntimeSpec::default().with_vars(|name| {
        vars.iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| (*value).to_owned())
    })
}

#[test]
fn defaults_are_libsoxrs() {
    let spec = RuntimeSpec::new(3);
    assert_eq!(spec.log2_min_dft_size, 10);
    assert_eq!(spec.log2_large_dft_size, 17);
    assert_eq!(spec.coef_size_kbytes, 400);
    assert_eq!(spec.num_threads, 3);
    assert_eq!(spec.coef_interpolation, CoefInterpolation::Auto);
    assert_eq!(RuntimeSpec::default().num_threads, 1);
}

#[test]
fn the_environment_overrides_only_what_it_sets() {
    let spec = with(&[
        ("SOXR_MIN_DFT_SIZE", "8"),
        ("SOXR_LARGE_DFT_SIZE", " 12 "),
        ("SOXR_COEFS_SIZE", "100"),
        ("SOXR_NUM_THREADS", "4"),
        ("SOXR_COEF_INTERP", "3"),
    ]);
    assert_eq!(spec.log2_min_dft_size, 8);
    assert_eq!(spec.log2_large_dft_size, 12);
    assert_eq!(spec.coef_size_kbytes, 100);
    assert_eq!(spec.num_threads, 4);
    assert_eq!(spec.coef_interpolation, CoefInterpolation::High);

    assert_eq!(
        with(&[("SOXR_COEF_INTERP", "2")]).coef_interpolation,
        CoefInterpolation::Low
    );
    assert_eq!(with(&[]), RuntimeSpec::default());
    assert_eq!(
        with(&[("SOXR_MIN_DFT_SIZE", "eight"), ("SOXR_COEF_INTERP", "4")]),
        RuntimeSpec::default()
    );
    let high = with(&[("SOXR_COEF_INTERP", "3")]);
    assert_eq!(
        high.with_vars(|name| (name == "SOXR_COEF_INTERP").then(|| "1".to_owned()))
            .coef_interpolation,
        CoefInterpolation::Auto
    );
}

#[test]
fn dft_sizes_out_of_range_are_the_defaults() {
    let sizes = |log2_min_dft_size, log2_large_dft_size| {
        RuntimeSpec {
            log2_min_dft_size,
            log2_large_dft_size,
            ..RuntimeSpec::default()
        }
        .dft_sizes()
    };
    assert_eq!(sizes(8, 20), (8, 20));
    // Each in its range, though the least is over the large
    assert_eq!(sizes(15, 8), (15, 8));
    for (min, large) in [(0, 0), (7, 7), (16, 21), (25, 24)] {
        assert_eq!(sizes(min, large), (10, 17));
    }
}

#[test]
fn zero_threads_are_all_cores() {
    assert!(RuntimeSpec::new(0).threads() >= 1);
    assert_eq!(RuntimeSpec::new(5).threads(), 5);
}
