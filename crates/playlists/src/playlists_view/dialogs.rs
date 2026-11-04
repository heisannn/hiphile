//! The dialogs that create, rename and delete a playlist, and the menu that
//! chooses one from the keyboard.

use std::sync::Arc;

use gpui_kit::component::{
    WindowExt as _, button::ButtonVariant, menu::PopupMenu, notification::Notification,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, DismissEvent, Entity, FocusHandle, Focusable as _,
    IntoElement as _, ParentElement as _, SharedString, Subscription, Window, anchored, deferred,
};
use subsonic::{PlaylistId, PlaylistWithSongs, Song, SongId};

use super::{Playlists, PlaylistsEvent};
use crate::name_form::NameForm;
use crate::writes::Write;

/// A menu of playlists, opened from the keyboard. It stands apart from any row
/// and from the screen, as the screen on show may be another feature's.
pub(super) struct Chooser {
    pub(super) menu: Entity<PopupMenu>,
    /// Where focus returns when it closes.
    previous_focus: Option<FocusHandle>,
    _dismissal: Subscription,
}

impl Playlists {
    /// Asks for a name and creates a playlist starting with `songs`.
    pub fn open_new_playlist(
        &mut self,
        songs: Arc<[Song]>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Its shortcut still works over a dialog.
        if window.has_active_dialog(cx) {
            return;
        }
        // Before the dialog, which returns focus to where it was opened from.
        self.close_chooser(window, cx);
        let form = self.new_playlist_form(songs, window, cx);
        open_name_dialog("New Playlist", form, window, cx);
    }

    pub(super) fn new_playlist_form(
        &self,
        songs: Arc<[Song]>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<NameForm> {
        let scope = match &*songs {
            [] => None,
            [song] => Some(format!("Starts with “{}”", song.title()).into()),
            songs => Some(format!("Starts with {} songs", songs.len()).into()),
        };
        let owner = cx.entity().downgrade();
        cx.new(|cx| {
            NameForm::new(
                "",
                "Create",
                scope,
                move |name, form, window, cx| {
                    owner
                        .update(cx, |this, cx| {
                            this.create_playlist(name, songs.clone(), form, window, cx);
                        })
                        .ok();
                },
                window,
                cx,
            )
        })
    }

    /// Creates the playlist while its dialog waits: unlike other edits it has
    /// no row to show until the server gives it an id.
    fn create_playlist(
        &mut self,
        name: String,
        songs: Arc<[Song]>,
        form: Entity<NameForm>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A list on its way would not have the new playlist.
        self.edits += 1;
        let client = self.session.client().clone();
        self._create_task = cx.spawn_in(window, async move |this, cx| {
            let ids: Vec<SongId> = songs.iter().map(|song| song.id().clone()).collect();
            let result = {
                let name = name.clone();
                cx.background_spawn(async move { client.create_playlist(&name, &ids).await })
                    .await
            };
            this.update_in(cx, |this, window, cx| {
                this.edits += 1;
                this.refresh(window, cx);
                match result {
                    Ok(created) => {
                        window.close_dialog(cx);
                        this.playlist_created(created, &name, songs.len(), window, cx);
                    }
                    Err(error) => form.update(cx, |form, cx| {
                        form.set_failed(session::describe(&error).into(), cx);
                    }),
                }
            })
            .ok();
        });
    }

    fn playlist_created(
        &mut self,
        created: PlaylistWithSongs,
        name: &str,
        song_count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = created.playlist().id().clone();
        // Without a list, the one on its way has the new playlist.
        if let Some(mut playlists) = self.loaded_list().map(|list| list.to_vec()) {
            playlists.push(created.playlist().clone());
            self.set_list(playlists, window, cx);
        }
        if song_count == 0 {
            // It may be hidden by the filter, which it should not stay under.
            self.filter
                .update(cx, |filter, cx| filter.set_value("", window, cx));
            self.apply_filter(cx);
            self.selected = None;
            self.select(id, window, cx);
            self.list_focus.focus(window, cx);
            cx.emit(PlaylistsEvent::CreatedEmpty);
        } else {
            let count = if song_count == 1 {
                "1 song".to_owned()
            } else {
                format!("{song_count} songs")
            };
            window.push_notification(
                Notification::new().message(format!("Created “{name}” with {count}")),
                cx,
            );
        }
    }

    pub(super) fn open_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_editable() {
            return;
        }
        let Some(selected) = self.selected_playlist() else {
            return;
        };
        let playlist = selected.id().clone();
        let name = selected.name().to_owned();
        let owner = cx.entity().downgrade();
        let form = cx.new(|cx| {
            NameForm::new(
                &name,
                "Rename",
                None,
                move |name, _, window, cx| {
                    window.close_dialog(cx);
                    owner
                        .update(cx, |this, cx| {
                            this.rename_playlist(playlist.clone(), name, window, cx);
                        })
                        .ok();
                },
                window,
                cx,
            )
        });
        open_name_dialog("Rename Playlist", form, window, cx);
    }

