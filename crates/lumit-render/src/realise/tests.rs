use super::*;

const W: u32 = 64;
const H: u32 = 64;

/// An sRGB plate with a value in every part of the curve, so a missed or
/// doubled decode shows up as a difference rather than as rounding.
fn plate() -> std::sync::Arc<Vec<u8>> {
    let mut px = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            px.extend_from_slice(&[(x * 4) as u8, (y * 4) as u8, ((x + y) * 2) as u8, 255]);
        }
    }
    std::sync::Arc::new(px)
}

/// One footage-shaped draw at the comp's own size, centred: eight-bit
/// pixels, no colour space, nothing on it.
fn draw(rgba: &std::sync::Arc<Vec<u8>>) -> CompLayerDraw {
    CompLayerDraw {
        layer: uuid::Uuid::now_v7(),
        source: DrawSource::Pixels {
            rgba: rgba.clone(),
            tex_w: W,
            tex_h: H,
            format: lumit_media::decode::PixelFormat::Srgb8,
            colour_space: None,
        },
        natural_size: (W as f32, H as f32),
        position: (W as f32 / 2.0, H as f32 / 2.0),
        anchor: (W as f32 / 2.0, H as f32 / 2.0),
        scale: (100.0, 100.0),
        rotation_deg: 0.0,
        opacity: 100.0,
        z: 0.0,
        rotation_x_deg: 0.0,
        rotation_y_deg: 0.0,
        three_d: false,
        ends_run: false,
        matte: None,
        blend: lumit_gpu::composite::Blend::Normal,
        mask_cov: None,
        pre: None,
        fx: lumit_core::fx::params::ResolvedStack::new(),
        fx_ids: Vec::new(),
        graph_fx: Vec::new(),
        neighbours: Vec::new(),
        flow_fields: Vec::new(),
        colour_tables: Vec::new(),
        dof_inputs: Vec::new(),
        mattes: Vec::new(),
        mask_paths: Vec::new(),
        roto_mattes: Vec::new(),
        planes: Vec::new(),
        points_schedules: Vec::new(),
        flare_lens_files: Vec::new(),
        fx_ref_width: None,
        fx_input_key: None,
        mb: Vec::new(),
        lights: Vec::new(),
        material: Default::default(),
        temporal_below: None,
        accumulation_below: None,
        flow_below: Vec::new(),
        in_scene: None,
    }
}

/// The placement `realise_segment` gives that draw, so the hand-built
/// reference below composites the same rectangle in the same place.
fn placed<'a>(
    l: &CompLayerDraw,
    texture: &'a wgpu::Texture,
) -> lumit_gpu::composite::CompositeLayer<'a> {
    lumit_gpu::composite::CompositeLayer {
        texture,
        size: l.natural_size,
        position: l.position,
        anchor: l.anchor,
        scale: l.scale,
        rotation_deg: l.rotation_deg,
        opacity: l.opacity,
        matte: None,
        blend: l.blend,
        z: l.z,
        rotation_x_deg: l.rotation_x_deg,
        rotation_y_deg: l.rotation_y_deg,
        three_d: l.three_d,
        layer_mask: None,
        pre: None,
        premultiplied: false,
    }
}

/// Run `f` with the driver's texture count read either side of it, and
/// hand back its picture and how many textures it made.
///
/// The batch is held open around it so nothing is submitted while it runs:
/// a submission is where wgpu hands back what a render has finished with,
/// and the question here is what the render made, not what survived it.
/// The lease is exclusive, so nothing else is allocating on this device.
///
/// A texture dropped before the count can still be handed back during it,
/// which only ever makes the count smaller, so the largest of three counts
/// is the real one. Seen on CI as one texture short.
fn made(ctx: &lumit_gpu::GpuContext, f: impl Fn() -> wgpu::Texture) -> (Vec<f32>, u64) {
    let mut most = (Vec::new(), 0);
    for _ in 0..3 {
        let (px, count) = made_once(ctx, &f);
        most = (px, most.1.max(count));
    }
    most
}

