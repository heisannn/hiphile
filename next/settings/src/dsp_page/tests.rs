use anti_webui::prelude::Role;
use next_ui::testing::{core, find, has_text, press, start, wait_until};

use super::*;

#[test]
fn the_crossfeed_settings_show_while_it_is_on() {
    let (_core, hiphile) = core("");
    let dsp = hiphile.settings().dsp().clone();
    let mut ui = start({
        let dsp = dsp.clone();
        move || dsp_page(&dsp)
    });
    assert!(find(Role::RadioGroup, "Preset").is_none());
    assert!(find(Role::ComboBox, "Separation").is_none());

    press(&mut ui, Role::Switch, "Crossfeed");
    assert!(dsp.snapshot().is_crossfeed_on());
    wait_until(&mut ui, || find(Role::RadioGroup, "Preset").is_some());
    for name in ["Separation", "Cutoff"] {
        assert!(find(Role::ComboBox, name).is_some(), "{name}");
    }

    press(&mut ui, Role::Radio, "Chu Moy");
    assert_eq!(dsp.snapshot().crossfeed_preset_ix(), 1);

    press(&mut ui, Role::Switch, "Crossfeed");
    wait_until(&mut ui, || find(Role::RadioGroup, "Preset").is_none());
}

#[test]
fn a_crossfeed_value_is_chosen_from_its_menu() {
    let (_core, hiphile) = core("crossfeed = true\n");
    let dsp = hiphile.settings().dsp().clone();
    let mut ui = start({
        let dsp = dsp.clone();
        move || dsp_page(&dsp)
    });
    let other = dsp
        .snapshot()
        .cutoff_options()
        .into_iter()
        .find(|choice| choice.value() != dsp.snapshot().cutoff_value())
        .expect("another cutoff");

    press(&mut ui, Role::ComboBox, "Cutoff");
    press(&mut ui, Role::MenuItem, other.label());

    assert_eq!(dsp.snapshot().cutoff_value(), other.value());
    wait_until(&mut ui, || {
        find(Role::ComboBox, "Cutoff").and_then(|field| field.value_text)
            == Some(other.label().to_owned())
    });
}

#[test]
fn a_crossfeed_setting_that_cannot_be_read_says_so() {
    let (_core, hiphile) = core("crossfeed =\n");
    let dsp = hiphile.settings().dsp().clone();
    let errors: Vec<String> = dsp.snapshot().errors().map(str::to_owned).collect();
    let _ui = start(move || dsp_page(&dsp));

    assert!(!errors.is_empty());
    for error in errors {
        assert!(has_text(&error), "{error}");
    }
}

#[test]
fn the_balance_is_on_the_page() {
    let (_core, hiphile) = core("balance_db = -1.5\n");
    let dsp = hiphile.settings().dsp().clone();
    let _ui = start(move || dsp_page(&dsp));

    assert!(find(Role::Group, "Balance").is_some());
    assert!(has_text("Right -1.5 dB"));
    assert!(find(Role::Button, "Reset Balance").is_some());
}
