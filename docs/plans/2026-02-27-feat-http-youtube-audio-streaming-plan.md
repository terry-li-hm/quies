---
title: "feat: HTTP/YouTube audio streaming layers"
type: feat
status: active
date: 2026-02-27
---

# feat: HTTP/YouTube Audio Streaming Layers

## Overview

Add the ability to play audio from URLs (YouTube via yt-dlp, direct HTTP audio files) as mixer layers alongside the existing noise generators. This turns quies from a noise-only tool into a full ambient sound mixer that can layer lofi music, nature soundscapes, and noise together.

Reference: [1-hour dark ambient focus track](https://youtu.be/0NCrui_dYJY) — the kind of content quies should play.

## Enhancement Summary

**Deepened on:** 2026-02-27
**Research agents used:** performance-oracle, architecture-strategist, code-simplicity-reviewer, yt-dlp/rodio-codec-researcher

### Key Changes from Research

1. **Cut ureq entirely** — use `curl` subprocess for direct HTTP downloads. Zero new compile dependencies. curl is ubiquitous on macOS/Linux; ureq would add ~15 transitive deps for a feature we can get from a subprocess.
2. **Cut `LayerKind` enum** — add `url: Option<String>` and `path: Option<PathBuf>` fields to `Layer` directly. Noise layers have `None` for both. Simpler, no new types.
3. **Cut `remove`, `layers`, `clean` commands** — `remove` can't actually remove from rodio's mixer (just silences). `layers` is redundant with `status`. `clean` is premature — `$TMPDIR` is cleaned by OS. Defer all three.
4. **yt-dlp format: `-f "bestaudio[ext=m4a]"`** — default `bestaudio` often returns opus-in-webm, which rodio can't decode without `symphonia-mkv` (not in default features). m4a (AAC in MP4) decodes out of the box.
5. **BufReader must be 256KB** — default 8KB = ~45ms of audio at 44.1kHz stereo f32. A single disk I/O stall >45ms causes an audible glitch. 256KB (~740ms buffer) provides comfortable margin.
6. **Atomic rename for downloads** — write to `.tmp` suffix, rename to final path on completion. Prevents the daemon from trying to decode a half-written file.
7. **Download status via `Arc<Mutex<LayerStatus>>`** — cleaner than `AtomicU8` encoding. Only touched on status transitions (not per-sample), so mutex cost is irrelevant.

## Problem Statement

quies currently only generates noise (brown, pink, white, etc). Real focus sessions benefit from layering noise with music or ambient soundscapes. Users currently need a separate player (browser tab, Spotify) running alongside quies. Combining everything into one tool with per-layer volume control is the core value proposition.

## Proposed Solution

**Download-to-file, then decode.** This is the simplest architecture that avoids the main technical obstacle (rodio's `Decoder` requires `Read + Seek`, which HTTP streams don't provide). Downloaded files on disk provide `Seek` for free.

Two source types:
1. **YouTube URLs** — yt-dlp subprocess downloads audio to temp file → rodio Decoder
2. **Direct HTTP audio URLs** — curl subprocess downloads to temp file → rodio Decoder

Both use subprocesses. **Zero new compile dependencies.** yt-dlp and curl are runtime dependencies only.

Live/infinite streams (e.g., lofi radio) deferred to Phase 3 — they require the streaming architecture (symphonia direct decode + rtrb ring buffer).

### Architecture

```
YouTube URL                          Direct HTTP URL
     |                                    |
     v                                    v
[yt-dlp subprocess]                  [curl subprocess]
  -f "bestaudio[ext=m4a]"             --max-filesize 200M
  --max-filesize 200m                    |
     |                                    v
     v                           /tmp/quies-<hash>.tmp
/tmp/quies-<hash>.tmp                    |
     |                                    v
     v                           rename → /tmp/quies-<hash>.mp3
rename → /tmp/quies-<hash>.m4a          |
     |                                    |
     └──────────────┬─────────────────────┘
                    v
     BufReader::with_capacity(256 * 1024, File)
                    |
                    v
        rodio::Decoder::new_looped(reader)
                    |
                    v
             VolumeSource<Decoder<...>>   ← reuse existing wrapper
                    |
                    v
             MixerDeviceSink.mixer().add()
```

**Key insight:** `VolumeSource<S>` is generic — it already works with any `Source<Item = f32>`. No changes to the volume/mute system needed.

## Technical Considerations

### rodio Decoder + File (no streaming complexity)

rodio's `Decoder::new()` requires `R: Read + Seek + Send + Sync + 'static`. `BufReader<File>` satisfies all bounds. The Mixer's `UniformSourceIterator` handles sample rate conversion (48kHz stream → mixer's native rate) and channel conversion (stereo → mixer channels) automatically.

```rust
use std::io::BufReader;
use std::fs::File;
use rodio::Decoder;

// CRITICAL: 256KB buffer to prevent audio glitches from disk I/O stalls.
// Default 8KB = ~45ms at 44.1kHz stereo f32 — a single disk hiccup causes audible dropout.
let file = BufReader::with_capacity(256 * 1024, File::open("/tmp/quies-abc123.m4a")?);
let source = Decoder::new_looped(file)?;
// source implements Source<Item = f32> + Send + 'static ✓
mixer.add(VolumeSource::new(source, vol, active));
```

### Looping

Noise generators are infinite. Audio files are finite. For ambient use, files should loop. rodio provides `Decoder::new_looped()` which loops the source seamlessly. Use this by default for audio layers.

### yt-dlp integration

**Pre-flight check** before attempting download:

```rust
fn check_ytdlp() -> anyhow::Result<()> {
    match Command::new("yt-dlp").arg("--version").output() {
        Ok(output) if output.status.success() => Ok(()),
        _ => anyhow::bail!("yt-dlp not found — install with: brew install yt-dlp"),
    }
}
```

**Download with correct format string:**

```rust
let output_tmp = temp_dir().join(format!("quies-{}.tmp", hash(&url)));
let output_path = temp_dir().join(format!("quies-{}.m4a", hash(&url)));

let output = Command::new("yt-dlp")
    .args([
        "-f", "bestaudio[ext=m4a]/bestaudio",  // m4a preferred, fallback to best
        "--max-filesize", "200m",
        "--no-playlist",                         // single video only
        "--no-progress",                         // clean stderr
        "-o", output_tmp.to_str().unwrap(),
        &url,
    ])
    .stdout(Stdio::null())
    .stderr(Stdio::piped())
    .output()?;

if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    anyhow::bail!("yt-dlp failed: {}", stderr.lines().last().unwrap_or("unknown error"));
}

