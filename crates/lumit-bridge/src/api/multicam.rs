//! Multicam comps as the interface reads them (`lumit_core::multicam`): the
//! angle a clip plays, the multicam clip under the playhead and the angles
//! of its comp, the angles a new clip starts on, markers on footage, and the
//! angle viewer's picture.
//!
//! Runs on the UI thread, like every other read and edit. Making a multicam
//! comp from footage is [`make`], which works on a thread of its own.

use flutter_rust_bridge::frb;
use lumit_core::model::{Composition, Document, LayerKind, ProjectItem};
use lumit_core::multicam::{self, Multicam};
use lumit_core::sequence::Clip;
use lumit_core::time::{CompTime, Rational};
use uuid::Uuid;

use crate::api::{
    composition::CompositionReference, effect::BridgeRational, footage::FootageReference,
    BridgeError,
};

pub mod grid;
pub mod make;
pub(crate) mod wave;

/// The angle a multicam clip plays, as the Cut timeline names and colours it.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeClipAngle {
    pub id: Uuid,
    /// Its place among the angles the comp offers, counting from one, or
    /// nought for an angle switched off there.
    pub number: u32,
    pub name: String,
    /// The angle row's label colour, which a clip of it wears unless it has
    /// one of its own.
    pub label: u8,
    /// A sound angle: an audio-only row, with no picture.
    pub sound: bool,
}

/// One angle of a multicam comp.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeAngle {
    pub id: Uuid,
    pub number: u32,
    pub name: String,
    pub label: u8,
    pub sound: bool,
}

/// A multicam comp's angles, and the ones a new clip of it starts on.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeMulticam {
    pub comp: Uuid,
    pub name: String,
    pub angles: Vec<BridgeAngle>,
    pub picture: Option<Uuid>,
    pub sound: Option<Uuid>,
    /// The comp's frame size and rate, which its angle viewer is drawn at.
    pub width: u32,
    pub height: u32,
    pub rate_num: u32,
    pub rate_den: u32,
    /// How many frames it lasts.
    pub frames: i64,
}

/// The multicam clip under the playhead: which comp, which moment of it, and
/// the angles its picture shows and its sound plays.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeMulticamAt {
    pub multicam: BridgeMulticam,
    /// The frame of the multicam comp the clip shows, at the comp's rate.
    pub frame: i64,
    /// The row the clip is on, and the clip.
    pub layer: Uuid,
    pub clip: Uuid,
    pub picture: Option<Uuid>,
    pub sound: Option<Uuid>,
    /// Angles heard as well, from sound clips added under the clip.
    pub also: Vec<Uuid>,
    /// The clip is footage one angle holds rather than a multicam clip, and
    /// `picture` is that angle: picking another puts it above, in step.
    pub plain: bool,
    /// Each angle's own timecode at this moment, in the order of
    /// `multicam.angles`, when asked for. `None` for a sound angle and in a
    /// gap.
    pub times: Vec<Option<BridgeAngleTime>>,
}

/// The timecode one angle's footage reads at a moment: its frame, counted
/// from the footage's start timecode at its own rate.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeAngleTime {
    pub frame: u64,
    pub fps_num: u32,
    pub fps_den: u32,
    pub drop_frame: bool,
}

/// One marker on a piece of footage.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeFootageMarker {
    pub id: Uuid,
    /// Seconds of the footage's own time.
    pub time: BridgeRational,
    pub label: String,
}

#[frb(ignore)]
fn rational_of(t: Rational) -> BridgeRational {
    BridgeRational {
        num: t.num(),
        den: t.den(),
    }
}

/// The angle `clip` plays, where it names one of its comp's angles.
#[frb(ignore)]
pub(crate) fn clip_angle(doc: &Document, clip: &Clip) -> Option<BridgeClipAngle> {
    let layer = multicam::angle_layer(doc, clip)?;
    let lumit_core::sequence::ClipSource::Comp(id) = clip.source else {
        return None;
    };
    let comp = doc.comp(id)?;
    let number = multicam::number_of(comp, layer.id).unwrap_or(0);
    Some(BridgeClipAngle {
        id: layer.id,
        number: u32::try_from(number).unwrap_or(0),
        name: layer.name.clone(),
        label: layer.label,
        sound: layer.audio_only,
    })
}

