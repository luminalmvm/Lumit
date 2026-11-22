//! Holographic foil's pass: one dispatch of `fx_holographic_foil.wgsl`, with
//! its own layout for the matte, the five optional pictures and the halved
//! copies of the layer a bevel reads. Each Material
//! has its own pipelines, built the first time they are drawn: the kernel with
//! every material in it took FXC (DX12) many minutes to compile. Each comes
//! lean (no wear, no Height, Direction, Pitch or Environment map, no lone
//! glitter flashes), which most frames need and which compiles in about half
//! the time, or full. Once one is drawn, the rest are built on a thread of
//! their own, one after another, so changing Material or turning on wear
//! later seldom waits.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, Weak};

use crate::GpuContext;

use super::hologram::Halver;
use super::lens_blur::{bind_layout, Slot};
use super::{work_texture, FxEngine};

/// The uniform's rows of four floats.
pub const HOLOGRAPHIC_FOIL_ROWS: usize = 2210;

/// Prism, Shatter, Glitter, Sparkle and Vortex.
const MATERIALS: usize = 5;

/// Shatter and Glitter, whose shards and flakes set their own pitch, and
/// Sparkle, which with Prism follows Pattern's grooves.
const SHATTER: u32 = 1;
const GLITTER: u32 = 2;
const SPARKLE: u32 = 3;

/// The grating's row: the Pattern choice in z, 3 for grooves From map.
const ROW_GRATING: usize = 4;
const FROM_MAP: f32 = 3.0;

/// Rows the choice of kernel reads, as the kernel's R_SIZE, R_RELIEF_MORE
/// and R_GLITTER: mm per pixel in z, wear in y, flake size in x and layers
/// in z.
const ROW_SIZE: usize = 0;
const ROW_WEAR: usize = 193;
const ROW_GLITTER: usize = 280;

/// The bevel's row: its width in x, 0 when the Relief is not a bevel.
const ROW_BEVEL: usize = 279;

/// Where the kernel reads the halved copies of the layer for a bevel.
const BEVEL_BINDING: u32 = 13;

/// The pictures only the full kernel reads: the Height, Direction, Pitch and
/// Environment maps, as bits of the flags.
const FULL_PICTURES: u32 = 0b11_1010;

/// One frame of Holographic foil. The rows mirror
/// `lumit_core::fx::effects::holographic_foil::HolographicFoilParams`.
#[derive(Debug, Clone, Copy)]
pub struct HolographicFoilOp<'a> {
    /// The Material choice, 0 to 4: which pipeline draws it.
    pub material: u32,
    pub rows: [[f32; 4]; HOLOGRAPHIC_FOIL_ROWS],
    /// Scales the grating's coverage, prepared at the seam.
    pub matte: Option<&'a wgpu::Texture>,
    pub height_map: Option<&'a wgpu::Texture>,
    pub foil_mask: Option<&'a wgpu::Texture>,
    pub direction_map: Option<&'a wgpu::Texture>,
    pub pitch_map: Option<&'a wgpu::Texture>,
    pub environment: Option<&'a wgpu::Texture>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    /// x: one bit for each picture bound, in the order of the op's fields.
    flags: [u32; 4],
    rows: [[f32; 4]; HOLOGRAPHIC_FOIL_ROWS],
}

pub(super) struct HolographicFoilFx {
    device: wgpu::Device,
    layout: wgpu::BindGroupLayout,
    pl: wgpu::PipelineLayout,
    module: wgpu::ShaderModule,
    /// Lean then full, for each material.
    pipelines: [OnceLock<wgpu::ComputePipeline>; 2 * MATERIALS],
    /// Halved copies of the layer, whose alpha a bevel reads blurred.
    halver: Halver,
}

impl HolographicFoilFx {
    fn new(ctx: &GpuContext) -> Self {
        let device = &ctx.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx-holographic-foil"),
            source: wgpu::ShaderSource::Wgsl(
                super::at_depth(
                    ctx.working(),
                    concat!(
                        include_str!("../fx_holo_core.wgsl"),
                        include_str!("../fx_holo_relief.wgsl"),
                        include_str!("../fx_holo_shatter.wgsl"),
                        include_str!("../fx_holo_glitter.wgsl"),
                        include_str!("../fx_holo_sparkle.wgsl"),
                        include_str!("../fx_holographic_foil.wgsl")
                    ),
                )
                .into(),
            ),
        });
        let mut slots = vec![
            (0, Slot::Tex),
            (1, Slot::Uniform),
            (2, Slot::Store(ctx.working())),
        ];
        slots.extend((3..9).map(|b| (b, Slot::Tex)));
        slots.push((BEVEL_BINDING, Slot::Tex));
        let layout = bind_layout(
            device,
            "fx-holographic-foil",
            &slots,
            wgpu::ShaderStages::COMPUTE,
        );
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fx-holographic-foil"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        Self {
            device: device.clone(),
            layout,
            pl,
            module,
            pipelines: Default::default(),
            halver: Halver::new(device),
        }
    }

    /// Pipeline `at` (material times two, plus one for the full kernel),
    /// built on first use with the kernel's MATERIAL and FULL fixed, so FXC
    /// compiles only that code. A pipeline another thread is building is
    /// waited for, never built twice.
    fn pipeline(&self, at: usize) -> &wgpu::ComputePipeline {
        let at = at.min(2 * MATERIALS - 1);
        self.pipelines[at].get_or_init(|| {
            let constants = HashMap::from([
                ("MATERIAL".to_owned(), (at / 2) as f64),
                ("FULL".to_owned(), (at % 2) as f64),
            ]);
            self.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("fx-holographic-foil"),
                    layout: Some(&self.pl),
                    module: &self.module,
                    entry_point: Some("holographic_foil"),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &constants,
                        ..Default::default()
                    },
                    cache: None,
                })
        })
    }
}

