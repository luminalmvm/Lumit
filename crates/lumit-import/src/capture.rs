//! The capture schema — the serde types that mirror the Lumit Bridge's
//! `capture.json`, `manifest.json` and `report.json` exactly as
//! [docs/impl/ae-import.md](../../../docs/impl/ae-import.md) §2 pins them.
//!
//! In plain terms: this is *what After Effects said*, written down in AE's own
//! words. Times are the DOM's float seconds, ids are AE's own integers,
//! property trees keep their match names, and nothing has been converted yet —
//! the converting happens later, in Rust, where the tests can watch it (K-410).
//!
//! Two shape rules run through every type here, and both come from how the
//! walker works:
//!
//! - **Almost everything is optional.** The walker reads one property at a time
//!   inside a try/catch, because one AE property that refuses to be read must
//!   never abort an export. Anything it could not read is simply absent, so a
//!   half-read layer still arrives — which is worth far more than no layer.
//! - **Unknown fields are ignored, never refused.** The schema grows by
//!   addition, so a newer Bridge writing a field this reader has never heard of
//!   parses fine and the extra field is dropped (docs/10 §1.1's rule for `.lum`
//!   files, applied here). Nothing in this file uses `deny_unknown_fields`, and
//!   nothing should.
//!
//! And one vocabulary rule, because it decides what every string in here
//! contains: **the enum-valued fields hold AE's own ExtendScript constant
//! names, verbatim** — `SCREEN`, `ALPHA_INVERTED`, `SUBTRACT`, `BEST`,
//! `PIXEL_MOTION`, `BEZIER`, `HOLD`. The walker does not lower-case or re-spell
//! them, because re-spelling is a conversion and conversions live on this side
//! of the seam (K-410). Match on them exactly as the note's §2 vocabulary
//! paragraph pins them.

use serde::Deserialize;

/// `manifest.json` — the bundle's identity and schema version.
///
/// Read first and on its own: the version decides whether the rest is worth
/// parsing at all (docs/11 §2.3 — refuse a newer major, accept older).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct Manifest {
    /// Always `lumit-ae-bundle`; anything else is not a bundle.
    pub format: Option<String>,
    /// Semver of the *capture schema*, not of Lumit or of the Bridge.
    pub version: Option<String>,
    /// The After Effects build that was walked, e.g. `26.0x67`.
    pub ae_version: Option<String>,
    /// The Bridge script's own version.
    pub bridge_version: Option<String>,
    /// Export date, as the Bridge wrote it.
    pub exported: Option<String>,
}

/// `capture.json` — the whole walk: the project-wide settings, the flat item
/// list, and the comps.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct Capture {
    /// The settings that belong to the project rather than to any item.
    pub project: Option<Project>,
    /// The project's items, flat; the tree is rebuilt from `parent_id`.
    #[serde(default)]
    pub items: Vec<Item>,
    /// One entry per composition, keyed back to its item by `id`.
    #[serde(default)]
    pub comps: Vec<Comp>,
}

/// The project-wide settings no item carries.
///
/// Here because docs/11 §3's colour flagging needs them and nothing downstream
/// can recover them afterwards: whether a comp relied on 8-bpc non-linear
/// blending arithmetic is a fact about the *project*, not about the comp.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct Project {
    /// 8, 16 or 32.
    pub bits_per_channel: Option<u32>,
    /// The working colour space's name, as AE reports it; empty means none set.
    pub working_space: Option<String>,
    pub linear_blending: Option<bool>,
    pub linearize_working_space: Option<bool>,
    /// `extendscript` or `javascript-1.0` — which engine the expressions expect.
    pub expression_engine: Option<String>,
}

