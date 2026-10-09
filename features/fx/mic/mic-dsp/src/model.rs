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
//! Asset format (`.micm`, little-endian), version 5:
//!
//! ```text
//! b"MICM" u32 version=5 u32 sample_rate u32 taps u32 patterns=9 u32 axes=5
//! f64 low_cut_hz[4]          built-in corner, then the three switch positions
//! f64 proximity_f0[9]        the mic's own gradient corner at each pattern step
//! u32 proximity_law          0 = Corner, 1 = Shelf, 2 = Dynamic
//! f64 proximity_params[4]    Corner: lo hi · Shelf: lo hi k · Dynamic: up down cap
//! 4 × output stage, one per low-cut position:
//!   u32 present (0: use position 0's)
//!   u32 poly_len, f64 poly[]  the curve: s + poly[0] s² + poly[1] s³ + …
//!   u32 n, n × (u32 kind, f64 p[3])  sections after the curve:
//!                             kind 0 high-pass (p0 = Hz), 1 peak (Hz, Q, dB),
//!                             2 second-order high-pass (Hz, Q),
//!                             3 the curve's input clamp (p0, not a filter)
//! u32 kernel_sets            bit k: low-cut position k has its own anchor set
//!                            (bit 0, the switch off, is always set)
//! for each set bit k, ascending: patterns × axes × ([P; taps] [G; taps]) f32
//! f64 delay                  (optional) the model's own fractional delay
//! f64 shelf_k[9]             (optional, Shelf law only) its k at each pattern step
//! f64 dual_weight[9]         (optional) the weight dual mode gives this model's
//!                            proximity filter when crossfading it with another's
//! f64 dual_pressure_hz       (optional) the pressure path's high-pass corner dual
//!                            mode crossfades (same weight; 0 = none, default 10)
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
    Shelf { f0: [f64; PATTERNS], lo: f64, hi: f64, k: [f64; PATTERNS] },
    /// Dynamics: raising proximity moves the pole down geometrically to
    /// `f0 / up`; lowering it slides a zero down from `f0` to
    /// `f0 / down` with `u²` — a deepening low shelf.
    ///
    /// Lowering it is a shelf `down^(u²)` deep that first raises the pole
    /// from `f0` toward `cap` (Hz); once the pole reaches `cap` it stays
    /// there and a second section's zero slides down from it. `cap` at or
    /// below `f0` (0 in older files) means the zero moves from the start.
    Dynamic { f0: [f64; PATTERNS], up: f64, down: f64, cap: f64 },
}

/// One `(zero, pole)` corner move, in Hz.
pub type Move = (f64, f64);

impl ProximityLaw {
    /// The corner moves for a proximity setting in percent, with the law
    /// evaluated around a `centre` corner and at `law_pattern`'s shelf
    /// depth, while what is cancelled is the kernel's own base (`pattern`'s
    /// corner and depth). Dual mode shares one centre between two mics; the
    /// 180 variant's backward mic takes the forward mic's pattern.
    #[must_use]
    pub fn moves_around(self, percent: f64, pattern: usize, centre: f64, law_pattern: usize) -> [Option<Move>; 2] {
        let own = self.f0(pattern);
        let k_at = |p: usize| match self {
            Self::Shelf { k, .. } => k.get(p).copied().unwrap_or(1.0),
            _ => 1.0,
        };
        let (k_own, k_law) = (k_at(pattern), k_at(law_pattern));
        let shifted = match self {
            Self::Corner { lo, hi, .. } => Self::Corner { f0: [centre; PATTERNS], lo, hi },
            Self::Shelf { lo, hi, .. } => Self::Shelf { f0: [centre; PATTERNS], lo, hi, k: [k_law; PATTERNS] },
            Self::Dynamic { up, down, cap, .. } => Self::Dynamic { f0: [centre; PATTERNS], up, down, cap },
        };
        shifted.moves(percent, law_pattern).map(|step| {
            step.map(|(zero, pole)| {
                // replace the base where it cancels the kernel's own
                let z = if (zero - centre).abs() < 1e-9 { own } else { zero };
                let p = if matches!(self, Self::Shelf { .. }) && k_law.mul_add(-centre, pole).abs() < 1e-9 { k_own * own } else { pole };
                (z, p)
            })
        })
    }

