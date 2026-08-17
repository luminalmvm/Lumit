use std::{println, sync::Arc};

use flutter_rust_bridge::frb;

use uuid::Uuid;

use crate::api::effect::BridgeRational;
use crate::api::layer::BridgeSpan;
use crate::api::{
    effect::BridgeEffectInstance,
    footage::FootageReference,
    layer::LayerReference,
    state::{LumitBridgeState, PROJECTS},
    worker_thread::{
        RenderCompRequest, RenderCompRequestWithPreview, RenderScopeRequest, SamplePixelsRequest,
        WorkerRequest,
        WorkerRequest::{RenderComp, RenderCompWithPreview},
    },
    BridgeError,
};

/// One timeline marker (docs/03 §11): a cue on the comp's timebase.
///
/// The engine's marker also carries a duration, a kind and any unknown fields
/// a newer Lumit wrote (docs/10 §1.1); none of the three has a control, so none
/// of them crosses. They are **not** lost on a write-back: [`core_markers`]
/// merges each incoming marker onto the one the document already holds under
/// that id, so the panel edits what it can see and the rest survives untouched
/// (K-270).
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeMarker {
    pub id: Uuid,
    pub time: BridgeRational,
    pub label: String,
}

/// A core marker as the bridge carries it. One conversion each way, shared by
/// the composition's list and by every layer's own (K-254) — two copies of this
/// mapping is two chances for a marker to mean something different depending on
/// which row it is drawn on.
#[frb(ignore)]
pub(crate) fn bridge_marker(m: &lumit_core::markers::Marker) -> BridgeMarker {
    BridgeMarker {
        id: m.id,
        time: BridgeRational {
            num: m.time.0.num(),
            den: m.time.0.den(),
        },
        label: m.label.clone(),
    }
}

/// A whole marker list coming back from the panel, merged onto the list the
/// document holds (K-270).
///
/// **Why a merge and not a conversion.** The bridge marker carries the three
/// fields a panel can edit — id, time, label — and the engine's marker carries
/// three more it cannot: the *kind* (a detected beat's provenance, plus its
/// confidence), a spanning marker's *duration*, and the `extra` map that keeps
/// fields a newer Lumit wrote (docs/10 §1.1's forward-compatibility promise).
/// Rebuilding each marker from the bridge's three fields alone flattened all
/// three to their defaults, so **dragging or renaming a beat marker on the
/// ruler turned it into an ordinary cue** — and then *Clear beat markers* could
/// no longer find it, because there was nothing left to say it had ever been
/// one. K-254's ruler markers made that a click away.
///
/// So each incoming marker is matched by id against what the document already
/// holds: found, and it keeps that marker's kind, duration and extra; new, and
/// it is a plain user marker, which is exactly what a marker the panel just
/// created is. Nothing about the frb surface changes — the panel has no use for
/// a kind it cannot edit, and inventing a control for one to fix a data-loss
/// bug would be the wrong order.
#[frb(ignore)]
pub(crate) fn core_markers(
    incoming: Vec<BridgeMarker>,
    existing: &[lumit_core::markers::Marker],
) -> Result<Vec<lumit_core::markers::Marker>, BridgeError> {
    incoming
        .into_iter()
        .map(|m| {
            let was = existing.iter().find(|e| e.id == m.id);
            core_marker(m, was)
        })
        .collect()
}

#[frb(ignore)]
pub(crate) fn core_marker(
    m: BridgeMarker,
    existing: Option<&lumit_core::markers::Marker>,
) -> Result<lumit_core::markers::Marker, BridgeError> {
    use lumit_core::time::{CompTime, Rational};
    Ok(lumit_core::markers::Marker {
        id: m.id,
        time: CompTime(
            Rational::new(m.time.num, m.time.den).map_err(|_| BridgeError::InvalidTime)?,
        ),
        // Everything the panel cannot edit is carried from the marker that was
        // already there; a marker it has just made has nothing to carry, and
        // takes the plain-user defaults.
        duration: existing.and_then(|e| e.duration),
        label: m.label,
        kind: existing.map(|e| e.kind).unwrap_or_default(),
        extra: existing.map(|e| e.extra.clone()).unwrap_or_default(),
    })
}

/// Every blend mode, in the order the Timeline's dropdown shows them. The index
/// into this list is what `LayerReference::get_blend`/`set_blend` speak, so the
/// two cannot disagree about what "3" means.
///
/// Stateless, so a free function: the dropdown is built before any layer is
/// selected.
#[frb(sync)]
pub fn list_blend_modes() -> Vec<String> {
    lumit_core::model::BlendMode::ALL
        .iter()
        .map(|mode| format!("{mode:?}"))
        .collect()
}

/// A composition's pixel dimensions.
#[frb(non_opaque)]
pub struct BridgeCompSize {
    pub width: u32,
    pub height: u32,
}

/// One layer of the comp read model (K-184): the plain-data handle Dart
/// addresses edits by, and everything the panels draw for it.
#[frb(non_opaque)]
pub struct BridgeLayerEntry {
    pub layer: LayerReference,
    pub info: crate::api::layer::BridgeLayerInfo,
}

/// The comp read model (K-184): what one `get_model` crossing carries. Dart
/// holds this and refreshes it when the engine reports a change; panels draw
/// from it with no bridge calls at all.
#[frb(non_opaque)]
pub struct BridgeCompModel {
    pub duration_frames: i64,
    /// The comp's rate as a plain number, for panels that map seconds to
    /// pixels (the waveform lane) without a bridge call per paint.
    pub fps: f64,
    /// The exact rate, for the Timeline's timecode readout: 29.97 must count
    /// 30 frames a second, which a double cannot say (docs/14 §2).
    pub fps_num: u32,
    pub fps_den: u32,
    /// The comp's master motion-blur shutter (K-120): whether layers with
    /// their own motion-blur switch actually blur. Drawn by the Timeline's
    /// master button; written through `set_motion_blur_enabled`.
    pub motion_blur_enabled: bool,
    /// The comp's background colour, scene-linear RGBA — what the Viewer
    /// bar's swatch shows. In the model so a bar that rebuilds on every
    /// arriving frame reads the held copy rather than asking the engine per
    /// rebuild (K-184); writes still go through `set_background`.
    pub background: [f32; 4],
    pub layers: Vec<BridgeLayerEntry>,
}

/// Everything the Composition settings dialog reads and writes.
///
/// The frame rate is the exact `num`/`den` pair and the duration is exact
/// rational **seconds**, never floating point (docs/14 §2). A dialog that
/// round-tripped 29.97 through a double would not hand it back as 30000/1001.
///
/// The duration is seconds rather than a frame count because the frame rate is
/// editable in the same dialog, and a frame count means nothing without knowing
/// which rate it was counted at: applying "1800 frames" after changing 60 fps to
/// 30 halved the comp's real length while every layer kept its own seconds, which
/// is what made the layers look retimed (K-180). Seconds are what the document
/// stores, so the rate can change without the comp getting longer or shorter.
/// Callers wanting the count ask [`CompositionReference::duration_frames`].
#[frb(non_opaque)]
pub struct BridgeCompSettings {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub duration: BridgeRational,
}

impl BridgeCompSettings {
    /// What a comp gets when nobody chose: 1920×1080, 60 fps, 30 seconds.
    ///
    /// Here rather than in the frontend so the New composition dialog and a
    /// `new_composition` with no settings cannot drift into different ideas of
    /// what a default comp is.
    #[frb(sync)]
    pub fn defaults() -> BridgeCompSettings {
        BridgeCompSettings {
            name: String::new(),
            width: 1920,
            height: 1080,
            fps_num: 60,
            fps_den: 1,
            duration: BridgeRational { num: 30, den: 1 },
        }
    }

    /// The engine types this settings block names, or `None` when the rate or
    /// duration is not a time at all.
    #[frb(ignore)]
    pub(crate) fn to_engine(
        &self,
    ) -> Option<(lumit_core::time::FrameRate, lumit_core::time::Duration)> {
        use lumit_core::time::{Duration, FrameRate, Rational};
        let rate = FrameRate::new(self.fps_num, self.fps_den).ok()?;
        let duration = Rational::new(self.duration.num, self.duration.den).ok()?;
        // A comp shorter than one frame has nothing to show, so the floor is one
        // frame at the rate being applied.
        let floor = rate.frame_duration().ok()?;
        Some((rate, Duration(duration.max(floor.0))))
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
#[frb]
pub struct CompositionReference {
    #[frb(name = "internalproject")]
    pub project: Uuid,
    #[frb(name = "internalid")]
    pub id: Uuid,
}

/// How playback should behave when the machine cannot render at the
/// composition's own rate — the choice the Viewer offers, and shows.
///
/// The two are genuinely different jobs, not a quality slider:
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BridgePlaybackMode {
    /// **Keep time; lower the resolution.** The realtime controller measures
    /// each frame and drops to a coarser preview tier until playback keeps up,
    /// so the picture stays in step with the sound and goes soft rather than
    /// stuttering. Frames are kept under the tier they were actually made at, so
    /// the cache bar can show them dimmed — held, but coarser than you are
    /// watching — and a second pass over the same stretch is served rather than
    /// rendered again.
    Adaptive,
    /// **Every frame, at the resolution asked for, however long it takes** — and
    /// kept, so the second pass over the same stretch plays properly. Sound
    /// plays while rendering holds the comp's rate and is paused by the worker
    /// if the picture falls genuinely behind (K-171): a paused track over a
    /// slow-motion picture, never a drifting one.
    EveryFrame,
}

/// One animated mask's shape at a moment (K-342): which mask, on which layer,
/// and the path it is showing there.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeAnimatedMaskPath {
    pub layer: Uuid,
    pub mask: Uuid,
    pub vertices: Vec<crate::api::layer::BridgeVertex>,
}

impl CompositionReference {
    #[frb(ignore)]
    pub fn new(project: Uuid, id: Uuid) -> CompositionReference {
        CompositionReference { project, id }
    }

