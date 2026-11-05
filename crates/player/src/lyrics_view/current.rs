use std::sync::Arc;

use config::LyricsHighlight;
use futures::FutureExt as _;
use futures::future::BoxFuture;
use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, SharedString};
use lyrics::{Finder, Lyrics, TrackQuery};
use subsonic::{Song, SongId};

use crate::player::Player;

/// How many songs' lyrics stay in memory, for going back and forth in a queue.
const FOUND_KEPT: usize = 64;

pub(crate) type FindLyrics =
    Arc<dyn Fn(TrackQuery) -> BoxFuture<'static, Result<Lyrics, lyrics::Error>> + Send + Sync>;

pub(super) enum Lookup {
    Nothing,
    Loading(SongId),
    Done(SongId, Arc<Lyrics>),
    Failed(SongId, SharedString),
}

impl Lookup {
    fn song(&self) -> Option<&SongId> {
        match self {
            Self::Nothing => None,
            Self::Loading(song) | Self::Done(song, _) | Self::Failed(song, _) => Some(song),
        }
    }
}

/// Another set of lines takes the place of those on show.
pub(super) struct NewLines;

/// The lyrics of the song `player` is on, shared by all that show them so that a
/// song is looked up once. It looks only when one of them asks.
pub struct CurrentLyrics {
    player: Entity<Player>,
    find_lyrics: FindLyrics,
    pub(super) lookup: Lookup,
    /// Songs whose lyrics were found, or found not to exist, oldest first.
    pub(super) found: Vec<(SongId, Arc<Lyrics>)>,
    highlight: LyricsHighlight,
}

impl EventEmitter<NewLines> for CurrentLyrics {}

impl CurrentLyrics {
    pub fn new(player: Entity<Player>, finder: Finder) -> Self {
        let find_lyrics = Arc::new(move |query: TrackQuery| {
            let finder = finder.clone();
            async move { finder.find_lyrics(&query).await }.boxed()
        });
        Self::with_find_lyrics(player, find_lyrics)
    }

    pub(crate) fn with_find_lyrics(player: Entity<Player>, find_lyrics: FindLyrics) -> Self {
        Self {
            player,
            find_lyrics,
            lookup: Lookup::Nothing,
            found: Vec::new(),
            highlight: LyricsHighlight::default(),
        }
    }

    pub(super) fn highlight(&self) -> LyricsHighlight {
        self.highlight
    }

    pub fn set_highlight(&mut self, highlight: LyricsHighlight, cx: &mut Context<Self>) {
        self.highlight = highlight;
        cx.notify();
    }

    pub(super) fn is_current_song_looked_up(&self, cx: &App) -> bool {
        self.lookup.song() == self.player.read(cx).current_song().map(Song::id)
    }

    /// Looks for the lyrics of the current song, unless they are on show or on
    /// their way. A failed search is tried again.
    pub(crate) fn look_up_current(&mut self, cx: &mut Context<Self>) {
        let Some(song) = self.player.read(cx).current_song().cloned() else {
            self.lookup = Lookup::Nothing;
            cx.notify();
            return;
        };
        let is_failed = matches!(self.lookup, Lookup::Failed(..));
        if self.lookup.song() == Some(song.id()) && !is_failed {
            return;
        }
        cx.emit(NewLines);
        let id = song.id().clone();
        if let Some((_, lyrics)) = self.found.iter().find(|(found, _)| *found == id) {
            self.lookup = Lookup::Done(id, lyrics.clone());
            cx.notify();
            return;
        }
        self.lookup = Lookup::Loading(id.clone());
        let found = cx.background_spawn((self.find_lyrics)(query_for(&song)));
        // It goes on when the song changes: the token it may be waiting for is
        // what the next song needs, and asking for another could be refused.
        cx.spawn(async move |this, cx| {
            let found = found.await;
            this.update(cx, |this, cx| this.finish_lookup(id, found, cx))
                .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn finish_lookup(
        &mut self,
        song: SongId,
        found: Result<Lyrics, lyrics::Error>,
        cx: &mut Context<Self>,
    ) {
        let is_awaited = matches!(&self.lookup, Lookup::Loading(awaited) if *awaited == song);
        let lookup = match found {
            Ok(lyrics) => {
                let lyrics = self.keep_found(&song, lyrics);
                Lookup::Done(song, lyrics)
            }
            Err(error) => Lookup::Failed(song, describe(&error).into()),
        };
        if is_awaited {
            self.lookup = lookup;
            cx.emit(NewLines);
            cx.notify();
        }
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

pub(super) fn query_for(song: &Song) -> TrackQuery {
    TrackQuery::new(song.title())
        .artist(song.artist())
        .album_artist(song.album_artist())
        .album(song.album())
        // A server that does not know a song's length may say zero
        .duration(song.duration().filter(|duration| !duration.is_zero()))
}

pub(super) fn describe(error: &lyrics::Error) -> String {
    match error {
        lyrics::Error::Network(_) => {
            "Couldn't load lyrics. Check your connection and try again.".to_owned()
        }
        lyrics::Error::HttpStatus(_)
        | lyrics::Error::Api { .. }
        | lyrics::Error::InvalidResponse
        | lyrics::Error::TokenUnavailable => {
            "Couldn't load lyrics. The lyrics services aren't answering as they should, so try again later."
                .to_owned()
        }
    }
}
