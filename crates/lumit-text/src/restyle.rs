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

#[cfg(test)]
mod tests {
    use super::shift;
    use lumit_core::anim::Property;
    use lumit_core::model::{LinearColour, TextDocument};
    use lumit_core::text::{ParagraphStyle, TextAlign, TextAnimator};

    fn document(text: &str, size: f64, align: TextAlign) -> TextDocument {
        TextDocument {
            text: text.into(),
            expression: None,
            keys: None,
            size,
            fill: LinearColour([1.0; 4]),
            path: None,
            path_offset: Property::zero(),
            animators: Vec::new(),
            style: Box::default(),
            paragraph: ParagraphStyle {
                align,
                ..ParagraphStyle::default()
            },
            captions: None,
            extra: Default::default(),
        }
    }

    /// Nothing changed, nothing moves; retyped left-aligned words grow from
    /// where they start, so they don't move either.
    #[test]
    fn the_anchor_stays_when_nothing_needs_it_to_move() {
        let line = document("Hello", 48.0, TextAlign::Left);
        assert_eq!(shift(&line, &line), (0.0, 0.0));
        let retyped = document("Hello there", 48.0, TextAlign::Left);
        assert_eq!(shift(&line, &retyped), (0.0, 0.0));
        let mut spoken = document("Hello", 48.0, TextAlign::Centre);
        spoken.expression = Some("\"Hi\"".into());
        let mut respoken = spoken.clone();
        respoken.text = "Hello there".into();
        assert_eq!(shift(&spoken, &respoken), (0.0, 0.0));
    }

    /// A bigger size, centred words retyped, or a first animator's padding
    /// each move the anchor so the words stay where they were.
    #[test]
    fn the_anchor_follows_a_restyle() {
        let centred = document("Hello", 48.0, TextAlign::Centre);
        let bigger = document("Hello", 96.0, TextAlign::Centre);
        let (x, y) = shift(&centred, &bigger);
        assert!(x > 0.0 && y > 0.0, "{x} {y}");
        let retyped = document("Hello there", 48.0, TextAlign::Centre);
        let (x, y) = shift(&centred, &retyped);
        assert!(x > 0.0 && y.abs() < 1e-6, "{x} {y}");
        let mut animated = centred.clone();
        animated.animators.push(TextAnimator::new("Animator 1"));
        let (x, y) = shift(&centred, &animated);
        assert!(x > 0.0 && y > 0.0, "{x} {y}");
    }
}
