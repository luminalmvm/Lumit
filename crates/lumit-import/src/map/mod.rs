//! The structural mapping: an After Effects capture becomes a whole new Lumit
//! document ([docs/11-AE-IMPORT.md](../../../../docs/11-AE-IMPORT.md),
//! [docs/impl/ae-import.md](../../../../docs/impl/ae-import.md) §4).
//!
//! # In plain terms
//!
//! This is the half of the import that does the thinking. It reads the walk
//! the Lumit Bridge wrote — After Effects' own words, unaltered — and builds a
//! Lumit project out of it: the folder tree, the footage items, the
//! compositions, every layer with its keyframes, masks, mattes and effects.
//!
//! Two rules run through the whole of it, and they are the reason it reads the
//! way it does.
//!
//! **An import never fails.** Not for a broken item, not for a composition
//! whose settings never arrived, not for a layer pointing at footage that is
//! gone. Every one of those becomes a row in the report and the import carries
//! on, because a person who has just waited for a two-hundred-comp project to
//! convert is not helped by an error message. The only way to lose something
//! silently is to not write it down, so everything that changed on the way
//! across is written down.
//!
//! **Import makes a new project.** Merging a capture into a project that is
//! already open is a later piece of work; today the answer is always a fresh
//! [`Document`], which is why every After Effects id here becomes a brand-new
//! Lumit id rather than trying to match anything.
//!
//! What After Effects knew and Lumit has no field for — its item ids, its
//! renderer name, its interpretation settings, a layer's stretch percentage —
//! is not thrown away. It is parked in an **`ae` namespace** on whichever
//! object it belonged to, and `.lum` carries unknown fields through load and
//! save untouched (docs/10 §1.1), so it survives indefinitely and a later
//! Lumit that grows the field can pick it up.

mod curves;
mod effects;
mod fx_colour;
mod fx_distort;
mod layers;
mod props;
mod styles;
mod table;
mod time;

use std::collections::{BTreeMap, HashMap};

use lumit_core::model::{
    Composition, Document, Folder, FootageItem, LinearColour, MediaRef, MotionBlur, ProjectItem,
    SequenceRef, SolidDef,
};
use lumit_core::time::Rational;
use uuid::Uuid;

use crate::capture::{Capture, Comp, Item};
use crate::report::{ImportReport, ItemPath, Outcome, Reason};

pub use effects::{map_effect, MappedEffect};

use props::{ae_extra, ae_map};
use time::{TimeBase, DEFAULT_DURATION, DEFAULT_FPS};

/// What a project item turned out to be — the half of an [`Item`] the layer
/// mapping needs, so a layer naming a source knows whether it is a Precomp
/// layer, a Solid layer or a Footage layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ItemKind {
    Folder,
    Footage,
    Solid,
    Comp,
}

/// The After Effects id → Lumit id table, built before anything is mapped so
/// that a layer can name a comp that has not been built yet.
#[derive(Debug, Default)]
pub(crate) struct Items {
    by_ae_id: HashMap<i64, (Uuid, ItemKind)>,
    /// Each item's own raster, where it has one — a comp's dimensions, a
    /// solid's, a piece of footage's. **A layer's effects are measured against
    /// this, not against the composition**: After Effects runs an
    /// effect on the layer, so its per cents are per cents of the layer's own
    /// frame and its points are points in it. The two only coincide while the
    /// layer happens to be the comp's size.
    sizes: HashMap<i64, (f64, f64)>,
}

impl Items {
    pub(crate) fn get(&self, ae_id: i64) -> Option<(Uuid, ItemKind)> {
        self.by_ae_id.get(&ae_id).copied()
    }

    /// The raster an item draws at, for the layers that draw from it.
    pub(crate) fn size(&self, ae_id: i64) -> Option<(f64, f64)> {
        self.sizes.get(&ae_id).copied()
    }
}

