//! The document model, Phase 0 scope (docs/03-DATA-MODEL.md).
//!
//! Phase 0 carries projects, folders, footage items, compositions, and Footage
//! layers with spans — no properties/keyframes yet (slice arrives in Phase 1).
//! All mutation goes through operations (ops.rs); this module is data + queries.

use std::sync::Arc;

use crate::anim::Property;
use crate::expression::ExpressionContext;
use crate::time::{CompTime, Duration, FrameRate, Rational};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Linear-light RGBA (docs/10-FILE-FORMAT.md §1.1).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LinearColour(pub [f32; 4]);

impl LinearColour {
    pub const BLACK: Self = Self([0.0, 0.0, 0.0, 1.0]);
}

/// A content fingerprint for a media file (docs/10-FILE-FORMAT.md §2): file
/// size, last-modified time, and a hash of the head and tail bytes. Used by the
/// relink resolver's step 3 to recognise a moved or renamed file by its content
/// rather than its path. This is the stored *data*; lumit-project computes it
/// from a file on disk (`fingerprint_path`).
///
/// The hash samples the head and tail rather than the whole file — cheap even
/// for multi-gigabyte footage, and enough to tell distinct captures apart. It
/// is a relink *heuristic*, not a cryptographic identity: two files that differ
/// only deep in the middle can collide, which is why relink stays advisory
/// (path first, and the resolver confirms candidates before adopting them).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    /// File length in bytes.
    pub size: u64,
    /// Last-modified time, whole seconds since the Unix epoch (advisory: a copy
    /// keeps the content but may not keep the mtime, so matching ignores it).
    pub mtime_secs: i64,
    /// Lowercase hex blake3 of `size ++ head ++ tail` (see the type note).
    pub head_tail_hash: String,
}

impl Fingerprint {
    /// Whether two fingerprints likely denote the same file *content*. Size and
    /// the head/tail hash must agree; mtime is ignored, so a file copied or
    /// moved to a new location (fresh mtime) still matches its original.
    #[must_use]
    pub fn likely_same_content(&self, other: &Fingerprint) -> bool {
        self.size == other.size && self.head_tail_hash == other.head_tail_hash
    }
}

/// Media reference (docs/03-DATA-MODEL.md §3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaRef {
    /// Path relative to the project file's directory — the one path a saved
    /// project carries (docs/10 §2, K-173). Rebased against the project's
    /// location on every save.
    pub relative_path: String,
    /// The file's location on THIS machine, this session. Never serialized
    /// (K-173): an absolute path embeds the local username — the exact thing
    /// docs/10 §2 promises the file never contains — and the tester sharing
    /// a project found theirs inside. Projects saved before K-173 still
    /// carry one, so it still *deserializes* and serves as the resolver's
    /// step-2 fallback; it simply never gets written again.
    #[serde(default, skip_serializing)]
    pub absolute_path: String,
    /// Content fingerprint for path-independent relink (docs/10 §2). Optional:
    /// absent in projects saved before fingerprints, and skipped on save when
    /// unset, so those files round-trip byte-for-byte unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<Fingerprint>,
    /// Unknown fields from newer Lumit versions, preserved on load/save
    /// (docs/10-FILE-FORMAT.md §1.1 — mandatory forward compatibility).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl MediaRef {
    /// The path to *show* for this reference — the Project panel's Path column
    /// (docs/07 §3.1, docs/15 §12A.3a).
    ///
    /// The relative path, which is the one a saved project actually carries
    /// (K-173), falling back to the absolute path when there is no relative one
    /// — a project that has never been saved has nothing to be relative to, and
    /// showing the file's real location then is more use than showing nothing.
    ///
    /// Display data, and only that: it says where the reference *points*, not
    /// whether anything is there. Resolving it against the project folder and
    /// deciding whether the file exists is [`crate::model`]'s business no more
    /// than probing is — `lumit_project::resolve_media` answers that.
    #[must_use]
    pub fn display_path(&self) -> &str {
        if self.relative_path.is_empty() {
            &self.absolute_path
        } else {
            &self.relative_path
        }
    }
}

/// A footage item that is a folder of numbered stills rather than one file
/// (docs/03-DATA-MODEL.md §3, K-539).
///
/// Deliberately just the rate. Which files are in the run, where it starts and
/// how long it is are re-read from the folder every time it is opened, because
/// the files on disk are the truth about a sequence and a saved copy of them
/// would only ever be a stale one. The rate is the exception: stills carry no
/// frame rate of their own, so it is the one thing about a sequence nobody but
/// the project can say.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceRef {
    /// The rate the run plays at. Defaults to 25 on import (K-539).
    pub frame_rate: FrameRate,
    /// Unknown fields from newer Lumit versions, preserved on load/save
    /// (docs/10-FILE-FORMAT.md §1.1 — mandatory forward compatibility).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for SequenceRef {
    fn default() -> Self {
        Self {
            frame_rate: FrameRate::FPS_25,
            extra: serde_json::Map::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FootageItem {
    pub id: Uuid,
    pub name: String,
    pub media: MediaRef,
    /// The colour space this footage arrives in, by the loaded config's own
    /// name (K-490, docs/impl/ocio.md §3.1).
    ///
    /// `None` — the usual case — means the built-in interpretation defaults:
    /// video is Rec.709, stills are sRGB, and the container's own metadata
    /// wins where it says anything (docs/06 §3.2). A name is **kept even when
    /// the config that defined it is missing**: a name is the user's statement
    /// about the file, and losing it because a path moved would be a silent
    /// edit of their project.
    ///
    /// It is a field on the item rather than a map beside the items — unlike a
    /// label it changes pixels, and unlike a proxy it applies to exactly one
    /// item kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colour_space: Option<String>,
    /// Set when this item is an image sequence: `media` then names *a* file of
    /// the numbered run — usually the first — and the item's frames are the
    /// run's files in numeric order (K-539).
    ///
    /// Absent, and skipped on save, for the ordinary one-file case, so every
    /// project written before sequences existed round-trips byte for byte.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<SequenceRef>,
    /// Unknown fields from newer Lumit versions, preserved on load/save
    /// (docs/10-FILE-FORMAT.md §1.1 — mandatory forward compatibility).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl FootageItem {
    /// The rate to read a numbered run of stills at, or `None` when this item
    /// is one ordinary file (K-539).
    ///
    /// The exact pair, never `fps()`: a rate that goes through a float does not
    /// come back (docs/14 §2). This is what everything that opens media builds
    /// its `lumit_media::MediaSource` from — the engine crates cannot see
    /// `lumit-media`, so the pair is what crosses.
    #[must_use]
    pub fn sequence_fps(&self) -> Option<(u32, u32)> {
        self.sequence
            .as_ref()
            .map(|s| (s.frame_rate.num(), s.frame_rate.den()))
    }
}

/// A footage item's **proxy**: a second media reference standing in for the
/// original while you work (docs/03-DATA-MODEL.md §3a).
///
/// # In plain terms
///
/// A proxy is a small, cheap copy of a piece of footage — usually half size —
/// that the Viewer decodes instead of the real thing, so scrubbing a folder of
/// 6K clips feels like scrubbing a folder of small ones. The original is
/// swapped back in for delivery, so the file you send is never the small one.
///
/// It is a *second* reference beside [`FootageItem::media`], never a
/// replacement: the original's path, size, rate and duration remain the
/// item's own, and everything the picture is laid out by keeps coming from
/// them. Only the pixels are read elsewhere.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyRef {
    /// Where the stand-in file lives — path pair and fingerprint, resolved and
    /// relinked exactly like the original's reference.
    pub media: MediaRef,
    /// This item's own *use proxy* switch. Off keeps the proxy attached and
    /// reads the original, which is how one clip is checked at full quality
    /// without giving up the proxy that took minutes to make.
    #[serde(default = "default_true")]
    pub enabled: bool,
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
    /// **Layer groups** (K-702, [`crate::group`]): named folds over runs of
    /// [`Self::layers`], drawn as a header row in the Timeline's outline with
    /// its members indented beneath it.
    ///
    /// Purely organisational — the render walk never reads this, and a comp
    /// renders identically whether its layers are grouped, ungrouped, folded
    /// or open. **Precompose** is the collapse that *does* change the picture,
    /// and stays the separate thing it was; a group's own menu offers it.
    ///
    /// A list beside the layers rather than a mark on each one, so a group is
    /// invisible to every path that reads a layer to draw it — see the module
    /// docs for why, and for what happens to a member that is deleted or
    /// dragged out of the run (it leaves the group, quietly, with nothing to
    /// repair).
    // Skipped while empty so a project saved before groups existed re-saves
    // byte-identical (docs/10 §1.1; the round-trip test pins it).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<crate::group::LayerGroup>,
    /// Timeline markers (cues, chapters, detected beats — docs/03-DATA-MODEL.md
    /// §11), in no required order (snapping and drawing sort as needed).
    #[serde(default)]
    pub markers: Vec<crate::markers::Marker>,
    /// Comp-wide motion-blur shutter (docs/06). Off by default; when on, only
    /// layers whose own `motion_blur` switch is set actually blur.
    #[serde(default)]
    pub motion_blur: MotionBlur,
    /// **The master fader**, in dB (docs/09 §3.1, K-691): one gain stage on
    /// the summed mix, ahead of the safety limiter. 0 is unity; −100 and
    /// below is exact silence, the same −∞ knee a layer's Volume has.
    ///
    /// A *stage*, not a per-layer multiplier, and that distinction is the
    /// whole reason it is here rather than folded into each layer's gain.
    /// Folding would give the same samples — multiplication distributes over
    /// the sum — but it would make every strip's meter read post-fader, and a
    /// strip's bar means "how loud is this layer", not "how loud has the
    /// master left it". Pulling the master down must move one bar.
    ///
    /// Project data, not a setting: it is part of what the composition
    /// sounds like, so it saves, undoes, and exports. A plain number rather
    /// than a [`Property`]: the board draws a fader and a value, and an
    /// automated master is the Composer's problem, not v1's. A nested comp's
    /// own master rides on the Precomp layer as a carrier, exactly as that
    /// layer's Volume does.
    #[serde(default)]
    pub master_volume_db: f64,
    /// The **confirmed beat grid** (docs/09 §5, K-698): the tempo and phase
    /// the last beat detection ran its grid at, kept so the Timeline's beat
    /// band can number bars without re-running the analysis. `None` until a
    /// detection with a grid lands, and cleared with the generated markers.
    ///
    /// Project data rather than panel state because the bar numbers are a
    /// reading of the *document* — reopening a cut-to-the-grid project must
    /// show the same bars it was cut against.
    // Skipped while `None` so a project saved before the field existed
    // re-saves byte-identical (K-040's quiet half; the round-trip test
    // pins it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beat_grid: Option<BeatGrid>,
    /// Unknown fields from newer Lumit versions, preserved on load/save
    /// (docs/10-FILE-FORMAT.md §1.1 — mandatory forward compatibility).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// A confirmed tempo grid over a composition's timeline (docs/09 §5, K-698):
/// beats at `bpm`, the first of them `phase` seconds in. Bars are the grid
/// read four beats at a time — v1 assumes common time, which is what the
/// scene's material is in.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BeatGrid {
    /// Beats per minute. Always positive: a grid with no tempo is `None`
    /// on the composition, never a zero here.
    pub bpm: f64,
    /// Where beat zero falls, in comp seconds — the detection's phase nudge.
    pub phase: crate::time::Rational,
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
    /// How each two-axis property is shown and edited (K-571, docs/03 §6.5).
    /// Absent in every project written before separate axes existed, which is
    /// exactly the default — so old files load unchanged, and a project that
    /// has never been separated writes nothing.
    #[serde(default, skip_serializing_if = "AxisModes::is_default")]
    pub axis_modes: AxisModes,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// How one two-axis transform property is shown and edited (K-571).
///
/// The axes are always stored as separate scalar properties (§6.1) — this says
/// nothing about storage and everything about the rows the panels draw and how
/// an edit to one axis reaches the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AxisMode {
    /// One row, one value box, and an edit carries the other axis with it so
    /// the x:y ratio holds. Scale's default; meaningless on the other pairs.
    Linked,
    /// One row, one box per axis, each edited on its own. One stopwatch, so
    /// keying the row keys every axis in it.
    Combined,
    /// A row per axis, each with its own stopwatch, keyframes and graph curve.
    Separated,
}

/// The mode of each pair that has one. Anchor point and Position start
/// `Combined`; Scale starts `Linked`, because a scale that stops being
/// proportional is nearly always a mistake rather than an intention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AxisModes {
    pub anchor: AxisMode,
    pub position: AxisMode,
    pub scale: AxisMode,
}

impl Default for AxisModes {
    fn default() -> Self {
        Self {
            anchor: AxisMode::Combined,
            position: AxisMode::Combined,
            scale: AxisMode::Linked,
        }
    }
}

impl AxisModes {
    /// Nothing to write: every pair is where a fresh layer leaves it.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn get(&self, pair: TransformPair) -> AxisMode {
        match pair {
            TransformPair::Anchor => self.anchor,
            TransformPair::Position => self.position,
            TransformPair::Scale => self.scale,
        }
    }

    pub fn set(&mut self, pair: TransformPair, mode: AxisMode) {
        match pair {
            TransformPair::Anchor => self.anchor = mode,
            TransformPair::Position => self.position = mode,
            TransformPair::Scale => self.scale = mode,
        }
    }
}

/// Which multi-axis transform property an axis-mode edit names (K-571).
///
/// Rotation is one angle and Opacity one number, so neither is here: a pair is
/// exactly a property whose axes could be told apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransformPair {
    Anchor,
    Position,
    Scale,
}

impl TransformPair {
    /// The properties this pair covers, in row order. Position's z is one of
    /// them: a separated Position on a 3D layer is three rows, not two — and on
    /// a 2D layer the z row is not drawn at all, exactly as it is not today.
    pub fn props(self) -> &'static [TransformProp] {
        match self {
            Self::Anchor => &[TransformProp::AnchorX, TransformProp::AnchorY],
            Self::Position => &[
                TransformProp::PositionX,
                TransformProp::PositionY,
                TransformProp::PositionZ,
            ],
            Self::Scale => &[TransformProp::ScaleX, TransformProp::ScaleY],
        }
    }
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
            axis_modes: AxisModes::default(),
            extra: serde_json::Map::new(),
        }
    }
}

