//! Bluebox — a tube harmonics generator after a well-known boutique
//! two-tube "mix bus" processor and its plug-in.
//!
//! A clean-room recreation from black-box measurements of the plug-in and
//! its public manual (`saturate/spec/hg2-reference.md`): the same controls
//! doing the same things, aimed at sounding the same rather than nulling.
//! The measured constants live in [`crate::bluebox_tables`].
//!
//! ```text
//!  x ─ ×input ─ HP ─┬──────────────────────┬─ + ─ ×pentode·density ─ P(·) ─ T_triode(·) ─────┐
//!                   └ band(freq) ─ S(·) ─ ×sat ┘                                              │
//!   y ─ mix ─ ×output ─ air ─ cal ─ ×makeup/density ───────────────────────────────────────────┘
//! ```
//!
//! - Two tube stages in series, as the manual says. The pentode's knob is
//!   gain into its curve (so it pushes the triode too); its curve is
//!   measured, even-order at first. The triode's knob *reshapes* its stage
//!   rather than just driving it — measured as a separate curve at knob 50,
//!   75 and 100 (linear at 25) and crossfaded between them. The triode
//!   clips at a fixed absolute level, which is why every drive setting
//!   meets the same ceiling.
//! - The knobs' laws are measured tapers (Triode and Output share one pot
//!   law; Pentode has its own).
//! - Saturation is a parallel path into the pentode: a band filter chosen
//!   by Sat Freq (Low / Flat / High), its own tube curve (Alt Tube: a
//!   hotter voicing with more gain), and a level from the Saturation knob.
//! - The stages are stored as measured, operating point included (the
//!   pentode is strongly one-sided — a single-ended stage near cut-off —
//!   which is where its even harmonics come from); the output coupling
//!   blocks the DC they hand on.
//! - Density drives both stages harder and gives the level back after
//!   them. Cal and Air are high shelves after the tubes. Mix is linear.
//! - The two channels are independent and identical.

use core::f64::consts::PI;

use crate::bluebox_tables as t;

/// Saturation Frequency.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SatFreq {
    Low,
    #[default]
    Flat,
    High,
}

/// Cal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Cal {
    Dark,
    #[default]
    Norm,
    Bright,
}

/// The reference's controls, in its own units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlueboxParams {
    /// 0..=100.
    pub pentode: f64,
    /// 0..=100.
    pub triode: f64,
    pub saturation_in: bool,
    /// 0..=100.
    pub saturation: f64,
    pub saturation_freq: SatFreq,
    pub alt_tube: bool,
    pub air_in: bool,
    /// 0..=100.
    pub air: f64,
    pub cal: Cal,
    /// −100..=100.
    pub density: f64,
    /// −12..=12 dB.
    pub input_db: f64,
    /// 0..=100 (50 is unity).
    pub output: f64,
    /// 0..=100 %.
    pub mix: f64,
}

impl Default for BlueboxParams {
    fn default() -> Self {
        Self {
            pentode: 50.0,
            triode: 50.0,
            saturation_in: false,
            saturation: 50.0,
            saturation_freq: SatFreq::Flat,
            alt_tube: false,
            air_in: false,
            air: 50.0,
            cal: Cal::Norm,
            density: 0.0,
            input_db: 0.0,
            output: 50.0,
            mix: 100.0,
        }
    }
}

/// A measured knob law: dB at evenly spaced knob positions over `lo..=hi`,
/// linearly interpolated.
fn taper(table: &[f64], lo: f64, hi: f64, v: f64) -> f64 {
    let last = table.len().saturating_sub(1);
    let pos = ((v - lo) / (hi - lo)).clamp(0.0, 1.0) * dsp_core::num::count_to_f64(last);
    let i = dsp_core::num::f64_to_index(libm::floor(pos)).min(last.saturating_sub(1));
    let frac = pos - dsp_core::num::count_to_f64(i);
    let a = table.get(i).copied().unwrap_or(0.0);
    let b = table.get(i.saturating_add(1)).copied().unwrap_or(a);
    (b - a) * frac + a
}

