//! A ratio that may change: band-limited interpolation at any time, through
//! one prototype low-pass filter that widens with the ratio when downsampling,
//! so that its band follows the lower rate

use std::sync::LazyLock;

use crate::Error;
use crate::chain::DC_SAMPLES;
use crate::design::{self, Kernel};
use crate::dither::Seeds;
use crate::io::{Input, Output, Sample};
use crate::lanes::{self, Gains, Lanes, Ready};
use crate::poly::lagrange;
use crate::real::dot;

/// The most the filter widens by. Over this ratio it widens no more, so that
/// its length stays bounded, and the output aliases.
const MOST_STRETCH: f64 = 64.0;
/// Of the prototype, in cycles per sample of the lower rate: 0.9 and 1 of its
/// Nyquist frequency
const PASSBAND_END: f64 = 0.45;
const STOPBAND_BEGIN: f64 = 0.5;
/// Of the prototype, as in the quality spec
const PRECISION: f64 = 20.0;
/// Parts of a sample that the prototype's table holds a cubic for
const SEGMENTS: usize = 128;
/// 2^64, for 64.64 fixed point
const FIXED_ONE: f64 = 18_446_744_073_709_551_616.0;
/// Input frames dropped at once from the front of the buffers, so that the
/// rest moves rarely
const DRAIN_LEN: i64 = 4096;
/// Output frames planned at a time
const BATCH_LEN: usize = 4096;
/// About what working out a tap costs, in multiply-adds
const TAP_WORK: usize = 4;

static PROTOTYPE: LazyLock<Prototype> = LazyLock::new(Prototype::new);

/// The low-pass filter at the lower rate, as a function of time
struct Prototype {
    half_len: f64,
    /// For each segment from `-half_len` on, the coefficients of a cubic in
    /// the position within it, from the constant up
    segments: Vec<[f64; 4]>,
    /// The same by where the segments lie within a sample: for each of the
    /// `SEGMENTS` places, the segments there from the first sample on, so that
    /// the taps of an upsampling filter are read in order
    places: Vec<[f64; 4]>,
    samples: usize,
}

impl Prototype {
    fn new() -> Self {
        let attenuation = design::attenuation(PRECISION);
        let kernel = Kernel::low_pass(PASSBAND_END, STOPBAND_BEGIN, attenuation, 1.0);
        let kernel = kernel.with_gain(1.0 / kernel.dc_gain(DC_SAMPLES));
        let half_len = kernel.half_len();
        let at = |node: f64| kernel.at(node / SEGMENTS as f64 - half_len);
        // One more than the span needs, for a time that rounds onto its end
        let segments = (0..=(2.0 * half_len * SEGMENTS as f64) as usize + 1)
            .map(|segment| {
                let base = segment as f64;
                lagrange([-1.0, 0.0, 1.0, 2.0].map(|node| at(base + node)))
            })
            .collect::<Vec<_>>();
        let samples = segments.len().div_ceil(SEGMENTS);
        let places = (0..SEGMENTS)
            .flat_map(|place| (0..samples).map(move |sample| place + sample * SEGMENTS))
            .map(|segment| segments.get(segment).copied().unwrap_or_default())
            .collect();
        Self {
            half_len,
            segments,
            places,
            samples,
        }
    }

    /// At `time` samples from its centre, less than its half length either
    /// way
    fn at(&self, time: f64) -> f64 {
        let position = (time + self.half_len) * SEGMENTS as f64;
        let segment = position as usize;
        self.cubic(segment, position - segment as f64)
    }

    /// Of `segment`, at `x` from 0 to 1 along it
    fn cubic(&self, segment: usize, x: f64) -> f64 {
        let [c0, c1, c2, c3] = self.segments[segment];
        ((c3 * x + c2) * x + c1) * x + c0
    }

    /// Input frames that every output frame keeps behind it, for the widest
    /// filter
    fn history(&self) -> i64 {
        (self.half_len * MOST_STRETCH).ceil() as i64 + 2
    }
}

