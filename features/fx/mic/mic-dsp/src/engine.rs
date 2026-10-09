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


/// Samples of latency, as the reference reports.
pub const LATENCY: usize = 24;

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

/// Run a section's inverse over a kernel, in place. A high-pass's zeros
/// at DC become poles just inside it (0.05 Hz), so the result stays
/// bounded however the measured kernel was truncated.
fn remove(kernel: &mut [f64], section: Biquad, sample_rate: f64) {
    let leak = core::f64::consts::TAU * 0.05 / sample_rate;
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
    has_curve: bool,
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
            settings: Settings::default(),
            has_curve,
            model,
        };
        chain.apply(Settings::default(), true);
        chain
    }

    #[must_use]
    pub const fn settings(&self) -> Settings {
        self.settings
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
        let moves = self.model.proximity.moves(s.proximity, s.pattern);
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
        let stage = self.model.stage(s.low_cut).cloned().unwrap_or_default();
        self.poly = stage.poly;
        for (i, section) in self.post.iter_mut().enumerate() {
            *section = stage.post.get(i).map_or_else(Biquad::identity, |&p| design(p, sr));
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
        self.linear.conv_p.set_kernel(&self.kernel_p);
        self.linear.conv_g.set_kernel(&self.kernel_g);
        if self.has_curve {
            let sr = self.model.sample_rate;
            let post = self.model.stage(cut).map(|s| s.post.clone()).unwrap_or_default();
            for section in post {
                remove(&mut self.kernel_p, design(section, sr), sr);
                remove(&mut self.kernel_g, design(section, sr), sr);
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
        self.low_cut.process(y) * self.out_gain
    }
}
