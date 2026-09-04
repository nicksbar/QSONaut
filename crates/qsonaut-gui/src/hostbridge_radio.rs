use anyhow::{anyhow, Result};
use qsonaut_hostbridge_client::{HostBridgeClient, HostBridgeConfig, HostBridgeEvent};
use qsonaut_hostbridge_protocol::{
    control_id_key, HostHello, RadioCapabilitiesInfo, RadioState, ServerMessage, WireControlValue,
    WireMode,
};
use qsonaut_radio::{
    drivers::ConfiguredRadio, ControlId, IcomCiVRadio, LinkHealth, MeterId, Mode, Radio,
    RadioCapabilities, TunerStatus,
};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// QSONaut-owned remote radio handle. Host device paths and leases remain
/// entirely inside HostBridge; this handle only retains the negotiated
/// catalog entry and the session transport.
#[derive(Clone)]
pub(crate) struct HostBridgeRadio {
    client: Arc<HostBridgeClient>,
    events: Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<HostBridgeEvent>>>,
    state: Arc<Mutex<RadioState>>,
    capabilities: Arc<Mutex<RadioCapabilitiesInfo>>,
    link_health: Arc<Mutex<LinkHealth>>,
    raw_responses: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    hello: HostHello,
    device_id: String,
    driver: qsonaut_hostbridge_protocol::RadioDriver,
    model: Option<String>,
    baud_rate: Option<u32>,
    radio_address: Option<u8>,
    connected: Arc<Mutex<bool>>,
    media_queue: Arc<Mutex<VecDeque<Vec<f32>>>>,
    scope_queue: Arc<Mutex<VecDeque<Vec<u8>>>>,
    audio_source: Option<(String, qsonaut_hostbridge_protocol::AudioFormat)>,
    audio_output: Option<(String, qsonaut_hostbridge_protocol::AudioFormat)>,
    media_seen: Arc<AtomicBool>,
    meter_seen: Arc<AtomicBool>,
    pending_read: Arc<Mutex<Option<(String, std::time::Instant)>>>,
}

static REMOTE_MEDIA_QUEUE: OnceLock<Arc<Mutex<VecDeque<Vec<f32>>>>> = OnceLock::new();
static REMOTE_CLIENT: OnceLock<Mutex<Option<Arc<HostBridgeClient>>>> = OnceLock::new();

pub(crate) fn remote_media_queue() -> Arc<Mutex<VecDeque<Vec<f32>>>> {
    REMOTE_MEDIA_QUEUE
        .get_or_init(|| Arc::new(Mutex::new(VecDeque::with_capacity(8))))
        .clone()
}

fn new_scope_queue() -> Arc<Mutex<VecDeque<Vec<u8>>>> {
    Arc::new(Mutex::new(VecDeque::with_capacity(8)))
}

pub(crate) enum RadioHandle {
    Local(ConfiguredRadio),
    Remote(Box<HostBridgeRadio>),
}

impl From<ConfiguredRadio> for RadioHandle {
    fn from(radio: ConfiguredRadio) -> Self {
        Self::Local(radio)
    }
}

impl RadioHandle {
    pub(crate) fn hostbridge_catalog(&self) -> Option<HostHello> {
        match self {
            Self::Local(_) => None,
            Self::Remote(radio) => Some(radio.hello.clone()),
        }
    }

    pub(crate) fn pump_events(&self) -> Option<bool> {
        match self {
            Self::Local(_) => None,
            Self::Remote(radio) => {
                radio.pump_events();
                Some(radio.connected())
            }
        }
    }

    pub(crate) fn as_icom(&self) -> Option<&IcomCiVRadio> {
        match self {
            Self::Local(radio) => radio.as_icom(),
            Self::Remote(_) => None,
        }
    }

    pub(crate) fn is_remote(&self) -> bool {
        matches!(self, Self::Remote(_))
    }

    pub(crate) fn remote_scope_queue(&self) -> Option<Arc<Mutex<VecDeque<Vec<u8>>>>> {
        match self {
            Self::Local(_) => None,
            Self::Remote(radio) => Some(radio.scope_queue.clone()),
        }
    }

