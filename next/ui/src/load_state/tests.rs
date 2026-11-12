use std::cell::Cell;
use std::rc::Rc;

use super::*;
use crate::testing::{find, has_text, press, start};

#[test]
fn a_pane_that_loads_says_what_it_waits_for() {
    let _ui = start(|| pane_loading("Loading Albums"));

    assert!(find(Role::ProgressBar, "Loading Albums").is_some());
}

#[test]
fn a_failure_says_what_failed_and_why_and_tries_again() {
    let tries = Rc::new(Cell::new(0));
    let counted = tries.clone();
    let mut ui = start(move || {
        load_failure(
            "Albums didn't load",
            "The server didn't answer.",
            move || counted.set(counted.get() + 1),
        )
    });

    let heading = find(Role::Heading, "Albums didn't load").expect("the heading");
    assert_eq!(heading.level, Some(2));
    assert!(has_text("The server didn't answer."));
    press(&mut ui, Role::Button, "Retry");
    assert_eq!(tries.get(), 1);
}

#[test]
fn an_empty_pane_says_what_is_missing_and_offers_what_to_do() {
    let _ui = start(|| {
        empty_state(
            "No playlists",
            Some("Playlists you make show here."),
            Some(styled_button("New Playlist", ButtonStyle::tonal())),
        )
    });

    assert!(has_text("No playlists"));
    assert!(has_text("Playlists you make show here."));
    assert!(find(Role::Button, "New Playlist").is_some());
}

#[test]
fn no_matches_has_nothing_to_press() {
    let _ui = start(|| no_matches("No albums match", None));

    assert!(has_text("No albums match"));
    assert!(crate::testing::nodes(Role::Button).is_empty());
}
