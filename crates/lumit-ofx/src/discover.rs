//! Finding the plugins installed on this machine, and offering them.
//!
//! # In plain terms
//!
//! Everything before this module could host a plugin somebody handed it by
//! path. This is the part that goes looking: it reads the standard OFX folders
//! (and anything `OFX_PLUGIN_PATH` adds), opens each bundle it finds, asks
//! every plugin in it to describe itself, and hands the results to whoever is
//! keeping the effect catalogue. After that a plugin is an effect like any
//! other — it appears in Effects & presets, it applies to a layer, it draws its
//! own controls.
//!
//! Four rules the scan follows, all of them from docs/12 §2.6:
//!
//! * **A bundle that will not load is a line in a report, never a dialogue.**
//!   Somebody else's installer left a broken file on the machine; that is not
//!   worth interrupting the person for, and it must not cost them the other
//!   bundles in the folder.
//! * **The user's switched-off list is consulted before registration**, not
//!   after. A plugin the user has turned off is never described, never
//!   instantiated, and never in the menu.
//! * **A rescan adds; it never replaces.** Registration is additive, so
//!   the second scan of a session skips everything the first one registered —
//!   by name, before any work is done, which is also what stops a rescan
//!   leaking a second copy of a schema it already has.
//! * **Never on the interface's thread.** Opening bundles means running other
//!   people's start-up code and spawning processes. The scan is a plain
//!   blocking function; the caller puts it on a worker (the bridge does).
//!
//! # The two arrangements
//!
//! [`Hosting::Broker`] is the shipping one: one broker process per bundle
//! (docs/12 §2.3), so a plugin that dies costs a frame rather than the session.
//! [`Hosting::InProcess`] loads the bundle here instead, which is what the
//! tests use — proving that a folder of bundles becomes the right set of
//! effects needs no second process, and the broker's own behaviour has its own
//! tests in its own crate.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use lumit_core::fx::{EffectDef, EffectSchema};
use uuid::Uuid;

use crate::bundle::{self, Bundle};
use crate::def::{BrokerHost, LocalHost, OfxEffectDef, PluginHost, Rendering, SharedBroker};
use crate::describe::{describe_bundle, Context, PluginDescriptor};
use crate::image::Frame16;
use crate::instance::ParamSnapshot;
use crate::ipc::broker::{Broker, BrokerConfig};
use crate::schema::schema_of;

/// The frame size the shared-memory ring is built for when a bundle's broker is
/// spawned at scan time.
///
/// The ring is sized **once per broker** (docs/impl/ofx-host.md §4) and the scan
/// happens before any composition is open, so there is no comp size to size it
/// from. 1080p is the honest guess: it is the commonest delivery and it leaves
/// the ring fifteen slots deep.
// ponytail: a 4K comp then renders through a three-slot ring, which the note
// already names as the floor. The upgrade is a broker respawned at the comp's
// size when the first frame is asked for, not a different design.
pub const SCAN_FRAME: (usize, usize) = (1920, 1080);

/// Which side of a process boundary a discovered plugin runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hosting {
    /// One broker process per bundle — the shipping arrangement.
    Broker,
    /// Loaded into this process. For tests, and for a bundle Lumit ships.
    InProcess,
}

/// What a scan was asked to do.
pub struct ScanOptions {
    /// The directories to look in. [`ScanOptions::standard`] fills this from
    /// the platform's own folders plus `OFX_PLUGIN_PATH`.
    pub paths: Vec<PathBuf>,
    /// Plugin identifiers the user has switched off (docs/12 §2.6). Consulted
    /// before a plugin is described, so a disabled plugin's code never runs.
    pub disabled: BTreeSet<String>,
    /// Which arrangement to host what is found in.
    pub hosting: Hosting,
}

impl ScanOptions {
    /// The standard search paths, nothing disabled, hosted in brokers — what
    /// start-up asks for before the preferences are read into it.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            paths: bundle::search_paths(),
            disabled: BTreeSet::new(),
            hosting: Hosting::Broker,
        }
    }
}

