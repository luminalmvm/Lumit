//! Walking skeleton for the pixel pass (docs/05-ARCHITECTURE.md §1.1,
//! docs/06-RENDER-PIPELINE.md §1.1): `lumit-eval`'s demand-pull executor
//! driving `lumit-gpu`'s real compositor through the `FrameSource` /
//! `KernelExecutor` / `CacheStore` trait seams, with pixels read back and
//! checked. This is the first end-to-end proof that graph-driven rendering
//! produces the same linear-light results the compositor produces directly —
//! the seam the full preview/export migration will widen.
//!
//! In plain terms: the executor walks the wiring diagram and asks these
//! little adapters to do each step; the adapters call the same GPU kernels
//! the shipped renderer uses. If the colours that come back are right, the
//! sockets fit.
//!
//! Scope: Source (solids), Transform (identity passthrough — placement
//! resolution stays with the adapter's owner), Composite (Normal blend,
//! per-layer opacity), CompOutput (over the comp background). Retime, masks
//! and adjustments are later slices. Skips cleanly when no GPU adapter is
//! present, like every other lumit-gpu test.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use lumit_eval::epoch::{Epoch, EpochToken};
use lumit_eval::exec::{
    render_frame, CacheStore, ExecError, FrameHandle, FrameSource, KernelExecutor,
};
use lumit_eval::graph::{EvalGraph, Node, NodeId, NodeKind, SourceRef};
use lumit_eval::FrameKey;
use lumit_gpu::composite::{Blend, CompositeLayer, Compositor};
use lumit_gpu::{ColourEngine, GpuContext};
use uuid::Uuid;

/// The frame store handles index into. Shared by the two adapters via
/// `Rc<RefCell<…>>` — single-threaded test plumbing, nothing more.
struct Frames {
    textures: Vec<wgpu::Texture>,
}

impl Frames {
    fn push(&mut self, tex: wgpu::Texture) -> FrameHandle {
        self.textures.push(tex);
        FrameHandle(self.textures.len() as u64 - 1)
    }

    fn get(&self, h: FrameHandle) -> Option<&wgpu::Texture> {
        self.textures.get(usize::try_from(h.0).ok()?)
    }
}

/// Solid sources: uuid → sRGB byte colour, uploaded and linearised at comp
/// size exactly as the shipped renderer prepares sources.
struct SolidSource {
    ctx: Rc<GpuContext>,
    colour: Rc<ColourEngine>,
    frames: Rc<RefCell<Frames>>,
    solids: HashMap<Uuid, [u8; 4]>,
    size: (u32, u32),
    fetches: usize,
}

impl FrameSource for SolidSource {
    fn source_frame(
        &mut self,
        source: &SourceRef,
        _t: f64,
        _token: &EpochToken,
    ) -> Result<FrameHandle, ExecError> {
        let SourceRef::Solid(id) = source else {
            return Err(ExecError::Node {
                node: 0,
                message: format!("skeleton handles solids only, got {source:?}"),
            });
        };
        let rgba = self.solids.get(id).ok_or(ExecError::Node {
            node: 0,
            message: "unknown solid".into(),
        })?;
        self.fetches += 1;
        let (w, h) = self.size;
        let px: Vec<u8> = std::iter::repeat_n(*rgba, (w * h) as usize)
            .flatten()
            .collect();
        let srgb = self.colour.upload_srgb8(&self.ctx, &px, w, h);
        let linear = self.colour.linearise(&self.ctx, &srgb);
        Ok(self.frames.borrow_mut().push(linear))
    }
}

/// Non-source nodes over the real compositor. Layer semantics (opacity here;
/// the full transform later) are resolved by the adapter from its own layer
/// table — the executor stays semantics-blind, as designed.
struct GpuKernels {
    ctx: Rc<GpuContext>,
    compositor: Compositor,
    frames: Rc<RefCell<Frames>>,
    size: (u32, u32),
    background: [f64; 4],
    /// layer id → opacity percent (the slice of the snapshot this adapter
    /// resolves; defaults to 100).
    opacity: HashMap<Uuid, f32>,
}