// Atomic rename: only expose file once fully written
std::fs::rename(&output_tmp, &output_path)?;
```

**yt-dlp exit codes:** 0 = success, 1 = generic error, 2 = user abort. Surface last line of stderr on failure — it contains the human-readable error.

**Format rationale:** Default `bestaudio` returns opus-in-webm on most YouTube videos. rodio's default features don't include `symphonia-mkv` for webm container demuxing. AAC in MP4 container (`ext=m4a`) decodes with rodio's built-in MP4/AAC support. The fallback `/bestaudio` handles edge cases where m4a isn't available.

For the daemon, the download runs on a background `std::thread` so the main event loop stays responsive. Layer status shows `[downloading...]` until the file is ready, then transitions to `[playing]`.

### Direct HTTP download (curl subprocess)

**Zero new dependencies.** curl is pre-installed on macOS and virtually all Linux distros:

```rust
let output_tmp = temp_dir().join(format!("quies-{}.tmp", hash(&url)));
let output_path = temp_dir().join(format!("quies-{}.mp3", hash(&url)));

let output = Command::new("curl")
    .args([
        "-fSL",                              // fail on HTTP errors, show errors, follow redirects
        "--max-filesize", "209715200",       // 200MB in bytes
        "-o", output_tmp.to_str().unwrap(),
        &url,
    ])
    .stdout(Stdio::null())
    .stderr(Stdio::piped())
    .output()?;

if !output.status.success() {
    anyhow::bail!("download failed: {}", String::from_utf8_lossy(&output.stderr));
}

