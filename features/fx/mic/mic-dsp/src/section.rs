//! The one IIR shape the modelled mics need: a first-order corner move.
//!
//! `H(z) = g (1 − z₀z⁻¹) / (1 − p z⁻¹)`, normalised to unit
//! gain at Nyquist, so it only reshapes the low end. Both the low-cut switch
//! (the mic's built-in high-pass corner moved from its own value to 60, 100
//! or 200 Hz) and proximity (the gradient path's corner moved toward 20 or
//! 500 Hz) are exactly this ratio of two one-pole responses.

use core::f64::consts::PI;

/// An analog corner as a digital pole: bilinear, prewarped to the corner.
///
/// `(1 − t)/(1 + t)` with `t = tan(πf/fs)`, so the digital corner lands
/// exactly on `f` — what the reference does (measured against the
/// unwarped map, its 500 Hz proximity corner sits at `(fs/π)·tan(π·500/fs)`
/// = 500.18 Hz).
#[must_use]
pub fn pole(corner_hz: f64, sample_rate: f64) -> f64 {
    let t = (PI * corner_hz.max(0.0) / sample_rate).min(1.5).tan();
    (1.0 - t) / (1.0 + t)
}

/// `(1 − z₀z⁻¹)/(1 − p z⁻¹)` with unit gain at Nyquist.
#[derive(Clone, Copy, Debug, Default)]
pub struct CornerShift {
    b0: f64,
    b1: f64,
    a1: f64,
    x1: f64,
    y1: f64,
}

impl CornerShift {
    /// Identity until [`Self::set`] is called.
    #[must_use]
    pub const fn identity() -> Self {
        Self { b0: 1.0, b1: 0.0, a1: 0.0, x1: 0.0, y1: 0.0 }
    }

    /// Move a corner from `from_hz` to `to_hz`: a zero cancels the old
    /// pole, a pole puts the new one in. State is kept.
    pub fn set(&mut self, from_hz: f64, to_hz: f64, sample_rate: f64) {
        let z0 = pole(from_hz, sample_rate);
        let p = pole(to_hz, sample_rate);
        let g = (1.0 + p) / (1.0 + z0);
        self.b0 = g;
        self.b1 = -g * z0;
        self.a1 = p;
    }

    /// Pass everything through (state kept).
    pub const fn make_identity(&mut self) {
        self.b0 = 1.0;
        self.b1 = 0.0;
        self.a1 = 0.0;
    }

    pub const fn reset(&mut self) {
        self.x1 = 0.0;
        self.y1 = 0.0;
    }

    pub fn process(&mut self, x: f64) -> f64 {
        let y = self.a1.mul_add(self.y1, self.b1.mul_add(self.x1, self.b0 * x));
        self.x1 = x;
        self.y1 = y;
        y
    }
}

/// A general second-order section, transposed direct form II, `a₀ = 1`.
#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    s1: f64,
    s2: f64,
}

impl Default for Biquad {
    fn default() -> Self {
        Self::identity()
    }
}

impl Biquad {
    #[must_use]
    pub const fn identity() -> Self {
        Self { b: [1.0, 0.0, 0.0], a: [0.0, 0.0], s1: 0.0, s2: 0.0 }
    }

    /// First-order high-pass, prewarped: `(1+p)/2 · (1 − z⁻¹)/(1 − p z⁻¹)`.
    #[must_use]
    pub fn high_pass(corner_hz: f64, sample_rate: f64) -> Self {
        let p = pole(corner_hz, sample_rate);
        let g = f64::midpoint(1.0, p);
        Self { b: [g, -g, 0.0], a: [-p, 0.0], s1: 0.0, s2: 0.0 }
    }

    /// Second-order high-pass, bilinear prewarped to the corner, with `q`.
    #[must_use]
    pub fn high_pass2(corner_hz: f64, q: f64, sample_rate: f64) -> Self {
        // H(s) = s² / (s² + s·w/q + w²) with s = (1 − z⁻¹)/(1 + z⁻¹),
        // w = tan(π f / fs).
        let w = (core::f64::consts::PI * corner_hz.max(0.0) / sample_rate).min(1.5).tan();
        let k = w / q.abs().max(1e-6);
        let ww = w * w;
        let a0 = 1.0 + k + ww;
        Self {
            b: [1.0 / a0, -2.0 / a0, 1.0 / a0],
            a: [2.0 * (ww - 1.0) / a0, (1.0 - k + ww) / a0],
            s1: 0.0,
            s2: 0.0,
        }
    }

    /// Peaking EQ (the RBJ cookbook form): `gain_db` at `centre_hz`,
    /// bandwidth set by `q`.
    #[must_use]
    pub fn peak(centre_hz: f64, q: f64, gain_db: f64, sample_rate: f64) -> Self {
        let amp = 10f64.powf(gain_db / 40.0);
        let w0 = core::f64::consts::TAU * centre_hz / sample_rate;
        let alpha = w0.sin() / (2.0 * q);
        let cos = w0.cos();
        let a0 = alpha.mul_add(1.0 / amp, 1.0);
        Self {
            b: [alpha.mul_add(amp, 1.0) / a0, -2.0 * cos / a0, alpha.mul_add(-amp, 1.0) / a0],
            a: [-2.0 * cos / a0, alpha.mul_add(-1.0 / amp, 1.0) / a0],
            s1: 0.0,
            s2: 0.0,
        }
    }

    /// The section's inverse (numerator and denominator swapped), with
    /// zeros on z = 1 — a high-pass's — pulled in to `1 − leak` so the
    /// inverse stays stable: exact above a fraction of a hertz, bounded
    /// below.
    #[must_use]
    pub fn inverse(self, leak: f64) -> Self {
        let [b0, b1, b2] = self.b;
        let [a1, a2] = self.a;
        // b(z) = b0 (1 − r1 z⁻¹)(1 − r2 z⁻¹): only the DC zeros (r = 1,
        // b0 + b1 + b2 = 0 for one, and b1 = −2 b0, b2 = b0 for two) need
        // moving; replace each by 1 − leak.
        let r = 1.0 - leak;
        let (d1, d2) = if (b2 - b0).abs() < 1e-12 * b0.abs() && 2.0f64.mul_add(b0, b1).abs() < 1e-12 * b0.abs() {
            (-2.0 * r, r * r)
        } else if b2 == 0.0 && (b0 + b1).abs() < 1e-12 * b0.abs() {
            (-r, 0.0)
        } else {
            (b1 / b0, b2 / b0)
        };
        Self { b: [1.0 / b0, a1 / b0, a2 / b0], a: (d1, d2).into(), s1: 0.0, s2: 0.0 }
    }

    pub const fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }

    pub fn process(&mut self, x: f64) -> f64 {
        let [b0, b1, b2] = self.b;
        let [a1, a2] = self.a;
        let y = b0.mul_add(x, self.s1);
        self.s1 = a1.mul_add(-y, b1.mul_add(x, self.s2));
        self.s2 = a2.mul_add(-y, b2 * x);
        y
    }
}
