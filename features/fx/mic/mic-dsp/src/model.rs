//! A modelled mic as data: its measured responses and the few numbers
//! that describe how its controls move them.
//!
//! The responses come from a measurement of the reference (see
//! `signal-analyzer`'s `sphere_capture`): for each of the nine pattern
//! steps and each of five axis anchors (0, 45, 90, 135, 180°), the two
//! kernels `P` and `G` with `out = P∗(front + rear) + G∗(front − rear)` —
//! the pressure and gradient parts. Axis positions between anchors are a
//! linear crossfade of the two neighbouring anchors, which is exactly what
//! the reference does.
//!
//! The low-cut switch is, for most mics, an exact first-order move of the
//! mic's built-in high-pass corner; for the rest it changes the response in
//! ways no post-EQ reproduces, and those positions carry their own set of
//! anchors.
//!
//! Asset format (`.micm`, little-endian), version 4:
//!
//! ```text
//! b"MICM" u32 version=4 u32 sample_rate u32 taps u32 patterns=9 u32 axes=5
//! f64 low_cut_hz[4]          built-in corner, then the three switch positions
//! f64 proximity_f0[9]        the mic's own gradient corner at each pattern step
//! u32 proximity_law          0 = Corner, 1 = Shelf, 2 = Dynamic
//! f64 proximity_params[4]    Corner: lo hi · Shelf: lo hi k · Dynamic: up down
//! u32 poly_len, f64 poly[]   the output stage's curve: s + poly[0] s² + poly[1] s³ + …
//! u32 post_len, f64 post_hz[] first-order high-passes after the curve (prewarped)
//! u32 kernel_sets            bit k: low-cut position k has its own anchor set
//!                            (bit 0, the switch off, is always set)
//! for each set bit k, ascending: patterns × axes × ([P; taps] [G; taps]) f32
//!                            (the measured response, curve's high-passes
//!                            included; the engine derives the curve's input)
//! ```

/// Pattern steps, Omni (0) to Figure-8 (8).
pub const PATTERNS: usize = 9;
/// Axis anchors, every 45° from 0 to 180.
pub const AXES: usize = 5;
/// Degrees between axis anchors.
pub const AXIS_STEP_DEG: f64 = 45.0;
/// Low-cut positions: off and three switch settings.
pub const LOW_CUTS: usize = 4;

/// How the proximity control moves a mic's gradient-path response.
///
/// Every law is one or two first-order corner moves relative to the
/// setting of 0 %, with `f0` the mic's own corner at the current pattern
/// step and `u` the setting as −1 … +1. Each move is `(zero, pole)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProximityLaw {
    /// Condensers and ribbons: the pole moves from `f0` toward `lo` (20 Hz)
    /// linearly as `u` rises to +1, toward `hi` (500 Hz) with `u²` as it
    /// falls to −1.
    Corner { f0: [f64; PATTERNS], lo: f64, hi: f64 },
    /// As [`Self::Corner`], but the gradient path is a shelf rather than a
    /// high-pass: a zero follows the pole at `k` times its frequency.
    Shelf { f0: [f64; PATTERNS], lo: f64, hi: f64, k: f64 },
    /// Dynamics: raising proximity moves the pole down geometrically to
    /// `f0 / up`; lowering it slides a zero down from `f0` to
    /// `f0 / down` with `u²` — a deepening low shelf.
    Dynamic { f0: [f64; PATTERNS], up: f64, down: f64 },
}

/// One `(zero, pole)` corner move, in Hz.
pub type Move = (f64, f64);

impl ProximityLaw {
    /// The corner moves for a proximity setting in percent.
    #[must_use]
    pub fn moves(self, percent: f64, pattern: usize) -> [Option<Move>; 2] {
        let u = (percent / 100.0).clamp(-1.0, 1.0);
        let corner = |f0: f64, lo: f64, hi: f64| if u >= 0.0 { (f0 - lo).mul_add(-u, f0) } else { (hi - f0).mul_add(u * u, f0) };
        match self {
            Self::Corner { f0, lo, hi } => {
                let f0 = f0.get(pattern).copied().unwrap_or(100.0);
                [Some((f0, corner(f0, lo, hi))), None]
            }
            Self::Shelf { f0, lo, hi, k } => {
                let f0 = f0.get(pattern).copied().unwrap_or(100.0);
                let p = corner(f0, lo, hi);
                [Some((f0, p)), Some((k * p, k * f0))]
            }
            Self::Dynamic { f0, up, down } => {
                let f0 = f0.get(pattern).copied().unwrap_or(100.0);
                if u >= 0.0 { [Some((f0, f0 * up.powf(-u))), None] } else { [Some((f0 * down.powf(-u * u), f0)), None] }
            }
        }
    }
}

/// One modelled mic.
#[derive(Clone, Debug)]
pub struct MicModel {
    pub sample_rate: f64,
    pub taps: usize,
    /// One anchor set per low-cut position that has its own; `None` means
    /// the position is the off set plus a corner move.
    sets: [Option<Vec<f32>>; LOW_CUTS],
    /// The built-in high-pass corner (Filter off) and the three switch
    /// positions it moves to.
    pub low_cut_hz: [f64; LOW_CUTS],
    pub proximity: ProximityLaw,
    /// The output stage's curve: `y = s + poly[0] s² + poly[1] s³ + …`.
    pub poly: [f64; MAX_POLY],
    /// First-order high-pass corners after the curve.
    pub post_hz: Vec<f64>,
}

