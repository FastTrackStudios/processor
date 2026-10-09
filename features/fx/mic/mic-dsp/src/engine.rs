//! One modelled mic, end to end: capsule pair in, mic out.
//!
//! ```text
//! front ───────────────┬─(swap)─┬─ u = f + r ── P∗ ─────────────────┐
//! rear ── rear trim ───┘        └─ v = f − r ── G∗ ── proximity ───┴─(+)─ s
//!
//! s ── curve s + a₂s² + … ── mic's high-passes ── low cut ── output, phase
//! ```
//!
//! The curve sits after everything that shapes the response (pattern,
//! axis, proximity, rear trim all change the harmonics only through the
//! level they deliver) and before the mic's high-passes, low cut, output
//! gain and phase (which filter or scale the harmonics with the rest).
//! ```text
//! ```
//!
//! `P`/`G` are the crossfade of the two axis anchors around the axis
//! setting. Proximity moves the gradient path's corner; the low cut moves
//! the mic's built-in high-pass corner — each an exact first-order ratio.
//! The output reports the same 24 samples of latency the reference does:
//! the kernels are stored from 24 samples before the reference's time zero,
//! because its rear path looks ahead by that much.

use crate::conv::Convolver;
use crate::model::{AXES, AXIS_STEP_DEG, MicModel, PATTERNS};
use crate::model::{MAX_POLY, MAX_POST, PostSection};
use crate::section::{Biquad, CornerShift};


/// Samples of latency at 48 kHz, as the reference reports.
pub const LATENCY: usize = 24;

/// Samples of latency at a sample rate: the reference looks ahead a fixed
/// 0.5 ms (22, 24, 44 and 48 samples at 44.1, 48, 88.2 and 96 kHz), and
/// the kernels carry that look-ahead.
#[must_use]
pub fn latency(sample_rate: f64) -> usize {
    dsp_core::num::f64_to_index((sample_rate * 0.0005).round())
}

/// Every control of one mic.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Omni (0) … Figure-8 (8).
    pub pattern: usize,
    /// 0–180°.
    pub axis_deg: f64,
    /// 0 off, 1–3 the low-cut switch positions.
    pub low_cut: usize,
    /// −100 … +100 %.
    pub proximity: f64,
    /// −12 … +12 dB.
    pub output_db: f64,
    pub phase_invert: bool,
    /// Gain on the rear capsule, −6 … +6 dB.
    pub rear_trim_db: f64,
    /// Swap the capsules (Swap Channels and Master Reverse both do this
    /// to a mono mic).
    pub swap: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            pattern: 4,
            axis_deg: 0.0,
            low_cut: 0,
            proximity: 0.0,
            output_db: 0.0,
            phase_invert: false,
            rear_trim_db: 0.0,
            swap: false,
        }
    }
}

