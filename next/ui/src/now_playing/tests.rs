use serde_json::json;

use super::*;
use crate::testing::{has_text, nodes, start};

fn song(title: &str) -> Song {
    serde_json::from_value(json!({ "id": title, "title": title })).unwrap()
}

#[test]
fn only_the_song_the_player_is_on_has_a_state() {
    let now = NowPlaying::new(Some(SongId::new("a")), false).unwrap();
    assert_eq!(now.state_of(&SongId::new("a")), Some(PlayState::Paused));
    assert_eq!(now.state_of(&SongId::new("b")), None);
    assert_eq!(NowPlaying::new(None, true), None);
}

#[test]
fn a_rows_label_says_what_its_icon_and_its_look_show() {
    assert_eq!(song_row_label("One", None, false), "One");
    assert_eq!(
        song_row_label("One", Some(PlayState::Playing), false),
        "One, playing"
    );
    assert_eq!(
        song_row_label("One", Some(PlayState::Paused), true),
        "One, paused, skipped"
    );
}

#[test]
fn instrumentals_are_skipped_only_while_the_player_skips_them() {
    assert!(is_skipped(&song("One (Instrumental)"), true));
    assert!(!is_skipped(&song("One (Instrumental)"), false));
    assert!(!is_skipped(&song("One"), true));
}

#[test]
fn the_icon_follows_the_player() {
    let state = signal(None);
    let mut ui = start(move || play_state_icon(move || state.get()));

    assert!(!has_text(&icons::GRAPHIC_EQ.ch.to_string()));
    state.set(Some(PlayState::Playing));
    ui.update();
    assert!(has_text(&icons::GRAPHIC_EQ.ch.to_string()));
    state.set(Some(PlayState::Paused));
    ui.update();
    assert!(has_text(&icons::PAUSE.ch.to_string()));
    assert!(!has_text(&icons::GRAPHIC_EQ.ch.to_string()));
}

#[test]
fn the_icon_says_nothing_to_assistive_technology() {
    let _ui = start(|| play_state_icon(|| Some(PlayState::Playing)));

    assert!(nodes(Role::Image).is_empty());
}