/// Highest curve coefficient stored (`s⁷`).
pub const MAX_POLY: usize = 6;
/// Most high-passes after the curve.
pub const MAX_POST: usize = 4;

/// Why an asset was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelError {
    BadMagic,
    BadVersion,
    BadShape,
    Truncated,
}

/// A little-endian reader over the asset.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], ModelError> {
        let end = self.at.checked_add(N).ok_or(ModelError::Truncated)?;
        let s = self.bytes.get(self.at..end).ok_or(ModelError::Truncated)?;
        self.at = end;
        let mut out = [0u8; N];
        out.copy_from_slice(s);
        Ok(out)
    }

    fn u32(&mut self) -> Result<u32, ModelError> {
        self.take::<4>().map(u32::from_le_bytes)
    }

    fn f64(&mut self) -> Result<f64, ModelError> {
        self.take::<8>().map(f64::from_le_bytes)
    }

    fn f32s(&mut self, count: usize) -> Result<Vec<f32>, ModelError> {
        let len = count.checked_mul(4).ok_or(ModelError::BadShape)?;
        let end = self.at.checked_add(len).ok_or(ModelError::Truncated)?;
        let s = self.bytes.get(self.at..end).ok_or(ModelError::Truncated)?;
        self.at = end;
        Ok(s.chunks_exact(4).map(|c| f32::from_le_bytes([c.first().copied().unwrap_or(0), c.get(1).copied().unwrap_or(0), c.get(2).copied().unwrap_or(0), c.get(3).copied().unwrap_or(0)])).collect())
    }
}

impl MicModel {
    /// Parse a `.micm` asset.
    ///
    /// # Errors
    /// The asset is not version 4 of the format, does not have the
    /// 9 × 5 layout, or is shorter than its header says.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ModelError> {
        let mut r = Reader { bytes, at: 0 };
        if r.take::<4>()? != *b"MICM" {
            return Err(ModelError::BadMagic);
        }
        if r.u32()? != 4 {
            return Err(ModelError::BadVersion);
        }
        let sample_rate = f64::from(r.u32()?);
        let taps = dsp_core::u32_to_index(r.u32()?);
        if dsp_core::u32_to_index(r.u32()?) != PATTERNS || dsp_core::u32_to_index(r.u32()?) != AXES {
            return Err(ModelError::BadShape);
        }
        let mut low_cut_hz = [0.0; LOW_CUTS];
        for v in &mut low_cut_hz {
            *v = r.f64()?;
        }
        let mut f0 = [0.0; PATTERNS];
        for v in &mut f0 {
            *v = r.f64()?;
        }
        let law = r.u32()?;
        let mut params = [0.0; 4];
        for v in &mut params {
            *v = r.f64()?;
        }
        let [first, second, third, _] = params;
        let proximity = match law {
            0 => ProximityLaw::Corner { f0, lo: first, hi: second },
            1 => ProximityLaw::Shelf { f0, lo: first, hi: second, k: third },
            2 => ProximityLaw::Dynamic { f0, up: first, down: second },
            _ => return Err(ModelError::BadShape),
        };
        let mut poly = [0.0; MAX_POLY];
        let poly_len = dsp_core::u32_to_index(r.u32()?);
        if poly_len > MAX_POLY {
            return Err(ModelError::BadShape);
        }
        for v in poly.iter_mut().take(poly_len) {
            *v = r.f64()?;
        }
        let mut post_hz = [0.0; MAX_POST];
        let post_len = dsp_core::u32_to_index(r.u32()?);
        if post_len > MAX_POST {
            return Err(ModelError::BadShape);
        }
        for v in post_hz.iter_mut().take(post_len) {
            *v = r.f64()?;
        }
        let mask = r.u32()? | 1;
        let count = PATTERNS
            .checked_mul(AXES)
            .and_then(|n| n.checked_mul(2))
            .and_then(|n| n.checked_mul(taps))
            .ok_or(ModelError::BadShape)?;
        let mut sets: [Option<Vec<f32>>; LOW_CUTS] = Default::default();
        let mut bit = 1u32;
        for set in &mut sets {
            if mask & bit != 0 {
                *set = Some(r.f32s(count)?);
            }
            bit = bit.wrapping_shl(1);
        }
        Ok(Self { sample_rate, taps, sets, low_cut_hz, proximity, poly, post_hz: post_hz.into_iter().take(post_len).collect() })
    }

    /// Whether a low-cut position has its own anchors (otherwise it is the
    /// off set with the built-in corner moved).
    #[must_use]
    pub fn has_set(&self, low_cut: usize) -> bool {
        self.sets.get(low_cut).is_some_and(Option::is_some)
    }

    /// The `(P, G)` kernels at one low-cut position, pattern step and axis
    /// anchor (the off set where the position has none of its own).
    #[must_use]
    pub fn anchor(&self, low_cut: usize, pattern: usize, axis: usize) -> Option<(&[f32], &[f32])> {
        let set = self.sets.get(low_cut).and_then(Option::as_ref).or_else(|| self.sets.first().and_then(Option::as_ref))?;
        let index = pattern.checked_mul(AXES)?.checked_add(axis)?;
        let pair = set.chunks_exact(self.taps.checked_mul(2)?).nth(index)?;
        Some(pair.split_at(self.taps))
    }
}
