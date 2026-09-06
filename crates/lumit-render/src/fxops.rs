//! The one place a resolved effect stack becomes GPU passes.
//!
//! In plain terms: `lumit_core::fx::resolve_stack` turns a layer's effect
//! list into plain numbers, and this module walks that list calling the
//! matching `FxEngine` kernel for each entry. The preview, the export
//! renderer, and adjustment-layer staging all call through here, so an
//! effect wired up once runs identically in all three — and there is no match
//! over effects at all any more: the parameters come out of the stack's arena,
//! and the kernel out of `gpufx`'s table, looked up by the effect's own name.

use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

use crate::gpufx::{AuxData, AuxKind, AuxSlot};

type Tex = wgpu::Texture;

/// A parsed-and-uploaded `.cube` LUT ready to bind (docs/08 §3.11,
/// docs/impl/lut.md): the 3D cube texture, its per-axis size `N`, and the input
/// domain the file declared. Held by [`LutCache`] and cloned into a `run_ops`
/// `luts` slot; `wgpu::Texture` is an `Arc` handle, so the clone is cheap and
/// shares the one upload.
#[derive(Clone)]
pub struct LoadedLut {
    pub texture: Tex,
    pub size: u32,
    /// Where the cube came from and when that file last changed — the same
    /// two things [`LutCache`] keys on, carried so the per-effect cache
    /// can name a `lut` op by its file without reading the cube back.
    pub path: String,
    pub mtime: Option<std::time::SystemTime>,
    /// `DOMAIN_MIN` / `DOMAIN_MAX` from the file (default `0..1`), carried to
    /// the kernel so the GPU remaps exactly as the CPU oracle does.
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
}

/// How many distinct `.cube` files stay uploaded at once. A comp references a
/// handful at most (docs/impl/lut.md §4); past that the least recently used one
/// is dropped, which releases its GPU texture. Small on purpose: an unbounded
/// cache turned every path a session ever touched into a permanent upload.
const LUT_CACHE_MAX: usize = 8;

/// The parsed-and-uploaded `.cube` files a render can bind, keyed by **path and
/// last-modified time** and bounded to [`LUT_CACHE_MAX`] entries, most recently
/// used first (docs/impl/lut.md §4).
///
/// **Why the mtime is part of the key.** Grading is iterative: export a cube
/// from the grading tool, look at it in Lumit, adjust, export again over the
/// same path. Keyed by path alone, the second export never appeared — Lumit
/// kept showing the first grade until the application was restarted, with
/// nothing on screen to say so. A file whose mtime has moved is a
/// different entry, so it is parsed and uploaded again.
///
/// A path the filesystem will not stat (it vanished, or the platform has no
/// mtime for it) keys as `None`, which still matches itself: such a file is
/// cached by path exactly as before rather than being re-read every frame.
#[derive(Default)]
pub struct LutCache {
    /// Most recently used first. A `Vec` rather than a map because the bound is
    /// eight: a linear scan of eight strings is faster than hashing one, and
    /// the ordering the LRU needs is the `Vec`'s own.
    entries: Vec<(String, Option<std::time::SystemTime>, LoadedLut)>,
}

impl LutCache {
    /// The cube at `path`, parsed and uploaded on a miss. `None` for anything
    /// that is not a usable 3D cube — an unreadable file, a parse error, or a
    /// 1D LUT — which leaves the effect a labelled passthrough rather than a
    /// render failure (docs/08 §3.11, the never-crash rule).
    pub fn get_or_load(&mut self, ctx: &GpuContext, path: &str) -> Option<LoadedLut> {
        let mtime = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
        if let Some(i) = self
            .entries
            .iter()
            .position(|(p, t, _)| p == path && *t == mtime)
        {
            // Touch: this entry is now the most recently used.
            let hit = self.entries.remove(i);
            let lut = hit.2.clone();
            self.entries.insert(0, hit);
            return Some(lut);
        }
        // A stale entry for this path (the file was edited) is replaced, not
        // kept alongside: the old grade is exactly what nobody wants back.
        self.entries.retain(|(p, _, _)| p != path);
        let text = std::fs::read_to_string(path).ok()?;
        let lumit_core::lut::Lut::Cube3d(l) = lumit_core::lut::parse_cube(&text).ok()? else {
            return None;
        };
        let loaded = LoadedLut {
            texture: lumit_gpu::fx::upload_lut_3d(ctx, l.size as u32, &l.data),
            size: l.size as u32,
            path: path.to_owned(),
            mtime,
            domain_min: l.domain_min,
            domain_max: l.domain_max,
        };
        self.entries
            .insert(0, (path.to_owned(), mtime, loaded.clone()));
        self.entries.truncate(LUT_CACHE_MAX);
        Some(loaded)
    }

    /// How many cubes are held — the bound's test hook, and a number worth
    /// having when a profile asks where the video memory went.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// One effect's finished output, held on the card. Costed as the
/// `Rgba16Float` work texture it is — eight bytes a pixel — so the budget is
/// a true count of video memory.
pub struct CachedTex(pub Tex);

impl lumit_cache::ByteSized for CachedTex {
    fn byte_size(&self) -> usize {
        (self.0.width() as usize) * (self.0.height() as usize) * 8
    }
}

/// How much video memory the per-effect cache holds by default: a few dozen
/// 1080p intermediates, which is a handful of layers' worth of stacks around
/// the playhead. Settable through [`FxCache::set_budget`].
pub const FX_CACHE_DEFAULT_BUDGET: usize = 256 * 1024 * 1024;

/// The **per-effect intermediate cache** (docs/06 §5.1): every
/// effect's output, kept on the card under the content name of *everything
/// that went into it* — the layer's source, the raster, and each op up to and
/// including this one. Editing the last effect of a stack then re-runs only
/// that effect, because the picture the one before it produced is still held
/// under a name nothing in the edit changed.
///
/// The same store holds **nested frames**: a non-collapsed Precomp's
/// finished linear texture, under its own frame key mixed with the raster it
/// was made at ([`nested_texture_key`]). One store, one budget, one set of
/// clear hooks; the two kinds of entry cannot collide because each name
/// begins with its own tag.
///
/// VRAM only, and deliberately so: an intermediate is worth keeping for the
/// seconds between two edits of the same stack, not across sessions. It sits
/// in [`crate::realise::Realiser`] beside the LUT cache and is consulted by
/// [`run_ops`]; entries are only *added* on committed, non-playback renders
/// ([`Self::keep_outputs`]), so a drag's provisional pictures and a playback
/// run's hundreds of frames never churn it — they still read from it.
pub struct FxCache {
    lru: lumit_cache::ByteLru<u128, CachedTex>,
    keep: bool,
    /// Nested frames the decode planner was told are held for the frame being
    /// rendered ([`Self::pin_nested`]). A plan that skipped a nested
    /// comp's decodes on the strength of a lookup must find the texture still
    /// here when the realiser asks, whatever this frame's own inserts evict
    /// in between — so the planner's lookup takes a handle, and the handle
    /// lives until the next frame's plan drops it.
    pins: std::collections::HashMap<u128, Tex>,
    /// Test hooks: how many kernels [`run_ops`] actually ran against this
    /// cache, and how many ops it skipped because their output was held.
    runs: u64,
    hits: u64,
    /// Test hooks: nested frames realised, and served held.
    nested_made: u64,
    nested_served: u64,
}

impl Default for FxCache {
    fn default() -> Self {
        Self::new(FX_CACHE_DEFAULT_BUDGET)
    }
}

impl FxCache {
    #[must_use]
    pub fn new(budget_bytes: usize) -> Self {
        Self {
            lru: lumit_cache::ByteLru::new(budget_bytes),
            keep: false,
            pins: std::collections::HashMap::new(),
            runs: 0,
            hits: 0,
            nested_made: 0,
            nested_served: 0,
        }
    }

    /// The finished texture of a nested comp's frame, by the name the realiser
    /// files it under, or `None` when it must be realised. Counted.
    pub fn nested(&mut self, key: u128) -> Option<Tex> {
        let held = self
            .pins
            .get(&key)
            .cloned()
            .or_else(|| self.lru.get(&key).map(|t| t.0.clone()));
        if held.is_some() {
            self.nested_served += 1;
        } else {
            self.nested_made += 1;
        }
        held
    }

