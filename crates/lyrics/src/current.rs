use std::sync::{Arc, Weak};

use config::LyricsHighlight;
use executor::{Executor, Publish, Published};
use futures::FutureExt as _;
use futures::future::BoxFuture;
use subsonic::{Song, SongId};
use tokio::sync::watch;

use crate::{Error, Finder, Lyrics, TrackQuery};

/// How many songs' lyrics stay in memory, for going back and forth in a queue.
const FOUND_KEPT: usize = 64;

pub type FindLyrics =
    Arc<dyn Fn(TrackQuery) -> BoxFuture<'static, Result<Lyrics, Error>> + Send + Sync>;

#[derive(Clone, Debug)]
pub enum Lookup {
    Nothing,
    Loading(SongId),
    Done(SongId, Arc<Lyrics>),
    Failed(SongId, Arc<str>),
}

impl PartialEq for Lookup {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Nothing, Self::Nothing) => true,
            (Self::Loading(a), Self::Loading(b)) => a == b,
            (Self::Done(a, found), Self::Done(b, other)) => a == b && Arc::ptr_eq(found, other),
            (Self::Failed(a, reason), Self::Failed(b, other)) => a == b && reason == other,
            _ => false,
        }
    }
}

impl Lookup {
    pub fn song(&self) -> Option<&SongId> {
        match self {
            Self::Nothing => None,
            Self::Loading(song) | Self::Done(song, _) | Self::Failed(song, _) => Some(song),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LyricsSnapshot {
    lookup: Lookup,
    highlight: LyricsHighlight,
}

impl LyricsSnapshot {
    pub fn lookup(&self) -> &Lookup {
        &self.lookup
    }

    pub fn highlight(&self) -> LyricsHighlight {
        self.highlight
    }
}

/// The lyrics of the song on show, shared by all that show them so that a song is looked up
/// once. It looks only when asked.
#[derive(Clone)]
pub struct CurrentLyrics(Arc<Inner>);

struct WeakLyrics(Weak<Inner>);

struct Inner {
    executor: Executor,
    find_lyrics: FindLyrics,
    state: Published<State>,
}

struct State {
    shown: LyricsSnapshot,
    /// Songs whose lyrics were found, or found not to exist, oldest first.
    found: Vec<(SongId, Arc<Lyrics>)>,
}

impl Publish for State {
    type Snapshot = LyricsSnapshot;

    fn snapshot(&self) -> &LyricsSnapshot {
        &self.shown
    }
}

impl CurrentLyrics {
    pub fn new(executor: Executor, finder: Finder) -> Self {
        let find_lyrics = Arc::new(move |query: TrackQuery| {
            let finder = finder.clone();
            async move { finder.find_lyrics(&query).await }.boxed()
        });
        Self::with_find_lyrics(executor, find_lyrics)
    }

    pub fn with_find_lyrics(executor: Executor, find_lyrics: FindLyrics) -> Self {
        Self(Arc::new(Inner {
            executor,
            find_lyrics,
            state: Published::new(State {
                shown: LyricsSnapshot {
                    lookup: Lookup::Nothing,
                    highlight: LyricsHighlight::default(),
                },
                found: Vec::new(),
            }),
        }))
    }

    pub fn snapshot(&self) -> LyricsSnapshot {
        self.0.state.snapshot()
    }

    /// Sees each new snapshot. A borrow of it must not be held while calling these lyrics, which
    /// publish under their own lock.
    pub fn subscribe(&self) -> watch::Receiver<LyricsSnapshot> {
        self.0.state.subscribe()
    }

    pub fn set_highlight(&self, highlight: LyricsHighlight) {
        self.0
            .state
            .update(|state| state.shown.highlight = highlight);
    }

    /// Looks for the lyrics of `song`, unless they are on show or on their way. A failed search
    /// is tried again. Without a song, shows nothing.
    pub fn look_up(&self, song: Option<&Song>) {
        let Some(song) = song else {
            self.0
                .state
                .update(|state| state.shown.lookup = Lookup::Nothing);
            return;
        };
        let id = song.id().clone();
        let is_asked = self.0.state.update(|state| {
            let is_failed = matches!(state.shown.lookup, Lookup::Failed(..));
            if state.shown.lookup.song() == Some(&id) && !is_failed {
                return false;
            }
            if let Some((_, lyrics)) = state.found.iter().find(|(found, _)| *found == id) {
                state.shown.lookup = Lookup::Done(id.clone(), lyrics.clone());
                return false;
            }
            state.shown.lookup = Lookup::Loading(id.clone());
            true
        });
        if !is_asked {
            return;
        }
        let found = (self.0.find_lyrics)(query_for(song));
        let weak = self.downgrade();
        // It goes on when the song changes: the token it may be waiting for is what the next
        // song needs, and asking for another could be refused.
        self.0
            .executor
            .spawn(async move {
                let found = found.await;
                if let Some(this) = weak.upgrade() {
                    this.finish_lookup(id, found);
                }
            })
            .detach();
    }

    fn finish_lookup(&self, song: SongId, found: Result<Lyrics, Error>) {
        self.0.state.update(|state| {
            let is_awaited =
                matches!(&state.shown.lookup, Lookup::Loading(awaited) if *awaited == song);
            let lookup = match found {
                Ok(lyrics) => {
                    let lyrics = state.keep_found(&song, lyrics);
                    Lookup::Done(song, lyrics)
                }
                Err(error) => Lookup::Failed(song, describe(&error).into()),
            };
            if is_awaited {
                state.shown.lookup = lookup;
            }
        });
    }

    fn downgrade(&self) -> WeakLyrics {
        WeakLyrics(Arc::downgrade(&self.0))
    }
}

impl WeakLyrics {
    fn upgrade(&self) -> Option<CurrentLyrics> {
        self.0.upgrade().map(CurrentLyrics)
    }
}

impl State {
    fn keep_found(&mut self, song: &SongId, lyrics: Lyrics) -> Arc<Lyrics> {
        if let Some((_, kept)) = self.found.iter().find(|(found, _)| found == song) {
            return kept.clone();
        }
        let lyrics = Arc::new(lyrics);
        if self.found.len() >= FOUND_KEPT {
            self.found.remove(0);
        }
        self.found.push((song.clone(), lyrics.clone()));
        lyrics
    }
}

fn query_for(song: &Song) -> TrackQuery {
    TrackQuery::new(song.title())
        .artist(song.artist())
        .album_artist(song.album_artist())
        .album(song.album())
        // A server that does not know a song's length may say zero
        .duration(song.duration().filter(|duration| !duration.is_zero()))
}

fn describe(error: &Error) -> String {
    match error {
        Error::Network(_) => {
            "Couldn't load lyrics. Check your connection and try again.".to_owned()
        }
        Error::HttpStatus(_) | Error::Api { .. } | Error::InvalidResponse | Error::TokenUnavailable => {
            "Couldn't load lyrics. The lyrics services aren't answering as they should, so try again later."
                .to_owned()
        }
    }
}

#[cfg(test)]
mod tests;
