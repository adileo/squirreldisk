//! Tiny procedural sound effects – no audio files shipped.

use rodio::buffer::SamplesBuffer;
use rodio::Source;
use std::f32::consts::TAU;
use std::num::NonZero;

const SR: u32 = 44_100;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Sfx {
    /// Something dropped into the collector.
    Plop,
    /// One item deleted.
    Crunch,
    /// All deletions done.
    Success,
    /// Zoom in / out.
    Blip,
    BlipDown,
    Error,
    /// Drag started.
    Pick,
}

pub struct Sounds {
    sink: Option<rodio::MixerDeviceSink>,
    bank: Vec<(Sfx, Vec<f32>)>,
    last: std::collections::HashMap<Sfx, std::time::Instant>,
}

struct Rng(u32);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

fn samples(secs: f32) -> usize {
    (secs * SR as f32) as usize
}

fn plop() -> Vec<f32> {
    let n = samples(0.18);
    let mut phase = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            let f = 180.0 + 700.0 * (-t * 28.0).exp();
            phase += TAU * f / SR as f32;
            let env = (1.0 - (-t * 400.0).exp()) * (-t * 22.0).exp();
            phase.sin() * env * 0.8
        })
        .collect()
}

fn crunch() -> Vec<f32> {
    let n = samples(0.32);
    let mut rng = Rng(0x9E3779B9);
    let mut lp = 0.0f32;
    let mut phase = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            // crackly noise: sparse impulses through a sweeping low-pass
            let grain = if rng.next() > 0.55 { rng.next() } else { rng.next() * 0.2 };
            let cutoff = 0.05 + 0.5 * (-t * 9.0).exp();
            lp += (grain - lp) * cutoff;
            let noise = lp * (-t * 11.0).exp() * 1.6;
            // low thump
            phase += TAU * (90.0 + 60.0 * (-t * 30.0).exp()) / SR as f32;
            let thump = phase.sin() * (-t * 25.0).exp() * 0.7;
            (noise + thump).clamp(-1.0, 1.0) * 0.8
        })
        .collect()
}

fn success() -> Vec<f32> {
    let notes = [523.25f32, 659.25, 783.99, 1046.5];
    let step = 0.085;
    let n = samples(step * notes.len() as f32 + 0.5);
    let mut out = vec![0.0f32; n];
    for (k, f) in notes.iter().enumerate() {
        let start = samples(step * k as f32);
        for i in start..n {
            let t = (i - start) as f32 / SR as f32;
            let env = (1.0 - (-t * 300.0).exp()) * (-t * 6.0).exp();
            let s = (TAU * f * t).sin() * 0.6 + (TAU * f * 2.0 * t).sin() * 0.15 + (TAU * f * 3.0 * t).sin() * 0.06;
            out[i] += s * env * 0.35;
        }
    }
    out
}

fn blip(up: bool) -> Vec<f32> {
    let n = samples(0.09);
    let mut phase = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            let k = t / 0.09;
            let f = if up { 520.0 + 380.0 * k } else { 900.0 - 380.0 * k };
            phase += TAU * f / SR as f32;
            let env = (1.0 - (-t * 500.0).exp()) * (1.0 - k).powf(2.0);
            (phase.sin() * 0.7 + (phase * 2.0).sin() * 0.1) * env * 0.35
        })
        .collect()
}

fn error() -> Vec<f32> {
    let n = samples(0.35);
    (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            let f = if t < 0.15 { 220.0 } else { 175.0 };
            let lt = if t < 0.15 { t } else { t - 0.15 };
            let env = (1.0 - (-lt * 400.0).exp()) * (-lt * 14.0).exp();
            let x = (TAU * f * t).sin();
            (x.signum() * 0.25 + x * 0.5) * env * 0.4
        })
        .collect()
}

fn pick() -> Vec<f32> {
    let n = samples(0.06);
    let mut rng = Rng(12345);
    let mut lp = 0.0;
    (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            lp += (rng.next() - lp) * 0.35;
            let tone = (TAU * 1400.0 * t).sin() * 0.3;
            (lp * 0.8 + tone) * (-t * 70.0).exp() * 0.5
        })
        .collect()
}

impl Sounds {
    pub fn new() -> Self {
        let mut sink = rodio::DeviceSinkBuilder::open_default_sink().ok();
        if let Some(s) = sink.as_mut() {
            s.log_on_drop(false);
        }
        let bank = vec![
            (Sfx::Plop, plop()),
            (Sfx::Crunch, crunch()),
            (Sfx::Success, success()),
            (Sfx::Blip, blip(true)),
            (Sfx::BlipDown, blip(false)),
            (Sfx::Error, error()),
            (Sfx::Pick, pick()),
        ];
        Sounds { sink, bank, last: Default::default() }
    }

    pub fn play(&mut self, sfx: Sfx, volume: f32) {
        let Some(sink) = &self.sink else { return };
        // Avoid machine-gun repetition.
        let now = std::time::Instant::now();
        if let Some(t) = self.last.get(&sfx) {
            if now.duration_since(*t).as_millis() < 45 {
                return;
            }
        }
        self.last.insert(sfx, now);
        if let Some((_, data)) = self.bank.iter().find(|(s, _)| *s == sfx) {
            let src = SamplesBuffer::new(NonZero::new(1).unwrap(), NonZero::new(SR).unwrap(), data.clone()).amplify(volume);
            sink.mixer().add(src);
        }
    }
}
