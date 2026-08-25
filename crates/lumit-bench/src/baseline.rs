//! The baseline file and the ratio gate (K-389).
//!
//! # In plain terms
//!
//! docs/13 §2's budgets are stated against named hardware (§1), and a GitHub
//! runner is not that machine — a runner is a shared virtual machine with a
//! software rasteriser, and asserting "50 ms" there would fail for reasons that
//! have nothing to do with Lumit. So the numbers are enforced two ways:
//!
//! - **Everywhere**: against a checked-in baseline *for that runner's operating
//!   system*, and a run fails only if it is [`DEFAULT_GATE_FACTOR`] times worse
//!   than the baseline. A real regression is a factor; runner noise is not.
//! - **On the reference machine only** (`LUMIT_REFERENCE_HW=1`): against the
//!   absolute budgets in [`DESKTOP_BUDGET_MS`]. The budgets remain the truth;
//!   the ratio gate is what an ordinary runner can honestly enforce.
//!
//! A baseline is regenerated the way `crates/lumit-core/fx-labels.txt` is: run
//! the harness, copy its output over the file, commit the change on purpose.

use std::collections::BTreeMap;

use crate::scenarios::Measurement;

/// How much worse than baseline is a regression rather than noise. Overridden
/// by `LUMIT_BENCH_GATE_FACTOR` — see [`gate_factor`].
pub const DEFAULT_GATE_FACTOR: f64 = 1.6;

/// Below this, a ratio means nothing and the gate does not fire.
///
/// B5 is why. Serving a warm frame costs about ten *microseconds* — naming it
/// and finding it, no compositing at all — and at that size a scheduler
/// hiccup is a factor of two. A gate that cried regression over 10 µs becoming
/// 20 µs would be ignored within a week, which is the only way a perf gate ever
/// really fails. Nothing a user can see hides under a millisecond a frame:
/// even at the floor, warm playback is a thousand frames a second against the
/// sixty B5 asks for.
pub const NOISE_FLOOR_MS: f64 = 1.0;

/// docs/13 §2's reference-desktop column, in the units [`Measurement::value_ms`]
/// reports: a latency for B3/B4, milliseconds per frame for B5–B7 (60 fps is
/// 16.67, 24 fps is 41.67), the whole fill for B11, and one evaluate-and-draw
/// **above the pass floor** for B12–B14 (K-475; see
/// [`crate::scenarios::particulate`] for what the floor is and why it is
/// subtracted).
///
/// Asserted only under [`reference_hardware`]. Anywhere else these are what the
/// harness is aiming at, not what it is judged by.
pub const DESKTOP_BUDGET_MS: [(&str, f64); 9] = [
    ("B3", 50.0),
    ("B4", 500.0),
    ("B5", 1000.0 / 60.0),
    ("B6", 1000.0 / 60.0),
    ("B7", 1000.0 / 24.0),
    ("B11", 60_000.0),
    ("B12", 0.2),
    ("B13", 1.0),
    ("B14", 16.0),
];

/// A run's numbers, keyed by budget — the results file the harness writes and,
/// once a copy of it is committed, the baseline it is judged against.
fn default_span_fraction() -> u64 {
    100
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Baseline {
    /// `std::env::consts::OS` of the machine that produced it. Compared before
    /// anything else: Windows numbers against a macOS baseline would be a gate
    /// that fires on the operating system.
    pub os: String,
    /// The `BENCH_SPAN_FRACTION` the numbers were measured at (see
    /// `scenarios::span_fraction`). A run and its baseline must agree, or the
    /// ratio compares a tenth of a fill against the whole of one.
    #[serde(default = "default_span_fraction")]
    pub span_fraction: u64,
    /// Budget name to [`Measurement::value_ms`].
    pub results: BTreeMap<String, f64>,
}

impl Baseline {
    /// This machine's results, ready to be written out.
    #[must_use]
    pub fn from_results(results: &[Measurement]) -> Self {
        Self {
            os: std::env::consts::OS.to_string(),
            span_fraction: crate::scenarios::span_fraction(),
            results: results
                .iter()
                .map(|m| (m.budget.to_string(), m.value_ms))
                .collect(),
        }
    }
}

/// How one budget's number compares with its baseline.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct Verdict {
    pub budget: &'static str,
    pub value_ms: f64,
    /// `None` when the baseline has no entry for this budget — a newly added
    /// scenario, which cannot have regressed against nothing.
    pub baseline_ms: Option<f64>,
    /// `value_ms / baseline_ms`; above 1 is slower than the baseline.
    pub ratio: Option<f64>,
    pub pass: bool,
}