    pub(crate) fn remote_scope_client(&self) -> Option<Arc<HostBridgeClient>> {
        match self {
            Self::Local(_) => None,
            Self::Remote(radio) => Some(radio.client.clone()),
        }
    }

    pub(crate) fn remote_scope_supported(&self) -> bool {
        match self {
            Self::Local(_) => false,
            Self::Remote(radio) => radio.capabilities_snapshot().scope,
        }
    }

    pub(crate) fn capabilities(&self) -> RadioCapabilities {
        match self {
            Self::Local(radio) => radio.capabilities(),
            Self::Remote(radio) => radio.capabilities(),
        }
    }
    pub(crate) fn supported_controls(&self) -> Vec<ControlId> {
        match self {
            Self::Local(radio) => radio.supported_controls(),
            Self::Remote(radio) => radio.supported_controls(),
        }
    }
    pub(crate) fn supported_meters(&self) -> Vec<MeterId> {
        match self {
            Self::Local(radio) => radio.supported_meters(),
            Self::Remote(radio) => radio.supported_meters(),
        }
    }
    pub(crate) fn supports_meter(&self, id: MeterId) -> bool {
        match self {
            Self::Local(radio) => radio.supports_meter(id),
            Self::Remote(radio) => radio.supports_meter(id),
        }
    }
    pub(crate) fn supports_control_write(&self, id: ControlId) -> bool {
        match self {
            Self::Local(radio) => radio.supports_control_write(id),
            Self::Remote(radio) => radio.supports_control_write(id),
        }
    }
    pub(crate) fn link_health(&self) -> LinkHealth {
        match self {
            Self::Local(radio) => radio.link_health(),
            Self::Remote(radio) => radio.link_health(),
        }
    }

    pub(crate) fn refresh_state(&self) -> Result<()> {
        match self {
            Self::Local(_) => Ok(()),
            Self::Remote(radio) => {
                radio.ensure_connected()?;
                radio.schedule_read("state", || radio.client.get_state())
            }
        }
    }
}

#[async_trait::async_trait]
impl Radio for RadioHandle {
    async fn get_frequency_hz(&self) -> Result<u64> {
        match self {
            Self::Local(radio) => radio.get_frequency_hz().await,
            Self::Remote(radio) => radio.get_frequency_hz().await,
        }
    }
    async fn set_frequency_hz(&self, hz: u64) -> Result<()> {
        match self {
            Self::Local(radio) => radio.set_frequency_hz(hz).await,
            Self::Remote(radio) => radio.set_frequency_hz(hz).await,
        }
    }
    async fn get_mode(&self) -> Result<Mode> {
        match self {
            Self::Local(radio) => radio.get_mode().await,
            Self::Remote(radio) => radio.get_mode().await,
        }
    }
    async fn set_mode(&self, mode: Mode) -> Result<()> {
        match self {
            Self::Local(radio) => radio.set_mode(mode).await,
            Self::Remote(radio) => radio.set_mode(mode).await,
        }
    }
    async fn set_ptt(&self, enabled: bool) -> Result<()> {
        match self {
            Self::Local(radio) => radio.set_ptt(enabled).await,
            Self::Remote(radio) => radio.set_ptt(enabled).await,
        }
    }
    async fn get_ptt(&self) -> Result<bool> {
        match self {
            Self::Local(radio) => radio.get_ptt().await,
            Self::Remote(radio) => radio.get_ptt().await,
        }
    }
    async fn get_control(&self, id: ControlId) -> Result<Option<qsonaut_radio::ControlValue>> {
        match self {
            Self::Local(radio) => radio.get_control(id).await,
            Self::Remote(radio) => {
                radio.ensure_connected()?;
                let key = control_id_key(id);
                // Remote replies are delivered by the session event pump.
                // Never wait here: this method is called from the radio
                // worker, which must keep pumping media and control events.
                radio.schedule_read(format!("control:{key}"), || {
                    radio.client.get_control(key.clone())
                })?;
                Ok(radio.state().controls.get(&key).cloned().map(Into::into))
            }
        }
    }
    async fn set_control(&self, id: ControlId, value: qsonaut_radio::ControlValue) -> Result<()> {
        match self {
            Self::Local(radio) => radio.set_control(id, value).await,
            Self::Remote(radio) => {
                radio.ensure_connected()?;
                let key = control_id_key(id);
                let wire_value: WireControlValue = value.clone().into();
                radio.client.set_control(key.clone(), wire_value)?;
                if let Ok(mut state) = radio.state.lock() {
                    state.controls.insert(key, value.into());
                }
                Ok(())
            }
        }
    }
    async fn get_meter(&self, id: MeterId) -> Result<Option<u8>> {
        match self {
            Self::Local(radio) => radio.get_meter(id).await,
            Self::Remote(radio) => {
                radio.ensure_connected()?;
                let wire_id = id.into();
                // Meter replies are asynchronous for the same reason as
                // controls. Return the last sample while the event pump
                // applies the newly requested sample when it arrives.
                radio.schedule_read(format!("meter:{wire_id:?}"), || {
                    radio.client.get_meter(wire_id)
                })?;
                Ok(radio.state().meters.get(&wire_id).copied())
            }
        }
    }
    async fn start_tuner(&self) -> Result<()> {
        match self {
            Self::Local(radio) => radio.start_tuner().await,
            Self::Remote(radio) => {
                radio.ensure_connected()?;
                radio.client.start_tuner()?;
                Ok(())
            }
        }
    }
    async fn get_tuner_status(&self) -> Result<Option<TunerStatus>> {
        match self {
            Self::Local(radio) => radio.get_tuner_status().await,
            Self::Remote(radio) => {
                radio.ensure_connected()?;
                radio.schedule_read("tuner", || radio.client.get_tuner_status())?;
                Ok(radio.state().tuner.map(Into::into))
            }
        }
    }
    fn capabilities(&self) -> RadioCapabilities {
        self.capabilities()
    }
}

