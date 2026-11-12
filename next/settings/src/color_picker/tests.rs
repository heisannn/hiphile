use anti_webui::core::event::NamedKey;
use next_ui::testing::{find, focus, has_text, key, start};

use super::*;
use crate::testing::{replace, value};

const START: u32 = 0x4f5bd5;

/// The picker, and each color it gave, last the latest.
fn start_picker() -> (anti_webui::core::Ui, Signal<Vec<u32>>) {
    let made = std::rc::Rc::new(std::cell::Cell::new(None));
    let ui = start({
        let made = made.clone();
        move || {
            let given = signal(Vec::new());
            made.set(Some(given));
            color_picker(START, move |rgb| given.update(|given| given.push(rgb)))
        }
    });
    (ui, made.get().expect("the view made"))
}

fn slider(name: &str) -> f64 {
    find(Role::Slider, name)
        .and_then(|node| node.value)
        .unwrap_or_else(|| panic!("no slider {name:?}"))
        .now
}

fn last(given: Signal<Vec<u32>>) -> u32 {
    given.with_untracked(|given| *given.last().expect("a color given"))
}

#[test]
fn the_sliders_start_at_the_color() {
    let (_ui, given) = start_picker();
    let start = Hct::from_argb(0xff00_0000 | START);

    assert!((slider("Hue") - start.hue()).abs() < 0.01);
    assert!((slider("Chroma") - start.chroma()).abs() < 0.01);
    assert!((slider("Tone") - start.tone()).abs() < 0.01);
    assert_eq!(value(Role::TextInput, "Hex"), "4f5bd5");
    assert_eq!(last(given), START);
}

#[test]
fn a_slider_gives_the_color_at_once_and_says_it_in_hex() {
    let (mut ui, given) = start_picker();
    focus(&mut ui, Role::Slider, "Tone");

    key(&mut ui, NamedKey::End);
    assert_eq!(last(given), 0xffffff);
    assert_eq!(value(Role::TextInput, "Hex"), "ffffff");
    key(&mut ui, NamedKey::Home);
    assert_eq!(last(given), 0x000000);
    assert_eq!(value(Role::TextInput, "Hex"), "000000");
}

#[test]
fn more_chroma_than_the_color_can_have_leaves_the_slider_where_it_was_put() {
    let (mut ui, given) = start_picker();
    focus(&mut ui, Role::Slider, "Chroma");

    key(&mut ui, NamedKey::End);
    assert_eq!(slider("Chroma"), f64::from(MAX_CHROMA));
    let shown = Hct::from_argb(0xff00_0000 | last(given));
    assert!(shown.chroma() < f64::from(MAX_CHROMA), "{}", shown.chroma());
    assert!((slider("Hue") - Hct::from_argb(0xff00_0000 | START).hue()).abs() < 0.01);
}

#[test]
fn a_color_typed_in_hex_moves_the_sliders() {
    let (mut ui, given) = start_picker();

    replace(&mut ui, Role::TextInput, "Hex", "#ff0000");
    assert_eq!(last(given), 0xff0000);
    let red = Hct::from_argb(0xffff_0000);
    assert!((slider("Hue") - red.hue()).abs() < 0.01);
    assert!((slider("Tone") - red.tone()).abs() < 0.01);
    assert_eq!(value(Role::TextInput, "Hex"), "#ff0000");
}

#[test]
fn what_cannot_be_a_color_says_so_and_fewer_digits_wait() {
    let (mut ui, given) = start_picker();

    replace(&mut ui, Role::TextInput, "Hex", "ff0");
    assert_eq!(last(given), START);
    assert!(!has_text(NOT_HEX));

    replace(&mut ui, Role::TextInput, "Hex", "ff00zz");
    assert_eq!(last(given), START);
    assert!(has_text(NOT_HEX));

    replace(&mut ui, Role::TextInput, "Hex", "00ff00");
    assert_eq!(last(given), 0x00ff00);
    assert!(!has_text(NOT_HEX));
}
