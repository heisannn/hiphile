use std::sync::Arc;

use crate::cache::{Cache, WhileInUse};
use crate::math::{normalize_fir, spline_coefficients};
use crate::sinc::{self, KaiserParams};

pub(crate) const DEFAULT_CACHE_LIMIT: usize = 12;
/// Rows that a quadratic through eight points takes, around the one it is for
const SPLINE_POINTS: usize = 8;
/// Coefficients per tap of a row between which [`FilterBank::fractional`]
/// interpolates
const SPLINE_ORDERS: usize = 3;

/// Kaiser beta, power and the attenuation they reach, for each length of filter
/// in steps of two taps from the first
const HALF_BAND_WINDOWS: (usize, [(f64, f64, f64); 12]) = (
    8,
    [
        (4.1308468534586913, 1.1752580009977263, 55.5446),
        (4.4241520324148826, 1.8004881791443044, 81.4191),
        (5.2615232289173663, 1.8133318236025469, 96.3392),
        (5.9433751227216174, 1.8730186391986436, 111.1315),
        (6.8308658290513815, 1.8549555110340281, 125.4653),
        (7.6648458290312904, 1.8565766090828464, 139.7379),
        (8.2038728664307605, 1.9269521820570166, 154.0532),
        (8.7865150946655142, 1.9775307667441668, 168.2101),
        (9.5945017884101773, 1.9718456992078597, 182.1076),
        (10.5163141145985240, 1.9504067820201083, 195.5668),
        (10.2382465206362470, 2.1608923446870087, 209.0610),
        (10.9976060250714000, 2.1536533525688935, 222.5010),
    ],
);
const THIRD_BAND_WINDOWS: (usize, [(f64, f64, f64); 10]) = (
    6,
    [
        (3.9888564562781847, 1.5869927184268915, 66.5701),
        (4.6986694038145007, 1.8086068597928262, 86.4715),
        (5.5995071329337822, 1.8930163360942349, 106.1195),
        (6.3627287800257228, 1.9945748322093975, 125.2307),
        (7.4299550711428308, 1.9893400572347544, 144.3469),
        (8.0667715944075642, 2.0928201458699909, 163.4099),
        (8.7469970226288822, 2.1640279784268355, 181.0694),
        (10.0823430069835230, 2.0896678025321922, 199.2880),
        (10.9222206090489510, 2.1221681162186004, 216.6865),
        (21.2017743894772010, 1.1856768080118900, 233.9188),
    ],
);

static WHOLE_STEP_BANKS: Cache<FilterBank> = Cache::new(DEFAULT_CACHE_LIMIT);
/// A set number of positions can take tens of megabytes, and any number can be asked for
static BANKS_OF_SET_POSITIONS: WhileInUse<FilterBank> = WhileInUse::new();
/// Costly to build and few in number, so none is ever dropped
static FRACTIONAL_BANKS: Cache<FilterBank> = Cache::new(usize::MAX);

pub(crate) fn set_cache_limit(limit: usize) {
    WHOLE_STEP_BANKS.set_limit(limit);
}

/// Fractional delay filters by position: row 0 delays by nothing and the last
/// row by nearly or wholly a sample.
pub(crate) struct FilterBank {
    fracs: usize,
    attenuation: f64,
    is_third: bool,
    filter_len: usize,
    filter_size: usize,
    is_fractional: bool,
    table: Vec<f64>,
}

struct Window {
    params: KaiserParams,
    attenuation: f64,
    filter_len: usize,
}

impl FilterBank {
    /// One row per output position, of plain taps, for a ratio of whole numbers
    /// with `out_step` as its divisor.
    pub(crate) fn whole_step(out_step: usize, attenuation: f64, is_third: bool) -> Arc<Self> {
        let window = window_for(attenuation, is_third);
        let attenuation = window.attenuation;
        WHOLE_STEP_BANKS.get_or_build(
            |bank| {
                !bank.is_fractional
                    && bank.fracs == out_step
                    && bank.is_third == is_third
                    && bank.attenuation == attenuation
            },
            || Self::build_whole_step(out_step, &window, is_third),
        )
    }

