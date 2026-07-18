//! The document model, Phase 0 scope (docs/03-DATA-MODEL.md).
//!
//! Phase 0 carries projects, folders, footage items, compositions, and Footage
//! layers with spans — no properties/keyframes yet (slice arrives in Phase 1).
//! All mutation goes through operations (ops.rs); this module is data + queries.

use crate::anim::Property;
use crate::time::{CompTime, Duration, FrameRate};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Linear-light RGBA (docs/10-FILE-FORMAT.md §1.1).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LinearColour(pub [f32; 4]);

impl LinearColour {
    pub const BLACK: Self = Self([0.0, 0.0, 0.0, 1.0]);
}

/// Media reference (docs/03-DATA-MODEL.md §3). Fingerprint lands in slice 4.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaRef {
    pub relative_path: String,
    pub absolute_path: String,
    /// Unknown fields from newer Lumit versions, preserved on load/save
    /// (docs/10-FILE-FORMAT.md §1.1 — mandatory forward compatibility).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FootageItem {
    pub id: Uuid,
    pub name: String,
    pub media: MediaRef,
    /// Unknown fields from newer Lumit versions, preserved on load/save
    /// (docs/10-FILE-FORMAT.md §1.1 — mandatory forward compatibility).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// A shared solid definition (docs/03-DATA-MODEL.md §2): solids are assets,
/// so many layers can reference one colour/size and they dedupe naturally.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolidDef {
    pub id: Uuid,
    pub name: String,
    pub colour: LinearColour,
    pub width: u32,
    pub height: u32,
    /// Unknown fields from newer Lumit versions, preserved on load/save
    /// (docs/10-FILE-FORMAT.md §1.1 — mandatory forward compatibility).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub id: Uuid,
    pub name: String,
    /// Ordered children ids (docs/03-DATA-MODEL.md §2 table).
    pub children: Vec<Uuid>,
    /// Unknown fields from newer Lumit versions, preserved on load/save
    /// (docs/10-FILE-FORMAT.md §1.1 — mandatory forward compatibility).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Composition {
    pub id: Uuid,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    pub duration: Duration,
    pub background: LinearColour,
    /// Preview/export span (docs/01-GLOSSARY.md: work area); None = full comp.
    #[serde(default)]
    pub work_area: Option<(CompTime, CompTime)>,
    /// Index 0 = top of the stack.
    pub layers: Vec<Layer>,
    /// Timeline markers (cues, chapters, detected beats — docs/03-DATA-MODEL.md
    /// §11), in no required order (snapping and drawing sort as needed).
    #[serde(default)]
    pub markers: Vec<crate::markers::Marker>,
    /// Comp-wide motion-blur shutter (docs/06). Off by default; when on, only
    /// layers whose own `motion_blur` switch is set actually blur.
    #[serde(default)]
    pub motion_blur: MotionBlur,
    /// Unknown fields from newer Lumit versions, preserved on load/save
    /// (docs/10-FILE-FORMAT.md §1.1 — mandatory forward compatibility).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Comp-wide motion-blur settings (docs/06, K-120). Per-layer motion blur is a
/// cheap transform-sampled blur: with the comp master on, each layer whose own
/// `motion_blur` switch is set is drawn `samples` times across the open shutter,
/// its transform re-evaluated at each sub-frame time and the draws averaged, so
/// the layer smears along its own motion. The shutter *shape* is one comp
/// setting, exactly as in After Effects; the per-layer switch decides who blurs.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MotionBlur {
    /// Comp master enable. Off means nothing blurs, whatever the layer switches.
    pub enabled: bool,
    /// Shutter angle in degrees: the fraction of the frame interval the shutter
    /// is open is `shutter_angle / 360`. 180 (half a frame) is the AE default.
    pub shutter_angle: f64,
    /// Shutter phase in degrees: where the open interval sits relative to the
    /// frame time. -90 centres the blur on the frame (the AE default), pairing
    /// with a 180 angle to open a quarter-frame either side.
    pub shutter_phase: f64,
    /// Sub-frame samples per blurred layer across the open shutter (≥ 2 to blur;
    /// higher is smoother and more expensive). 16 is a tasteful default.
    pub samples: u32,
}

impl Default for MotionBlur {
    fn default() -> Self {
        Self {
            enabled: false,
            shutter_angle: 180.0,
            shutter_phase: -90.0,
            samples: 16,
        }
    }
}

impl MotionBlur {
    /// The docs/06 §4 hard ceiling on shutter samples (256). The UI clamps its
    /// own control to 2–64, but `samples` is plain saved data: a hand-edited or
    /// damaged project could carry any u32, and every offset returned here
    /// becomes a full draw of the layer per frame — so the budget is enforced
    /// where the offsets are made, not just at the control (docs/14 §5,
    /// budgeted work). Applied inside [`sample_offsets`], the one source both
    /// the render and the frame key read, so the two stay consistent.
    pub const MAX_SAMPLES: u32 = 256;

    /// The sub-frame sample offsets, in *frames*, across the open shutter
    /// (docs/06 §4, K-120). For `samples` = N the k-th midpoint offset is
    /// `phase_frac + (k + 0.5)/N · open_frac`, where `open_frac =
    /// shutter_angle/360` and `phase_frac = shutter_phase/360` — the shutter
    /// centres of N equal slices. A caller turns each offset into a comp-time
    /// sample by adding `t_comp + offset · dt` (dt = one frame in comp
    /// seconds). The AE defaults (angle 180, phase −90) give a window centred
    /// on the frame, spanning [−0.25, +0.25] frame.
    ///
    /// Empty unless the comp master is on and `samples` ≥ 2 (a single sample
    /// is no blur), so a caller can treat a non-empty result as "this comp
    /// blurs" without re-checking. `samples` is capped at [`Self::MAX_SAMPLES`]
    /// (the docs/06 §4 maximum), so a damaged file can never demand an
    /// unbounded number of sub-frame draws. Deterministic and side-effect
    /// free, so preview and export derive identical sample times from it
    /// (K-031).
    pub fn sample_offsets(&self) -> Vec<f64> {
        if !self.enabled || self.samples < 2 {
            return Vec::new();
        }
        let n = self.samples.min(Self::MAX_SAMPLES);
        let open_frac = self.shutter_angle / 360.0;
        let phase_frac = self.shutter_phase / 360.0;
        (0..n)
            .map(|k| phase_frac + (f64::from(k) + 0.5) / f64::from(n) * open_frac)
            .collect()
    }
}

