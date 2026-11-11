//! Graphics templates end to end, without a GPU: making a template's
//! controls, reading and writing the rows they name, the values a placed copy
//! keeps, the document a copy plays, and data rows feeding copies.
//!
//! Each test asserts what a copy shows or what an edit writes, so a template
//! that drew the wrong value, lost a copy's own setting or fed the wrong
//! column fails here and not only in the render.

// A test binary: a failed setup step should stop the test, loudly, and the
// no-panic rule of docs/14 is about the engine's own paths.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod authoring;
mod copies;
mod data;
mod kit;
