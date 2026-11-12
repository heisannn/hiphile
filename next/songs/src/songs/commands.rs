use std::cell::Cell;
use std::sync::Arc;

use anti_webui::prelude::{NamedKey, untrack};
use anti_webui_md3e::components::MenuItem;
use hiphile_core::navigation::Location;
use next_ui::favorite_songs::{are_favorites, favorite_songs_item, toggle_favorite_songs};
use next_ui::keymap::{KeyBinding, KeyContext, Keystroke, bind_keys, keystroke_for};
use next_ui::menus::{
    Songs, add_to_playlist_item, go_to_album_item, go_to_artist_item, queue_items,
};
use next_ui::{AlbumAction, Command};
use playback::Placement;
use subsonic::{Song, SongId};

use super::table::Table;

const CONTEXT: &str = "Songs";

/// What the keys of the rows do to the songs selected, besides Enter, which plays them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SongCommand {
    PlayNext,
    AddToQueue,
    AddToPlaylist,
    ToggleFavorite,
    GoToAlbum,
    GoToArtist,
}

/// Binds the keys of the rows on this thread, the first time it is called there.
pub(super) fn bind_song_keys() {
    thread_local! {
        static BOUND: Cell<bool> = const { Cell::new(false) };
    }
    if BOUND.with(|bound| bound.replace(true)) {
        return;
    }
    let enter = || Keystroke::named(NamedKey::Enter);
    bind_keys([
        KeyBinding::new(enter().with_alt(), SongCommand::PlayNext, CONTEXT),
        KeyBinding::new(enter().with_shift(), SongCommand::AddToQueue, CONTEXT),
        KeyBinding::new(Keystroke::primary('p'), SongCommand::AddToPlaylist, CONTEXT),
        KeyBinding::new(
            Keystroke::primary('d'),
            SongCommand::ToggleFavorite,
            CONTEXT,
        ),
        KeyBinding::new(Keystroke::primary('l'), SongCommand::GoToAlbum, CONTEXT),
        KeyBinding::new(
            Keystroke::primary('a').with_shift(),
            SongCommand::GoToArtist,
            CONTEXT,
        ),
    ]);
}

/// What a menu shows for the key of `command`.
fn shortcut(command: SongCommand) -> String {
    keystroke_for(&command, CONTEXT).map_or_else(String::new, |stroke| stroke.to_string())
}

impl Table {
    /// The selected songs on show, in their order.
    pub(super) fn selected_songs(&self) -> Vec<Song> {
        let selection = self.selection;
        untrack(|| {
            self.shown.with(|shown| {
                let Some(shown) = shown else {
                    return Vec::new();
                };
                selection.with(|selected| {
                    (0..shown.visible().len())
                        .filter_map(|position| shown.get(position))
                        .filter(|song| selected.contains(song.id()))
                        .cloned()
                        .collect()
                })
            })
        })
    }

    /// What a menu opened on the row of `song` acts on: the selection when the row is in it, and
    /// the song alone otherwise.
    pub(super) fn songs_for(&self, song: &SongId) -> Vec<Song> {
        if untrack(|| self.selection.contains(song)) {
            return self.selected_songs();
        }
        untrack(|| {
            self.shown.with(|shown| {
                let shown = shown.as_ref()?;
                let position = shown.position(|shown| shown.id() == song)?;
                shown.get(position).cloned()
            })
        })
        .into_iter()
        .collect()
    }

    /// What Play does to `songs`: a song alone plays with those after it on show, as from a click
    /// in an album, and songs chosen together play by themselves.
    fn play(&self, songs: &[Song]) -> Option<Command> {
        match songs {
            [] => None,
            [song] => untrack(|| {
                self.shown.with(|shown| {
                    let shown = shown.as_ref()?;
                    let position = shown.position(|shown| shown.id() == song.id())?;
                    let (songs, start) = shown.queue_from(position)?;
                    Some(Command::Play {
                        songs,
                        start: Some(start),
                    })
                })
            }),
            songs => AlbumAction::Play.command(songs.into()),
        }
    }

    /// Enter on the rows, and a double click on one: plays what the row of `song` stands for.
    pub(super) fn open(&self, song: &SongId) {
        if let Some(play) = self.play(&self.songs_for(song)) {
            self.cx.commands.send(play);
        }
    }

    /// The menu of `songs`, made as it opens.
    pub(super) fn menu_groups(&self, songs: Vec<Song>) -> Vec<Vec<MenuItem>> {
        let (commands, known) = (&self.cx.commands, self.cx.known);
        let play = self.play(&songs);
        let songs: Arc<[Song]> = songs.into();
        let list = Songs::List(songs.clone());
        let keys = [
            Keystroke::named(NamedKey::Enter).to_string(),
            shortcut(SongCommand::PlayNext),
            shortcut(SongCommand::AddToQueue),
        ];
        let queue = queue_items(commands, known, &list, play)
            .into_iter()
            .zip(keys)
            .map(|(item, key)| item.trailing(key))
            .collect();
        let mut others = vec![
            add_to_playlist_item(commands, known, &list),
            favorite_songs_item(commands, known, songs.clone())
                .trailing(shortcut(SongCommand::ToggleFavorite)),
        ];
        if let [song] = &*songs {
            others.extend(
                go_to_album_item(commands, song.album_id())
                    .map(|item| item.trailing(shortcut(SongCommand::GoToAlbum))),
            );
            others.extend(
                go_to_artist_item(commands, song.artist_id())
                    .map(|item| item.trailing(shortcut(SongCommand::GoToArtist))),
            );
        }
        vec![queue, others]
    }

    /// Where the keys of the rows work, on the songs selected.
    pub(super) fn key_context(&self) -> KeyContext<SongCommand> {
        let (enabled, run) = (self.clone(), self.clone());
        KeyContext::new(
            CONTEXT,
            move |command| enabled.is_enabled(*command),
            move |command| run.run(command),
        )
    }

    fn is_enabled(&self, command: SongCommand) -> bool {
        let songs = self.selected_songs();
        let known = self.cx.known;
        match command {
            SongCommand::PlayNext | SongCommand::AddToQueue => {
                !songs.iter().all(|song| known.is_skipped(song))
            }
            SongCommand::AddToPlaylist => !songs.is_empty(),
            SongCommand::ToggleFavorite => are_favorites(known, &songs).is_some(),
            SongCommand::GoToAlbum => matches!(&*songs, [song] if song.album_id().is_some()),
            SongCommand::GoToArtist => matches!(&*songs, [song] if song.artist_id().is_some()),
        }
    }

    fn run(&self, command: SongCommand) {
        let songs = self.selected_songs();
        let commands = &self.cx.commands;
        let enqueue = |placement| AlbumAction::Enqueue(placement).command(songs.clone().into());
        let sent = match command {
            SongCommand::PlayNext => enqueue(Placement::Next),
            SongCommand::AddToQueue => enqueue(Placement::Last),
            SongCommand::AddToPlaylist => AlbumAction::ChoosePlaylist.command(songs.clone().into()),
            SongCommand::ToggleFavorite => {
                toggle_favorite_songs(commands, self.cx.known, &songs);
                None
            }
            SongCommand::GoToAlbum => songs
                .first()
                .and_then(Song::album_id)
                .map(|album| Command::Show(Location::Albums(Some(album.clone())))),
            SongCommand::GoToArtist => songs
                .first()
                .and_then(Song::artist_id)
                .map(|artist| Command::Show(Location::Artists(Some(artist.clone())))),
        };
        if let Some(command) = sent {
            commands.send(command);
        }
    }
}
