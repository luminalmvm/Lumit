//! Particulate's GPU passes (docs/08 §3.86; docs/impl/particulate.md §5–§7):
//! evaluate, compact, draw.
//!
//! **In plain terms.** The kernel next door works out where every particle is
//! and packs the live ones together; this file is the host half — it makes the
//! buffers, hands the numbers over, and runs the four passes in order. Nothing
//! here does arithmetic on a parameter: every value arrives already reduced by
//! `lumit_core::fx::effects::particulate`'s own `packed`, so the CPU reference
//! and this path multiply by numbers that came from one expression.
//!
//! **Why it is four passes and not one.** A particle system's output is
//! *variable length* — how many are alive is not known until they have all been
//! asked — and a GPU cannot append to a list without either atomics or a
//! prefix sum. Atomics would make the order of the compacted stream depend on
//! which workgroup arrived first, which would make `id` order a scheduling
//! artefact and two renders of one frame able to disagree. So: count, scan,
//! place. Deterministic by construction (particulate.md §5).

use crate::GpuContext;

use super::{work_texture, FxEngine};

/// The evaluate passes' workgroup width: one thread per candidate, and the
/// same 64 the kernel declares.
const EVAL_WORKGROUP: u32 = 64;

/// `max_compute_workgroups_per_dimension` as **every** device guarantees it.
///
/// The context asks for `wgpu::Limits::default()`, so this is not a number to
/// look up on the adapter — it is the number the device was opened with, and a
/// dispatch past it is a validation error on the best card there is.
const MAX_WORKGROUPS_PER_DIM: u32 = 65_535;

/// The most births one frame's window may record before the oldest are let go.
///
/// The candidate set is Emit rate times the longest life, and both of those are
/// open-ended controls — a rate of a million and a life of an hour is a typeable
/// document, and a buffer sized off it is an allocation with no ceiling
/// (14-ENGINEERING-RULES §6). Dropping candidates *older* than this changes
/// nothing the cap rule would not already have dropped: what survives an
/// overload is the newest `cap`, and there are four times that many newer
/// candidates still in play.
///
/// **Why it is this number and not a round one.** It was 8 000 000, chosen as a
/// memory budget alone — thirty-two megabytes of flags. Memory was never the
/// binding constraint: [`ParticulatePass::evaluate`] dispatches one workgroup
/// per 64 candidates, so 8 000 000 candidates asks for 125 000 workgroups
/// against a limit of 65 535, and the device rejects the dispatch. What the
/// user saw was the whole frame break — the failed compute pass invalidates the
/// encoder, so the draw that follows it cannot begin either, and every later
/// frame at that rate does it again. Typing a big Emit rate is enough to reach
/// it (the row's slider stops at 1 000 but its hard maximum is open), which is
/// exactly how it was reached.
///
/// So the ceiling is the *dispatch* limit, and the memory budget follows from
/// it rather than the other way round: 65 535 × 64 candidates is 4 194 240, or
/// about sixteen megabytes of flags.
pub const MAX_CANDIDATES: u64 = (MAX_WORKGROUPS_PER_DIM as u64) * (EVAL_WORKGROUP as u64);

