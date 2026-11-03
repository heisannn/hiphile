use cpal::traits::{DeviceTrait, HostTrait};

use crate::Error;

#[derive(Clone, Debug)]
pub struct OutputDevice {
    name: String,
    sample_rate: Option<u32>,
    is_default: bool,
}

impl OutputDevice {
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Rate of the device's current default output configuration.
    pub fn sample_rate(&self) -> Option<u32> {
        self.sample_rate
    }

    pub fn is_default(&self) -> bool {
        self.is_default
    }
}

/// Output devices of the platform's default audio host.
pub fn output_devices() -> Result<Vec<OutputDevice>, Error> {
    let host = cpal::default_host();
    let default_id = host
        .default_output_device()
        .and_then(|device| device.id().ok());
    let devices = host
        .output_devices()?
        .map(|device| {
            let sample_rate = device
                .default_output_config()
                .ok()
                .map(|config| config.sample_rate());
            let is_default = default_id.is_some() && device.id().ok() == default_id;
            OutputDevice {
                name: device_name(&device),
                sample_rate,
                is_default,
            }
        })
        .collect();
    Ok(devices)
}

pub(crate) fn device_name(device: &cpal::Device) -> String {
    device
        .description()
        .map(|description| description.name().to_owned())
        .unwrap_or_else(|_| "Unknown device".to_owned())
}
