use std::sync::Arc;

use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Sizable as _, h_flex, list::ListItem,
    menu::ContextMenuExt as _,
};
use gpui_kit::{
    App, ElementId, FocusHandle, IntoElement, MouseButton, ParentElement as _, RenderOnce,
    SharedString, Styled as _, WeakEntity, Window, div, prelude::FluentBuilder as _,
};
use subsonic::{Song, StarTarget};
use ui::{
    PlayState, PlayStateIcon, format_duration, skippable_song, song_artist_link, song_row_label,
};

use super::Library;
use crate::{AddToQueue, Open, PlayNext};

#[derive(IntoElement)]
pub(super) struct SongRow {
    pub(super) song: Song,
    pub(super) ix: usize,
    /// Whether the row names the song's artist.
    pub(super) is_by_another_artist: bool,
    pub(super) state: Option<PlayState>,
    pub(super) is_selected: bool,
    pub(super) is_focused: bool,
    pub(super) owner: WeakEntity<Library>,
    pub(super) songs_focus: FocusHandle,
}

impl RenderOnce for SongRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            song,
            ix,
            is_by_another_artist,
            state,
            is_selected,
            is_focused,
            owner,
            songs_focus,
        } = self;
        let muted_foreground = cx.theme().muted_foreground;
        let title = SharedString::from(song.title().to_owned());
        let artist = song_artist_link(&song)
            .filter(|_| is_by_another_artist)
            .map(|artist| artist.small());
        let has_artist = song.artist_id().is_some();
        let number = song
            .track()
            .map(|track| track.to_string())
            .unwrap_or_default();
        let duration = song.duration().map(format_duration).unwrap_or_default();
        let on_click = owner.clone();
        ListItem::new(ElementId::Name(format!("song-{}", song.id()).into()))
            .selected(is_selected)
            .secondary_selected(is_focused && is_selected)
            .h_10()
            .px_6()
            .w_full()
            .accessibility_label(song_row_label(&song, &title, state, cx))
            .child(skippable_song(
                &song,
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_4()
                    .text_sm()
                    .child(
                        h_flex()
                            .w_6()
                            .flex_none()
                            .justify_end()
                            .text_color(muted_foreground)
                            .map(|this| match state {
                                Some(state) => this.child(PlayStateIcon::new(state)),
                                None => this.child(number),
                            }),
                    )
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(div().truncate().child(title))
                            .children(artist),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_color(muted_foreground)
                            .child(duration),
                    ),
                cx,
            ))
            .on_click(move |event, window, cx| {
                on_click
                    .update(cx, |this, cx| {
                        this.songs_focus.focus(window, cx);
                        this.select_song(ix, cx);
                        if event.click_count() >= 2 {
                            this.play_from(ix, window, cx);
                        }
                    })
                    .ok();
            })
            // The menu acts on the selection, so the row under the pointer
            // becomes it first.
            .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                owner
                    .update(cx, |this, cx| {
                        this.songs_focus.focus(window, cx);
                        this.select_song(ix, cx);
                    })
                    .ok();
            })
            .context_menu(move |menu, window, cx| {
                let menu = menu.action_context(songs_focus.clone());
                let menu = actions::queue_menu_items(menu, &song, Open, PlayNext, AddToQueue, cx)
                    .separator();
                let menu = actions::add_to_playlist_submenu(
                    menu,
                    actions::add_songs_command(Arc::from([song.clone()])),
                    window,
                    cx,
                );
                actions::favorite_menu_item(menu, &StarTarget::Song(song.id().clone()), cx)
                    .when(has_artist, |menu| {
                        menu.menu("Go to Artist", Box::new(actions::GoToArtist))
                    })
            })
    }
}
