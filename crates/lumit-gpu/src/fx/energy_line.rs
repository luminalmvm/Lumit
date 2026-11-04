//! Energy line's passes (`fx_energy_line.wgsl`): the core from the pieces the
//! host laid out, the shared exponential halo over it, then the light laid
//! over the picture.

use std::sync::{Mutex, OnceLock};

use crate::{compiling::Compiling, GpuContext};

use super::blur::GlowOctaveOp;
use super::lens_blur::{bind_layout, scratch, Slot};
use super::{work_texture, FxEngine};

/// The tile side the host filed the pieces by, as the WGSL has it.
pub const TILE: u32 = 16;

/// One frame's light, already worked out by `lumit_core`, which this crate
/// doesn't see.
#[derive(Debug, Clone, Copy)]
pub struct EnergyLineOp<'a> {
    /// Two per piece: `(ax, ay, bx, by)`, then `(along at a, along at b,
    /// shutter moment, presence)`.
    pub pieces: &'a [[f32; 4]],
    /// The tile columns and rows.
    pub tiles: [u32; 2],
    /// `cols * rows + 1` list starts.
    pub starts: &'a [u32],
    pub items: &'a [u32],
    pub octaves: &'a [GlowOctaveOp],
    pub core_colour: [f32; 3],
    pub glow_colour: [f32; 3],
    /// The tight glow round the core, 0 for none.
    pub halo: f32,
    /// Shutter moments the pieces are spread over, at least 1.
    pub samples: u32,
    pub core_half: f32,
    pub taper: [f32; 2],
    pub softness: f32,
    pub core_warp: f32,
    pub glow_warp: f32,
    pub inv_size: f32,
    pub evolution: f32,
    pub drift: [f32; 2],
    pub glow_gain: f32,
    pub flicker: f32,
    pub composite: bool,
    pub mix: f32,
    pub seed: u32,
    pub noise_octaves: u32,
    pub gain: f32,
    pub lacunarity: f32,
    /// Bit 0 Perlin, bit 1 turbulent, as the noise core reads them.
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    core_colour: [f32; 4],
    glow_colour: [f32; 4],
    w: u32,
    h: u32,
    cols: u32,
    rows: u32,
    core_half: f32,
    taper0: f32,
    taper1: f32,
    softness: f32,
    core_warp: f32,
    glow_warp: f32,
    inv_size: f32,
    evolution: f32,
    drift_x: f32,
    drift_y: f32,
    glow_gain: f32,
    flicker: f32,
    composite: u32,
    mix_amt: f32,
    seed: u32,
    octaves: u32,
    gain: f32,
    lacunarity: f32,
    flags: u32,
    items_at: u32,
    cycle: i32,
    samples: u32,
    halo: f32,
    _pad: u32,
}

pub(super) struct EnergyLineFx {
    core: (wgpu::BindGroupLayout, wgpu::ComputePipeline),
    combine: (wgpu::BindGroupLayout, wgpu::ComputePipeline),
}

/// The pipelines, compiled on the thread pool as the engine is built, so the
/// first frame that draws one doesn't wait for the shader.
pub(super) struct LazyEnergyLine {
    device: wgpu::Device,
    format: wgpu::TextureFormat,
    building: Mutex<Option<Compiling<EnergyLineFx>>>,
    ready: OnceLock<EnergyLineFx>,
}

impl LazyEnergyLine {
    pub(super) fn spawn(ctx: &GpuContext) -> Self {
        let (device, format) = (ctx.device.clone(), ctx.working());
        let building = Compiling::start({
            let device = device.clone();
            move || EnergyLineFx::new(&device, format)
        });
        Self {
            device,
            format,
            building: Mutex::new(Some(building)),
            ready: OnceLock::new(),
        }
    }

    fn get(&self) -> &EnergyLineFx {
        self.ready.get_or_init(|| {
            self.building
                .lock()
                .ok()
                .and_then(|mut held| held.take())
                .map_or_else(
                    || EnergyLineFx::new(&self.device, self.format),
                    Compiling::wait,
                )
        })
    }
}

/// What the core is written into before the halo spreads it: two channels
/// would do, but the halo passes read four.
const CORE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

impl EnergyLineFx {
    fn new(device: &wgpu::Device, working: wgpu::TextureFormat) -> Self {
        let source = format!(
            "{}{}",
            include_str!("../fx_noise_core.wgsl"),
            include_str!("../fx_energy_line.wgsl")
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx-energy-line"),
            source: wgpu::ShaderSource::Wgsl(super::at_depth(working, &source).into()),
        });
        let compute = |label: &str, entry: &str, slots: &[(u32, Slot)]| {
            let bgl = bind_layout(device, label, slots, wgpu::ShaderStages::COMPUTE);
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[&bgl],
                push_constant_ranges: &[],
            });
            let pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            });
            (bgl, pipe)
        };
        let core = compute(
            "fx-energy-line-core",
            "el_core",
            &[
                (0, Slot::Uniform),
                (1, Slot::Spans),
                (2, Slot::Spans),
                (3, Slot::Store(CORE_FORMAT)),
            ],
        );
        let combine = compute(
            "fx-energy-line-combine",
            "el_combine",
            &[
                (0, Slot::Uniform),
                (4, Slot::Tex),
                (5, Slot::Tex),
                (6, Slot::Tex),
                (7, Slot::Store(working)),
            ],
        );
        Self { core, combine }
    }
}

