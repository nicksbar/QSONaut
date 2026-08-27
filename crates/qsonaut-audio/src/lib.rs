use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
        Arc,
    },
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Context, Result};
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, Sample, SampleFormat, SampleRate, Stream, StreamError, SupportedStreamConfig,
};
use resample::{resample_f32, BandlimitedResampler};

pub mod resample;

pub const CANONICAL_SAMPLE_RATE_HZ: u32 = 48_000;
pub const CANONICAL_CHANNELS: u16 = 1;
const MONITOR_TARGET_LATENCY_MS: u32 = 120;
const MONITOR_MAX_ADJUSTMENT_PPM: i32 = 2_500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioDeviceKind {
    Input,
    Output,
}

#[derive(Debug, Clone)]
pub struct AudioService {
    preferred_device_name: Option<String>,
    enabled: bool,
}

pub struct AudioStream {
    _stream: Stream,
    samples_rx: Receiver<Vec<f32>>,
    errors_rx: Receiver<String>,
    pending: VecDeque<f32>,
    requested_channels: usize,
    device_sample_rate_hz: u32,
    output_sample_rate_hz: u32,
    device_channels: u16,
    device_sample_format: SampleFormat,
    fallback_attempts: Vec<String>,
}

pub struct AudioMonitor {
    samples_tx: SyncSender<Vec<f32>>,
    errors_rx: Receiver<String>,
    dropped_chunks: Arc<AtomicU64>,
    underruns: Arc<AtomicU64>,
    buffered_samples: Arc<AtomicUsize>,
    primed: Arc<AtomicBool>,
    clock_adjustment_ppm: Arc<AtomicI32>,
    resampler: std::sync::Mutex<MonitorResamplerState>,
    volume: Arc<std::sync::atomic::AtomicU32>,
    input_sample_rate_hz: u32,
    output_sample_rate_hz: u32,
    output_channels: u16,
    output_sample_format: SampleFormat,
    fallback_attempts: Vec<String>,
    _stream: Stream,
}

struct MonitorResamplerState {
    input_sample_rate_hz: u32,
    resampler: BandlimitedResampler,
    controller: MonitorClockController,
}

#[derive(Default)]
struct MonitorClockController {
    integral_ppm: f64,
}

impl MonitorClockController {
    fn update(&mut self, buffered: usize, target: usize, elapsed_s: f64, primed: bool) -> i32 {
        if !primed || target == 0 {
            self.integral_ppm = 0.0;
            return 0;
        }
        let error = (buffered as f64 - target as f64) / target as f64;
        self.integral_ppm = (self.integral_ppm - error * 80.0 * elapsed_s).clamp(-1_000.0, 1_000.0);
        (-error * 1_500.0 + self.integral_ppm).round().clamp(
            f64::from(-MONITOR_MAX_ADJUSTMENT_PPM),
            f64::from(MONITOR_MAX_ADJUSTMENT_PPM),
        ) as i32
    }
}