fn db(x: f64) -> f64 {
    libm::pow(10.0, x / 20.0)
}

/// A tube stage's curve: polynomial core into an asymmetric soft rail.
/// The input is held inside the core's monotonic range, so driving harder
/// only ever pins the stage on its rail.
#[derive(Clone, Copy, Debug)]
pub struct Curve {
    a2: f64,
    a3: f64,
    rail_pos: f64,
    rail_neg: f64,
    knee: f64,
    u_lo: f64,
    u_hi: f64,
}

impl Curve {
    /// From the fitted `[a2, a3, ln rail+, ln rail−, ln knee]`.
    #[must_use]
    pub fn new(p: [f64; 5]) -> Self {
        let [a2, a3, lp, ln, lk] = p;
        // turning points of z = u + a2 u² + a3 u³ (where dz/du = 0)
        let (mut u_lo, mut u_hi) = (-1e9_f64, 1e9_f64);
        if a3 != 0.0 {
            let disc = 4.0 * a2 * a2 - 12.0 * a3;
            if disc >= 0.0 {
                let r = libm::sqrt(disc);
                for root in [(-2.0 * a2 + r) / (6.0 * a3), (-2.0 * a2 - r) / (6.0 * a3)] {
                    if root > 0.0 {
                        u_hi = u_hi.min(root);
                    } else {
                        u_lo = u_lo.max(root);
                    }
                }
            }
        } else if a2 != 0.0 {
            let root = -0.5 / a2;
            if root > 0.0 { u_hi = root } else { u_lo = root }
        }
        Self { a2, a3, rail_pos: libm::exp(lp), rail_neg: libm::exp(ln), knee: libm::exp(lk), u_lo, u_hi }
    }