fn db_to_gain(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// The two kernels and the gradient path's proximity sections.
struct Path {
    conv_p: Convolver,
    conv_g: Convolver,
    proximity: [CornerShift; 2],
}

impl Path {
    fn new(taps: usize) -> Self {
        Self { conv_p: Convolver::new(taps), conv_g: Convolver::new(taps), proximity: [CornerShift::identity(); 2] }
    }

    fn process(&mut self, sum: f64, diff: f64) -> f64 {
        let gradient = self.proximity.iter_mut().fold(self.conv_g.process(diff), |x, sec| sec.process(x));
        self.conv_p.process(sum) + gradient
    }

    fn reset(&mut self) {
        self.conv_p.reset();
        self.conv_g.reset();
        self.proximity.iter_mut().for_each(CornerShift::reset);
    }
}

fn design(section: PostSection, sample_rate: f64) -> Biquad {
    match section {
        PostSection::HighPass(hz) => Biquad::high_pass(hz, sample_rate),
        PostSection::Peak { hz, q, db } => Biquad::peak(hz, q, db, sample_rate),
        PostSection::HighPass2 { hz, q } => Biquad::high_pass2(hz, q, sample_rate),
    }
}

/// Where a removed high-pass's zeros go (Hz): low enough that the curve's
/// input `K/B` is exact through the audio band (2 Hz cost LD-47K 35 dB).
const LEAK_HZ: f64 = 0.05;

/// Run a section's inverse over a kernel, in place. A high-pass's zeros
/// at DC become poles at [`LEAK_HZ`], so the result stays bounded however
/// the measured kernel was truncated.
fn remove(kernel: &mut [f64], section: Biquad, sample_rate: f64) {
    let leak = core::f64::consts::TAU * LEAK_HZ / sample_rate;
    let mut inv = section.inverse(leak);
    for v in kernel.iter_mut() {
        *v = inv.process(*v);
    }
}

/// A mic model running.
///
/// The output is the measured response exactly, plus the output stage's
/// distortion: `y = K∗x + B·(a₂s² + a₃s³ + …)`, where `B` is the mic's
/// high-passes after the curve and `s = (K/B)∗x` the level the curve sees.
/// Computing the linear part from `K` itself, rather than as `B·((K/B)∗x)`,
/// keeps it exact: `K/B` does not die away within the measured window, and
/// truncating it costs ~−70 dB at the low end, but only `s` sees that error,
/// and `s` only feeds terms 50 dB down.
pub struct MicChain {
    model: MicModel,
    settings: Settings,
    linear: Path,
    /// Only run when the model has a curve.
    pre: Path,
    low_cut: CornerShift,
    /// The sections after the curve, on the distortion terms.
    post: [Biquad; MAX_POST],
    /// The curve's coefficients at the current low-cut position.
    poly: [f64; MAX_POLY],
    out_gain: f64,
    rear_gain: f64,
    kernel_p: Vec<f64>,
    kernel_g: Vec<f64>,
    /// Working space for re-timing kernels (no allocation on a change).
    scratch: Vec<f64>,
    has_curve: bool,
    /// Dual mode: what the kernels are re-timed and re-centred to.
    dual: DualAdjust,
}

/// How dual mode changes one mic.
///
/// Each model carries its own fractional delay (linear interpolation;
/// 0.146 samples for all but Sphere Diffuse), and in dual mode both are
/// re-timed to the mix-weighted delay;
/// Align moves one mic by distance / 340 m/s with a near-ideal fractional
/// delay; and both share one proximity corner, the mix-weighted mean of
/// their own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DualAdjust {
    /// The model's own delay, samples (what the kernels carry).
    pub own_delay: f64,
    /// The delay to re-time to (= `own_delay` when not in dual mode).
    pub delay: f64,
    /// Extra delay (Align), samples, ≥ 0.
    pub align: f64,
    /// Shared proximity corner, if any.
    pub centre: Option<f64>,
}

impl DualAdjust {
    #[must_use]
    pub const fn none(own_delay: f64) -> Self {
        Self { own_delay, delay: own_delay, align: 0.0, centre: None }
    }
}

/// Linear-interpolation delay `(1 − d) + d z⁻¹` re-timed from `from` to
/// `to` (both in 0 … 1), run over a kernel in place.
fn retime(kernel: &mut [f64], from: f64, to: f64) {
    if (from - to).abs() < 1e-12 {
        return;
    }
    // y[n] = ((1−to) x[n] + to x[n−1] − from y[n−1]) / (1−from)
    let (mut px, mut py) = (0.0, 0.0);
    for v in kernel.iter_mut() {
        let x = *v;
        let y = from.mul_add(-py, (1.0 - to).mul_add(x, to * px));
        let y = y / (1.0 - from);
        px = x;
        py = y;
        *v = y;
    }
}

/// Half-length of the fractional-delay interpolator (taps each side),
/// inside the 24 samples of look-ahead the kernels carry.
const SHIFT_HALF: usize = 20;

/// Delay a kernel by `samples` (≥ 0) with a Blackman-windowed sinc, in
/// place.
fn shift(kernel: &mut [f64], scratch: &mut [f64], samples: f64) {
    use core::f64::consts::PI;
    if samples.abs() < 1e-12 {
        return;
    }
    let whole = dsp_core::f64_to_index(samples.floor());
    let frac = samples - samples.floor();
    let span = dsp_core::count_to_f64(SHIFT_HALF.saturating_add(1));
    // taps[j] weights input n − whole − (j − SHIFT_HALF)
    let mut taps = [0.0f64; 2 * SHIFT_HALF + 1];
    for (j, tap) in taps.iter_mut().enumerate() {
        let t = dsp_core::count_to_f64(j) - dsp_core::count_to_f64(SHIFT_HALF) - frac;
        let w = 0.08f64.mul_add((2.0 * PI * t / span).cos(), 0.5f64.mul_add((PI * t / span).cos(), 0.42));
        let sinc = if t.abs() < 1e-12 { 1.0 } else { (PI * t).sin() / (PI * t) };
        *tap = sinc * w;
    }
    for (n, out) in scratch.iter_mut().enumerate() {
        *out = taps
            .iter()
            .enumerate()
            .filter_map(|(j, h)| {
                let back = whole.checked_add(j)?.checked_sub(SHIFT_HALF);
                let src = match back {
                    Some(b) => n.checked_sub(b)?,
                    None => n.checked_add(SHIFT_HALF.checked_sub(whole.checked_add(j)?)?)?,
                };
                kernel.get(src).map(|x| h * x)
            })
            .sum();
    }
    kernel.copy_from_slice(scratch);
}