std::fs::rename(&output_tmp, &output_path)?;
```

**Why curl over ureq:** ureq adds ~15 transitive compile deps. curl adds zero. For a "download file to disk" operation, a subprocess is perfectly adequate. Portability concern is negligible — curl ships with macOS and every major Linux distro.

### Download management

**Concurrent download cap:** Max 3 simultaneous downloads. Use a `Arc<Semaphore>` (simple counting semaphore with `Mutex<usize>`) to gate download thread spawning.

**Download timeout:** yt-dlp has built-in timeout. For curl, add `--max-time 600` (10 minutes — generous for a 200MB file on any reasonable connection).

### File size guard

Cap downloads at 200MB (enough for 2+ hours of audio). yt-dlp: `--max-filesize 200m`. curl: `--max-filesize 209715200`.

### Temp file lifecycle

Downloaded audio goes to `$TMPDIR/quies-<url-hash>.<ext>`. Files are:
- Written as `.tmp`, atomically renamed on completion (prevents partial-file decode attempts)
- Cached across daemon restarts (cache hit = instant layer add via path check)
- Cleaned by OS on reboot (`$TMPDIR` semantics)
- Cleaned on `quies stop` (iterate known paths, best-effort delete)

No `quies clean` command for now — `$TMPDIR` handles cleanup naturally.

### Codec support

rodio's default features include MP3, FLAC, WAV, Vorbis, AAC, MP4. By forcing yt-dlp to output m4a (AAC in MP4 container), we stay within rodio's default codec support. No `symphonia-all` feature flag needed.

Direct HTTP URLs may serve any format — MP3, FLAC, WAV will work. Exotic formats will produce a decode error surfaced to the user.

### Error propagation

| Scenario | Behavior |
|----------|----------|
| yt-dlp not installed | `anyhow::bail!("yt-dlp not found — install with: brew install yt-dlp")` |
| curl not installed | `anyhow::bail!("curl not found")` (extremely unlikely) |
| Download fails (network) | Layer status → `[error: download failed]`, user retries with `quies add` again |
| Decode fails (format) | Layer status → `[error: unsupported format]`, layer silenced |
| File too large | yt-dlp/curl enforce 200MB limit, subprocess exits non-zero |
| yt-dlp URL not supported | Surface yt-dlp's stderr last line to user |
| Download in progress | Layer shows `[downloading...]` in status output |

### Layer status

```rust
enum LayerStatus {
    Downloading,
    Playing,
    Error(String),
}

// In Layer struct — only touched on state transitions, not per-sample
status: Arc<Mutex<LayerStatus>>,
```

`Arc<Mutex<LayerStatus>>` is the right choice here. The mutex is only locked when:
1. Download thread updates status (once per download)
2. `status` command reads it (user-initiated, infrequent)

It's never touched in the audio hot path (per-sample iteration). The alternative (`AtomicU8` with encoded states) would require unsafe string handling for the error message and gains nothing.

## Implementation Plan

### Phase 2a: File-based audio layers (core)

**`src/audio.rs` changes (~60 lines new):**

1. Add optional fields to `Layer` struct:
   ```rust
   pub struct Layer {
       pub name: String,
       volume: Arc<AtomicU32>,
       active: Arc<AtomicBool>,
       // New fields for audio layers:
       pub url: Option<String>,
       pub path: Option<PathBuf>,
       pub status: Arc<Mutex<LayerStatus>>,
   }
   ```
2. New method `AudioEngine::add_audio_layer(name, path)` that:
   - Opens file with `BufReader::with_capacity(256 * 1024, file)`
   - Creates `Decoder::new_looped()` for seamless looping
   - Wraps in `VolumeSource`, adds to mixer
3. `LayerStatus` enum: `Downloading`, `Playing`, `Error(String)`.

**`src/main.rs` changes (~80 lines new):**

1. New daemon command:
   - `add <name> <url>` — detect URL type, spawn download thread, add layer when ready
2. Download orchestration:
   - Spawn `std::thread` for download
   - URL detection: `youtube.com` or `youtu.be` → yt-dlp path, everything else → curl path
   - Pre-flight: check yt-dlp/curl exists before spawning thread
   - On completion: atomic rename `.tmp` → final, update `LayerStatus`, call `add_audio_layer`
   - On failure: update `LayerStatus` to `Error(message)`
3. Cache check: if final path already exists, skip download, add layer immediately.

**`src/ui.rs` changes (~5 lines):**

1. Show download status in `quies status` output: `[downloading...]`, `[error: ...]`, or volume bar.

**`Cargo.toml` changes:**

None. Zero new dependencies. Both yt-dlp and curl are runtime subprocess dependencies.

### Phase 2b: CLI ergonomics

1. CLI shorthand: `quies add lofi https://youtu.be/0NCrui_dYJY` sends `add` command to running daemon.

