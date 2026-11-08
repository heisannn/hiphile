//! The channels: conversion from the input type, the stages of a constant
//! ratio, the queue of output, and conversion to the output type

use std::thread;

use crate::Error;
use crate::chain::{Pipeline, StageDesign};
use crate::dither::{Seeds, Tpdf};
use crate::io::{Input, Output, Sample};
use crate::real::Real;

/// Zeros fed at a time after the end of the input
const FLUSH_LEN: usize = 1024;
/// How close a ratio given to a resampler that keeps one must be to its own,
/// relative to it, to be taken as the same
const SAME_RATIO: f64 = 5e-16;
/// Below this many multiply-adds in a call, threads cost more than they save
const LEAST_PARALLEL_WORK: usize = 1 << 19;
/// About what a sample of a constant ratio costs, in multiply-adds, at HQ
const SAMPLE_WORK: usize = 32;

/// Every channel of a resampler, behind the types it was made for
pub(crate) trait Lanes<I, O>: Send {
    /// Takes `frames` of `input` and runs them through every channel
    fn push(&mut self, input: &Input<'_, I>, frames: usize);
    /// Frames of output ready in every channel
    fn queued(&self) -> usize;
    /// Makes ready up to `frames`, as far as the input so far goes, and past
    /// its end with zeros once it has `ended`
    fn fill(&mut self, frames: usize, ended: bool);
    /// Writes the first `frames` ready to `output` from its frame `start` on,
    /// and returns how many samples were clipped
    fn write(&mut self, output: &mut Output<'_, '_, O>, start: usize, frames: usize) -> u64;
    /// Back to the state after creation, with the ratio of creation
    fn clear(&mut self);
    /// Input frames per output frame, from the next output frame to the one
    /// after
    fn io_ratio(&self) -> f64;
    fn set_io_ratio(&mut self, io_ratio: f64, slew_len: usize) -> Result<(), Error>;
    /// Of the output that `input_frames` of input make once it ends, the
    /// frames after the first `output_frames`
    fn left(&self, input_frames: u64, output_frames: u64) -> u64;
    /// In output frames, how far `input_frames` of input reach past the time
    /// of output frame `output_frames`
    fn reach(&self, input_frames: u64, output_frames: u64) -> f64;
}

/// Conversions to and from the engine's units, where full scale is 1
#[derive(Clone, Copy, Debug)]
pub(crate) struct Gains {
    pub(crate) input: f64,
    pub(crate) output: f64,
}

/// A channel's output that is ready, and the dither it goes out with
pub(crate) struct Ready<T> {
    pub(crate) queue: Vec<T>,
    read: usize,
    dither: Option<Tpdf>,
}

impl<T: Real> Ready<T> {
    pub(crate) fn new(dither: Option<Seeds>, channel: usize) -> Self {
        Self {
            queue: Vec::new(),
            read: 0,
            dither: dither.map(|seeds| Tpdf::new(seeds, channel)),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.queue.len() - self.read
    }

    /// Drops what was written, once it is most of the queue
    pub(crate) fn compact(&mut self) {
        if self.read > 0 && self.read * 2 >= self.queue.len() {
            self.queue.drain(..self.read);
            self.read = 0;
        }
    }

    pub(crate) fn clear(&mut self) {
        self.queue.clear();
        self.read = 0;
        if let Some(dither) = &mut self.dither {
            dither.restart();
        }
    }
}

/// Writes the first `frames` ready in every channel to `output` from its
/// frame `start` on, and returns how many samples were clipped
pub(crate) fn write<T: Real, O: Sample>(
    channels: &mut [&mut Ready<T>],
    gain: f64,
    output: &mut Output<'_, '_, O>,
    start: usize,
    frames: usize,
) -> u64 {
    let count = channels.len();
    let mut clips = 0;
    // Both channels of interleaved stereo in one loop that the compiler
    // vectorizes, as one each is strided
    if let ([left, right], Output::Interleaved(buffer)) = (&mut *channels, &mut *output)
        && left.dither.is_none()
        && right.dither.is_none()
    {
        let slots = buffer[start * 2..][..frames * 2].as_chunks_mut::<2>().0;
        let samples = left.queue[left.read..][..frames]
            .iter()
            .zip(&right.queue[right.read..][..frames]);
        if gain == 1.0 {
            for (slot, (left, right)) in slots.iter_mut().zip(samples) {
                slot[0] = O::from_f64(left.to_double(), &mut clips);
                slot[1] = O::from_f64(right.to_double(), &mut clips);
            }
        } else {
            for (slot, (left, right)) in slots.iter_mut().zip(samples) {
                slot[0] = O::from_f64(left.to_double() * gain, &mut clips);
                slot[1] = O::from_f64(right.to_double() * gain, &mut clips);
            }
        }
        left.read += frames;
        right.read += frames;
        return clips;
    }
    for (channel, ready) in channels.iter_mut().enumerate() {
        let samples = &ready.queue[ready.read..ready.read + frames];
        let (slots, stride) = output.channel(channel, count, start);
        let slots = slots.chunks_mut(stride).zip(samples);
        match &mut ready.dither {
            Some(dither) => {
                for (slot, sample) in slots {
                    let value = sample.to_double() * gain + dither.next_value();
                    slot[0] = O::from_f64(value, &mut clips);
                }
            }
            None if gain == 1.0 => {
                for (slot, sample) in slots {
                    slot[0] = O::from_f64(sample.to_double(), &mut clips);
                }
            }
            None => {
                for (slot, sample) in slots {
                    slot[0] = O::from_f64(sample.to_double() * gain, &mut clips);
                }
            }
        }
        ready.read += frames;
    }
    clips
}

/// Of `threads`, those worth using for `work` multiply-adds
pub(crate) fn useful_threads(threads: usize, work: usize) -> usize {
    if work < LEAST_PARALLEL_WORK {
        1
    } else {
        threads
    }
}

/// Runs `work` on all of `items`, or on parts of them on up to `threads`
/// threads. Each item takes the same steps either way, so the output does not
/// depend on the threads.
pub(crate) fn each<L: Send>(items: &mut [L], threads: usize, work: impl Fn(&mut [L]) + Sync) {
    let threads = threads.min(items.len());
    if threads <= 1 {
        work(items);
        return;
    }
    let work = &work;
    let per_thread = items.len().div_ceil(threads);
    thread::scope(|scope| {
        for part in items.chunks_mut(per_thread) {
            scope.spawn(move || work(part));
        }
    });
}

struct Lane<T> {
    channel: usize,
    pipeline: Pipeline<T>,
    ready: Ready<T>,
    converted: Vec<T>,
}

pub(crate) struct ConstantLanes<T> {
    lanes: Vec<Lane<T>>,
    gains: Gains,
    io_ratio: f64,
    threads: usize,
}

impl<T: Real> ConstantLanes<T> {
    pub(crate) fn new(
        designs: &[StageDesign<T>],
        channels: usize,
        io_ratio: f64,
        gains: Gains,
        dither: Option<Seeds>,
        threads: usize,
    ) -> Self {
        let lanes = (0..channels)
            .map(|channel| Lane {
                channel,
                pipeline: Pipeline::new(designs),
                ready: Ready::new(dither, channel),
                converted: Vec::new(),
            })
            .collect();
        Self {
            lanes,
            gains,
            io_ratio,
            threads,
        }
    }
}

impl<T: Real, I: Sample, O: Sample> Lanes<I, O> for ConstantLanes<T> {
    fn push(&mut self, input: &Input<'_, I>, frames: usize) {
        let channels = self.lanes.len();
        let gain = self.gains.input;
        let threads = useful_threads(self.threads, frames * channels * SAMPLE_WORK);
        let split = match &mut self.lanes[..] {
            [left, right] => {
                let (left, right) = (&mut left.converted, &mut right.converted);
                left.clear();
                right.clear();
                if gain == 1.0 {
                    input.split_stereo(frames, left, right, |sample| {
                        T::from_double(sample.to_f64())
                    })
                } else {
                    input.split_stereo(frames, left, right, |sample| {
                        T::from_double(sample.to_f64() * gain)
                    })
                }
            }
            _ => false,
        };
        each(&mut self.lanes, threads, |lanes| {
            for lane in lanes {
                if !split {
                    lane.converted.clear();
                    let channel = input.channel(lane.channel, channels, 0);
                    if gain == 1.0 {
                        channel.extend(frames, &mut lane.converted, |sample| {
                            T::from_double(sample.to_f64())
                        });
                    } else {
                        channel.extend(frames, &mut lane.converted, |sample| {
                            T::from_double(sample.to_f64() * gain)
                        });
                    }
                }
                lane.ready.compact();
                lane.pipeline.push(&lane.converted, &mut lane.ready.queue);
            }
        });
    }