impl HostBridgeRadio {
    pub(crate) async fn connect(config: HostBridgeConfig) -> Result<Self> {
        let requested_device_id = config.radio_device_id.clone();
        let requested_audio_source_id = config.audio_source_id.clone();
        let requested_audio_output_id = config.audio_output_id.clone();
        let driver = config.radio_driver;
        let model = config.radio_model.clone();
        let baud_rate = config.radio_baud_rate;
        let radio_address = config.radio_address;
        let (client, mut events) = HostBridgeClient::spawn(config);
        let client = Arc::new(client);
        let hello = tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                match events.recv().await {
                    Some(HostBridgeEvent::Connected(hello)) => break Ok(hello),
                    Some(HostBridgeEvent::SafetyDisarmed { reason }) => {
                        break Err(anyhow!("HostBridge session disarmed: {reason}"));
                    }
                    Some(HostBridgeEvent::Disconnected { reason }) => {
                        break Err(anyhow!("HostBridge disconnected during startup: {reason}"));
                    }
                    Some(_) => {}
                    None => break Err(anyhow!("HostBridge event stream closed during startup")),
                }
            }
        })
        .await
        .map_err(|_| anyhow!("timed out waiting for HostBridge authentication"))??;

        let device_id = if let Some(requested) = requested_device_id {
            let device = hello.capabilities.radio_devices.iter().find(|device| {
                device.id == requested
                    || requested
                        .rsplit_once(':')
                        .is_some_and(|(physical_id, _)| device.id == physical_id)
            });
            let device = device.ok_or_else(|| {
                anyhow!("saved HostBridge radio selection is no longer advertised")
            })?;
            if device.in_use {
                anyhow::bail!("saved HostBridge radio selection is currently in use")
            }
            device.id.clone()
        } else {
            hello
                .capabilities
                .radio_devices
                .iter()
                .find(|device| !device.in_use)
                .or_else(|| hello.capabilities.radio_devices.first())
                .map(|device| device.id.clone())
                .ok_or_else(|| anyhow!("HostBridge advertised no selectable radio devices"))?
        };

        let driver = driver.ok_or_else(|| anyhow!("HostBridge radio driver was not selected"))?;
        client.select_radio(
            device_id.clone(),
            driver,
            model.clone(),
            baud_rate,
            radio_address,
        )?;
        let audio_source = hello.capabilities.audio_sources.iter().find_map(|source| {
            if requested_audio_source_id
                .as_deref()
                .is_some_and(|requested| requested != source.id)
            {
                return None;
            }
            source
                .formats
                .iter()
                .any(|format| {
                    format.codec == qsonaut_hostbridge_protocol::AudioCodec::PcmS16Le
                        && format.sample_rate_hz == 48_000
                })
                .then(|| {
                    (
                        source.id.clone(),
                        source
                            .formats
                            .iter()
                            .find(|format| {
                                format.codec == qsonaut_hostbridge_protocol::AudioCodec::PcmS16Le
                                    && format.sample_rate_hz == 48_000
                            })
                            .expect("format selected by predicate")
                            .clone(),
                    )
                })
        });
        if let Some((source_id, format)) = &audio_source {
            client.select_audio(true, source_id.clone(), format.clone())?;
        }
        let audio_output = hello.capabilities.audio_outputs.iter().find_map(|output| {
            if requested_audio_output_id
                .as_deref()
                .is_some_and(|requested| requested != output.id)
            {
                return None;
            }
            output
                .formats
                .iter()
                .find(|format| {
                    format.codec == qsonaut_hostbridge_protocol::AudioCodec::PcmS16Le
                        && format.sample_rate_hz == 48_000
                })
                .map(|format| (output.id.clone(), format.clone()))
        });
        if let Some((output_id, format)) = &audio_output {
            client.select_audio_output(true, output_id.clone(), format.clone())?;
        }
        client.get_state()?;
        {
            let mut current = REMOTE_CLIENT
                .get_or_init(|| Mutex::new(None))
                .lock()
                .map_err(|_| anyhow!("HostBridge client registry is poisoned"))?;
            if current
                .as_ref()
                .is_some_and(|active| !Arc::ptr_eq(active, &client))
            {
                let _ = client.shutdown();
                anyhow::bail!(
                    "another HostBridge session is active; stop it before connecting this profile"
                );
            }
            if let Ok(mut queue) = remote_media_queue().lock() {
                queue.clear();
            }
            *current = Some(client.clone());
        }
        let (initial_state, capabilities) = tokio::time::timeout(Duration::from_secs(5), async {
            let mut capabilities = None;
            loop {
                match events.recv().await {
                    Some(HostBridgeEvent::Server(ServerMessage::RadioCapabilities(next))) => {
                        capabilities = Some(next);
                    }
                    Some(HostBridgeEvent::Server(ServerMessage::State(state))) => {
                        break Ok((state, capabilities.unwrap_or_default()))
                    }
                    Some(HostBridgeEvent::SafetyDisarmed { reason }) => {
                        break Err(anyhow!(
                            "HostBridge session disarmed during startup: {reason}"
                        ));
                    }
                    Some(HostBridgeEvent::Disconnected { reason }) => {
                        break Err(anyhow!("HostBridge disconnected during startup: {reason}"));
                    }
                    Some(_) => {}
                    None => break Err(anyhow!("HostBridge event stream closed during startup")),
                }
            }
        })
        .await
        .map_err(|_| anyhow!("timed out waiting for initial HostBridge radio state"))??;
        let radio = Self {
            client,
            events: Arc::new(Mutex::new(events)),
            state: Arc::new(Mutex::new(initial_state)),
            capabilities: Arc::new(Mutex::new(capabilities)),
            link_health: Arc::new(Mutex::new(LinkHealth::default())),
            raw_responses: Arc::new(Mutex::new(HashMap::new())),
            hello,
            device_id,
            driver,
            model,
            baud_rate,
            radio_address,
            connected: Arc::new(Mutex::new(true)),
            media_queue: remote_media_queue(),
            scope_queue: new_scope_queue(),
            audio_source,
            audio_output,
            media_seen: Arc::new(AtomicBool::new(false)),
            meter_seen: Arc::new(AtomicBool::new(false)),
            pending_read: Arc::new(Mutex::new(None)),
        };
        let event_pump = radio.clone();
        std::thread::Builder::new()
            .name("qsonaut-hostbridge-events".to_string())
            .spawn(move || {
                while event_pump.pump_events() {
                    std::thread::sleep(Duration::from_millis(2));
                }
            })
            .map_err(|error| anyhow!("failed to start HostBridge event pump: {error}"))?;
        Ok(radio)
    }

    pub(crate) fn host_name(&self) -> &str {
        &self.hello.host_name
    }

    pub(crate) fn device_id(&self) -> &str {
        &self.device_id
    }

    pub(crate) fn capabilities_advertised(&self) -> &qsonaut_hostbridge_protocol::Capabilities {
        &self.hello.capabilities
    }

    pub(crate) fn pump_events(&self) -> bool {
        let Ok(mut events) = self.events.lock() else {
            return false;
        };
        loop {
            let event = match events.try_recv() {
                Ok(event) => event,
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => return true,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => return false,
            };
            match event {
                HostBridgeEvent::Server(ServerMessage::State(next)) => {
                    self.clear_pending_read("state");
                    if let Ok(mut state) = self.state.lock() {
                        // HostBridge state snapshots currently contain core
                        // state. Meter/control replies arrive asynchronously;
                        // replacing the snapshot here erased those values on
                        // the next frequency poll before the UI could render
                        // them.
                        if next.frequency_hz.is_some() {
                            state.frequency_hz = next.frequency_hz;
                        }
                        if next.mode.is_some() {
                            state.mode = next.mode;
                        }
                        if next.ptt.is_some() {
                            state.ptt = next.ptt;
                        }
                        state.controls.extend(next.controls);
                        state.meters.extend(next.meters);
                        if next.tuner.is_some() {
                            state.tuner = next.tuner;
                        }
                    }
                }
                HostBridgeEvent::Server(ServerMessage::RadioCapabilities(next)) => {
                    if let Ok(mut capabilities) = self.capabilities.lock() {
                        *capabilities = next;
                    }
                }
                HostBridgeEvent::Server(ServerMessage::ControlValue {
                    control_id, value, ..
                }) => {
                    self.clear_pending_read(&format!("control:{control_id}"));
                    if let Some(value) = value {
                        if let Ok(mut state) = self.state.lock() {
                            state.controls.insert(control_id, value);
                        }
                    }
                }
                HostBridgeEvent::Server(ServerMessage::MeterValue {
                    meter_id, value, ..
                }) => {
                    self.clear_pending_read(&format!("meter:{meter_id:?}"));
                    if let Some(value) = value {
                        if !self.meter_seen.swap(true, Ordering::Relaxed) {
                            tracing::info!(meter = ?meter_id, value, "First HostBridge meter response received");
                        }
                        if let Ok(mut state) = self.state.lock() {
                            state.meters.insert(meter_id, value);
                        }
                    }
                }
                HostBridgeEvent::Server(ServerMessage::TunerStatus { status, .. }) => {
                    self.clear_pending_read("tuner");
                    if let Ok(mut state) = self.state.lock() {
                        state.tuner = status;
                    }
                }
                HostBridgeEvent::Server(ServerMessage::LinkHealth { health, .. }) => {
                    if let Ok(mut current) = self.link_health.lock() {
                        *current = LinkHealth::from(health);
                    }
                }
                HostBridgeEvent::Server(ServerMessage::RawProtocol {
                    request_id: Some(request_id),
                    response,
                }) => {
                    if let Ok(mut responses) = self.raw_responses.lock() {
                        responses.insert(request_id, response);
                    }
                }
                HostBridgeEvent::Server(ServerMessage::Error {
                    code,
                    message,
                    request_id,
                }) => {
                    if code != "media_frames_dropped" {
                        self.clear_all_pending_reads();
                    }
                    tracing::warn!(?request_id, %code, %message, "HostBridge request failed");
                }
                HostBridgeEvent::SafetyDisarmed { .. }
                | HostBridgeEvent::Disconnected { .. }
                | HostBridgeEvent::Reconnecting => {
                    self.clear_all_pending_reads();
                    if let Ok(mut connected) = self.connected.lock() {
                        *connected = false;
                    }
                    if let Ok(mut state) = self.state.lock() {
                        state.ptt = Some(false);
                    }
                    if let Ok(mut queue) = self.media_queue.lock() {
                        queue.clear();
                    }
                }
                HostBridgeEvent::Connected(_) => {
                    // A reconnect creates a new host session and releases the
                    // old lease. Reacquire only the selected host radio; TX
                    // state is intentionally never replayed.
                    let _ = self.client.select_radio(
                        self.device_id.clone(),
                        self.driver,
                        self.model.clone(),
                        self.baud_rate,
                        self.radio_address,
                    );
                    if let Some((source_id, format)) = &self.audio_source {
                        let _ = self
                            .client
                            .select_audio(true, source_id.clone(), format.clone());
                    }
                    if let Some((output_id, format)) = &self.audio_output {
                        let _ = self.client.select_audio_output(
                            true,
                            output_id.clone(),
                            format.clone(),
                        );
                    }
                    // Audio source selection is repeated by the host session
                    // policy on reconnect; this queue must never retain stale
                    // frames from the previous session.
                    if let Ok(mut queue) = self.media_queue.lock() {
                        queue.clear();
                    }
                    let _ = self.client.get_state();
                    if let Ok(mut connected) = self.connected.lock() {
                        *connected = true;
                    }
                }
                HostBridgeEvent::Media { header, payload } => {
                    if header.sample_rate_hz != 48_000
                        || header.codec != qsonaut_hostbridge_protocol::AudioCodec::PcmS16Le
                        || header.channels == 0
                    {
                        continue;
                    }
                    let samples = pcm_s16le_to_mono(&payload, header.channels);
                    if !self.media_seen.swap(true, Ordering::Relaxed) {
                        tracing::info!(
                            samples = samples.len(),
                            payload_bytes = payload.len(),
                            channels = header.channels,
                            "First HostBridge RX media frame received"
                        );
                    }
                    if let Ok(mut queue) = self.media_queue.lock() {
                        if queue.len() >= 8 {
                            queue.pop_front();
                        }
                        queue.push_back(samples);
                    }
                }
                HostBridgeEvent::Server(ServerMessage::ScopeFrame { bins }) => {
                    if let Ok(mut queue) = self.scope_queue.lock() {
                        if queue.len() >= 8 {
                            queue.pop_front();
                        }
                        queue.push_back(bins);
                    }
                }
                HostBridgeEvent::Server(_) => {}
            }
        }
    }

    fn connected(&self) -> bool {
        self.connected.lock().map(|value| *value).unwrap_or(false)
    }

    fn ensure_connected(&self) -> Result<()> {
        self.pump_events();
        if self.connected.lock().map(|value| *value).unwrap_or(false) {
            Ok(())
        } else {
            Err(anyhow!("HostBridge radio session is disconnected"))
        }
    }

    fn state(&self) -> RadioState {
        self.pump_events();
        self.state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_default()
    }

    fn schedule_read<F>(&self, key: impl Into<String>, send: F) -> Result<()>
    where
        F: FnOnce() -> Result<()>,
    {
        let key = key.into();
        let mut pending = self
            .pending_read
            .lock()
            .map_err(|_| anyhow!("HostBridge pending-read state poisoned"))?;
        const REMOTE_READ_TIMEOUT: Duration = Duration::from_secs(2);
        if pending
            .as_ref()
            .is_some_and(|(_, started)| started.elapsed() < REMOTE_READ_TIMEOUT)
        {
            return Ok(());
        }
        // Record the request before enqueueing it. The transport can receive
        // a very fast HostBridge reply on its event thread before send()
        // returns; recording afterwards loses that clear and permanently
        // suppresses later control/meter reads.
        *pending = Some((key.clone(), std::time::Instant::now()));
        drop(pending);
        if let Err(error) = send() {
            self.clear_pending_read(&key);
            return Err(error);
        }
        Ok(())
    }

    fn clear_pending_read(&self, key: &str) {
        if let Ok(mut pending) = self.pending_read.lock() {
            if pending
                .as_ref()
                .is_some_and(|(pending_key, _)| pending_key == key)
            {
                *pending = None;
            }
        }
    }

    fn clear_all_pending_reads(&self) {
        if let Ok(mut pending) = self.pending_read.lock() {
            *pending = None;
        }
    }

    fn capabilities_snapshot(&self) -> RadioCapabilitiesInfo {
        self.capabilities
            .lock()
            .map(|value| value.clone())
            .unwrap_or_default()
    }
}

