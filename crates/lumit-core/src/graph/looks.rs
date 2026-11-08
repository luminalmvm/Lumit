//! A box's label colour and whether it's folded down to its heading, on
//! either kind of graph. Presentation only: neither changes a pixel, so
//! neither reaches a frame key.

use serde::{Deserialize, Serialize};

/// How a graph's boxes are drawn, beside where they sit. Keyed by whatever
/// names a box on its graph: a [`NodeRef`](super::NodeRef) on a layer's graph,
/// a box id on a node graph.
///
/// Left out of the saved file while empty, and an older Lumit reading a file
/// that has it skips the field, so the project still opens there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoxLooks<K> {
    /// The label chip a box is tinted with. Chip 0 is the quiet default, so a
    /// box on it isn't listed.
    #[serde(default = "Vec::new", skip_serializing_if = "Vec::is_empty")]
    pub colours: Vec<(K, u32)>,
    /// The boxes folded down to their heading.
    #[serde(default = "Vec::new", skip_serializing_if = "Vec::is_empty")]
    pub collapsed: Vec<K>,
}

impl<K> Default for BoxLooks<K> {
    fn default() -> Self {
        Self {
            colours: Vec::new(),
            collapsed: Vec::new(),
        }
    }
}

impl<K: Copy + PartialEq> BoxLooks<K> {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.colours.is_empty() && self.collapsed.is_empty()
    }

    /// The chip `key` is tinted with, 0 for none.
    #[must_use]
    pub fn colour(&self, key: K) -> u32 {
        self.colours
            .iter()
            .find(|(k, _)| *k == key)
            .map_or(0, |(_, c)| *c)
    }

    /// Tint `key` with chip `colour`; chip 0 takes the tint away.
    pub fn set_colour(&mut self, key: K, colour: u32) {
        self.colours.retain(|(k, _)| *k != key);
        if colour != 0 {
            self.colours.push((key, colour));
        }
    }

    #[must_use]
    pub fn is_collapsed(&self, key: K) -> bool {
        self.collapsed.contains(&key)
    }

    pub fn set_collapsed(&mut self, key: K, collapsed: bool) {
        self.collapsed.retain(|k| *k != key);
        if collapsed {
            self.collapsed.push(key);
        }
    }

    /// Forget every box `keep` turns down, and say whether any went.
    pub fn retain(&mut self, keep: impl Fn(K) -> bool) -> bool {
        let before = (self.colours.len(), self.collapsed.len());
        self.colours.retain(|(k, _)| keep(*k));
        self.collapsed.retain(|k| keep(*k));
        before != (self.colours.len(), self.collapsed.len())
    }
}