impl GpuKernels {
    /// A comp-sized frame drawn 1:1 (identity placement) at `opacity`.
    fn full_frame<'a>(&self, tex: &'a wgpu::Texture, opacity: f32) -> CompositeLayer<'a> {
        CompositeLayer {
            texture: tex,
            size: (self.size.0 as f32, self.size.1 as f32),
            position: (0.0, 0.0),
            anchor: (0.0, 0.0),
            scale: (100.0, 100.0),
            rotation_deg: 0.0,
            opacity,
            matte: None,
            blend: Blend::Normal,
            z: 0.0,
            rotation_x_deg: 0.0,
            rotation_y_deg: 0.0,
            three_d: false,
            layer_mask: None,
            pre: None,
        }
    }
}

impl KernelExecutor for GpuKernels {
    fn run(
        &mut self,
        node: NodeId,
        kind: &NodeKind,
        inputs: &[FrameHandle],
        _t: f64,
        _token: &EpochToken,
    ) -> Result<FrameHandle, ExecError> {
        let err = |message: String| ExecError::Node { node, message };
        let (w, h) = self.size;
        match kind {
            // Identity placement in this slice: the frame passes through.
            NodeKind::Transform { .. } => inputs
                .first()
                .copied()
                .ok_or_else(|| err("transform needs an input".into())),
            NodeKind::Composite { layer, blend, .. } => {
                if *blend != lumit_core::model::BlendMode::Normal {
                    return Err(err(format!("skeleton blends Normal only, got {blend:?}")));
                }
                let opacity = self.opacity.get(layer).copied().unwrap_or(100.0);
                let frames = self.frames.borrow();
                let top = inputs
                    .first()
                    .and_then(|&h| frames.get(h))
                    .ok_or_else(|| err("composite needs its layer input".into()))?;
                // [top] over transparency, or [top, below] over the seed —
                // exactly the accumulator semantics compile() encodes.
                let seed = match inputs.get(1) {
                    Some(&below) => Some(
                        frames
                            .get(below)
                            .ok_or_else(|| err("missing seed".into()))?,
                    ),
                    None => None,
                };
                let out = self.compositor.composite_seeded(
                    &self.ctx,
                    w,
                    h,
                    [0.0, 0.0, 0.0, 0.0],
                    &[self.full_frame(top, opacity)],
                    None,
                    seed,
                );
                drop(frames);
                Ok(self.frames.borrow_mut().push(out))
            }
            NodeKind::CompOutput { .. } => {
                let frames = self.frames.borrow();
                let out = match inputs.first() {
                    Some(&acc) => {
                        let acc = frames.get(acc).ok_or_else(|| err("missing input".into()))?;
                        self.compositor.composite(
                            &self.ctx,
                            w,
                            h,
                            self.background,
                            &[self.full_frame(acc, 100.0)],
                        )
                    }
                    None => self
                        .compositor
                        .composite(&self.ctx, w, h, self.background, &[]),
                };
                drop(frames);
                Ok(self.frames.borrow_mut().push(out))
            }
            other => Err(err(format!("skeleton does not run {other:?} yet"))),
        }
    }
}

/// A real (if tiny) cache: hits return the stored handle, so a repeated
/// render does zero GPU work.
#[derive(Default)]
struct MapCache {
    entries: HashMap<u128, FrameHandle>,
}

impl CacheStore for MapCache {
    fn get(&mut self, key: FrameKey) -> Option<FrameHandle> {
        self.entries.get(&key.0).copied()
    }

    fn put(&mut self, key: FrameKey, frame: FrameHandle) {
        self.entries.insert(key.0, frame);
    }
}

fn srgb_encode(linear: f64) -> f64 {
    if linear <= 0.003_130_8 {
        12.92 * linear
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    }
}

