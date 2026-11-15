//! The batches behind the two commands that reshape a whole comp: trim to
//! the work area, and crop to a region.

use super::{remapped, Op, OpError};
use crate::anim::Animation;
use crate::model::{Document, Layer, TransformProp};
use crate::time::{CompTime, Duration};
use uuid::Uuid;

/// The batch [`Op::TrimCompToWorkArea`] applies.
///
/// # In plain terms
///
/// You marked a stretch of the timeline as the bit you are working on. This
/// makes the composition *be* that stretch: it becomes as long as the work
/// area, and every layer and every marker slides back by however far the work
/// area started in, so what was under the work area is now the whole comp.
/// Nothing is thrown away — a layer that fell outside now hangs off the end and
/// can be slid back in.
///
/// **The work area is cleared first, and that ordering matters.** A batch's
/// inverse is its members' inverses in reverse, so clearing first means the old
/// work area is put back *last* on undo — after the comp is long again, and so
/// without [`Op::SetWorkArea`]'s clamp cutting it down to the shortened comp.
pub(super) fn trim_comp_to_work_area(doc: &Document, comp: Uuid) -> Result<Op, OpError> {
    let c = doc.comp(comp).ok_or(OpError::UnknownComp)?;
    // No work area is the whole comp, so there is nothing to trim to.
    let Some((start, end)) = c.work_area else {
        return Ok(Op::Batch { ops: vec![] });
    };
    let duration = end.delta(start).map_err(|_| OpError::InvalidSpan)?;
    let back = Duration(start.0);
    let shift = |t: CompTime| t.sub_dur(back).map_err(|_| OpError::InvalidSpan);

    let mut ops = vec![
        Op::SetWorkArea {
            comp,
            work_area: None,
        },
        Op::SetCompSettings {
            comp,
            name: c.name.clone(),
            width: c.width,
            height: c.height,
            frame_rate: c.frame_rate,
            duration,
            background: c.background,
        },
        // The clock keeps reading what it read: the comp's first frame is
        // now the work area's, so the Start timecode moves on by as much.
        Op::SetCompStart {
            comp,
            start: c
                .start_timecode()
                .add_dur(back)
                .map_err(|_| OpError::InvalidSpan)?,
        },
    ];
    for l in &c.layers {
        ops.extend(through_lock(
            comp,
            l,
            vec![Op::SetLayerSpan {
                comp,
                layer: l.id,
                in_point: shift(l.in_point)?,
                out_point: shift(l.out_point)?,
                start_offset: shift(l.start_offset)?,
            }],
        ));
    }
    // Comp markers are on the comp's own clock, so they travel with the
    // picture. A layer's markers are in layer time (§11) and move with
    // their layer for free.
    if !c.markers.is_empty() {
        let mut markers = c.markers.clone();
        for m in &mut markers {
            m.time = shift(m.time)?;
        }
        ops.push(Op::SetCompMarkers { comp, markers });
    }
    Ok(Op::Batch { ops })
}

/// The batch [`Op::CropCompToRegion`] applies.
///
/// # In plain terms
///
/// You swept a rectangle on the picture; this makes the composition that
/// rectangle. The frame becomes the rectangle's size, and every layer that is
/// not parented to another moves back by the rectangle's top-left corner, so
/// the picture inside the rectangle stays exactly where it was. Parented layers
/// need no move of their own — they travel with the parent they hang from.
///
/// **A position driven by an expression is left alone**, because there is no
/// number to move: the expression decides where the layer goes on every frame,
/// and rewriting its text is not this command's business.
pub(super) fn crop_comp_to_region(
    doc: &Document,
    comp: Uuid,
    x: f64,
    y: f64,
    width: u32,
    height: u32,
) -> Result<Op, OpError> {
    let c = doc.comp(comp).ok_or(OpError::UnknownComp)?;
    if width == 0 || height == 0 {
        return Err(OpError::InvalidSpan);
    }
    let mut ops = vec![Op::SetCompSettings {
        comp,
        name: c.name.clone(),
        width,
        height,
        frame_rate: c.frame_rate,
        duration: c.duration,
        background: c.background,
    }];
    for l in &c.layers {
        if l.parent.is_some() {
            continue;
        }
        let moves: Vec<Op> = [
            (TransformProp::PositionX, &l.transform.position_x, x),
            (TransformProp::PositionY, &l.transform.position_y, y),
        ]
        .into_iter()
        .filter_map(|(prop, property, delta)| {
            shifted(&property.animation, -delta).map(|animation| Op::SetTransformProperty {
                comp,
                layer: l.id,
                prop,
                animation,
            })
        })
        .collect();
        if !moves.is_empty() {
            ops.extend(through_lock(comp, l, moves));
        }
    }
    Ok(Op::Batch { ops })
}

/// `ops` for one layer, with the lock lifted around them when the layer has one.
///
/// **A comp-level reshape moves the whole world, and a locked layer rides with
/// it** — which is what After Effects does, and the only behaviour that keeps
/// the picture together: a locked layer left where it was would sit at the
/// wrong time, or in the wrong place, beside everything that moved. Without
/// this the whole command would refuse, because the lock guard reads
/// each member of the batch on its own way through [`apply`] and a span or a
/// position is exactly what a lock protects.
///
/// The lock comes off and goes back on **inside the same batch**, so it is one
/// undo step and the journal holds no unlocked moment anyone can stop at — and
/// the batch's own reversal puts the switch back the same way.
fn through_lock(comp: Uuid, layer: &Layer, ops: Vec<Op>) -> Vec<Op> {
    if !layer.switches.locked {
        return ops;
    }
    let switch = |locked| Op::SetLayerLocked {
        comp,
        layer: layer.id,
        locked,
    };
    let mut out = Vec::with_capacity(ops.len() + 2);
    out.push(switch(false));
    out.extend(ops);
    out.push(switch(true));
    out
}

/// `animation` with every value moved by `delta`, or `None` when there is
/// nothing to move — an expression, or a shift of nought.
fn shifted(animation: &Animation, delta: f64) -> Option<Animation> {
    if delta == 0.0 {
        return None;
    }
    remapped(animation, 1.0, delta)
}