/// The timecode `angle`'s footage reads at moment `t` of its multicam comp,
/// counted at the footage's own rate from its start timecode. `None` for a
/// sound angle, in a gap, and for footage with no rate of its own.
#[cfg(feature = "media")]
#[frb(ignore)]
fn angle_time(
    state: &crate::api::state::LumitBridgeState,
    doc: &Document,
    angle: &lumit_core::model::Layer,
    t: Rational,
) -> Option<BridgeAngleTime> {
    let LayerKind::Sequence { clips } = &angle.kind else {
        return None;
    };
    if angle.audio_only {
        return None;
    }
    let lt = t.checked_sub(angle.start_offset.0).ok()?;
    let clip = clips
        .iter()
        .find(|c| c.place_start <= lt && lt < c.place_end())?;
    let lumit_core::sequence::ClipSource::Footage(item) = clip.source else {
        return None;
    };
    let ProjectItem::Footage(f) = doc.item(item)? else {
        return None;
    };
    let at = clip.source_at(lt)?;
    let src = FootageReference::resolve_source(state, f)?;
    let info = crate::probe::ensure_probed(src.clone())?;
    let video = info.video.as_ref()?;
    let (num, den) = (
        u32::try_from(video.fps_num).ok()?,
        u32::try_from(video.fps_den).ok()?,
    );
    let rate = lumit_core::time::FrameRate::new(num, den).ok()?;
    let (start, drop_frame) =
        crate::api::interpret::start_timecode(f, &src, Some(rate)).unwrap_or((0, false));
    // A hair under a whole frame is that frame.
    let along = (at.to_f64() * f64::from(num) / f64::from(den.max(1)) + 1e-6).floor();
    Some(BridgeAngleTime {
        frame: start + along.max(0.0) as u64,
        fps_num: num,
        fps_den: den,
        drop_frame,
    })
}

/// A build that reads no media knows no footage's timecode.
#[cfg(not(feature = "media"))]
#[frb(ignore)]
fn angle_time(
    state: &crate::api::state::LumitBridgeState,
    doc: &Document,
    angle: &lumit_core::model::Layer,
    t: Rational,
) -> Option<BridgeAngleTime> {
    let _ = (state, doc, angle, t);
    None
}

/// A multicam comp as the angle viewer reads it.
#[frb(ignore)]
pub(crate) fn multicam_of(comp: &Composition) -> Option<BridgeMulticam> {
    if !comp.is_multicam() {
        return None;
    }
    let (picture, sound) = multicam::starting_angles(comp);
    let angles = multicam::offered(comp)
        .enumerate()
        .map(|(i, l)| BridgeAngle {
            id: l.id,
            number: u32::try_from(i + 1).unwrap_or(u32::MAX),
            name: l.name.clone(),
            label: l.label,
            sound: l.audio_only,
        })
        .collect();
    Some(BridgeMulticam {
        comp: comp.id,
        name: comp.name.clone(),
        angles,
        picture,
        sound,
        width: comp.width,
        height: comp.height,
        rate_num: comp.frame_rate.num(),
        rate_den: comp.frame_rate.den(),
        frames: comp.frame_rate.frame_at(CompTime(comp.duration.0)),
    })
}

impl CompositionReference {
    /// Whether this is a multicam comp: what the Project panel marks it by.
    #[frb(sync)]
    pub fn is_multicam(&self) -> Result<bool, BridgeError> {
        let doc = self.document()?;
        let comp = doc.comp(self.id).ok_or(BridgeError::InvalidComp)?;
        Ok(comp.is_multicam())
    }

