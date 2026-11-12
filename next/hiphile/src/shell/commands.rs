use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use anti_webui::prelude::*;
use anti_webui_md3e::components::{Snackbar, Snackbars};
use hiphile_core::favorites::FavoritesNotice;
use hiphile_core::playlists::PlaylistsNotice;
use hiphile_core::{Hiphile, Workspace};
use next_host::{Pending, Scope, notices};
use next_player::Panels;
use next_ui::playlist_dialogs::{PlaylistDialog, PlaylistDialogs};
use next_ui::{AlbumAction, Command, Commands, Covers, Known, ScreenContext};
use subsonic::{AlbumId, Song};

use crate::navigation::Navigation;

/// What the commands of the screens act on in the workspace's window.
#[derive(Clone)]
pub(super) struct Shell {
    navigation: Navigation,
    workspace: Workspace,
    settings_open: Signal<bool>,
    search_open: Signal<bool>,
    pub(super) dialogs: PlaylistDialogs,
    pub(super) panels: Panels,
    known: Known,
    snackbars: Snackbars,
    covers: Covers,
    scope: Scope,
    /// The songs of an album being had for an action, which the next action on an album calls
    /// off.
    album: Rc<RefCell<Option<Pending>>>,
}

impl Shell {
    /// Follows what the commands go by, and tells in `snackbars` what the workspace could not do,
    /// for as long as the current owner lives.
    pub(super) fn new(
        hiphile: &Hiphile,
        workspace: Workspace,
        navigation: Navigation,
        settings_open: Signal<bool>,
        search_open: Signal<bool>,
        snackbars: Snackbars,
    ) -> Self {
        tell_notices(&workspace, snackbars);
        Self {
            navigation,
            known: Known::follow(hiphile, &workspace),
            workspace,
            settings_open,
            search_open,
            dialogs: PlaylistDialogs::new(),
            panels: Panels::new(),
            snackbars,
            covers: Covers::new(),
            scope: Scope::here(),
            album: Rc::default(),
        }
    }

    /// What the screens are made with, which sends their commands here.
    pub(super) fn context(&self) -> ScreenContext {
        let this = self.clone();
        let commands = Commands::new(move |command| this.run(command));
        ScreenContext::new(
            self.workspace.clone(),
            commands,
            self.known,
            self.snackbars,
            self.covers,
        )
    }

    pub(super) fn run(&self, command: Command) {
        let workspace = &self.workspace;
        match command {
            Command::Show(location) => {
                self.close_lyrics();
                self.navigation.go(location);
            }
            Command::ShowInPlace(location) => {
                self.close_lyrics();
                self.navigation.show_in_place(location);
            }
            // Out of the lyrics, back to where the window was
            Command::GoBack => {
                if !self.close_lyrics() {
                    self.navigation.back();
                }
            }
            Command::GoForward => {
                if !untrack(|| self.panels.lyrics.get()) {
                    self.navigation.forward();
                }
            }
            Command::OpenSettings => self.settings_open.set(true),
            Command::OpenSearch => self.search_open.set(true),
            // What could not play is told by the player's alerts
            Command::Play { songs, start } => {
                let _ = match start {
                    Some(start) => workspace.player().play(&songs, start),
                    None => workspace.player().play_whole(&songs),
                };
            }
            Command::Shuffle(songs) => {
                let player = workspace.player();
                if !songs.is_empty() && !player.snapshot().is_shuffled() {
                    player.toggle_shuffle();
                }
                let _ = player.play_whole(&songs);
            }
            Command::Enqueue { songs, placement } => {
                workspace.player().enqueue(&songs, placement);
            }
            Command::AddToPlaylist { playlist, songs } => {
                workspace.playlists().add_songs(&playlist, &songs);
            }
            Command::NewPlaylist(songs) => self.dialogs.open(PlaylistDialog::New(songs)),
            Command::ChoosePlaylist(songs) => self.choose_playlist(songs),
            Command::ActOnAlbum { album, action } => self.act_on_album(album, action),
            Command::ToggleFavorite(favorite) => workspace.favorites().toggle(favorite),
            Command::Refresh => workspace.refresh(untrack(|| self.navigation.destination())),
            Command::PlayEntry(entry) => workspace.player().play_entry(entry),
            Command::TogglePlayback => workspace.player().toggle_playback(),
            Command::Next => workspace.player().next(),
            Command::Previous => workspace.player().previous(),
            // The bar reads where the song is often enough to show where it went
            Command::SeekForward => {
                workspace.player().seek_forward();
            }
            Command::SeekBackward => {
                workspace.player().seek_backward();
            }
            Command::VolumeUp => workspace.player().volume_up(),
            Command::VolumeDown => workspace.player().volume_down(),
            Command::ToggleMute => workspace.player().toggle_mute(),
            Command::ToggleShuffle => workspace.player().toggle_shuffle(),
            Command::SetRepeat(repeat) => workspace.player().set_repeat(repeat),
            Command::ToggleQueue => self.panels.toggle_queue(),
            Command::ToggleLyrics => {
                let lyrics = self.panels.lyrics;
                if lyrics.get_untracked() || workspace.player().snapshot().has_song() {
                    lyrics.set(!lyrics.get_untracked());
                }
            }
            Command::SaveQueue => self.save_queue(),
        }
    }

    /// Whether they were open.
    fn close_lyrics(&self) -> bool {
        let lyrics = self.panels.lyrics;
        let was_open = lyrics.get_untracked();
        if was_open {
            lyrics.set(false);
        }
        was_open
    }

    fn save_queue(&self) {
        let songs = self.workspace.player().snapshot().queued_songs();
        if !songs.is_empty() {
            self.dialogs.open(PlaylistDialog::New(songs));
        }
    }

    /// With no playlist to choose, it is the name of a new one that is asked for.
    fn choose_playlist(&self, songs: Arc<[Song]>) {
        if songs.is_empty() {
            return;
        }
        let dialog = if untrack(|| self.known.playlist_choices()).is_empty() {
            PlaylistDialog::New(songs)
        } else {
            PlaylistDialog::Choose(songs)
        };
        self.dialogs.open(dialog);
    }

    fn act_on_album(&self, album: AlbumId, action: AlbumAction) {
        if let Some(before) = self.album.borrow_mut().take() {
            before.cancel();
        }
        let this = self.clone();
        let pending = action.start(
            album,
            &self.workspace,
            self.scope,
            self.snackbars,
            move |command| this.run(command),
        );
        *self.album.borrow_mut() = Some(pending);
    }
}

/// Tells in `snackbars` what the playlists and the favorites could not do, and what they did
/// that no screen shows. The screens leave these to the shell, as the one on show may not be the
/// one that asked.
fn tell_notices(workspace: &Workspace, snackbars: Snackbars) {
    notices(workspace.playlists().notices(), move |notice| {
        snackbars.show(match notice {
            PlaylistsNotice::WriteFailed(message) => Snackbar::new(message).long(),
            PlaylistsNotice::SongsAdded(message) => Snackbar::new(message),
        });
    });
    notices(workspace.favorites().notices(), move |notice| {
        if let FavoritesNotice::WriteFailed(message) = notice {
            snackbars.show(Snackbar::new(message).long());
        }
    });
}

#[cfg(test)]
mod tests;
