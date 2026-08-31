//! One effect instance as the Effect controls panel reads and edits it, and the
//! parameter value type that carries every kind of parameter across the seam.
//!
//! # In plain terms
//!
//! An effect has parameters, and they are not all numbers: a blur has a radius
//! (a number), a fill has a colour, a tile has a centre point, a glow has an
//! on/off switch, a noise has a random seed, a dropdown has a chosen option, a
//! displacement map has a file, and a depth blur points at another layer. Any of
//! the number-shaped ones may also be *animated* — following a curve of
//! keyframes instead of holding one value.
//!
//! [`BridgeEffectValue`] is one type that can be any of those things, so the
//! panel can read a parameter without knowing in advance which kind it is, and
//! write it back without flattening it. Its rule is that reading and writing are
//! exact inverses: whatever comes out can go straight back in and the document is
//! unchanged. That is what lets the panel treat "read the value, change one
//! field, write it" as safe — the ordinary way every control in it works.

use std::sync::Arc;

use flutter_rust_bridge::frb;
pub use lumit_core::model::EffectInstance;
use lumit_core::{
    anim::{Animation, Keyframe, Property, SideInterp},
    expression::ExpressionContext,
    model::{EffectParam, EffectValue, FileParam},
    time::Rational,
};
use serde_json::json;
use uuid::Uuid;

use crate::api::{layer::LayerReference, state::PROJECTS, BridgeError};

/// One built-in effect as the Add-effect menu needs it: the stable `name` to
/// pass to [`crate::api::layer::LayerReference::add_effect`], the sentence-case
/// `label` to draw, and the category to group under. `category` is a stable
/// machine key the menu sorts by; `category_label` is its heading (K-090).
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeEffectInfo {
    pub name: String,
    pub label: String,
    pub category: String,
    pub category_label: String,
    /// Where this entry came from: [`NAMESPACE_BUILTIN`] or [`NAMESPACE_OFX`].
    ///
    /// A discovered plugin is drawn exactly as a built-in is — same row, same
    /// star, same drag — and this is the one difference: docs/12 §2.6 asks for
    /// "a small provenance tag in the effect's context menu", and nothing else.
    /// It rides on the listing rather than being a second call, because the
    /// browser needs it for every row it draws.
    pub namespace: String,
    /// The sockets an instance of this entry would draw, from its declaration
    /// alone (K-471 §1.3) — the parameters that can take a wire.
    ///
    /// Here because the Graph panel has to know an entry's ports *before* it is
    /// in the document: it is what lets adding a driver and joining it to the
    /// wire in hand be one commit and so one undo step, and what lets the Tab
    /// search show only the entries a dragged wire could land on. `wired` is
    /// always false — nothing is wired on a catalogue entry.
    pub inputs: Vec<crate::api::graph::BridgePort>,
    /// The output sockets — empty for every image effect, and the declared
    /// [`Signature::Data`](lumit_core::fx::Signature::Data) ports for a driver.
    pub outputs: Vec<crate::api::graph::BridgePort>,
}

/// Every built-in **effect**, in schema order — the Add-effect menu's source of
/// truth ([`lumit_core::fx::BUILTINS`]), and the frb form of v0's `list_effects`.
///
/// Stateless, so it is a free function rather than a method: the menu is
/// available before any project is open.
///
/// **The drivers are here too**, filed under Controls
/// ([`FxCategory::grouping`](lumit_core::fx::FxCategory::grouping)): one search
/// surface offers everything a layer can be given, and applying a driver puts
/// it on the layer's graph rather than its stack — `LayerReference::add_effect`
/// decides that, so no caller has to. [`list_drivers`] still answers the
/// canvas's narrower question: which entries may be *dropped on the graph*.
#[frb(sync)]
pub fn list_effects() -> Vec<BridgeEffectInfo> {
    catalogue(|_| true)
}

/// The Drivers family (K-471 §1.3) — the Graph panel's own search list, in the
/// same shape and the same schema order as [`list_effects`].
///
/// Its own listing rather than a filter the frontend applies, because the
/// distinction is the engine's: a driver makes a value, not a picture, so the
/// canvas is the only place one can be *dropped*. Every entry carries the
/// `controls` grouping key and heading its browse family gives it, so a driver
/// reads as one more Controls entry wherever it is listed.
#[frb(sync)]
pub fn list_drivers() -> Vec<BridgeEffectInfo> {
    catalogue(|category| category == lumit_core::fx::FxCategory::Drivers)
}

/// An entry Lumit itself declares.
pub const NAMESPACE_BUILTIN: &str = "builtin";
/// An entry that came out of an OFX plugin on this machine (docs/12 §2.6).
pub const NAMESPACE_OFX: &str = "ofx";
/// An entry that came out of an **audio plugin** — CLAP or VST3, told apart
/// only by the match name's own prefix (K-707, AP5). One value for both
/// standards because the browser draws them the same: one Audio plugins
/// group, one switch-off command, one provenance line.
pub const NAMESPACE_AUDIO: &str = "audio";

/// Whether a catalogue name is an audio plugin's — the match name's own prefix,
/// which is the one place provenance is carried (K-707).
#[frb(ignore)]
fn is_audio_match_name(match_name: &str) -> bool {
    match_name.starts_with(lumit_core::fx::CLAP_MATCH_PREFIX)
        || match_name.starts_with(lumit_core::fx::VST3_MATCH_PREFIX)
}

/// The category key a plugin's own menu path becomes.
///
/// Prefixed, so a plugin whose grouping happens to read `Blur & sharpen` still
/// gets a heading of its own rather than being folded into Lumit's — the two
/// are not the same list and a coincidence of wording must not merge them. A
/// plugin that declares no grouping at all falls to the bare prefix, and the
/// browser draws that heading in its own words.
#[frb(ignore)]
fn plugin_category_key(grouping: &str) -> String {
    if grouping.is_empty() {
        NAMESPACE_OFX.to_owned()
    } else {
        format!("{NAMESPACE_OFX}/{grouping}")
    }
}

/// The catalogue walk both listings share, so the two cannot drift apart on
/// what an entry looks like.
///
/// **The whole catalogue, not the built-in slice** (K-593/K-594): the run-time
/// half is where a discovered plugin lives, and walking `BUILTINS` would have
/// left every plugin out of the one listing the browser reads. The built-ins
/// still come first and in schema order, because that is the order the
/// catalogue itself keeps.
#[frb(ignore)]
fn catalogue(keep: impl Fn(lumit_core::fx::FxCategory) -> bool) -> Vec<BridgeEffectInfo> {
    lumit_core::fx::BUILTIN_DEFS
        .iter()
        .map(lumit_core::fx::EffectDef::schema)
        .filter(|schema| keep(schema.category))
        .map(|schema| {
            let (inputs, outputs) = crate::api::graph::catalogue_ports(schema.match_name);
            // A plugin places itself: its declared grouping is its heading, and
            // none of Lumit's ten categories is a claim about somebody else's
            // effect (docs/12 §2.6). Read here so `list_effects` is still one
            // call — a browser that had to ask a second question per row would
            // be one call per effect per rebuild.
            let plugin = lumit_ofx::discover::plugin_of(schema.match_name);
            // An audio plugin is one group, not one per vendor (AP5): neither
            // standard declares a menu path the way OFX does, so the browser
            // gets a single Audio plugins heading, worded by the frontend —
            // which is what an empty `category_label` means.
            if is_audio_match_name(schema.match_name) {
                return BridgeEffectInfo {
                    name: schema.match_name.to_owned(),
                    label: schema.label.to_owned(),
                    category: NAMESPACE_AUDIO.to_owned(),
                    category_label: String::new(),
                    namespace: NAMESPACE_AUDIO.to_owned(),
                    inputs,
                    outputs,
                };
            }
            BridgeEffectInfo {
                name: schema.match_name.to_owned(),
                label: schema.label.to_owned(),
                // Shared with v0 rather than restated, so the two frontends cannot
                // disagree about which key a category has.
                //
                // The *browse* family, not the declared one: a driver declares
                // Drivers and is filed under Controls, and this is the one seam
                // where that merge happens.
                category: plugin.as_ref().map_or_else(
                    || crate::edits::fx_category_key(schema.category.grouping()).to_owned(),
                    |found| plugin_category_key(&found.grouping),
                ),
                category_label: plugin.as_ref().map_or_else(
                    || schema.category.grouping().label().to_owned(),
                    |found| found.grouping.clone(),
                ),
                // **Built-in means in the compile-time slice**, not "the scan
                // has never heard of it". The two agree for every plugin that
                // arrived through the scan, which is every plugin in a running
                // Lumit; they part company for a definition registered by some
                // other route, and calling that one a built-in would have the
                // frontend offer to do built-in things to somebody else's
                // effect (K-595).
                namespace: if lumit_core::fx::BUILTIN_DEFS
                    .builtins()
                    .any(|def| def.schema().match_name == schema.match_name)
                {
                    NAMESPACE_BUILTIN
                } else {
                    NAMESPACE_OFX
                }
                .to_owned(),
                inputs,
                outputs,
            }
        })
        .collect()
}

/// What one scan of the machine's plugin folders did (docs/12 §2.6).
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BridgePluginScan {
    /// The labels of the effects this scan added, in the order they were found.
    /// Empty for a rescan that found nothing new, which is the usual answer.
    pub registered: Vec<String>,
    /// One calm line per bundle or plugin turned away — a broken install, a
    /// context this host cannot drive, a plugin the user switched off. Shown as
    /// a report, never as a dialogue.
    pub skipped: Vec<String>,
}

/// Scan the standard OFX folders (and `OFX_PLUGIN_PATH`) **and the standard
/// CLAP and VST3 folders** (and `CLAP_PATH`, `VST3_PATH`), and offer what is
/// found as effects.
///
/// **Deliberately not `frb(sync)`**: this opens other people's bundles and
/// spawns a broker process per bundle, which is tens of milliseconds at best
/// and seconds on a machine with a large suite installed. flutter_rust_bridge
/// runs it on a worker and hands Dart a future, so the interface never waits on
/// it — the start-up scan and the rescan command are the same call.
///
/// Registration is additive and idempotent: calling this twice registers each
/// plugin once (K-593), so a rescan after installing something new is safe at
/// any moment.
pub fn rescan_plugins() -> BridgePluginScan {
    let prefs = lumit_project::PluginPrefs::load_default();
    // The stored preference becomes the running one first, so a plugin
    // registered by an earlier scan and switched off since is switched off for
    // this scan's renders too, not only absent from it.
    lumit_ofx::discover::set_disabled(&prefs.disabled);
    let options = lumit_ofx::discover::ScanOptions {
        disabled: prefs.disabled.clone(),
        ..lumit_ofx::discover::ScanOptions::standard()
    };
    let outcome =
        lumit_ofx::discover::scan(&options, &mut |def| lumit_render::gpufx::ofx::register(def));
    let mut scan = BridgePluginScan {
        registered: outcome
            .registered
            .iter()
            .map(|found| found.label.clone())
            .collect(),
        skipped: outcome.skipped,
    };
    scan_audio_plugins(&prefs, &mut scan);
    scan
}

