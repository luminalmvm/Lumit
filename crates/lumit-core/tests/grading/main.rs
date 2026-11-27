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
