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
//! resolution stays with the adapter's owner), Composite (all blend modes via
//! the adapter's `BlendMode` → `Blend` map, per-layer opacity, and layer-mask
//! coverage), Masks (coverage applied at Composite), Adjust (an adjustment
//! layer's effect stack over the composite below), CompOutput (over the comp
//! background). Retime is the remaining slice. Skips cleanly when no GPU
//! adapter is present, like every other lumit-gpu test.

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
use lumit_gpu::fx::{FxEngine, InvertOp};
use lumit_gpu::{ColourEngine, GpuContext};
use uuid::Uuid;

/// One resolved effect an adjustment layer applies to the composite below it.
/// The production adapter will resolve a layer's full effect stack from the
/// document; the skeleton pins the one it needs (Invert is parameter-light and
/// its result is trivial to predict, so it makes a clean seam proof).
#[derive(Clone, Copy)]
enum AdjustFx {
    Invert { mix: f32 },
}

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

/// Solid sources: uuid → (sRGB byte colour, pixel size), uploaded and
/// linearised exactly as the shipped renderer prepares sources.
struct SolidSource {
    ctx: Rc<GpuContext>,
    colour: Rc<ColourEngine>,
    frames: Rc<RefCell<Frames>>,
    solids: HashMap<Uuid, ([u8; 4], (u32, u32))>,
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
        let (rgba, (w, h)) = self.solids.get(id).ok_or(ExecError::Node {
            node: 0,
            message: "unknown solid".into(),
        })?;
        self.fetches += 1;
        let px: Vec<u8> = std::iter::repeat_n(*rgba, (w * h) as usize)
            .flatten()
            .collect();
        let srgb = self.colour.upload_srgb8(&self.ctx, &px, *w, *h);
        let linear = self.colour.linearise(&self.ctx, &srgb);
        Ok(self.frames.borrow_mut().push(linear))
    }
}

/// One layer's resolved placement — the values the real adapter will read
/// out of the document snapshot at time `t` (Property::value_at); the test
/// pins them in a table instead.
#[derive(Clone, Copy)]
struct Placement {
    /// Layer-pixel size the transform applies to (the source's size).
    size: (f32, f32),
    position: (f32, f32),
    anchor: (f32, f32),
    scale: (f32, f32),
    rotation_deg: f32,
    opacity: f32,
}

impl Placement {
    /// Identity: a `(w, h)` frame drawn 1:1 at full opacity.
    fn full_frame(w: f32, h: f32) -> Self {
        Self {
            size: (w, h),
            position: (0.0, 0.0),
            anchor: (0.0, 0.0),
            scale: (100.0, 100.0),
            rotation_deg: 0.0,
            opacity: 100.0,
        }
    }
}

/// Non-source nodes over the real compositor. Layer semantics (placement,
/// opacity; masks/retimes/effects later) are resolved by the adapter from
/// its own layer table — the executor stays semantics-blind, as designed.
struct GpuKernels {
    ctx: Rc<GpuContext>,
    compositor: Compositor,
    frames: Rc<RefCell<Frames>>,
    size: (u32, u32),
    background: [f64; 4],
    /// layer id → resolved placement (defaults to full-frame identity).
    layers: HashMap<Uuid, Placement>,
    /// layer id → its rasterised mask coverage (alpha), applied at Composite
    /// via `CompositeLayer::layer_mask`. The production adapter rasterises the
    /// layer's mask shapes here; the test uploads a ready alpha texture. Absent
    /// = no mask (fully visible).
    masks: HashMap<Uuid, wgpu::Texture>,
    /// The effect engine an adjustment layer runs its stack through.
    fx: FxEngine,
    /// adjustment-layer id → the effect stack it applies to the composite
    /// below (empty / absent = a no-op adjustment).
    adjusts: HashMap<Uuid, Vec<AdjustFx>>,
}

