use std::cell::Cell;

use anti_webui::prelude::NamedKey;
use next_ui::keymap::{KeyBinding, Keystroke, bind_keys};

/// Where the album list's keys work.
pub(super) const ALBUMS: &str = "LibraryAlbums";
/// Where the keys of the songs of the album on show work.
pub(super) const SONGS: &str = "LibrarySongs";

/// What the keys of Albums do. On the album list they act on the album on show, on its songs on
/// the song the selection is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AlbumsKey {
    /// From the albums to the songs of the one on show.
    Open,
    Play,
    PlayNext,
    AddToQueue,
    AddToPlaylist,
    GoToArtist,
    ToggleFavorite,
    /// From the songs back to the albums.
    Back,
}

thread_local! {
    static BOUND: Cell<bool> = const { Cell::new(false) };
}

/// Binds the keys of Albums once on this thread, as the keymap is the thread's.
pub(super) fn bind() {
    if BOUND.with(|bound| bound.replace(true)) {
        return;
    }
    let enter = || Keystroke::named(NamedKey::Enter);
    let mut bindings = vec![
        KeyBinding::new(enter(), AlbumsKey::Open, ALBUMS),
        KeyBinding::new(
            Keystroke::named(NamedKey::ArrowRight),
            AlbumsKey::Open,
            ALBUMS,
        ),
        KeyBinding::new(enter().with_primary(), AlbumsKey::Play, ALBUMS),
        KeyBinding::new(
            Keystroke::named(NamedKey::ArrowLeft),
            AlbumsKey::Back,
            SONGS,
        ),
        KeyBinding::new(Keystroke::named(NamedKey::Escape), AlbumsKey::Back, SONGS),
    ];
    for context in [ALBUMS, SONGS] {
        bindings.extend([
            KeyBinding::new(enter().with_alt(), AlbumsKey::PlayNext, context),
            KeyBinding::new(enter().with_shift(), AlbumsKey::AddToQueue, context),
            KeyBinding::new(Keystroke::primary('p'), AlbumsKey::AddToPlaylist, context),
            KeyBinding::new(
                Keystroke::primary('a').with_shift(),
                AlbumsKey::GoToArtist,
                context,
            ),
            KeyBinding::new(Keystroke::primary('d'), AlbumsKey::ToggleFavorite, context),
        ]);
    }
    bind_keys(bindings);
}