/// Which transform property an op addresses (stable, serialisable path).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    /// The animations a pair's axes need in order to be shown on **one** row
    /// again (K-571): every animated axis gains a key wherever any other
    /// animated axis in the pair has one, so a diamond on the recombined row
    /// means the same thing on every axis under it.
    ///
    /// Exact, not resampled: the planted key takes the value the curve already
    /// had there and the span it lands in is re-described around it
    /// ([`Property::insert_key_preserving_shape`]), so the picture does not
    /// move. A **static** axis is left alone — a constant needs no keys to stay
    /// constant, and keying it would light a stopwatch nobody asked for.
    ///
    /// Returns only the axes that actually changed, so a recombine of a pair
    /// with nothing to merge is a single mode edit.
    pub fn unified_axes(
        &self,
        pair: TransformPair,
    ) -> Vec<(TransformProp, crate::anim::Animation)> {
        let mut times: Vec<Rational> = Vec::new();
        for prop in pair.props() {
            if let crate::anim::Animation::Keyframed(keys) = &self.get(*prop).animation {
                times.extend(keys.iter().map(|k| k.time));
            }
        }
        let mut out = Vec::new();
        for prop in pair.props() {
            let mut property = self.get(*prop).clone();
            if !property.is_animated() {
                continue;
            }
            let mut changed = false;
            for t in &times {
                changed |= property.insert_key_preserving_shape(*t);
            }
            if changed {
                out.push((*prop, property.animation));
            }
        }
        out
    }

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

/// How a layer-input parameter samples the layer it references (K-142,
/// revising K-125's two-way "after effects" bool). Applies uniformly to a
/// track matte's source ([`MatteRef`]) and to an effect's Layer-reference
/// input (the Depth of field depth layer, docs/impl/layer-input.md):
/// - `None` — the referenced layer's **raw** footage/solid only: no masks,
///   no effects (the rawest input a consumer can read).
/// - `Masks` — the source **with its own masks** applied, but not its effects.
/// - `EffectsAndMasks` — the source **with its effects and masks** (K-125's
///   `after_effects = true`): a keyed greenscreen matte, a graded depth pass.
///
/// Temporal effects on the source (echo, flow motion blur, a nested depth
/// reference) are still not sub-sampled through a layer input in v1 — the
/// spatial and colour stack applies, but an echo/flow effect degrades to a
/// still (the documented K-125 boundary, unchanged by K-142).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LayerInputSource {
    /// Raw source pixels only — no masks, no effects.
    None,
    /// Source plus its own masks, but not its effects.
    Masks,
    /// Source with its effects and masks (K-125's `after_effects = true`).
    /// The default (owner K-142 follow-up): the most complete source is what a
    /// new matte/depth input should sample unless the user narrows it.
    #[default]
    EffectsAndMasks,
}

impl LayerInputSource {
    /// Migrate K-125's boolean: `true` (after effects) → `EffectsAndMasks`;
    /// `false` → `Masks`. The historical source-only path (`after_effects =
    /// false`) already applied the source's masks, so `Masks` — not `None` — is
    /// its faithful mapping. A matte with neither field migrates to the default
    /// (`EffectsAndMasks`) via [`MatteRefRepr`], not through this function.
    pub fn from_after_effects(after_effects: bool) -> Self {
        if after_effects {
            Self::EffectsAndMasks
        } else {
            Self::Masks
        }
    }

    /// From a `Choice` param's index (0 = None, 1 = Masks, 2 = Effects and
    /// masks); any other value falls back to the default.
    pub fn from_choice(v: u32) -> Self {
        match v {
            1 => Self::Masks,
            2 => Self::EffectsAndMasks,
            _ => Self::None,
        }
    }

    /// This mode's `Choice` index (0/1/2), for storing on an effect parameter.
    pub fn to_choice(self) -> u32 {
        match self {
            Self::None => 0,
            Self::Masks => 1,
            Self::EffectsAndMasks => 2,
        }
    }

    /// A stable byte for the frame cache key: switching modes must retire stale
    /// frames, so the discriminant joins the key (0/1/2).
    pub fn key_byte(self) -> u8 {
        self.to_choice() as u8
    }

    /// Whether the referenced layer's own masks gate the sampled input.
    pub fn applies_masks(self) -> bool {
        !matches!(self, Self::None)
    }

    /// Whether the referenced layer's own effect stack runs into the input.
    pub fn folds_effects(self) -> bool {
        matches!(self, Self::EffectsAndMasks)
    }
}

/// Using another layer's alpha or luma to gate this layer
/// (docs/01-GLOSSARY.md §6: matte — any layer, one matte may serve many).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "MatteRefRepr")]
pub struct MatteRef {
    pub layer: Uuid,
    pub channel: MatteChannel,
    pub inverted: bool,
    /// How the matte samples its source layer (K-142): raw source (`None`),
    /// source + masks (`Masks`), or the source's processed picture
    /// (`EffectsAndMasks` — a keyed or blurred matte). Replaces K-125's
    /// `after_effects` bool; old projects migrate through [`MatteRefRepr`]
    /// (`true` → `EffectsAndMasks`, `false` → `Masks`). New inputs default to
    /// `EffectsAndMasks`.
    #[serde(default)]
    pub source: LayerInputSource,
}

/// Deserialisation shim for [`MatteRef`] that accepts both the current
/// `source: LayerInputSource` field and K-125's legacy `after_effects: bool`,
/// so saved projects still load (K-142). When `source` is present it wins;
/// otherwise the legacy bool is migrated (`true` → `EffectsAndMasks`, `false`
/// → `Masks`); a matte with neither field takes the default. New projects
/// always serialise `source`.
#[derive(Deserialize)]
struct MatteRefRepr {
    layer: Uuid,
    channel: MatteChannel,
    #[serde(default)]
    inverted: bool,
    #[serde(default)]
    source: Option<LayerInputSource>,
    #[serde(default)]
    after_effects: Option<bool>,
}

impl From<MatteRefRepr> for MatteRef {
    fn from(r: MatteRefRepr) -> Self {
        // `source` wins; else migrate the legacy bool; else the default
        // (`EffectsAndMasks`) for a matte that predates both fields.
        let source = r
            .source
            .or_else(|| r.after_effects.map(LayerInputSource::from_after_effects))
            .unwrap_or_default();
        MatteRef {
            layer: r.layer,
            channel: r.channel,
            inverted: r.inverted,
            source,
        }
    }
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
    /// An **audio plugin** — a CLAP one today, a VST3 one on the same road in
    /// AP4 (docs/impl/audio-plugins.md, K-700).
    ///
    /// One namespace for both standards on purpose: both collapse into one
    /// internal definition and nothing downstream of describe knows which a
    /// plugin speaks. Which it is, where anybody needs to know, is carried by
    /// the match name's own prefix — the same place the OFX provenance is
    /// carried, and for the same reason: the host minted the name.
    ///
    /// It is its own namespace rather than [`EffectNamespace::Ofx`]'s
    /// neighbour-by-accident because the picture path filters on exactly this:
    /// an audio effect must never be resolved as an image op, and saying so
    /// once here is cheaper than every walk asking the catalogue what kind of
    /// thing it just found.
    Clap,
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
    /// A reference to one of the **owning layer's masks**, whose *geometry* an
    /// effect walks (K-408, docs/08 §1.2): the mask id, or `None` for the
    /// "First mask" entry, which resolves to whichever mask is first at render
    /// time. A `Some` id that no longer names a mask on the layer degrades to
    /// the effect's no-op, never an error. Static in v1, exactly as a layer
    /// reference is — the *shape* animates (the mask's own path keyframes),
    /// which mask is named does not.
    ///
    /// Deliberately not an [`EffectValue::Layer`] holding a mask id: the two
    /// are different things, and the walks that look for referenced layers —
    /// the frame key's, the decode planner's — would each have to learn which
    /// ids were secretly masks.
    MaskPath(Option<Uuid>),
    /// A tone curve, as its own control points (K-412): an ordered list of
    /// 2..=16 `[x, y]` pairs in the unit square, the identity diagonal
    /// `[[0, 0], [1, 1]]` by default.
    ///
    /// Static in v1, exactly as [`EffectValue::File`], [`EffectValue::Layer`]
    /// and [`EffectValue::MaskPath`] are: a list that grows and shrinks has no
    /// interpolation between two keyframes, which is why After Effects' own
    /// curve blob steps rather than animating.
    ///
    /// Stored as written and straightened on read
    /// ([`crate::fx::CurvePoints::sanitised`]) rather than on write, so a
    /// project a hand or an importer edited opens and renders instead of being
    /// rejected.
    Curve(Vec<[f32; 2]>),
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
    /// The user's own name for this instance, shown in place of the effect's
    /// label wherever the stack is drawn (K-321) — "Blur the sign", not
    /// "Gaussian blur". `None` (the default, and what every older project
    /// deserialises to) shows the label; rendering, expressions and every
    /// `match_name` lookup are untouched by it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_name: Option<String>,
    /// Which of this effect's **vector pairs** are linked (K-443), by the
    /// pair's stem — `light` for `light_x` / `light_y`, as
    /// [`EffectSchema::pairs`](crate::fx::EffectSchema::pairs) names them.
    ///
    /// # In plain terms
    ///
    /// A point is drawn as two wells with a chain between them. Closed, dragging
    /// one number moves the other with it; open, the two are independent. Which
    /// way the chain is set is the user's choice about *this* effect on *this*
    /// layer, so it is saved with the document — like a custom name, and unlike
    /// a value it never animates, has no keyframes and is read by no kernel.
    ///
    /// **Empty means every pair is unlinked**, which is exactly what a project
    /// written before the flag existed deserialises to and exactly how it
    /// behaved: two numbers that moved on their own. So the field is
    /// `#[serde(default)]` and is left out of the file when empty, and no
    /// format version moves (K-258 — an untouched document saves back the same
    /// bytes).
    ///
    /// **The proportional edit itself is not here.** Scaling y as x is dragged
    /// is what the panel does with a linked pair while the gesture is live; the
    /// document's business is only *which* pairs are tied together. Kept sorted
    /// so two documents that were given the same links save identically,
    /// whatever order the toggles were clicked in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub linked_pairs: Vec<String>,
    /// An **audio plugin's own memory of itself**, as hex (K-700,
    /// docs/impl/audio-plugins.md §4).
    ///
    /// # In plain terms
    ///
    /// A plugin's knobs are Lumit properties and keyframe like anything else,
    /// but a plugin also keeps things no knob names — which impulse response a
    /// reverb loaded, what a curve display was drawn as. It hands that over as
    /// a run of bytes it alone understands. Lumit **never parses it**: the blob
    /// is written into the project, handed back to the plugin when the layer
    /// opens again, and that is the whole of the contract. A plugin that is no
    /// longer installed keeps its blob, its rows and its keyframes, so
    /// installing it again finds everything where it was (docs/12 §1).
    ///
    /// Hex rather than raw bytes because the `.lum` is pretty-printed JSON, and
    /// a `Vec<u8>` there is one number per line.
    /// ponytail: base64 if a vendor's blob ever makes the doubling matter —
    /// it is a string either way, so the field does not move.
    ///
    /// **Nothing in Lumit writes one yet.** v1 is parameters-only (§6): no
    /// plugin GUI, so nothing but a state load changes plugin state, and a
    /// round-trip is the whole of what the format owes. The plugin's own
    /// floating window — the package after AP5 — is what makes reading the
    /// blob back off a live instance necessary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_state: Option<String>,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl EffectInstance {
    /// The plugin state blob as bytes, or `None` for the overwhelmingly
    /// ordinary instance that has none.
    ///
    /// Hex that will not decode answers `None` — a hand-edited or truncated
    /// blob opens the project without it rather than refusing the project
    /// (14-ENGINEERING-RULES §4).
    #[must_use]
    pub fn plugin_state_bytes(&self) -> Option<Vec<u8>> {
        hex::decode(self.plugin_state.as_ref()?).ok()
    }

    /// Write a plugin's blob onto this instance.
    pub fn set_plugin_state(&mut self, bytes: &[u8]) {
        self.plugin_state = (!bytes.is_empty()).then(|| hex::encode(bytes));
    }

    /// Whether the vector pair named by `stem` is linked (K-443). Unknown to
    /// this instance means unlinked, which is every pair of every older project.
    #[must_use]
    pub fn pair_linked(&self, stem: &str) -> bool {
        self.linked_pairs.iter().any(|s| s == stem)
    }

    /// Link or unlink the vector pair named by `stem`, and answer whether that
    /// changed anything — `false` when the pair was already that way, which is
    /// the caller's cue not to commit an op that would undo to itself.
    ///
    /// Takes any stem, including one this effect has no pair for: the
    /// declaration is what the panel offers a chain for, and a document that
    /// names a pair a later build removed is a stale line to ignore, never a
    /// fault (14-ENGINEERING-RULES §4).
    pub fn set_pair_linked(&mut self, stem: &str, linked: bool) -> bool {
        match (self.pair_linked(stem), linked) {
            (true, true) | (false, false) => false,
            (false, true) => {
                self.linked_pairs.push(stem.to_owned());
                self.linked_pairs.sort();
                true
            }
            (true, false) => {
                self.linked_pairs.retain(|s| s != stem);
                true
            }
        }
    }

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

