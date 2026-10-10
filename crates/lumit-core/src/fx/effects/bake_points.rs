//! Bake points writes the stream wired into it to disk, a frame at a time,
//! and hands the written stream on.
//!
//! Some streams only exist while a frame is being drawn, because their
//! points depend on a picture: Scatter's, Emit from image's, and anything
//! changed by a pattern that reads one. A driver or another layer cannot read
//! those. A baked stream is plain data, so it can be read anywhere. It is also
//! quicker to read back than a heavy stream is to make.
//!
//! Press Bake and the layer's span is walked once, on its own thread. What it
//! finds is kept beside the Track points analyses, in the `track/` cache
//! folder and not in the project. Mode says what the effect hands on: the
//! bake when it still matches what is above it, the bake whatever has
//! changed, or the live stream.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::fx::effects::vary_points::{POINTS_IN, POINTS_OUT};
use crate::fx::points::{self, PointsStream, Projection};
use crate::fx::{
    EffectDef, EffectMetadata, EffectSchema, ParamGroup, ParamId, Params, ResolveCx, Signature,
    Value,
};
use crate::model::{Composition, Document, EffectInstance, EffectValue, Layer, LayerKind};
use lumit_fx_macros::Effect;

/// One spelling, because the bake job and the button's doorway both compare
/// against it.
pub const MATCH_NAME: &str = "bake_points";

/// The Mode index that hands on the bake whatever has changed above.
const MODE_READ: u32 = 1;
/// The Mode index that never reads a bake.
pub const MODE_BYPASS: u32 = 2;

/// The most frames one bake holds.
///
/// ponytail: three fixed budgets and no rows for them. A bake lives whole in
/// memory once it is read, and [`MAX_BYTES`] is what bounds that. Past any of
/// the three the bake stops early and the frames after it read as not baked.
/// Reading frames from the file as they are asked for is the upgrade.
pub const MAX_FRAMES: usize = 20_000;
/// The most points kept from one frame. See [`MAX_FRAMES`].
pub const MAX_POINTS: usize = 100_000;
/// The most one bake holds over all its frames, in bytes: 160 MB. It is
/// counted as the frames are packed, so it is also the size of the file
/// before it is compressed, give or take the few bytes that say what the
/// bake is. About two million points of a plain stream, and fewer of one
/// that carries named columns, each of which may hold four numbers a point.
/// See [`MAX_FRAMES`].
pub const MAX_BYTES: usize = 160 << 20;

/// What a frame is counted as costing before any of its points: the lengths
/// and the one-value columns, which come to about 600 bytes in the file.
const FRAME_BYTES: usize = 1024;

/// Just the disc's softness, since the size and colour are the stream's own.
pub const BAKE_GROUPS: &[ParamGroup] = &[ParamGroup {
    label: "Point",
    params: &["feather"],
    collapsed: false,
    visible_when: None,
    visible_when_lens_elements: None,
}];

/// Bake points' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "bake_points",
    label = "Bake points",
    version = 1,
    category = Generate,
    cost = Moderate,
    roi = FullFrame,
    premultiplied = true,
    // Not seeded: it has no clock of its own, and what feeds it is in the
    // frame's name already.
    seeded = false,
    groups = BAKE_GROUPS,
)]
pub struct BakePoints {
    /// Which stream is handed on. Auto is the bake while it still matches
    /// what is above this effect, and the live stream otherwise. Read is the
    /// bake whatever has changed since, and nothing where there is none.
    /// Bypass is always the live stream.
    #[choice(label = "Mode", options = ["Auto", "Read", "Bypass"], default = 0)]
    pub mode: u32,

    /// Start the bake. A button, not a value.
    #[action(label = "Bake")]
    pub bake: (),

    /// Stop a running bake.
    #[action(label = "Cancel")]
    pub cancel: (),

    /// How soft the disc a point is drawn as is, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub feather: f32,

    /// The Mix every effect ends with, per cent. At 0 the stream is still
    /// handed on and nothing is drawn.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub mix: f32,
}

