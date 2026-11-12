//! The Viewer's matte view: a shot drawn alone, its stack stopping at one
//! Correction that shows its key rather than its grade.
//!
//! Preview only. The worker applies it to its own copy of the document, so
//! nothing is written, and an export never sees one.

use uuid::Uuid;

use super::Shot;
use crate::model::{Document, EffectInstance, EffectParam, EffectValue, Layer, LayerKind};

/// How the key is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatteMode {
    /// The picture where the key is, flat grey elsewhere.
    Highlight = 1,
    /// The key itself, black and white.
    Key = 2,
    /// Highlight over black.
    HighlightBlack = 3,
    /// Highlight over white.
    HighlightWhite = 4,
    /// Highlight over the picture in black and white.
    HighlightMono = 5,
}

/// Which node's key the Viewer shows, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatteView {
    pub shot: Shot,
    pub effect: Uuid,
    pub mode: MatteMode,
}

/// The row the copy puts on the node, which no schema declares: the
/// Correction reads it as it resolves, and the frame key hashes it with the
/// node's other rows, so the view names frames of its own.
pub const MATTE_VIEW_ROW: &str = "matte_view";

const CORRECTION: &str = "correction";

/// `doc` with `view` applied, or `None` where it changes nothing: a shot or
/// node that has gone, or a node that is not a Correction. A node in a group's
/// grade or a shared node, which the shot reaches through a reference, is cut
/// in that list, the shot's own stack stopping at the reference.
#[must_use]
pub fn apply(doc: &Document, view: MatteView) -> Option<Document> {
    let held =
        shot_stack(doc, view.shot).and_then(|stack| super::groups::holder(doc, stack, view.effect));
    let mut doc = doc.clone();
    let comp = doc.comp_mut(view.shot.comp)?;
    if !comp.layers.iter().any(|l| l.id == view.shot.layer) {
        return None;
    }
    // Hidden rather than taken out, so they still parent what they parent.
    for l in &mut comp.layers {
        l.switches.visible = l.id == view.shot.layer;
    }
    let layer = comp.layers.iter_mut().find(|l| l.id == view.shot.layer)?;
    let effects = stack_mut(layer, view.shot.clip)?;
    let Some((at, group, part)) = held else {
        mark(effects, view)?;
        return Some(doc);
    };
    effects.truncate(at + 1);
    let g = doc.grade_groups.iter_mut().find(|g| g.id == group)?;
    mark(g.part_mut(part), view)?;
    Some(doc)
}

fn shot_stack(doc: &Document, shot: Shot) -> Option<&[EffectInstance]> {
    let layer = doc
        .comp(shot.comp)?
        .layers
        .iter()
        .find(|l| l.id == shot.layer)?;
    match (shot.clip, &layer.kind) {
        (None, _) => Some(&layer.effects),
        (Some(clip), LayerKind::Sequence { clips }) => {
            Some(&clips.iter().find(|c| c.id == clip)?.effects)
        }
        _ => None,
    }
}

fn stack_mut(layer: &mut Layer, clip: Option<Uuid>) -> Option<&mut Vec<EffectInstance>> {
    match clip {
        None => Some(&mut layer.effects),
        Some(clip) => {
            let LayerKind::Sequence { clips } = &mut layer.kind else {
                return None;
            };
            Some(&mut clips.iter_mut().find(|c| c.id == clip)?.effects)
        }
    }
}

/// `effects` cut after the view's node, and the node set to show its key.
fn mark(effects: &mut Vec<EffectInstance>, view: MatteView) -> Option<()> {
    let at = effects
        .iter()
        .position(|e| e.id == view.effect && e.effect.match_name == CORRECTION)?;
    effects.truncate(at + 1);
    // A node inside a run stays in it: the run reads the key off its input,
    // as the render does, and Highlight shows the run's picture with this
    // node in it. With nothing on above it, the node is Serial anyway, and
    // says so, as the view of a joining node is the run's to draw.
    let alone = !effects.iter().take(at).any(|e| e.enabled);
    let node = effects.last_mut()?;
    node.enabled = true;
    if let Some(p) = node.params.iter_mut().find(|p| p.id == "combine" && alone) {
        p.value = EffectValue::Choice(0);
    }
    node.params.push(EffectParam {
        id: MATTE_VIEW_ROW.to_owned(),
        value: EffectValue::Choice(view.mode as u32),
        extra: serde_json::Map::new(),
    });
    Some(())
}