/// The ratio moving linearly from `from` to `to` over `len` output frames, of
/// which `done` have gone
#[derive(Clone, Copy, Debug)]
struct Slew {
    from: f64,
    to: f64,
    len: u64,
    done: u64,
}

/// The time of the next output frame, and how it steps
#[derive(Clone, Copy, Debug)]
struct Clock {
    output: u64,
    /// In input frames, in 64.64 fixed point
    time: u128,
    /// Once any slew is over
    ratio: f64,
    slew: Option<Slew>,
    /// The output frame and the time from which the ratio has been constant.
    /// The end of the output is counted from there as a constant ratio counts
    /// it from the start, so that a ratio that never changes ends where a
    /// constant one does.
    since: (u64, u128),
}

impl Clock {
    fn new(ratio: f64) -> Self {
        Self {
            output: 0,
            time: 0,
            ratio,
            slew: None,
            since: (0, 0),
        }
    }

    /// From the next output frame to the one after. Over a slew, the ratio
    /// halfway through each step, so that the slew moves the time by its
    /// length times the mean of the two ratios.
    fn step(&self) -> f64 {
        match self.slew {
            Some(slew) => {
                let along = (slew.done as f64 + 0.5) / slew.len as f64;
                slew.from + (slew.to - slew.from) * along
            }
            None => self.ratio,
        }
    }

    fn advance(&mut self) {
        self.time += (self.step() * FIXED_ONE).round() as u128;
        self.output += 1;
        if let Some(slew) = &mut self.slew {
            slew.done += 1;
            if slew.done == slew.len {
                self.slew = None;
                self.since = (self.output, self.time);
            }
        }
    }

    fn set(&mut self, ratio: f64, slew_len: u64) {
        if slew_len > 0 {
            self.slew = Some(Slew {
                from: self.step(),
                to: ratio,
                len: slew_len,
                done: 0,
            });
        } else if self.slew.is_some() || ratio != self.ratio {
            self.slew = None;
            self.since = (self.output, self.time);
        }
        self.ratio = ratio;
    }

    /// Whether the output that `input_frames` of input make once it ends has
    /// the next frame. Each frame lies within the input by at least half of
    /// its step.
    fn has_next(&self, input_frames: u64) -> bool {
        match self.slew {
            Some(_) => span(input_frames, self.time) >= self.step() / 2.0,
            None => self.output - self.since.0 < self.constant_total(input_frames),
        }
    }

    /// Of the output that `input_frames` of input make once it ends, the
    /// frames from the next on
    fn left(&self, input_frames: u64) -> u64 {
        let mut clock = *self;
        let mut left = 0;
        while clock.slew.is_some() {
            if !clock.has_next(input_frames) {
                return left;
            }
            left += 1;
            clock.advance();
        }
        let total = clock.constant_total(input_frames);
        left + total.saturating_sub(clock.output - clock.since.0)
    }

    /// The output frames from `since` on, at a constant ratio
    fn constant_total(&self, input_frames: u64) -> u64 {
        let span = span(input_frames, self.since.1);
        (span / self.ratio + 0.5).floor().max(0.0) as u64
    }
}

/// From `time` to the end of `input_frames` frames, in input frames
fn span(input_frames: u64, time: u128) -> f64 {
    ((u128::from(input_frames) << 64) as i128 - time as i128) as f64 / FIXED_ONE
}

/// Where an output frame's window lies in the buffers, and where the output
/// frame lies within it
#[derive(Clone, Copy, Debug)]
struct Window {
    start: usize,
    len: usize,
    /// From the first input frame of the window, in input frames
    offset: f64,
    /// Of the prototype's time to input frames: 1 when upsampling, and the
    /// inverse of the ratio, held to [`MOST_STRETCH`], when downsampling
    scale: f64,
}

