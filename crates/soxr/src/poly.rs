//! Interpolation at any times, by a short filter whose taps depend on where
//! the output falls between input samples

use std::sync::Arc;

use crate::clock::{Clock, Fraction, Ratio};
use crate::design::Kernel;
use crate::real::Real;

/// Input samples dropped at once from the front of a stage's buffer, so that
/// the rest moves rarely
const DRAIN_LEN: u64 = 4096;
/// Output samples whose windows are found before any is computed
const BATCH_LEN: usize = 64;
/// Of the rows, at least this share must have one tap for the outputs of
/// those to be worked out apart, as telling them apart costs the rest
const LEAST_SINGLE_SHARE: usize = 4;
/// The fewest cycles taken at once. Each place of the cycle takes a call of
/// [`Real::fir`], which costs more than its dot products when it makes few.
const LEAST_CYCLES: usize = 16;
/// The most input samples that the cycles taken at once span, so that a
/// stage's input usually holds them
const MOST_CYCLED_INPUT: usize = 4096;
/// What a [`Cycle`] keeps for each numerator, on top of its row
pub(crate) const CYCLE_BYTES: usize = 3 * size_of::<usize>();

/// The taps for each position between two input samples. Each row is padded
/// with zeros to the filter's span.
pub(crate) enum Taps<T> {
    /// A row for each numerator of the clock's fraction, and for each place
    /// of the cycle the tap of its row and its value if the row has only one
    /// that is not 0. `singles` is empty when too few rows have one.
    Exact {
        rows: Vec<T>,
        singles: Vec<Option<(usize, T)>>,
        cycle: Cycle,
    },
    /// Rows of polynomial coefficients for each of `segments` equal parts of a
    /// sample, from the constant up, `order + 1` rows to a segment. They stay
    /// in `f64`, so that each tap rounds once, as an exact row's does.
    Interpolated {
        rows: Vec<f64>,
        segments_log2: u32,
        order: usize,
    },
    /// The four taps of cubic Lagrange interpolation, worked out at each time
    Cubic,
}

/// The rows in the order that the outputs take them from the clock at 0, and
/// the first [`BATCH_LEN`] of them again after the end, so that a batch from
/// any place reads straight on
pub(crate) struct Cycle {
    /// Where the row of each place starts
    rows: Vec<usize>,
    /// How far the window of each place is past that of place 0
    offsets: Vec<usize>,
    /// The place of each numerator
    places: Vec<usize>,
}

impl Cycle {
    /// How far the window moves in a cycle
    fn period(&self) -> usize {
        self.offsets[self.places.len()]
    }

    fn new(step: Ratio, span: usize) -> Self {
        let (top, bottom) = step.fraction.expect("exact taps need a fraction");
        let (step_whole, step_numerator) = (top / bottom, top % bottom);
        let denominator = bottom as usize;
        let len = denominator + BATCH_LEN + 1;
        let mut cycle = Self {
            rows: Vec::with_capacity(len),
            offsets: Vec::with_capacity(len),
            places: vec![0; denominator],
        };
        let (mut whole, mut numerator) = (0, 0);
        for place in 0..len {
            cycle.rows.push(numerator as usize * span);
            cycle.offsets.push(whole as usize);
            if place < denominator {
                cycle.places[numerator as usize] = place;
            }
            whole += step_whole;
            numerator += step_numerator;
            if numerator >= bottom {
                numerator -= bottom;
                whole += 1;
            }
        }
        cycle
    }
}

/// What every channel shares
pub(crate) struct PolyFilter<T> {
    taps: Taps<T>,
    len: usize,
    /// `len` rounded up to whole [`Real::PADDING`]: the input that each
    /// output reads
    span: usize,
    step: Ratio,
    /// For exact rows, how many cycles to take at once, or 0
    cycles: usize,
}

/// How a stage holds its taps
#[derive(Clone, Copy, Debug)]
pub(crate) enum Layout {
    Exact,
    Interpolated { segments_log2: u32, order: usize },
}

