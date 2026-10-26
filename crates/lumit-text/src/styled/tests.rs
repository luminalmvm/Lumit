use lumit_core::text::{ParagraphStyle, TextStyle};

use super::*;

const WHITE: LinearColour = LinearColour([1.0, 1.0, 1.0, 1.0]);

/// Kerned Inter, which is enough to take the styled path.
fn kerned() -> TextStyle {
    TextStyle {
        kerning: Kerning::Metrics,
        ..TextStyle::default()
    }
}

fn block<'a>(text: &'a str, style: &'a TextStyle, paragraph: &'a ParagraphStyle) -> TextBlock<'a> {
    TextBlock {
        text,
        size: 72.0,
        fill: WHITE,
        style,
        paragraph,
    }
}

/// The width of the first line's letters.
fn line_width(text: &str, style: &TextStyle) -> f32 {
    let paragraph = ParagraphStyle::default();
    let l = layout(&block(text, style, &paragraph), false);
    let carets = &l.lines[0].carets;
    carets[carets.len() - 1] - carets[0]
}

fn ink(r: &RasterText) -> u64 {
    r.rgba.chunks_exact(4).map(|p| u64::from(p[3])).sum()
}

#[test]
fn the_layout_box_is_the_rasters_own_size() {
    let paragraph = ParagraphStyle::default();
    let stroked = TextStyle {
        stroke_on: true,
        stroke_width: 9.0,
        baseline_shift: 14.0,
        ..kerned()
    };
    for style in [kerned(), stroked] {
        for text in ["Hello", "two\nlines", "gjpq", ""] {
            let b = block(text, &style, &paragraph);
            let r = rasterise(&b, &[]);
            let l = layout(&b, false);
            assert_eq!((l.width, l.height), (r.width, r.height), "{text:?}");
            let a = rasterise(&b, &[GlyphXform::default()]);
            let la = layout(&b, true);
            assert_eq!(
                (la.width, la.height),
                (a.width, a.height),
                "{text:?} animated"
            );
        }
    }
}

#[test]
fn styled_text_is_drawn_and_drawn_the_same_twice() {
    let (style, paragraph) = (kerned(), ParagraphStyle::default());
    let b = block("Lumit", &style, &paragraph);
    let a = rasterise(&b, &[]);
    assert!(ink(&a) > 10_000, "ink {}", ink(&a));
    assert_eq!(a.rgba, rasterise(&b, &[]).rgba);
}

#[test]
fn empty_styled_text_is_one_transparent_pixel_with_a_caret() {
    let (style, paragraph) = (kerned(), ParagraphStyle::default());
    let b = block("", &style, &paragraph);
    let r = rasterise(&b, &[]);
    assert_eq!((r.width, r.height, r.rgba[3]), (1, 1, 0));
    let l = layout(&b, false);
    assert_eq!(l.lines.len(), 1);
    assert_eq!(l.lines[0].carets.len(), 1);
    assert!(l.lines[0].baseline > 0.0);
}

/// Inter kerns "AV" together, and switching kerning off gives the room back.
#[test]
fn kerning_pulls_a_pair_together() {
    let off = TextStyle {
        ligatures: false,
        ..TextStyle::default()
    };
    let on = TextStyle {
        ligatures: false,
        ..kerned()
    };
    assert!(line_width("AV", &on) < line_width("AV", &off) - 1.0);
}

/// 100 thousandths of 72 px is 7.2 px between each pair, and nothing after
/// the last letter.
#[test]
fn tracking_adds_room_between_letters_only() {
    let tracked = TextStyle {
        tracking: 100.0,
        ..kerned()
    };
    let grown = line_width("HHHH", &tracked) - line_width("HHHH", &kerned());
    assert!((grown - 3.0 * 7.2).abs() < 0.01, "{grown}");
}

#[test]
fn horizontal_scale_squeezes_the_line() {
    let half = TextStyle {
        scale_x: 50.0,
        ..kerned()
    };
    let (full, squeezed) = (line_width("Lumit", &kerned()), line_width("Lumit", &half));
    assert!((squeezed - full * 0.5).abs() < 0.01, "{squeezed} vs {full}");
}