impl Drop for HostBridgeRadio {
    fn drop(&mut self) {
        // The event-pump thread owns a clone of this handle, so dropping the
        // radio worker alone does not otherwise close the transport or release
        // HostBridge's exclusive radio lease. Explicit shutdown is idempotent
        // and causes the pump to exit after the session is closed.
        let _ = self.client.shutdown();
        if let Ok(mut current) = REMOTE_CLIENT.get_or_init(|| Mutex::new(None)).lock() {
            if current
                .as_ref()
                .is_some_and(|client| Arc::ptr_eq(client, &self.client))
            {
                *current = None;
            }
        }
        if let Ok(mut queue) = self.media_queue.lock() {
            queue.clear();
        }
    }
}

fn pcm_s16le_to_mono(payload: &[u8], channels: u8) -> Vec<f32> {
    let channels = usize::from(channels);
    payload
        .chunks_exact(2 * channels)
        .map(|frame| {
            let sum: i32 = frame
                .chunks_exact(2)
                .map(|sample| i16::from_le_bytes([sample[0], sample[1]]) as i32)
                .sum();
            sum as f32 / (channels as f32 * i16::MAX as f32)
        })
        .collect()
}

pub(crate) fn send_remote_pcm(pcm: &[i16], sample_rate_hz: u32) -> Result<()> {
    let client = REMOTE_CLIENT
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| anyhow!("HostBridge client registry is poisoned"))?
        .clone()
        .ok_or_else(|| anyhow!("no HostBridge audio session is active"))?;
    send_remote_pcm_with_client(&client, pcm, sample_rate_hz)
}

