//! What DoP needs of a device beyond what cpal offers: bit-exact 24 bits or
//! more at the DoP rate, and no other app's sound mixed in.

#[cfg(target_os = "macos")]
mod core_audio;
#[cfg(all(test, target_os = "macos"))]
pub(crate) use core_audio::holder;
#[cfg(target_os = "macos")]
pub(crate) use core_audio::{Exclusive, frame_rates};

#[cfg(not(target_os = "macos"))]
pub(crate) use elsewhere::{Exclusive, frame_rates};

/// The shared modes of other platforms' audio resample and mix, which would
/// turn DoP into noise.
#[cfg(not(target_os = "macos"))]
mod elsewhere {
    use crate::Error;

    pub(crate) struct Exclusive;

    impl Exclusive {
        pub(crate) fn take(_: &cpal::Device, _: u32) -> Result<Self, Error> {
            Err(Error::UnsupportedDopRate)
        }

        pub(crate) fn confirm(&self, _: u32) -> Result<(), Error> {
            Err(Error::UnsupportedDopRate)
        }
    }

    pub(crate) fn frame_rates(_: &cpal::Device) -> Vec<u32> {
        Vec::new()
    }
}