/// The **audio** half of a rescan: the machine's CLAP *and VST3* folders,
/// described in a broker process, registered into the same catalogue (K-700,
/// K-707).
///
/// One call for both standards on purpose. `search_paths` is both standards'
/// folders, `scan_brokered` spawns the same broker binary for either kind of
/// file, and what comes back is the same definition — so a VST3 plugin becomes
/// an effect by exactly the road a CLAP one does, and the frontend cannot tell
/// which it got except by the match name.
///
/// One list of switched-off identifiers serves both hosts (K-594), and the
/// session's copy is written before the scan for the same reason the OFX one
/// is: a plugin switched off earlier must be switched off for *this* session's
/// mixes too, not merely absent from the listing.
///
/// A machine with neither folder scans nothing, registers nothing and reports
/// nothing, which is what a build with no audio plugins installed should cost.
fn scan_audio_plugins(prefs: &lumit_project::PluginPrefs, scan: &mut BridgePluginScan) {
    lumit_aplug::set_disabled(&prefs.disabled);
    let outcome = lumit_aplug::scan_brokered(
        &lumit_aplug::search_paths(),
        &lumit_aplug::session_disabled(),
        None,
    );
    for found in outcome.found {
        // A rescan is not a second effect. `register` would refuse the name
        // anyway; asking first is what stops a rescan leaking the definition it
        // built on the way to being refused, since a catalogue entry is
        // `'static` and leaking is how that lifetime is spelled.
        if lumit_core::fx::BUILTIN_DEFS
            .get(&found.match_name)
            .is_some()
        {
            continue;
        }
        // The definition and the mix seam arrive together, which is what AP1
        // deliberately stopped short of: registering is what makes the layer's
        // stack able to hold one, and `open_audio` is what makes it sound.
        if lumit_core::fx::BUILTIN_DEFS.register(found.def.leak()) {
            scan.registered.push(found.label);
        }
    }
    scan.skipped.extend(outcome.skipped);
}

/// Switch a discovered plugin on or off, by the `match_name` the listing hands
/// out (docs/12 §2.6).
///
/// Takes the match name rather than the plugin's own identifier because that is
/// what the browser holds; deriving one from the other is the engine's business
/// and would otherwise be a rule the frontend had to know. A name that is not a
/// plugin's is simply ignored.
///
/// Two things happen: the answer is written to the preferences, so it survives
/// a restart, and it takes effect **now** — a plugin switched off mid-session
/// renders its input unchanged and its layers wear a badge, rather than the
/// change waiting for a relaunch. Switching one back on does not re-register it
/// within the session if it was never scanned in; a rescan does that.
///
/// # Errors
///
/// [`BridgeError::WriteFailed`] when the preference could not be written — the
/// answer still holds for this session.
#[frb(sync)]
pub fn set_plugin_enabled(effect: String, enabled: bool) -> Result<(), BridgeError> {
    // Three prefixes, two hosts, one preference file (K-594, K-707): the name's
    // own prefix says which host is told, and the file is written the same way
    // whichever it was. CLAP and VST3 are one host and one switched-off list —
    // the identifier is a plugin id or a class id, and the list holds either.
    let audio = effect
        .strip_prefix(lumit_core::fx::CLAP_MATCH_PREFIX)
        .or_else(|| effect.strip_prefix(lumit_core::fx::VST3_MATCH_PREFIX));
    let identifier = match (effect.strip_prefix(lumit_core::fx::OFX_MATCH_PREFIX), audio) {
        (Some(ofx), _) => {
            lumit_ofx::discover::set_enabled(ofx, enabled);
            ofx
        }
        (_, Some(plugin)) => {
            lumit_aplug::set_enabled(plugin, enabled);
            plugin
        }
        _ => return Ok(()),
    };
    let mut prefs = lumit_project::PluginPrefs::load_default();
    if !prefs.set_enabled(identifier, enabled) {
        return Ok(());
    }
    prefs.save_default().map_err(|_| BridgeError::WriteFailed)
}

/// What plugins have asked Lumit to say since this was last called, oldest
/// first, taken as they are read.
///
/// **Never modal** (docs/12 §2.2, open question): a plugin that wants to tell
/// the user something gets a calm toast on the status strip and nothing more,
/// until the owner says what the Message suite should look like. A plugin that
/// asks a *question* has already been told "you decide" at the suite, which is
/// the reply OFX defines for a host that cannot ask.
///
/// Empty on every session with no plugins, which is what makes polling it from
/// the shell's existing tick free.
#[frb(sync)]
pub fn plugin_messages() -> Vec<String> {
    lumit_ofx::host::state()
        .take_messages()
        .into_iter()
        .map(|message| message.text)
        .filter(|text| !text.is_empty())
        .collect()
}

/// One saved `.lumfx` preset in the user's library: the display name it was
/// saved under and the file to read when applying it.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgePresetInfo {
    pub name: String,
    pub path: String,
}

/// Every `.lumfx` in the preset library folder, sorted by name — what the
/// Effects & presets browser lists. A file that is not a preset (unreadable,
/// or not preset JSON) is simply not listed; the folder is the user's to put
/// things in, and a stray file there is not a fault.
#[frb(sync)]
pub fn list_presets() -> Vec<BridgePresetInfo> {
    lumit_project::presets_dir()
        .map(|dir| presets_in(&dir, lumit_core::preset::PRESET_EXTENSION))
        .unwrap_or_default()
}

/// Every `.lumgrp` **node group** in the same library folder, sorted by name
/// (K-651) — what the graph canvas's search offers beside the drivers.
///
/// The same folder as the effect presets, because it is the same kind of thing:
/// something this person saved to use again, on any project.
#[frb(sync)]
pub fn list_node_groups() -> Vec<BridgePresetInfo> {
    lumit_project::presets_dir()
        .map(|dir| presets_in(&dir, lumit_core::preset::GROUP_EXTENSION))
        .unwrap_or_default()
}

/// Where the preset library lives, created on first ask — the save dialogue's
/// default folder, so a saved preset appears in the listing without the user
/// navigating anywhere. `None` only when the platform has no home directory.
#[frb(sync)]
pub fn presets_dir_path() -> Option<String> {
    let dir = lumit_project::presets_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.to_string_lossy().into_owned())
}

