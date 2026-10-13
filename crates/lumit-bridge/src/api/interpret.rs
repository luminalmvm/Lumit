//! Footage interpretation across the seam (docs/01-GLOSSARY.md: Interpretation).
//!
//! **In plain terms.** The Interpret footage window asks one question on the
//! way in, what the item is read as now and what its file says of itself, and
//! makes one edit on the way out. Everything between is the window's own
//! draft: nothing is committed until it is accepted, and then it is one undo
//! step however many items it was opened on.
//!
//! The engine does the arithmetic. What a draft would make of the file (its
//! rate, length and size) is asked here, never worked out in Dart, so the
//! window can't show a number the Timeline then disagrees with.

use flutter_rust_bridge::frb;
use lumit_core::interpretation::{
    AlphaMode, FieldOrder, Interpretation, LinearLight, Pulldown, StartTimecode,
};
use lumit_core::model::{FootageItem, ProjectItem};
use lumit_core::time::FrameRate;
use lumit_core::Op;

use crate::api::effect::BridgeRational;
use crate::api::footage::FootageReference;
use crate::api::BridgeError;

/// What a file's alpha channel means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeAlphaMode {
    Straight,
    Ignore,
    Premultiplied,
}

/// Whether a frame is two fields, and which was shot first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeFieldOrder {
    Off,
    Upper,
    Lower,
}

/// A film cadence to take back out of interlaced video.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgePulldown {
    Off,
    /// 3:2 pulldown.
    ThreeTwo,
    /// 24Pa, the advanced cadence.
    Advanced,
}

/// Whether a file's numbers are already linear light.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeLinearLight {
    /// A float file is, anything else carries its colour space's curve.
    Auto,
    On,
    Off,
}

/// How a footage item's file is read: everything the window edits.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeInterpretation {
    pub alpha: BridgeAlphaMode,
    /// The colour a premultiplied file was saved over, sRGB bytes.
    pub matte_r: u8,
    pub matte_g: u8,
    pub matte_b: u8,
    pub invert_alpha: bool,
    /// The rate the frames play at, as the exact pair. 0/1 is the file's own.
    /// A run of stills always has one, since it has no other.
    pub fps_num: u32,
    pub fps_den: u32,
    /// The first frame's timecode, `HH:MM:SS:FF` with a `;` before the frames
    /// when drop-frame. `None` is the file's own.
    pub timecode: Option<String>,
    pub fields: BridgeFieldOrder,
    pub preserve_edges: bool,
    pub pulldown: BridgePulldown,
    /// Where the file's first frame sits in the cadence, 0 to 4.
    pub pulldown_phase: u8,
    /// How much wider than tall one pixel is. 1/1 is square.
    pub pixel_aspect_num: u32,
    pub pixel_aspect_den: u32,
    /// How many times the file plays through, at least once.
    pub loops: u32,
    /// The colour space the file arrives in, by the config's own name.
    pub colour_space: Option<String>,
    pub linear_light: BridgeLinearLight,
    pub preserve_rgb: bool,
}

/// What a file says of itself, before any interpretation: the window's "the
/// file says" lines, and what its choices are offered against.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeFootageFacts {
    /// Whether there is a picture at all. A sound file has nothing to read.
    pub has_picture: bool,
    /// One picture that doesn't run, so no rate, fields or loop to set.
    pub is_still: bool,
    /// A numbered run of stills, whose rate is the item's to give.
    pub is_sequence: bool,
    pub has_alpha: bool,
    /// Float samples, which read as linear light unless told otherwise.
    pub float: bool,
    pub fps_num: u32,
    pub fps_den: u32,
    pub frames: u64,
    pub duration: BridgeRational,
    pub width: u32,
    pub height: u32,
    /// The field order the file declares.
    pub fields: BridgeFieldOrder,
    /// The pixel shape the file declares, 1/1 when it says square or nothing.
    pub pixel_aspect_num: u32,
    pub pixel_aspect_den: u32,
    /// The file's own start timecode, as it writes it.
    pub timecode: Option<String>,
}

/// One item as the window opens on it.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeFootageReading {
    pub now: BridgeInterpretation,
    pub file: BridgeFootageFacts,
}