/// One resolved Particulate. Mirrors `lumit_core::fx::points::PointsParams`,
/// `DrawStyle` and the threaded schedule, flattened to the numbers the kernel
/// reads — every distance in **raster** pixels, as the resolve step left them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParticulateOp<'a> {
    /// The emitter's centre, three axes.
    pub emitter_pos: [f32; 3],
    /// The Line/Ellipse/Rectangle extents.
    pub emitter_wh: [f32; 2],
    /// The Point/Ellipse/Rectangle extent through the layer's plane.
    pub emitter_depth: f32,
    pub emitter_angle_deg: f32,
    pub direction_deg: f32,
    /// Launch elevation out of the layer's plane, degrees.
    pub direction_z_deg: f32,
    pub spread_deg: f32,
    /// The elevation's own cone, degrees.
    pub spread_z_deg: f32,
    pub speed: f32,
    /// `0..=1`.
    pub speed_jitter: f32,
    /// The `EmitterShape` code: 0 Point, 1 Line, 2 Ellipse, 3 Rectangle,
    /// 4 Mask path.
    pub shape: u32,
    pub seed: u32,
    /// The most particles to **draw**: Max particles, halved once per
    /// degradation rung.
    pub cap: u32,
    pub life: f32,
    /// `0..=1`.
    pub life_jitter: f32,
    pub size: f32,
    /// `0..=1`.
    pub size_jitter: f32,
    pub rotation_deg: f32,
    pub rotation_jitter_deg: f32,
    pub spin_deg: f32,
    pub align_to_motion: bool,
    /// Scene-linear RGBA at birth and at death.
    pub colour: [f32; 4],
    pub end_colour: [f32; 4],
    /// The air's own speed on all three axes.
    pub wind: [f32; 3],
    pub gravity: f32,
    pub drag: f32,
    pub turbulence: f32,
    pub turbulence_scale: f32,
    pub turbulence_speed: f32,
    /// The **fixed** central-difference step the turbulence speed is read at,
    /// so one frame key names one picture whatever raster is previewing.
    pub eps: f32,
    /// The comp frame's length in seconds of layer time.
    pub dt: f32,
    /// One entry per recorded frame plus a closing one: `[birth offset,
    /// bit pattern of (t − that frame's start) in seconds, the frame's first
    /// birth index low and high]`. The steady flow's frames come first and
    /// then the bursts'. The subtraction is the host's, in `f64`, so the small
    /// age never has to be recovered from inside a large clock.
    pub frames: &'a [[u32; 4]],
    /// How many births the window records — the candidate count.
    pub candidates: u32,
    /// The two baked over-life curves, size then opacity, 257 entries each.
    pub curves: &'a [f32],
    /// The mask path as `(x, y, arc length, unused)` per vertex; empty
    /// for every other emitter shape, and the documented no-op for Mask path.
    pub path: &'a [[f32; 4]],
    pub path_total: f32,
    /// Streak length in seconds; zero in every other mode.
    pub tail_seconds: f32,
    /// `0..=1`.
    pub feather: f32,
    /// The host Mix, `0..=1`, folded into the particle's own coverage.
    pub mix: f32,
    /// The `RenderMode` code: 0 Disc, 1 Sprite, 2 Streak. **An unset Sprite
    /// arrives as Disc** — the host resolves the fallback so the kernel has one
    /// less branch (particulate.md §2).
    pub mode: u32,
    /// Where the composition's camera puts a particle off the layer's plane:
    /// a row-major 3×4 already scaled to **this raster**, or `None` on
    /// a 2D layer, a comp with no camera, and every project saved before the
    /// axis existed. `None` is not "the identity matrix" — the kernel takes a
    /// different branch and touches the position's bits not at all, which is
    /// what makes the flat picture bit-identical rather than nearly so.
    pub projection: Option<[[f32; 4]; 3]>,
    /// The `Edge` code: 0 None, 1 Kill, 2 Wrap. The frame is the target.
    pub edge: u32,
    /// How far past the frame a particle goes before the edge acts, px.
    pub margin: f32,
    /// The `Drive` codes for size, opacity and colour: 0 Age, 1 Speed,
    /// 2 Random.
    pub drives: [u32; 3],
    /// The speeds, px per second, a Speed drive reads as its two ends.
    pub speed_range: [f32; 2],
    /// The Density layer's picture births follow, in this raster, or `None`
    /// for births spread evenly over the emitter.
    pub density: Option<&'a wgpu::Texture>,
    /// How particles combine where they overlap: an index into
    /// `lumit_core::model::BlendMode::ALL`, 0 being Normal.
    pub blend: u32,
}

/// [`ParticulateOp::blend`]'s Add, the one mode besides Normal the card's own
/// blending draws exactly.
const BLEND_ADD: u32 = 6;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct ParticulateParams {
    pub(super) em_pos: [f32; 2],
    pub(super) em_wh: [f32; 2],
    pub(super) em_angle: f32,
    pub(super) dir: f32,
    pub(super) spread: f32,
    pub(super) speed: f32,

    pub(super) speed_jitter: f32,
    pub(super) shape: u32,
    pub(super) seed: u32,
    pub(super) cap: u32,

    pub(super) life: f32,
    pub(super) life_jitter: f32,
    pub(super) size: f32,
    pub(super) size_jitter: f32,

    pub(super) rotation: f32,
    pub(super) rotation_jitter: f32,
    pub(super) spin: f32,
    pub(super) align: u32,

    pub(super) colour: [f32; 4],
    pub(super) end_colour: [f32; 4],

    pub(super) wind: [f32; 2],
    pub(super) gravity: f32,
    pub(super) drag: f32,

    pub(super) turb: f32,
    pub(super) turb_scale: f32,
    pub(super) turb_speed: f32,
    pub(super) eps: f32,

    pub(super) dt: f32,
    pub(super) _spare: [u32; 2],
    pub(super) frames: u32,

    pub(super) candidates: u32,
    pub(super) path_len: u32,
    pub(super) path_total: f32,
    pub(super) tail: f32,

    pub(super) feather: f32,
    pub(super) mix: f32,
    pub(super) mode: u32,
    pub(super) target_w: f32,

    pub(super) target_h: f32,
    pub(super) sprite_w: f32,
    pub(super) sprite_h: f32,
    pub(super) wind_z: f32,

    pub(super) em_z: f32,
    pub(super) em_depth: f32,
    pub(super) dir_z: f32,
    pub(super) spread_z: f32,

    pub(super) proj0: [f32; 4],
    pub(super) proj1: [f32; 4],
    pub(super) proj2: [f32; 4],

    pub(super) project: u32,
    pub(super) field_mode: u32,
    pub(super) field_invert: u32,
    pub(super) field_threshold: f32,

    /// The size of the "who stood" target, or 0 for an ordinary draw.
    pub(super) probe_w: u32,
    pub(super) probe_h: u32,
    /// How a sprite sits on its point, each as the amount past the plain
    /// square of the point's size, so all nought is that square. Only the
    /// generic points draw writes them.
    pub(super) stamp_unit: [f32; 2],

    pub(super) stamp_anchor: [f32; 2],
    pub(super) stamp_uv: [f32; 2],

    pub(super) stamp_corner: f32,
    pub(super) edge: u32,
    pub(super) margin: f32,
    /// Always nought; see the kernel's `turb_cycle`.
    pub(super) turb_cycle: i32,

    pub(super) size_by: u32,
    pub(super) opacity_by: u32,
    pub(super) colour_by: u32,
    pub(super) speed_min: f32,

    pub(super) speed_max: f32,
    pub(super) density: u32,
    pub(super) blend: u32,
    pub(super) pair_max: u32,

    pub(super) scan_len: u32,
    pub(super) tile_scale: u32,
    pub(super) _pad: [u32; 2],
}

