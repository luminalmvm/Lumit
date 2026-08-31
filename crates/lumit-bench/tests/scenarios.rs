//! One `#[ignore]`d test per budget, so a single number can be measured on its
//! own (docs/13-PERFORMANCE-RULES.md §2, K-389).
//!
//! # In plain terms
//!
//! The binary runs all six and writes a file; these run one each and print it.
//! That is what you want while working on the thing being measured:
//!
//! ```text
//! cargo test --release -p lumit-bench --test scenarios b7 -- --ignored --nocapture
//! ```
//!
//! They are ignored because they are measurements, not oracles — B11 alone
//! renders the whole twenty-second work area, which no ordinary suite should
//! wait for. Nothing here asserts a budget: a test that failed on a busy
//! machine would teach everyone to ignore it. The gate lives in
//! [`lumit_bench::baseline`] and runs in CI, against that runner's own
//! baseline.
//!
//! Skips politely on a machine with no ffmpeg and on one with no GPU adapter
//! (`LUMIT_REQUIRE_GPU` turns the second skip into a failure).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_bench::scenarios::{Harness, Measurement};

/// The same directory the smoke test uses, so the media is generated once per
/// machine rather than once per run.
fn media_dir() -> std::path::PathBuf {
    std::env::var_os("BENCH_MEDIA").map_or_else(
        || std::env::temp_dir().join("lumit-bench-media"),
        std::path::PathBuf::from,
    )
}

/// Build the harness, or say why this machine cannot measure and skip.
fn harness() -> Option<Harness> {
    match Harness::new(&media_dir()) {
        Ok(h) => Some(h),
        Err(e) => {
            eprintln!("skipping: reference media unavailable ({e})");
            None
        }
    }
}

/// Run one scenario and print its number. A missing GPU adapter is the polite
/// skip every other GPU test uses; any other failure is a real one.
fn measure(scenario: impl FnOnce(&Harness) -> Result<Measurement, String>) {
    let Some(h) = harness() else { return };
    match scenario(&h) {
        Ok(m) => println!(
            "{}  ({:.1} fps over {} frames)",
            serde_json::to_string(&m).unwrap_or_default(),
            m.fps(),
            m.frames
        ),
        Err(e) if e.contains("adapter") => lumit_gpu::no_adapter(),
        Err(e) => panic!("{e}"),
    }
}

#[test]
#[ignore = "measurement, not an oracle: run it explicitly"]
fn b3_scrub_latency() {
    measure(Harness::b3_scrub);
}

#[test]
#[ignore = "measurement, not an oracle: run it explicitly"]
fn b4_refine_to_full() {
    measure(Harness::b4_refine);
}

#[test]
#[ignore = "measurement, not an oracle: run it explicitly"]
fn b5_warm_playback() {
    measure(Harness::b5_warm_playback);
}

#[test]
#[ignore = "measurement, not an oracle: run it explicitly"]
fn b6_cold_adaptive_playback() {
    measure(Harness::b6_cold_adaptive_playback);
}

#[test]
#[ignore = "measurement, not an oracle: run it explicitly"]
fn b7_cold_full_playback() {
    measure(Harness::b7_cold_full_playback);
}

#[test]
#[ignore = "measurement, not an oracle: run it explicitly"]
fn b11_idle_fill() {
    measure(Harness::b11_idle_fill);
}

/// **B12-B14 in one run** (K-475): the three per-effect numbers, which share a
/// floor measurement and so are measured together rather than one at a time.
/// No reference media, so no ffmpeg — only a graphics adapter.
#[test]
#[ignore = "measurement, not an oracle: run it explicitly"]
fn b12_b14_particulate() {
    match lumit_bench::scenarios::particulate::budgets(&mut |m: Measurement| {
        println!("{}", serde_json::to_string(&m).unwrap_or_default());
    }) {
        Ok(_) => {}
        Err(e) if e.contains("adapter") => lumit_gpu::no_adapter(),
        Err(e) => panic!("{e}"),
    }
}

/// **B15-B17 in one run** (K-704, docs/impl/puppet.md §3 test 14): the puppet's
/// warp, mesh build and per-frame solve. Pure CPU — no media and no adapter —
/// so this one runs anywhere, and the gate is the same one every other row
/// gets: [`lumit_bench::baseline`] against the runner's own numbers, and the
/// absolute budgets on the reference machine.
#[test]
#[ignore = "measurement, not an oracle: run it explicitly"]
fn b15_b17_puppet() {
    lumit_bench::scenarios::puppet::budgets(&mut |m: Measurement| {
        println!("{}", serde_json::to_string(&m).unwrap_or_default());
    })
    .unwrap();
}
