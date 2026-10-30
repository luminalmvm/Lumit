//! Ties on a graph's wires.
//!
//! A tie gathers some wires at one spot on the canvas, so a bundle of them
//! runs side by side through it like a cable tie. Both kinds of graph keep
//! them beside their notes. Nothing reads them, so a tie changes no pixel and
//! reaches no frame key.

use serde::{Deserialize, Serialize};

/// One tie: where it sits and the wires through it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireTie {
    /// Its middle, on the canvas.
    pub at: [f64; 2],
    /// Each wire by the socket it ends in, `box/socket`, named as the canvas
    /// names its boxes. A wire that has gone is skipped where the tie is drawn.
    pub wires: Vec<String>,
}