/// One render pass drawing into `view`, the picture kept or cleared.
fn draw_pass<'e>(
    enc: &'e mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    load: wgpu::LoadOp<wgpu::Color>,
) -> wgpu::RenderPass<'e> {
    enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("fx-particulate-draw"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            ops: wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    })
}

/// Bytes a particle takes in the tiles' copy of it: five rows of four.
const PREP_BYTES: u64 = 5 * 16;

/// The tiles' side, px, the same 16 the kernel declares, before
/// [`tile_scale`].
const TILE: u32 = 16;

/// The most (tile, particle) pairs the tiles sort for a cap of `cap`: room
/// for every particle to reach sixteen tiles, and never less than a million,
/// so a thousand sprites each across a thousand tiles still fit. Two million
/// at the most, which is sixteen megabytes each way and a dispatch of 8 192
/// blocks. Past it the oldest particles are left off the layer, the cap rule
/// again.
fn pair_max(cap: u32) -> u32 {
    cap.saturating_mul(16)
        .clamp(1 << 20, 1 << 21)
        .next_multiple_of(SCAN_BLOCK)
}

/// The words at the head of the tiles' `scan` buffer, before its numbers.
const SCAN_HEAD: u32 = 16;

/// How many 16 px steps a tile's side is, so a frame has fewer tiles than
/// the sort's sixteen bits can name.
fn tile_scale(w: u32, h: u32) -> u32 {
    let mut m = 1;
    while m < 64 && w.div_ceil(TILE * m) * h.div_ceil(TILE * m) >= 0xffff {
        m *= 2;
    }
    m
}

/// How many numbers the tiles' `scan` holds: a particle's count each, the
/// sort's digit counts, or two words a tile, whichever is most.
fn scan_len(cap: u32, w: u32, h: u32) -> u32 {
    let side = TILE * tile_scale(w, h);
    let tiles = w.div_ceil(side) * h.div_ceil(side);
    cap.max(pair_max(cap)).max(tiles * 2)
}

/// The scan's block width — the same 256 the kernel declares.
const SCAN_BLOCK: u32 = 256;

/// Words of stream per particle: position 3, speed 3, age, life, size,
/// rotation, colour 2 (half pairs), id 2, and the draw's own tail 3.
pub(super) const STREAM_WORDS: u64 = 19;

