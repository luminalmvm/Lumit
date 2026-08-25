//! Run every scenario and write the numbers out — the entry point CI drives
//! (docs/13-PERFORMANCE-RULES.md §7.3, K-389).
//!
//! # In plain terms
//!
//! `cargo run --release -p lumit-bench` builds docs/13 §1's reference comp,
//! drives the engine through the six headless scenarios, prints each number as
//! it lands, and writes them all as one JSON file. Nothing else: judging the
//! numbers is the gate's job, and the gate is opt-in.
//!
//! Build it **released**. The workspace optimises the engine's maths crates
//! even in a debug build, but the harness's own loop and everything around it
//! is only fast when the whole thing is; a debug number is not comparable with
//! anything.
//!
//! Environment:
//!
//! | | |
//! |---|---|
//! | `BENCH_OUT` | where the results JSON goes (default `target/bench-results.json`) |
//! | `BENCH_MEDIA` | where the generated clips live (default a fixed temp directory, reused between runs) |
//! | `BENCH_BASELINE` | a baseline JSON to judge this run against; without it the run only measures |
//! | `LUMIT_BENCH_GATE_FACTOR` | how much worse than baseline fails (default 1.6) |
//! | `LUMIT_REFERENCE_HW` | set on docs/13 §1's reference desktop, where the absolute budgets are asserted too |
//!
//! The checked-in baselines live in `crates/lumit-bench/baselines/<os>.json`,
//! one per operating system, and the CI job `performance gates (ratio vs
//! baseline)` points `BENCH_BASELINE` at the one for the runner it is on.
//! Regenerating one is the same run with `BENCH_OUT` aimed at it — a results
//! file and a baseline file are the same format on purpose — then committing
//! what changed, the way `crates/lumit-core/fx-labels.txt` is regenerated:
//!
//! ```text
//! BENCH_OUT=crates/lumit-bench/baselines/windows.json cargo run --release -p lumit-bench
//! ```
//!
//! Exit code: 0 when every scenario ran and every enabled gate held, 1
//! otherwise — including "this machine has no GPU adapter", because a harness
//! that measured nothing has not passed.

use std::path::PathBuf;
use std::process::ExitCode;

use lumit_bench::baseline::{self, Baseline};
use lumit_bench::scenarios::{Harness, Measurement};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lumit-bench: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let harness = Harness::new(&media_dir())?;

    // Printed as each lands: B11 alone is a thousand-odd frames, so a run that
    // said nothing until the end would look hung.
    let mut results = Vec::new();
    let measured = harness.all(&mut |m: Measurement| {
        println!(
            "{}",
            serde_json::to_string(&m).unwrap_or_else(|_| format!("{m:?}"))
        );
    });
    results.extend(measured?);

    let out = out_path();
    let banked = Baseline::from_results(&results);
    let text =
        serde_json::to_string_pretty(&banked).map_err(|e| format!("serialising results: {e}"))?;
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    std::fs::write(&out, format!("{text}\n"))
        .map_err(|e| format!("writing {}: {e}", out.display()))?;
    eprintln!("lumit-bench: wrote {}", out.display());

    let mut failed = Vec::new();

    if let Some(path) = std::env::var_os("BENCH_BASELINE") {
        let path = PathBuf::from(path);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        let base: Baseline =
            serde_json::from_str(&text).map_err(|e| format!("parsing {}: {e}", path.display()))?;
        let factor = baseline::gate_factor();
        for v in baseline::compare(&base, &results, factor)? {
            match (v.baseline_ms, v.ratio) {
                (Some(b), Some(r)) => eprintln!(
                    "{} {:.2}x baseline ({:.1} ms against {:.1} ms) — {}",
                    v.budget,
                    r,
                    v.value_ms,
                    b,
                    if v.pass { "ok" } else { "REGRESSED" }
                ),
                _ => eprintln!("{} {:.1} ms — no baseline entry", v.budget, v.value_ms),
            }
            if !v.pass {
                failed.push(format!("{} regressed past {factor}x baseline", v.budget));
            }
        }
    }

    if baseline::reference_hardware() {
        failed.extend(baseline::budget_breaches(&results));
    }

    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed.join("; "))
    }
}

/// Where the results go.
fn out_path() -> PathBuf {
    std::env::var_os("BENCH_OUT").map_or_else(
        || PathBuf::from("target").join("bench-results.json"),
        PathBuf::from,
    )
}

/// Where the generated media lives — a fixed directory rather than a temporary
/// one, so a second run does not re-encode 2400 frames of 1080p60.
fn media_dir() -> PathBuf {
    std::env::var_os("BENCH_MEDIA").map_or_else(
        || std::env::temp_dir().join("lumit-bench-media"),
        PathBuf::from,
    )
}