/// One number per point, or one number for all of them.
#[derive(Debug, Clone, PartialEq)]
enum Column {
    /// Every point has this value, so it is written once.
    Same(f32),
    /// One value per point. Empty for a column the stream did not carry.
    Each(Vec<f32>),
}

impl Column {
    /// Compared by bits, so what is read back is what was written, down to
    /// the sign of a nought.
    fn of(values: Vec<f32>) -> Self {
        match values.first() {
            Some(first) if values.iter().all(|v| v.to_bits() == first.to_bits()) => {
                Column::Same(*first)
            }
            _ => Column::Each(values),
        }
    }

    fn at(&self, i: usize) -> f32 {
        match self {
            Column::Same(v) => *v,
            Column::Each(values) => values.get(i).copied().unwrap_or(0.0),
        }
    }
}

/// A frame's ids: where they start when they count up by one, or all of them.
#[derive(Debug, Clone, PartialEq)]
enum Ids {
    Run(u64),
    Each(Vec<u64>),
}

/// How many columns every frame has: position, speed, age, life, size,
/// rotation and colour, then the three a stream may leave out, stretch, pick
/// and index. A stream's named columns follow them, four each.
const COLUMNS: usize = 18;
/// Where the columns a stream may leave out begin.
const STRETCH: usize = 14;
const PICK: usize = 16;
const INDEX: usize = 17;

/// One frame of a stream, with nothing written twice.
///
/// Most of a stream says the same thing for every point: no speed, one
/// colour, one size, nothing off the layer's plane. A column like that is one
/// number here, which is most of why a bake is small.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    count: u32,
    columns: Vec<Column>,
    ids: Ids,
    /// The name and width of each named column, in the stream's own order.
    /// Their numbers are the columns after the first [`COLUMNS`], four a
    /// name.
    names: Vec<(String, u8)>,
    /// The stream's own raster factor, kept as it came.
    px_scale: f32,
}

impl Frame {
    fn pack(s: &PointsStream) -> Self {
        let n = s.len();
        let mut columns = Vec::with_capacity(COLUMNS);
        // One column per component, so a stream flat on its layer costs
        // nothing for its third axis. A column the stream left out stays
        // empty.
        fn split<const N: usize>(rows: &[[f32; N]], n: usize, out: &mut Vec<Column>) {
            for axis in 0..N {
                out.push(if rows.is_empty() {
                    Column::Each(Vec::new())
                } else {
                    Column::of(
                        (0..n)
                            .map(|i| {
                                rows.get(i)
                                    .and_then(|r| r.get(axis))
                                    .copied()
                                    .unwrap_or(0.0)
                            })
                            .collect(),
                    )
                });
            }
        }
        let one = |values: &[f32], out: &mut Vec<Column>| {
            out.push(if values.is_empty() {
                Column::Each(Vec::new())
            } else {
                Column::of(
                    (0..n)
                        .map(|i| values.get(i).copied().unwrap_or(0.0))
                        .collect(),
                )
            });
        };
        split(&s.position, n, &mut columns);
        split(&s.speed, n, &mut columns);
        one(&s.age, &mut columns);
        one(&s.life, &mut columns);
        one(&s.size, &mut columns);
        one(&s.rotation, &mut columns);
        split(&s.colour, n, &mut columns);
        split(&s.stretch, n, &mut columns);
        one(&s.pick, &mut columns);
        one(&s.index, &mut columns);
        let named = s.named.iter().take(points::NAMED_MAX);
        let names = (named.clone().map(|c| (c.name.clone(), c.width))).collect();
        for column in named {
            split(&column.values, n, &mut columns);
        }
        let first = s.id.first().copied().unwrap_or(0);
        let run = (s.id.iter().zip(0u64..)).all(|(id, step)| first.checked_add(step) == Some(*id));
        Frame {
            count: u32::try_from(n).unwrap_or(u32::MAX),
            columns,
            ids: if run {
                Ids::Run(first)
            } else {
                Ids::Each(s.id.clone())
            },
            names,
            px_scale: s.px_scale,
        }
    }