/// The listing itself, on any folder and for either extension — split from
/// [`list_presets`] so the scan is testable without touching the user's real
/// library, and shared with [`list_node_groups`] so an effect preset and a node
/// group are found, named and sorted by exactly the same rules.
#[frb(ignore)]
pub(crate) fn presets_in(dir: &std::path::Path, extension: &str) -> Vec<BridgePresetInfo> {
    #[derive(serde::Deserialize)]
    struct Named {
        name: Option<String>,
        // Presence is the "is this actually one of ours" check — an effect
        // preset carries `effects`, a node group carries `nodes`. What is in
        // them is parsed properly at load time.
        effects: Option<serde_json::Value>,
        nodes: Option<serde_json::Value>,
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<BridgePresetInfo> = entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if !path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
            {
                return None;
            }
            let text = std::fs::read_to_string(&path).ok()?;
            // It must at least be one of our documents with its list in it; the
            // saved display name wins, the file's stem stands in without one.
            let named: Named = serde_json::from_str(&text).ok()?;
            if !named
                .effects
                .as_ref()
                .or(named.nodes.as_ref())
                .is_some_and(serde_json::Value::is_array)
            {
                return None;
            }
            let name = named
                .name
                .filter(|n| !n.trim().is_empty())
                .or_else(|| Some(path.file_stem()?.to_string_lossy().into_owned()))?;
            Some(BridgePresetInfo {
                name,
                path: path.to_string_lossy().into_owned(),
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// What `scalar` reads as at `time` — the value the picture is actually showing.
///
/// The panel needs this at exactly two moments, and both would be wrong done in
/// Dart. Turning animation *off* keeps the value the curve currently has, rather
/// than snapping to whatever the first key holds; adding a key at the playhead
/// seeds it with the value already on screen, so the act of adding a key never
/// moves the picture. Bezier keys make either one a real evaluation, not a
/// lerp — so this is the engine's own [`lumit_core::anim::evaluate`] rather than
/// a second implementation that would disagree with the renderer.
///
/// Sampling is in `f64` seconds, matching the engine: exactness is a property of
/// key *times* (which cross as integer pairs), not of a sampled value.
#[frb(sync)]
pub fn sample_scalar(scalar: BridgeScalar, time: BridgeRational) -> f64 {
    sample_at(scalar, seconds_of(time))
}

/// Every one of `scalars` at the same `time`, in the order they were given.
///
/// The same answers as calling [`sample_scalar`] once per scalar, and the reason
/// it exists is that the panel wants them all at once: on each frame of a scrub
/// or a playback, *every* animated row on screen asks what its curve reads now.
/// That was one crossing of the boundary per row per frame — chatter that grew
/// with the number of lanes open, so a `U` on a busy layer made the playhead
/// lag over frames the cache already held. One crossing carries the lot.
#[frb(sync)]
pub fn sample_scalars(scalars: Vec<BridgeScalar>, time: BridgeRational) -> Vec<f64> {
    let seconds = seconds_of(time);
    scalars
        .into_iter()
        .map(|scalar| sample_at(scalar, seconds))
        .collect()
}

/// A bridge time in seconds, and zero for the denominator no rational has.
fn seconds_of(time: BridgeRational) -> f64 {
    if time.den == 0 {
        0.0
    } else {
        time.num as f64 / time.den as f64
    }
}

/// The shared body of the two samplers above: what one scalar reads at
/// `seconds`, with no expression context.
fn sample_at(scalar: BridgeScalar, seconds: f64) -> f64 {
    match scalar {
        BridgeScalar::Static(value) => value,
        BridgeScalar::Keyframed(keys) => {
            let keys: Vec<Keyframe> = keys
                .iter()
                .map(|k| Keyframe {
                    time: Rational::new(k.time.num, k.time.den).unwrap_or(Rational::ZERO),
                    value: k.value,
                    interp_in: k.interp_in.write(),
                    interp_out: k.interp_out.write(),
                })
                .collect();
            lumit_core::anim::evaluate(&keys, seconds).unwrap_or(0.0)
        }
        BridgeScalar::Expression(expr) => lumit_core::expression::evaluate(&expr, None),
    }
}

#[frb(sync)]
pub fn sample_scalar_with_context(
    scalar: BridgeScalar,
    time: BridgeRational,
    layer: LayerReference,
) -> f64 {
    let seconds = seconds_of(time);
    match scalar {
        BridgeScalar::Static(value) => value,
        BridgeScalar::Keyframed(keys) => {
            let keys: Vec<Keyframe> = keys
                .iter()
                .map(|k| Keyframe {
                    time: Rational::new(k.time.num, k.time.den).unwrap_or(Rational::ZERO),
                    value: k.value,
                    interp_in: k.interp_in.write(),
                    interp_out: k.interp_out.write(),
                })
                .collect();
            lumit_core::anim::evaluate(&keys, seconds).unwrap_or(0.0)
        }
        BridgeScalar::Expression(expr) => {
            let Some(doc) = document_for(&layer) else {
                return 0.0;
            };

            lumit_core::expression::evaluate(
                &expr,
                Some(Arc::new(ExpressionContext {
                    document: doc.clone(),
                    comp: Some(layer.comp_id),
                    layer: Some(layer.layer_id),
                    comp_time: Rational::new(time.num, time.den)
                        .unwrap_or(Rational::ZERO)
                        .to_f64(),
                    current_depth: 0,
                })),
            )
        }
    }
}

/// The project document a layer reference points into.
///
/// `None` when the project has gone — closed between the panel asking and this
/// answering, or a lock poisoned by an unrelated panic. Neither is worth taking
/// the app down for from inside an FFI call, where a panic unwinds across the
/// language boundary rather than into a handler, so the samplers below fall
/// back to the un-driven value instead.
fn document_for(layer: &LayerReference) -> Option<Arc<lumit_core::Document>> {
    let projects = PROJECTS.read().ok()?;
    let project = projects.get(&layer.project_id)?.clone();
    drop(projects);
    let state = project.read().ok()?;
    Some(state.store.snapshot())
}

#[frb(sync)]
pub fn sample_scalar_range_with_context(
    scalar: BridgeScalar,
    layer: LayerReference,
    start: BridgeRational,
    end: BridgeRational,
    samples: i64,
) -> Vec<f64> {
    match scalar {
        BridgeScalar::Expression(expr) => {
            let Some(doc) = document_for(&layer) else {
                return Vec::new();
            };

            let start = Rational::new(start.num, start.den)
                .unwrap_or(Rational::ZERO)
                .to_f64();

            let end = Rational::new(end.num, end.den)
                .unwrap_or(Rational::ZERO)
                .to_f64();

            lumit_core::expression::evaluate_range(
                &expr,
                Some(&ExpressionContext {
                    document: doc.clone(),
                    comp: Some(layer.comp_id),
                    layer: Some(layer.layer_id),
                    comp_time: 0.0, // this time will be overwritten internally,
                    current_depth: 0,
                }),
                start,
                end,
                samples,
            )
        }
        // Only an expression needs sampling by evaluation. A static value is
        // flat and a keyframed one is drawn from its keys, both of which the
        // graph editor already has without asking the engine.
        _ => Vec::new(),
    }
}

/// One declared parameter of an effect, as the panel needs to *draw* it:
/// what to call it, what kind of control it is, and the range or option list
/// that control needs.
///
/// This is the schema, not the value — [`BridgeEffectValue`] carries what a
/// particular instance currently holds. The panel needs both: the value to show,
/// and this to know whether "0.5" wants a slider from 0 to 100 or a colour
/// channel, and what the third entry in a dropdown is called.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeParamInfo {
    /// Stable snake_case id — the key [`BridgeEffectInstance::get_value`] and
    /// `set_value` take.
    pub id: String,
    pub label: String,
    pub kind: BridgeParamKind,
    /// What the number *is* (K-443): the rider the row draws beside the value,
    /// and — on a point pair — the unit a Viewer pick has to write in. Declared
    /// per parameter engine-side, so a row's unit travels with the row rather
    /// than with its id and the panel never has to guess.
    pub unit: BridgeUnit,
}

/// What kind of control a parameter wants, and the numbers that control needs.
///
/// Mirrors [`lumit_core::fx::ParamKind`]. `Seed` and `Layer` carry nothing: a
/// seed is any `u32`, and a layer picker's options are the comp's own layers,
/// which the panel already has.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub enum BridgeParamKind {
    Float {
        default: f64,
        /// The slider's travel. Typing may exceed it (docs/08 §1.2); only
        /// `hard_min`/`hard_max` may not.
        slider_min: f64,
        slider_max: f64,
        /// Hard bounds, either side open (K-090: a threshold clamps at zero
        /// below and runs unbounded above).
        hard_min: Option<f64>,
        hard_max: Option<f64>,
    },
    /// A whole-number parameter (docs/08 §1.2): the value is still a Float
    /// scalar in the model — the kind only asks the row to step, display and
    /// commit whole numbers.
    Int {
        default: i64,
        slider_min: i64,
        slider_max: i64,
        hard_min: Option<i64>,
        hard_max: Option<i64>,
    },
    /// Degrees, drawn as a dial beneath the number (docs/07 §6). The value
    /// crossing the bridge is a [`BridgeEffectValue::Float`] — an angle is a
    /// number of degrees, and this kind only says which control to draw.
    /// Unbounded, so the dial winds through full turns.
    Angle {
        default: f64,
        /// Snapping increment in degrees while a modifier is held.
        dial_step: f64,
    },
    Choice {
        options: Vec<String>,
        default: u32,
        /// Option indices after which the dropdown draws a group divider (T21).
        /// Empty for an ungrouped list.
        dividers_after: Vec<u32>,
    },
    Bool {
        default: bool,
    },
    Colour {
        /// Scene-linear RGBA. Channels animate independently in the model, so
        /// the panel edits four scalars behind one swatch.
        default: Vec<f64>,
        /// Per-channel edit range — a linear value may exceed 1 (an HDR tint)
        /// or dip below 0 (a lift), so each colour declares its own.
        min: f64,
        max: f64,
    },
    Seed,
    File {
        /// Lower-case extensions without the dot, for the open dialog.
        filter: Vec<String>,
        filter_name: String,
    },
    Layer,
    /// One of the **owning layer's masks**, whose geometry the effect walks
    /// (K-408, docs/08 §1.2). The panel draws the layer's masks by name, with
    /// "First mask" as the unset entry; the mask names come from the read model
    /// the panel already holds, so the row costs no call of its own.
    MaskPath,
    /// A tone curve, drawn as a curve editor (K-412). The panel edits the
    /// point list itself; there is no range to declare, because the points
    /// live in the unit square by definition.
    Curve,
    /// A closed range (K-414), drawn as a track and thumb with the value
    /// beside it. `min`/`max` are the travel *and* the hard bound — that is
    /// what closed means — so the row refuses a typed value outside them.
    ///
    /// The value crossing the bridge is a [`BridgeEffectValue::Float`], the
    /// arrangement `Int` and `Angle` already use: the kind says which control
    /// to draw, not how the number is stored, so the row keeps every float
    /// affordance including keyframes and the graph editor.
    Slider {
        default: f64,
        min: f64,
        max: f64,
    },
    /// A **button** (K-417), drawn as one and pressed through
    /// [`crate::api::layer::LayerReference::fire_effect_action`]. It carries no
    /// value at all — no default, no range, nothing in
    /// [`BridgeEffectInstanceInfo::values`] — because a press is an event and
    /// not a number that could be keyframed, undone or interpolated.
    Action,
}

/// Every parameter `effect` declares, in schema order — what the panel draws a
/// row per.
///
/// Keyed by the same `match_name` [`list_effects`] hands out and `add_effect`
/// takes. An unknown name is an empty list rather than an error: a project
/// carrying an effect this build does not know still opens, and its instance
/// simply has no rows to draw.
#[frb(sync)]
pub fn list_parameters(effect: String) -> Vec<BridgeParamInfo> {
    // The **whole** catalogue: a plugin's parameters have to reach Effect
    // controls exactly as a built-in's do, and `BUILTINS` is the compile-time
    // half alone (K-593/K-594).
    let Some(schema) = lumit_core::fx::BUILTIN_DEFS
        .get(&effect)
        .map(lumit_core::fx::EffectDef::schema)
    else {
        return Vec::new();
    };

    schema.params.iter().map(bridge_param).collect()
}

/// One declared row, as the panel reads it.
///
/// Lifted out of [`list_parameters`] so the **derived** rows an instance carries
/// (docs/impl/custom-shader.md §1.5) cross by the identical road: a derived
/// parameter is an ordinary `ParamSchema`, and nothing downstream of here may
/// be able to tell the two apart.
#[frb(ignore)]
pub(crate) fn bridge_param(param: &lumit_core::fx::ParamSchema) -> BridgeParamInfo {
    use lumit_core::fx::ParamKind;

    let kind = match param.kind {
        ParamKind::Float {
            default,
            slider,
            hard,
        } => BridgeParamKind::Float {
            default,
            slider_min: slider.0,
            slider_max: slider.1,
            hard_min: hard.0,
            hard_max: hard.1,
        },
        // A closed range (K-414) crosses as its own kind now that the
        // panel draws one: a track and thumb with the value beside it.
        // The *value* still crosses as a Float scalar, so the row keeps
        // every float path — keyframes, the graph editor, the
        // expression seed — exactly as an Int row does.
        ParamKind::Slider { default, range } => BridgeParamKind::Slider {
            default,
            min: range.0,
            max: range.1,
        },
        ParamKind::Int {
            default,
            slider,
            hard,
        } => BridgeParamKind::Int {
            default,
            slider_min: slider.0,
            slider_max: slider.1,
            hard_min: hard.0,
            hard_max: hard.1,
        },
        ParamKind::Choice {
            options,
            default,
            dividers_after,
        } => BridgeParamKind::Choice {
            options: options.iter().map(|o| (*o).to_owned()).collect(),
            default,
            dividers_after: dividers_after.to_vec(),
        },
        ParamKind::Bool { default } => BridgeParamKind::Bool { default },
        ParamKind::Colour { default, range } => BridgeParamKind::Colour {
            default: default.to_vec(),
            min: range.0,
            max: range.1,
        },
        ParamKind::Seed => BridgeParamKind::Seed,
        ParamKind::File {
            filter,
            filter_name,
        } => BridgeParamKind::File {
            filter: filter.iter().map(|f| (*f).to_owned()).collect(),
            filter_name: filter_name.to_owned(),
        },
        ParamKind::Angle { default, dial_step } => BridgeParamKind::Angle { default, dial_step },
        // `self_default` is an engine-side instantiation detail
        // (K-288) — the panel draws the same picker either way, and
        // the value it edits already carries the layer id.
        ParamKind::Layer { .. } => BridgeParamKind::Layer,
        // `self_default` is an engine-side resolution detail here too
        // (K-408): the panel always offers "First mask" as its unset
        // entry, and what an unset row comes to is the render's answer,
        // not a control the panel draws differently.
        ParamKind::MaskPath { .. } => BridgeParamKind::MaskPath,
        // The declared default is not sent: a fresh instance is born
        // with it written into the document (`default_param_value`),
        // so the panel draws the curve it stores, never one the seam
        // had to describe.
        ParamKind::Curve { .. } => BridgeParamKind::Curve,
        // A button (K-417). The row crosses so the panel can draw one;
        // the *value* never does, because there is none — the press
        // goes back as an event on the owning layer
        // (`fire_effect_action`), not as a write.
        ParamKind::Action => BridgeParamKind::Action,
    };
    BridgeParamInfo {
        id: param.id.to_owned(),
        label: param.label.to_owned(),
        kind,
        unit: bridge_unit(param.unit),
    }
}

/// The unit a parameter's number is in (K-443) — what the row draws as its
/// rider beside the value, and what a point pick has to write in.
///
/// Mirrors [`lumit_core::fx::Unit`] with the two the seam has no use for
/// folded away: `Unset` is a build failure engine-side, so nothing that ships
/// can carry it, and `PctDiag` is forbidden to every parameter (K-419). Both
/// arrive here as [`BridgeUnit::Raw`] — the panel draws no rider, which is the
/// honest answer for a unit that must not exist.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeUnit {
    /// A plain number: a gamma, a count, a stop, a rate in Hz. No rider.
    Raw,
    /// Per cent, where 100 is the whole of whatever it is a share of.
    Percent,
    /// Pixels at composition size (px@comp) — the one spatial unit (K-419).
    Px,
    Degrees,
    Seconds,
    /// Comp-rate frames.
    Frames,
}

#[frb(ignore)]
pub(crate) fn bridge_unit(unit: lumit_core::fx::Unit) -> BridgeUnit {
    use lumit_core::fx::Unit as U;
    match unit {
        U::Percent => BridgeUnit::Percent,
        U::Px => BridgeUnit::Px,
        U::Degrees => BridgeUnit::Degrees,
        U::Seconds => BridgeUnit::Seconds,
        U::Frames => BridgeUnit::Frames,
        // See [`BridgeUnit`]: neither can reach a shipped parameter, and
        // "no rider" is what a panel should draw if one ever did.
        U::Raw | U::Unset | U::PctDiag => BridgeUnit::Raw,
    }
}

/// One **vector pair** of an effect: two adjacent `_x`/`_y` Float parameters
/// the panel draws as one row of two wells with a chain between them (K-443).
///
/// The convention used to be read off the ids at the seam, by whoever needed
/// it; [`lumit_core::fx::EffectSchema::pairs`] is the declaration answering it
/// now, so the panel, the link flag on the instance and the engine all get one
/// answer. `stem` is the key the link flag is stored under, so it is the
/// pair's identity rather than either half's id.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeParamPair {
    pub stem: String,
    pub x: String,
    pub y: String,
}

/// An effect's vector pairs, in schema order — the fourth static list beside
/// [`list_parameters`], and memoised on the Dart side for the same reason
/// (K-183: the schema never changes, and a fetch per card per rebuild is
/// exactly the traffic the budget test forbids).
///
/// An unknown match name is an empty list rather than an error, like every
/// other schema read: a project carrying an effect this build does not know
/// still opens.
#[frb(sync)]
pub fn list_pairs(effect: String) -> Vec<BridgeParamPair> {
    // The **whole** catalogue: a plugin's parameters have to reach Effect
    // controls exactly as a built-in's do, and `BUILTINS` is the compile-time
    // half alone (K-593/K-594).
    let Some(schema) = lumit_core::fx::BUILTIN_DEFS
        .get(&effect)
        .map(lumit_core::fx::EffectDef::schema)
    else {
        return Vec::new();
    };
    schema
        .pairs()
        .map(|p| BridgeParamPair {
            stem: p.stem.to_owned(),
            x: p.x.to_owned(),
            y: p.y.to_owned(),
        })
        .collect()
}

