mod audio;
mod config;
mod downloader;
mod injector;
mod transcriber;
mod vad;

use crate::config::{Config, InjectMethod, OutputMode, MODELS};
use anyhow::Result;
use slint::{ModelRc, SharedString, VecModel};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

slint::include_modules!();

// ─── Shared app state ────────────────────────────────────────────────────────

struct AppState {
    config: Config,
    /// Dropping the Stream stops cpal audio capture
    _audio_stream: Option<cpal::Stream>,
    active: Arc<AtomicBool>,
}

impl AppState {
    fn new(config: Config) -> Self {
        Self {
            config,
            _audio_stream: None,
            active: Arc::new(AtomicBool::new(false)),
        }
    }
}

// ─── Entry point ─────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let config = Config::load();
    let state = Arc::new(Mutex::new(AppState::new(config)));

    let window = MainWindow::new()?;

    // ── Populate model list ───────────────────────────────────────────────────
    {
        let model_names: Vec<SharedString> = MODELS
            .iter()
            .map(|(_, _, _, desc)| SharedString::from(*desc))
            .collect();
        window.set_model_names(ModelRc::new(VecModel::from(model_names)));

        let st = state.lock().unwrap();
        let current_idx = MODELS
            .iter()
            .position(|(id, ..)| *id == st.config.model)
            .unwrap_or(0) as i32;
        window.set_model_index(current_idx);
    }

    // ── Populate mic list ─────────────────────────────────────────────────────
    {
        let devices = audio::list_input_devices();
        let device_names: Vec<SharedString> = devices.iter().map(SharedString::from).collect();
        window.set_device_names(ModelRc::new(VecModel::from(device_names)));

        let st = state.lock().unwrap();
        let dev_idx = if st.config.input_device.is_empty() || st.config.input_device == "Default" {
            0i32
        } else {
            audio::list_input_devices()
                .iter()
                .position(|d| *d == st.config.input_device)
                .unwrap_or(0) as i32
        };
        window.set_device_index(dev_idx);
    }

    // ── Set inject method ────────────────────────────────────────────────────
    {
        let st = state.lock().unwrap();
        let idx = match st.config.inject_method {
            InjectMethod::Wtype   => 0i32,
            InjectMethod::Ydotool => 1i32,
        };
        window.set_inject_method_index(idx);
    }

    // ── Set output mode + file path ──────────────────────────────────────────
    {
        let st = state.lock().unwrap();
        let idx = match st.config.output_mode {
            OutputMode::Inject => 0i32,
            OutputMode::File   => 1i32,
        };
        window.set_output_mode_index(idx);
        window.set_output_file_path(st.config.output_file.clone().into());
    }

    // ── Check model exists ────────────────────────────────────────────────────
    {
        let st = state.lock().unwrap();
        let missing = !st.config.model_path().exists();
        window.set_model_missing(missing);
        window.set_show_settings(missing);
        if missing {
            window.set_status_text(
                "No model downloaded — open Settings to download one".into(),
            );
        }
    }

    // ── Callbacks ────────────────────────────────────────────────────────────

    // Toggle dictation Start / Stop
    {
        let window_weak = window.as_weak();
        let state_clone = state.clone();
        window.on_toggle_dictation(move || {
            let mut st = state_clone.lock().unwrap();

            if st.active.load(Ordering::Relaxed) {
                // ── STOP ──────────────────────────────────────────────────────
                st.active.store(false, Ordering::Relaxed);
                st._audio_stream = None; // drop → cpal stops → audio_rx disconnects
                if let Some(w) = window_weak.upgrade() {
                    w.set_is_listening(false);
                    w.set_is_processing(false);
                    w.set_status_text("Ready".into());
                }
                log::info!("Dictation stopped.");
            } else {
                // ── START ──────────────────────────────────────────────────────
                let model_path = st.config.model_path();
                if !model_path.exists() {
                    if let Some(w) = window_weak.upgrade() {
                        w.set_status_text(
                            "Model not found — download it in Settings".into(),
                        );
                        w.set_show_settings(true);
                    }
                    return;
                }

                // Validate prerequisites based on output mode
                match st.config.output_mode {
                    OutputMode::Inject => {
                        if st.config.inject_method == InjectMethod::Ydotool {
                            injector::ensure_ydotoold_running();
                        }
                        if !injector::check_inject_tool(&st.config.inject_method) {
                            let tool = match st.config.inject_method {
                                InjectMethod::Wtype   => "wtype",
                                InjectMethod::Ydotool => "ydotool",
                            };
                            if let Some(w) = window_weak.upgrade() {
                                w.set_status_text(
                                    format!("'{}' not found in PATH — install it first", tool).into(),
                                );
                            }
                            return;
                        }
                    }
                    OutputMode::File => {
                        if st.config.output_file.is_empty() {
                            if let Some(w) = window_weak.upgrade() {
                                w.set_status_text(
                                    "No output file chosen — open Settings to select one".into(),
                                );
                                w.set_show_settings(true);
                            }
                            return;
                        }
                    }
                }

                if let Some(w) = window_weak.upgrade() {
                    w.set_is_loading(true);
                    w.set_status_text("Loading model…".into());
                }

                st.active.store(true, Ordering::Relaxed);

                // Clone everything the threads need before spawning
                let active_vad     = st.active.clone();
                let active_whisper = st.active.clone();
                let cfg_vad        = st.config.clone();
                let cfg_whisper    = st.config.clone();
                let ww_whisper     = window_weak.clone();

                // Bounded audio channel (cpal callback → VAD thread)
                let (audio_tx, audio_rx) = mpsc::sync_channel::<Vec<f32>>(512);
                // Unbounded segment channel (VAD → Whisper)
                let (segment_tx, segment_rx) = mpsc::sync_channel::<Vec<f32>>(8);

                // Thread A: VAD + speech accumulation
                thread::Builder::new()
                    .name("vad".into())
                    .spawn(move || {
                        run_vad_thread(audio_rx, segment_tx, active_vad, cfg_vad);
                    })
                    .expect("Failed to spawn VAD thread");

                // Thread B: Whisper inference + text injection
                thread::Builder::new()
                    .name("whisper".into())
                    .spawn(move || {
                        run_whisper_thread(segment_rx, active_whisper, cfg_whisper, ww_whisper);
                    })
                    .expect("Failed to spawn Whisper thread");

                // Start audio capture; store stream (dropping it stops capture)
                match audio::start_capture(&st.config.input_device, audio_tx) {
                    Ok(stream) => {
                        st._audio_stream = Some(stream);
                        log::info!("Dictation started.");
                    }
                    Err(e) => {
                        st.active.store(false, Ordering::Relaxed);
                        if let Some(w) = window_weak.upgrade() {
                            w.set_is_loading(false);
                            w.set_status_text(format!("Audio error: {}", e).into());
                        }
                    }
                }
            }
        });
    }

    // Model selection changed
    {
        let window_weak = window.as_weak();
        let state_clone = state.clone();
        window.on_model_changed(move |idx| {
            if let Some(&(id, _, _, _)) = MODELS.get(idx as usize) {
                let mut st = state_clone.lock().unwrap();
                st.config.model = id.to_string();
                let _ = st.config.save();
                let missing = !st.config.model_path().exists();
                drop(st);
                if let Some(w) = window_weak.upgrade() {
                    w.set_model_missing(missing);
                    w.set_download_status("".into());
                    w.set_downloading(false);
                    w.set_download_progress(0.0);
                }
            }
        });
    }

    // Device selection changed
    {
        let state_clone = state.clone();
        window.on_device_changed(move |idx| {
            let devices = audio::list_input_devices();
            if let Some(name) = devices.get(idx as usize) {
                let mut st = state_clone.lock().unwrap();
                st.config.input_device = if name == "Default" {
                    String::new()
                } else {
                    name.clone()
                };
                let _ = st.config.save();
            }
        });
    }

    // Inject method changed
    {
        let state_clone = state.clone();
        window.on_inject_method_changed(move |idx| {
            let method = match idx {
                1 => InjectMethod::Ydotool,
                _ => InjectMethod::Wtype,
            };
            // Start ydotoold immediately when the user switches to ydotool
            // so the daemon is ready before the first dictation session.
            if method == InjectMethod::Ydotool {
                std::thread::spawn(|| injector::ensure_ydotoold_running());
            }
            let mut st = state_clone.lock().unwrap();
            st.config.inject_method = method;
            let _ = st.config.save();
        });
    }

    // Output mode changed
    {
        let state_clone = state.clone();
        window.on_output_mode_changed(move |idx| {
            let mode = match idx {
                1 => OutputMode::File,
                _ => OutputMode::Inject,
            };
            let mut st = state_clone.lock().unwrap();
            st.config.output_mode = mode;
            let _ = st.config.save();
        });
    }

    // Browse for output file
    {
        let window_weak = window.as_weak();
        let state_clone = state.clone();
        window.on_choose_output_file(move || {
            let ww = window_weak.clone();
            let sc = state_clone.clone();
            thread::Builder::new()
                .name("filepicker".into())
                .spawn(move || {
                    let chosen = open_file_dialog();
                    if let Some(path) = chosen {
                        let mut st = sc.lock().unwrap();
                        st.config.output_file = path.clone();
                        let _ = st.config.save();
                        drop(st);
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(w) = ww.upgrade() {
                                w.set_output_file_path(path.into());
                            }
                        });
                    }
                })
                .expect("Failed to spawn filepicker thread");
        });
    }

    // Download model
    {
        let window_weak = window.as_weak();
        let state_clone = state.clone();
        window.on_download_model(move || {
            let st = state_clone.lock().unwrap();
            let url = config::model_url(st.config.model_filename());
            let dest = st.config.model_path();
            let model_name = st.config.model.clone();
            drop(st);

            if let Some(w) = window_weak.upgrade() {
                w.set_downloading(true);
                w.set_download_status(format!("Starting {}…", model_name).into());
                w.set_download_progress(0.0);
            }

            let ww = window_weak.clone();
            thread::Builder::new()
                .name("download".into())
                .spawn(move || {
                    let result = downloader::download(&url, &dest, |done, total| {
                        let pct = total.map(|t| done as f32 / t as f32).unwrap_or(0.0);
                        let mb = done / 1_048_576;
                        let status = total
                            .map(|t| format!("{}/{} MB", mb, t / 1_048_576))
                            .unwrap_or_else(|| format!("{} MB", mb));
                        let ww2 = ww.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(w) = ww2.upgrade() {
                                w.set_download_status(status.into());
                                w.set_download_progress(pct);
                            }
                        });
                    });

                    let ww2 = ww.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(w) = ww2.upgrade() {
                            match result {
                                Ok(()) => {
                                    w.set_downloading(false);
                                    w.set_model_missing(false);
                                    w.set_download_status("Download complete!".into());
                                    w.set_status_text("Ready".into());
                                }
                                Err(e) => {
                                    w.set_downloading(false);
                                    w.set_download_status(format!("Error: {}", e).into());
                                }
                            }
                        }
                    });
                })
                .expect("Failed to spawn download thread");
        });
    }

    window.run()?;
    Ok(())
}

