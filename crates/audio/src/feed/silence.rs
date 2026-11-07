use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::Item;
use crate::output::Spec;

/// -60 dBFS, in every channel. A recording's noise floor is usually well under
/// it, and the music seldom is.
const SILENCE_CEILING: f32 = 0.001;

/// A pause in the music, kept as it is. Anything longer is no part of the
/// performance, such as the gap before a hidden track, and goes the way of the
/// silence around a track. It also bounds what has to be kept in memory.
const LONGEST_PAUSE: Duration = Duration::from_secs(10);

/// While on, leaves out the silence at the start and end of a track and after
/// each seek, and any silence longer than [`LONGEST_PAUSE`].
///
/// A position in the track stays what it was in the file, as lyrics and seeks
/// go by, so what is left out is reported as an [`Item::Skipped`] ahead of the
/// sound after it. A seek is reported once its sound is found.
pub(super) struct SilenceGate {
    spec: Spec,
    is_on: Arc<AtomicBool>,
    /// Where decoding last started, and the frames decoded since.
    from: Duration,
    frames: u64,
    /// Silence is being dropped, rather than kept in case sound follows.
    is_dropping: bool,
    owes_report: bool,
    /// The silence since the last sound, which goes out only if sound follows.
    held: Vec<f32>,
}

impl SilenceGate {
    pub(super) fn new(spec: Spec, is_on: Arc<AtomicBool>) -> Self {
        Self {
            spec,
            is_on,
            from: Duration::ZERO,
            frames: 0,
            is_dropping: true,
            owes_report: false,
            held: Vec::new(),
        }
    }

    fn channels(&self) -> usize {
        usize::from(self.spec.channels)
    }

    fn position(&self, frames: u64) -> Duration {
        self.from + Duration::from_secs_f64(frames as f64 / f64::from(self.spec.rate))
    }

    fn longest_pause_samples(&self) -> usize {
        (LONGEST_PAUSE.as_secs_f64() * f64::from(self.spec.rate)) as usize * self.channels()
    }

    /// `resumed` is where decoding went on after a seek, or `None` past the end.
    pub(super) fn seeked(&mut self, resumed: Option<Duration>, out: &mut Vec<Item>) {
        self.held.clear();
        let Some(resumed) = resumed else {
            out.push(Item::Seeked(None));
            return;
        };
        self.from = resumed;
        self.frames = 0;
        if self.is_on.load(Ordering::Relaxed) {
            self.is_dropping = true;
            self.owes_report = true;
        } else {
            self.is_dropping = false;
            out.push(Item::Seeked(Some(resumed)));
        }
    }

    pub(super) fn pass(&mut self, samples: Vec<f32>, out: &mut Vec<Item>) {
        let channels = self.channels();
        let frames = samples.len() / channels;
        let sound = if self.is_on.load(Ordering::Relaxed) {
            sound_within(&samples, channels)
        } else {
            Some((0, frames))
        };
        let first_frame = self.frames;
        self.frames += frames as u64;
        let Some((first, end)) = sound else {
            if !self.is_dropping {
                self.held.extend(samples);
                if self.held.len() > self.longest_pause_samples() {
                    self.held = Vec::new();
                    self.is_dropping = true;
                }
            }
            return;
        };
        let start = if self.is_dropping {
            let at = first_frame + first as u64;
            if self.owes_report {
                out.push(Item::Seeked(Some(self.position(at))));
            } else if at > 0 {
                out.push(Item::Skipped(self.position(at)));
            }
            self.is_dropping = false;
            self.owes_report = false;
            first
        } else {
            if !self.held.is_empty() {
                out.push(Item::Samples(std::mem::take(&mut self.held)));
            }
            0
        };
        self.held
            .extend_from_slice(&samples[end * channels..frames * channels]);
        out.push(Item::Samples(if start == 0 && end == frames {
            samples
        } else {
            samples[start * channels..end * channels].to_vec()
        }));
    }

    pub(super) fn end(&mut self, out: &mut Vec<Item>) {
        if self.owes_report {
            self.owes_report = false;
            out.push(Item::Seeked(Some(self.position(self.frames))));
        }
        if !self.is_on.load(Ordering::Relaxed) && !self.held.is_empty() {
            out.push(Item::Samples(std::mem::take(&mut self.held)));
        }
        self.held = Vec::new();
        out.push(Item::End);
    }
}

