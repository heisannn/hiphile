#![expect(
    unsafe_code,
    reason = "hog mode and physical formats have no safe binding, in cpal or elsewhere"
)]

use std::ffi::c_void;
use std::mem::{MaybeUninit, size_of};
use std::ptr::{NonNull, null};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::DeviceTrait;
use objc2_core_audio::{
    AudioDeviceID, AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectID,
    AudioObjectPropertyAddress, AudioObjectSetPropertyData, AudioStreamID,
    AudioStreamRangedDescription, kAudioDevicePropertyHogMode,
    kAudioDevicePropertyNominalSampleRate, kAudioDevicePropertyStreams,
    kAudioHardwarePropertyTranslateUIDToDevice, kAudioObjectPropertyElementMain,
    kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyScopeOutput, kAudioObjectSystemObject,
    kAudioObjectUnknown, kAudioStreamPropertyAvailablePhysicalFormats,
    kAudioStreamPropertyPhysicalFormat,
};
use objc2_core_audio_types::{
    AudioStreamBasicDescription, kAudioFormatFlagIsFloat, kAudioFormatFlagIsNonMixable,
    kAudioFormatFlagIsSignedInteger, kAudioFormatLinearPCM,
};
use objc2_core_foundation::CFString;

use crate::Error;

const FORMAT_SWITCH_TIMEOUT: Duration = Duration::from_secs(5);
const FORMAT_SWITCH_POLL: Duration = Duration::from_millis(20);
const NO_HOG: Pid = -1;
const LEAST_INTEGER_BITS: u32 = 24;
const FLOAT_BITS: u32 = 32;
const DOP_CHANNELS: u32 = 2;

type Pid = i32;

/// This process's hold on a device, so that no other app can play on it or
/// change its format. Released on drop.
pub(crate) struct Exclusive {
    device: AudioDeviceID,
    stream: AudioStreamID,
}

impl Exclusive {
    /// Holds `device` and sets it to the widest format for DoP at `rate`.
    pub(crate) fn take(device: &cpal::Device, rate: u32) -> Result<Self, Error> {
        let id = audio_device_of(device).ok_or(Error::OutputDeviceUnavailable)?;
        let stream = output_stream(id).ok_or(Error::UnsupportedDopRate)?;
        let format = physical_formats(stream)
            .into_iter()
            .filter(|format| is_for_dop(&format.mFormat) && covers(format, rate))
            .max_by_key(preference)
            .ok_or(Error::UnsupportedDopRate)?;
        let exclusive = Self::hog(id, stream)?;
        let mut format = format.mFormat;
        format.mSampleRate = f64::from(rate);
        set_physical_format(stream, format)?;
        Ok(exclusive)
    }

    /// Checks that the stream just opened left the device as DoP needs it. At
    /// another rate the stream would resample DoP into loud noise.
    pub(crate) fn confirm(&self, rate: u32) -> Result<(), Error> {
        let wanted = f64::from(rate);
        let nominal = read::<f64>(
            self.device,
            &global_address(kAudioDevicePropertyNominalSampleRate),
        );
        let physical = read::<AudioStreamBasicDescription>(
            self.stream,
            &global_address(kAudioStreamPropertyPhysicalFormat),
        );
        let is_right = nominal == Some(wanted)
            && physical.is_some_and(|format| format.mSampleRate == wanted && is_for_dop(&format));
        if is_right {
            Ok(())
        } else {
            Err(Error::UnsupportedDopRate)
        }
    }

    fn hog(device: AudioDeviceID, stream: AudioStreamID) -> Result<Self, Error> {
        let address = hog_address();
        let own = std::process::id() as Pid;
        let holder = read::<Pid>(device, &address)
            .ok_or_else(|| Error::ExclusiveOutput("it does not report who holds it".to_owned()))?;
        if holder == own {
            return Ok(Self { device, stream });
        }
        if holder != NO_HOG {
            return Err(Error::DeviceHeld(holder));
        }
        write(device, &address, &own)
            .map_err(|status| Error::ExclusiveOutput(format!("CoreAudio error {status}")))?;
        Ok(Self { device, stream })
    }
}

impl Drop for Exclusive {
    fn drop(&mut self) {
        let own = std::process::id() as Pid;
        if read::<Pid>(self.device, &hog_address()) == Some(own) {
            // Best effort: a device that is gone holds nothing
            write(self.device, &hog_address(), &NO_HOG).ok();
        }
    }
}

