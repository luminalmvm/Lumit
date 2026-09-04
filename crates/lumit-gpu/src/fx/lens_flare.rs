//! The Lens flare GPU pipeline (docs/08 §3.27, docs/impl/lens-flare.md):
//! per-frame ray-trace compute, splat-build compute, an additive hardware
//! raster of one small quad per ray, the Matte-mode source detection, and
//! the combine kernel. The engine-pure maths and the bake live in
//! `lumit_core::fx::lens_flare`; this module consumes pre-baked data
//! through [`FlareBakeData`] (the caller converts, keeping this crate
//! lumit-core-free in production, exactly as the effect op structs do).
//!
//! In plain terms: every frame, a few hundred thousand tiny ray programs
//! push light through the chosen lens on the graphics card — once per flare
//! source — and each ray that survives dabs its share of the light onto a
//! flare image, spread over the little patch its neighbours say it covers;
//! one last pass lays that (plus a baked starburst sprite per source) over
//! the picture. In Matte mode the sources themselves are found on the card
//! first: the matte layer's brightest points, detected by two small kernels.
//! The slow maths — the Fourier transforms — never runs here; it arrives as
//! textures baked on the CPU and cached by parameter hash.

use std::collections::{HashMap, HashSet};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{channel, Receiver, Sender},
    Arc, Mutex, OnceLock,
};

use crate::{GpuContext, WORKING_FORMAT};

use super::{work_texture, FxEngine};

/// The CPU bake, as something another thread can own and run.
///
/// An `Arc` rather than a borrowed `&dyn Fn` because the bake may be handed to
/// the bake thread and outlive the frame that asked for it. The caller
/// builds one per flare op per frame — a single small allocation beside a
/// pass that traces hundreds of thousands of rays.
pub type FlareBake = Arc<dyn Fn() -> FlareBakeData + Send + Sync>;

/// The resolved Lens flare op in lumit-gpu's own terms: plain numbers plus
/// the per-frame wavelength table, all derived by the caller (lumit-render)
/// from `lumit_core::fx::lens_flare` so the formulas live in one place.
#[derive(Debug, Clone)]
pub struct LensFlareOp {
    /// Manual light position as a fraction of the raster (x right, y down)
    /// — the caller divides its raster-pixel parameter by the raster.
    /// Still the position the frame-grid probe reasons about, and the centre
    /// of [`Self::manual_lights`].
    pub light_frac: [f32; 2],
    /// Manual mode's light list — one entry per light, whatever its size.
    /// Each entry is `[x, y, r, g, b, ext_x, ext_y]`: position and
    /// half-extent as raster fractions, colour in scene-linear RGB. A zero
    /// extent is a point source; a larger one is an AREA source, which the
    /// trace integrates per ray rather than replicating into samples (the
    /// earlier way, and what this layout's two extra floats replaced).
    /// Empty falls back to a single white light at [`Self::light_frac`].
    pub manual_lights: Vec<[f32; 7]>,
    /// Master gain; 0 short-circuits to the identity.
    pub intensity: f32,
    /// Traced wavelength bands with their radiometric sub-samples
    /// (lumit_core `spectral_bands`).
    pub bands: Vec<FlareBand>,
    /// How many ranked ghosts render.
    pub max_ghosts: u32,
    /// 0..1 coating blend.
    pub coating: f32,
    /// Focus distance, metres; the sensor shift derives from it and
    /// the bake's focal length inside the apply.
    pub focus_m: f32,
    /// Working f-stop: the stop-down scale derives from it and the
    /// bake's native f-number inside the apply.
    pub fstop: f32,
    /// Iris blade count for the in-shader pupil mask.
    pub blades: u32,
    /// Iris rotation, degrees.
    pub aperture_rotation_deg: f32,
    /// 0..1 iris roundness (the wide-open blend applies inside the apply).
    pub roundness: f32,
    /// 0..1 iris edge softness.
    pub aperture_softness: f32,
    /// Ghost blur radius in raster pixels (px@comp).
    pub ghost_softness: f32,
    /// Pupil-grid side for this quality.
    pub grid: u32,
    /// Flare-buffer divisor (2 on Draft, else 1).
    pub flare_div: u32,
    /// Raster px per sensor mm at the FULL raster width (lumit_core
    /// `screen_transform(w)`); the flare buffer's own transform scales by
    /// its divisor.
    pub screen_transform: f32,
    /// Starburst gain.
    pub starburst_intensity: f32,
    /// Whole-flare scale about the optical centre (ghosts and starbursts).
    pub scale: f32,
    /// Horizontal squeeze about the frame centre.
    pub anamorphic: f32,
    /// Source mode: 0 Manual, 1 Matte, 2 Lights (resolves as Manual until
    /// light layers land).
    pub source: u32,
    /// Matte mode's soft luma gate (lumit_core `threshold_gate`).
    pub threshold: f32,
    /// See `threshold`.
    pub threshold_softness: f32,
    /// Scene-linear RGB multiplying every light's colour.
    pub light_tint: [f32; 3],
    /// Matte/Lights: whether a detected source's own colour tints its flare.
    pub use_source_colour: bool,
    /// Matte mode: read the matte inverted (`1 − rgb`) when detecting, so its
    /// dark parts are the lights — the uniform matte row's Invert.
    pub matte_invert: bool,
    /// How the flare element combines with the layer under it — an index
    /// into `lumit_core::fx::lens_flare::BLEND_OPTIONS`.
    pub blend: u32,
    /// 0..1.
    pub mix: f32,
    /// `lumit_core::fx::lens_flare::bake_key` of the op — the bake cache key.
    pub bake_key: u64,
}

/// One traced wavelength band with its radiometric sub-samples —
/// restated from `lumit_core::fx::lens_flare::SpectralBand` because this
/// crate does not depend on lumit-core. The geometry is traced once at
/// [`Self::traced_nm`]; the energy is carried at the eight sub-samples,
/// each reading the baked reflectance table at its own wavelength. The
/// caller folds the ghost energy gain and Ghost intensity into
/// [`Self::sub_rgb`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlareBand {
    /// The wavelength the geometry is traced at (dispersion-scaled).
    pub traced_nm: f32,
    /// Reflectance-table lambda index per sub-sample.
    pub sub_idx: [u32; 8],
    /// Linear working RGB weight per sub-sample.
    pub sub_rgb: [[f32; 3]; 8],
}

/// The bake handed across the crate seam: plain buffers, no lumit-core
/// types. Produced by the caller from `lumit_core::fx::lens_flare::bake`
/// only when the cache misses (the `bake` argument of
/// [`FxEngine::lens_flare`] is lazy).
#[derive(Debug, Clone)]
pub struct FlareBakeData {
    /// Surface rows: radius, z, semi_ap, cauchy_a, cauchy_b,
    /// coating_layers, is_stop, pad — the WGSL `Surface` layout.
    pub surfaces: Vec<[f32; 8]>,
    /// Ranked ghost pairs, brightest first.
    pub ghosts: Vec<[u32; 4]>,
    /// Each pair's image spread (fraction of the sensor diagonal), parallel
    /// to `ghosts` — the adaptive grid budget's input.
    pub spreads: Vec<f32>,
    /// Sensor plane z, mm.
    pub sensor_z_mm: f32,
    /// Focal length, mm — the in-shader light direction's z.
    pub focal_mm: f32,
    /// Native f-number (the stop-down and wide-open-roundness reference).
    pub native_fstop: f32,
    /// Pupil spray radius, mm.
    pub pupil_mm: f32,
    /// Ray start z, mm.
    pub start_z_mm: f32,
    /// The bake's auto-exposure gain, multiplied into every ghost's energy.
    pub energy_gain: f32,
    /// The per-surface spectral reflectance table, flat in the
    /// lumit-core layout: `[surface][direction 0 = forward, 1 = reverse]
    /// [lambda 69][cos 16]`. The trace kernel reads it in place of solving a
    /// thin-film stack per ray.
    pub reflectance: Vec<f32>,
    /// Starburst sprite, `sb_fields × sb_res²` RGB triplets, slice-major.
    pub starburst: Vec<f32>,
    /// See `starburst`.
    pub sb_res: u32,
    /// Field-angle slices in `starburst`: the sprite is baked at
    /// several field angles, because the mechanical stops clip the iris
    /// into a cat's-eye off-axis, and the combine blends the two slices
    /// bracketing each light. Uploaded as one atlas `sb_res` wide by
    /// `sb_res × sb_fields` tall; 0 is read as 1 (a single on-axis slice).
    pub sb_fields: u32,
}

/// What the frame-time spread probe reads from a cached bake,
/// handed back across the crate seam to the caller's `probe` closure (the
/// same lazy-callback pattern as the bake itself — the formulas stay in
/// lumit-core, which this crate deliberately does not depend on).
pub struct FlareProbeBake<'a> {
    /// Surface rows in the `FlareBakeData` layout.
    pub surfaces: &'a [[f32; 8]],
    /// Ranked ghost pairs, brightest first.
    pub ghosts: &'a [[u32; 4]],
    /// Sensor plane z, mm.
    pub sensor_z_mm: f32,
    /// Focal length, mm.
    pub focal_mm: f32,
    /// Native f-number.
    pub native_fstop: f32,
    /// Pupil spray radius, mm.
    pub pupil_mm: f32,
    /// Ray start z, mm.
    pub start_z_mm: f32,
    /// Each pair's bake-time image spread, parallel to `ghosts` — the rung
    /// floor the probe's budget plan raises from.
    pub spreads: &'a [f32],
    /// How many ranked pairs this frame renders — the probe stops there.
    pub pair_count: usize,
}

/// One cached GPU-side bake: uploaded textures and the surface buffer.
struct GpuBaked {
    surfaces: wgpu::Buffer,
    /// The baked reflectance table, as the trace kernel's storage
    /// binding.
    reflectance: wgpu::Buffer,
    surface_count: u32,
    /// The raw surface rows, retained for [`FlareProbeBake`] —
    /// a few hundred bytes beside the uploaded buffer.
    surface_rows: Vec<[f32; 8]>,
    ghosts: Vec<[u32; 4]>,
    spreads: Vec<f32>,
    sensor_z_mm: f32,
    focal_mm: f32,
    native_fstop: f32,
    pupil_mm: f32,
    start_z_mm: f32,
    energy_gain: f32,
    starburst: wgpu::Texture,
}

/// The per-frame scratch one flare render works through: the ray landings and
/// the splats the raster pulls from.
///
/// **Why this is pooled rather than allocated per frame.** These are the two
/// big buffers in the effect — tens of megabytes at working qualities — and a
/// frame used to create both, use them for a few milliseconds and drop them.
/// A graphics driver does not hand that memory straight back: it recycles it
/// when the submission it belonged to retires, so a Viewer re-rendering
/// continuously (a drag, or the idle cache fill) kept a rolling backlog of
/// abandoned tens-of-megabyte buffers, and on a unified-memory machine that
/// is how a flare in the composition ends up filling the graphics memory it
/// shares with everything else. Held and reused, the frame allocates nothing.
struct Scratch {
    rays: wgpu::Buffer,
    splats: wgpu::Buffer,
    /// The f32 splat accumulator, three channels a pixel, pooled with
    /// the rest because it is the same shape of per-frame scratch.
    accum: wgpu::Buffer,
    ray_bytes: u64,
    splat_bytes: u64,
    accum_bytes: u64,
}

