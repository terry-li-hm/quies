use std::io::BufReader;
use std::num::NonZero;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::Decoder;
use rodio::source::noise::{Blue, Brownian, Pink, Red, Violet, WhiteUniform};
use rodio::{DeviceSinkBuilder, MixerDeviceSink, Source};

#[derive(Clone, Copy)]
pub enum NoiseType {
    White,
    Pink,
    Brown,
    Red,
    Blue,
    Violet,
}

pub struct PresetLayer {
    pub name: &'static str,
    pub noise_type: NoiseType,
    pub volume: f32,
}

pub const PRESETS: &[(&str, &[PresetLayer])] = &[
    (
        "default",
        &[PresetLayer {
            name: "Brown Noise",
            noise_type: NoiseType::Brown,
            volume: 0.5,
        }],
    ),
    (
        "focus",
        &[
            PresetLayer {
                name: "Brown Noise",
                noise_type: NoiseType::Brown,
                volume: 0.6,
            },
            PresetLayer {
                name: "Pink Noise",
                noise_type: NoiseType::Pink,
                volume: 0.2,
            },
        ],
    ),
    (
        "deep",
        &[
            PresetLayer {
                name: "Brown Noise",
                noise_type: NoiseType::Brown,
                volume: 0.8,
            },
            PresetLayer {
                name: "Pink Noise",
                noise_type: NoiseType::Pink,
                volume: 0.1,
            },
        ],
    ),
];

pub enum LayerStatus {
    Playing,
    Downloading,
    Error(String),
}

pub struct Layer {
    pub name: String,
    pub volume: Arc<AtomicU32>,
    pub active: Arc<AtomicBool>,
    pub url: Option<String>,
    pub path: Option<PathBuf>,
    pub status: Arc<Mutex<LayerStatus>>,
}

pub struct AudioEngine {
    stream: MixerDeviceSink,
    pub layers: Vec<Layer>,
}

fn get_vol(v: &AtomicU32) -> f32 {
    f32::from_bits(v.load(Relaxed))
}

fn set_vol(v: &AtomicU32, val: f32) {
    v.store(val.to_bits(), Relaxed);
}

impl AudioEngine {
    pub fn new() -> anyhow::Result<Self> {
        let stream = DeviceSinkBuilder::open_default_sink()?;
        Ok(Self {
            stream,
            layers: Vec::new(),
        })
    }

    pub fn add_layer(&mut self, name: &str, noise_type: NoiseType, volume: f32) {
        let vol = Arc::new(AtomicU32::new(volume.to_bits()));
        let active = Arc::new(AtomicBool::new(true));
        let sr = NonZero::new(44100u32).unwrap();

        let mixer = self.stream.mixer();
        match noise_type {
            NoiseType::White => mixer.add(VolumeSource::new(WhiteUniform::new(sr), vol.clone(), active.clone())),
            NoiseType::Pink => mixer.add(VolumeSource::new(Pink::new(sr), vol.clone(), active.clone())),
            NoiseType::Brown => mixer.add(VolumeSource::new(Brownian::new(sr), vol.clone(), active.clone())),
            NoiseType::Red => mixer.add(VolumeSource::new(Red::new(sr), vol.clone(), active.clone())),
            NoiseType::Blue => mixer.add(VolumeSource::new(Blue::new(sr), vol.clone(), active.clone())),
            NoiseType::Violet => mixer.add(VolumeSource::new(Violet::new(sr), vol.clone(), active.clone())),
        }

        self.layers.push(Layer {
            name: name.to_string(),
            volume: vol,
            active,
            url: None,
            path: None,
            status: Arc::new(Mutex::new(LayerStatus::Playing)),
        });
    }

    pub fn add_audio_layer(&mut self, name: &str, path: PathBuf, url: &str, volume: f32) -> anyhow::Result<()> {
        let vol = Arc::new(AtomicU32::new(volume.to_bits()));
        let active = Arc::new(AtomicBool::new(true));

        // 256KB buffer: default 8KB = ~45ms at 44.1kHz stereo f32 — too small.
        let file = BufReader::with_capacity(256 * 1024, std::fs::File::open(&path)?);
        let source = Decoder::new_looped(file)?;
        self.stream.mixer().add(VolumeSource::new(source, vol.clone(), active.clone()));

        self.layers.push(Layer {
            name: name.to_string(),
            volume: vol,
            active,
            url: Some(url.to_string()),
            path: Some(path),
            status: Arc::new(Mutex::new(LayerStatus::Playing)),
        });
        Ok(())
    }

