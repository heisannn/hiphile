//! Choosing the device configuration for a source.

/// Sample rate and channel count of a stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Spec {
    pub(crate) rate: u32,
    pub(crate) channels: u16,
}

/// One supported 32-bit float configuration range of a device.
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

/// Picks the output configuration for `source`.
///
/// The source rate wins whenever the device supports it, so the device is
/// switched to the file's rate and nothing is resampled. Otherwise the
/// device's current rate is kept and the source is resampled to it. Channels
/// prefer an exact match, then the smallest layout that holds the source.
pub(crate) fn plan(ranges: &[ConfigRange], device_rate: u32, source: Spec) -> Option<Spec> {
    let rate = [source.rate, device_rate]
        .into_iter()
        .find(|rate| ranges.iter().any(|range| range.contains(*rate)))
        .or_else(|| {
            ranges
                .first()
                .map(|range| source.rate.clamp(range.min_rate, range.max_rate))
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
            plan(&SPEAKERS, 48_000, spec(88_200, 2)),
            Some(spec(88_200, 2))
        );
    }

    #[test]
    fn keeps_device_rate_when_source_rate_is_unsupported() {
        assert_eq!(
            plan(&SPEAKERS, 48_000, spec(192_000, 2)),
            Some(spec(48_000, 2))
        );
    }

    #[test]
    fn widens_mono_to_stereo() {
        assert_eq!(
            plan(&SPEAKERS, 44_100, spec(44_100, 1)),
            Some(spec(44_100, 2))
        );
    }

    #[test]
    fn prefers_exact_channel_match() {
        let ranges = [range(2, 44_100, 192_000), range(6, 44_100, 192_000)];
        assert_eq!(
            plan(&ranges, 48_000, spec(48_000, 6)),
            Some(spec(48_000, 6))
        );
    }

    #[test]
    fn downmixes_when_device_has_fewer_channels() {
        assert_eq!(
            plan(&SPEAKERS, 48_000, spec(48_000, 6)),
            Some(spec(48_000, 2))
        );
    }

    #[test]
    fn clamps_when_neither_rate_is_listed() {
        let ranges = [range(2, 8_000, 32_000)];
        assert_eq!(
            plan(&ranges, 44_100, spec(48_000, 2)),
            Some(spec(32_000, 2))
        );
    }

    #[test]
    fn no_ranges_means_no_plan() {
        assert_eq!(plan(&[], 48_000, spec(48_000, 2)), None);
    }
}
