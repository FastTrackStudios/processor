//! Measured decay → RT60 for the engines whose `decay` is a feedback law
//! rather than a time.
//!
//! Room and Hall convert `decay` to seconds themselves
//! (`AlgorithmType::t60_range`), within a few percent. The rest — springs,
//! the plates (the Dattorro tank's own conversion measured 11–12 % short),
//! Cloud, Bloom, Shimmer, Chorale, Swell — set a loop gain from it, so the
//! same knob position meant 0.5 s on one and 20 s on another, and "decay
//! 0.8" said nothing about how long anything rang. These tables are what
//! each one actually does, measured through the Reverb block by
//! `fx-blocks/examples/rt60_table.rs` (impulse, Schroeder decay, T20 × 3,
//! size 0.7, modulation 0.2). The chain maps the user's `decay` across the
//! measured span, log-spaced like the calibrated engines, and hands the
//! engine the setting that measured to that time — so every algorithm's
//! decay is a time, and the surface can say which.
//!
//! Only the rising part of each curve is kept (a flat floor — the early
//! cluster dominating a short tail — and anything past an engine's stable
//! top are dropped), so each table is strictly increasing and invertible.
//!
//! Generated — re-run the example and the generator after changing an
//! engine's decay law.

use core::sync::atomic::{AtomicBool, Ordering};

use crate::algorithm::AlgorithmType;

/// Hand every engine its raw `decay`, tables or not — only for the tool
/// that measures the tables (it has to see each engine's own law).
static RAW: AtomicBool = AtomicBool::new(false);

/// See [`RAW`]. Measurement only.
pub fn measure_raw(on: bool) {
    RAW.store(on, Ordering::Relaxed);
}

/// Whether the tables are bypassed (see [`measure_raw`]).
#[must_use]
pub fn raw() -> bool {
    RAW.load(Ordering::Relaxed)
}

/// One engine's measured curve: `decay` settings and the RT60 (s) each gave.
pub struct DecayTable {
    pub decay: &'static [f64],
    pub t60: &'static [f64],
}

impl DecayTable {
    /// The span of times this engine reaches.
    #[must_use]
    pub const fn range(&self) -> (f64, f64) {
        (self.t60[0], self.t60[self.t60.len() - 1])
    }

    /// The engine `decay` that measured to `t60_s` (interpolated in log
    /// time; clamped to the table's ends).
    #[must_use]
    pub fn engine_decay(&self, t60_s: f64) -> f64 {
        let n = self.t60.len();
        if t60_s <= self.t60[0] {
            return self.decay[0];
        }
        if t60_s >= self.t60[n - 1] {
            return self.decay[n - 1];
        }
        let lt = t60_s.ln();
        for i in 1..n {
            if t60_s <= self.t60[i] {
                let (a, b) = (self.t60[i - 1].ln(), self.t60[i].ln());
                let f = if b > a { (lt - a) / (b - a) } else { 0.0 };
                return self.decay[i - 1] + f * (self.decay[i] - self.decay[i - 1]);
            }
        }
        self.decay[n - 1]
    }
}

/// Plate (variant 0): 1.06 s … 21.91 s.
const PLATE_0: DecayTable = DecayTable {
    decay: &[0.000, 0.050, 0.100, 0.150, 0.200, 0.250, 0.300, 0.350, 0.400, 0.450, 0.500, 0.550, 0.600, 0.650, 0.700, 0.750, 0.800, 0.850, 0.900, 0.950],
    t60: &[1.060, 1.234, 1.403, 1.623, 1.880, 2.208, 2.585, 3.036, 3.563, 4.194, 4.919, 5.823, 6.838, 8.019, 9.448, 11.199, 13.284, 15.657, 18.532, 21.911],
};

/// Plate (variant 1): 0.84 s … 17.71 s.
const PLATE_1: DecayTable = DecayTable {
    decay: &[0.000, 0.050, 0.100, 0.150, 0.200, 0.250, 0.300, 0.350, 0.400, 0.450, 0.500, 0.550, 0.600, 0.650, 0.700, 0.750, 0.800, 0.850, 0.900, 0.950],
    t60: &[0.839, 0.899, 0.962, 1.024, 1.091, 1.185, 1.307, 1.434, 1.554, 1.712, 1.920, 2.179, 2.469, 2.880, 3.412, 4.113, 5.069, 6.730, 9.837, 17.715],
};

/// Plate (variant 2): 1.19 s … 20.83 s.
const PLATE_2: DecayTable = DecayTable {
    decay: &[0.000, 0.050, 0.100, 0.150, 0.200, 0.250, 0.300, 0.350, 0.400, 0.450, 0.500, 0.550, 0.600, 0.650, 0.700, 0.750, 0.800, 0.850, 0.900, 0.950],
    t60: &[1.193, 1.249, 1.315, 1.394, 1.477, 1.581, 1.752, 1.999, 2.176, 2.353, 2.582, 2.971, 3.354, 3.848, 4.569, 5.465, 6.516, 8.586, 12.309, 20.826],
};

/// Spring (variant 0): 0.49 s … 5.88 s.
const SPRING_0: DecayTable = DecayTable {
    decay: &[0.000, 0.050, 0.150, 0.200, 0.250, 0.300, 0.400, 0.450, 0.500, 0.550, 0.600, 0.650, 0.700, 0.750, 0.800, 0.850, 0.900, 0.950, 1.000],
    t60: &[0.491, 0.576, 0.589, 0.627, 0.698, 0.759, 0.889, 0.944, 1.020, 1.114, 1.284, 1.452, 1.575, 1.766, 2.094, 2.471, 3.142, 4.025, 5.885],
};