    /// What this frame counts as against [`MAX_BYTES`].
    fn bytes(&self) -> usize {
        let numbers: usize = (self.columns.iter())
            .map(|column| match column {
                Column::Each(values) => values.len() * 4,
                Column::Same(_) => 0,
            })
            .sum();
        let ids = match &self.ids {
            Ids::Each(ids) => ids.len() * 8,
            Ids::Run(_) => 0,
        };
        let names: usize = self.names.iter().map(|(name, _)| name.len()).sum();
        FRAME_BYTES + numbers + ids + names
    }

    fn unpack(&self, projection: Projection) -> PointsStream {
        let n = self.count as usize;
        let at = |column: usize, i: usize| self.columns.get(column).map_or(0.0, |c| c.at(i));
        let carried = |column: usize| match self.columns.get(column) {
            Some(Column::Each(values)) => !values.is_empty(),
            Some(Column::Same(_)) => true,
            None => false,
        };
        let each = |column: usize| (0..n).map(|i| at(column, i)).collect::<Vec<f32>>();
        PointsStream {
            position: (0..n).map(|i| [at(0, i), at(1, i), at(2, i)]).collect(),
            speed: (0..n).map(|i| [at(3, i), at(4, i), at(5, i)]).collect(),
            age: each(6),
            life: each(7),
            size: each(8),
            rotation: each(9),
            colour: (0..n)
                .map(|i| [at(10, i), at(11, i), at(12, i), at(13, i)])
                .collect(),
            id: match &self.ids {
                Ids::Run(first) => (0..n as u64).map(|i| first.saturating_add(i)).collect(),
                Ids::Each(ids) => ids.clone(),
            },
            projection,
            stretch: if carried(STRETCH) {
                (0..n)
                    .map(|i| [at(STRETCH, i), at(STRETCH + 1, i)])
                    .collect()
            } else {
                Vec::new()
            },
            pick: if carried(PICK) {
                each(PICK)
            } else {
                Vec::new()
            },
            index: if carried(INDEX) {
                each(INDEX)
            } else {
                Vec::new()
            },
            named: (self.names.iter().zip((COLUMNS..).step_by(4)))
                .map(|((name, width), first)| points::Named {
                    name: name.clone(),
                    width: *width,
                    values: if carried(first) {
                        (0..n)
                            .map(|i| std::array::from_fn(|k| at(first + k, i)))
                            .collect()
                    } else {
                        Vec::new()
                    },
                })
                .collect(),
            px_scale: self.px_scale,
        }
    }
}

/// A finished bake: a stream for every frame of a layer's span.
#[derive(Debug, Clone, PartialEq)]
pub struct Baked {
    /// What was above the effect when this was baked. See [`fingerprint`].
    pub fingerprint: [u8; 32],
    /// A hash of the frames themselves. A frame's name takes it, so a new
    /// bake is never served a frame drawn from the old one.
    pub content: [u8; 32],
    /// The composition's rate, which the frames count at.
    pub fps: f64,
    /// The composition frame the first baked frame is.
    pub first: i64,
    /// How many frames the layer's span had. More than [`frames`](Self::frames)
    /// holds when a budget stopped the bake early.
    pub span: u32,
    /// Which of [`unique`](Self::unique) each frame reads, so a stream that
    /// holds still is written once.
    pub frames: Vec<u32>,
    pub unique: Vec<Frame>,
}

impl Baked {
    /// An empty bake of a span starting at composition frame `first`.
    #[must_use]
    pub fn new(fingerprint: [u8; 32], fps: f64, first: i64, span: u32) -> Self {
        Baked {
            fingerprint,
            content: [0; 32],
            fps,
            first,
            span,
            frames: Vec::new(),
            unique: Vec::new(),
        }
    }

    /// Take the next frame's stream, if it fits in `room` bytes. Answers how
    /// many bytes it added to what the bake holds, which is none for a frame
    /// the same as the one before. `None` when there is no room, and the
    /// frame is not kept.
    pub fn push(&mut self, stream: &PointsStream, room: usize) -> Option<usize> {
        let frame = Frame::pack(stream);
        let added = if self.unique.last() == Some(&frame) {
            0
        } else {
            let bytes = frame.bytes();
            if bytes > room {
                return None;
            }
            self.unique.push(frame);
            bytes
        };
        let held = self.unique.len().saturating_sub(1);
        self.frames.push(u32::try_from(held).unwrap_or(u32::MAX));
        Some(added)
    }