    pub(super) fn rename_playlist(
        &mut self,
        playlist: PlaylistId,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(loaded) = self
            .loaded_detail_mut()
            .filter(|loaded| loaded.playlist.id() == &playlist)
        {
            loaded.playlist = loaded.playlist.clone().with_name(&name);
        }
        let renamed = self.loaded_list().map(|list| {
            list.iter()
                .map(|listed| {
                    if listed.id() == &playlist {
                        listed.clone().with_name(&name)
                    } else {
                        listed.clone()
                    }
                })
                .collect::<Vec<_>>()
        });
        if let Some(renamed) = renamed {
            self.set_list(renamed, window, cx);
        }
        self.write(Write::Rename { playlist, name }, window, cx);
    }

    pub(super) fn open_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_editable() {
            return;
        }
        let Some(selected) = self.selected_playlist() else {
            return;
        };
        let playlist = selected.id().clone();
        let title = SharedString::from(format!("Delete “{}”?", selected.name()));
        let owner = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let playlist = playlist.clone();
            let owner = owner.clone();
            alert
                .title(title.clone())
                .description("The songs stay in your library. This can't be undone.")
                .ok_text("Delete")
                .ok_variant(ButtonVariant::Danger)
                .show_cancel(true)
                .on_ok(move |_, window, cx| {
                    owner
                        .update(cx, |this, cx| {
                            this.delete_playlist(playlist.clone(), window, cx);
                        })
                        .ok();
                    true
                })
        });
    }

    /// Takes the playlist off the list, selecting the one that takes its place.
    pub(super) fn delete_playlist(
        &mut self,
        playlist: PlaylistId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let position = self.selected_position();
        let remaining = self.loaded_list().map(|list| {
            list.iter()
                .filter(|listed| listed.id() != &playlist)
                .cloned()
                .collect::<Vec<_>>()
        });
        if self.selected.as_ref() == Some(&playlist) {
            self.clear_selection();
        }
        if let Some(remaining) = remaining {
            self.set_list(remaining, window, cx);
        }
        if let Some(next) = position
            .and_then(|position| {
                self.visible_playlist(position)
                    .or_else(|| self.visible_playlist(position.checked_sub(1)?))
            })
            .map(|listed| listed.id().clone())
        {
            self.select(next, window, cx);
        }
        self.write(Write::Delete { playlist }, window, cx);
        self.list_focus.focus(window, cx);
    }

    /// Opens the choice of playlist for `songs`, taking focus until it closes.
    pub fn open_chooser(
        &mut self,
        songs: Arc<[Song]>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if songs.is_empty() {
            return;
        }
        let previous_focus = window.focused(cx);
        let command = actions::add_songs_command(songs);
        let menu = PopupMenu::build(window, cx, move |menu, _, cx| {
            actions::playlist_choices(menu, &command, cx)
        });
        menu.focus_handle(cx).focus(window, cx);
        let dismissal = cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, window, cx| {
            this.close_chooser(window, cx);
        });
        self.chooser = Some(Chooser {
            menu,
            previous_focus,
            _dismissal: dismissal,
        });
        cx.emit(PlaylistsEvent::ChooserChanged);
    }

    /// Returns focus to where it was, unless it has moved on from the menu.
    fn close_chooser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(chooser) = self.chooser.take() else {
            return;
        };
        if chooser.menu.focus_handle(cx).contains_focused(window, cx)
            && let Some(previous) = chooser.previous_focus
        {
            previous.focus(window, cx);
        }
        cx.emit(PlaylistsEvent::ChooserChanged);
    }

    /// The open choice of playlist, to draw over whichever screen is showing.
    pub fn render_chooser(&self, window: &Window) -> Option<AnyElement> {
        let chooser = self.chooser.as_ref()?;
        let viewport = window.viewport_size();
        Some(
            deferred(
                anchored()
                    .position(gpui_kit::point(viewport.width / 2., viewport.height / 4.))
                    .anchor(gpui_kit::Anchor::TopCenter)
                    .snap_to_window()
                    .child(chooser.menu.clone()),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}

pub(super) fn open_name_dialog(
    title: &'static str,
    form: Entity<NameForm>,
    window: &mut Window,
    cx: &mut App,
) {
    let body = form.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let submitted = body.clone();
        let submitting = body.clone();
        dialog
            .title(title)
            .child(body.clone())
            // Enter reaches the dialog from the input. The form decides whether
            // that closes it: a failure has to stay on show.
            .on_ok(move |_, window, cx| {
                submitted.update(cx, |form, cx| form.submit(window, cx));
                false
            })
            // What is being saved goes through either way, so it can't be
            // cancelled.
            .on_cancel(move |_, _, cx| !submitting.read(cx).is_submitting())
    });
    form.update(cx, |form, cx| form.focus_input(window, cx));
}
