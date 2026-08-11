use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serialport::SerialPort;
use std::{
    fs,
    io::ErrorKind,
    io::Read,
    io::Write,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
    time::Instant,
};

const CI_V_FRAME_START: u8 = 0xFE;
const CI_V_FRAME_END: u8 = 0xFD;
const MIN_SCOPE_BINS_FOR_DISPLAY: usize = 180;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Usb,
    Lsb,
    Cw,
    Data,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseMode {
    Lsb,
    Usb,
    Am,
    Cw,
    Rtty,
    Fm,
    Wfm,
    CwR,
    RttyR,
    Unknown(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperatingMode {
    pub base: BaseMode,
    pub data_mode: bool,
    pub filter: Option<u8>,
}

impl OperatingMode {
    pub fn label(self) -> String {
        let base = match self.base {
            BaseMode::Lsb => "LSB",
            BaseMode::Usb => "USB",
            BaseMode::Am => "AM",
            BaseMode::Cw => "CW",
            BaseMode::Rtty => "RTTY",
            BaseMode::Fm => "FM",
            BaseMode::Wfm => "WFM",
            BaseMode::CwR => "CW-R",
            BaseMode::RttyR => "RTTY-R",
            BaseMode::Unknown(v) => return format!("MODE_{v:#04x}"),
        };

        if self.data_mode && matches!(self.base, BaseMode::Lsb | BaseMode::Usb | BaseMode::Am | BaseMode::Fm) {
            format!("{base}-D")
        } else {
            base.to_string()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlId {
    AfGain,
    RfGain,
    Squelch,
    RfPower,
    Preamp,
    Attenuator,
    NoiseBlanker,
    NoiseReduction,
    DataMode,
    Filter,
    Agc,
    Rit,
    Xit,
    Split,
    Tuner,
    RawCiV,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ControlValue {
    Bool(bool),
    U8(u8),
    I32(i32),
    U64(u64),
    Mode(Mode),
    Text(String),
    Raw(Vec<u8>),
}

#[derive(Debug, Clone, Default)]
pub struct RadioCapabilities {
    pub can_get_frequency: bool,
    pub can_set_frequency: bool,
    pub can_get_mode: bool,
    pub can_set_mode: bool,
    pub can_get_ptt: bool,
    pub can_set_ptt: bool,
    pub can_raw_protocol: bool,
}

#[async_trait]
pub trait RadioHal: Send + Sync {
    async fn get_frequency_hz(&self) -> Result<u64>;
    async fn set_frequency_hz(&self, hz: u64) -> Result<()>;

    async fn get_mode(&self) -> Result<Mode>;
    async fn set_mode(&self, mode: Mode) -> Result<()>;

    async fn set_ptt(&self, enabled: bool) -> Result<()>;

    async fn protocol_write_read(&self, _request: &[u8]) -> Result<Vec<u8>> {
        Ok(Vec::new())
    }

    async fn get_control(&self, _id: ControlId) -> Result<Option<ControlValue>> {
        Ok(None)
    }

    async fn set_control(&self, _id: ControlId, _value: ControlValue) -> Result<()> {
        Ok(())
    }

    fn capabilities(&self) -> RadioCapabilities;
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

pub fn enumerate_serial_ports() -> Result<Vec<String>> {
    let mut ports = Vec::new();
    for entry in fs::read_dir("/dev")? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("ttyUSB") || name.starts_with("ttyACM") || name.starts_with("ttyS") {
            ports.push(format!("/dev/{name}"));
        }
    }
    ports.sort();
    Ok(ports)
}

#[derive(Clone)]
pub struct IcomCiVRadio {
    port: String,
    baud_rate: u32,
    controller_address: u8,
    radio_address: u8,
    serial_port: Arc<Mutex<Option<Box<dyn SerialPort>>>>,
}

#[derive(Debug, Clone, Copy)]
enum ControlOp {
    Set,
    Get,
}

#[derive(Debug, Clone, Copy)]
enum ControlEncoding {
    Bool,
    U8,
    Level255Bcd,
}

#[derive(Debug, Clone, Copy)]
struct CiVControlSpec {
    id: ControlId,
    cmd: u8,
    subcmd: u8,
    encoding: ControlEncoding,
}

const ICOM_CIV_CONTROL_SPECS: &[CiVControlSpec] = &[
    CiVControlSpec {
        id: ControlId::AfGain,
        cmd: 0x14,
        subcmd: 0x01,
        encoding: ControlEncoding::Level255Bcd,
    },
    CiVControlSpec {
        id: ControlId::RfGain,
        cmd: 0x14,
        subcmd: 0x02,
        encoding: ControlEncoding::Level255Bcd,
    },
    CiVControlSpec {
        id: ControlId::Squelch,
        cmd: 0x14,
        subcmd: 0x03,
        encoding: ControlEncoding::Level255Bcd,
    },
    CiVControlSpec {
        id: ControlId::RfPower,
        cmd: 0x14,
        subcmd: 0x0A,
        encoding: ControlEncoding::Level255Bcd,
    },
    CiVControlSpec {
        id: ControlId::Preamp,
        cmd: 0x16,
        subcmd: 0x02,
        encoding: ControlEncoding::Bool,
    },
    CiVControlSpec {
        id: ControlId::Attenuator,
        cmd: 0x11,
        subcmd: 0x00,
        encoding: ControlEncoding::Bool,
    },
    CiVControlSpec {
        id: ControlId::NoiseBlanker,
        cmd: 0x16,
        subcmd: 0x22,
        encoding: ControlEncoding::Bool,
    },
    CiVControlSpec {
        id: ControlId::NoiseReduction,
        cmd: 0x16,
        subcmd: 0x40,
        encoding: ControlEncoding::Bool,
    },
    CiVControlSpec {
        id: ControlId::Agc,
        cmd: 0x16,
        subcmd: 0x12,
        encoding: ControlEncoding::U8,
    },
    CiVControlSpec {
        id: ControlId::Split,
        cmd: 0x0F,
        subcmd: 0x00,
        encoding: ControlEncoding::Bool,
    },
];

impl IcomCiVRadio {
        fn set_operating_mode_blocking(&self, base_mode: BaseMode, data_mode: bool, filter: u8) -> Result<()> {
            let mode_byte = base_mode_to_civ_mode(base_mode)
                .with_context(|| format!("unsupported base mode for CI-V set: {base_mode:?}"))?;
            let filter = filter.clamp(1, 3);
            let data_byte = if data_mode { 0x01 } else { 0x00 };
            let _ = self.transact(&[0x26, 0x00, mode_byte, data_byte, filter], false)?;
            Ok(())
        }

        pub async fn set_operating_mode_details(&self, base_mode: BaseMode, data_mode: bool, filter: u8) -> Result<()> {
            self.set_operating_mode_blocking(base_mode, data_mode, filter)
        }

    pub fn new(port: impl Into<String>, baud_rate: u32, address: u8) -> Self {
        Self {
            port: port.into(),
            baud_rate,
            controller_address: address,
            radio_address: 0x94,
            serial_port: Arc::new(Mutex::new(None)),
        }
    }

    pub fn with_radio_address(mut self, radio_address: u8) -> Self {
        self.radio_address = radio_address;
        self
    }

    pub fn probe(&self) -> Result<RadioStatus> {
        self.probe_direct_serial()
    }

    pub async fn probe_stream_status(&self) -> Result<RadioStatus> {
        self.probe_stream_status_blocking()
    }

    fn probe_direct_serial(&self) -> Result<RadioStatus> {
        self.with_serial_port(Duration::from_millis(700), |port| {
            let freq_cmd = self.build_frame(0x03);
            self.write_frame(port, &freq_cmd)?;
            let freq = self.read_response_matching(port, Duration::from_millis(1200), |frame| {
                frame_matches_request(frame, &[0x03])
            })?;
            let freq_value = parse_frequency(&freq);

            let mode_cmd = self.build_frame(0x04);
            self.write_frame(port, &mode_cmd)?;
            let mode = self.read_response_matching(port, Duration::from_millis(1200), |frame| {
                frame_matches_request(frame, &[0x04])
            })?;
            let mode_details = parse_mode_details(&mode);
            let mode_value = mode_details.map(|m| m.label());

            Ok(RadioStatus {
                frequency_hz: freq_value,
                mode: mode_value,
                mode_details,
            })
        })
    }

    fn probe_stream_status_blocking(&self) -> Result<RadioStatus> {
        self.with_serial_port(Duration::from_millis(300), |port| {
            let freq_cmd = self.build_frame(0x03);
            self.write_frame(port, &freq_cmd)?;
            let freq = self.read_response_matching(port, Duration::from_millis(320), |frame| {
                frame_matches_request(frame, &[0x03])
            })?;
            let freq_value = parse_frequency(&freq);

            let mode_cmd = self.build_frame(0x04);
            self.write_frame(port, &mode_cmd)?;
            let mode = self.read_response_matching(port, Duration::from_millis(320), |frame| {
                frame_matches_request(frame, &[0x04])
            })?;
            let mode_details = parse_mode_details(&mode);
            let mode_value = mode_details.map(|m| m.label());

            Ok(RadioStatus {
                frequency_hz: freq_value,
                mode: mode_value,
                mode_details,
            })
        })
    }

    fn build_frame(&self, cmd: u8) -> Vec<u8> {
        self.build_frame_payload(&[cmd])
    }

    fn build_frame_payload(&self, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(5 + payload.len() + 1);
        out.push(CI_V_FRAME_START);
        out.push(CI_V_FRAME_START);
        out.push(self.radio_address);
        out.push(self.controller_address);
        out.extend_from_slice(payload);
        out.push(CI_V_FRAME_END);
        out
    }

    fn with_serial_port<T, F>(&self, timeout: Duration, mut operation: F) -> Result<T>
    where
        F: FnMut(&mut Box<dyn SerialPort>) -> Result<T>,
    {
        let mut guard = self
            .serial_port
            .lock()
            .map_err(|_| anyhow::anyhow!("radio serial port lock poisoned"))?;

        if guard.is_none() {
            *guard = Some(self.open_port_with_timeout(timeout)?);
        }

        let port = guard.as_mut().context("radio serial port slot unavailable")?;
        operation(port)
    }

    fn open_port_with_timeout(&self, timeout: Duration) -> Result<Box<dyn SerialPort>> {
        let configured_port = self.port.trim();
        let candidates = if configured_port.is_empty() {
            enumerate_serial_ports().unwrap_or_default()
        } else {
            vec![configured_port.to_string()]
        };

        let mut failures = Vec::new();
        for candidate in &candidates {
            let open_result = serialport::new(candidate, self.baud_rate)
                .timeout(timeout)
                .open();
            match open_result {
                Ok(port) => {
                    eprintln!("[rigforge-radio] opened serial port: {candidate}");
                    return Ok(port);
                }
                Err(err) => {
                    eprintln!("[rigforge-radio] failed to open serial port: {candidate}: {err}");
                    failures.push(format!("{candidate}: {err}"));
                }
            }
        }

        Err(anyhow!(
            "failed to open serial port{} (tried: {})",
            if configured_port.is_empty() {
                String::new()
            } else {
                format!(" {}", configured_port)
            },
            failures.join("; ")
        ))
    }

    fn close_serial_port(&self) {
        if let Ok(mut guard) = self.serial_port.lock() {
            *guard = None;
        }
    }


    fn write_frame(&self, port: &mut Box<dyn SerialPort>, frame: &[u8]) -> Result<()> {
        port.write_all(frame)
            .context("failed to write CI-V frame")?;
        thread::sleep(Duration::from_millis(120));
        Ok(())
    }

    fn read_response_matching<F>(
        &self,
        port: &mut Box<dyn SerialPort>,
        timeout: Duration,
        mut matcher: F,
    ) -> Result<Vec<u8>>
    where
        F: FnMut(&[u8]) -> bool,
    {
        let deadline = Instant::now() + timeout;
        let mut buf = [0u8; 1024];
        let mut out = Vec::new();

        while Instant::now() < deadline {
            match port.read(&mut buf) {
                Ok(bytes) if bytes > 0 => {
                    out.extend_from_slice(&buf[..bytes]);
                    for frame in extract_ci_v_frames(&out) {
                        if !is_radio_to_controller_frame(&frame, self.radio_address, self.controller_address) {
                            continue;
                        }

                        if matcher(&frame) {
                            return Ok(frame);
                        }
                    }
                }
                Ok(_) => {}
                Err(err) if err.kind() == ErrorKind::TimedOut => {
                    // keep waiting until timeout
                }
                Err(err) => return Err(err).context("failed to read matched CI-V response"),
            }

            thread::sleep(Duration::from_millis(10));
        }

        Ok(Vec::new())
    }

    fn read_any_frame(&self, port: &mut Box<dyn SerialPort>) -> Result<Vec<u8>> {
        let mut buf = [0u8; 256];
        let mut out = Vec::new();
        let mut attempts = 0;
        while attempts < 5 {
            match port.read(&mut buf) {
                Ok(bytes) if bytes > 0 => {
                    out.extend_from_slice(&buf[..bytes]);
                    if let Some(parsed) = extract_ci_v_frame(&out) {
                        return Ok(parsed.to_vec());
                    }
                }
                Ok(_) => {}
                Err(err) if err.kind() == ErrorKind::TimedOut => {
                    // transient serial idle; keep trying
                }
                Err(err) => {
                    return Err(err).context("failed to read CI-V response");
                }
            }
            attempts += 1;
            thread::sleep(Duration::from_millis(8));
        }
        Ok(out)
    }

    fn transact(&self, payload: &[u8], expect_data_frame: bool) -> Result<Vec<u8>> {
        let frame = self.build_frame_payload(payload);
        self.with_serial_port(Duration::from_millis(700), |port| {
            self.write_frame(port, &frame)?;

            if expect_data_frame {
                self.read_response_matching(port, Duration::from_millis(1500), |response| {
                    frame_matches_request(response, payload)
                })
            } else {
                self.read_any_frame(port)
            }
        })
    }

    fn set_frequency_blocking(&self, hz: u64) -> Result<()> {
        let mut payload = Vec::with_capacity(1 + 5);
        payload.push(0x05);
        payload.extend_from_slice(&encode_civ_frequency_bcd(hz));
        let _ = self.transact(&payload, false)?;
        Ok(())
    }

    fn set_mode_blocking(&self, mode: Mode) -> Result<()> {
        match mode {
            Mode::Data => {
                let response = self.transact(&[0x26, 0x00], true)?;
                let current_filter = parse_mode_details(&response)
                    .and_then(|d| d.filter)
                    .unwrap_or(1);
                self.set_operating_mode_blocking(BaseMode::Usb, true, current_filter)?;
            }
            _ => {
                let mode_byte = mode_to_civ_mode(mode)?;
                let _ = self.transact(&[0x06, mode_byte], false)?;
                // Explicitly clear DATA mode for non-data modes.
                let _ = self.transact(&[0x1A, 0x06, 0x00, 0x00], false)?;
            }
        }
        Ok(())
    }

    fn set_ptt_blocking(&self, enabled: bool) -> Result<()> {
        let payload = [0x1C, 0x00, if enabled { 0x01 } else { 0x00 }];
        let _ = self.transact(&payload, false)?;
        Ok(())
    }

    fn get_frequency_blocking(&self) -> Result<u64> {
        let response = self.transact(&[0x03], true)?;
        parse_frequency(&response).context("frequency not present in CI-V response")
    }

    fn get_mode_blocking(&self) -> Result<Mode> {
        let response = self.transact(&[0x04], true)?;
        parse_mode(&response).context("mode not present or unsupported in CI-V response")
    }

    fn run_control_op(&self, id: ControlId, op: ControlOp, value: Option<ControlValue>) -> Result<Option<ControlValue>> {
        if id == ControlId::DataMode {
            return match op {
                ControlOp::Set => {
                    let enabled = match value.context("missing control value for set operation")? {
                        ControlValue::Bool(v) => v,
                        _ => anyhow::bail!("DataMode control expects bool value"),
                    };
                    if enabled {
                        let _ = self.transact(&[0x1A, 0x06, 0x01, 0x01], false)?;
                    } else {
                        let _ = self.transact(&[0x1A, 0x06, 0x00, 0x00], false)?;
                    }
                    Ok(None)
                }
                ControlOp::Get => {
                    let response = self.transact(&[0x26, 0x00], true)?;
                    let details = parse_mode_details(&response).context("unable to decode mode details")?;
                    Ok(Some(ControlValue::Bool(details.data_mode)))
                }
            };
        }

        if id == ControlId::Filter {
            return match op {
                ControlOp::Set => {
                    let filter = match value.context("missing control value for set operation")? {
                        ControlValue::U8(v) if (1..=3).contains(&v) => v,
                        _ => anyhow::bail!("Filter control expects U8 value in 1..=3"),
                    };
                    let response = self.transact(&[0x26, 0x00], true)?;
                    let details = parse_mode_details(&response).context("unable to decode current mode for filter set")?;
                    let mode_byte = base_mode_to_civ_mode(details.base).context("unsupported base mode for filter set")?;
                    let data_byte = if details.data_mode { 0x01 } else { 0x00 };
                    let _ = self.transact(&[0x26, 0x00, mode_byte, data_byte, filter], false)?;
                    Ok(None)
                }
                ControlOp::Get => {
                    let response = self.transact(&[0x26, 0x00], true)?;
                    let details = parse_mode_details(&response).context("unable to decode mode details")?;
                    Ok(Some(ControlValue::U8(details.filter.unwrap_or(1))))
                }
            };
        }

        let spec = ICOM_CIV_CONTROL_SPECS
            .iter()
            .find(|spec| spec.id == id)
            .with_context(|| format!("unsupported CI-V control: {id:?}"))?;

        match op {
            ControlOp::Set => {
                let v = value.context("missing control value for set operation")?;
                let encoded = encode_control_value(spec.encoding, v)?;
                let mut payload = Vec::with_capacity(2 + encoded.len());
                payload.push(spec.cmd);
                payload.push(spec.subcmd);
                payload.extend_from_slice(&encoded);
                let _ = self.transact(&payload, false)?;
                Ok(None)
            }
            ControlOp::Get => {
                let payload = [spec.cmd, spec.subcmd];
                let response = self.transact(&payload, true)?;
                let decoded = decode_control_response(spec, &response)?;
                Ok(Some(decoded))
            }
        }
    }

    fn wait_for_spectrum_data_frame(
        &self,
        port: &mut Box<dyn SerialPort>,
        timeout: Duration,
    ) -> Result<(Option<Vec<u8>>, bool)> {
        let deadline = Instant::now() + timeout;
        let mut buf = [0u8; 1024];
        let mut out = Vec::new();
        let mut ack_frame: Option<Vec<u8>> = None;
        let mut saw_nak = false;

        while Instant::now() < deadline {
            match port.read(&mut buf) {
                Ok(bytes) if bytes > 0 => {
                    out.extend_from_slice(&buf[..bytes]);
                    for frame in extract_ci_v_frames(&out) {
                        if is_spectrum_data_frame(&frame) {
                            return Ok((Some(frame), saw_nak));
                        }
                        if is_nak_frame(&frame) {
                            saw_nak = true;
                        }
                        if is_ack_frame(&frame) {
                            ack_frame = Some(frame);
                        }
                    }
                }
                Ok(_) => {
                    thread::sleep(Duration::from_millis(20));
                }
                Err(err) if err.kind() == ErrorKind::TimedOut => {
                    // keep waiting until deadline
                }
                Err(err) => {
                    return Err(err).context("failed while waiting for spectrum data frame");
                }
            }
        }

        Ok((ack_frame, saw_nak))
    }

    fn enable_spectrum_stream_blocking(&self, timeout: Duration) -> Result<Vec<u8>> {
        let mut saw_nak = false;

        // Try progressively simpler/compatible bootstrap sequences across Icom variants:
        // A) full scope enable + output + request
        // B) output + request
        // C) request only (works when scope is already on)
        // D) legacy command fallback
        let sequences: &[&[&[u8]]] = &[
            &[&[0x27, 0x10, 0x01], &[0x27, 0x11, 0x01], &[0x27, 0x14, 0x00], &[0x27, 0x1A, 0x00], &[0x27, 0x00]],
            &[&[0x27, 0x10, 0x01], &[0x27, 0x20, 0x01], &[0x27, 0x00]],
            &[&[0x27, 0x11, 0x01], &[0x27, 0x00]],
            &[&[0x27, 0x20, 0x01], &[0x27, 0x00]],
            &[&[0x27, 0x00]],
            &[&[0x27, 0x12, 0x00]],
        ];

        for sequence in sequences {
            let result = self.with_serial_port(Duration::from_millis(700), |port| {
                for payload in *sequence {
                    self.write_frame(port, &self.build_frame_payload(payload))?;
                }

                self.wait_for_spectrum_data_frame(port, timeout)
            });

            let (frame, got_nak) = match result {
                Ok((frame, got_nak)) => (frame, got_nak),
                Err(err) => {
                    self.close_serial_port();
                    return Err(err);
                }
            };
            saw_nak |= got_nak;
            if let Some(frame) = frame {
                return Ok(frame);
            }
        }

        if saw_nak {
            anyhow::bail!(
                "radio rejected one or more scope bootstrap commands (NAK). RigForge already tried multiple CI-V scope strategies, including request-only fallback. On IC-7300, verify CI-V USB port is 'Unlink from [REMOTE]' and baud is Auto/115200"
            )
        } else {
            anyhow::bail!(
                "spectrum stream did not start within {:?} after trying scope bootstrap variants",
                timeout
            )
        }
    }

    fn disable_spectrum_stream_blocking(&self) -> Result<()> {
        let _ = self.transact(&[0x27, 0x11, 0x00], false)?;
        let _ = self.transact(&[0x27, 0x20, 0x00], false)?;
        let _ = self.transact(&[0x27, 0x10, 0x00], false)?;
        self.close_serial_port();
        Ok(())
    }

    pub async fn enable_spectrum_stream(&self, timeout: Duration) -> Result<Vec<u8>> {
        self.enable_spectrum_stream_blocking(timeout)
    }

    pub async fn disable_spectrum_stream(&self) -> Result<()> {
        self.disable_spectrum_stream_blocking()
    }

    pub async fn request_scope_waveform_bins(&self) -> Result<Vec<u8>> {
        self.request_scope_waveform_bins_blocking()
    }

    pub async fn try_scope_waveform_bins_stream(&self, timeout: Duration) -> Result<Option<Vec<u8>>> {
        self.try_scope_waveform_bins_stream_blocking(timeout)
    }

    pub async fn set_scope_sweep_speed(&self, speed: u8) -> Result<()> {
        // 0x00=FAST, 0x01=MID, 0x02=SLOW
        let sweep = speed.min(2);
        let _ = self.transact(&[0x27, 0x1A, sweep], false)?;
        Ok(())
    }

    fn request_scope_waveform_bins_blocking(&self) -> Result<Vec<u8>> {
        let frame = self.build_frame_payload(&[0x27, 0x00]);
        let mut last_response = Vec::new();
        let mut saw_request_nak = false;

        let result = self.with_serial_port(Duration::from_millis(500), |port| {
            // Fast path: if scope stream is already active (e.g., via 27 11/27 10),
            // read the next incoming waveform frame directly for low-latency updates.
            if let Some(bins) = self.read_stream_scope_bins(port, Duration::from_millis(220))? {
                return Ok(Some(bins));
            }

            let mut fallback_bins: Option<Vec<u8>> = None;
            let mut segmented_parts: Vec<Vec<u8>> = Vec::new();
            let mut segmented_total: Option<usize> = None;

            for _ in 0..4 {
                self.write_frame(port, &frame)?;

                // IC-7300 USB can emit scope waveform as sequential chunks (division 01..11).
                // Read multiple frames after each request and assemble when possible.
                for _ in 0..16 {
                    let response = self.read_any_frame(port)?;
                    if response.is_empty() {
                        continue;
                    }
                    last_response = response.clone();

                    if is_nak_frame(&response) {
                        saw_request_nak = true;
                        break;
                    }
                    if !is_spectrum_data_frame(&response) {
                        continue;
                    }

                    if let Some((part_idx, part_total, bins)) = parse_scope_waveform_segment(&response) {
                        if part_total > 0 {
                            if segmented_total.is_none() {
                                segmented_total = Some(part_total);
                                segmented_parts = vec![Vec::new(); part_total];
                            }
                            if let Some(total) = segmented_total {
                                if part_total == total && part_idx > 0 && part_idx <= total {
                                    segmented_parts[part_idx - 1] = bins;
                                    if segmented_parts.iter().all(|part| !part.is_empty()) {
                                        let mut merged = Vec::new();
                                        for part in &segmented_parts {
                                            merged.extend_from_slice(part);
                                        }
                                        if !merged.is_empty() {
                                            return Ok(Some(merged));
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if let Some(bins) = parse_scope_waveform_bins(&response) {
                        if !bins.is_empty() {
                            fallback_bins = Some(bins);
                        }
                    }
                }

                if let Some(bins) = fallback_bins {
                    return Ok(Some(bins));
                }

                if saw_request_nak {
                    break;
                }
            }

            if saw_request_nak {
                let _ = self.transact(&[0x27, 0x10, 0x01], false);
                let _ = self.transact(&[0x27, 0x11, 0x01], false);
                let _ = self.transact(&[0x27, 0x14, 0x00], false);

                let deadline = Instant::now() + Duration::from_millis(550);
                let mut segmented_parts: Vec<Vec<u8>> = Vec::new();
                let mut segmented_total: Option<usize> = None;
                let mut fallback_bins: Option<Vec<u8>> = None;

                while Instant::now() < deadline {
                    let response = self.read_any_frame(port)?;
                    if response.is_empty() || !is_spectrum_data_frame(&response) {
                        continue;
                    }

                    if let Some((part_idx, part_total, bins)) = parse_scope_waveform_segment(&response) {
                        if part_total > 0 {
                            if segmented_total.is_none() {
                                segmented_total = Some(part_total);
                                segmented_parts = vec![Vec::new(); part_total];
                            }
                            if let Some(total) = segmented_total {
                                if part_total == total && part_idx > 0 && part_idx <= total {
                                    segmented_parts[part_idx - 1] = bins;
                                    if segmented_parts.iter().all(|part| !part.is_empty()) {
                                        let mut merged = Vec::new();
                                        for part in &segmented_parts {
                                            merged.extend_from_slice(part);
                                        }
                                        if !merged.is_empty() {
                                            return Ok(Some(merged));
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if let Some(bins) = parse_scope_waveform_bins(&response) {
                        if !bins.is_empty() {
                            fallback_bins = Some(bins);
                        }
                    }
                }

                if let Some(bins) = fallback_bins {
                    return Ok(Some(bins));
                }
            }

            Ok(None)
        });

        let _bins: Option<Vec<u8>> = match result {
            Ok(Some(bins)) => return Ok(bins),
            Ok(None) => None,
            Err(err) => {
                self.close_serial_port();
                return Err(err);
            }
        };

        if saw_request_nak {
            let _ = self.transact(&[0x27, 0x10, 0x01], false);
            let _ = self.transact(&[0x27, 0x11, 0x01], false);
            let _ = self.transact(&[0x27, 0x14, 0x00], false);

            let deadline = Instant::now() + Duration::from_millis(550);
            let mut segmented_parts: Vec<Vec<u8>> = Vec::new();
            let mut segmented_total: Option<usize> = None;
            let mut fallback_bins: Option<Vec<u8>> = None;

            while Instant::now() < deadline {
                let response = self.with_serial_port(Duration::from_millis(500), |port| self.read_any_frame(port))?;
                if response.is_empty() || !is_spectrum_data_frame(&response) {
                    continue;
                }

                if let Some((part_idx, part_total, bins)) = parse_scope_waveform_segment(&response) {
                    if part_total > 0 {
                        if segmented_total.is_none() {
                            segmented_total = Some(part_total);
                            segmented_parts = vec![Vec::new(); part_total];
                        }
                        if let Some(total) = segmented_total {
                            if part_total == total && part_idx > 0 && part_idx <= total {
                                segmented_parts[part_idx - 1] = bins;
                                if segmented_parts.iter().all(|part| !part.is_empty()) {
                                    let mut merged = Vec::new();
                                    for part in &segmented_parts {
                                        merged.extend_from_slice(part);
                                    }
                                    if !merged.is_empty() {
                                        return Ok(merged);
                                    }
                                }
                            }
                        }
                    }
                }

                if let Some(bins) = parse_scope_waveform_bins(&response) {
                    if !bins.is_empty() {
                        fallback_bins = Some(bins);
                    }
                }
            }

            if let Some(bins) = fallback_bins {
                return Ok(bins);
            }
        }

        anyhow::bail!(
            "unable to parse scope waveform bins from CI-V response after repeated requests{}: {}",
            if saw_request_nak {
                " (including 27 11 stream fallback)"
            } else {
                ""
            },
            format_hex_bytes(&last_response)
        )
    }

    fn try_scope_waveform_bins_stream_blocking(&self, timeout: Duration) -> Result<Option<Vec<u8>>> {
        let result = self.with_serial_port(Duration::from_millis(500), |port| {
            self.read_stream_scope_bins(port, timeout)
        });

        match result {
            Ok(v) => Ok(v),
            Err(err) => {
                self.close_serial_port();
                Err(err)
            }
        }
    }

    fn read_stream_scope_bins(
        &self,
        port: &mut Box<dyn SerialPort>,
        timeout: Duration,
    ) -> Result<Option<Vec<u8>>> {
        let deadline = Instant::now() + timeout;
        let mut segmented_parts: Vec<Vec<u8>> = Vec::new();
        let mut segmented_total: Option<usize> = None;
        let mut best_fallback: Option<Vec<u8>> = None;

        while Instant::now() < deadline {
            let response = match self.read_any_frame(port) {
                Ok(r) => r,
                Err(_) => continue,
            };
            if response.is_empty() {
                continue;
            }
            if !is_spectrum_data_frame(&response) {
                continue;
            }

            if is_nak_frame(&response) {
                continue;
            }

            if let Some((part_idx, part_total, bins)) = parse_scope_waveform_segment(&response) {
                if part_total > 0 {
                    if segmented_total.is_none() {
                        segmented_total = Some(part_total);
                        segmented_parts = vec![Vec::new(); part_total];
                    }
                    if let Some(total) = segmented_total {
                        if part_total == total && part_idx > 0 && part_idx <= total {
                            segmented_parts[part_idx - 1] = bins.clone();
                            if segmented_parts.iter().all(|part| !part.is_empty()) {
                                let mut merged = Vec::new();
                                for part in &segmented_parts {
                                    merged.extend_from_slice(part);
                                }
                                if merged.len() >= MIN_SCOPE_BINS_FOR_DISPLAY {
                                    return Ok(Some(merged));
                                }
                            }
                        }
                    }
                }

                if !bins.is_empty()
                    && best_fallback
                        .as_ref()
                        .map(|b| bins.len() > b.len())
                        .unwrap_or(true)
                {
                    best_fallback = Some(bins);
                }
            }

            if let Some(bins) = parse_scope_waveform_bins(&response) {
                if !bins.is_empty() {
                    if best_fallback
                        .as_ref()
                        .map(|b| bins.len() > b.len())
                        .unwrap_or(true)
                    {
                        best_fallback = Some(bins);
                    }
                }
            }
        }

        Ok(best_fallback.filter(|b| b.len() >= MIN_SCOPE_BINS_FOR_DISPLAY))
    }
}

fn parse_scope_waveform_segment(frame: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    if frame.len() < 10 {
        return None;
    }
    if frame.first().copied() != Some(CI_V_FRAME_START)
        || frame.get(1).copied() != Some(CI_V_FRAME_START)
        || frame.last().copied() != Some(CI_V_FRAME_END)
    {
        return None;
    }
    if frame[4] != 0x27 || frame[5] != 0x00 {
        return None;
    }

    let payload = &frame[6..frame.len() - 1];
    if payload.len() < 3 {
        return None;
    }

    let current = decode_scope_division_number(payload[1]).or_else(|| decode_scope_division_number(payload[0]));
    let total = decode_scope_division_number(payload[2]).or_else(|| payload.get(1).and_then(|v| decode_scope_division_number(*v)));
    let (current, total) = (current?, total?);

    let data_offset = if payload.len() > 13 {
        13
    } else if payload.len() > 6 {
        6
    } else if payload.len() > 3 {
        3
    } else {
        payload.len()
    };

    let bins = payload[data_offset..].to_vec();
    Some((current as usize, total as usize, bins))
}

fn decode_scope_division_number(v: u8) -> Option<u8> {
    if (1..=11).contains(&v) {
        return Some(v);
    }

    let low = v & 0x0F;
    let high = (v >> 4) & 0x0F;
    if low > 9 || high > 9 {
        return None;
    }
    let n = high * 10 + low;
    if (1..=11).contains(&n) {
        Some(n)
    } else {
        None
    }
}

#[async_trait]
impl Radio for IcomCiVRadio {
    async fn frequency(&self) -> Result<u64> {
        self.get_frequency_blocking()
    }

    async fn set_frequency(&self, hz: u64) -> Result<()> {
        self.set_frequency_blocking(hz)
    }

    async fn mode(&self) -> Result<Mode> {
        self.get_mode_blocking()
    }

    async fn set_mode(&self, mode: Mode) -> Result<()> {
        self.set_mode_blocking(mode)
    }

    async fn ptt(&self, enabled: bool) -> Result<()> {
        self.set_ptt_blocking(enabled)
    }
}

#[async_trait]
impl RadioHal for IcomCiVRadio {
    async fn get_frequency_hz(&self) -> Result<u64> {
        self.get_frequency_blocking()
    }

    async fn set_frequency_hz(&self, hz: u64) -> Result<()> {
        self.set_frequency_blocking(hz)
    }

    async fn get_mode(&self) -> Result<Mode> {
        self.get_mode_blocking()
    }

    async fn set_mode(&self, mode: Mode) -> Result<()> {
        self.set_mode_blocking(mode)
    }

    async fn set_ptt(&self, enabled: bool) -> Result<()> {
        self.set_ptt_blocking(enabled)
    }

    async fn protocol_write_read(&self, request: &[u8]) -> Result<Vec<u8>> {
        if request.first().copied() != Some(CI_V_FRAME_START)
            || request.get(1).copied() != Some(CI_V_FRAME_START)
            || request.last().copied() != Some(CI_V_FRAME_END)
        {
            anyhow::bail!("raw CI-V request must be a full frame: FE FE ... FD");
        }

        self.with_serial_port(Duration::from_millis(700), |port| {
            self.write_frame(port, request)?;
            self.read_any_frame(port)
        })
    }

    async fn get_control(&self, id: ControlId) -> Result<Option<ControlValue>> {
        if id == ControlId::RawCiV {
            return Ok(None);
        }
        self.run_control_op(id, ControlOp::Get, None)
    }

    async fn set_control(&self, id: ControlId, value: ControlValue) -> Result<()> {
        if id == ControlId::RawCiV {
            anyhow::bail!("RawCiV is write/read through protocol_write_read, not set_control");
        }
        self.run_control_op(id, ControlOp::Set, Some(value))?;
        Ok(())
    }

    fn capabilities(&self) -> RadioCapabilities {
        RadioCapabilities {
            can_get_frequency: true,
            can_set_frequency: true,
            can_get_mode: true,
            can_set_mode: true,
            can_get_ptt: false,
            can_set_ptt: true,
            can_raw_protocol: true,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RadioStatus {
    pub frequency_hz: Option<u64>,
    pub mode: Option<String>,
    pub mode_details: Option<OperatingMode>,
}

fn extract_ci_v_frame(response: &[u8]) -> Option<&[u8]> {
    let starts: Vec<usize> = response
        .windows(2)
        .enumerate()
        .filter_map(|(idx, w)| (w[0] == CI_V_FRAME_START && w[1] == CI_V_FRAME_START).then_some(idx))
        .collect();

    for start in starts.into_iter().rev() {
        let candidate = &response[start..];
        let end = candidate.iter().position(|&b| b == CI_V_FRAME_END).map(|idx| start + idx)?;
                if end >= start + 5 {
            return Some(&response[start..=end]);
        }
    }
    None
}

fn extract_ci_v_frames(response: &[u8]) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    let mut idx = 0usize;

    while idx + 1 < response.len() {
        if response[idx] == CI_V_FRAME_START && response[idx + 1] == CI_V_FRAME_START {
            if let Some(rel_end) = response[idx..].iter().position(|&b| b == CI_V_FRAME_END) {
                let end = idx + rel_end;
                if end >= idx + 5 {
                    frames.push(response[idx..=end].to_vec());
                }
                idx = end + 1;
                continue;
            }
            break;
        }
        idx += 1;
    }

    frames
}

fn is_spectrum_data_frame(frame: &[u8]) -> bool {
    if frame.len() < 8 {
        return false;
    }

    let is_legacy = frame[4] == 0x27 && frame[5] == 0x12 && frame[6] == 0x01;
    let is_scope_wave = frame[4] == 0x27 && frame[5] == 0x00;

    frame.first().copied() == Some(CI_V_FRAME_START)
        && frame.get(1).copied() == Some(CI_V_FRAME_START)
        && frame.last().copied() == Some(CI_V_FRAME_END)
        && (is_legacy || is_scope_wave)
}

    fn is_ack_frame(frame: &[u8]) -> bool {
        frame.len() >= 6
        && frame.first().copied() == Some(CI_V_FRAME_START)
        && frame.get(1).copied() == Some(CI_V_FRAME_START)
        && frame.last().copied() == Some(CI_V_FRAME_END)
        && frame[4] == 0xFB
    }

    fn is_nak_frame(frame: &[u8]) -> bool {
        frame.len() >= 6
        && frame.first().copied() == Some(CI_V_FRAME_START)
        && frame.get(1).copied() == Some(CI_V_FRAME_START)
        && frame.last().copied() == Some(CI_V_FRAME_END)
        && frame[4] == 0xFA
    }

fn parse_scope_waveform_bins(frame: &[u8]) -> Option<Vec<u8>> {
    if frame.len() < 10 {
        return None;
    }
    if frame.first().copied() != Some(CI_V_FRAME_START)
        || frame.get(1).copied() != Some(CI_V_FRAME_START)
        || frame.last().copied() != Some(CI_V_FRAME_END)
    {
        return None;
    }
    if frame[4] != 0x27 || frame[5] != 0x00 {
        return None;
    }

    let payload = &frame[6..frame.len() - 1];

    // Scope waveform frames carry metadata first, followed by FFT bins.
    // Common IC-7300 layout:
    //   [division][mode][center_freq:5][span:5][fft_data...]
    // so bins typically start near offset 13.
    let bins = if payload.len() > 13 {
        payload[13..].to_vec()
    } else if payload.len() > 6 {
        payload[6..].to_vec()
    } else {
        Vec::new()
    };

    if bins.is_empty() {
        None
    } else {
        Some(bins)
    }
}

fn format_hex_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}


fn parse_frequency(response: &[u8]) -> Option<u64> {
    let payload = response
        .windows(2)
        .position(|w| w[0] == CI_V_FRAME_START && w[1] == CI_V_FRAME_START)
        .map(|idx| &response[idx..])?;

    let end = payload.iter().position(|&b| b == CI_V_FRAME_END)?;
    let frame = &payload[..=end];
    if frame.len() <= 6 {
        return None;
    }
    if frame[4] != 0x03 {
        return None;
    }

    let data = &frame[5..frame.len() - 1];
    if data.is_empty() {
        return None;
    }

    // CI-V frequency payload is little-endian BCD bytes (command 0x03/0x00 style).
    // IC-7300 returns 5 bytes for frequency. If there is a selector byte present,
    // the payload still starts at the data area and we decode the first 5 bytes.
    let freq_bytes = if data.len() >= 5 { &data[..5] } else { data };
    decode_civ_frequency_bcd(freq_bytes)
}

fn decode_civ_frequency_bcd(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() {
        return None;
    }

    let mut value = 0u64;
    let mut place = 1u64;

    for &b in bytes {
        let low = b & 0x0F;
        let high = (b >> 4) & 0x0F;
        if low > 9 || high > 9 {
            return None;
        }

        value += (low as u64) * place;
        place *= 10;
        value += (high as u64) * place;
        place *= 10;
    }

    Some(value)
}

fn encode_civ_frequency_bcd(hz: u64) -> [u8; 5] {
    let mut remaining = hz;
    let mut out = [0u8; 5];
    for b in &mut out {
        let low = (remaining % 10) as u8;
        remaining /= 10;
        let high = (remaining % 10) as u8;
        remaining /= 10;
        *b = (high << 4) | low;
    }
    out
}

fn mode_to_civ_mode(mode: Mode) -> Result<u8> {
    let v = match mode {
        Mode::Lsb => 0x00,
        Mode::Usb => 0x01,
        Mode::Cw => 0x03,
        Mode::Data => 0x01,
    };
    Ok(v)
}

#[cfg(test)]
fn civ_mode_to_mode(mode: u8) -> Option<Mode> {
    match mode {
        0x00 => Some(Mode::Lsb),
        0x01 => Some(Mode::Usb),
        0x03 | 0x07 => Some(Mode::Cw),
        _ => None,
    }
}

fn civ_mode_to_base_mode(mode: u8) -> BaseMode {
    match mode {
        0x00 => BaseMode::Lsb,
        0x01 => BaseMode::Usb,
        0x02 => BaseMode::Am,
        0x03 => BaseMode::Cw,
        0x04 => BaseMode::Rtty,
        0x05 => BaseMode::Fm,
        0x06 => BaseMode::Wfm,
        0x07 => BaseMode::CwR,
        0x08 => BaseMode::RttyR,
        other => BaseMode::Unknown(other),
    }
}

fn base_mode_to_civ_mode(mode: BaseMode) -> Option<u8> {
    match mode {
        BaseMode::Lsb => Some(0x00),
        BaseMode::Usb => Some(0x01),
        BaseMode::Am => Some(0x02),
        BaseMode::Cw => Some(0x03),
        BaseMode::Rtty => Some(0x04),
        BaseMode::Fm => Some(0x05),
        BaseMode::Wfm => Some(0x06),
        BaseMode::CwR => Some(0x07),
        BaseMode::RttyR => Some(0x08),
        BaseMode::Unknown(_) => None,
    }
}

fn encode_control_value(encoding: ControlEncoding, value: ControlValue) -> Result<Vec<u8>> {
    match (encoding, value) {
        (ControlEncoding::Bool, ControlValue::Bool(v)) => Ok(vec![if v { 1 } else { 0 }]),
        (ControlEncoding::U8, ControlValue::U8(v)) => Ok(vec![v]),
        (ControlEncoding::Level255Bcd, ControlValue::U8(v)) => Ok(encode_level_255_bcd(v)),
        _ => anyhow::bail!("control value type does not match control encoding"),
    }
}

fn decode_control_response(spec: &CiVControlSpec, response: &[u8]) -> Result<ControlValue> {
    let payload = response
        .windows(2)
        .position(|w| w[0] == CI_V_FRAME_START && w[1] == CI_V_FRAME_START)
        .map(|idx| &response[idx..])
        .context("no CI-V frame in response")?;

    let end = payload
        .iter()
        .position(|&b| b == CI_V_FRAME_END)
        .context("no CI-V frame terminator in response")?;
    let frame = &payload[..=end];
    if frame.len() < 8 {
        anyhow::bail!("CI-V control response frame too short")
    }
    if frame[4] != spec.cmd || frame[5] != spec.subcmd {
        anyhow::bail!(
            "CI-V control response command mismatch: expected {:02X} {:02X}, got {:02X} {:02X}",
            spec.cmd,
            spec.subcmd,
            frame[4],
            frame[5]
        )
    }
    let data = &frame[6..frame.len() - 1];

    let v = match spec.encoding {
        ControlEncoding::Bool => {
            let b = *data.first().context("missing boolean control response data")?;
            ControlValue::Bool(b != 0)
        }
        ControlEncoding::U8 => {
            let b = *data.first().context("missing U8 control response data")?;
            ControlValue::U8(b)
        }
        ControlEncoding::Level255Bcd => {
            let v = decode_level_255_bcd(data).context("invalid BCD level payload")?;
            ControlValue::U8(v)
        }
    };
    Ok(v)
}

fn encode_level_255_bcd(v: u8) -> Vec<u8> {
    let hundreds = v / 100;
    let tens = (v / 10) % 10;
    let ones = v % 10;
    vec![hundreds, (tens << 4) | ones]
}

fn decode_level_255_bcd(bytes: &[u8]) -> Option<u8> {
    if bytes.len() < 2 {
        return None;
    }
    let high = bytes[0] & 0x0F;
    let tens = (bytes[1] >> 4) & 0x0F;
    let ones = bytes[1] & 0x0F;
    if high > 2 || tens > 9 || ones > 9 {
        return None;
    }
    let value = high * 100 + tens * 10 + ones;
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_live_response_frame_from_radio() {
        let frame = [
            0xFE, 0xFE, 0xE0, 0x94, 0x03, 0x00, 0x80, 0x18, 0x07, 0x00, 0xFD,
        ];
        assert_eq!(parse_frequency(&frame), Some(7_188_000));
    }

    #[test]
    fn decodes_bcd_frequency_digits() {
        assert_eq!(decode_civ_frequency_bcd(&[0x00, 0x80, 0x18, 0x07, 0x00]), Some(7_188_000));
        assert_eq!(decode_civ_frequency_bcd(&[0x00, 0x40, 0x07, 0x14, 0x00]), Some(14_074_000));
    }

    #[test]
    fn encodes_bcd_frequency_digits() {
        assert_eq!(encode_civ_frequency_bcd(7_188_000), [0x00, 0x80, 0x18, 0x07, 0x00]);
        assert_eq!(encode_civ_frequency_bcd(14_074_000), [0x00, 0x40, 0x07, 0x14, 0x00]);
    }

    #[test]
    fn maps_mode_to_from_civ() {
        assert_eq!(mode_to_civ_mode(Mode::Lsb).ok(), Some(0x00));
        assert_eq!(mode_to_civ_mode(Mode::Usb).ok(), Some(0x01));
        assert_eq!(mode_to_civ_mode(Mode::Cw).ok(), Some(0x03));
        assert_eq!(civ_mode_to_mode(0x00), Some(Mode::Lsb));
        assert_eq!(civ_mode_to_mode(0x01), Some(Mode::Usb));
        assert_eq!(civ_mode_to_mode(0x03), Some(Mode::Cw));
        assert_eq!(civ_mode_to_mode(0x05), None);
    }

    #[test]
    fn parses_mode_details_with_data_and_filter() {
        let frame = [0xFE, 0xFE, 0xE0, 0x94, 0x04, 0x01, 0x01, 0x02, 0xFD];
        let details = parse_mode_details(&frame).expect("mode details expected");
        assert_eq!(details.base, BaseMode::Usb);
        assert!(details.data_mode);
        assert_eq!(details.filter, Some(2));
        assert_eq!(details.label(), "USB-D");
    }

    #[test]
    fn uses_configured_port_when_present() {
        let radio = IcomCiVRadio::new("/dev/ttyUSB0", 115_200, 0xE0);
        assert_eq!(radio.port, "/dev/ttyUSB0");
    }

    #[test]
    fn falls_back_to_enumerated_ports_when_config_is_missing() {
        let radio = IcomCiVRadio::new("", 115_200, 0xE0);
        assert_eq!(radio.port, "");
    }

    #[test]
    fn bcd_level_roundtrip() {
        let encoded = encode_level_255_bcd(173);
        assert_eq!(encoded, vec![0x01, 0x73]);
        assert_eq!(decode_level_255_bcd(&encoded), Some(173));
    }

    #[test]
    fn parses_captured_ft8_frames_fixture() {
        let fixture = include_str!("../tests/fixtures/ic7300_ft8_status_frames.txt");
        let mut lines = fixture
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'));

        let freq_line = lines.next().expect("freq frame in fixture");
        let mode_line = lines.next().expect("mode frame in fixture");

        let freq = parse_frequency(&parse_hex_line(freq_line)).expect("frequency decode from fixture");
        let mode = parse_mode_details(&parse_hex_line(mode_line)).expect("mode decode from fixture");

        assert_eq!(freq, 14_074_000);
        assert_eq!(mode.label(), "USB-D");
        assert_eq!(mode.filter, Some(1));
    }

    #[test]
    fn detects_spectrum_data_frame_shape() {
        let legacy = [0xFE, 0xFE, 0xE0, 0x94, 0x27, 0x12, 0x01, 0x10, 0x00, 0xAB, 0xFD];
        let scope_wave = [0xFE, 0xFE, 0xE0, 0x94, 0x27, 0x00, 0x01, 0x11, 0xFD];
        assert!(is_spectrum_data_frame(&legacy));
        assert!(is_spectrum_data_frame(&scope_wave));
    }

    #[test]
    fn extracts_multiple_frames_from_buffer() {
        let ack = [0xFE, 0xFE, 0xE0, 0x94, 0xFB, 0xFD];
        let spec = [0xFE, 0xFE, 0xE0, 0x94, 0x27, 0x12, 0x01, 0x10, 0x00, 0xAB, 0xFD];
        let mut buf = Vec::new();
        buf.extend_from_slice(&ack);
        buf.extend_from_slice(&spec);

        let frames = extract_ci_v_frames(&buf);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], ack);
        assert_eq!(frames[1], spec);
        assert!(is_spectrum_data_frame(&frames[1]));
    }

    #[test]
    fn detects_nak_frame() {
        let nak = [0xFE, 0xFE, 0xE0, 0x94, 0xFA, 0xFD];
        assert!(is_nak_frame(&nak));
    }

    #[test]
    fn detects_ack_frame() {
        let ack = [0xFE, 0xFE, 0xE0, 0x94, 0xFB, 0xFD];
        assert!(is_ack_frame(&ack));
    }

    #[test]
    fn parses_scope_waveform_bins() {
        let frame = [
            0xFE, 0xFE, 0xE0, 0x94, 0x27, 0x00,
            0x00, 0x01, 0x0B, 0x00, 0x14, 0x32,
            0x20, 0x80, 0xFF, 0x40, 0xFD,
        ];
        let bins = parse_scope_waveform_bins(&frame).expect("bins expected");
        assert_eq!(bins, vec![0x20, 0x80, 0xFF, 0x40]);
    }

    #[test]
    fn rejects_non_scope_waveform_frame_for_bins() {
        let frame = [0xFE, 0xFE, 0xE0, 0x94, 0x04, 0x01, 0x01, 0xFD];
        assert!(parse_scope_waveform_bins(&frame).is_none());
    }

    fn parse_hex_line(line: &str) -> Vec<u8> {
        line.split_whitespace()
            .map(|t| u8::from_str_radix(t, 16).expect("valid hex token"))
            .collect()
    }
}

fn parse_mode(response: &[u8]) -> Option<Mode> {
    let details = parse_mode_details(response)?;
    let base = match details.base {
        BaseMode::Lsb => Mode::Lsb,
        BaseMode::Usb => Mode::Usb,
        BaseMode::Cw | BaseMode::CwR => Mode::Cw,
        _ => return None,
    };
    if details.data_mode {
        Some(Mode::Data)
    } else {
        Some(base)
    }
}

fn parse_mode_details(response: &[u8]) -> Option<OperatingMode> {
    let payload = response
        .windows(2)
        .position(|w| w[0] == CI_V_FRAME_START && w[1] == CI_V_FRAME_START)
        .map(|idx| &response[idx..])?;

    let end = payload.iter().position(|&b| b == CI_V_FRAME_END)?;
    let frame = &payload[..=end];
    if frame.len() <= 6 {
        return None;
    }
    if frame[4] != 0x04 && frame[4] != 0x26 {
        return None;
    }

    let data = &frame[5..frame.len() - 1];
    let mode = *data.first()?;
    let data_on = data.get(1).copied().unwrap_or(0) != 0;
    let filter = data.get(2).copied().filter(|v| (1..=3).contains(v));
    Some(OperatingMode {
        base: civ_mode_to_base_mode(mode),
        data_mode: data_on,
        filter,
    })
}

fn frame_matches_request(frame: &[u8], payload: &[u8]) -> bool {
    if payload.is_empty() || frame.len() < 6 {
        return false;
    }
    if frame.first().copied() != Some(CI_V_FRAME_START)
        || frame.get(1).copied() != Some(CI_V_FRAME_START)
        || frame.last().copied() != Some(CI_V_FRAME_END)
    {
        return false;
    }

    let expected_cmd = payload[0];
    let response_cmd = frame[4];
    if response_cmd != expected_cmd {
        // Some Icom rigs may answer mode-detail-style queries with 0x04 payloads.
        if expected_cmd == 0x26 && response_cmd == 0x04 {
            return true;
        }
        return false;
    }

    // For command families with subcommand addressing, match subcommand too.
    if payload.len() >= 2 {
        let cmd_has_sub = matches!(
            expected_cmd,
            0x11 | 0x14 | 0x15 | 0x16 | 0x1A | 0x1C | 0x1F | 0x26 | 0x27
        );
        if cmd_has_sub {
            return frame.get(5).copied() == Some(payload[1]);
        }
    }

    true
}

fn is_radio_to_controller_frame(frame: &[u8], radio_address: u8, controller_address: u8) -> bool {
    frame.len() >= 6
        && frame.first().copied() == Some(CI_V_FRAME_START)
        && frame.get(1).copied() == Some(CI_V_FRAME_START)
        && frame.last().copied() == Some(CI_V_FRAME_END)
        && frame.get(2).copied() == Some(controller_address)
        && frame.get(3).copied() == Some(radio_address)
}

