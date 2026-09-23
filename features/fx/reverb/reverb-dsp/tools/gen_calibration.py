import sys, collections
rows = [l.split('\t') for l in open(sys.argv[1]).read().split('\n') if l.strip()]
data = collections.defaultdict(list)
for a, v, d, t in rows:
    if t.strip() == 'nan':
        continue
    data[(int(a), int(v))].append((float(d), float(t)))
NAMES = {2: 'Plate', 3: 'Spring', 4: 'Cloud', 5: 'Bloom', 6: 'Shimmer', 7: 'Chorale', 10: 'Swell'}
# Natively calibrated (room/hall/plate-0) and non-time engines are left out.
WANT = [(2, 0), (2, 1), (2, 2), (3, 0), (3, 1), (4, 0), (5, 0), (6, 0), (7, 0), (10, 0)]
MAX_T = 30.0
out = []
out.append('''//! Measured decay → RT60 for the engines whose `decay` is a feedback law
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
//! Generated — after changing an engine's decay law, re-measure and
//! regenerate: `cargo run --release -p fx-blocks --example rt60_table >
//! rt60.tsv` (add `--burst 10:0` rows for Swell), then
//! `python3 tools/gen_calibration.py rt60.tsv src/calibration.rs`.

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
''')
names = []
for key in WANT:
    pts = sorted(data.get(key, []))
    kept = []
    for d, t in pts:
        if t > MAX_T or t <= 0:
            continue
        if not kept or t > kept[-1][1] * 1.02:
            kept.append((d, t))
        elif t < kept[-1][1] * 0.8:
            # A collapse past the stable top: stop here.
            break
    if len(kept) < 3:
        print(f'skip {key}: {kept}', file=sys.stderr)
        continue
    a, v = key
    nm = f'{NAMES[a].upper()}_{v}'
    names.append((a, v, nm))
    ds = ', '.join(f'{d:.3f}' for d, _ in kept)
    ts = ', '.join(f'{t:.3f}' for _, t in kept)
    out.append(f'''
/// {NAMES[a]} (variant {v}): {kept[0][1]:.2f} s … {kept[-1][1]:.2f} s.
const {nm}: DecayTable = DecayTable {{
    decay: &[{ds}],
    t60: &[{ts}],
}};
''')
out.append('''
/// The measured curve for `algorithm` / `variant`, when its decay is a
/// feedback law; `None` for the engines that convert time themselves (and
/// for those whose decay is not a tail length — Magneto's heads, NonLinear's
/// window).
#[must_use]
pub const fn table(algorithm: AlgorithmType, variant: usize) -> Option<&'static DecayTable> {
    match (algorithm, variant) {
''')
AT = {2: 'Plate', 3: 'Spring', 4: 'Cloud', 5: 'Bloom', 6: 'Shimmer', 7: 'Chorale', 10: 'Swell'}
single = {a for a, v, _ in names if a not in (2, 3)}
for a, v, nm in names:
    if a in (2, 3):
        out.append(f'        (AlgorithmType::{AT[a]}, {v}) => Some(&{nm}),\n')
    else:
        out.append(f'        (AlgorithmType::{AT[a]}, _) => Some(&{nm}),\n')
out.append('''        _ => None,
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
''')
open(sys.argv[2], 'w').write(''.join(out))
print('tables:', [(a, v) for a, v, _ in names])
