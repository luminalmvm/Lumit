//! Where a grown picture goes: its new size and how far its anchor slides.

/// The quad a grown picture needs (docs/08 §3.39).
///
/// **In plain terms.** Tile with Output width or height above 100 % hands back a
/// picture wider than the layer it came from, grown evenly on all four sides.
/// The layer's placement is expressed as a rectangle (`size`) with a pin in it
/// (`anchor`); to draw the wider picture in the same place, the rectangle grows
/// by the same factor and the pin slides to stay over the same pixel. Every
/// layer whose stack grew nothing has a factor of exactly 1, and both functions
/// then return what they were given, to the bit.
pub(super) fn grow_size(natural: (f32, f32), grow: (f32, f32)) -> (f32, f32) {
    (natural.0 * grow.0, natural.1 * grow.1)
}

/// How far the pin slides: half the width the growth added, on each axis.
/// See [`grow_size`].
pub(super) fn grow_offset(natural: (f32, f32), grow: (f32, f32)) -> (f32, f32) {
    (
        natural.0 * (grow.0 - 1.0) * 0.5,
        natural.1 * (grow.1 - 1.0) * 0.5,
    )
}
