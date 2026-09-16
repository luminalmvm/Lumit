//! Every addon this machine has ever seen (docs/impl/lfx.md §5.3).
//!
//! # In plain terms
//!
//! Two of the three tables the Addons page is drawn from live for a session
//! only: what the last scan registered, and what it turned away. Both are
//! refilled at every launch, and both are empty about a plugin that was never
//! offered at all - one switched off before the scan, one whose folder has gone
//! since, one on a machine this session has not scanned.
//!
//! The scan does *mention* such a plugin: the report carries a skip line naming
//! it and saying "switched off in preferences". What a skip line is not is a
//! **row** - a label, a vendor, a version, a kind and a state, which is what
//! the Installed section is made of and what nothing can parse a sentence into.
//! So there is a third table, and it is the only one that outlives the process:
//! everything ever seen, by identifier, with when it was last seen and why it
//! was last turned away.
//!
//! One file for four kinds of plugin, beside `plugins.json`, because the page
//! lists all four together and a roster per host would be four files to keep in
//! step.
//!
//! # Losing it costs a rescan and never an error
//!
//! An absent or damaged file reads as **nothing seen**: the page opens with
//! whatever this session's scan found and fills in again at the next one. That
//! is the same fail-open shape [`crate::PluginPrefs`] has, and it is right here
//! for the same reason - this is a record of what was noticed, not a record of
//! what is trusted. §6.2's key fingerprints are the opposite case and live in a
//! file of their own, where damaged is a refusal rather than a first use
//! (§11 item 16).
//!
//! It is read under `lumit_ingress::Limits::PLUGIN_ROSTER`: every string in it
//! was copied out of somebody else's manifest, and the application's data area
//! roams, so the copy being read need not be the copy this machine wrote.
//!
//! The **writer is held to the same ceiling**, by forgetting the rows seen
//! longest ago until the file fits. One bundle's honest listing can be several
//! megabytes of roster all by itself, and a write with no ceiling would put the
//! file permanently past the reader - which answers "nothing has ever been
//! seen" for every plugin of every kind on the machine, silently. Forgetting
//! the oldest costs a rescan; writing a file that cannot be read back costs the
//! whole of the Installed section.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use lumit_ingress::Limits;
use serde::{Deserialize, Serialize};

/// Where the roster is kept. `None` only when the platform has no home
/// directory, in which case nothing is remembered and nothing is an error.
#[must_use]
pub fn plugin_roster_path() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "Lumit", "Lumit")?;
    Some(dirs.data_dir().join("addons.json"))
}

/// Which host offered a plugin.
///
/// A closed vocabulary of four, and the spellings are the ones the bridge's
/// `ADDON_KINDS` publishes to Dart, so the roster, the page and the listing
/// cannot come to three opinions about what an OFX plugin is called.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginKind {
    /// Lumit's own native plugin API (docs/12 §3).
    Lfx,
    /// OpenFX (docs/12 §2).
    Ofx,
    /// CLAP audio plugins (docs/12 §4).
    Clap,
    /// VST3 audio plugins (docs/12 §4).
    Vst3,
}

impl PluginKind {
    /// All four, in the order the page groups them.
    pub const ALL: [PluginKind; 4] = [
        PluginKind::Lfx,
        PluginKind::Ofx,
        PluginKind::Clap,
        PluginKind::Vst3,
    ];

    /// The one word this kind is spelled with everywhere it crosses a boundary.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            PluginKind::Lfx => "lfx",
            PluginKind::Ofx => "ofx",
            PluginKind::Clap => "clap",
            PluginKind::Vst3 => "vst3",
        }
    }

    /// The kind that word names, or `None` for a word outside the four.
    #[must_use]
    pub fn from_word(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == word)
    }
}

