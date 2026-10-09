//! The one IIR shape the modelled mics need: a first-order corner move.
//!
//! `H(z) = g (1 − z₀z⁻¹) / (1 − p z⁻¹)`, normalised to unit
//! gain at Nyquist, so it only reshapes the low end. Both the low-cut switch
//! (the mic's built-in high-pass corner moved from its own value to 60, 100
//! or 200 Hz) and proximity (the gradient path's corner moved toward 20 or
//! 500 Hz) are exactly this ratio of two one-pole responses.

use core::f64::consts::PI;

/// An analog corner as a digital pole: bilinear, not prewarped.
///
/// `(1 − a)/(1 + a)` with `a = πf/fs` — what the reference does: its
/// 500 Hz proximity corner measures 500.18 Hz, exactly this map's warp.
#[must_use]
pub fn pole(corner_hz: f64, sample_rate: f64) -> f64 {
    let a = PI * corner_hz.max(0.0) / sample_rate;
    (1.0 - a) / (1.0 + a)
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