/// What an interpretation makes of a file: the numbers the rest of Lumit will
/// then see for it.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeInterpreted {
    pub fps_num: u32,
    pub fps_den: u32,
    pub frames: u64,
    pub duration: BridgeRational,
    pub width: u32,
    pub height: u32,
}

/// A guess at what a file's alpha means.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeAlphaGuess {
    pub alpha: BridgeAlphaMode,
    pub matte_r: u8,
    pub matte_g: u8,
    pub matte_b: u8,
}

/// A guess at the pulldown cadence in a file.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgePulldownGuess {
    pub pulldown: BridgePulldown,
    pub phase: u8,
}

fn fields_out(fields: FieldOrder) -> BridgeFieldOrder {
    match fields {
        FieldOrder::Off => BridgeFieldOrder::Off,
        FieldOrder::Upper => BridgeFieldOrder::Upper,
        FieldOrder::Lower => BridgeFieldOrder::Lower,
    }
}

/// The rate a timecode is counted at: the frames' own, never a field rate.
fn timecode_rate(f: &FootageItem, native: Option<FrameRate>) -> Option<FrameRate> {
    f.sequence
        .as_ref()
        .map(|s| s.frame_rate)
        .or(f.interpretation.frame_rate)
        .or(native)
}

fn reading_of(f: &FootageItem, native: Option<FrameRate>) -> BridgeInterpretation {
    let it = &f.interpretation;
    let (alpha, matte) = match it.alpha {
        AlphaMode::Straight => (BridgeAlphaMode::Straight, [0; 3]),
        AlphaMode::Ignore => (BridgeAlphaMode::Ignore, [0; 3]),
        AlphaMode::Premultiplied { matte } => (BridgeAlphaMode::Premultiplied, matte),
    };
    let rate = f.sequence.as_ref().map(|s| s.frame_rate).or(it.frame_rate);
    let (pulldown, pulldown_phase) = match it.pulldown {
        Pulldown::Off => (BridgePulldown::Off, 0),
        Pulldown::ThreeTwo { phase } => (BridgePulldown::ThreeTwo, phase % 5),
        Pulldown::Advanced { phase } => (BridgePulldown::Advanced, phase % 5),
    };
    let (pixel_aspect_num, pixel_aspect_den) = it.pixel_aspect.unwrap_or((1, 1));
    BridgeInterpretation {
        alpha,
        matte_r: matte[0],
        matte_g: matte[1],
        matte_b: matte[2],
        invert_alpha: it.invert_alpha,
        fps_num: rate.map_or(0, FrameRate::num),
        fps_den: rate.map_or(1, FrameRate::den),
        timecode: it
            .start_timecode
            .zip(timecode_rate(f, native))
            .map(|(start, rate)| start.text(rate)),
        fields: fields_out(it.fields),
        preserve_edges: it.preserve_edges,
        pulldown,
        pulldown_phase,
        pixel_aspect_num,
        pixel_aspect_den,
        loops: it.plays(),
        colour_space: f.colour_space.clone(),
        linear_light: match it.linear_light {
            LinearLight::Auto => BridgeLinearLight::Auto,
            LinearLight::On => BridgeLinearLight::On,
            LinearLight::Off => BridgeLinearLight::Off,
        },
        preserve_rgb: it.preserve_rgb,
    }
}

