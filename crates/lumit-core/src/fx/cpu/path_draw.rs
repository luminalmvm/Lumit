//! The empty path drawing every path effect starts from.

use super::*;

impl PathDrawParams {
    /// An empty drawing — the no-op every path effect degrades to, and the base
    /// each of them fills in with `..PathDrawParams::blank()`.
    ///
    /// Not [`Default`], because an array of 512 does not derive one.
    #[must_use]
    pub fn blank() -> Self {
        Self {
            segments: [[0.0; 4]; PATH_PRIMITIVES],
            arcs: [0.0; PATH_PRIMITIVES],
            count: 0,
            half_width: 0.0,
            band: 0.5,
            inv_segment: 0.0,
            duty: 2.0,
            phase: 0.0,
            wiggle_amp: 0.0,
            wiggle_freq: 0.0,
            wiggle_tick: 0.0,
            seed: 0,
            colour: [0.0; 3],
            opacity: 0.0,
            style: PAINT_ON_ORIGINAL,
            mix: 0.0,
        }
    }
}