#[test]
fn all_caps_sets_the_capitals() {
    let caps = TextStyle {
        caps: Caps::All,
        ..kerned()
    };
    assert!((line_width("lumit", &caps) - line_width("LUMIT", &kerned())).abs() < 0.01);
    let small = TextStyle {
        caps: Caps::Small,
        ..kerned()
    };
    assert!(line_width("lumit", &small) < line_width("LUMIT", &kerned()) * 0.8);
}

/// A break starts a new line one leading further down, and each line knows
/// which character it starts at, whichever line ending the text uses.
#[test]
fn a_break_starts_a_new_line_one_leading_down() {
    let paragraph = ParagraphStyle::default();
    let auto = layout(&block("one\ntwo", &kerned(), &paragraph), false);
    assert_eq!(auto.lines.len(), 2);
    assert_eq!((auto.lines[0].start, auto.lines[1].start), (0, 4));
    assert_eq!(auto.lines[0].carets.len(), 4);
    assert!((auto.lines[1].baseline - auto.lines[0].baseline - 72.0 * 1.2).abs() < 0.01);

    let set = TextStyle {
        leading: Some(100.0),
        ..kerned()
    };
    let spaced = ParagraphStyle {
        space_before: 6.0,
        space_after: 4.0,
        ..ParagraphStyle::default()
    };
    let l = layout(&block("one\r\ntwo\rthree", &set, &spaced), false);
    assert_eq!(
        l.lines.iter().map(|l| l.start).collect::<Vec<_>>(),
        [0, 5, 9]
    );
    assert!((l.lines[1].baseline - l.lines[0].baseline - 110.0).abs() < 0.01);
}

#[test]
fn alignment_moves_the_short_line() {
    let start = |align| {
        let paragraph = ParagraphStyle {
            align,
            ..ParagraphStyle::default()
        };
        let l = layout(
            &block("a long first line\nshort", &kerned(), &paragraph),
            false,
        );
        (l.lines[0].carets[0], l.lines[1].carets[0])
    };
    let (left, centre, right) = (
        start(TextAlign::Left),
        start(TextAlign::Centre),
        start(TextAlign::Right),
    );
    // The longest line fills the block, so it never moves.
    assert!((left.0 - centre.0).abs() < 0.01 && (left.0 - right.0).abs() < 0.01);
    assert!((left.1 - left.0).abs() < 0.01);
    assert!(right.1 > centre.1 && centre.1 > left.1);
    assert!(((right.1 - left.1) * 0.5 - (centre.1 - left.1)).abs() < 0.01);

    let indented = ParagraphStyle {
        indent_left: 30.0,
        indent_first: 10.0,
        ..ParagraphStyle::default()
    };
    let l = layout(&block("short", &kerned(), &indented), false);
    assert!((l.lines[0].carets[0] - left.0 - 40.0).abs() < 0.01);
}

/// A box wraps the words at its spaces, and justifying stretches every
/// line but the last to the box.
#[test]
fn a_box_wraps_and_justifies() {
    let text = "one two three four five six";
    let boxed = |justify| ParagraphStyle {
        box_width: 400.0,
        justify,
        ..ParagraphStyle::default()
    };
    let ragged = layout(&block(text, &kerned(), &boxed(false)), false);
    assert!(ragged.lines.len() > 1);
    assert!((ragged.right - ragged.left - 400.0).abs() < 0.01);
    // The space a line broke at belongs to neither line.
    let first = &ragged.lines[0];
    assert_eq!(ragged.lines[1].start, first.start + first.carets.len());
    assert!(first.carets.last().unwrap() - ragged.left < 400.0);

    let full = layout(&block(text, &kerned(), &boxed(true)), false);
    assert_eq!(full.lines.len(), ragged.lines.len());
    assert!((full.lines[0].carets.last().unwrap() - full.right).abs() < 0.01);
    let last = full.lines.last().unwrap();
    assert!(last.carets.last().unwrap() < &(full.right - 1.0));
}

