//! Export (docs/06-RENDER-PIPELINE.md §7): render every work-area frame
//! through the compositor at full resolution and encode to H.264/mp4.
//!
//! In plain terms: the same pixels the Viewer shows, written to a file — the
//! preview-equals-export promise (K-031) holds because this path reuses the
//! identical colour engine and compositor. Precomp layers render recursively:
//! the nested comp becomes a texture the parent composites like any other
//! source. Runs on its own thread with its own decoders (K-017); progress
//! streams back; cancel is checked every frame.

#![cfg(feature = "media")]

pub use crate::pixels::{px_tile, solid_rgba, srgb_decode, srgb_encode};
use lumit_core::model::{Composition, Document, LayerKind, MatteChannel, ProjectItem};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use uuid::Uuid;

type Tex = egui_wgpu::wgpu::Texture;

pub enum ExportEvent {
    /// Which encoder the ladder settled on ("NVENC", "software x264", …),
    /// sent once the file is open.
    Encoder(&'static str),
    Progress {
        frame: usize,
        total: usize,
    },
    Done(PathBuf),
    Failed(String),
}

pub struct ExportHandle {
    pub events: Receiver<ExportEvent>,
    cancel: Arc<AtomicBool>,
}

impl ExportHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Everything the export thread needs about one footage item.
#[derive(Clone)]
pub struct ItemInfo {
    pub path: PathBuf,
    pub fps: f64,
    pub frames: usize,
}

/// One audio-bearing layer, as the export thread needs it: where its file
/// is, its comp-timeline span, and its start offset (the same trio the
/// preview mix uses, so export audio matches playback).
#[derive(Clone)]
pub struct AudioJob {
    pub path: PathBuf,
    pub in_s: f64,
    pub out_s: f64,
    pub offset_s: f64,
}

/// Delivery presets (docs/06-RENDER-PIPELINE.md §7.5): frame, codec, and
/// bitrates as data, not code. Custom keeps the comp's own size and the
/// dialogue's choices.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ExportPreset {
    Custom,
    Youtube1080p60,
    Youtube4k60,
    Vertical1080p60,
}

/// The parameter row one preset stamps into the export dialogue.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PresetParams {
    pub size: (u32, u32),
    pub codec: lumit_media::encode::VideoCodec,
    /// VBR average target, bits/second.
    pub target_bps: i64,
    /// VBR peak, bits/second.
    pub peak_bps: i64,
}

/// Audio on all delivery presets: AAC 320 kbps, 48 kHz (docs/06 §7.5).
pub const PRESET_AUDIO_BPS: i64 = 320_000;
/// Export audio sample rate (docs/06 §7.5: 48 kHz on delivery presets).
pub const EXPORT_AUDIO_RATE: u32 = 48_000;

impl ExportPreset {
    pub const ALL: [ExportPreset; 4] = [
        ExportPreset::Custom,
        ExportPreset::Youtube1080p60,
        ExportPreset::Youtube4k60,
        ExportPreset::Vertical1080p60,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ExportPreset::Custom => "Custom (comp size)",
            ExportPreset::Youtube1080p60 => "YouTube 1080p60",
            ExportPreset::Youtube4k60 => "YouTube 4K60",
            ExportPreset::Vertical1080p60 => "Vertical 1080×1920p60",
        }
    }

    /// The parameters this preset stamps; None for Custom (the dialogue's
    /// own fields apply).
    pub fn params(self) -> Option<PresetParams> {
        use lumit_media::encode::VideoCodec;
        match self {
            ExportPreset::Custom => None,
            // H.264 high, VBR 16 target / 24 peak (docs/06 §7.5).
            ExportPreset::Youtube1080p60 => Some(PresetParams {
                size: (1920, 1080),
                codec: VideoCodec::H264,
                target_bps: 16_000_000,
                peak_bps: 24_000_000,
            }),
            // HEVC (the ladder falls back to x265 when no hardware offers
            // it), VBR 45 target / 60 peak — YouTube's 2160p60 band.
            ExportPreset::Youtube4k60 => Some(PresetParams {
                size: (3840, 2160),
                codec: VideoCodec::Hevc,
                target_bps: 45_000_000,
                peak_bps: 60_000_000,
            }),
            // The vertical variant of the 1080p60 preset (docs/06 §7.5).
            ExportPreset::Vertical1080p60 => Some(PresetParams {
                size: (1080, 1920),
                codec: VideoCodec::H264,
                target_bps: 16_000_000,
                peak_bps: 24_000_000,
            }),
        }
    }

    /// Suggested file name for the save dialogue.
    pub fn default_file_name(self) -> &'static str {
        match self {
            ExportPreset::Custom => "export.mp4",
            ExportPreset::Youtube1080p60 => "youtube-1080p60.mp4",
            ExportPreset::Youtube4k60 => "youtube-4k60.mp4",
            ExportPreset::Vertical1080p60 => "vertical-1080x1920.mp4",
        }
    }
}

/// Everything one queued export needs beyond the document snapshot: the
/// resolved output size, codec, rates, and whether audio joins.
#[derive(Clone)]
pub struct ExportSpec {
    pub codec: lumit_media::encode::VideoCodec,
    pub target: (u32, u32),
    /// Average video bitrate in bits/second; None = encoder default quality.
    pub bit_rate: Option<i64>,
    /// VBR peak in bits/second.
    pub max_rate: Option<i64>,
    pub include_audio: bool,
    pub audio_bit_rate: i64,
}

/// One export waiting its turn. The document and audio jobs are snapshotted
/// at queue time (docs/06 §7.1): later edits never alter a queued item.
pub struct QueuedExport {
    pub doc: Arc<Document>,
    pub comp_id: Uuid,
    pub items: HashMap<Uuid, ItemInfo>,
    pub audio: Vec<AudioJob>,
    pub out_path: PathBuf,
    pub spec: ExportSpec,
}

pub fn start(
    doc: Arc<Document>,
    comp_id: Uuid,
    items: HashMap<Uuid, ItemInfo>,
    audio: Vec<AudioJob>,
    gpu: lumit_gpu::GpuContext,
    out_path: PathBuf,
    spec: ExportSpec,
) -> ExportHandle {
    let (tx, events) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    std::thread::spawn(move || {
        let result = run(
            &doc, comp_id, &items, &audio, &gpu, &out_path, &spec, &tx, &flag,
        );
        let _ = match result {
            Ok(()) if flag.load(Ordering::Relaxed) => {
                let _ = std::fs::remove_file(&out_path); // no half files
                tx.send(ExportEvent::Failed("cancelled".into()))
            }
            Ok(()) => tx.send(ExportEvent::Done(out_path)),
            Err(e) => {
                let _ = std::fs::remove_file(&out_path);
                tx.send(ExportEvent::Failed(e))
            }
        };
    });
    ExportHandle { events, cancel }
}

