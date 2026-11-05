use gpui_kit::component::{Selectable as _, list::ListItem, menu::ContextMenuExt as _};
use gpui_kit::{
    App, ElementId, FocusHandle, IntoElement, MouseButton, ParentElement as _, RenderOnce,
    SharedString, Styled as _, WeakEntity, Window, prelude::FluentBuilder as _,
};
use subsonic::{Album, AlbumId, StarTarget};
use ui::{ArtistLine, RowText};

use super::{Library, add_album_command};
use crate::{AddToQueue, Play, PlayNext};

/// An album in the album list, whose commands act on the selection.
#[derive(IntoElement)]
pub(super) struct AlbumRow {
    id: AlbumId,
    name: SharedString,
    details: ArtistLine,
    has_artist: bool,
    is_selected: bool,
    is_focused: bool,
    owner: WeakEntity<Library>,
    albums_focus: FocusHandle,
}

impl AlbumRow {
    pub(super) fn new(
        album: &Album,
        is_selected: bool,
        is_focused: bool,
        owner: WeakEntity<Library>,
        albums_focus: FocusHandle,
    ) -> Self {
        Self {
            id: album.id().clone(),
            name: album.name().to_owned().into(),
            details: ArtistLine::of_album(album),
            has_artist: album.artist_id().is_some(),
            is_selected,
            is_focused,
            owner,
            albums_focus,
        }
    }
}

impl RenderOnce for AlbumRow {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Self {
            id,
            name,
            details,
            has_artist,
            is_selected,
            is_focused,
            owner,
            albums_focus,
        } = self;
        let on_click = owner.clone();
        let click_id = id.clone();
        let id_for_menu = id.clone();
        ListItem::new(ElementId::Name(format!("album-{id}").into()))
            .selected(is_selected)
            .secondary_selected(is_focused && is_selected)
            .h_12()
            .px_4()
            .w_full()
            .accessibility_label(name.clone())
            .child(RowText::new(name).detail_line(details))
            .on_click(move |_, window, cx| {
                on_click
                    .update(cx, |this, cx| {
                        this.albums_focus.focus(window, cx);
                        this.choose_album(click_id.clone(), cx);
                    })
                    .ok();
            })
            // The menu acts on the selection, so the row under the pointer
            // becomes it first.
            .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                owner
                    .update(cx, |this, cx| {
                        this.albums_focus.focus(window, cx);
                        this.select_album(id.clone(), cx);
                    })
                    .ok();
            })
            .context_menu(move |menu, window, cx| {
                let menu = menu
                    .action_context(albums_focus.clone())
                    .menu("Play", Box::new(Play))
                    .menu("Play Next", Box::new(PlayNext))
                    .menu("Add to Queue", Box::new(AddToQueue))
                    .separator();
                let menu = actions::add_to_playlist_submenu(menu, add_album_command, window, cx);
                actions::favorite_menu_item(menu, &StarTarget::Album(id_for_menu.clone()), cx)
                    .when(has_artist, |menu| {
                        menu.menu("Go to Artist", Box::new(actions::GoToArtist))
                    })
            })
    }
}
