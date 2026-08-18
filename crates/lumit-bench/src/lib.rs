//! lumit-bench: the headless performance harness (docs/13-PERFORMANCE-RULES.md
//! §7.3, K-389).
//!
//! # In plain terms
//!
//! docs/13 promises numbers — "a scrub shows something within 50 ms", "the
//! twenty-second work area caches in under a minute". A promise nobody measures
//! is a slogan, so this crate builds the exact composition those promises are
//! stated against and drives the real engine through it with no window and no
//! Flutter in sight.
//!
//! It is a **development** crate. Nothing in the application depends on it; the
//! shipped library is `lumit-bridge`, which has never heard of it. It is a
//! workspace member so that `cargo fmt`, `cargo clippy` and `cargo test` cover
//! it like everything else.
//!
//! Four pieces:
//!
//! - [`media`] makes the reference comp's footage with the ffmpeg command line,
//!   rather than committing tens of megabytes of video to a public repository.
//! - [`comp`] builds docs/13 §1's composition over that media, through the same
//!   document model the application edits.
//! - [`scenarios`] drives the engine through it with a stopwatch: B3, B4, B5,
//!   B6, B7 and B11. K-389 records which budgets a headless harness can reach
//!   and which stay manual or real-window (B1/B2, B8–B10).
//! - [`baseline`] is the gate — the checked-in numbers a run is compared with,
//!   per operating system, and the factor that separates a regression from a
//!   noisy runner.
//!
//! The binary (`src/main.rs`) runs all six and writes the JSON; each scenario is
//! also an `#[ignore]`d test, so one budget can be measured on its own.

pub mod baseline;
pub mod comp;
pub mod media;
pub mod scenarios;

pub use baseline::Baseline;
pub use comp::{reference_comp, LAYER_COUNT};
pub use media::RefMedia;
pub use scenarios::{Harness, Measurement};
