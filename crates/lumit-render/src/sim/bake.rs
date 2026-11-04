//! A simulation baked to disk: the state every [`KEEP_EVERY`](super::KEEP_EVERY)
//! frames, in files named by the walk, in the folder the project caches its
//! frames in, so a heavy sim opens without simulating. A baked state is only
//! ever a shortcut, like a kept one.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use lumit_core::fx::points::stepped::sim::{physics, Ledger};
use lumit_core::model::Document;
use uuid::Uuid;

use super::{colliders, frame_on, hand_over, start_from, walk_name, Live, Source, KEEP_EVERY};
use crate::track::{AnalysisError, Progress, Requested};

/// The folder under a cache root that bakes live in.
const FOLDER: &str = "sim";

/// The first bytes of a baked state, and how its header is laid out.
const MAGIC: &[u8; 8] = b"LUMSIM01";
const HEAD: usize = 8 + 8 + 8 + 8 + 8;

/// The most a baked state says it unpacks to: a million particles.
const MOST: usize = 16 + 48 * 1_000_000;

/// The most one bake writes, in bytes. Past it the bake stops where it got to.
const MOST_ON_DISK: u64 = 64 << 30;

/// The open project's cache folder, where its bakes are.
static ROOT: Mutex<Option<PathBuf>> = Mutex::new(None);

/// How many states have been read back from a bake.
#[cfg(feature = "test-fixtures")]
pub static READS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Bakes go in and come from `cache_root` from now on, the folder the project's
/// frames are cached in by its own settings.
pub fn set_root(cache_root: Option<PathBuf>) {
    if let Ok(mut root) = ROOT.lock() {
        *root = cache_root.map(|r| r.join(FOLDER));
    }
}

fn root() -> Option<PathBuf> {
    ROOT.lock()
        .ok()
        .and_then(|r| r.clone())
        .or_else(|| Some(lumit_project::cache_dir()?.join(FOLDER)))
}

fn hex(name: &[u8; 32]) -> String {
    name.iter().map(|b| format!("{b:02x}")).collect()
}

fn walk_dir(root: &Path, effect: Uuid, name: &[u8; 32]) -> PathBuf {
    root.join(effect.to_string()).join(hex(name))
}

/// The frames baked for this walk, and their folder.
fn baked_frames(effect: Uuid, name: &[u8; 32]) -> Option<(PathBuf, Vec<i64>)> {
    let dir = walk_dir(&root()?, effect, name);
    let listed = lumit_ingress::files::list(&dir).ok()?;
    let mut frames: Vec<i64> = listed
        .iter()
        .filter_map(|(file, _)| file.strip_suffix(".sim")?.parse().ok())
        .collect();
    frames.sort_unstable();
    (!frames.is_empty()).then_some((dir, frames))
}

/// A state read back from disk.
pub(super) struct Baked {
    pub ledger: Ledger,
    pub bytes: Vec<u8>,
}

fn pack(ledger: &Ledger, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEAD + bytes.len() / 2);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&ledger.frame.to_le_bytes());
    out.extend_from_slice(&ledger.carry.to_le_bytes());
    out.extend_from_slice(&ledger.births.to_le_bytes());
    out.extend_from_slice(&u64::from(ledger.high).to_le_bytes());
    out.extend_from_slice(&lz4_flex::compress_prepend_size(bytes));
    out
}

/// The inverse. The size the file announces is checked before anything is
/// allocated for it, since the file's number is not this program's.
fn unpack(file: &[u8]) -> Option<Baked> {
    let word = |at: usize| -> Option<[u8; 8]> { file.get(at..at + 8)?.try_into().ok() };
    if file.get(..8)? != MAGIC {
        return None;
    }
    let body = file.get(HEAD..)?;
    let announced = u32::from_le_bytes(body.get(..4)?.try_into().ok()?);
    if announced as usize > MOST {
        return None;
    }
    Some(Baked {
        ledger: Ledger {
            frame: i64::from_le_bytes(word(8)?),
            carry: f64::from_le_bytes(word(16)?),
            births: u64::from_le_bytes(word(24)?),
            high: u64::from_le_bytes(word(32)?) != 0,
        },
        bytes: lz4_flex::decompress_size_prepended(body).ok()?,
    })
}