impl FxEngine {
    /// Draw one Particulate over a working texture, returning a new texture of
    /// the same size (docs/08 §3.86).
    ///
    /// `sprite` is the Sprite layer's rendered picture, `None` in every
    /// other mode **and** when the row is unset — in which case `op.mode` is
    /// already Disc, because a render mode must always draw something.
    ///
    /// The picture is copied first and the particles drawn over it, so this is
    /// an ordinary "picture in, picture out" pass with an instanced draw in the
    /// middle rather than a compositor of its own.
    pub fn particulate(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        sprite: Option<&wgpu::Texture>,
        op: &ParticulateOp<'_>,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-particulate-out");
        {
            // The input, unchanged, is what a frame with no live particle looks
            // like — so the copy happens first and every early return below is
            // the effect's honest passthrough.
            let mut enc = ctx.encoder("fx-particulate-copy");
            enc.copy_texture_to_texture(
                src.as_image_copy(),
                out.as_image_copy(),
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
        }
        let Some(pass) = self.particulate_prepare(ctx, src, w, h, sprite, op) else {
            return out;
        };
        let view = out.create_view(&Default::default());
        let mut enc = ctx.encoder("fx-particulate");
        pass.evaluate(self, &mut enc);
        // **The cancellation point** (particulate.md §7) is here, between the
        // evaluate and the draw: two passes, one boundary. No token reaches an
        // effect kernel today — the export's flag is checked once a frame
        // (`export::run`) — so the seam is named rather than faked, and a
        // cancel lands on the frame this pass belongs to.
        if op.blend == 0 {
            let mut rp = draw_pass(&mut enc, &view, wgpu::LoadOp::Load);
            rp.set_pipeline(&self.particulate_draw);
            rp.set_bind_group(0, &pass.empty, &[]);
            rp.set_bind_group(1, &pass.draw_bind, &[]);
            rp.draw_indirect(&pass.args, 0);
            drop(rp);
            drop(enc);
            return out;
        }
        // Any other mode: the particles meet each other on a clear layer of
        // their own, which then goes over the picture as one.
        let blend = &self.particulate_blend;
        let add = op.blend == BLEND_ADD;
        let layer = if add {
            let layer = work_texture(ctx, w, h, "fx-particulate-layer");
            let layer_view = layer.create_view(&Default::default());
            let mut rp = draw_pass(
                &mut enc,
                &layer_view,
                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            );
            rp.set_pipeline(&blend.draw_add);
            rp.set_bind_group(0, &pass.empty, &[]);
            rp.set_bind_group(1, &pass.draw_bind, &[]);
            rp.draw_indirect(&pass.args, 0);
            drop(rp);
            layer
        } else {
            self.particulate_tiles(ctx, &mut enc, &pass, w, h, op.cap)
        };
        let finish_bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-particulate-finish-bind"),
            layout: &blend.finish_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(
                    &layer.create_view(&Default::default()),
                ),
            }],
        });
        let mut rp = draw_pass(&mut enc, &view, wgpu::LoadOp::Load);
        rp.set_pipeline(&blend.finish);
        rp.set_bind_group(0, &pass.empty, &[]);
        rp.set_bind_group(1, &pass.empty, &[]);
        rp.set_bind_group(2, &finish_bind, &[]);
        rp.draw(0..3, 0..1);
        drop(rp);
        drop(enc);
        if add {
            ctx.recycle(layer);
        }
        out
    }

    /// Every mode but Normal and Add reads the colour already there, which
    /// the card's blending cannot, so the tiles work it out (the kernel's
    /// Particle blend section), into a layer of the particles' own.
    fn particulate_tiles(
        &self,
        ctx: &GpuContext,
        enc: &mut wgpu::CommandEncoder,
        pass: &ParticulatePass,
        w: u32,
        h: u32,
        cap: u32,
    ) -> wgpu::Texture {
        let blend = &self.particulate_blend;
        // ponytail: the layer and the sort's scratch are made fresh each
        // frame, and a new buffer reads as nought, which the counts at the
        // head of `scan` rely on. Pool them, cleared, if a profile shows the
        // allocation.
        let tiles = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("fx-particulate-tiles"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let storage = |label: &str, bytes: u64| {
            ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes.max(4),
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        };
        let side = TILE * tile_scale(w, h);
        let (tx, ty) = (w.div_ceil(side), h.div_ceil(side));
        let most = pair_max(cap);
        let len = scan_len(cap, w, h);
        let prep = storage("fx-particulate-prep", u64::from(cap) * PREP_BYTES);
        let pairs_a = storage("fx-particulate-pairs-a", u64::from(most) * 8);
        let pairs_b = storage("fx-particulate-pairs-b", u64::from(most) * 8);
        let scan = storage(
            "fx-particulate-scan",
            (u64::from(SCAN_HEAD) + u64::from(len) + u64::from(len.div_ceil(SCAN_BLOCK))) * 4,
        );
        let tiles_view = tiles.create_view(&Default::default());
        let bind = |src: &wgpu::Buffer, dst: &wgpu::Buffer| {
            ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fx-particulate-tiles-bind"),
                layout: &blend.tiles_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: pass.args.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: prep.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&tiles_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: src.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: dst.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: scan.as_entire_binding(),
                    },
                ],
            })
        };
        // The sort's first pass reads `pairs_a` into `pairs_b` and the second
        // reads it back, so the tiles read `pairs_a` too.
        let (f, b) = (&bind(&pairs_a, &pairs_b), &bind(&pairs_b, &pairs_a));
        let particles = cap.div_ceil(EVAL_WORKGROUP);
        let numbers = len.div_ceil(SCAN_BLOCK);
        let pairs = most.div_ceil(SCAN_BLOCK);
        let t = &blend.tiles;
        let steps = [
            (f, &t.prep, particles, 1),
            (f, &t.scan_block, numbers, 1),
            (f, &t.scan_sums, 1, 1),
            (f, &t.scan_add, numbers, 1),
            (f, &t.emit, particles, 1),
            (f, &t.hist_lo, pairs, 1),
            (f, &t.scan_block, numbers, 1),
            (f, &t.scan_sums, 1, 1),
            (f, &t.scan_add, numbers, 1),
            (f, &t.place_lo, pairs, 1),
            (b, &t.hist_hi, pairs, 1),
            (b, &t.scan_block, numbers, 1),
            (b, &t.scan_sums, 1, 1),
            (b, &t.scan_add, numbers, 1),
            (b, &t.place_hi, pairs, 1),
            (f, &t.ranges_clear, (tx * ty).div_ceil(SCAN_BLOCK), 1),
            (f, &t.ranges, pairs, 1),
            (f, &t.tile, tx, ty),
        ];
        let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("fx-particulate-tiles"),
            timestamp_writes: None,
        });
        cp.set_bind_group(0, &pass.empty, &[]);
        cp.set_bind_group(1, &pass.draw_bind, &[]);
        for (group, pipeline, x, y) in steps {
            cp.set_bind_group(2, group, &[]);
            cp.set_pipeline(pipeline);
            cp.dispatch_workgroups(x, y, 1);
        }
        drop(cp);
        tiles
    }

    /// The compacted stream itself, read back to the host — the **oracle hook**
    /// (docs/08 §1.6), and what PS7's goldens will pin.
    ///
    /// Runs the evaluate and compaction passes and nothing else, then maps the
    /// stream buffer. It has `readback_linear_f32`'s shape and its reason: a
    /// test that can only see pixels can only hold the *picture* to the CPU
    /// reference, and points-stream.md §3.2 asks a stricter question of the
    /// numbers — ≤ 2 ULP, because a consumer reads them as data and data has no
    /// perceptual tolerance.
    ///
    /// Answers the live count and the stream's words; `None` when the device
    /// would not give them back.
    pub fn particulate_stream(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &ParticulateOp<'_>,
    ) -> Option<(u32, Vec<u32>)> {
        let pass = self.particulate_prepare(ctx, src, w, h, None, op)?;
        let stream_bytes = u64::from(op.cap) * STREAM_WORDS * 4;
        let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx-particulate-readback"),
            size: stream_bytes + 16,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        {
            let mut enc = ctx.encoder("fx-particulate-eval");
            pass.evaluate(self, &mut enc);
            enc.copy_buffer_to_buffer(&pass.args, 0, &staging, 0, 16);
            enc.copy_buffer_to_buffer(&pass.stream, 0, &staging, 16, stream_bytes);
        }
        ctx.flush();
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        ctx.device.poll(wgpu::Maintain::Wait);
        rx.recv().ok()?.ok()?;
        let words: Vec<u32> = bytemuck::cast_slice(&slice.get_mapped_range()).to_vec();
        staging.unmap();
        // Word 1 of the indirect arguments is the instance count: how many
        // particles the cap rule left standing.
        let count = *words.get(1)?;
        Some((count, words.get(4..)?.to_vec()))
    }

    /// Everything one Particulate pass needs bound, or `None` when there is
    /// nothing to draw — no candidates, no capacity, or a Mix of nothing, each
    /// of which leaves the input picture exactly as it was.
    fn particulate_prepare(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        sprite: Option<&wgpu::Texture>,
        op: &ParticulateOp<'_>,
    ) -> Option<ParticulatePass> {
        use wgpu::util::DeviceExt;
        // **The dispatch can never be illegal**, whatever a caller hands over
        // (docs/14 §4). Every in-tree caller trims its schedule to
        // [`MAX_CANDIDATES`] before it gets here, so this cannot fire today; it
        // is the crate boundary's own answer to the question the trim answers
        // upstream, because a `lumit-gpu` entry point that faults the device on
        // some input is a fault in `lumit-gpu`. Keeping the oldest candidates
        // draws the wrong particles — the cap rule keeps the *newest* — but a
        // wrong frame is recoverable and an invalidated encoder is not.
        let candidates = op.candidates.min(MAX_CANDIDATES as u32);
        let cap = op.cap;
        if candidates == 0 || cap == 0 || op.mix <= 0.0 || op.frames.len() < 2 {
            return None;
        }
        let blocks = candidates.div_ceil(SCAN_BLOCK);
        let sprite_size = sprite.map_or((1.0, 1.0), |t| (t.width() as f32, t.height() as f32));
        let proj = op.projection.unwrap_or([[0.0; 4]; 3]);
        let u = ParticulateParams {
            em_pos: [op.emitter_pos[0], op.emitter_pos[1]],
            em_wh: op.emitter_wh,
            em_angle: op.emitter_angle_deg,
            dir: op.direction_deg,
            spread: op.spread_deg,
            speed: op.speed,
            speed_jitter: op.speed_jitter,
            shape: op.shape,
            seed: op.seed,
            cap,
            life: op.life,
            life_jitter: op.life_jitter,
            size: op.size,
            size_jitter: op.size_jitter,
            rotation: op.rotation_deg,
            rotation_jitter: op.rotation_jitter_deg,
            spin: op.spin_deg,
            align: u32::from(op.align_to_motion),
            colour: op.colour,
            end_colour: op.end_colour,
            wind: [op.wind[0], op.wind[1]],
            gravity: op.gravity,
            drag: op.drag,
            turb: op.turbulence,
            turb_scale: op.turbulence_scale,
            turb_speed: op.turbulence_speed,
            eps: op.eps,
            dt: op.dt,
            _spare: [0; 2],
            frames: op.frames.len() as u32 - 1,
            candidates,
            path_len: op.path.len() as u32,
            path_total: op.path_total,
            tail: op.tail_seconds,
            feather: op.feather,
            mix: op.mix,
            mode: op.mode,
            target_w: w as f32,
            target_h: h as f32,
            sprite_w: sprite_size.0,
            sprite_h: sprite_size.1,
            wind_z: op.wind[2],
            em_z: op.emitter_pos[2],
            em_depth: op.emitter_depth,
            dir_z: op.direction_z_deg,
            spread_z: op.spread_z_deg,
            proj0: proj[0],
            proj1: proj[1],
            proj2: proj[2],
            project: u32::from(op.projection.is_some()),
            // Particulate's points were decided by the compaction; only the
            // generic draw's two field-testing callers turn the vertex-stage
            // rejection on.
            field_mode: 0,
            field_invert: 0,
            field_threshold: 0.0,
            probe_w: 0,
            probe_h: 0,
            stamp_unit: [0.0; 2],
            stamp_anchor: [0.0; 2],
            stamp_uv: [0.0; 2],
            stamp_corner: 0.0,
            edge: op.edge,
            margin: op.margin,
            turb_cycle: 0,
            size_by: op.drives[0],
            opacity_by: op.drives[1],
            colour_by: op.drives[2],
            speed_min: op.speed_range[0],
            speed_max: op.speed_range[1],
            density: u32::from(op.density.is_some()),
            blend: op.blend,
            pair_max: pair_max(cap),
            scan_len: scan_len(cap, w, h),
            tile_scale: tile_scale(w, h),
            _pad: [0; 2],
        };
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-particulate-params"),
                contents: bytemuck::bytes_of(&u),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let frames_buf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-particulate-frames"),
                contents: bytemuck::cast_slice(op.frames),
                usage: wgpu::BufferUsages::STORAGE,
            });
        // A storage binding cannot be empty, so an absent path is one dead
        // vertex nothing reads — `path_len` under two is the no-op the kernel
        // checks, never the buffer's length.
        let path_data: &[[f32; 4]] = if op.path.is_empty() {
            &[[0.0; 4]]
        } else {
            op.path
        };
        let path_buf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-particulate-path"),
                contents: bytemuck::cast_slice(path_data),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let curves_buf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-particulate-curves"),
                contents: bytemuck::cast_slice(op.curves),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let scratch = |label: &str, words: u64| {
            ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: words.max(1) * 4,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        };
        let ranks = scratch("fx-particulate-ranks", u64::from(candidates));
        let sums = scratch("fx-particulate-blocks", u64::from(blocks) + 1);
        // **The declared peak scratch** (docs/13 §6): the cap *is* the budget,
        // which is why Max particles is a parameter and not a guess.
        let stream = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx-particulate-stream"),
            size: u64::from(cap) * STREAM_WORDS * 4,
            // COPY_SRC so the §1.6 oracle can read the stream back
            // (`particulate_stream`). It costs an allocation flag and buys the
            // only test that can hold these numbers to two ULP.
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let args = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx-particulate-args"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-particulate-bind"),
            layout: &self.particulate_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: ubuf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: frames_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: curves_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: path_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: ranks.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: sums.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: stream.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: args.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(
                        &op.density.unwrap_or(src).create_view(&Default::default()),
                    ),
                },
            ],
        });
        let draw_bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-particulate-draw-bind"),
            layout: &self.particulate_draw_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: ubuf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: stream.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        &sprite.unwrap_or(src).create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(
                        &src.create_view(&Default::default()),
                    ),
                },
            ],
        });
        let empty = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-particulate-empty"),
            layout: &self.particulate_empty_layout,
            entries: &[],
        });
        Some(ParticulatePass {
            bind,
            draw_bind,
            empty,
            stream,
            args,
            candidates,
            blocks,
            // The buffers the bind group points at, held so they outlive it.
            _keep: [ubuf, frames_buf, curves_buf, path_buf, ranks, sums],
        })
    }
}

