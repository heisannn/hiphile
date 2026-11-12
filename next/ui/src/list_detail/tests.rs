use anti_webui::core::Ui;
use anti_webui::core::event::WindowEvent;
use anti_webui::core::geometry::{Scale, SizeI};

use super::*;
use crate::testing::{drag, find, focus, frames, key, start_at};

fn view(split: Split) -> Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(list_detail(split, div(), div()).flex_1())
}

fn start(split: Split, width: i32) -> Ui {
    let mut ui = start_at(move || view(split), (width, 600));
    frames(&mut ui, 16);
    ui
}

fn resize(ui: &mut Ui, width: i32) {
    ui.handle_event(WindowEvent::Resized {
        size: SizeI::new(width, 600),
        scale: Scale::ONE,
    });
    frames(ui, 16);
}

fn shown() -> f64 {
    let handle = find(Role::Separator, "List Width").expect("the handle");
    handle.value.expect("a value").now
}

#[test]
fn the_list_keeps_its_width_where_the_detail_has_room() {
    assert_eq!(list_width_in(320.0, 1200.0), 320.0);
    assert_eq!(list_width_in(600.0, 1200.0), LIST_MAX_WIDTH);
}

#[test]
fn the_list_gets_narrower_before_the_detail_gets_narrower_than_its_least() {
    assert_eq!(list_width_in(480.0, 800.0), 800.0 - GAP - DETAIL_MIN_WIDTH);
    assert_eq!(list_width_in(320.0, 500.0), LIST_MIN_WIDTH);
}

#[test]
fn the_list_starts_at_its_width() {
    let _ui = start(Split::new(), 1200);

    assert_eq!(shown(), LIST_WIDTH as f64);
}

#[test]
fn dragging_the_handle_moves_the_divider() {
    let split = Split::new();
    let mut ui = start(split, 1200);
    let bounds = find(Role::Separator, "List Width").unwrap().bounds;
    let (x, y) = (
        bounds.x + bounds.width / 2.0,
        bounds.y + bounds.height / 2.0,
    );

    drag(&mut ui, (x, y), (x + 100.0, y));

    assert_eq!(split.list_width(), LIST_WIDTH + 100.0);
    assert_eq!(shown(), (LIST_WIDTH + 100.0) as f64);
}

#[test]
fn a_drag_stops_at_the_bounds() {
    let split = Split::new();
    let mut ui = start(split, 1200);
    let bounds = find(Role::Separator, "List Width").unwrap().bounds;
    let (x, y) = (
        bounds.x + bounds.width / 2.0,
        bounds.y + bounds.height / 2.0,
    );

    drag(&mut ui, (x, y), (x + 600.0, y));
    assert_eq!(split.list_width(), LIST_MAX_WIDTH);

    let x = x + LIST_MAX_WIDTH - LIST_WIDTH;
    drag(&mut ui, (x, y), (x - 600.0, y));
    assert_eq!(split.list_width(), LIST_MIN_WIDTH);
}

#[test]
fn the_keys_move_the_divider() {
    let split = Split::new();
    let mut ui = start(split, 1200);
    focus(&mut ui, Role::Separator, "List Width");

    key(&mut ui, NamedKey::ArrowRight);
    assert_eq!(split.list_width(), LIST_WIDTH + KEY_STEP);
    key(&mut ui, NamedKey::ArrowLeft);
    key(&mut ui, NamedKey::ArrowLeft);
    assert_eq!(split.list_width(), LIST_WIDTH - KEY_STEP);
    key(&mut ui, NamedKey::End);
    assert_eq!(split.list_width(), LIST_MAX_WIDTH);
    key(&mut ui, NamedKey::Home);
    assert_eq!(split.list_width(), LIST_MIN_WIDTH);
}

#[test]
fn a_narrow_window_takes_from_the_list_what_it_gives_back_when_wider() {
    let split = Split::new();
    let mut ui = start(split, 1200);
    focus(&mut ui, Role::Separator, "List Width");
    key(&mut ui, NamedKey::End);

    resize(&mut ui, 800);
    assert_eq!(shown(), (800.0 - GAP - DETAIL_MIN_WIDTH) as f64);
    assert_eq!(split.list_width(), LIST_MAX_WIDTH);

    resize(&mut ui, 1200);
    assert_eq!(shown(), LIST_MAX_WIDTH as f64);
}

#[test]
fn the_keys_stop_where_the_detail_keeps_its_least() {
    let split = Split::new();
    let mut ui = start(split, 800);
    focus(&mut ui, Role::Separator, "List Width");

    key(&mut ui, NamedKey::End);

    assert_eq!(split.list_width(), 800.0 - GAP - DETAIL_MIN_WIDTH);
}