/// The furthest state baked of `name` past `after` and up to `frame`.
pub(super) fn nearest(
    effect: Uuid,
    name: &[u8; 32],
    frame: i64,
    after: Option<i64>,
) -> Option<Baked> {
    let (dir, frames) = baked_frames(effect, name)?;
    let at = frames
        .iter()
        .rev()
        .find(|f| **f <= frame && after.is_none_or(|a| **f > a))?;
    let read = unpack(&lumit_ingress::files::read(&dir.join(format!("{at}.sim"))).ok()?);
    #[cfg(feature = "test-fixtures")]
    READS.fetch_add(1, Ordering::Relaxed);
    read
}

/// The frames of the walk a layer shows from its in point to its out point, as
/// `first..end`. Under a Retime that is the furthest it reads each way.
fn frames_shown(src: &Source<'_>) -> (i64, i64) {
    let rate = src.comp.frame_rate;
    let (from, to) = (
        rate.frame_at(src.layer.in_point),
        rate.frame_at(src.layer.out_point),
    );
    let shown = |f: i64| {
        let tc = rate.time_of_frame(f).map_or(0.0, |t| t.0.to_f64());
        let lt = tc - src.layer.start_offset.0.to_f64();
        src.layer.source_time_at(lt) / src.dt
    };
    let (mut first, mut end) = (f64::INFINITY, f64::NEG_INFINITY);
    let span = to.saturating_sub(from).max(0);
    // Without a Retime the two ends say it all.
    let every = if src.layer.retime.is_some() {
        1
    } else {
        span.max(1)
    };
    let mut f = from;
    while f <= to {
        let at = shown(f);
        first = first.min(at);
        end = end.max(at);
        f += every;
    }
    if !(first.is_finite() && end.is_finite()) {
        return (0, 0);
    }
    (first.floor().max(0.0) as i64, (end.ceil() as i64).max(0))
}

/// What `effect` on `layer` is baked to, as frames from its in point and the
/// frames its span holds. `None` when nothing matches its rows as they stand.
#[must_use]
pub fn status(doc: &Arc<Document>, comp: Uuid, layer: Uuid, effect: Uuid) -> Option<(u32, u32)> {
    let c = doc.comp(comp)?;
    let l = c.layers.iter().find(|l| l.id == layer)?;
    let e = l.effects.iter().find(|e| e.id == effect)?;
    let carriage =
        crate::trails::carriage(doc, c, l, e, lumit_core::fx::drivers::NO_CLONE, [0.0; 2]);
    let (_, frames) = baked_frames(effect, &walk_name(&carriage))?;
    let src = Source::find(doc, comp, layer, effect, lumit_core::fx::drivers::NO_CLONE)?;
    let (first, end) = frames_shown(&src);
    // A state every so often covers the frames up to the next.
    let reached = (*frames.last()? + KEEP_EVERY).min(end);
    Some((
        u32::try_from((reached - first).max(0)).unwrap_or(u32::MAX),
        u32::try_from((end - first).max(0)).unwrap_or(u32::MAX),
    ))
}

/// Bake on `effect`: its simulation if it has one, otherwise a Bake points bake.
pub fn press(doc: Arc<Document>, comp: Uuid, layer: Uuid, effect: Uuid) -> Requested {
    let simulates = doc.comp(comp).is_some_and(|c| {
        c.layers
            .iter()
            .filter(|l| l.id == layer)
            .flat_map(|l| &l.effects)
            .any(|e| e.id == effect && lumit_core::fx::points::stepped::sim::wanted(e))
    });
    if !simulates {
        return crate::track::request_bake(doc, comp, layer, effect);
    }
    crate::track::start(effect, move |cancel| {
        let step = |step| crate::track::report(effect, step);
        let outcome = match walk(&doc, comp, layer, effect, cancel, &step) {
            Ok(()) => Progress::Done,
            Err(AnalysisError::Cancelled) => Progress::Cancelled,
            Err(e) => Progress::Failed(e),
        };
        crate::track::finish(effect, Some(outcome));
    })
}

