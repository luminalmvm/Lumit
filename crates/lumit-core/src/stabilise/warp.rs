//! The warp a stabilised frame is drawn with: the CPU reference the WGSL kernel
//! (`fx_stabilise.wgsl`) is held to, op for op.
//!
//! In plain terms: for every pixel of the output, the frame's map says which
//! point of the input it shows, and the input is read there with a bicubic
//! blend of the sixteen pixels round it. Bicubic rather than the straight blend
//! of four, because a stabilised shot is moved by a fraction of a pixel on
//! nearly every frame, and the straight blend softens a picture by a different
//! amount at every fraction, which reads as a shimmer.
//!
//! A camera that reads its sensor row by row adds one more step: the point is
//! moved to where the sensor saw it ([`unroll`]). With that term off a pixel
//! costs what it always did.

/// One frame's warp in the raster being drawn: output pixel to input pixel,
/// and the rectangle of the output that is kept, `[x0, y0, x1, y1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WarpParams {
    pub map: [[f32; 3]; 3],
    pub keep: [f32; 4],
    /// How the picture moved while the frame was read, see [`unroll`].
    pub motion: [f32; 4],
    /// The rolling shutter, see [`unroll`]. A nought first value is no term.
    pub readout: [f32; 4],
}

/// Where the frame's sensor saw the point a global shutter would put at `q`.
///
/// `motion` is `(α, β, vx, vy)`: while the frame was read the picture moved
/// `[[α, −β], [β, α]]·q + (vx, vy)` pixels a frame, the frame's slide, turn and
/// growth. `readout` is `(r, axis, size, 0)`: the read took `r` of a frame
/// interval along x (axis 0) or y (axis 1), over `size` pixels starting at
/// `start`. A point at `c` along the axis was read
/// `r·((c − start) / size − 0.5)` of a frame from the middle,
/// so it is found that far along the motion, with one refinement for the
/// motion having moved it along the axis too. Only the frame's similarity is
/// read here, per pixel; the four-corner fit is applied once, by the map.
#[must_use]
pub fn unroll(q: [f32; 2], motion: [f32; 4], readout: [f32; 4]) -> [f32; 2] {
    let m = motion;
    let v = [
        m[0] * q[0] - m[1] * q[1] + m[2],
        m[1] * q[0] + m[0] * q[1] + m[3],
    ];
    let along = |p: [f32; 2]| if readout[1] > 0.5 { p[1] } else { p[0] };
    let rho = readout[0] * ((along(q) - readout[3]) / readout[2] - 0.5);
    let q1 = [q[0] + v[0] * rho, q[1] + v[1] * rho];
    let rho = readout[0] * ((along(q1) - readout[3]) / readout[2] - 0.5);
    [q[0] + v[0] * rho, q[1] + v[1] * rho]
}

/// `m`, a map between points of the layer's frame measured from its corner,
/// for a raster the frame sits `off` pixels in on: the same map about the
/// frame's own corner. Exactly `m` when nothing has grown the raster.
#[must_use]
pub fn about(m: [[f32; 3]; 3], off: [f32; 2]) -> [[f32; 3]; 3] {
    if off == [0.0; 2] {
        return m;
    }
    let [ox, oy] = off;
    // Each row read at the point less the offset, then the offset added back
    // to the result through its weight.
    let [r0, r1, r2] = m.map(|r| [r[0], r[1], r[2] - r[0] * ox - r[1] * oy]);
    let back = |r: [f32; 3], o: f32| [0, 1, 2].map(|c| r[c] + o * r2[c]);
    [back(r0, ox), back(r1, oy), r2]
}

/// Catmull-Rom weights for a sample `t` of the way between the middle two of
/// four taps.
fn weights(t: f32) -> [f32; 4] {
    let t2 = t * t;
    let t3 = t2 * t;
    [
        0.5 * (-t3 + 2.0 * t2 - t),
        0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
        0.5 * (-3.0 * t3 + 4.0 * t2 + t),
        0.5 * (t3 - t2),
    ]
}

fn tap(src: &[f32], w: u32, h: u32, x: i32, y: i32) -> [f32; 4] {
    if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
        return [0.0; 4];
    }
    let i = ((y as u32 * w + x as u32) * 4) as usize;
    match src.get(i..i + 4) {
        Some(px) => [px[0], px[1], px[2], px[3]],
        None => [0.0; 4],
    }
}

/// The input read at `(sx, sy)` in pixel-centre coordinates, held between the
/// four nearest pixels so an edge never rings.
#[must_use]
pub fn bicubic(src: &[f32], w: u32, h: u32, sx: f32, sy: f32) -> [f32; 4] {
    let (fx, fy) = (sx - 0.5, sy - 0.5);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (wx, wy) = (weights(fx - x0), weights(fy - y0));
    let (xi, yi) = (x0 as i32, y0 as i32);
    let mut acc = [0.0f32; 4];
    let mut lo = [3.0e38f32; 4];
    let mut hi = [-3.0e38f32; 4];
    for (j, wyj) in wy.iter().enumerate() {
        let mut row = [0.0f32; 4];
        for (i, wxi) in wx.iter().enumerate() {
            let c = tap(src, w, h, xi - 1 + i as i32, yi - 1 + j as i32);
            for k in 0..4 {
                row[k] += c[k] * wxi;
            }
            if (1..=2).contains(&i) && (1..=2).contains(&j) {
                for k in 0..4 {
                    lo[k] = lo[k].min(c[k]);
                    hi[k] = hi[k].max(c[k]);
                }
            }
        }
        for k in 0..4 {
            acc[k] += row[k] * wyj;
        }
    }
    for k in 0..4 {
        acc[k] = acc[k].clamp(lo[k], hi[k]);
    }
    acc
}

/// Draw `src` (`w` by `h`, premultiplied RGBA) through the warp into a new
/// picture of the same size.
#[must_use]
pub fn warp(src: &[f32], w: u32, h: u32, p: &WarpParams) -> Vec<f32> {
    let mut out = vec![0.0f32; (w * h * 4) as usize];
    let m = &p.map;
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let kept = px >= p.keep[0] && py >= p.keep[1] && px <= p.keep[2] && py <= p.keep[3];
            if !kept {
                continue;
            }
            let u = m[0][0] * px + m[0][1] * py + m[0][2];
            let t = m[1][0] * px + m[1][1] * py + m[1][2];
            let d = m[2][0] * px + m[2][1] * py + m[2][2];
            if d <= 0.0 {
                continue;
            }
            let mut at = [u / d, t / d];
            if p.readout[0] != 0.0 {
                at = unroll(at, p.motion, p.readout);
            }
            let v = bicubic(src, w, h, at[0], at[1]);
            let i = ((y * w + x) * 4) as usize;
            if let Some(dst) = out.get_mut(i..i + 4) {
                dst.copy_from_slice(&v);
            }
        }
    }
    out
}