/// One row of the project panel: a folder, a comp, a footage item, or a solid.
///
/// The kind-specific fields sit inline rather than in a nested payload, which
/// is the AE DOM's own shape (an `Item` simply exposes the attributes that
/// apply to it) and means an unrecognised future `kind` still parses.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct Item {
    /// AE's own item id — the link target for `Comp::id` and `Layer::source_id`.
    pub id: Option<i64>,
    pub name: Option<String>,
    /// The containing folder's id. Comps have no name of their own in
    /// `comps[]`; it lives here.
    pub parent_id: Option<i64>,
    /// `folder` | `comp` | `footage` | `solid`.
    pub kind: Option<String>,

    // --- footage ---
    /// The file on disk, as AE recorded it.
    pub path: Option<String>,
    pub fps: Option<f64>,
    /// The file's own frame rate, before any override — the only way to tell
    /// whether `fps_override` actually changed anything.
    pub native_fps: Option<f64>,
    pub duration: Option<f64>,
    /// AE's conform frame rate; 0 means "no override".
    pub fps_override: Option<f64>,
    /// `IGNORE` | `STRAIGHT` | `PREMULTIPLIED`.
    pub alpha: Option<String>,
    /// The matte colour a premultiplied item is unmultiplied against.
    pub premul_colour: Option<Vec<f64>>,
    pub invert_alpha: Option<bool>,
    /// AE's loop count.
    #[serde(rename = "loop")]
    pub loop_count: Option<i64>,
    /// `OFF` | `UPPER_FIELD_FIRST` | `LOWER_FIELD_FIRST`.
    pub fields: Option<String>,
    /// AE's pulldown phase, stringified.
    pub remove_pulldown: Option<String>,
    pub is_still: Option<bool>,
    /// The footage is a folder of numbered stills read as one item, not a
    /// single file (K-439). After Effects says so in the file alias, which
    /// targets a *folder* rather than a file — a field that names itself,
    /// not a byte offset. [`Item::path`] is then that folder.
    pub is_sequence: Option<bool>,
    /// A sequence's file name up to its number (`Depth`) and from the end of
    /// its number (`_depth.exr`). Recorded because they are the only thing
    /// After Effects knows about the run that the folder itself does not say;
    /// nothing reads them yet.
    pub sequence_prefix: Option<String>,
    pub sequence_suffix: Option<String>,
    pub is_placeholder: Option<bool>,
    pub is_missing: Option<bool>,

    // --- solid (and footage: both carry a size) ---
    /// The solid's colour, AE's 0..1 floats.
    pub colour: Option<Vec<f64>>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// A composition's settings and its layer stack (docs/11 §2.2 item 2).
///
/// There is no `name` here on purpose: the comp is also an [`Item`], and the
/// name lives there, once.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct Comp {
    /// The matching [`Item::id`].
    pub id: Option<i64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Pixel aspect ratio.
    pub par: Option<f64>,
    pub fps: Option<f64>,
    pub duration: Option<f64>,
    /// Start time in seconds (AE's start timecode, resolved).
    pub start: Option<f64>,
    pub bg_colour: Option<Vec<f64>>,
    pub motion_blur: Option<MotionBlur>,
    /// Classic 3D / Advanced 3D / CINEMA 4D, recorded verbatim.
    pub renderer: Option<String>,
    /// "Preserve frame rate when nested" (docs/11 §2.2 item 2).
    pub preserve_nested_fps: Option<bool>,
    /// "Preserve resolution when nested".
    pub preserve_nested_resolution: Option<bool>,
    #[serde(default)]
    pub markers: Vec<Marker>,
    /// In AE's stacking order, top first.
    #[serde(default)]
    pub layers: Vec<Layer>,
}

/// A comp's motion-blur settings, in AE's own units.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct MotionBlur {
    /// The comp's motion-blur master switch. The settings below are stored
    /// whether or not it is on, so the switch is the fact that matters.
    pub enabled: Option<bool>,
    pub shutter_angle: Option<f64>,
    pub shutter_phase: Option<f64>,
    /// Samples per frame.
    pub samples: Option<u32>,
    pub adaptive_limit: Option<u32>,
}

/// A comp or layer marker (docs/11 §2.2 item 13). `t` matches
/// [`Keyframe::t`]: both are a time on the same clock.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct Marker {
    pub t: Option<f64>,
    pub duration: Option<f64>,
    pub comment: Option<String>,
    /// AE's chapter-link text.
    pub chapter: Option<String>,
    /// AE's label colour index.
    pub label: Option<u32>,
}