    pub fn float_at_with_context(
        &self,
        id: &str,
        lt: f64,
        context: Arc<ExpressionContext>,
    ) -> Option<f64> {
        match self.param(id)? {
            EffectValue::Float(p) => Some(p.value_at_with_context(lt, context)),
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

    pub fn colour_at_with_context(
        &self,
        id: &str,
        lt: f64,
        context: Arc<ExpressionContext>,
    ) -> Option<[f64; 4]> {
        match self.param(id)? {
            EffectValue::Colour(ch) => Some([
                ch[0].value_at_with_context(lt, context.clone()),
                ch[1].value_at_with_context(lt, context.clone()),
                ch[2].value_at_with_context(lt, context.clone()),
                ch[3].value_at_with_context(lt, context.clone()),
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

    /// A mask-path parameter's named mask id, or `None` when the parameter is
    /// absent, not a mask path, or on the "First mask" entry (K-408). `None`
    /// is not "no mask": which mask it comes to is
    /// [`crate::mask::mask_path_at`]'s answer, and depends on the schema's
    /// `self_default` and on what the layer actually carries.
    pub fn mask_ref(&self, id: &str) -> Option<Uuid> {
        match self.param(id)? {
            EffectValue::MaskPath(m) => *m,
            _ => None,
        }
    }

    /// How a Layer-reference parameter `id` samples its source (K-142): the
    /// `<id>_source` Choice param if present (the current form, written by the
    /// inspector combobox), else the legacy `<id>_after_effects` bool (K-125:
    /// `true` → `EffectsAndMasks`, `false` → `Masks`), else the default
    /// (`EffectsAndMasks`). Reading
    /// the legacy bool lets a project saved with the old checkbox keep its
    /// behaviour without a migration pass over the effect stack.
    pub fn layer_source(&self, id: &str) -> LayerInputSource {
        if let Some(EffectValue::Choice(v)) = self.param(&format!("{id}_source")) {
            return LayerInputSource::from_choice(*v);
        }
        if let Some(b) = self.bool_of(&format!("{id}_after_effects")) {
            return LayerInputSource::from_after_effects(b);
        }
        LayerInputSource::default()
    }
}

fn default_true() -> bool {
    true
}

/// `skip_serializing_if` for a field whose default is `true`: an untouched
/// project writes no line for it, so older files round-trip unchanged.
fn is_true(b: &bool) -> bool {
    *b
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
    /// Shy (docs/07 §4.2): hidden from the Timeline's layer list while the
    /// comp's shy filter is on. Pure list housekeeping — it never changes what
    /// renders, which is why the evaluator does not read it.
    #[serde(default)]
    pub shy: bool,
    /// Guide (K-497): the layer is *for reference only*. The Viewer draws it
    /// like any other layer; a walk that produces a file skips it, wherever it
    /// sits — including inside a precomp rendered into a parent — and
    /// regardless of solo. Off by default, so old projects deliver unchanged.
    #[serde(default)]
    pub guide: bool,
    /// Accepts lights (K-361): the layer is shaded by the comp's Light layers.
    /// Defaults on, so placing a light lights the scene without hunting for a
    /// switch — but a comp with no lights shades nothing either way, so 2D
    /// montage work pays nothing for the default.
    #[serde(default = "default_true")]
    pub accepts_lights: bool,
}

impl Layer {
    /// Whether this layer is a Light (K-360) — asked often enough by the
    /// lighting path that it is worth a name rather than a `matches!` at each
    /// call site.
    #[must_use]
    pub fn is_light(&self) -> bool {
        matches!(self.kind, LayerKind::Light { .. })
    }

    /// Whether this layer acts as an adjustment layer (K-537) — the one answer
    /// every picture path asks, so the [`Layer::adjustment`] flag and the
    /// legacy [`LayerKind::Adjustment`] take a single path and cannot drift.
    ///
    /// True means: ignore this layer's own source, composite everything below
    /// it, and run this layer's effect stack on that.
    #[must_use]
    pub fn is_adjustment(&self) -> bool {
        self.adjustment || matches!(self.kind, LayerKind::Adjustment)
    }

    /// Whether the adjustment switch means anything on this layer (K-537):
    /// **any layer that shows something in the Viewer**.
    ///
    /// A Camera is a viewpoint, a Light is something other layers see, a Null
    /// is a transform and nothing else, and an Audio layer (K-435) is sound —
    /// none of the four has a picture to set aside, so none of them can grade
    /// what is under it either. Everything else can, including a layer whose
    /// own visibility switch is off: hiding a layer and making it an adjustment
    /// are two separate answers, and the switch is drawn either way.
    #[must_use]
    pub fn can_adjust(&self) -> bool {
        !self.audio_only
            && !matches!(
                self.kind,
                LayerKind::Camera { .. } | LayerKind::Light { .. } | LayerKind::Null
            )
    }
}

/// Whether any layer in `comp` is soloed (K-105). When true, the compositor
/// renders only the soloed layers. Shared by the preview and export paths so
/// they agree on what is visible.
pub fn any_solo(comp: &Composition) -> bool {
    comp.layers.iter().any(|l| l.switches.solo)
}

/// Whether any layer that **draws** is soloed (K-435) — the question the
/// picture asks, where [`any_solo`] is the one the mixer asks.
///
/// **In plain terms.** Solo means "just this one". Soloing a music track means
/// just that sound; it cannot sensibly mean an empty picture, because the track
/// has no picture to show. So the two halves count solos separately: the mixer
/// counts every soloed layer, and the compositor counts only the layers that
/// could have been on screen. Solo an Audio layer and the picture does not
/// notice — which is also what keeps its switches out of the frame key.
pub fn any_picture_solo(comp: &Composition) -> bool {
    comp.layers.iter().any(|l| l.switches.solo && !l.audio_only)
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
            shy: false,
            guide: false,
            accepts_lights: true,
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
                    || (l.is_adjustment() && l.switches.fx && l.effects.iter().any(|e| e.enabled)))
        })
    });
    let forced = !layer.masks.is_empty()
        // Paint is stamped into the layer's own raster (K-227), which splicing
        // a collapsed precomp never produces.
        || !layer.paint.is_empty()
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
    /// One footage item as the layer's source. Retiming lives on the layer
    /// itself ([`Layer::retime`]), not here: this variant carried a second,
    /// rival retime store until K-249 deleted it, and a document written
    /// before that is converted on open (the `0.1.0` → `0.2.0` migration in
    /// `lumit-project`).
    Footage { item: Uuid },
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
    Camera {
        zoom: Property,
        /// The **solve link** (K-417, docs/03 §5.6): the id of a layer whose
        /// camera solve drives this camera, in the same composition.
        ///
        /// **In plain terms.** A tracked shot knows where the real camera was
        /// on every frame. Rather than copying that into keyframes the moment
        /// it is solved, the Camera layer *points at* the tracked layer and
        /// derives its placement per frame — so re-solving, re-trimming or
        /// re-timing that layer moves the camera with it, and nothing has to
        /// be re-baked. While the link is set the camera's own transform and
        /// zoom are the **correction lane** (K-578): what they hold over and
        /// above [`Self::Camera::correction_base`] is added to the solved pose,
        /// so the shot can be tracked once and nudged afterwards.
        /// **Convert to keyframes** ([`crate::track::bake_solve_link`]) turns
        /// the corrected motion into ordinary keys and severs the link.
        ///
        /// The named layer is the one the analysis was run on — or a Precomp
        /// layer that contains it, which is how the owner's precomp workflow
        /// resolves (K-417's parent-comp ruling). `None` is an ordinary camera
        /// the user drives by hand, which is every camera today.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        solve_link: Option<Uuid>,
        /// The **zero of the correction lane** (K-578): the pose this camera's
        /// own properties held at the moment the link was made.
        ///
        /// **In plain terms.** A linked camera's rows keep showing its own
        /// numbers, and dragging one is allowed — but a linked camera is not
        /// *at* those numbers, it is at the solve. So the rows are read as a
        /// **difference** from where they started, and that difference is added
        /// to the solved pose. This is where they started.
        ///
        /// Without it the same numbers would have to mean an absolute pose (the
        /// fallback a lost link falls back to) and an offset (the correction) at
        /// once, which they cannot: a camera created at the comp's centre would
        /// read as a correction of half a comp. `None` — on a camera with no
        /// link, or one from a project written before this existed — means
        /// there is no correction lane, and the solve is followed exactly.
        ///
        /// Boxed for the reason [`Self::Light`]'s definition is: seven `f64`
        /// would make Camera the widest variant, and every layer of every
        /// composition would carry the width.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        correction_base: Option<Box<CameraPose>>,
    },
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
    /// Vector art as the layer's own picture (docs/03-DATA-MODEL.md §7.2,
    /// K-237): one or more paths, each with a fill and a stroke, drawn at
    /// whatever resolution the frame is rendered at.
    ///
    /// The paths are `mask::BezierPath` — the same path type a mask uses, and
    /// deliberately so: a shape layer's path and a mask's path differ in what
    /// they *do*, not in what they are.
    Shape {
        #[serde(default)]
        contents: Vec<crate::shape::ShapeItem>,
    },
    /// A Null layer (docs/01-GLOSSARY.md): an invisible layer with no source
    /// and no size, carrying only a transform, so other layers can be parented
    /// to it and moved as a rig. It never draws. Masks and effects can be added
    /// to it — nothing enforces otherwise — but with no pixels to act on they
    /// never run, the same as on a Camera layer.
    Null,
    /// A Light layer (K-360, docs/03-DATA-MODEL.md §5.5): a source of light in
    /// the composition. Like a Camera it draws no pixels of its own — it is
    /// something other layers *see* — and like a Camera it carries its
    /// placement in the ordinary layer transform, so it animates, parents and
    /// is dragged with everything already built for that.
    ///
    /// The **area** kind is the one that earns the layer: a rectangle of a real
    /// width and height, rather than a point pretending to be one. What reads
    /// it first is the Lens flare's Lights source mode (K-257's reserved
    /// option), where a light with real extent flares as its own shape rather
    /// than as a dot — the machinery K-355 already built for detected sources.
    ///
    /// Boxed because eight animatable [`Property`] channels make [`LightDef`]
    /// several times the size of any other layer kind, and every layer in
    /// every composition would otherwise pay for it.
    Light {
        #[serde(default)]
        light: Box<LightDef>,
    },
}

/// What kind of light a [`LayerKind::Light`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LightKind {
    /// A point of light with no extent — the classic bare bulb.
    #[default]
    Point,
    /// A cone, aimed by the layer's own rotation.
    Spot,
    /// A rectangle of a real size: a softbox, a window, a strip light. The
    /// kind a compositor actually reaches for, and the reason [`LightDef`]
    /// carries a width and a height at all.
    Area,
}

/// A Light layer's own properties (K-360). The placement is NOT here — it is
/// the layer's ordinary transform, so a light animates and parents exactly as
/// every other layer does.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LightDef {
    #[serde(default)]
    pub kind: LightKind,
    /// Scene-linear RGB. Animatable per channel, like every colour in the
    /// model.
    #[serde(default = "LightDef::white")]
    pub colour: [Property; 3],
    /// Master gain on the light's contribution.
    #[serde(default = "LightDef::one")]
    pub intensity: Property,
    /// The emitting rectangle's half-width and half-height in comp pixels —
    /// meaningful for [`LightKind::Area`], ignored by the other two. Half
    /// rather than full, so it matches the Lens flare's Source size dials
    /// (K-355) and a light can be measured from its centre outward.
    #[serde(default = "LightDef::zero_pair")]
    pub half_size: [Property; 2],
    /// The spot cone's half-angle in degrees; ignored unless the kind is
    /// [`LightKind::Spot`].
    #[serde(default = "LightDef::default_cone")]
    pub cone_deg: Property,
    /// How far the light reaches before it has fallen to nothing, in comp
    /// pixels. Zero means no falloff at all — the light does not weaken with
    /// distance, which is what a compositor usually wants from a flare source.
    #[serde(default = "Property::zero")]
    pub falloff_px: Property,
}

impl LightDef {
    fn one() -> Property {
        Property::fixed(1.0)
    }
    fn white() -> [Property; 3] {
        [Self::one(), Self::one(), Self::one()]
    }
    fn zero_pair() -> [Property; 2] {
        [Property::zero(), Property::zero()]
    }
    fn default_cone() -> Property {
        Property::fixed(45.0)
    }
}

impl Default for LightDef {
    fn default() -> Self {
        Self {
            kind: LightKind::default(),
            colour: Self::white(),
            intensity: Self::one(),
            half_size: Self::zero_pair(),
            cone_deg: Self::default_cone(),
            falloff_px: Property::zero(),
        }
    }
}

/// One Light layer resolved at a comp time — what the renderer hands to the
/// effects that read lights (K-360). Positions are comp pixels, matching every
/// other px@comp quantity the render path carries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedLight {
    pub kind: LightKind,
    /// Centre, in comp pixels.
    pub position: (f64, f64),
    /// Scene-linear RGB, already multiplied by intensity.
    pub colour: [f32; 3],
    /// Half-width and half-height in comp pixels; zero for a point.
    pub half_size: (f64, f64),
    /// The spot cone's half-angle in degrees, and the layer's own rotation
    /// about z, which is what aims it.
    pub cone_deg: f64,
    pub rotation_deg: f64,
    pub falloff_px: f64,
    /// Depth in comp pixels, and the two out-of-plane rotations (K-361). The
    /// Lens flare ignores these — it works in the projected picture, where a
    /// light is wherever it lands. Shading needs them: a rectangle sitting in
    /// the same plane as the surface it lights is edge-on and throws nothing,
    /// so a softbox that does anything at all is a softbox in front.
    pub z: f64,
    pub rotation_x_deg: f64,
    pub rotation_y_deg: f64,
}

/// The active camera's evaluated placement at one comp time — what both the
/// preview and the export pipeline hand to the GPU camera matrix, so the two
/// can never disagree (K-031).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CameraPose {
    /// Focal distance in comp pixels (the z=0 plane maps 1:1).
    pub zoom: f64,
    pub position: (f64, f64, f64),
    /// (x, y, z) rotation in degrees.
    pub rotation_deg: (f64, f64, f64),
}

/// One Camera layer's placement at comp time `t`, from its own properties.
/// `None` when `layer` is not a Camera.
#[must_use]
pub fn stored_camera_pose(layer: &Layer, t: f64) -> Option<CameraPose> {
    stored_camera_pose_lt(layer, crate::time::layer_time(t, layer.start_offset.0))
}

/// [`stored_camera_pose`] at a **layer** time, which is the clock every property
/// is actually evaluated at.
///
/// Split out because the correction lane's zero (K-578) is captured at layer
/// time nought — a fixed moment of the layer's own clock, so moving the layer
/// along the timeline never moves the correction with it.
#[must_use]
pub fn stored_camera_pose_lt(layer: &Layer, lt: f64) -> Option<CameraPose> {
    let LayerKind::Camera { zoom, .. } = &layer.kind else {
        return None;
    };
    let tr = &layer.transform;
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
}

impl Composition {
    /// The topmost visible Camera layer whose span contains `t` — the one that
    /// is *active*. None → the comp renders flat (3D switches ignored).
    ///
    /// Split out of [`Self::camera_pose`] because the solve link (K-417) needs
    /// the layer as well as the numbers: a linked camera's placement is derived
    /// from another layer rather than read off its own properties, and one rule
    /// for which camera is active has to serve both readings.
    #[must_use]
    pub fn active_camera(&self, t: f64) -> Option<&Layer> {
        self.layers.iter().find(|l| {
            matches!(l.kind, LayerKind::Camera { .. })
                && l.switches.visible
                && t >= l.in_point.0.to_f64()
                && t < l.out_point.0.to_f64()
        })
    }

    /// The active camera's placement at comp time `t`, read off its **own**
    /// stored properties at its layer time. None → the comp renders flat.
    ///
    /// A camera carrying a solve link derives its placement from the tracked
    /// layer instead ([`crate::track::camera_pose_at`], K-417); this answers
    /// what the document itself holds, which is what the link's fallback reads
    /// and what **Convert to keyframes** writes back.
    pub fn camera_pose(&self, t: f64) -> Option<CameraPose> {
        self.active_camera(t).and_then(|l| stored_camera_pose(l, t))
    }

