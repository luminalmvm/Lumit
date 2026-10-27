//! Particulate's Trails: each frame's particles drawn over the picture kept
//! from the frame before.
//!
//! A frame's picture is the walk from the layer's in point. The picture so far
//! is faded, zoomed and turned by one frame of the Trails rows, then that
//! frame's particles are drawn over it, read at that frame's own time through
//! the driver walk. Pictures are kept on the card every [`KEEP_EVERY`] frames
//! and at the last frame drawn, named by the walk, the frame and the raster,
//! so playing on draws one frame and scrubbing back a few. A kept picture is
//! only a shortcut: a frame reached from one is the frame walked from the
//! start.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use lumit_core::expression::ExpressionContext;
use lumit_core::fx::effects::particulate::Particulate as P;
use lumit_core::fx::params::Params;
use lumit_core::fx::points::{Projection, RenderMode, Trails};
use lumit_core::fx::registry::EffectMetadata;
use lumit_core::model::LayerKind;
use lumit_gpu::fx::points_draw::{DrawPoint, FieldTest, PointsDrawOp};
use lumit_gpu::fx::stylise::TransformOp;
use lumit_gpu::fx::temporal::EchoOp;
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

use crate::fxops::CachedTex;
use crate::gpufx::AuxSlot;

type Tex = wgpu::Texture;

/// How often a walk keeps its picture, in frames.
const KEEP_EVERY: i64 = 8;

/// The most bytes kept pictures hold between them, on the card.
const KEPT_BYTES: usize = 512 << 20;

/// Kept pictures, for the one card they are on.
struct Store {
    ledger: Arc<lumit_budget::Ledger>,
    pictures: lumit_cache::ByteLru<u128, CachedTex>,
    /// The latest frame kept of each walk.
    latest: HashMap<[u8; 32], i64>,
}

static KEPT: Mutex<Option<Store>> = Mutex::new(None);

/// Runs `f` on the store for `ctx`'s card. Another card starts it afresh.
fn with_store<R>(ctx: &GpuContext, f: impl FnOnce(&mut Store) -> R) -> Option<R> {
    let mut slot = KEPT.lock().ok()?;
    if !slot
        .as_ref()
        .is_some_and(|s| Arc::ptr_eq(&s.ledger, ctx.ledger()))
    {
        let mut pictures = lumit_cache::ByteLru::new(KEPT_BYTES);
        pictures.account_against(Arc::clone(ctx.ledger()), lumit_budget::Tier::Vram);
        *slot = Some(Store {
            ledger: Arc::clone(ctx.ledger()),
            pictures,
            latest: HashMap::new(),
        });
    }
    slot.as_mut().map(f)
}

fn name(walk: &[u8; 32], frame: i64) -> u128 {
    let mut h = blake3::Hasher::new();
    h.update(walk);
    h.update(&frame.to_le_bytes());
    let mut out = [0u8; 16];
    out.copy_from_slice(&h.finalize().as_bytes()[..16]);
    u128::from_le_bytes(out)
}

/// The furthest picture kept of `walk` from `from` to `frame`, and its frame.
fn kept(ctx: &GpuContext, walk: &[u8; 32], frame: i64, from: i64) -> Option<(i64, Tex)> {
    with_store(ctx, |s| {
        let mut latest = s.latest.get(walk).copied().filter(|l| *l <= frame);
        let mut k = frame.div_euclid(KEEP_EVERY) * KEEP_EVERY;
        // Furthest first: the latest where it is ahead, then every eighth.
        loop {
            let at = match latest {
                Some(l) if l >= k => {
                    latest = None;
                    l
                }
                _ => {
                    k -= KEEP_EVERY;
                    k + KEEP_EVERY
                }
            };
            if at < from {
                return None;
            }
            if let Some(t) = s.pictures.get(&name(walk, at)) {
                return Some((at, t.0.clone()));
            }
        }
    })
    .flatten()
}

/// Keep `tex` as `walk`'s picture at `frame`: for good, or as the latest,
/// which the next latest replaces.
fn keep(ctx: &GpuContext, walk: &[u8; 32], frame: i64, tex: &Tex, sticky: bool) {
    let taken = with_store(ctx, |s| {
        if !sticky {
            if let Some(old) = s.latest.insert(*walk, frame) {
                if old != frame && old % KEEP_EVERY != 0 {
                    s.pictures.remove(&name(walk, old));
                }
            }
        }
        s.pictures.insert(name(walk, frame), CachedTex(tex.clone()))
    });
    // The store holds it past the frame now, so the frame stops paying for it.
    if taken == Some(true) {
        ctx.hand_off_vram(u64::from(tex.width()) * u64::from(tex.height()) * 8);
    }
}

/// One frame of the walk, as it is drawn.
struct Frame {
    points: Vec<DrawPoint>,
    feather: f32,
    projection: Option<[[f32; 4]; 3]>,
    /// How much of the picture before is left, and the inverse affine it is
    /// sampled through. `None` keeps nothing.
    carry: Option<(f32, [f32; 4], [f32; 2])>,
}

