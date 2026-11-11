//! The motion fields a layer's flow-reading effects are handed, made in the
//! decode worker beside the neighbour frames they are measured against.
//!
//! Flow motion blur (docs/08 §3.2, offset +1) and Datamosh (§3.12, offset -1)
//! each want one dense field, brought up to the frame's own size. Denoise
//! wants one for every frame it reads either side, at the size it was
//! measured at, and none at all for a frame that is no use to it.
//!
//! **One measurement per offset asked for**. Motion blur and Datamosh want
//! opposite directions, so a single shared field was never something both
//! could read; the layer used to carry one and the first effect in stack order
//! silently took it. They are separate entries in the flow cache, keyed by the
//! frame pair, so a stack with only one of them measures exactly once.
//!
//! **Each pair once, both ways.** Denoise reads the frame after as well as the
//! frame before, so the pair a frame measures going forward is the pair the
//! next frame wants going back. Its pairs are always measured earlier frame
//! first and turned round when asked the other way: two frames either side
//! costs two new pairs a frame, not four, and which was asked first never
//! changes the answer. Runs on the decode thread.

use lumit_core::fx::temporal::FlowWant;

use super::{
    flow_for, flow_settings_key, CachedFlow, CompJob, FlowFieldData, FlowKey, FramePixels,
};

/// Every field `job` asked for, by offset, measured from this frame (`px`) to
/// the neighbours already decoded (`temporal`). A neighbour that failed to
/// decode just leaves its offset absent, degrading the effect that wanted it
/// to a passthrough.
///
/// For Denoise (any want but [`FlowWant::Full`]) a neighbour that is the same
/// source frame as this one, as the frame a Blend or Flow retime mixes into
/// it, or as a nearer neighbour, gets no field either: a held end or a slowed
/// clip repeats frames, and the same noise twice is not two measurements of
/// it. Nor does a neighbour across an edit point, which never reaches
/// `temporal`.
///
/// Missing footage, drawn as bars, has no fields: bars are no frame of the
/// file, and filed as one they would be found again once it is back.
pub(super) fn neighbour_fields(
    flow_engine: &mut Option<lumit_flow::FlowEngine>,
    flow_cache: &mut lumit_cache::ByteLru<FlowKey, CachedFlow>,
    gpu: Option<&lumit_gpu::GpuContext>,
    job: &CompJob,
    px: &FramePixels,
    temporal: &[(i32, Vec<u8>)],
) -> Result<Vec<(i32, FlowFieldData)>, String> {
    if job.slate {
        return Ok(Vec::new());
    }
    // Nearest first, so a repeated frame is kept where it is nearest.
    let mut wants = job.flow_neighbours.clone();
    wants.sort_by_key(|&(o, _)| (o.abs(), o));
    let mut seen = vec![job.source_frame];
    seen.extend(job.blend.map(|(partner, _)| partner));
    let mut out: Vec<(i32, FlowFieldData)> = Vec::with_capacity(wants.len());
    for (offset, want) in wants {
        // A cold frame at Frames 5 measures ten pairs, long enough for the
        // Viewer to have moved on. Asked between pairs, so every pair filed
        // is whole and a frame is never drawn with fewer neighbours than it
        // asked for.
        if crate::cancel::requested() {
            return Err(crate::cancel::CANCELLED.to_owned());
        }
        let Some((_, other)) = temporal.iter().find(|(o, _)| *o == offset) else {
            continue;
        };
        let nb = job
            .temporal
            .iter()
            .find(|(o, _)| *o == offset)
            .map_or(job.source_frame, |(_, f)| *f);
        let repeated = seen.contains(&nb);
        seen.push(nb);
        let field = match want {
            FlowWant::Still if repeated => continue,
            FlowWant::Measured if repeated => continue,
            // No motion, fully trusted: the frame is usable where it is.
            FlowWant::Still => (vec![0.0], vec![0.0], vec![1.0], (1, 1)),
            FlowWant::Measured | FlowWant::Full => {
                measured(flow_engine, flow_cache, gpu, job, px, other, nb, want)
            }
        };
        out.push((offset, field));
    }
    out.sort_by_key(|(o, _)| *o);
    Ok(out)
}

