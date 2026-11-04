//! Changes to playlists' songs, shown at once and saved behind.

use gpui_kit::component::{
    WindowExt as _,
    button::Button,
    notification::{Notification, NotificationType},
};
use gpui_kit::{AppContext as _, Context, Window};
use subsonic::{PlaylistId, Song};

use super::Playlists;
use crate::entries::EntryId;
use crate::writes::Write;

pub(super) struct RemovedSong {
    playlist: PlaylistId,
    ix: usize,
    song: Song,
}

/// Identifies the notification of the last removal, which replaces the one
/// before: only the last removal can be undone.
struct SongRemoved;
/// Identifies the notification of the last addition.
struct SongsAdded;

impl Playlists {
    /// Whether a change to `playlist` has yet to reach the server.
    pub(super) fn has_unsaved(&self, playlist: &PlaylistId) -> bool {
        self.sending.as_ref() == Some(playlist) || self.writes.contains(playlist)
    }

    /// Queues `write` and starts on it unless another is going.
    pub(super) fn write(&mut self, write: Write, window: &mut Window, cx: &mut Context<Self>) {
        self.edits += 1;
        self.writes.push(write);
        if self.sending.is_none() {
            self.write_next(window, cx);
        }
    }

    /// Sends the next write. Once none is left, fetches what the server made
    /// of them, as only it knows a playlist's new totals and cover.
    fn write_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(write) = self.writes.pop() else {
            self.sending = None;
            self.refresh(window, cx);
            return;
        };
        self.sending = Some(write.playlist().clone());
        let client = self.session.client().clone();
        self._write_task = cx.spawn_in(window, async move |this, cx| {
            let sent = write.clone();
            let result = cx
                .background_spawn(async move { sent.send(client).await })
                .await;
            this.update_in(cx, |this, window, cx| {
                if let Err(error) = result {
                    this.save_failed(write.playlist(), window, cx);
                    let message = format!("{} {}", write.failure(), session::describe(&error));
                    window.push_notification((NotificationType::Error, message), cx);
                }
                this.write_next(window, cx);
            })
            .ok();
        });
    }

    fn save_failed(&mut self, playlist: &PlaylistId, window: &mut Window, cx: &mut Context<Self>) {
        self.writes.forget(playlist);
        self.edits += 1;
        if self
            .loaded_detail()
            .is_some_and(|loaded| loaded.playlist.id() == playlist)
        {
            self.diverged = true;
        }
        // The removal may not have happened, so that putting the song back
        // would make two of it.
        if self
            .removed
            .as_ref()
            .is_some_and(|removed| &removed.playlist == playlist)
        {
            self.removed = None;
            window.remove_notification::<SongRemoved>(cx);
        }
    }

    /// Saves the song list on show after an edit to it.
    fn write_entries(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(loaded) = self.loaded_detail() else {
            return;
        };
        let write = Write::ReplaceSongs {
            playlist: loaded.playlist.id().clone(),
            songs: loaded.entries.song_ids(),
        };
        self.write(write, window, cx);
    }

    fn move_entry(
        &mut self,
        id: EntryId,
        to_ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_edit_songs() {
            return;
        }
        let moved = self
            .loaded_detail_mut()
            .is_some_and(|loaded| loaded.entries.move_entry(id, to_ix));
        if moved {
            self.selected_entry = Some(id);
            self.write_entries(window, cx);
            self.select_entry(id, cx);
        }
    }

    pub(super) fn move_selected_entry(
        &mut self,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(id), Some(ix)) = (self.selected_entry, self.selected_entry_ix()) else {
            return;
        };
        if let Some(to_ix) = ix.checked_add_signed(delta) {
            self.move_entry(id, to_ix, window, cx);
        }
    }

    /// Moves the dragged entry to where `target` is: after it when dragged
    /// down, before it when dragged up, as the row's drop line shows.
    pub(super) fn drop_entry(
        &mut self,
        dragged: EntryId,
        target: EntryId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let to_ix = self
            .loaded_detail()
            .and_then(|loaded| loaded.entries.position(target));
        if let Some(to_ix) = to_ix {
            self.songs_focus.focus(window, cx);
            self.move_entry(dragged, to_ix, window, cx);
        }
    }

    /// Takes the selected song out and offers to put it back.
    pub(super) fn remove_selected_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_edit_songs() {
            return;
        }
        let Some(id) = self.selected_entry else {
            return;
        };
        let Some(loaded) = self.loaded_detail_mut() else {
            return;
        };
        let playlist = loaded.playlist.id().clone();
        let Some((ix, song)) = loaded.entries.remove(id) else {
            return;
        };
        let next_ix = ix.min(loaded.entries.len().saturating_sub(1));
        self.selected_entry = None;
        self.select_entry_at(next_ix, cx);
        self.write_entries(window, cx);

        let message = format!("Removed “{}”", song.title());
        self.removed = Some(RemovedSong { playlist, ix, song });
        let owner = cx.entity().downgrade();
        window.push_notification(
            Notification::new()
                .id::<SongRemoved>()
                .message(message)
                .action(move |_, _, _| {
                    let owner = owner.clone();
                    Button::new("undo-remove")
                        .label("Undo")
                        .on_click(move |_, window, cx| {
                            owner
                                .update(cx, |this, cx| this.undo_removal(window, cx))
                                .ok();
                            window.remove_notification::<SongRemoved>(cx);
                        })
                })
                // An action alone keeps it up until it is dismissed.
                .autohide(true),
            cx,
        );
        cx.notify();
    }

    pub(super) fn undo_removal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(RemovedSong { playlist, ix, song }) = self.removed.take() else {
            return;
        };
        let write = match self.loaded_detail_mut() {
            Some(loaded) if loaded.playlist.id() == &playlist => {
                let id = loaded.entries.insert(ix, song);
                self.selected_entry = Some(id);
                self.write_entries(window, cx);
                self.select_entry(id, cx);
                return;
            }
            _ => Write::InsertSong {
                playlist,
                ix,
                song: song.id().clone(),
            },
        };
        self.write(write, window, cx);
    }

    /// Adds `songs` after those already in `playlist`, and says so: the
    /// playlist is usually not on show.
    pub fn add_songs(
        &mut self,
        playlist: &PlaylistId,
        songs: &[Song],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if songs.is_empty() {
            return;
        }
        if let Some(loaded) = self
            .loaded_detail_mut()
            .filter(|loaded| loaded.playlist.id() == playlist)
        {
            loaded.entries.extend(songs.iter().cloned());
            cx.notify();
        }
        let write = Write::AddSongs {
            playlist: playlist.clone(),
            songs: songs.iter().map(|song| song.id().clone()).collect(),
        };
        self.write(write, window, cx);
        let name = self
            .loaded_list()
            .and_then(|list| list.iter().find(|listed| listed.id() == playlist))
            .map_or_else(
                || "the playlist".to_owned(),
                |listed| listed.name().to_owned(),
            );
        window.push_notification(
            Notification::new()
                .id::<SongsAdded>()
                .message(format!("Added to {name}")),
            cx,
        );
    }
}
