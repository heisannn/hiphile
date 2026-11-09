use gpui_kit::{Modifiers, MouseButton, Pixels, Point, TestAppContext, VisualTestContext, point};

use super::*;

struct Fixture<'a> {
    dsp: DspSettings,
    slider: Entity<BalanceSlider>,
    cx: &'a mut VisualTestContext,
}

fn fixture(cx: &mut TestAppContext) -> Fixture<'_> {
    cx.update(|cx| {
        gpui_kit::init(cx);
        bind_keys(cx);
    });
    let executor = cx.update(|cx| gpui_host::executor(cx));
    let dsp = DspSettings::new(executor, Err(config::Error::NoHomeDir));
    let held = dsp.clone();
    let (slider, cx) = cx.add_window_view(|_, cx| BalanceSlider::new(held, cx));
    cx.run_until_parked();
    Fixture { dsp, slider, cx }
}

impl Fixture<'_> {
    fn shown(&mut self) -> f32 {
        self.slider
            .read_with(self.cx, |slider, cx| slider.slider.read(cx).value().end())
    }

    fn balance(&self) -> (f64, String) {
        let snapshot = self.dsp.snapshot();
        (snapshot.balance_db(), snapshot.balance_text())
    }

    /// The point on the line `fraction` of the way from its left end
    fn on_line(&mut self, fraction: f32) -> Point<Pixels> {
        let line = self.cx.debug_bounds("balance-line").unwrap();
        point(line.left() + line.size.width * fraction, line.center().y)
    }

    fn press(&mut self, keys: &str) {
        self.slider.update_in(self.cx, |slider, window, cx| {
            slider.focus.focus(window, cx);
        });
        self.cx.simulate_keystrokes(keys);
        self.cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn a_key_moves_the_balance_by_half_a_db_and_home_and_end_go_to_the_ends(cx: &mut TestAppContext) {
    let mut it = fixture(cx);
    it.press("left");
    assert_eq!(it.balance(), (-0.5, "Right -0.5 dB".to_owned()));
    assert_eq!(it.shown(), -0.5);

    it.press("right right");
    assert_eq!(it.balance(), (0.5, "Left -0.5 dB".to_owned()));

    it.press("home");
    assert_eq!(it.balance(), (-12.0, "Right -12.0 dB".to_owned()));
    it.press("end");
    assert_eq!(it.balance(), (12.0, "Left -12.0 dB".to_owned()));
    assert_eq!(it.shown(), 12.0);
}

#[gpui_kit::test]
fn the_pointer_sets_the_balance_where_it_drags_the_thumb_and_puts_it(cx: &mut TestAppContext) {
    let mut it = fixture(cx);
    let thumb = it.cx.debug_bounds("balance-thumb").unwrap().center();
    let three_quarters = it.on_line(0.75);
    it.cx.simulate_mouse_move(thumb, None, Modifiers::default());
    it.cx
        .simulate_mouse_down(thumb, MouseButton::Left, Modifiers::default());
    // The first move starts the drag and the second is the first it hears of
    it.cx
        .simulate_mouse_move(three_quarters, MouseButton::Left, Modifiers::default());
    it.cx
        .simulate_mouse_move(three_quarters, MouseButton::Left, Modifiers::default());
    it.cx
        .simulate_mouse_up(three_quarters, MouseButton::Left, Modifiers::default());
    it.cx.run_until_parked();
    assert_eq!(it.balance(), (6.0, "Left -6.0 dB".to_owned()));

    // Taking the thumb gives the slider the keys
    it.cx.simulate_keystrokes("left");
    it.cx.run_until_parked();
    assert_eq!(it.balance(), (5.5, "Left -5.5 dB".to_owned()));

    let quarter = it.on_line(0.25);
    it.cx
        .simulate_mouse_down(quarter, MouseButton::Left, Modifiers::default());
    it.cx
        .simulate_mouse_up(quarter, MouseButton::Left, Modifiers::default());
    it.cx.run_until_parked();
    assert_eq!(it.balance(), (-6.0, "Right -6.0 dB".to_owned()));
}

#[gpui_kit::test]
fn reset_centers_the_balance_and_the_thumb(cx: &mut TestAppContext) {
    let mut it = fixture(cx);
    it.press("left left left");
    it.slider
        .read_with(it.cx, |slider, _| assert!(slider.is_resettable()));

    it.slider
        .update_in(it.cx, |slider, window, cx| slider.reset(window, cx));

    assert_eq!(it.shown(), 0.0);
    assert_eq!(it.balance(), (0.0, "Center".to_owned()));
    it.slider
        .read_with(it.cx, |slider, _| assert!(!slider.is_resettable()));
    it.cx.run_until_parked();
}