    /// This comp's angles, when it is a multicam comp.
    #[frb(sync)]
    pub fn multicam(&self) -> Result<Option<BridgeMulticam>, BridgeError> {
        let doc = self.document()?;
        let comp = doc.comp(self.id).ok_or(BridgeError::InvalidComp)?;
        Ok(multicam_of(comp))
    }

    /// The multicam clip under `frame` of this comp: on the top picture row
    /// with one, else on the top sound row with one, else footage a multicam
    /// comp's angle holds. `None` where there is none, which is what the angle
    /// viewer stands still for. With `timecodes` each angle's timecode comes
    /// too.
    #[frb(sync)]
    pub fn multicam_at(
        &self,
        frame: i64,
        timecodes: bool,
    ) -> Result<Option<BridgeMulticamAt>, BridgeError> {
        let proj = self.project()?;
        let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let doc = state.store.snapshot();
        let comp = doc.comp(self.id).ok_or(BridgeError::InvalidComp)?;
        let t = comp
            .frame_rate
            .time_of_frame(frame.max(0))
            .map_err(|_| BridgeError::InvalidTime)?;
        let Some(under) = multicam::under(&doc, comp, t.0) else {
            return Ok(None);
        };
        let Some(mc) = doc.comp(under.comp) else {
            return Ok(None);
        };
        let Some(multicam) = multicam_of(mc) else {
            return Ok(None);
        };
        let times = if timecodes {
            multicam::offered(mc)
                .map(|angle| angle_time(&state, &doc, angle, under.time))
                .collect()
        } else {
            Vec::new()
        };
        Ok(Some(BridgeMulticamAt {
            multicam,
            frame: mc.frame_rate.frame_at(CompTime(under.time)),
            layer: under.layer,
            clip: under.clip,
            picture: under.picture,
            sound: under.sound,
            also: under.also,
            plain: under.plain,
            times,
        }))
    }

    /// The angles a new clip of this multicam comp starts on, one undo step.
    /// An unset picture is the first angle with one, and an unset sound
    /// follows the picture.
    #[frb(sync)]
    pub fn set_multicam_starts(
        &self,
        picture: Option<Uuid>,
        sound: Option<Uuid>,
    ) -> Result<(), BridgeError> {
        let comp = self.composition()?;
        let Some(mut mark) = comp.multicam() else {
            return Err(BridgeError::InvalidComp);
        };
        if (mark.picture, mark.sound) == (picture, sound) {
            return Ok(());
        }
        (mark.picture, mark.sound) = (picture, sound);
        self.commit(lumit_core::ops::Op::SetCompMulticam {
            comp: self.id,
            multicam: Some(Box::new(mark)),
        })
    }

    /// Make this comp a multicam comp, its Sequence layers its angles, or an
    /// ordinary comp again. One undo step.
    #[frb(sync)]
    pub fn set_multicam(&self, on: bool) -> Result<(), BridgeError> {
        let comp = self.composition()?;
        if comp.is_multicam() == on {
            return Ok(());
        }
        self.commit(lumit_core::ops::Op::SetCompMulticam {
            comp: self.id,
            multicam: on.then(|| Box::new(Multicam::default())),
        })
    }

    /// Ask for the angle viewer's picture: page `page` of this multicam
    /// comp's angles at its `frame`, drawn `width` by `height` at `scale` for
    /// `view`. It arrives down the same transport as every other picture.
    #[allow(clippy::too_many_arguments)]
    #[frb(sync)]
    pub fn render_angles(
        &self,
        frame: u64,
        page: u32,
        scale: f32,
        width: u32,
        height: u32,
        view: u32,
    ) -> Result<(), BridgeError> {
        let project = crate::api::project::ProjectReference::new(self.project);
        let (rate, frames) = {
            let doc = self.document()?;
            let comp = doc.comp(self.id).ok_or(BridgeError::InvalidComp)?;
            let frames = comp.frame_rate.frame_at(CompTime(comp.duration.0));
            (
                (comp.frame_rate.num(), comp.frame_rate.den()),
                u64::try_from(frames.max(1)).unwrap_or(1),
            )
        };
        let state = project.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        let Some(sender) = &state.sender else {
            return Err(BridgeError::InvalidWorkerState);
        };
        sender
            .send(crate::api::worker_thread::WorkerRequest::RenderItem(
                crate::api::worker_thread::RenderItemRequest {
                    project: project.clone(),
                    of: crate::scratch::ScratchOf::Angles {
                        comp: self.id,
                        page,
                    },
                    effects: true,
                    frame,
                    scale,
                    size: (width, height),
                    rate,
                    frames,
                    view,
                },
            ))
            .map_err(|_| BridgeError::InvalidWorkerState)
    }
}

