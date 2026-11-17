//! Stabilise's warp on the card (`fx_stabilise.wgsl`), and its Fill.

use crate::GpuContext;

use super::{work_texture, FxEngine};

/// One frame's warp in the raster being drawn, worked out by `lumit_core`,
/// which this crate does not see.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StabiliseOp {
    /// Output pixel to input pixel, pixel centres at half-pixels.
    pub map: [[f32; 3]; 3],
    /// The rectangle of the output that is kept, `[x0, y0, x1, y1]`.
    pub keep: [f32; 4],
    /// How the picture moved while the frame was read, `(α, β, vx, vy)`.
    pub motion: [f32; 4],
    /// The rolling shutter, `(r, axis, size, 0)`. A nought `r` is no term.
    pub readout: [f32; 4],
}

/// A filled frame in the raster being drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StabiliseFillOp {
    /// Output pixel to a pixel of the frame drawn.
    pub map: [[f32; 3]; 3],
    /// The part of every frame that is used, `[x0, y0, x1, y1]`.
    pub rect: [f32; 4],
    /// How far in from that edge a frame fades in, in pixels.
    pub feather: f32,
    /// How the frame drawn moved while it was read.
    pub motion: [f32; 4],
    /// The rolling shutter, as [`StabiliseOp::readout`].
    pub readout: [f32; 4],
}

/// One neighbour a fill lays: the decoded frame, the map from a pixel of the
/// frame drawn to the same point of it, and how it moved while it was read.
pub type Neighbour<'a> = (&'a wgpu::Texture, [[f32; 3]; 3], [f32; 4]);

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    m0: [f32; 4],
    m1: [f32; 4],
    m2: [f32; 4],
    keep: [f32; 4],
    n0: [f32; 4],
    n1: [f32; 4],
    n2: [f32; 4],
    rect: [f32; 4],
    fill: [f32; 4],
    motion: [f32; 4],
    readout: [f32; 4],
}

fn row(r: [f32; 3]) -> [f32; 4] {
    [r[0], r[1], r[2], 0.0]
}

