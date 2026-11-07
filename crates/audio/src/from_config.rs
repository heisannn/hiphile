use config::{DsdFilter, DsdOutput, DsdParameters, DsdRate, ResamplingQuality};

use crate::output::{OutputSettings, Resampling, ResamplingWindow};

impl OutputSettings {
    /// What the settings in `config.toml` come to. DSD applies only on the
    /// device it was turned on for.
    pub fn from_config(
        device: Option<String>,
        fixed_rate: Option<u32>,
        quality: ResamplingQuality,
        dsd: &DsdOutput,
    ) -> Self {
        let dop = (dsd.is_on && dsd.device == device).then(|| dsd_settings(dsd));
        Self::new(device, fixed_rate)
            .with_resampling(Resampling::new(
                quality.fft_size(),
                window_of(quality.window()),
            ))
            .with_dop(dop)
            .with_dop_fade_in(dsd.fades_in)
    }
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
        let on_dac = OutputSettings::from_config(dac.clone(), None, quality, &dsd_for("dac"));
        assert!(on_dac.dop().is_some());
        let elsewhere = OutputSettings::from_config(dac, None, quality, &dsd_for("other"));
        assert!(elsewhere.dop().is_none());
        let off = OutputSettings::from_config(
            Some("dac".to_owned()),
            None,
            quality,
            &DsdOutput::default(),
        );
        assert!(off.dop().is_none());
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
        let output = OutputSettings::from_config(None, None, ResamplingQuality::default(), &dsd);
        assert!(output.dop().is_none());
    }
}
