use anti_webui::core::Ui;
use anti_webui::core::access::{AccessAction, perform};
use anti_webui::core::event::NamedKey;
use next_ui::testing::{TestCore, click, core, find, key, nodes, press, start, wait_until};

use super::*;

struct Fixture {
    dsp: DspSettings,
    ui: Ui,
    _core: TestCore,
}

fn fixture() -> Fixture {
    let (core, hiphile) = core("");
    let dsp = hiphile.settings().dsp().clone();
    let ui = start({
        let dsp = dsp.clone();
        move || balance_slider(&dsp)
    });
    Fixture {
        dsp,
        ui,
        _core: core,
    }
}

impl Fixture {
    fn balance(&self) -> (f64, String) {
        let snapshot = self.dsp.snapshot();
        (snapshot.balance_db(), snapshot.balance_text())
    }

    /// Where the thumb is, as the slider tells assistive technology.
    fn shown(&self) -> f64 {
        nodes(Role::Slider)[0].value.as_ref().expect("a value").now
    }

    fn press(&mut self, keys: &[NamedKey]) {
        perform(nodes(Role::Slider)[0].id, AccessAction::Focus);
        self.ui.update();
        for &named in keys {
            key(&mut self.ui, named);
        }
    }

    /// Presses the track `fraction` of the way from its left end.
    fn click_along(&mut self, fraction: f32) {
        let bounds = find(Role::Group, NAME).expect("the balance").bounds;
        let inset = 2.0;
        let x = bounds.x as f32 + inset + (240.0 - 2.0 * inset) * fraction;
        let y = bounds.y as f32 + bounds.height as f32 / 2.0;
        click(&mut self.ui, x, y);
    }
}

#[test]
fn a_key_moves_the_balance_by_half_a_db_and_home_and_end_go_to_the_ends() {
    let mut it = fixture();
    it.press(&[NamedKey::ArrowLeft]);
    assert_eq!(it.balance(), (-0.5, "Right -0.5 dB".to_owned()));
    assert_eq!(it.shown(), -0.5);

    it.press(&[NamedKey::ArrowRight, NamedKey::ArrowRight]);
    assert_eq!(it.balance(), (0.5, "Left -0.5 dB".to_owned()));
    it.press(&[NamedKey::ArrowDown]);
    assert_eq!(it.balance(), (0.0, "Center".to_owned()));

    it.press(&[NamedKey::Home]);
    assert_eq!(it.balance(), (-12.0, "Right -12.0 dB".to_owned()));
    it.press(&[NamedKey::ArrowLeft]);
    assert_eq!(it.balance(), (-12.0, "Right -12.0 dB".to_owned()));
    it.press(&[NamedKey::End]);
    assert_eq!(it.balance(), (12.0, "Left -12.0 dB".to_owned()));
    assert_eq!(it.shown(), 12.0);
}

#[test]
fn the_pointer_sets_the_balance_where_it_presses_and_a_key_goes_on_from_there() {
    let mut it = fixture();
    it.click_along(0.75);
    assert_eq!(it.balance(), (6.0, "Left -6.0 dB".to_owned()));
    assert!(next_ui::testing::has_text("Left -6.0 dB"));

    // Taking the thumb gives the slider the keys
    key(&mut it.ui, NamedKey::ArrowLeft);
    assert_eq!(it.balance(), (5.5, "Left -5.5 dB".to_owned()));

    it.click_along(0.0);
    assert_eq!(it.balance(), (-12.0, "Right -12.0 dB".to_owned()));
    key(&mut it.ui, NamedKey::ArrowLeft);
    assert_eq!(it.balance(), (-12.0, "Right -12.0 dB".to_owned()));
}

#[test]
fn reset_centers_the_balance_and_the_thumb() {
    let mut it = fixture();
    assert!(find(Role::Button, "Reset Balance").is_none());
    it.press(&[NamedKey::ArrowLeft; 3]);

    press(&mut it.ui, Role::Button, "Reset Balance");

    assert_eq!(it.balance(), (0.0, "Center".to_owned()));
    wait_until(&mut it.ui, it_is_centered);
    assert!(find(Role::Button, "Reset Balance").is_none());
}

fn it_is_centered() -> bool {
    nodes(Role::Slider)[0]
        .value
        .as_ref()
        .is_some_and(|v| v.now == 0.0)
}