impl FxEngine {
    /// Draw a linear working texture through one frame's warp, returning a new
    /// texture of the same size.
    pub fn stabilise(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &StabiliseOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-stabilise-out");
        self.dispatch(
            ctx,
            &self.stabilise,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&Params {
                m0: row(op.map[0]),
                m1: row(op.map[1]),
                m2: row(op.map[2]),
                keep: op.keep,
                motion: op.motion,
                readout: op.readout,
                ..bytemuck::Zeroable::zeroed()
            }),
        );
        out
    }

    /// Draw a filled frame: each neighbour laid through its map in the order
    /// given, farthest first, then the frame itself over them all. With no
    /// neighbours it is the frame alone, its edge feathered.
    #[allow(clippy::too_many_arguments)]
    pub fn stabilise_fill(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        neighbours: &[Neighbour<'_>],
        w: u32,
        h: u32,
        op: &StabiliseFillOp,
    ) -> wgpu::Texture {
        let same = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let mut acc: Option<wgpu::Texture> = None;
        for (frame, to_frame, motion) in neighbours
            .iter()
            .copied()
            .chain(std::iter::once((src, same, op.motion)))
        {
            let out = work_texture(ctx, w, h, "fx-stabilise-fill");
            self.dispatch(
                ctx,
                &self.stabilise_fill,
                frame,
                acc.as_ref().unwrap_or(frame),
                &out,
                w,
                h,
                bytemuck::bytes_of(&Params {
                    m0: row(op.map[0]),
                    m1: row(op.map[1]),
                    m2: row(op.map[2]),
                    keep: [0.0; 4],
                    n0: row(to_frame[0]),
                    n1: row(to_frame[1]),
                    n2: row(to_frame[2]),
                    rect: op.rect,
                    fill: [op.feather, f32::from(u8::from(acc.is_some())), 0.0, 0.0],
                    motion,
                    readout: op.readout,
                }),
            );
            acc = Some(out);
        }
        acc.unwrap_or_else(|| src.clone())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{StabiliseFillOp, StabiliseOp};
    use crate::fx::common::{readback_linear_f32, upload_linear_f32};
    use lumit_core::stabilise::fill::{lay, Layer, SAME_FRAME};
    use lumit_core::stabilise::warp::{warp, WarpParams};

    /// A picture with detail at every scale: soft ramps under sharp ripples,
    /// seen `dx` pixels to the right of where the first frame sees it.
    fn corpus(dx: i32, w: u32, h: u32) -> Vec<f32> {
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let (fx, fy) = ((x as i32 + dx) as f32, y as f32);
                let r = 0.5 + 0.4 * (fx * 0.7).sin() * (fy * 0.3).cos();
                let g = fx / w as f32;
                let b = 0.5 + 0.5 * ((fx + 2.0 * fy) * 0.45).sin();
                out.extend_from_slice(&[r, g, b, 1.0]);
            }
        }
        out
    }

    fn worst(a: &[f32], b: &[f32]) -> f32 {
        a.iter()
            .zip(b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0, f32::max)
    }

    /// The warp on the card agrees with its CPU reference on a slide, a turn,
    /// a lean and a rolling shutter taken out, gives the picture back for the
    /// identity, leaves the edges a move drags in transparent, keeps only
    /// Crop's frame, and is bit-stable.
    #[test]
    fn wgsl_stabilise_matches_the_cpu_oracle() {
        let Some(ctx) = crate::test_support::lease() else {
            crate::no_adapter();
            return;
        };
        let fx = ctx.fx();
        let (w, h) = (48u32, 32u32);
        let img = corpus(0, w, h);
        let tex = upload_linear_f32(&ctx, &img, w, h);
        let all = [0.0, 0.0, w as f32, h as f32];
        let (c, s) = ((2.0f32).to_radians().cos(), (2.0f32).to_radians().sin());
        let (cx, cy) = (24.0f32, 16.0f32);
        let turn = [
            [1.05 * c, -1.05 * s, cx - 1.05 * (c * cx - s * cy)],
            [1.05 * s, 1.05 * c, cy - 1.05 * (s * cx + c * cy)],
            [0.0, 0.0, 1.0],
        ];
        let cases = [
            (
                "identity",
                [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                all,
            ),
            (
                "slide",
                [[1.0, 0.0, 3.25], [0.0, 1.0, -1.5], [0.0, 0.0, 1.0]],
                all,
            ),
            ("turn", turn, all),
            (
                "lean",
                [
                    [1.0, 0.02, 0.5],
                    [-0.01, 0.98, 0.25],
                    [0.0004, -0.0003, 1.0],
                ],
                all,
            ),
            (
                "crop",
                [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                [6.0, 4.0, 42.0, 28.0],
            ),
            ("rolling", turn, all),
        ];
        // A camera panning and turning while a read top to bottom takes 0.8 of
        // a frame: each row is moved by a different amount.
        let rolling = ([0.01, 0.02, 4.0, -2.5], [0.8, 1.0, h as f32, 0.0]);
        for (name, map, keep) in cases {
            let (motion, readout) = if name == "rolling" {
                rolling
            } else {
                ([0.0; 4], [0.0; 4])
            };
            let wp = WarpParams {
                map,
                keep,
                motion,
                readout,
            };
            let op = StabiliseOp {
                map,
                keep,
                motion,
                readout,
            };
            let cpu = warp(&img, w, h, &wp);
            let out = fx.stabilise(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
            let d = worst(&cpu, &gpu);
            assert!(d < 2e-3, "{name}: worst diff {d}");
            match name {
                "identity" => assert!(
                    worst(&gpu, &img) < 1e-3,
                    "the identity gives the picture back"
                ),
                // Output pixel x shows input x + 3.25: the rightmost columns
                // read past the input and are transparent.
                "slide" => assert_eq!(gpu[((5 * w + (w - 1)) * 4 + 3) as usize], 0.0),
                "crop" => {
                    assert_eq!(
                        gpu[((2 * w + 2) * 4 + 3) as usize],
                        0.0,
                        "outside the frame is cut"
                    );
                    assert!(
                        gpu[((16 * w + 24) * 4 + 3) as usize] > 0.99,
                        "inside is kept"
                    );
                }
                "rolling" => {
                    let plain = warp(
                        &img,
                        w,
                        h,
                        &WarpParams {
                            readout: [0.0; 4],
                            ..wp
                        },
                    );
                    assert!(worst(&gpu, &plain) > 0.05, "the rows are moved");
                }
                _ => assert!(gpu != img, "{name}: the warp moves something"),
            }
            let again = fx.stabilise(&ctx, &tex, w, h, &op);
            assert_eq!(
                gpu,
                readback_linear_f32(&ctx, &again, w, h).unwrap(),
                "{name}: bit-stable"
            );
        }
    }

    /// Fill: the strip a slide drags into view comes from the neighbour, from
    /// the right place of it; the interior is the frame's own, as the plain
    /// warp draws it; what no frame reaches stays empty; and the card agrees
    /// with the CPU reference with a feather and trims.
    #[test]
    fn wgsl_stabilise_fill_takes_the_edge_from_the_neighbour() {
        let Some(ctx) = crate::test_support::lease() else {
            crate::no_adapter();
            return;
        };
        let fx = ctx.fx();
        let (w, h) = (48u32, 32u32);
        let all = [0.0, 0.0, w as f32, h as f32];
        // The camera panned ten pixels right: a point at x on this frame is at
        // x - 10 on the next.
        let (now, next) = (corpus(0, w, h), corpus(10, w, h));
        let (t_now, t_next) = (
            upload_linear_f32(&ctx, &now, w, h),
            upload_linear_f32(&ctx, &next, w, h),
        );
        let to_next = [[1.0, 0.0, -10.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        // Output pixels show this frame six pixels right and three up: the
        // right six columns and the top three rows are past its edges.
        let map = [[1.0, 0.0, 6.0], [0.0, 1.0, -3.0], [0.0, 0.0, 1.0]];
        let px = |img: &[f32], x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            img[i..i + 4].to_vec()
        };
        let still = [0.0f32; 4];
        let op = StabiliseFillOp {
            map,
            rect: all,
            feather: 0.0,
            motion: still,
            readout: still,
        };
        let out = fx.stabilise_fill(&ctx, &t_now, &[(&t_next, to_next, still)], w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let plain = StabiliseOp {
            map,
            keep: all,
            motion: still,
            readout: still,
        };
        let plain = fx.stabilise(&ctx, &t_now, w, h, &plain);
        let plain = readback_linear_f32(&ctx, &plain, w, h).unwrap();
        for y in 3..h {
            for x in 0..w {
                let got = px(&gpu, x, y);
                let want = if x < w - 6 {
                    assert_eq!(got, px(&plain, x, y), "({x}, {y}) is the frame's own");
                    px(&now, x + 6, y - 3)
                } else {
                    px(&next, x - 4, y - 3)
                };
                assert!(
                    worst(&got, &want) < 2e-3,
                    "({x}, {y}): {got:?} against {want:?}"
                );
            }
        }
        for x in 0..w {
            assert_eq!(px(&gpu, x, 1)[3], 0.0, "nothing reaches above the frame");
        }
        // With a rolling shutter taken out, each frame by its own motion.
        let (m_now, m_next) = ([0.01, -0.02, 3.0, 1.5], [-0.01, 0.015, 5.0, -1.0]);
        let readout = [0.7, 1.0, h as f32, 0.0];
        for (feather, rect, readout) in [
            (0.0, all, still),
            (4.0, [1.0, 2.0, w as f32 - 2.0, h as f32 - 1.0], still),
            (4.0, [1.0, 2.0, w as f32 - 2.0, h as f32 - 1.0], readout),
        ] {
            let layer = |to_frame, motion| Layer {
                map,
                to_frame,
                rect,
                feather,
                motion,
                readout,
            };
            let under = lay(&next, None, w, h, &layer(to_next, m_next));
            let cpu = lay(&now, Some(&under), w, h, &layer(SAME_FRAME, m_now));
            let op = StabiliseFillOp {
                map,
                rect,
                feather,
                motion: m_now,
                readout,
            };
            let laid = [(&t_next, to_next, m_next)];
            let out = fx.stabilise_fill(&ctx, &t_now, &laid, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
            let d = worst(&cpu, &gpu);
            assert!(
                d < 2e-3,
                "feather {feather}, readout {readout:?}: worst diff {d}"
            );
        }
        // A picture with transparency: where its frame covers, it stays
        // transparent, and the neighbour shows only past its edge.
        let mut holed = now.clone();
        for (i, px) in holed.chunks_mut(4).enumerate() {
            if (i as u32 % w) < 20 {
                px.fill(0.0);
            }
        }
        let t_holed = upload_linear_f32(&ctx, &holed, w, h);
        let layer = |to_frame| Layer {
            map,
            to_frame,
            rect: all,
            feather: 4.0,
            motion: still,
            readout: still,
        };
        let under = lay(&next, None, w, h, &layer(to_next));
        let cpu = lay(&holed, Some(&under), w, h, &layer(SAME_FRAME));
        let op = StabiliseFillOp {
            map,
            rect: all,
            feather: 4.0,
            motion: still,
            readout: still,
        };
        let out = fx.stabilise_fill(&ctx, &t_holed, &[(&t_next, to_next, still)], w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        assert!(
            worst(&cpu, &gpu) < 2e-3,
            "transparency: worst diff {}",
            worst(&cpu, &gpu)
        );
        assert_eq!(px(&gpu, 8, 16), vec![0.0; 4], "the hole stays transparent");
    }
}
