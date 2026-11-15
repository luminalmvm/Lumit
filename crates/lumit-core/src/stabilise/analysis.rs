//! What one analysis of a clip found, and the table the render reads it from.
//!
//! In plain terms: the slow half of stabilising is following hundreds of specks
//! through every frame of the clip. What comes out of it is two things. How the
//! whole picture moved from each frame to the next, fitted two ways (a slide
//! with a turn and a growth, and a full perspective warp), and the specks
//! themselves, which the Viewer draws and which a user can drop.
//! Everything the effect does with them afterwards is quick and is redone on
//! every change of its rows.
//!
//! Thread: built on the analysis thread, published here, read by render workers
//! and the bridge. The table only ever hands out `Arc`s, so a lock is held to
//! clone one and never across any work.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// How the picture moved between two neighbouring analysed frames, in source
/// pixels: points of the earlier frame are taken to where they are in the later
/// one. Measured from the specks of the one layer of the picture the analysis
/// follows through the shot, the background where it can tell.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Step {
    /// A similarity `(a, b, tx, ty)`: `x' = a·x − b·y + tx`, `y' = b·x + a·y + ty`.
    pub similarity: [f64; 4],
    /// A homography, row-major, mapping column vectors `(x, y, 1)`.
    pub homography: [f64; 9],
    /// Specks that agreed with the homography.
    pub kept: u32,
    /// Specks that agreed with the similarity.
    pub kept_similarity: u32,
    /// Specks seen on both frames.
    pub seen: u32,
    /// The two frames are of different shots: the path starts again after it.
    pub cut: bool,
}

impl Step {
    /// No motion at all, for a pair nothing could be fitted to.
    pub const STILL: Step = Step {
        similarity: [1.0, 0.0, 0.0, 0.0],
        homography: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        kept: 0,
        kept_similarity: 0,
        seen: 0,
        cut: false,
    };

    /// Whether enough specks agreed with the homography for it to be used.
    #[must_use]
    pub fn trusted(&self) -> bool {
        self.kept >= MIN_KEPT && !self.cut
    }

    /// Whether enough specks agreed with the similarity for it to be used.
    #[must_use]
    pub fn trusted_similarity(&self) -> bool {
        self.kept_similarity >= MIN_KEPT && !self.cut
    }
}

/// Below this many agreeing specks a step's fit is not used, and the path
/// carries the motion of the steps either side over it instead.
pub const MIN_KEPT: u32 = 8;

/// One followed speck, compact: where it was on each analysed frame from its
/// first, in source pixels, and whether it moved with the picture on each step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    /// The tracker's id, stable for one analysis.
    pub id: u32,
    /// The analysed frame it was first seen on, counted from the analysis's
    /// first frame.
    pub first: u32,
    /// One position per frame from `first`, contiguous.
    pub points: Vec<[f32; 2]>,
    /// One bit per step from `first`: set when the speck belonged to the layer
    /// the step was measured from. Eight steps a byte, lowest bit first.
    pub agreed: Vec<u8>,
}

impl Run {
    /// Where the speck was on analysed frame `frame`, if it was seen there.
    #[must_use]
    pub fn at(&self, frame: u32) -> Option<[f32; 2]> {
        let i = frame.checked_sub(self.first)?;
        self.points.get(i as usize).copied()
    }

    /// Whether the speck agreed with the step out of analysed frame `frame`.
    #[must_use]
    pub fn agreed_at(&self, frame: u32) -> bool {
        let Some(i) = frame.checked_sub(self.first) else {
            return false;
        };
        let (byte, bit) = ((i / 8) as usize, i % 8);
        self.agreed.get(byte).is_some_and(|b| b & (1 << bit) != 0)
    }

    /// The last analysed frame it was seen on.
    #[must_use]
    pub fn last(&self) -> u32 {
        self.first + (self.points.len() as u32).saturating_sub(1)
    }
}

/// Pack per-step agreement flags into the bytes [`Run::agreed`] keeps.
#[must_use]
pub fn pack_agreed(flags: &[bool]) -> Vec<u8> {
    let mut out = vec![0u8; flags.len().div_ceil(8)];
    for (i, &f) in flags.iter().enumerate() {
        if f {
            if let Some(b) = out.get_mut(i / 8) {
                *b |= 1 << (i % 8);
            }
        }
    }
    out
}

/// A whole analysis of one clip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Analysis {
    /// The rate the frames were read at: the file's own, with no Interpret
    /// footage rate on it. The frames are numbered by source frame, so a rate
    /// set later re-times them and changes nothing found
    /// ([`super::shown::rate`]).
    pub fps: f64,
    /// Analysed frames per frame of the item's rate: two for a clip read as
    /// fields, four fifths for one with its pulldown taken out, one otherwise.
    pub rate_scale: f64,
    /// The raster the positions are in: the footage's own pixels, or a Precomp
    /// layer's.
    pub width: u32,
    pub height: u32,
    /// Those pixels to one of the raster the specks were followed on: more
    /// than one when the frames were read smaller.
    pub scale: f64,
    /// The source frame the analysis starts on.
    pub first: i64,
    /// How many frames were analysed. Never fewer than one.
    pub frames: u32,
    /// How many frames were asked for: the clip, or the Frames row's span.
    /// Fewer analysed than asked for is a run that stopped early.
    pub asked: u32,
    /// `frames - 1` steps, the motion out of each analysed frame but the last.
    pub steps: Vec<Step>,
    /// Every followed speck, in id order.
    pub runs: Vec<Run>,
    /// The share of a frame interval the sensor took to read a frame, along x
    /// and along y: negative for a read the other way. At most one is non-zero,
    /// and both are nought when the clip showed no rolling shutter.
    pub readout: [f32; 2],
}

