//! Decoding ahead of the device, on a thread of each track's own: reading a
//! track may wait on the network, and nothing else should wait with it.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use crate::decode::Decoder;
use crate::output::Spec;
use crate::{Error, Track};

/// What decoding has come to, in order.
pub(crate) enum Item {
    /// Interleaved, at the track's [`Feed::spec`].
    Samples(Vec<f32>),
    /// Where decoding went on after a [`Feed::seek`], or `None` past the end.
    Seeked(Option<Duration>),
    End,
    Failed(Error),
}

#[derive(Clone, Copy)]
struct Opened {
    spec: Spec,
    duration: Option<Duration>,
}

struct State {
    /// Taken by [`Opening::poll`].
    opened: Option<Result<Opened, Error>>,
    /// Whether opening has ended, whichever way.
    open_ended: bool,
    items: VecDeque<Item>,
    /// The samples among `items`.
    samples: usize,
    /// How much audio to decode ahead.
    ahead: Duration,
    /// A seek the thread has yet to start. Anything it finishes meanwhile is
    /// out of date.
    seek: Option<Duration>,
    /// Nothing more comes without a seek.
    ended: bool,
    closed: bool,
}

struct Channel {
    state: Mutex<State>,
    /// Wakes the thread. The engine thread polls, and never waits on it.
    changed: Condvar,
}

/// What the thread does next.
enum Work {
    Seek(Duration),
    Decode,
}

impl Channel {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn finish_opening(&self, opened: Result<Opened, Error>) {
        let mut state = self.lock();
        state.open_ended = true;
        state.opened = Some(opened);
    }

    /// Waits for something to do; `None` once the feed has been dropped.
    fn next_work(&self, spec: Spec) -> Option<Work> {
        let mut state = self.lock();
        loop {
            if state.closed {
                return None;
            }
            if let Some(to) = state.seek.take() {
                return Some(Work::Seek(to));
            }
            if !state.ended && state.samples < samples_in(state.ahead, spec) {
                return Some(Work::Decode);
            }
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn push(&self, item: Item) {
        let mut state = self.lock();
        if state.seek.is_some() {
            return;
        }
        match &item {
            Item::Samples(samples) => state.samples += samples.len(),
            Item::Seeked(Some(_)) => {}
            Item::Seeked(None) | Item::End | Item::Failed(_) => state.ended = true,
        }
        state.items.push_back(item);
    }

    fn seek(&self, to: Duration) {
        let mut state = self.lock();
        state.seek = Some(to);
        state.items.clear();
        state.samples = 0;
        state.ended = false;
        drop(state);
        self.changed.notify_all();
    }
}

fn samples_in(time: Duration, spec: Spec) -> usize {
    (time.as_secs_f64() * f64::from(spec.rate) * f64::from(spec.channels)) as usize
}

/// Ends the thread, once any read it is blocked on returns. Whoever drops it
/// runs the track's cancel hook first if that read may wait on the network.
struct Link(Arc<Channel>);

impl Drop for Link {
    fn drop(&mut self) {
        self.0.lock().closed = true;
        self.0.changed.notify_all();
    }
}

/// Reports a panicking decoder as the track's failure, as the engine would
/// otherwise wait for it forever.
struct PanicReport(Arc<Channel>);

impl Drop for PanicReport {
    fn drop(&mut self) {
        if !thread::panicking() {
            return;
        }
        let error = Error::Decode("decoding stopped unexpectedly".to_owned());
        let mut state = self.0.lock();
        if !state.open_ended {
            state.open_ended = true;
            state.opened = Some(Err(error));
        } else {
            state.items.push_back(Item::Failed(error));
            state.ended = true;
        }
    }
}

fn run(track: Track, channel: &Channel) {
    let mut decoder = match Decoder::open(track) {
        Ok(decoder) => decoder,
        Err(error) => {
            channel.finish_opening(Err(error));
            return;
        }
    };
    let spec = decoder.spec();
    channel.finish_opening(Ok(Opened {
        spec,
        duration: decoder.duration(),
    }));
    while let Some(work) = channel.next_work(spec) {
        let item = match work {
            Work::Seek(to) => decoder.seek(to).map(Item::Seeked),
            Work::Decode => {
                let mut samples = Vec::new();
                decoder.decode_into(&mut samples).map(|more| {
                    if more {
                        Item::Samples(samples)
                    } else {
                        Item::End
                    }
                })
            }
        };
        channel.push(item.unwrap_or_else(Item::Failed));
    }
}

/// A track on its way to becoming a [`Feed`].
pub(crate) struct Opening(Link);

pub(crate) enum Polled {
    Waiting(Opening),
    Opened(Feed),
    Failed(Error),
}

impl Opening {
    /// Starts opening `track`, decoding `ahead` of what has been taken once
    /// it has opened.
    pub(crate) fn start(track: Track, ahead: Duration) -> Result<Self, Error> {
        let channel = Arc::new(Channel {
            state: Mutex::new(State {
                opened: None,
                open_ended: false,
                items: VecDeque::new(),
                samples: 0,
                ahead,
                seek: None,
                ended: false,
                closed: false,
            }),
            changed: Condvar::new(),
        });
        thread::Builder::new()
            .name("hiphile-audio-feed".to_owned())
            .spawn({
                let channel = channel.clone();
                move || {
                    let _report = PanicReport(channel.clone());
                    run(track, &channel);
                }
            })
            .map_err(|error| Error::Read(error.to_string()))?;
        Ok(Self(Link(channel)))
    }

    /// Goes on from `to` once the track has opened, reported as a
    /// [`Item::Seeked`] first of all.
    pub(crate) fn seek(&self, to: Duration) {
        self.0.0.seek(to);
    }

    pub(crate) fn poll(self) -> Polled {
        let opened = self.0.0.lock().opened.take();
        match opened {
            None => Polled::Waiting(self),
            Some(Err(error)) => Polled::Failed(error),
            Some(Ok(opened)) => Polled::Opened(Feed {
                link: self.0,
                spec: opened.spec,
                duration: opened.duration,
            }),
        }
    }
}

/// An opened track, decoding ahead of what the engine thread has taken. None
/// of its methods wait for the track's data.
pub(crate) struct Feed {
    link: Link,
    spec: Spec,
    duration: Option<Duration>,
}

impl Feed {
    pub(crate) fn spec(&self) -> Spec {
        self.spec
    }

    pub(crate) fn duration(&self) -> Option<Duration> {
        self.duration
    }

    /// The next thing decoding has come to, if it has come to anything.
    pub(crate) fn take(&self) -> Option<Item> {
        let mut state = self.link.0.lock();
        let item = state.items.pop_front()?;
        if let Item::Samples(samples) = &item {
            state.samples -= samples.len();
        }
        drop(state);
        self.link.0.changed.notify_all();
        Some(item)
    }

    /// Whether `time` of audio is ready to take, or all that is left of the
    /// track, keeping at least that much ready from now on.
    pub(crate) fn has_ahead(&self, time: Duration) -> bool {
        let mut state = self.link.0.lock();
        if state.ended || state.samples >= samples_in(time, self.spec) {
            return true;
        }
        if state.ahead < time {
            state.ahead = time;
            drop(state);
            self.link.0.changed.notify_all();
        }
        false
    }

    /// Goes on from `to`, reported as an [`Item::Seeked`] first of all. What
    /// had not been taken is gone. Only the last of several seeks made before
    /// the thread gets to them is carried out.
    pub(crate) fn seek(&self, to: Duration) {
        self.link.0.seek(to);
    }
}

#[cfg(test)]
mod tests;
