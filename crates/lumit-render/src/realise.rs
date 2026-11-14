//! Turning a draw list into pixels on the graphics card.
//!
//! # In plain terms
//!
//! [`crate::draw`] describes a frame; this module *makes* it. [`Realiser`]
//! borrows the GPU primitives from whoever owns them — the egui Viewer, the
//! headless renderer the Flutter frontend drives, or the exporter — and walks a
//! draw list: upload each layer's pixels, convert them to the linear working
//! space, run its effect stack, apply its matte and masks, place it with its
//! transform, and blend it onto the accumulating frame. Nested comps recurse;
//! adjustment layers split the walk in two so the stack below can be
//! composited, processed, and blended back by coverage.
//!
//! Because every caller drives this one walk, a comp looks the same in the
//! viewport, in Flutter, and in the exported file.

use crate::draw::{
    AccumulationBelow, CompLayerDraw, DrawSource, GraphDraw, GraphStep, LayerInputDraw,
};
use grow::{grow_offset, grow_size};
use order::{draw_order, runs_of};
use shadows::{casts_shadows, shadow_only};

mod grow;
mod lighting;
mod neighbour_textures;
mod order;
mod paint;
mod passes;
mod picture;
mod shadows;

pub(crate) mod dof;
pub(crate) mod scene;

/// One `node_graph` op's own walk, as [`crate::fxops::run_ops`] calls it: the
/// picture in, its raster, and the picture out. Borrowed for as long as the
/// draw the plan came from.
type GraphClosure<'a> = Box<dyn Fn(wgpu::Texture, u32, u32) -> wgpu::Texture + 'a>;

/// Where a node graph box's picture sits, in raster pixels from the frame's
/// top-left corner.
///
/// Never smaller than the frame, so a graph whose pictures all stay inside it
/// runs exactly as it did before a box kept anything past the edge. Past it, a
/// box keeps what a Read, a Transform or an undistort puts there, so the box
/// after it can bring it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Bounds {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
}

impl Bounds {
    fn frame(w: u32, h: u32) -> Self {
        Self { x: 0, y: 0, w, h }
    }

    /// `[left, top, right, bottom]`, as [`lumit_core::fx::registry::EffectDef::bounds`]
    /// takes it.
    fn rect(self) -> [f32; 4] {
        [
            self.x as f32,
            self.y as f32,
            self.x as f32 + self.w as f32,
            self.y as f32 + self.h as f32,
        ]
    }

    /// How far past an `n`-pixel edge a box may keep: `frames` frames (0 for
    /// no limit of its own), and never past the largest picture. Whether it is
    /// the largest picture that stops it.
    fn room(n: u32, frames: u32) -> (f32, bool) {
        let frames = if frames == 0 { u32::MAX } else { frames };
        let (asked, most) = (
            n.saturating_mul(frames),
            lumit_core::largest_picture::side().saturating_sub(n) / 2,
        );
        (asked.min(most) as f32, most < asked)
    }

    /// The whole pixels that cover `rect` and the `w × h` frame, keeping at
    /// most `frames` frames past each edge (0 for no limit of its own) and
    /// never more than the largest picture.
    fn covering(rect: [f32; 4], w: u32, h: u32, frames: u32) -> Self {
        let ((rx, cap_x), (ry, cap_y)) = (Self::room(w, frames), Self::room(h, frames));
        // Cut by the largest picture rather than by the overscan asked for.
        if (cap_x && (rect[0] < -rx || rect[2] > w as f32 + rx))
            || (cap_y && (rect[1] < -ry || rect[3] > h as f32 + ry))
        {
            lumit_core::largest_picture::note_cut();
        }
        // Nothing sensible to cover reads as the frame's own edge.
        let low = |v: f32, room: f32| {
            if v.is_nan() {
                0.0
            } else {
                v.floor().clamp(-room, 0.0)
            }
        };
        let high = |v: f32, n: u32, room: f32| {
            if v.is_nan() {
                n as f32
            } else {
                v.ceil().clamp(n as f32, n as f32 + room)
            }
        };
        let (x0, y0) = (low(rect[0], rx), low(rect[1], ry));
        let (x1, y1) = (high(rect[2], w, rx), high(rect[3], h, ry));
        Self {
            x: x0 as i32,
            y: y0 as i32,
            w: (x1 - x0) as u32,
            h: (y1 - y0) as u32,
        }
    }

    /// The most a box may keep round a `w × h` frame.
    fn limit(w: u32, h: u32, frames: u32) -> Self {
        let (rx, ry) = (
            Self::room(w, frames).0 as u32,
            Self::room(h, frames).0 as u32,
        );
        Self {
            x: -(rx as i32),
            y: -(ry as i32),
            w: w + rx * 2,
            h: h + ry * 2,
        }
    }

    /// Whether this reaches the limit on any side the limit is past the
    /// frame.
    fn at_limit(self, w: u32, h: u32, frames: u32) -> bool {
        let limit = Self::limit(w, h, frames);
        let [x0, y0, x1, y1] = self.rect();
        let [lx0, ly0, lx1, ly1] = limit.rect();
        (lx0 < 0.0 && x0 <= lx0)
            || (ly0 < 0.0 && y0 <= ly0)
            || (lx1 > w as f32 && x1 >= lx1)
            || (ly1 > h as f32 && y1 >= ly1)
    }

    fn union(self, other: Self) -> Self {
        let (x0, y0) = (self.x.min(other.x), self.y.min(other.y));
        let x1 = (self.x + self.w as i32).max(other.x + other.w as i32);
        let y1 = (self.y + self.h as i32).max(other.y + other.h as i32);
        Self {
            x: x0,
            y: y0,
            w: (x1 - x0) as u32,
            h: (y1 - y0) as u32,
        }
    }

    /// This window of the frame's space, for the compositor.
    fn region(self) -> lumit_gpu::composite::Region {
        lumit_gpu::composite::Region {
            x: self.x as f32,
            y: self.y as f32,
            w: self.w as f32,
            h: self.h as f32,
        }
    }
}

/// What a 3D viewport's ground is drawn with: a linear colour with straight
/// alpha, and where the floor is, in comp pixels: the x its lines are counted
/// from, its height, and how far apart its lines are. The picture a viewport
/// draws can be wider than the comp, so these come from the comp itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewGround {
    pub colour: [f32; 4],
    pub middle: f32,
    pub floor: f32,
    pub step: f32,
}

/// The GPU primitives that turn a comp draw list into a linear texture,
/// borrowed from whichever owner is compositing — a frontend's viewer, the
/// headless renderer, or the export renderer. Factoring the realise logic behind
/// one borrowed handle is what lets preview and export share a single re-render
/// path (`render_below_at`, docs/impl/temporal-rerender.md): all of them drive
/// the identical compositor, so a comp realises the same in the viewport and
/// the file.
pub struct Realiser<'a> {
    /// Owned handle (a cheap Arc-backed clone via [`lumit_gpu::GpuContext::
    /// from_parts`]) so the realise code can keep passing `&self.ctx`; the
    /// engines below cannot be cloned, so they stay borrowed.
    pub ctx: lumit_gpu::GpuContext,
    pub engine: &'a lumit_gpu::ColourEngine,
    pub compositor: &'a lumit_gpu::composite::Compositor,
    pub fx: &'a lumit_gpu::fx::FxEngine,
    pub lut_cache: &'a std::cell::RefCell<crate::fxops::LutCache>,
    /// The per-effect intermediate cache, held by the same owner as
    /// the LUT cache and for the same reason: it outlives a frame. Every
    /// layer's stack reads from it; only committed renders add to it (see
    /// [`crate::fxops::FxCache::keep_outputs`]).
    pub fx_cache: &'a std::cell::RefCell<crate::fxops::FxCache>,
    /// The preview render scale: every composite this
    /// walk performs allocates its target at [`lumit_gpu::composite::scaled_size`] of the
    /// comp dims while all geometry stays in logical comp pixels. A field
    /// rather than a parameter so the nested/below/adjustment recursions
    /// inherit it with no signature ripple. Export always builds with 1.0, so
    /// the preview == export identity is untouched at full scale.
    pub render_scale: f32,
    /// The project's anti-aliasing sample count (docs/impl/anti-aliasing.md):
    /// how many coverage samples per pixel the composite is drawn with. 1 is
    /// the picture Lumit made before the setting existed. A field beside
    /// [`Self::render_scale`], for the same reason — the nested, below and
    /// adjustment recursions inherit it with no signature ripple — but **not**
    /// like it in one way that matters: `render_scale` differs between preview
    /// and export by design, and this must not. Both paths read the same
    /// project field, which is what keeps the preview-equals-export identity
    /// true with anti-aliasing on.
    ///
    /// Already run through [`lumit_gpu::supported_sample_count`] by whoever
    /// built the realiser: by the time it is here it is a count this adapter
    /// really offers.
    pub samples: u32,
    /// The frame's recorder, when this render is being watched (docs/13 §7.1):
    /// it counts finished layers for the Viewer's progress bar and measures
    /// each layer and effect for the render-time indicators. `None` — every
    /// frame of playback, and every unwatched render — walks exactly as it did
    /// before this existed: no clocks, no fences, no extra allocations.
    pub profiler: Option<&'a crate::profile::FrameProfiler>,
    /// The footage input transforms this frame may need, already baked and
    /// uploaded (docs/impl/ocio.md §5.2). `None` — no config, an unusable
    /// one, or any of the test builders — means every source linearises
    /// through the built-in interpretation, which is what this always did.
    ///
    /// Every source of **image content** reads through it: a layer's own pixels
    /// ([`DrawSource::Pixels`]), a matte source's ([`crate::draw::MatteDraw`]),
    /// and a layer input's — a Light wrap background plate, a Texturize texture
    /// ([`crate::draw::DofInputDraw`]). A mask's coverage raster is not image
    /// content: it is a shape's alpha, drawn by Lumit rather than decoded from
    /// anybody's camera, so it has no colour space to be interpreted through and
    /// takes the built-in path deliberately.
    pub colour_inputs: Option<&'a crate::colour::InputTransforms>,
    /// The project's OCIO config, loaded and usable, for the OCIO effects'
    /// tables (docs/08 §3.97). `None` - no config, an unusable one, or any of
    /// the test builders - leaves the three config-bound OCIO effects as
    /// passthroughs; the file transform reads its file either way.
    pub colour_config: Option<&'a std::sync::Arc<crate::colour::Loaded>>,
    /// The flow backend a composite measurement runs on (docs/08 §3.2),
    /// held by the render's owner beside the caches above and for the same
    /// reason: the solver's plan is worth keeping between frames. `None` — a
    /// caller that never asks for one, and every builder in the tests that has
    /// no use for it — means a Motion blur or Datamosh on an adjustment or
    /// Precomp layer degrades to the passthrough it was before this existed,
    /// which is exactly what an unavailable GPU flow does too.
    pub flow: Option<&'a std::cell::RefCell<CompositeFlow>>,
    /// Makes Clone to points' pictures for a stream that was only made
    /// during the render, handed where the list belongs and how many points
    /// the stream holds ([`crate::build::late_clone_pictures`]). The render's
    /// owner supplies it, since it holds the document and the decoded pixels
    /// and this walk holds neither. `None`, which is every builder in the
    /// tests, leaves such a list absent, so nothing is stamped.
    pub late_clones: Option<&'a LateCloneMaker<'a>>,
    /// Where the walk tells the Viewer how far a node graph composition's
    /// picture reaches past its frame. Each graph walked on its own sets it
    /// as its Output is cut back to the frame, so the outermost graph, which
    /// finishes last, is the one left in it. `None` for every render that
    /// draws no outline.
    pub graph_reach: Option<&'a std::cell::Cell<Option<Reach>>>,
}

/// How far a node graph composition's picture reaches past its frame:
/// `[left, top, right, bottom]` in the composition's own pixels from the
/// frame's top-left corner, and whether it got as far as a box may keep.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reach {
    pub rect: [f32; 4],
    pub at_limit: bool,
}

/// What [`Realiser::late_clones`] calls.
pub type LateCloneMaker<'a> = dyn Fn(&crate::draw::LateClones, usize) -> Vec<LayerInputDraw> + 'a;

/// The flow backend for measuring a **composite's** motion (docs/08 §3.2),
/// built on the render's own device the first time a layer actually asks for
/// one.
///
/// # In plain terms
///
/// Measuring motion means comparing two pictures, and building the flow solver
/// compiles a dozen shaders. A project with no Motion blur and no Datamosh
/// on an adjustment or Precomp layer never asks, so it never pays: this starts
/// empty and fills itself in on the first question.
#[derive(Default)]
pub struct CompositeFlow {
    engine: Option<lumit_flow::FlowEngine>,
    /// The pictures made of neighbour frames, kept for the next frame.
    neighbours: neighbour_textures::NeighbourTextures,
    /// The synthesis model a Precomp on Flow with RIFE paints with, opened
    /// as the decode pool opens its own.
    synthesis: Option<(
        u64,
        Result<lumit_ml::synthesis::Synthesis, lumit_ml::error::MlError>,
    )>,
    /// Whether a model that will not paint fails the frame, as an export's
    /// does, and the sentence it failed with.
    refuse: bool,
    refused: Option<String>,
}