    fn queued(&self) -> usize {
        self.lanes.first().map_or(0, |lane| lane.ready.len())
    }

    fn fill(&mut self, frames: usize, ended: bool) {
        let threads = useful_threads(self.threads, frames * self.lanes.len() * SAMPLE_WORK);
        each(&mut self.lanes, threads, |lanes| {
            for lane in lanes {
                loop {
                    let missing = frames.saturating_sub(lane.ready.len());
                    lane.pipeline.produce(missing, &mut lane.ready.queue);
                    if !ended || lane.ready.len() >= frames {
                        break;
                    }
                    lane.pipeline.push_zeros(FLUSH_LEN, &mut lane.ready.queue);
                }
            }
        });
    }

    fn write(&mut self, output: &mut Output<'_, '_, O>, start: usize, frames: usize) -> u64 {
        let mut channels: Vec<_> = self.lanes.iter_mut().map(|lane| &mut lane.ready).collect();
        write(&mut channels, self.gains.output, output, start, frames)
    }

    fn clear(&mut self) {
        for lane in &mut self.lanes {
            lane.pipeline.clear();
            lane.ready.clear();
        }
    }

    fn io_ratio(&self) -> f64 {
        self.io_ratio
    }

    fn set_io_ratio(&mut self, io_ratio: f64, _: usize) -> Result<(), Error> {
        if (io_ratio - self.io_ratio).abs() <= SAME_RATIO * self.io_ratio {
            Ok(())
        } else {
            Err(Error::VaryingRatio)
        }
    }

    fn left(&self, input_frames: u64, output_frames: u64) -> u64 {
        let total = (input_frames as f64 / self.io_ratio + 0.5).floor() as u64;
        total.saturating_sub(output_frames)
    }

    fn reach(&self, input_frames: u64, output_frames: u64) -> f64 {
        input_frames as f64 / self.io_ratio - output_frames as f64
    }
}

#[cfg(test)]
mod tests;
