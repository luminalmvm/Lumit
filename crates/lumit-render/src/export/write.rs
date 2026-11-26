//! The frame loop every picture export runs, and the files it writes into.
//! Runs on the export's thread, with the writing on a thread of its own.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use lumit_core::model::{Composition, Document};
use lumit_gpu::device::Fault;
use lumit_media::encode::BitDepth;
use uuid::Uuid;

use super::{captions, frame, held, ExportEvent, ExportSpec};
use crate::mix::mixdown::audio_samples_through;

/// One file an export writes, open and ready for its frames.
pub(super) struct Out {
    pub(super) spec: ExportSpec,
    pub(super) path: PathBuf,
    pub(super) subtitles: captions::Embedded,
    pub(super) sink: Sink,
    pack: frame::Pack,
}

impl Out {
    pub(super) fn new(
        spec: ExportSpec,
        path: PathBuf,
        subtitles: captions::Embedded,
        sink: Sink,
        comp: &Composition,
    ) -> Self {
        let pack = frame::Pack::new(&spec, comp, sink.size());
        Self {
            spec,
            path,
            subtitles,
            sink,
            pack,
        }
    }
}

/// The renderer an export draws with, set up to write files.
pub(super) fn renderer(
    doc: &Arc<Document>,
    spec: &ExportSpec,
) -> Result<crate::headless::HeadlessRenderer, String> {
    // The export renders through the SAME walk the Viewer does — the headless
    // preview at full decode quality (preview == export by
    // construction, gated by the bit-identity matrix in `headless::tests`).
    // Its own renderer on its own device, so an export never contends with the
    // Viewer's GPU work.
    let mut renderer =
        crate::headless::HeadlessRenderer::new().map_err(|e| format!("export renderer: {e}"))?;
    // A layer whose model engine will not paint stops the run here rather than
    // being drawn with the built-in engine and written to the file under the
    // model's name. The pre-flight turns away everything the document alone can
    // show; this is the rest of it (docs/08 §3.1, docs/impl/addons.md §6.3).
    renderer.refuse_substitution();
    renderer.hand_over_prefetched();
    // The project's colour config, before anything is written: the refusal has
    // to happen with the config in hand, and it has to happen before a file
    // exists rather than halfway through one.
    renderer.sync_colour(doc);
    spec.check_with_colour(renderer.colour())?;
    // A config's space is delivered by binding its baked table to the SAME
    // display blit the Viewer presents through (docs/impl/ocio.md §5.2). Not a
    // second transform at the pack stage: that would be a second implementation
    // of one transform in the delivery path, which is the exact structure the
    // preview-equals-export promise forbids.
    renderer.set_colour_output(spec.colour_space.ocio_name().map(str::to_owned));
    Ok(renderer)
}

