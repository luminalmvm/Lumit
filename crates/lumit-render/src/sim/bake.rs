//! A simulation baked to disk: the state every few frames, in files named by
//! the walk, so a heavy sim opens without simulating and travels with Collect.

use std::sync::Arc;

use lumit_core::fx::points::sim::Ledger;
use lumit_core::model::Document;

/// A state read back from disk.
pub(super) struct Baked {
    pub ledger: Ledger,
    pub bytes: Vec<u8>,
}

/// The furthest state baked of `walk` past `after` and up to `frame`.
pub(super) fn nearest(
    _doc: &Arc<Document>,
    _walk: &[u8; 32],
    _frame: i64,
    _after: Option<i64>,
) -> Option<Baked> {
    None
}
