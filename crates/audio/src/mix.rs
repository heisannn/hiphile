/// Two tracks crossing over one stream's length of `frames`: one falling to
/// silence and one rising from it, each already in the same rate and channels.
///
/// The gains sum to one, so two tracks at full level cannot clip. They sum to
/// less than equal power does at the middle, which would have to be limited.
pub(crate) struct Blend {
    channels: usize,
    frames: u64,
    done: u64,
    outgoing: Vec<f32>,
    incoming: Vec<f32>,
    outgoing_ended: bool,
}

impl Blend {
    pub(crate) fn new(channels: usize, frames: u64) -> Self {
        Self {
            channels,
            frames,
            done: 0,
            outgoing: Vec::new(),
            incoming: Vec::new(),
            outgoing_ended: false,
        }
    }

    pub(crate) fn outgoing_mut(&mut self) -> &mut Vec<f32> {
        &mut self.outgoing
    }

    pub(crate) fn incoming_mut(&mut self) -> &mut Vec<f32> {
        &mut self.incoming
    }

    /// From here on the outgoing track counts as silence.
    pub(crate) fn end_outgoing(&mut self) {
        self.outgoing_ended = true;
    }

    pub(crate) fn wants_outgoing(&self) -> bool {
        let queued = (self.outgoing.len() / self.channels) as u64;
        !self.outgoing_ended
            && self.done + queued < self.frames
            && self.outgoing.len() <= self.incoming.len()
    }

    pub(crate) fn is_done(&self) -> bool {
        self.done >= self.frames
    }

    /// Appends all it can mix so far, and past the end of the crossing, the
    /// incoming track as it is. Whatever is left of the outgoing track then
    /// goes unheard.
    pub(crate) fn mix_into(&mut self, out: &mut Vec<f32>) {
        let left = usize::try_from(self.frames - self.done)
            .unwrap_or(usize::MAX)
            .saturating_mul(self.channels);
        let ready = if self.outgoing_ended {
            self.incoming.len()
        } else {
            self.incoming.len().min(self.outgoing.len())
        };
        let mixing = ready.min(left);
        out.reserve(mixing);
        let frames = self.incoming[..mixing].chunks_exact(self.channels);
        for (ix, frame) in frames.enumerate() {
            let rising = rising_gain(self.done + ix as u64, self.frames);
            let falling = 1.0 - rising;
            for (channel, sample) in frame.iter().enumerate() {
                let fading = self
                    .outgoing
                    .get(ix * self.channels + channel)
                    .copied()
                    .unwrap_or(0.0);
                out.push(sample * rising + fading * falling);
            }
        }
        self.done += (mixing / self.channels) as u64;
        self.outgoing.drain(..mixing.min(self.outgoing.len()));
        self.incoming.drain(..mixing);
        if self.is_done() {
            self.outgoing.clear();
            out.append(&mut self.incoming);
        }
    }

    /// Ends the crossing now, appending what is left of the incoming track.
    pub(crate) fn cut_into(&mut self, out: &mut Vec<f32>) {
        self.done = self.frames;
        self.outgoing.clear();
        out.append(&mut self.incoming);
    }
}

/// A smoothstep, as it leaves and arrives without a corner that could be heard.
pub(crate) fn rising_gain(frame: u64, frames: u64) -> f32 {
    let at = (frame as f64 / frames as f64) as f32;
    at * at * (3.0 - 2.0 * at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mixed(blend: &mut Blend) -> Vec<f32> {
        let mut out = Vec::new();
        blend.mix_into(&mut out);
        out
    }

    #[test]
    fn it_falls_from_one_track_to_the_other_at_constant_sum() {
        let mut blend = Blend::new(2, 100);
        blend.outgoing_mut().extend([1.0; 200]);
        blend.incoming_mut().extend([1.0; 200]);
        let out = mixed(&mut blend);
        assert_eq!(out.len(), 200);
        assert!(out.iter().all(|sample| (*sample - 1.0).abs() < 1e-6));
        assert!(blend.is_done());

        let mut blend = Blend::new(1, 100);
        blend.outgoing_mut().extend([1.0; 100]);
        blend.incoming_mut().extend([0.0; 100]);
        let out = mixed(&mut blend);
        assert_eq!(out[0], 1.0);
        assert!(out.windows(2).all(|pair| pair[1] <= pair[0]));
        assert!(*out.last().unwrap() < 0.001);
    }

    #[test]
    fn it_waits_for_both_tracks() {
        let mut blend = Blend::new(1, 100);
        assert!(blend.wants_outgoing());
        blend.outgoing_mut().extend([1.0; 10]);
        assert!(!blend.wants_outgoing());
        assert!(mixed(&mut blend).is_empty());
        blend.incoming_mut().extend([0.0; 4]);
        assert_eq!(mixed(&mut blend).len(), 4);
        blend.incoming_mut().extend([0.0; 6]);
        assert_eq!(mixed(&mut blend).len(), 6);
        assert!(blend.wants_outgoing());
    }

    #[test]
    fn the_incoming_track_goes_on_as_it_is_past_the_crossing() {
        let mut blend = Blend::new(1, 2);
        blend.outgoing_mut().extend([1.0, 1.0, 1.0]);
        blend.incoming_mut().extend([0.5, 0.5, 0.25, 0.125]);
        let out = mixed(&mut blend);
        assert_eq!(out[2..], [0.25, 0.125]);
        assert!(blend.is_done());
        assert!(!blend.wants_outgoing());
    }

    #[test]
    fn an_outgoing_track_that_ends_early_counts_as_silence() {
        let mut blend = Blend::new(1, 100);
        blend.outgoing_mut().extend([1.0; 10]);
        blend.end_outgoing();
        assert!(!blend.wants_outgoing());
        blend.incoming_mut().extend([1.0; 50]);
        let out = mixed(&mut blend);
        assert_eq!(out.len(), 50);
        assert_eq!(out[20], rising_gain(20, 100));
    }

    #[test]
    fn it_needs_no_more_of_the_outgoing_track_than_its_length() {
        let mut blend = Blend::new(1, 10);
        blend.outgoing_mut().extend([1.0; 10]);
        blend.incoming_mut().extend([1.0; 10]);
        assert!(!blend.wants_outgoing());
    }

    #[test]
    fn a_cut_hands_over_what_is_left_of_the_incoming_track() {
        let mut blend = Blend::new(1, 100);
        blend.outgoing_mut().extend([1.0; 10]);
        blend.incoming_mut().extend([0.5; 4]);
        let mut out = Vec::new();
        blend.cut_into(&mut out);
        assert_eq!(out, [0.5; 4]);
        assert!(blend.is_done());
    }
}