    /// Every visible Light layer whose span contains `t`, evaluated at its own
    /// layer time and in the comp's own pixels (K-360).
    ///
    /// Top of the stack first, which is the order the effects that read lights
    /// take them in — so a frame that has more lights than an effect can carry
    /// spends its slots on the ones nearest the top, the same rule the layer
    /// stack uses everywhere else. A light switched off is not a light, exactly
    /// as a layer switched off is not on the picture (K-230).
    pub fn lights_at(&self, t: f64) -> Vec<ResolvedLight> {
        self.layers
            .iter()
            .filter_map(|l| {
                let LayerKind::Light { light } = &l.kind else {
                    return None;
                };
                if !l.switches.visible || t < l.in_point.0.to_f64() || t >= l.out_point.0.to_f64() {
                    return None;
                }
                let lt = crate::time::layer_time(t, l.start_offset.0);
                let tr = &l.transform;
                let gain = light.intensity.value_at(lt).max(0.0) as f32;
                Some(ResolvedLight {
                    kind: light.kind,
                    position: (tr.position_x.value_at(lt), tr.position_y.value_at(lt)),
                    colour: [
                        (light.colour[0].value_at(lt) as f32).max(0.0) * gain,
                        (light.colour[1].value_at(lt) as f32).max(0.0) * gain,
                        (light.colour[2].value_at(lt) as f32).max(0.0) * gain,
                    ],
                    // Only an area light has extent; the other two are points,
                    // whatever the stored numbers happen to say.
                    half_size: match light.kind {
                        LightKind::Area => (
                            light.half_size[0].value_at(lt).max(0.0),
                            light.half_size[1].value_at(lt).max(0.0),
                        ),
                        _ => (0.0, 0.0),
                    },
                    cone_deg: light.cone_deg.value_at(lt).clamp(0.0, 180.0),
                    rotation_deg: tr.rotation.value_at(lt),
                    falloff_px: light.falloff_px.value_at(lt).max(0.0),
                    z: tr.position_z.value_at(lt),
                    rotation_x_deg: tr.rotation_x.value_at(lt),
                    rotation_y_deg: tr.rotation_y.value_at(lt),
                })
            })
            .collect()
    }
}

/// v1 text: single run. Styled runs, fonts and animators follow the doc.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextDocument {
    pub text: String,
    /// When set, the words come from this expression at each frame instead of
    /// from `text` — the same expression language the numeric properties use,
    /// printed rather than measured (K-210, docs/03-DATA-MODEL.md §9.1).
    ///
    /// `text` is left alone while an expression drives the layer, so switching
    /// the expression off restores the words that were typed there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    /// Pixel size at natural scale.
    pub size: f64,
    pub fill: LinearColour,
    /// **Text on a path** (K-607): the id of a mask **on this layer** whose
    /// curve the glyphs run along. Unset — the ordinary case — is absent from
    /// the file and lays the line straight, and so is a mask id that names
    /// nothing, so deleting the mask hands the words back rather than
    /// emptying the layer.
    ///
    /// A mask id rather than a path of its own: the layer already carries
    /// drawable, keyable, draggable paths, and a second place to put one would
    /// be a second set of tools to edit it with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<Uuid>,
    /// How far along the path the first glyph starts, px@comp. Animatable like
    /// every other number in the document, and written as a bare number while
    /// it is still, so a document nobody has slid writes the bytes it always
    /// did.
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_zero"
    )]
    pub path_offset: Property,
    /// **The letters can move separately** (K-609): each animator names a set
    /// of per-letter offsets and the stretch of the words they apply to. Empty
    /// — the ordinary case — is absent from the file, and the layer draws the
    /// bytes it always drew (K-258).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub animators: Vec<crate::text::TextAnimator>,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl TextDocument {
    /// The words this document shows at layer time `lt`.
    ///
    /// **Every reader of a text layer's content goes through here**, so the
    /// rasteriser and the cache key can never disagree about what the layer
    /// says — a disagreement that would serve a cached frame of the old words.
    pub fn resolved_text(&self, context: Arc<ExpressionContext>) -> std::borrow::Cow<'_, str> {
        match &self.expression {
            None => std::borrow::Cow::Borrowed(&self.text),
            Some(e) => std::borrow::Cow::Owned(crate::expression::evaluate_text(e, Some(context))),
        }
    }
}

/// Per-layer composite operator (docs/06-RENDER-PIPELINE.md §blend domains).
/// The full After Effects set (K-162, T24): Normal / Add / Multiply run as
/// fixed-function linear blends; the perceptual set is computed against the
/// destination snapshot. Serialised by variant name, so adding modes never
/// disturbs existing files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum BlendMode {
    #[default]
    Normal,
    // Darken group.
    Darken,
    Multiply,
    ColourBurn,
    LinearBurn,
    DarkerColour,
    // Lighten group.
    Add,
    Lighten,
    Screen,
    ColourDodge,
    LighterColour,
    // Contrast group.
    Overlay,
    SoftLight,
    HardLight,
    LinearLight,
    VividLight,
    PinLight,
    HardMix,
    // Comparative group.
    Difference,
    Exclusion,
    /// dst − src per channel, clamped at black — the photographic subtract
    /// (GEN-1, K-151). Computed in linear light like Add's light-addition twin.
    Subtract,
    Divide,
    // Component (HSL) group.
    Hue,
    Saturation,
    Colour,
    Luminosity,
}

impl BlendMode {
    /// Every blend mode in After Effects' menu order, grouped
    /// darken → lighten → contrast → comparative → component (K-162). The
    /// single source of truth for the layer dropdown and the effect Mode
    /// param (T21), so the two never drift.
    pub const ALL: &'static [BlendMode] = &[
        BlendMode::Normal,
        BlendMode::Darken,
        BlendMode::Multiply,
        BlendMode::ColourBurn,
        BlendMode::LinearBurn,
        BlendMode::DarkerColour,
        BlendMode::Add,
        BlendMode::Lighten,
        BlendMode::Screen,
        BlendMode::ColourDodge,
        BlendMode::LighterColour,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::HardLight,
        BlendMode::LinearLight,
        BlendMode::VividLight,
        BlendMode::PinLight,
        BlendMode::HardMix,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Subtract,
        BlendMode::Divide,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Colour,
        BlendMode::Luminosity,
    ];

    /// Every mode's display name, in [`Self::ALL`]'s order — the same words
    /// [`Self::name`] returns, as one `'static` list so a schema's Choice
    /// parameter can offer the layer modes verbatim (the injected effect
    /// Blend row, K-425). `blend_mode_all_is_complete_and_named` holds the two
    /// in step.
    pub const NAMES: &'static [&'static str] = &[
        "Normal",
        "Darken",
        "Multiply",
        "Colour burn",
        "Linear burn",
        "Darker colour",
        "Add",
        "Lighten",
        "Screen",
        "Colour dodge",
        "Lighter colour",
        "Overlay",
        "Soft light",
        "Hard light",
        "Linear light",
        "Vivid light",
        "Pin light",
        "Hard mix",
        "Difference",
        "Exclusion",
        "Subtract",
        "Divide",
        "Hue",
        "Saturation",
        "Colour",
        "Luminosity",
    ];

    /// The mode's display name (British English, sentence case — docs/15).
    pub fn name(self) -> &'static str {
        match self {
            BlendMode::Normal => "Normal",
            BlendMode::Darken => "Darken",
            BlendMode::Multiply => "Multiply",
            BlendMode::ColourBurn => "Colour burn",
            BlendMode::LinearBurn => "Linear burn",
            BlendMode::DarkerColour => "Darker colour",
            BlendMode::Add => "Add",
            BlendMode::Lighten => "Lighten",
            BlendMode::Screen => "Screen",
            BlendMode::ColourDodge => "Colour dodge",
            BlendMode::LighterColour => "Lighter colour",
            BlendMode::Overlay => "Overlay",
            BlendMode::SoftLight => "Soft light",
            BlendMode::HardLight => "Hard light",
            BlendMode::LinearLight => "Linear light",
            BlendMode::VividLight => "Vivid light",
            BlendMode::PinLight => "Pin light",
            BlendMode::HardMix => "Hard mix",
            BlendMode::Difference => "Difference",
            BlendMode::Exclusion => "Exclusion",
            BlendMode::Subtract => "Subtract",
            BlendMode::Divide => "Divide",
            BlendMode::Hue => "Hue",
            BlendMode::Saturation => "Saturation",
            BlendMode::Colour => "Colour",
            BlendMode::Luminosity => "Luminosity",
        }
    }
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
    /// Label colour (TL2): an index into the theme's label palette, shown as
    /// the chip beside the layer number in the outline. 0 by default; purely
    /// organisational — never rendered into the picture.
    #[serde(default)]
    pub label: u8,
    /// The layer's own markers (docs/03 §11): cues drawn on its bar rather than
    /// on the comp's ruler.
    ///
    /// **A copy, not a view.** Dropping a composition into another one brings
    /// that comp's markers along as the layer's, so its beats are visible where
    /// the layer sits — but they are this layer's from then on, and deleting
    /// one here never reaches into the composition it came from. The alternative
    /// (drawing the source comp's live list) makes a delete on one row change a
    /// different comp, and every other place that comp is used.
    ///
    /// Pre-composing deliberately leaves this empty: the markers it copies into
    /// the new comp are the ones already on the ruler above, and drawing them
    /// again on the Precomp layer would say the same thing twice.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<crate::markers::Marker>,
    /// Per-layer audio volume in dB (docs/09 §6): 0 = unity, boostable to
    /// +50; −100 and below reads as −∞ (exact silence). Animatable like any
    /// property — fades are volume keyframes. Only heard on layers whose
    /// source carries an audio stream; harmless everywhere else. Never feeds
    /// the frame cache key (sound, not pixels).
    #[serde(default = "Property::zero")]
    pub volume_db: Property,
    /// Per-layer **Pan** — a constant-power stereo balance (docs/09 §6,
    /// K-694, reversing that section's "Pan is not in v1"). −100 is full
    /// left, 0 centre, +100 full right: a percentage of the way to one side,
    /// so a value well reads "L 50" without doing arithmetic first.
    ///
    /// Animatable like any property — the same stopwatch, the same lane
    /// diamonds, the same graph strip — because a sweep across a hit is one
    /// of the two things anybody ever does with a pan control (the other is
    /// setting it once and forgetting it). Only heard on layers whose source
    /// carries an audio stream, and, like Volume, never part of the frame
    /// cache key: it is sound, not pixels.
    #[serde(default = "Property::zero")]
    pub pan: Property,
    /// This layer is **sound and nothing else** — an Audio layer
    /// ([01-GLOSSARY.md] "Audio layer", docs/09-AUDIO.md §6, K-435).
    ///
    /// **In plain terms.** A music file has no picture, so a layer holding one
    /// only ever makes sound. A video file has both — and sometimes you want
    /// just its sound on its own row, to fade it or cut to it without the
    /// picture coming along. This flag is that choice: the layer keeps its
    /// footage source and its Volume, and the drawing half is simply not asked
    /// for. Set when an audio-only file is placed (there is no picture to
    /// draw), and by *Add audio only* on a footage item that has both.
    ///
    /// It is a flag on the layer rather than a [`LayerKind`] of its own
    /// (K-435): the source is still a footage item, so retiming, waveforms,
    /// mixing and the project file all keep working unchanged, and the only
    /// thing that differs is that nothing draws. That is why every picture
    /// path — the frame key in `lumit-eval`, the decode plan and the draw
    /// builder in `lumit-render` — skips a layer with this set, exactly as it
    /// skips a hidden one, and why the audio path does not look at it at all.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub audio_only: bool,
    /// This layer is acting as an **adjustment layer** (K-537): its own picture
    /// is set aside and its effect stack runs on the composite of everything
    /// beneath it instead.
    ///
    /// **In plain terms.** An adjustment layer is a layer that grades what is
    /// under it: you put a colour correction on it and everything below picks
    /// the correction up. This flag turns any layer that draws into one —
    /// footage, a solid, a precomp, a text layer — and turning it off makes the
    /// layer itself again. Nothing is thrown away while it is on: the footage
    /// item, the masks, the transform and the effects all sit where they were,
    /// which is what lets the switch be flicked back and forth.
    ///
    /// A flag rather than a [`LayerKind`] of its own for exactly the reason
    /// [`Self::audio_only`] is one: a kind flip cannot round-trip a footage
    /// layer, because the kind is where the source lives, so switching would
    /// have to throw the source away and switching back could not get it back.
    /// [`LayerKind::Adjustment`] stays as the kind *New adjustment layer*
    /// makes — a comp-sized container with no source at all — and every picture
    /// path asks [`Layer::is_adjustment`], which answers for both.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub adjustment: bool,
    /// Retime (K-197): layer-local time → source time, in seconds. An ordinary
    /// keyframable [`Property`] like any other — the graph editor, the
    /// stopwatch and the lane diamonds treat it exactly as they treat
    /// Position. `None` means the layer is not retimed at all and plays at
    /// source rate, which is why it is an `Option` rather than a property that
    /// is always there: an un-retimed layer shows no Retime row (docs/07 §4.3)
    /// and skips the map entirely.
    ///
    /// Enabled from the Timeline with Ctrl+Alt+T, which installs the identity
    /// map ([`Layer::identity_retime`]) so switching it on changes nothing
    /// visible and gives the row something to key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retime: Option<Property>,
    /// How a fractional source moment becomes pixels: nearest, blend, or
    /// optical flow (docs/04-RETIMING.md §10).
    ///
    /// A **render policy, not part of the map** — §10 is explicit that the two
    /// are orthogonal — so it sits beside [`Self::retime`] rather than inside
    /// it, exactly as [`crate::sequence::Clip`] has carried its own since it
    /// was written. It used to live inside the layer's segment store, which
    /// tied "how in-betweens are made" to "which retime system you use" for no
    /// reason; K-249 untangled them when that store went.
    ///
    /// Applies whether or not the layer is retimed: an un-retimed layer whose
    /// comp runs at a different rate from its source is already asking for
    /// frames between two it has.
    #[serde(default)]
    pub interpolation: crate::retime::Interpolation,
    /// The Flow tuning kept while the policy is Nearest or Blend, so switching
    /// flow off to compare against the plain shot and back on again costs
    /// nothing.
    ///
    /// **Document state, not a UI stash**: it serialises, undoes and copies
    /// with the layer like everything else, which a view-side memory of "the
    /// last settings" would not. `None` once the policy is Flow again — the
    /// live parameters are then inside [`Self::interpolation`], and two copies
    /// of the same tuning would be one too many. Boxed because most layers
    /// never have one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parked_flow: Option<Box<crate::retime::FlowParams>>,
    #[serde(default)]
    pub blend: BlendMode,
    /// Masks gate the layer's alpha before effects/transform
    /// (docs/06-RENDER-PIPELINE.md render order).
    #[serde(default)]
    pub masks: Vec<crate::mask::Mask>,
    /// Paint strokes stamped into the layer's own pixels, before its masks
    /// gate them and before its effects run (docs/03 §7.1, K-227).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paint: Vec<crate::paint::PaintStroke>,
    /// The ordered effect stack (docs/03 §8; applied top-to-bottom after
    /// masks, before transform — docs/06 render order).
    #[serde(default)]
    pub effects: Vec<EffectInstance>,
    /// The layer's **driver graph** (K-471): the drivers it carries, the wires
    /// from them into parameters and mattes, and where the boxes sit on the
    /// Graph panel's canvas.
    ///
    /// **Additive, and empty by default.** [`Self::effects`] above remains the
    /// only authority for the picture — the graph's image-path nodes are
    /// *derived* from it — so this field only ever adds wiring beside the list.
    /// A layer that has never opened the Graph panel carries an empty one, it
    /// is left out of the saved file entirely, and every project written before
    /// drivers existed loads to it and saves back the same bytes.
    #[serde(default, skip_serializing_if = "crate::graph::LayerGraph::is_empty")]
    pub graph: crate::graph::LayerGraph,
    pub switches: Switches,
    /// Unknown fields from newer Lumit versions, preserved on load/save
    /// (docs/10-FILE-FORMAT.md §1.1 — mandatory forward compatibility).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Layer {
    /// The identity Retime across a layer's own span: two linear keys running
    /// source time alongside local time, so the layer plays at source rate.
    /// What Ctrl+Alt+T installs — the AE Time Remap starting state.
    ///
    /// `from` and `to` are the layer's **local** in and out points — its comp
    /// span less its `start_offset` — not zero and its duration (K-213). A
    /// trimmed layer's visible range does not begin at its own zero, and keys
    /// that stopped short of it froze the tail: past the last key a property
    /// holds, so the part of the layer beyond `duration` played one frame over
    /// and over. Spanning the real range is also what puts the two keys on the
    /// layer's start and end where the Timeline draws them, rather than at the
    /// start of the composition.
    pub fn identity_retime(from: Rational, to: Rational) -> Property {
        let key = |time: Rational, value: f64| crate::anim::Keyframe {
            time,
            value,
            interp_in: crate::anim::SideInterp::Linear,
            interp_out: crate::anim::SideInterp::Linear,
        };
        Property {
            animation: crate::anim::Animation::Keyframed(vec![
                key(from, from.to_f64()),
                key(to, to.to_f64()),
            ]),
            extra: serde_json::Map::new(),
        }
    }

    /// Whether a Retime map is the **identity** one — every moment of the layer
    /// showing the same moment of its source, which is what switching Retime on
    /// installs and what an untouched map still is (K-236).
    ///
    /// Worth asking, because "the layer has a Retime property" and "the layer
    /// has been retimed" are different questions, and only the second one
    /// justifies putting keys into a cut. A map with two keys that read back
    /// their own times is a map nobody has shaped.
    pub fn is_identity_retime(retime: &Property) -> bool {
        let crate::anim::Animation::Keyframed(keys) = &retime.animation else {
            // A Static map holds one moment for the whole layer, which is a
            // freeze — a deliberate retime, not an identity.
            return false;
        };
        keys.iter().all(|key| {
            matches!(key.interp_in, crate::anim::SideInterp::Linear)
                && matches!(key.interp_out, crate::anim::SideInterp::Linear)
                // The value *is* the time it sits at: source time equals layer
                // time, which is the whole of what identity means. Compared as
                // the f64 the keyframe stores, since that is what was written.
                && (key.value - key.time.to_f64()).abs() < 1e-9
        })
    }

    /// Which moment of the source this layer shows at layer-local time `lt`
    /// (seconds). The Retime property when it has one, otherwise `lt` itself —
    /// an un-retimed layer reads its source at its own clock.
    ///
    /// The one place the mapping is decided, so the render plan and the frame
    /// cache key can never disagree about which source frame a layer shows.
    pub fn source_time_at(&self, lt: f64) -> f64 {
        match &self.retime {
            Some(retime) => retime.value_at(lt),
            None => lt,
        }
    }

    /// Where the playhead lands inside this layer's nested comp when the user
    /// opens it from here, standing at outer comp time `t` (K-624).
    ///
    /// Over the layer's span the answer is the moment the layer is actually
    /// showing, so it runs through the same two steps the renderer takes — the
    /// start offset, then the Retime map — rather than through arithmetic of
    /// its own. A precomp slowed to half speed therefore opens on the frame on
    /// screen, not on the frame the outer ruler reads.
    ///
    /// Off the span there is no such moment: before it the nested comp has not
    /// begun, so the answer is its start; after it, it has finished, so the
    /// answer is `inner_duration`. Both are the ends the caller would clamp to
    /// anyway, said here so every caller says them the same way.
    pub fn entry_time(&self, t: f64, inner_duration: f64) -> f64 {
        let end = inner_duration.max(0.0);
        if t < self.in_point.0.to_f64() {
            return 0.0;
        }
        if t >= self.out_point.0.to_f64() {
            return end;
        }
        let lt = crate::time::layer_time(t, self.start_offset.0);
        self.source_time_at(lt).clamp(0.0, end)
    }
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

