//! Finding the LFX plugins installed on this machine, and offering them
//! (docs/impl/lfx.md §5).
//!
//! # In plain terms
//!
//! Everything else in this crate could host a bundle somebody handed it by
//! path. This is the part that goes looking: it walks the search paths, spawns
//! one broker per bundle, has that broker read the bundle's own listing
//! **before any of the plugin's code**, then describe whatever is not switched
//! off, and turns what comes back into catalogue rows and report lines. After
//! that an LFX plugin is an effect like any other - it appears in Effects &
//! presets under the family it declared, it applies to a layer, it keyframes.
//!
//! Four rules the scan follows, three of them the older hosts' and one new:
//!
//! * **A bundle that will not load is a line in a report, never a dialogue.**
//!   Somebody else's installer left a broken folder on the machine; that is not
//!   worth interrupting the person for, and it must not cost them the other
//!   bundles beside it.
//! * **The user's switched-off list is consulted before describe**, not after.
//!   It travels with `HostMessage::Describe`, so a switched-off plugin is never
//!   described and none of its own code runs; and a bundle with nothing left to
//!   describe is not asked for one at all, so its module is never opened and
//!   its `init` never runs. That is the first of the three places a disable
//!   reaches (§5.4).
//! * **A rescan adds; it never replaces.** The discovered table is keyed by match
//!   name and a name it already holds is skipped before any work is done, which
//!   is also what stops a rescan leaking a second copy of a schema it has.
//! * **Never on the interface's thread.** Spawning brokers and running other
//!   people's start-up code is a plain blocking function; the caller puts it on
//!   a worker.
//!
//! # The three tables (§5.3)
//!
//! [`discovered`] is what registered this session and [`refusals`] is what the
//! scan turned away, each with its own typed [`LfxRejection`] rather than a
//! sentence somebody has to parse. Both are session tables, refilled by the
//! start-up scan at every launch.
//!
//! The third is the roster, and it is the answer to failure 6: a plugin
//! switched off *before* a scan is **mentioned** by the report - one skip line
//! carrying its id - and has no structured row anywhere, and a label, a vendor,
//! a version and a kind is what the Installed section is made of. So a scan
//! hands back [`ScanOutcome::listed`] as well: every plugin every bundle's
//! listing declared, whatever became of it. The roster itself is a file in the
//! application's data area and belongs to `lumit-project`; writing it is the
//! composition root's, because a plugin host that read the preference area
//! would be a plugin host that depends on the project format.
//!
//! # One broker per bundle, spawned and shut down again
//!
//! `lumit-aplug`'s arrangement rather than `lumit-ofx`'s: the scan opens a
//! broker, asks it for the listing and the description, and drops it. A layer
//! that later wants a live instance gets a broker of its own. A bundle that
//! takes its process down while loading is one line in the report and costs the
//! rest of the folder nothing.
//!
//! There is **no in-process scan**, and that is not an omission. docs/12:354-356
//! forbids an in-process path in version 1, and the listing is a stranger's
//! structured text that must be read in the broker rather than in the process
//! holding the project, the media handles and the windows (§11 item 7). Nothing
//! in this module reads or parses a bundle's own TOML; the listing arrives
//! through [`Broker::manifest`] or it does not arrive, and
//! `the_host_never_parses_the_bundles_own_toml` holds that for this file as
//! well as for the supervisor.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};

use lumit_budget::Ledger;
use lumit_core::fx::{EffectSchema, FxCategory, LFX_MATCH_PREFIX};

use crate::bundle;
use crate::ipc::broker::Picture;
use crate::ipc::broker::{
    nothing_disabled, Broker, BrokerConfig, BrokerError, DisableList, ProcessJob, Rendered,
};
use crate::ipc::proto::{InstanceId, PixelDepth, PluginIdentity};
use crate::ipc::ring::RingPlan;
use crate::{schema, LfxRejection};

/// What a switched-off plugin files under its instance. Read as a **key** by
/// the seam that badges the layer, never shown verbatim.
///
/// `lumit-ipc`'s rather than this host's, and re-exported here so the name is
/// where every caller already looks for it (§4.3).
pub use lumit_ipc::DISABLED_REASON;

/// The frame size a scan's brokers size their ring for.
///
/// The scan happens before any composition is open, so there is no comp size to
/// size a ring from, and 1080p is the OFX host's honest guess for the same
/// reason: it is the commonest delivery. Half floats, because that is what
/// Lumit's working texture holds; an fp32 project regrows the ring at its first
/// frame, and these brokers are shut down before any frame at all.
pub const SCAN_FRAME: (u32, u32) = (1920, 1080);

// ------------------------------------------------------------ what is found --

/// The facts about one plugin that a bundle's own listing is enough to fill in.
///
/// Shared by [`DiscoveredPlugin`], [`Refusal`] and [`ScanOutcome::listed`] on
/// purpose: they are the same row in three states, and three structs carrying
/// the same five fields is three places for the Addons page to disagree with
/// itself.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AddonRow {
    /// The plugin's own reverse-domain identifier - what the switched-off list
    /// names, and the one thing that survives an upgrade or a move.
    pub identifier: String,
    /// The name a person sees.
    pub label: String,
    /// Who wrote it.
    pub vendor: String,
    /// Major, minor and patch, as the plugin declares them.
    pub release: (u32, u32, u32),
    /// Which bundle directory it came out of.
    pub bundle: PathBuf,
}

impl AddonRow {
    /// The release as a person writes it.
    #[must_use]
    pub fn version(&self) -> String {
        let (major, minor, patch) = self.release;
        format!("{major}.{minor}.{patch}")
    }