    #[frb(ignore)]
    pub fn project_id(&self) -> Uuid {
        self.project
    }

    #[frb(ignore)]
    pub fn id(&self) -> Uuid {
        self.id
    }

    #[frb(ignore)]
    pub(crate) fn project(&self) -> Result<Arc<std::sync::RwLock<LumitBridgeState>>, BridgeError> {
        let projects = PROJECTS.read().map_err(|_| BridgeError::ReadFailed)?;
        let project = projects.get(&self.project);

        let p = project.ok_or(BridgeError::InvalidProject)?;
        Ok(p.clone())
    }

    /// The comp's pixel dimensions. The Viewer needs these to work out what
    /// fraction of comp resolution it is actually showing, which is the `scale`
    /// every render request carries — without them it could only ever ask for
    /// full resolution.
    #[frb(sync)]
    pub fn get_size(&self) -> Result<BridgeCompSize, BridgeError> {
        let comp = self.composition()?;
        Ok(BridgeCompSize {
            width: comp.width,
            height: comp.height,
        })
    }

    /// Everything the Composition settings dialog shows.
    ///
    /// The frame rate crosses as an exact `{num, den}` pair and the duration as
    /// exact rational seconds, never as a float — docs/14 §2's rational-time rule.
    /// 29.97 fps is 30000/1001, and a dialog that round-tripped it through a double
    /// would not give it back.
    #[frb(sync)]
    pub fn get_settings(&self) -> Result<BridgeCompSettings, BridgeError> {
        let comp = self.composition()?;
        Ok(BridgeCompSettings {
            name: comp.name.clone(),
            width: comp.width,
            height: comp.height,
            fps_num: comp.frame_rate.num(),
            fps_den: comp.frame_rate.den(),
            duration: BridgeRational {
                num: comp.duration.0.num(),
                den: comp.duration.0.den(),
            },
        })
    }

    /// How many frames the comp is long at its own rate — the Timeline's axis,
    /// and one past the last frame the transport can reach.
    ///
    /// Derived rather than stored: the document holds a length in seconds, and
    /// the count is that length read at whatever rate the comp currently has.
    #[frb(sync)]
    pub fn duration_frames(&self) -> Result<i64, BridgeError> {
        let comp = self.composition()?;
        Ok(comp
            .frame_rate
            .frame_at(lumit_core::time::CompTime(comp.duration.0)))
    }

    /// The document's revision number: bumped once per committed change, undo,
    /// redo or recovery. The Dart read model compares it per rebuild — one
    /// cheap crossing — and re-reads [`Self::get_model`] only when it moved,
    /// so a rebuild of an unchanged document costs exactly one call (K-184).
    #[frb(sync)]
    pub fn document_revision(&self) -> Result<u64, BridgeError> {
        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(proj.store.revision())
    }

    /// The whole comp as the panels draw it, in ONE crossing (K-184): every
    /// layer's handle and its full [`BridgeLayerInfo`] (switches, blend, span
    /// as frames, transform, every effect's every value), plus the comp's
    /// length. This is the Dart read model's refresh: read once per document
    /// change, never per widget rebuild — so selecting a layer, or any other
    /// pure-interface change, costs zero bridge calls.
    #[frb(sync)]
    pub fn get_model(&self) -> Result<BridgeCompModel, BridgeError> {
        let comp = self.composition()?;
        Ok(BridgeCompModel {
            duration_frames: comp
                .frame_rate
                .frame_at(lumit_core::time::CompTime(comp.duration.0)),
            fps: comp.frame_rate.fps(),
            fps_num: comp.frame_rate.num(),
            fps_den: comp.frame_rate.den(),
            motion_blur_enabled: comp.motion_blur.enabled,
            background: comp.background.0,
            layers: comp
                .layers
                .iter()
                .map(|layer| BridgeLayerEntry {
                    layer: LayerReference::new(self.project, self.id, layer.id),
                    info: crate::api::layer::read_layer_info(&comp, layer),
                })
                .collect(),
        })
    }

    /// Apply the Composition settings dialog, as one undo step.
    ///
    /// Dimensions are clamped to 16..=16384 and the duration to at least one frame,
    /// so a dialog cannot commit a comp that is zero pixels wide or zero frames
    /// long. The background colour is preserved: it is not part of this dialog, and
    /// `SetCompSettings` carries the whole settings block.
    ///
    /// Changing only the frame rate changes only the frame rate: the duration
    /// crosses as seconds, so the comp keeps its real length and every layer keeps
    /// its own timing — the comp shows more (or fewer) frames per second and
    /// nothing plays faster (K-180).
    #[frb(sync)]
    pub fn set_settings(&self, settings: BridgeCompSettings) -> Result<(), BridgeError> {
        let comp = self.composition()?;
        let (frame_rate, duration) = settings.to_engine().ok_or(BridgeError::InvalidFrameRate)?;

        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store
            .commit(lumit_core::Op::SetCompSettings {
                comp: self.id,
                name: settings.name,
                width: settings.width.clamp(16, 16384),
                height: settings.height.clamp(16, 16384),
                frame_rate,
                duration,
                background: comp.background,
            })
            .map_err(BridgeError::OpError)?;
        Ok(())
    }

    /// This composition's background colour, scene-linear RGBA (docs/07 §2.2
    /// item 10). What the composite is drawn onto where nothing covers it, and
    /// what an export writes there — distinct from the Viewer's transparency
    /// grid (K-352), which only decides whether that backdrop is *drawn*.
    #[frb(sync)]
    #[must_use]
    pub fn background(&self) -> [f32; 4] {
        self.composition()
            .map(|c| c.background.0)
            .unwrap_or([0.0, 0.0, 0.0, 1.0])
    }

    /// Set this composition's background colour — one op, one undo step
    /// (K-357). A document edit that reaches the export, unlike the Viewer's
    /// preview-only grid.
    #[frb(sync)]
    pub fn set_background(&self, rgba: [f32; 4]) -> Result<(), BridgeError> {
        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store
            .commit(lumit_core::Op::SetCompBackground {
                comp: self.id,
                background: lumit_core::model::LinearColour(rgba),
            })
            .map_err(BridgeError::OpError)?;
        Ok(())
    }

    /// Turn the comp's master motion-blur shutter on or off (K-120), keeping
    /// the shutter's angle, phase and sample count as they are. One op, one
    /// undo step — the Timeline's master button.
    #[frb(sync)]
    pub fn set_motion_blur_enabled(&self, on: bool) -> Result<(), BridgeError> {
        let comp = self.composition()?;
        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store
            .commit(lumit_core::Op::SetCompMotionBlur {
                comp: self.id,
                motion_blur: lumit_core::model::MotionBlur {
                    enabled: on,
                    ..comp.motion_blur
                },
            })
            .map_err(BridgeError::OpError)?;
        Ok(())
    }