impl FxEngine {
    /// Draw one energy line over a linear working texture, returning a new
    /// one of the same size. Mix 0 hands the picture back, and so do no
    /// pieces over the picture; no pieces on their own leave nothing.
    pub fn energy_line(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &EnergyLineOp<'_>,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        use wgpu::BindingResource as R;
        use wgpu::BufferUsages as B;
        use wgpu::TextureUsages as U;
        if (op.pieces.is_empty() && op.composite) || op.mix <= 0.0 || w == 0 || h == 0 {
            return src.clone();
        }
        let (Ok(items_at), [cols, rows]) = (u32::try_from(op.starts.len()), op.tiles) else {
            return src.clone();
        };
        let fx = self.energy_line.get();
        let buffer = |label, contents: &[u8], usage| {
            ctx.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents,
                    usage,
                })
        };
        let params = buffer(
            "fx-energy-line-params",
            bytemuck::bytes_of(&Params {
                core_colour: [op.core_colour[0], op.core_colour[1], op.core_colour[2], 0.0],
                glow_colour: [op.glow_colour[0], op.glow_colour[1], op.glow_colour[2], 0.0],
                w,
                h,
                cols,
                rows,
                core_half: op.core_half,
                taper0: op.taper[0],
                taper1: op.taper[1],
                softness: op.softness,
                core_warp: op.core_warp,
                glow_warp: op.glow_warp,
                inv_size: op.inv_size,
                evolution: op.evolution,
                drift_x: op.drift[0],
                drift_y: op.drift[1],
                glow_gain: op.glow_gain,
                flicker: op.flicker,
                composite: u32::from(op.composite),
                mix_amt: op.mix,
                seed: op.seed,
                octaves: op.noise_octaves,
                gain: op.gain,
                lacunarity: op.lacunarity,
                flags: op.flags,
                items_at,
                cycle: 0,
                samples: op.samples.max(1),
                halo: op.halo,
                _pad: 0,
            }),
            B::UNIFORM,
        );
        // A binding can't be empty, and no tile lists this one.
        let none = [[0.0f32; 4]; 2];
        let pieces = buffer(
            "fx-energy-line-pieces",
            bytemuck::cast_slice(if op.pieces.is_empty() {
                &none
            } else {
                op.pieces
            }),
            B::STORAGE,
        );
        // One buffer: the starts, then the lists. Never empty, since a
        // binding can't be.
        let mut words = Vec::with_capacity(op.starts.len() + op.items.len());
        words.extend_from_slice(op.starts);
        words.extend_from_slice(op.items);
        let bins = buffer(
            "fx-energy-line-bins",
            bytemuck::cast_slice(&words),
            B::STORAGE,
        );
        let core = scratch(
            ctx,
            w,
            h,
            CORE_FORMAT,
            U::STORAGE_BINDING | U::TEXTURE_BINDING,
            "fx-energy-line-core",
        );
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let group = |layout, entries: &[wgpu::BindGroupEntry<'_>]| {
            ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fx-energy-line"),
                layout,
                entries,
            })
        };
        let entry = |binding, resource| wgpu::BindGroupEntry { binding, resource };
        let pass = |pipe: &wgpu::ComputePipeline, bind: &wgpu::BindGroup| {
            let mut enc = ctx.encoder("fx-energy-line");
            {
                let mut p = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("fx-energy-line"),
                    timestamp_writes: None,
                });
                p.set_pipeline(pipe);
                p.set_bind_group(0, bind, &[]);
                p.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
            }
            drop(enc);
        };
        let v_core = view(&core);
        let core_bind = group(
            &fx.core.0,
            &[
                entry(0, params.as_entire_binding()),
                entry(1, pieces.as_entire_binding()),
                entry(2, bins.as_entire_binding()),
                entry(3, R::TextureView(&v_core)),
            ],
        );
        pass(&fx.core.1, &core_bind);
        let halo = if op.glow_gain > 0.0 && !op.pieces.is_empty() {
            self.glow_exponential(ctx, &core, w, h, op.octaves)
        } else {
            work_texture(ctx, w, h, "fx-energy-line-no-halo")
        };
        let out = work_texture(ctx, w, h, "fx-energy-line-out");
        let (v_src, v_halo, v_out) = (view(src), view(&halo), view(&out));
        let combine_bind = group(
            &fx.combine.0,
            &[
                entry(0, params.as_entire_binding()),
                entry(4, R::TextureView(&v_src)),
                entry(5, R::TextureView(&v_core)),
                entry(6, R::TextureView(&v_halo)),
                entry(7, R::TextureView(&v_out)),
            ],
        );
        pass(&fx.combine.1, &combine_bind);
        ctx.recycle(halo);
        out
    }
}