/// Decode every audio job (resampled to `rate`), lay each on the comp strip
/// at its offset and trim, and sum — the one mixdown all comp audio flows
/// through: preview playback, beat detection, and export, so they cannot
/// disagree about what the comp sounds like.
pub fn mixdown(jobs: &[AudioJob], rate: u32, duration_s: f64) -> Vec<f32> {
    let decoded: Vec<(lumit_media::AudioBuffer, &AudioJob)> = jobs
        .iter()
        .filter_map(|job| {
            lumit_media::audio::decode_all(&job.path, rate)
                .ok()
                .map(|buf| (buf, job))
        })
        .collect();
    let total_frames = (duration_s * f64::from(rate)).round().max(0.0) as usize;
    let placements: Vec<lumit_audio::mix::PlacedAudio> = decoded
        .iter()
        .filter_map(|(buf, job)| {
            let (start_frame, src_start, len) = lumit_audio::mix::place_on_timeline(
                job.in_s,
                job.out_s,
                job.offset_s,
                buf.samples.len() / 2,
                rate,
            )?;
            Some(lumit_audio::mix::PlacedAudio {
                start_frame,
                samples: &buf.samples[src_start * 2..(src_start + len) * 2],
                gain: 1.0,
            })
        })
        .collect();
    lumit_audio::mix::mix_stereo(&placements, total_frames)
}

/// How many audio samples (per channel) belong before the end of video
/// frame `frame_count` — the A/V interleaving rule. Cumulative rounding, so
/// the running total never drifts from `frames / fps × rate`.
pub fn audio_samples_through(frame_count: usize, fps: f64, rate: u32) -> usize {
    if fps <= 0.0 {
        return 0;
    }
    ((frame_count as f64 / fps) * f64::from(rate)).round() as usize
}

/// Renderer state carried down the precomp recursion.
struct Renderer<'a> {
    doc: &'a Document,
    items: &'a HashMap<Uuid, ItemInfo>,
    gpu: &'a lumit_gpu::GpuContext,
    colour: lumit_gpu::ColourEngine,
    compositor: lumit_gpu::Compositor,
    decoders: HashMap<Uuid, lumit_media::VideoDecoder>,
    /// Flow interpolation backend, sharing the export device; falls back to
    /// the CPU oracle by itself (export MUST honour the Flow policy, K-019).
    flow: lumit_flow::FlowEngine,
    /// Effect kernels, sharing the export device (docs/08; the same passes
    /// the preview runs, so effects export pixel-identically).
    fx: lumit_gpu::fx::FxEngine,
}

/// A layer's source, prepared for compositing: a linear texture plus the
/// natural pixel size its transform applies to.
struct Prepared {
    tex: Tex,
    natural: (f32, f32),
    /// Layer-space mask coverage texture (Precomp layers — GPU-sourced
    /// pixels get their masks as a texture, docs/06 render order).
    mask: Option<Tex>,
}

/// One inner layer of a collapsed Precomp, owned so its texture outlives the
/// borrow-taking CompositeLayer pass (docs/06 §1.4 splice, export side).
struct CollapsedSpec {
    p: Prepared,
    position: (f32, f32),
    anchor: (f32, f32),
    scale: (f32, f32),
    rotation_deg: f32,
    opacity: f32,
    z: f32,
    rotation_x_deg: f32,
    rotation_y_deg: f32,
    three_d: bool,
    blend: lumit_gpu::Blend,
    pre: [[f32; 4]; 4],
}

