//! `ChorusChain` — multi-engine stereo chorus processor.
//!
//! Supports 4 engine types (Cubic, BBD, Tape, Orbit) with
//! 1–4 voices per channel. Implements the Processor trait.

use audiocore_dsp::{AudioConfig, Processor};

use crate::engine::{ChorusEngine, EffectType, EngineType, create_voices};

/// Maximum number of voices per channel.
const MAX_VOICES: usize = 4;

/// Complete stereo chorus/flanger/vibrato processor.
pub struct ChorusChain {
    voices_l: Vec<Box<dyn ChorusEngine>>,
    voices_r: Vec<Box<dyn ChorusEngine>>,

    /// Number of active voices per channel (1–4).
    pub num_voices: usize,
    /// LFO rate in Hz.
    pub rate_hz: f64,
    /// Modulation depth (0..1).
    pub depth: f64,
    /// Feedback amount (0..1). Mainly for flanger.
    pub feedback: f64,
    /// Color/tone parameter (0..1). Engine-specific meaning.
    pub color: f64,
    /// Engine type.
    pub engine: EngineType,
    /// Effect type (chorus/flanger/vibrato).
    pub effect_type: EffectType,
    /// Dry/wet mix (0..1). For vibrato, set to 1.0 (wet only).
    pub mix: f64,
    /// Stereo width (0..1). 0 = mono, 1 = full stereo spread.
    pub width: f64,
}

impl ChorusChain {
    #[must_use]
    pub fn new() -> Self {
        let engine = EngineType::Cubic;
        Self {
            voices_l: create_voices(engine, MAX_VOICES),
            voices_r: create_voices_stereo(engine, MAX_VOICES),
            num_voices: 2,
            rate_hz: 1.0,
            depth: 0.5,
            feedback: 0.0,
            color: 0.5,
            engine,
            effect_type: EffectType::Chorus,
            mix: 0.5,
            width: 1.0,
        }
    }

    /// Switch the chorus engine. Recreates all voices.
    pub fn set_engine(&mut self, engine: EngineType) {
        if self.engine != engine {
            self.engine = engine;
            self.voices_l = create_voices(engine, MAX_VOICES);
            self.voices_r = create_voices_stereo(engine, MAX_VOICES);
        }
    }
}

/// Create right-channel voices with stereo phase offset.
fn create_voices_stereo(engine: EngineType, count: usize) -> Vec<Box<dyn ChorusEngine>> {
    use crate::engine::{
        BbdVoice, ChorusEngine, CubicVoice, EngineType, JunoVoice, OrbitVoice, TapeVoice,
    };
    (0..count)
        .map(|i| {
            let offset = i as f64 / count as f64 + 0.25; // +90° for stereo
            let voice: Box<dyn ChorusEngine> = match engine {
                EngineType::Cubic => Box::new(CubicVoice::new(offset)),
                EngineType::Bbd => Box::new(BbdVoice::new(offset)),
                EngineType::Tape => Box::new(TapeVoice::new(offset)),
                EngineType::Orbit => Box::new(OrbitVoice::new(offset)),
                EngineType::Juno => Box::new(JunoVoice::new(offset)),
            };
            voice
        })
        .collect()
}

/// Each engine's wet level made up to unity (linear gain) — measured with
/// `fx-blocks/examples/chorus_gain.rs` on pink noise; see its output for the
/// residual per mix.
const fn engine_makeup(engine: EngineType) -> f64 {
    // 10^(−dB/20) of each engine's fully-wet level, less 0.3 dB: at a
    // mid mix the dry and the (partly correlated) wet still add a little.
    match engine {
        EngineType::Cubic => 0.892, // +0.7 dB hot
        EngineType::Bbd => 0.955,   // +0.1 dB
        EngineType::Tape => 0.610,  // +4.0 dB hot
        EngineType::Orbit => 0.861, // +1.0 dB hot
        EngineType::Juno => 1.047,  // −0.7 dB
    }
}

impl Default for ChorusChain {
    fn default() -> Self {
        Self::new()
    }
}

impl Processor for ChorusChain {
    fn reset(&mut self) {
        for v in &mut self.voices_l {
            v.reset();
        }
        for v in &mut self.voices_r {
            v.reset();
        }
    }

    fn update(&mut self, config: AudioConfig) {
        for v in &mut self.voices_l {
            v.update(config.sample_rate);
        }
        for v in &mut self.voices_r {
            v.update(config.sample_rate);
        }
    }

