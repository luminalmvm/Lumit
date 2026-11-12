//! Tests for the effect ops.

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

/// **A `.cube` edited on disk must be re-read** (docs/impl/lut.md §4).
///
/// The regression: the cache keyed by path alone, so exporting a new grade
/// over the same filename — the whole loop of grading — kept showing the
/// first one until the application was restarted, with nothing on screen to
/// say the file on disk and the picture had parted company.
#[test]
fn an_edited_cube_is_read_again() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
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

// ----- The per-effect cache -----

/// A stack of built-in effects with one float parameter set on each:
/// `(match_name, param id, value)`.
fn stack(spec: &[(&str, &str, f32)]) -> lumit_core::fx::params::ResolvedStack {
    let insts: Vec<_> = spec
        .iter()
        .map(|(name, param, value)| {
            let mut inst = lumit_core::fx::builtins::instantiate(name).expect("a built-in");
            for p in &mut inst.params {
                if p.id == *param {
                    p.value = lumit_core::model::EffectValue::Float(
                        lumit_core::anim::Property::fixed(f64::from(*value)),
                    );
                }
            }
            inst
        })
        .collect();
    lumit_core::fx::resolved::resolve_stack(
        &insts,
        0.0,
        1000.0,
        1.0,
        &lumit_core::fx::markers::MarkerContext::NONE,
        std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
    )
}

const W: u32 = 8;
const H: u32 = 8;

fn source(ctx: &GpuContext) -> Tex {
    let px: Vec<f32> = (0..(W * H * 4))
        .map(|i| match i % 4 {
            3 => 1.0,
            _ => (i % 13) as f32 / 13.0,
        })
        .collect();
    lumit_gpu::fx::common::upload_linear_f32(ctx, &px, W, H)
}

/// `run_ops` with every side list empty, through `cache` under `key`.
fn run(
    fx: &FxEngine,
    ctx: &GpuContext,
    ops: &lumit_core::fx::params::ResolvedStack,
    cache: &std::cell::RefCell<FxCache>,
    key: u128,
) -> Vec<f32> {
    let out = run_ops(
        fx,
        ctx,
        source(ctx),
        W,
        H,
        ops,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &Side::NONE,
        &[],
        None,
        Some((cache, key)),
    );
    lumit_gpu::fx::common::readback_linear_f32(ctx, &out, W, H).expect("readback")
}

fn warm_cache() -> std::cell::RefCell<FxCache> {
    let mut c = FxCache::default();
    c.keep_outputs(true);
    std::cell::RefCell::new(c)
}

/// [`run`] inside a frame batch — the shape the realiser renders in, where
/// the work-texture pool is live and an intermediate handed back is given
/// to the next pass instead of a fresh one being made. Answers how many
/// work textures the walk had to create, which is how many it was holding
/// at once.
fn textures_made_by(
    fx: &FxEngine,
    ctx: &GpuContext,
    ops: &lumit_core::fx::params::ResolvedStack,
    cache: &std::cell::RefCell<FxCache>,
    key: u128,
) -> u64 {
    let src = source(ctx);
    let before = ctx.work_textures_made();
    ctx.begin_frame();
    let _out = run_ops(
        fx,
        ctx,
        src,
        W,
        H,
        ops,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &Side::NONE,
        &[],
        None,
        Some((cache, key)),
    );
    ctx.end_frame();
    ctx.work_textures_made() - before
}

/// A stack of `n` Exposure ops, each at its own value so each output has a
/// name of its own.
fn long_stack(n: usize) -> lumit_core::fx::params::ResolvedStack {
    let spec: Vec<_> = (0..n)
        .map(|i| ("exposure", "stops", 0.05 * (i + 1) as f32))
        .collect();
    stack(&spec)
}

/// **The second rung of the ladder** (docs/13 §4): under pressure the cold
/// half of the intermediates goes, and the ledger hears about it.
#[test]
fn a_card_under_pressure_gives_back_the_cold_intermediates() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let mut c = FxCache::default();
    c.keep_outputs(true);
    c.account_against(std::sync::Arc::clone(ctx.ledger()));
    let warm = std::cell::RefCell::new(c);
    textures_made_by(fx, &ctx, &long_stack(6), &warm, 7);
    let before = warm.borrow().stats().0;
    assert!(before > 0, "the store took the walk's outputs");

    assert!(
        !warm.borrow_mut().trim_under_pressure(),
        "a card with room keeps every intermediate it has"
    );

    let used = ctx.ledger().used(lumit_budget::Tier::Vram);
    ctx.ledger().set_budget(lumit_budget::Tier::Vram, used);
    assert!(
        warm.borrow_mut().trim_under_pressure(),
        "a full card gives the cold ones back"
    );
    let after = warm.borrow().stats().0;
    assert!(
        after < before,
        "{after} bytes held where there were {before}"
    );
    assert_eq!(
        ctx.ledger().used(lumit_budget::Tier::Vram),
        after as u64,
        "and the ledger followed them out"
    );
}

