use anyhow::Result;
use async_trait::async_trait;

#[derive(Debug, Clone, Copy)]
pub enum Mode {
    Usb,
    Lsb,
    Cw,
    Data,
}

#[async_trait]
pub trait Radio: Send + Sync {
    async fn frequency(&self) -> Result<u64>;
    async fn set_frequency(&self, hz: u64) -> Result<()>;

    async fn mode(&self) -> Result<Mode>;
    async fn set_mode(&self, mode: Mode) -> Result<()>;

    async fn ptt(&self, enabled: bool) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct NullRadio {
    frequency_hz: u64,
    mode: std::sync::Mutex<Mode>,
}

#[async_trait]
impl Radio for NullRadio {
    async fn frequency(&self) -> Result<u64> {
        Ok(self.frequency_hz)
    }

    async fn set_frequency(&self, _hz: u64) -> Result<()> {
        Ok(())
    }

    async fn mode(&self) -> Result<Mode> {
        Ok(*self.mode.lock().expect("mode mutex poisoned"))
    }

    async fn set_mode(&self, mode: Mode) -> Result<()> {
        *self.mode.lock().expect("mode mutex poisoned") = mode;
        Ok(())
    }

    async fn ptt(&self, _enabled: bool) -> Result<()> {
        Ok(())
    }
}

impl Default for Mode {
    fn default() -> Self {
        Self::Usb
    }
}