/// Render every frame of `span` once and write it to every file in `outs`,
/// with the sound and captions that go beside it. The files share the first
/// one's depth, rate and quality.
#[allow(clippy::too_many_arguments)]
pub(super) fn frames(
    renderer: &mut crate::headless::HeadlessRenderer,
    open: &dyn Fn() -> Result<crate::headless::HeadlessRenderer, String>,
    doc: &Arc<Document>,
    comp_id: Uuid,
    span: &frame::Span,
    outs: &mut [Out],
    audio_mix: Option<&[f32]>,
    tx: &Sender<ExportEvent>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let Some(first) = outs.first() else {
        return Ok(());
    };
    let (out_fps, total) = (span.out_fps, span.total);
    let (depth, quality) = (first.spec.depth, first.spec.render.quality);
    let rate = first.spec.audio_rate;
    let chans = usize::from(first.spec.audio_layout.channels());
    let (packs, writers): (Vec<_>, Vec<_>) = outs
        .iter_mut()
        .map(|out| {
            let sound = frame::carries_sound(&out.spec);
            (&out.pack, (&mut out.sink, &mut out.subtitles, sound))
        })
        .unzip();

    // The files are written on a thread of their own, so one frame is encoded
    // while the next is rendered. Only one finished frame waits between the
    // two, so this holds a frame or two more than a loop that took turns.
    let (rendered, written) = std::thread::scope(|s| {
        let (frames, queued) = std::sync::mpsc::sync_channel::<Vec<Vec<u8>>>(1);
        let mut writers = writers;
        let writer = std::thread::Builder::new()
            .name("lumit-export-writer".into())
            .spawn_scoped(s, move || -> Result<usize, String> {
                let mut audio_fed = 0usize;
                for (frame_n, packed) in queued.iter().enumerate() {
                    // Interleave: after each picture frame, the samples that cover it,
                    // so the muxer keeps sound and picture together in the file.
                    let upto = audio_mix.map(|mix| {
                        audio_samples_through(frame_n + 1, out_fps, rate).min(mix.len() / chans)
                    });
                    for ((sink, subtitles, sound), rgba) in writers.iter_mut().zip(&packed) {
                        sink.write_rgba(rgba)?;
                        if let (Some(mix), Some(upto), true, Sink::Video { encoder, .. }) =
                            (audio_mix, upto, *sound, &mut **sink)
                        {
                            if upto > audio_fed {
                                encoder
                                    .write_audio(&mix[audio_fed * chans..upto * chans])
                                    .map_err(|e| e.to_string())?;
                            }
                        }
                        subtitles.feed(sink, (frame_n + 1) as f64 / out_fps)?;
                    }
                    audio_fed = audio_fed.max(upto.unwrap_or(0));
                    let _ = tx.send(ExportEvent::Progress {
                        frame: frame_n + 1,
                        total,
                    });
                }
                Ok(audio_fed)
            })
            .map_err(|e| format!("export writer: {e}"))?;
        // Crop in composition pixels first, then letterbox into each file's
        // frame when the size was changed, then pack to what each file carries.
        // The two arms differ only in how wide a channel is: a sixteen-bit
        // export reads the composite back at sixteen bits and stays there, so
        // the extra width is the pipeline's own rather than a stretched byte.
        let render = |renderer: &mut crate::headless::HeadlessRenderer,
                      src: usize|
         -> Result<Rendered, String> {
            Ok(match depth {
                // Only started here: the bytes are collected by `finish`
                // below once the next frame is on its way.
                BitDepth::Eight => {
                    Rendered::Coming(renderer.start_preview(doc, comp_id, src as u64, quality)?)
                }
                BitDepth::Sixteen => Rendered::Bytes({
                    let (px, _, _) =
                        renderer.render_preview16(doc, comp_id, src as u64, quality)?;
                    pack_all(&packs, px, |p, px| p.frame(px))
                }),
                // The float depths read the composite back as it stands: no
                // display transform, no curve, no clamp (docs/06 §7.4a). The
                // channel and alpha choices still apply, as they are about what
                // the file carries, not about how bright it is.
                BitDepth::Half | BitDepth::Float => Rendered::Bytes({
                    let (px, _, _) =
                        renderer.render_preview_linear(doc, comp_id, src as u64, quality)?;
                    pack_all(&packs, px, |p, px| p.float(px))
                }),
            })
        };
        let finish = |renderer: &mut crate::headless::HeadlessRenderer,
                      frame: Rendered|
         -> Result<Vec<Vec<u8>>, String> {
            let pending = match frame {
                Rendered::Bytes(packed) => return Ok(packed),
                Rendered::Coming(pending) => pending,
            };
            Ok(pack_all(
                &packs,
                renderer.finish_preview(pending)?,
                |p, px| p.frame(px),
            ))
        };
        // A frame the card could not draw comes back black or half drawn, so
        // each is watched, and one that faulted is drawn again once the card
        // has finished with everything before it, on a new device if it was
        // lost. One that still can't be drawn fails the export: a file is
        // never written with a frame missing from it.
        let guarded = |renderer: &mut crate::headless::HeadlessRenderer,
                       src: usize|
         -> Result<Result<Rendered, String>, String> {
            renderer.watch_faults();
            let over_before = renderer.frames_over_budget();
            let frame = render(renderer, src);
            // A frame bigger than the whole budget is still drawn, but if the
            // card then had no room, that is the reason given.
            let over = (renderer.frames_over_budget() != over_before)
                .then(|| renderer.over_budget())
                .flatten();
            match renderer.faults() {
                Some(Fault::Starved(why)) => Ok(Err(over.map_or(why, |o| o.to_string()))),
                // Reported, and the frame kept as drawn, as it always was.
                Some(Fault::Refused(why)) => {
                    let _ = writeln!(std::io::stderr(), "export: frame {src}: {why}");
                    frame.map(Ok)
                }
                None => frame.map(Ok),
            }
        };
        let redo = |renderer: &mut crate::headless::HeadlessRenderer,
                    src: usize,
                    mut why: String|
         -> Result<Vec<Vec<u8>>, String> {
            for _ in 0..FRAME_TRIES {
                let _ = writeln!(
                    std::io::stderr(),
                    "export: drawing frame {src} again ({why})"
                );
                if renderer.device_lost() {
                    *renderer = open()?;
                } else {
                    renderer.clear_frame_textures();
                    renderer.drop_attachments();
                    renderer.settle_gpu();
                }
                why = match guarded(renderer, src)? {
                    Err(why) => why,
                    Ok(frame) => match finish(renderer, frame) {
                        Ok(_) | Err(_) if renderer.device_lost() => {
                            "the graphics device was lost".into()
                        }
                        done => return done,
                    },
                };
            }
            Err(format!("frame {src} could not be drawn: {why}"))
        };
        // A frame read back from a device lost meanwhile is drawn again.
        let deliver = |renderer: &mut crate::headless::HeadlessRenderer,
                       (src, frame): (usize, Rendered)|
         -> Result<Vec<Vec<u8>>, String> {
            match finish(renderer, frame) {
                Ok(_) | Err(_) if renderer.device_lost() => {
                    redo(renderer, src, "the graphics device was lost".into())
                }
                done => done,
            }
        };
        let src_of = |n: usize| span.source(n);
        // The footage is decoded a couple of frames ahead, on the threads
        // playback reads with, so one frame decodes while another renders.
        // Only while that is safe: a few files a frame, frames small enough
        // that the ones in flight fit in half the decode cache, and footage
        // that turns up in time. Past any of those it stops for good, and
        // each frame decodes its own footage as it always did.
        const DECODE_AHEAD: usize = 2;
        const MOST_WANTS: usize = 8;
        const LONGEST_WAIT: std::time::Duration = std::time::Duration::from_secs(2);
        let mut ahead = Some(crate::prefetch::Prefetcher::default());
        // What was asked for each frame from this one on, in order.
        let mut asked = std::collections::VecDeque::new();
        let mut asked_to = 0usize;
        let mut rendered = Ok(());
        let mut waiting: Option<(usize, Rendered)> = None;
        // Once a frame has faulted, the card is let finish each frame before
        // the next is drawn. Slower, and the same pictures.
        let mut careful = false;
        'frames: for frame_n in 0..total {
            held::wait(cancel);
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let src = src_of(frame_n);
            let mut stop = false;
            if let Some(prefetch) = ahead.as_mut() {
                // Only this frame's own at first, until its size is known.
                let reach = if frame_n == 0 { 0 } else { DECODE_AHEAD };
                while asked_to <= frame_n + reach && asked_to < total {
                    // A layer of a layered file is left to the render: the
                    // decode thread reads each one once and then skips it.
                    let wants: Vec<_> = renderer
                        .prefetch_wants(doc, comp_id, src_of(asked_to) as u64, quality)
                        .into_iter()
                        .filter(|w| !lumit_media::reads_own(&w.source))
                        .collect();
                    if wants.len() > MOST_WANTS {
                        stop = true;
                        break;
                    }
                    asked.push_back(
                        wants
                            .iter()
                            .map(|w| (w.item, w.frame, w.target_width))
                            .collect::<Vec<_>>(),
                    );
                    prefetch.request(wants);
                    asked_to += 1;
                }
            }
            let owed: Vec<_> = if frame_n < asked_to {
                asked.pop_front().unwrap_or_default()
            } else {
                Vec::new()
            };
            if let Some(prefetch) = ahead.as_ref() {
                let since = web_time::Instant::now();
                let mut bytes = 0usize;
                let mut landed = prefetch.drain();
                loop {
                    for done in landed.drain(..) {
                        bytes = bytes.saturating_add(done.decoded.rgba.len());
                        renderer.preload_decoded(done);
                    }
                    if owed
                        .iter()
                        .all(|&(item, frame, width)| renderer.holds_decoded(item, frame, width))
                        || cancel.load(Ordering::Relaxed)
                    {
                        break;
                    }
                    // A decode that failed or was dropped sends nothing, so
                    // the wait is short, and a wait that runs out stops
                    // decoding ahead. The render below decodes what is missing.
                    if since.elapsed() > LONGEST_WAIT {
                        stop = true;
                        break;
                    }
                    landed.extend(prefetch.wait(std::time::Duration::from_millis(50)));
                }
                let in_flight = bytes.saturating_mul(DECODE_AHEAD + 1);
                stop |= in_flight > crate::decode::DEFAULT_DECODE_CACHE_BYTES / 2;
            }
            if stop {
                // Its threads end with it.
                ahead = None;
            }
            // Footage a frame still to come asked for too is copied for this
            // one, not handed over, or that frame would wait for it in vain.
            for &(item, frame, width) in &owed {
                if asked
                    .iter()
                    .flatten()
                    .any(|&later| later == (item, frame, width))
                {
                    renderer.keep_decoded(item, frame, width);
                }
            }
            let frame = match guarded(&mut *renderer, src) {
                Ok(Ok(frame)) => frame,
                Ok(Err(why)) => {
                    careful = true;
                    // The frame before is read back first, from the device it
                    // was drawn on, as the redo may replace it.
                    if let Some(before) = waiting.take() {
                        match deliver(&mut *renderer, before) {
                            Ok(packed) => {
                                if frames.send(packed).is_err() {
                                    break;
                                }
                            }
                            Err(e) => {
                                rendered = Err(e);
                                break;
                            }
                        }
                    }
                    match redo(&mut *renderer, src, why) {
                        Ok(packed) => Rendered::Bytes(packed),
                        Err(e) => {
                            rendered = Err(e);
                            break;
                        }
                    }
                }
                Err(e) => {
                    rendered = Err(e);
                    break;
                }
            };
            if careful {
                renderer.settle_gpu();
            }
            // An eight-bit frame waits a turn, so its read-back runs while
            // the next one renders. Any other goes at once, after the one
            // still waiting.
            let ready: Vec<(usize, Rendered)> = match frame {
                Rendered::Coming(_) => waiting.replace((src, frame)).into_iter().collect(),
                Rendered::Bytes(_) => waiting.take().into_iter().chain([(src, frame)]).collect(),
            };
            for before in ready {
                let packed = match deliver(&mut *renderer, before) {
                    Ok(packed) => packed,
                    Err(e) => {
                        rendered = Err(e);
                        break 'frames;
                    }
                };
                // Refused only when the writer has stopped, and it says why below.
                if frames.send(packed).is_err() {
                    break 'frames;
                }
            }
        }
        // And the last, when nothing stopped the loop.
        if let Some(last) = waiting.filter(|_| rendered.is_ok() && !cancel.load(Ordering::Relaxed))
        {
            match deliver(&mut *renderer, last) {
                Ok(packed) => {
                    let _ = frames.send(packed);
                }
                Err(e) => rendered = Err(e),
            }
        }
        // Joined here rather than by the scope, so a writer that panicked
        // is an error like any other.
        drop(frames);
        let written = writer
            .join()
            .unwrap_or_else(|_| Err("the export's writer stopped".into()));
        Ok::<_, String>((rendered, written))
    })?;
    rendered?;
    // A run that stops here leaves the tidying to the caller, which removes
    // what every file began.
    if cancel.load(Ordering::Relaxed) {
        return Ok(());
    }
    let audio_fed = written?;
    for out in outs.iter_mut() {
        // Any samples the per-frame rounding left behind.
        if let (Some(mix), true, Sink::Video { encoder, .. }) =
            (audio_mix, frame::carries_sound(&out.spec), &mut out.sink)
        {
            if mix.len() / chans > audio_fed {
                encoder
                    .write_audio(&mix[audio_fed * chans..])
                    .map_err(|e| e.to_string())?;
            }
        }
        out.subtitles.feed(&mut out.sink, f64::INFINITY)?;
        out.sink.finish()?;
    }
    Ok(())
}

