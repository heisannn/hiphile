use std::sync::Arc;

use gpui_kit::component::{
    Selectable as _, Sizable as _, list::ListItem, menu::ContextMenuExt as _,
};
use gpui_kit::{
    App, ElementId, FocusHandle, IntoElement, MouseButton, ParentElement as _, RenderOnce,
    SharedString, Styled as _, WeakEntity, Window, div, prelude::FluentBuilder as _,
};
use subsonic::{Song, StarTarget};
use ui::{AlbumLink, PlayState, PlayStateIcon, format_duration, song_artist_link};

use super::{Songs, columns};
use crate::{AddToQueue, GoToAlbum, Open, PlayNext};

#[derive(IntoElement)]
pub(super) struct SongRow {
    pub(super) song: Song,
    pub(super) state: Option<PlayState>,
    pub(super) is_selected: bool,
    pub(super) is_focused: bool,
    pub(super) owner: WeakEntity<Songs>,
    pub(super) focus: FocusHandle,
}

impl RenderOnce for SongRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            song,
            state,
            is_selected,
            is_focused,
            owner,
            focus,
        } = self;
        let id = song.id().clone();
        let title = SharedString::from(song.title().to_owned());
        let album = AlbumLink::of_song(&song).map(|album| album.small());
        let duration = song.duration().map(format_duration).unwrap_or_default();
        let artist = song_artist_link(&song).map(|artist| artist.small());
        let has_album = song.album_id().is_some();
        let has_artist = song.artist_id().is_some();
        let on_click = owner.clone();
        let click_id = id.clone();
        ListItem::new(ElementId::Name(format!("song-{id}").into()))
            .selected(is_selected)
            .secondary_selected(is_focused && is_selected)
            .h_8()
            .px_4()
            .w_full()
            .accessibility_label(PlayState::row_label(&title, state))
            .child(
                columns(
                    state.map(PlayStateIcon::new),
                    div().min_w_0().truncate().child(title),
                    artist,
                    album,
                    div().child(duration),
                    cx,
                )
                .text_sm(),
            )
            .on_click(move |event, window, cx| {
                on_click
                    .update(cx, |this, cx| {
                        this.focus.focus(window, cx);
                        this.select_song(click_id.clone(), cx);
                        if event.click_count() >= 2 {
                            this.play_selected(window, cx);
                        }
                    })
                    .ok();
            })
            // The menu acts on the selection, so the row under the pointer
            // becomes it first.
            .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                owner
                    .update(cx, |this, cx| {
                        this.focus.focus(window, cx);
                        this.select_song(id.clone(), cx);
                    })
                    .ok();
            })
            .context_menu(move |menu, window, cx| {
                let menu = menu
                    .action_context(focus.clone())
                    .menu("Play", Box::new(Open))
                    .menu("Play Next", Box::new(PlayNext))
                    .menu("Add to Queue", Box::new(AddToQueue))
                    .separator();
                let menu = actions::add_to_playlist_submenu(
                    menu,
                    actions::add_songs_command(Arc::from([song.clone()])),
                    window,
                    cx,
                );
                actions::favorite_menu_item(menu, &StarTarget::Song(song.id().clone()), cx)
                    .when(has_album, |menu| {
                        menu.menu("Go to Album", Box::new(GoToAlbum))
                    })
                    .when(has_artist, |menu| {
                        menu.menu("Go to Artist", Box::new(actions::GoToArtist))
                    })
            })
    }
}
