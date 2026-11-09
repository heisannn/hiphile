//! Changes to playlists' songs, shown at once and saved behind.

use gpui_kit::component::{WindowExt as _, button::Button, notification::Notification};
use gpui_kit::{Context, Window};
use hiphile_core::playlists::EntryId;

use super::Playlists;

/// Identifies the notification of the last removal, which replaces the one
/// before: only the last removal can be undone.
pub(super) struct SongRemoved;

impl Playlists {
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
        if self.playlists.move_entry(id, to_ix) {
            self.selected_entry = Some(id);
            self.sync(window, cx);
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
            .and_then(|loaded| loaded.entries().position(target));
        if let Some(to_ix) = to_ix {
            self.songs_focus.focus(window, cx);
            self.move_entry(dragged, to_ix, window, cx);
        }
    }

    pub(super) fn remove_selected_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_edit_songs() {
            return;
        }
        let Some(id) = self.selected_entry else {
            return;
        };
        let Some((ix, song)) = self.playlists.remove_entry(id) else {
            return;
        };
        self.selected_entry = None;
        self.sync(window, cx);
        let len = self
            .loaded_detail()
            .map_or(0, |loaded| loaded.entries().len());
        self.select_entry_at(ix.min(len.saturating_sub(1)), cx);

        let message = format!("Removed “{}”", song.title());
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
                // An action alone keeps it up until it is dismissed
                .autohide(true),
            cx,
        );
    }

    pub(super) fn undo_removal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let put_back = self.playlists.undo_removal();
        self.sync(window, cx);
        if let Some(id) = put_back {
            self.select_entry(id, cx);
        }
    }
}