/// Layer transform (docs/03-DATA-MODEL.md §6; 2.5D fields join with the
/// camera work — all maths is 4x4 from day one at the evaluator level).
/// Dimensions are separated scalars in Phase 1 (AE's separated-dimensions
/// mode); coupled spatial paths arrive with the motion-path unit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransformGroup {
    pub anchor_x: Property,
    pub anchor_y: Property,
    pub position_x: Property,
    pub position_y: Property,
    /// Percent, 100 = natural size.
    pub scale_x: Property,
    pub scale_y: Property,
    /// Degrees (z rotation — the 2D rotation).
    pub rotation: Property,
    /// 2.5D additions (K-023; serde-defaulted so pre-3D projects load).
    #[serde(default = "Property::zero")]
    pub position_z: Property,
    #[serde(default = "Property::zero")]
    pub rotation_x: Property,
    #[serde(default = "Property::zero")]
    pub rotation_y: Property,
    /// Percent, 0..100.
    pub opacity: Property,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for TransformGroup {
    fn default() -> Self {
        Self {
            anchor_x: Property::fixed(0.0),
            anchor_y: Property::fixed(0.0),
            position_x: Property::fixed(0.0),
            position_y: Property::fixed(0.0),
            scale_x: Property::fixed(100.0),
            scale_y: Property::fixed(100.0),
            rotation: Property::fixed(0.0),
            position_z: Property::fixed(0.0),
            rotation_x: Property::fixed(0.0),
            rotation_y: Property::fixed(0.0),
            opacity: Property::fixed(100.0),
            extra: serde_json::Map::new(),
        }
    }
}

/// Which transform property an op addresses (stable, serialisable path).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransformProp {
    AnchorX,
    AnchorY,
    PositionX,
    PositionY,
    PositionZ,
    ScaleX,
    ScaleY,
    Rotation,
    RotationX,
    RotationY,
    Opacity,
}

impl TransformGroup {
    pub fn get(&self, prop: TransformProp) -> &Property {
        match prop {
            TransformProp::AnchorX => &self.anchor_x,
            TransformProp::AnchorY => &self.anchor_y,
            TransformProp::PositionX => &self.position_x,
            TransformProp::PositionY => &self.position_y,
            TransformProp::ScaleX => &self.scale_x,
            TransformProp::ScaleY => &self.scale_y,
            TransformProp::Rotation => &self.rotation,
            TransformProp::PositionZ => &self.position_z,
            TransformProp::RotationX => &self.rotation_x,
            TransformProp::RotationY => &self.rotation_y,
            TransformProp::Opacity => &self.opacity,
        }
    }

    pub fn get_mut(&mut self, prop: TransformProp) -> &mut Property {
        match prop {
            TransformProp::AnchorX => &mut self.anchor_x,
            TransformProp::AnchorY => &mut self.anchor_y,
            TransformProp::PositionX => &mut self.position_x,
            TransformProp::PositionY => &mut self.position_y,
            TransformProp::ScaleX => &mut self.scale_x,
            TransformProp::ScaleY => &mut self.scale_y,
            TransformProp::Rotation => &mut self.rotation,
            TransformProp::PositionZ => &mut self.position_z,
            TransformProp::RotationX => &mut self.rotation_x,
            TransformProp::RotationY => &mut self.rotation_y,
            TransformProp::Opacity => &mut self.opacity,
        }
    }
}

/// Using another layer's alpha or luma to gate this layer
/// (docs/01-GLOSSARY.md §6: matte — any layer, one matte may serve many).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatteRef {
    pub layer: Uuid,
    pub channel: MatteChannel,
    pub inverted: bool,
    /// Whether the matte samples the source layer *after* its own effect stack
    /// (a keyed greenscreen, a blurred edge) rather than its raw source pixels.
    /// Default false — a matte reads source pixels only, matching the historical
    /// behaviour and letting the source's effects be irrelevant to the matte.
    /// When true the source's effects run into the matte texture before it gates
    /// the consumer (docs/impl/layer-input.md; K-decision). Temporal effects on
    /// the source (echo, flow motion blur) are not sub-sampled through a matte in
    /// v1 — a matte after-effects applies the source's spatial and colour stack.
    #[serde(default)]
    pub after_effects: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatteChannel {
    Alpha,
    Luma,
}

/// Where an effect implementation comes from (docs/03-DATA-MODEL.md §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectNamespace {
    /// Ships in the box (docs/08-EFFECTS.md tier lists).
    Builtin,
    /// An OpenFX plugin (docs/12-PLUGINS.md).
    Ofx,
    /// A native LFX plugin (docs/12-PLUGINS.md).
    Lfx,
    /// Unknown to this build (AE import or missing plugin): renders as
    /// identity with a badge, round-trips untouched.
    Placeholder,
}

/// Which effect an instance is: namespace + stable match name + version.
/// The version participates in the frame key (K-016), so changing an
/// effect's maths invalidates stale cached frames rather than mixing
/// generations (docs/08-EFFECTS.md §1.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectKey {
    pub namespace: EffectNamespace,
    pub match_name: String,
    pub version: u32,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// A file-valued parameter: the set of file paths it references plus a
/// hold-keyframed index that selects which one is live at a given time
/// (K-111). Two file paths cannot be blended, so the index only ever *steps*
/// (hold keyframes — see [`crate::anim::SideInterp::Hold`]); the common case is
/// a single path with a static index. An empty `paths` means unset, and the
/// consuming effect treats that as identity (a no-op) rather than erroring.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileParam {
    /// The distinct file paths this parameter references (usually exactly one).
    pub paths: Vec<String>,
    /// f64-valued selector into `paths`, animated with hold keyframes only.
    /// Rounded and clamped at evaluation, so it never lands between paths.
    pub index: Property,
}

impl FileParam {
    /// A single static path — the common, non-animated case.
    pub fn single(path: impl Into<String>) -> Self {
        Self {
            paths: vec![path.into()],
            index: Property::fixed(0.0),
        }
    }

    /// The unset parameter (no file chosen yet).
    pub fn empty() -> Self {
        Self {
            paths: Vec::new(),
            index: Property::fixed(0.0),
        }
    }

    /// The path live at layer time `lt` (seconds), or None when unset. The
    /// index is rounded and clamped into range, so a hold-keyframed index steps
    /// cleanly between paths and never selects a fraction of one.
    pub fn path_at(&self, lt: f64) -> Option<&str> {
        if self.paths.is_empty() {
            return None;
        }
        let last = (self.paths.len() - 1) as f64;
        let i = self.index.value_at(lt).round().clamp(0.0, last) as usize;
        self.paths.get(i).map(String::as_str)
    }
}

