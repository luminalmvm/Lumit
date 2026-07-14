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
use kiriko_core::model::{Composition, Document, LayerKind, MatteChannel, ProjectItem};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use uuid::Uuid;

type Tex = egui_wgpu::wgpu::Texture;

pub enum ExportEvent {
    Progress { frame: usize, total: usize },
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

/// A named export size. Native keeps the composition's own resolution; the
/// others letterbox the comp into a standard delivery frame (K-002 gate: the
/// YouTube/vertical presets).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ExportPreset {
    Native,
    Youtube1080,
    Youtube2160,
    Vertical1080,
}

impl ExportPreset {
    /// The target pixel size for this preset given the comp's own size.
    pub fn target(self, comp_w: u32, comp_h: u32) -> (u32, u32) {
        match self {
            ExportPreset::Native => (comp_w, comp_h),
            ExportPreset::Youtube1080 => (1920, 1080),
            ExportPreset::Youtube2160 => (3840, 2160),
            ExportPreset::Vertical1080 => (1080, 1920),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ExportPreset::Native => "Native (comp size)",
            ExportPreset::Youtube1080 => "YouTube 1080p",
            ExportPreset::Youtube2160 => "YouTube 4K",
            ExportPreset::Vertical1080 => "Vertical 1080×1920",
        }
    }
}

