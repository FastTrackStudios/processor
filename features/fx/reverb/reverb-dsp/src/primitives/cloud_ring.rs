//! Cloud's tank: one long ring of delay + allpass sections, tapped along
//! its length.
//!
//! Built to the structure measured from Strymon BigSky's Cloud (the plug-in,
//! with signal-analyzer's `bigsky_match`), not to `CloudSeed`'s parallel
//! combs:
//!
//! - **One ring, ~803 ms a trip.** With Decay at its top the plug-in's
//!   impulse response is bit-identical across Decay settings for the first
//!   803 ms after the tank's input — nothing recirculates sooner — and the
//!   recirculated part then carries one decay gain per trip.
//! - **Long allpasses inside the ring set a floor.** Below Decay ≈ 3000
//!   BigSky's Cloud never rings shorter than ~4.5 s, and its tail decays
//!   smoothly even when the loop returns 36 dB down: allpasses ring on their
//!   own feedback, which Decay does not touch. Here each section carries one
//!   long allpass whose gain gives that floor (never longer than the decay
//!   asked for — decay stays a time).
//! - **Taps along the ring, alternating sides.** A pulse travelling a section
//!   lights up a fixed pattern of output taps — BigSky's first section reads
//!   L at 24.5 ms, R at 49.4, R at 53.6, L at 71.1, L at 74.4 — so the output
//!   holds level for as long as energy is still travelling the ring: the
//!   plateau, with nothing like a multitap ahead of it.
//! - **Both sides in at one point.** The plug-in's response is identical
//!   across Decay for a whole trip with input on both sides, so both enter
//!   the ring together, ahead of everything; the stereo comes from the taps,
//!   which alternate sides along the ring (mirrored section to section).
//!
//! Short allpasses after each long one are the Diffusion inside the tank
//! (the knob's other half — the input chain ahead of the ring is the first).
//!
//! Allocates in [`CloudRing::new`] only; `tick` is allocation-free.

use dsp_core::num;

use super::one_pole::Lp1;
use audiocore_dsp::delay_line::DelayLine;
use audiocore_dsp::denormal::flush;

