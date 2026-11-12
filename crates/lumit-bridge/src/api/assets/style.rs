//! A text style read from the model and written back, and where a restyle
//! leaves the anchor.

use flutter_rust_bridge::frb;

use super::{
    colour_of, linear_of, BridgeAxisSpacing, BridgeCaps, BridgeFontAxisValue, BridgeKerning,
    BridgeParagraphStyle, BridgeScript, BridgeTextAlign, BridgeTextStyle,
};

/// How far the anchor has to move for `after` to sit where `before` sat. The
/// live preview and the committed write both ask here, so a value being
/// dragged draws what letting go of it will write.
#[frb(ignore)]
pub(crate) fn restyle_shift(
    before: &lumit_core::model::TextDocument,
    after: &lumit_core::model::TextDocument,
) -> (f64, f64) {
    // A line on a path already has its room and its corner at the layer's
    // origin, so nothing there moves.
    let straight = before.path.is_none() && after.path.is_none();
    // Retyping the words alone leaves the anchor where it is.
    let restyled = before.size != after.size
        || before.style != after.style
        || before.paragraph != after.paragraph
        || before.animators.is_empty() != after.animators.is_empty();
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
    let l = lumit_text::layout(
        &lumit_text::TextBlock::of(document, &document.text),
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

pub(super) fn read_style(style: &lumit_core::text::TextStyle) -> BridgeTextStyle {
    use lumit_core::text::{Caps, Kerning, Script};
    BridgeTextStyle {
        family: style.family.clone(),
        face: style.face.clone(),
        axes: style
            .axes
            .iter()
            .map(|a| BridgeFontAxisValue {
                tag: a.tag.clone(),
                value: a.value,
            })
            .collect(),
        axis_spacing: match style.axis_spacing {
            lumit_core::text::AxisSpacing::Adaptive => BridgeAxisSpacing::Adaptive,
            lumit_core::text::AxisSpacing::PerCharacter => BridgeAxisSpacing::PerCharacter,
            lumit_core::text::AxisSpacing::Default => BridgeAxisSpacing::Default,
        },
        leading: style.leading,
        kerning: match style.kerning {
            Kerning::Off => BridgeKerning::Off,
            Kerning::Metrics => BridgeKerning::Metrics,
        },
        tracking: style.tracking,
        scale_x: style.scale_x,
        scale_y: style.scale_y,
        baseline_shift: style.baseline_shift,
        caps: match style.caps {
            Caps::Normal => BridgeCaps::Normal,
            Caps::All => BridgeCaps::All,
            Caps::Small => BridgeCaps::Small,
        },
        script: match style.script {
            Script::Normal => BridgeScript::Normal,
            Script::Superscript => BridgeScript::Superscript,
            Script::Subscript => BridgeScript::Subscript,
        },
        faux_bold: style.faux_bold,
        faux_italic: style.faux_italic,
        ligatures: style.ligatures,
        fill_on: style.fill_on,
        stroke_on: style.stroke_on,
        stroke: colour_of(style.stroke),
        stroke_width: style.stroke_width,
        stroke_over: style.stroke_over,
        strokes_together: style.strokes_together,
    }
}

pub(super) fn style_of(style: BridgeTextStyle) -> lumit_core::text::TextStyle {
    use lumit_core::text::{Caps, Kerning, Script};
    lumit_core::text::TextStyle {
        family: style.family,
        face: style.face,
        axes: style
            .axes
            .into_iter()
            .filter(|a| a.value.is_finite() && lumit_core::text::axis_tag(&a.tag).is_some())
            .map(|a| lumit_core::text::FontAxisValue {
                tag: a.tag,
                value: a.value,
            })
            .collect(),
        axis_spacing: match style.axis_spacing {
            BridgeAxisSpacing::Adaptive => lumit_core::text::AxisSpacing::Adaptive,
            BridgeAxisSpacing::PerCharacter => lumit_core::text::AxisSpacing::PerCharacter,
            BridgeAxisSpacing::Default => lumit_core::text::AxisSpacing::Default,
        },
        leading: style.leading.filter(|l| l.is_finite() && *l >= 0.0),
        kerning: match style.kerning {
            BridgeKerning::Off => Kerning::Off,
            BridgeKerning::Metrics => Kerning::Metrics,
        },
        tracking: style.tracking,
        scale_x: style.scale_x,
        scale_y: style.scale_y,
        baseline_shift: style.baseline_shift,
        caps: match style.caps {
            BridgeCaps::Normal => Caps::Normal,
            BridgeCaps::All => Caps::All,
            BridgeCaps::Small => Caps::Small,
        },
        script: match style.script {
            BridgeScript::Normal => Script::Normal,
            BridgeScript::Superscript => Script::Superscript,
            BridgeScript::Subscript => Script::Subscript,
        },
        faux_bold: style.faux_bold,
        faux_italic: style.faux_italic,
        ligatures: style.ligatures,
        fill_on: style.fill_on,
        stroke_on: style.stroke_on,
        stroke: linear_of(style.stroke),
        stroke_width: style.stroke_width,
        stroke_over: style.stroke_over,
        strokes_together: style.strokes_together,
        extra: serde_json::Map::new(),
    }
}

pub(super) fn read_paragraph(paragraph: &lumit_core::text::ParagraphStyle) -> BridgeParagraphStyle {
    use lumit_core::text::TextAlign;
    BridgeParagraphStyle {
        align: match paragraph.align {
            TextAlign::Left => BridgeTextAlign::Left,
            TextAlign::Centre => BridgeTextAlign::Centre,
            TextAlign::Right => BridgeTextAlign::Right,
        },
        box_width: Some(paragraph.box_width).filter(|w| *w > 0.0),
        justify: paragraph.justify,
        indent_left: paragraph.indent_left,
        indent_right: paragraph.indent_right,
        indent_first: paragraph.indent_first,
        space_before: paragraph.space_before,
        space_after: paragraph.space_after,
    }
}

pub(super) fn paragraph_of(paragraph: BridgeParagraphStyle) -> lumit_core::text::ParagraphStyle {
    use lumit_core::text::TextAlign;
    lumit_core::text::ParagraphStyle {
        align: match paragraph.align {
            BridgeTextAlign::Left => TextAlign::Left,
            BridgeTextAlign::Centre => TextAlign::Centre,
            BridgeTextAlign::Right => TextAlign::Right,
        },
        box_width: paragraph
            .box_width
            .filter(|w| w.is_finite() && *w > 0.0)
            .unwrap_or(0.0),
        justify: paragraph.justify,
        indent_left: paragraph.indent_left,
        indent_right: paragraph.indent_right,
        indent_first: paragraph.indent_first,
        space_before: paragraph.space_before,
        space_after: paragraph.space_after,
        extra: serde_json::Map::new(),
    }
}
