use config::{
    DsdFilter, DsdOutput, DsdParameters, DsdRate, R8brainParameters, R8brainPhase,
    ResamplingQuality, SampleRate, SoxrInterpolation, SoxrParameters, SoxrPhase,
};

use crate::output::{
    self, OutputSettings, R8brainSettings, Resampling, ResamplingWindow, SoxrSettings,
};

impl OutputSettings {
    /// What the settings in `config.toml` come to. DSD applies only on the
    /// device it was turned on for.
    pub fn from_config(
        device: Option<String>,
        sample_rate: SampleRate,
        quality: ResamplingQuality,
        dsd: &DsdOutput,
    ) -> Self {
        let dop = (dsd.is_on && dsd.device == device).then(|| dsd_settings(dsd));
        Self::new(device, None)
            .with_sample_rate(sample_rate)
            .with_resampling(match quality {
                ResamplingQuality::R8brain(parameters) => {
                    Resampling::R8brain(r8brain_settings(parameters))
                }
                ResamplingQuality::Soxr(parameters) => Resampling::Soxr(soxr_settings(parameters)),
                _ => Resampling::new(quality.fft_size(), window_of(quality.window())),
            })
            .with_dop(dop)
            .with_dop_fade_in(dsd.fades_in)
    }
}

fn r8brain_settings(parameters: R8brainParameters) -> R8brainSettings {
    let filter = parameters.quality.filter();
    let performance = parameters.performance;
    let mut settings = R8brainSettings::default();
    let options = &mut settings.options;
    options.transition_band = filter.transition_band;
    options.attenuation = filter.attenuation;
    options.phase = match filter.phase {
        R8brainPhase::Linear => output::R8brainPhase::Linear,
        R8brainPhase::Minimum => output::R8brainPhase::Minimum,
    };
    options.max_input_len = performance.max_input_frames as usize;
    options.extended_fft = performance.extended_fft;
    options.whole_stepping = filter.whole_stepping;
    options.filter_fractions = filter.filter_fractions;
    settings.cache_limits.filters = performance.filter_cache_size as usize;
    settings.cache_limits.filter_banks = performance.filter_bank_cache_size as usize;
    settings
}

/// A preset comes to libsoxr's own but for the rolloff, which changes nothing in `soxr`, so that
/// a custom filter with a preset's values sounds as it did.
fn soxr_settings(parameters: SoxrParameters) -> SoxrSettings {
    let filter = parameters.quality.filter();
    let performance = parameters.performance;
    let mut settings = SoxrSettings::default();
    let quality = &mut settings.quality;
    quality.precision = filter.precision;
    quality.passband_end = filter.passband_end;
    quality.phase_response = match filter.phase {
        SoxrPhase::Linear => soxr::Phase::Linear,
        SoxrPhase::Intermediate => soxr::Phase::Intermediate,
        SoxrPhase::Minimum => soxr::Phase::Minimum,
    }
    .response();
    quality.double_precision = filter.double_precision;
    let runtime = &mut settings.runtime;
    runtime.coef_size_kbytes = performance.coef_table_kb as usize;
    runtime.coef_interpolation = match performance.coef_interpolation {
        SoxrInterpolation::Auto => soxr::CoefInterpolation::Auto,
        SoxrInterpolation::Linear => soxr::CoefInterpolation::Low,
        SoxrInterpolation::Cubic => soxr::CoefInterpolation::High,
    };
    // soxr takes 0, which is out of its range, as its default
    let log2 = |size: u32| size.checked_ilog2().unwrap_or(0);
    runtime.log2_min_dft_size = log2(performance.min_dft_size);
    runtime.log2_large_dft_size = log2(performance.large_dft_size);
    settings
}

fn dsd_settings(output: &DsdOutput) -> dsd::Settings {
    let parameters = output.quality.parameters();
    let rate = match output.rate {
        Some(DsdRate::Dsd64) => dsd::DopRate::Dsd64,
        Some(DsdRate::Dsd128) => dsd::DopRate::Dsd128,
        Some(DsdRate::Dsd256) | None => dsd::DopRate::Dsd256,
    };
    let can_widen_noise_band = rate == dsd::DopRate::Dsd256;
    let modulation = modulation_of(&parameters, can_widen_noise_band);
    dsd::Settings::new(rate, filter_of(&parameters), modulation)
        .with_matching_source_family(output.matches_song_family)
}