    /// The row one entry of a bundle's listing declares.
    fn of(identity: &PluginIdentity, bundle: &Path) -> Self {
        Self {
            identifier: identity.id.clone(),
            label: identity.name.clone(),
            vendor: identity.vendor.clone(),
            release: (identity.major, identity.minor, identity.patch),
            bundle: bundle.to_path_buf(),
        }
    }
}

/// One plugin that became a catalogue entry.
#[derive(Clone, Debug)]
pub struct DiscoveredPlugin {
    /// The name the catalogue answers to - `lfx:` and the plugin's identifier.
    pub match_name: String,
    /// Who it is and where it came from.
    pub row: AddonRow,
    /// The picture families it claimed, **the heading first**; the rest are
    /// search keywords the browser matches on (§2.4).
    pub categories: Vec<FxCategory>,
    /// The declaration everything downstream sees. Leaked for the session, as
    /// every hosted schema is, and minted once per match name however many
    /// times the folder is scanned.
    pub schema: &'static EffectSchema,
}

/// One plugin the scan turned away, and why.
///
/// The reason is the [`LfxRejection`] itself rather than a rendered sentence:
/// the Addons page prints it, `lfx-validator` asks for one by name, and a badge
/// that matched on prose is the coupling §4.3 records as a trap.
#[derive(Clone, Debug, PartialEq)]
pub struct Refusal {
    /// Who it was.
    pub row: AddonRow,
    /// Which no it got.
    pub why: LfxRejection,
}

/// What a scan was asked to do.
#[derive(Clone, Debug, Default)]
pub struct ScanOptions {
    /// The directories to look in. [`ScanOptions::standard`] fills this from
    /// [`bundle::search_paths`].
    pub paths: Vec<PathBuf>,
    /// Where the broker executable is, if not beside Lumit's own - for a test
    /// or a build tree.
    pub exe: Option<PathBuf>,
    /// Extra environment for each broker. Lumit sets none of its own; the tests
    /// use it to tell a plugin to misbehave on purpose.
    pub env: Vec<(String, String)>,
}

impl ScanOptions {
    /// The standard search paths - what start-up asks for once the preference
    /// has been read into the running list with [`set_disabled`].
    ///
    /// **There is no switched-off list here**, deliberately, and that is where
    /// this parts company with the two older hosts' `ScanOptions`. A scan reads
    /// the running table and hands the broker a share of it, so a second copy
    /// on the options would be a second opinion: a field the scan merged into
    /// the running list could never take an identifier back out of it, and a
    /// field the scan replaced it with would wipe the preference every time
    /// start-up asked for the standard paths. One table, seeded by
    /// [`set_disabled`] and edited by [`set_enabled`], is what makes §5.4's
    /// three places agree.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            paths: bundle::search_paths(),
            ..Self::default()
        }
    }
}

/// What one scan did.
#[derive(Clone, Debug, Default)]
pub struct ScanOutcome {
    /// The plugins registered **by this scan**. A rescan that finds nothing new
    /// answers with an empty list, which is the normal case.
    pub registered: Vec<DiscoveredPlugin>,
    /// The plugins this scan turned away, each with its own refusal.
    pub refused: Vec<Refusal>,
    /// Every plugin every bundle's listing declared, registered or not - the
    /// roster's raw material, and the row a plugin switched off before the scan
    /// would otherwise not have (§5.3).
    pub listed: Vec<AddonRow>,
    /// One calm sentence per bundle or plugin turned away, in the order it
    /// happened. Nothing here is shown modally.
    pub skipped: Vec<String>,
}

// -------------------------------------------------------------- the tables --

/// Everything discovered this session, by match name.
static DISCOVERED: Mutex<BTreeMap<String, DiscoveredPlugin>> = Mutex::new(BTreeMap::new());

/// Everything refused this session, by identifier.
static REFUSED: Mutex<BTreeMap<String, Refusal>> = Mutex::new(BTreeMap::new());

/// The plugins switched off, by identifier - the running copy of the
/// preference, so switching one off stops it rendering *now* rather than at the
/// next launch.
///
/// A [`DisableList`] rather than a plain `Mutex`, and that is the whole of the
/// first of §5.4's three places: [`running_list`] hands a broker a **share of
/// this table** rather than a copy of it, so a plugin switched off while a scan
/// is running is switched off for the describe that has not happened yet. A
/// snapshot here would freeze the list at the moment the broker was spawned and
/// let a plugin ticked off milliseconds later run its `init` anyway.
static DISABLED: LazyLock<DisableList> = LazyLock::new(nothing_disabled);

/// Every plugin this session has registered, in match-name order.
#[must_use]
pub fn discovered() -> Vec<DiscoveredPlugin> {
    held(&DISCOVERED).values().cloned().collect()
}

/// What a registered plugin declared, by the catalogue name it answers to.
#[must_use]
pub fn plugin_of(match_name: &str) -> Option<DiscoveredPlugin> {
    held(&DISCOVERED).get(match_name).cloned()
}

/// Every plugin this session turned away, in identifier order.
#[must_use]
pub fn refusals() -> Vec<Refusal> {
    held(&REFUSED).values().cloned().collect()
}

/// Why one plugin was turned away, by identifier.
#[must_use]
pub fn refusal_of(identifier: &str) -> Option<Refusal> {
    held(&REFUSED).get(identifier).cloned()
}

/// The one way into this module's session tables that is not a scan, kept in a
/// module of its own so the call site says what it is.
///
/// It is compiled into the shipping library and it cannot sensibly be a
/// feature: Cargo unifies a dev-dependency's features with the ordinary one in
/// the same build, so switching it on for `lumit-bridge`'s suite would switch
/// it on everywhere. What holds it to its one caller instead is this module's
/// name, `#[doc(hidden)]` so it is not offered beside [`scan`] and
/// [`set_enabled`], and `crates/lumit-lfx/tests/the_test_seam_has_one_caller.rs`,
/// which fails the build if a second caller appears anywhere under a crate's
/// `src`.
#[doc(hidden)]
pub mod for_test {
    use super::{held, AddonRow, LfxRejection, Refusal, REFUSED};

