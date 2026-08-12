use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
        Arc,
    },
    time::Duration,
};

use anyhow::{anyhow, bail, Context, Result};
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, Sample, SampleFormat, SampleRate, Stream, StreamError, SupportedStreamConfig,
};

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
        if channels == 0 {
            bail!("audio channel count must be positive");
        }

        let host = cpal::default_host();
        let device = select_device(
            &host,
            AudioDeviceKind::Input,
            self.preferred_device_name.as_deref(),
        )?;
        let supported = select_config(&device, AudioDeviceKind::Input, sample_rate_hz, channels)?;
        let stream_config = supported.config();
        let device_channels = stream_config.channels as usize;
        let (samples_tx, samples_rx) = mpsc::sync_channel::<Vec<i16>>(32);
        let (errors_tx, errors_rx) = mpsc::channel::<String>();
        let error_callback = move |error: StreamError| {
            let _ = errors_tx.send(error.to_string());
        };

        macro_rules! input_stream {
            ($sample:ty) => {{
                let tx = samples_tx.clone();
                device.build_input_stream(
                    &stream_config,
                    move |data: &[$sample], _| {
                        let converted = data
                            .iter()
                            .copied()
                            .map(i16::from_sample)
                            .collect::<Vec<_>>();
                        send_downmixed_i16(&converted, device_channels, &tx);
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
            requested_channels: channels as usize,
        })
    }
}

impl AudioStream {
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

    match kind {
        AudioDeviceKind::Input => host.default_input_device(),
        AudioDeviceKind::Output => host.default_output_device(),
    }
    .ok_or_else(|| anyhow!("no default {} audio device is available", kind_label(kind)))
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
    let requested_rate = SampleRate(sample_rate_hz);
    ranges
        .iter()
        .filter(|range| {
            range.min_sample_rate() <= requested_rate && range.max_sample_rate() >= requested_rate
        })
        .min_by_key(|range| {
            (
                range.channels().abs_diff(requested_channels),
                sample_format_preference(range.sample_format()),
            )
        })
        .map(|range| (*range).with_sample_rate(requested_rate))
        .with_context(|| {
            format!(
                "{} audio device does not support {} Hz",
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

fn send_downmixed_i16(data: &[i16], channels: usize, sender: &SyncSender<Vec<i16>>) {
    if channels == 0 {
        return;
    }
    let output = data
        .chunks_exact(channels)
        .map(|frame| {
            let sum = frame.iter().map(|sample| i32::from(*sample)).sum::<i32>();
            (sum / channels as i32) as i16
        })
        .collect::<Vec<_>>();
    let _ = sender.try_send(output);
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
    fn downmixes_stereo_without_clipping() {
        let (tx, rx) = mpsc::sync_channel(1);
        send_downmixed_i16(&[i16::MAX, i16::MAX, i16::MIN, i16::MIN], 2, &tx);
        assert_eq!(rx.recv().unwrap(), vec![i16::MAX, i16::MIN]);
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
}