impl GpuKernels {
    fn placed<'a>(&self, tex: &'a wgpu::Texture, p: Placement) -> CompositeLayer<'a> {
        CompositeLayer {
            texture: tex,
            size: p.size,
            position: p.position,
            anchor: p.anchor,
            scale: p.scale,
            rotation_deg: p.rotation_deg,
            opacity: p.opacity,
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

    /// A comp-sized frame drawn 1:1 (accumulator hand-off).
    fn full_frame<'a>(&self, tex: &'a wgpu::Texture, opacity: f32) -> CompositeLayer<'a> {
        let mut p = Placement::full_frame(self.size.0 as f32, self.size.1 as f32);
        p.opacity = opacity;
        self.placed(tex, p)
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
            // Masks pass the frame through here; the coverage is applied at
            // Composite via `layer_mask`, exactly as the shipped renderer does
            // (the shader multiplies the layer's alpha by the mask's alpha).
            NodeKind::Masks { .. } => inputs
                .first()
                .copied()
                .ok_or_else(|| err("masks needs an input".into())),
            NodeKind::Composite { layer, blend, .. } => {
                let placement = self
                    .layers
                    .get(layer)
                    .copied()
                    .unwrap_or(Placement::full_frame(w as f32, h as f32));
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
                // The layer's blend mode carries through the seam via the
                // adapter's BlendMode → Blend map (the production adapter will
                // resolve this the same way the shipped renderer's draw builder
                // does). Snapshot-computed modes (Screen, Overlay…) read the
                // layer below as their destination, which the accumulator
                // hand-off already supplies as the seed.
                let mut top_layer = self.placed(top, placement);
                top_layer.blend = blend_of(*blend);
                top_layer.layer_mask = self.masks.get(layer);
                let out = self.compositor.composite_seeded(
                    &self.ctx,
                    w,
                    h,
                    [0.0, 0.0, 0.0, 0.0],
                    &[top_layer],
                    None,
                    seed,
                );
                drop(frames);
                Ok(self.frames.borrow_mut().push(out))
            }
            // An adjustment layer runs its effect stack over the composite
            // below it (its single input), returning the adjusted accumulator.
            // The executor stays semantics-blind: which effects, and their
            // parameters, are the adapter's to resolve (here from a table; the
            // production adapter reads the layer's effect list at time `t`).
            NodeKind::Adjust { layer } => {
                let &below = inputs
                    .first()
                    .ok_or_else(|| err("adjust needs the composite below".into()))?;
                let ops = self.adjusts.get(layer).cloned().unwrap_or_default();
                // Start from the input texture; each op returns a fresh one.
                let mut current = {
                    let frames = self.frames.borrow();
                    let src = frames
                        .get(below)
                        .ok_or_else(|| err("missing adjust input".into()))?;
                    // No ops: an identity copy so the pass still yields a frame
                    // (a bare Invert with mix 0 is the identity kernel).
                    self.fx.invert(&self.ctx, src, w, h, &InvertOp { mix: 0.0 })
                };
                for op in &ops {
                    current = match op {
                        AdjustFx::Invert { mix } => {
                            self.fx
                                .invert(&self.ctx, &current, w, h, &InvertOp { mix: *mix })
                        }
                    };
                }
                Ok(self.frames.borrow_mut().push(current))
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

/// The document's `BlendMode` → the compositor's `Blend`. The two enums carry
/// the same variants by design (K-162), so this is a plain 1:1 map. It mirrors
/// the shipped renderer's `blend_of` (lumit-ui) — the production pixel-pass
/// adapter will own the identical map, since the model↔GPU bridge is an
/// app-layer concern (lumit-gpu itself stays model-agnostic).
fn blend_of(m: lumit_core::model::BlendMode) -> Blend {
    use lumit_core::model::BlendMode as M;
    match m {
        M::Normal => Blend::Normal,
        M::Add => Blend::Add,
        M::Multiply => Blend::Multiply,
        M::Screen => Blend::Screen,
        M::Overlay => Blend::Overlay,
        M::SoftLight => Blend::SoftLight,
        M::HardLight => Blend::HardLight,
        M::Lighten => Blend::Lighten,
        M::Darken => Blend::Darken,
        M::Subtract => Blend::Subtract,
        M::ColourBurn => Blend::ColourBurn,
        M::LinearBurn => Blend::LinearBurn,
        M::DarkerColour => Blend::DarkerColour,
        M::ColourDodge => Blend::ColourDodge,
        M::LighterColour => Blend::LighterColour,
        M::LinearLight => Blend::LinearLight,
        M::VividLight => Blend::VividLight,
        M::PinLight => Blend::PinLight,
        M::HardMix => Blend::HardMix,
        M::Difference => Blend::Difference,
        M::Exclusion => Blend::Exclusion,
        M::Divide => Blend::Divide,
        M::Hue => Blend::Hue,
        M::Saturation => Blend::Saturation,
        M::Colour => Blend::Colour,
        M::Luminosity => Blend::Luminosity,
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

    /// Solids at comp size (the common case; sized solids pass a map directly).
    fn source(&self, solids: HashMap<Uuid, [u8; 4]>) -> SolidSource {
        let sized = solids
            .into_iter()
            .map(|(id, rgba)| (id, (rgba, self.size)))
            .collect();
        self.source_sized(sized)
    }

    fn source_sized(&self, solids: HashMap<Uuid, ([u8; 4], (u32, u32))>) -> SolidSource {
        SolidSource {
            ctx: Rc::clone(&self.ctx),
            colour: Rc::clone(&self.colour),
            frames: Rc::clone(&self.frames),
            solids,
            fetches: 0,
        }
    }

    fn kernels(&self, background: [f64; 4], layers: HashMap<Uuid, Placement>) -> GpuKernels {
        GpuKernels {
            ctx: Rc::clone(&self.ctx),
            compositor: Compositor::new(&self.ctx),
            frames: Rc::clone(&self.frames),
            size: self.size,
            background,
            layers,
            masks: HashMap::new(),
            fx: FxEngine::new(&self.ctx),
            adjusts: HashMap::new(),
        }
    }

    /// Upload an `w×h` RGBA mask; only its alpha channel is read as coverage
    /// (the shader samples `layer_mask.a`). Aligning the mask to comp size
    /// keeps the coverage crisp — texel centres land on comp-pixel centres.
    fn mask(&self, rgba: &[u8], w: u32, h: u32) -> wgpu::Texture {
        self.colour.upload_srgb8(&self.ctx, rgba, w, h)
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
    let mut half_red = Placement::full_frame(8.0, 8.0);
    half_red.opacity = 50.0;
    let mut kernels = rig.kernels([0.0, 0.0, 0.0, 1.0], HashMap::from([(l_top, half_red)]));
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

/// A non-Normal blend mode carries through the seams: the layer's `Add` mode
/// reaches the compositor via the one canonical `BlendMode` → `Blend` map, and
/// the accumulator hand-off feeds it the layer below as its destination. Two
/// opaque solids on separate channels (red on top, green below) add to a frame
/// that carries *both* channels — the give-away that the top blended with the
/// bottom rather than replacing it (Normal would drop the green).
/// A layer mask gates the layer through the seams: a full-frame red solid
/// under a mask that is opaque on the left half and transparent on the right
/// shows red on the left and the background on the right. The `Masks` node
/// passes the frame through; the coverage lands at Composite via `layer_mask`,
/// the same route the shipped renderer uses. Proves the mask vocabulary widens
/// the skeleton without touching the executor (which stays semantics-blind).
#[test]
fn a_layer_mask_gates_the_layer_through_the_seams() {
    let Some(rig) = Rig::new((8, 8)) else { return };
    let (solid, layer, comp) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    let mut source = rig.source(HashMap::from([(solid, [255, 0, 0, 255])]));
    // 8×8 mask aligned to the comp: left four columns opaque (alpha 255),
    // right four transparent (alpha 0). RGB is ignored — only .a is read.
    let mut mask_px = Vec::with_capacity(8 * 8 * 4);
    for _y in 0..8 {
        for x in 0..8 {
            let a = if x < 4 { 255 } else { 0 };
            mask_px.extend_from_slice(&[255, 255, 255, a]);
        }
    }
    let mask_tex = rig.mask(&mask_px, 8, 8);
    let mut kernels = rig.kernels([0.0, 0.0, 0.0, 1.0], HashMap::new());
    kernels.masks.insert(layer, mask_tex);
    let mut cache = MapCache::default();
    // source → masks → transform → composite → output.
    let node = |kind, inputs| Node { kind, inputs };
    let graph = EvalGraph {
        nodes: vec![
            node(
                NodeKind::Source {
                    source: SourceRef::Solid(solid),
                },
                vec![],
            ),
            node(NodeKind::Masks { count: 1 }, vec![0]),
            node(NodeKind::Transform { layer }, vec![1]),
            node(
                NodeKind::Composite {
                    layer,
                    blend: lumit_core::model::BlendMode::Normal,
                    has_matte: false,
                },
                vec![2],
            ),
            node(
                NodeKind::CompOutput {
                    comp,
                    width: 8,
                    height: 8,
                },
                vec![3],
            ),
        ],
        output: 4,
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
    let px = rig.readback(out);
    for y in 0..8usize {
        for x in 0..8usize {
            let p = &px[(y * 8 + x) * 4..(y * 8 + x) * 4 + 4];
            let expected: [u8; 4] = if x < 4 {
                [255, 0, 0, 255] // masked-in: the red solid
            } else {
                [0, 0, 0, 255] // masked-out: the background shows
            };
            assert_eq!(p, expected, "pixel ({x},{y})");
        }
    }
}

/// An adjustment layer runs its effect stack over the composite below it,
/// through the seams: a red solid with an Invert adjustment above it reads back
/// as cyan. Invert works in linear light — red (1,0,0) → (0,1,1) → cyan once
/// displayed — so this also pins that the effect ran in the right domain, on
/// the accumulator the executor handed it, with the executor semantics-blind
/// (the effect choice lives in the adapter's table).
#[test]
fn an_adjustment_layer_runs_its_effect_over_the_composite_below() {
    let Some(rig) = Rig::new((8, 8)) else { return };
    let (red, l_red, adj, comp) = (
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
    );
    let mut source = rig.source(HashMap::from([(red, [255, 0, 0, 255])]));
    let mut kernels = rig.kernels([0.0, 0.0, 0.0, 1.0], HashMap::new());
    kernels
        .adjusts
        .insert(adj, vec![AdjustFx::Invert { mix: 1.0 }]);
    let mut cache = MapCache::default();
    // source(red) → transform → composite → adjust(invert) → output.
    let node = |kind, inputs| Node { kind, inputs };
    let graph = EvalGraph {
        nodes: vec![
            node(
                NodeKind::Source {
                    source: SourceRef::Solid(red),
                },
                vec![],
            ),
            node(NodeKind::Transform { layer: l_red }, vec![0]),
            node(
                NodeKind::Composite {
                    layer: l_red,
                    blend: lumit_core::model::BlendMode::Normal,
                    has_matte: false,
                },
                vec![1],
            ),
            node(NodeKind::Adjust { layer: adj }, vec![2]),
            node(
                NodeKind::CompOutput {
                    comp,
                    width: 8,
                    height: 8,
                },
                vec![3],
            ),
        ],
        output: 4,
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
    // Invert of red in linear light is cyan; a small tolerance covers the
    // linear→display round-trip (as the linear-blend test uses).
    let px = rig.readback(out);
    let (r, g, b, a) = (i32::from(px[0]), i32::from(px[1]), i32::from(px[2]), px[3]);
    assert!(
        r <= 1 && (g - 255).abs() <= 1 && (b - 255).abs() <= 1 && a == 255,
        "Invert adjustment over red should read back cyan ≈(0,255,255,255), got ({r},{g},{b},{a})"
    );
}

#[test]
fn a_blend_mode_carries_through_the_seams() {
    let Some(rig) = Rig::new((8, 8)) else { return };
    let (green_bottom, red_top) = (Uuid::now_v7(), Uuid::now_v7());
    let (l_top, l_bottom, comp) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    let v = 128u8; // a mid value on its own channel, so Add never clamps
    let mut source = rig.source(HashMap::from([
        (green_bottom, [0, v, 0, 255]),
        (red_top, [v, 0, 0, 255]),
    ]));
    let mut kernels = rig.kernels([0.0, 0.0, 0.0, 1.0], HashMap::new());
    let mut cache = MapCache::default();
    let node = |kind, inputs| Node { kind, inputs };
    let graph = EvalGraph {
        nodes: vec![
            node(
                NodeKind::Source {
                    source: SourceRef::Solid(green_bottom),
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
                    source: SourceRef::Solid(red_top),
                },
                vec![],
            ),
            node(NodeKind::Transform { layer: l_top }, vec![3]),
            node(
                NodeKind::Composite {
                    layer: l_top,
                    blend: lumit_core::model::BlendMode::Add,
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
    // Add mixes in linear light; the channels don't overlap, so each survives
    // its own round-trip back to ~v. The key point is that both are present:
    // green did not get replaced (Normal would leave green = 0).
    let px = rig.readback(out);
    let (r, g, b, a) = (i32::from(px[0]), i32::from(px[1]), i32::from(px[2]), px[3]);
    let vi = i32::from(v);
    assert!(
        (r - vi).abs() <= 1 && (g - vi).abs() <= 1 && b == 0 && a == 255,
        "Add of red-over-green should keep both channels ≈({vi},{vi},0,255), got ({r},{g},{b},{a})"
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

/// A transform's placement carries through the seams: a 4×4 solid positioned
/// into the bottom-right quadrant of an 8×8 comp colours exactly those
/// pixels, with the background everywhere else — the compositor's own
/// `place_matrix` maths, driven by the executor.
#[test]
fn a_placed_layer_lands_where_the_transform_puts_it() {
    let Some(rig) = Rig::new((8, 8)) else { return };
    let (solid, layer, comp) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    let mut source = rig.source_sized(HashMap::from([(solid, ([255, 0, 0, 255], (4, 4)))]));
    let placement = Placement {
        size: (4.0, 4.0),
        position: (4.0, 4.0),
        anchor: (0.0, 0.0),
        scale: (100.0, 100.0),
        rotation_deg: 0.0,
        opacity: 100.0,
    };
    let mut kernels = rig.kernels([0.0, 0.0, 0.0, 1.0], HashMap::from([(layer, placement)]));
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
    let px = rig.readback(out);
    for y in 0..8usize {
        for x in 0..8usize {
            let p = &px[(y * 8 + x) * 4..(y * 8 + x) * 4 + 4];
            let expected: [u8; 4] = if x >= 4 && y >= 4 {
                [255, 0, 0, 255] // the placed solid
            } else {
                [0, 0, 0, 255] // the background
            };
            assert_eq!(p, expected, "pixel ({x},{y})");
        }
    }
}