fn filter_of(parameters: &DsdParameters) -> dsd::Filter {
    match parameters.filter {
        DsdFilter::Sinc {
            length,
            window,
            cutoff,
        } => dsd::Filter::Sinc {
            length: match length {
                256 => dsd::SincLength::L256,
                512 => dsd::SincLength::L512,
                2048 => dsd::SincLength::L2048,
                _ => dsd::SincLength::L1024,
            },
            window: window_of(window),
            cutoff: cutoff.map(|cutoff| cutoff as f32),
        },
        DsdFilter::Fft { size, window } => dsd::Filter::Fft {
            size,
            window: window_of(window),
        },
    }
}

/// At lower rates a wider noise band raises the noise below 20 kHz by far more
/// than it could ever be worth.
fn modulation_of(parameters: &DsdParameters, can_widen_noise_band: bool) -> dsd::Modulation {
    dsd::Modulation::default()
        .with_order(match parameters.order {
            3 => dsd::Order::Three,
            5 => dsd::Order::Five,
            9 => dsd::Order::Nine,
            _ => dsd::Order::Seven,
        })
        .with_noise_gain(parameters.noise_gain)
        .with_noise_band(if parameters.wide_noise_band && can_widen_noise_band {
            dsd::NoiseBand::To48Khz
        } else {
            dsd::NoiseBand::To20Khz
        })
        .with_optimized_zeros(parameters.optimized_zeros)
        .with_dither_db(parameters.dither_db)
        .with_input_level_db(parameters.input_level_db)
        .with_limiter_ceiling_db(parameters.limiter_ceiling_db)
}

fn window_of(window: config::ResamplingWindow) -> ResamplingWindow {
    match window {
        config::ResamplingWindow::Hann => ResamplingWindow::Hann,
        config::ResamplingWindow::Hann2 => ResamplingWindow::Hann2,
        config::ResamplingWindow::Blackman => ResamplingWindow::Blackman,
        config::ResamplingWindow::Blackman2 => ResamplingWindow::Blackman2,
        config::ResamplingWindow::BlackmanHarris => ResamplingWindow::BlackmanHarris,
        config::ResamplingWindow::BlackmanHarris2 => ResamplingWindow::BlackmanHarris2,
    }
}

#[cfg(test)]
mod tests {
    use config::{R8brainQuality, SoxrQuality};

    use super::*;

    fn dsd_for(device: &str) -> DsdOutput {
        let mut dsd = DsdOutput::default();
        dsd.is_on = true;
        dsd.device = Some(device.to_owned());
        dsd
    }

    #[test]
    fn dsd_applies_only_on_its_own_device() {
        let quality = ResamplingQuality::default();
        let dac = Some("dac".to_owned());
        let on_dac = OutputSettings::from_config(
            dac.clone(),
            SampleRate::MatchSong,
            quality,
            &dsd_for("dac"),
        );
        assert!(on_dac.dop().is_some());
        let elsewhere =
            OutputSettings::from_config(dac, SampleRate::MatchSong, quality, &dsd_for("other"));
        assert!(elsewhere.dop().is_none());
        let off = OutputSettings::from_config(
            Some("dac".to_owned()),
            SampleRate::MatchSong,
            quality,
            &DsdOutput::default(),
        );
        assert!(off.dop().is_none());
    }

    #[test]
    fn resampling_follows_the_quality() {
        let resampling_of = |quality| {
            OutputSettings::from_config(None, SampleRate::MatchSong, quality, &DsdOutput::default())
                .resampling()
        };
        assert_eq!(
            resampling_of(ResamplingQuality::R8brain(R8brainParameters::default())),
            Resampling::R8brain(R8brainSettings::default())
        );
        assert_eq!(
            resampling_of(ResamplingQuality::Max),
            Resampling::new(4096, ResamplingWindow::BlackmanHarris2)
        );
    }