/// The process that holds `device`, or `-1`.
#[cfg(test)]
pub(crate) fn holder(device: &cpal::Device) -> Option<i32> {
    read(audio_device_of(device)?, &hog_address())
}

/// The DoP rates at which `device` can run bit-exact, as far as its formats
/// tell. Whether it decodes DoP is not something it reports.
pub(crate) fn frame_rates(device: &cpal::Device) -> Vec<u32> {
    let Some(stream) = audio_device_of(device).and_then(output_stream) else {
        return Vec::new();
    };
    let formats = physical_formats(stream);
    dsd::DopRate::ALL
        .into_iter()
        .flat_map(|rate| [rate.frame_rate_for(44_100), rate.frame_rate_for(48_000)])
        .filter(|rate| {
            formats
                .iter()
                .any(|format| is_for_dop(&format.mFormat) && covers(format, *rate))
        })
        .collect()
}

/// A float format can only be a virtual device, or a driver that converts on
/// its own, and 24-bit integers survive in it exactly. A non-mixable format
/// makes the device's virtual format integer too, of which cpal cannot read
/// the configuration that every later stream starts from.
fn is_for_dop(format: &AudioStreamBasicDescription) -> bool {
    let is_integer = format.mFormatFlags & kAudioFormatFlagIsSignedInteger != 0;
    let is_float = format.mFormatFlags & kAudioFormatFlagIsFloat != 0;
    format.mFormatID == kAudioFormatLinearPCM
        && format.mFormatFlags & kAudioFormatFlagIsNonMixable == 0
        && format.mChannelsPerFrame >= DOP_CHANNELS
        && ((is_integer && format.mBitsPerChannel >= LEAST_INTEGER_BITS)
            || (is_float && format.mBitsPerChannel == FLOAT_BITS))
}

fn covers(format: &AudioStreamRangedDescription, rate: u32) -> bool {
    let rate = f64::from(rate);
    (format.mSampleRateRange.mMinimum..=format.mSampleRateRange.mMaximum).contains(&rate)
        || format.mFormat.mSampleRate == rate
}

/// Exactly two channels first, then integers, then more bits.
fn preference(format: &AudioStreamRangedDescription) -> (bool, bool, u32) {
    let format = &format.mFormat;
    (
        format.mChannelsPerFrame == DOP_CHANNELS,
        format.mFormatFlags & kAudioFormatFlagIsSignedInteger != 0,
        format.mBitsPerChannel,
    )
}

/// The switch happens in the background; a stream opened before it is done
/// would run on the old format.
fn set_physical_format(
    stream: AudioStreamID,
    format: AudioStreamBasicDescription,
) -> Result<(), Error> {
    let address = global_address(kAudioStreamPropertyPhysicalFormat);
    let is_set = |current: &AudioStreamBasicDescription| {
        current.mSampleRate == format.mSampleRate
            && current.mBitsPerChannel == format.mBitsPerChannel
            && current.mFormatFlags == format.mFormatFlags
            && current.mChannelsPerFrame == format.mChannelsPerFrame
    };
    if read::<AudioStreamBasicDescription>(stream, &address).is_some_and(|now| is_set(&now)) {
        return Ok(());
    }
    write(stream, &address, &format)
        .map_err(|status| Error::Output(format!("could not set the format: {status}")))?;
    let deadline = Instant::now() + FORMAT_SWITCH_TIMEOUT;
    while Instant::now() < deadline {
        if read::<AudioStreamBasicDescription>(stream, &address).is_some_and(|now| is_set(&now)) {
            return Ok(());
        }
        thread::sleep(FORMAT_SWITCH_POLL);
    }
    Err(Error::Output(
        "the device did not reach the DoP format in time".to_owned(),
    ))
}

fn audio_device_of(device: &cpal::Device) -> Option<AudioDeviceID> {
    let id = device.id().ok()?;
    let uid = CFString::from_str(id.id());
    let uid = NonNull::from(&*uid);
    let address = global_address(kAudioHardwarePropertyTranslateUIDToDevice);
    let mut found: AudioDeviceID = kAudioObjectUnknown;
    let mut size = size_of::<AudioDeviceID>() as u32;
    // SAFETY: The property takes a CFString reference as its qualifier and
    // writes one device id; both outlive the call and sizes are given.
    let status = unsafe {
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject as AudioObjectID,
            NonNull::from(&address),
            size_of::<NonNull<CFString>>() as u32,
            NonNull::from(&uid).cast::<c_void>().as_ptr(),
            NonNull::from(&mut size),
            NonNull::from(&mut found).cast(),
        )
    };
    (status == 0 && found != kAudioObjectUnknown).then_some(found)
}

