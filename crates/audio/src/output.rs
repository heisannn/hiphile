use cpal::traits::DeviceTrait;

use crate::Error;

/// Rates a device can be fixed at, among those it supports.
const STANDARD_RATES: [u32; 10] = [
    44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000, 705_600, 768_000,
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResamplingWindow {
    Hann,
    Hann2,
    Blackman,
    Blackman2,
    BlackmanHarris,
    #[default]
    BlackmanHarris2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resampling {
    fft_size: u32,
    window: ResamplingWindow,
}

impl Resampling {
    /// `fft_size` is the least input frames one FFT takes. It is rounded up to
    /// a multiple of the block the two rates allow, and `0` counts as `1`.
    pub fn new(fft_size: u32, window: ResamplingWindow) -> Self {
        Self {
            fft_size: fft_size.max(1),
            window,
        }
    }

    pub fn fft_size(&self) -> u32 {
        self.fft_size
    }

    pub fn window(&self) -> ResamplingWindow {
        self.window
    }
}

impl Default for Resampling {
    fn default() -> Self {
        Self::new(256, ResamplingWindow::default())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutputSettings {
    device: Option<String>,
    fixed_rate: Option<u32>,
    resampling: Resampling,
}

impl OutputSettings {
    /// `device` is an [`crate::OutputDevice::id`]; `None` follows the system's
    /// default device. `fixed_rate` runs the device at that rate and resamples
    /// to it; `None` follows each track's rate.
    pub fn new(device: Option<String>, fixed_rate: Option<u32>) -> Self {
        Self {
            device,
            fixed_rate,
            resampling: Resampling::default(),
        }
    }

    pub fn with_resampling(mut self, resampling: Resampling) -> Self {
        self.resampling = resampling;
        self
    }

    pub fn device(&self) -> Option<&str> {
        self.device.as_deref()
    }

    pub fn fixed_rate(&self) -> Option<u32> {
        self.fixed_rate
    }

    pub fn resampling(&self) -> Resampling {
        self.resampling
    }

    pub(crate) fn has_same_device_and_rate(&self, other: &Self) -> bool {
        self.device == other.device && self.fixed_rate == other.fixed_rate
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Spec {
    pub(crate) rate: u32,
    pub(crate) channels: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ConfigRange {
    pub(crate) channels: u16,
    pub(crate) min_rate: u32,
    pub(crate) max_rate: u32,
}

impl ConfigRange {
    fn contains(&self, rate: u32) -> bool {
        (self.min_rate..=self.max_rate).contains(&rate)
    }
}

/// The 32-bit float configurations `device` offers, the only format the
/// engine writes.
pub(crate) fn config_ranges(device: &cpal::Device) -> Result<Vec<ConfigRange>, Error> {
    Ok(device
        .supported_output_configs()?
        .filter(|range| range.sample_format() == cpal::SampleFormat::F32)
        .map(|range| ConfigRange {
            channels: range.channels(),
            min_rate: range.min_sample_rate(),
            max_rate: range.max_sample_rate(),
        })
        .collect())
}

/// The standard rates within `ranges`, lowest first.
pub(crate) fn standard_rates(ranges: &[ConfigRange]) -> Vec<u32> {
    STANDARD_RATES
        .into_iter()
        .filter(|rate| ranges.iter().any(|range| range.contains(*rate)))
        .collect()
}

/// `fixed_rate`, or otherwise the source rate, wins whenever the device
/// supports it, so without a fixed rate the device is switched to the file's
/// rate and nothing is resampled. Otherwise the device's current rate is kept
/// and the source is resampled to it. Channels prefer an exact match, then the
/// smallest layout that holds the source.
pub(crate) fn plan(
    ranges: &[ConfigRange],
    device_rate: u32,
    source: Spec,
    fixed_rate: Option<u32>,
) -> Option<Spec> {
    let wanted_rate = fixed_rate.unwrap_or(source.rate);
    let rate = [wanted_rate, device_rate]
        .into_iter()
        .find(|rate| ranges.iter().any(|range| range.contains(*rate)))
        .or_else(|| {
            ranges
                .first()
                .map(|range| wanted_rate.clamp(range.min_rate, range.max_rate))
        })?;
    let mut candidates = ranges
        .iter()
        .filter(|range| range.contains(rate))
        .map(|range| range.channels);
    let channels = {
        let all: Vec<u16> = candidates.by_ref().collect();
        all.iter()
            .copied()
            .find(|channels| *channels == source.channels)
            .or_else(|| {
                all.iter()
                    .copied()
                    .filter(|channels| *channels > source.channels)
                    .min()
            })
            .or_else(|| all.iter().copied().max())?
    };
    Some(Spec { rate, channels })
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn range(channels: u16, min_rate: u32, max_rate: u32) -> ConfigRange {
        ConfigRange {
            channels,
            min_rate,
            max_rate,
        }
    }

    const fn spec(rate: u32, channels: u16) -> Spec {
        Spec { rate, channels }
    }

    /// A typical built-in output: discrete rates, stereo only.
    const SPEAKERS: [ConfigRange; 4] = [
        range(2, 44_100, 44_100),
        range(2, 48_000, 48_000),
        range(2, 88_200, 88_200),
        range(2, 96_000, 96_000),
    ];

    #[test]
    fn switches_device_to_supported_source_rate() {
        assert_eq!(
            plan(&SPEAKERS, 48_000, spec(88_200, 2), None),
            Some(spec(88_200, 2))
        );
    }

    #[test]
    fn keeps_device_rate_when_source_rate_is_unsupported() {
        assert_eq!(
            plan(&SPEAKERS, 48_000, spec(192_000, 2), None),
            Some(spec(48_000, 2))
        );
    }

    #[test]
    fn widens_mono_to_stereo() {
        assert_eq!(
            plan(&SPEAKERS, 44_100, spec(44_100, 1), None),
            Some(spec(44_100, 2))
        );
    }

    #[test]
    fn prefers_exact_channel_match() {
        let ranges = [range(2, 44_100, 192_000), range(6, 44_100, 192_000)];
        assert_eq!(
            plan(&ranges, 48_000, spec(48_000, 6), None),
            Some(spec(48_000, 6))
        );
    }

    #[test]
    fn downmixes_when_device_has_fewer_channels() {
        assert_eq!(
            plan(&SPEAKERS, 48_000, spec(48_000, 6), None),
            Some(spec(48_000, 2))
        );
    }

    #[test]
    fn clamps_when_neither_rate_is_listed() {
        let ranges = [range(2, 8_000, 32_000)];
        assert_eq!(
            plan(&ranges, 44_100, spec(48_000, 2), None),
            Some(spec(32_000, 2))
        );
    }

    #[test]
    fn a_fixed_rate_wins_over_the_source_rate() {
        assert_eq!(
            plan(&SPEAKERS, 44_100, spec(96_000, 2), Some(48_000)),
            Some(spec(48_000, 2))
        );
    }

    #[test]
    fn keeps_device_rate_when_the_fixed_rate_is_unsupported() {
        assert_eq!(
            plan(&SPEAKERS, 48_000, spec(44_100, 2), Some(192_000)),
            Some(spec(48_000, 2))
        );
    }

    #[test]
    fn lists_the_standard_rates_a_device_supports() {
        assert_eq!(standard_rates(&SPEAKERS), [44_100, 48_000, 88_200, 96_000]);
        assert_eq!(
            standard_rates(&[range(2, 48_000, 192_000)]),
            [48_000, 88_200, 96_000, 176_400, 192_000]
        );
        assert_eq!(
            standard_rates(&[range(2, 384_000, 768_000)]),
            [384_000, 705_600, 768_000]
        );
        assert!(standard_rates(&[]).is_empty());
    }

    #[test]
    fn no_ranges_means_no_plan() {
        assert_eq!(plan(&[], 48_000, spec(48_000, 2), None), None);
    }
}