/// **Pressure never moves a pixel.** Everything the renderer does on a
/// memory reading — stop filing, give the cold intermediates back, hand the
/// batch over effect by effect instead of once at the end — is invisible in
/// the output by construction, which is the whole reason those are the
/// steps it is allowed to take without being told (docs/13 §4; the rungs
/// that *do* change what is drawn belong to the caller, and export takes
/// none of them).
///
/// This is the test that makes that claim falsifiable: the same stack, run
/// on an empty card and on a full one, byte for byte.
#[test]
fn a_card_under_pressure_draws_the_same_picture() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let ops = long_stack(5);

    let easy = run(fx, &ctx, &ops, &warm_cache(), 7);
    assert_eq!(ctx.vram_pressure(), lumit_budget::Pressure::Easy);

    let used = ctx.ledger().used(lumit_budget::Tier::Vram);
    assert!(used > 0, "the first walk put textures on the card");
    ctx.ledger().set_budget(lumit_budget::Tier::Vram, used);
    assert_eq!(ctx.vram_pressure(), lumit_budget::Pressure::Full);

    let full = run(fx, &ctx, &ops, &warm_cache(), 7);
    assert_eq!(
        easy, full,
        "a full card renders what an empty one renders, to the bit"
    );
}

/// **A passthrough op must never put a picture the walk did not make into
/// the queue.** Such an op hands its input on unchanged (`tex == given`),
/// and that input is one of three things the walk does not own: the
/// layer's source, the store's held prefix, or the entry queued one op
/// earlier. Queued anyway, the first drop to make room records a clear
/// over it and offers it to the pool, and the next pass draws its output
/// *into* it.
///
/// The source is the case a test can see directly: a Roto brush with no
/// matte first, two ops after it, room in the store for one. Without the
/// guard the walk clears the picture it was handed and draws op two over
/// the top of it — visible to whoever else holds that texture, which in a
/// real frame is the realiser. Caught by review, not by a test, which is
/// why this one exists.
#[test]
fn a_passthrough_op_does_not_hand_the_layers_source_back_to_the_pool() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let src = source(&ctx);
    let before = lumit_gpu::fx::common::readback_linear_f32(&ctx, &src, W, H).expect("readback");

    // Room for exactly one output, so making room is forced on the
    // second op.
    let mut c = FxCache::new((W * H * 8) as usize);
    c.keep_outputs(true);
    let warm = std::cell::RefCell::new(c);

    let inst = |name: &str, stops: f32| {
        let mut i = lumit_core::fx::builtins::instantiate(name).expect("a built-in");
        for p in &mut i.params {
            if p.id == "stops" {
                p.value = lumit_core::model::EffectValue::Float(lumit_core::anim::Property::fixed(
                    f64::from(stops),
                ));
            }
        }
        i
    };
    let insts = vec![
        inst(lumit_core::roto::ROTO_BRUSH, 0.0),
        inst("exposure", 0.05),
        inst("exposure", 0.10),
    ];
    let ops = lumit_core::fx::resolved::resolve_stack(
        &insts,
        0.0,
        1000.0,
        1.0,
        &lumit_core::fx::markers::MarkerContext::NONE,
        std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
    );

    // Inside a frame, so the pool is live and a dropped entry is really
    // recycled — the shape the realiser runs in.
    ctx.begin_frame();
    let out = run_ops(
        fx,
        &ctx,
        src.clone(),
        W,
        H,
        &ops,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &Side::NONE,
        &[],
        None,
        Some((&warm, 7)),
    );
    ctx.end_frame();

    assert_ne!(out, src, "the walk made a new picture for its output");
    let after = lumit_gpu::fx::common::readback_linear_f32(&ctx, &src, W, H).expect("readback");
    assert_eq!(
        before, after,
        "the layer's source was handed to the pool and drawn over"
    );
}