    /// Add a Solid layer backed by a fresh SolidDef filed in the Solids
    /// auto-folder — one batch, one undo step, matching the egui frontend. The
    /// solid is comp-sized and white, named "White solid N".
    #[frb(sync)]
    pub fn add_solid_layer(&self) -> Result<LayerReference, BridgeError> {
        use lumit_core::model::{LinearColour, ProjectItem, SolidDef};
        use lumit_core::ops::AutoFolderKind;

        let comp = self.composition()?;
        let doc = self.document()?;
        let (folder, mut ops) = crate::edits::ensure_auto_folder_ops(&doc, AutoFolderKind::Solids);

        let def = Uuid::now_v7();
        let solids = doc
            .items
            .iter()
            .filter(|i| matches!(i, ProjectItem::Solid(_)))
            .count();
        let name = format!("White solid {}", solids + 1);

        // The folder op may itself be an AddItem, so the index has to account
        // for what this batch has already inserted.
        let added = ops
            .iter()
            .filter(|o| matches!(o, lumit_core::Op::AddItem { .. }))
            .count();
        ops.push(lumit_core::Op::AddItem {
            index: doc.items.len() + added,
            item: Box::new(ProjectItem::Solid(SolidDef {
                id: def,
                name: name.clone(),
                colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
                width: comp.width,
                height: comp.height,
                extra: serde_json::Map::new(),
            })),
        });
        ops.push(crate::edits::file_into_folder_op(&doc, folder, def));

        let layer = crate::edits::base_layer(
            name,
            lumit_core::model::LayerKind::Solid { def },
            comp.duration.0,
            crate::edits::centred_transform(
                f64::from(comp.width),
                f64::from(comp.height),
                comp.width,
                comp.height,
            ),
        );
        let id = layer.id;
        ops.push(lumit_core::Op::AddLayer {
            comp: self.id,
            index: 0,
            layer: Box::new(layer),
        });

        self.commit(lumit_core::Op::Batch { ops })?;
        Ok(LayerReference::new(self.project, self.id, id))
    }

    /// Place another composition into this one as a Precomp layer.
    ///
    /// Refuses to nest a comp inside itself. A deeper cycle — A inside B inside
    /// A — is not checked here, because doing it properly means walking the
    /// whole tree on every insertion; the render guards defensively against one
    /// and the Hierarchy panel bounds its own recursion. The one-step case is
    /// checked because it is the one a user reaches by accident.
    #[frb(sync)]
    pub fn add_precomp_layer(
        &self,
        comp: &CompositionReference,
    ) -> Result<LayerReference, BridgeError> {
        if comp.id == self.id {
            return Err(BridgeError::InvalidComp);
        }
        let inner = comp.composition()?;
        let outer = self.composition()?;

        let mut layer = crate::edits::base_layer(
            inner.name.clone(),
            lumit_core::model::LayerKind::Precomp { comp: inner.id },
            outer.duration.0,
            crate::edits::centred_transform(
                f64::from(inner.width),
                f64::from(inner.height),
                outer.width,
                outer.height,
            ),
        );
        // The comp's own markers come with it as the layer's (K-254): a
        // composition dropped into another is a piece of material, and its
        // beats are part of what you are placing. Copied, not referenced —
        // from here they are this layer's, and editing them never reaches back
        // into the composition they came from. New ids for the same reason.
        layer.markers = inner
            .markers
            .iter()
            .map(|m| lumit_core::markers::Marker {
                id: Uuid::now_v7(),
                ..m.clone()
            })
            .collect();
        self.add_at_top(layer)
    }

    /// Pack `layer_ids` into a new composition and put that comp back in their
    /// place as a Precomp layer — the Pre-compose dialogue's one call
    /// (`Ctrl+Shift+C`, docs/07 §13.4).
    ///
    /// The new comp inherits this one's size, rate, background and — unless
    /// `adjust_duration` asks otherwise — its duration too, which is what K-068
    /// asks of a comp created inside an active one.
    ///
    /// `leave_attributes` is the dialogue's first choice, and only ever offered
    /// for a single layer: the layer moves into the new comp stripped back to
    /// its source, and its transform, effects, masks, retime, blend and
    /// switches stay behind on the Precomp layer, so the picture is unchanged
    /// but the attributes now act on the nested comp. Asking for it with more
    /// than one layer is refused rather than half-applied. Without it every
    /// layer moves whole, and the Precomp layer is a plain centred one.
    ///
    /// `adjust_duration` trims the new comp to the selection's own span: its
    /// duration becomes `max(out) - min(in)`, every packed layer shifts back by
    /// `min(in)`, and the Precomp layer spans that same stretch with a start
    /// offset that lines inner time zero up with it. Without it the new comp is
    /// as long as this one and nothing moves in time at all: every packed layer
    /// keeps the times it had, and the Precomp layer spans the whole comp.
    ///
    /// The layers go in at the depth of the topmost one, so a precompose in
    /// the middle of a stack does not send it to the front. The comp auto-files
    /// into the Compositions folder however it was made (K-068), and the whole
    /// move is one [`Op::Batch`], so one undo step puts the layers back.
    ///
    /// A packed layer whose parent or matte stayed behind keeps the id it
    /// pointed at, and the engine reads a link it cannot resolve as no link —
    /// the parent chain stops there (`layer_parent_chain`). Nothing dangles
    /// into a crash, and clearing them here would only spell the same result.
    #[frb(sync)]
    pub fn precompose(
        &self,
        layer_ids: Vec<Uuid>,
        name: String,
        leave_attributes: bool,
        adjust_duration: bool,
    ) -> Result<LayerReference, BridgeError> {
        use lumit_core::model::{Composition, MotionBlur, ProjectItem};
        use lumit_core::ops::AutoFolderKind;
        use lumit_core::time::CompTime;
        use lumit_core::Op;

        let comp = self.composition()?;
        let doc = self.document()?;

        // Read in stack order, not selection order, so the packed comp holds
        // the layers the way the timeline showed them. What is actually packed
        // is what was actually found: an id belonging to some other comp would
        // otherwise fail the batch on its way through `RemoveLayer` and lose
        // the whole precompose with it.
        let packed: Vec<lumit_core::model::Layer> = comp
            .layers
            .iter()
            .filter(|l| layer_ids.contains(&l.id))
            .cloned()
            .collect();
        if packed.is_empty() {
            return Err(BridgeError::InvalidLayer);
        }
        // Leaving the attributes behind means there is one layer for them to
        // act on. Asked for a stack, the dialogue offers Move instead, and the
        // engine refuses rather than picking a layer for the user.
        if leave_attributes && packed.len() > 1 {
            return Err(BridgeError::InvalidLayer);
        }

        // Every packed layer sits at or below this index, so the slot is still
        // a valid one once the batch's removals have run.
        let index = comp
            .layers
            .iter()
            .position(|l| packed.iter().any(|p| p.id == l.id))
            .unwrap_or(0);

        // The selection's own span, and the shift that brings it back to zero
        // inside the new comp. Without `adjust_duration` the shift is nothing
        // and the span is the whole comp, which is what leaves timing alone.
        let min_in = packed
            .iter()
            .map(|l| l.in_point)
            .min()
            .unwrap_or(CompTime::ZERO);
        let max_out = packed
            .iter()
            .map(|l| l.out_point)
            .max()
            .unwrap_or(CompTime(comp.duration.0));
        let span = max_out
            .delta(min_in)
            .map_err(|_| BridgeError::InvalidTime)?;
        let (duration, shift, precomp_in, precomp_out) =
            if adjust_duration && !span.0.is_zero() && !span.0.is_negative() {
                (span, min_in, min_in, max_out)
            } else {
                (
                    comp.duration,
                    CompTime::ZERO,
                    CompTime::ZERO,
                    CompTime(comp.duration.0),
                )
            };
        // Time moves as a whole: a layer's in point, out point and start
        // offset all step back by the same amount, so a packed layer plays the
        // same footage at the same moment of the Precomp layer as before.
        let shift_back = |t: CompTime| -> Result<CompTime, BridgeError> {
            t.sub_dur(lumit_core::time::Duration(shift.0))
                .map_err(|_| BridgeError::InvalidTime)
        };

        let name = if name.trim().is_empty() {
            let existing = doc
                .items
                .iter()
                .filter(
                    |i| matches!(i, ProjectItem::Composition(c) if c.name.starts_with("Pre-comp ")),
                )
                .count();
            format!("Pre-comp {}", existing + 1)
        } else {
            name.trim().to_string()
        };

        let mut inner_layers = Vec::with_capacity(packed.len());
        for src in &packed {
            let mut layer = src.clone();
            if leave_attributes {
                // Stripped back to its source: the attributes are staying
                // behind on the Precomp layer, and a copy on both would apply
                // each of them twice.
                layer.transform = crate::edits::centred_transform(
                    f64::from(comp.width),
                    f64::from(comp.height),
                    comp.width,
                    comp.height,
                );
                layer.effects.clear();
                layer.masks.clear();
                layer.retime = None;
                layer.blend = Default::default();
                layer.switches = Default::default();
                layer.parent = None;
                layer.matte = None;
            }
            layer.in_point = shift_back(src.in_point)?;
            layer.out_point = shift_back(src.out_point)?;
            layer.start_offset = shift_back(src.start_offset)?;
            inner_layers.push(layer);
        }

        let inner = Composition {
            id: Uuid::now_v7(),
            name: name.clone(),
            width: comp.width,
            height: comp.height,
            frame_rate: comp.frame_rate,
            duration,
            background: comp.background,
            work_area: None,
            layers: inner_layers,
            // The comp's markers go in with the layers (K-254). They are part
            // of how the work is laid out, and a packed section that loses its
            // cues has lost the map to itself. Shifted with everything else
            // when `adjust_duration` moves time back to zero, and any that fall
            // outside the new comp's span are left behind rather than parked
            // where nothing can reach them.
            markers: comp
                .markers
                .iter()
                .filter_map(|m| {
                    let time = shift_back(m.time).ok()?;
                    (!time.0.is_negative() && time.0 <= duration.0).then(|| {
                        lumit_core::markers::Marker {
                            id: Uuid::now_v7(),
                            time,
                            ..m.clone()
                        }
                    })
                })
                .collect(),
            motion_blur: MotionBlur::default(),
            extra: serde_json::Map::new(),
        };
        let inner_id = inner.id;

        // Comps auto-file into the Compositions folder, however they are made
        // (K-068) — a precomp that landed at the project root would be the one
        // comp the habit missed.
        let (folder, mut ops) =
            crate::edits::ensure_auto_folder_ops(&doc, AutoFolderKind::Compositions);
        let queued = ops
            .iter()
            .filter(|o| matches!(o, Op::AddItem { .. }))
            .count();
        ops.push(Op::AddItem {
            index: doc.items.len() + queued,
            item: Box::new(ProjectItem::Composition(inner)),
        });
        ops.push(crate::edits::file_into_folder_op(&doc, folder, inner_id));

        for layer in &packed {
            ops.push(Op::RemoveLayer {
                comp: self.id,
                layer: layer.id,
            });
        }

        let mut layer = crate::edits::base_layer(
            name,
            lumit_core::model::LayerKind::Precomp { comp: inner_id },
            precomp_out.0,
            if leave_attributes {
                packed[0].transform.clone()
            } else {
                crate::edits::centred_transform(
                    f64::from(comp.width),
                    f64::from(comp.height),
                    comp.width,
                    comp.height,
                )
            },
        );
        layer.in_point = precomp_in;
        layer.out_point = precomp_out;
        // Inner time zero is the moment the new comp starts in this one, so a
        // trimmed comp needs the offset to line the two up; an untrimmed one
        // starts at zero and needs none.
        layer.start_offset = shift;
        if leave_attributes {
            let src = &packed[0];
            layer.effects = src.effects.clone();
            layer.masks = src.masks.clone();
            layer.retime = src.retime.clone();
            layer.blend = src.blend;
            layer.switches = src.switches.clone();
        }
        let layer_id = layer.id;
        ops.push(Op::AddLayer {
            comp: self.id,
            index,
            layer: Box::new(layer),
        });

        self.commit(Op::Batch { ops })?;
        Ok(LayerReference::new(self.project, self.id, layer_id))
    }