/// The frames from the first with sound up to and past the last, if any has.
fn sound_within(samples: &[f32], channels: usize) -> Option<(usize, usize)> {
    let is_sound = |sample: &f32| sample.abs() > SILENCE_CEILING;
    let first = samples.iter().position(is_sound)?;
    let last = samples.iter().rposition(is_sound)?;
    Some((first / channels, last / channels + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 1_000;

    fn gate(is_on: bool) -> SilenceGate {
        let spec = Spec {
            rate: RATE,
            channels: 2,
        };
        SilenceGate::new(spec, Arc::new(AtomicBool::new(is_on)))
    }

    fn silence(frames: usize) -> Vec<f32> {
        vec![0.0; frames * 2]
    }

    fn sound(frames: usize) -> Vec<f32> {
        vec![0.5; frames * 2]
    }

    /// What comes out as text: `s4` for four samples, `@1500` for a seek and
    /// `~1500` for silence left out.
    fn shown(items: &[Item]) -> Vec<String> {
        items
            .iter()
            .map(|item| match item {
                Item::Samples(samples) => format!("s{}", samples.len()),
                Item::Seeked(Some(at)) => format!("@{}", at.as_millis()),
                Item::Seeked(None) => "past".to_owned(),
                Item::Skipped(at) => format!("~{}", at.as_millis()),
                Item::End => "end".to_owned(),
                Item::Failed(_) => "failed".to_owned(),
            })
            .collect()
    }

    #[test]
    fn the_start_and_end_of_a_track_go_and_the_position_tells() {
        let mut gate = gate(true);
        let mut out = Vec::new();
        gate.pass(silence(500), &mut out);
        assert!(out.is_empty());
        let mut block = silence(100);
        block.extend(sound(100));
        block.extend(silence(50));
        gate.pass(block, &mut out);
        gate.pass(silence(500), &mut out);
        gate.end(&mut out);
        assert_eq!(shown(&out), ["~600", "s200", "end"]);
    }

    #[test]
    fn a_track_starting_with_sound_is_not_told_as_a_jump() {
        let mut gate = gate(true);
        let mut out = Vec::new();
        gate.pass(sound(10), &mut out);
        gate.end(&mut out);
        assert_eq!(shown(&out), ["s20", "end"]);
    }

    #[test]
    fn a_pause_in_the_music_stays() {
        let mut gate = gate(true);
        let mut out = Vec::new();
        gate.pass(sound(10), &mut out);
        gate.pass(silence(2_000), &mut out);
        assert_eq!(shown(&out), ["s20"]);
        gate.pass(sound(10), &mut out);
        gate.end(&mut out);
        assert_eq!(shown(&out), ["s20", "s4000", "s20", "end"]);
    }

    #[test]
    fn a_silence_too_long_for_a_pause_goes_too() {
        let mut gate = gate(true);
        let mut out = Vec::new();
        gate.pass(sound(1_000), &mut out);
        for _ in 0..11 {
            gate.pass(silence(1_000), &mut out);
        }
        gate.pass(sound(1), &mut out);
        gate.end(&mut out);
        assert_eq!(shown(&out), ["s2000", "~12000", "s2", "end"]);
    }

    #[test]
    fn a_silent_track_is_only_its_end() {
        let mut gate = gate(true);
        let mut out = Vec::new();
        gate.pass(silence(100), &mut out);
        gate.end(&mut out);
        assert_eq!(shown(&out), ["end"]);
    }

    #[test]
    fn a_seek_is_told_where_the_sound_goes_on() {
        let mut gate = gate(true);
        let mut out = Vec::new();
        gate.pass(sound(1), &mut out);
        out.clear();
        gate.seeked(Some(Duration::from_secs(5)), &mut out);
        assert!(out.is_empty());
        gate.pass(sound(1), &mut out);
        gate.seeked(Some(Duration::from_secs(5)), &mut out);
        let mut block = silence(250);
        block.extend(sound(1));
        gate.pass(block, &mut out);
        assert_eq!(shown(&out), ["@5000", "s2", "@5250", "s2"]);
    }

    #[test]
    fn a_seek_into_the_last_silence_is_told_before_the_end() {
        let mut gate = gate(true);
        let mut out = Vec::new();
        gate.seeked(Some(Duration::from_secs(5)), &mut out);
        gate.pass(silence(100), &mut out);
        gate.end(&mut out);
        assert_eq!(shown(&out), ["@5100", "end"]);
    }

    #[test]
    fn a_seek_past_the_end_is_told_at_once() {
        let mut gate = gate(true);
        let mut out = Vec::new();
        gate.seeked(None, &mut out);
        assert_eq!(shown(&out), ["past"]);
    }

    #[test]
    fn off_everything_goes_through() {
        let mut gate = gate(false);
        let mut out = Vec::new();
        gate.pass(silence(100), &mut out);
        gate.seeked(Some(Duration::from_secs(5)), &mut out);
        gate.pass(silence(100), &mut out);
        gate.end(&mut out);
        assert_eq!(shown(&out), ["s200", "@5000", "s200", "end"]);
    }

    #[test]
    fn turning_it_off_lets_out_what_it_held() {
        let mut gate = gate(true);
        let mut out = Vec::new();
        gate.pass(sound(1), &mut out);
        gate.pass(silence(100), &mut out);
        gate.is_on.store(false, Ordering::Relaxed);
        gate.end(&mut out);
        assert_eq!(shown(&out), ["s2", "s200", "end"]);
    }

    #[test]
    fn a_single_channel_with_sound_keeps_its_frame() {
        let mut gate = gate(true);
        let mut out = Vec::new();
        let mut block = silence(10);
        block[7] = 0.5;
        gate.pass(block, &mut out);
        assert_eq!(shown(&out), ["~3", "s2"]);
    }
}
