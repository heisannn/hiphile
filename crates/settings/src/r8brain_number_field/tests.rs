use gpui_kit::{TestAppContext, VisualTestContext};

use super::*;
use crate::audio_settings::tests::settings_saving_to;

struct Fixture<'a> {
    audio: Entity<AudioSettings>,
    field: Entity<R8brainNumberField>,
    cx: &'a mut VisualTestContext,
    store: config::Store,
    _dir: tempfile::TempDir,
    _subscription: gpui_kit::Subscription,
}

fn fixture(number: R8brainNumber, cx: &mut TestAppContext) -> Fixture<'_> {
    cx.update(gpui_kit::init);
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let (audio, _told, subscription) = settings_saving_to(&store, cx);
    audio.update(cx, |audio, cx| audio.select_resampler(1, cx));
    let held = audio.clone();
    let (field, cx) =
        cx.add_window_view(|window, cx| R8brainNumberField::new(held, number, window, cx));
    Fixture {
        audio,
        field,
        cx,
        store,
        _dir: dir,
        _subscription: subscription,
    }
}

impl Fixture<'_> {
    fn parameters(&mut self) -> R8brainParameters {
        self.audio
            .read_with(self.cx, |audio, _| audio.r8brain_parameters())
    }

    fn filter(&mut self) -> config::R8brainFilter {
        self.parameters().quality.filter()
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
        self.audio
            .update(self.cx, |audio, cx| audio.select_r8brain_quality(ix, cx));
        self.cx.run_until_parked();
    }

    fn settle(&mut self) {
        self.cx.executor().advance_clock(SETTLE);
        self.cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn the_field_starts_from_the_parameters(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::Attenuation, cx);
    assert_eq!(it.shown(), "180.15");
}

#[gpui_kit::test]
fn a_change_counts_once_it_has_settled(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::TransitionBand, cx);
    it.type_text("3.5");
    it.cx.run_until_parked();
    assert_eq!(it.filter().transition_band, 2.0);

    it.settle();
    assert_eq!(it.filter().transition_band, 3.5);
    assert_eq!(
        it.store
            .r8brain_parameters()
            .unwrap()
            .quality
            .filter()
            .transition_band,
        3.5
    );
}

#[gpui_kit::test]
fn only_the_last_of_several_changes_counts(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::Attenuation, cx);
    it.type_text("1");
    it.type_text("10");
    it.type_text("100");
    it.settle();
    assert_eq!(it.filter().attenuation, 100.0);
}

#[gpui_kit::test]
fn enter_counts_at_once_and_shows_the_number_held_to_the_range(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::Attenuation, cx);
    it.type_text("999");
    it.press_enter();
    assert_eq!(it.filter().attenuation, 218.0);
    assert_eq!(it.shown(), "218");
}

#[gpui_kit::test]
fn typing_beyond_the_range_waits_for_enter(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::Attenuation, cx);
    it.type_text("1");
    it.settle();
    assert_eq!(it.parameters().quality, R8brainQuality::High);
    assert_eq!(it.shown(), "1");

    it.press_enter();
    assert_eq!(it.filter().attenuation, 49.0);
    assert_eq!(it.shown(), "49");
}

#[gpui_kit::test]
fn text_that_is_no_number_gives_way_to_the_value(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::TransitionBand, cx);
    it.type_text("");
    it.press_enter();
    assert_eq!(it.filter().transition_band, 2.0);
    assert_eq!(it.shown(), "2");
}

#[gpui_kit::test]
fn an_empty_field_lets_the_resampler_choose_the_filter_positions(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::FilterFractions, cx);
    assert_eq!(it.shown(), "");

    it.type_text("512");
    it.press_enter();
    assert_eq!(it.filter().filter_fractions, Some(512));

    it.type_text("");
    it.press_enter();
    assert_eq!(it.filter().filter_fractions, None);
    assert_eq!(it.shown(), "");
}