impl AudioMonitor {
    pub fn open(sample_rate_hz: u32, preferred_device_name: Option<&str>) -> Result<Self> {
        let host = cpal::default_host();
        let device = select_device(&host, AudioDeviceKind::Output, preferred_device_name)?;
        let (samples_tx, samples_rx) = mpsc::sync_channel::<Vec<f32>>(8);
        let samples_rx = Arc::new(std::sync::Mutex::new(samples_rx));
        let (errors_tx, errors_rx) = mpsc::channel::<String>();
        let dropped_chunks = Arc::new(AtomicU64::new(0));
        let underruns = Arc::new(AtomicU64::new(0));
        let buffered_samples = Arc::new(AtomicUsize::new(0));
        let primed = Arc::new(AtomicBool::new(false));
        let clock_adjustment_ppm = Arc::new(AtomicI32::new(0));
        let pending = Arc::new(std::sync::Mutex::new(VecDeque::<f32>::new()));
        let candidates = config_candidates(&device, AudioDeviceKind::Output, sample_rate_hz, 1)?;
        let mut failures = Vec::new();
        for supported in candidates {
            let label = config_label(&supported);
            let config = supported.config();
            let output_sample_rate_hz = config.sample_rate.0;
            let channels = config.channels as usize;
            let target_samples =
                (output_sample_rate_hz as usize * MONITOR_TARGET_LATENCY_MS as usize / 1_000)
                    .max(1);

            macro_rules! output_stream {
                ($sample:ty) => {{
                    let pending = pending.clone();
                    let samples_rx = samples_rx.clone();
                    let errors_tx = errors_tx.clone();
                    let buffered_samples = buffered_samples.clone();
                    let primed = primed.clone();
                    let underruns = underruns.clone();
                    device.build_output_stream(
                        &config,
                        move |data: &mut [$sample], _| {
                            if let Ok(mut queue) = pending.lock() {
                                if let Ok(receiver) = samples_rx.lock() {
                                    while let Ok(chunk) = receiver.try_recv() {
                                        buffered_samples.fetch_add(chunk.len(), Ordering::Relaxed);
                                        queue.extend(chunk);
                                    }
                                }
                                if !primed.load(Ordering::Relaxed)
                                    && buffered_samples.load(Ordering::Relaxed) >= target_samples
                                {
                                    primed.store(true, Ordering::Release);
                                }
                                for frame in data.chunks_mut(channels) {
                                    let active = primed.load(Ordering::Acquire);
                                    let value = active.then(|| queue.pop_front()).flatten();
                                    if value.is_some() {
                                        buffered_samples.fetch_sub(1, Ordering::Relaxed);
                                    } else if active {
                                        primed.store(false, Ordering::Release);
                                        underruns.fetch_add(1, Ordering::Relaxed);
                                    }
                                    let value = value.unwrap_or_default();
                                    frame.fill(<$sample>::from_sample(value));
                                }
                            }
                        },
                        move |error: StreamError| {
                            let _ = errors_tx.send(error.to_string());
                        },
                        None,
                    )
                }};
            }
            let built = match supported.sample_format() {
                SampleFormat::I8 => output_stream!(i8),
                SampleFormat::I16 => output_stream!(i16),
                SampleFormat::I32 => output_stream!(i32),
                SampleFormat::I64 => output_stream!(i64),
                SampleFormat::U8 => output_stream!(u8),
                SampleFormat::U16 => output_stream!(u16),
                SampleFormat::U32 => output_stream!(u32),
                SampleFormat::U64 => output_stream!(u64),
                SampleFormat::F32 => output_stream!(f32),
                SampleFormat::F64 => output_stream!(f64),
                format => {
                    failures.push(format!("{label}: unsupported format {format:?}"));
                    continue;
                }
            };
            let stream = match built {
                Ok(stream) => stream,
                Err(error) => {
                    failures.push(format!("{label}: open failed: {error}"));
                    continue;
                }
            };
            if let Err(error) = stream.play() {
                failures.push(format!("{label}: start failed: {error}"));
                continue;
            }
            return Ok(Self {
                samples_tx,
                errors_rx,
                dropped_chunks,
                underruns,
                buffered_samples,
                primed,
                clock_adjustment_ppm,
                resampler: std::sync::Mutex::new(MonitorResamplerState {
                    input_sample_rate_hz: sample_rate_hz,
                    resampler: BandlimitedResampler::new(sample_rate_hz, output_sample_rate_hz),
                    controller: MonitorClockController::default(),
                }),
                volume: Arc::new(std::sync::atomic::AtomicU32::new(1.0_f32.to_bits())),
                input_sample_rate_hz: sample_rate_hz,
                output_sample_rate_hz,
                output_channels: config.channels,
                output_sample_format: supported.sample_format(),
                fallback_attempts: failures,
                _stream: stream,
            });
        }
        bail!(
            "unable to open any output audio configuration: {}",
            failures.join("; ")
        )
    }

    pub fn set_volume(&self, volume: f32) {
        self.volume
            .store(volume.clamp(0.0, 2.0).to_bits(), Ordering::Relaxed);
    }

    pub fn push(&self, samples: &[i16]) {
        self.push_at_sample_rate(samples, self.input_sample_rate_hz);
    }

    pub fn push_at_sample_rate(&self, samples: &[i16], sample_rate_hz: u32) {
        let samples = samples
            .iter()
            .map(|sample| *sample as f32 / i16::MAX as f32)
            .collect::<Vec<_>>();
        self.push_f32_at_sample_rate(&samples, sample_rate_hz);
    }

