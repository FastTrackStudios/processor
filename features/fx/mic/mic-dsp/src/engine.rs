//! One modelled mic, end to end: capsule pair in, mic out.
//!
//! ```text
//! front ───────────────┬─(swap)─┬─ u = f + r ── P∗ ─────────────────┐
//! rear ── rear trim ───┘        └─ v = f − r ── G∗ ── proximity ───┴─(+)── low cut ── output, phase
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
use crate::section::CornerShift;

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

/// A mic model running.
pub struct MicChain {
    model: MicModel,
    settings: Settings,
    conv_p: Convolver,
    conv_g: Convolver,
    proximity: [CornerShift; 2],
    low_cut: CornerShift,
    out_gain: f64,
    rear_gain: f64,
    kernel_p: Vec<f64>,
    kernel_g: Vec<f64>,
}

impl MicChain {
    #[must_use]
    pub fn new(model: MicModel) -> Self {
        let taps = model.taps;
        let mut chain = Self {
            conv_p: Convolver::new(taps),
            conv_g: Convolver::new(taps),
            proximity: [CornerShift::identity(); 2],
            low_cut: CornerShift::identity(),
            out_gain: 1.0,
            rear_gain: 1.0,
            kernel_p: vec![0.0; taps],
            kernel_g: vec![0.0; taps],
            settings: Settings::default(),
            model,
        };
        chain.apply(Settings::default(), true);
        chain
    }

    #[must_use]
    pub const fn settings(&self) -> Settings {
        self.settings
    }

    /// Change controls. Kernels are rebuilt only when pattern or axis
    /// moved (`force` rebuilds regardless).
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
        for (section, step) in self.proximity.iter_mut().zip(self.model.proximity.moves(s.proximity, s.pattern)) {
            match step {
                Some((zero, pole)) => section.set(zero, pole, sr),
                None => section.make_identity(),
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
        self.conv_p.set_kernel(&self.kernel_p);
        self.conv_g.set_kernel(&self.kernel_g);
    }

    pub fn reset(&mut self) {
        self.conv_p.reset();
        self.conv_g.reset();
        self.proximity.iter_mut().for_each(CornerShift::reset);
        self.low_cut.reset();
    }

    /// One capsule-pair sample in, one mic sample out.
    pub fn process(&mut self, front: f64, rear: f64) -> f64 {
        // Rear trim belongs to the input's rear channel, before the swap.
        let rear = rear * self.rear_gain;
        let (f, r) = if self.settings.swap { (rear, front) } else { (front, rear) };
        let p = self.conv_p.process(f + r);
        let g = self.proximity.iter_mut().fold(self.conv_g.process(f - r), |x, s| s.process(x));
        self.low_cut.process(p + g) * self.out_gain
    }
}
