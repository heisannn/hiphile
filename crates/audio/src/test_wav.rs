/// 16-bit PCM WAV of `frames` frames, every sample `value`.
pub(crate) fn wav(rate: u32, channels: u16, frames: u32, value: i16) -> Vec<u8> {
    wav_with(rate, channels, frames, |_| value)
}

/// 16-bit PCM WAV whose every sample is its frame number, so a decoded
/// sample tells which frame it came from. `frames` must fit in an `i16`.
pub(crate) fn wav_counting(rate: u32, channels: u16, frames: u32) -> Vec<u8> {
    assert!(frames <= i16::MAX as u32);
    wav_with(rate, channels, frames, |frame| frame as i16)
}

pub(crate) fn counting_frame(sample: f32) -> u32 {
    (sample * 32_768.0).round() as u32
}

pub(crate) fn wav_with(
    rate: u32,
    channels: u16,
    frames: u32,
    sample: impl Fn(u32) -> i16,
) -> Vec<u8> {
    wav_by_channel(rate, channels, frames, |frame, _| sample(frame))
}

/// As [`wav_with`], with `sample` taking the frame and the channel.
pub(crate) fn wav_by_channel(
    rate: u32,
    channels: u16,
    frames: u32,
    sample: impl Fn(u32, u16) -> i16,
) -> Vec<u8> {
    let data_len = frames * u32::from(channels) * 2;
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
    bytes.extend_from_slice(&(channels * 2).to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for frame in 0..frames {
        for channel in 0..channels {
            bytes.extend_from_slice(&sample(frame, channel).to_le_bytes());
        }
    }
    bytes
}