/// The lens flare's pipelines, its bake cache and its scratch pool, one field
/// on [`FxEngine`].
pub struct LensFlareFx {
    trace: wgpu::ComputePipeline,
    build_splats: wgpu::ComputePipeline,
    detect_tiles: wgpu::ComputePipeline,
    detect_pick: wgpu::ComputePipeline,
    /// The splat deposit and its resolve: the accumulation moved off
    /// the raster blender, which could only sum in the flare buffer's fp16.
    deposit: wgpu::ComputePipeline,
    resolve: wgpu::ComputePipeline,
    blur: wgpu::ComputePipeline,
    combine: wgpu::ComputePipeline,
    trace_layout: wgpu::BindGroupLayout,
    detect_layout: wgpu::BindGroupLayout,
    deposit_layout: wgpu::BindGroupLayout,
    blur_layout: wgpu::BindGroupLayout,
    combine_layout: wgpu::BindGroupLayout,
    /// Baked resources keyed by `bake_key`. The mutex is held only for
    /// get/insert — never across an upload or a submit.
    cache: Mutex<BakeCache<Arc<GpuBaked>>>,
    /// An idle [`Scratch`] waiting to be used again, at most one deep: two
    /// flares rendering at once (two open projects) is possible but not the
    /// case worth holding memory for, so a second render makes its own and
    /// whichever finishes first keeps the slot.
    scratch: Mutex<Option<Scratch>>,
    /// The off-thread bake, when this engine is allowed one. `None`
    /// until the first deferred miss, and never built at all on an engine
    /// whose bakes must be exact — the exporter's (see
    /// [`FxEngine::set_deferred_flare_bakes`]).
    baker: OnceLock<Option<Baker>>,
    /// Whether a miss may render the previous lens while the bake is made
    /// beside the frame. Off by default: an engine that has not been told
    /// otherwise bakes inside the frame exactly as it did before, so a path
    /// that has not opted in — the exporter's — cannot draw a provisional
    /// picture by omission.
    pub(super) deferred: std::sync::atomic::AtomicBool,
    /// The key of the last bake a frame actually drew with, which is what a
    /// frame whose own bake is not ready falls back to.
    last_drawn: Mutex<Option<u64>>,
    /// Bakes handed to the baker and not yet collected.
    in_flight: Mutex<HashSet<u64>>,
    /// Bakes the thread has finished that no frame has uploaded yet. Filled
    /// by [`Self::poll_landed`] — which any thread may call, no device needed
    /// — and drained by [`Self::collect`] on the render thread. This split is
    /// what lets an *idle* worker notice a bake has landed: `bake_pending`
    /// used to read `in_flight`, which only `collect` cleared, and `collect`
    /// only ran inside a frame render — so after the bake finished, the
    /// republish tick saw "still pending" forever and the picture stayed one
    /// lens behind until the user happened to move the playhead.
    landed: Mutex<Vec<(u64, FlareBakeData)>>,
    /// Bumped whenever a bake is queued and again when one lands. A frame
    /// rendered across a change of this number may have drawn a lens other
    /// than the one its parameters name, so the caller must not file it under
    /// a name that says otherwise — see `FxEngine::flare_bake_generation`.
    pub(super) generation: AtomicU64,
    /// Bumped each time a frame actually drew **something other than** the
    /// bake its parameters name — the deferred fallback to the previous lens,
    /// or no flare at all because there is no previous lens yet.
    ///
    /// This is the precise form of the question the generation could only
    /// answer roughly. The generation moves when a bake is *queued*, which
    /// says a bake is being made somewhere, not that this frame drew the
    /// wrong optics — and while a keyframed aperture keeps one queued, that
    /// was every frame of every comp, flare or no flare.
    pub(super) substitutions: AtomicU64,
}

/// The bake thread and its two channels.
struct Baker {
    jobs: Sender<BakeJob>,
    done: Mutex<Receiver<(u64, Option<FlareBakeData>)>>,
}

struct BakeJob {
    key: u64,
    bake: FlareBake,
}

impl Baker {
    /// Start the bake thread, or answer `None` on a machine that will not give
    /// us one — where every miss then bakes inside the frame, which is what it
    /// did before this existed.
    fn start() -> Option<Baker> {
        let (jobs, queue) = channel::<BakeJob>();
        let (finished, done) = channel::<(u64, Option<FlareBakeData>)>();
        std::thread::Builder::new()
            .name("lumit-flare-bake".into())
            .spawn(move || Self::run(&queue, &finished))
            .ok()
            .map(|_| Baker {
                jobs,
                done: Mutex::new(done),
            })
    }

    /// The bake loop. Ends when the sender is dropped — the engine going away.
    ///
    /// **Cancellation is by supersession, and it is exact** (docs/14 §6): a
    /// bake is named by a hash of the parameters that produced it, so a job
    /// whose key is not among those still queued behind it is a lens the user
    /// has already moved past. Dragging the f-stop slider queues a key a tick,
    /// and only the last of them is worth half a second of optics; the rest
    /// are dropped before they start.
    fn run(queue: &Receiver<BakeJob>, finished: &Sender<(u64, Option<FlareBakeData>)>) {
        while let Ok(job) = queue.recv() {
            // Everything else already waiting, so the newest is known before
            // anything is baked.
            let mut pending: Vec<BakeJob> = vec![job];
            while let Ok(next) = queue.try_recv() {
                pending.push(next);
            }
            // Only the last survives, and only if nothing else asked for it
            // in the meantime — the ones before it are keys nobody is looking
            // at any more.
            let Some(wanted) = pending.pop() else {
                continue;
            };
            for dropped in pending {
                // Answered with nothing: the engine takes the key off its
                // in-flight list, so a lens abandoned mid-drag does not leave
                // the frame permanently unnameable. If it turns out to be
                // wanted after all, the next frame that asks re-queues it.
                if finished.send((dropped.key, None)).is_err() {
                    return;
                }
            }
            let data = (wanted.bake)();
            if finished.send((wanted.key, Some(data))).is_err() {
                return; // nobody is collecting any more
            }
        }
    }
}

/// A bounded, oldest-first cache of bakes by parameter hash.
///
/// **Why oldest-first and not clear-the-lot.** Earlier the map simply
/// emptied when it overflowed. A bake is the effect's one slow, blocking,
/// CPU-side step, and the way the lens picker is used is to try lenses — so
/// every ninth pick threw away the eight bakes just paid for, and stepping
/// back to a lens seen a moment ago paid for it all over again. Dropping the
/// oldest entry instead keeps a working set of recent lenses hot. Correctness
/// never depends on the cache (docs/14 §5): a miss is a rebake, nothing more.
pub(super) struct BakeCache<T> {
    by_key: HashMap<u64, T>,
    /// Insertion order, oldest first.
    order: std::collections::VecDeque<u64>,
    cap: usize,
}

impl<T: Clone> BakeCache<T> {
    pub(super) fn new(cap: usize) -> Self {
        Self {
            by_key: HashMap::new(),
            order: std::collections::VecDeque::new(),
            cap: cap.max(1),
        }
    }

    pub(super) fn get(&self, key: u64) -> Option<T> {
        self.by_key.get(&key).cloned()
    }

    /// Remember `value` under `key`, evicting oldest-first to stay at the
    /// cap. Returns whatever the key holds afterwards — an existing entry
    /// wins, so a racing double-build leaves the map single-valued.
    pub(super) fn insert(&mut self, key: u64, value: T) -> T {
        if let Some(held) = self.by_key.get(&key) {
            return held.clone();
        }
        while self.by_key.len() >= self.cap {
            match self.order.pop_front() {
                Some(oldest) => {
                    self.by_key.remove(&oldest);
                }
                // Unreachable while the two stay in step, but a cache may
                // never grow without bound on the strength of an invariant.
                None => self.by_key.clear(),
            }
        }
        self.order.push_back(key);
        self.by_key.insert(key, value.clone());
        value
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.by_key.len()
    }
}

/// How many blend modes the combine kernel's `flare_blend` implements — the
/// length of `lumit_core::fx::lens_flare::BLEND_OPTIONS`, pinned by
/// test. An index past the last option clamps rather than faulting.
pub const BLEND_COUNT: u32 = 13;

/// Distinct sources detection may find, and so the light slots the trace
/// carries — must equal `lumit_core::fx::lens_flare::MAX_SOURCES` (pinned by
/// test). One source is one slot however large it is: an area
/// source is integrated inside the ray loop, not replicated into samples.
pub const MAX_SOURCES: u32 = 16;

/// Detection tile side — must equal `lens_flare::DETECT_TILE` (pinned by
/// the same test).
const DETECT_TILE: u32 = 32;

/// Byte budget for the per-frame trace scratch — rays and splats
/// together. A **hard** cap, not a hint: where the earlier batch size
/// bottomed out at one combo and then let eight lights at an Ultra grid ask
/// for a hundred megabytes anyway, the light dimension now splits too, so no
/// setting can push the allocation past this.
pub(super) const SCRATCH_BYTE_BUDGET: u64 = 48_000_000;

/// Bytes one traced ray occupies (WGSL `Ray`: pos.xy, weight, pad, rgb,
/// pad — the rgb added where the weight alone once carried the energy).
pub(super) const RAY_BYTES: u64 = 32;

/// Bytes one splat occupies (WGSL `Splat`: centre, two half-axes,
/// peak rgb, live, two pads). One per RAY, where the drawn cell it replaces
/// was one per (grid−1)² quad.
pub(super) const SPLAT_BYTES: u64 = 48;

/// Ray–surface steps one command buffer may hold before the frame submits
/// what it has and opens another.
///
/// **Why a frame is split at all.** Every operating system kills a graphics
/// submission that runs too long — macOS and Windows both watch for it — and
/// a killed submission does not merely drop a frame: it takes the device with
/// it, so the Viewer stays frozen for the rest of the session and re-opening
/// the project does not help, because the process's graphics device is the
/// thing that died. The flare's cost is set by parameters the user is free to
/// wind up (Quality, Max ghosts, an eight-source matte), so the frame is
/// broken into submissions small enough that no combination can reach the
/// watchdog. Splitting changes nothing about the picture: the batches are
/// queued in the same order and blend in the same order, they are merely
/// handed over in several pieces.
pub(super) const STEPS_PER_SUBMIT: u64 = 48_000_000;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TraceParams {
    surface_count: u32,
    combo_count: u32,
    grid: u32,
    combo_offset: u32,
    coating: f32,
    aspect: f32,
    focal_mm: f32,
    screen_transform: f32,
    raster_w: f32,
    raster_h: f32,
    light_count: u32,
    sensor_shift_mm: f32,
    pupil_mm: f32,
    start_z_mm: f32,
    sensor_z_mm: f32,
    stop_scale: f32,
    cell_area_px: f32,
    ray_stride: u32,
    /// Padding that keeps the uniform a multiple of 16 bytes; held the
    /// per-slot quad count until per-ray splats replaced quads.
    _pad_stride: u32,
    blades: u32,
    rot_rad: f32,
    roundness: f32,
    softness: f32,
    light_offset: u32,
}