/// The document's form of what the window sent, keeping whatever a newer
/// Lumit wrote on the item that this one doesn't know.
///
/// `Err` only for a rate that isn't one. The other numbers are held to what
/// the engine reads (`lumit_render::source::interpretation`), so a stray one
/// can't be stored and then quietly ignored.
fn to_engine(
    to: &BridgeInterpretation,
    f: &FootageItem,
    native: Option<FrameRate>,
) -> Result<Interpretation, BridgeError> {
    let rate = match (to.fps_num, f.sequence.is_some()) {
        // A run's rate lives on the run, and nought is "the file's own".
        (_, true) | (0, _) => None,
        _ => {
            let fps = f64::from(to.fps_num) / f64::from(to.fps_den.max(1));
            if to.fps_den == 0 || !(0.01..=999.0).contains(&fps) {
                return Err(BridgeError::InvalidFrameRate);
            }
            let rate = FrameRate::new(to.fps_num, to.fps_den)
                .map_err(|_| BridgeError::InvalidFrameRate)?;
            // Conforming to the rate the file already has is not conforming.
            (Some(rate) != native).then_some(rate)
        }
    };
    let counted_at = f
        .sequence
        .as_ref()
        .map(|s| s.frame_rate)
        .or(rate)
        .or(native);
    let aspect = (to.pixel_aspect_num, to.pixel_aspect_den);
    let aspect_ok = aspect.1 > 0
        && aspect.0 != aspect.1
        && (0.1..=10.0).contains(&(f64::from(aspect.0) / f64::from(aspect.1)));
    let fields = match to.fields {
        BridgeFieldOrder::Off => FieldOrder::Off,
        BridgeFieldOrder::Upper => FieldOrder::Upper,
        BridgeFieldOrder::Lower => FieldOrder::Lower,
    };
    let phase = to.pulldown_phase % 5;
    Ok(Interpretation {
        alpha: match to.alpha {
            BridgeAlphaMode::Straight => AlphaMode::Straight,
            BridgeAlphaMode::Ignore => AlphaMode::Ignore,
            BridgeAlphaMode::Premultiplied => AlphaMode::Premultiplied {
                matte: [to.matte_r, to.matte_g, to.matte_b],
            },
        },
        invert_alpha: to.invert_alpha,
        frame_rate: rate,
        // Text that isn't a timecode leaves the one already there alone.
        start_timecode: match (&to.timecode, counted_at) {
            (None, _) => None,
            (Some(text), Some(rate)) => {
                StartTimecode::parse(text, rate).or(f.interpretation.start_timecode)
            }
            (Some(_), None) => f.interpretation.start_timecode,
        },
        fields,
        preserve_edges: to.preserve_edges && fields != FieldOrder::Off,
        // A cadence is made of fields, so there is none without them.
        pulldown: match (to.pulldown, fields) {
            (_, FieldOrder::Off) | (BridgePulldown::Off, _) => Pulldown::Off,
            (BridgePulldown::ThreeTwo, _) => Pulldown::ThreeTwo { phase },
            (BridgePulldown::Advanced, _) => Pulldown::Advanced { phase },
        },
        pixel_aspect: aspect_ok.then_some(aspect),
        loops: match to.loops.min(9999) {
            0 | 1 => 0,
            n => n,
        },
        linear_light: match to.linear_light {
            BridgeLinearLight::Auto => LinearLight::Auto,
            BridgeLinearLight::On => LinearLight::On,
            BridgeLinearLight::Off => LinearLight::Off,
        },
        preserve_rgb: to.preserve_rgb,
        extra: f.interpretation.extra.clone(),
    })
}

/// The file's own rate, from the probe of it read plainly.
#[cfg(feature = "media")]
fn native_rate(src: &lumit_media::MediaSource) -> Option<FrameRate> {
    let probe = crate::probe::ensure_probed(src.raw())?;
    let video = probe.video.as_ref()?;
    FrameRate::new(
        u32::try_from(video.fps_num).ok()?,
        u32::try_from(video.fps_den).ok()?,
    )
    .ok()
}

/// The timecode of this item's first frame: the one it was given, else the
/// file's own. Counted in the item's frames as they are interpreted, so a
/// clip with its fields separated starts at twice the number.
#[cfg(feature = "media")]
///
/// `shown` is the rate it will be written at. Drop-frame is only said of a
/// rate that drops, so a clip stamped at 29.97 and conformed to 25 counts on
/// plainly.
pub(crate) fn start_timecode(
    f: &FootageItem,
    src: &lumit_media::MediaSource,
    shown: Option<FrameRate>,
) -> Option<(u64, bool)> {
    let mut start = f.interpretation.start_timecode.or_else(|| {
        let raw = crate::probe::ensure_probed(src.raw())?;
        StartTimecode::parse(raw.timecode.as_deref()?, native_rate(src)?)
    })?;
    start.drop_frame &= shown.is_some_and(StartTimecode::drops_at);
    let frame = usize::try_from(start.frame).unwrap_or(usize::MAX);
    Some((
        src.interpret.frames_per_pass(frame) as u64,
        start.drop_frame,
    ))
}