impl<T: Real> PolyFilter<T> {
    /// Steps by `step` input samples an output, through `len` taps of `kernel`
    pub(crate) fn new(kernel: &Kernel, len: usize, step: Ratio, layout: Layout) -> Self {
        let span = len.next_multiple_of(T::PADDING);
        let tap = |fraction: f64, index: usize| {
            T::from_double(kernel.at(fraction + (len / 2) as f64 - 1.0 - index as f64))
        };
        let taps = match (layout, step.fraction) {
            (Layout::Exact, Some((_, denominator))) => {
                let mut rows = vec![T::ZERO; denominator as usize * span];
                for (numerator, row) in rows.chunks_exact_mut(span).enumerate() {
                    let fraction = numerator as f64 / denominator as f64;
                    for (index, tap_of) in row[..len].iter_mut().enumerate() {
                        *tap_of = tap(fraction, index);
                    }
                }
                exact(rows, span, step)
            }
            (
                Layout::Interpolated {
                    segments_log2,
                    order,
                },
                _,
            ) => interpolated(kernel, len, span, segments_log2, order),
            (Layout::Exact, None) => unreachable!("exact taps need a fraction"),
        };
        let cycles = cycles(&taps);
        Self {
            taps,
            len,
            span,
            step,
            cycles,
        }
    }

    /// For QQ: no filter, only cubic interpolation. With a fraction whose
    /// table of weights fits in `budget` bytes, the weights of each numerator
    /// are worked out once.
    pub(crate) fn cubic(step: Ratio, budget: usize) -> Self {
        let taps = match step.fraction {
            Some((_, denominator))
                if usize::try_from(denominator)
                    .ok()
                    .and_then(|rows| rows.checked_mul(4 * size_of::<T>() + CYCLE_BYTES))
                    .is_some_and(|bytes| bytes <= budget) =>
            {
                let rows = (0..denominator)
                    .flat_map(|numerator| {
                        lagrange_weights(numerator as f64 / denominator as f64).map(T::from_double)
                    })
                    .collect();
                exact(rows, 4, step)
            }
            _ => Taps::Cubic,
        };
        let cycles = cycles(&taps);
        Self {
            taps,
            len: 4,
            span: 4,
            step,
            cycles,
        }
    }
}

fn cycles<T>(taps: &Taps<T>) -> usize {
    let Taps::Exact { cycle, .. } = taps else {
        return 0;
    };
    let denominator = cycle.places.len();
    let cycles = (BATCH_LEN / denominator).max(LEAST_CYCLES) / LEAST_CYCLES * LEAST_CYCLES;
    if cycles * cycle.period() > MOST_CYCLED_INPUT {
        0
    } else {
        cycles
    }
}

fn exact<T: Real>(rows: Vec<T>, span: usize, step: Ratio) -> Taps<T> {
    let of_rows: Vec<_> = rows
        .chunks_exact(span)
        .map(|row| {
            let mut taps = row.iter().enumerate().filter(|(_, tap)| **tap != T::ZERO);
            match (taps.next(), taps.next()) {
                (Some((index, tap)), None) => Some((index, *tap)),
                _ => None,
            }
        })
        .collect();
    let cycle = Cycle::new(step, span);
    let singles = if of_rows.iter().flatten().count() * LEAST_SINGLE_SHARE < of_rows.len() {
        Vec::new()
    } else {
        cycle.rows.iter().map(|row| of_rows[row / span]).collect()
    };
    Taps::Exact {
        rows,
        singles,
        cycle,
    }
}

