//! The screen's keys, besides those of its lists: the list's go to the playlist chosen, the songs'
//! to the song selected.

use anti_webui::prelude::NamedKey;
use next_ui::keymap::{KeyBinding, Keystroke, bind_keys, is_bound};

pub(super) const LIST: &str = "Playlists";
pub(super) const SONGS: &str = "PlaylistSongs";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum KeyCommand {
    /// From the list to the songs.
    OpenSongs,
    Play,
    PlayNext,
    AddToQueue,
    Rename,
    Delete,
    /// From the songs to the list.
    Back,
    MoveUp,
    MoveDown,
    Remove,
    ToggleFavorite,
    GoToAlbum,
    GoToArtist,
    AddToPlaylist,
}

/// Binds the keys, once for the thread, whose keymap it is.
pub(super) fn bind() {
    if is_bound::<KeyCommand>() {
        return;
    }
    let named = Keystroke::named;
    for context in [LIST, SONGS] {
        bind_keys([
            KeyBinding::new(
                named(NamedKey::Enter).with_alt(),
                KeyCommand::PlayNext,
                context,
            ),
            KeyBinding::new(
                named(NamedKey::Enter).with_shift(),
                KeyCommand::AddToQueue,
                context,
            ),
        ]);
    }
    bind_keys([
        KeyBinding::new(named(NamedKey::ArrowRight), KeyCommand::OpenSongs, LIST),
        KeyBinding::new(
            named(NamedKey::Enter).with_primary(),
            KeyCommand::Play,
            LIST,
        ),
        KeyBinding::new(named(NamedKey::F(2)), KeyCommand::Rename, LIST),
        KeyBinding::new(
            named(NamedKey::Backspace).with_primary(),
            KeyCommand::Delete,
            LIST,
        ),
        KeyBinding::new(named(NamedKey::ArrowLeft), KeyCommand::Back, SONGS),
        KeyBinding::new(named(NamedKey::Escape), KeyCommand::Back, SONGS),
        KeyBinding::new(
            named(NamedKey::ArrowUp).with_alt(),
            KeyCommand::MoveUp,
            SONGS,
        ),
        KeyBinding::new(
            named(NamedKey::ArrowDown).with_alt(),
            KeyCommand::MoveDown,
            SONGS,
        ),
        KeyBinding::new(named(NamedKey::Delete), KeyCommand::Remove, SONGS),
        KeyBinding::new(named(NamedKey::Backspace), KeyCommand::Remove, SONGS),
        KeyBinding::new(Keystroke::primary('d'), KeyCommand::ToggleFavorite, SONGS),
        KeyBinding::new(Keystroke::primary('l'), KeyCommand::GoToAlbum, SONGS),
        KeyBinding::new(
            Keystroke::primary('a').with_shift(),
            KeyCommand::GoToArtist,
            SONGS,
        ),
        KeyBinding::new(Keystroke::primary('p'), KeyCommand::AddToPlaylist, SONGS),
    ]);
}