    /// Hash what was pushed. Called once, when the last frame is in.
    pub fn seal(&mut self) {
        let mut h = blake3::Hasher::new();
        for frame in &self.unique {
            h.update(&frame.count.to_le_bytes());
            h.update(&frame.px_scale.to_le_bytes());
            h.update(&(frame.names.len() as u64).to_le_bytes());
            for (name, width) in &frame.names {
                h.update(&[*width]);
                h.update(&(name.len() as u64).to_le_bytes());
                h.update(name.as_bytes());
            }
            for column in &frame.columns {
                match column {
                    Column::Same(v) => {
                        h.update(&[0]).update(&v.to_le_bytes());
                    }
                    Column::Each(values) => {
                        h.update(&[1]).update(&(values.len() as u64).to_le_bytes());
                        for v in values {
                            h.update(&v.to_le_bytes());
                        }
                    }
                }
            }
            match &frame.ids {
                Ids::Run(first) => {
                    h.update(&[0]).update(&first.to_le_bytes());
                }
                Ids::Each(ids) => {
                    h.update(&[1]).update(&(ids.len() as u64).to_le_bytes());
                    for id in ids {
                        h.update(&id.to_le_bytes());
                    }
                }
            }
        }
        for frame in &self.frames {
            h.update(&frame.to_le_bytes());
        }
        self.content = *h.finalize().as_bytes();
    }
}

/// A bake as it is written to disk, laid out for a compressor.
///
/// A point moves a little from one frame to the next, so most of the bits of
/// each number are the bits it had the frame before. Every number is written
/// as the bits that changed since the same point's on the frame before, and
/// a column's bytes are regrouped so the high bytes of every number sit
/// together. The high bytes are then long runs of nought, which is what LZ4
/// is good at. Nothing is rounded: reading it back gives the same bits.
///
/// ponytail: the guess for a number is its value on the frame before, which
/// comes to about 24 bytes a point where every column of a stream moves, and
/// next to nothing for one that holds still. Guessing from the two frames
/// before, or Deflate in place of LZ4, is the upgrade if bakes of busy
/// streams grow too large.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stored {
    fingerprint: [u8; 32],
    content: [u8; 32],
    fps: f64,
    first: i64,
    span: u32,
    frames: Vec<u32>,
    unique: Vec<StoredFrame>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct StoredFrame {
    count: u32,
    /// Empty for [`Column::Same`], whose one value is in `same`.
    columns: Vec<Vec<u8>>,
    same: Vec<Option<f32>>,
    /// The first id of a run, or nothing for ids written out in `ids`.
    run: Option<u64>,
    /// Each id less the one before it, regrouped as the columns are.
    ids: Vec<u8>,
    names: Vec<(String, u8)>,
    px_scale: f32,
}

/// `words` with their bytes regrouped: every first byte, then every second,
/// and so on.
fn regroup<const N: usize>(words: &[[u8; N]]) -> Vec<u8> {
    (0..N)
        .flat_map(|byte| words.iter().map(move |word| word[byte]))
        .collect()
}

/// The inverse of [`regroup`], for `n` words. `None` when the bytes are not
/// `n` words long.
fn ungroup<const N: usize>(bytes: &[u8], n: usize) -> Option<Vec<[u8; N]>> {
    if bytes.len() != n.checked_mul(N)? {
        return None;
    }
    Some(
        (0..n)
            .map(|i| std::array::from_fn(|byte| bytes.get(byte * n + i).copied().unwrap_or(0)))
            .collect(),
    )
}

impl Frame {
    fn id_list(&self) -> Vec<u64> {
        match &self.ids {
            Ids::Run(first) => (0..u64::from(self.count))
                .map(|i| first.saturating_add(i))
                .collect(),
            Ids::Each(ids) => ids.clone(),
        }
    }

