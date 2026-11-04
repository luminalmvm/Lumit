//! Model layers on the way to the mesh pass: each file's scene, read once and
//! kept, and the draw that carries it.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use lumit_core::expression::ExpressionContext;
use lumit_core::model::{Composition, Document, Layer, LayerKind, ProjectItem, ShadowMode};
use lumit_core::model_layer::{ModelSettings, UpAxis};
use lumit_gpu::mesh::display::ModelDisplay;
use lumit_scene::{Bounds, ModelScene};
use uuid::Uuid;

/// One Model layer, as the mesh pass draws it.
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
    /// Where its nodes are, one for each of `shutter` or one for the frame.
    /// Empty when it draws as its file has it.
    pub poses: Vec<lumit_scene::pose::Pose>,
    pub opacity: f32,
    pub casts_shadows: ShadowMode,
    pub accepts_shadows: ShadowMode,
    pub accepts_lights: bool,
    pub holdout: bool,
    /// The materials its rows change, and when one of them is glass, what
    /// it bends, laid there each time its run of 3D layers is drawn.
    pub(crate) looks: Vec<crate::model_material::Look>,
    pub(crate) behind: Mutex<Behind>,
}

impl ModelDraw {
    /// Whether it cuts through to what is under its run. A view showing the
    /// models themselves draws a holdout like any other.
    pub(crate) fn holds_out(&self) -> bool {
        self.holdout && display().takes_shadows()
    }
}

/// What a model's glass bends.
pub(crate) enum Behind {
    Unset,
    /// Left out of its run while the picture it bends is made.
    LeftOut,
    /// The comp under its run with the run drawn over it, glass left out.
    Bends(wgpu::Texture),
}

/// The most bytes kept scenes hold between them, in memory.
const KEPT_BYTES: usize = 2 << 30;

/// A file by its path, the stamp it had when it was read and the Blender
/// choices made by then, so a model saved over is read again, and so is one
/// that wanted a Blender once one is picked.
type Key = (PathBuf, u64, Option<SystemTime>, u64);

/// A read that failed is kept too, so a broken file isn't read every frame.
struct Kept(Option<Arc<ModelScene>>);

impl lumit_cache::ByteSized for Kept {
    fn byte_size(&self) -> usize {
        self.0.as_ref().map_or(0, |s| s.byte_size())
    }
}

static KEPT: Mutex<Option<lumit_cache::ByteLru<Key, Kept>>> = Mutex::new(None);

/// Each file's missing pictures, kept after its scene goes so its badge
/// never reads it again.
static MISSING: Mutex<Option<HashMap<Key, Vec<String>>>> = Mutex::new(None);

/// The last scene read from each file, while it lives, drawn in its place
/// while the file saved over is read again.
static LAST: Mutex<Option<HashMap<PathBuf, std::sync::Weak<ModelScene>>>> = Mutex::new(None);

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
    /// How the Viewer's view being drawn on this thread shows models. Export
    /// never sets it, so it draws them as they are.
    static DISPLAY: Cell<ModelDisplay> = const {
        Cell::new(ModelDisplay {
            shading: lumit_gpu::mesh::display::Shading::Final,
            faces: false,
        })
    };
}

/// Show models on this thread `display`'s way from now on.
pub fn show_models(display: ModelDisplay) {
    DISPLAY.with(|d| d.set(display));
}

/// How models are shown on this thread.
pub(crate) fn display() -> ModelDisplay {
    DISPLAY.with(Cell::get)
}

/// Frame name `base` as this thread shows models: the same name for the look
/// they export with, another for any other.
pub(crate) fn named_as_shown(base: u128) -> u128 {
    let code = display().code();
    if code == 0 {
        return base;
    }
    let mut h = blake3::Hasher::new();
    h.update(b"models/");
    h.update(&base.to_le_bytes());
    h.update(&[code]);
    let mut k = [0u8; 16];
    k.copy_from_slice(&h.finalize().as_bytes()[..16]);
    u128::from_le_bytes(k)
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
        let last = last_read(&key);
        read_beside(key);
        return last;
    }
    read_and_keep(key.clone()).or_else(|| last_read(&key))
}

