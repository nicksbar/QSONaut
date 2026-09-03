use qsonaut_hostbridge_client::{HostBridgeClient, HostBridgeConfig, HostBridgeEvent};
use qsonaut_hostbridge_protocol::{
    AudioCodec, MediaDirection, MediaFrameHeader, RadioDriver, ScopeConfiguration, ServerMessage,
    WireMeterId, MEDIA_HEADER_VERSION,
};
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let endpoint = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "ws://127.0.0.1:8765".into());
    let access_key = std::env::var("HOSTBRIDGE_ACCESS_KEY")?;
    let password = std::env::var("HOSTBRIDGE_PASSWORD")?;
    let (client, mut events) = HostBridgeClient::spawn(HostBridgeConfig {
        endpoint,
        client_name: "QSONaut HostBridge smoke test".into(),
        access_key,
        password,
        ..Default::default()
    });
    let exercise_scope = std::env::var_os("HOSTBRIDGE_SCOPE").is_some();
    let mut radio_selected = false;
    let mut scope_started = false;

    let deadline = tokio::time::sleep(Duration::from_secs(8));
    tokio::pin!(deadline);
    while let Ok(Some(event)) = tokio::time::timeout_at(deadline.deadline(), events.recv()).await {
        match &event {
            HostBridgeEvent::Media { header, payload } => {
                println!("Media {header:?} payload_bytes={}", payload.len());
            }
            _ => println!("{event:?}"),
        }
        if let HostBridgeEvent::Connected(hello) = &event {
            let requested_radio_id = std::env::var("HOSTBRIDGE_RADIO_ID").ok();
            if let Some(radio) = hello.capabilities.radio_devices.iter().find(|device| {
                !device.in_use
                    && requested_radio_id
                        .as_deref()
                        .is_none_or(|requested| requested == device.id)
            }) {
                let driver = match std::env::var("HOSTBRIDGE_DRIVER")
                    .unwrap_or_else(|_| "icom_civ".into())
                    .as_str()
                {
                    "yaesu_cat" => RadioDriver::YaesuCat,
                    "yaesu_legacy_cat" => RadioDriver::YaesuLegacyCat,
                    "kenwood_cat" => RadioDriver::KenwoodCat,
                    _ => RadioDriver::IcomCiv,
                };
                let model = std::env::var("HOSTBRIDGE_MODEL").ok();
                let baud_rate = std::env::var("HOSTBRIDGE_BAUD")
                    .ok()
                    .and_then(|baud| baud.parse().ok());
                println!(
                    "Selecting physical radio {} ({}) with {:?} {:?}",
                    radio.id, radio.label, driver, model
                );
                client.select_radio(radio.id.clone(), driver, model, baud_rate, None)?;
                radio_selected = true;
                for meter in [
                    WireMeterId::Signal,
                    WireMeterId::Power,
                    WireMeterId::Swr,
                    WireMeterId::Alc,
                    WireMeterId::Compression,
                    WireMeterId::Current,
                    WireMeterId::Voltage,
                    WireMeterId::Temperature,
                ] {
                    client.get_meter(meter)?;
                }
                client.get_control("IpPlus")?;
            }
            let source_id = std::env::var("HOSTBRIDGE_SOURCE_ID").ok();
            if let Some(source) = hello.capabilities.audio_sources.iter().find(|source| {
                source_id
                    .as_deref()
                    .is_none_or(|requested| requested == source.id)
            }) {
                if let Some(format) = source.formats.first() {
                    client.select_audio(true, source.id.clone(), format.clone())?;
                }
            }
            let output_id = std::env::var("HOSTBRIDGE_OUTPUT_ID").ok();
            if let Some(output) = hello.capabilities.audio_outputs.iter().find(|output| {
                output_id
                    .as_deref()
                    .is_none_or(|requested| requested == output.id)
            }) {
                if let Some(format) = output.formats.first() {
                    client.select_audio_output(true, output.id.clone(), format.clone())?;
                    let sample_count = 960_usize * usize::from(format.channels);
                    let payload = vec![0_u8; sample_count * 2];
                    client.send_media(
                        MediaFrameHeader {
                            version: MEDIA_HEADER_VERSION,
                            stream_id: 2,
                            direction: MediaDirection::ClientToHost,
                            codec: AudioCodec::PcmS16Le,
                            sequence: 0,
                            timestamp_samples: 0,
                            sample_rate_hz: format.sample_rate_hz,
                            channels: format.channels,
                            payload_bytes: payload.len() as u32,
                        },
                        &payload,
                    )?;
                }
            }
            client.get_state()?;
        }
        if matches!(&event, HostBridgeEvent::Server(ServerMessage::State(_)))
            && exercise_scope
            && radio_selected
            && !scope_started
        {
            client.configure_scope(Some("scope-config".into()), ScopeConfiguration::default())?;
            client.start_scope(Some("scope-start".into()))?;
            scope_started = true;
        }
    }
    if scope_started {
        client.stop_scope(Some("scope-stop".into()))?;
    }
    client.shutdown()?;
    Ok(())
}