#[gpui_kit::test]
fn whole_numbers_stay_whole(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::MaxInputFrames, cx);
    it.type_text("4096");
    it.press_enter();
    assert_eq!(it.parameters().performance.max_input_frames, 4096);

    assert_eq!(R8brainNumber::FilterCacheSize.parse("3.6"), Some(Some(3.6)));
    let mut parameters = R8brainParameters::default();
    R8brainNumber::FilterCacheSize.set(&mut parameters, Some(3.6));
    assert_eq!(parameters.performance.filter_cache_size, 4);
}

#[gpui_kit::test]
fn the_field_follows_a_change_from_elsewhere(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::Attenuation, cx);
    it.audio.update(it.cx, |audio, cx| {
        audio.set_r8brain_bit_depth("16-bit", cx);
    });
    it.cx.run_until_parked();
    assert_eq!(it.shown(), "136.45");
}

#[gpui_kit::test]
fn the_field_follows_a_preset(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::TransitionBand, cx);
    it.audio.update(it.cx, |audio, cx| {
        audio.select_r8brain_quality(3, cx);
    });
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
    assert_eq!(it.parameters().quality, R8brainQuality::Low);
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
    assert_eq!(it.parameters().quality, R8brainQuality::Low);
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
fn another_number_settling_leaves_typing_alone(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::Attenuation, cx);
    it.focus();
    it.type_text("150");
    it.audio.update(it.cx, |audio, cx| {
        audio.change_r8brain(
            |parameters| parameters.performance.max_input_frames = 4096,
            cx,
        );
    });
    it.cx.run_until_parked();
    assert_eq!(it.shown(), "150");

    it.settle();
    assert_eq!(it.filter().attenuation, 150.0);
}

#[gpui_kit::test]
fn the_same_number_leaves_the_preset_and_another_makes_it_custom(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::Attenuation, cx);
    it.audio.update(it.cx, |audio, cx| {
        audio.select_r8brain_quality(1, cx);
    });
    it.cx.run_until_parked();

    it.type_text("136.45");
    it.press_enter();
    assert_eq!(it.parameters().quality, R8brainQuality::Medium);

    it.type_text("137");
    it.press_enter();
    assert!(matches!(it.parameters().quality, R8brainQuality::Custom(_)));
}

#[gpui_kit::test]
fn what_leaves_the_sound_alone_leaves_the_preset_too(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::MaxInputFrames, cx);
    it.type_text("4096");
    it.press_enter();
    assert_eq!(it.parameters().quality, R8brainQuality::High);
}

#[gpui_kit::test]
fn reset_goes_back_to_the_default(cx: &mut TestAppContext) {
    let mut it = fixture(R8brainNumber::FilterBankCacheSize, cx);
    it.field
        .read_with(it.cx, |field, cx| assert!(!field.is_resettable(cx)));
    it.type_text("3");
    it.press_enter();
    it.field
        .read_with(it.cx, |field, cx| assert!(field.is_resettable(cx)));

    it.field
        .update_in(it.cx, |field, window, cx| field.reset(window, cx));
    assert_eq!(it.parameters().performance.filter_bank_cache_size, 12);
    assert_eq!(it.shown(), "12");
}

#[gpui_kit::test]
fn numbers_that_differ_between_presets_leave_the_reset_to_the_preset(cx: &mut TestAppContext) {
    for number in [R8brainNumber::Attenuation, R8brainNumber::TransitionBand] {
        let mut it = fixture(number, cx);
        it.select_preset(0);
        it.field
            .read_with(it.cx, |field, cx| assert!(!field.is_resettable(cx)));
        it.type_text("10");
        it.press_enter();
        it.field
            .read_with(it.cx, |field, cx| assert!(!field.is_resettable(cx)));
    }
}

#[test]
fn every_number_of_the_defaults_is_in_its_range() {
    let parameters = R8brainParameters::default();
    for number in [
        R8brainNumber::Attenuation,
        R8brainNumber::TransitionBand,
        R8brainNumber::MaxInputFrames,
        R8brainNumber::FilterCacheSize,
        R8brainNumber::FilterBankCacheSize,
    ] {
        let value = number.value(&parameters).unwrap();
        assert_eq!(number.range().parse(&value.to_string()), Some(value));
    }
}
