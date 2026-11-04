use gpui_kit::component::{Selectable as _, list::ListItem, menu::ContextMenuExt as _};
use gpui_kit::{
    App, ElementId, FocusHandle, IntoElement, MouseButton, ParentElement as _, RenderOnce,
    SharedString, Styled as _, WeakEntity, Window,
};
use subsonic::{Artist, ArtistId};
use ui::{RowText, format_album_count};

use super::{Artists, add_selection_command};
use crate::{AddToQueue, Play, PlayNext};

/// An artist in the list, whose commands act on the selection.
#[derive(IntoElement)]
pub(super) struct ArtistRow {
    id: ArtistId,
    name: SharedString,
    details: Option<SharedString>,
    is_selected: bool,
    is_focused: bool,
    owner: WeakEntity<Artists>,
    list_focus: FocusHandle,
}

impl ArtistRow {
    pub(super) fn new(
        artist: &Artist,
        is_selected: bool,
        is_focused: bool,
        owner: WeakEntity<Artists>,
        list_focus: FocusHandle,
    ) -> Self {
        Self {
            id: artist.id().clone(),
            name: artist.name().to_owned().into(),
            details: artist
                .album_count()
                .map(|count| format_album_count(count as usize).into()),
            is_selected,
            is_focused,
            owner,
            list_focus,
        }
    }
}

impl RenderOnce for ArtistRow {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Self {
            id,
            name,
            details,
            is_selected,
            is_focused,
            owner,
            list_focus,
        } = self;
        let on_click = owner.clone();
        let click_id = id.clone();
        ListItem::new(ElementId::Name(format!("artist-row-{id}").into()))
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
                        this.select_artist(click_id.clone(), cx);
                    })
                    .ok();
            })
            // The menu acts on the selection, so the row under the pointer
            // becomes it first.
            .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                owner
                    .update(cx, |this, cx| {
                        this.list_focus.focus(window, cx);
                        this.select_artist(id.clone(), cx);
                    })
                    .ok();
            })
            .context_menu(move |menu, window, cx| {
                let menu = menu
                    .action_context(list_focus.clone())
                    .menu("Play", Box::new(Play))
                    .menu("Play Next", Box::new(PlayNext))
                    .menu("Add to Queue", Box::new(AddToQueue))
                    .separator();
                actions::add_to_playlist_submenu(menu, add_selection_command, window, cx)
            })
    }
}
