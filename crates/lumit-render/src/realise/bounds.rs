//! The pixel boxes a realised layer covers.

use super::*;

impl Bounds {
    pub(super) fn frame(w: u32, h: u32) -> Self {
        Self { x: 0, y: 0, w, h }
    }

    /// `[left, top, right, bottom]`, as [`lumit_core::fx::registry::EffectDef::bounds`]
    /// takes it.
    pub(super) fn rect(self) -> [f32; 4] {
        [
            self.x as f32,
            self.y as f32,
            self.x as f32 + self.w as f32,
            self.y as f32 + self.h as f32,
        ]
    }

    /// How far past an `n`-pixel edge a box may keep: `frames` frames (0 for
    /// no limit of its own), and never past the largest picture. Whether it is
    /// the largest picture that stops it.
    pub(super) fn room(n: u32, frames: u32) -> (f32, bool) {
        let frames = if frames == 0 { u32::MAX } else { frames };
        let (asked, most) = (
            n.saturating_mul(frames),
            lumit_core::largest_picture::side().saturating_sub(n) / 2,
        );
        (asked.min(most) as f32, most < asked)
    }

    /// The whole pixels that cover `rect` and the `w × h` frame, keeping at
    /// most `frames` frames past each edge (0 for no limit of its own) and
    /// never more than the largest picture.
    pub(super) fn covering(rect: [f32; 4], w: u32, h: u32, frames: u32) -> Self {
        let ((rx, cap_x), (ry, cap_y)) = (Self::room(w, frames), Self::room(h, frames));
        // Cut by the largest picture rather than by the overscan asked for.
        if (cap_x && (rect[0] < -rx || rect[2] > w as f32 + rx))
            || (cap_y && (rect[1] < -ry || rect[3] > h as f32 + ry))
        {
            lumit_core::largest_picture::note_cut();
        }
        // Nothing sensible to cover reads as the frame's own edge.
        let low = |v: f32, room: f32| {
            if v.is_nan() {
                0.0
            } else {
                v.floor().clamp(-room, 0.0)
            }
        };
        let high = |v: f32, n: u32, room: f32| {
            if v.is_nan() {
                n as f32
            } else {
                v.ceil().clamp(n as f32, n as f32 + room)
            }
        };
        let (x0, y0) = (low(rect[0], rx), low(rect[1], ry));
        let (x1, y1) = (high(rect[2], w, rx), high(rect[3], h, ry));
        Self {
            x: x0 as i32,
            y: y0 as i32,
            w: (x1 - x0) as u32,
            h: (y1 - y0) as u32,
        }
    }

    /// The most a box may keep round a `w × h` frame.
    pub(super) fn limit(w: u32, h: u32, frames: u32) -> Self {
        let (rx, ry) = (
            Self::room(w, frames).0 as u32,
            Self::room(h, frames).0 as u32,
        );
        Self {
            x: -(rx as i32),
            y: -(ry as i32),
            w: w + rx * 2,
            h: h + ry * 2,
        }
    }

    /// Whether this reaches the limit on any side the limit is past the
    /// frame.
    pub(super) fn at_limit(self, w: u32, h: u32, frames: u32) -> bool {
        let limit = Self::limit(w, h, frames);
        let [x0, y0, x1, y1] = self.rect();
        let [lx0, ly0, lx1, ly1] = limit.rect();
        (lx0 < 0.0 && x0 <= lx0)
            || (ly0 < 0.0 && y0 <= ly0)
            || (lx1 > w as f32 && x1 >= lx1)
            || (ly1 > h as f32 && y1 >= ly1)
    }

    pub(super) fn union(self, other: Self) -> Self {
        let (x0, y0) = (self.x.min(other.x), self.y.min(other.y));
        let x1 = (self.x + self.w as i32).max(other.x + other.w as i32);
        let y1 = (self.y + self.h as i32).max(other.y + other.h as i32);
        Self {
            x: x0,
            y: y0,
            w: (x1 - x0) as u32,
            h: (y1 - y0) as u32,
        }
    }

    /// This window of the frame's space, for the compositor.
    pub(super) fn region(self) -> lumit_gpu::composite::Region {
        lumit_gpu::composite::Region {
            x: self.x as f32,
            y: self.y as f32,
            w: self.w as f32,
            h: self.h as f32,
        }
    }
}
