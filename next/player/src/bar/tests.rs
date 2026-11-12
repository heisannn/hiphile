use std::cell::Cell;
use std::rc::Rc;

use anti_webui::core::Ui;
use anti_webui::core::access::Checked;
use anti_webui::core::event::NamedKey;
use next_ui::testing::{
    NOWHERE, TestCore, find, focus, has_text, key, press, signed_in_core, start, wait_until,
};
use serde_json::json;

use super::*;

const ERROR: &str = "The file couldn't be read. Press Play to try again.";

fn song(id: &str) -> Song {
    serde_json::from_value(json!({
        "id": id,
        "title": format!("Title {id}"),
        "artist": "Artist",
        "album": "Album",
        "duration": 200,
    }))
    .unwrap()
}

struct Bar {
    _core: TestCore,
    ui: Ui,
    player: playback::Player,
    panels: Panels,
}

/// The bar of a player that `setup` has put in the state the test starts from, with that state
/// shown.
fn bar(setup: impl FnOnce(&playback::Player)) -> Bar {
    let (core, hiphile) = signed_in_core(NOWHERE);
    let player = hiphile.workspace().unwrap().player().clone();
    setup(&player);
    let panels = Rc::new(Cell::new(None));
    let kept = panels.clone();
    let shown = player.clone();
    let ui = start(move || {
        let panels = Panels::new();
        kept.set(Some(panels));
        player_bar(shown, signal(None), panels)
    });
    Bar {
        _core: core,
        ui,
        player,
        panels: panels.get().unwrap(),
    }
}

fn with_a_song(player: &playback::Player) {
    player.fill_queue(&[song("a")], 0);
}

fn is_enabled(role: Role, label: &str) -> bool {
    let node = find(role, label).unwrap_or_else(|| panic!("no {role:?} named {label:?}"));
    !node.disabled
}

#[test]
fn with_nothing_to_play_only_what_needs_no_song_can_be_pressed() {
    let _bar = bar(|_| {});

    assert!(!is_enabled(Role::Button, "Previous"));
    assert!(!is_enabled(Role::Button, "Play"));
    assert!(!is_enabled(Role::Button, "Next"));
    assert!(!is_enabled(Role::ToggleButton, "Lyrics"));
    assert!(is_enabled(Role::ToggleButton, "Shuffle"));
    assert!(is_enabled(Role::ToggleButton, "Repeat"));
    assert!(is_enabled(Role::ToggleButton, "Queue"));
    assert!(find(Role::Slider, "Position").is_none());
}

#[test]
fn the_song_shows_with_its_artist_and_album() {
    let _bar = bar(with_a_song);

    assert!(has_text("Title a"));
    assert!(has_text("Artist — Album"));
    assert!(is_enabled(Role::Button, "Play"));
    assert!(find(Role::Slider, "Position").is_some());
}

#[test]
fn while_a_song_plays_the_button_pauses() {
    let _bar = bar(|player| {
        with_a_song(player);
        player.pretend_playing(true);
    });

    assert!(find(Role::Button, "Pause").is_some());
    assert!(find(Role::Button, "Play").is_none());
}

#[test]
fn an_error_takes_the_place_of_the_artist() {
    let _bar = bar(|player| {
        with_a_song(player);
        player.set_error(ERROR);
    });

    assert!(has_text(ERROR));
    assert!(!has_text("Artist — Album"));
}

#[test]
fn shuffle_switches_the_order() {
    let mut bar = bar(|_| {});

    press(&mut bar.ui, Role::ToggleButton, "Shuffle");
    assert!(bar.player.snapshot().is_shuffled());
    wait_until(&mut bar.ui, || {
        find(Role::ToggleButton, "Shuffle").unwrap().checked == Some(Checked::True)
    });
}

#[test]
fn repeat_goes_from_off_to_all_to_one_and_back() {
    let mut bar = bar(|_| {});
    let told = || find(Role::ToggleButton, "Repeat").unwrap().value_text;
    assert_eq!(told().as_deref(), Some("Off"));

    for (mode, shown) in [
        (RepeatMode::All, "All"),
        (RepeatMode::One, "One"),
        (RepeatMode::Off, "Off"),
    ] {
        press(&mut bar.ui, Role::ToggleButton, "Repeat");
        assert_eq!(bar.player.snapshot().repeat(), mode);
        wait_until(&mut bar.ui, || told().as_deref() == Some(shown));
    }
}

#[test]
fn mute_silences_and_then_says_unmute() {
    let mut bar = bar(|_| {});

    press(&mut bar.ui, Role::Button, "Mute");
    assert!(bar.player.snapshot().is_muted());
    wait_until(&mut bar.ui, || find(Role::Button, "Unmute").is_some());

    press(&mut bar.ui, Role::Button, "Unmute");
    assert!(!bar.player.snapshot().is_muted());
}

#[test]
fn the_volume_moves_with_the_keys() {
    let mut bar = bar(|player| player.set_volume(0.5));
    let level = || find(Role::Slider, "Volume").unwrap().value_text;
    assert_eq!(level().as_deref(), Some("50%"));

    focus(&mut bar.ui, Role::Slider, "Volume");
    key(&mut bar.ui, NamedKey::ArrowUp);
    assert_eq!(bar.player.snapshot().volume(), 0.55);
    wait_until(&mut bar.ui, || level().as_deref() == Some("55%"));
}

#[test]
fn the_toggles_open_and_close_their_panels() {
    let mut bar = bar(with_a_song);

    press(&mut bar.ui, Role::ToggleButton, "Queue");
    assert!(bar.panels.queue.get());
    press(&mut bar.ui, Role::ToggleButton, "Lyrics");
    assert!(bar.panels.lyrics.get());
    assert_eq!(
        find(Role::ToggleButton, "Lyrics").unwrap().checked,
        Some(Checked::True)
    );

    press(&mut bar.ui, Role::ToggleButton, "Queue");
    assert!(!bar.panels.queue.get());
}

#[test]
fn the_cover_opens_the_lyrics() {
    let mut bar = bar(with_a_song);

    press(&mut bar.ui, Role::Button, "Lyrics");
    assert!(bar.panels.lyrics.get());
}