impl MicChain {
    #[must_use]
    pub fn new(model: MicModel) -> Self {
        let taps = model.taps;
        let has_curve = (0..crate::model::LOW_CUTS).any(|k| model.stage(k).is_some_and(|s| s.poly.iter().any(|&a| a != 0.0)));
        let mut chain = Self {
            linear: Path::new(taps),
            pre: Path::new(if has_curve { taps } else { 0 }),
            low_cut: CornerShift::identity(),
            post: [Biquad::identity(); MAX_POST],
            poly: [0.0; MAX_POLY],
            out_gain: 1.0,
            rear_gain: 1.0,
            kernel_p: vec![0.0; taps],
            kernel_g: vec![0.0; taps],
            scratch: vec![0.0; taps],
            settings: Settings::default(),
            has_curve,
            dual: DualAdjust::none(0.0),
            model,
        };
        chain.apply(Settings::default(), true);
        chain
    }

    #[must_use]
    pub const fn settings(&self) -> Settings {
        self.settings
    }

    #[must_use]
    pub const fn model(&self) -> &MicModel {
        &self.model
    }

    /// Samples of latency: the look-ahead the model's kernels carry.
    #[must_use]
    pub fn latency(&self) -> usize {
        latency(self.model.sample_rate)
    }

    /// Set the dual-mode adjustment (rebuilds the kernels if it changed).
    pub fn set_dual(&mut self, dual: DualAdjust) {
        if dual != self.dual {
            self.dual = dual;
            self.apply(self.settings, true);
        }
    }

    /// Change controls. Kernels are rebuilt only when pattern, axis or low
    /// cut moved (`force` rebuilds regardless).
    pub fn apply(&mut self, s: Settings, force: bool) {
        let reshape = force
            || s.pattern != self.settings.pattern
            || s.low_cut != self.settings.low_cut
            || s.axis_deg.to_bits() != self.settings.axis_deg.to_bits();
        self.settings = s;
        if reshape {
            self.rebuild_kernels();
        }
        let sr = self.model.sample_rate;
        let moves = match self.dual.centre {
            Some(centre) => self.model.proximity.moves_around(s.proximity, s.pattern, centre),
            None => self.model.proximity.moves(s.proximity, s.pattern),
        };
        for path in [&mut self.linear, &mut self.pre] {
            for (section, step) in path.proximity.iter_mut().zip(moves) {
                match step {
                    Some((zero, pole)) => section.set(zero, pole, sr),
                    None => section.make_identity(),
                }
            }
        }
        let cut = &self.model.low_cut_hz;
        let off = cut.first().copied().unwrap_or(20.0);
        // A position with its own anchors already has its low end; the
        // others are the off anchors with the built-in corner moved.
        if self.model.has_set(s.low_cut) {
            self.low_cut.make_identity();
        } else {
            self.low_cut.set(off, cut.get(s.low_cut).copied().unwrap_or(off), sr);
        }
        // By reference: this runs on the audio thread when a control moves.
        let stage = self.model.stage(s.low_cut);
        self.poly = stage.map_or([0.0; MAX_POLY], |st| st.poly);
        for (i, section) in self.post.iter_mut().enumerate() {
            *section = stage.and_then(|st| st.post.get(i)).map_or_else(Biquad::identity, |&p| design(p, sr));
        }
        let sign = if s.phase_invert { -1.0 } else { 1.0 };
        self.out_gain = sign * db_to_gain(s.output_db);
        self.rear_gain = db_to_gain(s.rear_trim_db);
    }

