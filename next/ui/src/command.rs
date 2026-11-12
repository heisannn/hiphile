use std::rc::Rc;
use std::sync::Arc;

use anti_webui_md3e::components::{Snackbar, Snackbars};
use hiphile_core::Workspace;
use hiphile_core::favorites::Favorite;
use hiphile_core::library::album_songs;
use hiphile_core::navigation::Location;
use next_host::{Pending, Scope};
use playback::Placement;
use subsonic::{AlbumId, PlaylistId, Song};

/// What a screen asks of the shell, which goes elsewhere, opens what is the window's, or acts on
/// the workspace for it.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Show(Location),
    GoBack,
    GoForward,
    OpenSearch,
    OpenSettings,
    /// Puts `songs` in the queue in place of what it has and plays them: from the one at `start`,
    /// or from the start for `None`. Those skipped as instrumentals are left out.
    Play {
        songs: Arc<[Song]>,
        start: Option<usize>,
    },
    /// Adds `songs` to the queue, starting them when nothing is playing.
    Enqueue {
        songs: Arc<[Song]>,
        placement: Placement,
    },
    AddToPlaylist {
        playlist: PlaylistId,
        songs: Arc<[Song]>,
    },
    /// Asks for a name and makes a playlist that starts with the songs, which may be none.
    NewPlaylist(Arc<[Song]>),
    /// Asks which playlist the songs go to: the choice that Add to Playlist offers in menus, for
    /// the keyboard.
    ChoosePlaylist(Arc<[Song]>),
    /// Does `action` to the album's songs once they are had.
    ActOnAlbum {
        album: AlbumId,
        action: AlbumAction,
    },
    /// Adds it to the favorites, or takes it out when it is one.
    ToggleFavorite(Favorite),
    /// Reads again what the screen on show shows.
    Refresh,
}

/// Where a screen sends its [`Command`]s. Cheap to clone.
#[derive(Clone)]
pub struct Commands(Rc<dyn Fn(Command)>);

impl Commands {
    pub fn new(handle: impl Fn(Command) + 'static) -> Self {
        Self(Rc::new(handle))
    }

    pub fn send(&self, command: Command) {
        (self.0)(command);
    }
}

/// What to do with songs that are not at hand yet, such as an album's.
#[derive(Clone, Debug, PartialEq)]
pub enum AlbumAction {
    Play,
    Enqueue(Placement),
    /// `None` for a new playlist.
    AddToPlaylist(Option<PlaylistId>),
    ChoosePlaylist,
}

impl AlbumAction {
    /// What does this to `songs`. `None` when there are none.
    pub fn command(self, songs: Arc<[Song]>) -> Option<Command> {
        if songs.is_empty() {
            return None;
        }
        Some(match self {
            Self::Play => Command::Play { songs, start: None },
            Self::Enqueue(placement) => Command::Enqueue { songs, placement },
            Self::AddToPlaylist(Some(playlist)) => Command::AddToPlaylist { playlist, songs },
            Self::AddToPlaylist(None) => Command::NewPlaylist(songs),
            Self::ChoosePlaylist => Command::ChoosePlaylist(songs),
        })
    }

    /// Has `album`'s songs, the saved ones when there are any, and gives `then` what does this to
    /// them. Why they could not be had goes to `snackbars`. It waits in `scope`, so nothing comes
    /// once its owner is gone, and cancelling it calls the fetch off.
    pub fn start(
        self,
        album: AlbumId,
        workspace: &Workspace,
        scope: Scope,
        snackbars: Snackbars,
        then: impl FnOnce(Command) + 'static,
    ) -> Pending {
        let songs = album_songs(&next_host::executor(), workspace.session(), album);
        scope.on_ui(songs, move |songs| match songs {
            Ok(songs) => {
                if let Some(command) = self.command(songs) {
                    then(command);
                }
            }
            Err(error) => snackbars.show(Snackbar::new(error).long()),
        })
    }
}

#[cfg(test)]
mod tests;
