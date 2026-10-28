//! The built-in flares, compiled in.

use super::design::{Design, Element, Kind};

/// Each built-in flare's group and its design as JSON, in the order the
/// window shows them.
pub const BUILT_IN: &[(&str, &str)] = &[];

/// The groups the built-in flares are filed under.
pub const GROUPS: &[&str] = &["Natural", "Cinema", "Sci-fi", "Stage", "Sun", "Subtle"];

/// What a fresh Sprite flare draws: the first built-in flare, or a glow with
/// a train of ghosts when there are none.
pub fn default_design() -> Design {
    if let Some(d) = BUILT_IN.first().and_then(|(_, json)| Design::parse(json)) {
        return d;
    }
    Design {
        lumit_flare: super::design::FORMAT,
        name: String::new(),
        elements: vec![Element::new(Kind::Glow), Element::new(Kind::Ghosts)],
    }
}