    /// For each of `ids`, where the same point sits in this frame.
    fn places(&self, ids: &[u64]) -> Vec<Option<usize>> {
        let mut at: HashMap<u64, usize> = HashMap::new();
        for (i, id) in self.id_list().into_iter().enumerate() {
            at.entry(id).or_insert(i);
        }
        ids.iter().map(|id| at.get(id).copied()).collect()
    }

    /// The bits of column `column` for the point at `place` in this frame, or
    /// nought for a point that was not in it.
    fn bits(&self, column: usize, place: Option<usize>) -> u32 {
        match (self.columns.get(column), place) {
            (Some(c), Some(i)) => c.at(i).to_bits(),
            _ => 0,
        }
    }

    fn store(&self, before: Option<&Frame>) -> StoredFrame {
        let ids = self.id_list();
        let places = before.map(|b| b.places(&ids));
        let changed = |column: usize, i: usize, value: f32| {
            let place = places.as_ref().and_then(|p| p.get(i).copied().flatten());
            let was = before.map_or(0, |b| b.bits(column, place));
            (value.to_bits() ^ was).to_le_bytes()
        };
        let mut columns = Vec::with_capacity(self.columns.len());
        let mut same = Vec::with_capacity(self.columns.len());
        for (k, column) in self.columns.iter().enumerate() {
            match column {
                Column::Same(v) => {
                    columns.push(Vec::new());
                    same.push(Some(*v));
                }
                Column::Each(values) => {
                    let words: Vec<[u8; 4]> = values
                        .iter()
                        .enumerate()
                        .map(|(i, v)| changed(k, i, *v))
                        .collect();
                    columns.push(regroup(&words));
                    same.push(None);
                }
            }
        }
        let (run, steps) = match &self.ids {
            Ids::Run(first) => (Some(*first), Vec::new()),
            Ids::Each(ids) => {
                let before = std::iter::once(0).chain(ids.iter().copied());
                let steps: Vec<[u8; 8]> = ids
                    .iter()
                    .zip(before)
                    .map(|(id, was)| id.wrapping_sub(was).to_le_bytes())
                    .collect();
                (None, regroup(&steps))
            }
        };
        StoredFrame {
            count: self.count,
            columns,
            same,
            run,
            ids: steps,
            names: self.names.clone(),
            px_scale: self.px_scale,
        }
    }

    fn restore(stored: &StoredFrame, before: Option<&Frame>) -> Option<Frame> {
        // The shape a frame is always written in. Anything else is not one.
        let columns = stored.columns.len();
        if stored.names.len() > points::NAMED_MAX
            || columns != COLUMNS + 4 * stored.names.len()
            || columns != stored.same.len()
        {
            return None;
        }
        let n = stored.count as usize;
        let ids = match stored.run {
            Some(first) => Ids::Run(first),
            None => {
                let mut id = 0u64;
                Ids::Each(
                    ungroup::<8>(&stored.ids, n)?
                        .into_iter()
                        .map(|step| {
                            id = id.wrapping_add(u64::from_le_bytes(step));
                            id
                        })
                        .collect(),
                )
            }
        };
        let mut frame = Frame {
            count: stored.count,
            columns: Vec::with_capacity(stored.columns.len()),
            ids,
            names: stored.names.clone(),
            px_scale: stored.px_scale,
        };
        let places = before.map(|b| b.places(&frame.id_list()));
        for (k, (bytes, same)) in stored.columns.iter().zip(&stored.same).enumerate() {
            if let Some(v) = same {
                frame.columns.push(Column::Same(*v));
                continue;
            }
            // A column the stream did not carry is empty however many points
            // there are.
            let carried = if bytes.is_empty() { 0 } else { n };
            let values = ungroup::<4>(bytes, carried)?
                .into_iter()
                .enumerate()
                .map(|(i, word)| {
                    let place = places.as_ref().and_then(|p| p.get(i).copied().flatten());
                    let was = before.map_or(0, |b| b.bits(k, place));
                    f32::from_bits(u32::from_le_bytes(word) ^ was)
                })
                .collect();
            frame.columns.push(Column::Each(values));
        }
        Some(frame)
    }
}