impl FootageReference {
    /// This footage's markers, earliest first.
    #[frb(sync)]
    pub fn markers(&self) -> Result<Vec<BridgeFootageMarker>, BridgeError> {
        let doc = self.document()?;
        let mut markers: Vec<BridgeFootageMarker> = doc
            .footage_markers
            .get(&self.id)
            .into_iter()
            .flatten()
            .map(|m| BridgeFootageMarker {
                id: m.id,
                time: rational_of(m.time.0),
                label: m.label.clone(),
            })
            .collect();
        markers.sort_by(|a, b| {
            let t = |m: &BridgeFootageMarker| Rational::new(m.time.num, m.time.den).ok();
            t(a).cmp(&t(b))
        });
        Ok(markers)
    }

    /// Put a marker on this footage at `at`, seconds of its own time, one
    /// undo step. Nothing where one is already there.
    #[frb(sync)]
    pub fn add_marker(&self, at: BridgeRational) -> Result<(), BridgeError> {
        let at = Rational::new(at.num, at.den).map_err(|_| BridgeError::InvalidTime)?;
        if at.is_negative() {
            return Err(BridgeError::InvalidTime);
        }
        let mut markers = self.held_markers()?;
        if markers.iter().any(|m| m.time.0 == at) {
            return Ok(());
        }
        markers.push(lumit_core::markers::Marker::user(Uuid::now_v7(), at));
        self.write_markers(markers)
    }

    /// Take one marker off this footage, one undo step.
    #[frb(sync)]
    pub fn remove_marker(&self, id: Uuid) -> Result<(), BridgeError> {
        let mut markers = self.held_markers()?;
        let before = markers.len();
        markers.retain(|m| m.id != id);
        if markers.len() == before {
            return Ok(());
        }
        self.write_markers(markers)
    }

    /// Take every marker off this footage, one undo step.
    #[frb(sync)]
    pub fn clear_markers(&self) -> Result<(), BridgeError> {
        if self.held_markers()?.is_empty() {
            return Ok(());
        }
        self.write_markers(Vec::new())
    }

    #[frb(ignore)]
    fn document(&self) -> Result<std::sync::Arc<Document>, BridgeError> {
        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(proj.store.snapshot())
    }

    #[frb(ignore)]
    fn held_markers(&self) -> Result<Vec<lumit_core::markers::Marker>, BridgeError> {
        let doc = self.document()?;
        if !matches!(doc.item(self.id), Some(ProjectItem::Footage(_))) {
            return Err(BridgeError::InvalidItem);
        }
        Ok(doc
            .footage_markers
            .get(&self.id)
            .cloned()
            .unwrap_or_default())
    }

    #[frb(ignore)]
    fn write_markers(&self, markers: Vec<lumit_core::markers::Marker>) -> Result<(), BridgeError> {
        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store
            .commit(lumit_core::ops::Op::SetFootageMarkers {
                id: self.id,
                markers,
            })
            .map(|_| ())
            .map_err(BridgeError::OpError)
    }
}

/// Whether a layer is an angle with a picture: what the angle viewer draws a
/// cell of.
#[frb(ignore)]
pub(crate) fn draws(layer: &lumit_core::model::Layer) -> bool {
    matches!(layer.kind, LayerKind::Sequence { .. }) && !layer.audio_only
}
