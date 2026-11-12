use std::time::Instant;

use anti_webui::core::Ui;
use anti_webui::core::event::{Modifiers, NamedKey, PointerButton, WindowEvent};
use anti_webui::core::geometry::PointF;
use next_host::follow;
use next_ui::testing::{NOWHERE, TestCore, find, focus, key, signed_in_core, start, wait_until};
use serde_json::json;
use subsonic::Song;

use super::*;

fn song(id: &str) -> Song {
    serde_json::from_value(json!({ "id": id, "title": id, "duration": 200 })).unwrap()
}

fn shown_line() -> (TestCore, Ui, playback::Player) {
    let (core, hiphile) = signed_in_core(NOWHERE);
    let player = hiphile.workspace().unwrap().player().clone();
    player.fill_queue(&[song("a"), song("b")], 0);
    let shown = player.clone();
    let ui = start(move || {
        let snapshot = follow(shown.subscribe());
        div()
            .w(600.0_f32)
            .child(position_line(shown, snapshot, || false))
    });
    (core, ui, player)
}

/// Paused after it started, which it can be moved in.
fn started(ui: &mut Ui, player: &playback::Player) {
    player.pretend_playing(false);
    wait_until(ui, || !find(Role::Slider, "Position").unwrap().disabled);
}

fn told() -> String {
    let line = find(Role::Slider, "Position").expect("the seek bar");
    line.value_text.unwrap_or_default()
}

/// Lets the timers that are due by then run.
fn after(ui: &mut Ui, wait: Duration) {
    ui.tick(Instant::now() + wait);
    ui.update();
}

fn pointer(ui: &mut Ui, x: f32, y: f32, pressed: Option<bool>) {
    let position = PointF::new(x, y);
    ui.handle_event(WindowEvent::PointerMoved {
        position,
        modifiers: Modifiers::default(),
    });
    if let Some(pressed) = pressed {
        ui.handle_event(WindowEvent::PointerButton {
            position,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::default(),
            time: 0,
        });
    }
    ui.update();
}

/// Where along the seek bar `fraction` is, in the window.
fn along(fraction: f32) -> (f32, f32) {
    let bounds = find(Role::Slider, "Position").unwrap().bounds;
    let inset = HANDLE / 2.0;
    let x = bounds.x as f32 + inset + (bounds.width as f32 - 2.0 * inset) * fraction;
    (x, bounds.y as f32 + bounds.height as f32 / 2.0)
}

#[test]
fn the_position_is_read_again_while_there_is_a_song() {
    let (_core, mut ui, player) = shown_line();
    player.place_at(Duration::from_secs(50));
    after(&mut ui, REFRESH * 2);
    assert_eq!(told(), "0:50 of 3:20");

    player.place_at(Duration::from_secs(80));
    after(&mut ui, REFRESH * 2);
    assert_eq!(told(), "1:20 of 3:20");
}

#[test]
fn until_the_song_has_started_it_cannot_be_moved() {
    let (_core, _ui, _player) = shown_line();

    assert!(find(Role::Slider, "Position").unwrap().disabled);
}

#[test]
fn a_key_seeks_at_once() {
    let (_core, mut ui, player) = shown_line();
    started(&mut ui, &player);

    focus(&mut ui, Role::Slider, "Position");
    key(&mut ui, NamedKey::ArrowRight);
    assert_eq!(told(), "0:10 of 3:20");
}

#[test]
fn a_drag_shows_where_it_would_go_and_seeks_when_let_go() {
    let (_core, mut ui, player) = shown_line();
    started(&mut ui, &player);

    let (x, y) = along(0.25);
    pointer(&mut ui, x, y, Some(true));
    let (x, y) = along(0.5);
    pointer(&mut ui, x, y, None);
    // What the player says the position is does not take the line back
    after(&mut ui, REFRESH * 2);
    assert_eq!(told(), "1:40 of 3:20");

    pointer(&mut ui, x, y, Some(false));
    assert_eq!(told(), "1:40 of 3:20");
    // Then it follows the player again, whose engine has nothing loaded to move
    after(&mut ui, REFRESH * 2);
    let now = format_duration(player.position());
    assert_eq!(told(), format!("{now} of 3:20"));
}

#[test]
fn another_song_ends_a_drag_without_seeking() {
    let (_core, mut ui, player) = shown_line();
    started(&mut ui, &player);
    let (x, y) = along(0.5);
    pointer(&mut ui, x, y, Some(true));
    assert_eq!(told(), "1:40 of 3:20");

    player.fill_queue(&[song("a"), song("b")], 1);
    wait_until(&mut ui, || told() == "0:00 of 3:20");
    pointer(&mut ui, x, y, Some(false));
    assert_eq!(told(), "0:00 of 3:20");
}

#[test]
fn while_the_song_waits_for_its_data_the_time_gives_way_to_a_spinner() {
    let (_core, mut ui, player) = shown_line();
    player.pretend_playing(true);
    player.pretend_buffering(true);

    wait_until(&mut ui, || find(Role::ProgressBar, "Buffering").is_some());
}