    fn rebuild_kernels(&mut self) {
        let pattern = self.settings.pattern.min(PATTERNS.saturating_sub(1));
        let pos = (self.settings.axis_deg.clamp(0.0, 180.0) / AXIS_STEP_DEG).clamp(0.0, dsp_core::count_to_f64(AXES.saturating_sub(1)));
        let lo = dsp_core::f64_to_index(pos.floor()).min(AXES.saturating_sub(2));
        let t = pos - dsp_core::count_to_f64(lo);
        let hi = lo.saturating_add(1);
        let cut = self.settings.low_cut;
        if let (Some((p0, g0)), Some((p1, g1))) = (self.model.anchor(cut, pattern, lo), self.model.anchor(cut, pattern, hi)) {
            for ((k, a), b) in self.kernel_p.iter_mut().zip(p0).zip(p1) {
                *k = (f64::from(*b) - f64::from(*a)).mul_add(t, f64::from(*a));
            }
            for ((k, a), b) in self.kernel_g.iter_mut().zip(g0).zip(g1) {
                *k = (f64::from(*b) - f64::from(*a)).mul_add(t, f64::from(*a));
            }
        }
        if self.dual.delay.to_bits() != self.dual.own_delay.to_bits() || self.dual.align != 0.0 {
            retime(&mut self.kernel_p, self.dual.own_delay, self.dual.delay);
            retime(&mut self.kernel_g, self.dual.own_delay, self.dual.delay);
            shift(&mut self.kernel_p, &mut self.scratch, self.dual.align);
            shift(&mut self.kernel_g, &mut self.scratch, self.dual.align);
        }
        self.linear.conv_p.set_kernel(&self.kernel_p);
        self.linear.conv_g.set_kernel(&self.kernel_g);
        if self.has_curve {
            let sr = self.model.sample_rate;
            if let Some(stage) = self.model.stage(cut) {
                for &section in &stage.post {
                    remove(&mut self.kernel_p, design(section, sr), sr);
                    remove(&mut self.kernel_g, design(section, sr), sr);
                }
            }
            self.pre.conv_p.set_kernel(&self.kernel_p);
            self.pre.conv_g.set_kernel(&self.kernel_g);
        }
    }

    pub fn reset(&mut self) {
        self.linear.reset();
        self.pre.reset();
        self.low_cut.reset();
        self.post.iter_mut().for_each(Biquad::reset);
    }

    /// One capsule-pair sample in, one mic sample out.
    pub fn process(&mut self, front: f64, rear: f64) -> f64 {
        let y = self.process_uncut(front, rear);
        self.low_cut.process(y) * self.out_gain
    }

    /// Everything before the low cut (and the output gain).
    fn process_uncut(&mut self, front: f64, rear: f64) -> f64 {
        // Rear trim belongs to the input's rear channel, before the swap.
        let rear = rear * self.rear_gain;
        let (fr, rr) = if self.settings.swap { (rear, front) } else { (front, rear) };
        let mut y = self.linear.process(fr + rr, fr - rr);
        if self.has_curve {
            let level = self.pre.process(fr + rr, fr - rr);
            // a₂s² + a₃s³ + … by Horner, through the high-passes after
            // the curve.
            let higher = self.poly.iter().rev().fold(0.0_f64, |acc, &a| acc.mul_add(level, a));
            let terms = level * level * higher;
            y += self.post.iter_mut().fold(terms, |x, sec| sec.process(x));
        }
        y
    }
}


/// Two modelled mics, mixed — the reference's dual mode.
pub struct DualMic {
    pub mic1: MicChain,
    pub mic2: MicChain,
    mix: f64,
    solo: Solo,
}

/// Which mic is heard alone, if either.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Solo {
    Off,
    Mic1,
    Mic2,
}

/// Dual-mode controls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DualSettings {
    /// 0 … 1 (mic 1 … mic 2).
    pub mix: f64,
    /// Mic 2's position relative to mic 1, cm (−2 … +2).
    pub align_cm: f64,
    pub solo: Solo,
}

impl DualMic {
    #[must_use]
    pub fn new(mic1: MicChain, mic2: MicChain) -> Self {
        let mut dual = Self { mic1, mic2, mix: 0.0, solo: Solo::Off };
        dual.set(DualSettings { mix: 0.0, align_cm: 0.0, solo: Solo::Off });
        dual
    }