fn srgb_decode(encoded: f64) -> f64 {
    if encoded <= 0.040_45 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    }
}

struct Rig {
    ctx: Rc<GpuContext>,
    colour: Rc<ColourEngine>,
    frames: Rc<RefCell<Frames>>,
    size: (u32, u32),
}

impl Rig {
    fn new(size: (u32, u32)) -> Option<Self> {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("skipping: no GPU adapter");
            return None;
        };
        let ctx = Rc::new(ctx);
        let colour = Rc::new(ColourEngine::new(&ctx));
        Some(Self {
            ctx,
            colour,
            frames: Rc::new(RefCell::new(Frames {
                textures: Vec::new(),
            })),
            size,
        })
    }

    fn source(&self, solids: HashMap<Uuid, [u8; 4]>) -> SolidSource {
        SolidSource {
            ctx: Rc::clone(&self.ctx),
            colour: Rc::clone(&self.colour),
            frames: Rc::clone(&self.frames),
            solids,
            size: self.size,
            fetches: 0,
        }
    }

    fn kernels(&self, background: [f64; 4], opacity: HashMap<Uuid, f32>) -> GpuKernels {
        GpuKernels {
            ctx: Rc::clone(&self.ctx),
            compositor: Compositor::new(&self.ctx),
            frames: Rc::clone(&self.frames),
            size: self.size,
            background,
            opacity,
        }
    }

    /// sRGB bytes of one produced frame (via the display transform, exactly
    /// as the Viewer and export read frames out).
    fn readback(&self, h: FrameHandle) -> Vec<u8> {
        let frames = self.frames.borrow();
        let tex = frames.get(h).expect("handle is valid");
        let shown = self.colour.display(&self.ctx, tex);
        self.colour.readback8(&self.ctx, &shown).expect("readback")
    }
}

/// source → transform → composite → output for one full-frame solid layer.
fn one_layer_graph(solid: Uuid, layer: Uuid, comp: Uuid, w: u32, h: u32) -> EvalGraph {
    let node = |kind, inputs| Node { kind, inputs };
    EvalGraph {
        nodes: vec![
            node(
                NodeKind::Source {
                    source: SourceRef::Solid(solid),
                },
                vec![],
            ),
            node(NodeKind::Transform { layer }, vec![0]),
            node(
                NodeKind::Composite {
                    layer,
                    blend: lumit_core::model::BlendMode::Normal,
                    has_matte: false,
                },
                vec![1],
            ),
            node(
                NodeKind::CompOutput {
                    comp,
                    width: w,
                    height: h,
                },
                vec![2],
            ),
        ],
        output: 3,
    }
}

#[test]
fn executor_drives_the_compositor_to_the_right_pixels() {
    let Some(rig) = Rig::new((8, 8)) else { return };
    let (solid, layer, comp) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    let mut source = rig.source(HashMap::from([(solid, [255, 0, 0, 255])]));
    let mut kernels = rig.kernels([0.0, 0.0, 0.0, 1.0], HashMap::new());
    let mut cache = MapCache::default();
    let graph = one_layer_graph(solid, layer, comp, 8, 8);
    let token = Epoch::new().token();

    let out = render_frame(
        &graph,
        0.0,
        None,
        &mut source,
        &mut kernels,
        &mut cache,
        &token,
    )
    .expect("skeleton renders");
    // An opaque full-frame red solid over an opaque black background: pure
    // red at every pixel, bit-exact through upload → linearise → composite →
    // display, the same round-trip the direct compositor tests pin.
    let px = rig.readback(out);
    assert!(
        px.chunks_exact(4).all(|p| p == [255, 0, 0, 255]),
        "expected pure red, got first pixel {:?}",
        &px[..4]
    );
}