/// Sections in the ring.
const SECTIONS: usize = 4;
/// Section delay lengths (ms) at size 0.5. With each section's short
/// allpasses (a pure delay at zero Diffusion) they add up to BigSky's
/// 803.3 ms trip: 167.3, 217.6, 189.3 and 229.1 ms a section. The ratios
/// avoid common factors so the ring does not flutter.
const DELAY_MS: [f64; SECTIONS] = [151.23, 197.49, 174.0, 206.81];
/// The long (floor) allpass in each section, ms. Intervals that recur in
/// BigSky's Cloud impulse response.
const LONG_AP_MS: [f64; SECTIONS] = [61.04, 85.40, 43.35, 95.60];
/// The two short (diffusion) allpasses in each section, ms.
const SHORT_AP_MS: [[f64; 2]; SECTIONS] = [[4.77, 11.3], [6.21, 13.9], [5.43, 9.87], [7.09, 15.2]];
/// The ring's output taps: BigSky's Cloud's first pass through its tank,
/// pulse for pulse — read off the plug-in at Diffusion −10 (every
/// allpass a pure delay), MOD 0, Decay at its top (nothing decays inside
/// one trip), as `(ms after the tank's input, right side?, gain re the
/// first tap)`. Each tap sits at that time along the ring, so a pulse
/// travelling our ring lights the outputs where it lit BigSky's.
const FIRST_PASS_TAPS: [(f64, bool, f64); 98] = [
    (24.479, false, -0.998),
    (49.438, true, -0.998),
    (53.646, true, 0.751),
    (71.146, false, 0.749),
    (74.438, false, -1.000),
    (85.396, true, 1.000),
    (85.396, false, -1.000),
    (109.875, false, -1.000),
    (134.833, true, -1.000),
    (139.042, true, 0.752),
    (156.542, false, 0.750),
    (159.833, false, -1.002),
    (207.479, false, 1.000),
    (207.479, true, -1.000),
    (214.896, true, -0.998),
    (231.958, false, -1.000),
    (236.458, false, 0.438),
    (256.917, true, -1.000),
    (261.125, true, 0.752),
    (268.479, false, -1.000),
    (268.479, true, 1.000),
    (271.250, true, 0.433),
    (278.625, false, 0.750),
    (281.917, false, -1.002),
    (292.958, false, -1.000),
    (300.292, true, -1.000),
    (317.917, true, -1.000),
    (321.854, false, 0.438),
    (322.125, true, 0.752),
    (339.625, false, 0.750),
    (342.917, false, -1.003),
    (356.646, true, 0.438),
    (390.562, false, -0.801),
    (390.562, true, 0.801),
    (401.146, true, -0.749),
    (401.771, false, -0.328),
    (402.396, true, 1.004),
    (415.042, false, -0.801),
    (422.375, true, -1.000),
    (425.521, false, -0.749),
    (440.000, true, -0.801),
    (443.937, false, 0.438),
    (444.208, true, 0.602),
    (461.708, false, 0.601),
    (465.000, false, -0.803),
    (478.729, true, 0.438),
    (483.375, true, -1.001),
    (486.542, true, -0.747),
    (487.167, false, -0.328),
    (487.792, true, 1.007),
    (488.854, true, -0.332),
    (504.937, false, 0.438),
    (510.917, false, -0.750),
    (512.646, false, 0.705),
    (512.646, true, 0.701),
    (537.125, false, -0.701),
    (538.854, false, 1.003),
    (539.729, true, 0.438),
    (562.083, true, -0.701),
    (566.292, true, 0.527),
    (566.458, true, -0.442),
    (567.083, false, 0.246),
    (574.250, true, -0.328),
    (583.792, false, 0.526),
    (587.083, false, -0.702),
    (599.896, true, 0.749),
    (605.458, true, -0.802),
    (608.625, true, -0.748),
    (609.250, false, -0.328),
    (609.875, true, 1.007),
    (624.250, false, 1.000),
    (627.021, false, 0.347),
    (633.000, false, -0.751),
    (643.125, false, -0.437),
    (651.854, true, -0.438),
    (652.479, false, 0.246),
    (661.812, true, 0.350),
    (669.625, true, -0.750),
    (670.250, false, -0.328),
    (670.875, true, 1.006),
    (685.292, true, 0.750),
    (694.000, false, -0.750),
    (696.333, true, -0.328),
    (706.458, true, 0.246),
    (727.542, true, -0.701),
    (728.521, false, -0.438),
    (731.354, false, 0.750),
    (731.771, true, 0.329),
    (746.333, false, 1.000),
    (749.104, false, 0.303),
    (757.333, true, -0.328),
    (773.937, true, -0.438),
    (774.562, false, 0.246),
    (783.896, true, 0.307),
    (791.708, true, -0.601),
    (791.854, true, 0.254),
    (792.333, false, -0.263),
    (792.958, true, 0.803),
];
/// Most taps any one section can hold.
const MAX_SECTION_TAPS: usize = 48;
/// Overall tap level (the chain's wet calibration sets the final level).
const TAP_GAIN: f64 = 0.25;
/// BigSky's mono early pulse against its first ring tap: 0.1856 / 0.1328.
pub const EARLY_TO_TAP: f64 = 1.4;
/// Shortest the long allpasses ring, seconds: BigSky's Cloud floor.
pub const FLOOR_T60: f64 = 4.5;
/// How much longer than asked the floor allpasses make the tail, seconds
/// (at and below ~8 s; it fades out above).
const EXCESS_S: f64 = 1.4;
/// The largest size scale the buffers are sized for.
const MAX_SCALE: f64 = 1.6;
/// Headroom for modulation, samples.
const MOD_HEADROOM: f64 = 64.0;