    /// File a nested comp's finished frame under its name, when the
    /// cache is taking entries ([`Self::keep_outputs`]).
    pub fn put_nested(&mut self, key: u128, tex: Tex) {
        if self.keep {
            self.lru.insert(key, CachedTex(tex));
        }
    }

    /// Whether a nested frame is held, for the decode planner — and if
    /// it is, hold it for the frame about to be rendered (see `pins`).
    pub fn pin_nested(&mut self, key: u128) -> bool {
        if let Some(t) = self.lru.get(&key) {
            self.pins.insert(key, t.0.clone());
            true
        } else {
            false
        }
    }

    /// Drop the previous frame's pins: called as each frame's plan begins.
    pub fn unpin_nested(&mut self) {
        self.pins.clear();
    }

    /// Whether the renders that follow may *add* to the cache. Lookups always
    /// happen; this gates inserts to committed, non-playback frames.
    pub fn keep_outputs(&mut self, keep: bool) {
        self.keep = keep;
    }

    pub fn set_budget(&mut self, bytes: usize) {
        self.lru.set_budget(bytes);
    }

    pub fn clear(&mut self) {
        self.lru.clear();
        self.pins.clear();
    }

    /// `(used_bytes, budget_bytes, entries)`.
    #[must_use]
    pub fn stats(&self) -> (usize, usize, usize) {
        (
            self.lru.used_bytes(),
            self.lru.budget_bytes(),
            self.lru.len(),
        )
    }

    /// `(kernels run, ops served from the cache)` since construction.
    #[must_use]
    pub fn counts(&self) -> (u64, u64) {
        (self.runs, self.hits)
    }