2. Presets with URLs (hardcoded `&'static str`):
   ```rust
   ("lofi", &[
       PresetLayer::noise("Brown Noise", NoiseType::Brown, 0.3),
       PresetLayer::url("Dark Ambient", "https://youtu.be/0NCrui_dYJY", 0.5),
   ]),
   ```

## Acceptance Criteria

- [ ] `quies start` + `quies add lofi https://youtu.be/0NCrui_dYJY` downloads and plays the audio as a mixer layer
- [ ] `quies add rain https://example.com/rain.mp3` works for direct HTTP audio
- [ ] `quies vol lofi 0.3` adjusts the audio layer volume
- [ ] `quies mute lofi` mutes the audio layer
- [ ] `quies status` shows download state (`[downloading...]`, `[error]`) and volume
- [ ] Audio loops seamlessly when the file ends
- [ ] Files >200MB are rejected (yt-dlp/curl enforce limit)
- [ ] Missing yt-dlp produces a clear error message with install instructions
- [ ] Downloaded files are cached (re-adding same URL is instant)
- [ ] `quies stop` cleans up temp files (best-effort)
- [ ] Zero new compile dependencies (`Cargo.toml` unchanged)
- [ ] `cargo clippy` clean

## Dependencies & Risks

| Risk | Mitigation |
|------|-----------|
| yt-dlp not installed on user's system | Pre-flight check with clear error message + install instructions |
| YouTube URL format changes | yt-dlp handles this; just keep yt-dlp updated |
| Large file downloads consume disk | 200MB cap enforced by yt-dlp/curl flags |
| rodio can't decode yt-dlp output | Force m4a format (`-f "bestaudio[ext=m4a]/bestaudio"`) — AAC/MP4 is in rodio defaults |
| Download blocks daemon responsiveness | Background thread for downloads, non-blocking status polling |
| Partial file decoded mid-download | Atomic rename: write to `.tmp`, rename to final path on completion |
| Too many concurrent downloads | Counting semaphore caps at 3 simultaneous downloads |
| curl not available | Pre-installed on macOS + all major Linux distros. Pre-flight check as safety net |

## Future Considerations (Phase 3+)

- **Live streaming**: symphonia direct decode + rtrb ring buffer for infinite streams (lofi radio, etc.)
- **TOML config file**: User-defined presets with URLs
- **Add/remove layers in TUI**: Interactive URL entry
- **Fade transitions**: Crossfade when switching presets
- **`stream-download-rs`**: For instant-start playback (download and play simultaneously). Requires tokio.
- **`remove` command**: Can't actually remove from rodio mixer, only silence. Revisit if rodio adds mixer removal API.
- **`clean` command**: Manual cache cleanup. Low priority since `$TMPDIR` is cleaned by OS.

## Sources & References

- [rodio issue #580](https://github.com/RustAudio/rodio/issues/580) — Decoder requires Seek, stream-download-rs recommended
- [rodio issue #439](https://github.com/RustAudio/rodio/issues/439) — Forked MP3 decoder approach (not needed with file-based approach)
- [stream-download-rs](https://github.com/aschey/stream-download-rs) — For Phase 3 live streaming
- [symphonia ReadOnlySource](https://docs.rs/symphonia-core/0.5.5/symphonia_core/io/struct.ReadOnlySource.html) — For Phase 3 non-seekable streams
- [rtrb ring buffer](https://github.com/mgeier/rtrb) — For Phase 3 real-time audio buffering
- [rodio Decoder docs](https://docs.rs/rodio/0.22.1/rodio/decoder/struct.Decoder.html) — All constructors require Read + Seek
- [stall detection pattern](~/docs/solutions/runtime-errors/chrome-download-stall-recovery.md) — Applicable to download monitoring