/// Every footage item composition `comp` can put on screen: its own Footage
/// layers, the footage its Sequence layers' clips name, and — transitively —
/// everything the compositions it nests can show, whether they are reached
/// through a Precomp layer or through a comp-sourced clip.
///
/// # In plain terms
///
/// "If I open this comp, which files on disk might it want?" A project's
/// Project panel can hold hundreds of items a given comp never touches, so
/// answering this before opening anything is what lets the renderer look at
/// only the files that can actually appear. An empty comp answers "none".
///
/// Layers are walked whatever their switches and in/out points say: a hidden
/// layer, or one the playhead is not inside, is still something this comp can
/// show a moment later, and the answer must not wobble with the playhead.
/// Nesting cycles terminate (a comp is walked at most once), a Precomp layer
/// naming a comp that no longer exists is skipped rather than an error, and
/// every id appears exactly once, in walk order — the same document always
/// gives the same list.
#[must_use]
pub fn comp_footage_items(doc: &Document, comp: &Composition) -> Vec<Uuid> {
    let mut found: Vec<Uuid> = Vec::new();
    let mut walked: Vec<Uuid> = vec![comp.id];
    collect_comp_footage(doc, comp, &mut found, &mut walked);
    found
}

/// One comp's contribution to [`comp_footage_items`], plus the descent into the
/// comps it names. `walked` holds every comp already collected, so a comp
/// reached twice (a diamond) costs one walk and a cycle terminates.
fn collect_comp_footage(
    doc: &Document,
    comp: &Composition,
    found: &mut Vec<Uuid>,
    walked: &mut Vec<Uuid>,
) {
    for layer in &comp.layers {
        // An Audio layer's file is opened by the mixer, never by the picture
        // (K-435): naming it here would have the renderer probe and index the
        // video stream of a clip placed for its sound alone.
        if layer.audio_only {
            continue;
        }
        match &layer.kind {
            LayerKind::Footage { item } => {
                if !found.contains(item) {
                    found.push(*item);
                }
            }
            LayerKind::Precomp { comp: nested } => {
                descend_into_comp(doc, *nested, found, walked);
            }
            LayerKind::Sequence { clips } => {
                for clip in clips {
                    match clip.source {
                        crate::sequence::ClipSource::Footage(item) => {
                            if !found.contains(&item) {
                                found.push(item);
                            }
                        }
                        crate::sequence::ClipSource::Comp(nested) => {
                            descend_into_comp(doc, nested, found, walked);
                        }
                    }
                }
            }
            // No media source of their own: a solid is a colour, text and
            // shapes are drawn, a camera is a viewpoint, a light is something
            // other layers see, an adjustment layer works on the composite
            // below it, a null draws nothing.
            LayerKind::Solid { .. }
            | LayerKind::Text { .. }
            | LayerKind::Shape { .. }
            | LayerKind::Camera { .. }
            | LayerKind::Light { .. }
            | LayerKind::Adjustment
            | LayerKind::Null => {}
        }
    }
}

/// Walk nested comp `id` for [`comp_footage_items`], unless it has been walked
/// already (a diamond) or is not in the document (a dangling reference).
fn descend_into_comp(doc: &Document, id: Uuid, found: &mut Vec<Uuid>, walked: &mut Vec<Uuid>) {
    if walked.contains(&id) {
        return;
    }
    let Some(nested) = doc.comp(id) else {
        return;
    };
    walked.push(id);
    collect_comp_footage(doc, nested, found, walked);
}

/// Whether `layer` names project item `id` as its source — one layer's
/// contribution to [`Document::item_is_used`].
///
/// Written as an exhaustive match so a new [`LayerKind`] cannot quietly place
/// an item the `in use` badge then fails to notice: the compiler asks.
#[must_use]
fn layer_names_item(layer: &Layer, id: Uuid) -> bool {
    match &layer.kind {
        LayerKind::Footage { item } => *item == id,
        LayerKind::Solid { def } => *def == id,
        LayerKind::Precomp { comp } => *comp == id,
        LayerKind::Sequence { clips } => clips.iter().any(|c| match c.source {
            crate::sequence::ClipSource::Footage(item) => item == id,
            crate::sequence::ClipSource::Comp(comp) => comp == id,
        }),
        // Nothing from the Project panel: a drawn layer carries its own art, a
        // camera is a viewpoint, a light is something other layers see, an
        // adjustment layer works on the composite below it, a null draws
        // nothing.
        LayerKind::Text { .. }
        | LayerKind::Shape { .. }
        | LayerKind::Camera { .. }
        | LayerKind::Light { .. }
        | LayerKind::Adjustment
        | LayerKind::Null => false,
    }
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
    /// Project items' colour tags, by item id (K-451, docs/15 §12A.3a): an
    /// index into the same label palette a layer's chip uses, 0 = untagged.
    ///
    /// # In plain terms
    ///
    /// The Project panel lets an item be tagged with a colour, which tints its
    /// row icon and gives the filter chips beside the search well something to
    /// filter on. It is organisation, never anything the picture sees.
    ///
    /// **A map beside the items rather than a field on each of them**, which is
    /// where a layer keeps its own label. A colour tag is one byte and purely
    /// organisational, and there are four kinds of project item — putting it on
    /// all four would have written `label: 0` at every place any of them is
    /// built, in the engine and in a hundred and thirty tests, to store a value
    /// that is almost always the default. Absent means untagged, so a project
    /// nobody has tagged gains no line for it and every older `.lum` reads as
    /// untagged rather than failing — the serde-default rule docs/10 §1.1 gives
    /// every additive field.
    ///
    /// A `BTreeMap` so save output is ordered by id and two saves of the same
    /// document are byte-identical (docs/10 §1). An entry for an item that has
    /// since been deleted is harmless and deliberately kept: undoing the delete
    /// brings the item back still wearing its colour.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub item_labels: std::collections::BTreeMap<Uuid, u8>,
    /// Footage items' proxies, by item id ([`ProxyRef`], docs/03 §3a).
    ///
    /// **A map beside the items rather than a field on each of them**, for the
    /// reason `item_labels` above gives at length: only one of the four kinds
    /// of project item can have a proxy, almost no item has one, and a field
    /// would have written `proxy: None` at every place a `FootageItem` is
    /// built. An entry whose item has since been deleted is harmless and
    /// deliberately kept, so undoing the delete brings the proxy back with it.
    ///
    /// A `BTreeMap`, so two saves of the same document are byte-identical
    /// (docs/10 §1), and absent when empty, so a project with no proxies gains
    /// no line for it.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub proxies: std::collections::BTreeMap<Uuid, ProxyRef>,
    /// The project-wide *use proxies* master switch (docs/03 §3a).
    ///
    /// On — the default, and what a file written before proxies existed loads
    /// as — means each item's own [`ProxyRef::enabled`] decides. Off means the
    /// whole project reads originals however many proxies are attached: the one
    /// switch for "show me what I am actually delivering". It changes no
    /// picture's *geometry* and no timing, only which file the pixels come out
    /// of, so it is saved with the project and undoable like any other edit.
    ///
    /// Export ignores it entirely and delivers full resolution unless asked
    /// otherwise — see `lumit_render::export::RenderOptions::use_proxies`.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub use_proxies: bool,
    /// How hard the renderer works at the edges of transformed layers
    /// (K-274, docs/impl/anti-aliasing.md).
    ///
    /// A **project** property, not a preference, and deliberately so: it
    /// changes what a comp looks like, so it has to travel in the `.lum` and
    /// match when the file is opened on another machine. One value serves both
    /// preview and export — a preview that anti-aliased differently from the
    /// file would break the K-031 preview-equals-export identity, which the
    /// whole render path is built around.
    #[serde(default)]
    pub anti_aliasing: AntiAliasing,
    /// Where *this project's* rendered frames are parked, overriding the
    /// application-wide choice (docs/06-RENDER-PIPELINE.md §5.4, docs/07 §15).
    ///
    /// `None` — the usual case — means "whatever the application is set to". A
    /// project only carries one of these when the user has asked for this project
    /// in particular to cache somewhere: a scratch drive it lives on, or beside
    /// itself so the cache travels with it. It is in the document rather than in
    /// the settings file precisely because it belongs to the project, so it
    /// survives being opened on another machine and moves with a copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_location: Option<CacheLocation>,
    /// This project's colour management: which OCIO config its colour space
    /// names come from (K-490, docs/impl/ocio.md §3.1).
    ///
    /// A project property for the same reason `anti_aliasing` is: it changes
    /// what a comp looks like, so it travels in the `.lum` and matches when the
    /// file is opened on another machine. Absent from the file when it is the
    /// default, so a project that has never named a config is byte-identical to
    /// one written before this existed.
    #[serde(default, skip_serializing_if = "ColourManagement::is_default")]
    pub colour: ColourManagement,
    /// The project's own colour shelf: the colours kept for this project, in
    /// the order they were kept (K-448, docs/07 §6.1).
    ///
    /// # In plain terms
    ///
    /// Colours a project uses over and over — a brand red, the two greys a
    /// title sits on — are kept here so every picker in the application can
    /// offer them. They live **inside the picker** rather than on a toolbar
    /// strip (K-448), and inside the *project* rather than in a preference,
    /// because they belong to the job: a copy of the `.lum` carries them, and
    /// so does the machine it is opened on next.
    ///
    /// A `Vec`, because the order is the user's own and there is no other key
    /// to sort by; absent from the file when it is empty, so a project nobody
    /// has kept a colour in is byte-identical to one written before this
    /// existed (docs/10 §1.1).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub swatches: Vec<Swatch>,
    /// How the interface was arranged for this project, as the frontend's own
    /// JSON: the panel layout, which comps were open, where the playhead sat
    /// (K-245, docs/10-FILE-FORMAT.md §1.2).
    ///
    /// **Opaque to the engine.** Nothing here reads inside it; it is carried,
    /// stored and handed back. That is deliberate — the shape belongs to
    /// whichever frontend wrote it, and an engine that understood it would have
    /// to be changed every time a panel gained a setting.
    ///
    /// It lives in the document, rather than only in the local settings file,
    /// so a project shared with someone else opens arranged the way its author
    /// left it. It is a *hint*: a reader that already has its own record of this
    /// project prefers that, and one that cannot make sense of this ignores it.
    /// `None` for a project that has never been arranged, which is why an older
    /// build's file gains no line for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_state: Option<serde_json::Value>,
    /// Unknown fields from newer Lumit versions, preserved on load/save
    /// (docs/10-FILE-FORMAT.md §1.1 — mandatory forward compatibility).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// One colour on the project's shelf (see [`Document::swatches`]).
///
/// The channels are the picker's own: 0–1 for black to white, the same numbers
/// its hex box reads, so a swatch means the same thing whether it is applied to
/// a display colour on the 0–255 dial or to a scene-linear one. Alpha is kept
/// because a colour parameter has one.
///
/// The name is optional and nothing generates one: a shelf of colours is read
/// by eye, and a list of "Colour 1, Colour 2" would be noise. It is here so a
/// project that *does* name its colours can say so.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Swatch {
    pub colour: LinearColour,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// How many coverage samples per pixel the composite is drawn with (K-274,
/// docs/impl/anti-aliasing.md).
///
/// # In plain terms
///
/// A layer is drawn as a rectangle placed by its transform. Rotate it a few
/// degrees and its edge crosses a pixel diagonally — but a pixel is either
/// drawn or not, so the edge comes out as a staircase, and on a slow rotation
/// the steps crawl. Asking about coverage more than once per pixel and
/// averaging the answers is what smooths it, and these are the numbers of
/// questions on offer. More is smoother and costs more memory; [`Self::Off`]
/// is the picture Lumit made before this setting existed.
///
/// The named counts are the ones graphics hardware actually implements, which
/// is why this is four choices rather than a free number. A card that will not
/// give the count asked for falls back to the highest it will — down to
/// [`Self::Off`] — and says which it used; that is a machine's limit, never a
/// project's error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AntiAliasing {
    /// One sample per pixel: no anti-aliasing.
    Off,
    X2,
    /// Four samples: the standard trade, and what a card that will not give
    /// eight falls back to.
    X4,
    /// The default (K-274: on by default; K-286). Eight samples smooths the
    /// shallow diagonals four still steps on, which is where the crawl is
    /// most visible, and it costs one more multisample attachment beside the
    /// comp frame rather than more shading. A card that will not give eight
    /// falls back to [`Self::X4`] and says so.
    #[default]
    X8,
}