/// The frame-time optics both the production trace and the §8.5 debug hook
/// derive before filling [`TraceParams`] — shared with the CPU reference:
/// the stop-down scale, the wide-open roundness blend, and the thin-lens
/// focus shift (f²/(1000·d − f), exactly as the CPU reference computes
/// it). One place, so the two fills cannot drift.
struct FrameOptics {
    stop_scale: f32,
    wide_open: f32,
    sensor_shift_mm: f32,
}

fn frame_optics(native_fstop: f32, focal_mm: f32, fstop: f32, focus_m: f32) -> FrameOptics {
    let stop_scale = if native_fstop > 0.0 && fstop > 0.0 {
        (native_fstop / fstop).clamp(0.05, 1.0)
    } else {
        1.0
    };
    let native = native_fstop.max(0.7);
    let wide_open = (1.0 - (fstop / native - 1.0).clamp(0.0, 2.0) / 2.0).clamp(0.0, 1.0);
    let f = focal_mm;
    let sensor_shift_mm = if focus_m <= 0.0 {
        0.0
    } else {
        (f * f / (1000.0 * focus_m - f).max(f)).clamp(0.0, f)
    };
    FrameOptics {
        stop_scale,
        wide_open,
        sensor_shift_mm,
    }
}

/// What the deposit and resolve stages need beyond the splats: the flare
/// buffer's size, because a splat's centre and axes are in its PIXELS, how
/// many splats the dispatch covers, and the deposit pyramid's level table —
/// per level `[width, height, pixel offset into the accumulator, 0]`,
/// level 0 the raster itself.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DepositDims {
    /// `[raster w, raster h, splat count, level count]`. The level DIMS are
    /// not passed: FXC cannot dynamically index a uniform array without
    /// unrolling every loop that touches it, and the shader derives them
    /// from the raster instead — `ceil(raster / 2^level)`, which iterated
    /// ceil-halving ([`deposit_levels_of`]) provably equals, so the sizing
    /// here and the indexing there cannot disagree.
    head: [u32; 4],
}

/// Most accumulator levels the deposit pyramid holds — the twin of
/// `lumit_core::fx::lens_flare::MAX_DEPOSIT_LEVELS` and of the WGSL `Dims`
/// array size, pinned by test.
pub const MAX_DEPOSIT_LEVELS: usize = 12;

/// The deposit pyramid's level dimensions — the exact twin of
/// `lumit_core::fx::lens_flare::deposit_levels` (lumit-gpu stays
/// lumit-core-free in production, so the formula is mirrored and a test
/// pins the two together).
pub fn deposit_levels_of(w: u32, h: u32) -> Vec<(u32, u32)> {
    let (mut lw, mut lh) = (w.max(1), h.max(1));
    let mut out = vec![(lw, lh)];
    while out.len() < MAX_DEPOSIT_LEVELS && lw.max(lh) > 32 {
        lw = lw.div_ceil(2);
        lh = lh.div_ceil(2);
        out.push((lw, lh));
    }
    out
}

