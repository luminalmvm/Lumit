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
        display_blend: false,
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

/// **An Echo trail on a float plate is the plate's own colour.** The
/// neighbour frames come out of the same decode as the frame they sit beside,
/// so a float plate's neighbours are floats too, and they were uploaded as
/// eight-bit bytes: the trail came out as static and the export with it.
#[test]
fn a_float_plates_neighbours_are_read_as_floats() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        lumit_gpu::no_adapter();
        return;
    };
    let lut_cache = std::cell::RefCell::new(crate::fxops::LutCache::default());
    let fx_cache = std::cell::RefCell::new(crate::fxops::FxCache::default());
    let realiser = realiser(&ctx, &lut_cache, &fx_cache);
    let colour = [2.0f32, 0.5, 0.25, 1.0];
    let float_plate: Vec<u8> = (0..W * H)
        .flat_map(|_| colour)
        .flat_map(f32::to_le_bytes)
        .collect();
    let mut l = draw(&std::sync::Arc::new(float_plate.clone()));
    if let DrawSource::Pixels { format, .. } = &mut l.source {
        *format = lumit_media::decode::PixelFormat::LinearF32;
    }
    let brighter: Vec<u8> = (0..W * H)
        .flat_map(|_| [colour[0] * 2.0, colour[1] * 2.0, colour[2] * 2.0, 1.0])
        .flat_map(f32::to_le_bytes)
        .collect();
    l.neighbours = vec![(-1, std::sync::Arc::new(brighter), W, H)];
    let mut echo = lumit_core::fx::builtins::instantiate("echo").expect("echo is a built-in");
    for p in &mut echo.params {
        match p.id.as_str() {
            "echoes" => {
                p.value =
                    lumit_core::model::EffectValue::Float(lumit_core::anim::Property::fixed(1.0))
            }
            "mode" => p.value = lumit_core::model::EffectValue::Choice(1),
            _ => {}
        }
    }
    l.fx = lumit_core::fx::resolved::resolve_stack(
        std::slice::from_ref(&echo),
        0.0,
        1000.0,
        1.0,
        &lumit_core::fx::markers::MarkerContext::NONE,
        std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
    );

    let out = realiser.realise(None, W, H, [0.0; 4], std::slice::from_ref(&l));
    let px = lumit_gpu::fx::common::readback_linear_f32(&ctx, &out, W, H).expect("reads back");
    let centre = ((H / 2 * W + W / 2) * 4) as usize;
    for c in 0..3 {
        let want = colour[c] * 1.6;
        assert!(
            (px[centre + c] - want).abs() < want * 0.01,
            "channel {c}: {} where the trail should make {want}",
            px[centre + c]
        );
    }
}

/// **A stack's picture is multiplied out once.** A layer's own pixels arrive
/// with their colour apart from their alpha; the stack is handed them
/// multiplied out and the composite reads what it made as such. Before, the
/// composite multiplied the stack's picture by its alpha a second time, so
/// any light an effect laid where the layer was part clear came out dimmed:
/// here a Fill on a half-clear pixel drew a quarter of the light, and Light
/// rays past an opaque solid's edge fell off a cliff there.
#[test]
fn a_stack_on_a_layers_own_pixels_is_multiplied_out_once() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        lumit_gpu::no_adapter();
        return;
    };
    let lut_cache = std::cell::RefCell::new(crate::fxops::LutCache::default());
    let fx_cache = std::cell::RefCell::new(crate::fxops::FxCache::default());
    let realiser = realiser(&ctx, &lut_cache, &fx_cache);
    let fill = {
        let inst = lumit_core::fx::builtins::instantiate("fill").expect("a built-in");
        lumit_core::fx::resolved::resolve_stack(
            &[inst],
            0.0,
            1000.0,
            1.0,
            &lumit_core::fx::markers::MarkerContext::NONE,
            std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
        )
    };
    let centre = |alpha: u8| {
        let rgba = std::sync::Arc::new([255u8, 255, 255, alpha].repeat((W * H) as usize));
        let mut l = draw(&rgba);
        l.fx = fill.clone();
        let (px, _) = made(&ctx, || {
            realiser.realise(None, W, H, [0.0; 4], std::slice::from_ref(&l))
        });
        let i = (((H / 2) * W + W / 2) * 4) as usize;
        [px[i], px[i + 1], px[i + 2], px[i + 3]]
    };
    let full = centre(255);
    let half = centre(128);
    let a = 128.0 / 255.0;
    assert!(full[0] + full[1] + full[2] > 0.1, "the fill draws: {full:?}");
    for c in 0..3 {
        assert!(
            (half[c] - full[c] * a).abs() < 4e-3,
            "a half-clear pixel carries half the light, channel {c}: {half:?} of {full:?}"
        );
    }
    assert!((half[3] - a).abs() < 4e-3, "and half the cover: {half:?}");
}
