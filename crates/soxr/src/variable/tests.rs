use super::*;

fn kernel() -> Kernel {
    let attenuation = design::attenuation(PRECISION);
    let kernel = Kernel::low_pass(PASSBAND_END, STOPBAND_BEGIN, attenuation, 1.0);
    kernel.with_gain(1.0 / kernel.dc_gain(DC_SAMPLES))
}

/// At least 12 dB under the stop-band, the most near the ends, where the
/// window's slope is unbounded
#[test]
fn the_table_follows_the_prototype() {
    let kernel = kernel();
    let prototype = &*PROTOTYPE;
    let steps = 1_000_000;
    let worst = (1..steps)
        .map(|step| {
            let time = prototype.half_len * (2.0 * step as f64 / steps as f64 - 1.0);
            (prototype.at(time) - kernel.at(time)).abs()
        })
        .fold(0.0, f64::max);
    let allowed = 10f64.powf(-(design::attenuation(PRECISION) + 12.0) / 20.0);
    assert!(worst < allowed, "{worst:e}");
}

#[test]
fn a_slew_moves_the_time_by_the_mean_of_its_ratios() {
    let mut clock = Clock::new(1.0);
    for _ in 0..10 {
        clock.advance();
    }
    clock.set(2.0, 1000);
    let start = clock.time;
    for _ in 0..1000 {
        clock.advance();
    }
    assert!(clock.slew.is_none());
    assert_eq!(clock.since, (1010, clock.time));
    let moved = (clock.time - start) as f64 / FIXED_ONE;
    assert!((moved - 1500.0).abs() < 1e-9, "{moved}");
    assert_eq!(clock.step(), 2.0);
}

#[test]
fn what_is_left_is_what_stepping_finds() {
    for (slew_len, input_frames) in [(0, 5000), (100, 5000), (4000, 5000), (100_000, 7000)] {
        let mut clock = Clock::new(0.7);
        for _ in 0..1000 {
            clock.advance();
        }
        clock.set(1.9, slew_len);
        let left = clock.left(input_frames);
        let mut stepped = 0;
        while clock.has_next(input_frames) {
            stepped += 1;
            clock.advance();
        }
        assert_eq!(left, stepped, "{slew_len}");
    }
}

#[test]
fn a_ratio_set_again_keeps_the_count() {
    let mut clock = Clock::new(48_000.0 / 44_100.0);
    for _ in 0..777 {
        clock.advance();
    }
    clock.set(48_000.0 / 44_100.0, 0);
    assert_eq!(clock.since, (0, 0));
}
