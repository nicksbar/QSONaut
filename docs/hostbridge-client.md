# HostBridge client boundary

QSONaut owns the HostBridge client workflow through the
`qsonaut-hostbridge-client` crate. The crate is deliberately smaller than the
GUI: it owns WebSocket sessions, authentication, wire/media validation, and
reconnect signals; QSONaut presentation and operating policy consume events.

## Start a session

```rust,no_run
use qsonaut_hostbridge_client::{HostBridgeClient, HostBridgeConfig};

let (client, mut events) = HostBridgeClient::spawn(HostBridgeConfig {
    endpoint: "ws://192.168.1.107:8765".into(),
    client_name: "QSONaut desktop".into(),
    access_key: "station-1".into(),
    password: "configured-secret".into(),
    ..Default::default()
});

while let Some(event) = events.recv().await {
    // Connected contains the dynamic host radio/audio catalogs.
    // Media contains validated host-to-client PCM frames.
    // SafetyDisarmed clears every local TX arm/autosequence.
    let _ = event;
}

client.shutdown()?;
# Ok::<(), anyhow::Error>(())
```

After `Connected`, QSONaut selects the actual host-side IDs from the returned
capabilities:

```rust,no_run
# let host_radio_id = "host-radio".to_owned();
# let host_audio_id = "host-input".to_owned();
# let host_output_id = "host-output".to_owned();
# let receive_format = qsonaut_hostbridge_protocol::AudioFormat { codec: qsonaut_hostbridge_protocol::AudioCodec::PcmS16Le, channels: 1, sample_rate_hz: 48_000 };
# let transmit_format = receive_format.clone();
client.select_radio(host_radio_id)?;
client.select_audio(true, host_audio_id, receive_format)?;
client.select_audio_output(true, host_output_id, transmit_format)?;
client.set_frequency(14_074_000)?;
client.set_mode(qsonaut_hostbridge_protocol::WireMode::Usb)?;
```

Once the radio is selected, the host sends `radio_capabilities`. This is the
authoritative driver surface for that physical radio: core frequency/mode/PTT
operations, readable and writable Rigwright controls, supported meters, and
tuner support. QSONaut should build its remote controls from this message and
must not expose local device paths or ask the operator to type a Rigwright
control identifier.

Control and meter operations are carried as typed protocol messages. Control
identifiers are stable wire keys derived from Rigwright's public `ControlId`
names (for example `RfPower` and `DataMode`); their read/write permissions are
advertised independently. Meter values are normalized to the Rigwright `u8`
range. Tuner operations are available only when `radio_capabilities.tuner` is
true. The HostBridge remains authoritative for unsupported operations,
hardware ownership, and all TX safety decisions.

The client must not assume device paths or compile-time catalog entries.
The GUI HostBridge backend stores the optional `radio_device_id`,
`audio_source_id`, and `audio_output_id` selections in the radio profile. Blank
IDs use the first compatible negotiated host entry. Host capture is converted
to QSONaut's canonical mono 48 kHz processing stream; digital TX emits bounded
PCM frames to the selected host output.

## Safety and reconnect rules

- A new `Connected` event is a new session; reacquire radio and audio choices.
- `SafetyDisarmed` must clear local PTT, modem TX, and autosequence state.
- Never replay `set_ptt(true)` across reconnect.
- Host-to-client media arrives as validated 48 kHz S16LE frames.
- Client-to-host media is rejected locally unless it has the protocol version,
  direction, and exact payload length required by HostBridge.
- Media selection never implies PTT.

The current API emits server acknowledgements/errors as `HostBridgeEvent::Server`.
HostBridge release/0.1.1 supports optional request IDs echoed in successful
acknowledgements, allowing the next integration layer to correlate concurrent
control operations without relying on message ordering.
Use `HostBridgeClient::new_request_id()` with the `*_with_request_id` methods
when the caller needs to correlate a specific control operation.