impl AntiAliasing {
    /// The sample count to hand the renderer.
    #[must_use]
    pub fn samples(self) -> u32 {
        match self {
            Self::Off => 1,
            Self::X2 => 2,
            Self::X4 => 4,
            Self::X8 => 8,
        }
    }

    /// The setting a sample count corresponds to; anything not one of the four
    /// counts reads as [`Self::Off`], so a value from a newer build that this
    /// one cannot draw degrades to the picture it can.
    #[must_use]
    pub fn from_samples(n: u32) -> Self {
        match n {
            2 => Self::X2,
            4 => Self::X4,
            8 => Self::X8,
            _ => Self::Off,
        }
    }
}

/// Where a project's rendered frames are parked (docs/06 §5.4). The document's
/// own answer; the application-wide setting has the same three choices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CacheLocation {
    /// Under the application's own cache folder, keyed by document id.
    AppData,
    /// In a `<project>.lum-cache/` folder beside the project file, so a copy of
    /// the project carries its cache with it. Falls back to [`Self::AppData`]
    /// until the project has been saved and therefore has a file to sit beside.
    BesideProject,
    /// Under a folder the user picked.
    Custom { folder: String },
}

/// The project's colour management (K-490, docs/impl/ocio.md §3.1).
///
/// In plain terms: an OCIO config is a folder of colour-space definitions that
/// studios publish, so a Lumit project can agree with a Nuke or Resolve project
/// about what its pixels mean. Naming one fills every colour list in the
/// application — footage interpretation, the Viewer's picker, the export's
/// output space — with that config's own vocabulary.
///
/// Only the *name* of the file lives here. The parsed config, the resolved
/// chains and the baked tables are derived state, rebuilt from the file and
/// cached by content hash exactly as decoded footage is (§3.2), so undo never
/// re-parses and two projects naming one config share one parse.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ColourManagement {
    /// The `config.ocio` file. `None` — the usual case — means the built-in
    /// colour family only, exactly the behaviour that predates this field.
    ///
    /// A [`MediaRef`] deliberately, rather than a bare path: the relative-path
    /// serialisation, the never-write-an-absolute-path promise (K-173) and
    /// content-fingerprint relink all already exist and are tested, so a config
    /// that moved with its project keeps working and one that moved elsewhere
    /// relinks through the machinery footage already uses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<MediaRef>,
}

impl ColourManagement {
    /// Whether this is the untouched default — the test that keeps an older
    /// `.lum` byte-identical, since serde writes no line for it.
    #[must_use]
    pub fn is_default(&self) -> bool {
        self.config.is_none()
    }
}

