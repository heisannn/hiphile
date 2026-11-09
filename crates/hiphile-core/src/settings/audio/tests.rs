use gpui_kit::TestAppContext;

use super::*;

/// How many times each change that the app passes on has been made, which a `watch` that keeps
/// only the last cannot count.
#[derive(Default)]
pub(in crate::settings) struct Told {
    pub(super) output: u32,
    pub(super) transitions: u32,
}

/// How many times the output has changed.
pub(in crate::settings) fn told(settings: &AudioSettings) -> u32 {
    settings.0.state.read(|state| state.told.output)
}

fn told_transitions(settings: &AudioSettings) -> u32 {
    settings.0.state.read(|state| state.told.transitions)
}

pub(in crate::settings) const DAC: &str = "coreaudio:dac";

/// Takes every DSD rate of both families.
pub(in crate::settings) fn dac() -> OutputDevice {
    OutputDevice::listed(
        DAC,
        "DAC",
        &[44_100, 48_000, 88_200, 96_000, 176_400, 192_000],
        &[176_400, 192_000, 352_800, 384_000, 705_600, 768_000],
    )
}

/// On GPUI's executor, which the test's `run_until_parked` drives.
pub(in crate::settings) fn settings_saving_to(
    store: &config::Store,
    cx: &mut TestAppContext,
) -> AudioSettings {
    let quality = ResamplingQuality::default();
    let dsd = DsdOutput::default();
    let executor = cx.update(|cx| gpui_host::executor(cx));
    AudioSettings::with(
        executor,
        Some(store.clone()),
        OutputSettings::from_config(None, SampleRate::MatchSong, quality, &dsd),
        quality,
        dsd,
        None,
    )
}

#[gpui_kit::test]
fn a_preset_is_told_saved_and_reaches_the_output(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);

    settings.select_rubato_quality(2);
    cx.run_until_parked();

    assert_eq!(told(&settings), 1);
    assert_eq!(store.resampling_quality().unwrap(), ResamplingQuality::Max);
    let settings = settings.snapshot();
    assert_eq!(settings.rubato_quality_ix(), 2);
    assert_eq!(
        settings.output().resampling(),
        audio::Resampling::new(4096, audio::ResamplingWindow::BlackmanHarris2)
    );
}

#[gpui_kit::test]
fn r8brain_is_told_saved_and_reaches_the_output(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);

    settings.select_resampler(1);
    cx.run_until_parked();

    assert_eq!(told(&settings), 1);
    assert_eq!(
        store.resampling_quality().unwrap(),
        ResamplingQuality::R8brain(R8brainParameters::default())
    );
    let settings = settings.snapshot();
    assert_eq!(settings.resampler_ix(), 1);
    assert!(settings.uses_r8brain());
    assert_eq!(
        settings.output().resampling(),
        audio::Resampling::R8brain(audio::R8brainSettings::default())
    );
}

#[gpui_kit::test]
fn r8brain_values_from_elsewhere_stay_out_of_the_file(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let mut filter = config::R8brainQuality::High.filter();
    filter.attenuation = 100.0;
    let mut from_environment = R8brainParameters::default();
    from_environment.quality = config::R8brainQuality::Custom(filter);
    let quality = ResamplingQuality::R8brain(from_environment);
    let dsd = DsdOutput::default();
    let output = OutputSettings::from_config(None, SampleRate::MatchSong, quality, &dsd);
    let executor = cx.update(|cx| gpui_host::executor(cx));
    let settings = AudioSettings::with(executor, Some(store.clone()), output, quality, dsd, None);

    settings.type_r8brain(R8brainNumber::MaxInputFrames, "4096");
    settings.commit_r8brain(R8brainNumber::MaxInputFrames);
    cx.run_until_parked();

    let text = std::fs::read_to_string(store.config_path()).unwrap();
    assert!(text.contains("r8brain_max_input_frames = 4096"), "{text}");
    assert!(!text.contains("r8brain_attenuation"), "{text}");
}

#[gpui_kit::test]
fn each_resampler_comes_back_at_the_quality_it_had(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_rubato_quality(1);
    settings.select_resampler(1);
    settings.select_r8brain_quality(0);
    settings.select_resampler(0);
    cx.run_until_parked();
    assert_eq!(store.resampling_quality().unwrap(), ResamplingQuality::High);
    let snapshot = settings.snapshot();
    assert_eq!(snapshot.resampler_ix(), 0);
    assert_eq!(snapshot.rubato_quality_ix(), 1);

    settings.select_resampler(1);
    assert_eq!(settings.snapshot().r8brain_quality_ix(), 0);
}