    /// Add a Text layer with the "Text" starter document, centred.
    #[frb(sync)]
    pub fn add_text_layer(&self) -> Result<LayerReference, BridgeError> {
        use lumit_core::anim::Property;
        use lumit_core::model::{LinearColour, TextDocument, TransformGroup};

        let comp = self.composition()?;
        let size = 72.0_f64;
        let text = "Text";
        // The anchor sits on the estimated glyph bounds so the layer rotates
        // and scales about its own middle rather than its top-left corner.
        let estimated_width = text.chars().count() as f64 * size * 0.5;

        let layer = crate::edits::base_layer(
            "Text".into(),
            lumit_core::model::LayerKind::Text {
                document: TextDocument {
                    text: text.into(),
                    expression: None,
                    size,
                    fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
                    extra: serde_json::Map::new(),
                },
            },
            comp.duration.0,
            TransformGroup {
                anchor_x: Property::fixed(estimated_width * 0.5),
                anchor_y: Property::fixed(size * 0.5),
                position_x: Property::fixed(f64::from(comp.width) * 0.5),
                position_y: Property::fixed(f64::from(comp.height) * 0.5),
                ..TransformGroup::default()
            },
        );
        self.add_at_top(layer)
    }

    /// Add a Shape layer holding `contents`, at the top of the stack (K-237).
    ///
    /// The art is in the layer's own coordinates, and the layer is placed so
    /// that art lands where it was drawn: the anchor sits on the art's own
    /// top-left corner and Position carries it to the same place in the comp.
    /// A shape tool that dragged a rectangle across the picture therefore makes
    /// a layer whose rectangle is exactly where the drag was.
    #[frb(sync)]
    pub fn add_shape_layer(
        &self,
        name: String,
        contents: Vec<crate::api::layer::BridgeShapeItem>,
    ) -> Result<LayerReference, BridgeError> {
        use lumit_core::anim::Property;
        use lumit_core::model::TransformGroup;

        if contents.is_empty() || contents.iter().any(|i| i.vertices.len() < 2) {
            return Err(BridgeError::EmptyPath);
        }
        let comp = self.composition()?;
        let items: Vec<lumit_core::shape::ShapeItem> =
            contents.iter().map(|i| i.write_item()).collect();
        // The art's own box: the layer's natural size, and where it sits.
        let (x0, y0, _x1, _y1) =
            lumit_core::shape::contents_bounds(&items).ok_or(BridgeError::EmptyPath)?;

        let layer = crate::edits::base_layer(
            name,
            lumit_core::model::LayerKind::Shape { contents: items },
            comp.duration.0,
            TransformGroup {
                anchor_x: Property::fixed(0.0),
                anchor_y: Property::fixed(0.0),
                position_x: Property::fixed(x0),
                position_y: Property::fixed(y0),
                ..TransformGroup::default()
            },
        );
        self.add_at_top(layer)
    }

    /// Add a text layer **where the Type tool clicked**, already holding the
    /// document it should hold, as one op (K-230).
    ///
    /// The tool used to make a layer and then correct it: `add_text_layer`
    /// starts a layer saying "Text" in the middle of the composition, and the
    /// tool then wrote an empty line into it and moved it to the click. Three
    /// ops for one gesture, so `Ctrl+Z` walked back through two states nobody
    /// had ever seen — an empty box, then the word "Text" — before the layer
    /// finally went away. One op is one undo step, and undoing it removes the
    /// layer, which is what making a layer means.
    ///
    /// The anchor sits on the **left end of the baseline**, so what is typed
    /// runs to the right of the point clicked and sits on it rather than
    /// straddling it. It is recentred on the finished line when the edit ends.
    #[frb(sync)]
    pub fn add_text_layer_at(
        &self,
        document: crate::api::assets::BridgeTextDocument,
        x: f64,
        y: f64,
    ) -> Result<LayerReference, BridgeError> {
        use lumit_core::anim::Property;
        use lumit_core::model::TransformGroup;

        let comp = self.composition()?;
        let size = document.size;
        let layer = crate::edits::base_layer(
            "Text".into(),
            lumit_core::model::LayerKind::Text {
                document: crate::api::assets::text_document_of(document),
            },
            comp.duration.0,
            TransformGroup {
                anchor_x: Property::fixed(0.0),
                anchor_y: Property::fixed(size),
                position_x: Property::fixed(x),
                position_y: Property::fixed(y),
                ..TransformGroup::default()
            },
        );
        self.add_at_top(layer)
    }

    /// Add a Camera layer at the comp centre. The default zoom is the After
    /// Effects 50 mm model, `comp width × 50/36`.
    #[frb(sync)]
    pub fn add_camera_layer(&self) -> Result<LayerReference, BridgeError> {
        use lumit_core::anim::Property;
        use lumit_core::model::TransformGroup;

        let comp = self.composition()?;
        let layer = crate::edits::base_layer(
            "Camera".into(),
            lumit_core::model::LayerKind::Camera {
                zoom: Property::fixed(f64::from(comp.width) * 50.0 / 36.0),
            },
            comp.duration.0,
            TransformGroup {
                position_x: Property::fixed(f64::from(comp.width) * 0.5),
                position_y: Property::fixed(f64::from(comp.height) * 0.5),
                ..TransformGroup::default()
            },
        );
        self.add_at_top(layer)
    }

    /// Add a Light layer at the comp centre (K-360).
    ///
    /// `kind` is 0 point, 1 spot, 2 area — an integer rather than the enum
    /// because that is the shape every other frb choice takes. An **area**
    /// light starts at a tenth of the comp's width and height, which is a
    /// softbox rather than a pinprick: a light with no size would draw exactly
    /// as a point one and leave nothing to discover.
    #[frb(sync)]
    pub fn add_light_layer(&self, kind: u32) -> Result<LayerReference, BridgeError> {
        use lumit_core::anim::Property;
        use lumit_core::model::{LightDef, LightKind, TransformGroup};

        let comp = self.composition()?;
        let kind = match kind {
            1 => LightKind::Spot,
            2 => LightKind::Area,
            _ => LightKind::Point,
        };
        let half = |v: u32| Property::fixed(f64::from(v) * 0.1);
        let light = Box::new(LightDef {
            kind,
            half_size: match kind {
                LightKind::Area => [half(comp.width), half(comp.height)],
                _ => [Property::zero(), Property::zero()],
            },
            ..LightDef::default()
        });
        let name = match kind {
            LightKind::Point => "Point light",
            LightKind::Spot => "Spot light",
            LightKind::Area => "Area light",
        };
        let layer = crate::edits::base_layer(
            name.into(),
            lumit_core::model::LayerKind::Light { light },
            comp.duration.0,
            TransformGroup {
                position_x: Property::fixed(f64::from(comp.width) * 0.5),
                position_y: Property::fixed(f64::from(comp.height) * 0.5),
                ..TransformGroup::default()
            },
        );
        self.add_at_top(layer)
    }