/// One custom-shader instance holding `source`, resolved as the walk
/// resolves it.
fn shader_stack(source: &str) -> lumit_core::fx::params::ResolvedStack {
    let mut inst = lumit_core::fx::builtins::instantiate("custom_shader").expect("a built-in");
    let mut block = serde_json::Map::new();
    block.insert("source".into(), source.into());
    inst.extra
        .insert("shader".into(), serde_json::Value::Object(block));
    lumit_core::fx::resolved::resolve_stack(
        std::slice::from_ref(&inst),
        0.0,
        1000.0,
        1.0,
        &lumit_core::fx::markers::MarkerContext::NONE,
        std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
    )
}

/// The whole carriage, end to end: the document holds a shader, the
/// resolve walk reads it and puts its name in the bag, and the picture that
/// comes out is the one the shader describes — with no parallel list, no new
/// aux kind and nothing owned in the arena.
#[test]
fn a_custom_shader_in_a_stack_draws_what_its_text_says() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let plain = run(fx, &ctx, &shader_stack(""), &warm_cache(), 7);
    let before =
        lumit_gpu::fx::common::readback_linear_f32(&ctx, &source(&ctx), W, H).expect("read");
    assert_eq!(
        plain, before,
        "an instance with no program renders its input unchanged"
    );

    // Half the colour, alpha untouched.
    let halved = run(
        fx,
        &ctx,
        &shader_stack(
            "fn shade(uv: vec2<f32>) -> vec4<f32> {
                     let c = lumit_sample(uv);
                     return vec4<f32>(c.rgb * 0.5, c.a);
                 }
",
        ),
        &warm_cache(),
        7,
    );
    for i in (0..halved.len()).step_by(4) {
        assert!(
            (halved[i] - before[i] * 0.5).abs() < 2e-3,
            "pixel {i}: {} vs {}",
            halved[i],
            before[i] * 0.5
        );
        assert!((halved[i + 3] - before[i + 3]).abs() < 2e-3);
    }
}

/// **Editing the last effect re-runs only that one** — and the
/// picture is the one a cold walk makes.
#[test]
fn editing_the_last_effect_reruns_only_that_one() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let cache = warm_cache();

    let first = stack(&[
        ("saturation", "saturation", 20.0),
        ("exposure", "stops", 1.0),
    ]);
    run(fx, &ctx, &first, &cache, 7);
    assert_eq!(cache.borrow().counts(), (2, 0), "a cold walk runs both");
    assert_eq!(cache.borrow().stats().2, 2, "and files both outputs");

    let edited = stack(&[
        ("saturation", "saturation", 20.0),
        ("exposure", "stops", 2.0),
    ]);
    let warm = run(fx, &ctx, &edited, &cache, 7);
    assert_eq!(
        cache.borrow().counts(),
        (3, 1),
        "the saturation's output was held; only the exposure ran"
    );

    let cold = run(fx, &ctx, &edited, &warm_cache(), 7);
    assert_eq!(
        warm, cold,
        "a held prefix makes the same picture as a cold walk"
    );
}

/// An upstream edit renames everything after it: both ops run again.
#[test]
fn an_upstream_edit_misses() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let cache = warm_cache();
    run(
        fx,
        &ctx,
        &stack(&[
            ("saturation", "saturation", 20.0),
            ("exposure", "stops", 1.0),
        ]),
        &cache,
        7,
    );
    run(
        fx,
        &ctx,
        &stack(&[
            ("saturation", "saturation", 50.0),
            ("exposure", "stops", 1.0),
        ]),
        &cache,
        7,
    );
    assert_eq!(cache.borrow().counts(), (4, 0));
    // A different source under the same stack is a miss too.
    run(
        fx,
        &ctx,
        &stack(&[
            ("saturation", "saturation", 50.0),
            ("exposure", "stops", 1.0),
        ]),
        &cache,
        8,
    );
    assert_eq!(cache.borrow().counts(), (6, 0));
}