#[test]
fn an_outline_adds_ink_and_room_for_it() {
    let paragraph = ParagraphStyle::default();
    let outlined = TextStyle {
        stroke_on: true,
        stroke: LinearColour([1.0, 0.0, 0.0, 1.0]),
        stroke_width: 8.0,
        ..kerned()
    };
    let plain = rasterise(&block("Lumit", &kerned(), &paragraph), &[]);
    let stroked = rasterise(&block("Lumit", &outlined, &paragraph), &[]);
    assert!(ink(&stroked) > ink(&plain));
    assert_eq!(stroked.width, plain.width + 8);
    // Some pixel is the outline's red, not the fill's white.
    assert!(stroked
        .rgba
        .chunks_exact(4)
        .any(|p| p[3] == 255 && p[0] == 255 && p[1] == 0));
    // Letter by letter, a wide outline covers the next letter's fill. All
    // fills over all strokes leaves every fill whole.
    let white = |r: &RasterText| r.rgba.chunks_exact(4).filter(|p| p == &[255; 4]).count();
    let [apart, together] = [false, true].map(|strokes_together| {
        let wide = TextStyle {
            stroke_over: false,
            stroke_width: 30.0,
            strokes_together,
            ..outlined.clone()
        };
        white(&rasterise(&block("Lumit", &wide, &paragraph), &[]))
    });
    assert_eq!(together, white(&plain));
    assert!(apart < together, "{apart} {together}");

    let hollow = TextStyle {
        fill_on: false,
        ..outlined
    };
    let ring = rasterise(&block("Lumit", &hollow, &paragraph), &[]);
    assert!(ring.rgba.chunks_exact(4).all(|p| p[3] == 0 || p[1] < 255));
}

#[test]
fn faux_bold_and_superscript_change_the_ink() {
    let paragraph = ParagraphStyle::default();
    let regular = ink(&rasterise(&block("Lumit", &kerned(), &paragraph), &[]));
    let bold = TextStyle {
        faux_bold: true,
        ..kerned()
    };
    assert!(ink(&rasterise(&block("Lumit", &bold, &paragraph), &[])) > regular);
    let raised = TextStyle {
        script: Script::Superscript,
        ..kerned()
    };
    assert!(ink(&rasterise(&block("Lumit", &raised, &paragraph), &[])) < regular / 2);
}

/// A font this machine doesn't have draws in the built-in face, the same
/// as a layer that never named one.
#[test]
fn a_missing_font_falls_back_to_the_built_in_face() {
    let paragraph = ParagraphStyle::default();
    let missing = TextStyle {
        family: "No Such Family 6f1c".into(),
        face: "Bold".into(),
        ..kerned()
    };
    let a = rasterise(&block("Lumit", &missing, &paragraph), &[]);
    let b = rasterise(&block("Lumit", &kerned(), &paragraph), &[]);
    assert_eq!((a.width, a.height), (b.width, b.height));
    assert_eq!(a.rgba, b.rgba);
}

#[test]
fn an_animator_fades_a_styled_letter() {
    let paragraph = ParagraphStyle::default();
    let style = kerned();
    let b = block("HH", &style, &paragraph);
    let still = rasterise(&b, &[GlyphXform::default(), GlyphXform::default()]);
    let gone = GlyphXform {
        opacity: 0.0,
        ..GlyphXform::default()
    };
    let faded = rasterise(&b, &[GlyphXform::default(), gone]);
    assert_eq!((still.width, still.height), (faded.width, faded.height));
    let (whole, half) = (ink(&still), ink(&faded));
    assert!(half * 2 > whole - whole / 10 && half * 2 < whole + whole / 10);
}

#[test]
fn styled_text_runs_along_a_path_and_converts_to_outlines() {
    let paragraph = ParagraphStyle::default();
    let style = TextStyle {
        stroke_on: true,
        stroke_width: 3.0,
        ..kerned()
    };
    let b = block("Lumit", &style, &paragraph);
    let path = MaskPolyline {
        points: vec![[20.0, 120.0], [400.0, 120.0]],
        arc: vec![0.0, 380.0],
        closed: false,
        feather: 0.0,
        expansion: 0.0,
        corners: Vec::new(),
    };
    let r = rasterise_on_path(&b, &path, 0.0, 420, 200, &[]);
    assert_eq!((r.width, r.height), (420, 200));
    assert!(ink(&r) > 10_000);
    // The ink sits on the path's baseline, not at the top of the box.
    let top: u64 = r.rgba[..420 * 40 * 4]
        .chunks_exact(4)
        .map(|p| u64::from(p[3]))
        .sum();
    assert_eq!(top, 0);

    assert_eq!(outlines(&b, None, 0.0).len(), 5);
    let items = crate::shape_items(&b, None, 0.0);
    assert!(items
        .iter()
        .all(|i| i.stroke.is_some() && i.stroke_width.value_at(0.0) == 3.0));
}