    /// Put one refusal in the session table, for the badge test in
    /// `lumit-bridge` and no other caller.
    ///
    /// The `REFUSED` table is filled by [`scan`](super::scan) and by nothing
    /// else, and a scan needs a bundle, a broker executable and a second
    /// process - none of which `lumit-bridge`'s suite has. What that suite has
    /// to prove is [`refusal_of`](super::refusal_of)'s *reader*: that
    /// `badge_of` asks this table **before** it falls through to the namespace
    /// arm, so a plugin the scan turned away badges `plugin_refused` with its
    /// own sentence underneath rather than "this plugin is not installed on
    /// this machine" (§4.3, §11 item 10). Without a way in, that branch order
    /// is asserted by prose and by nothing else, and an edit that moved the
    /// fall-through above it would leave every test green.
    ///
    /// It is a seam, not a feature: nothing in the shipping path calls it, and
    /// a refusal filed here is cleared by the next scan that registers the
    /// plugin, exactly as one filed by [`scan`](super::scan) is.
    pub fn file_refusal(row: AddonRow, why: LfxRejection) {
        let refusal = Refusal { row, why };
        held(&REFUSED).insert(refusal.row.identifier.clone(), refusal);
    }
}

/// Switch a discovered plugin on or off for the rest of this session.
///
/// Persisting the answer is the caller's - the preference file belongs to
/// `lumit-project` and engine crates do not read it. What this does is make the
/// answer true immediately: the next frame of a switched-off plugin is identity
/// with a badge, rather than the change waiting for a restart (§5.4).
pub fn set_enabled(identifier: &str, enabled: bool) {
    let mut disabled = switched_off();
    if enabled {
        disabled.remove(identifier);
    } else {
        disabled.insert(identifier.to_owned());
    }
}

/// Whether `identifier` is switched off right now.
#[must_use]
pub fn is_disabled(identifier: &str) -> bool {
    switched_off().contains(identifier)
}

/// Seed the running switched-off list from the stored preference, before a scan
/// reads it.
pub fn set_disabled(list: &BTreeSet<String>) {
    *switched_off() = list.clone();
}

/// The running switched-off list, copied.
#[must_use]
pub fn disabled_now() -> BTreeSet<String> {
    switched_off().clone()
}

/// The running switched-off list itself, whatever a poisoned lock says - the
/// same reasoning [`held`] gives for the other two tables.
fn switched_off() -> MutexGuard<'static, BTreeSet<String>> {
    DISABLED.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One of the three tables, whatever a poisoned lock says.
///
/// A poisoned table is a table some other thread panicked while holding; the
/// contents are still a `BTreeMap` and losing every plugin on the machine is a
/// worse answer than reading one a panicking thread half-wrote.
fn held<T>(table: &Mutex<T>) -> MutexGuard<'_, T> {
    table.lock().unwrap_or_else(PoisonError::into_inner)
}

// --------------------------------------------------------------- the gate --

/// What a host answers a render with. Never a `Result`: a plugin that failed
/// still owes the caller a picture, and [`Rendering::error`] is the sentence the
/// badge is taken from.
#[derive(Clone, Debug)]
pub struct Rendering {
    /// The picture to carry on with - the plugin's work, or the input byte for
    /// byte where there was none.
    pub pixels: Picture,
    /// What the instance said it reads, as offsets relative to the frame that
    /// was rendered.
    pub frames_needed: Vec<i32>,
    /// Why this frame is not the plugin's work, if it is not. Filed under the
    /// instance and read as a **key**, never shown verbatim.
    pub error: Option<String>,
}

/// Whatever is driving one plugin instance: a broker in the shipping path, and
/// whatever a test hands the gate.
///
/// Two entry points, because two are what the running switched-off list has to
/// be read inside (§5.4). Creating and destroying an instance is the catalogue
/// entry's business and does not go through here.
///
/// **Called with the frame's own lease held**, and never from a rebuild path
/// (docs/impl/lfx.md §4.2, §4.4). The lease rather than a lock is what makes
/// telling an instance its values and asking it for a frame one indivisible
/// pair: a leased instance is not visible to any other frame, so there is
/// nobody for this call to interleave with. No lock of the pool's rows is held
/// across it either - the rows are read once per live effect per frame by a
/// frame-key walk that may not wait behind a render.
///
/// The one lock that *is* held across a call through this trait is the
/// bundle's [`Serial`](crate::pool::Serial), and only for a bundle one of whose
/// plugins declared `lfx.thread-unsafe` - which is the whole of what that
/// declaration buys, and is why an implementor blocking here stops every plugin
/// of such a bundle and nobody else (§2.6). For every other bundle, every other
/// instance and every other row is free to render while this call blocks on
/// somebody else's code in somebody else's process.
pub trait LfxHost: Send + Sync {
    /// Render one frame.
    fn process(&self, instance: InstanceId, job: &ProcessJob<'_>) -> Rendering;

    /// One of an instance's `ACTION` rows was pressed.
    ///
    /// # Errors
    ///
    /// [`BrokerError`] - the plugin is switched off, or the broker said no.
    fn press(&self, instance: InstanceId, param: &str) -> Result<(), BrokerError>;
}

/// A host that answers for a plugin the user has switched off.
///
/// **The second of the three places a disable reaches** (§5.4). The gate is read
/// per call rather than baked in at scan time, because a plugin may be switched
/// off while a comp is open: the frame that follows must be identity and
/// badged, not the plugin's work, and it must be so *now* rather than at the
/// next launch. That is failure 5 - inside an open project the instance keeps
/// resolving, renders identity, and the panel badges `plugin_disabled`, the
/// placeholder that says why.
///
/// The sentence it files is `lumit-ipc`'s shared [`DISABLED_REASON`] and never a
/// twin of it: the badge decides "switched off" rather than "failed" by string
/// equality against that one constant, over a table every hosted effect files
/// into (§4.3).
pub struct Gated {
    /// Which plugin this is, so the running list can be asked about it.
    identifier: String,
    /// Whoever actually renders, when the plugin is on.
    inner: Arc<dyn LfxHost>,
}

impl Gated {
    /// Wrap a host so that every call through it reads the running list first.
    #[must_use]
    pub fn new(identifier: impl Into<String>, inner: Arc<dyn LfxHost>) -> Self {
        Self {
            identifier: identifier.into(),
            inner,
        }
    }

