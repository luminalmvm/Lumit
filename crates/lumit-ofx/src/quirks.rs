//! The quirks table: per-plugin deviations, as data.
//!
//! # In plain terms
//!
//! Every OFX host implements the standard slightly differently, and every
//! commercial plugin carries a table of workarounds for the hosts it knows.
//! Lumit needs the mirror image of that table, and it needs it from the first
//! day rather than after the workarounds have been sprinkled through the code
//! as `if identifier == …` (docs/12 §2.5). So the deviations live in
//! `quirks.json`, keyed by the plugin's own identifier and version, and the
//! code only ever asks the table a question.
//!
//! Today the table is empty, and an empty table that parses is exactly the
//! shipping default: no plugin has yet earned an entry.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Deserialize;

/// Watchdog defaults, from docs/12 §2.3: ten seconds for a render, two for a
/// control action. docs/impl/ofx-host.md §4 sketches thirty for a render; the
/// spec's number is the one that ships and K-592 records why. Nothing else in
/// the code knows either number — the broker reads them from here — so changing
/// the shipped answer is this line and a superseding decision entry, and a
/// single plugin that needs longer needs no code change at all: its
/// `render_timeout_ms` in `quirks.json` is the exception mechanism.
const DEFAULT_RENDER_TIMEOUT: Duration = Duration::from_secs(10);
/// See [`DEFAULT_RENDER_TIMEOUT`].
const DEFAULT_CONTROL_TIMEOUT: Duration = Duration::from_secs(2);

/// The whole table.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuirksTable {
    /// One entry per plugin that needs one. Order matters only in that the
    /// first entry matching an identifier and version wins.
    #[serde(default)]
    plugins: Vec<PluginQuirks>,
}

/// One plugin's entry.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginQuirks {
    /// The plugin's own `pluginIdentifier`, matched exactly.
    identifier: String,
    /// Restrict the entry to one major version, or leave it out for all.
    #[serde(default)]
    version_major: Option<u32>,
    /// Override the render deadline, in milliseconds.
    #[serde(default)]
    render_timeout_ms: Option<u64>,
    /// Override the control-action deadline, in milliseconds.
    #[serde(default)]
    control_timeout_ms: Option<u64>,
    /// Pin a suite to a version other than the newest we have, for a plugin
    /// that asks for a version it then uses incorrectly.
    #[serde(default)]
    suite_versions: BTreeMap<String, i32>,
    /// Why this entry exists. Nothing branches on it; it is carried through
    /// so a diagnostic can say why a plugin is being treated specially.
    #[serde(default)]
    note: Option<String>,
}

/// The answers for one plugin, defaults filled in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quirks {
    /// The render deadline for this plugin.
    pub render_timeout: Duration,
    /// The deadline for describe, instance and other control actions.
    pub control_timeout: Duration,
    /// Suite versions this plugin must be given, by suite name.
    pub suite_versions: BTreeMap<String, i32>,
    /// Why this plugin has an entry, for the diagnostics panel and for the
    /// next person to read the table.
    pub note: Option<String>,
}

impl Default for Quirks {
    fn default() -> Self {
        Self {
            render_timeout: DEFAULT_RENDER_TIMEOUT,
            control_timeout: DEFAULT_CONTROL_TIMEOUT,
            suite_versions: BTreeMap::new(),
            note: None,
        }
    }
}

impl QuirksTable {
    /// The table Lumit ships, embedded at build time so there is no file to
    /// be missing at run time.
    ///
    /// A malformed shipped file reads as an empty table rather than stopping
    /// the host: no plugin gets its workaround, which is a worse day than
    /// usual but not a dead editor. The test suite is what keeps the file
    /// well-formed.
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
    pub fn for_plugin(&self, identifier: &str, version_major: u32) -> Quirks {
        let mut quirks = Quirks::default();
        let Some(entry) = self.plugins.iter().find(|entry| {
            entry.identifier == identifier
                && entry
                    .version_major
                    .is_none_or(|major| major == version_major)
        }) else {
            return quirks;
        };
        if let Some(ms) = entry.render_timeout_ms {
            quirks.render_timeout = Duration::from_millis(ms);
        }
        if let Some(ms) = entry.control_timeout_ms {
            quirks.control_timeout = Duration::from_millis(ms);
        }
        quirks.suite_versions = entry.suite_versions.clone();
        quirks.note = entry.note.clone();
        quirks
    }
}
