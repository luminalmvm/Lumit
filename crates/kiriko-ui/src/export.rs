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

pub fn start(
    doc: Arc<Document>,
    comp_id: Uuid,
    items: HashMap<Uuid, ItemInfo>,
    gpu: kiriko_gpu::GpuContext,
    out_path: PathBuf,
    bit_rate: Option<i64>,
) -> ExportHandle {
    let (tx, events) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    std::thread::spawn(move || {
        let result = run(&doc, comp_id, &items, &gpu, &out_path, bit_rate, &tx, &flag);
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
}

impl Renderer<'_> {
    /// Prepare one layer's source at comp time `t` (None = contributes
    /// nothing). `parent_dims` sizes solids; `visited` guards precomp cycles.
    fn prepare(
        &mut self,
        layer: &kiriko_core::model::Layer,
        t: f64,
        parent_dims: (u32, u32),
        visited: &mut Vec<Uuid>,
    ) -> Result<Option<Prepared>, String> {
        if t < layer.in_point.0.to_f64() || t >= layer.out_point.0.to_f64() {
            return Ok(None);
        }
        let lt = t - layer.start_offset.0.to_f64();
        match &layer.kind {
            LayerKind::Footage { item } => {
                let Some(info) = self.items.get(item) else {
                    return Ok(None);
                };
                let source_frame =
                    ((lt * info.fps).round().max(0.0) as usize).min(info.frames.saturating_sub(1));
                if !self.decoders.contains_key(item) {
                    let index = kiriko_media::index::build_frame_index(&info.path)
                        .map_err(|e| e.to_string())?;
                    let dec = kiriko_media::VideoDecoder::open(&info.path, index)
                        .map_err(|e| e.to_string())?;
                    self.decoders.insert(*item, dec);
                }
                let dec = self.decoders.get_mut(item).ok_or("decoder missing")?;
                let mut px = dec
                    .frame_rgba(source_frame, None)
                    .map_err(|e| e.to_string())?;
                kiriko_core::mask::apply_masks(
                    &mut px.rgba,
                    px.width,
                    px.height,
                    f64::from(px.width),
                    f64::from(px.height),
                    &layer.masks,
                );
                let src = self
                    .colour
                    .upload_srgb8(self.gpu, &px.rgba, px.width, px.height);
                Ok(Some(Prepared {
                    tex: self.colour.linearise(self.gpu, &src),
                    natural: (px.width as f32, px.height as f32),
                }))
            }
            LayerKind::Solid { colour } => {
                let px = solid_rgba(*colour);
                // Masked solids rasterise at parent resolution; plain ones tile.
                let (w, h) = if layer.masks.is_empty() {
                    (16, 16)
                } else {
                    parent_dims
                };
                let mut rgba = px_tile(&px, w, h);
                kiriko_core::mask::apply_masks(
                    &mut rgba,
                    w,
                    h,
                    f64::from(parent_dims.0),
                    f64::from(parent_dims.1),
                    &layer.masks,
                );
                let src = self.colour.upload_srgb8(self.gpu, &rgba, w, h);
                Ok(Some(Prepared {
                    tex: self.colour.linearise(self.gpu, &src),
                    natural: (parent_dims.0 as f32, parent_dims.1 as f32),
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
                Ok(Some(Prepared {
                    tex,
                    natural: (nested.width as f32, nested.height as f32),
                }))
            }
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
        let dims = (comp.width, comp.height);
        let mut prepared: HashMap<Uuid, Prepared> = HashMap::new();
        for l in &comp.layers {
            let needed = l.switches.visible
                || comp.layers.iter().any(|c| {
                    c.switches.visible && c.matte.as_ref().is_some_and(|m| m.layer == l.id)
                });
            if !needed {
                continue;
            }
            if let Some(p) = self.prepare(l, t, dims, visited)? {
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
                    let rendered = self.compositor.composite(
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
                        }],
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
                matte: l.matte.as_ref().and_then(|mr| {
                    matte_tex.get(&l.id).map(|mt| kiriko_gpu::MatteInput {
                        texture: mt,
                        luma: matches!(mr.channel, MatteChannel::Luma),
                        inverted: mr.inverted,
                    })
                }),
                blend: match l.blend {
                    kiriko_core::model::BlendMode::Normal => kiriko_gpu::Blend::Normal,
                    kiriko_core::model::BlendMode::Add => kiriko_gpu::Blend::Add,
                    kiriko_core::model::BlendMode::Multiply => kiriko_gpu::Blend::Multiply,
                    kiriko_core::model::BlendMode::Screen => kiriko_gpu::Blend::Screen,
                },
            });
        }

        let bg = comp.background.0;
        Ok(self.compositor.composite(
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
    let mut encoder = kiriko_media::Encoder::open_with_bitrate(
        out_path,
        comp.width,
        comp.height,
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
        encoder.write_rgba(&rgba).map_err(|e| e.to_string())?;
        let _ = tx.send(ExportEvent::Progress {
            frame: frame_n + 1,
            total,
        });
    }
    encoder.finish().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn srgb_encode(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let e = if v <= 0.003_130_8 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (e * 255.0).round() as u8
}

pub fn solid_rgba(c: kiriko_core::model::LinearColour) -> [u8; 4] {
    [
        srgb_encode(c.0[0]),
        srgb_encode(c.0[1]),
        srgb_encode(c.0[2]),
        (c.0[3].clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

pub fn px_tile(px: &[u8; 4], w: u32, h: u32) -> Vec<u8> {
    std::iter::repeat_n(*px, (w * h) as usize)
        .flatten()
        .collect()
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
