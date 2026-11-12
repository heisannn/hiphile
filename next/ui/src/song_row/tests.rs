use std::sync::Arc;

use anti_webui::core::tree::{NodeKind, with_doc};
use anti_webui_md3e::icons;
use serde_json::json;

use super::*;
use crate::context_menu::ContextMenu;
use crate::link::song_line;
use crate::row_list::{RowList, row_list};
use crate::testing::{
    NOWHERE, click_text, has_text, recording_commands, screen_context, signed_in_core, start,
};
use crate::{Command, Covers};
use hiphile_core::favorites::Favorite;
use hiphile_core::navigation::Location;

fn song(id: &str, title: &str, seconds: u64) -> Song {
    serde_json::from_value(json!({
        "id": id,
        "title": title,
        "artist": "Miles Davis",
        "artistId": "ar1",
        "album": "Kind of Blue",
        "albumId": "al1",
        "duration": seconds,
    }))
    .unwrap()
}

/// Where the text `shown` starts and ends across the window.
fn span_of(shown: &str) -> (i32, i32) {
    with_doc(|doc| {
        let id = doc
            .nodes
            .iter()
            .find_map(|(id, node)| match &node.kind {
                NodeKind::Text(text) if text.text == shown => Some(id),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no text {shown:?}"));
        let rect = doc.window_rect(id).expect("a place");
        (rect.x, rect.x + rect.width)
    })
}

fn row(content: Div) -> Div {
    div().flex().flex_row().w(640).h(48).child(content)
}

#[test]
fn a_song_shows_its_title_number_and_how_long_it_lasts() {
    let _ui = start(|| {
        let song = song("s1", "Blue in Green", 337);
        row(song_row(
            SongRow::new(&song, song_words(&song, None), || None).number(Some(3)),
        ))
    });

    assert!(has_text("Blue in Green"));
    assert!(has_text("3"));
    assert!(has_text("5:37"));
}

#[test]
fn its_artist_and_album_go_there() {
    let (commands, sent) = recording_commands();
    let mut ui = start(move || {
        let song = song("s1", "Blue in Green", 337);
        let line = song_line(&song, &commands);
        row(song_row(SongRow::new(
            &song,
            song_words(&song, Some(line)),
            || None,
        )))
    });

    click_text(&mut ui, "Miles Davis");
    click_text(&mut ui, "Kind of Blue");

    assert_eq!(
        *sent.borrow(),
        [
            Command::Show(Location::Artists(Some(subsonic::ArtistId::new("ar1")))),
            Command::Show(Location::Albums(Some(subsonic::AlbumId::new("al1")))),
        ]
    );
}

#[test]
fn the_song_that_plays_says_so() {
    let state = signal(None);
    let mut ui = start(move || {
        let song = song("s1", "Blue in Green", 337);
        row(song_row(SongRow::new(
            &song,
            song_words(&song, None),
            move || state.get(),
        )))
    });

    let playing = icons::GRAPHIC_EQ.ch.to_string();
    assert!(!has_text(&playing));
    state.set(Some(PlayState::Playing));
    ui.update();
    assert!(has_text(&playing));
}

#[test]
fn the_lanes_of_every_row_and_of_the_headings_line_up() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let (commands, _) = recording_commands();
    let _ui = start(move || {
        let cx = screen_context(&hiphile, commands);
        let menu = ContextMenu::new(Covers::new());
        let songs: Arc<[Song]> = Arc::from([
            song("s1", "Short", 59),
            song("s2", "Long", 3_723),
            song("s3", "Playing", 600),
        ]);
        let rows = RowList::new(
            "Songs",
            move || songs.clone(),
            |song: &Song| song.id().clone(),
            move |song, _| {
                let state = (song.title() == "Playing").then_some(PlayState::Paused);
                let favorite = Favorite::Song(song.clone());
                let buttons = RowButtons::new(favorite, &cx.commands, cx.known, menu, Vec::new);
                song_row(SongRow::new(song, song_words(song, None), move || state).buttons(buttons))
            },
        );
        div()
            .flex()
            .flex_col()
            .w(640)
            .h(400)
            .child(song_headings(div().child(text("Title")), div().child(text("Time"))).h(32))
            .child(row_list(rows).flex_1())
    });

    let (title, time) = (span_of("Title"), span_of("Time"));
    for duration in ["0:59", "1:02:03", "10:00"] {
        assert_eq!(span_of(duration).1, time.1, "{duration}");
    }
    for words in ["Short", "Long", "Playing"] {
        assert_eq!(span_of(words).0, title.0, "{words}");
    }
}

#[test]
fn numbers_of_any_length_end_in_line() {
    let _ui = start(|| {
        let (seven, twelve) = (song("s1", "Seven", 60), song("s2", "Twelve", 60));
        div()
            .flex()
            .flex_col()
            .child(row(song_row(
                SongRow::new(&seven, song_words(&seven, None), || None).number(Some(7)),
            )))
            .child(row(song_row(
                SongRow::new(&twelve, song_words(&twelve, None), || None).number(Some(12)),
            )))
    });

    assert_eq!(span_of("7").1, span_of("12").1);
    assert_eq!(span_of("Seven").0, span_of("Twelve").0);
}
