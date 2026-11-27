// A test binary: a failed setup step should stop the test, loudly, and the
// no-panic rule of docs/14 is about the engine's own paths.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod authoring;
mod copies;
mod data;
mod kit;