    /// `(nested frames realised, nested frames served held)` since
    /// construction.
    #[must_use]
    pub fn nested_counts(&self) -> (u64, u64) {
        (self.nested_made, self.nested_served)
    }
}

/// The name a nested comp's finished texture is filed under: its
/// frame key, and the two things that decide the texture's shape which the
/// key does not — the exact render scale the realiser allocates at (the key
/// holds the 1% tier, and two scales in one tier can differ by a pixel) and
/// the sample count the device resolved to.
#[must_use]
pub fn nested_texture_key(frame_key: u128, render_scale: f32, samples: u32) -> u128 {
    let mut h = blake3::Hasher::new();
    h.update(b"nested/1/");
    h.update(&frame_key.to_le_bytes());
    h.update(&render_scale.to_bits().to_le_bytes());
    h.update(&samples.to_le_bytes());
    let mut k = [0u8; 16];
    k.copy_from_slice(&h.finalize().as_bytes()[..16]);
    u128::from_le_bytes(k)
}

/// One layer-input slot as [`run_ops`] receives it — the realised twin of
/// [`crate::draw::LayerInputDraw`] (docs/impl/layer-input.md).
pub enum LayerInput {
    /// Nothing to read: the effect degrades to its labelled no-op.
    Absent,
    /// The effect's **own input** at its point in the stack. There is no
    /// texture to carry here because only `run_ops` knows it — it is the
    /// picture the chain is holding when the op comes round, which on an
    /// adjustment layer is the composite of everything below.
    ThisLayer,
    /// Another layer, already rendered alone at this raster.
    Texture(Tex),
}

impl LayerInput {
    /// The texture this slot names, given the texture the chain currently
    /// holds. `None` for [`LayerInput::Absent`] — the passthrough.
    #[must_use]
    pub fn texture<'a>(&'a self, current: &'a Tex) -> Option<&'a Tex> {
        match self {
            LayerInput::Absent => None,
            LayerInput::ThisLayer => Some(current),
            LayerInput::Texture(t) => Some(t),
        }
    }
}

/// Render one referenced layer alone into the depth input a depth-of-field
/// effect samples (docs/impl/layer-input.md §2). The **one** helper the preview
/// (`GpuViewer`) and export (`Renderer`) paths both call, so the depth pass is
/// byte-identical in the viewport and the file — exactly as
/// `Compositor::motion_blur_average` and the matte "render alone" composite are
/// shared.
///
/// The effect stack runs on the consuming layer's own working raster `(w, h)`
/// (the layer's decoded size, which shrinks under reduced-resolution preview),
/// and the DoF kernel reads the depth at that same pixel grid — so the depth
/// input must be exactly `(w, h)` and aligned with the layer texture. v1 model
/// (documented in docs/08 §3.22): the referenced layer's **source** is
/// resampled to fill `(w, h)` — the depth pass is expected to share the
/// footage's framing (the standard "footage + matching depth pass" workflow),
/// so it is stretched to the working raster and its own transform is not
/// applied. A placement-aware depth is a recorded follow-up. `linear` is the
/// referenced layer's source in the working linear format, sized
/// `(src_w, src_h)`; each caller uploads/linearises it its own way, as the
/// matte path does, and this helper owns only the resample so it never drifts.
pub fn render_layer_input(
    compositor: &lumit_gpu::Compositor,
    ctx: &GpuContext,
    w: u32,
    h: u32,
    linear: &Tex,
    src_w: f32,
    src_h: f32,
) -> Tex {
    compositor.composite_with_camera(
        ctx,
        w,
        h,
        [0.0, 0.0, 0.0, 0.0],
        &[lumit_gpu::CompositeLayer {
            texture: linear,
            size: (src_w.max(1.0), src_h.max(1.0)),
            position: (0.0, 0.0),
            anchor: (0.0, 0.0),
            // Stretch the source to fill the whole working raster.
            scale: (
                w as f32 / src_w.max(1.0) * 100.0,
                h as f32 / src_h.max(1.0) * 100.0,
            ),
            rotation_deg: 0.0,
            // Full opacity: a depth pass is read as a scalar, never dimmed.
            opacity: 100.0,
            matte: None,
            blend: lumit_gpu::Blend::Normal,
            z: 0.0,
            rotation_x_deg: 0.0,
            rotation_y_deg: 0.0,
            three_d: false,
            layer_mask: None,
            pre: None,
        }],
        None,
    )
}

/// Run `ops` over `tex` in order, returning the final texture (the input
/// unchanged when `ops` is empty). `w`/`h` are the texture's raster size.
/// `neighbours` are the layer's decoded neighbour frames keyed by offset
/// (empty unless the stack has a temporal effect); a temporal op like Echo
/// reads them, single-frame ops ignore them. `flow_fields` are the layer's
/// dense motion fields (per-pixel `(u, v)` at this raster size) keyed by the
/// neighbour offset each was measured against — one entry per flow-consuming
/// effect in the stack (Flow motion blur reads `+1`, Datamosh `-1`; §3.2,
/// §3.12), since the two want opposite measurements and a single shared field
/// was never something both could read. An op whose offset is absent is
/// a passthrough (degrade, never fault). `luts` is the parallel LUT list (docs/08 §3.11): the
/// k-th `lut` op binds `luts[k]` — a `None` slot (unset, missing, 1D or
/// unreadable file) is a passthrough, exactly like a missing flow field.
/// `layer_inputs` is the parallel layer-input list (docs/08 §3.28,
/// docs/impl/layer-input.md): the k-th layer-input-consuming op — `light_wrap`
/// alone, since Depth of field's depth pass moved onto the matte list —
/// binds `layer_inputs[k]`, the referenced layer rendered alone at comp size,
/// [`LayerInput::ThisLayer`] for the effect's own input, or
/// [`LayerInput::Absent`] (unset, missing or cyclic) for a passthrough, exactly
/// like a missing LUT. `flare_lens` is the parallel custom-prescription list
/// (`lens_file` as content hash + text; None = use the picked library
/// lens): the k-th `lens_flare` op binds `flare_lens[k]`.
///
/// `mattes` is the parallel **Matte** list (docs/08 §2.6): one slot per
/// op whose effect declares any [`MatteRole`](lumit_core::fx::MatteRole) with a
/// parameter — which is every op — exactly as `build.rs`'s `mattes_for`
/// enumerates them. What a bound slot *does* is the role's second question, and
/// the only branch in this function that cares: an effect on the generic
/// strength semantic gets the dissolve below, and an effect that claims the
/// matte inside its own maths (Gaussian blur, Glow, Depth of field, the Lens
/// flare) gets the texture handed to its kernel instead — never both, or the
/// matte would be applied twice. An absent slot (the default, and every
/// project saved before the matte list existed) runs no extra pass at all, so
/// the picture is byte-for-byte what it was.
///
/// `mask_paths` is the parallel **mask-path** list (docs/08 §1.2):
/// one flattened polyline per
/// [`ParamKind::MaskPath`](lumit_core::fx::ParamKind::MaskPath) **row** of every
/// op that declares any, exactly as `build.rs`'s `mask_paths_for` enumerates
/// them — an op takes as many in a row as its schema declares (one for the three
/// line-drawing effects, two for the Matte key's garbage mattes). Its own
/// counter, not the matte's: every op takes a matte and almost none takes a
/// path, so one shared index would hand a path to whichever effect happened to
/// sit above. An empty polyline is the effect's documented no-op.
///
/// `cache` is the per-effect intermediate cache and the content name
/// of `tex` — the layer's source as the draw builder named it. `None` (no
/// store, or an input nothing can name yet: an adjustment layer's composite, a
/// nested comp, a text or shape layer) walks the stack exactly as before. With
/// both, the walk starts after the longest run of ops whose outputs are held,
/// and files each output it makes when the cache is taking them.
#[allow(clippy::too_many_arguments)]
pub fn run_ops(
    fx: &FxEngine,
    ctx: &GpuContext,
    tex: Tex,
    w: u32,
    h: u32,
    ops: &lumit_core::fx::ResolvedStack,
    neighbours: &[(i32, Tex)],
    flow_fields: &[(i32, Tex)],
    luts: &[Option<LoadedLut>],
    layer_inputs: &[LayerInput],
    flare_lens: &[Option<(u64, String)>],
    mattes: &[LayerInput],
    mask_paths: &[lumit_core::mask::MaskPolyline],
    points_schedules: &[lumit_core::fx::points::PointsSchedule],
    timings: Option<&mut Vec<f32>>,
    cache: Option<(&std::cell::RefCell<FxCache>, u128)>,
) -> Tex {
    run_ops_with_roto(
        fx,
        ctx,
        tex,
        w,
        h,
        ops,
        neighbours,
        flow_fields,
        luts,
        layer_inputs,
        flare_lens,
        mattes,
        mask_paths,
        points_schedules,
        &[],
        timings,
        cache,
    )
}

/// [`run_ops`] with the **roto carriage** threaded through: one slot per
/// `roto_brush` op, in stack order, holding the matte its propagation filed for
/// this layer's source frame — or `None`, which is the effect's passthrough.
///
/// A separate entry point rather than a seventeenth parameter on the one every
/// test calls: the roto matte is the only side list whose *absence* is the
/// overwhelmingly normal case, and `run_ops` forwarding an empty slice says so
/// in one line instead of at twenty call sites.
///
/// ponytail: two entry points for one walk; fold them back into one parameter
/// list the moment a *second* side list wants the same treatment, since three
/// forwarding wrappers would cost more than the twenty edits do.
#[allow(clippy::too_many_arguments)]
pub fn run_ops_with_roto(
    fx: &FxEngine,
    ctx: &GpuContext,
    tex: Tex,
    w: u32,
    h: u32,
    ops: &lumit_core::fx::ResolvedStack,
    neighbours: &[(i32, Tex)],
    flow_fields: &[(i32, Tex)],
    luts: &[Option<LoadedLut>],
    layer_inputs: &[LayerInput],
    flare_lens: &[Option<(u64, String)>],
    mattes: &[LayerInput],
    mask_paths: &[lumit_core::mask::MaskPolyline],
    points_schedules: &[lumit_core::fx::points::PointsSchedule],
    roto_mattes: &[Option<Tex>],
    mut timings: Option<&mut Vec<f32>>,
    cache: Option<(&std::cell::RefCell<FxCache>, u128)>,
) -> Tex {
    let mut tex = tex;
    // The working raster, which **one** effect can grow mid-stack: Tile
    // with Output width or height above 100 % stamps copies past the frame's
    // edges, and the ops after it run on the wider picture so those copies are
    // real picture to them rather than transparency. Every other op returns the
    // size it was given, so this pair never moves and the walk is what it was.
    // The caller reads the finished size off the texture; a caller that cannot
    // take a wider one crops it back through `lumit_gpu::fx::fit_centred`.
    let (mut w, mut h) = (w, h);
    // The name of each op's output, before anything runs: the input's
    // name, the raster, the flare substitution count, then op after op — so
    // the k-th name covers ops 0..=k. `None` from the first op that binds
    // something the name cannot cover (another layer's picture, the neighbour
    // frames, the flow field) onwards: an output that depends on a picture
    // nobody named must not be filed under a name that omits it.
    let subs_before = fx.flare_substitutions();
    let keys: Vec<Option<u128>> = cache.map_or_else(
        || vec![None; ops.len()],
        |(_, input_key)| {
            op_keys(
                input_key,
                w,
                h,
                subs_before,
                ops,
                luts,
                layer_inputs,
                flare_lens,
                mattes,
                mask_paths,
                points_schedules,
            )
        },
    );
    // The longest held prefix, found from the *last* op backwards: an edit
    // to the last effect is the common case, and it is answered by the op
    // before it in one lookup.
    let mut start = 0usize;
    if let Some((store, _)) = cache {
        let mut store = store.borrow_mut();
        for i in (0..ops.len()).rev() {
            let Some(key) = keys.get(i).copied().flatten() else {
                continue;
            };
            if let Some(held) = store.lru.get(&key) {
                tex = held.0.clone();
                // The held picture may have been made by a growing op,
                // in which case the ops that follow it run at *its* raster and
                // not the layer's.
                w = tex.width();
                h = tex.height();
                start = i + 1;
                store.hits += start as u64;
                break;
            }
        }
    }
    // Outputs made this walk, filed at the end rather than as they are made:
    // a flare bake queued *during* the walk means a picture of the previous
    // lens, which must not be filed under the name of the new one.
    let mut made: Vec<(u128, Tex)> = Vec::new();
    // The k-th `lut` op consumes the k-th `luts` slot (the whole threading
    // contract — see `build.rs`'s `lut_files` and CompLayerDraw's lut_files); a
    // slot is present only when its `.cube` file loaded. The k-th
    // layer-input-consuming op consumes the k-th `layer_inputs` slot the same
    // way. Both share one counter because `build.rs`'s `layer_inputs_for`
    // enumerates them with one predicate, in one order; two counters would let
    // the two sides drift apart silently.
    //
    // An effect names its list rather than carrying an arm of its own
    // (`GpuEffect::aux`), and the loop below advances the counter it
    // names — so the ops that count along a list are counted in one place, in
    // stack order. The whole-list kinds take no counter because they were never
    // per-op.
    let mut lut_i = 0usize;
    let mut dof_i = 0usize;
    let mut flare_i = 0usize;
    // The Matte's own counter. Deliberately *not* `dof_i`: the two lists
    // are enumerated by two different predicates — one effect takes a background
    // plate, every effect takes a matte — and sharing one index would bind a
    // matte to whichever Light wrap happened to sit above it. Same contract,
    // second predicate; it advances outside the GPU lookup below because
    // `build.rs` fills a slot per *op*, not per kernel.
    let mut matte_i = 0usize;
    // The mask path's own counter, for the same reason:
    // `build.rs` flattens one polyline per path **row** of every op that
    // declares any, and this advances on exactly that count —
    // `EffectSchema::mask_paths`, the one enumeration both sides run, so there
    // is no second rule to keep in step.
    let mut path_i = 0usize;
    // The points carriage's own counter (points-stream.md §3.3), on the
    // signature's own points predicate — the one `build.rs` fills by, which
    // answers for a **consumer** as well as a producer. Neither the layer's
    // clock, nor the whole Emit rate track, nor a stream a wire brings in is a
    // number in the bag, so they ride beside the op the way a polyline does.
    let mut sched_i = 0usize;
    // The roto carriage's own counter, on its own predicate — one slot
    // per `roto_brush` op, which is the enumeration `build.rs`'s
    // `roto_mattes_for` fills by. Its own rather than shared for the mask
    // path's reason: almost no op is a Roto brush, and one shared index would
    // hand a matte to whichever effect happened to sit above.
    let mut roto_i = 0usize;
    for (i, resolved) in ops.iter().enumerate() {
        let role = resolved.def.schema().matte;
        let paths_n = resolved.def.schema().mask_path_count();
        let mask_paths_of_op = mask_paths
            .get(path_i..(path_i + paths_n).min(mask_paths.len()))
            .unwrap_or(&[]);
        path_i += paths_n;
        let schedule = if lumit_core::fx::points::wants_carriage(resolved.def.signature()) {
            let slot = points_schedules.get(sched_i);
            sched_i += 1;
            slot
        } else {
            None
        };
        let matte = if role.param().is_some() {
            let slot = mattes.get(matte_i);
            matte_i += 1;
            slot
        } else {
            None
        };
        // The auxiliary layer's own counter, on the schema's own
        // `layer_input` predicate — the one `build.rs` fills by. It is read
        // here rather than inside the `AuxKind` match below because an effect
        // may want a layer input *and* something else: Motion blur reads a
        // whole flow field and a Motion vectors layer, and a variant per pair
        // is the combinatorial seam the matte was kept out of.
        let layer_input = if resolved.def.schema().layer_input().is_some() {
            let slot = layer_inputs.get(dof_i);
            dof_i += 1;
            slot
        } else {
            None
        };
        let roto = if resolved.def.schema().match_name == lumit_core::roto::ROTO_BRUSH {
            let slot = roto_mattes.get(roto_i).and_then(|o| o.as_ref());
            roto_i += 1;
            Some(slot)
        } else {
            None
        };
        let gpu = crate::gpufx::gpu_effect(resolved.def.schema().match_name);
        // An op whose output is already held: its counters still advance (the
        // lists are 1:1 with the ops, held or not; the matte, mask-path and
        // layer-input slots were read above), its timing is still pushed (the
        // profiler pairs the list with the stack's ids), and nothing else
        // happens.
        if i < start {
            match gpu.map(|g| g.aux()) {
                Some(AuxKind::Lut) => lut_i += 1,
                Some(AuxKind::LensFile) => flare_i += 1,
                _ => {}
            }
            // `roto_i` was advanced above with the other per-op slot reads, so
            // nothing more is owed here.
            if let Some(into) = timings.as_mut() {
                into.push(0.0);
            }
            continue;
        }
        // Only a bound matte costs anything: the input texture is held (a
        // cheap handle clone) so the dissolve below has something to lerp
        // back towards, and nothing at all happens when the row is unset.
        let matte = matte.and_then(|m| m.texture(&tex)).cloned();
        // The mattes and layer inputs were all rendered at the layer's own
        // raster, before the walk started. Once an op has grown it they
        // no longer line up with the picture, so they are grown into the same
        // margin — a no-op, and not even a copy, on every stack that has no
        // growing op in it.
        let matte = matte.map(|m| lumit_gpu::fx::fit_centred(ctx, m, w, h));
        // The Channel pick and Invert, once, before anyone reads the matte: a
        // bound matte on an effect that carries the injected Channel row is
        // rewritten to a grey of the chosen channel, inverted here and nowhere
        // else. Luminance with Invert off runs no pass, which is what keeps
        // that case byte for byte; an effect that owns its channel choice (no
        // Channel row) reads the raw RGBA as it always has, Invert included.
        let schema = resolved.def.schema();
        let matte = matte.map(|m| {
            let channel = resolved.params.choice(lumit_core::fx::MATTE_CHANNEL_ID, 0);
            let invert = resolved.params.bool(lumit_core::fx::MATTE_INVERT_ID, false);
            if schema.matte_channel() && lumit_core::fx::cpu::matte_needs_prepare(channel, invert) {
                fx.matte_prepare(ctx, &m, w, h, channel, invert)
            } else {
                m
            }
        });
        // **Who gets it** — the one branch. A generic effect's matte is
        // spent in the dissolve after the kernel, and must therefore not reach
        // the kernel; an override's is spent inside the kernel, and must
        // therefore not be dissolved again afterwards. Splitting the same
        // `Option` two ways here is what keeps that a single decision rather
        // than a rule each effect could disagree with.
        let (own_matte, generic_matte) = if role.generic() {
            (None, matte.as_ref())
        } else {
            (matte.as_ref(), None)
        };
        let mut unmatted_input = generic_matte.map(|_| tex.clone());
        // Rebound only when an op grows the raster, so that the dissolve
        // below reads a matte the same size as the picture it is dissolving.
        let mut grown_matte: Option<Tex> = None;
        // Only a *profiled* render reads a clock here, and it reads it either
        // side of a fence — see crate::profile on why an unfenced span would
        // time the paperwork rather than the work. One reading per op, whether
        // or not it draws, so the timings stay 1:1 with the resolve's own id
        // list (`resolve_stack_temporal_named`).
        let started = timings.as_ref().map(|_| std::time::Instant::now());
        // The parameters come out of the stack's arena and the kernel out of
        // the GPU table, looked up by the effect's own name. An op with no
        // table entry (an orchestration-only effect) passes the texture through
        // — the convention a missing LUT or flow field already uses, never a
        // fault (engine crates do not panic, 14-ENGINEERING-RULES §4).
        if let Some(gpu) = gpu {
            // The Blend row: anything but Normal runs the kernel at
            // Mix 100 and applies the blend and the Mix itself, afterwards and
            // once — the same decision `cpu::apply_stack` makes, read from
            // the same function. Normal leaves the kernel's own Mix alone and
            // runs no pass.
            let blend = lumit_core::fx::cpu::blend_seam(schema, resolved.params);
            let params = match &blend {
                Some((_, _, entries)) => lumit_core::fx::Params::new(entries),
                None => resolved.params,
            };
            let mut blend_input = blend.as_ref().map(|_| tex.clone());
            // As above: a Light wrap's background plate, sized to the
            // layer, grown into the margin an earlier op added.
            let fitted_layer_input = layer_input
                .and_then(|l| l.texture(&tex))
                .cloned()
                .map(|t| lumit_gpu::fx::fit_centred(ctx, t, w, h));
            let data = match gpu.aux() {
                AuxKind::None => AuxData::None,
                AuxKind::Lut => {
                    let slot = luts.get(lut_i).and_then(|o| o.as_ref());
                    lut_i += 1;
                    AuxData::Lut(slot)
                }
                AuxKind::LensFile => {
                    let lens = flare_lens.get(flare_i).and_then(|o| o.as_ref());
                    flare_i += 1;
                    AuxData::LensFile(lens)
                }
                AuxKind::Neighbours => AuxData::Neighbours(neighbours),
                // Which measurement this op reads is the effect's own, from the
                // one table in lumit-core that the decode worker also asks -
                // so the field an effect is handed is the one it asked
                // to have measured, and a stack with both consumers no longer
                // gives the second one whatever the first happened to want.
                AuxKind::FlowField => AuxData::FlowField {
                    field: lumit_core::fx::effect_flow_neighbour(gpu.match_name()).and_then(
                        |offset| {
                            flow_fields
                                .iter()
                                .find(|(o, _)| *o == offset)
                                .map(|(_, t)| t)
                        },
                    ),
                    neighbours,
                },
            };
            tex = gpu.run(
                fx,
                ctx,
                &tex,
                w,
                h,
                params,
                AuxSlot::new(
                    data,
                    own_matte,
                    matte.as_ref(),
                    fitted_layer_input.as_ref(),
                    mask_paths_of_op,
                    schedule,
                    resolved.instance,
                    resolved.lt,
                ),
            );
            // A grown raster. The two passes below and every op after
            // this one read texel by texel, so the pictures they compare against
            // — the input the Blend row lerps from, the input the generic matte
            // dissolves back to, the matte itself — are grown into the same
            // margin, transparently. `fit_centred` returns its argument when the
            // size already matches, so nothing at all happens on the ordinary
            // path and the picture stays byte for byte what it was.
            if tex.width() != w || tex.height() != h {
                w = tex.width();
                h = tex.height();
                blend_input = blend_input.map(|t| lumit_gpu::fx::fit_centred(ctx, t, w, h));
                unmatted_input = unmatted_input.map(|t| lumit_gpu::fx::fit_centred(ctx, t, w, h));
                grown_matte = matte
                    .clone()
                    .map(|t| lumit_gpu::fx::fit_centred(ctx, t, w, h));
            }
            if let (Some((mode, mix, _)), Some(input)) = (blend, blend_input) {
                tex = fx.blend_mix(ctx, &input, &tex, w, h, mode, mix);
            }
        }

        // **The Roto brush** (docs/impl/roto.md §5). It has no entry in
        // the GPU table — there is no kernel to write — because what it does is
        // Set matte's arithmetic over a picture nobody picked: multiply this
        // layer's alpha by the propagation's matte, leaving the colour alone
        // (§2.2's straight-alpha rule, which `set_matte` already fuses into its
        // one pass). Matte mode inverts it; the Matte view shows the matte
        // itself, because that is how a matte is judged; Boundary keeps the
        // picture for the viewer's overlay to draw an edge over, and at the
        // stack seam is Result.
        //
        // An **empty slot passes through**, running no pass at all: outside the
        // propagated span there is no matte, and holding a neighbour's would be
        // a wrong answer wearing a right one's face.
        if let Some(Some(plane)) = roto {
            let plane = lumit_gpu::fx::fit_centred(ctx, plane.clone(), w, h);
            let view = resolved
                .params
                .choice(lumit_core::fx::effects::roto_brush::VIEW_ID, 0);
            let invert = resolved
                .params
                .choice(lumit_core::fx::effects::roto_brush::MODE_ID, 0)
                == 1;
            tex = if view == 1 {
                // The matte itself, as an opaque grey picture.
                fx.set_matte(
                    ctx,
                    &plane,
                    w,
                    h,
                    None,
                    &lumit_gpu::fx::SetMatteOp {
                        channel: 0,
                        combine: false,
                        invert,
                        mix: 1.0,
                    },
                )
            } else {
                fx.set_matte(
                    ctx,
                    &tex,
                    w,
                    h,
                    Some(&plane),
                    &lumit_gpu::fx::SetMatteOp {
                        channel: 0,
                        // Intersect with the layer's own alpha rather than
                        // replace it: a matte says which of *this* picture
                        // to keep, and a layer that was already partly
                        // transparent stays so.
                        combine: true,
                        invert,
                        mix: 1.0,
                    },
                )
            };
        }

        // The generic strength semantic, one implementation for every
        // effect that has not claimed the matte for itself: after the effect's
        // own Mix (inside its kernel, or the blend pass above), dissolve back
        // to the picture it was handed, by the matte's luma. Invert is passed
        // only when the prepare pass above did not already apply it — an
        // effect with no Channel row — so it is applied exactly once.
        if let (Some(m), Some(input)) = (grown_matte.as_ref().or(generic_matte), unmatted_input) {
            let invert = !schema.matte_channel()
                && resolved.params.bool(lumit_core::fx::MATTE_INVERT_ID, false);
            tex = fx.matte_mix(ctx, &input, &tex, m, w, h, invert);
        }

        if let Some(key) = keys.get(i).copied().flatten() {
            made.push((key, tex.clone()));
        }

        if let (Some(started), Some(into)) = (started, timings.as_mut()) {
            // Same reason as the per-layer fence in `realise`: a frame's
            // commands are batched, and timing a queue that has not been
            // handed over would measure nothing. A measured frame gives the
            // batching up, effect by effect.
            ctx.flush();
            ctx.device.poll(wgpu::Maintain::Wait);
            into.push(started.elapsed().as_secs_f32() * 1000.0);
        }
    }
    if let Some((store, _)) = cache {
        let mut store = store.borrow_mut();
        store.runs += ops.len().saturating_sub(start) as u64;
        if store.keep && fx.flare_substitutions() == subs_before {
            for (key, out) in made {
                store.lru.insert(key, CachedTex(out));
            }
        }
    }
    tex
}

/// The content name of each op's output: `keys[k]` covers the input,
/// the raster, the flare substitution count and ops `0..=k`, with
/// each op's parameters and the identity of whatever rides beside it. `None`
/// from the first op that binds a picture nobody named — another layer (a
/// plate or a matte texture), the neighbour frames, the flow field — through
/// to the end: the v1 rule is that such an op breaks the chain rather than
/// being named by a guess.
/// [`LayerInput::ThisLayer`] and [`LayerInput::Absent`] are functions of the
/// chain itself, so they do not break it.
#[allow(clippy::too_many_arguments)]
fn op_keys(
    input_key: u128,
    w: u32,
    h_px: u32,
    flare_substitutions: u64,
    ops: &lumit_core::fx::ResolvedStack,
    luts: &[Option<LoadedLut>],
    layer_inputs: &[LayerInput],
    flare_lens: &[Option<(u64, String)>],
    mattes: &[LayerInput],
    mask_paths: &[lumit_core::mask::MaskPolyline],
    points_schedules: &[lumit_core::fx::points::PointsSchedule],
) -> Vec<Option<u128>> {
    let mut h = blake3::Hasher::new();
    h.update(b"fxcache/1/");
    h.update(&input_key.to_le_bytes());
    h.update(&w.to_le_bytes());
    h.update(&h_px.to_le_bytes());
    h.update(&flare_substitutions.to_le_bytes());
    let (mut lut_i, mut dof_i, mut flare_i, mut matte_i, mut path_i) = (0, 0, 0, 0, 0);
    let mut sched_i = 0usize;
    let mut broken = false;
    ops.iter()
        .map(|resolved| {
            let schema = resolved.def.schema();
            resolved.feed_hash(&mut |b| {
                h.update(b);
            });
            for _ in 0..schema.mask_path_count() {
                if let Some(p) = mask_paths.get(path_i) {
                    h.update(&[u8::from(p.closed)]);
                    h.update(&p.feather.to_le_bytes());
                    h.update(&p.expansion.to_le_bytes());
                    for pt in &p.points {
                        h.update(&pt[0].to_le_bytes());
                        h.update(&pt[1].to_le_bytes());
                    }
                }
                path_i += 1;
            }
            // The schedule names this op's output as surely as its parameters
            // do: it carries the layer's clock and the whole history of the
            // Emit rate track, neither of which is in the bag `feed_hash`
            // walked. Without it a scrub would be answered from the cache with
            // the previous frame's particles.
            if lumit_core::fx::points::wants_carriage(resolved.def.signature()) {
                if let Some(sched) = points_schedules.get(sched_i) {
                    h.update(&sched.t.to_le_bytes());
                    h.update(&sched.schedule.dt().to_le_bytes());
                    h.update(&sched.schedule.first_frame().to_le_bytes());
                    h.update(&sched.schedule.first_birth().to_le_bytes());
                    for n in sched.schedule.counts() {
                        h.update(&n.to_le_bytes());
                    }
                    // And the camera it draws its third axis through,
                    // for exactly the same reason: move the comp's camera and
                    // this op's picture moves, while nothing `feed_hash`
                    // walked has changed at all.
                    h.update(&[u8::from(sched.projection.is_some())]);
                    // **And the wire**. The stream itself is not hashed
                    // and must not be — it is a pure function of the producer's
                    // bag, the time and the camera, and the producer sits
                    // strictly earlier in this stack, so it is already
                    // inside this op's cumulative key. What is *not* is which
                    // producer the wire names, or whether one is drawn at all:
                    // cut the wire and nothing in the consumer's own bag moves.
                    h.update(&sched.input_from.unwrap_or(u32::MAX).to_le_bytes());
                    h.update(&(sched.input.len() as u32).to_le_bytes());
                    for row in sched.projection.unwrap_or_default().m {
                        for v in row {
                            h.update(&v.to_le_bytes());
                        }
                    }
                }
                sched_i += 1;
            }
            if schema.matte.param().is_some() {
                if let Some(LayerInput::Texture(_)) = mattes.get(matte_i) {
                    broken = true;
                }
                matte_i += 1;
            }
            // The layer-input list moved onto the schema's own predicate when
            // Motion blur gained its Motion vectors row: an op
            // may take a layer input beside any other aux, so it is no longer
            // an AuxKind. A bound plate is a picture nobody named - it breaks
            // the chain exactly as a bound matte does.
            if schema.layer_input().is_some() {
                if let Some(LayerInput::Texture(_)) = layer_inputs.get(dof_i) {
                    broken = true;
                }
                dof_i += 1;
            }
            match crate::gpufx::gpu_effect(schema.match_name).map(|g| g.aux()) {
                Some(AuxKind::Lut) => {
                    if let Some(Some(lut)) = luts.get(lut_i) {
                        h.update(lut.path.as_bytes());
                        let mtime = lut
                            .mtime
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map_or(0, |d| d.as_nanos());
                        h.update(&mtime.to_le_bytes());
                    }
                    lut_i += 1;
                }
                Some(AuxKind::LensFile) => {
                    if let Some(Some((hash, _))) = flare_lens.get(flare_i) {
                        h.update(&hash.to_le_bytes());
                    }
                    flare_i += 1;
                }
                Some(AuxKind::Neighbours | AuxKind::FlowField) => broken = true,
                Some(AuxKind::None) | None => {}
            }
            if broken {
                return None;
            }
            let mut k = [0u8; 16];
            k.copy_from_slice(&h.clone().finalize().as_bytes()[..16]);
            Some(u128::from_le_bytes(k))
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A tiny valid `.cube` of `size`, with an optional declared domain.
    fn cube_text(size: usize, domain: Option<([f32; 3], [f32; 3])>) -> String {
        let mut s = format!("LUT_3D_SIZE {size}\n");
        if let Some((lo, hi)) = domain {
            s += &format!("DOMAIN_MIN {} {} {}\n", lo[0], lo[1], lo[2]);
            s += &format!("DOMAIN_MAX {} {} {}\n", hi[0], hi[1], hi[2]);
        }
        let maxf = (size - 1) as f32;
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    s += &format!(
                        "{} {} {}\n",
                        r as f32 / maxf,
                        g as f32 / maxf,
                        b as f32 / maxf
                    );
                }
            }
        }
        s
    }

