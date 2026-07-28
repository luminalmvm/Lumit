use std::sync::Arc;

use flutter_rust_bridge::frb;
use lumit_core::model::{EffectInstance, Layer};

use uuid::Uuid;

use crate::api::{
    effect::{BridgeEffectInstance, BridgeRational, BridgeScalar},
    project_item::ItemReference,
    state::{LumitBridgeState, PROJECTS},
    BridgeError,
};

/// A layer's on/off switches, read as a group because the Timeline draws them
/// as one column block and reading them one at a time would be six crossings
/// per row per frame.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeLayerSwitches {
    pub visible: bool,
    pub audible: bool,
    pub locked: bool,
    pub solo: bool,
    /// 2.5D: positions in z and honours the active camera (K-023).
    pub three_d: bool,
    /// The fx switch: bypass the whole effect stack (docs/08 §1.5).
    pub fx: bool,
    /// Per-layer motion blur (K-120); only blurs when the comp's master
    /// shutter is also on.
    pub motion_blur: bool,
    /// Precomp layers only: collapse transformations (docs/06 §1.4).
    pub collapse: bool,
    /// Shy (docs/07 §4.2): hidden from the Timeline's list while the comp's
    /// shy filter is on. Never changes what renders.
    pub shy: bool,
}

/// Which switch an edit names. One enum rather than eight methods so the
/// Timeline's switch column is one handler, and so a new switch cannot be added
/// engine-side without the compiler pointing at every arm here.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeLayerSwitch {
    Visible,
    Audible,
    Locked,
    Solo,
    ThreeD,
    Fx,
    MotionBlur,
    Collapse,
    Shy,
}

/// Where a layer sits on the comp timeline, in exact rational seconds.
///
/// `start_offset` is where the layer's own time 0 falls, which is what a slip
/// edit moves and what makes trimming the in point *not* re-time the content.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeSpan {
    pub in_point: BridgeRational,
    /// Exclusive; must be after `in_point`.
    pub out_point: BridgeRational,
    pub start_offset: BridgeRational,
}

/// What kind of source a layer has — what the Timeline draws its bar and its
/// label colour from. The payloads the model carries (the footage item, the
/// text document, the clip list) are reached through their own readers rather
/// than duplicated here.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeLayerKind {
    Footage,
    Solid,
    Precomp,
    Text,
    Camera,
    Sequence,
    Adjustment,
    NullObject,
}

/// One clip on a Sequence layer, as the Timeline needs to draw it: where it
/// starts on the layer's own timeline and how long it occupies there.
///
/// The source trim and the retime map are not carried: nothing draws them yet,
/// and a value type that pretends to round-trip what no control can edit is how
/// a write quietly loses information.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeClip {
    pub id: Uuid,
    pub place_start: BridgeRational,
    pub place_duration: BridgeRational,
}

/// Everything the Timeline outline, its bars, and the Hierarchy draw for one
/// layer, in one crossing (K-183). Read one getter at a time this cost
/// seven-plus bridge calls per row per rebuild — each cloning the composition
/// out of the snapshot — plus two time↔frame trips per bar.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeLayerInfo {
    pub name: String,
    pub kind: BridgeLayerKind,
    pub switches: BridgeLayerSwitches,
    /// Blend mode as an index into `list_blend_modes`.
    pub blend: u32,
    pub span: BridgeSpan,
    /// The span at the comp's own rate, so drawing needs no time↔frame trips.
    pub in_frame: i64,
    pub out_frame: i64,
    /// Sequence clip starts as comp frames (empty on other kinds) — what the
    /// bar draws its split lines from.
    pub clip_frames: Vec<i64>,
    pub parent: Option<Uuid>,
    /// The parent layer's current name, so the outline's parent picker renders
    /// with no second lookup. None when there is no parent, or it is dangling.
    pub parent_name: Option<String>,
    /// The whole transform, one scalar per property (K-184).
    pub transform: BridgeTransform,
    /// Every effect on the layer, with every parameter's value (K-184). Plain
    /// data for *drawing*; an edit reads fresh instance handles at commit time.
    pub effects: Vec<crate::api::effect::BridgeEffectInstanceInfo>,
    /// The label colour index (0-7), drawn as the outline's swatch.
    pub label: u8,
    /// The layer's matte, for the outline's matte cell (K-184: the row draws
    /// with no bridge calls). Writes still go through `set_matte`.
    pub matte: Option<BridgeMatte>,
    /// The Retime property (K-197), or None when the layer is not retimed —
    /// which is exactly what decides whether the fold-out shows a Retime row.
    pub retime: Option<BridgeScalar>,
}

/// Build one layer's [`BridgeLayerInfo`] from an already-fetched composition —
/// the shared body of [`LayerReference::get_info`] and the comp-wide
/// [`crate::api::composition::CompositionReference::get_model`] (K-184).
#[frb(ignore)]
pub(crate) fn read_layer_info(
    comp: &lumit_core::model::Composition,
    layer: &Layer,
) -> BridgeLayerInfo {
    use lumit_core::model::LayerKind as K;
    let clip_frames = match &layer.kind {
        K::Sequence { clips } => clips
            .iter()
            .map(|c| {
                comp.frame_rate
                    .frame_at(lumit_core::time::CompTime(c.place_start))
            })
            .collect(),
        _ => Vec::new(),
    };
    let s = layer.switches;
    BridgeLayerInfo {
        name: layer.name.clone(),
        kind: match &layer.kind {
            K::Footage { .. } => BridgeLayerKind::Footage,
            K::Solid { .. } => BridgeLayerKind::Solid,
            K::Precomp { .. } => BridgeLayerKind::Precomp,
            K::Text { .. } => BridgeLayerKind::Text,
            K::Camera { .. } => BridgeLayerKind::Camera,
            K::Sequence { .. } => BridgeLayerKind::Sequence,
            K::Adjustment => BridgeLayerKind::Adjustment,
            K::NullObject => BridgeLayerKind::NullObject,
        },
        switches: BridgeLayerSwitches {
            visible: s.visible,
            audible: s.audible,
            locked: s.locked,
            solo: s.solo,
            three_d: s.three_d,
            fx: s.fx,
            motion_blur: s.motion_blur,
            collapse: s.collapse,
            shy: s.shy,
        },
        blend: lumit_core::model::BlendMode::ALL
            .iter()
            .position(|b| *b == layer.blend)
            .unwrap_or(0) as u32,
        span: BridgeSpan {
            in_point: rational_of(layer.in_point.0),
            out_point: rational_of(layer.out_point.0),
            start_offset: rational_of(layer.start_offset.0),
        },
        in_frame: comp.frame_rate.frame_at(layer.in_point),
        out_frame: comp.frame_rate.frame_at(layer.out_point),
        clip_frames,
        parent: layer.parent,
        parent_name: layer.parent.and_then(|p| {
            comp.layers
                .iter()
                .find(|l| l.id == p)
                .map(|l| l.name.clone())
        }),
        transform: BridgeTransform::read(&layer.transform),
        effects: layer
            .effects
            .iter()
            .map(crate::api::effect::read_instance_info)
            .collect(),
        label: layer.label,
        matte: layer.matte.as_ref().map(|m| BridgeMatte {
            layer: m.layer,
            luma: matches!(m.channel, lumit_core::model::MatteChannel::Luma),
            inverted: m.inverted,
        }),
        retime: layer.retime.as_ref().map(BridgeScalar::read),
    }
}