impl Renderer<'_> {
    /// Decode one footage item at `source_time` (seconds), apply the layer's
    /// masks, and upload — shared by Footage layers and Sequence footage clips.
    // contains_key + insert (not the entry API): the value is built fallibly
    // (index + decoder open both return Result), which entry() can't express.
    #[allow(clippy::map_entry)]
    #[allow(clippy::too_many_arguments)]
    fn prepare_footage(
        &mut self,
        item: Uuid,
        source_time: f64,
        pair: bool,
        flow: bool,
        sample_fps: Option<f64>,
        masks: &[lumit_core::mask::Mask],
    ) -> Result<Option<Prepared>, String> {
        let Some(info) = self.items.get(&item) else {
            return Ok(None);
        };
        // Same frame-pick + interpolation the preview uses, so export matches
        // (K-031). `pair` = fetch the neighbour; `flow` = flow-synthesise it;
        // `sample_fps` = the K-095 conform rate.
        let (source_frame, blend_frame) =
            crate::pixels::frame_pick(source_time, info.fps, info.frames, pair, sample_fps);
        if !self.decoders.contains_key(&item) {
            let index =
                lumit_media::index::build_frame_index(&info.path).map_err(|e| e.to_string())?;
            let dec =
                lumit_media::VideoDecoder::open(&info.path, index).map_err(|e| e.to_string())?;
            self.decoders.insert(item, dec);
        }
        let dec = self.decoders.get_mut(&item).ok_or("decoder missing")?;
        let mut px = dec
            .frame_rgba(source_frame, None)
            .map_err(|e| e.to_string())?;
        if let Some((ceil, w)) = blend_frame {
            let px2 = dec.frame_rgba(ceil, None).map_err(|e| e.to_string())?;
            px.rgba = if flow {
                self.flow.interpolate(
                    &px.rgba,
                    &px2.rgba,
                    px.width as usize,
                    px.height as usize,
                    w,
                )
            } else {
                crate::pixels::blend_rgba(&px.rgba, &px2.rgba, w)
            };
        }
        lumit_core::mask::apply_masks(
            &mut px.rgba,
            px.width,
            px.height,
            f64::from(px.width),
            f64::from(px.height),
            masks,
        );
        let src = self
            .colour
            .upload_srgb8(self.gpu, &px.rgba, px.width, px.height);
        Ok(Some(Prepared {
            tex: self.colour.linearise(self.gpu, &src),
            natural: (px.width as f32, px.height as f32),
            mask: None,
        }))
    }

    /// Prepare one layer's source at comp time `t` (None = contributes
    /// nothing); `visited` guards precomp cycles.
    fn prepare(
        &mut self,
        layer: &lumit_core::model::Layer,
        t: f64,
        visited: &mut Vec<Uuid>,
    ) -> Result<Option<Prepared>, String> {
        if t < layer.in_point.0.to_f64() || t >= layer.out_point.0.to_f64() {
            return Ok(None);
        }
        let lt = t - layer.start_offset.0.to_f64();
        match &layer.kind {
            // An adjustment layer has no source of its own; with no effect
            // stack yet it contributes nothing to the export.
            LayerKind::Adjustment => Ok(None),
            LayerKind::Footage { item, retime } => {
                // Retime maps local → source time; preview uses the same
                // Retime::evaluate + frame-pick, so export matches preview (K-031).
                let source_time = retime.as_ref().map(|r| r.evaluate(lt)).unwrap_or(lt);
                use lumit_core::retime::Interpolation;
                let interp = retime.as_ref().map(|r| &r.interpolation);
                let pair = matches!(interp, Some(Interpolation::Blend | Interpolation::Flow(_)));
                let flow = matches!(interp, Some(Interpolation::Flow(_)));
                let sample_fps = match interp {
                    Some(Interpolation::Flow(p)) => p.input_fps,
                    _ => None,
                };
                self.prepare_footage(*item, source_time, pair, flow, sample_fps, &layer.masks)
            }
            LayerKind::Sequence { clips } => {
                // The clip under the playhead, decoded like footage; a comp
                // clip or a gap contributes nothing (comp clips join later).
                match lumit_core::sequence::resolve(clips, lt) {
                    Some((_id, lumit_core::sequence::ClipSource::Footage(item), st)) => {
                        use lumit_core::retime::Interpolation;
                        let interp = lumit_core::sequence::active_clip(clips, lt)
                            .map(|c| c.interpolation.clone());
                        let pair =
                            matches!(interp, Some(Interpolation::Blend | Interpolation::Flow(_)));
                        let flow = matches!(interp, Some(Interpolation::Flow(_)));
                        let sample_fps = match &interp {
                            Some(Interpolation::Flow(p)) => p.input_fps,
                            _ => None,
                        };
                        self.prepare_footage(item, st, pair, flow, sample_fps, &layer.masks)
                    }
                    _ => Ok(None),
                }
            }
            LayerKind::Solid { def } => {
                let Some(sd) = self.doc.solid(*def) else {
                    return Ok(None); // deleted def degrades to nothing, never an error
                };
                let px = solid_rgba(sd.colour);
                // Masked solids rasterise at their own size; plain ones tile.
                let (w, h) = if layer.masks.is_empty() {
                    (16, 16)
                } else {
                    (sd.width, sd.height)
                };
                let mut rgba = px_tile(&px, w, h);
                lumit_core::mask::apply_masks(
                    &mut rgba,
                    w,
                    h,
                    f64::from(sd.width),
                    f64::from(sd.height),
                    &layer.masks,
                );
                let src = self.colour.upload_srgb8(self.gpu, &rgba, w, h);
                Ok(Some(Prepared {
                    tex: self.colour.linearise(self.gpu, &src),
                    natural: (sd.width as f32, sd.height as f32),
                    mask: None,
                }))
            }
            LayerKind::Text { document } => {
                let fill = solid_rgba(document.fill);
                let r = lumit_text::rasterise_line(
                    &document.text,
                    document.size as f32,
                    [fill[0], fill[1], fill[2]],
                );
                let mut rgba = r.rgba;
                lumit_core::mask::apply_masks(
                    &mut rgba,
                    r.width,
                    r.height,
                    f64::from(r.width),
                    f64::from(r.height),
                    &layer.masks,
                );
                let src = self.colour.upload_srgb8(self.gpu, &rgba, r.width, r.height);
                Ok(Some(Prepared {
                    tex: self.colour.linearise(self.gpu, &src),
                    natural: (r.width as f32, r.height as f32),
                    mask: None,
                }))
            }
            LayerKind::Precomp { comp } => {
                if visited.contains(comp) {
                    return Ok(None); // cycle guard: contribute nothing
                }
                let Some(nested) = self.doc.comp(*comp) else {
                    return Ok(None);
                };
                visited.push(*comp);
                let tex = self.render_comp_linear(nested, lt, visited)?;
                visited.pop();
                let mask = (!layer.masks.is_empty()).then(|| {
                    let rgba = mask_rgba(&lumit_core::mask::combined_coverage(
                        &layer.masks,
                        nested.width,
                        nested.height,
                        f64::from(nested.width),
                        f64::from(nested.height),
                    ));
                    self.colour
                        .upload_srgb8(self.gpu, &rgba, nested.width, nested.height)
                });
                Ok(Some(Prepared {
                    tex,
                    natural: (nested.width as f32, nested.height as f32),
                    mask,
                }))
            }
            // Cameras shape the view; they never draw pixels themselves.
            LayerKind::Camera { .. } => Ok(None),
        }
    }

    /// Gather a collapsed Precomp's inner layers as owned draw specs, the
    /// parent placement multiplied in front (docs/06 §1.4 — the same splice
    /// the preview's draw list performs, so export stays pixel-identical).
    /// Inner mattes cannot occur here: collapse_state forces an intermediate
    /// for them, so this path never sees one.
    fn collect_collapsed(
        &mut self,
        nested: &Composition,
        t: f64,
        visited: &mut Vec<Uuid>,
        pre: [[f32; 4]; 4],
        out: &mut Vec<CollapsedSpec>,
    ) -> Result<(), String> {
        for l in nested.layers.iter().rev() {
            if !l.switches.visible {
                continue;
            }
            let lt = t - l.start_offset.0.to_f64();
            let in_span = t >= l.in_point.0.to_f64() && t < l.out_point.0.to_f64();
            if !in_span {
                continue;
            }
            if let LayerKind::Precomp { comp: inner_id } = &l.kind {
                if matches!(
                    lumit_core::model::collapse_state(self.doc, nested, l, lt),
                    lumit_core::model::CollapseState::Active
                ) {
                    if visited.contains(inner_id) {
                        continue;
                    }
                    let Some(inner) = self.doc.comp(*inner_id) else {
                        continue;
                    };
                    let tr = &l.transform;
                    let child = lumit_gpu::place_matrix(
                        (
                            tr.position_x.value_at(lt) as f32,
                            tr.position_y.value_at(lt) as f32,
                        ),
                        (
                            tr.anchor_x.value_at(lt) as f32,
                            tr.anchor_y.value_at(lt) as f32,
                        ),
                        (
                            tr.scale_x.value_at(lt) as f32,
                            tr.scale_y.value_at(lt) as f32,
                        ),
                        tr.rotation.value_at(lt) as f32,
                        tr.position_z.value_at(lt) as f32,
                        tr.rotation_x.value_at(lt) as f32,
                        tr.rotation_y.value_at(lt) as f32,
                    );
                    visited.push(*inner_id);
                    let r = self.collect_collapsed(
                        inner,
                        lt,
                        visited,
                        lumit_gpu::concat_place(pre, child),
                        out,
                    );
                    visited.pop();
                    r?;
                    continue;
                }
            }
            let Some(p) = self.prepare(l, t, visited)? else {
                continue;
            };
            let diag = ((nested.width as f32).powi(2) + (nested.height as f32).powi(2)).sqrt();
            let markers = lumit_core::fx::MarkerContext::for_layer(nested, l);
            let neighbours = self.footage_neighbours(l, lt, nested)?;
            let flow = self.footage_flow_field(l, lt, nested)?;
            let p = Prepared {
                tex: self.apply_fx(p.tex, l, lt, diag, &markers, &neighbours, flow.as_ref()),
                natural: p.natural,
                mask: p.mask,
            };
            let tr = &l.transform;
            out.push(CollapsedSpec {
                p,
                position: (
                    tr.position_x.value_at(lt) as f32,
                    tr.position_y.value_at(lt) as f32,
                ),
                anchor: (
                    tr.anchor_x.value_at(lt) as f32,
                    tr.anchor_y.value_at(lt) as f32,
                ),
                scale: (
                    tr.scale_x.value_at(lt) as f32,
                    tr.scale_y.value_at(lt) as f32,
                ),
                rotation_deg: tr.rotation.value_at(lt) as f32,
                opacity: tr.opacity.value_at(lt) as f32,
                z: tr.position_z.value_at(lt) as f32,
                rotation_x_deg: tr.rotation_x.value_at(lt) as f32,
                rotation_y_deg: tr.rotation_y.value_at(lt) as f32,
                three_d: l.switches.three_d,
                blend: blend_of(l.blend),
                pre,
            });
        }
        Ok(())
    }

    /// Decode a footage layer's neighbour source frames for a temporal
    /// effect (echo etc.), returning them as linear textures keyed by offset
    /// — the export twin of the preview's neighbour decode (K-031: same
    /// window, same retime mapping, same nearest frame-pick, unmasked like
    /// preview). Empty unless the stack is temporal, so a plain layer decodes
    /// nothing extra.
    fn footage_neighbours(
        &mut self,
        layer: &lumit_core::model::Layer,
        lt: f64,
        comp: &Composition,
    ) -> Result<Vec<(i32, Tex)>, String> {
        use lumit_core::model::LayerKind;
        if !lumit_core::fx::stack_is_temporal(&layer.effects, layer.switches.fx) {
            return Ok(Vec::new());
        }
        let LayerKind::Footage { item, retime } = &layer.kind else {
            return Ok(Vec::new());
        };
        let (fps, frames, path) = {
            let Some(info) = self.items.get(item) else {
                return Ok(Vec::new());
            };
            (info.fps, info.frames, info.path.clone())
        };
        if !self.decoders.contains_key(item) {
            let index = lumit_media::index::build_frame_index(&path).map_err(|e| e.to_string())?;
            let dec = lumit_media::VideoDecoder::open(&path, index).map_err(|e| e.to_string())?;
            self.decoders.insert(*item, dec);
        }
        let comp_dt = 1.0 / comp.frame_rate.fps().max(1.0);
        let mut out = Vec::new();
        for o in lumit_core::fx::stack_temporal_window(&layer.effects, layer.switches.fx)
            .into_iter()
            .filter(|&o| o != 0)
        {
            let nlt = lt + f64::from(o) * comp_dt;
            let nst = retime.as_ref().map(|r| r.evaluate(nlt)).unwrap_or(nlt);
            let (nf, _) = crate::pixels::frame_pick(nst, fps, frames, false, None);
            let dec = self.decoders.get_mut(item).ok_or("decoder missing")?;
            let px = dec.frame_rgba(nf, None).map_err(|e| e.to_string())?;
            let src = self
                .colour
                .upload_srgb8(self.gpu, &px.rgba, px.width, px.height);
            out.push((o, self.colour.linearise(self.gpu, &src)));
        }
        Ok(out)
    }

    /// Compute a footage layer's dense forward flow field for Flow motion blur
    /// (docs/08 §3.2) — the export twin of the preview's decode-worker flow
    /// (K-031: the same `to_gray` → forward-flow call on the same source
    /// frames, so preview and export match). None unless the stack wants one.
    /// The current frame and the +1 neighbour are picked exactly as
    /// [`Self::footage_neighbours`] picks its frames (same retime mapping,
    /// same comp step, unmasked), and flow runs on the shared [`Self::flow`]
    /// engine; the field uploads as an `rg32float` texture the same size as
    /// the source, matching the prepared texture at full-resolution export.
    fn footage_flow_field(
        &mut self,
        layer: &lumit_core::model::Layer,
        lt: f64,
        comp: &Composition,
    ) -> Result<Option<Tex>, String> {
        use lumit_core::model::LayerKind;
        if !lumit_core::fx::stack_wants_flow_field(&layer.effects, layer.switches.fx) {
            return Ok(None);
        }
        let LayerKind::Footage { item, retime } = &layer.kind else {
            return Ok(None);
        };
        let (fps, frames, path) = {
            let Some(info) = self.items.get(item) else {
                return Ok(None);
            };
            (info.fps, info.frames, info.path.clone())
        };
        if !self.decoders.contains_key(item) {
            let index = lumit_media::index::build_frame_index(&path).map_err(|e| e.to_string())?;
            let dec = lumit_media::VideoDecoder::open(&path, index).map_err(|e| e.to_string())?;
            self.decoders.insert(*item, dec);
        }
        let comp_dt = 1.0 / comp.frame_rate.fps().max(1.0);
        let pick = |o: i32| {
            let nlt = lt + f64::from(o) * comp_dt;
            let nst = retime.as_ref().map(|r| r.evaluate(nlt)).unwrap_or(nlt);
            crate::pixels::frame_pick(nst, fps, frames, false, None).0
        };
        let (f0, f1) = (pick(0), pick(1));
        let dec = self.decoders.get_mut(item).ok_or("decoder missing")?;
        let cur = dec.frame_rgba(f0, None).map_err(|e| e.to_string())?;
        let next = dec.frame_rgba(f1, None).map_err(|e| e.to_string())?;
        let (w, h) = (cur.width as usize, cur.height as usize);
        if next.width as usize != w || next.height as usize != h {
            return Ok(None);
        }
        let ga = lumit_flow::to_gray(&cur.rgba, w, h);
        let gb = lumit_flow::to_gray(&next.rgba, w, h);
        let (fwd, _bwd) = self.flow.flow_pair(&ga, &gb);
        Ok(Some(lumit_gpu::fx::upload_flow_field(
            self.gpu, &fwd.u, &fwd.v, cur.width, cur.height,
        )))
    }

    /// Run a layer's live effect stack on its prepared linear texture
    /// (docs/08 §1.5: after masks, before transform), resolved against the
    /// comp diagonal — export renders full-resolution, so no decode scaling.
    /// `markers` is the layer's §1.4 marker context, built by the same
    /// shared constructor preview uses (K-031); `neighbours` are the temporal
    /// effect's neighbour frames (empty for a plain stack); `flow_field` is
    /// the layer's dense motion field for Flow motion blur (None otherwise).
    #[allow(clippy::too_many_arguments)]
    fn apply_fx(
        &self,
        tex: Tex,
        layer: &lumit_core::model::Layer,
        lt: f64,
        comp_diag: f32,
        markers: &lumit_core::fx::MarkerContext,
        neighbours: &[(i32, Tex)],
        flow_field: Option<&Tex>,
    ) -> Tex {
        if !layer.switches.fx || layer.effects.is_empty() {
            return tex;
        }
        // Export renders at full resolution: px@comp parameters are already
        // raster pixels (§2.3 factor 1).
        let resolved = lumit_core::fx::resolve_stack(&layer.effects, lt, comp_diag, 1.0, markers);
        let (w, h) = (tex.width(), tex.height());
        // The motion field must match the texture it smears (both are the
        // full-resolution source); a mismatch degrades to a passthrough.
        let flow = flow_field.filter(|f| f.width() == w && f.height() == h);
        crate::fxops::run_ops(&self.fx, self.gpu, tex, w, h, &resolved, neighbours, flow)
    }

    /// Render a whole comp at time `t` into a linear fp16 texture (recursive
    /// through Precomp layers).
    /// The adjustment layer's comp-space coverage (docs/06 §1.5): its mask
    /// raster — white where the effects apply — placed by its transform, so
    /// the transform moves the coverage map, never the picture. No masks
    /// means full coverage (a white quad over the whole comp).
    fn adjust_coverage(
        &self,
        comp: &Composition,
        l: &lumit_core::model::Layer,
        lt: f64,
        camera: Option<lumit_gpu::Mat4>,
    ) -> Tex {
        let white = [255u8, 255, 255, 255];
        let comp_cov;
        let (rgba, w, h): (&[u8], u32, u32) = if l.masks.is_empty() {
            (&white, 1, 1)
        } else {
            comp_cov = mask_rgba(&lumit_core::mask::combined_coverage(
                &l.masks,
                comp.width,
                comp.height,
                f64::from(comp.width),
                f64::from(comp.height),
            ));
            (&comp_cov, comp.width, comp.height)
        };
        let src = self.colour.upload_srgb8(self.gpu, rgba, w, h);
        let linear = self.colour.linearise(self.gpu, &src);
        let tr = &l.transform;
        self.compositor.composite_with_camera(
            self.gpu,
            comp.width,
            comp.height,
            [0.0, 0.0, 0.0, 0.0],
            &[lumit_gpu::CompositeLayer {
                texture: &linear,
                size: (comp.width as f32, comp.height as f32),
                position: (
                    tr.position_x.value_at(lt) as f32,
                    tr.position_y.value_at(lt) as f32,
                ),
                anchor: (
                    tr.anchor_x.value_at(lt) as f32,
                    tr.anchor_y.value_at(lt) as f32,
                ),
                scale: (
                    tr.scale_x.value_at(lt) as f32,
                    tr.scale_y.value_at(lt) as f32,
                ),
                rotation_deg: tr.rotation.value_at(lt) as f32,
                // Layer opacity is applied once, in the blend itself.
                opacity: 100.0,
                matte: None,
                blend: lumit_gpu::Blend::Normal,
                z: tr.position_z.value_at(lt) as f32,
                rotation_x_deg: tr.rotation_x.value_at(lt) as f32,
                rotation_y_deg: tr.rotation_y.value_at(lt) as f32,
                three_d: l.switches.three_d,
                layer_mask: None,
                pre: None,
            }],
            camera,
        )
    }

    fn render_comp_linear(
        &mut self,
        comp: &Composition,
        t: f64,
        visited: &mut Vec<Uuid>,
    ) -> Result<Tex, String> {
        let camera = comp
            .camera_pose(t)
            .map(|pose| camera_mat(comp.width, comp.height, pose));
        let mut prepared: HashMap<Uuid, Prepared> = HashMap::new();
        // Collapsed Precomp layers splice their inner draws instead of
        // rendering an intermediate (docs/06 §1.4) — same rule as preview,
        // decided by the same collapse_state, so the two stay pixel-identical.
        let mut spliced: HashMap<Uuid, Vec<CollapsedSpec>> = HashMap::new();
        for l in &comp.layers {
            let needed = l.switches.visible
                || comp.layers.iter().any(|c| {
                    c.switches.visible && c.matte.as_ref().is_some_and(|m| m.layer == l.id)
                });
            if !needed {
                continue;
            }
            let lt = t - l.start_offset.0.to_f64();
            if let LayerKind::Precomp { comp: nested_id } = &l.kind {
                if matches!(
                    lumit_core::model::collapse_state(self.doc, comp, l, lt),
                    lumit_core::model::CollapseState::Active
                ) {
                    if visited.contains(nested_id) {
                        continue;
                    }
                    let Some(nested) = self.doc.comp(*nested_id) else {
                        continue;
                    };
                    let tr = &l.transform;
                    let pre = lumit_gpu::place_matrix(
                        (
                            tr.position_x.value_at(lt) as f32,
                            tr.position_y.value_at(lt) as f32,
                        ),
                        (
                            tr.anchor_x.value_at(lt) as f32,
                            tr.anchor_y.value_at(lt) as f32,
                        ),
                        (
                            tr.scale_x.value_at(lt) as f32,
                            tr.scale_y.value_at(lt) as f32,
                        ),
                        tr.rotation.value_at(lt) as f32,
                        tr.position_z.value_at(lt) as f32,
                        tr.rotation_x.value_at(lt) as f32,
                        tr.rotation_y.value_at(lt) as f32,
                    );
                    let mut specs = Vec::new();
                    visited.push(*nested_id);
                    let r = self.collect_collapsed(nested, lt, visited, pre, &mut specs);
                    visited.pop();
                    r?;
                    spliced.insert(l.id, specs);
                    continue;
                }
            }
            if let Some(p) = self.prepare(l, t, visited)? {
                let diag = ((comp.width as f32).powi(2) + (comp.height as f32).powi(2)).sqrt();
                let markers = lumit_core::fx::MarkerContext::for_layer(comp, l);
                let neighbours = self.footage_neighbours(l, lt, comp)?;
                let flow = self.footage_flow_field(l, lt, comp)?;
                let p = Prepared {
                    tex: self.apply_fx(p.tex, l, lt, diag, &markers, &neighbours, flow.as_ref()),
                    natural: p.natural,
                    mask: p.mask,
                };
                prepared.insert(l.id, p);
            }
        }

        // Matte textures: the matte layer rendered alone into comp space.
        let mut matte_tex: HashMap<Uuid, Tex> = HashMap::new();
        for l in comp.layers.iter().filter(|l| l.switches.visible) {
            if let Some(mr) = &l.matte {
                if let (Some(src_layer), Some(mp)) = (
                    comp.layers.iter().find(|x| x.id == mr.layer),
                    prepared.get(&mr.layer),
                ) {
                    let mlt = t - src_layer.start_offset.0.to_f64();
                    let mtr = &src_layer.transform;
                    let rendered = self.compositor.composite_with_camera(
                        self.gpu,
                        comp.width,
                        comp.height,
                        [0.0, 0.0, 0.0, 0.0],
                        &[lumit_gpu::CompositeLayer {
                            texture: &mp.tex,
                            size: mp.natural,
                            position: (
                                mtr.position_x.value_at(mlt) as f32,
                                mtr.position_y.value_at(mlt) as f32,
                            ),
                            anchor: (
                                mtr.anchor_x.value_at(mlt) as f32,
                                mtr.anchor_y.value_at(mlt) as f32,
                            ),
                            scale: (
                                mtr.scale_x.value_at(mlt) as f32,
                                mtr.scale_y.value_at(mlt) as f32,
                            ),
                            rotation_deg: mtr.rotation.value_at(mlt) as f32,
                            opacity: mtr.opacity.value_at(mlt) as f32,
                            matte: None,
                            blend: lumit_gpu::Blend::Normal,
                            z: mtr.position_z.value_at(mlt) as f32,
                            rotation_x_deg: mtr.rotation_x.value_at(mlt) as f32,
                            rotation_y_deg: mtr.rotation_y.value_at(mlt) as f32,
                            three_d: src_layer.switches.three_d,
                            layer_mask: mp.mask.as_ref(),
                            pre: None,
                        }],
                        camera,
                    );
                    matte_tex.insert(l.id, rendered);
                }
            }
        }

        let bg = comp.background.0;
        let bg = [
            f64::from(bg[0]),
            f64::from(bg[1]),
            f64::from(bg[2]),
            f64::from(bg[3]),
        ];
        let comp_diag = ((comp.width as f32).powi(2) + (comp.height as f32).powi(2)).sqrt();
        // Adjustment staging (docs/06 §1.5): at each live adjustment layer,
        // everything gathered so far composites into an intermediate (the
        // first stage over the comp background, later ones seeded on the
        // previous blend), the stack runs on it, and the coverage blend
        // becomes the seed the layers above composite onto. Mirrors the
        // preview's realise split exactly (K-031).
        let mut acc: Option<Tex> = None;
        let mut draws: Vec<lumit_gpu::CompositeLayer> = Vec::new();
        for l in comp.layers.iter().rev() {
            if !l.switches.visible {
                continue;
            }
            if matches!(l.kind, LayerKind::Adjustment) {
                if t < l.in_point.0.to_f64() || t >= l.out_point.0.to_f64() {
                    continue;
                }
                let lt = t - l.start_offset.0.to_f64();
                let fx = if l.switches.fx {
                    // The §1.4 marker context, built by the same shared
                    // constructor preview uses (K-031).
                    let markers = lumit_core::fx::MarkerContext::for_layer(comp, l);
                    lumit_core::fx::resolve_stack(&l.effects, lt, comp_diag, 1.0, &markers)
                } else {
                    Vec::new()
                };
                if fx.is_empty() {
                    continue;
                }
                let below = self.compositor.composite_seeded(
                    self.gpu,
                    comp.width,
                    comp.height,
                    bg,
                    &draws,
                    camera,
                    acc.as_ref(),
                );
                draws.clear();
                let processed = crate::fxops::run_ops(
                    &self.fx,
                    self.gpu,
                    below.clone(),
                    comp.width,
                    comp.height,
                    &fx,
                    // An adjustment layer processes the composite below; no
                    // footage neighbour frames or flow field (temporal effects
                    // on adjustment layers are a later refinement).
                    &[],
                    None,
                );
                let coverage = self.adjust_coverage(comp, l, lt, camera);
                let opacity = (l.transform.opacity.value_at(lt) as f32 / 100.0).clamp(0.0, 1.0);
                acc = Some(self.fx.adjust_blend(
                    self.gpu,
                    &below,
                    &processed,
                    &coverage,
                    comp.width,
                    comp.height,
                    opacity,
                ));
                continue;
            }
            if let Some(specs) = spliced.get(&l.id) {
                for spec in specs {
                    draws.push(lumit_gpu::CompositeLayer {
                        texture: &spec.p.tex,
                        size: spec.p.natural,
                        position: spec.position,
                        anchor: spec.anchor,
                        scale: spec.scale,
                        rotation_deg: spec.rotation_deg,
                        opacity: spec.opacity,
                        z: spec.z,
                        rotation_x_deg: spec.rotation_x_deg,
                        rotation_y_deg: spec.rotation_y_deg,
                        three_d: spec.three_d,
                        matte: None,
                        blend: spec.blend,
                        layer_mask: spec.p.mask.as_ref(),
                        pre: Some(spec.pre),
                    });
                }
                continue;
            }
            let Some(p) = prepared.get(&l.id) else {
                continue;
            };
            let lt = t - l.start_offset.0.to_f64();
            let tr = &l.transform;
            draws.push(lumit_gpu::CompositeLayer {
                texture: &p.tex,
                size: p.natural,
                position: (
                    tr.position_x.value_at(lt) as f32,
                    tr.position_y.value_at(lt) as f32,
                ),
                anchor: (
                    tr.anchor_x.value_at(lt) as f32,
                    tr.anchor_y.value_at(lt) as f32,
                ),
                scale: (
                    tr.scale_x.value_at(lt) as f32,
                    tr.scale_y.value_at(lt) as f32,
                ),
                rotation_deg: tr.rotation.value_at(lt) as f32,
                opacity: tr.opacity.value_at(lt) as f32,
                z: tr.position_z.value_at(lt) as f32,
                rotation_x_deg: tr.rotation_x.value_at(lt) as f32,
                rotation_y_deg: tr.rotation_y.value_at(lt) as f32,
                three_d: l.switches.three_d,
                matte: l.matte.as_ref().and_then(|mr| {
                    matte_tex.get(&l.id).map(|mt| lumit_gpu::MatteInput {
                        texture: mt,
                        luma: matches!(mr.channel, MatteChannel::Luma),
                        inverted: mr.inverted,
                    })
                }),
                blend: blend_of(l.blend),
                layer_mask: p.mask.as_ref(),
                pre: None,
            });
        }

        Ok(self.compositor.composite_seeded(
            self.gpu,
            comp.width,
            comp.height,
            bg,
            &draws,
            camera,
            acc.as_ref(),
        ))
    }
}