/// One collapsible parameter group of an effect (docs/08 §1.2, K-145/K-257):
/// the panel tucks the named member rows behind a twirl. An empty `label`
/// renders headerless (the rows appear in place, no twirl) — the shape a
/// conditional run of parameters takes. `visible_when_param` with a
/// non-empty `visible_when_values` shows the group only while that sibling
/// Choice parameter holds one of those indices.
// A group whose schema says its rows are per glass element of a lens (K-371)
// arrives as exactly that same shape: `visible_when_param` is the Lens
// dropdown and `visible_when_values` lists the lenses whose prescription has
// enough elements for the row. The panel therefore has one visibility rule to
// implement, not two, and learns nothing about optics. Deliberately NOT a doc
// comment: frb mirrors those into the generated Dart, and this note is about
// how the Rust side fills the fields in, which the frontend need not read.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeParamGroup {
    pub label: String,
    /// Member parameter ids — a contiguous run of the schema's parameters.
    pub params: Vec<String>,
    /// Whether the twirl starts closed.
    pub collapsed: bool,
    /// See the struct docs.
    pub visible_when_param: Option<String>,
    /// See the struct docs. Empty when the group is unconditional.
    pub visible_when_values: Vec<u32>,
}

/// The Lens flare's lens-pick parameter, whose value the per-element coating
/// rows' visibility is resolved against (K-371).
///
/// Spelled once, here, because getting it wrong is silent: the panel looks the
/// sibling up by id, finds nothing, and hides every row that names it — which
/// is exactly what shipping `"lens"` instead of `"lens_model"` did. The rows
/// that survived were the ones with an unreachable threshold, whose empty
/// value set means "always". The constants test pins this against the schema.
pub(crate) const LENS_PICK_PARAM: &str = "lens_model";

/// Every parameter group `effect` declares, in schema order (empty for an
/// effect with none, or an unknown name). The panel inserts each group's
/// twirl at its first member's position and hides the members it covers from
/// the flat run.
#[frb(sync)]
pub fn list_parameter_groups(effect: String) -> Vec<BridgeParamGroup> {
    // The **whole** catalogue: a plugin's parameters have to reach Effect
    // controls exactly as a built-in's do, and `BUILTINS` is the compile-time
    // half alone (K-593/K-594).
    let Some(schema) = lumit_core::fx::BUILTIN_DEFS
        .get(&effect)
        .map(lumit_core::fx::EffectDef::schema)
    else {
        return Vec::new();
    };
    schema
        .groups
        .iter()
        .map(|g| BridgeParamGroup {
            label: g.label.to_owned(),
            params: g.params.iter().map(|p| (*p).to_owned()).collect(),
            collapsed: g.collapsed,
            // The per-element rows (K-371) become an ordinary "this sibling
            // Choice holds one of these" rule, resolved from the lens library
            // here so the frontend needs no new mechanism and no notion of an
            // element. The two conditions are mutually exclusive by
            // construction: a group declares one or the other.
            visible_when_param: match (g.visible_when, g.visible_when_lens_elements) {
                (Some((id, _)), _) => Some(id.to_owned()),
                (None, Some(_)) => Some(LENS_PICK_PARAM.to_owned()),
                (None, None) => None,
            },
            visible_when_values: match (g.visible_when, g.visible_when_lens_elements) {
                (Some((_, vs)), _) => vs.to_vec(),
                (None, Some(n)) => {
                    let lenses = lumit_core::fx::lens_flare::lenses_with_at_least(n);
                    // **An empty set means "always visible"** (see
                    // `BridgeParamGroup`), which is the opposite of what an
                    // unreachable threshold wants: no bundled lens has
                    // nineteen elements, so element 19's row must never draw
                    // rather than always draw. A set holding one impossible
                    // index says that in the vocabulary the panel already has.
                    if lenses.is_empty() {
                        vec![u32::MAX]
                    } else {
                        lenses
                    }
                }
                (None, None) => Vec::new(),
            },
        })
        .collect()
}

/// One greying rule: `param`'s row is editable only while `on` satisfies
/// `cond`. The panel evaluates it against values it already holds, so ticking
/// a switch greys its dependent row without a round trip;
/// `lumit_core::fx::param_enabled` is the same rule in Rust and the authority
/// the tests pin.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeEnabledWhen {
    pub param: String,
    pub on: String,
    pub cond: BridgeEnabledCond,
}

/// The condition half of a [`BridgeEnabledWhen`], mirroring
/// [`lumit_core::fx::EnabledCond`].
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub enum BridgeEnabledCond {
    /// Editable while the named bool holds this value.
    BoolIs(bool),
    /// Editable while the named choice is on this option index.
    ChoiceIs(u32),
    /// Editable while the named choice is on anything but this index.
    ChoiceIsNot(u32),
    /// Editable while the named layer reference actually names a layer.
    LayerSet,
}

/// Every greying rule `effect` declares (empty for an effect whose controls are
/// all independent, which is most of them, or for an unknown name — a project
/// carrying an effect this build does not know still opens).
#[frb(sync)]
pub fn list_enabled_when(effect: String) -> Vec<BridgeEnabledWhen> {
    use lumit_core::fx::EnabledCond;

    // The **whole** catalogue: a plugin's parameters have to reach Effect
    // controls exactly as a built-in's do, and `BUILTINS` is the compile-time
    // half alone (K-593/K-594).
    let Some(schema) = lumit_core::fx::BUILTIN_DEFS
        .get(&effect)
        .map(lumit_core::fx::EffectDef::schema)
    else {
        return Vec::new();
    };
    schema
        .enabled_when
        .iter()
        .map(|r| BridgeEnabledWhen {
            param: r.param.to_owned(),
            on: r.on.to_owned(),
            cond: match r.cond {
                EnabledCond::BoolIs(v) => BridgeEnabledCond::BoolIs(v),
                EnabledCond::ChoiceIs(i) => BridgeEnabledCond::ChoiceIs(i),
                EnabledCond::ChoiceIsNot(i) => BridgeEnabledCond::ChoiceIsNot(i),
                EnabledCond::LayerSet => BridgeEnabledCond::LayerSet,
            },
        })
        .collect()
}

/// An exact rational time in seconds, as `num / den`.
///
/// Keyframe times cross as the integer pair the document stores, never as
/// floating-point seconds (docs/17 "rational time crosses as integers"): a key
/// at 1/3 s read back as 0.333… and written again would no longer land on the
/// frame it was set on, and this round trip has to be exact.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeRational {
    pub num: i64,
    /// Always positive in anything the engine hands out; a zero or negative
    /// denominator coming the other way is refused, not normalised.
    pub den: i64,
}

/// A bezier side's After Effects-compatible handle: `speed` in value-units per
/// second, `influence` as a fraction of the gap to the neighbouring key.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeBezierSide {
    pub speed: f64,
    pub influence: f64,
}

/// An **automatic** bezier side ([`SideInterp::Auto`]): its speed is computed
/// from the key's neighbours, `clamped` saying whether the computation is the
/// plain smooth one or the one that cannot overshoot them.
///
/// `speed` and `influence` are the ease the side carried when it was last
/// free. They cross in both directions untouched, which is what makes
/// Free → Auto → Free give the custom ease back without the write path having
/// to consult what was there before.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeAutoSide {
    pub clamped: bool,
    pub speed: f64,
    pub influence: f64,
}

/// How a keyframe joins its neighbour on one side ([`SideInterp`]).
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BridgeSideInterp {
    Hold,
    Linear,
    Bezier(BridgeBezierSide),
    Auto(BridgeAutoSide),
}

impl BridgeSideInterp {
    #[frb(ignore)]
    pub(crate) fn read(side: SideInterp) -> BridgeSideInterp {
        match side {
            SideInterp::Hold => BridgeSideInterp::Hold,
            SideInterp::Linear => BridgeSideInterp::Linear,
            SideInterp::Bezier { speed, influence } => {
                BridgeSideInterp::Bezier(BridgeBezierSide { speed, influence })
            }
            SideInterp::Auto {
                clamped,
                speed,
                influence,
            } => BridgeSideInterp::Auto(BridgeAutoSide {
                clamped,
                speed,
                influence,
            }),
        }
    }

    #[frb(ignore)]
    pub(crate) fn write(self) -> SideInterp {
        match self {
            BridgeSideInterp::Hold => SideInterp::Hold,
            BridgeSideInterp::Linear => SideInterp::Linear,
            BridgeSideInterp::Bezier(side) => SideInterp::Bezier {
                speed: side.speed,
                influence: side.influence,
            },
            BridgeSideInterp::Auto(side) => SideInterp::Auto {
                clamped: side.clamped,
                speed: side.speed,
                influence: side.influence,
            },
        }
    }
}

/// One keyframe on one scalar channel.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeKeyframe {
    pub time: BridgeRational,
    pub value: f64,
    /// Approaching this key.
    pub interp_in: BridgeSideInterp,
    /// Leaving this key.
    pub interp_out: BridgeSideInterp,
}

impl BridgeKeyframe {
    /// One key, its time carried into **comp** time by `offset` — the layer's
    /// `start_offset`, where its own zero sits on the composition's clock
    /// (K-213). The engine keys every property in layer-local seconds so a
    /// layer's animation travels with it; the interface draws and edits in comp
    /// frames. This is the one place the two are reconciled.
    #[frb(ignore)]
    fn read_at(key: &Keyframe, offset: Rational) -> BridgeKeyframe {
        let time = key.time.checked_add(offset).unwrap_or(key.time);
        BridgeKeyframe {
            time: BridgeRational {
                num: time.num(),
                den: time.den(),
            },
            value: key.value,
            interp_in: BridgeSideInterp::read(key.interp_in),
            interp_out: BridgeSideInterp::read(key.interp_out),
        }
    }

    /// The way back: a comp-time key returned to the layer's own clock.
    #[frb(ignore)]
    fn write_at(&self, offset: Rational) -> Result<Keyframe, BridgeError> {
        Ok(Keyframe {
            time: Rational::new(self.time.num, self.time.den)
                .map_err(|_| BridgeError::InvalidKeyframes)?
                .checked_sub(offset)
                .map_err(|_| BridgeError::InvalidKeyframes)?,
            value: self.value,
            interp_in: self.interp_in.write(),
            interp_out: self.interp_out.write(),
        })
    }
}

/// One animatable scalar channel: a single number, or the curve it follows.
///
/// The two are separate variants rather than a number plus an "animated" flag
/// because the panel must both tell them apart *and* write either back
/// unchanged. A keyframed parameter read as its value at time zero, then written
/// again, would silently delete the animation — which is exactly the trap the
/// `f64`-only predecessor of this type could not avoid, and why it answered
/// nothing at all for an animated parameter.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub enum BridgeScalar {
    Static(f64),
    /// At least one key, strictly ascending in time — the invariant the
    /// engine's keyframe ops maintain, enforced here on the way in.
    Keyframed(Vec<BridgeKeyframe>),
    Expression(String),
}

impl BridgeScalar {
    /// This channel with its keys on the composition's clock — see
    /// [`BridgeKeyframe::read_at`] for why the seam converts (K-213). Pass the
    /// layer's `start_offset`; the offset is never guessed, so a caller that
    /// has no layer cannot forget one.
    #[frb(ignore)]
    pub(crate) fn read_at(property: &Property, offset: Rational) -> BridgeScalar {
        match &property.animation {
            Animation::Static(value) => BridgeScalar::Static(*value),
            // A keyframed property with no keys is not a curve anything can
            // evaluate, and the editing ops never produce one (removing the last
            // key collapses to static). It reads as the value the engine itself
            // would evaluate it to, so it normalises on write-back rather than
            // being an unwritable value.
            Animation::Keyframed(keys) if keys.is_empty() => {
                BridgeScalar::Static(property.value_at(0.0))
            }
            Animation::Keyframed(keys) => BridgeScalar::Keyframed(
                keys.iter()
                    .map(|k| BridgeKeyframe::read_at(k, offset))
                    .collect(),
            ),
            Animation::Expression(expr) => BridgeScalar::Expression(expr.clone()),
        }
    }

