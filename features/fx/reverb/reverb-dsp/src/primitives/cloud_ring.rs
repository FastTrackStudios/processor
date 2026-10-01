//! Cloud's tank: a long tapped line whose first 803 ms is a ring.
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
//!   floor — not a decay process, and nothing that densifies. So the ring's
//!   end both feeds back (× the decay gain) and runs on, unrecirculated,
//!   into six more sections: a ~5.7 s line tapped at every one of those
//!   pulses ([`FIRST_PASS_TAPS`]).
//! - **Taps along the line, alternating sides** — the measured pulses — so
//!   the output holds level for as long as energy is still travelling: the
//!   plateau, and the floor, by construction.
//! - **Both sides in at one point.** The response is identical across Decay
//!   for a whole trip with input on both sides, so both enter together.
//!
//! Short allpasses in every section are the Diffusion inside the tank (the
//! knob's other half — the input chain ahead is the first): at zero they
//! are pure delays and the line is BigSky's pulse train; raised, each pulse
//! smears, more so the further down the line it is.
//!
//! An earlier version made the floor with long allpasses inside the ring;
//! they multiplied the echo paths on every trip and the tail went to fog at
//! zero Diffusion, where BigSky's stays grainy (echo density ≤ 0.18 for 3 s).
//!
//! Allocates in [`CloudRing::new`] only; `tick` is allocation-free.

use dsp_core::num;

use super::cloud_taps::FIRST_PASS_TAPS;
use super::one_pole::Lp1;
use audiocore_dsp::delay_line::DelayLine;
use audiocore_dsp::denormal::flush;

/// Sections in the loop.
const RING_SECTIONS: usize = 4;
/// Sections in all: the loop, then the unrecirculated extension.
const SECTIONS: usize = 10;
/// Section delay lengths (ms) at size 0.5. In the loop, with each
/// section's short allpasses (a pure delay at zero Diffusion), they add up
/// to BigSky's 803.3 ms trip; the extension takes the line to ~5.7 s, past
/// the last measured pulse. Ratios avoid common factors.
const DELAY_MS: [f64; SECTIONS] = [151.23, 197.49, 174.0, 206.81, 712.57, 751.33, 731.91, 713.29, 727.13, 742.61];
/// The two short (diffusion) allpasses in each section, ms.
const SHORT_AP_MS: [[f64; 2]; SECTIONS] = [
    [4.77, 11.3],
    [6.21, 13.9],
    [5.43, 9.87],
    [7.09, 15.2],
    [5.11, 12.7],
    [6.83, 10.9],
    [4.97, 14.3],
    [6.47, 11.9],
    [5.29, 13.1],
    [7.31, 10.3],
];
/// Most taps any one section can hold.
const MAX_SECTION_TAPS: usize = 256;
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
/// is mono). Recovered from BigSky's Cloud by deconvolving its tank (its
/// Diffusion −10 response) out of its output at other Diffusion settings:
///
/// - four allpasses in series, 22.35 / 28.60 / 55.85 / 76.35 ms — each
///   leaves a faint pulse either side of the chain's 183 ms main pulse, and
///   the series alone accounts for the second-order pulses (the ±6.25 ms
///   pair is 28.60 − 22.35: one stage passing twice, another instantly);
/// - five short allpasses NESTED inside the last two (the total stays
///   183 ms at zero gain): they leave their own ± pairs, absent at Diffusion
///   −10 and growing with it, then saturating — 10.42 / 13.34 / 14.90 ms
///   at 0.11·(1 − e^(−g/0.12)), 7.44 / 9.52 ms at half that. 55.85 holds
///   14.90 + 10.42 and 76.35 holds 13.34 + 9.52 + 7.44, where the missing
///   cross-terms put them; those two stages run at 0.86 × the gain of the
///   plain ones (their first-order pulses, at every setting measured).
const INPUT_STAGE_MS: [f64; 4] = [22.35, 28.60, 55.85, 76.35];
/// The nested allpasses: `(stage, delay ms, level of their gain law)`.
const INPUT_NESTED: [(usize, f64, f64); 5] =
    [(2, 14.90, 1.0), (2, 10.42, 1.0), (3, 13.34, 1.0), (3, 9.52, 0.5), (3, 7.44, 0.5)];
/// The nesting stages' gain against the plain ones'.
const NESTING_STAGE_GAIN: f64 = 0.86;

/// The input diffusion: four allpasses, the last two with allpasses nested
/// in their loops, quadrature-modulated.
pub struct InputChain {
    /// The outer stages; a stage's `base_ms` is its own line (its total
    /// less what it nests).
    stages: [RingAp; 4],
    nested: [RingAp; 5],
    sample_rate: f64,
}