    pub fn push_f32(&self, samples: &[f32]) {
        self.push_f32_at_sample_rate(samples, self.input_sample_rate_hz);
    }

    pub fn push_f32_at_sample_rate(&self, samples: &[f32], sample_rate_hz: u32) {
        let volume = f32::from_bits(self.volume.load(Ordering::Relaxed));
        let samples = samples
            .iter()
            .map(|sample| (*sample * volume).clamp(-1.0, 1.0))
            .collect::<Vec<_>>();
        let samples = self.resampler.lock().map_or_else(
            |_| Vec::new(),
            |mut state| {
                if state.input_sample_rate_hz != sample_rate_hz {
                    *state = MonitorResamplerState {
                        input_sample_rate_hz: sample_rate_hz,
                        resampler: BandlimitedResampler::new(
                            sample_rate_hz,
                            self.output_sample_rate_hz,
                        ),
                        controller: MonitorClockController::default(),
                    };
                }
                let target = (self.output_sample_rate_hz as usize
                    * MONITOR_TARGET_LATENCY_MS as usize
                    / 1_000)
                    .max(1);
                let adjustment = state.controller.update(
                    self.buffered_samples.load(Ordering::Relaxed),
                    target,
                    samples.len() as f64 / f64::from(sample_rate_hz),
                    self.primed.load(Ordering::Acquire),
                );
                self.clock_adjustment_ppm
                    .store(adjustment, Ordering::Relaxed);
                state.resampler.set_rate_adjustment_ppm(adjustment);
                state.resampler.process(&samples)
            },
        );
        if !samples.is_empty() && self.samples_tx.try_send(samples).is_err() {
            self.dropped_chunks.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn take_error(&self) -> Option<String> {
        self.errors_rx.try_recv().ok()
    }

    pub fn take_dropped_chunks(&self) -> u64 {
        self.dropped_chunks.swap(0, Ordering::Relaxed)
    }

    pub fn take_underruns(&self) -> u64 {
        self.underruns.swap(0, Ordering::Relaxed)
    }

    pub fn clock_adjustment_ppm(&self) -> i32 {
        self.clock_adjustment_ppm.load(Ordering::Relaxed)
    }

    pub fn buffered_latency_ms(&self) -> u32 {
        ((self.buffered_samples.load(Ordering::Relaxed) as u64 * 1_000)
            / u64::from(self.output_sample_rate_hz)) as u32
    }

    pub fn output_sample_rate_hz(&self) -> u32 {
        self.output_sample_rate_hz
    }

    pub fn output_channels(&self) -> u16 {
        self.output_channels
    }

    pub fn output_sample_format(&self) -> SampleFormat {
        self.output_sample_format
    }

    pub fn fallback_attempts(&self) -> &[String] {
        &self.fallback_attempts
    }
}

impl Default for AudioService {
    fn default() -> Self {
        Self::new(None, true)
    }
}

impl AudioService {
    pub fn new(preferred_device_name: Option<String>, enabled: bool) -> Self {
        Self {
            preferred_device_name,
            enabled,
        }
    }

    pub fn input_devices() -> Result<Vec<String>> {
        device_names(AudioDeviceKind::Input)
    }

    pub fn output_devices() -> Result<Vec<String>> {
        device_names(AudioDeviceKind::Output)
    }

    pub fn open_stream(&self, sample_rate_hz: u32, channels: u16) -> Result<AudioStream> {
        if !self.enabled {
            bail!("audio capture is disabled in settings");
        }
        if channels != 1 {
            bail!("audio capture exposes one downmixed channel; requested {channels} channels");
        }

        let host = cpal::default_host();
        let device = select_device(
            &host,
            AudioDeviceKind::Input,
            self.preferred_device_name.as_deref(),
        )?;
        let candidates =
            config_candidates(&device, AudioDeviceKind::Input, sample_rate_hz, channels)?;
        let mut failures = Vec::new();
        for supported in candidates {
            let label = config_label(&supported);
            let stream_config = supported.config();
            let device_sample_rate_hz = stream_config.sample_rate.0;
            let device_channels = stream_config.channels as usize;
            let (samples_tx, samples_rx) = mpsc::sync_channel::<Vec<f32>>(32);
            let (errors_tx, errors_rx) = mpsc::channel::<String>();

            macro_rules! input_stream {
                ($sample:ty) => {{
                    let tx = samples_tx.clone();
                    let errors_tx = errors_tx.clone();
                    let mut resampler =
                        BandlimitedResampler::new(device_sample_rate_hz, sample_rate_hz);
                    device.build_input_stream(
                        &stream_config,
                        move |data: &[$sample], _| {
                            let converted = data
                                .iter()
                                .copied()
                                .map(f32::from_sample)
                                .collect::<Vec<_>>();
                            let downmixed = downmix_f32(&converted, device_channels);
                            let resampled = resampler.process(&downmixed);
                            if !resampled.is_empty() {
                                let _ = tx.try_send(resampled);
                            }
                        },
                        move |error: StreamError| {
                            let _ = errors_tx.send(error.to_string());
                        },
                        None,
                    )
                }};
            }

            let built = match supported.sample_format() {
                SampleFormat::I8 => input_stream!(i8),
                SampleFormat::I16 => input_stream!(i16),
                SampleFormat::I32 => input_stream!(i32),
                SampleFormat::I64 => input_stream!(i64),
                SampleFormat::U8 => input_stream!(u8),
                SampleFormat::U16 => input_stream!(u16),
                SampleFormat::U32 => input_stream!(u32),
                SampleFormat::U64 => input_stream!(u64),
                SampleFormat::F32 => input_stream!(f32),
                SampleFormat::F64 => input_stream!(f64),
                format => {
                    failures.push(format!("{label}: unsupported format {format:?}"));
                    continue;
                }
            };
            let stream = match built {
                Ok(stream) => stream,
                Err(error) => {
                    failures.push(format!("{label}: open failed: {error}"));
                    continue;
                }
            };
            if let Err(error) = stream.play() {
                failures.push(format!("{label}: start failed: {error}"));
                continue;
            }
            return Ok(AudioStream {
                _stream: stream,
                samples_rx,
                errors_rx,
                pending: VecDeque::new(),
                requested_channels: 1,
                device_sample_rate_hz,
                output_sample_rate_hz: sample_rate_hz,
                device_channels: stream_config.channels,
                device_sample_format: supported.sample_format(),
                fallback_attempts: failures,
            });
        }
        bail!(
            "unable to open any input audio configuration: {}",
            failures.join("; ")
        )
    }
}

impl AudioStream {
    pub fn device_sample_rate_hz(&self) -> u32 {
        self.device_sample_rate_hz
    }

    pub fn output_sample_rate_hz(&self) -> u32 {
        self.output_sample_rate_hz
    }

    pub fn device_channels(&self) -> u16 {
        self.device_channels
    }

    pub fn device_sample_format(&self) -> SampleFormat {
        self.device_sample_format
    }

    pub fn fallback_attempts(&self) -> &[String] {
        &self.fallback_attempts
    }

    pub fn read_chunk(&mut self, bytes_to_read: usize) -> Result<Vec<i16>> {
        let frame_bytes = 2 * self.requested_channels;
        if !bytes_to_read.is_multiple_of(frame_bytes) {
            bail!("PCM byte length is not an even multiple of the requested frame size");
        }
        let requested_samples = bytes_to_read / frame_bytes;
        while self.pending.len() < requested_samples {
            match self.errors_rx.try_recv() {
                Ok(error) => bail!("audio input stream failed: {error}"),
                Err(TryRecvError::Disconnected) | Err(TryRecvError::Empty) => {}
            }
            let block = self
                .samples_rx
                .recv_timeout(Duration::from_secs(2))
                .context("timed out waiting for audio input")?;
            self.pending.extend(block);
        }
        Ok(self
            .pending
            .drain(..requested_samples)
            .map(f32_to_i16)
            .collect())
    }

    /// Read a PCM chunk while remaining responsive to application shutdown.
    ///
    /// The ordinary read keeps its two-second hardware timeout. GUI workers
    /// use this variant so multiple profile streams do not each add that full
    /// timeout while being joined during exit.
    pub fn read_chunk_until_stopped(
        &mut self,
        bytes_to_read: usize,
        stop: &AtomicBool,
    ) -> Result<Option<Vec<i16>>> {
        let frame_bytes = 2 * self.requested_channels;
        if !bytes_to_read.is_multiple_of(frame_bytes) {
            bail!("PCM byte length is not an even multiple of the requested frame size");
        }
        let requested_samples = bytes_to_read / frame_bytes;
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.pending.len() < requested_samples {
            if stop.load(Ordering::Relaxed) {
                return Ok(None);
            }
            match self.errors_rx.try_recv() {
                Ok(error) => bail!("audio input stream failed: {error}"),
                Err(TryRecvError::Disconnected) | Err(TryRecvError::Empty) => {}
            }
            match self.samples_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(block) => self.pending.extend(block),
                Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    bail!("timed out waiting for audio input")
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    bail!("audio input stream disconnected")
                }
            }
        }
        Ok(Some(
            self.pending
                .drain(..requested_samples)
                .map(f32_to_i16)
                .collect(),
        ))
    }