    /// This channel as an [`Animation`], or a calm error when the keys are not a
    /// curve the engine can evaluate.
    ///
    /// Deliberately separate from assigning it: a point or a colour has to
    /// validate every channel *before* writing any of them, or a bad third
    /// channel would leave the parameter half-updated.
    #[frb(ignore)]
    pub(crate) fn animation_at(&self, offset: Rational) -> Result<Animation, BridgeError> {
        match self {
            BridgeScalar::Static(value) => Ok(Animation::Static(*value)),
            BridgeScalar::Keyframed(keys) => {
                if keys.is_empty() {
                    return Err(BridgeError::InvalidKeyframes);
                }
                let mut out: Vec<Keyframe> = Vec::with_capacity(keys.len());
                for key in keys {
                    let key = key.write_at(offset)?;
                    // Ascending, unique times: `anim::evaluate` walks the list
                    // assuming it is sorted, so an unsorted one does not error,
                    // it silently evaluates wrongly.
                    if out.last().is_some_and(|previous| key.time <= previous.time) {
                        return Err(BridgeError::InvalidKeyframes);
                    }
                    out.push(key);
                }
                Ok(Animation::Keyframed(out))
            }
            BridgeScalar::Expression(expr) => Ok(Animation::Expression(expr.clone())),
        }
    }
}

/// A point parameter: two independently animatable axes.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgePoint {
    pub x: BridgeScalar,
    pub y: BridgeScalar,
}

/// A colour parameter: four independently animatable scene-linear channels.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeColour {
    pub r: BridgeScalar,
    pub g: BridgeScalar,
    pub b: BridgeScalar,
    pub a: BridgeScalar,
}

/// A file parameter: the paths it references, and the index that selects which
/// one is live. Two paths cannot be blended, so the index only ever steps
/// (hold keyframes, K-111); the common case is one path and a static index.
/// An empty `paths` means unset, which the consuming effect treats as identity.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeFileParam {
    pub paths: Vec<String>,
    pub index: BridgeScalar,
}

/// One effect parameter's value — the bridge mirror of [`EffectValue`], with one
/// variant per kind so no parameter is unreachable.
///
/// Reading and writing are exact inverses (see the module docs): the write side
/// replaces only what a value actually carries, leaving each property's
/// forward-compatibility `extra` fields in place (docs/10 §1.1), so a document
/// saved by a newer Lumit does not lose anything by being read and written here.
///
/// A `Layer` carries a bare id rather than a `LayerReference` because an effect
/// instance is a detached copy that does not know its own composition; the panel
/// resolves the id against `CompositionReference::get_layers`.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub enum BridgeEffectValue {
    Float(BridgeScalar),
    Point(BridgePoint),
    Colour(BridgeColour),
    Bool(bool),
    Choice(u32),
    Seed(u32),
    File(BridgeFileParam),
    Layer(Option<Uuid>),
    /// Which of the owning layer's masks an effect walks (K-408): the mask id,
    /// or `None` for "First mask". The *geometry* never crosses — the render
    /// flattens it engine-side, beside the op.
    MaskPath(Option<Uuid>),
    /// A tone curve as its own control points (K-412): 2..=16 `[x, y]` pairs
    /// in the unit square, in x order. Crosses as written — the engine
    /// straightens what it reads (`CurvePoints::sanitised`), so a panel
    /// mid-drag need not, and a curve is never refused for being momentarily
    /// out of order.
    Curve(Vec<Vec<f32>>),
}

impl BridgeEffectValue {
    /// `offset` is the owning layer's `start_offset`, carrying every key onto
    /// the composition's clock (K-213).
    #[frb(ignore)]
    fn read_at(value: &EffectValue, offset: Rational) -> BridgeEffectValue {
        match value {
            EffectValue::Float(property) => {
                BridgeEffectValue::Float(BridgeScalar::read_at(property, offset))
            }
            EffectValue::Point(x, y) => BridgeEffectValue::Point(BridgePoint {
                x: BridgeScalar::read_at(x, offset),
                y: BridgeScalar::read_at(y, offset),
            }),
            EffectValue::Colour(channels) => BridgeEffectValue::Colour(BridgeColour {
                r: BridgeScalar::read_at(&channels[0], offset),
                g: BridgeScalar::read_at(&channels[1], offset),
                b: BridgeScalar::read_at(&channels[2], offset),
                a: BridgeScalar::read_at(&channels[3], offset),
            }),
            EffectValue::Bool(value) => BridgeEffectValue::Bool(*value),
            EffectValue::Choice(index) => BridgeEffectValue::Choice(*index),
            EffectValue::Seed(seed) => BridgeEffectValue::Seed(*seed),
            EffectValue::File(file) => BridgeEffectValue::File(BridgeFileParam {
                paths: file.paths.clone(),
                index: BridgeScalar::read_at(&file.index, offset),
            }),
            EffectValue::Layer(layer) => BridgeEffectValue::Layer(*layer),
            EffectValue::MaskPath(mask) => BridgeEffectValue::MaskPath(*mask),
            EffectValue::Curve(points) => {
                BridgeEffectValue::Curve(points.iter().map(|xy| xy.to_vec()).collect())
            }
        }
    }

    /// Overwrite `target` with this value, held inside `bounds`.
    ///
    /// A parameter's *kind* is declared by the effect's schema and is not the
    /// panel's to change, so a mismatched pair is refused rather than replacing
    /// the value: writing a number to a colour would leave an instance the
    /// effect's own resolver cannot read, and it would be undoable but not
    /// obviously wrong on screen.
    ///
    /// `bounds` is the parameter's declared hard range ([`hard_bounds`]), and it
    /// is applied here rather than trusted to the caller — see [`clamp_animation`].
    #[frb(ignore)]
    fn write_at(
        self,
        target: &mut EffectValue,
        offset: Rational,
        bounds: (Option<f64>, Option<f64>),
    ) -> Result<(), BridgeError> {
        match (self, target) {
            (BridgeEffectValue::Float(scalar), EffectValue::Float(property)) => {
                property.animation = clamp_animation(scalar.animation_at(offset)?, bounds);
                Ok(())
            }
            (BridgeEffectValue::Point(point), EffectValue::Point(x, y)) => {
                let (ax, ay) = (point.x.animation_at(offset)?, point.y.animation_at(offset)?);
                x.animation = clamp_animation(ax, bounds);
                y.animation = clamp_animation(ay, bounds);
                Ok(())
            }
            (BridgeEffectValue::Colour(colour), EffectValue::Colour(channels)) => {
                let animations = [
                    colour.r.animation_at(offset)?,
                    colour.g.animation_at(offset)?,
                    colour.b.animation_at(offset)?,
                    colour.a.animation_at(offset)?,
                ];
                for (property, animation) in channels.iter_mut().zip(animations) {
                    property.animation = clamp_animation(animation, bounds);
                }
                Ok(())
            }
            (BridgeEffectValue::Bool(value), EffectValue::Bool(target)) => {
                *target = value;
                Ok(())
            }
            (BridgeEffectValue::Choice(index), EffectValue::Choice(target)) => {
                *target = index;
                Ok(())
            }
            (BridgeEffectValue::Seed(seed), EffectValue::Seed(target)) => {
                *target = seed;
                Ok(())
            }
            (BridgeEffectValue::File(file), EffectValue::File(target)) => {
                let animation = file.index.animation_at(offset)?;
                *target = FileParam {
                    paths: file.paths,
                    index: Property {
                        animation,
                        // The index's own forward-compatibility fields survive a
                        // path change, as they do for every other property here.
                        extra: std::mem::take(&mut target.index.extra),
                    },
                };
                Ok(())
            }
            (BridgeEffectValue::Layer(layer), EffectValue::Layer(target)) => {
                *target = layer;
                Ok(())
            }
            (BridgeEffectValue::MaskPath(mask), EffectValue::MaskPath(target)) => {
                *target = mask;
                Ok(())
            }
            (BridgeEffectValue::Curve(points), EffectValue::Curve(target)) => {
                // A pair that is not a pair is dropped rather than refused:
                // the value is straightened on read anyway, and a row short
                // of a coordinate is a malformed message, not a kind
                // mismatch.
                *target = points
                    .iter()
                    .filter(|xy| xy.len() >= 2)
                    .map(|xy| [xy[0], xy[1]])
                    .collect();
                Ok(())
            }
            _ => Err(BridgeError::ParamKindMismatch),
        }
    }
}

/// The two numbers a parameter's stored value may never leave, or `None` either
/// side where it is unbounded there (docs/08 §1.2).
///
/// **A slider's travel is not a bound.** Typing past it is allowed, and that is
/// the whole difference between a soft range and a hard one — so only the `hard`
/// pair, a closed [`ParamKind::Slider`]'s range (which is both its travel and
/// its bound, K-414) and a colour's per-channel range answer here. Every other
/// kind is unbounded by declaration, an `Angle` deliberately so: it winds
/// through full turns rather than stopping at 360.
///
/// [`ParamKind::Slider`]: lumit_core::fx::ParamKind::Slider
#[frb(ignore)]
fn hard_bounds(kind: &lumit_core::fx::ParamKind) -> (Option<f64>, Option<f64>) {
    use lumit_core::fx::ParamKind;
    match *kind {
        ParamKind::Float { hard, .. } => hard,
        ParamKind::Int { hard, .. } => (hard.0.map(|v| v as f64), hard.1.map(|v| v as f64)),
        // A closed range and a colour's channel range are hard by definition.
        ParamKind::Slider { range, .. } | ParamKind::Colour { range, .. } => {
            (Some(range.0), Some(range.1))
        }
        _ => (None, None),
    }
}

/// `animation` with every value it can take pulled inside `bounds`.
///
/// **Clamping an animation means clamping its keys**, not only the number the
/// playhead happens to be over — the same rule the mask scalars' own
/// `clamped_property` follows. A radius keyed to −40 three seconds away is just
/// as far out of range as one set to −40 now, and it would arrive the moment the
/// playhead did.
///
/// An expression cannot be clamped here at all: it is a string until it runs, so
/// it passes through and the resolve step's own reads keep their clamps. A pair
/// that is not a range — either end NaN, or a low above its high — is left alone
/// rather than trusted, because `f64::clamp` panics on both and an engine crate
/// does not panic (docs/14 §4).
#[frb(ignore)]
fn clamp_animation(animation: Animation, bounds: (Option<f64>, Option<f64>)) -> Animation {
    let lo = bounds.0.unwrap_or(f64::NEG_INFINITY);
    let hi = bounds.1.unwrap_or(f64::INFINITY);
    if !(lo <= hi) || (lo.is_infinite() && hi.is_infinite()) {
        return animation;
    }
    match animation {
        Animation::Static(value) => Animation::Static(value.clamp(lo, hi)),
        Animation::Keyframed(keys) => Animation::Keyframed(
            keys.into_iter()
                .map(|key| Keyframe {
                    value: key.value.clamp(lo, hi),
                    ..key
                })
                .collect(),
        ),
        expression => expression,
    }
}