    /// Add an Adjustment layer: a comp-sized effect container with no source of
    /// its own, centred so scale and rotation pivot about the middle.
    #[frb(sync)]
    pub fn add_adjustment_layer(&self) -> Result<LayerReference, BridgeError> {
        let comp = self.composition()?;
        let layer = crate::edits::base_layer(
            "Adjustment".into(),
            lumit_core::model::LayerKind::Adjustment,
            comp.duration.0,
            crate::edits::centred_transform(
                f64::from(comp.width),
                f64::from(comp.height),
                comp.width,
                comp.height,
            ),
        );
        self.add_at_top(layer)
    }

    /// Add a Null layer: an invisible layer with no source of its own, carrying
    /// only a transform, for parenting rigs. It has no size, so only its
    /// position is centred and the anchor stays at the origin.
    #[frb(sync)]
    pub fn add_null_layer(&self) -> Result<LayerReference, BridgeError> {
        use lumit_core::anim::Property;
        use lumit_core::model::TransformGroup;

        let comp = self.composition()?;
        let layer = crate::edits::base_layer(
            "Null".into(),
            lumit_core::model::LayerKind::Null,
            comp.duration.0,
            TransformGroup {
                position_x: Property::fixed(f64::from(comp.width) * 0.5),
                position_y: Property::fixed(f64::from(comp.height) * 0.5),
                ..TransformGroup::default()
            },
        );
        self.add_at_top(layer)
    }

    /// Add an empty Sequence layer — a clip row spanning the comp.
    #[frb(sync)]
    pub fn add_sequence_layer(&self) -> Result<LayerReference, BridgeError> {
        let comp = self.composition()?;
        let layer = crate::edits::base_layer(
            "Sequence".into(),
            lumit_core::model::LayerKind::Sequence { clips: Vec::new() },
            comp.duration.0,
            crate::edits::centred_transform(
                f64::from(comp.width),
                f64::from(comp.height),
                comp.width,
                comp.height,
            ),
        );
        self.add_at_top(layer)
    }

    /// The comp's work area — the span the Viewer previews and the export
    /// writes — or `None` for the whole comp.
    #[frb(sync)]
    pub fn get_work_area(&self) -> Result<Option<BridgeSpan>, BridgeError> {
        Ok(self.composition()?.work_area.map(|(a, b)| BridgeSpan {
            in_point: BridgeRational {
                num: a.0.num(),
                den: a.0.den(),
            },
            out_point: BridgeRational {
                num: b.0.num(),
                den: b.0.den(),
            },
            // A work area has no content of its own to slip, so this is always
            // zero; the field is shared with a layer span for one type.
            start_offset: BridgeRational { num: 0, den: 1 },
        }))
    }

    /// Set the work area, or clear it with `None`.
    #[frb(sync)]
    pub fn set_work_area(&self, span: Option<BridgeSpan>) -> Result<(), BridgeError> {
        use lumit_core::time::{CompTime, Rational};
        let work_area = match span {
            None => None,
            Some(span) => {
                let a = Rational::new(span.in_point.num, span.in_point.den)
                    .map_err(|_| BridgeError::InvalidTime)?;
                let b = Rational::new(span.out_point.num, span.out_point.den)
                    .map_err(|_| BridgeError::InvalidTime)?;
                Some((CompTime(a), CompTime(b)))
            }
        };
        self.commit(lumit_core::Op::SetWorkArea {
            comp: self.id,
            work_area,
        })
    }

    /// Every marker on this comp, in the order the document holds them.
    #[frb(sync)]
    pub fn get_markers(&self) -> Result<Vec<BridgeMarker>, BridgeError> {
        Ok(self
            .composition()?
            .markers
            .iter()
            .map(bridge_marker)
            .collect())
    }

    /// Replace the whole marker list — one op, trivially invertible, which is
    /// also how beat detection commits a regenerated set.
    #[frb(sync)]
    pub fn set_markers(&self, markers: Vec<BridgeMarker>) -> Result<(), BridgeError> {
        // Merged onto what the comp already holds, so a dragged or renamed beat
        // marker stays a beat marker (K-270).
        let markers = core_markers(markers, &self.composition()?.markers)?;

        self.commit(lumit_core::Op::SetCompMarkers {
            comp: self.id,
            markers,
        })
    }

    /// Paste a layer copied by [`crate::api::layer::LayerReference::copy_layer`]
    /// into this composition, at the top of the stack (K-275).
    ///
    /// `at_frame` is where the layer's **in point** lands: the playhead, in the
    /// ordinary case. `None` keeps the time it was copied at, which is the
    /// setting for putting the same layer at the same moment in a second comp —
    /// the two paste behaviours the owner asked for, decided by the caller
    /// rather than by a mode this end has to remember.
    ///
    /// Whichever is chosen, the layer moves as one: in point, out point and
    /// `start_offset` all shift together (`lumit_core::edit_layer_span`'s
    /// `MoveIn`, the same rule the `[` key follows), so its keyframes and the
    /// source frames it shows travel with it rather than sliding against it.
    ///
    /// **What is not copied is a reference to something that is not here.** The
    /// pasted layer gets a fresh id and fresh effect ids — two layers sharing an
    /// id would make every op that names one ambiguous — and its parent and
    /// track matte are kept only when they still name a layer in *this* comp.
    /// A parent that came from another composition is dropped rather than left
    /// dangling: a layer parented to nothing visible would be a puzzle, and
    /// re-parenting is one drag.
    #[frb(sync)]
    pub fn paste_layer(
        &self,
        text: String,
        at_frame: Option<i64>,
    ) -> Result<LayerReference, BridgeError> {
        #[derive(serde::Deserialize)]
        struct Copied {
            comp: Uuid,
            layer: lumit_core::model::Layer,
        }
        let copied: Copied = serde_json::from_str(&text).map_err(|_| BridgeError::InvalidItem)?;
        let mut layer = copied.layer;
        let comp = self.composition()?;

        layer.id = Uuid::now_v7();
        for effect in &mut layer.effects {
            effect.id = Uuid::now_v7();
        }
        // A reference only survives if what it names is here. Pasting back into
        // the comp it was copied from keeps both; pasting elsewhere keeps
        // neither, because neither id means anything there.
        let here = |id: Uuid| copied.comp == self.id && comp.layers.iter().any(|l| l.id == id);
        if layer.parent.is_some_and(|p| !here(p)) {
            layer.parent = None;
        }
        if layer.matte.as_ref().is_some_and(|m| !here(m.layer)) {
            layer.matte = None;
        }

        if let Some(frame) = at_frame {
            let at = comp
                .frame_rate
                .time_of_frame(frame.max(0))
                .map_err(|_| BridgeError::InvalidTime)?;
            let (in_point, out_point, start_offset) = lumit_core::ops::edit_layer_span(
                layer.in_point,
                layer.out_point,
                layer.start_offset,
                at,
                lumit_core::ops::SpanEdit::MoveIn,
            )
            .ok_or(BridgeError::InvalidTime)?;
            layer.in_point = in_point;
            layer.out_point = out_point;
            layer.start_offset = start_offset;
        }

        self.add_at_top(layer)
    }

    /// Insert `layer` at the top of the stack.
    #[frb(ignore)]
    fn add_at_top(&self, layer: lumit_core::model::Layer) -> Result<LayerReference, BridgeError> {
        let id = layer.id;
        self.commit(lumit_core::Op::AddLayer {
            comp: self.id,
            index: 0,
            layer: Box::new(layer),
        })?;
        Ok(LayerReference::new(self.project, self.id, id))
    }

    #[frb(ignore)]
    fn document(&self) -> Result<std::sync::Arc<lumit_core::Document>, BridgeError> {
        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(proj.store.snapshot())
    }

    #[frb(ignore)]
    fn commit(&self, op: lumit_core::Op) -> Result<(), BridgeError> {
        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store.commit(op).map_err(BridgeError::OpError)?;
        Ok(())
    }

    /// The composition this reference names, cloned out of the current snapshot.
    #[frb(ignore)]
    pub(crate) fn composition(&self) -> Result<lumit_core::model::Composition, BridgeError> {
        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let snapshot = proj.store.snapshot();

        match snapshot.item(self.id).ok_or(BridgeError::InvalidComp)? {
            lumit_core::model::ProjectItem::Composition(composition) => Ok(composition.clone()),
            // A CompositionReference pointing at a non-composition item means the
            // id was reused or the reference outlived its item.
            _ => Err(BridgeError::InvalidComp),
        }
    }

