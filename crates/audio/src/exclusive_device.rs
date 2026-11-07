//! What cpal does not offer: holding a device so that no other app plays on
//! it, and what DoP needs beyond that, bit-exact 24 bits or more at the DoP
//! rate.

#[cfg(target_os = "macos")]
mod core_audio;
#[cfg(any(target_os = "windows", test))]
mod sample_layout;
#[cfg(target_os = "windows")]
mod wasapi;

#[cfg(all(test, target_os = "macos"))]
pub(crate) use core_audio::holder;
#[cfg(target_os = "macos")]
pub(crate) use core_audio::{Exclusive, OriginalFormat, frame_rates};
#[cfg(target_os = "windows")]
pub(crate) use wasapi::{Stream, listed_rates, pcm_ranges};

#[cfg(not(target_os = "macos"))]
pub(crate) use elsewhere::{Exclusive, OriginalFormat, frame_rates};

#[cfg(not(target_os = "windows"))]
pub(crate) fn listed_rates(_: &cpal::Device, shared_rates: &[u32]) -> Vec<u32> {
    shared_rates.to_vec()
}

/// What a device can run at while held. Holding it changes none of that here.
#[cfg(not(target_os = "windows"))]
pub(crate) fn pcm_ranges(
    device: &cpal::Device,
) -> Result<Vec<crate::output::ConfigRange>, crate::Error> {
    crate::output::config_ranges(device)
}

/// DoP takes hog mode and physical formats, which only macOS has here. The
/// shared modes of other platforms' audio resample and mix, which would turn
/// DoP into noise. A WASAPI stream holds its device by itself.
#[cfg(not(target_os = "macos"))]
mod elsewhere {
    use crate::Error;

    pub(crate) struct Exclusive;

    #[derive(Clone, Copy)]
    pub(crate) struct OriginalFormat;

    impl OriginalFormat {
        pub(crate) fn restore(self) {}
    }

    impl Exclusive {
        pub(crate) fn for_dop(_: &cpal::Device, _: u32) -> Result<Self, Error> {
            Err(Error::UnsupportedDopRate)
        }

        pub(crate) fn for_pcm(_: &cpal::Device, _: u32) -> Result<Self, Error> {
            Ok(Self)
        }

        pub(crate) fn replaced_format(&self) -> Option<OriginalFormat> {
            None
        }

        pub(crate) fn confirm(&self, _: u32) -> Result<(), Error> {
            Err(Error::UnsupportedDopRate)
        }
    }

    pub(crate) fn frame_rates(_: &cpal::Device) -> Vec<u32> {
        Vec::new()
    }
}
