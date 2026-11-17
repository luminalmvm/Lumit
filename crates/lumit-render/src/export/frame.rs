//! What every frame of one export shares, whoever writes the file: the
//! composition frames it covers, the comp's sound cut to them, and how a
//! finished picture becomes the bytes the file carries. The desktop's writer
//! (`run`) and the browser build's (`web`) both come through here, so the two
//! cannot make different files.

use lumit_audio::plan::MixPlan;
use lumit_core::model::{Composition, Document};
use lumit_core::pixels::{Channel, Resample};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::{
    frame_span, letterbox_resize_f32, pack_frame, pack_frame_f32, AlphaMode, AudioLayout, Channels,
    ColourTransform, Crop, ExportSpec,
};
use crate::mix::graph::{build_graph, GraphInfo};
use crate::mix::jobs::AudioJob;
use crate::mix::live::RackBakes;
use crate::mix::mixdown::audio_samples_through;
use crate::mix::render::{aim, render};

/// The stretch of the composition an export writes, and the rate it writes it at.
pub(super) struct Span {
    /// The composition's own rate.
    pub fps: f64,
    /// The file's rate.
    pub out_fps: f64,
    pub first: usize,
    pub end: usize,
    /// How many frames the file holds, at `out_fps`.
    pub total: usize,
}

impl Span {
    pub(super) fn of(comp: &Composition, spec: &ExportSpec) -> Self {
        let fps = comp.frame_rate.fps().max(1.0);
        let (first, end) = frame_span(comp, spec);
        // The output rate. A rate other than the comp's resamples by nearest comp
        // frame over the same wall-clock span, so a 60 fps comp exported at 30
        // shows every other frame and lasts exactly as long.
        let out_fps = spec.fps.unwrap_or(fps).clamp(1.0, 1000.0);
        let out_fps = spec
            .format
            .caps()
            .max_fps
            .map_or(out_fps, |max| out_fps.min(f64::from(max)));
        let span_seconds = (end - first) as f64 / fps;
        let total = ((span_seconds * out_fps).round() as usize).max(1);
        Self {
            fps,
            out_fps,
            first,
            end,
            total,
        }
    }

    /// The comp frame under output frame `n`: exact when the rates match
    /// (the rounding is then of an integer), nearest otherwise.
    pub(super) fn source(&self, n: usize) -> usize {
        let src = self.first + ((n as f64) * self.fps / self.out_fps).round() as usize;
        src.min(self.end.saturating_sub(1))
    }
}

/// The comp's audio, mixed exactly as playback mixes it, then cut to the
/// export range and padded so sound and picture end together. `None` when the
/// file carries no sound, or the export was stopped. `on_progress` is handed
/// how far through the mixing runs it is, 0 to 1.
pub(super) fn sound(
    spec: &ExportSpec,
    doc: &Arc<Document>,
    comp: &Composition,
    jobs: &[AudioJob],
    span: &Span,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(f64),
) -> Option<Vec<f32>> {
    // Sound only joins a container that can hold it: a folder of stills has
    // nowhere to put it, and the dialogue says so rather than silently
    // dropping it. An audio-only export is nothing *but* sound, so the
    // include-audio tick has no say there.
    //
    // A silent comp exported as sound still writes silence of the right
    // length — an empty .wav would look like a failure that wasn't one — but
    // a video export of a silent comp carries no audio stream at all rather
    // than a mute one.
    if !carries_sound(spec) || (jobs.is_empty() && spec.format.caps().video) {
        return None;
    }
    let rate = spec.audio_rate;
    let (mut plan, _) = desk(spec, doc, comp, jobs, cancel)?;
    let full = match spec.loudness {
        Some(target) => {
            aim(
                &mut plan,
                rate,
                target.lufs,
                target.true_peak_db,
                cancel,
                &mut |f| on_progress(f * 0.5),
            )?;
            render(Arc::new(plan), rate, true, &mut |f| {
                on_progress(0.5 + f * 0.5)
            })
        }
        None => render(Arc::new(plan), rate, true, on_progress),
    };
    Some(cut(spec, span, &full))
}

/// Whether the file carries sound at all.
pub(super) fn carries_sound(spec: &ExportSpec) -> bool {
    let caps = spec.format.caps();
    caps.audio && (spec.include_audio || !caps.video)
}

