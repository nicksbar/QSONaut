use anyhow::Result;

#[derive(Debug, Clone)]
pub struct AudioDevice {
    pub name: String,
    pub default_sample_rate_hz: u32,
    pub channels: u8,
}

#[derive(Debug, Default, Clone)]
pub struct AudioService;

impl AudioService {
    pub async fn enumerate_devices(&self) -> Result<Vec<AudioDevice>> {
        Ok(vec![AudioDevice {
            name: "No audio backend selected (M0 mock)".to_string(),
            default_sample_rate_hz: 48_000,
            channels: 1,
        }])
    }
}
