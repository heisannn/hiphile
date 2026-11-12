use anti_webui::prelude::Role;
use hiphile_core::settings::R8brainSetting;
use next_ui::testing::{core, find, frames, has_text, is_modal_open, press, start, wait_until};

use super::*;

/// DSD on for a device that the settings name; nothing here plays.
const DSD_ON: &str = "output_device = \"coreaudio:Dac\"\noutput_format = \"dsd\"\n\
                      dop_device = \"coreaudio:Dac\"\n";

fn heading(text: &str) -> bool {
    find(Role::Heading, text).is_some()
}

#[test]
fn each_resampler_shows_its_own_settings() {
    let (_core, hiphile) = core("");
    let audio = hiphile.settings().audio().clone();
    let mut ui = start({
        let audio = audio.clone();
        move || audio_page(&audio)
    });
    assert!(find(Role::Radio, "Medium").is_some());
    assert!(find(Role::ComboBox, "FFT size").is_some());
    assert!(!heading("r8brain filter"));

    press(&mut ui, Role::Radio, "r8brain");
    assert!(audio.snapshot().uses_r8brain());
    wait_until(&mut ui, || heading("r8brain filter"));
    assert!(heading("r8brain performance"));
    assert!(find(Role::TextInput, "r8brain attenuation").is_some());
    assert!(find(Role::ComboBox, "FFT size").is_none());
    assert!(has_text(
        "Higher keeps more treble and removes more unwanted sound, with more delay."
    ));

    press(&mut ui, Role::Radio, "soxr");
    wait_until(&mut ui, || heading("soxr filter"));
    assert!(!heading("r8brain filter"));
    assert!(find(Role::ComboBox, "Precision").is_some());
    assert!(find(Role::RadioGroup, "Coefficient interpolation").is_some());
}

#[test]
fn a_changed_resampler_setting_can_be_reset() {
    let (_core, hiphile) = core("");
    let audio = hiphile.settings().audio().clone();
    audio.select_resampler(1);
    let mut ui = start({
        let audio = audio.clone();
        move || audio_page(&audio)
    });
    assert!(find(Role::Button, "Reset r8brain quality").is_none());

    press(&mut ui, Role::Radio, "Max");
    wait_until(&mut ui, || {
        find(Role::Button, "Reset r8brain quality").is_some()
    });
    press(&mut ui, Role::Button, "Reset r8brain quality");

    assert!(
        !audio
            .snapshot()
            .is_r8brain_resettable(R8brainSetting::Quality)
    );
    wait_until(&mut ui, || {
        find(Role::Button, "Reset r8brain quality").is_none()
    });
}

#[test]
fn dsd_says_why_it_cannot_turn_on() {
    if !AudioSettings::is_dsd_offered() {
        return;
    }
    let (_core, hiphile) = core("");
    let audio = hiphile.settings().audio().clone();
    let obstacle = audio.snapshot().dsd_obstacle().expect("no device chosen");
    let _ui = start(move || audio_page(&audio));

    assert!(has_text(obstacle));
    assert!(find(Role::Radio, "DSD (DoP)").unwrap().disabled);
    assert!(!heading("DSD filter"));
}

#[test]
fn while_dsd_is_on_the_pcm_settings_say_why_they_do_nothing() {
    if !AudioSettings::is_dsd_offered() {
        return;
    }
    let (_core, hiphile) = core(DSD_ON);
    let audio = hiphile.settings().audio().clone();
    audio.select_resampler(1);
    assert!(audio.snapshot().is_dsd_on());
    let _ui = start(move || audio_page(&audio));

    for note in [
        "DSD sets the rate while it's on.",
        "DSD cuts in and out at once while it's on.",
        "DSD uses the DSD filter settings below while it's on.",
    ] {
        assert!(has_text(note), "{note}");
    }
    assert!(find(Role::ComboBox, "Sample rate").unwrap().disabled);
    assert!(find(Role::Switch, "Fade").unwrap().disabled);
    assert!(find(Role::Radio, "Rubato").unwrap().disabled);
    assert!(!heading("r8brain filter"));
    assert!(heading("DSD filter"));
    assert!(heading("DSD modulator"));
    assert!(find(Role::ComboBox, "DSD rate").is_some());
}

#[test]
fn the_base_rate_asks_before_it_changes() {
    if !AudioSettings::is_dsd_offered() {
        return;
    }
    let (_core, hiphile) = core(DSD_ON);
    let audio = hiphile.settings().audio().clone();
    let mut ui = start({
        let audio = audio.clone();
        move || audio_page(&audio)
    });

    press(&mut ui, Role::Radio, "Match each song");
    assert!(has_text("Match each song's base rate?"));
    press(&mut ui, Role::Button, "Cancel");
    assert!(!audio.snapshot().matches_song_family());
    frames(&mut ui, 1000);
    assert!(!is_modal_open());

    press(&mut ui, Role::Radio, "Match each song");
    press(&mut ui, Role::Button, "Match");
    assert!(audio.snapshot().matches_song_family());
}

#[test]
fn turning_dsd_off_asks_nothing() {
    if !AudioSettings::is_dsd_offered() {
        return;
    }
    let (_core, hiphile) = core(DSD_ON);
    let audio = hiphile.settings().audio().clone();
    let mut ui = start({
        let audio = audio.clone();
        move || audio_page(&audio)
    });

    press(&mut ui, Role::Radio, "PCM");
    assert!(!audio.snapshot().is_dsd_on());
    wait_until(&mut ui, || !heading("DSD filter"));
}
