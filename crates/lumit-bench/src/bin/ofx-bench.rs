//! Fetch and build the OFX conformance bench, and say where it landed
//! (docs/impl/ofx-host.md §5 item 1).
//!
//! # In plain terms
//!
//! `cargo run -p lumit-bench --bin ofx-bench` puts openfx-misc and ntsc-rs in a
//! folder the host's conformance test knows to look in, building them from
//! source if they are not there yet. Run it once; every run after that finds
//! the folder already full and does nothing.
//!
//! Environment:
//!
//! | | |
//! |---|---|
//! | `LUMIT_OFX_BENCH` | where the bundles go (default a fixed temp directory, reused between runs) |
//! | `LUMIT_REQUIRE_OFX_BENCH` | fail rather than report when the bench cannot be built — for a CI job that has proved it can |
//!
//! Exit code: 0 when the bench is there, or when it is not and nobody insisted
//! — a machine without a compiler is a machine that skips the pass, not a
//! broken one. 1 when `LUMIT_REQUIRE_OFX_BENCH` is set and the bench is absent,
//! which is the same shape as `LUMIT_REQUIRE_GPU` in the test suite: a gate is
//! only worth having where it has been shown to hold.

use std::process::ExitCode;

fn main() -> ExitCode {
    let dir = lumit_bench::ofx::bench_dir();
    match lumit_bench::ofx::ensure(&dir) {
        Ok(bundles) => {
            println!("{}", dir.display());
            for bundle in bundles {
                eprintln!("lumit-bench ofx: {}", bundle.display());
            }
            ExitCode::SUCCESS
        }
        Err(why) => {
            eprintln!("lumit-bench ofx: the bench was not built — {why}");
            if std::env::var_os("LUMIT_REQUIRE_OFX_BENCH").is_some() {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
    }
}
