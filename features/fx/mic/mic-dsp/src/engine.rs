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
use crate::model::Move;
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
    proximity: [CornerShift; 3],
    /// Dual mode: the other mic's proximity filter over this kernel's own
    /// base, and the weights the two branches are crossfaded with.
    cross: [CornerShift; 3],
    /// … and the pressure path's: its high-pass moved to the other mic's.
    pressure_cross: CornerShift,
    weights: Option<[f64; 2]>,
}

impl Path {
    fn new(taps: usize) -> Self {
        Self {
            conv_p: Convolver::new(taps),
            conv_g: Convolver::new(taps),
            proximity: [CornerShift::identity(); 3],
            cross: [CornerShift::identity(); 3],
            pressure_cross: CornerShift::identity(),
            weights: None,
        }
    }

    fn process(&mut self, sum: f64, diff: f64) -> f64 {
        let g = self.conv_g.process(diff);
        let own = self.proximity.iter_mut().fold(g, |x, sec| sec.process(x));
        let p = self.conv_p.process(sum);
        match self.weights {
            Some([a, b]) => {
                let gradient = a.mul_add(own, b * self.cross.iter_mut().fold(g, |x, sec| sec.process(x)));
                a.mul_add(p, b * self.pressure_cross.process(p)) + gradient
            }
            None => p + own,
        }
    }

    fn reset(&mut self) {
        self.conv_p.reset();
        self.conv_g.reset();
        self.proximity.iter_mut().for_each(CornerShift::reset);
        self.cross.iter_mut().for_each(CornerShift::reset);
        self.pressure_cross.reset();
    }
}

fn design(section: PostSection, sample_rate: f64) -> Biquad {
    match section {
        PostSection::HighPass(hz) => Biquad::high_pass(hz, sample_rate),
        PostSection::Peak { hz, q, db } => Biquad::peak(hz, q, db, sample_rate),
        PostSection::HighPass2 { hz, q } => Biquad::high_pass2(hz, q, sample_rate),
    }
}

/// Where a cancelled proximity high-pass's zero at DC goes (Hz).
const BASE_LEAK_HZ: f64 = 0.05;

/// Where the pressure block's missing high-pass goes when the other mic has
/// none (Hz): one pole, so far lower than [`BASE_LEAK_HZ`] is safe.
const PRESSURE_LEAK_HZ: f64 = 0.005;

/// A base's inverse followed by another filter, with a leaked DC pole and a
/// following high-pass's zero at DC cancelled exactly: Corner to Corner is
/// then one clean corner move (the leak alone costs −52 dB at 20 Hz).
fn merge(inverse: Option<Move>, [a, b]: [Option<Move>; 2]) -> [Option<Move>; 3] {
    match (inverse, a) {
        (Some((zero, pole)), Some((0.0, next))) if (pole - BASE_LEAK_HZ).abs() < 1e-12 => [Some((zero, next)), b, None],
        _ => [inverse, a, b],
    }
}

/// Where the inverted sections' zeros at DC go (Hz). The reference's curve
/// input is `K/B` with no leak at all (its phase follows `K/B` to 2 Hz), but
/// lower than this the kernels' residual DC drifts the curve (0.005 Hz:
/// DN-7 −50 → −37 dB on pink noise); higher bends the audio band (2 Hz cost
/// LD-47K 35 dB).
const LEAK_HZ: f64 = 0.05;

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
    low_cut: CornerShift,
    /// Dual mode: this response's low end moved to the other mic's.
    low_cross: CornerShift,
    /// The sections after the curve, on the distortion terms.
    post: [Biquad; MAX_POST],
    /// Their inverses, run on the linear output: the curve's input.
    inverse: [Biquad; MAX_POST],
    /// The curve's coefficients at the current low-cut position.
    poly: [f64; MAX_POLY],
    /// … and the level its input is held to.
    clamp: f64,
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
    /// The pattern step whose proximity base (corner, shelf depth) the mic
    /// takes, if not its own.
    pub law_pattern: Option<usize>,
    /// Another mic's proximity filter (law and pattern step) run in place
    /// of this mic's own: the 180 variant's backward mic runs mic 1's.
    pub foreign: Option<(crate::model::ProximityLaw, usize)>,
    /// Dual mode: the other mic's proximity filter (law, step) crossfaded
    /// with this mic's own, with weights `[own, other]`.
    pub cross: Option<(crate::model::ProximityLaw, usize, [f64; 2])>,
    /// Dual mode: the other mic's low-end corner at its low-cut position.
    pub cross_low: Option<f64>,
    /// Dual mode: the other mic's pressure-path corner.
    pub cross_pressure: Option<f64>,
}