    /// The whole gradient-path proximity filter at a setting, base
    /// included, as `(zero, pole)` sections: a high-pass at the corner
    /// (Corner), a shelf at it (Shelf), or only the moves (Dynamic, whose
    /// base is flat). The kernels carry the filter at 0 %.
    #[must_use]
    pub fn filter(self, percent: f64, pattern: usize) -> [Option<Move>; 2] {
        let u = (percent / 100.0).clamp(-1.0, 1.0);
        let corner = |f0: f64, lo: f64, hi: f64| if u >= 0.0 { (f0 - lo).mul_add(-u, f0) } else { (hi - f0).mul_add(u * u, f0) };
        match self {
            Self::Corner { lo, hi, .. } => [Some((0.0, corner(self.f0(pattern), lo, hi))), None],
            Self::Shelf { lo, hi, k, .. } => {
                let k = k.get(pattern).copied().unwrap_or(1.0);
                let c = corner(self.f0(pattern), lo, hi);
                [Some((k * c, c)), None]
            }
            Self::Dynamic { .. } => self.moves(percent, pattern),
        }
    }

    /// The section that undoes the base the kernels carry at a pattern step
    /// (`leak` Hz stands in for a pole at DC under a high-pass's zero).
    #[must_use]
    pub fn base_inverse(self, pattern: usize, leak: f64) -> Option<Move> {
        let f0 = self.f0(pattern);
        match self {
            Self::Corner { .. } => Some((f0, leak)),
            Self::Shelf { k, .. } => Some((f0, k.get(pattern).copied().unwrap_or(1.0) * f0)),
            Self::Dynamic { .. } => None,
        }
    }

    /// The mic's own corner at a pattern step.
    #[must_use]
    pub fn f0(self, pattern: usize) -> f64 {
        let table = match self {
            Self::Corner { f0, .. } | Self::Shelf { f0, .. } | Self::Dynamic { f0, .. } => f0,
        };
        table.get(pattern).copied().unwrap_or(100.0)
    }

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
                let k = k.get(pattern).copied().unwrap_or(1.0);
                let p = corner(f0, lo, hi);
                [Some((f0, p)), Some((k * p, k * f0))]
            }
            Self::Dynamic { f0, up, down, cap } => {
                let f0 = f0.get(pattern).copied().unwrap_or(100.0);
                let depth = down.powf(u * u);
                if u >= 0.0 {
                    [Some((f0, f0 * up.powf(-u))), None]
                } else if cap <= f0 {
                    [Some((f0 / depth, f0)), None]
                } else if f0 * depth <= cap {
                    [Some((f0, f0 * depth)), None]
                } else {
                    // the shelf's whole depth is f0·z / cap² = 1 / depth
                    [Some((f0, cap)), Some((cap.powi(2) / (f0 * depth), cap))]
                }
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
    /// The output stage per low-cut position (`None`: position 0's).
    stages: [Option<OutputStage>; LOW_CUTS],
    /// The model's own fractional delay in samples (linear interpolation):
    /// 0.146 for every model but Sphere Diffuse (0). Only dual mode sees it.
    pub delay: f64,
    /// Dual mode's weight on this model's proximity filter, per pattern
    /// step (relative to Sphere Linear's; 1 when the file gives none).
    pub dual_weight: [f64; PATTERNS],
    /// Dual mode's pressure-path block: a high-pass at this corner (0: none)
    /// crossfaded with the same weights (Sphere Linear's is 10 Hz).
    pub dual_pressure_hz: f64,
}

/// One section after the output curve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PostSection {
    /// First-order high-pass at this corner (Hz).
    HighPass(f64),
    /// Peaking EQ.
    Peak { hz: f64, q: f64, db: f64 },
    /// Second-order high-pass.
    HighPass2 { hz: f64, q: f64 },
}