#[gpui_kit::test]
fn custom_after_r8brain_starts_from_the_medium_values(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(1);

    settings.select_rubato_quality(3);
    cx.run_until_parked();

    assert_eq!(
        store.resampling_quality().unwrap(),
        ResamplingQuality::Custom {
            fft_size: 256,
            window: ResamplingWindow::BlackmanHarris2,
        }
    );
    let settings = settings.snapshot();
    assert!(!settings.uses_r8brain());
    assert_eq!(
        settings.output().resampling(),
        audio::Resampling::new(256, audio::ResamplingWindow::BlackmanHarris2)
    );
}

#[gpui_kit::test]
fn choosing_custom_is_saved_but_keeps_the_sound(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_rubato_quality(1);
    cx.run_until_parked();
    assert_eq!(told(&settings), 1);

    settings.select_rubato_quality(3);
    cx.run_until_parked();

    assert_eq!(told(&settings), 1);
    let custom = ResamplingQuality::Custom {
        fft_size: 1024,
        window: ResamplingWindow::BlackmanHarris2,
    };
    assert_eq!(store.resampling_quality().unwrap(), custom);
    assert_eq!(settings.snapshot().rubato_quality_ix(), 3);
}

#[gpui_kit::test]
fn another_value_turns_a_preset_custom_and_the_same_one_does_not(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_rubato_quality(1);
    cx.run_until_parked();
    assert_eq!(told(&settings), 1);

    settings.set_fft_size("1024");
    settings.set_window("Blackman-Harris²");
    cx.run_until_parked();
    assert_eq!(told(&settings), 1);
    assert_eq!(store.resampling_quality().unwrap(), ResamplingQuality::High);
    assert_eq!(settings.snapshot().rubato_quality_ix(), 1);

    settings.set_window("Hann");
    cx.run_until_parked();
    assert_eq!(told(&settings), 2);
    assert_eq!(
        store.resampling_quality().unwrap(),
        ResamplingQuality::Custom {
            fft_size: 1024,
            window: ResamplingWindow::Hann,
        }
    );
    let snapshot = settings.snapshot();
    assert_eq!(snapshot.rubato_quality_ix(), 3);
    assert_eq!(snapshot.fft_size_value(), "1024");

    settings.select_rubato_quality(2);
    let snapshot = settings.snapshot();
    assert_eq!(snapshot.fft_size_value(), "4096");
    assert_eq!(snapshot.window_value(), "Blackman-Harris²");
}

#[gpui_kit::test]
fn custom_values_are_told_saved_and_reach_the_output(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_rubato_quality(3);

    settings.set_fft_size("8192");
    settings.set_window("Hann²");
    cx.run_until_parked();

    assert_eq!(told(&settings), 2);
    assert_eq!(
        store.resampling_quality().unwrap(),
        ResamplingQuality::Custom {
            fft_size: 8192,
            window: ResamplingWindow::Hann2,
        }
    );
    let settings = settings.snapshot();
    assert_eq!(
        settings.output().resampling(),
        audio::Resampling::new(8192, audio::ResamplingWindow::Hann2)
    );
    assert_eq!(settings.fft_size_value(), "8192");
    assert_eq!(settings.window_value(), "Hann²");
}

#[gpui_kit::test]
fn another_device_or_rate_keeps_the_resampling(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_rubato_quality(1);

    settings.set_device("a:1");
    settings.set_rate("48000");
    cx.run_until_parked();

    assert_eq!(
        settings.snapshot().output().resampling(),
        audio::Resampling::new(1024, audio::ResamplingWindow::BlackmanHarris2)
    );
}

#[gpui_kit::test]
fn transitions_are_told_saved_and_leave_the_output_alone(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);

    settings.set_trim_silence(true);
    settings.set_trim_silence(true);
    settings.set_crossfade("5");
    settings.set_crossfade("5");
    settings.set_play_pause_fade(true);
    settings.set_play_pause_fade(true);
    cx.run_until_parked();
    assert_eq!(told_transitions(&settings), 3);
    assert_eq!(told(&settings), 0);
    assert!(store.trim_silence().unwrap());
    assert_eq!(store.crossfade_seconds().unwrap(), 5);
    assert!(store.play_pause_fade().unwrap());
    let snapshot = settings.snapshot();
    assert_eq!(snapshot.crossfade_value(), "5");
    assert!(snapshot.is_trim_silence_on());
    assert_eq!(snapshot.transitions().crossfade, Duration::from_secs(5));

    settings.set_trim_silence(false);
    settings.set_crossfade("0");
    settings.set_play_pause_fade(false);
    cx.run_until_parked();
    assert!(!store.trim_silence().unwrap());
    assert_eq!(store.crossfade_seconds().unwrap(), 0);
    assert!(!store.play_pause_fade().unwrap());
}

