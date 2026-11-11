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
//! **Denoise measures neighbours and chains them.** Its motion to the frame
//! two along is its motion to the next frame, then that frame's motion on
//! (`lumit_flow::chain`), so only frames next to each other are ever measured.
//! Each pair is measured earlier frame first and kept with the confidence of
//! both directions, since the pair a frame measures going forward is the pair
//! the next frame reads going back. Playing forward, that is one new pair a
//! frame however many frames either side it reads, and which frame was asked
//! for first never changes the answer. Runs on the decode thread.

use std::sync::Arc;

use lumit_core::fx::temporal::FlowWant;
use lumit_flow::chain::Leg;

use super::flow_store::{FlowCache, HeldGray, HeldLink, Link, Read};
use super::{flow_for, flow_settings_key, CompJob, FlowFieldData, FramePixels};

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
/// file, and filed as one they would be found again once it is back. Nor is
/// anything filed when a frame was read in place of one still on its way
/// (`stood_in`, the count of those before this frame was read): it is measured
/// as it is and kept by nobody.
#[allow(clippy::too_many_arguments)] // one worker call; bundling would hide it
pub(super) fn neighbour_fields(
    flow_engine: &mut Option<lumit_flow::FlowEngine>,
    flow_cache: &mut FlowCache,
    gpu: Option<&lumit_gpu::GpuContext>,
    job: &CompJob,
    px: &FramePixels,
    temporal: &[(i32, Arc<Vec<u8>>)],
    stood_in: u64,
) -> Result<Vec<(i32, FlowFieldData)>, String> {
    if job.slate {
        return Ok(Vec::new());
    }
    let file = lumit_media::stand_ins() == stood_in;
    // Nearest first, so a repeated frame is kept where it is nearest and each
    // side's chain grows outwards one frame at a time.
    let mut wants = job.flow_neighbours.clone();
    wants.sort_by_key(|&(o, _)| (o.abs(), o));
    let chained = wants
        .iter()
        .filter(|(_, w)| *w == FlowWant::Measured)
        .count();
    if chained > 0 {
        // Each frame either side is one pair live, and the pair the next frame
        // measures one more. A pair is two legs of four floats a pixel.
        let (gw, gh) = settings().working_size(px.width as usize, px.height as usize);
        flow_cache.want_links(chained + 2, gw * gh * 32 + 64, gw * gh * 4 + 32);
    }
    let mut seen = vec![job.source_frame];
    seen.extend(job.blend.map(|(partner, _)| partner));
    // Each side's chain: the frame it has reached, at which offset, and the
    // motion from this frame to it.
    let mut ahead: Option<(usize, i32, Arc<Leg>)> = None;
    let mut behind: Option<(usize, i32, Arc<Leg>)> = None;
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
            FlowWant::Still => Arc::new(Leg::from_planes(&[0.0], &[0.0], &[1.0], 1, 1)),
            FlowWant::Full => full(flow_engine, flow_cache, gpu, job, px, other, nb),
            FlowWant::Measured => {
                let side = if offset > 0 { &mut ahead } else { &mut behind };
                let (from, from_px) = match side.as_ref() {
                    Some((f, o, _)) => (*f, temporal.iter().find(|(t, _)| t == o)),
                    None => (job.source_frame, None),
                };
                let from_px = from_px.map_or(px.rgba.as_slice(), |(_, p)| p.as_slice());
                let link = link(
                    (flow_engine, flow_cache, gpu),
                    job,
                    px,
                    (from, from_px),
                    (nb, other),
                    file,
                );
                let step = if from < nb { &link.on } else { &link.back };
                let leg = match side.take() {
                    Some((_, _, so_far)) => Arc::new(so_far.then(step)),
                    None => Arc::clone(step),
                };
                let field = Arc::clone(&leg);
                *side = Some((nb, offset, leg));
                field
            }
        };
        out.push((offset, field));
    }
    out.sort_by_key(|(o, _)| *o);
    Ok(out)
}

/// What an effect measures at: half resolution, not the retime default of
/// native, for the reasons [`full`] gives.
fn settings() -> lumit_flow::FlowSettings {
    lumit_flow::FlowSettings {
        divisor: 2,
        ..lumit_flow::FlowSettings::default()
    }
}