/// The conversion's running state: where times come from, where a report row
/// goes, and the origin the current layer's key times are measured against.
pub struct Conv<'a> {
    pub(crate) report: &'a mut ImportReport,
    pub(crate) tb: TimeBase,
    /// The current layer's start time. After Effects reports a layer
    /// property's key times on the **composition's** clock; Lumit stores them
    /// on the layer's own, which begins at `start_offset`. Subtracting this is
    /// the whole of the difference, and doing it in one place is what keeps a
    /// layer that was dragged along the timeline from importing with its
    /// animation in the wrong decade.
    pub(crate) offset: Rational,
    /// **The frame the current layer's effects run on**. After Effects
    /// measures a Twirl radius as a per cent of the layer and a blur centre as
    /// a point, where Lumit reads px@comp and a per cent of the frame (docs/08
    /// §2.3), so those conversions cannot be done without knowing the frame —
    /// and the frame an effect sees is the **layer's** raster, not the
    /// composition's. [`layers::map_layer`] sets this to the layer's source
    /// size around its effect stack and puts the composition's back afterwards;
    /// a layer with no source of its own (text, a shape, a null) draws at the
    /// composition's size, which is what it falls back to.
    pub(crate) size: (f64, f64),
    /// The current layer's own span, in its own timebase. The two clock-reading
    /// After Effects controls (Ripple's Wave Speed, Wave warp's) become
    /// keyframes across it (docs/08 §3.53).
    pub(crate) span: (Rational, Rational),
    /// AE stacking index → Lumit layer id, so an effect parameter naming a
    /// layer (Set matte's, Displacement map's) resolves to the same id the
    /// layer itself was given.
    pub(crate) layer_ids: BTreeMap<u32, Uuid>,
    /// The current layer's masks in After Effects' own order, each with the
    /// perimeter of its path. An effect parameter naming a mask (Vegas' Path,
    /// Scribble's Mask, Stroke's Path) resolves through the index; the
    /// perimeter is what turns AE's *count* of segments into Lumit's segment
    /// *length* (docs/11 §5's Vegas row).
    pub(crate) masks: Vec<(Uuid, f64)>,
    /// The current layer's own AE stacking index, or 0 outside a layer. An
    /// effect parameter naming a layer may name **this** one — After Effects
    /// defaults Set Channels' four source pickers to it — and telling that
    /// apart from a genuine second source is the difference between an exact
    /// conversion and a reported approximation (docs/11 §5's Set Channels row).
    pub(crate) self_index: u32,
}

impl Conv<'_> {
    /// A capture time, in the current layer's own timebase.
    pub(crate) fn layer_time(&self, t: f64) -> Rational {
        self.offset_from(self.tb.seconds(t))
    }

    /// The same subtraction, for a time already made exact.
    pub(crate) fn offset_from(&self, t: Rational) -> Rational {
        t.checked_sub(self.offset).unwrap_or(t)
    }

    /// The current frame's diagonal in pixels — what After Effects' per cents
    /// of the layer convert through (docs/08 §2.3). [`Self::size`] says which
    /// frame that is.
    pub(crate) fn diagonal(&self) -> f64 {
        let (w, h) = self.size;
        (w * w + h * h).sqrt().max(1.0)
    }
}