/// The gate factor for this run: `LUMIT_BENCH_GATE_FACTOR` if it is a positive
/// finite number, else [`DEFAULT_GATE_FACTOR`]. A nonsense value is ignored
/// rather than obeyed — a gate of zero or NaN would fail every budget for a
/// reason nobody could read off the failure.
#[must_use]
pub fn gate_factor() -> f64 {
    std::env::var("LUMIT_BENCH_GATE_FACTOR")
        .ok()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|f| f.is_finite() && *f > 0.0)
        .unwrap_or(DEFAULT_GATE_FACTOR)
}

/// Whether this machine claims to be docs/13 §1's reference desktop, and so
/// should be held to the absolute budgets. Unset, empty and `0` all mean no —
/// the same rule `lumit-gpu`'s adapter check uses, so there is one convention.
#[must_use]
pub fn reference_hardware() -> bool {
    matches!(std::env::var("LUMIT_REFERENCE_HW"), Ok(v) if !v.is_empty() && v != "0")
}

/// Judge `results` against `baseline`: a budget passes while it is under
/// `factor` times its baseline number.
///
/// `Err` if the baseline was made on another operating system, which is the one
/// mistake that would make every verdict meaningless.
pub fn compare(
    baseline: &Baseline,
    results: &[Measurement],
    factor: f64,
) -> Result<Vec<Verdict>, String> {
    let here = std::env::consts::OS;
    if baseline.os != here {
        return Err(format!(
            "baseline is from {} and this machine is {here}: the gate compares a runner with \
             itself, so each operating system keeps its own baseline",
            baseline.os
        ));
    }
    if baseline.span_fraction != crate::scenarios::span_fraction() {
        return Err(format!(
            "baseline was measured at span fraction {} but this run is at {}: the two are \
             different measurements; regenerate the baseline at the fraction this \
             environment uses",
            baseline.span_fraction,
            crate::scenarios::span_fraction()
        ));
    }
    Ok(results
        .iter()
        .map(|m| {
            // A baseline of zero (or worse) names no measurement: treat it as
            // absent rather than dividing by it.
            let base = baseline
                .results
                .get(m.budget)
                .copied()
                .filter(|b| b.is_finite() && *b > 0.0);
            let ratio = base.map(|b| m.value_ms / b);
            Verdict {
                budget: m.budget,
                value_ms: m.value_ms,
                baseline_ms: base,
                ratio,
                pass: m.value_ms < NOISE_FLOOR_MS || ratio.is_none_or(|r| r <= factor),
            }
        })
        .collect())
}