/// One plugin that became an effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredPlugin {
    /// The name the catalogue answers to — `ofx:` and the plugin's identifier.
    pub match_name: String,
    /// The plugin's own reverse-domain identifier, which is what the
    /// switched-off list names and what [`set_enabled`] takes.
    pub identifier: String,
    /// The name a person sees.
    pub label: String,
    /// The plugin's own menu path, e.g. `Filter/Blur`. Effects & presets shows
    /// it under this rather than under one of Lumit's own categories, because
    /// none of those is a claim about somebody else's effect (docs/12 §2.6).
    pub grouping: String,
    /// Which bundle binary it came out of.
    pub bundle: PathBuf,
}

/// What one scan did.
#[derive(Debug, Default)]
pub struct ScanOutcome {
    /// The plugins registered **by this scan**. A rescan that finds nothing new
    /// answers with an empty list, which is the normal case.
    pub registered: Vec<DiscoveredPlugin>,
    /// One calm sentence per bundle or plugin turned away, in the order it
    /// happened. Nothing here is shown modally.
    pub skipped: Vec<String>,
}

/// Everything discovered this session, by match name.
static DISCOVERED: Mutex<BTreeMap<String, DiscoveredPlugin>> = Mutex::new(BTreeMap::new());

/// The plugins switched off, by identifier — the running copy of the
/// preference, so switching one off stops it rendering *now* rather than at the
/// next launch.
static DISABLED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// Every plugin this session has registered, in registration order.
#[must_use]
pub fn discovered() -> Vec<DiscoveredPlugin> {
    DISCOVERED
        .lock()
        .map(|table| table.values().cloned().collect())
        .unwrap_or_default()
}

/// What a registered plugin declared, by the catalogue name it answers to.
#[must_use]
pub fn plugin_of(match_name: &str) -> Option<DiscoveredPlugin> {
    DISCOVERED.lock().ok()?.get(match_name).cloned()
}

/// Switch a discovered plugin on or off for the rest of this session.
///
/// Persisting the answer is the caller's — the preference file belongs to
/// `lumit-project` and engine crates do not read it (docs/05). What this does
/// is make the answer true immediately: a plugin switched off renders identity
/// and the layer wears a badge, rather than the change waiting for a restart.
pub fn set_enabled(identifier: &str, enabled: bool) {
    let Ok(mut disabled) = DISABLED.lock() else {
        return;
    };
    if enabled {
        disabled.remove(identifier);
    } else {
        disabled.insert(identifier.to_owned());
    }
}

/// Whether `identifier` is switched off right now.
#[must_use]
pub fn is_disabled(identifier: &str) -> bool {
    DISABLED
        .lock()
        .map(|set| set.contains(identifier))
        .unwrap_or(false)
}

/// Seed the running switched-off list from the stored preference, before a
/// scan reads it.
pub fn set_disabled(list: &BTreeSet<String>) {
    if let Ok(mut disabled) = DISABLED.lock() {
        *disabled = list.clone();
    }
}

/// A host that answers for a plugin the user has switched off.
///
/// The gate is read per render rather than baked in at scan time, because a
/// plugin may be switched off while a comp is open: the frame that follows must
/// be identity and badged, not the plugin's work.
struct Gated {
    identifier: String,
    inner: Arc<dyn PluginHost>,
}

impl PluginHost for Gated {
    fn render(
        &self,
        inst: Uuid,
        time: f64,
        params: &ParamSnapshot,
        source: Frame16,
        neighbours: &[(i32, Frame16)],
    ) -> Rendering {
        if is_disabled(&self.identifier) {
            return Rendering {
                frame: source,
                error: Some(DISABLED_REASON.to_owned()),
            };
        }
        self.inner.render(inst, time, params, source, neighbours)
    }