/// Map a whole capture onto a fresh document.
///
/// The one entry point. Never returns an error: everything that could not be
/// carried is a row in the returned [`ImportReport`].
#[must_use]
pub fn map_capture(capture: &Capture) -> (Document, ImportReport) {
    let mut report = ImportReport::default();
    let mut doc = Document::new();

    // Pass one: hand every item a Lumit id, so the tree and the layers can
    // both refer to things that have not been built yet.
    let mut items = Items::default();
    // A comp keeps its dimensions on its `comps` entry rather than on its item
    // row, so both are read here — the sizes table below wants either.
    let comps: HashMap<i64, &Comp> = capture
        .comps
        .iter()
        .filter_map(|c| c.id.map(|id| (id, c)))
        .collect();
    let mut order: Vec<(&Item, Uuid, ItemKind)> = Vec::with_capacity(capture.items.len());
    for item in &capture.items {
        let Some(kind) = item_kind(item) else {
            report.row(
                ItemPath::item(&item_name(item)),
                Outcome::Skipped,
                Reason::ItemUnreadable,
            );
            continue;
        };
        let Some(ae_id) = item.id else {
            report.row(
                ItemPath::item(&item_name(item)),
                Outcome::Skipped,
                Reason::ItemUnreadable,
            );
            continue;
        };
        let uuid = Uuid::now_v7();
        items.by_ae_id.insert(ae_id, (uuid, kind));
        let raster = match kind {
            ItemKind::Comp => comps
                .get(&ae_id)
                .and_then(|c| c.width.zip(c.height))
                .map(|(w, h)| (f64::from(w), f64::from(h))),
            ItemKind::Footage | ItemKind::Solid => item
                .width
                .zip(item.height)
                .map(|(w, h)| (f64::from(w), f64::from(h))),
            ItemKind::Folder => None,
        };
        if let Some((w, h)) = raster.filter(|(w, h)| *w >= 1.0 && *h >= 1.0) {
            items.sizes.insert(ae_id, (w, h));
        }
        order.push((item, uuid, kind));
    }

    // Pass two: build them, in the capture's own order, which is the Project
    // panel's order.
    for (item, uuid, kind) in &order {
        let name = item_name(item);
        let built = match kind {
            ItemKind::Folder => ProjectItem::Folder(Folder {
                id: *uuid,
                name,
                children: Vec::new(),
                extra: ae_extra("id", serde_json::json!(item.id)),
            }),
            ItemKind::Footage => ProjectItem::Footage(footage(&mut report, item, *uuid, &name)),
            ItemKind::Solid => ProjectItem::Solid(SolidDef {
                id: *uuid,
                name,
                colour: linear_colour(item.colour.as_deref()),
                width: item.width.unwrap_or(1).max(1),
                height: item.height.unwrap_or(1).max(1),
                extra: ae_extra("id", serde_json::json!(item.id)),
            }),
            ItemKind::Comp => {
                let ae_id = item.id.unwrap_or_default();
                match comps.get(&ae_id) {
                    Some(comp) => ProjectItem::Composition(composition(
                        &mut report,
                        comp,
                        *uuid,
                        &name,
                        &items,
                    )),
                    None => {
                        // The item list said there was a comp and the walk had
                        // nothing to say about it. Importing it empty keeps
                        // every precomp layer that names it resolvable.
                        report.row(ItemPath::item(&name), Outcome::Skipped, Reason::CompMissing);
                        ProjectItem::Composition(empty_comp(*uuid, &name, ae_id))
                    }
                }
            }
        };
        report.imported();
        doc.items.push(built);
    }

    // Pass three: the folder tree. Ids are already handed out, so this is a
    // single walk with no ordering worries.
    let mut children: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for (item, uuid, _) in &order {
        let Some(parent) = item.parent_id.filter(|id| *id != 0) else {
            continue;
        };
        if let Some((parent, ItemKind::Folder)) = items.get(parent) {
            children.entry(parent).or_default().push(*uuid);
        }
    }
    for item in &mut doc.items {
        if let ProjectItem::Folder(folder) = item {
            if let Some(kids) = children.remove(&folder.id) {
                folder.children = kids;
            }
        }
    }

    doc.extra = project_extra(&mut report, capture);
    (doc, report)
}

/// docs/11 §3's colour note: an 8-bpc project without linear blending did its
/// compositing in a different arithmetic from Lumit's scene-linear one, so
/// blend results can shift. Said once, for the project, because it is a fact
/// about the project.
fn project_extra(
    report: &mut ImportReport,
    capture: &Capture,
) -> serde_json::Map<String, serde_json::Value> {
    let Some(project) = capture.project.as_ref() else {
        return serde_json::Map::new();
    };
    let bits = project.bits_per_channel.unwrap_or(8);
    if bits == 8 && project.linear_blending != Some(true) {
        report.row(
            ItemPath::default(),
            Outcome::Adjusted,
            Reason::ProjectBlendingDiffers { bits },
        );
    }
    ae_map(vec![
        ("bits_per_channel", serde_json::json!(bits)),
        ("working_space", serde_json::json!(project.working_space)),
        (
            "linear_blending",
            serde_json::json!(project.linear_blending),
        ),
        (
            "linearize_working_space",
            serde_json::json!(project.linearize_working_space),
        ),
        (
            "expression_engine",
            serde_json::json!(project.expression_engine),
        ),
    ])
}

/// What the Project panel calls this item.
///
/// An **empty** name is no name rather than a name that happens to be blank:
/// After Effects stores nothing for an item the user never renamed, and the
/// `.aep` reader has already put the file's own name back where it can. A row
/// with a blank label in the Project panel is unusable and unfindable, so
/// anything still nameless here says so out loud.
fn item_name(item: &Item) -> String {
    item.name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("(unnamed)")
        .to_string()
}

fn item_kind(item: &Item) -> Option<ItemKind> {
    Some(match item.kind.as_deref()? {
        "folder" => ItemKind::Folder,
        "footage" => ItemKind::Footage,
        "solid" => ItemKind::Solid,
        "comp" => ItemKind::Comp,
        _ => return None,
    })
}

