use gpui_kit::component::{Selectable as _, list::ListItem, menu::ContextMenuExt as _};
use gpui_kit::{
    App, ElementId, FocusHandle, IntoElement, MouseButton, ParentElement as _, RenderOnce,
    SharedString, Styled as _, WeakEntity, Window, prelude::FluentBuilder as _,
};
use subsonic::{Playlist, PlaylistId};
use ui::{RowText, format_duration, format_song_count, join_details};

use super::Playlists;
use crate::{AddToQueue, Delete, Play, PlayNext, Rename};

#[derive(IntoElement)]
pub(super) struct PlaylistRow {
    id: PlaylistId,
    name: SharedString,
    details: SharedString,
    is_editable: bool,
    has_songs: bool,
    is_selected: bool,
    is_focused: bool,
    owner: WeakEntity<Playlists>,
    list_focus: FocusHandle,
}

impl PlaylistRow {
    pub(super) fn new(
        playlist: &Playlist,
        username: &str,
        is_selected: bool,
        is_focused: bool,
        owner: WeakEntity<Playlists>,
        list_focus: FocusHandle,
    ) -> Self {
        Self {
            id: playlist.id().clone(),
            name: playlist.name().to_owned().into(),
            details: playlist_details(playlist, username),
            is_editable: playlist.is_editable_by(username),
            has_songs: playlist.song_count() != Some(0),
            is_selected,
            is_focused,
            owner,
            list_focus,
        }
    }
}

impl RenderOnce for PlaylistRow {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Self {
            id,
            name,
            details,
            is_editable,
            has_songs,
            is_selected,
            is_focused,
            owner,
            list_focus,
        } = self;
        let on_click = owner.clone();
        let click_id = id.clone();
        ListItem::new(ElementId::Name(format!("playlist-{id}").into()))
            .selected(is_selected)
            .secondary_selected(is_focused && is_selected)
            .h_12()
            .px_4()
            .w_full()
            .accessibility_label(name.clone())
            .child(RowText::new(name).details(details))
            .on_click(move |_, window, cx| {
                on_click
                    .update(cx, |this, cx| {
                        this.list_focus.focus(window, cx);
                        this.choose(click_id.clone(), window, cx);
                    })
                    .ok();
            })
            // The menu acts on the selection, so the row under the pointer
            // becomes it first.
            .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                owner
                    .update(cx, |this, cx| {
                        this.list_focus.focus(window, cx);
                        this.select(id.clone(), window, cx);
                    })
                    .ok();
            })
            .context_menu(move |menu, _, _| {
                menu.action_context(list_focus.clone())
                    .menu_with_disabled("Play", Box::new(Play), !has_songs)
                    .menu_with_disabled("Play Next", Box::new(PlayNext), !has_songs)
                    .menu_with_disabled("Add to Queue", Box::new(AddToQueue), !has_songs)
                    .when(is_editable, |menu| {
                        menu.separator()
                            .menu("Rename…", Box::new(Rename))
                            .menu("Delete…", Box::new(Delete))
                    })
            })
    }
}

fn playlist_details(playlist: &Playlist, username: &str) -> SharedString {
    let owner = playlist
        .owner()
        .filter(|_| !playlist.is_owned_by(username))
        .map(str::to_owned);
    let count = playlist
        .song_count()
        .map(|count| format_song_count(count as usize));
    let duration = playlist
        .duration()
        .filter(|_| playlist.song_count() != Some(0))
        .map(format_duration);
    join_details([owner, count, duration])
}