/// Frame `lt`'s particles and rows, everything read at that layer time.
fn frame_at(
    trails: &Trails,
    effect: uuid::Uuid,
    lt: f64,
    dt: f64,
    px_scale: f32,
    streak: bool,
) -> Option<Frame> {
    let doc = &trails.doc;
    let comp = doc.comp(trails.comp)?;
    let layer = comp.layers.iter().find(|l| l.id == trails.layer)?;
    let inst = layer.effects.iter().find(|e| e.id == effect)?;
    let tc = lt + layer.start_offset.0.to_f64();
    let context = Arc::new(ExpressionContext {
        document: Arc::clone(doc),
        comp: Some(comp.id),
        layer: Some(layer.id),
        comp_time: tc,
        current_depth: 0,
        inputs: None,
    });
    let audio = crate::audio_tap::DocumentAudio::new(doc, comp, tc);
    // A wired row reads its wire, as the bag does.
    let row = |id: &str, or: f64| -> f32 {
        lumit_core::fx::drivers::driven_param(
            &layer.graph,
            effect,
            id,
            lt,
            Arc::clone(&context),
            Some(&audio),
        )
        .or_else(|| inst.float_at_with_context(id, lt, Arc::clone(&context)))
        .unwrap_or(or) as f32
    };
    let stream = lumit_core::fx::drivers::effect_stream_in(
        &layer.graph,
        effect,
        lt,
        Arc::clone(&context),
        Some(&audio),
        Projection::FLAT,
        None,
        trails.clone,
    )
    .unwrap_or_default()
    .rescaled(px_scale);
    // The camera at this frame, moved by the room the picture has, as the
    // draw builder moves it.
    let projection = crate::build::points_projection(doc, comp, layer, tc, lt, context.clone());
    let projection = if trails.inset == [0.0; 2] {
        projection
    } else {
        let [mut x, mut y, w] = projection.unwrap_or_default().m;
        for ((x, y), w) in x.iter_mut().zip(y.iter_mut()).zip(w) {
            *x += trails.inset[0] * w;
            *y += trails.inset[1] * w;
        }
        Some(Projection { m: [x, y, w] })
    }
    .map(|p| p.rescaled(px_scale));
    let back = if streak {
        row("streak_length", 0.02).max(0.0)
    } else {
        0.0
    };
    let fade = (row("persistence", 0.0) / 100.0)
        .clamp(0.0, 1.0)
        .powf(dt as f32);
    let grow = (1.0 + row("zoom", 0.0) / 100.0).max(0.0).powf(dt as f32);
    let turn = row("turn", 0.0) * dt as f32;
    // Zoom and Turn go about the emitter, where it is drawn.
    let at = [
        row("position_x", 960.0) * px_scale,
        row("position_y", 540.0) * px_scale,
        row("position_z", 0.0) * px_scale,
    ];
    let centre = projection.map_or([at[0], at[1]], |p| p.apply(at).0);
    let carry = (fade > 0.0)
        .then(|| {
            lumit_core::fx::maths::transform_inverse(centre, centre, [grow; 2], turn, [0.0; 2])
        })
        .flatten()
        .map(|(m, off)| (fade, m, off));
    Some(Frame {
        points: crate::gpufx::stream_points(&stream, back),
        feather: (row("feather", 100.0) / 100.0).clamp(0.0, 1.0),
        projection: projection.map(|p| p.m),
        carry,
    })
}

/// A transparent picture the size of `like`.
fn blank(fx: &FxEngine, ctx: &GpuContext, like: &Tex, w: u32, h: u32) -> Tex {
    fx.transform(
        ctx,
        like,
        w,
        h,
        None,
        &TransformOp {
            m: [1.0, 0.0, 0.0, 1.0],
            off: [0.0; 2],
            opacity: 0.0,
            mix: 1.0,
            edge: 0,
        },
    )
}

/// `f`'s particles drawn over what `before` leaves of itself, or over nothing.
#[allow(clippy::too_many_arguments)]
fn one(
    fx: &FxEngine,
    ctx: &GpuContext,
    like: &Tex,
    w: u32,
    h: u32,
    before: Option<&Tex>,
    f: &Frame,
    seed: u32,
    sprite: Option<&Tex>,
) -> Tex {
    let base = match (before, f.carry) {
        (Some(b), Some((opacity, m, off))) => fx.transform(
            ctx,
            b,
            w,
            h,
            None,
            &TransformOp {
                m,
                off,
                opacity,
                mix: 1.0,
                edge: 0,
            },
        ),
        _ => blank(fx, ctx, like, w, h),
    };
    let out = fx.points_draw(
        ctx,
        &base,
        w,
        h,
        None,
        &PointsDrawOp {
            points: &f.points,
            feather: f.feather,
            mix: 1.0,
            projection: f.projection,
            field: FieldTest::None,
            seed,
            mode: u32::from(sprite.is_some()),
            sprite,
        },
    );
    ctx.recycle(base);
    out
}

