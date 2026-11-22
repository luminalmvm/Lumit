//! How far past their room a styled block's animated letters reach.

use lumit_core::text::GlyphXform;
use zeno::{Point, Transform};

use super::{animated, face_of, lay, respace, Run};
use crate::animated::{animator_margin, beyond, joined};
use crate::TextBlock;

/// [`crate::animated::overhang`] for a styled block. Each letter is taken as
/// its advance from the ascent to the descent, with the room an outline or a
/// slant is given round it, which is close enough to grow a raster by.
pub(crate) fn overhang(block: &TextBlock<'_>, xforms: &[GlyphXform]) -> [u32; 4] {
    let face = face_of(block);
    let mut laid = lay(block, &face);
    respace(&mut laid, &face, xforms, block);
    if laid.glyphs.is_empty() {
        return [0; 4];
    }
    let margin = animator_margin(laid.full);
    let run = Run::Straight {
        origin: laid.origin(margin),
    };
    let (w, h) = laid.raster_size(margin);
    let chars: Vec<char> = block.text.chars().collect();
    let pad = laid.pad.max(laid.pad_top).max(laid.pad_bottom);
    let (top, bottom) = (-laid.ascent - pad, laid.descent + pad);
    #[allow(clippy::cast_precision_loss)] // px
    let size = (w as f32, h as f32);
    let mut ink = [0.0, 0.0, size.0, size.1];
    for glyph in &laid.glyphs {
        let x = xforms.get(glyph.ch).copied().unwrap_or_default();
        let blank = !chars.get(glyph.ch).is_some_and(|c| !c.is_whitespace());
        let gone = !(x.opacity > 0.0 && x.scale[0].abs() >= 1e-4 && x.scale[1].abs() >= 1e-4);
        let Some(place) = run.place(&laid, glyph, 0.0).filter(|_| !(blank || gone)) else {
            continue;
        };
        let matrix = animated(Transform::translation(glyph.nudge, 0.0), glyph.advance, &x);
        let matrix = matrix.then(&place);
        let (x0, x1) = (-pad, glyph.advance + pad);
        for (cx, cy) in [(x0, top), (x1, top), (x0, bottom), (x1, bottom)] {
            let p = matrix.transform_point(Point::new(cx, cy));
            ink = joined(ink, [p.x, p.y, p.x, p.y]);
        }
    }
    beyond(ink, size)
}