impl Window {
    fn taps(&self, prototype: &Prototype, taps: &mut Vec<f64>) {
        taps.clear();
        if self.scale == 1.0 {
            // Every tap lies as far along its segment, a sample apart
            let position = (self.offset + prototype.half_len) * SEGMENTS as f64;
            let first = position as usize;
            let x = position - first as f64;
            let place = first % SEGMENTS * prototype.samples;
            let place = &prototype.places[place..][..prototype.samples];
            let last = first / SEGMENTS;
            let from = (last + 1).saturating_sub(self.len);
            taps.extend(
                place[from..=last]
                    .iter()
                    .rev()
                    .map(|&[c0, c1, c2, c3]| ((c3 * x + c2) * x + c1) * x + c0),
            );
            taps.resize(self.len, 0.0);
        } else {
            taps.extend(
                (0..self.len)
                    .map(|tap| self.scale * prototype.at(self.scale * (self.offset - tap as f64))),
            );
        }
    }
}

/// Output frames for one thread to work out: their windows, and for each
/// channel its input and where its output goes
struct Part<'a> {
    windows: &'a [Window],
    channels: Vec<(&'a [f64], &'a mut [f64])>,
}

impl Part<'_> {
    fn compute(&mut self) {
        let mut taps = Vec::new();
        for (frame, window) in self.windows.iter().enumerate() {
            window.taps(&PROTOTYPE, &mut taps);
            for (input, output) in &mut self.channels {
                output[frame] = dot(&taps, &input[window.start..][..window.len]);
            }
        }
    }
}

struct Channel {
    index: usize,
    /// Input from the frame that `VariableLanes::first` names, in the
    /// engine's units, with zeros before the input and after its end
    buffer: Vec<f64>,
    ready: Ready<f64>,
}

pub(crate) struct VariableLanes {
    channels: Vec<Channel>,
    gains: Gains,
    threads: usize,
    initial_ratio: f64,
    clock: Clock,
    first: i64,
    /// Input frames so far
    received: u64,
    windows: Vec<Window>,
}

impl VariableLanes {
    pub(crate) fn new(
        channels: usize,
        io_ratio: f64,
        gains: Gains,
        dither: Option<Seeds>,
        threads: usize,
    ) -> Self {
        let channels = (0..channels)
            .map(|index| Channel {
                index,
                buffer: Vec::new(),
                ready: Ready::new(dither, index),
            })
            .collect();
        let mut lanes = Self {
            channels,
            gains,
            threads,
            initial_ratio: io_ratio,
            clock: Clock::new(io_ratio),
            first: 0,
            received: 0,
            windows: Vec::new(),
        };
        lanes.restart();
        lanes
    }

    fn restart(&mut self) {
        let history = PROTOTYPE.history();
        self.clock = Clock::new(self.initial_ratio);
        self.first = -history;
        self.received = 0;
        for channel in &mut self.channels {
            channel.buffer.clear();
            channel.buffer.resize(history as usize, 0.0);
            channel.ready.clear();
        }
    }

    fn buffered(&self) -> i64 {
        self.first
            + self
                .channels
                .first()
                .map_or(0, |channel| channel.buffer.len()) as i64
    }

    /// Plans up to `frames` output frames, as far as the input so far goes,
    /// or once it has ended as far as the output goes, with zeros after it
    fn plan(&mut self, frames: usize, ended: Option<u64>) {
        let prototype = &*PROTOTYPE;
        self.windows.clear();
        while self.windows.len() < frames {
            let ratio = self.clock.step();
            let scale = 1.0 / ratio.clamp(1.0, MOST_STRETCH);
            let reach = prototype.half_len / scale;
            let whole = (self.clock.time >> 64) as i64;
            let fraction = (self.clock.time as u64) as f64 / FIXED_ONE;
            let low = whole + (fraction - reach).floor() as i64 + 1;
            let high = whole + (fraction + reach).ceil() as i64 - 1;
            match ended {
                Some(input_frames) if !self.clock.has_next(input_frames) => break,
                Some(_) => {
                    let len = (high + 1 - self.first) as usize;
                    for channel in &mut self.channels {
                        if channel.buffer.len() < len {
                            channel.buffer.resize(len, 0.0);
                        }
                    }
                }
                None if high >= self.buffered() => break,
                None => {}
            }
            self.windows.push(Window {
                start: (low - self.first) as usize,
                len: (high - low + 1) as usize,
                offset: (whole - low) as f64 + fraction,
                scale,
            });
            self.clock.advance();
        }
    }

