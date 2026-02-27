use std::num::NonZero;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use std::sync::Arc;
use std::time::Duration;

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

pub struct Layer {
    pub name: String,
    pub volume: Arc<AtomicU32>,
    pub active: Arc<AtomicBool>,
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
        });
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