/// Working buffers for a walk nothing else will share.
pub(super) fn fresh(
    fx: &lumit_gpu::fx::FxEngine,
    ctx: &lumit_gpu::GpuContext,
    name: [u8; 32],
    cap: u32,
) -> Live {
    Live {
        name,
        work: Arc::new(fx.sim_work(ctx, cap)),
        ledger: Ledger {
            frame: i64::MAX,
            ..Ledger::default()
        },
        between: None,
        colliders: colliders::Models::default(),
        _held: None,
    }
}

/// The whole of one bake: the frames the layer shows, walked from the start of
/// the walk, with a state written every so often.
fn walk(
    doc: &Arc<Document>,
    comp: Uuid,
    layer: Uuid,
    effect: Uuid,
    cancel: &AtomicBool,
    report: &dyn Fn(Progress),
) -> Result<(), AnalysisError> {
    let src = Source::find(doc, comp, layer, effect, lumit_core::fx::drivers::NO_CLONE)
        .ok_or(AnalysisError::Unreadable)?;
    let (first, end) = frames_shown(&src);
    if end <= first {
        return Err(AnalysisError::NoFrames);
    }
    let total = (end - first) as usize;
    report(Progress::Tracking { done: 0, total });
    let ctx = lumit_gpu::GpuContext::headless().map_err(|_| AnalysisError::Unreadable)?;
    let fx = lumit_gpu::fx::FxEngine::new(&ctx);
    let carriage = crate::trails::carriage(
        doc,
        src.comp,
        src.layer,
        src.inst,
        lumit_core::fx::drivers::NO_CLONE,
        [0.0; 2],
    );
    let name = walk_name(&carriage);
    let root = root().ok_or(AnalysisError::Unreadable)?;
    // One effect has one bake, so the last one goes.
    let _ = lumit_ingress::files::remove_dir_all(&root.join(effect.to_string()));
    let dir = walk_dir(&root, effect, &name);
    lumit_ingress::files::create_dir_all(&dir).map_err(|_| AnalysisError::Unreadable)?;

    let start = src.inputs(0.0).ok_or(AnalysisError::Unreadable)?;
    let cap = start.rows.points().cap;
    let mut live = fresh(&fx, &ctx, name, cap);
    start_from(&fx, &ctx, &src, &name, &mut live, 0, 0).ok_or(AnalysisError::Unreadable)?;
    let mut now = start;
    let mut written = 0u64;
    let mut work = 0u64;
    // A bake stopped part of the way is the frames it got to.
    while live.ledger.frame < end {
        if cancel.load(Ordering::Relaxed) {
            return Err(AnalysisError::Cancelled);
        }
        report(Progress::Tracking {
            done: (live.ledger.frame - first).max(0) as usize,
            total,
        });
        now = frame_on(&fx, &ctx, &src, &mut live, &now).ok_or(AnalysisError::Unreadable)?;
        hand_over(
            &ctx,
            &mut work,
            u64::from(cap) * u64::from(physics(&now.rows).steps),
        );
        // Nothing before the in point but the one state the first frame walks from.
        let frame = live.ledger.frame;
        if frame % KEEP_EVERY == 0 && frame + KEEP_EVERY > first {
            let bytes = fx
                .sim_read(&ctx, &live.work)
                .ok_or(AnalysisError::Unreadable)?;
            let packed = pack(&live.ledger, &bytes);
            let file = dir.join(format!("{frame}.sim"));
            lumit_ingress::files::write(&file, &packed).map_err(|_| AnalysisError::Unreadable)?;
            written += packed.len() as u64;
            if written > MOST_ON_DISK {
                break;
            }
        }
    }
    report(Progress::Tracking { done: total, total });
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// What a bake writes reads back whole, and a file saying more than a bake
    /// can hold is no bake.
    #[test]
    fn a_baked_state_reads_back() {
        let ledger = Ledger {
            frame: 16,
            carry: 0.25,
            births: 4096,
            high: true,
        };
        let bytes: Vec<u8> = (0..4096u32).flat_map(|v| v.to_le_bytes()).collect();
        let file = pack(&ledger, &bytes);
        let back = unpack(&file).unwrap();
        assert_eq!(back.ledger, ledger);
        assert_eq!(back.bytes, bytes);
        let mut lying = file;
        lying[HEAD..HEAD + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(unpack(&lying).is_none());
    }
}
