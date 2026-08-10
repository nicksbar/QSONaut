use anyhow::Result;
use tracing_subscriber::{fmt, EnvFilter};

pub fn init(default_filter: &str) -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));
    fmt().with_env_filter(filter).with_target(false).compact().init();
    Ok(())
}