/// One pass's bound resources, alive until the encoder is done with them.
struct ParticulatePass {
    bind: wgpu::BindGroup,
    draw_bind: wgpu::BindGroup,
    empty: wgpu::BindGroup,
    stream: wgpu::Buffer,
    args: wgpu::Buffer,
    candidates: u32,
    blocks: u32,
    _keep: [wgpu::Buffer; 6],
}

impl ParticulatePass {
    /// Count, scan, place — the three kernels, in the one order that makes a
    /// compacted slot a function of the birth index rather than of which
    /// workgroup arrived first.
    fn evaluate(&self, fx: &FxEngine, enc: &mut wgpu::CommandEncoder) {
        let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("fx-particulate-evaluate"),
            timestamp_writes: None,
        });
        cp.set_bind_group(0, &self.bind, &[]);
        cp.set_pipeline(&fx.particulate_alive);
        cp.dispatch_workgroups(self.candidates.div_ceil(EVAL_WORKGROUP), 1, 1);
        cp.set_pipeline(&fx.particulate_scan);
        cp.dispatch_workgroups(self.blocks, 1, 1);
        cp.set_pipeline(&fx.particulate_blocks);
        cp.dispatch_workgroups(1, 1, 1);
        cp.set_pipeline(&fx.particulate_scatter);
        cp.dispatch_workgroups(self.candidates.div_ceil(EVAL_WORKGROUP), 1, 1);
    }
}