/// The coefficients of a polynomial in the position within each segment,
/// through the taps at the segment's ends, and for cubics at one more each side
fn interpolated<T>(
    kernel: &Kernel,
    len: usize,
    span: usize,
    segments_log2: u32,
    order: usize,
) -> Taps<T> {
    let segments = 1usize << segments_log2;
    let at = |node: f64, index: usize| {
        kernel.at(node / segments as f64 + (len / 2) as f64 - 1.0 - index as f64)
    };
    let mut rows = vec![0.0; segments * (order + 1) * span];
    for (segment, rows) in rows.chunks_exact_mut((order + 1) * span).enumerate() {
        let base = segment as f64;
        for index in 0..len {
            let coefficients = if order == 1 {
                let [v0, v1] = [at(base, index), at(base + 1.0, index)];
                vec![v0, v1 - v0]
            } else {
                let values = [-1.0, 0.0, 1.0, 2.0].map(|node| at(base + node, index));
                lagrange(values).to_vec()
            };
            for (power, coefficient) in coefficients.into_iter().enumerate() {
                rows[power * span + index] = coefficient;
            }
        }
    }
    Taps::Interpolated {
        rows,
        segments_log2,
        order,
    }
}

/// The coefficients of `x^0` to `x^3` of the cubic through `values` at -1, 0,
/// 1 and 2
pub(crate) fn lagrange([before, at, after, next]: [f64; 4]) -> [f64; 4] {
    [
        at,
        -before / 3.0 - at / 2.0 + after - next / 6.0,
        before / 2.0 - at + after / 2.0,
        -before / 6.0 + at / 2.0 - after / 2.0 + next / 6.0,
    ]
}

/// The weights of the samples at -1, 0, 1 and 2 for a value at `x`
pub(crate) fn lagrange_weights(x: f64) -> [f64; 4] {
    let (plus, minus, minus2) = (x + 1.0, x - 1.0, x - 2.0);
    [
        -x * minus * minus2 / 6.0,
        plus * minus * minus2 / 2.0,
        -plus * x * minus2 / 2.0,
        plus * x * minus / 6.0,
    ]
}

/// One channel's state
pub(crate) struct PolyStage<T> {
    filter: Arc<PolyFilter<T>>,
    clock: Clock,
    /// The samples from `dropped - (len / 2 - 1)` on
    buffer: Vec<T>,
    dropped: u64,
    /// The taps of an output worked out at its time
    row: Vec<T>,
    /// For each output of a batch with a full row, where its window starts in
    /// the buffer, where its row starts, where it goes in the batch and its
    /// sum
    windows: [usize; BATCH_LEN],
    rows: [usize; BATCH_LEN],
    slots: [usize; BATCH_LEN],
    sums: [T; BATCH_LEN],
}

impl<T: Real> PolyStage<T> {
    pub(crate) fn new(filter: Arc<PolyFilter<T>>) -> Self {
        let mut stage = Self {
            clock: Clock::new(filter.step),
            buffer: Vec::new(),
            dropped: 0,
            row: vec![T::ZERO; filter.span],
            windows: [0; BATCH_LEN],
            rows: [0; BATCH_LEN],
            slots: [0; BATCH_LEN],
            sums: [T::ZERO; BATCH_LEN],
            filter,
        };
        stage.clear();
        stage
    }

    pub(crate) fn clear(&mut self) {
        self.clock.reset();
        self.buffer.clear();
        self.buffer.resize(self.filter.len / 2 - 1, T::ZERO);
        self.dropped = 0;
    }

    pub(crate) fn push(&mut self, input: &[T], out: &mut Vec<T>) {
        self.feed(input);
        self.produce(usize::MAX, out);
    }

    /// Once this many more samples have come, every output with the earlier
    /// ones in its window has been produced
    pub(crate) fn memory(&self) -> usize {
        self.filter.span + 1
    }

    pub(crate) fn feed(&mut self, input: &[T]) {
        self.buffer.extend_from_slice(input);
    }