impl Baked {
    /// This bake as it is written to disk.
    #[must_use]
    pub fn stored(&self) -> Stored {
        let before = std::iter::once(None).chain(self.unique.iter().map(Some));
        Stored {
            fingerprint: self.fingerprint,
            content: self.content,
            fps: self.fps,
            first: self.first,
            span: self.span,
            frames: self.frames.clone(),
            unique: self
                .unique
                .iter()
                .zip(before)
                .map(|(frame, before)| frame.store(before))
                .collect(),
        }
    }

    /// The bake a file held. `None` for one that does not add up, which is
    /// read as no bake.
    #[must_use]
    pub fn restored(stored: &Stored) -> Option<Self> {
        let mut unique: Vec<Frame> = Vec::with_capacity(stored.unique.len());
        for frame in &stored.unique {
            let frame = Frame::restore(frame, unique.last())?;
            unique.push(frame);
        }
        Some(Baked {
            fingerprint: stored.fingerprint,
            content: stored.content,
            fps: stored.fps,
            first: stored.first,
            span: stored.span,
            frames: stored.frames.clone(),
            unique,
        })
    }
}

/// Every bake in hand, by Bake points instance. Track points' table, for its
/// reason: the stream is handed out in this crate at resolve time, and the
/// walk that does it is given no store. The bake job in `lumit-render` puts
/// them in. The lock is only ever held to clone or swap one `Arc`.
fn table() -> &'static RwLock<HashMap<Uuid, Arc<Baked>>> {
    static TABLE: OnceLock<RwLock<HashMap<Uuid, Arc<Baked>>>> = OnceLock::new();
    TABLE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Put a finished bake in the table under the instance it was made for.
pub fn publish(effect: Uuid, baked: Baked) {
    if let Ok(mut held) = table().write() {
        held.insert(effect, Arc::new(baked));
    }
}

/// The bake in hand for `effect`, whatever it was made from.
#[must_use]
pub fn baked(effect: Uuid) -> Option<Arc<Baked>> {
    table().read().ok()?.get(&effect).cloned()
}

/// Drop every bake `keep` does not ask for, which is what closing a project
/// does. The files in the cache folder are left alone.
pub fn retain(keep: impl Fn(&Uuid) -> bool) {
    if let Ok(mut held) = table().write() {
        held.retain(|id, _| keep(id));
    }
}

/// A hash of everything in the document that decides the stream wired into
/// `effect` on `layer`. A bake made under one fingerprint is stale under
/// another.
///
/// It covers the layer itself with the stack cut off above this effect: every
/// stored row and its keyframes, the wires and the drivers, the masks, the
/// text or the source it shows and when it shows it. It covers the footage
/// item, solid or nested composition the layer shows, every layer a row or a
/// tap above names, and the composition's size, rate and length. What only
/// places the finished picture is left out, so moving the layer keeps a bake.
///
/// It does not cover what the document does not hold or what is further
/// off: a media file changed on disk, compositions nested deeper than the
/// layer's own source, the clips of a Sequence layer, the sources of the
/// layers it names, layers an expression reads, and what an adjustment layer
/// sees beneath it. Bake again after changing one of those.
///
/// `None` when `effect` is not on `layer`.
#[must_use]
pub fn fingerprint(
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &Layer,
    effect: Uuid,
) -> Option<[u8; 32]> {
    // Asked on every resolve, and a document does not change once it is
    // shared. So the answer is kept per document, by its address. A held
    // `Weak` keeps the address from being given to another document.
    type Kept = (Weak<Document>, Uuid, Uuid, Option<[u8; 32]>);
    static MEMO: Mutex<Vec<Kept>> = Mutex::new(Vec::new());
    let same = |kept: &Kept| {
        kept.1 == layer.id && kept.2 == effect && std::ptr::eq(kept.0.as_ptr(), Arc::as_ptr(doc))
    };
    let kept = |memo: &Vec<Kept>| memo.iter().find(|k| same(k)).map(|k| k.3);
    if let Some(made) = MEMO.lock().ok().and_then(|memo| kept(&memo)) {
        return made;
    }
    let made = fingerprint_of(doc, comp, layer, effect);
    if let Ok(mut memo) = MEMO.lock() {
        memo.retain(|kept| kept.0.strong_count() > 0);
        if memo.len() >= 64 {
            memo.remove(0);
        }
        memo.push((Arc::downgrade(doc), layer.id, effect, made));
    }
    made
}