/// One effect in a layer's stack, as the Effect controls panel holds it.
///
/// A **detached copy**, not a live handle: reading the stack clones it out of the
/// document, and [`Self::set_value`] edits that clone without committing
/// anything. That is what makes a drag cheap — Dart stages a value, renders it
/// through `CompositionReference::render_frame_with_preview`, and touches the
/// document, the undo history and the disk exactly once, on release, through
/// `LayerReference::set_effects` (docs/17 ABI v11/v12; GUIDE "Staging versus
/// committing").
#[frb(opaque)]
pub struct BridgeEffectInstance {
    // `pub(crate)` since K-713: the Roto brush's stroke accessors live in
    // `api::roto`, next to the rest of that effect's surface, and stage on the
    // same copy every other row stages on.
    pub(crate) effect: EffectInstance,
    /// Where the owning layer's own zero sits on the composition's clock, so a
    /// handle read out of a layer still knows how to speak comp time about its
    /// keyframes (K-213). Carried rather than looked up: the handle is a
    /// snapshot, and the layer it came from is the only place this is known.
    offset: Rational,
}

/// One parameter's current value, as [`BridgeEffectInstance::get_info`]
/// carries it.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeParamValue {
    pub id: String,
    pub value: BridgeEffectValue,
}

/// Everything a panel draws for one effect instance, in one crossing (K-183):
/// its id, match name, bypass state, and every parameter's current value. The
/// instance is an opaque handle, so `id()`/`name()`/`get_value()` each cross
/// the bridge — a card that read them one at a time cost a call per field per
/// parameter per rebuild.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeEffectInstanceInfo {
    pub id: Uuid,
    pub name: String,
    /// The user's own name for the instance (K-321), or `None` to show the
    /// effect's label. `name` stays the `match_name` either way — it is the
    /// schema key, not a display string.
    pub custom_name: Option<String>,
    pub enabled: bool,
    pub values: Vec<BridgeParamValue>,
    /// The stems of the vector pairs this instance has chained (K-443), sorted.
    /// Empty is "every pair unlinked", which is what every older project means.
    ///
    /// In the read model rather than asked per pair, for the reason every other
    /// field here is: a chain glyph is drawn per point row per rebuild, and a
    /// call apiece is exactly the hover-hot traffic the budget test forbids.
    pub linked_pairs: Vec<String>,
    /// Why this instance is not doing its own work, if it is not (docs/12 §1,
    /// §2.3) — one of [`BADGE_REASONS`], or `None` for the ordinary case.
    ///
    /// A **key**, not a sentence: the panel draws the calm badge in the user's
    /// own language (K-303). Four things it can say — the plugin failed, the
    /// plugin is switched off, the plugin is not installed on this machine, or
    /// this build has never heard of the effect at all. The last two are the
    /// placeholder docs/12 §1 requires: the instance is kept, values and all,
    /// so saving cannot lose it.
    pub badge_reason: Option<String>,
    /// The engine's or the plugin's own words about the failure, where there
    /// are any — shown beneath the badge, verbatim and untranslated, because it
    /// is somebody else's sentence about somebody else's code.
    pub badge_detail: Option<String>,
    /// The rows **this instance** has beyond its effect's schema
    /// (docs/impl/custom-shader.md §1.5) — the Custom shader's own uniforms,
    /// read off the source it holds, and empty for every other effect.
    ///
    /// Here rather than asked for per card, and for the reason every other
    /// field of this struct is: the panel draws its rows on every rebuild, and
    /// a call apiece is exactly the traffic `bridge_call_budget_test` forbids.
    /// The declared half stays memoised Dart-side under the match name
    /// ([`list_parameters`]); this is the half that cannot be, because it is a
    /// fact about the instance. The two concatenated are what
    /// [`BridgeEffectInstance::list_parameters`] answers in one piece.
    pub derived_params: Vec<BridgeParamInfo>,
}

/// Every value [`BridgeEffectInstanceInfo::badge_reason`] can take.
///
/// A closed list, spelled once, so the frontend's table of sentences can be
/// held against it — `test/l10n/engine_labels_test.dart` reads this very
/// declaration and fails if a reason has no translation, exactly as it does for
/// the import report's reasons and the colour config's refusals.
pub const BADGE_REASONS: &[&str] = &[
    "plugin_failed",
    "plugin_disabled",
    "plugin_missing",
    "unknown_effect",
    "shader_failed",
];

/// Why this instance is not doing its own work, if it is not.
///
/// The order is the order of certainty. A failure recorded against *this
/// instance* by the last render is the most specific thing anyone knows, so it
/// is read first; only then does the question become the more general "is there
/// anything in the catalogue by that name at all".
#[frb(ignore)]
fn badge_of(effect: &EffectInstance) -> (Option<String>, Option<String>) {
    let name = effect.effect.match_name.as_str();
    // A switched-off **audio** plugin, first: the answer is the session's own
    // list rather than a recorded failure, and it outranks any stale note a
    // bake filed before the switch was flicked (AP5). The identifier is the
    // match name shorn of its prefix — exactly what `set_plugin_enabled`
    // wrote into the list.
    if let Some(identifier) = name
        .strip_prefix(lumit_core::fx::CLAP_MATCH_PREFIX)
        .or_else(|| name.strip_prefix(lumit_core::fx::VST3_MATCH_PREFIX))
    {
        if lumit_aplug::session_disabled()
            .lock()
            .is_ok_and(|list| list.contains(identifier))
        {
            return (Some("plugin_disabled".to_owned()), None);
        }
    }
    if let Some(why) = lumit_render::gpufx::ofx::error_of(effect.id) {
        // A switched-off plugin files the reason key itself, so the badge says
        // "switched off" rather than reporting it as a failure.
        return if why == lumit_ofx::discover::DISABLED_REASON {
            (Some("plugin_disabled".to_owned()), None)
        } else {
            (Some("plugin_failed".to_owned()), Some(why))
        };
    }
    // A Custom shader that does not compile is the one built-in that can wear a
    // badge (docs/impl/custom-shader.md §2.2). It is not a fault in Lumit and it
    // is not modal: the effect renders as identity, the values below are still
    // live and still saved, and the compiler's own sentence goes underneath.
    if let Some(why) = shader_error(effect) {
        return (Some("shader_failed".to_owned()), Some(why));
    }
    if lumit_core::fx::BUILTIN_DEFS.get(name).is_some() {
        return (None, None);
    }
    // Nothing answers to that name. An `ofx:` or `clap:` one is a plugin this
    // machine has not got — uninstalled, switched off before the scan, or a
    // project made on somebody else's machine; anything else is an effect from
    // a newer Lumit. Both are inert placeholders and neither is an error
    // (docs/12 §1, docs/08 §5) — an audio plugin's inertness being that its
    // link is left out of the chain and the sound goes through dry (K-700).
    if matches!(
        effect.effect.namespace,
        lumit_core::model::EffectNamespace::Ofx | lumit_core::model::EffectNamespace::Clap
    ) {
        (Some("plugin_missing".to_owned()), None)
    } else {
        (Some("unknown_effect".to_owned()), None)
    }
}

/// What became of the shader one Custom shader instance holds
/// (docs/impl/custom-shader.md §2.2).
///
/// # In plain terms
///
/// Somebody typed a program. Either it works, or the compiler has something to
/// say about it — and the words are the compiler's own, untranslated, because it
/// is somebody else's sentence about somebody else's code (K-303). The line
/// numbers in it have been moved back onto the text the user is looking at, so
/// "line 3" means the third line they typed rather than the third line of the
/// wrapper Lumit put around it.
///
/// A shader that is merely *unfinished* is not a failure: `error` is `None` for
/// an instance with no source, which renders as a passthrough and wears no badge.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeShaderStatus {
    /// The refusal or the compiler's message, or `None` when the shader draws.
    pub error: Option<String>,
    /// One calm sentence per annotation that would not parse. A typo in a doc
    /// comment costs that row and not the other eight (§2.2), so these are
    /// notes beside working rows rather than an error instead of them.
    pub notes: Vec<String>,
}

// --- The inner shader graph (docs/impl/custom-shader.md §4, K-642, CS4) -----

/// What a wire in the inner graph carries, for the canvas to colour sockets
/// by. Widths one to three are numbers, a vec4 is a colour, and a picture is
/// the identity of a texture only the Sample box can read. No colour crosses
/// the bridge; the frontend maps type to theme token, exactly as the layer
/// graph's `BridgePortType` does.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeShaderTy {
    F32,
    Vec2,
    Vec3,
    Vec4,
    Picture,
}

#[frb(ignore)]
fn bridge_shader_ty(ty: lumit_core::fx::shader::graph::GraphTy) -> BridgeShaderTy {
    use lumit_core::fx::shader::graph::GraphTy;
    match ty {
        GraphTy::F32 => BridgeShaderTy::F32,
        GraphTy::Vec2 => BridgeShaderTy::Vec2,
        GraphTy::Vec3 => BridgeShaderTy::Vec3,
        GraphTy::Vec4 => BridgeShaderTy::Vec4,
        GraphTy::Picture => BridgeShaderTy::Picture,
    }
}

/// One port as the inner canvas draws it: its name (an id the frontend
/// translates — the engine sends no English here) and its nominal type.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeShaderPort {
    pub id: String,
    pub ty: BridgeShaderTy,
}

/// One box of the inner graph, resolved for drawing. `label` is set only for
/// a Parameter box — the user's own word for their own control, shown as-is
/// and never translated; every other box is named by the frontend from `kind`.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeShaderGraphNode {
    pub id: u32,
    pub kind: String,
    pub label: Option<String>,
    pub inputs: Vec<BridgeShaderPort>,
    pub outputs: Vec<BridgeShaderPort>,
}

/// A stored inner graph as the canvas draws it, plus the one sentence that
/// says whether it will compile. The canvas draws a broken graph exactly as it
/// draws a working one — being broken is a state to work in, and the badge is
/// the messenger (§2.2) — so the boxes and the error travel together.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeShaderGraphView {
    pub nodes: Vec<BridgeShaderGraphNode>,
    pub error: Option<String>,
}

/// One entry of the inner graph's add-search: a kind the frontend translates,
/// filed under its family.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeShaderNodeKind {
    pub kind: String,
    pub category: String,
}

/// The v1 node vocabulary (custom-shader.md §4.3), in listing order. Static:
/// asked once and memoised Dart-side like every other catalogue.
#[frb(sync)]
pub fn list_shader_nodes() -> Vec<BridgeShaderNodeKind> {
    lumit_core::fx::shader::graph::NODE_SPECS
        .iter()
        .map(|spec| BridgeShaderNodeKind {
            kind: spec.kind.to_owned(),
            category: spec.category.to_owned(),
        })
        .collect()
}

