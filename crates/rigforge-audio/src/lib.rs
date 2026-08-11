use std::{
    io::{BufReader, Read},
    path::Path,
    process::{Child, Command, Stdio},
};

use anyhow::{bail, Context, Result};

#[derive(Debug, Clone)]
pub struct AudioService {
    preferred_device_name: Option<String>,
    enabled: bool,
}

#[derive(Debug)]
pub struct AudioStream {
    _child: Child,
    stdout: BufReader<std::process::ChildStdout>,
    _sample_rate_hz: u32,
    channels: u16,
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

    pub fn open_stream(&self, sample_rate_hz: u32, channels: u16) -> Result<AudioStream> {
        if !self.enabled {
            bail!("audio capture disabled by config; enable it with RIGFORGE_AUDIO_ENABLED=true or audio.enabled=true");
        }

        self.preflight_audio_permissions()?;

        let mut cmd = Command::new("arecord");
        cmd.arg("-q")
            .arg("-f")
            .arg("S16_LE")
            .arg("-r")
            .arg(sample_rate_hz.to_string())
            .arg("-c")
            .arg(channels.to_string())
            .arg("-t")
            .arg("raw")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(device_name) = self.preferred_device_name.as_deref() {
            if !device_name.trim().is_empty() {
                cmd.arg("-D").arg(device_name);
            }
        }

        let mut child = cmd.spawn().context("failed to spawn arecord for streaming capture")?;
        let stdout = BufReader::new(child.stdout.take().context("arecord stdout was not captured")?);

        Ok(AudioStream {
            _child: child,
            stdout,
            _sample_rate_hz: sample_rate_hz,
            channels,
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

}

impl AudioStream {
    pub fn read_chunk(&mut self, bytes_to_read: usize) -> Result<Vec<i16>> {
        let mut buf = vec![0u8; bytes_to_read];
        self.stdout.read_exact(&mut buf).context("audio stream ended")?;
        decode_pcm_bytes_to_i16_samples(&buf, self.channels as usize)
            .context("failed to decode PCM audio bytes")
    }
}

fn decode_pcm_bytes_to_i16_samples(bytes: &[u8], channels: usize) -> Result<Vec<i16>> {
    if channels == 0 {
        bail!("audio channel count must be positive");
    }
    if bytes.len() % (2 * channels) != 0 {
        bail!("PCM byte length is not an even multiple of the sample size");
    }

    let mut out = Vec::with_capacity(bytes.len() / (2 * channels));
    for chunk in bytes.chunks_exact(2 * channels) {
        let mut sample = 0i16;
        for frame in chunk.chunks_exact(2) {
            let raw = i16::from_le_bytes([frame[0], frame[1]]);
            sample = sample.saturating_add(raw);
        }
        out.push(sample / channels as i16);
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_pcm_bytes_to_i16_samples_handles_mono_payload() {
        let bytes = [0x00, 0x00, 0x00, 0x80, 0xFF, 0x7F, 0x00, 0xC0];
        let samples = decode_pcm_bytes_to_i16_samples(&bytes, 1).expect("mono PCM should decode");
        assert_eq!(samples, vec![0i16, -32768i16, 32767i16, -16384i16]);
    }
}
