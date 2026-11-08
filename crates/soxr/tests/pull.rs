//! Output that pulls its input

mod common;

use std::num::NonZeroUsize;

use common::{Random, noise, oneshot, spec, total_output};
use soxr::{Error, Input, Quality, Resampler, Source, SourceError};

const RATES: (f64, f64) = (48_000.0, 44_100.0);

/// Interleaved samples in memory, which records what it is asked for and can
/// fail from a pull on
struct Memory<'a> {
    samples: &'a [f32],
    channels: usize,
    taken: usize,
    most: Option<NonZeroUsize>,
    fails_at: Option<usize>,
    requests: Vec<usize>,
}

impl<'a> Memory<'a> {
    fn new(samples: &'a [f32], channels: usize) -> Self {
        Self {
            samples,
            channels,
            taken: 0,
            most: None,
            fails_at: None,
            requests: Vec::new(),
        }
    }

    fn at_most(self, frames: usize) -> Self {
        Self {
            most: NonZeroUsize::new(frames),
            ..self
        }
    }

    fn failing_at(self, pull: usize) -> Self {
        Self {
            fails_at: Some(pull),
            ..self
        }
    }
}

impl Source<f32> for Memory<'_> {
    fn pull(&mut self, frames: usize) -> Result<Input<'_, f32>, SourceError> {
        if self.fails_at == Some(self.requests.len()) {
            return Err(SourceError);
        }
        self.requests.push(frames);
        let start = self.taken * self.channels;
        let given = frames.min(self.samples.len() / self.channels - self.taken);
        self.taken += given;
        Ok(Input::Interleaved(
            &self.samples[start..start + given * self.channels],
        ))
    }

    fn most_frames(&self) -> Option<NonZeroUsize> {
        self.most
    }
}

/// Calls `output` with room for `room` frames until it gives none
fn drain(
    resampler: &mut Resampler,
    source: &mut (impl Source<f32> + ?Sized),
    room: usize,
) -> Vec<f32> {
    let channels = resampler.channels();
    let mut output = Vec::new();
    let mut buffer = vec![0.0f32; room * channels];
    loop {
        let written = resampler.output(source, &mut buffer[..]).unwrap();
        if written == 0 {
            return output;
        }
        output.extend_from_slice(&buffer[..written * channels]);
    }
}

fn same_bits(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits())
}

#[test]
fn every_pull_asks_for_the_most_frames_of_the_source() {
    let input = noise(50_000, 3);
    let mut resampler = Resampler::<f32, f32>::new(RATES.0, RATES.1, 1).unwrap();
    let mut source = Memory::new(&input, 1).at_most(1000);
    let output = drain(&mut resampler, &mut source, 4410);
    assert_eq!(output.len(), 45_937);
    assert!(source.requests.iter().all(|&request| request == 1000));
    let whole: Vec<f32> = oneshot(RATES, spec(Quality::High), 1, &input);
    assert!(same_bits(&output, &whole));
}

/// With no most, a call asks for what its room needs, the same every time
#[test]
fn every_pull_of_a_call_asks_for_what_its_room_needs() {
    let input = noise(50_000, 3);
    let mut resampler = Resampler::<f32, f32>::new(RATES.0, RATES.1, 1).unwrap();
    let mut source = Memory::new(&input, 1);
    let output = drain(&mut resampler, &mut source, 4410);
    assert_eq!(output.len(), 45_937);
    assert!(source.requests.iter().all(|&request| request == 4800));

    let ratio = resampler.io_ratio();
    let mut random = Random::new(9);
    resampler.clear();
    let mut source = Memory::new(&input, 1);
    let mut output = Vec::new();
    loop {
        let room = random.len(2000);
        let asked = source.requests.len();
        let mut buffer = vec![0.0f32; room];
        let written = resampler.output(&mut source, &mut buffer[..]).unwrap();
        let needed = (room as f64 * ratio).ceil() as usize;
        assert!(source.requests[asked..].iter().all(|&r| r == needed));
        if written == 0 {
            break;
        }
        output.extend_from_slice(&buffer[..written]);
    }
    let whole: Vec<f32> = oneshot(RATES, spec(Quality::High), 1, &input);
    assert!(same_bits(&output, &whole));
}