#[test]
fn saved_transitions_come_back_and_bad_ones_are_told() {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    store.save_trim_silence(true).unwrap();
    store.save_crossfade_seconds(3).unwrap();
    store.save_play_pause_fade(true).unwrap();
    let (transitions, error) = read_saved_transitions(Some(&store));
    assert!(transitions.trim_silence);
    assert_eq!(transitions.crossfade, Duration::from_secs(3));
    assert!(transitions.fades_play_and_pause);
    assert!(error.is_none());

    store
        .save_crossfade_seconds(config::LONGEST_CROSSFADE_SECONDS + 1)
        .unwrap();
    let (transitions, error) = read_saved_transitions(Some(&store));
    assert_eq!(transitions, Transitions::default());
    assert!(error.is_some());
}

#[gpui_kit::test]
fn the_crossfade_note_says_when_the_device_may_have_to_switch(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    assert!(settings.snapshot().crossfade_note().is_none());
    settings.set_crossfade("5");
    assert!(settings.snapshot().crossfade_note().is_some());
    settings.set_rate("48000");
    assert!(settings.snapshot().crossfade_note().is_none());
}

#[gpui_kit::test]
fn exclusive_mode_is_told_saved_and_survives_other_changes(cx: &mut TestAppContext) {
    if !audio::CAN_PLAY_EXCLUSIVE {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);

    settings.set_exclusive(true);
    settings.set_exclusive(true);
    cx.run_until_parked();
    assert_eq!(told(&settings), 1);
    assert!(store.exclusive_mode().unwrap());

    settings.pretend_listed(vec![dac()]);
    settings.set_device(DAC);
    settings.set_rate("48000");
    settings.select_rubato_quality(2);
    cx.run_until_parked();
    assert!(settings.snapshot().output().is_exclusive());

    settings.turn_dsd_on();
    cx.run_until_parked();
    assert!(settings.snapshot().output().is_exclusive());
    settings.turn_dsd_off();

    settings.set_exclusive(false);
    cx.run_until_parked();
    assert!(!store.exclusive_mode().unwrap());
    assert!(!settings.snapshot().is_exclusive_on());
}

#[gpui_kit::test]
fn each_change_is_published_at_once(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    let mut changes = settings.subscribe();

    settings.set_rate("48000");
    assert!(changes.has_changed().unwrap());
    assert_eq!(
        changes.borrow_and_update().output().sample_rate(),
        SampleRate::Fixed(48_000)
    );
}

#[gpui_kit::test]
fn changes_waiting_for_a_write_are_written_together(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);

    settings.set_rate("48000");
    settings.set_trim_silence(true);
    settings.set_soxr_coef_table("1600");
    settings.set_soxr_min_dft("4096");
    cx.run_until_parked();

    assert_eq!(store.sample_rate().unwrap(), SampleRate::Fixed(48_000));
    assert!(store.trim_silence().unwrap());
    let performance = store.soxr_parameters().unwrap().performance;
    assert_eq!(performance.coef_table_kb, 1600);
    assert_eq!(performance.min_dft_size, 4096);
    let text = std::fs::read_to_string(store.config_path()).unwrap();
    assert!(!text.contains("output_device"), "{text}");
    assert!(!text.contains("soxr_precision"), "{text}");
}

fn labels(options: &[Choice]) -> Vec<&str> {
    options.iter().map(Choice::label).collect()
}

fn values(options: &[Choice]) -> Vec<&str> {
    options.iter().map(Choice::value).collect()
}

#[test]
fn system_default_comes_first_and_devices_keep_their_ids() {
    let options = device_options(
        [("a:1", "Speakers"), ("a:2", "DAC")].into_iter(),
        None,
        true,
    );
    assert_eq!(labels(&options), ["System default", "Speakers", "DAC"]);
    assert_eq!(values(&options), [SYSTEM_DEFAULT, "a:1", "a:2"]);
}

#[test]
fn a_chosen_device_that_is_gone_stays_an_option() {
    let options = device_options([("a:1", "Speakers")].into_iter(), Some("a:2"), true);
    assert_eq!(
        labels(&options),
        ["System default", "Speakers", "Disconnected device"]
    );
    assert_eq!(values(&options)[2], "a:2");
}

#[test]
fn a_chosen_device_is_not_called_gone_before_the_first_listing() {
    let options = device_options(std::iter::empty(), Some("a:2"), false);
    assert_eq!(labels(&options), ["System default", "Loading"]);
}