/// The mic's output stage: a static curve, then filters.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutputStage {
    /// `y = s + poly[0] s² + poly[1] s³ + …`.
    pub poly: [f64; MAX_POLY],
    pub post: Vec<PostSection>,
    /// The polynomial's input is held to ±this (the reference's curve stops
    /// growing beyond it); infinite when the file gives none.
    pub clamp: f64,
}

/// The fractional delay nearly every model carries (samples).
pub const MODEL_DELAY: f64 = 0.146_03;

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
    /// The asset is not version 5 of the format, does not have the
    /// 9 × 5 layout, or is shorter than its header says.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ModelError> {
        let mut r = Reader { bytes, at: 0 };
        if r.take::<4>()? != *b"MICM" {
            return Err(ModelError::BadMagic);
        }
        if r.u32()? != 5 {
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
            1 => ProximityLaw::Shelf { f0, lo: first, hi: second, k: [third; PATTERNS] },
            2 => ProximityLaw::Dynamic { f0, up: first, down: second, cap: third },
            _ => return Err(ModelError::BadShape),
        };
        let mut stages: [Option<OutputStage>; LOW_CUTS] = Default::default();
        for stage in &mut stages {
            if r.u32()? == 0 {
                continue;
            }
            let mut poly = [0.0; MAX_POLY];
            let poly_len = dsp_core::u32_to_index(r.u32()?);
            if poly_len > MAX_POLY {
                return Err(ModelError::BadShape);
            }
            for v in poly.iter_mut().take(poly_len) {
                *v = r.f64()?;
            }
            let n = dsp_core::u32_to_index(r.u32()?);
            if n > MAX_POST {
                return Err(ModelError::BadShape);
            }
            let mut post = Vec::with_capacity(n);
            let mut clamp = f64::INFINITY;
            for _ in 0..n {
                let kind = r.u32()?;
                let p = [r.f64()?, r.f64()?, r.f64()?];
                post.push(match kind {
                    0 => PostSection::HighPass(p[0]),
                    1 => PostSection::Peak { hz: p[0], q: p[1], db: p[2] },
                    2 => PostSection::HighPass2 { hz: p[0], q: p[1] },
                    3 => {
                        clamp = p[0];
                        continue;
                    }
                    _ => return Err(ModelError::BadShape),
                });
            }
            *stage = Some(OutputStage { poly, post, clamp });
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
        let delay = r.f64().unwrap_or(MODEL_DELAY);
        let mut proximity = proximity;
        if let ProximityLaw::Shelf { k, .. } = &mut proximity {
            let mut table = *k;
            if table.iter_mut().try_for_each(|v| r.f64().map(|x| *v = x)).is_ok() {
                *k = table;
            }
        }
        let mut dual_weight = [1.0; PATTERNS];
        let mut table = dual_weight;
        if table.iter_mut().try_for_each(|v| r.f64().map(|x| *v = x)).is_ok() {
            dual_weight = table;
        }
        let dual_pressure_hz = r.f64().unwrap_or(10.0);
        Ok(Self { sample_rate, taps, sets, low_cut_hz, proximity, stages, delay, dual_weight, dual_pressure_hz })
    }

    /// The output stage at a low-cut position.
    #[must_use]
    pub fn stage(&self, low_cut: usize) -> Option<&OutputStage> {
        self.stages.get(low_cut).and_then(Option::as_ref).or_else(|| self.stages.first().and_then(Option::as_ref))
    }

    /// Whether a low-cut position has its own anchors (otherwise it is the
    /// off set with the built-in corner moved).
    #[must_use]
    pub fn has_set(&self, low_cut: usize) -> bool {
        self.sets.get(low_cut).is_some_and(Option::is_some)
    }

    /// The high-pass the response carries at a low-cut position, as dual
    /// mode crossfades it: the built-in corner (moved by the switch) for a
    /// model whose low cut is a corner move, 10 Hz for one measured with its
    /// own kernels at every position.
    #[must_use]
    pub fn low_end_corner(&self, position: usize) -> f64 {
        let measured = (1..LOW_CUTS).any(|k| self.has_set(k));
        if measured { 10.0 } else { self.low_cut_hz.get(position).copied().unwrap_or(10.0).max(1.0) }
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