/// A Schroeder allpass with a slowly modulated, linearly interpolated
/// delay.
struct RingAp {
    line: DelayLine,
    base_ms: f64,
    len: f64,
    g: f64,
    depth: f64,
    phase: f64,
    inc: f64,
}

impl RingAp {
    fn new(base_ms: f64, sample_rate: f64, phase: f64) -> Self {
        let cap = (base_ms * 1e-3 * sample_rate).mul_add(MAX_SCALE, MOD_HEADROOM + 8.0);
        Self {
            line: DelayLine::new(num::f64_to_index(cap)),
            base_ms,
            len: base_ms * 1e-3 * sample_rate,
            g: 0.0,
            depth: 0.0,
            phase,
            inc: 0.0,
        }
    }

    #[inline]
    fn tick(&mut self, x: f64) -> f64 {
        let mut d = self.len;
        if self.depth > 0.0 {
            self.phase += self.inc;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
            d += self.depth * (self.phase * core::f64::consts::TAU).sin();
        }
        let cap = num::count_to_f64(self.line.len()) - 4.0;
        // Cubic: a linear read is a gentle low-pass, and the ring applies
        // it on every pass — the top octave died early.
        let delayed = self.line.read_cubic(d.clamp(1.0, cap));
        let v = self.g.mul_add(-delayed, x);
        self.line.write(flush(v));
        self.g.mul_add(v, delayed)
    }
}

/// Cloud's input chain: four allpasses in series, the same on both sides
/// (the plug-in's early field is mono). Delays read off BigSky's Cloud at
/// a small Diffusion, where each stage leaves one faint pulse before the
/// chain's main pulse and one after it: 22.35, 28.60, 55.85 and 76.35 ms,
/// summing to the 183 ms the chain takes at zero gain.
const INPUT_AP_MS: [f64; 4] = [22.35, 28.60, 55.85, 76.35];

/// The input diffusion: [`INPUT_AP_MS`] allpasses, quadrature-modulated.
pub struct InputChain {
    stages: [RingAp; 4],
    sample_rate: f64,
}

impl InputChain {
    #[must_use]
    pub fn new(sample_rate: f64) -> Self {
        // Quadrature: the four stages' LFOs a quarter-cycle apart, so the
        // modulation does not pump in common.
        let stages = core::array::from_fn(|i| {
            RingAp::new(
                INPUT_AP_MS.get(i).copied().unwrap_or(20.0),
                sample_rate,
                num::count_to_f64(i) * 0.25,
            )
        });
        Self { stages, sample_rate }
    }

    /// Size 0.5 is BigSky's chain; it scales with the ring.
    pub fn set_size(&mut self, size: f64) {
        let scale = (size.clamp(0.0, 1.0) + 0.5).min(MAX_SCALE);
        for ap in &mut self.stages {
            ap.len = ap.base_ms * 1e-3 * self.sample_rate * scale;
        }
    }

    /// The allpasses' gain: Diffusion. At zero each stage is a pure delay.
    pub fn set_gain(&mut self, g: f64) {
        for ap in &mut self.stages {
            ap.g = g.clamp(0.0, 0.9);
        }
    }

    /// LFO depth (samples) and rate (Hz); each stage a touch off the rate.
    pub fn set_modulation(&mut self, depth: f64, rate_hz: f64) {
        for (i, ap) in self.stages.iter_mut().enumerate() {
            ap.depth = depth;
            ap.inc = rate_hz * 0.07f64.mul_add(num::count_to_f64(i), 0.9) / self.sample_rate;
        }
    }

    pub fn clear(&mut self) {
        for ap in &mut self.stages {
            ap.line.clear();
        }
    }

    #[inline]
    pub fn tick(&mut self, x: f64) -> f64 {
        self.stages.iter_mut().fold(x, |v, ap| ap.tick(v))
    }
}

