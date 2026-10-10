//! Measured static curves, read back by cubic Hermite interpolation.
//!
//! Stored as values and slopes on a uniform grid, which is exact for the
//! polynomial region near zero, where a curve's small-signal harmonics live.

use dsp_core::num::{count_to_f64, f64_to_index};

/// `f(u)` from `values`/`slopes` sampled uniformly over `[−umax, umax]`;
/// flat past either end (the curve is on its rail there).
#[must_use]
pub fn hermite(values: &[f64], slopes: &[f64], umax: f64, u: f64) -> f64 {
    let last = values.len().saturating_sub(1);
    let step = 2.0 * umax / count_to_f64(last.max(1));
    let pos = (u + umax) / step;
    if pos <= 0.0 {
        return values.first().copied().unwrap_or(0.0);
    }
    let i = f64_to_index(pos);
    let (Some(&y0), Some(&y1), Some(&s0), Some(&s1)) = (values.get(i), values.get(i.saturating_add(1)), slopes.get(i), slopes.get(i.saturating_add(1))) else {
        return values.last().copied().unwrap_or(0.0);
    };
    let x = pos - count_to_f64(i);
    let (m0, m1) = (s0 * step, s1 * step);
    // Horner form of the cubic Hermite basis
    let d = y1 - y0;
    ((((m0 + m1) - 2.0 * d) * x + (3.0 * d - 2.0 * m0 - m1)) * x + m0) * x + y0
}

/// As [`hermite`], on a grid uniform in `s = asinh(k·u/umax) / asinh(k)` —
/// dense near zero, where a curve bends most for small signals, and sparse
/// out on its rails. `slopes` are `df/ds`.
#[must_use]
pub fn hermite_warped(values: &[f64], slopes: &[f64], umax: f64, k: f64, u: f64) -> f64 {
    let s = libm::asinh(k * u / umax) / libm::asinh(k);
    hermite(values, slopes, 1.0, s)
}