/// The pair between frames `a` and `b` of this job's source, each with its
/// pixels, from the cache when it is there and `file` lets the cache be used.
/// A frame's luma is made once and kept for the next pair that starts from it.
/// Both are filed by how each frame was read as well as by its number, as the
/// decoded frames are: the channels the layer extracts, and the stamp of a
/// frame in a run of stills, which may have been saved over.
fn link(
    (flow_engine, flow_cache, gpu): (
        &mut Option<lumit_flow::FlowEngine>,
        &mut FlowCache,
        Option<&lumit_gpu::GpuContext>,
    ),
    job: &CompJob,
    px: &FramePixels,
    a: (usize, &[u8]),
    b: (usize, &[u8]),
    file: bool,
) -> Arc<Link> {
    let (early, late) = if a.0 <= b.0 { (a, b) } else { (b, a) };
    let size = (px.width as usize, px.height as usize);
    let set = settings();
    let setting = flow_settings_key(&set);
    let read = |frame: usize| -> (usize, Read) {
        (frame, (job.channels.clone(), job.source.frame_stamp(frame)))
    };
    let key = (job.item, read(early.0), read(late.0), setting, size);
    if let Some(HeldLink(hit)) = flow_cache.links.get(&key).filter(|_| file) {
        return Arc::clone(hit);
    }
    let floats = px.format == lumit_media::decode::PixelFormat::LinearF32;
    let mut gray = |(frame, rgba): (usize, &[u8])| {
        let key = (job.item, read(frame), setting, size, floats);
        if let Some(HeldGray(g)) = flow_cache.grays.get(&key).filter(|_| file) {
            return Arc::clone(g);
        }
        // Read as the samples are: a float plate's bytes are not eight-bit
        // colour, and measured as if they were the motion is noise.
        let g = Arc::new(lumit_flow::chain::flow_gray(
            rgba, size.0, size.1, floats, &set,
        ));
        if file {
            flow_cache.grays.insert(key, HeldGray(Arc::clone(&g)));
        }
        g
    };
    let (ga, gb) = (gray(early), gray(late));
    let (on, back) = flow_engine
        .get_or_insert_with(|| super::flow_engine_for(gpu))
        .flow_pair_with(&ga, &gb, &set);
    let link = Arc::new(Link {
        on: Arc::new(Leg::measured(&on, &back)),
        back: Arc::new(Leg::measured(&back, &on)),
    });
    if file {
        flow_cache.links.insert(key, HeldLink(Arc::clone(&link)));
    }
    link
}

/// The field from this frame to neighbour frame `nb` (`other`), with its
/// confidence, at the frame's own size: what Motion blur and Datamosh read.
fn full(
    flow_engine: &mut Option<lumit_flow::FlowEngine>,
    flow_cache: &mut FlowCache,
    gpu: Option<&lumit_gpu::GpuContext>,
    job: &CompJob,
    px: &FramePixels,
    other: &[u8],
    nb: usize,
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
    // is a tiny window on content that is often periodic (container ribs,
    // railings, brickwork), where the patch matches many positions equally
    // well and picks one. Halving doubles what each patch spans relative to
    // that repeat, which is the difference between disambiguating it and
    // guessing. It is also the working resolution docs/impl/optical-flow.md §1
    // names as the default, and a quarter of the cost.
    let set = settings();
    let key = (
        job.item,
        job.source_frame,
        nb,
        flow_settings_key(&set),
        false,
    );
    let (fwd, bwd) = match flow_cache.get(&key) {
        Some(hit) => (hit.fwd.clone(), hit.bwd.clone()),
        None => {
            // Read as the samples are, as Denoise's pairs are.
            let (ga, gb, _) = if px.format == lumit_media::decode::PixelFormat::LinearF32 {
                lumit_flow::flow_grays_f32(&px.rgba, other, w, h, &set)
            } else {
                lumit_flow::flow_grays(&px.rgba, other, w, h, &set)
            };
            flow_for(
                flow_engine,
                flow_cache,
                gpu,
                job.item,
                job.source_frame,
                nb,
                &ga,
                &gb,
                &set,
                false,
            )
        }
    };
    // The per-pixel confidence Motion blur tapers the streak by (FX-19); the
    // same deterministic function export runs, so the two match. Datamosh
    // ignores it. The vectors scale with the image (a 3 px move at half res
    // is 6 px at full), while the confidence is a 0..1 weight and must not be
    // touched by that scaling.
    let conf = lumit_flow::confidence(&fwd, &bwd);
    let (u, v, c) = lumit_flow::field_to_size(&fwd, &conf, w, h);
    Arc::new(Leg::from_planes(&u, &v, &c, w, h))
}
