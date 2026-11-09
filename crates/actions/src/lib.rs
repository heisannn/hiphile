//! Actions that one feature dispatches and another handles.
//!
//! Features never depend on each other: `library` dispatches [`PlaySongs`]
//! and the app shell routes it to `player`. Only cross-feature actions belong
//! here; a feature's own commands stay in that feature.

use std::collections::HashSet;
use std::sync::Arc;

mod instrumentals;

use gpui_kit::component::{WindowExt as _, menu::PopupMenu, notification::NotificationType};
use gpui_kit::{
    Action, App, Context, Global, KeyBinding, SharedString, Subscription, Task, Window, actions,
};
use hiphile_core::favorites::Favorite;
use hiphile_core::library::album_songs;
use hiphile_core::playlists::PlaylistChoice;
use subsonic::{AlbumId, ArtistId, PlaylistId, Song, StarTarget};

pub use instrumentals::{is_skipped, queue_menu_items, set_skip_instrumentals, without_skipped};

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

/// The playlists songs can be added to, which the app shell keeps in step with
/// the playlists for the menus of every feature.
#[derive(Default)]
struct PlaylistChoices(Arc<[PlaylistChoice]>);

impl Global for PlaylistChoices {}

pub fn set_playlist_choices(choices: Arc<[PlaylistChoice]>, cx: &mut App) {
    cx.set_global(PlaylistChoices(choices));
}

/// The returned command takes the playlist to add to, or `None` for a new one.
pub fn add_songs_command(songs: Arc<[Song]>) -> impl Fn(Option<&PlaylistId>) -> Box<dyn Action> {
    move |playlist| match playlist {
        Some(playlist) => Box::new(AddToPlaylist::new(playlist.clone(), songs.clone())),
        None => Box::new(NewPlaylist::new(songs.clone())),
    }
}

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
    choices.iter().fold(menu, |menu, choice| {
        let name = SharedString::from(choice.name().clone());
        menu.menu(name, command(Some(choice.id())))
    })
}

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

#[derive(Clone, Debug, PartialEq)]
pub enum AlbumAction {
    Play,
    Enqueue(Placement),
    /// `None` for a new playlist.
    AddToPlaylist(Option<PlaylistId>),
    ChoosePlaylist,
}

impl AlbumAction {
    /// `None` when `songs` is empty.
    pub fn command(self, songs: Arc<[Song]>) -> Option<Box<dyn Action>> {
        if songs.is_empty() {
            return None;
        }
        Some(match self {
            Self::Play => Box::new(PlaySongs::whole(songs)),
            Self::Enqueue(placement) => Box::new(EnqueueSongs::new(songs, placement)),
            Self::AddToPlaylist(playlist) => add_songs_command(songs)(playlist.as_ref()),
            Self::ChoosePlaylist => Box::new(ChoosePlaylist::new(songs)),
        })
    }

    /// Does this to `album`'s songs, the saved ones when there are any, until the task is
    /// dropped.
    pub fn start(
        self,
        album: AlbumId,
        session: &session::Session,
        window: &Window,
        cx: &App,
    ) -> Task<()> {
        let songs = album_songs(&gpui_host::executor(cx), session, album);
        window.spawn(cx, async move |cx| {
            let songs = songs.await;
            cx.update(|window, cx| self.finish(songs, window, cx)).ok();
        })
    }

    fn finish(self, songs: Result<Arc<[Song]>, String>, window: &mut Window, cx: &mut App) {
        match songs {
            Ok(songs) => {
                if let Some(command) = self.command(songs) {
                    window.dispatch_action(command, cx);
                }
            }
            Err(error) => window.push_notification((NotificationType::Error, error), cx),
        }
    }
}

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

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = hiphile, no_json)]
pub struct ShowArtist {
    artist: ArtistId,
}

impl ShowArtist {
    pub fn new(artist: ArtistId) -> Self {
        Self { artist }
    }

    pub fn artist(&self) -> &ArtistId {
        &self.artist
    }
}

actions!(
    hiphile,
    [
        /// Shows the artist of what is selected. The view with the selection
        /// handles it, and dispatches [`ShowArtist`].
        GoToArtist
    ]
);

pub fn bind_go_to_artist(context: &'static str, cx: &mut App) {
    let key = if cx.platform_info().is_mac() {
        "cmd-shift-a"
    } else {
        "ctrl-shift-a"
    };
    cx.bind_keys([KeyBinding::new(key, GoToArtist, Some(context))]);
}

