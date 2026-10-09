//! Bluebox behaves like the two-tube processor it was measured from: unity
//! at its defaults, one fixed output ceiling whatever the drive, closed
//! knobs mute, Density keeps the level, the parallel saturation adds to it,
//! a dry Mix of 0 — and it stays finite under anything at any rate.

use dsp_core::num::{count_to_f64, f64_to_index};
use saturate_dsp::bluebox::{Bluebox, BlueboxParams, SatFreq};

fn sine(hz: f64, amp: f64, secs: f64, sr: f64) -> Vec<f64> {
    (0..f64_to_index(secs * sr)).map(|i| amp * (2.0 * std::f64::consts::PI * hz * count_to_f64(i) / sr).sin()).collect()
}

fn run(p: BlueboxParams, x: &[f64], sr: f64) -> Vec<f64> {
    let mut fx = Bluebox::new(sr);
    fx.set(p);
    x.iter().map(|&s| fx.process(s, s).0).collect()
}

fn rms_db(x: &[f64]) -> f64 {
    10.0 * (x.iter().map(|v| v * v).sum::<f64>() / count_to_f64(x.len())).log10()
}

fn tail(x: &[f64]) -> &[f64] {
    x.get(x.len() / 2..).unwrap_or(x)
}

fn gain_db(p: BlueboxParams, amp: f64) -> f64 {
    let x = sine(1000.0, amp, 0.5, 48_000.0);
    rms_db(tail(&run(p, &x, 48_000.0))) - rms_db(tail(&x))
}

#[test]
fn defaults_are_unity_for_quiet_signals() {
    let g = gain_db(BlueboxParams::default(), 1e-3);
    assert!(g.abs() < 0.75, "default small-signal gain {g:.2} dB");
}

#[test]
fn every_drive_meets_the_same_ceiling() {
    // +12 dBFS in: the reference's output sits ≈ 9 dB under its input at every
    // Pentode/Triode setting — the last stage clips at a fixed level
    let levels: Vec<f64> = [(50.0, 50.0), (100.0, 50.0), (50.0, 100.0), (100.0, 100.0)]
        .iter()
        .map(|&(pentode, triode)| gain_db(BlueboxParams { pentode, triode, ..BlueboxParams::default() }, 4.0))
        .collect();
    let (lo, hi) = levels.iter().fold((f64::MAX, f64::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    assert!(hi - lo < 3.0, "ceilings spread {levels:?}");
    assert!((-14.0..-5.0).contains(&lo), "ceiling {levels:?}");
}

#[test]
fn closed_knobs_mute() {
    for p in [
        BlueboxParams { pentode: 0.0, ..BlueboxParams::default() },
        BlueboxParams { triode: 0.0, ..BlueboxParams::default() },
        BlueboxParams { output: 0.0, ..BlueboxParams::default() },
    ] {
        assert!(gain_db(p, 0.1) < -60.0, "{p:?}");
    }
}

#[test]
fn density_keeps_quiet_levels_and_saturation_adds_level() {
    let base = gain_db(BlueboxParams::default(), 1e-3);
    let dense = gain_db(BlueboxParams { density: -100.0, ..BlueboxParams::default() }, 1e-3);
    assert!((dense - base).abs() < 1.0, "density −100 moved the level {:.2} dB", dense - base);
    let sat = gain_db(BlueboxParams { saturation_in: true, saturation: 100.0, saturation_freq: SatFreq::Flat, ..BlueboxParams::default() }, 1e-3);
    // the parallel path sums in phase: +6.5 dB at Saturation 100 on the reference
    assert!((4.5..8.5).contains(&(sat - base)), "saturation added {:.2} dB", sat - base);
}

#[test]
fn mix_zero_is_the_dry_signal() {
    let x = sine(440.0, 0.5, 0.25, 48_000.0);
    let y = run(BlueboxParams { pentode: 100.0, triode: 100.0, mix: 0.0, ..BlueboxParams::default() }, &x, 48_000.0);
    assert!(x.iter().zip(&y).all(|(a, b)| (a - b).abs() < 1e-12));
}

#[test]
fn stays_finite_at_every_rate_and_extreme() {
    for sr in [44_100.0, 48_000.0, 96_000.0] {
        let mut seed = 7u32;
        let noise: Vec<f64> = (0..48_000)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (f64::from(seed) / f64::from(u32::MAX) - 0.5) * 8.0
            })
            .collect();
        for alt in [false, true] {
            let p = BlueboxParams {
                pentode: 100.0,
                triode: 100.0,
                saturation_in: true,
                saturation: 100.0,
                saturation_freq: SatFreq::Low,
                alt_tube: alt,
                air_in: true,
                air: 100.0,
                density: 100.0,
                input_db: 12.0,
                output: 100.0,
                ..BlueboxParams::default()
            };
            let y = run(p, &noise, sr);
            assert!(y.iter().all(|v| v.is_finite() && v.abs() < 8.0), "sr {sr} alt {alt}");
        }
    }
}