    /// Rows of quadratic coefficients, each tap three of them, for any ratio.
    /// `row(i)` and `row(i + 1)` bound the positions from `i / fracs` on.
    /// `fracs` of `None` picks a number that reaches `attenuation`.
    pub(crate) fn fractional(attenuation: f64, is_third: bool, fracs: Option<usize>) -> Arc<Self> {
        let window = window_for(attenuation, is_third);
        let attenuation = window.attenuation;
        match fracs {
            Some(fracs) => BANKS_OF_SET_POSITIONS.get_or_build(
                |bank| {
                    bank.fracs == fracs
                        && bank.is_third == is_third
                        && bank.attenuation == attenuation
                },
                || Self::build_fractional(&window, is_third, fracs),
            ),
            None => FRACTIONAL_BANKS.get_or_build(
                |bank| bank.is_third == is_third && bank.attenuation == attenuation,
                || {
                    let fracs = 6.4f64.powf(attenuation / 50.0).ceil() as usize;
                    Self::build_fractional(&window, is_third, fracs)
                },
            ),
        }
    }

    fn build_whole_step(out_step: usize, window: &Window, is_third: bool) -> Self {
        let filter_len = window.filter_len;
        let mut table = vec![0.0; filter_len * out_step];
        for (position, row) in table.chunks_exact_mut(filter_len).enumerate() {
            let frac_delay = (out_step - position) as f64 / out_step as f64;
            sinc::fractional_delay(row, 1, filter_len / 2, frac_delay, window.params);
            normalize_fir(row, 1, 1.0);
        }
        Self {
            fracs: out_step,
            attenuation: window.attenuation,
            is_third,
            filter_len,
            filter_size: filter_len,
            is_fractional: false,
            table,
        }
    }

    fn build_fractional(window: &Window, is_third: bool, fracs: usize) -> Self {
        let filter_len = window.filter_len;
        let filter_size = filter_len * SPLINE_ORDERS;
        let rows_before = (SPLINE_POINTS / 2) as i64 - 1;

        let mut table = vec![0.0; filter_size * (fracs + SPLINE_POINTS)];
        for (row_index, row) in table.chunks_exact_mut(filter_size).enumerate() {
            let position = row_index as i64 - rows_before;
            let frac_delay = (fracs as i64 - position) as f64 / fracs as f64;
            sinc::fractional_delay(
                row,
                SPLINE_ORDERS,
                filter_len / 2,
                frac_delay,
                window.params,
            );
            normalize_fir(row, SPLINE_ORDERS, 1.0);
        }

        // Going forwards overwrites only what later rows no longer read
        for index in (0..(fracs + 1) * filter_size).step_by(SPLINE_ORDERS) {
            let mut points = [0.0; SPLINE_POINTS];
            for (row, point) in points.iter_mut().enumerate() {
                *point = table[index + row * filter_size];
            }
            table[index..index + SPLINE_ORDERS].copy_from_slice(&spline_coefficients(points));
        }
        table.truncate((fracs + 1) * filter_size);

        Self {
            fracs,
            attenuation: window.attenuation,
            is_third,
            filter_len,
            filter_size,
            is_fractional: true,
            table,
        }
    }

    pub(crate) fn filter_len(&self) -> usize {
        self.filter_len
    }

    pub(crate) fn fracs(&self) -> usize {
        self.fracs
    }

    #[inline]
    pub(crate) fn row(&self, index: usize) -> &[f64] {
        &self.table[index * self.filter_size..][..self.filter_size]
    }
}

/// The shortest that reaches `attenuation`, or else the longest
fn window_for(attenuation: f64, is_third: bool) -> Window {
    let (shortest, rows): (usize, &[(f64, f64, f64)]) = if is_third {
        (THIRD_BAND_WINDOWS.0, &THIRD_BAND_WINDOWS.1)
    } else {
        (HALF_BAND_WINDOWS.0, &HALF_BAND_WINDOWS.1)
    };
    let index = rows
        .iter()
        .position(|row| row.2 >= attenuation)
        .unwrap_or(rows.len() - 1);
    let (beta, power, reached) = rows[index];
    Window {
        params: KaiserParams { beta, power },
        attenuation: reached,
        filter_len: shortest + index * 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bank_of_a_set_number_of_positions_goes_once_out_of_use() {
        let bank = FilterBank::fractional(180.15, false, Some(77));
        assert!(Arc::ptr_eq(
            &bank,
            &FilterBank::fractional(180.15, false, Some(77))
        ));
        let gone = Arc::downgrade(&bank);
        drop(bank);
        assert!(gone.upgrade().is_none());
    }
}