impl Realiser<'_> {
    /// The baked input transform for one colour space name, or `None` for the
    /// built-in interpretation — which is the answer for an unassigned item, a
    /// project with no config, a config that is not usable, and a name the
    /// loaded config does not have.
    fn input_transform(&self, space: &Option<String>) -> Option<&lumit_gpu::OcioArtefact> {
        self.colour_inputs?.get(space.as_deref())
    }

    /// Source pixels to a linear working texture, whichever width they arrived
    /// at. Every picture that comes from a file goes through here — a layer's
    /// own, a matte source's, a depth pass fed to an effect — so there is one
    /// answer to "what happens to a float plate" rather than three.
    ///
    /// **Eight-bit** is uploaded sRGB-encoded and decoded by the linearise
    /// pass, as it always was.
    ///
    /// **Float** is already scene-linear, so with no colour space assigned the
    /// upload *is* the answer and no pass runs at all. With one assigned — an
    /// ACEScg render, say — the same pass runs on the float values as they
    /// stand: an input transform is a colour conversion, not a decode, and
    /// `lumit_gpu::unencoded` leaves a float format alone, which is what makes
    /// the one pass right for both widths.
    fn upload_source(
        &self,
        rgba: &[u8],
        w: u32,
        h: u32,
        format: lumit_media::decode::PixelFormat,
        colour_space: &Option<String>,
    ) -> wgpu::Texture {
        let ocio = self.input_transform(colour_space);
        match (format, ocio) {
            (lumit_media::decode::PixelFormat::LinearF32, None) => {
                self.engine.upload_float_frame(&self.ctx, rgba, w, h)
            }
            (lumit_media::decode::PixelFormat::LinearF32, ocio) => {
                let src = self.engine.upload_float_frame(&self.ctx, rgba, w, h);
                // Into the source's own format, not the narrower working one:
                // the transform must not be where the precision goes.
                let format = src.format();
                self.engine.linearise_into(&self.ctx, &src, ocio, format)
            }
            (lumit_media::decode::PixelFormat::Srgb8, ocio) => {
                let src = self.engine.upload_srgb8(&self.ctx, rgba, w, h);
                self.engine.linearise_through(&self.ctx, &src, ocio)
            }
        }
    }

    /// Start one layer's measurement: the clock, and the list its effect stack
    /// writes a millisecond into per op. Both `None` unless this render is
    /// being measured, which is what keeps an ordinary frame free of them.
    #[allow(clippy::type_complexity)]
    fn layer_clock(&self) -> (Option<web_time::Instant>, Option<Vec<f32>>) {
        match self.profiler {
            Some(p) if p.timing() => (Some(web_time::Instant::now()), Some(Vec::new())),
            _ => (None, None),
        }
    }

    /// Close one layer's measurement and hand it to the profiler: the fence
    /// (see `crate::profile` on why the clock cannot be read without one), the
    /// per-effect list paired back up with the ids the draw carries, and the
    /// layer counted towards the progress bar.
    fn layer_done(
        &self,
        l: &CompLayerDraw,
        started: Option<web_time::Instant>,
        fx_ms: Option<Vec<f32>>,
    ) {
        let Some(p) = self.profiler else {
            return;
        };
        let ms = match started {
            Some(started) => {
                // Hand this layer's work over before waiting on it. Batching
                // holds a frame's commands back to the end, and a fence over an
                // empty queue would time nothing — so a *measured* frame gives
                // up the batching, layer by layer, which is the cost the
                // stopwatch already declares (docs/13 §7.1: measuring
                // waits for the card at each layer, which is why it is opt-in
                // and never runs during playback).
                self.ctx.flush();
                self.ctx.device.poll(wgpu::Maintain::Wait);
                started.elapsed().as_secs_f32() * 1000.0
            }
            // Progress is being reported but nothing is being measured: the
            // layer still counts towards the bar, it just has no number.
            None => 0.0,
        };
        let effects = fx_ms
            .map(|ms| {
                l.fx_ids
                    .iter()
                    .copied()
                    .zip(ms)
                    .map(|(effect, ms)| crate::profile::EffectTiming { effect, ms })
                    .collect()
            })
            .unwrap_or_default();
        p.layer_done(l.layer, ms, effects);
    }

    /// Read a layer's lenses (a kept lens's text, or a `lens_file` path) into
    /// (content hash, text) slots, 1:1 with the stack's lens ops. A `None`
    /// slot (unset, missing on disk, unreadable) degrades to the picked
    /// library lens inside the bake — a labelled fallback, never a fault.
    /// No cache, deliberately: a .lens file is about a kilobyte, the read
    /// is microseconds beside the frame it feeds, and the GPU bake cache
    /// keys on the CONTENT hash — so an edited file takes effect on the
    /// next frame instead of whenever a path-keyed cache is purged.
    fn load_flare_lens(
        &self,
        files: &[Option<lumit_core::fx::lens::LensSource>],
    ) -> Vec<Option<(u64, String)>> {
        use lumit_core::fx::lens::LensSource;
        files
            .iter()
            .map(|slot| {
                let text = match slot.as_ref()? {
                    LensSource::Text(text) => text.clone(),
                    LensSource::File(path) => std::fs::read_to_string(path).ok()?,
                };
                Some((lumit_core::fx::lens_flare::lens_text_hash(&text), text))
            })
            .collect()
    }

    /// Turn a layer's ordered `colour_tables` into the parallel `tables` list
    /// `run_ops` binds (docs/08 §3.11): each `Some(path)` is parsed and
    /// uploaded once — cached by path *and* last-modified time, bounded and
    /// LRU-evicted (docs/impl/lut.md §4) — and a 1D or unreadable/absent
    /// file yields a `None` slot (a labelled no-op, never a fault). The output
    /// is 1:1 and in order with `files`, so the k-th slot lines up with the
    /// k-th `lut` op.
    fn load_tables(
        &self,
        files: &[Option<crate::colour::TableRequest>],
    ) -> Vec<Option<crate::fxops::ColourTable>> {
        use crate::colour::TableRequest;
        use crate::fxops::ColourTable;
        let mut cache = self.lut_cache.borrow_mut();
        files
            .iter()
            .map(|slot| match slot.as_ref()? {
                TableRequest::Cube(path) => {
                    cache.get_or_load(&self.ctx, path).map(ColourTable::Cube)
                }
                TableRequest::Ocio(request) => cache
                    .get_or_bake(&self.ctx, self.engine, request, self.colour_config)
                    .map(ColourTable::Ocio),
            })
            .collect()
    }

    /// One planes-tier plane as a texture at the working raster `(w, h)`.
    ///
    /// Two steps, and they are memoised differently on purpose. The **upload**
    /// is per (instance, source frame), because the bytes do not move while a
    /// plane is being watched. The **resample** is per frame, because the
    /// working raster moves with the preview tier, and it goes through
    /// [`crate::fxops::layer_input::render_layer_input`] - the one helper the preview and
    /// export paths both stretch a picture to a working raster with, so a plane
    /// lands on the same pixel grid in the viewport and the file.
    ///
    /// A plane is at the **model's** own raster, a few hundred pixels on the
    /// long side, so this is a real resample and not the crop-and-pad
    /// `fit_centred` does: a sixteen-pixel plane centred in a frame would be a
    /// small square of depth in a sea of nothing.
    fn plane_texture(&self, p: &crate::draw::PlaneDraw, w: u32, h: u32) -> wgpu::Texture {
        let uploaded = self.upload_plane(p);
        crate::fxops::layer_input::render_layer_input(
            self.compositor,
            &self.ctx,
            w,
            h,
            &uploaded,
            false,
            p.width as f32,
            p.height as f32,
        )
    }

    /// One planes-tier plane uploaded at its own raster, once per (instance,
    /// source frame) and handed back on every later frame drawn through it
    /// (docs/impl/addons.md §6.1).
    ///
    /// Grey into all four channels, the same shape a roto matte takes and for
    /// the same reason: `set_matte` reads the luminance of whichever channel it
    /// is told to, so the plane reads correctly whichever way it is asked. The
    /// numbers are a reading and not colour, so no transfer function is applied
    /// to them.
    fn upload_plane(&self, p: &crate::draw::PlaneDraw) -> wgpu::Texture {
        self.fx_cache
            .borrow_mut()
            .plane(p.instance, p.source_frame, p.content, || {
                let n = (p.width as usize) * (p.height as usize);
                let mut halfs = Vec::with_capacity(n * 4);
                match p.kind {
                    crate::planes::PlaneKind::Depth => {
                        for i in 0..n {
                            let low = p.data.get(i * 2).copied().unwrap_or(0);
                            let high = p.data.get(i * 2 + 1).copied().unwrap_or(0);
                            let v =
                                f32::from(u16::from_le_bytes([low, high])) / f32::from(u16::MAX);
                            let bits = lumit_gpu::fx::common::f16_bits(v);
                            halfs.extend_from_slice(&[bits, bits, bits, bits]);
                        }
                    }
                    crate::planes::PlaneKind::Matte => {
                        // The same 256-entry table `upload_roto_matte` builds:
                        // a gray8 byte straight to the fp16 bits the texture
                        // holds, with no full-plane f32 temporary in between.
                        let mut table = [0u16; 256];
                        for (v, bits) in table.iter_mut().enumerate() {
                            *bits = lumit_gpu::fx::common::f16_bits(v as f32 / 255.0);
                        }
                        for i in 0..n {
                            let bits = table[usize::from(p.data.get(i).copied().unwrap_or(0))];
                            halfs.extend_from_slice(&[bits, bits, bits, bits]);
                        }
                    }
                }
                lumit_gpu::fx::common::upload_linear_f16(&self.ctx, &halfs, p.width, p.height)
            })
    }

    /// A baked picture - a roto matte, a plane - laid into a node graph's frame
    /// where the footage it was made from is laid.
    ///
    /// On a layer the stack runs on the layer's own raster, so a matte of the
    /// file lines up with the file by construction. A box runs on the graph's
    /// frame, and the file's picture is in it wherever its Read put it: centred
    /// at its natural size. So the baked picture is stretched to that size and
    /// set down at that place, and everything outside it reads nothing.
    ///
    /// `size` is the baked picture's own raster: the file's for a roto matte,
    /// and the model's for a plane, which is a few hundred pixels on the long
    /// side whatever the file is.
    fn lay_as_read(
        &self,
        baked: &wgpu::Texture,
        size: (u32, u32),
        read: &CompLayerDraw,
        w: u32,
        h: u32,
        scale: f32,
    ) -> wgpu::Texture {
        let (bw, bh) = (size.0.max(1) as f32, size.1.max(1) as f32);
        self.compositor.composite_with_camera(
            &self.ctx,
            w,
            h,
            [0.0, 0.0, 0.0, 0.0],
            &[lumit_gpu::composite::CompositeLayer {
                texture: baked,
                size: (bw, bh),
                position: (read.position.0 * scale, read.position.1 * scale),
                anchor: (bw * 0.5, bh * 0.5),
                scale: (
                    read.natural_size.0 * scale / bw * 100.0,
                    read.natural_size.1 * scale / bh * 100.0,
                ),
                rotation_deg: 0.0,
                // Full opacity: a matte is read as a number, never dimmed.
                opacity: 100.0,
                matte: None,
                blend: lumit_gpu::composite::Blend::Normal,
                z: 0.0,
                rotation_x_deg: 0.0,
                rotation_y_deg: 0.0,
                three_d: false,
                layer_mask: None,
                pre: None,
                premultiplied: false,
            }],
            None,
        )
    }

    /// Render a layer's layer-input slots (docs/impl/layer-input.md §2) — the
    /// depth passes of its `dof` effects, the matte sources of its Lens
    /// flares. Each [`LayerInputDraw::Layer`] (the referenced layer's source
    /// pixels) is uploaded, linearised and resampled into the effect's
    /// working raster `(w, h)` through the shared
    /// [`crate::fxops::layer_input::render_layer_input`], so the parallel list handed to
    /// `run_ops` is 1:1 with the stack's ops and aligned with the layer
    /// texture the kernel reads. Export renders these identically.
    ///
    /// One op's whole list of pictures, each rendered as it would be alone and
    /// with its own size kept beside it.
    fn several(
        &self,
        list: &[LayerInputDraw],
        w: u32,
        h: u32,
        of: (f32, f32),
        below: Option<&wgpu::Texture>,
    ) -> crate::fxops::LayerInput {
        crate::fxops::LayerInput::Several(
            self.render_layer_inputs(list, w, h, of, below)
                .into_iter()
                .zip(list.iter().map(LayerInputDraw::size))
                .collect(),
        )
    }

    /// Clone to points' list for a stream that was only made during the
    /// render: the builder is asked for the pictures now that the stream
    /// holds `points` points, and they are rendered like any other list.
    /// Absent where `slot` is not such a list or nobody can make one.
    fn late_clones(
        &self,
        slot: Option<&LayerInputDraw>,
        points: usize,
        w: u32,
        h: u32,
        of: (f32, f32),
    ) -> crate::fxops::LayerInput {
        match (slot, self.late_clones) {
            (Some(LayerInputDraw::Late(spec)), Some(make)) => {
                self.several(&make(spec, points), w, h, of, None)
            }
            _ => crate::fxops::LayerInput::Absent,
        }
    }

    /// [`LayerInputDraw::ThisLayer`] renders nothing here: it names the texture
    /// `run_ops` is carrying, so it passes through as [`LayerInput::ThisLayer`].
    ///
    /// `of` is the consuming layer's own size, px at comp size, which a map
    /// set down where its layer sits is measured against. `below` is the
    /// depth of the layers below an adjustment layer, when it was made.
    pub(crate) fn render_layer_inputs(
        &self,
        inputs: &[LayerInputDraw],
        w: u32,
        h: u32,
        of: (f32, f32),
        below: Option<&wgpu::Texture>,
    ) -> Vec<crate::fxops::LayerInput> {
        use crate::fxops::LayerInput;
        inputs
            .iter()
            .map(|slot| {
                let d = match slot {
                    LayerInputDraw::Absent => return LayerInput::Absent,
                    LayerInputDraw::ThisLayer => return LayerInput::ThisLayer,
                    LayerInputDraw::Below(_) => {
                        return below.map_or(LayerInput::Absent, |d| {
                            LayerInput::Depth(
                                lumit_gpu::fx::fit_centred(&self.ctx, d.clone(), w, h),
                                None,
                            )
                        })
                    }
                    LayerInputDraw::EffectOutput(id) => return LayerInput::EffectOutput(*id),
                    LayerInputDraw::EffectKey(id) => return LayerInput::EffectKey(*id),
                    // Made when its op comes round (`late_clones`).
                    LayerInputDraw::Late(_) => return LayerInput::Late,
                    // An op's whole list: each layer rendered as it would be
                    // alone, with its own size kept beside it.
                    LayerInputDraw::Several(list) => return self.several(list, w, h, of, below),
                    LayerInputDraw::Layer(d) => d,
                };
                // Onto the consuming layer's raster, where its placement says.
                let set_down = |linear: &wgpu::Texture| match &d.place {
                    Some(place) => crate::fxops::layer_input::place_layer_input(
                        self.compositor,
                        &self.ctx,
                        w,
                        h,
                        linear,
                        d.nested.is_some(),
                        d.natural,
                        place,
                        of,
                    ),
                    None => crate::fxops::layer_input::render_layer_input(
                        self.compositor,
                        &self.ctx,
                        w,
                        h,
                        linear,
                        d.nested.is_some(),
                        d.tex_w as f32,
                        d.tex_h as f32,
                    ),
                };
                // A Precomp input realises its nested comp exactly as a
                // Precomp layer's picture does; anything else is
                // the uploaded source pixels.
                let linear = if let Some(n) = d.nested.as_ref().filter(|n| n.reads_depth()) {
                    // Distance, not a picture, so the referenced layer's own
                    // stack below has nothing to work on.
                    let mut scene = None;
                    let depth = self.realise_scene_depth(n, n.trace_layers.then_some(&mut scene));
                    return LayerInput::Depth(set_down(&depth), scene.map(std::sync::Arc::new));
                } else if let Some((n, read)) =
                    d.nested.as_ref().and_then(|n| Some((n, n.read.as_ref()?)))
                {
                    return match self.realise_scene_pass(n, read) {
                        Some(pass) => LayerInput::Texture(set_down(&pass)),
                        None => LayerInput::Absent,
                    };
                } else if let Some(n) = &d.nested {
                    self.painted_input(n)
                } else {
                    // Through the referenced layer's own colour space:
                    // a background plate is a picture like any other, and it is
                    // read as what it is rather than as what the layer reading
                    // it happens to be.
                    self.upload_source(&d.rgba, d.tex_w, d.tex_h, d.format, &d.colour_space)
                };
                // The size it came out at. A comp is realised at the render
                // scale, so in a zoomed-out Viewer it is smaller than `d.tex_w`.
                let (lw, lh) = (linear.width(), linear.height());
                // Effects-and-masks depth: run the depth layer's own
                // stack on its texture before it is resampled, when the consumer's
                // depth source is Effects and masks (`d.fx` non-empty). Temporal
                // inputs stay empty in v1 (same boundary as the matte). Export
                // does the same, so the two depth passes match.
                let linear = if d.fx.is_empty() {
                    linear
                } else {
                    let tables = self.load_tables(&d.colour_tables);
                    // The referenced layer's **own** baked pictures. Without
                    // these its stack ran with every carriage empty, so a Roto
                    // brush or a Depth on the layer somebody pointed a Matte
                    // row at rendered as a passthrough and the consumer read
                    // the plain footage instead (docs/impl/addons.md §13). The
                    // carriage is filled by the same predicate and the same
                    // (instance, source frame) lookup the layer's own is.
                    let roto: Vec<Option<wgpu::Texture>> = d
                        .roto_mattes
                        .iter()
                        .map(|s| s.as_ref().map(|m| self.roto_texture(m, lw, lh)))
                        .collect();
                    let planes: Vec<Option<wgpu::Texture>> = d
                        .planes
                        .iter()
                        .map(|slot| {
                            slot.as_ref()
                                .map(|p| self.plane_texture(p, d.tex_w, d.tex_h))
                        })
                        .collect();
                    let side = crate::fxops::Side {
                        roto: &roto,
                        planes: &planes,
                        late: None,
                    };
                    // The graphs the referenced layer's own Node graph
                    // effects apply, at the raster its stack runs on.
                    let px = if d.natural[0] > 0.0 {
                        d.tex_w as f32 / d.natural[0]
                    } else {
                        1.0
                    };
                    let plans: Vec<GraphClosure<'_>> = d
                        .graph_fx
                        .iter()
                        .map(|plan| {
                            Box::new(move |tex: wgpu::Texture, gw: u32, gh: u32| {
                                self.realise_graph(plan, Some(&tex), gw, gh, px)
                            }) as GraphClosure<'_>
                        })
                        .collect();
                    let graphs: Vec<&dyn Fn(wgpu::Texture, u32, u32) -> wgpu::Texture> =
                        plans.iter().map(|f| &**f).collect();
                    // A comp comes out at the render scale, so its stack runs
                    // at that size, its distances scaled to match.
                    let mut fx = d.fx.clone();
                    fx.rescale_spatial(lw as f32 / d.tex_w.max(1) as f32);
                    crate::fxops::run_ops(
                        self.fx,
                        &self.ctx,
                        linear,
                        lw,
                        lh,
                        &fx,
                        &[],
                        &[],
                        &tables,
                        &[],
                        &self.load_flare_lens(&d.flare_lens_files),
                        &[],
                        // No mask paths through a referenced layer's own stack
                        // — the same v1 boundary its mattes take.
                        &[],
                        // And no birth schedules: a Particulate inside a
                        // referenced layer's own stack takes the same v1
                        // boundary, and an unscheduled op passes its picture
                        // through.
                        &[],
                        &side,
                        &graphs,
                        // A matte's own stack is part of the effect that reads
                        // it, not a row of its own: its cost is inside that
                        // layer's span already.
                        None,
                        // Nothing names a referenced layer's picture in v1,
                        // so its stack runs uncached.
                        None,
                    )
                };
                // Same boundary as the matte's: the referenced layer
                // is placed by `d.tex_w × d.tex_h` just below, so its own stack
                // is cropped back to that raster rather than growing it.
                let linear = lumit_gpu::fx::fit_centred(&self.ctx, linear, lw, lh);
                LayerInput::Texture(set_down(&linear))
            })
            .collect()
    }

    /// Realise a draw list into a linear comp texture (recursive for
    /// Nested), staging at each Adjust draw (docs/06 §1.5): everything
    /// before it composites into an intermediate, the adjustment's stack
    /// runs on that, and the two blend by coverage; the draws after
    /// composite straight onto the blended result (seeded, no resample).
    pub fn realise(
        &self,
        camera: Option<lumit_core::model::CameraPose>,
        width: u32,
        height: u32,
        background: [f64; 4],
        layers: &[CompLayerDraw],
    ) -> wgpu::Texture {
        self.realise_region(camera, width, height, background, layers, None, None)
    }

    /// A nested comp's picture: the texture held under its frame key
    /// when there is one, else [`Self::realise`] and — on a committed render —
    /// file it. The three places a Precomp is realised (a layer, a matte, a
    /// layer input) all come through here, so "the same nested comp used in
    /// five places renders once" (docs/06 §5.2) holds for every one of them.
    /// The texture is read-only downstream — the stack and the composite both
    /// make new textures — so handing the same one out twice is safe.
    pub fn realise_nested(
        &self,
        key: Option<u128>,
        camera: Option<lumit_core::model::CameraPose>,
        width: u32,
        height: u32,
        background: [f64; 4],
        layers: &[CompLayerDraw],
    ) -> wgpu::Texture {
        let Some(key) = key else {
            return self.realise(camera, width, height, background, layers);
        };
        let key = crate::fxops::nested_texture_key(key, self.render_scale, self.samples);
        // A measured walk (docs/13 §7.1) wants a number on every row inside
        // the Precomp too, so it realises the nested comp whether or not the
        // picture is held — and still files what it made.
        if self.profiler.is_none() {
            if let Some(held) = self.fx_cache.borrow_mut().nested(key) {
                return held;
            }
        }
        let made = self.realise(camera, width, height, background, layers);
        // A frame asked to stop may have drawn this short.
        if !crate::cancel::requested() {
            self.fx_cache.borrow_mut().put_nested(key, made.clone());
        }
        made
    }

    /// The closures [`crate::fxops::run_ops`] calls in place of a `node_graph`
    /// op's kernel: one per plan the draw carries, in the same stack order
    /// (docs/impl/node-graph-comp.md §2.4).
    ///
    /// The graph is walked at the raster the host stack is running on, and its
    /// own boxes are rescaled by the very factor the host's px parameters were:
    /// `fx_ref_width` where the builder resolved against a wider raster (a
    /// Precomp layer's, an adjustment's), and the layer's own natural width
    /// otherwise, which is the scale its decode already applied.
    ///
    /// Empty on every layer that applies no graph, which allocates nothing.
    fn graph_closures<'b>(&'b self, l: &'b CompLayerDraw, w: u32) -> Vec<GraphClosure<'b>> {
        let ref_w = l.fx_ref_width.unwrap_or(l.natural_size.0);
        let px = if ref_w > 0.0 { w as f32 / ref_w } else { 1.0 };
        l.graph_fx
            .iter()
            .map(|plan| {
                Box::new(move |tex: wgpu::Texture, gw: u32, gh: u32| {
                    self.realise_graph(plan, Some(&tex), gw, gh, px)
                }) as GraphClosure<'b>
            })
            .collect()
    }

    /// A transparent picture at `w` × `h` - what an unwired socket reads, and
    /// what a graph with nothing in it shows.
    ///
    /// The compositor with no layers, which is a clear to the background and
    /// nothing else, so the size is exactly the one asked for.
    fn transparent(&self, w: u32, h: u32) -> wgpu::Texture {
        self.compositor
            .composite_with_camera(&self.ctx, w, h, [0.0, 0.0, 0.0, 0.0], &[], None)
    }

    /// **A node graph's picture** (docs/impl/node-graph-comp.md §2.3): the
    /// plan's steps walked in order, one texture kept per step, and the one
    /// the Output names handed back.
    ///
    /// `w` and `h` are the raster every step is made at. `scale` is how many of
    /// those raster pixels one of the graph's own frame pixels is, which is the
    /// render scale when the graph **is** the comp and the host stack's own
    /// px_scale when it is applied as an effect. One number for both, because
    /// both ask the same question: a Read box is placed in the graph's pixels,
    /// and a box's px parameters were resolved in them.
    ///
    /// `provided` is the picture handed in from outside - the layer's own at
    /// that point in the stack. `None` is the comp viewed on its own, where
    /// the first picture Input reads transparent (§1.5).
    ///
    /// **What the walk does not carry in v1** (§2.3), each the same boundary a
    /// group header takes: neighbour frames, flow fields, the below-stack,
    /// paint, lighting and mask paths. Each is a carriage a step would grow,
    /// and none blocks the compositing the graph is for.
    ///
    /// Each step keeps its picture with its [`Bounds`], which can reach past
    /// the frame. Only the Output is cut back to it.
    pub fn realise_graph(
        &self,
        plan: &GraphDraw,
        provided: Option<&wgpu::Texture>,
        w: u32,
        h: u32,
        scale: f32,
    ) -> wgpu::Texture {
        use crate::fxops::LayerInput;
        let frame = Bounds::frame(w, h);
        // A picture moved onto another window of the frame's space, cut or
        // padded with transparent to fit it.
        let place = |(tex, from): (wgpu::Texture, Bounds), to: Bounds| {
            lumit_gpu::fx::fit_at(&self.ctx, tex, to.w, to.h, [from.x - to.x, from.y - to.y])
        };
        // One picture per step, `None` where the step reads transparent - an
        // unwired socket, a Read of an item somebody deleted, a Switch pointed
        // past its last picture. Nothing is allocated for one until a consumer
        // actually needs a picture to work on.
        let mut made: Vec<Option<(wgpu::Texture, Bounds)>> = Vec::with_capacity(plan.steps.len());
        let held = |made: &[Option<(wgpu::Texture, Bounds)>],
                    step: &Option<usize>|
         -> Option<(wgpu::Texture, Bounds)> {
            made.get((*step)?).cloned().flatten()
        };
        for step in &plan.steps {
            let tex = match step {
                // The picture handed in, or nothing at all.
                GraphStep::Provided => provided.cloned().map(|t| (t, frame)),
                GraphStep::SceneDepth(n) => {
                    let depth = self.realise_scene_depth(n, None);
                    Some((
                        crate::fxops::layer_input::render_layer_input(
                            self.compositor,
                            &self.ctx,
                            w,
                            h,
                            &depth,
                            false,
                            n.width as f32,
                            n.height as f32,
                        ),
                        frame,
                    ))
                }
                // A further picture Input: a layer rendered alone, on the
                // depth pass's own road.
                GraphStep::Picture(input) => {
                    match self
                        .render_layer_inputs(
                            std::slice::from_ref(input),
                            w,
                            h,
                            (w as f32, h as f32),
                            None,
                        )
                        .into_iter()
                        .next()
                    {
                        Some(LayerInput::Texture(t) | LayerInput::Depth(t, _)) => Some((t, frame)),
                        // `ThisLayer` cannot reach here: a graph has no layer
                        // for a reference to name.
                        _ => None,
                    }
                }
                // A project item at default placement, realised alone - the
                // matte path's "render alone at comp size", in the graph's
                // frame.
                GraphStep::Read(draw) => {
                    let linear = match &draw.source {
                        DrawSource::Pixels {
                            rgba,
                            tex_w,
                            tex_h,
                            format,
                            colour_space,
                        } => self.upload_source(rgba, *tex_w, *tex_h, *format, colour_space),
                        DrawSource::Nested {
                            width,
                            height,
                            background,
                            draws,
                            camera,
                            key,
                            ..
                        } => {
                            self.realise_nested(*key, *camera, *width, *height, *background, draws)
                        }
                        // A Read box makes no other kind of source; a
                        // transparent texel keeps the no-panic rule if that
                        // ever regresses.
                        DrawSource::Adjust
                        | DrawSource::Graph(_)
                        | DrawSource::Haze(_)
                        | DrawSource::Environment(_)
                        | DrawSource::Model(_) => self.transparent(1, 1),
                    };
                    // The placement is in the graph's own pixels, so it is
                    // carried into the raster by the one factor everything
                    // else in this walk is.
                    let size = (draw.natural_size.0 * scale, draw.natural_size.1 * scale);
                    let position = (draw.position.0 * scale, draw.position.1 * scale);
                    let anchor = (draw.anchor.0 * scale, draw.anchor.1 * scale);
                    // The whole picture is kept, wherever it lands, so a
                    // file bigger than the frame can still be scaled down
                    // into it by a box further on.
                    let m = lumit_gpu::composite::place_matrix(
                        position,
                        anchor,
                        draw.scale,
                        draw.rotation_deg,
                        0.0,
                        0.0,
                        0.0,
                    );
                    let mut reach = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
                    for (x, y) in [(0.0, 0.0), (size.0, 0.0), (0.0, size.1), size] {
                        let px = m[0][0] * x + m[1][0] * y + m[3][0];
                        let py = m[0][1] * x + m[1][1] * y + m[3][1];
                        reach = [
                            reach[0].min(px),
                            reach[1].min(py),
                            reach[2].max(px),
                            reach[3].max(py),
                        ];
                    }
                    let bounds = Bounds::covering(reach, w, h, plan.room);
                    Some((
                        self.compositor.composite_seeded(
                            &self.ctx,
                            w,
                            h,
                            [0.0, 0.0, 0.0, 0.0],
                            &[lumit_gpu::composite::CompositeLayer {
                                texture: &linear,
                                size,
                                position,
                                anchor,
                                scale: draw.scale,
                                rotation_deg: draw.rotation_deg,
                                opacity: draw.opacity,
                                matte: None,
                                blend: lumit_gpu::composite::Blend::Normal,
                                z: 0.0,
                                rotation_x_deg: 0.0,
                                rotation_y_deg: 0.0,
                                three_d: false,
                                layer_mask: None,
                                pre: None,
                                premultiplied: draw.premultiplied(),
                            }],
                            None,
                            None,
                            1.0,
                            1,
                            (bounds != frame).then(|| bounds.region()),
                        ),
                        bounds,
                    ))
                }
                GraphStep::Fx {
                    input,
                    ops,
                    matte,
                    picture,
                    picture_depth,
                    colour_tables,
                    flare_lens_files,
                    roto,
                    plane,
                    baked_from,
                    points,
                    ..
                } => {
                    let (tex, from) =
                        held(&made, input).unwrap_or_else(|| (self.transparent(w, h), frame));
                    // The box's px parameters were resolved in the graph's own
                    // frame; this is the raster they run on.
                    let mut ops = ops.clone();
                    ops.rescale_spatial(scale);
                    // Where the box's picture can reach, asked of each op with
                    // its places still measured from the frame's corner. The
                    // box runs on a raster that holds both that and what it
                    // was handed, with every place moved in to match.
                    let mut probe = ops.clone();
                    probe.set_in([0.0, 0.0], [0.0, 0.0, w as f32, h as f32]);
                    let want = Bounds::covering(
                        probe
                            .iter()
                            .fold(from.rect(), |rect, op| op.def.bounds(op.params, rect)),
                        w,
                        h,
                        plan.room,
                    );
                    let room = from.union(want);
                    if room != frame {
                        let (x, y) = (-room.x as f32, -room.y as f32);
                        ops.set_in([x, y], [x, y, w as f32, h as f32]);
                    }
                    let tex = place((tex, from), room);
                    let luts = self.load_tables(colour_tables);
                    let flare_lens = self.load_flare_lens(flare_lens_files);
                    let bind = |step: &Option<usize>| match held(&made, step) {
                        Some(t) => LayerInput::Texture(place(t, room)),
                        None => LayerInput::Absent,
                    };
                    // What an analysis filed for the file this box reads, laid
                    // where that file's own picture is laid: the Read step it
                    // was made from says where. One op, so one slot apiece.
                    let read = baked_from.and_then(|i| match plan.steps.get(i) {
                        Some(GraphStep::Read(draw)) => Some(&**draw),
                        _ => None,
                    });
                    let roto = [roto.as_ref().zip(read).map(|(m, read)| {
                        let matte = upload_roto_matte(&self.ctx, m);
                        let laid = self.lay_as_read(&matte, (m.width, m.height), read, w, h, scale);
                        place((laid, frame), room)
                    })];
                    let planes = [plane.as_ref().zip(read).map(|(p, read)| {
                        let plane = self.upload_plane(p);
                        let laid = self.lay_as_read(&plane, (p.width, p.height), read, w, h, scale);
                        place((laid, frame), room)
                    })];
                    let out = crate::fxops::run_ops(
                        self.fx,
                        &self.ctx,
                        tex,
                        room.w,
                        room.h,
                        &ops,
                        &[],
                        &[],
                        &luts,
                        &[match bind(picture) {
                            LayerInput::Texture(t) if *picture_depth => LayerInput::Depth(t, None),
                            other => other,
                        }],
                        &flare_lens,
                        &[bind(matte)],
                        // A graph has no masks and no graph of its own to run
                        // here: a nested graph was lowered into these very
                        // steps rather than left as an op.
                        &[],
                        points,
                        &crate::fxops::Side {
                            roto: &roto,
                            planes: &planes,
                            ..crate::fxops::Side::NONE
                        },
                        &[],
                        None,
                        // Nothing names a box's picture in v1, so every step
                        // runs uncached; the comp above it is cached by its
                        // frame key as any comp is.
                        None,
                    );
                    // An op that spreads, a blur, grows the raster evenly on
                    // each side. The picture keeps that, and what the ops
                    // said it reaches, and no more.
                    let grown = Bounds {
                        x: room.x - lumit_gpu::fx::centred(out.width(), room.w),
                        y: room.y - lumit_gpu::fx::centred(out.height(), room.h),
                        w: out.width(),
                        h: out.height(),
                    };
                    let [x0, y0, x1, y1] = want.rect();
                    let (gx, gy) = ((room.x - grown.x) as f32, (room.y - grown.y) as f32);
                    let keep =
                        Bounds::covering([x0 - gx, y0 - gy, x1 + gx, y1 + gy], w, h, plan.room);
                    Some((place((out, grown), keep), keep))
                }
                // A over B, both full-frame at identity on a transparent
                // ground: B first at Normal and full opacity, then A with the
                // mode and the opacity. An unwired A gives B; an unwired B
                // gives A over nothing.
                GraphStep::Dissolve {
                    input,
                    output,
                    matte,
                    channel,
                    invert,
                } => {
                    let pictures = [
                        held(&made, input),
                        held(&made, output),
                        held(&made, &Some(*matte)),
                    ];
                    let room = pictures
                        .iter()
                        .flatten()
                        .fold(frame, |all, (_, at)| all.union(*at));
                    let [input, output, matte] = pictures.map(|p| match p {
                        Some(p) => place(p, room),
                        None => self.transparent(room.w, room.h),
                    });
                    // The Channel pick and Invert as run_ops reads them.
                    let matte = match channel {
                        Some(c) if lumit_core::fx::cpu::matte_needs_prepare(*c, *invert) => self
                            .fx
                            .matte_prepare(&self.ctx, &matte, room.w, room.h, *c, *invert),
                        _ => matte,
                    };
                    let invert = channel.is_none() && *invert;
                    Some((
                        self.fx
                            .matte_mix(&self.ctx, &input, &output, &matte, room.w, room.h, invert),
                        room,
                    ))
                }
                GraphStep::Merge {
                    a,
                    b,
                    mode,
                    opacity,
                } => {
                    let (a, b) = (held(&made, a), held(&made, b));
                    if a.is_none() && b.is_none() {
                        None
                    } else {
                        // Each laid where it sits, on a raster that holds both.
                        fn full(
                            (texture, at): &(wgpu::Texture, Bounds),
                            blend: lumit_gpu::composite::Blend,
                            opacity: f32,
                        ) -> lumit_gpu::composite::CompositeLayer<'_> {
                            lumit_gpu::composite::CompositeLayer {
                                texture,
                                size: (at.w as f32, at.h as f32),
                                position: (at.x as f32, at.y as f32),
                                anchor: (0.0, 0.0),
                                scale: (100.0, 100.0),
                                rotation_deg: 0.0,
                                opacity,
                                matte: None,
                                blend,
                                z: 0.0,
                                rotation_x_deg: 0.0,
                                rotation_y_deg: 0.0,
                                three_d: false,
                                layer_mask: None,
                                pre: None,
                                // Every picture in a graph has been composited.
                                premultiplied: true,
                            }
                        }
                        let bounds = [&a, &b]
                            .into_iter()
                            .flatten()
                            .fold(frame, |all, (_, at)| all.union(*at));
                        let mut layers = Vec::with_capacity(2);
                        if let Some(b) = &b {
                            layers.push(full(b, lumit_gpu::composite::Blend::Normal, 100.0));
                        }
                        if let Some(a) = &a {
                            let mode = lumit_core::model::BlendMode::ALL
                                .get(*mode as usize)
                                .copied()
                                .unwrap_or(lumit_core::model::BlendMode::Normal);
                            layers.push(full(a, crate::build::blend_of(mode), *opacity));
                        }
                        Some((
                            self.compositor.composite_seeded(
                                &self.ctx,
                                w,
                                h,
                                [0.0, 0.0, 0.0, 0.0],
                                &layers,
                                None,
                                None,
                                1.0,
                                1,
                                (bounds != frame).then(|| bounds.region()),
                            ),
                            bounds,
                        ))
                    }
                }
                // The picture the Index names, and transparent when that
                // socket is unwired or the index is out of range.
                GraphStep::Switch { inputs, index } => usize::try_from(*index)
                    .ok()
                    .and_then(|i| inputs.get(i))
                    .and_then(|step| held(&made, step)),
            };
            made.push(tex);
        }
        let shown = plan
            .output
            .and_then(|step| made.get(step).cloned().flatten());
        // A graph walked as a composition of its own tells the Viewer how far
        // its picture reaches. One applied to a layer has no outline to draw.
        if let (None, Some(cell)) = (provided, self.graph_reach) {
            let scale = scale.max(1e-6);
            cell.set(
                shown
                    .as_ref()
                    .map(|(_, bounds)| *bounds)
                    .filter(|bounds| *bounds != frame)
                    .map(|bounds| Reach {
                        rect: bounds.rect().map(|v| v / scale),
                        at_limit: bounds.at_limit(w, h, plan.room),
                    }),
            );
        }
        match plan.output {
            // Cut back to the frame, the one place that happens.
            Some(_) => shown.map_or_else(|| self.transparent(w, h), |pic| place(pic, frame)),
            // Nothing wired to the Output, and the passthrough a dangling
            // Node graph effect degrades to.
            None => provided.cloned().unwrap_or_else(|| self.transparent(w, h)),
        }
    }

    /// As [`Self::realise`], but compositing only `region` of the composition
    /// — the Viewer's region of interest. The returned texture is the
    /// region's size, not the comp's, and every layer lands where it would
    /// have, simply windowed.
    ///
    /// **A region is refused, and the whole frame composited, when the draw
    /// list holds an adjustment layer or a motion-blurring layer.** Both stage
    /// through a comp-sized intermediate whose maths is written against the
    /// comp raster, and a half-applied window there would be a wrong picture
    /// rather than a slow one. The caller crops the result instead, so what it
    /// gets back is the same picture either way — a region never changes the
    /// image, only how much of it is computed. `realise` itself is the
    /// no-region entry, which is also what the nested-comp recursion below
    /// calls: a Precomp renders itself entire, and this comp's window applies
    /// once, to the finished thing.
    ///
    /// `ground` is a 3D viewport's ground: the floor is drawn into the picture
    /// wherever no 3D layer is nearer. Only ever given for the comp a viewport
    /// shows, never for a Precomp inside it.
    #[allow(clippy::too_many_arguments)]
    pub fn realise_region(
        &self,
        camera: Option<lumit_core::model::CameraPose>,
        width: u32,
        height: u32,
        background: [f64; 4],
        layers: &[CompLayerDraw],
        region: Option<lumit_gpu::composite::Region>,
        ground: Option<ViewGround>,
    ) -> wgpu::Texture {
        let region = region.filter(|_| region_is_safe(layers));
        // Depth is what tells the profiler which layers are rows of the
        // composition being rendered and which are inside a Precomp (see
        // crate::profile). Paired around the whole walk, recursion included, so
        // a nested comp's layers can never be mistaken for this comp's.
        if let Some(p) = self.profiler {
            p.enter_comp();
        }
        // One command buffer for the whole walk, recursion included. Every pass
        // below records into it and nothing reaches the driver until the
        // outermost `end_frame`, so a frame costs one round trip rather than one
        // per layer and per effect. `begin_frame` nests, which is what lets this
        // sit on the recursive entry point rather than being threaded by hand.
        //
        // It is also the one place a frame's video memory can still be refused
        // (docs/13 §3): a texture cannot be denied half way through a pass, so
        // the whole frame is reserved for before a single one of them records.
        let (tw, th) = lumit_gpu::composite::scaled_size(width, height, self.render_scale);
        self.open_frame(tw, th, layers);
        let out = self.realise_at_depth(camera, width, height, background, layers, region, ground);
        self.ctx.end_frame();
        if let Some(p) = self.profiler {
            p.leave_comp();
        }
        out
    }

    /// Reserve this frame's video memory and open its command batch, stepping
    /// down the degradation ladder (docs/13 §4) if the governor says no.
    ///
    /// # In plain terms
    ///
    /// The two rungs the renderer owns are here, in order and cheapest first:
    /// stop filling the intermediate cache, then give the cold half of it back.
    /// Both are invisible — the picture that comes out is identical, only made
    /// again rather than read back — which is exactly why they are the rungs
    /// that may be taken on a memory reading. The rungs that *are* visible (the
    /// preview resolution tier, tiling, the flow swap, the calm banner) change
    /// what is drawn, so they belong to the caller, which reads the denial
    /// count and steps down deliberately; export never takes them at all.
    ///
    /// A frame that is still refused after both is **rendered anyway**. There
    /// is nothing honest to return instead — a texture cannot be refused half
    /// way through a pass, and a black frame is a worse answer than a slow one
    /// — so what the refusal buys is a truthful readout and a governor that
    /// says no earlier next time, not an abort. Nested frames take this path
    /// too and reserve nothing: they are already inside their parent's
    /// reservation, and counting them again would refuse a frame that fits.
    fn open_frame(&self, tw: u32, th: u32, layers: &[CompLayerDraw]) {
        let estimate = self.frame_estimate(tw, th, layers);
        if self.ctx.try_begin_frame(estimate).is_ok() {
            return;
        }
        // Rung one is already taken and takes itself back: the effect walk
        // reads the same pressure and stops filing while it lasts
        // ([`crate::fxops::run_ops`]), so there is no flag here to set and
        // later remember to clear. Rung two is this.
        self.fx_cache.borrow_mut().trim_under_pressure();
        self.let_go_of_kept();
        if self.ctx.try_begin_frame(estimate).is_err() {
            self.ctx.begin_frame();
        }
    }

    /// What this frame is expected to hold on the card at its **peak** — not
    /// its total, which is a far larger and far less useful number.
    ///
    /// Layers are composited one at a time into one accumulator and the effect
    /// walk ping-pongs through the frame's texture pool, so a hundred-layer
    /// comp peaks at a handful of pictures rather than a hundred. What does
    /// grow with the project is the layers that stage through a comp-sized
    /// intermediate of their own — an adjustment layer, which needs everything
    /// below it composited first, and a motion-blurred one, which holds an
    /// accumulator across its shutter. They are the same two the region
    /// window steps aside for ([`region_is_safe`]), and for the same reason.
    fn frame_estimate(&self, tw: u32, th: u32, layers: &[CompLayerDraw]) -> u64 {
        /// The accumulator, the layer being drawn, the two the effect walk
        /// passes between, and headroom for the matte or plate an effect reads
        /// beside them.
        const FRAME_PEAK_TEXTURES: u64 = 6;
        let staged = layers
            .iter()
            .filter(|l| {
                matches!(l.source, DrawSource::Adjust | DrawSource::Haze(_)) || !l.mb.is_empty()
            })
            .count() as u64;
        lumit_gpu::texture_bytes(self.ctx.working(), tw, th)
            .saturating_mul(FRAME_PEAK_TEXTURES.saturating_add(staged))
    }

    #[allow(clippy::too_many_arguments)]
    fn realise_at_depth(
        &self,
        camera: Option<lumit_core::model::CameraPose>,
        width: u32,
        height: u32,
        background: [f64; 4],
        layers: &[CompLayerDraw],
        region: Option<lumit_gpu::composite::Region>,
        mut ground: Option<ViewGround>,
    ) -> wgpu::Texture {
        // The actual raster this comp's composites land on; all geometry
        // below stays in the logical `width`×`height` comp pixels.
        let (tw, th) = lumit_gpu::composite::scaled_size(width, height, self.render_scale);
        let (surround, layers) = crate::environment::peel(layers);
        // Only the look a comp exports with takes it, the others being for
        // seeing the models themselves.
        let surround = surround.filter(|_| crate::model::display().takes_shadows());
        let lit_by = surround.map(|d| (d, self.environment_light(d)));
        let mut acc = lit_by
            .as_ref()
            .filter(|(d, _)| d.background)
            .and_then(|(d, e)| self.environment_background(e, d, camera, (width, height)));
        let environment = lit_by.as_ref().map(|(_, e)| e);
        let mut start = 0usize;
        // The depth of the layers below, made only when an adjustment layer's
        // effect reads it, as metres per unit of the camera's view.
        let metres_per_w = camera.and_then(|pose| {
            let ppm = layers
                .iter()
                .find_map(|l| below_px_per_metre(&l.dof_inputs))?;
            Some((pose.zoom / ppm.max(1e-6)) as f32)
        });
        let mut depth: Option<wgpu::Texture> = None;
        for (i, l) in layers.iter().enumerate() {
            // The haze ends its segment, and so does the picture glass bends.
            let haze = matches!(l.source, DrawSource::Haze(_));
            let glass = crate::model_layer::glass_starts(layers, i);
            if !haze && !glass && !matches!(l.source, DrawSource::Adjust) {
                continue;
            }
            let below = self.realise_segment(
                camera,
                width,
                height,
                background,
                &layers[start..i + usize::from(haze)],
                &acc,
                region,
                None,
                None,
                metres_per_w.map(|m| (m, &mut depth)),
                if haze { ground.take() } else { None },
                environment,
            );
            if haze || glass {
                (acc, start) = (Some(below), i + usize::from(haze));
                let at = (width, height);
                self.lay_behind(layers, i, &acc, camera, at, background, region, environment);
                continue;
            }
            // An adjustment layer processes the composite below, which has no
            // footage neighbour frames. A temporal effect's neighbours are
            // the below-stack built again (`measure_below_flow`). Its LUT and
            // depth-of-field effects still apply (§3.11, §3.22): load/render
            // them the same way the per-layer path does, so preview stays
            // identical to export. The adjustment stack runs on the
            // comp-sized composite, so its depth inputs resample to comp size.
            let tables = self.load_tables(&l.colour_tables);
            let layer_inputs =
                self.render_layer_inputs(&l.dof_inputs, tw, th, l.natural_size, depth.as_ref());
            let mattes = self.render_layer_inputs(&l.mattes, tw, th, l.natural_size, None);
            let flare_lens = self.load_flare_lens(&l.flare_lens_files);
            // The stack was resolved against the comp raster; this render
            // target may be smaller (reduced-resolution preview), and every
            // px-dimensioned parameter must shrink with it or the flare's
            // light (and every aperture and radius) lands past where the
            // user put it.
            let fx_ops = match l.fx_ref_width {
                Some(ref_w) if ref_w > 0.0 => {
                    let mut ops = l.fx.clone();
                    ops.rescale_spatial(tw as f32 / ref_w);
                    ops
                }
                _ => l.fx.clone(),
            };
            // Posterize Time everything-below (docs/08 §3.25): the input this
            // adjustment's own effects run on is the below-stack held at the
            // posterised time, not the plain below-composite. The held draws and
            // camera were built by the shared `below_draws_at` (identical to the
            // texture export's `render_below_at` produces); the coverage
            // blend below still lays the result over the live below-at-t, so a
            // mask reveals the held region. None on an ordinary adjustment.
            // Accumulation motion blur (docs/08 §3.26) takes precedence: it
            // renders N sub-frame below-stacks and averages them; else Posterize
            // holds one below-stack; else the plain below-composite.
            // An adjustment layer's own cost starts here: everything below it
            // has been composited (and timed as its own layers), and what
            // follows — the held or accumulated below-stack, this stack, the
            // coverage and the blend — is what this row is spending.
            let (started, mut fx_ms) = self.layer_clock();
            let fx_input = if let Some(ab) = &l.accumulation_below {
                self.accumulate_below(width, height, background, ab, &below)
            } else if let Some(tb) = &l.temporal_below {
                self.realise(tb.camera, width, height, background, &tb.draws)
            } else {
                below.clone()
            };
            // **The motion of the composite below** (docs/08 §3.2). An
            // adjustment layer has no decoded frames, so its Motion blur
            // and Datamosh used to bind nothing and pass through — on exactly
            // the layer §3.2 calls the commonest place to put the effect. The
            // below-stack was built again at each neighbour time; realise it
            // and measure between the two here, where both are textures.
            // Empty unless one of those effects is live.
            let (flow_neighbours, flow) =
                self.measure_below_flow(l, &fx_input, width, height, background);
            // A Node graph effect on an adjustment layer applies to the
            // composite below, which is the picture it is handed - so the
            // closures are built here exactly as they are for a layer's own
            // stack.
            let plans = self.graph_closures(l, tw);
            let graphs: Vec<&dyn Fn(wgpu::Texture, u32, u32) -> wgpu::Texture> =
                plans.iter().map(|f| &**f).collect();
            // The adjustment's own stack, coverage and blend all run on the
            // ACTUAL raster: `adjust_blend` reads its three inputs texel by
            // texel, so they must agree on their size.
            let processed = crate::fxops::run_ops(
                self.fx,
                &self.ctx,
                fx_input,
                tw,
                th,
                &fx_ops,
                &flow_neighbours,
                &flow,
                &tables,
                &layer_inputs,
                &flare_lens,
                &mattes,
                &l.mask_paths,
                &l.points_schedules,
                // An adjustment layer has no source frames, so nothing was
                // ever propagated through it: a Roto brush there passes
                // through. It can still stamp clones over a late stream.
                &crate::fxops::Side {
                    late: Some(&|slot: usize, points: usize| {
                        self.late_clones(l.dof_inputs.get(slot), points, tw, th, l.natural_size)
                    }),
                    ..crate::fxops::Side::NONE
                },
                &graphs,
                fx_ms.as_mut(),
                // The composite below carries no name in v1.
                None,
            );
            // An adjustment layer's stack cannot grow the raster: what
            // follows blends it against the composite beneath it, texel for
            // texel, and there is no "beneath" outside the comp to grow into.
            // A Tile above 100 % output on an adjustment layer therefore reads
            // as the plain clipped tiling it always did.
            let processed = lumit_gpu::fx::fit_centred(&self.ctx, processed, tw, th);
            let coverage = self.coverage_texture(camera, width, height, l);
            acc = Some(self.fx.adjust_blend(
                &self.ctx,
                &below,
                &processed,
                &coverage,
                tw,
                th,
                (l.opacity / 100.0).clamp(0.0, 1.0),
            ));
            self.layer_done(l, started, fx_ms);
            start = i + 1;
        }
        self.realise_segment(
            camera,
            width,
            height,
            background,
            &layers[start..],
            &acc,
            region,
            None,
            None,
            None,
            ground,
            environment,
        )
    }

    /// Accumulation motion blur (docs/08 §3.26, docs/impl/temporal-rerender.md
    /// §3): render each sub-frame below-stack through the same realise path,
    /// average the N finished composites with the hardware additive-at-`1/N` pass
    /// ([`lumit_gpu::composite::Compositor::accumulate`]), then blend that average against
    /// the frame-time below-composite `below` by `mix` (a linear interpolation
    /// the additive blend gives exactly). The result stands in for the
    /// below-composite the adjustment's own effects and coverage blend see. A
    /// still scene averages back to `below` bit-for-bit; a moving one smears.
    /// Export runs the identical combine, so the two agree.
    fn accumulate_below(
        &self,
        width: u32,
        height: u32,
        background: [f64; 4],
        ab: &AccumulationBelow,
        below: &wgpu::Texture,
    ) -> wgpu::Texture {
        if ab.samples.is_empty() {
            // No samples (N < 2) degrades to the plain below — never a panic.
            return below.clone();
        }
        // The sub-frames and `below` are all at the ACTUAL raster size; the
        // combine is a full-frame identity pass, so it runs at that size too.
        let (tw, th) = lumit_gpu::composite::scaled_size(width, height, self.render_scale);
        // **The Matte scales Shutter angle per pixel**. With none bound
        // — the default, and every project saved before this — the equal-weight
        // hardware pass below runs exactly as it always did, byte for byte.
        // With one, each pixel's weights come from how far open its
        // own shutter is, and the average is taken over a shorter slice of the
        // same N moments, shrunk toward the frame's own instant.
        let matte = self
            .render_layer_inputs(
                std::slice::from_ref(&ab.matte),
                tw,
                th,
                (width as f32, height as f32),
                None,
            )
            .into_iter()
            .next()
            .and_then(|slot| slot.texture(below).cloned());
        // Each sample is rendered, folded into the running average, and let go
        // of before the next one starts.
        //
        // **This is the whole shape of the thing.** The samples used to be
        // collected into a `Vec` and handed to the combine in one go, which
        // meant N finished composites alive together: a thirty-two-sample
        // shutter at 4K is two gigabytes of them, waiting for a pass that reads
        // each exactly once and never looks at it again. Both combines were
        // already one-sample-at-a-time internally; all that was needed was to
        // let the samples arrive that way.
        //
        // Peak memory is now flat in N — the accumulator plus whichever sample
        // is in hand — and the arithmetic is unchanged, which is the part that
        // had to be got right rather than merely made smaller. The equal-weight
        // path still sums in fp32 across the same two ping-ponged targets, in
        // the same order, resolving once at the end;
        // `streaming_accumulation_matches_the_one_shot` in `lumit-gpu` holds
        // that byte for byte against the previous implementation.
        let n = ab.samples.len();
        let sample_at = |k: usize| -> Option<wgpu::Texture> {
            let (draws, camera) = ab.samples.get(k)?;
            let frame = self.realise(*camera, width, height, background, draws);
            // Hand this sample's work to the card and wait for it before
            // starting the next. A frame is one batch, and nothing a batch
            // allocates comes back until it has run: with the N samples inside
            // it, every sample's intermediates stayed alive together. Eight
            // samples over one 1080p layer held 2.1 GB against 200 MB for the
            // frame alone, which is enough to take a card down. Waiting per
            // sample keeps one sample's scratch alive at a time. The wait costs
            // the overlap between encoding a sample and drawing the last, small
            // beside N full renders.
            self.ctx.flush();
            self.ctx.settle();
            Some(frame)
        };

        let average = if let Some(matte) = matte {
            // Channel and Invert, once, before anything reads it. The
            // dispatch seam does this for every other effect; this one has no
            // dispatch, so it does it here.
            let matte =
                if lumit_core::fx::cpu::matte_needs_prepare(ab.matte_channel, ab.matte_invert) {
                    self.fx.matte_prepare(
                        &self.ctx,
                        &matte,
                        tw,
                        th,
                        ab.matte_channel,
                        ab.matte_invert,
                    )
                } else {
                    matte
                };
            let mut acc: Option<wgpu::Texture> = None;
            for k in 0..n {
                let Some(frame) = sample_at(k) else { continue };
                acc = Some(self.fx.accumulate_shutter_step(
                    &self.ctx,
                    acc.as_ref(),
                    &frame,
                    &matte,
                    tw,
                    th,
                    ab.anchor,
                    n as f32,
                    k,
                ));
            }
            // `n` is not zero — the empty case returned above — so the loop ran
            // at least once unless every sample failed to render, which is the
            // plain below rather than a panic.
            match acc {
                Some(acc) => acc,
                None => return below.clone(),
            }
        } else {
            // Equal weights 1/N sum to 1: the premultiplied arithmetic mean.
            let weight = 1.0 / n as f32;
            let mut running = self.compositor.accumulator(&self.ctx, tw, th);
            for k in 0..n {
                let Some(frame) = sample_at(k) else { continue };
                self.compositor
                    .add_into(&self.ctx, &mut running, &frame, weight);
            }
            self.compositor.resolve(&self.ctx, &running)
        };
        if ab.mix >= 1.0 {
            average
        } else {
            // Mix blends the blurred average against the live below-composite.
            self.compositor.accumulate(
                &self.ctx,
                tw,
                th,
                &[(below, 1.0 - ab.mix), (&average, ab.mix)],
            )
        }
    }

    /// **Motion measured between two composites** (docs/08 §3.2): the
    /// picture this layer's effects run on, and the same picture built again at
    /// each neighbour time ([`CompLayerDraw::flow_below`]).
    ///
    /// Returns the neighbour pictures and the fields measured against them, in
    /// the same shape a footage layer's decoded neighbours and fields arrive in
    /// — so nothing downstream knows where they came from. Both halves matter:
    /// Motion blur reads only the field, Datamosh also drags the `-1`
    /// picture along it, and the contract that each consumer gets the
    /// measurement it asked for holds here because the builder emitted one
    /// entry per offset the stack wanted.
    ///
    /// Empty in, empty out — and empty is the common case, so an ordinary
    /// frame does not reach the flow engine at all.
    #[allow(clippy::type_complexity)]
    fn measure_below_flow(
        &self,
        l: &CompLayerDraw,
        now: &wgpu::Texture,
        width: u32,
        height: u32,
        background: [f64; 4],
    ) -> (Vec<(i32, wgpu::Texture)>, Vec<(i32, wgpu::Texture)>) {
        let mut neighbours = Vec::with_capacity(l.flow_below.len());
        let mut fields = Vec::with_capacity(l.flow_below.len());
        for (offset, draws, camera, measure) in &l.flow_below {
            let then = self.realise(*camera, width, height, background, draws);
            // Only an offset a flow consumer asked for is measured. An Echo
            // or a plugin reads the neighbour picture and nothing more. A
            // field measured on the other side of the frame is turned round.
            if let Some((under, want)) = *measure {
                if let Some(field) = self.field_to(now, &then, under != *offset, want) {
                    fields.push((under, field));
                }
            }
            neighbours.push((*offset, then));
        }
        (neighbours, fields)
    }

    /// One measurement, entirely on the card: two pictures in, the `rgba32float`
    /// field the kernels read out (docs/08 §3.2).
    ///
    /// The flow engine's texture entry point converts each picture to the luma
    /// its pyramid starts from with one compute pass, so neither composite is
    /// ever read back — a readback of two 1080p frames costs several times what
    /// the measurement does.
    ///
    /// `None` is always a degrade and never a fault: no backend was given, the
    /// device has no GPU flow, or the solver faulted (in which case the engine
    /// has already given up on the GPU for good). The caller then binds no
    /// field, and an effect with no field is its documented passthrough.
    ///
    /// ponytail: measured per realise, not cached across them. The decode
    /// worker's flow cache keys by *content* — the source item and the two
    /// frames — and a composite has no such name (`fx_input_key` is `None` for
    /// exactly this reason), so filing one under the layer and the time
    /// would go stale the moment somebody edited a layer below. Caching would
    /// also save the smaller half: the neighbour composite is a whole extra comp
    /// render and the flow is ~8 ms on top of it. So the ceiling is a doubled
    /// comp render for every frame that reaches a flow effect above an
    /// adjustment, and the trigger is that doubling measured: docs/13 B3's
    /// scrub p95 on a comp whose flow effect sits on an adjustment layer
    /// against the same effect over footage, where the decode worker's cache
    /// answers instead. If the composite path is the one missing B3's 50 ms
    /// while the footage path holds it, this is the whole of the difference.
    /// The upgrade is to name the below-composite the way a nested comp's
    /// frame is named, and then both halves can be cached under the one name.
    fn measure_flow(
        &self,
        a: &wgpu::Texture,
        b: &wgpu::Texture,
        turned: bool,
    ) -> Option<wgpu::Texture> {
        let cell = self.flow?;
        // The solver reads these two pictures on an encoder of its own, and a
        // command that has not been submitted has not run.
        self.ctx.flush();
        let mut held = cell.borrow_mut();
        let engine = held
            .engine
            .get_or_insert_with(|| lumit_flow::FlowEngine::with_context(&self.ctx));
        // Half resolution, the same working size the decode worker measures an
        // effect's motion at and for its reasons (docs/impl/optical-flow.md §1):
        // a wider patch relative to repeating detail, at a quarter of the cost.
        // Sharing that setting is also what keeps a composite measured the way
        // footage is, rather than by a second rule nobody wrote down.
        let set = lumit_flow::FlowSettings {
            divisor: 2,
            ..lumit_flow::FlowSettings::default()
        };
        let (fwd, bwd) = engine.flow_pair_textures(a, b, &set)?;
        // The forward–backward confidence the streak is steered by (FX-19), and
        // the field brought up to the raster the effect runs on — the same two
        // deterministic functions the decode path applies to a measured pair, so
        // a composite's field and a footage layer's are the same kind of thing.
        let conf = lumit_flow::confidence(&fwd, &bwd);
        let (w, h) = (a.width(), a.height());
        let (mut u, mut v, c) = lumit_flow::field_to_size(&fwd, &conf, w as usize, h as usize);
        if turned {
            u.iter_mut().chain(v.iter_mut()).for_each(|x| *x = -*x);
        }
        Some(lumit_gpu::fx::common::upload_flow_field(
            &self.ctx, &u, &v, &c, w, h,
        ))
    }

    /// The adjustment layer's comp-space coverage (docs/06 §1.5): its mask
    /// raster — white where the effects apply — placed by its transform,
    /// so the transform moves the coverage map, never the picture. No
    /// masks means full coverage (a white quad over the whole comp).
    /// One matte source rendered alone into comp space.
    ///
    /// Deliberately at FULL comp resolution whatever the render scale: the
    /// fragment samples the matte by normalised comp UV, so any size is
    /// correct — shrink it later if it ever shows in a profile.
    ///
    /// Both places a matte gates something come through here — an ordinary
    /// layer's picture, and an adjustment layer's coverage — so the two can
    /// never disagree about what a matte source looks like.
    ///
    /// `region` is the window the layer it gates is drawn into. The matte is
    /// read by where it sits in that target, so it has to cover the same window
    /// or the whole comp gets squeezed into it.
    fn matte_texture(
        &self,
        width: u32,
        height: u32,
        cam_mat: Option<glam::Mat4>,
        m: &crate::draw::MatteDraw,
        region: Option<lumit_gpu::composite::Region>,
    ) -> wgpu::Texture {
        // A Precomp matte realises its nested comp exactly as a Precomp
        // layer's picture does (the layer-input shape); anything else is the
        // uploaded source pixels.
        let linear = if let Some(n) = &m.nested {
            self.realise_nested(n.key, n.camera, n.width, n.height, n.background, &n.draws)
        } else {
            // Through the matte source's own colour space,
            // as its own picture would be: log footage read as
            // sRGB gates by the wrong shape, and a matte is
            // nothing but a shape.
            self.upload_source(&m.rgba, m.tex_w, m.tex_h, m.format, &m.colour_space)
        };
        // As for a layer input: a comp comes out at the render scale.
        let (lw, lh) = (linear.width(), linear.height());
        // After-effects matte: run the matte source's own
        // stack on its texture before it gates the consumer, so a keyed
        // or blurred matte works. Temporal inputs stay empty in v1 — the
        // matte source's echo/flow degrades to a still (documented). The
        // same run export performs, so the two agree.
        let linear = if m.fx.is_empty() {
            linear
        } else {
            let tables = self.load_tables(&m.colour_tables);
            // The matte source's **own** baked pictures, uploaded exactly as
            // the layer-input path uploads a referenced layer's. Without them
            // a track matte whose source wears a Roto brush or a Depth gated
            // the consumer with the source's plain footage.
            let roto: Vec<Option<wgpu::Texture>> = m
                .roto_mattes
                .iter()
                .map(|s| s.as_ref().map(|r| self.roto_texture(r, m.tex_w, m.tex_h)))
                .collect();
            let planes: Vec<Option<wgpu::Texture>> = m
                .planes
                .iter()
                .map(|slot| {
                    slot.as_ref()
                        .map(|p| self.plane_texture(p, m.tex_w, m.tex_h))
                })
                .collect();
            let side = crate::fxops::Side {
                roto: &roto,
                planes: &planes,
                late: None,
            };
            // The graphs the matte source's own Node graph effects apply,
            // at the raster its stack runs on.
            let px = if m.natural_size.0 > 0.0 {
                m.tex_w as f32 / m.natural_size.0
            } else {
                1.0
            };
            let plans: Vec<GraphClosure<'_>> = m
                .graph_fx
                .iter()
                .map(|plan| {
                    Box::new(move |tex: wgpu::Texture, gw: u32, gh: u32| {
                        self.realise_graph(plan, Some(&tex), gw, gh, px)
                    }) as GraphClosure<'_>
                })
                .collect();
            let graphs: Vec<&dyn Fn(wgpu::Texture, u32, u32) -> wgpu::Texture> =
                plans.iter().map(|f| &**f).collect();
            crate::fxops::run_ops(
                self.fx,
                &self.ctx,
                linear,
                m.tex_w,
                m.tex_h,
                &m.fx,
                &[],
                &[],
                &tables,
                &[],
                &self.load_flare_lens(&m.flare_lens_files),
                &[],
                // As above: a referenced layer's own stack walks no
                // mask path, and carries no birth schedule, in v1.
                &[],
                &[],
                &side,
                &graphs,
                // A matte's own stack is part of the layer it
                // gates, not a row of its own.
                None,
                // As above: unnamed in v1, so uncached.
                None,
            )
        };
        // A matte source's own stack cannot grow the raster
        // either: it is placed by `m.natural_size` below,
        // and a matte is a shape, not a picture that reaches
        // further. Cropped back to the size it was placed at.
        let linear = lumit_gpu::fx::fit_centred(&self.ctx, linear, lw, lh);
        self.compositor.composite_seeded(
            &self.ctx,
            width,
            height,
            [0.0, 0.0, 0.0, 0.0],
            &[lumit_gpu::composite::CompositeLayer {
                texture: &linear,
                size: m.natural_size,
                position: m.position,
                anchor: m.anchor,
                scale: m.scale,
                rotation_deg: m.rotation_deg,
                opacity: m.opacity,
                matte: None,
                blend: lumit_gpu::composite::Blend::Normal,
                z: m.z,
                rotation_x_deg: m.rotation_x_deg,
                rotation_y_deg: m.rotation_y_deg,
                three_d: m.three_d,
                layer_mask: None,
                pre: m.pre,
                premultiplied: m.nested.is_some(),
            }],
            cam_mat,
            None,
            1.0,
            1,
            region,
        )
    }

    fn coverage_texture(
        &self,
        camera: Option<lumit_core::model::CameraPose>,
        width: u32,
        height: u32,
        l: &CompLayerDraw,
    ) -> wgpu::Texture {
        let white = [255u8, 255, 255, 255];
        let (rgba, w, h): (&[u8], u32, u32) = match &l.mask_cov {
            Some((rgba, w, h)) => (rgba, *w, *h),
            None => (&white, 1, 1),
        };
        let src = self.engine.upload_srgb8(&self.ctx, rgba, w, h);
        let linear = self.engine.linearise(&self.ctx, &src);
        let cam_mat = camera.map(|pose| crate::export::camera_mat(width, height, pose));
        // **The adjustment layer's own Matte**. An adjustment has no
        // picture for a matte to gate — what it has is coverage, which is
        // exactly where the effect lands, so the matte multiplies into that.
        // The result reads the way it does on any other layer: the stack
        // applies where the matte says and nowhere else. `None` on every
        // adjustment without one, which composites as it always did.
        let matte_tex = l
            .matte
            .as_ref()
            // No region: a comp with an adjustment layer never gets one.
            .map(|m| self.matte_texture(width, height, cam_mat, m, None));
        let matte = matte_tex
            .as_ref()
            .map(|mt| lumit_gpu::composite::MatteInput {
                texture: mt,
                luma: l.matte.as_ref().is_some_and(|m| m.luma),
                inverted: l.matte.as_ref().is_some_and(|m| m.inverted),
            });
        // Rendered at the render scale: `adjust_blend` reads coverage texel by
        // texel against the below/processed rasters, so they must match.
        self.compositor.composite_seeded(
            &self.ctx,
            width,
            height,
            [0.0, 0.0, 0.0, 0.0],
            &[lumit_gpu::composite::CompositeLayer {
                texture: &linear,
                size: l.natural_size,
                position: l.position,
                anchor: l.anchor,
                scale: l.scale,
                rotation_deg: l.rotation_deg,
                // Layer opacity is applied once, in the blend itself.
                opacity: 100.0,
                matte,
                blend: lumit_gpu::composite::Blend::Normal,
                z: l.z,
                rotation_x_deg: l.rotation_x_deg,
                rotation_y_deg: l.rotation_y_deg,
                three_d: l.three_d,
                layer_mask: None,
                pre: l.pre,
                premultiplied: false,
            }],
            cam_mat,
            None,
            self.render_scale,
            self.samples,
            // Coverage is only asked for by an adjustment layer, and a region
            // is refused whenever the comp has one — see `realise_region`.
            None,
        )
    }

    // One more argument than clippy likes. The alternative is a struct that
    // exists only to be unpacked at the single call site.
    #[allow(clippy::too_many_arguments)]
    /// Composite one adjustment-free run of draws; `seed` (a previous
    /// stage's output) replaces the cleared background when present.
    pub(crate) fn realise_segment(
        &self,
        camera: Option<lumit_core::model::CameraPose>,
        width: u32,
        height: u32,
        background: [f64; 4],
        layers: &[CompLayerDraw],
        seed: &Option<wgpu::Texture>,
        region: Option<lumit_gpu::composite::Region>,
        scene_depth: Option<f32>,
        capture: Option<&mut Option<lumit_gpu::fx::lens_blur::SceneLayers>>,
        below: Option<(f32, &mut Option<wgpu::Texture>)>,
        ground: Option<ViewGround>,
        environment: Option<&lumit_gpu::fx::environment::EnvironmentLight>,
    ) -> wgpu::Texture {
        let (haze, layers) = crate::haze::peel(layers);
        let mut linear_textures: Vec<wgpu::Texture> = Vec::with_capacity(layers.len());
        // A layer's picture before the camera blurred it, for the depth pass,
        // where a small solid far off focus would otherwise fade out of it.
        let mut sharp: Vec<Option<(wgpu::Texture, (f32, f32))>> = Vec::new();
        // How much wider and taller each layer's finished picture is than the
        // source it started from. `(1.0, 1.0)` for every layer whose
        // stack holds no Tile above 100 % output, which is every layer until one
        // does — and multiplying a placement by 1.0 moves nothing, so the
        // composite below is byte for byte what it was.
        let mut grown: Vec<(f32, f32)> = Vec::with_capacity(layers.len());
        // Where a highlight is seen from: the camera the frame is drawn
        // through, which is the default one when the comp has no camera.
        let eye = camera
            .unwrap_or_else(|| {
                lumit_core::camera::default_pose(f64::from(width), f64::from(height))
            })
            .position;
        let eye = [eye.0, eye.1, eye.2].map(|v| v as f32);
        // A shadow needs every caster's picture before anything is lit, so
        // where this segment has one, the lighting waits for the whole list.
        let shadowed = casts_shadows(layers);
        let maps = self.shadow_maps(layers, haze);
        // There, a model is drawn after the 3D layers, whose casters it takes.
        let order = shadows::models_last(layers, shadowed);
        let mut early = None;
        let mut waiting = Vec::new();
        // Each model's depth, by its place in the list.
        let mut model_depths = Vec::new();
        // Particles in the scene: each one's depth by layer, and where its
        // emitter is, by its place in the list.
        let (seen, mut made) = self.scene_frame(camera, (width, height), layers);
        let mut scene_depths = Vec::with_capacity(layers.len());
        let mut emitters = Vec::new();
        for &i in &order {
            let l = &layers[i];
            // One row's own cost: its source uploaded and linearised (or its
            // Precomp realised entire) and then its effect stack. The composite
            // that follows is one pass over the whole segment rather than a
            // per-layer act, so it lands in the frame total instead
            // (crate::profile).
            let (started, mut fx_ms) = self.layer_clock();
            let entered = seen
                .as_ref()
                .filter(|_| scene::draws_in_scene(l))
                .map(|frame| scene::Seen::enter(frame, l));
            let (mut tex, mut grow, model_depth) = match made.get_mut(i).and_then(Option::take) {
                Some(made) => made,
                None => {
                    let model = matches!(l.source, DrawSource::Model(_));
                    let casters = (shadowed && model).then(|| {
                        let (sets, each) = early.get_or_insert_with(|| {
                            let so_far = (&order[..], &linear_textures[..], &grown[..]);
                            self.casters_so_far(layers, so_far, (width, height), eye)
                        });
                        each[i].0.and_then(|k| sets.get(k))
                    });
                    let shadows = (casters.flatten(), maps.as_ref(), environment);
                    self.layer_picture(l, camera, (width, height), shadows, &mut fx_ms)
                }
            };
            drop(entered);
            if let Some(depth) = model_depth {
                let holdout = matches!(&l.source, DrawSource::Model(m) if m.holds_out());
                model_depths.push((linear_textures.len(), depth, holdout));
            }
            let claimed = scene::claim(l).map(|(depth, emitter)| {
                emitters.push((linear_textures.len(), emitter));
                model_depths.push((linear_textures.len(), depth.clone(), false));
                depth
            });
            if shadowed {
                // Timed now and finished once it is lit, below, so its row
                // still shows the whole of its cost.
                let so_far = started.map(|started| {
                    self.ctx.flush();
                    self.ctx.device.poll(wgpu::Maintain::Wait);
                    started.elapsed()
                });
                waiting.push((so_far, fx_ms));
            } else {
                let blurred;
                (tex, grow, blurred) = self.lit(
                    l,
                    tex,
                    eye,
                    camera,
                    (None, None, None, environment),
                    grow,
                    (width, height),
                    claimed.as_ref(),
                );
                if scene_depth.is_some() || below.is_some() {
                    sharp.resize(linear_textures.len() + 1, None);
                    sharp[linear_textures.len()] = blurred;
                }
                self.layer_done(l, started, fx_ms);
            }
            grown.push(grow);
            linear_textures.push(tex);
            scene_depths.push(claimed);
        }
        linear_textures = shadows::unpermute(linear_textures, &order);
        grown = shadows::unpermute(grown, &order);
        let waiting = shadows::unpermute(waiting, &order);
        let scene_depths = shadows::unpermute(scene_depths, &order);
        let model_depths = model_depths
            .into_iter()
            .map(|(k, d, h)| (order[k], d, h))
            .collect();
        let emitters: Vec<_> = emitters.into_iter().map(|(k, e)| (order[k], e)).collect();
        if shadowed {
            let (sets, each) = early.unwrap_or_else(|| {
                self.shadow_casters(layers, &linear_textures, &grown, width, height, eye)
            });
            let lit: Vec<_> = linear_textures
                .into_iter()
                .zip(layers)
                .zip(each)
                .zip(waiting)
                .zip(&grown)
                .zip(&scene_depths)
                .map(
                    |(((((tex, l), (set, own)), (so_far, fx_ms)), grow), depth)| {
                        let started = so_far.and_then(|d| web_time::Instant::now().checked_sub(d));
                        let casters = set.and_then(|k| sets.get(k));
                        let size = (width, height);
                        let lit = self.lit(
                            l,
                            tex,
                            eye,
                            camera,
                            (casters, own, maps.as_ref(), environment),
                            *grow,
                            size,
                            depth.as_ref(),
                        );
                        self.layer_done(l, started, fx_ms);
                        lit
                    },
                )
                .collect();
            let keep_sharp = scene_depth.is_some() || below.is_some();
            linear_textures = Vec::with_capacity(lit.len());
            grown.clear();
            for (tex, grow, blurred) in lit {
                if keep_sharp {
                    sharp.push(blurred);
                }
                grown.push(grow);
                linear_textures.push(tex);
            }
        }
        let models = crate::model_layer::joined(&self.ctx, &mut linear_textures, model_depths);
        let cam_mat = camera.map(|pose| crate::export::camera_mat(width, height, pose));
        // Per-layer motion blur (docs/06 §4): a blurring layer's
        // fx-processed texture is drawn at each sub-frame placement and
        // averaged into one comp-sized smear by the shared helper both preview
        // and export call. The layer's real blend/opacity/matte/mask
        // then apply once to the averaged image, at the 1:1 composite below.
        let mb_textures: Vec<Option<wgpu::Texture>> = linear_textures
            .iter()
            .zip(layers)
            .zip(&grown)
            .map(|((tex, l), g)| {
                (!l.mb.is_empty()).then(|| {
                    // A grown picture is placed by the grown transform at every
                    // sub-frame too, or the smear would be of a layer
                    // in a different place from the one the composite draws.
                    let mb: Vec<lumit_gpu::composite::MbSample> = if *g == (1.0, 1.0) {
                        Vec::new()
                    } else {
                        let (dx, dy) = grow_offset(l.natural_size, *g);
                        l.mb.iter()
                            .map(|smp| lumit_gpu::composite::MbSample {
                                anchor: (smp.anchor.0 + dx, smp.anchor.1 + dy),
                                ..*smp
                            })
                            .collect()
                    };
                    self.compositor.motion_blur_average(
                        &self.ctx,
                        width,
                        height,
                        tex,
                        l.premultiplied(),
                        grow_size(l.natural_size, *g),
                        if mb.is_empty() { &l.mb } else { &mb },
                        l.three_d,
                        l.pre,
                        cam_mat,
                        self.render_scale,
                        self.samples,
                    )
                })
            })
            .collect();
        // Layer-space mask textures (Precomp masks — GPU mask pass). No colour
        // space is applied to one and none ever will be: this raster is a
        // shape's coverage, drawn here from the mask's own geometry, not image
        // content that arrived in somebody's colour space.
        let mask_textures: Vec<Option<wgpu::Texture>> = layers
            .iter()
            .zip(&grown)
            .map(|(l, g)| {
                l.mask_cov.as_ref().map(|(rgba, w, h)| {
                    // A grown picture is placed over a wider quad, and
                    // the mask is sampled across that quad — so a mask left at
                    // the layer's own size would stretch, and its edge would
                    // move. Grown into the same margin with nothing in it: the
                    // copies Tile put outside the layer are outside the mask,
                    // which is what a mask means.
                    let (mw, mh) = (
                        ((*w as f32 * g.0).round() as u32).max(*w),
                        ((*h as f32 * g.1).round() as u32).max(*h),
                    );
                    let src = self.engine.upload_srgb8(&self.ctx, rgba, *w, *h);
                    if (mw, mh) == (*w, *h) {
                        src
                    } else {
                        let linear = self.engine.linearise(&self.ctx, &src);
                        lumit_gpu::fx::fit_centred(&self.ctx, linear, mw, mh)
                    }
                })
            })
            .collect();
        // Matte layers render alone into comp space (one texture per consumer;
        // the shared-matte cache optimisation arrives with the evaluator).
        // Deliberately at FULL comp resolution whatever the render scale: the
        // fragment samples the matte by normalised comp UV, so any size is
        // correct — shrink it later if it ever shows in a profile.
        let matte_textures: Vec<Option<wgpu::Texture>> = layers
            .iter()
            .map(|l| {
                l.matte
                    .as_ref()
                    .map(|m| self.matte_texture(width, height, cam_mat, m, region))
            })
            .collect();
        let comp_layers: Vec<lumit_gpu::composite::CompositeLayer> = linear_textures
            .iter()
            .zip(layers)
            .zip(&matte_textures)
            .zip(&mask_textures)
            .zip(&mb_textures)
            .zip(&grown)
            .map(|(((((texture, l), matte_tex), mask_tex), mb_tex), g)| {
                let matte = matte_tex
                    .as_ref()
                    .map(|mt| lumit_gpu::composite::MatteInput {
                        texture: mt,
                        luma: l.matte.as_ref().is_some_and(|m| m.luma),
                        inverted: l.matte.as_ref().is_some_and(|m| m.inverted),
                    });
                match mb_tex {
                    // Motion-blurred: composite the averaged comp-sized smear
                    // 1:1 (identity placement), the layer's real blend, opacity,
                    // matte and mask applied once to the averaged image.
                    Some(avg) => lumit_gpu::composite::CompositeLayer {
                        texture: avg,
                        size: (width as f32, height as f32),
                        position: (0.0, 0.0),
                        anchor: (0.0, 0.0),
                        scale: (100.0, 100.0),
                        rotation_deg: 0.0,
                        opacity: l.opacity,
                        z: 0.0,
                        rotation_x_deg: 0.0,
                        rotation_y_deg: 0.0,
                        three_d: false,
                        matte,
                        blend: l.blend,
                        layer_mask: mask_tex.as_ref(),
                        pre: None,
                        // An average of copies is premultiplied, whatever they were.
                        premultiplied: true,
                    },
                    None => lumit_gpu::composite::CompositeLayer {
                        texture,
                        // A picture its stack grew covers more of layer
                        // space than the layer's own rectangle, evenly on all
                        // four sides — so the quad is that much bigger and the
                        // anchor moves with it, which leaves every pixel of the
                        // original exactly where it was.
                        size: grow_size(l.natural_size, *g),
                        position: l.position,
                        anchor: {
                            let (dx, dy) = grow_offset(l.natural_size, *g);
                            (l.anchor.0 + dx, l.anchor.1 + dy)
                        },
                        scale: l.scale,
                        rotation_deg: l.rotation_deg,
                        opacity: l.opacity,
                        z: l.z,
                        rotation_x_deg: l.rotation_x_deg,
                        rotation_y_deg: l.rotation_y_deg,
                        three_d: l.three_d,
                        matte,
                        blend: l.blend,
                        layer_mask: mask_tex.as_ref(),
                        pre: l.pre,
                        premultiplied: l.premultiplied(),
                    },
                }
            })
            .collect();
        let eye = camera.map(|pose| {
            [
                pose.position.0 as f32,
                pose.position.1 as f32,
                pose.position.2 as f32,
            ]
        });
        let (order, cuts): (Vec<usize>, Vec<Option<lumit_gpu::composite::Cut>>) = draw_order(
            layers,
            &linear_textures,
            &grown,
            cam_mat.zip(eye),
            &emitters,
        )
        .into_iter()
        .unzip();
        let comp_layers: Vec<lumit_gpu::composite::CompositeLayer> =
            order.iter().map(|&i| comp_layers[i].clone()).collect();
        // The depth pass reads each layer's coverage before the camera blurred
        // it.
        let depth_layers: Vec<lumit_gpu::composite::CompositeLayer> = order
            .iter()
            .zip(&comp_layers)
            .map(|(&i, c)| {
                match sharp
                    .get(i)
                    .and_then(Option::as_ref)
                    .filter(|_| layers[i].mb.is_empty())
                {
                    Some((texture, g)) => lumit_gpu::composite::CompositeLayer {
                        texture,
                        size: grow_size(layers[i].natural_size, *g),
                        anchor: {
                            let (dx, dy) = grow_offset(layers[i].natural_size, *g);
                            (layers[i].anchor.0 + dx, layers[i].anchor.1 + dy)
                        },
                        ..c.clone()
                    },
                    None => c.clone(),
                }
            })
            .collect();
        // The depth of everything so far, for an adjustment layer above that
        // reads it, laid over what the segments below left.
        if let (Some((metres_per_w, depth)), Some(cam)) = (below, cam_mat) {
            *depth = Some(self.compositor.composite_depth_joined(
                &self.ctx,
                width,
                height,
                &runs_of(layers, &order, &depth_layers),
                cam,
                metres_per_w,
                0.0,
                self.render_scale,
                depth.as_ref(),
                &models,
            ));
        }
        if let (Some(metres_per_w), Some(cam)) = (scene_depth, cam_mat) {
            let depth = self.compositor.composite_depth_joined(
                &self.ctx,
                width,
                height,
                &runs_of(layers, &order, &depth_layers),
                cam,
                metres_per_w,
                0.0,
                self.render_scale,
                None,
                &models,
            );
            // The 3D layers themselves too, for rays to meet: each picture and
            // its plane in the camera's frame, and the 2D layers drawn flat
            // behind everything.
            if let (Some(out), Some(pose)) = (capture, camera) {
                let frame = lumit_gpu::composite::camera_frame(
                    width as f32,
                    height as f32,
                    pose.zoom as f32,
                    (
                        pose.position.0 as f32,
                        pose.position.1 as f32,
                        pose.position.2 as f32,
                    ),
                    (
                        pose.rotation_deg.0 as f32,
                        pose.rotation_deg.1 as f32,
                        pose.rotation_deg.2 as f32,
                    ),
                );
                let mut planes = Vec::new();
                let mut textures = Vec::new();
                for ((texture, l), g) in linear_textures.iter().zip(layers).zip(&grown) {
                    if !l.three_d || shadow_only(l) || models.iter().any(|m| &m.colour == texture) {
                        continue;
                    }
                    let m = frame * placed_in_3d(texture, l, *g).world();
                    planes.push(lumit_gpu::fx::lens_blur::ScenePlane {
                        u: m.col(0).truncate().to_array(),
                        v: m.col(1).truncate().to_array(),
                        origin: m.col(3).truncate().to_array(),
                        opacity: (l.opacity / 100.0).clamp(0.0, 1.0),
                    });
                    textures.push(texture.clone());
                }
                let flat: Vec<lumit_gpu::composite::CompositeLayer> =
                    comp_layers.into_iter().filter(|c| !c.three_d).collect();
                let backdrop = (!flat.is_empty()).then(|| {
                    self.compositor.composite_seeded(
                        &self.ctx,
                        width,
                        height,
                        [0.0; 4],
                        &flat,
                        None,
                        None,
                        self.render_scale,
                        self.samples,
                        None,
                    )
                });
                *out = Some(lumit_gpu::fx::lens_blur::SceneLayers {
                    planes,
                    textures,
                    backdrop,
                    comp: (width as f32, height as f32),
                    zoom: pose.zoom as f32,
                    px_per_metre: pose.zoom as f32 / metres_per_w.max(1e-9),
                });
            }
            return depth;
        }
        // A cropped frame has no whole floor to draw, so a region goes without.
        let ground = match (ground, camera, cam_mat) {
            (Some(ground), Some(pose), Some(cam)) if region.is_none() => Some((ground, pose, cam)),
            _ => None,
        };
        // The floor goes in after the 3D layers, so 2D layers stacked above
        // them, titles and the like, draw over it.
        let split = match ground {
            Some(_) => comp_layers
                .iter()
                .rposition(|l| l.three_d)
                .map_or(0, |i| i + 1),
            None => comp_layers.len(),
        };
        let below = self.compositor.composite_joined(
            &self.ctx,
            width,
            height,
            background,
            &comp_layers[..split],
            cam_mat,
            seed.as_ref(),
            self.render_scale,
            self.samples,
            region,
            &cuts[..split],
            &models,
        );
        let below = self.haze_over(
            below,
            haze,
            (layers, &linear_textures, &grown),
            &comp_layers[..split],
            (&models, maps.as_ref()),
            camera,
            (width, height),
        );
        let Some((ground, pose, cam)) = ground else {
            return below;
        };
        let floored = self.ground_over(
            below,
            &comp_layers,
            &models,
            cam,
            &pose,
            (width, height),
            ground,
        );
        if split == comp_layers.len() {
            return floored;
        }
        self.compositor.composite_joined(
            &self.ctx,
            width,
            height,
            background,
            &comp_layers[split..],
            cam_mat,
            Some(&floored),
            self.render_scale,
            self.samples,
            region,
            &cuts[split..],
            &models,
        )
    }

    /// A 3D viewport's ground laid over `frame`, hidden wherever one of
    /// `layers` is nearer the eye.
    #[allow(clippy::too_many_arguments)]
    fn ground_over(
        &self,
        frame: wgpu::Texture,
        layers: &[lumit_gpu::composite::CompositeLayer<'_>],
        models: &[lumit_gpu::mesh::ModelPicture],
        cam: glam::Mat4,
        pose: &lumit_core::model::CameraPose,
        (width, height): (u32, u32),
        ground: ViewGround,
    ) -> wgpu::Texture {
        let (depth, near) = self.view_depth(layers, models, cam, (width, height), false);
        let [right, up, forward] = lumit_core::camera::axes(pose.rotation_deg);
        let v = |p: (f64, f64, f64)| [p.0 as f32, p.1 as f32, p.2 as f32];
        let (tw, th) = (frame.width(), frame.height());
        let op = lumit_gpu::fx::ground::GroundOp {
            eye: v(pose.position),
            right: v(right),
            up: v(up),
            forward: v(forward),
            zoom: pose.zoom.max(1.0) as f32,
            comp: (width as f32, height as f32),
            floor: ground.floor,
            middle: (ground.middle, 0.0),
            step: ground.step,
            scale: tw as f32 / width.max(1) as f32,
            near,
            colour: ground.colour,
        };
        self.fx.ground(&self.ctx, &frame, &depth, tw, th, &op)
    }
}