impl InputChain {
    #[must_use]
    pub fn new(sample_rate: f64) -> Self {
        let line_ms = |i: usize| {
            let total = INPUT_STAGE_MS.get(i).copied().unwrap_or(20.0);
            total - INPUT_NESTED.iter().filter(|n| n.0 == i).map(|n| n.1).sum::<f64>()
        };
        // Quadrature: the stages' LFOs a quarter-cycle apart, so the
        // modulation does not pump in common.
        Self {
            stages: core::array::from_fn(|i| RingAp::new(line_ms(i), sample_rate, num::count_to_f64(i) * 0.25)),
            nested: core::array::from_fn(|i| {
                RingAp::new(INPUT_NESTED.get(i).map_or(10.0, |n| n.1), sample_rate, num::count_to_f64(i) * 0.25 + 0.125)
            }),
            sample_rate,
        }
    }

    fn all_mut(&mut self) -> impl Iterator<Item = &mut RingAp> {
        self.stages.iter_mut().chain(self.nested.iter_mut())
    }

    /// Size 0.5 is BigSky's chain; it scales with the ring. Whole samples:
    /// a cubic read at a fractional delay is a gentle low-pass.
    pub fn set_size(&mut self, size: f64) {
        let scale = (size.clamp(0.0, 1.0) + 0.5).min(MAX_SCALE);
        let k = 1e-3 * self.sample_rate * scale;
        for ap in self.all_mut() {
            ap.len = (ap.base_ms * k).round();
        }
    }

    /// The allpasses' gain: Diffusion. At zero each stage is a pure delay.
    pub fn set_gain(&mut self, g: f64) {
        let g = g.clamp(0.0, 0.9);
        for (i, ap) in self.stages.iter_mut().enumerate() {
            let nests = INPUT_NESTED.iter().any(|n| n.0 == i);
            ap.g = if nests { g * NESTING_STAGE_GAIN } else { g };
        }
        let inner = 0.11 * (1.0 - (-g / 0.12).exp());
        for (ap, n) in self.nested.iter_mut().zip(INPUT_NESTED.iter()) {
            ap.g = inner * n.2;
        }
    }

    /// LFO depth (samples) and rate (Hz); each stage a touch off the rate.
    pub fn set_modulation(&mut self, depth: f64, rate_hz: f64) {
        let sr = self.sample_rate;
        for (i, ap) in self.all_mut().enumerate() {
            ap.depth = depth;
            ap.inc = rate_hz * 0.07f64.mul_add(num::count_to_f64(i % 4), 0.9) / sr;
        }
    }

    pub fn clear(&mut self) {
        for ap in self.all_mut() {
            ap.line.clear();
        }
    }

    #[inline]
    pub fn tick(&mut self, x: f64) -> f64 {
        let (first, rest) = self.stages.split_at_mut(2);
        let mut v = x;
        for ap in first.iter_mut() {
            v = ap.tick(v);
        }
        let (n55, n76) = self.nested.split_at_mut(2);
        if let [s55, s76] = rest {
            v = s55.tick_nested(v, n55);
            v = s76.tick_nested(v, n76);
        }
        v
    }
}