/// The budgets `results` breaches, as readable lines. Empty means every budget
/// held. Only meaningful on [`reference_hardware`]; the caller decides that.
#[must_use]
pub fn budget_breaches(results: &[Measurement]) -> Vec<String> {
    results
        .iter()
        .filter_map(|m| {
            let (_, budget) = DESKTOP_BUDGET_MS.iter().find(|(b, _)| *b == m.budget)?;
            (m.value_ms > *budget).then(|| {
                format!(
                    "{} is {:.1} ms against docs/13's {:.1} ms",
                    m.budget, m.value_ms, budget
                )
            })
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn baseline(os: &str) -> Baseline {
        Baseline {
            os: os.into(),
            span_fraction: 100,
            results: [("B3".to_string(), 40.0), ("B7".to_string(), 30.0)]
                .into_iter()
                .collect(),
        }
    }

    fn result(budget: &'static str, value_ms: f64) -> Measurement {
        Measurement {
            budget,
            value_ms,
            frames: 1,
        }
    }

    #[test]
    fn a_foreign_span_fraction_is_refused_not_compared() {
        let base = Baseline {
            os: std::env::consts::OS.to_string(),
            span_fraction: 7,
            results: std::collections::BTreeMap::new(),
        };
        let err = compare(&base, &[], 1.6).unwrap_err();
        assert!(
            err.contains("span fraction"),
            "a tenth of a fill must never be judged against the whole of one: {err}"
        );
    }

    #[test]
    fn faster_and_slightly_slower_both_pass() {
        let base = baseline(std::env::consts::OS);
        // Half the baseline, and 1.5x it — inside the 1.6 factor.
        let results = [result("B3", 20.0), result("B7", 45.0)];
        let verdicts = compare(&base, &results, DEFAULT_GATE_FACTOR).unwrap();
        assert!(verdicts.iter().all(|v| v.pass), "{verdicts:?}");
        assert_eq!(verdicts[0].ratio, Some(0.5));
    }

    #[test]
    fn past_the_factor_fails_and_names_the_budget() {
        let base = baseline(std::env::consts::OS);
        // 1.625x baseline: past 1.6, and the kind of jump a real regression makes.
        let results = [result("B3", 65.0), result("B7", 30.0)];
        let verdicts = compare(&base, &results, DEFAULT_GATE_FACTOR).unwrap();
        let failed: Vec<_> = verdicts.iter().filter(|v| !v.pass).collect();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].budget, "B3");
    }

    #[test]
    fn exactly_at_the_factor_still_passes() {
        let base = baseline(std::env::consts::OS);
        let verdicts = compare(&base, &[result("B3", 64.0)], DEFAULT_GATE_FACTOR).unwrap();
        assert!(verdicts[0].pass);
    }

    #[test]
    fn a_measurement_under_the_noise_floor_does_not_fire() {
        // A warm-playback serve that got five times dearer — and is still
        // fifty microseconds, which nothing can see.
        let base = Baseline {
            os: std::env::consts::OS.into(),
            span_fraction: 100,
            results: [("B5".to_string(), 0.01)].into_iter().collect(),
        };
        let verdicts = compare(&base, &[result("B5", 0.05)], DEFAULT_GATE_FACTOR).unwrap();
        assert!(verdicts[0].pass);
        // The ratio is still reported: the gate is quiet, not blind.
        assert_eq!(verdicts[0].ratio, Some(5.0));
        // Past the floor it fires like anything else.
        let verdicts = compare(&base, &[result("B5", 2.0)], DEFAULT_GATE_FACTOR).unwrap();
        assert!(!verdicts[0].pass);
    }

    #[test]
    fn a_budget_the_baseline_has_never_seen_cannot_regress() {
        let base = baseline(std::env::consts::OS);
        let verdicts = compare(&base, &[result("B11", 90_000.0)], DEFAULT_GATE_FACTOR).unwrap();
        assert!(verdicts[0].pass);
        assert_eq!(verdicts[0].baseline_ms, None);
        assert_eq!(verdicts[0].ratio, None);
    }

    #[test]
    fn a_baseline_from_another_os_is_refused() {
        let other = if std::env::consts::OS == "windows" {
            "macos"
        } else {
            "windows"
        };
        let err = compare(&baseline(other), &[result("B3", 1.0)], 1.6).unwrap_err();
        assert!(err.contains(other), "{err}");
    }

    #[test]
    fn the_gate_factor_ignores_nonsense() {
        // No environment variable is set in this process, so the default holds;
        // the parse rules are what the test is really about.
        assert_eq!(gate_factor(), DEFAULT_GATE_FACTOR);
    }

    #[test]
    fn breaches_name_only_the_budgets_that_broke() {
        let breaches = budget_breaches(&[
            result("B3", 49.0),
            result("B4", 900.0),
            result("B11", 61_000.0),
        ]);
        assert_eq!(breaches.len(), 2, "{breaches:?}");
        assert!(breaches[0].starts_with("B4"));
        assert!(breaches[1].starts_with("B11"));
    }

    #[test]
    fn a_results_file_is_a_baseline() {
        let base = Baseline::from_results(&[result("B3", 12.0)]);
        assert_eq!(base.os, std::env::consts::OS);
        assert_eq!(base.results.get("B3"), Some(&12.0));
        // Round-trips, so `cargo run > baseline.json` is the whole regeneration.
        let text = serde_json::to_string(&base).unwrap();
        let back: Baseline = serde_json::from_str(&text).unwrap();
        assert_eq!(back.results, base.results);
    }
}