/// An op that binds a picture nobody named — another layer's texture as a
/// matte, the neighbour frames — breaks the chain: nothing from it on is
/// filed, and the same stack runs in full again.
#[test]
fn a_bound_picture_breaks_the_chain() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = ctx.fx();

    // A matte texture on the first op.
    let cache = warm_cache();
    let ops = stack(&[
        ("saturation", "saturation", 20.0),
        ("exposure", "stops", 1.0),
    ]);
    for _ in 0..2 {
        run_ops(
            fx,
            &ctx,
            source(&ctx),
            W,
            H,
            &ops,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[LayerInput::Texture(source(&ctx)), LayerInput::Absent],
            &[],
            &[],
            &Side::NONE,
            &[],
            None,
            Some((&cache, 7)),
        );
    }
    assert_eq!(
        cache.borrow().counts(),
        (4, 0),
        "nothing after a matte is held"
    );
    assert_eq!(cache.borrow().stats().2, 0);

    // The matte on the *second* op: the first op's output is still named.
    let cache = warm_cache();
    for _ in 0..2 {
        run_ops(
            fx,
            &ctx,
            source(&ctx),
            W,
            H,
            &ops,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[LayerInput::Absent, LayerInput::Texture(source(&ctx))],
            &[],
            &[],
            &Side::NONE,
            &[],
            None,
            Some((&cache, 7)),
        );
    }
    assert_eq!(
        cache.borrow().counts(),
        (3, 1),
        "the chain holds up to the matte"
    );

    // Neighbour frames (Echo) break it the same way.
    let cache = warm_cache();
    let ops = stack(&[("echo", "decay", 0.5), ("exposure", "stops", 1.0)]);
    for _ in 0..2 {
        run_ops(
            fx,
            &ctx,
            source(&ctx),
            W,
            H,
            &ops,
            &[(-1, source(&ctx))],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &Side::NONE,
            &[],
            None,
            Some((&cache, 7)),
        );
    }
    assert_eq!(cache.borrow().counts(), (4, 0));
    assert_eq!(cache.borrow().stats().2, 0);
}

/// `TABLE_EFFECTS` is the predicate `build.rs` fills the colour-table list
/// by, and `AuxKind::Lut` is the one `run_ops` counts by. One list, so the
/// two must name the same effects.
#[test]
fn table_effects_are_the_lut_aux_declarers() {
    let mut declared: Vec<&str> = lumit_core::fx::catalogue::BUILTIN_DEFS
        .builtins()
        .map(|d| d.schema().match_name)
        .filter(|n| crate::gpufx::gpu_effect(n).is_some_and(|g| g.aux() == AuxKind::Lut))
        .collect();
    declared.sort_unstable();
    let mut listed = TABLE_EFFECTS.to_vec();
    listed.sort_unstable();
    assert_eq!(declared, listed);
}

// ----- The frame's texture pool -----

/// The chain, batched, with every side list empty and no cache.
fn chain(fx: &FxEngine, ctx: &GpuContext, ops: &lumit_core::fx::params::ResolvedStack) -> Vec<f32> {
    let out = run_ops(
        fx,
        ctx,
        source(ctx),
        W,
        H,
        ops,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &Side::NONE,
        &[],
        None,
        None,
    );
    lumit_gpu::fx::common::readback_linear_f32(ctx, &out, W, H).expect("readback")
}

/// Three effects on one layer make two work textures, not three: the third
/// op writes into the picture the second one read. The pixels do not move,
/// and a cache that is keeping outputs gets its own textures back.
///
/// A frame is one command buffer, so before this every effect in a stack
/// held a frame-sized texture until the frame ended.
#[test]
fn a_chain_writes_into_the_texture_the_op_before_it_finished_with() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let ops = stack(&[
        ("saturation", "saturation", 20.0),
        ("exposure", "stops", 1.0),
        ("exposure", "stops", -0.5),
    ]);

    // The pool lives inside a frame batch, so a walk outside one is the
    // same chain with nothing reused.
    let before = ctx.work_textures_made();
    let plain = chain(fx, &ctx, &ops);
    let alone = ctx.work_textures_made() - before;
    assert_eq!(
        alone, 4,
        "the source and one texture an op, with nothing to reuse"
    );

    ctx.begin_frame();
    let pooled = chain(fx, &ctx, &ops);
    let batched = ctx.work_textures_made() - before - alone;
    ctx.end_frame();
    assert_eq!(pooled, plain, "the same picture, whosever texture it is in");
    assert_eq!(
        batched, 3,
        "the source and two textures for three ops, the third writing into the first"
    );

    // A cache that is taking outputs reads them on a later frame, so none
    // of them may be written into.
    let cache = warm_cache();
    let before = ctx.work_textures_made();
    ctx.begin_frame();
    let kept = run(fx, &ctx, &ops, &cache, 11);
    let made = ctx.work_textures_made() - before;
    ctx.end_frame();
    assert_eq!(kept, plain, "and the cached walk draws the same picture");
    assert_eq!(made, alone, "a filed picture is never handed back");
    ctx.begin_frame();
    let again = run(fx, &ctx, &ops, &cache, 11);
    ctx.end_frame();
    assert_eq!(again, plain, "which the walk after it reads unharmed");
}
