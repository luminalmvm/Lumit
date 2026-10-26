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
    params::ParamId, params::Params, params::Value, registry::EffectDef, registry::EffectMetadata,
    registry::ResolveCx, registry::Signature, schema::EffectSchema, schema::ParamGroup,
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
        // No frame is written with more, and everything below is sized by
        // this number, which a file can say anything in.
        if stored.count as usize > MAX_POINTS {
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
    fn modify_points(
        &self,
        _p: Params<'_>,
        cx: &crate::fx::registry::ModifyCx<'_>,
    ) -> Option<PointsStream> {
        cx.input().cloned()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::anim::Property;
    use crate::expression::ExpressionContext;
    use crate::fx::MarkerContext;
    use crate::graph::{Edge, InputRef, NodeRef, OutputRef};
    use crate::model::*;
    use crate::time::{CompTime, Duration, FrameRate, Rational};

    /// A stream of the points `ids` at time `t`: every column carried, each
    /// point somewhere of its own, and one colour for all of them.
    fn points(ids: &[u64], t: f32) -> PointsStream {
        let each =
            |f: &dyn Fn(f32) -> f32| -> Vec<f32> { ids.iter().map(|&id| f(id as f32)).collect() };
        PointsStream {
            position: ids
                .iter()
                .map(|&id| {
                    [
                        100.0 + id as f32 * 7.5 + t * 30.0,
                        50.0 - t * id as f32,
                        0.0,
                    ]
                })
                .collect(),
            speed: ids.iter().map(|&id| [30.0, -(id as f32), 0.0]).collect(),
            age: each(&|id| t + id * 0.01),
            life: each(&|_| 2.0),
            size: each(&|id| 4.0 + id),
            rotation: each(&|id| id * 0.3 + t),
            colour: ids.iter().map(|_| [0.5, 0.25, 1.0, 1.0]).collect(),
            id: ids.to_vec(),
            projection: Projection::FLAT,
            stretch: ids.iter().map(|&id| [1.0, 1.0 + id as f32 * 0.1]).collect(),
            pick: ids.iter().map(|&id| (id % 2) as f32).collect(),
            index: Vec::new(),
            named: vec![
                points::Named {
                    name: "tint".into(),
                    width: 4,
                    values: ids
                        .iter()
                        .map(|&id| [id as f32 * 0.1, t, 0.5, 1.0])
                        .collect(),
                },
                points::Named {
                    name: "heat".into(),
                    width: 1,
                    values: ids
                        .iter()
                        .map(|&id| [id as f32 + t, 0.0, 0.0, 0.0])
                        .collect(),
                },
            ],
            px_scale: 0.5,
        }
    }

    fn bits(s: &PointsStream) -> Vec<u32> {
        let rows = s.position.iter().chain(&s.speed).flatten();
        let ones = s.age.iter().chain(&s.rotation).chain(&s.size);
        rows.chain(ones).map(|v| v.to_bits()).collect()
    }

    /// The frames of a bake: still for a frame, then point 1 dies, 40 is
    /// born and the rest come in another order, then nothing at all, then
    /// points with no stretch or pick and a nought with its sign.
    fn frames() -> Vec<PointsStream> {
        let mut last = points(&[0, 1, 2, 3, 4, 5], 2.0 / 24.0);
        last.stretch.clear();
        last.pick.clear();
        last.rotation[0] = -0.0;
        vec![
            points(&[0, 1, 2, 3, 4, 5], 0.0),
            points(&[0, 1, 2, 3, 4, 5], 0.0),
            points(&[5, 3, 0, 2, 4, 40], 1.0 / 24.0),
            PointsStream {
                px_scale: 0.5,
                ..PointsStream::default()
            },
            last,
        ]
    }

    fn bake(fingerprint: [u8; 32]) -> Baked {
        let mut baked = Baked::new(fingerprint, 24.0, 0, 5);
        for stream in frames() {
            assert!(baked.push(&stream, MAX_BYTES).is_some());
        }
        baked.seal();
        baked
    }

    /// What is read back from a bake's file is the stream that was baked, to
    /// the bit, however its points were born, died or shuffled between
    /// frames. Each number is written as its change from the same point on
    /// the frame before, so a slip in matching points up would read back as
    /// points in the wrong place.
    #[test]
    fn a_bake_reads_back_from_its_file_to_the_bit() {
        let streams = frames();
        let mut baked = Baked::new([3; 32], 24.0, 0, 5);
        let mut added = Vec::new();
        for stream in &streams {
            added.push(baked.push(stream, MAX_BYTES).expect("there is room"));
        }
        assert_eq!(added[1], 0, "a frame that holds still costs nothing");
        assert_eq!(baked.frames, [0, 0, 1, 2, 3]);
        assert_eq!(baked.unique.len(), 4);
        // A frame with no room is left out, and the bake is as it was.
        assert_eq!(baked.push(&streams[2], 10), None);
        assert_eq!(baked.frames.len(), 5);

        baked.seal();
        assert_ne!(baked.content, [0; 32]);
        let mut other = bake([3; 32]);
        assert_eq!(
            other.content, baked.content,
            "the same frames, the same hash"
        );
        other.unique[2].px_scale = 1.0;
        other.seal();
        assert_ne!(other.content, baked.content, "and other frames another");

        let stored = baked.stored();
        let back = Baked::restored(&stored).expect("the bake reads back");
        assert_eq!(back, baked);
        let back = Arc::new(back);
        for (k, stream) in streams.iter().enumerate() {
            let read = Reading::Baked(Arc::clone(&back), back.frames[k]);
            let read = read.stream(Projection::FLAT).expect("a baked frame");
            assert!(read == *stream, "frame {k} reads back as it was baked");
            assert_eq!(bits(&read), bits(stream), "frame {k}, to the bit");
        }

        // A file that does not add up is no bake, rather than a wrong one.
        let broken = |f: &dyn Fn(&mut StoredFrame)| {
            let mut s = stored.clone();
            f(&mut s.unique[1]);
            Baked::restored(&s)
        };
        assert!(broken(&|f| {
            f.ids.pop();
        })
        .is_none());
        assert!(broken(&|f| f.count = MAX_POINTS as u32 + 1).is_none());
        assert!(broken(&|f| {
            f.columns.pop();
        })
        .is_none());
        assert!(broken(&|f| f.columns[0].truncate(3)).is_none());
    }

    fn secs(n: i64, d: i64) -> CompTime {
        CompTime(Rational::new(n, d).unwrap())
    }

    fn set(inst: &mut EffectInstance, id: &str, value: f64) {
        let row = inst.params.iter_mut().find(|p| p.id == id).unwrap();
        row.value = EffectValue::Float(Property::fixed(value));
    }

    fn mode(inst: &mut EffectInstance, mode: u32) {
        let row = inst.params.iter_mut().find(|p| p.id == "mode").unwrap();
        row.value = EffectValue::Choice(mode);
    }

    /// A board with Pick points, Bake points and Connect points on it, and a
    /// Math driver wired into each of the other two.
    fn board() -> (Document, Uuid) {
        let pick = crate::fx::instantiate("pick_points").unwrap();
        let bake = crate::fx::instantiate(MATCH_NAME).unwrap();
        let connect = crate::fx::instantiate("connect_points").unwrap();
        let above = crate::fx::instantiate("math").unwrap();
        let below = crate::fx::instantiate("math").unwrap();
        let stream = |from: Uuid, to: Uuid| Edge {
            from: OutputRef::EffectData {
                effect: from,
                port: "points".into(),
            },
            to: InputRef::Param {
                node: NodeRef::Effect(to),
                port: "points".into(),
            },
        };
        let drive = |from: Uuid, to: Uuid, port: &str| Edge {
            from: OutputRef::Driver {
                node: from,
                port: "value".into(),
            },
            to: InputRef::Param {
                node: NodeRef::Effect(to),
                port: port.into(),
            },
        };
        let def = Uuid::now_v7();
        let mut layer = Layer {
            graph: Default::default(),
            markers: Vec::new(),
            id: Uuid::now_v7(),
            name: "Board".into(),
            kind: LayerKind::Solid { def },
            in_point: secs(0, 1),
            out_point: secs(5, 24),
            start_offset: secs(0, 1),
            transform: TransformGroup::default(),
            matte: None,
            parent: None,
            label: 0,
            volume_db: Property::zero(),
            pan: Property::zero(),
            audio_only: false,
            audio_streams: Default::default(),
            adjustment: false,
            pitch_follows_speed: false,
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
            graph_inputs: None,
            blend: BlendMode::Normal,
            masks: Vec::new(),
            paint: Vec::new(),
            puppet: None,
            effects: Vec::new(),
            styles: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        layer.graph.edges = vec![
            stream(pick.id, bake.id),
            stream(bake.id, connect.id),
            drive(above.id, pick.id, "to"),
            drive(below.id, connect.id, "max_distance"),
        ];
        layer.graph.nodes = vec![above, below];
        layer.effects = vec![pick, bake, connect];
        let comp = Composition {
            graph: None,
            master_volume_db: 0.0,
            lighting: Default::default(),
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
            id: Uuid::now_v7(),
            name: "Bake".into(),
            width: 640,
            height: 360,
            frame_rate: FrameRate::new(24, 1).unwrap(),
            duration: Duration(Rational::new(5, 24).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![layer],
            markers: Vec::new(),
            motion_blur: MotionBlur::default(),
            extra: serde_json::Map::new(),
        };
        let comp_id = comp.id;
        let mut doc = Document::new();
        doc.items.push(ProjectItem::Solid(SolidDef {
            id: def,
            name: "Board".into(),
            colour: LinearColour([0.1, 0.2, 0.9, 1.0]),
            width: 640,
            height: 360,
            extra: serde_json::Map::new(),
        }));
        doc.items.push(ProjectItem::Composition(comp));
        (doc, comp_id)
    }

    /// Auto hands on the bake only while what is above the effect is what it
    /// was baked from. Moving the layer, or changing what is below the
    /// effect, keeps it. Changing an effect above or a driver feeding one
    /// does not. Read hands on the bake whatever, and nothing past its end.
    #[test]
    fn auto_reads_a_bake_only_while_what_is_above_it_is_unchanged() {
        let (plain, comp_id) = board();
        let edited = |f: &dyn Fn(&mut Layer)| {
            let mut doc = plain.clone();
            f(&mut doc.comp_mut(comp_id).unwrap().layers[0]);
            Arc::new(doc)
        };
        let base = edited(&|_| {});
        let at = |doc: &Arc<Document>, t: f64| {
            let comp = doc.comp(comp_id).unwrap();
            let layer = &comp.layers[0];
            reading(doc, comp, layer, &layer.effects[1], t)
        };
        let frame = |doc: &Arc<Document>, t: f64| match at(doc, t) {
            Reading::Baked(_, unique) => Some(unique),
            _ => None,
        };
        let live = |doc: &Arc<Document>, t: f64| matches!(at(doc, t), Reading::Live);
        let comp = base.comp(comp_id).unwrap();
        let layer = &comp.layers[0];
        let effect = layer.effects[1].id;
        assert_eq!(fingerprint(&base, comp, layer, Uuid::now_v7()), None);
        assert!(live(&base, 0.0), "nothing is baked yet");

        let print = fingerprint(&base, comp, layer, effect).unwrap();
        publish(effect, bake(print));
        assert_eq!(frame(&base, 2.0 / 24.0), Some(1));
        assert_eq!(frame(&base, 4.0 / 24.0), Some(3));
        let read = at(&base, 2.0 / 24.0).stream(Projection::FLAT).unwrap();
        assert!(read == frames()[2], "Auto hands on the baked frame");

        let moved = edited(&|l| {
            l.transform.position_x = Property::fixed(400.0);
            l.name = "Moved".into();
        });
        assert_eq!(frame(&moved, 2.0 / 24.0), Some(1), "moving keeps a bake");
        let below = edited(&|l| {
            set(&mut l.effects[2], "max_distance", 3.0);
            set(&mut l.graph.nodes[1], "a", 9.0);
        });
        assert_eq!(frame(&below, 2.0 / 24.0), Some(1), "so does what is below");

        let above = edited(&|l| set(&mut l.effects[0], "to", 20.0));
        assert!(live(&above, 2.0 / 24.0), "an effect above counts");
        let driven = edited(&|l| set(&mut l.graph.nodes[0], "a", 9.0));
        assert!(live(&driven, 2.0 / 24.0), "and a driver feeding one");
        assert!(live(&base, 9.0), "past the bake is live");

        let read = edited(&|l| {
            set(&mut l.effects[0], "to", 20.0);
            mode(&mut l.effects[1], MODE_READ);
        });
        assert_eq!(frame(&read, 2.0 / 24.0), Some(1), "Read takes a stale bake");
        let past = at(&read, 9.0).stream(Projection::FLAT).unwrap();
        assert!(past.is_empty(), "and nothing past its end");
        let bypass = edited(&|l| mode(&mut l.effects[1], MODE_BYPASS));
        assert!(live(&bypass, 2.0 / 24.0));

        // What the effect resolves to says which frame of which bake is read,
        // so a picture drawn from one is never served for another.
        let derived = |doc: &Arc<Document>, t: f64| {
            let layer = &doc.comp(comp_id).unwrap().layers[0];
            let context = ExpressionContext {
                document: Arc::clone(doc),
                comp: Some(comp_id),
                layer: Some(layer.id),
                comp_time: t,
                current_depth: 0,
                inputs: None,
            };
            let stack = crate::fx::resolved::resolve_stack(
                &layer.effects,
                t,
                1000.0,
                1.0,
                &MarkerContext::NONE,
                Arc::new(context),
            );
            let params = stack.get(1).expect("the bake resolves").params;
            (
                params.choice(BakePoints::DERIVED_BAKE, u32::MAX),
                params.int(BakePoints::DERIVED_FRAME, i32::MAX),
            )
        };
        let held = baked(effect).unwrap();
        let [a, b, c, d, ..] = held.content;
        let head = u32::from_le_bytes([a, b, c, d]);
        assert_eq!(derived(&base, 2.0 / 24.0), (head, 1));
        assert_eq!(derived(&bypass, 2.0 / 24.0), (0, -1));
        assert_eq!(derived(&read, 9.0), (0, -2));

        retain(|id| *id != effect);
        assert!(baked(effect).is_none());
        assert!(matches!(at(&read, 2.0 / 24.0), Reading::Nothing));
    }
}