/// One plugin, as the roster remembers it.
///
/// Everything here can be filled in from a bundle's own listing, which is what
/// makes the row available for a plugin whose code has never run.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct RosterEntry {
    /// The plugin's own identifier - what the switched-off list names, and the
    /// one thing about it that survives an upgrade or a move.
    pub identifier: String,
    /// What a person reads.
    pub label: String,
    /// Who wrote it.
    pub vendor: String,
    /// `major.minor.patch`, as text, because that is how a vendor writes a
    /// release and the page only ever prints it.
    pub version: String,
    /// The bundle or file it came out of, as text: the page's description line,
    /// and never a path this crate opens.
    pub location: String,
    /// Seconds since the epoch at the last scan that saw it; nought for a
    /// plugin written down by an installer and not yet scanned.
    pub last_seen: u64,
    /// Why the last scan turned it away, if one did. Cleared by a scan that
    /// registered it, so a plugin that was mended stops wearing its old
    /// sentence.
    pub last_refusal: Option<String>,
}

/// Everything ever seen, by kind and identifier.
///
/// The map's key is `<kind>:<identifier>` rather than the identifier alone: two
/// standards may legitimately use the same reverse-DNS name for the same
/// vendor's two builds of one effect, and a roster that folded them together
/// would show one row that switched two plugins off.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct PluginRoster {
    /// One record per plugin, sorted - so the file cannot grow a duplicate and
    /// its order cannot drift between saves.
    pub entries: BTreeMap<String, RosterEntry>,
}

impl PluginRoster {
    /// The key one kind's identifier is filed under.
    #[must_use]
    pub fn key(kind: PluginKind, identifier: &str) -> String {
        format!("{}:{identifier}", kind.as_str())
    }

    /// Read the roster from `path`. An absent, over-long or damaged file is
    /// "nothing has ever been seen".
    #[must_use]
    pub fn load(path: &Path) -> Self {
        lumit_ingress::read_to_string_capped(path, Limits::PLUGIN_ROSTER.bytes)
            .ok()
            .and_then(|text| serde_json::from_str::<Self>(&text).ok())
            .unwrap_or_default()
    }

    /// Read from [`plugin_roster_path`], or an empty roster when there is none.
    #[must_use]
    pub fn load_default() -> Self {
        plugin_roster_path()
            .map(|path| Self::load(&path))
            .unwrap_or_default()
    }

    /// What the roster remembers about one plugin.
    #[must_use]
    pub fn get(&self, kind: PluginKind, identifier: &str) -> Option<&RosterEntry> {
        self.entries.get(&Self::key(kind, identifier))
    }

    /// Every record of one kind, in identifier order.
    #[must_use]
    pub fn of_kind(&self, kind: PluginKind) -> Vec<&RosterEntry> {
        let prefix = format!("{}:", kind.as_str());
        self.entries
            .iter()
            .filter(|(key, _)| key.starts_with(&prefix))
            .map(|(_, entry)| entry)
            .collect()
    }

    /// Write down that a scan saw this plugin and could offer it.
    ///
    /// The timestamp is taken here rather than passed in, and the old refusal
    /// is **cleared**: a plugin that registered this time is not still refused,
    /// and a row wearing last month's sentence is worse than a row with none.
    pub fn saw(&mut self, kind: PluginKind, entry: RosterEntry) {
        let key = Self::key(kind, &entry.identifier);
        self.entries.insert(
            key,
            RosterEntry {
                last_seen: now_seconds(),
                last_refusal: None,
                ..entry
            },
        );
    }

    /// Write down that a scan saw this plugin and turned it away.
    ///
    /// `last_seen` moves too: the plugin *was* there, which is exactly what
    /// tells a refusal apart from a plugin whose folder has gone.
    pub fn refused(&mut self, kind: PluginKind, entry: RosterEntry, why: &str) {
        let key = Self::key(kind, &entry.identifier);
        self.entries.insert(
            key,
            RosterEntry {
                last_seen: now_seconds(),
                last_refusal: Some(why.to_owned()),
                ..entry
            },
        );
    }