/// Resolve one graph document for drawing: every box's ports and types, and
/// whether the whole thing compiles.
///
/// A pure question — nothing is staged and no document moves — which is also
/// how the canvas refuses a drop: build the candidate graph, ask, and decline
/// visually when the answer names a mismatch or a cycle. The engine stays the
/// single validator; the panel never learns the type rules (K-183's spirit:
/// display and forward, decide in Rust). Called on gestures and reloads, never
/// in a rebuild.
#[frb(sync)]
pub fn shader_graph_view(graph: String) -> BridgeShaderGraphView {
    use lumit_core::fx::shader::graph::{ports_of, ShaderGraph};
    let parsed: Result<ShaderGraph, _> = serde_json::from_str(&graph);
    let Ok(g) = parsed else {
        return BridgeShaderGraphView {
            nodes: Vec::new(),
            error: Some("the stored graph does not parse".to_owned()),
        };
    };
    let error = lumit_core::fx::shader::compile::compile(&g)
        .err()
        .map(|why| why.to_string());
    let nodes = g
        .nodes
        .iter()
        .map(|node| {
            let (inputs, outputs) = ports_of(node);
            let port = |(id, ty): &(&'static str, lumit_core::fx::shader::graph::GraphTy)| {
                BridgeShaderPort {
                    id: (*id).to_owned(),
                    ty: bridge_shader_ty(*ty),
                }
            };
            BridgeShaderGraphNode {
                id: node.id,
                kind: node.kind.clone(),
                label: (node.kind == "param").then(|| {
                    node.settings
                        .get("label")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .map_or_else(
                            || {
                                node.settings
                                    .get("id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("?")
                                    .to_owned()
                            },
                            str::to_owned,
                        )
                }),
                inputs: inputs.iter().map(port).collect(),
                outputs: outputs.iter().map(port).collect(),
            }
        })
        .collect();
    BridgeShaderGraphView { nodes, error }
}

/// Why this instance's shader will not draw, in the compiler's own words, or
/// `None` when it will (docs/impl/custom-shader.md §2.1, §2.2).
///
/// Two failures, and the seam reports them as one sentence because the person
/// reading it is looking at one text box:
///
/// - a **refusal** — the source binds its own group, shadows a host name,
///   declares no `shade`, or declares a parameter the grammar cannot carry.
///   Read straight off the §1.4 line reader, so it answers on a machine with no
///   graphics card.
/// - a **compile error** — naga's own message about the assembled module, with
///   its line numbers moved back onto the user's own text, which is the only
///   numbering the person typing has.
///
/// A fresh instance with no source is **not** a failure: an effect the user has
/// not filled in yet is a passthrough, not a fault (K-111), and it wears no
/// badge.
#[frb(ignore)]
fn shader_error(effect: &EffectInstance) -> Option<String> {
    use lumit_core::fx::effects::custom_shader as cs;
    // The graph is master when it is there (§4.1, CS4): the sentence the badge
    // wears is about what will actually render, which is the graph's compile —
    // never the cached text beside it.
    let source: &str = match cs::graph_of(effect) {
        Some(graph) => match lumit_core::fx::shader::compile::source_for(graph) {
            Ok(text) => text,
            Err(why) => return Some(why.to_owned()),
        },
        None => cs::source_of(effect)?,
    };
    if source.trim().is_empty() {
        return None;
    }
    let program = match lumit_core::fx::shader::program_for(source) {
        Ok(program) => program,
        Err(refusal) => return Some(refusal.to_string()),
    };
    validated(program).clone()
}

/// Whether an assembled module compiles, answered once per distinct source.
///
/// The read model rebuilds on every document change and asks this per Custom
/// shader instance; a naga parse and validation is milliseconds, which is a
/// frame's whole budget, so a stack of two instances sharing a source would pay
/// it twice per refresh for an answer that cannot have moved. Keyed by the
/// source hash, exactly as the pipeline cache is (§3.1).
///
/// ponytail: never evicts, one small entry per distinct source this session —
/// the same ceiling `program_for`'s own parse cache carries, and it is bounded
/// by the same thing (how many shaders one sitting types). Bound the two
/// together if a heap profile ever names either.
#[frb(ignore)]
fn validated(program: &lumit_core::fx::shader::ShaderProgram) -> &'static Option<String> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static SEEN: OnceLock<Mutex<HashMap<u64, &'static Option<String>>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(map) = seen.lock() {
        if let Some(hit) = map.get(&program.source_hash) {
            return hit;
        }
    }
    let answer: &'static Option<String> = Box::leak(Box::new(
        lumit_render::validate_shader(&program.assembled)
            .err()
            .map(|message| program.remap_error(&message)),
    ));
    if let Ok(mut map) = seen.lock() {
        map.insert(program.source_hash, answer);
    }
    answer
}

/// Give a **staged or displayed** copy of an instance the derived rows its own
/// source declares (docs/impl/custom-shader.md §1.5), at their defaults.
///
/// The sibling of `backfill_builtin_params`, and deliberately not part of it:
/// that one is also the project reader's forward migration, and a derived row
/// written into the document at load would be the parameter set changing with
/// nobody's edit behind it — the one thing §1.5 forbids. Here it reaches only
/// the two copies the bridge makes: the clone `read_instance_info` reads, so a
/// derived row draws its value rather than a dash, and the staged copy a handle
/// holds, so `set_value` can write one. A staged copy reaches the document only
/// alongside an edit the user actually made, which is what makes adopting a row
/// the user's act rather than the panel's.
#[frb(ignore)]
fn fill_derived(effect: &mut EffectInstance) {
    let Some(def) = lumit_core::fx::BUILTIN_DEFS.get(effect.effect.match_name.as_str()) else {
        return;
    };
    // `derived` answers `&'static [ParamSchema]` — a session-lived parse cache,
    // not a borrow of the instance — so the read is over before the write.
    for param in def.derived(effect) {
        let Some(value) = lumit_core::fx::default_param_value(&param.kind) else {
            continue; // a button has nothing to fill (K-417)
        };
        if !effect.params.iter().any(|have| have.id == param.id) {
            effect.params.push(EffectParam {
                id: param.id.to_owned(),
                value,
                extra: serde_json::Map::new(),
            });
        }
    }
}

/// The rows an instance's own source declares, as the panel reads them.
#[frb(ignore)]
fn derived_params_of(effect: &EffectInstance) -> Vec<BridgeParamInfo> {
    lumit_core::fx::BUILTIN_DEFS
        .get(effect.effect.match_name.as_str())
        .map(|def| def.derived(effect).iter().map(bridge_param).collect())
        .unwrap_or_default()
}

/// Build one instance's [`BridgeEffectInstanceInfo`] — the shared body of
/// [`BridgeEffectInstance::get_info`] and the comp read model (K-184).
#[frb(ignore)]
pub(crate) fn read_instance_info(
    effect: &EffectInstance,
    offset: Rational,
) -> BridgeEffectInstanceInfo {
    // Report every parameter the schema declares, not only the ones this
    // instance happens to carry — the same filling [`BridgeEffectInstance::new`]
    // does, for the other way in: the comp read model (K-184) hands raw
    // `EffectInstance`s straight here without a handle. Without it a parameter
    // added after the instance was saved draws a blank row.
    //
    // Filled on a clone, because reading may not edit the document. The value
    // lands for real when the user changes it, through the staged copy.
    let mut filled = effect.clone();
    lumit_core::fx::backfill_builtin_params(std::slice::from_mut(&mut filled));
    fill_derived(&mut filled);
    let effect = &filled;
    let (badge_reason, badge_detail) = badge_of(effect);
    BridgeEffectInstanceInfo {
        id: effect.id,
        name: effect.effect.match_name.clone(),
        custom_name: effect.custom_name.clone(),
        enabled: effect.enabled,
        values: effect
            .params
            .iter()
            .map(|p| BridgeParamValue {
                id: p.id.to_string(),
                value: BridgeEffectValue::read_at(&p.value, offset),
            })
            .collect(),
        linked_pairs: effect.linked_pairs.clone(),
        badge_reason,
        badge_detail,
        derived_params: derived_params_of(effect),
    }
}

impl BridgeEffectInstance {
    /// Rust-side only (`frb(ignore)`): a handle is made *from a layer*, which is
    /// where the keyframe offset comes from (K-213). It was exposed to Dart and
    /// never called from there — an instance can only be got from the layer
    /// that owns it — and a Dart constructor with no layer would have no
    /// honest offset to take.
    #[frb(ignore)]
    pub fn new(effect: EffectInstance, offset: Rational) -> BridgeEffectInstance {
        // Give the staged copy every parameter its schema declares before the
        // frontend touches it. `instantiate` copies the schema at the
        // moment an effect is created and nothing has ever brought an older
        // instance up to a schema that grew afterwards, so a parameter added
        // later read as absent and refused writes — the row drew blank and the
        // control was dead. Filling here rather than in each accessor is what
        // makes that true for `get_info`, `get_value`, `get_parameters` and
        // `set_value` alike, since all four read this one field.
        //
        // This is a staged copy: `LayerReference::set_effects` is what commits,
        // so a filled parameter reaches the document only alongside an edit the
        // user actually made. An effect with no built-in schema (OFX, a
        // placeholder) is left exactly as it is.
        let mut effect = effect;
        lumit_core::fx::backfill_builtin_params(std::slice::from_mut(&mut effect));
        // And the rows this instance's *own* source declares (§1.5), so a
        // derived control is as live as a declared one: `get_value` answers it,
        // `set_value` writes it, and the commit is the ordinary one.
        fill_derived(&mut effect);
        BridgeEffectInstance { effect, offset }
    }

    /// One read for everything a card draws — see [`BridgeEffectInstanceInfo`].
    #[frb(sync)]
    pub fn get_info(&self) -> BridgeEffectInstanceInfo {
        read_instance_info(&self.effect, self.offset)
    }

    /// This instance's own id — what the stack ops on
    /// [`crate::api::layer::LayerReference`] address it by.
    #[frb(sync)]
    pub fn id(&self) -> Uuid {
        self.effect.id
    }

    #[frb(sync)]
    pub fn name(&self) -> String {
        self.effect.effect.match_name.clone()
    }

    /// False when the effect is individually bypassed (docs/08 §1.5) — the state
    /// of the checkbox in its title bar.
    #[frb(sync)]
    pub fn enabled(&self) -> bool {
        self.effect.enabled
    }

    /// Stage the user's own name for this instance (K-321) — an empty or
    /// whitespace name clears it back to the effect's label. Staging only, like
    /// `set_value`: `LayerReference::set_effects` is the commit.
    #[frb(sync)]
    pub fn set_custom_name(&mut self, name: String) {
        let trimmed = name.trim();
        self.effect.custom_name = (!trimmed.is_empty()).then(|| trimmed.to_string());
    }

    /// Bypass or enable this instance on the **staged** copy, like
    /// `set_custom_name`. The commit is `LayerReference::set_graph` for a
    /// driver node, whose `B` badge this is; a stack effect has its own
    /// committing op (`LayerReference::set_effect_enabled`) and does not need
    /// this.
    #[frb(sync)]
    pub fn set_enabled(&mut self, enabled: bool) {
        self.effect.enabled = enabled;
    }

    /// Whether the vector pair keyed by `stem` is chained (K-443). A stem this
    /// effect has no pair for is unlinked, never an error.
    #[frb(sync)]
    pub fn pair_linked(&self, stem: String) -> bool {
        self.effect.pair_linked(&stem)
    }

    /// Chain or unchain the vector pair keyed by `stem`, on the **staged**
    /// copy — `LayerReference::set_effects` is the commit, exactly as
    /// `set_custom_name` and `set_value` are, so a toggle is one op and one
    /// undo step like every other effect-stack edit.
    ///
    /// Answers whether anything moved, so a caller can skip a commit that
    /// would undo to itself. The proportional drag a chained pair takes is
    /// deliberately **not** here: it is UI-time arithmetic while a gesture is
    /// live, and the document's business is only which pairs are tied.
    #[frb(sync)]
    pub fn set_pair_linked(&mut self, stem: String, linked: bool) -> bool {
        self.effect.set_pair_linked(&stem, linked)
    }

    /// Every row **this instance** draws, in order: the rows its effect
    /// declares, then the rows its own state derives (docs/impl/custom-shader.md
    /// §1.5, docs/impl/effect-registry.md §4).
    ///
    /// The owed half of [`list_parameters`], which is keyed by match name and so
    /// can only ever answer the first half. For every effect but the Custom
    /// shader the two lists are the same list; for a Custom shader the tail is
    /// the uniforms the source it holds declares, and nothing downstream can
    /// tell a derived row from a declared one — same widgets, same keyframes,
    /// same expressions.
    ///
    /// **Not the panel's per-rebuild road.** The panel reads the declared half
    /// from its Dart-side memo and the derived half off
    /// [`BridgeEffectInstanceInfo::derived_params`], which it already holds; this
    /// is the one-piece answer for a caller that has a handle and no read model.
    #[frb(sync)]
    pub fn list_parameters(&self) -> Vec<BridgeParamInfo> {
        let Some(def) = lumit_core::fx::BUILTIN_DEFS.get(self.effect.effect.match_name.as_str())
        else {
            return Vec::new();
        };
        def.schema()
            .params
            .iter()
            .chain(def.derived(&self.effect))
            .map(bridge_param)
            .collect()
    }

