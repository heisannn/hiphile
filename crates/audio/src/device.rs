use cpal::traits::{DeviceTrait, HostTrait};

use crate::output;
use crate::{Error, dop_device};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputDevice {
    id: Option<String>,
    name: String,
    sample_rate: Option<u32>,
    sample_rates: Vec<u32>,
    dop_rates: Vec<u32>,
    is_default: bool,
}

impl OutputDevice {
    /// What [`crate::OutputSettings`] names this device by, stable across
    /// launches. `None` when the system does not report one.
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Rate of the device's current default output configuration.
    pub fn sample_rate(&self) -> Option<u32> {
        self.sample_rate
    }

    /// The standard rates the device can run at, lowest first.
    pub fn sample_rates(&self) -> &[u32] {
        &self.sample_rates
    }

    /// The DoP rates at which frames would reach the device intact. Says
    /// nothing about whether it decodes them.
    pub fn dop_rates(&self) -> &[u32] {
        &self.dop_rates
    }

    pub fn is_default(&self) -> bool {
        self.is_default
    }
}

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
            let id = device.id().ok();
            let is_default = default_id.is_some() && id == default_id;
            let sample_rates = output::config_ranges(&device)
                .map(|ranges| output::standard_rates(&ranges))
                .unwrap_or_default();
            OutputDevice {
                id: id.map(|id| id.to_string()),
                name: device_name(&device),
                sample_rate,
                sample_rates,
                dop_rates: dop_device::frame_rates(&device),
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
