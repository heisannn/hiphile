use std::sync::Arc;

use gpui_kit::component::{
    WindowExt as _,
    button::ButtonVariant,
    menu::PopupMenu,
    notification::{Notification, NotificationType},
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, DismissEvent, Entity, FocusHandle, Focusable as _,
    IntoElement as _, ParentElement as _, SharedString, Subscription, Window, anchored, deferred,
};
use hiphile_core::playlists::{CreatedPlaylist, new_playlist_scope};
use hiphile_core::selection;
use subsonic::{PlaylistId, Song};

use super::{Playlists, PlaylistsEvent};
use crate::name_form::NameForm;

/// A menu of playlists, opened from the keyboard. It stands apart from any row
/// and from the screen, as the screen on show may be another feature's.
pub(super) struct Chooser {
    pub(super) menu: Entity<PopupMenu>,
    previous_focus: Option<FocusHandle>,
    _dismissal: Subscription,
}

impl Playlists {
    pub fn open_new_playlist(
        &mut self,
        songs: Arc<[Song]>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Its shortcut still works over a dialog
        if window.has_active_dialog(cx) {
            return;
        }
        // Before the dialog, which returns focus to where it was opened from
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
        let scope = new_playlist_scope(&songs).map(SharedString::from);
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
        let created = self.playlists.create(&name, &songs);
        self._create_task = cx.spawn_in(window, async move |this, cx| {
            let created = created.await;
            this.update_in(cx, |this, window, cx| match created {
                Ok(created) => {
                    window.close_dialog(cx);
                    this.playlist_created(created, songs.is_empty(), window, cx);
                }
                Err(error) => form.update(cx, |form, cx| form.set_failed(error.into(), cx)),
            })
            .ok();
        });
    }

    fn playlist_created(
        &mut self,
        created: CreatedPlaylist,
        is_empty: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync(window, cx);
        if is_empty {
            // It may be hidden by the filter, which it should not stay under
            self.filter
                .update(cx, |filter, cx| filter.set_value("", window, cx));
            self.apply_filter(window, cx);
            self.selected = None;
            self.select(created.id().clone(), window, cx);
            self.list_focus.focus(window, cx);
            cx.emit(PlaylistsEvent::CreatedEmpty);
        } else {
            window.push_notification(
                Notification::new().message(created.message().to_owned()),
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
                move |name, form, window, cx| {
                    let renamed = owner.update(cx, |this, cx| {
                        this.rename_playlist(playlist.clone(), &name, window, cx)
                    });
                    match renamed {
                        Ok(Ok(())) => window.close_dialog(cx),
                        Ok(Err(error)) => {
                            // The form is being updated until this returns
                            window.defer(cx, move |_, cx| {
                                form.update(cx, |form, cx| form.set_failed(error.into(), cx));
                            });
                        }
                        Err(_) => {}
                    }
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
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.playlists.rename(playlist, name)?;
        self.sync(window, cx);
        Ok(())
    }

    pub(super) fn open_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(playlist) = self.selected.clone() else {
            return;
        };
        let Some(confirmation) = self.shown.delete_confirmation(&playlist) else {
            return;
        };
        let title = SharedString::from(confirmation.title().to_owned());
        let message = SharedString::from(confirmation.message().to_owned());
        let owner = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let playlist = playlist.clone();
            let owner = owner.clone();
            alert
                .title(title.clone())
                .description(message.clone())
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
        if let Err(error) = self.playlists.delete(playlist.clone()) {
            window.push_notification((NotificationType::Error, error), cx);
            return;
        }
        self.sync(window, cx);
        let next = position
            .and_then(|position| selection::after_removal(position, self.shown.visible().len()))
            .and_then(|position| self.visible_playlist(position))
            .map(|listed| listed.id().clone());
        match next {
            Some(next) => self.select(next, window, cx),
            None if self.selected.as_ref() == Some(&playlist) => self.clear_selection(window, cx),
            None => {}
        }
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