impl Document {
    pub fn new() -> Self {
        Self {
            id: Uuid::now_v7(),
            items: Vec::new(),
            auto_folders: AutoFolders::default(),
            item_labels: std::collections::BTreeMap::new(),
            proxies: std::collections::BTreeMap::new(),
            use_proxies: true,
            anti_aliasing: AntiAliasing::default(),
            cache_location: None,
            colour: ColourManagement::default(),
            swatches: Vec::new(),
            ui_state: None,
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

    /// This item's colour tag: an index into the label palette, 0 = untagged
    /// (see [`Document::item_labels`]). An item nobody has tagged — and every
    /// item in a project saved before tags existed — answers 0.
    #[must_use]
    pub fn item_label(&self, id: Uuid) -> u8 {
        self.item_labels.get(&id).copied().unwrap_or(0)
    }

    /// The proxy attached to footage item `id`, whether or not it is switched
    /// on (the Project panel's row draws an attached-but-off proxy differently
    /// from none at all).
    #[must_use]
    pub fn proxy(&self, id: Uuid) -> Option<&ProxyRef> {
        self.proxies.get(&id)
    }

    /// The proxy this item's pixels come from **as far as the document is
    /// concerned**: the project switch on, the item's own switch on, and a
    /// proxy attached. `None` means "read the original".
    ///
    /// This is only half the answer. Whether the file on the end of it is
    /// usable — present, readable, and agreeing with the original about how
    /// long the footage is — is a question about media, which this crate does
    /// not open; `lumit_render::source::effective_media` asks both halves and
    /// is the single resolution point the decode planner and the frame key
    /// both go through.
    #[must_use]
    pub fn proxy_in_use(&self, id: Uuid) -> Option<&MediaRef> {
        if !self.use_proxies {
            return None;
        }
        let p = self.proxies.get(&id)?;
        p.enabled.then_some(&p.media)
    }

    /// Whether any composition places `id` as a layer — the Project panel's
    /// `in use` badge (docs/07 §3.1, docs/15 §12A.3a).
    ///
    /// # In plain terms
    ///
    /// "Is this asset actually in anything?" A composition counts as used when
    /// it is placed inside another composition, exactly as footage and a solid
    /// do; a composition nothing nests is simply not used, however much is in
    /// it.
    ///
    /// **Direct placement only, deliberately.** The badge says "a layer
    /// somewhere names this", not "some render might reach it": the two agree
    /// for footage anyway (a nested comp's own footage is used by that comp's
    /// layers, which this sees), and answering the transitive question would
    /// make the badge come and go as an unrelated comp was nested elsewhere.
    /// [`comp_footage_items`] is the transitive walk, and it exists for the
    /// renderer, which does need it.
    ///
    /// Layers are counted whatever their switches say — a hidden layer still
    /// places the asset — and a Sequence layer's clips count, each naming its
    /// own footage or comp. The same document always gives the same answer.
    ///
    /// One pass over the layer list, stopping at the first hit and allocating
    /// nothing, so a panel of rows asking one each is a walk per row rather
    /// than a cached table to keep honest across every edit. Measured on a
    /// document far past any real one — 100 compositions of 100 layers, every
    /// one of its 1,100 items asked — the whole sweep is well inside a frame;
    /// a cache would be machinery bought with nothing.
    #[must_use]
    pub fn item_is_used(&self, id: Uuid) -> bool {
        self.items.iter().any(|item| match item {
            ProjectItem::Composition(c) => c.layers.iter().any(|l| layer_names_item(l, id)),
            _ => false,
        })
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

    /// **What "has been retimed" means** (K-236). Switching Retime on installs
    /// the identity map, so the presence of the property says nothing about
    /// whether the layer has actually been retimed — and only the second
    /// justifies a razor putting keys into both halves of a cut.
    #[test]
    fn an_untouched_retime_map_is_the_identity_one() {
        let map =
            Layer::identity_retime(Rational::new(0, 1).unwrap(), Rational::new(4, 1).unwrap());
        assert!(Layer::is_identity_retime(&map));
    }

    #[test]
    fn a_shaped_retime_map_is_not() {
        let mut map =
            Layer::identity_retime(Rational::new(0, 1).unwrap(), Rational::new(4, 1).unwrap());
        // Half speed: the layer's four seconds show the source's first two.
        if let crate::anim::Animation::Keyframed(keys) = &mut map.animation {
            if let Some(last) = keys.last_mut() {
                last.value = 2.0;
            }
        }
        assert!(!Layer::is_identity_retime(&map));
    }

    #[test]
    fn an_eased_map_that_happens_to_end_where_it_started_is_not_identity() {
        // The values read back their own times, but the curve between them
        // does not: an eased pair is a ramp, not an identity.
        let mut map =
            Layer::identity_retime(Rational::new(0, 1).unwrap(), Rational::new(4, 1).unwrap());
        if let crate::anim::Animation::Keyframed(keys) = &mut map.animation {
            if let Some(first) = keys.first_mut() {
                first.interp_out = crate::anim::SideInterp::Hold;
            }
        }
        assert!(!Layer::is_identity_retime(&map));
    }

    #[test]
    fn a_frozen_frame_is_a_retime_however_it_is_written() {
        let frozen = Property::fixed(1.5);
        assert!(!Layer::is_identity_retime(&frozen));
    }

    /// A Precomp layer sitting at 2..6 s on the outer ruler, whose four
    /// seconds show its nested comp's first two — half speed.
    fn half_speed_precomp() -> Layer {
        let mut map =
            Layer::identity_retime(Rational::new(0, 1).unwrap(), Rational::new(4, 1).unwrap());
        if let crate::anim::Animation::Keyframed(keys) = &mut map.animation {
            keys.last_mut().unwrap().value = 2.0;
        }
        Layer {
            graph: Default::default(),
            markers: Vec::new(),
            id: Uuid::now_v7(),
            name: "nested".into(),
            kind: LayerKind::Precomp {
                comp: Uuid::now_v7(),
            },
            in_point: secs(2),
            out_point: secs(6),
            start_offset: secs(2),
            transform: TransformGroup::default(),
            matte: None,
            parent: None,
            label: 0,
            volume_db: Property::zero(),
            pan: Property::zero(),
            audio_only: false,
            adjustment: false,
            retime: Some(map),
            interpolation: Default::default(),
            parked_flow: None,
            blend: BlendMode::Normal,
            masks: Vec::new(),
            paint: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        }
    }

    /// **Opening a precomp lands on the frame it is showing** (K-624). Standing
    /// four seconds into the outer comp is two seconds into a layer that starts
    /// at two, and a half-speed map shows the nested comp's first second there.
    #[test]
    fn entering_a_precomp_maps_the_playhead_through_the_retime() {
        assert!((half_speed_precomp().entry_time(4.0, 10.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn entering_a_precomp_from_before_its_span_lands_on_its_start() {
        assert_eq!(half_speed_precomp().entry_time(1.0, 10.0), 0.0);
    }

    #[test]
    fn entering_a_precomp_from_after_its_span_lands_on_its_end() {
        assert_eq!(half_speed_precomp().entry_time(7.0, 10.0), 10.0);
        // The out point is exclusive, so the first moment off the end is the end.
        assert_eq!(half_speed_precomp().entry_time(6.0, 10.0), 10.0);
    }

    /// A map may reach past the nested comp (overrun, glossary §4); the
    /// playhead may not.
    #[test]
    fn entering_a_precomp_never_lands_outside_the_nested_comp() {
        assert_eq!(half_speed_precomp().entry_time(4.0, 0.5), 0.5);
    }

    /// `BlendMode::ALL` must list every variant exactly once (the layer
    /// dropdown and the effect Mode param both iterate it — a missing mode
    /// would silently vanish from the UI). Names must be unique and non-empty.
    #[test]
    fn blend_mode_all_is_complete_and_named() {
        use std::collections::HashSet;
        // Every variant, so the compiler forces this list to grow with the enum.
        let every = [
            BlendMode::Normal,
            BlendMode::Darken,
            BlendMode::Multiply,
            BlendMode::ColourBurn,
            BlendMode::LinearBurn,
            BlendMode::DarkerColour,
            BlendMode::Add,
            BlendMode::Lighten,
            BlendMode::Screen,
            BlendMode::ColourDodge,
            BlendMode::LighterColour,
            BlendMode::Overlay,
            BlendMode::SoftLight,
            BlendMode::HardLight,
            BlendMode::LinearLight,
            BlendMode::VividLight,
            BlendMode::PinLight,
            BlendMode::HardMix,
            BlendMode::Difference,
            BlendMode::Exclusion,
            BlendMode::Subtract,
            BlendMode::Divide,
            BlendMode::Hue,
            BlendMode::Saturation,
            BlendMode::Colour,
            BlendMode::Luminosity,
        ];
        let in_all: HashSet<_> = BlendMode::ALL.iter().copied().collect();
        assert_eq!(in_all.len(), BlendMode::ALL.len(), "ALL has a duplicate");
        for m in every {
            assert!(in_all.contains(&m), "{m:?} missing from BlendMode::ALL");
        }
        assert_eq!(BlendMode::ALL.len(), every.len());
        let names: HashSet<_> = BlendMode::ALL.iter().map(|m| m.name()).collect();
        assert_eq!(names.len(), BlendMode::ALL.len(), "a name is duplicated");
        assert!(BlendMode::ALL.iter().all(|m| !m.name().is_empty()));
        let listed: Vec<&str> = BlendMode::ALL.iter().map(|m| m.name()).collect();
        assert_eq!(
            listed,
            BlendMode::NAMES,
            "NAMES must be ALL's names, in order"
        );
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
    fn matte_ref_source_migrates_from_after_effects_bool() {
        // K-142: a matte saved with K-125's `after_effects` bool migrates to the
        // three-way source, so old projects still load.
        let base = serde_json::json!({
            "layer": Uuid::now_v7(),
            "channel": "Alpha",
            "inverted": false,
        });
        // Legacy `after_effects: true` → EffectsAndMasks (the source's picture).
        let mut with_true = base.clone();
        with_true
            .as_object_mut()
            .unwrap()
            .insert("after_effects".into(), serde_json::json!(true));
        let m: MatteRef = serde_json::from_value(with_true).unwrap();
        assert_eq!(m.source, LayerInputSource::EffectsAndMasks);

        // Legacy `after_effects: false` → Masks (source-only historically still
        // applied the source's masks, so Masks is its faithful mapping).
        let mut with_false = base.clone();
        with_false
            .as_object_mut()
            .unwrap()
            .insert("after_effects".into(), serde_json::json!(false));
        let m: MatteRef = serde_json::from_value(with_false).unwrap();
        assert_eq!(m.source, LayerInputSource::Masks);

        // Absent entirely (pre-K-125) → the default, Effects and masks.
        let m: MatteRef = serde_json::from_value(base.clone()).unwrap();
        assert_eq!(m.source, LayerInputSource::EffectsAndMasks);

        // The current form round-trips through `source`, and never re-emits the
        // legacy bool.
        let now = MatteRef {
            layer: Uuid::now_v7(),
            channel: MatteChannel::Luma,
            inverted: true,
            source: LayerInputSource::Masks,
        };
        let v = serde_json::to_value(now).unwrap();
        assert!(v.get("after_effects").is_none());
        let back: MatteRef = serde_json::from_value(v).unwrap();
        assert_eq!(back.source, LayerInputSource::Masks);
    }

    #[test]
    fn effect_layer_source_reads_choice_then_legacy_bool() {
        // K-142: `layer_source` prefers the `<id>_source` Choice, falls back to
        // the legacy `<id>_after_effects` bool, then the default.
        let mut e = crate::fx::instantiate("dof").unwrap();
        // Fresh instance carries neither sibling, so it is the default
        // (Effects and masks — owner K-142 follow-up).
        assert_eq!(e.layer_source("depth"), LayerInputSource::EffectsAndMasks);

        // A legacy bool is honoured (true → EffectsAndMasks).
        e.params.push(EffectParam {
            id: "depth_after_effects".into(),
            value: EffectValue::Bool(true),
            extra: serde_json::Map::new(),
        });
        assert_eq!(e.layer_source("depth"), LayerInputSource::EffectsAndMasks);

        // The current Choice sibling wins over the legacy bool when both exist.
        e.params.push(EffectParam {
            id: "depth_source".into(),
            value: EffectValue::Choice(LayerInputSource::Masks.to_choice()),
            extra: serde_json::Map::new(),
        });
        assert_eq!(e.layer_source("depth"), LayerInputSource::Masks);
    }

    #[test]
    fn layer_input_source_maps_each_option_to_its_sampling() {
        // K-142: the render paths (draws.rs / export.rs) branch on these two
        // predicates to choose masks and effects, so pin the mapping here — each
        // option selects the intended sampling.
        use LayerInputSource::*;
        // None: raw source only (no masks, no effects).
        assert!(!None.applies_masks());
        assert!(!None.folds_effects());
        // Masks: source + masks, no effects.
        assert!(Masks.applies_masks());
        assert!(!Masks.folds_effects());
        // Effects and masks: source + masks + effects.
        assert!(EffectsAndMasks.applies_masks());
        assert!(EffectsAndMasks.folds_effects());

        // The Choice index round-trips, and the cache-key byte is distinct per
        // mode (so switching modes retires stale frames).
        for m in [None, Masks, EffectsAndMasks] {
            assert_eq!(LayerInputSource::from_choice(m.to_choice()), m);
            assert_eq!(m.key_byte() as u32, m.to_choice());
        }
        assert_eq!(
            [
                None.key_byte(),
                Masks.key_byte(),
                EffectsAndMasks.key_byte()
            ],
            [0, 1, 2]
        );
        // The default is Effects and masks (owner K-142 follow-up): a new
        // matte/depth input samples the most complete source unless narrowed.
        assert_eq!(LayerInputSource::default(), EffectsAndMasks);
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
        // Same forward-compat rule for shy: absent means off.
        assert!(!Switches::default().shy);
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

    /// Flow tuning parked while the policy is Nearest is document state: it
    /// survives a save/load, and a project saved before the field existed still
    /// loads (the key is simply absent).
    #[test]
    fn parked_flow_round_trips_and_old_projects_still_load() {
        let mut layer = comp_with_cameras().layers.remove(0);
        let params = crate::retime::FlowParams {
            smoothness: 80.0,
            detail: crate::retime::VectorDetail::Ultra,
            ..Default::default()
        };
        layer.interpolation = crate::retime::Interpolation::Nearest;
        layer.parked_flow = Some(Box::new(params.clone()));

        let json = serde_json::to_string(&layer).unwrap();
        let back: Layer = serde_json::from_str(&json).unwrap();
        assert_eq!(back.parked_flow.as_deref(), Some(&params));
        assert_eq!(back, layer);

        // An old project has no `parked_flow` key at all — which is also what
        // a layer that never parked anything writes.
        layer.parked_flow = None;
        let old = serde_json::to_string(&layer).unwrap();
        assert!(
            !old.contains("parked_flow"),
            "nothing parked, nothing saved"
        );
        assert_eq!(serde_json::from_str::<Layer>(&old).unwrap(), layer);
    }

    /// **A line on a path round-trips, and a straight one writes what it always
    /// wrote** (K-607, K-258). The two new fields are absent from the file
    /// until they are used, so every `.lum` ever saved opens here unchanged —
    /// and, just as importantly, every frame those projects have banked keeps
    /// its name.
    #[test]
    fn text_on_a_path_round_trips_and_a_straight_line_writes_no_key() {
        let mask = crate::mask::Mask::ellipse(60.0, 40.0, 30.0, 18.0);
        let mut document = TextDocument {
            text: "Lumit".into(),
            expression: None,
            size: 48.0,
            fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
            path: None,
            path_offset: crate::anim::Property::zero(),
            animators: Vec::new(),
            extra: serde_json::Map::new(),
        };

        let straight = serde_json::to_string(&document).unwrap();
        assert!(
            !straight.contains("path"),
            "a straight line wrote a path key: {straight}"
        );
        assert_eq!(
            serde_json::from_str::<TextDocument>(&straight).unwrap(),
            document
        );

        document.path = Some(mask.id);
        document.path_offset = crate::anim::Property::fixed(37.5);
        let json = serde_json::to_string(&document).unwrap();
        assert_eq!(
            serde_json::from_str::<TextDocument>(&json).unwrap(),
            document
        );
        // The offset writes as a bare number while it is still, the way every
        // other still property in the document does.
        assert!(json.contains("\"path_offset\":37.5"), "{json}");
    }

    /// **A layer with no animators writes no animators key** (K-609, K-258):
    /// the whole per-letter model is absent from the file until somebody adds
    /// one, so every `.lum` saved before it existed opens byte-identical and
    /// every frame those projects have banked keeps its name.
    #[test]
    fn text_animators_are_absent_until_there_are_some() {
        let mut document = TextDocument {
            text: "Lumit".into(),
            expression: None,
            size: 48.0,
            fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
            path: None,
            path_offset: crate::anim::Property::zero(),
            animators: Vec::new(),
            extra: serde_json::Map::new(),
        };
        let plain = serde_json::to_string(&document).unwrap();
        assert!(!plain.contains("animators"), "{plain}");

        let mut animator = crate::text::TextAnimator::new("Cascade");
        animator.selector.end = crate::anim::Property::fixed(30.0);
        animator.selector.basis = crate::text::SelectorBasis::Words;
        animator.position_y = crate::anim::Property::fixed(-60.0);
        document.animators.push(animator);
        let json = serde_json::to_string(&document).unwrap();
        assert_eq!(
            serde_json::from_str::<TextDocument>(&json).unwrap(),
            document
        );
        // The numbers write as bare numbers while they are still, and the
        // untouched ones are simply not there.
        assert!(json.contains("\"position_y\":-60.0"), "{json}");
        assert!(!json.contains("scale_x"), "{json}");
    }

    /// **The adjustment switch survives a save/load, and an old file is
    /// byte-identical** (K-537): a layer that is not an adjustment writes no
    /// key at all, so a project saved before the flag existed loads exactly as
    /// it did — and reads back with the switch off.
    ///
    /// The legacy [`LayerKind::Adjustment`] is not migrated on load (the
    /// decision entry says why), so an old adjustment layer keeps its kind and
    /// still answers [`Layer::is_adjustment`] — the one question every picture
    /// path asks.
    #[test]
    fn the_adjustment_flag_round_trips_and_old_projects_load_unchanged() {
        let mut layer = comp_with_cameras().layers.remove(0);
        layer.kind = LayerKind::Solid {
            def: uuid::Uuid::now_v7(),
        };

        // Off: no key, and an old file (which has none either) reads the same.
        let old = serde_json::to_string(&layer).unwrap();
        assert!(
            !old.contains("adjustment"),
            "a layer that is not an adjustment writes nothing"
        );
        let back: Layer = serde_json::from_str(&old).unwrap();
        assert_eq!(back, layer);
        assert!(!back.is_adjustment());

        // On: written, read back, and the source is still there.
        layer.adjustment = true;
        let json = serde_json::to_string(&layer).unwrap();
        let back: Layer = serde_json::from_str(&json).unwrap();
        assert_eq!(back, layer);
        assert!(back.is_adjustment());
        assert!(matches!(back.kind, LayerKind::Solid { .. }));

        // A layer born an adjustment answers the same question, with the flag
        // never set — the two paths meet at `is_adjustment`.
        let mut born = layer.clone();
        born.adjustment = false;
        born.kind = LayerKind::Adjustment;
        assert!(born.is_adjustment());

        // And the four with no picture refuse the switch.
        for kind in [
            LayerKind::Camera {
                zoom: Property::zero(),
                solve_link: None,
                correction_base: None,
            },
            LayerKind::Null,
        ] {
            let mut nothing = layer.clone();
            nothing.kind = kind;
            assert!(!nothing.can_adjust());
        }
        let mut sound = layer.clone();
        sound.audio_only = true;
        assert!(!sound.can_adjust(), "an Audio layer has no picture either");
    }

    fn comp_with_cameras() -> Composition {
        let mut comp = Composition {
            master_volume_db: 0.0,
            groups: Vec::new(),
            beat_grid: None,
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
            graph: Default::default(),
            markers: Vec::new(),
            id: Uuid::now_v7(),
            name: name.into(),
            kind: LayerKind::Camera {
                zoom: Property::fixed(zoom),
                solve_link: None,
                correction_base: None,
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
            label: 0,
            volume_db: crate::anim::Property::zero(),
            pan: crate::anim::Property::zero(),
            audio_only: false,
            adjustment: false,
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
            blend: BlendMode::Normal,
            masks: Vec::new(),
            paint: Vec::new(),
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
            source: LayerInputSource::None,
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
                expression: None,
                size: 12.0,
                fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
                path: None,
                path_offset: crate::anim::Property::zero(),
                animators: Vec::new(),
                extra: serde_json::Map::new(),
            },
        };
        inner.switches.visible = true;
        inner.matte = Some(MatteRef {
            layer: inner_matted.layers[0].id,
            channel: MatteChannel::Alpha,
            inverted: false,
            source: LayerInputSource::None,
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

    /// An empty composition of the given size, for the footage-reference walk.
    fn bare_comp(name: &str) -> Composition {
        Composition {
            master_volume_db: 0.0,
            groups: Vec::new(),
            beat_grid: None,
            id: Uuid::now_v7(),
            name: name.into(),
            width: 64,
            height: 64,
            frame_rate: FrameRate::new(25, 1).unwrap(),
            duration: Duration(Rational::new(4, 1).unwrap()),
            background: LinearColour([0.0, 0.0, 0.0, 1.0]),
            work_area: None,
            layers: Vec::new(),
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        }
    }

    /// One layer of `kind`, in span for the comp's whole length.
    fn bare_layer(kind: LayerKind) -> Layer {
        Layer {
            graph: Default::default(),
            markers: Vec::new(),
            id: Uuid::now_v7(),
            name: "layer".into(),
            kind,
            in_point: secs(0),
            out_point: secs(4),
            start_offset: secs(0),
            transform: TransformGroup::default(),
            matte: None,
            parent: None,
            label: 0,
            volume_db: crate::anim::Property::zero(),
            pan: crate::anim::Property::zero(),
            audio_only: false,
            adjustment: false,
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
            blend: BlendMode::Normal,
            masks: Vec::new(),
            paint: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        }
    }

    /// The footage-reference walk answers with what the comp can show and
    /// nothing else: a comp with no layers names no footage however full the
    /// project is, a comp names only its own sources, and footage reachable
    /// only through a Precomp layer or a comp-sourced clip is still named.
    ///
    /// This is the walk the renderer probes by (`HeadlessRenderer::sync_items`),
    /// so "names nothing" is what stops an empty comp opening every file in the
    /// project before it can show its first frame.
    #[test]
    fn the_footage_walk_names_what_the_comp_can_show_and_no_more() {
        let (a, b, c, unused) = (
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
        );
        let mut doc = Document::new();

        // Deepest first: `inner` shows `c` only.
        let mut inner = bare_comp("inner");
        inner
            .layers
            .push(bare_layer(LayerKind::Footage { item: c }));
        let inner_id = inner.id;

        // `middle` reaches `c` through a Precomp layer and shows `b` itself,
        // on a hidden layer — hidden today is visible after one click, and the
        // answer must not depend on a switch.
        let mut middle = bare_comp("middle");
        middle
            .layers
            .push(bare_layer(LayerKind::Precomp { comp: inner_id }));
        let mut hidden = bare_layer(LayerKind::Footage { item: b });
        hidden.switches.visible = false;
        middle.layers.push(hidden);
        let middle_id = middle.id;

        // `outer` shows `a` on a Sequence clip and nests `middle` twice — the
        // diamond a cycle guard must survive without repeating an id.
        let mut outer = bare_comp("outer");
        outer.layers.push(bare_layer(LayerKind::Sequence {
            clips: vec![crate::sequence::Clip::new(
                crate::sequence::ClipSource::Footage(a),
                Rational::new(0, 1).unwrap(),
                Rational::new(2, 1).unwrap(),
                Rational::new(0, 1).unwrap(),
                Rational::new(2, 1).unwrap(),
            )],
        }));
        outer
            .layers
            .push(bare_layer(LayerKind::Precomp { comp: middle_id }));
        outer
            .layers
            .push(bare_layer(LayerKind::Precomp { comp: middle_id }));
        let outer_id = outer.id;

        // A comp nobody nests, showing an item nobody else does.
        let mut orphan = bare_comp("orphan");
        orphan
            .layers
            .push(bare_layer(LayerKind::Footage { item: unused }));
        let orphan_id = orphan.id;

        let empty = bare_comp("empty");
        let empty_id = empty.id;

        for comp in [inner, middle, outer, orphan, empty] {
            doc.items.push(ProjectItem::Composition(comp));
        }

        let named = |id: Uuid| comp_footage_items(&doc, doc.comp(id).unwrap());

        assert!(
            named(empty_id).is_empty(),
            "an empty comp can show nothing, however much the project holds"
        );
        assert_eq!(named(inner_id), vec![c]);
        assert_eq!(
            named(middle_id),
            vec![c, b],
            "a hidden layer's footage counts, and the Precomp's comes first"
        );
        assert_eq!(
            named(outer_id),
            vec![a, c, b],
            "a clip's source, then everything the nested comps reach, once each"
        );
        assert_eq!(named(orphan_id), vec![unused]);
        for id in [inner_id, middle_id, outer_id, empty_id] {
            assert!(
                !named(id).contains(&unused),
                "footage only another comp shows must never be named"
            );
        }
    }

    /// A Precomp cycle terminates, and a Precomp layer naming a comp that is
    /// not in the document is skipped rather than being an error.
    #[test]
    fn the_footage_walk_survives_a_cycle_and_a_dangling_precomp() {
        let item = Uuid::now_v7();
        let mut doc = Document::new();

        let mut one = bare_comp("one");
        let mut two = bare_comp("two");
        let (one_id, two_id) = (one.id, two.id);
        one.layers
            .push(bare_layer(LayerKind::Precomp { comp: two_id }));
        one.layers.push(bare_layer(LayerKind::Footage { item }));
        two.layers
            .push(bare_layer(LayerKind::Precomp { comp: one_id }));
        two.layers.push(bare_layer(LayerKind::Precomp {
            comp: Uuid::now_v7(),
        }));
        two.layers.push(bare_layer(LayerKind::Sequence {
            clips: vec![crate::sequence::Clip::new(
                crate::sequence::ClipSource::Comp(one_id),
                Rational::new(0, 1).unwrap(),
                Rational::new(2, 1).unwrap(),
                Rational::new(0, 1).unwrap(),
                Rational::new(2, 1).unwrap(),
            )],
        }));
        doc.items.push(ProjectItem::Composition(one));
        doc.items.push(ProjectItem::Composition(two));

        assert_eq!(
            comp_footage_items(&doc, doc.comp(one_id).unwrap()),
            vec![item]
        );
        assert_eq!(
            comp_footage_items(&doc, doc.comp(two_id).unwrap()),
            vec![item]
        );
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

    fn bare_footage(name: &str, relative: &str, absolute: &str) -> FootageItem {
        FootageItem {
            sequence: None,
            id: Uuid::now_v7(),
            name: name.into(),
            media: MediaRef {
                relative_path: relative.into(),
                absolute_path: absolute.into(),
                fingerprint: None,
                extra: serde_json::Map::new(),
            },
            colour_space: None,
            extra: serde_json::Map::new(),
        }
    }

    /// The Path column shows the path the saved project actually carries
    /// (K-173: the absolute one is never written), and falls back to the
    /// absolute one only when there is no relative path to show — an imported
    /// file in a project that has never been saved.
    #[test]
    fn a_media_reference_shows_its_relative_path_and_falls_back_to_the_absolute() {
        let both = bare_footage("a.mp4", "footage/a.mp4", "D:/shoot/a.mp4");
        assert_eq!(both.media.display_path(), "footage/a.mp4");

        let unsaved = bare_footage("a.mp4", "", "D:/shoot/a.mp4");
        assert_eq!(unsaved.media.display_path(), "D:/shoot/a.mp4");

        let neither = bare_footage("a.mp4", "", "");
        assert_eq!(neither.media.display_path(), "");
    }

    /// The `in use` badge: footage, a solid and a nested composition all count
    /// as used the moment a layer names them, and an item nothing names does
    /// not — whatever else the project holds.
    #[test]
    fn an_item_is_used_when_some_comps_layer_names_it() {
        let footage = bare_footage("a.mp4", "a.mp4", "");
        let used_footage = footage.id;
        let idle_footage = bare_footage("b.mp4", "b.mp4", "");
        let idle_id = idle_footage.id;

        let solid = SolidDef {
            id: Uuid::now_v7(),
            name: "White".into(),
            colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
            width: 64,
            height: 64,
            extra: serde_json::Map::new(),
        };
        let solid_id = solid.id;

        let mut inner = bare_comp("Inner");
        let inner_id = inner.id;
        inner
            .layers
            .push(bare_layer(LayerKind::Footage { item: used_footage }));

        let mut outer = bare_comp("Outer");
        outer
            .layers
            .push(bare_layer(LayerKind::Precomp { comp: inner_id }));
        outer
            .layers
            .push(bare_layer(LayerKind::Solid { def: solid_id }));
        // A drawn layer names no project item at all.
        outer.layers.push(bare_layer(LayerKind::Adjustment));

        let mut doc = Document::new();
        doc.items.push(ProjectItem::Footage(footage));
        doc.items.push(ProjectItem::Footage(idle_footage));
        doc.items.push(ProjectItem::Solid(solid));
        doc.items.push(ProjectItem::Composition(inner));
        doc.items.push(ProjectItem::Composition(outer));

        assert!(doc.item_is_used(used_footage));
        assert!(doc.item_is_used(solid_id));
        assert!(
            doc.item_is_used(inner_id),
            "a comp placed as a layer is used"
        );
        assert!(!doc.item_is_used(idle_id), "nothing places this footage");
        assert!(
            !doc.item_is_used(doc.items[4].id()),
            "the outer comp is in nothing, however much is in it"
        );
        assert!(!doc.item_is_used(Uuid::now_v7()), "an id nobody knows");
    }

    /// A Sequence layer's clips place items too — each clip names its own
    /// footage or composition, and the badge must see them.
    #[test]
    fn a_sequence_clip_counts_as_placing_its_source() {
        let footage = bare_footage("a.mp4", "a.mp4", "");
        let item = footage.id;
        let mut source = bare_comp("Source");
        let source_id = source.id;
        source.layers.push(bare_layer(LayerKind::Adjustment));

        let mut cut = bare_comp("Cut");
        cut.layers.push(bare_layer(LayerKind::Sequence {
            clips: vec![
                crate::sequence::Clip::new(
                    crate::sequence::ClipSource::Footage(item),
                    Rational::new(0, 1).unwrap(),
                    Rational::new(1, 1).unwrap(),
                    Rational::new(0, 1).unwrap(),
                    Rational::new(1, 1).unwrap(),
                ),
                crate::sequence::Clip::new(
                    crate::sequence::ClipSource::Comp(source_id),
                    Rational::new(1, 1).unwrap(),
                    Rational::new(1, 1).unwrap(),
                    Rational::new(0, 1).unwrap(),
                    Rational::new(1, 1).unwrap(),
                ),
            ],
        }));

        let mut doc = Document::new();
        doc.items.push(ProjectItem::Footage(footage));
        doc.items.push(ProjectItem::Composition(source));
        doc.items.push(ProjectItem::Composition(cut));

        assert!(doc.item_is_used(item));
        assert!(doc.item_is_used(source_id));
    }

    /// A hidden layer, or one the playhead is never inside, still *places* the
    /// asset: the badge says what the project contains, not what is on screen.
    #[test]
    fn a_hidden_layer_still_uses_its_item() {
        let footage = bare_footage("a.mp4", "a.mp4", "");
        let item = footage.id;
        let mut comp = bare_comp("Comp 1");
        let mut layer = bare_layer(LayerKind::Footage { item });
        layer.switches.visible = false;
        comp.layers.push(layer);

        let mut doc = Document::new();
        doc.items.push(ProjectItem::Footage(footage));
        doc.items.push(ProjectItem::Composition(comp));

        assert!(doc.item_is_used(item));
    }

    /// The measurement behind [`Document::item_is_used`] having no cache
    /// (K-451): a document far past any real one, every item asked, and the
    /// whole sweep well inside a frame. If this ever stops being true the
    /// answer is a table invalidated on edits — not a slower panel.
    #[test]
    fn asking_every_item_of_a_huge_project_is_cheap() {
        let mut doc = Document::new();
        let mut footage_ids = Vec::new();
        for n in 0..1_000 {
            let f = bare_footage(&format!("{n}.mp4"), &format!("{n}.mp4"), "");
            footage_ids.push(f.id);
            doc.items.push(ProjectItem::Footage(f));
        }
        for c in 0..100 {
            let mut comp = bare_comp(&format!("Comp {c}"));
            for l in 0..100 {
                comp.layers.push(bare_layer(LayerKind::Footage {
                    item: footage_ids[(c * 100 + l) % footage_ids.len()],
                }));
            }
            doc.items.push(ProjectItem::Composition(comp));
        }

        let started = std::time::Instant::now();
        let used = doc
            .items
            .iter()
            .filter(|i| doc.item_is_used(i.id()))
            .count();
        let elapsed = started.elapsed();

        assert_eq!(used, 1_000, "every footage item is placed; no comp is");
        // Generous against a loaded CI machine; the point is the order of
        // magnitude, not the number.
        assert!(
            elapsed < std::time::Duration::from_millis(250),
            "a whole-panel sweep took {elapsed:?} — time to cache after all"
        );
    }

    /// **Old projects load unchanged** (K-571). A transform written before
    /// separate axes existed carries no `axis_modes` at all, and reads as the
    /// default: both pairs combined, Scale linked.
    #[test]
    fn a_transform_without_axis_modes_reads_as_the_default() {
        let json = r#"{
            "anchor_x": {"animation": {"Static": 0.0}},
            "anchor_y": {"animation": {"Static": 0.0}},
            "position_x": {"animation": {"Static": 0.0}},
            "position_y": {"animation": {"Static": 0.0}},
            "scale_x": {"animation": {"Static": 100.0}},
            "scale_y": {"animation": {"Static": 100.0}},
            "rotation": {"animation": {"Static": 0.0}},
            "opacity": {"animation": {"Static": 100.0}}
        }"#;
        let group: TransformGroup = serde_json::from_str(json).unwrap();
        assert_eq!(group.axis_modes, AxisModes::default());
        assert_eq!(group.axis_modes.get(TransformPair::Scale), AxisMode::Linked);
        assert_eq!(
            group.axis_modes.get(TransformPair::Position),
            AxisMode::Combined
        );
    }

    /// And a default one writes nothing, so a project that never separated
    /// anything is byte-identical to the one the old build wrote.
    #[test]
    fn default_axis_modes_are_not_written_and_separated_ones_round_trip() {
        let plain = TransformGroup::default();
        let text = serde_json::to_string(&plain).unwrap();
        assert!(
            !text.contains("axis_modes"),
            "the default is absence, not a field: {text}"
        );

        let mut separated = TransformGroup::default();
        separated
            .axis_modes
            .set(TransformPair::Position, AxisMode::Separated);
        let back: TransformGroup =
            serde_json::from_str(&serde_json::to_string(&separated).unwrap()).unwrap();
        assert_eq!(back, separated);
    }

    /// An unknown key from a newer build survives a load and save beside the
    /// new field (docs/03 §12's forward-compatibility rule).
    #[test]
    fn an_unknown_transform_field_survives_beside_the_axis_modes() {
        let mut group = TransformGroup::default();
        group
            .axis_modes
            .set(TransformPair::Anchor, AxisMode::Separated);
        let mut value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&group).unwrap()).unwrap();
        value["some_future_field"] = serde_json::json!(7);
        let back: TransformGroup = serde_json::from_value(value).unwrap();
        assert_eq!(back.axis_modes.anchor, AxisMode::Separated);
        assert_eq!(
            back.extra.get("some_future_field"),
            Some(&serde_json::json!(7))
        );
    }

    /// **Coming back together does not move the picture** (K-571). Two axes
    /// keyed at different times gain each other's key times, and every axis
    /// reads exactly what it read before at every moment.
    #[test]
    fn recombining_unions_the_key_times_without_changing_a_value() {
        use crate::anim::{Animation, Keyframe, SideInterp};
        let key = |t: i64, v: f64| Keyframe {
            time: Rational::new(t, 1).unwrap(),
            value: v,
            interp_in: SideInterp::Bezier {
                speed: 0.0,
                influence: 1.0 / 3.0,
            },
            interp_out: SideInterp::Bezier {
                speed: 0.0,
                influence: 1.0 / 3.0,
            },
        };
        let mut group = TransformGroup::default();
        group.position_x.animation = Animation::Keyframed(vec![key(0, 0.0), key(4, 100.0)]);
        group.position_y.animation = Animation::Keyframed(vec![key(1, 10.0), key(3, 50.0)]);

        let before: Vec<[f64; 2]> = (0..=40)
            .map(|i| {
                let t = f64::from(i) / 10.0;
                [group.position_x.value_at(t), group.position_y.value_at(t)]
            })
            .collect();

        let unified = group.unified_axes(TransformPair::Position);
        assert_eq!(unified.len(), 2, "both axes gained the other's times");
        for (prop, animation) in unified {
            group.get_mut(prop).animation = animation;
        }

        let times = |a: &Animation| match a {
            Animation::Keyframed(keys) => keys.iter().map(|k| k.time.to_f64()).collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        assert_eq!(times(&group.position_x.animation), vec![0.0, 1.0, 3.0, 4.0]);
        assert_eq!(times(&group.position_y.animation), vec![0.0, 1.0, 3.0, 4.0]);

        for (i, expected) in before.iter().enumerate() {
            let t = f64::from(i as i32) / 10.0;
            let x = group.position_x.value_at(t);
            let y = group.position_y.value_at(t);
            assert!(
                (x - expected[0]).abs() < 1e-9,
                "x at {t}: {x} vs {}",
                expected[0]
            );
            assert!(
                (y - expected[1]).abs() < 1e-9,
                "y at {t}: {y} vs {}",
                expected[1]
            );
        }
    }

    /// A static axis is left static: a constant needs no keys to stay constant.
    #[test]
    fn recombining_leaves_a_static_axis_alone() {
        use crate::anim::{Animation, Keyframe, SideInterp};
        let mut group = TransformGroup::default();
        group.scale_x.animation = Animation::Keyframed(vec![Keyframe {
            time: Rational::new(2, 1).unwrap(),
            value: 50.0,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        }]);
        let unified = group.unified_axes(TransformPair::Scale);
        assert!(unified.is_empty(), "nothing to merge onto a constant");
        assert!(!group.scale_y.is_animated());
    }
}
