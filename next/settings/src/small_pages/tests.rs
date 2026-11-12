use anti_webui::core::access::Checked;
use anti_webui::core::event::NamedKey;
use anti_webui::prelude::Role;
use config::{Appearance, LyricsHighlight, ThemeColor};
use next_ui::testing::{core, find, focus, has_text, key, press, start, wait_until};

use super::*;

/// A settings file that is no TOML.
const UNREADABLE: &str = "skip_instrumentals =\n";

fn is_checked(role: Role, name: &str) -> bool {
    find(role, name).and_then(|node| node.checked) == Some(Checked::True)
}

#[test]
fn skip_instrumentals_is_saved_and_shown() {
    let (_core, hiphile) = core("");
    let queue = hiphile.settings().queue().clone();
    let mut ui = start({
        let queue = queue.clone();
        move || queue_page(&queue)
    });
    assert!(!is_checked(Role::Switch, "Skip instrumentals"));

    press(&mut ui, Role::Switch, "Skip instrumentals");
    assert!(queue.snapshot().is_skip_instrumentals_on());
    wait_until(&mut ui, || is_checked(Role::Switch, "Skip instrumentals"));
}

#[test]
fn a_setting_that_cannot_be_read_says_so() {
    let (_core, hiphile) = core(UNREADABLE);
    let queue = hiphile.settings().queue().clone();
    let error = queue.snapshot().error().expect("an error").to_owned();
    let _ui = start(move || queue_page(&queue));

    assert!(error.starts_with("Couldn't read this setting"), "{error}");
    assert!(has_text(&error));
}

#[test]
fn the_highlight_is_one_of_the_radio_buttons() {
    let (_core, hiphile) = core("");
    let lyrics = hiphile.settings().lyrics().clone();
    let mut ui = start({
        let lyrics = lyrics.clone();
        move || lyrics_page(&lyrics)
    });
    assert!(find(Role::RadioGroup, "Highlight").is_some());
    assert!(is_checked(Role::Radio, "By word"));

    press(&mut ui, Role::Radio, "By line");
    assert_eq!(lyrics.snapshot().highlight(), LyricsHighlight::ByLine);
    wait_until(&mut ui, || is_checked(Role::Radio, "By line"));
    assert!(!is_checked(Role::Radio, "By word"));
}

#[test]
fn a_label_chooses_its_radio_button() {
    let (_core, hiphile) = core("");
    let lyrics = hiphile.settings().lyrics().clone();
    let mut ui = start({
        let lyrics = lyrics.clone();
        move || lyrics_page(&lyrics)
    });

    next_ui::testing::click_text(&mut ui, "By line");
    assert_eq!(lyrics.snapshot().highlight(), LyricsHighlight::ByLine);
}

fn appearance_page_in_light(appearance: &AppearanceSettings) -> Div {
    appearance_page(appearance, Schemes::new(|| false, signal(None)))
}

#[test]
fn the_theme_is_chosen_and_saved() {
    let (_core, hiphile) = core("");
    let appearance = hiphile.settings().appearance().clone();
    let mut ui = start({
        let appearance = appearance.clone();
        move || appearance_page_in_light(&appearance)
    });
    assert!(is_checked(Role::Radio, "System"));

    press(&mut ui, Role::Radio, "Dark");
    assert_eq!(appearance.snapshot().appearance(), Some(Appearance::Dark));
    wait_until(&mut ui, || is_checked(Role::Radio, "Dark"));

    press(&mut ui, Role::Radio, "System");
    assert_eq!(appearance.snapshot().appearance(), None);
}

#[test]
fn a_theme_that_cannot_be_read_says_so() {
    let (_core, hiphile) = core("appearance = \"purple\"\n");
    let appearance = hiphile.settings().appearance().clone();
    let error = appearance
        .snapshot()
        .errors()
        .next()
        .expect("an error")
        .to_owned();
    let _ui = start(move || appearance_page_in_light(&appearance));

    assert!(has_text(&error));
}

#[test]
fn the_color_is_the_cover_until_one_is_chosen() {
    let (_core, hiphile) = core("");
    let appearance = hiphile.settings().appearance().clone();
    let mut ui = start({
        let appearance = appearance.clone();
        move || appearance_page_in_light(&appearance)
    });
    assert!(is_checked(Role::Radio, "From the cover"));

    press(&mut ui, Role::Radio, "Teal");
    assert_eq!(appearance.snapshot().theme_color(), ThemeColor::Teal);
    wait_until(&mut ui, || is_checked(Role::Radio, "Teal"));
    assert!(!is_checked(Role::Radio, "From the cover"));
}

#[test]
fn a_custom_color_is_picked_below_the_swatches_while_it_is_chosen() {
    let (_core, hiphile) = core("");
    let appearance = hiphile.settings().appearance().clone();
    let mut ui = start({
        let appearance = appearance.clone();
        move || appearance_page_in_light(&appearance)
    });
    assert!(find(Role::Group, "Custom color").is_none());

    press(&mut ui, Role::Radio, "Custom");
    wait_until(&mut ui, || find(Role::Group, "Custom color").is_some());
    focus(&mut ui, Role::Slider, "Tone");
    key(&mut ui, NamedKey::End);
    assert_eq!(
        appearance.snapshot().theme_color(),
        ThemeColor::Custom(0xffffff)
    );

    press(&mut ui, Role::Radio, "Teal");
    wait_until(&mut ui, || find(Role::Group, "Custom color").is_none());
    press(&mut ui, Role::Radio, "Custom");
    assert_eq!(
        appearance.snapshot().theme_color(),
        ThemeColor::Custom(0xffffff)
    );
}

#[test]
fn a_custom_color_from_the_file_is_chosen_and_in_the_picker() {
    let (_core, hiphile) = core("theme_color = \"#2e7d32\"\n");
    let appearance = hiphile.settings().appearance().clone();
    let _ui = start(move || appearance_page_in_light(&appearance));

    assert!(is_checked(Role::Radio, "Custom"));
    assert_eq!(crate::testing::value(Role::TextInput, "Hex"), "2e7d32");
}

#[test]
fn a_color_that_cannot_be_read_says_so() {
    let (_core, hiphile) = core("theme_color = \"mauve\"\n");
    let appearance = hiphile.settings().appearance().clone();
    let error = appearance
        .snapshot()
        .errors()
        .next()
        .expect("an error")
        .to_owned();
    let _ui = start(move || appearance_page_in_light(&appearance));

    assert!(error.starts_with("Couldn't read the color"), "{error}");
    assert!(has_text(&error));
    assert!(is_checked(Role::Radio, "From the cover"));
}