#[test]
fn rates_follow_the_device() {
    let options = rate_options(Some(&[44_100, 96_000]), SampleRate::MatchSong, true);
    assert_eq!(
        labels(&options),
        ["Match each song", "x2", "44.1 kHz", "96 kHz"]
    );
    assert_eq!(values(&options), [MATCH_SONG, "x2", "44100", "96000"]);
}

#[test]
fn a_fixed_rate_is_not_called_unsupported_without_a_device() {
    let options = rate_options(None, SampleRate::Fixed(48_000), true);
    assert_eq!(labels(&options), ["Match each song", "48 kHz"]);
}

#[test]
fn a_fixed_rate_the_device_lacks_stays_an_option() {
    let options = rate_options(Some(&[44_100]), SampleRate::Fixed(192_000), true);
    assert_eq!(
        labels(&options),
        ["Match each song", "44.1 kHz", "192 kHz (not supported)"]
    );
    assert_eq!(values(&options)[2], "192000");
}

#[test]
fn a_multiple_is_offered_when_either_family_reaches_it() {
    let options = rate_options(
        Some(&[48_000, 176_400, 192_000]),
        SampleRate::MatchSong,
        true,
    );
    assert_eq!(
        values(&options),
        [MATCH_SONG, "x4", "48000", "176400", "192000"]
    );
}

#[test]
fn a_multiple_the_device_lacks_stays_an_option() {
    let options = rate_options(Some(&[44_100]), SampleRate::SongMultiple(4), true);
    assert_eq!(
        labels(&options),
        ["Match each song", "44.1 kHz", "x4 (not supported)"]
    );
    assert_eq!(values(&options)[2], "x4");
}

#[test]
fn a_multiple_is_not_offered_where_it_cannot_reach_the_device() {
    let rates: &[u32] = &[44_100, 96_000];
    let options = rate_options(Some(rates), SampleRate::MatchSong, false);
    assert_eq!(values(&options), [MATCH_SONG, "44100", "96000"]);

    let options = rate_options(Some(rates), SampleRate::SongMultiple(2), false);
    assert_eq!(
        labels(&options),
        [
            "Match each song",
            "44.1 kHz",
            "96 kHz",
            "x2 (not supported)"
        ]
    );
}

#[gpui_kit::test]
fn a_multiple_is_told_saved_and_reaches_the_output(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);

    settings.set_rate("x4");
    cx.run_until_parked();

    assert_eq!(told(&settings), 1);
    assert_eq!(store.sample_rate().unwrap(), SampleRate::SongMultiple(4));
    let snapshot = settings.snapshot();
    assert_eq!(snapshot.output().sample_rate(), SampleRate::SongMultiple(4));
    assert_eq!(snapshot.rate_value(), "x4");

    settings.set_rate(MATCH_SONG);
    cx.run_until_parked();
    assert_eq!(store.sample_rate().unwrap(), SampleRate::MatchSong);
}

#[gpui_kit::test]
fn what_waits_and_what_is_typed_are_written_before_quitting(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);

    settings.set_rate("48000");
    settings.set_trim_silence(true);
    settings.type_r8brain(R8brainNumber::MaxInputFrames, "4096");
    settings.save_before_quitting();

    assert_eq!(store.sample_rate().unwrap(), SampleRate::Fixed(48_000));
    assert!(store.trim_silence().unwrap());
    let parameters = store.r8brain_parameters().unwrap();
    assert_eq!(parameters.performance.max_input_frames, 4096);
    assert_eq!(
        settings
            .snapshot()
            .r8brain_text(R8brainNumber::MaxInputFrames),
        "4096"
    );
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_number_beyond_its_range_is_written_held_to_it_before_quitting(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);

    settings.type_r8brain(R8brainNumber::Attenuation, "1");
    settings.save_before_quitting();

    let filter = store.r8brain_parameters().unwrap().quality.filter();
    assert_eq!(filter.attenuation, 49.0);
}

#[gpui_kit::test]
fn with_dsd_on_the_crossfade_note_says_dsd_plays_songs_whole(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.pretend_listed(vec![dac()]);
    settings.set_device(DAC);
    settings.set_crossfade("5");
    let note = settings.snapshot().crossfade_note();
    assert!(note.is_some());

    settings.turn_dsd_on();

    let snapshot = settings.snapshot();
    assert_ne!(snapshot.crossfade_note(), note);
    assert_eq!(
        snapshot.crossfade_note(),
        snapshot.dsd_note(PcmSetting::Crossfade)
    );
}