/// The field from this frame to neighbour frame `nb` (`other`), with its
/// confidence, at the size `want` asks for.
#[allow(clippy::too_many_arguments)]
fn measured(
    flow_engine: &mut Option<lumit_flow::FlowEngine>,
    flow_cache: &mut lumit_cache::ByteLru<FlowKey, CachedFlow>,
    gpu: Option<&lumit_gpu::GpuContext>,
    job: &CompJob,
    px: &FramePixels,
    other: &[u8],
    nb: usize,
    want: FlowWant,
) -> FlowFieldData {
    let (w, h) = (px.width as usize, px.height as usize);
    // An effect asking for motion has no parameters of its own, and sharing
    // the retime policy's settings would make one layer's blur depend on the
    // other's retime.
    //
    // Half resolution, though, not the retime default of native. Retime
    // measures natively so that preview and export agree about the picture;
    // an effect has no such argument, and a smaller working size is *better*
    // here rather than merely cheaper. Between consecutive frames of a fast
    // camera move the displacement is large, and an 8×8 patch on a 1080p frame
    // is a tiny window on content that is often periodic — container ribs,
    // railings, brickwork — where the patch matches many positions equally
    // well and picks one. Halving doubles what each patch spans relative to
    // that repeat, which is the difference between disambiguating it and
    // guessing. It is also the working resolution docs/impl/optical-flow.md §1
    // names as the default, and a quarter of the cost.
    let set = lumit_flow::FlowSettings {
        divisor: 2,
        ..lumit_flow::FlowSettings::default()
    };
    // Read as the samples are: a float plate's bytes are not eight-bit
    // colour, and measured as if they were the motion is noise.
    let grays = |a: &[u8], b: &[u8]| {
        let (ga, gb, _) = if px.format == lumit_media::decode::PixelFormat::LinearF32 {
            lumit_flow::flow_grays_f32(a, b, w, h, &set)
        } else {
            lumit_flow::flow_grays(a, b, w, h, &set)
        };
        (ga, gb)
    };
    let (fwd, bwd) = if want == FlowWant::Full || job.source_frame <= nb {
        pair(
            flow_engine,
            flow_cache,
            gpu,
            job.item,
            job.source_frame,
            nb,
            &set,
            || grays(&px.rgba, other),
        )
    } else {
        let (later, earlier) = pair(
            flow_engine,
            flow_cache,
            gpu,
            job.item,
            nb,
            job.source_frame,
            &set,
            || grays(other, &px.rgba),
        );
        (earlier, later)
    };
    // The per-pixel confidence Motion blur tapers the streak by and Denoise
    // weighs a neighbour by (FX-19); the same deterministic function export
    // runs, so the two match. Datamosh ignores it.
    let conf = lumit_flow::confidence(&fwd, &bwd);
    match want {
        // Motion blur and Datamosh want a field at the frame's own size. The
        // vectors scale with the image — a 3 px move at half res is 6 px at
        // full — while the confidence is a 0..1 weight and must not be
        // touched by that scaling.
        FlowWant::Full => {
            let (u, v, c) = lumit_flow::field_to_size(&fwd, &conf, w, h);
            (u, v, c, (px.width, px.height))
        }
        // Denoise reads it at the size it was measured at, as Timewarp does,
        // which at 4K is a sixteenth of the memory.
        _ => {
            let size = (fwd.w as u32, fwd.h as u32);
            (fwd.u, fwd.v, conf, size)
        }
    }
}

/// The pair measured from frame `a` to frame `b` and back, from the cache
/// when it is there; the luma `grays` are only made when it is not.
#[allow(clippy::too_many_arguments)]
fn pair(
    flow_engine: &mut Option<lumit_flow::FlowEngine>,
    flow_cache: &mut lumit_cache::ByteLru<FlowKey, CachedFlow>,
    gpu: Option<&lumit_gpu::GpuContext>,
    item: uuid::Uuid,
    a: usize,
    b: usize,
    set: &lumit_flow::FlowSettings,
    grays: impl FnOnce() -> (lumit_flow::Gray, lumit_flow::Gray),
) -> (lumit_flow::FlowField, lumit_flow::FlowField) {
    if let Some(hit) = flow_cache.get(&(item, a, b, flow_settings_key(set), false)) {
        return (hit.fwd.clone(), hit.bwd.clone());
    }
    let (ga, gb) = grays();
    flow_for(
        flow_engine,
        flow_cache,
        gpu,
        item,
        a,
        b,
        &ga,
        &gb,
        set,
        false,
    )
}
