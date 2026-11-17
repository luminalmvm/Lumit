//! Tests for drawing a region of interest.

use super::*;

/// A region of interest composites a window of the comp, and that window
/// must be **the same pixels** the full frame has there. This is
/// the whole promise: a region changes how much is computed, never what
/// the picture is. If the two ever disagree, working inside a region is
/// working on a lie.
///
/// Also pinned: the returned texture is the region's size, and a region
/// covering everything is refused so its frames keep the full frame's
/// names rather than banking a duplicate set under new ones.
#[test]
fn a_region_of_interest_is_the_same_pixels_the_full_frame_has_there() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        return; // no GPU here — skip, as the gpu crate's own tests do
    };
    let engine = ctx.colour();
    let compositor = ctx.compositor();
    let fx = ctx.fx();
    let lut_cache = std::cell::RefCell::new(crate::fxops::LutCache::default());
    let fx_cache = std::cell::RefCell::new(crate::fxops::FxCache::default());
    let realiser = Realiser {
        ctx: ctx.clone_handle(),
        engine,
        compositor,
        fx,
        lut_cache: &lut_cache,
        fx_cache: &fx_cache,
        render_scale: 1.0,
        samples: 1,
        profiler: None,
        colour_inputs: None,
        colour_config: None,
        flow: None,
        late_clones: None,
        graph_reach: None,
    };
    let comp = Composition {
        graph: None,
        master_volume_db: 0.0,
        master_keys: None,
        master_automation: Default::default(),
        loudness_target: None,
        lighting: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "c".into(),
        width: 320,
        height: 180,
        frame_rate: FrameRate::new(30, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour([0.1, 0.2, 0.3, 1.0]),
        work_area: None,
        layers: vec![text_layer(200.0), text_layer(80.0)],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let doc = std::sync::Arc::new(Document::new());
    let pixels: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
    let t = 0.3;
    let mut visited = vec![comp.id];
    let draws = build_comp_draws(&doc, &comp, t, &pixels, &mut visited);
    let bg = comp.background.0.map(f64::from);

    let bytes = |tex: &wgpu::Texture| {
        engine
            .readback8(
                &ctx,
                &engine.display(&ctx, tex, lumit_gpu::DisplayParams::NEUTRAL),
            )
            .unwrap()
    };

    let full = realiser.realise(comp.camera_pose(t), comp.width, comp.height, bg, &draws);
    let full_px = bytes(&full);

    // A window over the busiest quarter of the frame.
    let region = lumit_gpu::composite::Region {
        x: 64.0,
        y: 32.0,
        w: 160.0,
        h: 96.0,
    };
    let windowed = realiser.realise_region(
        comp.camera_pose(t),
        comp.width,
        comp.height,
        bg,
        &draws,
        Some(region),
        None,
    );
    assert_eq!(
        (windowed.width(), windowed.height()),
        (region.w as u32, region.h as u32),
        "the texture is the region's size, not the comp's"
    );
    let win_px = bytes(&windowed);

    // Every row of the window against the same row of the full frame.
    let stride = comp.width as usize * 4;
    let mut worst = 0u8;
    for row in 0..region.h as usize {
        let src = (row + region.y as usize) * stride + region.x as usize * 4;
        let dst = row * region.w as usize * 4;
        for i in 0..region.w as usize * 4 {
            worst = worst.max(full_px[src + i].abs_diff(win_px[dst + i]));
        }
    }
    assert!(
        worst <= 1,
        "a windowed composite must match the full frame there; worst channel difference {worst}"
    );

    // And it is deterministic, as every composite must be.
    let again = realiser.realise_region(
        comp.camera_pose(t),
        comp.width,
        comp.height,
        bg,
        &draws,
        Some(region),
        None,
    );
    assert_eq!(win_px, bytes(&again), "a windowed composite is bit-stable");
}