    /// Which plugin this gate is about.
    #[must_use]
    pub fn identifier(&self) -> &str {
        &self.identifier
    }
}

impl LfxHost for Gated {
    fn process(&self, instance: InstanceId, job: &ProcessJob<'_>) -> Rendering {
        if is_disabled(&self.identifier) {
            return Rendering {
                // Byte for byte, and cloned rather than re-read: a switched-off
                // plugin owes the layer the picture it was given, unchanged and
                // unconverted.
                //
                // *ponytail:* the OFX gate moves its input (`frame: source`) and
                // pays nothing; this one copies a whole frame - ~64 MiB at 4K
                // fp16 - because [`ProcessJob`] *borrows* the input and a
                // borrowed picture cannot be moved out of. It is the price of
                // one shape for both hosts rather than an oversight, and the
                // way out is [`Rendering::pixels`] as a `Cow`, which is a change
                // to the trait every implementation answers through and belongs
                // with the catalogue entry that consumes it (§4.2).
                pixels: job.input.clone(),
                frames_needed: Vec::new(),
                error: Some(DISABLED_REASON.to_owned()),
            };
        }
        self.inner.process(instance, job)
    }

    fn press(&self, instance: InstanceId, param: &str) -> Result<(), BrokerError> {
        if is_disabled(&self.identifier) {
            return Err(BrokerError::SwitchedOff);
        }
        self.inner.press(instance, param)
    }
}

/// The picture a broker's answer carries, or the input with a sentence.
///
/// Written here rather than in the catalogue entry because it is the shape
/// [`LfxHost`] promises and every implementation has to reach it the same way:
/// identity on failure, and the error's own words for the badge.
#[must_use]
pub fn rendering_of(job: &ProcessJob<'_>, answer: Result<Rendered, BrokerError>) -> Rendering {
    match answer {
        Ok(rendered) => Rendering {
            pixels: rendered.pixels,
            frames_needed: rendered.frames_needed,
            error: None,
        },
        Err(error) => Rendering {
            pixels: job.input.clone(),
            frames_needed: Vec::new(),
            error: Some(error.to_string()),
        },
    }
}

// --------------------------------------------------------------- the scan --

/// Scan the named directories and offer everything new that is found.
///
/// The ledger is the governor's: every ring a scan's brokers map is reserved
/// from it and given back when the broker is dropped, which is at the end of
/// each bundle.
///
/// **The switched-off list a scan reads is the running one**, and a scan never
/// writes to it. The caller seeds it from `PluginPrefs` with [`set_disabled`]
/// before scanning - which replaces it whole, so a plugin switched back on in
/// the preferences is on again - and ticks land on it through [`set_enabled`]
/// for the rest of the session. The three places a disable reaches have to
/// agree, and one table is how they do.
///
/// Blocking, and not to be called from the interface's thread.
#[must_use]
pub fn scan(options: &ScanOptions, ledger: &Arc<Ledger>) -> ScanOutcome {
    let mut outcome = ScanOutcome::default();
    for dir in &options.paths {
        for found in bundle::scan_dir(dir) {
            scan_bundle(&found, options, ledger, &mut outcome);
        }
    }
    outcome
}

/// One bundle: a broker, its listing, its description, and then nothing.
fn scan_bundle(
    found: &Path,
    options: &ScanOptions,
    ledger: &Arc<Ledger>,
    outcome: &mut ScanOutcome,
) {
    // Which build belongs to this machine, asked now and **answered after the
    // listing**. A bundle whose only build is for another CPU still declared
    // plugins, and §5.3 says [`ScanOutcome::listed`] is every plugin every
    // listing declared, whatever became of it: §7.2's `missing` row is drawn
    // from exactly this, and returning here would leave the one case that most
    // needs a row with nothing but a skip sentence.
    let payload = bundle::payload(found);

    // The shipped quirks are keyed by plugin id and a scan does not know one
    // until the listing has been read, which is after the broker is up - so a
    // scan runs on the defaults, as the audio host's does. The per-plugin entry
    // is read where it matters, when a layer opens a live instance.
    //
    // The module path is empty where there is no build for this machine, which
    // is a path nothing opens and nothing tries to: the broker opens the module
    // lazily, on the first `Describe`, and a bundle with no payload never
    // reaches one.
    let mut config = BrokerConfig::new(
        found,
        payload.clone().unwrap_or_default(),
        RingPlan::frame(SCAN_FRAME.0, SCAN_FRAME.1, PixelDepth::F16),
    );
    config.exe.clone_from(&options.exe);
    config.env.clone_from(&options.env);
    config.disabled = running_list();

    let mut broker = match Broker::spawn(config, ledger) {
        Ok(broker) => broker,
        Err(error) => {
            outcome.skipped.push(skip_line(found, &error.to_string()));
            return;
        }
    };

    // The listing first, always. It is the cheap answer, it runs none of the
    // bundle's code, and it is the only thing a plugin switched off before this
    // scan will ever have a row from.
    let listing: Vec<PluginIdentity> = match broker.manifest() {
        Ok(entries) => entries.to_vec(),
        Err(error) => {
            outcome.skipped.push(skip_line(found, &error.to_string()));
            return;
        }
    };
    // Every plugin the listing declared gets its row, **whatever becomes of it**
    // (§5.3). This is the raw material of the roster and of §7.2's Installed
    // section, and it is filled in before anything else can turn a plugin away.
    for identity in &listing {
        outcome.listed.push(AddonRow::of(identity, found));
    }

    // Only now: a bundle carrying no build for this machine has given the page
    // its rows and has nothing else to give. One calm sentence, and the module
    // is never asked for.
    if payload.is_none() {
        outcome.skipped.push(skip_line(
            found,
            "it holds no payload this machine's architecture can run",
        ));
        return;
    }

    // **Negotiation runs from the listing, before `create` and before any of
    // that plugin's own code** (§4.3). A plugin whose required list this host
    // cannot satisfy never enters the catalogue, and is recorded with the
    // extension named rather than dropped into the same silence a switched-off
    // one makes. It counts against `wanted` below, so a bundle with nothing but
    // unsatisfiable plugins in it is never opened either; a bundle that also
    // holds one this host can satisfy is opened for that one, and the refused
    // plugin's own `describe` runs in it, since the broker's `Describe` filters
    // on the switched-off list and not on this.
    let mut unsatisfiable: BTreeSet<String> = BTreeSet::new();
    let mut wanted = 0usize;
    for identity in &listing {
        // The mention §5.3 asks for, and it is made **here** rather than where
        // the older host makes it: the disable travels with `Describe`, so a
        // switched-off plugin does not come back from the second process at all
        // and `offer` is not normally reached for it. The listing is what names
        // it, which is the whole reason the listing is read first; `offer`'s own
        // check is the backstop for a tick landing after the describe.
        if is_disabled(&identity.id) {
            outcome.skipped.push(skip_line(
                found,
                &format!("{}: switched off in preferences", identity.id),
            ));
            continue;
        }
        if let Err(why) = crate::extensions::negotiate(&identity.id, &identity.required_extensions)
        {
            unsatisfiable.insert(identity.id.clone());
            file_refusal(AddonRow::of(identity, found), why, outcome);
            continue;
        }
        wanted += 1;
    }

    // **Nothing in this bundle is wanted, so the module is never opened**
    // (§5.4 place 1). The disable travels with `Describe` and the broker
    // filters on it - but it opens the module first, and a describe is what
    // opens it. For a one-plugin bundle, which is the commonest shape and the
    // one a person switches off precisely because it misbehaves, asking for a
    // describe nobody wants an answer to would run the library's own
    // initialisers and its `init` at every start-up scan after the tick. The
    // listing rows are already in `outcome.listed` and the skip lines are
    // already filed, so the page is owed nothing more by a bundle with no
    // plugin left to describe.
    if wanted == 0 {
        return;
    }

    let described = match broker.describe() {
        Ok(described) => described.to_vec(),
        Err(error) => {
            outcome.skipped.push(skip_line(found, &error.to_string()));
            return;
        }
    };
    // What the second process turned away, each against its own plugin - the
    // raw material of the `REFUSED` table. A describe that failed reaching the
    // host as an absence is what makes a broken plugin look exactly like one
    // the user switched off.
    for (id, why) in broker.refused() {
        if unsatisfiable.contains(id) {
            // Already refused from the listing, with the same sentence. Filing
            // it twice would put two rows on the page for one plugin - and the
            // row is not built either, since the one thing to do with it is
            // drop it.
            continue;
        }
        let row = listing
            .iter()
            .find(|identity| identity.id == *id)
            .map_or_else(
                || AddonRow {
                    identifier: id.clone(),
                    label: String::new(),
                    vendor: String::new(),
                    release: (0, 0, 0),
                    bundle: found.to_path_buf(),
                },
                |identity| AddonRow::of(identity, found),
            );
        file_refusal(row, why.clone(), outcome);
    }
    // Lines about the bundle rather than about any one plugin in it.
    for line in broker.report() {
        outcome.skipped.push(skip_line(found, &line.to_string()));
    }

    for plugin in &described {
        offer(found, plugin, &unsatisfiable, outcome);
    }
}

/// Turn one described plugin into a catalogue row, unless something says not to.
///
/// The four refusals, in the order they are cheapest to answer:
///
/// 1. the listing already refused it for an extension this host has not got;
/// 2. the user switched it off;
/// 3. this session already discovered it (a rescan, or a second copy);
/// 4. Lumit cannot write its declaration down.
///
/// The first is a guard rather than a formality. It is unreachable in version 1 -
/// `OFFERED_EXTENSIONS` is empty and the describe path negotiates for itself,
/// so no plugin with a required list survives the broker - but the set the host
/// offers and the set the describe path enforces stop being trivially equal the
/// day `lfx.temporal` has a table behind it, and cataloguing a plugin whose
/// refusal was filed moments earlier would both offer an effect that cannot be
/// instantiated and **erase the refusal** at the `REFUSED` line below.
///
/// The schema is built **last** so that a plugin turned away for any of the
/// first three costs no leak, which is what makes a rescan free rather than
/// merely idempotent.
fn offer(
    found: &Path,
    plugin: &crate::ipc::proto::DescribedPlugin,
    unsatisfiable: &BTreeSet<String>,
    outcome: &mut ScanOutcome,
) {
    let identifier = plugin.identity.id.clone();
    if unsatisfiable.contains(&identifier) {
        return;
    }
    if is_disabled(&identifier) {
        outcome.skipped.push(skip_line(
            found,
            &format!("{identifier}: switched off in preferences"),
        ));
        return;
    }
    let match_name = format!("{LFX_MATCH_PREFIX}{identifier}");

    // **One guard, held across the look and the insert.** Nothing in the public
    // API stops two workers scanning overlapping folders, and a check that let
    // the lock go before `Box::leak` would let both of them leak a schema and
    // the second replace the first - under a catalogue that may already be
    // holding a pointer to the one replaced.
    let mut table = held(&DISCOVERED);
    if let Some(already) = table.get(&match_name) {
        let first = already.row.bundle.clone();
        drop(table);
        // A rescan of the same bundle is the silence this table exists for. Two
        // *different* bundles declaring one id is not: it is the ordinary
        // outcome of a vendor install followed by a Lumit install (§6.2), and
        // which of them wins is search-path order, which nobody can see. So the
        // page is told both, and which one is live.
        if first != found {
            outcome.skipped.push(skip_line(
                found,
                &format!(
                    "{identifier}: already offered from {}, which is the copy in use",
                    first.display()
                ),
            ));
        }
        return;
    }

    // The one place the five fields are spelled (§5.3). A second spelling here
    // would be a second place for the Addons page to disagree with the listing
    // about what a plugin is called.
    let row = AddonRow::of(&plugin.identity, found);
    let descriptor = crate::describe::PluginDescriptor::from(plugin.clone());
    let schema = match schema::schema_of(&descriptor) {
        Ok(schema) => schema,
        Err(why) => {
            drop(table);
            file_refusal(row, why, outcome);
            return;
        }
    };
    // Everything the lowering could not take at face value: a control this
    // build cannot draw, a family outside the eight, a range the panel cannot
    // draw. Every one of them is a line and the plugin still loads (§3.6).
    for note in schema::notes(&descriptor) {
        outcome
            .skipped
            .push(skip_line(found, &format!("{identifier}: {note}")));
    }

    let discovered = DiscoveredPlugin {
        match_name: match_name.clone(),
        categories: schema::families(&descriptor),
        row,
        schema: Box::leak(Box::new(schema)),
    };
    table.insert(match_name, discovered.clone());
    drop(table);
    // A plugin that registered is not still refused. A row wearing last
    // session's sentence beside a working effect is worse than no sentence,
    // and the roster clears its own for the same reason.
    held(&REFUSED).remove(&identifier);
    outcome.registered.push(discovered);
}

/// File one refusal in the session table and in the report at once, so the two
/// cannot come to different opinions about what was turned away.
fn file_refusal(row: AddonRow, why: LfxRejection, outcome: &mut ScanOutcome) {
    outcome.skipped.push(skip_line(
        &row.bundle,
        &format!("{}: {why}", row.identifier),
    ));
    let refusal = Refusal { row, why };
    held(&REFUSED).insert(refusal.row.identifier.clone(), refusal.clone());
    outcome.refused.push(refusal);
}

/// The one lock every case that ticks a box in the running switched-off list
/// takes, wherever in this crate it lives.
///
/// That list is one table for the whole process - which is the point of it -
/// so two cases editing it at once would read each other's ticks. It is here
/// rather than in each suite because [`crate::def`]'s cases tick the same boxes
/// this module's do, and two locks over one table is not a lock.
#[cfg(test)]
pub(crate) fn gate_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The switched-off list as the broker takes it: **a share of the running
/// table**, not a copy of it, so a plugin switched off while a scan is running
/// is switched off for the describe that has not happened yet.
///
/// `Broker::disabled_now` reads this list at the moment the `Describe` is
/// written, which is what makes the read late; handing over a snapshot taken
/// when the broker was spawned would defeat both ends of that and let a tick
/// landing between the spawn and the describe miss by milliseconds.
///
/// Public because this is what anything spawning a broker of its own must put
/// in [`BrokerConfig::disabled`] - a copy taken there is the bug this function
/// exists to stop being written twice.
#[must_use]
pub fn running_list() -> DisableList {
    Arc::clone(&DISABLED)
}

/// One line of the scan report: which bundle, and what happened.
fn skip_line(found: &Path, why: &str) -> String {
    format!("{}: {why}", found.display())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::ipc::proto::RectI;

    use super::*;

    /// A host that counts what reached it and paints every sample white.
    ///
    /// The count is the assertion: §5.4's promise is that a switched-off
    /// plugin's code does not run, and a gate that returned the input while
    /// still calling through would keep the picture and break the promise.
    #[derive(Default)]
    struct Counting {
        renders: AtomicUsize,
        presses: AtomicUsize,
    }

    impl LfxHost for Counting {
        fn process(&self, _instance: InstanceId, job: &ProcessJob<'_>) -> Rendering {
            self.renders.fetch_add(1, Ordering::SeqCst);
            Rendering {
                pixels: Picture::F32(vec![1.0; job.input.len()]),
                frames_needed: vec![-1, 0, 1],
                error: None,
            }
        }

        fn press(&self, _instance: InstanceId, _param: &str) -> Result<(), BrokerError> {
            self.presses.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    /// A four-pixel frame and the job that renders the whole of it.
    fn a_job(input: &Picture) -> ProcessJob<'_> {
        ProcessJob {
            time: 0.0,
            bounds: RectI::of(2, 2),
            roi: RectI::of(2, 2),
            input,
            neighbours: &[],
        }
    }

    /// Failure 5: a plugin switched off while a comp is open renders identity
    /// **byte for byte** and files the one shared reason the badge seam reads,
    /// and the plugin's own code is not reached at all.
    #[test]
    fn a_switched_off_plugin_renders_identity_and_files_the_shared_reason() {
        let _guard = gate_lock();
        let inner = Arc::new(Counting::default());
        let gate = Gated::new("com.example.blur", Arc::clone(&inner) as Arc<dyn LfxHost>);

        let input = Picture::F32(vec![0.25, 0.5, 0.75, 1.0]);
        let job = a_job(&input);

        set_enabled("com.example.blur", false);
        let answer = gate.process(7, &job);
        assert_eq!(
            answer.pixels, input,
            "the picture it was given, unchanged and unconverted"
        );
        assert_eq!(
            answer.error.as_deref(),
            Some(DISABLED_REASON),
            "the constant `badge_of` tests against, never a twin of it"
        );
        assert!(
            answer.frames_needed.is_empty(),
            "a plugin that is not running asks for no neighbours"
        );
        assert_eq!(
            inner.renders.load(Ordering::SeqCst),
            0,
            "a switched-off plugin's code does not run"
        );

        assert!(matches!(
            gate.press(7, "reset"),
            Err(BrokerError::SwitchedOff)
        ));
        assert_eq!(
            BrokerError::SwitchedOff.to_string(),
            DISABLED_REASON,
            "the typed refusal says the shared word, not a sentence of this host's"
        );
        assert_eq!(inner.presses.load(Ordering::SeqCst), 0);

        set_enabled("com.example.blur", true);
    }

    /// The list is read **per call**, not baked in when the gate was built - so
    /// switching a plugin off stops it rendering now rather than at the next
    /// launch, and switching it back on needs no rescan (§5.4).
    #[test]
    fn a_plugin_switched_off_mid_session_stops_rendering_now() {
        let _guard = gate_lock();
        let inner = Arc::new(Counting::default());
        let gate = Gated::new("com.example.warp", Arc::clone(&inner) as Arc<dyn LfxHost>);
        let input = Picture::F32(vec![0.25, 0.5, 0.75, 1.0]);
        let job = a_job(&input);

        set_enabled("com.example.warp", true);
        assert!(gate.process(1, &job).error.is_none(), "on to begin with");
        assert_eq!(gate.identifier(), "com.example.warp");

        set_enabled("com.example.warp", false);
        assert!(is_disabled("com.example.warp"));
        assert_eq!(
            gate.process(1, &job).error.as_deref(),
            Some(DISABLED_REASON)
        );

        // And back, with the same gate: re-enabling is instant and needs no
        // rescan, because filtering is not unregistering (§11 item 8).
        set_enabled("com.example.warp", true);
        assert!(gate.process(1, &job).error.is_none());
        assert_eq!(
            inner.renders.load(Ordering::SeqCst),
            2,
            "twice, not three times"
        );
    }

    /// Seeding the running list from the stored preference replaces it whole,
    /// which is what a fresh read of `plugins.json` means.
    #[test]
    fn the_running_list_is_seeded_whole_from_the_preference() {
        let _guard = gate_lock();
        let mut stored = BTreeSet::new();
        stored.insert("com.example.one".to_owned());
        stored.insert("com.example.two".to_owned());
        set_disabled(&stored);
        assert_eq!(disabled_now(), stored);

        set_enabled("com.example.one", true);
        assert!(!is_disabled("com.example.one"));
        assert!(is_disabled("com.example.two"));

        set_disabled(&BTreeSet::new());
        assert!(disabled_now().is_empty());
    }

    /// The handle half of §5.4 place 1: the list a broker is spawned with is a
    /// **share** of the running table and not a snapshot of it, so a tick
    /// landing after the spawn is in the list the `Describe` reads.
    ///
    /// **No broker is started and no describe is run**, which is why this is
    /// not the case §5.4 place 1 is pinned by - that one drives a real second
    /// process and is
    /// `a_plugin_switched_off_after_the_broker_starts_is_still_switched_off_at_describe`
    /// in `lumit-lfx-broker`'s own suite. What is here is the wiring under it:
    /// that what `scan_bundle` puts in [`BrokerConfig::disabled`] is the same
    /// table [`set_enabled`] edits, which a copy would quietly stop being.
    #[test]
    fn the_disable_list_a_broker_is_spawned_with_is_a_share_of_the_running_table() {
        let _guard = gate_lock();
        set_disabled(&BTreeSet::new());

        let mut config = BrokerConfig::new(
            Path::new("Late.lfx.bundle"),
            PathBuf::new(),
            RingPlan::frame(SCAN_FRAME.0, SCAN_FRAME.1, PixelDepth::F16),
        );
        // Exactly what `scan_bundle` hands `Broker::spawn`, taken before the
        // tick - which is the window the shipping scan leaves open between the
        // spawn, the handshake and the `Manifest` round trip.
        config.disabled = running_list();
        let seen = || {
            config
                .disabled
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains("com.example.late")
        };
        assert!(!seen(), "nothing is switched off when the broker starts");

        set_enabled("com.example.late", false);
        assert!(
            seen(),
            "a tick landing after the spawn reaches the describe that has not \
             happened yet"
        );

        set_enabled("com.example.late", true);
        assert!(!seen(), "and switching it back on is just as live");
        set_disabled(&BTreeSet::new());
    }

    /// A scan **reads** the running switched-off list and never writes to it,
    /// so a preference that narrows takes effect at the next scan rather than
    /// leaving last scan's identifiers switched off for the rest of the session
    /// (§5.4).
    ///
    /// The folder is empty on purpose: what is under test is what a scan does
    /// to the table on its way past, which no bundle is needed to show.
    #[test]
    fn a_scan_reads_the_running_list_and_never_writes_to_it() {
        let _guard = gate_lock();
        let root = tempfile::tempdir().expect("a temporary directory");
        let ledger = Ledger::new();
        let options = ScanOptions {
            paths: vec![root.path().to_path_buf()],
            ..ScanOptions::default()
        };

        let mut stored = BTreeSet::new();
        stored.insert("com.example.off".to_owned());
        set_disabled(&stored);
        let _ = scan(&options, &ledger);
        assert_eq!(
            disabled_now(),
            stored,
            "the scan read the list; it did not add to it"
        );

        // The person ticks the box back on and the preference is read again.
        // A scan that had merged the last one into the running table would
        // leave `com.example.off` switched off here - `is_disabled` true,
        // `Gated` rendering identity, `LfxDef::lease` refusing - with no
        // preference anywhere saying so.
        set_disabled(&BTreeSet::new());
        let _ = scan(&options, &ledger);
        assert!(
            !is_disabled("com.example.off"),
            "a narrower preference is the whole of the running list"
        );
        assert!(disabled_now().is_empty());
    }

    /// One described plugin, with an identity and nothing else. The lowering
    /// takes a panel of no rows, which is what lets these cases drive `offer`
    /// itself rather than a second process.
    fn a_described(id: &str) -> crate::ipc::proto::DescribedPlugin {
        crate::ipc::proto::DescribedPlugin {
            identity: PluginIdentity {
                id: id.to_owned(),
                name: "Example".to_owned(),
                vendor: "Example".to_owned(),
                major: 1,
                ..PluginIdentity::default()
            },
            ..crate::ipc::proto::DescribedPlugin::default()
        }
    }

    /// The same plugin installed twice - a vendor's copy in the system folder
    /// and Lumit's own in the addons folder, which is what §6.2 creates. The
    /// first bundle the search paths reach wins, and the page is **told**:
    /// silence would leave a person with two installs, one live, and no way to
    /// see which (§5.3).
    #[test]
    fn two_bundles_declaring_one_id_name_the_one_that_won() {
        let _guard = gate_lock();
        set_disabled(&BTreeSet::new());
        let identifier = "com.example.twice";
        let system = Path::new("/usr/lib/lfx/Twice.lfx.bundle");
        let addons = Path::new("/home/example/addons/Twice.lfx.bundle");
        let plugin = a_described(identifier);
        let none = BTreeSet::new();

        let mut first = ScanOutcome::default();
        offer(system, &plugin, &none, &mut first);
        assert_eq!(
            first.registered.len(),
            1,
            "the first copy the walk reached registered"
        );

        let mut second = ScanOutcome::default();
        offer(addons, &plugin, &none, &mut second);
        assert!(
            second.registered.is_empty(),
            "one id is one effect, however many copies are installed"
        );
        let line = second.skipped.join("\n");
        assert!(
            line.contains(&addons.display().to_string())
                && line.contains(&system.display().to_string()),
            "the report names both bundles and the one in use: {line}"
        );

        // And a rescan of the copy that won is the silence the table exists
        // for - a line every launch would be a report nobody reads.
        let mut again = ScanOutcome::default();
        offer(system, &plugin, &none, &mut again);
        assert!(again.registered.is_empty() && again.skipped.is_empty());
    }

    /// A plugin the **listing** already refused for an extension this host has
    /// not got is not catalogued if it comes back described anyway - which
    /// would offer an effect that cannot be instantiated and erase the refusal
    /// filed moments earlier.
    #[test]
    fn a_plugin_refused_from_the_listing_is_never_offered_described() {
        let _guard = gate_lock();
        set_disabled(&BTreeSet::new());
        let identifier = "com.example.needy";
        let bundle = Path::new("/usr/lib/lfx/Needy.lfx.bundle");
        let plugin = a_described(identifier);

        let mut outcome = ScanOutcome::default();
        file_refusal(
            AddonRow::of(&plugin.identity, bundle),
            LfxRejection::RequiresExtension {
                id: identifier.to_owned(),
                extension: "lfx.temporal".to_owned(),
            },
            &mut outcome,
        );
        let mut unsatisfiable = BTreeSet::new();
        unsatisfiable.insert(identifier.to_owned());

        offer(bundle, &plugin, &unsatisfiable, &mut outcome);
        assert!(
            outcome.registered.is_empty(),
            "an effect the host cannot satisfy does not enter the catalogue"
        );
        assert!(
            plugin_of(&format!("{LFX_MATCH_PREFIX}{identifier}")).is_none(),
            "and no schema was leaked for it"
        );
        assert!(
            refusal_of(identifier).is_some(),
            "the refusal it was filed under is still there"
        );
    }

    /// A frame that did not come back is the input with the error's own words,
    /// and a frame that did is the plugin's work with none - the shape every
    /// [`LfxHost`] reaches the same way.
    #[test]
    fn a_frame_that_did_not_come_back_is_the_input_with_its_own_sentence() {
        let input = Picture::F32(vec![0.25, 0.5, 0.75, 1.0]);
        let job = a_job(&input);

        let failed = rendering_of(&job, Err(BrokerError::Timeout));
        assert_eq!(failed.pixels, input, "identity, byte for byte");
        assert_eq!(
            failed.error.as_deref(),
            Some(BrokerError::Timeout.to_string().as_str())
        );

        let worked = rendering_of(
            &job,
            Ok(Rendered {
                pixels: Picture::F32(vec![1.0; 4]),
                frames_needed: vec![0],
            }),
        );
        assert_eq!(worked.error, None);
        assert_eq!(worked.frames_needed, vec![0]);
    }
}
