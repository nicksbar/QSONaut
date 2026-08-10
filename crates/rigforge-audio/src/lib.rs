use std::{
    fs,
    path::Path,
    process::Command,
    time::Duration,
};

use anyhow::{bail, Context, Result};

#[derive(Debug, Clone)]
pub struct AudioDevice {
    pub name: String,
    pub default_sample_rate_hz: u32,
    pub channels: u8,
}

#[derive(Debug, Clone)]
pub struct CaptureOptions {
    pub preferred_device_name: Option<String>,
    pub sample_rate_hz: Option<u32>,
    pub channels: Option<u16>,
    pub duration: Duration,
}

impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            preferred_device_name: None,
            sample_rate_hz: Some(48_000),
            channels: Some(1),
            duration: Duration::from_secs(10),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CaptureSummary {
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub samples_written: u64,
    pub peak_dbfs: f32,
    pub rms_dbfs: f32,
}

#[derive(Debug, Default, Clone)]
pub struct AudioService;

impl AudioService {
    pub async fn enumerate_devices(&self) -> Result<Vec<AudioDevice>> {
        self.enumerate_from_proc_asound()
    }

    pub fn capture_wav<P: AsRef<Path>>(&self, path: P, options: CaptureOptions) -> Result<CaptureSummary> {
        self.preflight_audio_permissions()?;

        let sample_rate_hz = options.sample_rate_hz.unwrap_or(48_000);
        let channels = options.channels.unwrap_or(1);
        let duration_secs = options.duration.as_secs().max(1);

        let mut cmd = Command::new("arecord");
        cmd.arg("-f")
            .arg("S16_LE")
            .arg("-r")
            .arg(sample_rate_hz.to_string())
            .arg("-c")
            .arg(channels.to_string())
            .arg("-d")
            .arg(duration_secs.to_string())
            .arg(path.as_ref());

        if let Some(device_name) = options.preferred_device_name.as_deref() {
            cmd.arg("-D").arg(device_name);
        }

        let status = cmd.status().context(
            "failed to execute arecord; install alsa-utils and verify audio permissions/device names",
        )?;
        if !status.success() {
            bail!("arecord failed with status {status}");
        }

        let mut reader = hound::WavReader::open(path.as_ref())
            .with_context(|| format!("failed to open captured WAV {}", path.as_ref().display()))?;
        let mut stats = SignalStats::default();
        for s in reader.samples::<i16>() {
            let s = s.context("invalid sample in captured WAV")?;
            stats.update(s as f32 / i16::MAX as f32);
        }

        Ok(CaptureSummary {
            sample_rate_hz,
            channels,
            samples_written: stats.samples,
            peak_dbfs: amplitude_to_dbfs(stats.peak_abs),
            rms_dbfs: amplitude_to_dbfs(stats.rms()),
        })
    }

    fn preflight_audio_permissions(&self) -> Result<()> {
        let probe = Path::new("/dev/snd/controlC0");
        if probe.exists() {
            match std::fs::File::open(probe) {
                Ok(_) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => bail!(
                    "no permission to access {}. Add your user to the 'audio' group and re-login/restart WSL",
                    probe.display()
                ),
                Err(_) => Ok(()),
            }
        } else {
            Ok(())
        }
    }

    fn enumerate_from_proc_asound(&self) -> Result<Vec<AudioDevice>> {
        let cards = fs::read_to_string("/proc/asound/cards").unwrap_or_default();
        let mut out = Vec::new();
        for line in cards.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed
                .chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
            {
                out.push(AudioDevice {
                    name: trimmed.to_string(),
                    default_sample_rate_hz: 48_000,
                    channels: 1,
                });
            }
        }
        Ok(out)
    }
}

#[derive(Debug, Default, Clone)]
struct SignalStats {
    samples: u64,
    sum_sq: f64,
    peak_abs: f32,
}

impl SignalStats {
    fn update(&mut self, s: f32) {
        let a = s.abs();
        if a > self.peak_abs {
            self.peak_abs = a;
        }
        self.sum_sq += (s as f64) * (s as f64);
        self.samples += 1;
    }

    fn rms(&self) -> f32 {
        if self.samples == 0 {
            0.0
        } else {
            (self.sum_sq / self.samples as f64).sqrt() as f32
        }
    }
}

fn amplitude_to_dbfs(amplitude: f32) -> f32 {
    if amplitude <= 0.0 {
        -120.0
    } else {
        20.0 * amplitude.log10()
    }
}
