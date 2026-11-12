use anti_webui::core::Ui;
use anti_webui::core::event::NamedKey;
use config::R8brainQuality;
use next_ui::testing::{TestCore, core, find, has_text, key, press, start, wait_until};

use super::*;
use crate::controls::never;
use crate::testing::{replace, settle, value};

struct R8brain {
    audio: AudioSettings,
    number: R8brainNumber,
    ui: Ui,
    _core: TestCore,
}

/// The field of `number`, with r8brain resampling.
fn r8brain(number: R8brainNumber) -> R8brain {
    let (core, hiphile) = core("");
    let audio = hiphile.settings().audio().clone();
    audio.select_resampler(1);
    let ui = start({
        let audio = audio.clone();
        move || {
            let shown = next_host::follow(audio.subscribe());
            r8brain_number_field(&audio, shown, number, never())
        }
    });
    R8brain {
        audio,
        number,
        ui,
        _core: core,
    }
}

impl R8brain {
    fn filter(&self) -> config::R8brainFilter {
        self.audio.snapshot().r8brain_filter()
    }

    fn shown(&self) -> String {
        value(Role::TextInput, self.number.name())
    }

    fn type_text(&mut self, text: &str) {
        replace(&mut self.ui, Role::TextInput, self.number.name(), text);
    }

    fn wait_for(&mut self, shown: &str) {
        let name = self.number.name();
        wait_until(&mut self.ui, || value(Role::TextInput, name) == shown);
    }

    fn focus(&mut self) {
        next_ui::testing::focus(&mut self.ui, Role::TextInput, self.number.name());
    }

    /// Takes the focus to the next stop, the button after the field.
    fn leave(&mut self) {
        key(&mut self.ui, NamedKey::Tab);
    }

    fn press_enter(&mut self) {
        key(&mut self.ui, NamedKey::Enter);
    }
}

#[test]
fn the_field_starts_from_the_parameters() {
    let it = r8brain(R8brainNumber::Attenuation);
    assert_eq!(it.shown(), "180.15");
}

#[test]
fn typing_counts_once_it_has_settled() {
    let mut it = r8brain(R8brainNumber::TransitionBand);
    it.type_text("3.5");
    assert_eq!(it.filter().transition_band, 2.0);

    let audio = it.audio.clone();
    wait_until(&mut it.ui, || {
        audio.snapshot().r8brain_filter().transition_band == 3.5
    });
}

#[test]
fn enter_shows_the_number_held_to_the_range() {
    let mut it = r8brain(R8brainNumber::Attenuation);
    it.type_text("999");
    it.press_enter();
    assert_eq!(it.filter().attenuation, 218.0);
    it.wait_for("218");
}

#[test]
fn typing_beyond_the_range_is_said_and_stays_until_enter() {
    let mut it = r8brain(R8brainNumber::Attenuation);
    it.type_text("1");
    settle(&mut it.ui);
    assert_eq!(it.shown(), "1");
    assert!(has_text("Enter 49 to 218."));

    it.press_enter();
    it.wait_for("49");
    assert!(!has_text("Enter 49 to 218."));
}

#[test]
fn text_that_is_no_number_gives_way_to_the_value() {
    let mut it = r8brain(R8brainNumber::TransitionBand);
    it.type_text("");
    it.press_enter();
    it.wait_for("2");
}

#[test]
fn what_no_number_has_cannot_be_typed() {
    let mut it = r8brain(R8brainNumber::FilterCacheSize);
    it.type_text("1x2.5-");
    assert_eq!(it.shown(), "125");
}

#[test]
fn the_field_follows_a_change_from_elsewhere() {
    let mut it = r8brain(R8brainNumber::Attenuation);
    it.audio.set_r8brain_bit_depth("16-bit");
    it.wait_for("136.45");
}

#[test]
fn the_field_follows_a_preset() {
    let mut it = r8brain(R8brainNumber::TransitionBand);
    it.audio.select_r8brain_quality(3);
    it.wait_for("0.5");
}

#[test]
fn a_preset_reaches_the_field_in_focus_and_leaving_it_keeps_the_preset() {
    let mut it = r8brain(R8brainNumber::Attenuation);
    it.focus();
    it.audio.select_r8brain_quality(3);
    it.wait_for("218");

    it.audio.select_r8brain_quality(0);
    it.wait_for("136.45");
    it.leave();
    assert_eq!(
        it.audio.snapshot().r8brain_parameters().quality,
        R8brainQuality::Low
    );
}

