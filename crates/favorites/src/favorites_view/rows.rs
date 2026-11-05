use actions::GoToArtist;
use gpui_kit::component::{
    Selectable as _, Sizable as _, h_flex, list::ListItem, menu::ContextMenuExt as _,
};
use gpui_kit::{
    App, ClickEvent, ElementId, FocusHandle, IntoElement, MouseButton, ParentElement as _,
    RenderOnce, SharedString, Styled as _, WeakEntity, Window, prelude::FluentBuilder as _,
};
use subsonic::{Album, Artist, Song, StarTarget};
use ui::{
    ArtistLine, CoverArt, PlayState, RowText, format_album_count, format_duration, song_with_cover,
};

use super::{Favorites, add_selection_command};
use crate::{AddToQueue, GoToAlbum, Open, Play, PlayNext};

pub(super) struct RowContext {
    pub(super) target: StarTarget,
    pub(super) is_selected: bool,
    pub(super) is_focused: bool,
    pub(super) owner: WeakEntity<Favorites>,
    pub(super) focus: FocusHandle,
}

impl RowContext {
    fn row(self, id: String) -> (ListItem, StarTarget, FocusHandle) {
        let Self {
            target,
            is_selected,
            is_focused,
            owner,
            focus,
        } = self;
        let on_click = owner.clone();
        let click_target = target.clone();
        let right_target = target.clone();
        let row = ListItem::new(ElementId::Name(id.into()))
            .selected(is_selected)
            .secondary_selected(is_focused && is_selected)
            .h_12()
            .px_4()
            .w_full()
            .on_click(move |event: &ClickEvent, window, cx| {
                on_click
                    .update(cx, |this, cx| {
                        this.focus.focus(window, cx);
                        this.select(click_target.clone(), cx);
                        if event.click_count() >= 2 {
                            this.open_selected(window, cx);
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
                        this.select(right_target.clone(), cx);
                    })
                    .ok();
            });
        (row, target, focus)
    }
}

#[derive(IntoElement)]
pub(super) struct SongRow {
    pub(super) song: Song,
    pub(super) cover: Option<SharedString>,
    pub(super) state: Option<PlayState>,
    pub(super) context: RowContext,
}

impl RenderOnce for SongRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            song,
            cover,
            state,
            context,
        } = self;
        let title = SharedString::from(song.title().to_owned());
        let duration = song.duration().map(format_duration).unwrap_or_default();
        let details = ArtistLine::of_song(&song);
        let has_album = song.album_id().is_some();
        let has_artist = song.artist_id().is_some();
        let (row, target, focus) = context.row(format!("favorite-song-{}", song.id()));
        row.accessibility_label(PlayState::row_label(&title, state))
            .child(song_with_cover(
                cover,
                title,
                details,
                state,
                duration.into(),
                cx,
            ))
            .context_menu(move |menu, window, cx| {
                let menu = menu
                    .action_context(focus.clone())
                    .menu("Play", Box::new(Open))
                    .menu("Play Next", Box::new(PlayNext))
                    .menu("Add to Queue", Box::new(AddToQueue))
                    .separator();
                let menu =
                    actions::add_to_playlist_submenu(menu, add_selection_command, window, cx);
                actions::favorite_menu_item(menu, &target, cx)
                    .when(has_album, |menu| {
                        menu.menu("Go to Album", Box::new(GoToAlbum))
                    })
                    .when(has_artist, |menu| {
                        menu.menu("Go to Artist", Box::new(GoToArtist))
                    })
            })
    }
}

#[derive(IntoElement)]
pub(super) struct AlbumRow {
    pub(super) album: Album,
    pub(super) cover: Option<SharedString>,
    pub(super) context: RowContext,
}

impl RenderOnce for AlbumRow {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Self {
            album,
            cover,
            context,
        } = self;
        let name = SharedString::from(album.name().to_owned());
        let details = ArtistLine::of_album(&album);
        let has_artist = album.artist_id().is_some();
        let (row, target, focus) = context.row(format!("favorite-album-{}", album.id()));
        row.accessibility_label(name.clone())
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_3()
                    .child(CoverArt::new(cover).small())
                    .child(RowText::new(name).detail_line(details)),
            )
            .context_menu(move |menu, window, cx| {
                let menu = menu
                    .action_context(focus.clone())
                    .menu("Play", Box::new(Play))
                    .menu("Play Next", Box::new(PlayNext))
                    .menu("Add to Queue", Box::new(AddToQueue))
                    .separator();
                let menu =
                    actions::add_to_playlist_submenu(menu, add_selection_command, window, cx);
                actions::favorite_menu_item(menu, &target, cx)
                    .menu("Show in Albums", Box::new(Open))
                    .when(has_artist, |menu| {
                        menu.menu("Go to Artist", Box::new(GoToArtist))
                    })
            })
    }
}

#[derive(IntoElement)]
pub(super) struct ArtistRow {
    pub(super) artist: Artist,
    pub(super) cover: Option<SharedString>,
    pub(super) context: RowContext,
}

impl RenderOnce for ArtistRow {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Self {
            artist,
            cover,
            context,
        } = self;
        let name = SharedString::from(artist.name().to_owned());
        // Zero says nothing of an artist the server knows for their songs
        let details = artist
            .album_count()
            .filter(|count| *count > 0)
            .map(|count| SharedString::from(format_album_count(count as usize)));
        let (row, target, focus) = context.row(format!("favorite-artist-{}", artist.id()));
        row.accessibility_label(name.clone())
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_3()
                    .child(CoverArt::artist(cover).small())
                    .child(RowText::new(name).details(details)),
            )
            .context_menu(move |menu, _, cx| {
                let menu = menu
                    .action_context(focus.clone())
                    .menu("Go to Artist", Box::new(Open))
                    .separator();
                actions::favorite_menu_item(menu, &target, cx)
            })
    }
}
