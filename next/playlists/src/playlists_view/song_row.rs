//! A song of the playlist open: its row and its menu.

use anti_webui::prelude::*;
use anti_webui_md3e::components::{MenuItem, menu_item};
use anti_webui_md3e::typography::TextStyle;
use hiphile_core::favorites::Favorite;
use hiphile_core::playlists::{Entry, EntryId};
use next_ui::Command;
use next_ui::context_menu::more_button;
use next_ui::cover::CoverKind;
use next_ui::details::format_duration;
use next_ui::favorite_button::favorite_button;
use next_ui::link::song_line;
use next_ui::menus::song_menu;
use next_ui::now_playing::{play_state_icon, skippable};
use next_ui::row_cover::row_cover;
use next_ui::row_list::ROW_PADDING;
use next_ui::row_text::row_text;
use next_ui::type_scale::muted_text;

use super::Playlists;

/// Room for the number of a song in a long playlist, which the icon of the song playing takes.
const NUMBER_WIDTH: f32 = 32.0;

/// Where the song is in the playlist, or whether it plays, its cover, title, artist and album,
/// how long it lasts, whether it is a favorite and its menu. A song that would be skipped fades.
pub(super) fn song_row(screen: &Playlists, entry: &Entry, position: usize) -> Div {
    let song = entry.song().clone();
    let id = entry.id();
    let (commands, known) = (&screen.cx.commands, screen.cx.known);
    let now_playing = screen.now_playing;
    let playing = song.id().clone();
    let state =
        memo(move || now_playing.with(|now| now.as_ref().and_then(|now| now.state_of(&playing))));
    let number = div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .justify_end()
        .w(NUMBER_WIDTH)
        .child(show(
            move || state.get().is_none(),
            move || muted_text(TextStyle::BODY_MEDIUM, (position + 1).to_string()),
        ))
        .child(show(
            move || state.get().is_some(),
            move || play_state_icon(move || state.get()),
        ));
    let words = row_text(song.title().to_owned(), None)
        .flex_1()
        .child(song_line(&song, commands));
    let duration = song
        .duration()
        .map(|duration| muted_text(TextStyle::BODY_MEDIUM, format_duration(duration)).flex_none());
    let skipped = song.clone();
    let shown = skippable(
        div()
            .flex()
            .flex_row()
            .flex_1()
            .min_w(0)
            .items_center()
            .gap(12.0_f32)
            .child(number)
            .child(row_cover(&screen.cx.workspace, song.cover_art(), CoverKind::Album).flex_none())
            .child(words)
            .children(duration),
        move || known.is_skipped(&skipped),
    );
    let menu_of = screen.clone();
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(4.0_f32)
        .pl(ROW_PADDING)
        .child(shown.mr(8))
        .child(favorite_button(Favorite::Song(song), commands, known))
        .child(more_button(screen.menu, move || {
            song_menu_of(&menu_of, id).unwrap_or_default()
        }))
}

/// The menu of the entry: what every song has, with moving it and taking it out when the songs
/// can be edited. Play plays the playlist from it.
pub(super) fn song_menu_of(screen: &Playlists, entry: EntryId) -> Option<Vec<Vec<MenuItem>>> {
    let shown = screen.state.snapshot();
    let entries = shown.open_playlist()?.entries();
    let ix = entries.position(entry)?;
    let song = entries.get(ix)?.song().clone();
    let play = Command::Play {
        songs: entries.songs(),
        start: Some(ix),
    };
    let mut groups = song_menu(&screen.cx.commands, screen.cx.known, &song, Some(play));
    if shown.are_songs_editable() {
        let len = entries.len();
        let (up, down, remove) = (screen.clone(), screen.clone(), screen.clone());
        groups.insert(
            1,
            vec![
                menu_item("Move Up")
                    .disabled_when(move || ix == 0)
                    .on_click(move || up.move_entry(entry, -1)),
                menu_item("Move Down")
                    .disabled_when(move || ix + 1 == len)
                    .on_click(move || down.move_entry(entry, 1)),
            ],
        );
        groups.push(vec![
            menu_item("Remove from Playlist").on_click(move || remove.remove(entry)),
        ]);
    }
    Some(groups)
}
