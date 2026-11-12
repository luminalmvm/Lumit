//! Letters set at their own place on a variable font's axes.

use lumit_core::text::{AxisSpacing, GlyphXform, TextAlign};

use super::Laid;
use crate::fonts::Face;
use crate::TextBlock;

/// The face a block names, at the layer's own place on its axes.
pub(super) fn face_of(block: &TextBlock<'_>) -> std::sync::Arc<Face> {
    let face = crate::fonts::face(&block.style.family, &block.style.face);
    if block.style.axes.is_empty() {
        face
    } else {
        std::sync::Arc::new(face.with_axes(&block.style.axes))
    }
}

/// How close two places on an axis may be and still share one font: a
/// hundredth of a unit, far finer than an eye can tell apart.
fn place_key(moves: &[([u8; 4], f32)]) -> Vec<([u8; 4], i32)> {
    #[allow(clippy::cast_possible_truncation)] // a place on an axis, far inside i32
    moves
        .iter()
        .map(|(tag, v)| (*tag, (v * 100.0).round() as i32))
        .collect()
}

/// **Letters set at their own place on the font's axes.** Each letter an
/// animator moves on an axis is given that place, and the room it takes
/// changes with it: a bold letter is wider than a light one. How the line
/// makes that room is the layer's Variable font spacing.
///
/// The words are shaped at the layer's own place, so kerning and the breaks
/// of a wrapped paragraph are the layer's, and only the letters' own widths
/// change here.
pub(super) fn respace(laid: &mut Laid, face: &Face, xforms: &[GlyphXform], block: &TextBlock<'_>) {
    if xforms.iter().all(|x| x.axes.is_empty()) {
        return;
    }
    // The shaper's own advances, which a font without an advance table still
    // varies through its outlines' end points, the way it shaped the line.
    let base_shaper = harfrust::ShaperFont::new(&face.shaper);
    let mut keys: Vec<Vec<([u8; 4], i32)>> = Vec::new();
    // The width each letter gains, by its place in `laid.glyphs`.
    let mut gains = vec![0.0f32; laid.glyphs.len()];
    let base = |tag: [u8; 4]| face.place_on(tag).unwrap_or(f32::NAN);
    for (g, glyph) in laid.glyphs.iter_mut().enumerate() {
        let Some(x) = xforms.get(glyph.ch) else {
            continue;
        };
        let moves: Vec<([u8; 4], f32)> = x
            .axes
            .resolved(&base)
            .filter(|(_, v)| v.is_finite())
            .collect();
        if moves.is_empty() {
            continue;
        }
        let key = place_key(&moves);
        let place = if let Some(i) = keys.iter().position(|k| *k == key) {
            i
        } else {
            keys.push(key);
            laid.places.push(face.varied(&moves));
            laid.places.len() - 1
        };
        glyph.place = Some(place);
        let Some(varied) = laid.places.get(place) else {
            continue;
        };
        let id = harfrust::GlyphId::new(glyph.glyph);
        let now = harfrust::ShaperFont::new(varied).glyph_h_advance(id);
        let was = base_shaper.glyph_h_advance(id);
        if let Some(gain) = gains.get_mut(g) {
            #[allow(clippy::cast_precision_loss)] // font units, far inside f32
            {
                *gain = (now - was) as f32 * glyph.scale * laid.scale_x;
            }
        }
    }
    match block.style.axis_spacing {
        AxisSpacing::PerCharacter => {
            for (glyph, gain) in laid.glyphs.iter_mut().zip(&gains) {
                glyph.nudge = -gain * 0.5;
            }
        }
        spacing @ (AxisSpacing::Adaptive | AxisSpacing::Default) => {
            // Line by line, every letter is pushed on by what the letters
            // before it on the line gained.
            let mut pushed = vec![0.0f32; laid.lines.len().max(1)];
            for (glyph, gain) in laid.glyphs.iter_mut().zip(&gains) {
                let Some(push) = pushed.get_mut(glyph.line) else {
                    continue;
                };
                glyph.x += *push;
                glyph.advance += gain;
                *push += gain;
            }
            // Adaptive lines each line up again by what it gained in all.
            if spacing == AxisSpacing::Adaptive {
                let share = match block.paragraph.align {
                    TextAlign::Left => 0.0,
                    TextAlign::Centre => 0.5,
                    TextAlign::Right => 1.0,
                };
                for glyph in &mut laid.glyphs {
                    if let Some(gain) = pushed.get(glyph.line) {
                        glyph.x -= gain * share;
                    }
                }
            }
        }
    }
}