/// Spring (variant 1): 0.41 s … 4.23 s.
const SPRING_1: DecayTable = DecayTable {
    decay: &[0.000, 0.150, 0.200, 0.250, 0.350, 0.400, 0.500, 0.550, 0.600, 0.650, 0.700, 0.750, 0.800, 0.850, 0.900, 0.950, 1.000],
    t60: &[0.408, 0.427, 0.509, 0.553, 0.578, 0.693, 0.778, 0.838, 0.995, 1.056, 1.133, 1.360, 1.574, 1.835, 2.328, 2.892, 4.231],
};

/// Cloud (variant 0): 2.48 s … 28.03 s.
const CLOUD_0: DecayTable = DecayTable {
    decay: &[0.000, 0.400, 0.450, 0.500, 0.550, 0.600, 0.650, 0.700, 0.750, 0.800, 0.850],
    t60: &[2.477, 2.618, 2.882, 3.362, 4.160, 5.382, 7.199, 10.000, 13.940, 19.669, 28.027],
};

/// Bloom (variant 0): 0.87 s … 21.44 s.
const BLOOM_0: DecayTable = DecayTable {
    decay: &[0.000, 0.050, 0.100, 0.150, 0.200, 0.250, 0.300, 0.350, 0.400, 0.450, 0.500, 0.550, 0.600, 0.650, 0.700, 0.750, 0.800, 0.850],
    t60: &[0.871, 0.892, 0.946, 1.060, 1.349, 1.421, 1.473, 1.554, 1.934, 2.044, 2.481, 2.924, 3.441, 4.199, 5.358, 7.231, 10.841, 21.437],
};

/// Shimmer (variant 0): 0.52 s … 11.13 s.
const SHIMMER_0: DecayTable = DecayTable {
    decay: &[0.000, 0.050, 0.100, 0.150, 0.200, 0.250, 0.300, 0.350, 0.400, 0.450, 0.500, 0.550, 0.600, 0.650, 0.700, 0.750, 0.800, 0.850, 0.900, 0.950, 1.000],
    t60: &[0.523, 0.552, 0.589, 0.627, 0.670, 0.723, 0.775, 0.842, 0.914, 1.012, 1.137, 1.285, 1.460, 1.719, 2.096, 2.623, 3.485, 4.510, 7.594, 9.285, 11.133],
};

/// Chorale (variant 0): 0.50 s … 8.95 s.
const CHORALE_0: DecayTable = DecayTable {
    decay: &[0.000, 0.050, 0.100, 0.150, 0.200, 0.250, 0.300, 0.350, 0.400, 0.450, 0.500, 0.550, 0.600, 0.650, 0.700, 0.750, 0.800, 0.850, 0.900, 0.950, 1.000],
    t60: &[0.500, 0.534, 0.567, 0.601, 0.631, 0.674, 0.723, 0.785, 0.855, 0.940, 1.049, 1.174, 1.333, 1.522, 1.753, 2.137, 2.754, 3.793, 5.080, 6.152, 8.952],
};

/// Swell (variant 0): 0.52 s … 4.43 s.
const SWELL_0: DecayTable = DecayTable {
    decay: &[0.000, 0.050, 0.100, 0.150, 0.200, 0.250, 0.300, 0.350, 0.400, 0.450, 0.500, 0.550, 0.600, 0.650, 0.700, 0.750, 0.800, 0.850, 0.900, 0.950, 1.000],
    t60: &[0.517, 0.549, 0.584, 0.623, 0.653, 0.685, 0.715, 0.748, 0.788, 0.829, 0.898, 0.980, 1.090, 1.214, 1.342, 1.504, 1.739, 2.049, 2.505, 3.201, 4.434],
};

/// The measured curve for `algorithm` / `variant`, when its decay is a
/// feedback law; `None` for the engines that convert time themselves (and
/// for those whose decay is not a tail length — Magneto's heads, NonLinear's
/// window).
#[must_use]
pub const fn table(algorithm: AlgorithmType, variant: usize) -> Option<&'static DecayTable> {
    match (algorithm, variant) {
        (AlgorithmType::Plate, 0) => Some(&PLATE_0),
        (AlgorithmType::Plate, 1) => Some(&PLATE_1),
        (AlgorithmType::Plate, 2) => Some(&PLATE_2),
        (AlgorithmType::Spring, 0) => Some(&SPRING_0),
        (AlgorithmType::Spring, 1) => Some(&SPRING_1),
        (AlgorithmType::Cloud, _) => Some(&CLOUD_0),
        (AlgorithmType::Bloom, _) => Some(&BLOOM_0),
        (AlgorithmType::Shimmer, _) => Some(&SHIMMER_0),
        (AlgorithmType::Chorale, _) => Some(&CHORALE_0),
        (AlgorithmType::Swell, _) => Some(&SWELL_0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_table_rises_and_inverts() {
        for (a, v) in [
            (AlgorithmType::Plate, 0),
            (AlgorithmType::Plate, 1),
            (AlgorithmType::Plate, 2),
            (AlgorithmType::Spring, 0),
            (AlgorithmType::Spring, 1),
            (AlgorithmType::Cloud, 0),
            (AlgorithmType::Bloom, 0),
            (AlgorithmType::Shimmer, 0),
            (AlgorithmType::Chorale, 0),
            (AlgorithmType::Swell, 0),
        ] {
            let Some(t) = table(a, v) else { continue };
            assert!(t.t60.windows(2).all(|w| w[1] > w[0]), "{a:?}/{v} rises");
            assert!(t.decay.windows(2).all(|w| w[1] > w[0]), "{a:?}/{v} decay rises");
            for (d, s) in t.decay.iter().zip(t.t60) {
                assert!((t.engine_decay(*s) - d).abs() < 1e-9, "{a:?}/{v} inverts at {s}");
            }
        }
    }
}