/// The pyramid's level count and its total size in pixels (all levels —
/// about a third again over level 0), which is what the accumulator is
/// allocated and cleared for.
fn deposit_pyramid_of(w: u32, h: u32) -> (u32, u64) {
    let levels = deposit_levels_of(w, h);
    let px = levels
        .iter()
        .map(|&(lw, lh)| u64::from(lw) * u64::from(lh))
        .sum();
    (levels.len() as u32, px)
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DetectParams {
    w: u32,
    h: u32,
    tiles_x: u32,
    tiles_y: u32,
    threshold: f32,
    softness: f32,
    use_source_colour: u32,
    /// 1 = read the matte inverted (`1 − rgb`), the Matte row's Invert;
    /// mirrors `lumit_core`'s `detect_lights` argument.
    invert: u32,
    tint: [f32; 3],
    _pad1: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlurParams {
    w: u32,
    h: u32,
    radius: u32,
    dir: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CombineParams {
    w: f32,
    h: f32,
    fw: f32,
    fh: f32,
    intensity: f32,
    sb_intensity: f32,
    sb_half: f32,
    squeeze: f32,
    fscale: f32,
    mix_amt: f32,
    light_count: u32,
    blend: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuCombo {
    bounce1: u32,
    bounce2: u32,
    lambda_nm: f32,
    _pad: f32,
    /// Index into the band table: the combo names the band, the
    /// band's eight sub-samples carry the colour the combo used to.
    band: u32,
    /// The third and fourth bounces of a four-bounce path, or
    /// `NO_BOUNCE` for the two-bounce ghosts that were all this struct held
    /// until then. They took two of the padding slots, so the layout — and
    /// every stride around it — is unchanged.
    bounce3: u32,
    bounce4: u32,
    /// This ghost's own **Fresnel number**, which sets how fine the
    /// diffraction fringes on its rim are; `0` leaves the plain analytic
    /// polygon. It took the struct's last padding slot, so the layout — and
    /// every stride around it — is again unchanged.
    ring_fresnel: f32,
}

/// One radiometric sub-sample in the WGSL `BandSub` layout, at
/// `band * 8 + k`: where in the reflectance table its wavelength sits, and
/// its RGB weight with the frame's energy gain already folded in.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuBandSub {
    lambda_idx: u32,
    r: f32,
    g: f32,
    b: f32,
}

/// The band table for a frame's `op.bands`, with `gain` folded into every
/// sub-sample weight — the trace kernel's binding 8.
fn band_subs_of(bands: &[FlareBand], gain: f32) -> Vec<GpuBandSub> {
    let mut out = Vec::with_capacity(bands.len() * 8);
    for band in bands {
        for k in 0..8 {
            let rgb = band.sub_rgb[k];
            out.push(GpuBandSub {
                lambda_idx: band.sub_idx[k],
                r: rgb[0] * gain,
                g: rgb[1] * gain,
                b: rgb[2] * gain,
            });
        }
    }
    out
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuSurface {
    row: [f32; 8],
}

/// One flare source in the WGSL `Light` layout: pos.xy, rgb, the source's
/// half-extent as a raster fraction, one pad.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuLight {
    row: [f32; 8],
}

/// The Lens flare's pipelines, built on a thread of their own.
///
/// # In plain terms
///
/// Compiling a shader is the graphics card's driver turning our code into
/// something it can run, and it happens when the program starts rather than
/// when it was written. Most of Lumit's kernels take a few milliseconds each.
/// The flare's ray tracer is not most kernels — it is the largest shader in the
/// program by a distance, and on a real card its three pipelines take about six
/// and a half seconds to compile.
///
/// That was six and a half seconds between opening a project and seeing
/// anything at all, because the render worker built every pipeline before it
/// would answer its first request — including for a composition with nothing in
/// it, and for the overwhelming majority of projects that contain no lens flare
/// at all. So the flare's pipelines are built on a background thread while the
/// rest of the engine gets on with the first frame, and the first frame that
/// actually draws a flare waits for that thread to finish. By then it long
/// since has.
///
/// Nothing about what the effect *draws* changes: this is only about when the
/// compiling happens.
pub(super) struct LazyFlare {
    /// For the fallback build: a device handle is cheap to hold (it is a
    /// reference-counted handle, not a copy of the card).
    device: wgpu::Device,
    /// The background build, taken and joined by the first [`Self::get`].
    building: Mutex<Option<std::thread::JoinHandle<LensFlareFx>>>,
    ready: OnceLock<LensFlareFx>,
    /// [`FxEngine::set_deferred_flare_bakes`] can be answered before there is
    /// an engine to tell — the worker sets it at start-up, which is precisely
    /// the moment this exists to keep free. Held here and applied on build.
    deferred: AtomicBool,
}

impl LazyFlare {
    pub(super) fn spawn(ctx: &GpuContext) -> Self {
        let device = ctx.device.clone();
        let building = std::thread::Builder::new()
            .name("lumit-flare-pipelines".into())
            .spawn({
                let device = device.clone();
                move || LensFlareFx::with_device(&device)
            })
            .ok();
        Self {
            device,
            building: Mutex::new(building),
            ready: OnceLock::new(),
            deferred: AtomicBool::new(false),
        }
    }

    /// The engine, waiting for the background build if it has not landed.
    ///
    /// A machine that would give us no thread, or a build thread that died,
    /// builds here instead — slowly, but on the path that actually needs it
    /// rather than never.
    pub(super) fn get(&self) -> &LensFlareFx {
        self.ready.get_or_init(|| {
            let built = self
                .building
                .lock()
                .ok()
                .and_then(|mut held| held.take())
                .and_then(|thread| thread.join().ok())
                .unwrap_or_else(|| LensFlareFx::with_device(&self.device));
            built
                .deferred
                .store(self.deferred.load(Ordering::Relaxed), Ordering::Relaxed);
            built
        })
    }

    /// The engine only if it is already built — for the questions asked on
    /// every frame (is a bake pending? what generation are we on?), which must
    /// never be the thing that waits for a compile. Neither has an answer worth
    /// having before the first flare is drawn anyway: nothing has been baked.
    pub(super) fn ready(&self) -> Option<&LensFlareFx> {
        self.ready.get()
    }

    pub(super) fn set_deferred(&self, deferred: bool) {
        self.deferred.store(deferred, Ordering::Relaxed);
        if let Some(lf) = self.ready.get() {
            lf.deferred.store(deferred, Ordering::Relaxed);
        }
    }

    /// Forget every bake, count and policy a previous test left here, so a
    /// shared engine (`crate::test_support`) starts each test the way a new
    /// one would. An engine still building has nothing to forget.
    #[cfg(any(test, feature = "test-fixtures"))]
    pub(super) fn reset_for_tests(&self) {
        self.deferred.store(false, Ordering::Relaxed);
        if let Some(lf) = self.ready.get() {
            lf.reset_for_tests();
        }
    }
}

impl LensFlareFx {
    /// See [`Self::cache`]. Raised from eight: a bake is a surface
    /// buffer and one 256² sprite, about a megabyte, so holding a couple of
    /// dozen costs less than one preview frame's working set and covers
    /// trying lenses — the way the picker is actually used.
    const CACHE_CAP: usize = 24;

    /// Everything here is built from the device alone, which is what lets
    /// [`LazyFlare`] build it on a thread that has no `GpuContext` to hand.
    pub(super) fn with_device(device: &wgpu::Device) -> Self {
        let storage_entry =
            |binding: u32, read_only: bool, vis: wgpu::ShaderStages| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: vis,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            };
        let uniform_entry = |binding: u32, vis: wgpu::ShaderStages| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: vis,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let texture_entry = |binding: u32, vis: wgpu::ShaderStages| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: vis,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };

        let c = wgpu::ShaderStages::COMPUTE;
        let trace_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx-lens-flare-trace-layout"),
            entries: &[
                storage_entry(0, true, c),
                storage_entry(1, true, c),
                storage_entry(2, false, c),
                // Binding 3 held the per-cell landed areas once; the
                // numbering is left alone so nothing else has to move.
                storage_entry(4, false, c),
                uniform_entry(5, c),
                storage_entry(6, true, c),
                // The spectral pair: the baked reflectance table and
                // the frame's band sub-samples.
                storage_entry(7, true, c),
                storage_entry(8, true, c),
                // Binding 9 held the ring masks until a closed form
                // replaced them; the numbering is left alone so nothing
                // else has to move.
            ],
        });
        let detect_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx-lens-flare-detect-layout"),
            entries: &[
                texture_entry(0, c),
                storage_entry(1, false, c),
                storage_entry(2, false, c),
                uniform_entry(3, c),
            ],
        });
        let deposit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx-lens-flare-deposit-layout"),
            entries: &[
                // The splats, the f32 accumulator they scatter into, the dims,
                // and the flare texture the resolve writes once at the end.
                storage_entry(0, true, c),
                storage_entry(1, false, c),
                uniform_entry(2, c),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: c,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: WORKING_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx-lens-flare-blur-layout"),
            entries: &[
                texture_entry(0, c),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: c,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: WORKING_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                uniform_entry(2, c),
            ],
        });
        let combine_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx-lens-flare-combine-layout"),
            entries: &[
                texture_entry(0, c),
                texture_entry(1, c),
                texture_entry(2, c),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: c,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: WORKING_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                uniform_entry(4, c),
                storage_entry(5, true, c),
            ],
        });

        let trace_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx-lens-flare-trace"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../fx_lens_flare_trace.wgsl").into()),
        });
        let detect_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx-lens-flare-detect"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../fx_lens_flare_detect.wgsl").into()),
        });
        let deposit_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx-lens-flare-deposit"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../fx_lens_flare_deposit.wgsl").into()),
        });
        let combine_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx-lens-flare-combine"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../fx_lens_flare_combine.wgsl").into()),
        });
        let blur_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx-lens-flare-blur"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../fx_lens_flare_blur.wgsl").into()),
        });

        let trace_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fx-lens-flare-trace-pl"),
            bind_group_layouts: &[&trace_layout],
            push_constant_ranges: &[],
        });
        let compute = |entry: &str, label: &str, module: &wgpu::ShaderModule, pl| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(pl),
                module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let trace = compute("trace", "fx-lens-flare-trace", &trace_mod, &trace_pl);
        let build_splats = compute(
            "build_splats",
            "fx-lens-flare-splats",
            &trace_mod,
            &trace_pl,
        );

        let detect_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fx-lens-flare-detect-pl"),
            bind_group_layouts: &[&detect_layout],
            push_constant_ranges: &[],
        });
        let detect_tiles = compute(
            "detect_tiles",
            "fx-lens-flare-detect-tiles",
            &detect_mod,
            &detect_pl,
        );
        let detect_pick = compute(
            "detect_pick",
            "fx-lens-flare-detect-pick",
            &detect_mod,
            &detect_pl,
        );

        let deposit_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fx-lens-flare-deposit-pl"),
            bind_group_layouts: &[&deposit_layout],
            push_constant_ranges: &[],
        });
        // Two entry points over one layout: the scatter, then the
        // single write into the fp16 texture. There is no raster pipeline any
        // more — the blender was the thing that could not add in f32.
        let deposit = compute(
            "deposit",
            "fx-lens-flare-deposit",
            &deposit_mod,
            &deposit_pl,
        );
        let resolve = compute(
            "resolve",
            "fx-lens-flare-resolve",
            &deposit_mod,
            &deposit_pl,
        );
        let combine_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fx-lens-flare-combine-pl"),
            bind_group_layouts: &[&combine_layout],
            push_constant_ranges: &[],
        });
        let combine = compute(
            "combine",
            "fx-lens-flare-combine",
            &combine_mod,
            &combine_pl,
        );
        let blur_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fx-lens-flare-blur-pl"),
            bind_group_layouts: &[&blur_layout],
            push_constant_ranges: &[],
        });
        let blur = compute("blur", "fx-lens-flare-blur", &blur_mod, &blur_pl);

        Self {
            trace,
            build_splats,
            detect_tiles,
            detect_pick,
            deposit,
            resolve,
            blur,
            combine,
            trace_layout,
            detect_layout,
            deposit_layout,
            blur_layout,
            combine_layout,
            cache: Mutex::new(BakeCache::new(Self::CACHE_CAP)),
            scratch: Mutex::new(None),
            baker: OnceLock::new(),
            deferred: std::sync::atomic::AtomicBool::new(false),
            last_drawn: Mutex::new(None),
            in_flight: Mutex::new(HashSet::new()),
            landed: Mutex::new(Vec::new()),
            generation: AtomicU64::new(0),
            substitutions: AtomicU64::new(0),
        }
    }

    /// The GPU bake for `op.bake_key`, or `None` when there is nothing to draw
    /// with yet.
    ///
    /// Two behaviours, chosen by [`FxEngine::set_deferred_flare_bakes`]:
    ///
    /// - **Exact** (the default, and what an export runs): a miss bakes here
    ///   and now, outside the lock, exactly as it always did. A racing
    ///   double-build is harmless — the bake is a pure function — and the
    ///   insert keeps whichever landed first.
    /// - **Deferred** (the Viewer): a miss hands the bake to the bake
    ///   thread and answers with the lens the last frame drew, so choosing a
    ///   lens is a wait you can watch rather than half a second of stopped
    ///   picture. With nothing drawn yet the answer is `None` and the flare
    ///   sits this frame out.
    ///
    /// Determinism is untouched either way: the bake is the same pure function
    /// of the same key wherever it runs, and the frame that finally draws the
    /// new lens is the frame the exact path would have drawn. What the caller
    /// must not do is *name* a provisional frame as though it were the real
    /// one — see [`FxEngine::flare_bake_generation`].
    fn baked(&self, ctx: &GpuContext, op: &LensFlareOp, bake: &FlareBake) -> Option<Arc<GpuBaked>> {
        // Anything the bake thread finished since the last frame, uploaded
        // here — on the render thread, which is the only thread that may
        // touch the device.
        self.collect(ctx);

        if let Ok(cache) = self.cache.lock() {
            if let Some(hit) = cache.get(op.bake_key) {
                drop(cache);
                self.remember_drawn(op.bake_key);
                return Some(hit);
            }
        }

        if !self.deferred.load(Ordering::Relaxed) {
            let data = bake();
            let built = Arc::new(upload_bake(ctx, &data));
            let stored = match self.cache.lock() {
                Ok(mut cache) => cache.insert(op.bake_key, built),
                Err(_) => built,
            };
            self.remember_drawn(op.bake_key);
            return Some(stored);
        }

        self.queue(op.bake_key, bake);

        // From here the frame draws something its parameters do not name, so
        // it is not a frame anybody may bank. Counted once, whether a
        // previous lens stands in or nothing does.
        self.substitutions.fetch_add(1, Ordering::Relaxed);

        // The lens the last frame drew. Not "any cached bake": stepping back
        // and forth through the picker must show the lens you just left, not
        // whichever entry the map happens to hold.
        let previous = self.last_drawn.lock().ok().and_then(|held| *held)?;
        self.cache.lock().ok().and_then(|cache| cache.get(previous))
    }

    /// Note which bake a frame drew with, so the next frame has something to
    /// fall back on while its own is being made.
    fn remember_drawn(&self, key: u64) {
        if let Ok(mut held) = self.last_drawn.lock() {
            *held = Some(key);
        }
    }

    /// Hand one bake to the bake thread, once per key. A key already in flight
    /// is not queued again, and a bake thread that cannot be started leaves
    /// the key unqueued — the next frame simply asks again.
    fn queue(&self, key: u64, bake: &FlareBake) {
        {
            let Ok(mut flight) = self.in_flight.lock() else {
                return;
            };
            if !flight.insert(key) {
                return;
            }
        }
        let queued = self
            .baker
            .get_or_init(Baker::start)
            .as_ref()
            .is_some_and(|baker| {
                baker
                    .jobs
                    .send(BakeJob {
                        key,
                        bake: Arc::clone(bake),
                    })
                    .is_ok()
            });
        if queued {
            // Queued, so this frame is drawing something other than what its
            // parameters name; the caller reads this to decide whether the
            // frame may be filed under that name.
            self.generation.fetch_add(1, Ordering::Relaxed);
        } else if let Ok(mut flight) = self.in_flight.lock() {
            flight.remove(&key);
        }
    }

    /// See [`LazyFlare::reset_for_tests`]. A bake still on the thread is
    /// waited for first, so it cannot surface in a later test as a lens
    /// nobody there asked for.
    #[cfg(any(test, feature = "test-fixtures"))]
    pub(super) fn reset_for_tests(&self) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while self.bake_pending() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if let Ok(mut cache) = self.cache.lock() {
            *cache = BakeCache::new(Self::CACHE_CAP);
        }
        if let Ok(mut landed) = self.landed.lock() {
            landed.clear();
        }
        if let Ok(mut flight) = self.in_flight.lock() {
            flight.clear();
        }
        if let Ok(mut last) = self.last_drawn.lock() {
            *last = None;
        }
        self.generation.store(0, Ordering::Relaxed);
        self.substitutions.store(0, Ordering::Relaxed);
        self.deferred.store(false, Ordering::Relaxed);
    }

    /// Take everything the bake thread has finished off its channel: clear
    /// each key from the in-flight list, bump the generation, and park the
    /// data for [`Self::collect`] to upload. Needs no device, so **any**
    /// thread may call it — which is the point: the worker's idle tick asks
    /// `bake_pending` between frames, and a landed bake must read as landed
    /// there, not only once a frame render happens to come by.
    fn poll_landed(&self) {
        let Some(baker) = self.baker.get().and_then(Option::as_ref) else {
            return;
        };
        // The finished bakes, taken out from under the lock before anything
        // else is locked: the rule is the rule even for a channel (docs/14).
        let mut fresh: Vec<(u64, Option<FlareBakeData>)> = Vec::new();
        if let Ok(done) = baker.done.lock() {
            while let Ok(one) = done.try_recv() {
                fresh.push(one);
            }
        }
        if fresh.is_empty() {
            return;
        }
        for (key, data) in fresh {
            // A superseded key comes back with nothing: it is taken off the
            // in-flight list and nothing is parked for it.
            if let Some(data) = data {
                if let Ok(mut landed) = self.landed.lock() {
                    landed.push((key, data));
                }
            }
            if let Ok(mut flight) = self.in_flight.lock() {
                flight.remove(&key);
            }
            self.generation.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Upload and file everything the bake thread has finished. Called at the
    /// top of every [`Self::baked`], which is the only place that runs on the
    /// render thread with a device to hand.
    fn collect(&self, ctx: &GpuContext) {
        self.poll_landed();
        let parked: Vec<(u64, FlareBakeData)> = match self.landed.lock() {
            Ok(mut landed) => landed.drain(..).collect(),
            Err(_) => Vec::new(),
        };
        for (key, data) in parked {
            let built = Arc::new(upload_bake(ctx, &data));
            if let Ok(mut cache) = self.cache.lock() {
                cache.insert(key, built);
            }
        }
    }

    /// Queue a bake by key without asking for a picture — see
    /// [`FxEngine::warm_flare_bake`]. Answers whether anything was queued.
    pub(super) fn warm(&self, key: u64, bake: &FlareBake) -> bool {
        let before = self.generation.load(Ordering::Relaxed);
        self.queue(key, bake);
        self.generation.load(Ordering::Relaxed) != before
    }

    /// Whether a bake is being made right now — so a frame drawn during it is
    /// showing a lens other than the one it names.
    pub(super) fn bake_pending(&self) -> bool {
        // Landed-but-not-uploaded is not pending: the upload is microseconds
        // inside the next frame, and it is that next frame the caller is
        // deciding whether to make.
        self.poll_landed();
        self.in_flight
            .lock()
            .map(|f| !f.is_empty())
            .unwrap_or(false)
    }

    /// The bake for this key, made here and now if it is not held — whatever
    /// the deferral policy. What the trace-debug path and any caller that
    /// needs *this* lens rather than a picture uses.
    fn baked_exact(
        &self,
        ctx: &GpuContext,
        op: &LensFlareOp,
        bake: &FlareBake,
    ) -> Option<Arc<GpuBaked>> {
        self.collect(ctx);
        if let Ok(cache) = self.cache.lock() {
            if let Some(hit) = cache.get(op.bake_key) {
                return Some(hit);
            }
        }
        let built = Arc::new(upload_bake(ctx, &bake()));
        Some(match self.cache.lock() {
            Ok(mut cache) => cache.insert(op.bake_key, built),
            Err(_) => built,
        })
    }

    /// Take the pooled scratch if it is free and big enough, else build one.
    /// The lock covers the take and nothing else — never an allocation, never
    /// an encode (docs/14 §4).
    fn take_scratch(
        &self,
        ctx: &GpuContext,
        ray_bytes: u64,
        splat_bytes: u64,
        accum_bytes: u64,
    ) -> Scratch {
        let held = self.scratch.lock().ok().and_then(|mut s| s.take());
        match held {
            // Big enough and not wildly oversized: reuse as is. The upper
            // bound gives a frame that once ran at Ultra its memory back
            // when the quality goes down again, instead of holding the
            // peak for the session.
            Some(s)
                if s.ray_bytes >= ray_bytes
                    && s.splat_bytes >= splat_bytes
                    && s.accum_bytes >= accum_bytes
                    && s.ray_bytes <= ray_bytes.saturating_mul(4)
                    && s.splat_bytes <= splat_bytes.saturating_mul(4)
                    && s.accum_bytes <= accum_bytes.saturating_mul(4) =>
            {
                s
            }
            _ => Scratch {
                rays: scratch_buffer(ctx, "fx-lens-flare-rays", ray_bytes),
                splats: scratch_buffer(ctx, "fx-lens-flare-splats", splat_bytes),
                accum: accum_buffer(ctx, accum_bytes),
                accum_bytes,
                ray_bytes,
                splat_bytes,
            },
        }
    }

    /// Hand the scratch back for the next frame.
    fn put_scratch(&self, scratch: Scratch) {
        if let Ok(mut slot) = self.scratch.lock() {
            if slot.is_none() {
                *slot = Some(scratch);
            }
        }
    }
}

/// The splat accumulator (a level pyramid). Cleared to
/// zero each frame, so it needs `COPY_DST` beside the storage binding.
///
/// NOT clamped to [`SCRATCH_BYTE_BUDGET`]: that budget bounds the per-batch
/// ray/splat scratch, whose size the settings can wind up, where this is a
/// framebuffer-scale allocation the raster alone decides — level 0 must
/// hold every pixel of the flare buffer or the deposit writes past the end
/// (the clamp silently truncated exactly that way past 2K, and the padded
/// pyramid would have hit it sooner).
fn accum_buffer(ctx: &GpuContext, bytes: u64) -> wgpu::Buffer {
    ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("fx-lens-flare-accum"),
        size: bytes.max(16),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// A storage buffer for the trace scratch. Never zero-sized (wgpu rejects
/// that) and never past the budget.
fn scratch_buffer(ctx: &GpuContext, label: &str, bytes: u64) -> wgpu::Buffer {
    ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes.clamp(16, SCRATCH_BYTE_BUDGET),
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    })
}