/// The five pipelines and three layouts, built once per device.
pub(super) struct ParticulatePipelines {
    pub layout: wgpu::BindGroupLayout,
    pub draw_layout: wgpu::BindGroupLayout,
    pub empty_layout: wgpu::BindGroupLayout,
    pub alive: wgpu::ComputePipeline,
    pub scan: wgpu::ComputePipeline,
    pub blocks: wgpu::ComputePipeline,
    pub scatter: wgpu::ComputePipeline,
    pub draw: wgpu::RenderPipeline,
    pub blend: ParticulateBlendPipelines,
}

impl ParticulatePipelines {
    pub(super) fn new(ctx: &GpuContext, module: &wgpu::ShaderModule) -> Self {
        let storage = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fx-particulate-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    storage(1, true),
                    storage(2, true),
                    storage(3, true),
                    storage(4, false),
                    storage(5, false),
                    storage(6, false),
                    storage(7, false),
                    wgpu::BindGroupLayoutEntry {
                        binding: 8,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            });
        // The draw's own group. The stream is bound **read-only** here: a
        // vertex stage may not write, and it has nothing to write anyway.
        let draw_layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fx-particulate-draw-layout"),
                entries: &[
                    // The tiles' two compute passes read the first three as
                    // the draw does.
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT
                            .union(wgpu::ShaderStages::COMPUTE),
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX.union(wgpu::ShaderStages::COMPUTE),
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT.union(wgpu::ShaderStages::COMPUTE),
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // The picture the points were thrown at, read in the
                    // **vertex** stage by Scatter's rejection. Every
                    // other caller binds its own input here and never looks.
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            });
        // The draw's bindings live at `@group(1)` because the compute half
        // already owns `@group(0)` in the same module, and one module is what
        // keeps the kernel and its twin in one file.
        let empty_layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fx-particulate-empty-layout"),
                entries: &[],
            });
        let compute_pl = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-particulate-pl"),
                bind_group_layouts: &[&layout],
                push_constant_ranges: &[],
            });
        let draw_pl = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-particulate-draw-pl"),
                bind_group_layouts: &[&empty_layout, &draw_layout],
                push_constant_ranges: &[],
            });
        // Started here and collected below, so the four compile beside the
        // draw pipeline and not one after another.
        let compute = |entry: &'static str, label: &'static str| {
            let (device, layout, module) = (ctx.device.clone(), compute_pl.clone(), module.clone());
            crate::Compiling::start(move || {
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(label),
                    layout: Some(&layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
            })
        };
        let alive = compute("pt_alive", "fx-particulate-alive");
        let scan = compute("pt_scan", "fx-particulate-scan");
        let blocks = compute("pt_blocks", "fx-particulate-blocks");
        let scatter = compute("pt_scatter", "fx-particulate-scatter");
        let draw = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("fx-particulate-draw"),
                layout: Some(&draw_pl),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some("pt_vs"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some("pt_fs"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: ctx.working(),
                        // Premultiplied `over`, the same blend the compositor's
                        // Normal uses: the particle's own colour, and what was
                        // there kept by however much it did not cover.
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                // **Never multisampled.** An fp16 blend into a multisample
                // target is where the physical flare's nondeterminism came
                // from, and a particle field is that same shape at scale.
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });
        let blend = ParticulateBlendPipelines::new(ctx, module, &empty_layout, &draw_layout);
        Self {
            layout,
            draw_layout,
            empty_layout,
            alive: alive.wait(),
            scan: scan.wait(),
            blocks: blocks.wait(),
            scatter: scatter.wait(),
            draw,
            blend,
        }
    }
}