/// One effect parameter's value (docs/08-EFFECTS.md §1.2 types, v1 subset).
/// Floats, angles and percentages are all `Float`; points animate per axis;
/// colours animate per channel (scene-linear RGBA). Bool/Choice/Seed are
/// static in v1 — the tier-1 staples don't keyframe them. `File` carries a
/// path chosen from a dialog, animatable only by stepping (hold keys, K-111).
/// `Layer` references another layer as an auxiliary picture (a depth pass for
/// depth of field, docs/impl/layer-input.md), the same shape [`MatteRef`]
/// uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EffectValue {
    Float(Property),
    Point(Property, Property),
    Colour([Property; 4]),
    Bool(bool),
    Choice(u32),
    Seed(u32),
    File(FileParam),
    /// A reference to another layer in the same composition, sampled as an
    /// auxiliary input (a depth pass for depth of field, docs/impl/
    /// layer-input.md). `None` when unset; a `Some` id that no longer names a
    /// layer degrades to unset (a labelled no-op), never an error. Static in
    /// v1 — a layer reference does not keyframe.
    Layer(Option<Uuid>),
}

/// One named parameter on an effect instance. `id` is the stable snake_case
/// identifier (expressions address it; the UI shows the declared label).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectParam {
    pub id: String,
    pub value: EffectValue,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// One image operation in a layer's effect stack (docs/03-DATA-MODEL.md §8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectInstance {
    pub id: Uuid,
    pub effect: EffectKey,
    /// Individually bypassed effects render as identity (not animatable —
    /// docs/08 §1.5; the effect's own Mix parameter is the animatable dial).
    pub enabled: bool,
    /// Ordered as declared by the effect's schema.
    pub params: Vec<EffectParam>,
    /// Whether a temporal re-render effect (accumulation motion blur, Posterize
    /// Time — docs/impl/temporal-rerender.md) re-evaluates this effect at each
    /// sub-frame / held sample. Default true; set false to hold a stochastic or
    /// costly effect (a particle system) at the frame time instead of running
    /// it N times. Ignored unless a temporal re-render effect is sampling.
    #[serde(default = "default_true")]
    pub sample_temporally: bool,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl EffectInstance {
    /// The parameter named `id`, if the instance carries it.
    pub fn param(&self, id: &str) -> Option<&EffectValue> {
        self.params.iter().find(|p| p.id == id).map(|p| &p.value)
    }

    /// A float parameter's evaluated value at layer time `lt` (the common
    /// case), or None when absent or not a Float.
    pub fn float_at(&self, id: &str, lt: f64) -> Option<f64> {
        match self.param(id)? {
            EffectValue::Float(p) => Some(p.value_at(lt)),
            _ => None,
        }
    }

    /// A colour parameter's evaluated scene-linear RGBA at layer time `lt`
    /// (channels animate independently), or None when absent or not a
    /// Colour.
    pub fn colour_at(&self, id: &str, lt: f64) -> Option<[f64; 4]> {
        match self.param(id)? {
            EffectValue::Colour(ch) => Some([
                ch[0].value_at(lt),
                ch[1].value_at(lt),
                ch[2].value_at(lt),
                ch[3].value_at(lt),
            ]),
            _ => None,
        }
    }

    /// A bool parameter's value, or None when the parameter is absent or not a
    /// Bool. Bools are static in v1 (they do not keyframe), so there is no time
    /// argument — an absent flag (an older project saved before the parameter
    /// existed) reads as None, which callers treat as the default (false).
    pub fn bool_of(&self, id: &str) -> Option<bool> {
        match self.param(id)? {
            EffectValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// A file parameter's live path at layer time `lt` (the hold-keyframed
    /// index selects it), or None when the parameter is absent, not a File, or
    /// unset.
    pub fn path_at(&self, id: &str, lt: f64) -> Option<&str> {
        match self.param(id)? {
            EffectValue::File(f) => f.path_at(lt),
            _ => None,
        }
    }

    /// A layer-reference parameter's target id, or None when the parameter is
    /// absent, not a Layer, or unset (docs/impl/layer-input.md). The caller
    /// renders that layer alone at comp size and threads its texture to the
    /// effect (a depth pass for depth of field), the same way `path_at` feeds
    /// a LUT its file.
    pub fn layer_ref(&self, id: &str) -> Option<Uuid> {
        match self.param(id)? {
            EffectValue::Layer(l) => *l,
            _ => None,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Switches {
    pub visible: bool,
    pub audible: bool,
    pub locked: bool,
    /// 2.5D: this layer positions in z and honours the active camera.
    #[serde(default)]
    pub three_d: bool,
    /// Precomp layers only: collapse transformations (docs/06 §1.4). Inner
    /// layers composite straight into the parent with concatenated
    /// transforms — no intermediate raster, content resampled once. Certain
    /// conditions force an intermediate anyway; see [`collapse_state`].
    #[serde(default)]
    pub collapse: bool,
    /// The fx switch (docs/08 §1.5): off bypasses the layer's whole effect
    /// stack. Defaults on, so old projects load with effects live.
    #[serde(default = "default_true")]
    pub fx: bool,
    /// Solo / isolate (K-105): while any layer in the composition is soloed,
    /// only soloed layers render — a quick way to view one layer (or a few)
    /// against nothing. Off by default, so nothing changes until it is set.
    #[serde(default)]
    pub solo: bool,
    /// Per-layer motion blur (K-120): when set and the comp's motion-blur master
    /// is on, this layer is drawn across the open shutter and its transform
    /// samples averaged, smearing it along its own motion. Off by default.
    #[serde(default)]
    pub motion_blur: bool,
}

/// Whether any layer in `comp` is soloed (K-105). When true, the compositor
/// renders only the soloed layers. Shared by the preview and export paths so
/// they agree on what is visible.
pub fn any_solo(comp: &Composition) -> bool {
    comp.layers.iter().any(|l| l.switches.solo)
}

impl Default for Switches {
    fn default() -> Self {
        Self {
            visible: true,
            audible: true,
            locked: false,
            three_d: false,
            collapse: false,
            fx: true,
            solo: false,
            motion_blur: false,
        }
    }
}

/// What the collapse switch actually does for a layer at local time `lt`
/// (docs/06-RENDER-PIPELINE.md §1.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollapseState {
    /// Not a Precomp layer, or the switch is off: default nesting.
    Off,
    /// Collapsing: inner layers splice into the parent, transforms
    /// concatenated, no intermediate.
    Active,
    /// The switch is set but something forces an intermediate anyway (a mask,
    /// a non-Normal blend, opacity below 100%, or being consumed as a matte).
    /// Renders like Off; the UI dims the switch.
    Forced,
}

/// Evaluate the §1.4 collapse rules for `layer` inside `comp` at local time
/// `lt`. Beyond the layer's own mask/blend/opacity/effects and being matte
/// consumed, two inner conditions force: an inner layer using a matte (a
/// matte renders "alone into comp space", and splicing that across comps is
/// a later refinement — forcing keeps preview and export pixel-identical),
/// and an inner adjustment layer with a live stack (K-091: its effects
/// apply to the composite beneath it *within its own comp*, and splicing
/// would hand it the whole parent stack instead).
pub fn collapse_state(doc: &Document, comp: &Composition, layer: &Layer, lt: f64) -> CollapseState {
    let LayerKind::Precomp { comp: nested_id } = &layer.kind else {
        return CollapseState::Off;
    };
    if !layer.switches.collapse {
        return CollapseState::Off;
    }
    let inner_forces = doc.comp(*nested_id).is_some_and(|nested| {
        nested.layers.iter().any(|l| {
            l.switches.visible
                && (l.matte.is_some()
                    || (matches!(l.kind, LayerKind::Adjustment)
                        && l.switches.fx
                        && l.effects.iter().any(|e| e.enabled)))
        })
    });
    let forced = !layer.masks.is_empty()
        // §1.4: any live effect on the Precomp layer itself — its stack runs
        // on the nested comp's raster, which splicing never produces.
        || (layer.switches.fx && layer.effects.iter().any(|e| e.enabled))
        || layer.blend != BlendMode::Normal
        || layer.transform.opacity.value_at(lt) < 99.999
        || inner_forces
        || comp
            .layers
            .iter()
            .any(|l| l.matte.as_ref().is_some_and(|m| m.layer == layer.id));
    if forced {
        CollapseState::Forced
    } else {
        CollapseState::Active
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LayerKind {
    Footage {
        item: Uuid,
        /// Retime map (docs/04-RETIMING.md): local time → source time. None =
        /// no retiming (plays at source rate). Defaulted for projects saved
        /// before Retime existed.
        #[serde(default)]
        retime: Option<crate::retime::Retime>,
    },
    /// A SolidDef asset as this layer's source (docs/01-GLOSSARY.md: Solid
    /// layer; docs/03-DATA-MODEL.md §5.2 — solids are assets so they dedupe).
    Solid { def: Uuid },
    /// Another composition as this layer's source (docs/01-GLOSSARY.md:
    /// Precomp layer). Cycles are invalid states, guarded at insertion and
    /// defensively at render.
    Precomp { comp: Uuid },
    /// Editable styled text (v1: one run — docs/03-DATA-MODEL.md §9.1).
    Text { document: TextDocument },
    /// A 3D viewpoint (docs/01-GLOSSARY.md: Camera layer). Only affects
    /// layers with the 3D switch; the topmost visible camera is active.
    /// `zoom` is the AE model: focal distance in comp pixels — the z=0
    /// plane maps 1:1.
    Camera { zoom: Property },
    /// A Sequence layer (docs/01-GLOSSARY.md, §5.3): clips cut back-to-back on
    /// one row — Lumit's Vegas-style editing surface. Resolution lives in
    /// [`crate::sequence`].
    Sequence {
        #[serde(default)]
        clips: Vec<crate::sequence::Clip>,
    },
    /// An adjustment layer (docs/01-GLOSSARY.md): no source of its own — its
    /// masks and effect stack apply to the accumulated composite of every layer
    /// beneath it, within its span. A comp-sized container for effects.
    Adjustment,
}

/// The active camera's evaluated placement at one comp time — what both the
/// preview and the export pipeline hand to the GPU camera matrix, so the two
/// can never disagree (K-031).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraPose {
    /// Focal distance in comp pixels (the z=0 plane maps 1:1).
    pub zoom: f64,
    pub position: (f64, f64, f64),
    /// (x, y, z) rotation in degrees.
    pub rotation_deg: (f64, f64, f64),
}

impl Composition {
    /// The topmost visible Camera layer whose span contains `t`, evaluated at
    /// its layer time. None → the comp renders flat (3D switches ignored).
    pub fn camera_pose(&self, t: f64) -> Option<CameraPose> {
        self.layers.iter().find_map(|l| {
            let LayerKind::Camera { zoom } = &l.kind else {
                return None;
            };
            if !l.switches.visible || t < l.in_point.0.to_f64() || t >= l.out_point.0.to_f64() {
                return None;
            }
            let lt = t - l.start_offset.0.to_f64();
            let tr = &l.transform;
            Some(CameraPose {
                zoom: zoom.value_at(lt),
                position: (
                    tr.position_x.value_at(lt),
                    tr.position_y.value_at(lt),
                    tr.position_z.value_at(lt),
                ),
                rotation_deg: (
                    tr.rotation_x.value_at(lt),
                    tr.rotation_y.value_at(lt),
                    tr.rotation.value_at(lt),
                ),
            })
        })
    }
}

/// v1 text: single run. Styled runs, fonts and animators follow the doc.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextDocument {
    pub text: String,
    /// Pixel size at natural scale.
    pub size: f64,
    pub fill: LinearColour,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Per-layer composite operator — the linear subset first
/// (docs/06-RENDER-PIPELINE.md §blend domains; the perceptual set joins with
/// the ping-pong compositing pass).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BlendMode {
    #[default]
    Normal,
    Add,
    Multiply,
    Screen,
    Overlay,
    SoftLight,
    HardLight,
    Lighten,
    Darken,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: Uuid,
    pub name: String,
    pub kind: LayerKind,
    pub in_point: CompTime,
    /// Exclusive; must be > in_point.
    pub out_point: CompTime,
    /// Where layer time 0 sits on the comp timeline.
    pub start_offset: CompTime,
    /// Defaulted for projects saved before transforms existed (forward compat).
    #[serde(default)]
    pub transform: TransformGroup,
    /// Matte reference; a missing/deleted target degrades to "no matte"
    /// (docs/03-DATA-MODEL.md §5.1 invariants), never an error.
    #[serde(default)]
    pub matte: Option<MatteRef>,
    /// Parent layer (K-103): this layer's transform is applied *within* the
    /// parent's coordinate space, so moving or rotating the parent carries the
    /// child with it (After Effects parenting / null-object rigs). `None` = no
    /// parent, unchanged behaviour. A missing, deleted, or cyclic parent
    /// degrades to "no parent" at render time, never an error (same invariant
    /// as `matte`). Cycles are also rejected at edit time (`SetLayerParent`).
    #[serde(default)]
    pub parent: Option<Uuid>,
    #[serde(default)]
    pub blend: BlendMode,
    /// Masks gate the layer's alpha before effects/transform
    /// (docs/06-RENDER-PIPELINE.md render order).
    #[serde(default)]
    pub masks: Vec<crate::mask::Mask>,
    /// The ordered effect stack (docs/03 §8; applied top-to-bottom after
    /// masks, before transform — docs/06 render order).
    #[serde(default)]
    pub effects: Vec<EffectInstance>,
    pub switches: Switches,
    /// Unknown fields from newer Lumit versions, preserved on load/save
    /// (docs/10-FILE-FORMAT.md §1.1 — mandatory forward compatibility).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// The chain of parent layer ids above `layer` in `comp`, nearest first
/// (K-103). Stops at a layer with no parent or a parent not in the comp, and
/// breaks any cycle, so it always terminates and never repeats an id. Excludes
/// `layer` itself.
pub fn layer_parent_chain(comp: &Composition, layer: Uuid) -> Vec<Uuid> {
    let mut chain: Vec<Uuid> = Vec::new();
    let mut current = layer;
    // One hop per layer at most; a repeat would be a cycle, caught below.
    for _ in 0..comp.layers.len() {
        let Some(l) = comp.layers.iter().find(|l| l.id == current) else {
            break;
        };
        let Some(parent) = l.parent else {
            break;
        };
        if parent == layer || chain.contains(&parent) {
            break; // cycle
        }
        chain.push(parent);
        current = parent;
    }
    chain
}

/// Would pointing `layer`'s parent at `new_parent` form a cycle — either a
/// self-parent, or `layer` already being an ancestor of `new_parent`? Used to
/// reject a bad [`crate::Op::SetLayerParent`] before it lands. (Whether
/// `new_parent` exists in the comp is a separate check the op also makes.)
pub fn parenting_would_cycle(comp: &Composition, layer: Uuid, new_parent: Uuid) -> bool {
    new_parent == layer || layer_parent_chain(comp, new_parent).contains(&layer)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProjectItem {
    Footage(FootageItem),
    Folder(Folder),
    Composition(Composition),
    Solid(SolidDef),
}

impl ProjectItem {
    pub fn id(&self) -> Uuid {
        match self {
            ProjectItem::Footage(f) => f.id,
            ProjectItem::Folder(f) => f.id,
            ProjectItem::Composition(c) => c.id,
            ProjectItem::Solid(s) => s.id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            ProjectItem::Footage(f) => &f.name,
            ProjectItem::Folder(f) => &f.name,
            ProjectItem::Composition(c) => &c.name,
            ProjectItem::Solid(s) => &s.name,
        }
    }

    pub fn set_name(&mut self, name: String) {
        match self {
            ProjectItem::Footage(f) => f.name = name,
            ProjectItem::Folder(f) => f.name = name,
            ProjectItem::Composition(c) => c.name = name,
            ProjectItem::Solid(s) => s.name = name,
        }
    }
}

/// The folders Lumit files new assets into automatically: the first solid
/// creates a "Solids" folder, the first comp a "Compositions" folder, and
/// later ones follow the folder by id — so renaming or nesting the folder
/// doesn't break the habit. A deleted folder is simply recreated on next use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AutoFolders {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solids: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compositions: Option<Uuid>,
}

/// The whole editable document (docs/01-GLOSSARY.md: Project).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub id: Uuid,
    /// Flat item storage; Project panel order = Vec order, folders reference by id.
    pub items: Vec<ProjectItem>,
    /// Where new solids/comps are filed (see [`AutoFolders`]).
    #[serde(default)]
    pub auto_folders: AutoFolders,
    /// Unknown fields from newer Lumit versions, preserved on load/save
    /// (docs/10-FILE-FORMAT.md §1.1 — mandatory forward compatibility).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Document {
    pub fn new() -> Self {
        Self {
            id: Uuid::now_v7(),
            items: Vec::new(),
            auto_folders: AutoFolders::default(),
            extra: serde_json::Map::new(),
        }
    }

    pub fn item(&self, id: Uuid) -> Option<&ProjectItem> {
        self.items.iter().find(|i| i.id() == id)
    }

    pub fn item_mut(&mut self, id: Uuid) -> Option<&mut ProjectItem> {
        self.items.iter_mut().find(|i| i.id() == id)
    }

    pub fn comp(&self, id: Uuid) -> Option<&Composition> {
        match self.item(id) {
            Some(ProjectItem::Composition(c)) => Some(c),
            _ => None,
        }
    }

    pub fn comp_mut(&mut self, id: Uuid) -> Option<&mut Composition> {
        match self.item_mut(id) {
            Some(ProjectItem::Composition(c)) => Some(c),
            _ => None,
        }
    }

    pub fn solid(&self, id: Uuid) -> Option<&SolidDef> {
        match self.item(id) {
            Some(ProjectItem::Solid(s)) => Some(s),
            _ => None,
        }
    }

    pub fn folder(&self, id: Uuid) -> Option<&Folder> {
        match self.item(id) {
            Some(ProjectItem::Folder(f)) => Some(f),
            _ => None,
        }
    }

    /// Ids that sit at the Project panel root: every item not referenced as
    /// any folder's child (missing children are ignored, never an error).
    pub fn root_items(&self) -> Vec<Uuid> {
        let mut in_folder = std::collections::HashSet::new();
        for item in &self.items {
            if let ProjectItem::Folder(f) = item {
                in_folder.extend(f.children.iter().copied());
            }
        }
        self.items
            .iter()
            .map(|i| i.id())
            .filter(|id| !in_folder.contains(id))
            .collect()
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::time::{CompTime, Rational};

    fn secs(s: i64) -> CompTime {
        CompTime(Rational::new(s, 1).unwrap())
    }

    #[test]
    fn effect_instance_sample_temporally_defaults_true() {
        // An effect saved before the temporal-rerender flag existed loads with
        // it on (docs/10 §1.1 forward compat), so old projects behave as before.
        let e = crate::fx::instantiate("blur").unwrap();
        assert!(e.sample_temporally);
        let mut v = serde_json::to_value(&e).unwrap();
        v.as_object_mut().unwrap().remove("sample_temporally");
        let back: EffectInstance = serde_json::from_value(v).unwrap();
        assert!(back.sample_temporally);
    }

    #[test]
    fn matte_ref_after_effects_defaults_false() {
        // A matte saved before the after-effects toggle existed loads source-only
        // (the historical behaviour), so old projects render unchanged.
        let m = MatteRef {
            layer: Uuid::now_v7(),
            channel: MatteChannel::Alpha,
            inverted: false,
            after_effects: false,
        };
        let mut v = serde_json::to_value(m).unwrap();
        v.as_object_mut().unwrap().remove("after_effects");
        let back: MatteRef = serde_json::from_value(v).unwrap();
        assert!(!back.after_effects);
    }

    #[test]
    fn file_param_steps_by_its_hold_keyed_index() {
        use crate::anim::{Animation, Keyframe, SideInterp};

        // Unset: no path.
        assert_eq!(FileParam::empty().path_at(0.0), None);

        // Single static path: always that path, at any time.
        let one = FileParam::single("look.cube");
        assert_eq!(one.path_at(0.0), Some("look.cube"));
        assert_eq!(one.path_at(99.0), Some("look.cube"));

        // Two paths, index hold-keyed 0 -> 1 at t = 2 s: the path holds until
        // the key, then steps, and never lands between the two.
        let hold = |t: i64, v: f64| Keyframe {
            time: Rational::new(t, 1).unwrap(),
            value: v,
            interp_in: SideInterp::Hold,
            interp_out: SideInterp::Hold,
        };
        let anim = FileParam {
            paths: vec!["a.cube".into(), "b.cube".into()],
            index: Property {
                animation: Animation::Keyframed(vec![hold(0, 0.0), hold(2, 1.0)]),
                extra: serde_json::Map::new(),
            },
        };
        assert_eq!(anim.path_at(0.0), Some("a.cube"));
        assert_eq!(anim.path_at(1.9), Some("a.cube")); // held right up to the key
        assert_eq!(anim.path_at(2.0), Some("b.cube")); // steps exactly at the key
        assert_eq!(anim.path_at(50.0), Some("b.cube")); // and stays

        // A fractional or out-of-range index rounds to the nearest path and
        // clamps into range — never an index panic.
        let frac = |v: f64| FileParam {
            paths: vec!["a.cube".into(), "b.cube".into()],
            index: Property::fixed(v),
        };
        assert_eq!(frac(0.4).path_at(0.0), Some("a.cube"));
        assert_eq!(frac(0.6).path_at(0.0), Some("b.cube"));
        assert_eq!(frac(9.0).path_at(0.0), Some("b.cube")); // clamp above
        assert_eq!(frac(-3.0).path_at(0.0), Some("a.cube")); // clamp below
    }

    #[test]
    fn motion_blur_defaults_and_forward_compat() {
        // The AE-style defaults: off, half-frame shutter centred on the frame.
        let mb = MotionBlur::default();
        assert!(!mb.enabled);
        assert_eq!(mb.shutter_angle, 180.0);
        assert_eq!(mb.shutter_phase, -90.0);
        assert_eq!(mb.samples, 16);
        // A comp saved before motion blur existed (no `motion_blur` key) loads
        // with the default rather than failing (docs/10 §1.1 forward compat).
        // Build a real comp, strip the key, and confirm it re-loads defaulted.
        let mut v = serde_json::to_value(comp_with_cameras()).unwrap();
        v.as_object_mut().unwrap().remove("motion_blur");
        let comp: Composition = serde_json::from_value(v).unwrap();
        assert_eq!(comp.motion_blur, MotionBlur::default());
        // And a layer without the `motion_blur` switch defaults it off.
        assert!(!Switches::default().motion_blur);
    }

    #[test]
    fn motion_blur_sample_offsets_are_centred_and_span_the_shutter() {
        // Off, or fewer than two samples, is no blur (empty offsets).
        assert!(MotionBlur::default().sample_offsets().is_empty());
        let mut one = MotionBlur {
            enabled: true,
            samples: 1,
            ..MotionBlur::default()
        };
        assert!(one.sample_offsets().is_empty());
        one.samples = 0;
        assert!(one.sample_offsets().is_empty());

        // AE defaults (angle 180, phase −90) with N=4: four slice centres of
        // the half-frame window, symmetric about the frame time (0).
        let mb = MotionBlur {
            enabled: true,
            shutter_angle: 180.0,
            shutter_phase: -90.0,
            samples: 4,
        };
        let offs = mb.sample_offsets();
        assert_eq!(offs.len(), 4);
        // open_frac = 0.5, phase_frac = −0.25 → −0.25 + (k+0.5)/4·0.5.
        let expect = [-0.1875, -0.0625, 0.0625, 0.1875];
        for (got, want) in offs.iter().zip(expect) {
            assert!((got - want).abs() < 1e-12, "{got} vs {want}");
        }
        // Centred: the mean offset is the frame time, and the set is symmetric.
        let mean: f64 = offs.iter().sum::<f64>() / offs.len() as f64;
        assert!(mean.abs() < 1e-12, "mean {mean}");
        for (lo, hi) in offs.iter().zip(offs.iter().rev()) {
            assert!((lo + hi).abs() < 1e-12);
        }
        // The window spans exactly the open shutter (angle/360 of a frame).
        let span = offs.last().unwrap() - offs.first().unwrap();
        let slice = 0.5 / 4.0; // one sample sits half a slice in from each edge
        assert!((span - (0.5 - slice)).abs() < 1e-12, "span {span}");
    }

    #[test]
    fn motion_blur_sample_count_is_capped_at_the_docs_maximum() {
        // `samples` is plain saved data (the UI clamps its own control, a
        // hand-edited file need not), and each offset becomes a full draw of
        // the layer per frame — so the docs/06 §4 maximum (256) is enforced in
        // sample_offsets itself, the one source both the render and the frame
        // key read. A damaged file asking for millions of samples gets the
        // capped, still-centred window instead of an unbounded draw list.
        let mb = MotionBlur {
            enabled: true,
            samples: 1_000_000,
            ..MotionBlur::default()
        };
        let offs = mb.sample_offsets();
        assert_eq!(offs.len(), MotionBlur::MAX_SAMPLES as usize);
        // Still the centred AE-default window: mean at the frame time.
        let mean: f64 = offs.iter().sum::<f64>() / offs.len() as f64;
        assert!(mean.abs() < 1e-12, "mean {mean}");
        // At or below the cap nothing changes.
        let at_cap = MotionBlur {
            enabled: true,
            samples: MotionBlur::MAX_SAMPLES,
            ..MotionBlur::default()
        };
        assert_eq!(at_cap.sample_offsets().len(), 256);
    }

    #[test]
    fn file_param_serde_round_trips() {
        let fp = FileParam::single("C:/luts/teal-orange.cube");
        let json = serde_json::to_string(&fp).unwrap();
        assert_eq!(fp, serde_json::from_str::<FileParam>(&json).unwrap());

        // And wrapped in an EffectValue (the shape projects save/load).
        let ev = EffectValue::File(fp);
        let ev_json = serde_json::to_string(&ev).unwrap();
        assert_eq!(ev, serde_json::from_str::<EffectValue>(&ev_json).unwrap());
    }

    fn comp_with_cameras() -> Composition {
        let mut comp = Composition {
            id: Uuid::now_v7(),
            name: "cam test".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(60, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour([0.0, 0.0, 0.0, 1.0]),
            work_area: None,
            layers: Vec::new(),
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        let cam = |name: &str, zoom: f64, z_pos: f64, visible: bool, in_s: i64, out_s: i64| Layer {
            id: Uuid::now_v7(),
            name: name.into(),
            kind: LayerKind::Camera {
                zoom: Property::fixed(zoom),
            },
            in_point: secs(in_s),
            out_point: secs(out_s),
            start_offset: secs(0),
            transform: TransformGroup {
                position_z: Property::fixed(z_pos),
                ..TransformGroup::default()
            },
            matte: None,
            parent: None,
            blend: BlendMode::Normal,
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches {
                visible,
                ..Switches::default()
            },
            extra: serde_json::Map::new(),
        };
        comp.layers.push(cam("hidden", 500.0, -10.0, false, 0, 10));
        comp.layers.push(cam("short", 800.0, -20.0, true, 2, 4));
        comp.layers.push(cam("main", 1200.0, -30.0, true, 0, 10));
        comp
    }

    /// The §1.4 collapse rules: Off for non-precomps and unset switches,
    /// Active for a clean collapsed Precomp, Forced by a mask, a non-Normal
    /// blend, sub-100 opacity, or being consumed as a matte.
    #[test]
    fn collapse_state_follows_the_force_rules() {
        let doc = Document::new();
        let mut comp = comp_with_cameras();
        let nested = Uuid::now_v7();
        let mut pre = comp.layers[0].clone();
        pre.id = Uuid::now_v7();
        pre.kind = LayerKind::Precomp { comp: nested };
        pre.switches.visible = true;
        pre.switches.collapse = true;
        pre.blend = BlendMode::Normal;
        pre.masks.clear();
        pre.transform = TransformGroup::default();
        comp.layers.push(pre.clone());

        // Clean collapsed Precomp → Active.
        assert_eq!(
            collapse_state(&doc, &comp, &pre, 1.0),
            CollapseState::Active
        );
        // Switch off → Off; non-Precomp kinds are always Off.
        let mut off = pre.clone();
        off.switches.collapse = false;
        assert_eq!(collapse_state(&doc, &comp, &off, 1.0), CollapseState::Off);
        assert_eq!(
            collapse_state(&doc, &comp, &comp.layers[0], 1.0),
            CollapseState::Off
        );
        // Each §1.4 force: mask, blend, opacity, matte consumption.
        let mut masked = pre.clone();
        masked
            .masks
            .push(crate::mask::Mask::rectangle(0.0, 0.0, 1.0, 1.0));
        assert_eq!(
            collapse_state(&doc, &comp, &masked, 1.0),
            CollapseState::Forced
        );
        // §1.4: a live effect stack on the Precomp layer itself forces —
        // splicing has no nested-comp raster for the stack to run on. The
        // fx switch or disabling every effect lifts it.
        let mut effected = pre.clone();
        effected
            .effects
            .push(crate::fx::instantiate("blur").unwrap());
        assert_eq!(
            collapse_state(&doc, &comp, &effected, 1.0),
            CollapseState::Forced
        );
        effected.switches.fx = false;
        assert_eq!(
            collapse_state(&doc, &comp, &effected, 1.0),
            CollapseState::Active
        );
        effected.switches.fx = true;
        effected.effects[0].enabled = false;
        assert_eq!(
            collapse_state(&doc, &comp, &effected, 1.0),
            CollapseState::Active
        );
        let mut blended = pre.clone();
        blended.blend = BlendMode::Add;
        assert_eq!(
            collapse_state(&doc, &comp, &blended, 1.0),
            CollapseState::Forced
        );
        let mut faded = pre.clone();
        faded.transform.opacity = Property::fixed(50.0);
        assert_eq!(
            collapse_state(&doc, &comp, &faded, 1.0),
            CollapseState::Forced
        );
        let mut consumer = comp.layers[0].clone();
        consumer.id = Uuid::now_v7();
        consumer.matte = Some(MatteRef {
            layer: pre.id,
            channel: MatteChannel::Alpha,
            inverted: false,
            after_effects: false,
        });
        let mut comp2 = comp.clone();
        comp2.layers.push(consumer);
        assert_eq!(
            collapse_state(&doc, &comp2, &pre, 1.0),
            CollapseState::Forced
        );
        // An inner layer consuming a matte forces too (export-parity rule).
        let mut inner_matted = comp_with_cameras();
        let mut inner = inner_matted.layers[0].clone();
        inner.id = Uuid::now_v7();
        inner.kind = LayerKind::Text {
            document: TextDocument {
                text: "m".into(),
                size: 12.0,
                fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
                extra: serde_json::Map::new(),
            },
        };
        inner.switches.visible = true;
        inner.matte = Some(MatteRef {
            layer: inner_matted.layers[0].id,
            channel: MatteChannel::Alpha,
            inverted: false,
            after_effects: false,
        });
        inner_matted.layers.push(inner);
        let nested_real_id = inner_matted.id;
        let mut doc2 = Document::new();
        doc2.items.push(ProjectItem::Composition(inner_matted));
        let mut pre2 = pre.clone();
        pre2.kind = LayerKind::Precomp {
            comp: nested_real_id,
        };
        assert_eq!(
            collapse_state(&doc2, &comp, &pre2, 1.0),
            CollapseState::Forced
        );
    }

    /// K-091: an inner adjustment layer with a live effect stack forces the
    /// intermediate — its effects apply to the composite beneath it within
    /// its own comp, and splicing would hand it the parent stack instead.
    /// A bypassed stack (fx switch off, or every effect disabled) collapses
    /// normally.
    #[test]
    fn an_inner_live_adjustment_layer_forces_the_intermediate() {
        let mut inner_comp = comp_with_cameras();
        let mut adj = inner_comp.layers[0].clone();
        adj.id = Uuid::now_v7();
        adj.kind = LayerKind::Adjustment;
        adj.switches.visible = true;
        adj.effects
            .push(crate::fx::instantiate("saturation").unwrap());
        inner_comp.layers.push(adj);
        let nested_id = inner_comp.id;
        let mut doc = Document::new();
        doc.items.push(ProjectItem::Composition(inner_comp));

        let comp = comp_with_cameras();
        let mut pre = comp.layers[0].clone();
        pre.id = Uuid::now_v7();
        pre.kind = LayerKind::Precomp { comp: nested_id };
        pre.switches.visible = true;
        pre.switches.collapse = true;
        pre.blend = BlendMode::Normal;
        pre.masks.clear();
        pre.transform = TransformGroup::default();
        assert_eq!(
            collapse_state(&doc, &comp, &pre, 1.0),
            CollapseState::Forced
        );

        // Bypass the stack both ways: each restores Active.
        let with = |edit: &dyn Fn(&mut Layer)| {
            let mut doc = Document::new();
            let mut inner_comp = comp_with_cameras();
            let mut adj = inner_comp.layers[0].clone();
            adj.id = Uuid::now_v7();
            adj.kind = LayerKind::Adjustment;
            adj.switches.visible = true;
            adj.effects
                .push(crate::fx::instantiate("saturation").unwrap());
            edit(&mut adj);
            let nested_id = inner_comp.id;
            inner_comp.layers.push(adj);
            doc.items.push(ProjectItem::Composition(inner_comp));
            let mut pre = pre.clone();
            pre.kind = LayerKind::Precomp { comp: nested_id };
            collapse_state(&doc, &comp, &pre, 1.0)
        };
        assert_eq!(
            with(&|l| l.switches.fx = false),
            CollapseState::Active,
            "fx switch off must not force"
        );
        assert_eq!(
            with(&|l| l.effects[0].enabled = false),
            CollapseState::Active,
            "a fully disabled stack must not force"
        );
        assert_eq!(
            with(&|l| l.switches.visible = false),
            CollapseState::Active,
            "a hidden adjustment layer must not force"
        );
    }

    /// The topmost visible in-span camera wins; hidden and out-of-span ones
    /// never do; no camera at all → None (flat comp).
    #[test]
    fn camera_pose_picks_topmost_visible_in_span() {
        let comp = comp_with_cameras();
        // t=1: "hidden" is invisible, "short" not yet in span → "main".
        let pose = comp.camera_pose(1.0).unwrap();
        assert_eq!(pose.zoom, 1200.0);
        assert_eq!(pose.position.2, -30.0);
        // t=3: "short" is topmost visible in-span.
        let pose = comp.camera_pose(3.0).unwrap();
        assert_eq!(pose.zoom, 800.0);
        assert_eq!(pose.position.2, -20.0);
        // Out point is exclusive.
        assert_eq!(comp.camera_pose(4.0).unwrap().zoom, 1200.0);
        // No cameras → flat.
        let mut flat = comp_with_cameras();
        flat.layers.clear();
        assert!(flat.camera_pose(1.0).is_none());
    }

    #[test]
    fn parent_chain_walks_up_and_cycles_are_detected() {
        let mut comp = comp_with_cameras();
        let (a, b, c) = (comp.layers[0].id, comp.layers[1].id, comp.layers[2].id);
        // No parents yet: empty chains, but a self-parent is still a cycle.
        assert!(layer_parent_chain(&comp, c).is_empty());
        assert!(parenting_would_cycle(&comp, a, a));
        // Build a <- b <- c (b parented to a, c parented to b).
        comp.layers[1].parent = Some(a);
        comp.layers[2].parent = Some(b);
        assert_eq!(layer_parent_chain(&comp, b), vec![a]);
        assert_eq!(layer_parent_chain(&comp, c), vec![b, a]);
        // a may not adopt b or c (they descend from a) — that would loop.
        assert!(parenting_would_cycle(&comp, a, b));
        assert!(parenting_would_cycle(&comp, a, c));
        // But c re-parenting straight to a is fine (still a DAG upward).
        assert!(!parenting_would_cycle(&comp, c, a));
    }

    #[test]
    fn set_layer_parent_op_round_trips_and_rejects_bad_parents() {
        use crate::ops::{apply, Op, OpError};
        let comp = comp_with_cameras();
        let (a, b) = (comp.layers[0].id, comp.layers[1].id);
        let comp_id = comp.id;
        let mut doc = Document::new();
        doc.items.push(ProjectItem::Composition(comp));

        // Parenting b to a, then undoing with the returned inverse.
        let set = Op::SetLayerParent {
            comp: comp_id,
            layer: b,
            parent: Some(a),
        };
        let inv = apply(&mut doc, &set).expect("valid parent applies");
        assert_eq!(doc.comp(comp_id).unwrap().layers[1].parent, Some(a));
        assert_eq!(
            inv,
            Op::SetLayerParent {
                comp: comp_id,
                layer: b,
                parent: None
            }
        );
        apply(&mut doc, &inv).expect("inverse applies");
        assert_eq!(doc.comp(comp_id).unwrap().layers[1].parent, None);

        // With b parented to a again, a→b is a cycle; self and unknown also fail.
        apply(&mut doc, &set).unwrap();
        let cycle = Op::SetLayerParent {
            comp: comp_id,
            layer: a,
            parent: Some(b),
        };
        assert_eq!(apply(&mut doc, &cycle), Err(OpError::InvalidParent));
        let self_parent = Op::SetLayerParent {
            comp: comp_id,
            layer: a,
            parent: Some(a),
        };
        assert_eq!(apply(&mut doc, &self_parent), Err(OpError::InvalidParent));
        let unknown = Op::SetLayerParent {
            comp: comp_id,
            layer: a,
            parent: Some(Uuid::now_v7()),
        };
        assert_eq!(apply(&mut doc, &unknown), Err(OpError::InvalidParent));
    }

    #[test]
    fn solo_op_round_trips_and_any_solo_reports() {
        use crate::ops::{apply, Op};
        let mut comp = comp_with_cameras();
        let a = comp.layers[0].id;
        assert!(!any_solo(&comp), "nothing soloed to start");
        comp.layers[0].switches.solo = true;
        assert!(any_solo(&comp));
        comp.layers[0].switches.solo = false;

        let comp_id = comp.id;
        let mut doc = Document::new();
        doc.items.push(ProjectItem::Composition(comp));
        let inv = apply(
            &mut doc,
            &Op::SetLayerSolo {
                comp: comp_id,
                layer: a,
                solo: true,
            },
        )
        .unwrap();
        assert!(doc.comp(comp_id).unwrap().layers[0].switches.solo);
        assert!(any_solo(doc.comp(comp_id).unwrap()));
        assert_eq!(
            inv,
            Op::SetLayerSolo {
                comp: comp_id,
                layer: a,
                solo: false
            }
        );
        apply(&mut doc, &inv).unwrap();
        assert!(!doc.comp(comp_id).unwrap().layers[0].switches.solo);
    }
}