/// The pupil grid one pair gets, from the quality base and the pair's image
/// spread — the exact twin of `lumit_core::fx::lens_flare::pair_grid`
/// (lumit-gpu stays lumit-core-free in production, so the formula is
/// mirrored and a test pins the two together).
pub fn pair_grid_of(base: u32, spread: f32) -> u32 {
    let mult = if spread < 0.5 {
        1.0
    } else if spread < 1.5 {
        1.75
    } else {
        2.5
    };
    ((base as f32 * mult).round() as u32).clamp(8, 512)
}

/// Fixed-point steps in one unit of radiance in the splat accumulator
/// — the twin of `ACCUM_SCALE` in `fx_lens_flare_deposit.wgsl`, pinned against
/// the shader text by test.
pub const ACCUM_SCALE: f32 = 16777216.0;

/// The radiance one accumulator channel may reach before its u32 wraps:
/// `u32::MAX / ACCUM_SCALE`. A test measures the CPU reference's
/// brightest pixel against it.
pub const ACCUM_CEILING: f32 = 255.99998;

/// One ghost's Fresnel number from its image spread and the working stop —
/// the exact twin of `lumit_core::fx::lens_flare::ghost_fresnel_number`,
/// pinned by test.
///
/// `spread` is the bake's measured spread already scaled by the frame's
/// `stop_scale`, since stopping down shrinks the ghost as well as the pupil.
pub fn ghost_fresnel_of(spread: f32, fstop: f32) -> f32 {
    if !(spread.is_finite() && fstop.is_finite()) || spread <= 0.0 || fstop <= 0.0 {
        return 0.0;
    }
    let sensor_diag = (SENSOR_MM[0] * SENSOR_MM[0] + SENSOR_MM[1] * SENSOR_MM[1]).sqrt();
    let a_um = spread * sensor_diag * 0.5 * 1000.0;
    (a_um / (2.0 * fstop * RING_LAMBDA_UM)).clamp(0.0, 1.0e6)
}

/// The full-frame sensor the trace projects onto, mm — the twin of
/// `lumit_core::fx::lens_flare::SENSOR_MM`.
pub const SENSOR_MM: [f32; 2] = [36.0, 24.0];

/// The wavelength the ghost-edge ringing is scaled at, µm — the twin of
/// `lumit_core::fx::lens_flare::RING_LAMBDA_UM`.
pub const RING_LAMBDA_UM: f32 = 0.55;

/// The flare buffer's padded dimensions for Squeeze/Scale under 1 — the
/// exact twin of `lumit_core::fx::lens_flare::flare_pad_dims`,
/// pinned by test.
pub fn flare_pad_dims_of(fw: u32, fh: u32, squeeze: f32, scale: f32) -> (u32, u32) {
    let squeeze = squeeze.clamp(0.25, 4.0);
    let fscale = scale.clamp(0.05, 20.0);
    let px = (1.0 / (squeeze * fscale)).clamp(1.0, 2.0);
    let py = (1.0 / fscale).clamp(1.0, 2.0);
    (
        ((fw as f32) * px).ceil() as u32,
        ((fh as f32) * py).ceil() as u32,
    )
}

/// One dispatch of the trace → splats → raster chain: a run of combos that
/// share a pupil grid, and a chunk of the frame's lights.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Batch {
    /// The pupil grid every combo in the batch traces at.
    pub(super) grid: u32,
    /// First combo of the batch in the frame's combo table.
    pub(super) combo_offset: u32,
    /// How many combos it covers.
    pub(super) combos: u32,
    /// First light.
    pub(super) light_offset: u32,
    /// How many lights.
    pub(super) lights: u32,
    /// Scratch the batch needs: ray landings and their splats.
    pub(super) ray_bytes: u64,
    pub(super) splat_bytes: u64,
}

impl Batch {
    /// Ray–surface steps the batch asks the card for — the cost model the
    /// submission split is paced by. One traced ray walks the prescription
    /// about three times (forward to the far bounce, back to the near one,
    /// forward to the sensor), which is where the 3 comes from; it only has
    /// to be proportional.
    pub(super) fn steps(&self, surface_count: u32) -> u64 {
        u64::from(self.grid)
            * u64::from(self.grid)
            * u64::from(self.combos)
            * u64::from(self.lights)
            * u64::from(surface_count.max(1))
            * 3
    }

    /// The deposit's cost for this batch, in the same step-shaped units:
    /// the pixels its splats touch, summed from the per-combo
    /// estimates. Independent of the ray count — the splats of a coarser
    /// grid are individually larger, so a ghost's deposit always costs its
    /// own image area times the kernel overlap, however it is sampled.
    pub(super) fn deposit_px(&self, combo_costs: &[u64]) -> u64 {
        let from = self.combo_offset as usize;
        let to = (self.combo_offset + self.combos) as usize;
        let per_light: u64 = combo_costs
            .get(from..to.min(combo_costs.len()))
            .map(|c| c.iter().sum())
            .unwrap_or(0);
        per_light.saturating_mul(u64::from(self.lights))
    }
}

/// One combo's deposit cost estimate: the pixels its splats will
/// touch, from the pair's bake-time image spread. The quadratic B-spline
/// reaches one and a half grid steps each way, so each splat covers about
/// nine times its own cell of the ghost — nine times the ghost's area in
/// total, whatever the grid. `spread` is the pair's bounding measure as a
/// fraction of the sensor diagonal; squaring the whole diagonal extent
/// over-counts elongated ghosts, which errs the safe way for a pacing
/// bound.
pub(super) fn combo_deposit_cost(spread: f32, diag_px: f32) -> u64 {
    let extent = (spread.clamp(0.0, 4.0) * diag_px.max(0.0)).min(1.0e5);
    (9.0 * extent * extent) as u64
}

/// Which batches of a plan end a command buffer: `true` at index `i`
/// means the frame hands over what it has encoded after batch `i` and opens a
/// fresh encoder.
///
/// Separate from the encoding loop so it can be checked without a graphics
/// card — the thing it prevents (a submission long enough for the operating
/// system to kill, taking the device with it) is precisely the thing a test
/// cannot afford to reproduce.
///
/// Paced by the trace's ray–surface steps AND the deposit's pixels:
/// earlier only the trace was counted, and the deposit — nine times
/// each ghost's image area in atomic adds, per combo per light — rode along
/// unmetered, so a frame of big defocused ghosts packed *seconds* of scatter
/// into one submission. That is the shape of submission the watchdog kills,
/// and a killed submission takes the device (and the session) with it.
pub(super) fn plan_flushes(plan: &[Batch], surface_count: u32, combo_costs: &[u64]) -> Vec<bool> {
    let mut flushes = vec![false; plan.len()];
    let mut pending = 0u64;
    for (i, batch) in plan.iter().enumerate() {
        pending = pending
            .saturating_add(batch.steps(surface_count))
            .saturating_add(batch.deposit_px(combo_costs));
        if pending >= STEPS_PER_SUBMIT {
            flushes[i] = true;
            pending = 0;
        }
    }
    flushes
}