#[allow(clippy::too_many_arguments)]
fn run(
    doc: &Document,
    comp_id: Uuid,
    items: &HashMap<Uuid, ItemInfo>,
    audio_jobs: &[AudioJob],
    gpu: &lumit_gpu::GpuContext,
    out_path: &std::path::Path,
    spec: &ExportSpec,
    tx: &Sender<ExportEvent>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let comp = doc.comp(comp_id).ok_or("composition missing")?;
    let fps = comp.frame_rate.fps().max(1.0);
    let comp_frames = (comp.duration.0.to_f64() * fps).round().max(1.0) as usize;
    // The work area is the export range (docs/01-GLOSSARY.md; K-037 relies on it).
    let (first, end) = match comp.work_area {
        Some((a, b)) => {
            let s = ((a.0.to_f64() * fps).round() as usize).min(comp_frames.saturating_sub(1));
            let e = ((b.0.to_f64() * fps).round() as usize).clamp(s + 1, comp_frames);
            (s, e)
        }
        None => (0, comp_frames),
    };
    let total = end - first;
    let _ = tx.send(ExportEvent::Progress { frame: 0, total });

    // The comp's audio, mixed exactly as playback mixes it, then cut to the
    // export range and padded so sound and picture end together.
    let rate = EXPORT_AUDIO_RATE;
    let audio_mix: Option<Vec<f32>> = if spec.include_audio && !audio_jobs.is_empty() {
        let full = mixdown(audio_jobs, rate, comp.duration.0.to_f64());
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let start = audio_samples_through(first, fps, rate).min(full.len() / 2);
        let expect = audio_samples_through(total, fps, rate);
        let mut cut = full[start * 2..(start + expect).min(full.len() / 2) * 2].to_vec();
        cut.resize(expect * 2, 0.0);
        Some(cut)
    } else {
        None
    };

    let mut renderer = Renderer {
        doc,
        items,
        gpu,
        colour: lumit_gpu::ColourEngine::new(gpu),
        compositor: lumit_gpu::Compositor::new(gpu),
        decoders: HashMap::new(),
        flow: lumit_flow::FlowEngine::with_context(gpu),
        fx: lumit_gpu::fx::FxEngine::new(gpu),
    };
    // Encoded frame dimensions must be even for 4:2:0 H.264/HEVC.
    let (tw, th) = (spec.target.0 & !1, spec.target.1 & !1);
    let (tw, th) = (tw.max(2), th.max(2));
    let resize = (tw, th) != (comp.width, comp.height);
    let audio_settings = audio_mix
        .as_ref()
        .map(|_| lumit_media::encode::AudioSettings {
            rate,
            bit_rate: spec.audio_bit_rate,
        });
    let mut encoder = lumit_media::Encoder::open(
        out_path,
        &lumit_media::encode::VideoSettings {
            codec: spec.codec,
            width: tw,
            height: th,
            fps_num: i32::try_from(comp.frame_rate.fps().round() as i64).unwrap_or(60),
            fps_den: 1,
            bit_rate: spec.bit_rate,
            max_rate: spec.max_rate,
        },
        audio_settings.as_ref(),
    )
    .map_err(|e| e.to_string())?;
    let _ = tx.send(ExportEvent::Encoder(encoder.encoder_label()));

    let mut audio_fed = 0usize;
    for frame_n in 0..total {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let t = (first + frame_n) as f64 / fps;
        let mut visited = vec![comp_id];
        let linear = renderer.render_comp_linear(comp, t, &mut visited)?;
        let shown = renderer.colour.display(gpu, &linear);
        let rgba = renderer
            .colour
            .readback8(gpu, &shown)
            .map_err(|e| e.to_string())?;
        // Letterbox into the delivery frame when a preset changes the size.
        let rgba = if resize {
            crate::pixels::letterbox_resize(&rgba, comp.width, comp.height, tw, th)
        } else {
            rgba
        };
        encoder.write_rgba(&rgba).map_err(|e| e.to_string())?;
        // Interleave: after each picture frame, the samples that cover it,
        // so the muxer keeps sound and picture together in the file.
        if let Some(mix) = &audio_mix {
            let upto = audio_samples_through(frame_n + 1, fps, rate).min(mix.len() / 2);
            if upto > audio_fed {
                encoder
                    .write_audio(&mix[audio_fed * 2..upto * 2])
                    .map_err(|e| e.to_string())?;
                audio_fed = upto;
            }
        }
        let _ = tx.send(ExportEvent::Progress {
            frame: frame_n + 1,
            total,
        });
    }
    // Any samples the per-frame rounding left behind.
    if let Some(mix) = &audio_mix {
        if mix.len() / 2 > audio_fed {
            encoder
                .write_audio(&mix[audio_fed * 2..])
                .map_err(|e| e.to_string())?;
        }
    }
    encoder.finish().map_err(|e| e.to_string())?;
    Ok(())
}

