use std::cell::Cell;
use std::rc::Rc;

use anti_webui::core::Ui;
use next_ui::testing::{signed_in_core, start, wait_until};
use serde_json::json;

use super::*;
use crate::testing::{CoverServer, png};

fn song(id: &str, cover: Option<&str>) -> Song {
    let mut song = json!({ "id": id, "title": id, "duration": 200 });
    if let Some(cover) = cover {
        song["coverArt"] = json!(cover);
    }
    serde_json::from_value(song).unwrap()
}

fn red(_: u32, _: u32) -> [u8; 4] {
    [200, 40, 40, 255]
}

/// The cover that the view shows, read from outside it.
fn watched(hiphile: Hiphile) -> (Ui, Signal<Option<Cover>>) {
    let seen = Rc::new(Cell::new(None));
    let kept = seen.clone();
    let workspace = hiphile.workspace().unwrap();
    let ui = start(move || {
        kept.set(Some(now_playing_cover(&hiphile, &workspace)));
        div()
    });
    (ui, seen.get().unwrap())
}

#[test]
fn the_cover_of_what_plays_comes_from_the_server() {
    let server = CoverServer::new(png(8, red));
    let (_core, hiphile) = signed_in_core(server.url());
    let player = hiphile.workspace().unwrap().player().clone();
    let (mut ui, cover) = watched(hiphile);
    assert_eq!(cover.get(), None);

    player.fill_queue(&[song("a", Some("al-a"))], 0);
    wait_until(&mut ui, || cover.with(Option::is_some));
    assert_eq!(server.asked(), 1);
}

#[test]
fn a_song_with_the_same_cover_keeps_it_without_asking_again() {
    let server = CoverServer::new(png(8, red));
    let (_core, hiphile) = signed_in_core(server.url());
    let player = hiphile.workspace().unwrap().player().clone();
    let (mut ui, cover) = watched(hiphile);
    let songs = [song("a", Some("al-a")), song("b", Some("al-a"))];
    player.fill_queue(&songs, 0);
    wait_until(&mut ui, || cover.with(Option::is_some));
    let first = cover.get();

    player.fill_queue(&songs, 1);
    ui.update();
    assert_eq!(cover.get(), first);
    assert_eq!(server.asked(), 1);
}

#[test]
fn a_song_without_a_cover_has_none() {
    let server = CoverServer::new(png(8, red));
    let (_core, hiphile) = signed_in_core(server.url());
    let player = hiphile.workspace().unwrap().player().clone();
    let (mut ui, cover) = watched(hiphile);
    player.fill_queue(&[song("a", Some("al-a")), song("b", None)], 0);
    wait_until(&mut ui, || cover.with(Option::is_some));

    player.fill_queue(&[song("a", Some("al-a")), song("b", None)], 1);
    wait_until(&mut ui, || cover.with(Option::is_none));
}
