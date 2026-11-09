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

/// The lyrics of the song the player is on, shared by all that show them so that a song is
/// looked up once. It looks only while some [`Wanted`] is held, and only once for each song: a
/// failed lookup is tried again only by [`retry`](Self::retry).
#[derive(Clone)]
pub struct CurrentLyrics(Arc<Inner>);

/// Held while the lyrics show, so that the song's lyrics are looked up. Dropping it lets go.
#[must_use]
pub struct Wanted(WeakLyrics);

impl Drop for Wanted {
    fn drop(&mut self) {
        if let Some(lyrics) = self.0.upgrade() {
            lyrics.0.state.update(|state| state.wanted -= 1);
        }
    }
}

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
    song: Option<Song>,
    /// How many [`Wanted`] are held.
    wanted: usize,
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
                song: None,
                wanted: 0,
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

    /// The song the player is on, whose lyrics are looked up while they are wanted.
    pub fn set_song(&self, song: Option<&Song>) {
        let asked = self.0.state.update(|state| {
            if state.song.as_ref().map(Song::id) != song.map(Song::id) {
                state.song = song.cloned();
            }
            state.look_up(false)
        });
        self.ask(asked);
    }

    /// Looks up the song's lyrics unless they were already, and keeps looking up those of each
    /// song after it until every [`Wanted`] is dropped.
    pub fn want(&self) -> Wanted {
        let asked = self.0.state.update(|state| {
            state.wanted += 1;
            state.look_up(false)
        });
        self.ask(asked);
        Wanted(self.downgrade())
    }

    /// Looks again for lyrics whose lookup failed.
    pub fn retry(&self) {
        let asked = self.0.state.update(|state| state.look_up(true));
        self.ask(asked);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn song(&self) -> Option<SongId> {
        self.0
            .state
            .read(|state| state.song.as_ref().map(|song| song.id().clone()))
    }

    fn ask(&self, asked: Option<(SongId, TrackQuery)>) {
        let Some((id, query)) = asked else {
            return;
        };
        let found = (self.0.find_lyrics)(query);
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
    /// Shows the song's lyrics while they are wanted, and gives what is to be asked for them.
    fn look_up(&mut self, is_retry: bool) -> Option<(SongId, TrackQuery)> {
        if self.wanted == 0 {
            return None;
        }
        let Some(song) = &self.song else {
            self.shown.lookup = Lookup::Nothing;
            return None;
        };
        let id = song.id();
        let is_failed = matches!(self.shown.lookup, Lookup::Failed(..));
        if self.shown.lookup.song() == Some(id) && !(is_retry && is_failed) {
            return None;
        }
        if let Some((_, lyrics)) = self.found.iter().find(|(found, _)| found == id) {
            self.shown.lookup = Lookup::Done(id.clone(), lyrics.clone());
            return None;
        }
        self.shown.lookup = Lookup::Loading(id.clone());
        Some((id.clone(), query_for(song)))
    }

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