#[test]
fn a_preset_replaces_typing_that_has_not_settled() {
    let mut it = r8brain(R8brainNumber::Attenuation);
    it.type_text("150");
    it.audio.select_r8brain_quality(0);
    it.wait_for("136.45");

    settle(&mut it.ui);
    it.leave();
    assert_eq!(
        it.audio.snapshot().r8brain_parameters().quality,
        R8brainQuality::Low
    );
}

#[test]
fn typing_keeps_its_form_once_it_has_settled() {
    let mut it = r8brain(R8brainNumber::TransitionBand);
    it.type_text("3.50");
    settle(&mut it.ui);
    assert_eq!(it.filter().transition_band, 3.5);
    assert_eq!(it.shown(), "3.50");

    it.leave();
    it.wait_for("3.5");
}

#[test]
fn an_empty_field_shows_that_the_resampler_chooses_the_filter_positions() {
    let mut it = r8brain(R8brainNumber::FilterFractions);
    it.type_text("512");
    it.press_enter();
    it.wait_for("512");

    it.type_text("");
    it.press_enter();
    assert_eq!(it.filter().filter_fractions, None);
    settle(&mut it.ui);
    assert_eq!(it.shown(), "");
}

#[test]
fn reset_shows_the_default() {
    let mut it = r8brain(R8brainNumber::FilterBankCacheSize);
    let reset = "Reset r8brain filter bank cache";
    assert!(find(Role::Button, reset).is_none());
    it.type_text("3");
    it.press_enter();
    wait_until(&mut it.ui, || find(Role::Button, reset).is_some());

    press(&mut it.ui, Role::Button, reset);
    it.wait_for("12");
    wait_until(&mut it.ui, || find(Role::Button, reset).is_none());
}

#[test]
fn the_buttons_and_the_arrow_keys_step_by_the_ranges_step() {
    let mut it = r8brain(R8brainNumber::TransitionBand);
    press(&mut it.ui, Role::Button, "Increase r8brain transition band");
    assert_eq!(it.shown(), "2.5");
    assert!(find(Role::TextInput, it.number.name()).unwrap().focusable);

    key(&mut it.ui, NamedKey::ArrowDown);
    key(&mut it.ui, NamedKey::ArrowDown);
    assert_eq!(it.shown(), "1.5");
    let audio = it.audio.clone();
    wait_until(&mut it.ui, || {
        audio.snapshot().r8brain_filter().transition_band == 1.5
    });

    press(&mut it.ui, Role::Button, "Decrease r8brain transition band");
    assert_eq!(it.shown(), "1");
}

#[test]
fn a_step_stops_at_the_end_of_the_range() {
    let mut it = r8brain(R8brainNumber::Attenuation);
    it.type_text("218");
    press(&mut it.ui, Role::Button, "Increase r8brain attenuation");
    assert_eq!(it.shown(), "218");
}

struct Limit {
    limit: CacheLimit,
    ui: Ui,
    _core: TestCore,
}

const COVER_LIMIT: &str = "Cover cache limit";

fn cover_limit() -> Limit {
    let (core, hiphile) = core("");
    let limit = hiphile.settings().cover_cache().clone();
    let ui = start({
        let limit = limit.clone();
        move || {
            let shown = next_host::follow(limit.subscribe());
            cache_limit_field(&limit, shown)
        }
    });
    Limit {
        limit,
        ui,
        _core: core,
    }
}

#[test]
fn leaving_the_limit_shows_it_as_it_counts() {
    let mut it = cover_limit();
    assert_eq!(value(Role::TextInput, COVER_LIMIT), "8");
    replace(&mut it.ui, Role::TextInput, COVER_LIMIT, "");
    key(&mut it.ui, NamedKey::Tab);
    assert_eq!(value(Role::TextInput, COVER_LIMIT), "8");

    replace(&mut it.ui, Role::TextInput, COVER_LIMIT, "007");
    assert_eq!(it.limit.snapshot().gib(), 7.0);
    assert_eq!(value(Role::TextInput, COVER_LIMIT), "007");
    key(&mut it.ui, NamedKey::Tab);
    assert_eq!(value(Role::TextInput, COVER_LIMIT), "7");
}

#[test]
fn reset_shows_the_default_limit() {
    let mut it = cover_limit();
    replace(&mut it.ui, Role::TextInput, COVER_LIMIT, "3");
    wait_until(&mut it.ui, || {
        find(Role::Button, "Reset Cover cache limit").is_some()
    });

    press(&mut it.ui, Role::Button, "Reset Cover cache limit");
    assert_eq!(value(Role::TextInput, COVER_LIMIT), "8");
    assert!(it.limit.snapshot().is_default());
    wait_until(&mut it.ui, || {
        find(Role::Button, "Reset Cover cache limit").is_none()
    });
}
