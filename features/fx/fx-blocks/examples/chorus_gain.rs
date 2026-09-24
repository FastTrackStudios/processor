//! How loud the chorus block plays against its input, per engine and mix —
//! a chorus should sit near unity (a pedal's level knob aside).
//! `cargo run --release -p fx-blocks --example chorus_gain`

use daw::plugin::{PluginEvents, PluginInstance};
use fx_blocks::NativeMod;

const SR: f64 = 48_000.0;
const BLOCK: usize = 512;

fn main() {
    // A guitar-ish tone: a few harmonics of 196 Hz.
    let tone: Vec<f32> = (0..SR as usize * 2)
        .map(|i| {
            let t = i as f32 / SR as f32;
            (1..6).map(|h| (std::f32::consts::TAU * 196.0 * h as f32 * t).sin() * 0.2 / h as f32).sum()
        })
        .collect();
    let rms = |x: &[f32]| (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt();
    for engine in 0..5 {
        for mix in [0.5, 1.0] {
            let mut m = NativeMod::chorus(SR);
            for (n, v) in [("engine", f64::from(engine)), ("mix", mix), ("depth", 0.35), ("rate", 0.8)] {
                m.set_named(n, v);
            }
            m.prepare(SR, BLOCK as u32).unwrap();
            let mut out = Vec::new();
            for c in tone.chunks(BLOCK) {
                let (mut l, mut r) = (vec![0.0f32; c.len()], vec![0.0f32; c.len()]);
                m.process_block(c, c, &mut l, &mut r, &PluginEvents::EMPTY).unwrap();
                out.extend(l.iter().zip(&r).map(|(a, b)| (a + b) * 0.5));
            }
            let half = tone.len() / 2;
            let db = 20.0 * (rms(&out[half..]) / rms(&tone[half..])).log10();
            println!("engine {engine} mix {mix}: {db:+.1} dB");
        }
    }
}