    /// **A `.cube` edited on disk must be re-read** (docs/impl/lut.md §4).
    ///
    /// The regression: the cache keyed by path alone, so exporting a new grade
    /// over the same filename — the whole loop of grading — kept showing the
    /// first one until the application was restarted, with nothing on screen to
    /// say the file on disk and the picture had parted company.
    #[test]
    fn an_edited_cube_is_read_again() {
        let Some(ctx) = lumit_gpu::test_support::lease() else {
            lumit_gpu::no_adapter();
            return;
        };
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("grade.cube");
        std::fs::write(&path, cube_text(2, None)).expect("written");
        let name = path.to_string_lossy().to_string();

        let mut cache = LutCache::default();
        let first = cache.get_or_load(&ctx, &name).expect("loaded");
        assert_eq!(first.size, 2);
        assert_eq!(cache.len(), 1);

        // Re-asking without touching the file is a cache hit: still one entry,
        // still the cube that was parsed the first time.
        let again = cache.get_or_load(&ctx, &name).expect("loaded");
        assert_eq!(cache.len(), 1, "an untouched file is not read twice");
        assert_eq!(again.size, first.size);

        // Now edit it. The sleep is the point of the test, not laziness: two
        // writes inside one filesystem timestamp tick are indistinguishable,
        // and the cache is keyed on that tick.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, cube_text(3, None)).expect("re-written");
        let edited = cache.get_or_load(&ctx, &name).expect("loaded");
        assert_eq!(edited.size, 3, "the new grade is the one that renders");
        assert_eq!(
            cache.len(),
            1,
            "and the stale entry is replaced, not kept beside it"
        );
    }