/// Cut the frame's grid-major combo table into [`Batch`]es that each fit
/// [`SCRATCH_BYTE_BUDGET`] and stay under one submission's worth of
/// deposit work.
///
/// The combo table is sorted by grid, so equal grids are contiguous: each run
/// becomes one or more batches, split by however many (light × combo) slots
/// the budget affords at that grid. When even ONE combo across every light
/// will not fit — an Ultra grid with eight matte sources — the lights split
/// too, which is what makes the budget a real bound rather than a wish.
/// Lights are chunked inside the combo batch so the drawn order stays
/// light-major within a batch, exactly as it was.
///
/// `combo_costs` (parallel to `combo_grids`, [`combo_deposit_cost`] each) is
/// the other half: a batch is the atomic unit of encoding, so a flush
/// between batches cannot save a frame whose ONE batch holds sixty-four
/// frame-filling deposits. The slot count is therefore also capped so a
/// batch's deposit stays about one [`STEPS_PER_SUBMIT`] — a batch of one
/// combo and one light is always allowed, and is itself about that size for
/// the biggest ghost a padded 4K buffer can hold.
pub(super) fn plan_batches(
    combo_grids: &[u32],
    light_count: u32,
    combo_costs: &[u64],
) -> Vec<Batch> {
    let mut plan = Vec::new();
    let lights_total = light_count.max(1);
    let mut offset = 0usize;
    while offset < combo_grids.len() {
        let grid = combo_grids[offset].clamp(2, 512);
        let run_end = combo_grids[offset..]
            .iter()
            .position(|&g| g != combo_grids[offset])
            .map(|n| offset + n)
            .unwrap_or(combo_grids.len());
        let rays = u64::from(grid) * u64::from(grid);
        // One splat per RAY, not one drawn cell per quad.
        let per_slot = rays * (RAY_BYTES + SPLAT_BYTES);
        // At least one slot always runs: a single (light × combo) pair at the
        // widest grid is 27 MB, inside the budget, so this floor is a
        // formality that keeps the loop total rather than a silent overrun.
        let slots = (SCRATCH_BYTE_BUDGET / per_slot.max(1)).max(1);
        // The deposit cap: the run's worst deposit sets how many slots one
        // submission can afford.
        let worst_cost = combo_costs
            .get(offset..run_end.min(combo_costs.len()))
            .and_then(|c| c.iter().max().copied())
            .unwrap_or(0);
        let slots = slots.min((STEPS_PER_SUBMIT / worst_cost.max(1)).max(1));
        let light_chunk = lights_total
            .min(slots.min(u64::from(u32::MAX)) as u32)
            .max(1);
        let combo_cap = (slots / u64::from(light_chunk)).clamp(1, 64) as u32;
        let mut at = offset;
        while at < run_end {
            let combos = combo_cap.min((run_end - at) as u32);
            let mut light_at = 0u32;
            while light_at < lights_total {
                let lights = light_chunk.min(lights_total - light_at);
                let slots_used = u64::from(lights) * u64::from(combos);
                plan.push(Batch {
                    grid,
                    combo_offset: at as u32,
                    combos,
                    light_offset: light_at,
                    lights,
                    ray_bytes: slots_used * rays * RAY_BYTES,
                    splat_bytes: slots_used * rays * SPLAT_BYTES,
                });
                light_at += lights;
            }
            at += combos as usize;
        }
        offset = run_end;
    }
    plan
}

/// Upload one bake's buffers as GPU resources.
fn upload_bake(ctx: &GpuContext, data: &FlareBakeData) -> GpuBaked {
    use wgpu::util::DeviceExt;
    let rows: Vec<GpuSurface> = data
        .surfaces
        .iter()
        .map(|&row| GpuSurface { row })
        .collect();
    let surfaces = ctx
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fx-lens-flare-surfaces"),
            contents: bytemuck::cast_slice(&rows),
            usage: wgpu::BufferUsages::STORAGE,
        });
    // A bake with no surfaces has no table; WGSL cannot bind an empty
    // buffer, so one zero stands in (every lookup then reads out of range
    // and returns 0, exactly as the CPU's `table.get` does).
    let table: &[f32] = if data.reflectance.is_empty() {
        &[0.0]
    } else {
        &data.reflectance
    };
    let reflectance = ctx
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fx-lens-flare-reflectance"),
            contents: bytemuck::cast_slice(table),
            usage: wgpu::BufferUsages::STORAGE,
        });
    let float_texture = |label: &str, w: u32, h: u32, format: wgpu::TextureFormat, bytes: &[u8]| {
        ctx.device.create_texture_with_data(
            &ctx.queue,
            &wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            bytes,
        )
    };
    // The starburst's RGB triplets pad to rgba32float rows (alpha unused);
    // f32 keeps the CPU/GPU oracle tight.
    let mut rgba = Vec::with_capacity(data.starburst.len() / 3 * 4);
    for rgb in data.starburst.chunks_exact(3) {
        rgba.extend_from_slice(rgb);
        rgba.push(0.0f32);
    }
    // One atlas, the field slices stacked vertically, so `sb_tex`
    // stays a plain 2D texture and the combine offsets its taps by slice.
    let starburst = float_texture(
        "fx-lens-flare-starburst",
        data.sb_res,
        data.sb_res * data.sb_fields.max(1),
        wgpu::TextureFormat::Rgba32Float,
        bytemuck::cast_slice(&rgba),
    );
    GpuBaked {
        surfaces,
        reflectance,
        surface_count: data.surfaces.len() as u32,
        surface_rows: data.surfaces.clone(),
        ghosts: data.ghosts.clone(),
        spreads: data.spreads.clone(),
        sensor_z_mm: data.sensor_z_mm,
        focal_mm: data.focal_mm,
        native_fstop: data.native_fstop,
        pupil_mm: data.pupil_mm,
        start_z_mm: data.start_z_mm,
        energy_gain: data.energy_gain,
        starburst,
    }
}