fn send_remote_pcm_with_client(
    client: &HostBridgeClient,
    pcm: &[i16],
    sample_rate_hz: u32,
) -> Result<()> {
    const MAX_SAMPLES_PER_FRAME: usize = 120_000;
    for (sequence, chunk) in pcm.chunks(MAX_SAMPLES_PER_FRAME).enumerate() {
        let payload = chunk
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect::<Vec<_>>();
        client.send_media(
            qsonaut_hostbridge_protocol::MediaFrameHeader {
                version: qsonaut_hostbridge_protocol::MEDIA_HEADER_VERSION,
                stream_id: 1,
                direction: qsonaut_hostbridge_protocol::MediaDirection::ClientToHost,
                codec: qsonaut_hostbridge_protocol::AudioCodec::PcmS16Le,
                sequence: sequence as u64,
                timestamp_samples: (sequence * MAX_SAMPLES_PER_FRAME) as u64,
                sample_rate_hz,
                channels: 1,
                payload_bytes: payload.len() as u32,
            },
            &payload,
        )?;
    }
    Ok(())
}

#[async_trait::async_trait]
impl Radio for HostBridgeRadio {
    async fn get_frequency_hz(&self) -> Result<u64> {
        self.ensure_connected()?;
        // HostBridge pushes state changes through the background event pump.
        // Do not issue a synchronous GetState request on every 100 ms GUI
        // poll; that competes with CI-V scope and media delivery.
        self.state()
            .frequency_hz
            .ok_or_else(|| anyhow!("HostBridge has not supplied a radio frequency yet"))
    }