/// A 3D layer's picture where it really sits, motion blur or not: the plane
/// its rays meet and its place in the depth order.
pub(crate) fn placed_in_3d<'a>(
    texture: &'a wgpu::Texture,
    l: &CompLayerDraw,
    g: (f32, f32),
) -> lumit_gpu::composite::CompositeLayer<'a> {
    let (dx, dy) = grow_offset(l.natural_size, g);
    lumit_gpu::composite::CompositeLayer {
        texture,
        size: grow_size(l.natural_size, g),
        position: l.position,
        anchor: (l.anchor.0 + dx, l.anchor.1 + dy),
        scale: l.scale,
        rotation_deg: l.rotation_deg,
        opacity: l.opacity,
        z: l.z,
        rotation_x_deg: l.rotation_x_deg,
        rotation_y_deg: l.rotation_y_deg,
        three_d: true,
        matte: None,
        blend: l.blend,
        layer_mask: None,
        pre: l.pre,
        premultiplied: l.premultiplied(),
    }
}

/// The pixels per metre a slot that reads the depth below carries, if one does.
fn below_px_per_metre(slots: &[LayerInputDraw]) -> Option<f64> {
    slots.iter().find_map(|s| match s {
        LayerInputDraw::Below(ppm) => Some(*ppm),
        LayerInputDraw::Several(list) => below_px_per_metre(list),
        _ => None,
    })
}

