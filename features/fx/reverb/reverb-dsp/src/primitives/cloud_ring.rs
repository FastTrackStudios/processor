//! Cloud's tank: allpass stages feeding one tapped line, which recirculates
//! every 803.3 ms.
//!
//! Built to the structure measured from Strymon BigSky's Cloud (the plug-in,
//! with signal-analyzer's `bigsky_match`), not to `CloudSeed`'s parallel
//! combs:
//!
//! - **One loop, 803.3 ms a trip.** The plug-in's impulse response is
//!   bit-identical across Decay settings for the first 803 ms after the
//!   tank's input — nothing recirculates sooner — and what recirculates
//!   carries one decay gain per trip.
//! - **The first pass is longer than the loop.** At Decay 1000 the loop
//!   returns at −80 dB, yet the plug-in still rings ~5 s: ~1000 sparse pulses
//!   whose level falls ~13 dB/s. That single pass IS BigSky's 4.5 s decay
//!   floor — not a decay process, and nothing that densifies.
//! - **That pass is one tap pattern, six times.** Its pulses are 158 taps
//!   ([`PATTERN_TAPS`]) applied to the tank's input and again after each of
//!   five allpass stages in series — A, B, C, B, B, four allpasses each and
//!   no plain delay between them — at 1, 1, 1, 1, 0.8 and 0.7. Each stage's
//!   output also has a tap of its own. Read off the plug-in at Diffusion
//!   −8 and −6, where each pulse carries the first-order satellites of the
//!   allpasses ahead of it; at Diffusion −10 they are pure delays and the
//!   copies line up to the sample (85.40, 207.48, 268.48, 390.56,
//!   512.65 ms). A model of exactly this — the chain, the stages, the
//!   pattern and the measured pulse shape — fits BigSky's own output to
//!   1–3 % of its energy from Diffusion −8 to +4, with every allpass at
//!   0.85 × Diffusion except the long two in A (and in the input chain),
//!   which run at 0.85 × that.
//! - **Both sides in at one point.** The response is identical across Decay
//!   for a whole trip with input on both sides, so both enter together.
//!
//! The recirculation here is the tank's input, delayed a trip: each pass
//! is the first again. BigSky's second pass is not — what it feeds back has
//! been through some of the stages (its second pass correlates with the
//! first at only ~0.1, with B's 122 ms spacing in the difference) — and is
//! not yet modelled.
//!
//! Allocates in [`CloudRing::new`] only; `tick` is allocation-free.

use dsp_core::num;

use super::cloud_taps::PATTERN_TAPS;
use super::one_pole::Lp1;
use audiocore_dsp::delay_line::DelayLine;
use audiocore_dsp::denormal::flush;

/// The tank's allpass stages in series, `(delay ms, gain re Diffusion's)`
/// each: A, B, C, B, B.
const STAGES: [[(f64, f64); 4]; 5] = [
    [(26.04, 0.85), (13.34, 1.0), (10.42, 1.0), (35.60, 0.85)],
    [(19.06, 1.0), (14.90, 1.0), (37.23, 1.0), (50.90, 1.0)],
    [(25.44, 1.0), (18.60, 1.0), (9.52, 1.0), (7.44, 1.0)],
    [(19.06, 1.0), (14.90, 1.0), (37.23, 1.0), (50.90, 1.0)],
    [(19.06, 1.0), (14.90, 1.0), (37.23, 1.0), (50.90, 1.0)],
];
/// How much of the input and of each stage's output the pattern line gets.
const JUNCTION_MIX: [f64; 6] = [1.0, 1.0, 1.0, 1.0, 0.8, 0.7];
/// Each stage's own tap, `(left, right)` re the first pattern tap.
const JUNCTION_OUT: [(f64, f64); 5] = [(-1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-0.801, 0.801), (0.705, 0.701)];
/// Overall tap level (the chain's wet calibration sets the final level).
const TAP_GAIN: f64 = 0.25;
/// BigSky's mono early pulse against its first ring tap: 0.1856 / 0.1328.
pub const EARLY_TO_TAP: f64 = 1.4;
/// The loop's gain per 803.3 ms trip for each decay time (T20 of the whole
/// response): our own loop, calibrated against BigSky's measured Decay →
/// T20 law (bigsky_match's Decay sweep, inverted). The first pass gives
/// the ~4.5 s floor on its own; this gain is what lengthens it. BigSky's
/// own per-trip gains are not used directly: measured off one returning
/// pulse against its Decay-50000 render, they are relative, and its loop
/// re-enters at more than one point.
/// `(T20 s, loop gain per trip)`, interpolated in log time.
const LOOP_GAIN: [(f64, f64); 11] = [
    (4.50, 0.0672),
    (4.53, 0.0866),
    (4.77, 0.1766),
    (5.60, 0.3084),
    (8.15, 0.5105),
    (11.39, 0.6153),
    (19.83, 0.7603),
    (30.60, 0.8351),
    (57.16, 0.9137),
    (75.19, 0.9338),
    (93.12, 0.9462),
];
/// BigSky's floor: the decay of its first pass alone.
const FLOOR_T60: f64 = 4.5;
/// BigSky's trip, seconds.
const BIGSKY_TRIP_S: f64 = 0.8033;
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
        // Schroeder's sign, as BigSky's: −g on the instant path (its
        // first-order pulses come out negative against the main one).
        let v = self.g.mul_add(delayed, x);
        self.line.write(flush(v));
        (-self.g).mul_add(v, delayed)
    }
}