/// How many more times a frame the card could not draw is tried before the
/// export fails.
const FRAME_TRIES: usize = 2;

/// One frame packed for every file: copied for each but the last, which
/// takes it, so a single file costs no copy.
fn pack_all<T: Clone>(
    packs: &[&frame::Pack],
    px: Vec<T>,
    pack: impl Fn(&frame::Pack, Vec<T>) -> Vec<u8>,
) -> Vec<Vec<u8>> {
    let Some((last, rest)) = packs.split_last() else {
        return Vec::new();
    };
    let mut packed: Vec<Vec<u8>> = rest.iter().map(|p| pack(p, px.clone())).collect();
    packed.push(pack(last, px));
    packed
}

/// A frame on its way to the files: their bytes, or at eight bits its
/// read-back still coming off the graphics card.
enum Rendered {
    Bytes(Vec<Vec<u8>>),
    Coming(lumit_gpu::PendingReadback),
}

/// Where the rendered frames go: the mp4 muxer, a GIF, or the numbered stills. One
/// type so the frame loop stays single.
pub(super) enum Sink {
    Video {
        encoder: lumit_media::encode::Encoder,
        size: (u32, u32),
    },
    Images {
        encoder: lumit_media::encode::ImageSequenceEncoder,
        size: (u32, u32),
        /// Frames written so far — exactly the files a cancel removes.
        written: usize,
    },
    Gif {
        encoder: lumit_media::encode::gif::GifEncoder,
        size: (u32, u32),
    },
    Webp {
        encoder: lumit_media::encode::webp::WebpEncoder,
        size: (u32, u32),
    },
}