    fn process(&mut self, left: &mut [f64], right: &mut [f64]) {
        let n = self.num_voices.clamp(1, MAX_VOICES);
        // The voices are modulated apart, so they add in power: normalised
        // by 1/√n they sum to one voice's level (an average, 1/n, lost ~3 dB
        // per doubling of voices), and each engine's own gain is made up so
        // the wet sits at the dry's level.
        let wet_gain = engine_makeup(self.engine) / (n as f64).sqrt();
        // Equal-power mix: a decorrelated wet and the dry at 50/50 keep the
        // level (a linear crossfade dipped ~3 dB there — the drop a player
        // heard switching the chorus on).
        // A flanger's wet is the dry a few ms late — correlated, so it sums
        // in amplitude and the linear crossfade is the level-neutral one
        // (equal power put it ~2 dB up at a mid mix). The BBD's filtering
        // decorrelates its wet enough that it sits with the chorus law
        // (linear left it ~3 dB down) — both measured, see `chorus_gain`.
        let mix = self.mix.clamp(0.0, 1.0);
        let linear = self.effect_type == EffectType::Flanger && self.engine != EngineType::Bbd;
        let (dry_gain, mix_gain) = if linear {
            (1.0 - mix, mix)
        } else {
            let theta = mix * std::f64::consts::FRAC_PI_2;
            (theta.cos(), theta.sin())
        };

        for i in 0..left.len().min(right.len()) {
            let dry_l = left[i];
            let dry_r = right[i];

            let mut wet_l: f64 = 0.0;
            let mut wet_r: f64 = 0.0;

            for v in 0..n {
                wet_l += self.voices_l[v].tick(
                    left[i],
                    self.rate_hz,
                    self.depth,
                    self.feedback,
                    self.color,
                    self.effect_type,
                );
                wet_r += self.voices_r[v].tick(
                    right[i],
                    self.rate_hz,
                    self.depth,
                    self.feedback,
                    self.color,
                    self.effect_type,
                );
            }

            wet_l *= wet_gain;
            wet_r *= wet_gain;

            // Stereo width
            let mono_wet = (wet_l + wet_r) * 0.5;
            wet_l = (wet_l - mono_wet).mul_add(self.width, mono_wet);
            wet_r = (wet_r - mono_wet).mul_add(self.width, mono_wet);

            // Vibrato: wet only
            if self.effect_type == EffectType::Vibrato {
                left[i] = wet_l;
                right[i] = wet_r;
            } else {
                left[i] = dry_l.mul_add(dry_gain, wet_l * mix_gain);
                right[i] = dry_r.mul_add(dry_gain, wet_r * mix_gain);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Engaged at any mix, every engine plays at the level it is fed —
    /// switching a chorus on must not drop (or jump) the guitar.
    #[test]
    fn every_engine_sits_at_unity_at_any_mix() {
        let mut seed = 11u32;
        let (mut b0, mut b1) = (0.0f64, 0.0f64);
        let input: Vec<f64> = (0..(SR as usize) * 3)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let w = f64::from(seed >> 8) / f64::from(1u32 << 24) - 0.5;
                b0 = 0.997 * b0 + 0.029 * w;
                b1 = 0.95 * b1 + 0.048 * w;
                (b0 + b1 + 0.02 * w) * 2.0
            })
            .collect();
        let rms = |x: &[f64]| (x.iter().map(|s| s * s).sum::<f64>() / x.len() as f64).sqrt();
        for engine in [EngineType::Cubic, EngineType::Bbd, EngineType::Tape, EngineType::Orbit, EngineType::Juno] {
            for mix in [0.25, 0.5, 0.75, 1.0] {
                let mut c = ChorusChain::new();
                c.set_engine(engine);
                c.mix = mix;
                c.depth = 0.35;
                c.rate_hz = 0.8;
                c.update(config());
                let (mut l, mut r) = (input.clone(), input.clone());
                for (bl, br) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
                    c.process(bl, br);
                }
                let half = input.len() / 2;
                let out: Vec<f64> = l[half..].iter().zip(&r[half..]).map(|(a, b)| (a + b) * 0.5).collect();
                let db = 20.0 * (rms(&out) / rms(&input[half..])).log10();
                assert!(db.abs() < 1.5, "{engine:?} at mix {mix}: {db:+.1} dB");
            }
        }
    }
    use std::f64::consts::PI;

    const SR: f64 = 48000.0;

    fn config() -> AudioConfig {
        AudioConfig {
            sample_rate: SR,
            max_buffer_size: 512,
        }
    }

