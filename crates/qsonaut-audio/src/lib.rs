use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
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

pub mod resample;

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
    samples_rx: Receiver<Vec<i16>>,
    errors_rx: Receiver<String>,
    pending: VecDeque<i16>,
    requested_channels: usize,
    device_sample_rate_hz: u32,
    output_sample_rate_hz: u32,
}

pub struct AudioMonitor {
    samples_tx: SyncSender<Vec<i16>>,
    errors_rx: Receiver<String>,
    dropped_chunks: Arc<std::sync::atomic::AtomicU64>,
    resampler: std::sync::Mutex<MonitorResampler>,
    volume: Arc<std::sync::atomic::AtomicU32>,
    input_sample_rate_hz: u32,
    _stream: Stream,
}

struct MonitorResampler {
    input_sample_rate_hz: u32,
    output_sample_rate_hz: u32,
    source_position: f64,
    source: VecDeque<i16>,
}

type CaptureResampler = MonitorResampler;

impl MonitorResampler {
    fn new(input_sample_rate_hz: u32, output_sample_rate_hz: u32) -> Self {
        Self {
            input_sample_rate_hz,
            output_sample_rate_hz,
            source_position: 0.0,
            source: VecDeque::new(),
        }
    }

    fn process(&mut self, samples: &[i16]) -> Vec<i16> {
        if samples.is_empty() || self.input_sample_rate_hz == self.output_sample_rate_hz {
            return samples.to_vec();
        }
        let step = self.input_sample_rate_hz as f64 / self.output_sample_rate_hz as f64;
        self.source.extend(samples.iter().copied());
        let mut output = Vec::new();
        while self.source_position + 1.0 < self.source.len() as f64 {
            let left = self.source_position.floor() as usize;
            let right = left + 1;
            let fraction = (self.source_position - left as f64) as f32;
            output.push(
                (self.source[left] as f32 * (1.0 - fraction) + self.source[right] as f32 * fraction)
                    .round()
                    .clamp(i16::MIN as f32, i16::MAX as f32) as i16,
            );
            self.source_position += step;
        }
        let consumed = self.source_position.floor() as usize;
        if consumed > 0 {
            self.source
                .drain(..consumed.min(self.source.len().saturating_sub(1)));
            self.source_position -= consumed as f64;
        }
        output
    }

    fn set_input_sample_rate(&mut self, input_sample_rate_hz: u32) {
        if self.input_sample_rate_hz != input_sample_rate_hz {
            self.input_sample_rate_hz = input_sample_rate_hz;
            self.source_position = 0.0;
            self.source.clear();
        }
    }
}

impl AudioMonitor {
    pub fn open(sample_rate_hz: u32, preferred_device_name: Option<&str>) -> Result<Self> {
        let host = cpal::default_host();
        let device = select_device(&host, AudioDeviceKind::Output, preferred_device_name)?;
        let supported = select_config(&device, AudioDeviceKind::Output, sample_rate_hz, 1)
            .or_else(|_| device.default_output_config().map_err(anyhow::Error::from))?;
        let config = supported.config();
        let output_sample_rate_hz = config.sample_rate.0;
        let channels = config.channels as usize;
        let (samples_tx, samples_rx) = mpsc::sync_channel::<Vec<i16>>(8);
        let (errors_tx, errors_rx) = mpsc::channel::<String>();
        let dropped_chunks = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let pending = Arc::new(std::sync::Mutex::new(VecDeque::<i16>::new()));

        macro_rules! output_stream {
            ($sample:ty) => {{
                let pending = pending.clone();
                let errors_tx = errors_tx.clone();
                device.build_output_stream(
                    &config,
                    move |data: &mut [$sample], _| {
                        if let Ok(mut queue) = pending.lock() {
                            while let Ok(chunk) = samples_rx.try_recv() {
                                queue.extend(chunk);
                            }
                            for frame in data.chunks_mut(channels) {
                                let value = queue.pop_front().unwrap_or_default();
                                frame.fill(<$sample>::from_sample(value));
                            }
                        }
                    },
                    move |error: StreamError| {
                        let _ = errors_tx.send(error.to_string());
                    },
                    None,
                )?
            }};
        }
        let stream = match supported.sample_format() {
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
            format => bail!("unsupported output sample format: {format:?}"),
        };
        stream.play().context("failed to start audio monitor")?;
        Ok(Self {
            samples_tx,
            errors_rx,
            dropped_chunks,
            resampler: std::sync::Mutex::new(MonitorResampler::new(
                sample_rate_hz,
                output_sample_rate_hz,
            )),
            volume: Arc::new(std::sync::atomic::AtomicU32::new(1.0_f32.to_bits())),
            input_sample_rate_hz: sample_rate_hz,
            _stream: stream,
        })
    }

