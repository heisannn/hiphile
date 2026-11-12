//! The dialogs that file songs in playlists: naming a new one, and choosing one from the keyboard.

use std::sync::Arc;

use anti_webui::prelude::*;
use anti_webui_md3e::components::{
    ButtonStyle, Snackbar, TextFieldStyle, dialog, indeterminate_circular_progress,
    outlined_text_field, styled_button,
};
use anti_webui_md3e::typography::TextStyle;
use hiphile_core::navigation::Location;
use hiphile_core::playlists::{PlaylistChoice, new_playlist_scope};
use next_host::Scope;
use subsonic::{PlaylistId, Song};

use crate::dialog_parts::{dialog_actions, dialog_content, focused_field, focused_once_laid_out};
use crate::row_list::{RowList, Selection, row_list};
use crate::row_text::row_text;
use crate::type_scale::muted_text;
use crate::{Command, ScreenContext};

const WIDTH: i32 = 400;
const ROW_HEIGHT: f32 = 40.0;
/// Rows of playlists shown at once, past which the list scrolls.
const ROWS_SHOWN: usize = 6;

/// What a [`PlaylistDialogs`] asks.
#[derive(Clone, Debug, PartialEq)]
pub enum PlaylistDialog {
    /// The name of a new playlist that starts with the songs, which may be none.
    New(Arc<[Song]>),
    /// Which playlist the songs go to.
    Choose(Arc<[Song]>),
}

/// The playlist dialogs of a window, which the shell opens for [`Command::NewPlaylist`] and
/// [`Command::ChoosePlaylist`]. A dialog that could not do what it was asked stays open and says
/// why. Cheap to copy.
#[derive(Clone, Copy)]
pub struct PlaylistDialogs {
    open: Signal<bool>,
    /// What it shows, which stays while it closes.
    shown: Signal<Option<PlaylistDialog>>,
    /// How many times it has been opened or changed, as each time starts afresh.
    opened: Signal<u64>,
    /// Whether it waits for the server, when it can't be closed.
    busy: Signal<bool>,
}

impl Default for PlaylistDialogs {
    fn default() -> Self {
        Self::new()
    }
}

impl PlaylistDialogs {
    pub fn new() -> Self {
        Self {
            open: signal(false),
            shown: signal(None),
            opened: signal(0),
            busy: signal(false),
        }
    }

    /// Opens `dialog`, unless one is open already, which it does not go over.
    pub fn open(&self, dialog: PlaylistDialog) {
        if self.open.get_untracked() {
            return;
        }
        self.show(dialog);
        self.open.set(true);
    }

    fn show(&self, dialog: PlaylistDialog) {
        self.busy.set(false);
        self.shown.set(Some(dialog));
        self.opened.update(|opened| *opened += 1);
    }

    pub fn close(&self) {
        self.open.set(false);
    }

    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    pub fn view(&self, cx: &ScreenContext) -> Show {
        let this = *self;
        let cx = cx.clone();
        dialog(
            move || this.open.get(),
            move || {
                if !this.busy.get_untracked() {
                    this.close();
                }
            },
            move || {
                let cx = cx.clone();
                div().child(dyn_view(move || {
                    this.opened.get();
                    let cx = cx.clone();
                    untrack(move || match this.shown.get() {
                        Some(PlaylistDialog::New(songs)) => new_playlist(this, &cx, songs),
                        Some(PlaylistDialog::Choose(songs)) => choose_playlist(this, &cx, songs),
                        None => div(),
                    })
                }))
            },
        )
    }
}

