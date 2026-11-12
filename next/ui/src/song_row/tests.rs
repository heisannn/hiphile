use hiphile_core::navigation::Location;
use serde_json::json;
use subsonic::{AlbumId, ArtistId};

use super::*;
use crate::Command;
use crate::testing::{click_text, has_text, recording_commands, start};

fn song() -> Song {
    serde_json::from_value(json!({
        "id": "s1",
        "title": "Blue in Green",
        "artist": "Miles Davis",
        "artistId": "ar1",
        "album": "Kind of Blue",
        "albumId": "al1",
        "duration": 337,
    }))
    .unwrap()
}

#[test]
fn a_song_shows_its_title_and_how_long_it_lasts() {
    let (commands, _) = recording_commands();
    let _ui = start(move || song_with_cover(&song(), div(), || None, &commands));

    assert!(has_text("Blue in Green"));
    assert!(has_text("5:37"));
}

#[test]
fn its_artist_and_album_go_there() {
    let (commands, sent) = recording_commands();
    let mut ui = start(move || song_with_cover(&song(), div(), || None, &commands));

    click_text(&mut ui, "Miles Davis");
    click_text(&mut ui, "Kind of Blue");

    assert_eq!(
        *sent.borrow(),
        [
            Command::Show(Location::Artists(Some(ArtistId::new("ar1")))),
            Command::Show(Location::Albums(Some(AlbumId::new("al1")))),
        ]
    );
}

#[test]
fn the_song_that_plays_says_so() {
    let (commands, _) = recording_commands();
    let state = signal(None);
    let mut ui = start(move || song_with_cover(&song(), div(), move || state.get(), &commands));

    let playing = anti_webui_md3e::icons::GRAPHIC_EQ.ch.to_string();
    assert!(!has_text(&playing));
    state.set(Some(PlayState::Playing));
    ui.update();
    assert!(has_text(&playing));
}