    async fn set_frequency_hz(&self, hz: u64) -> Result<()> {
        self.ensure_connected()?;
        self.client.set_frequency(hz)?;
        if let Ok(mut state) = self.state.lock() {
            state.frequency_hz = Some(hz);
        }
        Ok(())
    }

    async fn get_mode(&self) -> Result<Mode> {
        self.ensure_connected()?;
        self.state()
            .mode
            .map(mode_from_wire)
            .ok_or_else(|| anyhow!("HostBridge has not supplied a radio mode yet"))
    }

    async fn set_mode(&self, mode: Mode) -> Result<()> {
        self.ensure_connected()?;
        self.client.set_mode(wire_mode(mode))?;
        if let Ok(mut state) = self.state.lock() {
            state.mode = Some(wire_mode(mode));
        }
        Ok(())
    }

    async fn set_ptt(&self, enabled: bool) -> Result<()> {
        if enabled {
            self.ensure_connected()?;
        }
        self.client.set_ptt(enabled)?;
        if let Ok(mut state) = self.state.lock() {
            state.ptt = Some(enabled);
        }
        Ok(())
    }

    async fn get_ptt(&self) -> Result<bool> {
        self.ensure_connected()?;
        self.state()
            .ptt
            .ok_or_else(|| anyhow!("HostBridge has not supplied PTT state yet"))
    }

