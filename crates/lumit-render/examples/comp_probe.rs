//! Temporary by-hand probe: where does a real project's measured frame go?
//! Opens a .lum, renders frames of named comps with profiling on, and prints
//! the profile total, the per-layer sum, and wall-clock per render — the
//! instrument for "the readout says 106 ms but the rows sum to 9".
//! Run with:
//!   cargo run --release -p lumit-render --example comp_probe -- <project.lum> <CompName> [CompName...]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::model::ProjectItem;
use lumit_render::{HeadlessRenderer, Quality};
use std::sync::{Arc, Mutex};
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = std::path::PathBuf::from(args.next().expect("project path"));
    let wanted: Vec<String> = args.collect();

    let t = Instant::now();
    let (mut doc, _manifest) = lumit_project::open(&path).expect("open project");
    println!("open: {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
    let dir = path.parent().expect("project dir").to_path_buf();
    let t = Instant::now();
    let (_relinked, missing) = lumit_project::resolve_all_media(&mut doc, &dir, &[]);
    println!(
        "resolve_all_media: {:.1} ms, missing: {:?}",
        t.elapsed().as_secs_f64() * 1000.0,
        missing
    );
    let doc = Arc::new(doc);

    let mut r = HeadlessRenderer::new().expect("renderer");
    r.watch_frames(true);
    r.measure_frames(true);

    // Timestamp every progress report so the stage boundaries have times.
    let stages: Arc<Mutex<Vec<(u32, f32, f64)>>> = Arc::new(Mutex::new(Vec::new()));
    let t0 = Instant::now();
    {
        let stages = Arc::clone(&stages);
        r.set_progress_sink(Some(Arc::new(move |p| {
            stages.lock().unwrap().push((
                p.stage.code(),
                p.fraction,
                t0.elapsed().as_secs_f64() * 1000.0,
            ));
        })));
    }
    let profiles: Arc<Mutex<Vec<lumit_render::FrameProfile>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let profiles = Arc::clone(&profiles);
        r.set_profile_sink(Some(Arc::new(move |p| {
            profiles.lock().unwrap().push(p);
        })));
    }

    // Where does the draw-list build spend its time? Every comp, timed alone.
    if std::env::var("PROBE_ALL_COMPS").is_ok() {
        let pixels = std::collections::HashMap::new();
        let mut rows: Vec<(f64, String, usize)> = Vec::new();
        for it in &doc.items {
            let ProjectItem::Composition(c) = it else {
                continue;
            };
            let t_mid = c.duration.0.to_f64() * 0.5;
            let t = Instant::now();
            let mut visited = vec![c.id];
            std::hint::black_box(lumit_render::build_comp_draws_at(
                &doc,
                c,
                t_mid,
                t_mid,
                &pixels,
                &mut visited,
                None,
                false,
            ));
            rows.push((
                t.elapsed().as_secs_f64() * 1000.0,
                c.name.clone(),
                c.layers.len(),
            ));
        }
        rows.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        for (ms, name, layers) in rows.iter().take(12) {
            println!("build alone: {ms:8.2} ms  {layers:3} layers  {name}");
        }
        // The hottest comp, bisected by mutation: what part of its layers
        // carries the cost?
        if let Some((_, hot_name, _)) = rows.first() {
            let hot = doc
                .items
                .iter()
                .find_map(|it| match it {
                    ProjectItem::Composition(c) if &c.name == hot_name => Some(c.clone()),
                    _ => None,
                })
                .expect("hot comp");
            for l in &hot.layers {
                println!(
                    "  layer '{}': kind {:?}, {} masks, {} effects [{}], {} styles, retime {}",
                    l.name,
                    std::mem::discriminant(&l.kind),
                    l.masks.len(),
                    l.effects.len(),
                    l.effects
                        .iter()
                        .map(|e| e.effect.match_name.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    l.styles.len(),
                    l.retime.is_some(),
                );
            }
            // Each variant at a time nothing has built yet: a repeat of an
            // already-built time answers from some memo and says nothing.
            let mut fresh = 0.0_f64;
            let mut time_variant = |label: &str, c: &lumit_core::model::Composition| {
                fresh += 0.037;
                let t_at = c.duration.0.to_f64() * 0.25 + fresh;
                let t = Instant::now();
                let mut visited = vec![c.id];
                std::hint::black_box(lumit_render::build_comp_draws_at(
                    &doc,
                    c,
                    t_at,
                    t_at,
                    &pixels,
                    &mut visited,
                    None,
                    false,
                ));
                println!(
                    "  variant {label}: {:.2} ms",
                    t.elapsed().as_secs_f64() * 1000.0
                );
            };
            time_variant("as-is", &hot);
            time_variant("as-is again, fresh time", &hot);
            let mut no_fx = hot.clone();
            for l in &mut no_fx.layers {
                l.effects.clear();
            }
            time_variant("effects stripped", &no_fx);
            let mut no_masks = hot.clone();
            for l in &mut no_masks.layers {
                l.masks.clear();
            }
            time_variant("masks stripped", &no_masks);
            let mut no_retime = hot.clone();
            for l in &mut no_retime.layers {
                l.retime = None;
            }
            time_variant("retime stripped", &no_retime);
            let mut flat = hot.clone();
            for l in &mut flat.layers {
                l.effects.clear();
                l.masks.clear();
                l.retime = None;
                l.styles.clear();
            }
            time_variant("all stripped", &flat);
        }
        // Clips, one layer at a time — which layers carry the every-call cost?
        if let Some(clips) = doc.items.iter().find_map(|it| match it {
            ProjectItem::Composition(c) if c.name == "Clips" => Some(c),
            _ => None,
        }) {
            let mut fresh = 0.0;
            let mut rows: Vec<(f64, String)> = Vec::new();
            for l in &clips.layers {
                let mut alone = clips.clone();
                alone.layers = vec![l.clone()];
                fresh += 0.013;
                let t_at = clips.duration.0.to_f64() * 0.5 + fresh;
                let t = Instant::now();
                let mut visited = vec![alone.id];
                std::hint::black_box(lumit_render::build_comp_draws_at(
                    &doc,
                    &alone,
                    t_at,
                    t_at,
                    &pixels,
                    &mut visited,
                    None,
                    false,
                ));
                rows.push((t.elapsed().as_secs_f64() * 1000.0, l.name.clone()));
            }
            rows.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
            for (ms, name) in rows.iter().take(10) {
                println!("clips layer alone: {ms:8.2} ms  {name}");
            }
            // The hot layer bisected: which of its parts re-pays per call?
            if let Some(hot_l) = clips
                .layers
                .iter()
                .filter(|l| l.name == rows[0].1)
                .max_by_key(|l| l.effects.len())
            {
                let mut fresh = 0.5_f64;
                let mut probe = |label: &str, l: &lumit_core::model::Layer| {
                    let mut alone = clips.clone();
                    alone.layers = vec![l.clone()];
                    fresh += 0.017;
                    let t_at = clips.duration.0.to_f64() * 0.5 + fresh;
                    for round in 0..2 {
                        let t = Instant::now();
                        let mut visited = vec![alone.id];
                        std::hint::black_box(lumit_render::build_comp_draws_at(
                            &doc,
                            &alone,
                            t_at,
                            t_at,
                            &pixels,
                            &mut visited,
                            None,
                            false,
                        ));
                        println!(
                            "  hot layer {label} round {round}: {:.2} ms",
                            t.elapsed().as_secs_f64() * 1000.0
                        );
                    }
                };
                probe("as-is", hot_l);
                let mut no_fx = hot_l.clone();
                no_fx.effects.clear();
                probe("no effects", &no_fx);
                let mut one_fx = hot_l.clone();
                one_fx
                    .effects
                    .retain(|e| e.effect.match_name.starts_with("S_"));
                probe("only S_*", &one_fx);
                let mut native_fx = hot_l.clone();
                native_fx
                    .effects
                    .retain(|e| !e.effect.match_name.starts_with("S_"));
                probe("without S_*", &native_fx);
                let mut no_masks = hot_l.clone();
                no_masks.masks.clear();
                probe("no masks", &no_masks);
            }
        }
    }

    struct Stub;
    impl lumit_eval::SourceStamper for Stub {
        fn stamp(&self, item: uuid::Uuid, _lt: f64, _native: bool) -> Option<(String, u64)> {
            Some((item.to_string(), 0))
        }
    }

    for name in &wanted {
        let comp = doc.items.iter().find_map(|it| match it {
            ProjectItem::Composition(c) if &c.name == name => Some(c),
            _ => None,
        });
        let Some(comp) = comp else {
            println!("comp not found: {name}");
            continue;
        };
        // --- CPU-side costs, no GPU involved. ---
        {
            let t_mid = comp.duration.0.to_f64() * 0.5;
            let n = 200;
            let t = Instant::now();
            for i in 0..n {
                std::hint::black_box(comp.camera_pose(t_mid + i as f64 * 1e-7));
            }
            let per_pose = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
            let n = 20;
            let t = Instant::now();
            for i in 0..n {
                std::hint::black_box(lumit_eval::comp_frame_key(
                    &doc,
                    comp,
                    t_mid + i as f64 * 1e-7,
                    lumit_eval::Quality::default(),
                    &Stub,
                ));
            }
            let per_key = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
            let pixels = std::collections::HashMap::new();
            let t = Instant::now();
            for i in 0..n {
                let mut visited = vec![comp.id];
                std::hint::black_box(lumit_render::build_comp_draws_at(
                    &doc,
                    comp,
                    t_mid + i as f64 * 1e-7,
                    t_mid,
                    &pixels,
                    &mut visited,
                    None,
                    false,
                ));
            }
            let per_build = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
            println!(
                "\n[{name}] CPU: camera_pose {per_pose:.3} ms | comp_frame_key {per_key:.2} ms | build_comp_draws_at (no pixels) {per_build:.2} ms"
            );
            // The camera curve itself, if the comp has one.
            if let Some(cam) = comp
                .layers
                .iter()
                .find(|l| matches!(l.kind, lumit_core::model::LayerKind::Camera { .. }))
            {
                if let lumit_core::anim::Animation::Keyframed(keys) =
                    &cam.transform.position_x.animation
                {
                    let n = 100_000;
                    let t = Instant::now();
                    for i in 0..n {
                        std::hint::black_box(lumit_core::anim::evaluate(
                            keys,
                            t_mid + (i % 100) as f64 * 0.01,
                        ));
                    }
                    let ns = t.elapsed().as_secs_f64() * 1e9 / f64::from(n);
                    println!(
                        "[{name}] camera position_x: {} keys, evaluate() {ns:.0} ns/sample",
                        keys.len()
                    );
                }
            }
        }
        let fps = comp.frame_rate.num() as f64 / comp.frame_rate.den() as f64;
        let frames = (comp.duration.0.to_f64() * fps) as u64;
        println!(
            "\n=== {name} ({}x{}, {:.0} fps, {} frames, {} layers) ===",
            comp.width,
            comp.height,
            fps,
            frames,
            comp.layers.len()
        );
        // A handful of frames spread through the comp; the second render of the
        // same frame says what the caches change.
        let picks = [frames / 4, frames / 2, frames / 2, 3 * frames / 4];
        for (i, &frame) in picks.iter().enumerate() {
            stages.lock().unwrap().clear();
            let t = Instant::now();
            let out = r.render_preview(&doc, comp.id, frame, Quality::default(), 1.0);
            let wall = t.elapsed().as_secs_f64() * 1000.0;
            match out {
                Ok((_rgba, w, h)) => {
                    let profile = profiles.lock().unwrap().pop();
                    let (total, layer_sum, layer_lines) = match &profile {
                        Some(p) => (
                            p.total_ms,
                            p.layers.iter().map(|l| l.ms).sum::<f32>(),
                            p.layers
                                .iter()
                                .map(|l| format!("{:.1}", l.ms))
                                .collect::<Vec<_>>()
                                .join(" "),
                        ),
                        None => (0.0, 0.0, String::from("(no profile)")),
                    };
                    println!(
                        "frame {frame} ({}x{}) pick#{i}: wall {wall:.1} ms | profile total {total:.1} ms | layers sum {layer_sum:.1} ms [{layer_lines}]",
                        w, h
                    );
                    // Stage transitions from the progress reports.
                    let stages = stages.lock().unwrap();
                    if !stages.is_empty() {
                        let base = stages.first().unwrap().2;
                        let names = ["plan", "decode", "build", "composite", "present"];
                        let mut last_stage = u32::MAX;
                        let mut line = String::new();
                        for &(code, _f, at) in stages.iter() {
                            if code != last_stage {
                                line.push_str(&format!(
                                    " {}@{:.1}ms",
                                    names.get(code as usize).unwrap_or(&"?"),
                                    at - base
                                ));
                                last_stage = code;
                            }
                        }
                        println!(
                            "  stages:{line} end@{:.1}ms",
                            stages.last().unwrap().2 - base
                        );
                    }
                }
                Err(e) => println!("frame {frame}: ERROR {e}"),
            }
        }
    }
}