impl FxEngine {
    /// Apply one Lens flare (docs/08 §3.27) to a linear working texture,
    /// returning a new texture of the same size. `bake` is called only when
    /// the op's bake key misses the cache (the caller wraps
    /// `lumit_core::fx::lens_flare::bake`). `matte` is the Matte source's
    /// rendered layer (the DoF layer-input shape) — read only when
    /// `op.source == 1`; an absent matte there means no sources, the
    /// labelled-no-op convention. The whole frame — source detection, trace,
    /// energy, splat build, the additive ghost raster in batches, and the
    /// combine — encodes into ONE encoder and submits once.
    #[allow(clippy::too_many_arguments)]
    pub fn lens_flare(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &LensFlareOp,
        matte: Option<&wgpu::Texture>,
        bake: &FlareBake,
        probe: &dyn Fn(&FlareProbeBake) -> Vec<u32>,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        let out = work_texture(ctx, w, h, "fx-lens-flare-out");
        let lf = self.lens_flare.get();

        // Neutral short-circuit mirror (the combine kernel also guards, but
        // skipping the whole pipeline is the honest fast path).
        let ghost_count_max = op.max_ghosts.min(200);
        let wanted = op.intensity > 0.0 && op.mix > 0.0;
        let baked = if wanted {
            lf.baked(ctx, op, bake)
        } else {
            None
        };
        // A deferred bake that has nothing to fall back on yet leaves the
        // frame with no flare at all rather than a wrong one: `live` is
        // "there is something to draw", and everything below already reads it
        // as that.
        let live = baked.is_some();

        // Matte mode runs with MAX_SOURCES candidate slots (dead ones carry
        // zero weight and cost no fill); Manual and the prepared Lights mode
        // run one.
        let matte_mode = op.source == 1;
        // The frame's light list. Manual fills its slots from the CPU — one
        // per light, size and all; Matte mode overwrites the buffer
        // with the detection kernels below and may fill any of them, so it
        // always dispatches the lot.
        let mut light_rows = vec![GpuLight { row: [0.0; 8] }; MAX_SOURCES as usize];
        // Matte mode leaves every slot zero here on purpose: the detection
        // kernels below fill them, and if no matte is bound they never run —
        // which is what makes an unset matte the labelled no-op rather than a
        // manual light nobody asked for.
        let mut manual_count = 1;
        if !matte_mode {
            if op.manual_lights.is_empty() {
                light_rows[0] = GpuLight {
                    row: [
                        op.light_frac[0],
                        op.light_frac[1],
                        op.light_tint[0],
                        op.light_tint[1],
                        op.light_tint[2],
                        0.0,
                        0.0,
                        0.0,
                    ],
                };
            } else {
                let n = op.manual_lights.len().min(MAX_SOURCES as usize);
                for (slot, light) in op.manual_lights.iter().take(n).enumerate() {
                    light_rows[slot] = GpuLight {
                        row: [
                            light[0], light[1], light[2], light[3], light[4], light[5], light[6],
                            0.0,
                        ],
                    };
                }
                manual_count = n as u32;
            }
        }
        let light_count = if matte_mode {
            MAX_SOURCES
        } else {
            manual_count
        };
        let lights_buf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-lens-flare-lights"),
                contents: bytemuck::cast_slice(&light_rows),
                usage: wgpu::BufferUsages::STORAGE,
            });

        // Flare buffer (half size on Draft), and its 4x multisample target:
        // the ghost raster draws into the multisampled texture and
        // resolves into `flare_tex`, which everything downstream (blur,
        // combine) reads exactly as before. The multisample texture exists
        // only while a live frame draws.
        let div = op.flare_div.max(1);
        let (fw, fh) = ((w / div).max(1), (h / div).max(1));
        // The buffer renders PADDED for Squeeze/Scale under 1: the
        // combine samples past the base extent there, and the padding puts
        // real flare where a zero-outside tap showed black. Geometry
        // is centred; the screen transform stays derived from the base.
        let (fpw, fph) = flare_pad_dims_of(fw, fh, op.anamorphic, op.scale);
        let flare_tex = work_texture(ctx, fpw, fph, "fx-lens-flare-buffer");
        // No multisample target — the raster antialiases itself.
        let _ = live;

        let mut encoder = ctx.encoder("fx-lens-flare-enc");

        // Matte-mode source detection (impl note §6): tile maxima, then the
        // serial top-K pick — both before any trace pass reads the lights.
        if live && matte_mode {
            if let Some(matte) = matte {
                let (mw, mh) = (matte.width(), matte.height());
                let tiles_x = mw.div_ceil(DETECT_TILE);
                let tiles_y = mh.div_ceil(DETECT_TILE);
                let tiles_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("fx-lens-flare-tiles"),
                    // Ten words per tile: the brightest pixel's
                    // luma and index, then the gated coverage, colour and
                    // flux moments that describe the whole lit area of it.
                    size: u64::from(tiles_x * tiles_y) * 40,
                    usage: wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                });
                let dp = ctx
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("fx-lens-flare-detect-params"),
                        contents: bytemuck::bytes_of(&DetectParams {
                            w: mw,
                            h: mh,
                            tiles_x,
                            tiles_y,
                            threshold: op.threshold,
                            softness: op.threshold_softness,
                            use_source_colour: u32::from(op.use_source_colour),
                            invert: u32::from(op.matte_invert),
                            tint: op.light_tint,
                            _pad1: 0.0,
                        }),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("fx-lens-flare-detect-bind"),
                    layout: &lf.detect_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(
                                &matte.create_view(&Default::default()),
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: tiles_buf.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: lights_buf.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: dp.as_entire_binding(),
                        },
                    ],
                });
                {
                    let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("fx-lens-flare-detect-tiles-pass"),
                        timestamp_writes: None,
                    });
                    cpass.set_pipeline(&lf.detect_tiles);
                    cpass.set_bind_group(0, &bind, &[]);
                    cpass.dispatch_workgroups(tiles_x, tiles_y, 1);
                }
                {
                    let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("fx-lens-flare-detect-pick-pass"),
                        timestamp_writes: None,
                    });
                    cpass.set_pipeline(&lf.detect_pick);
                    cpass.set_bind_group(0, &bind, &[]);
                    cpass.dispatch_workgroups(1, 1, 1);
                }
            }
            // No matte bound: the zero-filled lights render nothing — the
            // labelled-no-op convention for an unset layer reference.
        }

        // Always clear the flare buffer (a zero-ghost frame must not read
        // stale memory). A live frame overwrites every texel in the
        // resolve, so this is what covers the idle one — and the frame that
        // plans no batches at all, whose resolve writes the cleared
        // accumulator's zeros.
        {
            let view = flare_tex.create_view(&Default::default());
            let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fx-lens-flare-clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }

        if let Some(baked) = &baked {
            // Build the (ghost × wavelength) combo table for this frame; the
            // light dimension rides the dispatch z instead (its population is
            // only known GPU-side in Matte mode).
            let ghost_count = (ghost_count_max as usize).min(baked.ghosts.len());
            let gain = baked.energy_gain;
            // Manual mode's frame-time grid probe: ask the caller
            // (who owns the lumit-core maths) for the frame's final per-pair
            // grids — each pair's rung floor raised toward its need under
            // the frame's bounded ray headroom. Matte lights exist GPU-side
            // only, so Matte mode keeps the bake grids; the CPU reference
            // gates the same way and parity holds.
            let frame_grids: Vec<u32> = if !matte_mode {
                probe(&FlareProbeBake {
                    surfaces: &baked.surface_rows,
                    ghosts: &baked.ghosts,
                    sensor_z_mm: baked.sensor_z_mm,
                    focal_mm: baked.focal_mm,
                    native_fstop: baked.native_fstop,
                    pupil_mm: baked.pupil_mm,
                    start_z_mm: baked.start_z_mm,
                    spreads: &baked.spreads,
                    pair_count: ghost_count,
                })
            } else {
                Vec::new()
            };
            // Each pair gets its own pupil grid by its measured image spread
            // (mirroring `lumit_core::fx::lens_flare::pair_grid`), so
            // combos are sorted grid-major: a run of equal-grid combos is one
            // dispatch batch, and the scratch is sized for the widest grid.
            // The working stop shrinks the pupil, and with it every ghost —
            // which is what each path's rim-fringe scale is derived from.
            let stop_scale =
                frame_optics(baked.native_fstop, baked.focal_mm, op.fstop, op.focus_m).stop_scale;
            // The flare buffer's diagonal scales each pair's bake spread
            // into the deposit-cost estimate the flush pacing reads.
            let diag_px = ((fpw * fpw + fph * fph) as f32).sqrt();
            let mut tagged: Vec<(u32, u64, GpuCombo)> =
                Vec::with_capacity(ghost_count * op.bands.len());
            for (gi, ghost) in baked.ghosts.iter().take(ghost_count).enumerate() {
                let spread = baked.spreads.get(gi).copied().unwrap_or(1.0);
                let rung = pair_grid_of(op.grid, spread);
                let pg = frame_grids.get(gi).copied().unwrap_or(rung);
                for (bi, band) in op.bands.iter().enumerate() {
                    tagged.push((
                        pg,
                        combo_deposit_cost(spread * stop_scale, diag_px),
                        GpuCombo {
                            bounce1: ghost[0],
                            bounce2: ghost[1],
                            lambda_nm: band.traced_nm,
                            _pad: 0.0,
                            band: bi as u32,
                            bounce3: ghost[2],
                            bounce4: ghost[3],
                            ring_fresnel: ghost_fresnel_of(spread * stop_scale, op.fstop),
                        },
                    ));
                }
            }
            // Stable sort: equal grids become contiguous runs and ties keep
            // the ranked pair order (determinism, §2.4).
            tagged.sort_by_key(|&(g, _, _)| g);
            let combo_grids: Vec<u32> = tagged.iter().map(|&(g, _, _)| g).collect();
            let combo_costs: Vec<u64> = tagged.iter().map(|&(_, c, _)| c).collect();
            let combos: Vec<GpuCombo> = tagged.into_iter().map(|(_, _, c)| c).collect();
            if !combos.is_empty() {
                // The frame's dispatch plan. Combos are sorted
                // grid-major, so the table falls into runs of one grid; each
                // run is cut into batches of combos and chunks of lights that
                // fit the scratch budget, and every batch strides the scratch
                // by ITS OWN grid. One stride once served the whole
                // frame — the widest grid in it — so a single frame-filling
                // ghost made every compact ghost dispatch and draw at that
                // ghost's ray count, tens of times the rays they own.
                let plan = plan_batches(&combo_grids, light_count, &combo_costs);
                let combos_buf = ctx
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("fx-lens-flare-combos"),
                        contents: bytemuck::cast_slice(&combos),
                        usage: wgpu::BufferUsages::STORAGE,
                    });
                // The bands' sub-samples, carrying the exposure
                // gain the combo colour used to. Non-empty whenever the
                // combo table is: a combo exists only per band.
                let band_subs = band_subs_of(&op.bands, gain);
                let bands_buf = ctx
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("fx-lens-flare-bands"),
                        contents: bytemuck::cast_slice(&band_subs),
                        usage: wgpu::BufferUsages::STORAGE,
                    });
                // One scratch for the whole frame, big enough for its widest
                // batch and pooled between frames (see [`Scratch`]).
                let (need_rays, need_splats) = plan.iter().fold((16u64, 16u64), |(r, s), b| {
                    (r.max(b.ray_bytes), s.max(b.splat_bytes))
                });
                // Three fixed-point channels a pixel, across every level of
                // the deposit pyramid.
                let (level_count, accum_px) = deposit_pyramid_of(fpw, fph);
                let accum_bytes = accum_px * 3 * 4;
                let scratch = lf.take_scratch(ctx, need_rays, need_splats, accum_bytes);
                // Start from nothing: the accumulator is pooled, so last
                // frame's sums are still in it.
                encoder.clear_buffer(&scratch.accum, 0, Some(accum_bytes));

                // The resolve writes it once, at the end.
                let flare_view = flare_tex.create_view(&Default::default());
                // Where the frame breaks its work into separate submissions.
                let flushes = plan_flushes(&plan, baked.surface_count, &combo_costs);
                for (bi, job) in plan.iter().enumerate() {
                    let Batch {
                        grid,
                        combo_offset: offset,
                        combos: batch,
                        light_offset: light_from,
                        lights: light_chunk,
                        ..
                    } = *job;
                    let batch_rays = grid * grid;
                    // Frame-time optics shared with the CPU reference,
                    // plus the launch cell area in flare-buffer px².
                    let FrameOptics {
                        stop_scale,
                        wide_open,
                        sensor_shift_mm,
                    } = frame_optics(baked.native_fstop, baked.focal_mm, op.fstop, op.focus_m);
                    let st_flare = op.screen_transform / div as f32;
                    let cell_mm = 2.0 * baked.pupil_mm * stop_scale / (grid.max(2) - 1) as f32;
                    let params = ctx
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("fx-lens-flare-trace-params"),
                            contents: bytemuck::bytes_of(&TraceParams {
                                surface_count: baked.surface_count,
                                combo_count: batch,
                                grid,
                                combo_offset: offset,
                                coating: op.coating,
                                aspect: h as f32 / w.max(1) as f32,
                                focal_mm: baked.focal_mm,
                                // Project into the flare buffer's raster.
                                screen_transform: st_flare,
                                raster_w: fpw as f32,
                                raster_h: fph as f32,
                                light_count: light_chunk,
                                sensor_shift_mm,
                                pupil_mm: baked.pupil_mm * stop_scale,
                                start_z_mm: baked.start_z_mm,
                                sensor_z_mm: baked.sensor_z_mm,
                                stop_scale,
                                cell_area_px: cell_mm * cell_mm * st_flare * st_flare,
                                // This batch's own stride.
                                ray_stride: batch_rays,
                                _pad_stride: 0,
                                blades: op.blades.clamp(3, 16),
                                rot_rad: op.aperture_rotation_deg.to_radians(),
                                roundness: op.roundness.max(wide_open),
                                softness: op.aperture_softness,
                                light_offset: light_from,
                            }),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                    let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("fx-lens-flare-trace-bind"),
                        layout: &lf.trace_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: baked.surfaces.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: combos_buf.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: scratch.rays.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 4,
                                resource: scratch.splats.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 5,
                                resource: params.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 6,
                                resource: lights_buf.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 7,
                                resource: baked.reflectance.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 8,
                                resource: bands_buf.as_entire_binding(),
                            },
                        ],
                    });
                    // The deposit's own view of the frame: the flare
                    // buffer's size, and EXACTLY how many splats this batch
                    // filled. It has to be exact — the dispatch's last
                    // workgroup runs a tail of up to 63 idle threads, and a
                    // looser bound would have them deposit whatever the
                    // previous batch left in those slots.
                    let splats_here = light_chunk * batch * batch_rays;
                    let deposit_dims =
                        ctx.device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("fx-lens-flare-deposit-dims"),
                                contents: bytemuck::bytes_of(&DepositDims {
                                    head: [fpw, fph, splats_here, level_count],
                                }),
                                usage: wgpu::BufferUsages::UNIFORM,
                            });
                    let deposit_bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("fx-lens-flare-deposit-bind"),
                        layout: &lf.deposit_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: scratch.splats.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: scratch.accum.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: deposit_dims.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::TextureView(&flare_view),
                            },
                        ],
                    });
                    // Each stage in its own pass: the pass boundary is the
                    // write-then-read barrier between them. The splat stage
                    // reads its NEIGHBOURS' landings for the footprint,
                    // and a neighbour traced by another workgroup
                    // needs a pass boundary to be visible.
                    let stages: [(&wgpu::ComputePipeline, u32, &str); 2] = [
                        (&lf.trace, batch_rays, "fx-lens-flare-trace-pass"),
                        (&lf.build_splats, batch_rays, "fx-lens-flare-splats-pass"),
                    ];
                    for (pipeline, x_items, label) in stages {
                        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some(label),
                            timestamp_writes: None,
                        });
                        cpass.set_pipeline(pipeline);
                        cpass.set_bind_group(0, &bind, &[]);
                        cpass.dispatch_workgroups(x_items.div_ceil(64), batch, light_chunk);
                    }
                    {
                        // The deposit: one thread per splat, scattering
                        // into the f32 accumulator. This is where the raster
                        // pass used to be, and the reason it is not one any
                        // more is that the blender could only sum in fp16.
                        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("fx-lens-flare-deposit-pass"),
                            timestamp_writes: None,
                        });
                        cpass.set_pipeline(&lf.deposit);
                        cpass.set_bind_group(0, &deposit_bind, &[]);
                        cpass.dispatch_workgroups(splats_here.div_ceil(64), 1, 1);
                    }
                    // Hand over what is encoded before it grows into a
                    // submission the operating system would kill (see
                    // [`STEPS_PER_SUBMIT`]).
                    if flushes[bi] {
                        // The guard is dropped first because the flush needs
                        // the batch it borrows. Inside a frame batch this
                        // submits the frame so far and opens a fresh buffer;
                        // outside one it submits this pass's own. Either way
                        // the reason is unchanged — the scratch below is
                        // recycled once this work has gone to the driver.
                        drop(encoder);
                        ctx.flush();
                        encoder = ctx.encoder("fx-lens-flare-enc");
                    }
                }
                // One write into the fp16 texture, now that every batch has
                // added its light in f32. `splat_count` is nothing to
                // the resolve, which walks the raster rather than the splats.
                let resolve_dims =
                    ctx.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("fx-lens-flare-resolve-dims"),
                            contents: bytemuck::bytes_of(&DepositDims {
                                head: [fpw, fph, 0, level_count],
                            }),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                let resolve_bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("fx-lens-flare-resolve-bind"),
                    layout: &lf.deposit_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: scratch.splats.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: scratch.accum.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: resolve_dims.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(&flare_view),
                        },
                    ],
                });
                {
                    let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("fx-lens-flare-resolve-pass"),
                        timestamp_writes: None,
                    });
                    cpass.set_pipeline(&lf.resolve);
                    cpass.set_bind_group(0, &resolve_bind, &[]);
                    cpass.dispatch_workgroups(fpw.div_ceil(8), fph.div_ceil(8), 1);
                }
                lf.put_scratch(scratch);
            }

            // Ghost blur (FlareSim's Ghost Blur): 3 separable box
            // passes over the flare buffer, ping-ponging through a scratch
            // texture — an even pass count lands the result back in
            // `flare_tex` for the combine.
            // Mirrors `lumit_core::fx::lens_flare::ghost_blur_radius`,
            // cap included (an uncapped radius on a 4K frame is a
            // thousand taps per pixel across six passes — a GPU timeout).
            // Ghost softness is px@comp and already raster pixels
            // here, so the radius is the number itself over the flare
            // buffer's own divisor — a distance the padding does not
            // change; the passes run over the padded buffer.
            let radius = {
                let r = op.ghost_softness.max(0.0) / op.flare_div.max(1) as f32;
                (r.round() as u32).min(80)
            };
            if radius > 0 {
                let scratch_tex = work_texture(ctx, fpw, fph, "fx-lens-flare-blur-scratch");
                for pass in 0..3u32 {
                    for dir in 0..2u32 {
                        let _ = pass;
                        let (src_t, dst_t) = if dir == 0 {
                            (&flare_tex, &scratch_tex)
                        } else {
                            (&scratch_tex, &flare_tex)
                        };
                        let bp = ctx
                            .device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("fx-lens-flare-blur-params"),
                                contents: bytemuck::bytes_of(&BlurParams {
                                    w: fpw,
                                    h: fph,
                                    radius,
                                    dir,
                                }),
                                usage: wgpu::BufferUsages::UNIFORM,
                            });
                        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("fx-lens-flare-blur-bind"),
                            layout: &lf.blur_layout,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: wgpu::BindingResource::TextureView(
                                        &src_t.create_view(&Default::default()),
                                    ),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: wgpu::BindingResource::TextureView(
                                        &dst_t.create_view(&Default::default()),
                                    ),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 2,
                                    resource: bp.as_entire_binding(),
                                },
                            ],
                        });
                        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("fx-lens-flare-blur-pass"),
                            timestamp_writes: None,
                        });
                        cpass.set_pipeline(&lf.blur);
                        cpass.set_bind_group(0, &bind, &[]);
                        // x runs ALONG the blur axis in tiles of 64, y across
                        // it — the shape the line cache needs.
                        let (along, across) = if dir == 0 { (fpw, fph) } else { (fph, fpw) };
                        cpass.dispatch_workgroups(along.div_ceil(64), across, 1);
                    }
                }
            }
        }

        // Combine. The starburst texture must exist even when the bake was
        // skipped (identity path): bind a 1×1 black stand-in then.
        let black;
        let sb_tex = match &baked {
            Some(b) => &b.starburst,
            None => {
                black = work_texture(ctx, 1, 1, "fx-lens-flare-black");
                &black
            }
        };
        let fscale = op.scale.clamp(0.05, 20.0);
        let cp = CombineParams {
            w: w as f32,
            h: h as f32,
            fw: fw as f32,
            fh: fh as f32,
            intensity: op.intensity,
            sb_intensity: op.starburst_intensity,
            sb_half: 0.6 * fscale * w.min(h) as f32,
            squeeze: op.anamorphic.clamp(0.25, 4.0),
            fscale,
            mix_amt: op.mix,
            light_count,
            blend: op.blend.min(BLEND_COUNT - 1),
        };
        let cp_buf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-lens-flare-combine-params"),
                contents: bytemuck::bytes_of(&cp),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let combine_bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-lens-flare-combine-bind"),
            layout: &lf.combine_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &src.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &flare_tex.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        &sb_tex.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(
                        &out.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: cp_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: lights_buf.as_entire_binding(),
                },
            ],
        });
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-lens-flare-combine-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&lf.combine);
            cpass.set_bind_group(0, &combine_bind, &[]);
            cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        drop(encoder);
        out
    }

    /// The trace-oracle hook (docs/impl/lens-flare.md §8.5): run the trace
    /// pass alone for the first `combo_limit` (pair × band) combos of the
    /// op's MANUAL light and read the ray buffer back — the whole WGSL `Ray`
    /// per corner, combo-major: `[pos_x, pos_y, weight, pad, r, g, b, pad]`,
    /// weight −1 being the GPU's dead sentinel where the CPU returns None.
    /// The weight is geometry (feather × iris mask) and the rgb
    /// is the band-integrated energy, so the two halves of
    /// `trace_splat_spectral` can be checked apart. `w`/`h` feed the aspect
    /// the in-shader light direction uses. Diagnostics and tests only; no
    /// production path calls it.
    pub fn lens_flare_trace_debug(
        &self,
        ctx: &GpuContext,
        op: &LensFlareOp,
        bake: &FlareBake,
        combo_limit: u32,
        w: u32,
        h: u32,
    ) -> Vec<[f32; 8]> {
        use wgpu::util::DeviceExt;
        let lf = self.lens_flare.get();
        // The debug trace wants the real bake, whatever the policy: it is a
        // test and diagnostic path, and an answer for the previous lens would
        // be an answer to a question nobody asked.
        let Some(baked) = lf.baked_exact(ctx, op, bake) else {
            return Vec::new();
        };
        let ghost_count = (op.max_ghosts as usize).min(baked.ghosts.len());
        let stop_scale =
            frame_optics(baked.native_fstop, baked.focal_mm, op.fstop, op.focus_m).stop_scale;
        let mut combos: Vec<GpuCombo> = Vec::new();
        'outer: for (gi, ghost) in baked.ghosts.iter().take(ghost_count).enumerate() {
            for (bi, band) in op.bands.iter().enumerate() {
                if combos.len() >= combo_limit as usize {
                    break 'outer;
                }
                combos.push(GpuCombo {
                    bounce1: ghost[0],
                    bounce2: ghost[1],
                    lambda_nm: band.traced_nm,
                    _pad: 0.0,
                    band: bi as u32,
                    bounce3: ghost[2],
                    bounce4: ghost[3],
                    ring_fresnel: ghost_fresnel_of(
                        baked.spreads.get(gi).copied().unwrap_or(1.0) * stop_scale,
                        op.fstop,
                    ),
                });
            }
        }
        if combos.is_empty() {
            return Vec::new();
        }
        // The op's own band weights, unscaled: the debug hook answers what
        // one traced ray carries, not what a frame's auto-exposure makes of
        // it (the production path folds `energy_gain` in here).
        let band_subs = band_subs_of(&op.bands, 1.0);
        let bands_buf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-lens-flare-dbg-bands"),
                contents: bytemuck::cast_slice(&band_subs),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let grid = op.grid.clamp(2, 128);
        let ray_count = grid * grid;
        let combos_buf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-lens-flare-dbg-combos"),
                contents: bytemuck::cast_slice(&combos),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let mut light_rows = vec![GpuLight { row: [0.0; 8] }; MAX_SOURCES as usize];
        light_rows[0] = GpuLight {
            row: [
                op.light_frac[0],
                op.light_frac[1],
                op.light_tint[0],
                op.light_tint[1],
                op.light_tint[2],
                0.0,
                0.0,
                0.0,
            ],
        };
        let lights_buf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-lens-flare-dbg-lights"),
                contents: bytemuck::cast_slice(&light_rows),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let rays_size = combos.len() as u64 * u64::from(ray_count) * RAY_BYTES;
        let rays_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx-lens-flare-dbg-rays"),
            size: rays_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        // The layout requires every binding; the trace entry never touches
        // the splat buffer, so a minimal stand-in satisfies it.
        let splats_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx-lens-flare-dbg-splats"),
            size: SPLAT_BYTES,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let params = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-lens-flare-dbg-params"),
                contents: bytemuck::bytes_of(&{
                    let FrameOptics {
                        stop_scale,
                        wide_open,
                        sensor_shift_mm,
                    } = frame_optics(baked.native_fstop, baked.focal_mm, op.fstop, op.focus_m);
                    let cell_mm = 2.0 * baked.pupil_mm * stop_scale / (grid.max(2) - 1) as f32;
                    TraceParams {
                        surface_count: baked.surface_count,
                        combo_count: combos.len() as u32,
                        grid,
                        combo_offset: 0,
                        coating: op.coating,
                        aspect: h as f32 / w.max(1) as f32,
                        focal_mm: baked.focal_mm,
                        screen_transform: op.screen_transform,
                        raster_w: w as f32,
                        raster_h: h as f32,
                        light_count: 1,
                        sensor_shift_mm,
                        pupil_mm: baked.pupil_mm * stop_scale,
                        start_z_mm: baked.start_z_mm,
                        sensor_z_mm: baked.sensor_z_mm,
                        stop_scale,
                        cell_area_px: cell_mm * cell_mm * op.screen_transform * op.screen_transform,
                        ray_stride: grid * grid,
                        _pad_stride: 0,
                        blades: op.blades.clamp(3, 16),
                        rot_rad: op.aperture_rotation_deg.to_radians(),
                        roundness: op.roundness.max(wide_open),
                        softness: op.aperture_softness,
                        light_offset: 0,
                    }
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-lens-flare-dbg-bind"),
            layout: &lf.trace_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: baked.surfaces.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: combos_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: rays_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: splats_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: lights_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: baked.reflectance.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: bands_buf.as_entire_binding(),
                },
            ],
        });
        let read_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx-lens-flare-dbg-read"),
            size: rays_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("fx-lens-flare-dbg-enc"),
            });
        {
            let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-lens-flare-dbg-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&lf.trace);
            cpass.set_bind_group(0, &bind, &[]);
            cpass.dispatch_workgroups(ray_count.div_ceil(64), combos.len() as u32, 1);
        }
        enc.copy_buffer_to_buffer(&rays_buf, 0, &read_buf, 0, rays_size);
        ctx.submit([enc.finish()]);
        let slice = read_buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        ctx.device.poll(wgpu::Maintain::Wait);
        if rx.recv().map(|r| r.is_err()).unwrap_or(true) {
            return Vec::new();
        }
        let data = slice.get_mapped_range();
        data.chunks_exact(RAY_BYTES as usize)
            .map(|row| {
                let f = |i: usize| f32::from_le_bytes([row[i], row[i + 1], row[i + 2], row[i + 3]]);
                [f(0), f(4), f(8), f(12), f(16), f(20), f(24), f(28)]
            })
            .collect()
    }
}
