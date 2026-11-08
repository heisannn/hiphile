use super::*;

#[test]
fn whole_rates_give_their_fraction() {
    assert_eq!(
        Ratio::of_rates(44_100.0, 48_000.0).fraction,
        Some((147, 160))
    );
    assert_eq!(
        Ratio::of_rates(96_000.0, 44_100.0).fraction,
        Some((320, 147))
    );
    assert_eq!(Ratio::of_rates(1.0, 1e6).fraction, Some((1, 1_000_000)));
}

#[test]
fn other_rates_give_a_fraction_that_divides_to_the_ratio() {
    let ratio = Ratio::of_rates(44_100.0, 47_999.5);
    assert_eq!(ratio.fraction, Some((88_200, 95_999)));
    assert_eq!(Ratio::of_rates(44_100.0, 50_000.123).fraction, None);
    assert_eq!(Ratio::of_rates(1.0, std::f64::consts::PI).fraction, None);
}

#[test]
fn scaling_keeps_the_fraction_in_lowest_terms() {
    let ratio = Ratio::of_rates(44_100.0, 48_000.0).scaled(2, 1);
    assert_eq!(ratio.fraction, Some((147, 80)));
    assert_eq!(ratio.value, 2.0 * 44_100.0 / 48_000.0);
    assert_eq!(
        Ratio::of_rates(4.0, 1.0).scaled(1, 2).fraction,
        Some((2, 1))
    );
}

#[test]
fn an_exact_clock_keeps_time_exactly() {
    let mut clock = Clock::new(Ratio::of_rates(147.0, 80.0));
    for _ in 0..80 * 1000 {
        clock.advance();
    }
    assert_eq!(clock.whole(), 147 * 1000);
    assert_eq!(clock.fraction_f64(), 0.0);
}

#[test]
fn a_fixed_clock_holds_the_double_it_steps_by() {
    let step = std::f64::consts::E / 3.0;
    let mut clock = Clock::new(Ratio {
        value: step,
        fraction: None,
    });
    let count = 1_000_000u64;
    for _ in 0..count {
        clock.advance();
    }
    let time = clock.whole() as f64 + clock.fraction_f64();
    assert!((time - step * count as f64).abs() < 1e-9);
    assert_eq!(fixed(1.5), 3 << 63);
}
