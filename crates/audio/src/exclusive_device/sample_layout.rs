const HUNDRED_NS_PER_SECOND: f64 = 10_000_000.0;

/// How a WASAPI exclusive stream lays out a sample, which the driver chooses
/// from: unlike the shared mode, nothing converts for the stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SampleLayout {
    Int32,
    /// 24 valid bits at the top of 32.
    Int24In32,
    Int24,
    Int16,
    Float32,
}

impl SampleLayout {
    /// Integers first, as a float format means the driver converts anyway.
    #[cfg(target_os = "windows")]
    pub(super) const PREFERRED: [Self; 5] = [
        Self::Int32,
        Self::Int24In32,
        Self::Int24,
        Self::Int16,
        Self::Float32,
    ];

    pub(super) fn bytes(self) -> usize {
        match self {
            Self::Int32 | Self::Int24In32 | Self::Float32 => 4,
            Self::Int24 => 3,
            Self::Int16 => 2,
        }
    }

    #[cfg(target_os = "windows")]
    pub(super) fn valid_bits(self) -> u16 {
        match self {
            Self::Int32 | Self::Float32 => 32,
            Self::Int24In32 | Self::Int24 => 24,
            Self::Int16 => 16,
        }
    }

    #[cfg(target_os = "windows")]
    pub(super) fn is_float(self) -> bool {
        self == Self::Float32
    }

    /// `out` holds exactly [`Self::bytes`] for each of `samples`.
    pub(super) fn write(self, samples: &[f32], out: &mut [u8]) {
        for (sample, slot) in samples.iter().zip(out.chunks_exact_mut(self.bytes())) {
            match self {
                Self::Float32 => slot.copy_from_slice(&sample.to_le_bytes()),
                Self::Int32 => slot.copy_from_slice(&quantized(*sample, 32).to_le_bytes()),
                Self::Int24In32 => {
                    slot.copy_from_slice(&(quantized(*sample, 24) << 8).to_le_bytes());
                }
                Self::Int24 => slot.copy_from_slice(&quantized(*sample, 24).to_le_bytes()[..3]),
                Self::Int16 => slot.copy_from_slice(&quantized(*sample, 16).to_le_bytes()[..2]),
            }
        }
    }
}

/// In `f64`, where every `f32` scaled by these sizes is exact, so samples that
/// came from integers of this size get the same integers back.
fn quantized(sample: f32, bits: i32) -> i32 {
    let scale = 2f64.powi(bits - 1);
    (f64::from(sample) * scale)
        .round()
        .clamp(-scale, scale - 1.0) as i32
}

/// Front left and right for stereo, front center for mono, otherwise as many
/// of the standard positions as there are channels.
pub(super) fn speaker_mask(channels: u16) -> u32 {
    match channels {
        1 => 0x4,
        2 => 0x3,
        channels => 1u32
            .checked_shl(u32::from(channels))
            .map_or(u32::MAX, |bit| bit - 1),
    }
}

/// The period, in the 100 ns units of `IAudioClient::Initialize`, that fits
/// the `frames` it answered `AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED` with. This is
/// the formula its documentation gives.
pub(super) fn aligned_period(frames: u32, rate: u32) -> i64 {
    (HUNDRED_NS_PER_SECOND * f64::from(frames) / f64::from(rate)).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(layout: SampleLayout, sample: f32) -> Vec<u8> {
        let mut out = vec![0; layout.bytes()];
        layout.write(&[sample], &mut out);
        out
    }

    #[test]
    fn integers_that_became_floats_come_back_the_same() {
        for value in [-32_768i16, -1, 0, 1, 12_345, 32_767] {
            let sample = f32::from(value) / 32_768.0;
            assert_eq!(written(SampleLayout::Int16, sample), value.to_le_bytes());
        }
        for value in [-8_388_608i32, -1, 0, 1, 1_234_567, 8_388_607] {
            let sample = value as f32 / 8_388_608.0;
            let bytes = value.to_le_bytes();
            assert_eq!(written(SampleLayout::Int24, sample), bytes[..3]);
            assert_eq!(
                written(SampleLayout::Int24In32, sample),
                (value << 8).to_le_bytes()
            );
        }
    }

    #[test]
    fn full_scale_does_not_wrap() {
        assert_eq!(written(SampleLayout::Int16, 1.0), i16::MAX.to_le_bytes());
        assert_eq!(written(SampleLayout::Int16, -1.5), i16::MIN.to_le_bytes());
        assert_eq!(written(SampleLayout::Int32, 1.0), i32::MAX.to_le_bytes());
        assert_eq!(written(SampleLayout::Int32, -1.0), i32::MIN.to_le_bytes());
        assert_eq!(written(SampleLayout::Int24, 1.0), [0xFF, 0xFF, 0x7F]);
        assert_eq!(written(SampleLayout::Int24, -1.0), [0x00, 0x00, 0x80]);
    }

    #[test]
    fn floats_go_as_they_are() {
        assert_eq!(written(SampleLayout::Float32, 0.25), 0.25f32.to_le_bytes());
    }

    #[test]
    fn writes_each_sample_at_its_own_place() {
        let mut out = vec![0; 6];
        SampleLayout::Int24.write(&[0.0, -1.0], &mut out);
        assert_eq!(out, [0, 0, 0, 0, 0, 0x80]);
    }

    #[test]
    fn speakers_follow_the_channels() {
        assert_eq!(speaker_mask(1), 0x4);
        assert_eq!(speaker_mask(2), 0x3);
        assert_eq!(speaker_mask(6), 0x3F);
        assert_eq!(speaker_mask(32), u32::MAX);
    }

    #[test]
    fn the_period_fits_the_frames_the_driver_answered() {
        assert_eq!(aligned_period(480, 48_000), 100_000);
        assert_eq!(aligned_period(441, 44_100), 100_000);
        assert_eq!(aligned_period(449, 44_100), 101_814);
    }
}
