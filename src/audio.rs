use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc;

/// Returns ["Default", device1, device2, ...]
pub fn list_input_devices() -> Vec<String> {
    let host = cpal::default_host();
    let mut names = vec!["Default".to_string()];
    if let Ok(devices) = host.input_devices() {
        for d in devices {
            if let Ok(desc) = d.description() {
                names.push(desc.name().to_string());
            }
        }
    }
    names
}

/// Get the display name for a device.
fn device_display_name(d: &cpal::Device) -> String {
    use cpal::traits::DeviceTrait;
    d.description()
        .map(|desc| desc.name().to_string())
        .unwrap_or_else(|_| "<unknown>".to_string())
}

/// Resample `input` from `in_rate` Hz mono f32 to 16 kHz using linear
/// interpolation. Fast and good enough for voice ASR.
pub fn resample_to_16k(input: &[f32], in_rate: u32) -> Vec<f32> {
    if in_rate == 16_000 {
        return input.to_vec();
    }
    let ratio = in_rate as f64 / 16_000.0;
    let out_len = (input.len() as f64 / ratio) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src = i as f64 * ratio;
        let idx = src as usize;
        let frac = (src - idx as f64) as f32;
        let a = input.get(idx).copied().unwrap_or(0.0);
        let b = input.get(idx + 1).copied().unwrap_or(a);
        out.push(a + (b - a) * frac);
    }
    out
}

/// Average multi-channel interleaved samples to mono.
pub fn to_mono(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels == 1 {
        return samples.to_vec();
    }
    samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Start capturing audio from the named device (empty/Default = system default).
/// Sends 16 kHz mono f32 chunks down `tx`.
/// Drop the returned Stream to stop capture.
pub fn start_capture(
    device_name: &str,
    tx: mpsc::SyncSender<Vec<f32>>,
) -> Result<cpal::Stream> {
    let host = cpal::default_host();

    let device = if device_name.is_empty() || device_name == "Default" {
        host.default_input_device()
            .ok_or_else(|| anyhow!("No default input device found"))?
    } else {
        host.input_devices()?
            .find(|d| {
                d.description()
                    .map(|desc| desc.name() == device_name)
                    .unwrap_or(false)
            })
            .ok_or_else(|| anyhow!("Input device '{}' not found", device_name))?
    };

    let default_cfg = device.default_input_config()?;
    // In cpal 0.17, SampleRate and ChannelCount are plain type aliases (u32 / u16)
    let in_rate: u32 = default_cfg.sample_rate();
    let channels: usize = default_cfg.channels() as usize;
    let fmt = default_cfg.sample_format();

    log::info!(
        "Audio: device='{}' rate={}Hz channels={} format={:?}",
        device_display_name(&device),
        in_rate,
        channels,
        fmt,
    );

    let stream_cfg: cpal::StreamConfig = default_cfg.clone().into();
    let err_fn = |e| log::error!("Audio stream error: {}", e);

    let stream = match fmt {
        cpal::SampleFormat::F32 => {
            let tx2 = tx.clone();
            device.build_input_stream(
                &stream_cfg,
                move |data: &[f32], _| {
                    let mono = to_mono(data, channels);
                    let chunk = resample_to_16k(&mono, in_rate);
                    let _ = tx2.try_send(chunk);
                },
                err_fn,
                None,
            )?
        }
        cpal::SampleFormat::I16 => {
            let tx2 = tx.clone();
            device.build_input_stream(
                &stream_cfg,
                move |data: &[i16], _| {
                    let f32s: Vec<f32> = data.iter().map(|&s| s as f32 / 32_768.0).collect();
                    let mono = to_mono(&f32s, channels);
                    let chunk = resample_to_16k(&mono, in_rate);
                    let _ = tx2.try_send(chunk);
                },
                err_fn,
                None,
            )?
        }
        cpal::SampleFormat::U16 => {
            let tx2 = tx.clone();
            device.build_input_stream(
                &stream_cfg,
                move |data: &[u16], _| {
                    let f32s: Vec<f32> =
                        data.iter().map(|&s| (s as f32 - 32_768.0) / 32_768.0).collect();
                    let mono = to_mono(&f32s, channels);
                    let chunk = resample_to_16k(&mono, in_rate);
                    let _ = tx2.try_send(chunk);
                },
                err_fn,
                None,
            )?
        }
        other => {
            return Err(anyhow!("Unsupported sample format: {:?}", other));
        }
    };

    stream.play()?;
    Ok(stream)
}