    /// Up to `most` samples of output, as far as the input fed so far goes
    pub(crate) fn produce(&mut self, most: usize, out: &mut Vec<T>) {
        let mut left = most;
        while left > 0 {
            let cycled = self.produce_cycles(left, out);
            if cycled > 0 {
                left -= cycled;
                continue;
            }
            let batch = left.min(BATCH_LEN);
            let made = match &self.filter.taps {
                Taps::Exact { .. } => self.produce_exact(batch, out),
                _ => self.produce_each(batch, out),
            };
            left -= made;
            if made < batch {
                break;
            }
        }
        let start = self
            .clock
            .whole()
            .min(self.dropped + self.buffer.len() as u64);
        if start - self.dropped >= DRAIN_LEN {
            self.buffer.drain(..(start - self.dropped) as usize);
            self.dropped = start;
        }
    }

    /// Where the next output's window starts in the buffer, if the buffer
    /// holds all of it
    fn next_start(&self) -> Option<usize> {
        let start = (self.clock.whole() - self.dropped) as usize;
        (start + self.filter.span <= self.buffer.len()).then_some(start)
    }

    /// Up to `most` outputs of exact rows, and how many. An output of a row
    /// with one tap is its product with its sample, which is what the dot
    /// product of the row comes to, and the rest go to [`Real::dots`].
    fn produce_exact(&mut self, most: usize, out: &mut Vec<T>) -> usize {
        let Taps::Exact {
            rows,
            singles,
            cycle,
        } = &self.filter.taps
        else {
            unreachable!("only exact taps come here");
        };
        let Clock::Exact {
            whole,
            numerator,
            step_numerator,
            denominator,
            ..
        } = &mut self.clock
        else {
            unreachable!("exact taps come with an exact clock");
        };
        let span = self.filter.span;
        let Some(last) = self.buffer.len().checked_sub(span) else {
            return 0;
        };
        let place = cycle.places[*numerator as usize];
        // Wraps when the window of place 0 would come before the buffer
        let base = ((*whole - self.dropped) as usize).wrapping_sub(cycle.offsets[place]);
        let offsets = &cycle.offsets[place..][..most.min(BATCH_LEN)];
        let count = offsets.partition_point(|offset| base.wrapping_add(*offset) <= last);
        let windows = &mut self.windows[..count];
        for (window, offset) in windows.iter_mut().zip(offsets) {
            *window = base.wrapping_add(*offset);
        }
        let first = out.len();
        out.resize(first + count, T::ZERO);
        let made = &mut out[first..];
        let tap_starts = &cycle.rows[place..][..count];
        if singles.is_empty() {
            T::dots(rows, tap_starts, &self.buffer, windows, span, made);
        } else {
            let mut full = 0;
            for (index, single) in singles[place..][..count].iter().enumerate() {
                let window = self.windows[index];
                if let Some((tap_index, tap)) = single {
                    made[index] = *tap * self.buffer[window + tap_index];
                } else {
                    self.windows[full] = window;
                    self.rows[full] = tap_starts[index];
                    self.slots[full] = index;
                    full += 1;
                }
            }
            let sums = &mut self.sums[..full];
            let (tap_starts, windows) = (&self.rows[..full], &self.windows[..full]);
            T::dots(rows, tap_starts, &self.buffer, windows, span, sums);
            for (sum, index) in sums.iter().zip(&self.slots) {
                made[*index] = *sum;
            }
        }
        let next = place + count;
        *whole = self.dropped + base.wrapping_add(cycle.offsets[next]) as u64;
        let product = u128::from(next as u64 % *denominator) * u128::from(*step_numerator);
        *numerator = (product % u128::from(*denominator)) as u64;
        count
    }