    /// The declared domain travels with the cube, so the kernel can remap
    /// through it exactly as the CPU oracle does. A default-domain file
    /// still reads 0..1.
    #[test]
    fn the_declared_domain_travels_with_the_cube() {
        let Some(ctx) = lumit_gpu::test_support::lease() else {
            lumit_gpu::no_adapter();
            return;
        };
        let dir = tempfile::tempdir().expect("a temp dir");
        let mut cache = LutCache::default();

        let plain = dir.path().join("plain.cube");
        std::fs::write(&plain, cube_text(2, None)).expect("written");
        let loaded = cache
            .get_or_load(&ctx, &plain.to_string_lossy())
            .expect("loaded");
        assert_eq!(loaded.domain_min, [0.0; 3]);
        assert_eq!(loaded.domain_max, [1.0; 3]);

        let log = dir.path().join("log.cube");
        std::fs::write(
            &log,
            cube_text(2, Some(([-0.25, 0.0, 0.1], [1.5, 0.75, 1.0]))),
        )
        .expect("written");
        let loaded = cache
            .get_or_load(&ctx, &log.to_string_lossy())
            .expect("loaded");
        assert_eq!(loaded.domain_min, [-0.25, 0.0, 0.1]);
        assert_eq!(loaded.domain_max, [1.5, 0.75, 1.0]);
    }