    #[test]
    fn silence_in_silence_out() {
        let mut c = ChorusChain::new();
        c.update(config());

        let mut l = vec![0.0; 4800];
        let mut r = vec![0.0; 4800];
        c.process(&mut l, &mut r);

        for (i, &s) in l.iter().enumerate() {
            assert!(s.abs() < 1e-10, "Non-zero at {i}: {s}");
        }
    }

    #[test]
    fn zero_mix_passes_through() {
        let mut c = ChorusChain::new();
        c.mix = 0.0;
        c.update(config());

        let input: Vec<f64> = (0..4800)
            .map(|i| (2.0 * PI * 440.0 * f64::from(i) / SR).sin() * 0.5)
            .collect();
        let mut l = input.clone();
        let mut r = input.clone();

        c.process(&mut l, &mut r);

        for (i, (&out, &inp)) in l.iter().zip(input.iter()).enumerate() {
            assert!(
                (out - inp).abs() < 1e-10,
                "Zero mix should pass through at {i}: {out} vs {inp}"
            );
        }
    }

    #[test]
    fn all_engines_no_nan() {
        for engine in &[
            EngineType::Cubic,
            EngineType::Bbd,
            EngineType::Tape,
            EngineType::Orbit,
        ] {
            let mut c = ChorusChain::new();
            c.set_engine(*engine);
            c.depth = 1.0;
            c.rate_hz = 2.0;
            c.feedback = 0.7;
            c.num_voices = 2;
            c.update(config());

            let mut l: Vec<f64> = (0..48000)
                .map(|i| (2.0 * PI * 440.0 * f64::from(i) / SR).sin() * 0.5)
                .collect();
            let mut r = l.clone();

            c.process(&mut l, &mut r);

            for (i, &s) in l.iter().enumerate() {
                assert!(s.is_finite(), "NaN in {engine:?} at {i}");
            }
        }
    }

    #[test]
    fn all_engines_all_effects_no_nan() {
        for engine in &[
            EngineType::Cubic,
            EngineType::Bbd,
            EngineType::Tape,
            EngineType::Orbit,
        ] {
            for effect in &[EffectType::Chorus, EffectType::Flanger, EffectType::Vibrato] {
                let mut c = ChorusChain::new();
                c.set_engine(*engine);
                c.effect_type = *effect;
                c.depth = 1.0;
                c.rate_hz = 3.0;
                c.feedback = 0.8;
                c.num_voices = 4;
                c.update(config());

                let mut l: Vec<f64> = (0..24000)
                    .map(|i| (2.0 * PI * 440.0 * f64::from(i) / SR).sin() * 0.5)
                    .collect();
                let mut r = l.clone();

                c.process(&mut l, &mut r);

                for (i, &s) in l.iter().enumerate() {
                    assert!(s.is_finite(), "NaN in {engine:?}/{effect:?} at {i}");
                }
            }
        }
    }

    #[test]
    fn engine_switch_works() {
        let mut c = ChorusChain::new();
        c.update(config());

        // Process some with cubic
        let mut l = vec![0.5; 480];
        let mut r = vec![0.5; 480];
        c.process(&mut l, &mut r);

        // Switch to BBD
        c.set_engine(EngineType::Bbd);
        c.update(config());

        let mut l = vec![0.5; 480];
        let mut r = vec![0.5; 480];
        c.process(&mut l, &mut r);

        // Should not crash
    }

    #[test]
    fn different_engines_produce_different_output() {
        let input: Vec<f64> = (0..9600)
            .map(|i| (2.0 * PI * 440.0 * f64::from(i) / SR).sin() * 0.5)
            .collect();

        let mut outputs = Vec::new();

        for engine in &[
            EngineType::Cubic,
            EngineType::Bbd,
            EngineType::Tape,
            EngineType::Orbit,
        ] {
            let mut c = ChorusChain::new();
            c.set_engine(*engine);
            c.depth = 0.5;
            c.rate_hz = 1.0;
            c.mix = 1.0;
            c.num_voices = 1;
            c.update(config());

            let mut l = input.clone();
            let mut r = input.clone();
            c.process(&mut l, &mut r);
            outputs.push(l);
        }

        // Each pair should differ
        for i in 0..outputs.len() {
            for j in (i + 1)..outputs.len() {
                let diff: f64 = outputs[i]
                    .iter()
                    .zip(outputs[j].iter())
                    .map(|(a, b)| (a - b).abs())
                    .sum::<f64>()
                    / 9600.0;
                assert!(diff > 0.001, "Engines {i} and {j} should differ: {diff}");
            }
        }
    }
}