/// The comp's desk as an export plays it: built offline, files read through
/// the pool. `None` when the export was stopped.
pub(super) fn desk(
    spec: &ExportSpec,
    doc: &Arc<Document>,
    comp: &Composition,
    jobs: &[AudioJob],
    cancel: &AtomicBool,
) -> Option<(MixPlan, GraphInfo)> {
    let rate = spec.audio_rate;
    let pool = lumit_audio::stream::pool();
    build_graph(
        doc,
        comp,
        jobs.to_vec(),
        &|job| Some(pool.source(&job.sound, rate)),
        rate,
        comp.duration.0.to_f64(),
        &mut RackBakes::default(),
        None,
        true,
        cancel,
        None,
    )
    .ok()
}

/// A whole mix cut to the export range and padded so sound and picture end
/// together, folded to the spec's layout.
pub(super) fn cut(spec: &ExportSpec, span: &Span, full: &[f32]) -> Vec<f32> {
    let rate = spec.audio_rate;
    // The cut starts where the range starts on the comp's own clock, and
    // covers the output's duration — total frames at the *output* rate —
    // so sound and picture end together whatever rate was chosen.
    let start = audio_samples_through(span.first, span.fps, rate).min(full.len() / 2);
    let expect = audio_samples_through(span.total, span.out_fps, rate);
    let mut cut = full[start * 2..(start + expect).min(full.len() / 2) * 2].to_vec();
    cut.resize(expect * 2, 0.0);
    // The mix is stereo throughout — playback's own mix, so preview and
    // export cannot disagree — and folds down once, at the very end,
    // where nothing else reads it.
    match spec.audio_layout {
        AudioLayout::Stereo => cut,
        AudioLayout::Mono => lumit_audio::mix::downmix_to_mono(&cut),
    }
}

/// The frame the file is delivered at, before any rule of the codec's.
///
/// The crop happens in composition pixels, so the picture that leaves the
/// compositor is cropped first and sized afterwards. When the delivery size
/// *is* the comp's own — every Custom export — the cropped size becomes the
/// file's size, which is what cropping is for; a preset that asked for a
/// different frame letterboxes the cropped picture into it, exactly as an
/// uncropped one does.
pub(super) fn delivered(spec: &ExportSpec, comp: &Composition) -> (u32, u32) {
    match spec.target {
        Some(t) if t != (comp.width, comp.height) => t,
        _ => spec.crop.output_size(comp.width, comp.height),
    }
}

/// How a finished picture becomes what the file carries: cropped in
/// composition pixels, letterboxed into the delivery frame when the size was
/// changed, then packed.
pub(super) struct Pack {
    crop: Crop,
    comp: (u32, u32),
    cropped: (u32, u32),
    size: (u32, u32),
    resample: Resample,
    channels: Channels,
    alpha: AlphaMode,
    colour: Option<ColourTransform>,
}

impl Pack {
    /// For a file whose frames are `size`.
    pub(super) fn new(spec: &ExportSpec, comp: &Composition, size: (u32, u32)) -> Self {
        Self {
            crop: spec.crop,
            comp: (comp.width, comp.height),
            cropped: spec.crop.output_size(comp.width, comp.height),
            size,
            resample: spec.resample,
            channels: spec.channels,
            // A GIF pixel is either clear or solid, so the edge pixels it keeps
            // are written at their own colour, not darkened by their coverage.
            alpha: if spec.format.caps().alpha_mode {
                spec.alpha
            } else {
                AlphaMode::Straight
            },
            // Worked out once: the primaries matrix and the destination curve
            // are the same for every frame, and deriving them per pixel would
            // be the same answer two million times over.
            colour: spec.colour_space.transform(),
        }
    }

    /// The composite at eight or sixteen bits a channel, as the file's bytes.
    pub(super) fn frame<C: Channel>(&self, px: Vec<C>) -> Vec<u8> {
        let px = self.crop.apply(px, self.comp.0, self.comp.1, 4);
        let px = if self.size == self.cropped {
            px
        } else {
            lumit_core::pixels::letterbox_resize(
                &px,
                self.cropped.0,
                self.cropped.1,
                self.size.0,
                self.size.1,
                self.resample,
            )
        };
        pack_frame(&px, self.channels, self.alpha, self.colour.as_ref())
    }

    /// The composite in floats, as the file's bytes. The resize runs on the
    /// floats, so a delivered EXR is filtered rather than point-sampled.
    pub(super) fn float(&self, px: Vec<f32>) -> Vec<u8> {
        let px = self.crop.apply(px, self.comp.0, self.comp.1, 4);
        let px = if self.size == self.cropped {
            px
        } else {
            letterbox_resize_f32(
                &px,
                self.cropped.0,
                self.cropped.1,
                self.size.0,
                self.size.1,
            )
        };
        pack_frame_f32(&px, self.channels, self.alpha)
    }
}
