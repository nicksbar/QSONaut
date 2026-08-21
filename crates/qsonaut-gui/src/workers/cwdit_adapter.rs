use cwdit_dsp::{Debouncer, Goertzel, MovingAverage, RunLengthEncoder, Threshold};
use cwdit_morse::{BootstrapDecoder, Decoded, TimingEstimator};

/// QSONaut adapter around cw-dit's IO-free DSP/Morse crates.
///
/// The input is the existing 12 kHz mono stream already produced by QSONaut's
/// audio worker. One adapter instance represents one selected CW channel.
pub(super) struct CwDitChannel {
    filter: Goertzel,
    smoother: MovingAverage,
    slicer: ChannelSlicer,
    rle: RunLengthEncoder,
    debouncer: Debouncer,
    decoder: BootstrapDecoder,
    sample_rate_hz: f32,
    block_len: u32,
    text: String,
}

enum ChannelSlicer {
    Classic(Threshold),
}

impl CwDitChannel {
    pub(super) fn new(sample_rate_hz: u32, tone_hz: u32, wpm: u8) -> Self {
        let sample_rate = sample_rate_hz as f32;
        let wpm = f32::from(wpm.clamp(5, 40));
        // Fine enough for period-based mark classification while leaving the
        // Goertzel reasonably selective around a 5–40 WPM audio channel.
        // Match cw-dit's audio decode policy: a quarter-dit integration
        // window preserves keying edges while still averaging RF noise.
        let block_len = ((0.25 * 1.2 / wpm) * sample_rate)
            .round()
            .max((sample_rate / tone_hz as f32).ceil() + 1.0)
            .max(16.0) as u32;
        let envelope_rate = sample_rate / block_len as f32;
        let dit_ticks = 1.2 * envelope_rate / wpm;
        let smoother = MovingAverage::new((dit_ticks / 4.0).round().clamp(2.0, 16.0) as usize);
        let slicer =
            ChannelSlicer::Classic(Threshold::new(envelope_rate, 1.0, 0.005).with_snr_gate(2.5));
        let min_run = (dit_ticks / 5.0).round().max(2.0) as u32;
        let decoder = BootstrapDecoder::new(TimingEstimator::from_wpm(wpm, envelope_rate))
            .with_adapt(true)
            .with_period_classification(true);
        Self {
            filter: Goertzel::new(tone_hz as f32, sample_rate, block_len),
            smoother,
            slicer,
            rle: RunLengthEncoder::new(),
            debouncer: Debouncer::new(min_run),
            decoder,
            sample_rate_hz: sample_rate,
            block_len,
            text: String::new(),
        }
    }

    pub(super) fn push_samples(&mut self, samples: &[f32]) -> Vec<Decoded> {
        let mut output = Vec::new();
        for &sample in samples {
            if let Some(envelope) = self.filter.push(sample) {
                let mark = match &mut self.slicer {
                    ChannelSlicer::Classic(slicer) => slicer.push(self.smoother.push(envelope)),
                };
                if let Some(run) = self.rle.push(mark).and_then(|run| self.debouncer.push(run)) {
                    for event in self.decoder.push(run.mark, run.duration) {
                        self.accumulate(event);
                        output.push(event);
                    }
                }
            }
        }
        output
    }

    pub(super) fn text(&self) -> &str {
        &self.text
    }

    pub(super) fn block_len(&self) -> u32 {
        self.block_len
    }

    pub(super) fn envelope_rate(&self) -> f32 {
        self.sample_rate_hz / self.block_len as f32
    }

    fn accumulate(&mut self, event: Decoded) {
        match event {
            Decoded::Char(character) => self.text.push(character),
            Decoded::WordBreak => self.text.push(' '),
            Decoded::Unknown => self.text.push('?'),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CwDitChannel;

    fn keyed_tone(text: &str, tone_hz: f32, wpm: f32) -> Vec<f32> {
        let dot = (12_000.0 * 1.2 / wpm) as usize;
        let map = |character: char| match character {
            'C' => "-.-.",
            'Q' => "--.-",
            'D' => "-..",
            'E' => ".",
            _ => "",
        };
        let mut samples = Vec::new();
        samples.extend(std::iter::repeat_n(0.0, 12_000));
        for (word_index, word) in text.split_whitespace().enumerate() {
            for (char_index, character) in word.chars().enumerate() {
                for (element_index, element) in map(character).chars().enumerate() {
                    let length = if element == '-' { dot * 3 } else { dot };
                    for index in 0..length {
                        samples.push(
                            (2.0 * std::f32::consts::PI * tone_hz * index as f32 / 12_000.0).sin()
                                * 0.5,
                        );
                    }
                    if element_index + 1 < map(character).len() {
                        samples.extend(std::iter::repeat_n(0.0, dot));
                    }
                }
                if char_index + 1 < word.len() {
                    samples.extend(std::iter::repeat_n(0.0, dot * 3));
                }
            }
            if word_index + 1 < text.split_whitespace().count() {
                samples.extend(std::iter::repeat_n(0.0, dot * 7));
            }
        }
        samples.extend(std::iter::repeat_n(0.0, 12_000));
        samples
    }

    #[test]
    fn selected_channel_decodes_generated_cw() {
        let samples = keyed_tone("CQ DE", 700.0, 20.0);
        let mut channel = CwDitChannel::new(12_000, 700, 20);
        let mut text = String::new();
        for event in channel.push_samples(&samples) {
            if let cwdit_morse::Decoded::Char(character) = event {
                text.push(character);
            }
        }
        assert!(text.contains('C') || text.contains('Q'));
    }

    #[test]
    fn selected_channel_ignores_silence() {
        let mut channel = CwDitChannel::new(12_000, 700, 20);
        assert!(channel.push_samples(&vec![0.0; 12_000 * 3]).is_empty());
    }
}
