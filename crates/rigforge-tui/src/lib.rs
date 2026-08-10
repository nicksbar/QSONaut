use anyhow::Result;
use rigforge_ai::LanguageModel;
use rigforge_core::{AppConfig, AppEventBus};
use rigforge_radio::Radio;
use tracing::info;

pub async fn render_shell(
    config: &AppConfig,
    events: &AppEventBus,
    radio: &dyn Radio,
    _ai: &dyn LanguageModel,
) -> Result<()> {
    let mut rx = events.subscribe();
    while let Ok(event) = rx.try_recv() {
        info!(?event, "event");
    }

    let hz = radio.frequency().await.unwrap_or(0);

    println!("=== RigForge (M0 shell) ===");
    println!("Callsign: {}", config.station.callsign.as_deref().unwrap_or("(unset)"));
    println!("Grid:     {}", config.station.grid.as_deref().unwrap_or("(unset)"));
    println!("Radio Hz: {}", hz);
    println!("AI:       {} ({})", if config.ai.enabled { "on" } else { "off" }, config.ai.provider);
    println!();
    println!("Realtime path active: core/audio/dsp/radio/modes/tui");
    println!("AI path active: advisory only");
    println!();
    println!("Docs loaded from ./docs as implementation research input.");

    Ok(())
}