struct Tap {
    pos: usize,
    right: bool,
    gain: f64,
}

struct Section {
    delay: DelayLine,
    base_ms: f64,
    len: usize,
    taps: Vec<Tap>,
    long_ap: RingAp,
    short: [RingAp; 2],
}

/// The ring. Feed it the (mono-diffused) left and right inputs; it returns
/// the tapped stereo output.
pub struct CloudRing {
    sections: [Section; SECTIONS],
    sample_rate: f64,
    scale: f64,
    t60: f64,
    loop_gain: f64,
    /// The ring's end, carried round to its start on the next sample.
    feedback: f64,
    dc_x: f64,
    dc_y: f64,
    dc_r: f64,
    damp: Lp1,
    damping_on: bool,
}

impl CloudRing {
    #[must_use]
    pub fn new(sample_rate: f64) -> Self {
        let sections = core::array::from_fn(|i| {
            let base_ms = DELAY_MS.get(i).copied().unwrap_or(200.0);
            let cap = (base_ms * 1e-3 * sample_rate).mul_add(MAX_SCALE, 8.0);
            let short_ms = SHORT_AP_MS.get(i).copied().unwrap_or([5.0, 10.0]);
            let phase = num::count_to_f64(i) * 0.25;
            Section {
                delay: DelayLine::new(num::f64_to_index(cap)),
                base_ms,
                len: 1,
                taps: Vec::with_capacity(MAX_SECTION_TAPS),
                long_ap: RingAp::new(LONG_AP_MS.get(i).copied().unwrap_or(60.0), sample_rate, phase),
                short: [
                    RingAp::new(short_ms[0], sample_rate, phase + 0.13),
                    RingAp::new(short_ms[1], sample_rate, phase + 0.61),
                ],
            }
        });
        let mut ring = Self {
            sections,
            sample_rate,
            scale: 1.0,
            t60: FLOOR_T60,
            loop_gain: 0.0,
            feedback: 0.0,
            dc_x: 0.0,
            dc_y: 0.0,
            // ~5 Hz DC blocker on the recirculation.
            dc_r: 1.0 - core::f64::consts::TAU * 5.0 / sample_rate,
            damp: Lp1::new(),
            damping_on: false,
        };
        ring.damp.set_freq(20_000.0, sample_rate);
        ring.set_size(0.5);
        ring
    }

    /// Size 0.5 is BigSky's ring (803 ms a trip); 0 → 0.5×, 1 → 1.5×.
    ///
    /// Allocation-free: the per-section tap lists are refilled within the
    /// capacity reserved in [`Self::new`].
    pub fn set_size(&mut self, size: f64) {
        self.scale = (size.clamp(0.0, 1.0) + 0.5).min(MAX_SCALE);
        let ms = self.sample_rate * 1e-3 * self.scale;
        for s in &mut self.sections {
            s.len = num::f64_to_index(s.base_ms * ms).max(2);
            s.long_ap.len = s.long_ap.base_ms * ms;
            for ap in &mut s.short {
                ap.len = ap.base_ms * ms;
            }
            s.taps.clear();
        }
        // Walk the ring as a pulse does at zero Diffusion — each section's
        // delay, then its short allpasses (pure delays) — and drop each
        // tap on the delay line where that time falls. A tap that lands on
        // the allpasses goes to the line's end.
        let mut start = 0.0;
        let mut section = 0;
        for &(t_ms, right, gain) in &FIRST_PASS_TAPS {
            let t = t_ms * ms;
            while let Some(s) = self.sections.get(section) {
                let span = num::count_to_f64(s.len) + s.short.iter().map(|a| a.len).sum::<f64>();
                if t < start + span {
                    break;
                }
                start += span;
                section += 1;
            }
            let Some(s) = self.sections.get_mut(section) else { break };
            if s.taps.len() < MAX_SECTION_TAPS {
                let pos = num::f64_to_index(t - start).clamp(1, s.len);
                s.taps.push(Tap { pos, right, gain: gain * TAP_GAIN });
            }
        }
        self.update_gains();
    }