/// One layer of a comp (docs/11 §2.2 item 3).
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct Layer {
    /// AE's 1-based stacking index; `parent_index` and `Matte::layer_index`
    /// both point at one of these.
    pub index: Option<u32>,
    pub name: Option<String>,
    /// `footage | solid | precomp | text | shape | null | adjustment | camera |
    /// light | audio`.
    pub kind: Option<String>,
    /// The [`Item::id`] this layer draws from, where it has one.
    pub source_id: Option<i64>,
    pub in_point: Option<f64>,
    pub out_point: Option<f64>,
    pub start_time: Option<f64>,
    /// AE's time stretch, as a percentage (100 = unstretched).
    pub stretch: Option<f64>,
    pub parent_index: Option<u32>,
    /// AE's label colour index.
    pub label: Option<u32>,
    /// Blend mode, as the DOM names it — `NORMAL`, `SCREEN`, `DISSOLVE`.
    pub blend: Option<String>,
    pub preserve_transparency: Option<bool>,
    /// `NO_AUTO_ORIENT` | `ALONG_PATH` | `CAMERA_OR_POINT_OF_INTEREST` |
    /// `CHARACTERS_TOWARD_CAMERA`. On a camera this is the one/two-node flag
    /// (docs/11 §2.2 item 12): a two-node camera orients at its point of
    /// interest.
    pub auto_orient: Option<String>,
    /// `PARALLEL` | `SPOT` | `POINT` | `AMBIENT`, on light layers only.
    pub light_type: Option<String>,
    pub matte: Option<Matte>,
    pub switches: Option<Switches>,
    #[serde(default)]
    pub markers: Vec<Marker>,
    pub time_remap_enabled: Option<bool>,
    /// The layer's top-level property groups — Transform, Masks, Effects and
    /// the rest — in DOM order. The layer is itself the root group, and its
    /// name is already recorded above, so what is stored is its children.
    #[serde(default)]
    pub properties: Vec<Property>,
}

/// The matte reference, both AE generations at once: 23.0+ records a type and
/// the referenced layer's index, the legacy form implies the layer above.
/// Normalising the two is Rust's job at mapping time, not the walker's.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct Matte {
    /// `NO_TRACK_MATTE` | `ALPHA` | `ALPHA_INVERTED` | `LUMA` |
    /// `LUMA_INVERTED`.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// The matte layer's stacking index, where the 23.0+ form named one.
    pub layer_index: Option<u32>,
    /// Whether *this* layer is being used as somebody else's matte — which is
    /// all the legacy above-layer form has to say about itself.
    pub is_track_matte: Option<bool>,
}

/// The layer switches (docs/11 §2.2 item 3). `quality` (`WIREFRAME` | `DRAFT` |
/// `BEST`) and `frame_blending` (`NO_FRAME_BLEND` | `FRAME_MIX` |
/// `PIXEL_MOTION`) are AE enum names rather than booleans.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct Switches {
    /// AE's video switch — whether the layer draws at all.
    pub enabled: Option<bool>,
    pub audio: Option<bool>,
    pub solo: Option<bool>,
    pub lock: Option<bool>,
    pub shy: Option<bool>,
    pub quality: Option<String>,
    pub motion_blur: Option<bool>,
    pub adjustment: Option<bool>,
    pub three_d: Option<bool>,
    /// Collapse transformations / continuously rasterise.
    pub collapse: Option<bool>,
    pub frame_blending: Option<String>,
    pub guide: Option<bool>,
    /// AE's fx switch — whether the layer's effect stack renders at all.
    pub effects_active: Option<bool>,
}

/// One node of the property tree, group or leaf.
///
/// One struct covers both because that is how the JSON reads: a node is a
/// *group* when `group` is present and a *leaf* otherwise, exactly as the DOM
/// distinguishes a `PropertyGroup` from a `Property`. Splitting it into an
/// enum would buy a tag the Bridge does not write, and would refuse any node
/// whose shape a later Bridge changes.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct Property {
    /// AE's stable identifier — `ADBE Position`, `ADBE Gaussian Blur 2`. The
    /// effect table is keyed by these, never by display names.
    pub match_name: Option<String>,
    /// The display name, which the user may have renamed.
    pub name: Option<String>,
    /// Whether the node is switched on. Present on group nodes, and it is what
    /// carries an effect instance's enabled state (docs/11 §2.2 item 9).
    pub enabled: Option<bool>,

    /// Present on the mask nodes under `ADBE Mask Parade`, and on nothing else:
    /// the facts that belong to the mask itself rather than to one of its
    /// properties.
    pub mask: Option<Mask>,

    /// Present on group nodes: the children, in DOM order.
    pub group: Option<Vec<Property>>,

    /// Present on leaf nodes: AE's `propertyValueType` name.
    pub value_type: Option<String>,
    /// The static value, verbatim — a number, an array, a string, or a shape
    /// object (see [`Shape`]). Kept as raw JSON because AE's property values
    /// are genuinely that varied, and converting is a later phase's job.
    pub value: Option<serde_json::Value>,
    /// The animation, when there is one. A value copy: no resampling, no
    /// baking (K-025).
    pub keyframes: Option<Vec<Keyframe>>,
    /// The expression source text, verbatim; never evaluated by the Bridge.
    pub expression: Option<String>,
    pub expression_enabled: Option<bool>,
    /// For a dimension-separated property, the per-dimension followers. The
    /// leader's own keyframes are *not* the animation — these are.
    pub separated: Option<Vec<Property>>,
    /// The ExtendScript error text, when this property could not be read at
    /// all (Curves' point list and its `CUSTOM_VALUE` siblings). Recorded so
    /// the import report can name it rather than quietly dropping it.
    pub unreadable: Option<String>,
}

