use std::cell::{Cell, RefCell};
use std::rc::Rc;

use anti_webui::core::Ui;
use anti_webui::prelude::*;
use hiphile_core::test_support::FavoritesServer;
use serde_json::json;

use super::*;
use crate::testing::{
    NOWHERE, TestCore, recording_commands, screen_context, signed_in_core, start, wait_until,
};

fn songs() -> Vec<Song> {
    ["s1", "s2"]
        .into_iter()
        .map(|id| serde_json::from_value(json!({ "id": id, "title": id })).unwrap())
        .collect()
}

struct Shown {
    ui: Ui,
    commands: Commands,
    known: Known,
    sent: Rc<RefCell<Vec<Command>>>,
    _core: TestCore,
}

/// What the screens go by on the workspace on `server`, once the favorites are known there.
fn known_on(server: &str) -> Shown {
    let (core, hiphile) = signed_in_core(server);
    let (commands, sent) = recording_commands();
    let kept = Rc::new(Cell::new(None));
    let ui = start({
        let (kept, commands) = (kept.clone(), commands.clone());
        move || {
            kept.set(Some(screen_context(&hiphile, commands).known));
            div()
        }
    });
    Shown {
        ui,
        commands,
        known: kept.get().expect("made with the view"),
        sent,
        _core: core,
    }
}

fn wait_until_known(shown: &mut Shown) {
    let (known, first) = (shown.known, StarTarget::Song(songs()[0].id().clone()));
    wait_until(&mut shown.ui, || known.is_favorite(&first).is_some());
}

fn toggled(shown: &Shown) -> Vec<String> {
    shown
        .sent
        .borrow()
        .iter()
        .map(|command| match command {
            Command::ToggleFavorite(Favorite::Song(song)) => song.id().to_string(),
            other => panic!("{other:?} was sent"),
        })
        .collect()
}

#[test]
fn with_some_not_favorites_those_are_added() {
    let server = FavoritesServer::new(&["s1"]);
    let mut shown = known_on(server.url());
    wait_until_known(&mut shown);

    assert_eq!(are_favorites(shown.known, &songs()), Some(false));
    toggle_favorite_songs(&shown.commands, shown.known, &songs());

    assert_eq!(toggled(&shown), ["s2"]);
}

#[test]
fn with_every_one_a_favorite_all_are_taken_out() {
    let server = FavoritesServer::new(&["s1", "s2"]);
    let mut shown = known_on(server.url());
    wait_until_known(&mut shown);

    assert_eq!(are_favorites(shown.known, &songs()), Some(true));
    toggle_favorite_songs(&shown.commands, shown.known, &songs());

    assert_eq!(toggled(&shown), ["s1", "s2"]);
}

#[test]
fn nothing_goes_until_the_favorites_are_known() {
    let shown = known_on(NOWHERE);

    assert_eq!(are_favorites(shown.known, &songs()), None);
    toggle_favorite_songs(&shown.commands, shown.known, &songs());

    assert!(toggled(&shown).is_empty());
}

#[test]
fn no_songs_are_no_favorites_to_toggle() {
    let server = FavoritesServer::new(&["s1"]);
    let mut shown = known_on(server.url());
    wait_until_known(&mut shown);

    assert_eq!(are_favorites(shown.known, &[]), None);
}
