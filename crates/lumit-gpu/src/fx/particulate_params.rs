//! The numbers Particulate's kernels read, laid out as `Params` in
//! `fx_particulate.wgsl` declares them.

use super::particulate::{pair_max, scan_len, tile_scale, ParticulateOp};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct ParticulateParams {
    pub(super) em_pos: [f32; 2],
    pub(super) em_wh: [f32; 2],
    pub(super) em_angle: f32,
    pub(super) dir: f32,
    pub(super) spread: f32,
    pub(super) speed: f32,

    pub(super) speed_jitter: f32,
    pub(super) shape: u32,
    pub(super) seed: u32,
    pub(super) cap: u32,

    pub(super) life: f32,
    pub(super) life_jitter: f32,
    pub(super) size: f32,
    pub(super) size_jitter: f32,

    pub(super) rotation: f32,
    pub(super) rotation_jitter: f32,
    pub(super) spin: f32,
    pub(super) align: u32,

    pub(super) colour: [f32; 4],
    pub(super) end_colour: [f32; 4],

    pub(super) wind: [f32; 2],
    pub(super) gravity: f32,
    pub(super) drag: f32,

    pub(super) turb: f32,
    pub(super) turb_scale: f32,
    pub(super) turb_speed: f32,
    pub(super) eps: f32,

    pub(super) dt: f32,
    pub(super) lead: [f32; 2],
    pub(super) frames: u32,

    pub(super) candidates: u32,
    pub(super) path_len: u32,
    pub(super) path_total: f32,
    pub(super) tail: f32,

    pub(super) feather: f32,
    pub(super) mix: f32,
    pub(super) mode: u32,
    pub(super) target_w: f32,

    pub(super) target_h: f32,
    pub(super) sprite_w: f32,
    pub(super) sprite_h: f32,
    pub(super) wind_z: f32,

    pub(super) em_z: f32,
    pub(super) em_depth: f32,
    pub(super) dir_z: f32,
    pub(super) spread_z: f32,

    pub(super) proj0: [f32; 4],
    pub(super) proj1: [f32; 4],
    pub(super) proj2: [f32; 4],

    pub(super) project: u32,
    pub(super) field_mode: u32,
    pub(super) field_invert: u32,
    pub(super) field_threshold: f32,

    /// The size of the "who stood" target, or 0 for an ordinary draw.
    pub(super) probe_w: u32,
    pub(super) probe_h: u32,
    /// How a sprite sits on its point, each as the amount past the plain
    /// square of the point's size, so all nought is that square. Only the
    /// generic points draw writes them.
    pub(super) stamp_unit: [f32; 2],

    pub(super) stamp_anchor: [f32; 2],
    pub(super) stamp_uv: [f32; 2],

    pub(super) stamp_corner: f32,
    pub(super) edge: u32,
    pub(super) margin: f32,
    /// Always nought; see the kernel's `turb_cycle`.
    pub(super) turb_cycle: i32,

    pub(super) size_by: u32,
    pub(super) opacity_by: u32,
    pub(super) colour_by: u32,
    pub(super) speed_min: f32,

    pub(super) speed_max: f32,
    pub(super) density: u32,
    pub(super) blend: u32,
    pub(super) pair_max: u32,

    pub(super) scan_len: u32,
    pub(super) tile_scale: u32,
    pub(super) _pad: [u32; 2],
}

impl ParticulateParams {
    /// `op` as the kernels read it, drawn into a raster of `w` by `h`.
    pub(super) fn of(
        op: &ParticulateOp<'_>,
        (w, h): (u32, u32),
        sprite_size: (f32, f32),
        candidates: u32,
    ) -> Self {
        let proj = op.projection.unwrap_or([[0.0; 4]; 3]);
        let cap = op.cap;
        ParticulateParams {
            em_pos: [op.emitter_pos[0], op.emitter_pos[1]],
            em_wh: op.emitter_wh,
            em_angle: op.emitter_angle_deg,
            dir: op.direction_deg,
            spread: op.spread_deg,
            speed: op.speed,
            speed_jitter: op.speed_jitter,
            shape: op.shape,
            seed: op.seed,
            cap,
            life: op.life,
            life_jitter: op.life_jitter,
            size: op.size,
            size_jitter: op.size_jitter,
            rotation: op.rotation_deg,
            rotation_jitter: op.rotation_jitter_deg,
            spin: op.spin_deg,
            align: u32::from(op.align_to_motion),
            colour: op.colour,
            end_colour: op.end_colour,
            wind: [op.wind[0], op.wind[1]],
            gravity: op.gravity,
            drag: op.drag,
            turb: op.turbulence,
            turb_scale: op.turbulence_scale,
            turb_speed: op.turbulence_speed,
            eps: op.eps,
            dt: op.dt,
            lead: [op.tail_lead, 0.0],
            frames: op.frames.len() as u32 - 1,
            candidates,
            path_len: op.path.len() as u32,
            path_total: op.path_total,
            tail: op.tail_seconds,
            feather: op.feather,
            mix: op.mix,
            mode: op.mode,
            target_w: w as f32,
            target_h: h as f32,
            sprite_w: sprite_size.0,
            sprite_h: sprite_size.1,
            wind_z: op.wind[2],
            em_z: op.emitter_pos[2],
            em_depth: op.emitter_depth,
            dir_z: op.direction_z_deg,
            spread_z: op.spread_z_deg,
            proj0: proj[0],
            proj1: proj[1],
            proj2: proj[2],
            project: u32::from(op.projection.is_some()),
            // Particulate's points were decided by the compaction; only the
            // generic draw's two field-testing callers turn the vertex-stage
            // rejection on.
            field_mode: 0,
            field_invert: 0,
            field_threshold: 0.0,
            probe_w: 0,
            probe_h: 0,
            stamp_unit: [0.0; 2],
            stamp_anchor: [0.0; 2],
            stamp_uv: [0.0; 2],
            stamp_corner: 0.0,
            edge: op.edge,
            margin: op.margin,
            turb_cycle: 0,
            size_by: op.drives[0],
            opacity_by: op.drives[1],
            colour_by: op.drives[2],
            speed_min: op.speed_range[0],
            speed_max: op.speed_range[1],
            density: u32::from(op.density.is_some()),
            blend: op.blend,
            pair_max: pair_max(cap),
            scan_len: scan_len(cap, w, h),
            tile_scale: tile_scale(w, h),
            _pad: [0; 2],
        }
    }
}
