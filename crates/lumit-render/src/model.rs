//! Model layers on the way to the mesh pass: each file's scene, read once and
//! kept, and the draw that carries it.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use lumit_core::model::{Document, ProjectItem, ShadowMode};
use lumit_core::model_layer::{ModelSettings, UpAxis};
use lumit_scene::{Bounds, ModelScene};
use uuid::Uuid;

/// One Model layer, as the mesh pass draws it.
#[derive(Debug, Clone)]
pub struct ModelDraw {
    pub item: Uuid,
    pub scene: Arc<ModelScene>,
    /// The layer's own pixels to comp pixels, parents and all, as every 3D
    /// layer is placed. Column-major.
    pub placement: [[f32; 4]; 4],
    /// Model space (metres, Y up) to the layer's pixels: the half turn about X
    /// that takes Y up to the comp's Y down, the Up axis and the Units, or the
    /// fit. Applied before `placement`. Column-major.
    pub model_to_comp: [[f32; 4]; 4],
    pub opacity: f32,
    pub casts_shadows: ShadowMode,
    pub accepts_shadows: ShadowMode,
    pub accepts_lights: bool,
}

/// The most bytes kept scenes hold between them, in memory.
const KEPT_BYTES: usize = 2 << 30;

/// A file by its path and the stamp it had when it was read, so a model saved
/// over is read again.
type Key = (PathBuf, u64, Option<SystemTime>);

/// A read that failed is kept too, so a broken file isn't read every frame.
struct Kept(Option<Arc<ModelScene>>);

impl lumit_cache::ByteSized for Kept {
    fn byte_size(&self) -> usize {
        self.0.as_ref().map_or(0, |s| s.byte_size())
    }
}

static KEPT: Mutex<Option<lumit_cache::ByteLru<Key, Kept>>> = Mutex::new(None);

/// Count kept scenes against the governor's `ledger` from here on.
pub(crate) fn account_against(ledger: &Arc<lumit_budget::Ledger>) {
    if let Ok(mut slot) = KEPT.lock() {
        let mut kept = lumit_cache::ByteLru::new(KEPT_BYTES);
        kept.account_against(Arc::clone(ledger), lumit_budget::Tier::Ram);
        *slot = Some(kept);
    }
}

/// The scene of model item `item`, read the first time it's asked for and
/// kept after. `None` when the file is missing or won't open.
pub(crate) fn scene(doc: &Document, item: Uuid) -> Option<Arc<ModelScene>> {
    let Some(ProjectItem::Footage(f)) = doc.item(item) else {
        return None;
    };
    let path = PathBuf::from(f.media.on_disk());
    let meta = std::fs::metadata(&path).ok()?;
    let key = (path, meta.len(), meta.modified().ok());
    let mut slot = KEPT.lock().ok()?;
    let kept = slot.get_or_insert_with(|| lumit_cache::ByteLru::new(KEPT_BYTES));
    if let Some(Kept(scene)) = kept.get(&key) {
        return scene.clone();
    }
    if kept
        .pressure()
        .is_some_and(lumit_budget::Pressure::should_trim)
    {
        kept.evict_cold_half();
    }
    // ponytail: read on the render thread, with the lock held, so the first
    // frame with a new model waits for it. A background read with a stand-in
    // frame is the upgrade if big scenes make that hurt.
    let scene = lumit_scene::read(
        &key.0,
        lumit_ingress::Limits::MODEL,
        &AtomicBool::new(false),
    )
    .ok()
    .map(Arc::new);
    kept.insert(key, Kept(scene.clone()));
    scene
}

/// Model space to the layer's pixels, for a model whose box at rest is
/// `bounds`, in a comp `comp` pixels in size. Column-major.
pub fn model_to_comp(settings: &ModelSettings, bounds: &Bounds, comp: (f32, f32)) -> [[f32; 4]; 4] {
    use glam::{Mat4, Vec3};
    // The comp is Y down, the model Y up: half a turn about X turns it over
    // and keeps it facing the camera.
    let flip = Mat4::from_rotation_x(std::f32::consts::PI);
    let (lo, hi) = (Vec3::from(bounds.min), Vec3::from(bounds.max));
    let (scale, centre) = match fitted(settings.up_axis, bounds, comp).filter(|_| settings.fit) {
        Some(scale) => (scale, (lo + hi) / 2.0),
        None => (settings.units as f32, Vec3::ZERO),
    };
    (flip
        * Mat4::from_scale(Vec3::splat(scale))
        * up_turn(settings.up_axis)
        * Mat4::from_translation(-centre))
    .to_cols_array_2d()
}

/// A Z-up file turned so its Z is our Y.
fn up_turn(up: UpAxis) -> glam::Mat4 {
    match up {
        UpAxis::Y => glam::Mat4::IDENTITY,
        UpAxis::Z => glam::Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2),
    }
}

/// The comp pixels a metre Fit to comp gives a model whose box is `bounds`:
/// as the default camera sees it, so the near face fits too, with a little
/// room round it. `None` for a box with no width or height.
fn fitted(up: UpAxis, bounds: &Bounds, comp: (f32, f32)) -> Option<f32> {
    let size = up_turn(up)
        .transform_vector3(glam::Vec3::from(bounds.max) - glam::Vec3::from(bounds.min))
        .abs();
    let z = lumit_core::camera::default_zoom(f64::from(comp.0)) as f32;
    let fit = |c: f32, across: f32| c * z / (across * z + c * size.z / 2.0);
    (size.x > 0.0 && size.y > 0.0).then(|| fit(comp.0, size.x).min(fit(comp.1, size.y)) * 0.9)
}