impl Sink {
    fn size(&self) -> (u32, u32) {
        match self {
            Sink::Video { size, .. }
            | Sink::Images { size, .. }
            | Sink::Gif { size, .. }
            | Sink::Webp { size, .. } => *size,
        }
    }

    fn write_rgba(&mut self, rgba: &[u8]) -> Result<(), String> {
        match self {
            Sink::Video { encoder, .. } => encoder.write_rgba(rgba).map_err(|e| e.to_string()),
            Sink::Gif { encoder, .. } => encoder.write_rgba(rgba).map_err(|e| e.to_string()),
            Sink::Webp { encoder, .. } => encoder.write_rgba(rgba).map_err(|e| e.to_string()),
            Sink::Images {
                encoder, written, ..
            } => {
                encoder.write_rgba(rgba).map_err(|e| e.to_string())?;
                *written += 1;
                Ok(())
            }
        }
    }

    fn finish(&mut self) -> Result<(), String> {
        match self {
            Sink::Video { encoder, .. } => encoder.finish().map_err(|e| e.to_string()),
            Sink::Images { encoder, .. } => encoder.finish().map_err(|e| e.to_string()),
            Sink::Gif { encoder, .. } => encoder.finish().map_err(|e| e.to_string()),
            Sink::Webp { encoder, .. } => encoder.finish().map_err(|e| e.to_string()),
        }
    }

    /// Every file this wrote: the chosen path for one file, or each still of
    /// a sequence.
    pub(super) fn files(&self, chosen_path: &std::path::Path) -> Vec<PathBuf> {
        match self {
            Sink::Images { written, .. } => (1..=*written)
                .map(|n| lumit_media::encode::sequence_frame_path(chosen_path, n))
                .collect(),
            _ => vec![chosen_path.to_path_buf()],
        }
    }

    /// Remove what a cancelled or failed image export left behind. The mp4
    /// path needs nothing here — its half file is removed by the caller, which
    /// cannot know a sequence's file names; this does.
    pub(super) fn remove_written(&self, chosen_path: &std::path::Path) {
        if let Sink::Images { written, .. } = self {
            for n in 1..=*written {
                let _ =
                    std::fs::remove_file(lumit_media::encode::sequence_frame_path(chosen_path, n));
            }
        }
    }
}
