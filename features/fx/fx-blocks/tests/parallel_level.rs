//! A delay's or reverb's `level` is a gain on the wet: with `mix` pinned at
//! 1, `level = 20·log10(m)` plays exactly what `mix = m` did.

use daw::plugin::{PluginEvents, PluginInstance};
use fx_blocks::{NativeDelay, NativeReverb};

const SR: f64 = 48_000.0;
const BLOCK: usize = 256;

fn render(fx: &mut dyn PluginInstance) -> Vec<f32> {
    fx.prepare(SR, BLOCK as u32).unwrap();
    let ev = PluginEvents::default();
    let mut out = Vec::new();
    for b in 0..80 {
        let mut input = vec![0.0f32; BLOCK];
        if b == 0 {
            input[0] = 1.0;
        }
        let (mut ol, mut or) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
        fx.process_block(&input, &input, &mut ol, &mut or, &ev).unwrap();
        out.extend_from_slice(&ol);
    }
    out
}

fn assert_same(a: &[f32], b: &[f32]) {
    let peak = a.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let diff = a.iter().zip(b).fold(0.0f32, |m, (x, y)| m.max((x - y).abs()));
    assert!(peak > 0.01, "the effect made sound");
    assert!(diff < 1e-4 * peak.max(1.0), "max diff {diff}");
}

#[test]
fn delay_level_matches_the_mix_it_replaces() {
    let mut by_mix = NativeDelay::new(SR);
    by_mix.set_named("time", 120.0);
    by_mix.set_named("mix", 0.25);
    let mut by_level = NativeDelay::new(SR);
    by_level.set_named("time", 120.0);
    by_level.set_named("mix", 1.0);
    by_level.set_named("level", 20.0 * 0.25f64.log10());
    assert_same(&render(&mut by_mix), &render(&mut by_level));
}

#[test]
fn reverb_level_matches_the_mix_it_replaces() {
    let mut by_mix = NativeReverb::new(SR);
    by_mix.set_named("mix", 0.2);
    let mut by_level = NativeReverb::new(SR);
    by_level.set_named("mix", 1.0);
    by_level.set_named("level", 20.0 * 0.2f64.log10());
    assert_same(&render(&mut by_mix), &render(&mut by_level));
}