#[test]
fn two_layers_blend_in_linear_light_through_the_seams() {
    let Some(rig) = Rig::new((8, 8)) else { return };
    let (red, green) = (Uuid::now_v7(), Uuid::now_v7());
    let (l_top, l_bottom, comp) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    let mut source = rig.source(HashMap::from([
        (red, [255, 0, 0, 255]),
        (green, [0, 255, 0, 255]),
    ]));
    // Top layer red at 50% opacity over the opaque green bottom layer.
    let mut kernels = rig.kernels([0.0, 0.0, 0.0, 1.0], HashMap::from([(l_top, 50.0)]));
    let mut cache = MapCache::default();
    let node = |kind, inputs| Node { kind, inputs };
    let graph = EvalGraph {
        nodes: vec![
            node(
                NodeKind::Source {
                    source: SourceRef::Solid(green),
                },
                vec![],
            ),
            node(NodeKind::Transform { layer: l_bottom }, vec![0]),
            node(
                NodeKind::Composite {
                    layer: l_bottom,
                    blend: lumit_core::model::BlendMode::Normal,
                    has_matte: false,
                },
                vec![1],
            ),
            node(
                NodeKind::Source {
                    source: SourceRef::Solid(red),
                },
                vec![],
            ),
            node(NodeKind::Transform { layer: l_top }, vec![3]),
            node(
                NodeKind::Composite {
                    layer: l_top,
                    blend: lumit_core::model::BlendMode::Normal,
                    has_matte: false,
                },
                vec![4, 2],
            ),
            node(
                NodeKind::CompOutput {
                    comp,
                    width: 8,
                    height: 8,
                },
                vec![5],
            ),
        ],
        output: 6,
    };
    let token = Epoch::new().token();
    let out = render_frame(
        &graph,
        0.0,
        None,
        &mut source,
        &mut kernels,
        &mut cache,
        &token,
    )
    .expect("skeleton renders");

    // Half red over full green must mix in LINEAR light: each channel is
    // 0.5 in linear, ≈ 188 once sRGB-encoded — the physically-correct value
    // the direct compositor test also pins (naive byte mixing would say 128).
    let expected = (srgb_encode(0.5 * srgb_decode(1.0)) * 255.0).round() as i32;
    let px = rig.readback(out);
    let (r, g, b, a) = (i32::from(px[0]), i32::from(px[1]), i32::from(px[2]), px[3]);
    assert!(
        (r - expected).abs() <= 1 && (g - expected).abs() <= 1 && b == 0 && a == 255,
        "expected ≈({expected},{expected},0,255), got ({r},{g},{b},{a})"
    );
}

#[test]
fn a_cached_frame_re_renders_with_zero_gpu_work() {
    let Some(rig) = Rig::new((8, 8)) else { return };
    let (solid, layer, comp) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    let mut source = rig.source(HashMap::from([(solid, [0, 0, 255, 255])]));
    let mut kernels = rig.kernels([0.0, 0.0, 0.0, 1.0], HashMap::new());
    let mut cache = MapCache::default();
    let graph = one_layer_graph(solid, layer, comp, 8, 8);
    let token = Epoch::new().token();
    let key = Some(FrameKey(7));

    let first = render_frame(
        &graph,
        0.0,
        key,
        &mut source,
        &mut kernels,
        &mut cache,
        &token,
    )
    .expect("first render");
    let textures_after_first = rig.frames.borrow().textures.len();
    let fetches_after_first = source.fetches;

    let second = render_frame(
        &graph,
        0.0,
        key,
        &mut source,
        &mut kernels,
        &mut cache,
        &token,
    )
    .expect("second render");
    assert_eq!(second, first, "the cache returns the same frame");
    assert_eq!(
        rig.frames.borrow().textures.len(),
        textures_after_first,
        "no new GPU textures on a cache hit"
    );
    assert_eq!(source.fetches, fetches_after_first, "no re-decode on a hit");
    // And the cached frame still reads back correctly.
    let px = rig.readback(second);
    assert!(
        px.chunks_exact(4).all(|p| p == [0, 0, 255, 255]),
        "expected pure blue, got first pixel {:?}",
        &px[..4]
    );
}
