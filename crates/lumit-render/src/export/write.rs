//! The frame loop every picture export runs, and the files it writes into.
//! Runs on the export's thread, with the writing on a thread of its own.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use lumit_core::model::{Composition, Document};
use lumit_media::encode::BitDepth;
use uuid::Uuid;

use super::{captions, frame, held, letterbox_resize_f32, pack_frame_f32, ExportEvent, ExportSpec};
use crate::mix::mixdown::audio_samples_through;

/// Render every frame of `span` and write it to `sink`, with the sound and
/// captions that go beside it.
#[allow(clippy::too_many_arguments)]
pub(super) fn frames(
    renderer: &mut crate::headless::HeadlessRenderer,
    doc: &Arc<Document>,
    comp_id: Uuid,
    comp: &Composition,
    spec: &ExportSpec,
    span: &frame::Span,
    sink: &mut Sink,
    subtitles: &mut captions::Embedded,
    audio_mix: Option<&[f32]>,
    out_path: &Path,
    tx: &Sender<ExportEvent>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let (out_fps, total) = (span.out_fps, span.total);
    let rate = spec.audio_rate;
    let chans = usize::from(spec.audio_layout.channels());
    let (crop_w, crop_h) = spec.crop.output_size(comp.width, comp.height);
    let resize = sink.size() != (crop_w, crop_h);
    let pack = frame::Pack::new(spec, comp, sink.size());

    let (tw, th) = sink.size();
    // The file is written on a thread of its own, so one frame is encoded
    // while the next is rendered. Only one finished frame waits between the
    // two, so this holds a frame or two more than a loop that took turns.
    let (rendered, written) = std::thread::scope(|s| {
        let (frames, queued) = std::sync::mpsc::sync_channel::<Vec<u8>>(1);
        let (sink, subtitles) = (&mut *sink, &mut *subtitles);
        let writer = std::thread::Builder::new()
            .name("lumit-export-writer".into())
            .spawn_scoped(s, move || -> Result<usize, String> {
                let mut audio_fed = 0usize;
                for (frame_n, rgba) in queued.iter().enumerate() {
                    sink.write_rgba(&rgba)?;
                    // Interleave: after each picture frame, the samples that cover it,
                    // so the muxer keeps sound and picture together in the file.
                    if let (Some(mix), Sink::Video { encoder, .. }) = (audio_mix, &mut *sink) {
                        let upto = audio_samples_through(frame_n + 1, out_fps, rate)
                            .min(mix.len() / chans);
                        if upto > audio_fed {
                            encoder
                                .write_audio(&mix[audio_fed * chans..upto * chans])
                                .map_err(|e| e.to_string())?;
                            audio_fed = upto;
                        }
                    }
                    subtitles.feed(sink, (frame_n + 1) as f64 / out_fps)?;
                    let _ = tx.send(ExportEvent::Progress {
                        frame: frame_n + 1,
                        total,
                    });
                }
                Ok(audio_fed)
            })
            .map_err(|e| format!("export writer: {e}"))?;
        // Crop in composition pixels first, then letterbox into the delivery
        // frame when the size was changed, then pack to what the file carries.
        // The two arms differ only in how wide a channel is: a sixteen-bit
        // export reads the composite back at sixteen bits and stays there, so
        // the extra width is the pipeline's own rather than a stretched byte.
        let render = |renderer: &mut crate::headless::HeadlessRenderer,
                      src: usize|
         -> Result<Rendered, String> {
            Ok(match spec.depth {
                // Only started here: the bytes are collected by `finish`
                // below once the next frame is on its way.
                BitDepth::Eight => Rendered::Coming(renderer.start_preview(
                    doc,
                    comp_id,
                    src as u64,
                    spec.render.quality,
                )?),
                BitDepth::Sixteen => Rendered::Bytes({
                    let (px, _, _) =
                        renderer.render_preview16(doc, comp_id, src as u64, spec.render.quality)?;
                    pack.frame(px)
                }),
                // The float depths read the composite back as it stands: no
                // display transform, no curve, no clamp (docs/06 §7.4a). The
                // channel and alpha choices still apply — they are about what the
                // file carries, not about how bright it is — and the resize runs
                // on the floats so a delivered EXR is filtered rather than
                // point-sampled.
                BitDepth::Half | BitDepth::Float => Rendered::Bytes({
                    let (px, _, _) = renderer.render_preview_linear(
                        doc,
                        comp_id,
                        src as u64,
                        spec.render.quality,
                    )?;
                    let px = spec.crop.apply(px, comp.width, comp.height, 4);
                    let px = if resize {
                        letterbox_resize_f32(&px, crop_w, crop_h, tw, th)
                    } else {
                        px
                    };
                    pack_frame_f32(&px, spec.channels, spec.alpha)
                }),
            })
        };
        let finish = |renderer: &mut crate::headless::HeadlessRenderer,
                      frame: Rendered|
         -> Result<Vec<u8>, String> {
            let pending = match frame {
                Rendered::Bytes(rgba) => return Ok(rgba),
                Rendered::Coming(pending) => pending,
            };
            Ok(pack.frame(renderer.finish_preview(pending)?))
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
        let quality = spec.render.quality;
        let mut ahead = Some(crate::prefetch::Prefetcher::default());
        // What was asked for each frame from this one on, in order.
        let mut asked = std::collections::VecDeque::new();
        let mut asked_to = 0usize;
        let mut rendered = Ok(());
        let mut waiting = None;
        for frame_n in 0..total {
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
            let frame = match render(&mut *renderer, src) {
                Ok(frame) => frame,
                Err(e) => {
                    rendered = Err(e);
                    break;
                }
            };
            // An eight-bit frame waits a turn, so its read-back runs while
            // the next one renders. Any other goes at once.
            let ready = match frame {
                Rendered::Coming(_) => waiting.replace(frame),
                Rendered::Bytes(_) => Some(frame),
            };
            let Some(before) = ready else {
                continue;
            };
            let rgba = match finish(&mut *renderer, before) {
                Ok(rgba) => rgba,
                Err(e) => {
                    rendered = Err(e);
                    break;
                }
            };
            // Refused only when the writer has stopped, and it says why below.
            if frames.send(rgba).is_err() {
                break;
            }
        }
        // And the last, when nothing stopped the loop.
        if let Some(last) = waiting.filter(|_| rendered.is_ok() && !cancel.load(Ordering::Relaxed))
        {
            match finish(&mut *renderer, last) {
                Ok(rgba) => {
                    let _ = frames.send(rgba);
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
    if cancel.load(Ordering::Relaxed) {
        sink.remove_written(out_path);
        return Ok(());
    }
    let audio_fed = match written {
        Ok(fed) => fed,
        Err(e) => {
            // A folder of stills that failed half-way is tidied rather than
            // left as a trap that looks like a finished export.
            sink.remove_written(out_path);
            return Err(e);
        }
    };
    // Any samples the per-frame rounding left behind.
    if let (Some(mix), Sink::Video { encoder, .. }) = (audio_mix, &mut *sink) {
        if mix.len() / chans > audio_fed {
            encoder
                .write_audio(&mix[audio_fed * chans..])
                .map_err(|e| e.to_string())?;
        }
    }
    subtitles.feed(sink, f64::INFINITY)?;
    sink.finish()
}

/// A frame on its way to the file: its bytes, or at eight bits its
/// read-back still coming off the graphics card.
enum Rendered {
    Bytes(Vec<u8>),
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

    /// Remove what a cancelled or failed image export left behind. The mp4
    /// path needs nothing here — its half file is removed by the caller, which
    /// cannot know a sequence's file names; this does.
    fn remove_written(&self, chosen_path: &std::path::Path) {
        if let Sink::Images { written, .. } = self {
            for n in 1..=*written {
                let _ =
                    std::fs::remove_file(lumit_media::encode::sequence_frame_path(chosen_path, n));
            }
        }
    }
}
