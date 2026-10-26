//! Text layers: the words and how they are set, and where After Effects'
//! origin for them falls inside the raster Lumit lays them into.

use lumit_core::model::TextDocument;

use crate::capture::Property;
use crate::report::{ItemPath, Outcome, Reason};

use super::props::{axis_of, child, display_name, group, match_name_of, still};
use super::{linear_colour, Conv};

/// The Source Text value as the script wrote it, or null when there is none.
pub(super) fn source_text(props: &[Property]) -> &serde_json::Value {
    child(group(props, "ADBE Text Properties"), "ADBE Text Document")
        .and_then(|node| node.value.as_ref())
        .unwrap_or(&serde_json::Value::Null)
}

/// The words and what the script can read of how they are set: the font, the
/// spacing, the outline and the alignment. What Lumit's text has no place for
/// is reported, and stays in the bundle.
pub(super) fn text(conv: &mut Conv<'_>, path: &ItemPath, props: &[Property]) -> TextDocument {
    use lumit_core::text::{Caps, Kerning, ParagraphStyle, Script, TextAlign, TextStyle};

    let parts = group(props, "ADBE Text Properties");
    let node = child(parts, "ADBE Text Document");
    // The project file's own reader cannot decode a text document yet.
    if let Some(node) = node.filter(|node| node.unreadable.is_some()) {
        conv.report.row(
            path.property(display_name(node, "Source Text")),
            Outcome::Skipped,
            Reason::PropertyUnreadable {
                match_name: match_name_of(node).to_string(),
            },
        );
    }
    let doc = source_text(props);
    let number = |key: &str| doc.get(key).and_then(serde_json::Value::as_f64);
    let on = |key: &str| doc.get(key).and_then(serde_json::Value::as_bool);
    let word = |key: &str| {
        doc.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let colour = |key: &str| {
        let c: Vec<f64> = (0..3).filter_map(|i| axis_of(doc.get(key)?, i)).collect();
        (c.len() == 3).then(|| linear_colour(Some(&c)))
    };

    // ParagraphJustification by After Effects' own numbers, which is how the
    // script writes it. 7413 is left. The four from 7416 justify a box and say
    // where its last line goes.
    let justification = doc.get("justification").and_then(serde_json::Value::as_i64);
    let (align, justify) = match justification {
        Some(7414) => (TextAlign::Right, false),
        Some(7415) => (TextAlign::Centre, false),
        Some(7416) => (TextAlign::Left, true),
        Some(7417) => (TextAlign::Right, true),
        Some(7418) => (TextAlign::Centre, true),
        // 7419 stretches the last line too, which Lumit leaves alone.
        Some(7419) => (TextAlign::Left, true),
        _ => (TextAlign::Left, false),
    };

    let changes = node.is_some_and(|node| {
        node.keyframes.as_deref().is_some_and(|k| !k.is_empty())
            || (node.expression_enabled == Some(true)
                && node
                    .expression
                    .as_deref()
                    .is_some_and(|s| !s.trim().is_empty()))
    });
    if changes
        || !group(parts, "ADBE Text Animators").is_empty()
        || still(parts, "ADBE Text Path", 0).is_some_and(|mask| mask != 0.0)
        || number("tsume").is_some_and(|tsume| tsume != 0.0)
        || !matches!(justification, None | Some(7413..=7418))
    {
        conv.report.row(
            path.clone(),
            Outcome::Adjusted,
            Reason::TextStylingNotMapped,
        );
    }

    // A project file names its font by PostScript name alone, where the
    // script writes the family and the face as well.
    let (family, face) = match word("fontFamily") {
        family if !family.is_empty() => (family, word("fontStyle")),
        _ => lumit_text::fonts::by_postscript_name(&word("font")).unwrap_or_default(),
    };

    // More Options' Fill & Stroke: 2 is All fills over all strokes, 3 All
    // strokes over all fills, and 1 leaves it to the Character panel's choice.
    let together = still(parts, "ADBE Text Render Order", 0)
        .map(|v| v.round() as i64)
        .filter(|o| matches!(o, 2 | 3));
    TextDocument {
        text: word("text"),
        expression: None,
        size: number("fontSize").unwrap_or(72.0),
        fill: colour("fillColor").unwrap_or(lumit_core::model::LinearColour([1.0, 1.0, 1.0, 1.0])),
        // After Effects' text-on-a-path rides in the layer's own mask list,
        // which the importer does not read yet: an imported title lays straight.
        path: None,
        path_offset: lumit_core::anim::Property::zero(),
        animators: Vec::new(),
        style: Box::new(TextStyle {
            family,
            face,
            // With auto leading on, After Effects still writes the number it
            // worked out. Lumit works out its own.
            leading: number("leading").filter(|_| on("autoLeading") == Some(false)),
            // After Effects kerns by the font's own metrics unless told not to.
            kerning: Kerning::Metrics,
            tracking: number("tracking").unwrap_or(0.0),
            // After Effects writes a scale of 100 % as 1.
            scale_x: number("horizontalScale").unwrap_or(1.0) * 100.0,
            scale_y: number("verticalScale").unwrap_or(1.0) * 100.0,
            baseline_shift: number("baselineShift").unwrap_or(0.0),
            caps: match (on("allCaps"), on("smallCaps")) {
                (Some(true), _) => Caps::All,
                (_, Some(true)) => Caps::Small,
                _ => Caps::Normal,
            },
            script: match (on("superscript"), on("subscript")) {
                (Some(true), _) => Script::Superscript,
                (_, Some(true)) => Script::Subscript,
                _ => Script::Normal,
            },
            faux_bold: on("fauxBold").unwrap_or(false),
            faux_italic: on("fauxItalic").unwrap_or(false),
            fill_on: on("applyFill").unwrap_or(true),
            stroke_on: on("applyStroke").unwrap_or(false),
            stroke: colour("strokeColor").unwrap_or(lumit_core::model::LinearColour::BLACK),
            stroke_width: number("strokeWidth").unwrap_or(1.0),
            stroke_over: together.map_or_else(|| on("strokeOverFill").unwrap_or(true), |o| o == 3),
            strokes_together: together.is_some(),
            ..TextStyle::default()
        }),
        paragraph: ParagraphStyle {
            align,
            justify,
            box_width: doc
                .get("boxTextSize")
                .filter(|_| on("boxText") == Some(true))
                .and_then(|size| axis_of(size, 0))
                .unwrap_or(0.0),
            ..ParagraphStyle::default()
        },
        extra: serde_json::Map::new(),
    }
}

/// Where After Effects' origin for a text layer falls inside the raster Lumit
/// lays the same words into.
///
/// Point text hangs from its first baseline, at the side it is aligned to. Box
/// text hangs from its box, whose top left corner the script records.
pub(super) fn text_origin(document: &TextDocument, ae: &serde_json::Value) -> (f64, f64) {
    use lumit_core::text::TextAlign;

    let block = lumit_text::layout(&lumit_text::TextBlock::of(document, &document.text), false);
    let (left, right) = (f64::from(block.left), f64::from(block.right));
    let baseline = block
        .lines
        .first()
        .map_or(0.0, |line| f64::from(line.baseline));
    if document.paragraph.box_width > 0.0 {
        let corner = |axis: usize| {
            ae.get("boxTextPos")
                .and_then(|pos| axis_of(pos, axis))
                .unwrap_or(0.0)
        };
        return (
            left - corner(0),
            baseline - f64::from(block.ascent) - corner(1),
        );
    }
    let x = match document.paragraph.align {
        TextAlign::Left => left,
        TextAlign::Centre => (left + right) / 2.0,
        TextAlign::Right => right,
    };
    (x, baseline)
}