fn made_once(ctx: &lumit_gpu::GpuContext, f: &impl Fn() -> wgpu::Texture) -> (Vec<f32>, u64) {
    // Two throwaway renders first, each followed by a command buffer of its
    // own and a wait. A render's dropped textures come back only when a
    // LATER submission finishes, so without something behind them they sit
    // pending and are handed back in the middle of the count instead.
    for _ in 0..2 {
        drop(f());
        drop(ctx.encoder("settling"));
        ctx.settle();
    }
    // The test before this one on the shared device may have dropped far
    // more than a render's worth, and that comes back over several
    // submissions. So wait until the count has stood still twice running.
    let mut live = ctx.live_objects().0;
    let mut still = 0;
    for _ in 0..32 {
        drop(ctx.encoder("settling"));
        ctx.settle();
        let now = ctx.live_objects().0;
        still = if now == live { still + 1 } else { 0 };
        live = now;
        if still == 2 {
            break;
        }
    }
    let before = ctx.live_objects().0;
    ctx.begin_frame();
    let out = f();
    let after = ctx.live_objects().0;
    ctx.end_frame();
    let px =
        lumit_gpu::fx::common::readback_linear_f32(ctx, &out, W, H).expect("the comp reads back");
    drop(out);
    ctx.settle();
    (px, after.saturating_sub(before))
}

/// A realiser on the shared device, with everything a plain footage layer
/// never asks for left out.
fn realiser<'a>(
    shared: &'a lumit_gpu::test_support::Lease,
    lut_cache: &'a std::cell::RefCell<crate::fxops::LutCache>,
    fx_cache: &'a std::cell::RefCell<crate::fxops::FxCache>,
) -> Realiser<'a> {
    Realiser {
        ctx: shared.clone_handle(),
        engine: shared.colour(),
        compositor: shared.compositor(),
        fx: shared.fx(),
        lut_cache,
        fx_cache,
        render_scale: 1.0,
        samples: 1,
        profiler: None,
        colour_inputs: None,
        colour_config: None,
        flow: None,
        late_clones: None,
        graph_reach: None,
    }
}

/// **A plain eight-bit layer skips the linearise pass.** Its picture is
/// the one the upload-linearise-composite route drew, to the byte, and it
/// costs one texture fewer: at 8K that texture is a quarter of a gigabyte
/// and a full-frame pass, per layer, per frame.
#[test]
fn a_plain_footage_layer_goes_straight_to_the_composite() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        lumit_gpu::no_adapter();
        return;
    };
    let lut_cache = std::cell::RefCell::new(crate::fxops::LutCache::default());
    let fx_cache = std::cell::RefCell::new(crate::fxops::FxCache::default());
    let realiser = realiser(&ctx, &lut_cache, &fx_cache);
    let rgba = plate();
    let l = draw(&rgba);

    let (got, short) = made(&ctx, || {
        realiser.realise(None, W, H, [0.0; 4], std::slice::from_ref(&l))
    });
    // The route this replaced, by hand: the sRGB upload, a working-format
    // copy of it, and the same composite draw over the same background.
    let (want, long) = made(&ctx, || {
        let src = ctx.colour().upload_srgb8(&ctx, &rgba, W, H);
        let linear = ctx.colour().linearise(&ctx, &src);
        ctx.compositor().composite_seeded(
            &ctx,
            W,
            H,
            [0.0; 4],
            &[placed(&l, &linear)],
            None,
            None,
            1.0,
            1,
            None,
        )
    });

    assert_eq!(got, want, "the hardware decode is the decode the pass did");
    // Fewer, not exactly one fewer: the plain route drops its upload inside
    // the call, and a driver that hands it back before the count ends makes
    // the count one short on some runners.
    assert!(
        short < long,
        "the working-format copy is the texture that is no longer made (short {short} long {long})"
    );
}
