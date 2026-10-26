//! Grid's Lines type: a line grid on its own plane, tipped and turned the way
//! Sapphire's S_Grid does it. This is the CPU reference, and the numbers the
//! GPU kernel (`fx_gridlines.wgsl`) is handed, so the two read one setup.

use super::grid::Grid;

/// How far the camera sits from the untipped grid, px@comp: half a 1920 wide
/// frame, which is where S_Grid's sits. At Z dist and Tele lens width 1 the
/// flat grid lands at its own size.
const CAMERA_PX: f32 = 960.0;

/// One family of lines, worked out once a frame, in raster pixels on the
/// grid's own plane.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LineAxis {
    /// The gap between lines. Nought draws none.
    pub cell: f32,
    /// Where line 0 sits.
    pub origin: f32,
    /// Half a line's width.
    pub half: f32,
    /// How far the lines run either side of the middle, along the other
    /// axis. Negative runs them without end.
    pub reach: f32,
    /// The last line's index, or -1 for lines without end.
    pub last: i32,
    /// The line counted from for the major lines.
    pub middle: i32,
}

/// Everything one frame of Lines needs, in raster pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridLines {
    /// Where the grid's middle lands: Position.
    pub centre: [f32; 2],
    /// The grid's across, down and outward directions after Swing, Latitude
    /// and Roll.
    pub axes: [[f32; 3]; 3],
    /// The camera's focal length and its distance from the grid.
    pub focal: f32,
    pub distance: f32,
    /// The up and down lines, spaced across.
    pub u: LineAxis,
    /// The across lines, spaced down.
    pub v: LineAxis,
    /// Every this many lines is a major line, or 0 for none.
    pub major_every: i32,
    pub major_width: f32,
    /// Premultiplied.
    pub colour: [f32; 4],
    /// 0..1.
    pub mix: f32,
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// How much of a pixel at `x` the band `-half..half` covers, both in pixels.
fn span(x: f32, half: f32) -> f32 {
    ((x + 0.5).min(half) - (x - 0.5).max(-half)).clamp(0.0, 1.0)
}