// ─── VAD thread ───────────────────────────────────────────────────────────────

fn run_vad_thread(
    audio_rx: mpsc::Receiver<Vec<f32>>,
    segment_tx: mpsc::SyncSender<Vec<f32>>,
    active: Arc<AtomicBool>,
    cfg: Config,
) {
    let mut vad = match vad::Vad::new(cfg.vad_threshold) {
        Ok(v) => v,
        Err(e) => {
            log::error!("VAD init failed: {}", e);
            return;
        }
    };

    const MIN_SPEECH_SAMPLES: usize = 8_000;    // 0.5 s at 16 kHz
    const MAX_SPEECH_SAMPLES: usize = 16_000 * 25; // 25 s max

    // VAD runs at 512 samples = 32 ms per frame
    // silence_frames_needed = silence_ms / 32
    let silence_frames_needed = (cfg.silence_duration_ms / 32).max(5) as u32;

    let mut speech_buf: Vec<f32> = Vec::new();
    let mut silence_frames: u32 = 0;

    loop {
        match audio_rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(chunk) => {
                for (is_speech, frame) in vad.feed(&chunk) {
                    if is_speech {
                        silence_frames = 0;
                        speech_buf.extend_from_slice(&frame);
                        if speech_buf.len() >= MAX_SPEECH_SAMPLES {
                            flush_segment(&speech_buf, &segment_tx);
                            speech_buf.clear();
                        }
                    } else if !speech_buf.is_empty() {
                        speech_buf.extend_from_slice(&frame);
                        silence_frames += 1;
                        if silence_frames >= silence_frames_needed {
                            if speech_buf.len() >= MIN_SPEECH_SAMPLES {
                                flush_segment(&speech_buf, &segment_tx);
                            }
                            speech_buf.clear();
                            silence_frames = 0;
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !active.load(Ordering::Relaxed) {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    // Flush remaining speech
    if speech_buf.len() >= MIN_SPEECH_SAMPLES {
        flush_segment(&speech_buf, &segment_tx);
    }
    log::info!("VAD thread exited.");
}

fn flush_segment(buf: &[f32], tx: &mpsc::SyncSender<Vec<f32>>) {
    let _ = tx.try_send(buf.to_vec());
}

// ─── Whisper thread ───────────────────────────────────────────────────────────

fn run_whisper_thread(
    segment_rx: mpsc::Receiver<Vec<f32>>,
    active: Arc<AtomicBool>,
    cfg: Config,
    window: slint::Weak<MainWindow>,
) {
    // Show loading state
    {
        let w = window.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(win) = w.upgrade() {
                win.set_is_loading(true);
                win.set_status_text("Loading model…".into());
            }
        });
    }

    let mut engine = match transcriber::Transcriber::new(
        &cfg.model_path(),
        cfg.n_threads,
        &cfg.language,
    ) {
        Ok(e) => e,
        Err(err) => {
            log::error!("{}", err);
            let msg = format!("Model error: {}", err);
            let w = window.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(win) = w.upgrade() {
                    win.set_is_loading(false);
                    win.set_is_listening(false);
                    win.set_status_text(msg.into());
                }
            });
            return;
        }
    };

    // Model loaded — go live
    {
        let w = window.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(win) = w.upgrade() {
                win.set_is_loading(false);
                win.set_is_listening(true);
                win.set_status_text("Listening…".into());
            }
        });
    }

    let inject_method = cfg.inject_method.clone();
    let inject_delay  = cfg.inject_delay_ms;
    let output_mode   = cfg.output_mode.clone();
    let output_file   = cfg.output_file.clone();
    let mut transcript_buf = String::new();

    // segment_rx implements IntoIterator — loop ends when VAD thread drops sender
    for segment in segment_rx {
        if !active.load(Ordering::Relaxed) {
            break;
        }

        {
            let w = window.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(win) = w.upgrade() {
                    win.set_is_processing(true);
                    win.set_status_text("Processing…".into());
                }
            });
        }

        match engine.transcribe(&segment) {
            Ok(Some(text)) => {
                log::info!("Transcribed: {:?}", text);
                match output_mode {
                    OutputMode::Inject => {
                        injector::inject_text(&text, &inject_method, inject_delay);
                    }
                    OutputMode::File => {
                        append_to_file(&output_file, &text);
                    }
                }

                // Rolling transcript display (~200 chars)
                if !transcript_buf.is_empty() {
                    transcript_buf.push(' ');
                }
                transcript_buf.push_str(&text);
                if transcript_buf.len() > 250 {
                    let keep = transcript_buf.len() - 200;
                    if let Some(offset) = transcript_buf[keep..].char_indices().next() {
                        transcript_buf = transcript_buf[keep + offset.0..].to_string();
                    }
                }

                let display = transcript_buf.clone();
                let w = window.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = w.upgrade() {
                        win.set_is_processing(false);
                        win.set_is_listening(true);
                        win.set_transcript(display.into());
                        win.set_status_text("Listening…".into());
                    }
                });
            }
            Ok(None) => {
                let w = window.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = w.upgrade() {
                        win.set_is_processing(false);
                        win.set_is_listening(true);
                        win.set_status_text("Listening…".into());
                    }
                });
            }
            Err(e) => {
                log::error!("Transcription error: {}", e);
                let msg = format!("Error: {}", e);
                let w = window.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = w.upgrade() {
                        win.set_is_processing(false);
                        win.set_status_text(msg.into());
                    }
                });
            }
        }
    }

    // Dictation session ended
    let _ = active.compare_exchange(true, false, Ordering::Relaxed, Ordering::Relaxed);
    let w = window.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(win) = w.upgrade() {
            win.set_is_loading(false);
            win.set_is_listening(false);
            win.set_is_processing(false);
            win.set_status_text("Ready".into());
        }
    });

    log::info!("Whisper thread exited.");
}

// ─── File append helper ───────────────────────────────────────────────────────

fn append_to_file(path: &str, text: &str) {
    use std::fs::OpenOptions;
    use std::io::Write;
    match OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut f) => {
            if let Err(e) = write!(f, "{} ", text) {
                log::error!("Failed to write to output file '{}': {}", path, e);
            }
        }
        Err(e) => log::error!("Failed to open output file '{}': {}", path, e),
    }
}

// ─── Native file picker ───────────────────────────────────────────────────────

fn open_file_dialog() -> Option<String> {
    // Try kdialog first (native on KDE/Plasma)
    if let Ok(out) = std::process::Command::new("kdialog")
        .args([
            "--getsavefilename",
            &dirs::home_dir()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
            "Text files (*.txt *.md *.org);;All files (*)",
        ])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }

    // Fallback: zenity (GTK desktops / GNOME)
    if let Ok(out) = std::process::Command::new("zenity")
        .args([
            "--file-selection",
            "--save",
            "--title=Choose output file",
            "--file-filter=Text files|*.txt *.md *.org",
            "--file-filter=All files|*",
        ])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }

    log::warn!("No file picker available (kdialog and zenity not found)");
    None
}
