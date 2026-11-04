//! Actions that one feature dispatches and another handles.
//!
//! Features never depend on each other: `library` dispatches [`PlaySongs`]
//! and the app shell routes it to `player`. Only cross-feature actions belong
//! here; a feature's own commands stay in that feature.

use std::fmt::Display;
use std::sync::Arc;

use gpui_kit::component::{WindowExt as _, menu::PopupMenu, notification::NotificationType};
use gpui_kit::{Action, App, Context, Global, SharedString, Window};
use subsonic::{AlbumId, PlaylistId, Song};

/// Add `songs` after those already in `playlist`.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = hiphile, no_json)]
pub struct AddToPlaylist {
    playlist: PlaylistId,
    songs: Arc<[Song]>,
}

impl AddToPlaylist {
    pub fn new(playlist: PlaylistId, songs: Arc<[Song]>) -> Self {
        Self { playlist, songs }
    }

    pub fn playlist(&self) -> &PlaylistId {
        &self.playlist
    }

    pub fn songs(&self) -> &Arc<[Song]> {
        &self.songs
    }
}

/// Ask for a name and create a playlist starting with `songs`, which may be
/// none.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = hiphile, no_json)]
pub struct NewPlaylist {
    songs: Arc<[Song]>,
}

impl NewPlaylist {
    pub fn new(songs: Arc<[Song]>) -> Self {
        Self { songs }
    }

    /// A playlist starting without songs.
    pub fn empty() -> Self {
        Self::new(Arc::from([]))
    }

    pub fn songs(&self) -> &Arc<[Song]> {
        &self.songs
    }
}

/// Ask which playlist to add `songs` to, from the keyboard: the same choices
/// as [`add_to_playlist_submenu`], which is reached with the pointer.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = hiphile, no_json)]
pub struct ChoosePlaylist {
    songs: Arc<[Song]>,
}

impl ChoosePlaylist {
    pub fn new(songs: Arc<[Song]>) -> Self {
        Self { songs }
    }

    pub fn songs(&self) -> &Arc<[Song]> {
        &self.songs
    }
}

/// The playlists songs can be added to, which the feature owning playlists
/// keeps up to date for the menus of every other feature.
#[derive(Default)]
struct PlaylistChoices(Arc<[(PlaylistId, SharedString)]>);

impl Global for PlaylistChoices {}

/// Replaces the playlists offered by [`add_to_playlist_submenu`].
pub fn set_playlist_choices(choices: Vec<(PlaylistId, SharedString)>, cx: &mut App) {
    cx.set_global(PlaylistChoices(choices.into()));
}

/// The command for one choice of where to add `songs`: the playlist, or
/// `None` for a new one.
pub fn add_songs_command(songs: Arc<[Song]>) -> impl Fn(Option<&PlaylistId>) -> Box<dyn Action> {
    move |playlist| match playlist {
        Some(playlist) => Box::new(AddToPlaylist::new(playlist.clone(), songs.clone())),
        None => Box::new(NewPlaylist::new(songs.clone())),
    }
}

/// Adds every choice of where to add songs to `menu`.
pub fn playlist_choices(
    menu: PopupMenu,
    command: impl Fn(Option<&PlaylistId>) -> Box<dyn Action>,
    cx: &App,
) -> PopupMenu {
    let choices = cx
        .try_global::<PlaylistChoices>()
        .map(|choices| choices.0.clone())
        .unwrap_or_default();
    let menu = menu.scrollable(true).menu("New Playlist…", command(None));
    let menu = if choices.is_empty() {
        menu
    } else {
        menu.separator()
    };
    choices.iter().fold(menu, |menu, (playlist, name)| {
        menu.menu(name.clone(), command(Some(playlist)))
    })
}

/// Adds an Add to Playlist submenu to a context or dropdown menu; `command`
/// is what each of its items runs, usually [`add_songs_command`].
pub fn add_to_playlist_submenu(
    menu: PopupMenu,
    command: impl Fn(Option<&PlaylistId>) -> Box<dyn Action> + 'static,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    menu.submenu("Add to Playlist", window, cx, move |menu, _, cx| {
        playlist_choices(menu, &command, cx)
    })
}

/// What to do with an album's songs once they have been fetched.
#[derive(Clone, Debug, PartialEq)]
pub enum AlbumAction {
    Play,
    Enqueue(Placement),
    /// Add them to this playlist, or to a new one for `None`.
    AddToPlaylist(Option<PlaylistId>),
    ChoosePlaylist,
}

impl AlbumAction {
    /// The command that does this to `songs`, unless there are none.
    pub fn command(self, songs: Arc<[Song]>) -> Option<Box<dyn Action>> {
        if songs.is_empty() {
            return None;
        }
        Some(match self {
            Self::Play => Box::new(PlaySongs::new(songs, 0)),
            Self::Enqueue(placement) => Box::new(EnqueueSongs::new(songs, placement)),
            Self::AddToPlaylist(playlist) => add_songs_command(songs)(playlist.as_ref()),
            Self::ChoosePlaylist => Box::new(ChoosePlaylist::new(songs)),
        })
    }

    /// Does this once the album's songs have been fetched, or says why they
    /// couldn't be.
    pub fn finish<E: Display>(
        self,
        songs: Result<Arc<[Song]>, E>,
        window: &mut Window,
        cx: &mut App,
    ) {
        match songs {
            Ok(songs) => {
                if let Some(command) = self.command(songs) {
                    window.dispatch_action(command, cx);
                }
            }
            Err(error) => {
                let message = format!("Couldn't load this album. {error}");
                window.push_notification((NotificationType::Error, message), cx);
            }
        }
    }
}

/// Show `album` in the library.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = hiphile, no_json)]
pub struct ShowAlbum {
    album: AlbumId,
}

impl ShowAlbum {
    pub fn new(album: AlbumId) -> Self {
        Self { album }
    }

    pub fn album(&self) -> &AlbumId {
        &self.album
    }
}

/// Replace the play queue with `songs` and start playing at `start_ix`.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = hiphile, no_json)]
pub struct PlaySongs {
    songs: Arc<[Song]>,
    start_ix: usize,
}

impl PlaySongs {
    pub fn new(songs: Arc<[Song]>, start_ix: usize) -> Self {
        Self { songs, start_ix }
    }

    pub fn songs(&self) -> &Arc<[Song]> {
        &self.songs
    }

    pub fn start_ix(&self) -> usize {
        self.start_ix
    }
}

/// Where [`EnqueueSongs`] adds its songs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Right after the current song.
    Next,
    /// After every other song.
    Last,
}

/// Add `songs` to the play queue at `placement`, starting them when nothing
/// is playing.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = hiphile, no_json)]
pub struct EnqueueSongs {
    songs: Arc<[Song]>,
    placement: Placement,
}

impl EnqueueSongs {
    pub fn new(songs: Arc<[Song]>, placement: Placement) -> Self {
        Self { songs, placement }
    }

    pub fn songs(&self) -> &Arc<[Song]> {
        &self.songs
    }

    pub fn placement(&self) -> Placement {
        self.placement
    }
}