/// WGSL's `smoothstep`, written out so both sides agree.
fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl GridLines {
    /// The setup for a resolved Grid, its pixel rows already at the raster's
    /// size, and the raster factor for the camera.
    #[must_use]
    pub fn of(g: &Grid, px_scale: f32) -> Self {
        let (sw, cw) = g.swing.to_radians().sin_cos();
        let (sl, cl) = g.latitude.clamp(-89.0, 89.0).to_radians().sin_cos();
        let (sr, cr) = g.roll.to_radians().sin_cos();
        // Swing on the grid's plane, then Latitude tips the top away, then
        // Roll turns it about the view. Each column is one of the grid's own
        // directions.
        let col = |x: f32, y: f32| {
            let (x, y) = (x * cw - y * sw, x * sw + y * cw);
            let (y, z) = (y * cl, -y * sl);
            [x * cr - y * sr, x * sr + y * cr, z]
        };
        let axes = [col(1.0, 0.0), col(0.0, 1.0), [-sl * sr, sl * cr, cl]];
        let tele = g.tele_lens_width.clamp(0.2, 3.0);
        let camera = CAMERA_PX * px_scale.max(0.0);
        let width = g.line_width.max(0.0);
        let mut u = LineAxis {
            half: 0.5 * width * g.v_line_rel_width.max(0.0),
            reach: -1.0,
            last: -1,
            ..LineAxis::default()
        };
        let mut v = LineAxis {
            half: 0.5 * width * g.h_line_rel_width.max(0.0),
            ..u
        };
        if g.size_from == 1 {
            // Position and Corner are two corners of one cell.
            u.cell = (g.corner_x - g.position_x).abs();
            v.cell = (g.corner_y - g.position_y).abs();
        } else {
            let (gw, gh) = (g.grid_size_x.max(0.0), g.grid_size_y.max(0.0));
            let (bx, by) = (
                g.boxes_across.clamp(1, 100_000),
                g.boxes_down.clamp(1, 100_000),
            );
            u.cell = gw / bx as f32;
            u.origin = -0.5 * gw;
            u.last = bx;
            u.middle = bx / 2;
            v.cell = gh / by as f32;
            v.origin = -0.5 * gh;
            v.last = by;
            v.middle = by / 2;
            // The lines run to the outside edge of the border lines.
            u.reach = 0.5 * gh + v.half;
            v.reach = 0.5 * gw + u.half;
        }
        let a = g.colour[3];
        GridLines {
            centre: [g.position_x, g.position_y],
            axes,
            focal: camera * tele,
            distance: camera * g.z_dist.max(0.01),
            u,
            v,
            major_every: g.major_line_spacing.max(0),
            major_width: g.major_line_width.max(1.0),
            colour: [g.colour[0] * a, g.colour[1] * a, g.colour[2] * a, a],
            mix: (g.mix / 100.0).clamp(0.0, 1.0),
        }
    }

    /// Where the camera's ray through a raster point meets the grid, in the
    /// grid's own across and down. `None` past the horizon.
    fn plane_at(&self, x: f32, y: f32) -> Option<[f32; 2]> {
        let ray = [x - self.centre[0], y - self.centre[1], self.focal];
        let facing = dot(self.axes[2], ray);
        let t = self.distance * self.axes[2][2] / facing;
        if !(t > 0.0 && t.is_finite()) {
            return None;
        }
        let hit = [t * ray[0], t * ray[1], t * ray[2] - self.distance];
        Some([dot(self.axes[0], hit), dot(self.axes[1], hit)])
    }

    /// One family's share of a pixel. `along` is the pixel's place across
    /// the lines and `step` how far that moves in one pixel; `across` and
    /// `step_across` the same along them.
    fn family(&self, axis: &LineAxis, along: f32, step: f32, across: f32, step_across: f32) -> f32 {
        if !(axis.cell > 0.0 && axis.half > 0.0 && step > 0.0) {
            return 0.0;
        }
        let mut j = ((along - axis.origin) / axis.cell + 0.5)
            .floor()
            .clamp(-1.0e6, 1.0e6);
        if axis.last >= 0 {
            j = j.clamp(0.0, axis.last as f32);
        }
        let major = self.major_every > 0 && (j as i32 - axis.middle) % self.major_every == 0;
        let half = if major {
            axis.half * self.major_width
        } else {
            axis.half
        };
        let near = span((along - axis.origin - j * axis.cell) / step, half / step);
        // Cells only a pixel or two across can't be drawn line by line, so
        // they fade to the share of a cell the lines cover.
        let thicker = if self.major_every > 0 {
            (self.major_every as f32 - 1.0 + self.major_width) / self.major_every as f32
        } else {
            1.0
        };
        let mut share = (2.0 * axis.half * thicker / axis.cell).min(1.0);
        // A grid with ends only has that tone between them.
        if axis.last >= 0 {
            let run = 0.5 * axis.last as f32 * axis.cell;
            let s = step.min(axis.cell);
            share *= span((along - axis.origin - run) / s, (run + axis.half) / s);
        }
        let cover = share + (near - share) * smooth(1.0, 3.0, axis.cell / step);
        if axis.reach >= 0.0 && step_across > 0.0 {
            cover * span(across / step_across, axis.reach / step_across)
        } else {
            cover
        }
    }

    /// How much of pixel (`x`, `y`) the lines cover, 0..1.
    #[must_use]
    pub fn coverage(&self, x: f32, y: f32) -> f32 {
        let (Some(here), Some(right), Some(below)) = (
            self.plane_at(x + 0.5, y + 0.5),
            self.plane_at(x + 1.5, y + 0.5),
            self.plane_at(x + 0.5, y + 1.5),
        ) else {
            return 0.0;
        };
        let du = (right[0] - here[0]).hypot(below[0] - here[0]);
        let dv = (right[1] - here[1]).hypot(below[1] - here[1]);
        // Near the horizon one pixel spans so much of the plane that its
        // footprint would smear the grid's ends out past them, so an end is
        // softened over a cell at most.
        let end = |step: f32, cell: f32| if cell > 0.0 { step.min(cell) } else { step };
        let a = self.family(&self.u, here[0], du, here[1], end(dv, self.v.cell));
        let b = self.family(&self.v, here[1], dv, here[0], end(du, self.u.cell));
        a.max(b)
    }

    /// Draws the lines over a premultiplied picture, then Mix.
    pub fn draw(&self, rgba: &mut [f32], w: u32, _h: u32) {
        if self.mix <= 0.0 || w == 0 {
            return;
        }
        let w = w as usize;
        for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
            let k = self.coverage((i % w) as f32, (i / w) as f32);
            if k <= 0.0 {
                continue;
            }
            let keep = 1.0 - self.colour[3] * k;
            for (c, lane) in px.iter_mut().enumerate() {
                let over = self.colour[c] * k + *lane * keep;
                *lane += (over - *lane) * self.mix;
            }
        }
    }
}
