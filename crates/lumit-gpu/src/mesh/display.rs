//! How a 3D view shows its models: as they export, under a studio light of
//! their own, as lines, or by which way they face. Only the Viewer ever asks
//! for anything but the look they export with.

use super::{upload::triangles, Vertex, COLOUR, DEPTH};
use crate::GpuContext;
use lumit_scene::ModelScene;
use wgpu::util::DeviceExt;

/// What a model's faces show in a view.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Shading {
    /// Lit by the comp's own lights, as it exports.
    #[default]
    Final,
    /// Lit by a studio light of its own, whatever lights the comp has.
    Shaded,
    /// Its edges only.
    Wireframe,
    /// Shaded, with its edges over the faces.
    ShadedWire,
    /// Which way each pixel faces, as colour.
    Normals,
}

/// How one view shows models.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ModelDisplay {
    pub shading: Shading,
    /// Fronts tinted blue and backs red.
    pub faces: bool,
}

impl ModelDisplay {
    /// One number for the whole of it, nought for the look a model exports
    /// with.
    #[must_use]
    pub fn code(self) -> u8 {
        self.shading as u8 * 2 + u8::from(self.faces)
    }

    /// Whether it takes shadows at all: only as it exports, as the other
    /// looks are for seeing the model itself.
    #[must_use]
    pub fn takes_shadows(self) -> bool {
        self.shading == Shading::Final
    }

    pub(super) fn draws_faces(self) -> bool {
        self.shading != Shading::Wireframe
    }

    pub(super) fn draws_wires(self) -> bool {
        matches!(self.shading, Shading::Wireframe | Shading::ShadedWire)
    }

    /// The shader's say: how the faces shade (nought as they export, one in
    /// the studio light, two as their normals) and whether they are tinted.
    pub(super) fn flags(self) -> (f32, f32) {
        let shade = match self.shading {
            Shading::Final => 0.0,
            Shading::Shaded | Shading::ShadedWire | Shading::Wireframe => 1.0,
            Shading::Normals => 2.0,
        };
        (shade, f32::from(u8::from(self.faces)))
    }

    /// The lines' colour, linear and premultiplied: light on their own, and
    /// a faint dark over the faces, so a dense mesh still shows them.
    pub(super) fn wire(self) -> [f32; 4] {
        match self.shading {
            Shading::ShadedWire => [0.0, 0.0, 0.0, 0.35],
            _ => [0.75, 0.75, 0.75, 1.0],
        }
    }
}

/// The lines' pipeline at `samples`: each triangle's edges, a hair nearer
/// than the faces under them.
pub(super) fn wire_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    samples: u32,
) -> wgpu::RenderPipeline {
    let attributes = wgpu::vertex_attr_array![
        0 => Float32x3, 1 => Float32x3, 2 => Float32x4,
        3 => Float32x2, 4 => Float32x2, 5 => Float32x4,
    ];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("mesh-wire"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_wire"),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &attributes,
            }],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_wire"),
            targets: &[Some(wgpu::ColorTargetState {
                format: COLOUR,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::LineList,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::GreaterEqual,
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        },
        multiview: None,
        cache: None,
    })
}

/// A model's edges as lines, two corners each, laid out like its triangles
/// so a part's run of them starts at twice its first index. `None` when the
/// card has no room for them, asked again the next time a view draws lines.
// ponytail: an edge two triangles share is drawn twice; weld them if the
// memory is ever missed.
pub(super) fn edges(
    ctx: &GpuContext,
    scene: &ModelScene,
) -> Option<(wgpu::Buffer, lumit_budget::Reservation)> {
    let mut lines: Vec<u32> = Vec::new();
    for p in scene.meshes.iter().flat_map(|m| &m.primitives) {
        for t in triangles(p).unwrap_or_default().chunks_exact(3) {
            lines.extend_from_slice(&[t[0], t[1], t[1], t[2], t[2], t[0]]);
        }
    }
    if lines.is_empty() {
        lines.extend([0; 2]);
    }
    let bytes = std::mem::size_of_val(lines.as_slice()) as u64;
    if bytes > ctx.device.limits().max_buffer_size {
        return None;
    }
    let held = ctx.ledger().reserve(lumit_budget::Tier::Vram, bytes).ok()?;
    let buffer = ctx
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh-edges"),
            contents: bytemuck::cast_slice(&lines),
            usage: wgpu::BufferUsages::INDEX,
        });
    Some((buffer, held))
}