fn fingerprint_of(
    doc: &Document,
    comp: &Composition,
    layer: &Layer,
    effect: Uuid,
) -> Option<[u8; 32]> {
    // The model's own serialisation, straight into the hash. Its maps are
    // ordered, so the same document gives the same bytes on every run.
    fn feed<T: Serialize>(h: &mut blake3::Hasher, value: &T) {
        let _ = serde_json::to_writer(&mut *h, value);
        h.update(&[0]);
    }
    let at = layer.effects.iter().position(|e| e.id == effect)?;
    let mut h = blake3::Hasher::new();
    h.update(b"lumit-bake/1/");
    feed(
        &mut h,
        &(comp.width, comp.height, comp.frame_rate, comp.duration),
    );
    let mut above = layer.clone();
    above.effects.truncate(at);
    above.styles.clear();
    above.transform = Default::default();
    above.parent = None;
    above.name.clear();
    above.label = 0;
    above.blend = Default::default();
    above.switches = Default::default();
    // Of the graph, only what feeds this effect or one above it: the wires
    // into them, and the drivers those wires lead back to. A driver reading
    // this effect's own stream is below it, and wiring one must not make the
    // bake it is there to read stale.
    use crate::graph::{InputRef, NodeRef, OutputRef};
    let mut reached: Vec<NodeRef> = layer
        .effects
        .iter()
        .take(at + 1)
        .map(|e| NodeRef::Effect(e.id))
        .collect();
    let feeds = |to: &InputRef, reached: &[NodeRef]| match to {
        InputRef::Param { node, .. } => reached.contains(node),
        InputRef::Matte { effect } => reached.contains(&NodeRef::Effect(*effect)),
    };
    loop {
        let before = reached.len();
        for edge in &layer.graph.edges {
            if let OutputRef::Driver { node, .. } = edge.from {
                let driver = NodeRef::Driver(node);
                if feeds(&edge.to, &reached) && !reached.contains(&driver) {
                    reached.push(driver);
                }
            }
        }
        if reached.len() == before {
            break;
        }
    }
    above.graph.edges.retain(|edge| feeds(&edge.to, &reached));
    above
        .graph
        .nodes
        .retain(|node| reached.contains(&NodeRef::Driver(node.id)));
    above.graph.layout.clear();
    above.graph.exposed.clear();
    above.graph.groups.clear();
    feed(&mut h, &above);
    let source = match layer.kind {
        LayerKind::Footage { item } => Some(item),
        LayerKind::Solid { def } => Some(def),
        LayerKind::Precomp { comp: nested } => Some(nested),
        _ => None,
    };
    for item in doc.items.iter().filter(|i| Some(i.id()) == source) {
        feed(&mut h, item);
    }
    let named = above
        .effects
        .iter()
        .chain(&above.graph.nodes)
        .flat_map(|e| &e.params)
        .filter_map(|p| match p.value {
            EffectValue::Layer(Some(id)) => Some(id),
            _ => None,
        });
    for id in named {
        if let Some(other) = comp.layers.iter().find(|l| l.id == id) {
            feed(&mut h, other);
        }
    }
    Some(*h.finalize().as_bytes())
}

/// The bake `inst` may read: the one in hand, if it was made from what is
/// above the effect now, or whatever it was made from when Mode is Read.
#[must_use]
pub fn usable(
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &Layer,
    inst: &EffectInstance,
) -> Option<Arc<Baked>> {
    let baked = baked(inst.id)?;
    let fresh = || fingerprint(doc, comp, layer, inst.id) == Some(baked.fingerprint);
    (mode_of(inst) == MODE_READ || fresh()).then_some(baked)
}

fn mode_of(inst: &EffectInstance) -> u32 {
    match inst.param("mode") {
        Some(EffectValue::Choice(v)) => *v,
        _ => 0,
    }
}

