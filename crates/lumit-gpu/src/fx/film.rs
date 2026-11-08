//! Film's passes (`fx_film.wgsl`), in the order `lumit_core::fx::film::cpu`
//! runs them: expose, the spreading passes on Glow's blurs, the couplers, the
//! grain's draw, development, then the gate and the Mix.

use std::sync::OnceLock;

use crate::GpuContext;

use super::blur::GlowShape;
use super::lens_blur::{bind_layout, Slot};
use super::{work_texture, FxEngine};

/// One spreading pass: the picture kept, plus each blur of it, per channel.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FilmStageOp {
    pub keep: [f32; 3],
    pub terms: Vec<(GlowShape, [f32; 3])>,
}

impl FilmStageOp {
    fn is_identity(&self) -> bool {
        self.terms.is_empty() && self.keep == [1.0; 3]
    }
}

/// One frame's Film, already resolved. Mirrors
/// `lumit_core::fx::effects::film::FilmFrame`.
#[derive(Debug, Clone, PartialEq)]
pub struct FilmOp<'a> {
    pub camera: [[f32; 3]; 3],
    /// Stops Clipped highlights lifts by.
    pub boost: f32,
    pub inverse: [[f32; 3]; 3],
    pub couplers: [[f32; 3]; 3],
    pub to_rec709: [[f32; 3]; 3],
    pub couplers_on: bool,
    /// 0 print, 1 scan, 2 slide, 3 the picture's own.
    pub shown: u32,
    pub positive: bool,
    pub gamma: f32,
    pub top: [f32; 3],
    pub grey: [f32; 3],
    pub slope: [f32; 3],
    pub grain: f32,
    pub grain_colour: f32,
    pub seed: u32,
    pub tick: i32,
    pub print_gain: [f32; 3],
    pub print_gamma: f32,
    pub scan_low: [f32; 3],
    pub scan_top: [f32; 3],
    pub weave: [f32; 2],
    pub mix: f32,
    pub bloom: FilmStageOp,
    pub scatter: FilmStageOp,
    pub halation: FilmStageOp,
    pub held: FilmStageOp,
    pub grain_blur: FilmStageOp,
    /// The tables, and where each starts in them: curve, curve before the
    /// couplers, print curve, grain, print cube, scan cube.
    pub tables: &'a [f32],
    pub at: [u32; 6],
    /// Samples in a curve, a grain table and a cube's side.
    pub sizes: [u32; 3],
    pub floor: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    cam: [[f32; 4]; 3],
    inv: [[f32; 4]; 3],
    cpl: [[f32; 4]; 3],
    out: [[f32; 4]; 3],
    top: [f32; 4],
    grey: [f32; 4],
    slope: [f32; 4],
    print_gain: [f32; 4],
    scan_low: [f32; 4],
    scan_top: [f32; 4],
    k0: [f32; 4],
    k1: [f32; 4],
    k2: [f32; 4],
    k3: [f32; 4],
    weave: [f32; 4],
    gamma: f32,
    grain: f32,
    grain_colour: f32,
    print_gamma: f32,
    shown: u32,
    flags: u32,
    seed: u32,
    tick: i32,
    at: [u32; 6],
    curve_n: u32,
    grain_n: u32,
    lut_n: u32,
    floor: f32,
    boost: f32,
    _p: u32,
}

fn v4(v: [f32; 3]) -> [f32; 4] {
    [v[0], v[1], v[2], 0.0]
}

fn rows(m: &[[f32; 3]; 3]) -> [[f32; 4]; 3] {
    m.map(v4)
}

impl FilmOp<'_> {
    fn params(&self) -> Params {
        Params {
            cam: rows(&self.camera),
            inv: rows(&self.inverse),
            cpl: rows(&self.couplers),
            out: rows(&self.to_rec709),
            top: v4(self.top),
            grey: v4(self.grey),
            slope: v4(self.slope),
            print_gain: v4(self.print_gain),
            scan_low: v4(self.scan_low),
            scan_top: v4(self.scan_top),
            k0: [0.0; 4],
            k1: [0.0; 4],
            k2: [0.0; 4],
            k3: [0.0; 4],
            weave: [self.weave[0], self.weave[1], self.mix, 0.0],
            gamma: self.gamma,
            grain: self.grain,
            grain_colour: self.grain_colour,
            print_gamma: self.print_gamma,
            shown: self.shown,
            flags: u32::from(self.couplers_on)
                | (u32::from(self.positive) << 1)
                | (u32::from(self.grain > 0.0) << 2),
            seed: self.seed,
            tick: self.tick,
            at: self.at,
            curve_n: self.sizes[0],
            grain_n: self.sizes[1],
            lut_n: self.sizes[2],
            floor: self.floor,
            boost: self.boost,
            _p: 0,
        }
    }
}

pub(super) struct FilmFx {
    layout: wgpu::BindGroupLayout,
    expose: wgpu::ComputePipeline,
    acc: wgpu::ComputePipeline,
    couple: wgpu::ComputePipeline,
    noise: wgpu::ComputePipeline,
    develop: wgpu::ComputePipeline,
    finish: wgpu::ComputePipeline,
}

impl FilmFx {
    fn new(ctx: &GpuContext) -> Self {
        let device = &ctx.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx-film"),
            source: wgpu::ShaderSource::Wgsl(
                super::at_depth(
                    ctx.working(),
                    concat!(
                        include_str!("../fx_noise_core.wgsl"),
                        include_str!("../fx_film.wgsl")
                    ),
                )
                .into(),
            ),
        });
        let layout = bind_layout(
            device,
            "fx-film",
            &[
                (0, Slot::Tex),
                (1, Slot::Tex),
                (2, Slot::Store(ctx.working())),
                (3, Slot::Uniform),
                (4, Slot::Tex),
                (5, Slot::Tex),
                (6, Slot::Spans),
            ],
            wgpu::ShaderStages::COMPUTE,
        );
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fx-film"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipe = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Self {
            expose: pipe("film_expose"),
            acc: pipe("film_acc"),
            couple: pipe("film_couple"),
            noise: pipe("film_noise"),
            develop: pipe("film_develop"),
            finish: pipe("film_finish"),
            layout,
        }
    }
}