    /// The WGSL text this instance holds, or `None` when it holds none
    /// (docs/impl/custom-shader.md §1.2).
    ///
    /// The source is **instance state, not a parameter**: `Value` is `Copy` and
    /// hashed field by field, a kilobyte of text is neither, and two shader
    /// sources cannot be interpolated. So it does not ride
    /// [`BridgeEffectInstanceInfo::values`] with the numbers; it is read here,
    /// on the gesture that opens an editor, rather than per rebuild.
    #[frb(sync)]
    pub fn shader_source(&self) -> Option<String> {
        lumit_core::fx::effects::custom_shader::source_of(&self.effect).map(str::to_owned)
    }

    /// Where the text came from, when it was loaded from a file — remembered for
    /// reload and **never read at render**: a project must be one file that opens
    /// on another machine.
    #[frb(sync)]
    pub fn shader_origin(&self) -> Option<String> {
        self.effect
            .extra
            .get(lumit_core::fx::effects::custom_shader::EXTRA_KEY)?
            .get("origin")?
            .as_str()
            .map(str::to_owned)
    }

    /// Stage this instance's shader source on the **staged** copy, exactly as
    /// `set_custom_name` and `set_value` do: `LayerReference::set_effects` is the
    /// commit, so a shader edit is one `SetLayerEffects` and one undo step like
    /// every other effect-stack edit.
    ///
    /// `origin` is the file the text was read from, or `None` for text the user
    /// typed — which is the honest answer once they have typed it, since it no
    /// longer says what that file says. An empty `source` clears the block back
    /// to a fresh instance: a passthrough with no badge (K-111).
    ///
    /// The rows the new source declares are **offered**, not adopted: the
    /// document keeps the values it has, an id that has gone keeps its row and
    /// its expression, and an id that is new draws at its default until the user
    /// touches it (§1.5).
    #[frb(sync)]
    pub fn set_shader_source(&mut self, source: String, origin: Option<String>) {
        use lumit_core::fx::effects::custom_shader::EXTRA_KEY;
        if source.trim().is_empty() {
            self.effect.extra.remove(EXTRA_KEY);
            return;
        }
        let mut block = serde_json::Map::new();
        // Written from the first commit and read by nothing yet (§5): the day
        // `glsl-in` is turned on, an older project says which language its text
        // is in rather than being guessed at.
        block.insert("language".to_owned(), json!("wgsl"));
        block.insert("source".to_owned(), json!(source));
        if let Some(path) = origin {
            block.insert("origin".to_owned(), json!(path));
        }
        // Anything else already under the key — §4's `graph`, a field a newer
        // Lumit wrote (K-065) — is kept: this call owns the text, not the block.
        if let Some(serde_json::Value::Object(had)) = self.effect.extra.get(EXTRA_KEY) {
            for (key, value) in had {
                block.entry(key.clone()).or_insert_with(|| value.clone());
            }
        }
        self.effect
            .extra
            .insert(EXTRA_KEY.to_owned(), serde_json::Value::Object(block));
    }

    /// Whether this instance's shader will draw, and why not when it will not
    /// (docs/impl/custom-shader.md §2.1, §2.2).
    ///
    /// The per-instance answer the badge wears and the editor anchors its
    /// message to. `error` is `None` for a shader that compiles **and** for an
    /// instance with no source at all, because an effect the user has not filled
    /// in yet is a passthrough rather than a failure.
    #[frb(sync)]
    pub fn shader_status(&self) -> BridgeShaderStatus {
        BridgeShaderStatus {
            error: shader_error(&self.effect),
            // `program_of` is graph-aware (§4.1), so the notes are about the
            // text that will actually render whichever view authored it.
            notes: lumit_core::fx::effects::custom_shader::program_of(&self.effect)
                .map(|program| program.notes.clone())
                .unwrap_or_default(),
        }
    }

    /// The stored inner graph, as its JSON text, or `None` for a hand-written
    /// (or empty) shader (docs/impl/custom-shader.md §4, CS4).
    ///
    /// Read on the gesture that enters the graph, never per rebuild — the same
    /// contract `shader_source` has, and for the same reason.
    #[frb(sync)]
    pub fn shader_graph(&self) -> Option<String> {
        lumit_core::fx::effects::custom_shader::graph_of(&self.effect)
            .map(std::string::ToString::to_string)
    }

    /// Stage a whole inner graph on this instance (§4.1, CS4), exactly as
    /// `set_shader_source` stages text: `LayerReference::set_effects` is the
    /// commit, so a graph edit is one `SetLayerEffects` and one undo step.
    ///
    /// The graph becomes master. When it compiles, the compiled WGSL is written
    /// into `source` in the same staging — the cached text §4.1 keeps so a
    /// build that cannot compile the graph can still render it — and `origin`
    /// is dropped, the text no longer being any file's. When it does not
    /// compile, the graph is stored anyway with the last text left standing:
    /// being broken is a normal state to pass through, and the badge says so.
    ///
    /// # Errors
    /// [`BridgeError::InvalidShaderGraph`] when the text is not a graph
    /// document at all — a caller bug, not a user state.
    #[frb(sync)]
    pub fn set_shader_graph(&mut self, graph: String) -> Result<(), BridgeError> {
        use lumit_core::fx::effects::custom_shader::EXTRA_KEY;
        let parsed: serde_json::Value =
            serde_json::from_str(&graph).map_err(|_| BridgeError::InvalidShaderGraph)?;
        if serde_json::from_value::<lumit_core::fx::shader::graph::ShaderGraph>(parsed.clone())
            .is_err()
        {
            return Err(BridgeError::InvalidShaderGraph);
        }
        let compiled = lumit_core::fx::shader::compile::source_for(&parsed).ok();
        let mut block = match self.effect.extra.remove(EXTRA_KEY) {
            Some(serde_json::Value::Object(had)) => had,
            _ => serde_json::Map::new(),
        };
        block.insert("language".to_owned(), json!("wgsl"));
        block.insert("graph".to_owned(), parsed);
        if let Some(text) = compiled {
            block.insert("source".to_owned(), json!(text));
            block.remove("origin");
        }
        self.effect
            .extra
            .insert(EXTRA_KEY.to_owned(), serde_json::Value::Object(block));
        Ok(())
    }

    /// Detach the inner graph (§4.1): keep the compiled text, drop the `graph`
    /// key, and leave an ordinary hand-written shader behind. One staged edit,
    /// committed with the stack, so it is one undo step — and it is not
    /// reversible by another button, which is the honest shape: the graph is
    /// gone because the user said so.
    #[frb(sync)]
    pub fn detach_shader_graph(&mut self) {
        use lumit_core::fx::effects::custom_shader::EXTRA_KEY;
        let Some(serde_json::Value::Object(block)) = self.effect.extra.get_mut(EXTRA_KEY) else {
            return;
        };
        // The kept text is compiled afresh rather than trusted from the cache
        // field (§4.1 — the cached text is a convenience, not an authority).
        let compiled = block
            .get("graph")
            .and_then(|g| lumit_core::fx::shader::compile::source_for(g).ok());
        if let Some(text) = compiled {
            block.insert("source".to_owned(), json!(text));
        }
        block.remove("graph");
    }

    #[frb(ignore)]
    pub fn get_effects(&self) -> EffectInstance {
        self.effect.clone()
    }

    #[frb(sync)]
    pub fn serialize(&self) -> String {
        let serialized = json!(&self.effect);
        serialized.to_string()
    }

    #[frb(sync)]
    pub fn get_parameters(&self) -> Vec<String> {
        self.effect
            .params
            .iter()
            .map(|f| f.id.to_string())
            .collect()
    }

    /// A parameter's value, whatever kind it is. An unknown `id` is an error;
    /// every parameter an instance actually carries is expressible, so there is
    /// no "cannot represent this one" answer any more.
    #[frb(sync)]
    pub fn get_value(&self, id: String) -> Result<BridgeEffectValue, BridgeError> {
        Ok(BridgeEffectValue::read_at(
            &self.param(&id)?.value,
            self.offset,
        ))
    }

    /// Overwrite a parameter on this staged copy. Nothing is committed — see the
    /// type's own documentation; `LayerReference::set_effects` is the commit.
    ///
    /// Refused when `value` is of a different kind from the parameter, so a
    /// control can never quietly change what a parameter *is*.
    ///
    /// **The hard range is enforced here, not in the panel** (docs/08 §1.2,
    /// K-620). Every way a number reaches an effect parameter — typed, scrubbed,
    /// dragged in the graph editor, picked off the Viewer, wired from a node,
    /// pasted, loaded from a preset — passes through this one call, and both the
    /// preview and the commit stage through it, so clamping once here is what
    /// makes the picture a scrub shows and the value it lands on the same number.
    /// A control that also clamps its own reading is agreeing with the engine,
    /// not deciding for it; a control that forgets to can no longer render a
    /// value the parameter does not have.
    #[frb(sync)]
    pub fn set_value(&mut self, id: String, value: BridgeEffectValue) -> Result<(), BridgeError> {
        // An effect this build does not know has no schema to consult, so its
        // parameters stay unbounded rather than refused: a project carrying one
        // still opens and still edits, exactly as `list_parameters` allows.
        let bounds = lumit_core::fx::schema(&self.effect.effect.match_name)
            .and_then(|schema| schema.params.iter().find(|p| p.id == id))
            .map_or((None, None), |p| hard_bounds(&p.kind));

        // Every parameter the schema declares is already present: `new` fills
        // the staged copy. A name that is still missing is one no schema
        // declares — a caller bug, not an old project — and stays refused.
        let offset = self.offset;
        let param = self
            .effect
            .params
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or(BridgeError::InvalidParam)?;

        value.write_at(&mut param.value, offset, bounds)
    }

    #[frb(ignore)]
    fn param(&self, id: &str) -> Result<&EffectParam, BridgeError> {
        self.effect
            .params
            .iter()
            .find(|p| p.id == id)
            .ok_or(BridgeError::InvalidParam)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// **The coating rows resolve against a parameter that exists** (K-371).
    ///
    /// A group's visibility is looked up in the panel by sibling id, and a
    /// sibling that is not there fails silently — the rows naming it simply
    /// never draw. Shipping `"lens"` instead of `"lens_model"` did exactly
    /// that, leaving only the rows with an unreachable threshold, whose empty
    /// value set means "always visible". So: the id names a real Choice, and
    /// every emitted rule is one the panel can act on.
    #[test]
    fn every_element_row_names_a_real_sibling_and_a_reachable_set() {
        let flare = lumit_core::fx::BUILTINS
            .iter()
            .find(|s| s.match_name == "lens_flare")
            .expect("the Lens flare is a builtin");
        assert!(
            flare.params.iter().any(|p| p.id == LENS_PICK_PARAM),
            "`{LENS_PICK_PARAM}` must be a parameter of the Lens flare"
        );

        let groups = list_parameter_groups("lens_flare".to_owned());
        let mut per_element = 0;
        for g in &groups {
            let Some(param) = &g.visible_when_param else {
                assert!(g.visible_when_values.is_empty());
                continue;
            };
            assert!(
                flare.params.iter().any(|p| p.id == param),
                "group `{}` is resolved against `{param}`, which the schema \
                 does not declare",
                g.label
            );
            assert!(
                !g.visible_when_values.is_empty(),
                "a conditional group with an empty value set reads as \
                 unconditional, which is the opposite of what it means"
            );
            if param == LENS_PICK_PARAM {
                per_element += 1;
            }
        }
        assert_eq!(
            per_element,
            lumit_core::fx::lens_flare::MAX_COATING_ELEMENTS,
            "every element row must carry a rule"
        );

        // A threshold no bundled lens reaches must resolve to "never", and it
        // says so with an index no Lens choice can hold.
        let deep = groups
            .iter()
            .filter(|g| g.visible_when_param.as_deref() == Some(LENS_PICK_PARAM))
            .filter(|g| g.visible_when_values == vec![u32::MAX])
            .count();
        let reachable = lumit_core::fx::lens_flare::library_element_counts()
            .into_iter()
            .max()
            .expect("a library");
        assert_eq!(
            deep,
            lumit_core::fx::lens_flare::MAX_COATING_ELEMENTS - reachable as usize,
            "the rows past the deepest bundled lens are the ones that never draw"
        );
    }
}
