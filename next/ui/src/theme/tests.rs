use anti_webui::prelude::{div, signal};
use anti_webui_md3e::color_scheme;

use super::*;
use crate::testing::{frames, start};

#[test]
fn the_appearance_chosen_wins_over_the_system() {
    assert!(is_dark(Some(Appearance::Dark), false));
    assert!(!is_dark(Some(Appearance::Light), true));
    assert!(is_dark(None, true));
    assert!(!is_dark(None, false));
}

#[test]
fn the_scheme_follows_the_appearance_and_the_seed() {
    let dark = signal(false);
    let seed = signal(None);
    let mut ui = start(move || {
        follow_theme(move || dark.get(), move || seed.get(), || false);
        div()
    });
    assert_eq!(color_scheme(), scheme(DEFAULT_SEED, false));

    dark.set(true);
    seed.set(Some(rgb(0xc0392b)));
    frames(&mut ui, 1500);
    assert_eq!(color_scheme(), scheme(rgb(0xc0392b), true));
}

#[test]
fn with_less_motion_the_scheme_changes_at_once() {
    let dark = signal(false);
    let mut ui = start(move || {
        follow_theme(move || dark.get(), || None, || true);
        div()
    });

    dark.set(true);
    ui.update();
    assert_eq!(color_scheme(), scheme(DEFAULT_SEED, true));
}