    #[test]
    fn the_ends_of_the_r8brain_ranges_are_what_the_resampler_takes() {
        let lowest = || {
            let mut filter = R8brainQuality::High.filter();
            filter.transition_band = *config::R8BRAIN_TRANSITION_BANDS.start();
            filter.attenuation = *config::R8BRAIN_ATTENUATIONS.start();
            filter.filter_fractions = Some(*config::R8BRAIN_FILTER_FRACTIONS.start());
            let mut parameters = R8brainParameters::default();
            parameters.quality = R8brainQuality::Custom(filter);
            parameters.performance.max_input_frames = *config::R8BRAIN_MAX_INPUT_FRAMES.start();
            parameters.performance.filter_cache_size = *config::R8BRAIN_CACHE_SIZES.start();
            parameters.performance.filter_bank_cache_size = *config::R8BRAIN_CACHE_SIZES.start();
            parameters
        };
        let highest = || {
            let mut filter = R8brainQuality::High.filter();
            filter.transition_band = *config::R8BRAIN_TRANSITION_BANDS.end();
            filter.attenuation = *config::R8BRAIN_ATTENUATIONS.end();
            filter.filter_fractions = Some(*config::R8BRAIN_FILTER_FRACTIONS.end());
            let mut parameters = R8brainParameters::default();
            parameters.quality = R8brainQuality::Custom(filter);
            parameters.performance.max_input_frames = *config::R8BRAIN_MAX_INPUT_FRAMES.end();
            parameters.performance.filter_cache_size = *config::R8BRAIN_CACHE_SIZES.end();
            parameters.performance.filter_bank_cache_size = *config::R8BRAIN_CACHE_SIZES.end();
            parameters
        };
        for parameters in [lowest(), highest()] {
            let settings = r8brain_settings(parameters);
            settings.cache_limits.apply().unwrap();
            r8brain::Resampler::new(44_100.0, 47_999.5, settings.options).unwrap();
        }
        r8brain::CacheLimits::default().apply().unwrap();
    }

    fn r8brain_preset_options(quality: R8brainQuality) -> r8brain::Options {
        let mut parameters = R8brainParameters::default();
        parameters.quality = quality;
        r8brain_settings(parameters).options
    }

    const PRESETS: [R8brainQuality; 4] = [
        R8brainQuality::Low,
        R8brainQuality::Medium,
        R8brainQuality::High,
        R8brainQuality::Max,
    ];

    #[test]
    fn each_r8brain_preset_takes_more_delay_than_the_last() {
        for (source, target) in [
            (44_100.0, 48_000.0),
            (48_000.0, 44_100.0),
            (44_100.0, 96_000.0),
            (96_000.0, 44_100.0),
            (44_100.0, 88_200.0),
            (192_000.0, 48_000.0),
        ] {
            let delays = PRESETS.map(|quality| {
                r8brain::Resampler::new(source, target, r8brain_preset_options(quality))
                    .unwrap()
                    .input_before_output(0)
            });
            assert!(
                delays.windows(2).all(|pair| pair[0] < pair[1]),
                "{source} -> {target}: {delays:?}"
            );
        }
    }

    #[test]
    fn each_r8brain_preset_keeps_the_top_of_hearing_at_its_level() {
        const FREQUENCY: f64 = 20_000.0;
        for quality in PRESETS {
            let mut resampler =
                r8brain::Resampler::new(44_100.0, 48_000.0, r8brain_preset_options(quality))
                    .unwrap();
            let input: Vec<f64> = (0..66_150)
                .map(|frame| {
                    (std::f64::consts::TAU * FREQUENCY * f64::from(frame) / 44_100.0).sin()
                })
                .collect();
            let output = resampler.process(&input);
            let steady = &output[12_000..output.len() - 12_000];
            let peak = steady
                .iter()
                .fold(0.0f64, |peak, sample| peak.max(sample.abs()));
            assert!((peak - 1.0).abs() < 1e-3, "{quality:?}: {peak}");
        }
    }

    #[test]
    fn soxr_follows_the_quality_and_its_default_is_the_default_settings() {
        let resampling_of = |quality| {
            OutputSettings::from_config(None, SampleRate::MatchSong, quality, &DsdOutput::default())
                .resampling()
        };
        assert_eq!(
            resampling_of(ResamplingQuality::Soxr(SoxrParameters::default())),
            Resampling::Soxr(SoxrSettings::default())
        );
    }

    /// But for the rolloff, which changes nothing in `soxr`
    #[test]
    fn the_soxr_presets_are_libsoxrs() {
        for (quality, of_soxr) in [
            (SoxrQuality::Low, soxr::Quality::Low),
            (SoxrQuality::Medium, soxr::Quality::Medium),
            (SoxrQuality::High, soxr::Quality::High),
            (SoxrQuality::VeryHigh, soxr::Quality::VeryHigh),
        ] {
            let mut parameters = SoxrParameters::default();
            parameters.quality = quality;
            let mut expected = soxr::QualitySpec::new(of_soxr);
            expected.rolloff = soxr::Rolloff::Small;
            assert_eq!(soxr_settings(parameters).quality, expected, "{quality:?}");

            parameters.quality = SoxrQuality::Custom(quality.filter());
            assert_eq!(soxr_settings(parameters).quality, expected, "{quality:?}");
        }
    }