impl RingAp {
    /// A nested allpass (Gardner): `inner` sits in this one's delay path,
    /// so the loop is this line plus the inner allpasses.
    #[inline]
    fn tick_nested(&mut self, x: f64, inner: &mut [Self]) -> f64 {
        let mut d = self.len;
        if self.depth > 0.0 {
            self.phase += self.inc;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
            d += self.depth * (self.phase * core::f64::consts::TAU).sin();
        }
        let cap = num::count_to_f64(self.line.len()) - 4.0;
        let w = inner.iter_mut().fold(self.line.read_cubic(d.clamp(1.0, cap)), |v, ap| ap.tick(v));
        let v = self.g.mul_add(w, x);
        self.line.write(flush(v));
        (-self.g).mul_add(v, w)
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

struct Section {
    delay: DelayLine,
    base_ms: f64,
    len: usize,
    taps: Vec<Tap>,
    short: [RingAp; 2],
}

/// The tank. Feed it the (mono-diffused) left and right inputs; it returns
/// the tapped stereo output.
pub struct CloudRing {
    sections: [Section; SECTIONS],
    sample_rate: f64,
    scale: f64,
    t60: f64,
    loop_gain: f64,
    /// The loop's end, carried round to its start on the next sample.
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
            let cap = (base_ms * 1e-3 * sample_rate).mul_add(MAX_SCALE, 16.0);
            let short_ms = SHORT_AP_MS.get(i).copied().unwrap_or([5.0, 10.0]);
            let phase = num::count_to_f64(i) * 0.25;
            Section {
                delay: DelayLine::new(num::f64_to_index(cap)),
                base_ms,
                len: 1,
                taps: Vec::with_capacity(MAX_SECTION_TAPS),
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
    /// Allocation-free: the per-section tap lists are refilled within the
    /// capacity reserved in [`Self::new`].
    pub fn set_size(&mut self, size: f64) {
        self.scale = (size.clamp(0.0, 1.0) + 0.5).min(MAX_SCALE);
        let ms = self.sample_rate * 1e-3 * self.scale;
        for s in &mut self.sections {
            s.len = num::f64_to_index(s.base_ms * ms).max(2);
            for ap in &mut s.short {
                // Whole samples (see `InputChain::set_size`).
                ap.len = (ap.base_ms * ms).round();
            }
            s.taps.clear();
        }
        // Walk the line as a pulse does at zero Diffusion — each section's
        // short allpasses (pure delays then), then its delay — and drop
        // each tap on the delay line where that time falls. A tap that
        // lands on the allpasses goes to the line's start.
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
            let aps: f64 = s.short.iter().map(|a| a.len).sum();
            if s.taps.len() < MAX_SECTION_TAPS {
                let pos = num::f64_to_index((t - start - aps).max(0.0).round()).min(s.len);
                let base = gain * TAP_GAIN;
                s.taps.push(Tap { pos, right, base, gain: base, t_s: t / self.sample_rate });
            }
        }
        self.update_gains();
    }

    /// Decay time, seconds (≥ 1e5 holds the loop: Infinite).
    pub fn set_t60(&mut self, t60_s: f64) {
        self.t60 = t60_s.max(0.05);
        self.update_gains();
    }

    /// Gain of the short allpasses along the line (Diffusion's tank half).
    pub fn set_diffusion(&mut self, g: f64) {
        let g = g.clamp(0.0, 0.8);
        for s in &mut self.sections {
            for ap in &mut s.short {
                ap.g = g;
            }
        }
    }

    /// Slow modulation of the line's allpasses: `depth` in samples, `rate_hz`
    /// the mean rate — each allpass runs a little off it so they never align.
    pub fn set_modulation(&mut self, depth: f64, rate_hz: f64) {
        let sr = self.sample_rate;
        for (i, s) in self.sections.iter_mut().enumerate() {
            let k = num::count_to_f64(i);
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

    /// The loop's trip in samples as energy travels it: the delays plus each
    /// allpass's mean delay (a Schroeder allpass delays energy by its own
    /// length on average, whatever its gain).
    fn trip_samples(&self) -> f64 {
        self.sections
            .iter()
            .take(RING_SECTIONS)
            .map(|s| num::count_to_f64(s.len) + s.short.iter().map(|a| a.len).sum::<f64>())
            .sum()
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
        let trip_ratio = self.trip_samples() / (BIGSKY_TRIP_S * self.sample_rate);
        self.loop_gain = g_trip.powf(trip_ratio).min(1.0);
        // Shorter than BigSky's floor (which it cannot go): tilt the first
        // pass itself down to the time asked for.
        let tilt = if t < FLOOR_T60 { 1.0 / t - 1.0 / FLOOR_T60 } else { 0.0 };
        for s in &mut self.sections {
            for tap in &mut s.taps {
                tap.gain = tap.base * 10f64.powf(-3.0 * tap.t_s * tilt);
            }
        }
    }

    pub fn clear(&mut self) {
        for s in &mut self.sections {
            s.delay.clear();
            for ap in &mut s.short {
                ap.line.clear();
            }
        }
        self.feedback = 0.0;
        self.dc_x = 0.0;
        self.dc_y = 0.0;
        self.damp.reset();
    }

    /// One sample: both inputs enter at the loop's start.
    #[inline]
    pub fn tick(&mut self, in_l: f64, in_r: f64) -> (f64, f64) {
        let mut sig = self.feedback + 0.5 * (in_l + in_r);
        let (mut out_l, mut out_r) = (0.0, 0.0);
        for (i, s) in self.sections.iter_mut().enumerate() {
            // Diffusion first: BigSky's tank fogs within ~300 ms at Diffusion
            // 0, before the measured input chain alone could, so its
            // diffusers act ahead of even the first taps.
            for ap in &mut s.short {
                sig = ap.tick(sig);
            }
            s.delay.write(sig);
            // `read(1)` is the sample just written: a delay of k is read(k + 1).
            for tap in &s.taps {
                let v = s.delay.read(tap.pos + 1) * tap.gain;
                if tap.right {
                    out_r += v;
                } else {
                    out_l += v;
                }
            }
            sig = s.delay.read(s.len + 1);
            if i + 1 == RING_SECTIONS {
                // The loop's end: DC-block, damp if asked, the trip's one
                // decay gain — and carry on, unrecirculated, into the
                // extension.
                let y = sig - self.dc_x + self.dc_r * self.dc_y;
                self.dc_x = sig;
                self.dc_y = flush(y);
                let mut back = self.dc_y;
                if self.damping_on {
                    back = self.damp.tick(back);
                }
                self.feedback = flush(back * self.loop_gain);
            }
        }
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