    /// Appends the output frames planned to every channel, each frame's taps
    /// worked out once for all of them, and the frames shared out over the
    /// threads
    fn compute(&mut self) {
        let windows = &self.windows;
        let taps: usize = windows.iter().map(|window| window.len).sum();
        let work = taps * (self.channels.len() + TAP_WORK);
        let threads = lanes::useful_threads(self.threads, work).min(windows.len());
        let per_part = windows.len().div_ceil(threads.max(1)).max(1);
        let mut parts: Vec<Part<'_>> = windows
            .chunks(per_part)
            .map(|windows| Part {
                windows,
                channels: Vec::with_capacity(self.channels.len()),
            })
            .collect();
        for channel in &mut self.channels {
            let queue = &mut channel.ready.queue;
            let from = queue.len();
            queue.resize(from + windows.len(), 0.0);
            for (part, output) in parts.iter_mut().zip(queue[from..].chunks_mut(per_part)) {
                part.channels.push((&channel.buffer, output));
            }
        }
        lanes::each(&mut parts, threads, |parts| {
            for part in parts {
                part.compute();
            }
        });
    }

    /// Drops the input that no output frame to come can reach. Over a ratio
    /// of 64 that may be all of it.
    fn drain(&mut self) {
        let reached = (self.clock.time >> 64) as i64 - PROTOTYPE.history();
        let keep = reached.min(self.buffered());
        if keep - self.first >= DRAIN_LEN {
            let dropped = (keep - self.first) as usize;
            for channel in &mut self.channels {
                channel.buffer.drain(..dropped);
            }
            self.first = keep;
        }
    }
}

impl<I: Sample, O: Sample> Lanes<I, O> for VariableLanes {
    fn push(&mut self, input: &Input<'_, I>, frames: usize) {
        let count = self.channels.len();
        for channel in &mut self.channels {
            channel.ready.compact();
            let gain = self.gains.input;
            input
                .channel(channel.index, count, 0)
                .extend(frames, &mut channel.buffer, |sample| sample.to_f64() * gain);
        }
        self.received += frames as u64;
    }

    fn queued(&self) -> usize {
        self.channels
            .first()
            .map_or(0, |channel| channel.ready.len())
    }

    fn fill(&mut self, frames: usize, ended: bool) {
        let ended = ended.then_some(self.received);
        loop {
            let batch = frames
                .saturating_sub(<Self as Lanes<I, O>>::queued(self))
                .min(BATCH_LEN);
            if batch == 0 {
                break;
            }
            self.plan(batch, ended);
            let planned = self.windows.len();
            self.compute();
            if planned < batch {
                break;
            }
        }
        self.drain();
    }

    fn write(&mut self, output: &mut Output<'_, '_, O>, start: usize, frames: usize) -> u64 {
        let mut channels: Vec<_> = self
            .channels
            .iter_mut()
            .map(|channel| &mut channel.ready)
            .collect();
        lanes::write(&mut channels, self.gains.output, output, start, frames)
    }

    fn clear(&mut self) {
        self.restart();
    }

    fn io_ratio(&self) -> f64 {
        self.clock.step()
    }

    fn set_io_ratio(&mut self, io_ratio: f64, slew_len: usize) -> Result<(), Error> {
        self.clock.set(io_ratio, slew_len as u64);
        Ok(())
    }

    fn left(&self, input_frames: u64, _: u64) -> u64 {
        <Self as Lanes<I, O>>::queued(self) as u64 + self.clock.left(input_frames)
    }

    fn reach(&self, input_frames: u64, _: u64) -> f64 {
        let queued = <Self as Lanes<I, O>>::queued(self) as f64;
        queued + span(input_frames, self.clock.time) / self.clock.step()
    }
}

#[cfg(test)]
mod tests;