    fn capabilities(&self) -> RadioCapabilities {
        let capabilities = self.capabilities_snapshot();
        RadioCapabilities {
            can_get_frequency: capabilities.can_get_frequency,
            can_set_frequency: capabilities.can_set_frequency,
            can_get_mode: capabilities.can_get_mode,
            can_set_mode: capabilities.can_set_mode,
            can_get_ptt: capabilities.can_get_ptt,
            can_set_ptt: capabilities.can_set_ptt,
            can_get_power: capabilities.can_get_power,
            can_set_power: capabilities.can_set_power,
            can_raw_protocol: capabilities.can_raw_protocol,
        }
    }

    fn supports_meter(&self, id: MeterId) -> bool {
        self.capabilities_snapshot().meters.contains(&id.into())
    }
    fn supports_control(&self, id: ControlId) -> bool {
        self.capabilities_snapshot()
            .controls
            .iter()
            .any(|capability| capability.id == control_id_key(id))
    }
    fn supports_control_read(&self, id: ControlId) -> bool {
        self.capabilities_snapshot()
            .controls
            .iter()
            .any(|capability| capability.id == control_id_key(id) && capability.readable)
    }
    fn supports_control_write(&self, id: ControlId) -> bool {
        self.capabilities_snapshot()
            .controls
            .iter()
            .any(|capability| capability.id == control_id_key(id) && capability.writable)
    }
    fn link_health(&self) -> LinkHealth {
        self.pump_events();
        self.link_health
            .lock()
            .map(|health| *health)
            .unwrap_or_default()
    }
    async fn get_tuner_status(&self) -> Result<Option<TunerStatus>> {
        self.ensure_connected()?;
        self.client.get_tuner_status()?;
        Ok(self.state().tuner.map(Into::into))
    }
    async fn protocol_write_read(&self, request: &[u8]) -> Result<Vec<u8>> {
        self.ensure_connected()?;
        let request_id = HostBridgeClient::new_request_id();
        self.client
            .raw_protocol(Some(request_id.clone()), request.to_vec())?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            self.pump_events();
            if let Ok(mut responses) = self.raw_responses.lock() {
                if let Some(response) = responses.remove(&request_id) {
                    return Ok(response);
                }
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("timed out waiting for HostBridge raw protocol response")
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}

fn mode_from_wire(mode: WireMode) -> Mode {
    match mode {
        WireMode::Usb => Mode::Usb,
        WireMode::Lsb => Mode::Lsb,
        WireMode::Cw => Mode::Cw,
        WireMode::Data => Mode::Data,
        WireMode::Am => Mode::Am,
        WireMode::Fm => Mode::Fm,
        WireMode::Wfm => Mode::Wfm,
        WireMode::Rtty => Mode::Rtty,
        WireMode::CwReverse => Mode::CwReverse,
        WireMode::RttyReverse => Mode::RttyReverse,
    }
}

fn wire_mode(mode: Mode) -> WireMode {
    match mode {
        Mode::Usb => WireMode::Usb,
        Mode::Lsb => WireMode::Lsb,
        Mode::Cw => WireMode::Cw,
        Mode::Data => WireMode::Data,
        Mode::Am => WireMode::Am,
        Mode::Fm => WireMode::Fm,
        Mode::Wfm => WireMode::Wfm,
        Mode::Rtty => WireMode::Rtty,
        Mode::CwReverse => WireMode::CwReverse,
        Mode::RttyReverse => WireMode::RttyReverse,
    }
}

#[cfg(test)]
mod tests {
    use super::pcm_s16le_to_mono;

    #[test]
    fn remote_pcm_is_downmixed_to_normalized_mono() {
        let payload = [0xFF, 0x7F, 0x00, 0x80, 0x00, 0x40, 0x00, 0xC0];
        let samples = pcm_s16le_to_mono(&payload, 2);
        assert_eq!(samples.len(), 2);
        assert!(samples[0].abs() < 0.001);
        assert!(samples[1].abs() < 0.001);
    }
}
