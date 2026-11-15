//! A Shape row's points: a closed outline drawn inside a box, such as a
//! Correction window's polygon.
//!
//! **In plain terms.** The points are where the user clicked, in the box's own
//! space: `-1, -1` is its top-left corner and `1, 1` its bottom-right. The box
//! itself moves, grows and turns with the rows beside it, so the outline
//! follows them without a key on any point. That is also why the points never
//! animate: a list that grows and shrinks has nothing to interpolate.

/// The most points a Shape row holds.
pub const SHAPE_MAX_POINTS: usize = 32;

/// One Shape row's points, in order round the outline.
///
/// Fixed-size and `Copy` so it can ride in the resolved parameters the way a
/// curve does. Only the first `len` entries mean anything and the rest are
/// zero, so two equal shapes compare and hash alike.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapePoints {
    xy: [[f32; 2]; SHAPE_MAX_POINTS],
    len: u32,
}

// No wider than a curve, so a Shape left every arena slot the size it was.
const _: () = assert!(size_of::<ShapePoints>() <= size_of::<super::params::CurvePoints>());

impl ShapePoints {
    /// No points at all.
    pub const EMPTY: Self = Self {
        xy: [[0.0; 2]; SHAPE_MAX_POINTS],
        len: 0,
    };

    /// Read a stored point list: a point that is not a number is dropped,
    /// the rest are held inside the box, and only the first
    /// [`SHAPE_MAX_POINTS`] are kept. The order is the outline's, so it is
    /// never sorted.
    #[must_use]
    pub fn sanitised(points: &[[f32; 2]]) -> Self {
        let mut out = Self::EMPTY;
        let kept = points
            .iter()
            .filter(|p| p[0].is_finite() && p[1].is_finite())
            .take(SHAPE_MAX_POINTS);
        for (slot, p) in out.xy.iter_mut().zip(kept) {
            *slot = [p[0].clamp(-1.0, 1.0), p[1].clamp(-1.0, 1.0)];
            out.len += 1;
        }
        out
    }

    /// The points, in order.
    #[must_use]
    pub fn points(&self) -> &[[f32; 2]] {
        self.xy.get(..self.len as usize).unwrap_or(&[])
    }

    /// How many points there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for ShapePoints {
    fn default() -> Self {
        Self::EMPTY
    }
}
