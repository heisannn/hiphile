use std::cell::Cell;
use std::rc::Rc;

use hiphile_core::alerts::{FAVORITES_FAILED, SYNC_FAILED};

use super::*;
use crate::testing::{find, has_text, press, start};

#[test]
fn a_pane_that_loads_says_what_it_waits_for() {
    let _ui = start(|| pane_loading(ALBUMS.loading()));

    assert!(find(Role::ProgressBar, "Loading Albums").is_some());
}

#[test]
fn a_failure_says_what_failed_and_why_and_tries_again() {
    let tries = Rc::new(Cell::new(0));
    let counted = tries.clone();
    let mut ui = start(move || {
        load_failure(
            ALBUMS.load_failed(),
            "The server didn't answer.",
            move || counted.set(counted.get() + 1),
        )
    });

    let heading = find(Role::Heading, "Couldn't load albums").expect("the heading");
    assert_eq!(heading.level, Some(2));
    assert!(has_text("The server didn't answer."));
    press(&mut ui, Role::Button, "Retry");
    assert_eq!(tries.get(), 1);
}

#[test]
fn an_empty_pane_says_what_is_missing_and_offers_what_to_do() {
    let _ui = start(|| {
        empty_state(
            &PLAYLISTS.none(),
            Some("Playlists you make show here."),
            Some(styled_button("New Playlist", ButtonStyle::tonal())),
        )
    });

    assert!(has_text("No playlists"));
    assert!(has_text("Playlists you make show here."));
    assert!(find(Role::Button, "New Playlist").is_some());
}

#[test]
fn no_matches_says_the_filter_and_has_nothing_to_press() {
    let query = signal(" jazz ".to_owned());
    let mut ui = start(move || no_matches(move || query.get(), None));

    assert!(has_text("No matches for “jazz”"));
    assert!(crate::testing::nodes(Role::Button).is_empty());
    query.set("jazz piano".to_owned());
    ui.update();
    assert!(has_text("No matches for “jazz piano”"));
}

#[test]
fn the_words_of_each_state_are_the_same_for_every_subject() {
    assert_eq!(SONGS.loading(), "Loading Songs");
    assert_eq!(SONGS.updating(), "Updating Songs");
    assert_eq!(SONGS.load_failed(), "Couldn't load songs");
    assert_eq!(ALBUM.load_failed(), "Couldn't load this album");
    assert_eq!(SONGS.refresh_failed(), "Couldn't refresh songs");
    assert_eq!(PLAYLISTS.none(), "No playlists");
    assert_eq!(ALBUMS.none_on_server(), "No albums on this server");
}

#[test]
fn a_pane_and_the_alert_for_the_same_failure_say_it_alike() {
    assert_eq!(LIBRARY.refresh_failed(), SYNC_FAILED);
    assert_eq!(FAVORITES.load_failed(), FAVORITES_FAILED);
}