impl DualAdjust {
    #[must_use]
    pub const fn none(own_delay: f64) -> Self {
        Self { own_delay, delay: own_delay, align: 0.0, centre: None, law_pattern: None, foreign: None, cross: None, cross_low: None, cross_pressure: None }
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
            inverse: [Biquad::identity(); MAX_POST],
            low_cut: CornerShift::identity(),
            low_cross: CornerShift::identity(),
            post: [Biquad::identity(); MAX_POST],
            poly: [0.0; MAX_POLY],
            clamp: f64::INFINITY,
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
        let law = self.model.proximity;
        let law_pattern = self.dual.law_pattern.unwrap_or(s.pattern);
        let pad = |[a, b]: [Option<Move>; 2]| [a, b, None];
        let moves = match (self.dual.foreign, self.dual.centre) {
            (Some((other, pattern)), _) => {
                // the kernels' own base out, the other mic's whole filter in
                merge(law.base_inverse(s.pattern, BASE_LEAK_HZ), other.filter(s.proximity, pattern))
            }
            (None, Some(centre)) => pad(law.moves_around(s.proximity, s.pattern, centre, law_pattern)),
            (None, None) if law_pattern != s.pattern => pad(law.moves_around(s.proximity, s.pattern, law.f0(law_pattern), law_pattern)),
            (None, None) => pad(law.moves(s.proximity, s.pattern)),
        };
        let path = &mut self.linear;
        for (section, step) in path.proximity.iter_mut().zip(moves) {
            match step {
                Some((zero, pole)) => section.set(zero, pole, sr),
                None => section.make_identity(),
            }
        }
        path.weights = self.dual.cross.map(|(_, _, w)| w);
        match self.dual.cross_pressure {
            // a flat block's missing high-pass is a pole at DC: leaked
            Some(target) => path.pressure_cross.set(self.model.dual_pressure_hz, target.max(PRESSURE_LEAK_HZ), sr),
            None => path.pressure_cross.make_identity(),
        }
        if let Some((other, pattern, _)) = self.dual.cross {
            // this kernel's own base out, the other mic's whole filter in
            let steps = merge(law.base_inverse(s.pattern, BASE_LEAK_HZ), other.filter(s.proximity, pattern));
            for (section, step) in path.cross.iter_mut().zip(steps) {
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
        // dual mode: from the low end this response carries to the other mic's
        match self.dual.cross_low {
            Some(target) => self.low_cross.set(self.model.low_end_corner(0), target, sr),
            None => self.low_cross.make_identity(),
        }
        // By reference: this runs on the audio thread when a control moves.
        let stage = self.model.stage(s.low_cut);
        self.poly = stage.map_or([0.0; MAX_POLY], |st| st.poly);
        self.clamp = stage.map_or(f64::INFINITY, |st| st.clamp);
        let leak = core::f64::consts::TAU * LEAK_HZ / sr;
        for (i, (section, inverse)) in self.post.iter_mut().zip(&mut self.inverse).enumerate() {
            *section = stage.and_then(|st| st.post.get(i)).map_or_else(Biquad::identity, |&p| design(p, sr));
            *inverse = section.inverse(leak);
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
    }

    pub fn reset(&mut self) {
        self.linear.reset();
        self.inverse.iter_mut().for_each(Biquad::reset);
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
            // The curve sees the response with the sections after it undone
            // (`K/B`), as a running filter: a kernel truncated to the
            // measured window got its sub-audio gain wrong, and program
            // material's content below 5 Hz then cost dynamics 30 dB.
            let level = self.inverse.iter_mut().fold(y, |x, sec| sec.process(x)).clamp(-self.clamp, self.clamp);
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
            let (p1, p2) = (self.mic1.settings().pattern, self.mic2.settings().pattern);
            let (law1, law2) = (self.mic1.model().proximity, self.mic2.model().proximity);
            // the proximity filters crossfade weighted by each model's own
            // weight; the cores and low ends crossfade plainly
            let g1 = (1.0 - m) * self.mic1.model().dual_weight.get(p1).copied().unwrap_or(1.0);
            let g2 = m * self.mic2.model().dual_weight.get(p2).copied().unwrap_or(1.0);
            let total = g1 + g2;
            let (b1, b2) = if total > 0.0 { (g1 / total, g2 / total) } else { (1.0 - m, m) };
            // Validated between Corner and Shelf models; a Dynamic one (flat
            // base) needs a leaky inverse that blows up the kernels' residual
            // low end, and its dual block is not identified yet.
            let dynamic = |l: crate::model::ProximityLaw| matches!(l, crate::model::ProximityLaw::Dynamic { .. });
            let same = (law1 == law2 && p1 == p2) || dynamic(law1) || dynamic(law2);
            let cross1 = (!same).then_some((law2, p2, [b1, b2]));
            let cross2 = (!same).then_some((law1, p1, [b2, b1]));
            let (pr1, pr2) = (self.mic1.model().dual_pressure_hz, self.mic2.model().dual_pressure_hz);
            let end_1 = self.mic1.model().low_end_corner(self.mic1.settings().low_cut);
            let end_2 = self.mic2.model().low_end_corner(self.mic2.settings().low_cut);
            let align = s.align_cm / 100.0 / 340.0 * sr;
            self.mic1.set_dual(DualAdjust { delay, align: (-align).max(0.0), cross: cross1, cross_low: Some(end_2), cross_pressure: (!same).then_some(pr2), ..DualAdjust::none(d1) });
            self.mic2.set_dual(DualAdjust { delay, align: align.max(0.0), cross: cross2, cross_low: Some(end_1), cross_pressure: (!same).then_some(pr1), ..DualAdjust::none(d2) });
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
                // each mic's low end (built-in corner and low cut) is
                // crossfaded with the other's, as are the proximity filters
                // (weighted, in the gradient path); the cores mix plainly.
                let w = self.mix;
                let a = self.mic1.process_uncut(front, rear);
                let b = self.mic2.process_uncut(front, rear);
                let a = (1.0 - w).mul_add(self.mic1.low_cut.process(a), w * self.mic1.low_cross.process(a));
                let b = w.mul_add(self.mic2.low_cut.process(b), (1.0 - w) * self.mic2.low_cross.process(b));
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

    /// Apply the 180 controls; call it again after changing either mic's
    /// settings (the backward mic takes the forward mic's proximity base).
    pub fn set(&mut self, s: StereoSettings) {
        let own = self.backward.model().delay;
        let forward = self.forward.settings().pattern;
        // the backward mic runs the forward mic's proximity filter (measured:
        // its gradient low end follows mic 1's model, law and pattern)
        let law = self.forward.model().proximity;
        let adjust = if law == self.backward.model().proximity {
            // one model: its own law at mic 1's step, nothing to cancel
            DualAdjust { law_pattern: Some(forward), ..DualAdjust::none(own) }
        } else {
            // two models: −20..−50 dB; the coupling is not fully identified
            DualAdjust { foreign: Some((law, forward)), ..DualAdjust::none(own) }
        };
        self.backward.set_dual(adjust);
        let p = s.pan.clamp(-1.0, 1.0);
        // Each mic's pan gain is applied twice, before its curve and after
        // it: the level falls as g², its second harmonic as g and its third
        // as g² relative to it (measured; a gain on the output alone misses
        // by −55 dB at −80 %).
        self.gains = [(1.0 - p).min(1.0), (1.0 + p).min(1.0)];
        let w = s.width.clamp(0.0, 2.0);
        self.mix = if w <= 1.0 { [f64::midpoint(1.0, w), w.mul_add(-w, 1.0) / 2.0] } else { [1.0, -(w - 1.0) / 2.0] };
    }

    /// One capsule-pair sample in, `(left, right)` out.
    pub fn process(&mut self, front: f64, rear: f64) -> (f64, f64) {
        let [g1, g2] = self.gains;
        let m1 = self.forward.process(front * g1, rear * g1) * g1;
        let m2 = self.backward.process(front * g2, rear * g2) * g2;
        let [a, b] = self.mix;
        (a.mul_add(m1, b * m2), b.mul_add(m1, a * m2))
    }
}