    /// Decay time of the ring, seconds (≥ 1e5 holds it: Infinite).
    pub fn set_t60(&mut self, t60_s: f64) {
        self.t60 = t60_s.max(0.05);
        self.update_gains();
    }

    /// Gain of the short allpasses inside the ring (Diffusion's tank half).
    pub fn set_diffusion(&mut self, g: f64) {
        let g = g.clamp(0.0, 0.8);
        for s in &mut self.sections {
            for ap in &mut s.short {
                ap.g = g;
            }
        }
    }

    /// Slow modulation of the ring's allpasses: `depth` in samples at the
    /// short allpasses (the long ones take a third of it), `rate_hz` the
    /// mean rate — each allpass runs a little off it so they never align.
    pub fn set_modulation(&mut self, depth: f64, rate_hz: f64) {
        let sr = self.sample_rate;
        for (i, s) in self.sections.iter_mut().enumerate() {
            let k = num::count_to_f64(i);
            s.long_ap.depth = depth / 3.0;
            s.long_ap.inc = rate_hz * 0.13f64.mul_add(k, 0.6) / sr;
            for (j, ap) in s.short.iter_mut().enumerate() {
                ap.depth = depth;
                ap.inc = rate_hz * 0.11f64.mul_add(k + 2.0 * num::count_to_f64(j), 0.8) / sr;
            }
        }
    }

    /// In-loop damping: a one-pole low-pass on the recirculation, or off.
    pub fn set_damping(&mut self, cutoff_hz: Option<f64>) {
        self.damping_on = cutoff_hz.is_some();
        if let Some(fc) = cutoff_hz {
            self.damp.set_freq(fc.clamp(200.0, 20_000.0), self.sample_rate);
        }
    }

    /// Trip length in samples as energy travels it: the delays plus each
    /// allpass's mean delay (a Schroeder allpass delays energy by its own
    /// length on average, whatever its gain).
    fn trip_samples(&self) -> f64 {
        self.sections
            .iter()
            .map(|s| num::count_to_f64(s.len) + s.long_ap.len + s.short.iter().map(|a| a.len).sum::<f64>())
            .sum()
    }

    fn update_gains(&mut self) {
        let sr = self.sample_rate;
        // The floor allpasses' ringing adds to the loop's decay: with both
        // set to the requested time the tail measured ~1.4 s long below
        // ~8 s, the excess fading out above (bigsky_match's Decay sweep).
        // Ask the loop for that much less, so the tail lands on the time.
        let t = self.t60;
        // Near the floor less of it: the floor allpasses are capped at the
        // loop's time there, so they add less.
        let excess = if t <= 8.0 {
            EXCESS_S * ((t - 2.5) / 3.0).clamp(0.0, 1.0)
        } else {
            EXCESS_S * (-(t - 8.0) / 15.0).exp()
        };
        // Very long tails lose a little per trip besides (interpolation,
        // the DC blocker): 4 % short at 57 s, 9 % at 75 s. Ask for more.
        let long = 1.0 - 0.002 * (t - 30.0).clamp(0.0, 100.0);
        let loop_t60 = (t - excess).max(t * 0.35) / long;
        // One decay gain per trip.
        let trip_s = self.trip_samples() / sr;
        self.loop_gain = 10f64.powf(-3.0 * trip_s / loop_t60).min(1.0);
        // The long allpasses ring at the floor, or shorter when the decay
        // asked for is — a short Cloud is still a short reverb.
        let floor = FLOOR_T60.min(loop_t60);
        for s in &mut self.sections {
            let d_s = s.long_ap.len / sr;
            s.long_ap.g = 10f64.powf(-3.0 * d_s / floor).min(0.95);
        }
    }