/// A footage layer's waveform peaks: the whole source bucketed to a fixed
/// count, plus its length so the lane can map comp time onto buckets.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeAudioPeaks {
    pub duration_seconds: f64,
    /// Interleaved `[min0, max0, min1, max1, …]`, each in −1..1.
    pub pairs: Vec<f32>,
}

/// A layer used as another layer's matte (docs/03 §5.1).
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeMatte {
    pub layer: Uuid,
    /// Whether the matte reads the source's alpha or its luminance.
    pub luma: bool,
    pub inverted: bool,
}

/// A layer's whole transform, one scalar per property.
///
/// Read as a group rather than a property at a time because the panel draws them
/// as a group and a drag on one axis previews the others unchanged — eleven
/// round trips per frame to rebuild what one call already has would be the
/// snapshot habit creeping back in. Writing is per-property (see
/// [`LayerReference::set_transform`]), which is what keeps each edit exactly
/// invertible.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeTransform {
    pub anchor_x: BridgeScalar,
    pub anchor_y: BridgeScalar,
    pub position_x: BridgeScalar,
    pub position_y: BridgeScalar,
    /// The 2.5D depth (K-023). Present on every layer; only meaningful, and only
    /// drawn, when the layer's 3D switch is on.
    pub position_z: BridgeScalar,
    /// Percent, 100 = natural size.
    pub scale_x: BridgeScalar,
    pub scale_y: BridgeScalar,
    /// Degrees, about z — the 2D rotation.
    pub rotation: BridgeScalar,
    pub rotation_x: BridgeScalar,
    pub rotation_y: BridgeScalar,
    /// Percent, 0..100.
    pub opacity: BridgeScalar,
}

