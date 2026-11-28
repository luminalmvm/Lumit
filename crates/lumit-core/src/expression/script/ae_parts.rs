use crate::anim::Property;

/// Up to four properties (a colour's channels), then numbers that do not
/// animate: a checkbox or a menu.
pub(super) struct Parts<'a> {
    pub(super) props: [Option<&'a Property>; 4],
    pub(super) tail: [Option<f64>; 3],
}

impl<'a> Parts<'a> {
    pub(super) fn one(p: &'a Property) -> Self {
        Parts {
            props: [Some(p), None, None, None],
            tail: [None; 3],
        }
    }

    pub(super) fn two(x: &'a Property, y: &'a Property) -> Self {
        Parts {
            props: [Some(x), Some(y), None, None],
            tail: [None; 3],
        }
    }

    pub(super) fn fixed(value: f64) -> Self {
        Parts {
            props: [None; 4],
            tail: [Some(value), None, None],
        }
    }
}
