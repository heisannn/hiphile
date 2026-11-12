//! Renaming a playlist, and the question before deleting one.

use anti_webui::prelude::*;
use anti_webui_md3e::components::{
    AlertDialog, ButtonStyle, TextFieldStyle, alert_dialog, dialog, outlined_text_field,
    styled_button,
};
use next_ui::dialog_parts::{dialog_actions, dialog_content, focused_field};
use subsonic::PlaylistId;

use super::Playlists;

const WIDTH: i32 = 400;

/// The screen's dialogs, of which one is open at a time. Cheap to copy.
#[derive(Clone, Copy)]
pub(super) struct Dialogs {
    rename_open: Signal<bool>,
    /// What is renamed, which stays while the dialog closes.
    renaming: Signal<Option<PlaylistId>>,
    delete_open: Signal<bool>,
    deleting: Signal<Option<PlaylistId>>,
}

impl Dialogs {
    pub(super) fn new() -> Self {
        Self {
            rename_open: signal(false),
            renaming: signal(None),
            delete_open: signal(false),
            deleting: signal(None),
        }
    }

    fn is_open(&self) -> bool {
        self.rename_open.get_untracked() || self.delete_open.get_untracked()
    }

    pub(super) fn rename(&self, playlist: PlaylistId) {
        if self.is_open() {
            return;
        }
        self.renaming.set(Some(playlist));
        self.rename_open.set(true);
    }

    pub(super) fn delete(&self, playlist: PlaylistId) {
        if self.is_open() {
            return;
        }
        self.deleting.set(Some(playlist));
        self.delete_open.set(true);
    }

    fn close(&self) {
        self.rename_open.set(false);
        self.delete_open.set(false);
    }
}

/// Asks for the new name. A name the playlist can't take keeps the dialog open and says why.
pub(super) fn rename_dialog(screen: &Playlists) -> Show {
    let dialogs = screen.dialogs;
    let screen = screen.clone();
    dialog(
        move || dialogs.rename_open.get(),
        move || dialogs.close(),
        move || match dialogs.renaming.get_untracked() {
            Some(playlist) => rename(&screen, playlist),
            None => div(),
        },
    )
}

fn rename(screen: &Playlists, playlist: PlaylistId) -> Div {
    let dialogs = screen.dialogs;
    let current = screen
        .playlist(&playlist)
        .map(|playlist| playlist.name().to_owned())
        .unwrap_or_default();
    let name = signal(current);
    let error = signal(None::<String>);
    effect(move || {
        name.with(|_| ());
        untrack(|| error.set(None));
    });
    let blank = move || name.with(|name| name.trim().is_empty());
    let state = screen.state.clone();
    let submit = move || {
        if untrack(blank) {
            return;
        }
        match state.rename(playlist.clone(), &name.get_untracked()) {
            Ok(()) => dialogs.close(),
            Err(failure) => error.set(Some(failure)),
        }
    };
    let field = {
        let submit = submit.clone();
        outlined_text_field(
            name,
            TextFieldStyle::new()
                .label("Name")
                .on_submit(move |_| submit())
                .error_when(move || error.with(Option::is_some))
                .supporting(move || error.get().unwrap_or_default()),
        )
        .w(WIDTH)
    };
    let actions = dialog_actions()
        .child(styled_button("Cancel", ButtonStyle::text()).on_click(move |_| dialogs.close()))
        .child(
            styled_button("Rename", ButtonStyle::filled().disabled_when(blank))
                .on_click(move |_| submit()),
        );
    dialog_content("Rename Playlist")
        .child(focused_field(field))
        .child(actions)
}

/// Asks before deleting, which can't be undone, naming the playlist.
pub(super) fn delete_dialog(screen: &Playlists) -> View {
    let dialogs = screen.dialogs;
    let screen = screen.clone();
    dyn_view(move || {
        let playlist = dialogs.deleting.get();
        let screen = screen.clone();
        untrack(move || {
            let playlist = playlist?;
            let confirmation = screen.state.snapshot().delete_confirmation(&playlist)?;
            let alert = AlertDialog::new()
                .title(confirmation.title())
                .text(confirmation.message())
                .dismiss(move || {
                    styled_button("Cancel", ButtonStyle::text()).on_click(move |_| dialogs.close())
                })
                .confirm(move || {
                    let (screen, playlist) = (screen.clone(), playlist.clone());
                    styled_button("Delete", ButtonStyle::text()).on_click(move |_| {
                        dialogs.close();
                        screen.delete(playlist.clone());
                    })
                });
            Some(alert_dialog(
                move || dialogs.delete_open.get(),
                move || dialogs.close(),
                alert,
            ))
        })
    })
}