    pub fn read_frames_f32_until_stopped(
        &mut self,
        requested_samples: usize,
        stop: &AtomicBool,
    ) -> Result<Option<Vec<f32>>> {
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.pending.len() < requested_samples {
            if stop.load(Ordering::Relaxed) {
                return Ok(None);
            }
            match self.errors_rx.try_recv() {
                Ok(error) => bail!("audio input stream failed: {error}"),
                Err(TryRecvError::Disconnected) | Err(TryRecvError::Empty) => {}
            }
            match self.samples_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(block) => self.pending.extend(block),
                Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    bail!("timed out waiting for audio input")
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    bail!("audio input stream disconnected")
                }
            }
        }
        Ok(Some(self.pending.drain(..requested_samples).collect()))
    }
}

pub fn play_pcm_blocking(
    pcm: &[i16],
    sample_rate_hz: u32,
    preferred_device_name: Option<&str>,
    abort: Arc<AtomicBool>,
) -> Result<()> {
    let host = cpal::default_host();
    let device = select_device(&host, AudioDeviceKind::Output, preferred_device_name)?;
    let source_pcm = pcm
        .iter()
        .map(|sample| *sample as f32 / i16::MAX as f32)
        .collect::<Vec<_>>();
    let candidates = config_candidates(&device, AudioDeviceKind::Output, sample_rate_hz, 1)?;
    let mut failures = Vec::new();
    let mut active = None;
    for supported in candidates {
        let label = config_label(&supported);
        let config = supported.config();
        let channels = config.channels as usize;
        let pcm = Arc::new(resample_f32(
            &source_pcm,
            sample_rate_hz,
            config.sample_rate.0,
        ));
        let cursor = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (done_tx, done_rx) = mpsc::sync_channel::<()>(1);
        let (error_tx, error_rx) = mpsc::channel::<String>();

        macro_rules! output_stream {
            ($sample:ty) => {{
                let pcm = pcm.clone();
                let cursor = cursor.clone();
                let done_tx = done_tx.clone();
                let error_tx = error_tx.clone();
                device.build_output_stream(
                    &config,
                    move |data: &mut [$sample], _| {
                        let start = cursor.load(Ordering::Relaxed);
                        let mut source_index = start;
                        for frame in data.chunks_mut(channels) {
                            let value = pcm.get(source_index).copied();
                            let rendered = value
                                .map(<$sample>::from_sample)
                                .unwrap_or(<$sample as Sample>::EQUILIBRIUM);
                            frame.fill(rendered);
                            if value.is_some() {
                                source_index += 1;
                            }
                        }
                        cursor.store(source_index, Ordering::Release);
                        if source_index >= pcm.len() {
                            let _ = done_tx.try_send(());
                        }
                    },
                    move |error: StreamError| {
                        let _ = error_tx.send(error.to_string());
                    },
                    None,
                )
            }};
        }

        let built = match supported.sample_format() {
            SampleFormat::I8 => output_stream!(i8),
            SampleFormat::I16 => output_stream!(i16),
            SampleFormat::I32 => output_stream!(i32),
            SampleFormat::I64 => output_stream!(i64),
            SampleFormat::U8 => output_stream!(u8),
            SampleFormat::U16 => output_stream!(u16),
            SampleFormat::U32 => output_stream!(u32),
            SampleFormat::U64 => output_stream!(u64),
            SampleFormat::F32 => output_stream!(f32),
            SampleFormat::F64 => output_stream!(f64),
            format => {
                failures.push(format!("{label}: unsupported format {format:?}"));
                continue;
            }
        };
        let stream = match built {
            Ok(stream) => stream,
            Err(error) => {
                failures.push(format!("{label}: open failed: {error}"));
                continue;
            }
        };
        if let Err(error) = stream.play() {
            failures.push(format!("{label}: start failed: {error}"));
            continue;
        }
        active = Some((stream, done_rx, error_rx));
        break;
    }
    let Some((_stream, done_rx, error_rx)) = active else {
        bail!(
            "unable to open any output audio configuration: {}",
            failures.join("; ")
        );
    };

    loop {
        if abort.load(Ordering::Acquire) {
            bail!("audio output canceled");
        }
        if let Ok(error) = error_rx.try_recv() {
            bail!("audio output stream failed: {error}");
        }
        match done_rx.recv_timeout(Duration::from_millis(20)) {
            Ok(()) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                bail!("audio output stream stopped unexpectedly")
            }
        }
    }
    // The completion signal means the last buffer was handed to the backend;
    // leave the stream alive briefly so that buffer can reach the device.
    std::thread::sleep(Duration::from_millis(100));
    Ok(())
}

