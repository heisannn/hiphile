use std::sync::Arc;

use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Sizable as _, h_flex, list::ListItem,
    menu::ContextMenuExt as _,
};
use gpui_kit::{
    App, ElementId, FocusHandle, IntoElement, MouseButton, ParentElement as _, RenderOnce,
    SharedString, Styled as _, WeakEntity, Window, div, prelude::FluentBuilder as _,
};
use subsonic::Song;
use ui::{
    CoverArt, PlayState, PlayStateIcon, ReorderExt as _, RowText, format_duration, join_details,
};

use super::Playlists;
use crate::entries::EntryId;
use crate::{AddToQueue, MoveDown, MoveUp, Open, PlayNext, Remove, ShowAlbum};

/// A song of the playlist on show, whose commands act on the selection.
#[derive(IntoElement)]
pub(super) struct SongRow {
    pub(super) id: EntryId,
    pub(super) song: Song,
    pub(super) cover: Option<SharedString>,
    pub(super) ix: usize,
    /// How many songs the list holds.
    pub(super) len: usize,
    pub(super) state: Option<PlayState>,
    pub(super) is_editable: bool,
    pub(super) is_selected: bool,
    pub(super) is_focused: bool,
    pub(super) owner: WeakEntity<Playlists>,
    pub(super) songs_focus: FocusHandle,
}

impl RenderOnce for SongRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            id,
            song,
            cover,
            ix,
            len,
            state,
            is_editable,
            is_selected,
            is_focused,
            owner,
            songs_focus,
        } = self;
        let muted_foreground = cx.theme().muted_foreground;
        let title = SharedString::from(song.title().to_owned());
        let details = join_details([song.artist(), song.album()]);
        let duration = song.duration().map(format_duration).unwrap_or_default();
        let has_album = song.album_id().is_some();
        let on_click = owner.clone();
        let on_right_click = owner.clone();
        let row = ListItem::new(ElementId::NamedInteger("playlist-entry".into(), id.get()))
            .selected(is_selected)
            .secondary_selected(is_focused && is_selected)
            .h_12()
            .px_6()
            .w_full()
            .accessibility_label(PlayState::row_label(&title, state))
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_3()
                    .child(
                        // The playing song shows a sound icon in place of its
                        // number.
                        h_flex()
                            .w_6()
                            .flex_none()
                            .justify_end()
                            .text_sm()
                            .text_color(muted_foreground)
                            .map(|this| match state {
                                Some(state) => this.child(PlayStateIcon::new(state)),
                                None => this.child((ix + 1).to_string()),
                            }),
                    )
                    .child(CoverArt::new(cover).small())
                    .child(RowText::new(title.clone()).details(details))
                    .child(
                        div()
                            .flex_none()
                            .text_sm()
                            .text_color(muted_foreground)
                            .child(duration),
                    ),
            )
            .on_click(move |event, window, cx| {
                on_click
                    .update(cx, |this, cx| {
                        this.songs_focus.focus(window, cx);
                        this.select_entry(id, cx);
                        if event.click_count() >= 2
                            && let Some(ix) = this.selected_entry_ix()
                        {
                            this.play_from(ix, window, cx);
                        }
                    })
                    .ok();
            })
            // The menu acts on the selection, so the row under the pointer
            // becomes it first.
            .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                on_right_click
                    .update(cx, |this, cx| {
                        this.songs_focus.focus(window, cx);
                        this.select_entry(id, cx);
                    })
                    .ok();
            });
        row.when(is_editable, |row| {
            row.reorderable(id, ix, title, move |dragged, window, cx| {
                owner
                    .update(cx, |this, cx| this.drop_entry(dragged, id, window, cx))
                    .ok();
            })
        })
        .context_menu(move |menu, window, cx| {
            let menu = menu
                .action_context(songs_focus.clone())
                .menu("Play", Box::new(Open))
                .menu("Play Next", Box::new(PlayNext))
                .menu("Add to Queue", Box::new(AddToQueue))
                .when(is_editable, |menu| {
                    menu.separator()
                        .menu_with_disabled("Move Up", Box::new(MoveUp), ix == 0)
                        .menu_with_disabled("Move Down", Box::new(MoveDown), ix + 1 == len)
                })
                .separator();
            let menu = actions::add_to_playlist_submenu(
                menu,
                actions::add_songs_command(Arc::from([song.clone()])),
                window,
                cx,
            );
            menu.when(has_album, |menu| {
                menu.menu("Show Album", Box::new(ShowAlbum))
            })
            .when(is_editable, |menu| {
                menu.separator()
                    .menu("Remove from Playlist", Box::new(Remove))
            })
        })
    }
}
