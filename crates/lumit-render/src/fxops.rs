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
    /// `DOMAIN_MIN` / `DOMAIN_MAX` from the file (default `0..1`), carried to
    /// the kernel so the GPU remaps exactly as the CPU oracle does (K-271).
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
/// nothing on screen to say so (K-271). A file whose mtime has moved is a
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

/// One layer-input slot as [`run_ops`] receives it — the realised twin of
/// [`crate::draw::LayerInputDraw`] (docs/impl/layer-input.md, K-288).
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
/// byte-identical in the viewport and the file (K-031) — exactly as
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
/// reads them, single-frame ops ignore them. `flow_field` is the layer's
/// dense motion field (per-pixel `(u, v)` at this raster size), present only
/// when the stack has a flow-consuming effect (Flow motion blur, or Datamosh
/// — §3.12, K-107); a missing field makes that effect a passthrough
/// (degrade, never fault). `luts` is the parallel LUT list (docs/08 §3.11): the
/// k-th `lut` op binds `luts[k]` — a `None` slot (unset, missing, 1D or
/// unreadable file) is a passthrough, exactly like a missing flow field.
/// `layer_inputs` is the parallel layer-input list (docs/08 §3.28,
/// docs/impl/layer-input.md): the k-th layer-input-consuming op — `light_wrap`
/// alone, since K-395 moved Depth of field's depth pass onto the matte list —
/// binds `layer_inputs[k]`, the referenced layer rendered alone at comp size,
/// [`LayerInput::ThisLayer`] for the effect's own input (K-288), or
/// [`LayerInput::Absent`] (unset, missing or cyclic) for a passthrough, exactly
/// like a missing LUT. `flare_lens` is the parallel custom-prescription list
/// (K-264, `lens_file` as content hash + text; None = use the picked library
/// lens): the k-th `lens_flare` op binds `flare_lens[k]`.
///
/// `mattes` is the parallel **Matte** list (K-395, docs/08 §2.6): one slot per
/// op whose effect declares any [`MatteRole`](lumit_core::fx::MatteRole) with a
/// parameter — which is every op — exactly as `build.rs`'s `mattes_for`
/// enumerates them. What a bound slot *does* is the role's second question, and
/// the only branch in this function that cares: an effect on the generic
/// strength semantic gets the dissolve below, and an effect that claims the
/// matte inside its own maths (Gaussian blur, Glow, Depth of field, the Lens
/// flare) gets the texture handed to its kernel instead — never both, or the
/// matte would be applied twice. An absent slot (the default, and every project
/// saved before K-395) runs no extra pass at all, so the picture is
/// byte-for-byte what it was (K-258).
///
/// `mask_paths` is the parallel **mask-path** list (K-408, docs/08 §1.2): one
/// flattened polyline per op whose effect declares a
/// [`ParamKind::MaskPath`](lumit_core::fx::ParamKind::MaskPath) row, exactly as
/// `build.rs`'s `mask_paths_for` enumerates them. Its own counter, not the
/// matte's: every op takes a matte and almost none takes a path, so one shared
/// index would hand a path to whichever effect happened to sit above. An empty
/// polyline is the effect's documented no-op.
#[allow(clippy::too_many_arguments)]
pub fn run_ops(
    fx: &FxEngine,
    ctx: &GpuContext,
    tex: Tex,
    w: u32,
    h: u32,
    ops: &lumit_core::fx::ResolvedStack,
    neighbours: &[(i32, Tex)],
    flow_field: Option<&Tex>,
    luts: &[Option<LoadedLut>],
    layer_inputs: &[LayerInput],
    flare_lens: &[Option<(u64, String)>],
    mattes: &[LayerInput],
    mask_paths: &[lumit_core::mask::MaskPolyline],
    mut timings: Option<&mut Vec<f32>>,
) -> Tex {
    let mut tex = tex;
    // The k-th `lut` op consumes the k-th `luts` slot (the whole threading
    // contract — see `build.rs`'s `lut_files` and CompLayerDraw's lut_files); a
    // slot is present only when its `.cube` file loaded. The k-th
    // layer-input-consuming op consumes the k-th `layer_inputs` slot the same
    // way. Both share one counter because `build.rs`'s `layer_inputs_for`
    // enumerates them with one predicate, in one order; two counters would let
    // the two sides drift apart silently.
    //
    // An effect names its list rather than carrying an arm of its own
    // (`GpuEffect::aux`, K-387), and the loop below advances the counter it
    // names — so the ops that count along a list are counted in one place, in
    // stack order. The whole-list kinds take no counter because they were never
    // per-op.
    let mut lut_i = 0usize;
    let mut dof_i = 0usize;
    let mut flare_i = 0usize;
    // The Matte's own counter (K-395). Deliberately *not* `dof_i`: the two lists
    // are enumerated by two different predicates — one effect takes a background
    // plate, every effect takes a matte — and sharing one index would bind a
    // matte to whichever Light wrap happened to sit above it. Same contract,
    // second predicate; it advances outside the GPU lookup below because
    // `build.rs` fills a slot per *op*, not per kernel.
    let mut matte_i = 0usize;
    // The mask path's own counter (K-408), for the same reason: `build.rs`
    // flattens one polyline per op whose schema declares a path row, and this
    // advances on exactly that predicate — `EffectSchema::mask_path`, the one
    // both sides call, so there is no second rule to keep in step.
    let mut path_i = 0usize;
    for resolved in ops.iter() {
        let role = resolved.def.schema().matte;
        let mask_path = if resolved.def.schema().mask_path().is_some() {
            let slot = mask_paths.get(path_i);
            path_i += 1;
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
        // Only a bound matte costs anything: the input texture is held (a
        // cheap handle clone) so the dissolve below has something to lerp
        // back towards, and nothing at all happens when the row is unset.
        let matte = matte.and_then(|m| m.texture(&tex)).cloned();
        // **Who gets it** — the one branch (K-395). A generic effect's matte is
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
        let unmatted_input = generic_matte.map(|_| tex.clone());
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
        if let Some(gpu) = crate::gpufx::gpu_effect(resolved.def.schema().match_name) {
            let data = match gpu.aux() {
                AuxKind::None => AuxData::None,
                AuxKind::Lut => {
                    let slot = luts.get(lut_i).and_then(|o| o.as_ref());
                    lut_i += 1;
                    AuxData::Lut(slot)
                }
                AuxKind::LayerInput => {
                    let slot = layer_inputs.get(dof_i).and_then(|o| o.texture(&tex));
                    dof_i += 1;
                    AuxData::LayerInput(slot)
                }
                AuxKind::LensFile => {
                    let lens = flare_lens.get(flare_i).and_then(|o| o.as_ref());
                    flare_i += 1;
                    AuxData::LensFile(lens)
                }
                AuxKind::Neighbours => AuxData::Neighbours(neighbours),
                AuxKind::FlowField => AuxData::FlowField {
                    field: flow_field,
                    neighbours,
                },
            };
            tex = gpu.run(
                fx,
                ctx,
                &tex,
                w,
                h,
                resolved.params,
                AuxSlot::new(data, own_matte, mask_path),
            );
        }

        // The generic strength semantic (K-395), one implementation for every
        // effect that has not claimed the matte for itself: after the effect's
        // own Mix (which happens inside its kernel), dissolve back to the
        // picture it was handed, by the matte's luma.
        if let (Some(m), Some(input)) = (generic_matte, unmatted_input) {
            tex = fx.matte_mix(
                ctx,
                &input,
                &tex,
                m,
                w,
                h,
                resolved.params.bool(lumit_core::fx::MATTE_INVERT_ID, false),
            );
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
    tex
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

    /// **A `.cube` edited on disk must be re-read** (K-271, docs/impl/lut.md §4).
    ///
    /// The regression: the cache keyed by path alone, so exporting a new grade
    /// over the same filename — the whole loop of grading — kept showing the
    /// first one until the application was restarted, with nothing on screen to
    /// say the file on disk and the picture had parted company.
    #[test]
    fn an_edited_cube_is_read_again() {
        let Ok(ctx) = lumit_gpu::GpuContext::headless() else {
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
    /// through it exactly as the CPU oracle does (K-271). A default-domain file
    /// still reads 0..1.
    #[test]
    fn the_declared_domain_travels_with_the_cube() {
        let Ok(ctx) = lumit_gpu::GpuContext::headless() else {
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
        let Ok(ctx) = lumit_gpu::GpuContext::headless() else {
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
}