    /// Apply dual controls (each mic's own settings go through
    /// [`MicChain::apply`]; mic 2 should share mic 1's proximity, output,
    /// phase, rear trim and swap).
    pub fn set(&mut self, s: DualSettings) {
        self.mix = s.mix.clamp(0.0, 1.0);
        self.solo = s.solo;
        let d1 = self.mic1.model().delay;
        let d2 = self.mic2.model().delay;
        let sr = self.mic1.model().sample_rate;
        if s.solo == Solo::Off {
            let m = self.mix;
            let delay = (1.0 - m).mul_add(d1, m * d2);
            let p1 = self.mic1.settings().pattern;
            let p2 = self.mic2.settings().pattern;
            let f1 = self.mic1.model().proximity.f0(p1);
            let f2 = self.mic2.model().proximity.f0(p2);
            let centre = (1.0 - m).mul_add(f1, m * f2);
            let align = s.align_cm / 100.0 / 340.0 * sr;
            self.mic1.set_dual(DualAdjust { own_delay: d1, delay, align: (-align).max(0.0), centre: Some(centre) });
            self.mic2.set_dual(DualAdjust { own_delay: d2, delay, align: align.max(0.0), centre: Some(centre) });
        } else {
            self.mic1.set_dual(DualAdjust::none(d1));
            self.mic2.set_dual(DualAdjust::none(d2));
        }
    }

    pub fn process(&mut self, front: f64, rear: f64) -> f64 {
        match self.solo {
            Solo::Mic1 => self.mic1.process(front, rear),
            Solo::Mic2 => self.mic2.process(front, rear),
            Solo::Off => {
                // The reference crossfades block by block, not whole mics:
                // the two mics are mixed before the low cut, and the mix
                // then runs through both low cuts, crossfaded again. (With
                // one model at two patterns, a per-mic low cut misses by
                // −30 dB; this nulls to −75.)
                let w = self.mix;
                let x = (1.0 - w).mul_add(self.mic1.process_uncut(front, rear), w * self.mic2.process_uncut(front, rear));
                let a = self.mic1.low_cut.process(x);
                let b = self.mic2.low_cut.process(x);
                (1.0 - w).mul_add(a, w * b) * self.mic1.out_gain
            }
        }
    }
}

/// The reference's 180 variant: one capsule pair, two mics — mic 1 facing
/// forward (the ordinary model) and mic 2 facing back (its own measured
/// models) — on the left and right.
///
/// `Mic Pan` is a balance: mic 1 is scaled by `min(1, 1 − p)²`, mic 2 by
/// `min(1, 1 + p)²`. `Stereo Width` crossfeeds: `L = α m1 + β m2`,
/// `R = β m1 + α m2`, with `α = (1 + w)/2`, `β = (1 − w²)/2` up to 100 %
/// and `α = 1`, `β = −(w − 1)/2` beyond. Both exact against the reference.
pub struct StereoMic {
    pub forward: MicChain,
    pub backward: MicChain,
    gains: [f64; 2],
    mix: [f64; 2],
}

/// The 180 variant's own controls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StereoSettings {
    /// −1 … +1 (mic 1 … mic 2).
    pub pan: f64,
    /// 0 … 2 (0 … 200 %).
    pub width: f64,
}

impl StereoMic {
    #[must_use]
    pub fn new(forward: MicChain, backward: MicChain) -> Self {
        let mut s = Self { forward, backward, gains: [1.0; 2], mix: [1.0, 0.0] };
        s.set(StereoSettings { pan: 0.0, width: 1.0 });
        s
    }

    pub fn set(&mut self, s: StereoSettings) {
        let p = s.pan.clamp(-1.0, 1.0);
        let g1 = (1.0 - p).min(1.0);
        let g2 = (1.0 + p).min(1.0);
        self.gains = [g1 * g1, g2 * g2];
        let w = s.width.clamp(0.0, 2.0);
        self.mix = if w <= 1.0 { [f64::midpoint(1.0, w), w.mul_add(-w, 1.0) / 2.0] } else { [1.0, -(w - 1.0) / 2.0] };
    }

    /// One capsule-pair sample in, `(left, right)` out.
    pub fn process(&mut self, front: f64, rear: f64) -> (f64, f64) {
        let m1 = self.forward.process(front, rear) * self.gains[0];
        let m2 = self.backward.process(front, rear) * self.gains[1];
        let [a, b] = self.mix;
        (a.mul_add(m1, b * m2), b.mul_add(m1, a * m2))
    }
}