/// Asks for the name, and makes the playlist while it waits: unlike other changes to the
/// playlists, it has nothing to show until the server gives it an id. One with songs is told of
/// once made, and an empty one is shown.
fn new_playlist(dialogs: PlaylistDialogs, cx: &ScreenContext, songs: Arc<[Song]>) -> Div {
    let name = signal(String::new());
    let error = signal(None::<String>);
    let busy = dialogs.busy;
    effect(move || {
        name.with(|_| ());
        untrack(|| error.set(None));
    });
    let blank = move || name.with(|name| name.trim().is_empty());
    let scope = Scope::here();
    let submit = {
        let cx = cx.clone();
        let songs = songs.clone();
        move || {
            if busy.get_untracked() || untrack(blank) {
                return;
            }
            busy.set(true);
            let created = cx
                .workspace
                .playlists()
                .create(&name.get_untracked(), &songs);
            let (commands, snackbars, songs) = (cx.commands.clone(), cx.snackbars, songs.clone());
            let _ = scope.on_ui(created, move |created| {
                busy.set(false);
                match created {
                    Ok(created) => {
                        dialogs.close();
                        if songs.is_empty() {
                            let to = Location::Playlists(Some(created.id().clone()));
                            commands.send(Command::Show(to));
                        } else {
                            snackbars.show(Snackbar::new(created.message()));
                        }
                    }
                    Err(failure) => error.set(Some(failure)),
                }
            });
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
                .supporting(move || error.get().unwrap_or_default())
                .disabled_when(move || busy.get()),
        )
        .w(WIDTH)
    };
    let row = dialog_actions()
        .child(show(
            move || busy.get(),
            || indeterminate_circular_progress().mr(8),
        ))
        .child(
            styled_button(
                "Cancel",
                ButtonStyle::text().disabled_when(move || busy.get()),
            )
            .on_click(move |_| dialogs.close()),
        )
        .child(
            styled_button(
                "Create",
                ButtonStyle::filled().disabled_when(move || busy.get() || blank()),
            )
            .on_click(move |_| submit()),
        );
    dialog_content("New Playlist")
        .children(new_playlist_scope(&songs).map(|scope| muted_text(TextStyle::BODY_MEDIUM, scope)))
        .child(focused_field(field))
        .child(row)
}

/// The user's playlists, of which Enter, a double click or Add takes the songs to the one chosen.
/// It has New Playlist… too, which names one for them instead.
fn choose_playlist(dialogs: PlaylistDialogs, cx: &ScreenContext, songs: Arc<[Song]>) -> Div {
    let known = cx.known;
    let choices = untrack(|| known.playlist_choices());
    let selection = Selection::<PlaylistId>::new();
    if let Some(first) = choices.first() {
        selection.select(first.id().clone());
    }
    let add = {
        let commands = cx.commands.clone();
        let songs = songs.clone();
        move |playlist: PlaylistId| {
            dialogs.close();
            commands.send(Command::AddToPlaylist {
                playlist,
                songs: songs.clone(),
            });
        }
    };
    let list = node_ref();
    let rows = row_list(
        RowList::new(
            "Playlists",
            move || known.playlist_choices(),
            |choice: &PlaylistChoice| choice.id().clone(),
            |choice, _| row_text(choice.name().to_string(), None).px(16.0_f32),
        )
        .row_height(ROW_HEIGHT)
        .selection(selection)
        .node_ref(list)
        .on_open({
            let add = add.clone();
            move |playlist| add(playlist)
        }),
    );
    let height = move || {
        let rows = known.playlist_choices().len().clamp(1, ROWS_SHOWN);
        // With the room between the edge of the list and its rows
        rows as f32 * ROW_HEIGHT + 16.0
    };
    let rows = div().w(WIDTH).dyn_style(move |s| s.h(height())).child(rows);
    let row = dialog_actions()
        .child(
            styled_button("New Playlist…", ButtonStyle::text())
                .on_click(move |_| dialogs.show(PlaylistDialog::New(songs.clone()))),
        )
        .child(div().flex_1())
        .child(styled_button("Cancel", ButtonStyle::text()).on_click(move |_| dialogs.close()))
        .child(
            styled_button(
                "Add",
                ButtonStyle::filled().disabled_when(move || selection.cursor().is_none()),
            )
            .on_click(move |_| {
                if let Some(playlist) = selection.cursor() {
                    add(playlist);
                }
            }),
        );
    dialog_content("Add to Playlist")
        .child(focused_once_laid_out(rows, move |_, _| list.get()))
        .child(row)
}

#[cfg(test)]
mod tests;
