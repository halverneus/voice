use anyhow::Result;

/// Energy-based Voice Activity Detector.
///
/// Processes 16 kHz mono f32 audio in 512-sample (32 ms) frames.
/// Speech is detected when the RMS energy exceeds a threshold.
///
/// The `threshold` parameter (0.0–1.0 from config) is mapped to an RMS
/// range of 0.0–0.1, so the default 0.5 → 0.05 RMS.
/// Typical microphone voice: 0.03–0.15 RMS.
/// Typical silence/noise floor: 0.005–0.02 RMS.
///
/// If Silero-based detection is needed in the future, replace this struct
/// while keeping the same `feed()` interface.
pub struct Vad {
    /// RMS threshold above which a frame is considered speech
    threshold: f32,
    /// Buffered samples waiting to form a complete frame
    buf: Vec<f32>,
}

const FRAME: usize = 512; // 32 ms at 16 kHz

impl Vad {
    /// `threshold`: 0.0–1.0 from config, mapped to 0.0–0.1 RMS range.
    pub fn new(threshold: f32) -> Result<Self> {
        // Map user's 0..1 to a reasonable RMS range.
        // 0.5 → 0.025  (comfortable default for a quiet room)
        let rms = (threshold * 0.05).max(0.001_f32);
        Ok(Self {
            threshold: rms,
            buf: Vec::new(),
        })
    }

    /// Feed arbitrary-length 16 kHz mono f32 samples.
    /// Returns `(is_speech, frame)` for every complete 512-sample window consumed.
    /// Leftover samples are buffered internally for the next call.
    pub fn feed(&mut self, samples: &[f32]) -> Vec<(bool, Vec<f32>)> {
        self.buf.extend_from_slice(samples);
        let mut results = Vec::new();
        while self.buf.len() >= FRAME {
            let frame: Vec<f32> = self.buf.drain(..FRAME).collect();
            let rms = rms(&frame);
            results.push((rms > self.threshold, frame));
        }
        results
    }
}

fn rms(samples: &[f32]) -> f32 {
    let sum_sq: f32 = samples.iter().map(|&s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}