    /// Place `footage` into this composition as a new top layer.
    ///
    /// The layer's span is the media's own duration in comp frames, and its
    /// transform is anchored on the media's own centre at the comp centre (K-150),
    /// so a placed clip appears centred and pivots about its middle. Both fall
    /// back to the comp's duration and size when the media cannot be probed —
    /// a missing file still places, so the user can relink it rather than being
    /// unable to add it at all.
    ///
    /// The duration comes from the container's real `duration_seconds`, not from
    /// a frame count: audio-only media has no video frame count or rate, and
    /// reconstructing seconds from those silently clamped such a clip to one frame.
    #[frb(sync)]
    /// Place `footage` in this composition as a new layer.
    ///
    /// `as_sequence` is Settings ▸ Interface ▸ Editing ▸ *Video arrives as a
    /// Sequence layer* (K-246), forwarded by the frontend. On, media that
    /// **runs** — a video stream longer than a single frame — arrives as a
    /// one-clip Sequence layer, ready to be cut on its own row; a still image
    /// never does, because there is nothing in one frame to cut. Off, and for
    /// stills either way, this is the plain Footage layer it always was.
    ///
    /// It is one call rather than "add, then convert" so the choice is one
    /// undo step and one funnel: every route into a comp — a drop, a
    /// double-click, a menu — comes through here and cannot disagree with the
    /// others about what a video import becomes.
    pub fn add_footage_layer(
        &self,
        footage: &FootageReference,
        as_sequence: bool,
    ) -> Result<(), BridgeError> {
        let proj = self.project()?;
        let comp = self.composition()?;

        let layer = {
            let p = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            let doc = p.store.snapshot();

            let item = footage.id();
            let Some(lumit_core::model::ProjectItem::Footage(f)) = doc.item(item) else {
                return Err(BridgeError::InvalidItem);
            };

            let (out, nat_w, nat_h) = Self::footage_span_and_size(&p, f, &comp);
            let sequenced = as_sequence && Self::runs_as_video(&p, f);

            let kind = if sequenced {
                lumit_core::model::LayerKind::Sequence {
                    clips: vec![lumit_core::sequence::Clip::new(
                        lumit_core::sequence::ClipSource::Footage(item),
                        lumit_core::time::Rational::ZERO,
                        out,
                        lumit_core::time::Rational::ZERO,
                        out,
                    )],
                }
            } else {
                lumit_core::model::LayerKind::Footage { item }
            };

            crate::edits::base_layer(
                f.name.clone(),
                kind,
                out,
                crate::edits::centred_transform(nat_w, nat_h, comp.width, comp.height),
            )
        };

        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store
            .commit(lumit_core::Op::AddLayer {
                comp: self.id,
                index: 0,
                layer: Box::new(layer),
            })
            .map_err(BridgeError::OpError)?;
        Ok(())
    }

    /// Whether this media is something to cut: a video stream that runs for
    /// more than a single frame (K-246).
    ///
    /// A still image probes with a video stream too — one frame of it — which
    /// is why the question is about duration and not about the stream being
    /// there. Media that will not probe answers **false**: a Sequence layer is
    /// the more elaborate shape, and guessing wrong towards the plain one is
    /// the cheaper mistake to undo.
    ///
    /// Image sequences will qualify by this same rule once they are a footage
    /// kind at all (docs/TODO.md) — they run, so they answer true with no
    /// change here.
    #[frb(ignore)]
    fn runs_as_video(state: &LumitBridgeState, footage: &lumit_core::model::FootageItem) -> bool {
        #[cfg(feature = "media")]
        {
            let Some(path) = FootageReference::resolve_path(state, footage) else {
                return false;
            };
            // `ensure_probed`, not the prober: the file was queued for the
            // worker when it was imported, so this is normally a look-up. It
            // probes here and now when it is not, because `add_footage_layer`
            // is synchronous and must answer with what the file actually is
            // rather than with a guess it would have to revise.
            let Some(info) = crate::probe::ensure_probed(&path) else {
                return false;
            };
            let Some(video) = info.video.as_ref() else {
                return false;
            };
            let fps = video.fps();
            // Half a frame's slack, so a one-frame still cannot creep over the
            // line on a rounded duration.
            let one_frame = if fps > 0.0 { 1.0 / fps } else { 0.0 };
            info.duration_seconds > one_frame * 1.5
        }

        #[cfg(not(feature = "media"))]
        {
            let _ = (state, footage);
            false
        }
    }

    /// The span end and natural pixel size a placed clip should take: the media's
    /// own when it probes, the comp's when it does not.
    #[frb(ignore)]
    fn footage_span_and_size(
        state: &LumitBridgeState,
        footage: &lumit_core::model::FootageItem,
        comp: &lumit_core::model::Composition,
    ) -> (lumit_core::time::Rational, f64, f64) {
        let fallback = (
            comp.duration.0,
            f64::from(comp.width),
            f64::from(comp.height),
        );

        #[cfg(feature = "media")]
        {
            let Some(path) = FootageReference::resolve_path(state, footage) else {
                return fallback;
            };
            let Some(info) = crate::probe::ensure_probed(&path) else {
                return fallback;
            };
            let frames = (info.duration_seconds * comp.frame_rate.fps()).round() as i64;
            let out = comp
                .frame_rate
                .time_of_frame(frames.max(1))
                .map(|t| t.0)
                .unwrap_or(comp.duration.0);
            // Audio-only media has no video stream at all, so it takes the comp's
            // size — there is no natural size to anchor on.
            let (nat_w, nat_h) = match &info.video {
                Some(v) if v.width > 0 && v.height > 0 => (f64::from(v.width), f64::from(v.height)),
                _ => (f64::from(comp.width), f64::from(comp.height)),
            };
            (out, nat_w, nat_h)
        }

        // Without the media feature nothing probes, so a placed clip spans the
        // whole comp at comp size.
        #[cfg(not(feature = "media"))]
        {
            let _ = (state, footage);
            fallback
        }
    }

    /// The **shape every animated mask is actually showing** at `frame`
    /// (K-342), so the Viewer can draw a keyed mask's wireframe where the
    /// picture has it rather than where its still path used to be.
    ///
    /// Only masks that carry path keys are listed — a still mask's own
    /// vertices already say where it is, and sending them again would put the
    /// whole document through here on every frame. An empty answer, which is
    /// the ordinary case, means "nothing moved; use what you have".
    ///
    /// Evaluated engine-side on purpose: interpolating two paths means
    /// reconciling vertex counts by splitting cubics (K-339), and a second
    /// implementation of that in Dart would drift from the one that draws the
    /// pixels — the wireframe would stop matching the mask it describes.
    #[frb(sync)]
    pub fn animated_mask_paths_at(
        &self,
        frame: i64,
    ) -> Result<Vec<BridgeAnimatedMaskPath>, BridgeError> {
        let comp = self.composition()?;
        // Not clamped at zero: a layer may start before the composition, and
        // its masks are keyed on its own clock either way.
        let time = comp
            .frame_rate
            .time_of_frame(frame)
            .map_err(|_| BridgeError::InvalidTime)?;
        let mut out = Vec::new();
        for layer in &comp.layers {
            // A mask lives on its layer's clock, as every other property does
            // (K-213).
            let local = time.0.checked_sub(layer.start_offset.0).unwrap_or(time.0);
            for mask in &layer.masks {
                if mask.path_keys.is_empty() {
                    continue;
                }
                let path = mask.path_at(local.to_f64());
                out.push(BridgeAnimatedMaskPath {
                    layer: layer.id,
                    mask: mask.id,
                    vertices: path
                        .vertices
                        .iter()
                        .map(|v| crate::api::layer::BridgeVertex {
                            x: v.pos.0,
                            y: v.pos.1,
                            tan_in_x: v.tan_in.0,
                            tan_in_y: v.tan_in.1,
                            tan_out_x: v.tan_out.0,
                            tan_out_y: v.tan_out.1,
                        })
                        .collect(),
                });
            }
        }
        Ok(out)
    }

    #[frb(sync)]
    pub fn get_layers(&self) -> Result<Vec<LayerReference>, BridgeError> {
        Ok(self
            .composition()?
            .layers
            .iter()
            .map(|i| LayerReference::new(self.project, self.id, i.id))
            .collect())
    }