    /// The outputs of [`PolyFilter::cycles`] whole cycles, if `most` and the
    /// buffer take them, and how many. The outputs at a place of the cycle
    /// share its row and have windows a period apart, which [`Real::fir`]
    /// takes together, and which make what [`Self::produce_exact`] makes.
    fn produce_cycles(&mut self, most: usize, out: &mut Vec<T>) -> usize {
        let cycles = self.filter.cycles;
        let Taps::Exact {
            rows,
            singles,
            cycle,
        } = &self.filter.taps
        else {
            return 0;
        };
        let Clock::Exact {
            whole, numerator, ..
        } = &mut self.clock
        else {
            unreachable!("exact taps come with an exact clock");
        };
        let denominator = cycle.places.len();
        let (span, period) = (self.filter.span, cycle.period());
        if cycles == 0 || most < cycles * denominator {
            return 0;
        }
        let place = cycle.places[*numerator as usize];
        let start = (*whole - self.dropped) as usize;
        let base = start.wrapping_sub(cycle.offsets[place]);
        // The place and window of the output `offset` after the first
        let at = |offset: usize| {
            let (place, wrapped) = match place + offset {
                within if within < denominator => (within, 0),
                beyond => (beyond - denominator, period),
            };
            (place, base.wrapping_add(cycle.offsets[place] + wrapped))
        };
        let (_, last) = at(denominator - 1);
        if last + (cycles - 1) * period + span > self.buffer.len() {
            return 0;
        }
        let first = out.len();
        out.resize(first + cycles * denominator, T::ZERO);
        let made = &mut out[first..];
        let sums = &mut self.sums[..cycles];
        for offset in 0..denominator {
            let (place, window) = at(offset);
            let slots = made[offset..].iter_mut().step_by(denominator);
            if let Some(Some((index, tap))) = singles.get(place) {
                let samples = self.buffer[window + index..].iter().step_by(period);
                for (slot, sample) in slots.zip(samples) {
                    *slot = *tap * *sample;
                }
            } else {
                let row = &rows[cycle.rows[place]..][..span];
                T::fir(row, &self.buffer, window, period, sums);
                for (slot, sum) in slots.zip(sums.iter()) {
                    *slot = *sum;
                }
            }
        }
        *whole += (cycles * period) as u64;
        cycles * denominator
    }

    /// Up to `most` outputs whose taps are worked out at their times
    fn produce_each(&mut self, most: usize, out: &mut Vec<T>) -> usize {
        let span = self.filter.span;
        let mut count = 0;
        while count < most {
            let Some(start) = self.next_start() else {
                break;
            };
            self.work_out_row();
            out.push(T::dot_padded(&self.row, &self.buffer[start..][..span]));
            self.clock.advance();
            count += 1;
        }
        count
    }

    /// The taps at the clock's time, into [`Self::row`]
    fn work_out_row(&mut self) {
        let span = self.filter.span;
        match &self.filter.taps {
            Taps::Interpolated {
                rows,
                segments_log2,
                order,
            } => {
                let (segment, position) = segment_of(&self.clock, *segments_log2);
                let rows = &rows[segment * (order + 1) * span..][..(order + 1) * span];
                for (index, tap) in self.row.iter_mut().enumerate() {
                    let mut value = 0.0;
                    for power in (0..=*order).rev() {
                        value = value * position + rows[power * span + index];
                    }
                    *tap = T::from_double(value);
                }
            }
            Taps::Cubic => {
                let weights = lagrange_weights(self.clock.fraction_f64());
                for (tap, weight) in self.row.iter_mut().zip(weights) {
                    *tap = T::from_double(weight);
                }
            }
            Taps::Exact { .. } => unreachable!("exact taps have their own loop"),
        }
    }
}

/// Which segment the clock's fraction lies in, and where in it, from 0 to 1
fn segment_of(clock: &Clock, segments_log2: u32) -> (usize, f64) {
    match (clock.fraction(), clock) {
        (Fraction::Fixed(fraction), _) => {
            let segment = (fraction >> (64 - segments_log2)) as usize;
            let rest = fraction << segments_log2;
            (segment, rest as f64 / 18_446_744_073_709_551_616.0)
        }
        (Fraction::Exact(numerator), Clock::Exact { denominator, .. }) => {
            let scaled = u128::from(numerator) << segments_log2;
            let denominator = u128::from(*denominator);
            (
                (scaled / denominator) as usize,
                (scaled % denominator) as f64 / denominator as f64,
            )
        }
        (Fraction::Exact(_), Clock::Fixed { .. }) => unreachable!("the clock gave its own kind"),
    }
}