fn output_stream(device: AudioDeviceID) -> Option<AudioStreamID> {
    let address = AudioObjectPropertyAddress {
        mSelector: kAudioDevicePropertyStreams,
        mScope: kAudioObjectPropertyScopeOutput,
        mElement: kAudioObjectPropertyElementMain,
    };
    read_list::<AudioStreamID>(device, &address)
        .first()
        .copied()
}

fn physical_formats(stream: AudioStreamID) -> Vec<AudioStreamRangedDescription> {
    read_list(
        stream,
        &global_address(kAudioStreamPropertyAvailablePhysicalFormats),
    )
}

fn hog_address() -> AudioObjectPropertyAddress {
    global_address(kAudioDevicePropertyHogMode)
}

fn global_address(selector: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    }
}

/// `T` must be the property's plain-data type.
fn read<T: Copy>(object: AudioObjectID, address: &AudioObjectPropertyAddress) -> Option<T> {
    let mut value = MaybeUninit::<T>::uninit();
    let mut size = size_of::<T>() as u32;
    // SAFETY: `value` has room for the `size` bytes the call may write
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(address),
            0,
            null(),
            NonNull::from(&mut size),
            NonNull::from(&mut value).cast(),
        )
    };
    // SAFETY: Success with the whole size means the property filled `value`
    (status == 0 && size as usize == size_of::<T>()).then(|| unsafe { value.assume_init() })
}

/// `T` must be the plain-data type of the property's elements.
fn read_list<T: Copy>(object: AudioObjectID, address: &AudioObjectPropertyAddress) -> Vec<T> {
    let mut size = 0u32;
    // SAFETY: Writes only `size`
    let status = unsafe {
        AudioObjectGetPropertyDataSize(
            object,
            NonNull::from(address),
            0,
            null(),
            NonNull::from(&mut size),
        )
    };
    if status != 0 || size == 0 {
        return Vec::new();
    }
    let count = size as usize / size_of::<T>();
    let mut values = Vec::<T>::with_capacity(count);
    size = (count * size_of::<T>()) as u32;
    // SAFETY: `values` has room for the `size` bytes the call may write
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(address),
            0,
            null(),
            NonNull::from(&mut size),
            NonNull::new(values.as_mut_ptr())
                .expect("a vector's pointer is never null")
                .cast(),
        )
    };
    if status != 0 {
        return Vec::new();
    }
    // SAFETY: The call filled `size` bytes of whole elements of plain data
    unsafe { values.set_len(size as usize / size_of::<T>()) };
    values
}

fn write<T>(
    object: AudioObjectID,
    address: &AudioObjectPropertyAddress,
    value: &T,
) -> Result<(), i32> {
    // SAFETY: `value` is read for the size of `T`, which the property holds
    let status = unsafe {
        AudioObjectSetPropertyData(
            object,
            NonNull::from(address),
            0,
            null(),
            size_of::<T>() as u32,
            NonNull::from(value).cast(),
        )
    };
    if status == 0 { Ok(()) } else { Err(status) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(flags: u32, bits: u32, channels: u32) -> AudioStreamBasicDescription {
        AudioStreamBasicDescription {
            mSampleRate: 705_600.0,
            mFormatID: kAudioFormatLinearPCM,
            mFormatFlags: flags,
            mBytesPerPacket: 0,
            mFramesPerPacket: 1,
            mBytesPerFrame: 0,
            mChannelsPerFrame: channels,
            mBitsPerChannel: bits,
            mReserved: 0,
        }
    }

    #[test]
    fn dop_takes_what_keeps_its_bits_and_can_be_mixed() {
        let integer = kAudioFormatFlagIsSignedInteger;
        assert!(is_for_dop(&format(integer, 32, 2)));
        assert!(is_for_dop(&format(integer, 24, 2)));
        assert!(is_for_dop(&format(kAudioFormatFlagIsFloat, 32, 2)));
        assert!(!is_for_dop(&format(integer, 16, 2)));
        assert!(!is_for_dop(&format(integer, 32, 1)));
        assert!(!is_for_dop(&format(
            integer | kAudioFormatFlagIsNonMixable,
            32,
            2
        )));
    }
}
