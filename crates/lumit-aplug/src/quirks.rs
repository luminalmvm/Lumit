//! The quirks table: per-plugin deviations, as data.
//!
//! # In plain terms
//!
//! Every host implements a plugin standard slightly differently, and every
//! commercial plugin carries a table of workarounds for the hosts it knows.
//! Lumit needs the mirror image of that table, and it needs it from the first
//! day rather than after the workarounds have been sprinkled through the code as
//! `if id == …` (the OFX lesson, docs/12 §2.5, carried over verbatim by
//! docs/impl/audio-plugins.md §5). So the deviations live in `quirks.json`,
//! keyed by the plugin's own identifier, and the code only ever asks the table a
//! question.
//!
//! Today the table is empty, and an empty table that parses is exactly the
//! shipping default: no plugin has yet earned an entry.
//!
//! # The two deadlines, and why one of them is not a number here
//!
//! A **control** action — describe, create, save — keeps the OFX host's two
//! seconds. A **block** does not: its deadline is the lookahead margin the
//! caller has left (docs/impl/audio-plugins.md §3), which is a fact about how
//! far ahead of the playhead the chain worker has got, not about the plugin. All
//! the table can say about a block is a **floor**: never give this plugin less
//! than so many milliseconds, for one that is honestly slow to start.

use std::time::Duration;

use serde::Deserialize;

/// The control-action deadline (docs/12 §2.3's two seconds, which describe,
/// create and save inherit unchanged).
const DEFAULT_CONTROL_TIMEOUT: Duration = Duration::from_secs(2);

/// One block's own length in time: 512 frames at 48 kHz, to the nanosecond.
///
/// The floor under every block deadline. A margin shorter than the block being
/// asked for is not a margin, it is a caller that has already fallen behind, and
/// giving the plugin less time than the sound it is being handed would fail
/// blocks that were never going to be late.
pub const BLOCK_PERIOD: Duration = Duration::from_nanos(10_666_667);

/// The whole table.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuirksTable {
    /// One entry per plugin that needs one. Order matters only in that the
    /// first entry matching an identifier wins.
    #[serde(default)]
    plugins: Vec<PluginQuirks>,
}

/// One plugin's entry.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginQuirks {
    /// The plugin's own stable identifier, matched exactly.
    identifier: String,
    /// Override the control-action deadline, in milliseconds.
    #[serde(default)]
    control_timeout_ms: Option<u64>,
    /// Raise the floor under the per-block deadline, in milliseconds.
    #[serde(default)]
    block_floor_ms: Option<u64>,
    /// Why this entry exists. Nothing branches on it; it is carried through so
    /// a diagnostic can say why a plugin is being treated specially.
    #[serde(default)]
    note: Option<String>,
}

/// The answers for one plugin, defaults filled in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quirks {
    /// The deadline for describe, create, save and the other control actions.
    pub control_timeout: Duration,
    /// The shortest deadline a block may be given, whatever margin the caller
    /// has left.
    pub block_floor: Duration,
    /// Why this plugin has an entry, for the diagnostics panel and for the next
    /// person to read the table.
    pub note: Option<String>,
}

impl Default for Quirks {
    fn default() -> Self {
        Self {
            control_timeout: DEFAULT_CONTROL_TIMEOUT,
            block_floor: BLOCK_PERIOD,
            note: None,
        }
    }
}

impl Quirks {
    /// The deadline one block gets: the caller's remaining lookahead margin,
    /// never less than the floor.
    #[must_use]
    pub fn block_deadline(&self, margin: Duration) -> Duration {
        margin.max(self.block_floor)
    }
}

impl QuirksTable {
    /// The table Lumit ships, embedded at build time so there is no file to be
    /// missing at run time.
    ///
    /// A malformed shipped file reads as an empty table rather than stopping the
    /// host: no plugin gets its workaround, which is a worse day than usual but
    /// not a dead editor. The test suite is what keeps the file well-formed.
    #[must_use]
    pub fn shipped() -> Self {
        Self::parse(include_str!("../quirks.json")).unwrap_or_default()
    }

    /// Parse a table.
    ///
    /// # Errors
    ///
    /// The `serde_json` error, so a test can say what is wrong with the file.
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// The answers for one plugin.
    #[must_use]
    pub fn for_plugin(&self, identifier: &str) -> Quirks {
        let mut quirks = Quirks::default();
        let Some(entry) = self
            .plugins
            .iter()
            .find(|entry| entry.identifier == identifier)
        else {
            return quirks;
        };
        if let Some(ms) = entry.control_timeout_ms {
            quirks.control_timeout = Duration::from_millis(ms);
        }
        if let Some(ms) = entry.block_floor_ms {
            quirks.block_floor = Duration::from_millis(ms);
        }
        quirks.note = entry.note.clone();
        quirks
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_table_parses_and_answers_the_defaults() {
        let table = QuirksTable::shipped();
        let quirks = table.for_plugin("com.vendor.anything");
        assert_eq!(quirks, Quirks::default());
        assert_eq!(quirks.control_timeout, Duration::from_secs(2));
        assert_eq!(quirks.block_floor, BLOCK_PERIOD);
    }

    #[test]
    fn a_block_deadline_is_the_margin_but_never_below_the_floor() {
        let quirks = Quirks::default();
        assert_eq!(
            quirks.block_deadline(Duration::from_millis(85)),
            Duration::from_millis(85),
            "a caller with lookahead in hand gets the lookahead"
        );
        assert_eq!(
            quirks.block_deadline(Duration::from_millis(1)),
            BLOCK_PERIOD,
            "a caller that has fallen behind still gets one block's worth"
        );
    }

    #[test]
    fn an_entry_overrides_only_what_it_names() {
        let table = QuirksTable::parse(
            r#"{"plugins":[{"identifier":"com.vendor.slow","block_floor_ms":50,
                "note":"warms up on the first block"}]}"#,
        )
        .unwrap();
        let quirks = table.for_plugin("com.vendor.slow");
        assert_eq!(quirks.block_floor, Duration::from_millis(50));
        assert_eq!(quirks.control_timeout, Duration::from_secs(2));
        assert!(quirks.note.is_some());
        assert_eq!(table.for_plugin("com.vendor.other"), Quirks::default());
    }
}