impl Analysis {
    /// How far, in the analysis's pixels, a speck may miss a fitted motion and
    /// still count as moving with it: a fixed distance on the raster the
    /// specks were followed on, since that is where their noise is.
    #[must_use]
    pub fn agree_px(&self) -> f64 {
        super::motion::agree_px(self.scale)
    }

    /// Whether the analysis holds together: at least one frame, one step
    /// fewer than frames, every speck inside the span and every number the
    /// plan reads finite. A file that parses but fails this is read as no
    /// file at all.
    #[must_use]
    pub fn is_sound(&self) -> bool {
        let positive = |v: f64| v.is_finite() && v > 0.0;
        let finite = |v: &[f64]| v.iter().all(|x| x.is_finite());
        self.frames >= 1
            && self.asked >= self.frames
            && self.steps.len() == self.frames as usize - 1
            && positive(self.fps)
            && positive(self.rate_scale)
            && positive(self.scale)
            && self.width > 0
            && self.height > 0
            && self.readout.iter().all(|r| r.is_finite())
            && self
                .steps
                .iter()
                .all(|s| finite(&s.similarity) && finite(&s.homography))
            && self.runs.windows(2).all(|w| w[0].id < w[1].id)
            && self.runs.iter().all(|r| {
                !r.points.is_empty()
                    && u64::from(r.first) + r.points.len() as u64 <= u64::from(self.frames)
                    && r.points.iter().flatten().all(|v| v.is_finite())
            })
    }

    /// What names this analysis's specks: their ids, where each starts and
    /// how long it lives, and where it starts. Two analyses with the same
    /// name hand out the same ids for the same specks, so a list of dropped
    /// ids kept under it means what it meant when it was made.
    #[must_use]
    pub fn points_key(&self) -> [u8; 32] {
        let mut h = blake3::Hasher::new();
        h.update(b"stabilise-points/");
        h.update(&self.first.to_le_bytes());
        h.update(&self.frames.to_le_bytes());
        for r in &self.runs {
            h.update(&r.id.to_le_bytes());
            h.update(&r.first.to_le_bytes());
            h.update(&(r.points.len() as u64).to_le_bytes());
            if let Some(p) = r.points.first() {
                h.update(&p[0].to_le_bytes());
                h.update(&p[1].to_le_bytes());
            }
        }
        *h.finalize().as_bytes()
    }
}

/// A finished analysis as the render reads it: the analysis itself and the
/// rows it was asked under, so a stale one is never used.
#[derive(Debug, Clone, PartialEq)]
pub struct Filed {
    /// What the effect's rows and the layer said when it was asked for. Two
    /// equal keys mean nothing that changes what an analysis finds has moved
    /// since; a different one is an analysis worth redoing, still drawn with
    /// until it is.
    pub key: [u8; 32],
    /// The footage item or nested comp it read. A layer now showing anything
    /// else is never drawn with it.
    pub source: Uuid,
    /// Bumped each time anything is filed, so a frame made under an earlier
    /// analysis is never served after a later one.
    pub generation: u64,
    /// [`Analysis::points_key`], which a dropped list is kept against.
    pub points: [u8; 32],
    pub analysis: Arc<Analysis>,
}

/// Every analysis in hand, by Stabilise instance.
fn table() -> &'static RwLock<HashMap<Uuid, Filed>> {
    static TABLE: OnceLock<RwLock<HashMap<Uuid, Filed>>> = OnceLock::new();
    TABLE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// The generation the next filing takes.
fn next_generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// File a finished analysis of `source` under the instance it was made for.
pub fn publish(effect: Uuid, key: [u8; 32], source: Uuid, analysis: Analysis) {
    let filed = Filed {
        key,
        source,
        generation: next_generation(),
        points: analysis.points_key(),
        analysis: Arc::new(analysis),
    };
    if let Ok(mut held) = table().write() {
        held.insert(effect, filed);
    }
}

/// The analysis filed for `effect`, whatever it was asked under.
#[must_use]
pub fn filed(effect: Uuid) -> Option<Filed> {
    table().read().ok()?.get(&effect).cloned()
}

/// What asks for an analysis: the effect, the document, its comp and its layer.
pub type Missing = dyn Fn(Uuid, &Arc<crate::model::Document>, Uuid, Uuid) + Send + Sync;

/// The renderer's way of asking for an analysis, once it has given one.
fn on_missing_cell() -> &'static OnceLock<Box<Missing>> {
    static ASK: OnceLock<Box<Missing>> = OnceLock::new();
    &ASK
}

/// Hand over how an analysis is asked for. The first one given is kept, which
/// is the renderer's: this crate cannot start one itself.
pub fn on_missing(ask: Box<Missing>) {
    let _ = on_missing_cell().set(ask);
}

/// A frame drew `effect` on `layer` of `comp` with nothing filed for it: ask
/// for an analysis. Nothing happens before the renderer has said how.
pub fn missing(effect: Uuid, doc: &Arc<crate::model::Document>, comp: Uuid, layer: Uuid) {
    if let Some(ask) = on_missing_cell().get() {
        ask(effect, doc, comp, layer);
    }
}

/// Drop every analysis `keep` does not ask for: what closing a project does.
/// The sidecar files are left alone.
pub fn retain(keep: impl Fn(&Uuid) -> bool) {
    if let Ok(mut held) = table().write() {
        held.retain(|id, _| keep(id));
    }
}