/// Which transform property an edit names ([`lumit_core::model::TransformProp`]).
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeTransformProp {
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

/// A document rational as the integer pair the bridge carries.
#[frb(ignore)]
fn rational_of(r: lumit_core::time::Rational) -> BridgeRational {
    BridgeRational {
        num: r.num(),
        den: r.den(),
    }
}

/// The inverse. A zero or negative denominator is refused rather than
/// normalised: it means the caller built a time wrongly, and quietly fixing it
/// would put a span somewhere nobody asked for.
#[frb(ignore)]
fn comp_time(r: BridgeRational) -> Result<lumit_core::time::Rational, BridgeError> {
    lumit_core::time::Rational::new(r.num, r.den).map_err(|_| BridgeError::InvalidTime)
}

impl BridgeTransformProp {
    #[frb(ignore)]
    pub(crate) fn core(self) -> lumit_core::model::TransformProp {
        use lumit_core::model::TransformProp as P;
        match self {
            BridgeTransformProp::AnchorX => P::AnchorX,
            BridgeTransformProp::AnchorY => P::AnchorY,
            BridgeTransformProp::PositionX => P::PositionX,
            BridgeTransformProp::PositionY => P::PositionY,
            BridgeTransformProp::PositionZ => P::PositionZ,
            BridgeTransformProp::ScaleX => P::ScaleX,
            BridgeTransformProp::ScaleY => P::ScaleY,
            BridgeTransformProp::Rotation => P::Rotation,
            BridgeTransformProp::RotationX => P::RotationX,
            BridgeTransformProp::RotationY => P::RotationY,
            BridgeTransformProp::Opacity => P::Opacity,
        }
    }
}

impl BridgeTransform {
    #[frb(ignore)]
    pub(crate) fn read(group: &lumit_core::model::TransformGroup) -> BridgeTransform {
        BridgeTransform {
            anchor_x: BridgeScalar::read(&group.anchor_x),
            anchor_y: BridgeScalar::read(&group.anchor_y),
            position_x: BridgeScalar::read(&group.position_x),
            position_y: BridgeScalar::read(&group.position_y),
            position_z: BridgeScalar::read(&group.position_z),
            scale_x: BridgeScalar::read(&group.scale_x),
            scale_y: BridgeScalar::read(&group.scale_y),
            rotation: BridgeScalar::read(&group.rotation),
            rotation_x: BridgeScalar::read(&group.rotation_x),
            rotation_y: BridgeScalar::read(&group.rotation_y),
            opacity: BridgeScalar::read(&group.opacity),
        }
    }

    /// Write this whole group onto `target`, for the drag preview — which needs
    /// a document to render, not an op to commit.
    #[frb(ignore)]
    pub(crate) fn write(
        &self,
        target: &mut lumit_core::model::TransformGroup,
    ) -> Result<(), BridgeError> {
        target.anchor_x.animation = self.anchor_x.animation()?;
        target.anchor_y.animation = self.anchor_y.animation()?;
        target.position_x.animation = self.position_x.animation()?;
        target.position_y.animation = self.position_y.animation()?;
        target.position_z.animation = self.position_z.animation()?;
        target.scale_x.animation = self.scale_x.animation()?;
        target.scale_y.animation = self.scale_y.animation()?;
        target.rotation.animation = self.rotation.animation()?;
        target.rotation_x.animation = self.rotation_x.animation()?;
        target.rotation_y.animation = self.rotation_y.animation()?;
        target.opacity.animation = self.opacity.animation()?;
        Ok(())
    }
}

#[derive(Debug)]
#[frb]
pub struct LayerReference {
    #[frb(name = "internalprojectId")]
    pub project_id: Uuid,

    #[frb(name = "internalcompId")]
    pub comp_id: Uuid,

    #[frb(name = "internallayerId")]
    pub layer_id: Uuid,
}

impl LayerReference {
    #[frb(ignore)]
    pub fn new(project_id: Uuid, comp_id: Uuid, layer_id: Uuid) -> LayerReference {
        LayerReference {
            project_id,
            comp_id,
            layer_id,
        }
    }

    #[frb(ignore)]
    pub fn project_id(&self) -> Uuid {
        self.project_id
    }

    #[frb(ignore)]
    pub fn comp_id(&self) -> Uuid {
        self.comp_id
    }

    #[frb(ignore)]
    pub fn id(&self) -> Uuid {
        self.layer_id
    }

    #[frb(ignore)]
    fn project(&self) -> Result<Arc<std::sync::RwLock<LumitBridgeState>>, BridgeError> {
        let projects = PROJECTS.read().map_err(|_| BridgeError::ReadFailed)?;
        let project = projects.get(&self.project_id);

        let p = project.ok_or(BridgeError::InvalidProject)?;
        Ok(p.clone())
    }

    /// The composition this layer lives in, cloned out of the current snapshot.
    /// The read lock is released by the time it returns, so a caller is free to
    /// take the write lock next.
    #[frb(ignore)]
    fn composition(&self) -> Result<lumit_core::model::Composition, BridgeError> {
        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let snapshot = proj.store.snapshot();

        match snapshot
            .item(self.comp_id)
            .ok_or(BridgeError::InvalidItem)?
        {
            lumit_core::model::ProjectItem::Composition(composition) => Ok(composition.clone()),
            _ => Err(BridgeError::InvalidItem),
        }
    }

    #[frb(ignore)]
    pub(crate) fn item(&self) -> Result<Layer, BridgeError> {
        self.composition()?
            .layers
            .into_iter()
            .find(|l| l.id == self.layer_id)
            .ok_or(BridgeError::InvalidLayer)
    }

    #[frb(sync)]
    pub fn equals(&self, layer: &LayerReference) -> bool {
        self.comp_id == layer.comp_id
            && self.project_id == layer.project_id
            && self.layer_id == layer.layer_id
    }

    #[frb(sync)]
    pub fn get_name(&self) -> Result<String, BridgeError> {
        let item = self.item()?;

        Ok(item.name)
    }

    /// One read for everything a row draws — see [`BridgeLayerInfo`]. One
    /// document lock and one crossing, where the per-field getters cost one of
    /// each per field.
    #[frb(sync)]
    pub fn get_info(&self) -> Result<BridgeLayerInfo, BridgeError> {
        let comp = self.composition()?;
        let layer = comp
            .layers
            .iter()
            .find(|l| l.id == self.layer_id)
            .ok_or(BridgeError::InvalidLayer)?;
        Ok(read_layer_info(&comp, layer))
    }

    #[frb(sync)]
    pub fn rename(&self, name: String) -> Result<(), BridgeError> {
        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;

        proj.store
            .commit(lumit_core::Op::RenameLayer {
                comp: self.comp_id,
                layer: self.layer_id,
                name,
            })
            .map_err(BridgeError::OpError)?;

        Ok(())
    }

    /// Serialise this layer's whole effect stack to `.lumfx` JSON.
    ///
    /// Returns the text rather than writing a file: choosing where something
    /// goes is the file picker's job, and the engine has no business opening one.
    /// A layer with no effects still saves — an empty preset is a valid, if
    /// unexciting, document.
    #[frb(sync)]
    pub fn save_preset(&self, name: String) -> Result<String, BridgeError> {
        let effects = self.item()?.effects;
        serde_json::to_string_pretty(&serde_json::json!({
            "format": 1,
            "name": name,
            "effects": effects,
        }))
        .map_err(|_| BridgeError::InvalidPreset)
    }

    /// Append a `.lumfx` preset's effects to this layer's stack, as one op.
    ///
    /// Each arrives with a **fresh** instance id (K-065): applying one preset to
    /// several layers must not give them effects that share an id, since an id
    /// is instance identity and every op that names an effect uses it.
    ///
    /// A document written by a newer Lumit still loads — unknown fields ride
    /// along in each effect's `extra` map, exactly as the project file tolerates
    /// additions. Only text that is not a preset at all is refused.
    #[frb(sync)]
    pub fn load_preset(&self, text: String) -> Result<(), BridgeError> {
        #[derive(serde::Deserialize)]
        struct Preset {
            effects: Vec<EffectInstance>,
        }

        let preset: Preset = serde_json::from_str(&text).map_err(|_| BridgeError::InvalidPreset)?;
        let fresh: Vec<EffectInstance> = preset
            .effects
            .into_iter()
            .map(|mut effect| {
                effect.id = Uuid::now_v7();
                effect
            })
            .collect();

        self.with_effects(move |effects| {
            effects.extend(fresh);
            Ok(())
        })
    }

    /// The clips on this Sequence layer, in the order it holds them.
    ///
    /// An empty list on a layer that is not a Sequence, rather than an error:
    /// the Timeline asks every row whether it has clips to draw, and a footage
    /// row simply has none.
    #[frb(sync)]
    pub fn get_clips(&self) -> Result<Vec<BridgeClip>, BridgeError> {
        let layer = self.item()?;
        let lumit_core::model::LayerKind::Sequence { clips } = &layer.kind else {
            return Ok(Vec::new());
        };
        Ok(clips
            .iter()
            .map(|c| BridgeClip {
                id: c.id,
                place_start: rational_of(c.place_start),
                place_duration: rational_of(c.place_duration),
            })
            .collect())
    }

    /// Razor: cut the clip under `frame` in two, at the playhead.
    ///
    /// The two halves keep their places — a cut must not shift what comes after
    /// it, which is the beat-sync covenant (K-071). An eased ramp that cannot be
    /// split cleanly at this time is a calm error, exactly as the egui razor
    /// reports it, rather than a cut that silently changes the speed curve.
    #[frb(sync)]
    pub fn cut_clip_at(&self, frame: i64) -> Result<(), BridgeError> {
        let (mut clips, index, tau) = self.clip_under(frame)?;
        let (left, right) = clips[index].cut(tau).ok_or(BridgeError::UncuttableClip)?;
        clips.splice(index..=index, [left, right]);
        self.commit_clips(clips)
    }

    /// Delete the clip under `frame`, leaving a gap.
    ///
    /// A gap is legal on the Vegas surface (K-071), so the clips after it stay
    /// where they are rather than rippling back — again so a cut never moves
    /// anything that was already in time with the music.
    #[frb(sync)]
    pub fn delete_clip_at(&self, frame: i64) -> Result<(), BridgeError> {
        let (mut clips, index, _) = self.clip_under(frame)?;
        clips.remove(index);
        self.commit_clips(clips)
    }

    /// Turn a Footage layer into a Sequence layer holding one clip of the whole
    /// source — the way into the clip-editing surface.
    ///
    /// Remove-then-add at the same index rather than an in-place kind change,
    /// because a layer's kind is not something any single op edits; the batch
    /// makes it one undo step. Only footage converts.
    #[frb(sync)]
    pub fn convert_to_sequenced(&self) -> Result<(), BridgeError> {
        use lumit_core::model::LayerKind;
        use lumit_core::sequence::{Clip, ClipSource};
        use lumit_core::time::Rational;

        let layer = self.item()?;
        let LayerKind::Footage { item, retime } = &layer.kind else {
            return Err(BridgeError::NotFootage);
        };
        let comp = self.composition()?;
        let index = comp
            .layers
            .iter()
            .position(|l| l.id == self.layer_id)
            .ok_or(BridgeError::InvalidLayer)?;

        // The layer's own span length is the fallback when the media has not
        // probed; a quarter-second floor keeps a clip from being unclickable.
        let span = (layer.out_point.0.to_f64() - layer.in_point.0.to_f64()).max(0.04);
        let duration =
            Rational::from_f64_on_grid(span, Rational::FLICK_DEN).unwrap_or(layer.out_point.0);

        let mut converted = layer.clone();
        converted.kind = LayerKind::Sequence {
            clips: vec![Clip {
                id: Uuid::now_v7(),
                source: ClipSource::Footage(*item),
                source_in: Rational::ZERO,
                source_out: duration,
                place_start: Rational::ZERO,
                place_duration: duration,
                retime: retime.clone().unwrap_or_else(|| {
                    lumit_core::retime::Retime::identity(duration, Rational::ZERO)
                }),
                interpolation: Default::default(),
                extra: serde_json::Map::new(),
            }],
        };

        self.commit(lumit_core::Op::Batch {
            ops: vec![
                lumit_core::Op::RemoveLayer {
                    comp: self.comp_id,
                    layer: self.layer_id,
                },
                lumit_core::Op::AddLayer {
                    comp: self.comp_id,
                    index,
                    layer: Box::new(converted),
                },
            ],
        })
    }

    /// The clips, the index of the one under `frame`, and the layer-local time
    /// there.
    #[frb(ignore)]
    fn clip_under(
        &self,
        frame: i64,
    ) -> Result<
        (
            Vec<lumit_core::sequence::Clip>,
            usize,
            lumit_core::time::Rational,
        ),
        BridgeError,
    > {
        let layer = self.item()?;
        let lumit_core::model::LayerKind::Sequence { clips } = &layer.kind else {
            return Err(BridgeError::NotSequence);
        };
        let comp = self.composition()?;
        let at = comp
            .frame_rate
            .time_of_frame(frame)
            .map_err(|_| BridgeError::InvalidTime)?;
        // Layer-local: the playhead less where this layer's own time 0 sits.
        let tau =
            at.0.checked_sub(layer.start_offset.0)
                .map_err(|_| BridgeError::InvalidTime)?;
        let index = clips
            .iter()
            .position(|c| c.contains(tau.to_f64()))
            .ok_or(BridgeError::NoClipThere)?;
        Ok((clips.clone(), index, tau))
    }

    #[frb(ignore)]
    fn commit_clips(&self, clips: Vec<lumit_core::sequence::Clip>) -> Result<(), BridgeError> {
        self.commit(lumit_core::Op::SetSequenceClips {
            comp: self.comp_id,
            layer: self.layer_id,
            clips,
        })
    }

    /// The project item this layer draws from, when it has one.
    ///
    /// `None` for the kinds that have no source of their own — a solid's
    /// definition, an adjustment layer, a camera, a text layer. The Viewer needs
    /// it to ask whether a footage layer's file is still there, which is what
    /// puts the missing-media slate on screen instead of a black frame.
    #[frb(sync)]
    pub fn get_source_item(&self) -> Result<Option<ItemReference>, BridgeError> {
        use lumit_core::model::LayerKind;
        let layer = self.item()?;
        let id = match layer.kind {
            LayerKind::Footage { item, .. } => item,
            LayerKind::Precomp { comp } => comp,
            LayerKind::Solid { def } => def,
            LayerKind::Text { .. }
            | LayerKind::Camera { .. }
            | LayerKind::Sequence { .. }
            | LayerKind::Adjustment => return Ok(None),
            LayerKind::NullObject => return Ok(None),
        };

        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let doc = proj.store.snapshot();
        Ok(doc
            .item(id)
            .map(|item| crate::api::project_item::item_reference(self.project_id, item)))
    }

    /// What kind of source this layer has.
    #[frb(sync)]
    pub fn get_kind(&self) -> Result<BridgeLayerKind, BridgeError> {
        use lumit_core::model::LayerKind as K;
        Ok(match self.item()?.kind {
            K::Footage { .. } => BridgeLayerKind::Footage,
            K::Solid { .. } => BridgeLayerKind::Solid,
            K::Precomp { .. } => BridgeLayerKind::Precomp,
            K::Text { .. } => BridgeLayerKind::Text,
            K::Camera { .. } => BridgeLayerKind::Camera,
            K::Sequence { .. } => BridgeLayerKind::Sequence,
            K::Adjustment => BridgeLayerKind::Adjustment,
            K::NullObject => BridgeLayerKind::NullObject,
        })
    }

    /// All eight switches at once.
    #[frb(sync)]
    pub fn get_switches(&self) -> Result<BridgeLayerSwitches, BridgeError> {
        let s = self.item()?.switches;
        Ok(BridgeLayerSwitches {
            visible: s.visible,
            audible: s.audible,
            locked: s.locked,
            solo: s.solo,
            three_d: s.three_d,
            fx: s.fx,
            motion_blur: s.motion_blur,
            collapse: s.collapse,
            shy: s.shy,
        })
    }

    /// Set one switch. One op each, so each click is one undo step.
    #[frb(sync)]
    pub fn set_switch(&self, switch: BridgeLayerSwitch, on: bool) -> Result<(), BridgeError> {
        let (comp, layer) = (self.comp_id, self.layer_id);
        self.commit(match switch {
            BridgeLayerSwitch::Visible => lumit_core::Op::SetLayerVisible {
                comp,
                layer,
                visible: on,
            },
            BridgeLayerSwitch::Audible => lumit_core::Op::SetLayerAudible {
                comp,
                layer,
                audible: on,
            },
            BridgeLayerSwitch::Locked => lumit_core::Op::SetLayerLocked {
                comp,
                layer,
                locked: on,
            },
            BridgeLayerSwitch::Solo => lumit_core::Op::SetLayerSolo {
                comp,
                layer,
                solo: on,
            },
            BridgeLayerSwitch::ThreeD => lumit_core::Op::SetLayerThreeD {
                comp,
                layer,
                three_d: on,
            },
            BridgeLayerSwitch::Fx => lumit_core::Op::SetLayerFx {
                comp,
                layer,
                fx: on,
            },
            BridgeLayerSwitch::MotionBlur => lumit_core::Op::SetLayerMotionBlur {
                comp,
                layer,
                motion_blur: on,
            },
            BridgeLayerSwitch::Collapse => lumit_core::Op::SetLayerCollapse {
                comp,
                layer,
                collapse: on,
            },
            BridgeLayerSwitch::Shy => lumit_core::Op::SetLayerShy {
                comp,
                layer,
                shy: on,
            },
        })
    }

    /// The label-colour index: which chip the Timeline draws beside the layer
    /// number, as an index into the theme's label palette (TL2).
    #[frb(sync)]
    pub fn get_label(&self) -> Result<u8, BridgeError> {
        Ok(self.item()?.label)
    }

    #[frb(sync)]
    pub fn set_label(&self, label: u8) -> Result<(), BridgeError> {
        let (comp, layer) = (self.comp_id, self.layer_id);
        self.commit(lumit_core::Op::SetLayerLabel { comp, layer, label })
    }

    /// Where this layer sits on the comp timeline.
    #[frb(sync)]
    pub fn get_span(&self) -> Result<BridgeSpan, BridgeError> {
        let layer = self.item()?;
        Ok(BridgeSpan {
            in_point: rational_of(layer.in_point.0),
            out_point: rational_of(layer.out_point.0),
            start_offset: rational_of(layer.start_offset.0),
        })
    }

    /// Move or trim the layer. One op, so a drag that changes the in point and
    /// the start offset together — a slip edit — is still one undo step.
    ///
    /// An out point at or before the in point is refused by the op rather than
    /// clamped here: a zero-length layer is not something the Timeline should be
    /// able to produce by accident, and silently widening it would hide the bug
    /// that produced it.
    #[frb(sync)]
    pub fn set_span(&self, span: BridgeSpan) -> Result<(), BridgeError> {
        use lumit_core::time::CompTime;
        let (comp, layer) = (self.comp_id, self.layer_id);
        self.commit(lumit_core::Op::SetLayerSpan {
            comp,
            layer,
            in_point: CompTime(comp_time(span.in_point)?),
            out_point: CompTime(comp_time(span.out_point)?),
            start_offset: CompTime(comp_time(span.start_offset)?),
        })
    }

    /// This layer's blend mode, as an index into [`list_blend_modes`].
    #[frb(sync)]
    pub fn get_blend(&self) -> Result<u32, BridgeError> {
        let blend = self.item()?.blend;
        Ok(lumit_core::model::BlendMode::ALL
            .iter()
            .position(|b| *b == blend)
            .unwrap_or(0) as u32)
    }

    #[frb(sync)]
    pub fn set_blend(&self, index: u32) -> Result<(), BridgeError> {
        let blend = *lumit_core::model::BlendMode::ALL
            .get(index as usize)
            .ok_or(BridgeError::InvalidBlendMode)?;
        let (comp, layer) = (self.comp_id, self.layer_id);
        self.commit(lumit_core::Op::SetLayerBlend { comp, layer, blend })
    }

    /// The layer used as this one's matte, if any.
    #[frb(sync)]
    pub fn get_matte(&self) -> Result<Option<BridgeMatte>, BridgeError> {
        use lumit_core::model::MatteChannel;
        Ok(self.item()?.matte.map(|m| BridgeMatte {
            layer: m.layer,
            luma: matches!(m.channel, MatteChannel::Luma),
            inverted: m.inverted,
        }))
    }

    /// Point this layer at another as its matte, or clear it with `None`.
    ///
    /// A matte naming a layer that is not there degrades to "no matte" at render
    /// (docs/03 §5.1 invariants), so this does not refuse one — the Timeline can
    /// set a matte and delete its target without the document becoming invalid.
    #[frb(sync)]
    pub fn set_matte(&self, matte: Option<BridgeMatte>) -> Result<(), BridgeError> {
        use lumit_core::model::{LayerInputSource, MatteChannel, MatteRef};
        let (comp, layer) = (self.comp_id, self.layer_id);
        self.commit(lumit_core::Op::SetLayerMatte {
            comp,
            layer,
            matte: matte.map(|m| MatteRef {
                layer: m.layer,
                channel: if m.luma {
                    MatteChannel::Luma
                } else {
                    MatteChannel::Alpha
                },
                inverted: m.inverted,
                source: LayerInputSource::default(),
            }),
        })
    }

    /// This layer's transform parent, if any (K-103).
    #[frb(sync)]
    pub fn get_parent(&self) -> Result<Option<Uuid>, BridgeError> {
        Ok(self.item()?.parent)
    }

    /// Parent this layer to another, or clear it with `None`.
    ///
    /// A self-parent, an unknown layer, or one that would close a cycle is
    /// refused by the op — a parent loop has no defined transform, so unlike a
    /// dangling matte it cannot be allowed to exist and be ignored later.
    #[frb(sync)]
    pub fn set_parent(&self, parent: Option<Uuid>) -> Result<(), BridgeError> {
        let (comp, layer) = (self.comp_id, self.layer_id);
        self.commit(lumit_core::Op::SetLayerParent {
            comp,
            layer,
            parent,
        })
    }

    /// Move this layer to `new_index` in the stack (0 = top).
    #[frb(sync)]
    pub fn reorder(&self, new_index: usize) -> Result<(), BridgeError> {
        let (comp, layer) = (self.comp_id, self.layer_id);
        self.commit(lumit_core::Op::ReorderLayer {
            comp,
            layer,
            new_index,
        })
    }

    /// Remove this layer from its composition.
    #[frb(sync)]
    pub fn delete(&self) -> Result<(), BridgeError> {
        let (comp, layer) = (self.comp_id, self.layer_id);
        self.item()?;
        self.commit(lumit_core::Op::RemoveLayer { comp, layer })
    }

    /// Copy this layer, inserting the copy directly above the original.
    ///
    /// The copy is a fresh layer with fresh effect ids, not a second reference
    /// to the same one: two layers sharing an id would make every op that names
    /// a layer ambiguous.
    #[frb(sync)]
    pub fn duplicate(&self) -> Result<LayerReference, BridgeError> {
        let mut copy = self.item()?;
        let comp = self.composition()?;
        let index = comp
            .layers
            .iter()
            .position(|l| l.id == self.layer_id)
            .ok_or(BridgeError::InvalidLayer)?;

        copy.id = Uuid::now_v7();
        copy.name = format!("{} copy", copy.name);
        for effect in &mut copy.effects {
            effect.id = Uuid::now_v7();
        }
        // A duplicate of a layer that was somebody's matte or parent must not
        // inherit being pointed *at* — but it keeps what it points at itself.
        let new_id = copy.id;

        self.commit(lumit_core::Op::AddLayer {
            comp: self.comp_id,
            index,
            layer: Box::new(copy),
        })?;
        Ok(LayerReference::new(self.project_id, self.comp_id, new_id))
    }

    /// Commit `op` against this layer's project.
    #[frb(ignore)]
    pub(crate) fn commit(&self, op: lumit_core::Op) -> Result<(), BridgeError> {
        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store.commit(op).map_err(BridgeError::OpError)?;
        Ok(())
    }

    /// Whether this layer positions in z and honours the active camera (K-023).
    ///
    /// Read-only for now: the switch's *toggle* is a Timeline op that has not
    /// been ported. The Effect controls panel needs the reader regardless, to
    /// decide whether to draw the z and x/y-rotation rows at all — a 2D layer
    /// showing 3D controls that do nothing is worse than not showing them. A
    /// camera is 3D by construction whatever its switch says.
    #[frb(sync)]
    pub fn is_three_d(&self) -> Result<bool, BridgeError> {
        let layer = self.item()?;
        Ok(layer.switches.three_d
            || matches!(layer.kind, lumit_core::model::LayerKind::Camera { .. }))
    }

    /// This layer's whole transform.
    #[frb(sync)]
    pub fn get_transform(&self) -> Result<BridgeTransform, BridgeError> {
        Ok(BridgeTransform::read(&self.item()?.transform))
    }

    /// Whether this layer's source actually carries sound.
    ///
    /// What decides whether the Audio group appears under a layer at all
    /// (docs/07 §4.3): every layer *has* a Volume property in the model, but on
    /// a solid or a title it can never be heard, and a control that cannot do
    /// anything is worse than no control. Footage is the case that matters, and
    /// the answer is the container's own: a file with an audio stream.
    ///
    /// Probing opens the file with FFmpeg, so this is deliberately **not**
    /// `#[frb(sync)]`. A layer whose media cannot be resolved answers false —
    /// a missing file is not a reason to offer a volume control.
    /// The layer's source audio as `buckets` (min, max) peak pairs across the
    /// WHOLE source, interleaved `[min0, max0, min1, max1, …]` (K-172). The
    /// peaks belong to the file, not the placement, so a trim or a drag never
    /// invalidates them — the Timeline's waveform lane maps them through the
    /// live in/out/offset each paint. Deliberately not `#[frb(sync)]`: it
    /// decodes the whole track. Empty when the layer has no decodable audio.
    // ponytail: decodes the file once per asking layer per session — no
    // persistent peak files yet (docs/TODO), and two layers on one file
    // decode twice. Cache per item when a real project feels it.
    pub fn audio_peaks(&self, buckets: u32) -> Result<BridgeAudioPeaks, BridgeError> {
        let empty = BridgeAudioPeaks {
            duration_seconds: 0.0,
            pairs: Vec::new(),
        };
        let layer = self.item()?;
        let lumit_core::model::LayerKind::Footage { item, .. } = layer.kind else {
            return Ok(empty);
        };
        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let snapshot = proj.store.snapshot();
        let Some(lumit_core::model::ProjectItem::Footage(footage)) = snapshot.item(item) else {
            return Ok(empty);
        };

        #[cfg(feature = "media")]
        {
            let Some(path) = crate::api::footage::FootageReference::resolve_path(&proj, footage)
            else {
                return Ok(empty);
            };
            let Ok(buffer) = lumit_media::audio::decode_all(&path, 48_000) else {
                return Ok(empty);
            };
            let pairs = lumit_audio::mix::waveform_peaks(&buffer.samples, buckets as usize);
            Ok(BridgeAudioPeaks {
                duration_seconds: buffer.duration_seconds(),
                pairs: pairs.into_iter().flat_map(|(lo, hi)| [lo, hi]).collect(),
            })
        }

        #[cfg(not(feature = "media"))]
        {
            let _ = buckets;
            Ok(empty)
        }
    }

    /// Whether this layer has a picture to sample — the mirror of
    /// [`Self::has_audio`], and what tells a matte or a layer-valued effect
    /// parameter which layers are worth offering (K-194).
    ///
    /// Every synthetic kind draws; a Camera does not (it *is* a viewpoint);
    /// footage draws only when its container carries a video stream, so an
    /// audio-only clip answers false. Probing costs an FFmpeg open, so callers
    /// ask when a menu opens, never while drawing a row.
    #[frb(sync)]
    pub fn has_picture(&self) -> Result<bool, BridgeError> {
        use lumit_core::model::LayerKind as K;
        let layer = self.item()?;
        let item = match layer.kind {
            K::Camera { .. } => return Ok(false),
            K::Footage { item, .. } => item,
            // Solids, text, precomps, sequences and adjustments all draw.
            _ => return Ok(true),
        };

        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let snapshot = proj.store.snapshot();
        let Some(lumit_core::model::ProjectItem::Footage(footage)) = snapshot.item(item) else {
            return Ok(false);
        };

        #[cfg(feature = "media")]
        {
            let Some(path) = crate::api::footage::FootageReference::resolve_path(&proj, footage)
            else {
                return Ok(false);
            };
            Ok(lumit_media::probe::probe(&path)
                .map(|p| p.video.is_some())
                .unwrap_or(false))
        }

        // Without a decoder nothing can be probed. Footage is assumed to draw
        // rather than assumed not to: the opposite would empty every matte
        // menu on a build with no media feature.
        #[cfg(not(feature = "media"))]
        {
            let _ = footage;
            Ok(true)
        }
    }

    pub fn has_audio(&self) -> Result<bool, BridgeError> {
        let layer = self.item()?;
        let lumit_core::model::LayerKind::Footage { item, .. } = layer.kind else {
            return Ok(false);
        };

        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let snapshot = proj.store.snapshot();
        let Some(lumit_core::model::ProjectItem::Footage(footage)) = snapshot.item(item) else {
            return Ok(false);
        };

        #[cfg(feature = "media")]
        {
            let Some(path) = crate::api::footage::FootageReference::resolve_path(&proj, footage)
            else {
                return Ok(false);
            };
            Ok(lumit_media::probe::probe(&path)
                .map(|p| p.audio.is_some())
                .unwrap_or(false))
        }

        // Without a decoder nothing can be probed, so nothing claims to have
        // sound rather than every footage layer claiming it.
        #[cfg(not(feature = "media"))]
        {
            let _ = footage;
            Ok(false)
        }
    }

    /// This layer's Retime property — layer-local time → source time, in
    /// seconds (K-197) — or `None` when the layer is not retimed, which is what
    /// hides the row.
    #[frb(sync)]
    pub fn get_retime_property(&self) -> Result<Option<BridgeScalar>, BridgeError> {
        Ok(self.item()?.retime.as_ref().map(BridgeScalar::read))
    }

    /// Turn Retime on or off (Alt+Shift+T), returning whether it is now on.
    ///
    /// On installs the identity map — source time running alongside local time
    /// — so switching it on changes nothing visible and gives the row something
    /// to key, exactly as AE's Time Remap does. Off removes the property
    /// rather than flattening it: "not retimed" and "retimed to exactly 1×" are
    /// different states in the file, and only the first skips the map.
    #[frb(sync)]
    pub fn toggle_retime_property(&self) -> Result<bool, BridgeError> {
        let layer = self.item()?;
        let on = layer.retime.is_none();
        let retime = on.then(|| {
            let duration = layer
                .out_point
                .0
                .checked_sub(layer.in_point.0)
                .unwrap_or(layer.out_point.0);
            Layer::identity_retime(duration)
        });
        self.commit(lumit_core::Op::SetRetimeProperty {
            comp: self.comp_id,
            layer: self.layer_id,
            retime,
        })?;
        Ok(on)
    }

    /// Replace the Retime property's whole animation, as one undoable step —
    /// the same coarse-grained shape as a transform property, for the same
    /// invertibility reason. Refused on a layer that is not retimed: the row
    /// only exists once it is.
    #[frb(sync)]
    pub fn set_retime_property(&self, value: BridgeScalar) -> Result<(), BridgeError> {
        let animation = value.animation()?;
        let mut retime = self.item()?.retime.clone().ok_or(BridgeError::NotRetimed)?;
        retime.animation = animation;
        self.commit(lumit_core::Op::SetRetimeProperty {
            comp: self.comp_id,
            layer: self.layer_id,
            retime: Some(retime),
        })
    }

    /// This layer's Volume, in dB (docs/09 §6): 0 is unity.
    #[frb(sync)]
    pub fn get_volume_db(&self) -> Result<BridgeScalar, BridgeError> {
        Ok(BridgeScalar::read(&self.item()?.volume_db))
    }

    /// Set the Volume, as one undoable step — the same coarse-grained shape as
    /// a transform property, and for the same invertibility reason.
    #[frb(sync)]
    pub fn set_volume_db(&self, value: BridgeScalar) -> Result<(), BridgeError> {
        let animation = value.animation()?;
        self.item()?;

        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store
            .commit(lumit_core::Op::SetLayerVolume {
                comp: self.comp_id,
                layer: self.layer_id,
                animation,
            })
            .map_err(BridgeError::OpError)?;
        Ok(())
    }

    /// Replace several transform properties at once, as one undoable step.
    ///
    /// For a control that acts on a whole row: Position's stopwatch has to key
    /// x and y together, and two ops would be two undo steps for one click.
    /// They are separate properties in the model — that is what makes a
    /// per-axis curve possible — so a batch is how one gesture stays one step.
    ///
    /// An empty list is a no-op rather than an empty commit, so a caller need
    /// not check before calling.
    #[frb(sync)]
    pub fn set_transforms(
        &self,
        props: Vec<BridgeTransformProp>,
        values: Vec<BridgeScalar>,
    ) -> Result<(), BridgeError> {
        // Two parallel lists rather than a list of pairs: frb has no tuple, and
        // a struct for two fields used in one place is more ceremony than the
        // length check it saves.
        if props.len() != values.len() {
            return Err(BridgeError::MismatchedTransforms);
        }
        if props.is_empty() {
            return Ok(());
        }
        self.item()?;

        let mut ops = Vec::with_capacity(props.len());
        for (prop, value) in props.into_iter().zip(values) {
            ops.push(lumit_core::Op::SetTransformProperty {
                comp: self.comp_id,
                layer: self.layer_id,
                prop: prop.core(),
                animation: value.animation()?,
            });
        }
        // One op stays one op; a batch of one would undo the same but reads
        // worse in the journal.
        let op = if ops.len() == 1 {
            ops.into_iter().next().ok_or(BridgeError::InvalidLayer)?
        } else {
            lumit_core::Op::Batch { ops }
        };
        self.commit(op)
    }

    /// Replace one transform property's whole animation, as one
    /// [`lumit_core::Op::SetTransformProperty`].
    ///
    /// One property per op, not the whole group: the op is exactly invertible
    /// that way, so a nudged Position is one undo step that puts back precisely
    /// what was there — where committing all eleven would make undo restore ten
    /// properties nobody touched.
    #[frb(sync)]
    pub fn set_transform(
        &self,
        prop: BridgeTransformProp,
        value: BridgeScalar,
    ) -> Result<(), BridgeError> {
        let animation = value.animation()?;
        // Confirm the layer is there before committing, so a stale reference is
        // a calm error rather than a failed op.
        self.item()?;

        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store
            .commit(lumit_core::Op::SetTransformProperty {
                comp: self.comp_id,
                layer: self.layer_id,
                prop: prop.core(),
                animation,
            })
            .map_err(BridgeError::OpError)?;
        Ok(())
    }

    #[frb(sync)]
    pub fn get_effects(&self) -> Result<Vec<BridgeEffectInstance>, BridgeError> {
        let layer = self.item()?;

        Ok(layer
            .effects
            .iter()
            .map(|f| BridgeEffectInstance::new(f.clone()))
            .collect())
    }

    /// Read this layer's effect stack, let `edit` change a clone of it, and
    /// commit the result as a single [`lumit_core::Op::SetLayerEffects`].
    ///
    /// The shared tail of every stack op below, exactly as v0's
    /// `edits::with_effects` is: one user action becomes one op and therefore one
    /// undo step (docs/17 "commands down"), and the two frontends cannot drift
    /// apart on what an effect edit means.
    ///
    /// Unlike v0 there is no drag overlay to discard here. The frb preview lives
    /// in the render request (`CompositionReference::render_frame_with_preview`)
    /// rather than in a field beside the document, so a failed edit cannot leave
    /// a stale staged value laid over later frames — the bug v0 had to clear the
    /// overlay at the top of `with_effects` to avoid.
    #[frb(ignore)]
    fn with_effects(
        &self,
        edit: impl FnOnce(&mut Vec<EffectInstance>) -> Result<(), BridgeError>,
    ) -> Result<(), BridgeError> {
        let mut effects = self.item()?.effects;
        edit(&mut effects)?;

        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store
            .commit(lumit_core::Op::SetLayerEffects {
                comp: self.comp_id,
                layer: self.layer_id,
                effects,
            })
            .map_err(BridgeError::OpError)?;
        Ok(())
    }

    /// Append the built-in effect named `name` to this layer's stack.
    ///
    /// Seeded at composition size, because a few effects' defaults are positions
    /// (a transform's anchor and position start at the centre of the frame), and
    /// a fresh effect should look like identity rather than dragging the picture
    /// to a corner. An unknown name is refused; nothing partial is committed.
    #[frb(sync)]
    pub fn add_effect(&self, name: String) -> Result<(), BridgeError> {
        let comp = self.composition()?;
        let instance = lumit_core::fx::instantiate_for_raster(
            &name,
            f64::from(comp.width),
            f64::from(comp.height),
        )
        .ok_or(BridgeError::UnknownEffectName)?;

        self.with_effects(move |effects| {
            effects.push(instance);
            Ok(())
        })
    }

    /// Remove `effect` from this layer's stack. An effect that is no longer there
    /// is an error rather than a silent success, so a double-click on Remove
    /// cannot look as though it deleted a second effect.
    #[frb(sync)]
    pub fn remove_effect(&self, effect: &BridgeEffectInstance) -> Result<(), BridgeError> {
        let id = effect.id();
        self.with_effects(move |effects| {
            let before = effects.len();
            effects.retain(|e| e.id != id);
            if effects.len() == before {
                return Err(BridgeError::InvalidEffect);
            }
            Ok(())
        })
    }

    /// Move `effect` to `new_index` in the stack — drag-to-reorder.
    ///
    /// The index clamps into range rather than failing: past the end lands the
    /// effect at the bottom, negative lands it at the top. A drag that overshoots
    /// the list is an ordinary thing for a pointer to do, and refusing it would
    /// leave the effect where it started with no explanation.
    #[frb(sync)]
    pub fn reorder_effect(
        &self,
        effect: &BridgeEffectInstance,
        new_index: i64,
    ) -> Result<(), BridgeError> {
        let id = effect.id();
        self.with_effects(move |effects| {
            let from = effects
                .iter()
                .position(|e| e.id == id)
                .ok_or(BridgeError::InvalidEffect)?;
            let instance = effects.remove(from);
            let to = usize::try_from(new_index).unwrap_or(0).min(effects.len());
            effects.insert(to, instance);
            Ok(())
        })
    }

    /// Enable or bypass `effect`. A bypassed effect renders as identity and is
    /// not animatable (docs/08 §1.5 — the effect's own Mix parameter is the
    /// animatable dial).
    #[frb(sync)]
    pub fn set_effect_enabled(
        &self,
        effect: &BridgeEffectInstance,
        enabled: bool,
    ) -> Result<(), BridgeError> {
        let id = effect.id();
        self.with_effects(move |effects| {
            let instance = effects
                .iter_mut()
                .find(|e| e.id == id)
                .ok_or(BridgeError::InvalidEffect)?;
            instance.enabled = enabled;
            Ok(())
        })
    }

    /// Commit a staged effect stack — the mouse-up for a gesture Dart has been
    /// editing through `BridgeEffectInstance::set_value` and previewing through
    /// `CompositionReference::render_frame_with_preview`. The whole drag becomes
    /// one undo step, which is the entire point of staging (docs/17 ABI v12).
    ///
    /// Only parameter *values* may cross this way: the staged stack must still
    /// name the same effects, in the same order, as the document does. Otherwise
    /// a stack read before some other action removed an effect from it would
    /// resurrect that effect on mouse-up, and reordering or deleting would have
    /// two paths — this one, which cannot say what it meant, and the dedicated
    /// ops above, which can.
    #[frb(sync)]
    pub fn set_effects(&self, effects: Vec<BridgeEffectInstance>) -> Result<(), BridgeError> {
        let staged: Vec<EffectInstance> = effects
            .iter()
            .map(BridgeEffectInstance::get_effects)
            .collect();

        self.with_effects(move |current| {
            let same_stack = current.len() == staged.len()
                && current.iter().zip(&staged).all(|(a, b)| a.id == b.id);
            if !same_stack {
                return Err(BridgeError::StaleEffectStack);
            }
            *current = staged;
            Ok(())
        })
    }
}
