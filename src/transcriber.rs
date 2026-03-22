use anyhow::Result;
use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct Transcriber {
    ctx: WhisperContext,
    n_threads: u32,
    language: String,
}

impl Transcriber {
    pub fn new(model_path: &Path, n_threads: u32, language: &str) -> Result<Self> {
        log::info!("Loading Whisper model: {}", model_path.display());

        let mut params = WhisperContextParameters::default();
        // use_gpu defaults to true when compiled with cuda/vulkan feature;
        // calling it explicitly here makes intent clear.
        params.use_gpu(true);

        // WhisperContext::new_with_params accepts anything that implements AsRef<Path>
        let ctx = WhisperContext::new_with_params(model_path, params)
            .map_err(|e| anyhow::anyhow!("Failed to load '{}': {:?}", model_path.display(), e))?;

        log::info!("Model loaded.");
        Ok(Self {
            ctx,
            n_threads,
            language: language.to_string(),
        })
    }

    /// Transcribe 16 kHz mono f32 audio. Returns cleaned text, or None if
    /// nothing useful was recognised.
    pub fn transcribe(&mut self, audio: &[f32]) -> Result<Option<String>> {
        let mut state = self.ctx.create_state()?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(self.n_threads as i32);
        params.set_language(Some(&self.language));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_single_segment(true);
        params.set_no_speech_thold(0.6);

        state.full(params, audio)?;

        // full_n_segments() returns c_int (i32) directly in whisper-rs 0.16
        let n = state.full_n_segments();
        let mut text = String::new();
        for i in 0..n {
            if let Some(seg) = state.get_segment(i) {
                // to_str_lossy never fails (replaces invalid UTF-8 with replacement char)
                if let Ok(s) = seg.to_str_lossy() {
                    text.push_str(&s);
                }
            }
        }

        Ok(clean_text(&text))
    }
}

/// Filter Whisper hallucinations and normalise whitespace.
fn clean_text(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    // "[BLANK_AUDIO]", "[inaudible]", etc.
    if s.starts_with('[') && s.ends_with(']') {
        return None;
    }
    // "(background noise)", "(silence)", etc.
    if s.starts_with('(') && s.ends_with(')') {
        return None;
    }
    // Common hallucinations on silent/noise-only audio
    let lower = s.to_lowercase();
    let hallucinations = [
        "thank you for watching",
        "thanks for watching",
        "thank you.",
        "thanks.",
    ];
    for h in hallucinations {
        if lower == h {
            return None;
        }
    }
    Some(s.trim_start().to_string())
}