    pub fn clear(&mut self) {
        for s in &mut self.sections {
            s.delay.clear();
            s.long_ap.line.clear();
            for ap in &mut s.short {
                ap.line.clear();
            }
        }
        self.feedback = 0.0;
        self.dc_x = 0.0;
        self.dc_y = 0.0;
        self.damp.reset();
    }

    /// One sample: both inputs enter at the ring's start.
    #[inline]
    pub fn tick(&mut self, in_l: f64, in_r: f64) -> (f64, f64) {
        let mut sig = self.feedback + 0.5 * (in_l + in_r);
        let (mut out_l, mut out_r) = (0.0, 0.0);
        for s in &mut self.sections {
            s.delay.write(sig);
            for tap in &s.taps {
                let v = s.delay.read(tap.pos) * tap.gain;
                if tap.right {
                    out_r += v;
                } else {
                    out_l += v;
                }
            }
            let mut v = s.delay.read(s.len);
            v = s.long_ap.tick(v);
            for ap in &mut s.short {
                v = ap.tick(v);
            }
            sig = v;
        }
        // DC-block the recirculation, damp it if asked, then the one decay
        // gain of the trip.
        let y = sig - self.dc_x + self.dc_r * self.dc_y;
        self.dc_x = sig;
        self.dc_y = flush(y);
        let mut back = self.dc_y;
        if self.damping_on {
            back = self.damp.tick(back);
        }
        self.feedback = flush(back * self.loop_gain);
        (out_l, out_r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn impulse_energy(ring: &mut CloudRing, seconds: f64) -> Vec<f64> {
        // Energy per 100 ms window.
        let n = num::f64_to_index(seconds * 48_000.0);
        let mut out = Vec::new();
        let mut acc = 0.0;
        for i in 0..n {
            let x = if i == 0 { 1.0 } else { 0.0 };
            let (l, r) = ring.tick(x, x);
            acc += l * l + r * r;
            if (i + 1) % 4800 == 0 {
                out.push(acc);
                acc = 0.0;
            }
        }
        out
    }

    /// Nothing recirculates before one trip: the output up to ~800 ms is
    /// the same whatever the decay, as on BigSky.
    #[test]
    fn decay_only_acts_after_one_trip() {
        let mut a = CloudRing::new(48_000.0);
        let mut b = CloudRing::new(48_000.0);
        // Both well above the floor, so the long allpasses are the same
        // (near it, they shorten with the decay).
        a.set_t60(60.0);
        b.set_t60(10.0);
        a.set_diffusion(0.5);
        b.set_diffusion(0.5);
        let ea = impulse_energy(&mut a, 1.5);
        let eb = impulse_energy(&mut b, 1.5);
        // 7 windows = 700 ms, inside one trip (803 ms).
        for k in 0..7 {
            assert!((ea[k] - eb[k]).abs() <= 1e-9 * ea[k].max(1e-30), "window {k} differs before a trip");
        }
        assert!(eb[14] < ea[14] * 0.9, "the shorter decay is quieter after a trip");
    }

    /// A long decay rings long and a short one dies — and Infinite holds.
    #[test]
    fn decay_is_bounded_and_ordered() {
        for t60 in [1.0, 4.5, 20.0] {
            let mut r = CloudRing::new(48_000.0);
            r.set_t60(t60);
            r.set_diffusion(0.6);
            let e = impulse_energy(&mut r, 8.0);
            assert!(e.iter().all(|v| v.is_finite()));
            let early: f64 = e[2..8].iter().sum();
            let late: f64 = e[70..80].iter().sum();
            assert!(late < early, "T60 {t60}: decays");
        }
        let mut inf = CloudRing::new(48_000.0);
        inf.set_t60(1.0e6);
        inf.set_diffusion(0.6);
        let e = impulse_energy(&mut inf, 20.0);
        assert!(e.iter().all(|v| v.is_finite() && *v < 100.0), "Infinite stays bounded");
    }
}