fn device_names(kind: AudioDeviceKind) -> Result<Vec<String>> {
    let host = cpal::default_host();
    let devices = match kind {
        AudioDeviceKind::Input => host.input_devices()?.collect::<Vec<_>>(),
        AudioDeviceKind::Output => host.output_devices()?.collect::<Vec<_>>(),
    };
    let mut names = devices
        .into_iter()
        .filter_map(|device| device.name().ok())
        .collect::<Vec<_>>();
    names.sort_by_key(|name| name.to_ascii_lowercase());
    names.dedup();
    Ok(names)
}

fn select_device(
    host: &cpal::Host,
    kind: AudioDeviceKind,
    preferred_name: Option<&str>,
) -> Result<Device> {
    if let Some(preferred) = preferred_name.filter(|name| !name.trim().is_empty()) {
        let devices = match kind {
            AudioDeviceKind::Input => host.input_devices()?.collect::<Vec<_>>(),
            AudioDeviceKind::Output => host.output_devices()?.collect::<Vec<_>>(),
        };
        if let Some(device) = devices
            .into_iter()
            .find(|device| device.name().is_ok_and(|name| name == preferred))
        {
            return Ok(device);
        }
        bail!(
            "configured {} audio device is unavailable: {preferred}",
            kind_label(kind)
        );
    }

    // WSLg publishes its microphone and speaker through PulseAudio. CPAL 0.15
    // uses ALSA on Linux, so prefer the ALSA Pulse bridge when it is present
    // instead of ALSA's often-unusable `default` device inside WSL.
    if wslg_pulse_available() {
        let devices = match kind {
            AudioDeviceKind::Input => host.input_devices()?.collect::<Vec<_>>(),
            AudioDeviceKind::Output => host.output_devices()?.collect::<Vec<_>>(),
        };
        if let Some(device) = devices.into_iter().find(|device| {
            device
                .name()
                .is_ok_and(|name| name.eq_ignore_ascii_case("pulse"))
        }) {
            return Ok(device);
        }
        bail!(
            "WSLg audio is available, but the ALSA PulseAudio bridge is missing; install the libasound2-plugins package"
        );
    }

    match kind {
        AudioDeviceKind::Input => host.default_input_device(),
        AudioDeviceKind::Output => host.default_output_device(),
    }
    .ok_or_else(|| anyhow!("no default {} audio device is available", kind_label(kind)))
}