/// Coverage bytes → white RGBA whose alpha is the coverage (the layer-mask
/// texture format the compositor samples).
pub fn mask_rgba(coverage: &[u8]) -> Vec<u8> {
    coverage.iter().flat_map(|c| [255, 255, 255, *c]).collect()
}

/// Model blend → GPU blend (export copy of the preview mapping; both paths
/// must agree or preview and export diverge, K-031).
fn blend_of(b: lumit_core::model::BlendMode) -> lumit_gpu::Blend {
    use lumit_core::model::BlendMode;
    match b {
        BlendMode::Normal => lumit_gpu::Blend::Normal,
        BlendMode::Add => lumit_gpu::Blend::Add,
        BlendMode::Multiply => lumit_gpu::Blend::Multiply,
        BlendMode::Screen => lumit_gpu::Blend::Screen,
        BlendMode::Overlay => lumit_gpu::Blend::Overlay,
        BlendMode::SoftLight => lumit_gpu::Blend::SoftLight,
        BlendMode::HardLight => lumit_gpu::Blend::HardLight,
        BlendMode::Lighten => lumit_gpu::Blend::Lighten,
        BlendMode::Darken => lumit_gpu::Blend::Darken,
    }
}

/// CameraPose (core model) -> GPU camera matrix: the single conversion both
/// the preview and the export path share, so they cannot disagree (K-031).
pub fn camera_mat(
    comp_w: u32,
    comp_h: u32,
    pose: lumit_core::model::CameraPose,
) -> lumit_gpu::Mat4 {
    lumit_gpu::camera_matrix(
        comp_w as f32,
        comp_h as f32,
        pose.zoom as f32,
        (
            pose.position.0 as f32,
            pose.position.1 as f32,
            pose.position.2 as f32,
        ),
        (
            pose.rotation_deg.0 as f32,
            pose.rotation_deg.1 as f32,
            pose.rotation_deg.2 as f32,
        ),
    )
}