pub fn start(
    doc: Arc<Document>,
    comp_id: Uuid,
    items: HashMap<Uuid, ItemInfo>,
    gpu: kiriko_gpu::GpuContext,
    out_path: PathBuf,
    bit_rate: Option<i64>,
    target: (u32, u32),
) -> ExportHandle {
    let (tx, events) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    std::thread::spawn(move || {
        let result = run(
            &doc, comp_id, &items, &gpu, &out_path, bit_rate, target, &tx, &flag,
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

/// Renderer state carried down the precomp recursion.
struct Renderer<'a> {
    doc: &'a Document,
    items: &'a HashMap<Uuid, ItemInfo>,
    gpu: &'a kiriko_gpu::GpuContext,
    colour: kiriko_gpu::ColourEngine,
    compositor: kiriko_gpu::Compositor,
    decoders: HashMap<Uuid, kiriko_media::VideoDecoder>,
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
        masks: &[kiriko_core::mask::Mask],
    ) -> Result<Option<Prepared>, String> {
        let Some(info) = self.items.get(&item) else {
            return Ok(None);
        };
        // Same frame-pick + interpolation the preview uses, so export matches
        // (K-031). `pair` = fetch the neighbour; `flow` = flow-synthesise it.
        let (source_frame, blend_frame) =
            crate::pixels::frame_pick(source_time, info.fps, info.frames, pair);
        if !self.decoders.contains_key(&item) {
            let index =
                kiriko_media::index::build_frame_index(&info.path).map_err(|e| e.to_string())?;
            let dec =
                kiriko_media::VideoDecoder::open(&info.path, index).map_err(|e| e.to_string())?;
            self.decoders.insert(item, dec);
        }
        let dec = self.decoders.get_mut(&item).ok_or("decoder missing")?;
        let mut px = dec
            .frame_rgba(source_frame, None)
            .map_err(|e| e.to_string())?;
        if let Some((ceil, w)) = blend_frame {
            let px2 = dec.frame_rgba(ceil, None).map_err(|e| e.to_string())?;
            px.rgba = if flow {
                kiriko_flow::interpolate(
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
        kiriko_core::mask::apply_masks(
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
        layer: &kiriko_core::model::Layer,
        t: f64,
        visited: &mut Vec<Uuid>,
    ) -> Result<Option<Prepared>, String> {
        if t < layer.in_point.0.to_f64() || t >= layer.out_point.0.to_f64() {
            return Ok(None);
        }
        let lt = t - layer.start_offset.0.to_f64();
        match &layer.kind {
            LayerKind::Footage { item, retime } => {
                // Retime maps local → source time; preview uses the same
                // Retime::evaluate + frame-pick, so export matches preview (K-031).
                let source_time = retime.as_ref().map(|r| r.evaluate(lt)).unwrap_or(lt);
                use kiriko_core::retime::Interpolation;
                let interp = retime.as_ref().map(|r| &r.interpolation);
                let pair = matches!(interp, Some(Interpolation::Blend | Interpolation::Flow(_)));
                let flow = matches!(interp, Some(Interpolation::Flow(_)));
                self.prepare_footage(*item, source_time, pair, flow, &layer.masks)
            }
            LayerKind::Sequence { clips } => {
                // The clip under the playhead, decoded like footage; a comp
                // clip or a gap contributes nothing (comp clips join later).
                match kiriko_core::sequence::resolve(clips, lt) {
                    Some((_id, kiriko_core::sequence::ClipSource::Footage(item), st)) => {
                        use kiriko_core::retime::Interpolation;
                        let interp = kiriko_core::sequence::active_clip(clips, lt)
                            .map(|c| c.interpolation.clone());
                        let pair =
                            matches!(interp, Some(Interpolation::Blend | Interpolation::Flow(_)));
                        let flow = matches!(interp, Some(Interpolation::Flow(_)));
                        self.prepare_footage(item, st, pair, flow, &layer.masks)
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
                kiriko_core::mask::apply_masks(
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
                let r = kiriko_text::rasterise_line(
                    &document.text,
                    document.size as f32,
                    [fill[0], fill[1], fill[2]],
                );
                let mut rgba = r.rgba;
                kiriko_core::mask::apply_masks(
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
                    let rgba = mask_rgba(&kiriko_core::mask::combined_coverage(
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

    /// Render a whole comp at time `t` into a linear fp16 texture (recursive
    /// through Precomp layers).
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
        for l in &comp.layers {
            let needed = l.switches.visible
                || comp.layers.iter().any(|c| {
                    c.switches.visible && c.matte.as_ref().is_some_and(|m| m.layer == l.id)
                });
            if !needed {
                continue;
            }
            if let Some(p) = self.prepare(l, t, visited)? {
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
                        &[kiriko_gpu::CompositeLayer {
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
                            blend: kiriko_gpu::Blend::Normal,
                            z: mtr.position_z.value_at(mlt) as f32,
                            rotation_x_deg: mtr.rotation_x.value_at(mlt) as f32,
                            rotation_y_deg: mtr.rotation_y.value_at(mlt) as f32,
                            three_d: src_layer.switches.three_d,
                            layer_mask: mp.mask.as_ref(),
                        }],
                        camera,
                    );
                    matte_tex.insert(l.id, rendered);
                }
            }
        }

        let mut draws: Vec<kiriko_gpu::CompositeLayer> = Vec::new();
        for l in comp.layers.iter().rev() {
            if !l.switches.visible {
                continue;
            }
            let Some(p) = prepared.get(&l.id) else {
                continue;
            };
            let lt = t - l.start_offset.0.to_f64();
            let tr = &l.transform;
            draws.push(kiriko_gpu::CompositeLayer {
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
                    matte_tex.get(&l.id).map(|mt| kiriko_gpu::MatteInput {
                        texture: mt,
                        luma: matches!(mr.channel, MatteChannel::Luma),
                        inverted: mr.inverted,
                    })
                }),
                blend: blend_of(l.blend),
                layer_mask: p.mask.as_ref(),
            });
        }

        let bg = comp.background.0;
        Ok(self.compositor.composite_with_camera(
            self.gpu,
            comp.width,
            comp.height,
            [
                f64::from(bg[0]),
                f64::from(bg[1]),
                f64::from(bg[2]),
                f64::from(bg[3]),
            ],
            &draws,
            camera,
        ))
    }
}

#[allow(clippy::too_many_arguments)]
fn run(
    doc: &Document,
    comp_id: Uuid,
    items: &HashMap<Uuid, ItemInfo>,
    gpu: &kiriko_gpu::GpuContext,
    out_path: &std::path::Path,
    bit_rate: Option<i64>,
    target: (u32, u32),
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

    let mut renderer = Renderer {
        doc,
        items,
        gpu,
        colour: kiriko_gpu::ColourEngine::new(gpu),
        compositor: kiriko_gpu::Compositor::new(gpu),
        decoders: HashMap::new(),
    };
    // Encoded frame dimensions must be even for 4:2:0 H.264/HEVC.
    let (tw, th) = (target.0 & !1, target.1 & !1);
    let (tw, th) = (tw.max(2), th.max(2));
    let resize = (tw, th) != (comp.width, comp.height);
    let mut encoder = kiriko_media::Encoder::open_with_bitrate(
        out_path,
        tw,
        th,
        i32::try_from(comp.frame_rate.fps().round() as i64).unwrap_or(60),
        1,
        bit_rate,
    )
    .map_err(|e| e.to_string())?;

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
        let _ = tx.send(ExportEvent::Progress {
            frame: frame_n + 1,
            total,
        });
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
fn blend_of(b: kiriko_core::model::BlendMode) -> kiriko_gpu::Blend {
    use kiriko_core::model::BlendMode;
    match b {
        BlendMode::Normal => kiriko_gpu::Blend::Normal,
        BlendMode::Add => kiriko_gpu::Blend::Add,
        BlendMode::Multiply => kiriko_gpu::Blend::Multiply,
        BlendMode::Screen => kiriko_gpu::Blend::Screen,
        BlendMode::Overlay => kiriko_gpu::Blend::Overlay,
        BlendMode::SoftLight => kiriko_gpu::Blend::SoftLight,
        BlendMode::HardLight => kiriko_gpu::Blend::HardLight,
        BlendMode::Lighten => kiriko_gpu::Blend::Lighten,
        BlendMode::Darken => kiriko_gpu::Blend::Darken,
    }
}

/// CameraPose (core model) -> GPU camera matrix: the single conversion both
/// the preview and the export path share, so they cannot disagree (K-031).
pub fn camera_mat(
    comp_w: u32,
    comp_h: u32,
    pose: kiriko_core::model::CameraPose,
) -> kiriko_gpu::Mat4 {
    kiriko_gpu::camera_matrix(
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
