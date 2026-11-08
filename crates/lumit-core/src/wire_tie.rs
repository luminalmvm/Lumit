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
    /// A reroute point: every wire through the one spot, where a tie lays
    /// them side by side.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub point: bool,
}

impl WireTie {
    /// The ties a saved set of boxes carries, measured from `origin`. A tie
    /// keeps the wires that are in `saved`, each named by the box its input end
    /// is on and that socket, and goes when none are left.
    #[must_use]
    pub fn saved(ties: &[WireTie], saved: &[(uuid::Uuid, &str)], origin: [f64; 2]) -> Vec<WireTie> {
        ties.iter()
            .filter_map(|tie| {
                let wires: Vec<String> = tie
                    .wires
                    .iter()
                    .filter(|w| input_end(w).is_some_and(|end| saved.contains(&end)))
                    .cloned()
                    .collect();
                (!wires.is_empty()).then(|| WireTie {
                    at: [tie.at[0] - origin[0], tie.at[1] - origin[1]],
                    wires,
                    point: tie.point,
                })
            })
            .collect()
    }

    /// Saved ties landing at `at`, their wires renamed to the boxes' fresh
    /// ids. A wire naming a box that did not come with them is dropped.
    #[must_use]
    pub fn minted(
        ties: &[WireTie],
        fresh: &std::collections::BTreeMap<uuid::Uuid, uuid::Uuid>,
        at: [f64; 2],
    ) -> Vec<WireTie> {
        ties.iter()
            .filter_map(|tie| {
                let wires: Vec<String> = tie
                    .wires
                    .iter()
                    .filter_map(|w| {
                        let (old, _) = input_end(w)?;
                        let new = fresh.get(&old)?;
                        Some(w.replace(&old.to_string(), &new.to_string()))
                    })
                    .collect();
                (!wires.is_empty()).then(|| WireTie {
                    at: [at[0] + tie.at[0], at[1] + tie.at[1]],
                    wires,
                    point: tie.point,
                })
            })
            .collect()
    }
}

/// The box id and socket a wire's name ends in: `…<id>/<socket>`.
fn input_end(wire: &str) -> Option<(uuid::Uuid, &str)> {
    let (box_key, socket) = wire.rsplit_once('/')?;
    let id = box_key.get(box_key.len().checked_sub(36)?..)?;
    Some((uuid::Uuid::parse_str(id).ok()?, socket))
}
