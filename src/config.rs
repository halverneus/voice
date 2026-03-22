use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Available Whisper models with display info.
/// (id, filename, approx_size_mb, description)
pub const MODELS: &[(&str, &str, u64, &str)] = &[
    ("tiny.en",        "ggml-tiny.en.bin",        78,   "Tiny (EN only) — fastest, ~78 MB"),
    ("tiny",           "ggml-tiny.bin",            78,   "Tiny — fastest, multilingual, ~78 MB"),
    ("base.en",        "ggml-base.en.bin",        148,   "Base (EN only) — recommended, ~148 MB"),
    ("base",           "ggml-base.bin",           148,   "Base — multilingual, ~148 MB"),
    ("small.en",       "ggml-small.en.bin",       488,   "Small (EN only) — better accuracy, ~488 MB"),
    ("small",          "ggml-small.bin",          488,   "Small — multilingual, ~488 MB"),
    ("medium.en",      "ggml-medium.en.bin",     1533,   "Medium (EN only) — high accuracy, ~1.5 GB"),
    ("medium",         "ggml-medium.bin",        1533,   "Medium — multilingual, ~1.5 GB"),
    (
        "large-v3-turbo",
        "ggml-large-v3-turbo.bin",
        1620,
        "Large-v3-turbo — best, ~1.6 GB",
    ),
];

pub const MODEL_BASE_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/";

pub fn model_url(filename: &str) -> String {
    format!("{}{}", MODEL_BASE_URL, filename)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum InjectMethod {
    Wtype,
    Ydotool,
}

impl Default for InjectMethod {
    fn default() -> Self {
        InjectMethod::Wtype
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Model ID from MODELS list (e.g. "base.en")
    #[serde(default = "default_model")]
    pub model: String,

    /// Directory where model .bin files are stored
    #[serde(default = "default_models_dir")]
    pub models_dir: String,

    /// Audio input device name, or empty for system default
    #[serde(default)]
    pub input_device: String,

    /// How to inject text into the focused window
    #[serde(default)]
    pub inject_method: InjectMethod,

    /// Whisper language code (e.g. "en", "auto")
    #[serde(default = "default_language")]
    pub language: String,

    /// Number of CPU threads for Whisper inference
    #[serde(default = "default_n_threads")]
    pub n_threads: u32,

    /// Silero VAD threshold 0.0–1.0 (higher = more conservative)
    #[serde(default = "default_vad_threshold")]
    pub vad_threshold: f32,

    /// Milliseconds of silence before flushing a speech segment to Whisper
    #[serde(default = "default_silence_ms")]
    pub silence_duration_ms: u64,

    /// Delay in ms between receiving text and injecting it (helps with focus)
    #[serde(default = "default_inject_delay_ms")]
    pub inject_delay_ms: u64,
}

fn default_model() -> String {
    "base.en".to_string()
}
fn default_models_dir() -> String {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("voice")
        .join("models")
        .to_string_lossy()
        .to_string()
}
fn default_language() -> String {
    "en".to_string()
}
fn default_n_threads() -> u32 {
    4
}
fn default_vad_threshold() -> f32 {
    0.5
}
fn default_silence_ms() -> u64 {
    600
}
fn default_inject_delay_ms() -> u64 {
    0
}

impl Default for Config {
    fn default() -> Self {
        Self {
            model: default_model(),
            models_dir: default_models_dir(),
            input_device: String::new(),
            inject_method: InjectMethod::default(),
            language: default_language(),
            n_threads: default_n_threads(),
            vad_threshold: default_vad_threshold(),
            silence_duration_ms: default_silence_ms(),
            inject_delay_ms: default_inject_delay_ms(),
        }
    }
}

impl Config {
    pub fn load() -> Self {
        match std::fs::read_to_string(config_path()) {
            Ok(s) => toml::from_str(&s).unwrap_or_default(),
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    pub fn models_dir_path(&self) -> PathBuf {
        let p = PathBuf::from(&self.models_dir);
        // Expand ~ manually since std doesn't do it
        if let Ok(stripped) = p.strip_prefix("~") {
            if let Some(home) = dirs::home_dir() {
                return home.join(stripped);
            }
        }
        p
    }

    pub fn model_path(&self) -> PathBuf {
        let filename = MODELS
            .iter()
            .find(|(id, ..)| *id == self.model)
            .map(|(_, f, ..)| *f)
            .unwrap_or("ggml-base.en.bin");
        self.models_dir_path().join(filename)
    }

    pub fn model_filename(&self) -> &'static str {
        MODELS
            .iter()
            .find(|(id, ..)| *id == self.model)
            .map(|(_, f, ..)| *f)
            .unwrap_or("ggml-base.en.bin")
    }
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("voice")
        .join("config.toml")
}