/// Particulate with Trails on, over `input`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw(
    fx: &FxEngine,
    ctx: &GpuContext,
    input: &Tex,
    w: u32,
    h: u32,
    p: Params<'_>,
    aux: AuxSlot<'_>,
    trails: &Trails,
) -> Tex {
    let inst = P::read(p);
    let style = inst.draw_style();
    let px_scale = P::px_scale_of(p);
    let (effect, _) = aux.op();
    let t = aux.schedule().map_or(0.0, |s| s.t);
    let Some(comp) = trails.doc.comp(trails.comp) else {
        return input.clone();
    };
    let Some(layer) = comp.layers.iter().find(|l| l.id == trails.layer) else {
        return input.clone();
    };
    let dt = 1.0 / comp.frame_rate.fps().max(1.0);
    // Before the in point there is nothing yet.
    if t.is_nan() || t < 0.0 {
        return input.clone();
    }
    let target = (t / dt + 1e-6).floor() as i64;
    let from = target
        .saturating_sub(lumit_core::fx::points::MAX_SCAN_FRAMES)
        .max(0);
    let streak = style.mode == RenderMode::Streak;
    let sprite = <P as EffectMetadata>::SCHEMA
        .layer_inputs()
        .position(|row| row == "sprite_layer")
        .and_then(|at| aux.layer_inputs().get(at)?.as_ref().map(|l| &l.tex))
        .filter(|_| style.mode == RenderMode::Sprite);
    // A walk that started late is not the walk from the in point. Every frame
    // is stamped with the sprite as it is now, which another frame may not
    // share, so a sprite walk keeps nothing.
    let keeps = from == 0 && sprite.is_none();

    // The walk's name: what it reads, the raster it is drawn at, and the
    // camera a 3D layer is seen through.
    let walk = {
        let mut h_ = blake3::Hasher::new();
        h_.update(b"trails/");
        h_.update(&trails.key);
        h_.update(&w.to_le_bytes());
        h_.update(&h.to_le_bytes());
        h_.update(&px_scale.to_le_bytes());
        for v in trails.inset {
            h_.update(&v.to_le_bytes());
        }
        h_.update(&trails.clone.0.to_le_bytes());
        h_.update(&trails.clone.1.to_le_bytes());
        if layer.switches.three_d {
            for cam in comp
                .layers
                .iter()
                .filter(|l| matches!(l.kind, LayerKind::Camera { .. }))
            {
                for id in std::iter::once(cam.id)
                    .chain(lumit_core::model::layer_parent_chain(comp, cam.id))
                {
                    if let Some(one) = comp.layers.iter().find(|l| l.id == id) {
                        let _ = serde_json::to_writer(&mut h_, one);
                    }
                }
            }
        }
        *h_.finalize().as_bytes()
    };

    let (mut frame, mut picture) = match keeps.then(|| kept(ctx, &walk, target, from)).flatten() {
        Some((k, tex)) => (k, Some(tex)),
        None => (from - 1, None),
    };
    let start = frame;
    // Whether `picture` is in the store, so never handed back.
    let mut held = picture.is_some();
    while frame < target {
        // A frame nobody wants any more stops here, and keeps where it got to.
        if crate::cancel::requested() {
            crate::cancel::cut_short();
            break;
        }
        let k = frame + 1;
        let Some(made) = frame_at(trails, effect, k as f64 * dt, dt, px_scale, streak) else {
            break;
        };
        let next = one(
            fx,
            ctx,
            input,
            w,
            h,
            picture.as_ref(),
            &made,
            inst.seed,
            sprite,
        );
        if let Some(old) = picture.take().filter(|_| !held) {
            ctx.recycle(old);
        }
        held = false;
        frame = k;
        if keeps && k % KEEP_EVERY == 0 {
            keep(ctx, &walk, k, &next, true);
            held = true;
            // A long walk hands its work over as it goes, so the card is not
            // left holding every picture it passed.
            ctx.flush();
        }
        picture = Some(next);
    }
    if let Some(last) = picture
        .as_ref()
        .filter(|_| keeps && frame > start && frame % KEEP_EVERY != 0)
    {
        keep(ctx, &walk, frame, last, false);
        held = true;
    }
    let Some(mut shown) = picture.filter(|_| frame == target) else {
        return input.clone();
    };
    // A time between frames draws its own particles over the frame before.
    if t - target as f64 * dt > 1e-9 {
        if let Some(made) = frame_at(trails, effect, t, dt, px_scale, streak) {
            let over = fx.points_draw(
                ctx,
                &shown,
                w,
                h,
                None,
                &PointsDrawOp {
                    points: &made.points,
                    feather: made.feather,
                    mix: 1.0,
                    projection: made.projection,
                    field: FieldTest::None,
                    seed: inst.seed,
                    mode: u32::from(sprite.is_some()),
                    sprite,
                },
            );
            if !held {
                ctx.recycle(shown);
            }
            shown = over;
            held = false;
        }
    }
    // The kept picture goes over the layer's, by Echo's In front.
    let mut weights = [0.0; 16];
    weights[0] = 1.0;
    let out = fx.echo(
        ctx,
        input,
        &[(-1, &shown)],
        w,
        h,
        None,
        &EchoOp {
            weights,
            mode: 1,
            mix: style.mix,
        },
    );
    if !held {
        ctx.recycle(shown);
    }
    out
}
