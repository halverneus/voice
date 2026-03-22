# voice — Live Whisper dictation for Linux/Wayland

Small Slint GUI app that does VAD-triggered live speech-to-text via
whisper.cpp and injects the result directly into the focused window with
`wtype` (no clipboard, no manual stop-start).

## How it works

1. **cpal** captures mic audio at the device's native rate.
2. Audio is downmixed to mono and resampled to 16 kHz.
3. **Energy-based VAD** detects speech boundaries in 32 ms frames.
4. Complete speech segments are sent to **whisper.cpp** via `whisper-rs`.
5. Transcribed text is typed into the focused window via `wtype`.

Two threads run concurrently: one for audio/VAD and one for Whisper inference,
so audio is never dropped while the GPU is crunching.

## Requirements

Tools you already have:
- `wtype` — Wayland text injection (preferred, no daemon)
- `ydotool` + `ydotoold` — alternative injection method
- `rustup` / Rust stable toolchain
- Linuxbrew (`brew`) — provides compiler toolchain (see Build section)

## Build — immutable Fedora (Kinoite)

This system lacks `g++`/`gcc-devel`/`fontconfig-devel`. The `.cargo/config.toml`
in this repo handles everything automatically using linuxbrew.

### One-time setup

```bash
# C++ compiler and build tools
brew install llvm cmake

# Then build:
cargo build --release
```

That's it. No `rpm-ostree`, no pip, no virtual envs.

### CPU only (works everywhere, slower)

```bash
cargo build --release
```

### Nvidia GPU via Vulkan (recommended — vulkan-loader-devel already installed)

```bash
cargo build --release --features vulkan
```

### Nvidia GPU via CUDA (fastest, requires CUDA toolkit headers)

```bash
cargo build --release --features cuda
```

The binary lands at `target/release/voice`.

### Why linuxbrew clang++ instead of system gcc?

The immutable Fedora base image ships `libstdc++.so.6` (runtime) but not the
unversioned `libstdc++.so` linker stub that `gcc-devel` provides. The build
uses linuxbrew clang++ with libc++ and bakes the linuxbrew lib path into the
binary's rpath — all handled by `.cargo/config.toml`.

## First run

```bash
./target/release/voice
```

1. Settings open automatically if no model exists.
2. Choose a model. **Base (EN only)** is the recommended starting point.
3. Click **Download Model** and wait.
4. **Click in the app you want to dictate into** (Obsidian, etc.).
5. Click **● Start** in the Voice window.
6. Speak — text appears as each sentence is detected.
7. Click **■ Stop** when done.

**Key point:** Click your target app *before* speaking. The Voice window must
not have focus while you dictate. Once you've clicked Start, just click in
Obsidian and speak — you don't need to touch the Voice window again.

## Models

| Model | Size | Notes |
|-------|------|-------|
| tiny.en | 78 MB | Fastest; English only |
| base.en | 148 MB | **Recommended start** |
| small.en | 488 MB | Better accuracy |
| medium.en | 1.5 GB | High accuracy |
| large-v3-turbo | 1.6 GB | Best quality |

Models are stored in `~/.local/share/voice/models/`.

## Config

Auto-written to `~/.config/voice/config.toml` when you change settings in the UI.

```toml
model = "base.en"
input_device = ""          # empty = system default
inject_method = "wtype"    # or "ydotool"
language = "en"
n_threads = 4
vad_threshold = 0.5        # 0.0–1.0 mapped to RMS range 0–0.05
silence_duration_ms = 600  # flush segment after this many ms of silence
inject_delay_ms = 0        # delay before typing (increase if focus is slow)
```

**VAD threshold:** The default 0.5 maps to ~0.025 RMS energy. Raise it (toward 1.0)
if background noise triggers false speech. Lower it if you need to speak quietly.

## Troubleshooting

**Text doesn't appear:**
- Make sure your target window has keyboard focus *before* speaking.
- The Voice window must not be focused when you speak — click away after Start.
- Try `inject_method = "ydotool"` if `wtype` doesn't work (requires `ydotoold` daemon).

**Too much background noise triggers transcription:**
- Raise `vad_threshold` to 0.7–0.9.

**Long pause before text appears:**
- Use tiny.en or base.en models.
- Build with `--features vulkan` or `--features cuda`.
- Lower `silence_duration_ms` to 300–400.

**Vulkan build fails:**
- Make sure `vulkan-loader-devel` and `glslang` are installed (they are on this system).

**CUDA build fails:**
- Need CUDA toolkit headers: check if `cuda-devel` is available via brew or another source.