/// Built on first use: most projects never shoot on film.
pub(super) type LazyFilm = OnceLock<FilmFx>;

/// The textures one pass reads: the picture, the second picture, and the two
/// extras.
struct Reads<'t> {
    src: &'t wgpu::Texture,
    orig: &'t wgpu::Texture,
    a: &'t wgpu::Texture,
    b: &'t wgpu::Texture,
}

fn one(t: &wgpu::Texture) -> Reads<'_> {
    Reads {
        src: t,
        orig: t,
        a: t,
        b: t,
    }
}

fn tex(binding: u32, v: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(v),
    }
}

impl FxEngine {
    /// Film over a linear working texture (premultiplied), returning a new one
    /// of the same size.
    pub fn film(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &FilmOp<'_>,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        let fx = self.film.get_or_init(|| FilmFx::new(ctx));
        let tables = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-film-tables"),
                contents: bytemuck::cast_slice(op.tables),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let base = op.params();
        let run = |pipe: &wgpu::ComputePipeline, reads: Reads<'_>, params: &Params| {
            let out = work_texture(ctx, w, h, "fx-film");
            let ubuf = ctx
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("fx-film-params"),
                    contents: bytemuck::bytes_of(params),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            let view = |t: &wgpu::Texture| t.create_view(&Default::default());
            let (v0, v1, v2, v4, v5) = (
                view(reads.src),
                view(reads.orig),
                view(&out),
                view(reads.a),
                view(reads.b),
            );
            let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fx-film"),
                layout: &fx.layout,
                entries: &[
                    tex(0, &v0),
                    tex(1, &v1),
                    tex(2, &v2),
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: ubuf.as_entire_binding(),
                    },
                    tex(4, &v4),
                    tex(5, &v5),
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: tables.as_entire_binding(),
                    },
                ],
            });
            let mut enc = ctx.encoder("fx-film");
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("fx-film"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(pipe);
                pass.set_bind_group(0, &bind, &[]);
                pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
            }
            drop(enc);
            out
        };
        // == cpu::spread. Takes the picture and hands back its successor: every
        // blur first, then their sum with the picture, three blurs a pass.
        let spread = |input: wgpu::Texture, stage: &FilmStageOp| -> wgpu::Texture {
            if stage.is_identity() {
                return input;
            }
            let blurred: Vec<wgpu::Texture> = stage
                .terms
                .iter()
                .map(|(shape, _)| match shape {
                    GlowShape::Gaussian(g) => self.glow_gaussian(ctx, &input, w, h, g),
                    GlowShape::Exponential(o) => self.glow_exponential(ctx, &input, w, h, o),
                })
                .collect();
            let mut blurred = blurred;
            // A blur and nothing else is the blur itself.
            let lone = stage.keep == [0.0; 3]
                && matches!(stage.terms.as_slice(), [(_, weight)] if *weight == [1.0; 3]);
            if lone {
                if let Some(only) = blurred.pop() {
                    ctx.recycle(input);
                    return only;
                }
            }
            let mut acc: Option<wgpu::Texture> = None;
            let mut weights = stage.terms.iter().map(|(_, w)| *w);
            let mut done = 0;
            loop {
                let chunk = &blurred[done.min(blurred.len())..(done + 3).min(blurred.len())];
                let k = [weights.next(), weights.next(), weights.next()]
                    .map(|w| v4(w.unwrap_or([0.0; 3])));
                let (orig, k0) = match &acc {
                    None => (&input, v4(stage.keep)),
                    Some(sum) => (sum, v4([1.0; 3])),
                };
                let pick = |i: usize| chunk.get(i).unwrap_or(orig);
                let next = run(
                    &fx.acc,
                    Reads {
                        src: pick(0),
                        orig,
                        a: pick(1),
                        b: pick(2),
                    },
                    &Params {
                        k0,
                        k1: k[0],
                        k2: k[1],
                        k3: k[2],
                        ..base
                    },
                );
                if let Some(old) = acc.replace(next) {
                    ctx.recycle(old);
                }
                done += 3;
                if done >= blurred.len() {
                    break;
                }
            }
            for spent in blurred {
                ctx.recycle(spent);
            }
            ctx.recycle(input);
            acc.unwrap_or_else(|| work_texture(ctx, w, h, "fx-film"))
        };

        let mut lit = run(&fx.expose, one(src), &base);
        for stage in [&op.bloom, &op.scatter, &op.halation] {
            lit = spread(lit, stage);
        }
        let held = op
            .couplers_on
            .then(|| spread(run(&fx.couple, one(&lit), &base), &op.held));
        let noise =
            (op.grain > 0.0).then(|| spread(run(&fx.noise, one(&lit), &base), &op.grain_blur));
        let shown = run(
            &fx.develop,
            Reads {
                src: &lit,
                orig: src,
                a: held.as_ref().unwrap_or(&lit),
                b: noise.as_ref().unwrap_or(&lit),
            },
            &base,
        );
        let out = run(
            &fx.finish,
            Reads {
                orig: src,
                ..one(&shown)
            },
            &base,
        );
        for spent in [Some(lit), held, noise, Some(shown)].into_iter().flatten() {
            ctx.recycle(spent);
        }
        out
    }
}
