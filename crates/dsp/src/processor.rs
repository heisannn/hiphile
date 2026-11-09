use crate::Settings;
use crate::crossfeed::Filter;

const TRANSITION_SECONDS: f64 = 0.02;

/// Applies [`Settings`] to the first two channels of interleaved frames, at
/// one rate. A change of settings is heard as a short transition rather than
/// a step.
#[derive(Clone, Debug)]
pub struct Processor {
    rate: u32,
    channels: usize,
    /// The latest followed, which a crossfade under way may not have reached
    settings: Settings,
    /// The crossfeed heard, or being faded to
    filter: Option<Filter>,
    fade: Option<Fade>,
    gains: Glide,
}

/// From one crossfeed to another along a smoothstep
#[derive(Clone, Debug)]
struct Fade {
    from: Option<Filter>,
    done: u32,
    frames: u32,
}

/// From one pair of gains to another in a straight line, ending on it exactly
#[derive(Clone, Debug)]
struct Glide {
    from: [f64; 2],
    to: [f64; 2],
    done: u32,
    frames: u32,
}

impl Glide {
    fn is_at_rest(&self) -> bool {
        self.done >= self.frames
    }

    fn current(&self) -> [f64; 2] {
        if self.is_at_rest() {
            return self.to;
        }
        let at = f64::from(self.done) / f64::from(self.frames);
        [0, 1].map(|channel| self.from[channel] + (self.to[channel] - self.from[channel]) * at)
    }

    fn restart(&mut self, to: [f64; 2]) {
        self.from = self.current();
        self.to = to;
        self.done = 0;
    }

    fn next(&mut self) -> [f64; 2] {
        if !self.is_at_rest() {
            self.done += 1;
        }
        self.current()
    }
}

impl Processor {
    /// Starts at `settings`, with no transition.
    pub fn new(rate: u32, channels: usize, settings: Settings) -> Self {
        let gains = settings.balance().gains();
        let frames = transition_frames(rate);
        Self {
            rate,
            channels,
            settings,
            filter: settings
                .crossfeed()
                .map(|crossfeed| Filter::new(crossfeed, rate)),
            fade: None,
            gains: Glide {
                from: gains,
                to: gains,
                done: frames,
                frames,
            },
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings
    }

    /// Moves to `settings` over the next 20 ms. Balance glides from wherever it
    /// is. Another crossfeed waits for a crossfade under way to end, and only
    /// the latest one waits.
    pub fn follow(&mut self, settings: Settings) {
        if settings == self.settings {
            return;
        }
        if settings.balance() != self.settings.balance() {
            self.gains.restart(settings.balance().gains());
        }
        self.settings = settings;
        if self.fade.is_none() {
            self.start_fade();
        }
    }

    /// Whole frames of interleaved samples. A sample that is not a finite
    /// number counts as silence, unless the settings are neutral and nothing
    /// is moving, which leaves every sample as it is.
    pub fn process(&mut self, samples: &mut [f32]) {
        if self.channels < 2 || self.is_at_rest_and_neutral() {
            return;
        }
        for frame in samples.chunks_exact_mut(self.channels) {
            let [left, right] = self.next_frame([finite(frame[0]), finite(frame[1])]);
            frame[0] = left as f32;
            frame[1] = right as f32;
        }
        for filter in self
            .filter
            .iter_mut()
            .chain(self.fade.iter_mut().filter_map(|fade| fade.from.as_mut()))
        {
            filter.settle();
        }
    }

    /// The same processor at another rate, going on from where it is.
    pub fn at_rate(mut self, rate: u32) -> Self {
        if rate == self.rate {
            return self;
        }
        let frames = transition_frames(rate);
        let follow = |filter: Filter| filter.succeeded_by(filter.crossfeed(), rate);
        self.filter = self.filter.map(follow);
        if let Some(fade) = &mut self.fade {
            fade.from = fade.from.map(follow);
            fade.done = rescaled(fade.done, fade.frames, frames);
            fade.frames = frames;
        }
        self.gains.done = rescaled(self.gains.done, self.gains.frames, frames);
        self.gains.frames = frames;
        self.rate = rate;
        self
    }

    fn is_at_rest_and_neutral(&self) -> bool {
        self.filter.is_none()
            && self.fade.is_none()
            && self.gains.is_at_rest()
            && self.settings.balance().is_centered()
    }

    fn start_fade(&mut self) {
        let to = self.settings.crossfeed();
        if to == self.filter.as_ref().map(Filter::crossfeed) {
            return;
        }
        let from = self.filter.take();
        self.filter = to.map(|crossfeed| match &from {
            Some(filter) => filter.succeeded_by(crossfeed, self.rate),
            None => Filter::new(crossfeed, self.rate),
        });
        self.fade = Some(Fade {
            from,
            done: 0,
            frames: transition_frames(self.rate),
        });
    }

    fn next_frame(&mut self, input: [f64; 2]) -> [f64; 2] {
        let mut out = self
            .filter
            .as_mut()
            .map_or(input, |filter| filter.run(input));
        if let Some(fade) = &mut self.fade {
            let from = fade.from.as_mut().map_or(input, |filter| filter.run(input));
            fade.done += 1;
            if fade.done < fade.frames {
                let at = smoothstep(f64::from(fade.done) / f64::from(fade.frames));
                out = [0, 1].map(|channel| from[channel] + (out[channel] - from[channel]) * at);
            } else {
                self.fade = None;
                self.start_fade();
            }
        }
        let gains = self.gains.next();
        [out[0] * gains[0], out[1] * gains[1]]
    }
}

fn transition_frames(rate: u32) -> u32 {
    ((f64::from(rate) * TRANSITION_SECONDS).round() as u32).max(1)
}

fn rescaled(done: u32, from: u32, to: u32) -> u32 {
    (u64::from(done) * u64::from(to) / u64::from(from.max(1))) as u32
}

fn smoothstep(at: f64) -> f64 {
    at * at * (3.0 - 2.0 * at)
}

fn finite(sample: f32) -> f64 {
    if sample.is_finite() {
        f64::from(sample)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests;