/// What a Bake points instance hands on at one moment.
#[derive(Debug, Clone)]
pub enum Reading {
    /// The stream wired into it.
    Live,
    /// No points at all: Read, with no bake for this frame.
    Nothing,
    /// This frame of a bake, as its place in [`Baked::unique`].
    Baked(Arc<Baked>, u32),
}

impl Reading {
    /// The stream this reading stands for, in px@comp. `None` for
    /// [`Reading::Live`], which is whatever the wire brings.
    #[must_use]
    pub fn stream(&self, projection: Projection) -> Option<PointsStream> {
        match self {
            Reading::Live => None,
            Reading::Nothing => Some(PointsStream {
                projection,
                ..PointsStream::default()
            }),
            Reading::Baked(baked, unique) => Some(
                baked
                    .unique
                    .get(*unique as usize)
                    .map(|frame| frame.unpack(projection))
                    .unwrap_or_default(),
            ),
        }
    }
}

/// What `inst` on `layer` hands on at layer time `t`, by its Mode and the
/// bake in hand. The one answer every reader goes by: the resolve walk, the
/// draw builder, the frame's name and the status line.
///
/// ponytail: a moment between two frames reads the nearer frame, so baked
/// points hold still inside a frame and accumulation motion blur has nothing
/// to smear. Blending the two frames by id is the upgrade.
#[must_use]
pub fn reading(
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &Layer,
    inst: &EffectInstance,
    t: f64,
) -> Reading {
    let mode = mode_of(inst);
    if mode == MODE_BYPASS {
        return Reading::Live;
    }
    let miss = if mode == MODE_READ {
        Reading::Nothing
    } else {
        Reading::Live
    };
    let Some(baked) = usable(doc, comp, layer, inst) else {
        return miss;
    };
    let frame = ((t + layer.start_offset.0.to_f64()) * baked.fps).round();
    if !frame.is_finite() {
        return miss;
    }
    #[allow(clippy::cast_possible_truncation)]
    let frame = (frame as i64).checked_sub(baked.first);
    match frame
        .and_then(|k| usize::try_from(k).ok())
        .and_then(|k| baked.frames.get(k).copied())
    {
        Some(unique) => Reading::Baked(baked, unique),
        None => miss,
    }
}

impl BakePoints {
    /// The raster factor, since a stream off a wire arrives in px@comp.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// Which bake is being read, as the head of its content hash. In the bag
    /// so the picture kept for this effect is not served after a new bake.
    pub const DERIVED_BAKE: ParamId = ParamId::new("derived.bake");

    /// Which frame of the bake is being read, or a negative number: -1 for
    /// the live stream and -2 for nothing.
    pub const DERIVED_FRAME: ParamId = ParamId::new("derived.frame");
}

/// Bake points' behaviour. The stream it changes nothing in: which stream it
/// is handed is the whole of the effect, and that is decided by [`reading`]
/// before this is called.
pub struct BakePointsDef;

impl EffectDef for BakePointsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<BakePoints as EffectMetadata>::SCHEMA
    }

    /// A stream in and a stream out, beside the picture.
    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: POINTS_IN,
            extra: POINTS_OUT,
        }
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(BakePoints::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
        let doc = &cx.context.document;
        let read = || {
            let comp = doc.comp(cx.context.comp?)?;
            let layer = comp
                .layers
                .iter()
                .find(|l| Some(l.id) == cx.context.layer)?;
            Some(reading(doc, comp, layer, cx.inst, cx.lt))
        };
        let (bake, frame) = match read() {
            Some(Reading::Baked(baked, unique)) => {
                let [a, b, c, d, ..] = baked.content;
                (
                    u32::from_le_bytes([a, b, c, d]),
                    i32::try_from(unique).unwrap_or(i32::MAX),
                )
            }
            Some(Reading::Nothing) => (0, -2),
            _ => (0, -1),
        };
        push(BakePoints::DERIVED_BAKE, Value::Choice(bake));
        push(BakePoints::DERIVED_FRAME, Value::Int(frame));
    }

    /// The stream it was handed, which is the bake where one is being read.
    fn modify_points(&self, _p: Params<'_>, cx: &crate::fx::ModifyCx<'_>) -> Option<PointsStream> {
        cx.input().cloned()
    }
}
