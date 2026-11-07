use std::time::Duration;

use cpal::traits::{DeviceTrait, StreamTrait};

use crate::Error;
#[cfg(target_os = "windows")]
use crate::exclusive_device;
use crate::output::OutputSettings;

pub(crate) enum Stream {
    Shared(cpal::Stream),
    #[cfg(target_os = "windows")]
    Exclusive(exclusive_device::Stream),
}

impl Stream {
    /// Exclusive `settings` matter only on Windows, where they take a stream of
    /// another kind. Elsewhere, what holds the device is apart from the stream.
    #[cfg_attr(not(target_os = "windows"), expect(unused_variables))]
    pub(crate) fn open_pcm(
        device: &cpal::Device,
        config: cpal::StreamConfig,
        settings: &OutputSettings,
        render: impl FnMut(&mut [f32], &cpal::OutputCallbackInfo) + Send + 'static,
        on_error: impl FnMut(cpal::Error) + Send + 'static,
        rate_switch_timeout: Duration,
    ) -> Result<Self, Error> {
        #[cfg(target_os = "windows")]
        if settings.is_exclusive() {
            let follows_default = settings.device().is_none();
            return exclusive_device::Stream::open(
                device,
                config,
                follows_default,
                render,
                on_error,
            )
            .map(Self::Exclusive);
        }
        Ok(Self::Shared(device.build_output_stream(
            config,
            render,
            on_error,
            Some(rate_switch_timeout),
        )?))
    }

    pub(crate) fn play(&self) -> Result<(), Error> {
        match self {
            Self::Shared(stream) => stream.play()?,
            #[cfg(target_os = "windows")]
            Self::Exclusive(stream) => stream.play(),
        }
        Ok(())
    }

    pub(crate) fn pause(&self) -> Result<(), Error> {
        match self {
            Self::Shared(stream) => stream.pause()?,
            #[cfg(target_os = "windows")]
            Self::Exclusive(stream) => stream.pause(),
        }
        Ok(())
    }
}