/// A mask's own facts, which sit on the mask node rather than among its
/// properties — feather, opacity and expansion are ordinary child properties,
/// but these are not.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct Mask {
    /// `NONE` | `ADD` | `SUBTRACT` | `INTERSECT` | `LIGHTEN` | `DARKEN` |
    /// `DIFFERENCE`.
    pub mode: Option<String>,
    pub inverted: Option<bool>,
    /// AE's RotoBezier flag: the path's tangents are computed, not stored.
    pub roto_bezier: Option<bool>,
    pub locked: Option<bool>,
    /// The mask's outline colour in the Timeline — cosmetic, carried anyway.
    pub colour: Option<Vec<f64>>,
}

impl Property {
    /// The children of a group node, or nothing for a leaf.
    pub fn children(&self) -> &[Property] {
        self.group.as_deref().unwrap_or(&[])
    }

    /// This node's static value read as a bezier path, for mask and shape
    /// paths. Returns `None` when the value is absent or is not a path.
    pub fn shape(&self) -> Option<Shape> {
        let value = self.value.clone()?;
        serde_json::from_value(value).ok()
    }
}

/// A bezier path, as the DOM hands it over: parallel arrays, one entry per
/// vertex, tangents relative to their vertex.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct Shape {
    #[serde(default)]
    pub vertices: Vec<Vec<f64>>,
    #[serde(default)]
    pub in_tangents: Vec<Vec<f64>>,
    #[serde(default)]
    pub out_tangents: Vec<Vec<f64>>,
    pub closed: Option<bool>,
}

/// One keyframe, with everything docs/11 §2.2 item 5 lists.
///
/// Because Lumit's keyframe maths is AE-compatible (K-025), this is a value
/// copy and not a conversion — which is exactly why every side of every handle
/// has to survive the trip.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct Keyframe {
    /// Time in the DOM's float seconds.
    pub t: Option<f64>,
    /// The value, verbatim — same varied shapes as [`Property::value`].
    pub v: Option<serde_json::Value>,
    /// `LINEAR` | `BEZIER` | `HOLD`, per side.
    pub in_interp: Option<String>,
    pub out_interp: Option<String>,
    /// Temporal ease, **one entry per dimension** — the DOM returns an array,
    /// and a spatial property returns exactly one entry for all of them.
    /// Captured as-is.
    pub in_ease: Option<Vec<Ease>>,
    pub out_ease: Option<Vec<Ease>>,
    /// Spatial tangents, where the property is spatial.
    pub in_tangent: Option<Vec<f64>>,
    pub out_tangent: Option<Vec<f64>>,
    pub roving: Option<bool>,
    /// Temporal auto-bezier: the handles follow the neighbouring values.
    pub auto_bezier: Option<bool>,
    /// Temporal continuity: the two sides share one handle direction.
    pub continuous: Option<bool>,
    /// The same two flags again for the *spatial* handles, which AE keeps
    /// separately — a key can be smooth in time and cornered in space.
    pub spatial_auto_bezier: Option<bool>,
    pub spatial_continuous: Option<bool>,
}

/// One side of one dimension's temporal ease. Influence is in AE's 0.1–100
/// range, speed is in the property's units per second.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct Ease {
    pub speed: Option<f64>,
    pub influence: Option<f64>,
}

/// `report.json` — what the Bridge itself already knows went wrong.
///
/// Only unreadables so far: the walker has no other opinions to record. This
/// is the human-facing half of the bundle, so its rows name things the way a
/// report row shows them.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct Report {
    #[serde(default)]
    pub unreadables: Vec<Unreadable>,
}

/// One property the walker could not read, and where it was.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct Unreadable {
    /// The comp's name.
    pub comp: Option<String>,
    /// The layer's name.
    pub layer: Option<String>,
    /// The property path within the layer, as a reader would say it.
    pub path: Option<String>,
    pub match_name: Option<String>,
    /// The ExtendScript error text, verbatim.
    pub error: Option<String>,
}