/// Cloud's input chain, the same on both sides (the plug-in's early field
/// is mono): four allpasses in series, 22.35 / 28.60 / 55.85 / 76.35 ms —
/// each leaves a faint pulse either side of the chain's 183 ms main pulse,
/// and the series alone accounts for the second-order pulses (the ±6.25 ms
/// pair is 28.60 − 22.35: one stage passing twice, another instantly). The
/// first two run at the Diffusion gain, the long two at 0.85 × it.
///
/// Five shorter delays (14.90, 13.34, 10.42, 9.52, 7.44 ms) once looked
/// nested in here; they are the tank's stages, seen through the chain.
const INPUT_STAGES: [(f64, f64); 4] = [(22.35, 1.0), (28.60, 1.0), (55.85, 0.85), (76.35, 0.85)];

/// The input diffusion: four allpasses, quadrature-modulated.
pub struct InputChain {
    stages: [RingAp; 4],
    sample_rate: f64,
}

impl InputChain {
    #[must_use]
    pub fn new(sample_rate: f64) -> Self {
        // Quadrature: the stages' LFOs a quarter-cycle apart, so the
        // modulation does not pump in common.
        Self {
            stages: core::array::from_fn(|i| {
                RingAp::new(INPUT_STAGES.get(i).map_or(20.0, |s| s.0), sample_rate, num::count_to_f64(i) * 0.25)
            }),
            sample_rate,
        }
    }

    /// Size 0.5 is BigSky's chain; it scales with the ring. Whole samples:
    /// a cubic read at a fractional delay is a gentle low-pass.
    pub fn set_size(&mut self, size: f64) {
        let scale = (size.clamp(0.0, 1.0) + 0.5).min(MAX_SCALE);
        let k = 1e-3 * self.sample_rate * scale;
        for ap in &mut self.stages {
            ap.len = (ap.base_ms * k).round();
        }
    }

    /// The allpasses' gain: 0.85 × Diffusion. At zero each is a pure delay.
    pub fn set_gain(&mut self, g: f64) {
        let g = g.clamp(0.0, 0.9);
        for (ap, s) in self.stages.iter_mut().zip(INPUT_STAGES.iter()) {
            ap.g = g * s.1;
        }
    }

