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
//! coverage), Masks (coverage applied at Composite), CompOutput (over the comp
//! background). Adjust and Retime are the remaining slices. Skips cleanly when no GPU
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
                    1.0,
                    1,
                    None,
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

/// The document's `BlendMode` → the compositor's `Blend`. The two enums carry
/// the same variants by design, so this is a plain 1:1 map. It mirrors
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
    /// The shared GPU fixture's lease, held for as long as this rig's own
    /// device exists.
    ///
    /// # Why a rig with its own device still takes the lease
    ///
    /// This file's tests run in parallel, as Rust's harness runs everything,
    /// and each one used to call `GpuContext::headless()` on its own. That is
    /// one wgpu device per test, all live at once, all submitting to the same
    /// card — which on a CI runner's software adapter means contention, and on
    /// a real one means several tests inside the same TDR window. It shows up
    /// as flakes that move about when tests are added, which is the worst kind
    /// to chase.
    ///
    /// Holding the lease makes the device's *lifetime* the serialised thing,
    /// rather than nothing being serialised at all. The rig keeps building its
    /// own context because these tests drive a bare compositor skeleton and
    /// want a device with nothing else's state on it — but now only one of them
    /// exists at a time, and no shared-GPU test is running beside it either.
    ///
    /// Named with a leading underscore because it is never read: its whole job
    /// is to be dropped when the rig is.
    _lease: lumit_gpu::test_support::Lease,
    ctx: Rc<GpuContext>,
    colour: Rc<ColourEngine>,
    frames: Rc<RefCell<Frames>>,
    size: (u32, u32),
}

impl Rig {
    fn new(size: (u32, u32)) -> Option<Self> {
        // Before the device, not after: a rig that built its own card first and
        // then waited for the lease would be exactly the overlap this is here
        // to prevent.
        let lease = lumit_gpu::test_support::lease()?;
        let Ok(ctx) = GpuContext::headless() else {
            lumit_gpu::no_adapter();
            return None;
        };
        let ctx = Rc::new(ctx);
        let colour = Rc::new(ColourEngine::new(&ctx));
        Some(Self {
            _lease: lease,
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
        }
    }

    /// sRGB bytes of one produced frame (via the display transform, exactly
    /// as the Viewer and export read frames out).
    fn readback(&self, h: FrameHandle) -> Vec<u8> {
        let frames = self.frames.borrow();
        let tex = frames.get(h).expect("handle is valid");
        let shown = self
            .colour
            .display(&self.ctx, tex, lumit_gpu::DisplayParams::NEUTRAL);
        self.colour.readback8(&self.ctx, &shown).expect("readback")
    }
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
