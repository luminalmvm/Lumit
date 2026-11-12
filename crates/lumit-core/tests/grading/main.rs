//! The grading tools end to end at the core: the Correction's CPU twin group
//! by group, its Parallel and Layer runs, the matte view, CDL in and out, the
//! colour warper, the picks, and a shot's grade kept with the project.
//!
//! Each test checks pixels or the document against the formulas the modules
//! write down, so the GPU's oracle and the Grade panel stand on something
//! tested here too.

// A test asserts by panicking; the engine's no-panic rule is about the engine
// (the same allow every other integration test in the workspace carries).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod cdl;
mod combine;
mod correction;
mod document;
mod keys;
mod matte_view;
mod picks;
mod support;
mod warper;
