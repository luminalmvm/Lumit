//! Where a restyle leaves a text layer's anchor, so its words stay put.

/// How far the anchor has to move for `after` to sit where `before` sat. The
/// live preview and the committed write both ask here, so a value being
/// dragged draws what letting go of it will write.
#[must_use]
pub fn shift(
    before: &lumit_core::model::TextDocument,
    after: &lumit_core::model::TextDocument,
) -> (f64, f64) {
    // A line on a path already has its room and its corner at the layer's
    // origin, so nothing there moves.
    let straight = before.path.is_none() && after.path.is_none();
    let boxed = |d: &lumit_core::model::TextDocument| {
        d.animators
            .iter()
            .any(lumit_core::text::TextAnimator::has_box)
    };
    // Retyping the words alone leaves the anchor where it is, unless they are
    // point text lined up on the centre or the right, which grows from there.
    // Words an expression gives are not the ones typed, so they move nothing.
    // A box an animator adds pads the block, so it moves the words too.
    let retyped = before.text != after.text
        && before.expression.is_none()
        && after.expression.is_none()
        && after.paragraph.align != lumit_core::text::TextAlign::Left
        && after.paragraph.box_width <= 0.0;
    let restyled = before.size != after.size
        || before.style != after.style
        || before.paragraph != after.paragraph
        || before.animators.is_empty() != after.animators.is_empty()
        || boxed(before) != boxed(after)
        || retyped;
    if !(straight && restyled) {
        return (0.0, 0.0);
    }
    let (was, now) = (reference_point(before), reference_point(after));
    (now.0 - was.0, now.1 - was.1)
}

/// The point of a block that a restyle leaves where it is: the first
/// baseline, at the side the lines line up on. In the layer's own pixels.
fn reference_point(document: &lumit_core::model::TextDocument) -> (f64, f64) {
    use lumit_core::text::TextAlign;
    let l = crate::layout(
        &crate::TextBlock::of(document, &document.text),
        !document.animators.is_empty(),
    );
    let x = match document.paragraph.align {
        TextAlign::Left => l.left,
        TextAlign::Centre => (l.left + l.right) * 0.5,
        TextAlign::Right => l.right,
    };
    let y = l.lines.first().map_or(0.0, |line| line.baseline);
    (f64::from(x), f64::from(y))
}