    /// Hand a render request to the worker.
    ///
    /// Requests are not queued up behind each other: the worker drains its
    /// channel to the newest before rendering, so asking faster than it can
    /// render simply drops the frames in between rather than working through a
    /// backlog nothing will ever see. That is also why no request carries a
    /// generation — one worker thread renders sequentially and publishes down one
    /// stream, so responses arrive in the order they were asked for and the last
    /// one is always the newest. (The `TODO` that used to sit here asked for
    /// generations to stop out-of-order frames; with the queue coalescing and a
    /// single worker there is no out-of-order case for them to fix.)
    #[frb(ignore)]
    fn dispatch(&self, request: WorkerRequest) -> Result<(), BridgeError> {
        let p = self.project()?;
        let p = p.read().map_err(|_| BridgeError::ReadFailed)?;

        let Some(sender) = &p.sender else {
            return Err(BridgeError::InvalidWorkerState);
        };

        sender.send(request).map_err(|err| {
            println!("Error while requesting render: {err:?}");
            BridgeError::InvalidWorkerState
        })
    }

    /// Ask for `frame` at `scale` — 1.0 meaning "shown at comp resolution".
    /// Below 1.0 the engine decodes and composites smaller, which is how a
    /// Viewer that is not filling the screen stays cheap.
    #[frb(sync)]
    pub fn render_frame(
        &self,
        frame: u64,
        scale: f32,
        mode: BridgePlaybackMode,
    ) -> Result<(), BridgeError> {
        self.dispatch(RenderComp(RenderCompRequest {
            comp: self.clone(),
            frame,
            scale,
            mode,
        }))
    }

    /// Play from `from` at this comp's own rate, with sound.
    ///
    /// The frontend calls this and then paints whatever frames arrive: each one
    /// says which frame it is, so the transport and the playhead follow the
    /// picture rather than predicting it. Playback stops when the frontend says
    /// so ([`Self::stop_playback`]) or when it runs off the end, which arrives as
    /// `WorkerResponse::PlaybackEnded`.
    ///
    /// The sound is started here too, so "play" is one call rather than a pair
    /// the frontend has to keep in step — in BOTH modes. Every-frame used to
    /// play silent outright, which was coarser than K-171 asks for: sound
    /// plays while rendering keeps the comp's rate (which, cached, it mostly
    /// does now), and the worker PAUSES it if the picture falls genuinely
    /// behind — a paused track is honest, a drifting one is a lie in sync's
    /// clothing. Timestretch-to-match is K-171's recorded "later".
    ///
    /// `mode` comes from the frontend because it is a user *setting*, kept in the
    /// workspace file the frontend owns — stating it is not deciding anything.
    #[frb(sync)]
    pub fn play(&self, from: u64, scale: f32, mode: BridgePlaybackMode) -> Result<(), BridgeError> {
        // The mix's document is snapshotted HERE — it must be the comp as it
        // was when play was pressed — but the sound is started by the worker,
        // once it has banked a frame or two to start alongside it (the
        // pre-roll, docs/impl/playback-scheduler.md §5). Starting it here meant
        // the sound ran while the first frame was still being composited, and
        // adaptive playback then skipped to catch up: every press of play began
        // with a jump.
        let audio = {
            let state = self.project()?;
            let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
            state.store.snapshot()
        };
        // Building the mix means decoding, which is slow and asynchronous, so it
        // is kicked off HERE rather than after the pre-roll: the decode then
        // overlaps the first renders instead of following them. Only the "now
        // play" waits. A prepare of a mix already loaded is recognised by its
        // signature and costs nothing.
        self.audio_prepare()?;

        self.dispatch(WorkerRequest::Play(
            crate::api::worker_thread::PlayRequest {
                comp: self.clone(),
                from,
                mode,
                scale,
                audio,
            },
        ))
    }

    /// Set what the Viewer looks *through*, whole: `stops` of exposure and
    /// whether the tone map is engaged (K-314, docs/07 §2.2 items 12-13), and
    /// whether the comp's background colour is left out of the composite so
    /// the transparency grid can show through what nothing covers (K-352).
    ///
    /// **Preview only.** It moves how every frame the session renderer makes
    /// from here on is composited and display-encoded, and nothing else — no
    /// document, no op, no undo step. An export builds its own renderer and
    /// this is never sent to it, so an export is neutral and draws the
    /// backdrop by construction.
    ///
    /// One call carrying the whole look, so the renderer can never hold half
    /// of one. The frontend follows a *change* with its ordinary request for
    /// the frame under the playhead: a setting changes what the next frame is
    /// made of, and without an ask the picture would not move until something
    /// else did.
    #[frb(sync)]
    pub fn set_viewer_look(
        &self,
        stops: f64,
        tone_map: bool,
        transparent_background: bool,
        region: Option<Vec<f32>>,
    ) -> Result<(), BridgeError> {
        // A region arrives as a list because that is what crosses the bridge
        // cleanly; anything that is not four numbers is no region, which is
        // also how "cleared" is said.
        let region = region.and_then(|r| <[f32; 4]>::try_from(r.as_slice()).ok());
        self.dispatch(WorkerRequest::SetViewerLook {
            stops,
            tone_map,
            transparent_background,
            region,
        })
    }

    /// Stop playing, and silence the sound. Harmless when nothing is playing.
    #[frb(sync)]
    pub fn stop_playback(&self) -> Result<(), BridgeError> {
        crate::api::audio::audio_pause();
        self.dispatch(WorkerRequest::StopPlayback)
    }

    /// This composition's rate as a plain number, for turning frames into
    /// seconds. Falls back to 60 for a comp with a nonsense rate rather than
    /// dividing by zero.
    #[frb(sync)]
    pub fn fps(&self) -> f64 {
        self.composition()
            .map(|c| c.frame_rate.fps())
            .ok()
            .filter(|fps| *fps > 0.0)
            .unwrap_or(60.0)
    }

    /// The preview tier adaptive playback has settled on: 1 Full, 2 Half,
    /// 3 Third, 4 Quarter. Shown beside the mode so "why is it soft?" has an
    /// answer on screen rather than in a log.
    #[frb(sync)]
    pub fn playback_tier(&self) -> u32 {
        crate::realtime::tier()
    }

    /// Ask for `frame` with `layer`'s effect stack replaced by `effects` — the
    /// live drag path, which never touches the document.
    #[frb(sync)]
    pub fn render_frame_with_preview(
        &self,
        frame: u64,
        scale: f32,
        layer: LayerReference,
        effects: Vec<BridgeEffectInstance>,
    ) -> Result<(), BridgeError> {
        self.dispatch(RenderCompWithPreview(RenderCompRequestWithPreview {
            comp: self.clone(),
            frame,
            scale,
            layer,
            effects: Some(effects.iter().map(|i| i.get_effects()).collect()),
            transform: None,
            text: None,
            paint: None,
            contents: None,
            masks: None,
            clip_retime: None,
            retime: None,
        }))
    }

    /// Ask for `frame` with one clip's retime replaced — the live envelope
    /// drag, which never touches the document.
    ///
    /// A retime decides *which frame of the source* is decoded, so unlike a
    /// transform it cannot be previewed by re-compositing pixels that are
    /// already in hand: the provisional map has to reach the render plan, and
    /// it does that by riding along with the request and being patched onto a
    /// clone (K-247).
    #[frb(sync)]
    pub fn render_frame_with_clip_retime(
        &self,
        frame: u64,
        scale: f32,
        layer: LayerReference,
        clip: Uuid,
        retime: crate::api::effect::BridgeScalar,
    ) -> Result<(), BridgeError> {
        self.dispatch(RenderCompWithPreview(RenderCompRequestWithPreview {
            comp: self.clone(),
            frame,
            scale,
            layer,
            effects: None,
            transform: None,
            text: None,
            paint: None,
            contents: None,
            masks: None,
            clip_retime: Some((clip, retime)),
            retime: None,
        }))
    }

    /// Ask for `frame` with one layer's Retime map replaced — the live graph
    /// drag on the Retime channel, which never touches the document.
    ///
    /// The same reason as [`Self::render_frame_with_clip_retime`] one function
    /// up, for the layer's own map (K-197) rather than a clip's: a retime
    /// decides *which frame of the source* is decoded, so it cannot be
    /// previewed by re-compositing pixels already in hand. Without it the
    /// picture does not move until the key is let go, which is the one edit
    /// where watching it matters most.
    ///
    /// `retime` arrives on the comp clock like every keyframed value that
    /// crosses the seam (K-213); the worker returns it to the layer's own.
    #[frb(sync)]
    pub fn render_frame_with_retime(
        &self,
        frame: u64,
        scale: f32,
        layer: LayerReference,
        retime: crate::api::effect::BridgeScalar,
    ) -> Result<(), BridgeError> {
        self.dispatch(RenderCompWithPreview(RenderCompRequestWithPreview {
            comp: self.clone(),
            frame,
            scale,
            layer,
            effects: None,
            transform: None,
            text: None,
            paint: None,
            contents: None,
            masks: None,
            clip_retime: None,
            retime: Some(retime),
        }))
    }

