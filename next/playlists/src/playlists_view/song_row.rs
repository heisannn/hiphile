//! A song of the playlist open: its row and its menu.

use anti_webui::prelude::*;
use anti_webui_md3e::components::{MenuItem, menu_item};
use hiphile_core::favorites::Favorite;
use hiphile_core::playlists::{Entry, EntryId};
use next_ui::Command;
use next_ui::cover::CoverKind;
use next_ui::link::song_line;
use next_ui::menus::song_menu;
use next_ui::row_buttons::RowButtons;
use next_ui::row_cover::row_cover;
use next_ui::row_list::RowState;
use next_ui::song_row::{SongRow, song_words};

use super::Playlists;

/// Where the song is in the playlist, its cover, title, artist and album, whether it plays, how
/// long it lasts and its buttons. A song that would be skipped fades.
pub(super) fn song_row(screen: &Playlists, entry: &Entry, row: RowState<EntryId>) -> Div {
    let song = entry.song().clone();
    let id = entry.id();
    let (commands, known) = (&screen.cx.commands, screen.cx.known);
    let now_playing = screen.now_playing;
    let playing = song.id().clone();
    let state = move || now_playing.with(|now| now.as_ref().and_then(|now| now.state_of(&playing)));
    let menu_of = screen.clone();
    let position = u32::try_from(row.position() + 1).ok();
    let buttons = RowButtons::new(
        Favorite::Song(song.clone()),
        commands,
        known,
        screen.menu,
        move || song_menu_of(&menu_of, id).unwrap_or_default(),
    )
    .selected_when(move || row.is_selected());
    let skipped = song.clone();
    let words = song_words(&song, Some(song_line(&song, commands)));
    next_ui::song_row::song_row(
        SongRow::new(&song, words, state)
            .number(position)
            .cover(row_cover(
                &screen.cx.workspace,
                song.cover_art(),
                CoverKind::Album,
            ))
            .skipped_when(move || known.is_skipped(&skipped))
            .buttons(buttons),
    )
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