    /// Write the roster to `path`, creating the directory - **held to the same
    /// ceiling [`PluginRoster::load`] reads under**, by forgetting the oldest
    /// rows until it fits.
    ///
    /// The writer's ceiling is not a symmetry for its own sake. One bundle's
    /// listing may honestly declare `LFX_MAX_EFFECTS_PER_BUNDLE` plugins with
    /// strings up to `LFX_MAX_STRING_BYTES` each, which is several megabytes of
    /// roster on its own - past `Limits::PLUGIN_ROSTER`, which the reader
    /// answers by reading nothing at all. A writer with no ceiling would let
    /// one stranger's manifest put the file permanently past the reader and
    /// blank the Installed section for every plugin of every kind on the
    /// machine, silently, which is failure 6 reopened by the file that exists
    /// to close it.
    ///
    /// **The oldest are what goes.** A roster is a record of what was noticed,
    /// so forgetting what was noticed longest ago is the loss that costs least:
    /// a plugin still on the machine gets its row back at the next scan, and
    /// one whose folder has gone was never going to.
    ///
    /// # Errors
    ///
    /// A sentence naming what went wrong: the directory could not be made, the
    /// roster would not serialise, or the file would not be written.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("plugin roster: {e}"))?;
        }
        let cap = usize::try_from(Limits::PLUGIN_ROSTER.bytes).unwrap_or(usize::MAX);
        let mut text = written(self)?;
        if text.len() > cap {
            let mut pruned = self.clone();
            while text.len() > cap && pruned.forget_oldest(text.len() - cap) {
                text = written(&pruned)?;
            }
        }
        std::fs::write(path, text).map_err(|e| format!("plugin roster: {e}"))
    }

    /// Forget the rows seen longest ago, until at least `excess` bytes of
    /// them are gone. `false` when there was nothing left to forget, which is
    /// what ends [`PluginRoster::save`]'s loop.
    ///
    /// Measured rather than counted: entries differ by a factor of hundreds in
    /// length, so dropping a fixed number would either take a thousand rows off
    /// a roster that was one row over or go round a thousand times.
    fn forget_oldest(&mut self, excess: usize) -> bool {
        let mut oldest: Vec<(u64, String)> = self
            .entries
            .iter()
            .map(|(key, entry)| (entry.last_seen, key.clone()))
            .collect();
        oldest.sort();

        let mut forgotten = 0usize;
        let mut any = false;
        for (_, key) in oldest {
            let Some(entry) = self.entries.remove(&key) else {
                continue;
            };
            any = true;
            forgotten +=
                key.len() + serde_json::to_string(&entry).map_or(0, |written| written.len());
            if forgotten >= excess {
                break;
            }
        }
        any
    }

    /// Save to [`plugin_roster_path`]; a machine with no home directory keeps
    /// the answer for the session and says so calmly.
    ///
    /// # Errors
    ///
    /// As [`PluginRoster::save`], plus the machine having nowhere to keep it.
    pub fn save_default(&self) -> Result<(), String> {
        match plugin_roster_path() {
            Some(path) => self.save(&path),
            None => Err("this machine has no place to keep the plugin roster".to_owned()),
        }
    }
}

/// The roster as it goes on disk.
fn written(roster: &PluginRoster) -> Result<String, String> {
    serde_json::to_string_pretty(roster).map_err(|e| format!("plugin roster: {e}"))
}