    #[inline]
    #[must_use]
    pub fn shape(&self, u: f64) -> f64 {
        let u = u.clamp(self.u_lo, self.u_hi);
        let z = u + self.a2 * u * u + self.a3 * u * u * u;
        let rail = if z >= 0.0 { self.rail_pos } else { self.rail_neg };
        z / libm::pow(1.0 + libm::pow(libm::fabs(z / rail), self.knee), 1.0 / self.knee)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Biquad {
    c: [f64; 5],
    z1: f64,
    z2: f64,
}

impl Biquad {
    const IDENTITY: [f64; 5] = [1.0, 0.0, 0.0, 0.0, 0.0];

    #[inline]
    fn run(&mut self, x: f64) -> f64 {
        let [b0, b1, b2, a1, a2] = self.c;
        let y = b0 * x + self.z1;
        self.z1 = b1 * x - a1 * y + self.z2;
        self.z2 = b2 * x - a2 * y;
        y
    }

    const fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    /// Analog `(n2 s² + n1 s + n0) / (s² + d1 s + d0)` by the bilinear
    /// transform, pre-warped at `warp_hz`.
    fn analog(n: [f64; 3], d: [f64; 2], warp_hz: f64, sr: f64) -> [f64; 5] {
        let w = 2.0 * PI * warp_hz.min(0.45 * sr);
        let k = w / libm::tan(w / (2.0 * sr));
        let [n2, n1, n0] = n;
        let [d1, d0] = d;
        let k2 = k * k;
        let a0 = k2 + d1 * k + d0;
        [
            (n2 * k2 + n1 * k + n0) / a0,
            2.0 * (n0 - n2 * k2) / a0,
            (n2 * k2 - n1 * k + n0) / a0,
            2.0 * (d0 - k2) / a0,
            (k2 - d1 * k + d0) / a0,
        ]
    }

    fn lowpass(hz: f64, q: f64, sr: f64) -> [f64; 5] {
        let w = 2.0 * PI * hz;
        Self::analog([0.0, 0.0, w * w], [w / q, w * w], hz, sr)
    }

    fn highpass(hz: f64, q: f64, sr: f64) -> [f64; 5] {
        let w = 2.0 * PI * hz;
        Self::analog([1.0, 0.0, 0.0], [w / q, w * w], hz, sr)
    }

    /// A second-order high shelf, `gain_db` above `hz`.
    fn high_shelf(hz: f64, q: f64, gain_db: f64, sr: f64) -> [f64; 5] {
        let w = 2.0 * PI * hz;
        let amp = libm::pow(10.0, gain_db / 40.0);
        let root = libm::sqrt(amp);
        // A·(A s² + √A w/q s + w²) / (s² + √A w/q s + A w²)
        let num = [amp * amp, amp * root * w / q, amp * w * w];
        let den = [root * w / q, amp * w * w];
        Self::analog(num, den, hz, sr)
    }
}

/// The triode stage at measured position `i` (0: linear, 1..=3: knob 50, 75,
/// 100).
fn triode_at(i: usize, v: f64) -> f64 {
    use crate::table::hermite_warped as h;
    match i {
        1 => h(&t::TRIODE_50, &t::TRIODE_50_SLOPE, t::TRIODE_50_UMAX, t::WARP, v),
        2 => h(&t::TRIODE_75, &t::TRIODE_75_SLOPE, t::TRIODE_75_UMAX, t::WARP, v),
        3 => h(&t::TRIODE_100, &t::TRIODE_100_SLOPE, t::TRIODE_100_UMAX, t::WARP, v),
        _ => v,
    }
}

/// Positions `lo` and `lo + 1` crossfaded by `w`.
fn triode(lo: usize, w: f64, v: f64) -> f64 {
    let a = triode_at(lo, v);
    if w <= 0.0 { a } else { a + w * (triode_at(lo.saturating_add(1), v) - a) }
}

/// The static picture of the two tube stages, for a display.
///
/// For an input `x` in −1..1 (full scale): the pentode at its knob into the
/// triode at its, operating point taken off, normalised to the swing.
#[must_use]
pub fn static_curve(p: &BlueboxParams, x: f64) -> f64 {
    let fx = {
        let mut fx = Bluebox::new(48_000.0);
        fx.set(*p);
        fx
    };
    let stage = |u: f64| {
        let pentode = crate::table::hermite_warped(&t::PENTODE, &t::PENTODE_SLOPE, t::PENTODE_UMAX, t::WARP, fx.g_p * u);
        triode(fx.triode_lo, fx.triode_w, pentode) * fx.triode_level - fx.rest / fx.makeup.max(1e-9)
    };
    // normalise to the swing at full scale, so the picture is the shape
    let swing = stage(1.0).abs().max(stage(-1.0).abs()).max(1e-9);
    stage(x) / swing
}

/// One channel.
#[derive(Clone, Debug, Default)]
struct Channel {
    hp_x: f64,
    hp_y: f64,
    dc_x: f64,
    dc_y: f64,
    /// The tubes run at 4×: the main signal and the saturation band are
    /// interpolated separately and meet at the high rate.
    os_main: crate::oversample::Oversample4,
    os_band: crate::oversample::Oversample4,
    band: [Biquad; 2],
    cal: Biquad,
    air: Biquad,
}

impl Channel {
    fn reset(&mut self) {
        self.hp_x = 0.0;
        self.hp_y = 0.0;
        self.dc_x = 0.0;
        self.dc_y = 0.0;
        self.os_main.reset();
        self.os_band.reset();
        for b in &mut self.band {
            b.reset();
        }
        self.cal.reset();
        self.air.reset();
    }
}

/// The stereo Bluebox engine.
#[derive(Clone, Debug)]
pub struct Bluebox {
    sample_rate: f64,
    ch: [Channel; 2],
    sat_curve: [Curve; 2],
    // derived from params
    input: f64,
    /// Gain into the pentode stage (knob law and Density).
    g_p: f64,
    /// Triode: which pair of measured curves, the crossfade between them,
    /// and the level below knob 25 (where the stage is linear).
    triode_lo: usize,
    triode_w: f64,
    triode_level: f64,
    /// Density's level given back after the stages.
    makeup: f64,
    /// The stages' output at rest (no signal), taken off before the output
    /// coupling so a knob move never steps the operating point through it.
    rest: f64,
    /// Saturation knob level (0..1) times the band's path gain, divided by
    /// the drive into the curve (so the small-signal path gain is measured).
    sat_out: f64,
    /// Drive into the saturation tube.
    sat_in: f64,
    sat_alt: usize,
    sat_on: bool,
    air_on: bool,
    output: f64,
    wet: f64,
    hp_b: f64,
    hp_a: f64,
}

impl Bluebox {
    #[must_use]
    pub fn new(sample_rate: f64) -> Self {
        let k = libm::tan(PI * t::INPUT_HP_HZ / sample_rate);
        let mut s = Self {
            sample_rate,
            ch: [Channel::default(), Channel::default()],
            sat_curve: [Curve::new(t::SAT_CURVE), Curve::new(t::SAT_CURVE_ALT)],
            input: 1.0,
            g_p: 1.0,
            triode_lo: 1,
            triode_w: 0.0,
            triode_level: 1.0,
            makeup: 1.0,
            rest: 0.0,
            sat_out: 0.0,
            sat_in: 1.0,
            sat_alt: 0,
            sat_on: false,
            air_on: false,
            output: 1.0,
            wet: 1.0,
            hp_b: 1.0 / (1.0 + k),
            hp_a: (1.0 - k) / (1.0 + k),
        };
        s.set(BlueboxParams::default());
        s
    }

    #[must_use]
    pub const fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    pub fn set(&mut self, p: BlueboxParams) {
        let sr = self.sample_rate;
        // a closed Pentode/Triode/Output knob is silence, as on the reference
        let open = |v: f64| if v <= 0.0 { 0.0 } else { 1.0 };
        self.input = db(p.input_db.clamp(-12.0, 12.0));
        let dens = p.density.clamp(-100.0, 100.0);
        let dens_drive = taper(&t::DENSITY_DRIVE_DB, -100.0, 100.0, dens);
        let comp = taper(&t::DENSITY_COMP_DB, -100.0, 100.0, dens);
        self.g_p = open(p.pentode) * db(taper(&t::PENTODE_TAPER_DB, 0.0, 100.0, p.pentode) + dens_drive);
        // the triode knob reshapes the stage: measured curves at 50, 75 and
        // 100 (index 1..=3), linear at 25 (index 0); a level below 25
        let tk = p.triode.clamp(0.0, 100.0);
        self.triode_level = if tk < 25.0 { open(tk) * db(taper(&t::TRIODE_TAPER_DB, 0.0, 100.0, tk)) } else { 1.0 };
        let pos = ((tk - 25.0) / 25.0).clamp(0.0, 3.0);
        self.triode_lo = dsp_core::num::f64_to_index(libm::floor(pos)).min(2);
        self.triode_w = pos - dsp_core::num::count_to_f64(self.triode_lo);
        self.makeup = db(comp);
        let idle = crate::table::hermite_warped(&t::PENTODE, &t::PENTODE_SLOPE, t::PENTODE_UMAX, t::WARP, 0.0);
        self.rest = triode(self.triode_lo, self.triode_w, idle) * self.triode_level * self.makeup;
        self.output = open(p.output) * db(taper(&t::OUTPUT_TAPER_DB, 0.0, 100.0, p.output));
        self.wet = (p.mix / 100.0).clamp(0.0, 1.0);
        // saturation path
        self.sat_on = p.saturation_in;
        self.sat_alt = usize::from(p.alt_tube);
        let band: usize = match p.saturation_freq {
            SatFreq::Low => 0,
            SatFreq::Flat => 1,
            SatFreq::High => 2,
        };
        let row = t::SAT_BAND.get(band.saturating_mul(2).saturating_add(self.sat_alt)).copied().unwrap_or([1.0, 20_000.0, 0.7, 0.0, 0.7]);
        let [gain, lp_hz, lp_q, hp_hz, hp_q] = row;
        let drive = t::SAT_DRIVE.get(self.sat_alt).copied().unwrap_or(1.0);
        self.sat_in = drive;
        self.sat_out = taper(&t::SAT_LEVEL, 0.0, 100.0, p.saturation) * gain / drive;
        let lp = Biquad::lowpass(lp_hz, lp_q, sr);
        let hp = if hp_hz > 0.0 { Biquad::highpass(hp_hz, hp_q, sr) } else { Biquad::IDENTITY };
        let cal = match p.cal {
            Cal::Dark => Biquad::high_shelf(t::CAL_DARK[0], t::CAL_DARK[1], t::CAL_DARK[2], sr),
            Cal::Norm => Biquad::IDENTITY,
            Cal::Bright => Biquad::high_shelf(t::CAL_BRIGHT[0], t::CAL_BRIGHT[1], t::CAL_BRIGHT[2], sr),
        };
        self.air_on = p.air_in;
        let air = Biquad::high_shelf(
            libm::exp(taper(&t::AIR_LN_HZ, 0.0, 100.0, p.air)),
            taper(&t::AIR_Q, 0.0, 100.0, p.air),
            taper(&t::AIR_DB, 0.0, 100.0, p.air),
            sr,
        );
        for c in &mut self.ch {
            c.band[0].c = lp;
            c.band[1].c = hp;
            c.cal.c = cal;
            c.air.c = air;
        }
    }

    pub fn reset(&mut self) {
        for c in &mut self.ch {
            c.reset();
        }
    }

    #[inline]
    fn channel(&mut self, index: usize, dry: f64) -> f64 {
        let (hb, ha) = (self.hp_b, self.hp_a);
        let Some(ch) = self.ch.get_mut(index) else { return dry };
        let driven = dry * self.input;
        let main = hb * (driven - ch.hp_x) + ha * ch.hp_y;
        ch.hp_x = driven;
        ch.hp_y = main;
        let (g_p, lo, w, level, makeup, rest) = (self.g_p, self.triode_lo, self.triode_w, self.triode_level, self.makeup, self.rest);
        let stages = |into: f64| {
            let pentode = crate::table::hermite_warped(&t::PENTODE, &t::PENTODE_SLOPE, t::PENTODE_UMAX, t::WARP, g_p * into);
            triode(lo, w, pentode) * level * makeup - rest
        };
        let stages = if self.sat_on {
            let banded = ch.band.iter_mut().fold(main, |s, b| b.run(s));
            let (sat_in, sat_out) = (self.sat_in, self.sat_out);
            let tube = self.sat_curve.get(self.sat_alt).copied();
            ch.os_main.process2(&mut ch.os_band, main, banded, |m, b| {
                stages(m + tube.map_or(0.0, |c| sat_out * c.shape(sat_in * b)))
            })
        } else {
            ch.os_main.process(main, stages)
        };
        // the stages hand on their operating point; the output coupling
        // blocks it (same corner as the input transformer)
        let y = hb * (stages - ch.dc_x) + ha * ch.dc_y;
        ch.dc_x = stages;
        ch.dc_y = y;
        let y = ch.cal.run(y);
        let y = if self.air_on { ch.air.run(y) } else { y };
        let y = y * self.output;
        y * self.wet + dry * (1.0 - self.wet)
    }

    /// One stereo frame.
    pub fn process(&mut self, left: f64, right: f64) -> (f64, f64) {
        (self.channel(0, left), self.channel(1, right))
    }
}