impl FootageReference {
    /// How this item is read now, and what its file says of itself.
    ///
    /// Not sync: the file's own facts come off the container, which is not
    /// work for the UI isolate.
    pub fn interpretation(&self) -> Result<BridgeFootageReading, BridgeError> {
        let state = self.project()?;
        let (item, src) = {
            let proj = state.read().map_err(|_| BridgeError::ReadFailed)?;
            let doc = proj.store.snapshot();
            let Some(ProjectItem::Footage(f)) = doc.item(self.id) else {
                return Err(BridgeError::InvalidItem);
            };
            #[cfg(feature = "media")]
            let src = Self::resolve_source(&proj, f);
            #[cfg(not(feature = "media"))]
            let src: Option<()> = None;
            (f.clone(), src)
        };
        // With no decoder there is nothing to read off the file, and these
        // stand as they are.
        #[cfg_attr(not(feature = "media"), allow(unused_mut))]
        let mut file = BridgeFootageFacts {
            has_picture: false,
            is_still: false,
            is_sequence: item.sequence.is_some(),
            has_alpha: false,
            float: false,
            fps_num: 0,
            fps_den: 1,
            frames: 0,
            duration: BridgeRational { num: 0, den: 1 },
            width: 0,
            height: 0,
            fields: BridgeFieldOrder::Off,
            pixel_aspect_num: 1,
            pixel_aspect_den: 1,
            timecode: None,
        };
        #[cfg(feature = "media")]
        let native = src.as_ref().and_then(|src| {
            let raw = src.raw();
            let probe = crate::probe::ensure_probed(raw.clone())?;
            let video = probe.video.as_ref()?;
            let facts = lumit_media::probe::facts(&raw).unwrap_or_default();
            let seconds =
                lumit_core::time::Rational::from_f64_on_grid(probe.duration_seconds, 1000)
                    .unwrap_or(lumit_core::time::Rational::ZERO);
            file.has_picture = true;
            file.is_still = !probe.runs_as_video();
            file.has_alpha = facts.has_alpha;
            file.float = facts.float;
            file.fps_num = u32::try_from(video.fps_num).unwrap_or(0);
            file.fps_den = u32::try_from(video.fps_den).unwrap_or(1).max(1);
            file.frames = lumit_render::media_index::load_or_build_index(&raw)
                .map_or(0, |index| index.frame_count() as u64);
            file.duration = BridgeRational {
                num: seconds.num(),
                den: seconds.den(),
            };
            file.width = video.width;
            file.height = video.height;
            file.fields = match facts.fields {
                lumit_media::Fields::Off => BridgeFieldOrder::Off,
                lumit_media::Fields::Upper => BridgeFieldOrder::Upper,
                lumit_media::Fields::Lower => BridgeFieldOrder::Lower,
            };
            (file.pixel_aspect_num, file.pixel_aspect_den) = facts.pixel_aspect;
            file.timecode = probe.timecode.clone();
            FrameRate::new(file.fps_num, file.fps_den).ok()
        });
        #[cfg(not(feature = "media"))]
        let native = {
            let _ = src;
            None
        };
        Ok(BridgeFootageReading {
            now: reading_of(&item, native),
            file,
        })
    }

    /// What this file's alpha looks like it means, from its own frames, or
    /// `None` when they have no transparency to tell by.
    pub fn guess_alpha(&self) -> Result<Option<BridgeAlphaGuess>, BridgeError> {
        #[cfg(feature = "media")]
        {
            let Some(src) = self.source()? else {
                return Ok(None);
            };
            Ok(lumit_render::source::guess_alpha(&src).map(|alpha| {
                let (alpha, matte) = match alpha {
                    lumit_media::Alpha::Straight => (BridgeAlphaMode::Straight, [0; 3]),
                    lumit_media::Alpha::Ignore => (BridgeAlphaMode::Ignore, [0; 3]),
                    lumit_media::Alpha::Premultiplied(m) => (BridgeAlphaMode::Premultiplied, m),
                };
                BridgeAlphaGuess {
                    alpha,
                    matte_r: matte[0],
                    matte_g: matte[1],
                    matte_b: matte[2],
                }
            }))
        }
        #[cfg(not(feature = "media"))]
        Ok(None)
    }

