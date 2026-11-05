//! Actions that one feature dispatches and another handles.
//!
//! Features never depend on each other: `library` dispatches [`PlaySongs`]
//! and the app shell routes it to `player`. Only cross-feature actions belong
//! here; a feature's own commands stay in that feature.

use std::collections::HashSet;
use std::fmt::Display;
use std::sync::Arc;

use gpui_kit::component::{WindowExt as _, menu::PopupMenu, notification::NotificationType};
use gpui_kit::{
    Action, App, Context, Global, KeyBinding, SharedString, Subscription, Window, actions,
};
use subsonic::{Album, AlbumId, Artist, ArtistId, PlaylistId, Song, StarTarget};

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
            Self::Play => Box::new(PlaySongs::whole(songs)),
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

/// Show `artist` on the artists' screen.
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

/// Binds the key for [`GoToArtist`] in the view whose key context is `context`.
pub fn bind_go_to_artist(context: &'static str, cx: &mut App) {
    let key = if cx.platform_info().is_mac() {
        "cmd-shift-a"
    } else {
        "ctrl-shift-a"
    };
    cx.bind_keys([KeyBinding::new(key, GoToArtist, Some(context))]);
}

/// A song, an album or an artist, whole, as the favorites' screen lists it.
#[derive(Clone, Debug, PartialEq)]
pub enum Favorite {
    Song(Song),
    Album(Album),
    Artist(Artist),
}

impl Favorite {
    pub fn target(&self) -> StarTarget {
        match self {
            Self::Song(song) => StarTarget::Song(song.id().clone()),
            Self::Album(album) => StarTarget::Album(album.id().clone()),
            Self::Artist(artist) => StarTarget::Artist(artist.id().clone()),
        }
    }
}

/// Make `favorite` one of the user's favorites, or stop it being one.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = hiphile, no_json)]
pub struct SetFavorite {
    favorite: Favorite,
    is_favorite: bool,
}

impl SetFavorite {
    pub fn new(favorite: Favorite, is_favorite: bool) -> Self {
        Self {
            favorite,
            is_favorite,
        }
    }

    pub fn favorite(&self) -> &Favorite {
        &self.favorite
    }

    pub fn is_favorite(&self) -> bool {
        self.is_favorite
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

/// Binds the key for [`ToggleFavorite`] in the view whose key context is
/// `context`.
pub fn bind_toggle_favorite(context: &'static str, cx: &mut App) {
    let key = if cx.platform_info().is_mac() {
        "cmd-d"
    } else {
        "ctrl-d"
    };
    cx.bind_keys([KeyBinding::new(key, ToggleFavorite, Some(context))]);
}

/// What the user's favorites are, which the feature owning them keeps up to
/// date for every other feature. Holds nothing until the server has said.
#[derive(Default)]
struct KnownFavorites(Option<HashSet<StarTarget>>);

impl Global for KnownFavorites {}

/// Replaces what [`is_favorite`] goes by; `None` when it is not known.
pub fn set_known_favorites(favorites: Option<HashSet<StarTarget>>, cx: &mut App) {
    cx.set_global(KnownFavorites(favorites));
}

/// Whether `target` is a favorite, once that is known.
pub fn is_favorite(target: &StarTarget, cx: &App) -> Option<bool> {
    let favorites = cx.try_global::<KnownFavorites>()?.0.as_ref()?;
    Some(favorites.contains(target))
}

/// Calls for `view` to render again whenever the favorites change, for as long
/// as the subscription lives.
pub fn observe_favorites<V: 'static>(cx: &mut Context<V>) -> Subscription {
    cx.observe_global::<KnownFavorites>(|_, cx| cx.notify())
}

/// What [`ToggleFavorite`] does to `target` now, as a command's label says it.
pub fn favorite_command_label(target: &StarTarget, cx: &App) -> &'static str {
    if is_favorite(target, cx) == Some(true) {
        "Remove from Favorites"
    } else {
        "Add to Favorites"
    }
}

/// Adds the item for [`ToggleFavorite`] on `target` to a context or dropdown
/// menu, whose action context handles it. It stays disabled until the
/// favorites are known, as it could not say which way it goes.
pub fn favorite_menu_item(menu: PopupMenu, target: &StarTarget, cx: &App) -> PopupMenu {
    menu.menu_with_disabled(
        favorite_command_label(target, cx),
        Box::new(ToggleFavorite),
        is_favorite(target, cx).is_none(),
    )
}

/// Dispatches what [`ToggleFavorite`] means for `favorite`. Does nothing until
/// the favorites are known.
pub fn toggle_favorite(favorite: Favorite, window: &mut Window, cx: &mut App) {
    if let Some(is_favorite) = is_favorite(&favorite.target(), cx) {
        window.dispatch_action(Box::new(SetFavorite::new(favorite, !is_favorite)), cx);
    }
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

    /// Plays all of an album or a playlist, with no song chosen to start at.
    pub fn whole(songs: Arc<[Song]>) -> Self {
        Self {
            songs,
            start_ix: None,
        }
    }

    pub fn songs(&self) -> &Arc<[Song]> {
        &self.songs
    }

    /// The song chosen to start at, if one was.
    pub fn start_ix(&self) -> Option<usize> {
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