    fn frames_needed(&self, inst: Uuid, time: f64, params: &ParamSnapshot) -> Option<Vec<i32>> {
        if is_disabled(&self.identifier) {
            return None;
        }
        self.inner.frames_needed(inst, time, params)
    }

    fn press(
        &self,
        inst: Uuid,
        time: f64,
        params: &ParamSnapshot,
        name: &str,
        source: Frame16,
    ) -> Result<ParamSnapshot, String> {
        if is_disabled(&self.identifier) {
            return Err(DISABLED_REASON.to_owned());
        }
        self.inner.press(inst, time, params, name, source)
    }
}

/// What a switched-off plugin files under its instance. Read as a **key** by
/// the seam that badges the layer, never shown verbatim.
pub const DISABLED_REASON: &str = "plugin_disabled";

/// Scan the named directories and register everything new that is found.
///
/// `register` is the catalogue: it takes a definition and answers `false` when
/// the catalogue already knows that name. It is a callback rather than a direct
/// call because the definition has to land in **two** tables — the catalogue and
/// the render's pass table — and joining them is the composition root's job
/// (`lumit_render::gpufx::ofx::register`), not an engine crate's.
///
/// Blocking, and not to be called from the interface's thread.
pub fn scan(
    options: &ScanOptions,
    register: &mut dyn FnMut(&'static dyn EffectDef) -> bool,
) -> ScanOutcome {
    let mut outcome = ScanOutcome::default();
    for dir in &options.paths {
        for binary in bundle::scan_dir(dir) {
            match options.hosting {
                Hosting::Broker => scan_through_broker(&binary, options, register, &mut outcome),
                Hosting::InProcess => scan_in_process(&binary, options, register, &mut outcome),
            }
        }
    }
    outcome
}

/// One bundle, hosted in a broker process of its own.
fn scan_through_broker(
    binary: &std::path::Path,
    options: &ScanOptions,
    register: &mut dyn FnMut(&'static dyn EffectDef) -> bool,
    outcome: &mut ScanOutcome,
) {
    let mut broker = match Broker::spawn(BrokerConfig::new(binary, SCAN_FRAME)) {
        Ok(broker) => broker,
        Err(error) => {
            outcome.skipped.push(skip_line(binary, &error.to_string()));
            return;
        }
    };
    let descriptors: Vec<PluginDescriptor> = match broker.describe() {
        Ok(described) => described.to_vec(),
        Err(error) => {
            outcome.skipped.push(skip_line(binary, &error.to_string()));
            return;
        }
    };
    let shared = Arc::new(SharedBroker::new(broker));
    for (index, descriptor) in descriptors.iter().enumerate() {
        let plugin = u32::try_from(index).unwrap_or(u32::MAX);
        let host = |context| -> Arc<dyn PluginHost> {
            Arc::new(BrokerHost::new(Arc::clone(&shared), plugin, context))
        };
        offer(binary, descriptor, &host, options, register, outcome);
    }
}

/// One bundle, loaded into this process.
///
/// A fresh [`Bundle`] per plugin: a host owns the bundle it drives, and the
/// loader hands back the same module for the same path, so the second open is
/// the same library and not a second copy of it.
fn scan_in_process(
    binary: &std::path::Path,
    options: &ScanOptions,
    register: &mut dyn FnMut(&'static dyn EffectDef) -> bool,
    outcome: &mut ScanOutcome,
) {
    let mut opened = match Bundle::open(binary) {
        Ok(bundle) => bundle,
        Err(error) => {
            outcome.skipped.push(skip_line(binary, &error.to_string()));
            return;
        }
    };
    opened.load();
    let report = describe_bundle(&opened);
    for refused in &report.rejected {
        outcome.skipped.push(skip_line(
            binary,
            &format!("{}: {}", refused.identifier, refused.reason),
        ));
    }
    if report.effects.is_empty() {
        outcome
            .skipped
            .push(skip_line(binary, "it holds no effect this host can drive"));
        return;
    }
    for described in &report.effects {
        let host = |_context| -> Arc<dyn PluginHost> {
            // A bundle that opened once opens again; if it somehow does not,
            // the host is one that answers every render with a failure, which
            // is the badge rather than a lost scan.
            match Bundle::open(binary) {
                Ok(mut fresh) => {
                    fresh.load();
                    Arc::new(LocalHost::new(fresh, described.descriptor.clone()))
                }
                Err(_) => Arc::new(Absent),
            }
        };
        offer(
            binary,
            &described.descriptor,
            &host,
            options,
            register,
            outcome,
        );
    }
}

/// The host for a bundle that vanished between the scan and the render.
struct Absent;

impl PluginHost for Absent {
    fn render(
        &self,
        _: Uuid,
        _: f64,
        _: &ParamSnapshot,
        source: Frame16,
        _: &[(i32, Frame16)],
    ) -> Rendering {
        Rendering {
            frame: source,
            error: Some("the plugin's bundle could not be opened".to_owned()),
        }
    }

    fn frames_needed(&self, _: Uuid, _: f64, _: &ParamSnapshot) -> Option<Vec<i32>> {
        None
    }

    fn press(
        &self,
        _: Uuid,
        _: f64,
        _: &ParamSnapshot,
        _: &str,
        _: Frame16,
    ) -> Result<ParamSnapshot, String> {
        Err("the plugin's bundle could not be opened".to_owned())
    }
}

/// Turn one described plugin into a catalogue entry, unless something says not
/// to. The three refusals, in the order they are cheapest to answer:
///
/// 1. the user switched it off;
/// 2. this session already registered it (a rescan);
/// 3. Lumit cannot write its declaration down (two rows on one id, say).
///
/// The host is built **last**, and lazily, so a plugin refused for any of those
/// reasons costs no process and no bundle load.
fn offer(
    binary: &std::path::Path,
    descriptor: &PluginDescriptor,
    host: &dyn Fn(Context) -> Arc<dyn PluginHost>,
    options: &ScanOptions,
    register: &mut dyn FnMut(&'static dyn EffectDef) -> bool,
    outcome: &mut ScanOutcome,
) {
    let identifier = descriptor.identifier.clone();
    if options.disabled.contains(&identifier) || is_disabled(&identifier) {
        outcome.skipped.push(skip_line(
            binary,
            &format!("{identifier}: switched off in preferences"),
        ));
        return;
    }
    let match_name = format!("ofx:{identifier}");
    if known(&match_name) {
        return;
    }
    let schema = match schema_of(descriptor) {
        Ok(schema) => schema,
        Err(reason) => {
            outcome
                .skipped
                .push(skip_line(binary, &format!("{identifier}: {reason}")));
            return;
        }
    };
    let schema: &'static EffectSchema = Box::leak(Box::new(schema));
    let context = descriptor
        .contexts
        .first()
        .copied()
        .unwrap_or(Context::Filter);
    let gated: Arc<dyn PluginHost> = Arc::new(Gated {
        identifier: identifier.clone(),
        inner: host(context),
    });
    let def = OfxEffectDef::new(descriptor, schema, gated).leak();
    if !register(def) {
        outcome.skipped.push(skip_line(
            binary,
            &format!("{identifier}: an effect of that name is already in the catalogue"),
        ));
        return;
    }
    let found = DiscoveredPlugin {
        match_name: match_name.clone(),
        identifier,
        label: descriptor.label.clone(),
        grouping: descriptor.grouping.clone(),
        bundle: binary.to_path_buf(),
    };
    if let Ok(mut table) = DISCOVERED.lock() {
        table.insert(match_name, found.clone());
    }
    outcome.registered.push(found);
}

/// Whether this session already registered that name.
fn known(match_name: &str) -> bool {
    DISCOVERED
        .lock()
        .map(|table| table.contains_key(match_name))
        .unwrap_or(false)
}

/// One line of the scan report: which bundle, and what happened.
fn skip_line(binary: &std::path::Path, why: &str) -> String {
    format!("{}: {why}", binary.display())
}