    pub fn set_volume(&self, volume: f32) {
        self.volume
            .store(volume.clamp(0.0, 2.0).to_bits(), Ordering::Relaxed);
    }

    pub fn push(&self, samples: &[i16]) {
        self.push_at_sample_rate(samples, self.input_sample_rate_hz);
    }

    pub fn push_at_sample_rate(&self, samples: &[i16], sample_rate_hz: u32) {
        let volume = f32::from_bits(self.volume.load(Ordering::Relaxed));
        let samples = samples
            .iter()
            .map(|sample| (*sample as f32 * volume).clamp(i16::MIN as f32, i16::MAX as f32) as i16)
            .collect::<Vec<_>>();
        let samples = self.resampler.lock().map_or_else(
            |_| Vec::new(),
            |mut resampler| {
                resampler.set_input_sample_rate(sample_rate_hz);
                resampler.process(&samples)
            },
        );
        if self.samples_tx.try_send(samples).is_err() {
            self.dropped_chunks.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn take_error(&self) -> Option<String> {
        self.errors_rx.try_recv().ok()
    }

    pub fn take_dropped_chunks(&self) -> u64 {
        self.dropped_chunks.swap(0, Ordering::Relaxed)
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
        let supported = select_config(&device, AudioDeviceKind::Input, sample_rate_hz, channels)?;
        let stream_config = supported.config();
        let device_sample_rate_hz = stream_config.sample_rate.0;
        let device_channels = stream_config.channels as usize;
        let (samples_tx, samples_rx) = mpsc::sync_channel::<Vec<i16>>(32);
        let (errors_tx, errors_rx) = mpsc::channel::<String>();
        let error_callback = move |error: StreamError| {
            let _ = errors_tx.send(error.to_string());
        };

        macro_rules! input_stream {
            ($sample:ty) => {{
                let tx = samples_tx.clone();
                let mut resampler = CaptureResampler::new(device_sample_rate_hz, sample_rate_hz);
                device.build_input_stream(
                    &stream_config,
                    move |data: &[$sample], _| {
                        let converted = data
                            .iter()
                            .copied()
                            .map(i16::from_sample)
                            .collect::<Vec<_>>();
                        let downmixed = downmix_i16(&converted, device_channels);
                        let resampled = resampler.process(&downmixed);
                        if !resampled.is_empty() {
                            let _ = tx.try_send(resampled);
                        }
                    },
                    error_callback,
                    None,
                )?
            }};
        }

        let stream = match supported.sample_format() {
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
            format => bail!("unsupported input sample format: {format:?}"),
        };
        stream
            .play()
            .context("failed to start audio input stream")?;

        Ok(AudioStream {
            _stream: stream,
            samples_rx,
            errors_rx,
            pending: VecDeque::new(),
            requested_channels: 1,
            device_sample_rate_hz,
            output_sample_rate_hz: sample_rate_hz,
        })
    }
}

impl AudioStream {
    pub fn device_sample_rate_hz(&self) -> u32 {
        self.device_sample_rate_hz
    }

    pub fn output_sample_rate_hz(&self) -> u32 {
        self.output_sample_rate_hz
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
        Ok(self.pending.drain(..requested_samples).collect())
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
    let supported = select_config(&device, AudioDeviceKind::Output, sample_rate_hz, 1)
        .or_else(|_| device.default_output_config().map_err(anyhow::Error::from))?;
    let config = supported.config();
    let channels = config.channels as usize;
    let pcm = Arc::new(resample_linear_i16(
        pcm,
        sample_rate_hz,
        config.sample_rate.0,
    ));
    let cursor = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (done_tx, done_rx) = mpsc::sync_channel::<()>(1);
    let (error_tx, error_rx) = mpsc::channel::<String>();
    let error_callback = move |error: StreamError| {
        let _ = error_tx.send(error.to_string());
    };

    macro_rules! output_stream {
        ($sample:ty) => {{
            let pcm = pcm.clone();
            let cursor = cursor.clone();
            let done_tx = done_tx.clone();
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
                error_callback,
                None,
            )?
        }};
    }

    let stream = match supported.sample_format() {
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
        format => bail!("unsupported output sample format: {format:?}"),
    };
    stream
        .play()
        .context("failed to start audio output stream")?;

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

fn select_config(
    device: &Device,
    kind: AudioDeviceKind,
    sample_rate_hz: u32,
    requested_channels: u16,
) -> Result<SupportedStreamConfig> {
    let ranges = match kind {
        AudioDeviceKind::Input => device.supported_input_configs()?.collect::<Vec<_>>(),
        AudioDeviceKind::Output => device.supported_output_configs()?.collect::<Vec<_>>(),
    };
    ranges
        .iter()
        .min_by_key(|range| {
            let selected_rate =
                sample_rate_hz.clamp(range.min_sample_rate().0, range.max_sample_rate().0);
            (
                selected_rate.abs_diff(sample_rate_hz),
                range.channels().abs_diff(requested_channels),
                sample_format_preference(range.sample_format()),
            )
        })
        .map(|range| {
            let selected_rate = SampleRate(
                sample_rate_hz.clamp(range.min_sample_rate().0, range.max_sample_rate().0),
            );
            (*range).with_sample_rate(selected_rate)
        })
        .with_context(|| {
            format!(
                "{} audio device does not expose a usable stream configuration near {} Hz",
                kind_label(kind),
                sample_rate_hz
            )
        })
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

fn downmix_i16(data: &[i16], channels: usize) -> Vec<i16> {
    if channels == 0 {
        return Vec::new();
    }
    data.chunks_exact(channels)
        .map(|frame| {
            let sum = frame.iter().map(|sample| i32::from(*sample)).sum::<i32>();
            (sum / channels as i32) as i16
        })
        .collect()
}

fn resample_linear_i16(samples: &[i16], source_rate: u32, output_rate: u32) -> Vec<i16> {
    if samples.is_empty() || source_rate == output_rate || source_rate == 0 || output_rate == 0 {
        return samples.to_vec();
    }
    let output_len = ((samples.len() as u64 * output_rate as u64) / source_rate as u64) as usize;
    let rate_ratio = source_rate as f64 / output_rate as f64;
    (0..output_len)
        .map(|index| {
            let source_position = index as f64 * rate_ratio;
            let left = source_position.floor() as usize;
            let right = (left + 1).min(samples.len() - 1);
            let fraction = (source_position - left as f64) as f32;
            let value = samples[left] as f32 * (1.0 - fraction) + samples[right] as f32 * fraction;
            value.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_resampler_preserves_chunk_boundary_continuity() {
        let mut chunked = MonitorResampler::new(48_000, 44_100);
        let first = chunked.process(&(0..480).map(|value| value * 10).collect::<Vec<_>>());
        let second = chunked.process(&(480..960).map(|value| value * 10).collect::<Vec<_>>());
        let mut combined = first;
        combined.extend(second);

        let expected = resample_linear_i16(
            &(0..960).map(|value| value * 10).collect::<Vec<_>>(),
            48_000,
            44_100,
        );
        assert!((combined.len() as isize - expected.len() as isize).abs() <= 1);
        assert_eq!(&combined[..100], &expected[..100]);
    }

    #[test]
    fn downmixes_stereo_without_clipping() {
        assert_eq!(
            downmix_i16(&[i16::MAX, i16::MAX, i16::MIN, i16::MIN], 2),
            vec![i16::MAX, i16::MIN]
        );
    }

    #[test]
    fn capture_resampler_preserves_chunk_boundary_continuity() {
        let mut chunked = CaptureResampler::new(44_100, 48_000);
        let first = chunked.process(&(0..441).map(|value| value * 10).collect::<Vec<_>>());
        let second = chunked.process(&(441..882).map(|value| value * 10).collect::<Vec<_>>());
        let mut combined = first;
        combined.extend(second);

        let expected = resample_linear_i16(
            &(0..882).map(|value| value * 10).collect::<Vec<_>>(),
            44_100,
            48_000,
        );
        assert!((combined.len() as isize - expected.len() as isize).abs() <= 1);
        assert_eq!(&combined[..100], &expected[..100]);
    }

    #[test]
    fn resamples_output_to_the_device_rate() {
        assert_eq!(resample_linear_i16(&[0, 1000, 2000], 3, 6).len(), 6);
        assert_eq!(
            resample_linear_i16(&[0, 1000, 2000], 3, 3),
            vec![0, 1000, 2000]
        );
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
}