/// Particle blend's own passes: Add's draw, the tiles', and the one that
/// lays either layer over the picture.
pub(super) struct ParticulateBlendPipelines {
    pub tiles_layout: wgpu::BindGroupLayout,
    pub finish_layout: wgpu::BindGroupLayout,
    pub draw_add: wgpu::RenderPipeline,
    pub tiles: TilePipelines,
    pub finish: wgpu::RenderPipeline,
}

/// The tiles' compute passes, in the order a frame runs them.
pub(super) struct TilePipelines {
    pub prep: wgpu::ComputePipeline,
    pub scan_block: wgpu::ComputePipeline,
    pub scan_sums: wgpu::ComputePipeline,
    pub scan_add: wgpu::ComputePipeline,
    pub emit: wgpu::ComputePipeline,
    pub hist_lo: wgpu::ComputePipeline,
    pub hist_hi: wgpu::ComputePipeline,
    pub place_lo: wgpu::ComputePipeline,
    pub place_hi: wgpu::ComputePipeline,
    pub ranges_clear: wgpu::ComputePipeline,
    pub ranges: wgpu::ComputePipeline,
    pub tile: wgpu::ComputePipeline,
}

impl ParticulateBlendPipelines {
    fn new(
        ctx: &GpuContext,
        module: &wgpu::ShaderModule,
        empty_layout: &wgpu::BindGroupLayout,
        draw_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let buffer = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let tiles_layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fx-particulate-tiles-layout"),
                entries: &[
                    buffer(0, true),
                    buffer(1, false),
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: wgpu::TextureFormat::Rgba16Float,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    buffer(3, false),
                    buffer(4, false),
                    buffer(5, false),
                ],
            });
        let finish_layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fx-particulate-finish-layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                }],
            });
        let tiles_pl = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-particulate-tiles-pl"),
                bind_group_layouts: &[empty_layout, draw_layout, &tiles_layout],
                push_constant_ranges: &[],
            });
        let compute = |entry: &'static str| {
            let (device, layout, module) = (ctx.device.clone(), tiles_pl.clone(), module.clone());
            crate::Compiling::start(move || {
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
            })
        };
        let started = [
            "pt_prep",
            "pt_scan_block",
            "pt_scan_sums",
            "pt_scan_add",
            "pt_emit",
            "pt_hist_lo",
            "pt_hist_hi",
            "pt_place_lo",
            "pt_place_hi",
            "pt_ranges_clear",
            "pt_ranges",
            "pt_tile",
        ]
        .map(compute);
        let render = |label: &'static str,
                      layout: &wgpu::PipelineLayout,
                      vs: &'static str,
                      fs: &'static str,
                      blend: wgpu::BlendState| {
            ctx.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(label),
                    layout: Some(layout),
                    vertex: wgpu::VertexState {
                        module,
                        entry_point: Some(vs),
                        buffers: &[],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module,
                        entry_point: Some(fs),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: ctx.working(),
                            blend: Some(blend),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: Default::default(),
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    // Never multisampled, for the draw's own reason.
                    multisample: wgpu::MultisampleState::default(),
                    multiview: None,
                    cache: None,
                })
        };
        let draw_pl = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-particulate-add-pl"),
                bind_group_layouts: &[empty_layout, draw_layout],
                push_constant_ranges: &[],
            });
        // Add: the colours sum, and the coverage is the union, as the W3C
        // formula gives for it, so no particle reads what is already there.
        let draw_add = render(
            "fx-particulate-add",
            &draw_pl,
            "pt_vs",
            "pt_fs",
            wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
            },
        );
        let finish_pl = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-particulate-finish-pl"),
                bind_group_layouts: &[empty_layout, empty_layout, &finish_layout],
                push_constant_ranges: &[],
            });
        let finish = render(
            "fx-particulate-finish",
            &finish_pl,
            "pt_full_vs",
            "pt_finish",
            wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
        );
        let [prep, scan_block, scan_sums, scan_add, emit, hist_lo, hist_hi, place_lo, place_hi, ranges_clear, ranges, tile] =
            started.map(|c| c.wait());
        Self {
            tiles_layout,
            finish_layout,
            draw_add,
            tiles: TilePipelines {
                prep,
                scan_block,
                scan_sums,
                scan_add,
                emit,
                hist_lo,
                hist_hi,
                place_lo,
                place_hi,
                ranges_clear,
                ranges,
                tile,
            },
            finish,
        }
    }
}