/// Model layer `layer` as the mesh pass draws it at comp time `t_comp`,
/// `None` while its file is missing, won't open or is still being read.
pub(crate) fn draw(
    doc: &Document,
    comp: &Composition,
    layer: &Layer,
    t_comp: f64,
    context: &Arc<ExpressionContext>,
    picture: &dyn Fn(&lumit_core::model::EffectInstance, &str) -> Option<crate::draw::DofInputDraw>,
) -> Option<ModelDraw> {
    let LayerKind::Model { item, settings } = &layer.kind else {
        return None;
    };
    let (item, settings) = (*item, &**settings);
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
    let pose_at = |t| crate::model_motion::pose_at(comp, layer, settings, &scene, t, context);
    let frame_pose = pose_at(t_comp);
    let mut poses: Vec<_> = match frame_pose.is_some() {
        true => (offsets.iter())
            .filter_map(|off| pose_at(t_comp + off * dt))
            .collect(),
        false => Vec::new(),
    };
    // A model that stays put over the shutter is drawn once.
    if shutter.iter().all(|s| *s == placement)
        && poses.iter().all(|p| Some(p) == frame_pose.as_ref())
    {
        shutter.clear();
    }
    if shutter.is_empty() {
        poses = frame_pose.into_iter().collect();
    }
    let looks = crate::model_material::looks(settings, &scene, lt, context, picture);
    Some(ModelDraw {
        item,
        placement,
        model_to_comp: model_to_comp(
            settings,
            &scene.bounds,
            (comp.width as f32, comp.height as f32),
        ),
        shutter,
        poses,
        scene,
        opacity: layer
            .transform
            .opacity
            .value_at_with_context(lt, Arc::clone(context)) as f32,
        casts_shadows: material.casts_shadows,
        accepts_shadows: material.accepts_shadows,
        accepts_lights: layer.switches.accepts_lights,
        holdout: material.holdout,
        looks,
        behind: Mutex::new(Behind::Unset),
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
    Some((path, len, Some(at), lumit_scene::blender::choices()))
}

/// What reading `key` came to, `None` when it hasn't been read. A file saved
/// over with something that won't open keeps its last good scene.
fn kept(key: &Key) -> Option<Option<Arc<ModelScene>>> {
    let scene = {
        let mut slot = KEPT.lock().ok()?;
        let kept = slot.get_or_insert_with(|| lumit_cache::ByteLru::new(KEPT_BYTES));
        kept.get(key)?.0.clone()
    };
    Some(scene.or_else(|| last_read(key)))
}

/// The last scene read from `key`'s file, while it lives.
fn last_read(key: &Key) -> Option<Arc<ModelScene>> {
    LAST.lock().ok()?.as_ref()?.get(&key.0)?.upgrade()
}

/// The scene of model item `item` once its file has been read, `None` inside
/// when it wouldn't open. `None` while it hasn't been, and a read beside
/// starts, so a panel can ask as it draws.
pub fn scene_if_read(doc: &Document, item: Uuid) -> Option<Option<Arc<ModelScene>>> {
    let Some(key) = key_of(doc, item) else {
        return Some(None);
    };
    if let Some(scene) = kept(&key) {
        return Some(scene);
    }
    read_beside(key);
    None
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
    pose: Option<&lumit_scene::pose::Pose>,
) -> Option<[[f32; 3]; 8]> {
    let rest = kept(&key_of(doc, item)?)??.bounds;
    let bounds = pose.map_or(rest, |p| p.bounds);
    // Placed by its box at rest, so a model doesn't drift as it moves.
    let m = glam::Mat4::from_cols_array_2d(&model_to_comp(settings, &rest, comp));
    Some(std::array::from_fn(|i| {
        let corner = std::array::from_fn(|k| match i >> k & 1 {
            0 => bounds.min[k],
            _ => bounds.max[k],
        });
        m.transform_point3(glam::Vec3::from(corner)).to_array()
    }))
}

/// The pictures model item `item` names that are missing or won't read, as
/// it names them. Reads the file if nothing has yet.
pub fn missing(doc: &Document, item: Uuid) -> Vec<String> {
    let known = key_of(doc, item).and_then(|key| MISSING.lock().ok()?.as_ref()?.get(&key).cloned());
    let mut names =
        known.unwrap_or_else(|| scene(doc, item).map_or_else(Vec::new, |s| s.missing.clone()));
    let read = key_of(doc, item).and_then(|key| kept(&key)).flatten();
    if read.is_some_and(|s| lumit_gpu::mesh::deform::left_at_rest(&s)) {
        names.push(TOO_LARGE_TO_MOVE.to_owned());
    }
    names
}

/// What the missing list says of a model too large for this card to move.
pub const TOO_LARGE_TO_MOVE: &str = "Skins and morph targets, too large for this graphics card";

/// Whether model item `item` has been read and holds no takes.
pub fn known_still(doc: &Document, item: Uuid) -> bool {
    key_of(doc, item)
        .and_then(|key| kept(&key))
        .flatten()
        .is_some_and(|scene| scene.clips.is_empty())
}

/// How far along the ray from `origin` by `direction`, in its layer's pixels,
/// model item `item` placed by `settings` in a comp `comp` pixels in size is
/// first met as `pose` has it, in parts of `direction`, and which node's part
/// it met. `None` where it misses and before the file has been read, which
/// this never does.
pub fn hit(
    doc: &Document,
    item: Uuid,
    settings: &ModelSettings,
    comp: (f32, f32),
    pose: Option<&lumit_scene::pose::Pose>,
    origin: [f32; 3],
    direction: [f32; 3],
) -> Option<(f32, usize)> {
    use glam::{Mat4, Vec3};
    let (origin, direction) = (Vec3::from(origin), Vec3::from(direction));
    let scene = kept(&key_of(doc, item)?)??;
    let placed = Mat4::from_cols_array_2d(&model_to_comp(settings, &scene.bounds, comp));
    let mut nearest = None::<(f32, usize)>;
    for (k, node) in scene.nodes.iter().enumerate() {
        let Some(mesh) = node.mesh.and_then(|m| scene.meshes.get(m)) else {
            continue;
        };
        if pose.is_some_and(|p| p.hidden.get(k).copied().unwrap_or(false)) {
            continue;
        }
        // A pose gives vertices in model space, and with none they're in the
        // node's own, so the ray goes there.
        let world = match pose {
            Some(_) => Mat4::IDENTITY,
            None => scene.node_world(k),
        };
        let mover = pose.map(|p| p.mover(&scene, k));
        // The ray in the node's own space, which keeps its parts of direction.
        let back = (placed * world).inverse();
        let (o, d) = (
            back.transform_point3(origin),
            back.transform_vector3(direction),
        );
        // ponytail: a layer mirrored by a negative Scale isn't seen here, so
        // its single-sided parts are culled from the wrong side.
        let whole = pose.map_or(world, |p| p.worlds.get(k).copied().unwrap_or(world));
        let mirrored = (placed * whole).determinant() < 0.0;
        for p in &mesh.primitives {
            // Parts the pass leaves out: cut out or see-through all over.
            let m = p.material.and_then(|m| scene.materials.get(m));
            let gone = m.is_some_and(|m| match m.alpha_mode {
                lumit_scene::AlphaMode::Mask => m.base_colour[3] < m.alpha_cutoff,
                lumit_scene::AlphaMode::Blend => m.base_colour[3] <= 0.0,
                lumit_scene::AlphaMode::Opaque => false,
            });
            if gone {
                continue;
            }
            // glTF's default is one-sided.
            let culled = !mirrored && !m.is_some_and(|m| m.double_sided);
            let corner = |i: u32| match &mover {
                Some(mover) => mover.vertex(p, i as usize),
                None => p.positions.get(i as usize).copied().map(Vec3::from),
            };
            let count = if p.indices.is_empty() {
                p.positions.len()
            } else {
                p.indices.len()
            };
            let index = |i: usize| p.indices.get(i).copied().unwrap_or(i as u32);
            // On a skin the part met is the bone that moves what was met most.
            let bone = |i: u32| {
                let skin = scene.skins.get(node.skin?)?;
                let (joints, shares) = (p.joints.get(i as usize)?, p.weights.get(i as usize)?);
                let most = (0..4).max_by(|&a, &b| shares[a].total_cmp(&shares[b]))?;
                skin.joints.get(usize::from(joints[most])).copied()
            };
            for t in 0..count / 3 {
                let (Some(a), Some(b), Some(c)) = (
                    corner(index(t * 3)),
                    corner(index(t * 3 + 1)),
                    corner(index(t * 3 + 2)),
                ) else {
                    continue;
                };
                // Moller and Trumbore's test.
                let (e1, e2) = (b - a, c - a);
                let q = d.cross(e2);
                let det = e1.dot(q);
                // Seen from behind when the ray runs with the face's normal.
                if !det.is_normal() || (culled && det < 0.0) {
                    continue;
                }
                let s = o - a;
                let u = s.dot(q) / det;
                let r = s.cross(e1);
                let v = d.dot(r) / det;
                let along = e2.dot(r) / det;
                let inside = u >= 0.0 && v >= 0.0 && u + v <= 1.0 && along > 0.0;
                if inside && nearest.is_none_or(|(n, _)| along < n) {
                    nearest = Some((along, bone(index(t * 3)).unwrap_or(k)));
                }
            }
        }
    }
    nearest
}

/// Read the file `key` names and keep what comes of it.
fn read_and_keep(key: Key) -> Option<Arc<ModelScene>> {
    let stop = STOP.with(|s| s.borrow().clone()).unwrap_or_default();
    let cache = lumit_project::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("models");
    let read = lumit_scene::read(&key.0, lumit_ingress::Limits::MODEL, &cache, &stop);
    // A read stopped part way says nothing about the file, so it isn't kept.
    if matches!(read, Err(lumit_scene::SceneError::Cancelled)) {
        return None;
    }
    let scene = read.ok().map(Arc::new);
    if let (Some(s), Ok(mut m)) = (&scene, MISSING.lock()) {
        m.get_or_insert_with(HashMap::new)
            .insert(key.clone(), s.missing.clone());
    }
    let mut slot = KEPT.lock().ok()?;
    let kept = slot.get_or_insert_with(|| lumit_cache::ByteLru::new(KEPT_BYTES));
    if kept
        .pressure()
        .is_some_and(lumit_budget::Pressure::should_trim)
    {
        kept.evict_cold_half();
    }
    if let (Some(s), Ok(mut last)) = (&scene, LAST.lock()) {
        last.get_or_insert_with(HashMap::new)
            .insert(key.0.clone(), Arc::downgrade(s));
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

/// The turn `model_to_comp` gives a model, without its size.
pub(crate) fn up_right(up: UpAxis) -> glam::Quat {
    let flip = glam::Mat4::from_rotation_x(std::f32::consts::PI);
    glam::Quat::from_mat4(&(flip * up_turn(up)))
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

/// What [`fitted`] gives model item `item` turned by `up` in a comp `comp`
/// pixels in size, once its file has been read, which this never does.
pub fn fitted_units(doc: &Document, item: Uuid, up: UpAxis, comp: (f32, f32)) -> Option<f32> {
    fitted(up, &kept(&key_of(doc, item)?)??.bounds, comp)
}