/// Whether this layer's picture goes straight from its upload to the composite
/// draw, with nothing in between that wants the linear working format.
///
/// **In plain terms.** The composite samples a layer through a sampler, so an
/// eight-bit sRGB texture reads as the same linear values the linearise pass
/// would have written into a working-format copy of it. Everything else that
/// touches a layer's picture wants the working format: an effect stack, the
/// lighting pass, the camera's depth of field, and the motion-blur average,
/// whose fp32 accumulation is what keeps a still layer bit-exact (docs/06 §4).
/// One of those and the layer linearises as it always did.
fn straight_to_composite(l: &CompLayerDraw, camera: Option<lumit_core::model::CameraPose>) -> bool {
    l.fx.is_empty()
        && l.lights.is_empty()
        && l.mb.is_empty()
        && !(l.three_d && camera.is_some_and(|c| c.dof.is_some()))
        && l.material.casts_shadows == lumit_gpu::fx::lighting::LightingShadows::Off
}

/// Whether a region of interest can be applied to this draw list, or
/// whether the whole frame has to be composited and cropped instead.
///
/// Two stagings are written against a comp-sized intermediate: an **adjustment
/// layer**, whose effect stack runs on the composite of everything below it,
/// and a **motion-blurring layer**, whose sub-frame copies are averaged into a
/// comp-sized texture first. Windowing either of them halfway would produce a
/// wrong picture rather than a fast one, so the region steps aside and the
/// caller crops. Nothing about the image changes — only how much of it was
/// computed to get there.
fn region_is_safe(layers: &[CompLayerDraw]) -> bool {
    !layers.iter().any(CompLayerDraw::needs_whole_frame)
}

