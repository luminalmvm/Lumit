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
}

/// Every built-in effect, in schema order — the Add-effect menu's source of
/// truth ([`lumit_core::fx::BUILTINS`]), and the frb form of v0's `list_effects`.
///
/// Stateless, so it is a free function rather than a method: the menu is
/// available before any project is open.
#[frb(sync)]
pub fn list_effects() -> Vec<BridgeEffectInfo> {
    lumit_core::fx::BUILTINS
        .iter()
        // The Drivers family is in the catalogue (K-471 WP1) but is not an
        // Add-effect entry: a driver belongs in the Graph panel's own search,
        // where a wire can be drawn from it, and dropping one on a stack would
        // add a node that changes no pixel. WP2 gives the family its own listing
        // and this filter goes with it (docs/impl/node-graph.md §8).
        .filter(|schema| schema.category != lumit_core::fx::FxCategory::Drivers)
        .map(|schema| BridgeEffectInfo {
            name: schema.match_name.to_owned(),
            label: schema.label.to_owned(),
            // Shared with v0 rather than restated, so the two frontends cannot
            // disagree about which key a category has.
            category: crate::edits::fx_category_key(schema.category).to_owned(),
            category_label: schema.category.label().to_owned(),
        })
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
        .map(|dir| presets_in(&dir))
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

/// The listing itself, on any folder — split from [`list_presets`] so the scan
/// is testable without touching the user's real library.
#[frb(ignore)]
pub(crate) fn presets_in(dir: &std::path::Path) -> Vec<BridgePresetInfo> {
    #[derive(serde::Deserialize)]
    struct Named {
        name: Option<String>,
        // Presence is the "is this actually a preset" check; the effects
        // themselves are parsed properly at load time.
        effects: serde_json::Value,
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<BridgePresetInfo> = entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if !path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("lumfx"))
            {
                return None;
            }
            let text = std::fs::read_to_string(&path).ok()?;
            // It must at least be preset JSON with an effects list; the saved
            // display name wins, the file's stem stands in without one.
            let named: Named = serde_json::from_str(&text).ok()?;
            if !named.effects.is_array() {
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
    let seconds = if time.den == 0 {
        0.0
    } else {
        time.num as f64 / time.den as f64
    };
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
    let seconds = if time.den == 0 {
        0.0
    } else {
        time.num as f64 / time.den as f64
    };
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
    /// per parameter engine-side, so `centre_x` can be px@comp on one effect
    /// and a per cent of the frame on another without the panel guessing.
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
    use lumit_core::fx::ParamKind;

    let Some(schema) = lumit_core::fx::BUILTINS
        .iter()
        .find(|s| s.match_name == effect)
    else {
        return Vec::new();
    };

    schema
        .params
        .iter()
        .map(|param| {
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
                ParamKind::Angle { default, dial_step } => {
                    BridgeParamKind::Angle { default, dial_step }
                }
                // `self_default` is an engine-side instantiation detail
                // (K-288) — the panel draws the same picker either way, and
                // the value it edits already carries the layer id.
                ParamKind::Layer { .. } => BridgeParamKind::Layer,
                // `self_default` is an engine-side resolution detail here too
                // (K-408): the panel always offers "First mask" as its unset
                // entry, and what an unset row comes to is the render's answer,
                // not a control the panel draws differently.
                ParamKind::MaskPath { .. } => BridgeParamKind::MaskPath,
                ParamKind::Curve => BridgeParamKind::Curve,
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
        })
        .collect()
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
    let Some(schema) = lumit_core::fx::BUILTINS
        .iter()
        .find(|s| s.match_name == effect)
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
    let Some(schema) = lumit_core::fx::BUILTINS
        .iter()
        .find(|s| s.match_name == effect)
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

    let Some(schema) = lumit_core::fx::BUILTINS
        .iter()
        .find(|s| s.match_name == effect)
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

/// How a keyframe joins its neighbour on one side ([`SideInterp`]).
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BridgeSideInterp {
    Hold,
    Linear,
    Bezier(BridgeBezierSide),
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

    /// Overwrite `target` with this value.
    ///
    /// A parameter's *kind* is declared by the effect's schema and is not the
    /// panel's to change, so a mismatched pair is refused rather than replacing
    /// the value: writing a number to a colour would leave an instance the
    /// effect's own resolver cannot read, and it would be undoable but not
    /// obviously wrong on screen.
    #[frb(ignore)]
    fn write_at(self, target: &mut EffectValue, offset: Rational) -> Result<(), BridgeError> {
        match (self, target) {
            (BridgeEffectValue::Float(scalar), EffectValue::Float(property)) => {
                property.animation = scalar.animation_at(offset)?;
                Ok(())
            }
            (BridgeEffectValue::Point(point), EffectValue::Point(x, y)) => {
                let (ax, ay) = (point.x.animation_at(offset)?, point.y.animation_at(offset)?);
                x.animation = ax;
                y.animation = ay;
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
                    property.animation = animation;
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
    effect: EffectInstance,
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
    let effect = &filled;
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
    #[frb(sync)]
    pub fn set_value(&mut self, id: String, value: BridgeEffectValue) -> Result<(), BridgeError> {
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

        value.write_at(&mut param.value, offset)
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