/// Now, in seconds since the epoch. A clock set before 1970 reads as nought,
/// which is the same as "never seen" and is the right answer for a machine
/// whose clock cannot be believed.
fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// One entry, spelled the way a bundle's own listing would fill it in.
    fn a_blur() -> RosterEntry {
        RosterEntry {
            identifier: "com.example.blur".into(),
            label: "Example blur".into(),
            vendor: "Example".into(),
            version: "2.3.4".into(),
            location: "/plugins/Example.lfx.bundle".into(),
            ..RosterEntry::default()
        }
    }

    /// The answer to failure 6: a plugin switched off before a scan has no
    /// session row anywhere, and the page still draws it with a label, a
    /// vendor, a version and a place it came from.
    #[test]
    fn the_roster_remembers_a_plugin_no_session_table_holds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("addons.json");

        let mut roster = PluginRoster::default();
        roster.saw(PluginKind::Lfx, a_blur());
        roster.save(&path).unwrap();

        let read = PluginRoster::load(&path);
        let entry = read
            .get(PluginKind::Lfx, "com.example.blur")
            .expect("the row");
        assert_eq!(entry.label, "Example blur");
        assert_eq!(entry.vendor, "Example");
        assert_eq!(entry.version, "2.3.4");
        assert!(entry.last_seen > 0, "a scan saw it");
        assert_eq!(entry.last_refusal, None);
        assert!(
            read.get(PluginKind::Ofx, "com.example.blur").is_none(),
            "the kind is part of the key"
        );
    }

    /// A refusal is remembered with its own sentence, and a later scan that
    /// registered the plugin takes the sentence away rather than leaving the
    /// row wearing last month's reason.
    #[test]
    fn a_refusal_is_remembered_and_a_later_scan_clears_it() {
        let mut roster = PluginRoster::default();
        roster.refused(
            PluginKind::Lfx,
            a_blur(),
            "it requires the extension \"lfx.temporal\"",
        );
        let entry = roster
            .get(PluginKind::Lfx, "com.example.blur")
            .expect("the row");
        assert!(entry.last_refusal.is_some());
        assert!(entry.last_seen > 0, "it was there to be refused");

        roster.saw(PluginKind::Lfx, a_blur());
        assert_eq!(
            roster
                .get(PluginKind::Lfx, "com.example.blur")
                .and_then(|entry| entry.last_refusal.clone()),
            None
        );
        assert_eq!(roster.entries.len(), 1, "one plugin is one row");
    }

    /// Fail-open, like every other preference here: losing the roster costs a
    /// rescan and never a dialogue.
    #[test]
    fn a_missing_or_damaged_roster_reads_as_nothing_ever_seen() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            PluginRoster::load(&dir.path().join("not-there.json")),
            PluginRoster::default()
        );

        let damaged = dir.path().join("damaged.json");
        std::fs::write(&damaged, "{ not json").unwrap();
        assert_eq!(PluginRoster::load(&damaged), PluginRoster::default());
    }

    /// The file is a stranger's strings and the application's data area roams,
    /// so an over-long roster is refused **without being read** rather than
    /// believed.
    ///
    /// The over-long file is a roster that would plainly load if the ceiling
    /// were gone - one real entry, padded inside its own label - and the short
    /// one beside it is what keeps the assertion honest: a fixture of fields
    /// this struct ignores would pass with the cap taken away, which would pin
    /// nothing but `serde`'s own manners.
    ///
    /// The over-long file is written **round** [`PluginRoster::save`], which
    /// now holds itself to the reader's ceiling: this case is about a file that
    /// arrived some other way - roamed from a machine whose Lumit had no such
    /// ceiling, or edited by hand - and `a_roster_too_big_for_its_own_ceiling_
    /// is_pruned_rather_than_written` is the writer's half.
    #[test]
    fn a_roster_past_the_ingress_ceiling_reads_as_nothing_ever_seen() {
        let dir = tempfile::tempdir().unwrap();
        let cap = usize::try_from(Limits::PLUGIN_ROSTER.bytes).unwrap_or(usize::MAX);

        let short = dir.path().join("short.json");
        let mut inside = PluginRoster::default();
        inside.saw(PluginKind::Lfx, a_blur());
        inside.save(&short).unwrap();
        assert!(
            PluginRoster::load(&short)
                .get(PluginKind::Lfx, "com.example.blur")
                .is_some(),
            "a roster under the ceiling comes back"
        );

        let huge = dir.path().join("huge.json");
        let mut past = PluginRoster::default();
        past.saw(
            PluginKind::Lfx,
            RosterEntry {
                label: "x".repeat(cap),
                ..a_blur()
            },
        );
        std::fs::write(&huge, serde_json::to_string_pretty(&past).unwrap()).unwrap();
        assert!(
            std::fs::metadata(&huge).unwrap().len() > Limits::PLUGIN_ROSTER.bytes,
            "the fixture is past the ceiling it is testing"
        );
        assert_eq!(PluginRoster::load(&huge), PluginRoster::default());
        assert!(
            PluginRoster::load(&huge)
                .get(PluginKind::Lfx, "com.example.blur")
                .is_none(),
            "and the entry it carried never reached the roster"
        );
    }

    /// The writer is held to the reader's ceiling: a roster past it is pruned
    /// to fit rather than written whole, and what it keeps is what was seen
    /// most recently.
    ///
    /// Without this, one bundle declaring `LFX_MAX_EFFECTS_PER_BUNDLE` plugins
    /// with long strings is enough to put `addons.json` permanently past
    /// [`PluginRoster::load`], which answers "nothing has ever been seen" - for
    /// every plugin of every kind on the machine, fail-open and silent. That is
    /// failure 6 reopened by the file that exists to close it.
    #[test]
    fn a_roster_too_big_for_its_own_ceiling_is_pruned_rather_than_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("addons.json");
        let cap = usize::try_from(Limits::PLUGIN_ROSTER.bytes).unwrap_or(usize::MAX);

        // Four honest rows, each a fifth of the ceiling in its own label, seen
        // one after another - the shape a stranger's listing makes, in
        // miniature. `saw` would stamp them all with the same second, so the
        // ages are written down here.
        let mut roster = PluginRoster::default();
        for (age, name) in ["oldest", "older", "newer", "newest"].iter().enumerate() {
            let entry = RosterEntry {
                identifier: format!("com.example.{name}"),
                label: "x".repeat(cap / 3),
                last_seen: age as u64 + 1,
                ..a_blur()
            };
            roster
                .entries
                .insert(PluginRoster::key(PluginKind::Lfx, &entry.identifier), entry);
        }
        assert!(
            serde_json::to_string_pretty(&roster).unwrap().len() > cap,
            "the fixture is past the ceiling it is testing"
        );

        roster.save(&path).unwrap();
        assert!(
            std::fs::metadata(&path).unwrap().len() <= Limits::PLUGIN_ROSTER.bytes,
            "what landed is inside the ceiling its own reader holds"
        );

        let read = PluginRoster::load(&path);
        assert_ne!(
            read,
            PluginRoster::default(),
            "and it reads back, which is the whole of the point"
        );
        assert!(
            read.get(PluginKind::Lfx, "com.example.newest").is_some(),
            "the row seen most recently is the one kept"
        );
        assert!(
            read.get(PluginKind::Lfx, "com.example.oldest").is_none(),
            "and the one seen longest ago is what was forgotten"
        );
        assert!(
            !read.entries.is_empty() && read.entries.len() < 4,
            "pruned rather than emptied: {}",
            read.entries.len()
        );

        // A roster that already fits is written untouched - the ceiling is a
        // last resort, not a policy every save runs.
        let small = dir.path().join("small.json");
        let mut ordinary = PluginRoster::default();
        ordinary.saw(PluginKind::Lfx, a_blur());
        ordinary.save(&small).unwrap();
        assert_eq!(PluginRoster::load(&small), ordinary);
    }

    /// The four spellings are the ones the bridge publishes to Dart, and they
    /// round-trip both ways so a word read out of the file lands on the kind
    /// that wrote it.
    #[test]
    fn every_plugin_kind_is_one_word_that_round_trips() {
        for kind in PluginKind::ALL {
            assert_eq!(PluginKind::from_word(kind.as_str()), Some(kind));
        }
        assert_eq!(PluginKind::from_word("lv2"), None);
        assert_eq!(
            PluginKind::ALL.map(PluginKind::as_str).to_vec(),
            vec!["lfx", "ofx", "clap", "vst3"],
            "docs/impl/lfx.md §7.2's ADDON_KINDS, in its order"
        );
    }
}