/// One propagated matte on the card: a gray8 plane at the source's own
/// raster, uploaded as the working linear format so it goes through the same
/// sampler every other auxiliary picture does.
///
/// Grey into all four channels. `set_matte` reads the luminance of whichever
/// channel it is told to, so the alpha carries the same number as the colour and
/// the plane reads correctly whichever way it is asked. The values are the
/// matte's own 0..1 and are **not** colour, so no transfer function is applied
/// to them — a coverage is a weight, not a picture.
/// A gray8 byte goes straight to the fp16 bits the texture holds, through a
/// 256-entry table — the same numbers the f32 route produced, without the
/// full-plane f32 temporary in between.
///
// ponytail: uploaded fresh every rendered frame. An unchanged matte — the
// common case, since a propagated run does not move while it is being watched
// — re-uploads the whole plane per frame per Roto layer. Memoising the texture
// per (effect instance, source frame) is the upgrade, and it belongs beside
// `LutCache`/`FxCache` on the `Realiser`'s owner rather than in the roto store,
// which is process-wide and knows nothing of a device. Observable trigger: a
// Roto layer's upload showing up in the docs/13 budget traces, or scrubbing a
// 4K propagated shot dropping frames the matte solve is not paying for.
fn upload_roto_matte(ctx: &lumit_gpu::GpuContext, m: &crate::draw::RotoMatteDraw) -> wgpu::Texture {
    let mut table = [0u16; 256];
    for (v, bits) in table.iter_mut().enumerate() {
        *bits = lumit_gpu::fx::common::f16_bits(v as f32 / 255.0);
    }
    let n = (m.width as usize) * (m.height as usize);
    let mut halfs = Vec::with_capacity(n * 4);
    for i in 0..n {
        let a = table[usize::from(m.gray.get(i).copied().unwrap_or(0))];
        halfs.extend_from_slice(&[a, a, a, a]);
    }
    lumit_gpu::fx::common::upload_linear_f16(ctx, &halfs, m.width, m.height)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests;