    #[test]
    fn every_soxr_choice_reaches_the_resampler() {
        let mut filter = SoxrQuality::High.filter();
        filter.phase = SoxrPhase::Intermediate;
        filter.double_precision = true;
        let mut parameters = SoxrParameters::default();
        parameters.quality = SoxrQuality::Custom(filter);
        parameters.performance.coef_table_kb = 1;
        parameters.performance.coef_interpolation = SoxrInterpolation::Linear;
        parameters.performance.min_dft_size = 256;
        parameters.performance.large_dft_size = 1_048_576;
        let settings = soxr_settings(parameters);
        assert_eq!(settings.quality.phase_response, 25.0);
        assert!(settings.quality.double_precision);
        assert_eq!(settings.runtime.coef_size_kbytes, 1);
        assert_eq!(
            settings.runtime.coef_interpolation,
            soxr::CoefInterpolation::Low
        );
        assert_eq!(settings.runtime.log2_min_dft_size, 8);
        assert_eq!(settings.runtime.log2_large_dft_size, 20);
        assert_eq!(settings.runtime.num_threads, 1);
        parameters.performance.coef_interpolation = SoxrInterpolation::Cubic;
        assert_eq!(
            soxr_settings(parameters).runtime.coef_interpolation,
            soxr::CoefInterpolation::High
        );
        filter.phase = SoxrPhase::Minimum;
        parameters.quality = SoxrQuality::Custom(filter);
        assert_eq!(soxr_settings(parameters).quality.phase_response, 0.0);
    }

    #[test]
    fn the_ends_of_the_soxr_ranges_are_what_the_resampler_takes() {
        let with = |precision, passband_end, coef_table_kb, min_dft_size, large_dft_size| {
            let mut filter = SoxrQuality::default().filter();
            filter.precision = precision;
            filter.passband_end = passband_end;
            let mut parameters = SoxrParameters::default();
            parameters.quality = SoxrQuality::Custom(filter);
            parameters.performance.coef_table_kb = coef_table_kb;
            parameters.performance.min_dft_size = min_dft_size;
            parameters.performance.large_dft_size = large_dft_size;
            soxr_settings(parameters)
        };
        let precisions = config::SOXR_PRECISIONS;
        let passband_ends = config::SOXR_PASSBAND_ENDS;
        let tables = config::SOXR_COEF_TABLE_KBS;
        let min_sizes = config::SOXR_MIN_DFT_SIZES;
        let large_sizes = config::SOXR_LARGE_DFT_SIZES;
        for settings in [
            with(
                *precisions.start(),
                *passband_ends.start(),
                *tables.start(),
                min_sizes[0],
                large_sizes[0],
            ),
            with(
                *precisions.end(),
                *passband_ends.end(),
                *tables.end(),
                min_sizes[min_sizes.len() - 1],
                large_sizes[large_sizes.len() - 1],
            ),
        ] {
            for (from, to) in [(44_100.0, 47_999.5), (192_000.0, 44_100.0)] {
                let mut resampler = soxr::Resampler::<f32, f32>::builder(from, to, 2)
                    .quality(settings.quality)
                    .runtime(settings.runtime)
                    .build()
                    .unwrap();
                let mut out = vec![0.0; 2 * 1_000];
                resampler
                    .resample(&[0.25; 2 * 1_000][..], &mut out[..])
                    .unwrap();
            }
        }
    }

    #[test]
    fn the_default_r8brain_parameters_are_the_default_settings() {
        assert_eq!(
            r8brain_settings(R8brainParameters::default()),
            R8brainSettings::default()
        );
    }

    #[test]
    fn only_dsd256_has_the_wide_noise_band() {
        let at = |rate, wide_noise_band| {
            let mut parameters = config::DsdQuality::High.parameters();
            parameters.wide_noise_band = wide_noise_band;
            let mut dsd = dsd_for("dac");
            dsd.rate = Some(rate);
            dsd.quality = config::DsdQuality::Custom(parameters);
            dsd_settings(&dsd)
        };
        assert_eq!(at(DsdRate::Dsd64, true), at(DsdRate::Dsd64, false));
        assert_eq!(at(DsdRate::Dsd128, true), at(DsdRate::Dsd128, false));
        assert_ne!(at(DsdRate::Dsd256, true), at(DsdRate::Dsd256, false));
    }

    #[test]
    fn dsd_never_applies_on_the_system_default() {
        let mut dsd = DsdOutput::default();
        dsd.is_on = true;
        let output = OutputSettings::from_config(
            None,
            SampleRate::MatchSong,
            ResamplingQuality::default(),
            &dsd,
        );
        assert!(output.dop().is_none());
    }
}
