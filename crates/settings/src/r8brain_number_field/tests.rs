use std::time::Duration;

use config::R8brainQuality;
use gpui_kit::{TestAppContext, VisualTestContext};

use super::*;

struct Fixture<'a> {
    audio: AudioSettings,
    field: Entity<R8brainNumberField>,
    cx: &'a mut VisualTestContext,
    _dir: tempfile::TempDir,
}

fn fixture(number: R8brainNumber, cx: &mut TestAppContext) -> Fixture<'_> {
    cx.update(gpui_kit::init);
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let executor = cx.update(|cx| gpui_host::executor(cx));
    let audio = AudioSettings::new(executor, Ok(store));
    audio.select_resampler(1);
    let held = audio.clone();
    let (field, cx) =
        cx.add_window_view(|window, cx| R8brainNumberField::new(held, number, window, cx));
    Fixture {
        audio,
        field,
        cx,
        _dir: dir,
    }
}

impl Fixture<'_> {
    fn filter(&mut self) -> config::R8brainFilter {
        self.audio.snapshot().r8brain_filter()
    }

    fn shown(&mut self) -> String {
        self.field.read_with(self.cx, |field, cx| {
            field.input.read(cx).value().to_string()
        })
    }

    fn type_text(&mut self, text: &str) {
        let text = text.to_owned();
        self.field.update_in(self.cx, |field, window, cx| {
            field
                .input
                .update(cx, |input, cx| input.replace_all(text, window, cx));
        });
    }

    fn press_enter(&mut self) {
        self.field.update_in(self.cx, |field, _, cx| {
            field.input.update(cx, |_, cx| {
                cx.emit(InputEvent::PressEnter {
                    secondary: false,
                    shift: false,
                });
            });
        });
        self.cx.run_until_parked();
    }

    fn focus(&mut self) {
        self.field.update_in(self.cx, |field, window, cx| {
            field.input.update(cx, |input, cx| input.focus(window, cx));
        });
        self.cx.run_until_parked();
    }

    fn blur(&mut self) {
        self.field.update_in(self.cx, |field, window, cx| {
            window.blur(cx);
            field.input.update(cx, |_, cx| cx.emit(InputEvent::Blur));
        });
        self.cx.run_until_parked();
    }

    fn select_preset(&mut self, ix: usize) {
        self.audio.select_r8brain_quality(ix);
        self.cx.run_until_parked();
    }

    fn settle(&mut self) {
        self.cx.executor().advance_clock(Duration::from_secs(1));
        self.cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn the_field_starts_from_the_parameters(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::Attenuation, cx);
    assert_eq!(it.shown(), "180.15");
}

#[gpui_kit::test]
fn typing_counts_once_it_has_settled(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::TransitionBand, cx);
    it.type_text("3.5");
    it.cx.run_until_parked();
    assert_eq!(it.filter().transition_band, 2.0);

    it.settle();
    assert_eq!(it.filter().transition_band, 3.5);
}

#[gpui_kit::test]
fn enter_shows_the_number_held_to_the_range(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::Attenuation, cx);
    it.type_text("999");
    it.press_enter();
    assert_eq!(it.filter().attenuation, 218.0);
    assert_eq!(it.shown(), "218");
}

#[gpui_kit::test]
fn typing_beyond_the_range_stays_until_enter(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::Attenuation, cx);
    it.type_text("1");
    it.settle();
    assert_eq!(it.shown(), "1");

    it.press_enter();
    assert_eq!(it.shown(), "49");
}

#[gpui_kit::test]
fn text_that_is_no_number_gives_way_to_the_value(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::TransitionBand, cx);
    it.type_text("");
    it.press_enter();
    assert_eq!(it.shown(), "2");
}

#[gpui_kit::test]
fn the_field_follows_a_change_from_elsewhere(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::Attenuation, cx);
    it.audio.set_r8brain_bit_depth("16-bit");
    it.cx.run_until_parked();
    assert_eq!(it.shown(), "136.45");
}

#[gpui_kit::test]
fn the_field_follows_a_preset(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::TransitionBand, cx);
    it.audio.select_r8brain_quality(3);
    it.cx.run_until_parked();
    assert_eq!(it.shown(), "0.5");
}

#[gpui_kit::test]
fn a_preset_reaches_the_field_in_focus_and_leaving_it_keeps_the_preset(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::Attenuation, cx);
    it.focus();
    it.select_preset(3);
    assert_eq!(it.shown(), "218");

    it.select_preset(0);
    assert_eq!(it.shown(), "136.45");
    it.blur();
    assert_eq!(
        it.audio.snapshot().r8brain_parameters().quality,
        R8brainQuality::Low
    );
}

#[gpui_kit::test]
fn a_preset_replaces_typing_that_has_not_settled(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::Attenuation, cx);
    it.focus();
    it.type_text("150");
    it.select_preset(0);
    assert_eq!(it.shown(), "136.45");

    it.settle();
    it.blur();
    assert_eq!(
        it.audio.snapshot().r8brain_parameters().quality,
        R8brainQuality::Low
    );
}

#[gpui_kit::test]
fn typing_keeps_its_form_once_it_has_settled(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::TransitionBand, cx);
    it.focus();
    it.type_text("3.50");
    it.settle();
    assert_eq!(it.filter().transition_band, 3.5);
    assert_eq!(it.shown(), "3.50");

    it.blur();
    assert_eq!(it.shown(), "3.5");
}

#[gpui_kit::test]
fn an_empty_field_shows_that_the_resampler_chooses_the_filter_positions(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::FilterFractions, cx);
    it.type_text("512");
    it.press_enter();
    assert_eq!(it.shown(), "512");

    it.type_text("");
    it.press_enter();
    assert_eq!(it.filter().filter_fractions, None);
    assert_eq!(it.shown(), "");
}

#[gpui_kit::test]
fn reset_shows_the_default(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::FilterBankCacheSize, cx);
    it.field
        .read_with(it.cx, |field, _| assert!(!field.is_resettable()));
    it.type_text("3");
    it.press_enter();
    it.field
        .read_with(it.cx, |field, _| assert!(field.is_resettable()));

    it.field
        .update_in(it.cx, |field, window, cx| field.reset(window, cx));
    assert_eq!(it.shown(), "12");
}