/// Adds `favorite` to the favorites, or takes it out when it is one. The app shell handles it,
/// and the favorites decide which way it goes.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = hiphile, no_json)]
pub struct ToggleFavoriteOf {
    favorite: Favorite,
}

impl ToggleFavoriteOf {
    pub fn new(favorite: Favorite) -> Self {
        Self { favorite }
    }

    pub fn favorite(&self) -> &Favorite {
        &self.favorite
    }
}

actions!(
    hiphile,
    [
        /// Adds what is selected to the favorites, or takes it out. The view
        /// with the selection handles it, through [`toggle_favorite`].
        ToggleFavorite
    ]
);

pub fn bind_toggle_favorite(context: &'static str, cx: &mut App) {
    let key = if cx.platform_info().is_mac() {
        "cmd-d"
    } else {
        "ctrl-d"
    };
    cx.bind_keys([KeyBinding::new(key, ToggleFavorite, Some(context))]);
}

/// What the user's favorites are, which the app shell keeps in step with the
/// favorites for every feature. Holds nothing until the server has said.
#[derive(Default)]
struct KnownFavorites(Option<HashSet<StarTarget>>);

impl Global for KnownFavorites {}

pub fn set_known_favorites(favorites: Option<HashSet<StarTarget>>, cx: &mut App) {
    cx.set_global(KnownFavorites(favorites));
}

/// `None` until the favorites are known.
pub fn is_favorite(target: &StarTarget, cx: &App) -> Option<bool> {
    let favorites = cx.try_global::<KnownFavorites>()?.0.as_ref()?;
    Some(favorites.contains(target))
}

pub fn observe_favorites<V: 'static>(cx: &mut Context<V>) -> Subscription {
    cx.observe_global::<KnownFavorites>(|_, cx| cx.notify())
}

pub fn favorite_command_label(target: &StarTarget, cx: &App) -> &'static str {
    if is_favorite(target, cx) == Some(true) {
        "Remove from Favorites"
    } else {
        "Add to Favorites"
    }
}

/// Disabled until the favorites are known, as it could not say which way it goes.
pub fn favorite_menu_item(menu: PopupMenu, target: &StarTarget, cx: &App) -> PopupMenu {
    menu.menu_with_disabled(
        favorite_command_label(target, cx),
        Box::new(ToggleFavorite),
        is_favorite(target, cx).is_none(),
    )
}

pub fn toggle_favorite(favorite: Favorite, window: &mut Window, cx: &mut App) {
    window.dispatch_action(Box::new(ToggleFavoriteOf::new(favorite)), cx);
}

/// Replace the play queue with `songs` and start playing.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = hiphile, no_json)]
pub struct PlaySongs {
    songs: Arc<[Song]>,
    start_ix: Option<usize>,
}

impl PlaySongs {
    /// Starts at the song chosen, shuffled or not.
    pub fn new(songs: Arc<[Song]>, start_ix: usize) -> Self {
        Self {
            songs,
            start_ix: Some(start_ix),
        }
    }

    pub fn whole(songs: Arc<[Song]>) -> Self {
        Self {
            songs,
            start_ix: None,
        }
    }

    pub fn songs(&self) -> &Arc<[Song]> {
        &self.songs
    }

    pub fn start_ix(&self) -> Option<usize> {
        self.start_ix
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Next,
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

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;
    use subsonic::{AlbumId, SongId};

    use super::*;

    #[gpui_kit::test]
    fn a_favorite_is_unknown_until_they_are_told_then_goes_by_its_kind(cx: &mut TestAppContext) {
        let song = StarTarget::Song(SongId::new("x"));
        let album = StarTarget::Album(AlbumId::new("x"));
        cx.update(|cx| {
            assert_eq!(is_favorite(&song, cx), None);

            set_known_favorites(Some(HashSet::from([song.clone()])), cx);
            assert_eq!(is_favorite(&song, cx), Some(true));
            assert_eq!(is_favorite(&album, cx), Some(false));

            set_known_favorites(None, cx);
            assert_eq!(is_favorite(&song, cx), None);
        });
    }

    #[gpui_kit::test]
    fn the_command_says_which_way_it_goes(cx: &mut TestAppContext) {
        let song = StarTarget::Song(SongId::new("x"));
        let other = StarTarget::Song(SongId::new("y"));
        cx.update(|cx| {
            assert_eq!(favorite_command_label(&song, cx), "Add to Favorites");

            set_known_favorites(Some(HashSet::from([song.clone()])), cx);
            assert_eq!(favorite_command_label(&song, cx), "Remove from Favorites");
            assert_eq!(favorite_command_label(&other, cx), "Add to Favorites");
        });
    }
}