    /// The cache is bounded and evicts the least recently used, so a long
    /// session that touches many `.cube` files does not accumulate uploads
    /// (docs/impl/lut.md §4). Re-asking for an old file *keeps it alive*, which
    /// is what "least recently used" has to mean.
    #[test]
    fn the_cache_is_bounded_and_keeps_what_is_used() {
        let Some(ctx) = lumit_gpu::test_support::lease() else {
            lumit_gpu::no_adapter();
            return;
        };
        let dir = tempfile::tempdir().expect("a temp dir");
        let mut cache = LutCache::default();
        let path = |i: usize| dir.path().join(format!("cube{i}.cube"));
        for i in 0..=LUT_CACHE_MAX {
            std::fs::write(path(i), cube_text(2, None)).expect("written");
        }

        // Fill it exactly, then keep the first one fresh by asking for it again.
        for i in 0..LUT_CACHE_MAX {
            cache
                .get_or_load(&ctx, &path(i).to_string_lossy())
                .expect("loaded");
        }
        assert_eq!(cache.len(), LUT_CACHE_MAX);
        cache
            .get_or_load(&ctx, &path(0).to_string_lossy())
            .expect("still there");

        // One more file evicts the least recently used — which is now cube1,
        // not cube0.
        cache
            .get_or_load(&ctx, &path(LUT_CACHE_MAX).to_string_lossy())
            .expect("loaded");
        assert_eq!(cache.len(), LUT_CACHE_MAX, "the bound holds");
        assert!(
            cache
                .entries
                .iter()
                .any(|(p, _, _)| p == &*path(0).to_string_lossy()),
            "the one that was used again survived"
        );
        assert!(
            !cache
                .entries
                .iter()
                .any(|(p, _, _)| p == &*path(1).to_string_lossy()),
            "and the least recently used went"
        );
    }

    // ----- The per-effect cache -----

    /// A stack of built-in effects with one float parameter set on each:
    /// `(match_name, param id, value)`.
    fn stack(spec: &[(&str, &str, f32)]) -> lumit_core::fx::ResolvedStack {
        let insts: Vec<_> = spec
            .iter()
            .map(|(name, param, value)| {
                let mut inst = lumit_core::fx::instantiate(name).expect("a built-in");
                for p in &mut inst.params {
                    if p.id == *param {
                        p.value = lumit_core::model::EffectValue::Float(
                            lumit_core::anim::Property::fixed(f64::from(*value)),
                        );
                    }
                }
                inst
            })
            .collect();
        lumit_core::fx::resolve_stack(
            &insts,
            0.0,
            1000.0,
            1.0,
            &lumit_core::fx::MarkerContext::NONE,
            std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
        )
    }

    const W: u32 = 8;
    const H: u32 = 8;

    fn source(ctx: &GpuContext) -> Tex {
        let px: Vec<f32> = (0..(W * H * 4))
            .map(|i| match i % 4 {
                3 => 1.0,
                _ => (i % 13) as f32 / 13.0,
            })
            .collect();
        lumit_gpu::fx::upload_linear_f32(ctx, &px, W, H)
    }

    /// `run_ops` with every side list empty, through `cache` under `key`.
    fn run(
        fx: &FxEngine,
        ctx: &GpuContext,
        ops: &lumit_core::fx::ResolvedStack,
        cache: &std::cell::RefCell<FxCache>,
        key: u128,
    ) -> Vec<f32> {
        let out = run_ops(
            fx,
            ctx,
            source(ctx),
            W,
            H,
            ops,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            None,
            Some((cache, key)),
        );
        lumit_gpu::fx::readback_linear_f32(ctx, &out, W, H).expect("readback")
    }

    fn warm_cache() -> std::cell::RefCell<FxCache> {
        let mut c = FxCache::default();
        c.keep_outputs(true);
        std::cell::RefCell::new(c)
    }

    /// One custom-shader instance holding `source`, resolved as the walk
    /// resolves it.
    fn shader_stack(source: &str) -> lumit_core::fx::ResolvedStack {
        let mut inst = lumit_core::fx::instantiate("custom_shader").expect("a built-in");
        let mut block = serde_json::Map::new();
        block.insert("source".into(), source.into());
        inst.extra
            .insert("shader".into(), serde_json::Value::Object(block));
        lumit_core::fx::resolve_stack(
            std::slice::from_ref(&inst),
            0.0,
            1000.0,
            1.0,
            &lumit_core::fx::MarkerContext::NONE,
            std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
        )
    }