    /// The exact time frame `frame` starts at, as the rational the document
    /// stores.
    ///
    /// Exposed rather than left to Dart because keyframe times must be exact
    /// (docs/14 §2): at 29.97 fps a frame is 1001/30000 s, and a panel that
    /// worked that out in floating point would place keys that do not land on
    /// the frame they were set on. This is the engine's own
    /// `FrameRate::time_of_frame`, so there is one implementation of it.
    #[frb(sync)]
    pub fn time_of_frame(&self, frame: i64) -> Result<BridgeRational, BridgeError> {
        let comp = self.composition()?;
        // **Negative frames are real.** A layer may start before the composition
        // does, so this must answer for frames below zero rather than clamping
        // them to it — clamping here pinned a bar to the comp edge however far
        // left it was dragged.
        let time = comp
            .frame_rate
            .time_of_frame(frame)
            .map_err(|_| BridgeError::InvalidComp)?;
        Ok(BridgeRational {
            num: time.0.num(),
            den: time.0.den(),
        })
    }

    /// The frame containing `time` (floored) — the inverse of
    /// [`Self::time_of_frame`], for drawing a key at a frame position.
    #[frb(sync)]
    pub fn frame_at_time(&self, time: BridgeRational) -> Result<i64, BridgeError> {
        let comp = self.composition()?;
        let rational = lumit_core::time::Rational::new(time.num, time.den)
            .map_err(|_| BridgeError::InvalidComp)?;
        Ok(comp
            .frame_rate
            .frame_at(lumit_core::time::CompTime(rational)))
    }

    /// Ask the worker for a scope trace of `frame`.
    ///
    /// `kind` is the trace: 0 waveform, 1 parade, 2 vectorscope, 3 histogram.
    /// `colours` is background, trace, then the R, G and B tints, each as
    /// `[r, g, b]` — the panel's theme decides them, so the engine never has to
    /// know what a Lumit surface looks like.
    #[frb(sync)]
    pub fn render_scope(
        &self,
        frame: u64,
        scale: f32,
        kind: u32,
        colours: Vec<Vec<u8>>,
    ) -> Result<(), BridgeError> {
        // Five triples, and anything else is a caller bug rather than something
        // to pad out with a colour nobody chose.
        if colours.len() != 5 || colours.iter().any(|c| c.len() != 3) {
            return Err(BridgeError::InvalidScopeColours);
        }
        let mut packed = [[0_u8; 3]; 5];
        for (slot, colour) in packed.iter_mut().zip(&colours) {
            slot.copy_from_slice(colour);
        }

        self.dispatch(WorkerRequest::TraceScope(RenderScopeRequest {
            comp: self.clone(),
            frame,
            scale,
            kind,
            colours: packed,
        }))
    }

    /// Ask the worker for the pixels under the dropper: a `window × window`
    /// square of `frame` centred on the point `(u, v)` of the picture, each a
    /// fraction from 0 to 1 (docs/07 §6.1).
    ///
    /// **A fraction, not a pixel, and that is the point.** The picture actually
    /// read may be a reduced-resolution preview, so its pixel grid is neither
    /// the composition's nor anything the caller can know in advance. The reply
    /// says which raster it cut from (`width`, `height`) and where in that
    /// raster the window's centre landed, and every pixel the caller then names
    /// is in that same raster. Asking in composition pixels and indexing the
    /// reply with them is a real bug that has been made unwritable here: with a
    /// fitted Viewer the two grids differ by the preview scale, and the
    /// magnifier showed one repeated edge pixel — a flat colour where the
    /// picture should be.
    ///
    /// A window rather than the nine pixels the magnifier shows, because the
    /// pointer moves and the picture does not: the caller reads its grid out of
    /// the window it already holds and asks again only when the pointer nears
    /// the window's edge, the frame changes, or an edit lands.
    ///
    /// `layer` reads that layer **alone** rather than the composite — what a
    /// depth pick does, since a depth pass is usually hidden and so never
    /// appears in the composite at all. The answer arrives as
    /// `WorkerResponse::Sampled`, on the stream the frames and traces already
    /// ride; a frame with nothing to read publishes nothing, and the magnifier
    /// keeps what it had.
    #[frb(sync)]
    pub fn sample_pixels(
        &self,
        frame: u64,
        u: f64,
        v: f64,
        window: u32,
        scale: f32,
        layer: Option<LayerReference>,
    ) -> Result<(), BridgeError> {
        self.dispatch(WorkerRequest::SamplePixels(SamplePixelsRequest {
            comp: self.clone(),
            frame,
            scale,
            u,
            v,
            window,
            layer,
        }))
    }

    /// Ask for `frame` with `layer`'s transform replaced by `transform` — the
    /// same live-drag path as [`Self::render_frame_with_preview`], for the
    /// Transform rows. Never touches the document.
    #[frb(sync)]
    pub fn render_frame_with_transform_preview(
        &self,
        frame: u64,
        scale: f32,
        layer: LayerReference,
        transform: crate::api::layer::BridgeTransform,
    ) -> Result<(), BridgeError> {
        self.dispatch(RenderCompWithPreview(RenderCompRequestWithPreview {
            comp: self.clone(),
            frame,
            scale,
            layer,
            effects: None,
            transform: Some(transform),
            text: None,
            paint: None,
            contents: None,
            masks: None,
            clip_retime: None,
            retime: None,
        }))
    }

    /// Ask for `frame` with `layer`'s text document replaced by `document` —
    /// the same live path as the two above, for the Type tool (K-225).
    ///
    /// Typing is the one edit where the provisional value changes many times a
    /// second and the document must *not*: a `set_text` per keystroke would be
    /// an undo step per keystroke. So the tool previews as it types and writes
    /// once, when the edit ends.
    #[frb(sync)]
    pub fn render_frame_with_text_preview(
        &self,
        frame: u64,
        scale: f32,
        layer: LayerReference,
        document: crate::api::assets::BridgeTextDocument,
    ) -> Result<(), BridgeError> {
        self.dispatch(RenderCompWithPreview(RenderCompRequestWithPreview {
            comp: self.clone(),
            frame,
            scale,
            layer,
            effects: None,
            transform: None,
            text: Some(document),
            paint: None,
            contents: None,
            masks: None,
            clip_retime: None,
            retime: None,
        }))
    }

    /// Ask for `frame` with `layer`'s paint replaced by `strokes` — the same
    /// live path as the three above, for a stroke being dragged in the Timeline
    /// (K-239).
    ///
    /// A stroke's opacity is committed once, on release, so the drag is one undo
    /// step (K-238). Without a preview that also meant the picture did not move
    /// until the button came up, which is the wrong half of the trade: a value
    /// you drag has to show what it is doing. The whole list rides along rather
    /// than one stroke's opacity, because paint is stored and committed as a
    /// whole list, and a preview that took a different shape from the op would
    /// be a second way to describe the same thing.
    #[frb(sync)]
    pub fn render_frame_with_paint_preview(
        &self,
        frame: u64,
        scale: f32,
        layer: LayerReference,
        strokes: Vec<crate::api::layer::BridgeStroke>,
    ) -> Result<(), BridgeError> {
        self.dispatch(RenderCompWithPreview(RenderCompRequestWithPreview {
            comp: self.clone(),
            frame,
            scale,
            layer,
            effects: None,
            transform: None,
            text: None,
            paint: Some(strokes),
            contents: None,
            masks: None,
            clip_retime: None,
            retime: None,
        }))
    }

    /// Ask for `frame` with `layer`'s art replaced by `contents` — the shape
    /// layer's half of the call above (K-239).
    ///
    /// `transform` is for the one caller that needs both at once: a point drag
    /// that moves the art's bounding box has to move the layer with it, or the
    /// preview shows the untouched art sliding and the commit puts it back
    /// (K-308). Every other caller passes `None`.
    #[frb(sync)]
    pub fn render_frame_with_shape_preview(
        &self,
        frame: u64,
        scale: f32,
        layer: LayerReference,
        contents: Vec<crate::api::layer::BridgeShapeItem>,
        transform: Option<crate::api::layer::BridgeTransform>,
    ) -> Result<(), BridgeError> {
        self.dispatch(RenderCompWithPreview(RenderCompRequestWithPreview {
            comp: self.clone(),
            frame,
            scale,
            layer,
            effects: None,
            transform,
            text: None,
            paint: None,
            contents: Some(contents),
            masks: None,
            clip_retime: None,
            retime: None,
        }))
    }

    /// Ask for `frame` with `layer`'s masks replaced by `masks` — the mask's
    /// half of the two calls above (K-240).
    #[frb(sync)]
    pub fn render_frame_with_mask_preview(
        &self,
        frame: u64,
        scale: f32,
        layer: LayerReference,
        masks: Vec<crate::api::layer::BridgeMask>,
    ) -> Result<(), BridgeError> {
        self.dispatch(RenderCompWithPreview(RenderCompRequestWithPreview {
            comp: self.clone(),
            frame,
            scale,
            layer,
            effects: None,
            transform: None,
            text: None,
            paint: None,
            contents: None,
            clip_retime: None,
            retime: None,
            masks: Some(masks),
        }))
    }
}