/// A footage item. The path is carried as-is on both sides of the
/// [`MediaRef`] — relinking is a later phase, and a missing file is a report
/// row, never a blocked import (docs/11 §2.5).
fn footage(report: &mut ImportReport, item: &Item, id: Uuid, name: &str) -> FootageItem {
    let path = item.path.clone().unwrap_or_default();
    if item.is_missing == Some(true) {
        report.row(
            ItemPath::item(name),
            Outcome::Adjusted,
            Reason::MediaMissing { path: path.clone() },
        );
    }
    if item.is_placeholder == Some(true) {
        report.row(
            ItemPath::item(name),
            Outcome::Adjusted,
            Reason::MediaPlaceholder,
        );
    }
    FootageItem {
        id,
        name: name.to_string(),
        media: MediaRef {
            relative_path: path.clone(),
            absolute_path: path,
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        // A folder of numbered stills is one item, at the default rate:
        // After Effects' own conform rate for a sequence is a
        // preference rather than something the project file carries, so
        // reading a number out of these bytes would be a guess. The path is
        // the *folder* the run lives in — which is what the alias names —
        // and resolution turns it into the run's first file
        // (`lumit_project::resolve_all_media`).
        sequence: (item.is_sequence == Some(true)).then(SequenceRef::default),
        // Everything AE knew about how to read the file. Lumit has no field
        // for most of it yet; the `ae` namespace keeps it until it does.
        extra: ae_map(vec![
            ("id", serde_json::json!(item.id)),
            ("fps", serde_json::json!(item.fps)),
            ("native_fps", serde_json::json!(item.native_fps)),
            ("fps_override", serde_json::json!(item.fps_override)),
            ("duration", serde_json::json!(item.duration)),
            ("alpha", serde_json::json!(item.alpha)),
            ("premul_colour", serde_json::json!(item.premul_colour)),
            ("invert_alpha", serde_json::json!(item.invert_alpha)),
            ("loop", serde_json::json!(item.loop_count)),
            ("fields", serde_json::json!(item.fields)),
            ("remove_pulldown", serde_json::json!(item.remove_pulldown)),
            ("is_still", serde_json::json!(item.is_still)),
            ("sequence_prefix", serde_json::json!(item.sequence_prefix)),
            ("sequence_suffix", serde_json::json!(item.sequence_suffix)),
            ("is_placeholder", serde_json::json!(item.is_placeholder)),
            ("is_missing", serde_json::json!(item.is_missing)),
            ("width", serde_json::json!(item.width)),
            ("height", serde_json::json!(item.height)),
        ]),
        colour_space: None,
    }
}

/// A composition item with no `comps[]` entry: the shell that keeps every
/// reference to it resolvable.
fn empty_comp(id: Uuid, name: &str, ae_id: i64) -> Composition {
    let tb = TimeBase::fallback();
    Composition {
        master_volume_db: 0.0,
        groups: Vec::new(),
        beat_grid: None,
        id,
        name: name.to_string(),
        width: 1920,
        height: 1080,
        frame_rate: tb.rate(),
        duration: tb.duration(DEFAULT_DURATION),
        background: LinearColour::BLACK,
        work_area: None,
        layers: Vec::new(),
        markers: Vec::new(),
        motion_blur: MotionBlur::default(),
        extra: ae_extra("id", serde_json::json!(ae_id)),
    }
}

/// One composition: its settings, its markers, and its layer stack.
fn composition(
    report: &mut ImportReport,
    ae: &Comp,
    id: Uuid,
    name: &str,
    items: &Items,
) -> Composition {
    let path = ItemPath::item(name);

    let tb = match TimeBase::of_fps(ae.fps) {
        Some(tb) => tb,
        None => {
            report.row(
                path.clone(),
                Outcome::Adjusted,
                Reason::CompFrameRateGuessed { used: DEFAULT_FPS },
            );
            TimeBase::fallback()
        }
    };

    let duration = match ae.duration.filter(|d| d.is_finite() && *d > 0.0) {
        Some(d) => tb.duration(d),
        None => {
            report.row(
                path.clone(),
                Outcome::Adjusted,
                Reason::CompDurationGuessed {
                    used: DEFAULT_DURATION,
                },
            );
            tb.duration(DEFAULT_DURATION)
        }
    };

    if let Some(par) = ae.par.filter(|p| (p - 1.0).abs() > 1e-6) {
        report.row(
            path.clone(),
            Outcome::Adjusted,
            Reason::PixelAspectIgnored { par },
        );
    }
    if let Some(start) = ae.start.filter(|s| s.abs() > 1e-9) {
        report.row(
            path.clone(),
            Outcome::Adjusted,
            Reason::CompStartIgnored { start },
        );
    }
    if let Some(renderer) = ae
        .renderer
        .as_deref()
        .filter(|r| !matches!(*r, "ADBE Advanced 3d" | "ADBE Standard 3d"))
    {
        report.row(
            path.clone(),
            Outcome::Adjusted,
            Reason::RendererUnrecognised {
                renderer: renderer.to_string(),
            },
        );
    }
    if ae.preserve_nested_fps == Some(true) || ae.preserve_nested_resolution == Some(true) {
        report.row(
            path.clone(),
            Outcome::Adjusted,
            Reason::NestedPreserveIgnored {
                fps: ae.preserve_nested_fps == Some(true),
                resolution: ae.preserve_nested_resolution == Some(true),
            },
        );
    }

    // Layer ids first, so parenting and mattes can name a layer further down
    // the stack than the one being mapped.
    let ids: BTreeMap<u32, Uuid> = ae
        .layers
        .iter()
        .filter_map(|l| l.index.map(|i| (i, Uuid::now_v7())))
        .collect();

    let mut conv = Conv {
        report,
        tb,
        offset: Rational::ZERO,
        size: (
            f64::from(ae.width.unwrap_or(1920).max(1)),
            f64::from(ae.height.unwrap_or(1080).max(1)),
        ),
        span: (Rational::ZERO, Rational::ZERO),
        layer_ids: ids.clone(),
        masks: Vec::new(),
        self_index: 0,
    };
    let layers = ae
        .layers
        .iter()
        .filter_map(|layer| layers::map_layer(&mut conv, &path, layer, items, &ids))
        .collect();
    let markers = layers::markers(&mut conv, &ae.markers);

    Composition {
        master_volume_db: 0.0,
        groups: Vec::new(),
        beat_grid: None,
        id,
        name: name.to_string(),
        width: ae.width.unwrap_or(1920).max(1),
        height: ae.height.unwrap_or(1080).max(1),
        frame_rate: tb.rate(),
        duration,
        background: linear_colour(ae.bg_colour.as_deref()),
        work_area: None,
        layers,
        markers,
        motion_blur: motion_blur(ae),
        extra: ae_map(vec![
            ("id", serde_json::json!(ae.id)),
            ("par", serde_json::json!(ae.par)),
            ("start", serde_json::json!(ae.start)),
            ("renderer", serde_json::json!(ae.renderer)),
            (
                "preserve_nested_fps",
                serde_json::json!(ae.preserve_nested_fps),
            ),
            (
                "preserve_nested_resolution",
                serde_json::json!(ae.preserve_nested_resolution),
            ),
            (
                "adaptive_limit",
                serde_json::json!(ae.motion_blur.as_ref().and_then(|m| m.adaptive_limit)),
            ),
        ]),
    }
}

/// The comp's shutter. Angle, phase and sample count all have Lumit
/// counterparts in the same units; AE's adaptive sample limit does not, and
/// rides in the `ae` namespace.
fn motion_blur(ae: &Comp) -> MotionBlur {
    let default = MotionBlur::default();
    let Some(mb) = ae.motion_blur.as_ref() else {
        return default;
    };
    MotionBlur {
        enabled: mb.enabled.unwrap_or(false),
        shutter_angle: mb.shutter_angle.unwrap_or(default.shutter_angle),
        shutter_phase: mb.shutter_phase.unwrap_or(default.shutter_phase),
        samples: mb.samples.unwrap_or(default.samples).max(2),
    }
}

/// After Effects reports a colour as three or four 0–1 numbers in the
/// project's display space; Lumit stores scene-linear light.
pub(crate) fn linear_colour(c: Option<&[f64]>) -> LinearColour {
    let c = c.unwrap_or(&[]);
    let ch = |i: usize| c.get(i).copied().unwrap_or(0.0);
    LinearColour([
        srgb_to_linear(ch(0)),
        srgb_to_linear(ch(1)),
        srgb_to_linear(ch(2)),
        c.get(3).copied().unwrap_or(1.0) as f32,
    ])
}

/// The sRGB transfer function, on a 0–1 float rather than
/// [`lumit_core::pixels::srgb_decode`]'s byte — After Effects hands colours
/// over as floats, and rounding them to a byte first would lose a shade.
pub(crate) fn srgb_to_linear(v: f64) -> f32 {
    let v = v.clamp(0.0, 1.0);
    let linear = if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    };
    linear as f32
}
