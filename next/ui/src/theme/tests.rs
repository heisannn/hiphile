use anti_webui::prelude::{div, signal};
use anti_webui_md3e::color_scheme;
use hiphile_core::settings::THEME_COLORS;

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
fn the_scheme_follows_the_appearance_and_the_cover() {
    let dark = signal(false);
    let seed = signal(None);
    let mut ui = start(move || {
        let schemes = Schemes::new(move || dark.get(), seed);
        follow_theme(move || schemes.of(ThemeColor::Cover), || false);
        div()
    });
    assert_eq!(color_scheme(), cover_scheme(DEFAULT_SEED, false));

    dark.set(true);
    seed.set(Some(rgb(0xc0392b)));
    frames(&mut ui, 1500);
    assert_eq!(color_scheme(), cover_scheme(rgb(0xc0392b), true));
}

#[test]
fn a_picked_color_leaves_the_cover_out() {
    let seed = signal(Some(rgb(0xc0392b)));
    let color = signal(ThemeColor::Teal);
    let mut ui = start(move || {
        let schemes = Schemes::new(|| false, seed);
        follow_theme(move || schemes.of(color.get()), || false);
        div()
    });
    let teal = scheme(ThemeColor::Teal, None, false);
    assert_eq!(color_scheme(), teal);

    seed.set(Some(rgb(0x2e86c1)));
    frames(&mut ui, 1500);
    assert_eq!(color_scheme(), teal);

    color.set(ThemeColor::Cover);
    frames(&mut ui, 1500);
    assert_eq!(color_scheme(), cover_scheme(rgb(0x2e86c1), false));
}

#[test]
fn each_picked_color_gives_tonal_spots_primary_in_its_own_hue() {
    for (color, _) in THEME_COLORS {
        let Some(hue) = picked_hue(color) else {
            continue;
        };
        let primary = Hct::from_color(scheme(color, None, false).primary);
        let off = (primary.hue() - hue).rem_euclid(360.0);
        assert!(off.min(360.0 - off) < 5.0, "{color:?}: {}", primary.hue());
        assert!(
            (primary.chroma() - 36.0).abs() < 2.0,
            "{color:?}: {}",
            primary.chroma()
        );
    }
}

#[test]
fn a_swatch_shows_its_seed_and_stays_as_colorful_as_it() {
    for (color, _) in THEME_COLORS {
        let seed = Hct::from_color(swatch(color, None).seed);
        assert!(seed.chroma() > 40.0, "{color:?}: {}", seed.chroma());
        assert!(
            (seed.tone() - 70.0).abs() < 1.0,
            "{color:?}: {}",
            seed.tone()
        );
    }
    let gray = swatch(ThemeColor::Cover, Some(rgb(0x808080)));
    assert!(Hct::from_color(gray.seed).chroma() < 5.0);
}

#[test]
fn with_less_motion_the_scheme_changes_at_once() {
    let dark = signal(false);
    let mut ui = start(move || {
        follow_theme(move || cover_scheme(DEFAULT_SEED, dark.get()), || true);
        div()
    });

    dark.set(true);
    ui.update();
    assert_eq!(color_scheme(), cover_scheme(DEFAULT_SEED, true));
}
