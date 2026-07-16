//! Masks: bezier paths that gate a layer's alpha (docs/03-DATA-MODEL.md §7),
//! plus the scanline rasteriser that turns a path into pixel coverage.
//!
//! In plain terms: a mask is a drawn shape; inside the shape the layer shows,
//! outside it doesn't (or the reverse when inverted). The rasteriser walks
//! the shape row by row, finding where each row enters and leaves the shape —
//! with fractional edges and two vertical subsamples so boundaries render
//! smooth, not stair-stepped. Phase 1 scope: static paths, Add mode; animated
//! paths, feather and the full mode set follow the data model doc.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One path vertex with cubic tangent handles (layer-pixel coordinates;
/// tangents relative to the vertex).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vertex {
    pub pos: (f64, f64),
    pub tan_in: (f64, f64),
    pub tan_out: (f64, f64),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BezierPath {
    pub vertices: Vec<Vertex>,
    pub closed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mask {
    pub id: Uuid,
    pub name: String,
    pub path: BezierPath,
    pub inverted: bool,
    /// 0..100.
    pub opacity: f64,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Mask {
    pub fn rectangle(x: f64, y: f64, w: f64, h: f64) -> Self {
        let corner = |px: f64, py: f64| Vertex {
            pos: (px, py),
            tan_in: (0.0, 0.0),
            tan_out: (0.0, 0.0),
        };
        Self {
            id: Uuid::now_v7(),
            name: "Rectangle".into(),
            path: BezierPath {
                vertices: vec![
                    corner(x, y),
                    corner(x + w, y),
                    corner(x + w, y + h),
                    corner(x, y + h),
                ],
                closed: true,
            },
            inverted: false,
            opacity: 100.0,
            extra: serde_json::Map::new(),
        }
    }

    /// An `n`-point star with straight edges (corner vertices only), outer
    /// radius `outer`, inner radius `inner`. Points start at the top.
    pub fn star(cx: f64, cy: f64, outer: f64, inner: f64, n: usize) -> Self {
        let n = n.max(3);
        let mut vertices = Vec::with_capacity(n * 2);
        for i in 0..n * 2 {
            let r = if i % 2 == 0 { outer } else { inner };
            // -PI/2 puts the first outer point at the top.
            let a = std::f64::consts::PI * f64::from(i as u32) / f64::from(n as u32)
                - std::f64::consts::FRAC_PI_2;
            vertices.push(Vertex {
                pos: (cx + r * a.cos(), cy + r * a.sin()),
                tan_in: (0.0, 0.0),
                tan_out: (0.0, 0.0),
            });
        }
        Self {
            id: Uuid::now_v7(),
            name: "Star".into(),
            path: BezierPath {
                vertices,
                closed: true,
            },
            inverted: false,
            opacity: 100.0,
            extra: serde_json::Map::new(),
        }
    }

    /// Ellipse via the standard 4-vertex cubic approximation (kappa).
    pub fn ellipse(cx: f64, cy: f64, rx: f64, ry: f64) -> Self {
        const K: f64 = 0.552_284_749_830_793_4;
        let v = |px: f64, py: f64, tin: (f64, f64), tout: (f64, f64)| Vertex {
            pos: (px, py),
            tan_in: tin,
            tan_out: tout,
        };
        Self {
            id: Uuid::now_v7(),
            name: "Ellipse".into(),
            path: BezierPath {
                vertices: vec![
                    v((cx, cy - ry).0, cy - ry, (-rx * K, 0.0), (rx * K, 0.0)),
                    v(cx + rx, cy, (0.0, -ry * K), (0.0, ry * K)),
                    v(cx, cy + ry, (rx * K, 0.0), (-rx * K, 0.0)),
                    v(cx - rx, cy, (0.0, ry * K), (0.0, -ry * K)),
                ],
                closed: true,
            },
            inverted: false,
            opacity: 100.0,
            extra: serde_json::Map::new(),
        }
    }
}

/// Rasterise a closed path to 0..255 coverage at `w`×`h`, with the path's
/// layer-pixel coordinates scaled by (`sx`, `sy`) — pass the texture/natural
/// ratio so reduced-resolution decodes mask correctly. Even-odd fill,
/// fractional-span horizontal AA, two vertical subsamples.
pub fn rasterise(path: &BezierPath, w: u32, h: u32, sx: f64, sy: f64) -> Vec<u8> {
    let mut coverage = vec![0u8; (w * h) as usize];
    if path.vertices.len() < 3 || !path.closed {
        return coverage;
    }

    // Flatten cubics to polyline edges (fixed subdivision — paths are UI-drawn
    // and small; adaptive flattening arrives with the pen tool if needed).
    const SEGS: usize = 24;
    let n = path.vertices.len();
    let mut points: Vec<(f64, f64)> = Vec::with_capacity(n * SEGS);
    for i in 0..n {
        let a = &path.vertices[i];
        let b = &path.vertices[(i + 1) % n];
        let p0 = (a.pos.0 * sx, a.pos.1 * sy);
        let p1 = ((a.pos.0 + a.tan_out.0) * sx, (a.pos.1 + a.tan_out.1) * sy);
        let p2 = ((b.pos.0 + b.tan_in.0) * sx, (b.pos.1 + b.tan_in.1) * sy);
        let p3 = (b.pos.0 * sx, b.pos.1 * sy);
        for s in 0..SEGS {
            let t = s as f64 / SEGS as f64;
            let u = 1.0 - t;
            let x = u * u * u * p0.0
                + 3.0 * u * u * t * p1.0
                + 3.0 * u * t * t * p2.0
                + t * t * t * p3.0;
            let y = u * u * u * p0.1
                + 3.0 * u * u * t * p1.1
                + 3.0 * u * t * t * p2.1
                + t * t * t * p3.1;
            points.push((x, y));
        }
    }

    // Scanline with two vertical subsamples per row.
    let mut xs: Vec<f64> = Vec::with_capacity(16);
    for row in 0..h {
        let mut row_cov = vec![0.0f32; w as usize];
        for sub in 0..2 {
            let y = f64::from(row) + 0.25 + 0.5 * f64::from(sub);
            xs.clear();
            for e in 0..points.len() {
                let (x0, y0) = points[e];
                let (x1, y1) = points[(e + 1) % points.len()];
                if (y0 <= y && y1 > y) || (y1 <= y && y0 > y) {
                    xs.push(x0 + (y - y0) / (y1 - y0) * (x1 - x0));
                }
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            for pair in xs.chunks_exact(2) {
                let (start, end) = (pair[0].max(0.0), pair[1].min(f64::from(w)));
                if end <= start {
                    continue;
                }
                let (first, last) = (
                    start.floor() as usize,
                    (end.ceil() as usize).min(w as usize),
                );
                for (px, cell) in row_cov.iter_mut().enumerate().take(last).skip(first) {
                    let l = start.max(px as f64);
                    let r = end.min(px as f64 + 1.0);
                    if r > l {
                        *cell += ((r - l) * 0.5) as f32;
                    }
                }
            }
        }
        let base = (row * w) as usize;
        for (px, c) in row_cov.iter().enumerate() {
            coverage[base + px] = (c.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    coverage
}

/// Apply a layer's masks to straight-alpha RGBA8 pixels in place.
/// Multiple masks combine additively (clamped), then invert/opacity apply
/// per mask before combination — Phase 1 Add-mode semantics.
pub fn apply_masks(
    rgba: &mut [u8],
    w: u32,
    h: u32,
    natural_w: f64,
    natural_h: f64,
    masks: &[Mask],
) {
    if masks.is_empty() {
        return;
    }
    let total = combined_coverage(masks, w, h, natural_w, natural_h);
    for (px, t) in rgba.chunks_exact_mut(4).zip(total) {
        px[3] = ((u16::from(px[3]) * u16::from(t)) / 255) as u8;
    }
}

/// The combined 0..255 coverage of a mask stack at `w`×`h` (path coordinates
/// in `natural` space) — the same Add-mode maths [`apply_masks`] uses, exposed
/// so GPU-sourced layers (Precomps) can upload it as a texture instead of
/// editing pixels they don't have.
pub fn combined_coverage(
    masks: &[Mask],
    w: u32,
    h: u32,
    natural_w: f64,
    natural_h: f64,
) -> Vec<u8> {
    let sx = f64::from(w) / natural_w.max(1.0);
    let sy = f64::from(h) / natural_h.max(1.0);
    let mut total = vec![0u16; (w * h) as usize];
    for mask in masks {
        let cov = rasterise(&mask.path, w, h, sx, sy);
        let op = (mask.opacity.clamp(0.0, 100.0) / 100.0 * 255.0) as u16;
        for (t, c) in total.iter_mut().zip(cov) {
            let c = if mask.inverted {
                255 - u16::from(c)
            } else {
                u16::from(c)
            };
            *t = (*t + c * op / 255).min(255);
        }
    }
    total.into_iter().map(|t| t as u8).collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn rectangle_covers_exactly_its_area() {
        let m = Mask::rectangle(4.0, 4.0, 8.0, 8.0);
        let cov = rasterise(&m.path, 16, 16, 1.0, 1.0);
        assert_eq!(cov[(8 * 16 + 8) as usize], 255, "inside");
        assert_eq!(cov[(2 * 16 + 2) as usize], 0, "outside");
        let sum: f64 = cov.iter().map(|c| f64::from(*c) / 255.0).sum();
        assert!((sum - 64.0).abs() < 1.5, "area {sum} vs 64");
    }

    #[test]
    fn star_has_alternating_radii_and_closes() {
        let m = Mask::star(50.0, 50.0, 40.0, 16.0, 5);
        assert_eq!(m.path.vertices.len(), 10);
        assert!(m.path.closed);
        // Outer points sit ~40 from centre, inner ~16 — alternating.
        for (i, v) in m.path.vertices.iter().enumerate() {
            let r = ((v.pos.0 - 50.0).powi(2) + (v.pos.1 - 50.0).powi(2)).sqrt();
            let want = if i % 2 == 0 { 40.0 } else { 16.0 };
            assert!((r - want).abs() < 1e-9, "vertex {i} radius {r} vs {want}");
        }
        // First outer point is at the top (y < centre).
        assert!(m.path.vertices[0].pos.1 < 50.0);
        // Rasterises to a sensible non-zero, sub-bounding-box area.
        let cov = rasterise(&m.path, 100, 100, 1.0, 1.0);
        let sum: f64 = cov.iter().map(|c| f64::from(*c) / 255.0).sum();
        assert!(sum > 500.0 && sum < 5000.0, "star area {sum}");
    }

    #[test]
    fn ellipse_area_matches_pi_r_squared() {
        let m = Mask::ellipse(32.0, 32.0, 20.0, 20.0);
        let cov = rasterise(&m.path, 64, 64, 1.0, 1.0);
        let sum: f64 = cov.iter().map(|c| f64::from(*c) / 255.0).sum();
        let expect = std::f64::consts::PI * 20.0 * 20.0;
        assert!(
            (sum - expect).abs() / expect < 0.01,
            "area {sum} vs {expect}"
        );
    }

    #[test]
    fn scaled_rasterisation_masks_reduced_decodes_correctly() {
        // Path in natural 100×100 space, rasterised for a 50×50 decode.
        let m = Mask::rectangle(0.0, 0.0, 50.0, 100.0); // left half
        let cov = rasterise(&m.path, 50, 50, 0.5, 0.5);
        assert_eq!(cov[(25 * 50 + 10) as usize], 255, "left in");
        assert_eq!(cov[(25 * 50 + 40) as usize], 0, "right out");
    }

    #[test]
    fn apply_masks_gates_alpha_with_invert_and_opacity() {
        let m = Mask::rectangle(0.0, 0.0, 2.0, 4.0); // left half of 4×4
        let mut rgba = vec![255u8; 4 * 4 * 4];
        apply_masks(&mut rgba, 4, 4, 4.0, 4.0, std::slice::from_ref(&m));
        assert_eq!(rgba[4 * 4 + 3], 255, "left opaque");
        assert_eq!(rgba[(4 + 3) * 4 + 3], 0, "right transparent");

        let mut inv = m.clone();
        inv.inverted = true;
        inv.opacity = 50.0;
        let mut rgba = vec![255u8; 4 * 4 * 4];
        apply_masks(&mut rgba, 4, 4, 4.0, 4.0, &[inv]);
        assert_eq!(rgba[4 * 4 + 3], 0, "inverted left transparent");
        let right = rgba[(4 + 3) * 4 + 3];
        assert!((i16::from(right) - 127).abs() <= 2, "half opacity {right}");
    }
}
