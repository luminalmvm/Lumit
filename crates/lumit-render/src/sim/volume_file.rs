//! An OpenVDB file on the card, for the smoke marcher: each frame read once, kept
//! in memory while there is room, and the last few on the card.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use lumit_budget::{Reservation, Tier};
use lumit_core::model::{Document, Layer, LayerKind, ProjectItem};
use lumit_gpu::GpuContext;
use lumit_scene::vdb::DenseVolume;

use super::with_store;

/// The most bytes frames hold between them in memory.
const KEPT_BYTES: usize = 1 << 30;

/// How many frames stay on the card.
const ON_CARD: usize = 4;

struct Frame(Arc<DenseVolume>);

impl lumit_cache::ByteSized for Frame {
    fn byte_size(&self) -> usize {
        self.0.byte_size()
    }
}

static KEPT: Mutex<Option<lumit_cache::ByteLru<PathBuf, Frame>>> = Mutex::new(None);
/// A file's frames, and what its heat is multiplied by.
type FrameList = Arc<(Vec<PathBuf>, f32)>;

/// Each file's list, made once.
static LISTS: Mutex<Option<HashMap<PathBuf, FrameList>>> = Mutex::new(None);

/// A frame's field on the card, and what the ledger has granted for it.
pub(super) struct OnCard {
    file: PathBuf,
    field: wgpu::Texture,
    bounds: lumit_scene::Bounds,
    _held: Option<Reservation>,
}

/// The file `layer` draws through the marcher, if it is a volume.
pub(crate) fn path_of(doc: &Document, layer: &Layer) -> Option<PathBuf> {
    let LayerKind::Model { item, .. } = &layer.kind else {
        return None;
    };
    let Some(ProjectItem::Footage(f)) = doc.item(*item) else {
        return None;
    };
    lumit_scene::is_volume(f.media.on_disk()).then(|| PathBuf::from(f.media.on_disk()))
}

fn frame_file(path: &Path, frame: i64) -> Option<(PathBuf, f32)> {
    let mut lists = LISTS.lock().ok()?;
    let lists = lists.get_or_insert_with(HashMap::new);
    let held = lists.entry(path.to_path_buf()).or_insert_with(|| {
        let files = lumit_scene::vdb::frames(path);
        let gain = lumit_scene::vdb::heat_gain(&files);
        Arc::new((files, gain))
    });
    let last = held.0.len().checked_sub(1)?;
    let at = usize::try_from(frame.max(0)).unwrap_or(0).min(last);
    Some((held.0.get(at)?.clone(), held.1))
}

fn read(ctx: &GpuContext, file: &Path, gain: f32) -> Option<Arc<DenseVolume>> {
    if let Some(found) = KEPT
        .lock()
        .ok()?
        .as_mut()
        .and_then(|k| k.get(&file.to_path_buf()).map(|f| Arc::clone(&f.0)))
    {
        return Some(found);
    }
    let read = Arc::new(lumit_scene::vdb::read_dense(file, gain).ok()?);
    let mut kept = KEPT.lock().ok()?;
    kept.get_or_insert_with(|| {
        let mut kept = lumit_cache::ByteLru::new(KEPT_BYTES);
        kept.account_against(Arc::clone(ctx.ledger()), Tier::Ram);
        kept
    })
    .insert(file.to_path_buf(), Frame(Arc::clone(&read)));
    Some(read)
}

/// Frame `frame` of the volume at `path` on the card: its field and its box in
/// model space. `None` when it can't be read, is empty, or doesn't fit.
pub(super) fn field(
    ctx: &GpuContext,
    path: &Path,
    frame: i64,
) -> Option<Result<(wgpu::Texture, lumit_scene::Bounds), ()>> {
    let (file, gain) = frame_file(path, frame)?;
    let held = with_store(ctx, |s| {
        s.files
            .iter()
            .find(|c| c.file == file)
            .map(|c| (c.field.clone(), c.bounds))
    })?;
    if let Some(found) = held {
        return Some(Ok(found));
    }
    let dense = read(ctx, &file, gain)?;
    if dense.bounds.min == dense.bounds.max {
        return None;
    }
    let bytes = u64::from(dense.pitch) * 4 * u64::from(dense.cells[1]) * u64::from(dense.cells[2]);
    let Ok(reservation) = ctx.ledger().reserve(Tier::Vram, bytes) else {
        return Some(Err(()));
    };
    let texture = lumit_gpu::fx::smoke::field_from(ctx, dense.cells, dense.pitch, &dense.values);
    let bounds = dense.bounds;
    with_store(ctx, |s| {
        s.files.insert(
            0,
            OnCard {
                file,
                field: texture.clone(),
                bounds,
                _held: Some(reservation),
            },
        );
        s.files.truncate(ON_CARD);
    })?;
    Some(Ok((texture, bounds)))
}
