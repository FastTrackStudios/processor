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
//! Asset format (`.micm`, little-endian): `b"MICM"`, `u32` version (1),
//! `u32` sample rate, `u32` taps, `u32` patterns, `u32` axes, then
//! `patterns × axes × [P; taps] [G; taps]` as `f32`.

/// Pattern steps, Omni (0) to Figure-8 (8).
pub const PATTERNS: usize = 9;
/// Axis anchors, every 45° from 0 to 180.
pub const AXES: usize = 5;
/// Degrees between axis anchors.
pub const AXIS_STEP_DEG: f64 = 45.0;

/// How the proximity control moves a mic's gradient-path corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProximityLaw {
    /// Condensers and ribbons: from the mic's own corner `f0` (which may
    /// differ per pattern step), toward 20 Hz linearly as proximity rises
    /// to +100 %, toward 500 Hz with the square of the setting as it falls
    /// to −100 %.
    Corner { f0: [f64; PATTERNS] },
}

impl ProximityLaw {
    /// `(from, to)` corners for a proximity setting in percent.
    #[must_use]
    pub fn corners(self, percent: f64, pattern: usize) -> (f64, f64) {
        match self {
            Self::Corner { f0 } => {
                let f0 = f0.get(pattern).copied().unwrap_or(100.0);
                let u = (percent / 100.0).clamp(-1.0, 1.0);
                let to = if u >= 0.0 { (f0 - 20.0).mul_add(-u, f0) } else { (500.0 - f0).mul_add(u * u, f0) };
                (f0, to)
            }
        }
    }
}

/// One modelled mic.
#[derive(Clone, Debug)]
pub struct MicModel {
    pub sample_rate: f64,
    pub taps: usize,
    /// `PATTERNS × AXES × [P, G]`, `taps` each.
    kernels: Vec<f32>,
    /// The built-in high-pass corner (Filter off) and the three switch
    /// positions it moves to.
    pub low_cut_hz: [f64; 4],
    pub proximity: ProximityLaw,
}

/// Why an asset was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelError {
    BadMagic,
    BadVersion,
    BadShape,
    Truncated,
}

impl MicModel {
    /// Parse a `.micm` asset.
    ///
    /// # Errors
    /// The asset is not version 1 of the format, does not have the
    /// 9 × 5 layout, or is shorter than its header says.
    pub fn from_bytes(
        bytes: &[u8],
        low_cut_hz: [f64; 4],
        proximity: ProximityLaw,
    ) -> Result<Self, ModelError> {
        let word = |i: usize| -> Result<u32, ModelError> {
            let end = i.checked_add(4).ok_or(ModelError::Truncated)?;
            let b = bytes.get(i..end).ok_or(ModelError::Truncated)?;
            Ok(u32::from_le_bytes([
                b.first().copied().unwrap_or(0),
                b.get(1).copied().unwrap_or(0),
                b.get(2).copied().unwrap_or(0),
                b.get(3).copied().unwrap_or(0),
            ]))
        };
        if bytes.get(..4) != Some(b"MICM".as_slice()) {
            return Err(ModelError::BadMagic);
        }
        if word(4)? != 1 {
            return Err(ModelError::BadVersion);
        }
        let sample_rate = f64::from(word(8)?);
        let taps = dsp_core::u32_to_index(word(12)?);
        if dsp_core::u32_to_index(word(16)?) != PATTERNS || dsp_core::u32_to_index(word(20)?) != AXES {
            return Err(ModelError::BadShape);
        }
        let count = PATTERNS
            .checked_mul(AXES)
            .and_then(|n| n.checked_mul(2))
            .and_then(|n| n.checked_mul(taps))
            .ok_or(ModelError::BadShape)?;
        let body = bytes.get(24..).ok_or(ModelError::Truncated)?;
        if body.len() / 4 < count {
            return Err(ModelError::Truncated);
        }
        let kernels = body
            .chunks_exact(4)
            .take(count)
            .map(|c| f32::from_le_bytes([c.first().copied().unwrap_or(0), c.get(1).copied().unwrap_or(0), c.get(2).copied().unwrap_or(0), c.get(3).copied().unwrap_or(0)]))
            .collect();
        Ok(Self { sample_rate, taps, kernels, low_cut_hz, proximity })
    }

    /// The `(P, G)` kernels at one pattern step and axis anchor.
    #[must_use]
    pub fn anchor(&self, pattern: usize, axis: usize) -> Option<(&[f32], &[f32])> {
        let set = pattern.checked_mul(AXES)?.checked_add(axis)?;
        let pair = self.kernels.chunks_exact(self.taps.checked_mul(2)?).nth(set)?;
        Some(pair.split_at(self.taps))
    }
}