#[cfg(target_os = "linux")]
fn wslg_pulse_available() -> bool {
    std::env::var_os("PULSE_SERVER").is_some()
        && std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .is_ok_and(|release| release.to_ascii_lowercase().contains("microsoft"))
}

#[cfg(not(target_os = "linux"))]
fn wslg_pulse_available() -> bool {
    false
}

fn config_candidates(
    device: &Device,
    kind: AudioDeviceKind,
    sample_rate_hz: u32,
    requested_channels: u16,
) -> Result<Vec<SupportedStreamConfig>> {
    let ranges = match kind {
        AudioDeviceKind::Input => device.supported_input_configs()?.collect::<Vec<_>>(),
        AudioDeviceKind::Output => device.supported_output_configs()?.collect::<Vec<_>>(),
    };
    const COMMON_RATES: &[u32] = &[
        48_000, 44_100, 96_000, 88_200, 32_000, 24_000, 22_050, 16_000, 12_000, 8_000,
    ];
    let mut candidates = Vec::new();
    for range in &ranges {
        let mut rates = vec![
            sample_rate_hz.clamp(range.min_sample_rate().0, range.max_sample_rate().0),
            range.min_sample_rate().0,
            range.max_sample_rate().0,
        ];
        rates.extend(COMMON_RATES.iter().copied().filter(|rate| {
            range.min_sample_rate().0 <= *rate && *rate <= range.max_sample_rate().0
        }));
        rates.sort_unstable();
        rates.dedup();
        candidates.extend(
            rates
                .into_iter()
                .map(|rate| (*range).with_sample_rate(SampleRate(rate))),
        );
    }
    if let Ok(default) = match kind {
        AudioDeviceKind::Input => device.default_input_config(),
        AudioDeviceKind::Output => device.default_output_config(),
    } {
        candidates.push(default);
    }
    candidates.sort_by_key(|config| {
        (
            u8::from(config.sample_rate().0 < 24_000),
            config.sample_rate().0.abs_diff(sample_rate_hz),
            config.channels().abs_diff(requested_channels),
            sample_format_preference(config.sample_format()),
        )
    });
    candidates.dedup_by(|left, right| {
        left.sample_rate() == right.sample_rate()
            && left.channels() == right.channels()
            && left.sample_format() == right.sample_format()
    });
    Ok(candidates)
}