    /// LFO depth (samples) and rate (Hz); each stage a touch off the rate.
    pub fn set_modulation(&mut self, depth: f64, rate_hz: f64) {
        let sr = self.sample_rate;
        for (i, ap) in self.stages.iter_mut().enumerate() {
            ap.depth = depth;
            ap.inc = rate_hz * 0.07f64.mul_add(num::count_to_f64(i), 0.9) / sr;
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
    /// The measured gain, and the gain in use (reshaped below the floor).
    base: f64,
    gain: f64,
    /// When the pulse comes on BigSky's first pass, seconds.
    t_s: f64,
}

/// The tank. Feed it the (mono-diffused) left and right inputs; it returns
/// the tapped stereo output.
pub struct CloudRing {
    stages: [[RingAp; 4]; 5],
    /// The pattern's line: the input plus each stage's output.
    line: DelayLine,
    taps: Vec<Tap>,
    /// The mix into the line and each stage's own tap, as in use (tilted
    /// below the floor like the pattern), and when each stage's output
    /// comes, seconds.
    mix: [f64; 6],
    out: [(f64, f64); 5],
    junction_s: [f64; 6],
    /// The tank's input, held a trip for the recirculation.
    trip: DelayLine,
    trip_len: usize,
    sample_rate: f64,
    scale: f64,
    t60: f64,
    loop_gain: f64,
    /// The trip's end, carried round to the input on the next sample.
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
        let stages = core::array::from_fn(|k| {
            core::array::from_fn(|j| {
                let ms = STAGES.get(k).and_then(|s| s.get(j)).map_or(10.0, |a| a.0);
                RingAp::new(ms, sample_rate, num::count_to_f64(4 * k + j) * 0.19 % 1.0)
            })
        });
        let longest = PATTERN_TAPS.iter().fold(0.0f64, |m, t| m.max(t.0));
        let line_cap = (longest * 1e-3 * sample_rate).mul_add(MAX_SCALE, 64.0);
        let trip_cap = (BIGSKY_TRIP_S * sample_rate).mul_add(MAX_SCALE, 64.0);
        let mut ring = Self {
            stages,
            line: DelayLine::new(num::f64_to_index(line_cap)),
            taps: Vec::with_capacity(PATTERN_TAPS.len()),
            mix: JUNCTION_MIX,
            out: JUNCTION_OUT,
            junction_s: [0.0; 6],
            trip: DelayLine::new(num::f64_to_index(trip_cap)),
            trip_len: 1,
            sample_rate,
            scale: 1.0,
            t60: 4.5,
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

    /// Size 0.5 is BigSky's tank (803 ms a trip); 0 → 0.5×, 1 → 1.5×.
    ///
    /// Allocation-free: the tap list is refilled within the capacity
    /// reserved in [`Self::new`].
    pub fn set_size(&mut self, size: f64) {
        self.scale = (size.clamp(0.0, 1.0) + 0.5).min(MAX_SCALE);
        let ms = self.sample_rate * 1e-3 * self.scale;
        // Whole samples (see `InputChain::set_size`); the stages' outputs
        // come where their allpasses add up to.
        let mut at = 0.0;
        for (k, stage) in self.stages.iter_mut().enumerate() {
            for ap in stage.iter_mut() {
                ap.len = (ap.base_ms * ms).round();
                at += ap.len;
            }
            if let Some(t) = self.junction_s.get_mut(k + 1) {
                *t = at / self.sample_rate;
            }
        }
        let cap = self.line.len().saturating_sub(2);
        self.taps.clear();
        for &(t_ms, right, gain) in &PATTERN_TAPS {
            let pos = num::f64_to_index((t_ms * ms).round()).min(cap);
            let base = gain * TAP_GAIN;
            self.taps.push(Tap { pos, right, base, gain: base, t_s: t_ms * 1e-3 * self.scale });
        }
        self.trip_len = num::f64_to_index((BIGSKY_TRIP_S * 1e3 * ms).round()).clamp(2, self.trip.len().saturating_sub(2));
        self.update_gains();
    }

    /// Decay time, seconds (≥ 1e5 holds the loop: Infinite).
    pub fn set_t60(&mut self, t60_s: f64) {
        self.t60 = t60_s.max(0.05);
        self.update_gains();
    }

    /// The stages' allpass gain: 0.85 × Diffusion (each allpass at its own
    /// share of it).
    pub fn set_diffusion(&mut self, g: f64) {
        let g = g.clamp(0.0, 0.85);
        for (stage, spec) in self.stages.iter_mut().zip(STAGES.iter()) {
            for (ap, s) in stage.iter_mut().zip(spec.iter()) {
                ap.g = g * s.1;
            }
        }
    }

    /// Slow modulation of the stages' allpasses: `depth` in samples,
    /// `rate_hz` the mean rate — each allpass runs a little off it so they
    /// never align.
    pub fn set_modulation(&mut self, depth: f64, rate_hz: f64) {
        let sr = self.sample_rate;
        for (i, ap) in self.stages.iter_mut().flatten().enumerate() {
            ap.depth = depth;
            ap.inc = rate_hz * 0.06f64.mul_add(num::count_to_f64(i), 0.8) / sr;
        }
    }

    /// In-loop damping: a one-pole low-pass on the recirculation, or off.
    pub fn set_damping(&mut self, cutoff_hz: Option<f64>) {
        self.damping_on = cutoff_hz.is_some();
        if let Some(fc) = cutoff_hz {
            self.damp.set_freq(fc.clamp(200.0, 20_000.0), self.sample_rate);
        }
    }

    fn update_gains(&mut self) {
        let t = self.t60;
        // The loop's gain per trip, from BigSky's own law (log-time
        // interpolation; past the table, the trip's T60 extends it, and
        // Infinite holds).
        let first = LOOP_GAIN[0];
        let last = LOOP_GAIN[LOOP_GAIN.len() - 1];
        let g_trip = if t >= 1.0e5 {
            1.0
        } else if t <= first.0 {
            first.1
        } else if t >= last.0 {
            10f64.powf(-3.0 * BIGSKY_TRIP_S / t).max(last.1)
        } else {
            LOOP_GAIN.windows(2).find(|w| t <= w[1].0).map_or(last.1, |w| {
                let ((t0, g0), (t1, g1)) = (w[0], w[1]);
                let f = (t / t0).ln() / (t1 / t0).ln();
                (g1 - g0).mul_add(f, g0)
            })
        };
        // Those gains are per 803.3 ms trip; a resized ring takes the gain
        // that keeps the same decay rate.
        let trip_ratio = num::count_to_f64(self.trip_len) / (BIGSKY_TRIP_S * self.sample_rate);
        self.loop_gain = g_trip.powf(trip_ratio).min(1.0);
        // Shorter than BigSky's floor (which it cannot go): tilt the first
        // pass itself down to the time asked for. A pulse comes at its
        // stage's time plus its tap's, so the tilt splits between the two.
        let tilt = if t < FLOOR_T60 { 1.0 / t - 1.0 / FLOOR_T60 } else { 0.0 };
        let fall = |t_s: f64| 10f64.powf(-3.0 * t_s * tilt);
        for tap in &mut self.taps {
            tap.gain = tap.base * fall(tap.t_s);
        }
        for (k, m) in self.mix.iter_mut().enumerate() {
            let t_s = self.junction_s.get(k).copied().unwrap_or(0.0);
            *m = JUNCTION_MIX.get(k).copied().unwrap_or(0.0) * fall(t_s);
        }
        for (k, o) in self.out.iter_mut().enumerate() {
            let t_s = self.junction_s.get(k + 1).copied().unwrap_or(0.0);
            let (l, r) = JUNCTION_OUT.get(k).copied().unwrap_or((0.0, 0.0));
            *o = (l * TAP_GAIN * fall(t_s), r * TAP_GAIN * fall(t_s));
        }
    }

    pub fn clear(&mut self) {
        for ap in self.stages.iter_mut().flatten() {
            ap.line.clear();
        }
        self.line.clear();
        self.trip.clear();
        self.feedback = 0.0;
        self.dc_x = 0.0;
        self.dc_y = 0.0;
        self.damp.reset();
    }

    /// One sample: both inputs enter at the tank's start.
    #[inline]
    pub fn tick(&mut self, in_l: f64, in_r: f64) -> (f64, f64) {
        let x = self.feedback + 0.5 * (in_l + in_r);
        self.trip.write(x);
        let (mut out_l, mut out_r) = (0.0, 0.0);
        let mut v = x;
        let mut u = x * self.mix[0];
        for ((stage, m), o) in self.stages.iter_mut().zip(self.mix.iter().skip(1)).zip(self.out.iter()) {
            v = stage.iter_mut().fold(v, |s, ap| ap.tick(s));
            u = m.mul_add(v, u);
            out_l = o.0.mul_add(v, out_l);
            out_r = o.1.mul_add(v, out_r);
        }
        self.line.write(u);
        // `read(1)` is the sample just written: a delay of k is read(k + 1).
        for tap in &self.taps {
            let s = self.line.read(tap.pos + 1) * tap.gain;
            if tap.right {
                out_r += s;
            } else {
                out_l += s;
            }
        }
        // The trip's end: DC-block, damp if asked, the trip's one decay gain.
        // Read for the next sample, so a trip is `trip_len` exactly.
        let back = self.trip.read(self.trip_len);
        let y = back - self.dc_x + self.dc_r * self.dc_y;
        self.dc_x = back;
        self.dc_y = flush(y);
        let mut b = self.dc_y;
        if self.damping_on {
            b = self.damp.tick(b);
        }
        self.feedback = flush(b * self.loop_gain);
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
        // Both above BigSky's floor: below it the first pass itself is
        // tilted shorter, by design.
        a.set_t60(60.0);
        b.set_t60(6.0);
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
