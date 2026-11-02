//! Model layers on the way to the mesh pass: each file's scene, read once and
//! kept, and the draw that carries it.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use lumit_core::expression::ExpressionContext;
use lumit_core::model::{Composition, Document, Layer, ProjectItem, ShadowMode};
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
    /// `placement` at each shutter sample when the layer blurs, empty when
    /// it doesn't.
    pub shutter: Vec<[[f32; 4]; 4]>,
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

/// The files being read beside the frame.
static READING: Mutex<Option<HashSet<Key>>> = Mutex::new(None);
/// Moves when a read beside the frame starts or lands.
static READS: AtomicU64 = AtomicU64::new(0);
/// Moves each time a frame leaves out a model that is still being read.
static LEFT_OUT: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// On for the Viewer's thread: a model not read yet is read beside the
    /// frame and left out of it until it lands. Export never sets it, so it
    /// waits for every model.
    static ASIDE: Cell<bool> = const { Cell::new(false) };
    /// What stops a read on this thread part way: an export's cancel.
    static STOP: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
}

/// Stop any model this thread reads from now on when `flag` is raised.
pub(crate) fn stop_reads_with(flag: Arc<AtomicBool>) {
    STOP.with(|s| *s.borrow_mut() = Some(flag));
}

/// Read models beside the frame on this thread from now on, or not.
pub fn read_aside(on: bool) {
    ASIDE.with(|a| a.set(on));
}

/// A number that moves when a read beside the frame starts or lands.
pub fn reads() -> u64 {
    READS.load(Ordering::Relaxed)
}

/// How many frames have left out a model still being read.
pub fn left_out() -> u64 {
    LEFT_OUT.load(Ordering::Relaxed)
}

/// Whether a model is being read beside the frame right now.
pub fn reading() -> bool {
    READING
        .lock()
        .is_ok_and(|r| r.as_ref().is_some_and(|r| !r.is_empty()))
}

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
    let key = key_of(doc, item)?;
    if let Some(scene) = kept(&key) {
        return scene;
    }
    if ASIDE.with(Cell::get) {
        LEFT_OUT.fetch_add(1, Ordering::Relaxed);
        read_beside(key);
        return None;
    }
    read_and_keep(key)
}

/// Model layer `layer` as the mesh pass draws it at comp time `t_comp`,
/// `None` while its file is missing, won't open or is still being read.
pub(crate) fn draw(
    doc: &Document,
    comp: &Composition,
    layer: &Layer,
    item: Uuid,
    settings: &ModelSettings,
    t_comp: f64,
    context: &Arc<ExpressionContext>,
) -> Option<ModelDraw> {
    let scene = scene(doc, item)?;
    let material = &layer.transform.material;
    let lt = lumit_core::time::layer_time(t_comp, layer.start_offset.0);
    let placed = |t: f64| {
        // Each shutter sample reads `time` at its own moment.
        let context = Arc::new(ExpressionContext {
            comp_time: t,
            ..(**context).clone()
        });
        crate::placement::world_placement(comp, layer, t, context)
    };
    let dt = 1.0 / comp.frame_rate.fps().max(1.0);
    let offsets = match layer.switches.motion_blur {
        true => comp.motion_blur.sample_offsets(),
        false => Vec::new(),
    };
    let placement = crate::placement::world_placement(comp, layer, t_comp, Arc::clone(context));
    let mut shutter: Vec<_> = offsets
        .iter()
        .map(|off| placed(t_comp + off * dt))
        .collect();
    // A model that stays put over the shutter is drawn once.
    if shutter.iter().all(|s| *s == placement) {
        shutter.clear();
    }
    Some(ModelDraw {
        item,
        placement,
        model_to_comp: model_to_comp(
            settings,
            &scene.bounds,
            (comp.width as f32, comp.height as f32),
        ),
        shutter,
        scene,
        opacity: layer
            .transform
            .opacity
            .value_at_with_context(lt, Arc::clone(context)) as f32,
        casts_shadows: material.casts_shadows,
        accepts_shadows: material.accepts_shadows,
        accepts_lights: layer.switches.accepts_lights,
    })
}

/// The key model item `item`'s file is kept under, as it is on disk now.
fn key_of(doc: &Document, item: Uuid) -> Option<Key> {
    let Some(ProjectItem::Footage(f)) = doc.item(item) else {
        return None;
    };
    let path = PathBuf::from(f.media.on_disk());
    let (len, at) = lumit_media::sequence::MediaSource::file(&path)
        .stamped()
        .stamp?;
    Some((path, len, Some(at)))
}

/// What reading `key` came to, `None` when it hasn't been read.
fn kept(key: &Key) -> Option<Option<Arc<ModelScene>>> {
    let mut slot = KEPT.lock().ok()?;
    let kept = slot.get_or_insert_with(|| lumit_cache::ByteLru::new(KEPT_BYTES));
    kept.get(key).map(|Kept(scene)| scene.clone())
}

/// The eight corners of the box model item `item` fills in its layer's
/// pixels, as `settings` place it in a comp `comp` pixels in size, numbered
/// by the bits of x, y and z. `None` until its file has been read, which this
/// never does, so the Viewer can ask while it draws.
pub fn layer_box(
    doc: &Document,
    item: Uuid,
    settings: &ModelSettings,
    comp: (f32, f32),
) -> Option<[[f32; 3]; 8]> {
    let bounds = kept(&key_of(doc, item)?)??.bounds;
    let m = glam::Mat4::from_cols_array_2d(&model_to_comp(settings, &bounds, comp));
    Some(std::array::from_fn(|i| {
        let corner = std::array::from_fn(|k| match i >> k & 1 {
            0 => bounds.min[k],
            _ => bounds.max[k],
        });
        m.transform_point3(glam::Vec3::from(corner)).to_array()
    }))
}

/// Read the file `key` names and keep what comes of it.
fn read_and_keep(key: Key) -> Option<Arc<ModelScene>> {
    let stop = STOP.with(|s| s.borrow().clone()).unwrap_or_default();
    let read = lumit_scene::read(&key.0, lumit_ingress::Limits::MODEL, &stop);
    // A read stopped part way says nothing about the file, so it isn't kept.
    if matches!(read, Err(lumit_scene::SceneError::Cancelled)) {
        return None;
    }
    let scene = read.ok().map(Arc::new);
    let mut slot = KEPT.lock().ok()?;
    let kept = slot.get_or_insert_with(|| lumit_cache::ByteLru::new(KEPT_BYTES));
    if kept
        .pressure()
        .is_some_and(lumit_budget::Pressure::should_trim)
    {
        kept.evict_cold_half();
    }
    kept.insert(key, Kept(scene.clone()));
    scene
}

/// Read `key` on its own thread, once however many frames ask.
fn read_beside(key: Key) {
    let started = READING
        .lock()
        .is_ok_and(|mut r| r.get_or_insert_with(HashSet::new).insert(key.clone()));
    if !started {
        return;
    }
    READS.fetch_add(1, Ordering::Relaxed);
    let done = move |key: &Key| {
        if let Ok(mut r) = READING.lock() {
            r.get_or_insert_with(HashSet::new).remove(key);
        }
        READS.fetch_add(1, Ordering::Relaxed);
    };
    let spawned = std::thread::Builder::new()
        .name("lumit-model-read".into())
        .spawn({
            let key = key.clone();
            move || {
                read_and_keep(key.clone());
                done(&key);
            }
        });
    if spawned.is_err() {
        done(&key);
    }
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