fn config_label(config: &SupportedStreamConfig) -> String {
    format!(
        "{} Hz, {} channel(s), {:?}",
        config.sample_rate().0,
        config.channels(),
        config.sample_format()
    )
}

fn sample_format_preference(format: SampleFormat) -> u8 {
    match format {
        SampleFormat::F32 => 0,
        SampleFormat::I16 => 1,
        SampleFormat::I32 => 2,
        SampleFormat::F64 => 3,
        SampleFormat::U16 => 4,
        SampleFormat::U32 => 5,
        SampleFormat::I64 => 6,
        SampleFormat::U64 => 7,
        SampleFormat::I8 => 8,
        SampleFormat::U8 => 9,
        _ => 10,
    }
}

fn kind_label(kind: AudioDeviceKind) -> &'static str {
    match kind {
        AudioDeviceKind::Input => "input",
        AudioDeviceKind::Output => "output",
    }
}

fn downmix_f32(data: &[f32], channels: usize) -> Vec<f32> {
    if channels == 0 {
        return Vec::new();
    }
    data.chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

fn f32_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmixes_stereo_without_clipping() {
        assert_eq!(downmix_f32(&[1.0, 1.0, -1.0, -1.0], 2), vec![1.0, -1.0]);
    }

    #[test]
    fn prefers_decoder_quality_over_eight_bit_audio() {
        assert!(
            sample_format_preference(SampleFormat::F32)
                < sample_format_preference(SampleFormat::I8)
        );
        assert!(
            sample_format_preference(SampleFormat::I16)
                < sample_format_preference(SampleFormat::U8)
        );
    }

    #[test]
    fn rejects_non_mono_capture_contract_before_opening_a_device() {
        let service = AudioService::new(None, true);
        let error = service.open_stream(48_000, 2).err().unwrap();
        assert!(error.to_string().contains("one downmixed channel"));
    }

    #[test]
    fn monitor_clock_controller_corrects_long_running_device_drift() {
        let target = 5_760.0_f64;
        let mut buffered = target;
        let mut controller = MonitorClockController::default();
        let source_drift_ppm = 200.0_f64;
        let mut adjustment = 0_i32;
        for _ in 0..360_000 {
            adjustment = controller.update(buffered.max(0.0) as usize, target as usize, 0.1, true);
            let produced = 4_800.0
                * (1.0 + source_drift_ppm / 1_000_000.0)
                * (1.0 + f64::from(adjustment) / 1_000_000.0);
            buffered += produced - 4_800.0;
        }
        assert!((buffered - target).abs() < target * 0.05);
        assert!((-260..=-140).contains(&adjustment));
    }

    #[test]
    fn monitor_clock_controller_waits_until_output_is_primed() {
        let mut controller = MonitorClockController::default();
        assert_eq!(controller.update(0, 5_760, 0.1, false), 0);
    }
}