/// The most frames are the source's, which libsoxr lost on clearing
#[test]
fn clearing_keeps_the_most_frames() {
    let input = noise(50_000, 3);
    let mut resampler = Resampler::<f32, f32>::new(RATES.0, RATES.1, 1).unwrap();
    let mut source = Memory::new(&input, 1).at_most(1000);
    let first = drain(&mut resampler, &mut source, 4410);
    resampler.clear();
    let mut source = Memory::new(&input, 1).at_most(1000);
    let again = drain(&mut resampler, &mut source, 4410);
    assert_eq!(again.len(), 45_937);
    assert!(source.requests.iter().all(|&request| request == 1000));
    assert!(same_bits(&first, &again));
}

/// A failure stops the resampler until it is cleared
#[test]
fn a_failing_source_stops_the_resampler() {
    let input = noise(10_000, 4);
    let mut resampler = Resampler::<f32, f32>::new(RATES.0, RATES.1, 1).unwrap();
    let mut room = vec![0.0f32; 4410];
    let mut source = Memory::new(&input, 1).failing_at(0);
    assert_eq!(
        resampler.output(&mut source, &mut room[..]),
        Err(Error::InputFailure)
    );
    assert_eq!(resampler.error(), Some(Error::InputFailure));
    let mut source = Memory::new(&input, 1);
    assert_eq!(
        resampler.output(&mut source, &mut room[..]),
        Err(Error::InputFailure)
    );
    assert!(source.requests.is_empty());
    assert_eq!(
        resampler.resample(&input[..], &mut room[..]),
        Err(Error::InputFailure)
    );
    assert_eq!(resampler.finish(&mut room[..]), Err(Error::InputFailure));
    resampler.clear();
    assert_eq!(resampler.error(), None);
    assert_eq!(
        drain(&mut resampler, &mut source, 4410).len(),
        total_output(10_000, RATES)
    );
}

/// A failure after some output gives that output, and the error on the next
/// call
#[test]
fn a_failure_after_some_output_comes_with_the_next_call() {
    let input = noise(10_000, 4);
    let mut resampler = Resampler::<f32, f32>::new(RATES.0, RATES.1, 1).unwrap();
    let mut source = Memory::new(&input, 1).at_most(100).failing_at(40);
    let mut room = vec![0.0f32; 4410];
    let written = resampler.output(&mut source, &mut room[..]).unwrap();
    assert!(written > 0);
    assert_eq!(source.requests.len(), 40);
    assert_eq!(resampler.error(), Some(Error::InputFailure));
    assert_eq!(
        resampler.output(&mut source, &mut room[..]),
        Err(Error::InputFailure)
    );
}

/// The same input on every pull
struct Fixed<'a>(Input<'a, f32>);

impl Source<f32> for Fixed<'_> {
    fn pull(&mut self, _: usize) -> Result<Input<'_, f32>, SourceError> {
        Ok(self.0)
    }
}

#[test]
fn input_beyond_the_request_or_off_the_channels_is_a_failure() {
    let input = noise(10_000, 4);
    let mut room = vec![0.0f32; 882];
    let mut resampler = Resampler::<f32, f32>::new(RATES.0, RATES.1, 1).unwrap();
    let mut too_much = Fixed(Input::Interleaved(&input[..]));
    assert_eq!(
        resampler.output(&mut too_much, &mut room[..]),
        Err(Error::InputFailure)
    );
    let mut resampler = Resampler::<f32, f32>::new(RATES.0, RATES.1, 2).unwrap();
    let mut off_channels = Fixed(Input::Interleaved(&input[..3]));
    assert_eq!(
        resampler.output(&mut off_channels, &mut room[..]),
        Err(Error::InputFailure)
    );
}