/// Built on first use: most projects never draw any foil.
pub(super) type LazyHolographicFoil = OnceLock<Arc<HolographicFoilFx>>;

/// Which pipeline draws `op` with these pictures bound: the full kernel when
/// the frame has wear, a bevel, a map only it reads, or glitter flakes near or below
/// a pixel (the kernel draws their lone flashes from about one flake to a
/// pixel's footprint; the choice leaves a margin).
fn pipeline_for(op: &HolographicFoilOp<'_>, flags: u32) -> usize {
    let row = |r: usize, k: usize| f64::from(op.rows[r][k]);
    let mut full = flags & FULL_PICTURES != 0 || row(ROW_WEAR, 1) > 0.0 || row(ROW_BEVEL, 0) > 0.0;
    if op.material == GLITTER {
        let smm = 0.398_942_28 * row(ROW_SIZE, 2);
        let cell = 1.25 * row(ROW_GLITTER, 0);
        let flakes = row(ROW_GLITTER, 2) * std::f64::consts::TAU * smm * smm / (cell * cell);
        full |= flakes.is_nan() || flakes >= 0.99;
    }
    2 * (op.material as usize).min(MATERIALS - 1) + usize::from(full)
}

/// Builds every other pipeline on a thread of its own, one after another:
/// the other kernel of the material first drawn (`first`), then every lean
/// one, then every full one. The thread holds the pipelines weakly, so once
/// the engine lets them go it stops after the build in hand.
#[cfg(not(target_arch = "wasm32"))]
fn build_the_rest(fx: &Arc<HolographicFoilFx>, first: usize) {
    let fx: Weak<HolographicFoilFx> = Arc::downgrade(fx);
    let lean = (0..MATERIALS).map(|m| 2 * m);
    let order: Vec<usize> = std::iter::once(first ^ 1)
        .chain(lean.clone())
        .chain(lean.map(|at| at + 1))
        .filter(|&at| at != first)
        .collect();
    // Without the thread every pipeline is still built when first drawn.
    let _ = std::thread::Builder::new()
        .name("holographic foil pipelines".into())
        .spawn(move || {
            for at in order {
                let Some(fx) = fx.upgrade() else {
                    return;
                };
                fx.pipeline(at);
            }
        });
}

#[cfg(target_arch = "wasm32")]
fn build_the_rest(_fx: &Arc<HolographicFoilFx>, _first: usize) {}

impl FxEngine {
    /// Holographic foil over a linear working texture (premultiplied),
    /// returning a new one of the same size.
    pub fn holographic_foil(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &HolographicFoilOp<'_>,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        let mut first = false;
        let fx = self.holographic_foil.get_or_init(|| {
            first = true;
            Arc::new(HolographicFoilFx::new(ctx))
        });
        // A map the material does not read is left unbound, so it never
        // picks the slower kernel: the Direction map steers only the grooves
        // Pattern lays From map on Prism and Sparkle, and shards and flakes
        // set their own pitch.
        let steered = op.rows[ROW_GRATING][2] == FROM_MAP && matches!(op.material, 0 | SPARKLE);
        let pitched = !matches!(op.material, SHATTER | GLITTER);
        let pictures = [
            op.matte,
            op.height_map,
            op.foil_mask,
            op.direction_map.filter(|_| steered),
            op.pitch_map.filter(|_| pitched),
            op.environment,
        ];
        let flags = pictures
            .iter()
            .enumerate()
            .fold(0u32, |f, (i, t)| f | (u32::from(t.is_some()) << i));
        // A Height map takes the bevel's place, as it does the relief's: the
        // bevel's row goes to the kernel empty.
        let mut rows = op.rows;
        if op.height_map.is_some() {
            rows[ROW_BEVEL] = [0.0; 4];
        }
        let at = pipeline_for(op, flags);
        let pipeline = fx.pipeline(at);
        if first {
            build_the_rest(fx, at);
        }
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-holographic-foil-params"),
                contents: bytemuck::bytes_of(&Params {
                    flags: [flags, 0, 0, 0],
                    rows,
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let out = work_texture(ctx, w, h, "fx-holographic-foil-out");
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let v_src = view(src);
        let v_out = view(&out);
        // An unset picture is bound to the input, and its flag stays clear.
        let v_pics = pictures.map(|t| view(t.unwrap_or(src)));
        let halved = (rows[ROW_BEVEL][0] > 0.0).then(|| fx.halver.mips(ctx, src, w, h, true));
        let v_halved = view(halved.as_ref().unwrap_or(src));
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&v_src),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: ubuf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&v_out),
            },
        ];
        entries.extend(
            v_pics
                .iter()
                .zip(3u32..)
                .map(|(v, binding)| wgpu::BindGroupEntry {
                    binding,
                    resource: wgpu::BindingResource::TextureView(v),
                }),
        );
        entries.push(wgpu::BindGroupEntry {
            binding: BEVEL_BINDING,
            resource: wgpu::BindingResource::TextureView(&v_halved),
        });
        let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-holographic-foil"),
            layout: &fx.layout,
            entries: &entries,
        });
        let mut enc = ctx.encoder("fx-holographic-foil");
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-holographic-foil"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        drop(enc);
        out
    }
}