/// Collect the ItemInfo map from probed media (UI thread, cheap).
pub fn item_infos(
    doc: &Document,
    media: &crate::app_state::media::MediaRegistry,
) -> HashMap<Uuid, ItemInfo> {
    let mut map = HashMap::new();
    for item in &doc.items {
        if let ProjectItem::Footage(f) = item {
            if let Some(crate::app_state::media::MediaStatus::Ready { probe, frames, .. }) =
                media.map.get(&f.id)
            {
                if let Some(v) = &probe.video {
                    map.insert(
                        f.id,
                        ItemInfo {
                            path: PathBuf::from(&f.media.absolute_path),
                            fps: v.fps(),
                            frames: *frames,
                        },
                    );
                }
            }
        }
    }
    map
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_media::encode::VideoCodec;

    /// The delivery-preset table is spec (docs/06 §7.5): frame, codec, and
    /// bitrates are pinned here so a stray edit can't silently change what
    /// "YouTube 1080p60" means.
    #[test]
    fn preset_table_matches_the_spec() {
        let p = ExportPreset::Youtube1080p60.params().unwrap();
        assert_eq!(p.size, (1920, 1080));
        assert_eq!(p.codec, VideoCodec::H264);
        assert_eq!(p.target_bps, 16_000_000);
        assert_eq!(p.peak_bps, 24_000_000);

        let p = ExportPreset::Youtube4k60.params().unwrap();
        assert_eq!(p.size, (3840, 2160));
        assert_eq!(p.codec, VideoCodec::Hevc);
        assert_eq!(p.target_bps, 45_000_000);
        assert_eq!(p.peak_bps, 60_000_000);

        let p = ExportPreset::Vertical1080p60.params().unwrap();
        assert_eq!(p.size, (1080, 1920));
        assert_eq!(p.codec, VideoCodec::H264);
        assert_eq!(p.target_bps, 16_000_000);
        assert_eq!(p.peak_bps, 24_000_000);

        assert!(ExportPreset::Custom.params().is_none());
        assert_eq!(PRESET_AUDIO_BPS, 320_000);
        assert_eq!(EXPORT_AUDIO_RATE, 48_000);
    }

    #[test]
    fn every_preset_has_a_label_and_file_name() {
        for preset in ExportPreset::ALL {
            assert!(!preset.label().is_empty());
            assert!(preset.default_file_name().ends_with(".mp4"));
        }
    }

    /// The A/V interleave rule: cumulative rounding never drifts, and the
    /// total after all frames equals the whole soundtrack.
    #[test]
    fn audio_samples_through_never_drifts() {
        let (fps, rate) = (60.0, 48_000u32);
        // 60 fps at 48 kHz is exactly 800 samples per frame.
        assert_eq!(audio_samples_through(1, fps, rate), 800);
        assert_eq!(audio_samples_through(300, fps, rate), 240_000);
        // An awkward rate: 29.97 fps. Per-frame chunks vary by ±1 sample but
        // the cumulative total stays glued to the exact value.
        let fps = 30_000.0 / 1001.0;
        let mut prev = 0;
        for n in 1..=1000 {
            let now = audio_samples_through(n, fps, rate);
            let chunk = now - prev;
            assert!((1601..=1602).contains(&chunk), "frame {n} chunk {chunk}");
            let exact = n as f64 / fps * 48_000.0;
            assert!((now as f64 - exact).abs() <= 0.5, "frame {n} drifted");
            prev = now;
        }
        // Degenerate input answers zero, never panics.
        assert_eq!(audio_samples_through(100, 0.0, rate), 0);
    }

    /// A silent comp exports video-only; the padding rule keeps sound and
    /// picture the same length when there is audio.
    #[test]
    fn mixdown_of_no_jobs_is_silence_of_the_right_length() {
        let mix = mixdown(&[], 48_000, 2.0);
        assert_eq!(mix.len(), 96_000 * 2);
        assert!(mix.iter().all(|s| *s == 0.0));
    }
}
