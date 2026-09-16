//! The quirks table: per-plugin deviations, as data (docs/impl/lfx.md §3.5).
//!
//! # In plain terms
//!
//! Sooner or later one vendor's plugin will need longer than the shipped
//! deadline, or will need something else conceded to it. The older hosts learned
//! that the hard way and answered it with a table rather than with
//! `if identifier == …` sprinkled through the code, and this is the same table
//! for the same reason: the deviations live in `quirks.json`, keyed by the
//! plugin's own id, and the code only ever asks the table a question.
//!
//! LFX starts from a better place than OFX did - it is Lumit's own ABI, the
//! numbers are the header's, and a vendor who disagrees with one can be told to
//! fix the plugin - so the shipped table is **empty**, and an empty table that
//! parses is exactly the shipping default. It exists from the first day all the
//! same, because the day it is needed is a day nobody wants to be designing an
//! exception mechanism.
//!
//! The two numbers it holds are the ones docs/12 §2.3 gives every broker: ten
//! seconds for a frame, two for a control action. Nothing else in this crate
//! knows either number - the supervisor reads them from here - so changing the
//! shipped answer is one line and a superseding decision entry.

use std::time::Duration;

use serde::Deserialize;

/// How long a plugin may take over one frame before the watchdog counts a
/// strike. Ten seconds, because a frame can honestly take ten seconds.
const DEFAULT_PROCESS_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a control action may take: making an instance, putting values in
/// one, pressing a button, taking one down. Two seconds, because those happen
/// when nothing is rendering.
const DEFAULT_CONTROL_TIMEOUT: Duration = Duration::from_secs(2);

/// The whole table.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuirksTable {
    /// One entry per plugin that needs one. Order matters only in that the
    /// first entry matching an id wins.
    #[serde(default)]
    plugins: Vec<PluginQuirks>,
}

/// One plugin's entry.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginQuirks {
    /// The plugin's own reverse-DNS id, matched exactly - or, with a trailing
    /// `*`, a family of them under one prefix.
    id: String,
    /// Restrict the entry to one major version, or leave it out for all.
    #[serde(default)]
    version_major: Option<u32>,
    /// Override the frame deadline, in milliseconds.
    #[serde(default)]
    process_timeout_ms: Option<u64>,
    /// Override the control-action deadline, in milliseconds.
    #[serde(default)]
    control_timeout_ms: Option<u64>,
    /// Why this entry exists. Nothing branches on it; it is carried through so
    /// a diagnostic can say why a plugin is being treated specially.
    #[serde(default)]
    note: Option<String>,
}

/// The answers for one plugin, defaults filled in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quirks {
    /// How long one frame may take.
    pub process_timeout: Duration,
    /// How long describe, create, values, action and destroy may take.
    pub control_timeout: Duration,
    /// Why this plugin has an entry, for the scan report and for the next
    /// person to read the table.
    pub note: Option<String>,
}

impl Default for Quirks {
    fn default() -> Self {
        Self {
            process_timeout: DEFAULT_PROCESS_TIMEOUT,
            control_timeout: DEFAULT_CONTROL_TIMEOUT,
            note: None,
        }
    }
}

impl QuirksTable {
    /// The table Lumit ships, embedded at build time so there is no file to be
    /// missing at run time.
    ///
    /// A malformed shipped file reads as an empty table rather than stopping
    /// the host: no plugin gets its workaround, which is a worse day than usual
    /// but not a dead editor. The suite is what keeps the file well-formed.
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
    pub fn for_plugin(&self, id: &str, version_major: u32) -> Quirks {
        let mut quirks = Quirks::default();
        let Some(entry) = self.plugins.iter().find(|entry| {
            entry
                .id
                .strip_suffix('*')
                .map_or(entry.id == id, |prefix| id.starts_with(prefix))
                && entry
                    .version_major
                    .is_none_or(|major| major == version_major)
        }) else {
            return quirks;
        };
        if let Some(ms) = entry.process_timeout_ms {
            quirks.process_timeout = Duration::from_millis(ms);
        }
        if let Some(ms) = entry.control_timeout_ms {
            quirks.control_timeout = Duration::from_millis(ms);
        }
        quirks.note.clone_from(&entry.note);
        quirks
    }
}

