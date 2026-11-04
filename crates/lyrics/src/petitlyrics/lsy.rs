//! Reads times from the binary form of a song's timed lyrics. Their words are not
//! read from it: they come in another request.

use std::time::Duration;

const HEADER_TAG: &[u8] = b"MHDROBJT";
const HEADER_LENGTH: u32 = 44;
const VERSION: &[u8] = b"2.00";
const LINES_TAG: &[u8] = b"MLRCOBJT";
const LINES_TAG_AT: usize = 0x2c;
const TIMES_AT: usize = 0xcc;
const MOST_LINES: usize = 10_000;
/// Where each pair of a key's bits goes when the file says to rearrange them.
const KEY_BIT_PAIRS: [u32; 8] = [0, 2, 1, 4, 3, 6, 5, 7];
/// The times are hundredths of a second in 16 bits, so a long song wraps around.
const WRAP: u32 = 1 << 16;
/// The most times may fall from one line to the next and still be damage. A
/// greater fall is taken for wrapping.
const LARGEST_FALL_OF_DAMAGE: u16 = 32_768;

/// When each line of `payload` starts, unless it is of an unknown form, damaged,
/// or runs past a song of `length`.
pub(super) fn times(payload: &[u8], length: Option<Duration>) -> Option<Vec<Duration>> {
    let has_tag = |at: usize, tag: &[u8]| payload.get(at..at + tag.len()) == Some(tag);
    if !has_tag(0, HEADER_TAG)
        || read_u32(payload, 0x08)? != HEADER_LENGTH
        || !has_tag(0x0c, VERSION)
        || !has_tag(LINES_TAG_AT, LINES_TAG)
    {
        return None;
    }
    let chunk = usize::try_from(read_u32(payload, 0x34)?).ok()?;
    if chunk.checked_add(LINES_TAG_AT)? != payload.len() {
        return None;
    }
    let lines = usize::try_from(read_u32(payload, 0x38)?).ok()?;
    if lines == 0 || lines > MOST_LINES {
        return None;
    }
    let width = usize::from(read_u16(payload, 0x42)?);
    let expected = width
        .checked_add(2)?
        .checked_mul(lines)?
        .checked_add(TIMES_AT)?;
    if expected != payload.len() {
        return None;
    }

    let key = key(*payload.get(0x19)?, read_u16(payload, 0x1a)?);
    let mut times = Vec::with_capacity(lines);
    let mut wraps = 0;
    let mut last: Option<u16> = None;
    for line in 0..lines {
        let raw = read_u16(payload, TIMES_AT + line * 2)? ^ key;
        if let Some(last) = last
            && raw < last
        {
            if last - raw <= LARGEST_FALL_OF_DAMAGE {
                return None;
            }
            wraps += WRAP;
        }
        last = Some(raw);
        let time = Duration::from_millis(u64::from(wraps + u32::from(raw)) * 10);
        if length.is_some_and(|length| time > length) {
            return None;
        }
        times.push(time);
    }
    Some(times)
}

/// The key that hides the times.
fn key(rearrange: u8, protection_id: u16) -> u16 {
    if rearrange == 0 {
        return protection_id;
    }
    let mut key = 0;
    for (source, target) in KEY_BIT_PAIRS.iter().enumerate() {
        key |= ((protection_id >> (2 * source)) & 3) << (2 * target);
    }
    key
}

fn read_u16(payload: &[u8], at: usize) -> Option<u16> {
    let bytes = payload.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes(bytes.try_into().ok()?))
}

fn read_u32(payload: &[u8], at: usize) -> Option<u32> {
    let bytes = payload.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes(bytes.try_into().ok()?))
}