    /// The pulldown cadence this file's own frames show, or `None` when none
    /// stands out.
    pub fn guess_pulldown(&self) -> Result<Option<BridgePulldownGuess>, BridgeError> {
        #[cfg(feature = "media")]
        {
            let Some(src) = self.source()? else {
                return Ok(None);
            };
            Ok(match lumit_render::source::guess_pulldown(&src) {
                lumit_media::Pulldown::Off => None,
                lumit_media::Pulldown::ThreeTwo(phase) => Some(BridgePulldownGuess {
                    pulldown: BridgePulldown::ThreeTwo,
                    phase,
                }),
                lumit_media::Pulldown::Advanced(phase) => Some(BridgePulldownGuess {
                    pulldown: BridgePulldown::Advanced,
                    phase,
                }),
            })
        }
        #[cfg(not(feature = "media"))]
        Ok(None)
    }

    /// Where this item's file is and how it is read, with the lock let go.
    #[cfg(feature = "media")]
    #[frb(ignore)]
    fn source(&self) -> Result<Option<lumit_media::MediaSource>, BridgeError> {
        let state = self.project()?;
        let proj = state.read().map_err(|_| BridgeError::ReadFailed)?;
        match proj.store.snapshot().item(self.id) {
            Some(ProjectItem::Footage(f)) => Ok(Self::resolve_source(&proj, f)),
            _ => Err(BridgeError::InvalidItem),
        }
    }
}