/// How long a describe may take: the handshake's ceiling, or a control deadline
/// the table set longer than it.
///
/// Describe is filed with the control actions, but the first one is not a
/// plugin thinking - it is the broker **opening the module from disk** and
/// asking every plugin in it what it is, on a process that has only just said
/// hello. The rule is `lumit-ipc`'s, because every host must answer it the same
/// way; the number it is given is this host's.
#[must_use]
pub fn describe_deadline(quirks: &Quirks) -> Duration {
    lumit_ipc::describe_deadline(quirks.control_timeout)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// The shipped file parses, and is empty. Both halves matter: a table that
    /// did not parse would silently concede nothing to anybody, and an entry
    /// nobody has argued for is a workaround with no plugin behind it.
    #[test]
    fn the_shipped_quirks_table_parses_and_concedes_nothing() {
        let table =
            QuirksTable::parse(include_str!("../quirks.json")).expect("the shipped table parses");
        assert!(
            table.plugins.is_empty(),
            "an entry was added without a reason beside it"
        );
        assert_eq!(
            table.for_plugin("com.example.blur", 1),
            Quirks::default(),
            "an empty table is the shipping default"
        );
    }

    /// Ten seconds for a frame and two for a control action, from docs/12 §2.3 -
    /// and nothing else in the crate knows either number.
    #[test]
    fn the_shipped_deadlines_are_the_ones_the_spec_gives() {
        let quirks = Quirks::default();
        assert_eq!(quirks.process_timeout, Duration::from_secs(10));
        assert_eq!(quirks.control_timeout, Duration::from_secs(2));
    }

    /// An entry is the exception mechanism, and it is per plugin and per family.
    #[test]
    fn an_entry_overrides_the_deadlines_for_the_plugins_it_names() {
        let table = QuirksTable::parse(
            r#"{"plugins":[
                {"id":"com.example.slow","process_timeout_ms":30000,"note":"Bakes a LUT on the first frame"},
                {"id":"com.family.*","control_timeout_ms":5000}
            ]}"#,
        )
        .expect("a table");
        let slow = table.for_plugin("com.example.slow", 1);
        assert_eq!(slow.process_timeout, Duration::from_secs(30));
        assert_eq!(slow.control_timeout, Duration::from_secs(2));
        assert!(slow.note.is_some(), "an entry says why it exists");

        let family = table.for_plugin("com.family.anything", 4);
        assert_eq!(family.control_timeout, Duration::from_secs(5));
        assert_eq!(
            table.for_plugin("com.elsewhere.thing", 1),
            Quirks::default(),
            "a plugin nobody named gets the shipped answers"
        );
    }

    /// An entry may be pinned to one major version, for a vendor who fixed it.
    #[test]
    fn an_entry_pinned_to_a_major_version_leaves_the_others_alone() {
        let table = QuirksTable::parse(
            r#"{"plugins":[{"id":"com.example.fixed","version_major":1,"process_timeout_ms":30000}]}"#,
        )
        .expect("a table");
        assert_eq!(
            table.for_plugin("com.example.fixed", 1).process_timeout,
            Duration::from_secs(30)
        );
        assert_eq!(
            table.for_plugin("com.example.fixed", 2),
            Quirks::default(),
            "the release that fixed it gets no concession"
        );
    }

    /// The first describe opens a module from disk on a process that has only
    /// just started, so it waits under the handshake's ceiling rather than the
    /// two-second control deadline - unless the table asks for longer.
    #[test]
    fn describe_takes_the_handshake_ceiling_unless_the_table_asks_for_longer() {
        let shipped = Quirks::default();
        assert!(shipped.control_timeout < lumit_ipc::HANDSHAKE_TIMEOUT);
        assert_eq!(describe_deadline(&shipped), lumit_ipc::HANDSHAKE_TIMEOUT);

        let patient = Quirks {
            control_timeout: Duration::from_secs(30),
            ..Quirks::default()
        };
        assert_eq!(describe_deadline(&patient), Duration::from_secs(30));
    }
}