/// A payload of the form this reads, for tests here and of what reads the answers.
#[cfg(test)]
pub(super) fn payload(rearrange: u8, protection_id: u16, hundredths: &[u16]) -> Vec<u8> {
    const WIDTH: u16 = 4;
    let key = key(rearrange, protection_id);
    let length = TIMES_AT + hundredths.len() * (2 + usize::from(WIDTH));
    let mut payload = vec![0; length];
    payload[..8].copy_from_slice(HEADER_TAG);
    payload[0x08..0x0c].copy_from_slice(&HEADER_LENGTH.to_le_bytes());
    payload[0x0c..0x10].copy_from_slice(VERSION);
    payload[0x19] = rearrange;
    payload[0x1a..0x1c].copy_from_slice(&protection_id.to_le_bytes());
    payload[LINES_TAG_AT..LINES_TAG_AT + 8].copy_from_slice(LINES_TAG);
    let chunk = u32::try_from(length - LINES_TAG_AT).unwrap();
    payload[0x34..0x38].copy_from_slice(&chunk.to_le_bytes());
    let lines = u32::try_from(hundredths.len()).unwrap();
    payload[0x38..0x3c].copy_from_slice(&lines.to_le_bytes());
    payload[0x42..0x44].copy_from_slice(&WIDTH.to_le_bytes());
    for (ix, time) in hundredths.iter().enumerate() {
        let at = TIMES_AT + ix * 2;
        payload[at..at + 2].copy_from_slice(&(time ^ key).to_le_bytes());
    }
    payload
}

#[cfg(test)]
mod tests {
    use super::*;

    fn millis(times: &[u64]) -> Vec<Duration> {
        times.iter().copied().map(Duration::from_millis).collect()
    }

    #[test]
    fn a_key_rearranged_comes_out_as_the_known_one() {
        assert_eq!(key(1, 0xccc2), 0xf302);
        assert_eq!(key(0, 0xccc2), 0xccc2);
    }

    #[test]
    fn times_read_with_or_without_rearranging_the_key() {
        for rearrange in [0, 1] {
            let payload = payload(rearrange, 0xccc2, &[100, 250]);
            assert_eq!(times(&payload, None), Some(millis(&[1000, 2500])));
        }
    }

    #[test]
    fn a_long_song_wraps_around() {
        let payload = payload(0, 0, &[60_000, 65_000, 500, 60_000, 100]);
        assert_eq!(
            times(&payload, None),
            Some(millis(&[600_000, 650_000, 660_360, 1_255_360, 1_311_720]))
        );
    }

    #[test]
    fn a_small_fall_is_damage() {
        assert_eq!(times(&payload(0, 0, &[1000, 900]), None), None);
        assert_eq!(times(&payload(0, 0, &[40_000, 7_232]), None), None);
    }

    #[test]
    fn a_time_past_the_song_is_damage() {
        let payload = payload(0, 0, &[100, 2000]);
        assert_eq!(times(&payload, Some(Duration::from_secs(19))), None);
        assert_eq!(
            times(&payload, Some(Duration::from_secs(20))),
            Some(millis(&[1000, 20_000]))
        );
    }

    #[test]
    fn unknown_forms_and_damage_are_turned_down() {
        let good = payload(0, 0, &[100, 200]);
        assert!(times(&good, None).is_some());

        assert_eq!(times(&good[..good.len() - 1], None), None, "cut short");
        assert_eq!(times(&[], None), None, "empty");
        for (name, at, value) in [
            ("first tag", 0, 0),
            ("header length", 0x08, 45),
            ("version", 0x0c, 0),
            ("second tag", LINES_TAG_AT, 0),
            ("chunk length", 0x34, 1),
            ("line count", 0x38, 3),
            ("width", 0x42, 9),
        ] {
            let mut damaged = good.clone();
            damaged[at] = value;
            assert_eq!(times(&damaged, None), None, "{name}");
        }

        let mut too_many = good.clone();
        let many = u32::try_from(MOST_LINES + 1).unwrap();
        too_many[0x38..0x3c].copy_from_slice(&many.to_le_bytes());
        assert_eq!(times(&too_many, None), None, "too many lines");
        let mut overflowing = good;
        overflowing[0x38..0x3c].copy_from_slice(&u32::MAX.to_le_bytes());
        overflowing[0x42..0x44].copy_from_slice(&u16::MAX.to_le_bytes());
        assert_eq!(times(&overflowing, None), None, "overflow");
    }

    #[test]
    fn no_lines_are_turned_down() {
        assert_eq!(times(&payload(0, 0, &[]), None), None);
    }
}