#[test]
fn no_room_asks_for_nothing() {
    let input = noise(10_000, 4);
    let mut resampler = Resampler::<f32, f32>::new(RATES.0, RATES.1, 1).unwrap();
    let mut source = Memory::new(&input, 1);
    assert_eq!(resampler.output(&mut source, &mut [0.0f32; 0][..]), Ok(0));
    assert!(source.requests.is_empty());
    assert_eq!(resampler.error(), None);
}

/// Two channels in buffers of their own, given in random lengths up to what
/// is asked for
struct Split<'a> {
    channels: [&'a [f32]; 2],
    taken: usize,
    random: Random,
    lent: [&'a [f32]; 2],
}

impl Source<f32> for Split<'_> {
    fn pull(&mut self, frames: usize) -> Result<Input<'_, f32>, SourceError> {
        let [left, right] = self.channels;
        let start = self.taken;
        self.taken += self.random.len(frames).min(left.len() - start);
        self.lent = [&left[start..self.taken], &right[start..self.taken]];
        Ok(Input::Split(&self.lent))
    }
}

/// Split input and output, and a source that gives fewer frames than asked
/// for, change no bit
#[test]
fn any_layout_and_any_pull_give_the_same_bits() {
    let left = noise(20_000, 5);
    let right = noise(20_000, 6);
    let interleaved: Vec<f32> = left
        .iter()
        .zip(&right)
        .flat_map(|(l, r)| [*l, *r])
        .collect();
    let whole: Vec<f32> = oneshot(RATES, spec(Quality::High), 2, &interleaved);
    let mut resampler = Resampler::<f32, f32>::new(RATES.0, RATES.1, 2).unwrap();
    let mut source = Split {
        channels: [&left, &right],
        taken: 0,
        random: Random::new(8),
        lent: [&[], &[]],
    };
    let mut output = [Vec::new(), Vec::new()];
    let mut buffers = [vec![0.0f32; 700], vec![0.0f32; 700]];
    loop {
        let [l, r] = &mut buffers;
        let mut room = [&mut l[..], &mut r[..]];
        let written = resampler.output(&mut source, &mut room[..]).unwrap();
        if written == 0 {
            break;
        }
        for (channel, buffer) in output.iter_mut().zip(&buffers) {
            channel.extend_from_slice(&buffer[..written]);
        }
    }
    assert_eq!(output[0].len(), total_output(20_000, RATES));
    let joined: Vec<f32> = output[0]
        .iter()
        .zip(&output[1])
        .flat_map(|(l, r)| [*l, *r])
        .collect();
    assert!(same_bits(&joined, &whole));
}

/// Pushed and pulled input make one stream
#[test]
fn pushing_and_pulling_mix() {
    let input = noise(30_000, 7);
    let whole: Vec<f32> = oneshot(RATES, spec(Quality::High), 1, &input);
    let mut resampler = Resampler::<f32, f32>::new(RATES.0, RATES.1, 1).unwrap();
    let mut output = Vec::new();
    let mut room = vec![0.0f32; 1500];
    let mut taken = 0;
    while taken < 20_000 {
        let progress = resampler
            .resample(&input[taken..20_000], &mut room[..])
            .unwrap();
        taken += progress.input_frames;
        output.extend_from_slice(&room[..progress.output_frames]);
        let mut source = Memory::new(&input[taken..], 1).at_most(300);
        let written = resampler.output(&mut source, &mut room[..900]).unwrap();
        taken += source.taken;
        output.extend_from_slice(&room[..written]);
    }
    let mut source = Memory::new(&input[taken..], 1);
    output.extend(drain(&mut resampler, &mut source, 1500));
    assert!(same_bits(&output, &whole));
}