    /// Add a placeholder layer that's downloading. Returns its index and status handle.
    pub fn add_pending_layer(&mut self, name: &str, url: &str, volume: f32) -> (usize, Arc<AtomicU32>, Arc<AtomicBool>, Arc<Mutex<LayerStatus>>) {
        let vol = Arc::new(AtomicU32::new(volume.to_bits()));
        let active = Arc::new(AtomicBool::new(true));
        let status = Arc::new(Mutex::new(LayerStatus::Downloading));

        self.layers.push(Layer {
            name: name.to_string(),
            volume: vol.clone(),
            active: active.clone(),
            url: Some(url.to_string()),
            path: None,
            status: status.clone(),
        });
        let idx = self.layers.len() - 1;
        (idx, vol, active, status)
    }

    /// Activate a pending layer after download completes.
    pub fn activate_audio_layer(&mut self, idx: usize, path: PathBuf) -> anyhow::Result<()> {
        let file = BufReader::with_capacity(256 * 1024, std::fs::File::open(&path)?);
        let source = Decoder::new_looped(file)?;

        let layer = &mut self.layers[idx];
        layer.path = Some(path);
        self.stream.mixer().add(VolumeSource::new(source, layer.volume.clone(), layer.active.clone()));
        *layer.status.lock().unwrap() = LayerStatus::Playing;
        Ok(())
    }

    pub fn get_volume(&self, idx: usize) -> f32 {
        get_vol(&self.layers[idx].volume)
    }

    pub fn set_volume(&self, idx: usize, vol: f32) {
        set_vol(&self.layers[idx].volume, vol.clamp(0.0, 1.0));
    }

    pub fn volume_up(&self, idx: usize) {
        let cur = self.get_volume(idx);
        self.set_volume(idx, (cur + 0.05).min(1.0));
    }

    pub fn volume_down(&self, idx: usize) {
        let cur = self.get_volume(idx);
        self.set_volume(idx, (cur - 0.05).max(0.0));
    }

    pub fn is_active(&self, idx: usize) -> bool {
        self.layers[idx].active.load(Relaxed)
    }

    pub fn toggle_mute(&self, idx: usize) {
        let active = &self.layers[idx].active;
        active.store(!active.load(Relaxed), Relaxed);
    }

    pub fn find_layer(&self, name: &str) -> Option<usize> {
        let name_lower = name.to_lowercase();
        self.layers
            .iter()
            .position(|l| l.name.to_lowercase().contains(&name_lower))
    }

    pub fn status(&self) -> String {
        if self.layers.is_empty() {
            return "no layers".to_string();
        }
        self.layers
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let kind = if l.url.is_some() { "♪" } else { "~" };
                let status = l.status.lock().unwrap();
                match &*status {
                    LayerStatus::Downloading => format!("  {kind} {} [downloading...]", l.name),
                    LayerStatus::Error(e) => format!("  {kind} {} [error: {e}]", l.name),
                    LayerStatus::Playing => {
                        let vol = (self.get_volume(i) * 100.0).round() as u8;
                        let state = if self.is_active(i) { "" } else { " [off]" };
                        format!("  {kind} {} {}%{}", l.name, vol, state)
                    }
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

struct VolumeSource<S> {
    inner: S,
    volume: Arc<AtomicU32>,
    active: Arc<AtomicBool>,
}

impl<S> VolumeSource<S> {
    fn new(inner: S, volume: Arc<AtomicU32>, active: Arc<AtomicBool>) -> Self {
        Self { inner, volume, active }
    }
}

impl<S: Source<Item = f32>> Iterator for VolumeSource<S> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let sample = self.inner.next()?;
        if !self.active.load(Relaxed) {
            return Some(0.0);
        }
        Some(sample * get_vol(&self.volume))
    }
}

impl<S: Source<Item = f32>> Source for VolumeSource<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }

    fn channels(&self) -> NonZero<u16> {
        self.inner.channels()
    }

    fn sample_rate(&self) -> NonZero<u32> {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}
