use anti_webui::core::Ui;

use super::*;
use crate::testing::{drag, find, focus, frames, key, start_at};

const LEAST: f32 = 200.0;
const MOST: f32 = 400.0;

/// A handle with the pane it resizes after it, as the queue has.
fn start(width: Signal<f32>) -> Ui {
    let mut ui = start_at(
        move || {
            div()
                .size_full()
                .flex()
                .flex_row()
                .child(div().flex_1())
                .child(pane_handle(
                    "Width",
                    Resizes::After,
                    move || width.get(),
                    LEAST,
                    || MOST,
                    move |wanted| width.set(wanted),
                ))
                .child(div().flex_none().dyn_style(move |s| s.w(width.get())))
        },
        (1000, 600),
    );
    frames(&mut ui, 16);
    ui
}

fn middle() -> (f32, f32) {
    let bounds = find(Role::Separator, "Width").expect("the handle").bounds;
    (
        bounds.x + bounds.width / 2.0,
        bounds.y + bounds.height / 2.0,
    )
}

#[test]
fn dragging_toward_the_pane_after_it_makes_that_narrower() {
    let width = signal(300.0f32);
    let mut ui = start(width);
    let (x, y) = middle();

    drag(&mut ui, (x, y), (x + 50.0, y));
    assert_eq!(width.get_untracked(), 250.0);

    let (x, y) = middle();
    drag(&mut ui, (x, y), (x - 500.0, y));
    assert_eq!(width.get_untracked(), MOST);
}

#[test]
fn left_widens_the_pane_after_it() {
    let width = signal(300.0f32);
    let mut ui = start(width);
    focus(&mut ui, Role::Separator, "Width");

    key(&mut ui, NamedKey::ArrowLeft);
    assert_eq!(width.get_untracked(), 300.0 + KEY_STEP);
    key(&mut ui, NamedKey::ArrowRight);
    key(&mut ui, NamedKey::ArrowRight);
    assert_eq!(width.get_untracked(), 300.0 - KEY_STEP);
    key(&mut ui, NamedKey::Home);
    assert_eq!(width.get_untracked(), LEAST);
    key(&mut ui, NamedKey::End);
    assert_eq!(width.get_untracked(), MOST);
}

#[test]
fn it_tells_the_width_and_its_bounds() {
    let width = signal(300.0f32);
    let _ui = start(width);

    let value = find(Role::Separator, "Width").unwrap().value.unwrap();
    assert_eq!(
        (value.now, value.min, value.max),
        (300.0, LEAST as f64, MOST as f64)
    );
}
