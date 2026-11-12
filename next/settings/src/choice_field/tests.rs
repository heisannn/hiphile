use anti_webui::core::Ui;
use anti_webui::core::access::snapshot;
use anti_webui::core::event::NamedKey;
use hiphile_core::settings::{AudioSettings, AudioSnapshot};
use next_ui::testing::{TestCore, core, find, focus, key, nodes, press, start, wait_until};

use super::*;
use crate::controls::{never, when};

const NAME: &str = "Crossfade";

struct Fixture {
    audio: AudioSettings,
    /// How many of the crossfades are offered.
    offered: Signal<usize>,
    ui: Ui,
    _core: TestCore,
}

fn fixture(disabled: When) -> Fixture {
    let (core, hiphile) = core("");
    let audio = hiphile.settings().audio().clone();
    let offered = std::rc::Rc::new(std::cell::Cell::new(None));
    let ui = start({
        let (audio, offered) = (audio.clone(), offered.clone());
        move || {
            let count = signal(usize::MAX);
            offered.set(Some(count));
            let shown = next_host::follow(audio.subscribe());
            let set = audio.clone();
            choice_field(
                NAME,
                move || {
                    let count = count.get();
                    AudioSettings::crossfade_options()
                        .into_iter()
                        .take(count)
                        .collect()
                },
                move || shown.with(AudioSnapshot::crossfade_value),
                move |value| set.set_crossfade(value),
                disabled,
            )
        }
    });
    Fixture {
        audio,
        offered: offered.get().expect("the view made"),
        ui,
        _core: core,
    }
}

fn field() -> anti_webui::core::access::AccessNode {
    find(Role::ComboBox, NAME).expect("the field")
}

#[test]
fn the_field_is_one_stop_that_names_its_setting_and_says_the_choice() {
    let _it = fixture(never());
    let field = field();
    assert!(field.focusable);
    assert_eq!(field.value_text.as_deref(), Some("Off"));
    assert_eq!(field.expanded, Some(false));
    assert!(nodes(Role::TextInput).is_empty());
}

#[test]
fn enter_opens_the_menu_and_a_choice_is_set_and_closes_it() {
    let mut it = fixture(never());
    focus(&mut it.ui, Role::ComboBox, NAME);
    key(&mut it.ui, NamedKey::Enter);
    assert_eq!(field().expanded, Some(true));

    press(&mut it.ui, Role::MenuItem, "3 s");
    assert_eq!(it.audio.snapshot().crossfade_value(), "3");
    assert_eq!(field().expanded, Some(false));
    wait_until(&mut it.ui, || field().value_text.as_deref() == Some("3 s"));
    assert_eq!(snapshot().focus, Some(field().id));
}

#[test]
fn the_arrow_keys_open_the_menu_and_escape_closes_it() {
    let mut it = fixture(never());
    focus(&mut it.ui, Role::ComboBox, NAME);
    key(&mut it.ui, NamedKey::ArrowDown);
    assert_eq!(field().expanded, Some(true));

    key(&mut it.ui, NamedKey::Escape);
    assert_eq!(field().expanded, Some(false));
    assert_eq!(it.audio.snapshot().crossfade_value(), "0");
}

#[test]
fn a_press_opens_the_menu() {
    let mut it = fixture(never());
    press(&mut it.ui, Role::ComboBox, NAME);
    assert_eq!(field().expanded, Some(true));
    assert!(find(Role::MenuItem, "1 s").is_some());
}

#[test]
fn the_menu_follows_the_choices() {
    let mut it = fixture(never());
    it.offered.set(3);
    press(&mut it.ui, Role::ComboBox, NAME);
    assert_eq!(nodes(Role::MenuItem).len(), 3);

    it.offered.set(2);
    it.ui.update();
    assert_eq!(nodes(Role::MenuItem).len(), 2);
}

#[test]
fn a_disabled_field_says_so_and_does_not_open() {
    let mut it = fixture(when(|| true));
    assert!(field().disabled);
    focus(&mut it.ui, Role::ComboBox, NAME);
    key(&mut it.ui, NamedKey::Enter);
    assert_eq!(field().expanded, Some(false));
}