/// What `to` would make of a file that says `file`: the rate, length and size
/// the rest of Lumit would see. Pure arithmetic, the same the decoder does.
#[frb(sync)]
pub fn interpreted(file: BridgeFootageFacts, to: BridgeInterpretation) -> BridgeInterpreted {
    // An item of its own, so the sums go through the one road the engine's do.
    let mut item = FootageItem {
        id: uuid::Uuid::nil(),
        name: String::new(),
        media: lumit_core::model::MediaRef {
            relative_path: String::new(),
            absolute_path: String::new(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        colour_space: None,
        sequence: None,
        source_layer: None,
        interpretation: Interpretation::default(),
        extra: serde_json::Map::new(),
    };
    let native = FrameRate::new(file.fps_num, file.fps_den).ok();
    // A run of stills plays at the rate it is given and has no other.
    let run = file
        .is_sequence
        .then(|| FrameRate::new(to.fps_num, to.fps_den).ok());
    if let Ok(reading) = to_engine(&to, &item, native) {
        item.interpretation = reading;
    }
    let it = lumit_render::source::interpretation(&item);
    let own = run.flatten().or(native);
    let rate = own.map_or((0, 1), |r| {
        (
            i32::try_from(r.num()).unwrap_or(0),
            i32::try_from(r.den()).unwrap_or(1),
        )
    });
    let (fps_num, fps_den) = it.fps(rate);
    let raw_frames = usize::try_from(file.frames).unwrap_or(usize::MAX);
    let frames = it.frame_count(raw_frames) as u64;
    // Length is frames over rate, exactly, when there is a rate to divide by.
    let duration = match (u32::try_from(fps_num), u32::try_from(fps_den)) {
        (Ok(n), Ok(d)) if n > 0 && d > 0 && !file.is_still => BridgeRational {
            num: i64::try_from(frames.saturating_mul(u64::from(d))).unwrap_or(i64::MAX),
            den: i64::from(n),
        },
        _ => file.duration,
    };
    BridgeInterpreted {
        fps_num: u32::try_from(fps_num).unwrap_or(0),
        fps_den: u32::try_from(fps_den).unwrap_or(1).max(1),
        frames,
        duration,
        width: it.width(file.width),
        height: file.height,
    }
}

/// Read every one of `items` the way `to` says. One gesture, one undo step,
/// however many items: what the window does on OK, and what applying a
/// remembered interpretation to a selection does.
///
/// Every item takes every setting, so they end up read alike. A still keeps
/// the settings and the engine reads none of the ones about time from it,
/// since one picture has no rate, fields or loop. A run of stills takes the
/// rate as its own.
#[frb(sync)]
pub fn set_interpretation(
    items: Vec<FootageReference>,
    to: BridgeInterpretation,
) -> Result<(), BridgeError> {
    let Some(first) = items.first() else {
        return Ok(());
    };
    let state = first.project()?;

    // The document and where each item's file is, under the read lock and no
    // longer: nothing below may open a file with the project held.
    let (doc, sources) = {
        let proj = state.read().map_err(|_| BridgeError::ReadFailed)?;
        let doc = proj.store.snapshot();
        let mut sources = Vec::with_capacity(items.len());
        for item in &items {
            if item.project_id() != first.project_id() {
                return Err(BridgeError::InvalidItem);
            }
            let Some(ProjectItem::Footage(f)) = doc.item(item.id()) else {
                return Err(BridgeError::InvalidItem);
            };
            #[cfg(feature = "media")]
            sources.push(FootageReference::resolve_source(&proj, f));
            #[cfg(not(feature = "media"))]
            sources.push(None::<()>);
            let _ = f;
        }
        (doc, sources)
    };

    // A file's own rate is only asked after when something is measured
    // against it: a rate to conform to, or a timecode to count frames by.
    let measured = to.fps_num != 0 || to.timecode.is_some();
    let mut ops = Vec::new();
    #[cfg(feature = "media")]
    let mut reread = Vec::new();
    #[cfg(not(feature = "media"))]
    let reread: Vec<()> = Vec::new();
    for (item, src) in items.iter().zip(sources) {
        let Some(ProjectItem::Footage(f)) = doc.item(item.id()) else {
            continue;
        };
        #[cfg(feature = "media")]
        let native = src
            .as_ref()
            .filter(|_| measured && f.sequence.is_none())
            .and_then(native_rate);
        #[cfg(not(feature = "media"))]
        let native = {
            let _ = (&src, measured);
            None
        };

        let mut next = f.clone();
        next.interpretation = to_engine(&to, f, native)?;
        next.colour_space = to.colour_space.clone().filter(|s| !s.is_empty());
        // A run of stills takes the rate as its own, held to the same range
        // a conformed rate is. Nought leaves it as it is.
        if let (Some(run), true) = (next.sequence.as_mut(), to.fps_num != 0) {
            let fps = f64::from(to.fps_num) / f64::from(to.fps_den.max(1));
            if to.fps_den == 0 || !(0.01..=999.0).contains(&fps) {
                return Err(BridgeError::InvalidFrameRate);
            }
            run.frame_rate = FrameRate::new(to.fps_num, to.fps_den)
                .map_err(|_| BridgeError::InvalidFrameRate)?;
        }
        if next.sequence != f.sequence {
            if let Some(run) = &next.sequence {
                ops.push(Op::SetSequenceRate {
                    id: f.id,
                    frame_rate: run.frame_rate,
                });
            }
        }
        if next.colour_space != f.colour_space {
            ops.push(Op::SetFootageColourSpace {
                id: f.id,
                space: next.colour_space.clone(),
            });
        }
        if next.interpretation != f.interpretation {
            ops.push(Op::SetFootageInterpretation {
                id: f.id,
                interpretation: Box::new(next.interpretation.clone()),
            });
        }
        // Each item as it will be read, so the probe worker can be at the
        // files while the window closes.
        #[cfg(feature = "media")]
        if let Some(src) = src {
            reread.push(lumit_media::MediaSource {
                sequence_fps: next.sequence_fps(),
                interpret: lumit_render::source::interpretation(&next),
                ..src
            });
        }
    }
    if ops.is_empty() {
        return Ok(());
    }
    {
        let proj = state.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store
            .commit(Op::Batch { ops })
            .map_err(BridgeError::OpError)?;
    }
    // After the commit and outside the lock, as a relink's sweep does.
    for src in reread {
        crate::probe::request(src);
    }
    Ok(())
}