    /// The whole carriage, end to end: the document holds a shader, the
    /// resolve walk reads it and puts its name in the bag, and the picture that
    /// comes out is the one the shader describes — with no parallel list, no new
    /// aux kind and nothing owned in the arena.
    #[test]
    fn a_custom_shader_in_a_stack_draws_what_its_text_says() {
        let Some(ctx) = lumit_gpu::test_support::lease() else {
            lumit_gpu::no_adapter();
            return;
        };
        let fx = ctx.fx();
        let plain = run(fx, &ctx, &shader_stack(""), &warm_cache(), 7);
        let before = lumit_gpu::fx::readback_linear_f32(&ctx, &source(&ctx), W, H).expect("read");
        assert_eq!(
            plain, before,
            "an instance with no program renders its input unchanged"
        );

        // Half the colour, alpha untouched.
        let halved = run(
            fx,
            &ctx,
            &shader_stack(
                "fn shade(uv: vec2<f32>) -> vec4<f32> {
                     let c = lumit_sample(uv);
                     return vec4<f32>(c.rgb * 0.5, c.a);
                 }
",
            ),
            &warm_cache(),
            7,
        );
        for i in (0..halved.len()).step_by(4) {
            assert!(
                (halved[i] - before[i] * 0.5).abs() < 2e-3,
                "pixel {i}: {} vs {}",
                halved[i],
                before[i] * 0.5
            );
            assert!((halved[i + 3] - before[i + 3]).abs() < 2e-3);
        }
    }

    /// The source is in the per-effect cache key, because it is in the bag:
    /// editing a shader must rename its output, or the walk would serve the
    /// previous shader's picture out of the intermediate cache with nothing to
    /// say anything was wrong.
    #[test]
    fn editing_a_shaders_source_renames_its_output() {
        let Some(ctx) = lumit_gpu::test_support::lease() else {
            lumit_gpu::no_adapter();
            return;
        };
        let fx = ctx.fx();
        let cache = warm_cache();
        let half = "fn shade(uv: vec2<f32>) -> vec4<f32> { let c = lumit_sample(uv);                     return vec4<f32>(c.rgb * 0.5, c.a); }";
        let quarter = "fn shade(uv: vec2<f32>) -> vec4<f32> { let c = lumit_sample(uv);                        return vec4<f32>(c.rgb * 0.25, c.a); }";
        let first = run(fx, &ctx, &shader_stack(half), &cache, 7);
        assert_eq!(cache.borrow().counts(), (1, 0), "a cold walk runs it");
        let again = run(fx, &ctx, &shader_stack(half), &cache, 7);
        assert_eq!(
            cache.borrow().counts().1,
            1,
            "the same source is the same picture, served from the cache"
        );
        assert_eq!(first, again);
        let edited = run(fx, &ctx, &shader_stack(quarter), &cache, 7);
        assert_eq!(
            cache.borrow().counts().1,
            1,
            "and an edited one is a miss, not a stale hit"
        );
        assert_ne!(first, edited, "which is what makes the picture right");
    }

    /// **Editing the last effect re-runs only that one** — and the
    /// picture is the one a cold walk makes.
    #[test]
    fn editing_the_last_effect_reruns_only_that_one() {
        let Some(ctx) = lumit_gpu::test_support::lease() else {
            lumit_gpu::no_adapter();
            return;
        };
        let fx = ctx.fx();
        let cache = warm_cache();

        let first = stack(&[
            ("saturation", "saturation", 20.0),
            ("exposure", "stops", 1.0),
        ]);
        run(fx, &ctx, &first, &cache, 7);
        assert_eq!(cache.borrow().counts(), (2, 0), "a cold walk runs both");
        assert_eq!(cache.borrow().stats().2, 2, "and files both outputs");

        let edited = stack(&[
            ("saturation", "saturation", 20.0),
            ("exposure", "stops", 2.0),
        ]);
        let warm = run(fx, &ctx, &edited, &cache, 7);
        assert_eq!(
            cache.borrow().counts(),
            (3, 1),
            "the saturation's output was held; only the exposure ran"
        );

        let cold = run(fx, &ctx, &edited, &warm_cache(), 7);
        assert_eq!(
            warm, cold,
            "a held prefix makes the same picture as a cold walk"
        );
    }

    /// An upstream edit renames everything after it: both ops run again.
    #[test]
    fn an_upstream_edit_misses() {
        let Some(ctx) = lumit_gpu::test_support::lease() else {
            lumit_gpu::no_adapter();
            return;
        };
        let fx = ctx.fx();
        let cache = warm_cache();
        run(
            fx,
            &ctx,
            &stack(&[
                ("saturation", "saturation", 20.0),
                ("exposure", "stops", 1.0),
            ]),
            &cache,
            7,
        );
        run(
            fx,
            &ctx,
            &stack(&[
                ("saturation", "saturation", 50.0),
                ("exposure", "stops", 1.0),
            ]),
            &cache,
            7,
        );
        assert_eq!(cache.borrow().counts(), (4, 0));
        // A different source under the same stack is a miss too.
        run(
            fx,
            &ctx,
            &stack(&[
                ("saturation", "saturation", 50.0),
                ("exposure", "stops", 1.0),
            ]),
            &cache,
            8,
        );
        assert_eq!(cache.borrow().counts(), (6, 0));
    }

    /// **The ops after a growing one run on the wider raster** (docs/08
    /// §3.39). Tile above 100 % output is the only effect that can grow the
    /// working picture; what makes that worth having is that the effects below
    /// it in the stack then see the copies as picture rather than as
    /// transparency. Here an Exposure after the Tile brightens the margin as
    /// well as the frame, which it could only do if it was dispatched at the
    /// grown size.
    #[test]
    fn the_ops_after_a_growing_one_run_on_the_wider_raster() {
        use lumit_core::fx::{effects, Value};
        let Some(ctx) = lumit_gpu::test_support::lease() else {
            lumit_gpu::no_adapter();
            return;
        };
        let fx = ctx.fx();

        let mut ops = lumit_core::fx::ResolvedStack::new();
        ops.begin(&effects::tile::TileDef, uuid::Uuid::now_v7());
        ops.push(effects::tile::Tile::TILE_CENTRE_X, Value::Float(4.0));
        ops.push(effects::tile::Tile::TILE_CENTRE_Y, Value::Float(4.0));
        // The four sizes are px@comp: one whole-frame tile, stamped
        // over twice the frame.
        ops.push(effects::tile::Tile::TILE_WIDTH, Value::Float(W as f32));
        ops.push(effects::tile::Tile::TILE_HEIGHT, Value::Float(H as f32));
        ops.push(
            effects::tile::Tile::OUTPUT_WIDTH,
            Value::Float((W * 2) as f32),
        );
        ops.push(
            effects::tile::Tile::OUTPUT_HEIGHT,
            Value::Float((H * 2) as f32),
        );
        ops.push(effects::tile::Tile::MIX, Value::Float(100.0));
        ops.begin(&effects::exposure::ExposureDef, uuid::Uuid::now_v7());
        ops.push(effects::exposure::Exposure::STOPS, Value::Float(1.0));
        ops.push(effects::exposure::Exposure::MIX, Value::Float(100.0));

        let out = run_ops(
            fx,
            &ctx,
            source(&ctx),
            W,
            H,
            &ops,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            None,
            None,
        );
        assert_eq!(
            (out.width(), out.height()),
            (W * 2, H * 2),
            "the stack must hand back the raster Tile grew"
        );
        let grown = lumit_gpu::fx::readback_linear_f32(&ctx, &out, W * 2, H * 2).expect("readback");
        let flat = lumit_gpu::fx::readback_linear_f32(&ctx, &source(&ctx), W, H).expect("readback");
        // The margin: brightened copies, not the transparency a layer's edge
        // used to be past its own raster.
        let corner = 0usize;
        assert!(
            grown[corner + 3] > 0.5,
            "the margin must hold picture, alpha {}",
            grown[corner + 3]
        );
        // And the second op really ran there: +1 stop is a factor of two on the
        // colour channels, which the source's own texel says.
        let mid = (((H) * W * 2 + W) * 4) as usize;
        let src = 0usize;
        assert!(
            grown[mid] > flat[src] || grown[mid + 1] > flat[src + 1],
            "the Exposure after the Tile must have run on the grown raster"
        );
    }

    /// An op that binds a picture nobody named — another layer's texture as a
    /// matte, the neighbour frames — breaks the chain: nothing from it on is
    /// filed, and the same stack runs in full again.
    #[test]
    fn a_bound_picture_breaks_the_chain() {
        let Some(ctx) = lumit_gpu::test_support::lease() else {
            lumit_gpu::no_adapter();
            return;
        };
        let fx = ctx.fx();

        // A matte texture on the first op.
        let cache = warm_cache();
        let ops = stack(&[
            ("saturation", "saturation", 20.0),
            ("exposure", "stops", 1.0),
        ]);
        for _ in 0..2 {
            run_ops(
                fx,
                &ctx,
                source(&ctx),
                W,
                H,
                &ops,
                &[],
                &[],
                &[],
                &[],
                &[],
                &[LayerInput::Texture(source(&ctx)), LayerInput::Absent],
                &[],
                &[],
                None,
                Some((&cache, 7)),
            );
        }
        assert_eq!(
            cache.borrow().counts(),
            (4, 0),
            "nothing after a matte is held"
        );
        assert_eq!(cache.borrow().stats().2, 0);

        // The matte on the *second* op: the first op's output is still named.
        let cache = warm_cache();
        for _ in 0..2 {
            run_ops(
                fx,
                &ctx,
                source(&ctx),
                W,
                H,
                &ops,
                &[],
                &[],
                &[],
                &[],
                &[],
                &[LayerInput::Absent, LayerInput::Texture(source(&ctx))],
                &[],
                &[],
                None,
                Some((&cache, 7)),
            );
        }
        assert_eq!(
            cache.borrow().counts(),
            (3, 1),
            "the chain holds up to the matte"
        );

        // Neighbour frames (Echo) break it the same way.
        let cache = warm_cache();
        let ops = stack(&[("echo", "decay", 0.5), ("exposure", "stops", 1.0)]);
        for _ in 0..2 {
            run_ops(
                fx,
                &ctx,
                source(&ctx),
                W,
                H,
                &ops,
                &[(-1, source(&ctx))],
                &[],
                &[],
                &[],
                &[],
                &[],
                &[],
                &[],
                None,
                Some((&cache, 7)),
            );
        }
        assert_eq!(cache.borrow().counts(), (4, 0));
        assert_eq!(cache.borrow().stats().2, 0);
    }

    /// A `.cube` edited on disk is a different `lut` op: its mtime is in the
    /// name (as it is in the LUT cache's).
    #[test]
    fn a_lut_edited_on_disk_misses() {
        let Some(ctx) = lumit_gpu::test_support::lease() else {
            lumit_gpu::no_adapter();
            return;
        };
        let fx = ctx.fx();
        let cache = warm_cache();
        let lut = |mtime: u64| {
            let cube = vec![[1.0f32, 0.0, 0.0]; 8];
            Some(LoadedLut {
                texture: lumit_gpu::fx::upload_lut_3d(&ctx, 2, &cube),
                size: 2,
                domain_min: [0.0; 3],
                domain_max: [1.0; 3],
                path: "grade.cube".into(),
                mtime: Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(mtime)),
            })
        };
        let ops = stack(&[("lut", "mix", 100.0), ("exposure", "stops", 1.0)]);
        for mtime in [1, 1, 2] {
            run_ops(
                fx,
                &ctx,
                source(&ctx),
                W,
                H,
                &ops,
                &[],
                &[],
                &[lut(mtime)],
                &[],
                &[],
                &[],
                &[],
                &[],
                None,
                Some((&cache, 7)),
            );
        }
        assert_eq!(
            cache.borrow().counts(),
            (4, 2),
            "same file twice is a hit; the edited file runs both again"
        );
    }

    /// A bake merely being **made** renames nothing.
    ///
    /// Op outputs used to carry the flare bake *generation*, which moves the
    /// moment any bake is queued — so with a keyframed aperture keeping one
    /// queued, every walk of every stack in the project threw away its
    /// predecessor's outputs and re-ran every op. What belongs in the name is
    /// whether a flare actually stood other optics in, which is counted.
    #[test]
    fn a_bake_in_flight_does_not_rename_every_op() {
        let Some(ctx) = lumit_gpu::test_support::lease() else {
            lumit_gpu::no_adapter();
            return;
        };
        let fx = ctx.fx();
        let cache = warm_cache();
        let ops = stack(&[
            ("saturation", "saturation", 20.0),
            ("exposure", "stops", 1.0),
        ]);
        run(fx, &ctx, &ops, &cache, 7);
        run(fx, &ctx, &ops, &cache, 7);
        assert_eq!(cache.borrow().counts(), (2, 2));

        fx.set_deferred_flare_bakes(true);
        let bake = std::sync::Arc::new(|| lumit_gpu::fx::FlareBakeData {
            surfaces: Vec::new(),
            ghosts: Vec::new(),
            spreads: Vec::new(),
            sensor_z_mm: 0.0,
            focal_mm: 1.0,
            native_fstop: 1.0,
            pupil_mm: 1.0,
            start_z_mm: 0.0,
            energy_gain: 1.0,
            reflectance: Vec::new(),
            starburst: Vec::new(),
            sb_res: 1,
            sb_fields: 1,
        }) as lumit_gpu::fx::FlareBake;
        if !fx.warm_flare_bake(0xfeed_face, &bake) {
            return; // no bake thread on this machine
        }
        run(fx, &ctx, &ops, &cache, 7);
        assert_eq!(
            cache.borrow().counts(),
            (2, 4),
            "a bake in flight leaves every op's name — and so every held output — alone"
        );
        assert_eq!(
            fx.flare_substitutions(),
            0,
            "and nothing was stood in for: this stack holds no flare"
        );
    }

    /// The budget holds, and holds the most recent: with room for one output,
    /// the last op's survives, and the next identical walk starts after it.
    #[test]
    fn the_budget_evicts_the_oldest_output() {
        let Some(ctx) = lumit_gpu::test_support::lease() else {
            lumit_gpu::no_adapter();
            return;
        };
        let fx = ctx.fx();
        let mut c = FxCache::new((W * H * 8) as usize);
        c.keep_outputs(true);
        let cache = std::cell::RefCell::new(c);
        let ops = stack(&[
            ("saturation", "saturation", 20.0),
            ("exposure", "stops", 1.0),
        ]);
        run(fx, &ctx, &ops, &cache, 7);
        assert_eq!(cache.borrow().stats().2, 1, "one output fits");
        run(fx, &ctx, &ops, &cache, 7);
        assert_eq!(cache.borrow().counts(), (2, 2), "and it is the last one");

        cache.borrow_mut().set_budget(0);
        assert_eq!(cache.borrow().stats().2, 0);
        run(fx, &ctx, &ops, &cache, 7);
        assert_eq!(cache.borrow().stats().2, 0, "nothing fits in no budget");
    }

    /// A render that is not committed — a drag, playback — reads the cache
    /// and never adds to it.
    #[test]
    fn an_uncommitted_render_reads_but_never_writes() {
        let Some(ctx) = lumit_gpu::test_support::lease() else {
            lumit_gpu::no_adapter();
            return;
        };
        let fx = ctx.fx();
        let cache = std::cell::RefCell::new(FxCache::default());
        let ops = stack(&[
            ("saturation", "saturation", 20.0),
            ("exposure", "stops", 1.0),
        ]);
        run(fx, &ctx, &ops, &cache, 7);
        assert_eq!(cache.borrow().stats().2, 0);

        cache.borrow_mut().keep_outputs(true);
        run(fx, &ctx, &ops, &cache, 7);
        cache.borrow_mut().keep_outputs(false);
        let dragged = stack(&[
            ("saturation", "saturation", 20.0),
            ("exposure", "stops", 3.0),
        ]);
        run(fx, &ctx, &dragged, &cache, 7);
        assert_eq!(
            cache.borrow().counts(),
            (5, 1),
            "the drag was served the prefix"
        );
        assert_eq!(cache.borrow().stats().2, 2, "and filed nothing of its own");
    }
}
