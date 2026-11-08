use config::SampleRate;
use cpal::traits::DeviceTrait;

use crate::{Error, exclusive_device};

const DOP_CHANNELS: u16 = 2;

/// Rates a device can be fixed at, among those it supports.
pub(crate) const STANDARD_RATES: [u32; 10] = [
    44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000, 705_600, 768_000,
];

pub use dsd::ResamplingWindow;
pub use r8brain::Phase as R8brainPhase;

#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct R8brainSettings {
    pub options: r8brain::Options,
    pub cache_limits: r8brain::CacheLimits,
}

impl Default for R8brainSettings {
    /// The decoder gives 32-bit floating point, which the original has this
    /// quality for.
    fn default() -> Self {
        Self {
            options: r8brain::Options::BITS_24,
            cache_limits: r8brain::CacheLimits::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct SoxrSettings {
    pub quality: soxr::QualitySpec,
    pub runtime: soxr::RuntimeSpec,
}

impl Default for SoxrSettings {
    /// VHQ, as HQ's 20 bits fall short of the 32-bit floating point that the
    /// decoder gives. One thread, as the engine resamples a decoded packet at a
    /// time, whose few thousand frames are too little work for soxr to share.
    fn default() -> Self {
        Self {
            quality: soxr::QualitySpec::new(soxr::Quality::VeryHigh),
            runtime: soxr::RuntimeSpec::new(1),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Resampling {
    /// Rubato's. `fft_size` is the least frames one FFT takes at the lower of
    /// the two rates. It is rounded up to a multiple of the block the two rates
    /// allow, and `0` counts as `1`.
    Fft {
        fft_size: u32,
        window: ResamplingWindow,
    },
    R8brain(R8brainSettings),
    Soxr(SoxrSettings),
}

impl Resampling {
    pub fn new(fft_size: u32, window: ResamplingWindow) -> Self {
        Self::Fft { fft_size, window }
    }
}

impl Default for Resampling {
    fn default() -> Self {
        Self::new(256, ResamplingWindow::default())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutputSettings {
    device: Option<String>,
    sample_rate: SampleRate,
    resampling: Resampling,
    dop: Option<dsd::Settings>,
    dop_fades_in: bool,
    exclusive: bool,
}

impl Default for OutputSettings {
    fn default() -> Self {
        Self::new(None, None)
    }
}

impl OutputSettings {
    /// `device` is an [`crate::OutputDevice::id`]; `None` follows the system's
    /// default device. `fixed_rate` runs the device at that rate and resamples
    /// to it; `None` follows each track's rate.
    pub fn new(device: Option<String>, fixed_rate: Option<u32>) -> Self {
        Self {
            device,
            sample_rate: fixed_rate.map_or(SampleRate::MatchSong, SampleRate::Fixed),
            resampling: Resampling::default(),
            dop: None,
            dop_fades_in: true,
            exclusive: false,
        }
    }

    pub fn with_sample_rate(mut self, sample_rate: SampleRate) -> Self {
        self.sample_rate = sample_rate;
        self
    }

    pub fn with_resampling(mut self, resampling: Resampling) -> Self {
        self.resampling = resampling;
        self
    }

    /// Plays as DoP, at full level, on the named device. With no device named
    /// this stays PCM: the system's default can change to one that makes loud
    /// noise of DoP.
    pub fn with_dop(mut self, dop: Option<dsd::Settings>) -> Self {
        self.dop = dop.filter(|_| self.device.is_some());
        self
    }

    /// `false` starts DoP at full level where it would rise from silence.
    pub fn with_dop_fade_in(mut self, fades_in: bool) -> Self {
        self.dop_fades_in = fades_in;
        self
    }

    /// Keeps other apps off the device while PCM plays or is paused, on a
    /// platform that can. DoP always does.
    pub fn with_exclusive(mut self, exclusive: bool) -> Self {
        self.exclusive = exclusive && crate::CAN_PLAY_EXCLUSIVE;
        self
    }

    pub fn is_exclusive(&self) -> bool {
        self.exclusive
    }

    pub fn dop_fades_in(&self) -> bool {
        self.dop_fades_in
    }

    pub fn dop(&self) -> Option<&dsd::Settings> {
        self.dop.as_ref()
    }

    pub fn device(&self) -> Option<&str> {
        self.device.as_deref()
    }

    pub fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    /// Whether a multiple of the song's rate would reach the device. If not, a
    /// chosen multiple plays as the song's own rate.
    pub fn can_run_at_song_multiple(&self) -> bool {
        self.exclusive || crate::SHARED_STREAM_SETS_RATE
    }

    fn sample_rate_to_plan(&self) -> SampleRate {
        match self.sample_rate {
            SampleRate::SongMultiple(_) if !self.can_run_at_song_multiple() => {
                SampleRate::MatchSong
            }
            sample_rate => sample_rate,
        }
    }

    pub fn resampling(&self) -> Resampling {
        self.resampling
    }

    /// DoP runs at the rate its settings call for, whatever the fixed rate.
    pub(crate) fn has_same_device_and_rate(&self, other: &Self) -> bool {
        self.device == other.device
            && self.dop == other.dop
            && (self.dop.is_some()
                || (self.sample_rate == other.sample_rate && self.exclusive == other.exclusive))
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

/// What `device` offers for `settings`, to be planned with [`plan_for`].
pub(crate) fn ranges_for(
    device: &cpal::Device,
    settings: &OutputSettings,
) -> Result<Vec<ConfigRange>, Error> {
    if settings.dop().is_none() {
        return if settings.is_exclusive() {
            exclusive_device::pcm_ranges(device)
        } else {
            config_ranges(device)
        };
    }
    Ok(exclusive_device::frame_rates(device)
        .into_iter()
        .map(|rate| ConfigRange {
            channels: DOP_CHANNELS,
            min_rate: rate,
            max_rate: rate,
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

pub(crate) fn plan_for(
    settings: &OutputSettings,
    ranges: &[ConfigRange],
    device_rate: u32,
    source: Spec,
) -> Option<Spec> {
    match settings.dop() {
        Some(dop) => plan_dop(ranges, source.rate, dop),
        None => plan(ranges, device_rate, source, settings.sample_rate_to_plan()),
    }
}

/// A device that lacks the first rate gets the other family's, which is better
/// than failing or dropping to another DSD rate unasked.
fn plan_dop(ranges: &[ConfigRange], source_rate: u32, dop: &dsd::Settings) -> Option<Spec> {
    dop.frame_rates_for(source_rate)
        .into_iter()
        .find(|rate| ranges.iter().any(|range| range.contains(*rate)))
        .map(|rate| Spec {
            rate,
            channels: DOP_CHANNELS,
        })
}

/// The rates `sample_rate` asks for, best first. A multiple the device lacks
/// gives way to the next lower one, as a whole multiple of the source beats
/// whatever rate the device happens to be at.
fn wanted_rates(sample_rate: SampleRate, source_rate: u32) -> Vec<u32> {
    match sample_rate {
        SampleRate::MatchSong => vec![source_rate],
        SampleRate::Fixed(rate) => vec![rate],
        SampleRate::SongMultiple(multiple) => std::iter::successors(Some(multiple), |multiple| {
            (*multiple > 1).then_some(multiple / 2)
        })
        .map(|multiple| source_rate.saturating_mul(multiple))
        .collect(),
    }
}

/// `sample_rate` wins whenever the device supports it, so when matching songs
/// the device is switched to the file's rate and nothing is resampled.
/// Otherwise the device's current rate is kept and the source is resampled to
/// it. Channels prefer an exact match, then the smallest layout that holds the
/// source.
pub(crate) fn plan(
    ranges: &[ConfigRange],
    device_rate: u32,
    source: Spec,
    sample_rate: SampleRate,
) -> Option<Spec> {
    let wanted = wanted_rates(sample_rate, source.rate);
    let rate = wanted
        .iter()
        .copied()
        .chain([device_rate])
        .find(|rate| ranges.iter().any(|range| range.contains(*rate)))
        .or_else(|| {
            let lowest = *wanted.last()?;
            ranges
                .first()
                .map(|range| lowest.clamp(range.min_rate, range.max_rate))
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
            plan(&SPEAKERS, 48_000, spec(88_200, 2), SampleRate::MatchSong),
            Some(spec(88_200, 2))
        );
    }

    #[test]
    fn keeps_device_rate_when_source_rate_is_unsupported() {
        assert_eq!(
            plan(&SPEAKERS, 48_000, spec(192_000, 2), SampleRate::MatchSong),
            Some(spec(48_000, 2))
        );
    }

    #[test]
    fn widens_mono_to_stereo() {
        assert_eq!(
            plan(&SPEAKERS, 44_100, spec(44_100, 1), SampleRate::MatchSong),
            Some(spec(44_100, 2))
        );
    }

    #[test]
    fn prefers_exact_channel_match() {
        let ranges = [range(2, 44_100, 192_000), range(6, 44_100, 192_000)];
        assert_eq!(
            plan(&ranges, 48_000, spec(48_000, 6), SampleRate::MatchSong),
            Some(spec(48_000, 6))
        );
    }

    #[test]
    fn downmixes_when_device_has_fewer_channels() {
        assert_eq!(
            plan(&SPEAKERS, 48_000, spec(48_000, 6), SampleRate::MatchSong),
            Some(spec(48_000, 2))
        );
    }

    #[test]
    fn clamps_when_neither_rate_is_listed() {
        let ranges = [range(2, 8_000, 32_000)];
        assert_eq!(
            plan(&ranges, 44_100, spec(48_000, 2), SampleRate::MatchSong),
            Some(spec(32_000, 2))
        );
    }

    #[test]
    fn a_fixed_rate_wins_over_the_source_rate() {
        assert_eq!(
            plan(
                &SPEAKERS,
                44_100,
                spec(96_000, 2),
                SampleRate::Fixed(48_000)
            ),
            Some(spec(48_000, 2))
        );
    }

    #[test]
    fn keeps_device_rate_when_the_fixed_rate_is_unsupported() {
        assert_eq!(
            plan(
                &SPEAKERS,
                48_000,
                spec(44_100, 2),
                SampleRate::Fixed(192_000)
            ),
            Some(spec(48_000, 2))
        );
    }

    #[test]
    fn a_song_multiple_follows_each_song() {
        let ranges = [range(2, 44_100, 768_000)];
        assert_eq!(
            plan(
                &ranges,
                48_000,
                spec(44_100, 2),
                SampleRate::SongMultiple(4)
            ),
            Some(spec(176_400, 2))
        );
        assert_eq!(
            plan(
                &ranges,
                48_000,
                spec(48_000, 2),
                SampleRate::SongMultiple(2)
            ),
            Some(spec(96_000, 2))
        );
    }

    #[test]
    fn a_song_multiple_the_device_lacks_gives_way_to_a_lower_one() {
        assert_eq!(
            plan(
                &SPEAKERS,
                48_000,
                spec(44_100, 2),
                SampleRate::SongMultiple(16)
            ),
            Some(spec(88_200, 2))
        );
        assert_eq!(
            plan(
                &SPEAKERS,
                44_100,
                spec(96_000, 2),
                SampleRate::SongMultiple(2)
            ),
            Some(spec(96_000, 2))
        );
    }

    #[test]
    fn a_song_multiple_keeps_the_device_rate_when_no_multiple_fits() {
        assert_eq!(
            plan(
                &SPEAKERS,
                48_000,
                spec(192_000, 2),
                SampleRate::SongMultiple(2)
            ),
            Some(spec(48_000, 2))
        );
    }

    #[test]
    fn a_song_multiple_clamps_from_the_song_rate() {
        let ranges = [range(2, 100_000, 150_000)];
        assert_eq!(
            plan(
                &ranges,
                48_000,
                spec(44_100, 2),
                SampleRate::SongMultiple(16)
            ),
            Some(spec(100_000, 2))
        );
    }

    #[test]
    fn a_song_multiple_needs_a_stream_that_sets_the_rate() {
        let ranges = [range(2, 44_100, 768_000)];
        let shared = OutputSettings::new(None, None).with_sample_rate(SampleRate::SongMultiple(4));
        let expected = if crate::SHARED_STREAM_SETS_RATE {
            176_400
        } else {
            44_100
        };
        assert_eq!(
            plan_for(&shared, &ranges, 48_000, spec(44_100, 2)),
            Some(spec(expected, 2))
        );
        if crate::CAN_PLAY_EXCLUSIVE {
            let held = shared.with_exclusive(true);
            assert!(held.can_run_at_song_multiple());
            assert_eq!(
                plan_for(&held, &ranges, 48_000, spec(44_100, 2)),
                Some(spec(176_400, 2))
            );
        }
    }

    #[test]
    fn another_song_multiple_moves_pcm_but_makes_no_difference_to_dop() {
        let pcm = |multiple| {
            OutputSettings::new(Some("dac".to_owned()), None)
                .with_sample_rate(SampleRate::SongMultiple(multiple))
        };
        assert!(!pcm(2).has_same_device_and_rate(&pcm(4)));
        let dop = |multiple| pcm(multiple).with_dop(Some(dsd::Settings::default()));
        assert!(dop(2).has_same_device_and_rate(&dop(4)));
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
    fn dop_takes_the_standard_rate_and_then_the_other_family() {
        let both = [range(2, 176_400, 176_400), range(2, 192_000, 192_000)];
        let only_48 = [range(2, 192_000, 192_000)];
        let at =
            |rate| dsd::Settings::new(rate, dsd::Filter::default(), dsd::Modulation::default());
        let dsd64 = at(dsd::DopRate::Dsd64);
        assert_eq!(plan_dop(&both, 88_200, &dsd64), Some(spec(176_400, 2)));
        assert_eq!(plan_dop(&both, 96_000, &dsd64), Some(spec(176_400, 2)));
        assert_eq!(plan_dop(&only_48, 44_100, &dsd64), Some(spec(192_000, 2)));
        assert_eq!(plan_dop(&only_48, 44_100, &at(dsd::DopRate::Dsd128)), None);
    }

    #[test]
    fn dop_can_match_the_family_of_the_source() {
        let both = [range(2, 176_400, 176_400), range(2, 192_000, 192_000)];
        let matching = dsd::Settings::new(
            dsd::DopRate::Dsd64,
            dsd::Filter::default(),
            dsd::Modulation::default(),
        )
        .with_matching_source_family(true);
        assert_eq!(plan_dop(&both, 88_200, &matching), Some(spec(176_400, 2)));
        assert_eq!(plan_dop(&both, 96_000, &matching), Some(spec(192_000, 2)));
    }

    #[test]
    fn dop_needs_a_named_device() {
        let dop = Some(dsd::Settings::default());
        assert!(
            OutputSettings::new(None, None)
                .with_dop(dop)
                .dop()
                .is_none()
        );
        let named = OutputSettings::new(Some("dac".to_owned()), None).with_dop(dop);
        assert!(named.dop().is_some());
    }

    #[test]
    fn the_fade_makes_no_difference_to_the_stream() {
        let on = || {
            OutputSettings::new(Some("dac".to_owned()), None)
                .with_dop(Some(dsd::Settings::default()))
        };
        let without_fade = on().with_dop_fade_in(false);
        assert_ne!(on(), without_fade);
        assert!(on().has_same_device_and_rate(&without_fade));
    }

    #[test]
    fn a_fixed_rate_makes_no_difference_to_dop() {
        let on = |rate| {
            OutputSettings::new(Some("dac".to_owned()), rate)
                .with_dop(Some(dsd::Settings::default()))
        };
        assert!(on(None).has_same_device_and_rate(&on(Some(48_000))));
        let pcm = |rate| OutputSettings::new(Some("dac".to_owned()), rate);
        assert!(!pcm(None).has_same_device_and_rate(&pcm(Some(48_000))));
    }

    #[test]
    fn holding_the_device_moves_pcm_but_makes_no_difference_to_dop() {
        let pcm =
            |exclusive| OutputSettings::new(Some("dac".to_owned()), None).with_exclusive(exclusive);
        assert_eq!(pcm(true).is_exclusive(), crate::CAN_PLAY_EXCLUSIVE);
        if crate::CAN_PLAY_EXCLUSIVE {
            assert!(!pcm(true).has_same_device_and_rate(&pcm(false)));
        }
        let dop = |exclusive| pcm(exclusive).with_dop(Some(dsd::Settings::default()));
        assert!(dop(true).has_same_device_and_rate(&dop(false)));
    }

    #[test]
    fn no_ranges_means_no_plan() {
        assert_eq!(
            plan(&[], 48_000, spec(48_000, 2), SampleRate::MatchSong),
            None
        );
    }
}
